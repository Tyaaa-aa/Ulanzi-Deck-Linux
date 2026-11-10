//! The dial and key libraries, pinned dials and keys, and the lookups that put them together.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Action, Dial, Key, Profile};

/// A dial kept in the profile's library, placed on pages by link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryDial {
    pub id: Uuid,
    /// Never linked itself: `dial.library` is `None` in the library.
    #[serde(default)]
    pub dial: Dial,
}

/// A key kept in the profile's library, placed on pages by link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryKey {
    pub id: Uuid,
    /// What the tray calls it (a key's label can be hidden, empty or a live value).
    #[serde(default)]
    pub name: String,
    /// Never linked itself: `key.library` is `None` in the library.
    #[serde(default)]
    pub key: Key,
}

/// Dials and keys that stay the same on every page, by dial number (0..=2) and key number.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pinned {
    #[serde(with = "num_map", skip_serializing_if = "BTreeMap::is_empty")]
    pub dials: BTreeMap<u8, Dial>,
    #[serde(with = "num_map", skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<u8, Key>,
}

impl Pinned {
    pub fn is_empty(&self) -> bool {
        self.dials.is_empty() && self.keys.is_empty()
    }
}

/// A key as it works at one spot: what the library link resolves to, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    /// With the library's look and actions filled in; `None` for no key, or a link to nothing.
    pub key: Option<Key>,
    pub pinned: bool,
    pub library: Option<Uuid>,
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

impl Key {
    /// A name for this key in the library: its label unless it shows a live value, else its
    /// action's title.
    pub fn suggested_name(&self) -> String {
        let label = self.look.label.text.trim();
        if !label.is_empty() && !label.contains('{') {
            return label.to_string();
        }
        self.tap
            .as_ref()
            .and_then(crate::catalog::spec_for)
            .map(|s| s.title.to_string())
            .unwrap_or_else(|| "Key".into())
    }
}

impl Profile {
    pub fn library_key(&self, id: Uuid) -> Option<&LibraryKey> {
        self.key_library.iter().find(|k| k.id == id)
    }

    /// The pin for key `n`, else the page's own; a library link is followed.
    pub fn key_info(&self, page: usize, n: u8) -> KeyInfo {
        let pin = self.pinned.keys.get(&n);
        let raw = pin.or_else(|| self.pages.get(page).and_then(|p| p.keys.get(&n)));
        let Some(id) = raw.and_then(|k| k.library) else {
            return KeyInfo {
                key: raw.cloned(),
                pinned: pin.is_some(),
                library: None,
            };
        };
        KeyInfo {
            key: self.library_key(id).map(|l| Key {
                library: None,
                ..l.key.clone()
            }),
            pinned: pin.is_some(),
            library: Some(id),
        }
    }

    /// Key `n` as it works on `page`.
    pub fn key_at(&self, page: usize, n: u8) -> Option<Key> {
        self.key_info(page, n).key
    }

    /// [`Profile::key_at`] without the copy: the pin, the page's own or the library key. A linked
    /// key's own `library` field is the only part that differs from what works.
    pub fn key_ref(&self, page: usize, n: u8) -> Option<&Key> {
        let raw = self
            .pinned
            .keys
            .get(&n)
            .or_else(|| self.pages.get(page).and_then(|p| p.keys.get(&n)))?;
        match raw.library {
            Some(id) => self.library_key(id).map(|l| &l.key),
            None => Some(raw),
        }
    }

    /// Every key as it works on `page`: pins over the page's own, links followed.
    pub fn keys_at(&self, page: usize) -> BTreeMap<u8, Key> {
        let own = self.pages.get(page).map(|p| p.keys.keys()).into_iter().flatten();
        own.chain(self.pinned.keys.keys())
            .copied()
            .collect::<std::collections::BTreeSet<u8>>()
            .into_iter()
            .filter_map(|n| self.key_at(page, n).map(|k| (n, k)))
            .collect()
    }

    /// Page `page` as the deck shows it: its keys are [`Profile::keys_at`].
    pub fn shown_page(&self, page: usize) -> Option<super::Page> {
        let mut shown = self.pages.get(page)?.clone();
        shown.keys = self.keys_at(page);
        Some(shown)
    }

    /// Where a library key is placed: pages that link it, and whether a pin does.
    pub fn key_usage(&self, id: Uuid) -> Usage {
        let linked = |k: &Key| k.library == Some(id);
        Usage {
            pages: self.pages.iter().filter(|p| p.keys.values().any(linked)).count(),
            pinned: self.pinned.keys.values().any(linked),
        }
    }

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
        let keys = std::mem::take(&mut self.key_library);
        let find = |id| keys.iter().find(|l| l.id == id).map(|l| l.key.clone());
        // A link to nothing leaves no key.
        self.pinned.keys.retain(|_, k| match k.library {
            Some(id) => find(id).map(|l| *k = l).is_some(),
            None => true,
        });
        for page in &mut self.pages {
            page.keys.retain(|_, k| match k.library {
                Some(id) => find(id).map(|l| *k = l).is_some(),
                None => true,
            });
        }
        self.foreign_key_library.clear();
    }

    /// Whether anything here needs an app that knows the library: library dials, pins, dial
    /// names or links, or small buttons on pages.
    pub fn uses_library(&self) -> bool {
        !self.dial_library.is_empty()
            || !self.pinned.dials.is_empty()
            || self.pages.iter().any(|p| {
                !p.small_buttons.is_empty()
                    || p.dials.iter().any(|d| !d.name.is_empty() || d.library.is_some())
            })
    }

    /// Whether anything here needs an app that knows the key library: library keys, pinned keys
    /// or linked keys.
    pub fn uses_key_library(&self) -> bool {
        !self.key_library.is_empty()
            || !self.pinned.keys.is_empty()
            || self
                .pages
                .iter()
                .any(|p| p.keys.values().any(|k| k.library.is_some()))
    }
}

/// TOML tables need string keys; store dial and key numbers as `"0"`, `"1"`, ...
mod num_map {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, T: Serialize>(m: &BTreeMap<u8, T>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(m.iter().map(|(k, v)| (k.to_string(), v)))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<BTreeMap<u8, T>, D::Error> {
        let raw = BTreeMap::<String, T>::deserialize(d)?;
        raw.into_iter()
            .map(|(k, v)| {
                k.parse::<u8>()
                    .map(|i| (i, v))
                    .map_err(|_| serde::de::Error::custom(format!("number {k:?} is not a number")))
            })
            .collect()
    }
}
