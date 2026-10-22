use std::collections::HashMap;
use std::time::Duration;

use tokio::time::Instant;
use ulanzi_proto::Control;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Tap,
    Hold,
}

#[derive(Debug, Clone, Copy)]
struct Held {
    since: Instant,
    has_hold: bool,
    fired: bool,
}

/// Turns press/release pairs into taps and holds.
#[derive(Debug, Default)]
pub struct PressTracker {
    hold: Duration,
    held: HashMap<Control, Held>,
}

impl PressTracker {
    pub fn new(hold_ms: u32) -> PressTracker {
        PressTracker {
            hold: Duration::from_millis(hold_ms.max(50) as u64),
            held: HashMap::new(),
        }
    }

    pub fn on_press(&mut self, control: Control, now: Instant, has_hold: bool) {
        self.held.insert(
            control,
            Held {
                since: now,
                has_hold,
                fired: false,
            },
        );
    }

    /// A tap, a late-detected hold, or nothing (hold already fired, or no matching press).
    pub fn on_release(&mut self, control: Control, now: Instant) -> Option<Gesture> {
        let h = self.held.remove(&control)?;
        match (h.fired, h.has_hold && now.duration_since(h.since) >= self.hold) {
            (true, _) => None,
            (false, true) => Some(Gesture::Hold),
            (false, false) => Some(Gesture::Tap),
        }
    }

    /// Controls whose hold threshold has just passed while still held.
    pub fn poll(&mut self, now: Instant) -> Vec<Control> {
        let mut out = Vec::new();
        for (c, h) in &mut self.held {
            if h.has_hold && !h.fired && now.duration_since(h.since) >= self.hold {
                h.fired = true;
                out.push(*c);
            }
        }
        out
    }

    /// When [`poll`](Self::poll) next has something to report.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.held
            .values()
            .filter(|h| h.has_hold && !h.fired)
            .map(|h| h.since + self.hold)
            .min()
    }

    pub fn forget(&mut self, control: Control) {
        self.held.remove(&control);
    }
}
