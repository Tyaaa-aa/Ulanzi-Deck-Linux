//! The connection to OBS: connect, identify, load the current state, follow events, run requests,
//! and reconnect with backoff (at once when OBS starts).

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use ulanzi_core::model::{ObsAction, ObsSettings};
use ulanzi_core::obs::{ObsLive, ObsStats};

use super::detect::ObsConfig;
use super::protocol::{self, HandshakeError, OutputChange, Plan, Rename, Stopwatch, Tracked};

/// The connection as the app explains it (spec §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsState {
    NotInstalled,
    NotRunning,
    ServerOff,
    WrongPort { ours: u16, obs: u16 },
    AuthMissing,
    AuthFailed,
    TooOld,
    Connecting,
    Connected { obs: String, ws: String, scenes: usize },
    Dropped,
}

impl ObsState {
    pub fn id(&self) -> &'static str {
        match self {
            ObsState::NotInstalled => "not-installed",
            ObsState::NotRunning => "not-running",
            ObsState::ServerOff => "server-off",
            ObsState::WrongPort { .. } => "wrong-port",
            ObsState::AuthMissing => "auth-missing",
            ObsState::AuthFailed => "auth-failed",
            ObsState::TooOld => "too-old",
            ObsState::Connecting => "connecting",
            ObsState::Connected { .. } => "connected",
            ObsState::Dropped => "dropped",
        }
    }

    pub fn title(&self) -> String {
        match self {
            ObsState::NotInstalled => "OBS Studio not found".into(),
            ObsState::NotRunning => "OBS is closed".into(),
            ObsState::ServerOff => "OBS’s WebSocket server is off".into(),
            ObsState::WrongPort { .. } => "Wrong port".into(),
            ObsState::AuthMissing => "Password needed".into(),
            ObsState::AuthFailed => "Wrong password".into(),
            ObsState::TooOld => "OBS 28 or newer needed".into(),
            ObsState::Connecting => "Connecting…".into(),
            ObsState::Connected { .. } => "Connected".into(),
            ObsState::Dropped => "Connection lost".into(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            ObsState::NotInstalled => "Install OBS Studio 28 or newer from obsproject.com.".into(),
            ObsState::NotRunning => "The deck connects as soon as OBS starts.".into(),
            ObsState::ServerOff => {
                "In OBS: Tools → WebSocket Server Settings → Enable WebSocket server.".into()
            }
            ObsState::WrongPort { ours, obs } => {
                format!(
                    "OBS listens on port {obs}, the deck uses {ours}. Use port {obs} from OBS’s settings."
                )
            }
            ObsState::AuthMissing => {
                "OBS asks for a password. Find OBS Settings fills it in, or type the one from OBS.".into()
            }
            ObsState::AuthFailed => {
                "The password doesn’t match OBS’s. Find OBS Settings reads it, or type it again.".into()
            }
            ObsState::TooOld => {
                "This OBS has no WebSocket 5 server. Update OBS to version 28 or newer.".into()
            }
            ObsState::Connecting => String::new(),
            ObsState::Connected { obs, ws, scenes } => {
                format!(
                    "OBS {obs} · WebSocket {ws} · {scenes} scene{}",
                    if *scenes == 1 { "" } else { "s" }
                )
            }
            ObsState::Dropped => "Reconnecting…".into(),
        }
    }
}

/// What the client needs to know about the machine (replaced in tests).
#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub installed: bool,
    pub running: bool,
    pub config: Option<ObsConfig>,
}

pub type ProbeFn = Arc<dyn Fn() -> Probe + Send + Sync>;

/// The real machine.
pub fn system_probe() -> ProbeFn {
    Arc::new(|| {
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let paths = super::detect::session_config_paths();
        let path = std::env::var("PATH").unwrap_or_default();
        Probe {
            installed: super::detect::obs_installed(&path, &home, &paths),
            running: super::detect::obs_running(std::path::Path::new("/proc")),
            config: super::detect::find_config(&paths),
        }
    })
}

/// How often things happen (shorter in tests).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// How often to look for an OBS process while disconnected.
    pub poll: Duration,
    pub timeout: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            poll: Duration::from_secs(2),
            timeout: Duration::from_secs(5),
        }
    }
}

enum Cmd {
    Action(ObsAction, oneshot::Sender<Result<(), String>>),
    Request(String, Value, oneshot::Sender<Result<Value, String>>),
    Settings(ObsSettings),
    Reconnect,
}

/// The app's side of the connection.
#[derive(Clone)]
pub struct ObsHandle {
    cmd: mpsc::Sender<Cmd>,
    status: watch::Receiver<ObsState>,
    live: watch::Receiver<ObsLive>,
}

impl ObsHandle {
    /// Start the connection task. Name changes in OBS arrive on `renames`.
    pub fn spawn(
        settings: ObsSettings,
        probe: ProbeFn,
        timing: Timing,
        renames: mpsc::UnboundedSender<Rename>,
    ) -> ObsHandle {
        let (cmd, rx) = mpsc::channel(64);
        let (status_tx, status) = watch::channel(ObsState::Connecting);
        let (live_tx, live) = watch::channel(ObsLive::default());
        tokio::spawn(
            Task {
                settings,
                probe,
                timing,
                status: status_tx,
                live: live_tx,
                renames,
            }
            .run(rx),
        );
        ObsHandle { cmd, status, live }
    }

    pub async fn action(&self, a: &ObsAction) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::Action(a.clone(), tx))
            .await
            .map_err(|_| "OBS support stopped".to_string())?;
        rx.await.map_err(|_| "OBS support stopped".to_string())?
    }

    pub async fn request(&self, ty: &str, data: Value) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::Request(ty.into(), data, tx))
            .await
            .map_err(|_| "OBS support stopped".to_string())?;
        rx.await.map_err(|_| "OBS support stopped".to_string())?
    }

    pub async fn set_settings(&self, s: ObsSettings) {
        let _ = self.cmd.send(Cmd::Settings(s)).await;
    }

    pub async fn reconnect(&self) {
        let _ = self.cmd.send(Cmd::Reconnect).await;
    }

    pub fn status(&self) -> ObsState {
        self.status.borrow().clone()
    }

    pub fn watch_status(&self) -> watch::Receiver<ObsState> {
        self.status.clone()
    }

    pub fn watch_live(&self) -> watch::Receiver<ObsLive> {
        self.live.clone()
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Why a session ended.
enum End {
    Refused,
    Handshake(HandshakeError),
    /// A reconnect was asked for.
    Restart,
    /// New connection settings.
    Settings(ObsSettings),
    /// The app is shutting down.
    Quit,
}

struct Task {
    settings: ObsSettings,
    probe: ProbeFn,
    timing: Timing,
    status: watch::Sender<ObsState>,
    live: watch::Sender<ObsLive>,
    renames: mpsc::UnboundedSender<Rename>,
}

impl Task {
    fn set_status(&self, s: ObsState) {
        self.status.send_if_modified(|cur| {
            if *cur == s {
                return false;
            }
            tracing::info!("OBS: {} {}", s.title(), s.detail());
            *cur = s;
            true
        });
    }

    async fn run(mut self, mut rx: mpsc::Receiver<Cmd>) {
        let mut backoff = self.timing.backoff_min;
        loop {
            let end = match self.connect().await {
                Ok(ws) => {
                    backoff = self.timing.backoff_min;
                    self.session(ws, &mut rx).await
                }
                Err(e) => e,
            };
            self.live.send_modify(|l| *l = ObsLive::default());
            let state = match end {
                End::Quit => return,
                End::Restart => continue,
                End::Settings(s) => {
                    self.settings = s;
                    backoff = self.timing.backoff_min;
                    continue;
                }
                End::Handshake(h) => match h {
                    HandshakeError::AuthMissing => ObsState::AuthMissing,
                    HandshakeError::AuthFailed => ObsState::AuthFailed,
                    HandshakeError::TooOld => ObsState::TooOld,
                    HandshakeError::Dropped => ObsState::Dropped,
                },
                End::Refused => self.explain_refused(),
            };
            self.set_status(state);
            match self.wait(&mut rx, backoff).await {
                Wait::Quit => return,
                Wait::Retry => {}
                Wait::Settings(s) => {
                    self.settings = s;
                    backoff = self.timing.backoff_min;
                    continue;
                }
            }
            backoff = (backoff * 2).min(self.timing.backoff_max);
        }
    }

    /// Nobody answered at host:port: say why.
    fn explain_refused(&self) -> ObsState {
        let p = (self.probe)();
        if !p.running {
            return if p.installed {
                ObsState::NotRunning
            } else {
                ObsState::NotInstalled
            };
        }
        match p.config {
            Some(c) if !c.enabled => ObsState::ServerOff,
            Some(c) if c.port != self.settings.port => ObsState::WrongPort {
                ours: self.settings.port,
                obs: c.port,
            },
            _ => ObsState::ServerOff,
        }
    }

    /// Wait before the next try: the backoff, a newly started OBS, or a command.
    async fn wait(&self, rx: &mut mpsc::Receiver<Cmd>, backoff: Duration) -> Wait {
        let until = tokio::time::Instant::now() + backoff;
        let mut was_running = (self.probe)().running;
        let mut poll = tokio::time::interval(self.timing.poll);
        poll.tick().await;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(until) => return Wait::Retry,
                _ = poll.tick() => {
                    let running = (self.probe)().running;
                    if running && !was_running {
                        return Wait::Retry;
                    }
                    was_running = running;
                }
                c = rx.recv() => match c {
                    None => return Wait::Quit,
                    Some(Cmd::Action(_, reply)) => {
                        let _ = reply.send(Err(self.offline()));
                    }
                    Some(Cmd::Request(_, _, reply)) => {
                        let _ = reply.send(Err(self.offline()));
                    }
                    Some(Cmd::Settings(s)) => return Wait::Settings(s),
                    Some(Cmd::Reconnect) => return Wait::Retry,
                },
            }
        }
    }

    fn offline(&self) -> String {
        let s = self.status.borrow();
        format!("OBS isn’t connected: {}", s.title())
    }

    async fn connect(&self) -> Result<Ws, End> {
        let url = format!("ws://{}:{}", self.settings.host, self.settings.port);
        let ws = match tokio::time::timeout(self.timing.timeout, tokio_tungstenite::connect_async(&url)).await
        {
            Ok(Ok((ws, _))) => ws,
            _ => return Err(End::Refused),
        };
        let mut ws = ws;
        let hello = match next_json(&mut ws, self.timing.timeout).await {
            Ok(v) => v,
            Err(e) => return Err(End::Handshake(e)),
        };
        let identify =
            protocol::identify(&hello, self.settings.password.as_deref()).map_err(End::Handshake)?;
        ws.send(Message::text(identify.to_string()))
            .await
            .map_err(|_| End::Handshake(HandshakeError::Dropped))?;
        loop {
            let v = next_json(&mut ws, self.timing.timeout)
                .await
                .map_err(End::Handshake)?;
            if v.get("op").and_then(Value::as_u64) == Some(protocol::OP_IDENTIFIED) {
                return Ok(ws);
            }
        }
    }

    async fn session(&self, ws: Ws, rx: &mut mpsc::Receiver<Cmd>) -> End {
        let mut c = Conn {
            ws,
            tracked: Tracked::default(),
            next_id: 0,
            rec: Stopwatch::default(),
            stream: Stopwatch::default(),
            renames: self.renames.clone(),
            timeout: self.timing.timeout,
            reload: false,
            dead: false,
            close: None,
        };
        let state = match c.load().await {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.set_status(state);
        self.publish(&mut c);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut stats = tokio::time::interval(Duration::from_secs(2));
        loop {
            let reload = std::mem::take(&mut c.reload);
            if reload {
                match c.load().await {
                    Ok(s) => self.set_status(s),
                    Err(e) => return e,
                }
                self.publish(&mut c);
            }
            tokio::select! {
                msg = c.ws.next() => match c.incoming(msg) {
                    Ok(true) => self.publish(&mut c),
                    Ok(false) => {}
                    Err(e) => return e,
                },
                cmd = rx.recv() => match cmd {
                    None => return End::Quit,
                    Some(Cmd::Action(a, reply)) => {
                        let r = c.action(&a).await;
                        let _ = reply.send(r);
                        self.publish(&mut c);
                    }
                    Some(Cmd::Request(ty, data, reply)) => {
                        let r = c.call(&ty, data).await;
                        let _ = reply.send(r);
                    }
                    Some(Cmd::Settings(s)) => return End::Settings(s),
                    Some(Cmd::Reconnect) => return End::Restart,
                },
                _ = tick.tick() => {
                    if c.tracked.live.recording || c.tracked.live.streaming {
                        self.publish(&mut c);
                    }
                }
                _ = stats.tick() => {
                    if let Ok(v) = c.call("GetStats", json!({})).await {
                        c.tracked.live.stats = Some(stats_from(&v));
                        self.publish(&mut c);
                    }
                }
            }
            if c.dead {
                return End::Handshake(c.close.clone().unwrap_or(HandshakeError::Dropped));
            }
        }
    }

    /// Send the snapshot (with the timers brought up to date) if it changed.
    fn publish(&self, c: &mut Conn) {
        let now = Instant::now();
        c.tracked.live.connected = true;
        c.tracked.live.rec_secs = c.rec.secs(now);
        c.tracked.live.stream_secs = c.stream.secs(now);
        let snap = c.tracked.live.clone();
        self.live.send_if_modified(|l| {
            if *l == snap {
                return false;
            }
            *l = snap;
            true
        });
    }
}

enum Wait {
    Retry,
    Quit,
    Settings(ObsSettings),
}

fn stats_from(v: &Value) -> ObsStats {
    let f = |k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let total = f("outputTotalFrames") + f("renderTotalFrames");
    let skipped = f("outputSkippedFrames") + f("renderSkippedFrames");
    ObsStats {
        fps: f("activeFps").round() as u32,
        cpu: f("cpuUsage").round() as u32,
        dropped_permille: if total > 0.0 {
            (skipped / total * 1000.0).round() as u32
        } else {
            0
        },
    }
}

/// The next JSON message (pings and the like skipped); a close frame becomes its reason.
async fn next_json(ws: &mut Ws, timeout: Duration) -> Result<Value, HandshakeError> {
    loop {
        let msg = match tokio::time::timeout(timeout, ws.next()).await {
            Ok(Some(Ok(m))) => m,
            _ => return Err(HandshakeError::Dropped),
        };
        match msg {
            Message::Text(t) => return serde_json::from_str(&t).map_err(|_| HandshakeError::TooOld),
            Message::Close(f) => {
                return Err(f
                    .map(|f| protocol::close_reason(u16::from(f.code)))
                    .unwrap_or(HandshakeError::Dropped));
            }
            _ => {}
        }
    }
}

/// One identified session.
struct Conn {
    ws: Ws,
    tracked: Tracked,
    next_id: u64,
    rec: Stopwatch,
    stream: Stopwatch,
    renames: mpsc::UnboundedSender<Rename>,
    timeout: Duration,
    /// Scenes or items changed shape: load again.
    reload: bool,
    /// The socket closed (with its reason).
    dead: bool,
    close: Option<HandshakeError>,
}

impl Conn {
    /// Fetch everything the deck shows: version, scenes, outputs, inputs' mute, scene items.
    async fn load(&mut self) -> Result<ObsState, End> {
        let fail = |_: String| End::Handshake(HandshakeError::Dropped);
        let version = self.call("GetVersion", json!({})).await.map_err(fail)?;
        let scenes = self.call("GetSceneList", json!({})).await.map_err(fail)?;
        let now = Instant::now();
        let mut t = Tracked::default();
        t.live.connected = true;
        t.live.program = str_of(&scenes, "currentProgramSceneName");
        let mut names: Vec<String> = scenes
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
        t.live.scenes = names.clone();
        if let Ok(v) = self.call("GetStudioModeEnabled", json!({})).await
            && v.get("studioModeEnabled").and_then(Value::as_bool) == Some(true)
        {
            t.live.preview = Some(str_of(&scenes, "currentPreviewSceneName"));
        }
        if let Ok(v) = self.call("GetRecordStatus", json!({})).await {
            t.live.recording = bool_of(&v, "outputActive");
            t.live.rec_paused = bool_of(&v, "outputPaused");
            if t.live.recording {
                self.rec.start(now, ms_of(&v, "outputDuration") / 1000);
                if t.live.rec_paused {
                    self.rec.pause(now);
                }
            } else {
                self.rec.stop();
            }
        }
        if let Ok(v) = self.call("GetStreamStatus", json!({})).await {
            t.live.streaming = bool_of(&v, "outputActive");
            if t.live.streaming {
                self.stream.start(now, ms_of(&v, "outputDuration") / 1000);
            } else {
                self.stream.stop();
            }
        }
        if let Ok(v) = self.call("GetReplayBufferStatus", json!({})).await {
            t.live.replay = bool_of(&v, "outputActive");
        }
        if let Ok(v) = self.call("GetVirtualCamStatus", json!({})).await {
            t.live.vcam = bool_of(&v, "outputActive");
        }
        if let Ok(v) = self.call("GetInputList", json!({})).await {
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
            for i in inputs {
                // Inputs without audio answer with an error; they can't be muted anyway.
                if let Ok(m) = self.call("GetInputMute", json!({"inputName": i})).await
                    && bool_of(&m, "inputMuted")
                {
                    t.live.muted.insert(i);
                }
            }
        }
        for scene in &names {
            if let Ok(v) = self.call("GetSceneItemList", json!({"sceneName": scene})).await {
                for item in v
                    .get("sceneItems")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let id = item.get("sceneItemId").and_then(Value::as_i64).unwrap_or(-1);
                    t.items.insert(
                        (scene.clone(), id),
                        (str_of(item, "sourceName"), bool_of(item, "sceneItemEnabled")),
                    );
                }
            }
        }
        t.sync_hidden();
        t.live.stats = self.tracked.live.stats.clone();
        self.tracked = t;
        Ok(ObsState::Connected {
            obs: str_of(&version, "obsVersion"),
            ws: str_of(&version, "obsWebSocketVersion"),
            scenes: names.len(),
        })
    }

    async fn action(&mut self, a: &ObsAction) -> Result<(), String> {
        match protocol::plan(a, &self.tracked)? {
            Plan::Requests(list) => {
                for (ty, data) in list {
                    self.call(&ty, data).await?;
                }
            }
            Plan::InputVolume { input, op, step } => {
                let cur = self
                    .call("GetInputVolume", json!({"inputName": input}))
                    .await?
                    .get("inputVolumeDb")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let db = protocol::volume_db(cur, op, step);
                self.call("SetInputVolume", json!({"inputName": input, "inputVolumeDb": db}))
                    .await?;
            }
            Plan::ToggleFilter { source, filter } => {
                let on = self
                    .call(
                        "GetSourceFilter",
                        json!({"sourceName": source, "filterName": filter}),
                    )
                    .await?
                    .get("filterEnabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.call(
                    "SetSourceFilterEnabled",
                    json!({"sourceName": source, "filterName": filter, "filterEnabled": !on}),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Send a request and wait for its answer, handling events that arrive meanwhile.
    async fn call(&mut self, ty: &str, data: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = format!("d{}", self.next_id);
        self.ws
            .send(Message::text(protocol::request(&id, ty, data).to_string()))
            .await
            .map_err(|_| {
                self.dead = true;
                "The connection to OBS was lost".to_string()
            })?;
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let msg = match tokio::time::timeout_at(deadline, self.ws.next()).await {
                Ok(m) => m,
                Err(_) => return Err(format!("OBS didn’t answer {ty}")),
            };
            let Some(Ok(Message::Text(t))) = msg else {
                self.incoming(msg).ok();
                if self.dead {
                    return Err("The connection to OBS was lost".into());
                }
                continue;
            };
            let v: Value = serde_json::from_str(&t).unwrap_or_default();
            let d = v.get("d").cloned().unwrap_or_default();
            if v.get("op").and_then(Value::as_u64) == Some(protocol::OP_RESPONSE)
                && d.get("requestId").and_then(Value::as_str) == Some(id.as_str())
            {
                let status = d.get("requestStatus").cloned().unwrap_or_default();
                if status.get("result").and_then(Value::as_bool) == Some(true) {
                    return Ok(d.get("responseData").cloned().unwrap_or(json!({})));
                }
                let comment = status
                    .get("comment")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        format!(
                            "OBS refused {ty} (code {})",
                            status.get("code").and_then(Value::as_i64).unwrap_or(0)
                        )
                    });
                return Err(comment);
            }
            self.handle(&v);
        }
    }

    /// A message from the socket outside a request. Ok(true) when the snapshot changed.
    fn incoming(
        &mut self,
        msg: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) -> Result<bool, End> {
        match msg {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(&t).unwrap_or_default();
                Ok(self.handle(&v))
            }
            Some(Ok(Message::Close(f))) => {
                self.dead = true;
                let code = f
                    .map(|f| u16::from(f.code))
                    .unwrap_or(u16::from(CloseCode::Normal));
                self.close = Some(protocol::close_reason(code));
                Err(End::Handshake(protocol::close_reason(code)))
            }
            Some(Ok(_)) => Ok(false),
            _ => {
                self.dead = true;
                Err(End::Handshake(HandshakeError::Dropped))
            }
        }
    }

    /// Apply an event. True when the snapshot changed.
    fn handle(&mut self, v: &Value) -> bool {
        if v.get("op").and_then(Value::as_u64) != Some(protocol::OP_EVENT) {
            return false;
        }
        let d = v.get("d").cloned().unwrap_or_default();
        let ty = d.get("eventType").and_then(Value::as_str).unwrap_or_default();
        let data = d.get("eventData").cloned().unwrap_or(json!({}));
        let e = protocol::apply_event(&mut self.tracked, ty, &data);
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
        if let Some(r) = e.rename {
            let _ = self.renames.send(r);
        }
        if e.reload {
            self.reload = true;
        }
        e.changed
    }
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn bool_of(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn ms_of(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u64
}
