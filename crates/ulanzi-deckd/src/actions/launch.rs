//! Launching apps from their `.desktop` files.

use std::path::{Path, PathBuf};

/// The `Exec=` line of the `[Desktop Entry]` group as argv, with field codes (`%u`, `%F`, ...) removed.
pub fn exec_from_desktop(contents: &str) -> Option<Vec<String>> {
    let mut in_entry = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry && let Some(exec) = line.strip_prefix("Exec=") {
            let argv: Vec<String> = split_exec(exec)
                .into_iter()
                .filter(|t| !matches!(t.as_str(), "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k"))
                .map(|t| t.replace("%%", "%"))
                .collect();
            return (!argv.is_empty()).then_some(argv);
        }
    }
    None
}

/// Split per the Desktop Entry spec: whitespace-separated, double quotes group, backslash escapes inside quotes.
fn split_exec(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has_token = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                has_token = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

/// XDG application directories, highest priority first (including Flatpak exports).
pub fn application_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs = vec![data_home.clone(), data_home.join("flatpak/exports/share")];
    dirs.extend(data_dirs.split(':').map(PathBuf::from));
    dirs.push("/var/lib/flatpak/exports/share".into());
    dirs.into_iter().map(|d| d.join("applications")).collect()
}

/// Find `<id>` (with or without `.desktop`) in the given application directories.
pub fn find_desktop(id: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = if id.ends_with(".desktop") {
        id.to_string()
    } else {
        format!("{id}.desktop")
    };
    dirs.iter()
        .map(|d| d.join(&file))
        .find(|p| Path::new(p).is_file())
}
