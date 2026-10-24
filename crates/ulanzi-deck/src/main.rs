//! `ulanzi-deck`: the Ulanzi Deck app.

use std::path::PathBuf;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use ulanzi_core::model::{Model, Profile};
use ulanzi_deck::state::AppState;
use ulanzi_deck::ui;

use ulanzi_deck::APP_ID;

thread_local! {
    /// The window's state, for files opened while the app runs.
    static APP: std::cell::RefCell<Option<ulanzi_deck::state::State>> = const { std::cell::RefCell::new(None) };
}

/// `--screenshot` options: render the window to a PNG and exit (used to check the UI without a person
/// at the screen).
#[derive(Debug, Clone, Default)]
struct Shot {
    path: PathBuf,
    size: (i32, i32),
    dark: Option<bool>,
    select: Option<String>,
    /// `library`, `picker:<tab>`, `integrations`, `integrations:<plugin id>`, `action-picker`, ….
    open: Option<String>,
    /// 1-based page to show.
    page: Option<usize>,
    /// Milliseconds between reaching the service and taking the shot (`--wait`, default 1200).
    wait: u64,
}

/// The app's own icons when they are not in the system theme: next to the binary (tarball,
/// AppImage) or in the source tree (a development build).
fn add_icon_paths() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(prefix) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent()?.parent().map(|p| p.to_path_buf()))
    {
        dirs.push(prefix.join("share/icons/hicolor/scalable/apps"));
        dirs.push(prefix.join("share/icons/hicolor/symbolic/apps"));
    }
    dirs.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons")));
    for d in dirs.into_iter().filter(|d| d.is_dir()) {
        theme.add_search_path(&d);
    }
}

fn parse_size(s: &str) -> Option<(i32, i32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn parse_args() -> Option<Shot> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let path = get("--screenshot")?;
    Some(Shot {
        path: PathBuf::from(path),
        size: get("--size").and_then(|s| parse_size(&s)).unwrap_or((1280, 800)),
        dark: get("--theme").map(|t| t == "dark"),
        select: get("--select"),
        open: get("--open"),
        page: get("--page").and_then(|p| p.parse().ok()),
        wait: get("--wait").and_then(|w| w.parse().ok()).unwrap_or(1200),
    })
}

/// Depth-first search for a widget (screenshot mode only).
fn find_widget(root: &gtk::Widget, pred: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if pred(root) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(found) = find_widget(&c, pred) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// Popovers that are showing, anywhere under `root` (screenshot mode only).
fn collect_open_popovers(root: &gtk::Widget, out: &mut Vec<gtk::Popover>) {
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(p) = c.downcast_ref::<gtk::Popover>()
            && p.is_visible()
        {
            out.push(p.clone());
        }
        collect_open_popovers(&c, out);
        child = c.next_sibling();
    }
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn save_screenshot(window: &adw::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let (w, h) = (window.width() as f64, window.height() as f64);
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w, h);
    // Open popovers are surfaces of their own: draw them on top where the compositor put them.
    let mut popovers = Vec::new();
    collect_open_popovers(window.upcast_ref(), &mut popovers);
    let (wx, wy) = window.surface_transform();
    for p in popovers {
        let (Some(popup), (px, py)) = (p.surface().and_downcast::<gdk::Popup>(), p.surface_transform())
        else {
            continue;
        };
        snapshot.save();
        snapshot.translate(&gtk::graphene::Point::new(
            (popup.position_x() as f64 + px - wx) as f32,
            (popup.position_y() as f64 + py - wy) as f32,
        ));
        gtk::WidgetPaintable::new(Some(&p)).snapshot(&snapshot, p.width() as f64, p.height() as f64);
        snapshot.restore();
    }
    let node = snapshot.to_node().ok_or("nothing drawn")?;
    let renderer = window.renderer().ok_or("no renderer")?;
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).map_err(|e| e.to_string())
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ULANZI_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .init();
    // The privileged helper behind onboarding's "Grant Access…" (run through pkexec).
    if std::env::args().nth(1).as_deref() == Some("--install-rules") {
        return match ui::onboarding::install_rules() {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    let shot = parse_args();
    let flags = if shot.is_some() {
        gio::ApplicationFlags::NON_UNIQUE
    } else {
        // Opening a .ulanzideck file (from a file manager) starts an import.
        gio::ApplicationFlags::HANDLES_OPEN
    };
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    app.connect_startup(|_| {
        add_icon_paths();
        gtk::Window::set_default_icon_name(APP_ID);
        load_css();
    });
    app.connect_open(|app, files, _| {
        app.activate();
        let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
        // Wait until the window has reached the service, then show the import.
        let tries = std::rc::Rc::new(std::cell::Cell::new(0));
        glib::timeout_add_local(Duration::from_millis(250), move || {
            tries.set(tries.get() + 1);
            let ready = APP
                .with(|a| a.borrow().clone())
                .filter(|s| s.client.borrow().is_some());
            match ready {
                Some(state) => {
                    if let Some(root) = state.toast_root() {
                        for p in &paths {
                            ui::bundle_dialogs::open_import(&state, &root, p);
                        }
                    }
                    glib::ControlFlow::Break
                }
                None if tries.get() > 40 => glib::ControlFlow::Break,
                None => glib::ControlFlow::Continue,
            }
        });
    });
    app.connect_activate(move |app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let state = AppState::new(Profile::new_default("Default", Model::D200X));
        APP.with(|a| *a.borrow_mut() = Some(state.clone()));
        let win = ui::window::build(app, &state);
        {
            let import = gio::SimpleAction::new("import", None);
            let (st, window) = (state.clone(), win.window.clone());
            import.connect_activate(move |_, _| ui::bundle_dialogs::choose_and_import(&st, &window));
            app.add_action(&import);
            let export = gio::SimpleAction::new("export", None);
            let (st, window) = (state.clone(), win.window.clone());
            export.connect_activate(move |_, _| ui::bundle_dialogs::open_export(&st, &window, None));
            app.add_action(&export);
            app.set_accels_for_action("app.import", &["<Ctrl>o"]);
            app.set_accels_for_action("app.export", &["<Ctrl>e"]);
        }
        if let Some(shot) = shot.clone() {
            if let Some(dark) = shot.dark {
                adw::StyleManager::default().set_color_scheme(if dark {
                    adw::ColorScheme::ForceDark
                } else {
                    adw::ColorScheme::ForceLight
                });
            }
            // A fixed-size window: the compositor would otherwise restore the size it remembers.
            win.window.set_default_size(shot.size.0, shot.size.1);
            win.window.set_size_request(shot.size.0, shot.size.1);
        }
        win.window.present();
        let window = win.window.clone();
        let library_toggle = win.library_toggle.clone();
        let app = app.clone();
        let state2 = state.clone();
        let shot = shot.clone();
        {
            let onboarding = gio::SimpleAction::new("onboarding", None);
            let (state, window) = (state.clone(), window.clone());
            onboarding.connect_activate(move |_, _| ui::onboarding::open(&state, &window, None));
            app.add_action(&onboarding);
        }
        {
            let prefs = gio::SimpleAction::new("preferences", None);
            let (state, window) = (state.clone(), window.clone());
            prefs.connect_activate(move |_, _| ui::preferences::open(&state, &window));
            app.add_action(&prefs);
            app.set_accels_for_action("app.preferences", &["<Ctrl>comma"]);
        }
        if shot.is_none() {
            ui::preferences::follow_appearance(&state);
        }
        {
            let about = gio::SimpleAction::new("about", None);
            let window = window.clone();
            about.connect_activate(move |_, _| {
                let about = adw::AboutDialog::builder()
                    .application_name("Ulanzi Deck")
                    .application_icon(APP_ID)
                    .version(env!("CARGO_PKG_VERSION"))
                    .developer_name("Tyaaa-aa")
                    .license_type(gtk::License::MitX11)
                    .website("https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux")
                    .issue_url("https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues")
                    .comments("Configure the Ulanzi D200 and D200X stream decks on Linux.")
                    .build();
                about.add_acknowledgement_section(
                    Some("Protocol Notes"),
                    &["homedeck https://github.com/redphx/homedeck"],
                );
                about.add_legal_section(
                    "Lucide icons",
                    Some("© Lucide Contributors"),
                    gtk::License::Custom,
                    Some("ISC License"),
                );
                about.add_legal_section(
                    "Simple Icons",
                    Some("Simple Icons contributors"),
                    gtk::License::Custom,
                    Some(
                        "CC0 1.0. Brand logos are trademarks of their owners and are shown only to \
                         label your own keys; their use does not imply endorsement.",
                    ),
                );
                about.present(Some(&window));
            });
            app.add_action(&about);
        }
        ui::window::connect(&state, move || {
            let Some(shot) = shot else {
                if ui::onboarding::should_show() {
                    ui::onboarding::open(&state2, &window, None);
                }
                return;
            };
            if let Some(p) = shot.page {
                state2.set_page(p.saturating_sub(1));
            }
            if let Some(sel) = &shot.select {
                state2.select(|s| match sel.split_once(':') {
                    Some(("key", n)) => n
                        .split(',')
                        .filter_map(|k| k.parse().ok())
                        .for_each(|k| s.toggle(ulanzi_core::edit::Slot::Key(k))),
                    Some(("dial", n)) => s.click(ulanzi_core::edit::Slot::Dial(n.parse().unwrap_or(0))),
                    Some(("small", n)) => s.click(ulanzi_core::edit::Slot::Small(n.parse().unwrap_or(0))),
                    _ if sel == "screen" => s.click(ulanzi_core::edit::Slot::Screen),
                    _ => {}
                });
            }
            match shot.open.as_deref() {
                Some("library") => library_toggle.set_active(true),
                Some(o) if o.starts_with("onboarding") => {
                    ui::onboarding::open(&state2, &window, o.split_once(':').map(|(_, t)| t))
                }
                Some("preferences") => ui::preferences::open(&state2, &window),
                Some(o) if o.starts_with("integrations:") => {
                    ui::preferences::open(&state2, &window);
                    if let Some(d) = window.visible_dialog().and_downcast::<adw::PreferencesDialog>() {
                        d.set_visible_page_name("integrations");
                        let id = &o["integrations:".len()..];
                        d.push_subpage(&ui::integrations::subpage(&state2, &d, id));
                    }
                }
                Some("integrations") => {
                    ui::preferences::open(&state2, &window);
                    if let Some(d) = window.visible_dialog().and_downcast::<adw::PreferencesDialog>() {
                        d.set_visible_page_name("integrations");
                    }
                }
                Some("obs-setup") => ui::obs_setup::open_dialog(&state2, &window),
                Some("backups") => {
                    ui::preferences::open(&state2, &window);
                    if let Some(d) = window.visible_dialog().and_downcast::<adw::PreferencesDialog>() {
                        d.set_visible_page_name("backups");
                    }
                }
                Some("apps") => ui::app_picker::preview_dialog(&window),
                Some("templates") => ui::templates::open(&state2, &window),
                Some("export") => ui::bundle_dialogs::open_export(&state2, &window, None),
                Some(o) if o.starts_with("import:") => {
                    let path = PathBuf::from(&o["import:".len()..]);
                    ui::bundle_dialogs::open_import(&state2, &window, &path)
                }
                Some("autoswitch") => {
                    let name = state2.editor.borrow().profile().name.clone();
                    ui::autoswitch::open(&state2, &window, state2.profile_id(), &name)
                }
                Some("action-picker") => {
                    // Open the first Choose/Change picker shortly before the shot: its plugin
                    // sections are built when it opens, and the inspector (with the popover) is
                    // rebuilt while the plugins are still starting.
                    let (window, state) = (window.clone(), state2.clone());
                    let at = shot.wait.saturating_sub(500).max(300);
                    glib::timeout_add_local_once(Duration::from_millis(at), move || {
                        if let Some(b) = find_widget(window.upcast_ref(), &|w| {
                            w.downcast_ref::<gtk::MenuButton>()
                                .and_then(|b| b.label())
                                .is_some_and(|l| l == "Choose" || l == "Change")
                        }) && let Ok(b) = b.downcast::<gtk::MenuButton>()
                        {
                            b.popup();
                            // The plugin sections are last: scroll to the first one's heading.
                            let first = state
                                .plugins
                                .borrow()
                                .picker_groups()
                                .first()
                                .map(|g| g.name.clone());
                            glib::timeout_add_local_once(Duration::from_millis(200), move || {
                                let Some(pop) = b.popover() else { return };
                                let find =
                                    |pred: &dyn Fn(&gtk::Widget) -> bool| find_widget(pop.upcast_ref(), pred);
                                let (Some(sw), Some(list)) = (
                                    find(&|w| w.is::<gtk::ScrolledWindow>())
                                        .and_downcast::<gtk::ScrolledWindow>(),
                                    find(&|w| w.is::<gtk::ListBox>()),
                                ) else {
                                    return;
                                };
                                let heading = first.and_then(|name| {
                                    find(&|w| {
                                        w.downcast_ref::<gtk::Label>().is_some_and(|l| l.label() == name)
                                    })
                                });
                                let adj = sw.vadjustment();
                                let y = heading
                                    .and_then(|h| {
                                        h.compute_point(&list, &gtk::graphene::Point::new(0.0, 0.0))
                                    })
                                    .map(|p| p.y() as f64 - 8.0)
                                    .unwrap_or(adj.upper());
                                adj.set_value(y.min(adj.upper() - adj.page_size()));
                            });
                        }
                    });
                }
                Some(o) if o.starts_with("color") => {
                    // Open the first colour button's popover (after the inspector is built).
                    let (window, custom) = (window.clone(), o.ends_with(":custom"));
                    glib::timeout_add_local_once(Duration::from_millis(300), move || {
                        if let Some(b) = find_widget(window.upcast_ref(), &|w| {
                            w.tooltip_text().as_deref() == Some("Choose a colour")
                        }) && let Ok(b) = b.downcast::<gtk::MenuButton>()
                        {
                            b.popup();
                            if custom
                                && let Some(pop) = b.popover()
                                && let Some(t) =
                                    find_widget(pop.upcast_ref(), &|w| w.is::<gtk::ToggleButton>())
                                && let Ok(t) = t.downcast::<gtk::ToggleButton>()
                            {
                                t.set_active(true);
                            }
                        }
                    });
                }
                Some("about") => {
                    gio::prelude::ActionGroupExt::activate_action(&app, "about", None);
                }
                Some(o) if o.starts_with("picker") => {
                    ui::icon_picker::open_at(&state2, &window, o.split_once(':').map(|(_, t)| t))
                }
                _ => {}
            }
            glib::timeout_add_local_once(Duration::from_millis(shot.wait), move || {
                let (min, nat, _, _) = window.measure(gtk::Orientation::Horizontal, -1);
                eprintln!(
                    "window {}x{} (min width {min}, natural {nat})",
                    window.width(),
                    window.height()
                );
                match save_screenshot(&window, &shot.path) {
                    Ok(()) => println!("saved {}", shot.path.display()),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
                app.quit();
            });
        });
    });
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("800x700"), Some((800, 700)));
        assert_eq!(parse_size("800"), None);
    }
}
