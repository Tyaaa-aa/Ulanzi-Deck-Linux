//! A mock obs-websocket 5 server for the OBS plugin tests (ported from `tests/obs.rs`).

use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

/// The docs' example: password `supersecretpassword` with `SALT` and `CHALLENGE`.
const SALT: &str = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
const CHALLENGE: &str = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";
const EXPECTED: &str = "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=";

#[derive(Clone, Copy)]
pub enum Mode {
    Open,
    /// Only `supersecretpassword` is accepted (checked against the docs' fixed answer).
    Password(&'static str),
}

pub struct Mock {
    pub port: u16,
    events: mpsc::UnboundedSender<Value>,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    connections: Arc<Mutex<u32>>,
    generation: watch::Sender<u64>,
}

impl Mock {
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .map(|r| r.iter().map(|(t, _)| t.clone()).collect())
            .unwrap_or_default()
    }

    pub fn last(&self, ty: &str) -> Option<Value> {
        self.requests
            .lock()
            .ok()?
            .iter()
            .rev()
            .find(|(t, _)| t == ty)
            .map(|(_, d)| d.clone())
    }

    pub fn event(&self, ty: &str, data: Value) {
        let _ = self
            .events
            .send(json!({"op": 5, "d": {"eventType": ty, "eventIntent": 1, "eventData": data}}));
    }

    pub fn connections(&self) -> u32 {
        self.connections.lock().map(|n| *n).unwrap_or(0)
    }

    /// Close every open connection now (a server-side drop); later connections are served.
    pub fn drop_all(&self) {
        self.generation.send_modify(|g| *g += 1);
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
        "GetSceneCollectionList" => (
            true,
            json!({"currentSceneCollectionName": "Main", "sceneCollections": ["Main", "Stream"]}),
            None,
        ),
        "GetProfileList" => (
            true,
            json!({"currentProfileName": "Default", "profiles": ["Default", "Quiet"]}),
            None,
        ),
        "GetSourceFilterList" if data["sourceName"] == "Cam" => (
            true,
            json!({"filters": [
                {"filterName": "Color", "filterEnabled": true, "filterIndex": 0},
                {"filterName": "Sharpen", "filterEnabled": false, "filterIndex": 1}]}),
            None,
        ),
        "GetSourceFilterList" if data["sourceName"] == "Mic" => (
            true,
            json!({"filters": [{"filterName": "Noise", "filterEnabled": true, "filterIndex": 0}]}),
            None,
        ),
        "GetSourceFilterList" => (true, json!({"filters": []}), None),
        "GetStudioModeEnabled" => (true, json!({"studioModeEnabled": false}), None),
        "GetInputVolume" => (true, json!({"inputVolumeDb": -1.0, "inputVolumeMul": 0.89}), None),
        "GetStats" => (true, json!({"cpuUsage": 1.0, "activeFps": 30.0}), None),
        _ => (true, json!({}), None),
    }
}

pub async fn mock(mode: Mode) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (events, events_rx) = mpsc::unbounded_channel::<Value>();
    let events_rx = Arc::new(tokio::sync::Mutex::new(events_rx));
    let requests: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let connections: Arc<Mutex<u32>> = Arc::default();
    let (generation, _) = watch::channel(0u64);
    {
        let (requests, connections, generation) = (requests.clone(), connections.clone(), generation.clone());
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                if let Ok(mut n) = connections.lock() {
                    *n += 1;
                }
                let (requests, events_rx) = (requests.clone(), events_rx.clone());
                let mut gen_rx = generation.subscribe();
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(tcp).await else {
                        return;
                    };
                    let hello = match mode {
                        Mode::Password(_) => {
                            json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.4", "rpcVersion": 1,
                            "authentication": {"challenge": CHALLENGE, "salt": SALT}}})
                        }
                        Mode::Open => {
                            json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.4", "rpcVersion": 1}})
                        }
                    };
                    if ws.send(Message::text(hello.to_string())).await.is_err() {
                        return;
                    }
                    let Some(Ok(Message::Text(t))) = ws.next().await else {
                        return;
                    };
                    let id: Value = serde_json::from_str(&t).unwrap_or_default();
                    // The fixed answer is for `supersecretpassword` only.
                    if let Mode::Password(pw) = mode
                        && (pw != "supersecretpassword" || id["d"]["authentication"] != EXPECTED)
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
                    // Waiting for the guard means the previous connection is gone. Events sent
                    // while no client was connected must not reach this one.
                    while events_rx.try_recv().is_ok() {}
                    loop {
                        tokio::select! {
                            _ = gen_rx.changed() => return,
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
        generation,
    }
}
