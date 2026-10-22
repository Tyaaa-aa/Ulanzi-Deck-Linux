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
    p.dials[1].right = every_action().into_iter().nth(14);
    p.screen = ScreenConfig::Custom {
        widgets: vec![Widget::NowPlaying],
        tap: None,
        hold: Some(Action::NextPage),
    };
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
