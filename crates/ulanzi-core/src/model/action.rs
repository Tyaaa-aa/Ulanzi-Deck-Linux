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
    },
    Media {
        control: MediaControl,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        player: Option<String>,
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
    SleepNow,
    /// Show the next widget on the wide screen.
    ScreenNext,
    Timer {
        op: TimerOp,
    },
    Multi {
        steps: Vec<Step>,
    },
    Obs(ObsAction),
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

fn two_db() -> u8 {
    2
}

/// What the deck can do in OBS Studio (obs-websocket 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "obs", rename_all = "snake_case")]
pub enum ObsAction {
    /// Show a scene (on the preview instead, in studio mode, with `preview`).
    SetScene {
        scene: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        preview: bool,
    },
    ToggleScenes {
        a: String,
        b: String,
    },
    ToggleSource {
        scene: String,
        source: String,
    },
    ToggleRecord,
    StartRecord,
    StopRecord,
    /// Pause or resume the recording.
    PauseRecord,
    ToggleStream,
    StartStream,
    StopStream,
    ToggleReplay,
    SaveReplay,
    ToggleVirtualCam,
    ToggleMute {
        input: String,
    },
    /// Change an input's volume; `step` is in dB.
    InputVolume {
        input: String,
        op: VolumeOp,
        #[serde(default = "two_db")]
        step: u8,
    },
    ToggleFilter {
        source: String,
        filter: String,
    },
    /// Studio mode: send the preview to the program.
    StudioTransition,
    SetCollection {
        name: String,
    },
    SetProfile {
        name: String,
    },
    /// OBS's own screenshot of the program (saved where OBS saves them).
    Screenshot,
}
