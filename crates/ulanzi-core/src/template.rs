//! Profile templates: a few generic ones built in, plus any `.ulanzideck` the user keeps in
//! `~/.local/share/ulanzi-deck/templates/` (where "Save as Template" puts them).

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::Color;
use crate::bundle::{self, Bundle, BundleError, BundleProfile, Manifest, ManifestProfile};
use crate::model::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateSource {
    /// One of the templates that ship with the app (its id).
    BuiltIn(&'static str),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub source: TemplateSource,
}

/// `$XDG_DATA_HOME/ulanzi-deck/templates`.
pub fn user_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("ulanzi-deck").join("templates"))
}

/// Built-in templates, then the user's, sorted by name.
pub fn list() -> Vec<Template> {
    list_in(user_dir().as_deref())
}

/// [`list`] with the user's folder given (for tests).
pub fn list_in(user: Option<&Path>) -> Vec<Template> {
    let mut out: Vec<Template> = BUILT_IN
        .iter()
        .map(|(id, name, description, _)| Template {
            name: (*name).into(),
            description: (*description).into(),
            source: TemplateSource::BuiltIn(id),
        })
        .collect();
    let mut mine: Vec<Template> = user
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ulanzideck"))
        .filter_map(|p| {
            let m = bundle::read_manifest(&std::fs::read(&p).ok()?).ok()?;
            let first = m.profiles.first()?;
            Some(Template {
                name: first.name.clone(),
                description: format!(
                    "Your template · {} page{}",
                    first.pages,
                    if first.pages == 1 { "" } else { "s" }
                ),
                source: TemplateSource::File(p),
            })
        })
        .collect();
    mine.sort_by_key(|t| t.name.to_lowercase());
    out.extend(mine);
    out
}

/// A ready-to-import copy of the template with a fresh profile id.
pub fn instantiate(t: &Template) -> Result<Bundle, BundleError> {
    let mut b = match &t.source {
        TemplateSource::BuiltIn(id) => {
            let make = BUILT_IN
                .iter()
                .find(|(i, ..)| i == id)
                .map(|(.., f)| *f)
                .ok_or_else(|| BundleError::Io(format!("no template {id}")))?;
            let p = make();
            Bundle {
                manifest: Manifest {
                    format: bundle::FORMAT,
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    created: String::new(),
                    includes_settings: false,
                    min_app: None,
                    profiles: vec![ManifestProfile {
                        id: p.id,
                        name: p.name.clone(),
                        model: p.model,
                        pages: p.pages.len() as u32,
                        keys: p.pages.iter().map(|pg| pg.keys.len() as u32).sum(),
                    }],
                },
                macros: Default::default(),
                profiles: vec![BundleProfile {
                    profile: p,
                    icons: Vec::new(),
                }],
                settings: None,
            }
        }
        TemplateSource::File(path) => bundle::read(&std::fs::read(path)?)?,
    };
    for bp in &mut b.profiles {
        bp.profile.id = Uuid::new_v4();
        for pg in &mut bp.profile.pages {
            let new = Uuid::new_v4();
            for f in &mut bp.profile.foreign {
                if f.page == Some(pg.id) {
                    f.page = Some(new);
                }
            }
            pg.id = new;
        }
    }
    for (mp, bp) in b.manifest.profiles.iter_mut().zip(&b.profiles) {
        mp.id = bp.profile.id;
    }
    b.settings = None;
    Ok(b)
}

type Make = fn() -> Profile;

/// (id, name, description, builder)
const BUILT_IN: &[(&str, &str, &str, Make)] = &[
    (
        "streaming",
        "Streaming",
        "Scenes, recording, going live, mic and sound for OBS Studio",
        streaming,
    ),
    (
        "media-desk",
        "Media Desk",
        "Music, volume, screenshots and locking the screen",
        media_desk,
    ),
    (
        "video-editing",
        "Video Editing",
        "Shortcuts most video editors share. Change any key to match yours.",
        video_editing,
    ),
    (
        "productivity",
        "Productivity",
        "Copy and paste, windows, screenshots and quick launchers",
        productivity,
    ),
];

// ---- building blocks ----

fn c(hex: &str) -> Color {
    hex.parse().unwrap_or(Color::WHITE)
}

const BLUE: &str = "#62a0ea";
const GREEN: &str = "#57e389";
const ORANGE: &str = "#ffa348";
const RED: &str = "#ff7b63";
const PURPLE: &str = "#c061cb";
const YELLOW: &str = "#f8e45c";
const GREY: &str = "#deddda";

fn look(icon: &str, set: IconSet, label: &str, colour: &str) -> KeyLook {
    KeyLook {
        source: ImageSource::Icon {
            set,
            name: icon.into(),
        },
        background: Color::BLACK,
        foreground: c(colour),
        icon_scale: 0.62,
        label: Label {
            text: label.into(),
            visible: true,
            position: None,
        },
        dynamic: true,
        states: Vec::new(),
    }
}

fn key(icon: &str, label: &str, colour: &str, action: Action) -> Key {
    Key {
        look: look(icon, IconSet::Lucide, label, colour),
        tap: Some(action),
        hold: None,
    }
}

fn brand(icon: &str, label: &str, colour: &str, action: Action) -> Key {
    Key {
        look: look(icon, IconSet::Brands, label, colour),
        tap: Some(action),
        hold: None,
    }
}

fn sc(keys: &str) -> Action {
    Action::Shortcut { keys: keys.into() }
}

fn cmd(c: &str) -> Action {
    Action::Command {
        cmd: c.into(),
        terminal: false,
    }
}

fn media(control: MediaControl) -> Action {
    Action::Media {
        control,
        player: None,
    }
}

fn volume(target: AudioTarget, op: VolumeOp) -> Action {
    Action::Volume {
        target,
        op,
        step: 5,
        limit_100: true,
    }
}

fn scene(name: &str) -> Action {
    Action::obs("set_scene", &[("scene", name)])
}

fn profile(name: &str, keys: Vec<Key>, dials: [Dial; 3], screen: ScreenConfig) -> Profile {
    let mut p = Profile::new_default(name, Model::D200X);
    for (i, k) in keys.into_iter().enumerate() {
        p.pages[0].keys.insert(i as u8, k);
    }
    p.pages[0].dials = dials;
    p.screen = screen;
    p
}

fn volume_dial(target: AudioTarget) -> Dial {
    Dial {
        left: Some(volume(target.clone(), VolumeOp::Down)),
        press: Some(volume(target.clone(), VolumeOp::Mute)),
        right: Some(volume(target, VolumeOp::Up)),
        hold: None,
    }
}

fn brightness_dial() -> Dial {
    Dial {
        left: Some(Action::Brightness {
            op: BrightnessOp::Down { step: 10 },
        }),
        press: Some(Action::Brightness {
            op: BrightnessOp::Set { percent: 100 },
        }),
        right: Some(Action::Brightness {
            op: BrightnessOp::Up { step: 10 },
        }),
        hold: Some(Action::SleepNow),
    }
}

fn track_dial() -> Dial {
    Dial {
        left: Some(media(MediaControl::Previous)),
        press: Some(media(MediaControl::PlayPause)),
        right: Some(media(MediaControl::Next)),
        hold: None,
    }
}

fn now_playing() -> ScreenConfig {
    ScreenConfig::custom(
        vec![Widget::now_playing()],
        Some(media(MediaControl::PlayPause)),
        Some(media(MediaControl::Next)),
    )
}

// ---- the templates ----

fn streaming() -> Profile {
    profile(
        "Streaming",
        vec![
            key("clock", "Starting", PURPLE, scene("Starting Soon")),
            key("gamepad-2", "Main", PURPLE, scene("Main")),
            key("coffee", "Be Right Back", PURPLE, scene("Be Right Back")),
            key("flag", "Ending", PURPLE, scene("Ending")),
            brand(
                "obsstudio",
                "OBS",
                GREY,
                Action::LaunchApp {
                    desktop_id: Some("com.obsproject.Studio.desktop".into()),
                    exec: None,
                },
            ),
            key("radio", "Go Live", RED, Action::obs("toggle_stream", &[])),
            key("circle-dot", "Record", RED, Action::obs("toggle_record", &[])),
            key(
                "video",
                "Camera",
                BLUE,
                Action::obs("toggle_source", &[("scene", "Main"), ("source", "Camera")]),
            ),
            key(
                "mic-off",
                "Mic",
                ORANGE,
                volume(AudioTarget::Input, VolumeOp::Mute),
            ),
            key(
                "volume-x",
                "Sound",
                ORANGE,
                volume(AudioTarget::Default, VolumeOp::Mute),
            ),
            key("play", "Music", GREEN, media(MediaControl::PlayPause)),
            key("skip-forward", "Next Song", GREEN, media(MediaControl::Next)),
            key(
                "message-square",
                "Chat",
                BLUE,
                Action::OpenUrl {
                    url: "https://dashboard.twitch.tv/".into(),
                },
            ),
        ],
        [
            volume_dial(AudioTarget::Default),
            volume_dial(AudioTarget::Input),
            brightness_dial(),
        ],
        ScreenConfig::FirmwareClock {
            style: ClockStyle::Large,
        },
    )
}

fn media_desk() -> Profile {
    profile(
        "Media Desk",
        vec![
            key("skip-back", "Previous", GREEN, media(MediaControl::Previous)),
            key("play", "Play / Pause", GREEN, media(MediaControl::PlayPause)),
            key("skip-forward", "Next", GREEN, media(MediaControl::Next)),
            key("square", "Stop", GREEN, media(MediaControl::Stop)),
            key(
                "volume-x",
                "Mute",
                ORANGE,
                volume(AudioTarget::Default, VolumeOp::Mute),
            ),
            key(
                "volume-1",
                "Volume −",
                ORANGE,
                volume(AudioTarget::Default, VolumeOp::Down),
            ),
            key(
                "volume-2",
                "Volume +",
                ORANGE,
                volume(AudioTarget::Default, VolumeOp::Up),
            ),
            key(
                "mic-off",
                "Mic",
                ORANGE,
                volume(AudioTarget::Input, VolumeOp::Mute),
            ),
            key("camera", "Screenshot", BLUE, sc("print")),
            key("lock", "Lock", GREY, cmd("loginctl lock-session")),
            key(
                "sun",
                "Brighter",
                YELLOW,
                Action::Brightness {
                    op: BrightnessOp::Up { step: 20 },
                },
            ),
            key(
                "moon",
                "Dimmer",
                YELLOW,
                Action::Brightness {
                    op: BrightnessOp::Down { step: 20 },
                },
            ),
            key("power", "Deck Sleep", GREY, Action::SleepNow),
        ],
        [volume_dial(AudioTarget::Default), track_dial(), brightness_dial()],
        now_playing(),
    )
}

fn video_editing() -> Profile {
    profile(
        "Video Editing",
        vec![
            key("play", "Play / Stop", BLUE, sc("space")),
            key("rewind", "Back J", BLUE, sc("j")),
            key("square", "Stop K", BLUE, sc("k")),
            key("fast-forward", "Forward L", BLUE, sc("l")),
            key("undo-2", "Undo", GREY, sc("ctrl+z")),
            key("log-in", "Mark In", YELLOW, sc("i")),
            key("log-out", "Mark Out", YELLOW, sc("o")),
            key("step-back", "Frame ◀", BLUE, sc("left")),
            key("step-forward", "Frame ▶", BLUE, sc("right")),
            key("redo-2", "Redo", GREY, sc("ctrl+shift+z")),
            key("skip-back", "Start", BLUE, sc("home")),
            key("skip-forward", "End", BLUE, sc("end")),
            key("save", "Save", GREEN, sc("ctrl+s")),
        ],
        [
            Dial {
                left: Some(sc("left")),
                press: Some(sc("space")),
                right: Some(sc("right")),
                hold: Some(sc("home")),
            },
            Dial {
                left: Some(sc("up")),
                press: Some(sc("ctrl+z")),
                right: Some(sc("down")),
                hold: None,
            },
            Dial {
                left: Some(sc("minus")),
                press: None,
                right: Some(sc("equal")),
                hold: None,
            },
        ],
        ScreenConfig::FirmwareClock {
            style: ClockStyle::DateWeekday,
        },
    )
}

fn productivity() -> Profile {
    profile(
        "Productivity",
        vec![
            key("copy", "Copy", BLUE, sc("ctrl+c")),
            key("clipboard-paste", "Paste", BLUE, sc("ctrl+v")),
            key("scissors", "Cut", BLUE, sc("ctrl+x")),
            key("undo-2", "Undo", GREY, sc("ctrl+z")),
            key("redo-2", "Redo", GREY, sc("ctrl+shift+z")),
            key("terminal", "Terminal", GREEN, sc("ctrl+alt+t")),
            key("folder", "Files", GREEN, cmd("xdg-open ~")),
            key(
                "globe",
                "Browser",
                GREEN,
                Action::OpenUrl {
                    url: "https://start.duckduckgo.com".into(),
                },
            ),
            key("camera", "Screenshot", PURPLE, sc("print")),
            key("monitor", "Desktop", PURPLE, sc("super+d")),
            key("app-window", "Switch Window", PURPLE, sc("alt+tab")),
            key("x", "Close Window", RED, sc("alt+F4")),
            key("lock", "Lock", GREY, cmd("loginctl lock-session")),
        ],
        [volume_dial(AudioTarget::Default), track_dial(), brightness_dial()],
        ScreenConfig::FirmwareClock {
            style: ClockStyle::DateWeekday,
        },
    )
}
