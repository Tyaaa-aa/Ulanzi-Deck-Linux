//! The OBS plugin (`plugins/obs`) running in the real host against a mock obs-websocket.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use common::obs_mock::{Mode, mock};
use common::plugins::{PROMPTLY, Running, start};
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
    assert_eq!(m.contract, "1.1");
    let pickers: Vec<_> = m.pickers.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        pickers,
        [
            "scenes",
            "scene_keys",
            "audio_inputs",
            "scene_sources",
            "sources",
            "filters",
            "collections",
            "profiles"
        ]
    );
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
        status_where(
            &mut p,
            Status::Problem,
            |m| m.starts_with("OBS closed the connection during sign-in"),
            SOON
        )
        .await,
        "closed during sign-in"
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

#[tokio::test]
async fn obs_plugin_pickers_list_what_the_mock_holds() {
    let mock = mock(Mode::Open).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    // Before the plugin is told to connect, the pickers say so.
    assert_eq!(
        p.list("scenes", "", "").await,
        Err("Not connected to OBS".to_string())
    );
    p.send(Event::Init {
        settings: json!({"host": "127.0.0.1", "port": mock.port}),
    });
    let mut seen = Seen::default();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes"
            && s.state("scene") == "A")
            .await,
        "connected"
    );
    // The filter and list answers may land just after the status; ask until they are in.
    let mut filters = Vec::new();
    for _ in 0..100 {
        filters = p.list("filters", "", "").await.expect("filters");
        if filters.len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let names: Vec<_> = filters
        .iter()
        .map(|i| (i.group.as_deref(), i.id.as_str(), i.state.as_deref()))
        .collect();
    assert_eq!(
        names,
        [
            (Some("Cam"), "Color", Some("On")),
            (Some("Cam"), "Sharpen", Some("Off")),
            (Some("Mic"), "Noise", Some("On")),
        ]
    );
    let scenes = p.list("scenes", "", "").await.expect("scenes");
    let scenes: Vec<_> = scenes
        .iter()
        .map(|i| (i.id.as_str(), i.state.as_deref()))
        .collect();
    assert_eq!(scenes, [("A", Some("Live")), ("B", None)]);
    let sources = p.list("sources", "", "").await.expect("sources");
    let sources: Vec<_> = sources.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(sources, ["Cam", "Mic"]);
    let profiles = p.list("profiles", "quie", "").await.expect("profiles");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, "Quiet");
    let collections = p.list("collections", "", "").await.expect("collections");
    assert_eq!(collections[0].state.as_deref(), Some("Current"));
    let mic = p.list("audio_inputs", "", "").await.expect("inputs");
    assert_eq!(
        (mic[0].id.as_str(), mic[0].state.as_deref()),
        ("Mic", Some("Muted"))
    );
    let sources = p.list("scene_sources", "", "A").await.expect("sources");
    assert_eq!(sources[0].group.as_deref(), Some("A"));
}

/// Wait until the mock has received a `ty` request after the first `from` requests; its data.
async fn sent_since(mock: &common::obs_mock::Mock, from: usize, ty: &str) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + PROMPTLY;
    loop {
        if let Some((_, data)) = mock.sent().into_iter().skip(from).find(|(t, _)| t == ty) {
            return data;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {ty} reached the mock: {:?}",
            &mock.requests()[from.min(mock.requests().len())..]
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Every action in plugin.toml, through the real host, and the request each one sends to OBS.
#[tokio::test]
async fn obs_plugin_every_action_reaches_obs() {
    let mock = mock(Mode::Open).await;
    let mut p = start(PACKAGE, MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: json!({"host": "127.0.0.1", "port": mock.port}),
    });
    let mut seen = Seen::default();
    assert!(
        seen.wait(&mut p, |s| s.state("connected") == "yes"
            && s.state("visible.A/Cam") == "yes")
            .await,
        "connected with scene items"
    );

    // (action, params, request OBS gets, its data). Read-then-write actions list the write.
    let cases = [
        (
            "set_scene",
            json!({"scene": "B"}),
            "SetCurrentProgramScene",
            json!({"sceneName": "B"}),
        ),
        (
            "set_scene",
            json!({"scene": "B", "preview": true}),
            "SetCurrentPreviewScene",
            json!({"sceneName": "B"}),
        ),
        // The program scene is A: switch to B.
        (
            "toggle_scenes",
            json!({"a": "A", "b": "B"}),
            "SetCurrentProgramScene",
            json!({"sceneName": "B"}),
        ),
        (
            "toggle_source",
            json!({"scene": "A", "source": "Cam"}),
            "SetSceneItemEnabled",
            json!({"sceneName": "A", "sceneItemId": 1, "sceneItemEnabled": false}),
        ),
        ("toggle_record", json!({}), "ToggleRecord", json!({})),
        ("start_record", json!({}), "StartRecord", json!({})),
        ("stop_record", json!({}), "StopRecord", json!({})),
        ("pause_record", json!({}), "ToggleRecordPause", json!({})),
        ("toggle_stream", json!({}), "ToggleStream", json!({})),
        ("start_stream", json!({}), "StartStream", json!({})),
        ("stop_stream", json!({}), "StopStream", json!({})),
        ("toggle_replay", json!({}), "ToggleReplayBuffer", json!({})),
        ("save_replay", json!({}), "SaveReplayBuffer", json!({})),
        ("toggle_virtual_cam", json!({}), "ToggleVirtualCam", json!({})),
        (
            "toggle_mute",
            json!({"input": "Mic"}),
            "ToggleInputMute",
            json!({"inputName": "Mic"}),
        ),
        // The mock's Mic is at -1 dB.
        (
            "input_volume",
            json!({"input": "Mic", "op": "down", "step": 3}),
            "SetInputVolume",
            json!({"inputName": "Mic", "inputVolumeDb": -4.0}),
        ),
        (
            "input_volume",
            json!({"input": "Mic", "op": "set", "percent": 100}),
            "SetInputVolume",
            json!({"inputName": "Mic", "inputVolumeDb": 0.0}),
        ),
        (
            "input_volume",
            json!({"input": "Mic", "op": "mute"}),
            "ToggleInputMute",
            json!({"inputName": "Mic"}),
        ),
        // The mock answers GetSourceFilter without filterEnabled (off): turn it on.
        (
            "toggle_filter",
            json!({"source": "Cam", "filter": "Color"}),
            "SetSourceFilterEnabled",
            json!({"sourceName": "Cam", "filterName": "Color", "filterEnabled": true}),
        ),
        (
            "studio_transition",
            json!({}),
            "TriggerStudioModeTransition",
            json!({}),
        ),
        (
            "set_collection",
            json!({"name": "Stream"}),
            "SetCurrentSceneCollection",
            json!({"sceneCollectionName": "Stream"}),
        ),
        (
            "set_profile",
            json!({"name": "Quiet"}),
            "SetCurrentProfile",
            json!({"profileName": "Quiet"}),
        ),
        (
            "screenshot",
            json!({}),
            "TriggerHotkeyByName",
            json!({"hotkeyName": "OBSBasic.Screenshot"}),
        ),
    ];
    let mut ran: BTreeSet<&str> = BTreeSet::from(["reconnect", "forget_password"]);
    for (action, params, ty, want) in cases {
        ran.insert(action);
        let from = mock.sent().len();
        assert_eq!(p.act(action, params.clone()).await, Ok(()), "{action} {params}");
        assert_eq!(sent_since(&mock, from, ty).await, want, "{action} {params}");
    }
    // The two reads ahead of the writes went out too.
    assert!(mock.requests().contains(&"GetInputVolume".to_string()));
    assert_eq!(
        mock.last("GetSourceFilter"),
        Some(json!({"sourceName": "Cam", "filterName": "Color"}))
    );

    // reconnect: a second connection that loads again.
    let from = mock.sent().len();
    assert_eq!(p.act("reconnect", json!({})).await, Ok(()));
    sent_since(&mock, from, "GetVersion").await;
    assert_eq!(mock.connections(), 2);

    // forget_password: the saved password is deleted.
    p.send(Event::Init {
        settings: json!({"host": "127.0.0.1", "port": mock.port, "password": "pw"}),
    });
    let saved = Output::SecretsChanged(BTreeMap::from([("password".to_string(), "pw".to_string())]));
    assert!(p.saw(&saved, SOON).await, "password saved");
    assert_eq!(p.act("forget_password", json!({})).await, Ok(()));
    assert!(
        p.saw(&Output::SecretsChanged(BTreeMap::new()), SOON).await,
        "password deleted"
    );

    // Every action of the manifest ran above.
    let manifest = Manifest::parse(MANIFEST).expect("manifest");
    let declared: BTreeSet<&str> = manifest.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ran, declared);
}
