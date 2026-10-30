//! Ulanzi Deck core: profile model, storage, migration from the Python version, key rendering.

/// This app's version, compared with a plugin's `min_app`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod apps;
pub mod autoswitch;
pub mod bundle;
pub mod catalog;
mod color;
pub mod edit;
mod error;
pub mod import;
pub mod keys;
pub mod macro_record;
pub mod migrate;
pub mod model;
pub mod obs;
pub mod plugin;
pub mod render;
pub mod snapshot;
pub mod steam;
pub mod store;
pub mod template;
pub mod update;
pub mod upgrade;

pub use color::{Color, ColorParseError};
pub use error::CoreError;
