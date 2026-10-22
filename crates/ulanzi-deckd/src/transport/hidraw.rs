use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use tokio::io::unix::AsyncFd;
use ulanzi_proto::{PACKET_SIZE, Packet};

use super::Transport;
use super::discover::{DeviceState, open_error};

/// The deck's `/dev/hidrawN` node, non-blocking and driven by tokio.
pub struct Hidraw {
    fd: AsyncFd<File>,
}

impl Hidraw {
    pub fn open(node: &Path) -> Result<Hidraw, DeviceState> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(node)
            .map_err(|e| open_error(node.to_path_buf(), &e))?;
        // `AsyncFd::new` is deprecated because a custom `AsRawFd` could hand out a different fd later;
        // `std::fs::File` always returns the fd it owns, so the safe constructor is sound here and
        // keeps this crate free of `unsafe`.
        #[expect(deprecated)]
        let fd = AsyncFd::new(file).map_err(|e| DeviceState::Error {
            message: e.to_string(),
        })?;
        Ok(Hidraw { fd })
    }
}

#[async_trait::async_trait]
impl Transport for Hidraw {
    /// One `write(2)` per packet, bytes exactly as built (like hidapi's pass-through on Linux).
    async fn send(&mut self, packets: &[Packet]) -> io::Result<()> {
        for p in packets {
            loop {
                let mut guard = self.fd.writable().await?;
                match guard.try_io(|f| f.get_ref().write(p)) {
                    Ok(Ok(n)) if n == PACKET_SIZE => break,
                    Ok(Ok(n)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            format!("short write: {n} bytes"),
                        ));
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_would_block) => continue,
                }
            }
        }
        Ok(())
    }

    async fn recv(&mut self) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; PACKET_SIZE];
        loop {
            let mut guard = self.fd.readable().await?;
            match guard.try_io(|f| f.get_ref().read(&mut buf)) {
                Ok(Ok(0)) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                Ok(Ok(n)) => {
                    buf.truncate(n);
                    return Ok(buf);
                }
                Ok(Err(e)) => return Err(e),
                Err(_would_block) => continue,
            }
        }
    }
}
