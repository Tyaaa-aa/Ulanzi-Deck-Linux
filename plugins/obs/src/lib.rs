//! OBS Studio as a plugin: scenes, recording, streaming, replay buffer, virtual camera and audio,
//! with live state on key labels.
//!
//! One WebSocket to obs-websocket 5. Every handler only sends and returns (the host allows 500 ms
//! per call); answers arrive as `message` events and are matched to what was asked by request id.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ulanzi_plugin::{Cx, Item, Level, Payload, Plugin, Socket, Status, Timer};

use crate::actions::{ObsAction, Plan, VolumeOp, parse_action, plan, volume_db};
use crate::protocol::{
    HandshakeError, MAX_DYNAMIC_KEYS, OP_EVENT, OP_HELLO, OP_IDENTIFIED, OP_RESPONSE, OutputChange,
    Stopwatch, Tracked, apply_event, identify, request, state_pairs, stats_from,
};

pub mod actions;
pub mod protocol;

const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
const HANDSHAKE: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(1000);
/// A request unanswered after this many ticks is dropped.
const PENDING_TICKS: u64 = 5;
/// The host keeps at most this many distinct state keys per plugin for the instance's whole life
/// (a key can never be removed again), and ignores keys over 256 bytes and values over 1 KiB.
const HOST_STATE_KEYS: usize = 256;
const HOST_KEY_BYTES: usize = 256;
const HOST_VALUE_BYTES: usize = 1024;
/// Filter lists are fetched for at most this many sources, and at most this many requests wait
/// for OBS at once, so a big collection or an event burst cannot flood the host's event queue.
const MAX_FILTER_SOURCES: usize = 200;
const FILTERS_IN_FLIGHT: usize = 25;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Config {
    host: String,
    port: u16,
    password: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Opening(Socket),
    Hello(Socket),
    Identifying { socket: Socket, with_auth: bool },
    Live(Socket),
}

impl Phase {
    fn socket(self) -> Option<Socket> {
        match self {
            Phase::Idle => None,
            Phase::Opening(s) | Phase::Hello(s) | Phase::Live(s) => Some(s),
            Phase::Identifying { socket, .. } => Some(socket),
        }
    }
}

/// Why a connection is down; picks the status shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Down {
    Refused,
    Dropped,
    AuthMissing,
    AuthFailed,
    TooOld,
    NotAllowed,
}

impl Down {
    fn status(self) -> (Status, &'static str) {
        match self {
            Down::Refused => (
                Status::Problem,
                "OBS is closed or its WebSocket server is off. In OBS: Tools → WebSocket Server Settings → Enable WebSocket server.",
            ),
            Down::Dropped => (Status::Connecting, "Connection lost. Reconnecting…"),
            Down::AuthMissing => (
                Status::Problem,
                "Password needed. Enter it in Settings → Integrations → OBS Studio (OBS: Tools → WebSocket Server Settings → Show Connect Info).",
            ),
            Down::AuthFailed => (
                Status::Problem,
                "OBS closed the connection during sign-in. Check the password (OBS: Tools → WebSocket Server Settings → Show Connect Info).",
            ),
            Down::TooOld => (
                Status::Problem,
                "OBS 28 or newer needed. This OBS has no WebSocket 5 server.",
            ),
            Down::NotAllowed => (
                Status::Problem,
                "OBS must run on this computer: use host = \"localhost\" or \"127.0.0.1\".",
            ),
        }
    }
}

#[derive(Debug, Clone)]
enum Kind {
    Load { generation: u64 },
    Action,
    VolumeRead { input: String, op: VolumeOp, step: u8 },
    FilterRead { source: String, filter: String },
    Stats,
}

#[derive(Debug, Clone)]
struct Pending {
    ty: String,
    sent_tick: u64,
    kind: Kind,
    /// The scene or input a load request asked about.
    arg: String,
}

pub struct Obs {
    config: Option<Config>,
    phase: Phase,
    backoff: Duration,
    handshake: Option<Timer>,
    retry: Option<Timer>,
    tick: Option<Timer>,
    next_id: u64,
    ticks: u64,
    generation: u64,
    pending: BTreeMap<String, Pending>,
    tracked: Tracked,
    rec: Stopwatch,
    stream: Stopwatch,
    version: Option<(String, String)>,
    scenes_loaded: bool,
    preview_name: String,
    /// Values last sent to the host, by key.
    sent: BTreeMap<String, String>,
    /// Dynamic keys (`mute.*`, `visible.*`) last published with a value.
    dynamic: BTreeSet<String>,
    cap_warned: bool,
    /// Every key ever set on the host; the host's 256-key cap counts these for the whole life
    /// of the instance, so a new key is only published while the total stays under it.
    ever_published: BTreeSet<String>,
    key_cap_warned: bool,
    key_size_warned: bool,
    /// A scene, item or input event changed the shape; reload at the next tick, once.
    reload_wanted: bool,
    /// The collection or profile lists changed; fetch them at the next tick, once.
    lists_wanted: bool,
    /// Sources whose filter lists are still to be asked for.
    filter_todo: BTreeSet<String>,
    status: Option<(Status, String)>,
}

impl Default for Obs {
    fn default() -> Self {
        Self {
            config: None,
            phase: Phase::Idle,
            backoff: FIRST_BACKOFF,
            handshake: None,
            retry: None,
            tick: None,
            next_id: 0,
            ticks: 0,
            generation: 0,
            pending: BTreeMap::new(),
            tracked: Tracked::default(),
            rec: Stopwatch::default(),
            stream: Stopwatch::default(),
            version: None,
            scenes_loaded: false,
            preview_name: String::new(),
            sent: BTreeMap::new(),
            dynamic: BTreeSet::new(),
            cap_warned: false,
            ever_published: BTreeSet::new(),
            key_cap_warned: false,
            key_size_warned: false,
            reload_wanted: false,
            lists_wanted: false,
            filter_todo: BTreeSet::new(),
            status: None,
        }
    }
}

/// Cut `s` to at most `max` bytes without splitting a character.
fn truncate_at_boundary(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Without these two answers the plugin knows nothing useful about OBS.
fn is_essential(ty: &str) -> bool {
    matches!(ty, "GetVersion" | "GetSceneList")
}

fn strings_of(v: &Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn bool_of(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn secs_of(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u64 / 1000
}

fn parse_settings(settings: &Value) -> Result<(String, u16, Option<String>), String> {
    let host = match settings.get("host") {
        None | Some(Value::Null) => "localhost".to_string(),
        Some(Value::String(h)) if h.trim().is_empty() => "localhost".to_string(),
        Some(Value::String(h)) => h.trim().to_string(),
        Some(_) => return Err("host must be text".to_string()),
    };
    let port = match settings.get("port") {
        None | Some(Value::Null) => 4455,
        Some(p) => p
            .as_u64()
            .filter(|p| (1..=65535).contains(p))
            .ok_or("port must be a number from 1 to 65535")? as u16,
    };
    let password = settings
        .get("password")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    Ok((host, port, password))
}

/// Mark the current one of a list of names.
fn current_of(names: &[String], current: &str) -> Vec<Item> {
    names
        .iter()
        .map(|n| {
            let item = Item::new(n, n);
            if n == current { item.state("Current") } else { item }
        })
        .collect()
}

/// Every entry of picker `picker`, from what the plugin already knows (nothing is asked of OBS).
fn picker_items(t: &Tracked, picker: &str) -> Result<Vec<Item>, String> {
    let live = &t.live;
    // Sources in scenes, and the global audio inputs (a mic carries the noise filters).
    let sources_in_scenes: BTreeSet<&String> = live
        .sources
        .iter()
        .map(|(_, source)| source)
        .chain(live.inputs.iter())
        .chain(live.muted.iter())
        .collect();
    Ok(match picker {
        "scenes" => live
            .scenes
            .iter()
            .map(|n| {
                let item = Item::new(n, n);
                if *n == live.program {
                    item.state("Live")
                } else if live.preview.as_ref() == Some(n) {
                    item.state("Preview")
                } else {
                    item
                }
            })
            .collect(),
        "audio_inputs" => live
            .inputs
            .iter()
            .chain(live.muted.iter())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|n| {
                let item = Item::new(n, n);
                if live.muted.contains(n) {
                    item.state("Muted")
                } else {
                    item
                }
            })
            .collect(),
        "scene_sources" => live
            .scenes
            .iter()
            .flat_map(|scene| {
                live.sources
                    .iter()
                    .filter(move |(sc, _)| sc == scene)
                    .map(move |(sc, source)| Item::new(source, source).subtitle(sc).group(sc))
            })
            .collect(),
        "sources" => sources_in_scenes.into_iter().map(|n| Item::new(n, n)).collect(),
        "filters" => sources_in_scenes
            .into_iter()
            .filter_map(|source| t.filters.get(source).map(|list| (source, list)))
            .flat_map(|(source, list)| {
                list.iter().map(move |(name, on)| {
                    Item::new(name, name)
                        .group(source)
                        .state(if *on { "On" } else { "Off" })
                })
            })
            .collect(),
        "collections" => current_of(&t.collections, &t.collection),
        "profiles" => current_of(&t.profiles, &t.profile),
        other => return Err(format!("OBS has no picker \"{other}\"")),
    })
}

impl Obs {
    fn set_status(&mut self, cx: &mut Cx, status: Status, message: &str) {
        if self
            .status
            .as_ref()
            .is_some_and(|(s, m)| *s == status && m == message)
        {
            return;
        }
        self.status = Some((status, message.to_string()));
        cx.set_status(status, message);
        cx.log(Level::Info, message);
    }

    fn connect(&mut self, cx: &mut Cx, announce: bool) {
        let Some(config) = self.config.clone() else { return };
        if let Some(t) = self.retry.take() {
            cx.cancel_timer(t);
        }
        let url = format!("ws://{}:{}", config.host, config.port);
        if announce {
            let msg = format!("Connecting to OBS at {}:{}…", config.host, config.port);
            self.set_status(cx, Status::Connecting, &msg);
        }
        match cx.ws_open(&url) {
            Ok(s) => {
                self.phase = Phase::Opening(s);
                self.handshake = Some(cx.set_timer(HANDSHAKE, false));
            }
            Err(e) if e.starts_with("not allowed") => self.go_down(cx, Down::NotAllowed, false),
            Err(_) => self.go_down(cx, Down::Refused, false),
        }
    }

    /// Cancel timers, drop the connection and forget what was tracked.
    fn teardown(&mut self, cx: &mut Cx, close: bool) {
        for t in [self.handshake.take(), self.retry.take(), self.tick.take()]
            .into_iter()
            .flatten()
        {
            cx.cancel_timer(t);
        }
        if close && let Some(s) = self.phase.socket() {
            cx.close(s);
        }
        self.phase = Phase::Idle;
        self.pending.clear();
        self.tracked = Tracked::default();
        self.rec.stop();
        self.stream.stop();
        self.version = None;
        self.scenes_loaded = false;
        self.preview_name.clear();
        self.cap_warned = false;
        self.reload_wanted = false;
        self.lists_wanted = false;
        self.filter_todo.clear();
        self.publish(cx);
    }

    /// The connection is down: show why, reset the state and schedule the next try.
    fn go_down(&mut self, cx: &mut Cx, why: Down, close: bool) {
        self.teardown(cx, close);
        let (status, message) = why.status();
        self.set_status(cx, status, message);
        if why == Down::NotAllowed {
            return;
        }
        self.retry = Some(cx.set_timer(self.backoff, false));
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
    }

    /// Send what changed since last time.
    fn publish(&mut self, cx: &mut Cx) {
        let now = Instant::now();
        self.tracked.live.rec_secs = self.rec.secs(now);
        self.tracked.live.stream_secs = self.stream.secs(now);
        let live = &self.tracked.live;
        if live.connected && !self.cap_warned {
            let inputs: BTreeSet<_> = live.inputs.iter().chain(live.muted.iter()).collect();
            let sources: BTreeSet<_> = live.sources.iter().chain(live.hidden.iter()).collect();
            if inputs.len() + sources.len() > MAX_DYNAMIC_KEYS {
                self.cap_warned = true;
                cx.log(
                    Level::Warn,
                    &format!(
                        "OBS has more than {MAX_DYNAMIC_KEYS} inputs and sources; only the first {MAX_DYNAMIC_KEYS} get {{obs.mute.…}} and {{obs.visible.…}} values"
                    ),
                );
            }
        }
        let pairs = state_pairs(&self.tracked.live, &self.dynamic);
        self.dynamic.clear();
        for (key, value) in pairs {
            let dynamic = key.starts_with("mute.") || key.starts_with("visible.");
            let value = truncate_at_boundary(value, HOST_VALUE_BYTES);
            if self.sent.get(&key) != Some(&value) {
                if !self.may_publish(cx, &key) {
                    continue;
                }
                cx.set_state(&key, value.clone());
                self.sent.insert(key.clone(), value.clone());
            }
            if dynamic {
                if value.is_empty() {
                    self.sent.remove(&key);
                } else {
                    self.dynamic.insert(key);
                }
            }
        }
    }

    /// Whether the host will accept `key`: not over its size limit, and either already known to
    /// it or still under its lifetime key cap. Warns once for each reason otherwise.
    fn may_publish(&mut self, cx: &mut Cx, key: &str) -> bool {
        if key.len() > HOST_KEY_BYTES {
            if !self.key_size_warned {
                self.key_size_warned = true;
                cx.log(
                    Level::Warn,
                    &format!(
                        "A state key longer than {HOST_KEY_BYTES} bytes is not published (starts with “{}”)",
                        truncate_at_boundary(key.to_string(), 40)
                    ),
                );
            }
            return false;
        }
        if self.ever_published.contains(key) {
            return true;
        }
        if self.ever_published.len() >= HOST_STATE_KEYS {
            if !self.key_cap_warned {
                self.key_cap_warned = true;
                cx.log(
                    Level::Warn,
                    &format!(
                        "All {HOST_STATE_KEYS} state keys the host allows this plugin are used (renamed or removed scenes, sources and inputs keep theirs until the plugin restarts); new {{obs.…}} values are not published until you disable and enable the plugin"
                    ),
                );
            }
            return false;
        }
        self.ever_published.insert(key.to_string());
        true
    }

    fn update_ok_status(&mut self, cx: &mut Cx) {
        if let Some((obs, ws)) = self.version.clone()
            && self.scenes_loaded
        {
            let n = self.tracked.live.scenes.len();
            let msg = format!(
                "OBS {obs} · WebSocket {ws} · {n} scene{}",
                if n == 1 { "" } else { "s" }
            );
            self.tracked.live.connected = true;
            self.set_status(cx, Status::Ok, &msg);
        }
    }

    fn offline_error(&self) -> String {
        let title = match &self.status {
            Some((_, m)) => m
                .split_once(". ")
                .map_or(m.as_str(), |(t, _)| t)
                .trim_end_matches('.'),
            None => "not started",
        };
        format!("OBS isn’t connected: {title}")
    }

    fn send_request(
        &mut self,
        cx: &mut Cx,
        ty: &str,
        data: Value,
        kind: Kind,
        arg: &str,
    ) -> Result<(), String> {
        let Phase::Live(socket) = self.phase else {
            return Err(self.offline_error());
        };
        self.next_id += 1;
        let id = format!("o{}", self.next_id);
        let text = request(&id, ty, data).to_string();
        if cx.send(socket, Payload::Text(text)).is_err() {
            // The host lost the socket (its close notice may have been dropped): without this
            // the plugin would stay "connected" with frozen labels.
            self.go_down(cx, Down::Dropped, true);
            return Err(self.offline_error());
        }
        self.pending.insert(
            id,
            Pending {
                ty: ty.to_string(),
                sent_tick: self.ticks,
                kind,
                arg: arg.to_string(),
            },
        );
        Ok(())
    }

    fn load(&mut self, cx: &mut Cx) {
        let generation = self.generation;
        for ty in [
            "GetVersion",
            "GetSceneList",
            "GetStudioModeEnabled",
            "GetRecordStatus",
            "GetStreamStatus",
            "GetReplayBufferStatus",
            "GetVirtualCamStatus",
            "GetInputList",
            "GetSceneCollectionList",
            "GetProfileList",
        ] {
            let _ = self.send_request(cx, ty, json!({}), Kind::Load { generation }, "");
        }
    }

    fn reload_lists(&mut self, cx: &mut Cx) {
        let generation = self.generation;
        for ty in ["GetSceneCollectionList", "GetProfileList"] {
            let _ = self.send_request(cx, ty, json!({}), Kind::Load { generation }, "");
        }
    }

    /// Sources that can have filters: those in a scene, and the global audio inputs.
    fn known_sources(&self) -> BTreeSet<String> {
        let live = &self.tracked.live;
        live.sources
            .iter()
            .map(|(_, source)| source)
            .chain(live.inputs.iter())
            .chain(live.muted.iter())
            .cloned()
            .collect()
    }

    fn filters_in_flight(&self, source: &str) -> bool {
        self.pending
            .values()
            .any(|p| p.ty == "GetSourceFilterList" && p.arg == source)
    }

    /// Whether `source` may be (re)queued: it already has an entry, or the entries of sources
    /// that still exist plus the queue leave room under [`MAX_FILTER_SOURCES`].
    fn filter_room(&self, known: &BTreeSet<String>, source: &str) -> bool {
        if !known.contains(source) {
            return false;
        }
        if self.tracked.filters.contains_key(source) {
            return true;
        }
        let kept = self.tracked.filters.keys().filter(|k| known.contains(*k)).count();
        kept + self.filter_todo.len() < MAX_FILTER_SOURCES
    }

    /// Ask for the filters of every known source that has none yet, within the caps. Entries of
    /// sources that left stay until a reload settles (it rebuilds scenes one answer at a time).
    fn queue_filters(&mut self) {
        let known = self.known_sources();
        for source in &known {
            if !self.tracked.filters.contains_key(source)
                && !self.filters_in_flight(source)
                && self.filter_room(&known, source)
            {
                self.filter_todo.insert(source.clone());
            }
        }
    }

    /// Queue `source` again (a request was lost or went stale, or its filters changed).
    fn requeue_filters(&mut self, source: &str) {
        if self.filter_room(&self.known_sources(), source) {
            self.filter_todo.insert(source.to_string());
        }
    }

    /// Forget filter entries of sources that no longer exist, once a reload has fully landed.
    fn settle_filters(&mut self) {
        let busy = self.pending.values().any(|p| {
            matches!(p.kind, Kind::Load { .. })
                && matches!(
                    p.ty.as_str(),
                    "GetSceneList" | "GetSceneItemList" | "GetInputList" | "GetInputMute"
                )
        });
        if !busy {
            let known = self.known_sources();
            self.tracked.filters.retain(|source, _| known.contains(source));
        }
    }

    /// Send queued filter requests while fewer than [`FILTERS_IN_FLIGHT`] wait for an answer. A
    /// source that already has a request out stays queued and goes out after that answer.
    fn pump_filters(&mut self, cx: &mut Cx) {
        let generation = self.generation;
        let open = self
            .pending
            .values()
            .filter(|p| p.ty == "GetSourceFilterList")
            .count();
        let ready: Vec<String> = self
            .filter_todo
            .iter()
            .filter(|source| !self.filters_in_flight(source))
            .take(FILTERS_IN_FLIGHT.saturating_sub(open))
            .cloned()
            .collect();
        for source in ready {
            if !matches!(self.phase, Phase::Live(_)) {
                break;
            }
            self.filter_todo.remove(&source);
            let data = json!({"sourceName": source});
            if self
                .send_request(
                    cx,
                    "GetSourceFilterList",
                    data,
                    Kind::Load { generation },
                    &source,
                )
                .is_err()
            {
                break;
            }
        }
    }

    fn reload(&mut self, cx: &mut Cx) {
        self.generation += 1;
        self.tracked.items.clear();
        self.tracked.live.inputs.clear();
        self.tracked.live.muted.clear();
        let generation = self.generation;
        for ty in ["GetSceneList", "GetInputList"] {
            let _ = self.send_request(cx, ty, json!({}), Kind::Load { generation }, "");
        }
    }

    fn handle_text(&mut self, cx: &mut Cx, socket: Socket, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let op = v.get("op").and_then(Value::as_u64);
        match self.phase {
            Phase::Hello(s) if s == socket && op == Some(OP_HELLO) => {
                let password = self.config.as_ref().and_then(|c| c.password.clone());
                match identify(&v, password.as_deref()) {
                    Ok(msg) => {
                        let with_auth = msg["d"].get("authentication").is_some();
                        if cx.send(socket, Payload::Text(msg.to_string())).is_err() {
                            self.go_down(cx, Down::Dropped, true);
                        } else {
                            self.phase = Phase::Identifying { socket, with_auth };
                        }
                    }
                    Err(HandshakeError::AuthMissing) => self.go_down(cx, Down::AuthMissing, true),
                    Err(HandshakeError::TooOld) => self.go_down(cx, Down::TooOld, true),
                    Err(HandshakeError::AuthFailed) => self.go_down(cx, Down::AuthFailed, true),
                    Err(HandshakeError::Dropped) => self.go_down(cx, Down::Dropped, true),
                }
            }
            Phase::Identifying { socket: s, .. } if s == socket && op == Some(OP_IDENTIFIED) => {
                if let Some(t) = self.handshake.take() {
                    cx.cancel_timer(t);
                }
                self.backoff = FIRST_BACKOFF;
                self.phase = Phase::Live(socket);
                self.tick = Some(cx.set_timer(TICK, true));
                self.load(cx);
            }
            Phase::Live(s) if s == socket => match op {
                Some(OP_RESPONSE) => self.handle_response(cx, &v["d"]),
                Some(OP_EVENT) => self.handle_event(cx, &v["d"]),
                _ => {}
            },
            _ => {}
        }
    }

    fn handle_event(&mut self, cx: &mut Cx, d: &Value) {
        let ty = str_of(d, "eventType");
        let data = d.get("eventData").cloned().unwrap_or(json!({}));
        let e = apply_event(&mut self.tracked, &ty, &data);
        let now = Instant::now();
        match e.record {
            Some(OutputChange::Started) => self.rec.start(now, 0),
            Some(OutputChange::Paused) => self.rec.pause(now),
            Some(OutputChange::Resumed) => self.rec.resume(now),
            Some(OutputChange::Stopped) => self.rec.stop(),
            None => {}
        }
        match e.stream {
            Some(OutputChange::Started) => self.stream.start(now, 0),
            Some(OutputChange::Stopped) => self.stream.stop(),
            _ => {}
        }
        if e.reload {
            self.reload_wanted = true;
        }
        if e.lists {
            self.lists_wanted = true;
        }
        if let Some(source) = e.refilter {
            self.requeue_filters(&source);
        }
        if e.changed || e.reload {
            self.update_ok_status(cx);
            self.publish(cx);
        }
    }

    fn handle_response(&mut self, cx: &mut Cx, d: &Value) {
        let Some(p) = self.pending.remove(&str_of(d, "requestId")) else {
            return;
        };
        let status = d.get("requestStatus").cloned().unwrap_or_default();
        let ok = bool_of(&status, "result");
        let data = d.get("responseData").cloned().unwrap_or(json!({}));
        match p.kind.clone() {
            Kind::Load { generation } => {
                if generation == self.generation {
                    if ok {
                        self.apply_load(cx, &p, &data);
                    } else if is_essential(&p.ty) {
                        cx.log(Level::Warn, &format!("OBS refused {}", p.ty));
                        self.go_down(cx, Down::Dropped, true);
                    } else if p.ty == "GetSourceFilterList" {
                        // Remember "none" so the source is not asked about again and again.
                        self.tracked.filters.entry(p.arg.clone()).or_default();
                    }
                }
                if p.ty == "GetSourceFilterList" {
                    if generation != self.generation {
                        self.requeue_filters(&p.arg);
                    }
                    self.pump_filters(cx);
                }
                if generation == self.generation {
                    self.settle_filters();
                }
            }
            Kind::Action | Kind::VolumeRead { .. } | Kind::FilterRead { .. } => {
                if !ok {
                    let why = match status.get("comment").and_then(Value::as_str) {
                        Some(c) if !c.is_empty() => c.to_string(),
                        _ => format!(
                            "OBS refused {} (code {})",
                            p.ty,
                            status.get("code").and_then(Value::as_i64).unwrap_or(0)
                        ),
                    };
                    cx.log(Level::Warn, &why);
                    self.tracked.live.last_error = why;
                } else {
                    self.tracked.live.last_error.clear();
                    self.follow_up(cx, &p.kind, &data);
                }
                self.publish(cx);
            }
            Kind::Stats => {
                if ok {
                    self.tracked.live.stats = Some(stats_from(&data));
                    self.publish(cx);
                }
            }
        }
    }

    /// The second request of an action that reads before it writes.
    fn follow_up(&mut self, cx: &mut Cx, kind: &Kind, data: &Value) {
        let sent = match kind {
            Kind::VolumeRead { input, op, step } => {
                let cur = data.get("inputVolumeDb").and_then(Value::as_f64).unwrap_or(0.0);
                let db = volume_db(cur, *op, *step);
                self.send_request(
                    cx,
                    "SetInputVolume",
                    json!({"inputName": input, "inputVolumeDb": db}),
                    Kind::Action,
                    "",
                )
            }
            Kind::FilterRead { source, filter } => {
                let on = bool_of(data, "filterEnabled");
                self.send_request(
                    cx,
                    "SetSourceFilterEnabled",
                    json!({"sourceName": source, "filterName": filter, "filterEnabled": !on}),
                    Kind::Action,
                    "",
                )
            }
            _ => Ok(()),
        };
        if let Err(e) = sent {
            cx.log(Level::Warn, &e);
        }
    }

    /// Apply an answer of the initial (or repeated) load, as the built-in client does.
    fn apply_load(&mut self, cx: &mut Cx, p: &Pending, v: &Value) {
        let generation = self.generation;
        let now = Instant::now();
        match p.ty.as_str() {
            "GetVersion" => {
                self.version = Some((str_of(v, "obsVersion"), str_of(v, "obsWebSocketVersion")));
                self.update_ok_status(cx);
            }
            "GetSceneList" => {
                self.tracked.live.program = str_of(v, "currentProgramSceneName");
                let mut names: Vec<String> = v
                    .get("scenes")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.get("sceneName").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                // OBS lists scenes bottom-up.
                names.reverse();
                self.preview_name = str_of(v, "currentPreviewSceneName");
                if self.tracked.live.preview.is_some() {
                    self.tracked.live.preview = Some(self.preview_name.clone());
                }
                for scene in &names {
                    let _ = self.send_request(
                        cx,
                        "GetSceneItemList",
                        json!({"sceneName": scene}),
                        Kind::Load { generation },
                        scene,
                    );
                }
                self.tracked.live.scenes = names;
                self.scenes_loaded = true;
                self.update_ok_status(cx);
            }
            "GetStudioModeEnabled" => {
                self.tracked.live.preview =
                    bool_of(v, "studioModeEnabled").then(|| self.preview_name.clone());
            }
            "GetRecordStatus" => {
                let live = &mut self.tracked.live;
                live.recording = bool_of(v, "outputActive");
                live.rec_paused = bool_of(v, "outputPaused");
                if live.recording {
                    self.rec.start(now, secs_of(v, "outputDuration"));
                    if live.rec_paused {
                        self.rec.pause(now);
                    }
                } else {
                    self.rec.stop();
                }
            }
            "GetStreamStatus" => {
                self.tracked.live.streaming = bool_of(v, "outputActive");
                if self.tracked.live.streaming {
                    self.stream.start(now, secs_of(v, "outputDuration"));
                } else {
                    self.stream.stop();
                }
            }
            "GetReplayBufferStatus" => self.tracked.live.replay = bool_of(v, "outputActive"),
            "GetVirtualCamStatus" => self.tracked.live.vcam = bool_of(v, "outputActive"),
            "GetInputList" => {
                let inputs: Vec<String> = v
                    .get("inputs")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| i.get("inputName").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                for input in inputs {
                    // Inputs without audio answer with an error; they can't be muted anyway.
                    let _ = self.send_request(
                        cx,
                        "GetInputMute",
                        json!({"inputName": input}),
                        Kind::Load { generation },
                        &input,
                    );
                }
            }
            "GetInputMute" => {
                self.tracked.live.inputs.insert(p.arg.clone());
                if bool_of(v, "inputMuted") {
                    self.tracked.live.muted.insert(p.arg.clone());
                } else {
                    self.tracked.live.muted.remove(&p.arg);
                }
                self.queue_filters();
                self.pump_filters(cx);
            }
            "GetSceneCollectionList" => {
                self.tracked.collection = str_of(v, "currentSceneCollectionName");
                self.tracked.collections = strings_of(v, "sceneCollections");
            }
            "GetProfileList" => {
                self.tracked.profile = str_of(v, "currentProfileName");
                self.tracked.profiles = strings_of(v, "profiles");
            }
            "GetSourceFilterList" => {
                let filters = v
                    .get("filters")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|f| (str_of(f, "filterName"), bool_of(f, "filterEnabled")))
                    .collect();
                self.tracked.filters.insert(p.arg.clone(), filters);
            }
            "GetSceneItemList" => {
                for item in v
                    .get("sceneItems")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let id = item.get("sceneItemId").and_then(Value::as_i64).unwrap_or(-1);
                    self.tracked.items.insert(
                        (p.arg.clone(), id),
                        (str_of(item, "sourceName"), bool_of(item, "sceneItemEnabled")),
                    );
                }
                self.tracked.sync_hidden();
                self.queue_filters();
                self.pump_filters(cx);
            }
            _ => {}
        }
        self.publish(cx);
    }

    fn on_tick(&mut self, cx: &mut Cx) {
        if !matches!(self.phase, Phase::Live(_)) {
            return;
        }
        self.ticks += 1;
        let ticks = self.ticks;
        if self.reload_wanted {
            self.reload_wanted = false;
            self.reload(cx);
            if !matches!(self.phase, Phase::Live(_)) {
                return;
            }
        }
        if self.lists_wanted {
            self.lists_wanted = false;
            self.reload_lists(cx);
        }
        let stale: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| ticks - p.sent_tick > PENDING_TICKS)
            .map(|(id, _)| id.clone())
            .collect();
        let mut lost = false;
        let mut filters_lost: Vec<String> = Vec::new();
        for id in stale {
            if let Some(p) = self.pending.remove(&id) {
                match p.kind {
                    // Only what a key press asked for is worth a log line.
                    Kind::Action | Kind::VolumeRead { .. } | Kind::FilterRead { .. } => {
                        cx.log(Level::Warn, &format!("OBS didn’t answer {}", p.ty));
                    }
                    Kind::Load { generation } if generation == self.generation && is_essential(&p.ty) => {
                        cx.log(Level::Warn, &format!("OBS didn’t answer {}", p.ty));
                        lost = true;
                    }
                    Kind::Load { generation } => {
                        if p.ty == "GetSourceFilterList" && generation == self.generation {
                            filters_lost.push(p.arg);
                        }
                    }
                    Kind::Stats => {}
                }
            }
        }
        if lost {
            self.go_down(cx, Down::Dropped, true);
            return;
        }
        for source in filters_lost {
            self.requeue_filters(&source);
        }
        self.pump_filters(cx);
        if ticks.is_multiple_of(2) {
            let _ = self.send_request(cx, "GetStats", json!({}), Kind::Stats, "");
        }
        self.publish(cx);
    }
}

impl Plugin for Obs {
    fn init(&mut self, cx: &mut Cx, settings: Value) -> Result<(), String> {
        let (host, port, from_settings) = parse_settings(&settings)?;
        if let Some(p) = &from_settings {
            cx.set_secret("password", p);
            cx.log(
                Level::Warn,
                "The password is saved in the keyring. Delete the password line from [plugins.obs.settings] in settings.toml.",
            );
        }
        let password = from_settings.or_else(|| cx.secret("password").filter(|p| !p.is_empty()));
        let config = Config { host, port, password };
        if self.config.as_ref() == Some(&config) && self.phase != Phase::Idle {
            return Ok(());
        }
        self.teardown(cx, true);
        self.config = Some(config);
        self.backoff = FIRST_BACKOFF;
        self.connect(cx, true);
        Ok(())
    }

    fn action(&mut self, cx: &mut Cx, action: &str, params: Value) -> Result<(), String> {
        match action {
            "reconnect" => {
                self.teardown(cx, true);
                self.backoff = FIRST_BACKOFF;
                self.connect(cx, true);
                return Ok(());
            }
            "forget_password" => {
                cx.delete_secret("password");
                if let Some(c) = &mut self.config {
                    c.password = None;
                }
                return Ok(());
            }
            _ => {}
        }
        let parsed: ObsAction = parse_action(action, &params)?;
        if !matches!(self.phase, Phase::Live(_)) {
            return Err(self.offline_error());
        }
        match plan(&parsed, &self.tracked)? {
            Plan::Requests(list) => {
                for (ty, data) in list {
                    self.send_request(cx, &ty, data, Kind::Action, "")?;
                }
            }
            Plan::InputVolume { input, op, step } => {
                let data = json!({"inputName": input});
                self.send_request(
                    cx,
                    "GetInputVolume",
                    data,
                    Kind::VolumeRead { input, op, step },
                    "",
                )?;
            }
            Plan::ToggleFilter { source, filter } => {
                let data = json!({"sourceName": source, "filterName": filter});
                self.send_request(
                    cx,
                    "GetSourceFilter",
                    data,
                    Kind::FilterRead { source, filter },
                    "",
                )?;
            }
        }
        Ok(())
    }

    fn list_items(
        &mut self,
        _cx: &mut Cx,
        picker: &str,
        query: &str,
        group: &str,
    ) -> Result<Vec<Item>, String> {
        if !matches!(self.phase, Phase::Live(_)) {
            return Err("Not connected to OBS".to_string());
        }
        let items = picker_items(&self.tracked, picker)?;
        let query = query.to_lowercase();
        Ok(items
            .into_iter()
            .filter(|i| {
                (query.is_empty()
                    || i.label.to_lowercase().contains(&query)
                    || i.id.to_lowercase().contains(&query))
                    && (group.is_empty() || i.group.as_deref() == Some(group))
            })
            .collect())
    }

    fn opened(&mut self, _cx: &mut Cx, socket: Socket) {
        if self.phase == Phase::Opening(socket) {
            self.phase = Phase::Hello(socket);
        }
    }

    fn message(&mut self, cx: &mut Cx, socket: Socket, data: Payload) {
        if self.phase.socket() != Some(socket) {
            return;
        }
        if let Payload::Text(text) = data {
            self.handle_text(cx, socket, &text);
        }
    }

    fn closed(&mut self, cx: &mut Cx, socket: Socket, _reason: &str) {
        let why = match self.phase {
            Phase::Opening(s) if s == socket => Down::Refused,
            Phase::Hello(s) if s == socket => Down::Dropped,
            Phase::Identifying { socket: s, with_auth } if s == socket => {
                // The host passes only the close reason text, never the code, so a refused
                // password (4009) cannot be told from other closes: infer it from the phase.
                if with_auth {
                    Down::AuthFailed
                } else {
                    Down::Dropped
                }
            }
            Phase::Live(s) if s == socket => Down::Dropped,
            _ => return,
        };
        self.go_down(cx, why, false);
    }

    fn timer(&mut self, cx: &mut Cx, timer: Timer) {
        if self.handshake == Some(timer) {
            self.handshake = None;
            if !matches!(self.phase, Phase::Live(_) | Phase::Idle) {
                self.go_down(cx, Down::Dropped, true);
            }
        } else if self.retry == Some(timer) {
            self.retry = None;
            if self.phase == Phase::Idle {
                self.connect(cx, false);
            }
        } else if self.tick == Some(timer) {
            self.on_tick(cx);
        }
    }
}

ulanzi_plugin::export_plugin!(Obs);
