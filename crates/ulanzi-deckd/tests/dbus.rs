//! D-Bus API tests on a private bus (skipped when `dbus-daemon` is not installed).

mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use ulanzi_core::model::{Model, Profile, Settings};
use ulanzi_core::plugin::{Inspected, Manifest, PickItem};
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
    rig_full(setup, true).await
}

/// `host_up` false: the service has the plugins folder but no plugin host yet.
async fn rig_full(setup: impl FnOnce(&std::path::Path), host_up: bool) -> Option<Rig> {
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
        .serve_at(OBJECT_PATH, {
            let svc = DeckService::new(tx, store).with_plugins_dir(dir.path().join("plugins"));
            if host_up { svc.with_plugins(plugins) } else { svc }
        })
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

type Imported = (Vec<String>, Vec<String>, String, Vec<String>);
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
    let (imported, _warnings, _undo, _notes): Imported = p
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
    let (imported, _, undo, _): Imported = p
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

const PROBE_PACKAGE: &str = "ulanzi-plugin-probe";
const PROBE_MANIFEST: &str = include_str!("plugins/probe/plugin.toml");

/// A rig with the probe plugin installed.
async fn probe_rig() -> Option<Rig> {
    rig_with(|plugins| {
        let folder = plugins.join("probe");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("plugin.toml"), PROBE_MANIFEST).expect("manifest");
        std::fs::copy(common::plugins::wasm(PROBE_PACKAGE), folder.join("plugin.wasm")).expect("wasm");
    })
    .await
}

const WITH_SECRET: &str = "[plugins.probe]\nenabled = true\n\n[plugins.probe.settings]\npassword = \"hunter2\"\ngreeting = \"hi\"\n";

async fn stored(r: &Rig) -> Settings {
    toml::from_str(&std::fs::read_to_string(r._dir.path().join("c/settings.toml")).expect("read"))
        .expect("toml")
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_manifests_json() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    let json: String = p.call("PluginManifests", &()).await.expect("manifests");
    let m: BTreeMap<String, Manifest> = serde_json::from_str(&json).expect("json");
    assert_eq!(m["probe"].settings[0].key, "password");
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_list_items_round_trip() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    let json: String = p
        .call("PluginListItems", &("probe", "things", "lamp", "kitchen"))
        .await
        .expect("items");
    let items: Vec<PickItem> = serde_json::from_str(&json).expect("json");
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0].label, "One lamp kitchen");
    assert_eq!(items[1].subtitle, None);
    let name = |e: &zbus::Error| match e {
        zbus::Error::MethodError(name, ..) => name.to_string(),
        other => panic!("not an error reply: {other}"),
    };
    let e = p
        .call::<_, _, String>("PluginListItems", &("ghost", "things", "", ""))
        .await
        .expect_err("unknown plugin");
    assert_eq!(name(&e), "io.github.tyaaa_aa.UlanziDeck1.Error.NotFound");
    assert_eq!(error_message(e), "the ghost plugin is not installed");
    let e = p
        .call::<_, _, String>("PluginListItems", &("probe", "nope", "", ""))
        .await
        .expect_err("undeclared picker");
    assert_eq!(name(&e), "io.github.tyaaa_aa.UlanziDeck1.Error.Invalid");
    let e = p
        .call::<_, _, String>("PluginListItems", &("probe", "refuse", "", ""))
        .await
        .expect_err("plugin refuses");
    assert_eq!(name(&e), "io.github.tyaaa_aa.UlanziDeck1.Error.Unavailable");
    assert_eq!(error_message(e), "not connected");
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_inspect_dir_json() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    let folder = common::plugins::fixture(PROBE_PACKAGE, PROBE_MANIFEST);
    let json: String = p
        .call("PluginInspectDir", &(folder.path().to_string_lossy().as_ref(),))
        .await
        .expect("inspect");
    let i: Inspected = serde_json::from_str(&json).expect("json");
    assert_eq!(i.manifest.id, "probe");
    assert_eq!(i.installed_version.as_deref(), Some("1.0.0"));
    let e = p
        .call::<_, _, String>("PluginInspectDir", &("plugins/probe",))
        .await
        .expect_err("relative path");
    assert_eq!(
        error_message(e),
        "the plugin folder must be an absolute path, not \"plugins/probe\""
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_inspect_and_install_file_round_trip() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let file = common::plugins::package_file(
        r._dir.path(),
        "probe-1.0.0.ulanziplugin",
        PROBE_PACKAGE,
        PROBE_MANIFEST,
        &[],
    );
    let path = file.to_string_lossy().into_owned();
    // Inspecting works without Developer mode.
    let json: String = p
        .call("PluginInspectFile", &(path.as_str(),))
        .await
        .expect("inspect");
    let i: Inspected = serde_json::from_str(&json).expect("json");
    assert_eq!(i.manifest.id, "probe");
    assert_eq!(i.installed_version, None);
    // Installing needs it.
    let e = p
        .call::<_, _, String>("PluginInstallFile", &(path.as_str(),))
        .await
        .expect_err("developer mode is off");
    assert_eq!(
        error_message(e),
        "Turn on Developer mode to install unverified plugins"
    );
    p.call::<_, _, ()>("ApplySettings", &("developer_mode = true\n",))
        .await
        .expect("apply");
    let id: String = p
        .call("PluginInstallFile", &(path.as_str(),))
        .await
        .expect("install");
    assert_eq!(id, "probe");
    assert!(r._dir.path().join("plugins/probe/plugin.wasm").is_file());
    let json: String = p
        .call("PluginInspectFile", &(path.as_str(),))
        .await
        .expect("inspect again");
    let i: Inspected = serde_json::from_str(&json).expect("json");
    assert_eq!(i.installed_version.as_deref(), Some("1.0.0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn plugin_file_methods_need_an_absolute_path() {
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    for method in ["PluginInspectFile", "PluginInstallFile"] {
        let e = p
            .call::<_, _, String>(method, &("probe.ulanziplugin",))
            .await
            .expect_err("relative path");
        assert_eq!(
            error_message(e),
            "the plugin file must be an absolute path, not \"probe.ulanziplugin\""
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_settings_masks_plugin_secrets() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(WITH_SECRET,))
        .await
        .expect("apply");
    let text: String = p.call("GetSettings", &()).await.expect("get");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("password = \"••••\""), "{text}");
    assert!(text.contains("greeting = \"hi\""), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn placeholder_round_trip_keeps_or_drops() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(WITH_SECRET,))
        .await
        .expect("apply");
    let masked: String = p.call("GetSettings", &()).await.expect("get");
    p.call::<_, _, ()>("ApplySettings", &(masked.as_str(),))
        .await
        .expect("apply");
    assert_eq!(
        stored(&r).await.plugins["probe"].settings["password"].as_str(),
        Some("hunter2")
    );
    p.call::<_, _, ()>("PluginSetSecret", &("probe", "password", "new"))
        .await
        .expect("set");
    p.call::<_, _, ()>("ApplySettings", &(masked.as_str(),))
        .await
        .expect("apply");
    let s = stored(&r).await;
    let t = &s.plugins["probe"].settings;
    assert!(!t.contains_key("password"), "{t:?}");
    assert_eq!(t["greeting"].as_str(), Some("hi"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_secret_strips_the_settings_copy() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(WITH_SECRET,))
        .await
        .expect("apply");
    let mut signals = p.receive_signal("SettingsChanged").await.expect("signal");
    p.call::<_, _, ()>("PluginSetSecret", &("probe", "password", "new"))
        .await
        .expect("set");
    assert!(
        !stored(&r).await.plugins["probe"]
            .settings
            .contains_key("password")
    );
    let keys: Vec<String> = p.call("PluginSecretKeys", &("probe",)).await.expect("keys");
    assert_eq!(keys, ["password"]);
    tokio::time::timeout(Duration::from_secs(5), signals.next())
        .await
        .expect("SettingsChanged arrives");
    p.call::<_, _, ()>("PluginClearSecret", &("probe", "password"))
        .await
        .expect("clear");
    let keys: Vec<String> = p.call("PluginSecretKeys", &("probe",)).await.expect("keys");
    assert!(keys.is_empty(), "{keys:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn set_secret_errors_use_the_user_facing_text() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    let e = p
        .call::<_, _, ()>("PluginSetSecret", &("probe", "other", "x"))
        .await
        .expect_err("undeclared");
    assert_eq!(error_message(e), "Probe has no secret setting “other”");
    let e = p
        .call::<_, _, ()>("PluginSetSecret", &("probe", "password", ""))
        .await
        .expect_err("empty");
    assert_eq!(error_message(e), "Enter a value, or use Clear");
    let e = p
        .call::<_, _, ()>("PluginSetSecret", &("nope", "password", "x"))
        .await
        .expect_err("unknown");
    assert_eq!(error_message(e), "the nope plugin is not installed");
}

fn install_probe(plugins: &std::path::Path) {
    let folder = plugins.join("probe");
    std::fs::create_dir_all(&folder).expect("folder");
    std::fs::write(folder.join("plugin.toml"), PROBE_MANIFEST).expect("manifest");
    std::fs::copy(common::plugins::wasm(PROBE_PACKAGE), folder.join("plugin.wasm")).expect("wasm");
}

const NAMED_SECRETS: &str =
    "[plugins.gone]\nenabled = true\n\n[plugins.gone.settings]\nApi_Token = \"t0ken\"\nhost = \"h\"\n";

#[tokio::test(flavor = "multi_thread")]
async fn removed_plugin_secrets_stay_masked() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(WITH_SECRET,))
        .await
        .expect("apply");
    p.call::<_, _, ()>("PluginRemove", &("probe",))
        .await
        .expect("remove");
    let text: String = p.call("GetSettings", &()).await.expect("get");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("password = \"••••\""), "{text}");
    assert!(text.contains("greeting = \"hi\""), "{text}");
    // The masked text written back keeps the stored value.
    p.call::<_, _, ()>("ApplySettings", &(text.as_str(),))
        .await
        .expect("apply");
    assert_eq!(
        stored(&r).await.plugins["probe"].settings["password"].as_str(),
        Some("hunter2")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unparsable_manifest_secrets_are_masked_by_name() {
    let Some(r) = rig_with(|plugins| {
        std::fs::create_dir_all(plugins.join("gone")).expect("dir");
        std::fs::write(plugins.join("gone/plugin.toml"), "not a manifest [").expect("write");
    })
    .await
    else {
        return;
    };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(NAMED_SECRETS,))
        .await
        .expect("apply");
    let text: String = p.call("GetSettings", &()).await.expect("get");
    assert!(!text.contains("t0ken"), "{text}");
    assert!(text.contains("host = \"h\""), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn secrets_are_masked_before_the_host_is_up() {
    let Some(r) = rig_full(install_probe, false).await else {
        return;
    };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(WITH_SECRET,))
        .await
        .expect("apply");
    let text: String = p.call("GetSettings", &()).await.expect("get");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("greeting = \"hi\""), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn export_leaves_out_plugin_secrets() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    let mixed = format!("{WITH_SECRET}\n{NAMED_SECRETS}");
    p.call::<_, _, ()>("ApplySettings", &(mixed.as_str(),))
        .await
        .expect("apply");
    let out = r._dir.path().join("x.ulanzideck");
    p.call::<_, _, ()>(
        "ExportBundle",
        &(out.to_string_lossy().as_ref(), Vec::<String>::new(), true),
    )
    .await
    .expect("export");
    let b = ulanzi_core::bundle::read(&std::fs::read(&out).expect("read")).expect("bundle");
    let s = b.settings.expect("settings");
    assert!(!s.plugins["probe"].settings.contains_key("password"));
    assert_eq!(s.plugins["probe"].settings["greeting"].as_str(), Some("hi"));
    assert!(!s.plugins["gone"].settings.contains_key("Api_Token"));
    assert_eq!(s.plugins["gone"].settings["host"].as_str(), Some("h"));
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_set_secret_keeps_the_settings_copy() {
    let Some(r) = probe_rig().await else { return };
    let p = proxy(&r.client).await;
    p.call::<_, _, ()>("ApplySettings", &(WITH_SECRET,))
        .await
        .expect("apply");
    // "greeting" is not a secret setting: the call fails and nothing is stripped.
    p.call::<_, _, ()>("PluginSetSecret", &("probe", "greeting", "x"))
        .await
        .expect_err("undeclared");
    let s = stored(&r).await;
    assert_eq!(s.plugins["probe"].settings["password"].as_str(), Some("hunter2"));
    assert_eq!(s.plugins["probe"].settings["greeting"].as_str(), Some("hi"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_from_a_newer_version_survives_the_window() {
    use ulanzi_core::model::{ForeignSlot, Slot};
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let mut profile = r.profile.clone();
    profile.foreign.push(ForeignSlot {
        page: Some(profile.pages[0].id),
        slot: Slot::Key(3),
        raw: toml::Value::Table(toml::from_str("tap = { type = \"macro\" }").expect("raw")),
    });
    let text = ulanzi_core::store::profile_to_toml(&profile).expect("toml");
    let _: u64 = p
        .call("ApplyProfile", &(text.as_str(), 1u64))
        .await
        .expect("apply");
    let back: String = p
        .call("GetProfile", &(profile.id.to_string(),))
        .await
        .expect("get");
    assert!(back.contains("macro"), "{back}");
}

#[tokio::test(flavor = "multi_thread")]
#[allow(unsafe_code)]
async fn an_update_is_found_fetched_and_swapped_in() {
    use common::http::{reply, serve_late};
    const KEY: &str = "RWSs/atOYGdNly+XXewka2b44HRpYWS3v+8YvPtCQRRCzq2cw/8hVmRJ";
    const SUMS: &str = "84293ed06cb3210e7d549afec3140d0c48494416ad25b7f25196afffaa5eb796  Ulanzi_Deck-9.9.9-x86_64.AppImage\n";
    const SIG: &str = "untrusted comment: signature from minisign secret key
RUSs/atOYGdNl4/T6e0ANX1ZNjzAj99eG41ifrnMSPXk7oI56tOYLznY46X4ThsNc3mjcsjPLzsdCr5qmXhS2Cm81zdt+saSlgs=
trusted comment: file:SHA256SUMS
6oGLrfaCxJlDrCBCLbzjjfkgKcH3KzE3640CbHDEyNNWd7XNiweYbG/Neacs+qB085vpcaQil9606E3hJ4X5Ag==
";
    let Some(r) = rig().await else { return };
    let p = proxy(&r.client).await;
    let appimage = r._dir.path().join("Ulanzi.AppImage");
    std::fs::write(&appimage, b"old").expect("write");
    let (base, _) = serve_late(|base| {
        let asset = |n: &str, size: usize| {
            format!(r#"{{"name":"{n}","browser_download_url":"{base}/{n}","size":{size}}}"#)
        };
        let latest = format!(
            r#"{{"tag_name":"v9.9.9","name":"","draft":false,"prerelease":false,"published_at":"","body":"","assets":[{},{},{}]}}"#,
            asset("Ulanzi_Deck-9.9.9-x86_64.AppImage", 11),
            asset("SHA256SUMS", SUMS.len()),
            asset("SHA256SUMS.minisig", SIG.len()),
        );
        vec![
            reply("/latest", 200, latest.as_bytes()),
            reply("/Ulanzi_Deck-9.9.9-x86_64.AppImage", 200, b"asset bytes"),
            reply("/SHA256SUMS", 200, SUMS.as_bytes()),
            reply("/SHA256SUMS.minisig", 200, SIG.as_bytes()),
        ]
    });
    // Nothing else in this test program asks about updates.
    unsafe {
        std::env::set_var("ULANZI_RELEASES_URL", format!("{base}/latest"));
        std::env::set_var("ULANZI_TEST_RELEASE_KEY", KEY);
        std::env::set_var("APPIMAGE", &appimage);
        // The test program itself plays the program inside the image.
        std::env::set_var("APPDIR", "/");
    }
    let json: String = p.call("UpdateCheck", &()).await.expect("check");
    let status: ulanzi_core::update::UpdateStatus = serde_json::from_str(&json).expect("status");
    assert!(status.can_install, "{json}");
    let file: String = p.call("UpdateDownload", &()).await.expect("download");
    let good = std::fs::read(&file).expect("read");
    std::fs::write(&file, b"changed afterwards").expect("write");
    assert!(
        p.call::<_, _, String>("UpdateApply", &()).await.is_err(),
        "the file is checked again at the point of use"
    );
    assert_eq!(std::fs::read(&appimage).expect("read"), b"old");
    std::fs::write(&file, good).expect("write");
    let applied: String = p.call("UpdateApply", &()).await.expect("apply");
    assert_eq!(applied, file);
    assert_eq!(std::fs::read(&appimage).expect("read"), b"asset bytes");
    let snaps: Snapshots = p.call("ListSnapshots", &()).await.expect("snapshots");
    assert!(snaps.iter().any(|s| s.2 == "before-9-9-9"), "{snaps:?}");
}
