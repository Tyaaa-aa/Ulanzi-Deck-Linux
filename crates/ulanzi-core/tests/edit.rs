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
    assert_eq!(key(&e, 0).look, before);
    assert_eq!(key(&e, 0).tap, Some(Action::SleepNow));
}

#[test]
fn drop_action_on_dial_and_small_and_screen() {
    let mut e = editor_with_keys();
    e.drop_action(Slot::Small(1), Action::Back);
    assert_eq!(e.profile().small_buttons.right, Some(Action::Back));
    e.set_dial(1, |d| d.left = Some(Action::PrevPage));
    assert_eq!(e.profile().dials[1].left, Some(Action::PrevPage));
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
