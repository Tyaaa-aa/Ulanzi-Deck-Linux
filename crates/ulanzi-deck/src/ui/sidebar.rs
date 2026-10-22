//! Left sidebar: profiles, the current profile's pages, brightness.

use adw::prelude::*;
use gtk::glib;

use crate::runtime;
use crate::state::{Change, State};

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

    {
        let state = state.clone();
        column.append(&heading("Profiles", "New profile", move || new_profile(&state)));
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
            if matches!(c, Change::Profiles | Change::Profile) {
                while let Some(row) = profiles.row_at_index(0) {
                    profiles.remove(&row);
                }
                let current = state.profile_id();
                let list = state.profiles.borrow().clone();
                for (id, name) in &list {
                    let row = profile_row(name, "Always", *id == current);
                    profiles.append(&row);
                    if *id == current {
                        profiles.select_row(Some(&row));
                    }
                }
            }
            if matches!(c, Change::Profile | Change::Page) {
                while let Some(row) = pages.row_at_index(0) {
                    pages.remove(&row);
                }
                let e = state.editor.borrow();
                for (i, p) in e.profile().pages.iter().enumerate() {
                    let row = page_row(i + 1, &p.name, p.keys.len());
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
