use serde::{Deserialize, Serialize};

/// How the GUI edits a setting or action parameter. Kinds the host does not know are kept as
/// [`FieldKind::Other`] and never refused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum FieldKind {
    #[default]
    Text,
    Number,
    Toggle,
    Choice,
    Color,
    Json,
    Item,
    KeyCombo,
    File,
    Secret,
    Other(String),
}

impl From<String> for FieldKind {
    fn from(s: String) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "text" => Self::Text,
            "number" => Self::Number,
            "toggle" => Self::Toggle,
            "choice" => Self::Choice,
            "color" => Self::Color,
            "json" => Self::Json,
            "item" => Self::Item,
            "key_combo" => Self::KeyCombo,
            "file" => Self::File,
            "secret" => Self::Secret,
            _ => Self::Other(s),
        }
    }
}

impl From<FieldKind> for String {
    fn from(k: FieldKind) -> Self {
        match k {
            FieldKind::Text => "text".into(),
            FieldKind::Number => "number".into(),
            FieldKind::Toggle => "toggle".into(),
            FieldKind::Choice => "choice".into(),
            FieldKind::Color => "color".into(),
            FieldKind::Json => "json".into(),
            FieldKind::Item => "item".into(),
            FieldKind::KeyCombo => "key_combo".into(),
            FieldKind::File => "file".into(),
            FieldKind::Secret => "secret".into(),
            FieldKind::Other(s) => s,
        }
    }
}

/// One option of a `choice` field: a bare value, or a value with its own label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Choice {
    Plain(String),
    Labelled { value: String, label: String },
}

impl Choice {
    pub fn value(&self) -> &str {
        match self {
            Self::Plain(v) | Self::Labelled { value: v, .. } => v,
        }
    }

    /// The text shown to the user; a plain value gets its first letter upper-cased.
    pub fn label(&self) -> String {
        match self {
            Self::Labelled { label, .. } => label.clone(),
            Self::Plain(v) => {
                let mut chars = v.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
        }
    }
}

/// A plugin setting (`[[settings]]`) or an action parameter (`params`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub default: Option<toml::Value>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub picker: Option<String>,
    #[serde(default)]
    pub ask_first: bool,
    /// Show the field only while another one has one of some values (`op` is `set`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_if: Option<ShowIf>,
}

/// `show_if = { key = "op", is = ["set", "up"] }`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowIf {
    pub key: String,
    pub is: Vec<String>,
}

impl ShowIf {
    /// Whether the field shows when the other field's value is `value` (its text).
    pub fn holds(&self, value: &str) -> bool {
        self.is.iter().any(|v| v == value)
    }
}

/// The bound a number field without `min` or `max` gets.
pub const UNBOUNDED: f64 = 1e9;

impl Field {
    /// What a parameter's row shows when nothing is stored: the manifest default, else the first
    /// choice of a choice field, else the number a spin row falls back to (0 within the bounds).
    /// A newly added action stores exactly this, so what the rows show is what the key does.
    pub fn initial_value(&self) -> Option<toml::Value> {
        if let Some(d) = &self.default {
            return Some(d.clone());
        }
        match &self.kind {
            FieldKind::Choice => self
                .choices
                .first()
                .map(|c| toml::Value::String(c.value().to_string())),
            FieldKind::Number => {
                let n = 0.0_f64
                    .max(self.min.unwrap_or(-UNBOUNDED))
                    .min(self.max.unwrap_or(UNBOUNDED));
                Some(if self.is_integer() {
                    toml::Value::Integer(n.round() as i64)
                } else {
                    toml::Value::Float(n)
                })
            }
            _ => None,
        }
    }

    /// The label, or the key when there is none.
    pub fn title(&self) -> &str {
        if self.label.is_empty() {
            &self.key
        } else {
            &self.label
        }
    }

    /// Should a number be written as a TOML integer? True when at least one of `min`, `max`,
    /// `step` and `default` is given and every given one is a whole number.
    pub fn is_integer(&self) -> bool {
        let default = match &self.default {
            Some(toml::Value::Integer(i)) => Some(*i as f64),
            Some(toml::Value::Float(f)) => Some(*f),
            _ => None,
        };
        let given: Vec<f64> = [self.min, self.max, self.step, default]
            .into_iter()
            .flatten()
            .collect();
        !given.is_empty() && given.iter().all(|v| v.is_finite() && v.fract() == 0.0)
    }
}
