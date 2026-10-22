use std::path::{Path, PathBuf};

use ulanzi_core::migrate::migrate_legacy;
use ulanzi_core::model::*;
use ulanzi_core::store::Store;
use ulanzi_core::{Color, CoreError};

struct Env {
    _dir: tempfile::TempDir,
    yaml: PathBuf,
    store: Store,
}

/// Copy the fixture config + icons into a temp "~/.config/ulanzi".
fn env() -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let legacy = dir.path().join("legacy");
    std::fs::create_dir_all(legacy.join("icons")).expect("mkdir");
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(fx.join("legacy-config.yaml"), legacy.join("config.yaml")).expect("copy yaml");
    for f in ["red.png", "speaker.png"] {
        std::fs::copy(fx.join("icons").join(f), legacy.join("icons").join(f)).expect("copy icon");
    }
    let store = Store::at(
        dir.path().join("config"),
        dir.path().join("state"),
        dir.path().join("cache"),
    );
    Env {
        yaml: legacy.join("config.yaml"),
        store,
        _dir: dir,
    }
}

fn key(p: &Profile, i: u8) -> &Key {
    p.pages[0]
        .keys
        .get(&i)
        .unwrap_or_else(|| panic!("key {i} missing"))
}

#[test]
fn migrates_fixture() {
    let e = env();
    let report = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let p = e.store.load_profile(report.profile_id).expect("profile saved");
    assert_eq!(p.name, "Default");
    assert_eq!(p.model, Model::D200X);
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.pages[0].name, "Main");
    assert_eq!(report.keys, 13);
    assert_eq!(p.pages[0].keys.len(), 13);

    assert_eq!(
        key(&p, 0).look.source,
        ImageSource::Icon {
            set: IconSet::Lucide,
            name: "monitor".into()
        }
    );
    assert_eq!(key(&p, 0).look.background, "#2a0405".parse::<Color>().expect("c"));
    assert_eq!(key(&p, 0).look.foreground, "#b03133".parse::<Color>().expect("c"));
    assert_eq!(key(&p, 0).look.icon_scale, 0.70);
    assert_eq!(key(&p, 0).look.label.text, "screen off");
    assert_eq!(
        key(&p, 0).tap,
        Some(Action::Command {
            cmd: "echo screen-off".into(),
            terminal: false
        })
    );
    assert_eq!(
        key(&p, 1).tap,
        Some(Action::Shortcut {
            keys: "super+F12".into()
        })
    );
    assert_eq!(
        key(&p, 3).look.source,
        ImageSource::Image {
            path: "icons/red.png".into()
        }
    );
    assert_eq!(
        key(&p, 6).tap,
        Some(Action::Media {
            control: MediaControl::PlayPause,
            player: None
        })
    );
    assert_eq!(
        key(&p, 7).tap,
        Some(Action::Media {
            control: MediaControl::Next,
            player: Some("spotify".into())
        })
    );
    assert_eq!(
        key(&p, 9).tap,
        Some(Action::LaunchApp {
            desktop_id: None,
            exec: Some("firefox".into())
        })
    );
    assert_eq!(
        key(&p, 11).tap,
        Some(Action::Obs(ObsAction::ToggleScenes {
            a: "Gaming".into(),
            b: "Desktop".into()
        }))
    );
    assert_eq!(
        key(&p, 12).tap,
        Some(Action::PlaySound {
            path: "/x/y.mp3".into(),
            device: None
        })
    );

    assert_eq!(
        p.dials[1].right,
        Some(Action::Volume {
            target: AudioTarget::App {
                name: "Discord".into()
            },
            op: VolumeOp::Up,
            step: 4,
            limit_100: true
        })
    );
    assert_eq!(
        p.dials[0].press,
        Some(Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Mute,
            step: 2,
            limit_100: true
        })
    );
    assert_eq!(
        p.dials[2].left,
        Some(Action::Volume {
            target: AudioTarget::Device {
                name: "alsa_output.usb".into()
            },
            op: VolumeOp::Down,
            step: 4,
            limit_100: true
        })
    );
    assert_eq!(p.dials[2].press, None);

    assert_eq!(
        p.screen,
        ScreenConfig::Custom {
            widgets: vec![Widget::NowPlaying],
            tap: Some(Action::Media {
                control: MediaControl::PlayPause,
                player: None
            }),
            hold: None
        }
    );
    assert_eq!(p.small_buttons.left, Some(Action::PrevPage));
    assert_eq!(p.small_buttons.right, Some(Action::NextPage));

    let s = e.store.load_settings().expect("settings");
    assert_eq!(s.brightness, 10);
    assert!(s.hide_labels);
    assert_eq!(s.sleep_after_min, 0);
    assert_eq!(s.obs.port, 4444);
    assert_eq!(s.default_profile, Some(p.id));
    assert!(report.warnings.iter().any(|w| w.contains("4455")));
}

#[test]
fn copies_custom_images() {
    let e = env();
    let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let icons = e.store.profile_dir(r.profile_id).join("icons");
    assert!(icons.join("red.png").is_file());
    assert!(icons.join("speaker.png").is_file());
}

#[test]
fn missing_image_warns() {
    let e = env();
    let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let p = e.store.load_profile(r.profile_id).expect("profile");
    assert_eq!(key(&p, 8).look.source, ImageSource::None);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("key 8") && w.contains("missing.png")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn legacy_file_untouched() {
    let e = env();
    let before = std::fs::read(&e.yaml).expect("read");
    let mtime = std::fs::metadata(&e.yaml)
        .expect("meta")
        .modified()
        .expect("mtime");
    migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    assert_eq!(std::fs::read(&e.yaml).expect("read"), before);
    assert_eq!(
        std::fs::metadata(&e.yaml)
            .expect("meta")
            .modified()
            .expect("mtime"),
        mtime
    );
}

#[test]
fn second_run_refuses_without_force() {
    let e = env();
    migrate_legacy(&e.yaml, &e.store, false).expect("first");
    assert!(matches!(
        migrate_legacy(&e.yaml, &e.store, false),
        Err(CoreError::AlreadyMigrated)
    ));
    migrate_legacy(&e.yaml, &e.store, true).expect("forced");
}

#[test]
fn clock_modes_map() {
    for (mode, expected) in [
        (0, ScreenConfig::FirmwareStats),
        (1, ScreenConfig::FirmwareClock),
        (
            2,
            ScreenConfig::Custom {
                widgets: vec![Widget::Blank],
                tap: None,
                hold: None,
            },
        ),
    ] {
        let e = env();
        let text = std::fs::read_to_string(&e.yaml)
            .expect("read")
            .replace("clock_mode: 3", &format!("clock_mode: {mode}"));
        // Button 13 only feeds the Now Playing screen's tap; drop it for the other modes.
        let text = text.replace(
            "- label: Clock Button\n  action: media\n  params:\n    control: play_pause\n",
            "- null\n",
        );
        std::fs::write(&e.yaml, text).expect("write");
        let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
        assert_eq!(
            e.store.load_profile(r.profile_id).expect("p").screen,
            expected,
            "clock_mode {mode}"
        );
    }
}
