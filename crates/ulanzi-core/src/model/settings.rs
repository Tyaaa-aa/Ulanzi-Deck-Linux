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
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub appearance: AppearancePref,
    pub brightness: u8,
    /// Minutes without input before dimming; 0 = never.
    pub sleep_after_min: u32,
    pub sleep_brightness: u8,
    pub hold_ms: u32,
    pub hide_labels: bool,
    pub label_style: LabelStyleCfg,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<Uuid>,
    pub obs: ObsSettings,
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
            hide_labels: false,
            label_style: LabelStyleCfg::default(),
            default_profile: None,
            obs: ObsSettings::default(),
            autostart: true,
            tray: true,
            auto_switch: true,
            volume_overlay: true,
            profile_order: Vec::new(),
        }
    }
}

/// Firmware label style (sent with the label-style command).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LabelStyleCfg {
    pub align: String,
    pub color: u32,
    pub font_name: String,
    pub size: u32,
    pub weight: u32,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObsSettings {
    pub host: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub auto_detect: bool,
}

impl Default for ObsSettings {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 4455,
            password: None,
            auto_detect: true,
        }
    }
}
