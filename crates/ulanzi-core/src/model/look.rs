use serde::{Deserialize, Serialize};

use crate::Color;

/// How a key looks on the device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyLook {
    pub source: ImageSource,
    pub background: Color,
    pub foreground: Color,
    /// Share of the key the icon occupies (0.70 leaves 15% padding on each side).
    pub icon_scale: f32,
    pub label: Label,
    /// Show live state from the action (e.g. play/pause swap).
    pub dynamic: bool,
}

impl Default for KeyLook {
    fn default() -> Self {
        Self {
            source: ImageSource::None,
            background: Color::BLACK,
            foreground: Color::WHITE,
            icon_scale: 0.70,
            label: Label::default(),
            dynamic: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Label {
    pub text: String,
    pub visible: bool,
}

impl Default for Label {
    fn default() -> Self {
        Self {
            text: String::new(),
            visible: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconSet {
    Lucide,
    Brands,
}

/// Where a key's image comes from.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageSource {
    #[default]
    None,
    Icon {
        set: IconSet,
        name: String,
    },
    /// Path relative to the profile directory.
    Image {
        path: String,
    },
    Text {
        content: String,
    },
    App {
        desktop_id: String,
    },
    Steam {
        appid: u32,
    },
    Emoji {
        ch: String,
    },
}
