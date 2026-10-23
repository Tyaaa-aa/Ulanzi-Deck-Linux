//! First-run setup: welcome, connect, permissions, bring your setup, background service, done.
//! Shown once (a marker file in the state directory) and again from the menu.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ulanzi_core::edit::Slot;
use ulanzi_core::model::Model;

use crate::runtime;
use crate::state::{Change, State, Status};

/// The udev rules installed by "Grant Access…".
pub const RULES: &str = include_str!("../../../../packaging/udev/70-ulanzi-deck.rules");
const RULES_DEST: &str = "/etc/udev/rules.d/70-ulanzi-deck.rules";

/// One line of the permissions page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub title: &'static str,
    /// None while it can't be known (no deck plugged in).
    pub ok: Option<bool>,
    pub detail: &'static str,
}

/// What the permissions page shows for the daemon's device state and whether `/dev/uinput` opens.
pub fn permission_checks(device_state: Option<&str>, uinput_ok: bool) -> Vec<Check> {
    let deck = match device_state {
        Some("ready") | Some("busy") => Check {
            title: "Deck access",
            ok: Some(true),
            detail: "The service can talk to the deck",
        },
        Some("no_permission") => Check {
            title: "Deck access",
            ok: Some(false),
            detail: "The deck is plugged in but this user may not open it",
        },
        _ => Check {
            title: "Deck access",
            ok: None,
            detail: "Plug in the deck to check",
        },
    };
    let keys = if uinput_ok {
        Check {
            title: "Keyboard shortcuts and typing",
            ok: Some(true),
            detail: "Keys can send shortcuts and type text",
        }
    } else {
        Check {
            title: "Keyboard shortcuts and typing",
            ok: Some(false),
            detail: "Needs access to the virtual keyboard (/dev/uinput)",
        }
    };
    vec![deck, keys]
}

/// True when installing the rules would fix something.
pub fn needs_rules(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.ok == Some(false))
}

/// The command that installs the rules with administrator rights.
pub fn install_rules_cmd(exe: &Path) -> Vec<String> {
    vec![
        "pkexec".into(),
        exe.to_string_lossy().into_owned(),
        "--install-rules".into(),
    ]
}

/// Quote `s` for a POSIX shell when it needs it.
pub fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// The same fix, as commands to paste into a terminal.
pub fn manual_commands(exe: &Path) -> String {
    format!("sudo {} --install-rules", shell_quote(&exe.to_string_lossy()))
}

/// `ulanzi-deck --install-rules`, run as root through pkexec: copy the bundled rules and reload udev.
/// Takes no input besides the fixed file, so it is safe to run with administrator rights.
pub fn install_rules() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = format!("{RULES_DEST}.tmp");
    std::fs::write(&tmp, RULES).map_err(|e| format!("can't write {tmp}: {e}"))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, RULES_DEST).map_err(|e| e.to_string())?;
    let run = |args: &[&str]| {
        std::process::Command::new("udevadm")
            .args(args)
            .status()
            .map_err(|e| format!("udevadm: {e}"))
            .and_then(|s| {
                s.success()
                    .then_some(())
                    .ok_or_else(|| format!("udevadm {} failed", args.join(" ")))
            })
    };
    // uinput may be a module that is not loaded yet.
    let _ = std::process::Command::new("modprobe").arg("uinput").status();
    run(&["control", "--reload-rules"])?;
    run(&["trigger", "--subsystem-match=hidraw"])?;
    run(&["trigger", "--subsystem-match=misc", "--sysname-match=uinput"])?;
    Ok(())
}

fn uinput_ok() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}

fn marker() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("ulanzi-deck").join("onboarded"))
}

/// True until the user has finished or dismissed onboarding once.
pub fn should_show() -> bool {
    marker().is_some_and(|m| !m.exists())
}

fn mark_done() {
    if let Some(m) = marker() {
        let _ = m.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(m, b"1\n");
    }
}

fn device_state(state: &State) -> Option<String> {
    match &*state.status.borrow() {
        Status::Device { state, .. } => Some(state.clone()),
        _ => None,
    }
}

/// Step dots under each page.
fn dots(current: usize) -> gtk::Box {
    let b = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_bottom(18)
        .margin_top(6)
        .build();
    for i in 0..STEPS {
        let d = gtk::Box::builder().css_classes(["page-dot"]).build();
        if i == current {
            d.add_css_class("current");
        }
        d.set_valign(gtk::Align::Center);
        b.append(&d);
    }
    b
}

const STEPS: usize = 6;

/// A page: header, scrollable content, step dots and a bottom button row.
fn page(
    tag: &str,
    title: &str,
    step: usize,
    body: &impl IsA<gtk::Widget>,
    next: Option<&gtk::Button>,
) -> adw::NavigationPage {
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(440)
                .margin_start(18)
                .margin_end(18)
                .child(body)
                .build(),
        )
        .build();
    view.set_content(Some(&scroller));
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if let Some(n) = next {
        n.set_halign(gtk::Align::Center);
        n.add_css_class("pill");
        n.add_css_class("suggested-action");
        bottom.append(n);
    }
    bottom.append(&dots(step));
    view.add_bottom_bar(&bottom);
    adw::NavigationPage::builder()
        .tag(tag)
        .title(title)
        .child(&view)
        .build()
}

fn status_body(icon: &str, title: &str, description: &str) -> adw::StatusPage {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        .description(description)
        .css_classes(["compact"])
        .build()
}

/// A header that leaves room for the page's content (no scrolling on small dialogs).
fn small_header(icon: &str, title: &str, description: &str) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .build();
    b.append(
        &gtk::Image::builder()
            .icon_name(icon)
            .pixel_size(56)
            .css_classes(["dim-label"])
            .build(),
    );
    b.append(
        &gtk::Label::builder()
            .label(title)
            .css_classes(["title-1"])
            .build(),
    );
    b.append(
        &gtk::Label::builder()
            .label(description)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );
    b
}

fn check_row(c: &Check) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(c.title)
        .subtitle(c.detail)
        .build();
    let (icon, class) = match c.ok {
        Some(true) => ("emblem-ok-symbolic", "success"),
        Some(false) => ("dialog-warning-symbolic", "warning"),
        None => ("content-loading-symbolic", "dim-label"),
    };
    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class(class);
    row.add_prefix(&img);
    row
}

/// Open onboarding over `parent`, optionally at a page tag (screenshots).
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>, start: Option<&str>) {
    let dialog = adw::Dialog::builder()
        .title("Set Up Ulanzi Deck")
        .content_width(520)
        .content_height(640)
        .build();
    let nav = adw::NavigationView::new();
    dialog.set_child(Some(&nav));
    dialog.connect_closed(|_| mark_done());

    let go = |nav: &adw::NavigationView, tag: &'static str| {
        let nav = nav.clone();
        move |_: &gtk::Button| nav.push_by_tag(tag)
    };

    // 1. Welcome
    let next = gtk::Button::with_label("Get Started");
    next.connect_clicked(go(&nav, "connect"));
    let welcome = status_body(
        "io.github.tyaaa_aa.UlanziDeck",
        "Welcome to Ulanzi Deck",
        "Set up your Ulanzi D200/D200X in a minute: connect it, allow access, and bring over your old setup.",
    );
    nav.add(&page("welcome", "Welcome", 0, &welcome, Some(&next)));

    // 2. Connect
    nav.add(&connect_page(state, &nav));

    // 3. Permissions
    nav.add(&permissions_page(state, &nav));

    // 4. Bring your setup
    nav.add(&import_page(state, &nav));

    // 5. Background service
    nav.add(&background_page(state, &nav));

    // 6. Done
    let finish = gtk::Button::with_label("Start Using Ulanzi Deck");
    {
        let dialog = dialog.clone();
        finish.connect_clicked(move |_| {
            mark_done();
            dialog.close();
        });
    }
    let done = gtk::Box::new(gtk::Orientation::Vertical, 14);
    done.set_margin_top(18);
    done.append(
        &gtk::Label::builder()
            .label("All Set")
            .css_classes(["title-1"])
            .build(),
    );
    done.append(&crate::ui::how_it_works::build());
    done.append(
        &gtk::Label::builder()
            .label("Changes are saved and sent to the deck as you go. Press Ctrl+Z to undo anything.")
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label"])
            .build(),
    );
    nav.add(&page("done", "Done", 5, &done, Some(&finish)));

    // The welcome page is where it starts anyway.
    if let Some(tag) = start.filter(|t| *t != "welcome") {
        // Build the history so Back works from a deep link.
        for t in ["connect", "permissions", "import", "background", "done"] {
            nav.push_by_tag(t);
            if t == tag {
                break;
            }
        }
    }
    dialog.present(Some(parent));
}

fn connect_page(state: &State, nav: &adw::NavigationView) -> adw::NavigationPage {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let status = status_body("input-keyboard-symbolic", "", "");
    let spinner = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .build();
    body.append(&status);
    body.append(&spinner);

    // Dial base question (the deck reports "D200" either way).
    let model_group = adw::PreferencesGroup::builder()
        .title("Does your deck have the dial base?")
        .description("The base adds three dials and two small buttons")
        .build();
    let with = adw::ActionRow::builder()
        .title("Yes, with dials")
        .subtitle("D200X")
        .build();
    let without = adw::ActionRow::builder()
        .title("No, keys only")
        .subtitle("D200")
        .build();
    let r_with = gtk::CheckButton::new();
    let r_without = gtk::CheckButton::new();
    r_without.set_group(Some(&r_with));
    with.add_prefix(&r_with);
    with.set_activatable_widget(Some(&r_with));
    without.add_prefix(&r_without);
    without.set_activatable_widget(Some(&r_without));
    model_group.add(&with);
    model_group.add(&without);
    match state.model() {
        Model::D200X => r_with.set_active(true),
        Model::D200 => r_without.set_active(true),
    }
    {
        let state = state.clone();
        r_with.connect_toggled(move |b| {
            let m = if b.is_active() { Model::D200X } else { Model::D200 };
            if state.model() != m {
                state.edit(|e| e.set_model(m));
            }
        });
    }
    body.append(&model_group);

    let next = gtk::Button::with_label("Continue");
    {
        let nav = nav.clone();
        next.connect_clicked(move |_| nav.push_by_tag("permissions"));
    }
    let start_service = gtk::Button::builder()
        .label("Start the Service")
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    start_service.connect_clicked(|b| {
        b.set_sensitive(false);
        runtime::run(
            async {
                tokio::process::Command::new("systemctl")
                    .args(["--user", "start", "ulanzi-deckd"])
                    .status()
                    .await
            },
            |_| {},
        );
    });
    body.append(&start_service);

    let update = {
        let (state, status, spinner, model_group, start_service, next) = (
            state.clone(),
            status.downgrade(),
            spinner.downgrade(),
            model_group.downgrade(),
            start_service.downgrade(),
            next.downgrade(),
        );
        move || {
            let (Some(status), Some(spinner), Some(model_group), Some(start), Some(next)) = (
                status.upgrade(),
                spinner.upgrade(),
                model_group.upgrade(),
                start_service.upgrade(),
                next.upgrade(),
            ) else {
                return;
            };
            let s = state.status.borrow().clone();
            let (icon, title, desc, waiting, found) = match &s {
                Status::Connecting => (
                    "network-wired-symbolic",
                    "Connecting…".to_string(),
                    String::new(),
                    true,
                    false,
                ),
                Status::ServiceDown => (
                    "dialog-warning-symbolic",
                    "The Service Isn’t Running".into(),
                    "Ulanzi Deck runs a small background service that talks to the deck.".into(),
                    false,
                    false,
                ),
                Status::Device { state: st, model, .. } => match st.as_str() {
                    "ready" => (
                        "emblem-ok-symbolic",
                        format!("{} Connected", if model.is_empty() { "Deck" } else { model }),
                        "Your deck is ready.".into(),
                        false,
                        true,
                    ),
                    "no_permission" => (
                        "dialog-password-symbolic",
                        "Deck Found".into(),
                        "It needs permission first. That’s the next step.".into(),
                        false,
                        true,
                    ),
                    "busy" => (
                        "dialog-warning-symbolic",
                        "Deck In Use".into(),
                        "Another program has the deck open. Close it (for example the old Ulanzi app) \
                         and it connects here."
                            .into(),
                        true,
                        true,
                    ),
                    _ => (
                        "input-keyboard-symbolic",
                        "Plug In Your Deck".into(),
                        "Connect the Ulanzi D200 with its USB cable. It shows up here right away.".into(),
                        true,
                        false,
                    ),
                },
            };
            status.set_icon_name(Some(icon));
            status.set_title(&title);
            status.set_description(Some(&desc));
            spinner.set_visible(waiting);
            model_group.set_visible(found);
            start.set_visible(matches!(s, Status::ServiceDown));
            if matches!(s, Status::ServiceDown) {
                start.set_sensitive(true);
            }
            next.set_label(if found { "Continue" } else { "Skip for Now" });
        }
    };
    update();
    {
        let dial_seen = std::cell::Cell::new(false);
        let model_group = model_group.downgrade();
        let st = state.clone();
        state.on(move |c| match c {
            Change::Status => update(),
            // A turned dial answers the question.
            Change::Pressed(Slot::Dial(_)) if !dial_seen.get() => {
                dial_seen.set(true);
                if st.model() != Model::D200X {
                    st.edit(|e| e.set_model(Model::D200X));
                }
                if let Some(g) = model_group.upgrade() {
                    g.set_description(Some("Dial detected: set to D200X"));
                }
            }
            _ => {}
        });
    }
    page("connect", "Connect", 1, &body, Some(&next))
}

fn permissions_page(state: &State, nav: &adw::NavigationView) -> adw::NavigationPage {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.append(&small_header(
        "dialog-password-symbolic",
        "Allow Access",
        "Linux only lets programs use the deck and the virtual keyboard with permission. \
         This is a one-time step for this computer.",
    ));
    let group = adw::PreferencesGroup::new();
    body.append(&group);
    let grant = gtk::Button::builder()
        .label("Grant Access…")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    body.append(&grant);
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ulanzi-deck"));
    // The same fix for people who prefer a terminal, in plain words.
    let manual_group = adw::PreferencesGroup::builder()
        .title("Prefer the Terminal?")
        .description(
            "Only needed if Grant Access doesn’t work for you. Run this one command; it asks for \
             your password once.",
        )
        .build();
    let text = manual_commands(&exe);
    let command = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&text).as_str())
        .title_selectable(true)
        .css_classes(["monospace", "property"])
        .build();
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy the command")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let text = text.clone();
        copy.connect_clicked(move |b| {
            b.clipboard().set_text(&text);
            b.set_icon_name("object-select-symbolic");
        });
    }
    command.add_suffix(&copy);
    manual_group.add(&command);
    let what = adw::ActionRow::builder()
        .title("What it does")
        .subtitle(
            "Copies Ulanzi Deck’s device rule to /etc/udev/rules.d/ and reloads it, so your user \
             can open the deck and the virtual keyboard. Nothing else on the system changes.",
        )
        .subtitle_lines(0)
        .activatable(false)
        .build();
    manual_group.add(&what);
    body.append(&manual_group);

    let rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let (state, group, grant) = (state.clone(), group.downgrade(), grant.downgrade());
        let rows = rows.clone();
        Rc::new(move || {
            let (Some(group), Some(grant)) = (group.upgrade(), grant.upgrade()) else {
                return;
            };
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let checks = permission_checks(device_state(&state).as_deref(), uinput_ok());
            for c in &checks {
                let r = check_row(c);
                group.add(&r);
                rows.borrow_mut().push(r);
            }
            let needed = needs_rules(&checks);
            grant.set_visible(needed);
        })
    };
    refresh();
    {
        let refresh = refresh.clone();
        grant.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Waiting for Approval…");
            let argv = install_rules_cmd(&exe);
            let (b, refresh) = (b.clone(), refresh.clone());
            runtime::run(
                async move {
                    tokio::process::Command::new(&argv[0])
                        .args(&argv[1..])
                        .status()
                        .await
                },
                move |r| {
                    b.set_sensitive(true);
                    b.set_label("Grant Access…");
                    match r {
                        Ok(s) if s.success() => {
                            // udev applies the new rules a moment later.
                            let refresh = refresh.clone();
                            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                                refresh()
                            });
                        }
                        Ok(_) => b.set_label("Not Granted · Try Again"),
                        Err(_) => b.set_label("pkexec Is Missing · Use the Commands"),
                    }
                },
            );
        });
    }
    {
        let refresh = refresh.clone();
        state.on(move |c| {
            if c == Change::Status {
                refresh();
            }
        });
    }
    let next = gtk::Button::with_label("Continue");
    {
        let nav = nav.clone();
        next.connect_clicked(move |_| nav.push_by_tag("import"));
    }
    let page = page("permissions", "Permissions", 2, &body, Some(&next));
    page.connect_showing(move |_| refresh());
    page
}

/// A dry run of the old app's config in a scratch store: (keys, warnings).
fn preview_legacy(yaml: &Path) -> Result<(usize, Vec<String>), String> {
    let dir = std::env::temp_dir().join(format!("ulanzi-deck-preview-{}", std::process::id()));
    let store = ulanzi_core::store::Store::at(dir.join("config"), dir.join("state"), dir.join("cache"));
    let r = ulanzi_core::migrate::migrate_legacy(yaml, &store, true).map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&dir);
    r.map(|r| (r.keys, r.warnings))
}

fn import_page(state: &State, nav: &adw::NavigationView) -> adw::NavigationPage {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let legacy = ulanzi_core::migrate::legacy_config_path().filter(|p| p.is_file());
    let imported = ulanzi_core::store::Store::xdg()
        .map(|s| ulanzi_core::migrate::already_migrated(&s))
        .unwrap_or(false);
    let next = gtk::Button::with_label("Continue");
    let group = adw::PreferencesGroup::new();
    let offer_import = legacy.is_some() && !imported;
    if !offer_import {
        let nav = nav.clone();
        next.connect_clicked(move |_| nav.push_by_tag("background"));
    }
    match (&legacy, imported) {
        (Some(_), true) => {
            body.append(&status_body(
                "emblem-ok-symbolic",
                "Your Setup Is Here",
                "Your keys from the previous Ulanzi app were imported into the profile “Default”.",
            ));
        }
        (Some(path), false) => {
            let (title, desc) = match preview_legacy(path) {
                Ok((keys, warnings)) => {
                    let w = adw::ExpanderRow::builder()
                        .title(format!(
                            "{} thing{} to check",
                            warnings.len(),
                            if warnings.len() == 1 { "" } else { "s" }
                        ))
                        .subtitle("Parts that could not be brought over exactly")
                        .sensitive(!warnings.is_empty())
                        .build();
                    for line in warnings {
                        w.add_row(
                            &adw::ActionRow::builder()
                                .title(glib::markup_escape_text(&line).as_str())
                                .title_lines(3)
                                .build(),
                        );
                    }
                    group.add(&w);
                    (
                        "Bring Your Setup".to_string(),
                        format!(
                            "Found the previous Ulanzi app’s setup with {keys} key{}.",
                            if keys == 1 { "" } else { "s" }
                        ),
                    )
                }
                Err(e) => (
                    "Bring Your Setup".into(),
                    format!("The old setup can’t be read: {e}"),
                ),
            };
            body.append(&status_body("document-import-symbolic", &title, &desc));
            next.set_label("Import");
            let state = state.clone();
            let nav2 = nav.clone();
            next.connect_clicked(move |b| {
                b.set_sensitive(false);
                if let Some(c) = state.client.borrow().clone() {
                    let (state, nav) = (state.clone(), nav2.clone());
                    runtime::run(async move { c.migrate(false).await }, move |r| match r {
                        Ok((id, keys, _)) => {
                            crate::ui::window::open_profile(&state, id, true);
                            crate::ui::toast(&state, &format!("Imported {keys} keys"));
                            nav.push_by_tag("background");
                        }
                        Err(e) => crate::ui::toast(&state, &format!("Import failed: {e}")),
                    });
                }
            });
        }
        (None, _) => {
            body.append(&status_body(
                "document-new-symbolic",
                "Start Fresh",
                "You start with an empty profile. Drag actions onto keys to build your layout.",
            ));
        }
    }
    let bundle = adw::ActionRow::builder()
        .title("Open a .ulanzideck File")
        .subtitle("Shared profiles. Coming in a later update")
        .sensitive(false)
        .build();
    group.add(&bundle);
    body.append(&group);
    if offer_import {
        let fresh = gtk::Button::builder()
            .label("Start Fresh Instead")
            .halign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let nav = nav.clone();
        fresh.connect_clicked(move |_| nav.push_by_tag("background"));
        body.append(&fresh);
    }
    page("import", "Your Setup", 3, &body, Some(&next))
}

fn background_page(state: &State, nav: &adw::NavigationView) -> adw::NavigationPage {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    body.append(&status_body(
        "preferences-system-symbolic",
        "Runs in the Background",
        "The deck keeps working with this window closed. A small service handles the keys.",
    ));
    let group = adw::PreferencesGroup::new();
    let login = adw::SwitchRow::builder()
        .title("Start at Login")
        .subtitle("The deck works as soon as you sign in")
        .build();
    let tray = adw::SwitchRow::builder()
        .title("Tray Icon")
        .subtitle("Quick access to profiles and brightness")
        .active(state.settings.borrow().tray)
        .build();
    group.add(&login);
    group.add(&tray);
    body.append(&group);

    // Start at login mirrors the systemd user unit.
    {
        let login = login.downgrade();
        runtime::run(
            async {
                tokio::process::Command::new("systemctl")
                    .args(["--user", "is-enabled", "ulanzi-deckd"])
                    .output()
                    .await
            },
            move |r| {
                if let (Some(login), Ok(o)) = (login.upgrade(), r) {
                    login.set_active(String::from_utf8_lossy(&o.stdout).trim() == "enabled");
                    login.connect_active_notify(|row| {
                        let verb = if row.is_active() { "enable" } else { "disable" };
                        runtime::run(
                            async move {
                                tokio::process::Command::new("systemctl")
                                    .args(["--user", verb, "ulanzi-deckd"])
                                    .status()
                                    .await
                            },
                            |_| {},
                        );
                    });
                }
            },
        );
    }
    {
        let state = state.clone();
        tray.connect_active_notify(move |row| {
            let mut s = state.settings.borrow().clone();
            s.tray = row.is_active();
            *state.settings.borrow_mut() = s.clone();
            if let Some(c) = state.client.borrow().clone() {
                runtime::run(async move { c.apply_settings(&s).await }, |_| {});
            }
        });
    }
    let next = gtk::Button::with_label("Continue");
    {
        let nav = nav.clone();
        next.connect_clicked(move |_| nav.push_by_tag("done"));
    }
    page("background", "Background", 4, &body, Some(&next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_follow_device_state_and_uinput() {
        let c = permission_checks(Some("ready"), true);
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|c| c.ok == Some(true)));

        let c = permission_checks(Some("no_permission"), false);
        assert_eq!(c[0].ok, Some(false));
        assert_eq!(c[1].ok, Some(false));
        assert!(needs_rules(&c));

        // Without a deck plugged in, access to it can't be known yet.
        let c = permission_checks(Some("not_found"), true);
        assert_eq!(c[0].ok, None);
        assert!(!needs_rules(&c));
        let c = permission_checks(None, true);
        assert_eq!(c[0].ok, None);
    }

    #[test]
    fn rules_command_is_argv_and_manual_text_is_quoted() {
        let exe = std::path::Path::new("/opt/My Apps/ulanzi-deck");
        assert_eq!(
            install_rules_cmd(exe),
            ["pkexec", "/opt/My Apps/ulanzi-deck", "--install-rules"]
        );
        assert_eq!(shell_quote("/usr/bin/ulanzi-deck"), "/usr/bin/ulanzi-deck");
        assert_eq!(shell_quote("/opt/My Apps/x"), "'/opt/My Apps/x'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let text = manual_commands(exe);
        assert!(
            text.contains("sudo '/opt/My Apps/ulanzi-deck' --install-rules"),
            "{text}"
        );
    }
}
