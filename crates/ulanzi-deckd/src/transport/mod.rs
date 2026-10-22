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

/// A connected device: send packets, receive raw input reports.
#[async_trait::async_trait]
pub trait Transport: Send {
    async fn send(&mut self, packets: &[Packet]) -> io::Result<()>;
    async fn recv(&mut self) -> io::Result<Vec<u8>>;
}
