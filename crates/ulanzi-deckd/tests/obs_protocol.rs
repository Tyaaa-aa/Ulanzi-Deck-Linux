use serde_json::json;
use ulanzi_core::model::{ObsAction, VolumeOp};
use ulanzi_core::obs::ObsLive;
use ulanzi_deckd::obs::protocol::*;

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
}

#[test]
fn close_codes_explain_themselves() {
    assert_eq!(close_reason(4009), HandshakeError::AuthFailed);
    assert_eq!(close_reason(4010), HandshakeError::TooOld);
    assert_eq!(close_reason(1000), HandshakeError::Dropped);
}

fn live() -> Tracked {
    let mut t = Tracked::default();
    t.live.connected = true;
    t.live.program = "Gaming".into();
    t.live.scenes = vec!["Gaming".into(), "Desktop".into()];
    t.items.insert(("Gaming".into(), 7), ("Cam".into(), true));
    t
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
    assert_eq!(types(ObsAction::ToggleRecord), ["ToggleRecord"]);
    assert_eq!(types(ObsAction::PauseRecord), ["ToggleRecordPause"]);
    assert_eq!(types(ObsAction::StartStream), ["StartStream"]);
    assert_eq!(types(ObsAction::SaveReplay), ["SaveReplayBuffer"]);
    assert_eq!(types(ObsAction::ToggleVirtualCam), ["ToggleVirtualCam"]);
    assert_eq!(
        types(ObsAction::StudioTransition),
        ["TriggerStudioModeTransition"]
    );
    assert_eq!(types(ObsAction::Screenshot), ["TriggerHotkeyByName"]);
    assert_eq!(
        types(ObsAction::SetScene {
            scene: "Desktop".into(),
            preview: true
        }),
        ["SetCurrentPreviewScene"]
    );
    // Toggle two scenes: away from the one on air.
    match plan(
        &ObsAction::ToggleScenes {
            a: "Gaming".into(),
            b: "Desktop".into(),
        },
        &t,
    ) {
        Ok(Plan::Requests(r)) => assert_eq!(r[0].1["sceneName"], "Desktop"),
        other => panic!("{other:?}"),
    }
    // Show/hide flips the scene item's current state.
    match plan(
        &ObsAction::ToggleSource {
            scene: "Gaming".into(),
            source: "Cam".into(),
        },
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
        &ObsAction::ToggleSource {
            scene: "Gaming".into(),
            source: "Nope".into(),
        },
        &t,
    );
    assert!(matches!(missing, Err(e) if e.contains("Nope")));
    assert!(matches!(
        plan(
            &ObsAction::InputVolume {
                input: "Mic".into(),
                op: VolumeOp::Up,
                step: 2
            },
            &t
        ),
        Ok(Plan::InputVolume { .. })
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
    apply_event(
        &mut t,
        "RecordStateChanged",
        &json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
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
    let e = apply_event(
        &mut t,
        "SceneNameChanged",
        &json!({"oldSceneName": "Gaming", "sceneName": "Game"}),
    );
    assert_eq!(e.rename, Some(Rename::Scene("Gaming".into(), "Game".into())));
    assert!(t.live.scenes.contains(&"Game".to_string()));
    assert!(t.live.hidden.contains(&("Game".into(), "Cam".into())));
    let e = apply_event(
        &mut t,
        "InputNameChanged",
        &json!({"oldInputName": "Mic", "inputName": "Microphone"}),
    );
    assert_eq!(e.rename, Some(Rename::Input("Mic".into(), "Microphone".into())));
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
    let _ = ObsLive::default();
}
