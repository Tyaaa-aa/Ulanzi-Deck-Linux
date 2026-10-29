use serde::{Deserialize, Serialize};

use super::{Field, FieldKind};

/// The plugin contract this host implements (major, minor).
pub const CONTRACT: (u32, u32) = (1, 2);

/// Contract 1.2 fields: plugins built for an older contract keep their promise, so these are
/// accepted and ignored there (like `[[pickers]]` before 1.1).
const TOP_LEVEL_1_2: [&str; 3] = ["symbolic_icon", "color", "key_sets"];
const ACTION_1_2: [&str; 3] = ["states", "labels", "track"];

/// A plugin's `plugin.toml`. Tables the host does not read yet (faces, widgets, variables,
/// detect, auth) are accepted and ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub contract: String,
    /// The oldest Ulanzi Deck that can run the plugin (`MAJOR.MINOR.PATCH`); newer apps only.
    #[serde(default)]
    pub min_app: Option<String>,
    /// The plugin's full-colour icon: an `assets/` file (`.svg` or `.png`), shown in lists, the
    /// plugin's settings and the plugin store. A plain name (`mdi:lightbulb`) is looked up in
    /// the icon theme instead.
    #[serde(default)]
    pub icon: Option<String>,
    /// A one-colour version of the icon (`assets/….svg`) that the app tints to fit: small places
    /// such as section headers.
    #[serde(default)]
    pub symbolic_icon: Option<String>,
    /// The plugin's colour (`#rrggbb`): its action icons are drawn in it on fresh keys.
    #[serde(default)]
    pub color: Option<String>,
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
    /// Ready-made keys for items the user picks (contract 1.1).
    #[serde(default)]
    pub key_sets: Vec<KeySet>,
}

/// Ready-made keys: the user picks items from `picker` and each item gets the keys of the first
/// variant whose `groups` hold the item's group (a variant without groups takes any item).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySet {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub picker: String,
    /// Settings each picked item is added to, so its live state is published.
    #[serde(default)]
    pub track: Vec<Track>,
    pub variants: Vec<KeySetVariant>,
}

/// Add `value` (with `$id` and `$name` filled in) to setting `setting`: an item to a multi
/// `item` setting, a comma-separated entry to a `text` setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub setting: String,
    pub value: String,
    /// Only for items of these groups (empty: every item).
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySetVariant {
    #[serde(default)]
    pub groups: Vec<String>,
    pub keys: Vec<KeySetKey>,
}

/// One key of a variant. In `params`, `label` and `hold_params`, `$id` becomes the picked item's
/// id and `$name` its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySetKey {
    pub action: String,
    #[serde(default)]
    pub params: toml::Table,
    /// A one-colour `assets/….svg` drawn on the key; the action's icon when missing.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub label: String,
    /// The icon's colour (`#rrggbb`); the plugin's when missing.
    #[serde(default)]
    pub color: Option<String>,
    /// More control than the main key (brightness, speed); the user can leave these out.
    #[serde(default)]
    pub extra: bool,
    /// Looks for live states; the action's own when missing.
    #[serde(default)]
    pub states: Vec<ManifestState>,
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
    /// A one-colour `assets/….svg` the app tints: the action in lists and on fresh keys.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub params: Vec<Field>,
    /// How a key with this action looks in live states (first match wins).
    #[serde(default)]
    pub states: Vec<ManifestState>,
    /// Live values a key with this action can show as its label.
    #[serde(default)]
    pub labels: Vec<ManifestLabel>,
    /// Settings to add the action's parameters to when a key gets it, so its state is
    /// published (`value` may use `$<param>`).
    #[serde(default)]
    pub track: Vec<Track>,
}

impl ManifestAction {
    /// The parameters a newly added key with this action stores (see [`Field::initial_value`]).
    pub fn defaults(&self) -> toml::Table {
        self.params
            .iter()
            .filter_map(|p| p.initial_value().map(|d| (p.key.clone(), d)))
            .collect()
    }
}

/// A look for a live state. `when.value` and `label` may use `$<param>` for the key's action
/// parameters; `icon` is a one-colour `assets/….svg`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestState {
    pub when: crate::model::When,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

/// A live value a key can show: its name in the app and the label text (`$<param>` allowed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestLabel {
    pub name: String,
    pub text: String,
    /// Settings to add to so the value is published (`$<param>` allowed).
    #[serde(default)]
    pub track: Vec<Track>,
}

impl ManifestState {
    /// As a key's look state, with the plugin's icon file.
    pub fn to_look(&self, plugin: &str) -> crate::model::LookState {
        let color = |c: &Option<String>| {
            c.as_deref()
                .and_then(parse_color)
                .map(|[r, g, b]| crate::Color([r, g, b, 255]))
        };
        crate::model::LookState {
            when: self.when.clone(),
            look: crate::model::LookOverride {
                source: self.icon.clone().filter(|i| is_asset(i)).map(|icon| {
                    crate::model::ImageSource::Plugin {
                        plugin: plugin.to_string(),
                        icon,
                    }
                }),
                foreground: color(&self.color),
                background: color(&self.background),
                label: self.label.clone(),
            },
        }
    }
}

/// What the daemon returns when asked about a plugin folder before installing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inspected {
    pub manifest: Manifest,
    pub installed_version: Option<String>,
    /// Signed with a key the app trusts (installs without Developer mode).
    #[serde(default)]
    pub verified: bool,
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
    #[error("needs Ulanzi Deck {0} or newer")]
    NeedsApp(String),
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
    #[error("{scope}: icon \"{icon}\" {why}")]
    BadIcon {
        scope: String,
        icon: String,
        why: &'static str,
    },
    #[error("color \"{0}\" is not a #rrggbb colour")]
    BadColor(String),
    #[error("key set \"{set}\": {why}")]
    BadKeySet { set: String, why: String },
}

impl Manifest {
    /// Parse and validate: id, version, contract compatibility and unique action ids.
    ///
    /// Pickers are contract 1.1: a `contract = "1.0"` manifest keeps the 1.0 promise, so its
    /// `[[pickers]]` (whatever their shape) are accepted and ignored, and so is a field's
    /// `picker`. Picker checks apply from 1.1 on.
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let parse_err = |e: toml::de::Error| ManifestError::Parse(e.to_string());
        let mut table: toml::Table = toml::from_str(text).map_err(parse_err)?;
        let has_pickers = table
            .get("contract")
            .and_then(toml::Value::as_str)
            .and_then(contract_version)
            .is_none_or(|v| v >= (1, 1));
        let version = table
            .get("contract")
            .and_then(toml::Value::as_str)
            .and_then(contract_version);
        if !has_pickers {
            table.remove("pickers");
        }
        if version.is_some_and(|v| v < (1, 2)) {
            strip_1_2(&mut table);
        }
        let m: Manifest = table.try_into().map_err(parse_err)?;
        if !valid_id(&m.id) {
            return Err(ManifestError::BadId(m.id));
        }
        if !valid_semver(&m.version) {
            return Err(ManifestError::BadVersion(m.version));
        }
        if !contract_ok(&m.contract) {
            return Err(ManifestError::Incompatible { needs: m.contract });
        }
        if let Some(min) = &m.min_app {
            if !valid_semver(min) {
                return Err(ManifestError::BadVersion(min.clone()));
            }
            if semver(min) > semver(crate::APP_VERSION) {
                return Err(ManifestError::NeedsApp(min.clone()));
            }
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
        m.check_icons()?;
        if let Some(c) = &m.color
            && parse_color(c).is_none()
        {
            return Err(ManifestError::BadColor(c.clone()));
        }
        check_fields("the settings".into(), &m.settings)?;
        if has_pickers {
            m.check_pickers("the settings".into(), &m.settings)?;
        }
        for a in &m.actions {
            let scope = format!("action \"{}\"", a.id);
            check_fields(scope.clone(), &a.params)?;
            if let Some(f) = a.params.iter().find(|f| f.kind == FieldKind::Secret) {
                return Err(ManifestError::SecretParam {
                    action: a.id.clone(),
                    key: f.key.clone(),
                });
            }
            if !has_pickers {
                continue;
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
        if has_pickers {
            m.check_key_sets()?;
        }
        Ok(m)
    }

    /// Icons that name `assets/` files are safe relative paths with a usable type. Other values
    /// are icon theme names.
    fn check_icons(&self) -> Result<(), ManifestError> {
        let bad = |scope: &str, icon: &str, why| ManifestError::BadIcon {
            scope: scope.to_string(),
            icon: icon.to_string(),
            why,
        };
        let check = |scope: &str, icon: &Option<String>, symbolic: bool| match icon.as_deref() {
            Some(i) => match asset_icon_problem(i, symbolic) {
                Some(why) => Err(bad(scope, i, why)),
                None => Ok(()),
            },
            None => Ok(()),
        };
        check("the plugin", &self.icon, false)?;
        if let Some(i) = &self.symbolic_icon
            && !is_asset(i)
        {
            return Err(bad("symbolic_icon", i, "must be a file under assets/"));
        }
        check("symbolic_icon", &self.symbolic_icon, true)?;
        let colors = |states: &[ManifestState]| {
            states
                .iter()
                .flat_map(|st| [&st.color, &st.background])
                .flatten()
                .find(|c| parse_color(c).is_none())
                .map(|c| ManifestError::BadColor(c.clone()))
        };
        for a in &self.actions {
            let scope = format!("action \"{}\"", a.id);
            check(&scope, &a.icon, true)?;
            for st in &a.states {
                check(&scope, &st.icon, true)?;
            }
            if let Some(e) = colors(&a.states) {
                return Err(e);
            }
        }
        for k in &self.key_sets {
            let scope = format!("key set \"{}\"", k.id);
            check(&scope, &k.icon, true)?;
            for key in k.variants.iter().flat_map(|v| &v.keys) {
                check(&scope, &key.icon, true)?;
                for st in &key.states {
                    check(&scope, &st.icon, true)?;
                }
                if let Some(e) = colors(&key.states) {
                    return Err(e);
                }
                if let Some(c) = key.color.as_ref().filter(|c| parse_color(c).is_none()) {
                    return Err(ManifestError::BadColor(c.clone()));
                }
            }
        }
        Ok(())
    }

    fn check_key_sets(&self) -> Result<(), ManifestError> {
        for (i, k) in self.key_sets.iter().enumerate() {
            let bad = |why: String| ManifestError::BadKeySet {
                set: k.id.clone(),
                why,
            };
            if !valid_id(&k.id) {
                return Err(bad("the id must be a-z, 0-9 and _, starting with a letter".into()));
            }
            if self.key_sets[..i].iter().any(|o| o.id == k.id) {
                return Err(bad("declared twice".into()));
            }
            if self.picker(&k.picker).is_none() {
                return Err(bad(format!(
                    "picker \"{}\" is not declared in [[pickers]]",
                    k.picker
                )));
            }
            if k.variants.is_empty() || k.variants.iter().any(|v| v.keys.is_empty()) {
                return Err(bad("every variant needs at least one key".into()));
            }
            for key in k.variants.iter().flat_map(|v| &v.keys) {
                if self.action(&key.action).is_none() {
                    return Err(bad(format!("action \"{}\" is not declared", key.action)));
                }
            }
            if k.variants.iter().any(|v| v.keys.len() > MAX_KEY_SET_KEYS) {
                return Err(bad(format!("a variant has more than {MAX_KEY_SET_KEYS} keys")));
            }
            for t in &k.track {
                if !self.settings.iter().any(|f| f.key == t.setting) {
                    return Err(bad(format!(
                        "track names setting \"{}\", which is not declared",
                        t.setting
                    )));
                }
            }
        }
        Ok(())
    }

    /// The colour from `color`, if it is valid.
    pub fn rgb(&self) -> Option<[u8; 3]> {
        self.color.as_deref().and_then(parse_color)
    }

    /// Every icon file the manifest names (plugin, actions, states, key sets).
    pub fn icon_files(&self) -> Vec<&str> {
        let mut out: Vec<&str> = [&self.icon, &self.symbolic_icon]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        for a in &self.actions {
            out.extend(a.icon.as_deref());
            out.extend(a.states.iter().filter_map(|s| s.icon.as_deref()));
        }
        for k in &self.key_sets {
            out.extend(k.icon.as_deref());
            for key in k.variants.iter().flat_map(|v| &v.keys) {
                out.extend(key.icon.as_deref());
                out.extend(key.states.iter().filter_map(|s| s.icon.as_deref()));
            }
        }
        out.retain(|i| is_asset(i));
        out
    }

    pub fn key_set(&self, id: &str) -> Option<&KeySet> {
        self.key_sets.iter().find(|k| k.id == id)
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

/// Keys one variant of a key set may add (a full page of the largest deck).
pub const MAX_KEY_SET_KEYS: usize = 15;

/// Whether `icon` names a file in the plugin's `assets/` folder (rather than a theme icon).
pub fn is_asset(icon: &str) -> bool {
    icon.starts_with("assets/")
}

/// Why an `assets/` icon path is not usable; `None` when it is (or is a theme name).
fn asset_icon_problem(icon: &str, symbolic: bool) -> Option<&'static str> {
    if !is_asset(icon) {
        return None;
    }
    if icon.contains('\\')
        || icon.contains('\0')
        || icon.split('/').any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Some("must be a plain path inside assets/");
    }
    let lower = icon.to_ascii_lowercase();
    if symbolic {
        (!lower.ends_with(".svg")).then_some("must be an .svg file (one colour, tinted by the app)")
    } else {
        (!lower.ends_with(".svg") && !lower.ends_with(".png")).then_some("must be an .svg or .png file")
    }
}

/// `#rrggbb` as bytes.
pub fn parse_color(c: &str) -> Option<[u8; 3]> {
    let hex = c.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
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

/// `MAJOR.MINOR.PATCH` as numbers (a pre-release suffix such as `-beta` is ignored).
pub(crate) fn semver(v: &str) -> (u64, u64, u64) {
    let mut parts = v
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|p| p.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Drop the fields contract 1.2 added from an older manifest: top-level ones, those of each
/// action, and `show_if` of every field.
fn strip_1_2(table: &mut toml::Table) {
    for k in TOP_LEVEL_1_2 {
        table.remove(k);
    }
    let strip_fields = |fields: Option<&mut toml::Value>| {
        if let Some(toml::Value::Array(list)) = fields {
            for f in list.iter_mut().filter_map(toml::Value::as_table_mut) {
                f.remove("show_if");
            }
        }
    };
    strip_fields(table.get_mut("settings"));
    if let Some(toml::Value::Array(actions)) = table.get_mut("actions") {
        for a in actions.iter_mut().filter_map(toml::Value::as_table_mut) {
            for k in ACTION_1_2 {
                a.remove(k);
            }
            strip_fields(a.get_mut("params"));
        }
    }
}

/// `MAJOR.MINOR` as numbers.
fn contract_version(c: &str) -> Option<(u32, u32)> {
    let (major, minor) = c.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// `MAJOR.MINOR` with the same major as the host and a minor the host has.
fn contract_ok(c: &str) -> bool {
    contract_version(c).is_some_and(|(major, minor)| major == CONTRACT.0 && minor <= CONTRACT.1)
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
