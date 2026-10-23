//! obs-websocket 5 without I/O: the handshake, turning deck actions into requests, and keeping the
//! [`ObsLive`] snapshot current from events.

use std::collections::BTreeMap;
use std::time::Instant;

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ulanzi_core::model::{ObsAction, VolumeOp};
use ulanzi_core::obs::ObsLive;

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

pub fn close_reason(code: u16) -> HandshakeError {
    match code {
        4009 => HandshakeError::AuthFailed,
        4010 => HandshakeError::TooOld,
        _ => HandshakeError::Dropped,
    }
}

pub fn request(id: &str, ty: &str, data: Value) -> Value {
    json!({"op": OP_REQUEST, "d": {"requestType": ty, "requestId": id, "requestData": data}})
}

/// What the client keeps besides the snapshot: scene items (to show/hide them by name).
#[derive(Debug, Clone, Default)]
pub struct Tracked {
    pub live: ObsLive,
    /// (scene, item id) → (source name, enabled).
    pub items: BTreeMap<(String, i64), (String, bool)>,
}

impl Tracked {
    /// Recompute the hidden set from the items.
    pub fn sync_hidden(&mut self) {
        self.live.hidden = self
            .items
            .iter()
            .filter(|(_, (_, on))| !on)
            .map(|((scene, _), (source, _))| (scene.clone(), source.clone()))
            .collect();
    }
}

/// How to carry out an action.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Requests(Vec<(String, Value)>),
    /// Read the input's volume, then set it ([`volume_db`]).
    InputVolume {
        input: String,
        op: VolumeOp,
        step: u8,
    },
    /// Read the filter's state, then flip it.
    ToggleFilter {
        source: String,
        filter: String,
    },
}

fn one(ty: &str, data: Value) -> Result<Plan, String> {
    Ok(Plan::Requests(vec![(ty.to_string(), data)]))
}

fn need(what: &str, v: &str) -> Result<(), String> {
    if v.trim().is_empty() {
        Err(format!("Choose {what} for this key first"))
    } else {
        Ok(())
    }
}

pub fn plan(a: &ObsAction, t: &Tracked) -> Result<Plan, String> {
    use ObsAction::*;
    match a {
        SetScene { scene, preview } => {
            need("a scene", scene)?;
            let ty = if *preview {
                "SetCurrentPreviewScene"
            } else {
                "SetCurrentProgramScene"
            };
            one(ty, json!({"sceneName": scene}))
        }
        ToggleScenes { a, b } => {
            need("two scenes", a)?;
            need("two scenes", b)?;
            let to = if t.live.program == *a { b } else { a };
            one("SetCurrentProgramScene", json!({"sceneName": to}))
        }
        ToggleSource { scene, source } => {
            need("a scene", scene)?;
            need("a source", source)?;
            let (id, on) = t
                .items
                .iter()
                .find(|((s, _), (name, _))| s == scene && name == source)
                .map(|((_, id), (_, on))| (*id, *on))
                .ok_or_else(|| format!("“{source}” is not in the scene “{scene}”"))?;
            one(
                "SetSceneItemEnabled",
                json!({"sceneName": scene, "sceneItemId": id, "sceneItemEnabled": !on}),
            )
        }
        ToggleRecord => one("ToggleRecord", json!({})),
        StartRecord => one("StartRecord", json!({})),
        StopRecord => one("StopRecord", json!({})),
        PauseRecord => one("ToggleRecordPause", json!({})),
        ToggleStream => one("ToggleStream", json!({})),
        StartStream => one("StartStream", json!({})),
        StopStream => one("StopStream", json!({})),
        ToggleReplay => one("ToggleReplayBuffer", json!({})),
        SaveReplay => one("SaveReplayBuffer", json!({})),
        ToggleVirtualCam => one("ToggleVirtualCam", json!({})),
        ToggleMute { input } => {
            need("an audio input", input)?;
            one("ToggleInputMute", json!({"inputName": input}))
        }
        InputVolume { input, op, step } => {
            need("an audio input", input)?;
            if *op == VolumeOp::Mute {
                return one("ToggleInputMute", json!({"inputName": input}));
            }
            Ok(Plan::InputVolume {
                input: input.clone(),
                op: *op,
                step: *step,
            })
        }
        ToggleFilter { source, filter } => {
            need("a source", source)?;
            need("a filter", filter)?;
            Ok(Plan::ToggleFilter {
                source: source.clone(),
                filter: filter.clone(),
            })
        }
        StudioTransition => one("TriggerStudioModeTransition", json!({})),
        SetCollection { name } => {
            need("a scene collection", name)?;
            one("SetCurrentSceneCollection", json!({"sceneCollectionName": name}))
        }
        SetProfile { name } => {
            need("an OBS profile", name)?;
            one("SetCurrentProfile", json!({"profileName": name}))
        }
        Screenshot => one(
            "TriggerHotkeyByName",
            json!({"hotkeyName": "OBSBasic.Screenshot"}),
        ),
    }
}

/// The new volume in dB: steps clamp to −100…0 dB; `Set` is percent of full (linear) volume.
pub fn volume_db(current_db: f64, op: VolumeOp, step: u8) -> f64 {
    let v = match op {
        VolumeOp::Up => current_db + step as f64,
        VolumeOp::Down => current_db - step as f64,
        VolumeOp::Set { percent: 0 } => -100.0,
        VolumeOp::Set { percent } => 20.0 * (percent.min(100) as f64 / 100.0).log10(),
        VolumeOp::Mute => current_db,
    };
    v.clamp(-100.0, 0.0)
}

/// A name changed in OBS; profiles that use the old one are rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rename {
    Scene(String, String),
    Input(String, String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventEffect {
    pub changed: bool,
    pub rename: Option<Rename>,
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
            e.rename = Some(Rename::Scene(old, new));
        }
        "InputNameChanged" => {
            let (old, new) = (s(d, "oldInputName").to_string(), s(d, "inputName").to_string());
            if live.muted.remove(&old) {
                live.muted.insert(new.clone());
            }
            for (name, _) in t.items.values_mut() {
                if *name == old {
                    *name = new.clone();
                }
            }
            // A scene used as a source inside another scene is renamed by SceneNameChanged.
            t.sync_hidden();
            e.rename = Some(Rename::Input(old, new));
        }
        "SceneCreated"
        | "SceneRemoved"
        | "SceneItemCreated"
        | "SceneItemRemoved"
        | "SceneListChanged"
        | "CurrentSceneCollectionChanged"
        | "InputCreated"
        | "InputRemoved" => e.reload = true,
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
