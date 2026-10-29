//! On-disk layout and atomic persistence.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::CoreError;
use crate::model::{FORMAT_VERSION, Profile, Settings};

pub const PROFILE_FILE: &str = "profile.toml";
pub const SETTINGS_FILE: &str = "settings.toml";

/// Config, state and cache roots for the app.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    state: PathBuf,
    cache: PathBuf,
}

/// Result of reading one profile directory.
#[derive(Debug, Clone)]
pub enum ProfileLoad {
    Ok(Box<Profile>),
    /// The file did not parse; it was renamed to `path` so the user can fix it.
    Invalid {
        path: PathBuf,
        error: String,
    },
}

fn xdg(var: &str, fallback: &str) -> Result<PathBuf, CoreError> {
    if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(v));
    }
    let home = std::env::var_os("HOME").ok_or(CoreError::NoHome)?;
    Ok(PathBuf::from(home).join(fallback))
}

/// Where installed plugins and other app data live: `~/.local/share/ulanzi-deck`.
pub fn data_dir() -> Result<PathBuf, CoreError> {
    Ok(xdg("XDG_DATA_HOME", ".local/share")?.join("ulanzi-deck"))
}

/// Where plugins are installed: `<data folder>/plugins`.
pub fn plugins_dir() -> Result<PathBuf, CoreError> {
    Ok(data_dir()?.join("plugins"))
}

/// The file `icon` (an `assets/` path from the plugin's manifest) of installed plugin `plugin`;
/// `None` when the names could leave the plugin's folder.
pub fn plugin_asset(plugin: &str, icon: &str) -> Option<PathBuf> {
    let safe = |s: &str| {
        !s.is_empty() && !s.contains('\\') && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
    };
    if !safe(plugin) || plugin.contains('/') || !safe(icon) || !icon.starts_with("assets/") {
        return None;
    }
    Some(plugins_dir().ok()?.join(plugin).join(icon))
}

impl Store {
    /// Standard XDG locations: `~/.config/ulanzi-deck`, `~/.local/state/ulanzi-deck`, `~/.cache/ulanzi-deck`.
    pub fn xdg() -> Result<Store, CoreError> {
        Ok(Store {
            root: xdg("XDG_CONFIG_HOME", ".config")?.join("ulanzi-deck"),
            state: xdg("XDG_STATE_HOME", ".local/state")?.join("ulanzi-deck"),
            cache: xdg("XDG_CACHE_HOME", ".cache")?.join("ulanzi-deck"),
        })
    }

    pub fn at(root: impl Into<PathBuf>, state: impl Into<PathBuf>, cache: impl Into<PathBuf>) -> Store {
        Store {
            root: root.into(),
            state: state.into(),
            cache: cache.into(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }

    pub fn profile_dir(&self, id: Uuid) -> PathBuf {
        self.profiles_dir().join(id.to_string())
    }

    /// Missing file means defaults.
    pub fn load_settings(&self) -> Result<Settings, CoreError> {
        let path = self.root.join(SETTINGS_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => parse_versioned(&path, &text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(CoreError::io(&path, e)),
        }
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), CoreError> {
        let path = self.root.join(SETTINGS_FILE);
        // Private: it can hold the OBS password.
        write_atomic_mode(&path, to_toml(s)?.as_bytes(), 0o600).map_err(|e| CoreError::io(&path, e))
    }

    pub fn load_profile(&self, id: Uuid) -> Result<Profile, CoreError> {
        self.load_profile_at(&self.profile_dir(id).join(PROFILE_FILE))
    }

    fn load_profile_at(&self, path: &Path) -> Result<Profile, CoreError> {
        let text = fs::read_to_string(path).map_err(|e| CoreError::io(path, e))?;
        let mut p: Profile = parse_versioned(path, &text)?;
        for w in p.normalize() {
            tracing::warn!("{}: {w}", path.display());
        }
        Ok(p)
    }

    pub fn save_profile(&self, p: &Profile) -> Result<(), CoreError> {
        let path = self.profile_dir(p.id).join(PROFILE_FILE);
        write_atomic(&path, to_toml(p)?.as_bytes()).map_err(|e| CoreError::io(&path, e))
    }

    /// Move a profile's directory aside to `profiles/.deleted-<id>-<unix-ts>` (recoverable by hand).
    pub fn delete_profile(&self, id: Uuid) -> Result<PathBuf, CoreError> {
        let from = self.profile_dir(id);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let to = self.profiles_dir().join(format!(".deleted-{id}-{ts}"));
        fs::rename(&from, &to).map_err(|e| CoreError::io(&from, e))?;
        Ok(to)
    }

    /// Every profile directory, valid or not. Invalid files are renamed to
    /// `profile.toml.invalid-<unix-ts>` and reported, never fatal.
    pub fn list_profiles(&self) -> Vec<ProfileLoad> {
        let Ok(entries) = fs::read_dir(self.profiles_dir()) else {
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        let mut out = Vec::new();
        for dir in dirs {
            let path = dir.join(PROFILE_FILE);
            if !path.exists() {
                continue;
            }
            match self.load_profile_at(&path) {
                Ok(p) => out.push(ProfileLoad::Ok(Box::new(p))),
                Err(err) => {
                    let ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let aside = dir.join(format!("{PROFILE_FILE}.invalid-{ts}"));
                    let path = match fs::rename(&path, &aside) {
                        Ok(()) => aside,
                        Err(e) => {
                            tracing::error!("could not move invalid profile aside: {e}");
                            path
                        }
                    };
                    tracing::error!("{}: {err}", path.display());
                    out.push(ProfileLoad::Invalid {
                        path,
                        error: err.to_string(),
                    });
                }
            }
        }
        out
    }
}

/// Parse and normalize a profile received as text (e.g. over D-Bus).
pub fn parse_profile(text: &str) -> Result<Profile, CoreError> {
    let mut p: Profile = parse_versioned(Path::new("<profile>"), text)?;
    p.normalize();
    Ok(p)
}

fn to_toml<T: Serialize>(v: &T) -> Result<String, CoreError> {
    toml::to_string_pretty(v).map_err(|e| CoreError::Serialize(e.to_string()))
}

/// Parse a TOML file after checking its `version` field.
fn parse_versioned<T: DeserializeOwned>(path: &Path, text: &str) -> Result<T, CoreError> {
    let parse_err = |e: toml::de::Error| CoreError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    };
    let table: toml::Table = toml::from_str(text).map_err(parse_err)?;
    let found = table
        .get("version")
        .and_then(|v| v.as_integer())
        .unwrap_or(FORMAT_VERSION as i64);
    if found != FORMAT_VERSION as i64 {
        return Err(CoreError::UnsupportedVersion {
            path: path.to_path_buf(),
            found,
        });
    }
    toml::from_str(text).map_err(parse_err)
}

/// Write via a temp file in the same directory, fsync, then rename over the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_mode(path, bytes, 0o644)
}

/// [`write_atomic`] creating the file with permission bits `mode`.
pub fn write_atomic_mode(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
