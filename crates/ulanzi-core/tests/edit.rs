use ulanzi_core::Color;
use ulanzi_core::catalog::default_look;
use ulanzi_core::edit::{Editor, Field, Mixed, Selection, Slot, shared};
use ulanzi_core::model::*;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::rgb(r, g, b)
}

fn editor_with_keys() -> Editor {
    let mut p = Profile::new_default("P", Model::D200X);
    for (i, fg) in [(0u8, rgb(255, 0, 0)), (1, rgb(0, 255, 0)), (2, rgb(0, 0, 255))] {
        let look = KeyLook {
            foreground: fg,
            ..KeyLook::default()
        };
        p.pages[0].keys.insert(
            i,
            Key {
                look,
                tap: Some(Action::NextPage),
                hold: None,
            },
        );
    }
    Editor::new(p)
}

fn key(e: &Editor, i: u8) -> &Key {
    e.profile().pages[e.page()]
        .keys
        .get(&i)
        .unwrap_or_else(|| panic!("key {i}"))
}

#[test]
fn click_ctrl_shift_selection() {
    let mut s = Selection::default();
    s.click(Slot::Key(2));
    assert_eq!(s.keys(), vec![2]);
    s.toggle(Slot::Key(5));
    assert_eq!(s.keys(), vec![2, 5]);
    s.toggle(Slot::Key(2));
    assert_eq!(s.keys(), vec![5]);
    s.click(Slot::Key(3));
    s.range_to(Slot::Key(7));
    assert_eq!(s.keys(), vec![3, 4, 5, 6, 7]);
    s.range_to(Slot::Key(1));
    assert_eq!(s.keys(), vec![1, 2, 3]);
    s.select_all_keys();
    assert_eq!(s.keys().len(), 13);
    s.click(Slot::Dial(1));
    assert_eq!(s.slots(), vec![Slot::Dial(1)]);
    assert!(s.keys().is_empty());
    s.clear();
    assert!(s.slots().is_empty());
}

#[test]
fn bulk_edit_touches_only_changed_field() {
    let mut e = editor_with_keys();
    e.set_field(&[0, 1, 2], Field::Background(rgb(9, 9, 9)));
    assert_eq!(key(&e, 0).look.foreground, rgb(255, 0, 0));
    assert_eq!(key(&e, 1).look.foreground, rgb(0, 255, 0));
    assert_eq!(key(&e, 2).look.foreground, rgb(0, 0, 255));
    assert!((0..3).all(|i| key(&e, i).look.background == rgb(9, 9, 9)));
    assert!(e.undo());
    assert!((0..3).all(|i| key(&e, i).look.background == Color::BLACK));
}

#[test]
fn set_field_on_empty_key_creates_it() {
    let mut e = editor_with_keys();
    e.set_field(&[9], Field::LabelText("Hi".into()));
    assert_eq!(key(&e, 9).look.label.text, "Hi");
    assert_eq!(key(&e, 9).tap, None);
}

#[test]
fn shared_reports_mixed() {
    assert_eq!(shared([1, 1, 1].into_iter()), Mixed::One(1));
    assert_eq!(shared([1, 2].into_iter()), Mixed::Mixed);
    assert_eq!(shared(std::iter::empty::<u8>()), Mixed::None);
    let e = editor_with_keys();
    assert_eq!(e.shared_foreground(&[0, 1]), Mixed::Mixed);
    assert_eq!(e.shared_foreground(&[2]), Mixed::One(rgb(0, 0, 255)));
}

#[test]
fn drop_action_on_empty_key_gets_default_look() {
    let mut e = editor_with_keys();
    let a = Action::Media {
        control: MediaControl::PlayPause,
        player: None,
    };
    e.drop_action(Slot::Key(7), a.clone());
    assert_eq!(key(&e, 7).look, default_look(&a));
    assert_eq!(key(&e, 7).tap, Some(a));
}

#[test]
fn drop_action_keeps_custom_look() {
    let mut e = editor_with_keys();
    let before = key(&e, 0).look.clone();
    e.drop_action(Slot::Key(0), Action::SleepNow);
    // The custom colour stays; the empty icon and label are filled in.
    let after = &key(&e, 0).look;
    assert_eq!(after.foreground, before.foreground);
    assert_eq!(after.source, default_look(&Action::SleepNow).source);
    assert_eq!(after.label.text, default_look(&Action::SleepNow).label.text);
    assert_eq!(key(&e, 0).tap, Some(Action::SleepNow));
}

#[test]
fn drop_action_on_dial_and_small_and_screen() {
    let mut e = editor_with_keys();
    e.drop_action(Slot::Small(1), Action::Back);
    assert_eq!(e.profile().small_buttons.right, Some(Action::Back));
    e.set_dial(1, |d| d.left = Some(Action::PrevPage));
    assert_eq!(e.dial_info(1).dial.left, Some(Action::PrevPage));
    e.drop_action(Slot::Screen, Action::SleepNow);
    assert!(matches!(
        e.profile().screen,
        ScreenConfig::Custom {
            tap: Some(Action::SleepNow),
            ..
        }
    ));
}

#[test]
fn swap_and_undo() {
    let mut e = editor_with_keys();
    e.swap(0, 9);
    assert!(!e.profile().pages[0].keys.contains_key(&0));
    assert_eq!(key(&e, 9).look.foreground, rgb(255, 0, 0));
    e.undo();
    assert_eq!(key(&e, 0).look.foreground, rgb(255, 0, 0));
}

#[test]
fn paste_keys_keeps_layout_and_clips() {
    let mut e = editor_with_keys();
    e.copy_keys(&[0, 2]);
    e.paste_keys(11);
    assert_eq!(key(&e, 11).look.foreground, rgb(255, 0, 0));
    // key 2 is two slots after key 0, so it would land on 13 (the wide screen): dropped.
    assert!(!e.profile().pages[0].keys.contains_key(&13));
}

#[test]
fn paste_style_copies_look_not_action() {
    let mut e = editor_with_keys();
    e.set_field(&[5], Field::LabelText("x".into()));
    e.drop_action(Slot::Key(5), Action::SleepNow);
    e.copy_style(1);
    e.paste_style(&[5]);
    assert_eq!(key(&e, 5).look, key(&e, 1).look);
    assert_eq!(key(&e, 5).tap, Some(Action::SleepNow));
}

#[test]
fn move_to_page_uses_free_slots() {
    let mut e = editor_with_keys();
    let p2 = e.add_page("Two");
    e.set_page(p2);
    e.set_field(&[0], Field::LabelText("taken".into()));
    e.set_page(0);
    e.move_to_page(&[1, 2], p2);
    assert!(!e.profile().pages[0].keys.contains_key(&1));
    let two = &e.profile().pages[p2].keys;
    assert_eq!(two.get(&0).map(|k| k.look.label.text.as_str()), Some("taken"));
    assert_eq!(two.get(&1).map(|k| k.look.foreground), Some(rgb(0, 255, 0)));
    assert_eq!(two.get(&2).map(|k| k.look.foreground), Some(rgb(0, 0, 255)));
}

#[test]
fn remove_last_page_refused() {
    let mut e = editor_with_keys();
    assert!(!e.remove_page(0));
    let p = e.add_page("Two");
    assert!(e.remove_page(p));
    assert_eq!(e.profile().pages.len(), 1);
}

#[test]
fn redo_cleared_by_new_edit() {
    let mut e = editor_with_keys();
    e.set_field(&[0], Field::LabelText("a".into()));
    e.undo();
    assert!(e.can_redo());
    e.set_field(&[0], Field::LabelText("b".into()));
    assert!(!e.can_redo());
}

#[test]
fn history_is_bounded() {
    let mut e = editor_with_keys();
    for n in 0..250 {
        e.set_field(&[0], Field::LabelText(n.to_string()));
    }
    let mut undos = 0;
    while e.undo() {
        undos += 1;
    }
    assert_eq!(undos, 200);
}

#[test]
fn dirty_tracking() {
    let mut e = editor_with_keys();
    assert!(!e.is_dirty());
    e.set_field(&[0], Field::LabelVisible(false));
    assert!(e.is_dirty());
    e.mark_clean();
    assert!(!e.is_dirty());
    e.undo();
    assert!(e.is_dirty());
}

#[test]
fn typing_coalesces_into_one_undo_step() {
    let mut e = editor_with_keys();
    let before = key(&e, 0).look.label.text.clone();
    for t in ["H", "He", "Hel", "Hello"] {
        e.set_merge(Some("label"));
        e.set_field(&[0], Field::LabelText(t.into()));
    }
    e.set_merge(None);
    assert_eq!(key(&e, 0).look.label.text, "Hello");
    assert!(e.undo());
    assert_eq!(key(&e, 0).look.label.text, before);
}

#[test]
fn different_merge_keys_are_separate_steps() {
    let mut e = editor_with_keys();
    e.set_merge(Some("label"));
    e.set_field(&[0], Field::LabelText("A".into()));
    e.set_merge(Some("cmd"));
    e.set_field(&[0], Field::LabelText("B".into()));
    e.set_merge(None);
    assert!(e.undo());
    assert_eq!(key(&e, 0).look.label.text, "A");
}

#[test]
fn ending_a_merge_starts_a_new_step() {
    let mut e = editor_with_keys();
    e.set_merge(Some("label"));
    e.set_field(&[0], Field::LabelText("A".into()));
    e.set_merge(None);
    e.set_merge(Some("label"));
    e.set_field(&[1], Field::LabelText("B".into()));
    assert!(e.undo());
    assert_eq!(key(&e, 0).look.label.text, "A");
}

#[test]
fn set_model_is_undoable() {
    let mut e = editor_with_keys();
    let before = e.profile().model;
    let other = if before == Model::D200 {
        Model::D200X
    } else {
        Model::D200
    };
    e.set_model(other);
    assert_eq!(e.profile().model, other);
    assert!(e.undo());
    assert_eq!(e.profile().model, before);
}

fn launch(id: Option<&str>) -> Action {
    Action::LaunchApp {
        desktop_id: id.map(String::from),
        exec: None,
    }
}

fn editor_with(look: KeyLook, tap: Action) -> Editor {
    let mut p = Profile::new_default("P", Model::D200X);
    p.pages[0].keys.insert(
        0,
        Key {
            look,
            tap: Some(tap),
            hold: None,
        },
    );
    Editor::new(p)
}

#[test]
fn changing_action_updates_an_untouched_look() {
    let cmd = Action::Command {
        cmd: String::new(),
        terminal: false,
    };
    let url = Action::OpenUrl { url: String::new() };
    let mut e = editor_with(default_look(&cmd), cmd);
    e.set_key_actions(0, Some(url.clone()), None);
    assert_eq!(key(&e, 0).look, default_look(&url));
    // Picking from the list behaves the same.
    e.drop_action(Slot::Key(0), Action::NextPage);
    assert_eq!(key(&e, 0).look, default_look(&Action::NextPage));
}

#[test]
fn changing_action_keeps_what_the_user_customised() {
    let cmd = Action::Command {
        cmd: String::new(),
        terminal: false,
    };
    let url = Action::OpenUrl { url: String::new() };
    let mut look = default_look(&cmd);
    look.background = rgb(200, 0, 0);
    look.label.text = "Mine".into();
    let mut e = editor_with(look, cmd);
    e.set_key_actions(0, Some(url.clone()), None);
    let after = &key(&e, 0).look;
    assert_eq!(after.source, default_look(&url).source);
    assert_eq!(after.foreground, default_look(&url).foreground);
    assert_eq!(after.background, rgb(200, 0, 0));
    assert_eq!(after.label.text, "Mine");
}

#[test]
fn launch_app_uses_the_apps_icon_and_name() {
    let mut e = editor_with(default_look(&launch(None)), launch(None));
    e.set_key_actions(0, Some(launch(Some("zz-not-installed.desktop"))), None);
    let l = &key(&e, 0).look;
    assert_eq!(
        l.source,
        ImageSource::App {
            desktop_id: "zz-not-installed.desktop".into()
        }
    );
    assert_eq!(l.label.text, "zz-not-installed");
    // Switching to another app follows it too.
    e.set_key_actions(0, Some(launch(Some("zz-other.desktop"))), None);
    assert_eq!(
        key(&e, 0).look.source,
        ImageSource::App {
            desktop_id: "zz-other.desktop".into()
        }
    );
    assert_eq!(key(&e, 0).look.label.text, "zz-other");
}

#[test]
fn label_position_is_a_bulk_field() {
    let mut e = editor_with_keys();
    e.set_field(&[0, 1], Field::LabelPosition(Some(LabelPosition::Top)));
    assert_eq!(key(&e, 0).look.label.position, Some(LabelPosition::Top));
    assert_eq!(key(&e, 1).look.label.position, Some(LabelPosition::Top));
    assert_eq!(key(&e, 2).look.label.position, None);
}

#[test]
fn rename_profile_trims_and_ignores_empty() {
    let mut e = editor_with_keys();
    e.rename_profile("  Gaming  ");
    assert_eq!(e.profile().name, "Gaming");
    e.rename_profile("   ");
    assert_eq!(e.profile().name, "Gaming");
    assert!(e.undo());
    assert_eq!(e.profile().name, "P");
}

#[test]
fn dials_belong_to_the_page() {
    let mut e = editor_with_keys();
    e.set_dial(0, |d| d.press = Some(Action::NextPage));
    let two = e.add_page("Two");
    e.set_page(two);
    e.set_dial(0, |d| d.press = Some(Action::PrevPage));
    e.set_page(0);
    assert_eq!(e.dial_info(0).dial.press, Some(Action::NextPage));
    e.set_page(two);
    assert_eq!(e.dial_info(0).dial.press, Some(Action::PrevPage));
}

#[test]
fn moving_a_page_keeps_the_page_you_are_on() {
    let mut e = editor_with_keys();
    e.add_page("Two");
    e.add_page("Three"); // inserted after the current page (0): Main, Three, Two
    let names = |e: &Editor| {
        e.profile()
            .pages
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&e), ["Main", "Three", "Two"]);
    e.move_page(2, 0);
    assert_eq!(names(&e), ["Two", "Main", "Three"]);
    // Still on "Main".
    assert_eq!(e.profile().pages[e.page()].name, "Main");
}

#[test]
fn match_rule_is_an_undoable_edit() {
    let mut e = editor_with_keys();
    e.set_match(MatchRule {
        apps: vec!["org.blender.Blender.desktop".into()],
        priority: 2,
    });
    assert_eq!(e.profile().r#match.apps, ["org.blender.Blender.desktop"]);
    assert!(e.undo());
    assert!(e.profile().r#match.apps.is_empty());
}

#[test]
fn add_keys_keeps_each_group_on_one_page() {
    let mut e = Editor::new(Profile::new_default("P", Model::D200X));
    let k = |label: &str| Key {
        look: KeyLook {
            label: Label {
                text: label.into(),
                ..Label::default()
            },
            ..KeyLook::default()
        },
        tap: Some(Action::NextPage),
        hold: None,
    };
    let pages = e.profile().pages.len();
    // 13 slots: four groups of three fill twelve; the fifth goes to a new page.
    let groups: Vec<Vec<Key>> = (0..5)
        .map(|g| (0..3).map(|i| k(&format!("{g}.{i}"))).collect())
        .collect();
    assert_eq!(e.free_keys(), 13);
    let first = e.add_keys(groups, "Home", false);
    assert_eq!(first, 0);
    assert_eq!(e.profile().pages.len(), pages + 1);
    assert_eq!(e.profile().pages[0].keys.len(), 12);
    let next = &e.profile().pages[1];
    assert_eq!(next.name, "Home");
    assert_eq!(next.keys[&0].look.label.text, "4.0");
    // One undo step takes it all back.
    assert!(e.undo());
    assert_eq!(e.profile().pages.len(), pages);
    assert!(e.profile().pages[0].keys.is_empty());
    // On a new page from the start.
    let first = e.add_keys(vec![vec![k("a")]], "Home", true);
    assert_eq!(first, 1);
    assert_eq!(e.profile().pages[1].keys[&0].look.label.text, "a");
}

/// A profile whose key 3 and dial 1 came from a newer version.
fn with_foreign() -> Profile {
    let mut p = Profile::new_default("F", Model::D200X);
    let page = Some(p.pages[0].id);
    let raw: toml::Value = toml::Value::Table(toml::from_str("kind = \"new\"").expect("raw"));
    p.foreign.push(ForeignSlot {
        page,
        slot: Slot::Key(3),
        raw: raw.clone(),
    });
    p.foreign.push(ForeignSlot {
        page,
        slot: Slot::Dial(1),
        raw,
    });
    p
}

#[test]
fn pasting_onto_a_foreign_key_is_refused() {
    let mut e = Editor::new(with_foreign());
    e.set_field(&[2], Field::Background(Color::rgb(1, 2, 3)));
    e.copy_keys(&[2]);
    e.paste_keys(3);
    assert!(!e.profile().pages[0].keys.contains_key(&3));
    assert_eq!(e.profile().foreign.len(), 2);
}

#[test]
fn nothing_else_writes_over_a_foreign_slot() {
    let mut e = Editor::new(with_foreign());
    e.set_field(&[3], Field::Background(Color::rgb(1, 2, 3)));
    e.drop_action(Slot::Key(3), Action::NextPage);
    e.set_key_actions(3, Some(Action::NextPage), None);
    e.set_field(&[2], Field::Background(Color::rgb(1, 2, 3)));
    e.swap(2, 3);
    assert!(!e.profile().pages[0].keys.contains_key(&3));
    assert!(e.profile().pages[0].keys.contains_key(&2));
    e.set_dial(1, |d| d.press = Some(Action::NextPage));
    e.drop_action(Slot::Dial(1), Action::NextPage);
    assert_eq!(e.dial_info(1).dial, Dial::default());
}

#[test]
fn new_keys_do_not_land_on_a_foreign_slot() {
    let mut e = Editor::new(with_foreign());
    assert_eq!(e.free_keys(), 12);
    e.add_keys(vec![vec![Key::default(); 4]], "More", false);
    assert!(!e.profile().pages[0].keys.contains_key(&3));
    assert!(e.profile().pages[0].keys.contains_key(&4));
}

#[test]
fn clearing_a_foreign_key_removes_it() {
    let mut e = Editor::new(with_foreign());
    e.clear(&[3]);
    assert_eq!(e.profile().foreign.len(), 1);
    assert_eq!(e.profile().foreign[0].slot, Slot::Dial(1));
    e.clear_foreign(Slot::Dial(1));
    assert!(e.profile().foreign.is_empty());
}

#[test]
fn removing_a_page_removes_its_foreign_slots() {
    let mut e = Editor::new(with_foreign());
    e.add_page("Second");
    e.remove_page(0);
    assert!(e.profile().foreign.is_empty());
}

#[test]
fn pasting_a_style_onto_a_foreign_key_is_refused() {
    let mut e = Editor::new(with_foreign());
    e.set_field(&[2], Field::Background(Color::rgb(1, 2, 3)));
    e.copy_style(2);
    e.paste_style(&[3]);
    assert!(!e.profile().pages[0].keys.contains_key(&3));
}

fn macro_key(m: &Macro, own: bool) -> Editor {
    let mut p = Profile::new_default("M", Model::D200X);
    p.pages[0].keys.insert(
        0,
        Key {
            tap: Some(Action::Macro {
                id: m.id,
                own: own.then(|| Box::new(m.clone())),
            }),
            ..Key::default()
        },
    );
    let mut e = Editor::new(p);
    e.set_library(MacroLibrary {
        macros: vec![m.clone()],
    });
    e
}

fn own_of(e: &Editor) -> Option<Macro> {
    match &e.key(0).expect("key").tap {
        Some(Action::Macro { own, .. }) => own.as_deref().cloned(),
        _ => None,
    }
}

fn linked(e: &Editor) -> uuid::Uuid {
    match &e.key(0).expect("key").tap {
        Some(Action::Macro { id, .. }) => *id,
        other => panic!("{other:?}"),
    }
}

fn run_step(id: uuid::Uuid) -> MacroStep {
    MacroStep {
        event: MacroEvent::RunMacro { id },
        delay_ms: 0,
        look: LookOverride::default(),
        part: SequencePart::Press,
    }
}

#[test]
fn editing_for_this_key_stops_following_the_library() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, false);
    e.edit_for_this_key(Slot::Key(0));
    assert_eq!(own_of(&e).map(|o| o.name), Some("Lib".to_string()));
    let mut changed = m.clone();
    changed.name = "Changed".into();
    assert!(e.save_macro(changed));
    assert_eq!(own_of(&e).map(|o| o.name), Some("Lib".to_string()));
}

#[test]
fn save_as_new_links_the_key_and_undo_restores_both() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, true);
    let id = e.own_to_new(Slot::Key(0), "Fresh").expect("new");
    assert_ne!(id, m.id);
    assert_eq!(linked(&e), id);
    assert!(own_of(&e).is_none());
    assert_eq!(e.library().get(id).map(|n| n.name.as_str()), Some("Fresh"));
    assert!(e.undo());
    assert!(own_of(&e).is_some());
    assert_eq!(linked(&e), m.id);
    assert!(e.library().get(id).is_none());
}

#[test]
fn updating_the_library_links_the_key_and_undo_restores_the_old_version() {
    let m = Macro::new("Lib");
    let mut copy = m.clone();
    copy.name = "Better".into();
    let mut e = macro_key(&m, false);
    e.set_key_actions(
        0,
        Some(Action::Macro {
            id: m.id,
            own: Some(Box::new(copy)),
        }),
        None,
    );
    e.own_to_library(Slot::Key(0));
    assert!(own_of(&e).is_none());
    assert_eq!(e.library().get(m.id).map(|n| n.name.as_str()), Some("Better"));
    assert!(e.undo());
    assert_eq!(e.library().get(m.id).map(|n| n.name.as_str()), Some("Lib"));
    assert_eq!(own_of(&e).map(|o| o.name), Some("Better".to_string()));
}

#[test]
fn use_library_drops_the_copy_and_undo_brings_it_back() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, true);
    e.use_library(Slot::Key(0));
    assert!(own_of(&e).is_none());
    assert!(e.undo());
    assert!(own_of(&e).is_some());
}

#[test]
fn an_overwrite_keeps_the_previous_version() {
    let m = Macro::new("One");
    let mut e = macro_key(&m, false);
    let mut two = m.clone();
    two.name = "Two".into();
    e.save_macro(two.clone());
    let mut three = two.clone();
    three.name = "Three".into();
    e.save_macro(three);
    let now = e.library().get(m.id).expect("macro");
    let previous = now.previous.as_ref().expect("previous");
    assert_eq!(previous.name, "Two");
    assert!(previous.previous.is_none(), "one level only");
}

#[test]
fn with_the_library_macro_deleted_the_copy_keeps_working() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, true);
    e.delete_macro(m.id);
    assert!(e.library().get(m.id).is_none());
    assert!(own_of(&e).is_some());
    e.use_library(Slot::Key(0));
    assert!(own_of(&e).is_some(), "nothing to go back to");
    e.own_to_library(Slot::Key(0));
    assert!(e.library().get(m.id).is_some());
    assert!(own_of(&e).is_none());
}

#[test]
fn a_macro_cannot_run_itself() {
    let (mut a, mut b) = (Macro::new("A"), Macro::new("B"));
    let mut e = macro_key(&a, false);
    b.steps = vec![run_step(a.id)];
    assert!(e.save_macro(b.clone()));
    a.steps = vec![run_step(a.id)];
    assert!(!e.save_macro(a.clone()), "directly");
    a.steps = vec![run_step(b.id)];
    assert!(!e.save_macro(a), "through another macro");
    assert!(e.library().get(b.id).is_some());
}

#[test]
fn users_of_a_macro_are_found_on_every_page() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, false);
    e.add_page("Two");
    e.set_page(1);
    e.drop_action(Slot::Key(4), Action::Macro { id: m.id, own: None });
    assert_eq!(e.macro_users(m.id).len(), 2);
}

#[test]
fn undoing_a_key_edit_leaves_a_newer_library_alone() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, false);
    e.set_field(&[0], Field::Background(rgb(1, 2, 3)));
    let imported = Macro::new("Imported");
    let mut library = e.library().clone();
    library.macros.push(imported.clone());
    e.set_library(library.clone());
    assert!(e.undo());
    assert_eq!(e.library(), &library);
    assert!(!e.take_library_changed());
}

#[test]
fn a_two_part_change_can_be_one_undo_step() {
    let m = Macro::new("Lib");
    let mut e = macro_key(&m, false);
    let new = Macro::new("New");
    e.save_macro(new.clone());
    e.merge_next();
    e.set_slot_action(
        Slot::Key(0),
        Action::Macro {
            id: new.id,
            own: None,
        },
    );
    assert!(e.undo());
    assert_eq!(linked(&e), m.id);
    assert!(e.library().get(new.id).is_none());
}

fn fx_editor() -> Editor {
    let mut p = Profile::new_default("P", Model::D200X);
    p.pages[0].keys.insert(1, Key::default());
    Editor::new(p)
}

#[test]
fn editing_an_effect_is_one_undo_step() {
    let mut e = fx_editor();
    e.set_field(&[1], Field::Effect(Effect::of(EffectKind::Fire)));
    assert_eq!(e.key(1).expect("key").look.effect.kind, EffectKind::Fire);
    e.undo();
    assert_eq!(e.key(1).expect("key").look.effect.kind, EffectKind::None);
}

#[test]
fn the_pressed_effect_is_a_field() {
    let mut e = fx_editor();
    let own = PressedEffect::Own(Effect::default());
    e.set_field(&[1], Field::Pressed(own));
    assert_eq!(e.key(1).expect("key").look.pressed, own);
}

#[test]
fn a_state_keeps_its_own_effect_when_another_changes() {
    let mut e = fx_editor();
    let state = |is: &str, kind| LookState {
        when: When {
            value: "{x}".into(),
            is: vec![is.into()],
            not: Vec::new(),
        },
        look: LookOverride {
            effect: Some(Effect::of(kind)),
            ..LookOverride::default()
        },
    };
    let two = vec![state("on", EffectKind::Chase), state("off", EffectKind::Fire)];
    e.set_field(&[1], Field::States(two.clone()));
    let mut next = two;
    next[1].look.effect = Some(Effect::of(EffectKind::Glow));
    e.set_field(&[1], Field::States(next));
    let states = &e.key(1).expect("key").look.states;
    assert_eq!(states[0].look.effect.map(|f| f.kind), Some(EffectKind::Chase));
    assert_eq!(states[1].look.effect.map(|f| f.kind), Some(EffectKind::Glow));
}

fn press(a: Action) -> impl FnOnce(&mut Dial) {
    move |d| d.press = Some(a)
}

/// Two pages, page 0 current.
fn two_pages() -> Editor {
    let mut e = editor_with_keys();
    e.add_page("Two");
    e.set_page(0);
    e
}

#[test]
fn a_library_dial_linked_on_two_pages_changes_both() {
    let mut e = two_pages();
    e.set_dial(1, press(Action::NextPage));
    let id = e.add_dial_to_library(1).expect("added");
    assert_eq!(e.dial_info(1).library, Some(id));
    assert_eq!(
        e.profile().library_dial(id).expect("in library").dial.name,
        "Dial 2"
    );
    assert_eq!(
        e.profile().pages[0].dials[1],
        Dial {
            library: Some(id),
            ..Dial::default()
        }
    );
    e.set_page(1);
    e.place_dial(1, id);
    e.set_dial(1, press(Action::PrevPage));
    assert_eq!(e.profile().dial_at(0, 1).press, Some(Action::PrevPage));
    assert_eq!(e.profile().dial_at(1, 1).press, Some(Action::PrevPage));
    assert_eq!(e.profile().dial_usage(id).pages, 2);
}

#[test]
fn make_a_copy_stops_the_link() {
    let mut e = two_pages();
    e.set_dial(0, |d| {
        d.press = Some(Action::NextPage);
        d.name = "Volume".into();
    });
    let id = e.add_dial_to_library(0).expect("added");
    e.set_page(1);
    e.place_dial(0, id);
    e.own_copy_dial(0);
    let info = e.dial_info(0);
    assert_eq!(info.library, None);
    assert_eq!(info.dial.name, "Volume (copy)");
    assert_eq!(info.dial.press, Some(Action::NextPage));
    e.set_dial(0, press(Action::PrevPage));
    assert_eq!(e.profile().dial_at(0, 0).press, Some(Action::NextPage));
    assert!(e.undo());
    assert!(e.undo());
    assert_eq!(e.dial_info(0).library, Some(id));
}

fn own(a: Action) -> Dial {
    Dial {
        press: Some(a),
        ..Dial::default()
    }
}

fn link(id: uuid::Uuid) -> Dial {
    Dial {
        library: Some(id),
        ..Dial::default()
    }
}

/// A profile with two pages and a library dial "Volume" (press = Back).
fn with_library() -> (Profile, uuid::Uuid) {
    let mut p = Profile::new_default("L", Model::D200X);
    p.pages.push(Page::new("Two"));
    let id = uuid::Uuid::new_v4();
    p.dial_library.push(LibraryDial {
        id,
        dial: Dial {
            name: "Volume".into(),
            ..own(Action::Back)
        },
    });
    (p, id)
}

#[test]
fn swap_moves_a_pin_with_its_dial() {
    let mut p = Profile::new_default("S", Model::D200X);
    p.pages[0].dials[0] = own(Action::NextPage);
    p.pages[0].dials[2] = own(Action::PrevPage);
    p.pinned.dials.insert(0, own(Action::Back));
    let before = p.clone();
    let mut e = Editor::new(p);
    e.swap_dials(0, 2);
    let info = e.dial_info(2);
    assert!(info.pinned);
    assert_eq!(info.dial.press, Some(Action::Back));
    assert!(!e.profile().pinned.dials.contains_key(&0));
    let info = e.dial_info(0);
    assert!(!info.pinned);
    assert_eq!(info.dial.press, Some(Action::PrevPage));
    assert_eq!(e.profile().pages[0].dials[2], own(Action::NextPage));
    assert!(e.undo());
    assert_eq!(e.profile(), &before);
}

#[test]
fn swap_without_pins_swaps_the_page_dials() {
    let mut e = editor_with_keys();
    e.set_dial(0, press(Action::NextPage));
    e.swap_dials(0, 1);
    assert_eq!(e.dial_info(1).dial.press, Some(Action::NextPage));
    assert!(e.dial_info(0).dial.is_empty());
    assert!(e.undo());
    assert_eq!(e.dial_info(0).dial.press, Some(Action::NextPage));
}

#[test]
fn pinning_keeps_the_page_dial_underneath() {
    let mut e = editor_with_keys();
    let two = e.add_page("Two");
    e.set_page(two);
    assert_eq!(e.add_page("Three"), 2);
    e.set_page(2);
    e.set_dial(1, press(Action::NextPage));
    e.pin_dial(1);
    assert_eq!(e.profile().pages[2].dials[1], own(Action::NextPage));
    e.set_page(0);
    let info = e.dial_info(1);
    assert!(info.pinned);
    assert_eq!(info.dial.press, Some(Action::NextPage));
    e.unpin_dial(1);
    // Page 0 had none of its own, so it keeps what showed.
    assert_eq!(e.dial_info(1).dial.press, Some(Action::NextPage));
    assert!(!e.dial_info(1).pinned);
    assert!(e.profile().dial_at(1, 1).is_empty());
    assert_eq!(e.profile().dial_at(2, 1).press, Some(Action::NextPage));
}

#[test]
fn unpin_reveals_the_page_dial() {
    let mut p = Profile::new_default("U", Model::D200X);
    p.pages.push(Page::new("Two"));
    p.pages[1].dials[0] = own(Action::NextPage);
    p.pinned.dials.insert(0, own(Action::PrevPage));
    let mut e = Editor::new(p);
    e.set_page(1);
    assert_eq!(e.dial_info(0).dial.press, Some(Action::PrevPage));
    e.unpin_dial(0);
    assert_eq!(e.dial_info(0).dial.press, Some(Action::NextPage));
    e.set_page(0);
    assert!(e.dial_info(0).dial.is_empty());
}

#[test]
fn unpin_keeps_the_dial_on_a_page_without_its_own() {
    // A pin from the one-time pass or old profile-wide dials, with nothing on the pages.
    let mut p = Profile::new_default("U", Model::D200X);
    p.pages.push(Page::new("Two"));
    p.pages.push(Page::new("Three"));
    p.pages[2].dials[0] = own(Action::NextPage);
    p.pinned.dials.insert(0, own(Action::PrevPage));
    let before = p.clone();
    let mut e = Editor::new(p);
    e.set_page(1);
    e.unpin_dial(0);
    let info = e.dial_info(0);
    assert!(!info.pinned);
    assert_eq!(info.dial.press, Some(Action::PrevPage));
    assert!(e.profile().dial_at(0, 0).is_empty());
    assert_eq!(e.profile().dial_at(2, 0).press, Some(Action::NextPage));
    assert!(e.undo());
    assert_eq!(e.profile(), &before);
}

#[test]
fn unpin_of_a_migrated_small_button_keeps_it_on_this_page() {
    let mut p = Profile::new_default("S", Model::D200X);
    p.pages.push(Page::new("Two"));
    let mut e = Editor::new(p);
    e.set_page(1);
    e.unpin_small(1);
    assert!(!e.profile().small_pinned(1));
    assert_eq!(e.profile().small_at(1, 1), Some(Action::NextPage));
    assert_eq!(e.profile().small_at(0, 1), None);
}

#[test]
fn pin_does_nothing_on_an_empty_spot() {
    let mut p = Profile::new_default("E", Model::D200X);
    p.small_buttons = SmallButtons::default();
    p.pages.push(Page::new("Two"));
    let mut e = Editor::new(p);
    e.pin_dial(0);
    e.pin_small(0);
    assert!(e.profile().pinned.is_empty());
    assert!(!e.profile().small_pinned(0));
    assert!(!e.can_undo());
}

#[test]
fn swap_with_a_pin_swaps_the_page_dials_on_every_page() {
    let mut p = Profile::new_default("S", Model::D200X);
    p.pages.push(Page::new("Two"));
    p.pages[0].dials[0] = own(Action::NextPage);
    p.pages[1].dials[0] = own(Action::Back);
    p.pages[1].dials[2] = own(Action::SleepNow);
    p.pinned.dials.insert(0, own(Action::PrevPage));
    let mut e = Editor::new(p);
    e.swap_dials(0, 2);
    assert_eq!(e.profile().pinned.dials[&2], own(Action::PrevPage));
    // The other page's own dials moved with it, so what shows there only changed position.
    assert_eq!(e.profile().dial_at(1, 0).press, Some(Action::SleepNow));
    assert_eq!(e.profile().pages[1].dials[2], own(Action::Back));
    assert_eq!(e.profile().pages[0].dials[2], own(Action::NextPage));
    assert!(e.profile().pages[0].dials[0].is_empty());
}

#[test]
fn dropping_on_a_pin_replaces_it() {
    let (mut p, id) = with_library();
    p.pages[0].dials[0] = own(Action::NextPage);
    p.pages[1].dials[0] = own(Action::PrevPage);
    p.pinned.dials.insert(0, own(Action::SleepNow));
    let pages = p.pages.clone();
    let mut e = Editor::new(p);
    e.place_dial(0, id);
    let info = e.dial_info(0);
    assert!(info.pinned);
    assert_eq!(info.library, Some(id));
    assert_eq!(info.dial.press, Some(Action::Back));
    assert_eq!(e.profile().pages, pages);
    assert!(e.undo());
    assert_eq!(e.dial_info(0).dial.press, Some(Action::SleepNow));
}

#[test]
fn deleting_a_library_dial_blanks_its_links_removes_its_pin_and_undo_restores() {
    let (mut p, id) = with_library();
    p.pages[0].dials[1] = link(id);
    p.pages[1].dials[1] = link(id);
    p.pinned.dials.insert(2, link(id));
    p.pages[1].dials[2] = own(Action::NextPage);
    let before = p.clone();
    let mut e = Editor::new(p);
    e.delete_library_dial(id);
    assert!(e.profile().dial_library.is_empty());
    assert_eq!(e.profile().pages[0].dials[1], Dial::default());
    assert_eq!(e.profile().pages[1].dials[1], Dial::default());
    // The pin goes; each page shows its own dial again.
    assert!(!e.profile().pinned.dials.contains_key(&2));
    assert_eq!(e.profile().dial_at(1, 2).press, Some(Action::NextPage));
    assert!(e.undo());
    assert_eq!(e.profile(), &before);
}

#[test]
fn library_dials_copy_and_paste() {
    let mut e = two_pages();
    e.set_dial(0, |d| {
        d.press = Some(Action::NextPage);
        d.name = "Volume".into();
    });
    let a = e.add_dial_to_library(0).expect("added");
    let b = e.new_library_dial();
    // Copied from the tray, it pastes onto the deck as a link.
    e.copy_library_dial(a);
    e.paste_dial(1);
    assert_eq!(e.dial_info(1).library, Some(a));
    // Pasted onto another library dial: that one takes the contents and keeps its id.
    e.paste_library_dial(b);
    let lib = e.profile().library_dial(b).expect("still there");
    assert_eq!(lib.dial.press, Some(Action::NextPage));
    assert_eq!(lib.dial.name, "Volume");
    assert_eq!(lib.dial.library, None);
    assert!(e.undo());
    assert_eq!(e.profile().library_dial(b).expect("b").dial.press, None);
    // Pasting a dial onto the library dial it links to changes nothing.
    let before = e.profile().clone();
    e.paste_library_dial(a);
    assert_eq!(e.profile(), &before);
}

#[test]
fn copy_paste_keeps_a_link_in_the_same_profile() {
    let (mut p, id) = with_library();
    p.pages[0].dials[0] = link(id);
    let mut e = Editor::new(p);
    assert!(!e.has_dial_clip());
    e.copy_dial(0);
    assert!(e.has_dial_clip());
    e.set_page(1);
    e.paste_dial(2);
    assert_eq!(e.dial_info(2).library, Some(id));
    assert_eq!(e.profile().pages[1].dials[2], link(id));
    assert_eq!(e.profile().dial_usage(id).pages, 2);
}

#[test]
fn paste_into_another_profile_makes_an_own_copy() {
    let (mut p, id) = with_library();
    p.pages[0].dials[0] = link(id);
    let mut e = Editor::new(p);
    e.copy_dial(0);
    e.replace_profile(Profile::new_default("Other", Model::D200X));
    e.paste_dial(1);
    let d = &e.profile().pages[0].dials[1];
    assert_eq!(d.library, None);
    assert_eq!(d.name, "Volume");
    assert_eq!(d.press, Some(Action::Back));
}

#[test]
fn a_new_page_starts_without_dials() {
    let mut e = editor_with_keys();
    e.set_dial(0, press(Action::NextPage));
    e.set_dial(2, press(Action::Back));
    e.pin_dial(2);
    let at = e.add_page("Two");
    assert!(e.profile().pages[at].dials.iter().all(Dial::is_empty));
    e.set_page(at);
    assert!(e.dial_info(0).dial.is_empty());
    assert_eq!(e.dial_info(2).dial.press, Some(Action::Back));
}

#[test]
fn rename_trims_and_follows_the_link() {
    let (mut p, id) = with_library();
    p.pages[0].dials[0] = link(id);
    let mut e = Editor::new(p);
    e.rename_dial(0, "  Lights  ");
    assert_eq!(e.profile().library_dial(id).expect("kept").dial.name, "Lights");
    assert_eq!(e.profile().pages[0].dials[0], link(id));
    e.rename_dial(1, " Scroll ");
    assert_eq!(e.profile().pages[0].dials[1].name, "Scroll");
    assert!(e.undo());
    assert!(e.undo());
    assert_eq!(e.dial_info(0).dial.name, "Volume");
}

#[test]
fn a_library_dial_is_edited_and_named_by_its_id() {
    let (p, id) = with_library();
    let mut e = Editor::new(p);
    e.set_library_dial(id, press(Action::NextPage));
    e.rename_library_dial(id, "  Lights  ");
    // It keeps a name; an unknown id changes nothing.
    e.rename_library_dial(id, "  ");
    e.set_library_dial(uuid::Uuid::new_v4(), press(Action::Back));
    let dial = &e.profile().library_dial(id).expect("kept").dial;
    assert_eq!(
        (dial.press.clone(), dial.name.as_str()),
        (Some(Action::NextPage), "Lights")
    );
    assert!(e.undo());
    assert!(e.undo());
    assert!(!e.undo());
    assert_eq!(e.profile().library_dial(id).expect("kept").dial.name, "Volume");
}

#[test]
fn drop_action_on_a_linked_dial_edits_the_library_dial() {
    let (mut p, id) = with_library();
    p.pages[0].dials[0] = link(id);
    let mut e = Editor::new(p);
    e.drop_action(Slot::Dial(0), Action::NextPage);
    assert_eq!(
        e.profile().library_dial(id).expect("kept").dial.press,
        Some(Action::NextPage)
    );
    assert_eq!(e.profile().pages[0].dials[0], link(id));
    e.set_slot_action(Slot::Dial(0), Action::PrevPage);
    assert_eq!(
        e.profile().library_dial(id).expect("kept").dial.press,
        Some(Action::PrevPage)
    );
}

#[test]
fn new_and_remove_dials() {
    let mut e = editor_with_keys();
    let id = e.new_library_dial();
    assert_eq!(e.profile().library_dial(id).expect("made").dial.name, "New Dial");
    e.set_dial(0, press(Action::NextPage));
    e.pin_dial(0);
    e.set_dial(1, press(Action::Back));
    e.remove_dial(1);
    assert!(e.dial_info(1).dial.is_empty());
    // A pin is left alone; only the page's own dial goes.
    e.remove_dial(0);
    assert!(e.dial_info(0).pinned);
    assert_eq!(e.dial_info(0).dial.press, Some(Action::NextPage));
    assert!(e.profile().pages[0].dials[0].is_empty());
}

#[test]
fn dial_operations_leave_a_foreign_dial_alone() {
    let (mut p, id) = with_library();
    let f = with_foreign();
    p.foreign = f.foreign.clone();
    for f in &mut p.foreign {
        f.page = Some(p.pages[0].id);
    }
    p.pages[0].dials[0] = own(Action::NextPage);
    let before = p.clone();
    let mut e = Editor::new(p);
    assert_eq!(e.add_dial_to_library(1), None);
    e.place_dial(1, id);
    e.pin_dial(1);
    e.rename_dial(1, "X");
    e.remove_dial(1);
    e.swap_dials(0, 1);
    e.copy_dial(0);
    e.paste_dial(1);
    assert_eq!(e.profile(), &before);
    assert!(!e.can_undo());
}

#[test]
fn small_buttons_edit_the_pin_or_the_page() {
    let mut p = Profile::new_default("S", Model::D200X);
    p.pages.push(Page::new("Two"));
    let mut e = Editor::new(p);
    // Both sides are pinned in a new profile: the edit changes the pin.
    e.set_small_button(0, Some(Action::Back));
    assert_eq!(e.profile().small_buttons.left, Some(Action::Back));
    assert!(e.profile().pages[0].small_buttons.left.is_none());
    // Unpinned, the edit changes this page only.
    e.unpin_small(0);
    assert!(!e.profile().small_pinned(0));
    assert!(e.profile().small_pinned(1));
    e.set_small_button(0, Some(Action::SleepNow));
    assert_eq!(e.profile().pages[0].small_buttons.left, Some(Action::SleepNow));
    assert!(e.profile().pages[1].small_buttons.left.is_none());
    assert_eq!(e.profile().small_at(1, 0), None);
    // Pinning copies what shows to every page.
    e.pin_small(0);
    assert_eq!(e.profile().small_buttons.left, Some(Action::SleepNow));
    assert_eq!(e.profile().small_at(1, 0), Some(Action::SleepNow));
    // The slot ops follow the same rule.
    e.unpin_small(1);
    e.drop_action(Slot::Small(1), Action::Back);
    assert_eq!(e.profile().pages[0].small_buttons.right, Some(Action::Back));
    e.set_slot_action(Slot::Small(0), Action::NextPage);
    assert_eq!(e.profile().small_buttons.left, Some(Action::NextPage));
}
