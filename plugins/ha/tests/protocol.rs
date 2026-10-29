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
        ("ws://h:8123", "ws://h:8123/api/websocket"),
        ("wss://ha.lan", "wss://ha.lan:443/api/websocket"),
        ("ws://h:8123/api/websocket", "ws://h:8123/api/websocket"),
        ("http://h:8123/api/websocket/", "ws://h:8123/api/websocket"),
        ("https://h/ha/api/websocket", "wss://h:443/ha/api/websocket"),
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
        ("light.off", "off", json!({"brightness": null})),
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
            "light.off",
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
    assert_eq!(p["entity_count"], "6");
    assert_eq!(p["light.a"], "on");
    assert_eq!(p["light.a.name"], "Desk");
    assert_eq!(p["light.a.pct"], "50");
    assert_eq!(p["light.off.pct"], "0");
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
    assert!(
        err(
            "set_level",
            json!({"entity_id": "light.a", "op": "set", "percent": "x"})
        )
        .contains("percent")
    );
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
    for bad in ["Light.A", "light"] {
        for a in ["toggle", "turn_on", "run", "set_level"] {
            let e = err(a, json!({"entity_id": bad, "op": "up"}));
            assert!(e.contains("entity_id"), "{a} {bad:?}: {e}");
        }
        let p = json!({"domain": "a", "service": "b", "entity_id": bad});
        assert!(err("call_service", p).contains("entity_id"));
    }
    // Nothing picked yet: say so plainly, like the OBS plugin.
    for a in ["toggle", "turn_on", "turn_off", "run", "set_level"] {
        assert_eq!(err(a, json!({})), "Choose an entity for this key first", "{a}");
        assert_eq!(
            err(a, json!({"entity_id": "  ", "op": "up"})),
            "Choose an entity for this key first",
            "{a}"
        );
    }
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
            "climate",
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

#[test]
fn the_entity_list_template_is_bounded_and_ordered() {
    let t = ENTITY_LIST_TEMPLATE;
    // Stops at the budget and says so with the marker the parser knows.
    assert_eq!(LIST_BUDGET, 200_000);
    assert!(t.contains("> 200000"), "{t}");
    // A second budget bounds the JSON bytes whatever the encoder: 2 per ASCII, 12 per other character.
    assert_eq!(LIST_WEIGHT_BUDGET, 900_000);
    assert!(t.contains("ns.w + w > 900000"), "{t}");
    assert!(
        t.contains(r"(line | length) * 2 + (line | regex_replace('[\\x00-\\x7f]', '') | length) * 10"),
        "{t}"
    );
    assert!(t.contains(TRUNCATED_MARKER));
    // Control characters (tab and newline included) are replaced in names and states.
    assert!(
        t.contains(r"regex_replace('[\\x00-\\x1f\\x7f-\\x9f]', ' ')"),
        "{t}"
    );
    assert!(t.contains("[:60]") && t.contains("[:24]"), "{t}");
    // One line per entity: id, name, state.
    assert!(t.contains(r"s.entity_id ~ '\t' ~"), "{t}");
    // Preferred domains first: every domain a picker filters on is in the preferred list.
    let pref = format!(
        "set pref = [{}]",
        PREFERRED_DOMAINS
            .iter()
            .map(|d| format!("'{d}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(t.contains(&pref), "{t}");
    for d in TOGGLEABLE.iter().chain(RUNNABLE).chain(LEVELS) {
        assert!(PREFERRED_DOMAINS.contains(d), "{d} not preferred");
    }
    let first = t
        .find("selectattr('domain', 'in', pref)")
        .expect("preferred loop");
    let rest = t.find("rejectattr('domain', 'in', pref)").expect("rest loop");
    assert!(first < rest);
    let m = render_template_msg(7);
    assert_eq!(m["id"], 7);
    assert_eq!(m["type"], "render_template");
    assert_eq!(m["template"], t);
}

#[test]
fn entity_list_lines_parse() {
    let l = parse_entity_list("light.a\tKitchen\ton\nsensor.t\tTemp\t21.5\n");
    assert!(!l.truncated);
    assert_eq!(
        l.rows,
        [
            ListedEntity {
                entity_id: "light.a".into(),
                name: "Kitchen".into(),
                state: "on".into()
            },
            ListedEntity {
                entity_id: "sensor.t".into(),
                name: "Temp".into(),
                state: "21.5".into()
            },
        ]
    );
    // Malformed lines are skipped; an empty name or state is kept as empty.
    let l =
        parse_entity_list("garbage\nBad Id\tx\ton\nlight.b\t\toff\n\nswitch.c\tPlug\t\nlight.d\tonly two\n");
    let ids: Vec<&str> = l.rows.iter().map(|r| r.entity_id.as_str()).collect();
    assert_eq!(ids, ["light.b", "switch.c"]);
    assert_eq!(l.rows[0].name, "");
    assert_eq!(l.rows[1].state, "");
    // The marker line ends the list.
    let l = parse_entity_list(&format!("light.a\tA\ton\n{TRUNCATED_MARKER}"));
    assert!(l.truncated);
    assert_eq!(l.rows.len(), 1);
    // Defensive cuts even if Home Assistant sends more than the template allows.
    let l = parse_entity_list(&format!(
        "light.a\t{}\t{}\u{7}x\n",
        "n".repeat(100),
        "s".repeat(10)
    ));
    assert_eq!(l.rows[0].name.chars().count(), 60);
    assert_eq!(l.rows[0].state, "ssssssssss x");
    assert!(parse_entity_list("").rows.is_empty());
}

#[test]
fn domains_have_plural_group_labels() {
    assert_eq!(group_label("light"), "Lights");
    assert_eq!(group_label("switch"), "Switches");
    assert_eq!(group_label("sensor"), "Sensors");
    assert_eq!(group_label("binary_sensor"), "Binary sensors");
    assert_eq!(group_label("media_player"), "Media players");
    assert_eq!(group_label("climate"), "Climate");
    assert_eq!(group_label("input_boolean"), "Input booleans");
    assert_eq!(group_label("my_custom_thing"), "My custom thing");
}

fn sample_list() -> EntityList {
    parse_entity_list(
        "light.kitchen\tKitchen lamp\ton\nswitch.kettle\tKettle\toff\nsensor.kitchen_temp\tKitchen temperature\t21.5\n\
         scene.movie\tMovie time\tunknown\nmedia_player.tv\tTV\tplaying\nlock.door\tFront door\tlocked\nlight.bare\t\toff\n",
    )
}

#[test]
fn picker_items_filter_by_picker_query_and_group() {
    let l = sample_list();
    let ids = |items: Vec<ulanzi_plugin::Item>| items.into_iter().map(|i| i.id).collect::<Vec<_>>();
    let all = picker_items(&l, "entities", "", "").expect("entities");
    assert_eq!(all.len(), 7);
    let k = all.iter().find(|i| i.id == "light.kitchen").expect("kitchen");
    assert_eq!(k.label, "Kitchen lamp");
    assert_eq!(k.subtitle.as_deref(), Some("light.kitchen"));
    assert_eq!(k.group.as_deref(), Some("Lights"));
    assert_eq!(k.state.as_deref(), Some("on"));
    let bare = all.iter().find(|i| i.id == "light.bare").expect("bare");
    assert_eq!(bare.label, "light.bare");
    // Query: case-insensitive on label or id.
    let mut got = ids(picker_items(&l, "entities", "KIT", "").expect("q"));
    got.sort();
    assert_eq!(got, ["light.kitchen", "sensor.kitchen_temp"]);
    // Group: the label the GUI saw, or the raw domain.
    assert_eq!(
        ids(picker_items(&l, "any", "", "Sensors").expect("g")),
        ["sensor.kitchen_temp"]
    );
    assert_eq!(
        ids(picker_items(&l, "any", "", "sensor").expect("g")),
        ["sensor.kitchen_temp"]
    );
    // Per-action pickers keep only what the action accepts.
    let mut t = ids(picker_items(&l, "toggleable", "", "").expect("t"));
    t.sort();
    assert_eq!(
        t,
        ["light.bare", "light.kitchen", "media_player.tv", "switch.kettle"]
    );
    assert_eq!(
        ids(picker_items(&l, "runnable", "", "").expect("r")),
        ["scene.movie"]
    );
    let mut lv = ids(picker_items(&l, "levels", "", "").expect("l"));
    lv.sort();
    assert_eq!(lv, ["light.bare", "light.kitchen", "media_player.tv"]);
    assert!(picker_items(&l, "nope", "", "").is_err());
    // Every word must match somewhere, in any order: name, id or group heading.
    assert_eq!(
        ids(picker_items(&l, "entities", "lamp kitchen", "").expect("w")),
        ["light.kitchen"]
    );
    assert_eq!(
        ids(picker_items(&l, "entities", "kitchen lights", "").expect("w")),
        ["light.kitchen"]
    );
    assert!(
        picker_items(&l, "entities", "kitchen garage", "")
            .expect("w")
            .is_empty()
    );
}

#[test]
fn the_devices_picker_lists_what_ready_made_keys_fit() {
    let l = parse_entity_list(
        "light.a\tA\ton\nclimate.ac\tBedroom Aircon\tcool\nsensor.t\tT\t21\nfan.f\tF\toff\nscript.s\tS\toff\n",
    );
    let mut got: Vec<String> = picker_items(&l, "devices", "", "")
        .expect("d")
        .into_iter()
        .map(|i| i.id)
        .collect();
    got.sort();
    assert_eq!(got, ["climate.ac", "fan.f", "light.a", "script.s"]);
    let ac: Vec<String> = picker_items(&l, "climate", "bedroom aircon", "")
        .expect("c")
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(ac, ["climate.ac"]);
}

#[test]
fn climate_keys_step_on_the_devices_own_scale() {
    let t = tracked(&[(
        "climate.ac",
        "cool",
        json!({"temperature": 24.0, "target_temp_step": 0.5, "min_temp": 16.0, "max_temp": 31.0}),
    )]);
    let temp = |op: &str| {
        let c = call("climate", &json!({"entity_id": "climate.ac", "op": op}), &t);
        assert_eq!(
            (c.domain.as_str(), c.service.as_str()),
            ("climate", "set_temperature")
        );
        c.data["temperature"].as_f64().expect("number")
    };
    assert_eq!(temp("up"), 24.5);
    assert_eq!(temp("down"), 23.5);
    // On/off follows the tracked state: running goes off by mode; off turns on in the key's mode
    // (turn_on without one); untracked: toggle.
    let power = |p: Value, t: &Tracked| call("climate", &p, t);
    let off_call = power(json!({"entity_id": "climate.ac", "op": "power"}), &t);
    assert_eq!(
        (off_call.service.as_str(), &off_call.data["hvac_mode"]),
        ("set_hvac_mode", &json!("off"))
    );
    let off = tracked(&[("climate.ac", "off", json!({}))]);
    let on = power(
        json!({"entity_id": "climate.ac", "op": "power", "mode": "cool"}),
        &off,
    );
    assert_eq!(
        (on.service.as_str(), &on.data["hvac_mode"]),
        ("set_hvac_mode", &json!("cool"))
    );
    assert_eq!(
        power(json!({"entity_id": "climate.ac", "op": "power"}), &off).service,
        "turn_on"
    );
    assert_eq!(
        power(
            json!({"entity_id": "climate.ac", "op": "power"}),
            &Tracked::default()
        )
        .service,
        "toggle"
    );
    let set = call(
        "climate",
        &json!({"entity_id": "climate.ac", "op": "set", "temperature": 40}),
        &t,
    );
    assert_eq!(
        set.data["temperature"],
        json!(31.0),
        "kept within the device's range"
    );
    let mode = call(
        "climate",
        &json!({"entity_id": "climate.ac", "op": "mode", "mode": "dry"}),
        &t,
    );
    assert_eq!(
        (mode.service.as_str(), &mode.data["hvac_mode"]),
        ("set_hvac_mode", &json!("dry"))
    );
    // Untracked: say how to fix it. Wrong domain or mode: refused.
    let none = Tracked::default();
    assert!(
        plan("climate", &json!({"entity_id": "climate.ac", "op": "up"}), &none)
            .expect_err("untracked")
            .contains("entities")
    );
    assert!(plan("climate", &json!({"entity_id": "light.a", "op": "power"}), &t).is_err());
    assert!(
        plan(
            "climate",
            &json!({"entity_id": "climate.ac", "op": "mode", "mode": "turbo"}),
            &t
        )
        .is_err()
    );
}

#[test]
fn the_manifest_offers_device_keys() {
    use ulanzi_core::plugin::key_set::Picked;
    let m = ulanzi_core::plugin::Manifest::parse(include_str!("../plugin.toml")).expect("valid");
    assert_eq!(m.rgb(), Some([0x18, 0xbc, 0xf2]));
    let set = m.key_set("devices").expect("devices");
    let item = |id: &str, group: &str| Picked {
        id: id.into(),
        name: "Lamp".into(),
        group: Some(group.into()),
    };
    let light = set.keys_for(&m, &item("light.lamp", "Lights"), true);
    assert_eq!(light.len(), 3);
    assert_eq!(light[0].look.label.text, "Lamp");
    assert_eq!(
        light[2].look.label.text, "{ha.$entity_id.pct}%",
        "filled from the key's action when shown"
    );
    assert_eq!(
        set.keys_for(&m, &item("light.lamp", "Lights"), false).len(),
        1,
        "extras left out"
    );
    let ac = set.keys_for(&m, &item("climate.ac", "Climate"), true);
    assert_eq!(ac[2].look.label.text, "{ha.$entity_id.attr.temperature}°");
    assert_eq!(
        set.tracking(&item("climate.ac", "Climate")),
        [
            ("entities".to_string(), "climate.ac".to_string()),
            ("attributes".to_string(), "climate.ac:temperature".to_string())
        ]
    );
    assert_eq!(
        set.keys_for(&m, &item("switch.x", "Switches"), true).len(),
        1,
        "the catch-all variant"
    );
    // Every icon the manifest names is in assets/.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut icons: Vec<&str> = [&m.icon, &m.symbolic_icon]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    icons.extend(m.actions.iter().filter_map(|a| a.icon.as_deref()));
    icons.extend(m.key_sets.iter().flat_map(|k| {
        k.variants
            .iter()
            .flat_map(|v| v.keys.iter().filter_map(|k| k.icon.as_deref()))
    }));
    for i in icons {
        assert!(root.join(i).is_file(), "{i} is missing");
    }
}

#[test]
fn picker_domains_match_what_the_actions_accept() {
    let tracked = Tracked::default();
    for d in RUNNABLE {
        let p = json!({"entity_id": format!("{d}.x")});
        assert!(plan("run", &p, &tracked).is_ok(), "run {d}");
    }
    for d in LEVELS {
        let p = json!({"entity_id": format!("{d}.x"), "op": "set", "percent": 10});
        assert!(plan("set_level", &p, &tracked).is_ok(), "set_level {d}");
    }
    // TOGGLEABLE is a curated list (toggle accepts any domain), so there is nothing to derive it from.
    assert!(!TOGGLEABLE.contains(&"lock"));
}

#[test]
fn set_without_a_percent_uses_the_manifest_default() {
    let tracked = Tracked::default();
    let p = json!({"entity_id": "light.x", "op": "set"});
    let plan = plan("set_level", &p, &tracked).expect("plan");
    assert!(format!("{plan:?}").contains("100"), "{plan:?}");
}
