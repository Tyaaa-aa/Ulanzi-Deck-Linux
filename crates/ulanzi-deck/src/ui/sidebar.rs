//! Left sidebar: profiles, the current profile's pages, brightness.

use adw::prelude::*;
use gtk::glib;

use crate::runtime;
use crate::state::{Change, State};

/// Labels of the "+" menu, the profile right-click menu and the page right-click menu.
/// Menu items never end in an ellipsis (the menu_labels test enforces it).
pub(crate) const NEW_PROFILE_MENU: [&str; 3] = ["Empty Profile", "From Template", "Import File"];
pub(crate) const PROFILE_MENU: [&str; 7] = [
    "Move Up",
    "Move Down",
    "Auto-Switch",
    "Export",
    "Save as Template",
    "Rename",
    "Delete",
];
pub(crate) const PAGE_MENU: [&str; 4] = ["Move Up", "Move Down", "Rename", "Delete"];

fn heading(text: &str, add_tooltip: &str, on_add: impl Fn() + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["sidebar-heading"])
        .build();
    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(add_tooltip)
        .css_classes(["flat", "circular"])
        .margin_end(8)
        .valign(gtk::Align::End)
        .build();
    add.connect_clicked(move |_| on_add());
    row.append(&label);
    row.append(&add);
    row
}

/// "Profiles" with a + menu: empty profile, from a template, or from a file.
fn profiles_heading(state: &State) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(
        &gtk::Label::builder()
            .label("Profiles")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["sidebar-heading"])
            .build(),
    );
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let pop = gtk::Popover::builder().child(&menu).build();
    let item = |label: &str, f: Box<dyn Fn(&gtk::Button)>| {
        let b = gtk::Button::builder().label(label).css_classes(["flat"]).build();
        let pop = pop.clone();
        b.connect_clicked(move |b| {
            pop.popdown();
            f(b);
        });
        menu.append(&b);
    };
    {
        let state = state.clone();
        item(NEW_PROFILE_MENU[0], Box::new(move |_| new_profile(&state)));
    }
    {
        let state = state.clone();
        item(
            NEW_PROFILE_MENU[1],
            Box::new(move |b| crate::ui::templates::open(&state, b)),
        );
    }
    {
        let state = state.clone();
        item(
            NEW_PROFILE_MENU[2],
            Box::new(move |b| crate::ui::bundle_dialogs::choose_and_import(&state, b)),
        );
    }
    let add = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New profile")
        .css_classes(["flat", "circular"])
        .margin_end(8)
        .valign(gtk::Align::End)
        .popover(&pop)
        .build();
    row.append(&add);
    row
}

fn profile_row(name: &str, subtitle: &str, active: bool) -> gtk::ListBoxRow {
    let b = gtk::Box::builder()
        .spacing(12)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(6)
        .margin_end(6)
        .build();
    let badge = gtk::Image::builder()
        .icon_name("view-grid-symbolic")
        .css_classes(["app-badge"])
        .build();
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.append(
        &gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    text.append(
        &gtk::Label::builder()
            .label(subtitle)
            .xalign(0.0)
            .css_classes(["caption", "dim-label"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    text.set_hexpand(true);
    b.append(&badge);
    b.append(&text);
    if active {
        b.append(
            &gtk::Image::builder()
                .icon_name("object-select-symbolic")
                .tooltip_text("Active on the deck")
                .build(),
        );
    }
    gtk::ListBoxRow::builder().child(&b).build()
}

fn page_row(n: usize, name: &str, keys: usize) -> gtk::ListBoxRow {
    let b = gtk::Box::builder()
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    b.append(
        &gtk::Label::builder()
            .label(n.to_string())
            .css_classes(["page-number"])
            .build(),
    );
    b.append(
        &gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    b.append(
        &gtk::Label::builder()
            .label(format!("{keys} key{}", if keys == 1 { "" } else { "s" }))
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    gtk::ListBoxRow::builder().child(&b).build()
}

/// Build the sidebar content (inside a toolbar view supplied by the window).
pub fn build(state: &State) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let profiles = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let pages = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();

    column.append(&auto_switch_row(state));
    {
        let state = state.clone();
        column.append(&profiles_heading(&state));
    }
    column.append(&profiles);
    {
        let state = state.clone();
        column.append(&heading("Pages", "Add a page", move || {
            state.edit(|e| {
                let n = e.profile().pages.len() + 1;
                let at = e.add_page(&format!("Page {n}"));
                e.set_page(at);
            });
            state.notify(Change::Page);
        }));
    }
    column.append(&pages);
    scroller.set_child(Some(&column));
    root.append(&scroller);

    // Brightness at the bottom.
    let footer = gtk::Box::builder()
        .spacing(10)
        .margin_start(14)
        .margin_end(14)
        .margin_top(10)
        .margin_bottom(12)
        .build();
    footer.append(&gtk::Image::from_icon_name("display-brightness-symbolic"));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 5.0);
    scale.set_hexpand(true);
    scale.set_draw_value(false);
    scale.set_tooltip_text(Some("Deck brightness"));
    let value = gtk::Label::builder()
        .width_chars(4)
        .xalign(1.0)
        .css_classes(["dim-label", "numeric"])
        .build();
    footer.append(&scale);
    footer.append(&value);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&footer);

    // Fill and refill.
    let refresh = {
        let (state, profiles, pages, scale, value) = (
            state.clone(),
            profiles.clone(),
            pages.clone(),
            scale.clone(),
            value.clone(),
        );
        let updating = std::rc::Rc::new(std::cell::Cell::new(false));
        let updating2 = updating.clone();
        scale.connect_value_changed(glib::clone!(
            #[strong]
            state,
            #[strong]
            value,
            move |s| {
                let v = s.value().round() as u8;
                value.set_label(&format!("{v}%"));
                if updating2.get() {
                    return;
                }
                state.settings.borrow_mut().brightness = v;
                if let Some(c) = state.client.borrow().clone() {
                    runtime::run(async move { c.set_brightness(v).await }, |_| {});
                }
            }
        ));
        move |c: Change| {
            if matches!(c, Change::Profiles | Change::Profile | Change::Update) {
                while let Some(row) = profiles.row_at_index(0) {
                    profiles.remove(&row);
                }
                let current = state.profile_id();
                let list = state.profiles.borrow().clone();
                crate::ui::autoswitch::load_rules(&state);
                let default = state.settings.borrow().default_profile;
                for (i, (id, name)) in list.iter().enumerate() {
                    let rule = if *id == current {
                        Some(state.editor.borrow().profile().r#match.clone())
                    } else {
                        state.rules.borrow().get(id).cloned()
                    };
                    let sub = crate::ui::autoswitch::summary(rule.as_ref(), default == Some(*id));
                    let row = profile_row(name, &sub, *id == current);
                    attach_profile_menu(&state, &row, *id, name, i, list.len());
                    let st = state.clone();
                    reorderable(&row, "profile", i, move |from, to| move_profile(&st, from, to));
                    profiles.append(&row);
                    if *id == current {
                        profiles.select_row(Some(&row));
                    }
                }
                for name in state.too_new.borrow().iter() {
                    let row = profile_row(name, "Needs a newer version", false);
                    row.set_activatable(false);
                    row.add_css_class("dim-label");
                    if state
                        .update
                        .borrow()
                        .as_ref()
                        .is_some_and(|s| s.offered.is_some())
                    {
                        let update = gtk::Button::builder()
                            .label("Update")
                            .valign(gtk::Align::Center)
                            .build();
                        let st = state.clone();
                        update.connect_clicked(move |b| crate::ui::updates::open(&st, b));
                        if let Some(b) = row.child().and_downcast::<gtk::Box>() {
                            b.append(&update);
                        }
                    }
                    profiles.append(&row);
                }
            }
            if matches!(c, Change::Profile | Change::Page) {
                while let Some(row) = pages.row_at_index(0) {
                    pages.remove(&row);
                }
                let e = state.editor.borrow();
                let count = e.profile().pages.len();
                for (i, p) in e.profile().pages.iter().enumerate() {
                    let row = page_row(i + 1, &p.name, p.keys.len());
                    attach_page_menu(&state, &row, i, count, &p.name);
                    let st = state.clone();
                    reorderable(&row, "page", i, move |from, to| move_page(&st, from, to));
                    pages.append(&row);
                    if i == e.page() {
                        pages.select_row(Some(&row));
                    }
                }
            }
            if c == Change::Settings {
                updating.set(true);
                scale.set_value(state.settings.borrow().brightness as f64);
                updating.set(false);
            }
        }
    };
    refresh(Change::Profiles);
    refresh(Change::Page);
    refresh(Change::Settings);
    state.on(refresh);

    {
        let state = state.clone();
        pages.connect_row_activated(move |_, row| {
            let i = row.index().max(0) as usize;
            if i != state.editor.borrow().page() {
                state.set_page(i);
                let c = state.client.borrow().clone();
                if let Some(c) = c {
                    runtime::run(async move { c.set_page(i as u32).await }, |_| {});
                }
            }
        });
    }
    {
        let state = state.clone();
        profiles.connect_row_activated(move |_, row| {
            let i = row.index().max(0) as usize;
            let Some((id, _)) = state.profiles.borrow().get(i).cloned() else {
                return;
            };
            if id == state.profile_id() {
                return;
            }
            crate::ui::window::open_profile(&state, id, true);
        });
    }
    root
}

fn new_profile(state: &State) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let state = state.clone();
    let n = state.profiles.borrow().len() + 1;
    runtime::run(
        async move { c.create_profile(&format!("Profile {n}")).await },
        move |r| {
            if let Ok(id) = r {
                crate::ui::window::open_profile(&state, id, true);
            }
        },
    );
}

/// Right-click (or long-press) a profile for Rename and Delete; double-click renames.
fn attach_profile_menu(
    state: &State,
    row: &gtk::ListBoxRow,
    id: uuid::Uuid,
    name: &str,
    index: usize,
    count: usize,
) {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    {
        let st = state.clone();
        add_move_buttons(&menu, index, count, move |from, to| move_profile(&st, from, to));
    }
    let auto = gtk::Button::builder()
        .label(PROFILE_MENU[2])
        .css_classes(["flat"])
        .build();
    menu.append(&auto);
    {
        let (state, name) = (state.clone(), name.to_string());
        auto.connect_clicked(move |b| {
            if let Some(pop) = b
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                pop.popdown();
            }
            crate::ui::autoswitch::open(&state, b, id, &name);
        });
    }
    let export = gtk::Button::builder()
        .label(PROFILE_MENU[3])
        .css_classes(["flat"])
        .build();
    menu.append(&export);
    {
        let state = state.clone();
        export.connect_clicked(move |b| {
            if let Some(pop) = b
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                pop.popdown();
            }
            crate::ui::bundle_dialogs::open_export(&state, b, Some(id));
        });
    }
    let save_template = gtk::Button::builder()
        .label(PROFILE_MENU[4])
        .css_classes(["flat"])
        .build();
    menu.append(&save_template);
    {
        let (state, name) = (state.clone(), name.to_string());
        save_template.connect_clicked(move |b| {
            if let Some(pop) = b
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                pop.popdown();
            }
            crate::ui::templates::save_as_template(&state, id, &name);
        });
    }
    let rename = gtk::Button::builder()
        .label(PROFILE_MENU[5])
        .css_classes(["flat"])
        .build();
    let delete = gtk::Button::builder()
        .label(PROFILE_MENU[6])
        .css_classes(["flat", "error"])
        .build();
    menu.append(&rename);
    menu.append(&delete);
    let pop = gtk::Popover::builder().child(&menu).has_arrow(true).build();
    // Inside the row's own box, so the popover goes away with the row when the list refreshes.
    if let Some(b) = row.child().and_downcast::<gtk::Box>() {
        b.append(&pop);
    }
    {
        let (state, pop, name) = (state.clone(), pop.clone(), name.to_string());
        rename.connect_clicked(move |b| {
            pop.popdown();
            rename_dialog(&state, b, id, &name);
        });
    }
    {
        let (state, pop, name) = (state.clone(), pop.clone(), name.to_string());
        delete.connect_clicked(move |b| {
            pop.popdown();
            delete_dialog(&state, b, id, &name);
        });
    }
    let show = {
        let pop = pop.clone();
        move |x: f64, y: f64| {
            pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        }
    };
    let right = gtk::GestureClick::builder().button(3).build();
    {
        let show = show.clone();
        right.connect_pressed(move |_, _, x, y| show(x, y));
    }
    row.add_controller(right);
    let long = gtk::GestureLongPress::new();
    long.connect_pressed(move |_, x, y| show(x, y));
    row.add_controller(long);
    let double = gtk::GestureClick::new();
    {
        let (state, row2, name) = (state.clone(), row.clone(), name.to_string());
        double.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                rename_dialog(&state, &row2, id, &name);
            }
        });
    }
    row.add_controller(double);
}

fn rename_dialog(state: &State, near: &impl IsA<gtk::Widget>, id: uuid::Uuid, name: &str) {
    let entry = adw::EntryRow::builder().title("Name").text(name).build();
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.append(&entry);
    let dialog = adw::AlertDialog::builder()
        .heading("Rename Profile")
        .extra_child(&list)
        .default_response("rename")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    {
        let dialog = dialog.clone();
        entry.connect_entry_activated(move |_| {
            dialog.emit_by_name::<()>("response", &[&"rename"]);
            dialog.close();
        });
    }
    let state = state.clone();
    dialog.connect_response(Some("rename"), move |_, _| {
        let new = entry.text().trim().to_string();
        if new.is_empty() {
            return;
        }
        if id == state.profile_id() {
            state.edit(|e| e.rename_profile(&new));
        } else if let Some(c) = state.client.borrow().clone() {
            let st = state.clone();
            crate::runtime::run(
                async move {
                    let mut p = c.profile(id).await?;
                    p.name = new;
                    c.apply_now(&p).await
                },
                move |r| {
                    if let Err(e) = r {
                        crate::ui::toast(&st, &format!("Not renamed: {e}"));
                    }
                },
            );
        }
        // Show the new name right away; the service confirms it.
        if let Some(p) = state.profiles.borrow_mut().iter_mut().find(|(i, _)| *i == id) {
            p.1 = entry.text().trim().to_string();
        }
        state.notify(Change::Profiles);
    });
    dialog.present(Some(near));
}

fn delete_dialog(state: &State, near: &impl IsA<gtk::Widget>, id: uuid::Uuid, name: &str) {
    if state.profiles.borrow().len() <= 1 {
        crate::ui::toast(state, "The last profile can't be deleted");
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading("Delete Profile?")
        .body(format!(
            "“{name}” and its pages will be removed. A copy is kept in the profiles folder."
        ))
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let state = state.clone();
    dialog.connect_response(Some("delete"), move |_, _| {
        let Some(c) = state.client.borrow().clone() else {
            return;
        };
        // Leave the profile first if it is open.
        if id == state.profile_id()
            && let Some((other, _)) = state.profiles.borrow().iter().find(|(i, _)| *i != id).cloned()
        {
            crate::ui::window::open_profile(&state, other, true);
        }
        let st = state.clone();
        crate::runtime::run(async move { c.delete_profile(id).await }, move |r| {
            if let Err(e) = r {
                crate::ui::toast(&st, &format!("Not deleted: {e}"));
            }
        });
    });
    dialog.present(Some(near));
}

/// "Move Up" and "Move Down" buttons for a list menu.
fn add_move_buttons(menu: &gtk::Box, index: usize, count: usize, on_move: impl Fn(usize, usize) + 'static) {
    let on_move = std::rc::Rc::new(on_move);
    for (label, to) in [
        ("Move Up", index.checked_sub(1)),
        ("Move Down", Some(index + 1).filter(|t| *t < count)),
    ] {
        let b = gtk::Button::builder()
            .label(label)
            .css_classes(["flat"])
            .sensitive(to.is_some())
            .build();
        let on_move = on_move.clone();
        b.connect_clicked(move |b| {
            if let Some(pop) = b
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                pop.popdown();
            }
            if let Some(to) = to {
                on_move(index, to);
            }
        });
        menu.append(&b);
    }
}

/// Let `row` be dragged onto another row of the same `kind` to move it there.
fn reorderable(
    row: &gtk::ListBoxRow,
    kind: &'static str,
    index: usize,
    on_move: impl Fn(usize, usize) + 'static,
) {
    let prefix = format!("ulanzi-{kind}:");
    let source = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::MOVE)
        .build();
    {
        let payload = format!("{prefix}{index}");
        source
            .connect_prepare(move |_, _, _| Some(gtk::gdk::ContentProvider::for_value(&payload.to_value())));
    }
    {
        let row = row.clone();
        source.connect_drag_begin(move |src, _| {
            let icon = gtk::WidgetPaintable::new(Some(&row));
            src.set_icon(Some(&icon), 20, 20);
        });
    }
    row.add_controller(source);
    let target = gtk::DropTarget::new(glib::Type::STRING, gtk::gdk::DragAction::MOVE);
    {
        let row = row.clone();
        target.connect_enter(move |_, _, _| {
            row.add_css_class("drop-target");
            gtk::gdk::DragAction::MOVE
        });
    }
    {
        let row = row.clone();
        target.connect_leave(move |_| row.remove_css_class("drop-target"));
    }
    {
        let row = row.clone();
        target.connect_drop(move |_, value, _, _| {
            row.remove_css_class("drop-target");
            let Some(from) = value
                .get::<String>()
                .ok()
                .and_then(|s| s.strip_prefix(&prefix).and_then(|n| n.parse::<usize>().ok()))
            else {
                return false;
            };
            if from != index {
                on_move(from, index);
            }
            true
        });
    }
    row.add_controller(target);
}

fn move_page(state: &State, from: usize, to: usize) {
    state.edit(|e| e.move_page(from, to));
    state.notify(Change::Page);
}

/// Reorder profiles; the order is kept in the settings.
fn move_profile(state: &State, from: usize, to: usize) {
    {
        let mut list = state.profiles.borrow_mut();
        if from >= list.len() || to >= list.len() {
            return;
        }
        let p = list.remove(from);
        list.insert(to, p);
        state.settings.borrow_mut().profile_order = list.iter().map(|(id, _)| *id).collect();
    }
    state.notify(Change::Profiles);
    let s = state.settings.borrow().clone();
    if let Some(c) = state.client.borrow().clone() {
        let st = state.clone();
        runtime::run(async move { c.apply_settings(&s).await }, move |r| {
            if let Err(e) = r {
                crate::ui::toast(&st, &format!("Order not saved: {e}"));
            }
        });
    }
}

/// Right-click a page for Move Up/Down, Rename and Delete.
fn attach_page_menu(state: &State, row: &gtk::ListBoxRow, index: usize, count: usize, name: &str) {
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    {
        let st = state.clone();
        add_move_buttons(&menu, index, count, move |from, to| move_page(&st, from, to));
    }
    let rename = gtk::Button::builder()
        .label(PAGE_MENU[2])
        .css_classes(["flat"])
        .build();
    let delete = gtk::Button::builder()
        .label(PAGE_MENU[3])
        .css_classes(["flat", "error"])
        .sensitive(count > 1)
        .build();
    menu.append(&rename);
    menu.append(&delete);
    let pop = gtk::Popover::builder().child(&menu).build();
    if let Some(b) = row.child().and_downcast::<gtk::Box>() {
        b.append(&pop);
    }
    {
        let (state, pop, name) = (state.clone(), pop.clone(), name.to_string());
        rename.connect_clicked(move |b| {
            pop.popdown();
            let entry = adw::EntryRow::builder().title("Name").text(&name).build();
            let list = gtk::ListBox::builder()
                .css_classes(["boxed-list"])
                .selection_mode(gtk::SelectionMode::None)
                .build();
            list.append(&entry);
            let dialog = adw::AlertDialog::builder()
                .heading("Rename Page")
                .extra_child(&list)
                .default_response("rename")
                .close_response("cancel")
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("rename", "Rename");
            dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
            let st = state.clone();
            dialog.connect_response(Some("rename"), move |_, _| {
                let new = entry.text().trim().to_string();
                if !new.is_empty() {
                    st.edit(|e| e.rename_page(index, &new));
                }
            });
            dialog.present(Some(b));
        });
    }
    {
        let (state, pop) = (state.clone(), pop.clone());
        delete.connect_clicked(move |_| {
            pop.popdown();
            state.edit(|e| {
                e.remove_page(index);
            });
            state.notify(Change::Page);
            crate::ui::toast(&state, "Page deleted. Press Ctrl+Z to bring it back");
        });
    }
    let show = {
        let pop = pop.clone();
        move |x: f64, y: f64| {
            pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        }
    };
    let right = gtk::GestureClick::builder().button(3).build();
    {
        let show = show.clone();
        right.connect_pressed(move |_, _, x, y| show(x, y));
    }
    row.add_controller(right);
    let long = gtk::GestureLongPress::new();
    long.connect_pressed(move |_, x, y| show(x, y));
    row.add_controller(long);
}

/// "Auto-Switch" on/off above the profiles: profiles follow the app in front.
fn auto_switch_row(state: &State) -> gtk::Box {
    let row = gtk::Box::builder()
        .spacing(8)
        .margin_start(14)
        .margin_end(12)
        .margin_top(8)
        .margin_bottom(2)
        .tooltip_text(
            "Switch to a profile when its app comes to the front (set the apps from a profile's menu)",
        )
        .build();
    row.append(&gtk::Image::from_icon_name("view-refresh-symbolic"));
    row.append(
        &gtk::Label::builder()
            .label("Auto-Switch")
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    let switch = gtk::Switch::builder()
        .active(state.settings.borrow().auto_switch)
        .valign(gtk::Align::Center)
        .build();
    row.append(&switch);
    // Set from outside (tray, settings reload): don't send it back.
    let quiet = std::rc::Rc::new(std::cell::Cell::new(false));
    {
        let (state, quiet) = (state.clone(), quiet.clone());
        switch.connect_active_notify(move |sw| {
            if quiet.get() {
                return;
            }
            let v = sw.is_active();
            crate::ui::preferences::change_boxed(&state, Box::new(move |s| s.auto_switch = v));
        });
    }
    {
        let (st, sw) = (state.clone(), switch.downgrade());
        state.on(move |c| {
            if c != Change::Settings {
                return;
            }
            if let Some(sw) = sw.upgrade() {
                let v = st.settings.borrow().auto_switch;
                if sw.is_active() != v {
                    quiet.set(true);
                    sw.set_active(v);
                    quiet.set(false);
                }
            }
        });
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_labels_have_no_ellipsis() {
        for l in NEW_PROFILE_MENU.iter().chain(&PROFILE_MENU).chain(&PAGE_MENU) {
            assert!(!l.ends_with('…') && !l.ends_with("..."), "menu item {l:?}");
        }
    }
}
