//! The OBS plugin against the SDK's fake host: connection lifecycle, requests and live state.
use std::time::Duration;

use serde_json::Value;
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
    h.init(settings()).expect("init");
    let s = current(h);
    h.open(s);
    h.message(s, hello(false));
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
    let s = live(&mut h);
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
    let _ = s;
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
    let s = h.opened()[0].0;
    h.open(s);
    h.message(s, hello(true));
    for key in [
        "connected",
        "scene",
        "preview",
        "studio_mode",
        "scene_count",
        "recording",
        "rec_paused",
        "streaming",
        "replay",
        "vcam",
        "rec_time",
        "stream_time",
        "fps",
        "cpu",
        "dropped",
        "last_error",
    ] {
        assert!(!h.state(key).unwrap_or_default().contains(PASSWORD), "{key}");
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
