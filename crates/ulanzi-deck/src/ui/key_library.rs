//! The key library in the inspector: the library row of a key on the deck, and the parts of a
//! library key's editor (picked in the Library tray, edited in full at `LIBRARY_KEY`) that a deck
//! key does not have: its name, where it is used, and Delete.

use adw::prelude::*;
use ulanzi_core::model::*;
use uuid::Uuid;

use crate::state::State;
use crate::ui::action_editor::How;
use crate::ui::dial_inspector::{ask_delete, hidden_by_pin, name_row, usage_line, used_on};
use crate::ui::inspector::C;

/// The line under a key's title: the page, then where it comes from.
pub fn key_subtitle(page: &str, info: &KeyInfo, usage: Option<Usage>) -> String {
    let mut s = page.to_string();
    if info.library.is_some() {
        s.push_str(" · From your library");
        if let Some(u) = usage {
            s.push_str(&format!(" · {}", used_on(u.pages)));
        }
    }
    if info.pinned {
        s.push_str(" · Pinned to every page");
    }
    s
}

/// What a key's library and pin buttons are: (label, action) pairs, for the key as it is.
pub fn library_buttons(info: &KeyInfo) -> Vec<(&'static str, &'static str)> {
    // Nothing here, or a link to a library key that is gone.
    if info.key.is_none() {
        return Vec::new();
    }
    let lib = if info.library.is_some() {
        ("Make a Copy", "own-copy")
    } else {
        ("Add to Library", "add-to-library")
    };
    let pin = if info.pinned {
        ("Unpin", "unpin")
    } else {
        ("Pin to Every Page", "pin")
    };
    vec![lib, pin]
}

/// The library and pin buttons under a key's title, and a note when a pin covers this page's own
/// key. `None` for an empty key.
pub(super) fn library_row(ctx: &C, k: u8) -> Option<gtk::Box> {
    let (info, covered) = {
        let e = ctx.state.editor.borrow();
        let p = e.profile();
        let info = e.key_info(k);
        // Only when unpinning would show something else.
        let covered = p.covered_key(e.page(), k);
        (info, covered)
    };
    let buttons = library_buttons(&info);
    if buttons.is_empty() {
        return None;
    }
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    let row = gtk::Box::builder().spacing(6).homogeneous(true).build();
    for (label, action) in buttons {
        let b = gtk::Button::builder().label(label).build();
        let ctx = ctx.clone();
        b.connect_clicked(move |_| {
            ctx.commit(How::Structure, |e| match action {
                "own-copy" => e.own_copy_key(k),
                "add-to-library" => {
                    e.add_key_to_library(k);
                }
                "pin" => e.pin_key(k),
                _ => e.unpin_key(k),
            })
        });
        row.append(&b);
    }
    column.append(&row);
    if let Some(own) = covered {
        let picture =
            crate::ui::canvas::key_texture(&own.shown_look(), &crate::ui::canvas::profile_dir(&ctx.state));
        column.append(&hidden_by_pin(
            "key",
            picture,
            &own.suggested_name(),
            &[("Press", own.tap.as_ref()), ("Hold", own.hold.as_ref())],
        ));
    }
    Some(column)
}

/// The line under a library key's title.
pub fn library_subtitle(usage: Option<Usage>) -> String {
    match usage {
        Some(u) => format!("From your library · {}", usage_line(u)),
        None => "From your library".into(),
    }
}

/// The Name row of the library key being edited, renaming `title` as it is typed.
pub(super) fn name_entry(ctx: &C, title: &gtk::Box) -> Option<gtk::Widget> {
    let (id, name) = {
        let e = ctx.state.editor.borrow();
        let id = e.edited_library_key()?;
        (id, e.profile().library_key(id)?.name.clone())
    };
    // The title block's first label is the title.
    let title = title.first_child().and_downcast::<gtk::Label>()?;
    let ctx = ctx.clone();
    Some(
        name_row("New Key", &name, &title, move |t| {
            ctx.commit(How::Typing("library-key-name"), |e| e.rename_library_key(id, t))
        })
        .upcast(),
    )
}

/// Delete from Library, under the library key's editor.
pub(super) fn delete_button(state: &State) -> Option<gtk::Widget> {
    let id = state.editor.borrow().edited_library_key()?;
    let delete = gtk::Button::builder()
        .label("Delete From Library")
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    let state = state.clone();
    delete.connect_clicked(move |b| delete_library_keys(&state, vec![id], b));
    Some(delete.upcast())
}

/// Delete library keys `ids`, asking first (over `parent`) what happens to the keys using them.
pub(crate) fn delete_library_keys(state: &State, ids: Vec<Uuid>, parent: &impl IsA<gtk::Widget>) {
    let (names, uses) = {
        let e = state.editor.borrow();
        let p = e.profile();
        let names: Vec<String> = ids
            .iter()
            .filter_map(|id| p.library_key(*id).map(|l| l.name.clone()))
            .collect();
        (names, ids.iter().map(|id| p.key_uses(*id)).sum())
    };
    if names.is_empty() {
        return;
    }
    let keep = ids.clone();
    ask_delete(
        state,
        parent,
        &names,
        "key",
        uses,
        move |e| e.delete_library_keys_keep_uses(&keep),
        move |e| e.delete_library_keys(&ids),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: bool, pinned: bool, library: bool) -> KeyInfo {
        KeyInfo {
            key: key.then(Key::default),
            pinned,
            library: library.then(Uuid::nil),
        }
    }

    #[test]
    fn buttons_follow_the_key() {
        assert!(library_buttons(&info(false, false, false)).is_empty());
        assert_eq!(
            library_buttons(&info(true, false, false)),
            [("Add to Library", "add-to-library"), ("Pin to Every Page", "pin")]
        );
        assert_eq!(
            library_buttons(&info(true, true, true)),
            [("Make a Copy", "own-copy"), ("Unpin", "unpin")]
        );
        assert!(library_buttons(&info(false, false, true)).is_empty());
    }

    #[test]
    fn subtitle_says_where_it_comes_from() {
        let u = Usage {
            pages: 3,
            pinned: false,
        };
        assert_eq!(
            key_subtitle("Page 1 · Main", &info(true, false, false), None),
            "Page 1 · Main"
        );
        assert_eq!(
            key_subtitle("Page 1 · Main", &info(true, true, true), Some(u)),
            "Page 1 · Main · From your library · Used on 3 pages · Pinned to every page"
        );
    }

    fn labels(root: &gtk::Widget) -> Vec<String> {
        let mut out = Vec::new();
        let mut c = root.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if let Ok(l) = w.clone().downcast::<gtk::Label>() {
                out.push(l.label().to_string());
            }
            out.extend(labels(&w));
        }
        out
    }

    fn pump_until(done: impl Fn() -> bool) {
        let main = gtk::glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) && !done() {
            while main.iteration(false) {}
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_picked_library_key_opens_the_key_editor_and_hides_what_a_pin_covers() {
        crate::ui::with_gtk(|| {
            let mut p = Profile::new_default("T", Model::D200X);
            p.pages.push(Page::new("Two"));
            p.pages[0].keys.insert(
                0,
                Key {
                    tap: Some(Action::NextPage),
                    ..Key::default()
                },
            );
            let state = crate::state::AppState::new(p);
            let panel = crate::ui::inspector::Panel::build(&state);
            let id = state.editor.borrow_mut().add_key_to_library(0).expect("added");
            state.select_library(id);
            let has = |t: &str| labels(&panel.root).iter().any(|l| l == t);
            pump_until(|| has("Delete From Library"));
            assert!(
                has("Delete From Library") && has("When Held"),
                "the full key editor"
            );
            // An edit there is the library key's, so the deck key follows.
            state.edit(|e| {
                e.set_field(
                    &[ulanzi_core::edit::LIBRARY_KEY],
                    ulanzi_core::edit::Field::LabelText("Lib".into()),
                )
            });
            assert_eq!(
                state
                    .editor
                    .borrow()
                    .profile()
                    .key_at(0, 0)
                    .map(|k| k.look.label.text),
                Some("Lib".into())
            );

            // Page two's own key under a pin is shown, not edited.
            state.editor.borrow_mut().set_page(1);
            state.edit(|e| e.set_field(&[0], ulanzi_core::edit::Field::LabelText("Mine".into())));
            state.set_page(0);
            state.edit(|e| e.pin_key(0));
            state.select(|s| s.click(ulanzi_core::edit::Slot::Key(0)));
            state.set_page(1);
            pump_until(|| has("Hidden by the Pin"));
            assert!(has("Hidden by the Pin"), "the page's own key is shown");
            assert!(
                state.selection.borrow().contains(ulanzi_core::edit::Slot::Key(0)),
                "the selection stays across pages"
            );
        });
    }
}
