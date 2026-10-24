//! Who gets a key press: the focused text field or the deck canvas.
//!
//! Application accelerators fire before the focused widget, so a text field never saw Ctrl+A,
//! copy, paste, undo or Delete. The canvas shortcuts live on a capture-phase shortcut controller
//! on the window instead, and step aside whenever a text field has the focus.

use adw::prelude::*;
use gtk::glib;

/// Canvas shortcuts (accelerator, `win.` action). The context menu shows the same accelerators.
pub(crate) const CANVAS_SHORTCUTS: &[(&str, &str)] = &[
    ("<Control>z", "undo"),
    ("<Control><Shift>z", "redo"),
    ("<Control>y", "redo"),
    ("<Control>a", "select-all"),
    ("<Control>c", "copy"),
    ("<Control>v", "paste"),
    ("<Control><Shift>c", "copy-style"),
    ("<Control><Shift>v", "paste-style"),
    ("Delete", "clear"),
];

/// Keys a focused text field owns: editing, clipboard, undo, deleting and moving the cursor.
const TEXT_KEYS: &[&str] = &[
    "<Control>a",
    "<Control><Shift>a",
    "<Control>c",
    "<Control>v",
    "<Control>x",
    "<Control>z",
    "<Control><Shift>z",
    "<Control>y",
    // Not text keys elsewhere, but typing them in a field must not restyle the selected keys.
    "<Control><Shift>c",
    "<Control><Shift>v",
    "Delete",
    "BackSpace",
    "Left",
    "Right",
    "Up",
    "Down",
    "Home",
    "End",
];

/// The accelerator shown for `win.<action>` in menus.
pub(crate) fn accel_for(action: &str) -> Option<&'static str> {
    CANVAS_SHORTCUTS
        .iter()
        .find(|(_, a)| *a == action)
        .map(|(accel, _)| *accel)
}

/// Whether the window may act on `accel` when `text_focused` says a text field has the focus.
pub(crate) fn window_takes(accel: &str, text_focused: bool) -> bool {
    !(text_focused && TEXT_KEYS.contains(&accel))
}

/// Whether the focus sits in something that edits text (any `gtk::Editable` or a `gtk::TextView`,
/// or a widget inside one, such as the parts of an `adw::EntryRow`).
pub(crate) fn text_has_focus(focus: Option<&gtk::Widget>) -> bool {
    let mut w = focus.cloned();
    while let Some(widget) = w {
        if widget.is::<gtk::Editable>() || widget.is::<gtk::TextView>() {
            return true;
        }
        w = widget.parent();
    }
    false
}

/// Install the canvas shortcuts on `window`: a capture-phase controller, so they still win over
/// buttons and lists (as the old application accelerators did), but never over a text field.
pub(crate) fn install(window: &adw::ApplicationWindow) {
    let ctl = gtk::ShortcutController::new();
    ctl.set_propagation_phase(gtk::PropagationPhase::Capture);
    for (accel, action) in CANVAS_SHORTCUTS {
        let Some(trigger) = gtk::ShortcutTrigger::parse_string(accel) else {
            continue;
        };
        let name = format!("win.{action}");
        let callback = gtk::CallbackAction::new(move |w, _| {
            let focus = w.root().and_then(|r| r.focus());
            if !window_takes(accel, text_has_focus(focus.as_ref())) {
                return glib::Propagation::Proceed;
            }
            match w.activate_action(&name, None) {
                Ok(()) => glib::Propagation::Stop,
                Err(_) => glib::Propagation::Proceed,
            }
        });
        ctl.add_shortcut(gtk::Shortcut::new(Some(trigger), Some(callback)));
    }
    window.add_controller(ctl);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_EDITING: &[&str] = &[
        "<Control>a",
        "<Control>c",
        "<Control>v",
        "<Control>x",
        "<Control>z",
        "<Control><Shift>z",
        "<Control>y",
        "Delete",
        "BackSpace",
        "Left",
        "Right",
        "Up",
        "Down",
        "Home",
        "End",
    ];

    #[test]
    fn text_fields_keep_their_editing_keys() {
        for accel in TEXT_EDITING {
            assert!(!window_takes(accel, true), "{accel} must reach the text field");
        }
    }

    #[test]
    fn canvas_keeps_its_shortcuts_without_text_focus() {
        for (accel, _) in CANVAS_SHORTCUTS {
            assert!(window_takes(accel, false), "{accel} must reach the canvas");
        }
    }

    #[test]
    fn no_canvas_shortcut_fires_while_typing() {
        // Copy and paste style too: typing Ctrl+Shift+V in a label must not restyle keys.
        for (accel, action) in CANVAS_SHORTCUTS {
            assert!(
                !window_takes(accel, true),
                "{accel} ({action}) fired in a text field"
            );
        }
    }

    #[test]
    fn window_wide_shortcuts_still_work_in_text_fields() {
        for accel in ["<Control>l", "<Control>question", "<Control>comma", "<Control>o"] {
            assert!(window_takes(accel, true), "{accel}");
        }
    }

    #[test]
    fn context_menu_shows_the_canvas_accelerators() {
        assert_eq!(accel_for("copy"), Some("<Control>c"));
        assert_eq!(accel_for("paste"), Some("<Control>v"));
        assert_eq!(accel_for("copy-style"), Some("<Control><Shift>c"));
        assert_eq!(accel_for("paste-style"), Some("<Control><Shift>v"));
        assert_eq!(accel_for("clear"), Some("Delete"));
        assert_eq!(accel_for("redo"), Some("<Control><Shift>z"));
        assert_eq!(accel_for("library"), None);
    }

    #[test]
    fn canvas_accelerators_parse() {
        crate::ui::with_gtk(|| {
            for (accel, _) in CANVAS_SHORTCUTS {
                assert!(gtk::ShortcutTrigger::parse_string(accel).is_some(), "{accel}");
                assert!(gtk::accelerator_parse(*accel).is_some(), "{accel}");
            }
        });
    }

    #[test]
    fn focus_in_text_widgets_counts_as_text() {
        crate::ui::with_gtk(|| {
            let entry = gtk::Entry::new();
            assert!(text_has_focus(Some(entry.upcast_ref())));
            // The real focus widget of an Entry is its inner Text.
            let inner = entry.first_child().expect("entry has a text child");
            assert!(text_has_focus(Some(&inner)));
            assert!(text_has_focus(Some(gtk::Text::new().upcast_ref())));
            assert!(text_has_focus(Some(gtk::PasswordEntry::new().upcast_ref())));
            assert!(text_has_focus(Some(gtk::SearchEntry::new().upcast_ref())));
            let spin = gtk::SpinButton::with_range(0.0, 10.0, 1.0);
            assert!(text_has_focus(Some(spin.upcast_ref())));
            assert!(text_has_focus(Some(gtk::TextView::new().upcast_ref())));
            let row = adw::EntryRow::new();
            assert!(text_has_focus(Some(row.upcast_ref())));
            // A widget nested inside a text view (e.g. a child anchor) is still in the text.
            let tv = gtk::TextView::new();
            let child = gtk::Label::new(None);
            let anchor = tv.buffer().create_child_anchor(&mut tv.buffer().start_iter());
            tv.add_child_at_anchor(&child, &anchor);
            assert!(text_has_focus(Some(child.upcast_ref())));
        });
    }

    #[test]
    fn installed_shortcuts_skip_a_focused_text_field() {
        crate::ui::with_gtk(|| {
            use std::cell::Cell;
            use std::rc::Rc;
            let window = adw::ApplicationWindow::builder().build();
            let copies = Rc::new(Cell::new(0));
            let a = gtk::gio::SimpleAction::new("copy", None);
            let c = copies.clone();
            a.connect_activate(move |_, _| c.set(c.get() + 1));
            window.add_action(&a);
            let entry = gtk::Entry::new();
            let button = gtk::Button::with_label("key");
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.append(&entry);
            b.append(&button);
            window.set_content(Some(&b));
            install(&window);
            window.present();

            let ctl = window
                .observe_controllers()
                .into_iter()
                .filter_map(|o| o.ok().and_downcast::<gtk::ShortcutController>())
                .find(|c| c.propagation_phase() == gtk::PropagationPhase::Capture)
                .expect("capture-phase shortcut controller");
            let copy = (0..ctl.n_items())
                .filter_map(|i| ctl.item(i).and_downcast::<gtk::Shortcut>())
                .find(|s| {
                    s.trigger().is_some_and(|t| {
                        Some(t.to_str())
                            == gtk::ShortcutTrigger::parse_string("<Control>c").map(|p| p.to_str())
                    })
                })
                .expect("Ctrl+C shortcut");
            let run = |w: &gtk::Widget| {
                copy.action()
                    .expect("action")
                    .activate(gtk::ShortcutActionFlags::empty(), w, None)
            };

            assert!(button.grab_focus());
            assert!(run(window.upcast_ref()), "canvas copy runs with a button focused");
            assert_eq!(copies.get(), 1);

            assert!(entry.grab_focus());
            assert!(!run(window.upcast_ref()), "Ctrl+C must go on to the text field");
            assert_eq!(copies.get(), 1);
            window.destroy();
        });
    }

    #[test]
    fn focus_elsewhere_is_not_text() {
        crate::ui::with_gtk(|| {
            assert!(!text_has_focus(None));
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let button = gtk::Button::with_label("key");
            b.append(&button);
            assert!(!text_has_focus(Some(button.upcast_ref())));
            assert!(!text_has_focus(Some(b.upcast_ref())));
            assert!(!text_has_focus(Some(gtk::Label::new(Some("x")).upcast_ref())));
            assert!(!text_has_focus(Some(gtk::Switch::new().upcast_ref())));
        });
    }
}
