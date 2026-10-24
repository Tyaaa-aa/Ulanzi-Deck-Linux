//! obs-websocket 5 without I/O: the handshake, keeping the [`Live`] snapshot current from events,
//! and the key values published from it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const OP_HELLO: u64 = 0;
pub const OP_IDENTIFY: u64 = 1;
pub const OP_IDENTIFIED: u64 = 2;
pub const OP_EVENT: u64 = 5;
pub const OP_REQUEST: u64 = 6;
pub const OP_RESPONSE: u64 = 7;
const RPC_VERSION: u64 = 1;
/// Every event category except the high-volume ones (volume meters and the like).
const EVENTS_ALL: u64 = 0x7ff;

/// Why a connection did not get to (or left) the identified state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// OBS asks for a password and none is set.
    AuthMissing,
    /// The password was wrong (close code 4009).
    AuthFailed,
    /// Not obs-websocket 5 (OBS older than 28), or close code 4010.
    TooOld,
    /// The connection closed for another reason.
    Dropped,
}

/// `base64(sha256(base64(sha256(password + salt)) + challenge))`.
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret = b64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    b64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

/// The Identify message answering `hello`.
pub fn identify(hello: &Value, password: Option<&str>) -> Result<Value, HandshakeError> {
    let d = hello.get("d").ok_or(HandshakeError::TooOld)?;
    if hello.get("op").and_then(Value::as_u64) != Some(OP_HELLO)
        || d.get("rpcVersion").and_then(Value::as_u64).unwrap_or(0) < RPC_VERSION
    {
        return Err(HandshakeError::TooOld);
    }
    let mut out = json!({"rpcVersion": RPC_VERSION, "eventSubscriptions": EVENTS_ALL});
    if let Some(auth) = d.get("authentication") {
        let password = password
            .filter(|p| !p.is_empty())
            .ok_or(HandshakeError::AuthMissing)?;
        let s = |k: &str| auth.get(k).and_then(Value::as_str).unwrap_or_default();
        out["authentication"] = auth_string(password, s("salt"), s("challenge")).into();
    }
    Ok(json!({"op": OP_IDENTIFY, "d": out}))
}

pub fn request(id: &str, ty: &str, data: Value) -> Value {
    json!({"op": OP_REQUEST, "d": {"requestType": ty, "requestId": id, "requestData": data}})
}

/// Frames per second, CPU use and skipped frames of OBS.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Frames per second, rounded.
    pub fps: u32,
    /// OBS's CPU use, percent.
    pub cpu: u32,
    /// Frames skipped by the encoder or the network, per thousand frames.
    pub dropped_permille: u32,
}

/// From a `GetStats` response.
pub fn stats_from(v: &Value) -> Stats {
    let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let total = f("outputTotalFrames") + f("renderTotalFrames");
    let skipped = f("outputSkippedFrames") + f("renderSkippedFrames");
    Stats {
        fps: f("activeFps").round() as u32,
        cpu: f("cpuUsage").round() as u32,
        dropped_permille: if total > 0.0 {
            (skipped / total * 1000.0).round() as u32
        } else {
            0
        },
    }
}

/// What the plugin knows about OBS Studio right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Live {
    pub connected: bool,
    /// The scene on the program output.
    pub program: String,
    /// The preview scene, while studio mode is on.
    pub preview: Option<String>,
    pub scenes: Vec<String>,
    pub recording: bool,
    pub rec_paused: bool,
    pub rec_secs: u64,
    pub streaming: bool,
    pub stream_secs: u64,
    pub replay: bool,
    pub vcam: bool,
    /// Inputs that are muted.
    pub muted: BTreeSet<String>,
    /// (scene, source) pairs that are hidden.
    pub hidden: BTreeSet<(String, String)>,
    pub stats: Option<Stats>,
    /// OBS's last refusal of an action.
    pub last_error: String,
    /// Inputs whose mute state is known (muted or not); `muted` is the subset that is muted.
    pub inputs: BTreeSet<String>,
    /// (scene, source) pairs that exist; `hidden` is the subset that is hidden.
    pub sources: BTreeSet<(String, String)>,
}

/// What the client keeps besides the snapshot: scene items (to show/hide them by name).
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    pub live: Live,
    /// (scene, item id) → (source name, enabled).
    pub items: BTreeMap<(String, i64), (String, bool)>,
}

impl Tracked {
    /// Recompute the hidden set (and the known sources) from the items.
    pub fn sync_hidden(&mut self) {
        self.live.sources = self
            .items
            .iter()
            .map(|((scene, _), (source, _))| (scene.clone(), source.clone()))
            .collect();
        self.live.hidden = self
            .items
            .iter()
            .filter(|(_, (_, on))| !on)
            .map(|((scene, _), (source, _))| (scene.clone(), source.clone()))
            .collect();
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEffect {
    pub changed: bool,
    /// Scenes or items changed shape: fetch them again.
    pub reload: bool,
    /// Recording or streaming started, paused, resumed or stopped.
    pub record: Option<OutputChange>,
    pub stream: Option<OutputChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputChange {
    Started,
    Paused,
    Resumed,
    Stopped,
}

fn s<'a>(d: &'a Value, k: &str) -> &'a str {
    d.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn b(d: &Value, k: &str) -> bool {
    d.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn output_change(d: &Value) -> OutputChange {
    match s(d, "outputState") {
        "OBS_WEBSOCKET_OUTPUT_PAUSED" => OutputChange::Paused,
        "OBS_WEBSOCKET_OUTPUT_RESUMED" => OutputChange::Resumed,
        _ if b(d, "outputActive") => OutputChange::Started,
        _ => OutputChange::Stopped,
    }
}

/// Apply one event to the snapshot.
pub fn apply_event(t: &mut Tracked, ty: &str, d: &Value) -> EventEffect {
    let mut e = EventEffect {
        changed: true,
        ..EventEffect::default()
    };
    let live = &mut t.live;
    match ty {
        "CurrentProgramSceneChanged" => live.program = s(d, "sceneName").into(),
        "CurrentPreviewSceneChanged" => live.preview = Some(s(d, "sceneName").into()),
        "StudioModeStateChanged" => {
            live.preview = b(d, "studioModeEnabled").then(|| live.program.clone());
        }
        "InputMuteStateChanged" => {
            let name = s(d, "inputName").to_string();
            live.inputs.insert(name.clone());
            if b(d, "inputMuted") {
                live.muted.insert(name);
            } else {
                live.muted.remove(&name);
            }
        }
        "SceneItemEnableStateChanged" => {
            let key = (
                s(d, "sceneName").to_string(),
                d.get("sceneItemId").and_then(Value::as_i64).unwrap_or(-1),
            );
            match t.items.get_mut(&key) {
                Some(item) => item.1 = b(d, "sceneItemEnabled"),
                None => e.reload = true,
            }
            t.sync_hidden();
        }
        "RecordStateChanged" => {
            let c = output_change(d);
            match c {
                // Starting and stopping report transitional states too; only settled ones count.
                OutputChange::Started if s(d, "outputState") != "OBS_WEBSOCKET_OUTPUT_STARTED" => {
                    e.changed = false;
                    return e;
                }
                OutputChange::Stopped if s(d, "outputState") != "OBS_WEBSOCKET_OUTPUT_STOPPED" => {
                    e.changed = false;
                    return e;
                }
                _ => {}
            }
            live.recording = c != OutputChange::Stopped;
            live.rec_paused = c == OutputChange::Paused;
            if c == OutputChange::Stopped {
                live.rec_secs = 0;
            }
            e.record = Some(c);
        }
        "StreamStateChanged" => {
            let c = output_change(d);
            if !matches!(
                s(d, "outputState"),
                "OBS_WEBSOCKET_OUTPUT_STARTED" | "OBS_WEBSOCKET_OUTPUT_STOPPED"
            ) {
                e.changed = false;
                return e;
            }
            live.streaming = c == OutputChange::Started;
            if !live.streaming {
                live.stream_secs = 0;
            }
            e.stream = Some(c);
        }
        "ReplayBufferStateChanged" => live.replay = b(d, "outputActive"),
        "VirtualcamStateChanged" => live.vcam = b(d, "outputActive"),
        "SceneNameChanged" => {
            let (old, new) = (s(d, "oldSceneName").to_string(), s(d, "sceneName").to_string());
            for sc in live.scenes.iter_mut().filter(|sc| **sc == old) {
                *sc = new.clone();
            }
            if live.program == old {
                live.program = new.clone();
            }
            if live.preview.as_deref() == Some(old.as_str()) {
                live.preview = Some(new.clone());
            }
            t.items = std::mem::take(&mut t.items)
                .into_iter()
                .map(|((sc, id), v)| ((if sc == old { new.clone() } else { sc }, id), v))
                .collect();
            t.sync_hidden();
        }
        "InputNameChanged" => {
            let (old, new) = (s(d, "oldInputName").to_string(), s(d, "inputName").to_string());
            if live.muted.remove(&old) {
                live.muted.insert(new.clone());
            }
            if live.inputs.remove(&old) {
                live.inputs.insert(new.clone());
            }
            for (name, _) in t.items.values_mut() {
                if *name == old {
                    *name = new.clone();
                }
            }
            // A scene used as a source inside another scene is renamed by SceneNameChanged.
            t.sync_hidden();
        }
        "SceneCreated"
        | "SceneRemoved"
        | "SceneItemCreated"
        | "SceneItemRemoved"
        | "SceneListChanged"
        | "CurrentSceneCollectionChanged"
        | "InputCreated" => e.reload = true,
        "InputRemoved" => {
            // Forget it at once; the reload (which refills `inputs` from scratch) confirms.
            let name = s(d, "inputName");
            live.inputs.remove(name);
            live.muted.remove(name);
            e.reload = true;
        }
        _ => e.changed = false,
    }
    e
}

/// Counts recording or streaming time locally, so timers need no requests.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stopwatch {
    acc: u64,
    since: Option<Instant>,
    running: bool,
}

impl Stopwatch {
    /// Start, `already` seconds in (when joining an output that runs).
    pub fn start(&mut self, now: Instant, already: u64) {
        self.acc = already;
        self.since = Some(now);
        self.running = true;
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(t) = self.since.take() {
            self.acc += now.saturating_duration_since(t).as_secs();
        }
    }

    pub fn resume(&mut self, now: Instant) {
        if self.running && self.since.is_none() {
            self.since = Some(now);
        }
    }

    pub fn stop(&mut self) {
        *self = Stopwatch::default();
    }

    pub fn secs(&self, now: Instant) -> u64 {
        if !self.running {
            return 0;
        }
        self.acc
            + self
                .since
                .map(|t| now.saturating_duration_since(t).as_secs())
                .unwrap_or(0)
    }
}

/// `h:mm:ss`, or `mm:ss` under an hour.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Most `mute.*` plus `visible.*` keys published (the host allows 256 per plugin, 16 are fixed).
pub const MAX_DYNAMIC_KEYS: usize = 200;

fn yes(b: bool) -> String {
    if b { "yes" } else { "no" }.to_string()
}

/// Every key value to publish (the host adds the plugin id): the fixed ones, then `mute.*` and
/// `visible.*` in name order, at most [`MAX_DYNAMIC_KEYS`] of those. Offline gives the offline values.
///
/// `previously` holds the dynamic keys published before; any of them no longer present (offline,
/// removed input, past the cap) comes out as `""` so stale values never linger.
pub fn state_pairs(live: &Live, previously: &BTreeSet<String>) -> Vec<(String, String)> {
    let on = live.connected;
    let when = |v: String| if on { v } else { String::new() };
    let flag = |b: bool| yes(on && b);
    let mut out: Vec<(String, String)> = vec![
        ("connected".into(), yes(on)),
        ("scene".into(), when(live.program.clone())),
        ("preview".into(), when(live.preview.clone().unwrap_or_default())),
        ("studio_mode".into(), flag(live.preview.is_some())),
        ("scene_count".into(), when(live.scenes.len().to_string())),
        ("recording".into(), flag(live.recording)),
        ("rec_paused".into(), flag(live.recording && live.rec_paused)),
        ("streaming".into(), flag(live.streaming)),
        ("replay".into(), flag(live.replay)),
        ("vcam".into(), flag(live.vcam)),
        (
            "rec_time".into(),
            if on && live.recording {
                clock(live.rec_secs)
            } else {
                String::new()
            },
        ),
        (
            "stream_time".into(),
            if on && live.streaming {
                clock(live.stream_secs)
            } else {
                String::new()
            },
        ),
    ];
    let stats = live.stats.as_ref().filter(|_| on);
    out.push(("fps".into(), stats.map_or(String::new(), |s| s.fps.to_string())));
    out.push(("cpu".into(), stats.map_or(String::new(), |s| s.cpu.to_string())));
    out.push((
        "dropped".into(),
        stats.map_or(String::new(), |s| {
            format!("{}.{}%", s.dropped_permille / 10, s.dropped_permille % 10)
        }),
    ));
    out.push(("last_error".into(), live.last_error.clone()));
    if on {
        let mut dynamic: Vec<(String, String)> = Vec::new();
        let inputs: BTreeSet<&String> = live.inputs.iter().chain(live.muted.iter()).collect();
        for name in inputs {
            dynamic.push((format!("mute.{name}"), yes(live.muted.contains(name))));
        }
        let sources: BTreeSet<&(String, String)> = live.sources.iter().chain(live.hidden.iter()).collect();
        for key in sources {
            dynamic.push((
                format!("visible.{}/{}", key.0, key.1),
                yes(!live.hidden.contains(key)),
            ));
        }
        dynamic.truncate(MAX_DYNAMIC_KEYS);
        out.extend(dynamic);
    }
    let present: BTreeSet<&str> = out.iter().map(|(k, _)| k.as_str()).collect();
    let gone: Vec<(String, String)> = previously
        .iter()
        .filter(|k| !present.contains(k.as_str()))
        .map(|k| (k.clone(), String::new()))
        .collect();
    out.extend(gone);
    out
}
