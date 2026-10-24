//! Home Assistant as a plugin: toggle, run and set entities and call any service, with live entity
//! state on key labels.
//!
//! One WebSocket to `/api/websocket`. Every handler only sends and returns (the host allows 500 ms
//! per call); answers arrive as `message` events and are matched to what was asked by command id.
//! Only the entities named in the settings are subscribed to, never the whole install. For the
//! pickers one `render_template` subscription keeps a bounded list of every entity (id, name and
//! state) in memory; it is never published as state.
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde_json::Value;
use ulanzi_plugin::{Cx, Item, Level, Payload, Plugin, Socket, Status, Timer};

use crate::actions::{Plan, ServiceCall, plan};
use crate::protocol::{
    Config, EntityList, HOST_KEY_BYTES, HOST_STATE_KEYS, HOST_VALUE_BYTES, LiveInfo, MAX_ENTITIES, Tracked,
    auth_msg, call_service_msg, offline_pairs, parse_entity_list, parse_settings, picker_items, ping_msg,
    render_template_msg, state_pairs, subscribe_msg, valid_entity_id,
};

pub mod actions;
pub mod protocol;

const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
/// From opening the socket until the first snapshot has arrived.
const HANDSHAKE: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(1000);
/// A command unanswered after this many ticks is dropped.
const PENDING_TICKS: u64 = 5;
/// Service calls get longer (30 s): Home Assistant answers once the service has run, and a
/// script or a slow device can take a while.
const SERVICE_TICKS: u64 = 30;
/// A ping is sent every this many ticks; two unanswered ones mean the socket is dead.
const PING_EVERY_TICKS: u64 = 15;
const MAX_MISSED_PINGS: u32 = 2;
/// Longest error text from Home Assistant kept for the pickers.
const LIST_ERROR_BYTES: usize = 500;
const NOT_CONNECTED: &str = "Not connected to Home Assistant";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Opening(Socket),
    AwaitAuthRequired(Socket),
    Authenticating(Socket),
    Subscribing { socket: Socket, sub_id: u64 },
    Live { socket: Socket, sub_id: u64 },
}

impl Phase {
    fn socket(self) -> Option<Socket> {
        match self {
            Phase::Idle => None,
            Phase::Opening(s) | Phase::AwaitAuthRequired(s) | Phase::Authenticating(s) => Some(s),
            Phase::Subscribing { socket, .. } | Phase::Live { socket, .. } => Some(socket),
        }
    }
}

/// Why a connection is down; picks the status shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Down {
    Refused,
    Dropped,
    NoToken,
    BadToken,
    TooOld,
    NotAllowed,
    /// Home Assistant answered the subscription with an error (its text is in `last_error`).
    Rejected,
}

impl Down {
    fn status(self, addr: &str) -> (Status, String) {
        let (status, text) = match self {
            Down::Refused => (
                Status::Problem,
                format!("Cannot reach Home Assistant at {addr}. Check the address and that it is running."),
            ),
            Down::Dropped => (Status::Connecting, "Connection lost. Reconnecting…".to_string()),
            Down::NoToken => (
                Status::Problem,
                "Access token needed. Enter it in Settings → Integrations → Home Assistant.".to_string(),
            ),
            Down::BadToken => (
                Status::Problem,
                "Home Assistant rejected the access token. Create a long-lived access token (HA: your Profile → Security → Long-lived access tokens) and enter it in Settings → Integrations → Home Assistant.".to_string(),
            ),
            Down::TooOld => (
                Status::Problem,
                "Home Assistant 2022.4 or newer needed.".to_string(),
            ),
            Down::NotAllowed => (
                Status::Problem,
                "Home Assistant must be on your local network (a name ending in .local, or a local IP address)."
                    .to_string(),
            ),
            Down::Rejected => (
                Status::Problem,
                "Home Assistant refused the entity subscription (see the last_error value). Retrying…"
                    .to_string(),
            ),
        };
        (status, text)
    }

    /// Whether to try again by itself. A missing or rejected token, an old Home Assistant and a
    /// forbidden address wait for a settings change or the reconnect action instead.
    fn retries(self) -> bool {
        matches!(self, Down::Refused | Down::Dropped | Down::Rejected)
    }
}

#[derive(Debug, Clone)]
enum Kind {
    Subscribe,
    /// The entity list (`render_template`).
    List,
    Service {
        label: String,
    },
    Ping,
}

#[derive(Debug, Clone)]
struct Pending {
    kind: Kind,
    sent_tick: u64,
}

impl Pending {
    /// Ticks to wait for the answer.
    fn patience(&self) -> u64 {
        match self.kind {
            Kind::Service { .. } => SERVICE_TICKS,
            Kind::Subscribe | Kind::List | Kind::Ping => PENDING_TICKS,
        }
    }
}

/// The entity list of the current connection, for the pickers.
#[derive(Debug, Clone, Default)]
enum Listing {
    /// Asked for (or not yet), nothing rendered yet.
    #[default]
    Waiting,
    Ready(EntityList),
    /// Home Assistant's error text.
    Failed(String),
}

pub struct Ha {
    config: Option<Config>,
    phase: Phase,
    backoff: Duration,
    handshake: Option<Timer>,
    retry: Option<Timer>,
    tick: Option<Timer>,
    /// Command ids are per connection and strictly increasing, starting at 1.
    next_id: u64,
    ticks: u64,
    pending: BTreeMap<u64, Pending>,
    tracked: Tracked,
    live: LiveInfo,
    missed_pings: u32,
    /// The id of this connection's `render_template` subscription.
    list_id: Option<u64>,
    listing: Listing,
    truncation_logged: bool,
    /// Values last sent to the host, by key.
    sent: BTreeMap<String, String>,
    /// Keys currently holding a value (and entity states), so going offline can blank them.
    published: BTreeSet<String>,
    /// Every key ever set on the host; the host's 256-key cap counts these for the whole life
    /// of the instance, so a new key is only published while the total stays under it.
    ever_published: BTreeSet<String>,
    key_cap_warned: bool,
    key_size_warned: bool,
    status: Option<(Status, String)>,
    /// The last reason the host gave for a failed or closed socket, so each is logged once.
    last_reason: String,
}

impl Default for Ha {
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
            pending: BTreeMap::new(),
            tracked: Tracked::default(),
            live: LiveInfo::default(),
            missed_pings: 0,
            list_id: None,
            listing: Listing::Waiting,
            truncation_logged: false,
            sent: BTreeMap::new(),
            published: BTreeSet::new(),
            ever_published: BTreeSet::new(),
            key_cap_warned: false,
            key_size_warned: false,
            status: None,
            last_reason: String::new(),
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

/// `host:port` of a WebSocket endpoint, for messages.
fn address_of(ws_url: &str) -> String {
    let rest = ws_url.split_once("://").map_or(ws_url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// The entities to subscribe to: the listed ones plus those only used by an attribute.
fn subscription_ids(cfg: &Config) -> Vec<String> {
    let mut ids = cfg.entities.clone();
    for (id, _) in &cfg.attributes {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids
}

impl Ha {
    fn address(&self) -> String {
        self.config
            .as_ref()
            .map(|c| address_of(&c.url))
            .unwrap_or_default()
    }

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

    /// Log why the host failed or closed a socket, once per distinct reason. Host reasons are
    /// about the transport (DNS, TLS, size limits) and never contain the token.
    fn note_reason(&mut self, cx: &mut Cx, reason: &str) {
        let reason = reason.trim();
        if reason.is_empty() || self.last_reason == reason {
            return;
        }
        self.last_reason = reason.to_string();
        cx.log(
            Level::Warn,
            &format!(
                "Connection to Home Assistant failed: {}",
                truncate_at_boundary(reason.to_string(), 300)
            ),
        );
    }

    fn connect(&mut self, cx: &mut Cx, announce: bool) {
        let Some(config) = self.config.clone() else { return };
        if let Some(t) = self.retry.take() {
            cx.cancel_timer(t);
        }
        if config.token.is_none() {
            self.go_down(cx, Down::NoToken, false);
            return;
        }
        if announce {
            let msg = format!("Connecting to Home Assistant at {}…", address_of(&config.url));
            self.set_status(cx, Status::Connecting, &msg);
        }
        match cx.ws_open(&config.url) {
            Ok(s) => {
                self.phase = Phase::Opening(s);
                self.handshake = Some(cx.set_timer(HANDSHAKE, false));
            }
            Err(e) => {
                self.note_reason(cx, &e);
                let why = if e.starts_with("not allowed") {
                    Down::NotAllowed
                } else {
                    Down::Refused
                };
                self.go_down(cx, why, false);
            }
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
        // A fresh Tracked per connection: nothing of the old one may leak into the next.
        self.tracked = Tracked::default();
        self.live.connected = false;
        self.live.version.clear();
        self.next_id = 0;
        self.ticks = 0;
        self.missed_pings = 0;
        // The list belongs to the connection, like the tracked entities.
        self.list_id = None;
        self.listing = Listing::Waiting;
        self.publish(cx);
    }

    /// The connection is down: show why, reset the state and schedule the next try.
    fn go_down(&mut self, cx: &mut Cx, why: Down, close: bool) {
        self.teardown(cx, close);
        let (status, message) = why.status(&self.address());
        self.set_status(cx, status, &message);
        if !why.retries() {
            return;
        }
        self.retry = Some(cx.set_timer(self.backoff, false));
        self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
    }

    /// Send what changed since last time.
    fn publish(&mut self, cx: &mut Cx) {
        let pairs = match &self.config {
            Some(cfg) if self.live.connected => state_pairs(&self.tracked, cfg, &self.live),
            _ => {
                let mut keys = self.published.clone();
                keys.insert("connected".to_string());
                offline_pairs(&keys)
            }
        };
        self.set_pairs(cx, pairs);
    }

    fn set_pairs(&mut self, cx: &mut Cx, pairs: Vec<(String, String)>) {
        for (key, value) in pairs {
            let value = truncate_at_boundary(value, HOST_VALUE_BYTES);
            if self.sent.get(&key) != Some(&value) {
                if !self.may_publish(cx, &key) {
                    continue;
                }
                cx.set_state(&key, value.clone());
                self.sent.insert(key.clone(), value.clone());
            }
            if !value.is_empty() || valid_entity_id(&key) {
                self.published.insert(key);
            } else {
                self.published.remove(&key);
            }
        }
    }

    fn set_last_error(&mut self, cx: &mut Cx, text: &str) {
        if self.live.last_error != text {
            self.live.last_error = text.to_string();
            self.set_pairs(cx, vec![("last_error".to_string(), text.to_string())]);
        }
    }

    /// Whether the host will accept `key`: not over its size limit, and either already known to
    /// it or still under its lifetime key cap. Warns once for each reason otherwise.
    fn may_publish(&mut self, cx: &mut Cx, key: &str) -> bool {
        if key.len() > HOST_KEY_BYTES {
            self.warn_long_key(cx, key);
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
                        "All {HOST_STATE_KEYS} state keys the host allows this plugin are used (entities you stopped listing keep theirs until the plugin restarts); new {{ha.…}} values are not published until you disable and enable the plugin"
                    ),
                );
            }
            return false;
        }
        self.ever_published.insert(key.to_string());
        true
    }

    fn warn_long_key(&mut self, cx: &mut Cx, key: &str) {
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
    }

    /// Warn once about keys of the new settings that can never be published.
    fn check_key_sizes(&mut self, cx: &mut Cx, cfg: &Config) {
        let mut keys: Vec<String> = Vec::new();
        for id in &cfg.entities {
            keys.extend(["", ".name", ".unit", ".pct"].map(|s| format!("{id}{s}")));
        }
        keys.extend(cfg.attributes.iter().map(|(id, a)| format!("{id}.attr.{a}")));
        if let Some(long) = keys.iter().find(|k| k.len() > HOST_KEY_BYTES) {
            self.warn_long_key(cx, long);
        }
    }

    fn update_ok_status(&mut self, cx: &mut Cx) {
        let Some(cfg) = &self.config else { return };
        let n = cfg
            .entities
            .iter()
            .filter(|e| self.tracked.entities.contains_key(*e))
            .count();
        let msg = format!(
            "Home Assistant {} · {n} entit{}",
            self.live.version,
            if n == 1 { "y" } else { "ies" }
        );
        self.set_status(cx, Status::Ok, &msg);
    }

    fn offline_error(&self) -> String {
        let title = match &self.status {
            Some((_, m)) => m
                .split_once(". ")
                .map_or(m.as_str(), |(t, _)| t)
                .trim_end_matches('.'),
            None => "not started",
        };
        format!("Home Assistant isn’t connected: {title}")
    }

    /// Send one command of the live connection; the id is taken here.
    fn send_command(
        &mut self,
        cx: &mut Cx,
        socket: Socket,
        build: impl FnOnce(u64) -> Value,
        kind: Kind,
    ) -> Result<(), String> {
        self.next_id += 1;
        let id = self.next_id;
        let text = build(id).to_string();
        if cx.send(socket, Payload::Text(text)).is_err() {
            // The host lost the socket (its close notice may have been dropped): without this
            // the plugin would stay "connected" with frozen labels.
            self.go_down(cx, Down::Dropped, true);
            return Err(self.offline_error());
        }
        self.pending.insert(
            id,
            Pending {
                kind,
                sent_tick: self.ticks,
            },
        );
        Ok(())
    }

    fn call_service(&mut self, cx: &mut Cx, call: &ServiceCall) -> Result<(), String> {
        let Phase::Live { socket, .. } = self.phase else {
            return Err(self.offline_error());
        };
        let label = format!("{}.{}", call.domain, call.service);
        self.send_command(
            cx,
            socket,
            |id| call_service_msg(id, call),
            Kind::Service { label },
        )
    }

    fn handle_text(&mut self, cx: &mut Cx, socket: Socket, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or_default();
        match self.phase {
            Phase::AwaitAuthRequired(s) if s == socket && ty == "auth_required" => {
                let Some(token) = self.config.as_ref().and_then(|c| c.token.clone()) else {
                    self.go_down(cx, Down::NoToken, true);
                    return;
                };
                if cx
                    .send(socket, Payload::Text(auth_msg(&token).to_string()))
                    .is_err()
                {
                    self.go_down(cx, Down::Dropped, true);
                } else {
                    self.phase = Phase::Authenticating(socket);
                }
            }
            Phase::Authenticating(s) if s == socket => match ty {
                "auth_ok" => {
                    self.live.version = v
                        .get("ha_version")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    self.subscribe(cx, socket);
                }
                "auth_invalid" => self.go_down(cx, Down::BadToken, true),
                _ => {}
            },
            Phase::Subscribing { socket: s, sub_id } | Phase::Live { socket: s, sub_id } if s == socket => {
                // Anything from the server shows the socket is alive.
                self.missed_pings = 0;
                match ty {
                    "result" => self.handle_result(cx, &v),
                    "pong" => {
                        if let Some(id) = v.get("id").and_then(Value::as_u64) {
                            self.pending.remove(&id);
                        }
                    }
                    "event"
                        if self.list_id.is_some() && v.get("id").and_then(Value::as_u64) == self.list_id =>
                    {
                        self.handle_list_event(cx, &v);
                    }
                    "event" if v.get("id").and_then(Value::as_u64) == Some(sub_id) => {
                        self.handle_event(cx, socket, sub_id, &v);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn subscribe(&mut self, cx: &mut Cx, socket: Socket) {
        let ids = self.config.as_ref().map(subscription_ids).unwrap_or_default();
        if ids.is_empty() {
            // An empty `entity_ids` would mean every entity on the install: do not ask at all.
            self.go_live(cx, socket, 0);
            self.refresh(cx);
            self.subscribe_list(cx, socket);
            return;
        }
        let id = self.next_id + 1;
        if self
            .send_command(
                cx,
                socket,
                |id| subscribe_msg(id, &ids).unwrap_or(Value::Null),
                Kind::Subscribe,
            )
            .is_ok()
        {
            self.phase = Phase::Subscribing { socket, sub_id: id };
            self.subscribe_list(cx, socket);
        }
    }

    /// Ask once per connection for the entity list the pickers show.
    fn subscribe_list(&mut self, cx: &mut Cx, socket: Socket) {
        if self.phase.socket() != Some(socket) || self.list_id.is_some() {
            return;
        }
        let id = self.next_id + 1;
        if self
            .send_command(cx, socket, render_template_msg, Kind::List)
            .is_ok()
        {
            self.list_id = Some(id);
        }
    }

    /// A render of the entity list: `{"result": "<lines>"}`, or an error.
    fn handle_list_event(&mut self, cx: &mut Cx, v: &Value) {
        let event = v.get("event").unwrap_or(&Value::Null);
        let error = event
            .get("error")
            .or_else(|| event.get("result").and_then(|r| r.get("error")));
        if let Some(e) = error {
            let text = match e {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            self.list_failed(cx, &text);
            return;
        }
        let list = match event.get("result") {
            Some(Value::String(text)) => parse_entity_list(text),
            // Home Assistant turns output that looks like a number or a list into one; ours never
            // does, except an install with nothing to list.
            Some(_) => EntityList::default(),
            None => return,
        };
        if list.truncated && !self.truncation_logged {
            self.truncation_logged = true;
            cx.log(
                Level::Info,
                &format!(
                    "Home Assistant has more entities than the entity list holds: it was cut after {} (lights, switches and other controllable ones come first)",
                    list.rows.len()
                ),
            );
        }
        self.listing = Listing::Ready(list);
    }

    fn list_failed(&mut self, cx: &mut Cx, text: &str) {
        let text = truncate_at_boundary(text.trim().to_string(), LIST_ERROR_BYTES);
        let text = if text.is_empty() {
            "Home Assistant could not list its entities".to_string()
        } else {
            text
        };
        if !matches!(&self.listing, Listing::Failed(t) if *t == text) {
            cx.log(
                Level::Warn,
                &format!("Home Assistant could not list its entities: {text}"),
            );
        }
        self.listing = Listing::Failed(text);
    }

    fn go_live(&mut self, cx: &mut Cx, socket: Socket, sub_id: u64) {
        if let Some(t) = self.handshake.take() {
            cx.cancel_timer(t);
        }
        // Only now is the subscription confirmed (success result or first snapshot): a server that keeps failing before this point
        // keeps backing off.
        self.backoff = FIRST_BACKOFF;
        self.phase = Phase::Live { socket, sub_id };
        self.tick = Some(cx.set_timer(TICK, true));
        self.live.connected = true;
        self.last_reason.clear();
        self.set_last_error(cx, "");
    }

    fn refresh(&mut self, cx: &mut Cx) {
        self.update_ok_status(cx);
        self.publish(cx);
    }

    fn handle_event(&mut self, cx: &mut Cx, socket: Socket, sub_id: u64, v: &Value) {
        let first = matches!(self.phase, Phase::Subscribing { .. });
        let changed = self.tracked.apply(v.get("event").unwrap_or(&Value::Null));
        if first {
            self.go_live(cx, socket, sub_id);
        }
        if first || changed {
            self.refresh(cx);
        }
    }

    fn handle_result(&mut self, cx: &mut Cx, v: &Value) {
        let Some(p) = v
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|id| self.pending.remove(&id))
        else {
            return;
        };
        let ok = v.get("success").and_then(Value::as_bool).unwrap_or(false);
        let error = v.get("error");
        let text = |k: &str| {
            error
                .and_then(|e| e.get(k))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        match p.kind {
            Kind::Subscribe => {
                if !ok {
                    if text("code").as_deref() == Some("unknown_command") {
                        cx.log(Level::Warn, "Home Assistant does not know subscribe_entities");
                        self.go_down(cx, Down::TooOld, true);
                    } else {
                        let why = text("message")
                            .unwrap_or_else(|| "Home Assistant refused the entity subscription".to_string());
                        cx.log(Level::Warn, &why);
                        self.set_last_error(cx, &why);
                        self.go_down(cx, Down::Rejected, true);
                    }
                } else if let Phase::Subscribing { socket, sub_id } = self.phase {
                    // None of the listed entities may exist, and then no snapshot follows.
                    self.go_live(cx, socket, sub_id);
                    self.refresh(cx);
                }
            }
            Kind::List => {
                if !ok {
                    let why = text("message").unwrap_or_default();
                    self.list_failed(cx, &why);
                }
            }
            Kind::Service { label } => {
                if ok {
                    self.set_last_error(cx, "");
                } else {
                    let why = text("message").unwrap_or_else(|| format!("Home Assistant refused {label}"));
                    cx.log(Level::Warn, &why);
                    self.set_last_error(cx, &why);
                }
            }
            Kind::Ping => {}
        }
    }

    fn on_tick(&mut self, cx: &mut Cx) {
        let Phase::Live { socket, .. } = self.phase else {
            return;
        };
        self.ticks += 1;
        let ticks = self.ticks;
        let stale: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| ticks - p.sent_tick > p.patience())
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            match self.pending.remove(&id).map(|p| p.kind) {
                // Only what a key press asked for is worth a log line.
                Some(Kind::Service { label }) => {
                    cx.log(Level::Warn, &format!("Home Assistant didn’t answer {label}"));
                }
                Some(Kind::Ping) => self.missed_pings += 1,
                Some(Kind::Subscribe | Kind::List) | None => {}
            }
        }
        if self.missed_pings >= MAX_MISSED_PINGS {
            cx.log(Level::Warn, "Home Assistant stopped answering pings");
            self.go_down(cx, Down::Dropped, true);
            return;
        }
        if ticks.is_multiple_of(PING_EVERY_TICKS) {
            let _ = self.send_command(cx, socket, ping_msg, Kind::Ping);
        }
    }
}

impl Plugin for Ha {
    fn init(&mut self, cx: &mut Cx, settings: Value) -> Result<(), String> {
        if !settings.is_object() && !settings.is_null() {
            return Err("the settings must be a table: [plugins.ha.settings]".to_string());
        }
        if let Some(t) = settings
            .get("token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            cx.set_secret("token", t);
            cx.log(
                Level::Warn,
                "The access token is saved in the plugin’s secret store. Delete the token line from [plugins.ha.settings] in settings.toml.",
            );
        }
        let parsed = parse_settings(&settings).and_then(|cfg| {
            let n = subscription_ids(&cfg).len();
            if n > MAX_ENTITIES {
                Err(format!(
                    "at most {MAX_ENTITIES} entities can be followed, counting those only used by attributes, got {n}"
                ))
            } else {
                Ok(cfg)
            }
        });
        let mut cfg = match parsed {
            Ok(cfg) => cfg,
            Err(msg) => {
                // A settings mistake is shown as a status, not as a failed start.
                self.teardown(cx, true);
                self.config = None;
                self.set_status(
                    cx,
                    Status::Problem,
                    &format!(
                        "Check the Home Assistant settings: {}.",
                        msg.trim_end_matches('.')
                    ),
                );
                return Ok(());
            }
        };
        if cfg.token.is_none() {
            cfg.token = cx.secret("token").filter(|t| !t.is_empty());
        }
        if self.config.as_ref() == Some(&cfg) {
            return Ok(());
        }
        if !cfg.skipped.is_empty() {
            cx.log(
                Level::Warn,
                &format!(
                    "Ignored entries that are not valid entity ids or entity:attribute pairs: {}",
                    cfg.skipped.join(", ")
                ),
            );
        }
        self.check_key_sizes(cx, &cfg);
        self.teardown(cx, true);
        self.config = Some(cfg);
        self.backoff = FIRST_BACKOFF;
        self.connect(cx, true);
        Ok(())
    }

    fn action(&mut self, cx: &mut Cx, action: &str, params: Value) -> Result<(), String> {
        match plan(action, &params, &self.tracked)? {
            Plan::Reconnect => {
                self.teardown(cx, true);
                self.backoff = FIRST_BACKOFF;
                if let Some(c) = &mut self.config
                    && c.token.is_none()
                {
                    c.token = cx.secret("token").filter(|t| !t.is_empty());
                }
                if self.config.is_none() {
                    return Err(self.offline_error());
                }
                self.connect(cx, true);
                Ok(())
            }
            Plan::ForgetToken => {
                cx.delete_secret("token");
                if let Some(c) = &mut self.config {
                    c.token = None;
                    self.go_down(cx, Down::NoToken, true);
                }
                Ok(())
            }
            Plan::Call(call) => self.call_service(cx, &call),
        }
    }

    fn list_items(
        &mut self,
        _cx: &mut Cx,
        picker: &str,
        query: &str,
        group: &str,
    ) -> Result<Vec<Item>, String> {
        // Signed in: the entity subscription may still be on its way, the list may already be in.
        if !matches!(self.phase, Phase::Subscribing { .. } | Phase::Live { .. }) {
            return Err(NOT_CONNECTED.to_string());
        }
        match &self.listing {
            Listing::Waiting => picker_items(&EntityList::default(), picker, query, group),
            Listing::Failed(e) => Err(e.clone()),
            Listing::Ready(list) => picker_items(list, picker, query, group),
        }
    }

    fn opened(&mut self, _cx: &mut Cx, socket: Socket) {
        if self.phase == Phase::Opening(socket) {
            self.phase = Phase::AwaitAuthRequired(socket);
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

    fn closed(&mut self, cx: &mut Cx, socket: Socket, reason: &str) {
        let why = match self.phase {
            // The host refuses a public name only after DNS, by closing the socket.
            Phase::Opening(s) if s == socket && reason.starts_with("not allowed") => Down::NotAllowed,
            Phase::Opening(s) if s == socket => Down::Refused,
            p if p.socket() == Some(socket) => Down::Dropped,
            _ => return,
        };
        self.note_reason(cx, reason);
        self.go_down(cx, why, false);
    }

    fn timer(&mut self, cx: &mut Cx, timer: Timer) {
        if self.handshake == Some(timer) {
            self.handshake = None;
            let why = match self.phase {
                Phase::Live { .. } | Phase::Idle => return,
                // Never got an answer from the address: unreachable, not a lost connection.
                Phase::Opening(_) => Down::Refused,
                _ => Down::Dropped,
            };
            self.go_down(cx, why, true);
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

ulanzi_plugin::export_plugin!(Ha);
