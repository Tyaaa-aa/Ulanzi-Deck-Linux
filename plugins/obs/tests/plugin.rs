//! The OBS plugin against the SDK's fake host: connection lifecycle, requests and live state.
use std::time::Duration;

use serde_json::Value;
use ulanzi_plugin::Item;
use ulanzi_plugin::testing::Harness;
use ulanzi_plugin::{Level, Payload, Socket, Status, Timer, json};
use ulanzi_plugin_obs::Obs;

const PASSWORD: &str = "supersecretpassword";
const SALT: &str = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
const CHALLENGE: &str = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";
const AUTH: &str = "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=";

fn hello(auth: bool) -> Payload {
    let mut d = json!({"obsWebSocketVersion": "5.5.4", "rpcVersion": 1});
    if auth {
        d["authentication"] = json!({"challenge": CHALLENGE, "salt": SALT});
    }
    Payload::Text(json!({"op": 0, "d": d}).to_string())
}

fn texts(h: &Harness<Obs>, s: Socket) -> Vec<Value> {
    h.sent(s)
        .iter()
        .filter_map(|p| match p {
            Payload::Text(t) => serde_json::from_str(t).ok(),
            Payload::Binary(_) => None,
        })
        .collect()
}

/// Requests sent on `s` so far: (id, type, data).
fn sent_requests(h: &Harness<Obs>, s: Socket) -> Vec<(String, String, Value)> {
    texts(h, s)
        .into_iter()
        .filter(|m| m["op"] == 6)
        .map(|m| {
            (
                m["d"]["requestId"].as_str().unwrap_or_default().to_string(),
                m["d"]["requestType"].as_str().unwrap_or_default().to_string(),
                m["d"]["requestData"].clone(),
            )
        })
        .collect()
}

/// Answer a request. A failed one takes OBS's comment from `data["comment"]`.
fn answer(h: &mut Harness<Obs>, s: Socket, id: &str, ty: &str, ok: bool, data: Value) {
    let comment = data.get("comment").cloned();
    let mut status = json!({"result": ok, "code": if ok { 100 } else { 600 }});
    if let Some(c) = comment {
        status["comment"] = c;
    }
    let msg = json!({"op": 7, "d": {"requestType": ty, "requestId": id,
        "requestStatus": status, "responseData": data}});
    h.message(s, Payload::Text(msg.to_string()));
}

fn event(h: &mut Harness<Obs>, s: Socket, ty: &str, data: Value) {
    let msg = json!({"op": 5, "d": {"eventType": ty, "eventIntent": 1, "eventData": data}});
    h.message(s, Payload::Text(msg.to_string()));
}

fn settings() -> Value {
    json!({"host": "127.0.0.1", "port": 4455})
}

fn current(h: &Harness<Obs>) -> Socket {
    h.opened().last().expect("a socket was opened").0
}

/// Init, open, Hello, Identified, then the whole load answered.
fn live(h: &mut Harness<Obs>) -> Socket {
    live_with(h, settings(), false)
}

fn live_with(h: &mut Harness<Obs>, settings: Value, auth: bool) -> Socket {
    h.init(settings).expect("init");
    let s = current(h);
    h.open(s);
    h.message(s, hello(auth));
    h.message(
        s,
        Payload::Text(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string()),
    );
    let mut done = 0;
    loop {
        let reqs = sent_requests(h, s);
        if done == reqs.len() {
            break;
        }
        for (id, ty, data) in reqs[done..].iter().cloned() {
            let (ok, body) = match ty.as_str() {
                "GetVersion" => (
                    true,
                    json!({"obsVersion": "31.0.2", "obsWebSocketVersion": "5.5.4"}),
                ),
                "GetSceneList" => (
                    true,
                    json!({"currentProgramSceneName": "A", "currentPreviewSceneName": null,
                        "scenes": [{"sceneName": "B"}, {"sceneName": "A"}]}),
                ),
                "GetStudioModeEnabled" => (true, json!({"studioModeEnabled": false})),
                "GetRecordStatus" | "GetStreamStatus" => (
                    true,
                    json!({"outputActive": false, "outputPaused": false, "outputDuration": 0}),
                ),
                "GetReplayBufferStatus" | "GetVirtualCamStatus" => (true, json!({"outputActive": false})),
                "GetInputList" => (
                    true,
                    json!({"inputs": [{"inputName": "Mic"}, {"inputName": "Cam"}]}),
                ),
                "GetSceneItemList" if data["sceneName"] == "A" => (
                    true,
                    json!({"sceneItems": [{"sceneItemId": 1, "sourceName": "Cam", "sceneItemEnabled": true}]}),
                ),
                "GetSceneItemList" => (true, json!({"sceneItems": []})),
                "GetInputMute" if data["inputName"] == "Mic" => (true, json!({"inputMuted": true})),
                "GetInputMute" => (false, json!({"comment": "not an audio input"})),
                "GetSceneCollectionList" => (
                    true,
                    json!({"currentSceneCollectionName": "Main", "sceneCollections": ["Main", "Stream"]}),
                ),
                "GetProfileList" => (
                    true,
                    json!({"currentProfileName": "Default", "profiles": ["Default", "Quiet"]}),
                ),
                "GetSourceFilterList" if data["sourceName"] == "Cam" => (
                    true,
                    json!({"filters": [
                        {"filterName": "Color", "filterEnabled": true, "filterIndex": 0},
                        {"filterName": "Sharpen", "filterEnabled": false, "filterIndex": 1}]}),
                ),
                "GetSourceFilterList" if data["sourceName"] == "Mic" => (
                    true,
                    json!({"filters": [{"filterName": "Noise", "filterEnabled": true, "filterIndex": 0}]}),
                ),
                "GetSourceFilterList" => (true, json!({"filters": []})),
                other => panic!("unexpected load request {other}"),
            };
            answer(h, s, &id, &ty, ok, body);
        }
        done = reqs.len();
    }
    s
}

/// The single one-shot timer that is not the 5 s handshake timer.
fn retry_timer(h: &Harness<Obs>) -> (Timer, Duration) {
    let t: Vec<_> = h
        .timers()
        .into_iter()
        .filter(|(_, d, repeat)| !repeat && *d != Duration::from_secs(5))
        .collect();
    assert_eq!(t.len(), 1, "exactly one retry timer: {:?}", h.timers());
    (t[0].0, t[0].1)
}

fn one_shots(h: &Harness<Obs>) -> usize {
    h.timers().iter().filter(|(_, _, repeat)| !repeat).count()
}

fn tick(h: &Harness<Obs>) -> Timer {
    h.timers()
        .into_iter()
        .find(|(_, d, repeat)| *repeat && *d == Duration::from_millis(1000))
        .expect("tick timer")
        .0
}

fn status_msg(h: &Harness<Obs>) -> (Status, String) {
    h.status().expect("a status")
}

#[test]
fn connects_with_password_and_loads() {
    let mut h = Harness::<Obs>::new();
    h.init(json!({"host": "127.0.0.1", "port": 4455, "password": PASSWORD}))
        .expect("init");
    assert_eq!(h.opened()[0].1, "ws://127.0.0.1:4455");
    assert_eq!(
        status_msg(&h),
        (
            Status::Connecting,
            "Connecting to OBS at 127.0.0.1:4455…".to_string()
        )
    );
    let s = h.opened()[0].0;
    h.open(s);
    h.message(s, hello(true));
    let first = &texts(&h, s)[0];
    assert_eq!(first["op"], 1);
    assert_eq!(first["d"]["authentication"], AUTH);

    let mut h = Harness::<Obs>::new();
    h.secrets().insert("password".into(), PASSWORD.into());
    live(&mut h);
    assert_eq!(h.state("connected"), Some("yes"));
    assert_eq!(h.state("scene"), Some("A"));
    assert_eq!(h.state("scene_count"), Some("2"));
    assert_eq!(h.state("mute.Mic"), Some("yes"));
    assert_eq!(h.state("mute.Cam"), None);
    assert_eq!(h.state("visible.A/Cam"), Some("yes"));
    assert_eq!(
        status_msg(&h),
        (Status::Ok, "OBS 31.0.2 · WebSocket 5.5.4 · 2 scenes".to_string())
    );
}

#[test]
fn password_moves_to_the_keyring() {
    let mut h = Harness::<Obs>::new();
    h.init(json!({"host": "127.0.0.1", "port": 4455, "password": PASSWORD}))
        .expect("init");
    assert_eq!(h.secrets()["password"], PASSWORD);
    assert!(
        h.logs()
            .iter()
            .any(|(l, m)| *l == Level::Warn && m.contains("[plugins.obs.settings]"))
    );
    // Run the whole load, then look at every state key, the status and the log.
    let mut h = Harness::<Obs>::new();
    live_with(
        &mut h,
        json!({"host": "127.0.0.1", "port": 4455, "password": PASSWORD}),
        true,
    );
    assert_eq!(h.state("connected"), Some("yes"));
    for (key, value) in h.states() {
        assert!(!key.contains(PASSWORD) && !value.contains(PASSWORD), "{key}");
    }
    assert!(!status_msg(&h).1.contains(PASSWORD));
    assert!(h.logs().iter().all(|(_, m)| !m.contains(PASSWORD)));

    // A second start with the secret only (the line deleted from settings.toml).
    let mut h2 = Harness::<Obs>::new();
    h2.secrets().insert("password".into(), PASSWORD.into());
    h2.init(json!({})).expect("init");
    assert_eq!(h2.opened()[0].1, "ws://localhost:4455");
    let s = h2.opened()[0].0;
    h2.open(s);
    h2.message(s, hello(true));
    assert_eq!(texts(&h2, s)[0]["d"]["authentication"], AUTH);
}

#[test]
fn no_password_reports_auth_missing() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, hello(true));
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(msg.starts_with("Password needed"), "{msg}");
    assert!(msg.contains("Settings → Integrations → OBS Studio"), "{msg}");
    assert_eq!(retry_timer(&h).1, Duration::from_millis(1000));
}

#[test]
fn wrong_password_is_reported() {
    let mut h = Harness::<Obs>::new();
    h.init(json!({"host": "127.0.0.1", "port": 4455, "password": "nope"}))
        .expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, hello(true));
    h.closed(s, "Authentication failed.");
    let (st, msg) = status_msg(&h);
    assert_eq!(st, Status::Problem);
    assert!(
        msg.starts_with("OBS closed the connection during sign-in"),
        "{msg}"
    );
    assert_eq!(h.state("connected"), Some("no"));
}

#[test]
fn refused_backs_off() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    for want in [1000, 2000, 4000, 8000, 10000, 10000] {
        let s = current(&h);
        h.closed(s, "connection refused");
        assert!(one_shots(&h) <= 1, "{:?}", h.timers());
        let (t, d) = retry_timer(&h);
        assert_eq!(d, Duration::from_millis(want));
        h.fire(t);
        assert!(one_shots(&h) <= 1, "{:?}", h.timers());
    }
    assert_eq!(h.opened().len(), 7);
    let lines = h
        .logs()
        .iter()
        .filter(|(l, m)| *l == Level::Info && m.contains("OBS is closed"))
        .count();
    assert_eq!(lines, 1);
}

#[test]
fn actions_send_requests() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let last = |h: &Harness<Obs>| sent_requests(h, s).pop().expect("a request");

    h.action("set_scene", json!({"scene": "B"})).expect("ok");
    let (_, ty, data) = last(&h);
    assert_eq!(
        (ty.as_str(), data),
        ("SetCurrentProgramScene", json!({"sceneName": "B"}))
    );

    h.action("toggle_source", json!({"scene": "A", "source": "Cam"}))
        .expect("ok");
    let (_, ty, data) = last(&h);
    assert_eq!(ty, "SetSceneItemEnabled");
    assert_eq!(
        data,
        json!({"sceneName": "A", "sceneItemId": 1, "sceneItemEnabled": false})
    );

    h.action("input_volume", json!({"input": "Mic", "op": "up", "step": 3}))
        .expect("ok");
    let (id, ty, _) = last(&h);
    assert_eq!(ty, "GetInputVolume");
    answer(&mut h, s, &id, &ty, true, json!({"inputVolumeDb": -1.0}));
    let (_, ty, data) = last(&h);
    assert_eq!(ty, "SetInputVolume");
    assert_eq!(data, json!({"inputName": "Mic", "inputVolumeDb": 0.0}));

    h.action("toggle_filter", json!({"source": "Cam", "filter": "Blur"}))
        .expect("ok");
    let (id, ty, _) = last(&h);
    assert_eq!(ty, "GetSourceFilter");
    answer(&mut h, s, &id, &ty, true, json!({"filterEnabled": true}));
    let (_, ty, data) = last(&h);
    assert_eq!(ty, "SetSourceFilterEnabled");
    assert_eq!(
        data,
        json!({"sourceName": "Cam", "filterName": "Blur", "filterEnabled": false})
    );
}

#[test]
fn volume_up_from_silence_stays_quiet() {
    // At 0 % OBS's volume in dB is minus infinity, which its JSON writes as null.
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    h.action("input_volume", json!({"input": "Mic", "op": "up", "step": 2}))
        .expect("ok");
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    answer(
        &mut h,
        s,
        &id,
        &ty,
        true,
        json!({"inputVolumeDb": null, "inputVolumeMul": 0.0}),
    );
    let (_, ty, data) = sent_requests(&h, s).pop().expect("request");
    assert_eq!(ty, "SetInputVolume");
    assert_eq!(data, json!({"inputName": "Mic", "inputVolumeDb": -98.0}));
}

#[test]
fn action_while_offline_errors() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    let e = h.action("toggle_record", json!({})).expect_err("offline");
    assert!(e.starts_with("OBS isn’t connected: "), "{e}");
}

#[test]
fn refused_request_sets_last_error() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    h.action("set_scene", json!({"scene": "Missing"})).expect("ok");
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    let why = "No source was found by the name of `Missing`.";
    answer(&mut h, s, &id, &ty, false, json!({"comment": why}));
    assert_eq!(h.state("last_error"), Some(why));
    assert!(h.logs().iter().any(|(l, m)| *l == Level::Warn && m.contains(why)));

    h.action("set_scene", json!({"scene": "B"})).expect("ok");
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    answer(&mut h, s, &id, &ty, true, json!({}));
    assert_eq!(h.state("last_error"), Some(""));
}

#[test]
fn events_update_state() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    event(
        &mut h,
        s,
        "RecordStateChanged",
        json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
    assert_eq!(h.state("recording"), Some("yes"));
    let t = h.state("rec_time").expect("rec_time").to_string();
    assert!(
        t.len() == 5 && t.starts_with("00:0") && t.as_bytes()[4].is_ascii_digit(),
        "{t}"
    );
    event(
        &mut h,
        s,
        "InputMuteStateChanged",
        json!({"inputName": "Mic", "inputMuted": false}),
    );
    assert_eq!(h.state("mute.Mic"), Some("no"));
    event(&mut h, s, "CurrentProgramSceneChanged", json!({"sceneName": "B"}));
    assert_eq!(h.state("scene"), Some("B"));

    let before = sent_requests(&h, s).len();
    event(&mut h, s, "SceneCreated", json!({"sceneName": "C"}));
    assert_eq!(
        sent_requests(&h, s).len(),
        before,
        "the reload waits for the tick"
    );
    let t = tick(&h);
    h.fire(t);
    let new: Vec<String> = sent_requests(&h, s)[before..]
        .iter()
        .map(|r| r.1.clone())
        .collect();
    assert!(new.contains(&"GetSceneList".to_string()), "{new:?}");
    assert!(new.contains(&"GetInputList".to_string()), "{new:?}");
}

#[test]
fn unanswered_request_times_out() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    h.action("set_scene", json!({"scene": "B"})).expect("ok");
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    let t = tick(&h);
    for _ in 0..6 {
        h.fire(t);
    }
    assert!(
        h.logs()
            .iter()
            .any(|(l, m)| *l == Level::Warn && m == "OBS didn’t answer SetCurrentProgramScene")
    );
    // A late failed answer is no longer looked at.
    answer(&mut h, s, &id, &ty, false, json!({"comment": "late"}));
    assert_eq!(h.state("last_error"), Some(""));
}

#[test]
fn stats_every_two_ticks() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let t = tick(&h);
    let count = |h: &Harness<Obs>| sent_requests(h, s).iter().filter(|r| r.1 == "GetStats").count();
    h.fire(t);
    assert_eq!(count(&h), 0);
    h.fire(t);
    assert_eq!(count(&h), 1);
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    answer(
        &mut h,
        s,
        &id,
        &ty,
        true,
        json!({"activeFps": 59.9, "cpuUsage": 2.6, "outputTotalFrames": 1000, "outputSkippedFrames": 4,
            "renderTotalFrames": 0, "renderSkippedFrames": 0}),
    );
    assert_eq!(h.state("fps"), Some("60"));
    assert_eq!(h.state("cpu"), Some("3"));
    assert_eq!(h.state("dropped"), Some("0.4%"));
}

#[test]
fn stale_socket_is_ignored() {
    let mut h = Harness::<Obs>::new();
    let s1 = live(&mut h);
    h.action("reconnect", json!({})).expect("reconnect");
    let s2 = current(&h);
    assert_ne!(s1, s2);
    let status = status_msg(&h);
    let timers = h.timers().len();
    event(
        &mut h,
        s1,
        "CurrentProgramSceneChanged",
        json!({"sceneName": "B"}),
    );
    h.closed(s1, "connection lost");
    assert_eq!(h.timers().len(), timers);
    assert_eq!(status_msg(&h), status);
    assert_ne!(h.state("scene"), Some("B"));
    assert_eq!(one_shots(&h), 1, "only the new handshake timer: {:?}", h.timers());
}

#[test]
fn disconnect_clears_state() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    event(
        &mut h,
        s,
        "RecordStateChanged",
        json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
    assert_eq!(h.state("recording"), Some("yes"));
    h.closed(s, "connection lost");
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(h.state("scene"), Some(""));
    assert_eq!(h.state("recording"), Some("no"));
    assert_eq!(h.state("rec_time"), Some(""));
    assert_eq!(h.state("mute.Mic"), Some(""));
    assert_eq!(h.state("visible.A/Cam"), Some(""));
    assert_eq!(
        status_msg(&h),
        (Status::Connecting, "Connection lost. Reconnecting…".to_string())
    );
}

#[test]
fn settings_change_reconnects() {
    let mut h = Harness::<Obs>::new();
    live(&mut h);
    let opened = h.opened().len();
    let status = status_msg(&h);
    h.init(settings()).expect("same settings");
    assert_eq!(h.opened().len(), opened);
    assert_eq!(status_msg(&h), status);
    assert_eq!(h.state("connected"), Some("yes"));

    h.init(json!({"host": "127.0.0.1", "port": 4456}))
        .expect("new port");
    assert_eq!(h.opened().len(), opened + 1);
    assert_eq!(h.opened().last().expect("socket").1, "ws://127.0.0.1:4456");

    let opened = h.opened().len();
    assert_eq!(
        h.init(json!({"host": "127.0.0.1", "port": 0})),
        Err("port must be a number from 1 to 65535".to_string())
    );
    assert_eq!(h.opened().len(), opened);
}

#[test]
fn handshake_timeout_retries() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    let s = current(&h);
    h.open(s);
    let t = h
        .timers()
        .into_iter()
        .find(|(_, d, _)| *d == Duration::from_secs(5))
        .expect("handshake timer")
        .0;
    h.fire(t);
    assert_eq!(
        status_msg(&h),
        (Status::Connecting, "Connection lost. Reconnecting…".to_string())
    );
    assert_eq!(retry_timer(&h).1, Duration::from_millis(1000));
}

#[test]
fn state_keys_stay_under_the_host_cap_for_the_whole_life() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let rename = |h: &mut Harness<Obs>, old: &str, new: &str| {
        event(
            h,
            s,
            "InputNameChanged",
            json!({"oldInputName": old, "inputName": new}),
        );
    };
    let mut old = "Mic".to_string();
    for i in 0..300 {
        let new = format!("n{i}");
        rename(&mut h, &old, &new);
        old = new;
    }
    assert!(h.states().len() <= 256, "{} keys", h.states().len());
    // Keys published earlier keep updating; ones past the cap never appear.
    assert_eq!(h.state("mute.n10"), Some(""));
    assert_eq!(h.state("mute.n299"), None);
    event(&mut h, s, "CurrentProgramSceneChanged", json!({"sceneName": "B"}));
    assert_eq!(h.state("scene"), Some("B"));
    let warns = h
        .logs()
        .iter()
        .filter(|(l, m)| *l == Level::Warn && m.contains("state keys the host allows"))
        .count();
    assert_eq!(warns, 1);
    // A disconnect and reconnect does not make room.
    h.closed(s, "connection lost");
    assert!(h.states().len() <= 256);
}

#[test]
fn oversize_keys_and_values_are_handled() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let long = "x".repeat(300);
    event(
        &mut h,
        s,
        "InputNameChanged",
        json!({"oldInputName": "Mic", "inputName": long}),
    );
    assert!(h.states().keys().all(|k| k.len() <= 256));
    assert!(
        h.logs()
            .iter()
            .any(|(l, m)| *l == Level::Warn && m.contains("longer than 256 bytes"))
    );
    // A long error is cut at a character boundary, not dropped.
    h.action("set_scene", json!({"scene": "Missing"})).expect("ok");
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    answer(&mut h, s, &id, &ty, false, json!({"comment": "é".repeat(1500)}));
    let e = h.state("last_error").expect("last_error");
    assert!(
        !e.is_empty() && e.len() <= 1024 && e.chars().all(|c| c == 'é'),
        "{}",
        e.len()
    );
}

#[test]
fn a_lost_socket_takes_the_connection_down() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    h.lose_socket(s);
    let e = h.action("toggle_record", json!({})).expect_err("send fails");
    assert!(e.starts_with("OBS isn’t connected: "), "{e}");
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(
        status_msg(&h),
        (Status::Connecting, "Connection lost. Reconnecting…".to_string())
    );
    assert_eq!(retry_timer(&h).1, Duration::from_millis(1000));

    // The tick's GetStats finds a lost socket too.
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    h.lose_socket(s);
    let t = tick(&h);
    h.fire(t);
    h.fire(t);
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(retry_timer(&h).1, Duration::from_millis(1000));

    // And the second half of a read-then-write action.
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    h.action("input_volume", json!({"input": "Mic", "op": "up", "step": 3}))
        .expect("ok");
    let (id, ty, _) = sent_requests(&h, s).pop().expect("request");
    h.lose_socket(s);
    answer(&mut h, s, &id, &ty, true, json!({"inputVolumeDb": -1.0}));
    assert_eq!(h.state("connected"), Some("no"));
}

#[test]
fn reloads_are_coalesced_per_tick() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let before = sent_requests(&h, s).len();
    for i in 0..20 {
        event(&mut h, s, "SceneCreated", json!({"sceneName": format!("S{i}")}));
        event(&mut h, s, "InputCreated", json!({"inputName": format!("I{i}")}));
    }
    assert_eq!(sent_requests(&h, s).len(), before);
    let t = tick(&h);
    h.fire(t);
    let new: Vec<String> = sent_requests(&h, s)[before..]
        .iter()
        .map(|r| r.1.clone())
        .collect();
    assert_eq!(new, ["GetSceneList", "GetInputList"]);
    // Nothing further wanted: the next tick reloads nothing.
    h.fire(t);
    let reloads = sent_requests(&h, s)[before..]
        .iter()
        .filter(|r| r.1 == "GetSceneList")
        .count();
    assert_eq!(reloads, 1);
}

#[test]
fn a_failed_essential_load_goes_down() {
    for expire in [false, true] {
        for essential in ["GetVersion", "GetSceneList"] {
            let mut h = Harness::<Obs>::new();
            h.init(settings()).expect("init");
            let s = current(&h);
            h.open(s);
            h.message(s, hello(false));
            h.message(
                s,
                Payload::Text(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string()),
            );
            let (id, ty, _) = sent_requests(&h, s)
                .into_iter()
                .find(|r| r.1 == essential)
                .expect("request");
            if expire {
                let t = tick(&h);
                for _ in 0..6 {
                    h.fire(t);
                }
            } else {
                answer(&mut h, s, &id, &ty, false, json!({"comment": "no"}));
            }
            assert_eq!(h.state("connected"), Some("no"), "{essential} {expire}");
            assert_eq!(
                status_msg(&h),
                (Status::Connecting, "Connection lost. Reconnecting…".to_string())
            );
            assert_eq!(retry_timer(&h).1, Duration::from_millis(1000));
        }
    }
}

/// Open the current socket and get through Hello and Identified (no loads answered yet).
fn identified(h: &mut Harness<Obs>) -> Socket {
    let s = current(h);
    h.open(s);
    h.message(s, hello(false));
    h.message(
        s,
        Payload::Text(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string()),
    );
    s
}

#[test]
fn a_refusing_obs_keeps_backing_off_until_the_loads_succeed() {
    // OBS signs in every time, then refuses GetVersion: the retries still double to 10 s.
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    for want in [1000, 2000, 4000, 8000, 10000, 10000] {
        let s = identified(&mut h);
        let (id, ty, _) = sent_requests(&h, s)
            .into_iter()
            .find(|r| r.1 == "GetVersion")
            .expect("GetVersion");
        answer(&mut h, s, &id, &ty, false, json!({"comment": "not ready"}));
        assert_eq!(h.state("connected"), Some("no"));
        let (t, d) = retry_timer(&h);
        assert_eq!(d, Duration::from_millis(want));
        h.fire(t);
    }
    // The same refusal is logged once, not on every try.
    let refused = |h: &Harness<Obs>| {
        h.logs()
            .iter()
            .filter(|(l, m)| *l == Level::Warn && m == "OBS refused GetVersion")
            .count()
    };
    assert_eq!(refused(&h), 1, "{:?}", h.logs());
    // Identified alone does not reset the backoff either: unanswered loads expire.
    identified(&mut h);
    let t = tick(&h);
    for _ in 0..6 {
        h.fire(t);
    }
    assert_eq!(h.state("connected"), Some("no"));
    assert_eq!(retry_timer(&h).1, Duration::from_millis(10000));
}

#[test]
fn a_good_connection_resets_the_backoff() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    for _ in 0..4 {
        let s = current(&h);
        h.closed(s, "connection refused");
        let (t, _) = retry_timer(&h);
        h.fire(t);
    }
    // The next try gets through and loads: a later drop retries after 1 s again.
    let s = current(&h);
    h.open(s);
    h.message(s, hello(false));
    h.message(
        s,
        Payload::Text(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string()),
    );
    for (id, ty, _) in sent_requests(&h, s) {
        let body = match ty.as_str() {
            "GetVersion" => json!({"obsVersion": "32.2.0", "obsWebSocketVersion": "5.7.0"}),
            "GetSceneList" => json!({"currentProgramSceneName": "A", "scenes": [{"sceneName": "A"}]}),
            _ => json!({}),
        };
        answer(&mut h, s, &id, &ty, true, body);
    }
    assert_eq!(h.state("connected"), Some("yes"));
    h.closed(s, "connection lost");
    assert_eq!(retry_timer(&h).1, Duration::from_millis(1000));
}

#[test]
fn reconnect_and_new_settings_warn_about_a_refusal_again() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    let refuse = |h: &mut Harness<Obs>| {
        let s = identified(h);
        let (id, ty, _) = sent_requests(h, s)
            .into_iter()
            .find(|r| r.1 == "GetVersion")
            .expect("GetVersion");
        answer(h, s, &id, &ty, false, json!({"comment": "not ready"}));
    };
    let refused = |h: &Harness<Obs>| {
        h.logs()
            .iter()
            .filter(|(l, m)| *l == Level::Warn && m == "OBS refused GetVersion")
            .count()
    };
    refuse(&mut h);
    assert_eq!(refused(&h), 1);
    h.action("reconnect", json!({})).expect("ok");
    refuse(&mut h);
    assert_eq!(refused(&h), 2, "{:?}", h.logs());
    h.init(json!({"host": "127.0.0.2", "port": 4455})).expect("init");
    refuse(&mut h);
    assert_eq!(refused(&h), 3, "{:?}", h.logs());
}

#[test]
fn only_action_expiries_are_warned() {
    let mut h = Harness::<Obs>::new();
    live(&mut h);
    let t = tick(&h);
    for _ in 0..12 {
        h.fire(t);
    }
    assert!(
        h.logs().iter().all(|(_, m)| !m.contains("didn’t answer")),
        "{:?}",
        h.logs()
    );
}

#[test]
fn forget_password_clears_the_cached_one() {
    let mut h = Harness::<Obs>::new();
    h.init(json!({"host": "127.0.0.1", "port": 4455, "password": PASSWORD}))
        .expect("init");
    h.action("forget_password", json!({})).expect("ok");
    assert!(!h.secrets().contains_key("password"));
    h.action("reconnect", json!({})).expect("ok");
    let s = current(&h);
    h.open(s);
    h.message(s, hello(true));
    assert!(status_msg(&h).1.starts_with("Password needed"));
}

// ---- pickers (contract 1.1) ----

fn ids(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.id.as_str()).collect()
}

fn count(h: &Harness<Obs>, s: Socket, ty: &str) -> usize {
    sent_requests(h, s).iter().filter(|r| r.1 == ty).count()
}

#[test]
fn pickers_list_what_obs_holds() {
    let mut h = Harness::<Obs>::new();
    live(&mut h);
    assert_eq!(
        h.list_items("scenes", "", ""),
        Ok(vec![Item::new("A", "A").state("Live"), Item::new("B", "B")])
    );
    assert_eq!(
        h.list_items("audio_inputs", "", ""),
        Ok(vec![Item::new("Mic", "Mic").state("Muted")])
    );
    assert_eq!(
        h.list_items("scene_sources", "", ""),
        Ok(vec![Item::new("Cam", "Cam").subtitle("A").group("A")])
    );
    // Scene sources, and global inputs like a mic (its noise filters).
    assert_eq!(
        h.list_items("sources", "", ""),
        Ok(vec![Item::new("Cam", "Cam"), Item::new("Mic", "Mic")])
    );
    assert_eq!(
        h.list_items("filters", "", ""),
        Ok(vec![
            Item::new("Color", "Color").group("Cam").state("On"),
            Item::new("Sharpen", "Sharpen").group("Cam").state("Off"),
            Item::new("Noise", "Noise").group("Mic").state("On"),
        ])
    );
    assert_eq!(
        h.list_items("collections", "", ""),
        Ok(vec![
            Item::new("Main", "Main").state("Current"),
            Item::new("Stream", "Stream")
        ])
    );
    assert_eq!(
        h.list_items("profiles", "", ""),
        Ok(vec![
            Item::new("Default", "Default").state("Current"),
            Item::new("Quiet", "Quiet")
        ])
    );
    assert!(h.list_items("nope", "", "").is_err());
}

#[test]
fn scene_picker_marks_live_and_preview() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    event(
        &mut h,
        s,
        "StudioModeStateChanged",
        json!({"studioModeEnabled": true}),
    );
    event(&mut h, s, "CurrentPreviewSceneChanged", json!({"sceneName": "B"}));
    assert_eq!(
        h.list_items("scenes", "", ""),
        Ok(vec![
            Item::new("A", "A").state("Live"),
            Item::new("B", "B").state("Preview")
        ])
    );
    event(&mut h, s, "CurrentProgramSceneChanged", json!({"sceneName": "B"}));
    let items = h.list_items("scenes", "", "").expect("a list");
    assert_eq!(items[1].state.as_deref(), Some("Live"));
}

#[test]
fn pickers_filter_by_query_and_group() {
    let mut h = Harness::<Obs>::new();
    live(&mut h);
    assert_eq!(ids(&h.list_items("scenes", "b", "").expect("a list")), ["B"]);
    assert_eq!(
        ids(&h.list_items("filters", "SHARP", "").expect("a list")),
        ["Sharpen"]
    );
    assert_eq!(
        ids(&h.list_items("collections", "TREA", "").expect("a list")),
        ["Stream"]
    );
    assert!(h.list_items("scenes", "zzz", "").expect("a list").is_empty());
    assert_eq!(
        ids(&h.list_items("scene_sources", "", "A").expect("a list")),
        ["Cam"]
    );
    assert!(h.list_items("scene_sources", "", "B").expect("a list").is_empty());
    assert_eq!(
        ids(&h.list_items("filters", "", "Cam").expect("a list")),
        ["Color", "Sharpen"]
    );
    assert_eq!(
        ids(&h.list_items("filters", "", "Mic").expect("a list")),
        ["Noise"]
    );
    assert!(h.list_items("filters", "", "Nope").expect("a list").is_empty());
    // A group never matches part of a name, and plain lists have no groups.
    assert!(h.list_items("filters", "", "Ca").expect("a list").is_empty());
    assert!(h.list_items("scenes", "", "A").expect("a list").is_empty());
}

#[test]
fn pickers_need_a_connection() {
    let mut h = Harness::<Obs>::new();
    let off = Err("Not connected to OBS".to_string());
    assert_eq!(h.list_items("scenes", "", ""), off);
    let s = live(&mut h);
    assert!(h.list_items("scenes", "", "").is_ok());
    h.lose_socket(s);
    h.closed(s, "gone");
    assert_eq!(h.list_items("scenes", "", ""), off);
    assert_eq!(h.list_items("profiles", "", ""), off);
}

#[test]
fn lists_not_loaded_yet_are_empty_not_an_error() {
    let mut h = Harness::<Obs>::new();
    h.init(settings()).expect("init");
    let s = current(&h);
    h.open(s);
    h.message(s, hello(false));
    h.message(
        s,
        Payload::Text(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string()),
    );
    for picker in [
        "scenes",
        "audio_inputs",
        "scene_sources",
        "sources",
        "filters",
        "collections",
        "profiles",
    ] {
        assert_eq!(h.list_items(picker, "", ""), Ok(vec![]), "{picker}");
    }
}

#[test]
fn listing_sends_nothing_and_sets_no_state() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let (sent, states) = (sent_requests(&h, s).len(), h.states().clone());
    for picker in [
        "scenes",
        "audio_inputs",
        "scene_sources",
        "sources",
        "filters",
        "collections",
        "profiles",
    ] {
        let _ = h.list_items(picker, "x", "y");
        let _ = h.list_items(picker, "", "");
    }
    assert_eq!(sent_requests(&h, s).len(), sent);
    assert_eq!(h.states(), &states);
    assert!(
        h.states()
            .keys()
            .all(|k| !k.contains("filter") && !k.contains("profile") && !k.contains("collection")),
        "{:?}",
        h.states().keys()
    );
}

#[test]
fn filter_events_update_the_lists_in_place() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let before = sent_requests(&h, s).len();
    event(
        &mut h,
        s,
        "SourceFilterEnableStateChanged",
        json!({"sourceName": "Cam", "filterName": "Sharpen", "filterEnabled": true}),
    );
    event(
        &mut h,
        s,
        "SourceFilterNameChanged",
        json!({"sourceName": "Cam", "oldFilterName": "Color", "filterName": "Tint"}),
    );
    let items = h.list_items("filters", "", "Cam").expect("a list");
    assert_eq!(
        items,
        vec![
            Item::new("Tint", "Tint").group("Cam").state("On"),
            Item::new("Sharpen", "Sharpen").group("Cam").state("On"),
        ]
    );
    event(
        &mut h,
        s,
        "SourceFilterRemoved",
        json!({"sourceName": "Cam", "filterName": "Tint"}),
    );
    assert_eq!(
        ids(&h.list_items("filters", "", "Cam").expect("a list")),
        ["Sharpen"]
    );
    assert_eq!(
        sent_requests(&h, s).len(),
        before,
        "no request for in-place changes"
    );
}

#[test]
fn created_filters_are_fetched_once_per_tick() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let before = sent_requests(&h, s).len();
    for i in 0..20 {
        event(
            &mut h,
            s,
            "SourceFilterCreated",
            json!({"sourceName": "Cam", "filterName": format!("F{i}")}),
        );
    }
    assert_eq!(sent_requests(&h, s).len(), before, "waits for the tick");
    let t = tick(&h);
    h.fire(t);
    let new: Vec<_> = sent_requests(&h, s)[before..]
        .iter()
        .filter(|r| r.1 == "GetSourceFilterList")
        .cloned()
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    assert_eq!(new[0].2, json!({"sourceName": "Cam"}));
    answer(
        &mut h,
        s,
        &new[0].0,
        "GetSourceFilterList",
        true,
        json!({"filters": [{"filterName": "Only", "filterEnabled": true}]}),
    );
    assert_eq!(
        ids(&h.list_items("filters", "", "Cam").expect("a list")),
        ["Only"]
    );
}

#[test]
fn collection_and_profile_events_refresh_the_lists() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let before = sent_requests(&h, s).len();
    for _ in 0..10 {
        event(&mut h, s, "ProfileListChanged", json!({"profiles": ["Default"]}));
        event(
            &mut h,
            s,
            "SceneCollectionListChanged",
            json!({"sceneCollections": []}),
        );
    }
    event(
        &mut h,
        s,
        "CurrentProfileChanged",
        json!({"profileName": "Quiet"}),
    );
    event(
        &mut h,
        s,
        "CurrentSceneCollectionChanged",
        json!({"sceneCollectionName": "Stream"}),
    );
    assert_eq!(sent_requests(&h, s).len(), before, "waits for the tick");
    let t = tick(&h);
    h.fire(t);
    assert_eq!(count(&h, s, "GetProfileList"), 2);
    assert_eq!(count(&h, s, "GetSceneCollectionList"), 2);
    let reqs = sent_requests(&h, s);
    for (id, ty, _) in &reqs[before..] {
        match ty.as_str() {
            "GetProfileList" => answer(
                &mut h,
                s,
                id,
                ty,
                true,
                json!({"currentProfileName": "Quiet", "profiles": ["Default", "Quiet", "New"]}),
            ),
            "GetSceneCollectionList" => answer(
                &mut h,
                s,
                id,
                ty,
                true,
                json!({"currentSceneCollectionName": "Stream", "sceneCollections": ["Main", "Stream"]}),
            ),
            _ => {}
        }
    }
    assert_eq!(
        h.list_items("profiles", "", ""),
        Ok(vec![
            Item::new("Default", "Default"),
            Item::new("Quiet", "Quiet").state("Current"),
            Item::new("New", "New"),
        ])
    );
    assert_eq!(
        h.list_items("collections", "", ""),
        Ok(vec![
            Item::new("Main", "Main"),
            Item::new("Stream", "Stream").state("Current")
        ])
    );
}

#[test]
fn filter_requests_are_bounded() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    event(&mut h, s, "SceneItemCreated", json!({"sceneName": "A"}));
    let t = tick(&h);
    h.fire(t);
    let before = sent_requests(&h, s).len();
    let reqs = sent_requests(&h, s);
    let (id, ty, _) = reqs[before - 2].clone();
    assert_eq!(ty, "GetSceneList");
    answer(
        &mut h,
        s,
        &id,
        &ty,
        true,
        json!({"currentProgramSceneName": "A", "currentPreviewSceneName": null, "scenes": [{"sceneName": "A"}]}),
    );
    let (id, ty, _) = sent_requests(&h, s).pop().expect("scene items request");
    assert_eq!(ty, "GetSceneItemList");
    let items: Vec<Value> = (0..400)
        .map(|i| json!({"sceneItemId": i, "sourceName": format!("S{i}"), "sceneItemEnabled": true}))
        .collect();
    answer(&mut h, s, &id, &ty, true, json!({ "sceneItems": items }));
    let (mut total, mut max_open, mut answered) = (0, 0, before + 1);
    loop {
        let reqs = sent_requests(&h, s);
        let fresh: Vec<_> = reqs[answered..]
            .iter()
            .filter(|r| r.1 == "GetSourceFilterList")
            .cloned()
            .collect();
        answered = reqs.len();
        if fresh.is_empty() {
            break;
        }
        max_open = max_open.max(fresh.len());
        total += fresh.len();
        for (id, ty, _) in fresh {
            answer(&mut h, s, &id, &ty, true, json!({"filters": []}));
        }
    }
    assert_eq!(max_open, 25, "requests in flight at once");
    assert_eq!(total, 200, "sources asked");
}

#[test]
fn global_inputs_get_their_filters_too() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let asked: Vec<_> = sent_requests(&h, s)
        .into_iter()
        .filter(|r| r.1 == "GetSourceFilterList")
        .map(|r| r.2["sourceName"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(asked, ["Cam", "Mic"]);
    assert_eq!(ids(&h.list_items("sources", "mic", "").expect("a list")), ["Mic"]);
    assert_eq!(
        h.list_items("filters", "", "Mic"),
        Ok(vec![Item::new("Noise", "Noise").group("Mic").state("On")])
    );
}

#[test]
fn created_filter_during_a_request_is_asked_again_after_it() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let t = tick(&h);
    let asks = |h: &Harness<Obs>| {
        sent_requests(h, s)
            .into_iter()
            .filter(|r| r.1 == "GetSourceFilterList")
            .collect::<Vec<_>>()
    };
    let before = asks(&h).len();
    event(
        &mut h,
        s,
        "SourceFilterCreated",
        json!({"sourceName": "Cam", "filterName": "F1"}),
    );
    h.fire(t);
    assert_eq!(asks(&h).len(), before + 1);
    // A second creation while the first request is unanswered: nothing new goes out yet.
    event(
        &mut h,
        s,
        "SourceFilterCreated",
        json!({"sourceName": "Cam", "filterName": "F2"}),
    );
    h.fire(t);
    assert_eq!(asks(&h).len(), before + 1);
    let (id, ty, _) = asks(&h).pop().expect("request");
    answer(
        &mut h,
        s,
        &id,
        &ty,
        true,
        json!({"filters": [{"filterName": "F1", "filterEnabled": true}]}),
    );
    // The answer frees the source: it is asked about again, so F2 is not lost.
    assert_eq!(asks(&h).len(), before + 2);
    let (id, ty, _) = asks(&h).pop().expect("request");
    answer(
        &mut h,
        s,
        &id,
        &ty,
        true,
        json!({"filters": [{"filterName": "F1", "filterEnabled": true}, {"filterName": "F2", "filterEnabled": true}]}),
    );
    assert_eq!(
        ids(&h.list_items("filters", "", "Cam").expect("a list")),
        ["F1", "F2"]
    );
}

#[test]
fn a_dropped_filter_request_is_asked_again() {
    let mut h = Harness::<Obs>::new();
    let s = live(&mut h);
    let t = tick(&h);
    let before = count(&h, s, "GetSourceFilterList");
    event(
        &mut h,
        s,
        "SourceFilterCreated",
        json!({"sourceName": "Cam", "filterName": "F1"}),
    );
    h.fire(t);
    assert_eq!(count(&h, s, "GetSourceFilterList"), before + 1);
    for _ in 0..6 {
        h.fire(t);
    }
    assert!(
        count(&h, s, "GetSourceFilterList") >= before + 2,
        "asked again after the first went unanswered"
    );
}
