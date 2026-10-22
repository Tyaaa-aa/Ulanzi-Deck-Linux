//! Which profile belongs in front: match the focused window against each profile's apps.

use uuid::Uuid;

use crate::apps::AppEntry;

/// The focused window as the desktop reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    /// Window class / Wayland app id, e.g. `firefox`, `steam_app_1145360`.
    pub class: String,
    /// The app's desktop file name without `.desktop`, when the desktop knows it.
    pub desktop_id: String,
    pub title: String,
}

/// One profile's apps, each as the lowercase names its windows may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub profile: Uuid,
    pub apps: Vec<Vec<String>>,
    pub priority: i32,
}

/// Windows that never switch profiles: the desktop itself, launchers, and this app.
const IGNORED: &[&str] = &[
    "",
    "plasmashell",
    "org.kde.plasmashell",
    "krunner",
    "org.kde.krunner",
    "kwin",
    "kwin_wayland",
    "org.kde.kwin",
    "gnome-shell",
    "org.gnome.shell",
    "xdg-desktop-portal-kde",
    "ulanzi-deck",
    "io.github.tyaaa_aa.ulanzideck",
];

/// The names a window of `entry` may carry. `entry` is a desktop id (`firefox.desktop`) or a
/// Steam game (`steam:<appid>`); `app` adds its window class and program name when known.
pub fn aliases(entry: &str, app: Option<&AppEntry>) -> Vec<String> {
    if let Some(id) = entry.strip_prefix("steam:") {
        return vec![format!("steam_app_{id}")];
    }
    let stem = entry.trim_end_matches(".desktop").to_lowercase();
    let mut out = vec![stem.clone()];
    // org.kde.spectacle → also "spectacle" (Qt apps often report the short name).
    if let Some(last) = stem.rsplit('.').next()
        && last != stem
    {
        out.push(last.to_string());
    }
    if let Some(app) = app {
        if let Some(c) = &app.wm_class {
            out.push(c.to_lowercase());
        }
        // A Steam shortcut starts the game through Steam; the game's window says steam_app_<id>.
        if let Some(id) = app
            .exec
            .iter()
            .find_map(|a| a.split("rungameid/").nth(1))
            .map(|id| id.trim_end_matches('/'))
        {
            out.push(format!("steam_app_{id}"));
        } else if let Some(prog) = app.exec.first().and_then(|e| e.rsplit('/').next()) {
            out.push(prog.to_lowercase());
        }
    }
    out.dedup();
    out
}

/// The profile for `w`: the highest-priority profile with a matching app, else `fallback`.
/// `None` when the window should not cause a switch (the desktop shell, this app).
pub fn pick_profile(rules: &[Rule], w: &Window, fallback: Uuid) -> Option<Uuid> {
    let class = w.class.to_lowercase();
    let desktop = w.desktop_id.trim_end_matches(".desktop").to_lowercase();
    if IGNORED.contains(&class.as_str()) && IGNORED.contains(&desktop.as_str()) {
        return None;
    }
    let hit = |names: &Vec<String>| {
        names
            .iter()
            .any(|n| (!class.is_empty() && *n == class) || (!desktop.is_empty() && *n == desktop))
    };
    let best = rules
        .iter()
        .enumerate()
        .filter(|(_, r)| r.apps.iter().any(hit))
        // Highest priority; on a tie the earlier profile in the list.
        .max_by_key(|(i, r)| (r.priority, std::cmp::Reverse(*i)));
    Some(best.map(|(_, r)| r.profile).unwrap_or(fallback))
}
