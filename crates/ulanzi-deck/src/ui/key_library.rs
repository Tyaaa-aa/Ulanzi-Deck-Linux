//! The key library in the inspector: the library row of a key on the deck, and the view of a
//! library key picked in the Library tray. A library key is changed by editing any key that uses
//! it; the tray's view names, shows and deletes it.

use adw::prelude::*;
use ulanzi_core::model::*;
use uuid::Uuid;

use crate::state::State;
use crate::ui::action_editor::How;
use crate::ui::canvas;
use crate::ui::dial_inspector::{delete_body_for, header, name_row, note, used_on};
use crate::ui::inspector::{C, Section, View, padded};

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
        let own = p.pages.get(e.page()).and_then(|pg| pg.keys.get(&k));
        let covered = own.is_some() && own != p.pinned.keys.get(&k);
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
    if covered {
        column.append(&note(
            "This page has its own key here, hidden by the pin. Unpin to use it.",
        ));
    }
    Some(column)
}

/// A library key picked in the Library tray: its picture, name, where it is used, and Delete.
pub(super) fn library_view(ctx: &C, content: &gtk::Box, id: Uuid) -> View {
    let state = &ctx.state;
    let mut view = View::default();
    let Some((lib, usage)) = ({
        let e = state.editor.borrow();
        let p = e.profile();
        p.library_key(id).map(|l| (l.clone(), p.key_usage(id)))
    }) else {
        return view;
    };
    let (head, title) = header(&lib.name, "From your library", None);
    content.append(&padded(head));

    let page = view.page(Section::Press);
    let picture = gtk::Image::builder()
        .pixel_size(106)
        .css_classes(["key-preview"])
        .halign(gtk::Align::Start)
        .build();
    let dir = canvas::profile_dir(state);
    picture.set_paintable(canvas::key_texture(&lib.key.shown_look(), &dir).as_ref());
    page.append(&picture);
    let ctx2 = ctx.clone();
    page.append(&name_row("New Key", &lib.name, &title, move |t| {
        ctx2.commit(How::Typing("library-key-name"), |e| e.rename_library_key(id, t))
    }));
    page.append(&note(&format!(
        "{}. To change its look or what it does, edit any key that uses it: every use changes.",
        crate::ui::dial_inspector::usage_line(usage)
    )));

    let delete = adw::ButtonRow::builder().title("Delete from Library").build();
    delete.add_css_class("destructive-action");
    {
        let state = state.clone();
        delete.connect_activated(move |row| delete_library_key(&state, id, row));
    }
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.append(&delete);
    page.append(&list);
    view
}

/// Delete library key `id`, asking first (over `parent`) when a page uses it.
pub(crate) fn delete_library_key(state: &State, id: Uuid, parent: &impl IsA<gtk::Widget>) {
    let (name, usage) = {
        let e = state.editor.borrow();
        let p = e.profile();
        let Some(lib) = p.library_key(id) else { return };
        (lib.name.clone(), p.key_usage(id))
    };
    let gone = {
        let state = state.clone();
        move || {
            state.edit(|e| e.delete_library_key(id));
            state.select(|s| s.clear());
        }
    };
    let Some(body) = delete_body_for(usage, "key") else {
        gone();
        return;
    };
    let alert = adw::AlertDialog::builder()
        .heading(format!("Delete “{name}”?"))
        .body(body)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("delete", "Delete");
    alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    alert.connect_response(Some("delete"), move |_, _| gone());
    alert.present(Some(parent));
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
}
