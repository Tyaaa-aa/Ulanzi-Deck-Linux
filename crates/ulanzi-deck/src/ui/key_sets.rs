//! Ready-made keys from a plugin's `[[key_sets]]`: pick items (lights, fans, scenes…) in the
//! plugin's picker, then each one gets the keys that fit it, on this page or a new one. The
//! picked items are added to the settings the key set names, so the keys show live state.

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::model::Key;
use ulanzi_core::plugin::key_set::{Picked, short_names};
use ulanzi_core::plugin::{Field, FieldKind, KeySet, Manifest, PickItem};

use crate::state::State;

/// Most items one run adds.
const MAX_ITEMS: f64 = 40.0;

/// Start: pick items for key set `set` of `plugin`, then confirm where the keys go.
pub fn start(state: &State, parent: &impl IsA<gtk::Widget>, plugin: &str, set: &str) {
    let Some(m) = state.plugins.borrow().manifests.get(plugin).cloned() else {
        return;
    };
    let Some(ks) = m.key_set(set).cloned() else {
        return;
    };
    let Some(picker) = m.picker(&ks.picker).cloned() else {
        return;
    };
    let field = Field {
        key: ks.id.clone(),
        kind: FieldKind::Item,
        label: ks.name.clone(),
        picker: Some(ks.picker.clone()),
        max: Some(MAX_ITEMS),
        ..Field::default()
    };
    let (state2, parent2) = (state.clone(), parent.clone().upcast::<gtk::Widget>());
    crate::ui::item_picker::open(parent, state, plugin, &field, &picker, Vec::new(), move |picks| {
        if !picks.is_empty() {
            confirm(&state2, &parent2, &m, &ks, picks);
        }
    });
}

/// The picked entries as key set items, with names short enough for a key.
pub fn picked(picks: &[PickItem]) -> Vec<Picked> {
    let names: Vec<String> = picks.iter().map(|p| p.label.clone()).collect();
    short_names(&names)
        .into_iter()
        .zip(picks)
        .map(|(name, p)| Picked {
            id: p.id.clone(),
            name,
            group: p.group.clone(),
        })
        .collect()
}

/// The keys for every item (one group per item), extras when `extras`.
pub fn key_groups(m: &Manifest, ks: &KeySet, items: &[Picked], extras: bool) -> Vec<Vec<Key>> {
    items
        .iter()
        .map(|i| ks.keys_for(m, i, extras))
        .filter(|g| !g.is_empty())
        .collect()
}

fn count(groups: &[Vec<Key>]) -> usize {
    groups.iter().map(Vec::len).sum()
}

fn heading(n: usize) -> String {
    if n == 1 {
        "Add 1 Key?".into()
    } else {
        format!("Add {n} Keys?")
    }
}

fn confirm(state: &State, parent: &gtk::Widget, m: &Manifest, ks: &KeySet, picks: Vec<PickItem>) {
    let items = picked(&picks);
    let has_extras = items.iter().any(|i| {
        ks.variant_for(i.group.as_deref())
            .is_some_and(|v| v.keys.iter().any(|k| k.extra))
    });
    let free = state.editor.borrow().free_keys();
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    let alert = adw::AlertDialog::builder()
        .body(format!("For {}.", names.join(", ")))
        .default_response("here")
        .close_response("cancel")
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("new", "On a New Page");
    alert.add_response("here", "On This Page");
    alert.set_response_appearance("here", adw::ResponseAppearance::Suggested);
    let extras = adw::SwitchRow::builder()
        .title("Brightness, Speed and Temperature Keys")
        .subtitle("Turned off, each device gets one key")
        .active(true)
        .build();
    if has_extras {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        list.append(&extras);
        alert.set_extra_child(Some(&list));
    }
    let update = {
        let (alert, m, ks, items) = (alert.clone(), m.clone(), ks.clone(), items.clone());
        move |extras: bool| {
            let n = count(&key_groups(&m, &ks, &items, extras));
            alert.set_heading(Some(&heading(n)));
            // This page only when they all fit there.
            alert.set_response_enabled("here", n <= free);
            alert.set_default_response(Some(if n <= free { "here" } else { "new" }));
        }
    };
    update(true);
    extras.connect_active_notify(move |r| update(r.is_active()));
    let (state, m, ks) = (state.clone(), m.clone(), ks.clone());
    alert.connect_response(None, move |_, response| {
        let new_page = match response {
            "here" => false,
            "new" => true,
            _ => return,
        };
        let groups = key_groups(&m, &ks, &items, !has_extras || extras.is_active());
        let n = count(&groups);
        let page = {
            let name = m.name.clone();
            let mut first = 0;
            state.edit(|e| first = e.add_keys(groups, &name, new_page));
            first
        };
        state.set_page(page);
        track(&state, &m, &ks, &items);
        crate::ui::toast(&state, &format!("Added {n} keys"));
    });
    alert.present(Some(parent));
}

/// Add the picked items to the settings the key set names, so their state is published.
fn track(state: &State, m: &Manifest, ks: &KeySet, items: &[Picked]) {
    let adds: Vec<(String, String, String)> = items
        .iter()
        .flat_map(|i| ks.tracking(i))
        .map(|(k, v)| (m.id.clone(), k, v))
        .collect();
    add_tracked(state, adds);
}

/// What the keys of `profile` need published: for each plugin action, its `track` entries and
/// those of the label value it shows, with its parameters filled in. (plugin, setting, value).
pub fn needed(
    profile: &ulanzi_core::model::Profile,
    manifests: &std::collections::BTreeMap<String, Manifest>,
) -> Vec<(String, String, String)> {
    use ulanzi_core::model::{Action, fill_params};
    let mut out = Vec::new();
    for page in &profile.pages {
        for key in page.keys.values() {
            let Some(Action::Plugin {
                plugin,
                action,
                params,
            }) = &key.tap
            else {
                continue;
            };
            let Some(a) = manifests.get(plugin).and_then(|m| m.action(action)) else {
                continue;
            };
            let label = a.labels.iter().filter(|l| l.text == key.look.label.text);
            for t in a.track.iter().chain(label.flat_map(|l| &l.track)) {
                let value = fill_params(&t.value, params);
                // A parameter not chosen yet leaves its `$name`: nothing to track.
                if !value.contains('$') && !value.trim().is_empty() {
                    out.push((plugin.clone(), t.setting.clone(), value));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Add what the profile's keys need to their plugins' settings (never removes anything).
pub fn track_profile(state: &State, profile: &ulanzi_core::model::Profile) {
    let adds = needed(profile, &state.plugins.borrow().manifests);
    add_tracked(state, adds);
}

/// Add (plugin, setting, value) entries to the plugins' settings; sends the settings only when
/// one was missing. Says so once when a list is full.
fn add_tracked(state: &State, adds: Vec<(String, String, String)>) {
    if adds.is_empty() {
        return;
    }
    let manifests = state.plugins.borrow().manifests.clone();
    let missing: Vec<(String, String, String)> = {
        let s = state.settings.borrow();
        adds.into_iter()
            .filter(|(p, key, value)| {
                let mut t = s.plugins.get(p).map(|e| e.settings.clone()).unwrap_or_default();
                let Some(f) = manifests
                    .get(p)
                    .and_then(|m| m.settings.iter().find(|f| &f.key == key))
                else {
                    return false;
                };
                let before = t.clone();
                add_setting(&mut t, f, value) && t != before
            })
            .collect()
    };
    if missing.is_empty() {
        return;
    }
    let full = std::rc::Rc::new(std::cell::Cell::new(false));
    let full2 = full.clone();
    crate::ui::preferences::change_boxed(
        state,
        Box::new(move |s| {
            for (p, key, value) in missing {
                let Some(f) = manifests
                    .get(&p)
                    .and_then(|m| m.settings.iter().find(|f| f.key == key))
                else {
                    continue;
                };
                let table = &mut s.plugins.entry(p).or_default().settings;
                full2.set(full2.get() | !add_setting(table, f, &value));
            }
        }),
    );
    if full.get() {
        crate::ui::toast(
            state,
            "Some live states are not shown: a plugin's list of followed items is full",
        );
    }
}

/// Add `value` to setting `f` in `table`: to the list of a multi item setting (up to its `max`),
/// or as a comma-separated entry of a text setting. False when the list is full.
pub fn add_setting(table: &mut toml::Table, f: &Field, value: &str) -> bool {
    let entry = table.get(&f.key).cloned();
    match f.kind {
        FieldKind::Item => {
            let mut list: Vec<toml::Value> = match entry {
                Some(toml::Value::Array(a)) => a,
                Some(toml::Value::String(s)) if !s.is_empty() => vec![toml::Value::String(s)],
                _ => Vec::new(),
            };
            if list.iter().any(|v| v.as_str() == Some(value)) {
                return true;
            }
            if f.max.is_some_and(|max| list.len() as f64 >= max) {
                return false;
            }
            list.push(toml::Value::String(value.to_string()));
            table.insert(f.key.clone(), toml::Value::Array(list));
            true
        }
        _ => {
            let text = entry
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let mut parts: Vec<String> = text
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            if !parts.iter().any(|p| p == value) {
                parts.push(value.to_string());
            }
            table.insert(f.key.clone(), toml::Value::String(parts.join(", ")));
            true
        }
    }
}

/// The plugin's key sets, for the action list.
pub fn sets_of(state: &State, plugin: &str) -> Vec<KeySet> {
    state
        .plugins
        .borrow()
        .manifests
        .get(plugin)
        .map(|m| m.key_sets.clone())
        .unwrap_or_default()
}

/// A key set's icon, tinted for the current theme.
pub fn set_icon(plugin: &str, ks: &KeySet, size: i32) -> gtk::Widget {
    let png = ks.icon.as_deref().and_then(|i| {
        let fg = adw::StyleManager::default().is_dark();
        let c = if fg { [255, 255, 255, 230] } else { [0, 0, 0, 200] };
        ulanzi_core::render::plugin_icon_png(
            plugin,
            i,
            u32::try_from(size * 2).unwrap_or(48),
            Some(ulanzi_core::Color(c)),
        )
    });
    match png.and_then(|p| gtk::gdk::Texture::from_bytes(&glib::Bytes::from(&p)).ok()) {
        Some(t) => {
            let i = gtk::Image::from_paintable(Some(&t));
            i.set_pixel_size(size);
            i.upcast()
        }
        None => gtk::Image::from_icon_name("view-grid-symbolic").upcast(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(kind: &str, max: Option<f64>) -> Field {
        Field {
            key: "k".into(),
            kind: FieldKind::from(kind.to_string()),
            max,
            ..Field::default()
        }
    }

    #[test]
    fn tracked_items_are_added_once_and_within_the_limit() {
        let mut t = toml::Table::new();
        let f = field("item", Some(2.0));
        assert!(add_setting(&mut t, &f, "light.a"));
        assert!(add_setting(&mut t, &f, "light.a"));
        assert!(add_setting(&mut t, &f, "light.b"));
        assert!(!add_setting(&mut t, &f, "light.c"), "full");
        assert_eq!(t["k"].as_array().map(Vec::len), Some(2));
        let mut t = toml::Table::new();
        t.insert("k".into(), "climate.x:temperature".into());
        let f = field("text", None);
        assert!(add_setting(&mut t, &f, "climate.y:temperature"));
        assert!(add_setting(&mut t, &f, "climate.x:temperature"));
        assert_eq!(
            t["k"].as_str(),
            Some("climate.x:temperature, climate.y:temperature")
        );
    }

    #[test]
    fn picked_items_get_short_names() {
        let p = |id: &str, label: &str| PickItem {
            id: id.into(),
            label: label.into(),
            subtitle: None,
            group: Some("Lights".into()),
            icon: None,
            state: None,
        };
        let items = picked(&[
            p("light.a", "Bedroom Floor Lamp"),
            p("light.b", "Bedroom Bed Light"),
        ]);
        assert_eq!(items[0].name, "Floor Lamp");
        assert_eq!(items[1].name, "Bed Light");
        assert_eq!(items[1].group.as_deref(), Some("Lights"));
    }
}
