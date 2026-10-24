//! One row per plugin setting, from the manifest's `[[settings]]` (secrets excepted: they use a
//! password row that writes straight into the plugin's secret store).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::plugin::{Field, Manifest, PickItem, Picker};

use crate::plugins::{
    Control, Input, Labels, control_for, item_ids, item_summary, item_value, shown_text, value_from,
    wants_labels,
};
use crate::state::State;

/// Called with the value to store (`Ok(None)` removes the key) or the message to show.
pub type OnValue = Rc<dyn Fn(Result<Option<toml::Value>, String>)>;

/// `#rrggbb` for a colour (alpha dropped).
pub fn hex_of(c: &gdk::RGBA) -> String {
    format!("#{:06x}", crate::ui::preferences::rgb_u32(c))
}

/// A stored value as a number for a spin row.
fn number_of(v: Option<&toml::Value>) -> Option<f64> {
    match v {
        Some(toml::Value::Integer(i)) => Some(*i as f64),
        Some(toml::Value::Float(f)) => Some(*f),
        _ => None,
    }
}

fn set_help(row: &impl IsA<gtk::Widget>, help: &str, tooltip: bool) {
    if help.is_empty() {
        return;
    }
    if tooltip {
        row.set_tooltip_text(Some(help));
    } else if let Some(r) = row.upcast_ref::<gtk::Widget>().downcast_ref::<adw::ActionRow>() {
        r.set_subtitle(&glib::markup_escape_text(help));
    }
}

/// A choice row's items and the selected one. A stored value that is not one of the choices is
/// shown as an extra `(current: <value>)` item, not as the first choice; picking that item writes
/// nothing (`value_from` refuses it).
pub fn combo_items(
    labels: &[String],
    values: &[String],
    current: Option<&toml::Value>,
) -> (Vec<String>, u32) {
    let mut items = labels.to_vec();
    let selected = match current {
        None => 0,
        Some(v) => match v.as_str().and_then(|s| values.iter().position(|x| x == s)) {
            Some(i) => i,
            None => {
                items.push(format!("(current: {})", shown_text(Some(v))));
                items.len() - 1
            }
        },
    };
    (items, selected as u32)
}

/// What an item row keeps after a pick: the ids, the row's subtitle and the value to store.
pub fn picked(picks: &[PickItem], multi: bool) -> (Vec<String>, String, Option<toml::Value>) {
    let ids: Vec<String> = picks.iter().map(|p| p.id.clone()).collect();
    let shown = item_summary(&ids, picks);
    let value = item_value(&ids, multi);
    (ids, shown, value)
}

/// How a text row saves.
#[derive(Clone, Copy, PartialEq)]
pub enum Save {
    /// With the apply button or Enter (settings pages: one write per edit).
    OnApply,
    /// On every change, like the built-in text rows (the key editor).
    Live,
}

/// The row of an `item` field with a picker: the stored value (labels once picked), opening the
/// plugin's list. A pick is written at once (one write per pick), on the settings page too.
fn item_row(
    state: &State,
    plugin: &str,
    f: &Field,
    picker: Picker,
    current: Option<&toml::Value>,
    on: OnValue,
) -> gtk::Widget {
    let ids = item_ids(current, picker.multi);
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(f.title()).as_str())
        .subtitle(glib::markup_escape_text(&item_summary(&ids, &[])).as_str())
        .activatable(true)
        .build();
    set_help(&row, &f.help, true);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    // The ids the row shows now: what the dialog opens with next time.
    let shown: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(ids));
    let contract = state
        .plugins
        .borrow()
        .manifests
        .get(plugin)
        .map(|m| m.contract.clone());
    if contract.is_some_and(|c| wants_labels(&shown.borrow(), &c)) {
        name_ids(state, plugin, &picker.id, &row, &shown);
    }
    let (state, plugin, field) = (state.clone(), plugin.to_string(), f.clone());
    row.connect_activated(move |r| {
        let (weak, shown2, on, multi) = (r.downgrade(), shown.clone(), on.clone(), picker.multi);
        let current = shown.borrow().clone();
        crate::ui::item_picker::open(r, &state, &plugin, &field, &picker, current, move |picks| {
            let (ids, summary, value) = picked(&picks, multi);
            if let Some(r) = weak.upgrade() {
                r.set_subtitle(&glib::markup_escape_text(&summary));
            }
            *shown2.borrow_mut() = ids;
            on(Ok(value));
        });
    });
    row.upcast()
}

/// Show the labels of the ids `row` shows once the plugin's picker names them: asked in the
/// background, once per picker for all the rows built within a few seconds
/// ([`crate::state::AppState::item_labels`]). On an error the row keeps showing the ids.
fn name_ids(
    state: &State,
    plugin: &str,
    picker: &str,
    row: &adw::ActionRow,
    shown: &Rc<RefCell<Vec<String>>>,
) {
    let render = {
        let (row, shown) = (row.downgrade(), Rc::downgrade(shown));
        move |items: &[PickItem]| {
            let (Some(row), Some(shown)) = (row.upgrade(), shown.upgrade()) else {
                return;
            };
            if !items.is_empty() {
                row.set_subtitle(&glib::markup_escape_text(&item_summary(&shown.borrow(), items)));
            }
        }
    };
    let waiter = {
        let render = render.clone();
        Box::new(move |items: &[PickItem]| render(items))
    };
    let now = std::time::Instant::now();
    let lookup = state.item_labels.borrow_mut().want(plugin, picker, now, waiter);
    match lookup {
        Labels::Known(items) => render(&items),
        Labels::Wait => {}
        Labels::Ask => {
            let weak = Rc::downgrade(state);
            let (p, k) = (plugin.to_string(), picker.to_string());
            let (plugin, picker) = (plugin.to_string(), picker.to_string());
            let done = move |answer: Option<Vec<PickItem>>| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let now = std::time::Instant::now();
                let (waiters, items) = state
                    .item_labels
                    .borrow_mut()
                    .answer(&plugin, &picker, now, answer);
                render(&items);
                for w in waiters {
                    w(&items);
                }
            };
            let Some(c) = state.client.borrow().clone() else {
                done(None);
                return;
            };
            crate::runtime::run(
                async move { c.plugin_list_items(&p, &k, "", "").await },
                move |r| done(r.ok()),
            );
        }
    }
}

/// The row that edits `f` of plugin `m`, showing `current` (the stored value, else the default).
pub fn row(
    state: &State,
    m: &Manifest,
    f: &Field,
    current: Option<&toml::Value>,
    save: Save,
    on: OnValue,
) -> gtk::Widget {
    let title = glib::markup_escape_text(f.title()).to_string();
    match control_for(f, &m.pickers, current) {
        Control::Item { picker } => item_row(state, &m.id, f, picker, current, on),
        Control::Entry | Control::Json => {
            let row = adw::EntryRow::builder()
                .title(&title)
                .text(shown_text(current))
                .show_apply_button(save == Save::OnApply)
                .build();
            set_help(&row, &f.help, true);
            let help = (!f.help.is_empty()).then(|| f.help.clone());
            let field = f.clone();
            let commit = Rc::new(move |r: &adw::EntryRow| {
                let v = value_from(&field, Input::Text(r.text().to_string()));
                if let Err(e) = &v {
                    r.add_css_class("error");
                    r.set_tooltip_text(Some(e));
                }
                on(v);
            });
            let live = save == Save::Live;
            let c = commit.clone();
            row.connect_changed(move |r| {
                if r.has_css_class("error") {
                    r.remove_css_class("error");
                    r.set_tooltip_text(help.as_deref());
                }
                if live {
                    c(r);
                }
            });
            if !live {
                row.connect_apply(move |r| commit(r));
            }
            row.upcast()
        }
        Control::Spin {
            min,
            max,
            step,
            digits,
        } => {
            let row = adw::SpinRow::with_range(min, max, step);
            row.set_title(&title);
            row.set_digits(digits);
            row.set_value(number_of(current).unwrap_or(0.0_f64.max(min).min(max)));
            set_help(&row, &f.help, false);
            let field = f.clone();
            row.connect_value_notify(move |r| on(value_from(&field, Input::Number(r.value()))));
            row.upcast()
        }
        Control::Switch => {
            let row = adw::SwitchRow::builder()
                .title(&title)
                .active(matches!(current, Some(toml::Value::Boolean(true))))
                .build();
            set_help(&row, &f.help, false);
            let field = f.clone();
            row.connect_active_notify(move |r| on(value_from(&field, Input::Bool(r.is_active()))));
            row.upcast()
        }
        Control::Combo { labels, values } => {
            let (items, selected) = combo_items(&labels, &values, current);
            let items: Vec<&str> = items.iter().map(String::as_str).collect();
            let row = adw::ComboRow::builder()
                .title(&title)
                .model(&gtk::StringList::new(&items))
                .selected(selected)
                .build();
            set_help(&row, &f.help, false);
            let field = f.clone();
            row.connect_selected_notify(move |r| {
                on(value_from(&field, Input::Choice(r.selected() as usize)))
            });
            row.upcast()
        }
        Control::Color => {
            let row = adw::ActionRow::builder().title(&title).build();
            set_help(&row, &f.help, false);
            let button =
                gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).build()));
            button.set_valign(gtk::Align::Center);
            if let Some(toml::Value::String(s)) = current
                && let Ok(c) = gdk::RGBA::parse(s.as_str())
            {
                button.set_rgba(&c);
            }
            let field = f.clone();
            button.connect_rgba_notify(move |b| on(value_from(&field, Input::Color(hex_of(&b.rgba())))));
            row.add_suffix(&button);
            row.set_activatable_widget(Some(&button));
            row.upcast()
        }
        // Secrets never come here (see the module docs); show them like a value the app can't edit.
        Control::ReadOnly | Control::Password => {
            let row = adw::ActionRow::builder()
                .title(&title)
                .subtitle("Edit this in settings.toml")
                .build();
            set_help(&row, &f.help, true);
            row.upcast()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_written_as_hex() {
        assert_eq!(hex_of(&gdk::RGBA::new(1.0, 0.0, 0.5, 1.0)), "#ff0080");
        assert_eq!(hex_of(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.3)), "#000000");
    }

    #[test]
    fn a_stored_value_that_is_not_a_choice_stays_visible() {
        let labels = vec!["Up".to_string(), "Down".to_string()];
        let values = vec!["up".to_string(), "down".to_string()];
        let s = |v: &str| toml::Value::String(v.into());
        assert_eq!(combo_items(&labels, &values, None), (labels.clone(), 0));
        assert_eq!(
            combo_items(&labels, &values, Some(&s("down"))),
            (labels.clone(), 1)
        );
        assert_eq!(
            combo_items(&labels, &values, Some(&s("sideways"))),
            (vec!["Up".into(), "Down".into(), "(current: sideways)".into()], 2)
        );
    }

    #[test]
    fn spin_rows_read_both_number_types() {
        assert_eq!(number_of(Some(&toml::Value::Integer(4455))), Some(4455.0));
        assert_eq!(number_of(Some(&toml::Value::Float(0.5))), Some(0.5));
        assert_eq!(number_of(Some(&toml::Value::String("x".into()))), None);
    }

    #[test]
    fn a_pick_shows_labels_and_stores_ids() {
        let p = |id: &str, label: &str| PickItem {
            id: id.into(),
            label: label.into(),
            subtitle: None,
            group: None,
            icon: None,
            state: None,
        };
        let (ids, shown, v) = picked(&[p("light.k", "Kitchen")], false);
        assert_eq!(ids, vec!["light.k".to_string()]);
        assert_eq!(shown, "Kitchen");
        assert_eq!(v, Some(toml::Value::String("light.k".into())));
        let (_, shown, v) = picked(&[p("a", "A"), p("b", "B")], true);
        assert_eq!(shown, "A and B");
        assert_eq!(
            v,
            Some(toml::Value::Array(vec![
                toml::Value::String("a".into()),
                toml::Value::String("b".into())
            ]))
        );
        assert_eq!(picked(&[], true), (Vec::new(), "None".to_string(), None));
    }

    #[test]
    fn item_rows_show_the_labels_their_picker_named() {
        crate::ui::with_gtk(|| {
            use ulanzi_core::model::{Model, Profile};
            let state = crate::state::AppState::new(Profile::new_default("Default", Model::D200X));
            let m = Manifest::parse(
                "id = \"ha\"\nname = \"HA\"\nversion = \"1.0.0\"\ncontract = \"1.1\"\n\
                 [[pickers]]\nid = \"entities\"\n\
                 [[settings]]\nkey = \"light\"\nkind = \"item\"\npicker = \"entities\"\n",
            )
            .expect("manifest");
            state
                .plugins
                .borrow_mut()
                .manifests
                .insert("ha".into(), m.clone());
            let value = toml::Value::String("light.kitchen".into());
            let subtitle = |state: &State| {
                let w = row(
                    state,
                    &m,
                    &m.settings[0],
                    Some(&value),
                    Save::Live,
                    Rc::new(|_| {}),
                );
                w.downcast::<adw::ActionRow>()
                    .expect("item row")
                    .subtitle()
                    .map(|s| s.to_string())
            };
            // No service to ask: the id stays.
            assert_eq!(subtitle(&state).as_deref(), Some("light.kitchen"));
            // A fresh answer names it.
            let kitchen = PickItem {
                id: "light.kitchen".into(),
                label: "Kitchen".into(),
                subtitle: None,
                group: None,
                icon: None,
                state: None,
            };
            let now = std::time::Instant::now();
            state
                .item_labels
                .borrow_mut()
                .answer("ha", "entities", now, Some(vec![kitchen]));
            assert_eq!(subtitle(&state).as_deref(), Some("Kitchen"));
        });
    }
}
