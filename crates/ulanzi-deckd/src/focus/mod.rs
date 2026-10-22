//! Knowing which app is in front, for auto profile switching.
//!
//! Wayland keeps this from ordinary programs, so each desktop needs its own route, picked at
//! start: a KWin script (KDE Plasma), a Shell extension (GNOME), the compositor's IPC socket
//! (Hyprland, Sway), or the `_NET_ACTIVE_WINDOW` property (X11). Every route ends in
//! `DeckMsg::Focus`, either directly or through our D-Bus `WindowActivated` method.

use std::sync::Mutex;

use tokio::sync::mpsc;

use crate::deck::DeckMsg;

pub mod gnome;
pub mod hyprland;
pub mod kwin;
pub mod sway;
pub mod x11;

/// How the focused window is learned on this desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    Kwin,
    Gnome,
    Hyprland {
        socket: std::path::PathBuf,
    },
    Sway {
        socket: std::path::PathBuf,
    },
    X11,
    /// Not available, with the reason in words.
    None(String),
}

/// What [`detect`] looks at.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub current_desktop: String,
    pub hyprland_signature: Option<String>,
    pub runtime_dir: Option<String>,
    pub swaysock: Option<String>,
    pub wayland: bool,
    pub x11_display: bool,
    pub kwin_on_bus: bool,
    pub gnome_shell_on_bus: bool,
}

impl Env {
    /// The real environment (bus names are looked up by the caller).
    pub fn from_process(kwin_on_bus: bool, gnome_shell_on_bus: bool) -> Env {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Env {
            current_desktop: var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            hyprland_signature: var("HYPRLAND_INSTANCE_SIGNATURE"),
            runtime_dir: var("XDG_RUNTIME_DIR"),
            swaysock: var("SWAYSOCK"),
            wayland: var("WAYLAND_DISPLAY").is_some(),
            x11_display: var("DISPLAY").is_some(),
            kwin_on_bus,
            gnome_shell_on_bus,
        }
    }
}

/// Pick the focus source for this session.
pub fn detect(env: &Env) -> Backend {
    let desktop = env.current_desktop.to_lowercase();
    if env.kwin_on_bus {
        return Backend::Kwin;
    }
    if let (Some(sig), Some(run)) = (&env.hyprland_signature, &env.runtime_dir) {
        return Backend::Hyprland {
            socket: std::path::Path::new(run)
                .join("hypr")
                .join(sig)
                .join(".socket2.sock"),
        };
    }
    if let Some(sock) = &env.swaysock {
        return Backend::Sway { socket: sock.into() };
    }
    if env.gnome_shell_on_bus || desktop.contains("gnome") {
        return Backend::Gnome;
    }
    if desktop.contains("cosmic") {
        return Backend::None("Auto-switch isn't available on COSMIC yet".into());
    }
    if env.x11_display && !env.wayland {
        return Backend::X11;
    }
    if env.wayland {
        return Backend::None("This Wayland desktop doesn't tell other apps which window is in front".into());
    }
    Backend::None("No desktop session found".into())
}

/// What the app shows in the Auto-Switch dialog: (backend, state, detail).
/// `state` is `active` (reports are arriving), `waiting` or `unavailable`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub backend: String,
    pub state: String,
    pub detail: String,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);

pub fn status() -> Status {
    STATUS.lock().ok().and_then(|s| s.clone()).unwrap_or(Status {
        backend: "none".into(),
        state: "unavailable".into(),
        detail: "Not started".into(),
    })
}

pub fn set_status(backend: &str, state: &str, detail: &str) {
    if let Ok(mut s) = STATUS.lock() {
        *s = Some(Status {
            backend: backend.into(),
            state: state.into(),
            detail: detail.into(),
        });
    }
}

/// A focus report arrived: the route works.
pub fn mark_working() {
    if let Ok(mut s) = STATUS.lock()
        && let Some(st) = s.as_mut()
        && st.state != "active"
    {
        st.state = "active".into();
        st.detail = "Watching the app in front".into();
    }
}

/// Start the focus source for this desktop.
pub async fn run(conn: zbus::Connection, state_dir: std::path::PathBuf, tx: mpsc::Sender<DeckMsg>) {
    let on_bus = |name: &'static str| {
        let conn = conn.clone();
        async move {
            match zbus::fdo::DBusProxy::new(&conn).await {
                Ok(d) => match zbus::names::BusName::try_from(name) {
                    Ok(n) => d.name_has_owner(n).await.unwrap_or(false),
                    Err(_) => false,
                },
                Err(_) => false,
            }
        }
    };
    let env = Env::from_process(on_bus("org.kde.KWin").await, on_bus("org.gnome.Shell").await);
    match detect(&env) {
        Backend::Kwin => kwin::run(conn, state_dir).await,
        Backend::Gnome => gnome::run().await,
        Backend::Hyprland { socket } => hyprland::run(socket, tx).await,
        Backend::Sway { socket } => sway::run(socket, tx).await,
        Backend::X11 => x11::run(tx).await,
        Backend::None(why) => {
            tracing::info!("auto profile switching unavailable: {why}");
            set_status("none", "unavailable", &why);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_picks_backend_from_env() {
        let kde = Env {
            current_desktop: "KDE".into(),
            kwin_on_bus: true,
            wayland: true,
            ..Env::default()
        };
        assert_eq!(detect(&kde), Backend::Kwin);
        let hypr = Env {
            hyprland_signature: Some("abc".into()),
            runtime_dir: Some("/run/user/1000".into()),
            wayland: true,
            ..Env::default()
        };
        assert_eq!(
            detect(&hypr),
            Backend::Hyprland {
                socket: "/run/user/1000/hypr/abc/.socket2.sock".into()
            }
        );
        let sway = Env {
            swaysock: Some("/run/user/1000/sway-ipc.sock".into()),
            wayland: true,
            ..Env::default()
        };
        assert!(matches!(detect(&sway), Backend::Sway { .. }));
        let gnome = Env {
            current_desktop: "ubuntu:GNOME".into(),
            wayland: true,
            ..Env::default()
        };
        assert_eq!(detect(&gnome), Backend::Gnome);
        let x = Env {
            current_desktop: "XFCE".into(),
            x11_display: true,
            ..Env::default()
        };
        assert_eq!(detect(&x), Backend::X11);
    }

    #[test]
    fn no_backend_reports_reason() {
        let cosmic = Env {
            current_desktop: "COSMIC".into(),
            wayland: true,
            ..Env::default()
        };
        assert!(matches!(detect(&cosmic), Backend::None(why) if why.contains("COSMIC")));
        let other = Env {
            current_desktop: "river".into(),
            wayland: true,
            x11_display: true,
            ..Env::default()
        };
        assert!(matches!(detect(&other), Backend::None(why) if why.contains("Wayland")));
        assert!(matches!(detect(&Env::default()), Backend::None(_)));
    }
}
