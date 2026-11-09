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
