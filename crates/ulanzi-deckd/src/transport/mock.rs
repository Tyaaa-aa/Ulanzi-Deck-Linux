use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use ulanzi_proto::Packet;

use super::Transport;

/// In-memory device for tests: records sent packets, replays injected input reports.
pub struct MockTransport {
    handle: MockHandle,
    rx: mpsc::Receiver<io::Result<Vec<u8>>>,
}

/// Test-side view of a [`MockTransport`].
#[derive(Clone)]
pub struct MockHandle {
    pub sent: Arc<Mutex<Vec<Packet>>>,
    pub inject: mpsc::Sender<io::Result<Vec<u8>>>,
    /// When non-zero, sending fails with `ENODEV` once this many packets have been sent in total.
    pub fail_after: Arc<AtomicUsize>,
}

impl MockHandle {
    pub fn sent(&self) -> Vec<Packet> {
        self.sent.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Commands of the header packets sent so far (continuation chunks skipped).
    pub fn commands(&self) -> Vec<u16> {
        self.sent()
            .iter()
            .filter(|p| p[0] == 0x7c && p[1] == 0x7c)
            .map(|p| u16::from_be_bytes([p[2], p[3]]))
            .collect()
    }

    pub fn clear(&self) {
        if let Ok(mut v) = self.sent.lock() {
            v.clear();
        }
    }
}

impl MockTransport {
    pub fn new() -> (MockTransport, MockHandle) {
        let (tx, rx) = mpsc::channel(256);
        let handle = MockHandle {
            sent: Arc::default(),
            inject: tx,
            fail_after: Arc::new(AtomicUsize::new(0)),
        };
        (
            MockTransport {
                handle: handle.clone(),
                rx,
            },
            handle,
        )
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send(&mut self, packets: &[Packet]) -> io::Result<()> {
        let limit = self.handle.fail_after.load(Ordering::SeqCst);
        let mut sent = self
            .handle
            .sent
            .lock()
            .map_err(|_| io::Error::other("mock poisoned"))?;
        for p in packets {
            if limit != 0 && sent.len() >= limit {
                return Err(io::Error::from_raw_os_error(
                    rustix::io::Errno::NODEV.raw_os_error(),
                ));
            }
            sent.push(*p);
        }
        Ok(())
    }

    async fn recv(&mut self) -> io::Result<Vec<u8>> {
        match self.rx.recv().await {
            Some(r) => r,
            None => std::future::pending().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_records_packets() {
        let (mut t, h) = MockTransport::new();
        t.send(&[ulanzi_proto::brightness(5), ulanzi_proto::brightness(6)])
            .await
            .expect("send");
        assert_eq!(h.commands(), vec![0x000a, 0x000a]);
        h.inject.send(Ok(vec![1, 2, 3])).await.expect("inject");
        assert_eq!(t.recv().await.expect("recv"), vec![1, 2, 3]);
        h.fail_after.store(3, Ordering::SeqCst);
        let err = t
            .send(&[ulanzi_proto::brightness(7), ulanzi_proto::brightness(8)])
            .await
            .expect_err("fails");
        assert_eq!(err.raw_os_error(), Some(rustix::io::Errno::NODEV.raw_os_error()));
        assert_eq!(h.sent().len(), 3);
    }
}
