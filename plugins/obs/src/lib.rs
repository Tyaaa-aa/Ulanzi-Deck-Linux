//! OBS Studio as a plugin: scenes, recording, streaming, replay buffer, virtual camera and audio,
//! with live state on key labels.
//!
//! One WebSocket to obs-websocket 5. Every handler only sends and returns (the host allows 500 ms
//! per call); answers arrive as `message` events and are matched to what was asked by request id.
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ulanzi_plugin::{Cx, Level, Payload, Plugin, Socket, Status, Timer};

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
                "Password needed. Add password = \"…\" to [plugins.obs.settings] (OBS: Tools → WebSocket Server Settings → Show Connect Info).",
            ),
            Down::AuthFailed => (
                Status::Problem,
                "Wrong password. It doesn’t match OBS’s (Tools → WebSocket Server Settings → Show Connect Info).",
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
            status: None,
        }
    }
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
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
            if self.sent.get(&key) != Some(&value) {
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
        cx.send(socket, Payload::Text(text))?;
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
        ] {
            let _ = self.send_request(cx, ty, json!({}), Kind::Load { generation }, "");
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
            self.reload(cx);
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
                if generation == self.generation && ok {
                    self.apply_load(cx, &p, &data);
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
        let stale: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, p)| ticks - p.sent_tick > PENDING_TICKS)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(p) = self.pending.remove(&id) {
                cx.log(Level::Warn, &format!("OBS didn’t answer {}", p.ty));
            }
        }
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
