use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use ulanzi_proto::*;

fn sample_entries() -> BTreeMap<u8, ButtonEntry> {
    let mut e = BTreeMap::new();
    e.insert(
        0,
        ButtonEntry {
            state: 0,
            text: Some("Hi".into()),
            icon: Some("icons/red.png".into()),
        },
    );
    e.insert(
        5,
        ButtonEntry {
            state: 0,
            text: Some("Ünïcode ✓".into()),
            icon: None,
        },
    );
    e.insert(
        13,
        ButtonEntry {
            state: 0,
            text: None,
            icon: None,
        },
    );
    e
}

#[test]
fn manifest_matches_python() {
    assert_eq!(
        manifest_json(&sample_entries()),
        include_str!("fixtures/manifest_sample.json")
    );
}

#[test]
fn manifest_escapes_non_ascii_like_python() {
    let mut e = BTreeMap::new();
    e.insert(
        1,
        ButtonEntry {
            state: 0,
            text: Some("é😀".into()),
            icon: None,
        },
    );
    assert!(manifest_json(&e).contains(r#""Text":"\u00e9\ud83d\ude00""#));
}

#[test]
fn manifest_key_layout() {
    assert_eq!(manifest_key(0), "0_0");
    assert_eq!(manifest_key(7), "2_1");
    assert_eq!(manifest_key(13), "3_2");
}

#[test]
fn zip_contains_manifest_and_icons() {
    let imgs = [ButtonImage {
        name: "a.png".into(),
        png: vec![1, 2, 3],
    }];
    let z = button_zip(&sample_entries(), &imgs);
    let mut ar = zip::ZipArchive::new(Cursor::new(z)).expect("zip");
    let names: Vec<String> = (0..ar.len())
        .map(|i| ar.by_index(i).expect("entry").name().expect("name").to_string())
        .collect();
    assert_eq!(names, ["icons/a.png", "manifest.json"]);
    let mut m = String::new();
    ar.by_name("manifest.json")
        .expect("manifest")
        .read_to_string(&mut m)
        .expect("read");
    assert_eq!(m, manifest_json(&sample_entries()));
}

#[test]
fn file_packets_layout() {
    let d = vec![1u8; 3000];
    let p = file_packets(Command::SetButtons, &d);
    assert_eq!(p.len(), 3);
    assert_eq!(&p[0][..4], &[0x7c, 0x7c, 0x00, 0x01]);
    assert_eq!(&p[0][4..8], &3000u32.to_le_bytes());
    assert_eq!(p[1][0], 1);
    assert_eq!(p[2][3000 - 1016 - 1024 - 1], 1);
    assert_eq!(p[2][3000 - 1016 - 1024], 0);
}

#[test]
fn alert_zip_has_only_the_manifest() {
    let z = alert_zip(7);
    let mut ar = zip::ZipArchive::new(Cursor::new(z)).expect("zip");
    assert_eq!(ar.len(), 1);
    let mut m = String::new();
    ar.by_name("manifest.json")
        .expect("manifest")
        .read_to_string(&mut m)
        .expect("read");
    assert_eq!(m, r#"{"2_1":{"KeyType":1,"Progress":100,"State":1}}"#);
}
