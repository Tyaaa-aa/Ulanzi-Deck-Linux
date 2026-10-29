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
    /// Looks for live states, first match wins (a light that is on, an air conditioner that
    /// cools). Used while `dynamic` is on.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<LookState>,
}

/// Parts of a key's look that replace the key's own while something holds; `None` keeps it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LookOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ImageSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl LookOverride {
    pub fn is_empty(&self) -> bool {
        *self == LookOverride::default()
    }

    /// `look` with these parts put in.
    pub fn apply(&self, look: &mut KeyLook) {
        if let Some(s) = &self.source {
            look.source = s.clone();
        }
        if let Some(c) = self.foreground {
            look.foreground = c;
        }
        if let Some(c) = self.background {
            look.background = c;
        }
        if let Some(t) = &self.label {
            look.label.text = t.clone();
        }
    }
}

/// A look used while a live value matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookState {
    pub when: When,
    #[serde(default)]
    pub look: LookOverride,
}

/// A condition on a live value: `value` is a label text such as `{ha.$entity_id}` (`$<param>` is
/// the key's action parameter), compared without case to `is` (any of them) and `not` (none of
/// them). An empty value (nothing published yet) never matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    pub value: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub is: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not: Vec<String>,
}

impl When {
    /// Whether the resolved `value` meets the condition.
    pub fn holds(&self, value: &str) -> bool {
        let v = value.trim();
        let any = |list: &[String]| list.iter().any(|x| x.trim().eq_ignore_ascii_case(v));
        !v.is_empty() && (self.is.is_empty() || any(&self.is)) && !any(&self.not)
    }
}

/// `text` with every `$<key>` of `params` (string, number or bool values) filled in, longest keys
/// first so `$entity_id` wins over `$entity`.
pub fn fill_params(text: &str, params: &toml::Table) -> String {
    if !text.contains('$') {
        return text.to_string();
    }
    let mut keys: Vec<(&String, String)> = params
        .iter()
        .filter_map(|(k, v)| {
            let v = match v {
                toml::Value::String(s) => s.clone(),
                toml::Value::Integer(i) => i.to_string(),
                toml::Value::Float(f) => f.to_string(),
                toml::Value::Boolean(b) => b.to_string(),
                _ => return None,
            };
            Some((k, v))
        })
        .collect();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    let mut out = text.to_string();
    for (k, v) in keys {
        out = out.replace(&format!("${k}"), &v);
    }
    out
}

impl KeyLook {
    /// The look with the first state whose condition holds put in. `resolve` turns a label text
    /// (with `$<param>` already filled in) into its current value.
    pub fn with_state(&self, params: &toml::Table, resolve: impl Fn(&str) -> String) -> KeyLook {
        let mut look = self.clone();
        if !self.dynamic {
            return look;
        }
        if let Some(st) = self
            .states
            .iter()
            .find(|st| st.when.holds(&resolve(&fill_params(&st.when.value, params))))
        {
            st.look.apply(&mut look);
        }
        look
    }
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
            states: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Label {
    pub text: String,
    pub visible: bool,
    /// Where this key's label goes; `None` uses the position from Preferences (drawn by the deck).
    /// A key with its own position has its label drawn into the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<LabelPosition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelPosition {
    Top,
    Middle,
    Bottom,
}

impl LabelPosition {
    pub const ALL: [LabelPosition; 3] = [LabelPosition::Top, LabelPosition::Middle, LabelPosition::Bottom];

    pub fn title(self) -> &'static str {
        match self {
            LabelPosition::Top => "Top",
            LabelPosition::Middle => "Middle",
            LabelPosition::Bottom => "Bottom",
        }
    }
}

impl Default for Label {
    fn default() -> Self {
        Self {
            text: String::new(),
            visible: true,
            position: None,
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
    /// A one-colour icon from an installed plugin's `assets/` folder, tinted like a bundled icon.
    Plugin {
        plugin: String,
        icon: String,
    },
}
