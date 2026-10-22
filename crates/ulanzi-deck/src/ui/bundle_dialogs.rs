//! Import and export `.ulanzideck` files.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use uuid::Uuid;

use crate::client::{ImportChoice, Snapshot};
use crate::runtime;
use crate::state::{Change, State};

/// A toast with an action button (e.g. Undo).
pub fn toast_with(text: &str, button: &str, on_click: impl Fn() + 'static) {
    let t = adw::Toast::builder()
        .title(text)
        .button_label(button)
        .timeout(8)
        .build();
    t.connect_button_clicked(move |_| on_click());
    crate::ui::add_toast(t);
}

/// Restore a snapshot and reload (used by Undo buttons and the Backups page).
pub fn restore(state: &State, name: &str, done: impl FnOnce(Option<String>) + 'static) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let (st, name) = (state.clone(), name.to_string());
    runtime::run(async move { c.restore_snapshot(&name).await }, move |r| match r {
        Ok(before) => {
            reload_open_profile(&st);
            done(Some(before));
        }
        Err(e) => {
            crate::ui::toast(&st, &format!("Couldn't restore: {e}"));
            done(None);
        }
    });
}

/// After an import or restore: refresh the profile list and reopen the profile being edited.
fn reload_open_profile(state: &State) {
    state.rules.borrow_mut().clear();
    let id = state.profile_id();
    if let Some(c) = state.client.borrow().clone() {
        let st = state.clone();
        runtime::run(async move { c.profiles().await }, move |r| {
            if let Ok(list) = r {
                let still_there = list.iter().any(|(i, _)| *i == id);
                let first = list.first().map(|(i, _)| *i);
                *st.profiles.borrow_mut() = list;
                st.notify(Change::Profiles);
                if let Some(open) = if still_there { Some(id) } else { first } {
                    crate::ui::window::open_profile(&st, open, false);
                }
            }
        });
    }
}

fn pick_file(parent: &impl IsA<gtk::Widget>, on_file: impl Fn(PathBuf) + 'static) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Ulanzi Deck profiles"));
    filter.add_pattern("*.ulanzideck");
    filter.add_mime_type("application/x-ulanzideck");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Import Profiles")
        .filters(&filters)
        .modal(true)
        .build();
    let window = parent.root().and_downcast::<gtk::Window>();
    dialog.open(window.as_ref(), None::<&gio::Cancellable>, move |r| {
        if let Some(p) = r.ok().and_then(|f| f.path()) {
            on_file(p);
        }
    });
}

/// Choose a file, then show the import dialog for it.
pub fn choose_and_import(state: &State, parent: &impl IsA<gtk::Widget>) {
    let (state, parent) = (state.clone(), parent.clone().upcast::<gtk::Widget>());
    pick_file(&parent.clone(), move |p| open_import(&state, &parent, &p));
}

/// Show what a file holds and import it.
pub fn open_import(state: &State, parent: &impl IsA<gtk::Widget>, path: &Path) {
    let Some(c) = state.client.borrow().clone() else {
        crate::ui::toast(state, "The Ulanzi Deck service isn't running");
        return;
    };
    let (st, parent, path) = (
        state.clone(),
        parent.clone().upcast::<gtk::Widget>(),
        path.to_path_buf(),
    );
    let p2 = path.clone();
    runtime::run(async move { c.preview_bundle(&p2).await }, move |r| match r {
        Ok(preview) => import_dialog(&st, &parent, &path, preview),
        Err(e) => crate::ui::toast(&st, &format!("Can't import that file: {e}")),
    });
}

fn import_dialog(state: &State, parent: &gtk::Widget, path: &Path, preview: crate::client::BundlePreview) {
    let page = adw::PreferencesPage::new();
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let group = adw::PreferencesGroup::builder()
        .title(format!(
            "{} Profile{}",
            preview.manifest.profiles.len(),
            if preview.manifest.profiles.len() == 1 {
                ""
            } else {
                "s"
            }
        ))
        .description(glib::markup_escape_text(&file_name).as_str())
        .build();
    for p in &preview.manifest.profiles {
        let exists = preview.conflicts.contains(&p.name);
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&p.name).as_str())
            .subtitle(format!(
                "{} page{} · {} key{} · {:?}{}",
                p.pages,
                if p.pages == 1 { "" } else { "s" },
                p.keys,
                if p.keys == 1 { "" } else { "s" },
                p.model,
                if exists { " · already here" } else { "" }
            ))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("view-grid-symbolic"));
        group.add(&row);
    }
    page.add(&group);

    let options = adw::PreferencesGroup::new();
    let conflict = adw::ComboRow::builder()
        .title("Profiles That Already Exist")
        .model(&gtk::StringList::new(&["Keep Both", "Replace Mine", "Skip Them"]))
        .visible(!preview.conflicts.is_empty())
        .build();
    options.add(&conflict);
    let switch_to = adw::SwitchRow::builder()
        .title("Switch to It After Importing")
        .active(true)
        .build();
    options.add(&switch_to);
    let rules = adw::SwitchRow::builder()
        .title("Keep Its Auto-Switch Apps")
        .subtitle("The apps that bring a profile to the front")
        .active(true)
        .build();
    options.add(&rules);
    let settings = adw::SwitchRow::builder()
        .title("Also Use Its Settings")
        .subtitle("Brightness, labels and other preferences in the file")
        .visible(preview.manifest.includes_settings)
        .build();
    options.add(&settings);
    page.add(&options);

    let import = gtk::Button::builder()
        .label("Import")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .margin_bottom(18)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    view.add_bottom_bar(&import);
    let dialog = adw::Dialog::builder()
        .title("Import Profiles")
        .content_width(460)
        .content_height(560)
        .child(&view)
        .build();
    {
        let (state, dialog, path) = (state.clone(), dialog.clone(), path.to_path_buf());
        import.connect_clicked(move |b| {
            let Some(c) = state.client.borrow().clone() else {
                return;
            };
            b.set_sensitive(false);
            let choice = match conflict.selected() {
                1 => ImportChoice::Replace,
                2 => ImportChoice::Skip,
                _ => ImportChoice::KeepBoth,
            };
            let (keep, set, sw) = (rules.is_active(), settings.is_active(), switch_to.is_active());
            let (st, dialog, path) = (state.clone(), dialog.clone(), path.clone());
            runtime::run(
                async move { c.import_bundle(&path, choice, keep, set, sw).await },
                move |r| {
                    dialog.close();
                    match r {
                        Ok(done) => {
                            reload_open_profile(&st);
                            for w in &done.warnings {
                                crate::ui::toast(&st, w);
                            }
                            let n = done.imported.len();
                            let st2 = st.clone();
                            toast_with(
                                &format!("Imported {n} profile{}", if n == 1 { "" } else { "s" }),
                                "Undo",
                                move || restore(&st2, &done.undo, |_| {}),
                            );
                        }
                        Err(e) => crate::ui::toast(&st, &format!("Import failed: {e}")),
                    }
                },
            );
        });
    }
    dialog.present(Some(parent));
}

/// Choose profiles and save them to a file.
pub fn open_export(state: &State, parent: &impl IsA<gtk::Widget>, preselect: Option<Uuid>) {
    let profiles = state.profiles.borrow().clone();
    let current = preselect.unwrap_or_else(|| state.profile_id());
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
        .title("Profiles to Export")
        .description("Pictures on the keys go into the file too")
        .build();
    let chosen: Rc<RefCell<Vec<Uuid>>> = Rc::new(RefCell::new(vec![current]));
    for (id, name) in &profiles {
        let check = gtk::CheckButton::builder().active(*id == current).build();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(name).as_str())
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        let (chosen, id) = (chosen.clone(), *id);
        check.connect_toggled(move |c| {
            let mut v = chosen.borrow_mut();
            v.retain(|x| *x != id);
            if c.is_active() {
                v.push(id);
            }
        });
        group.add(&row);
    }
    page.add(&group);
    let more = adw::PreferencesGroup::new();
    let settings = adw::SwitchRow::builder()
        .title("Include Settings")
        .subtitle("Brightness, labels and other preferences. Never your OBS password.")
        .build();
    more.add(&settings);
    page.add(&more);

    let save = gtk::Button::builder()
        .label("Save…")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .margin_bottom(18)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    view.add_bottom_bar(&save);
    let dialog = adw::Dialog::builder()
        .title("Export Profiles")
        .content_width(420)
        .content_height(520)
        .child(&view)
        .build();
    {
        let (state, dialog) = (state.clone(), dialog.clone());
        save.connect_clicked(move |b| {
            let ids = chosen.borrow().clone();
            if ids.is_empty() {
                crate::ui::toast(&state, "Choose at least one profile");
                return;
            }
            let first_name = state
                .profiles
                .borrow()
                .iter()
                .find(|(i, _)| *i == ids[0])
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "Profiles".into());
            let name = if ids.len() == 1 {
                format!("{first_name}.ulanzideck")
            } else {
                "Ulanzi Deck Profiles.ulanzideck".into()
            };
            let fd = gtk::FileDialog::builder()
                .title("Export Profiles")
                .initial_name(name)
                .modal(true)
                .build();
            let window = b.root().and_downcast::<gtk::Window>();
            let (st, dialog, with_settings) = (state.clone(), dialog.clone(), settings.is_active());
            fd.save(window.as_ref(), None::<&gio::Cancellable>, move |r| {
                let Some(mut path) = r.ok().and_then(|f| f.path()) else {
                    return;
                };
                if path.extension().is_none_or(|e| e != "ulanzideck") {
                    path.set_extension("ulanzideck");
                }
                let Some(c) = st.client.borrow().clone() else {
                    return;
                };
                let (st, dialog) = (st.clone(), dialog.clone());
                let shown = path.display().to_string();
                runtime::run(
                    async move { c.export_bundle(&path, &ids, with_settings).await },
                    move |r| match r {
                        Ok(()) => {
                            dialog.close();
                            crate::ui::toast(&st, &format!("Saved {shown}"));
                        }
                        Err(e) => crate::ui::toast(&st, &format!("Couldn't save: {e}")),
                    },
                );
            });
        });
    }
    dialog.present(Some(parent));
}

/// "Before import" for a snapshot reason.
pub fn reason_words(reason: &str) -> String {
    match reason {
        "before-import" => "Before import".into(),
        "before-restore" => "Before restoring a backup".into(),
        "before-delete" => "Before deleting a profile".into(),
        "before-migration" => "Before importing the old setup".into(),
        "daily" => "Daily backup".into(),
        "manual" => "Backed up by you".into(),
        other => other.to_string(),
    }
}

/// Snapshot names for the Backups page, newest first.
pub fn list(state: &State, done: impl FnOnce(Vec<Snapshot>) + 'static) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    runtime::run(async move { c.list_snapshots().await }, move |r| {
        done(r.unwrap_or_default())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_in_words() {
        assert_eq!(reason_words("before-import"), "Before import");
        assert_eq!(reason_words("daily"), "Daily backup");
        assert_eq!(reason_words("something-else"), "something-else");
    }
}
