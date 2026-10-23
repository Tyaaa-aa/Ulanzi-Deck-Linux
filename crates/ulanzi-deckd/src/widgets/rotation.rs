//! Which widget the screen shows when there are several.

use std::time::{Duration, Instant};

use ulanzi_core::model::Rotation;

#[derive(Debug, Clone)]
pub struct Rotator {
    index: usize,
    since: Instant,
}

impl Rotator {
    pub fn new(now: Instant) -> Rotator {
        Rotator { index: 0, since: now }
    }

    /// The widget to show now out of `n`.
    pub fn current(&mut self, rotate: Rotation, n: usize, now: Instant) -> usize {
        if n == 0 {
            return 0;
        }
        if let Rotation::Every { secs } = rotate
            && now.saturating_duration_since(self.since) >= Duration::from_secs(secs.max(2) as u64)
        {
            self.index += 1;
            self.since = now;
        }
        self.index %= n;
        self.index
    }

    /// Show the next one (Next Widget action, or a press with "on press").
    pub fn next(&mut self, now: Instant) {
        self.index += 1;
        self.since = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_every_and_on_press() {
        let t0 = Instant::now();
        let mut r = Rotator::new(t0);
        let every = Rotation::Every { secs: 5 };
        assert_eq!(r.current(every, 3, t0 + Duration::from_secs(4)), 0);
        assert_eq!(r.current(every, 3, t0 + Duration::from_secs(5)), 1);
        assert_eq!(r.current(every, 3, t0 + Duration::from_secs(11)), 2);
        assert_eq!(r.current(every, 3, t0 + Duration::from_secs(16)), 0);
        let mut p = Rotator::new(t0);
        assert_eq!(p.current(Rotation::OnPress, 2, t0 + Duration::from_secs(60)), 0);
        p.next(t0);
        assert_eq!(p.current(Rotation::OnPress, 2, t0), 1);
        p.next(t0);
        assert_eq!(p.current(Rotation::Off, 2, t0), 0);
        assert_eq!(p.current(Rotation::Off, 0, t0), 0);
    }
}
