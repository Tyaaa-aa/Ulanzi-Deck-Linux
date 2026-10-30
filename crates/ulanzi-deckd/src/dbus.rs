//! The session-bus API (`io.github.tyaaa_aa.UlanziDeck1`) used by the GUI and `ulanzictl`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use tokio::sync::{broadcast, mpsc, oneshot};
use ulanzi_core::model::{BrightnessOp, Model, Profile, Settings, is_plugin_secret};
use ulanzi_core::store::{ProfileLoad, Store, parse_profile};
use ulanzi_proto::InputKind;
use uuid::Uuid;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

use crate::deck::{DeckEvent, DeckMsg, DeckSnapshot};
use crate::plugins::{PluginError, PluginHost};

pub const BUS_NAME: &str = "io.github.tyaaa_aa.UlanziDeck.Service";
pub const OBJECT_PATH: &str = "/io/github/tyaaa_aa/UlanziDeck";
pub const INTERFACE: &str = "io.github.tyaaa_aa.UlanziDeck1";
pub const API_VERSION: u32 = 1;
/// What the GUI sees instead of a stored OBS password; sending it back keeps the stored one.
pub const PASSWORD_PLACEHOLDER: &str = "••••";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "io.github.tyaaa_aa.UlanziDeck1.Error")]
pub enum DeckError {
    #[zbus(error)]
    ZBus(zbus::Error),
    /// Another client changed the profile first; reload and retry.
    StaleRevision(String),
    /// The profile or argument is not valid.
    Invalid(String),
    NotFound(String),
    /// The daemon is shutting down or busy.
    Unavailable(String),
}

type Result<T> = std::result::Result<T, DeckError>;

pub struct DeckService {
    tx: mpsc::Sender<DeckMsg>,
    store: Store,
    revisions: Mutex<HashMap<Uuid, u64>>,
    /// The window in front, for the app's "add the app in front" button.
    focused: Mutex<ulanzi_core::autoswitch::Window>,
    plugins: Option<PluginHost>,
    /// Where plugins are installed, to know their secret settings before the host is up.
    plugins_dir: Option<PathBuf>,
    /// Held while settings are read, changed and saved, so two writers cannot lose each other's change.
    settings_lock: tokio::sync::Mutex<()>,
}

impl DeckService {
    pub fn new(tx: mpsc::Sender<DeckMsg>, store: Store) -> DeckService {
        DeckService {
            tx,
            store,
            revisions: Mutex::default(),
            focused: Mutex::default(),
            plugins: None,
            plugins_dir: None,
            settings_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Give the service the plugin host.
    pub fn with_plugins(mut self, host: PluginHost) -> DeckService {
        self.set_plugins(host);
        self
    }

    /// Give the already published service the plugin host (it comes up in the background, after
    /// the deck has started).
    pub fn set_plugins(&mut self, host: PluginHost) {
        self.plugins = Some(host);
    }

    /// The folder plugins are installed in (read for `plugin.toml` files before the host is up).
    pub fn with_plugins_dir(mut self, dir: PathBuf) -> DeckService {
        self.plugins_dir = Some(dir);
        self
    }

    /// Secret setting keys by plugin id: from the `plugin.toml` files on disk (unparsable ones
    /// skipped), merged with the host's manifests once it is ready.
    fn secret_fields(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        if let Some(dir) = &self.plugins_dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for e in entries.flatten() {
                let Ok(text) = std::fs::read_to_string(e.path().join("plugin.toml")) else {
                    continue;
                };
                let Ok(m) = ulanzi_core::plugin::Manifest::parse(&text) else {
                    continue;
                };
                let keys: BTreeSet<String> = m.secret_keys().into_iter().map(String::from).collect();
                out.entry(m.id).or_default().extend(keys);
            }
        }
        if let Some(host) = &self.plugins {
            for (id, m) in host.manifests() {
                let keys: BTreeSet<String> = m.secret_keys().into_iter().map(String::from).collect();
                out.entry(id).or_default().extend(keys);
            }
        }
        out
    }

    /// Set or clear a plugin secret: the copy in `settings.toml` goes first, then the store.
    async fn write_secret(
        &self,
        id: String,
        key: String,
        value: Option<String>,
        emitter: &SignalEmitter<'_>,
    ) -> Result<()> {
        let host = self.plugins()?;
        let _guard = self.settings_lock.lock().await;
        // Only a plugin and key the host knows touch settings.toml; anything else gets the host's
        // own error and leaves the old copy alone.
        let declared = host
            .manifests()
            .get(&id)
            .is_some_and(|m| m.secret_keys().contains(&key.as_str()));
        let mut changed = None;
        if declared {
            let mut s = self
                .store
                .load_settings()
                .map_err(|e| DeckError::Unavailable(e.to_string()))?;
            if s.plugins
                .get_mut(&id)
                .and_then(|e| e.settings.remove(&key))
                .is_some()
            {
                self.store
                    .save_settings(&s)
                    .map_err(|e| DeckError::Unavailable(e.to_string()))?;
                host.apply_settings(&s).await;
                changed = Some(s);
            }
        }
        let result = host.set_secret(&id, &key, value).await.map_err(plugin_err);
        if let Some(s) = changed {
            self.send(DeckMsg::Settings(Box::new(s))).await?;
            let _ = Self::settings_changed(emitter).await;
        }
        result
    }

    fn plugins(&self) -> Result<&PluginHost> {
        self.plugins
            .as_ref()
            .ok_or_else(|| DeckError::Unavailable("plugins are not running yet".into()))
    }

    /// After an install: move the new plugin's secrets out of settings.toml (see
    /// [`PluginHost::adopt_secrets`]) and hand it its settings.
    async fn adopt_secrets(&self) {
        let Ok(host) = self.plugins() else { return };
        let _guard = self.settings_lock.lock().await;
        let Ok(mut s) = self.store.load_settings() else {
            return;
        };
        if host.adopt_secrets(&mut s).await {
            if let Err(e) = self.store.save_settings(&s) {
                tracing::warn!("cannot save the settings without the moved secrets: {e}");
            }
            host.apply_settings(&s).await;
        }
    }

    async fn send(&self, m: DeckMsg) -> Result<()> {
        self.tx
            .send(m)
            .await
            .map_err(|_| DeckError::Unavailable("the deck has stopped".into()))
    }

    async fn snapshot(&self) -> Result<DeckSnapshot> {
        let (tx, rx) = oneshot::channel();
        self.send(DeckMsg::Query(tx)).await?;
        rx.await
            .map_err(|_| DeckError::Unavailable("the deck has stopped".into()))
    }

    fn revision(&self, id: Uuid) -> u64 {
        self.revisions
            .lock()
            .map(|r| r.get(&id).copied().unwrap_or(1))
            .unwrap_or(1)
    }

    /// Every profile's revision moves on, so an open editor reloads instead of saving over a change.
    fn bump(&self, ids: &[Uuid]) {
        if let Ok(mut r) = self.revisions.lock() {
            for id in ids {
                let next = r.get(id).copied().unwrap_or(1) + 1;
                r.insert(*id, next);
            }
        }
    }

    /// Tell the deck about profiles that appeared, changed or went away on disk.
    async fn resync(&self) -> Result<()> {
        let on_disk: Vec<Uuid> = self
            .store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(p.id),
                _ => None,
            })
            .collect();
        reload(&self.tx, &self.store).await;
        for (id, _) in self.snapshot().await?.profiles {
            if !on_disk.contains(&id) {
                self.send(DeckMsg::RemoveProfile(id)).await?;
            }
        }
        Ok(())
    }

    fn snap(&self, reason: &str) -> Result<String> {
        ulanzi_core::snapshot::take(&self.store, reason)
            .map(|s| s.name)
            .map_err(|e| DeckError::Unavailable(format!("could not back up first: {e}")))
    }

    fn read_bundle(path: &str) -> Result<ulanzi_core::bundle::Bundle> {
        let bytes = std::fs::read(path).map_err(|e| DeckError::NotFound(format!("{path}: {e}")))?;
        ulanzi_core::bundle::read(&bytes).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    fn id(s: &str) -> Result<Uuid> {
        Uuid::parse_str(s).map_err(|_| DeckError::Invalid(format!("{s:?} is not a profile id")))
    }
}

fn is_placeholder(v: Option<&toml::Value>) -> bool {
    v.and_then(toml::Value::as_str) == Some(PASSWORD_PLACEHOLDER)
}

/// A plugin file path must be absolute: the daemon's working directory means nothing to callers.
fn absolute_file(path: &str) -> Result<()> {
    if std::path::Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(DeckError::Invalid(format!(
            "the plugin file must be an absolute path, not \"{path}\""
        )))
    }
}

/// A plugin error as a D-Bus error; the message is the sentence shown to the user.
fn plugin_err(e: PluginError) -> DeckError {
    let msg = e.to_string();
    match e {
        PluginError::NotInstalled(_) => DeckError::NotFound(msg),
        PluginError::Manifest(_)
        | PluginError::Load(_)
        | PluginError::Permission(_)
        | PluginError::Invalid(_) => DeckError::Invalid(msg),
        PluginError::Off(_) | PluginError::Failed(_) | PluginError::Action(_) | PluginError::Io(_) => {
            DeckError::Unavailable(msg)
        }
    }
}

/// Re-read settings and profiles from disk and hand them to the deck.
pub async fn reload(tx: &mpsc::Sender<DeckMsg>, store: &Store) {
    match store.load_settings() {
        Ok(s) => {
            let _ = tx.send(DeckMsg::Settings(Box::new(s))).await;
        }
        Err(e) => tracing::error!("reload: {e}"),
    }
    for l in store.list_profiles() {
        if let ProfileLoad::Ok(p) = l {
            let _ = tx.send(DeckMsg::Apply { profile: p }).await;
        }
    }
}

#[interface(name = "io.github.tyaaa_aa.UlanziDeck1")]
impl DeckService {
    /// Handshake: API version and current state.
    async fn hello(
        &self,
        client_version: String,
    ) -> Result<(u32, String, String, String, String, String, u32)> {
        tracing::debug!("hello from client {client_version}");
        let s = self.snapshot().await?;
        Ok((
            API_VERSION,
            env!("CARGO_PKG_VERSION").to_string(),
            s.device.code().to_string(),
            s.device.detail(),
            format!("{:?}", s.model),
            s.profile.to_string(),
            s.page as u32,
        ))
    }

    /// The firmware version the deck reported ("" until it has answered).
    async fn firmware(&self) -> Result<String> {
        Ok(self.snapshot().await?.firmware.unwrap_or_default())
    }

    async fn list_profiles(&self) -> Result<Vec<(String, String)>> {
        Ok(self
            .snapshot()
            .await?
            .profiles
            .into_iter()
            .map(|(id, n)| (id.to_string(), n))
            .collect())
    }

    async fn get_profile(&self, id: String) -> Result<String> {
        let p = self
            .store
            .load_profile(Self::id(&id)?)
            .map_err(|e| DeckError::NotFound(e.to_string()))?;
        toml::to_string_pretty(&p).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Replace a profile. `revision` must be the one the client last saw (1 for a fresh daemon).
    async fn apply_profile(&self, toml: String, revision: u64) -> Result<u64> {
        let profile = parse_profile(&toml).map_err(|e| DeckError::Invalid(e.to_string()))?;
        let current = self.revision(profile.id);
        if revision != current {
            return Err(DeckError::StaleRevision(format!(
                "profile is at revision {current}, not {revision}"
            )));
        }
        self.store
            .save_profile(&profile)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        let next = current + 1;
        if let Ok(mut r) = self.revisions.lock() {
            r.insert(profile.id, next);
        }
        self.send(DeckMsg::Apply {
            profile: Box::new(profile),
        })
        .await?;
        Ok(next)
    }

    async fn switch_profile(&self, id: String) -> Result<()> {
        self.send(DeckMsg::SwitchProfile(Self::id(&id)?)).await
    }

    async fn set_page(&self, index: u32) -> Result<()> {
        self.send(DeckMsg::SetPage(index as usize)).await
    }

    async fn next_page(&self) -> Result<()> {
        self.send(DeckMsg::NextPage).await
    }

    async fn prev_page(&self) -> Result<()> {
        self.send(DeckMsg::PrevPage).await
    }

    async fn set_brightness(&self, percent: u8) -> Result<()> {
        if percent > 100 {
            return Err(DeckError::Invalid("brightness is 0-100".into()));
        }
        self.send(DeckMsg::Brightness(BrightnessOp::Set { percent }))
            .await
    }

    async fn reload(&self) -> Result<()> {
        reload(&self.tx, &self.store).await;
        if let (Some(host), Ok(s)) = (&self.plugins, self.store.load_settings()) {
            host.apply_settings(&s).await;
        }
        Ok(())
    }

    /// Called by the desktop's window manager script when another window comes to the front.
    async fn window_activated(&self, class: String, desktop_id: String, title: String) -> Result<()> {
        let w = ulanzi_core::autoswitch::Window {
            class,
            desktop_id,
            title,
        };
        if let Ok(mut f) = self.focused.lock() {
            *f = w.clone();
        }
        crate::focus::mark_working();
        self.send(DeckMsg::Focus(w)).await
    }

    /// How auto-switch learns the app in front: (backend, state, detail), state being active,
    /// waiting or unavailable.
    async fn focus_status(&self) -> Result<(String, String, String)> {
        let s = crate::focus::status();
        Ok((s.backend, s.state, s.detail))
    }

    /// The window in front: (class, desktop id, title).
    async fn focused_window(&self) -> Result<(String, String, String)> {
        let w = self.focused.lock().map(|f| f.clone()).unwrap_or_default();
        Ok((w.class, w.desktop_id, w.title))
    }

    /// Check a `.ulanzideck` file: its manifest (TOML) and the names of profiles that already exist.
    async fn preview_bundle(&self, path: String) -> Result<(String, Vec<String>)> {
        let b = Self::read_bundle(&path)?;
        let have: Vec<Uuid> = self
            .store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(p.id),
                _ => None,
            })
            .collect();
        let conflicts = b
            .profiles
            .iter()
            .filter(|p| have.contains(&p.profile.id))
            .map(|p| p.profile.name.clone())
            .collect();
        let manifest = toml::to_string_pretty(&b.manifest).map_err(|e| DeckError::Invalid(e.to_string()))?;
        Ok((manifest, conflicts))
    }

    /// The shell commands a `.ulanzideck` file's keys and widgets would run, to show before importing.
    async fn bundle_commands(&self, path: String) -> Result<Vec<String>> {
        Ok(ulanzi_core::bundle::commands(&Self::read_bundle(&path)?))
    }

    /// Import a `.ulanzideck` file. `conflict` is keep_both, replace or skip. Returns the imported
    /// profile ids, warnings, and the snapshot taken first (restore it to undo).
    async fn import_bundle(
        &self,
        path: String,
        conflict: String,
        keep_rules: bool,
        settings: bool,
        switch_to: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(Vec<String>, Vec<String>, String)> {
        use ulanzi_core::bundle::{Conflict, ImportOptions};
        let b = Self::read_bundle(&path)?;
        let conflict = match conflict.as_str() {
            "replace" => Conflict::Replace,
            "skip" => Conflict::Skip,
            _ => Conflict::KeepBoth,
        };
        let undo = self.snap("before-import")?;
        let device = self.snapshot().await.ok().map(|s| s.model);
        let r = ulanzi_core::bundle::import(
            &self.store,
            &b,
            &ImportOptions {
                conflict,
                keep_rules,
                settings,
                device,
            },
        )
        .map_err(|e| DeckError::Invalid(e.to_string()))?;
        let touched: Vec<Uuid> = r.imported.iter().chain(&r.replaced).copied().collect();
        self.bump(&touched);
        self.resync().await?;
        if switch_to && let Some(id) = touched.first() {
            self.send(DeckMsg::SwitchProfile(*id)).await?;
        }
        let _ = Self::profiles_changed(&emitter).await;
        Ok((touched.iter().map(Uuid::to_string).collect(), r.warnings, undo))
    }

    /// Write profiles (all when `ids` is empty) to a `.ulanzideck` file.
    async fn export_bundle(&self, path: String, ids: Vec<String>, include_settings: bool) -> Result<()> {
        let ids: Vec<Uuid> = ids.iter().map(|i| Self::id(i)).collect::<Result<_>>()?;
        let bytes =
            ulanzi_core::bundle::export_with(&self.store, &ids, include_settings, &self.secret_fields())
                .map_err(|e| DeckError::Invalid(e.to_string()))?;
        ulanzi_core::store::write_atomic(std::path::Path::new(&path), &bytes)
            .map_err(|e| DeckError::Unavailable(format!("{path}: {e}")))
    }

    /// Backups, newest first: (name, created, reason, profile names).
    async fn list_snapshots(&self) -> Result<Vec<(String, String, String, Vec<String>)>> {
        Ok(ulanzi_core::snapshot::list(&self.store)
            .into_iter()
            .map(|s| (s.name, s.created, s.reason, s.profiles))
            .collect())
    }

    async fn take_snapshot(&self, reason: String) -> Result<String> {
        self.snap(&reason)
    }

    /// Put everything back as in snapshot `name`. Returns the snapshot taken just before (to undo).
    async fn restore_snapshot(
        &self,
        name: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String> {
        let before = ulanzi_core::snapshot::restore(&self.store, &name)
            .map_err(|e| DeckError::Invalid(e.to_string()))?;
        let ids: Vec<Uuid> = self
            .store
            .list_profiles()
            .into_iter()
            .filter_map(|l| match l {
                ProfileLoad::Ok(p) => Some(p.id),
                _ => None,
            })
            .collect();
        self.bump(&ids);
        self.resync().await?;
        let _ = Self::profiles_changed(&emitter).await;
        Ok(before.name)
    }

    /// Import the old Python config. Returns (profile id, keys, warnings).
    async fn migrate(&self, force: bool) -> Result<(String, u32, Vec<String>)> {
        let path = ulanzi_core::migrate::legacy_config_path()
            .filter(|p| p.is_file())
            .ok_or_else(|| {
                DeckError::NotFound("no old configuration at ~/.config/ulanzi/config.yaml".into())
            })?;
        self.snap("before-migration")?;
        let r = ulanzi_core::migrate::migrate_legacy(&path, &self.store, force)
            .map_err(|e| DeckError::Invalid(e.to_string()))?;
        reload(&self.tx, &self.store).await;
        let _ = self.tx.send(DeckMsg::SwitchProfile(r.profile_id)).await;
        Ok((r.profile_id.to_string(), r.keys as u32, r.warnings))
    }

    /// Current settings as TOML; plugin secrets are replaced by a placeholder.
    async fn get_settings(&self) -> Result<String> {
        let mut s = self
            .store
            .load_settings()
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        let fields = self.secret_fields();
        for (id, entry) in &mut s.plugins {
            for (k, v) in &mut entry.settings {
                if is_plugin_secret(&fields, id, k) {
                    *v = toml::Value::String(PASSWORD_PLACEHOLDER.into());
                }
            }
        }
        toml::to_string_pretty(&s).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Replace the settings. A password equal to the placeholder keeps the stored one.
    async fn apply_settings(&self, toml_text: String) -> Result<()> {
        let mut s: Settings = toml::from_str(&toml_text).map_err(|e| DeckError::Invalid(e.to_string()))?;
        let _guard = self.settings_lock.lock().await;
        let fields = self.secret_fields();
        if s.plugins.iter().any(|(id, e)| {
            e.settings
                .iter()
                .any(|(k, v)| is_placeholder(Some(v)) && is_plugin_secret(&fields, id, k))
        }) {
            let old = self
                .store
                .load_settings()
                .map_err(|e| DeckError::Unavailable(e.to_string()))?;
            for (id, entry) in &mut s.plugins {
                let masked: Vec<String> = entry
                    .settings
                    .iter()
                    .filter(|(k, v)| is_placeholder(Some(v)) && is_plugin_secret(&fields, id, k))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in masked {
                    match old.plugins.get(id).and_then(|o| o.settings.get(&k)) {
                        Some(v) => entry.settings.insert(k, v.clone()),
                        None => entry.settings.remove(&k),
                    };
                }
            }
        }
        self.store
            .save_settings(&s)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        if let Some(host) = &self.plugins {
            host.apply_settings(&s).await;
        }
        self.send(DeckMsg::Settings(Box::new(s))).await
    }

    /// Installed plugins (and plugin folders that could not be loaded):
    /// (id, name, version, status id, detail). Status ids: off, ready, starting, running, problem,
    /// failed, incompatible; the detail is the reason or message for the last three.
    async fn plugin_list(&self) -> Result<Vec<(String, String, String, String, String)>> {
        Ok(self
            .plugins()?
            .list()
            .into_iter()
            .map(|p| {
                (
                    p.id,
                    p.name,
                    p.version,
                    p.status.id().to_string(),
                    p.status.detail(),
                )
            })
            .collect())
    }

    /// Every installed plugin's manifest as JSON: `{ "<id>": Manifest }`.
    async fn plugin_manifests(&self) -> Result<String> {
        serde_json::to_string(&self.plugins()?.manifests()).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Check the plugin folder at `path` (absolute) like an install would, writing nothing; JSON
    /// `Inspected`. Works without Developer mode.
    async fn plugin_inspect_dir(&self, path: String) -> Result<String> {
        if !std::path::Path::new(&path).is_absolute() {
            return Err(DeckError::Invalid(format!(
                "the plugin folder must be an absolute path, not \"{path}\""
            )));
        }
        let i = self
            .plugins()?
            .inspect_dir(std::path::Path::new(&path))
            .await
            .map_err(plugin_err)?;
        serde_json::to_string(&i).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Check the `.ulanziplugin` file at `path` (absolute) like an install would, writing nothing
    /// that stays; JSON `Inspected`. Works without Developer mode.
    async fn plugin_inspect_file(&self, path: String) -> Result<String> {
        absolute_file(&path)?;
        let i = self
            .plugins()?
            .inspect_file(std::path::Path::new(&path))
            .await
            .map_err(plugin_err)?;
        serde_json::to_string(&i).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// The entries of the plugin's picker `picker` (declared in its `[[pickers]]`) matching `query`
    /// (`""`: all), only those in `group` unless it is `""`, as a JSON array of `PickItem`. Starts
    /// the plugin if needed. Errors: NotFound (not installed), Invalid (undeclared picker),
    /// Unavailable (the plugin's own message, for example "not connected", or it is off or busy).
    async fn plugin_list_items(
        &self,
        id: String,
        picker: String,
        query: String,
        group: String,
    ) -> Result<String> {
        let items = self
            .plugins()?
            .list_items(&id, &picker, &query, &group)
            .await
            .map_err(plugin_err)?;
        serde_json::to_string(&items).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// The declared secret keys of the plugin that are set (never their values).
    async fn plugin_secret_keys(&self, id: String) -> Result<Vec<String>> {
        self.plugins()?.secret_keys(&id).await.map_err(plugin_err)
    }

    /// Set a declared secret of the plugin; the copy in `settings.toml`, if any, is removed.
    async fn plugin_set_secret(
        &self,
        id: String,
        key: String,
        value: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        if value.is_empty() {
            return Err(DeckError::Invalid("Enter a value, or use Clear".into()));
        }
        self.write_secret(id, key, Some(value), &emitter).await
    }

    /// Clear a declared secret of the plugin (also the copy in `settings.toml`).
    async fn plugin_clear_secret(
        &self,
        id: String,
        key: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        self.write_secret(id, key, None, &emitter).await
    }

    /// Install (or replace) the plugin in the folder at `path` (absolute); returns its id. Only in
    /// Developer mode.
    async fn plugin_install_dir(&self, path: String) -> Result<String> {
        if !std::path::Path::new(&path).is_absolute() {
            return Err(DeckError::Invalid(format!(
                "the plugin folder must be an absolute path, not \"{path}\""
            )));
        }
        let developer_mode = self
            .store
            .load_settings()
            .map_err(|e| DeckError::Unavailable(e.to_string()))?
            .developer_mode;
        let id = self
            .plugins()?
            .install_dir(std::path::Path::new(&path), developer_mode)
            .await
            .map_err(plugin_err)?;
        self.adopt_secrets().await;
        Ok(id)
    }

    /// Install (or replace) the plugin in the `.ulanziplugin` file at `path` (absolute); returns
    /// its id. Signed packages (a valid `<path>.minisig` next to it) install any time; unsigned ones
    /// only in Developer mode.
    async fn plugin_install_file(&self, path: String) -> Result<String> {
        absolute_file(&path)?;
        let developer_mode = self
            .store
            .load_settings()
            .map_err(|e| DeckError::Unavailable(e.to_string()))?
            .developer_mode;
        let id = self
            .plugins()?
            .install_file(std::path::Path::new(&path), developer_mode)
            .await
            .map_err(plugin_err)?;
        self.adopt_secrets().await;
        Ok(id)
    }

    /// The plugin store's signed list as JSON (`{ "index": …, "offline": bool, "icons": … }`); a
    /// copy under 30 minutes old unless `refresh`.
    async fn plugin_store_index(&self, refresh: bool) -> Result<String> {
        let listing = self.plugins()?.store_index(refresh).await.map_err(plugin_err)?;
        serde_json::to_string(&listing).map_err(|e| DeckError::Invalid(e.to_string()))
    }

    /// Install plugin `id` from the store (signed: no Developer mode needed); returns its id. Sends
    /// `PluginInstallProgress` as it goes.
    async fn plugin_store_install(
        &self,
        id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String> {
        let host = self.plugins()?;
        let (tx, mut rx) = mpsc::unbounded_channel::<crate::plugins::InstallStage>();
        // At most ten download updates a second; every change of stage.
        let last = std::sync::Mutex::new((std::time::Instant::now(), ""));
        let progress: crate::plugins::Progress = std::sync::Arc::new(move |stage| {
            let Ok(mut last) = last.lock() else { return };
            let now = std::time::Instant::now();
            if last.1 == stage.id() && now.duration_since(last.0) < std::time::Duration::from_millis(100) {
                return;
            }
            *last = (now, stage.id());
            let _ = tx.send(stage);
        });
        let install = host.store_install(&id, progress);
        tokio::pin!(install);
        let installed = loop {
            tokio::select! {
                r = &mut install => break r,
                Some(stage) = rx.recv() => {
                    let (done, total) = match stage {
                        crate::plugins::InstallStage::Downloading { done, total } => (done, total),
                        _ => (0, 0),
                    };
                    let _ = DeckService::plugin_install_progress(&emitter, &id, stage.id(), done, total).await;
                }
            }
        };
        let id = installed.map_err(plugin_err)?;
        self.adopt_secrets().await;
        Ok(id)
    }

    /// Installed plugins the plugin store lists as official (made by the Ulanzi Deck project).
    async fn plugin_official(&self) -> Result<Vec<String>> {
        Ok(self.plugins()?.official())
    }

    /// Installed plugins whose package was signed with a key the app trusts.
    async fn plugin_verified(&self) -> Result<Vec<String>> {
        Ok(self.plugins()?.verified())
    }

    /// Stop the plugin and delete it and its secrets. Its settings are kept.
    async fn plugin_remove(&self, id: String) -> Result<()> {
        self.plugins()?.remove(&id).await.map_err(plugin_err)
    }

    /// Turn a plugin on or off (`[plugins.<id>].enabled` in the settings).
    async fn plugin_set_enabled(
        &self,
        id: String,
        enabled: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        let host = self.plugins()?;
        if !host.list().iter().any(|p| p.id == id) {
            return Err(plugin_err(PluginError::NotInstalled(id)));
        }
        let _guard = self.settings_lock.lock().await;
        let mut s = self
            .store
            .load_settings()
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        s.plugins.entry(id).or_default().enabled = enabled;
        self.store
            .save_settings(&s)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        host.apply_settings(&s).await;
        self.send(DeckMsg::Settings(Box::new(s))).await?;
        let _ = Self::settings_changed(&emitter).await;
        Ok(())
    }

    /// Stop and start a plugin again, forgetting earlier crashes.
    async fn plugin_restart(&self, id: String) -> Result<()> {
        self.plugins()?.restart(&id).await.map_err(plugin_err)
    }

    /// The plugin list or a plugin's status changed: call PluginList again.
    #[zbus(signal)]
    async fn plugins_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// Towns matching `query` for the weather widget: (label, latitude, longitude).
    async fn weather_places(&self, query: String) -> Result<Vec<(String, f64, f64)>> {
        let found = tokio::task::spawn_blocking(move || crate::widgets::weather::search(&query))
            .await
            .map_err(|e| DeckError::Unavailable(e.to_string()))?
            .map_err(DeckError::Unavailable)?;
        Ok(found.into_iter().map(|p| (p.label(), p.lat, p.lon)).collect())
    }

    /// The weather now, for the app's preview: (ok, temperature, WMO code, high, low, place or error).
    async fn weather_now(
        &self,
        place: String,
        lat: f64,
        lon: f64,
        fahrenheit: bool,
    ) -> (bool, f64, u8, f64, f64, String) {
        use ulanzi_core::render::screen::WeatherData;
        let w = tokio::task::spawn_blocking(move || {
            if lat.is_finite() && lon.is_finite() && !(lat == 0.0 && lon == 0.0) {
                crate::widgets::weather::fetch_at(&place, lat, lon, fahrenheit)
            } else {
                crate::widgets::weather::fetch(&place, fahrenheit)
            }
        })
        .await
        .unwrap_or_else(|e| WeatherData::Err(e.to_string()));
        match w {
            WeatherData::Ok {
                temp,
                code,
                place,
                high,
                low,
            } => (true, temp as f64, code, high as f64, low as f64, place),
            WeatherData::Err(e) => (false, 0.0, 0, 0.0, 0.0, e),
        }
    }

    /// What is playing: (anything, title, artist, album-art file, position s, length s, playing).
    /// Position and length are 0 when the player doesn't say.
    async fn now_playing(&self) -> (bool, String, String, String, u32, u32, bool) {
        let m = crate::screen::LAST_MEDIA.lock().ok().and_then(|m| m.clone());
        match m {
            Some(m) => (
                true,
                m.title,
                m.artist,
                m.art.map(|p| p.display().to_string()).unwrap_or_default(),
                m.position_s.unwrap_or(0),
                m.length_s.unwrap_or(0),
                m.playing,
            ),
            None => (false, String::new(), String::new(), String::new(), 0, 0, false),
        }
    }

    /// Create an empty profile and return its id.
    async fn create_profile(
        &self,
        name: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(DeckError::Invalid("a profile needs a name".into()));
        }
        let model = self.snapshot().await.map(|s| s.model).unwrap_or(Model::D200X);
        let p = Profile::new_default(name, model);
        self.store
            .save_profile(&p)
            .map_err(|e| DeckError::Unavailable(e.to_string()))?;
        let id = p.id.to_string();
        self.send(DeckMsg::Apply { profile: Box::new(p) }).await?;
        let _ = Self::profiles_changed(&emitter).await;
        Ok(id)
    }

    /// Delete a profile (its files are moved aside). The last profile cannot be deleted.
    async fn delete_profile(
        &self,
        id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<()> {
        let id = Self::id(&id)?;
        if self.snapshot().await?.profiles.len() <= 1 {
            return Err(DeckError::Invalid("the last profile cannot be deleted".into()));
        }
        self.snap("before-delete")?;
        self.store
            .delete_profile(id)
            .map_err(|e| DeckError::NotFound(e.to_string()))?;
        self.send(DeckMsg::RemoveProfile(id)).await?;
        let _ = Self::profiles_changed(&emitter).await;
        Ok(())
    }

    /// The revision a client must pass to ApplyProfile for this profile.
    async fn profile_revision(&self, id: String) -> Result<u64> {
        Ok(self.revision(Self::id(&id)?))
    }

    #[zbus(signal)]
    async fn profiles_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_state_changed(e: &SignalEmitter<'_>, state: &str, detail: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn input_event(e: &SignalEmitter<'_>, control: &str, kind: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn page_changed(e: &SignalEmitter<'_>, profile: &str, page: u32) -> zbus::Result<()>;

    /// Settings changed outside the app (tray, deck keys): read them again.
    #[zbus(signal)]
    async fn settings_changed(e: &SignalEmitter<'_>) -> zbus::Result<()>;

    /// How far `PluginStoreInstall` of `id` is: `stage` is preparing, downloading (`done` of
    /// `total` bytes), checking or installing.
    #[zbus(signal)]
    async fn plugin_install_progress(
        e: &SignalEmitter<'_>,
        id: &str,
        stage: &str,
        done: u64,
        total: u64,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_failed(e: &SignalEmitter<'_>, control: &str, message: &str) -> zbus::Result<()>;
}

fn kind_name(k: InputKind) -> &'static str {
    match k {
        InputKind::Press => "press",
        InputKind::Release => "release",
        InputKind::TurnLeft => "left",
        InputKind::TurnRight => "right",
    }
}

/// Send `PluginsChanged` now (the host has just come up) and whenever the plugin list or a status
/// changes, at most once per `PLUGINS_CHANGED_EVERY` (changes meanwhile are folded into the next
/// signal).
pub async fn forward_plugins(conn: Connection, host: PluginHost) {
    const PLUGINS_CHANGED_EVERY: std::time::Duration = std::time::Duration::from_millis(200);
    let Ok(emitter) = SignalEmitter::new(&conn, OBJECT_PATH) else {
        return;
    };
    let mut changes = host.watch();
    changes.mark_changed();
    while changes.changed().await.is_ok() {
        changes.borrow_and_update();
        // Keys draw their plugin action's live looks (see `catalog::plugin_states`).
        ulanzi_core::catalog::set_plugin_looks(host.manifests().values());
        if let Err(e) = DeckService::plugins_changed(&emitter).await {
            tracing::debug!("signal not sent: {e}");
        }
        // A plugin flipping its status quickly cannot flood the bus.
        tokio::time::sleep(PLUGINS_CHANGED_EVERY).await;
    }
}

/// Turn deck events into D-Bus signals until the deck stops.
pub async fn forward_signals(conn: Connection, mut events: broadcast::Receiver<DeckEvent>) {
    let emitter = match SignalEmitter::new(&conn, OBJECT_PATH) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("cannot emit signals: {e}");
            return;
        }
    };
    loop {
        let ev = match events.recv().await {
            Ok(ev) => ev,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("dropped {n} deck events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };
        let r = match ev {
            DeckEvent::State(s) => DeckService::device_state_changed(&emitter, s.code(), &s.detail()).await,
            DeckEvent::Input(i) => {
                DeckService::input_event(&emitter, &i.control.as_str(), kind_name(i.kind)).await
            }
            DeckEvent::Page { profile, page } => {
                DeckService::page_changed(&emitter, &profile.to_string(), page as u32).await
            }
            DeckEvent::ActionFailed { control, message } => {
                DeckService::action_failed(&emitter, &control, &message).await
            }
            DeckEvent::SettingsChanged => DeckService::settings_changed(&emitter).await,
        };
        if let Err(e) = r {
            tracing::debug!("signal not sent: {e}");
        }
    }
}
