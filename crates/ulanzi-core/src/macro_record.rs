//! Turning what the recorder window sees into macro steps. Works from timestamps, so it is tested
//! without a window.

use crate::model::{LookOverride, MacroEvent, MacroStep, MouseButton, SequencePart};

/// The longest delay a recording keeps: a pause longer than this was not meant.
const MAX_DELAY_MS: u64 = 600_000;

#[derive(Debug, Clone, PartialEq)]
enum Down {
    Key(String),
    Button(MouseButton),
}

/// Collects input with its times, in milliseconds from any starting point.
#[derive(Debug, Default)]
pub struct MacroRecorder {
    steps: Vec<(MacroEvent, u64)>,
    held: Vec<Down>,
}

impl MacroRecorder {
    fn push(&mut self, event: MacroEvent, at_ms: u64) {
        self.steps.push((event, at_ms));
    }

    /// A key went down or up. Auto-repeat and a release with no press are left out.
    pub fn key(&mut self, name: &str, down: bool, at_ms: u64) {
        let what = Down::Key(name.to_string());
        let was_down = self.held.contains(&what);
        let key = name.to_string();
        match (down, was_down) {
            (true, false) => {
                self.held.push(what);
                self.push(MacroEvent::KeyDown { key }, at_ms);
            }
            (false, true) => {
                self.held.retain(|h| *h != what);
                self.push(MacroEvent::KeyUp { key }, at_ms);
            }
            _ => {}
        }
    }

    pub fn button(&mut self, button: MouseButton, down: bool, at_ms: u64) {
        let what = Down::Button(button);
        let was_down = self.held.contains(&what);
        match (down, was_down) {
            (true, false) => {
                self.held.push(what);
                self.push(MacroEvent::MouseDown { button }, at_ms);
            }
            (false, true) => {
                self.held.retain(|h| *h != what);
                self.push(MacroEvent::MouseUp { button }, at_ms);
            }
            _ => {}
        }
    }

    pub fn scroll(&mut self, dx: i32, dy: i32, at_ms: u64) {
        self.push(MacroEvent::Scroll { dx, dy }, at_ms);
    }

    /// The steps, each with the time until the next. Anything still down is released, last pressed
    /// first: focus can go while a key is held.
    pub fn finish(mut self) -> Vec<MacroStep> {
        let end = self.steps.last().map_or(0, |(_, at)| *at);
        for down in std::mem::take(&mut self.held).into_iter().rev() {
            let event = match down {
                Down::Key(key) => MacroEvent::KeyUp { key },
                Down::Button(button) => MacroEvent::MouseUp { button },
            };
            self.steps.push((event, end));
        }
        let times: Vec<u64> = self.steps.iter().map(|(_, at)| *at).collect();
        self.steps
            .into_iter()
            .enumerate()
            .map(|(i, (event, at))| MacroStep {
                event,
                delay_ms: times
                    .get(i + 1)
                    .map_or(0, |next| next.saturating_sub(at).min(MAX_DELAY_MS))
                    as u32,
                look: LookOverride::default(),
                part: SequencePart::Press,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(steps: &[crate::model::MacroStep]) -> Vec<String> {
        steps
            .iter()
            .map(|s| match &s.event {
                MacroEvent::KeyDown { key } => format!("+{key}"),
                MacroEvent::KeyUp { key } => format!("-{key}"),
                MacroEvent::MouseDown { button } => format!("+{button:?}"),
                MacroEvent::MouseUp { button } => format!("-{button:?}"),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn keys_become_down_and_up_with_the_time_between() {
        let mut r = MacroRecorder::default();
        r.key("ctrl", true, 1000);
        r.key("a", true, 1040);
        r.key("a", false, 1200);
        r.key("ctrl", false, 1250);
        let steps = r.finish();
        assert_eq!(keys(&steps), ["+ctrl", "+a", "-a", "-ctrl"]);
        let delays: Vec<u32> = steps.iter().map(|s| s.delay_ms).collect();
        assert_eq!(delays, [40, 160, 50, 0]);
    }

    #[test]
    fn a_held_key_does_not_repeat() {
        let mut r = MacroRecorder::default();
        r.key("a", true, 0);
        r.key("a", true, 500);
        r.key("a", true, 530);
        r.key("a", false, 600);
        let steps = r.finish();
        assert_eq!(keys(&steps), ["+a", "-a"]);
        assert_eq!(steps[0].delay_ms, 600);
    }

    #[test]
    fn a_key_still_down_at_the_end_is_released() {
        let mut r = MacroRecorder::default();
        r.key("ctrl", true, 0);
        r.button(MouseButton::Left, true, 10);
        let steps = r.finish();
        assert_eq!(keys(&steps), ["+ctrl", "+Left", "-Left", "-ctrl"]);
    }

    #[test]
    fn a_release_with_no_press_is_dropped() {
        let mut r = MacroRecorder::default();
        r.key("enter", false, 0);
        r.key("a", true, 100);
        r.key("a", false, 150);
        assert_eq!(keys(&r.finish()), ["+a", "-a"]);
    }

    #[test]
    fn scrolling_is_recorded() {
        let mut r = MacroRecorder::default();
        r.scroll(0, -2, 0);
        r.scroll(1, 0, 90);
        let steps = r.finish();
        assert_eq!(steps[0].event, MacroEvent::Scroll { dx: 0, dy: -2 });
        assert_eq!(steps[0].delay_ms, 90);
    }

    #[test]
    fn delays_stop_at_ten_minutes() {
        let mut r = MacroRecorder::default();
        r.key("a", true, 0);
        r.key("a", false, 3_600_000);
        assert_eq!(r.finish()[0].delay_ms, 600_000);
    }
}
