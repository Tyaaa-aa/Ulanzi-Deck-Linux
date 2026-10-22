//! Ulanzi Deck core: profile model, storage, migration from the Python version, key rendering.

mod color;
mod error;
pub mod model;
pub mod store;

pub use color::{Color, ColorParseError};
pub use error::CoreError;
