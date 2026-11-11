//! The key library and pinned keys: model lookups, saving, and every edit that reaches them.

use ulanzi_core::edit::{Editor, Field, Slot};
use ulanzi_core::model::*;
use ulanzi_core::store::{parse_profile, profile_to_toml};
use uuid::Uuid;

fn labelled(text: &str, tap: Action) -> Key {
    Key {
        look: KeyLook {
            label: Label {
                text: text.into(),
                ..Label::default()
            },
            ..KeyLook::default()
        },
        tap: Some(tap),
        ..Key::default()
    }
}

fn link(id: Uuid) -> Key {
    Key {
        library: Some(id),
        ..Key::default()
    }
}

/// Two pages; key 0 on page 0 is "Mute".
fn two_pages() -> Editor {
    let mut p = Profile::new_default("K", Model::D200X);
    p.pages.push(Page::new("Two"));
    p.pages[0].keys.insert(0, labelled("Mute", Action::NextPage));
    Editor::new(p)
}

fn label(e: &Editor, page: usize, n: u8) -> Option<String> {
    e.profile().key_at(page, n).map(|k| k.look.label.text)
}

#[test]
fn library_links_and_pins_round_trip_and_need_2_6() {
    let mut p = Profile::new_default("K", Model::D200X);
    let id = Uuid::new_v4();
    p.key_library.push(LibraryKey {
        id,
        name: "Mute".into(),
        key: labelled("Mute", Action::NextPage),
    });
    p.pages[0].keys.insert(3, link(id));
    p.pinned.keys.insert(12, labelled("Home", Action::PrevPage));
    let text = profile_to_toml(&p).expect("toml");
    assert!(text.contains("min_app = \"2.6.0\""), "{text}");
    let back = parse_profile(&text).expect("parse");
    assert_eq!(back.key_library, p.key_library);
    assert_eq!(back.pinned.keys, p.pinned.keys);
    assert_eq!(back.pages[0].keys[&3], link(id));
}

#[test]
fn lookups_resolve_pins_then_own_then_links() {
    let mut p = Profile::new_default("K", Model::D200X);
    p.pages.push(Page::new("Two"));
    let id = Uuid::new_v4();
    p.key_library.push(LibraryKey {
        id,
        name: "Lib".into(),
        key: labelled("Lib", Action::NextPage),
    });
    p.pages[0].keys.insert(0, link(id));
    p.pages[0].keys.insert(1, labelled("Own", Action::NextPage));
    p.pages[1].keys.insert(1, labelled("Under", Action::NextPage));
    p.pinned.keys.insert(1, labelled("Pin", Action::NextPage));
    // A link to nothing shows no key.
    p.pages[0].keys.insert(2, link(Uuid::new_v4()));
    let info = p.key_info(0, 0);
    assert_eq!(info.library, Some(id));
    assert_eq!(info.key.expect("key").library, None);
    assert_eq!(p.key_at(0, 0).map(|k| k.look.label.text), Some("Lib".into()));
    assert_eq!(p.key_at(1, 1).map(|k| k.look.label.text), Some("Pin".into()));
    assert!(p.key_info(1, 1).pinned);
    assert_eq!(p.key_at(0, 2), None);
    let all = p.keys_at(1);
    assert_eq!(all.keys().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(p.keys_at(0).keys().copied().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(
        p.key_usage(id),
        Usage {
            pages: 1,
            pinned: false
        }
    );
}

#[test]
fn an_unreadable_library_key_survives_a_save() {
    let text = r#"
version = 1
id = "6f1b1c0e-0000-4000-8000-000000000001"
name = "K"
model = "D200"

[[key_library]]
id = "6f1b1c0e-0000-4000-8000-0000000000b1"
name = "Ok"

[[key_library]]
id = "6f1b1c0e-0000-4000-8000-0000000000b2"
name = "Future"
future = 1

[pinned.keys."4"]
future = 2

[[pages]]
id = "6f1b1c0e-0000-4000-8000-000000000002"
name = "Main"
"#;
    let p = parse_profile(text).expect("ok");
    assert_eq!(p.key_library.len(), 1);
    assert_eq!(p.foreign_key_library.len(), 1);
    assert!(
        p.foreign_shown(0, Slot::Key(4)).is_some(),
        "a pin from a newer version covers the key"
    );
    let out = profile_to_toml(&p).expect("ok");
    let t: toml::Table = toml::from_str(&out).expect("toml");
    assert_eq!(t["key_library"].as_array().expect("array").len(), 2);
    assert!(t["pinned"]["keys"].get("4").is_some());
}

#[test]
fn dropping_the_library_keeps_what_shows() {
    let mut e = two_pages();
    let id = e.add_key_to_library(0).expect("added");
    e.set_page(1);
    e.place_key(5, id);
    let mut p = e.profile().clone();
    p.pages[1].keys.insert(6, link(Uuid::new_v4()));
    p.drop_library();
    assert!(p.key_library.is_empty());
    assert_eq!(p.pages[0].keys[&0].look.label.text, "Mute");
    assert_eq!(p.pages[1].keys[&5].library, None);
    assert!(!p.pages[1].keys.contains_key(&6), "a link to nothing is dropped");
    assert!(!p.uses_key_library());
}

#[test]
fn add_to_library_links_here_and_edits_reach_every_page() {
    let mut e = two_pages();
    let id = e.add_key_to_library(0).expect("added");
    assert_eq!(e.profile().library_key(id).expect("lib").name, "Mute");
    assert_eq!(e.profile().pages[0].keys[&0], link(id));
    assert_eq!(e.add_key_to_library(0), Some(id), "already linked");
    assert_eq!(e.add_key_to_library(7), None, "nothing to add");
    e.set_page(1);
    e.place_key(2, id);
    e.set_field(&[2], Field::LabelText("Muted".into()));
    assert_eq!(label(&e, 0, 0).as_deref(), Some("Muted"));
    e.set_key_actions(2, Some(Action::PrevPage), None);
    assert_eq!(
        e.profile().key_at(0, 0).and_then(|k| k.tap),
        Some(Action::PrevPage)
    );
    assert_eq!(
        e.profile().key_usage(id),
        Usage {
            pages: 2,
            pinned: false
        }
    );
    // Make a Copy stops the link on this page only.
    e.own_copy_key(2);
    e.set_field(&[2], Field::LabelText("Mine".into()));
    assert_eq!(label(&e, 1, 2).as_deref(), Some("Mine"));
    assert_eq!(label(&e, 0, 0).as_deref(), Some("Muted"));
    assert_eq!(e.key_info(2).library, None);
}

#[test]
fn a_suggested_name_skips_live_labels() {
    let k = labelled("{volume}", Action::NextPage);
    assert_eq!(k.suggested_name(), "Next Page");
    assert_eq!(labelled(" Mute ", Action::NextPage).suggested_name(), "Mute");
    assert_eq!(Key::default().suggested_name(), "Key");
}

#[test]
fn pin_shows_everywhere_and_unpin_leaves_an_empty_key_empty() {
    let mut e = two_pages();
    e.pin_key(0);
    assert!(e.key_info(0).pinned);
    assert_eq!(label(&e, 1, 0).as_deref(), Some("Mute"));
    // Edits of a pinned key change the pin, on every page.
    e.set_page(1);
    e.set_field(&[0], Field::LabelText("Quiet".into()));
    assert_eq!(label(&e, 0, 0).as_deref(), Some("Quiet"));
    assert_eq!(
        e.profile().pages[0].keys[&0].look.label.text,
        "Mute",
        "own key kept underneath"
    );
    e.unpin_key(0);
    // Edited while pinned: no page has that copy, so this page keeps it.
    assert_eq!(label(&e, 1, 0).as_deref(), Some("Quiet"));
    assert_eq!(label(&e, 0, 0).as_deref(), Some("Mute"));

    let mut e = two_pages();
    e.pin_key(0);
    e.set_page(1);
    e.unpin_key(0);
    assert_eq!(e.profile().key_at(1, 0), None, "page 2 had no key there");
    assert_eq!(label(&e, 0, 0).as_deref(), Some("Mute"));
    assert!(e.undo());
    assert!(e.key_info(0).pinned);
}

#[test]
fn nothing_to_pin_on_an_empty_key() {
    let mut e = two_pages();
    e.pin_key(4);
    assert!(e.profile().pinned.keys.is_empty());
}

#[test]
fn a_pinned_key_takes_its_spot_on_every_page() {
    let mut e = two_pages();
    e.pin_key(0);
    e.set_page(1);
    assert_eq!(e.free_keys(), 12);
    // Clear leaves the pin alone.
    e.clear(&[0]);
    assert!(e.key_info(0).pinned);
    // Adding keys skips it.
    let at = e.add_keys(vec![vec![labelled("A", Action::NextPage)]], "New", false);
    assert_eq!(at, 1);
    assert!(e.profile().pages[1].keys.contains_key(&1));
    assert!(!e.profile().pages[1].keys.contains_key(&0));
}

#[test]
fn swapping_a_pinned_key_moves_the_pin_and_every_page() {
    let mut e = two_pages();
    e.set_page(1);
    e.set_field(&[3], Field::LabelText("P2".into()));
    e.set_page(0);
    e.pin_key(0);
    e.swap(0, 3);
    assert!(e.profile().pinned.keys.contains_key(&3));
    assert!(!e.profile().pinned.keys.contains_key(&0));
    assert_eq!(
        label(&e, 1, 0).as_deref(),
        Some("P2"),
        "page 2's own key moved too"
    );
    assert_eq!(label(&e, 1, 3).as_deref(), Some("Mute"));
    // Without pins only this page swaps.
    let mut e = two_pages();
    e.set_page(1);
    e.set_field(&[3], Field::LabelText("P2".into()));
    e.set_page(0);
    e.swap(0, 3);
    assert_eq!(label(&e, 1, 3).as_deref(), Some("P2"));
}

#[test]
fn copied_links_stay_links_in_their_profile() {
    let mut e = two_pages();
    let id = e.add_key_to_library(0).expect("added");
    e.copy_keys(&[0]);
    e.set_page(1);
    e.paste_keys(4);
    assert_eq!(e.profile().pages[1].keys[&4], link(id));
    // Another profile: an own copy.
    let mut other = Editor::new(Profile::new_default("O", Model::D200X));
    e.replace_profile(Profile::new_default("O", Model::D200X));
    e.paste_keys(1);
    let k = e.profile().pages[0].keys[&1].clone();
    assert_eq!(k.library, None);
    assert_eq!(k.look.label.text, "Mute");
    other.paste_keys(0);
    assert!(other.profile().pages[0].keys.is_empty());
}

#[test]
fn library_keys_copy_paste_move_rename_and_delete() {
    let mut e = two_pages();
    let a = e.add_key_to_library(0).expect("added");
    let b = e.new_library_key();
    assert_eq!(e.profile().library_key(b).expect("b").name, "New Key");
    e.copy_library_keys(&[a]);
    // Pasted in the library: a copy named after the original, numbered.
    let copy = e.paste_into_key_library(None);
    let lib = e.profile().library_key(copy[0]).expect("copy");
    assert_eq!(
        (lib.name.as_str(), lib.key.look.label.text.as_str()),
        ("Mute 2", "Mute")
    );
    assert!(e.undo());
    e.rename_library_key(b, "  Other ");
    e.rename_library_key(b, " ");
    assert_eq!(e.profile().library_key(b).expect("b").name, "Other");
    e.move_library_key(b, 0);
    assert_eq!(e.profile().key_library[0].id, b);
    // Paste the copied link onto the deck: linked.
    e.set_page(1);
    e.paste_keys(9);
    assert_eq!(e.profile().pages[1].keys[&9], link(a));
    e.set_page(0);
    e.pin_key(0);
    e.delete_library_key(a);
    assert!(e.profile().library_key(a).is_none());
    assert!(e.profile().pinned.keys.is_empty(), "its pin went");
    assert_eq!(e.profile().key_at(1, 9), None);
    assert!(e.undo());
    assert!(e.profile().library_key(a).is_some());
}

#[test]
fn a_library_key_is_edited_in_place_and_every_use_follows() {
    use ulanzi_core::edit::LIBRARY_KEY;
    let mut e = two_pages();
    let id = e.add_key_to_library(0).expect("added");
    e.set_page(1);
    e.place_key(4, id);
    assert_eq!(e.key(LIBRARY_KEY), None, "nothing picked");
    e.edit_library_key(Some(id));
    assert_eq!(
        e.key(LIBRARY_KEY).map(|k| k.look.label.text.as_str()),
        Some("Mute")
    );
    assert_eq!(e.key_info(LIBRARY_KEY).library, Some(id));
    e.set_field(&[LIBRARY_KEY], Field::LabelText("Quiet".into()));
    e.set_key_actions(LIBRARY_KEY, Some(Action::PrevPage), None);
    assert_eq!(label(&e, 0, 0).as_deref(), Some("Quiet"));
    assert_eq!(label(&e, 1, 4).as_deref(), Some("Quiet"));
    assert_eq!(
        e.profile().key_at(1, 4).and_then(|k| k.tap),
        Some(Action::PrevPage)
    );
    assert!(
        !e.profile().pinned.keys.contains_key(&LIBRARY_KEY),
        "the stand-in never stays in the profile"
    );
    // One step each, undone like any edit.
    assert!(e.undo());
    assert!(e.undo());
    assert_eq!(label(&e, 1, 4).as_deref(), Some("Mute"));
    // A blank new library key is built the same way.
    let fresh = e.new_library_key();
    e.edit_library_key(Some(fresh));
    e.drop_action(Slot::Key(LIBRARY_KEY), Action::NextPage);
    assert_eq!(
        e.profile().library_key(fresh).and_then(|l| l.key.tap.clone()),
        Some(Action::NextPage)
    );
}

#[test]
fn several_keys_pin_and_unpin_in_one_step() {
    let mut e = two_pages();
    e.set_field(&[1], Field::LabelText("B".into()));
    e.pin_keys(&[0, 1, 5]);
    assert!(e.key_info(0).pinned && e.key_info(1).pinned);
    assert!(!e.key_info(5).pinned, "nothing there to pin");
    assert!(e.undo());
    assert!(e.profile().pinned.keys.is_empty());
    e.pin_keys(&[0, 1]);
    e.unpin_keys(&[0, 1]);
    assert!(e.profile().pinned.keys.is_empty());
    assert_eq!(label(&e, 0, 1).as_deref(), Some("B"));
}

#[test]
fn deleting_a_library_key_can_keep_its_keys() {
    let mut e = two_pages();
    let id = e.add_key_to_library(0).expect("added");
    e.set_page(1);
    e.place_key(4, id);
    e.pin_key(4);
    e.delete_library_key_keep_uses(id);
    assert!(e.profile().library_key(id).is_none());
    let own = &e.profile().pages[0].keys[&0];
    assert_eq!((own.library, own.look.label.text.as_str()), (None, "Mute"));
    assert_eq!(e.profile().pinned.keys[&4].library, None);
    assert_eq!(label(&e, 1, 4).as_deref(), Some("Mute"));
}

#[test]
fn a_pin_hides_the_page_key_only_when_it_differs() {
    let mut e = two_pages();
    // Unpinned: nothing is hidden, however the key looks.
    assert_eq!(e.profile().covered_key(0, 0), None);
    let mut p = e.profile().clone();
    p.pinned.keys.insert(0, labelled("Home", Action::PrevPage));
    assert_eq!(
        p.covered_key(0, 0).map(|k| k.look.label.text),
        Some("Mute".into())
    );
    // The same key pinned hides nothing.
    p.pinned.keys.insert(0, labelled("Mute", Action::NextPage));
    assert_eq!(p.covered_key(0, 0), None);
    // Both linking one library key hide nothing, nor a pin over an empty spot.
    let id = Uuid::new_v4();
    p.key_library.push(LibraryKey {
        id,
        name: "Lamp".into(),
        key: labelled("Lamp", Action::NextPage),
    });
    p.pinned.keys.insert(0, link(id));
    p.pages[0].keys.insert(0, link(id));
    assert_eq!(p.covered_key(0, 0), None);
    assert_eq!(p.covered_key(1, 0), None);
    e = Editor::new(p);
    assert!(e.profile().pinned.keys.contains_key(&0));
}

#[test]
fn several_keys_go_into_the_library_in_one_step_without_duplicates() {
    let mut e = two_pages();
    let mut p = e.profile().clone();
    p.pages[0].keys.insert(1, labelled("Lamp", Action::PrevPage));
    p.pages[0].keys.insert(2, labelled("Lamp", Action::PrevPage));
    p.pages[0].keys.insert(3, labelled("Fan", Action::NextPage));
    e = Editor::new(p);
    e.add_key_to_library(3);
    let added = e.add_keys_to_library(&[0, 1, 2, 3, 4]);
    // Key 2 matches key 1 added a moment before; key 3 was linked; key 4 is empty.
    assert_eq!(
        added,
        ulanzi_core::edit::AddedKeys {
            new: 2,
            matched: 1,
            already: 1
        }
    );
    let lib = &e.profile().key_library;
    assert_eq!(lib.len(), 3);
    let k = &e.profile().pages[0].keys;
    assert_eq!(k[&1].library, k[&2].library);
    assert!(k[&0].library.is_some());
    assert_eq!(label(&e, 0, 2), Some("Lamp".into()));
    e.undo();
    assert_eq!(e.profile().key_library.len(), 1);
    assert_eq!(
        added.message().as_deref(),
        Some("Added 3 keys to your library. 1 matched a library key and links to it. 1 was in it already.")
    );
}

#[test]
fn library_keys_copy_into_another_profile_and_delete_together() {
    let mut e = two_pages();
    let a = e.add_key_to_library(0).expect("added");
    let b = e.new_library_key();
    e.place_key(5, b);
    e.copy_library_keys(&[a, b]);
    // Another profile: the clipboard comes along and pastes as own keys, or new library keys.
    let clip = e.take_clipboard();
    let mut other = Editor::new(Profile::new_default("O", Model::D200X));
    other.set_clipboard(clip);
    other.paste_keys(2);
    let k = &other.profile().pages[0].keys;
    assert_eq!((k[&2].library, k[&2].look.label.text.as_str()), (None, "Mute"));
    assert!(k.contains_key(&3), "side by side");
    let ids = other.paste_into_key_library(None);
    let names: Vec<_> = other
        .profile()
        .key_library
        .iter()
        .map(|l| l.name.clone())
        .collect();
    assert_eq!(names, vec!["Mute", "New Key"]);
    assert_eq!(ids.len(), 2);
    // Back home: both deleted in one step, the keys that used them kept as own copies.
    e.set_clipboard(other.take_clipboard());
    e.delete_library_keys_keep_uses(&[a, b]);
    assert!(e.profile().key_library.is_empty());
    assert_eq!(label(&e, 0, 0), Some("Mute".into()));
    assert!(e.profile().pages[0].keys.contains_key(&5));
    assert!(e.undo());
    e.delete_library_keys(&[a, b]);
    assert!(e.profile().pages[0].keys.is_empty());
    assert!(e.undo());
    assert_eq!(e.profile().key_library.len(), 2);
}

#[test]
fn copied_pictures_come_along_to_another_profile() {
    let mut k = labelled("Scream", Action::NextPage);
    k.look.source = ImageSource::Image {
        path: "icons/aa.png".into(),
    };
    k.look.states.push(LookState {
        when: When {
            value: "{x}".into(),
            is: vec!["on".into()],
            not: vec![],
        },
        look: LookOverride {
            source: Some(ImageSource::Image {
                path: "icons/bb.gif".into(),
            }),
            ..LookOverride::default()
        },
    });
    let mut p = Profile::new_default("K", Model::D200X);
    p.pages[0].keys.insert(1, k);
    let mut e = Editor::new(p);
    let id = e.add_key_to_library(1).expect("added");
    e.copy_library_keys(&[id]);
    let clip = e.take_clipboard();
    assert_eq!(clip.images(), vec!["icons/aa.png", "icons/bb.gif"]);

    let d = tempfile::tempdir().expect("dir");
    let (from, to) = (d.path().join("a"), d.path().join("b"));
    std::fs::create_dir_all(from.join("icons")).expect("dir");
    std::fs::write(from.join("icons/aa.png"), b"png").expect("write");
    std::fs::write(from.join("icons/bb.gif"), b"gif").expect("write");
    clip.bring_images(&from, &to);
    assert_eq!(std::fs::read(to.join("icons/aa.png")).expect("copied"), b"png");
    assert_eq!(std::fs::read(to.join("icons/bb.gif")).expect("copied"), b"gif");
}
