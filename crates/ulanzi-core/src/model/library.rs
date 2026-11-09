//! The dial library, pinned dials and the lookups that put them together.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Action, Dial, Profile};

/// A dial kept in the profile's library, placed on pages by link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryDial {
    pub id: Uuid,
    /// Never linked itself: `dial.library` is `None` in the library.
    #[serde(default)]
    pub dial: Dial,
}

/// Dials that stay the same on every page. Keys are dial numbers 0..=2.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pinned {
    #[serde(with = "dial_map", skip_serializing_if = "BTreeMap::is_empty")]
    pub dials: BTreeMap<u8, Dial>,
}

impl Pinned {
    pub fn is_empty(&self) -> bool {
        self.dials.is_empty()
    }
}

/// A dial as it works at one spot: what the library link resolves to, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct DialInfo {
    /// With the library's actions and name filled in; blank for a link to nothing.
    pub dial: Dial,
    pub pinned: bool,
    pub library: Option<Uuid>,
}

/// Where a library dial is placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub pages: usize,
    pub pinned: bool,
}

impl Usage {
    pub fn text(&self) -> String {
        match (self.pinned, self.pages) {
            (true, _) => "Pinned".into(),
            (false, 0) => "Not placed".into(),
            (false, 1) => "1 page".into(),
            (false, n) => format!("{n} pages"),
        }
    }
}

impl Dial {
    /// No actions, no name and no link.
    pub fn is_empty(&self) -> bool {
        *self == Dial::default()
    }
}

impl Profile {
    pub fn library_dial(&self, id: Uuid) -> Option<&LibraryDial> {
        self.dial_library.iter().find(|d| d.id == id)
    }

    /// The pin for this dial, else the page's own; a library link is followed.
    pub fn dial_info(&self, page: usize, n: u8) -> DialInfo {
        let pin = self.pinned.dials.get(&n);
        let raw = pin
            .or_else(|| self.pages.get(page).and_then(|p| p.dials.get(n as usize)))
            .cloned()
            .unwrap_or_default();
        let Some(id) = raw.library else {
            return DialInfo {
                dial: raw,
                pinned: pin.is_some(),
                library: None,
            };
        };
        let dial = self
            .library_dial(id)
            .map(|l| Dial {
                library: None,
                ..l.dial.clone()
            })
            .unwrap_or_default();
        DialInfo {
            dial,
            pinned: pin.is_some(),
            library: Some(id),
        }
    }

    pub fn dial_at(&self, page: usize, n: u8) -> Dial {
        self.dial_info(page, n).dial
    }

    /// The small button on `side` (0 left, 1 right): the pin, else the page's own.
    pub fn small_at(&self, page: usize, side: u8) -> Option<Action> {
        let pick = |s: &super::SmallButtons| {
            if side == 0 {
                s.left.clone()
            } else {
                s.right.clone()
            }
        };
        pick(&self.small_buttons).or_else(|| self.pages.get(page).and_then(|p| pick(&p.small_buttons)))
    }

    pub fn small_pinned(&self, side: u8) -> bool {
        let s = &self.small_buttons;
        if side == 0 {
            s.left.is_some()
        } else {
            s.right.is_some()
        }
    }

    /// Where a library dial is placed: pages that link it, and whether a pin does.
    pub fn dial_usage(&self, id: Uuid) -> Usage {
        let linked = |d: &Dial| d.library == Some(id);
        Usage {
            pages: self.pages.iter().filter(|p| p.dials.iter().any(linked)).count(),
            pinned: self.pinned.dials.values().any(linked),
        }
    }

    /// Make every linked dial an own copy of its library dial and empty the library.
    pub fn drop_library(&mut self) {
        let library = std::mem::take(&mut self.dial_library);
        let resolve = |d: &mut Dial| {
            let Some(id) = d.library else { return };
            *d = library
                .iter()
                .find(|l| l.id == id)
                .map(|l| Dial {
                    library: None,
                    ..l.dial.clone()
                })
                .unwrap_or_default();
        };
        self.pinned.dials.values_mut().for_each(resolve);
        self.pages
            .iter_mut()
            .flat_map(|p| p.dials.iter_mut())
            .for_each(resolve);
        self.foreign_library.clear();
    }

    /// Whether anything here needs an app that knows the library: library dials, pins, dial
    /// names or links, or small buttons on pages.
    pub fn uses_library(&self) -> bool {
        !self.dial_library.is_empty()
            || !self.pinned.is_empty()
            || self.pages.iter().any(|p| {
                !p.small_buttons.is_empty()
                    || p.dials.iter().any(|d| !d.name.is_empty() || d.library.is_some())
            })
    }
}

/// TOML tables need string keys; store dial numbers as `"0"`..`"2"`.
mod dial_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serializer};

    use super::Dial;

    pub fn serialize<S: Serializer>(m: &BTreeMap<u8, Dial>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(m.iter().map(|(k, v)| (k.to_string(), v)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<u8, Dial>, D::Error> {
        let raw = BTreeMap::<String, Dial>::deserialize(d)?;
        raw.into_iter()
            .map(|(k, v)| {
                k.parse::<u8>()
                    .map(|i| (i, v))
                    .map_err(|_| serde::de::Error::custom(format!("dial number {k:?} is not a number")))
            })
            .collect()
    }
}
