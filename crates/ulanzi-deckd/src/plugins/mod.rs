//! WebAssembly plugin host.

mod engine;
mod instance;

pub use engine::{LoadedPlugin, PluginEngine};
pub use instance::{Event, Exit, HostState, InstanceHandle, Output, spawn_instance};
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
    #[error("{0}")]
    Io(String),
}
