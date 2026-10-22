//! Choosing an installed app: icons, sections by kind (Games, Graphics & Design, …) and search.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use ulanzi_core::apps::{AppEntry, AppGroup};

use crate::ui::action_editor::installed_apps;

thread_local! {
    static ICONS: RefCell<HashMap<String, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
}

/// The app's icon (cached), or None when it has none we can load.
pub fn app_texture(desktop_id: &str) -> Option<gdk::Texture> {
    ICONS.with(|m| {
        m.borrow_mut()
            .entry(desktop_id.to_string())
            .or_insert_with(|| {
                ulanzi_core::apps::app_icon(desktop_id).and_then(|p| gdk::Texture::from_filename(p).ok())
            })
            .clone()
    })
}

/// A small icon + name line for an app.
pub fn app_label(desktop_id: Option<&str>, name: &str, size: i32) -> gtk::Box {
    let b = gtk::Box::builder().spacing(8).build();
    let img = gtk::Image::builder().pixel_size(size).build();
    match desktop_id.and_then(app_texture) {
        Some(t) => img.set_paintable(Some(&t)),
        None => img.set_icon_name(Some("application-x-executable-symbolic")),
    }
    b.append(&img);
    b.append(
        &gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(22)
            .build(),
    );
    b
}

fn matches(app: &AppEntry, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let hay = format!("{} {} {}", app.name, app.desktop_id, app.keywords.join(" ")).to_lowercase();
    q.split_whitespace().all(|w| hay.contains(w))
}

/// Build the list (on first open): section headers per kind of app, then the apps A–Z.
fn fill(
    list: &gtk::ListBox,
    search: &gtk::SearchEntry,
    current: Option<&str>,
    exclude: &[String],
    on_pick: Rc<dyn Fn(AppEntry)>,
    popover: &gtk::Popover,
) {
    let apps = installed_apps();
    // Index of each row: Some(app index) or a header for a group.
    enum Row {
        Header(AppGroup),
        App(usize),
    }
    let mut rows: Vec<Row> = Vec::new();
    for g in AppGroup::ALL {
        let members: Vec<usize> = apps
            .iter()
            .enumerate()
            .filter(|(_, a)| a.group() == g && !exclude.contains(&a.desktop_id))
            .map(|(i, _)| i)
            .collect();
        if members.is_empty() {
            continue;
        }
        let head = gtk::ListBoxRow::builder()
            .activatable(false)
            .selectable(false)
            .child(
                &gtk::Label::builder()
                    .label(g.title())
                    .xalign(0.0)
                    .css_classes(["heading", "app-section"])
                    .build(),
            )
            .build();
        list.append(&head);
        rows.push(Row::Header(g));
        for i in members {
            let a = &apps[i];
            let line = app_label(Some(&a.desktop_id), &a.name, 24);
            if current == Some(a.desktop_id.as_str()) {
                line.append(
                    &gtk::Image::builder()
                        .icon_name("object-select-symbolic")
                        .hexpand(true)
                        .halign(gtk::Align::End)
                        .build(),
                );
            }
            list.append(&gtk::ListBoxRow::builder().child(&line).build());
            rows.push(Row::App(i));
        }
    }
    let rows = Rc::new(rows);
    {
        let (rows, apps, search) = (rows.clone(), apps.clone(), search.clone());
        list.set_filter_func(move |row| {
            let q = search.text().to_lowercase();
            match rows.get(row.index().max(0) as usize) {
                Some(Row::App(i)) => matches(&apps[*i], q.trim()),
                // A section shows while any of its apps does.
                Some(Row::Header(g)) => apps.iter().any(|a| a.group() == *g && matches(a, q.trim())),
                None => false,
            }
        });
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |_| list.invalidate_filter());
    }
    {
        let (rows, apps, popover) = (rows.clone(), apps.clone(), popover.clone());
        list.connect_row_activated(move |_, row| {
            if let Some(Row::App(i)) = rows.get(row.index().max(0) as usize) {
                popover.popdown();
                on_pick(apps[*i].clone());
            }
        });
    }
    {
        // Enter picks the first match.
        let list = list.clone();
        search.connect_activate(move |_| {
            let mut i = 0;
            while let Some(row) = list.row_at_index(i) {
                if row.is_child_visible() && row.is_activatable() {
                    row.emit_activate();
                    break;
                }
                i += 1;
            }
        });
    }
}

/// A button that opens the app list. `child` is what the button shows.
pub fn button(
    child: &impl IsA<gtk::Widget>,
    current: Option<String>,
    exclude: Vec<String>,
    on_pick: impl Fn(AppEntry) + 'static,
) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .child(child)
        .valign(gtk::Align::Center)
        .build();
    let popover = gtk::Popover::new();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search apps")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(380)
        .max_content_height(460)
        .min_content_width(280)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    column.append(&search);
    column.append(&scroller);
    popover.set_child(Some(&column));
    button.set_popover(Some(&popover));
    let on_pick: Rc<dyn Fn(AppEntry)> = Rc::new(on_pick);
    let built = Rc::new(std::cell::Cell::new(false));
    {
        let (search, list, popover2) = (search.clone(), list.clone(), popover.clone());
        popover.connect_show(move |_| {
            if !built.replace(true) {
                fill(
                    &list,
                    &search,
                    current.as_deref(),
                    &exclude,
                    on_pick.clone(),
                    &popover2,
                );
            }
            search.set_text("");
            search.grab_focus();
        });
    }
    button
}

/// The app list in a dialog (screenshot mode, to check the list without a popover).
pub fn preview_dialog(parent: &impl IsA<gtk::Widget>) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_margin_top(12);
    column.set_margin_start(12);
    column.set_margin_end(12);
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search apps")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    let scroller = gtk::ScrolledWindow::builder().vexpand(true).child(&list).build();
    column.append(&search);
    column.append(&scroller);
    fill(
        &list,
        &search,
        Some("firefox.desktop"),
        &[],
        Rc::new(|_| {}),
        &gtk::Popover::new(),
    );
    adw::Dialog::builder()
        .title("Apps")
        .content_width(320)
        .content_height(560)
        .child(&column)
        .build()
        .present(Some(parent));
}
