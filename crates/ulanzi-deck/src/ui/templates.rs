//! New profiles from templates, and saving a profile as a template.

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::model::{Profile, ScreenConfig};
use ulanzi_core::template::{self, Template};
use uuid::Uuid;

use crate::client::ImportChoice;
use crate::runtime;
use crate::state::State;
use crate::ui::canvas::key_texture;

const MINI: i32 = 26;

/// A small drawing of the template's first page.
fn mini_deck(p: &Profile) -> gtk::Widget {
    let grid = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(4)
        .halign(gtk::Align::Center)
        .css_classes(["mini-deck"])
        .build();
    let page = &p.pages[0];
    let dir = std::path::Path::new("/nonexistent");
    let mut i = 0u8;
    for row in 0..3 {
        for col in 0..5 {
            if row == 2 && col == 3 {
                let screen = gtk::Box::builder().css_classes(["mini-screen"]).build();
                screen.set_size_request(MINI * 2 + 4, MINI);
                if matches!(p.screen, ScreenConfig::Custom { .. }) {
                    screen.append(
                        &gtk::Image::builder()
                            .icon_name("audio-x-generic-symbolic")
                            .hexpand(true)
                            .build(),
                    );
                } else {
                    screen.append(
                        &gtk::Label::builder()
                            .label("12:30")
                            .hexpand(true)
                            .css_classes(["caption-heading"])
                            .build(),
                    );
                }
                grid.attach(&screen, 3, 2, 2, 1);
                break;
            }
            let cell = gtk::Picture::builder()
                .can_shrink(true)
                .content_fit(gtk::ContentFit::Cover)
                .css_classes(["mini-key"])
                .build();
            cell.set_size_request(MINI, MINI);
            if let Some(t) = page.keys.get(&i).and_then(|k| key_texture(&k.look, dir)) {
                cell.set_paintable(Some(&t));
            }
            grid.attach(&cell, col, row, 1, 1);
            i += 1;
        }
    }
    grid.upcast()
}

/// Create a profile from `t` and switch to it.
fn create(state: &State, t: &Template, done: impl FnOnce(bool) + 'static) {
    let bytes = template::instantiate(t).and_then(|b| ulanzi_core::bundle::to_bytes(&b));
    let (Ok(bytes), Some(c)) = (bytes, state.client.borrow().clone()) else {
        crate::ui::toast(state, "Couldn't create the profile");
        done(false);
        return;
    };
    let path = glib::user_cache_dir().join(format!("ulanzi-deck-template-{}.ulanzideck", Uuid::new_v4()));
    if std::fs::write(&path, bytes).is_err() {
        crate::ui::toast(state, "Couldn't create the profile");
        done(false);
        return;
    }
    let (st, name) = (state.clone(), t.name.clone());
    runtime::run(
        async move {
            let r = c
                .import_bundle(&path, ImportChoice::KeepBoth, true, false, true)
                .await;
            let _ = std::fs::remove_file(&path);
            r
        },
        move |r| match r {
            Ok(_) => {
                crate::ui::bundle_dialogs::reload_open_profile(&st);
                crate::ui::toast(&st, &format!("Created “{name}”"));
                done(true);
            }
            Err(e) => {
                crate::ui::toast(&st, &format!("Couldn't create the profile: {e}"));
                done(false);
            }
        },
    );
}

/// The template chooser.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>) {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(2)
        .max_children_per_line(3)
        .column_spacing(12)
        .row_spacing(12)
        .homogeneous(true)
        .margin_start(18)
        .margin_end(18)
        .margin_top(12)
        .margin_bottom(18)
        .build();
    let dialog = adw::Dialog::builder()
        .title("New Profile from Template")
        .content_width(720)
        .content_height(600)
        .build();
    for t in template::list() {
        let Ok(b) = template::instantiate(&t) else {
            continue;
        };
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .build();
        card.append(&mini_deck(&b.profiles[0].profile));
        card.append(
            &gtk::Label::builder()
                .label(&t.name)
                .css_classes(["heading"])
                .xalign(0.0)
                .build(),
        );
        card.append(
            &gtk::Label::builder()
                .label(&t.description)
                .css_classes(["dim-label", "caption"])
                .wrap(true)
                .xalign(0.0)
                .max_width_chars(30)
                .build(),
        );
        let button = gtk::Button::builder()
            .child(&card)
            .css_classes(["card", "template-card"])
            .build();
        let (state, dialog, t) = (state.clone(), dialog.clone(), t.clone());
        button.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (dialog, b) = (dialog.clone(), b.clone());
            create(&state, &t, move |ok| {
                if ok {
                    dialog.close();
                } else {
                    b.set_sensitive(true);
                }
            });
        });
        flow.append(&button);
    }
    let note = gtk::Label::builder()
        .label("Make your own: right-click a profile → Save as Template…")
        .css_classes(["dim-label", "caption"])
        .margin_bottom(12)
        .build();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&flow);
    column.append(&note);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&column)
            .build(),
    ));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// A file name for a template from a profile name.
pub fn file_name(profile_name: &str) -> String {
    let stem: String = profile_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim()
        .to_string();
    format!("{}.ulanzideck", if stem.is_empty() { "Template" } else { &stem })
}

/// Save profile `id` into the user's templates folder.
pub fn save_as_template(state: &State, id: Uuid, name: &str) {
    let (Some(dir), Some(c)) = (template::user_dir(), state.client.borrow().clone()) else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::ui::toast(state, &format!("Couldn't save the template: {e}"));
        return;
    }
    let path = dir.join(file_name(name));
    let (st, name) = (state.clone(), name.to_string());
    runtime::run(
        async move { c.export_bundle(&path, &[id], false).await },
        move |r| match r {
            Ok(()) => crate::ui::toast(&st, &format!("Saved “{name}” as a template")),
            Err(e) => crate::ui::toast(&st, &format!("Couldn't save the template: {e}")),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_file_names_are_safe() {
        assert_eq!(file_name("My Edit / Layout"), "My Edit - Layout.ulanzideck");
        assert_eq!(file_name("../.."), "-----.ulanzideck");
        assert_eq!(file_name(""), "Template.ulanzideck");
    }
}
