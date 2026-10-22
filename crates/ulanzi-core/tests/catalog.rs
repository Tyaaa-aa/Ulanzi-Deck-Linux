use std::collections::HashSet;

use ulanzi_core::Color;
use ulanzi_core::catalog::{Category, catalog, default_look, describe, spec_for};
use ulanzi_core::model::*;
use uuid::Uuid;

fn samples() -> Vec<Action> {
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
            terminal: false,
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
        Action::TypeText { text: "hi".into() },
        Action::PlaySound {
            path: "/x.mp3".into(),
            device: None,
        },
        Action::Media {
            control: MediaControl::PlayPause,
            player: None,
        },
        Action::Media {
            control: MediaControl::Next,
            player: None,
        },
        Action::Media {
            control: MediaControl::Previous,
            player: None,
        },
        Action::Media {
            control: MediaControl::Stop,
            player: None,
        },
        Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Up,
            step: 5,
            limit_100: true,
        },
        Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Down,
            step: 5,
            limit_100: true,
        },
        Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Mute,
            step: 5,
            limit_100: true,
        },
        Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Set { percent: 50 },
            step: 5,
            limit_100: true,
        },
        Action::Brightness {
            op: BrightnessOp::Up { step: 10 },
        },
        Action::Brightness {
            op: BrightnessOp::Down { step: 10 },
        },
        Action::Brightness {
            op: BrightnessOp::Set { percent: 50 },
        },
        Action::SleepNow,
        Action::Multi { steps: vec![] },
        Action::Obs(ObsAction::SetScene { scene: "A".into() }),
        Action::Obs(ObsAction::ToggleScenes {
            a: "A".into(),
            b: "B".into(),
        }),
        Action::Obs(ObsAction::ToggleSource {
            scene: "A".into(),
            source: "Cam".into(),
        }),
        Action::Obs(ObsAction::ToggleRecord),
        Action::Obs(ObsAction::ToggleStream),
    ]
}

#[test]
fn every_variant_has_a_spec() {
    for a in samples() {
        assert!(spec_for(&a).is_some(), "no catalogue entry for {a:?}");
    }
}

#[test]
fn ids_unique() {
    let mut seen = HashSet::new();
    for s in catalog() {
        assert!(seen.insert(s.id), "duplicate id {}", s.id);
        assert!(
            ulanzi_core::render::IconLibrary::bundled()
                .svg(IconSet::Lucide, s.icon)
                .is_some(),
            "icon {} for {} is not bundled",
            s.icon,
            s.id
        );
    }
}

#[test]
fn default_look_uses_category_colour() {
    let look = default_look(&Action::NextPage);
    assert_eq!(look.foreground, Category::Navigation.color());
    assert_eq!(look.background, Color::BLACK);
    assert_eq!(look.label.text, "Next Page");
    assert!(matches!(
        look.source,
        ImageSource::Icon {
            set: IconSet::Lucide,
            ..
        }
    ));
}

#[test]
fn describe_examples() {
    let v = Action::Volume {
        target: AudioTarget::App {
            name: "Discord".into(),
        },
        op: VolumeOp::Up,
        step: 4,
        limit_100: true,
    };
    assert_eq!(describe(&v), "Volume up 4% · Discord");
    assert_eq!(
        describe(&Action::Shortcut {
            keys: "super+F12".into()
        }),
        "Shortcut · Super+F12"
    );
    let long = "a".repeat(80);
    let d = describe(&Action::Command {
        cmd: long,
        terminal: false,
    });
    assert!(d.chars().count() <= 48, "{d}");
    assert!(d.ends_with('…'));
    assert_eq!(
        describe(&Action::Media {
            control: MediaControl::PlayPause,
            player: None
        }),
        "Play / Pause"
    );
}
