use ulanzi_core::bundle::{Conflict, ImportOptions, commands, export, import, read};
use ulanzi_core::model::*;
use ulanzi_core::snapshot;
use ulanzi_core::store::{Store, profile_to_toml};
use uuid::Uuid;

fn store(d: &std::path::Path) -> Store {
    Store::at(d.join("c"), d.join("s"), d.join("k"))
}

fn step(event: MacroEvent, delay_ms: u32) -> MacroStep {
    MacroStep {
        event,
        delay_ms,
        look: LookOverride::default(),
        part: SequencePart::Press,
    }
}

fn down(key: &str) -> MacroEvent {
    MacroEvent::KeyDown { key: key.into() }
}

fn up(key: &str) -> MacroEvent {
    MacroEvent::KeyUp { key: key.into() }
}

fn every_event() -> Macro {
    let mut m = Macro::new("All");
    m.steps = vec![
        step(down("ctrl"), 10),
        step(up("ctrl"), 20),
        step(
            MacroEvent::MouseDown {
                button: MouseButton::Left,
            },
            0,
        ),
        step(
            MacroEvent::MouseUp {
                button: MouseButton::Forward,
            },
            0,
        ),
        step(MacroEvent::TypeText { text: "hi".into() }, 0),
        step(MacroEvent::Scroll { dx: 0, dy: -3 }, 0),
        step(MacroEvent::MoveBy { dx: 5, dy: 6 }, 0),
        step(MacroEvent::MoveTo { x: 0.25, y: 0.5 }, 0),
        step(MacroEvent::Wait { ms: 100 }, 0),
        step(MacroEvent::RunMacro { id: Uuid::new_v4() }, 0),
        step(
            MacroEvent::Action {
                action: Action::NextPage,
            },
            0,
        ),
    ];
    m
}

#[test]
fn a_macro_round_trips_through_toml() {
    for trigger in [
        Trigger::Once {
            again: PressAgain::Queue,
        },
        Trigger::WhileHeld {
            on_release: Release::StopNow,
        },
        Trigger::Toggle,
        Trigger::Sequence,
    ] {
        let mut m = every_event();
        m.trigger = trigger;
        m.standard_delay_ms = Some(50);
        m.feedback = Feedback::Count { show_total: false };
        m.previous = Some(Box::new(Macro::new("Old")));
        let lib = MacroLibrary { macros: vec![m] };
        let d = tempfile::tempdir().expect("tmp");
        let s = store(d.path());
        s.save_macros(&lib).expect("save");
        assert_eq!(s.load_macros().expect("load"), lib);
    }
}

#[test]
fn a_missing_library_is_empty() {
    let d = tempfile::tempdir().expect("tmp");
    assert_eq!(
        store(d.path()).load_macros().expect("load"),
        MacroLibrary::default()
    );
}

#[test]
fn standard_delays_replace_recorded_ones_without_losing_them() {
    let mut m = Macro::new("D");
    m.steps = vec![step(down("a"), 175)];
    assert_eq!(m.effective_delay(&m.steps[0]), 175);
    m.standard_delay_ms = Some(50);
    assert_eq!(m.effective_delay(&m.steps[0]), 50);
    assert_eq!(m.steps[0].delay_ms, 175);
    m.standard_delay_ms = None;
    assert_eq!(m.effective_delay(&m.steps[0]), 175);
}

#[test]
fn a_key_left_down_is_reported() {
    let mut m = Macro::new("Held");
    m.steps = vec![step(down("ctrl"), 0), step(down("a"), 0), step(up("a"), 0)];
    assert_eq!(m.end_state(), vec!["ctrl".to_string()]);
}

fn with_macro_key(id: Uuid, own: Option<Macro>) -> Profile {
    let mut p = Profile::new_default("P", Model::D200X);
    p.pages[0].keys.insert(
        0,
        Key {
            tap: Some(Action::Macro {
                id,
                own: own.map(Box::new),
            }),
            ..Key::default()
        },
    );
    p
}

#[test]
fn a_profile_using_a_macro_needs_2_2() {
    let text = profile_to_toml(&with_macro_key(Uuid::new_v4(), None)).expect("toml");
    assert!(text.contains("min_app = \"2.2.0\""), "{text}");
}

#[test]
fn a_bundle_carries_the_macros_its_profiles_use() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let (mut used, nested, other) = (Macro::new("Used"), Macro::new("Nested"), Macro::new("Other"));
    used.steps = vec![step(MacroEvent::RunMacro { id: nested.id }, 0)];
    let p = with_macro_key(used.id, None);
    s.save_profile(&p).expect("save");
    s.save_macros(&MacroLibrary {
        macros: vec![used.clone(), nested.clone(), other],
    })
    .expect("macros");
    let b = read(&export(&s, &[p.id], false).expect("export")).expect("read");
    let mut names: Vec<&str> = b.macros.macros.iter().map(|m| m.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Nested", "Used"]);

    let e = tempfile::tempdir().expect("tmp");
    let there = store(e.path());
    import(&there, &b, &keep_both()).expect("import");
    assert_eq!(there.load_macros().expect("load").macros.len(), 2);
}

fn keep_both() -> ImportOptions {
    ImportOptions {
        conflict: Conflict::KeepBoth,
        keep_rules: true,
        settings: false,
        device: None,
    }
}

#[test]
fn import_asks_before_replacing_a_macro_that_differs() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let m = Macro::new("Mine");
    let p = with_macro_key(m.id, None);
    s.save_profile(&p).expect("save");
    s.save_macros(&MacroLibrary {
        macros: vec![m.clone()],
    })
    .expect("macros");
    let bytes = export(&s, &[p.id], false).expect("export");

    let e = tempfile::tempdir().expect("tmp");
    let there = store(e.path());
    let mut changed = m.clone();
    changed.name = "Theirs".into();
    let lib = MacroLibrary {
        macros: vec![changed],
    };
    there.save_macros(&lib).expect("macros");
    let r = import(&there, &read(&bytes).expect("read"), &keep_both()).expect("import");
    assert_eq!(r.macro_conflicts, vec!["Mine".to_string()]);
    assert_eq!(there.load_macros().expect("load"), lib);
}

#[test]
fn commands_inside_macros_are_in_the_import_preview() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let cmd = |c: &str| {
        step(
            MacroEvent::Action {
                action: Action::Command {
                    cmd: c.into(),
                    terminal: false,
                },
            },
            0,
        )
    };
    let mut linked = Macro::new("Linked");
    linked.steps = vec![cmd("from the library")];
    let mut own = Macro::new("Own");
    own.steps = vec![cmd("from the key")];
    let mut p = with_macro_key(linked.id, None);
    p.pages[0].keys.insert(
        1,
        Key {
            tap: Some(Action::Macro {
                id: Uuid::new_v4(),
                own: Some(Box::new(own)),
            }),
            ..Key::default()
        },
    );
    s.save_profile(&p).expect("save");
    s.save_macros(&MacroLibrary { macros: vec![linked] })
        .expect("macros");
    let b = read(&export(&s, &[p.id], false).expect("export")).expect("read");
    let found = commands(&b);
    assert!(found.contains(&"from the library".to_string()), "{found:?}");
    assert!(found.contains(&"from the key".to_string()), "{found:?}");
}

#[test]
fn backups_hold_the_library_and_restore_it() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    s.save_profile(&Profile::new_default("P", Model::D200X))
        .expect("save");
    let lib = MacroLibrary {
        macros: vec![Macro::new("Kept")],
    };
    s.save_macros(&lib).expect("macros");
    let snap = snapshot::take(&s, "manual").expect("snapshot");
    s.save_macros(&MacroLibrary::default()).expect("clear");
    snapshot::restore(&s, &snap.name).expect("restore");
    assert_eq!(s.load_macros().expect("load"), lib);
}
