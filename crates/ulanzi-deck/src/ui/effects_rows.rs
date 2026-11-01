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
    State(usize, Effect),
    Preset(usize),
}

/// What the rows show.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub resting: Mixed<Effect>,
    pub pressed: Mixed<PressedEffect>,
    /// One entry per live state: its row's title and effect.
    pub states: Vec<(String, Effect)>,
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
pub fn state_rows(states: &[LookState], live: bool) -> Vec<(String, Effect)> {
    states
        .iter()
        .enumerate()
        .filter(|_| live)
        .map(|(n, st)| (state_title(st, n), st.look.effect.unwrap_or_default()))
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

/// Colour and Speed for `effect`; `send` gets the whole effect after each change.
fn options(out: &mut Vec<gtk::Widget>, effect: Effect, send: Rc<dyn Fn(Effect, bool)>) {
    if !shows_options(effect.kind) {
        return;
    }
    let value = Rc::new(Cell::new(effect));
    let color = adw::ActionRow::builder().title("Colour").build();
    let describe = |e: &Effect| e.color.map_or("Automatic".to_string(), |c| c.to_string());
    color.set_subtitle(&describe(&effect));
    let automatic = gtk::Button::builder()
        .label("Automatic")
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .visible(effect.color.is_some())
        .build();
    let button = {
        let (value, send, row, automatic) = (value.clone(), send.clone(), color.clone(), automatic.clone());
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
    color.add_suffix(&automatic);
    color.add_suffix(&button);
    out.push(color.upcast());

    let names: Vec<&str> = SPEEDS.iter().map(|s| s.1).collect();
    let at = SPEEDS.iter().position(|s| s.0 == effect.speed).unwrap_or(1) as u32;
    let speed = combo("Speed", &names, at, false);
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
    {
        let send = send.clone();
        row.connect_selected_notify(move |r| {
            send(
                Effect {
                    kind: kind_at(r.selected()),
                    ..shown
                },
                true,
            )
        });
    }
    out.push(row.upcast());
    options(out, shown, send);
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
        let send: Rc<dyn Fn(Effect, bool)> = Rc::new(move |e, again| commit(Edit::State(n, e), again));
        effect_row(&mut out, title, Mixed::One(*effect), send);
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
    let row = combo("When Pressed", &names, at, view.pressed == Mixed::Mixed);
    {
        let commit = commit.clone();
        row.connect_selected_notify(move |r| {
            let chosen = match r.selected() {
                0 => PressedEffect::Default,
                i => PressedEffect::Own(Effect {
                    kind: kind_at(i - 1),
                    ..own
                }),
            };
            commit(Edit::Pressed(chosen), true);
        });
    }
    out.push(row.upcast());
    if matches!(shown, PressedEffect::Own(_)) {
        let send: Rc<dyn Fn(Effect, bool)> =
            Rc::new(move |e, again| commit(Edit::Pressed(PressedEffect::Own(e)), again));
        options(&mut out, own, send);
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
        assert_eq!(rows[0], ("Effect When On".into(), Effect::of(EffectKind::Chase)));
        assert_eq!(
            rows[1].1.kind,
            EffectKind::None,
            "a state with no effect shows None"
        );
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
            states: Vec::new(),
            presets: Vec::new(),
        }
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
    fn a_plain_key_has_two_rows() {
        crate::ui::with_gtk(|| assert_eq!(titles(view()), ["Effect", "When Pressed"]));
    }

    #[test]
    fn a_real_effect_opens_its_options() {
        crate::ui::with_gtk(|| {
            let v = View {
                resting: Mixed::One(Effect::of(EffectKind::Fire)),
                ..view()
            };
            assert_eq!(titles(v), ["Effect", "Colour", "Speed", "When Pressed"]);
        });
    }

    #[test]
    fn presets_come_first_and_states_before_the_rest() {
        crate::ui::with_gtk(|| {
            let v = View {
                states: vec![("Effect When On".into(), Effect::default())],
                presets: vec!["Calm".into()],
                ..view()
            };
            assert_eq!(
                titles(v),
                [
                    "Effect Preset",
                    "Effect When On",
                    "Effect Otherwise",
                    "When Pressed"
                ]
            );
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
