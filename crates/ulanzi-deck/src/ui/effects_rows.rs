//! The effect rows of a key's Look section.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use ulanzi_core::edit::Mixed;
use ulanzi_core::model::{Effect, EffectKind, EffectSpeed, LookState, PressedEffect};

use crate::ui::color_popover::color_button;

const KINDS: [(EffectKind, &str); 5] = [
    (EffectKind::None, "None"),
    (EffectKind::Glow, "Glow"),
    (EffectKind::Pulse, "Pulse"),
    (EffectKind::Chase, "Chase"),
    (EffectKind::Fire, "Fire"),
];
const SPEEDS: [(EffectSpeed, &str); 3] = [
    (EffectSpeed::Slow, "Slow"),
    (EffectSpeed::Normal, "Normal"),
    (EffectSpeed::Fast, "Fast"),
];

/// A change the rows ask for. The bool beside it says the rows themselves must be built again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edit {
    Resting(Effect),
    Pressed(PressedEffect),
    /// `None`: same as the Otherwise row.
    State(usize, Option<Effect>),
    Preset(usize),
}

/// What the rows show.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub resting: Mixed<Effect>,
    pub pressed: Mixed<PressedEffect>,
    /// The Default look of a held key is a Pulse here, not nothing.
    pub default_pulses: bool,
    /// One entry per live state: its row's title and effect.
    pub states: Vec<(String, Option<Effect>)>,
    pub presets: Vec<String>,
}

fn title_case(s: &str) -> String {
    let mut chars = s.trim().chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The row title of a live state: `Effect When On`.
pub fn state_title(st: &LookState, n: usize) -> String {
    match (st.when.is.first(), st.when.not.first()) {
        (Some(v), _) => format!("Effect When {}", title_case(v)),
        (None, Some(v)) => format!("Effect When Not {}", title_case(v)),
        (None, None) => format!("Effect For State {}", n + 1),
    }
}

/// The states' rows, when Show Live State is on and one key is selected.
pub fn state_rows(states: &[LookState], live: bool) -> Vec<(String, Option<Effect>)> {
    states
        .iter()
        .enumerate()
        .filter(|_| live)
        .map(|(n, st)| (state_title(st, n), st.look.effect))
        .collect()
}

/// Colour and Speed mean something only for a real effect.
pub fn shows_options(kind: EffectKind) -> bool {
    kind != EffectKind::None
}

fn kind_at(i: u32) -> EffectKind {
    KINDS.get(i as usize).map_or(EffectKind::None, |k| k.0)
}

fn kind_index(kind: EffectKind) -> u32 {
    KINDS.iter().position(|k| k.0 == kind).unwrap_or(0) as u32
}

fn combo(title: &str, names: &[&str], selected: u32, mixed: bool) -> adw::ComboRow {
    let row = adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(names))
        .selected(selected)
        .build();
    if mixed {
        row.set_subtitle("Mixed");
    }
    row
}

fn kind_names() -> Vec<&'static str> {
    KINDS.iter().map(|k| k.1).collect()
}

/// The colour on the effect's own row, then a Speed row under it. `send` gets the whole effect after
/// each change.
fn options(
    out: &mut Vec<gtk::Widget>,
    row: &adw::ComboRow,
    value: Rc<Cell<Effect>>,
    send: Rc<dyn Fn(Effect, bool)>,
) {
    let effect = value.get();
    if !shows_options(effect.kind) {
        return;
    }
    let describe = |e: &Effect| e.color.map_or("Automatic colour".to_string(), |c| c.to_string());
    row.set_subtitle(&describe(&effect));
    let automatic = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Back to the automatic colour")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .visible(effect.color.is_some())
        .build();
    let button = {
        let (value, send, row, automatic) = (value.clone(), send.clone(), row.clone(), automatic.clone());
        color_button(
            effect.color,
            |_| {},
            move |c| {
                let e = Effect {
                    color: Some(c),
                    ..value.get()
                };
                value.set(e);
                row.set_subtitle(&describe(&e));
                automatic.set_visible(true);
                send(e, false);
            },
        )
    };
    {
        let (value, send) = (value.clone(), send.clone());
        automatic.connect_clicked(move |_| {
            let e = Effect {
                color: None,
                ..value.get()
            };
            value.set(e);
            send(e, true);
        });
    }
    row.add_suffix(&automatic);
    row.add_suffix(&button);

    let names: Vec<&str> = SPEEDS.iter().map(|s| s.1).collect();
    let at = SPEEDS.iter().position(|s| s.0 == effect.speed).unwrap_or(1) as u32;
    let speed = combo("Speed", &names, at, false);
    speed.add_css_class("dependent-row");
    speed.connect_selected_notify(move |r| {
        let e = Effect {
            speed: SPEEDS
                .get(r.selected() as usize)
                .map_or(EffectSpeed::Normal, |s| s.0),
            ..value.get()
        };
        value.set(e);
        send(e, false);
    });
    out.push(speed.upcast());
}

/// One line on what the effects are, for the section while it is closed.
pub fn summary(view: &View) -> String {
    let name = |kind| KINDS.iter().find(|k| k.0 == kind).map_or("None", |k| k.1);
    let resting = match view.resting {
        Mixed::One(e) if !e.is_none() => Some(format!("{} at rest", name(e.kind))),
        Mixed::Mixed => Some("Mixed".to_string()),
        _ => None,
    };
    let live = view
        .states
        .iter()
        .any(|s| s.1.is_some())
        .then(|| "By state".to_string());
    let pressed = match view.pressed {
        Mixed::One(PressedEffect::Default) if view.default_pulses => Some("Pulse when held".to_string()),
        Mixed::One(PressedEffect::Own(e)) if !e.is_none() => Some(format!("{} when held", name(e.kind))),
        _ => None,
    };
    let parts: Vec<String> = [live.or(resting), pressed].into_iter().flatten().collect();
    if parts.is_empty() {
        "None".to_string()
    } else {
        parts.join(", ")
    }
}

/// A resting effect changed to `kind`. A new one starts slow: every moving key costs the deck, and
/// slower ones cost less.
fn moved_to(effect: Effect, kind: EffectKind) -> Effect {
    let new = effect.is_none() && kind != EffectKind::None;
    let speed = if new { EffectSpeed::Slow } else { effect.speed };
    Effect {
        kind,
        speed,
        ..effect
    }
}

fn effect_row(
    out: &mut Vec<gtk::Widget>,
    title: &str,
    effect: Mixed<Effect>,
    send: Rc<dyn Fn(Effect, bool)>,
) {
    let shown = match effect {
        Mixed::One(e) => e,
        _ => Effect::default(),
    };
    let row = combo(
        title,
        &kind_names(),
        kind_index(shown.kind),
        effect == Mixed::Mixed,
    );
    let value = Rc::new(Cell::new(shown));
    {
        let (send, value) = (send.clone(), value.clone());
        row.connect_selected_notify(move |r| send(moved_to(value.get(), kind_at(r.selected())), true));
    }
    out.push(row.clone().upcast());
    options(out, &row, value, send);
}

/// A live state's effect: its own, or whatever Otherwise says.
fn state_row(
    out: &mut Vec<gtk::Widget>,
    title: &str,
    effect: Option<Effect>,
    send: Rc<dyn Fn(Option<Effect>, bool)>,
) {
    let mut names = vec!["Same as Otherwise"];
    names.extend(kind_names());
    let at = effect.map_or(0, |e| 1 + kind_index(e.kind));
    let row = combo(title, &names, at, false);
    let value = Rc::new(Cell::new(effect.unwrap_or_default()));
    {
        let (send, value) = (send.clone(), value.clone());
        row.connect_selected_notify(move |r| {
            let chosen = (r.selected() > 0).then(|| moved_to(value.get(), kind_at(r.selected() - 1)));
            send(chosen, true);
        });
    }
    out.push(row.clone().upcast());
    if effect.is_some() {
        options(out, &row, value, Rc::new(move |e, again| send(Some(e), again)));
    }
}

/// The rows for `view`, in order.
pub fn rows(view: View, commit: Rc<dyn Fn(Edit, bool)>) -> Vec<gtk::Widget> {
    let mut out = Vec::new();
    if !view.presets.is_empty() {
        let mut names = vec!["Choose…"];
        names.extend(view.presets.iter().map(String::as_str));
        let row = combo("Effect Preset", &names, 0, false);
        let commit = commit.clone();
        row.connect_selected_notify(move |r| {
            if r.selected() > 0 {
                commit(Edit::Preset(r.selected() as usize - 1), true);
            }
        });
        out.push(row.upcast());
    }
    for (n, (title, effect)) in view.states.iter().enumerate() {
        let commit = commit.clone();
        state_row(
            &mut out,
            title,
            *effect,
            Rc::new(move |e, again| commit(Edit::State(n, e), again)),
        );
    }
    let title = if view.states.is_empty() {
        "Effect"
    } else {
        "Effect Otherwise"
    };
    let send: Rc<dyn Fn(Effect, bool)> = {
        let commit = commit.clone();
        Rc::new(move |e, again| commit(Edit::Resting(e), again))
    };
    effect_row(&mut out, title, view.resting, send);

    let mut names = vec!["Default"];
    names.extend(kind_names());
    let shown = match view.pressed {
        Mixed::One(p) => p,
        _ => PressedEffect::Default,
    };
    let (at, own) = match shown {
        PressedEffect::Default => (0, Effect::default()),
        PressedEffect::Own(e) => (1 + kind_index(e.kind), e),
    };
    let row = combo("When Held", &names, at, view.pressed == Mixed::Mixed);
    let value = Rc::new(Cell::new(own));
    {
        let (commit, value) = (commit.clone(), value.clone());
        row.connect_selected_notify(move |r| {
            let chosen = match r.selected() {
                0 => PressedEffect::Default,
                i => PressedEffect::Own(Effect {
                    kind: kind_at(i - 1),
                    ..value.get()
                }),
            };
            commit(Edit::Pressed(chosen), true);
        });
    }
    out.push(row.clone().upcast());
    if matches!(shown, PressedEffect::Own(_)) {
        let send: Rc<dyn Fn(Effect, bool)> =
            Rc::new(move |e, again| commit(Edit::Pressed(PressedEffect::Own(e)), again));
        options(&mut out, &row, value, send);
    }
    out
}

#[cfg(test)]
mod tests {
    use ulanzi_core::model::{LookOverride, When};

    use super::*;

    fn state(is: &[&str], not: &[&str], effect: Option<Effect>) -> LookState {
        LookState {
            when: When {
                value: "{x}".into(),
                is: is.iter().map(|s| s.to_string()).collect(),
                not: not.iter().map(|s| s.to_string()).collect(),
            },
            look: LookOverride {
                effect,
                ..LookOverride::default()
            },
        }
    }

    #[test]
    fn a_state_is_named_from_its_condition() {
        assert_eq!(state_title(&state(&["on"], &[], None), 0), "Effect When On");
        assert_eq!(state_title(&state(&[], &["off"], None), 0), "Effect When Not Off");
        assert_eq!(state_title(&state(&[], &[], None), 2), "Effect For State 3");
    }

    #[test]
    fn the_rows_split_by_state_only_with_live_state_on() {
        let states = [
            state(&["on"], &[], Some(Effect::of(EffectKind::Chase))),
            state(&["off"], &[], None),
        ];
        assert!(state_rows(&states, false).is_empty());
        let rows = state_rows(&states, true);
        assert_eq!(
            rows[0],
            ("Effect When On".into(), Some(Effect::of(EffectKind::Chase)))
        );
        assert_eq!(rows[1].1, None, "no effect of its own");
    }

    #[test]
    fn colour_and_speed_show_only_for_a_real_effect() {
        assert!(!shows_options(EffectKind::None));
        assert!(shows_options(EffectKind::Fire));
    }

    fn view() -> View {
        View {
            resting: Mixed::One(Effect::default()),
            pressed: Mixed::One(PressedEffect::Default),
            default_pulses: true,
            states: Vec::new(),
            presets: Vec::new(),
        }
    }

    #[test]
    fn a_key_that_wont_pulse_doesnt_say_it_will() {
        let v = View {
            default_pulses: false,
            ..view()
        };
        assert_eq!(summary(&v), "None");
    }

    fn titles(v: View) -> Vec<String> {
        rows(v, Rc::new(|_, _| {}))
            .iter()
            .filter_map(|w| {
                w.downcast_ref::<adw::PreferencesRow>()
                    .map(|r| r.title().to_string())
            })
            .collect()
    }

    #[test]
    fn the_summary_says_what_a_key_does() {
        assert_eq!(summary(&view()), "Pulse when held");
        let v = View {
            resting: Mixed::One(Effect::of(EffectKind::Fire)),
            pressed: Mixed::One(PressedEffect::Own(Effect::default())),
            ..view()
        };
        assert_eq!(summary(&v), "Fire at rest");
        let v = View {
            pressed: Mixed::One(PressedEffect::Own(Effect::default())),
            ..view()
        };
        assert_eq!(summary(&v), "None");
    }

    #[test]
    fn a_plain_key_has_two_rows() {
        crate::ui::with_gtk(|| assert_eq!(titles(view()), ["Effect", "When Held"]));
    }

    #[test]
    fn a_real_effect_opens_its_options() {
        crate::ui::with_gtk(|| {
            let v = View {
                resting: Mixed::One(Effect::of(EffectKind::Fire)),
                ..view()
            };
            assert_eq!(titles(v), ["Effect", "Speed", "When Held"]);
        });
    }

    #[test]
    fn presets_come_first_and_states_before_the_rest() {
        crate::ui::with_gtk(|| {
            let v = View {
                states: vec![("Effect When On".into(), None)],
                presets: vec!["Calm".into()],
                ..view()
            };
            assert_eq!(
                titles(v),
                ["Effect Preset", "Effect When On", "Effect Otherwise", "When Held"]
            );
        });
    }

    #[test]
    fn a_state_follows_otherwise_until_it_chooses() {
        crate::ui::with_gtk(|| {
            let seen = Rc::new(Cell::new(None));
            let v = View {
                states: vec![("Effect When On".into(), None)],
                ..view()
            };
            let list = {
                let seen = seen.clone();
                rows(v, Rc::new(move |e, _| seen.set(Some(e))))
            };
            let row = list[0].downcast_ref::<adw::ComboRow>().expect("combo");
            assert_eq!(row.selected(), 0, "Same as Otherwise");
            row.set_selected(1);
            assert_eq!(seen.get(), Some(Edit::State(0, Some(Effect::default()))));
            row.set_selected(0);
            assert_eq!(seen.get(), Some(Edit::State(0, None)));
        });
    }

    #[test]
    fn a_new_resting_effect_starts_slow() {
        crate::ui::with_gtk(|| {
            let seen = Rc::new(Cell::new(None));
            let list = {
                let seen = seen.clone();
                rows(view(), Rc::new(move |e, _| seen.set(Some(e))))
            };
            list[0]
                .downcast_ref::<adw::ComboRow>()
                .expect("kind")
                .set_selected(4);
            let Some(Edit::Resting(e)) = seen.get() else {
                panic!("{:?}", seen.get());
            };
            assert_eq!((e.kind, e.speed), (EffectKind::Fire, EffectSpeed::Slow));
        });
    }

    #[test]
    fn a_new_speed_survives_choosing_another_effect() {
        crate::ui::with_gtk(|| {
            let seen = Rc::new(Cell::new(None));
            let v = View {
                resting: Mixed::One(Effect::of(EffectKind::Glow)),
                ..view()
            };
            let list = {
                let seen = seen.clone();
                rows(v, Rc::new(move |e, _| seen.set(Some(e))))
            };
            list[1]
                .downcast_ref::<adw::ComboRow>()
                .expect("speed")
                .set_selected(2);
            list[0]
                .downcast_ref::<adw::ComboRow>()
                .expect("kind")
                .set_selected(3);
            let Some(Edit::Resting(e)) = seen.get() else {
                panic!("{:?}", seen.get());
            };
            assert_eq!((e.kind, e.speed), (EffectKind::Chase, EffectSpeed::Fast));
        });
    }

    #[test]
    fn several_keys_that_differ_show_as_mixed() {
        crate::ui::with_gtk(|| {
            let v = View {
                resting: Mixed::Mixed,
                ..view()
            };
            let list = rows(v, Rc::new(|_, _| {}));
            let row = list[0].downcast_ref::<adw::ActionRow>().expect("row");
            assert_eq!(row.subtitle().as_deref(), Some("Mixed"));
        });
    }

    #[test]
    fn choosing_an_effect_asks_for_new_rows_and_keeps_colour_and_speed() {
        crate::ui::with_gtk(|| {
            let seen = Rc::new(Cell::new(None));
            let v = View {
                resting: Mixed::One(Effect {
                    kind: EffectKind::Glow,
                    color: Some(ulanzi_core::Color::WHITE),
                    speed: EffectSpeed::Fast,
                }),
                ..view()
            };
            let list = {
                let seen = seen.clone();
                rows(v, Rc::new(move |e, again| seen.set(Some((e, again)))))
            };
            list[0]
                .downcast_ref::<adw::ComboRow>()
                .expect("combo")
                .set_selected(4);
            let Some((Edit::Resting(e), true)) = seen.get() else {
                panic!("{:?}", seen.get());
            };
            assert_eq!(e.kind, EffectKind::Fire);
            assert_eq!(e.speed, EffectSpeed::Fast);
            assert_eq!(e.color, Some(ulanzi_core::Color::WHITE));
        });
    }
}
