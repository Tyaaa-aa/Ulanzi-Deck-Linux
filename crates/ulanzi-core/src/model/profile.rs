use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::foreign::{ForeignSlot, Slot};
use super::{Action, Background, KeyLook, LibraryDial, MediaTakeover, Pinned, Rotation, Widget};
use crate::Color;

/// The file format version written. 2 (Ulanzi Deck 2.1): OBS lives in the OBS plugin; older
/// files are upgraded when read (see [`crate::upgrade`]).
pub const FORMAT_VERSION: u32 = 2;
/// Screen keys are 0..=12.
pub const KEY_COUNT: u8 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Model {
    D200,
    D200X,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub id: Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub model: Model,
    #[serde(default)]
    pub palette: Vec<Color>,
    #[serde(default, rename = "match")]
    pub r#match: MatchRule,
    #[serde(default)]
    pub small_buttons: SmallButtons,
    /// Dials kept here and placed on pages by link.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dial_library: Vec<LibraryDial>,
    /// Dials that are the same on every page.
    #[serde(default, skip_serializing_if = "Pinned::is_empty")]
    pub pinned: Pinned,
    /// Before dials were per page they lived here; kept only to read old files (moved onto every
    /// page by [`Profile::normalize`]).
    #[serde(default, skip_serializing_if = "no_dials")]
    pub dials: [Dial; 3],
    #[serde(default)]
    pub screen: ScreenConfig,
    pub pages: Vec<Page>,
    /// The oldest app that reads all of this profile. Written only when a profile needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app: Option<String>,
    /// Parts from a newer version, kept as written. The store puts them back on save.
    #[serde(skip)]
    pub foreign: Vec<ForeignSlot>,
    /// Library dials from a newer version, kept as written.
    #[serde(skip)]
    pub foreign_library: Vec<toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MatchRule {
    pub apps: Vec<String>,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SmallButtons {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Action>,
}

impl SmallButtons {
    pub fn is_empty(&self) -> bool {
        self.left.is_none() && self.right.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Dial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub press: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<Action>,
    /// Shown in the app; library dials always have one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The library dial this one follows; its own actions and name are then ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Uuid>,
}

/// `Custom` holds actions that can carry plugin params, which makes it larger than the others; one per profile, so boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenConfig {
    FirmwareClock {
        #[serde(default)]
        style: ClockStyle,
    },
    FirmwareStats,
    Custom {
        #[serde(deserialize_with = "super::screen::widgets_compat")]
        widgets: Vec<Widget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tap: Option<Action>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold: Option<Action>,
        #[serde(default)]
        rotate: Rotation,
        #[serde(default)]
        background: Background,
        #[serde(default = "super::screen::default_accent")]
        accent: Color,
        /// Show Now Playing while music plays, then go back to the widgets.
        #[serde(default, skip_serializing_if = "MediaTakeover::is_off")]
        media: MediaTakeover,
    },
}

impl ScreenConfig {
    /// Widget mode with the default rotation, background and accent.
    pub fn custom(widgets: Vec<Widget>, tap: Option<Action>, hold: Option<Action>) -> ScreenConfig {
        ScreenConfig::Custom {
            widgets,
            tap,
            hold,
            rotate: Rotation::Off,
            background: Background::Theme,
            accent: super::screen::default_accent(),
            media: MediaTakeover::default(),
        }
    }
}

impl Default for ScreenConfig {
    fn default() -> Self {
        ScreenConfig::FirmwareClock {
            style: ClockStyle::Classic,
        }
    }
}

/// The deck's built-in clock layouts (drawn by the firmware over the wide-screen image).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockStyle {
    /// Analogue dial and digital time.
    #[default]
    Classic,
    DateWeekday,
    Weekday,
    Date,
    Large,
}

impl ClockStyle {
    pub const ALL: [ClockStyle; 5] = [
        ClockStyle::Classic,
        ClockStyle::DateWeekday,
        ClockStyle::Weekday,
        ClockStyle::Date,
        ClockStyle::Large,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ClockStyle::Classic => "Clock with dial",
            ClockStyle::DateWeekday => "Clock, date and weekday",
            ClockStyle::Weekday => "Clock and weekday",
            ClockStyle::Date => "Clock and date",
            ClockStyle::Large => "Large clock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    #[default]
    Page,
    Folder,
}

fn no_dials(d: &[Dial; 3]) -> bool {
    d.iter().all(|d| *d == Dial::default())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub kind: PageKind,
    #[serde(default, with = "key_map")]
    pub keys: BTreeMap<u8, Key>,
    /// What the three dials do on this page.
    #[serde(default, skip_serializing_if = "no_dials")]
    pub dials: [Dial; 3],
    /// This page's own small buttons, used where the profile pins none.
    #[serde(default, skip_serializing_if = "SmallButtons::is_empty")]
    pub small_buttons: SmallButtons,
}

impl Page {
    /// How many keys play a GIF or an effect at rest, which the deck's small CPU feels.
    pub fn moving_keys(&self) -> usize {
        let moves = |k: &Key| {
            let gif = matches!(&k.look.source, super::ImageSource::Image { path } if path.to_lowercase().ends_with(".gif"));
            gif || !k.look.effect.is_none()
        };
        self.keys.values().filter(|k| moves(k)).count()
    }

    pub fn new(name: &str) -> Page {
        Page {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: PageKind::Page,
            keys: BTreeMap::new(),
            dials: Default::default(),
            small_buttons: SmallButtons::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Key {
    pub look: KeyLook,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tap: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<Action>,
}

impl Key {
    /// Whether holding this key shows anything: an action's default effect, or one chosen for it.
    pub fn shows_press(&self) -> bool {
        match self.look.pressed {
            super::PressedEffect::Own(e) => !e.is_none(),
            super::PressedEffect::Default => self.tap.is_some() || self.hold.is_some(),
        }
    }

    /// The look before any press: a cycling key shows its last step, or its first when it shows
    /// the next one.
    pub fn shown_look(&self) -> KeyLook {
        self.live_look(&LiveKey::default())
    }

    /// The look at a moment the service told the app about.
    pub fn live_look(&self, live: &LiveKey) -> KeyLook {
        let mut look = self.look.clone();
        if let Some(Action::Cycle { steps, show }) = &self.tap
            && !steps.is_empty()
        {
            let at = live.step.unwrap_or_else(|| show.step(0, steps.len()));
            if let Some(step) = steps.get(at) {
                step.look.apply(&mut look);
            }
        }
        if let Some(Action::SwitchOutput { outputs, .. }) = &self.tap
            && let Some(o) = live.step.and_then(|at| outputs.values().nth(at))
        {
            o.look.apply(&mut look);
        }
        if look.dynamic
            && let Some(state) = live.state.and_then(|at| self.live_states().into_iter().nth(at))
        {
            state.look.apply(&mut look);
        }
        look
    }

    /// The live states this key follows: its own, else those its plugin action declares.
    pub fn live_states(&self) -> Vec<super::LookState> {
        match &self.tap {
            Some(Action::Plugin { plugin, action, .. }) if self.look.states.is_empty() => {
                crate::catalog::plugin_states(plugin, action)
            }
            _ => self.look.states.clone(),
        }
    }
}

/// Where a key stands right now, which only the service knows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveKey {
    /// The cycle step it shows; for a Switch Output key, which of its outputs (in the order
    /// they are stored) is the default output.
    pub step: Option<usize>,
    /// The live state that holds.
    pub state: Option<usize>,
}

/// Where the keys of the page on the deck stand. Keys left out have nothing live going on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveKeys {
    pub profile: Uuid,
    pub page: usize,
    pub keys: BTreeMap<u8, LiveKey>,
}

impl Profile {
    /// "Hide All Labels" for labels drawn into the key pictures (the deck hides its own).
    pub fn hide_drawn_labels(&mut self) {
        for page in &mut self.pages {
            for k in page.keys.values_mut() {
                if k.look.label.position.is_some() {
                    k.look.label.visible = false;
                }
            }
        }
    }

    /// Every action in the profile (keys, dials, small buttons, the screen; multi-action steps
    /// included), in that order.
    pub fn actions(&self) -> Vec<&Action> {
        fn add<'a>(out: &mut Vec<&'a Action>, a: &'a Option<Action>) {
            if let Some(a) = a {
                push(out, a);
            }
        }
        fn push<'a>(out: &mut Vec<&'a Action>, a: &'a Action) {
            out.push(a);
            match a {
                Action::Multi { steps } => steps.iter().for_each(|s| push(out, &s.action)),
                Action::Cycle { steps, .. } => steps.iter().for_each(|s| push(out, &s.action)),
                Action::Macro { own: Some(m), .. } => m.actions().for_each(|a| push(out, a)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for page in &self.pages {
            for k in page.keys.values() {
                add(&mut out, &k.tap);
                add(&mut out, &k.hold);
            }
            for d in &page.dials {
                for a in [&d.press, &d.left, &d.right, &d.hold] {
                    add(&mut out, a);
                }
            }
            add(&mut out, &page.small_buttons.left);
            add(&mut out, &page.small_buttons.right);
        }
        for d in self
            .dial_library
            .iter()
            .map(|l| &l.dial)
            .chain(self.pinned.dials.values())
        {
            for a in [&d.press, &d.left, &d.right, &d.hold] {
                add(&mut out, a);
            }
        }
        add(&mut out, &self.small_buttons.left);
        add(&mut out, &self.small_buttons.right);
        if let ScreenConfig::Custom { tap, hold, .. } = &self.screen {
            add(&mut out, tap);
            add(&mut out, hold);
        }
        out
    }

    /// The library macros this profile's keys use directly.
    pub fn macros_used(&self) -> BTreeSet<Uuid> {
        let mut out = BTreeSet::new();
        for a in self.actions() {
            match a {
                Action::Macro { own: Some(m), .. } => out.extend(m.runs()),
                Action::Macro { id, own: None } => {
                    out.insert(*id);
                }
                _ => {}
            }
        }
        out
    }

    /// Whether any key uses a macro.
    pub fn uses_macros(&self) -> bool {
        self.actions()
            .iter()
            .any(|a| matches!(a, Action::Macro { .. } | Action::StopMacros))
    }

    /// The ids of the plugins this profile's actions use: everywhere [`Profile::actions`] looks,
    /// plus old profile-wide dials not yet moved onto the pages.
    pub fn plugins_used(&self) -> BTreeSet<String> {
        fn add(out: &mut BTreeSet<String>, a: &Action) {
            match a {
                Action::Plugin { plugin, .. } => {
                    out.insert(plugin.clone());
                }
                Action::Multi { steps } => steps.iter().for_each(|s| add(out, &s.action)),
                Action::Cycle { steps, .. } => steps.iter().for_each(|s| add(out, &s.action)),
                _ => {}
            }
        }
        let mut out = BTreeSet::new();
        // `actions()` already lists multi-action steps; `add` also walks them for the old dials.
        for a in self.actions() {
            add(&mut out, a);
        }
        for d in &self.dials {
            for a in [&d.press, &d.left, &d.right, &d.hold].into_iter().flatten() {
                add(&mut out, a);
            }
        }
        out
    }
}

impl Profile {
    /// Every key back to the label settings from Preferences: shown, at the default position.
    pub fn clear_label_overrides(&mut self) {
        for key in self.pages.iter_mut().flat_map(|p| p.keys.values_mut()) {
            key.look.label.position = None;
            key.look.label.visible = true;
        }
    }

    /// A profile with one page "Main" and the small buttons flipping pages.
    pub fn new_default(name: &str, model: Model) -> Profile {
        Profile {
            version: FORMAT_VERSION,
            id: Uuid::new_v4(),
            name: name.into(),
            icon: None,
            model,
            palette: Vec::new(),
            r#match: MatchRule::default(),
            small_buttons: SmallButtons {
                left: Some(Action::PrevPage),
                right: Some(Action::NextPage),
            },
            dial_library: Vec::new(),
            pinned: Pinned::default(),
            dials: Default::default(),
            screen: ScreenConfig::default(),
            pages: vec![Page::new("Main")],
            min_app: None,
            foreign: Vec::new(),
            foreign_library: Vec::new(),
        }
    }

    /// Repair what a hand edit can break. Returns one warning per repair.
    pub fn normalize(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.pages.is_empty() {
            self.pages.push(Page::new("Main"));
            warnings.push(format!(
                "profile \"{}\" had no pages; added an empty \"Main\" page",
                self.name
            ));
        }
        // Dials used to be one set per profile: they are the pinned ones now.
        if !no_dials(&self.dials) {
            let old = std::mem::take(&mut self.dials);
            for (i, d) in old.into_iter().enumerate() {
                if !d.is_empty() {
                    self.pinned.dials.entry(i as u8).or_insert(d);
                }
            }
        }
        let bad: Vec<u8> = self.pinned.dials.keys().copied().filter(|i| *i >= 3).collect();
        for i in bad {
            self.pinned.dials.remove(&i);
            warnings.push(format!("removed pinned dial {i} (dials are 0-2)"));
        }
        // A dial that is the same on every page is a pinned one. Only for profiles that have none
        // yet, so a dial the user unpins is not pinned again.
        if self.pinned.is_empty() && self.pages.len() >= 2 {
            for i in 0..3usize {
                let first = &self.pages[0].dials[i];
                let same = !first.is_empty() && self.pages.iter().all(|p| &p.dials[i] == first);
                let blocked = self
                    .foreign
                    .iter()
                    .any(|f| f.slot == Slot::Dial(i as u8) && f.page.is_some());
                if same && !blocked {
                    let d = std::mem::take(&mut self.pages[0].dials[i]);
                    for p in &mut self.pages[1..] {
                        p.dials[i] = Dial::default();
                    }
                    self.pinned.dials.insert(i as u8, d);
                }
            }
        }
        for page in &mut self.pages {
            let bad: Vec<u8> = page.keys.keys().copied().filter(|i| *i >= KEY_COUNT).collect();
            for i in bad {
                page.keys.remove(&i);
                warnings.push(format!("page \"{}\": removed key {i} (keys are 0-12)", page.name));
            }
        }
        warnings
    }
}

/// TOML tables need string keys; store key indices as `"0"`..`"12"`.
mod key_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serializer};

    use super::Key;

    pub fn serialize<S: Serializer>(m: &BTreeMap<u8, Key>, s: S) -> Result<S::Ok, S::Error> {
        // BTreeMap<u8, _> iterates numerically, so the file lists keys 0, 1, 2, ... 12.
        s.collect_map(m.iter().map(|(k, v)| (k.to_string(), v)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<u8, Key>, D::Error> {
        let raw = BTreeMap::<String, Key>::deserialize(d)?;
        raw.into_iter()
            .map(|(k, v)| {
                k.parse::<u8>()
                    .map(|i| (i, v))
                    .map_err(|_| serde::de::Error::custom(format!("key index {k:?} is not a number")))
            })
            .collect()
    }
}
