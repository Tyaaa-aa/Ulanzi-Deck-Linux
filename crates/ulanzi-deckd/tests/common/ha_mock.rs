//! A mock Home Assistant WebSocket API for the Home Assistant plugin tests.
//!
//! It also holds a large fake registry (3000 entities) that it only returns to `get_states`:
//! the answer is about 3 MB, so a plugin that ever asked for it would see the host close the socket
//! (1 MiB per message) and its test would fail.
//!
//! `render_template` is answered like Home Assistant does (a success `result`, then an `event` with
//! `{"result": "<text>", "listeners": {...}}`), with a fixed text in the format of the plugin's entity
//! list template: the mock does not run Jinja.

use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy)]
pub enum Mode {
    /// Only this access token is accepted.
    Token(&'static str),
    /// Like `Token`, but none of the entities asked for exist: the subscription succeeds and no
    /// snapshot follows.
    NoEntities(&'static str),
}

impl Mode {
    fn token(self) -> &'static str {
        match self {
            Mode::Token(t) | Mode::NoEntities(t) => t,
        }
    }
}

pub struct Mock {
    pub port: u16,
    events: mpsc::UnboundedSender<Value>,
    requests: Arc<Mutex<Vec<Value>>>,
    connections: Arc<Mutex<u32>>,
    generation: watch::Sender<u64>,
}

impl Mock {
    /// The message types received after sign-in, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .map(|r| {
                r.iter()
                    .map(|m| m["type"].as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every message received after sign-in, in order.
    pub fn sent(&self) -> Vec<Value> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// The last `call_service` message received.
    pub fn last_service(&self) -> Option<Value> {
        self.requests
            .lock()
            .ok()?
            .iter()
            .rev()
            .find(|m| m["type"] == "call_service")
            .cloned()
    }

    pub fn connections(&self) -> u32 {
        self.connections.lock().map(|n| *n).unwrap_or(0)
    }

    /// An entity of the subscription changed to `state` with `attrs` merged in.
    pub fn push_change(&self, entity_id: &str, state: &str, attrs: Value) {
        let _ = self
            .events
            .send(json!({"c": {entity_id: {"+": {"s": state, "a": attrs}}}}));
    }

    /// Close every open connection now (a server-side drop); later connections are served.
    pub fn drop_all(&self) {
        self.generation.send_modify(|g| *g += 1);
    }
}

/// What the mock knows about an entity: (state, attributes).
fn known(entity_id: &str) -> Option<(&'static str, Value)> {
    match entity_id {
        "light.desk" => Some(("on", json!({"brightness": 128, "friendly_name": "Desk lamp"}))),
        "sensor.temp" => Some((
            "21.5",
            json!({"unit_of_measurement": "°C", "friendly_name": "Temperature"}),
        )),
        _ => None,
    }
}

/// About 3 MB of states, as `get_states` would answer on a big install.
fn registry() -> Value {
    let pad = "x".repeat(900);
    Value::Array(
        (0..3000)
            .map(|i| {
                json!({"entity_id": format!("sensor.fake_{i}"), "state": "1",
                    "attributes": {"friendly_name": format!("Fake {i}"), "pad": pad}})
            })
            .collect(),
    )
}

/// What the plugin's entity list template would render for the mock's install.
const ENTITY_LINES: &str = "light.desk\tDesk lamp\ton\nswitch.kettle\tKettle\toff\nscene.movie\tMovie time\tunknown\n\
    lock.door\tFront door\tlocked\nsensor.temp\tTemperature\t21.5\n";

fn result(id: &Value, ok: bool, body: Value, error: Option<&str>) -> Value {
    let mut m = json!({"id": id, "type": "result", "success": ok, "result": body});
    if let Some(e) = error {
        m["error"] = json!({"code": "home_assistant_error", "message": e});
    }
    m
}

pub async fn mock(mode: Mode) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (events, events_rx) = mpsc::unbounded_channel::<Value>();
    let events_rx = Arc::new(tokio::sync::Mutex::new(events_rx));
    let requests: Arc<Mutex<Vec<Value>>> = Arc::default();
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
                    let hello = json!({"type": "auth_required", "ha_version": "2025.10.1"});
                    if ws.send(Message::text(hello.to_string())).await.is_err() {
                        return;
                    }
                    let Some(Ok(Message::Text(t))) = ws.next().await else {
                        return;
                    };
                    let auth: Value = serde_json::from_str(&t).unwrap_or_default();
                    if auth["type"] != "auth" || auth["access_token"] != mode.token() {
                        let bad =
                            json!({"type": "auth_invalid", "message": "Invalid access token or password"});
                        let _ = ws.send(Message::text(bad.to_string())).await;
                        let _ = ws.close(None).await;
                        return;
                    }
                    let ok = json!({"type": "auth_ok", "ha_version": "2025.10.1"});
                    if ws.send(Message::text(ok.to_string())).await.is_err() {
                        return;
                    }
                    let mut events_rx = events_rx.lock().await;
                    // Waiting for the guard means the previous connection is gone. Changes sent
                    // while no client was connected must not reach this one.
                    while events_rx.try_recv().is_ok() {}
                    let mut sub_id: Option<Value> = None;
                    loop {
                        tokio::select! {
                            _ = gen_rx.changed() => return,
                            m = ws.next() => {
                                let Some(Ok(Message::Text(t))) = m else { return };
                                let v: Value = serde_json::from_str(&t).unwrap_or_default();
                                if let Ok(mut r) = requests.lock() {
                                    r.push(v.clone());
                                }
                                let id = &v["id"];
                                let mut out = Vec::new();
                                match v["type"].as_str().unwrap_or_default() {
                                    "subscribe_entities" => {
                                        sub_id = Some(id.clone());
                                        out.push(result(id, true, Value::Null, None));
                                        let mut snap = serde_json::Map::new();
                                        if let (Mode::Token(_), Some(ids)) = (mode, v["entity_ids"].as_array()) {
                                            for eid in ids.iter().filter_map(Value::as_str) {
                                                if let Some((s, a)) = known(eid) {
                                                    snap.insert(eid.to_string(), json!({"s": s, "a": a}));
                                                }
                                            }
                                            out.push(json!({"id": id, "type": "event", "event": {"a": snap}}));
                                        }
                                    }
                                    "call_service" if v["service"] == "boom" => {
                                        out.push(result(id, false, Value::Null, Some("Service boom failed in the mock")));
                                    }
                                    "call_service" => out.push(result(id, true, json!({"context": {}}), None)),
                                    "ping" => out.push(json!({"id": id, "type": "pong"})),
                                    "get_states" => out.push(result(id, true, registry(), None)),
                                    // The mock does not run Jinja: a fixed list in the template's format.
                                    "render_template" => {
                                        out.push(result(id, true, Value::Null, None));
                                        out.push(json!({"id": id, "type": "event", "event": {
                                            "result": ENTITY_LINES,
                                            "listeners": {"all": true, "entities": [], "domains": [], "time": false},
                                        }}));
                                    }
                                    _ => out.push(result(id, true, Value::Null, None)),
                                }
                                for m in out {
                                    if ws.send(Message::text(m.to_string())).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            e = events_rx.recv() => {
                                let Some(e) = e else { return };
                                let Some(id) = &sub_id else { continue };
                                let m = json!({"id": id, "type": "event", "event": e});
                                if ws.send(Message::text(m.to_string())).await.is_err() {
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
