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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ulanzi_core::model::{PluginEntry, Settings, table_to_json};
use ulanzi_core::plugin::{CONTRACT, Manifest, ManifestError};

use super::deck_queue::DeckQueue;
use super::ulanzi::plugin::types::Status;
use super::{
    Event, Exit, InstanceHandle, LoadedPlugin, Output, PluginEngine, PluginError, PluginInfo, PluginStatus,
    SecretStore, spawn_instance,
};
use crate::deck::DeckMsg;

/// A second crash within this long leaves the plugin failed.
const CRASH_WINDOW: Duration = Duration::from_secs(5 * 60);
/// An action waits this long for a starting plugin.
const START_WAIT: Duration = Duration::from_secs(2);
/// Stopping waits this long for an instance to end.
const STOP_WAIT: Duration = Duration::from_secs(5);

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

/// Every plugin folder in `dir`, loaded or with the reason it cannot be. Blocking.
fn scan(dir: &Path, engine: &Result<PluginEngine, String>) -> Vec<(String, Result<LoadedPlugin, Bad>)> {
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
    let entries = std::fs::read_dir(dir)
        .map(|d| d.flatten().collect())
        .unwrap_or(entries);
    let mut found = Vec::new();
    for entry in entries {
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
            tokio::task::spawn_blocking(move || scan(&dir, &engine))
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

    pub(super) fn watch(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Start an instance of `e` in the background: `Starting` now, `Running` once it runs.
    /// Called with the state lock held.
    fn launch(&self, id: &str, e: &mut Entry) {
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
                handle.post(Event::Init {
                    settings: table_to_json(&e.settings),
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
        e.run.take()
    }

    pub(super) async fn action(
        &self,
        plugin: &str,
        action: &str,
        params: serde_json::Value,
    ) -> Result<(), PluginError> {
        let deadline = Instant::now() + START_WAIT;
        let mut changes = self.changed.subscribe();
        let (handle, name) = loop {
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
                        Some(run) => break (Arc::clone(&run.handle), e.name.clone()),
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
        };

        let (reply, rx) = oneshot::channel();
        handle.post(Event::Action {
            action: action.to_string(),
            params,
            reply,
        });
        drop(handle);
        match rx.await {
            Ok(Ok(())) => Ok(()),
            // The instance answers with these exact texts when it stops or crashes meanwhile.
            Ok(Err(msg)) if msg == PluginError::Off(name.clone()).to_string() => Err(PluginError::Off(name)),
            Ok(Err(msg)) if msg == PluginError::Failed(name.clone()).to_string() => {
                Err(PluginError::Failed(name))
            }
            Ok(Err(msg)) => Err(PluginError::Action(msg)),
            // The event was dropped: the queue was full or the instance is gone.
            Err(_) => Err(PluginError::Failed(name)),
        }
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
                } else if settings_changed && let Some(run) = &e.run {
                    run.handle.post(Event::Init {
                        settings: table_to_json(&e.settings),
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
