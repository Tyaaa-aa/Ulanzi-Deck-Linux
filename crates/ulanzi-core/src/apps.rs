//! Installed applications (XDG `.desktop` files) and their icons, for the Launch App action and the
//! icon picker's Apps tab.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// File name, e.g. `firefox.desktop`.
    pub desktop_id: String,
    pub name: String,
    /// Icon name or absolute path from `Icon=`.
    pub icon: Option<String>,
    /// `Exec=` split into argv, field codes removed.
    pub exec: Vec<String>,
    pub keywords: Vec<String>,
}

thread_local! {
    static DATA_DIRS_OVERRIDE: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

/// Run `f` with [`data_dirs`] returning `dirs` on this thread (for tests).
pub fn with_data_dirs_override(dirs: Vec<PathBuf>, f: impl FnOnce()) {
    DATA_DIRS_OVERRIDE.with(|o| *o.borrow_mut() = Some(dirs));
    f();
    DATA_DIRS_OVERRIDE.with(|o| *o.borrow_mut() = None);
}

/// XDG data directories, highest priority first, including Flatpak exports.
pub fn data_dirs() -> Vec<PathBuf> {
    if let Some(dirs) = DATA_DIRS_OVERRIDE.with(|o| o.borrow().clone()) {
        return dirs;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let rest = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs = vec![data_home.clone(), data_home.join("flatpak/exports/share")];
    dirs.extend(rest.split(':').map(PathBuf::from));
    dirs.push("/var/lib/flatpak/exports/share".into());
    dirs.push("/var/lib/snapd/desktop".into());
    let mut seen = Vec::new();
    dirs.retain(|d| {
        let fresh = !seen.contains(d);
        seen.push(d.clone());
        fresh
    });
    dirs
}

pub fn application_dirs() -> Vec<PathBuf> {
    data_dirs().into_iter().map(|d| d.join("applications")).collect()
}

pub fn icon_theme_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut dirs = vec![home.join(".icons")];
    dirs.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    dirs
}

/// `Key=value` pairs of the `[Desktop Entry]` group (unlocalised keys only).
fn desktop_entry(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && !k.contains('[')
        {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

/// Split an `Exec=` value per the Desktop Entry spec and drop field codes (`%u`, `%F`, ...).
pub fn split_exec(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut has) = (false, false);
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out.into_iter()
        .filter(|t| !matches!(t.as_str(), "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k"))
        .map(|t| t.replace("%%", "%"))
        .collect()
}

fn parse(desktop_id: &str, text: &str) -> Option<AppEntry> {
    let e = desktop_entry(text);
    let yes = |k: &str| e.get(k).is_some_and(|v| v == "true");
    if yes("NoDisplay") || yes("Hidden") || e.get("Type").is_some_and(|t| t != "Application") {
        return None;
    }
    let exec = split_exec(e.get("Exec")?);
    if exec.is_empty() {
        return None;
    }
    Some(AppEntry {
        desktop_id: desktop_id.to_string(),
        name: e
            .get("Name")
            .cloned()
            .unwrap_or_else(|| desktop_id.trim_end_matches(".desktop").into()),
        icon: e.get("Icon").filter(|i| !i.is_empty()).cloned(),
        exec,
        keywords: e
            .get("Keywords")
            .map(|k| {
                k.split(';')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Every visible application in `dirs` (earlier dirs win for the same id), sorted by name.
pub fn scan(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut by_id: BTreeMap<String, Option<AppEntry>> = BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.flatten() {
            let id = e.file_name().to_string_lossy().into_owned();
            if !id.ends_with(".desktop") || by_id.contains_key(&id) {
                continue;
            }
            let parsed = std::fs::read_to_string(e.path())
                .ok()
                .and_then(|t| parse(&id, &t));
            by_id.insert(id, parsed);
        }
    }
    let mut apps: Vec<AppEntry> = by_id.into_values().flatten().collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The application with this desktop id, if installed.
pub fn lookup(desktop_id: &str) -> Option<AppEntry> {
    let id = if desktop_id.ends_with(".desktop") {
        desktop_id.to_string()
    } else {
        format!("{desktop_id}.desktop")
    };
    application_dirs().iter().find_map(|d| {
        std::fs::read_to_string(d.join(&id))
            .ok()
            .and_then(|t| parse(&id, &t))
    })
}

/// The user's icon theme(s): KDE's `kdeglobals`, else GNOME's setting, then common fallbacks.
pub fn current_icon_themes() -> Vec<String> {
    let mut themes = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("kdeglobals")) {
        let mut in_icons = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_icons = line == "[Icons]";
            } else if in_icons && let Some(t) = line.strip_prefix("Theme=") {
                themes.push(t.trim().to_string());
            }
        }
    }
    if themes.is_empty()
        && let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "icon-theme"])
            .output()
    {
        let t = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_matches('\'')
            .to_string();
        if !t.is_empty() {
            themes.push(t);
        }
    }
    for t in ["breeze", "Adwaita", "Papirus"] {
        if !themes.iter().any(|x| x == t) {
            themes.push(t.into());
        }
    }
    themes
}

fn size_rank(dir_name: &str) -> u32 {
    if dir_name == "scalable" || dir_name.starts_with("scalable") {
        return 10_000;
    }
    dir_name
        .split(['x', '@'])
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Find an icon file by name: absolute paths as they are; otherwise the largest match in each theme
/// (scalable first), then `hicolor`, then `pixmaps`.
pub fn find_icon(name: &str, roots: &[PathBuf], themes: &[&str]) -> Option<PathBuf> {
    if Path::new(name).is_absolute() {
        return Some(PathBuf::from(name));
    }
    let exts = ["svg", "png", "xpm"];
    let mut order: Vec<&str> = themes.to_vec();
    order.push("hicolor");
    for theme in order {
        let mut best: Option<(u32, PathBuf)> = None;
        for root in roots {
            let Ok(sizes) = std::fs::read_dir(root.join(theme)) else {
                continue;
            };
            for size in sizes.flatten() {
                let rank = size_rank(&size.file_name().to_string_lossy());
                for ctx in [
                    "apps",
                    "places",
                    "devices",
                    "categories",
                    "mimetypes",
                    "status",
                    "actions",
                ] {
                    for ext in exts {
                        let p = size.path().join(ctx).join(format!("{name}.{ext}"));
                        if p.is_file() && best.as_ref().is_none_or(|(r, _)| rank > *r) {
                            best = Some((rank, p));
                        }
                    }
                }
            }
        }
        if let Some((_, p)) = best {
            return Some(p);
        }
    }
    let pixmaps = roots.iter().filter_map(|r| r.parent().map(|p| p.join("pixmaps")));
    for dir in pixmaps.chain(std::iter::once(PathBuf::from("/usr/share/pixmaps"))) {
        for ext in exts {
            let p = dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Icon file for an installed application.
pub fn app_icon(desktop_id: &str) -> Option<PathBuf> {
    let icon = lookup(desktop_id)?.icon?;
    let themes = current_icon_themes();
    let refs: Vec<&str> = themes.iter().map(String::as_str).collect();
    find_icon(&icon, &icon_theme_dirs(), &refs)
}
