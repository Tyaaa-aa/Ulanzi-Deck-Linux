//! The OBS connection against a mock obs-websocket 5 server, for every state in spec §6.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use ulanzi_core::model::{ObsAction, ObsSettings};
use ulanzi_deckd::obs::detect::ObsConfig;
use ulanzi_deckd::obs::protocol::{Rename, auth_string};
use ulanzi_deckd::obs::{ObsHandle, ObsState, Probe, Timing};

#[derive(Clone)]
enum Mode {
    Open,
    Password(&'static str),
    Old,
}

struct Mock {
    port: u16,
    events: mpsc::UnboundedSender<Value>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    connections: Arc<Mutex<u32>>,
}

impl Mock {
    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .map(|r| r.iter().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default()
    }

    fn last(&self, ty: &str) -> Option<Value> {
        self.requests
            .lock()
            .ok()?
            .iter()
            .rev()
            .find(|(t, _)| t == ty)
            .map(|(_, d)| d.clone())
    }

    fn event(&self, ty: &str, data: Value) {
        let _ = self
            .events
            .send(json!({"op": 5, "d": {"eventType": ty, "eventIntent": 1, "eventData": data}}));
    }
}

fn answer(ty: &str, data: &Value) -> (bool, Value, Option<&'static str>) {
    match ty {
        "GetVersion" => (
            true,
            json!({"obsVersion": "31.0.2", "obsWebSocketVersion": "5.5.4"}),
            None,
        ),
        "GetSceneList" => (
            true,
            json!({"currentProgramSceneName": "A", "currentPreviewSceneName": null,
                   "scenes": [{"sceneName": "B", "sceneIndex": 1}, {"sceneName": "A", "sceneIndex": 0}]}),
            None,
        ),
        "GetSceneItemList" if data["sceneName"] == "A" => (
            true,
            json!({"sceneItems": [{"sceneItemId": 1, "sourceName": "Cam", "sceneItemEnabled": true}]}),
            None,
        ),
        "GetSceneItemList" => (true, json!({"sceneItems": []}), None),
        "GetInputList" => (
            true,
            json!({"inputs": [{"inputName": "Mic"}, {"inputName": "Cam"}]}),
            None,
        ),
        "GetInputMute" if data["inputName"] == "Mic" => (true, json!({"inputMuted": true}), None),
        "GetInputMute" => (false, json!({}), Some("not an audio input")),
        "GetRecordStatus" | "GetStreamStatus" | "GetReplayBufferStatus" | "GetVirtualCamStatus" => (
            true,
            json!({"outputActive": false, "outputPaused": false, "outputDuration": 0}),
            None,
        ),
        "GetStudioModeEnabled" => (true, json!({"studioModeEnabled": false}), None),
        "GetInputVolume" => (true, json!({"inputVolumeDb": -1.0, "inputVolumeMul": 0.89}), None),
        "SetCurrentProgramScene" if data["sceneName"] == "Missing" => (
            false,
            json!({}),
            Some("No source was found by the name of `Missing`."),
        ),
        _ => (true, json!({}), None),
    }
}

async fn mock_on(listener: TcpListener, mode: Mode) -> Mock {
    let port = listener.local_addr().expect("addr").port();
    let (events, events_rx) = mpsc::unbounded_channel::<Value>();
    let events_rx = Arc::new(tokio::sync::Mutex::new(events_rx));
    let requests: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let connections: Arc<Mutex<u32>> = Arc::default();
    {
        let (requests, connections) = (requests.clone(), connections.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                if let Ok(mut n) = connections.lock() {
                    *n += 1;
                }
                let (mode, requests, events_rx) = (mode.clone(), requests.clone(), events_rx.clone());
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(tcp).await else {
                        return;
                    };
                    let hello = match &mode {
                        Mode::Old => json!({"op": 0, "d": {"rpcVersion": 0}}),
                        Mode::Password(_) => {
                            json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.4", "rpcVersion": 1,
                            "authentication": {"challenge": "ch", "salt": "sa"}}})
                        }
                        Mode::Open => {
                            json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.4", "rpcVersion": 1}})
                        }
                    };
                    if ws.send(Message::text(hello.to_string())).await.is_err() {
                        return;
                    }
                    // Identify.
                    let Some(Ok(Message::Text(t))) = ws.next().await else {
                        return;
                    };
                    let id: Value = serde_json::from_str(&t).unwrap_or_default();
                    if let Mode::Password(pw) = mode
                        && id["d"]["authentication"] != auth_string(pw, "sa", "ch")
                    {
                        let _ = ws
                            .send(Message::Close(Some(CloseFrame {
                                code: 4009.into(),
                                reason: "Authentication failed.".into(),
                            })))
                            .await;
                        return;
                    }
                    if ws
                        .send(Message::text(
                            json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let mut events_rx = events_rx.lock().await;
                    loop {
                        tokio::select! {
                            m = ws.next() => {
                                let Some(Ok(Message::Text(t))) = m else { return };
                                let v: Value = serde_json::from_str(&t).unwrap_or_default();
                                let d = &v["d"];
                                let ty = d["requestType"].as_str().unwrap_or_default().to_string();
                                let data = d["requestData"].clone();
                                if let Ok(mut r) = requests.lock() {
                                    r.push((ty.clone(), data.clone()));
                                }
                                let (ok, resp, comment) = answer(&ty, &data);
                                let mut status = json!({"result": ok, "code": if ok { 100 } else { 600 }});
                                if let Some(c) = comment {
                                    status["comment"] = c.into();
                                }
                                let out = json!({"op": 7, "d": {"requestType": ty, "requestId": d["requestId"],
                                    "requestStatus": status, "responseData": resp}});
                                if ws.send(Message::text(out.to_string())).await.is_err() {
                                    return;
                                }
                            }
                            e = events_rx.recv() => {
                                let Some(e) = e else { return };
                                if ws.send(Message::text(e.to_string())).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
    }
    Mock {
        port,
        events,
        requests,
        connections,
    }
}

async fn mock(mode: Mode) -> Mock {
    mock_on(TcpListener::bind("127.0.0.1:0").await.expect("bind"), mode).await
}

fn fast() -> Timing {
    Timing {
        backoff_min: Duration::from_millis(100),
        backoff_max: Duration::from_millis(400),
        poll: Duration::from_millis(50),
        timeout: Duration::from_secs(2),
    }
}

fn settings(port: u16, password: Option<&str>) -> ObsSettings {
    ObsSettings {
        host: "127.0.0.1".into(),
        port,
        password: password.map(str::to_string),
        auto_detect: false,
    }
}

fn probe(p: Probe) -> ulanzi_deckd::obs::client::ProbeFn {
    Arc::new(move || p.clone())
}

fn running() -> Probe {
    Probe {
        installed: true,
        running: true,
        config: None,
    }
}

async fn wait_for(rx: &mut watch::Receiver<ObsState>, want: impl Fn(&ObsState) -> bool) -> ObsState {
    let r = tokio::time::timeout(Duration::from_secs(5), rx.wait_for(|s| want(s)))
        .await
        .map(|r| r.map(|s| s.clone()));
    match r {
        Ok(Ok(s)) => s,
        _ => panic!("state never arrived; last: {:?}", *rx.borrow()),
    }
}

fn start(s: ObsSettings, p: Probe, timing: Timing) -> (ObsHandle, mpsc::UnboundedReceiver<Rename>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (ObsHandle::spawn(s, probe(p), timing, tx), rx)
}

#[tokio::test]
async fn connects_loads_state_and_runs_actions() {
    let m = mock(Mode::Open).await;
    let (h, mut renames) = start(settings(m.port, None), running(), fast());
    let s = wait_for(&mut h.watch_status(), |s| matches!(s, ObsState::Connected { .. })).await;
    assert_eq!(s.detail(), "OBS 31.0.2 · WebSocket 5.5.4 · 2 scenes");
    let mut live = h.watch_live();
    let l = live.borrow().clone();
    assert!(l.connected);
    assert_eq!(l.program, "A");
    assert_eq!(l.scenes, ["A", "B"]);
    assert!(l.muted.contains("Mic"));

    h.action(&ObsAction::SetScene {
        scene: "B".into(),
        preview: false,
    })
    .await
    .expect("set scene");
    assert_eq!(m.last("SetCurrentProgramScene").expect("sent")["sceneName"], "B");
    m.event("CurrentProgramSceneChanged", json!({"sceneName": "B"}));
    tokio::time::timeout(Duration::from_secs(2), live.wait_for(|l| l.program == "B"))
        .await
        .expect("program follows events")
        .expect("watch");

    h.action(&ObsAction::ToggleSource {
        scene: "A".into(),
        source: "Cam".into(),
    })
    .await
    .expect("toggle source");
    let d = m.last("SetSceneItemEnabled").expect("sent");
    assert_eq!(
        (d["sceneItemId"].as_i64(), d["sceneItemEnabled"].as_bool()),
        (Some(1), Some(false))
    );

    // A dial on input volume: read, then set, never above 0 dB.
    h.action(&ObsAction::InputVolume {
        input: "Mic".into(),
        op: ulanzi_core::model::VolumeOp::Up,
        step: 2,
    })
    .await
    .expect("volume");
    assert_eq!(m.last("SetInputVolume").expect("sent")["inputVolumeDb"], 0.0);

    // OBS's own words when it refuses.
    let err = h
        .action(&ObsAction::SetScene {
            scene: "Missing".into(),
            preview: false,
        })
        .await
        .expect_err("refused");
    assert!(err.contains("No source was found"), "{err}");

    // Renames go to the deck.
    m.event(
        "SceneNameChanged",
        json!({"oldSceneName": "A", "sceneName": "Main"}),
    );
    let r = tokio::time::timeout(Duration::from_secs(2), renames.recv())
        .await
        .expect("rename in time");
    assert_eq!(r, Some(Rename::Scene("A".into(), "Main".into())));
}

#[tokio::test]
async fn timers_count_locally() {
    let m = mock(Mode::Open).await;
    let (h, _r) = start(settings(m.port, None), running(), fast());
    wait_for(&mut h.watch_status(), |s| matches!(s, ObsState::Connected { .. })).await;
    let before = m.requests().len();
    m.event(
        "RecordStateChanged",
        json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
    let mut live = h.watch_live();
    tokio::time::timeout(
        Duration::from_secs(4),
        live.wait_for(|l| l.recording && l.rec_secs >= 2),
    )
    .await
    .expect("timer counts")
    .expect("watch");
    let extra: Vec<String> = m.requests()[before..]
        .iter()
        .filter(|t| *t != "GetStats")
        .cloned()
        .collect();
    assert!(extra.is_empty(), "timers asked OBS: {extra:?}");
}

#[tokio::test]
async fn auth_missing_and_failed_are_reported_and_back_off() {
    let m = mock(Mode::Password("right")).await;
    let (h, _r) = start(settings(m.port, None), running(), fast());
    wait_for(&mut h.watch_status(), |s| *s == ObsState::AuthMissing).await;

    let m = mock(Mode::Password("right")).await;
    let (h, _r) = start(settings(m.port, Some("wrong")), running(), fast());
    wait_for(&mut h.watch_status(), |s| *s == ObsState::AuthFailed).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let tries = *m.connections.lock().expect("count");
    // 100, 200, 400, 400 ms…: a handful, not a storm.
    assert!(tries <= 6, "{tries} connection attempts in 1.5 s");
    // The right password fixes it without a restart.
    h.set_settings(settings(m.port, Some("right"))).await;
    wait_for(&mut h.watch_status(), |s| matches!(s, ObsState::Connected { .. })).await;
}

#[tokio::test]
async fn old_obs_is_too_old() {
    let m = mock(Mode::Old).await;
    let (h, _r) = start(settings(m.port, None), running(), fast());
    wait_for(&mut h.watch_status(), |s| *s == ObsState::TooOld).await;
}

async fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    l.local_addr().expect("addr").port()
}

#[tokio::test]
async fn refused_connections_explain_why() {
    let port = free_port().await;
    let cfg = |enabled: bool, p: u16| ObsConfig {
        enabled,
        port: p,
        auth: false,
        password: String::new(),
        path: "/x".into(),
    };
    let cases = [
        (
            Probe {
                installed: false,
                running: false,
                config: None,
            },
            ObsState::NotInstalled,
        ),
        (
            Probe {
                installed: true,
                running: false,
                config: None,
            },
            ObsState::NotRunning,
        ),
        (
            Probe {
                installed: true,
                running: true,
                config: Some(cfg(false, port)),
            },
            ObsState::ServerOff,
        ),
        (
            Probe {
                installed: true,
                running: true,
                config: Some(cfg(true, 4460)),
            },
            ObsState::WrongPort {
                ours: port,
                obs: 4460,
            },
        ),
    ];
    for (p, want) in cases {
        let (h, _r) = start(settings(port, None), p, fast());
        wait_for(&mut h.watch_status(), |s| *s == want).await;
        let err = h.action(&ObsAction::ToggleRecord).await.expect_err("offline");
        assert!(err.contains("isn’t connected"), "{err}");
    }
}

#[tokio::test]
async fn process_appearance_reconnects() {
    let port = free_port().await;
    let is_running = Arc::new(Mutex::new(false));
    let p = {
        let is_running = is_running.clone();
        Arc::new(move || Probe {
            installed: true,
            running: *is_running.lock().expect("lock"),
            config: None,
        })
    };
    let slow = Timing {
        backoff_min: Duration::from_secs(60),
        backoff_max: Duration::from_secs(60),
        ..fast()
    };
    let (tx, _rx) = mpsc::unbounded_channel();
    let h = ObsHandle::spawn(settings(port, None), p, slow, tx);
    wait_for(&mut h.watch_status(), |s| *s == ObsState::NotRunning).await;
    // OBS starts: its server comes up and the process appears.
    let _m = mock_on(
        TcpListener::bind(("127.0.0.1", port)).await.expect("rebind"),
        Mode::Open,
    )
    .await;
    *is_running.lock().expect("lock") = true;
    wait_for(&mut h.watch_status(), |s| matches!(s, ObsState::Connected { .. })).await;
}
