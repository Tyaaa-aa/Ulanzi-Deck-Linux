use serde::Deserialize;

/// The plugin contract this host implements (major, minor).
pub const CONTRACT: (u32, u32) = (1, 0);

/// A plugin's `plugin.toml`. Tables the host does not read yet (settings, pickers, faces, widgets,
/// variables, detect, auth) are accepted and ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub contract: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tested_on_hardware: bool,
    /// Start with the daemon even when no profile uses the plugin (alerts, background work).
    #[serde(default)]
    pub background: bool,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub actions: Vec<ManifestAction>,
}

/// What a plugin may reach outside its sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Permissions {
    /// Host names the plugin may connect to (`*.example.com` matches subdomains).
    pub network: Vec<String>,
    /// Devices on the local network.
    pub lan: bool,
    pub browser: bool,
    pub dbus: Vec<String>,
    pub programs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ManifestAction {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// Parameter descriptions, kept as written until the editor reads them.
    #[serde(default)]
    pub params: Vec<toml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("plugin.toml is not valid: {0}")]
    Parse(String),
    #[error("\"{0}\" is not a valid plugin id (a-z, 0-9 and _, starting with a letter, 2 to 32 characters)")]
    BadId(String),
    #[error("\"{0}\" is not a valid version (expected MAJOR.MINOR.PATCH)")]
    BadVersion(String),
    #[error("needs plugin contract {needs}")]
    Incompatible { needs: String },
    #[error("action \"{0}\" is declared twice")]
    DuplicateAction(String),
}

impl Manifest {
    /// Parse and validate: id, version, contract compatibility and unique action ids.
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let m: Manifest = toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))?;
        if !valid_id(&m.id) {
            return Err(ManifestError::BadId(m.id));
        }
        if !valid_semver(&m.version) {
            return Err(ManifestError::BadVersion(m.version));
        }
        if !contract_ok(&m.contract) {
            return Err(ManifestError::Incompatible { needs: m.contract });
        }
        for (i, a) in m.actions.iter().enumerate() {
            if m.actions[..i].iter().any(|b| b.id == a.id) {
                return Err(ManifestError::DuplicateAction(a.id.clone()));
            }
        }
        Ok(m)
    }

    /// Does the plugin ask to talk to the network at all?
    pub fn has_network(&self) -> bool {
        !self.permissions.network.is_empty() || self.permissions.lan
    }
}

/// `^[a-z][a-z0-9_]{1,31}$`
fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (2..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// `MAJOR.MINOR.PATCH`, digits only.
fn valid_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
}

/// `MAJOR.MINOR` with the same major as the host and a minor the host has.
fn contract_ok(c: &str) -> bool {
    let Some((major, minor)) = c.split_once('.') else {
        return false;
    };
    match (major.parse::<u32>(), minor.parse::<u32>()) {
        (Ok(major), Ok(minor)) => major == CONTRACT.0 && minor <= CONTRACT.1,
        _ => false,
    }
}
