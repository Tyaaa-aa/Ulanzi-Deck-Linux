//! OBS Studio over obs-websocket 5.

pub mod client;
pub mod detect;
pub mod protocol;

pub use client::{ObsHandle, ObsState, Probe, Timing, system_probe};
