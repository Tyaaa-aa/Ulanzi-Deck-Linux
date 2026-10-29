//! Connecting to OBS Studio: the status, the connection settings, "Find OBS Settings" and "Turn On
//! WebSocket Server". Shown in Preferences and as a sheet when an OBS action is chosen offline.

use adw::prelude::*;
use gtk::glib;

use crate::runtime;
use crate::state::{Change, State};

/// Icon and style for an OBS state.
pub fn state_icon(state: &str) -> (&'static str, &'static str) {
    match state {
        "connected" => ("object-select-symbolic", "success"),
        "connecting" | "" => ("content-loading-symbolic", "dim-label"),
        "not-running" | "dropped" => ("media-playback-pause-symbolic", "dim-label"),
        "not-installed" => ("dialog-information-symbolic", "dim-label"),
        _ => ("dialog-warning-symbolic", "warning"),
    }
}

/// Which extra buttons a state needs: (turn on the server, get OBS).
pub fn helpers_for(state: &str) -> (bool, bool) {
    (
        matches!(state, "server-off" | "not-running" | "wrong-port"),
        state == "not-installed",
    )
}

fn reload_settings(state: &State, port: &adw::SpinRow, password: &adw::PasswordEntryRow) {
    let Some(c) = state.client.borrow().clone() else {
        return;
    };
    let (s, port, password) = (state.clone(), port.downgrade(), password.downgrade());
    runtime::run(async move { c.settings().await }, move |r| {
        if let Ok(new) = r {
            if let Some(p) = port.upgrade() {
                p.set_value(new.obs.port as f64);
            }
            if let Some(p) = password.upgrade() {
                p.set_text(new.obs.password.as_deref().unwrap_or(""));
            }
            *s.settings.borrow_mut() = new;
        }
    });
}

/// The OBS group: status, settings and helpers. `change` saves a settings edit.
pub fn group(
    state: &State,
    change: impl Fn(&State, Box<dyn FnOnce(&mut ulanzi_core::model::Settings)>) + 'static,
) -> adw::PreferencesGroup {
    let change = std::rc::Rc::new(change);
    let s = state.settings.borrow().clone();
    let group = adw::PreferencesGroup::builder()
        .title("OBS Studio")
        .description("Scenes, sources, recording and streaming from the deck, through OBS’s WebSocket server (OBS 28 or newer)")
        .build();

    let status = adw::ActionRow::builder().build();
    let icon = gtk::Image::new();
    status.add_prefix(&icon);
    let reconnect = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Try Again Now")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    status.add_suffix(&reconnect);
    {
        let state = state.clone();
        reconnect.connect_clicked(move |_| {
            if let Some(c) = state.client.borrow().clone() {
                runtime::run(async move { c.obs_reconnect().await }, |_| {});
            }
        });
    }
    group.add(&status);

    let host = adw::EntryRow::builder().title("Host").text(&s.obs.host).build();
    let port = adw::SpinRow::with_range(1.0, 65535.0, 1.0);
    port.set_title("Port");
    port.set_value(s.obs.port as f64);
    let password = adw::PasswordEntryRow::builder()
        .title("Password")
        .text(s.obs.password.as_deref().unwrap_or(""))
        .build();
    {
        let (state, change) = (state.clone(), change.clone());
        host.connect_changed(move |r| {
            let v = r.text().to_string();
            change(&state, Box::new(move |s| s.obs.host = v))
        });
    }
    {
        let (state, change) = (state.clone(), change.clone());
        port.connect_value_notify(move |r| {
            let v = r.value() as u16;
            change(&state, Box::new(move |s| s.obs.port = v))
        });
    }
    {
        let (state, change) = (state.clone(), change.clone());
        password.connect_changed(move |r| {
            let v = r.text().to_string();
            change(
                &state,
                Box::new(move |s| s.obs.password = (!v.is_empty()).then_some(v)),
            )
        });
    }

    let find = adw::ButtonRow::builder()
        .title("Find OBS Settings")
        .start_icon_name("system-search-symbolic")
        .build();
    {
        let (state, port, password) = (state.clone(), port.clone(), password.clone());
        find.connect_activated(move |_| {
            let Some(c) = state.client.borrow().clone() else {
                return;
            };
            let (s, port, password) = (state.clone(), port.clone(), password.clone());
            runtime::run(async move { c.obs_use_detected().await }, move |r| match r {
                Ok(msg) => {
                    crate::ui::toast(&s, &msg);
                    reload_settings(&s, &port, &password);
                }
                Err(e) => crate::ui::toast(&s, &e.to_string()),
            });
        });
    }
    let enable = adw::ButtonRow::builder()
        .title("Turn On WebSocket Server")
        .start_icon_name("network-server-symbolic")
        .build();
    {
        let state = state.clone();
        enable.connect_activated(move |_| {
            let Some(c) = state.client.borrow().clone() else {
                return;
            };
            let s = state.clone();
            runtime::run(async move { c.obs_enable_server().await }, move |r| match r {
                Ok(msg) => crate::ui::toast(&s, &msg),
                Err(e) => crate::ui::toast(&s, &e.to_string()),
            });
        });
    }
    let get = adw::ButtonRow::builder()
        .title("Get OBS Studio")
        .end_icon_name("adw-external-link-symbolic")
        .build();
    get.connect_activated(|b| {
        let window = b.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new("https://obsproject.com").launch(
            window.as_ref(),
            None::<&gtk::gio::Cancellable>,
            |_| {},
        );
    });

    group.add(&host);
    group.add(&port);
    group.add(&password);
    group.add(&find);
    group.add(&enable);
    group.add(&get);

    // Follow the connection.
    let show = {
        let (state, status, icon, enable, get) = (
            state.clone(),
            status.downgrade(),
            icon.downgrade(),
            enable.downgrade(),
            get.downgrade(),
        );
        move || {
            let o = state.obs.borrow().clone();
            let (Some(status), Some(icon), Some(enable), Some(get)) =
                (status.upgrade(), icon.upgrade(), enable.upgrade(), get.upgrade())
            else {
                return;
            };
            let title = if o.title.is_empty() {
                "Connecting…".to_string()
            } else {
                o.title.clone()
            };
            status.set_title(&glib::markup_escape_text(&title));
            status.set_subtitle(&glib::markup_escape_text(&o.detail));
            let (name, class) = state_icon(&o.state);
            icon.set_icon_name(Some(name));
            for c in ["success", "warning", "dim-label"] {
                icon.remove_css_class(c);
            }
            icon.add_css_class(class);
            let (server, install) = helpers_for(&o.state);
            enable.set_visible(server);
            get.set_visible(install);
        }
    };
    show();
    state.on(move |c| {
        if c == Change::Obs {
            show();
        }
    });
    group
}

/// The sheet shown when an OBS action is chosen while OBS isn't connected.
pub fn open_dialog(state: &State, parent: &impl IsA<gtk::Widget>) {
    let page = adw::PreferencesPage::new();
    let intro = adw::PreferencesGroup::builder()
        .description(
            "1. Open OBS Studio.\n2. Press Find OBS Settings (or turn on its WebSocket server).\n3. When it says Connected, pick the scene or source for your key.",
        )
        .build();
    page.add(&intro);
    page.add(&group(state, |state, f| {
        crate::ui::preferences::change_boxed(state, f)
    }));
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title("Connect to OBS")
        .content_width(520)
        .content_height(620)
        .child(&toolbar)
        .build();
    dialog.present(Some(parent));
}

thread_local! {
    static OFFERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Offer the setup sheet once per run, when an OBS action is chosen while OBS isn't connected.
pub fn offer_once(state: &State, parent: &impl IsA<gtk::Widget>) {
    if state.obs.borrow().connected() || OFFERED.with(|o| o.replace(true)) {
        return;
    }
    open_dialog(state, parent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_get_the_right_helpers() {
        assert_eq!(helpers_for("server-off"), (true, false));
        assert_eq!(helpers_for("not-installed"), (false, true));
        assert_eq!(helpers_for("connected"), (false, false));
        assert_eq!(state_icon("connected").1, "success");
        assert_eq!(state_icon("auth-failed").1, "warning");
    }
}
