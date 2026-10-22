use std::io::Write;

use ulanzi_core::bundle::{BundleError, Conflict, ImportOptions, export, import, read};
use ulanzi_core::model::*;
use ulanzi_core::store::Store;

fn store(dir: &std::path::Path) -> Store {
    Store::at(dir.join("c"), dir.join("s"), dir.join("k"))
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("png");
    out.into_inner()
}

/// A profile with one key using a picture from its icons folder.
fn profile_with_icon(s: &Store, name: &str) -> Profile {
    let mut p = Profile::new_default(name, Model::D200X);
    p.pages[0].keys.insert(
        0,
        Key {
            look: KeyLook {
                source: ImageSource::Image {
                    path: "icons/x.png".into(),
                },
                ..KeyLook::default()
            },
            tap: Some(Action::NextPage),
            hold: None,
        },
    );
    p.r#match.apps = vec!["firefox.desktop".into()];
    s.save_profile(&p).expect("save");
    std::fs::create_dir_all(s.profile_dir(p.id).join("icons")).expect("dir");
    std::fs::write(s.profile_dir(p.id).join("icons/x.png"), png()).expect("icon");
    p
}

fn opts(conflict: Conflict) -> ImportOptions {
    ImportOptions {
        conflict,
        keep_rules: true,
        settings: false,
        device: Some(Model::D200X),
    }
}

#[test]
fn round_trips_profiles_with_icons() {
    let d = tempfile::tempdir().expect("tmp");
    let a = store(&d.path().join("a"));
    let p = profile_with_icon(&a, "Editing");
    let bytes = export(&a, &[p.id], false).expect("export");
    let b = read(&bytes).expect("read");
    assert_eq!(b.manifest.profiles.len(), 1);
    assert_eq!(b.manifest.profiles[0].name, "Editing");
    assert_eq!(b.manifest.profiles[0].pages, 1);
    assert_eq!(b.manifest.profiles[0].keys, 1);
    assert!(!b.manifest.includes_settings);

    let fresh = store(&d.path().join("b"));
    let r = import(&fresh, &b, &opts(Conflict::KeepBoth)).expect("import");
    assert_eq!(r.imported, vec![p.id]);
    assert_eq!(fresh.load_profile(p.id).expect("load"), p);
    assert_eq!(
        std::fs::read(fresh.profile_dir(p.id).join("icons/x.png")).expect("icon"),
        png()
    );
}

#[test]
fn export_all_with_settings_drops_obs_password() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    profile_with_icon(&s, "A");
    let mut settings = Settings::default();
    settings.obs.password = Some("s3cret".into());
    s.save_settings(&settings).expect("settings");
    let b = read(&export(&s, &[], true).expect("export")).expect("read");
    assert!(b.manifest.includes_settings);
    assert_eq!(b.settings.expect("settings").obs.password, None);
}

/// A ZIP with the given (name, declared bytes) entries.
fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        w.start_file(*name, o).expect("start");
        w.write_all(data).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

fn manifest_for(id: uuid::Uuid) -> Vec<u8> {
    format!(
        "format = 1\napp_version = \"2.0.0\"\ncreated = \"2026-10-23T00:00:00Z\"\nincludes_settings = false\n\n[[profiles]]\nid = \"{id}\"\nname = \"Evil\"\nmodel = \"D200X\"\npages = 1\nkeys = 0\n"
    )
    .into_bytes()
}

#[test]
fn rejects_unsafe_bundles() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = Profile::new_default("Evil", Model::D200X);
    let toml = toml::to_string(&p).expect("toml").into_bytes();
    let dir = format!("profiles/{}", p.id);
    let base = |extra: (&str, Vec<u8>)| {
        zip_of(&[
            ("manifest.toml", manifest_for(p.id)),
            (&format!("{dir}/profile.toml"), toml.clone()),
            extra,
        ])
    };
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (base(("../evil", b"x".to_vec())), "unsafe"),
        (base(("/etc/evil", b"x".to_vec())), "unsafe"),
        (base((&format!("{dir}/icons/../../x"), b"x".to_vec())), "unsafe"),
        (
            base((&format!("{dir}/icons/big.png"), vec![0u8; 60 * 1024 * 1024])),
            "large",
        ),
        (
            base((&format!("{dir}/icons/a.png"), b"#!/bin/sh\nrm -rf ~".to_vec())),
            "image",
        ),
        (b"not a zip".to_vec(), "bundle"),
    ];
    for (bytes, want) in cases {
        let err = read(&bytes).expect_err(want);
        let ok = match want {
            "unsafe" => matches!(err, BundleError::UnsafePath(_)),
            "large" => matches!(err, BundleError::TooLarge),
            "image" => matches!(err, BundleError::BadImage(_)),
            _ => matches!(err, BundleError::NotABundle),
        };
        assert!(ok, "{want}: got {err:?}");
    }
    assert!(s.list_profiles().is_empty(), "nothing written");
}

#[test]
fn conflicts_keep_both_replace_skip() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = profile_with_icon(&s, "A");
    let b = read(&export(&s, &[p.id], false).expect("export")).expect("read");

    let r = import(&s, &b, &opts(Conflict::KeepBoth)).expect("keep both");
    assert_eq!(r.imported.len(), 1);
    assert_ne!(r.imported[0], p.id);
    assert_eq!(s.load_profile(r.imported[0]).expect("copy").name, "A (2)");
    assert!(s.profile_dir(r.imported[0]).join("icons/x.png").exists());

    let r = import(&s, &b, &opts(Conflict::Skip)).expect("skip");
    assert!(r.imported.is_empty());
    assert_eq!(r.skipped, vec!["A".to_string()]);

    let mut edited = p.clone();
    edited.name = "Changed".into();
    s.save_profile(&edited).expect("edit");
    let r = import(&s, &b, &opts(Conflict::Replace)).expect("replace");
    assert_eq!(r.replaced, vec![p.id]);
    assert_eq!(s.load_profile(p.id).expect("load").name, "A");
}

#[test]
fn model_mismatch_warns() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = Profile::new_default("Small", Model::D200);
    s.save_profile(&p).expect("save");
    let b = read(&export(&s, &[p.id], false).expect("export")).expect("read");
    let other = store(&d.path().join("o"));
    let r = import(&other, &b, &opts(Conflict::KeepBoth)).expect("import");
    assert!(r.warnings.iter().any(|w| w.contains("D200")), "{:?}", r.warnings);
}

#[test]
fn keep_rules_false_clears_match() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = profile_with_icon(&s, "A");
    let b = read(&export(&s, &[p.id], false).expect("export")).expect("read");
    let other = store(&d.path().join("o"));
    let o = ImportOptions {
        keep_rules: false,
        ..opts(Conflict::KeepBoth)
    };
    import(&other, &b, &o).expect("import");
    assert!(other.load_profile(p.id).expect("load").r#match.apps.is_empty());
}
