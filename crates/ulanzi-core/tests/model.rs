use std::str::FromStr;

use ulanzi_core::model::*;
use ulanzi_core::store::{ProfileLoad, Store, parse_profile, write_atomic};
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
            volume: 60,
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
        Action::obs("toggle_scenes", &[("a", "Gaming"), ("b", "Desktop")]),
        Action::obs("toggle_record", &[]),
    ]
}

#[test]
fn settings_default_roundtrip() {
    let (_d, s) = store();
    let def = s.load_settings().expect("missing file gives defaults");
    assert_eq!(def, Settings::default());
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
        states: Vec::new(),
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
        _ => None,
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
        .replacen("version = 2", "version = 3", 1);
    std::fs::write(&path, text).expect("write");
    let err = s.load_profile(p.id).expect_err("version 3 must fail");
    assert!(
        matches!(err, CoreError::UnsupportedVersion { found: 3, .. }),
        "{err}"
    );
    assert!(err.to_string().contains("newer Ulanzi Deck"), "{err}");
}

#[test]
fn version_1_profiles_load_with_obs_as_the_plugin() {
    let (_d, s) = store();
    let p = Profile::new_default("V", Model::D200);
    s.save_profile(&p).expect("save");
    let path = s.profile_dir(p.id).join("profile.toml");
    let text = std::fs::read_to_string(&path)
        .expect("read")
        .replacen("version = 2", "version = 1", 1)
        + "\n[pages.keys.4.tap]\ntype = \"obs\"\nobs = \"toggle_mute\"\ninput = \"Mic\"\n";
    std::fs::write(&path, text).expect("write");
    let back = s.load_profile(p.id).expect("version 1 loads");
    assert_eq!(back.version, FORMAT_VERSION);
    match &back.pages[0].keys[&4].tap {
        Some(Action::Plugin {
            plugin,
            action,
            params,
        }) => {
            assert_eq!((plugin.as_str(), action.as_str()), ("obs", "toggle_mute"));
            assert_eq!(params["input"].as_str(), Some("Mic"));
        }
        other => panic!("{other:?}"),
    }
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
                lat: None,
                lon: None,
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
        media: MediaTakeover {
            on: true,
            back_after_s: 20,
        },
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
fn hiding_labels_reaches_drawn_labels() {
    let mut p = Profile::new_default("T", Model::D200X);
    let mut drawn = Key::default();
    drawn.look.label.text = "Top".into();
    drawn.look.label.position = Some(LabelPosition::Top);
    let mut plain = Key::default();
    plain.look.label.text = "Plain".into();
    p.pages[0].keys.insert(0, drawn);
    p.pages[0].keys.insert(1, plain);
    p.hide_drawn_labels();
    assert!(!p.pages[0].keys[&0].look.label.visible);
    // Firmware labels are hidden by the deck itself; their setting stays.
    assert!(p.pages[0].keys[&1].look.label.visible);
}

#[test]
fn live_states_pick_the_first_match_and_fill_params() {
    use ulanzi_core::Color;
    let blue = Color([0x4f, 0xc3, 0xf7, 0xff]);
    let look = KeyLook {
        states: vec![
            LookState {
                when: When {
                    value: "{ha.$entity_id}".into(),
                    is: vec!["cool".into()],
                    not: vec![],
                },
                look: LookOverride {
                    foreground: Some(blue),
                    ..LookOverride::default()
                },
            },
            LookState {
                when: When {
                    value: "{ha.$entity_id}".into(),
                    is: vec![],
                    not: vec!["off".into()],
                },
                look: LookOverride {
                    label: Some("On".into()),
                    ..LookOverride::default()
                },
            },
        ],
        ..KeyLook::default()
    };
    let mut params = toml::Table::new();
    params.insert("entity_id".into(), "climate.ac".into());
    let with = |value: &'static str| {
        look.with_state(&params, move |t| {
            assert_eq!(t, "{ha.climate.ac}");
            value.to_string()
        })
    };
    assert_eq!(with("cool").foreground, blue);
    assert_eq!(with("COOL").foreground, blue, "case does not matter");
    assert_eq!(with("dry").label.text, "On");
    assert_eq!(with("off"), look, "nothing holds: the key's own look");
    assert_eq!(with(""), look, "not published yet: no state holds");
    let fixed = KeyLook {
        dynamic: false,
        ..look.clone()
    };
    assert_eq!(
        fixed.with_state(&params, |_| "cool".into()),
        fixed,
        "live state off"
    );
    assert_eq!(
        fill_params("{ha.$entity_id.pct}%", &params),
        "{ha.climate.ac.pct}%"
    );
    // A cycle step round-trips through TOML.
    let a = Action::Cycle {
        steps: vec![CycleStep {
            action: Action::NextPage,
            look: LookOverride {
                label: Some("Next".into()),
                ..LookOverride::default()
            },
        }],
        show: CycleShows::Next,
    };
    // The step that ran last: before any press, the last one.
    assert_eq!(CycleShows::Last.step(0, 3), 2);
    assert_eq!(CycleShows::Last.step(1, 3), 0);
    assert_eq!(CycleShows::Next.step(4, 3), 1);
    let text = toml::to_string(&Key {
        look: KeyLook::default(),
        tap: Some(a.clone()),
        hold: None,
    })
    .expect("toml");
    let back: Key = toml::from_str(&text).expect("parse");
    assert_eq!(back.tap, Some(a));
}

#[test]
fn version_1_files_are_copied_once_before_the_upgrade() {
    let (d, s) = store();
    let p = Profile::new_default("V", Model::D200);
    s.save_profile(&p).expect("save");
    assert_eq!(
        s.keep_v1_copy().expect("check"),
        None,
        "already version 2: nothing to keep"
    );
    let path = s.profile_dir(p.id).join("profile.toml");
    let v1 = std::fs::read_to_string(&path)
        .expect("read")
        .replacen("version = 2", "version = 1", 1);
    std::fs::write(&path, &v1).expect("write");
    let kept = s.keep_v1_copy().expect("copy").expect("copied");
    assert!(kept.starts_with(d.path()));
    let copy = kept.join("profiles").join(p.id.to_string()).join("profile.toml");
    assert_eq!(std::fs::read_to_string(copy).expect("copy"), v1);
    assert_eq!(s.keep_v1_copy().expect("again"), None, "only once");
}

/// A saved profile as a TOML table, for adding things a newer version would write.
fn table_of(p: &Profile) -> toml::Table {
    toml::from_str(&toml::to_string(p).expect("serialize")).expect("table")
}

fn with_macro_key(p: &Profile) -> String {
    let mut t = table_of(p);
    let page = t["pages"].as_array_mut().expect("pages")[0]
        .as_table_mut()
        .expect("page");
    let keys = page
        .entry("keys")
        .or_insert(toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .expect("keys");
    keys.insert("0".into(), toml::Value::try_from(Key::default()).expect("key"));
    keys.insert(
        "3".into(),
        toml::from_str("tap = { type = \"macro\", id = 7 }").expect("key"),
    );
    toml::to_string(&t).expect("text")
}

#[test]
fn an_unknown_action_on_one_key_keeps_the_rest_of_the_profile() {
    let original = Profile::new_default("F", Model::D200X);
    let p = parse_profile(&with_macro_key(&original)).expect("loads");
    assert!(p.pages[0].keys.contains_key(&0));
    assert!(!p.pages[0].keys.contains_key(&3));
    assert_eq!(p.foreign.len(), 1);
    assert_eq!(p.foreign[0].slot, Slot::Key(3));
    assert_eq!(p.foreign[0].page, Some(original.pages[0].id));
    assert!(p.foreign_at(Some(original.pages[0].id), Slot::Key(3)).is_some());
}

#[test]
fn unknown_parts_on_a_dial_a_small_button_and_the_screen_are_kept() {
    let original = Profile::new_default("F", Model::D200X);
    let mut t = table_of(&original);
    let unknown: toml::Table = toml::from_str("type = \"macro\"").expect("action");
    let unknown = toml::Value::Table(unknown);
    t["small_buttons"]
        .as_table_mut()
        .expect("small")
        .insert("left".into(), unknown.clone());
    t.insert("screen".into(), unknown.clone());
    let page = t["pages"].as_array_mut().expect("pages")[0]
        .as_table_mut()
        .expect("page");
    let empty = || toml::Value::Table(toml::Table::new());
    let mut dial = toml::Table::new();
    dial.insert("press".into(), unknown);
    page.insert(
        "dials".into(),
        toml::Value::Array(vec![empty(), toml::Value::Table(dial), empty()]),
    );
    let p = parse_profile(&toml::to_string(&t).expect("text")).expect("loads");
    let slots: Vec<Slot> = p.foreign.iter().map(|f| f.slot).collect();
    assert!(slots.contains(&Slot::Small(0)), "{slots:?}");
    assert!(slots.contains(&Slot::Dial(1)), "{slots:?}");
    assert!(slots.contains(&Slot::Screen), "{slots:?}");
    assert_eq!(p.small_buttons.right, Some(Action::NextPage));
    assert_eq!(p.small_buttons.left, None);
    assert_eq!(p.screen, ScreenConfig::default());
}

#[test]
fn a_profile_from_2_1_0_has_no_foreign_slots() {
    let p = Profile::new_default("F", Model::D200X);
    let back = parse_profile(&toml::to_string(&p).expect("text")).expect("loads");
    assert!(back.foreign.is_empty());
    assert_eq!(back, p);
}

#[test]
fn an_unknown_field_above_the_keys_still_fails() {
    let mut t = table_of(&Profile::new_default("F", Model::D200X));
    t.insert("something_new".into(), toml::Value::Integer(1));
    assert!(parse_profile(&toml::to_string(&t).expect("text")).is_err());
}

fn saved_table(s: &Store, id: Uuid) -> toml::Table {
    let text = std::fs::read_to_string(s.profile_dir(id).join("profile.toml")).expect("read");
    toml::from_str(&text).expect("table")
}

#[test]
fn foreign_keys_survive_a_save_unchanged() {
    let (_d, s) = store();
    let original = Profile::new_default("F", Model::D200X);
    let p = parse_profile(&with_macro_key(&original)).expect("loads");
    s.save_profile(&p).expect("save");
    let t = saved_table(&s, p.id);
    let page = &t["pages"].as_array().expect("pages")[0];
    let key = &page["keys"]["3"];
    assert_eq!(key["tap"]["type"].as_str(), Some("macro"));
    assert_eq!(key["tap"]["id"].as_integer(), Some(7));
    let again = s.load_profile(p.id).expect("load again");
    assert_eq!(again.foreign, p.foreign);
}

#[test]
fn every_kind_of_foreign_slot_goes_back_where_it_came_from() {
    let mut p = Profile::new_default("F", Model::D200X);
    let raw: toml::Value = toml::Value::Table(toml::from_str("type = \"macro\"").expect("raw"));
    for (page, slot) in [
        (None, Slot::Small(1)),
        (None, Slot::Screen),
        (Some(p.pages[0].id), Slot::Dial(2)),
    ] {
        p.foreign.push(ForeignSlot {
            page,
            slot,
            raw: raw.clone(),
        });
    }
    let back = parse_profile(&ulanzi_core::store::profile_to_toml(&p).expect("toml")).expect("parse");
    assert_eq!(back.foreign, p.foreign);
}

#[test]
fn a_plain_profile_is_written_as_before() {
    let p = Profile::new_default("F", Model::D200X);
    let text = ulanzi_core::store::profile_to_toml(&p).expect("toml");
    assert_eq!(text, toml::to_string_pretty(&p).expect("pretty"));
    assert!(!text.contains("min_app"));
}

#[test]
fn a_profile_with_a_higher_version_stays_where_it_is() {
    let (_d, s) = store();
    let p = Profile::new_default("F", Model::D200X);
    s.save_profile(&p).expect("save");
    let path = s.profile_dir(p.id).join("profile.toml");
    let text = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, text.replacen("version = 2", "version = 3", 1)).expect("write");
    let loads = s.list_profiles();
    assert!(
        matches!(loads[..], [ProfileLoad::TooNew { found: 3, .. }]),
        "{loads:?}"
    );
    assert!(path.exists());
}

#[test]
fn a_profile_unreadable_above_key_level_is_too_new() {
    let (_d, s) = store();
    let p = Profile::new_default("F", Model::D200X);
    s.save_profile(&p).expect("save");
    let path = s.profile_dir(p.id).join("profile.toml");
    let mut t = saved_table(&s, p.id);
    t["pages"].as_array_mut().expect("pages")[0]
        .as_table_mut()
        .expect("page")
        .insert("layout".into(), toml::Value::Integer(4));
    std::fs::write(&path, toml::to_string(&t).expect("text")).expect("write");
    assert!(matches!(s.list_profiles()[..], [ProfileLoad::TooNew { .. }]));
    assert!(path.exists());
}

#[test]
fn unknown_settings_survive_a_save() {
    let (_d, s) = store();
    let mut settings = s.load_settings().expect("load");
    settings
        .extra
        .insert("from_the_future".into(), toml::Value::Boolean(true));
    s.save_settings(&settings).expect("save");
    let back = s.load_settings().expect("load again");
    assert_eq!(
        back.extra.get("from_the_future"),
        Some(&toml::Value::Boolean(true))
    );
    assert_eq!(back, settings);
}

#[test]
fn dates_in_real_keys_survive_next_to_a_foreign_key() {
    let mut p = Profile::new_default("F", Model::D200X);
    let params: toml::Table = toml::from_str("when = 2026-10-31T10:00:00Z").expect("params");
    p.pages[0].keys.insert(
        0,
        Key {
            tap: Some(Action::Plugin {
                plugin: "x".into(),
                action: "y".into(),
                params: params.clone(),
            }),
            ..Key::default()
        },
    );
    p.foreign.push(ForeignSlot {
        page: Some(p.pages[0].id),
        slot: Slot::Key(3),
        raw: toml::Value::Table(toml::from_str("kind = \"new\"").expect("raw")),
    });
    let text = ulanzi_core::store::profile_to_toml(&p).expect("toml");
    assert!(!text.contains("__toml_private"), "{text}");
    let back = parse_profile(&text).expect("parse");
    assert_eq!(back.pages[0].keys[&0].tap, p.pages[0].keys[&0].tap);
}

#[test]
fn unknown_nested_settings_survive_a_save() {
    let (_d, s) = store();
    let mut settings = s.load_settings().expect("load");
    settings.plugins.insert("x".into(), PluginEntry::default());
    s.save_settings(&settings).expect("save");
    let path = s.settings_path();
    let mut t: toml::Table = toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("toml");
    t["label_style"]
        .as_table_mut()
        .expect("label")
        .insert("shadow".into(), toml::Value::Boolean(true));
    t["plugins"]["x"]
        .as_table_mut()
        .expect("plugin")
        .insert("rank".into(), toml::Value::Integer(4));
    std::fs::write(&path, toml::to_string(&t).expect("text")).expect("write");
    let loaded = s.load_settings().expect("loads");
    s.save_settings(&loaded).expect("save again");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("shadow"), "{text}");
    assert!(text.contains("rank"), "{text}");
}

#[test]
fn settings_that_failed_to_load_are_never_overwritten() {
    let (_d, s) = store();
    s.save_settings(&Settings::default()).expect("save");
    let path = s.settings_path();
    std::fs::write(&path, "version = 3\nsecret = \"keep me\"\n").expect("write");
    assert!(s.load_settings().is_err());
    s.protect_settings();
    assert!(s.save_settings(&Settings::default()).is_err());
    assert!(std::fs::read_to_string(&path).expect("read").contains("keep me"));
}
