//! WebAssembly plugin host.

mod deck_queue;
mod engine;
mod install;
mod instance;
mod net;
mod secrets;
mod supervisor;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use ulanzi_core::model::Settings;

use crate::deck::DeckMsg;

pub use engine::{LoadedPlugin, PluginEngine};
pub use instance::{Dropped, Event, Exit, HostState, InstanceHandle, Output, spawn_instance};
pub use secrets::{FileSecrets, KeyringSecrets, SecretStore, system_secrets};
pub use ulanzi_core::plugin::ManifestError;

// The generated bindings cover the whole contract; not every item is used yet.
#[allow(dead_code)]
mod bindings {
    wasmtime::component::bindgen!({ world: "plugin", path: "../ulanzi-plugin/wit" });
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

    /// Turn plugins on or off per `[plugins.<id>].enabled`; running plugins whose settings
    /// changed get them again (`init`).
    pub async fn apply_settings(&self, settings: &Settings) {
        self.inner.apply_settings(settings).await
    }

    /// Install (or replace) the plugin in folder `src`; returns its id. Only in developer mode.
    pub async fn install_dir(&self, src: &Path, developer_mode: bool) -> Result<String, PluginError> {
        self.inner.install_dir(src, developer_mode).await
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
