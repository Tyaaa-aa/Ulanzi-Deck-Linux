//! Ulanzi Deck core: profile model, storage, migration from the Python version, key rendering.

pub mod apps;
pub mod autoswitch;
pub mod bundle;
pub mod catalog;
mod color;
pub mod edit;
mod error;
pub mod import;
pub mod migrate;
pub mod model;
pub mod render;
pub mod steam;
pub mod store;

pub use color::{Color, ColorParseError};
pub use error::CoreError;
