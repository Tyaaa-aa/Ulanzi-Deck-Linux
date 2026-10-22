use std::time::Duration;

use tokio::time::Instant;

/// Dims the deck after a period without input.
#[derive(Debug)]
pub struct SleepTimer {
    after: Option<Duration>,
    last: Instant,
    asleep: bool,
}

impl SleepTimer {
    /// `after_min == 0` means never sleep.
    pub fn new(after_min: u32, now: Instant) -> SleepTimer {
        let after = (after_min > 0).then(|| Duration::from_secs(after_min as u64 * 60));
        SleepTimer {
            after,
            last: now,
            asleep: false,
        }
    }

    pub fn touch(&mut self, now: Instant) {
        self.last = now;
    }

    pub fn should_sleep(&self, now: Instant) -> bool {
        !self.asleep && self.after.is_some_and(|a| now.duration_since(self.last) >= a)
    }

    pub fn sleep(&mut self) {
        self.asleep = true;
    }

    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    pub fn wake(&mut self, now: Instant) {
        self.asleep = false;
        self.last = now;
    }

    pub fn never(&self) -> bool {
        self.after.is_none()
    }
}
