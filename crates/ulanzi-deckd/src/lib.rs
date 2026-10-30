//! Ulanzi Deck daemon: owns the device, runs actions, serves D-Bus.

pub mod actions;
pub mod dbus;
pub mod deck;
pub mod focus;
pub mod macros;
pub mod plugins;
pub mod screen;
pub mod sysinfo;
pub mod transport;
pub mod tray;
pub mod updates;
pub mod vars;
pub mod widgets;
