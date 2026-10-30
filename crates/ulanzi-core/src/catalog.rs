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
    Device,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Navigation,
        Category::System,
        Category::Media,
        Category::Audio,
        Category::Device,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Navigation => "Navigation",
            Category::System => "System",
            Category::Media => "Media",
            Category::Audio => "Audio",
            Category::Device => "Device",
        }
    }

    /// Accent colour for the category (icon tint and library chip).
    pub fn color(self) -> Color {
        match self {
            Category::Navigation => Color::rgb(0x35, 0x84, 0xe4),
            Category::System => Color::rgb(0x33, 0xd1, 0x7a),
            Category::Media | Category::Audio => Color::rgb(0xff, 0x78, 0x00),
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
        "cycle",
        "Cycle Through Actions",
        System,
        "repeat",
        Action::Cycle {
            steps: Vec::new(),
            show: CycleShows::Last,
        },
        Action::Cycle { .. }
    ),
    spec!(
        "stop_macros",
        "Stop All Macros",
        System,
        "circle-stop",
        Action::StopMacros,
        Action::StopMacros
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
        "seek_back",
        "Skip Back",
        Media,
        "rewind",
        Action::Seek {
            seconds: -10,
            player: None
        },
        Action::Seek { seconds: ..=0, .. }
    ),
    spec!(
        "seek_forward",
        "Skip Forward",
        Media,
        "fast-forward",
        Action::Seek {
            seconds: 10,
            player: None
        },
        Action::Seek { seconds: 1.., .. }
    ),
    spec!(
        "switch_output",
        "Switch Output",
        Audio,
        "speaker",
        Action::SwitchOutput { devices: vec![] },
        Action::SwitchOutput { .. }
    ),
    spec!(
        "play_sound",
        "Play Sound",
        Audio,
        "music",
        Action::PlaySound {
            path: String::new(),
            device: None,
            volume: 100
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
    spec!(
        "screen_next",
        "Next Widget",
        Device,
        "panel-bottom",
        Action::ScreenNext,
        Action::ScreenNext
    ),
    spec!(
        "timer_start",
        "Timer Start / Pause",
        Device,
        "timer",
        Action::Timer {
            op: TimerOp::StartPause
        },
        Action::Timer {
            op: TimerOp::StartPause
        }
    ),
    spec!(
        "timer_reset",
        "Timer Reset",
        Device,
        "timer-reset",
        Action::Timer { op: TimerOp::Reset },
        Action::Timer { op: TimerOp::Reset }
    ),
    spec!(
        "timer_add",
        "Add Time",
        Device,
        "alarm-clock-plus",
        Action::Timer {
            op: TimerOp::Add { minutes: 5 }
        },
        Action::Timer {
            op: TimerOp::Add { .. }
        }
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
    if let Action::Plugin { plugin, action, .. } = a {
        return plugin_look(plugin, action);
    }
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

/// (plugin, action) → (icon, colour, live state looks).
type PluginLooks = std::collections::BTreeMap<(String, String), (Option<String>, Color, Vec<LookState>)>;

/// How each installed plugin's actions look on a fresh key.
static PLUGIN_LOOKS: std::sync::RwLock<PluginLooks> =
    std::sync::RwLock::new(std::collections::BTreeMap::new());

/// Read the installed plugins' action icons and colours, for [`default_look`] (the app calls this
/// whenever the plugins change).
pub fn set_plugin_looks<'a>(manifests: impl IntoIterator<Item = &'a crate::plugin::Manifest>) {
    let mut map = std::collections::BTreeMap::new();
    for m in manifests {
        let fg = m.rgb().map_or(Color::WHITE, |[r, g, b]| Color([r, g, b, 255]));
        for a in &m.actions {
            let icon = a.icon.clone().filter(|i| crate::plugin::is_asset(i));
            let states = a.states.iter().map(|st| st.to_look(&m.id)).collect();
            map.insert((m.id.clone(), a.id.clone()), (icon, fg, states));
        }
    }
    if let Ok(mut looks) = PLUGIN_LOOKS.write() {
        *looks = map;
    }
}

/// The live looks plugin `plugin` declares for `action` (empty when it declares none or is not
/// installed): what a key with that action uses when it has no looks of its own.
pub fn plugin_states(plugin: &str, action: &str) -> Vec<LookState> {
    PLUGIN_LOOKS
        .read()
        .ok()
        .and_then(|l| {
            l.get(&(plugin.to_string(), action.to_string()))
                .map(|e| e.2.clone())
        })
        .unwrap_or_default()
}

/// A plugin action's fresh look: its icon in the plugin's colour, no label (the user names the
/// key); plain when the plugin is not installed or has no icon for it.
fn plugin_look(plugin: &str, action: &str) -> KeyLook {
    let found = PLUGIN_LOOKS
        .read()
        .ok()
        .and_then(|l| l.get(&(plugin.to_string(), action.to_string())).cloned());
    match found {
        // The action's live looks are not copied: the key uses the plugin's current ones.
        Some((icon, fg, _)) => KeyLook {
            source: icon.map_or(ImageSource::None, |icon| ImageSource::Plugin {
                plugin: plugin.to_string(),
                icon,
            }),
            foreground: fg,
            ..KeyLook::default()
        },
        None => KeyLook::default(),
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
    if look.states == was.states {
        look.states = now.states;
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

pub fn pretty_combo(keys: &str) -> String {
    keys.split('+')
        .map(|k| {
            let k = k.trim();
            match k.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => "Ctrl".into(),
                "alt" => "Alt".into(),
                "shift" => "Shift".into(),
                "super" | "meta" | "win" | "hyper" => "Super".into(),
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
        Action::Seek { seconds, .. } if *seconds < 0 => format!("Back {} s", -seconds),
        Action::Seek { seconds, .. } => format!("Forward {seconds} s"),
        Action::SwitchOutput { devices } if devices.is_empty() => "Next output".into(),
        Action::SwitchOutput { devices } => format!("Next of {} outputs", devices.len()),
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
        Action::Macro { own: Some(m), .. } => m.name.clone(),
        Action::Macro { .. } => "Macro".into(),
        Action::Cycle { steps, .. } => match steps.len() {
            1 => "1 step".into(),
            n => format!("{n} steps, one per press"),
        },
        _ => title.into(),
    }
}

/// What [`problem`] checks against.
#[derive(Clone, Copy)]
pub struct Problems<'a> {
    pub app_installed: &'a dyn Fn(&str) -> bool,
    pub file_exists: &'a dyn Fn(&str) -> bool,
    /// Pages of the profile the key is in.
    pub pages: &'a [Uuid],
    /// Every profile; empty means "don't check".
    pub profiles: &'a [Uuid],
    /// Why a plugin action can't work (plugin id, action id); `None` when it looks fine.
    pub plugin: &'a dyn Fn(&str, &str) -> Option<String>,
    /// Whether a macro is in the library.
    pub macro_exists: &'a dyn Fn(Uuid) -> bool,
}

/// Why this action can't work as it is, in words for a warning badge; `None` when it looks fine.
pub fn problem(a: &Action, c: &Problems) -> Option<String> {
    let blank = |s: &str| s.trim().is_empty();
    match a {
        Action::Command { cmd, .. } if blank(cmd) => Some("No command entered".into()),
        Action::LaunchApp {
            desktop_id: Some(id), ..
        } if !(c.app_installed)(id) => Some(format!("“{}” isn’t installed", id.trim_end_matches(".desktop"))),
        Action::LaunchApp {
            desktop_id: None,
            exec: None,
        } => Some("No app chosen".into()),
        Action::OpenUrl { url } if blank(url) => Some("No address entered".into()),
        Action::Shortcut { keys } if blank(keys) => Some("No keys recorded".into()),
        Action::TypeText { text } if text.is_empty() => Some("No text entered".into()),
        Action::PlaySound { path, .. } if blank(path) || !(c.file_exists)(path) => {
            Some("The sound file is missing".into())
        }
        Action::GoToPage { page } | Action::OpenFolder { page } if !c.pages.contains(page) => {
            Some("That page no longer exists".into())
        }
        Action::SwitchProfile { profile } if !c.profiles.is_empty() && !c.profiles.contains(profile) => {
            Some("That profile no longer exists".into())
        }
        Action::Macro { id, own: None } if id.is_nil() => Some("No macro made yet".into()),
        Action::Macro { id, own: None } if !(c.macro_exists)(*id) => {
            Some("This macro no longer exists".into())
        }
        Action::Multi { steps } => steps.iter().find_map(|s| problem(&s.action, c)),
        Action::Cycle { steps, .. } => steps.iter().find_map(|s| problem(&s.action, c)),
        Action::Plugin { plugin, action, .. } => (c.plugin)(plugin, action),
        _ => None,
    }
}
