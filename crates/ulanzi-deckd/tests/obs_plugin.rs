//! The OBS plugin (`plugins/obs`) running in the real host against a mock obs-websocket.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::obs_mock::{Mode, mock};
use common::plugins::{Running, start, state_of};
use serde_json::json;
use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::plugins::ulanzi::plugin::types::Status;
use ulanzi_deckd::plugins::{Event, Output};

const PACKAGE: &str = "ulanzi-plugin-obs";
const MANIFEST: &str = include_str!("../../../plugins/obs/plugin.toml");
const SOON: Duration = Duration::from_secs(20);

/// Wait until `key` is `value` (any earlier values are skipped).
async fn until(p: &mut Running, key: &str, value: &str, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match state_of(p, key, left).await {
            Some(v) if v == value => return true,
            Some(_) => {}
            None => return false,
        }
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
    // Outputs arrive in order: the password is saved at init, the status comes before the first state.
    let saved = Output::SecretsChanged(BTreeMap::from([(
        "password".to_string(),
        "supersecretpassword".to_string(),
    )]));
    assert!(p.saw(&saved, SOON).await, "password saved");

    let ok = Output::Status {
        status: Status::Ok,
        message: "OBS 31.0.2 · WebSocket 5.5.4 · 2 scenes".into(),
    };
    assert!(p.saw(&ok, SOON).await, "ok status");
    assert!(until(&mut p, "connected", "yes", SOON).await, "connected");
    assert!(until(&mut p, "scene", "A", SOON).await, "scene");
    assert_eq!(p.act("set_scene", json!({"scene": "B"})).await, Ok(()));
    let mut seen = None;
    for _ in 0..200 {
        seen = mock.last("SetCurrentProgramScene");
        if seen.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(seen, Some(json!({"sceneName": "B"})));

    mock.event(
        "RecordStateChanged",
        json!({"outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"}),
    );
    assert!(until(&mut p, "recording", "yes", SOON).await, "recording");

    mock.drop_all();
    assert!(until(&mut p, "connected", "no", SOON).await, "offline after drop");
    assert!(until(&mut p, "connected", "yes", SOON).await, "back online");
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
