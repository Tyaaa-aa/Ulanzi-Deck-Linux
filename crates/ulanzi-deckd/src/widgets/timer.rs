//! The countdown timer (with an optional Pomodoro cycle: 25 min focus, 5 min break).

use std::time::{Duration, Instant};

use ulanzi_core::render::screen::TimerData;

const BREAK_MIN: u32 = 5;

#[derive(Debug, Clone)]
pub struct TimerState {
    total: Duration,
    /// Time left when it was last paused or reset.
    left: Duration,
    /// When it was started, if running.
    started: Option<Instant>,
    pomodoro: bool,
    on_break: bool,
    focus_min: u32,
}

impl TimerState {
    pub fn new(minutes: u32, pomodoro: bool) -> TimerState {
        let total = Duration::from_secs(minutes.max(1) as u64 * 60);
        TimerState {
            total,
            left: total,
            started: None,
            pomodoro,
            on_break: false,
            focus_min: minutes.max(1),
        }
    }

    /// Keep the running state but take new settings (after an edit).
    pub fn configure(&mut self, minutes: u32, pomodoro: bool) {
        if minutes.max(1) != self.focus_min || pomodoro != self.pomodoro {
            *self = TimerState::new(minutes, pomodoro);
        }
    }

    fn remaining(&self, now: Instant) -> Duration {
        match self.started {
            Some(s) => self.left.saturating_sub(now.saturating_duration_since(s)),
            None => self.left,
        }
    }

    pub fn start_pause(&mut self, now: Instant) {
        match self.started {
            Some(_) => {
                self.left = self.remaining(now);
                self.started = None;
            }
            None => {
                if self.left.is_zero() {
                    self.left = self.total;
                }
                self.started = Some(now);
            }
        }
    }

    pub fn reset(&mut self) {
        let (m, p) = (self.focus_min, self.pomodoro);
        *self = TimerState::new(m, p);
    }

    pub fn add(&mut self, minutes: u32, now: Instant) {
        let extra = Duration::from_secs(minutes as u64 * 60);
        self.left = self.remaining(now) + extra;
        if self.started.is_some() {
            self.started = Some(now);
        }
        self.total = self.total.max(self.left);
    }

    /// Advance; returns the screen data and whether it just finished (to alert the user).
    pub fn tick(&mut self, now: Instant) -> (TimerData, bool) {
        let mut finished = false;
        if self.started.is_some() && self.remaining(now).is_zero() {
            finished = true;
            if self.pomodoro {
                // Switch phase and keep going.
                self.on_break = !self.on_break;
                let min = if self.on_break { BREAK_MIN } else { self.focus_min };
                self.total = Duration::from_secs(min as u64 * 60);
                self.left = self.total;
                self.started = Some(now);
            } else {
                self.left = Duration::ZERO;
                self.started = None;
            }
        }
        let data = TimerData {
            remaining_s: self.remaining(now).as_secs_f32().ceil() as u32,
            total_s: self.total.as_secs() as u32,
            running: self.started.is_some(),
            phase: self
                .pomodoro
                .then(|| if self.on_break { "Break" } else { "Focus" }.to_string()),
        };
        (data, finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_counts_down_and_pomodoro_switches() {
        let t0 = Instant::now();
        let mut t = TimerState::new(1, false);
        assert_eq!(t.tick(t0).0.remaining_s, 60);
        t.start_pause(t0);
        assert_eq!(t.tick(t0 + Duration::from_secs(15)).0.remaining_s, 45);
        t.start_pause(t0 + Duration::from_secs(15));
        assert_eq!(t.tick(t0 + Duration::from_secs(100)).0.remaining_s, 45, "paused");
        t.add(1, t0 + Duration::from_secs(100));
        assert_eq!(t.tick(t0 + Duration::from_secs(100)).0.remaining_s, 105);
        t.start_pause(t0 + Duration::from_secs(100));
        let (d, done) = t.tick(t0 + Duration::from_secs(300));
        assert!(done && d.remaining_s == 0 && !d.running);
        t.reset();
        assert_eq!(t.tick(t0).0.remaining_s, 60);

        let mut p = TimerState::new(25, true);
        p.start_pause(t0);
        let (d, done) = p.tick(t0 + Duration::from_secs(25 * 60));
        assert!(done);
        assert_eq!(d.phase.as_deref(), Some("Break"));
        assert_eq!(d.total_s, 5 * 60);
        assert!(d.running);
    }
}
