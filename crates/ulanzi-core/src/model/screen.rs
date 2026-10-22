//! What the wide screen shows when it is in widget mode.

use serde::{Deserialize, Deserializer, Serialize};

use super::{Action, MediaControl, TimerOp};
use crate::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockFace {
    #[default]
    Digital,
    Analog,
    Minimal,
    Split,
}

impl ClockFace {
    pub const ALL: [ClockFace; 4] = [
        ClockFace::Digital,
        ClockFace::Analog,
        ClockFace::Minimal,
        ClockFace::Split,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ClockFace::Digital => "Digital",
            ClockFace::Analog => "Analogue",
            ClockFace::Minimal => "Minimal",
            ClockFace::Split => "Split",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stat {
    Cpu,
    Ram,
    Gpu,
    Temp,
    Net,
}

impl Stat {
    pub const ALL: [Stat; 5] = [Stat::Cpu, Stat::Ram, Stat::Gpu, Stat::Temp, Stat::Net];

    pub fn title(self) -> &'static str {
        match self {
            Stat::Cpu => "CPU",
            Stat::Ram => "Memory",
            Stat::Gpu => "GPU",
            Stat::Temp => "Temperature",
            Stat::Net => "Network",
        }
    }
}

fn yes() -> bool {
    true
}
fn twenty_five() -> u32 {
    25
}
fn thirty() -> u32 {
    30
}
fn ten() -> u32 {
    10
}
fn default_stats() -> Vec<Stat> {
    vec![Stat::Cpu, Stat::Ram, Stat::Temp]
}

/// One widget on the wide screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Widget {
    Blank,
    Clock {
        #[serde(default)]
        face: ClockFace,
        #[serde(default = "yes")]
        hour24: bool,
        #[serde(default)]
        seconds: bool,
        #[serde(default = "yes")]
        date: bool,
    },
    NowPlaying {
        #[serde(default = "yes")]
        progress: bool,
        #[serde(default)]
        blur_background: bool,
    },
    System {
        #[serde(default = "default_stats")]
        stats: Vec<Stat>,
        #[serde(default = "yes")]
        graphs: bool,
    },
    Timer {
        #[serde(default = "twenty_five")]
        minutes: u32,
        #[serde(default)]
        pomodoro: bool,
    },
    Weather {
        #[serde(default)]
        place: String,
        #[serde(default)]
        fahrenheit: bool,
    },
    Command {
        #[serde(default)]
        cmd: String,
        #[serde(default = "thirty")]
        every_s: u32,
        #[serde(default)]
        title: String,
    },
    Pictures {
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default = "ten")]
        every_s: u32,
    },
}

impl Widget {
    pub fn now_playing() -> Widget {
        Widget::NowPlaying {
            progress: true,
            blur_background: false,
        }
    }

    pub fn clock() -> Widget {
        Widget::Clock {
            face: ClockFace::Digital,
            hour24: true,
            seconds: false,
            date: true,
        }
    }

    /// A new widget of each kind, for the "Add Widget" menu.
    pub fn all_kinds() -> Vec<Widget> {
        vec![
            Widget::clock(),
            Widget::now_playing(),
            Widget::System {
                stats: default_stats(),
                graphs: true,
            },
            Widget::Timer {
                minutes: 25,
                pomodoro: false,
            },
            Widget::Weather {
                place: String::new(),
                fahrenheit: false,
            },
            Widget::Command {
                cmd: String::new(),
                every_s: 30,
                title: String::new(),
            },
            Widget::Pictures {
                paths: Vec::new(),
                every_s: 10,
            },
            Widget::Blank,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            Widget::Blank => "Blank",
            Widget::Clock { .. } => "Clock",
            Widget::NowPlaying { .. } => "Now Playing",
            Widget::System { .. } => "System",
            Widget::Timer { .. } => "Timer",
            Widget::Weather { .. } => "Weather",
            Widget::Command { .. } => "Command Output",
            Widget::Pictures { .. } => "Pictures",
        }
    }

    /// Lucide icon for lists.
    pub fn icon(&self) -> &'static str {
        match self {
            Widget::Blank => "square",
            Widget::Clock { .. } => "clock",
            Widget::NowPlaying { .. } => "music",
            Widget::System { .. } => "cpu",
            Widget::Timer { .. } => "timer",
            Widget::Weather { .. } => "cloud-sun",
            Widget::Command { .. } => "terminal",
            Widget::Pictures { .. } => "image",
        }
    }

    /// What tapping the screen does with this widget showing, unless the profile says otherwise.
    pub fn default_tap(&self) -> Option<Action> {
        match self {
            Widget::NowPlaying { .. } => Some(Action::Media {
                control: MediaControl::PlayPause,
                player: None,
            }),
            Widget::Timer { .. } => Some(Action::Timer {
                op: TimerOp::StartPause,
            }),
            _ => None,
        }
    }
}

/// Widgets saved before they had options were plain names (`"now_playing"`).
pub(crate) fn widgets_compat<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Widget>, D::Error> {
    let raw: Vec<toml::Value> = Vec::deserialize(d)?;
    raw.into_iter()
        .map(|v| {
            let v = match v {
                toml::Value::String(name) => {
                    let mut t = toml::Table::new();
                    t.insert("kind".into(), toml::Value::String(name));
                    toml::Value::Table(t)
                }
                other => other,
            };
            v.try_into::<Widget>().map_err(serde::de::Error::custom)
        })
        .collect()
}

/// When the screen moves to the next widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Rotation {
    #[default]
    Off,
    Every {
        secs: u32,
    },
    OnPress,
}

/// What is behind the widget.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Background {
    /// A dark backdrop that suits every widget.
    #[default]
    Theme,
    Solid {
        color: Color,
    },
    Gradient {
        from: Color,
        to: Color,
    },
    /// A picture in the profile's folder, dimmed so text stays readable.
    Picture {
        path: String,
    },
}

pub fn default_accent() -> Color {
    Color::rgb(0x62, 0xa0, 0xea)
}
