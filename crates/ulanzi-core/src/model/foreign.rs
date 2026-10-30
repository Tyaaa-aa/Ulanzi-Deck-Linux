//! Parts of a profile written by a newer version. A key is the smallest part: one that does not
//! parse is set aside as written, the rest of the profile loads, and saving puts it back, so nothing
//! is lost on the way through this version.

use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::{Action, Dial, Key, Profile, ScreenConfig};

pub use crate::edit::Slot;

/// A slot from a newer version, kept as written. Not serialized by serde: see `store`.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignSlot {
    /// `None` for the small buttons and the wide screen.
    pub page: Option<Uuid>,
    pub slot: Slot,
    pub raw: toml::Value,
}

impl Profile {
    /// The foreign part at `slot` of `page`, if there is one.
    pub fn foreign_at(&self, page: Option<Uuid>, slot: Slot) -> Option<&ForeignSlot> {
        self.foreign.iter().find(|f| f.page == page && f.slot == slot)
    }
}

fn parses<T: DeserializeOwned>(v: &toml::Value) -> bool {
    v.clone().try_into::<T>().is_ok()
}

const SMALL: [&str; 2] = ["left", "right"];

/// Take every slot that does not parse out of a profile's table and return them.
pub fn separate(table: &mut toml::Table) -> Vec<ForeignSlot> {
    let mut out = Vec::new();
    if let Some(small) = table.get_mut("small_buttons").and_then(|v| v.as_table_mut()) {
        for (i, side) in SMALL.iter().enumerate() {
            if small.get(*side).is_some_and(|v| !parses::<Action>(v))
                && let Some(raw) = small.remove(*side)
            {
                out.push(ForeignSlot {
                    page: None,
                    slot: Slot::Small(i as u8),
                    raw,
                });
            }
        }
    }
    if table.get("screen").is_some_and(|v| !parses::<ScreenConfig>(v))
        && let Some(raw) = table.remove("screen")
    {
        out.push(ForeignSlot {
            page: None,
            slot: Slot::Screen,
            raw,
        });
    }
    let pages = table.get_mut("pages").and_then(|v| v.as_array_mut());
    for page in pages.into_iter().flatten().filter_map(|p| p.as_table_mut()) {
        let id = page
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok());
        if let Some(keys) = page.get_mut("keys").and_then(|v| v.as_table_mut()) {
            let bad: Vec<String> = keys
                .iter()
                .filter(|(_, v)| !parses::<Key>(v))
                .map(|(k, _)| k.clone())
                .collect();
            for k in bad {
                if let (Ok(i), Some(raw)) = (k.parse::<u8>(), keys.get(&k).cloned()) {
                    keys.remove(&k);
                    out.push(ForeignSlot {
                        page: id,
                        slot: Slot::Key(i),
                        raw,
                    });
                }
            }
        }
        if let Some(dials) = page.get_mut("dials").and_then(|v| v.as_array_mut()) {
            for (i, dial) in dials.iter_mut().enumerate() {
                if !parses::<Dial>(dial) {
                    let raw = std::mem::replace(dial, toml::Value::Table(toml::Table::new()));
                    out.push(ForeignSlot {
                        page: id,
                        slot: Slot::Dial(i as u8),
                        raw,
                    });
                }
            }
        }
    }
    out
}

/// Put `foreign` back into a profile's table, where [`separate`] found each one.
pub fn restore(table: &mut toml::Table, foreign: &[ForeignSlot]) {
    let new_table = || toml::Value::Table(toml::Table::new());
    for f in foreign {
        match f.slot {
            Slot::Small(i) => {
                let Some(small) = table
                    .entry("small_buttons")
                    .or_insert_with(new_table)
                    .as_table_mut()
                else {
                    continue;
                };
                small.insert(SMALL[(i as usize).min(1)].into(), f.raw.clone());
            }
            Slot::Screen => {
                table.insert("screen".into(), f.raw.clone());
            }
            Slot::Key(_) | Slot::Dial(_) => {
                let id = f.page.map(|id| id.to_string());
                let page = table
                    .get_mut("pages")
                    .and_then(|v| v.as_array_mut())
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_table_mut())
                    .find(|p| p.get("id").and_then(|v| v.as_str()) == id.as_deref());
                let Some(page) = page else { continue };
                match f.slot {
                    Slot::Key(i) => {
                        if let Some(keys) = page.entry("keys").or_insert_with(new_table).as_table_mut() {
                            keys.insert(i.to_string(), f.raw.clone());
                        }
                    }
                    Slot::Dial(i) => {
                        let dials = page
                            .entry("dials")
                            .or_insert_with(|| toml::Value::Array(vec![new_table(); 3]));
                        if let Some(d) = dials.as_array_mut().and_then(|d| d.get_mut(i as usize)) {
                            *d = f.raw.clone();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
