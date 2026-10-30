//! Telling people about a new version: the banner, What's New and the update itself.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::update::{InstallKind, Release, UpdateStatus, notes_markup, split_notes};

use crate::runtime;
use crate::state::{Change, State};
use crate::updates::InstallFailure;

const RELEASES: &str = "https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/releases";
/// How often an open window asks the service what the daily check found.
const REFRESH: Duration = Duration::from_secs(60 * 60);

thread_local! {
    /// The open dialog's progress bar, while an update downloads.
    static BAR: RefCell<glib::WeakRef<gtk::ProgressBar>> = RefCell::default();
}

/// The update button's label and what to expect from it.
pub fn button_text(kind: InstallKind) -> (&'static str, Option<&'static str>) {
    match kind {
        InstallKind::Arch | InstallKind::Deb | InstallKind::Rpm => {
            ("Update", Some("You'll be asked for your password"))
        }
        _ => ("Update", None),
    }
}

/// The banner's sentence: the first line of the important note when there is one.
pub fn banner_title(release: &Release) -> String {
    match split_notes(&release.notes).0 {
        Some(note) => note.lines().next().unwrap_or_default().to_string(),
        None => format!("Ulanzi Deck {} is available", release.version()),
    }
}

fn page_of(release: Option<&Release>) -> String {
    match release {
        Some(r) => format!("{RELEASES}/tag/{}", r.tag),
        None => RELEASES.to_string(),
    }
}

fn open_page(widget: &impl IsA<gtk::Widget>, uri: &str) {
    let window = widget.root().and_downcast::<gtk::Window>();
    gtk::UriLauncher::new(uri).launch(window.as_ref(), None::<&gtk::gio::Cancellable>, |_| {});
}

/// Ask the service what it knows, and show What's New once after an update.
pub fn refresh(state: &State) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let st = state.clone();
    runtime::run(async move { c.update_status().await }, move |r| {
        if let Ok(status) = r {
            let first_start = status.whats_new_shown.as_deref() != Some(status.current.as_str());
            let installed = status.current_release.is_some();
            let version = status.current.clone();
            *st.update.borrow_mut() = Some(status);
            st.notify(Change::Update);
            if first_start {
                if let Some(c) = st.client.borrow().clone() {
                    let v = version.clone();
                    runtime::run(async move { c.update_whats_new_shown(&v).await }, |_| {});
                }
                if installed && let Some(window) = st.window.borrow().clone() {
                    open(&st, &window);
                }
            }
        }
    });
}

/// The banner on top of the main window, kept up to date.
pub fn banner(state: &State) -> adw::Banner {
    let banner = adw::Banner::new("");
    banner.set_button_label(Some("See What's New"));
    {
        let state = state.clone();
        banner.connect_button_clicked(move |b| open(&state, b));
    }
    let update = {
        let (state, banner) = (state.clone(), banner.clone());
        move |c: Change| {
            if c != Change::Update {
                return;
            }
            let offered = state.update.borrow().as_ref().and_then(|s| s.offered.clone());
            if let Some(r) = &offered {
                banner.set_title(&banner_title(r));
            }
            banner.set_revealed(offered.is_some() && !state.update_dismissed.get());
        }
    };
    state.on(update);
    {
        let state = state.clone();
        glib::timeout_add_local(REFRESH, move || {
            refresh(&state);
            glib::ControlFlow::Continue
        });
    }
    banner
}

/// Show `status`'s release: the one on offer, or the one this copy is.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>) {
    let status = state.update.borrow().clone();
    let Some(status) = status else {
        open_page(parent, RELEASES);
        return;
    };
    let Some(release) = status.offered.clone().or_else(|| status.current_release.clone()) else {
        open_page(parent, RELEASES);
        return;
    };
    dialog(state, parent, &status, &release);
}

fn date_of(published: &str) -> &str {
    published.split('T').next().unwrap_or(published)
}

fn dialog(state: &State, parent: &impl IsA<gtk::Widget>, status: &UpdateStatus, release: &Release) {
    let offered = status.offered.is_some();
    let dialog = adw::Dialog::builder()
        .title("What's New")
        .content_width(520)
        .content_height(560)
        .build();
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_start(18)
        .margin_end(18)
        .margin_top(6)
        .margin_bottom(12)
        .build();
    body.append(
        &gtk::Label::builder()
            .label(format!("Ulanzi Deck {}", release.version()))
            .css_classes(["title-2"])
            .xalign(0.0)
            .build(),
    );
    body.append(
        &gtk::Label::builder()
            .label(date_of(&release.published))
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .build(),
    );
    let (important, rest) = split_notes(&release.notes);
    if let Some(note) = important {
        let label = gtk::Label::builder()
            .use_markup(true)
            .label(notes_markup(&note))
            .wrap(true)
            .xalign(0.0)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .margin_bottom(10)
            .build();
        let card = gtk::Box::builder().css_classes(["card"]).build();
        card.append(&label);
        body.append(&card);
    }
    body.append(
        &gtk::Label::builder()
            .use_markup(true)
            .label(notes_markup(&rest))
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&body)
        .build();

    let (label, hint) = button_text(status.kind);
    let bar = gtk::ProgressBar::builder().visible(false).build();
    let note = gtk::Label::builder()
        .label(hint.unwrap_or_default())
        .css_classes(["dim-label", "caption"])
        .wrap(true)
        .visible(offered && status.can_install && hint.is_some())
        .build();
    let update = gtk::Button::builder()
        .label(label)
        .css_classes(["suggested-action", "pill"])
        .visible(offered && status.can_install)
        .build();
    let later = gtk::Button::builder().label("Not Now").visible(offered).build();
    let skip = gtk::Button::builder()
        .label("Skip This Version")
        .visible(offered)
        .build();
    let site = gtk::Button::builder().label("View on GitHub").build();
    let buttons = gtk::Box::builder().spacing(6).halign(gtk::Align::Center).build();
    buttons.append(&update);
    buttons.append(&later);
    buttons.append(&skip);
    buttons.append(&site);
    let foot = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(18)
        .margin_end(18)
        .margin_top(6)
        .margin_bottom(18)
        .build();
    foot.append(&bar);
    foot.append(&note);
    foot.append(&buttons);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroll));
    view.add_bottom_bar(&foot);
    dialog.set_child(Some(&view));

    {
        let uri = page_of(Some(release));
        site.connect_clicked(move |b| open_page(b, &uri));
    }
    {
        let (state, dialog) = (state.clone(), dialog.clone());
        later.connect_clicked(move |_| {
            state.update_dismissed.set(true);
            state.notify(Change::Update);
            dialog.close();
        });
    }
    {
        let (state, dialog, version) = (state.clone(), dialog.clone(), release.version().to_string());
        skip.connect_clicked(move |_| {
            if let Some(c) = state.client.borrow().clone() {
                let (st, version) = (state.clone(), version.clone());
                runtime::run(async move { c.update_skip(&version).await }, move |_| {
                    refresh(&st)
                });
            }
            dialog.close();
        });
    }
    {
        let state = state.clone();
        let (bar, note) = (bar.clone(), note.clone());
        let kind = status.kind;
        update.connect_clicked(move |b| start(&state, b, &bar, &note, kind));
    }
    dialog.present(Some(parent));
}

/// Show a problem under the buttons, and let the person try again.
fn stopped(button: &gtk::Button, bar: &gtk::ProgressBar, note: &gtk::Label, message: Option<String>) {
    bar.set_visible(false);
    button.set_sensitive(true);
    if let Some(m) = message {
        button.set_label("Try Again");
        note.set_label(&m);
        note.set_visible(true);
    }
}

fn start(state: &State, button: &gtk::Button, bar: &gtk::ProgressBar, note: &gtk::Label, kind: InstallKind) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    button.set_sensitive(false);
    bar.set_fraction(0.0);
    bar.set_visible(true);
    BAR.with(|b| b.borrow().set(Some(bar)));
    let (button, bar, note) = (button.clone(), bar.clone(), note.clone());
    runtime::run(
        async move {
            let file = c.update_download().await.map_err(|e| e.to_string())?;
            c.update_apply(&file).await.map_err(|e| e.to_string())?;
            Ok::<_, String>(file)
        },
        move |r| match r {
            Err(e) => stopped(&button, &bar, &note, Some(e)),
            Ok(file) => install(&button, &bar, &note, kind, PathBuf::from(file)),
        },
    );
}

fn install(
    button: &gtk::Button,
    bar: &gtk::ProgressBar,
    note: &gtk::Label,
    kind: InstallKind,
    file: PathBuf,
) {
    let (button, bar, note) = (button.clone(), bar.clone(), note.clone());
    runtime::run(
        async move { crate::updates::install(kind, &file).await },
        move |r| match r {
            Ok(()) => finish(&button, &bar, &note),
            Err(InstallFailure::Cancelled) => stopped(&button, &bar, &note, None),
            Err(InstallFailure::Failed(e)) => stopped(&button, &bar, &note, Some(e)),
        },
    );
}

fn finish(button: &gtk::Button, bar: &gtk::ProgressBar, note: &gtk::Label) {
    let app = button
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|w| w.application());
    let (b, bar, note) = (button.clone(), bar.clone(), note.clone());
    runtime::run(async { crate::updates::restart().await }, move |r| match r {
        Ok(()) => {
            if let Some(app) = app {
                app.quit();
            }
        }
        Err(e) => stopped(
            &b,
            &bar,
            &note,
            Some(format!("Updated. Restart Ulanzi Deck to finish: {e}")),
        ),
    });
}

/// Called for each `UpdateProgress` signal.
pub fn progress(stage: &str, done: u64, total: u64) {
    let Some(bar) = BAR.with(|b| b.borrow().upgrade()) else {
        return;
    };
    match stage {
        "downloading" if total > 0 => bar.set_fraction(done as f64 / total as f64),
        _ => bar.set_fraction(1.0),
    }
}

/// The Updates group for Preferences.
pub fn preferences_group(state: &State) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Updates").build();
    let status = state.update.borrow().clone();
    let check = adw::SwitchRow::builder()
        .title("Check for Updates")
        .subtitle("Asks GitHub once a day whether a new version is out")
        .active(status.as_ref().is_none_or(|s| s.check))
        .build();
    {
        let state = state.clone();
        check.connect_active_notify(move |r| {
            if let Some(c) = state.client.borrow().clone() {
                let on = r.is_active();
                runtime::run(async move { c.update_set_check(on).await }, |_| {});
            }
        });
    }
    group.add(&check);
    let version = adw::ActionRow::builder()
        .title("Version")
        .subtitle(env!("CARGO_PKG_VERSION"))
        .css_classes(["property"])
        .build();
    group.add(&version);
    let now = adw::ButtonRow::builder().title("Check Now").build();
    {
        let state = state.clone();
        now.connect_activated(move |row| {
            let Some(c) = state.client.borrow().clone() else {
                return;
            };
            row.set_sensitive(false);
            let (st, row) = (state.clone(), row.clone());
            runtime::run(async move { c.update_check().await }, move |r| {
                row.set_sensitive(true);
                match r {
                    Ok(status) => {
                        let found = status.offered.is_some();
                        *st.update.borrow_mut() = Some(status);
                        st.notify(Change::Update);
                        if found {
                            open(&st, &row);
                        } else {
                            crate::ui::toast(&st, "Ulanzi Deck is up to date");
                        }
                    }
                    Err(e) => crate::ui::toast(&st, &e.to_string()),
                }
            });
        });
    }
    group.add(&now);
    let news = adw::ButtonRow::builder().title("What's New").build();
    {
        let state = state.clone();
        news.connect_activated(move |row| open(&state, row));
    }
    group.add(&news);
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(notes: &str) -> Release {
        Release {
            tag: "v2.1.2".into(),
            title: String::new(),
            published: "2026-11-02T10:00:00Z".into(),
            notes: notes.into(),
            assets: Vec::new(),
        }
    }

    #[test]
    fn the_update_button_says_what_will_happen() {
        assert_eq!(
            button_text(InstallKind::Arch).1,
            Some("You'll be asked for your password")
        );
        assert_eq!(button_text(InstallKind::AppImage).1, None);
    }

    #[test]
    fn the_banner_prefers_the_important_note() {
        assert_eq!(
            banner_title(&release("### New\n\n- A")),
            "Ulanzi Deck 2.1.2 is available"
        );
        assert_eq!(
            banner_title(&release("### Important\n\nBack up first.\nAnother line.")),
            "Back up first."
        );
    }

    #[test]
    fn a_release_links_to_its_own_page() {
        assert!(page_of(Some(&release(""))).ends_with("/releases/tag/v2.1.2"));
        assert_eq!(date_of("2026-11-02T10:00:00Z"), "2026-11-02");
    }
}
