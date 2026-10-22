//! Preferences: appearance, background service, device, label style and integrations.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::{gdk, glib};
use ulanzi_core::model::{AppearancePref, Settings};

use crate::runtime;
use crate::state::{Change, State};

pub fn appearance_to_color_scheme(a: AppearancePref) -> adw::ColorScheme {
    match a {
        AppearancePref::System => adw::ColorScheme::Default,
        AppearancePref::Light => adw::ColorScheme::ForceLight,
        AppearancePref::Dark => adw::ColorScheme::ForceDark,
    }
}

/// Port and password from obs-websocket's `config.json` (the password only when it is required).
pub fn obs_from_websocket_config(json: &str) -> Option<(u16, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let port = u16::try_from(v.get("server_port")?.as_u64()?).ok()?;
    let auth = v.get("auth_required").and_then(|a| a.as_bool()).unwrap_or(false);
    let password = auth
        .then(|| {
            v.get("server_password")
                .and_then(|p| p.as_str())
                .map(String::from)
        })
        .flatten()
        .filter(|p| !p.is_empty());
    Some((port, password))
}

/// Apply the appearance setting now and whenever settings change.
pub fn follow_appearance(state: &State) {
    let apply = {
        let state = state.clone();
        move || {
            adw::StyleManager::default()
                .set_color_scheme(appearance_to_color_scheme(state.settings.borrow().appearance))
        }
    };
    apply();
    state.on(move |c| {
        if c == Change::Settings {
            apply();
        }
    });
}

thread_local! {
    static PENDING: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

/// Change a setting and send it to the service (debounced, so sliders don't flood it).
fn change(state: &State, f: impl FnOnce(&mut Settings)) {
    f(&mut state.settings.borrow_mut());
    PENDING.with(|p| {
        if let Some(id) = p.borrow_mut().take() {
            id.remove();
        }
        let state = state.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            PENDING.with(|p| p.borrow_mut().take());
            let s = state.settings.borrow().clone();
            if let Some(c) = state.client.borrow().clone() {
                let st = state.clone();
                runtime::run(async move { c.apply_settings(&s).await }, move |r| {
                    if let Err(e) = r {
                        crate::ui::toast(&st, &format!("Settings not saved: {e}"));
                    }
                });
            }
        });
        *p.borrow_mut() = Some(id);
    });
}

fn spin(title: &str, subtitle: &str, (min, max, step): (f64, f64, f64), value: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(title);
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row.set_value(value);
    row
}

fn general_page(state: &State) -> adw::PreferencesPage {
    let s = state.settings.borrow().clone();
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();

    let look = adw::PreferencesGroup::builder().title("Appearance").build();
    let style = adw::ComboRow::builder()
        .title("Style")
        .model(&gtk::StringList::new(&["Follow System", "Light", "Dark"]))
        .selected(match s.appearance {
            AppearancePref::System => 0,
            AppearancePref::Light => 1,
            AppearancePref::Dark => 2,
        })
        .build();
    {
        let state = state.clone();
        style.connect_selected_notify(move |r| {
            let a = match r.selected() {
                1 => AppearancePref::Light,
                2 => AppearancePref::Dark,
                _ => AppearancePref::System,
            };
            adw::StyleManager::default().set_color_scheme(appearance_to_color_scheme(a));
            change(&state, |s| s.appearance = a);
        });
    }
    look.add(&style);
    page.add(&look);

    let bg = adw::PreferencesGroup::builder()
        .title("Background Service")
        .description("The service runs the deck while this window is closed")
        .build();
    let login = adw::SwitchRow::builder().title("Start at Login").build();
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
                let (Some(login), Ok(o)) = (login.upgrade(), r) else {
                    return;
                };
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
            },
        );
    }
    bg.add(&login);
    let tray = adw::SwitchRow::builder()
        .title("Tray Icon")
        .subtitle("Profiles, brightness and a shortcut to this window")
        .active(s.tray)
        .build();
    {
        let state = state.clone();
        tray.connect_active_notify(move |r| {
            let v = r.is_active();
            change(&state, |s| s.tray = v)
        });
    }
    bg.add(&tray);
    let auto = adw::SwitchRow::builder()
        .title("Switch Profiles by App")
        .subtitle("Right-click a profile → Auto-Switch… to choose its apps")
        .active(s.auto_switch)
        .build();
    {
        let state = state.clone();
        auto.connect_active_notify(move |r| {
            let v = r.is_active();
            change(&state, |s| s.auto_switch = v)
        });
    }
    bg.add(&auto);
    page.add(&bg);

    let keys = adw::PreferencesGroup::builder().title("Keys").build();
    let hold = spin(
        "Hold Time",
        "Milliseconds before a press counts as a hold",
        (200.0, 1500.0, 50.0),
        s.hold_ms as f64,
    );
    {
        let state = state.clone();
        hold.connect_value_notify(move |r| {
            let v = r.value() as u32;
            change(&state, |s| s.hold_ms = v)
        });
    }
    keys.add(&hold);
    page.add(&keys);
    page
}

fn rgb_u32(c: &gdk::RGBA) -> u32 {
    let v = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u32;
    (v(c.red()) << 16) | (v(c.green()) << 8) | v(c.blue())
}

fn device_page(state: &State) -> adw::PreferencesPage {
    let s = state.settings.borrow().clone();
    let page = adw::PreferencesPage::builder()
        .title("Device")
        .icon_name("input-keyboard-symbolic")
        .build();

    let display = adw::PreferencesGroup::builder().title("Display").build();
    let brightness = spin("Brightness", "Percent", (0.0, 100.0, 5.0), s.brightness as f64);
    {
        let state = state.clone();
        brightness.connect_value_notify(move |r| {
            let v = r.value() as u8;
            change(&state, |s| s.brightness = v);
            state.notify(Change::Settings);
        });
    }
    display.add(&brightness);
    let sleep = spin(
        "Dim After",
        "Minutes without a press; 0 keeps it on",
        (0.0, 240.0, 1.0),
        s.sleep_after_min as f64,
    );
    {
        let state = state.clone();
        sleep.connect_value_notify(move |r| {
            let v = r.value() as u32;
            change(&state, |s| s.sleep_after_min = v)
        });
    }
    display.add(&sleep);
    let dim = spin(
        "Dimmed Brightness",
        "Percent while dimmed; 0 turns the screens off",
        (0.0, 100.0, 5.0),
        s.sleep_brightness as f64,
    );
    {
        let state = state.clone();
        dim.connect_value_notify(move |r| {
            let v = r.value() as u8;
            change(&state, |s| s.sleep_brightness = v)
        });
    }
    display.add(&dim);
    page.add(&display);

    let labels = adw::PreferencesGroup::builder()
        .title("Key Labels")
        .description("How the deck draws the text under each key")
        .build();
    let hide = adw::SwitchRow::builder()
        .title("Hide All Labels")
        .active(s.hide_labels)
        .build();
    {
        let state = state.clone();
        hide.connect_active_notify(move |r| {
            let v = r.is_active();
            change(&state, |s| s.hide_labels = v)
        });
    }
    labels.add(&hide);
    const ALIGNS: [(&str, &str); 3] = [("top", "Top"), ("center", "Middle"), ("bottom", "Bottom")];
    let pos = adw::ComboRow::builder()
        .title("Position")
        .model(&gtk::StringList::new(&ALIGNS.map(|(_, t)| t)))
        .selected(
            ALIGNS
                .iter()
                .position(|(a, _)| *a == s.label_style.align)
                .unwrap_or(2) as u32,
        )
        .build();
    {
        let state = state.clone();
        pos.connect_selected_notify(move |r| {
            let a = ALIGNS[(r.selected() as usize).min(2)].0.to_string();
            change(&state, |s| s.label_style.align = a)
        });
    }
    labels.add(&pos);
    let size = spin("Text Size", "", (6.0, 24.0, 1.0), s.label_style.size as f64);
    {
        let state = state.clone();
        size.connect_value_notify(move |r| {
            let v = r.value() as u32;
            change(&state, |s| s.label_style.size = v)
        });
    }
    labels.add(&size);
    let color_row = adw::ActionRow::builder().title("Text Colour").build();
    let color = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).build()));
    color.set_valign(gtk::Align::Center);
    let c = s.label_style.color;
    color.set_rgba(&gdk::RGBA::new(
        ((c >> 16) & 0xff) as f32 / 255.0,
        ((c >> 8) & 0xff) as f32 / 255.0,
        (c & 0xff) as f32 / 255.0,
        1.0,
    ));
    {
        let state = state.clone();
        color.connect_rgba_notify(move |b| {
            let v = rgb_u32(&b.rgba());
            change(&state, |s| s.label_style.color = v)
        });
    }
    color_row.add_suffix(&color);
    color_row.set_activatable_widget(Some(&color));
    labels.add(&color_row);
    let reset = adw::ButtonRow::builder()
        .title("Reset Every Key to These Settings")
        .start_icon_name("edit-undo-symbolic")
        .build();
    {
        let state = state.clone();
        reset.connect_activated(move |b| reset_label_overrides(&state, b));
    }
    labels.add(&reset);
    page.add(&labels);
    page
}

fn integrations_page(state: &State) -> adw::PreferencesPage {
    let s = state.settings.borrow().clone();
    let page = adw::PreferencesPage::builder()
        .title("Integrations")
        .icon_name("application-x-addon-symbolic")
        .build();

    let obs = adw::PreferencesGroup::builder()
        .title("OBS Studio")
        .description("Scene, source, recording and streaming actions talk to OBS through obs-websocket")
        .build();
    let host = adw::EntryRow::builder().title("Host").text(&s.obs.host).build();
    let port = spin("Port", "", (1.0, 65535.0, 1.0), s.obs.port as f64);
    let password = adw::PasswordEntryRow::builder()
        .title("Password")
        .text(s.obs.password.as_deref().unwrap_or(""))
        .build();
    {
        let state = state.clone();
        host.connect_changed(move |r| {
            let v = r.text().to_string();
            change(&state, |s| s.obs.host = v)
        });
    }
    {
        let state = state.clone();
        port.connect_value_notify(move |r| {
            let v = r.value() as u16;
            change(&state, |s| s.obs.port = v)
        });
    }
    {
        let state = state.clone();
        password.connect_changed(move |r| {
            let v = r.text().to_string();
            change(&state, |s| s.obs.password = (!v.is_empty()).then_some(v))
        });
    }
    let read = adw::ButtonRow::builder()
        .title("Read from OBS Settings")
        .start_icon_name("document-open-symbolic")
        .build();
    {
        let (state, port, password) = (state.clone(), port.clone(), password.clone());
        read.connect_activated(move |_| {
            let path = glib::user_config_dir().join("obs-studio/plugin_config/obs-websocket/config.json");
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| obs_from_websocket_config(&t))
            {
                Some((p, pw)) => {
                    port.set_value(p as f64);
                    password.set_text(pw.as_deref().unwrap_or(""));
                    crate::ui::toast(&state, "Filled in from OBS");
                }
                None => crate::ui::toast(
                    &state,
                    "OBS settings not found. Turn on the WebSocket server in OBS (Tools menu) first",
                ),
            }
        });
    }
    obs.add(&host);
    obs.add(&port);
    obs.add(&password);
    obs.add(&read);
    page.add(&obs);

    let input = adw::PreferencesGroup::builder().title("Keyboard").build();
    let ok = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    let row = adw::ActionRow::builder()
        .title("Shortcuts and Typing")
        .subtitle(if ok {
            "Ready: keys can press shortcuts and type text"
        } else {
            "Needs permission for the virtual keyboard"
        })
        .build();
    if !ok {
        let fix = gtk::Button::builder()
            .label("Fix…")
            .valign(gtk::Align::Center)
            .build();
        let state = state.clone();
        fix.connect_clicked(move |b| {
            if let Some(root) = b.root() {
                crate::ui::onboarding::open(&state, &root, Some("permissions"));
            }
        });
        row.add_suffix(&fix);
    }
    input.add(&row);
    page.add(&input);
    page
}

/// Clear per-key label positions and hidden labels in every profile, after asking.
fn reset_label_overrides(state: &State, near: &impl IsA<gtk::Widget>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Reset Every Key’s Label?")
        .body(
            "Keys with their own label position or a hidden label go back to the settings above, \
             in every profile.",
        )
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("reset", "Reset");
    dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
    let state = state.clone();
    dialog.connect_response(Some("reset"), move |_, _| {
        // The open profile through the editor (undoable), the others directly.
        state.edit(|e| e.clear_label_overrides());
        let current = state.profile_id();
        let others: Vec<uuid::Uuid> = state
            .profiles
            .borrow()
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| *id != current)
            .collect();
        if let Some(c) = state.client.borrow().clone() {
            let st = state.clone();
            runtime::run(
                async move {
                    for id in others {
                        let mut p = c.profile(id).await?;
                        p.clear_label_overrides();
                        c.apply_now(&p).await?;
                    }
                    Ok::<(), crate::client::ClientError>(())
                },
                move |r| match r {
                    Ok(()) => crate::ui::toast(&st, "Labels reset on every key"),
                    Err(e) => crate::ui::toast(&st, &format!("Not all profiles were reset: {e}")),
                },
            );
        }
    });
    dialog.present(Some(near));
}

/// Open the preferences dialog.
pub fn open(state: &State, parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::PreferencesDialog::builder().search_enabled(true).build();
    dialog.add(&general_page(state));
    dialog.add(&device_page(state));
    dialog.add(&integrations_page(state));
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_maps_to_color_scheme() {
        assert_eq!(
            appearance_to_color_scheme(AppearancePref::System),
            adw::ColorScheme::Default
        );
        assert_eq!(
            appearance_to_color_scheme(AppearancePref::Light),
            adw::ColorScheme::ForceLight
        );
        assert_eq!(
            appearance_to_color_scheme(AppearancePref::Dark),
            adw::ColorScheme::ForceDark
        );
    }

    #[test]
    fn reads_obs_websocket_config() {
        let json = r#"{"alerts_enabled":false,"auth_required":true,"first_load":false,"server_enabled":true,"server_password":"s3cret","server_port":4456}"#;
        assert_eq!(
            obs_from_websocket_config(json),
            Some((4456, Some("s3cret".to_string())))
        );
        let no_auth = r#"{"auth_required":false,"server_password":"x","server_port":4455}"#;
        assert_eq!(obs_from_websocket_config(no_auth), Some((4455, None)));
        assert_eq!(obs_from_websocket_config("{}"), None);
    }
}
