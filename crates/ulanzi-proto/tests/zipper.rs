use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use proptest::prelude::*;
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
    assert_eq!(names, ["dummy.txt", "icons/a.png", "manifest.json"]);
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
fn clean_check_finds_bad_byte() {
    let mut z = vec![0x55u8; 3000];
    assert!(zip_is_clean(&z));
    z[1016 + 1024] = 0x7c;
    assert!(!zip_is_clean(&z));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn zip_is_always_clean(seed in any::<u64>(), n in 1usize..14, len in 200usize..6000) {
        let mut x = seed | 1;
        let imgs: Vec<ButtonImage> = (0..n).map(|i| {
            let png = (0..len).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x as u8 }).collect();
            ButtonImage { name: format!("k{i}.png"), png }
        }).collect();
        let mut e = BTreeMap::new();
        for i in 0..n as u8 { e.insert(i, ButtonEntry { state: 0, text: None, icon: Some(format!("icons/k{i}.png")) }); }
        let z = button_zip(&e, &imgs);
        prop_assert!(zip_is_clean(&z));
    }
}
