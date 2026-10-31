//! What the macro editor shows for a macro's steps, and rearranging them. No GTK, so it is tested
//! directly.

use ulanzi_core::catalog::{describe, pretty_combo};
use ulanzi_core::model::{Macro, MacroEvent, MacroStep, MouseButton, SequencePart, Trigger};

/// Shown on the recorder.
pub const RECORD_HINT: &str = "Recording works while this window is in front. Linux stops apps from \
watching what you type in other apps, which keeps your passwords safe. Your macro still plays back \
in whatever app you're using.";

/// A chip's arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Down,
    Up,
    /// A down and up shown as one, or a step with neither.
    Both,
}

/// One chip in the editor: one step, or with standard delays a matching down and up.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub label: String,
    pub icon: &'static str,
    pub direction: Direction,
    /// The delay after it, when delays are shown and something follows.
    pub delay: Option<u32>,
    /// The steps it stands for, as indexes into the macro.
    pub steps: Vec<usize>,
}

fn button_name(b: MouseButton) -> &'static str {
    match b {
        MouseButton::Left => "Left",
        MouseButton::Right => "Right",
        MouseButton::Middle => "Middle",
        MouseButton::Back => "Back",
        MouseButton::Forward => "Forward",
    }
}

const KEYBOARD: &str = "input-keyboard-symbolic";
const MOUSE: &str = "input-mouse-symbolic";

fn step_chip(step: &MacroStep, index: usize) -> Chip {
    let chip = |label: String, icon, direction| Chip {
        label,
        icon,
        direction,
        delay: None,
        steps: vec![index],
    };
    match &step.event {
        MacroEvent::KeyDown { key } => chip(pretty_combo(key), KEYBOARD, Direction::Down),
        MacroEvent::KeyUp { key } => chip(pretty_combo(key), KEYBOARD, Direction::Up),
        MacroEvent::MouseDown { button } => chip(button_name(*button).into(), MOUSE, Direction::Down),
        MacroEvent::MouseUp { button } => chip(button_name(*button).into(), MOUSE, Direction::Up),
        MacroEvent::TypeText { text } => chip(format!("\"{text}\""), "insert-text-symbolic", Direction::Both),
        MacroEvent::Scroll { dy, .. } if *dy < 0 => chip("Scroll Up".into(), MOUSE, Direction::Both),
        MacroEvent::Scroll { dy, .. } if *dy > 0 => chip("Scroll Down".into(), MOUSE, Direction::Both),
        MacroEvent::Scroll { .. } => chip("Scroll Sideways".into(), MOUSE, Direction::Both),
        MacroEvent::MoveBy { dx, dy } => chip(format!("Move {dx}, {dy}"), MOUSE, Direction::Both),
        MacroEvent::MoveTo { .. } => chip("Go to Spot".into(), MOUSE, Direction::Both),
        MacroEvent::Wait { ms } => chip(format!("Wait {ms} ms"), "alarm-symbolic", Direction::Both),
        MacroEvent::RunMacro { .. } => chip(
            "Run Macro".into(),
            "media-playback-start-symbolic",
            Direction::Both,
        ),
        MacroEvent::Action { action } => chip(describe(action), "system-run-symbolic", Direction::Both),
    }
}

/// The steps `start` opens up to where everything it pressed is released again: keys or one button,
/// with nothing else between them. `None` when something stays down.
fn press_run(steps: &[(usize, &MacroStep)], start: usize) -> Option<usize> {
    let mut open: Vec<&str> = Vec::new();
    for (at, (_, step)) in steps.iter().enumerate().skip(start) {
        match &step.event {
            MacroEvent::KeyDown { key } => open.push(key),
            MacroEvent::KeyUp { key } => {
                let i = open.iter().position(|k| k == key)?;
                open.remove(i);
            }
            MacroEvent::MouseDown { .. } if at == start => {
                let next = steps.get(at + 1)?.1;
                return matches!((&step.event, &next.event),
                    (MacroEvent::MouseDown { button: a }, MacroEvent::MouseUp { button: b }) if a == b)
                .then_some(at + 1);
            }
            _ => return None,
        }
        if open.is_empty() {
            return Some(at);
        }
    }
    None
}

fn merged_chip(steps: &[(usize, &MacroStep)]) -> Chip {
    let indexes = steps.iter().map(|(i, _)| *i).collect();
    let pressed: Vec<String> = steps
        .iter()
        .filter_map(|(_, s)| match &s.event {
            MacroEvent::KeyDown { key } => Some(key.clone()),
            _ => None,
        })
        .collect();
    let (label, icon) = match steps.first().map(|(_, s)| &s.event) {
        Some(MacroEvent::MouseDown { button }) => (format!("{} Click", button_name(*button)), MOUSE),
        _ => (pretty_combo(&pressed.join("+")), KEYBOARD),
    };
    Chip {
        label,
        icon,
        direction: Direction::Both,
        delay: None,
        steps: indexes,
    }
}

/// The chips of one part of a macro. Without a Sequence every step is in the Press part.
pub fn chips(m: &Macro, part: SequencePart) -> Vec<Chip> {
    let steps: Vec<(usize, &MacroStep)> = m
        .steps
        .iter()
        .enumerate()
        .filter(|(_, s)| m.trigger != Trigger::Sequence || s.part == part)
        .collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < steps.len() {
        match press_run(&steps, at).filter(|_| m.standard_delay_ms.is_some()) {
            Some(end) => {
                out.push(merged_chip(&steps[at..=end]));
                at = end + 1;
            }
            None => {
                let (index, step) = steps[at];
                let mut chip = step_chip(step, index);
                let waits = !matches!(step.event, MacroEvent::Wait { .. });
                if m.standard_delay_ms.is_none() && at + 1 < steps.len() && waits {
                    chip.delay = Some(step.delay_ms);
                }
                out.push(chip);
                at += 1;
            }
        }
    }
    out
}

/// Move chip `from` to position `to` among the chips of `part`.
pub fn move_chip(m: &mut Macro, part: SequencePart, from: usize, to: usize) {
    let list = chips(m, part);
    let Some(chip) = list.get(from) else {
        return;
    };
    if from == to {
        return;
    }
    let mut moving = Vec::new();
    for i in chip.steps.iter().rev() {
        moving.insert(0, m.steps.remove(*i));
    }
    let after = chips(m, part);
    let at = match after.get(to).and_then(|c| c.steps.first()) {
        Some(first) => *first,
        None => after
            .last()
            .and_then(|c| c.steps.last())
            .map_or(m.steps.len(), |last| last + 1),
    };
    for (offset, step) in moving.into_iter().enumerate() {
        m.steps.insert(at + offset, step);
    }
}

/// Delete chip `index` of `part`, with every step it stands for.
pub fn remove_chip(m: &mut Macro, part: SequencePart, index: usize) {
    if let Some(chip) = chips(m, part).get(index) {
        for i in chip.steps.iter().rev() {
            m.steps.remove(*i);
        }
    }
}

/// The one sentence on a type card.
pub fn trigger_sentence(t: &Trigger) -> &'static str {
    match t {
        Trigger::Once { .. } => "Plays once each press.",
        Trigger::WhileHeld { .. } => "Repeats while you hold the key.",
        Trigger::Toggle => "Repeats until you press the key again.",
        Trigger::Sequence => "Separate steps for press, hold and release.",
    }
}

/// The warning under the steps when a macro ends with something held down.
pub fn held_warning(m: &Macro) -> Option<String> {
    let held = m.end_state();
    (!held.is_empty()).then(|| {
        let names: Vec<String> = held.iter().map(|k| pretty_combo(k)).collect();
        format!("This macro leaves {} held down", names.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulanzi_core::model::{LookOverride, Macro, MacroEvent, MacroStep, SequencePart, Trigger};

    fn step(event: MacroEvent, delay_ms: u32) -> MacroStep {
        MacroStep {
            event,
            delay_ms,
            look: LookOverride::default(),
            part: SequencePart::Press,
        }
    }

    fn key(name: &str, down: bool, delay: u32) -> MacroStep {
        let key = name.to_string();
        let e = if down {
            MacroEvent::KeyDown { key }
        } else {
            MacroEvent::KeyUp { key }
        };
        step(e, delay)
    }

    fn ctrl_e_then_text() -> Macro {
        let mut m = Macro::new("M");
        m.steps = vec![
            key("ctrl", true, 17),
            key("e", true, 18),
            key("e", false, 16),
            key("ctrl", false, 240),
            step(MacroEvent::TypeText { text: "hi".into() }, 0),
        ];
        m
    }

    fn labels(m: &Macro) -> Vec<String> {
        chips(m, SequencePart::Press)
            .iter()
            .map(|c| c.label.clone())
            .collect()
    }

    #[test]
    fn each_step_shows_with_its_delay() {
        let m = ctrl_e_then_text();
        let c = chips(&m, SequencePart::Press);
        assert_eq!(c.len(), 5);
        assert_eq!(c[0].label, "Ctrl");
        assert_eq!(c[0].direction, Direction::Down);
        assert_eq!(c[0].delay, Some(17));
        assert_eq!(c[2].direction, Direction::Up);
        assert_eq!(c[4].delay, None, "nothing follows the last step");
    }

    #[test]
    fn standard_delays_show_a_matching_down_and_up_as_one_chip() {
        let mut m = ctrl_e_then_text();
        m.standard_delay_ms = Some(50);
        assert_eq!(labels(&m), ["Ctrl+E", "\"hi\""]);
        let c = chips(&m, SequencePart::Press);
        assert_eq!(c[0].direction, Direction::Both);
        assert_eq!(c[0].delay, None);
    }

    #[test]
    fn a_key_left_down_is_not_merged() {
        let mut m = Macro::new("M");
        m.steps = vec![key("ctrl", true, 0), key("a", true, 0), key("a", false, 0)];
        m.standard_delay_ms = Some(50);
        assert_eq!(labels(&m), ["Ctrl", "A"]);
    }

    #[test]
    fn turning_standard_delays_off_shows_each_step_again() {
        let mut m = ctrl_e_then_text();
        m.standard_delay_ms = Some(50);
        m.standard_delay_ms = None;
        assert_eq!(chips(&m, SequencePart::Press).len(), 5);
        assert_eq!(m.steps[3].delay_ms, 240);
    }

    #[test]
    fn a_merged_chip_moves_as_one() {
        let mut m = ctrl_e_then_text();
        m.standard_delay_ms = Some(50);
        move_chip(&mut m, SequencePart::Press, 1, 0);
        assert_eq!(labels(&m), ["\"hi\"", "Ctrl+E"]);
        assert_eq!(m.steps.len(), 5);
    }

    #[test]
    fn a_chip_moves_to_the_end() {
        let mut m = ctrl_e_then_text();
        move_chip(&mut m, SequencePart::Press, 0, 4);
        assert!(matches!(&m.steps[4].event, MacroEvent::KeyDown { key } if key == "ctrl"));
    }

    #[test]
    fn deleting_a_merged_chip_removes_its_steps() {
        let mut m = ctrl_e_then_text();
        m.standard_delay_ms = Some(50);
        remove_chip(&mut m, SequencePart::Press, 0);
        assert_eq!(m.steps.len(), 1);
    }

    #[test]
    fn sequence_parts_stay_apart() {
        let mut m = ctrl_e_then_text();
        m.trigger = Trigger::Sequence;
        m.steps[4].part = SequencePart::Held;
        assert_eq!(chips(&m, SequencePart::Press).len(), 4);
        assert_eq!(chips(&m, SequencePart::Held).len(), 1);
        move_chip(&mut m, SequencePart::Press, 3, 0);
        assert_eq!(m.steps[4].part, SequencePart::Held);
    }

    #[test]
    fn leaving_sequence_keeps_every_step_in_view() {
        let mut m = ctrl_e_then_text();
        m.trigger = Trigger::Sequence;
        m.steps[4].part = SequencePart::ReleaseKey;
        m.trigger = Trigger::Toggle;
        assert_eq!(chips(&m, SequencePart::Press).len(), 5);
        remove_chip(&mut m, SequencePart::Press, 4);
        assert_eq!(m.steps.len(), 4);
    }

    #[test]
    fn each_type_has_a_plain_sentence() {
        let once = Trigger::default();
        let held = Trigger::WhileHeld {
            on_release: ulanzi_core::model::Release::FinishPass,
        };
        for t in [once, held, Trigger::Toggle, Trigger::Sequence] {
            assert!(trigger_sentence(&t).ends_with('.'));
        }
    }

    #[test]
    fn the_recorder_says_why_it_stays_in_its_window() {
        assert!(RECORD_HINT.starts_with("Recording works while this window is in front."));
        assert!(RECORD_HINT.contains("which keeps your passwords safe"));
        assert!(RECORD_HINT.ends_with("Your macro still plays back in whatever app you're using."));
    }

    #[test]
    fn a_macro_left_holding_something_says_so() {
        let mut m = Macro::new("M");
        m.steps = vec![key("ctrl", true, 0)];
        assert_eq!(
            held_warning(&m).as_deref(),
            Some("This macro leaves Ctrl held down")
        );
        m.steps.push(key("ctrl", false, 0));
        assert_eq!(held_warning(&m), None);
    }
}
