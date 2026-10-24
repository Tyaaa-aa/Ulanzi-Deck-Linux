//! The Home Assistant plugin against the SDK's fake host: connection lifecycle, live state and actions.
use std::time::Duration;

use serde_json::Value;
use ulanzi_plugin::testing::Harness;
use ulanzi_plugin::{Level, Payload, Socket, Status, Timer, json};
use ulanzi_plugin_ha::Ha;

const TOKEN: &str = "eyJ0eXAi.sekrit-token-value.zzz";
const VERSION: &str = "2025.10.1";

fn auth_required() -> Payload {
    Payload::Text(json!({"type": "auth_required", "ha_version": VERSION}).to_string())
}

fn auth_ok() -> Payload {
    Payload::Text(json!({"type": "auth_ok", "ha_version": VERSION}).to_string())
}

fn auth_invalid() -> Payload {
    Payload::Text(json!({"type": "auth_invalid", "message": "Invalid access token or password"}).to_string())
}

fn text(v: Value) -> Payload {
    Payload::Text(v.to_string())
}

fn texts(h: &Harness<Ha>, s: Socket) -> Vec<Value> {
    h.sent(s)
        .iter()
        .filter_map(|p| match p {
            Payload::Text(t) => serde_json::from_str(t).ok(),
            Payload::Binary(_) => None,
        })
        .collect()
}

/// Commands sent on `s`: (id, type, message).
fn sent_msgs(h: &Harness<Ha>, s: Socket) -> Vec<(u64, String, Value)> {
    texts(h, s)
        .into_iter()
        .filter(|m| m.get("id").is_some())
        .map(|m| {
            (
                m["id"].as_u64().unwrap_or_default(),
                m["type"].as_str().unwrap_or_default().to_string(),
                m,
            )
        })
        .collect()
}

fn snapshot(h: &mut Harness<Ha>, s: Socket, sub_id: u64, entities: Value) {
    h.message(
        s,
        text(json!({"id": sub_id, "type": "event", "event": {"a": entities}})),
    );
}

fn change(h: &mut Harness<Ha>, s: Socket, sub_id: u64, diff: Value) {
    h.message(
        s,
        text(json!({"id": sub_id, "type": "event", "event": {"c": diff}})),
    );
}

fn result(h: &mut Harness<Ha>, s: Socket, id: u64, ok: bool, error: Option<Value>) {
    let mut m = json!({"id": id, "type": "result", "success": ok, "result": null});
    if let Some(e) = error {
        m["error"] = e;
    }
    h.message(s, text(m));
}

fn settings() -> Value {
    json!({"url": "homeassistant.local", "token": TOKEN,
        "entities": ["light.desk", "sensor.temp"]})
}

fn desk_snapshot() -> Value {
    json!({
        "light.desk": {"s": "on", "a": {"friendly_name": "Desk", "brightness": 255}},
        "sensor.temp": {"s": "21.5", "a": {"unit_of_measurement": "°C", "friendly_name": "Temp"}},
    })
}

fn current(h: &Harness<Ha>) -> Socket {
    h.opened().last().expect("a socket was opened").0
}

/// Handshake on the current socket until the subscription result and the snapshot are in.
fn handshake(h: &mut Harness<Ha>, snap: Value) -> Socket {
    let s = current(h);
    h.open(s);
    h.message(s, auth_required());
    h.message(s, auth_ok());
    let sub = sent_msgs(h, s)
        .into_iter()
        .find(|m| m.1 == "subscribe_entities")
        .expect("subscribe sent");
    result(h, s, sub.0, true, None);
    snapshot(h, s, sub.0, snap);
    s
}

fn live(h: &mut Harness<Ha>) -> Socket {
    h.init(settings()).expect("init");
    handshake(h, desk_snapshot())
}

fn status_msg(h: &Harness<Ha>) -> (Status, String) {
    h.status().expect("a status")
}

/// The single one-shot timer that is not the 5 s handshake timer.
fn retry_timer(h: &Harness<Ha>) -> (Timer, Duration) {
    let t: Vec<_> = h
        .timers()
        .into_iter()
        .filter(|(_, d, repeat)| !repeat && *d != Duration::from_secs(5))
        .collect();
    assert_eq!(t.len(), 1, "exactly one retry timer: {:?}", h.timers());
    (t[0].0, t[0].1)
}

fn no_retry(h: &Harness<Ha>) {
    let t: Vec<_> = h
        .timers()
        .into_iter()
        .filter(|(_, d, repeat)| !repeat && *d != Duration::from_secs(5))
        .collect();
    assert!(t.is_empty(), "no retry expected: {t:?}");
}

fn tick(h: &Harness<Ha>) -> Timer {
    h.timers()
        .into_iter()
        .find(|(_, d, repeat)| *repeat && *d == Duration::from_millis(1000))
        .expect("tick timer")
        .0
}

fn assert_no_token(h: &Harness<Ha>) {
    assert!(
        h.states().values().all(|v| !v.contains(TOKEN)),
        "token in a state value"
    );
    if let Some((_, m)) = h.status() {
        assert!(!m.contains(TOKEN), "token in the status");
    }
    assert!(h.logs().iter().all(|(_, m)| !m.contains(TOKEN)), "token in a log");
}

#[test]
fn connects_authenticates_and_subscribes() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    assert_eq!(h.opened()[0].1, "ws://homeassistant.local:8123/api/websocket");
    let s = h.opened()[0].0;
    h.open(s);
    assert!(texts(&h, s).is_empty(), "the server speaks first");
    h.message(s, auth_required());
    assert_eq!(texts(&h, s), vec![json!({"type": "auth", "access_token": TOKEN})]);
    h.message(s, auth_ok());
    let msgs = sent_msgs(&h, s);
    assert_eq!(msgs.len(), 1);
    assert_eq!((msgs[0].0, msgs[0].1.as_str()), (1, "subscribe_entities"));
    assert_eq!(msgs[0].2["entity_ids"], json!(["light.desk", "sensor.temp"]));
    assert_ne!(h.state("connected"), Some("yes"));
    result(&mut h, s, 1, true, None);
    snapshot(&mut h, s, 1, desk_snapshot());
    assert_eq!(h.state("connected"), Some("yes"));
    assert_eq!(h.state("light.desk"), Some("on"));
    assert_eq!(h.state("light.desk.name"), Some("Desk"));
    assert_eq!(h.state("light.desk.pct"), Some("100"));
    assert_eq!(h.state("sensor.temp.unit"), Some("°C"));
    assert_eq!(h.state("version"), Some(VERSION));
    assert_eq!(h.state("entity_count"), Some("2"));
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Ok);
    assert!(
        msg.contains("Home Assistant 2025") && msg.contains("2 entities"),
        "{msg}"
    );
    assert!(sent_msgs(&h, s).iter().all(|m| m.1 != "get_states"));
    assert_no_token(&h);
}

#[test]
fn token_moves_to_the_secret_store() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    assert_eq!(h.secrets()["token"], TOKEN);
    assert!(
        h.logs()
            .iter()
            .any(|(l, m)| *l == Level::Warn && m.contains("[plugins.ha.settings]"))
    );
    assert_no_token(&h);

    // The second start has no token line and uses the stored one.
    let mut h2 = Harness::<Ha>::new();
    h2.secrets().insert("token".into(), TOKEN.into());
    h2.init(json!({"url": "homeassistant.local", "entities": ["light.desk"]}))
        .expect("init");
    let s = current(&h2);
    h2.open(s);
    h2.message(s, auth_required());
    assert_eq!(
        texts(&h2, s),
        vec![json!({"type": "auth", "access_token": TOKEN})]
    );

    // No token anywhere.
    let mut h3 = Harness::<Ha>::new();
    h3.init(json!({"url": "homeassistant.local"})).expect("init");
    assert!(h3.opened().is_empty());
    let (st, msg) = status_msg(&h3);
    assert_eq!(st, Status::Problem);
    assert!(msg.contains("token"), "{msg}");
    no_retry(&h3);
    // Non-object settings are a hard error.
    assert!(h3.init(json!("nope")).is_err());
}

#[test]
fn bad_token_reports_and_never_leaks() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, auth_required());
    h.message(s, auth_invalid());
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.contains("access token") && msg.contains("Profile"), "{msg}");
    no_retry(&h);
    assert_no_token(&h);
    // The server closing afterwards does not start a retry loop.
    h.closed(s, "bye");
    no_retry(&h);
    h.action("reconnect", json!({})).expect("reconnect");
    assert_eq!(h.opened().len(), 2);
    let e = h
        .action("toggle", json!({"entity_id": "light.desk"}))
        .expect_err("not live");
    assert!(!e.contains(TOKEN));
    assert_no_token(&h);
}

#[test]
fn refused_backs_off() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let mut got = Vec::new();
    for _ in 0..6 {
        let s = current(&h);
        h.closed(s, "refused");
        let (t, d) = retry_timer(&h);
        got.push(d);
        h.fire(t);
    }
    let secs: Vec<u64> = got.iter().map(Duration::as_secs).collect();
    assert_eq!(secs, [1, 2, 4, 8, 10, 10]);
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.contains("homeassistant.local:8123"), "{msg}");
    assert_no_token(&h);
}

#[test]
fn events_update_state_and_ignore_stale_sockets() {
    let mut h = Harness::<Ha>::new();
    let old = live(&mut h);
    change(
        &mut h,
        old,
        1,
        json!({"light.desk": {"+": {"s": "off", "a": {"brightness": 0}}}}),
    );
    assert_eq!(h.state("light.desk"), Some("off"));
    assert_eq!(h.state("light.desk.pct"), Some("0"));
    // A change for an entity that is not listed adds nothing.
    let before = h.states().clone();
    change(&mut h, old, 1, json!({"switch.other": {"+": {"s": "on"}}}));
    snapshot(&mut h, old, 1, json!({"switch.other": {"s": "on", "a": {}}}));
    assert_eq!(h.states(), &before);
    // A new connection; the old socket's events are ignored.
    h.init(json!({"url": "homeassistant.local", "token": TOKEN, "entities": ["light.desk"]}))
        .expect("init");
    let new = current(&h);
    assert_ne!(new, old);
    change(&mut h, old, 1, json!({"light.desk": {"+": {"s": "on"}}}));
    assert_ne!(h.state("light.desk"), Some("on"));
    assert_no_token(&h);
}

#[test]
fn reconnect_after_drop() {
    let mut h = Harness::<Ha>::new();
    let s = live(&mut h);
    let e = h.state("last_error").map(str::to_string);
    h.closed(s, "going away");
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(h.state("light.desk"), Some("offline"));
    assert_eq!(h.state("light.desk.name"), Some(""));
    assert_eq!(h.state("version"), Some(""));
    assert_eq!(status_msg(&h).0, Status::Connecting);
    assert_eq!(h.state("last_error").map(str::to_string), e);
    let (t, d) = retry_timer(&h);
    assert_eq!(d, Duration::from_secs(1));
    h.fire(t);
    assert_eq!(h.opened().len(), 2);
    let s2 = handshake(&mut h, desk_snapshot());
    assert_ne!(s, s2);
    assert_eq!(h.state("connected"), Some("yes"));
    assert_eq!(h.state("light.desk"), Some("on"));
    assert_eq!(h.state("light.desk.name"), Some("Desk"));
    // Ids restart on the new connection.
    assert_eq!(sent_msgs(&h, s2)[0].0, 1);
    assert_no_token(&h);
}

#[test]
fn entity_missing_in_ha() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    handshake(
        &mut h,
        json!({"light.desk": {"s": "on", "a": {"friendly_name": "Desk"}}}),
    );
    assert_eq!(h.state("sensor.temp"), Some("missing"));
    assert_eq!(h.state("entity_count"), Some("1"));
    assert!(status_msg(&h).1.contains("1 entity"));
}

#[test]
fn actions_send_call_service() {
    let mut h = Harness::<Ha>::new();
    h.init(json!({"url": "homeassistant.local", "token": TOKEN,
        "entities": ["light.desk", "cover.blind"]}))
        .expect("init");
    let s = handshake(
        &mut h,
        json!({"light.desk": {"s": "on", "a": {}},
            "cover.blind": {"s": "open", "a": {"current_position": 40}}}),
    );
    h.action("toggle", json!({"entity_id": "light.desk"}))
        .expect("toggle");
    let (id, ty, m) = sent_msgs(&h, s).pop().expect("sent");
    assert_eq!(ty, "call_service");
    assert_eq!(m["domain"], "homeassistant");
    assert_eq!(m["service"], "toggle");
    assert_eq!(m["target"]["entity_id"], "light.desk");

    result(
        &mut h,
        s,
        id,
        false,
        Some(json!({"code": "not_found", "message": "Service not found"})),
    );
    assert_eq!(h.state("last_error"), Some("Service not found"));
    assert!(
        h.logs()
            .iter()
            .any(|(l, m)| *l == Level::Warn && m.contains("Service not found"))
    );
    h.action("turn_on", json!({"entity_id": "light.desk"}))
        .expect("on");
    let (id2, _, _) = sent_msgs(&h, s).pop().expect("sent");
    assert!(id2 > id);
    result(&mut h, s, id2, true, None);
    assert_eq!(h.state("last_error"), Some(""));

    h.action(
        "call_service",
        json!({"domain": "notify", "service": "mobile", "data": {"message": "hi"}}),
    )
    .expect("call");
    let (_, _, m) = sent_msgs(&h, s).pop().expect("sent");
    assert_eq!(m["service_data"], json!({"message": "hi"}));
    assert!(m.get("target").is_none());

    h.action(
        "set_level",
        json!({"entity_id": "cover.blind", "op": "up", "step": 10}),
    )
    .expect("level");
    let (_, _, m) = sent_msgs(&h, s).pop().expect("sent");
    assert_eq!(m["service"], "set_cover_position");
    assert_eq!(m["service_data"]["position"], 50);

    // The token goes with forget_token.
    h.action("forget_token", json!({})).expect("forget");
    assert!(!h.secrets().contains_key("token"));
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.contains("token"));
    no_retry(&h);
    let e = h
        .action("toggle", json!({"entity_id": "light.desk"}))
        .expect_err("down");
    assert!(e.starts_with("Home Assistant isn’t connected"), "{e}");
    assert_eq!(h.state("light.desk"), Some("offline"));
    assert_no_token(&h);
}

#[test]
fn action_while_not_live_is_an_error() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let e = h
        .action("toggle", json!({"entity_id": "light.desk"}))
        .expect_err("connecting");
    assert!(e.starts_with("Home Assistant isn’t connected"), "{e}");
    // Validation errors still win.
    assert!(h.action("toggle", json!({"entity_id": "bad"})).is_err());
}

#[test]
fn ping_detects_a_dead_socket() {
    let mut h = Harness::<Ha>::new();
    let s = live(&mut h);
    let t = tick(&h);
    let pings = |h: &Harness<Ha>| sent_msgs(h, s).iter().filter(|m| m.1 == "ping").count();
    for _ in 0..14 {
        h.fire(t);
    }
    assert_eq!(pings(&h), 0);
    h.fire(t);
    assert_eq!(pings(&h), 1);
    // A pong clears the counter; another ping 15 ticks later.
    let (id, _, _) = sent_msgs(&h, s)
        .into_iter()
        .find(|m| m.1 == "ping")
        .expect("ping");
    h.message(s, text(json!({"id": id, "type": "pong"})));
    for _ in 0..40 {
        h.fire(t);
    }
    assert_eq!(h.state("connected"), Some("no"), "pings 2 and 3 unanswered");
    // With a pong for every ping the connection survives.
    let mut h = Harness::<Ha>::new();
    let s = live(&mut h);
    let t = tick(&h);
    for round in 0..4 {
        for _ in 0..15 {
            h.fire(t);
        }
        let (id, _, _) = sent_msgs(&h, s)
            .into_iter()
            .rfind(|m| m.1 == "ping")
            .expect("ping");
        assert_eq!(
            sent_msgs(&h, s).iter().filter(|m| m.1 == "ping").count(),
            round + 1
        );
        h.message(s, text(json!({"id": id, "type": "pong"})));
    }
    assert_eq!(h.state("connected"), Some("yes"));
}

#[test]
fn two_unanswered_pings_take_the_connection_down() {
    let mut h = Harness::<Ha>::new();
    let s = live(&mut h);
    let t = tick(&h);
    for _ in 0..60 {
        if h.state("connected") == Some("no") {
            break;
        }
        h.fire(t);
    }
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(status_msg(&h).0, Status::Connecting);
    assert_eq!(retry_timer(&h).1, Duration::from_secs(1));
    assert_eq!(sent_msgs(&h, s).iter().filter(|m| m.1 == "ping").count(), 2);
}

#[test]
fn a_lost_socket_takes_the_connection_down() {
    let mut h = Harness::<Ha>::new();
    let s = live(&mut h);
    h.lose_socket(s);
    let e = h
        .action("toggle", json!({"entity_id": "light.desk"}))
        .expect_err("send fails");
    assert!(e.starts_with("Home Assistant isn’t connected"), "{e}");
    assert_eq!(h.state("light.desk"), Some("offline"));
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(status_msg(&h).0, Status::Connecting);
    assert_eq!(retry_timer(&h).1, Duration::from_secs(1));
    assert_no_token(&h);
}

#[test]
fn unanswered_service_calls_expire_quietly() {
    let mut h = Harness::<Ha>::new();
    let s = live(&mut h);
    h.action("toggle", json!({"entity_id": "light.desk"}))
        .expect("toggle");
    let (id, _, _) = sent_msgs(&h, s).pop().expect("sent");
    let t = tick(&h);
    for _ in 0..7 {
        h.fire(t);
    }
    let warns = |h: &Harness<Ha>| {
        h.logs()
            .iter()
            .filter(|(l, m)| *l == Level::Warn && m.contains("didn’t answer"))
            .count()
    };
    assert_eq!(warns(&h), 1);
    // The late answer is ignored.
    result(&mut h, s, id, false, Some(json!({"message": "too late"})));
    assert_eq!(h.state("last_error"), Some(""));
    assert_eq!(h.state("connected"), Some("yes"));
    // A ping expiring is not logged as a service call.
    for _ in 0..14 {
        h.fire(t);
    }
    assert_eq!(warns(&h), 1);
}

#[test]
fn settings_change_reconnects() {
    let mut h = Harness::<Ha>::new();
    let old = live(&mut h);
    // Unchanged settings: nothing happens.
    h.init(settings()).expect("init");
    assert_eq!(h.opened().len(), 1);
    h.init(json!({"url": "homeassistant.local", "token": TOKEN, "entities": ["light.desk"]}))
        .expect("init");
    assert_eq!(h.opened().len(), 2);
    assert_ne!(current(&h), old);
    // Keys of the removed entity do not linger as "on".
    assert_eq!(h.state("sensor.temp"), Some("offline"));
    assert_eq!(h.state("light.desk"), Some("offline"));
    assert_eq!(h.state("connected"), Some("no"));
    change(&mut h, old, 1, json!({"light.desk": {"+": {"s": "on"}}}));
    assert_eq!(h.state("light.desk"), Some("offline"));
    handshake(&mut h, json!({"light.desk": {"s": "on", "a": {}}}));
    assert_eq!(h.state("light.desk"), Some("on"));
    assert_eq!(h.state("sensor.temp"), Some("offline"));
    assert_no_token(&h);
}

#[test]
fn state_keys_stay_under_the_host_cap_for_the_whole_life() {
    let mut h = Harness::<Ha>::new();
    for round in 0..12 {
        let ids: Vec<String> = (0..40).map(|i| format!("sensor.s{round}_{i}")).collect();
        h.init(
            json!({"url": "homeassistant.local", "token": TOKEN, "entities": ids,
            "attributes": [format!("sensor.s{round}_0:level")]}),
        )
        .expect("init");
        let snap: serde_json::Map<String, Value> = ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    json!({"s": "1", "a": {"friendly_name": id, "level": 3}}),
                )
            })
            .collect();
        handshake(&mut h, Value::Object(snap));
        assert!(h.states().len() <= 256, "{} keys", h.states().len());
    }
    // Keys published earlier keep updating; ones past the cap never appear.
    assert_eq!(h.state("sensor.s0_0"), Some("offline"));
    assert_eq!(h.state("sensor.s11_39"), None);
    // The first round's keys are known to the host; bring one back through a settings change.
    h.init(json!({"url": "homeassistant.local", "token": TOKEN, "entities": ["sensor.s0_0"]}))
        .expect("init");
    handshake(&mut h, json!({"sensor.s0_0": {"s": "9", "a": {}}}));
    assert_eq!(h.state("sensor.s0_0"), Some("9"));
    assert!(h.states().len() <= 256);
    let warns = h
        .logs()
        .iter()
        .filter(|(l, m)| *l == Level::Warn && m.contains("state keys the host allows"))
        .count();
    assert_eq!(warns, 1);
    // Reconnecting does not make room.
    let s = current(&h);
    h.closed(s, "gone");
    assert!(h.states().len() <= 256);
}

#[test]
fn oversize_keys_and_values_are_handled() {
    let mut h = Harness::<Ha>::new();
    let long_id = format!("sensor.{}", "a".repeat(255 - "sensor.".len()));
    assert_eq!(long_id.len(), 255);
    let long_attr = "x".repeat(120);
    h.init(json!({"url": "homeassistant.local", "token": TOKEN,
        "entities": [long_id, "light.desk"],
        "attributes": [format!("{long_id}:{long_attr}"), "light.desk:note"]}))
        .expect("init");
    let big = "€".repeat(3000);
    handshake(
        &mut h,
        json!({long_id.clone(): {"s": "7", "a": {}},
            "light.desk": {"s": "on", "a": {"note": big}}}),
    );
    assert_eq!(h.state(&long_id), Some("7"));
    assert!(h.states().keys().all(|k| k.len() <= 256));
    let note = h.state("light.desk.attr.note").expect("note");
    assert!(
        !note.is_empty() && note.len() <= 1024 && note.chars().all(|c| c == '€'),
        "{}",
        note.len()
    );
    let warns = h
        .logs()
        .iter()
        .filter(|(l, m)| *l == Level::Warn && m.contains("longer than 256 bytes"))
        .count();
    assert_eq!(warns, 1);
    // A long error is cut at a character boundary, not dropped.
    let s = current(&h);
    h.action("toggle", json!({"entity_id": "light.desk"}))
        .expect("toggle");
    let (id, _, _) = sent_msgs(&h, s).pop().expect("sent");
    result(&mut h, s, id, false, Some(json!({"message": "€".repeat(1500)})));
    let e = h.state("last_error").expect("last_error");
    assert!(e.len() <= 1024 && e.chars().all(|c| c == '€') && !e.is_empty());
}

#[test]
fn handshake_timeout_retries() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let s = current(&h);
    h.open(s);
    let (t, d) = h
        .timers()
        .into_iter()
        .find(|(_, d, r)| !r && *d == Duration::from_secs(5))
        .map(|(t, d, _)| (t, d))
        .expect("handshake timer");
    assert_eq!(d, Duration::from_secs(5));
    h.fire(t);
    assert_eq!(status_msg(&h).0, Status::Connecting);
    assert_eq!(retry_timer(&h).1, Duration::from_secs(1));
    assert_eq!(h.state("connected"), Some("no"));
}

#[test]
fn too_old_home_assistant() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, auth_required());
    h.message(s, auth_ok());
    result(
        &mut h,
        s,
        1,
        false,
        Some(json!({"code": "unknown_command", "message": "Unknown command."})),
    );
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.contains("2022.4"), "{msg}");
    no_retry(&h);
}

#[test]
fn a_refused_subscription_is_reported_and_backs_off() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let mut waits = Vec::new();
    for _ in 0..3 {
        let s = current(&h);
        h.open(s);
        h.message(s, auth_required());
        h.message(s, auth_ok());
        result(
            &mut h,
            s,
            1,
            false,
            Some(json!({"code": "home_assistant_error", "message": "Entity filter rejected"})),
        );
        assert_eq!(h.state("last_error"), Some("Entity filter rejected"));
        let (st, msg) = status_msg(&h);
        assert_eq!(st, Status::Problem);
        assert!(msg.contains("subscription"), "{msg}");
        let (t, d) = retry_timer(&h);
        waits.push(d.as_secs());
        h.fire(t);
    }
    // auth_ok alone does not reset the backoff.
    assert_eq!(waits, [1, 2, 4]);
}

#[test]
fn no_entities_means_no_subscription() {
    let mut h = Harness::<Ha>::new();
    h.init(json!({"url": "homeassistant.local", "token": TOKEN}))
        .expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, auth_required());
    h.message(s, auth_ok());
    // An empty entity filter would mean every entity of the install.
    assert!(sent_msgs(&h, s).is_empty());
    assert_eq!(h.state("connected"), Some("yes"));
    assert_eq!(status_msg(&h).0, Status::Ok);
}

#[test]
fn attributes_of_unlisted_entities_join_the_subscription() {
    let mut h = Harness::<Ha>::new();
    h.init(json!({"url": "homeassistant.local", "token": TOKEN,
        "entities": ["light.desk"], "attributes": ["climate.living:current_temperature"]}))
        .expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, auth_required());
    h.message(s, auth_ok());
    assert_eq!(
        sent_msgs(&h, s)[0].2["entity_ids"],
        json!(["light.desk", "climate.living"])
    );
    snapshot(
        &mut h,
        s,
        1,
        json!({"light.desk": {"s": "on", "a": {}},
            "climate.living": {"s": "heat", "a": {"current_temperature": 20.5}}}),
    );
    assert_eq!(h.state("climate.living.attr.current_temperature"), Some("20.5"));
    assert_eq!(h.state("climate.living"), None);
    assert_eq!(h.state("entity_count"), Some("1"));
    // Together with the entities they may not pass 40.
    let ids: Vec<String> = (0..40).map(|i| format!("light.l{i}")).collect();
    h.init(
        json!({"url": "homeassistant.local", "token": TOKEN, "entities": ids,
        "attributes": ["climate.living:current_temperature"]}),
    )
    .expect("init");
    assert_eq!(status_msg(&h).0, Status::Problem);
}

#[test]
fn bad_settings_are_a_status_not_a_crash() {
    let mut h = Harness::<Ha>::new();
    h.init(json!({"url": "ftp://x", "token": TOKEN})).expect("init");
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.contains("http"), "{msg}");
    assert!(h.opened().is_empty());
    assert_eq!(h.secrets()["token"], TOKEN);
    assert_no_token(&h);
}

#[test]
fn live_when_the_subscription_succeeds_even_without_a_snapshot() {
    let mut h = Harness::<Ha>::new();
    h.init(settings()).expect("init");
    let mut waits = Vec::new();
    for _ in 0..2 {
        let s = current(&h);
        h.open(s);
        h.message(s, auth_required());
        h.message(s, auth_ok());
        // None of the listed entities exists: HA may send no snapshot at all.
        result(&mut h, s, 1, true, None);
        assert_eq!(h.state("connected"), Some("yes"));
        assert_eq!(h.state("light.desk"), Some("missing"));
        assert_eq!(h.state("entity_count"), Some("0"));
        assert_eq!(status_msg(&h).0, Status::Ok);
        // No handshake timer is left to drop the connection.
        assert!(
            h.timers().iter().all(|(_, d, _)| *d != Duration::from_secs(5)),
            "{:?}",
            h.timers()
        );
        // A late snapshot still fills the values in.
        snapshot(&mut h, s, 1, desk_snapshot());
        assert_eq!(h.state("light.desk"), Some("on"));
        h.closed(s, "gone");
        let (t, d) = retry_timer(&h);
        waits.push(d.as_secs());
        h.fire(t);
    }
    // The confirmed subscription reset the backoff each time.
    assert_eq!(waits, [1, 1]);
}

#[test]
fn settings_problem_text_has_one_full_stop() {
    let mut h = Harness::<Ha>::new();
    h.init(json!({"url": "", "token": TOKEN})).expect("init");
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.starts_with("Check [plugins.ha.settings]: "), "{msg}");
    assert!(!msg.ends_with(".."), "{msg}");
}
