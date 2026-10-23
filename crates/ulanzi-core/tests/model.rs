use std::str::FromStr;

use ulanzi_core::model::*;
use ulanzi_core::store::{ProfileLoad, Store, write_atomic};
use ulanzi_core::{Color, CoreError};
use uuid::Uuid;

fn store() -> (tempfile::TempDir, Store) {
    let d = tempfile::tempdir().expect("tempdir");
    let s = Store::at(
        d.path().join("config"),
        d.path().join("state"),
        d.path().join("cache"),
    );
    (d, s)
}

fn every_action() -> Vec<Action> {
    let id = Uuid::new_v4();
    vec![
        Action::NextPage,
        Action::PrevPage,
        Action::GoToPage { page: id },
        Action::OpenFolder { page: id },
        Action::Back,
        Action::SwitchProfile { profile: id },
        Action::AutoProfile,
        Action::Command {
            cmd: "echo hi".into(),
            terminal: true,
        },
        Action::LaunchApp {
            desktop_id: Some("firefox.desktop".into()),
            exec: None,
        },
        Action::OpenUrl {
            url: "https://example.com".into(),
        },
        Action::Shortcut {
            keys: "super+F12".into(),
        },
        Action::TypeText { text: "hello".into() },
        Action::PlaySound {
            path: "/x/y.mp3".into(),
            device: Some("speakers".into()),
        },
        Action::Media {
            control: MediaControl::PlayPause,
            player: Some("spotify".into()),
        },
        Action::Volume {
            target: AudioTarget::App {
                name: "Discord".into(),
            },
            op: VolumeOp::Up,
            step: 4,
            limit_100: true,
        },
        Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Set { percent: 30 },
            step: 0,
            limit_100: false,
        },
        Action::Brightness {
            op: BrightnessOp::Down { step: 10 },
        },
        Action::SleepNow,
        Action::Multi {
            steps: vec![
                Step {
                    action: Action::NextPage,
                    delay_ms: 0,
                },
                Step {
                    action: Action::Shortcut {
                        keys: "ctrl+c".into(),
                    },
                    delay_ms: 150,
                },
            ],
        },
        Action::Obs(ObsAction::ToggleScenes {
            a: "Gaming".into(),
            b: "Desktop".into(),
        }),
        Action::Obs(ObsAction::ToggleRecord),
    ]
}

#[test]
fn settings_default_roundtrip() {
    let (_d, s) = store();
    let def = s.load_settings().expect("missing file gives defaults");
    assert_eq!(def, Settings::default());
    assert_eq!(def.obs.port, 4455);
    assert_eq!(def.hold_ms, 500);
    let mut changed = def.clone();
    changed.brightness = 10;
    changed.hide_labels = true;
    s.save_settings(&changed).expect("save");
    assert_eq!(s.load_settings().expect("load"), changed);
}

#[test]
fn profile_roundtrip_all_action_variants() {
    let (_d, s) = store();
    let mut p = Profile::new_default("Default", Model::D200X);
    for (i, a) in every_action().into_iter().enumerate() {
        let key = Key {
            look: KeyLook::default(),
            tap: Some(a.clone()),
            hold: Some(a),
        };
        p.pages[0].keys.insert((i % 13) as u8, key);
    }
    p.pages[0].keys.get_mut(&0).expect("key 0").look = KeyLook {
        source: ImageSource::Icon {
            set: IconSet::Lucide,
            name: "monitor".into(),
        },
        background: Color::from_str("#2a0405").expect("color"),
        foreground: Color::from_str("#b03133").expect("color"),
        icon_scale: 0.7,
        label: Label {
            text: "screen off".into(),
            visible: true,
            position: None,
        },
        dynamic: true,
    };
    p.pages[0].dials[1].right = every_action().into_iter().nth(14);
    p.screen = ScreenConfig::custom(vec![Widget::now_playing()], None, Some(Action::NextPage));
    s.save_profile(&p).expect("save");
    assert_eq!(s.load_profile(p.id).expect("load"), p);
}

#[test]
fn new_default_has_main_page_and_page_buttons() {
    let p = Profile::new_default("Default", Model::D200X);
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.pages[0].name, "Main");
    assert_eq!(p.small_buttons.left, Some(Action::PrevPage));
    assert_eq!(p.small_buttons.right, Some(Action::NextPage));
}

#[test]
fn color_parse_formats() {
    assert_eq!(Color::from_str("#1f0001").expect("rgb").0, [0x1f, 0, 1, 0xff]);
    assert_eq!(Color::from_str("#1F0001FF").expect("rgba").0, [0x1f, 0, 1, 0xff]);
    assert!(Color::from_str("#12").is_err());
    assert!(Color::from_str("1f0001").is_err());
    assert_eq!(Color([1, 2, 3, 255]).to_string(), "#010203");
    assert_eq!(Color([1, 2, 3, 4]).to_string(), "#01020304");
}

#[test]
fn normalize_repairs_empty_and_out_of_range() {
    let mut p = Profile::new_default("X", Model::D200X);
    p.pages.clear();
    let w = p.normalize();
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.pages[0].name, "Main");
    assert_eq!(w.len(), 1);

    p.pages[0].keys.insert(20, Key::default());
    let w = p.normalize();
    assert!(!p.pages[0].keys.contains_key(&20));
    assert!(w.iter().any(|m| m.contains("20")));
}

#[test]
fn invalid_profile_is_reported_not_fatal() {
    let (_d, s) = store();
    let good = Profile::new_default("Good", Model::D200X);
    s.save_profile(&good).expect("save");
    let bad = Uuid::new_v4();
    std::fs::create_dir_all(s.profile_dir(bad)).expect("mkdir");
    std::fs::write(s.profile_dir(bad).join("profile.toml"), "version = 1\nname = [").expect("write");

    let loads = s.list_profiles();
    assert_eq!(loads.len(), 2);
    assert!(
        loads
            .iter()
            .any(|l| matches!(l, ProfileLoad::Ok(p) if p.id == good.id))
    );
    let invalid = loads.iter().find_map(|l| match l {
        ProfileLoad::Invalid { path, error } => Some((path.clone(), error.clone())),
        ProfileLoad::Ok(_) => None,
    });
    let (path, error) = invalid.expect("invalid reported");
    assert!(
        path.file_name()
            .expect("name")
            .to_string_lossy()
            .starts_with("profile.toml.invalid-")
    );
    assert!(path.exists());
    assert!(!error.is_empty());
    assert!(!s.profile_dir(bad).join("profile.toml").exists());
}

#[test]
fn write_atomic_leaves_no_temp_on_success() {
    let d = tempfile::tempdir().expect("tempdir");
    let f = d.path().join("a.toml");
    write_atomic(&f, b"x = 1").expect("write");
    write_atomic(&f, b"x = 2").expect("overwrite");
    assert_eq!(std::fs::read_to_string(&f).expect("read"), "x = 2");
    let names: Vec<_> = std::fs::read_dir(d.path())
        .expect("ls")
        .map(|e| e.expect("e").file_name())
        .collect();
    assert_eq!(names.len(), 1);
}

#[test]
fn unknown_version_rejected() {
    let (_d, s) = store();
    let p = Profile::new_default("V", Model::D200);
    s.save_profile(&p).expect("save");
    let path = s.profile_dir(p.id).join("profile.toml");
    let text = std::fs::read_to_string(&path)
        .expect("read")
        .replacen("version = 1", "version = 2", 1);
    std::fs::write(&path, text).expect("write");
    let err = s.load_profile(p.id).expect_err("version 2 must fail");
    assert!(
        matches!(err, CoreError::UnsupportedVersion { found: 2, .. }),
        "{err}"
    );
}

#[test]
fn unknown_field_rejected() {
    let (_d, s) = store();
    let p = Profile::new_default("V", Model::D200);
    s.save_profile(&p).expect("save");
    let path = s.profile_dir(p.id).join("profile.toml");
    let text = std::fs::read_to_string(&path).expect("read") + "\nbrightnes = 5\n";
    std::fs::write(&path, text).expect("write");
    assert!(s.load_profile(p.id).is_err());
}

#[test]
fn settings_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, s) = store();
    s.save_settings(&Settings::default()).expect("save");
    let mode = std::fs::metadata(s.root().join("settings.toml"))
        .expect("meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "settings.toml can hold the OBS password");
}

#[test]
fn clock_style_defaults_and_roundtrips() {
    // Files written before clock styles existed still load (style defaults to the classic clock).
    let old: ScreenConfig = toml::from_str("mode = \"firmware_clock\"").expect("old format");
    assert_eq!(
        old,
        ScreenConfig::FirmwareClock {
            style: ClockStyle::Classic
        }
    );
    let new = ScreenConfig::FirmwareClock {
        style: ClockStyle::DateWeekday,
    };
    let text = toml::to_string(&new).expect("toml");
    assert_eq!(toml::from_str::<ScreenConfig>(&text).expect("back"), new);
    assert_eq!(ClockStyle::ALL.len(), 5);
}

#[test]
fn label_position_override_is_optional_and_clearable() {
    use ulanzi_core::model::{LabelPosition, Model, Profile};
    // Old files have no position.
    let l: ulanzi_core::model::Label = toml::from_str("text = \"Hi\"\nvisible = false").expect("parse");
    assert_eq!(l.position, None);
    let mut p = Profile::new_default("P", Model::D200X);
    let mut key = ulanzi_core::model::Key::default();
    key.look.label.position = Some(LabelPosition::Top);
    key.look.label.visible = false;
    p.pages[0].keys.insert(0, key);
    let text = toml::to_string(&p).expect("ser");
    assert!(text.contains("position = \"top\""), "{text}");
    p.clear_label_overrides();
    let l = &p.pages[0].keys[&0].look.label;
    assert_eq!((l.position, l.visible), (None, true));
}

#[test]
fn old_profile_wide_dials_move_onto_every_page() {
    use ulanzi_core::model::{Action, Model, Page, Profile};
    let mut old = Profile::new_default("P", Model::D200X);
    old.pages.push(Page::new("Two"));
    old.dials[1].press = Some(Action::NextPage);
    let text = toml::to_string(&old).expect("ser");
    let p = ulanzi_core::store::parse_profile(&text).expect("parse");
    assert!(
        p.pages
            .iter()
            .all(|pg| pg.dials[1].press == Some(Action::NextPage))
    );
    assert!(p.dials.iter().all(|d| *d == ulanzi_core::model::Dial::default()));
    // Saved again, the profile-wide table is gone.
    let again = toml::to_string(&p).expect("ser");
    assert!(!again.contains("[[dials]]"), "{again}");
}

#[test]
fn profiles_follow_the_saved_order() {
    use uuid::Uuid;
    let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let mut list = vec![(a, "A".to_string()), (b, "B".to_string()), (c, "C".to_string())];
    // c first, then a; b is not in the saved order so it goes last.
    ulanzi_core::model::order_profiles(&mut list, &[c, a]);
    assert_eq!(list.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![c, a, b]);
    let s: ulanzi_core::model::Settings = toml::from_str("").expect("defaults");
    assert!(s.profile_order.is_empty());
}

#[test]
fn old_widget_strings_still_parse() {
    use ulanzi_core::model::{ScreenConfig, Widget};
    let s: ScreenConfig =
        toml::from_str("mode = \"custom\"\nwidgets = [\"now_playing\", \"blank\"]\n").expect("old form");
    let ScreenConfig::Custom {
        widgets,
        rotate,
        accent,
        ..
    } = s
    else {
        panic!("custom");
    };
    assert!(matches!(widgets[0], Widget::NowPlaying { .. }));
    assert_eq!(widgets[1], Widget::Blank);
    assert_eq!(rotate, ulanzi_core::model::Rotation::Off);
    assert_eq!(accent, "#62a0ea".parse().expect("colour"));
}

#[test]
fn widgets_round_trip_with_options() {
    use ulanzi_core::model::*;
    let s = ScreenConfig::Custom {
        widgets: vec![
            Widget::Clock {
                face: ClockFace::Analog,
                hour24: false,
                seconds: true,
                date: true,
            },
            Widget::System {
                stats: vec![Stat::Cpu, Stat::Ram, Stat::Temp],
                graphs: true,
            },
            Widget::Timer {
                minutes: 25,
                pomodoro: true,
            },
            Widget::Weather {
                place: "Singapore".into(),
                fahrenheit: false,
            },
            Widget::Command {
                cmd: "uptime -p".into(),
                every_s: 30,
                title: "Uptime".into(),
            },
            Widget::Pictures {
                paths: vec!["icons/a.png".into()],
                every_s: 10,
            },
            Widget::now_playing(),
        ],
        tap: None,
        hold: Some(Action::ScreenNext),
        rotate: Rotation::Every { secs: 10 },
        background: Background::Gradient {
            from: "#1a1a2e".parse().expect("c"),
            to: "#16213e".parse().expect("c"),
        },
        accent: "#ff7b63".parse().expect("c"),
    };
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Wrap {
        screen: ScreenConfig,
    }
    let text = toml::to_string(&Wrap { screen: s.clone() }).expect("toml");
    let back: Wrap = toml::from_str(&text).expect("parse");
    assert_eq!(back.screen, s, "{text}");
}

#[test]
fn widget_defaults_and_actions() {
    use ulanzi_core::model::*;
    assert_eq!(
        Widget::now_playing().default_tap(),
        Some(Action::Media {
            control: MediaControl::PlayPause,
            player: None,
        })
    );
    assert_eq!(
        Widget::Timer {
            minutes: 25,
            pomodoro: false
        }
        .default_tap(),
        Some(Action::Timer {
            op: TimerOp::StartPause
        })
    );
    assert_eq!(Widget::Blank.title(), "Blank");
    let t: Action =
        toml::from_str("type = \"timer\"\nop = { kind = \"add\", minutes = 5 }\n").expect("timer action");
    assert_eq!(
        t,
        Action::Timer {
            op: TimerOp::Add { minutes: 5 }
        }
    );
}

#[test]
fn obs_actions_round_trip_and_old_ones_parse() {
    // Written by earlier versions: no `preview` field.
    let old: Key = toml::from_str("[tap]\nkind = \"obs\"\nobs = \"set_scene\"\nscene = \"Live\"\n")
        .expect("old set_scene");
    assert_eq!(
        old.tap,
        Some(Action::Obs(ObsAction::SetScene {
            scene: "Live".into(),
            preview: false
        }))
    );
    for a in [
        Action::Obs(ObsAction::InputVolume {
            input: "Mic/Aux".into(),
            op: VolumeOp::Down,
            step: 3,
        }),
        Action::Obs(ObsAction::ToggleFilter {
            source: "Cam".into(),
            filter: "Blur".into(),
        }),
        Action::Obs(ObsAction::PauseRecord),
        Action::Obs(ObsAction::Screenshot),
    ] {
        let k = Key {
            tap: Some(a),
            ..Key::default()
        };
        let text = toml::to_string(&k).expect("serialize");
        assert_eq!(toml::from_str::<Key>(&text).expect("parse"), k, "{text}");
    }
}
