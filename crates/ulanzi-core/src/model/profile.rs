use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Action, KeyLook};
use crate::Color;

pub const FORMAT_VERSION: u32 = 1;
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
    /// Before dials were per page they lived here; kept only to read old files (moved onto every
    /// page by [`Profile::normalize`]).
    #[serde(default, skip_serializing_if = "no_dials")]
    pub dials: [Dial; 3],
    #[serde(default)]
    pub screen: ScreenConfig,
    pub pages: Vec<Page>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScreenConfig {
    FirmwareClock {
        #[serde(default)]
        style: ClockStyle,
    },
    FirmwareStats,
    Custom {
        widgets: Vec<Widget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tap: Option<Action>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold: Option<Action>,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    Blank,
    NowPlaying,
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
}

impl Page {
    pub fn new(name: &str) -> Page {
        Page {
            id: Uuid::new_v4(),
            name: name.into(),
            kind: PageKind::Page,
            keys: BTreeMap::new(),
            dials: Default::default(),
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
            dials: Default::default(),
            screen: ScreenConfig::default(),
            pages: vec![Page::new("Main")],
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
        // Dials used to be one set per profile: give every page that set.
        if !no_dials(&self.dials) {
            let old = std::mem::take(&mut self.dials);
            for page in &mut self.pages {
                if no_dials(&page.dials) {
                    page.dials = old.clone();
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
