//! The OBS plugin (`plugins/obs`) running in the real host against a mock obs-websocket.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::obs_mock::{Mode, mock};
use common::plugins::{Running, start};
use serde_json::json;
use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::plugins::ulanzi::plugin::types::Status;
use ulanzi_deckd::plugins::{Event, Output};

const PACKAGE: &str = "ulanzi-plugin-obs";
const MANIFEST: &str = include_str!("../../../plugins/obs/plugin.toml");
const SOON: Duration = Duration::from_secs(20);

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

#[test]
fn obs_manifest_is_valid() {
    let m = Manifest::parse(MANIFEST).expect("manifest");
    assert_eq!(m.id, "obs");
    assert_eq!(m.name, "OBS Studio");
    assert!(m.background);
    assert_eq!(m.actions.len(), 22);
    assert_eq!(m.permissions.network, ["127.0.0.1", "localhost"]);
}

#[tokio::test]
async fn obs_plugin_end_to_end() {
    let mock = mock(Mode::Password("supersecretpassword")).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: json!({"host": "127.0.0.1", "port": mock.port, "password": "supersecretpassword"}),
    });
    let mut seen = Seen::default();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes"
            && s.state("scene") == "A")
            .await,
        "connected on scene A"
    );
    let ok = Output::Status {
        status: Status::Ok,
        message: "OBS 31.0.2 · WebSocket 5.5.4 · 2 scenes".into(),
    };
    assert!(seen.others.contains(&ok), "ok status");
    let saved = Output::SecretsChanged(BTreeMap::from([(
        "password".to_string(),
        "supersecretpassword".to_string(),
    )]));
    assert!(seen.others.contains(&saved), "password saved");

    assert_eq!(p.act("set_scene", json!({"scene": "B"})).await, Ok(()));
    let mut sent = None;
    for _ in 0..200 {
        sent = mock.last("SetCurrentProgramScene");
        if sent.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(sent, Some(json!({"sceneName": "B"})));

    mock.event(
        "RecordStateChanged",
        json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
    assert!(
        seen.wait(&mut p, |s| s.state("recording") == "yes").await,
        "recording"
    );

    mock.drop_all();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "no").await,
        "offline after drop"
    );
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes").await,
        "back online"
    );
    assert_eq!(mock.connections(), 2);
}

#[tokio::test]
async fn obs_plugin_wrong_password_and_foreign_host() {
    let mock = mock(Mode::Password("supersecretpassword")).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: json!({"host": "127.0.0.1", "port": mock.port, "password": "nope"}),
    });
    assert!(
        status_where(&mut p, Status::Problem, |m| m.starts_with("Wrong password"), SOON).await,
        "wrong password"
    );

    p.send(Event::Init {
        settings: json!({"host": "example.com", "port": mock.port}),
    });
    let foreign = Output::Status {
        status: Status::Problem,
        message: "OBS must run on this computer: use host = \"localhost\" or \"127.0.0.1\".".into(),
    };
    assert!(p.saw(&foreign, SOON).await, "foreign host refused");
    let before = mock.connections();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(mock.connections(), before, "no connection to a foreign host");
}
