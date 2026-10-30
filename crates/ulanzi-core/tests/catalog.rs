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
            volume: 100,
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
        Action::ScreenNext,
        Action::Timer {
            op: TimerOp::StartPause,
        },
        Action::Timer { op: TimerOp::Reset },
        Action::Timer {
            op: TimerOp::Add { minutes: 5 },
        },
        Action::Multi { steps: vec![] },
        Action::Seek {
            seconds: 10,
            player: None,
        },
        Action::Seek {
            seconds: -10,
            player: None,
        },
        Action::SwitchOutput { devices: vec![] },
    ]
}

#[test]
fn seek_and_output_read_well() {
    assert_eq!(
        describe(&Action::Seek {
            seconds: -15,
            player: None
        }),
        "Back 15 s"
    );
    assert_eq!(
        describe(&Action::Seek {
            seconds: 30,
            player: None
        }),
        "Forward 30 s"
    );
    assert_eq!(describe(&Action::SwitchOutput { devices: vec![] }), "Next output");
    assert_eq!(
        describe(&Action::SwitchOutput {
            devices: vec!["a".into(), "b".into()]
        }),
        "Next of 2 outputs"
    );
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

#[test]
fn problems_name_what_is_missing() {
    use ulanzi_core::catalog::{Problems, problem};
    let page = uuid::Uuid::new_v4();
    let ctx = Problems {
        macro_exists: &|_| true,
        app_installed: &|id| id == "firefox.desktop",
        file_exists: &|p| p == "/sounds/ding.oga",
        pages: &[page],
        profiles: &[],
        plugin: &|_, _| None,
    };
    let app = |id: &str| Action::LaunchApp {
        desktop_id: Some(id.into()),
        exec: None,
    };
    assert_eq!(problem(&app("firefox.desktop"), &ctx), None);
    assert!(problem(&app("gimp.desktop"), &ctx).is_some_and(|p| p.contains("gimp")));
    let sound = |p: &str| Action::PlaySound {
        path: p.into(),
        device: None,
        volume: 100,
    };
    assert_eq!(problem(&sound("/sounds/ding.oga"), &ctx), None);
    assert!(problem(&sound("/gone.oga"), &ctx).is_some());
    assert_eq!(problem(&Action::GoToPage { page }, &ctx), None);
    assert!(
        problem(
            &Action::GoToPage {
                page: uuid::Uuid::new_v4()
            },
            &ctx
        )
        .is_some()
    );
    // Inside a multi-action too.
    let multi = Action::Multi {
        steps: vec![Step {
            action: app("gimp.desktop"),
            delay_ms: 0,
        }],
    };
    assert!(problem(&multi, &ctx).is_some());
    assert!(
        problem(
            &Action::Command {
                cmd: " ".into(),
                terminal: false
            },
            &ctx
        )
        .is_some()
    );
}

#[test]
fn plugin_problem_comes_from_the_callback() {
    use ulanzi_core::catalog::{Problems, problem};
    let ctx = Problems {
        macro_exists: &|_| true,
        app_installed: &|_| true,
        file_exists: &|_| true,
        pages: &[],
        profiles: &[],
        plugin: &|p, a| Some(format!("{p}/{a}")),
    };
    let act = Action::Plugin {
        plugin: "obs".into(),
        action: "x".into(),
        params: Default::default(),
    };
    assert_eq!(problem(&act, &ctx).as_deref(), Some("obs/x"));
    let multi = Action::Multi {
        steps: vec![Step {
            action: act,
            delay_ms: 0,
        }],
    };
    assert_eq!(problem(&multi, &ctx).as_deref(), Some("obs/x"));
}
