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
        Hidraw::from_file(file).map_err(|e| DeviceState::Error {
            message: e.to_string(),
        })
    }

    /// Wrap an already-open, non-blocking file (the hidraw node, or a socket in tests).
    pub fn from_file(file: File) -> io::Result<Hidraw> {
        // `AsyncFd::new` is deprecated because a custom `AsRawFd` could hand out a different fd later;
        // `std::fs::File` always returns the fd it owns, so the safe constructor is sound here and
        // keeps this crate free of `unsafe`.
        #[expect(deprecated)]
        let fd = AsyncFd::new(file)?;
        Ok(Hidraw { fd })
    }
}

#[async_trait::async_trait]
impl Transport for Hidraw {
    /// One `write(2)` per packet, prefixed with report ID 0 as Ulanzi Studio does. The kernel strips a
    /// leading 0, so the deck gets the exact 1024 bytes; without the prefix it strips the first byte of
    /// any chunk that happens to start with 0x00 and the upload shifts by one.
    async fn send(&mut self, packets: &[Packet]) -> io::Result<()> {
        for p in packets {
            let report = ulanzi_proto::with_report_id(p);
            loop {
                let mut guard = self.fd.writable().await?;
                match guard.try_io(|f| f.get_ref().write(&report)) {
                    // usbhid reports 1025 (the stripped ID included); accept 1024 too.
                    Ok(Ok(n)) if n >= PACKET_SIZE => break,
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

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    use super::*;

    #[tokio::test]
    async fn writes_report_id_then_the_packet() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        ours.set_nonblocking(true).expect("nonblocking");
        let mut dev = Hidraw::from_file(File::from(std::os::fd::OwnedFd::from(ours))).expect("wrap");
        let mut p = ulanzi_proto::brightness(5);
        p[0] = 0x00; // a continuation chunk that starts with 0x00 must keep that byte
        dev.send(&[p]).await.expect("send");
        let mut buf = vec![0u8; PACKET_SIZE + 1];
        theirs.read_exact(&mut buf).expect("read");
        assert_eq!(buf[0], 0, "report ID");
        assert_eq!(&buf[1..], &p[..]);
    }
}
