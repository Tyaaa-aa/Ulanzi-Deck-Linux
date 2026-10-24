//! WebAssembly plugin host.

mod deck_queue;
mod engine;
mod install;
mod instance;
mod net;
mod package;
mod secrets;
mod supervisor;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use ulanzi_core::model::Settings;

use crate::deck::DeckMsg;

pub use engine::{LoadedPlugin, PluginEngine};
pub use instance::{Dropped, Event, Exit, HostState, InstanceHandle, Output, spawn_instance};
pub use secrets::{FileSecrets, KeyringSecrets, LazySecrets, SecretStore, system_secrets};
pub use ulanzi_core::plugin::ManifestError;

// The generated bindings cover the whole contract; not every item is used yet. The host binds
// the 1.0 exports (`plugin-base`); `list-items` (1.1) is looked up by name, so components built
// against 1.0 keep loading.
#[allow(dead_code)]
mod bindings {
    wasmtime::component::bindgen!({ world: "plugin-base", path: "../ulanzi-plugin/wit" });
}

pub use bindings::*;

/// Why a plugin could not be loaded or used. The `String`s are short, user-presentable reasons.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PluginError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// The wasm is not a valid component, or imports something the host does not provide.
    #[error("{0}")]
    Load(String),
    /// The plugin uses something its manifest does not ask for.
    #[error("{0}")]
    Permission(String),
    #[error("the {0} plugin is not installed")]
    NotInstalled(String),
    #[error("the {0} plugin is turned off")]
    Off(String),
    #[error("{0} stopped working")]
    Failed(String),
    /// The plugin refused an action (its own message), or could not take it right now.
    #[error("{0}")]
    Action(String),
    #[error("{0}")]
    Io(String),
    /// A request the plugin's manifest does not allow (for example an undeclared secret).
    #[error("{0}")]
    Invalid(String),
}

/// Where an installed plugin stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// Turned off in settings.
    Off,
    /// Loaded; starts on its first action.
    Ready,
    Starting,
    Running,
    /// Running, but reports a problem (its message).
    Problem(String),
    /// Crashed twice within 5 minutes (or could not start); off until restarted. Why.
    Failed(String),
    /// Cannot be loaded. Why.
    Incompatible(String),
}

impl PluginStatus {
    pub fn id(&self) -> &'static str {
        match self {
            PluginStatus::Off => "off",
            PluginStatus::Ready => "ready",
            PluginStatus::Starting => "starting",
            PluginStatus::Running => "running",
            PluginStatus::Problem(_) => "problem",
            PluginStatus::Failed(_) => "failed",
            PluginStatus::Incompatible(_) => "incompatible",
        }
    }

    /// The reason or message for `problem`, `failed` and `incompatible`; empty otherwise.
    pub fn detail(&self) -> String {
        match self {
            PluginStatus::Problem(m) | PluginStatus::Failed(m) | PluginStatus::Incompatible(m) => m.clone(),
            _ => String::new(),
        }
    }
}

/// One installed plugin (or a plugin folder that could not be loaded), for lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    /// The plugin id (its folder name).
    pub id: String,
    pub name: String,
    /// Empty when the manifest could not be read.
    pub version: String,
    pub status: PluginStatus,
}

/// The daemon's plugins: loads them, starts and supervises their instances, runs their actions
/// and installs or removes them. Cheap to clone; safe to use from several tasks at once.
#[derive(Clone)]
pub struct PluginHost {
    inner: Arc<supervisor::Host>,
}

impl PluginHost {
    /// Load every plugin folder in `dir` (compiled code is cached in `cache`, used as given) and
    /// start the enabled ones that `used` names or that run in the background; the others start
    /// on their first action. Broken folders are listed as `incompatible`. Must run inside tokio.
    pub async fn start(
        dir: PathBuf,
        cache: PathBuf,
        secrets: Arc<dyn SecretStore>,
        settings: &Settings,
        used: &BTreeSet<String>,
        deck: mpsc::Sender<DeckMsg>,
    ) -> PluginHost {
        PluginHost {
            inner: supervisor::Host::start(dir, cache, secrets, settings, used, deck).await,
        }
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        self.inner.list()
    }

    /// Bumps on any status or list change.
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.inner.watch()
    }

    /// Run `action` on `plugin`, starting it first if needed.
    pub async fn action(
        &self,
        plugin: &str,
        action: &str,
        params: serde_json::Value,
    ) -> Result<(), PluginError> {
        self.inner.action(plugin, action, params).await
    }

    /// The entries of `plugin`'s picker `picker` matching `query` (`""`: all), only those in `group`
    /// unless it is `""`; starts the plugin first if needed. Bounded: at most 5000 entries, every
    /// field cut. A picker the manifest does not declare is refused (`Invalid`); the plugin's own
    /// refusal comes back as `Action` with its message.
    pub async fn list_items(
        &self,
        plugin: &str,
        picker: &str,
        query: &str,
        group: &str,
    ) -> Result<Vec<ulanzi_core::plugin::PickItem>, PluginError> {
        self.inner.list_items(plugin, picker, query, group).await
    }

    /// Turn plugins on or off per `[plugins.<id>].enabled`; running plugins whose settings
    /// changed get them again (`init`).
    pub async fn apply_settings(&self, settings: &Settings) {
        self.inner.apply_settings(settings).await
    }

    /// Install (or replace) the plugin in folder `src`; returns its id. Only in developer mode.
    pub async fn install_dir(&self, src: &Path, developer_mode: bool) -> Result<String, PluginError> {
        self.inner.install_dir(src, developer_mode).await
    }

    /// Install (or replace) the plugin in the `.ulanziplugin` file `file`; returns its id. Only in
    /// developer mode (the package is unsigned).
    pub async fn install_file(&self, file: &Path, developer_mode: bool) -> Result<String, PluginError> {
        self.inner.install_file(file, developer_mode).await
    }

    /// Check the `.ulanziplugin` file `file` like an install would (unpacking it to a temporary
    /// folder and compiling it); works without Developer mode.
    pub async fn inspect_file(&self, file: &Path) -> Result<ulanzi_core::plugin::Inspected, PluginError> {
        self.inner.inspect_file(file).await
    }

    /// The manifests of the loaded plugins (folders that cannot be loaded have none).
    pub fn manifests(&self) -> BTreeMap<String, ulanzi_core::plugin::Manifest> {
        self.inner.manifests()
    }

    /// Check the plugin folder `src` like an install would (compiling it), writing nothing;
    /// works without Developer mode.
    pub async fn inspect_dir(&self, src: &Path) -> Result<ulanzi_core::plugin::Inspected, PluginError> {
        self.inner.inspect_dir(src).await
    }

    /// The declared secret keys of `id` that are set, in the secret store or (older plugins) in
    /// its settings.
    pub async fn secret_keys(&self, id: &str) -> Result<Vec<String>, PluginError> {
        self.inner.secret_keys(id).await
    }

    /// Set (or, with `None` or an empty value, clear) a declared secret of `id`, restarting
    /// the plugin when it was starting or running so it sees the new value.
    pub async fn set_secret(&self, id: &str, key: &str, value: Option<String>) -> Result<(), PluginError> {
        self.inner.set_secret(id, key, value).await
    }

    /// Stop the plugin and delete its folder and secrets. Its settings are kept.
    pub async fn remove(&self, id: &str) -> Result<(), PluginError> {
        self.inner.remove(id).await
    }

    /// Stop and start the plugin again, forgetting earlier crashes.
    pub async fn restart(&self, id: &str) -> Result<(), PluginError> {
        self.inner.restart(id).await
    }

    /// Stop every instance and wait (briefly) for them to end.
    pub async fn shutdown(&self) {
        self.inner.shutdown().await
    }
}

/// The plugin host as it comes up in the background, so the deck never waits for it: empty
/// until it is ready. Cheap to clone.
#[derive(Clone)]
pub struct PluginSlot {
    inner: Arc<Slot>,
}

struct Slot {
    stage: watch::Sender<Stage>,
    /// The task bringing the host up, until `shutdown` takes it.
    starting: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
enum Stage {
    Starting,
    Ready(PluginHost),
    /// Shut down before the host was ready, or its start failed.
    Stopped,
}

impl PluginSlot {
    fn with(stage: Stage) -> PluginSlot {
        PluginSlot {
            inner: Arc::new(Slot {
                stage: watch::Sender::new(stage),
                starting: Mutex::new(None),
            }),
        }
    }

    /// Bring the host up in a spawned task; `start` returns it ready. Must run inside tokio.
    pub fn start(start: impl Future<Output = PluginHost> + Send + 'static) -> PluginSlot {
        let slot = PluginSlot::with(Stage::Starting);
        let inner = Arc::clone(&slot.inner);
        let task = tokio::spawn(async move {
            use futures_util::FutureExt;
            let stage = match std::panic::AssertUnwindSafe(start).catch_unwind().await {
                Ok(host) => Stage::Ready(host),
                Err(_) => {
                    tracing::error!("the plugins could not start");
                    Stage::Stopped
                }
            };
            inner.stage.send_replace(stage);
        });
        *slot.lock_starting() = Some(task);
        slot
    }

    /// A host that is ready already.
    pub fn ready(host: PluginHost) -> PluginSlot {
        PluginSlot::with(Stage::Ready(host))
    }

    fn lock_starting(&self) -> std::sync::MutexGuard<'_, Option<JoinHandle<()>>> {
        self.inner.starting.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The host, once it is ready.
    pub fn get(&self) -> Option<PluginHost> {
        match &*self.inner.stage.borrow() {
            Stage::Ready(host) => Some(host.clone()),
            Stage::Starting | Stage::Stopped => None,
        }
    }

    /// The host is still coming up.
    pub fn is_starting(&self) -> bool {
        matches!(*self.inner.stage.borrow(), Stage::Starting)
    }

    /// Wait until the host is ready; `None` when it was shut down first.
    pub async fn wait(&self) -> Option<PluginHost> {
        let mut rx = self.inner.stage.subscribe();
        let stage = rx.wait_for(|s| !matches!(s, Stage::Starting)).await.ok()?.clone();
        match stage {
            Stage::Ready(host) => Some(host),
            Stage::Starting | Stage::Stopped => None,
        }
    }

    /// Stop the plugins: cancel a start still in progress, or shut the ready host down.
    pub async fn shutdown(&self) {
        let task = self.lock_starting().take();
        if let Some(task) = task {
            // A host still being made is dropped (which stops anything it began); one that is
            // made is in the slot by now: nothing awaits between the two.
            task.abort();
            let _ = task.await;
        }
        self.inner.stage.send_if_modified(|s| {
            let starting = matches!(s, Stage::Starting);
            if starting {
                *s = Stage::Stopped;
            }
            starting
        });
        if let Some(host) = self.get() {
            host.shutdown().await;
        }
    }
}
