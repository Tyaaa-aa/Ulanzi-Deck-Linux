//! Left sidebar: profiles, the current profile's pages, brightness.

use adw::prelude::*;
use gtk::glib;
use std::rc::Rc;

use ulanzi_core::Color;
use ulanzi_core::model::{IconSet, ImageSource, KeyLook, ProfileLook};
use ulanzi_core::store::Store;

use crate::runtime;
use crate::state::{Change, State};

/// Labels of the "+" menu, the profile right-click menu and the page right-click menu.
/// Menu items never end in an ellipsis (the menu_labels test enforces it).
pub(crate) const NEW_PROFILE_MENU: [&str; 3] = ["Empty Profile", "From Template", "Import File"];
pub(crate) const PROFILE_MENU: [&str; 8] = [
    "Move Up",
    "Move Down",
    "Auto-Switch",
    "Icon and Colour",
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

const TILE_PX: i32 = 32;
const TILE_ICON_SCALE: f32 = 0.7;

/// Colours that read well on a row in both light and dark themes.
const TINTS: [(&str, &str); 9] = [
    ("Blue", "#3584e4"),
    ("Teal", "#2190a4"),
    ("Green", "#3a944a"),
    ("Yellow", "#c88800"),
    ("Orange", "#ed5b00"),
    ("Red", "#e62d42"),
    ("Pink", "#d56199"),
    ("Purple", "#9141ac"),
    ("Slate", "#6f8396"),
];

fn store() -> Option<Store> {
    Store::xdg().ok()
}

/// The picture on a profile's tile, or `None` for the plain one.
fn tile_texture(id: uuid::Uuid, chosen: &ProfileLook, apps: &[String]) -> Option<gtk::gdk::Texture> {
    if chosen.is_default() {
        return None;
    }
    // A look from auto-switch shows every app the profile follows, up to three.
    if chosen.from_app.is_some() && apps.len() > 1 {
        let icons: Vec<Vec<u8>> = apps
            .iter()
            .filter_map(|a| ulanzi_core::apps::app_icon_png(a, 64))
            .collect();
        if let Some(stack) = ulanzi_core::render::stacked_icons(&icons) {
            return crate::ui::canvas::texture(&stack);
        }
    }
    let dark = adw::StyleManager::default().is_dark();
    let look = KeyLook {
        source: chosen.source.clone().unwrap_or(ImageSource::Icon {
            set: IconSet::Lucide,
            name: "layout-grid".into(),
        }),
        background: Color([0, 0, 0, 0]),
        foreground: chosen
            .color
            .unwrap_or(if dark { Color::WHITE } else { Color::BLACK }),
        icon_scale: TILE_ICON_SCALE,
        ..KeyLook::default()
    };
    crate::ui::canvas::key_texture(&look, &store()?.profile_dir(id))
}

/// The name of the style that tints a row with `color`, added to the app's styles once. It's a thin
/// layer of the colour, so it sits well on light and dark themes alike.
fn tint_class(color: Color) -> Option<String> {
    let [r, g, b, _] = color.0;
    let name = format!("profile-tint-{r:02x}{g:02x}{b:02x}");
    let new = LOADED_TINTS.with(|t| t.borrow_mut().insert(name.clone()));
    if new {
        let hex = format!("#{r:02x}{g:02x}{b:02x}");
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!(
            "row.{name} {{ background-color: alpha({hex}, 0.16); }}\n\
             row.{name}:hover {{ background-color: alpha({hex}, 0.24); }}\n\
             row.{name}:selected {{ background-color: alpha({hex}, 0.32); }}"
        ));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default()?,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    Some(name)
}

thread_local! {
    static LOADED_TINTS: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

/// The apps a profile follows.
fn rule_apps(state: &State, id: uuid::Uuid) -> Vec<String> {
    state
        .rules
        .borrow()
        .get(&id)
        .map_or(Vec::new(), |r| r.apps.clone())
}

/// Change a profile's look and show it.
fn set_profile_look(state: &State, id: uuid::Uuid, f: impl FnOnce(&mut ProfileLook)) {
    let Some(store) = store() else { return };
    let mut look = store.profile_look(id);
    f(&mut look);
    if let Err(e) = store.save_profile_look(id, &look) {
        crate::ui::show_toast(&format!("Couldn't save the look: {e}"));
    }
    state.notify(Change::Profiles);
}

/// Change a profile's look by hand: from now on it's the user's, not an app's.
fn choose_profile_look(state: &State, id: uuid::Uuid, f: impl FnOnce(&mut ProfileLook)) {
    set_profile_look(state, id, |l| {
        l.from_app = None;
        f(l);
    });
}

/// After a profile's auto-switch apps change, let its look follow them.
pub fn follow_apps(state: &State, id: uuid::Uuid, apps: &[String]) {
    let current = store().map(|s| s.profile_look(id)).unwrap_or_default();
    let next = current.follow_apps(apps, |app| ProfileLook {
        source: Some(ImageSource::App {
            desktop_id: app.to_string(),
        }),
        color: ulanzi_core::apps::app_color(app),
        from_app: Some(app.to_string()),
    });
    if next != current {
        set_profile_look(state, id, |l| *l = next);
    }
}

/// A picture chosen for profile `id` lands in the open profile: copy it to the one it's for, whose
/// exports carry it, as they do a key's pictures.
fn keep_with_profile(state: &State, id: uuid::Uuid, source: &ImageSource) {
    if let (ImageSource::Image { path }, Some(store)) = (source, store())
        && id != state.profile_id()
    {
        let (from, to) = (
            store.profile_dir(state.profile_id()).join(path),
            store.profile_dir(id).join(path),
        );
        let copied = to
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::copy(&from, &to).map(|_| ()));
        if let Err(e) = copied {
            crate::ui::show_toast(&format!("Couldn't copy the picture: {e}"));
        }
    }
}

/// Reset asks first: the choice can't be got back.
fn confirm_look_reset(state: &State, near: &gtk::Button, id: uuid::Uuid, redraw: Rc<dyn Fn()>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Reset the Look?")
        .body("This profile goes back to the plain icon and no colour.")
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("reset", "Reset")]);
    dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
    let state = state.clone();
    dialog.connect_response(Some("reset"), move |_, _| {
        choose_profile_look(&state, id, |l| *l = ProfileLook::default());
        redraw();
    });
    dialog.present(Some(near));
}

/// Choose a profile's icon and colour. Changes apply as they're made, so there's nothing to save.
fn look_dialog(state: &State, near: &impl IsA<gtk::Widget>, id: uuid::Uuid, name: &str) {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    let preview = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    body.append(&preview);
    let swatches = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(6)
        .halign(gtk::Align::Center)
        .build();
    let marks: Rc<std::cell::RefCell<Vec<(Color, gtk::Button)>>> = Rc::default();
    let redraw: Rc<dyn Fn()> = {
        let (state, name, preview, marks) = (state.clone(), name.to_string(), preview.clone(), marks.clone());
        Rc::new(move || {
            let look = store().map(|s| s.profile_look(id)).unwrap_or_default();
            preview.remove_all();
            let apps = rule_apps(&state, id);
            preview.append(&profile_row(&name, "", false, &look, id, &apps));
            for (c, b) in marks.borrow().iter() {
                if look.color == Some(*c) {
                    b.add_css_class("swatch-current");
                } else {
                    b.remove_css_class("swatch-current");
                }
            }
        })
    };

    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let icon = adw::ActionRow::builder().title("Icon").build();
    let choose = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    {
        let (state, redraw) = (state.clone(), redraw.clone());
        choose.connect_clicked(move |b| {
            let saved = store().and_then(|s| s.profile_look(id).source);
            let (st, redraw) = (state.clone(), redraw.clone());
            crate::ui::icon_picker::choose(&state, b, saved, move |source| {
                keep_with_profile(&st, id, &source);
                choose_profile_look(&st, id, |l| l.source = Some(source));
                redraw();
            });
        });
    }
    icon.add_suffix(&choose);
    list.append(&icon);
    let custom = adw::ActionRow::builder().title("Custom Colour").build();
    {
        let (state, redraw) = (state.clone(), redraw.clone());
        let current = store().and_then(|s| s.profile_look(id).color);
        custom.add_suffix(&crate::ui::color_popover::color_button(
            current,
            |_| {},
            move |c| {
                choose_profile_look(&state, id, |l| l.color = Some(c));
                redraw();
            },
        ));
    }
    list.append(&custom);

    let no_preview: Rc<dyn Fn(Option<Color>)> = Rc::new(|_| {});
    for (i, (tint, hex)) in TINTS.iter().enumerate() {
        let Ok(c) = hex.parse::<Color>() else { continue };
        let apply: Rc<dyn Fn(Color)> = {
            let (state, redraw) = (state.clone(), redraw.clone());
            Rc::new(move |c| {
                choose_profile_look(&state, id, |l| l.color = Some(c));
                redraw();
            })
        };
        let b = crate::ui::color_popover::swatch_button(c, &no_preview, &apply);
        b.set_tooltip_text(Some(tint));
        marks.borrow_mut().push((c, b.clone()));
        swatches.attach(&b, i as i32, 0, 1, 1);
    }
    body.append(&swatches);
    body.append(&list);
    redraw();

    let reset = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("edit-undo-symbolic")
                .label("Reset")
                .build(),
        )
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    {
        let state = state.clone();
        reset.connect_clicked(move |b| confirm_look_reset(&state, b, id, redraw.clone()));
    }
    body.append(&reset);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    adw::Dialog::builder()
        .title("Icon and Colour")
        .content_width(380)
        .child(&view)
        .build()
        .present(Some(near));
}

fn profile_row(
    name: &str,
    subtitle: &str,
    active: bool,
    look: &ProfileLook,
    id: uuid::Uuid,
    apps: &[String],
) -> gtk::ListBoxRow {
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
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .pixel_size(TILE_PX)
        .build();
    if let Some(tile) = tile_texture(id, look, apps) {
        badge.set_paintable(Some(&tile));
    }
    badge.set_overflow(gtk::Overflow::Hidden);
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
    let row = gtk::ListBoxRow::builder().child(&b).build();
    if let Some(class) = look.color.and_then(tint_class) {
        row.add_css_class(&class);
    }
    row
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
            let mut at = 0;
            state.edit(|e| {
                let n = e.profile().pages.len() + 1;
                at = e.add_page(&format!("Page {n}"));
            });
            state.go_to_page(at);
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
                    let look = store().map(|s| s.profile_look(*id)).unwrap_or_default();
                    let row = profile_row(name, &sub, *id == current, &look, *id, &rule_apps(&state, *id));
                    attach_profile_menu(&state, &row, *id, name, i, list.len());
                    let st = state.clone();
                    reorderable(&row, "profile", i, move |from, to| move_profile(&st, from, to));
                    profiles.append(&row);
                    if *id == current {
                        profiles.select_row(Some(&row));
                    }
                }
                for name in state.too_new.borrow().iter() {
                    let row = profile_row(
                        name,
                        "Needs a newer version",
                        false,
                        &ProfileLook::default(),
                        uuid::Uuid::nil(),
                        &[],
                    );
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
                state.go_to_page(i);
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
    let auto = menu_item(PROFILE_MENU[2], "view-refresh-symbolic", &["flat"]);
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
    let look = menu_item(PROFILE_MENU[3], "applications-graphics-symbolic", &["flat"]);
    menu.append(&look);
    {
        let (state, name) = (state.clone(), name.to_string());
        look.connect_clicked(move |b| {
            if let Some(pop) = b
                .ancestor(gtk::Popover::static_type())
                .and_downcast::<gtk::Popover>()
            {
                pop.popdown();
            }
            look_dialog(&state, b, id, &name);
        });
    }
    let export = menu_item(PROFILE_MENU[4], "document-send-symbolic", &["flat"]);
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
    let save_template = menu_item(PROFILE_MENU[5], "document-save-symbolic", &["flat"]);
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
    let rename = menu_item(PROFILE_MENU[6], "document-edit-symbolic", &["flat"]);
    let delete = menu_item(PROFILE_MENU[7], "user-trash-symbolic", &["flat", "error"]);
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

/// A right-click menu item: an icon, then its label.
fn menu_item(label: &str, icon: &str, classes: &[&str]) -> gtk::Button {
    let content = gtk::Box::builder().spacing(10).build();
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    gtk::Button::builder()
        .child(&content)
        .css_classes(classes)
        .build()
}

/// "Move Up" and "Move Down" buttons for a list menu.
fn add_move_buttons(menu: &gtk::Box, index: usize, count: usize, on_move: impl Fn(usize, usize) + 'static) {
    let on_move = std::rc::Rc::new(on_move);
    for (label, icon, to) in [
        ("Move Up", "go-up-symbolic", index.checked_sub(1)),
        (
            "Move Down",
            "go-down-symbolic",
            Some(index + 1).filter(|t| *t < count),
        ),
    ] {
        let b = menu_item(label, icon, &["flat"]);
        b.set_sensitive(to.is_some());
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
    let rename = menu_item(PAGE_MENU[2], "document-edit-symbolic", &["flat"]);
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
