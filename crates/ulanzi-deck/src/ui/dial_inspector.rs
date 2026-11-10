//! The inspector for a dial, a library dial picked in the Library tray, and a small button.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::edit::{Editor, Slot};
use ulanzi_core::model::*;
use uuid::Uuid;

use crate::state::{Change, State};
use crate::ui::action_editor::{self, How};
use crate::ui::inspector::{
    C, Section, View, Write, padded, page_name, slot_expander, slot_rows, title_block,
};

/// "Used on 1 page", "Used on N pages" or "Not placed".
fn used_on(pages: usize) -> String {
    match pages {
        0 => "Not placed".into(),
        1 => "Used on 1 page".into(),
        n => format!("Used on {n} pages"),
    }
}

/// The line under a dial's title.
pub fn subtitle(n: u8, info: &DialInfo, usage: Option<Usage>) -> String {
    let mut s = format!("Dial {}", n + 1);
    match (info.library, usage) {
        // Only pinned: "Pinned" below says where it is.
        (Some(_), Some(u)) if u.pages == 0 && u.pinned => s += " · From your library",
        (Some(_), Some(u)) => s += &format!(" · From your library · {}", used_on(u.pages)),
        _ if !info.pinned => s += " · This page only",
        _ => {}
    }
    if info.pinned {
        s += " · Pinned";
    }
    s
}

/// Under a linked dial's zones, when an edit shows in more than one place.
pub fn shared_note(u: Usage) -> Option<String> {
    let pages = |n: usize| {
        if n == 1 {
            "1 page".to_string()
        } else {
            format!("{n} pages")
        }
    };
    let text = match (u.pinned, u.pages) {
        (true, 0) => "Pinned to every page".to_string(),
        (true, n) => format!("Used on {} and pinned", pages(n)),
        (false, n) if n >= 2 => format!("Used on {n} pages"),
        _ => return None,
    };
    Some(format!("{text}. Edits show everywhere."))
}

/// A library dial's usage line in its own editor.
pub fn usage_line(u: Usage) -> String {
    match (u.pinned, u.pages) {
        (true, 0) => "Pinned".into(),
        (true, n) => format!("{} · Pinned", used_on(n)),
        (false, n) => used_on(n),
    }
}

/// What deleting a library dial in use empties or unpins; `None` when nothing uses it (no need to
/// ask).
pub fn delete_body(u: Usage) -> Option<String> {
    let (and_pinned, and_pin) = if u.pinned {
        (" and pinned", " and the pin removed")
    } else {
        ("", "")
    };
    match (u.pinned, u.pages) {
        (false, 0) => None,
        (true, 0) => Some("It's pinned. The pin will be removed.".into()),
        (_, 1) => Some(format!(
            "It's used on 1 page{and_pinned}. That dial will be empty{and_pin}."
        )),
        (_, n) => Some(format!(
            "It's used on {n} pages{and_pinned}. Those dials will be empty{and_pin}."
        )),
    }
}

/// The library dials the empty-dial picker lists: most used first (ties keep the library's
/// order), three of them unless `expanded`.
pub fn picker_rows(library: &[LibraryDial], usage: impl Fn(Uuid) -> Usage, expanded: bool) -> Vec<Uuid> {
    let mut rows: Vec<(usize, Uuid)> = library.iter().map(|l| (usage(l.id).pages, l.id)).collect();
    // Stable: equal counts stay in library order.
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    let take = if expanded { rows.len() } else { FOLDED };
    rows.into_iter().take(take).map(|(_, id)| id).collect()
}

/// Rows the picker shows before Show All Dials.
const FOLDED: usize = 3;

/// The dial's name, else "Dial N".
fn shown_name(n: u8, dial: &Dial) -> String {
    if dial.name.trim().is_empty() {
        format!("Dial {}", n + 1)
    } else {
        dial.name.clone()
    }
}

/// Title, subtitle and an optional button at the top right; returns the title label too.
fn header(title: &str, sub: &str, button: Option<gtk::Button>) -> (gtk::Box, gtk::Label) {
    let titles = title_block(title, sub);
    titles.set_hexpand(true);
    let label = titles
        .first_child()
        .and_downcast::<gtk::Label>()
        .expect("title_block starts with its title");
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&titles);
    if let Some(b) = button {
        b.set_valign(gtk::Align::Start);
        row.append(&b);
    }
    (row, label)
}

/// The Name row; `rename` gets the text as it is typed.
fn name_row(
    placeholder: &str,
    name: &str,
    title: &gtk::Label,
    rename: impl Fn(&str) + 'static,
) -> adw::PreferencesGroup {
    let entry = gtk::Entry::builder()
        .text(name)
        .placeholder_text(placeholder)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    let row = adw::ActionRow::builder()
        .title("Name")
        .activatable_widget(&entry)
        .build();
    row.add_suffix(&entry);
    let (title, placeholder) = (title.clone(), placeholder.to_string());
    entry.connect_changed(move |e| {
        let text = e.text();
        title.set_label(if text.trim().is_empty() {
            &placeholder
        } else {
            text.trim()
        });
        rename(&text);
    });
    let group = adw::PreferencesGroup::new();
    group.add(&row);
    group
}

type Pick = fn(&mut Dial) -> &mut Option<Action>;
/// Writes one zone of the dial being edited.
type SetZone = Rc<dyn Fn(&mut Editor, Pick, Option<Action>)>;
/// A button row: title, destructive, and its edit.
type Button = (&'static str, bool, Box<dyn Fn(&mut Editor)>);

/// The four zones; `set` writes one of them.
fn zones(ctx: &C, path: &str, dial: &Dial, set: SetZone) -> adw::PreferencesGroup {
    // The path names are the old expander titles, which the undo merging keys on.
    let zones: [(&str, Option<&Action>, Pick); 4] = [
        ("Turn Left", dial.left.as_ref(), |d| &mut d.left),
        ("Turn Right", dial.right.as_ref(), |d| &mut d.right),
        ("Press", dial.press.as_ref(), |d| &mut d.press),
        ("Hold", dial.hold.as_ref(), |d| &mut d.hold),
    ];
    // One list: each zone opens to its settings.
    let group = adw::PreferencesGroup::new();
    for (title, action, pick) in zones {
        let write: Write = {
            let (ctx, set) = (ctx.clone(), set.clone());
            Rc::new(move |a, how, _| ctx.commit(how, |e| set(e, pick, a)))
        };
        group.add(&slot_expander(
            ctx,
            &format!("{path}/{title}"),
            title,
            action,
            write,
        ));
    }
    group
}

/// A dim, wrapping line of text.
fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label"])
        .build()
}

/// A list of button rows; each runs its edit as one structural step.
fn buttons(ctx: &C, rows: Vec<Button>) -> Option<gtk::ListBox> {
    if rows.is_empty() {
        return None;
    }
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for (title, destructive, f) in rows {
        let row = adw::ButtonRow::builder().title(title).build();
        if destructive {
            // Added, not set: setting the classes would drop the row's own `button` style.
            row.add_css_class("destructive-action");
        }
        let ctx = ctx.clone();
        row.connect_activated(move |_| ctx.commit(How::Structure, |e| f(e)));
        list.append(&row);
    }
    Some(list)
}

/// The Open Library button: the tray opens with dial `n` outlined as the drop spot. Shown only
/// while the tray is shut.
fn open_library_button(state: &State, n: u8) -> gtk::Button {
    let b = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("io.github.tyaaa_aa.UlanziDeck-library-symbolic")
                .label("Open Library")
                .build(),
        )
        .build();
    b.connect_clicked(move |b| {
        let _ = b.activate_action("win.library-place", Some(&n.to_variant()));
    });
    b.set_visible(!state.tray_open.get());
    let (weak, st) = (b.downgrade(), state.clone());
    state.on_while(move |c| {
        let Some(b) = weak.upgrade() else { return false };
        if c == Change::Tray {
            b.set_visible(!st.tray_open.get());
        }
        true
    });
    b
}

pub(super) fn dial_view(ctx: &C, content: &gtk::Box, d: u8) -> View {
    let state = &ctx.state;
    // Rename from the dial's menu asks for the Name row.
    let focus = state.focus_name.take() == Some(Slot::Dial(d));
    let (info, usage, library_empty, covered) = {
        let e = state.editor.borrow();
        let info = e.dial_info(d);
        let usage = info.library.map(|id| e.profile().dial_usage(id));
        let own = e.profile().pages.get(e.page()).map(|p| &p.dials[d as usize]);
        let covered = info.pinned && own.is_some_and(|o| !o.is_empty());
        (info, usage, e.profile().dial_library.is_empty(), covered)
    };
    // A pin covers the page's dials even when blank: it keeps its Unpin.
    let empty = !info.pinned && info.library.is_none() && info.dial.is_empty();
    if empty && !library_empty {
        return empty_view(ctx, content, d);
    }
    let mut view = View::default();
    // With an empty library an empty dial is built here as the page's own.
    let (head, title) = if empty {
        header(
            &format!("Dial {}", d + 1),
            &format!("{} · Empty on this page", page_name(state)),
            Some(open_library_button(state, d)),
        )
    } else if info.library.is_none() && info.dial.name.trim().is_empty() {
        // The title already says "Dial N": name the page instead.
        let place = if info.pinned {
            "Pinned to every page"
        } else {
            "This page only"
        };
        header(
            &format!("Dial {}", d + 1),
            &format!("{} · {place}", page_name(state)),
            None,
        )
    } else {
        header(&shown_name(d, &info.dial), &subtitle(d, &info, usage), None)
    };
    content.append(&padded(head));

    let page = view.page(Section::Press);
    let name = {
        let ctx = ctx.clone();
        name_row(&format!("Dial {}", d + 1), &info.dial.name, &title, move |t| {
            ctx.commit(How::Typing("dial-name"), |e| e.rename_dial(d, t))
        })
    };
    page.append(&name);
    page.append(&zones(
        ctx,
        &format!("dial{d}"),
        &info.dial,
        Rc::new(move |e, pick, a| e.set_dial(d, |dial| *pick(dial) = a)),
    ));
    if let Some(text) = usage.filter(|_| info.library.is_some()).and_then(shared_note) {
        page.append(&note(&text));
    }

    let mut rows: Vec<Button> = Vec::new();
    if info.library.is_some() {
        rows.push(("Make a Copy", false, Box::new(move |e| e.own_copy_dial(d))));
    } else if !info.dial.is_empty() {
        rows.push((
            "Add to Library",
            false,
            Box::new(move |e| {
                e.add_dial_to_library(d);
            }),
        ));
    }
    if info.pinned {
        rows.push(("Unpin", false, Box::new(move |e| e.unpin_dial(d))));
    } else if !info.dial.is_empty() || info.library.is_some() {
        rows.push(("Pin to Every Page", false, Box::new(move |e| e.pin_dial(d))));
    }
    // Under a pin this would clear the hidden page dial, so it is offered only unpinned.
    if !info.pinned && (!info.dial.is_empty() || info.library.is_some()) {
        rows.push(("Remove from This Page", true, Box::new(move |e| e.remove_dial(d))));
    }
    if let Some(list) = buttons(ctx, rows) {
        page.append(&list);
    }
    if covered {
        page.append(&note(
            "This page has its own dial here, hidden by the pin. Unpin to use it.",
        ));
    }
    if focus {
        focus_entry(&name);
    }
    view
}

/// Put the typing focus in the Name row's entry once it is shown.
fn focus_entry(group: &adw::PreferencesGroup) {
    let mut stack = vec![group.clone().upcast::<gtk::Widget>()];
    while let Some(w) = stack.pop() {
        if let Ok(entry) = w.clone().downcast::<gtk::Entry>() {
            let weak = entry.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(e) = weak.upgrade() {
                    e.grab_focus();
                }
            });
            return;
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            c = child.next_sibling();
            stack.push(child);
        }
    }
}

/// Where `expanded` keeps the picker's Show All state.
const PICKER_PATH: &str = "dial-picker/all";

/// An empty dial: pick one from the library, make a new one, or open the tray.
fn empty_view(ctx: &C, content: &gtk::Box, d: u8) -> View {
    let state = &ctx.state;
    let mut view = View::default();
    let (head, _) = header(
        &format!("Dial {}", d + 1),
        &format!("{} · Empty on this page", page_name(state)),
        Some(open_library_button(state, d)),
    );
    content.append(&padded(head));

    // (id, name, first action, usage) in picker order, all of them.
    let (all, folded) = {
        let e = state.editor.borrow();
        let p = e.profile();
        let all: Vec<_> = picker_rows(&p.dial_library, |id| p.dial_usage(id), true)
            .into_iter()
            .filter_map(|id| p.library_dial(id))
            .map(|l| {
                let first = [&l.dial.press, &l.dial.left, &l.dial.right, &l.dial.hold]
                    .into_iter()
                    .find_map(|a| a.clone());
                (l.id, shown_name(0, &l.dial), first, p.dial_usage(l.id))
            })
            .collect();
        let folded = picker_rows(&p.dial_library, |id| p.dial_usage(id), false).len();
        (all, folded)
    };

    let page = view.page(Section::Press);
    page.set_spacing(12);
    page.append(
        &gtk::Label::builder()
            .label("Choose a Dial")
            .xalign(0.0)
            .css_classes(["heading"])
            .build(),
    );
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let expanded = state.expanded.borrow().contains(PICKER_PATH);
    // The rows past the first few slide open and shut in place.
    let mut extra: Vec<(gtk::ListBoxRow, gtk::Revealer)> = Vec::new();
    let mut ids = Vec::new();
    for (i, (id, name, first, usage)) in all.iter().enumerate() {
        let body = gtk::Box::builder()
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        body.append(&action_editor::action_chip(state, first.as_ref(), 32));
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
        labels.set_valign(gtk::Align::Center);
        labels.append(&gtk::Label::builder().label(name).xalign(0.0).build());
        // Pinned in the accent colour, as in the tray.
        let used = gtk::Label::builder()
            .label(usage.text())
            .xalign(0.0)
            .css_classes(if usage.pinned {
                ["caption", "accent"]
            } else {
                ["caption", "dim-label"]
            })
            .build();
        labels.append(&used);
        body.append(&labels);
        let shown = i < folded || expanded;
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .reveal_child(shown)
            .child(&body)
            .build();
        let row = gtk::ListBoxRow::builder()
            .activatable(true)
            .child(&revealer)
            .visible(shown)
            .build();
        if i >= folded {
            // Hidden once shut, so no empty row keeps its separator.
            let r = row.downgrade();
            revealer.connect_child_revealed_notify(move |rv| {
                if let Some(r) = r.upgrade()
                    && !rv.reveals_child()
                {
                    r.set_visible(false);
                }
            });
            extra.push((row.clone(), revealer));
        }
        list.append(&row);
        ids.push(*id);
    }
    // A click on a row reaches the list's row-activated, never the row's own activate.
    let mut toggle_row: Option<(gtk::ListBoxRow, Box<dyn Fn()>)> = None;
    if !extra.is_empty() {
        let count = all.len();
        let label = move |open: bool| {
            if open {
                "Show Fewer".to_string()
            } else {
                format!("Show All Dials ({count})")
            }
        };
        let text = gtk::Label::builder()
            .label(label(expanded))
            .margin_top(12)
            .margin_bottom(12)
            .css_classes(["accent", "heading"])
            .build();
        let toggle = gtk::ListBoxRow::builder().activatable(true).child(&text).build();
        list.append(&toggle);
        let state = state.clone();
        toggle_row = Some((
            toggle,
            Box::new(move || {
                let open = !state.expanded.borrow().contains(PICKER_PATH);
                if open {
                    state.expanded.borrow_mut().insert(PICKER_PATH.into());
                } else {
                    state.expanded.borrow_mut().remove(PICKER_PATH);
                }
                for (row, revealer) in &extra {
                    if open {
                        row.set_visible(true);
                    }
                    revealer.set_reveal_child(open);
                }
                text.set_label(&label(open));
            }),
        ));
    }
    {
        let ctx = ctx.clone();
        list.connect_row_activated(move |_, row| {
            if let Some((toggle, flip)) = &toggle_row
                && toggle == row
            {
                flip();
            } else if let Some(&id) = usize::try_from(row.index()).ok().and_then(|i| ids.get(i)) {
                ctx.commit(How::Structure, |e| e.place_dial(d, id));
            }
        });
    }
    page.append(&list);

    let new = adw::ButtonRow::builder()
        .title("New Dial")
        .start_icon_name("list-add-symbolic")
        .build();
    {
        let ctx = ctx.clone();
        new.connect_activated(move |_| {
            // Name it straight away.
            ctx.state.focus_name.set(Some(Slot::Dial(d)));
            ctx.commit(How::Structure, |e| {
                let id = e.new_library_dial();
                // Made and placed in one undo step.
                e.merge_next();
                e.place_dial(d, id);
            });
        });
    }
    let new_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    new_list.append(&new);
    page.append(&new_list);
    page.append(&note(&format!(
        "Or open the Library and drag a dial onto Dial {} on the deck.",
        d + 1
    )));
    view
}

/// A library dial picked in the Library tray: not a spot on the deck, the dial itself.
pub(super) fn library_view(ctx: &C, content: &gtk::Box, id: Uuid) -> View {
    let state = &ctx.state;
    let mut view = View::default();
    let Some((dial, usage)) = ({
        let e = state.editor.borrow();
        let p = e.profile();
        p.library_dial(id).map(|l| (l.dial.clone(), p.dial_usage(id)))
    }) else {
        return view;
    };
    let (head, title) = header(&shown_name(0, &dial), "From your library", None);
    content.append(&padded(head));

    let page = view.page(Section::Press);
    let ctx2 = ctx.clone();
    page.append(&name_row("New Dial", &dial.name, &title, move |t| {
        ctx2.commit(How::Typing("library-dial-name"), |e| e.rename_library_dial(id, t))
    }));
    page.append(&zones(
        ctx,
        "library",
        &dial,
        Rc::new(move |e, pick, a| e.set_library_dial(id, |dial| *pick(dial) = a)),
    ));
    page.append(&note(&usage_line(usage)));

    let delete = adw::ButtonRow::builder().title("Delete from Library").build();
    delete.add_css_class("destructive-action");
    {
        let state = state.clone();
        delete.connect_activated(move |row| delete_library_dial(&state, id, row));
    }
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    list.append(&delete);
    page.append(&list);
    view
}

/// Delete library dial `id`, asking first (over `parent`) when a page uses it.
pub(crate) fn delete_library_dial(state: &State, id: Uuid, parent: &impl IsA<gtk::Widget>) {
    let (name, usage) = {
        let e = state.editor.borrow();
        let p = e.profile();
        let Some(lib) = p.library_dial(id) else { return };
        (shown_name(0, &lib.dial), p.dial_usage(id))
    };
    let gone = {
        let state = state.clone();
        move || {
            state.edit(|e| e.delete_library_dial(id));
            state.select(|s| s.clear());
        }
    };
    let Some(body) = delete_body(usage) else {
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

pub(super) fn small_view(ctx: &C, content: &gtk::Box, s: u8) -> View {
    let mut view = View::default();
    let side = if s == 0 { "Left" } else { "Right" };
    let (action, pinned, covered) = {
        let e = ctx.state.editor.borrow();
        let p = e.profile();
        let pinned = p.small_pinned(s);
        let own = p.pages.get(e.page()).map(|pg| &pg.small_buttons);
        let own = own.and_then(|o| if s == 0 { o.left.as_ref() } else { o.right.as_ref() });
        (p.small_at(e.page(), s), pinned, pinned && own.is_some())
    };
    let sub = if pinned {
        "Pinned to every page".to_string()
    } else {
        page_name(&ctx.state)
    };
    content.append(&padded(title_block(&format!("{side} Button"), &sub)));
    let group = adw::PreferencesGroup::builder().title("Action").build();
    let write: Write = {
        let ctx = ctx.clone();
        Rc::new(move |a, how, _| ctx.commit(how, |e| e.set_small_button(s, a)))
    };
    // A pinned button is cleared by Unpin, never by a Remove that would unpin it unasked.
    for w in slot_rows(
        ctx,
        &format!("small{s}"),
        action.as_ref(),
        write,
        "Choose what this button does",
        !pinned,
    ) {
        group.add(&w);
    }
    let page = view.page(Section::Press);
    page.append(&group);
    let rows: Vec<Button> = if pinned {
        vec![("Unpin", false, Box::new(move |e| e.unpin_small(s)))]
    } else if action.is_some() {
        vec![("Pin to Every Page", false, Box::new(move |e| e.pin_small(s)))]
    } else {
        vec![]
    };
    if let Some(list) = buttons(ctx, rows) {
        page.append(&list);
    }
    if covered {
        page.append(&note(
            "This page has its own button here, hidden by the pin. Unpin to use it.",
        ));
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked(pinned: bool) -> DialInfo {
        DialInfo {
            dial: Dial::default(),
            pinned,
            library: Some(Uuid::new_v4()),
        }
    }

    #[test]
    fn subtitles() {
        let used = |pages, pinned| Some(Usage { pages, pinned });
        assert_eq!(
            subtitle(1, &linked(true), used(3, true)),
            "Dial 2 · From your library · Used on 3 pages · Pinned"
        );
        assert_eq!(
            subtitle(0, &linked(false), used(1, false)),
            "Dial 1 · From your library · Used on 1 page"
        );
        assert_eq!(
            subtitle(0, &linked(true), used(0, true)),
            "Dial 1 · From your library · Pinned"
        );
    }

    #[test]
    fn own_dial_subtitles() {
        let own = |pinned| DialInfo {
            dial: Dial::default(),
            pinned,
            library: None,
        };
        assert_eq!(subtitle(0, &own(false), None), "Dial 1 · This page only");
        assert_eq!(subtitle(2, &own(true), None), "Dial 3 · Pinned");
    }

    #[test]
    fn usage_notes_and_delete_warnings() {
        let u = |pages, pinned| Usage { pages, pinned };
        assert_eq!(shared_note(u(1, false)), None);
        assert_eq!(
            shared_note(u(3, false)).as_deref(),
            Some("Used on 3 pages. Edits show everywhere.")
        );
        assert_eq!(
            shared_note(u(3, true)).as_deref(),
            Some("Used on 3 pages and pinned. Edits show everywhere.")
        );
        assert_eq!(
            shared_note(u(0, true)).as_deref(),
            Some("Pinned to every page. Edits show everywhere.")
        );
        assert_eq!(usage_line(u(0, false)), "Not placed");
        assert_eq!(usage_line(u(2, false)), "Used on 2 pages");
        assert_eq!(usage_line(u(0, true)), "Pinned");
        assert_eq!(usage_line(u(1, true)), "Used on 1 page · Pinned");
        assert_eq!(delete_body(u(0, false)), None);
        assert_eq!(
            delete_body(u(0, true)).as_deref(),
            Some("It's pinned. The pin will be removed.")
        );
        assert_eq!(
            delete_body(u(1, false)).as_deref(),
            Some("It's used on 1 page. That dial will be empty.")
        );
        assert_eq!(
            delete_body(u(1, true)).as_deref(),
            Some("It's used on 1 page and pinned. That dial will be empty and the pin removed.")
        );
        assert_eq!(
            delete_body(u(3, true)).as_deref(),
            Some("It's used on 3 pages and pinned. Those dials will be empty and the pin removed.")
        );
    }

    fn state() -> crate::state::State {
        crate::state::AppState::new(Profile::new_default("Default", Model::D200X))
    }

    fn all<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Vec<T> {
        let mut out = Vec::new();
        let mut c = root.first_child();
        while let Some(w) = c {
            c = w.next_sibling();
            if let Ok(t) = w.clone().downcast::<T>() {
                out.push(t);
            }
            out.extend(all::<T>(&w));
        }
        out
    }

    /// Run the main loop until `done` (at most a few seconds).
    fn pump_until(done: impl Fn() -> bool) {
        let main = glib::MainContext::default();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            while main.iteration(false) {}
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn button_row(panel: &crate::ui::inspector::Panel, title: &str) -> Option<adw::ButtonRow> {
        all::<adw::ButtonRow>(&panel.root)
            .into_iter()
            .find(|r| r.title() == title)
    }

    #[test]
    fn new_dial_on_an_empty_dial_is_one_undo_step_and_asks_for_a_name() {
        crate::ui::with_gtk(|| {
            let state = state();
            let lib = state.editor.borrow_mut().new_library_dial();
            let panel = crate::ui::inspector::Panel::build(&state);
            let win = gtk::Window::builder().child(&panel.root).build();
            win.present();
            state.select(|s| s.click(Slot::Dial(1)));
            pump_until(|| button_row(&panel, "New Dial").is_some());
            button_row(&panel, "New Dial")
                .expect("empty dial picker")
                .emit_by_name::<()>("activated", &[]);
            let info = state.editor.borrow().dial_info(1);
            let id = info.library.expect("linked");
            assert_ne!(id, lib);
            assert_eq!(info.dial.name, "New Dial");
            // Its Name row gets the focus.
            let in_name = || {
                let entry = all::<gtk::Entry>(&panel.root)
                    .into_iter()
                    .find(|e| e.text() == "New Dial");
                let focus = gtk::prelude::GtkWindowExt::focus(&win);
                entry.zip(focus).is_some_and(|(e, f)| f.is_ancestor(&e))
            };
            pump_until(in_name);
            assert!(in_name());
            assert!(state.editor.borrow_mut().undo());
            let e = state.editor.borrow();
            assert!(e.dial_info(1).dial.is_empty());
            assert!(e.profile().library_dial(id).is_none());
            assert!(e.profile().library_dial(lib).is_some());
            drop(e);
            win.destroy();
        });
    }

    #[test]
    fn deleting_an_unused_library_dial_needs_no_question_and_ends_the_pick() {
        crate::ui::with_gtk(|| {
            let state = state();
            let id = state.editor.borrow_mut().new_library_dial();
            let panel = crate::ui::inspector::Panel::build(&state);
            state.select_library(id);
            pump_until(|| button_row(&panel, "Delete from Library").is_some());
            button_row(&panel, "Delete from Library")
                .expect("library view")
                .emit_by_name::<()>("activated", &[]);
            assert!(state.editor.borrow().profile().library_dial(id).is_none());
            assert_eq!(*state.selected_library.borrow(), None);
        });
    }

    #[test]
    fn a_blank_pin_offers_unpin_not_the_picker() {
        crate::ui::with_gtk(|| {
            // A pin linked to a library dial that is gone (deleting one now removes its pin).
            let mut p = Profile::new_default("Default", Model::D200X);
            p.pinned.dials.insert(
                0,
                Dial {
                    library: Some(Uuid::new_v4()),
                    ..Dial::default()
                },
            );
            let state = crate::state::AppState::new(p);
            state.edit(|e| {
                e.new_library_dial();
            });
            let panel = crate::ui::inspector::Panel::build(&state);
            state.select(|s| s.click(Slot::Dial(0)));
            pump_until(|| button_row(&panel, "Unpin").is_some());
            assert!(button_row(&panel, "Unpin").is_some());
            assert!(button_row(&panel, "New Dial").is_none());
        });
    }

    fn has_label(panel: &crate::ui::inspector::Panel, text: &str) -> bool {
        all::<gtk::Label>(&panel.root).iter().any(|l| l.label() == text)
    }

    #[test]
    fn a_pin_over_the_pages_own_dial_or_button_says_so() {
        crate::ui::with_gtk(|| {
            let dial = "This page has its own dial here, hidden by the pin. Unpin to use it.";
            let button = "This page has its own button here, hidden by the pin. Unpin to use it.";
            let mut p = Profile::new_default("Default", Model::D200X);
            p.pages.push(Page::new("Two"));
            let own = |a| Dial {
                press: Some(a),
                ..Dial::default()
            };
            p.pinned.dials.insert(0, own(Action::Back));
            p.pages[0].dials[0] = own(Action::NextPage);
            p.pages[0].small_buttons.left = Some(Action::SleepNow);
            let state = crate::state::AppState::new(p);
            let panel = crate::ui::inspector::Panel::build(&state);
            state.select(|s| s.click(Slot::Dial(0)));
            pump_until(|| has_label(&panel, dial));
            assert!(has_label(&panel, dial));
            state.select(|s| s.click(Slot::Small(0)));
            pump_until(|| has_label(&panel, button));
            assert!(has_label(&panel, button));
            // Page two has nothing of its own under the pins.
            state.set_page(1);
            state.select(|s| s.click(Slot::Dial(0)));
            pump_until(|| has_label(&panel, "Page 2 · Two · Pinned to every page"));
            assert!(has_label(&panel, "Page 2 · Two · Pinned to every page"));
            assert!(!has_label(&panel, dial));
            state.select(|s| s.click(Slot::Small(0)));
            pump_until(|| has_label(&panel, "Left Button"));
            assert!(has_label(&panel, "Left Button"));
            assert!(!has_label(&panel, button));
        });
    }

    #[test]
    fn picker_shows_most_used_first_and_folds_to_three() {
        let library: Vec<LibraryDial> = (0..5)
            .map(|_| LibraryDial {
                id: Uuid::new_v4(),
                dial: Dial::default(),
            })
            .collect();
        let ids: Vec<Uuid> = library.iter().map(|l| l.id).collect();
        let [a, b, c, d, e] = ids[..] else { unreachable!() };
        let pages = [0, 3, 1, 3, 2];
        let usage = |id: Uuid| Usage {
            pages: pages[ids.iter().position(|i| *i == id).expect("listed")],
            pinned: false,
        };
        assert_eq!(picker_rows(&library, usage, false), vec![b, d, e]);
        assert_eq!(picker_rows(&library, usage, true), vec![b, d, e, c, a]);
    }
}
