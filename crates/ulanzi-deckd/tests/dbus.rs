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
use ulanzi_deckd::plugins::{FileSecrets, PluginHost};
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
    rig_with(|_| {}).await
}

/// A rig whose plugins folder `setup` fills first.
async fn rig_with(setup: impl FnOnce(&std::path::Path)) -> Option<Rig> {
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
    std::fs::create_dir_all(dir.path().join("plugins")).expect("plugins dir");
    setup(&dir.path().join("plugins"));
    let plugins = PluginHost::start(
        dir.path().join("plugins"),
        dir.path().join("k/plugins"),
        Arc::new(FileSecrets::new(dir.path().join("c/plugin-secrets.toml"))),
        &Settings::default(),
        &Default::default(),
        tx.clone(),
    )
    .await;
    let (t, mock) = MockTransport::new();
    tx.send(DeckMsg::DeviceUp(Box::new(t), "/dev/mock".into()))
        .await
        .expect("up");
    // Answer the info request like the real deck, so the page upload is not held back.
    let mut info = vec![0x7c, 0x7c, 0x03, 0x03, 2, 0, 0, 0, b'{', b'}'];
    info.resize(1024, 0);
    mock.inject.send(Ok(info)).await.expect("info");

    let server = zbus::connection::Builder::address(bus.addr.as_str())
        .expect("addr")
        .name(BUS_NAME)
        .expect("name")
        .serve_at(OBJECT_PATH, DeckService::new(tx, store).with_plugins(plugins))
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
    // Uploads are spaced at least 2 s apart.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let cmds = r.mock.commands();
    assert!(cmds.contains(&0x0001) || cmds.contains(&0x000d), "{cmds:?}");
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

#[tokio::test(flavor = "multi_thread")]
async fn settings_roundtrip_hides_password() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let mut s: Settings =
        toml::from_str(&p.call::<_, _, String>("GetSettings", &()).await.expect("get")).expect("toml");
    s.brightness = 42;
    s.obs.password = Some("hunter2".into());
    p.call::<_, _, ()>("ApplySettings", &(toml::to_string(&s).expect("toml"),))
        .await
        .expect("apply");
    let text: String = p.call("GetSettings", &()).await.expect("get");
    assert!(!text.contains("hunter2"), "{text}");
    let back: Settings = toml::from_str(&text).expect("toml");
    assert_eq!(back.brightness, 42);
    assert_eq!(back.obs.password.as_deref(), Some("••••"));
}

#[tokio::test(flavor = "multi_thread")]
async fn apply_settings_keeps_password_placeholder() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let mut s = Settings::default();
    s.obs.password = Some("hunter2".into());
    p.call::<_, _, ()>("ApplySettings", &(toml::to_string(&s).expect("toml"),))
        .await
        .expect("apply");
    s.obs.password = Some("••••".into());
    s.brightness = 7;
    p.call::<_, _, ()>("ApplySettings", &(toml::to_string(&s).expect("toml"),))
        .await
        .expect("apply");
    let saved = std::fs::read_to_string(r._dir.path().join("c/settings.toml")).expect("read");
    assert!(
        saved.contains("hunter2"),
        "the real password must survive a round trip through the GUI"
    );
    assert!(saved.contains("brightness = 7"));
}

#[tokio::test(flavor = "multi_thread")]
async fn create_and_delete_profile() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let id: String = p.call("CreateProfile", &("Streaming",)).await.expect("create");
    let list: Vec<(String, String)> = p.call("ListProfiles", &()).await.expect("list");
    assert!(list.iter().any(|(i, n)| *i == id && n == "Streaming"), "{list:?}");
    p.call::<_, _, ()>("DeleteProfile", &(id.clone(),))
        .await
        .expect("delete");
    let list: Vec<(String, String)> = p.call("ListProfiles", &()).await.expect("list");
    assert!(!list.iter().any(|(i, _)| *i == id));
    let err = p
        .call::<_, _, ()>("DeleteProfile", &(r.profile.id.to_string(),))
        .await
        .expect_err("last one");
    assert!(err.to_string().contains("Invalid"), "{err}");
    // Deleted profiles are moved aside, not destroyed.
    let aside = std::fs::read_dir(r._dir.path().join("c/profiles"))
        .expect("ls")
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".deleted-"));
    assert!(aside);
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_revision_is_exposed() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let rev: u64 = p
        .call("ProfileRevision", &(r.profile.id.to_string(),))
        .await
        .expect("rev");
    assert_eq!(rev, 1);
}

type Imported = (Vec<String>, Vec<String>, String);
type Snapshots = Vec<(String, String, String, Vec<String>)>;

#[tokio::test(flavor = "multi_thread")]
async fn export_then_import_round_trip() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let file = r._dir.path().join("out.ulanzideck");
    let path = file.to_string_lossy().into_owned();
    p.call::<_, _, ()>("ExportBundle", &(path.as_str(), Vec::<String>::new(), false))
        .await
        .expect("export");
    let (manifest, conflicts): (String, Vec<String>) =
        p.call("PreviewBundle", &(path.as_str(),)).await.expect("preview");
    assert!(manifest.contains("Default"), "{manifest}");
    assert_eq!(conflicts, vec!["Default".to_string()]);
    let (imported, _warnings, _undo): Imported = p
        .call("ImportBundle", &(path.as_str(), "keep_both", true, false, false))
        .await
        .expect("import");
    assert_eq!(imported.len(), 1);
    let list: Vec<(String, String)> = p.call("ListProfiles", &()).await.expect("list");
    assert!(list.iter().any(|(_, n)| n == "Default (2)"), "{list:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn import_takes_snapshot_and_undo_restores() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let file = r._dir.path().join("out.ulanzideck");
    let path = file.to_string_lossy().into_owned();
    p.call::<_, _, ()>("ExportBundle", &(path.as_str(), Vec::<String>::new(), false))
        .await
        .expect("export");
    let (imported, _, undo): Imported = p
        .call("ImportBundle", &(path.as_str(), "keep_both", true, false, false))
        .await
        .expect("import");
    let snaps: Snapshots = p.call("ListSnapshots", &()).await.expect("list");
    assert!(
        snaps.iter().any(|s| s.0 == undo && s.2 == "before-import"),
        "{snaps:?}"
    );
    let _: String = p
        .call("RestoreSnapshot", &(undo.as_str(),))
        .await
        .expect("restore");
    let list: Vec<(String, String)> = p.call("ListProfiles", &()).await.expect("list");
    assert!(!list.iter().any(|(i, _)| *i == imported[0]), "{list:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn import_replace_reloads_open_profile() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let id = r.profile.id.to_string();
    let file = r._dir.path().join("out.ulanzideck");
    let path = file.to_string_lossy().into_owned();
    p.call::<_, _, ()>("ExportBundle", &(path.as_str(), vec![id.clone()], false))
        .await
        .expect("export");
    let before: u64 = p.call("ProfileRevision", &(id.as_str(),)).await.expect("rev");
    let _: Imported = p
        .call("ImportBundle", &(path.as_str(), "replace", true, false, false))
        .await
        .expect("import");
    let after: u64 = p.call("ProfileRevision", &(id.as_str(),)).await.expect("rev");
    assert!(
        after > before,
        "an open editor must see the change ({before} → {after})"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_profile_takes_snapshot() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let id: String = p.call("CreateProfile", &("Temp",)).await.expect("create");
    p.call::<_, _, ()>("DeleteProfile", &(id,)).await.expect("delete");
    let snaps: Snapshots = p.call("ListSnapshots", &()).await.expect("list");
    assert!(snaps.iter().any(|s| s.2 == "before-delete"), "{snaps:?}");
}

/// The message of a D-Bus error reply (what the user is shown).
fn error_message(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(_, Some(msg), _) => msg,
        other => panic!("not an error reply: {other}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_list_empty() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let list: Vec<(String, String, String, String, String)> = p.call("PluginList", &()).await.expect("list");
    assert_eq!(list, vec![]);
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_install_dir_needs_developer_mode() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let folder = r._dir.path().join("hello");
    std::fs::create_dir_all(&folder).expect("folder");
    let e = p
        .call::<_, _, String>("PluginInstallDir", &(folder.to_string_lossy().as_ref(),))
        .await
        .expect_err("developer mode is off");
    assert_eq!(
        error_message(e),
        "Turn on Developer mode to install plugins from a folder"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_install_dir_needs_an_absolute_path() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let e = p
        .call::<_, _, String>("PluginInstallDir", &("plugins/hello",))
        .await
        .expect_err("relative path");
    assert_eq!(
        error_message(e),
        "the plugin folder must be an absolute path, not \"plugins/hello\""
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_errors_use_the_user_facing_text() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    for method in ["PluginRemove", "PluginRestart"] {
        let e = p
            .call::<_, _, ()>(method, &("hello",))
            .await
            .expect_err("not installed");
        assert_eq!(error_message(e), "the hello plugin is not installed", "{method}");
    }
    let e = p
        .call::<_, _, ()>("PluginSetEnabled", &("hello", false))
        .await
        .expect_err("not installed");
    assert_eq!(error_message(e), "the hello plugin is not installed");
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_set_enabled_writes_settings() {
    // A folder without plugin.wasm: listed (as incompatible), and its switch can still be set.
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
    let p = proxy(&r.client).await;
    let list: Vec<(String, String, String, String, String)> = p.call("PluginList", &()).await.expect("list");
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].0, "hello");
    assert_eq!(list[0].3, "incompatible");
    assert!(!list[0].4.is_empty(), "a reason is given");
    p.call::<_, _, ()>("PluginSetEnabled", &("hello", false))
        .await
        .expect("set");
    let s: Settings =
        toml::from_str(&p.call::<_, _, String>("GetSettings", &()).await.expect("get")).expect("toml");
    assert!(!s.plugins["hello"].enabled);
    p.call::<_, _, ()>("PluginSetEnabled", &("hello", true))
        .await
        .expect("set");
    let saved = std::fs::read_to_string(r._dir.path().join("c/settings.toml")).expect("read");
    let s: Settings = toml::from_str(&saved).expect("toml");
    assert!(s.plugins["hello"].enabled);
}
