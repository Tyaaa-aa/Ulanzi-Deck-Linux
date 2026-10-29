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
/// `192.168.1.5`, `homeassistant.local:8123`, `http://host:8123`, `https://host/ha`, `ws://host:8123`
/// and a pasted `…/api/websocket` are accepted.
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
        "http" | "ws" => ("ws", 8123),
        "https" | "wss" => ("wss", 443),
        _ => {
            return Err(format!(
                "only http, https, ws and wss addresses work: {ADDRESS_HELP}"
            ));
        }
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
    // The full endpoint pasted from elsewhere: the suffix is added below.
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix("/api/websocket").unwrap_or(path);
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
fn pct_of(domain: &str, state: &str, attrs: &Map<String, Value>) -> String {
    let num = |k: &str| attrs.get(k).and_then(Value::as_f64);
    let pct = match domain {
        // Home Assistant sends brightness: null for a light that is off.
        "light" if state == "off" => Some(0.0),
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
                    pct_of(domain, &e.state, &e.attrs),
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

/// The most characters the entity list template writes before it stops (see [`ENTITY_LIST_TEMPLATE`]).
pub const LIST_BUDGET: usize = 200_000;
/// The most weight units (an upper bound of JSON bytes) it writes.
pub const LIST_WEIGHT_BUDGET: usize = 900_000;
/// The last line of a list that the template cut short.
pub const TRUNCATED_MARKER: &str = "…truncated";
/// Longest name and state kept per entity, in characters.
const NAME_CHARS: usize = 60;
const STATE_CHARS: usize = 24;

/// What `homeassistant.toggle`, `turn_on` and `turn_off` (the toggle, turn on and turn off
/// actions) handle well. Not lock (no turn_on/turn_off), not climate or vacuum (only some support it).
pub const TOGGLEABLE: &[&str] = &[
    "light",
    "switch",
    "fan",
    "input_boolean",
    "automation",
    "script",
    "cover",
    "media_player",
    "siren",
    "humidifier",
    "remote",
    "group",
];
/// What the run action accepts.
pub const RUNNABLE: &[&str] = &["scene", "script", "automation", "button", "input_button"];
/// What ready-made keys are made for.
pub const DEVICES: &[&str] = &[
    "light",
    "fan",
    "climate",
    "switch",
    "input_boolean",
    "cover",
    "media_player",
    "script",
    "scene",
    "button",
    "input_button",
];
/// What the set_level action accepts.
pub const LEVELS: &[&str] = &["light", "media_player", "fan", "cover"];
/// Listed first by the template, so a cut list keeps what keys can control.
pub const PREFERRED_DOMAINS: &[&str] = &[
    "light",
    "fan",
    "climate",
    "switch",
    "cover",
    "media_player",
    "input_boolean",
    "scene",
    "script",
    "automation",
    "button",
    "input_button",
    "siren",
    "humidifier",
    "remote",
    "group",
];

/// Every entity as one `entity_id \t name \t state` line, preferred domains first.
///
/// Control characters (tab and newline included) in names and states become spaces, names are cut
/// to 60 and states to 24 characters, and it writes the [`TRUNCATED_MARKER`] line instead of the
/// next entity once that would pass 200 000 characters, or 900 000 weight units counting 2 per
/// ASCII and 12 per other character. After the replacement an ASCII character takes at most 2
/// bytes in JSON (`"`, `\`, and the tab and newline separators are escaped), any other at most 4
/// as UTF-8 (orjson, current Home Assistant) or 12 as `\uXXXX\uXXXX` (`ensure_ascii`, as older
/// releases sent): an answer stays under 900 000 bytes plus its small envelope, below the host's
/// 1 MiB message limit, whichever encoder Home Assistant uses.
///
/// Iterating `states` makes Home Assistant render it at most once a minute.
pub const ENTITY_LIST_TEMPLATE: &str = r#"{%- set ns = namespace(n=0, w=0, full=false) -%}
{%- set pref = ['light', 'switch', 'fan', 'cover', 'media_player', 'input_boolean', 'scene', 'script', 'automation', 'button', 'input_button', 'siren', 'humidifier', 'remote', 'group'] -%}
{%- for s in (states | selectattr('domain', 'in', pref) | list) + (states | rejectattr('domain', 'in', pref) | list) -%}
{%- if not ns.full -%}
{%- set line = s.entity_id ~ '\t' ~ ((s.name | string | regex_replace('[\\x00-\\x1f\\x7f-\\x9f]', ' '))[:60]) ~ '\t' ~ ((s.state | string | regex_replace('[\\x00-\\x1f\\x7f-\\x9f]', ' '))[:24]) ~ '\n' -%}
{%- set w = (line | length) * 2 + (line | regex_replace('[\\x00-\\x7f]', '') | length) * 10 -%}
{%- if ns.n + (line | length) > 200000 or ns.w + w > 900000 -%}
{%- set ns.full = true -%}
…truncated
{%- else -%}
{%- set ns.n = ns.n + (line | length) -%}
{%- set ns.w = ns.w + w -%}
{{ line }}
{%- endif -%}
{%- endif -%}
{%- endfor -%}"#;

/// Subscribe to the entity list. Home Assistant answers with a `result`, then an `event` with
/// `{"result": "<lines>"}` now and after changes.
pub fn render_template_msg(id: u64) -> Value {
    json!({"id": id, "type": "render_template", "template": ENTITY_LIST_TEMPLATE})
}

/// One entity of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedEntity {
    pub entity_id: String,
    pub name: String,
    pub state: String,
}

impl ListedEntity {
    pub fn domain(&self) -> &str {
        self.entity_id.split('.').next().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityList {
    pub rows: Vec<ListedEntity>,
    /// The template stopped at its budget: some entities are missing.
    pub truncated: bool,
}

/// Control characters as spaces, at most `max` characters.
fn clean_field(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect()
}

/// Read what the template rendered. Lines that are not `entity_id \t name \t state` with a valid
/// entity id are skipped.
pub fn parse_entity_list(text: &str) -> EntityList {
    let mut list = EntityList::default();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim() == TRUNCATED_MARKER {
            list.truncated = true;
            break;
        }
        let mut parts = line.splitn(3, '\t');
        let (Some(id), Some(name), Some(state)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if !valid_entity_id(id) {
            continue;
        }
        list.rows.push(ListedEntity {
            entity_id: id.to_string(),
            name: clean_field(name, NAME_CHARS),
            state: clean_field(state, STATE_CHARS),
        });
    }
    let rank = |d: &str| {
        PREFERRED_DOMAINS
            .iter()
            .position(|p| *p == d)
            .unwrap_or(PREFERRED_DOMAINS.len())
    };
    list.rows.sort_by_cached_key(|r| {
        (
            rank(r.domain()),
            r.domain().to_string(),
            r.name.to_lowercase(),
            r.entity_id.clone(),
        )
    });
    list
}

/// A domain as a group heading: "Lights", "Binary sensors"; an unknown one as "My domain".
pub fn group_label(domain: &str) -> String {
    let known = match domain {
        "light" => "Lights",
        "switch" => "Switches",
        "sensor" => "Sensors",
        "binary_sensor" => "Binary sensors",
        "scene" => "Scenes",
        "script" => "Scripts",
        "automation" => "Automations",
        "button" => "Buttons",
        "input_button" => "Input buttons",
        "media_player" => "Media players",
        "cover" => "Covers",
        "fan" => "Fans",
        "climate" => "Climate",
        "input_boolean" => "Input booleans",
        "input_number" => "Input numbers",
        "input_select" => "Input selects",
        "input_text" => "Input texts",
        "input_datetime" => "Input dates and times",
        "lock" => "Locks",
        "siren" => "Sirens",
        "humidifier" => "Humidifiers",
        "remote" => "Remotes",
        "group" => "Groups",
        "vacuum" => "Vacuums",
        "water_heater" => "Water heaters",
        "valve" => "Valves",
        "lawn_mower" => "Lawn mowers",
        "camera" => "Cameras",
        "person" => "People",
        "device_tracker" => "Device trackers",
        "zone" => "Zones",
        "sun" => "Sun",
        "weather" => "Weather",
        "update" => "Updates",
        "number" => "Numbers",
        "select" => "Selects",
        "text" => "Texts",
        "timer" => "Timers",
        "counter" => "Counters",
        "alarm_control_panel" => "Alarm panels",
        "calendar" => "Calendars",
        "event" => "Events",
        "image" => "Images",
        "todo" => "To-do lists",
        "notify" => "Notifiers",
        "tts" => "Text to speech",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let spaced = domain.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The most entries one answer carries (the host keeps no more either).
const MAX_ITEMS: usize = 5000;

/// The domains a picker offers; `None` for every domain.
fn picker_domains(picker: &str) -> Result<Option<&'static [&'static str]>, String> {
    match picker {
        "entities" | "any" => Ok(None),
        "toggleable" => Ok(Some(TOGGLEABLE)),
        "runnable" => Ok(Some(RUNNABLE)),
        "levels" => Ok(Some(LEVELS)),
        "climate" => Ok(Some(&["climate"])),
        "devices" => Ok(Some(DEVICES)),
        other => Err(format!(
            "unknown picker \"{}\"",
            other.chars().take(60).collect::<String>()
        )),
    }
}

/// The entries of `picker` matching every word of `query` (case-insensitive, on the name, the id
/// or the group heading), only those
/// in `group` (a heading from [`group_label`] or a raw domain) unless it is empty.
pub fn picker_items(
    list: &EntityList,
    picker: &str,
    query: &str,
    group: &str,
) -> Result<Vec<ulanzi_plugin::Item>, String> {
    let domains = picker_domains(picker)?;
    let group = group.trim();
    let mut out = Vec::new();
    for row in &list.rows {
        let domain = row.domain();
        if domains.is_some_and(|d| !d.contains(&domain)) {
            continue;
        }
        let label = if row.name.trim().is_empty() {
            row.entity_id.clone()
        } else {
            row.name.clone()
        };
        let heading = group_label(domain);
        if !group.is_empty() && group != heading && group != domain {
            continue;
        }
        if !ulanzi_plugin::matches_query(query, &[&label, &row.entity_id, &heading]) {
            continue;
        }
        out.push(
            ulanzi_plugin::Item::new(row.entity_id.clone(), label)
                .subtitle(row.entity_id.clone())
                .group(heading)
                .state(row.state.clone()),
        );
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    Ok(out)
}
