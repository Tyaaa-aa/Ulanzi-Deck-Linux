//! Home Assistant WebSocket model: address, settings, message builders, the compressed entity
//! stream and the state keys derived from it. No I/O.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::actions::ServiceCall;

/// The host keeps at most this many distinct state keys per plugin for the instance's whole life.
pub const HOST_STATE_KEYS: usize = 256;
pub const MAX_ENTITIES: usize = 40;
pub const MAX_ATTRIBUTES: usize = 40;
/// The host ignores keys over 256 bytes and values over 1 KiB.
pub const HOST_KEY_BYTES: usize = 256;
pub const HOST_VALUE_BYTES: usize = 1024;

const ADDRESS_HELP: &str =
    "type the address like 192.168.1.5, homeassistant.local:8123 or https://ha.example";

/// Turn what a user types into the WebSocket endpoint.
///
/// `192.168.1.5`, `homeassistant.local:8123`, `http://host:8123`, `https://host/ha` are accepted.
/// No scheme means http; no port means 8123 (http) or 443 (https). Credentials, query and
/// fragment are refused without echoing them.
pub fn ws_url(input: &str) -> Result<String, String> {
    let text = input.trim();
    if text.is_empty() {
        return Err(format!("the address is empty: {ADDRESS_HELP}"));
    }
    let (scheme, rest) = match text.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("http".to_string(), text),
    };
    let (ws, default_port) = match scheme.as_str() {
        "http" => ("ws", 8123),
        "https" => ("wss", 443),
        _ => return Err(format!("only http and https addresses work: {ADDRESS_HELP}")),
    };
    if rest.contains(['?', '#']) {
        return Err(format!(
            "the address cannot have a query or fragment: {ADDRESS_HELP}"
        ));
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.contains('@') {
        return Err(format!(
            "the address cannot contain a user name or password (use the token setting): {ADDRESS_HELP}"
        ));
    }
    let (host, port) = split_host_port(authority, default_port)?;
    let path = path.trim_end_matches('/');
    Ok(format!("{ws}://{host}:{port}{path}/api/websocket"))
}

fn split_host_port(authority: &str, default_port: u16) -> Result<(String, u16), String> {
    let bad_port = || format!("the port must be a number from 1 to 65535: {ADDRESS_HELP}");
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        // [::1] or [::1]:8123
        let (addr, after) = inner.split_once(']').ok_or_else(|| ADDRESS_HELP.to_string())?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p),
            None if after.is_empty() => None,
            None => return Err(ADDRESS_HELP.to_string()),
        };
        (format!("[{addr}]"), port)
    } else {
        match authority.split_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err(format!("the address has no host name: {ADDRESS_HELP}"));
    }
    let port = match port {
        None => default_port,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 => n,
            _ => return Err(bad_port()),
        },
    };
    Ok((host.to_ascii_lowercase(), port))
}

/// `domain.object_id`, lowercase `[a-z0-9_]` on both sides, domain starting with a letter,
/// at most 255 bytes.
pub fn valid_entity_id(s: &str) -> bool {
    let Some((domain, object)) = s.split_once('.') else {
        return false;
    };
    // Like Home Assistant's own check: no `__`, no leading or trailing `_` (one bad id makes
    // the whole filtered subscribe fail).
    let word = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
            && !p.contains("__")
            && !p.starts_with('_')
            && !p.ends_with('_')
    };
    s.len() <= 255 && word(domain) && word(object) && domain.as_bytes()[0].is_ascii_lowercase()
}

/// The plugin's settings, parsed and cleaned.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    /// The WebSocket endpoint (see [`ws_url`]).
    pub url: String,
    pub token: Option<String>,
    pub entities: Vec<String>,
    /// `(entity id, attribute)` pairs to publish as `<eid>.attr.<attribute>`.
    pub attributes: Vec<(String, String)>,
    /// Entries of `entities` and `attributes` that were not valid, as typed.
    pub skipped: Vec<String>,
}

// The token must never reach a log or a failing test's output.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "<hidden>"))
            .field("entities", &self.entities)
            .field("attributes", &self.attributes)
            .field("skipped", &self.skipped)
            .finish()
    }
}

/// A list setting given as an array or as one text split at commas, semicolons and line breaks.
fn list_of(v: Option<&Value>) -> Vec<String> {
    let clean = |s: &str| s.trim().to_string();
    match v {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(clean)
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split([',', ';', '\n', '\r'])
            .map(clean)
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn valid_attribute(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// A skipped entry as typed, cut so it can be logged safely.
fn shorten(raw: &str) -> String {
    raw.chars().take(64).collect()
}

pub fn parse_settings(v: &Value) -> Result<Config, String> {
    let url = v.get("url").and_then(Value::as_str).unwrap_or_default();
    if url.trim().is_empty() {
        return Err(format!("the url setting is empty: {ADDRESS_HELP}"));
    }
    let url = ws_url(url)?;
    let token = v
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);

    let mut skipped = Vec::new();
    let mut entities: Vec<String> = Vec::new();
    for raw in list_of(v.get("entities")) {
        let id = raw.to_lowercase();
        if !valid_entity_id(&id) {
            skipped.push(shorten(&raw));
        } else if !entities.contains(&id) {
            entities.push(id);
        }
    }
    if entities.len() > MAX_ENTITIES {
        return Err(format!(
            "at most {MAX_ENTITIES} entities can be shown on keys, got {}",
            entities.len()
        ));
    }

    let mut attributes: Vec<(String, String)> = Vec::new();
    for raw in list_of(v.get("attributes")) {
        let pair = raw.split_once(':').and_then(|(e, a)| {
            let (e, a) = (e.trim().to_lowercase(), a.trim().to_lowercase());
            (valid_entity_id(&e) && valid_attribute(&a)).then_some((e, a))
        });
        match pair {
            Some(p) if !attributes.contains(&p) => attributes.push(p),
            Some(_) => {}
            None => skipped.push(shorten(&raw)),
        }
    }
    if attributes.len() > MAX_ATTRIBUTES {
        return Err(format!(
            "at most {MAX_ATTRIBUTES} attributes can be shown on keys, got {}",
            attributes.len()
        ));
    }
    Ok(Config {
        url,
        token,
        entities,
        attributes,
        skipped,
    })
}

pub fn auth_msg(token: &str) -> Value {
    json!({"type": "auth", "access_token": token})
}

/// Only the listed entities, never the whole install. `None` for an empty list: Home Assistant
/// would treat an empty filter as every entity.
pub fn subscribe_msg(id: u64, entities: &[String]) -> Option<Value> {
    if entities.is_empty() {
        return None;
    }
    Some(json!({"id": id, "type": "subscribe_entities", "entity_ids": entities}))
}

pub fn call_service_msg(id: u64, call: &ServiceCall) -> Value {
    let mut m = json!({
        "id": id,
        "type": "call_service",
        "domain": call.domain,
        "service": call.service,
        "service_data": call.data,
    });
    if let Some(e) = &call.entity_id {
        m["target"] = json!({"entity_id": e});
    }
    m
}

pub fn ping_msg(id: u64) -> Value {
    json!({"id": id, "type": "ping"})
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entity {
    pub state: String,
    pub attrs: Map<String, Value>,
}

/// The entities Home Assistant told us about.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tracked {
    pub entities: BTreeMap<String, Entity>,
}

impl Tracked {
    /// Apply one `subscribe_entities` event (compressed form). True if anything changed.
    pub fn apply(&mut self, event: &Value) -> bool {
        let mut changed = false;
        if let Some(snapshot) = event.get("a").and_then(Value::as_object) {
            for (id, raw) in snapshot {
                let Some(raw) = raw.as_object() else { continue };
                let entity = Entity {
                    state: raw.get("s").map(state_text).unwrap_or_default(),
                    attrs: raw
                        .get("a")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                };
                if self.entities.get(id) != Some(&entity) {
                    self.entities.insert(id.clone(), entity);
                    changed = true;
                }
            }
        }
        if let Some(diffs) = event.get("c").and_then(Value::as_object) {
            for (id, diff) in diffs {
                let Some(entity) = self.entities.get_mut(id) else {
                    continue;
                };
                let before = entity.clone();
                if let Some(plus) = diff.get("+") {
                    if let Some(s) = plus.get("s") {
                        entity.state = state_text(s);
                    }
                    if let Some(attrs) = plus.get("a").and_then(Value::as_object) {
                        for (k, v) in attrs {
                            entity.attrs.insert(k.clone(), v.clone());
                        }
                    }
                }
                if let Some(gone) = diff.get("-").and_then(|m| m.get("a")).and_then(Value::as_array) {
                    for k in gone.iter().filter_map(Value::as_str) {
                        entity.attrs.remove(k);
                    }
                }
                changed |= *entity != before;
            }
        }
        if let Some(removed) = event.get("r").and_then(Value::as_array) {
            for id in removed.iter().filter_map(Value::as_str) {
                changed |= self.entities.remove(id).is_some();
            }
        }
        changed
    }
}

fn state_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// What the connection knows about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveInfo {
    pub connected: bool,
    pub version: String,
    pub last_error: String,
}

/// A 0 to 100 level for the kinds of entity that have one.
fn pct_of(domain: &str, attrs: &Map<String, Value>) -> String {
    let num = |k: &str| attrs.get(k).and_then(Value::as_f64);
    let pct = match domain {
        "light" => num("brightness").map(|b| b / 255.0 * 100.0),
        "media_player" => num("volume_level").map(|v| v * 100.0),
        "fan" => num("percentage"),
        "cover" => num("current_position"),
        _ => None,
    };
    pct.map(|p| format!("{}", p.round().clamp(0.0, 100.0) as i64))
        .unwrap_or_default()
}

/// An attribute value as label text: strings as they are, whole numbers without `.0`, structures
/// as compact JSON, null as empty.
fn attr_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
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

/// Every state key and value to publish while connected.
pub fn state_pairs(t: &Tracked, cfg: &Config, live: &LiveInfo) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        (
            "connected".into(),
            if live.connected { "yes" } else { "no" }.into(),
        ),
        ("version".into(), live.version.clone()),
        ("last_error".into(), live.last_error.clone()),
        (
            "entity_count".into(),
            cfg.entities
                .iter()
                .filter(|e| t.entities.contains_key(*e))
                .count()
                .to_string(),
        ),
    ];
    for id in &cfg.entities {
        let domain = id.split('.').next().unwrap_or_default();
        let (state, name, unit, pct) = match t.entities.get(id) {
            Some(e) => {
                let text = |k: &str| e.attrs.get(k).map(attr_text).unwrap_or_default();
                (
                    e.state.clone(),
                    text("friendly_name"),
                    text("unit_of_measurement"),
                    pct_of(domain, &e.attrs),
                )
            }
            None => ("missing".into(), String::new(), String::new(), String::new()),
        };
        out.push((id.clone(), state));
        out.push((format!("{id}.name"), name));
        out.push((format!("{id}.unit"), unit));
        out.push((format!("{id}.pct"), pct));
    }
    for (id, attr) in &cfg.attributes {
        let value = t
            .entities
            .get(id)
            .and_then(|e| e.attrs.get(attr))
            .map(attr_text)
            .unwrap_or_default();
        out.push((format!("{id}.attr.{attr}"), value));
    }
    // The host ignores keys over 256 bytes; leave those out instead of sending them.
    out.into_iter()
        .filter(|(k, _)| k.len() <= HOST_KEY_BYTES)
        .map(|(k, v)| (k, truncate_at_boundary(v, HOST_VALUE_BYTES)))
        .collect()
}

/// What to publish when the connection is gone: `connected` is `no`, every entity state is
/// `offline`, everything else published is blanked. `last_error` keeps its text.
pub fn offline_pairs(published: &BTreeSet<String>) -> Vec<(String, String)> {
    published
        .iter()
        .filter(|k| k.as_str() != "last_error")
        .map(|k| {
            let value = if k == "connected" {
                "no"
            } else if valid_entity_id(k) {
                "offline"
            } else {
                ""
            };
            (k.clone(), value.to_string())
        })
        .collect()
}
