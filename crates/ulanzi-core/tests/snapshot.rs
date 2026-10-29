use ulanzi_core::model::*;
use ulanzi_core::snapshot::{KEEP, list, restore, take};
use ulanzi_core::store::Store;

fn store(dir: &std::path::Path) -> Store {
    Store::at(dir.join("c"), dir.join("s"), dir.join("k"))
}

#[test]
fn take_and_list_newest_first() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = Profile::new_default("A", Model::D200X);
    s.save_profile(&p).expect("save");
    let first = take(&s, "daily").expect("take");
    let second = take(&s, "before-import").expect("take");
    let l = list(&s);
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].name, second.name);
    assert_eq!(l[1].name, first.name);
    assert_eq!(l[0].reason, "before-import");
    assert_eq!(l[0].profiles, vec!["A".to_string()]);
}

#[test]
fn prunes_to_twenty_with_unique_names() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    s.save_profile(&Profile::new_default("A", Model::D200X))
        .expect("save");
    let names: Vec<String> = (0..KEEP + 1)
        .map(|_| take(&s, "test").expect("take").name)
        .collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "names are distinct");
    let l = list(&s);
    assert_eq!(l.len(), KEEP);
    assert!(!l.iter().any(|x| x.name == names[0]), "the oldest went");
}

#[test]
fn restore_snapshots_current_state_first() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let mut p = Profile::new_default("A", Model::D200X);
    s.save_profile(&p).expect("save");
    let snap = take(&s, "daily").expect("take");
    p.name = "Edited".into();
    s.save_profile(&p).expect("edit");
    let extra = Profile::new_default("Added later", Model::D200X);
    s.save_profile(&extra).expect("extra");

    let before = restore(&s, &snap.name).expect("restore");
    assert_eq!(s.load_profile(p.id).expect("load").name, "A");
    assert!(
        s.load_profile(extra.id).is_err(),
        "profiles not in the snapshot are moved aside"
    );
    assert_eq!(before.reason, "before-restore");
    let mut names = before.profiles.clone();
    names.sort();
    assert_eq!(names, vec!["Added later".to_string(), "Edited".to_string()]);
}

#[test]
fn restore_puts_the_settings_back() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    s.save_profile(&Profile::new_default("A", Model::D200X))
        .expect("save");
    let snap = take(&s, "daily").expect("take");
    let settings = Settings {
        brightness: 33,
        ..Settings::default()
    };
    s.save_settings(&settings).expect("settings");
    restore(&s, &snap.name).expect("restore");
    let now = s.load_settings().expect("load");
    assert_eq!(now.brightness, Settings::default().brightness);
}

#[test]
fn reason_is_sanitised() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let snap = take(&s, "../x y").expect("take");
    assert_eq!(snap.reason, "x-y");
    assert!(!snap.name.contains('/'));
}
