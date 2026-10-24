use std::collections::BTreeSet;

use ulanzi_plugin::{Value, json};
use ulanzi_plugin_ha::actions::*;
use ulanzi_plugin_ha::protocol::*;

#[test]
fn ws_url_accepts_the_forms_users_type() {
    let ok = [
        ("192.168.1.5", "ws://192.168.1.5:8123/api/websocket"),
        (
            "homeassistant.local:8123/",
            "ws://homeassistant.local:8123/api/websocket",
        ),
        ("https://ha.lan", "wss://ha.lan:443/api/websocket"),
        ("http://h:8123/ha", "ws://h:8123/ha/api/websocket"),
        ("  HTTP://H ", "ws://h:8123/api/websocket"),
    ];
    for (input, want) in ok {
        assert_eq!(ws_url(input).as_deref(), Ok(want), "{input:?}");
    }
    for bad in [
        "",
        "http://u:p@h",
        "h?x=1",
        "ftp://h",
        "http://",
        "h#frag",
        "h:99999",
    ] {
        let err = ws_url(bad).expect_err(bad);
        assert!(!err.is_empty());
        assert!(!err.contains("u:p"), "echoed userinfo: {err}");
    }
    let err = ws_url("http://user:secrettoken@h").expect_err("userinfo");
    assert!(!err.contains("secrettoken"), "{err}");
}

#[test]
fn settings_parse_lists_and_limits() {
    let url = "192.168.1.5";
    let cfg =
        parse_settings(&json!({"url": url, "entities": ["light.a", "Switch.B", "sensor.c"]})).expect("array");
    assert_eq!(cfg.entities, ["light.a", "switch.b", "sensor.c"]);
    let cfg =
        parse_settings(&json!({"url": url, "entities": "light.a, Switch.B\nsensor.c"})).expect("string");
    assert_eq!(cfg.entities, ["light.a", "switch.b", "sensor.c"]);
    assert!(cfg.skipped.is_empty());

    let cfg =
        parse_settings(&json!({"url": url, "entities": ["Light Desk", "light.", "light.a", "LIGHT.A"]}))
            .expect("skips");
    assert_eq!(cfg.entities, ["light.a"]);
    assert_eq!(cfg.skipped, ["Light Desk", "light."]);

    let many: Vec<String> = (0..41).map(|i| format!("light.l{i}")).collect();
    let err = parse_settings(&json!({"url": url, "entities": many})).expect_err("41");
    assert!(err.contains("40"), "{err}");
    let ok: Vec<String> = (0..40).map(|i| format!("light.l{i}")).collect();
    assert_eq!(
        parse_settings(&json!({"url": url, "entities": ok}))
            .expect("40")
            .entities
            .len(),
        40
    );

    let cfg = parse_settings(&json!({"url": url, "attributes": ["climate.x:current_temperature", "bad"]}))
        .expect("attrs");
    assert_eq!(
        cfg.attributes,
        [("climate.x".to_string(), "current_temperature".to_string())]
    );
    assert_eq!(cfg.skipped, ["bad"]);

    let err = parse_settings(&json!({"entities": "light.a"})).expect_err("no url");
    assert!(err.contains("url"), "{err}");
    let err = parse_settings(&json!({"url": "ftp://x"})).expect_err("bad url");
    assert!(
        err.contains("address") || err.contains("url") || err.contains("http"),
        "{err}"
    );

    let cfg = parse_settings(&json!({"url": url, "token": ""})).expect("empty token");
    assert_eq!(cfg.token, None);
    let cfg = parse_settings(&json!({"url": url, "token": "abc"})).expect("token");
    assert_eq!(cfg.token.as_deref(), Some("abc"));
    assert!(
        !format!("{cfg:?}").contains("abc"),
        "Debug must not show the token"
    );
}

#[test]
fn entity_ids_are_validated() {
    for ok in ["light.desk", "sensor.temp_2", "input_boolean.a1"] {
        assert!(valid_entity_id(ok), "{ok}");
    }
    let edge = format!("light.{}", "a".repeat(249));
    assert_eq!(edge.len(), 255);
    assert!(valid_entity_id(&edge));
    let long = format!("light.{}", "a".repeat(250));
    for bad in [
        "light.a__b",
        "_light.a",
        "light_.a",
        "light._a",
        "light.a_",
        "li__ght.a",
        "light.__",
    ] {
        assert!(!valid_entity_id(bad), "{bad}");
    }
    assert!(valid_entity_id("input_boolean.a_b"));
    for bad in [
        "",
        "light",
        "Light.A",
        "light.",
        ".a",
        "1light.a",
        "light.a.b",
        "light.a-b",
        &long,
    ] {
        assert!(!valid_entity_id(bad), "{bad}");
    }
}

#[test]
fn subscription_is_filtered_to_listed_entities() {
    assert_eq!(
        subscribe_msg(2, &["light.a".into()]),
        Some(json!({"id": 2, "type": "subscribe_entities", "entity_ids": ["light.a"]}))
    );
    // An empty filter would mean every entity of the install.
    assert_eq!(subscribe_msg(2, &[]), None);
    assert_eq!(auth_msg("tok"), json!({"type": "auth", "access_token": "tok"}));
    assert_eq!(ping_msg(9), json!({"id": 9, "type": "ping"}));
    let call = ServiceCall {
        domain: "light".into(),
        service: "turn_on".into(),
        entity_id: Some("light.a".into()),
        data: serde_json::Map::new(),
    };
    let msgs = [
        auth_msg("t"),
        subscribe_msg(1, &["light.a".into()]).expect("message"),
        call_service_msg(3, &call),
        ping_msg(4),
    ];
    let types: Vec<&str> = msgs.iter().map(|m| m["type"].as_str().expect("type")).collect();
    assert_eq!(types, ["auth", "subscribe_entities", "call_service", "ping"]);
    let m = &msgs[2];
    assert_eq!(m["id"], 3);
    assert_eq!(m["domain"], "light");
    assert_eq!(m["service"], "turn_on");
    assert_eq!(m["target"], json!({"entity_id": "light.a"}));
    assert_eq!(m["service_data"], json!({}));
    let bare = ServiceCall {
        entity_id: None,
        ..call
    };
    assert!(call_service_msg(5, &bare).get("target").is_none());
}

#[test]
fn compressed_events_apply() {
    let mut t = Tracked::default();
    let snap = json!({"a": {"light.a": {"s": "on", "a": {"brightness": 128, "friendly_name": "Desk"},
        "c": "ctx", "lc": 1.0, "lu": 1.0}}});
    assert!(t.apply(&snap));
    assert!(!t.apply(&snap), "identical snapshot changes nothing");
    let e = &t.entities["light.a"];
    assert_eq!(e.state, "on");
    assert_eq!(e.attrs["brightness"], 128);

    let change = json!({"c": {"light.a": {"+": {"s": "off", "a": {"brightness": 0}, "lu": 2.0},
        "-": {"a": ["friendly_name"]}}}});
    assert!(t.apply(&change));
    let e = &t.entities["light.a"];
    assert_eq!(e.state, "off");
    assert_eq!(e.attrs["brightness"], 0);
    assert!(!e.attrs.contains_key("friendly_name"));
    assert!(!t.apply(&change), "identical repeated change");

    assert!(t.apply(&json!({"r": ["light.a"]})));
    assert!(t.entities.is_empty());
    assert!(!t.apply(&json!({"r": ["light.a"]})));
    for garbage in [
        json!(null),
        json!("x"),
        json!([1]),
        json!({"c": 5}),
        json!({"a": {"x": 3}}),
        json!({"c": {"light.zz": {"+": {"s": "on"}}}}),
        json!({"r": "no"}),
    ] {
        assert!(!t.apply(&garbage), "{garbage}");
    }
}

fn tracked(entities: &[(&str, &str, Value)]) -> Tracked {
    let mut t = Tracked::default();
    for (id, state, attrs) in entities {
        t.entities.insert(
            (*id).to_string(),
            Entity {
                state: (*state).into(),
                attrs: attrs.as_object().cloned().unwrap_or_default(),
            },
        );
    }
    t
}

fn cfg(entities: &[&str], attributes: &[(&str, &str)]) -> Config {
    Config {
        url: "ws://h:8123/api/websocket".into(),
        token: None,
        entities: entities.iter().map(|s| (*s).to_string()).collect(),
        attributes: attributes
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect(),
        skipped: vec![],
    }
}

fn pairs(t: &Tracked, c: &Config) -> std::collections::BTreeMap<String, String> {
    let live = LiveInfo {
        connected: true,
        version: "2025.1.0".into(),
        last_error: String::new(),
    };
    state_pairs(t, c, &live).into_iter().collect()
}

#[test]
fn state_pairs_derive_values() {
    let t = tracked(&[
        (
            "light.a",
            "on",
            json!({"brightness": 128, "friendly_name": "Desk"}),
        ),
        ("media_player.m", "playing", json!({"volume_level": 0.35})),
        ("fan.f", "on", json!({"percentage": 60})),
        ("cover.c", "open", json!({"current_position": 30})),
        ("sensor.s", "21.5", json!({"unit_of_measurement": "°C"})),
        (
            "climate.x",
            "heat",
            json!({"current_temperature": 21, "target": 20.5, "long": "é".repeat(2000)}),
        ),
    ]);
    let c = cfg(
        &[
            "light.a",
            "media_player.m",
            "fan.f",
            "cover.c",
            "sensor.s",
            "gone.x",
        ],
        &[
            ("climate.x", "current_temperature"),
            ("climate.x", "target"),
            ("climate.x", "long"),
        ],
    );
    let p = pairs(&t, &c);
    assert_eq!(p["connected"], "yes");
    assert_eq!(p["version"], "2025.1.0");
    assert_eq!(p["last_error"], "");
    assert_eq!(p["entity_count"], "5");
    assert_eq!(p["light.a"], "on");
    assert_eq!(p["light.a.name"], "Desk");
    assert_eq!(p["light.a.pct"], "50");
    assert_eq!(p["media_player.m.pct"], "35");
    assert_eq!(p["fan.f.pct"], "60");
    assert_eq!(p["cover.c.pct"], "30");
    assert_eq!(p["sensor.s"], "21.5");
    assert_eq!(p["sensor.s.unit"], "°C");
    assert_eq!(p["sensor.s.pct"], "");
    assert_eq!(p["gone.x"], "missing");
    assert_eq!(p["gone.x.name"], "");
    assert_eq!(p["gone.x.unit"], "");
    assert_eq!(p["gone.x.pct"], "");
    assert_eq!(p["climate.x.attr.current_temperature"], "21");
    assert_eq!(p["climate.x.attr.target"], "20.5");
    let long = &p["climate.x.attr.long"];
    assert!(long.len() <= 1024 && long.len() >= 1022, "{}", long.len());
    assert!(long.chars().all(|c| c == 'é'));
    for v in p.values() {
        assert!(v.len() <= 1024);
    }
}

#[test]
fn offline_pairs_blank_what_was_published() {
    let published: BTreeSet<String> = [
        "connected",
        "light.a",
        "light.a.name",
        "last_error",
        "version",
        "light.a.attr.x",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    let p: std::collections::BTreeMap<String, String> = offline_pairs(&published).into_iter().collect();
    assert_eq!(p["connected"], "no");
    assert_eq!(p["light.a"], "offline");
    assert_eq!(p["light.a.name"], "");
    assert_eq!(p["version"], "");
    assert_eq!(p["light.a.attr.x"], "");
    assert!(!p.contains_key("last_error"));
}

type Row = (
    &'static str,
    Value,
    (&'static str, &'static str, Option<&'static str>, Value),
);

fn call(action: &str, params: &Value, t: &Tracked) -> ServiceCall {
    match plan(action, params, t) {
        Ok(Plan::Call(c)) => c,
        other => panic!("{action} {params}: {other:?}"),
    }
}

#[test]
fn actions_plan_service_calls() {
    let t = tracked(&[("cover.c", "open", json!({"current_position": 30}))]);
    let rows: Vec<Row> = vec![
        (
            "toggle",
            json!({"entity_id": "light.a"}),
            ("homeassistant", "toggle", Some("light.a"), json!({})),
        ),
        (
            "turn_on",
            json!({"entity_id": "light.a"}),
            ("homeassistant", "turn_on", Some("light.a"), json!({})),
        ),
        (
            "turn_off",
            json!({"entity_id": "light.a"}),
            ("homeassistant", "turn_off", Some("light.a"), json!({})),
        ),
        (
            "run",
            json!({"entity_id": "scene.movie"}),
            ("scene", "turn_on", Some("scene.movie"), json!({})),
        ),
        (
            "run",
            json!({"entity_id": "script.x"}),
            ("script", "turn_on", Some("script.x"), json!({})),
        ),
        (
            "run",
            json!({"entity_id": "automation.x"}),
            ("automation", "trigger", Some("automation.x"), json!({})),
        ),
        (
            "run",
            json!({"entity_id": "button.x"}),
            ("button", "press", Some("button.x"), json!({})),
        ),
        (
            "run",
            json!({"entity_id": "input_button.x"}),
            ("input_button", "press", Some("input_button.x"), json!({})),
        ),
        (
            "set_level",
            json!({"entity_id": "light.a", "op": "set", "percent": 40}),
            ("light", "turn_on", Some("light.a"), json!({"brightness_pct": 40})),
        ),
        (
            "set_level",
            json!({"entity_id": "light.a", "op": "up", "step": 10}),
            (
                "light",
                "turn_on",
                Some("light.a"),
                json!({"brightness_step_pct": 10}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "light.a", "op": "down", "step": 10}),
            (
                "light",
                "turn_on",
                Some("light.a"),
                json!({"brightness_step_pct": -10}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "light.a", "op": "up"}),
            (
                "light",
                "turn_on",
                Some("light.a"),
                json!({"brightness_step_pct": 5}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "light.a", "op": "up", "step": 500}),
            (
                "light",
                "turn_on",
                Some("light.a"),
                json!({"brightness_step_pct": 50}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "light.a", "op": "set", "percent": 150}),
            (
                "light",
                "turn_on",
                Some("light.a"),
                json!({"brightness_pct": 100}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "media_player.m", "op": "set", "percent": 35}),
            (
                "media_player",
                "volume_set",
                Some("media_player.m"),
                json!({"volume_level": 0.35}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "media_player.m", "op": "up"}),
            ("media_player", "volume_up", Some("media_player.m"), json!({})),
        ),
        (
            "set_level",
            json!({"entity_id": "media_player.m", "op": "down"}),
            ("media_player", "volume_down", Some("media_player.m"), json!({})),
        ),
        (
            "set_level",
            json!({"entity_id": "fan.f", "op": "set", "percent": 60}),
            ("fan", "set_percentage", Some("fan.f"), json!({"percentage": 60})),
        ),
        (
            "set_level",
            json!({"entity_id": "cover.c", "op": "up", "step": 10}),
            (
                "cover",
                "set_cover_position",
                Some("cover.c"),
                json!({"position": 40}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "cover.c", "op": "down", "step": 50}),
            (
                "cover",
                "set_cover_position",
                Some("cover.c"),
                json!({"position": 0}),
            ),
        ),
        (
            "set_level",
            json!({"entity_id": "cover.c", "op": "set", "percent": 70}),
            (
                "cover",
                "set_cover_position",
                Some("cover.c"),
                json!({"position": 70}),
            ),
        ),
        (
            "call_service",
            json!({"domain": "notify", "service": "mobile_app_x", "data": "{\"message\":\"hi\"}"}),
            ("notify", "mobile_app_x", None, json!({"message": "hi"})),
        ),
        (
            "call_service",
            json!({"domain": "light", "service": "turn_on", "entity_id": "light.a", "data": ""}),
            ("light", "turn_on", Some("light.a"), json!({})),
        ),
        (
            "call_service",
            json!({"domain": "light", "service": "turn_on"}),
            ("light", "turn_on", None, json!({})),
        ),
    ];
    for (action, params, (domain, service, entity, data)) in rows {
        let c = call(action, &params, &t);
        assert_eq!(c.domain, domain, "{action} {params}");
        assert_eq!(c.service, service, "{action} {params}");
        assert_eq!(c.entity_id.as_deref(), entity, "{action} {params}");
        assert_eq!(Value::Object(c.data), data, "{action} {params}");
    }
    assert_eq!(plan("reconnect", &json!({}), &t), Ok(Plan::Reconnect));
    assert_eq!(plan("forget_token", &json!({}), &t), Ok(Plan::ForgetToken));
}

#[test]
fn actions_reject_bad_input() {
    let t = Tracked::default();
    let err = |a: &str, p: Value| plan(a, &p, &t).expect_err(&format!("{a} {p}"));

    assert!(err("run", json!({"entity_id": "switch.x"})).contains("run"));
    assert!(err("set_level", json!({"entity_id": "cover.c", "op": "up"})).contains("cover.c"));
    let e = err(
        "set_level",
        json!({"entity_id": "sensor.s", "op": "set", "percent": 3}),
    );
    for d in ["light", "media_player", "fan", "cover"] {
        assert!(e.contains(d), "{e}");
    }
    assert!(err("set_level", json!({"entity_id": "light.a", "op": "set"})).contains("percent"));
    assert!(err("set_level", json!({"entity_id": "light.a", "op": "wiggle"})).contains("op"));

    for bad in [
        json!("[1]"),
        json!("not json"),
        json!("\"s\""),
        json!("x".repeat(5000)),
    ] {
        let p = json!({"domain": "notify", "service": "x", "data": bad});
        assert!(plan("call_service", &p, &t).is_err(), "{p}");
    }
    let huge = format!("{{\"k\":\"{}\"}}", "x".repeat(4096));
    assert!(
        plan(
            "call_service",
            &json!({"domain": "a", "service": "b", "data": huge}),
            &t
        )
        .is_err()
    );
    for (d, s) in [
        ("Notify", "x"),
        ("no tify", "x"),
        ("", "x"),
        ("notify", ""),
        ("notify", "a.b"),
    ] {
        let p = json!({"domain": d, "service": s});
        assert!(plan("call_service", &p, &t).is_err(), "{p}");
    }

    assert_eq!(
        plan("explode", &json!({}), &t),
        Err("unknown action: explode".to_string())
    );
    for bad in ["Light.A", "", "light"] {
        for a in ["toggle", "turn_on", "run", "set_level"] {
            let e = err(a, json!({"entity_id": bad, "op": "up"}));
            assert!(e.contains("entity_id"), "{a} {bad:?}: {e}");
        }
        if !bad.is_empty() {
            let p = json!({"domain": "a", "service": "b", "entity_id": bad});
            assert!(err("call_service", p).contains("entity_id"));
        }
    }
    assert!(err("toggle", json!({})).contains("entity_id"));
}

#[test]
fn manifest_declares_the_actions() {
    let m = ulanzi_core::plugin::Manifest::parse(include_str!("../plugin.toml")).expect("valid");
    assert_eq!(m.id, "ha");
    assert_eq!(m.name, "Home Assistant");
    assert!(m.background);
    assert!(m.permissions.lan);
    assert!(m.permissions.network.is_empty());
    let ids: Vec<&str> = m.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "toggle",
            "turn_on",
            "turn_off",
            "run",
            "set_level",
            "call_service",
            "reconnect",
            "forget_token"
        ]
    );
}

#[test]
fn hardening_of_inputs() {
    let t = Tracked::default();
    let p = json!({"entity_id": "light.a", "op": "set", "percent": "nan"});
    assert!(plan("set_level", &p, &t).is_err());
    let p = json!({"entity_id": "light.a", "op": "up", "step": "inf"});
    assert_eq!(call("set_level", &p, &t).data["brightness_step_pct"], 5);
    for data in [json!(5), json!([1]), json!(true)] {
        let p = json!({"domain": "a", "service": "b", "data": data});
        assert!(plan("call_service", &p, &t).is_err(), "{p}");
    }
    let p = json!({"domain": "a", "service": "b", "data": {"k": "x".repeat(5000)}});
    assert!(plan("call_service", &p, &t).is_err());
    let p = json!({"domain": "a", "service": "b", "data": {"k": 1}});
    assert_eq!(call("call_service", &p, &t).data["k"], 1);

    let long = "x".repeat(300);
    let cfg = parse_settings(&json!({"url": "h", "entities": [long], "attributes": [long]})).expect("ok");
    assert!(cfg.skipped.iter().all(|s| s.chars().count() <= 64));

    // Derived keys over the host's 256-byte limit are left out.
    let id = format!("light.{}", "a".repeat(248));
    let mut c = cfg;
    c.skipped.clear();
    c.entities = vec![id.clone()];
    c.attributes = vec![(id.clone(), "brightness".into())];
    let live = LiveInfo::default();
    let keys: Vec<String> = state_pairs(&Tracked::default(), &c, &live)
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert!(keys.iter().all(|k| k.len() <= 256));
    assert!(keys.contains(&id));
    assert!(!keys.contains(&format!("{id}.name")));
}
