//! The main window: sidebar, header with status, device canvas, inspector.

use adw::prelude::*;
use gtk::{gio, glib};
use ulanzi_core::edit::Slot;
use uuid::Uuid;

use crate::client::{Client, ClientEvent};
use crate::runtime;
use crate::state::{Change, State, Status, status_text};
use crate::ui::{canvas, inspector, sidebar};

pub struct Window {
    pub window: adw::ApplicationWindow,
    pub inner_split: adw::OverlaySplitView,
    pub library_toggle: gtk::ToggleButton,
}

fn status_pill(state: &State) -> gtk::MenuButton {
    let dot = gtk::Box::builder()
        .css_classes(["status-dot"])
        .valign(gtk::Align::Center)
        .build();
    let model = gtk::Label::builder().css_classes(["heading"]).build();
    let what = gtk::Label::builder().css_classes(["dim-label"]).build();
    let content = gtk::Box::builder().spacing(8).build();
    content.append(&dot);
    content.append(&model);
    content.append(&what);

    let detail = gtk::Label::builder()
        .wrap(true)
        .max_width_chars(36)
        .xalign(0.0)
        .build();
    let popover_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    popover_box.append(&detail);
    let popover = gtk::Popover::builder().child(&popover_box).build();

    let button = gtk::MenuButton::builder()
        .child(&content)
        .popover(&popover)
        .css_classes(["status-pill"])
        .tooltip_text("Deck status")
        .build();
    let update = {
        let state = state.clone();
        move |c: Change| {
            if c != Change::Status {
                return;
            }
            let s = state.status.borrow().clone();
            let (m, w) = status_text(&s);
            model.set_label(&m);
            what.set_label(&w);
            for cls in ["ok", "warn", "bad"] {
                dot.remove_css_class(cls);
            }
            let (cls, text) = match &s {
                Status::Connecting => ("warn", "Looking for the Ulanzi Deck service…".to_string()),
                Status::ServiceDown => (
                    "bad",
                    "The background service is not running, so the deck is idle. Start it from the banner."
                        .into(),
                ),
                Status::Device {
                    state: st, detail, ..
                } if st == "ready" => ("ok", format!("Connected at {detail}.")),
                Status::Device { detail, .. } => ("warn", format!("{detail}.")),
            };
            dot.add_css_class(cls);
            detail.set_label(&text);
        }
    };
    update(Change::Status);
    state.on(update);
    button
}

fn banner(state: &State) -> adw::Banner {
    let banner = adw::Banner::new("");
    {
        let state = state.clone();
        banner.connect_button_clicked(move |b| {
            if matches!(*state.status.borrow(), Status::ServiceDown) {
                b.set_button_label(Some("Starting…"));
                runtime::run(
                    async {
                        tokio::process::Command::new("systemctl")
                            .args(["--user", "start", "ulanzi-deckd"])
                            .status()
                            .await
                    },
                    |_| {},
                );
            } else if let Some(root) = b.root() {
                crate::ui::onboarding::open(&state, &root, Some("permissions"));
            }
        });
    }
    let update = {
        let (state, banner) = (state.clone(), banner.clone());
        move |c: Change| {
            if c != Change::Status {
                return;
            }
            let s = state.status.borrow().clone();
            let (show, title, button) = match &s {
                Status::ServiceDown => (true, "The Ulanzi Deck service is not running", Some("Start")),
                Status::Device { state: st, .. } if st == "no_permission" => (
                    true,
                    "This computer needs permission to use the deck",
                    Some("Fix…"),
                ),
                Status::Device { state: st, .. } if st == "busy" => {
                    (true, "Another program is using the deck", None)
                }
                Status::Device { state: st, .. } if st == "not_found" => {
                    (true, "No deck connected. Plug it in to see changes live.", None)
                }
                _ => (false, "", None),
            };
            banner.set_title(title);
            banner.set_button_label(button);
            banner.set_revealed(show && !state.demo.get());
        }
    };
    update(Change::Status);
    state.on(update);
    banner
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let file = gio::Menu::new();
    file.append(Some("Import"), Some("app.import"));
    file.append(Some("Export"), Some("app.export"));
    menu.append_section(None, &file);
    let app = gio::Menu::new();
    app.append(Some("Preferences"), Some("app.preferences"));
    app.append(Some("Set Up Deck"), Some("app.onboarding"));
    app.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    app.append(Some("About Ulanzi Deck"), Some("app.about"));
    menu.append_section(None, &app);
    menu
}

fn window_title(state: &State, title: &adw::WindowTitle) {
    let e = state.editor.borrow();
    let p = e.profile();
    let page = &p.pages[e.page()];
    title.set_title(&page.name);
    title.set_subtitle(&format!(
        "{} · Page {} of {}",
        p.name,
        e.page() + 1,
        p.pages.len()
    ));
}

pub fn build(app: &adw::Application, state: &State) -> Window {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Ulanzi Deck")
        .default_width(1280)
        .default_height(800)
        .width_request(360)
        .height_request(480)
        .build();

    // Sidebar.
    let side_header = adw::HeaderBar::builder()
        .title_widget(&adw::WindowTitle::new("Ulanzi Deck", ""))
        .show_end_title_buttons(false)
        .build();
    let side_view = adw::ToolbarView::new();
    side_view.add_top_bar(&side_header);
    side_view.set_content(Some(&sidebar::build(state)));

    // Content header.
    let title = adw::WindowTitle::new("", "");
    window_title(state, &title);
    let header = adw::HeaderBar::builder().title_widget(&title).build();
    header.pack_start(&status_pill(state));
    let menu = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&main_menu())
        .tooltip_text("Main Menu")
        .build();
    header.pack_end(&menu);
    let library_toggle = gtk::ToggleButton::builder()
        .icon_name("view-app-grid-symbolic")
        .tooltip_text("Actions (Ctrl+L)")
        .build();
    header.pack_end(&library_toggle);
    {
        let a = gio::SimpleAction::new("library", None);
        let t = library_toggle.clone();
        a.connect_activate(move |_, _| t.set_active(!t.is_active()));
        window.add_action(&a);
        app.set_accels_for_action("win.library", &["<Ctrl>l"]);
    }
    let redo = gtk::Button::builder()
        .icon_name("edit-redo-symbolic")
        .tooltip_text("Redo (Ctrl+Shift+Z)")
        .action_name("win.redo")
        .build();
    let undo = gtk::Button::builder()
        .icon_name("edit-undo-symbolic")
        .tooltip_text("Undo (Ctrl+Z)")
        .action_name("win.undo")
        .build();
    header.pack_end(&redo);
    header.pack_end(&undo);
    {
        let (state, title) = (state.clone(), title.clone());
        let st = state.clone();
        st.on(move |c| {
            if matches!(c, Change::Profile | Change::Page) {
                window_title(&state, &title);
            }
        });
    }

    // Canvas + inspector.
    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&header);
    content_view.add_top_bar(&banner(state));
    let inner_split = adw::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::End)
        .min_sidebar_width(320.0)
        .max_sidebar_width(380.0)
        .sidebar_width_fraction(0.3)
        .build();
    let canvas = canvas::build(state);
    content_view.set_content(Some(&canvas.widget));
    // The right panel runs the full height of the window, beside the canvas and its header.
    inner_split.set_content(Some(&content_view));
    let panel = inspector::Panel::build(state, &library_toggle);
    inner_split.set_sidebar(Some(&panel.root));
    // Window buttons belong to the rightmost header: the inspector's while it is shown beside the
    // canvas, the content header's otherwise.
    let sync_buttons = {
        let (header, split) = (header.clone(), inner_split.clone());
        move || header.set_show_end_title_buttons(split.is_collapsed() || !split.shows_sidebar())
    };
    sync_buttons();
    {
        let f = sync_buttons.clone();
        inner_split.connect_collapsed_notify(move |_| f());
    }
    inner_split.connect_show_sidebar_notify(move |_| sync_buttons());

    let outer_split = adw::OverlaySplitView::builder()
        .min_sidebar_width(240.0)
        .max_sidebar_width(300.0)
        .sidebar(&side_view)
        .content(&inner_split)
        .build();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&outer_split));
    crate::ui::set_toast_overlay(&toasts);
    window.set_content(Some(&toasts));

    // Narrow windows: both side panels become overlays.
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        900.0,
        adw::LengthUnit::Sp,
    ));
    narrow.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
    narrow.add_setter(&inner_split, "collapsed", Some(&true.to_value()));
    {
        let f = canvas.set_compact.clone();
        narrow.connect_apply(move |_| f(true));
        let f = canvas.set_compact.clone();
        narrow.connect_unapply(move |_| f(false));
    }
    // With the sidebar folded away, a header button brings it back.
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Profiles and Pages")
        .visible(false)
        .build();
    header.pack_start(&sidebar_toggle);
    outer_split
        .bind_property("collapsed", &sidebar_toggle, "visible")
        .sync_create()
        .build();
    outer_split
        .bind_property("show-sidebar", &sidebar_toggle, "active")
        .sync_create()
        .bidirectional()
        .build();
    // The panel's own window buttons only while it sits beside the canvas.
    inner_split
        .bind_property("collapsed", &panel.header, "show-end-title-buttons")
        .sync_create()
        .invert_boolean()
        .build();
    window.add_breakpoint(narrow);
    {
        let (inner, state) = (inner_split.clone(), state.clone());
        let st = state.clone();
        st.on(move |c| {
            if c == Change::Selection && inner.is_collapsed() {
                inner.set_show_sidebar(!state.selection.borrow().is_empty());
            }
        });
    }

    install_actions(&window, state);
    Window {
        window,
        inner_split,
        library_toggle,
    }
}

fn install_actions(window: &adw::ApplicationWindow, state: &State) {
    let action = |name: &str, f: Box<dyn Fn()>| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        window.add_action(&a);
    };
    let s = state.clone();
    action("undo", Box::new(move || s.undo()));
    let s = state.clone();
    action("redo", Box::new(move || s.redo()));
    let s = state.clone();
    action(
        "select-all",
        Box::new(move || {
            s.select(|sel| sel.select_all_keys());
        }),
    );
    let s = state.clone();
    action(
        "copy",
        Box::new(move || {
            let keys = s.selection.borrow().keys();
            s.editor.borrow_mut().copy_keys(&keys);
        }),
    );
    let s = state.clone();
    action(
        "paste",
        Box::new(move || {
            let at = s.selection.borrow().keys().first().copied();
            if let Some(at) = at {
                s.edit(|e| e.paste_keys(at));
            }
        }),
    );
    let s = state.clone();
    action(
        "copy-style",
        Box::new(move || {
            if let Some(k) = s.selection.borrow().keys().first().copied() {
                s.editor.borrow_mut().copy_style(k);
            }
        }),
    );
    let s = state.clone();
    action(
        "paste-style",
        Box::new(move || {
            let keys = s.selection.borrow().keys();
            s.edit(|e| e.paste_style(&keys));
        }),
    );
    let s = state.clone();
    action(
        "clear",
        Box::new(move || {
            let keys = s.selection.borrow().keys();
            s.edit(|e| e.clear(&keys));
        }),
    );
    {
        let pick = gio::SimpleAction::new("pick-image", None);
        let (s, w) = (state.clone(), window.clone());
        pick.connect_activate(move |_, _| crate::ui::icon_picker::open(&s, &w));
        let update = {
            let (s, pick) = (state.clone(), pick.clone());
            move || pick.set_enabled(!s.selection.borrow().keys().is_empty())
        };
        update();
        state.on(move |c| {
            if c == Change::Selection {
                update();
            }
        });
        window.add_action(&pick);
    }
    {
        let w = window.clone();
        action("shortcuts", Box::new(move || crate::ui::shortcuts::open(&w)));
    }
    let s = state.clone();
    action(
        "deselect",
        Box::new(move || {
            s.select(|sel| sel.clear());
        }),
    );
    if let Some(app) = window.application() {
        for (action, accels) in APP_ACCELS {
            app.set_accels_for_action(action, accels);
        }
    }
    // Undo, select, copy, paste and clear yield to a focused text field, so they can't be
    // application accelerators (those fire before the focused widget).
    crate::ui::key_routing::install(window);
}

/// Window shortcuts that fire everywhere, text fields included.
const APP_ACCELS: &[(&str, &[&str])] = &[
    ("win.shortcuts", &["<Control>question"]),
    ("win.deselect", &["Escape"]),
];

/// Show profile `id` in the editor, and make it the deck's active profile when `activate` is set.
pub fn open_profile(state: &State, id: Uuid, activate: bool) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let state = state.clone();
    runtime::run(
        async move {
            if activate {
                let _ = c.switch_profile(id).await;
            }
            let profile = c.profile(id).await;
            let list = c.profiles().await;
            (profile, list)
        },
        move |(profile, list)| {
            if let Ok(list) = list {
                *state.profiles.borrow_mut() = list;
            }
            if let Ok(p) = profile {
                state.load_profile(p);
            }
            state.notify(Change::Profiles);
        },
    );
}

thread_local! {
    static SETTINGS_RELOAD: std::cell::RefCell<Option<glib::SourceId>> = const { std::cell::RefCell::new(None) };
}

/// Read the settings again shortly (a turning brightness dial sends many changes).
fn reload_settings_soon(state: &State) {
    SETTINGS_RELOAD.with(|r| {
        if let Some(id) = r.borrow_mut().take() {
            id.remove();
        }
        let state = state.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            SETTINGS_RELOAD.with(|r| r.borrow_mut().take());
            let Some(c) = state.client.borrow().clone() else {
                return;
            };
            let s = state.clone();
            runtime::run(async move { c.settings().await }, move |r| {
                if let Ok(new) = r
                    && *s.settings.borrow() != new
                {
                    *s.settings.borrow_mut() = new;
                    s.notify(Change::Settings);
                }
            });
        });
        *r.borrow_mut() = Some(id);
    });
}

/// Record the OBS state; when connected, fetch the scene list (for pickers and warnings).
pub fn set_obs(state: &State, st: String, title: String, detail: String) {
    let connected = st == "connected";
    {
        let mut o = state.obs.borrow_mut();
        o.state = st;
        o.title = title;
        o.detail = detail;
        if !connected {
            o.scenes.clear();
        }
    }
    state.notify(Change::Obs);
    if connected && let Some(c) = state.client.borrow().clone() {
        let s = state.clone();
        runtime::run(async move { c.obs_list("scenes", "").await }, move |r| {
            if let Ok(list) = r {
                s.obs.borrow_mut().scenes = list;
                s.notify(Change::Obs);
            }
        });
    }
}

/// Fetch the plugin list and manifests. The list counts as known (key badges show) only once
/// both arrived; a failure (the plugin host still coming up) leaves it as it was. Only the newest
/// fetch is stored, and none that was in flight when the service went away.
pub fn refresh_plugins(state: &State) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let s = state.clone();
    let generation = state.plugins_fetch_started();
    runtime::run(
        async move {
            let list = c.plugin_list().await?;
            let manifests = c.plugin_manifests().await?;
            Ok::<_, crate::client::ClientError>((list, manifests))
        },
        move |r| match r {
            Ok((list, manifests)) => {
                // An older fetch, or one from before the service went away, is dropped.
                if s.plugins_fetched(generation, list, manifests) {
                    s.notify(Change::Plugins);
                }
            }
            Err(e) => tracing::debug!("plugin list: {e}"),
        },
    );
}

/// Connect to the service, load the active profile and settings, and follow events.
pub fn connect(state: &State, ready: impl FnOnce() + 'static) {
    let state = state.clone();
    runtime::run(
        async {
            let c = Client::connect().await?;
            let hello = c.hello().await?;
            let profile = c.profile(hello.active_profile).await?;
            let profiles = c.profiles().await?;
            let settings = c.settings().await?;
            Ok::<_, crate::client::ClientError>((c, hello, profile, profiles, settings))
        },
        move |r| {
            match r {
                Ok((c, hello, profile, profiles, settings)) => {
                    follow_events(&state, &c);
                    *state.client.borrow_mut() = Some(c);
                    *state.profiles.borrow_mut() = profiles;
                    *state.settings.borrow_mut() = settings;
                    *state.status.borrow_mut() = Status::Device {
                        state: hello.device_state,
                        detail: hello.device_detail,
                        model: hello.model,
                    };
                    state.load_profile(profile);
                    if (hello.active_page as usize) < state.editor.borrow().profile().pages.len() {
                        state.set_page(hello.active_page as usize);
                    }
                    state.notify(Change::Profiles);
                    state.notify(Change::Settings);
                    refresh_plugins(&state);
                    if let Some(c) = state.client.borrow().clone() {
                        let s = state.clone();
                        runtime::run(async move { c.obs_status().await }, move |r| {
                            if let Ok((st, title, detail)) = r {
                                set_obs(&s, st, title, detail);
                            }
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("{e}");
                    *state.status.borrow_mut() = Status::ServiceDown;
                }
            }
            state.notify(Change::Status);
            ready();
        },
    );
}

fn slot_of(control: &str) -> Option<Slot> {
    let (kind, n) = control.split_once(':').unwrap_or((control, ""));
    let n: u8 = n.parse().unwrap_or(0);
    match kind {
        "key" => Some(Slot::Key(n)),
        "small" => Some(Slot::Small(n)),
        "dial" => Some(Slot::Dial(n)),
        "screen" => Some(Slot::Screen),
        _ => None,
    }
}

fn follow_events(state: &State, c: &Client) {
    let (tx, rx) = async_channel::unbounded::<ClientEvent>();
    let mut events = c.events();
    runtime::runtime().spawn(async move {
        while let Ok(e) = events.recv().await {
            if tx.send(e).await.is_err() {
                break;
            }
        }
    });
    let state = state.clone();
    glib::spawn_future_local(async move {
        while let Ok(e) = rx.recv().await {
            match e {
                ClientEvent::State { state: st, detail } => {
                    let model = match &*state.status.borrow() {
                        Status::Device { model, .. } => model.clone(),
                        _ => String::new(),
                    };
                    *state.status.borrow_mut() = Status::Device {
                        state: st,
                        detail,
                        model,
                    };
                    state.notify(Change::Status);
                }
                ClientEvent::Disconnected => {
                    *state.status.borrow_mut() = Status::ServiceDown;
                    state.notify(Change::Status);
                    state.plugins_lost();
                    state.notify(Change::Plugins);
                }
                ClientEvent::PluginsChanged => refresh_plugins(&state),
                ClientEvent::Reconnected => {
                    refresh_plugins(&state);
                    let s = state.clone();
                    if let Some(c) = state.client.borrow().clone() {
                        runtime::run(async move { c.hello().await }, move |h| {
                            if let Ok(h) = h {
                                *s.status.borrow_mut() = Status::Device {
                                    state: h.device_state,
                                    detail: h.device_detail,
                                    model: h.model,
                                };
                                s.notify(Change::Status);
                            }
                        });
                    }
                }
                ClientEvent::Input { control, kind }
                    if kind == "press" || kind == "left" || kind == "right" =>
                {
                    if let Some(slot) = slot_of(&control) {
                        state.notify(Change::Pressed(slot));
                    }
                }
                ClientEvent::Page { profile, page } if profile == state.profile_id() => {
                    if page as usize != state.editor.borrow().page() {
                        state.set_page(page as usize);
                    }
                }
                ClientEvent::Page { profile, .. } => open_profile(&state, profile, false),
                ClientEvent::ProfilesChanged => {
                    let s = state.clone();
                    if let Some(c) = state.client.borrow().clone() {
                        runtime::run(async move { c.profiles().await }, move |l| {
                            if let Ok(l) = l {
                                *s.profiles.borrow_mut() = l;
                                s.notify(Change::Profiles);
                            }
                        });
                    }
                }
                ClientEvent::SettingsChanged => reload_settings_soon(&state),
                ClientEvent::Obs {
                    state: st,
                    title,
                    detail,
                } => {
                    set_obs(&state, st, title, detail);
                }
                ClientEvent::ActionFailed { control, message } => {
                    tracing::info!("{control}: {message}");
                    crate::ui::toast(&state, &message);
                }
                ClientEvent::Conflict { .. } => {
                    crate::ui::toast(&state, "Another app changed this profile; your edits were kept");
                }
                ClientEvent::SaveFailed { message } => {
                    crate::ui::toast(&state, &format!("Not saved: {message}"))
                }
                _ => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_menu_labels_have_no_ellipsis() {
        crate::ui::assert_menu_labels(&main_menu());
    }

    #[test]
    fn app_accelerators_leave_text_keys_alone() {
        // Application accelerators beat the focused widget, so none may be a text-editing key.
        for (action, accels) in APP_ACCELS {
            for accel in *accels {
                assert!(
                    crate::ui::key_routing::window_takes(accel, true),
                    "{action} steals {accel} from text fields"
                );
            }
        }
    }

    #[test]
    fn control_names_map_to_slots() {
        assert_eq!(slot_of("key:7"), Some(Slot::Key(7)));
        assert_eq!(slot_of("small:1"), Some(Slot::Small(1)));
        assert_eq!(slot_of("dial:2"), Some(Slot::Dial(2)));
        assert_eq!(slot_of("screen"), Some(Slot::Screen));
        assert_eq!(slot_of("unknown:40"), None);
    }
}
