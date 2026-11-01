//! Device I/O: discovery, the hidraw transport, hot-plug notifications and a mock for tests.

mod discover;
mod hidraw;
mod hotplug;
mod mock;

pub use discover::{DeviceState, find_holder, find_node, probe};
pub use hidraw::Hidraw;
pub use hotplug::watch_dev;
pub use mock::{MockHandle, MockTransport};

use std::io;

use ulanzi_proto::Packet;

/// What [`Transport::exchange`] did.
#[derive(Debug, PartialEq)]
pub enum Exchange {
    Got(Vec<u8>),
    Sent,
}

/// A connected device: send packets, receive raw input reports.
#[async_trait::async_trait]
pub trait Transport: Send {
    async fn send(&mut self, packets: &[Packet]) -> io::Result<()>;
    async fn recv(&mut self) -> io::Result<Vec<u8>>;
    /// Wait for whichever comes first: an input report, or room to write `next`. A write can stall
    /// for up to a second, and input must not wait for it. Safe to cancel: nothing is half done.
    async fn exchange(&mut self, next: Option<&Packet>) -> io::Result<Exchange>;
}
