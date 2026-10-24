use std::collections::BTreeSet;

use ulanzi_plugin::json;
use ulanzi_plugin_obs::actions::*;
use ulanzi_plugin_obs::protocol::*;

#[test]
fn auth_matches_the_documented_example() {
    // From the obs-websocket 5 protocol documentation.
    assert_eq!(
        auth_string(
            "supersecretpassword",
            "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
            "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="
        ),
        "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
    );
}

#[test]
fn identify_handles_auth() {
    let open = json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1}});
    let msg = identify(&open, None).expect("no auth needed");
    assert_eq!(msg["op"], 1);
    assert_eq!(msg["d"]["rpcVersion"], 1);
    assert_eq!(msg["d"]["eventSubscriptions"], 0x7ff);
    assert!(msg["d"].get("authentication").is_none());

    let locked = json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1,
        "authentication": {"challenge": "c", "salt": "s"}}});
    assert_eq!(identify(&locked, None), Err(HandshakeError::AuthMissing));
    assert_eq!(identify(&locked, Some("")), Err(HandshakeError::AuthMissing));
    let msg = identify(&locked, Some("pw")).expect("with password");
    assert_eq!(msg["d"]["authentication"], auth_string("pw", "s", "c"));

    let old = json!({"op": 0, "d": {"rpcVersion": 0}});
    assert_eq!(identify(&old, None), Err(HandshakeError::TooOld));
    assert_eq!(identify(&json!({"hello": 1}), None), Err(HandshakeError::TooOld));
    assert_eq!(request("7", "GetStats", json!({}))["d"]["requestId"], "7");
}

fn live() -> Tracked {
    let mut t = Tracked::default();
    t.live.connected = true;
    t.live.program = "Gaming".into();
    t.live.scenes = vec!["Gaming".into(), "Desktop".into()];
    t.items.insert(("Gaming".into(), 7), ("Cam".into(), true));
    t
}

fn parsed(action: &str, params: ulanzi_plugin::Value) -> ObsAction {
    parse_action(action, &params).unwrap_or_else(|e| panic!("{action}: {e}"))
}

#[test]
fn actions_become_requests() {
    let t = live();
    let types = |a: ObsAction| -> Vec<String> {
        match plan(&a, &t) {
            Ok(Plan::Requests(r)) => r.into_iter().map(|(ty, _)| ty).collect(),
            other => panic!("{a:?}: {other:?}"),
        }
    };
    assert_eq!(types(parsed("toggle_record", json!({}))), ["ToggleRecord"]);
    assert_eq!(types(parsed("pause_record", json!({}))), ["ToggleRecordPause"]);
    assert_eq!(types(parsed("start_stream", json!({}))), ["StartStream"]);
    assert_eq!(types(parsed("save_replay", json!({}))), ["SaveReplayBuffer"]);
    assert_eq!(
        types(parsed("toggle_virtual_cam", json!({}))),
        ["ToggleVirtualCam"]
    );
    assert_eq!(
        types(parsed("studio_transition", json!({}))),
        ["TriggerStudioModeTransition"]
    );
    assert_eq!(types(parsed("screenshot", json!({}))), ["TriggerHotkeyByName"]);
    assert_eq!(
        types(parsed("set_scene", json!({"scene": "Desktop", "preview": true}))),
        ["SetCurrentPreviewScene"]
    );
    // Toggle two scenes: away from the one on air.
    match plan(
        &parsed("toggle_scenes", json!({"a": "Gaming", "b": "Desktop"})),
        &t,
    ) {
        Ok(Plan::Requests(r)) => assert_eq!(r[0].1["sceneName"], "Desktop"),
        other => panic!("{other:?}"),
    }
    // Show/hide flips the scene item's current state.
    match plan(
        &parsed("toggle_source", json!({"scene": "Gaming", "source": "Cam"})),
        &t,
    ) {
        Ok(Plan::Requests(r)) => {
            assert_eq!(r[0].0, "SetSceneItemEnabled");
            assert_eq!(r[0].1["sceneItemId"], 7);
            assert_eq!(r[0].1["sceneItemEnabled"], false);
        }
        other => panic!("{other:?}"),
    }
    let missing = plan(
        &parsed("toggle_source", json!({"scene": "Gaming", "source": "Nope"})),
        &t,
    );
    assert!(matches!(missing, Err(e) if e.contains("Nope")));
    assert!(matches!(
        plan(
            &parsed("input_volume", json!({"input": "Mic", "op": "up", "step": 2})),
            &t
        ),
        Ok(Plan::InputVolume { .. })
    ));
    assert!(matches!(
        plan(&parsed("input_volume", json!({"input": "Mic", "op": "mute"})), &t),
        Ok(Plan::Requests(r)) if r[0].0 == "ToggleInputMute"
    ));
    assert!(matches!(
        plan(
            &parsed("toggle_filter", json!({"source": "Cam", "filter": "Blur"})),
            &t
        ),
        Ok(Plan::ToggleFilter { .. })
    ));
    let empty = ObsAction::SetScene {
        scene: String::new(),
        preview: false,
    };
    assert!(plan(&empty, &t).is_err());
}

#[test]
fn input_volume_clamps() {
    assert_eq!(volume_db(-10.0, VolumeOp::Up, 2), -8.0);
    assert_eq!(volume_db(-1.0, VolumeOp::Up, 2), 0.0);
    assert_eq!(volume_db(-99.0, VolumeOp::Down, 3), -100.0);
    // Set takes percent of full (linear) volume.
    assert!((volume_db(0.0, VolumeOp::Set { percent: 50 }, 2) - (-6.0206)).abs() < 0.01);
    assert_eq!(volume_db(-3.0, VolumeOp::Set { percent: 0 }, 2), -100.0);
    // Percent from the key's params is clamped.
    assert_eq!(
        parsed(
            "input_volume",
            json!({"input": "Mic", "op": "set", "percent": 250})
        ),
        ObsAction::InputVolume {
            input: "Mic".into(),
            op: VolumeOp::Set { percent: 100 },
            step: 2
        }
    );
}

#[test]
fn events_update_the_snapshot() {
    let mut t = live();
    let e = apply_event(
        &mut t,
        "CurrentProgramSceneChanged",
        &json!({"sceneName": "Desktop"}),
    );
    assert!(e.changed);
    assert_eq!(t.live.program, "Desktop");
    apply_event(
        &mut t,
        "InputMuteStateChanged",
        &json!({"inputName": "Mic", "inputMuted": true}),
    );
    assert!(t.live.muted.contains("Mic"));
    apply_event(
        &mut t,
        "SceneItemEnableStateChanged",
        &json!({"sceneName": "Gaming", "sceneItemId": 7, "sceneItemEnabled": false}),
    );
    assert!(t.live.hidden.contains(&("Gaming".into(), "Cam".into())));
    let e = apply_event(
        &mut t,
        "RecordStateChanged",
        &json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
    assert_eq!(e.record, Some(OutputChange::Started));
    assert!(t.live.recording && !t.live.rec_paused);
    apply_event(
        &mut t,
        "RecordStateChanged",
        &json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_PAUSED"}),
    );
    assert!(t.live.rec_paused);
    apply_event(
        &mut t,
        "StudioModeStateChanged",
        &json!({"studioModeEnabled": true}),
    );
    assert!(t.live.preview.is_some());
    apply_event(
        &mut t,
        "SceneNameChanged",
        &json!({"oldSceneName": "Gaming", "sceneName": "Game"}),
    );
    assert!(t.live.scenes.contains(&"Game".to_string()));
    assert!(t.live.hidden.contains(&("Game".into(), "Cam".into())));
    apply_event(
        &mut t,
        "InputNameChanged",
        &json!({"oldInputName": "Mic", "inputName": "Microphone"}),
    );
    assert!(t.live.muted.contains("Microphone"));
    assert!(apply_event(&mut t, "SceneCreated", &json!({"sceneName": "New"})).reload);
    assert!(!apply_event(&mut t, "SomethingElse", &json!({})).changed);
}

#[test]
fn stopwatch_counts_and_pauses() {
    use std::time::{Duration, Instant};
    let t0 = Instant::now();
    let mut w = Stopwatch::default();
    w.start(t0, 0);
    assert_eq!(w.secs(t0 + Duration::from_secs(65)), 65);
    w.pause(t0 + Duration::from_secs(65));
    assert_eq!(w.secs(t0 + Duration::from_secs(200)), 65);
    w.resume(t0 + Duration::from_secs(200));
    assert_eq!(w.secs(t0 + Duration::from_secs(210)), 75);
    w.stop();
    assert_eq!(w.secs(t0 + Duration::from_secs(300)), 0);
    // Joining a recording already running: start from OBS's own duration.
    w.start(t0, 3600);
    assert_eq!(w.secs(t0 + Duration::from_secs(1)), 3601);
}

#[test]
fn clock_formats() {
    assert_eq!(clock(0), "00:00");
    assert_eq!(clock(83), "01:23");
    assert_eq!(clock(3661), "1:01:01");
}

#[test]
fn parse_errors_use_the_builtin_wording() {
    assert_eq!(
        parse_action("set_scene", &json!({})),
        Err("Choose a scene for this key first".to_string())
    );
    assert_eq!(
        parse_action("set_scene", &json!({"scene": "  "})),
        Err("Choose a scene for this key first".to_string())
    );
    assert_eq!(
        parse_action("input_volume", &json!({"input": "Mic", "op": "loud"})),
        Err("op must be up, down, mute or set".to_string())
    );
    assert_eq!(
        parse_action("nope", &json!({})),
        Err("unknown action: nope".to_string())
    );
    match parse_action("input_volume", &json!({"input": "Mic", "op": "up"})) {
        Ok(ObsAction::InputVolume { step, .. }) => assert_eq!(step, 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn every_manifest_action_is_known() {
    let text = include_str!("../plugin.toml");
    let mut ids = Vec::new();
    let mut in_action = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("[[") || line.starts_with('[') {
            in_action = line == "[[actions]]";
        } else if in_action && let Some(rest) = line.strip_prefix("id = \"") {
            ids.push(rest.trim_end_matches('"').to_string());
        }
    }
    assert_eq!(ids.len(), 22, "{ids:?}");
    for id in ids {
        if id == "reconnect" || id == "forget_password" {
            continue;
        }
        let r = parse_action(&id, &json!({}));
        assert_ne!(r, Err(format!("unknown action: {id}")), "{id}");
    }
}

#[test]
fn state_pairs_offline_and_live() {
    let has = |v: &[(String, String)], k: &str, val: &str| v.contains(&(k.to_string(), val.to_string()));
    let off = state_pairs(&Live::default(), &BTreeSet::new());
    assert!(has(&off, "connected", "no"));
    assert!(has(&off, "scene", ""));
    assert!(has(&off, "rec_time", ""));
    assert!(has(&off, "fps", ""));
    assert_eq!(off.len(), 16);

    let mut l = Live {
        connected: true,
        program: "A".into(),
        recording: true,
        rec_secs: 83,
        muted: BTreeSet::from(["Mic".to_string()]),
        hidden: BTreeSet::from([("A".to_string(), "Cam".to_string())]),
        stats: Some(Stats {
            fps: 60,
            cpu: 3,
            dropped_permille: 4,
        }),
        ..Live::default()
    };
    l.scenes = vec!["A".into(), "B".into()];
    l.inputs = BTreeSet::from(["Desk".to_string()]);
    l.sources = BTreeSet::from([("A".to_string(), "Logo".to_string())]);
    l.last_error = "boom".into();
    let on = state_pairs(&l, &BTreeSet::new());
    for (k, v) in [
        ("connected", "yes"),
        ("scene", "A"),
        ("scene_count", "2"),
        ("recording", "yes"),
        ("rec_time", "01:23"),
        ("mute.Mic", "yes"),
        ("visible.A/Cam", "no"),
        ("fps", "60"),
        ("cpu", "3"),
        ("dropped", "0.4%"),
        ("mute.Desk", "no"),
        ("visible.A/Logo", "yes"),
        ("last_error", "boom"),
    ] {
        assert!(has(&on, k, v), "{k}={v} in {on:?}");
    }
    assert_eq!(on.len(), 16 + 4);
}

#[test]
fn dynamic_keys_are_capped() {
    let l = Live {
        connected: true,
        muted: (0..300).map(|i| format!("in{i:03}")).collect(),
        hidden: BTreeSet::from([("A".to_string(), "Cam".to_string())]),
        ..Live::default()
    };
    let pairs = state_pairs(&l, &BTreeSet::new());
    assert_eq!(pairs.iter().filter(|(k, _)| k.starts_with("mute.")).count(), 200);
    assert_eq!(pairs.iter().filter(|(k, _)| k.starts_with("visible.")).count(), 0);
    assert_eq!(pairs.len(), 216);
}

#[test]
fn offline_clears_published_dynamic_keys() {
    let l = Live {
        connected: true,
        muted: BTreeSet::from(["Mic".to_string()]),
        hidden: BTreeSet::from([("A".to_string(), "Cam".to_string())]),
        ..Live::default()
    };
    let published: BTreeSet<String> = state_pairs(&l, &BTreeSet::new())
        .into_iter()
        .map(|(k, _)| k)
        .filter(|k| k.contains('.'))
        .collect();
    assert_eq!(published.len(), 2);
    let off = state_pairs(&Live::default(), &published);
    for k in &published {
        assert!(off.contains(&(k.clone(), String::new())), "{k}");
    }
    // A key still present keeps its value; only absent ones are blanked.
    let again = state_pairs(&l, &published);
    assert!(again.contains(&("mute.Mic".to_string(), "yes".to_string())));
    assert_eq!(again.len(), 18);
}

#[test]
fn input_events_track_known_inputs() {
    let mut t = live();
    apply_event(
        &mut t,
        "InputMuteStateChanged",
        &json!({"inputName": "Mic", "inputMuted": false}),
    );
    assert!(t.live.inputs.contains("Mic") && !t.live.muted.contains("Mic"));
    apply_event(
        &mut t,
        "InputNameChanged",
        &json!({"oldInputName": "Mic", "inputName": "Voice"}),
    );
    assert!(t.live.inputs.contains("Voice") && !t.live.inputs.contains("Mic"));
    let e = apply_event(&mut t, "InputRemoved", &json!({"inputName": "Voice"}));
    assert!(e.reload && t.live.inputs.is_empty());
    // Scene items feed the known sources.
    t.sync_hidden();
    assert!(t.live.sources.contains(&("Gaming".into(), "Cam".into())));
}

#[test]
fn volume_params_are_strict() {
    assert_eq!(
        parse_action("input_volume", &json!({"input": "Mic", "op": "set"})),
        Err("Choose a volume".to_string())
    );
    match parse_action("input_volume", &json!({"input": "Mic", "op": "up", "step": 0})) {
        Ok(ObsAction::InputVolume { step, .. }) => assert_eq!(step, 1),
        other => panic!("{other:?}"),
    }
    match parse_action("input_volume", &json!({"input": "Mic", "op": "up", "step": 99})) {
        Ok(ObsAction::InputVolume { step, .. }) => assert_eq!(step, 20),
        other => panic!("{other:?}"),
    }
}

#[test]
fn manifest_is_valid_for_the_host() {
    let m = ulanzi_core::plugin::Manifest::parse(include_str!("../plugin.toml")).expect("valid");
    assert_eq!(m.id, "obs");
    assert_eq!(m.actions.len(), 22);
    assert!(m.background);
}
