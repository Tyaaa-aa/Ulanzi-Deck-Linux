//! Plugin state on its way to the deck. A plugin may set state faster than the deck takes it, so
//! pending values are coalesced per `(plugin, key)` (the latest wins) and memory stays bounded by
//! the number of distinct keys. Order is kept: a `PluginCleared` drops that plugin's values still
//! waiting before it, and anything set after it is delivered after it.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, mpsc};

use crate::deck::DeckMsg;

enum Slot {
    State { plugin: String, key: String },
    Cleared(String),
}

#[derive(Default)]
struct Pending {
    order: VecDeque<Slot>,
    values: HashMap<(String, String), String>,
    closed: bool,
}

impl Pending {
    fn state(&mut self, plugin: String, key: String, value: String) {
        let slot = (plugin, key);
        if let Some(v) = self.values.get_mut(&slot) {
            *v = value;
            return;
        }
        self.order.push_back(Slot::State {
            plugin: slot.0.clone(),
            key: slot.1.clone(),
        });
        self.values.insert(slot, value);
    }

    fn cleared(&mut self, plugin: String) {
        self.order
            .retain(|s| !matches!(s, Slot::State { plugin: p, .. } if *p == plugin));
        self.values.retain(|(p, _), _| *p != plugin);
        self.order.push_back(Slot::Cleared(plugin));
    }

    fn pop(&mut self) -> Option<DeckMsg> {
        Some(match self.order.pop_front()? {
            Slot::Cleared(plugin) => DeckMsg::PluginCleared(plugin),
            Slot::State { plugin, key } => {
                let value = self
                    .values
                    .remove(&(plugin.clone(), key.clone()))
                    .unwrap_or_default();
                DeckMsg::PluginState { plugin, key, value }
            }
        })
    }
}

struct Shared {
    pending: Mutex<Pending>,
    wake: Notify,
}

impl Shared {
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The sending side; never blocks. Dropping it ends the forwarding task once the queue is empty.
pub(super) struct DeckQueue(Arc<Shared>);

impl DeckQueue {
    /// Start the task that forwards queued messages to `deck`, in order.
    pub(super) fn spawn(deck: mpsc::Sender<DeckMsg>) -> DeckQueue {
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
            wake: Notify::new(),
        });
        let rx = Arc::clone(&shared);
        tokio::spawn(async move {
            loop {
                let (msg, closed) = {
                    let mut p = rx.pending();
                    (p.pop(), p.closed)
                };
                match msg {
                    Some(msg) => {
                        if deck.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None if closed => break,
                    None => rx.wake.notified().await,
                }
            }
        });
        DeckQueue(shared)
    }

    /// Queue `PluginState` or `PluginCleared`; other messages are not plugin output and are ignored.
    pub(super) fn send(&self, msg: DeckMsg) {
        {
            let mut p = self.0.pending();
            match msg {
                DeckMsg::PluginState { plugin, key, value } => p.state(plugin, key, value),
                DeckMsg::PluginCleared(plugin) => p.cleared(plugin),
                _ => return,
            }
        }
        // A stored permit wakes the task even when it is not waiting yet.
        self.0.wake.notify_one();
    }
}

impl Drop for DeckQueue {
    fn drop(&mut self) {
        self.0.pending().closed = true;
        self.0.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(p: &str, k: &str, v: &str) -> (String, String, String) {
        (p.into(), k.into(), v.into())
    }

    fn drain(p: &mut Pending) -> Vec<String> {
        std::iter::from_fn(|| p.pop())
            .map(|m| match m {
                DeckMsg::PluginState { plugin, key, value } => format!("{plugin}.{key}={value}"),
                DeckMsg::PluginCleared(plugin) => format!("clear {plugin}"),
                _ => "other".into(),
            })
            .collect()
    }

    #[test]
    fn latest_value_wins_in_place() {
        let mut p = Pending::default();
        for (a, b, c) in [
            state("a", "x", "1"),
            state("b", "y", "1"),
            state("a", "x", "2"),
            state("a", "x", "3"),
        ] {
            p.state(a, b, c);
        }
        assert_eq!(drain(&mut p), ["a.x=3", "b.y=1"]);
        assert!(p.values.is_empty());
    }

    #[test]
    fn cleared_drops_earlier_values_and_keeps_order() {
        let mut p = Pending::default();
        p.state("a".into(), "x".into(), "1".into());
        p.state("b".into(), "y".into(), "1".into());
        p.cleared("a".into());
        p.state("a".into(), "x".into(), "2".into());
        p.state("a".into(), "x".into(), "3".into());
        assert_eq!(drain(&mut p), ["b.y=1", "clear a", "a.x=3"]);
    }

    #[test]
    fn bounded_by_distinct_keys() {
        let mut p = Pending::default();
        for i in 0..10_000 {
            p.state("a".into(), "x".into(), i.to_string());
        }
        assert_eq!(p.order.len(), 1);
        assert_eq!(drain(&mut p), ["a.x=9999"]);
    }
}
