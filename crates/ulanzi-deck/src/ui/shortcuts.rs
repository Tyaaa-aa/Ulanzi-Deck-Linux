//! The Keyboard Shortcuts window: every shortcut and mouse gesture in one place.

use adw::prelude::*;

/// (title, accelerator or a plain description of a mouse gesture)
const GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Keys",
        &[
            ("Copy keys", "<Ctrl>c"),
            ("Paste keys", "<Ctrl>v"),
            ("Copy style (colours, image, label)", "<Ctrl><Shift>c"),
            ("Paste style", "<Ctrl><Shift>v"),
            ("Clear the selected keys", "Delete"),
        ],
    ),
    (
        "Selecting",
        &[
            ("Select all keys", "<Ctrl>a"),
            ("Select nothing (close the key editor)", "Escape"),
            ("Add or remove a key", "Ctrl + click"),
            ("Select a range of keys", "Shift + click"),
        ],
    ),
    (
        "Mouse",
        &[
            ("Copy, paste or clear a key", "Right-click"),
            ("Swap two keys", "Drag a key onto another"),
            ("Give a key an action", "Drag from the action list"),
            ("Use a picture", "Drop an image file on a key"),
            ("Rename, move or delete a page or profile", "Right-click it"),
            ("Reorder pages or profiles", "Drag them in the sidebar"),
        ],
    ),
    (
        "General",
        &[
            ("Undo", "<Ctrl>z"),
            ("Redo", "<Ctrl><Shift>z"),
            ("Search the actions", "<Ctrl>l"),
            ("Preferences", "<Ctrl>comma"),
            ("Keyboard shortcuts", "<Ctrl>question"),
        ],
    ),
];

pub fn open(parent: &impl IsA<gtk::Widget>) {
    let page = adw::PreferencesPage::new();
    for (title, rows) in GROUPS {
        let group = adw::PreferencesGroup::builder().title(*title).build();
        for (what, how) in *rows {
            let row = adw::ActionRow::builder().title(*what).build();
            // Real accelerators render as key caps; gestures as text.
            if gtk::accelerator_parse(*how).is_some() {
                row.add_suffix(&gtk::ShortcutLabel::new(how));
            } else {
                row.add_suffix(
                    &gtk::Label::builder()
                        .label(*how)
                        .css_classes(["dim-label"])
                        .build(),
                );
            }
            group.add(&row);
        }
        page.add(&group);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    adw::Dialog::builder()
        .title("Keyboard Shortcuts")
        .content_width(520)
        .content_height(640)
        .child(&view)
        .build()
        .present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_accelerators_parse() {
        // Needs a display; skipped on headless machines.
        crate::ui::with_gtk(|| {
            // Every entry that looks like an accelerator must be one.
            for (_, rows) in GROUPS {
                for (_, how) in *rows {
                    if how.starts_with('<') {
                        assert!(gtk::accelerator_parse(*how).is_some(), "{how}");
                    }
                }
            }
        });
    }
}
