use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a key, dial zone, small button or the wide screen does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    NextPage,
    PrevPage,
    GoToPage {
        page: Uuid,
    },
    OpenFolder {
        page: Uuid,
    },
    Back,
    SwitchProfile {
        profile: Uuid,
    },
    AutoProfile,
    Command {
        cmd: String,
        #[serde(default)]
        terminal: bool,
    },
    LaunchApp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        desktop_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exec: Option<String>,
    },
    OpenUrl {
        url: String,
    },
    Shortcut {
        keys: String,
    },
    TypeText {
        text: String,
    },
    PlaySound {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device: Option<String>,
        /// Percent of full volume, times the sound volume in Preferences.
        #[serde(default = "full_volume", skip_serializing_if = "is_full_volume")]
        volume: u8,
    },
    Media {
        control: MediaControl,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        player: Option<String>,
    },
    /// Jump forward (positive) or back (negative) in the playing track.
    Seek {
        seconds: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        player: Option<String>,
    },
    /// Make the next of these outputs the default (all outputs when empty).
    SwitchOutput {
        #[serde(default)]
        devices: Vec<String>,
        /// What switching to an output sets, and how the key looks while it plays, by output
        /// name. Any output can have one, in the ring or not.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        outputs: std::collections::BTreeMap<String, OutputSettings>,
    },
    Volume {
        target: AudioTarget,
        op: VolumeOp,
        #[serde(default = "default_step")]
        step: u8,
        #[serde(default = "yes")]
        limit_100: bool,
    },
    Brightness {
        op: BrightnessOp,
    },
    /// The brightness of a PC monitor, not the deck's.
    DisplayBrightness {
        display: DisplayTarget,
        op: BrightnessOp,
    },
    SleepNow,
    /// Show the next widget on the wide screen.
    ScreenNext,
    Timer {
        op: TimerOp,
    },
    Multi {
        steps: Vec<Step>,
    },
    /// Each press runs the next step, then starts over; the key shows the step that ran last or
    /// the one the next press runs (`show`).
    Cycle {
        steps: Vec<CycleStep>,
        #[serde(default)]
        show: CycleShows,
    },
    /// A macro from the library, or this key's own copy.
    Macro {
        id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        own: Option<Box<crate::model::Macro>>,
    },
    /// Stop every macro that is playing.
    StopMacros,
    /// An action a plugin provides. Kept as written when the plugin is missing or off.
    Plugin {
        plugin: String,
        action: String,
        #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
        params: toml::Table,
    },
}

fn full_volume() -> u8 {
    100
}

fn is_full_volume(v: &u8) -> bool {
    *v == 100
}

fn default_step() -> u8 {
    5
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: Action,
    #[serde(default)]
    pub delay_ms: u32,
}

/// Which step a cycling key shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleShows {
    /// The step that ran last: what the key did (the last step before the first press).
    #[default]
    Last,
    /// The step the next press runs.
    Next,
}

impl CycleShows {
    /// The step to show of `n`, when `next` runs next.
    pub fn step(self, next: usize, n: usize) -> usize {
        match self {
            CycleShows::Next => next % n,
            CycleShows::Last => (next % n + n - 1) % n,
        }
    }
}

/// One step of a [`Action::Cycle`]: its action and how the key looks while it is next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CycleStep {
    pub action: Action,
    #[serde(default, skip_serializing_if = "crate::model::LookOverride::is_empty")]
    pub look: crate::model::LookOverride,
}

/// One output of an [`Action::SwitchOutput`]: the levels a switch to it sets (before the
/// switch, so nothing plays loud) and the key's look while it is the default output.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputSettings {
    /// The output's own volume in percent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    /// App volumes in percent, by app name (matched like the Volume action matches).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub apps: std::collections::BTreeMap<String, u8>,
    #[serde(skip_serializing_if = "crate::model::LookOverride::is_empty")]
    pub look: crate::model::LookOverride,
}

/// Which of a Switch Output key's `outputs` is `current` (the default output's name), as
/// [`crate::model::LiveKey::step`] counts them.
pub fn playing_output(
    outputs: &std::collections::BTreeMap<String, OutputSettings>,
    current: &str,
) -> Option<usize> {
    outputs.keys().position(|name| name == current)
}

impl OutputSettings {
    pub fn is_empty(&self) -> bool {
        *self == OutputSettings::default()
    }
}

/// What a timer key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimerOp {
    StartPause,
    Reset,
    Add { minutes: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaControl {
    PlayPause,
    Next,
    Previous,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioTarget {
    Default,
    Input,
    Device { name: String },
    App { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VolumeOp {
    Up,
    Down,
    Mute,
    Set { percent: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrightnessOp {
    Set { percent: u8 },
    Up { step: u8 },
    Down { step: u8 },
}

/// Which PC monitors a Display Brightness key changes. A monitor goes by its name, never its
/// number, which changes between sessions; `nth` tells monitors with the same name apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DisplayTarget {
    All,
    Named {
        name: String,
        #[serde(default)]
        nth: u8,
    },
}

impl Action {
    /// An action of the OBS plugin (`plugin = "obs"`) with `params`.
    pub fn obs(action: &str, params: &[(&str, &str)]) -> Action {
        Action::Plugin {
            plugin: "obs".into(),
            action: action.into(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_string(), toml::Value::String((*v).to_string())))
                .collect(),
        }
    }
}

/// A TOML table as JSON, for handing to a plugin. Datetimes become their TOML text form (RFC 3339 when they carry an offset).
pub fn table_to_json(t: &toml::Table) -> serde_json::Value {
    serde_json::Value::Object(t.iter().map(|(k, v)| (k.clone(), value_to_json(v))).collect())
}

fn value_to_json(v: &toml::Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        toml::Value::String(s) => J::String(s.clone()),
        toml::Value::Integer(i) => J::from(*i),
        // NaN and infinity have no JSON form.
        toml::Value::Float(f) => serde_json::Number::from_f64(*f).map_or(J::Null, J::Number),
        toml::Value::Boolean(b) => J::Bool(*b),
        toml::Value::Datetime(d) => J::String(d.to_string()),
        toml::Value::Array(a) => J::Array(a.iter().map(value_to_json).collect()),
        toml::Value::Table(t) => table_to_json(t),
    }
}
