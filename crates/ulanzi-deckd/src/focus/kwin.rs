//! KDE Plasma: a small KWin script reports every window switch to our D-Bus `WindowActivated`
//! method; it is loaded when the service starts and again whenever KWin restarts.

use std::path::Path;

use futures_util::StreamExt;

use crate::dbus::{BUS_NAME, INTERFACE, OBJECT_PATH};

const PLUGIN: &str = "ulanzi-deck-focus";
const KWIN: &str = "org.kde.KWin";

/// The KWin script: forward the active window's class, desktop file and title.
pub fn kwin_script() -> String {
    format!(
        r#"// Ulanzi Deck: tell the deck service which app is in front (for auto profile switching).
function report(w) {{
    if (!w) return;
    callDBus("{BUS_NAME}", "{OBJECT_PATH}", "{INTERFACE}", "WindowActivated",
             String(w.resourceClass || ""), String(w.desktopFileName || ""), String(w.caption || ""));
}}
if (workspace.windowActivated) {{
    workspace.windowActivated.connect(report);
    report(workspace.activeWindow);
}} else {{
    workspace.clientActivated.connect(report);
    report(workspace.activeClient);
}}
"#
    )
}

async fn load(conn: &zbus::Connection, script: &Path) -> zbus::Result<()> {
    let kwin = zbus::Proxy::new(conn, KWIN, "/Scripting", "org.kde.kwin.Scripting").await?;
    let loaded: bool = kwin.call("isScriptLoaded", &(PLUGIN,)).await?;
    if loaded {
        let _: bool = kwin.call("unloadScript", &(PLUGIN,)).await?;
    }
    let path = script.to_string_lossy().into_owned();
    let id: i32 = kwin.call("loadScript", &(path.as_str(), PLUGIN)).await?;
    if id < 0 {
        return Err(zbus::Error::Failure("KWin refused the script".into()));
    }
    kwin.call::<_, _, ()>("start", &()).await
}

/// Remove the script (on shutdown).
pub async fn unload(conn: &zbus::Connection) {
    if let Ok(kwin) = zbus::Proxy::new(conn, KWIN, "/Scripting", "org.kde.kwin.Scripting").await {
        let _: zbus::Result<bool> = kwin.call("unloadScript", &(PLUGIN,)).await;
    }
}

/// Keep the focus reporter installed for as long as the service runs.
pub async fn run(conn: zbus::Connection, state_dir: std::path::PathBuf) {
    let script = state_dir.join("kwin-focus.js");
    if let Err(e) = std::fs::create_dir_all(&state_dir).and_then(|_| std::fs::write(&script, kwin_script())) {
        tracing::warn!("auto profile switching: cannot write the KWin script: {e}");
        return;
    }
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await else {
        return;
    };
    let kwin_running = dbus
        .name_has_owner(KWIN.try_into().expect("valid name"))
        .await
        .unwrap_or(false);
    if !kwin_running {
        tracing::info!("auto profile switching works on KDE Plasma for now (KWin not found)");
    } else {
        match load(&conn, &script).await {
            Ok(()) => {
                tracing::info!("auto profile switching: watching the app in front (KWin)");
                super::set_status("kwin", "waiting", "Waiting for the first window switch");
            }
            Err(e) => {
                tracing::warn!("auto profile switching: KWin script not loaded: {e}");
                super::set_status(
                    "kwin",
                    "unavailable",
                    &format!("KWin didn't accept the script: {e}"),
                );
            }
        }
    }
    // KWin restarted (or started late): load the script again.
    let Ok(mut changes) = dbus.receive_name_owner_changed_with_args(&[(0, KWIN)]).await else {
        return;
    };
    while let Some(sig) = changes.next().await {
        let Ok(args) = sig.args() else { continue };
        if args.new_owner().is_some()
            && let Err(e) = load(&conn, &script).await
        {
            tracing::warn!("auto profile switching: KWin script not reloaded: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_calls_our_service() {
        let s = kwin_script();
        assert!(s.contains("\"io.github.tyaaa_aa.UlanziDeck.Service\""));
        assert!(s.contains("\"WindowActivated\""));
        assert!(s.contains("workspace.windowActivated.connect(report)"));
    }
}
