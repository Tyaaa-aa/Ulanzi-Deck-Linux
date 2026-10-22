//! Every action the app offers: its name, category, icon and the look a key gets when the action is
//! dropped on it.

use uuid::Uuid;

use crate::Color;
use crate::model::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Navigation,
    System,
    Media,
    Audio,
    Obs,
    Device,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Navigation,
        Category::System,
        Category::Media,
        Category::Audio,
        Category::Obs,
        Category::Device,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Navigation => "Navigation",
            Category::System => "System",
            Category::Media => "Media",
            Category::Audio => "Audio",
            Category::Obs => "OBS Studio",
            Category::Device => "Device",
        }
    }

    /// Accent colour for the category (icon tint and library chip).
    pub fn color(self) -> Color {
        match self {
            Category::Navigation => Color::rgb(0x35, 0x84, 0xe4),
            Category::System => Color::rgb(0x33, 0xd1, 0x7a),
            Category::Media | Category::Audio => Color::rgb(0xff, 0x78, 0x00),
            Category::Obs => Color::rgb(0x91, 0x41, 0xac),
            Category::Device => Color::rgb(0xf6, 0xd3, 0x2d),
        }
    }
}

pub struct ActionSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub category: Category,
    /// Lucide icon name.
    pub icon: &'static str,
    /// A new action of this kind with sensible defaults.
    pub make: fn() -> Action,
    matches: fn(&Action) -> bool,
}

macro_rules! spec {
    ($id:literal, $title:literal, $cat:ident, $icon:literal, $make:expr, $pat:pat) => {
        ActionSpec {
            id: $id,
            title: $title,
            category: Category::$cat,
            icon: $icon,
            make: || $make,
            matches: |a| matches!(a, $pat),
        }
    };
}

fn vol(op: VolumeOp) -> Action {
    Action::Volume {
        target: AudioTarget::Default,
        op,
        step: 5,
        limit_100: true,
    }
}

static CATALOG: &[ActionSpec] = &[
    spec!(
        "next_page",
        "Next Page",
        Navigation,
        "chevron-right",
        Action::NextPage,
        Action::NextPage
    ),
    spec!(
        "prev_page",
        "Previous Page",
        Navigation,
        "chevron-left",
        Action::PrevPage,
        Action::PrevPage
    ),
    spec!(
        "go_to_page",
        "Go to Page",
        Navigation,
        "hash",
        Action::GoToPage { page: Uuid::nil() },
        Action::GoToPage { .. }
    ),
    spec!(
        "open_folder",
        "Open Folder",
        Navigation,
        "folder-open",
        Action::OpenFolder { page: Uuid::nil() },
        Action::OpenFolder { .. }
    ),
    spec!(
        "back",
        "Back",
        Navigation,
        "arrow-left",
        Action::Back,
        Action::Back
    ),
    spec!(
        "switch_profile",
        "Switch Profile",
        Navigation,
        "arrow-left-right",
        Action::SwitchProfile { profile: Uuid::nil() },
        Action::SwitchProfile { .. }
    ),
    spec!(
        "auto_profile",
        "Auto-Switch On/Off",
        Navigation,
        "refresh-cw",
        Action::AutoProfile,
        Action::AutoProfile
    ),
    spec!(
        "command",
        "Run Command",
        System,
        "terminal",
        Action::Command {
            cmd: String::new(),
            terminal: false
        },
        Action::Command { .. }
    ),
    spec!(
        "launch_app",
        "Launch App",
        System,
        "app-window",
        Action::LaunchApp {
            desktop_id: None,
            exec: None
        },
        Action::LaunchApp { .. }
    ),
    spec!(
        "open_url",
        "Open Website",
        System,
        "globe",
        Action::OpenUrl { url: String::new() },
        Action::OpenUrl { .. }
    ),
    spec!(
        "shortcut",
        "Keyboard Shortcut",
        System,
        "keyboard",
        Action::Shortcut { keys: String::new() },
        Action::Shortcut { .. }
    ),
    spec!(
        "type_text",
        "Type Text",
        System,
        "type",
        Action::TypeText { text: String::new() },
        Action::TypeText { .. }
    ),
    spec!(
        "multi",
        "Multi Action",
        System,
        "list-ordered",
        Action::Multi { steps: Vec::new() },
        Action::Multi { .. }
    ),
    spec!(
        "play_pause",
        "Play / Pause",
        Media,
        "play",
        Action::Media {
            control: MediaControl::PlayPause,
            player: None
        },
        Action::Media {
            control: MediaControl::PlayPause,
            ..
        }
    ),
    spec!(
        "next_track",
        "Next Track",
        Media,
        "skip-forward",
        Action::Media {
            control: MediaControl::Next,
            player: None
        },
        Action::Media {
            control: MediaControl::Next,
            ..
        }
    ),
    spec!(
        "prev_track",
        "Previous Track",
        Media,
        "skip-back",
        Action::Media {
            control: MediaControl::Previous,
            player: None
        },
        Action::Media {
            control: MediaControl::Previous,
            ..
        }
    ),
    spec!(
        "stop",
        "Stop",
        Media,
        "square",
        Action::Media {
            control: MediaControl::Stop,
            player: None
        },
        Action::Media {
            control: MediaControl::Stop,
            ..
        }
    ),
    spec!(
        "play_sound",
        "Play Sound",
        Audio,
        "music",
        Action::PlaySound {
            path: String::new(),
            device: None
        },
        Action::PlaySound { .. }
    ),
    spec!(
        "volume_up",
        "Volume Up",
        Audio,
        "volume-2",
        vol(VolumeOp::Up),
        Action::Volume { op: VolumeOp::Up, .. }
    ),
    spec!(
        "volume_down",
        "Volume Down",
        Audio,
        "volume-1",
        vol(VolumeOp::Down),
        Action::Volume {
            op: VolumeOp::Down,
            ..
        }
    ),
    spec!(
        "mute",
        "Mute",
        Audio,
        "volume-x",
        vol(VolumeOp::Mute),
        Action::Volume {
            op: VolumeOp::Mute,
            ..
        }
    ),
    spec!(
        "volume_set",
        "Set Volume",
        Audio,
        "sliders-horizontal",
        vol(VolumeOp::Set { percent: 50 }),
        Action::Volume {
            op: VolumeOp::Set { .. },
            ..
        }
    ),
    spec!(
        "obs_scene",
        "Switch Scene",
        Obs,
        "clapperboard",
        Action::Obs(ObsAction::SetScene { scene: String::new() }),
        Action::Obs(ObsAction::SetScene { .. })
    ),
    spec!(
        "obs_toggle_scenes",
        "Toggle Scenes",
        Obs,
        "repeat",
        Action::Obs(ObsAction::ToggleScenes {
            a: String::new(),
            b: String::new()
        }),
        Action::Obs(ObsAction::ToggleScenes { .. })
    ),
    spec!(
        "obs_source",
        "Show / Hide Source",
        Obs,
        "eye",
        Action::Obs(ObsAction::ToggleSource {
            scene: String::new(),
            source: String::new()
        }),
        Action::Obs(ObsAction::ToggleSource { .. })
    ),
    spec!(
        "obs_record",
        "Record",
        Obs,
        "circle-dot",
        Action::Obs(ObsAction::ToggleRecord),
        Action::Obs(ObsAction::ToggleRecord)
    ),
    spec!(
        "obs_stream",
        "Stream",
        Obs,
        "radio",
        Action::Obs(ObsAction::ToggleStream),
        Action::Obs(ObsAction::ToggleStream)
    ),
    spec!(
        "brightness_up",
        "Brightness Up",
        Device,
        "sun",
        Action::Brightness {
            op: BrightnessOp::Up { step: 10 }
        },
        Action::Brightness {
            op: BrightnessOp::Up { .. }
        }
    ),
    spec!(
        "brightness_down",
        "Brightness Down",
        Device,
        "sun-dim",
        Action::Brightness {
            op: BrightnessOp::Down { step: 10 }
        },
        Action::Brightness {
            op: BrightnessOp::Down { .. }
        }
    ),
    spec!(
        "brightness_set",
        "Set Brightness",
        Device,
        "sun-medium",
        Action::Brightness {
            op: BrightnessOp::Set { percent: 50 }
        },
        Action::Brightness {
            op: BrightnessOp::Set { .. }
        }
    ),
    spec!(
        "sleep_now",
        "Sleep Deck",
        Device,
        "moon",
        Action::SleepNow,
        Action::SleepNow
    ),
];

pub fn catalog() -> &'static [ActionSpec] {
    CATALOG
}

pub fn spec_for(a: &Action) -> Option<&'static ActionSpec> {
    CATALOG.iter().find(|s| (s.matches)(a))
}

/// The look a key gets when `a` is dropped on an empty key: the action's icon in its category
/// colour on black, labelled with the action's title.
pub fn default_look(a: &Action) -> KeyLook {
    let Some(spec) = spec_for(a) else {
        return KeyLook::default();
    };
    // An app launcher shows the app itself.
    if let Action::LaunchApp {
        desktop_id: Some(id), ..
    } = a
    {
        let name = crate::apps::lookup(id)
            .map(|e| e.name)
            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_string());
        return KeyLook {
            source: ImageSource::App {
                desktop_id: id.clone(),
            },
            label: Label {
                text: name,
                visible: true,
                position: None,
            },
            ..KeyLook::default()
        };
    }
    KeyLook {
        source: ImageSource::Icon {
            set: IconSet::Lucide,
            name: spec.icon.into(),
        },
        background: Color::BLACK,
        foreground: spec.category.color(),
        label: Label {
            text: spec.title.into(),
            visible: true,
            position: None,
        },
        ..KeyLook::default()
    }
}

/// After a key's action changes from `old` to `new`, replace each part of its look that still shows
/// the old action's default (icon, colours, label) with the new action's; customised parts stay.
pub fn refresh_look(look: &mut KeyLook, old: Option<&Action>, new: &Action) {
    let was = old.map(default_look).unwrap_or_default();
    let blank = KeyLook::default();
    let now = default_look(new);
    if look.source == was.source || look.source == blank.source {
        look.source = now.source;
    }
    if look.foreground == was.foreground {
        look.foreground = now.foreground;
    }
    if look.background == was.background {
        look.background = now.background;
    }
    if look.label.text == was.label.text || look.label.text.is_empty() {
        look.label.text = now.label.text;
    }
}

fn ellipsize(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

fn pretty_combo(keys: &str) -> String {
    keys.split('+')
        .map(|k| {
            let k = k.trim();
            match k.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => "Ctrl".into(),
                "alt" => "Alt".into(),
                "shift" => "Shift".into(),
                "super" | "meta" | "win" => "Super".into(),
                _ if k.chars().count() == 1 => k.to_uppercase(),
                _ => k.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn target_name(t: &AudioTarget) -> String {
    match t {
        AudioTarget::Default => "default output".into(),
        AudioTarget::Input => "microphone".into(),
        AudioTarget::Device { name } | AudioTarget::App { name } => name.clone(),
    }
}

/// One-line human summary of an action, for the inspector's action card.
pub fn describe(a: &Action) -> String {
    let title = spec_for(a).map(|s| s.title).unwrap_or("Action");
    match a {
        Action::Command { cmd, .. } => ellipsize(cmd, 48),
        Action::LaunchApp { desktop_id, exec } => {
            let name = desktop_id
                .as_deref()
                .map(|d| d.trim_end_matches(".desktop"))
                .or(exec.as_deref());
            name.map(|n| format!("Launch · {}", ellipsize(n, 36)))
                .unwrap_or_else(|| title.into())
        }
        Action::OpenUrl { url } => ellipsize(url, 48),
        Action::Shortcut { keys } => format!("Shortcut · {}", pretty_combo(keys)),
        Action::TypeText { text } => format!("Type “{}”", ellipsize(text, 32)),
        Action::PlaySound { path, .. } => {
            let file = std::path::Path::new(path)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned());
            format!("Play · {}", ellipsize(&file.unwrap_or_default(), 36))
        }
        Action::Volume { target, op, step, .. } => match op {
            VolumeOp::Up => format!("Volume up {step}% · {}", target_name(target)),
            VolumeOp::Down => format!("Volume down {step}% · {}", target_name(target)),
            VolumeOp::Mute => format!("Mute · {}", target_name(target)),
            VolumeOp::Set { percent } => format!("Volume {percent}% · {}", target_name(target)),
        },
        Action::Multi { steps } => format!("{} steps", steps.len()),
        Action::Obs(ObsAction::SetScene { scene }) if !scene.is_empty() => format!("Scene · {scene}"),
        Action::Obs(ObsAction::ToggleScenes { a, b }) if !a.is_empty() => format!("{a} ⇄ {b}"),
        Action::Obs(ObsAction::ToggleSource { source, .. }) if !source.is_empty() => {
            format!("Show / hide · {source}")
        }
        _ => title.into(),
    }
}
