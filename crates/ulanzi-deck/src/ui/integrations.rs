//! Settings → Integrations: installed plugins, their settings pages and developer installs.
//! The words and decisions come from [`crate::plugins`]; this is the GTK around them.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use ulanzi_core::plugin::{FieldKind, Inspected, Manifest, describe};

use crate::client::{Client, ClientError};
use crate::plugins::{PluginRow, StatusWords, banner, current, status_words};
use crate::runtime;
use crate::state::{Change, State};
use crate::ui::window::refresh_plugins;
use crate::ui::{plugin_fields, preferences};

/// The rows of the install dialog, in order: (title, subtitle).
pub fn install_rows(i: &Inspected) -> Vec<(String, String)> {
    let m = &i.manifest;
    let mut rows = vec![
        (
            "Unverified plugin".to_string(),
            "Not from the Ulanzi Deck marketplace. Only install plugins you trust.".to_string(),
        ),
        (format!("{} {}", m.name, m.version), m.description.clone()),
    ];
    if let Some(v) = &i.installed_version {
        rows.push((format!("Replaces version {v}"), String::new()));
    }
    rows.extend(describe(&m.permissions).into_iter().map(|l| (l, String::new())));
    rows
}

/// A status in one line for the plugin list: the title, then the daemon's detail.
pub fn status_line(w: &StatusWords) -> String {
    if w.subtitle.trim().is_empty() {
        w.title.clone()
    } else {
        format!("{} · {}", w.title, w.subtitle)
    }
}

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn toast(dialog: &adw::PreferencesDialog, text: &str) {
    dialog.add_toast(adw::Toast::builder().title(text).use_markup(false).build());
}

fn status_icon(w: &StatusWords) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(w.icon);
    icon.add_css_class(w.css);
    icon
}

/// Show the plugins as the app knows them again (and fetch the list again), so a switch flipped
/// by hand shows the truth after a call failed. Deferred: it may run inside that switch's signal.
fn show_truth(state: &State) {
    let state = state.clone();
    glib::idle_add_local_once(move || {
        state.notify(Change::Plugins);
        refresh_plugins(&state);
    });
}

/// Run a call on the service. An error (or no service) becomes a toast, and the plugins are shown
/// as they really are again.
fn call<T, F>(
    state: &State,
    dialog: &adw::PreferencesDialog,
    f: impl FnOnce(Client) -> F,
    ok: impl FnOnce(T) + 'static,
) where
    T: Send + 'static,
    F: Future<Output = Result<T, ClientError>> + Send + 'static,
{
    let Some(c) = state.client.borrow().clone() else {
        toast(dialog, &ClientError::NotRunning.to_string());
        show_truth(state);
        return;
    };
    let (state, dialog) = (state.clone(), dialog.downgrade());
    runtime::run(f(c), move |r| match r {
        Ok(v) => ok(v),
        Err(e) => {
            if let Some(d) = dialog.upgrade() {
                toast(&d, &e.to_string());
            }
            show_truth(&state);
        }
    });
}

fn set_enabled(state: &State, dialog: &adw::PreferencesDialog, id: &str, on: bool) {
    let (st, key) = (state.clone(), id.to_string());
    let id = id.to_string();
    call(
        state,
        dialog,
        move |c| async move { c.plugin_set_enabled(&id, on).await },
        move |()| {
            st.settings.borrow_mut().plugins.entry(key).or_default().enabled = on;
        },
    );
}

fn restart(state: &State, dialog: &adw::PreferencesDialog, id: &str) {
    let id = id.to_string();
    call(
        state,
        dialog,
        move |c| async move { c.plugin_restart(&id).await },
        |()| {},
    );
}

fn plugin_row(state: &State, dialog: &adw::PreferencesDialog, p: &PluginRow) -> adw::ActionRow {
    let w = status_words(p);
    let row = adw::ActionRow::builder()
        .title(escape(p.display_name()))
        .subtitle(escape(&status_line(&w)))
        .activatable(true)
        .build();
    row.add_prefix(&status_icon(&w));
    let switch = gtk::Switch::builder()
        .active(p.status != "off")
        .valign(gtk::Align::Center)
        .tooltip_text("Turned On")
        .build();
    {
        let (state, dialog, id) = (state.clone(), dialog.downgrade(), p.id.clone());
        switch.connect_active_notify(move |s| {
            if let Some(d) = dialog.upgrade() {
                set_enabled(&state, &d, &id, s.is_active());
            }
        });
    }
    row.add_suffix(&switch);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let (state, dialog, id) = (state.clone(), dialog.downgrade(), p.id.clone());
        row.connect_activated(move |_| {
            if let Some(d) = dialog.upgrade() {
                d.push_subpage(&subpage(&state, &d, &id));
            }
        });
    }
    row
}

/// The installed plugins: status, on/off, and a settings page each.
pub fn plugins_group(state: &State, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Plugins").build();
    // Weak: the listener below outlives the dialog until it reports the group gone.
    let rows: RefCell<Vec<glib::WeakRef<adw::ActionRow>>> = RefCell::default();
    // What the rows show now. A change that shows the same (a manifest reload, say) keeps the
    // rows, so a focused switch or a button does not lose focus while a status flips elsewhere.
    let shown: RefCell<Option<(bool, Vec<PluginRow>)>> = RefCell::default();
    let fill = {
        let (state, group, dialog) = (state.clone(), group.downgrade(), dialog.downgrade());
        move || {
            let (Some(group), Some(dialog)) = (group.upgrade(), dialog.upgrade()) else {
                return false;
            };
            let plugins = state.plugins.borrow().clone();
            let now = (plugins.loaded, plugins.list.clone());
            if shown.borrow().as_ref() == Some(&now) {
                return true;
            }
            *shown.borrow_mut() = Some(now);
            for r in rows.borrow_mut().drain(..) {
                if let Some(r) = r.upgrade() {
                    group.remove(&r);
                }
            }
            let add = |r: adw::ActionRow| {
                group.add(&r);
                rows.borrow_mut().push(r.downgrade());
            };
            if !plugins.loaded {
                add(adw::ActionRow::builder().title("Connecting…").build());
            } else if plugins.list.is_empty() {
                add(adw::ActionRow::builder().title("No plugins installed").build());
            } else {
                for p in &plugins.list {
                    add(plugin_row(&state, &dialog, p));
                }
            }
            true
        }
    };
    fill();
    state.on_while(move |c| if c == Change::Plugins { fill() } else { true });
    group
}

/// Developer Mode, and installing a plugin from a folder while it is on.
pub fn developer_group(state: &State, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let on = state.settings.borrow().developer_mode;
    let mode = adw::SwitchRow::builder()
        .title("Developer Mode")
        .subtitle("Install plugins from a folder. For plugin authors.")
        .active(on)
        .build();
    let install = adw::ButtonRow::builder()
        .title("Install from Folder…")
        .start_icon_name("folder-open-symbolic")
        .visible(on)
        .build();
    {
        let (state, install) = (state.clone(), install.downgrade());
        mode.connect_active_notify(move |r| {
            let v = r.is_active();
            preferences::change_now(&state, |s| s.developer_mode = v);
            if let Some(i) = install.upgrade() {
                i.set_visible(v);
            }
        });
    }
    {
        let (state, dialog) = (state.clone(), dialog.downgrade());
        install.connect_activated(move |_| {
            if let Some(d) = dialog.upgrade() {
                install_flow(&state, &d);
            }
        });
    }
    group.add(&mode);
    group.add(&install);
    group
}

fn cant_install(dialog: &adw::PreferencesDialog, why: &str) {
    let alert = adw::AlertDialog::builder()
        .heading("Can’t Install This Folder")
        .body(why)
        .default_response("close")
        .close_response("close")
        .build();
    alert.add_response("close", "Close");
    alert.present(Some(dialog));
}

/// Choose a plugin folder, show what it is and what it may do, and install it on "Install".
pub fn install_flow(state: &State, dialog: &adw::PreferencesDialog) {
    let chooser = gtk::FileDialog::builder()
        .title("Choose a Plugin Folder")
        .modal(true)
        .build();
    let window = dialog.root().and_downcast::<gtk::Window>();
    let (state, dialog) = (state.clone(), dialog.clone());
    chooser.select_folder(window.as_ref(), None::<&gio::Cancellable>, move |r| {
        // An error here is the chooser being closed.
        let Ok(folder) = r else { return };
        let Some(path) = folder.path() else {
            cant_install(&dialog, "The folder must be on this computer.");
            return;
        };
        let Some(c) = state.client.borrow().clone() else {
            cant_install(&dialog, &ClientError::NotRunning.to_string());
            return;
        };
        let p = path.clone();
        runtime::run(async move { c.plugin_inspect_dir(&p).await }, move |r| match r {
            Ok(i) => confirm_install(&state, &dialog, path, &i),
            Err(e) => cant_install(&dialog, &e.to_string()),
        });
    });
}

fn confirm_install(state: &State, dialog: &adw::PreferencesDialog, path: PathBuf, i: &Inspected) {
    let name = i.manifest.name.clone();
    let alert = adw::AlertDialog::builder()
        .heading(format!("Install {name}?"))
        .default_response("cancel")
        .close_response("cancel")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    for (n, (title, subtitle)) in install_rows(i).into_iter().enumerate() {
        let row = adw::ActionRow::builder().title(escape(&title)).build();
        if !subtitle.is_empty() {
            row.set_subtitle(&escape(&subtitle));
        }
        if n == 0 {
            let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
            icon.add_css_class("warning");
            row.add_prefix(&icon);
        }
        list.append(&row);
    }
    alert.set_extra_child(Some(&list));
    alert.add_response("cancel", "Cancel");
    alert.add_response("install", "Install");
    alert.set_response_appearance("install", adw::ResponseAppearance::Suggested);
    {
        let (state, dialog) = (state.clone(), dialog.clone());
        alert.connect_response(Some("install"), move |_, _| {
            let Some(c) = state.client.borrow().clone() else {
                cant_install(&dialog, &ClientError::NotRunning.to_string());
                return;
            };
            let (state, dialog, path, name) = (state.clone(), dialog.clone(), path.clone(), name.clone());
            runtime::run(
                async move { c.plugin_install_dir(&path).await },
                move |r| match r {
                    Ok(_) => {
                        toast(&dialog, &format!("{name} installed"));
                        refresh_plugins(&state);
                    }
                    Err(e) => cant_install(&dialog, &e.to_string()),
                },
            );
        });
    }
    alert.present(Some(dialog));
}

/// A password row for one secret setting. Never filled in: the app can't read secrets back.
/// Widgets are held weakly here and below: the page's widgets hold the [`Page`], not the reverse.
struct SecretRow {
    key: String,
    label: String,
    row: glib::WeakRef<adw::PasswordEntryRow>,
    clear: glib::WeakRef<gtk::Button>,
}

/// The status widgets updated in place when the plugin's status changes.
struct StatusRows {
    row: glib::WeakRef<adw::ActionRow>,
    icon: glib::WeakRef<gtk::Image>,
    on: glib::WeakRef<adw::SwitchRow>,
}

/// One plugin's settings page. Lives while its widgets or its listener (until the page is gone)
/// hold it.
struct Page {
    state: State,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
    id: String,
    nav: glib::WeakRef<adw::NavigationPage>,
    view: glib::WeakRef<adw::ToolbarView>,
    banner: glib::WeakRef<adw::Banner>,
    /// The manifest the groups were built from (`None` inside: built before it was known).
    built: RefCell<Option<Option<Manifest>>>,
    status: RefCell<Option<StatusRows>>,
    secrets: RefCell<Vec<SecretRow>>,
    /// Set while the page updates its own switch, so that is not taken as the user's.
    syncing: Cell<bool>,
    /// Popped (by the user, after removing, or because the plugin went away).
    gone: Cell<bool>,
    /// On screen (pushed and its transition done): only then may it pop itself.
    shown: Cell<bool>,
    /// The plugin was in the list at some point (a page opened for an unknown id never pops).
    seen: Cell<bool>,
}

impl Page {
    fn name(&self) -> String {
        let plugins = self.state.plugins.borrow();
        plugins
            .row(&self.id)
            .map(|r| r.display_name().to_string())
            .or_else(|| plugins.manifests.get(&self.id).map(|m| m.name.clone()))
            .unwrap_or_else(|| self.id.clone())
    }
}

fn status_group(
    p: &Rc<Page>,
    dialog: &adw::PreferencesDialog,
    m: Option<&Manifest>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Status").build();
    if let Some(m) = m.filter(|m| !m.description.is_empty()) {
        group.set_description(Some(&escape(&m.description)));
    }
    let row = adw::ActionRow::new();
    let icon = gtk::Image::new();
    row.add_prefix(&icon);
    group.add(&row);
    let on = adw::SwitchRow::builder().title("Turned On").build();
    {
        let (p, d) = (p.clone(), dialog.downgrade());
        on.connect_active_notify(move |r| {
            if p.syncing.get() {
                return;
            }
            if let Some(d) = d.upgrade() {
                set_enabled(&p.state, &d, &p.id, r.is_active());
            }
        });
    }
    group.add(&on);
    let again = adw::ButtonRow::builder()
        .title("Restart")
        .start_icon_name("view-refresh-symbolic")
        .build();
    {
        let (p, d) = (p.clone(), dialog.downgrade());
        again.connect_activated(move |_| {
            if let Some(d) = d.upgrade() {
                restart(&p.state, &d, &p.id);
            }
        });
    }
    group.add(&again);
    *p.status.borrow_mut() = Some(StatusRows {
        row: row.downgrade(),
        icon: icon.downgrade(),
        on: on.downgrade(),
    });
    group
}

fn secret_row(
    p: &Rc<Page>,
    dialog: &adw::PreferencesDialog,
    f: &ulanzi_core::plugin::Field,
) -> (SecretRow, adw::PasswordEntryRow) {
    let label = f.title().to_string();
    let row = adw::PasswordEntryRow::builder()
        .title(escape(&label))
        .show_apply_button(true)
        .build();
    if !f.help.is_empty() {
        row.set_tooltip_text(Some(&f.help));
    }
    let clear = gtk::Button::builder()
        .label("Clear")
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    row.add_suffix(&clear);
    {
        let (p, d, key) = (p.clone(), dialog.downgrade(), f.key.clone());
        row.connect_apply(move |r| {
            let Some(d) = d.upgrade() else { return };
            let (id, key, value) = (p.id.clone(), key.clone(), r.text().to_string());
            let (p, r) = (p.clone(), r.downgrade());
            call(
                &p.state.clone(),
                &d,
                move |c| async move { c.plugin_set_secret(&id, &key, &value).await },
                move |()| {
                    if let Some(r) = r.upgrade() {
                        r.set_text("");
                    }
                    load_secret_keys(&p);
                },
            );
        });
    }
    {
        let (p, d, key) = (p.clone(), dialog.downgrade(), f.key.clone());
        clear.connect_clicked(move |_| {
            let Some(d) = d.upgrade() else { return };
            let (id, key, p2) = (p.id.clone(), key.clone(), p.clone());
            call(
                &p.state,
                &d,
                move |c| async move { c.plugin_clear_secret(&id, &key).await },
                move |()| load_secret_keys(&p2),
            );
        });
    }
    (
        SecretRow {
            key: f.key.clone(),
            label,
            row: row.downgrade(),
            clear: clear.downgrade(),
        },
        row,
    )
}

/// Mark the secret rows whose value is saved (`<label> · saved`, with a Clear button).
fn load_secret_keys(p: &Rc<Page>) {
    if p.secrets.borrow().is_empty() {
        return;
    }
    let Some(c) = p.state.client.borrow().clone() else {
        return;
    };
    let (id, p) = (p.id.clone(), p.clone());
    runtime::run(async move { c.plugin_secret_keys(&id).await }, move |r| match r {
        Ok(saved) => {
            for s in p.secrets.borrow().iter() {
                let (Some(row), Some(clear)) = (s.row.upgrade(), s.clear.upgrade()) else {
                    continue;
                };
                let is_saved = saved.contains(&s.key);
                let title = if is_saved {
                    format!("{} · saved", s.label)
                } else {
                    s.label.clone()
                };
                row.set_title(&escape(&title));
                clear.set_visible(is_saved);
            }
        }
        Err(e) => tracing::debug!("plugin secret keys: {e}"),
    });
}

fn settings_group(p: &Rc<Page>, dialog: &adw::PreferencesDialog, m: &Manifest) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Settings").build();
    let table = p
        .state
        .settings
        .borrow()
        .plugins
        .get(&p.id)
        .map(|e| e.settings.clone())
        .unwrap_or_default();
    let mut secrets = Vec::new();
    for f in &m.settings {
        if f.kind == FieldKind::Secret {
            let (s, row) = secret_row(p, dialog, f);
            group.add(&row);
            secrets.push(s);
            continue;
        }
        let (state, id, key) = (p.state.clone(), p.id.clone(), f.key.clone());
        let on: plugin_fields::OnValue = Rc::new(move |r| {
            // An error is shown on the row; nothing is written.
            let Ok(v) = r else { return };
            let (id, key) = (id.clone(), key.clone());
            preferences::change_boxed(
                &state,
                Box::new(move |s| {
                    let entry = s.plugins.entry(id).or_default();
                    match v {
                        Some(v) => {
                            entry.settings.insert(key, v);
                        }
                        None => {
                            entry.settings.remove(&key);
                        }
                    }
                }),
            );
        });
        group.add(&plugin_fields::row(
            f,
            current(f, &table).as_ref(),
            plugin_fields::Save::OnApply,
            on,
        ));
    }
    *p.secrets.borrow_mut() = secrets;
    group
}

fn remove_group(p: &Rc<Page>, dialog: &adw::PreferencesDialog) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let remove = adw::ButtonRow::builder()
        .title("Remove…")
        .start_icon_name("user-trash-symbolic")
        .build();
    // Added, not set: setting the classes would drop the row's own `button` style.
    remove.add_css_class("destructive-action");
    let (p, d) = (p.clone(), dialog.downgrade());
    remove.connect_activated(move |_| {
        if let Some(d) = d.upgrade() {
            confirm_remove(&p, &d);
        }
    });
    group.add(&remove);
    group
}

fn confirm_remove(p: &Rc<Page>, dialog: &adw::PreferencesDialog) {
    let alert = adw::AlertDialog::builder()
        .heading(format!("Remove {}?", p.name()))
        .body(format!(
            "Its keys keep their settings and show “Needs the “{}” plugin” until you install it \
             again. Its saved passwords and tokens are deleted.",
            p.id
        ))
        .default_response("cancel")
        .close_response("cancel")
        .build();
    alert.add_response("cancel", "Cancel");
    alert.add_response("remove", "Remove");
    alert.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    let (p, d) = (p.clone(), dialog.downgrade());
    alert.connect_response(Some("remove"), move |_, _| {
        let Some(d) = d.upgrade() else { return };
        let (id, p2, d2) = (p.id.clone(), p.clone(), d.downgrade());
        call(
            &p.state,
            &d,
            move |c| async move { c.plugin_remove(&id).await },
            move |()| {
                if !p2.gone.replace(true)
                    && let Some(d) = d2.upgrade()
                {
                    d.pop_subpage();
                }
            },
        );
    });
    alert.present(Some(dialog));
}

/// (Re)build the page's groups for manifest `m`.
fn build(p: &Rc<Page>, m: Option<&Manifest>) {
    let (Some(view), Some(dialog)) = (p.view.upgrade(), p.dialog.upgrade()) else {
        return;
    };
    let page = adw::PreferencesPage::new();
    page.add(&status_group(p, &dialog, m));
    p.secrets.borrow_mut().clear();
    if let Some(m) = m.filter(|m| !m.settings.is_empty()) {
        page.add(&settings_group(p, &dialog, m));
    }
    page.add(&remove_group(p, &dialog));
    view.set_content(Some(&page));
    *p.built.borrow_mut() = Some(m.cloned());
    load_secret_keys(p);
}

/// Follow the plugin list: rebuild when the manifest changed, show the status, and leave when the
/// plugin is gone.
fn update(p: &Rc<Page>) {
    if p.gone.get() {
        return;
    }
    let (loaded, row, manifest) = {
        let plugins = p.state.plugins.borrow();
        (
            plugins.loaded,
            plugins.row(&p.id).cloned(),
            plugins.manifests.get(&p.id).cloned(),
        )
    };
    if row.is_some() {
        p.seen.set(true);
    }
    // The plugin went away while its page is on screen: leave. A page opened for an id that is not
    // installed shows that instead (and never pops another page).
    if loaded && row.is_none() && p.seen.get() && p.shown.get() {
        p.gone.set(true);
        if let Some(d) = p.dialog.upgrade() {
            d.pop_subpage();
        }
        return;
    }
    if p.built.borrow().as_ref() != Some(&manifest) {
        build(p, manifest.as_ref());
    }
    if let Some(nav) = p.nav.upgrade() {
        nav.set_title(&p.name());
    }
    // Before the list arrives (or while the service is away) the last status may be stale.
    let live = row.filter(|_| loaded);
    if let Some(b) = p.banner.upgrade() {
        match live.as_ref().and_then(banner) {
            Some((text, button)) => {
                b.set_title(&text);
                b.set_button_label(button);
                b.set_revealed(true);
            }
            None => b.set_revealed(false),
        }
    }
    let status = p.status.borrow();
    let Some(s) = status.as_ref() else { return };
    let (Some(srow), Some(icon), Some(on)) = (s.row.upgrade(), s.icon.upgrade(), s.on.upgrade()) else {
        return;
    };
    let w = match &live {
        Some(r) => status_words(r),
        None if loaded => StatusWords {
            title: "Not installed".into(),
            subtitle: String::new(),
            icon: "dialog-information-symbolic",
            css: "dim-label",
        },
        None => StatusWords {
            title: "Connecting…".into(),
            subtitle: String::new(),
            icon: "content-loading-symbolic",
            css: "dim-label",
        },
    };
    srow.set_title(&escape(&w.title));
    srow.set_subtitle(&escape(&w.subtitle));
    icon.set_icon_name(Some(w.icon));
    for c in ["success", "warning", "dim-label"] {
        icon.remove_css_class(c);
    }
    icon.add_css_class(w.css);
    p.syncing.set(true);
    if let Some(r) = &live {
        on.set_active(r.status != "off");
    }
    on.set_sensitive(live.is_some());
    p.syncing.set(false);
}

/// The settings page of plugin `id`, to push with `dialog.push_subpage`.
pub fn subpage(state: &State, dialog: &adw::PreferencesDialog, id: &str) -> adw::NavigationPage {
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    let bar = adw::Banner::builder().use_markup(false).build();
    view.add_top_bar(&bar);
    let nav = adw::NavigationPage::builder()
        .title(id)
        .tag(format!("plugin-{id}"))
        .child(&view)
        .build();
    let p = Rc::new(Page {
        state: state.clone(),
        dialog: dialog.downgrade(),
        id: id.to_string(),
        nav: nav.downgrade(),
        view: view.downgrade(),
        banner: bar.downgrade(),
        built: RefCell::new(None),
        status: RefCell::new(None),
        secrets: RefCell::new(Vec::new()),
        syncing: Cell::new(false),
        gone: Cell::new(false),
        shown: Cell::new(false),
        seen: Cell::new(false),
    });
    {
        let (p, d) = (p.clone(), dialog.downgrade());
        bar.connect_button_clicked(move |_| {
            if let Some(d) = d.upgrade() {
                restart(&p.state, &d, &p.id);
            }
        });
    }
    {
        let p = p.clone();
        nav.connect_hidden(move |_| p.gone.set(true));
    }
    {
        let p = p.clone();
        nav.connect_shown(move |_| {
            p.shown.set(true);
            update(&p);
        });
    }
    update(&p);
    // Dropped (with the page) once the page is popped or its widgets are gone.
    state.on_while(move |c| {
        if c == Change::Plugins {
            update(&p);
        }
        !p.gone.get() && p.nav.upgrade().is_some()
    });
    nav
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulanzi_core::plugin::{Inspected, Manifest};

    #[test]
    fn list_rows_show_the_detail_after_the_status() {
        let row = |status: &str, detail: &str| PluginRow {
            id: "obs".into(),
            name: "OBS Studio".into(),
            status: status.into(),
            detail: detail.into(),
            ..PluginRow::default()
        };
        assert_eq!(status_line(&status_words(&row("running", ""))), "Running");
        assert_eq!(
            status_line(&status_words(&row("failed", "it crashed"))),
            "Stopped working · it crashed"
        );
        assert_eq!(status_line(&status_words(&row("failed", ""))), "Stopped working");
    }

    #[test]
    fn install_rows_show_warning_and_permissions() {
        let manifest =
            Manifest::parse(include_str!("../../../../plugins/obs/plugin.toml")).expect("obs manifest");
        let description = manifest.description.clone();
        let i = Inspected {
            manifest,
            installed_version: Some("0.9.0".into()),
        };
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            install_rows(&i),
            vec![
                s(
                    "Unverified plugin",
                    "Not from the Ulanzi Deck marketplace. Only install plugins you trust."
                ),
                ("OBS Studio 1.0.0".to_string(), description),
                s("Replaces version 0.9.0", ""),
                s("Connect to 127.0.0.1, localhost", ""),
            ]
        );
    }
}
