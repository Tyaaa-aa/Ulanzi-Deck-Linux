//! The Home Assistant plugin (`plugins/ha`) running in the real host against a mock Home Assistant.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::ha_mock::{Mode, mock};
use common::plugins::{Running, start};
use serde_json::json;
use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::plugins::ulanzi::plugin::types::Status;
use ulanzi_deckd::plugins::{Event, Output};

const PACKAGE: &str = "ulanzi-plugin-ha";
const MANIFEST: &str = include_str!("../../../plugins/ha/plugin.toml");
const SOON: Duration = Duration::from_secs(60);
const TOKEN: &str = "mock-access-token-123";

/// The latest value of every state key, plus every other output, seen so far.
#[derive(Default)]
struct Seen {
    states: BTreeMap<String, String>,
    others: Vec<Output>,
}

impl Seen {
    fn state(&self, key: &str) -> &str {
        self.states.get(key).map_or("", String::as_str)
    }

    /// Read outputs until `done` holds for what has been seen; false when `SOON` runs out.
    async fn wait(&mut self, p: &mut Running, done: impl Fn(&Seen) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + SOON;
        while !done(self) {
            match tokio::time::timeout_at(deadline, p.out.recv()).await {
                Ok(Some(Output::State { key, value })) => {
                    self.states.insert(key, value);
                }
                Ok(Some(other)) => self.others.push(other),
                Ok(None) | Err(_) => return false,
            }
        }
        true
    }

    fn has_status(&self, want: Status, starts: &str) -> bool {
        self.others.iter().any(
            |o| matches!(o, Output::Status { status, message } if *status == want && message.starts_with(starts)),
        )
    }
}

/// Wait for a status with `status` whose message passes `ok`.
async fn status_where(p: &mut Running, want: Status, ok: impl Fn(&str) -> bool, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, p.out.recv()).await {
            Ok(Some(Output::Status { status, message })) if status == want && ok(&message) => return true,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return false,
        }
    }
}

fn settings(port: u16) -> serde_json::Value {
    json!({"url": format!("127.0.0.1:{port}"), "token": TOKEN,
        "entities": ["light.desk", "sensor.temp"]})
}

#[test]
fn ha_manifest_is_valid() {
    let m = Manifest::parse(MANIFEST).expect("manifest");
    assert_eq!(m.id, "ha");
    assert_eq!(m.name, "Home Assistant");
    assert!(m.background);
    assert_eq!(m.actions.len(), 8);
    assert!(m.permissions.lan);
    assert!(m.permissions.network.is_empty());
}

#[tokio::test]
async fn ha_plugin_end_to_end() {
    let mock = mock(Mode::Token(TOKEN)).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: settings(mock.port),
    });
    let mut seen = Seen::default();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes"
            && s.state("light.desk") == "on"
            && s.state("sensor.temp") == "21.5"
            && s.state("light.desk.name") == "Desk lamp"
            && s.state("sensor.temp.unit") == "°C")
            .await,
        "connected with the listed entities"
    );
    assert!(
        seen.has_status(Status::Ok, "Home Assistant 2025.10.1"),
        "ok status: {:?}",
        seen.others
    );
    let saved = Output::SecretsChanged(BTreeMap::from([("token".to_string(), TOKEN.to_string())]));
    assert!(seen.others.contains(&saved), "token saved");

    assert_eq!(p.act("toggle", json!({"entity_id": "light.desk"})).await, Ok(()));
    let mut sent = None;
    for _ in 0..600 {
        sent = mock.last_service();
        if sent.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let sent = sent.expect("call_service reached the mock");
    assert_eq!(sent["domain"], "homeassistant");
    assert_eq!(sent["service"], "toggle");
    assert_eq!(sent["target"]["entity_id"], "light.desk");

    mock.push_change("light.desk", "off", json!({"brightness": 0}));
    assert!(
        seen.wait(&mut p, |s| s.state("light.desk") == "off").await,
        "label follows the change"
    );

    assert_eq!(
        p.act(
            "call_service",
            json!({"domain": "light", "service": "boom", "entity_id": "light.desk"})
        )
        .await,
        Ok(())
    );
    assert!(
        seen.wait(&mut p, |s| s.state("last_error")
            == "Service boom failed in the mock")
            .await,
        "last_error shows the refusal"
    );

    mock.drop_all();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "no"
            && s.state("light.desk") == "offline")
            .await,
        "offline after drop"
    );
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes"
            && s.state("light.desk") == "on")
            .await,
        "back online with fresh values"
    );
    assert_eq!(mock.connections(), 2);
    assert!(
        !mock.requests().iter().any(|t| t == "get_states"),
        "the whole install is never requested: {:?}",
        mock.requests()
    );
}

#[tokio::test]
async fn ha_plugin_nothing_listed_exists() {
    // Home Assistant may send no snapshot at all when none of the entities exist.
    let mock = mock(Mode::NoEntities(TOKEN)).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: settings(mock.port),
    });
    let mut seen = Seen::default();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes"
            && s.state("light.desk") == "missing"
            && s.state("sensor.temp") == "missing")
            .await,
        "connected, entities missing"
    );
    // Past the 5 s handshake timer the connection is still the first one.
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(mock.connections(), 1, "no reconnect loop");
    assert_eq!(seen.state("connected"), "yes");
}

#[tokio::test]
async fn ha_plugin_bad_token_and_foreign_host() {
    let mock = mock(Mode::Token(TOKEN)).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: json!({"url": format!("127.0.0.1:{}", mock.port), "token": "wrong-token",
            "entities": ["light.desk"]}),
    });
    assert!(
        status_where(
            &mut p,
            Status::Problem,
            |m| m.starts_with("Home Assistant rejected the access token"),
            SOON
        )
        .await,
        "token rejected"
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(mock.connections(), 1, "a rejected token is not retried");

    // A public IP literal is refused by the host's lan check before anything is dialled.
    p.send(Event::Init {
        settings: json!({"url": "8.8.8.8", "token": TOKEN}),
    });
    assert!(
        status_where(
            &mut p,
            Status::Problem,
            |m| m.starts_with("Home Assistant must be on your local network"),
            SOON
        )
        .await,
        "public address refused"
    );
    // A public name passes the literal check and is refused after DNS: the socket closes while
    // opening, which the plugin reports as unreachable (and retries). Either way it is a Problem
    // and the mock sees nothing.
    p.send(Event::Init {
        settings: json!({"url": "example.com", "token": TOKEN}),
    });
    assert!(
        status_where(
            &mut p,
            Status::Problem,
            |m| m.contains("example.com") || m.contains("local network"),
            SOON
        )
        .await,
        "foreign name is a problem"
    );
    assert_eq!(mock.connections(), 1, "no connection to a foreign host");
}
