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

/// The row that edits `f`, showing `current` (the stored value, else the default).
pub fn row(f: &Field, current: Option<&toml::Value>, on: OnValue) -> gtk::Widget {
    let title = glib::markup_escape_text(f.title()).to_string();
    match control_for(f, current) {
        Control::Entry | Control::Json => {
            let row = adw::EntryRow::builder()
                .title(&title)
                .text(shown_text(current))
                .show_apply_button(true)
                .build();
            set_help(&row, &f.help, true);
            let help = (!f.help.is_empty()).then(|| f.help.clone());
            row.connect_changed(move |r| {
                if r.has_css_class("error") {
                    r.remove_css_class("error");
                    r.set_tooltip_text(help.as_deref());
                }
            });
            let field = f.clone();
            row.connect_apply(move |r| {
                let v = value_from(&field, Input::Text(r.text().to_string()));
                if let Err(e) = &v {
                    r.add_css_class("error");
                    r.set_tooltip_text(Some(e));
                }
                on(v);
            });
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
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let selected = match current {
                Some(toml::Value::String(s)) => values.iter().position(|v| v == s).unwrap_or(0),
                _ => 0,
            };
            let row = adw::ComboRow::builder()
                .title(&title)
                .model(&gtk::StringList::new(&labels))
                .selected(selected as u32)
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
    fn spin_rows_read_both_number_types() {
        assert_eq!(number_of(Some(&toml::Value::Integer(4455))), Some(4455.0));
        assert_eq!(number_of(Some(&toml::Value::Float(0.5))), Some(0.5));
        assert_eq!(number_of(Some(&toml::Value::String("x".into()))), None);
    }
}
