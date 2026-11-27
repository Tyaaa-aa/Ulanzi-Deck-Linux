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
    let mut profiles = Vec::new();
    for t in template::list_in(None) {
        let mut b = template::instantiate(&t).expect("built-in template");
        profiles.push(b.profiles.remove(0).profile);
    }
    // Keys from the other templates, for Media Desk's key library.
    let borrowed: Vec<Key> = profiles
        .iter()
        .filter(|p| p.name != "Media Desk")
        .flat_map(|p| p.pages[0].keys.values().take(4).cloned())
        .collect();
    for mut p in profiles {
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
                        place: "Oslo, Norway".into(),
                        fahrenheit: false,
                        lat: Some(59.91),
                        lon: Some(10.75),
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
            // Music takes the screen over everywhere but on Media Desk (where it already shows).
            media: MediaTakeover {
                on: p.name != "Media Desk",
                back_after_s: 10,
            },
        };
        if p.name == "Media Desk" {
            default = Some(p.id);
            // One of each effect, on the page the screenshots show.
            let looks = [
                (EffectKind::Glow, Color::rgb(0x62, 0xa0, 0xea)),
                (EffectKind::Fire, Color::rgb(0xff, 0x78, 0x00)),
                (EffectKind::Chase, Color::rgb(0x2e, 0xc2, 0x7e)),
                (EffectKind::Pulse, Color::rgb(0xc0, 0x61, 0xcb)),
            ];
            for (key, (kind, color)) in p.pages[0].keys.values_mut().zip(looks) {
                key.look.effect = Effect {
                    color: Some(color),
                    ..Effect::of(kind)
                };
            }
            p = with_library(p, &borrowed);
        }
        // Auto-switch rules, so the profiles follow an app in front.
        p.r#match.apps = match p.name.as_str() {
            "Streaming" => vec!["com.obsproject.Studio.desktop".into()],
            "Video Editing" => vec!["org.kde.kdenlive.desktop".into()],
            _ => Vec::new(),
        };
        order.push(p.id);
        store.save_profile(&p).expect("save profile");
        let icon = match p.name.as_str() {
            "Streaming" => "radio",
            "Media Desk" => "music",
            "Video Editing" => "clapperboard",
            _ => "briefcase",
        };
        let look = ProfileLook {
            source: Some(ImageSource::Icon {
                set: IconSet::Lucide,
                name: icon.into(),
            }),
            color: Some(accent),
            from_app: None,
        };
        store.save_profile_look(p.id, &look).expect("save look");
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

/// A key library worth looking at: the playback keys linked from the first page, the rest of
/// that page's keys and some from the other templates as library keys, and a second page with
/// room to drag them onto.
fn with_library(p: Profile, borrowed: &[Key]) -> Profile {
    let mut e = ulanzi_core::edit::Editor::new(p);
    let keys: Vec<u8> = e.profile().pages[0].keys.keys().copied().collect();
    e.add_keys_to_library(&keys[..keys.len().min(4)]);
    for k in keys.iter().skip(4) {
        // Into the library and back off the page: a library key that is not placed yet.
        if e.add_key_to_library(*k).is_some() {
            e.own_copy_key(*k);
        }
    }
    for key in borrowed {
        let id = e.new_library_key();
        let name = key.look.label.text.clone();
        e.set_library_key(id, |k| *k = key.clone());
        if !name.is_empty() {
            e.rename_library_key(id, &name);
        }
    }
    let extras = e.add_page("Extras");
    e.set_page(extras);
    let lib: Vec<_> = e.profile().key_library.iter().map(|l| l.id).collect();
    for (n, id) in [0u8, 1, 5].into_iter().zip(lib.iter().skip(4)) {
        e.place_key(n, *id);
    }
    e.set_page(0);
    e.profile().clone()
}
