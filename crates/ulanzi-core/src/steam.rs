//! Installed Steam games and the artwork Steam already keeps locally.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub appid: u32,
    pub name: String,
    /// Local cover art, if Steam downloaded some.
    pub art: Option<PathBuf>,
}

/// Steam installs that are not games.
const NOT_GAMES: [&str; 5] = [
    "Steam Linux Runtime",
    "Proton",
    "Steamworks Common",
    "SteamVR",
    "Steam Runtime",
];

/// The Steam data directory, native or Flatpak.
pub fn default_root() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
    .into_iter()
    .find(|p| p.join("steamapps").is_dir())
}

/// Values of every `"key" "value"` line with this key in a VDF/ACF file.
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let quoted = format!("\"{key}\"");
    text.lines()
        .filter_map(|l| l.trim().strip_prefix(&quoted))
        .filter_map(|rest| {
            let rest = rest.trim();
            rest.strip_prefix('"')?
                .strip_suffix('"')
                .map(|v| v.replace("\\\\", "\\"))
        })
        .collect()
}

/// Cover art Steam cached for `appid`: portrait art first, then the header, then any image.
pub fn art_for(root: &Path, appid: u32) -> Option<PathBuf> {
    let dir = root.join("appcache/librarycache").join(appid.to_string());
    for name in [
        "library_600x900.jpg",
        "library_capsule.jpg",
        "header.jpg",
        "library_header.jpg",
    ] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let flat = root
        .join("appcache/librarycache")
        .join(format!("{appid}_library_600x900.jpg"));
    if flat.is_file() {
        return Some(flat);
    }
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png"))
}

/// Every installed game in every library folder, sorted by name.
pub fn scan(root: &Path) -> Vec<Game> {
    let mut libraries = vec![root.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        for p in vdf_values(&text, "path") {
            let p = PathBuf::from(p);
            if !libraries.contains(&p) {
                libraries.push(p);
            }
        }
    }
    let mut games = Vec::new();
    for lib in libraries {
        let Ok(entries) = std::fs::read_dir(lib.join("steamapps")) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else {
                continue;
            };
            let appid = vdf_values(&text, "appid").first().and_then(|a| a.parse().ok());
            let title = vdf_values(&text, "name").into_iter().next();
            if let (Some(appid), Some(title)) = (appid, title)
                && !NOT_GAMES.iter().any(|n| title.starts_with(n))
                && !games.iter().any(|g: &Game| g.appid == appid)
            {
                games.push(Game {
                    appid,
                    art: art_for(root, appid),
                    name: title,
                });
            }
        }
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    games
}
