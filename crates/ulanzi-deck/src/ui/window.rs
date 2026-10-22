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
                Status::Device { state: st, .. } if st == "no_permission" => {
                    (true, "This computer needs permission to use the deck", None)
                }
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
    file.append(Some("Import…"), Some("app.import"));
    file.append(Some("Export…"), Some("app.export"));
    menu.append_section(None, &file);
    let app = gio::Menu::new();
    app.append(Some("Preferences"), Some("app.preferences"));
    app.append(Some("Set Up Deck…"), Some("app.onboarding"));
    app.append(Some("Keyboard Shortcuts"), Some("win.show-help-overlay"));
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
    inner_split.set_content(Some(&canvas::build(state)));
    let panel = inspector::Panel::build(state, &library_toggle);
    inner_split.set_sidebar(Some(&panel.root));
    content_view.set_content(Some(&inner_split));
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
        .content(&content_view)
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
    Window { window, inner_split }
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
    let s = state.clone();
    action(
        "deselect",
        Box::new(move || {
            s.select(|sel| sel.clear());
        }),
    );
    if let Some(app) = window.application() {
        app.set_accels_for_action("win.undo", &["<Ctrl>z"]);
        app.set_accels_for_action("win.redo", &["<Ctrl><Shift>z", "<Ctrl>y"]);
        app.set_accels_for_action("win.select-all", &["<Ctrl>a"]);
        app.set_accels_for_action("win.copy", &["<Ctrl>c"]);
        app.set_accels_for_action("win.paste", &["<Ctrl>v"]);
        app.set_accels_for_action("win.copy-style", &["<Ctrl><Shift>c"]);
        app.set_accels_for_action("win.paste-style", &["<Ctrl><Shift>v"]);
        app.set_accels_for_action("win.clear", &["Delete"]);
        app.set_accels_for_action("win.deselect", &["Escape"]);
    }
}

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
                }
                ClientEvent::Reconnected => {
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
    fn control_names_map_to_slots() {
        assert_eq!(slot_of("key:7"), Some(Slot::Key(7)));
        assert_eq!(slot_of("small:1"), Some(Slot::Small(1)));
        assert_eq!(slot_of("dial:2"), Some(Slot::Dial(2)));
        assert_eq!(slot_of("screen"), Some(Slot::Screen));
        assert_eq!(slot_of("unknown:40"), None);
    }
}
