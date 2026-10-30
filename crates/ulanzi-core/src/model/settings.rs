use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::profile::FORMAT_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearancePref {
    #[default]
    System,
    Light,
    Dark,
}

/// App and device settings (`settings.toml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub appearance: AppearancePref,
    pub brightness: u8,
    /// Minutes without input before dimming; 0 = never.
    pub sleep_after_min: u32,
    pub sleep_brightness: u8,
    pub hold_ms: u32,
    /// Volume of Play Sound keys, percent (each key's own volume is a share of it).
    pub sound_volume: u8,
    pub hide_labels: bool,
    pub label_style: LabelStyleCfg,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<Uuid>,
    /// Kept so older settings files still load. "Start at Login" is the systemd unit's
    /// enabled state (`systemctl --user is-enabled ulanzi-deckd`), not this value.
    pub autostart: bool,
    pub tray: bool,
    /// Switch profiles by the app in front (each profile's `match` apps).
    pub auto_switch: bool,
    /// Show volume changes on the wide screen for a moment.
    pub volume_overlay: bool,
    /// The order profiles are listed in (ids not listed come after, in their own order).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub profile_order: Vec<Uuid>,
    /// Per-plugin switch and settings, by plugin id (`[plugins.<id>]`).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, PluginEntry>,
    /// Allows installing plugins from a folder.
    pub developer_mode: bool,
    /// What a newer version wrote; kept so a save does not lose it.
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// One plugin's entry in `settings.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginEntry {
    pub enabled: bool,
    pub settings: toml::Table,
    /// What a newer version wrote; kept so a save does not lose it.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for PluginEntry {
    fn default() -> Self {
        Self {
            enabled: true,
            settings: toml::Table::new(),
            extra: toml::Table::new(),
        }
    }
}

/// Does a plugin setting's name look like a credential? Used for plugins whose manifest is not known.
pub fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["password", "token", "secret", "key"]
        .iter()
        .any(|w| k.contains(w))
}

/// Is `key` a secret setting of plugin `id`? `declared` has the secret keys of every plugin whose
/// manifest is known; for the others the name decides.
pub fn is_plugin_secret(declared: &BTreeMap<String, BTreeSet<String>>, id: &str, key: &str) -> bool {
    match declared.get(id) {
        Some(keys) => keys.contains(key),
        None => looks_secret(key),
    }
}

impl Settings {
    /// Remove every plugin secret (declared ones, and any key that looks like a credential), for
    /// files that get shared.
    pub fn strip_plugin_secrets(&mut self, declared: &BTreeMap<String, BTreeSet<String>>) {
        for (id, e) in &mut self.plugins {
            e.settings
                .retain(|k, _| !looks_secret(k) && !declared.get(id).is_some_and(|d| d.contains(k)));
        }
    }
}

/// Sort `list` by `order`; entries not in `order` keep their relative order at the end.
pub fn order_profiles<T>(list: &mut [(Uuid, T)], order: &[Uuid]) {
    list.sort_by_key(|(id, _)| order.iter().position(|o| o == id).unwrap_or(usize::MAX));
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            appearance: AppearancePref::System,
            brightness: 100,
            sleep_after_min: 10,
            sleep_brightness: 0,
            hold_ms: 500,
            sound_volume: 100,
            hide_labels: false,
            label_style: LabelStyleCfg::default(),
            default_profile: None,
            autostart: true,
            tray: true,
            auto_switch: true,
            volume_overlay: true,
            profile_order: Vec::new(),
            plugins: BTreeMap::new(),
            developer_mode: false,
            extra: toml::Table::new(),
        }
    }
}

/// Firmware label style (sent with the label-style command).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelStyleCfg {
    pub align: String,
    pub color: u32,
    pub font_name: String,
    pub size: u32,
    pub weight: u32,
    /// What a newer version wrote; kept so a save does not lose it.
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Default for LabelStyleCfg {
    fn default() -> Self {
        let d = ulanzi_proto::LabelStyle::default();
        Self {
            align: d.align,
            color: d.color,
            font_name: d.font_name,
            size: d.size,
            weight: d.weight,
            extra: toml::Table::new(),
        }
    }
}

impl LabelStyleCfg {
    pub fn to_proto(&self, show_title: bool) -> ulanzi_proto::LabelStyle {
        ulanzi_proto::LabelStyle {
            align: self.align.clone(),
            color: self.color,
            font_name: self.font_name.clone(),
            show_title,
            size: self.size,
            weight: self.weight,
        }
    }
}
