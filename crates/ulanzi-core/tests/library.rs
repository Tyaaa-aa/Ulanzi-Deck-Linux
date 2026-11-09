use ulanzi_core::model::*;
use ulanzi_core::store::{parse_profile, profile_to_toml};
use uuid::Uuid;

fn macro_action() -> Action {
    Action::Macro {
        id: Uuid::new_v4(),
        own: None,
    }
}

fn base() -> (Profile, Uuid) {
    let mut p = Profile::new_default("Test", Model::D200);
    let id = Uuid::new_v4();
    p.dial_library.push(LibraryDial {
        id,
        dial: Dial {
            name: "Volume".into(),
            left: Some(Action::PrevPage),
            ..Default::default()
        },
    });
    (p, id)
}

#[test]
fn library_pins_and_page_small_buttons_round_trip() {
    let (mut p, id) = base();
    p.pages[0].dials[1] = Dial {
        library: Some(id),
        ..Default::default()
    };
    p.pinned.dials.insert(
        0,
        Dial {
            name: "Media".into(),
            ..Default::default()
        },
    );
    p.pages[0].small_buttons.right = Some(Action::NextPage);
    let text = profile_to_toml(&p).expect("ok");
    assert!(text.contains("[[dial_library]]"), "{text}");
    assert!(
        text.contains("[pinned.dials.0]") || text.contains("[pinned.dials.\"0\"]"),
        "{text}"
    );
    assert!(text.contains("[pages.small_buttons.right]"), "{text}");
    p.min_app = Some("2.5.0".into());
    assert_eq!(parse_profile(&text).expect("ok"), p);
}

#[test]
fn dial_at_prefers_the_pin_then_the_page_then_the_library() {
    let (mut p, id) = base();
    p.pinned.dials.insert(
        0,
        Dial {
            name: "Pin".into(),
            ..Default::default()
        },
    );
    p.pages[0].dials[0].name = "Own".into();
    p.pages[0].dials[1].name = "Own".into();
    p.pages[0].dials[2] = Dial {
        library: Some(id),
        ..Default::default()
    };
    assert_eq!(p.dial_at(0, 0).name, "Pin");
    assert_eq!(p.dial_at(0, 1).name, "Own");
    let linked = p.dial_at(0, 2);
    assert_eq!(linked.name, "Volume");
    assert_eq!(linked.left, Some(Action::PrevPage));
    assert!(p.dial_info(0, 0).pinned);
    assert!(!p.dial_info(0, 1).pinned);
    assert_eq!(p.dial_info(0, 2).library, Some(id));
}

#[test]
fn a_dangling_link_is_blank() {
    let (mut p, _) = base();
    p.pages[0].dials[0] = Dial {
        library: Some(Uuid::new_v4()),
        ..Default::default()
    };
    let d = p.dial_at(0, 0);
    assert!(d.is_empty());
    assert!(d.name.is_empty());
}

#[test]
fn small_at_prefers_the_pin() {
    let mut p = Profile::new_default("Test", Model::D200);
    p.pages[0].small_buttons.left = Some(macro_action());
    assert_eq!(p.small_at(0, 0), Some(Action::PrevPage));
    assert!(p.small_pinned(0));
    p.small_buttons.left = None;
    assert_eq!(p.small_at(0, 0), p.pages[0].small_buttons.left);
    assert!(!p.small_pinned(0));
}

#[test]
fn usage_counts_pages_and_pins() {
    let (mut p, id) = base();
    p.pages.push(Page::new("Two"));
    p.pages.push(Page::new("Three"));
    assert_eq!(p.dial_usage(id).text(), "Not placed");
    let link = Dial {
        library: Some(id),
        ..Default::default()
    };
    p.pages[0].dials[0] = link.clone();
    assert_eq!(p.dial_usage(id).text(), "1 page");
    p.pages[2].dials[1] = link.clone();
    assert_eq!(
        p.dial_usage(id),
        Usage {
            pages: 2,
            pinned: false
        }
    );
    assert_eq!(p.dial_usage(id).text(), "2 pages");
    p.pinned.dials.insert(2, link);
    assert_eq!(p.dial_usage(id).text(), "Pinned");
}

#[test]
fn actions_include_library_pins_and_page_small_buttons() {
    let (mut p, _) = base();
    assert!(!p.uses_macros());
    p.dial_library[0].dial.press = Some(macro_action());
    assert!(p.uses_macros());
    p.dial_library.clear();
    p.pinned.dials.insert(
        1,
        Dial {
            hold: Some(macro_action()),
            ..Default::default()
        },
    );
    assert!(p.uses_macros());
    p.pinned.dials.clear();
    p.pages[0].small_buttons.left = Some(macro_action());
    assert!(p.uses_macros());
}

#[test]
fn uses_library_sees_each_part() {
    let mut p = Profile::new_default("Test", Model::D200);
    assert!(!p.uses_library());
    p.pages[0].dials[0].name = "x".into();
    assert!(p.uses_library());
    p.pages[0].dials[0].name.clear();
    p.pages[0].small_buttons.left = Some(Action::PrevPage);
    assert!(p.uses_library());
}

#[test]
fn an_old_profile_still_loads() {
    let text = r#"
version = 2
id = "6f1b1c0e-0000-4000-8000-000000000001"
name = "Old"
model = "D200"

[small_buttons.left]
type = "prev_page"

[[pages]]
id = "6f1b1c0e-0000-4000-8000-000000000002"
name = "Main"

[[pages.dials]]
[pages.dials.press]
type = "next_page"

[[pages.dials]]

[[pages.dials]]
"#;
    let p = parse_profile(text).expect("ok");
    assert!(!p.uses_library());
    let again = profile_to_toml(&p).expect("ok");
    assert!(!again.contains("dial_library") && !again.contains("pinned"));
    assert_eq!(parse_profile(&again).expect("ok"), p);
}

#[test]
fn out_of_range_pins_are_dropped() {
    let mut p = Profile::new_default("Test", Model::D200);
    p.pinned.dials.insert(
        1,
        Dial {
            name: "A".into(),
            ..Default::default()
        },
    );
    p.pinned.dials.insert(
        3,
        Dial {
            name: "B".into(),
            ..Default::default()
        },
    );
    let warnings = p.normalize();
    assert_eq!(p.pinned.dials.keys().copied().collect::<Vec<_>>(), vec![1]);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn library_profiles_save_min_app_2_5() {
    let min_app = |p: &Profile| {
        let t: toml::Table = toml::from_str(&profile_to_toml(p).expect("ok")).expect("toml");
        t.get("min_app").and_then(|v| v.as_str()).map(String::from)
    };
    let plain = Profile::new_default("Test", Model::D200);
    assert_eq!(min_app(&plain), None);

    let mut pin = plain.clone();
    pin.pinned.dials.insert(
        0,
        Dial {
            name: "A".into(),
            ..Default::default()
        },
    );
    assert_eq!(min_app(&pin).as_deref(), Some("2.5.0"));

    let mut mac = plain.clone();
    mac.pages[0].keys.insert(
        0,
        Key {
            tap: Some(macro_action()),
            ..Default::default()
        },
    );
    assert_eq!(min_app(&mac).as_deref(), Some("2.2.0"));

    let mut both = pin.clone();
    both.pages[0].keys.insert(
        0,
        Key {
            tap: Some(macro_action()),
            ..Default::default()
        },
    );
    assert_eq!(min_app(&both).as_deref(), Some("2.5.0"));
}

#[test]
fn an_unparseable_library_item_survives_load_and_save() {
    let text = r#"
version = 2
id = "6f1b1c0e-0000-4000-8000-000000000001"
name = "Test"
model = "D200"

[[dial_library]]
id = "6f1b1c0e-0000-4000-8000-0000000000a1"
[dial_library.dial]
name = "Ok"

[[dial_library]]
id = "6f1b1c0e-0000-4000-8000-0000000000a2"
[dial_library.dial]
name = "Future"
future = 1

[[pages]]
id = "6f1b1c0e-0000-4000-8000-000000000002"
name = "Main"
"#;
    let p = parse_profile(text).expect("ok");
    assert_eq!(p.dial_library.len(), 1);
    assert_eq!(p.foreign_library.len(), 1);
    let out = profile_to_toml(&p).expect("ok");
    let t: toml::Table = toml::from_str(&out).expect("toml");
    assert_eq!(t["dial_library"].as_array().expect("array").len(), 2);
}

#[test]
fn an_old_dial_that_clashes_with_a_pin_is_reported() {
    let mut p = Profile::new_default("Test", Model::D200);
    p.pinned.dials.insert(
        1,
        Dial {
            name: "Pin".into(),
            ..Default::default()
        },
    );
    p.dials[1] = Dial {
        name: "Old".into(),
        ..Default::default()
    };
    p.dials[2] = Dial {
        name: "Other".into(),
        ..Default::default()
    };
    let warnings = p.normalize();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(p.pinned.dials[&1].name, "Pin");
    assert_eq!(p.pinned.dials[&2].name, "Other");
}

#[test]
fn dropping_the_library_keeps_what_shows() {
    let (mut p, id) = base();
    let link = Dial {
        library: Some(id),
        ..Default::default()
    };
    p.pages[0].dials[0] = link.clone();
    p.pinned.dials.insert(2, link);
    p.pages[0].dials[1] = Dial {
        library: Some(Uuid::new_v4()),
        ..Default::default()
    };
    p.foreign_library.push(toml::Value::Integer(1));
    let before = p.clone();
    p.drop_library();
    for n in 0..3u8 {
        assert_eq!(p.dial_at(0, n), before.dial_at(0, n), "dial {n}");
        assert_eq!(p.dial_info(0, n).library, None);
    }
    assert_eq!(p.dial_at(0, 0).name, "Volume");
    assert!(p.dial_at(0, 1).is_empty());
    assert!(p.dial_library.is_empty());
    assert!(p.foreign_library.is_empty());
    assert!(p.dial_info(0, 2).pinned, "pins stay pins");
}
