//! Fill an empty configuration folder with the showcase profiles used for screenshots and the
//! guided test: the built-in templates, each with a widget screen.
//!
//! `cargo run -p ulanzi-core --example showcase -- <sandbox>` writes `<sandbox>/config`,
//! `<sandbox>/state` and `<sandbox>/cache` (point `XDG_CONFIG_HOME` etc. there).

use ulanzi_core::Color;
use ulanzi_core::model::*;
use ulanzi_core::store::Store;
use ulanzi_core::template;

fn main() {
    let Some(sandbox) = std::env::args().nth(1) else {
        eprintln!("usage: showcase <sandbox-dir>");
        std::process::exit(2);
    };
    let sandbox = std::path::PathBuf::from(sandbox);
    let root = sandbox.join("config/ulanzi-deck");
    let state = sandbox.join("state/ulanzi-deck");
    let store = Store::at(&root, &state, sandbox.join("cache/ulanzi-deck"));
    let mut order = Vec::new();
    let mut default = None;
    for t in template::list_in(None) {
        let mut b = template::instantiate(&t).expect("built-in template");
        let mut p = b.profiles.remove(0).profile;
        let (widgets, rotate, background, accent) = match p.name.as_str() {
            "Streaming" => (
                vec![
                    Widget::Clock {
                        face: ClockFace::Split,
                        hour24: true,
                        seconds: false,
                        date: true,
                    },
                    Widget::System {
                        stats: vec![Stat::Cpu, Stat::Gpu, Stat::Ram, Stat::Net],
                        graphs: true,
                    },
                ],
                Rotation::Every { secs: 10 },
                Background::Gradient {
                    from: Color::rgb(0x1a, 0x10, 0x2e),
                    to: Color::rgb(0x4a, 0x1d, 0x5c),
                },
                Color::rgb(0xc0, 0x61, 0xcb),
            ),
            "Media Desk" => (
                vec![
                    Widget::NowPlaying {
                        progress: true,
                        blur_background: true,
                    },
                    Widget::Clock {
                        face: ClockFace::Analog,
                        hour24: true,
                        seconds: true,
                        date: true,
                    },
                ],
                Rotation::OnPress,
                Background::Gradient {
                    from: Color::rgb(0x10, 0x1c, 0x2e),
                    to: Color::rgb(0x1c, 0x3d, 0x5a),
                },
                Color::rgb(0x62, 0xa0, 0xea),
            ),
            "Video Editing" => (
                vec![
                    Widget::Timer {
                        minutes: 25,
                        pomodoro: true,
                    },
                    Widget::Clock {
                        face: ClockFace::Minimal,
                        hour24: true,
                        seconds: false,
                        date: true,
                    },
                ],
                Rotation::OnPress,
                Background::Solid {
                    color: Color::rgb(0x1e, 0x1e, 0x1e),
                },
                Color::rgb(0xff, 0x78, 0x00),
            ),
            _ => (
                vec![
                    Widget::Clock {
                        face: ClockFace::Digital,
                        hour24: true,
                        seconds: false,
                        date: true,
                    },
                    Widget::Weather {
                        place: "Oslo".into(),
                        fahrenheit: false,
                    },
                ],
                Rotation::Every { secs: 15 },
                Background::Theme,
                Color::rgb(0x2e, 0xc2, 0x7e),
            ),
        };
        p.screen = ScreenConfig::Custom {
            widgets,
            tap: None,
            hold: None,
            rotate,
            background,
            accent,
        };
        if p.name == "Media Desk" {
            default = Some(p.id);
        }
        order.push(p.id);
        store.save_profile(&p).expect("save profile");
    }
    let mut settings = store.load_settings().unwrap_or_default();
    settings.default_profile = default;
    settings.profile_order = order;
    store.save_settings(&settings).expect("save settings");
    // A finished first run: the welcome screens don't cover the screenshots.
    std::fs::create_dir_all(&state).expect("state dir");
    std::fs::write(state.join("onboarded"), b"1\n").expect("onboarding marker");
    println!("showcase profiles written to {}", root.display());
}
