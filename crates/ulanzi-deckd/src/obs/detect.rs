//! Finding OBS: installed, running, and its WebSocket server settings.

use std::path::{Path, PathBuf};

/// OBS's WebSocket server settings, from its own config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsConfig {
    pub enabled: bool,
    pub port: u16,
    pub auth: bool,
    pub password: String,
    pub path: PathBuf,
}

/// Where OBS keeps the WebSocket settings: the native build, then the Flatpak.
pub fn config_paths(home: &Path, xdg_config: Option<&Path>) -> Vec<PathBuf> {
    let rel = "obs-studio/plugin_config/obs-websocket/config.json";
    let native = xdg_config
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"));
    vec![
        native.join(rel),
        home.join(".var/app/com.obsproject.Studio/config").join(rel),
    ]
}

pub fn read_config(path: &Path) -> Option<ObsConfig> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(ObsConfig {
        enabled: v.get("server_enabled").and_then(|x| x.as_bool()).unwrap_or(false),
        port: v
            .get("server_port")
            .and_then(|x| x.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(4455),
        auth: v.get("auth_required").and_then(|x| x.as_bool()).unwrap_or(false),
        password: v
            .get("server_password")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        path: path.to_path_buf(),
    })
}

/// The first config file that exists.
pub fn find_config(paths: &[PathBuf]) -> Option<ObsConfig> {
    paths.iter().find_map(|p| read_config(p))
}

/// An `obs` process is running (native or Flatpak, both run a process named `obs`).
pub fn obs_running(proc_root: &Path) -> bool {
    let Ok(dir) = std::fs::read_dir(proc_root) else {
        return false;
    };
    dir.flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .any(|e| {
            std::fs::read_to_string(e.path().join("comm"))
                .map(|c| c.trim() == "obs")
                .unwrap_or(false)
        })
}

/// OBS is installed: an `obs` binary on the PATH, the Flatpak, or at least its config folder.
pub fn obs_installed(path_var: &str, home: &Path, config_paths: &[PathBuf]) -> bool {
    std::env::split_paths(path_var).any(|d| d.join("obs").is_file())
        || [
            PathBuf::from("/var/lib/flatpak/app/com.obsproject.Studio"),
            home.join(".local/share/flatpak/app/com.obsproject.Studio"),
        ]
        .iter()
        .any(|p| p.is_dir())
        || config_paths
            .iter()
            .any(|p| p.ancestors().nth(3).is_some_and(Path::is_dir))
}

/// Turn the WebSocket server on in OBS's config. Only while OBS is closed: OBS rewrites the file
/// when it quits. Keeps an existing password; makes one when a password is required and none is set.
pub fn enable_server(path: &Path, running: bool) -> Result<ObsConfig, String> {
    if running {
        return Err("Close OBS first, or turn it on in OBS: Tools → WebSocket Server Settings".into());
    }
    let mut v: serde_json::Value = match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("OBS's settings file is damaged: {e}"))?,
        Err(_) => serde_json::json!({"server_port": 4455, "auth_required": true}),
    };
    let Some(obj) = v.as_object_mut() else {
        return Err("OBS's settings file is damaged".into());
    };
    obj.insert("server_enabled".into(), true.into());
    obj.entry("server_port").or_insert(4455.into());
    let auth = obj
        .get("auth_required")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let empty = obj
        .get("server_password")
        .and_then(|x| x.as_str())
        .is_none_or(str::is_empty);
    if auth && empty {
        obj.insert("server_password".into(), new_password().into());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?;
    ulanzi_core::store::write_atomic_mode(path, &text, 0o600).map_err(|e| e.to_string())?;
    read_config(path).ok_or_else(|| "Couldn't read OBS's settings back".into())
}

/// 16 letters and digits from the kernel's random source.
fn new_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_enables_the_server() {
        let dir = tempfile::tempdir().expect("tmp");
        let paths = config_paths(dir.path(), None);
        assert!(find_config(&paths).is_none());
        let p = &paths[1]; // the Flatpak one
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(
            p,
            r#"{"alerts_enabled":false,"auth_required":true,"first_load":false,"server_enabled":false,"server_password":"","server_port":4456}"#,
        )
        .expect("write");
        let c = find_config(&paths).expect("found");
        assert_eq!((c.enabled, c.port, c.auth), (false, 4456, true));
        assert!(enable_server(p, true).is_err(), "never while OBS runs");
        let c = enable_server(p, false).expect("enabled");
        assert!(c.enabled);
        assert_eq!(c.port, 4456);
        assert_eq!(c.password.len(), 16);
        // Other keys survive.
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).expect("read")).expect("json");
        assert_eq!(v["alerts_enabled"], false);
        // A second time keeps the password.
        assert_eq!(enable_server(p, false).expect("again").password, c.password);
    }

    #[test]
    fn finds_the_obs_process() {
        let dir = tempfile::tempdir().expect("tmp");
        let proc = dir.path();
        std::fs::create_dir_all(proc.join("12")).expect("mkdir");
        std::fs::write(proc.join("12/comm"), "bash\n").expect("write");
        assert!(!obs_running(proc));
        std::fs::create_dir_all(proc.join("345")).expect("mkdir");
        std::fs::write(proc.join("345/comm"), "obs\n").expect("write");
        assert!(obs_running(proc));
    }
}
