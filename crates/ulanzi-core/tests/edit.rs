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
    assert_eq!(e.dials()[1].left, Some(Action::PrevPage));
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
    // A new page starts with the dials of the page it was added from.
    let two = e.add_page("Two");
    e.set_page(two);
    assert_eq!(e.dials()[0].press, Some(Action::NextPage));
    e.set_dial(0, |d| d.press = Some(Action::PrevPage));
    e.set_page(0);
    assert_eq!(e.dials()[0].press, Some(Action::NextPage));
    // Copy one dial to every page.
    e.dial_to_all_pages(0);
    e.set_page(two);
    assert_eq!(e.dials()[0].press, Some(Action::NextPage));
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
    assert_eq!(e.dials()[1], Dial::default());
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
    assert!(e.take_library_changed() == false);
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
