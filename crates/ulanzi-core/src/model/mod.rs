//! Profiles, pages, keys, actions and settings, as stored in TOML.

mod action;
pub mod foreign;
mod look;
mod macros;
mod profile;
mod screen;
mod settings;

pub use action::*;
pub use foreign::{ForeignSlot, Slot};
pub use look::*;
pub use macros::*;
pub use profile::*;
pub use screen::*;
pub use settings::*;
