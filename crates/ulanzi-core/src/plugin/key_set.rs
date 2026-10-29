//! Turning a key set and the items the user picked into keys.

use crate::Color;
use crate::model::{Action, ImageSource, Key, KeyLook, Label};

use super::{KeySet, KeySetVariant, Manifest, is_asset};

/// `text` with `$id` and `$name` filled in.
pub fn fill(text: &str, id: &str, name: &str) -> String {
    text.replace("$id", id).replace("$name", name)
}

fn fill_value(v: &toml::Value, id: &str, name: &str) -> toml::Value {
    match v {
        toml::Value::String(s) => toml::Value::String(fill(s, id, name)),
        toml::Value::Array(a) => toml::Value::Array(a.iter().map(|v| fill_value(v, id, name)).collect()),
        toml::Value::Table(t) => toml::Value::Table(fill_table(t, id, name)),
        other => other.clone(),
    }
}

fn fill_table(t: &toml::Table, id: &str, name: &str) -> toml::Table {
    t.iter()
        .map(|(k, v)| (k.clone(), fill_value(v, id, name)))
        .collect()
}

/// One picked item: what the action stores, the name for labels and its group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub id: String,
    pub name: String,
    pub group: Option<String>,
}

fn in_groups(groups: &[String], group: Option<&str>) -> bool {
    groups.is_empty() || group.is_some_and(|g| groups.iter().any(|x| x == g))
}

impl KeySet {
    /// The first variant for an item of `group`.
    pub fn variant_for(&self, group: Option<&str>) -> Option<&KeySetVariant> {
        self.variants.iter().find(|v| in_groups(&v.groups, group))
    }

    /// The keys for `item`, extras only when `extras`. Each key shows the key's icon (else its
    /// action's) in the plugin's colour, and its label.
    pub fn keys_for(&self, manifest: &Manifest, item: &Picked, extras: bool) -> Vec<Key> {
        let Some(variant) = self.variant_for(item.group.as_deref()) else {
            return Vec::new();
        };
        let fg = manifest
            .rgb()
            .map(|[r, g, b]| Color([r, g, b, 255]))
            .unwrap_or(Color::WHITE);
        variant
            .keys
            .iter()
            .filter(|k| extras || !k.extra)
            .map(|k| {
                let icon = k
                    .icon
                    .clone()
                    .or_else(|| manifest.action(&k.action).and_then(|a| a.icon.clone()))
                    .filter(|i| is_asset(i));
                Key {
                    look: KeyLook {
                        source: icon.map_or(ImageSource::None, |icon| ImageSource::Plugin {
                            plugin: manifest.id.clone(),
                            icon,
                        }),
                        foreground: k
                            .color
                            .as_deref()
                            .and_then(super::manifest::parse_color)
                            .map_or(fg, |[r, g, b]| Color([r, g, b, 255])),
                        label: Label {
                            text: fill(&k.label, &item.id, &item.name),
                            ..Label::default()
                        },
                        // Only the key's own looks; otherwise it uses its action's when drawn.
                        states: k.states.iter().map(|st| st.to_look(&manifest.id)).collect(),
                        ..KeyLook::default()
                    },
                    tap: Some(Action::Plugin {
                        plugin: manifest.id.clone(),
                        action: k.action.clone(),
                        params: fill_table(&k.params, &item.id, &item.name),
                    }),
                    hold: None,
                }
            })
            .collect()
    }

    /// The (setting, value) pairs to add for `item`, so its live state is published.
    pub fn tracking(&self, item: &Picked) -> Vec<(String, String)> {
        self.track
            .iter()
            .filter(|t| in_groups(&t.groups, item.group.as_deref()))
            .map(|t| (t.setting.clone(), fill(&t.value, &item.id, &item.name)))
            .collect()
    }
}

/// Names short enough for a key: a leading phrase that two or more of `names` share ("Bedroom
/// Floor Lamp", "Bedroom Fan" → "Floor Lamp", "Fan") is dropped, the longest such phrase first.
/// A name is never emptied.
pub fn short_names(names: &[String]) -> Vec<String> {
    let words: Vec<Vec<&str>> = names.iter().map(|n| n.split_whitespace().collect()).collect();
    words
        .iter()
        .zip(names)
        .map(|(w, name)| {
            // The longest prefix (leaving at least one word) that another name starts with too.
            let cut = (1..w.len())
                .rev()
                .find(|&n| {
                    words
                        .iter()
                        .filter(|o| {
                            o.len() > n && o[..n].iter().zip(&w[..n]).all(|(a, b)| a.eq_ignore_ascii_case(b))
                        })
                        .count()
                        >= 2
                })
                .unwrap_or(0);
            if cut == 0 {
                name.trim().to_string()
            } else {
                w[cut..].join(" ")
            }
        })
        .collect()
}
