//! Client tests against a real daemon service on a private bus (skipped without `dbus-daemon`).

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ulanzi_core::model::{Model, Profile, Settings};
use ulanzi_core::store::Store;
use ulanzi_deck::client::{Client, ClientEvent};
use ulanzi_deckd::actions::Executor;
use ulanzi_deckd::dbus::{BUS_NAME, DeckService, OBJECT_PATH, forward_signals};
use ulanzi_deckd::deck::spawn_deck;
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

/// Start the daemon's D-Bus service (with a deck but no device) on `addr`.
async fn serve(addr: &str, store: &Store) -> Connection {
    let settings = store.load_settings().unwrap_or_default();
    let (tx, events) = spawn_deck(store.clone(), settings, Arc::new(Executor::new(None)));
    let conn = zbus::connection::Builder::address(addr)
        .expect("addr")
        .name(BUS_NAME)
        .expect("name")
        .serve_at(OBJECT_PATH, DeckService::new(tx, store.clone()))
        .expect("serve")
        .build()
        .await
        .expect("server");
    tokio::spawn(forward_signals(conn.clone(), events));
    conn
}

struct Rig {
    bus: Bus,
    _dir: tempfile::TempDir,
    store: Store,
    profile: Profile,
    server: Option<Connection>,
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
    store
        .save_settings(&Settings {
            default_profile: Some(profile.id),
            ..Settings::default()
        })
        .expect("settings");
    let server = Some(serve(&bus.addr, &store).await);
    Some(Rig {
        bus,
        _dir: dir,
        store,
        profile,
        server,
    })
}

fn renamed(p: &Profile, name: &str) -> Profile {
    Profile {
        name: name.into(),
        ..p.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hello_and_profile() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let h = c.hello().await.expect("hello");
    assert_eq!(h.api_version, 1);
    assert_eq!(h.active_profile, r.profile.id);
    let p = c.profile(r.profile.id).await.expect("profile");
    assert_eq!(p, r.profile);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_debounces_rapid_applies() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let base = c.profile(r.profile.id).await.expect("profile");
    for n in 0..10 {
        c.apply(renamed(&base, &format!("Name {n}")));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    c.flush().await;
    assert_eq!(c.revision(r.profile.id), 2, "ten quick edits are one save");
    assert_eq!(r.store.load_profile(r.profile.id).expect("load").name, "Name 9");
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_revision_reloads_then_reapplies() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let other = Client::connect_to(&r.bus.addr).await.expect("connect");
    let base = c.profile(r.profile.id).await.expect("profile");
    other.profile(r.profile.id).await.expect("profile");
    other.apply(renamed(&base, "From another window"));
    other.flush().await;
    let mut events = c.events();
    c.apply(renamed(&base, "Mine"));
    c.flush().await;
    assert_eq!(
        r.store.load_profile(r.profile.id).expect("load").name,
        "Mine",
        "the pending edit is not dropped"
    );
    let saw_conflict =
        std::iter::from_fn(|| events.try_recv().ok()).any(|e| matches!(e, ClientEvent::Conflict { .. }));
    assert!(saw_conflict, "the user is told another app changed the profile");
}

#[tokio::test(flavor = "multi_thread")]
async fn client_reconnects_and_resends_pending_edit() {
    let Some(mut r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let base = c.profile(r.profile.id).await.expect("profile");
    let mut events = c.events();
    if let Some(server) = r.server.take() {
        server.close().await.expect("close"); // the daemon goes away
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    c.apply(renamed(&base, "Edited while the service was down"));
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_ne!(
        r.store.load_profile(r.profile.id).expect("load").name,
        "Edited while the service was down"
    );
    r.server = Some(serve(&r.bus.addr, &r.store).await); // and comes back
    let mut landed = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if r.store.load_profile(r.profile.id).expect("load").name == "Edited while the service was down" {
            landed = true;
            break;
        }
    }
    let seen: Vec<ClientEvent> = std::iter::from_fn(|| events.try_recv().ok()).collect();
    assert!(
        landed,
        "the pending edit is sent once the service is back; events: {seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(e, ClientEvent::Disconnected)),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(e, ClientEvent::Reconnected)),
        "{seen:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_through_the_client() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let mut s = c.settings().await.expect("settings");
    s.brightness = 33;
    c.apply_settings(&s).await.expect("apply");
    assert_eq!(r.store.load_settings().expect("load").brightness, 33);
}

/// The window's GApplication ID and the service's bus name must differ, or the window takes the
/// service for a running copy of itself and exits.
#[test]
fn app_id_differs_from_service_bus_name() {
    assert_ne!(ulanzi_deck::APP_ID, ulanzi_deck::client::BUS_NAME);
    assert_eq!(ulanzi_deckd::dbus::BUS_NAME, ulanzi_deck::client::BUS_NAME);
}
