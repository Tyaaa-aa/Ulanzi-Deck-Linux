use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use inotify::{Inotify, WatchMask};
use tokio::sync::mpsc;

/// Fallback re-check interval, in case inotify is unavailable or misses an event.
const RECHECK: Duration = Duration::from_secs(2);

/// Ticks whenever a `hidraw*` node in `dev` appears, disappears or changes permissions, and every
/// 2 s regardless. The receiver drives re-probing; ticks may be coalesced.
pub fn watch_dev(dev: PathBuf) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel(4);
    let inotify_tx = tx.clone();
    tokio::spawn(async move {
        let stream = Inotify::init().and_then(|ino| {
            ino.watches()
                .add(&dev, WatchMask::CREATE | WatchMask::DELETE | WatchMask::ATTRIB)?;
            ino.into_event_stream([0u8; 4096])
        });
        match stream {
            Ok(mut events) => {
                while let Some(ev) = events.next().await {
                    let hidraw = ev
                        .ok()
                        .and_then(|e| e.name)
                        .is_some_and(|n| n.to_string_lossy().starts_with("hidraw"));
                    if hidraw && inotify_tx.try_send(()).is_err() && inotify_tx.is_closed() {
                        break;
                    }
                }
            }
            Err(e) => tracing::warn!(
                "inotify on {} unavailable ({e}); polling every 2 s",
                dev.display()
            ),
        }
    });
    tokio::spawn(async move {
        let mut t = tokio::time::interval(RECHECK);
        loop {
            t.tick().await;
            if tx.try_send(()).is_err() && tx.is_closed() {
                break;
            }
        }
    });
    rx
}
