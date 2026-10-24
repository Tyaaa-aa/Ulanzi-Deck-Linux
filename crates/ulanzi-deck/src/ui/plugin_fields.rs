//! One row per plugin setting, from the manifest's `[[settings]]` (secrets excepted: they use a
//! password row that writes straight into the plugin's secret store).

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::plugin::Field;

use crate::plugins::{Control, Input, control_for, shown_text, value_from};

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

/// How a text row saves.
#[derive(Clone, Copy, PartialEq)]
pub enum Save {
    /// With the apply button or Enter (settings pages: one write per edit).
    OnApply,
    /// On every change, like the built-in text rows (the key editor).
    Live,
}

/// The row that edits `f`, showing `current` (the stored value, else the default).
pub fn row(f: &Field, current: Option<&toml::Value>, save: Save, on: OnValue) -> gtk::Widget {
    let title = glib::markup_escape_text(f.title()).to_string();
    match control_for(f, current) {
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
}
