//! Ulanzi Deck GUI: the D-Bus client and the GTK user interface.

/// The app's ID (GApplication, desktop file, icons). Not the service's bus name.
pub const APP_ID: &str = "io.github.tyaaa_aa.UlanziDeck";

pub mod client;
pub mod runtime;
pub mod state;
pub mod ui;
