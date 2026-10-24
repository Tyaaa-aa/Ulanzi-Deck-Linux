//! The plugin host's state: which plugins are installed, their status and running instances.
//!
//! State sits behind a std `Mutex` that is only held for short, non-blocking sections (never
//! across an await, a guest call or an action reply). Lifecycle operations (settings, install,
//! remove, restart, shutdown) are also serialized by the async `ops` lock so they cannot
//! interleave. Each instance has a supervisor task that forwards its outputs and handles its end.
//! Messages to the deck go through a coalescing queue and one forwarding task (`DeckQueue`), so
//! they keep their order and nothing here ever waits on a full deck channel.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ulanzi_core::model::{PluginEntry, Settings, table_to_json};
use ulanzi_core::plugin::{CONTRACT, Manifest, ManifestError, PickItem};

use super::deck_queue::DeckQueue;
use super::instance::truncate;
use super::ulanzi::plugin::types::Status;
use super::{
    Dropped, Event, Exit, InstanceHandle, LoadedPlugin, Output, PluginEngine, PluginError, PluginInfo,
    PluginStatus, SecretStore, spawn_instance,
};
use crate::deck::DeckMsg;

/// A second crash within this long leaves the plugin failed.
const CRASH_WINDOW: Duration = Duration::from_secs(5 * 60);
/// An action waits this long for a starting plugin.
const START_WAIT: Duration = Duration::from_secs(2);
/// Stopping waits this long for an instance to end.
const STOP_WAIT: Duration = Duration::from_secs(5);
/// An action waits this long for its reply once queued: the 500 ms call budget plus a generous
/// margin (a call never outlasts its budget, blocking waits included).
const REPLY_WAIT: Duration = Duration::from_secs(2);
/// Longest `list-items` query or group passed to a plugin, in bytes; longer ones are cut.
const MAX_QUERY: usize = 1024;

pub(super) struct Host {
    pub(super) dir: PathBuf,
    /// The engine, or why it could not be created.
    pub(super) engine: Result<PluginEngine, String>,
    pub(super) secrets: Arc<dyn SecretStore>,
    deck: DeckQueue,
    rt: Handle,
    state: Mutex<State>,
    changed: watch::Sender<u64>,
    /// Serializes lifecycle operations.
    pub(super) ops: tokio::sync::Mutex<()>,
    /// Start attempts, unique for the host's lifetime.
    next_attempt: AtomicU64,
    me: Weak<Host>,
}

pub(super) struct State {
    plugins: BTreeMap<String, Entry>,
    /// The latest `[plugins.<id>]` tables, also for plugins not installed (yet).
    settings: BTreeMap<String, PluginEntry>,
    closed: bool,
}

struct Entry {
    name: String,
    version: String,
    status: PluginStatus,
    /// `None` for incompatible folders.
    loaded: Option<Arc<LoadedPlugin>>,
    enabled: bool,
    settings: toml::Table,
    /// Start without waiting for an action (used by a profile, background, or just installed).
    wanted: bool,
    /// The current start attempt (its instance carries the same number); a stop resets it,
    /// which cancels a start in progress.
    attempt: u64,
    run: Option<Run>,
    /// The instance last detached by a stop: its final outputs (secrets) still count while it
    /// ends. A late one, from an instance that outlived its stop, is ignored.
    draining: Option<u64>,
    last_crash: Option<Instant>,
}

/// A running instance.
pub(super) struct Run {
    attempt: u64,
    handle: Arc<InstanceHandle>,
    supervisor: JoinHandle<()>,
}

/// A plugin folder that cannot be loaded: what to list it as, and why.
pub(super) struct Bad {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) reason: String,
}

fn enabled_and_settings(settings: &BTreeMap<String, PluginEntry>, id: &str) -> (bool, toml::Table) {
    settings
        .get(id)
        .map_or((true, toml::Table::new()), |e| (e.enabled, e.settings.clone()))
}

/// Read and check `<folder>/plugin.toml`. A contract this host does not support is reported as
/// `<name> needs a newer Ulanzi Deck` even when the rest of the manifest does not parse (a future
/// contract may change its shape); `fallback` names the plugin when the manifest does not.
pub(super) fn read_manifest(folder: &Path, fallback: &str) -> Result<Manifest, Bad> {
    let text = std::fs::read_to_string(folder.join("plugin.toml")).map_err(|e| Bad {
        name: fallback.to_string(),
        version: String::new(),
        reason: format!("cannot read plugin.toml: {e}"),
    })?;
    let raw: Option<toml::Table> = text.parse().ok();
    let field = |key: &str| {
        raw.as_ref()
            .and_then(|t| t.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let name = field("name").unwrap_or_else(|| fallback.to_string());
    let version = field("version").unwrap_or_default();
    let bad = |reason: String| Bad {
        name: name.clone(),
        version: version.clone(),
        reason,
    };
    let newer = || bad(format!("{name} needs a newer Ulanzi Deck"));
    if field("contract").is_some_and(|c| unsupported_contract(&c)) {
        return Err(newer());
    }
    match Manifest::parse(&text) {
        Ok(m) => Ok(m),
        Err(ManifestError::Incompatible { .. }) => Err(newer()),
        Err(e) => Err(bad(e.to_string())),
    }
}

/// A well-formed `MAJOR.MINOR` this host cannot run.
fn unsupported_contract(c: &str) -> bool {
    let Some((major, minor)) = c.split_once('.') else {
        return false;
    };
    match (major.trim().parse::<u32>(), minor.trim().parse::<u32>()) {
        (Ok(major), Ok(minor)) => !(major == CONTRACT.0 && minor <= CONTRACT.1),
        _ => false,
    }
}

/// Load one installed plugin folder (`folder` is named after the plugin id).
fn load_folder(folder: &Path, id: &str, engine: &Result<PluginEngine, String>) -> Result<LoadedPlugin, Bad> {
    let m = read_manifest(folder, id)?;
    let bad = |reason: String| Bad {
        name: m.name.clone(),
        version: m.version.clone(),
        reason,
    };
    if m.id != id {
        return Err(bad(format!(
            "plugin id \"{}\" does not match its folder \"{id}\"",
            m.id
        )));
    }
    let engine = engine.as_ref().map_err(|e| bad(e.clone()))?;
    engine
        .load(&m, &folder.join("plugin.wasm"))
        .map_err(|e| bad(e.to_string()))
}

/// Folders in the plugins dir that are not plugins: hidden ones and unfinished installs.
pub(super) fn ignored_folder(name: &str) -> bool {
    name.starts_with('.') || name.ends_with(".new")
}

/// An install interrupted between moving `<id>` aside (as `.<id>.old`) and moving the new one
/// in: put the old folder back.
fn restore_interrupted(dir: &Path, entries: &[std::fs::DirEntry]) {
    for entry in entries {
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|n| n.strip_prefix('.'))
            .and_then(|n| n.strip_suffix(".old"))
        else {
            continue;
        };
        let target = dir.join(id);
        if id.is_empty() || std::fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        match std::fs::rename(entry.path(), &target) {
            Ok(()) => tracing::warn!(plugin = %id, "restored the plugin after an interrupted install"),
            Err(e) => {
                tracing::warn!(plugin = %id, "cannot restore the plugin after an interrupted install: {e}")
            }
        }
    }
}

/// Package folders (`.unpack-*`, see `install::unpack_and_validate`) left behind when the
/// service stopped mid-install: nothing uses them any more, delete them. Runs before any install
/// can start.
fn remove_unpack_leftovers(entries: &[std::fs::DirEntry]) {
    for entry in entries {
        let name = entry.file_name();
        if !name.to_str().is_some_and(|n| n.starts_with(".unpack-")) {
            continue;
        }
        // Never follow a link out of the plugins folder.
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        match std::fs::remove_dir_all(entry.path()) {
            Ok(()) => tracing::info!("deleted the unfinished install {}", entry.path().display()),
            Err(e) => tracing::warn!(
                "cannot delete the unfinished install {}: {e}",
                entry.path().display()
            ),
        }
    }
}

/// Sets its flag when dropped.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Every plugin folder in `dir`, loaded or with the reason it cannot be. Blocking. Stops before
/// the next plugin once `cancel` is set.
fn scan(
    dir: &Path,
    engine: &Result<PluginEngine, String>,
    cancel: &AtomicBool,
) -> Vec<(String, Result<LoadedPlugin, Bad>)> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("cannot read the plugins folder {}: {e}", dir.display());
            }
            return Vec::new();
        }
    };
    let entries: Vec<_> = entries.flatten().collect();
    restore_interrupted(dir, &entries);
    remove_unpack_leftovers(&entries);
    let entries = std::fs::read_dir(dir)
        .map(|d| d.flatten().collect())
        .unwrap_or(entries);
    let mut found = Vec::new();
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if ignored_folder(&name) || !entry.path().is_dir() {
            continue;
        }
        let loaded = load_folder(&entry.path(), &name, engine);
        if let Err(bad) = &loaded {
            tracing::warn!(plugin = %name, "cannot load the plugin: {}", bad.reason);
        }
        found.push((name, loaded));
    }
    found
}

impl Host {
    pub(super) async fn start(
        dir: PathBuf,
        cache: PathBuf,
        secrets: Arc<dyn SecretStore>,
        settings: &Settings,
        used: &BTreeSet<String>,
        deck: mpsc::Sender<DeckMsg>,
    ) -> Arc<Host> {
        let engine = PluginEngine::new(&cache).map_err(|e| {
            tracing::error!("plugins cannot run: {e}");
            e.to_string()
        });
        let scanned = {
            let (dir, engine) = (dir.clone(), engine.clone());
            // A start that is dropped (cancelled) stops compiling after the current plugin.
            // (Once the scan is done, the flag no longer matters.)
            let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
            let flag = Arc::clone(&cancel.0);
            tokio::task::spawn_blocking(move || scan(&dir, &engine, &flag))
                .await
                .unwrap_or_else(|e| {
                    tracing::error!("scanning plugins failed: {e}");
                    Vec::new()
                })
        };

        let host = Arc::new_cyclic(|me| Host {
            dir,
            engine,
            secrets,
            deck: DeckQueue::spawn(deck),
            rt: Handle::current(),
            state: Mutex::new(State {
                plugins: BTreeMap::new(),
                settings: settings.plugins.clone(),
                closed: false,
            }),
            changed: watch::Sender::new(0),
            ops: tokio::sync::Mutex::new(()),
            next_attempt: AtomicU64::new(0),
            me: me.clone(),
        });
        {
            let mut st = host.state();
            for (id, loaded) in scanned {
                let (enabled, table) = enabled_and_settings(&st.settings, &id);
                let entry = match loaded {
                    Ok(loaded) => Entry {
                        name: loaded.manifest.name.clone(),
                        version: loaded.manifest.version.clone(),
                        status: if enabled {
                            PluginStatus::Ready
                        } else {
                            PluginStatus::Off
                        },
                        wanted: used.contains(&id) || loaded.manifest.background,
                        loaded: Some(Arc::new(loaded)),
                        enabled,
                        settings: table,
                        attempt: 0,
                        run: None,
                        draining: None,
                        last_crash: None,
                    },
                    Err(bad) => Entry {
                        name: bad.name,
                        version: bad.version,
                        status: PluginStatus::Incompatible(bad.reason),
                        loaded: None,
                        enabled,
                        settings: table,
                        wanted: false,
                        attempt: 0,
                        run: None,
                        draining: None,
                        last_crash: None,
                    },
                };
                let entry = st.plugins.entry(id.clone()).or_insert(entry);
                if entry.wanted && entry.status == PluginStatus::Ready {
                    host.launch(&id, entry);
                }
            }
        }
        host.bump();
        host
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn to_deck(&self, msg: DeckMsg) {
        self.deck.send(msg);
    }

    pub(super) fn list(&self) -> Vec<PluginInfo> {
        self.state()
            .plugins
            .iter()
            .map(|(id, e)| PluginInfo {
                id: id.clone(),
                name: e.name.clone(),
                version: e.version.clone(),
                status: e.status.clone(),
            })
            .collect()
    }

    pub(super) fn manifests(&self) -> BTreeMap<String, Manifest> {
        self.state()
            .plugins
            .iter()
            .filter_map(|(id, e)| Some((id.clone(), e.loaded.as_ref()?.manifest.clone())))
            .collect()
    }

    /// The version of the installed plugin `id`, if it is listed with one.
    pub(super) fn installed_version(&self, id: &str) -> Option<String> {
        let st = self.state();
        st.plugins
            .get(id)
            .map(|e| e.version.clone())
            .filter(|v| !v.is_empty())
    }

    /// The manifest and settings table of `id`, or why it has none.
    fn manifest_of(&self, id: &str) -> Result<(Manifest, toml::Table), PluginError> {
        let st = self.state();
        let e = st
            .plugins
            .get(id)
            .ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        match &e.loaded {
            Some(loaded) => Ok((loaded.manifest.clone(), e.settings.clone())),
            None => Err(PluginError::Load(e.status.detail())),
        }
    }

    pub(super) async fn secret_keys(&self, id: &str) -> Result<Vec<String>, PluginError> {
        let (manifest, settings) = self.manifest_of(id)?;
        let stored = self
            .secrets
            .load(id)
            .await
            .map_err(|why| PluginError::Io(format!("cannot read the secrets: {why}")))?;
        Ok(manifest
            .secret_keys()
            .into_iter()
            .filter(|k| {
                stored.get(*k).is_some_and(|v| !v.is_empty())
                    || settings
                        .get(*k)
                        .and_then(toml::Value::as_str)
                        .is_some_and(|v| !v.is_empty())
            })
            .map(str::to_string)
            .collect())
    }

    pub(super) async fn set_secret(
        &self,
        id: &str,
        key: &str,
        value: Option<String>,
    ) -> Result<(), PluginError> {
        let _ops = self.ops.lock().await;
        let (run, relaunch) = {
            let mut st = self.state();
            let e = st
                .plugins
                .get_mut(id)
                .ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
            let Some(loaded) = &e.loaded else {
                return Err(PluginError::Load(e.status.detail()));
            };
            if !loaded.manifest.secret_keys().contains(&key) {
                return Err(PluginError::Invalid(format!(
                    "{} has no secret setting “{key}”",
                    e.name
                )));
            }
            let relaunch = e.enabled
                && matches!(
                    e.status,
                    PluginStatus::Starting | PluginStatus::Running | PluginStatus::Problem(_)
                );
            let run = Self::take_run(e);
            if relaunch {
                // Actions wait (briefly) for the new instance instead of starting another.
                e.status = PluginStatus::Starting;
                self.bump();
            }
            (run, relaunch)
        };
        if let Some(run) = run {
            // The instance's own last secrets are saved while it ends.
            stop_run(run).await;
        }
        let saved = async {
            let mut secrets = self
                .secrets
                .load(id)
                .await
                .map_err(|why| PluginError::Io(format!("cannot save the secret: {why}")))?;
            match value.filter(|v| !v.is_empty()) {
                Some(v) => secrets.insert(key.to_string(), v),
                None => secrets.remove(key),
            };
            self.secrets
                .save(id, &secrets)
                .await
                .map_err(|why| PluginError::Io(format!("cannot save the secret: {why}")))
        }
        .await;
        if relaunch {
            let mut st = self.state();
            if !st.closed
                && let Some(e) = st.plugins.get_mut(id)
                && e.enabled
            {
                e.last_crash = None;
                self.launch(id, e);
            }
        }
        saved
    }

    pub(super) fn watch(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Start an instance of `e` in the background: `Starting` now, `Running` once it runs.
    /// Called with the state lock held.
    fn launch(&self, id: &str, e: &mut Entry) {
        // A new run: the stopped instance has ended (or timed out), and its late outputs
        // must not overwrite what the new one does.
        e.draining = None;
        let Some(loaded) = e.loaded.clone() else {
            return;
        };
        let attempt = self.next_attempt.fetch_add(1, Ordering::Relaxed) + 1;
        e.attempt = attempt;
        e.status = PluginStatus::Starting;
        let (me, id) = (self.me.clone(), id.to_string());
        self.rt.spawn(async move {
            if let Some(host) = me.upgrade() {
                host.start_instance(id, attempt, loaded).await;
            }
        });
        self.bump();
    }

    async fn start_instance(&self, id: String, attempt: u64, loaded: Arc<LoadedPlugin>) {
        // Loading may wait for the keyring; it happens before the lock is taken.
        let secrets = self.secrets.load(&id).await;
        let mut st = self.state();
        let closed = st.closed;
        let Some(e) = st.plugins.get_mut(&id) else {
            return;
        };
        if closed || e.attempt != attempt || e.run.is_some() {
            return; // stopped, turned off or replaced meanwhile
        }
        let secrets = match secrets {
            Ok(secrets) => secrets,
            Err(why) => {
                // Starting with no secrets would overwrite the stored ones on the next change.
                tracing::warn!(target: "plugin", plugin = %id, "cannot read its secrets: {why}");
                e.status = PluginStatus::Failed(format!("cannot read its secrets: {why}"));
                self.bump();
                return;
            }
        };
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let engine = match &self.engine {
            Ok(engine) => engine,
            Err(why) => {
                e.status = PluginStatus::Failed(why.clone());
                self.bump();
                return;
            }
        };
        match spawn_instance(&loaded, engine, secrets, out_tx, self.rt.clone()) {
            Err(err) => {
                tracing::warn!(target: "plugin", plugin = %id, "cannot start: {err}");
                e.status = PluginStatus::Failed(err.to_string());
            }
            Ok(mut handle) => {
                let _ = handle.post(Event::Init {
                    settings: table_to_json(&loaded.manifest.settings_with_defaults(&e.settings)),
                });
                // The supervisor awaits the exit; the handle stays here, so dropping the entry
                // (or the host) still stops the instance.
                let exited = std::mem::replace(&mut handle.exited, oneshot::channel().1);
                let supervisor =
                    self.rt
                        .spawn(supervise(self.me.clone(), id.clone(), attempt, out_rx, exited));
                e.run = Some(Run {
                    attempt,
                    handle: Arc::new(handle),
                    supervisor,
                });
                e.status = PluginStatus::Running;
            }
        }
        self.bump();
    }

    /// Handle one output of instance `attempt` of `id`.
    async fn output(&self, id: &str, attempt: u64, output: Output) {
        match output {
            Output::State { key, value } => self.to_deck(DeckMsg::PluginState {
                plugin: id.to_string(),
                key,
                value,
            }),
            Output::Status { status, message } => {
                let mut st = self.state();
                let Some(e) = st.plugins.get_mut(id) else {
                    return;
                };
                if e.run.as_ref().is_none_or(|r| r.attempt != attempt) {
                    return;
                }
                let status = match status {
                    Status::Ok | Status::Connecting => PluginStatus::Running,
                    Status::Problem => PluginStatus::Problem(message),
                };
                if e.status != status {
                    e.status = status;
                    self.bump();
                }
            }
            Output::SecretsChanged(secrets) => {
                // A removed plugin, or an instance that outlived its stop, must not write its
                // secrets back (a late save would bring back deleted ones).
                let current = self.state().plugins.get(id).is_some_and(|e| {
                    e.run.as_ref().is_some_and(|r| r.attempt == attempt) || e.draining == Some(attempt)
                });
                if !current {
                    tracing::debug!(target: "plugin", plugin = %id, "secrets of a stopped instance ignored");
                    return;
                }
                if let Err(why) = self.secrets.save(id, &secrets).await {
                    tracing::warn!(target: "plugin", plugin = %id, "cannot save its secrets: {why}");
                }
            }
        }
    }

    /// Instance `attempt` of `id` ended. A crash restarts it once; a second crash within 5 minutes
    /// leaves it failed. Its state is cleared from the deck either way.
    fn ended(&self, id: &str, attempt: u64, exit: Exit) {
        let mut st = self.state();
        let closed = st.closed;
        if let Some(e) = st.plugins.get_mut(id)
            && !closed
            && e.run.as_ref().is_some_and(|r| r.attempt == attempt)
        {
            e.run = None;
            let why = match exit {
                Exit::Crashed(why) => why,
                Exit::Stopped => "stopped unexpectedly".to_string(),
            };
            let now = Instant::now();
            if e.last_crash.is_some_and(|t| now.duration_since(t) < CRASH_WINDOW) {
                let msg = format!("crashed again ({why}); left off until restarted");
                tracing::warn!(target: "plugin", plugin = %id, "{msg}");
                e.status = PluginStatus::Failed(why);
                self.bump();
            } else {
                tracing::warn!(target: "plugin", plugin = %id, "crashed ({why}); restarting");
                e.last_crash = Some(now);
                self.launch(id, e);
            }
        }
        // Still under the lock: a restarted instance's state cannot overtake this.
        self.to_deck(DeckMsg::PluginCleared(id.to_string()));
    }

    /// Detach `e`'s instance (and cancel a start in progress); stop it with [`stop_run`].
    fn take_run(e: &mut Entry) -> Option<Run> {
        e.attempt = 0;
        e.draining = e.run.as_ref().map(|r| r.attempt);
        e.run.take()
    }

    /// The running instance of `plugin` and its name, starting it first when it is `Ready` and
    /// waiting (up to `START_WAIT`) while it starts.
    async fn running(&self, plugin: &str) -> Result<(Arc<InstanceHandle>, String), PluginError> {
        let deadline = Instant::now() + START_WAIT;
        let mut changes = self.changed.subscribe();
        loop {
            changes.borrow_and_update();
            let name = {
                let mut st = self.state();
                let closed = st.closed;
                let Some(e) = st.plugins.get_mut(plugin) else {
                    return Err(PluginError::NotInstalled(plugin.to_string()));
                };
                match &e.status {
                    PluginStatus::Off => return Err(PluginError::Off(e.name.clone())),
                    PluginStatus::Incompatible(why) => return Err(PluginError::Load(why.clone())),
                    PluginStatus::Failed(_) => return Err(PluginError::Failed(e.name.clone())),
                    PluginStatus::Running | PluginStatus::Problem(_) => match &e.run {
                        Some(run) => return Ok((Arc::clone(&run.handle), e.name.clone())),
                        None => return Err(PluginError::Failed(e.name.clone())),
                    },
                    PluginStatus::Ready if closed => return Err(PluginError::Off(e.name.clone())),
                    PluginStatus::Ready => self.launch(plugin, e),
                    PluginStatus::Starting => {}
                }
                e.name.clone()
            };
            if !matches!(
                tokio::time::timeout_at(deadline, changes.changed()).await,
                Ok(Ok(()))
            ) {
                return Err(PluginError::Action(format!("{name} is still starting")));
            }
        }
    }

    /// Queue `event` (made with the reply sender) on `handle` and wait for its reply.
    async fn ask<T>(
        &self,
        plugin: &str,
        name: String,
        handle: Arc<InstanceHandle>,
        event: impl FnOnce(oneshot::Sender<Result<T, String>>) -> Event,
    ) -> Result<T, PluginError> {
        let (reply, rx) = oneshot::channel();
        let posted = handle.post(event(reply));
        drop(handle);
        if posted == Err(Dropped::Full) {
            return Err(PluginError::Action(format!("{name} is busy")));
        }
        let Ok(replied) = tokio::time::timeout(REPLY_WAIT, rx).await else {
            // Still working through earlier events (each call is bounded, so it is not stuck).
            tracing::warn!(target: "plugin", plugin = %plugin, "no reply within {} s", REPLY_WAIT.as_secs());
            return Err(PluginError::Action(format!("{name} is busy")));
        };
        match replied {
            Ok(Ok(value)) => Ok(value),
            // The instance answers with these exact texts when it stops or crashes meanwhile.
            Ok(Err(msg)) if msg == PluginError::Off(name.clone()).to_string() => Err(PluginError::Off(name)),
            Ok(Err(msg)) if msg == PluginError::Failed(name.clone()).to_string() => {
                Err(PluginError::Failed(name))
            }
            Ok(Err(msg)) => Err(PluginError::Action(msg)),
            // The event was dropped: the instance is gone.
            Err(_) => Err(PluginError::Failed(name)),
        }
    }

    pub(super) async fn action(
        &self,
        plugin: &str,
        action: &str,
        params: serde_json::Value,
    ) -> Result<(), PluginError> {
        let (handle, name) = self.running(plugin).await?;
        self.ask(plugin, name, handle, |reply| Event::Action {
            action: action.to_string(),
            params,
            reply,
        })
        .await
    }

    pub(super) async fn list_items(
        &self,
        plugin: &str,
        picker: &str,
        query: &str,
        group: &str,
    ) -> Result<Vec<PickItem>, PluginError> {
        // Checked before anything starts: a plugin that cannot answer is not started for nothing.
        {
            let st = self.state();
            let e = st
                .plugins
                .get(plugin)
                .ok_or_else(|| PluginError::NotInstalled(plugin.to_string()))?;
            let Some(loaded) = &e.loaded else {
                return Err(PluginError::Load(e.status.detail()));
            };
            if !loaded.lists_items {
                return Err(PluginError::Action(format!(
                    "{} has no list to pick from",
                    e.name
                )));
            }
            if loaded.manifest.picker(picker).is_none() {
                return Err(PluginError::Invalid(format!(
                    "{} has no picker \"{picker}\"",
                    e.name
                )));
            }
        }
        let (handle, name) = self.running(plugin).await?;
        let cut = |s: &str| truncate(s, MAX_QUERY).to_string();
        self.ask(plugin, name, handle, |reply| Event::ListItems {
            picker: picker.to_string(),
            query: cut(query),
            group: cut(group),
            reply,
        })
        .await
    }

    pub(super) async fn apply_settings(&self, settings: &Settings) {
        let _ops = self.ops.lock().await;
        let mut stopping = Vec::new();
        {
            let mut st = self.state();
            st.settings = settings.plugins.clone();
            let State {
                plugins,
                settings,
                closed,
            } = &mut *st;
            let mut changed = false;
            for (id, e) in plugins.iter_mut() {
                let (enabled, table) = enabled_and_settings(settings, id);
                let settings_changed = e.settings != table;
                let was_enabled = std::mem::replace(&mut e.enabled, enabled);
                e.settings = table;
                if e.loaded.is_none() {
                    continue;
                }
                if was_enabled && !enabled {
                    stopping.extend(Self::take_run(e));
                    e.status = PluginStatus::Off;
                    changed = true;
                } else if !was_enabled && enabled {
                    e.status = PluginStatus::Ready;
                    e.last_crash = None;
                    changed = true;
                    if e.wanted && !*closed {
                        self.launch(id, e);
                    }
                } else if settings_changed
                    && let Some(run) = &e.run
                    && let Some(loaded) = &e.loaded
                {
                    let _ = run.handle.post(Event::Init {
                        settings: table_to_json(&loaded.manifest.settings_with_defaults(&e.settings)),
                    });
                }
            }
            if changed {
                self.bump();
            }
        }
        for run in stopping {
            stop_run(run).await;
        }
    }

    pub(super) async fn remove(&self, id: &str) -> Result<(), PluginError> {
        let _ops = self.ops.lock().await;
        // Unlisted first: from here on actions find nothing to start, so no new instance can
        // save secrets after they are deleted below.
        let (mut entry, run) = {
            let mut st = self.state();
            let mut e = st
                .plugins
                .remove(id)
                .ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
            let run = Self::take_run(&mut e);
            (e, run)
        };
        self.bump();
        if let Some(run) = run {
            stop_run(run).await;
        }
        let folder = self.dir.join(id);
        let deleted = tokio::task::spawn_blocking(move || match std::fs::remove_dir_all(&folder) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(PluginError::Io(format!(
                "cannot delete {}: {e}",
                folder.display()
            ))),
            _ => Ok(()),
        })
        .await
        .map_err(|e| PluginError::Io(format!("cannot delete the plugin: {e}")))
        .and_then(|r| r);
        if let Err(e) = deleted {
            // Still installed: list it again, stopped.
            if entry.loaded.is_some() {
                entry.status = if entry.enabled {
                    PluginStatus::Ready
                } else {
                    PluginStatus::Off
                };
            }
            let mut st = self.state();
            if !st.plugins.contains_key(id) {
                st.plugins.insert(id.to_string(), entry);
            }
            drop(st);
            self.bump();
            return Err(e);
        }
        self.secrets
            .save(id, &BTreeMap::new())
            .await
            .map_err(|why| PluginError::Io(format!("the plugin is removed, but its secrets are not: {why}")))
    }

    pub(super) async fn restart(&self, id: &str) -> Result<(), PluginError> {
        let _ops = self.ops.lock().await;
        let run = {
            let mut st = self.state();
            let e = st
                .plugins
                .get_mut(id)
                .ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
            match &e.status {
                PluginStatus::Incompatible(why) => return Err(PluginError::Load(why.clone())),
                PluginStatus::Off => return Err(PluginError::Off(e.name.clone())),
                _ => {
                    let run = Self::take_run(e);
                    if e.enabled {
                        // Actions wait (briefly) for the new instance instead of starting another.
                        e.status = PluginStatus::Starting;
                        self.bump();
                    }
                    run
                }
            }
        };
        if let Some(run) = run {
            stop_run(run).await;
        }
        let mut st = self.state();
        if st.closed {
            return Ok(());
        }
        if let Some(e) = st.plugins.get_mut(id)
            && e.enabled
        {
            e.last_crash = None;
            self.launch(id, e);
        }
        Ok(())
    }

    pub(super) async fn shutdown(&self) {
        let _ops = self.ops.lock().await;
        let runs: Vec<Run> = {
            let mut st = self.state();
            st.closed = true;
            let mut runs = Vec::new();
            for e in st.plugins.values_mut().filter(|e| e.loaded.is_some()) {
                runs.extend(Self::take_run(e));
                e.status = PluginStatus::Off;
            }
            runs
        };
        futures_util::future::join_all(runs.into_iter().map(stop_run)).await;
        self.bump();
    }

    /// After an install: stop the old instance of `id` (if any). Returns it so the caller can
    /// wait for it to end before replacing its folder.
    pub(super) fn detach(&self, id: &str) -> Option<Run> {
        let mut st = self.state();
        let e = st.plugins.get_mut(id)?;
        let run = Self::take_run(e);
        if e.loaded.is_some() && e.enabled {
            // Actions wait (briefly) for the new instance instead of failing.
            e.status = PluginStatus::Starting;
            self.bump();
        }
        run
    }

    /// Register a freshly installed plugin (replacing any entry for its id) and start it when
    /// enabled; the old instance must be stopped already.
    pub(super) fn installed(&self, loaded: LoadedPlugin) {
        let mut st = self.state();
        let id = loaded.manifest.id.clone();
        let (enabled, table) = enabled_and_settings(&st.settings, &id);
        let closed = st.closed;
        let entry = Entry {
            name: loaded.manifest.name.clone(),
            version: loaded.manifest.version.clone(),
            status: if enabled {
                PluginStatus::Ready
            } else {
                PluginStatus::Off
            },
            loaded: Some(Arc::new(loaded)),
            enabled,
            settings: table,
            wanted: true,
            attempt: 0,
            run: None,
            draining: None,
            last_crash: None,
        };
        st.plugins.insert(id.clone(), entry);
        if enabled
            && !closed
            && let Some(e) = st.plugins.get_mut(&id)
        {
            self.launch(&id, e);
        }
        self.bump();
    }

    /// The old entry's instance came back (an install failed after stopping it): start it again.
    pub(super) fn resume(&self, id: &str) {
        let mut st = self.state();
        let closed = st.closed;
        let Some(e) = st.plugins.get_mut(id) else {
            return;
        };
        if e.loaded.is_none() || e.run.is_some() {
            return;
        }
        if e.enabled && !closed {
            self.launch(id, e);
        } else {
            e.status = if e.enabled {
                PluginStatus::Ready
            } else {
                PluginStatus::Off
            };
            self.bump();
        }
    }
}

/// Stop a detached instance and wait (up to 5 s) until its supervisor has handled the end:
/// its last outputs are delivered and its secrets saved.
pub(super) async fn stop_run(run: Run) {
    run.handle.stop();
    drop(run.handle);
    if tokio::time::timeout(STOP_WAIT, run.supervisor).await.is_err() {
        tracing::warn!(target: "plugin", "an instance did not stop within {} s", STOP_WAIT.as_secs());
    }
}

/// Forward instance `attempt`'s outputs until it exits, then report the end.
async fn supervise(
    host: Weak<Host>,
    id: String,
    attempt: u64,
    mut out: mpsc::UnboundedReceiver<Output>,
    mut exited: oneshot::Receiver<Exit>,
) {
    let exit = loop {
        tokio::select! {
            biased;
            Some(output) = out.recv() => match host.upgrade() {
                Some(host) => host.output(&id, attempt, output).await,
                None => return,
            },
            exit = &mut exited => {
                break exit.unwrap_or_else(|_| Exit::Crashed("ended without saying why".into()));
            }
        }
    };
    let Some(host) = host.upgrade() else {
        return;
    };
    // The instance's Store (and so its output sender) is gone before `exited` resolves.
    while let Ok(output) = out.try_recv() {
        host.output(&id, attempt, output).await;
    }
    host.ended(&id, attempt, exit);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts saves.
    #[derive(Default)]
    struct Saves(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl SecretStore for Saves {
        async fn load(&self, _plugin: &str) -> Result<BTreeMap<String, String>, String> {
            Ok(BTreeMap::new())
        }
        async fn save(&self, plugin: &str, _secrets: &BTreeMap<String, String>) -> Result<(), String> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(plugin.to_string());
            Ok(())
        }
    }

    fn entry(draining: Option<u64>) -> Entry {
        Entry {
            name: "Ghost".into(),
            version: "1.0.0".into(),
            status: PluginStatus::Ready,
            loaded: None,
            enabled: true,
            settings: toml::Table::new(),
            wanted: false,
            attempt: 0,
            run: None,
            draining,
            last_crash: None,
        }
    }

    #[test]
    fn a_cancelled_scan_stops_between_plugins() {
        let dir = tempfile::tempdir().expect("dir");
        for id in ["one", "two"] {
            std::fs::create_dir(dir.path().join(id)).expect("folder");
            std::fs::write(dir.path().join(id).join("plugin.toml"), "not a manifest").expect("manifest");
        }
        let engine = Err("no engine".to_string());
        assert_eq!(scan(dir.path(), &engine, &AtomicBool::new(false)).len(), 2);
        assert!(scan(dir.path(), &engine, &AtomicBool::new(true)).is_empty());
    }

    #[test]
    fn unpack_leftovers_are_deleted_at_start() {
        let dir = tempfile::tempdir().expect("dir");
        let leftover = dir.path().join(".unpack-AbC123");
        std::fs::create_dir_all(leftover.join("assets")).expect("leftover");
        std::fs::write(leftover.join("plugin.toml"), "id = \"x\"").expect("manifest");
        // Not leftovers: a plugin folder, another hidden folder, a file with the prefix.
        std::fs::create_dir(dir.path().join("unpack")).expect("plugin");
        std::fs::create_dir(dir.path().join(".hidden")).expect("hidden");
        std::fs::write(dir.path().join(".unpack-file"), "x").expect("file");
        let engine = Err("no engine".to_string());
        scan(dir.path(), &engine, &AtomicBool::new(false));
        assert!(!leftover.exists());
        assert!(dir.path().join("unpack").is_dir());
        assert!(dir.path().join(".hidden").is_dir());
        assert!(dir.path().join(".unpack-file").is_file());
    }

    #[tokio::test]
    async fn late_secrets_are_not_saved() {
        let dir = tempfile::tempdir().expect("dir");
        let saves = Arc::new(Saves::default());
        let (deck, _rx) = mpsc::channel(16);
        let host = Host::start(
            dir.path().join("plugins"),
            dir.path().join("cache"),
            Arc::clone(&saves) as Arc<dyn SecretStore>,
            &Settings::default(),
            &BTreeSet::new(),
            deck,
        )
        .await;
        let secrets = || Output::SecretsChanged(BTreeMap::from([("k".into(), "v".into())]));
        // Removed: no entry at all.
        host.output("ghost", 7, secrets()).await;
        // Installed again since, or stopped long ago: another instance's attempt.
        host.state().plugins.insert("ghost".into(), entry(Some(3)));
        host.output("ghost", 7, secrets()).await;
        assert!(saves.0.lock().expect("saves").is_empty());
        // The instance being stopped still saves its last change.
        host.output("ghost", 3, secrets()).await;
        assert_eq!(*saves.0.lock().expect("saves"), ["ghost"]);
        // Once a new run is launched, the stopped instance's late secrets no longer count.
        {
            let mut st = host.state();
            let e = st.plugins.get_mut("ghost").expect("ghost");
            host.launch("ghost", e);
        }
        host.output("ghost", 3, secrets()).await;
        assert_eq!(*saves.0.lock().expect("saves"), ["ghost"]);
    }
}
