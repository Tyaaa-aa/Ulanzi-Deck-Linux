//! Client tests against a real daemon service on a private bus (skipped without `dbus-daemon`).

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use ulanzi_core::model::{Model, Profile, Settings};
use ulanzi_core::store::Store;
use ulanzi_deck::client::{Client, ClientError, ClientEvent, ImportChoice};
use ulanzi_deckd::actions::Executor;
use ulanzi_deckd::dbus::{BUS_NAME, DeckService, OBJECT_PATH, forward_plugins, forward_signals};
use ulanzi_deckd::deck::spawn_deck;
use ulanzi_deckd::plugins::{FileSecrets, PluginHost};
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

/// Start the daemon's D-Bus service (with a deck but no device, and a plugin host on the
/// `plugins` folder under `dir`) on `addr`.
async fn serve(addr: &str, store: &Store, dir: &Path) -> Connection {
    let settings = store.load_settings().unwrap_or_default();
    let (tx, events) = spawn_deck(store.clone(), settings.clone(), Arc::new(Executor::new(None)));
    let plugins = PluginHost::start(
        dir.join("plugins"),
        dir.join("k/plugins"),
        Arc::new(FileSecrets::new(dir.join("c/plugin-secrets.toml"))),
        &settings,
        &Default::default(),
        tx.clone(),
    )
    .await;
    let conn = zbus::connection::Builder::address(addr)
        .expect("addr")
        .name(BUS_NAME)
        .expect("name")
        .serve_at(
            OBJECT_PATH,
            DeckService::new(tx, store.clone())
                .with_plugins_dir(dir.join("plugins"))
                .with_plugins(plugins.clone()),
        )
        .expect("serve")
        .build()
        .await
        .expect("server");
    tokio::spawn(forward_signals(conn.clone(), events));
    tokio::spawn(forward_plugins(conn.clone(), plugins));
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
    rig_with(|_| {}).await
}

/// A rig whose plugins folder `setup` fills first.
async fn rig_with(setup: impl FnOnce(&Path)) -> Option<Rig> {
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
    std::fs::create_dir_all(dir.path().join("plugins")).expect("plugins dir");
    setup(&dir.path().join("plugins"));
    let server = Some(serve(&bus.addr, &store, dir.path()).await);
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
    r.server = Some(serve(&r.bus.addr, &r.store, r._dir.path()).await); // and comes back
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

#[tokio::test(flavor = "multi_thread")]
async fn bundle_round_trip_over_dbus() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let file = r._dir.path().join("x.ulanzideck");
    c.export_bundle(&file, &[r.profile.id], false)
        .await
        .expect("export");
    let preview = c.preview_bundle(&file).await.expect("preview");
    assert_eq!(preview.manifest.profiles.len(), 1);
    assert_eq!(preview.conflicts, vec!["Default".to_string()]);
    let done = c
        .import_bundle(&file, ImportChoice::KeepBoth, true, false, false)
        .await
        .expect("import");
    assert_eq!(done.imported.len(), 1);
    let snaps = c.list_snapshots().await.expect("snapshots");
    assert!(snaps.iter().any(|s| s.name == done.undo));
    let undone = c.restore_snapshot(&done.undo).await.expect("restore");
    assert!(!undone.is_empty());
    let names: Vec<String> = c
        .profiles()
        .await
        .expect("list")
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert_eq!(names, vec!["Default".to_string()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_list_and_manifests_empty() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    assert_eq!(c.plugin_list().await.expect("list"), vec![]);
    assert!(c.plugin_manifests().await.expect("manifests").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_errors_reach_the_gui() {
    let Some(r) = rig().await else { return };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    match c.plugin_set_secret("hello", "password", "x").await {
        Err(ClientError::Service(m)) => assert_eq!(m, "the hello plugin is not installed"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn plugins_changed_is_forwarded() {
    let Some(r) = rig_with(|plugins| {
        let folder = plugins.join("hello");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(
            folder.join("plugin.toml"),
            include_str!("../../../examples/plugins/hello/plugin.toml"),
        )
        .expect("manifest");
    })
    .await
    else {
        return;
    };
    let c = Client::connect_to(&r.bus.addr).await.expect("connect");
    let list = c.plugin_list().await.expect("list");
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(
        (list[0].id.as_str(), list[0].status.as_str()),
        ("hello", "incompatible")
    );
    // Let the signal sent when the host came up go by.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut events = c.events();
    while events.try_recv().is_ok() {}
    c.plugin_remove("hello").await.expect("remove");
    let got = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.recv().await {
                Ok(ClientEvent::PluginsChanged) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(got, "PluginsChanged reaches the GUI");
    assert_eq!(c.plugin_list().await.expect("list"), vec![]);
}
