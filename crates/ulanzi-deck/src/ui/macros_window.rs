//! The Macros window (next to the Plugins button): every macro in the library, to make, edit, copy
//! or delete. Putting one on a key happens in the action list or on the key.

use adw::prelude::*;
use ulanzi_core::model::Macro;
use uuid::Uuid;

use crate::runtime;
use crate::state::{Change, State};
use crate::ui::library::confirm_delete;
use crate::ui::macro_editor::{self, MacroTarget};
use crate::ui::macro_steps::{step_count, type_name};

fn row(state: &State, m: &Macro) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib_escape(&m.name))
        .subtitle(format!("{} · {}", type_name(&m.trigger), step_count(m)))
        .activatable(true)
        .build();
    let id = m.id;
    if let Some(c) = state.client.borrow().clone() {
        let (row, summary) = (row.clone(), row.subtitle().unwrap_or_default());
        runtime::run(async move { c.macro_usage(id).await }, move |r| {
            let total: u32 = r.iter().flatten().map(|(_, n)| n).sum();
            let keys = if total == 1 { "key" } else { "keys" };
            row.set_subtitle(&format!("{summary} · on {total} {keys}"));
        });
    }
    {
        let state = state.clone();
        row.connect_activated(move |r| macro_editor::open(&state, r, MacroTarget::Library(id)));
    }
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Duplicate")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let state = state.clone();
        copy.connect_clicked(move |_| duplicate(&state, id));
    }
    row.add_suffix(&copy);
    let delete = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Delete")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let (state, name) = (state.clone(), m.name.clone());
        delete.connect_clicked(move |b| confirm_delete(&state, b.upcast_ref(), id, &name));
    }
    row.add_suffix(&delete);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}

fn glib_escape(text: &str) -> String {
    gtk::glib::markup_escape_text(text).to_string()
}

fn duplicate(state: &State, id: Uuid) {
    let Some(original) = state.editor.borrow().library().get(id).cloned() else {
        return;
    };
    let mut copy = original;
    copy.id = Uuid::new_v4();
    copy.name = format!("{} Copy", copy.name);
    copy.previous = None;
    state.edit(|e| {
        e.save_macro(copy);
    });
}

fn fill(state: &State, list: &gtk::ListBox, empty: &gtk::Label) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let mut macros = state.editor.borrow().library().macros.clone();
    macros.sort_by_key(|m| m.name.to_lowercase());
    for m in &macros {
        list.append(&row(state, m));
    }
    list.set_visible(!macros.is_empty());
    empty.set_visible(macros.is_empty());
}

/// Open the window.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::PreferencesDialog::builder()
        .title("Macros")
        .content_width(620)
        .content_height(560)
        .build();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Your Macros")
        .description("Put one on a key by dragging it from the action list, or choose Macro on the key.")
        .build();
    let new = gtk::Button::builder()
        .label("New Macro")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    {
        let state = state.clone();
        new.connect_clicked(move |b| macro_editor::open(&state, b, MacroTarget::New));
    }
    group.set_header_suffix(Some(&new));
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    let empty = gtk::Label::builder()
        .label("No macros yet. Make one with New Macro.")
        .css_classes(["dim-label"])
        .margin_top(24)
        .margin_bottom(24)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&list);
    column.append(&empty);
    group.add(&column);
    page.add(&group);
    dialog.add(&page);
    fill(state, &list, &empty);
    {
        let (st, alive) = (state.clone(), list.downgrade());
        let empty = empty.downgrade();
        state.on_while(move |c| {
            let (Some(list), Some(empty)) = (alive.upgrade(), empty.upgrade()) else {
                return false;
            };
            if c == Change::Macros {
                fill(&st, &list, &empty);
            }
            true
        });
    }
    dialog.present(Some(parent));
}
