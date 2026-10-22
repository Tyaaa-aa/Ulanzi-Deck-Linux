//! D-Bus API tests on a private bus (skipped when `dbus-daemon` is not installed).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use ulanzi_core::model::{Model, Profile, Settings};
use ulanzi_core::store::Store;
use ulanzi_deckd::actions::Executor;
use ulanzi_deckd::dbus::{BUS_NAME, DeckService, INTERFACE, OBJECT_PATH, forward_signals};
use ulanzi_deckd::deck::{DeckMsg, spawn_deck};
use ulanzi_deckd::transport::{MockHandle, MockTransport};
use zbus::Connection;

struct Bus {
    child: Child,
    addr: String,
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bus() -> Option<Bus> {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?).read_line(&mut line).ok()?;
    Some(Bus {
        child,
        addr: line.trim().to_string(),
    })
}

struct Rig {
    _bus: Bus,
    _dir: tempfile::TempDir,
    client: Connection,
    mock: MockHandle,
    profile: Profile,
}

async fn rig() -> Option<Rig> {
    let Some(bus) = bus() else {
        eprintln!("dbus-daemon not found; skipping");
        return None;
    };
    let dir = tempfile::tempdir().expect("tmp");
    let store = Store::at(dir.path().join("c"), dir.path().join("s"), dir.path().join("k"));
    let profile = Profile::new_default("Default", Model::D200X);
    store.save_profile(&profile).expect("save");
    let settings = Settings {
        default_profile: Some(profile.id),
        ..Settings::default()
    };
    let (tx, events) = spawn_deck(store.clone(), settings, Arc::new(Executor::new(None)));
    let (t, mock) = MockTransport::new();
    tx.send(DeckMsg::DeviceUp(Box::new(t), "/dev/mock".into()))
        .await
        .expect("up");

    let server = zbus::connection::Builder::address(bus.addr.as_str())
        .expect("addr")
        .name(BUS_NAME)
        .expect("name")
        .serve_at(OBJECT_PATH, DeckService::new(tx, store))
        .expect("serve")
        .build()
        .await
        .expect("server");
    tokio::spawn(forward_signals(server.clone(), events));
    std::mem::forget(server);
    let client = zbus::connection::Builder::address(bus.addr.as_str())
        .expect("addr")
        .build()
        .await
        .expect("client");
    tokio::time::sleep(Duration::from_millis(100)).await;
    Some(Rig {
        _bus: bus,
        _dir: dir,
        client,
        mock,
        profile,
    })
}

async fn proxy(c: &Connection) -> zbus::Proxy<'static> {
    zbus::Proxy::new(c, BUS_NAME, OBJECT_PATH, INTERFACE)
        .await
        .expect("proxy")
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_reports_state() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let (api, _ver, state, _detail, model, profile, page): (
        u32,
        String,
        String,
        String,
        String,
        String,
        u32,
    ) = p.call("Hello", &("test",)).await.expect("hello");
    assert_eq!(api, 1);
    assert_eq!(state, "ready");
    assert_eq!(model, "D200X");
    assert_eq!(profile, r.profile.id.to_string());
    assert_eq!(page, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_profile_bumps_revision_and_pushes() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let mut prof = r.profile.clone();
    prof.pages[0].keys.insert(
        0,
        ulanzi_core::model::Key {
            look: ulanzi_core::model::KeyLook {
                background: ulanzi_core::Color::rgb(200, 0, 0),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    r.mock.clear();
    let rev: u64 = p
        .call("ApplyProfile", &(toml::to_string(&prof).expect("toml"), 1u64))
        .await
        .expect("apply");
    assert_eq!(rev, 2);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(r.mock.commands().contains(&0x0001), "{:?}", r.mock.commands());
    let saved: String = p.call("GetProfile", &(prof.id.to_string(),)).await.expect("get");
    assert!(saved.contains("#c80000"));
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_revision_rejected() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let err = p
        .call::<_, _, u64>(
            "ApplyProfile",
            &(toml::to_string(&r.profile).expect("toml"), 5u64),
        )
        .await
        .expect_err("stale");
    assert!(err.to_string().contains("StaleRevision"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_profile_rejected_keeps_old() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let err = p
        .call::<_, _, u64>("ApplyProfile", &("name = [", 1u64))
        .await
        .expect_err("invalid");
    assert!(err.to_string().contains("Invalid"), "{err}");
    let saved: String = p
        .call("GetProfile", &(r.profile.id.to_string(),))
        .await
        .expect("get");
    assert!(saved.contains("name = \"Default\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn input_signal_emitted() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let mut sigs = p.receive_signal("InputEvent").await.expect("subscribe");
    let mut rep = vec![0x7c, 0x7c, 0x01, 0x01, 0, 0, 0, 0, 0, 2, 0, 1];
    rep.resize(1024, 0);
    r.mock.inject.send(Ok(rep)).await.expect("inject");
    let msg = tokio::time::timeout(Duration::from_secs(3), sigs.next())
        .await
        .expect("in time")
        .expect("signal");
    let (control, kind): (String, String) = msg.body().deserialize().expect("body");
    assert_eq!((control.as_str(), kind.as_str()), ("key:2", "press"));
}

#[tokio::test(flavor = "multi_thread")]
async fn second_instance_exits_3() {
    let Some(r) = rig().await else { return };
    let dir = tempfile::tempdir().expect("tmp");
    let out = Command::new(env!("CARGO_BIN_EXE_ulanzi-deckd"))
        .env("DBUS_SESSION_BUS_ADDRESS", &r._bus.addr)
        .env("XDG_CONFIG_HOME", dir.path().join("c"))
        .env("XDG_STATE_HOME", dir.path().join("s"))
        .env("XDG_CACHE_HOME", dir.path().join("k"))
        .output()
        .expect("run");
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
