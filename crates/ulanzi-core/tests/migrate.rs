use std::path::{Path, PathBuf};

use ulanzi_core::migrate::migrate_legacy;
use ulanzi_core::model::*;
use ulanzi_core::store::{ProfileLoad, Store};
use ulanzi_core::upgrade::{PINNED_MARKER, pin_shared_dials_once};
use ulanzi_core::{Color, CoreError};
use uuid::Uuid;

struct Env {
    _dir: tempfile::TempDir,
    yaml: PathBuf,
    store: Store,
}

/// Copy the fixture config + icons into a temp "~/.config/ulanzi".
fn env() -> Env {
    let dir = tempfile::tempdir().expect("tempdir");
    let legacy = dir.path().join("legacy");
    std::fs::create_dir_all(legacy.join("icons")).expect("mkdir");
    let fx = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    std::fs::copy(fx.join("legacy-config.yaml"), legacy.join("config.yaml")).expect("copy yaml");
    for f in ["red.png", "speaker.png"] {
        std::fs::copy(fx.join("icons").join(f), legacy.join("icons").join(f)).expect("copy icon");
    }
    let store = Store::at(
        dir.path().join("config"),
        dir.path().join("state"),
        dir.path().join("cache"),
    );
    Env {
        yaml: legacy.join("config.yaml"),
        store,
        _dir: dir,
    }
}

fn key(p: &Profile, i: u8) -> &Key {
    p.pages[0]
        .keys
        .get(&i)
        .unwrap_or_else(|| panic!("key {i} missing"))
}

#[test]
fn migrates_fixture() {
    let e = env();
    let report = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let p = e.store.load_profile(report.profile_id).expect("profile saved");
    assert_eq!(p.name, "Default");
    assert_eq!(p.model, Model::D200X);
    assert_eq!(p.pages.len(), 1);
    assert_eq!(p.pages[0].name, "Main");
    assert_eq!(report.keys, 13);
    assert_eq!(p.pages[0].keys.len(), 13);

    assert_eq!(
        key(&p, 0).look.source,
        ImageSource::Icon {
            set: IconSet::Lucide,
            name: "monitor".into()
        }
    );
    assert_eq!(key(&p, 0).look.background, "#2a0405".parse::<Color>().expect("c"));
    assert_eq!(key(&p, 0).look.foreground, "#b03133".parse::<Color>().expect("c"));
    assert_eq!(key(&p, 0).look.icon_scale, 0.70);
    assert_eq!(key(&p, 0).look.label.text, "screen off");
    assert_eq!(
        key(&p, 0).tap,
        Some(Action::Command {
            cmd: "echo screen-off".into(),
            terminal: false
        })
    );
    assert_eq!(
        key(&p, 1).tap,
        Some(Action::Shortcut {
            keys: "super+F12".into()
        })
    );
    assert_eq!(
        key(&p, 3).look.source,
        ImageSource::Image {
            path: "icons/red.png".into()
        }
    );
    assert_eq!(
        key(&p, 6).tap,
        Some(Action::Media {
            control: MediaControl::PlayPause,
            player: None
        })
    );
    assert_eq!(
        key(&p, 7).tap,
        Some(Action::Media {
            control: MediaControl::Next,
            player: Some("spotify".into())
        })
    );
    assert_eq!(
        key(&p, 9).tap,
        Some(Action::LaunchApp {
            desktop_id: None,
            exec: Some("firefox".into())
        })
    );
    assert_eq!(
        key(&p, 11).tap,
        Some(Action::obs("toggle_scenes", &[("a", "Gaming"), ("b", "Desktop")]))
    );
    assert_eq!(
        key(&p, 12).tap,
        Some(Action::PlaySound {
            path: "/x/y.mp3".into(),
            device: None,
            volume: 100
        })
    );

    assert_eq!(
        p.pages[0].dials[1].right,
        Some(Action::Volume {
            target: AudioTarget::App {
                name: "Discord".into()
            },
            op: VolumeOp::Up,
            step: 4,
            limit_100: true
        })
    );
    assert_eq!(
        p.pages[0].dials[0].press,
        Some(Action::Volume {
            target: AudioTarget::Default,
            op: VolumeOp::Mute,
            step: 2,
            limit_100: true
        })
    );
    assert_eq!(
        p.pages[0].dials[2].left,
        Some(Action::Volume {
            target: AudioTarget::Device {
                name: "alsa_output.usb".into()
            },
            op: VolumeOp::Down,
            step: 4,
            limit_100: true
        })
    );
    assert_eq!(p.pages[0].dials[2].press, None);

    assert_eq!(
        p.screen,
        ScreenConfig::custom(
            vec![Widget::now_playing()],
            Some(Action::Media {
                control: MediaControl::PlayPause,
                player: None
            }),
            None
        )
    );
    assert_eq!(p.small_buttons.left, Some(Action::PrevPage));
    assert_eq!(p.small_buttons.right, Some(Action::NextPage));

    let s = e.store.load_settings().expect("settings");
    assert_eq!(s.brightness, 10);
    assert!(s.hide_labels);
    assert_eq!(s.sleep_after_min, 0);
    assert_eq!(
        s.plugins["obs"].settings["port"].as_integer(),
        Some(4444),
        "for the OBS plugin"
    );
    assert_eq!(s.default_profile, Some(p.id));
    assert!(report.warnings.iter().any(|w| w.contains("4455")));
}

#[test]
fn generated_icon_files_become_icons() {
    let e = env();
    let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let p = e.store.load_profile(r.profile_id).expect("profile");
    let k = key(&p, 6);
    assert_eq!(
        k.look.source,
        ImageSource::Icon {
            set: IconSet::Lucide,
            name: "play".into()
        }
    );
    assert_eq!(k.look.background, "#000000".parse::<Color>().expect("c"));
    assert_eq!(k.look.foreground, "#3498db".parse::<Color>().expect("c"));
    assert!(
        !r.warnings.iter().any(|w| w.contains("key 6")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn copies_custom_images() {
    let e = env();
    let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let icons = e.store.profile_dir(r.profile_id).join("icons");
    assert!(icons.join("red.png").is_file());
    assert!(icons.join("speaker.png").is_file());
}

#[test]
fn missing_image_warns() {
    let e = env();
    let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    let p = e.store.load_profile(r.profile_id).expect("profile");
    assert_eq!(key(&p, 8).look.source, ImageSource::None);
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("key 8") && w.contains("missing.png")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn legacy_file_untouched() {
    let e = env();
    let before = std::fs::read(&e.yaml).expect("read");
    let mtime = std::fs::metadata(&e.yaml)
        .expect("meta")
        .modified()
        .expect("mtime");
    migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
    assert_eq!(std::fs::read(&e.yaml).expect("read"), before);
    assert_eq!(
        std::fs::metadata(&e.yaml)
            .expect("meta")
            .modified()
            .expect("mtime"),
        mtime
    );
}

#[test]
fn second_run_refuses_without_force() {
    let e = env();
    migrate_legacy(&e.yaml, &e.store, false).expect("first");
    assert!(matches!(
        migrate_legacy(&e.yaml, &e.store, false),
        Err(CoreError::AlreadyMigrated)
    ));
    migrate_legacy(&e.yaml, &e.store, true).expect("forced");
}

#[test]
fn clock_modes_map() {
    for (mode, expected) in [
        (0, ScreenConfig::FirmwareStats),
        (
            1,
            ScreenConfig::FirmwareClock {
                style: ClockStyle::Classic,
            },
        ),
        (2, ScreenConfig::custom(vec![Widget::Blank], None, None)),
    ] {
        let e = env();
        let text = std::fs::read_to_string(&e.yaml)
            .expect("read")
            .replace("clock_mode: 3", &format!("clock_mode: {mode}"));
        // Button 13 only feeds the Now Playing screen's tap; drop it for the other modes.
        let text = text.replace(
            "- label: Clock Button\n  action: media\n  params:\n    control: play_pause\n",
            "- null\n",
        );
        std::fs::write(&e.yaml, text).expect("write");
        let r = migrate_legacy(&e.yaml, &e.store, false).expect("migrate");
        assert_eq!(
            e.store.load_profile(r.profile_id).expect("p").screen,
            expected,
            "clock_mode {mode}"
        );
    }
}

fn loaded(store: &Store) -> Vec<Uuid> {
    store
        .list_profiles()
        .into_iter()
        .filter_map(|l| match l {
            ProfileLoad::Ok(p) => Some(p.id),
            _ => None,
        })
        .collect()
}

#[test]
fn an_import_takes_the_place_of_the_empty_starting_profile() {
    let e = env();
    let empty = Profile::new_default("Default", Model::D200X);
    e.store.save_profile(&empty).expect("save");
    let r = migrate_legacy(&e.yaml, &e.store, false).expect("import");
    assert_eq!(loaded(&e.store), [r.profile_id]);
}

#[test]
fn an_import_keeps_a_profile_with_keys() {
    let e = env();
    let mut mine = Profile::new_default("Default", Model::D200X);
    mine.pages[0].keys.insert(0, Key::default());
    e.store.save_profile(&mine).expect("save");
    migrate_legacy(&e.yaml, &e.store, false).expect("import");
    assert_eq!(loaded(&e.store).len(), 2);
}

fn press(a: Action) -> Dial {
    Dial {
        press: Some(a),
        ..Default::default()
    }
}

fn pages(n: usize) -> Profile {
    let mut p = Profile::new_default("Test", Model::D200);
    while p.pages.len() < n {
        p.pages.push(Page::new(&format!("P{}", p.pages.len())));
    }
    p.pages.truncate(n);
    p
}

#[test]
fn old_profile_wide_dials_become_pinned() {
    let mut p = pages(2);
    p.dials[0] = press(Action::NextPage);
    p.dials[2] = press(Action::PrevPage);
    p.normalize();
    assert_eq!(p.pinned.dials.len(), 2);
    assert_eq!(p.pinned.dials[&0], press(Action::NextPage));
    assert_eq!(p.pinned.dials[&2], press(Action::PrevPage));
    assert!(p.dials.iter().all(Dial::is_empty));
    assert!(p.pages.iter().all(|pg| pg.dials.iter().all(Dial::is_empty)));
}

#[test]
fn a_dial_the_same_on_every_page_becomes_pinned() {
    let mut p = pages(3);
    for pg in &mut p.pages {
        pg.dials[1] = press(Action::NextPage);
    }
    p.pages[0].dials[0] = press(Action::NextPage);
    p.pages[1].dials[0] = press(Action::PrevPage);
    p.pages[2].dials[0] = press(Action::NextPage);
    assert!(p.pin_shared_dials());
    assert_eq!(p.pinned.dials.len(), 1);
    assert_eq!(p.pinned.dials[&1], press(Action::NextPage));
    assert!(p.pages.iter().all(|pg| !pg.dials[0].is_empty()));
}

#[test]
fn the_one_time_pass_pins_and_keeps_page_copies() {
    let mut p = pages(2);
    for pg in &mut p.pages {
        pg.dials[2] = press(Action::Back);
    }
    assert!(p.pin_shared_dials());
    assert_eq!(p.pinned.dials[&2], press(Action::Back));
    // The copies stay under the pin, so unpinning can never wipe the dial.
    assert!(p.pages.iter().all(|pg| pg.dials[2] == press(Action::Back)));
}

#[test]
fn a_single_page_keeps_its_dials() {
    let mut p = pages(1);
    p.pages[0].dials[1] = press(Action::NextPage);
    assert!(!p.pin_shared_dials());
    p.normalize();
    assert!(p.pinned.is_empty());
    assert_eq!(p.pages[0].dials[1], press(Action::NextPage));
}

#[test]
fn normalize_no_longer_pins_a_dial_on_every_page() {
    let mut p = pages(3);
    for pg in &mut p.pages {
        pg.dials[1] = press(Action::NextPage);
    }
    let before = p.clone();
    p.normalize();
    assert_eq!(p, before);
}

#[test]
fn a_profile_using_the_library_is_left_alone() {
    // A profile with a pin is left alone, even if another dial is the same everywhere.
    let mut q = pages(3);
    q.pinned.dials.insert(0, press(Action::PrevPage));
    for pg in &mut q.pages {
        pg.dials[2] = press(Action::NextPage);
    }
    let before = q.clone();
    assert!(!q.pin_shared_dials());
    assert_eq!(q, before);
}

#[test]
fn a_page_with_a_foreign_dial_blocks_pinning_that_dial() {
    let mut p = pages(3);
    for pg in &mut p.pages {
        pg.dials[2] = press(Action::NextPage);
    }
    p.foreign.push(ForeignSlot {
        page: Some(p.pages[2].id),
        slot: Slot::Dial(2),
        raw: toml::Value::Table(toml::Table::new()),
    });
    // Same where it can be read, but the third page's dial is from a newer version.
    assert!(!p.pin_shared_dials());
    assert!(p.pinned.is_empty());
}

fn shared_dial_store(dir: &Path) -> (Store, Uuid) {
    let store = Store::at(dir.join("config"), dir.join("state"), dir.join("cache"));
    let mut p = pages(2);
    for pg in &mut p.pages {
        pg.dials[0] = press(Action::NextPage);
    }
    store.save_profile(&p).expect("save");
    (store, p.id)
}

#[test]
fn the_one_time_pass_runs_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (store, id) = shared_dial_store(dir.path());
    let changed = pin_shared_dials_once(&store).expect("pass");
    assert_eq!(changed, vec![id]);
    let p = store.load_profile(id).expect("load");
    assert_eq!(p.pinned.dials[&0], press(Action::NextPage));
    assert!(p.pages.iter().all(|pg| pg.dials[0] == press(Action::NextPage)));
    assert!(store.state_dir().join(PINNED_MARKER).exists());
    // A backup was taken first.
    let snaps = ulanzi_core::snapshot::list(&store);
    assert_eq!(snaps.len(), 1);
    assert!(snaps[0].reason.contains("pin"));

    // The user unpins it; a second start leaves it unpinned.
    let mut p = store.load_profile(id).expect("load");
    p.pinned.dials.clear();
    store.save_profile(&p).expect("save");
    assert!(pin_shared_dials_once(&store).expect("pass").is_empty());
    assert!(store.load_profile(id).expect("load").pinned.is_empty());
    assert_eq!(ulanzi_core::snapshot::list(&store).len(), 1);
}

#[test]
fn the_one_time_pass_with_nothing_to_pin_takes_no_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::at(dir.path().join("c"), dir.path().join("s"), dir.path().join("k"));
    store.save_profile(&pages(2)).expect("save");
    assert!(pin_shared_dials_once(&store).expect("pass").is_empty());
    assert!(ulanzi_core::snapshot::list(&store).is_empty());
    assert!(store.state_dir().join(PINNED_MARKER).exists());
}

#[test]
fn restoring_the_backup_from_before_pinning_brings_back_the_unpinned_dials() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (store, id) = shared_dial_store(dir.path());
    pin_shared_dials_once(&store).expect("pass");
    let backup = ulanzi_core::snapshot::list(&store)[0].name.clone();
    ulanzi_core::snapshot::restore(&store, &backup).expect("restore");
    assert!(store.load_profile(id).expect("load").pinned.is_empty());
}
