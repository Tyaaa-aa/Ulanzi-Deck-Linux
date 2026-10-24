use serde::{Deserialize, Serialize};

use super::{Field, FieldKind};

/// The plugin contract this host implements (major, minor).
pub const CONTRACT: (u32, u32) = (1, 1);

/// A plugin's `plugin.toml`. Tables the host does not read yet (faces, widgets, variables,
/// detect, auth) are accepted and ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Settings the GUI renders as the plugin's settings page.
    #[serde(default)]
    pub settings: Vec<Field>,
    /// Lists the user picks `item` fields from (contract 1.1); the plugin fills them (`list-items`).
    #[serde(default)]
    pub pickers: Vec<Picker>,
}

/// How the GUI lays out a picker's list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickerStyle {
    #[default]
    List,
    Grid,
}

/// A `[[pickers]]` entry: a list an `item` field (`picker = "<id>"`) is picked from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picker {
    pub id: String,
    #[serde(default)]
    pub style: PickerStyle,
    /// The entries carry groups; the GUI offers a group filter.
    #[serde(default)]
    pub groups: bool,
    /// Several entries can be picked; the setting stores a list of ids. Settings only.
    #[serde(default)]
    pub multi: bool,
}

/// One entry of a picker list as the plugin answered `list-items` (bounded by the host).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickItem {
    /// What the setting or action param stores.
    pub id: String,
    /// What the user sees.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Live state, shown dimmed next to the entry (for example "on").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// What a plugin may reach outside its sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestAction {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub params: Vec<Field>,
}

/// What the daemon returns when asked about a plugin folder before installing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inspected {
    pub manifest: Manifest,
    pub installed_version: Option<String>,
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
    #[error("\"{key}\" is declared twice in {scope}")]
    DuplicateField { scope: String, key: String },
    #[error("action \"{action}\": \"{key}\" cannot be a secret; secrets belong in [[settings]]")]
    SecretParam { action: String, key: String },
    #[error("a field in {scope} has an empty key")]
    EmptyFieldKey { scope: String },
    #[error("picker \"{0}\" is declared twice")]
    DuplicatePicker(String),
    #[error("\"{key}\" in {scope} uses picker \"{picker}\", which is not declared in [[pickers]]")]
    UnknownPicker {
        scope: String,
        key: String,
        picker: String,
    },
    #[error("action \"{action}\": \"{key}\" cannot use a picker with multi = true; only settings can")]
    MultiParam { action: String, key: String },
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
        for entry in &m.permissions.network {
            if let Some(why) = bad_network_entry(entry) {
                return Err(ManifestError::Parse(format!("{entry}: {why}")));
            }
        }
        for (i, a) in m.actions.iter().enumerate() {
            if m.actions[..i].iter().any(|b| b.id == a.id) {
                return Err(ManifestError::DuplicateAction(a.id.clone()));
            }
        }
        for (i, p) in m.pickers.iter().enumerate() {
            if m.pickers[..i].iter().any(|q| q.id == p.id) {
                return Err(ManifestError::DuplicatePicker(p.id.clone()));
            }
        }
        check_fields("the settings".into(), &m.settings)?;
        m.check_pickers("the settings".into(), &m.settings)?;
        for a in &m.actions {
            let scope = format!("action \"{}\"", a.id);
            check_fields(scope.clone(), &a.params)?;
            if let Some(f) = a.params.iter().find(|f| f.kind == FieldKind::Secret) {
                return Err(ManifestError::SecretParam {
                    action: a.id.clone(),
                    key: f.key.clone(),
                });
            }
            m.check_pickers(scope, &a.params)?;
            let multi = |f: &&Field| {
                f.picker
                    .as_deref()
                    .and_then(|p| m.picker(p))
                    .is_some_and(|p| p.multi)
            };
            if let Some(f) = a.params.iter().filter(|f| f.kind == FieldKind::Item).find(multi) {
                return Err(ManifestError::MultiParam {
                    action: a.id.clone(),
                    key: f.key.clone(),
                });
            }
        }
        Ok(m)
    }

    /// Every `item` field in `fields` that names a picker names a declared one. An `item` field
    /// without a picker is a plain text field (contract 1.0).
    fn check_pickers(&self, scope: String, fields: &[Field]) -> Result<(), ManifestError> {
        for f in fields.iter().filter(|f| f.kind == FieldKind::Item) {
            if let Some(picker) = &f.picker
                && self.picker(picker).is_none()
            {
                return Err(ManifestError::UnknownPicker {
                    scope,
                    key: f.key.clone(),
                    picker: picker.clone(),
                });
            }
        }
        Ok(())
    }

    /// Keys of the settings declared as secrets.
    pub fn secret_keys(&self) -> Vec<&str> {
        self.settings
            .iter()
            .filter(|f| f.kind == FieldKind::Secret)
            .map(|f| f.key.as_str())
            .collect()
    }

    /// `table` with every setting the user never set filled in from its manifest `default`, so
    /// the plugin sees what the settings page shows. Secrets have no defaults and are skipped.
    pub fn settings_with_defaults(&self, table: &toml::Table) -> toml::Table {
        let mut out = table.clone();
        for f in &self.settings {
            if f.kind == FieldKind::Secret {
                continue;
            }
            if let Some(d) = &f.default {
                out.entry(f.key.clone()).or_insert_with(|| d.clone());
            }
        }
        out
    }

    pub fn action(&self, id: &str) -> Option<&ManifestAction> {
        self.actions.iter().find(|a| a.id == id)
    }

    /// The declared `[[pickers]]` entry `id`.
    pub fn picker(&self, id: &str) -> Option<&Picker> {
        self.pickers.iter().find(|p| p.id == id)
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

/// Why a `network` entry cannot match anything useful (empty, or a wildcard that is too wide).
fn bad_network_entry(entry: &str) -> Option<&'static str> {
    let e = entry.strip_suffix('.').unwrap_or(entry);
    if e.is_empty() {
        return Some("empty host name");
    }
    if let Some(suffix) = e.strip_prefix("*.") {
        if suffix.split('.').filter(|l| !l.is_empty()).count() < 2 {
            return Some("a wildcard needs at least two labels after \"*.\"");
        }
    } else if e.contains('*') {
        return Some("\"*\" is only allowed as a leading \"*.\"");
    }
    None
}

fn check_fields(scope: String, fields: &[Field]) -> Result<(), ManifestError> {
    for (i, f) in fields.iter().enumerate() {
        if f.key.trim().is_empty() {
            return Err(ManifestError::EmptyFieldKey { scope });
        }
        if fields[..i].iter().any(|g| g.key == f.key) {
            return Err(ManifestError::DuplicateField {
                scope,
                key: f.key.clone(),
            });
        }
    }
    Ok(())
}
