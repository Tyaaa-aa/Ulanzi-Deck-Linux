//! The update check and download against a small local web server.

mod common;

use std::sync::{Arc, Mutex};

use ulanzi_core::store::Store;
use ulanzi_core::update::{Asset, InstallKind, Release, UpdateState};
use ulanzi_deckd::updates::check::{self, Source};
use ulanzi_deckd::updates::{UpdateError, download};

// A throwaway key; `SUMS` holds the SHA-256 of "asset bytes".
const KEY: &str = "RWSs/atOYGdNly+XXewka2b44HRpYWS3v+8YvPtCQRRCzq2cw/8hVmRJ";
const SUMS: &str =
    "84293ed06cb3210e7d549afec3140d0c48494416ad25b7f25196afffaa5eb796  Ulanzi_Deck-9.9.9-x86_64.AppImage\n";
const SIG: &str = "untrusted comment: signature from minisign secret key
RUSs/atOYGdNl4/T6e0ANX1ZNjzAj99eG41ifrnMSPXk7oI56tOYLznY46X4ThsNc3mjcsjPLzsdCr5qmXhS2Cm81zdt+saSlgs=
trusted comment: file:SHA256SUMS
6oGLrfaCxJlDrCBCLbzjjfkgKcH3KzE3640CbHDEyNNWd7XNiweYbG/Neacs+qB085vpcaQil9606E3hJ4X5Ag==
";
const ASSET: &str = "Ulanzi_Deck-9.9.9-x86_64.AppImage";

use common::http::{Reply, reply, serve, serve_late};

fn store(d: &tempfile::TempDir) -> Store {
    Store::at(d.path().join("c"), d.path().join("s"), d.path().join("k"))
}

fn release_json(base: &str, tag: &str, assets: &[&str]) -> Vec<u8> {
    let assets: Vec<String> = assets
        .iter()
        .map(|n| format!(r#"{{"name":"{n}","browser_download_url":"{base}/{n}","size":11}}"#))
        .collect();
    format!(
        r#"{{"tag_name":"{tag}","name":"Ulanzi Deck","draft":false,"prerelease":false,
        "published_at":"2026-11-02T10:00:00Z","body":"Notes","assets":[{}]}}"#,
        assets.join(",")
    )
    .into_bytes()
}

fn source(base: &str) -> Source {
    Source {
        url: format!("{base}/latest"),
        current: "2.1.1".into(),
        kind: InstallKind::AppImage,
    }
}

const ALL: [&str; 3] = [ASSET, "SHA256SUMS", "SHA256SUMS.minisig"];

/// A server whose `/latest` is `tag` with `assets`.
fn release_server(tag: &str, assets: &[&str], status: u16) -> (String, Arc<Mutex<Vec<String>>>) {
    let (tag, assets) = (
        tag.to_string(),
        assets.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
    );
    serve_late(move |base| {
        let names: Vec<&str> = assets.iter().map(String::as_str).collect();
        vec![reply("/latest", status, &release_json(base, &tag, &names))]
    })
}

#[test]
fn a_new_release_is_offered() {
    let d = tempfile::tempdir().expect("tmp");
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    let s = check::run(&store(&d), &source(&base), false).expect("check");
    assert_eq!(s.offered.as_ref().map(|r| r.tag.as_str()), Some("v9.9.9"));
    assert!(s.can_install);
    assert!(s.last_check.is_some());
}

#[test]
fn the_same_version_is_not_offered() {
    let d = tempfile::tempdir().expect("tmp");
    let (base, _) = release_server("v2.1.1", &ALL, 200);
    let s = check::run(&store(&d), &source(&base), false).expect("check");
    assert_eq!(s.offered, None);
}

#[test]
fn a_304_keeps_the_last_offer() {
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    check::run(&st, &source(&base), false).expect("first");
    let (quiet, seen) = serve(vec![reply("/latest", 304, b"")]);
    let s = check::run(&st, &source(&quiet), false).expect("second");
    assert_eq!(s.offered.as_ref().map(|r| r.tag.as_str()), Some("v9.9.9"));
    assert!(
        seen.lock().expect("seen")[0]
            .to_lowercase()
            .contains("if-none-match: \"tag1\"")
    );
}

#[test]
fn a_403_is_silent_and_keeps_the_last_offer() {
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    check::run(&st, &source(&base), false).expect("first");
    let (limited, _) = serve(vec![reply("/latest", 403, b"rate limit")]);
    let s = check::run(&st, &source(&limited), false).expect("automatic check stays quiet");
    assert_eq!(s.offered.as_ref().map(|r| r.tag.as_str()), Some("v9.9.9"));
    assert_eq!(
        check::run(&st, &source(&limited), true),
        Err(UpdateError::Offline),
        "Check Now reports it"
    );
}

#[test]
fn no_network_is_offline() {
    let d = tempfile::tempdir().expect("tmp");
    let src = source("http://127.0.0.1:9");
    assert_eq!(check::run(&store(&d), &src, true), Err(UpdateError::Offline));
}

#[test]
fn a_skipped_version_is_not_offered_until_a_newer_one() {
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    check::run(&st, &source(&base), false).expect("first");
    check::skip(&st, "9.9.9").expect("skip");
    let s = check::run(&st, &source(&base), false).expect("again");
    assert_eq!(s.offered, None);
    let (newer, _) = release_server("v9.9.10", &ALL, 200);
    let s = check::run(&st, &source(&newer), false).expect("newer");
    assert!(s.offered.is_some());
}

#[test]
fn a_release_with_no_signature_cannot_be_installed() {
    let d = tempfile::tempdir().expect("tmp");
    let (base, _) = release_server("v9.9.9", &[ASSET, "SHA256SUMS"], 200);
    let s = check::run(&store(&d), &source(&base), false).expect("check");
    assert!(s.offered.is_some());
    assert!(!s.can_install);
}

#[test]
fn another_install_kind_cannot_be_installed() {
    let d = tempfile::tempdir().expect("tmp");
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    let mut src = source(&base);
    src.kind = InstallKind::Other;
    assert!(!check::run(&store(&d), &src, false).expect("check").can_install);
}

#[allow(unsafe_code)]
fn trust_test_key() {
    // Debug builds only, and these tests are the only ones that read it.
    unsafe { std::env::set_var("ULANZI_TEST_RELEASE_KEY", KEY) };
}

fn release(base: &str) -> Release {
    let asset = |n: &str| Asset {
        name: n.into(),
        url: format!("{base}/{n}"),
        size: 11,
    };
    Release {
        tag: "v9.9.9".into(),
        title: String::new(),
        published: String::new(),
        notes: String::new(),
        assets: ALL.iter().map(|n| asset(n)).collect(),
    }
}

fn files(asset: Reply) -> Vec<Reply> {
    vec![
        asset,
        reply("/SHA256SUMS", 200, SUMS.as_bytes()),
        reply("/SHA256SUMS.minisig", 200, SIG.as_bytes()),
    ]
}

const ASSET_PATH: &str = "/Ulanzi_Deck-9.9.9-x86_64.AppImage";

#[test]
fn a_good_download_is_kept_and_reported() {
    trust_test_key();
    let d = tempfile::tempdir().expect("tmp");
    let (base, _) = serve(files(reply(ASSET_PATH, 200, b"asset bytes")));
    let stages: Mutex<Vec<String>> = Mutex::default();
    let path = download::fetch(&store(&d), &release(&base), InstallKind::AppImage, |s, _, _| {
        stages.lock().expect("stages").push(s.to_string());
    })
    .expect("fetch");
    assert_eq!(std::fs::read(&path).expect("read"), b"asset bytes");
    let stages = stages.lock().expect("stages");
    assert!(stages.iter().any(|s| s == "downloading"));
    assert!(stages.iter().any(|s| s == "verifying"));
    assert_eq!(stages.last().map(String::as_str), Some("ready"));
}

#[test]
fn an_interrupted_download_leaves_nothing_behind() {
    trust_test_key();
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let mut cut = reply(ASSET_PATH, 200, b"asset");
    cut.claim = Some(11);
    let (base, _) = serve(files(cut));
    let r = download::fetch(&st, &release(&base), InstallKind::AppImage, |_, _, _| {});
    assert!(r.is_err());
    assert!(!st.cache_dir().join("updates").join("9.9.9").exists());
}

#[test]
fn a_tampered_download_is_deleted() {
    trust_test_key();
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let (base, _) = serve(files(reply(ASSET_PATH, 200, b"asset bytez")));
    let r = download::fetch(&st, &release(&base), InstallKind::AppImage, |_, _, _| {});
    assert_eq!(r, Err(UpdateError::Unverified));
    assert!(!st.cache_dir().join("updates").join("9.9.9").exists());
}

#[test]
fn the_appimage_swap_replaces_the_real_file_behind_a_symlink() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().expect("tmp");
    let real = d.path().join("real.AppImage");
    let link = d.path().join("Ulanzi.AppImage");
    let new = d.path().join("new");
    std::fs::write(&real, b"old").expect("write");
    std::fs::write(&new, b"new").expect("write");
    std::os::unix::fs::symlink(&real, &link).expect("link");
    download::swap_appimage(&new, &link).expect("swap");
    assert!(link.is_symlink());
    assert_eq!(std::fs::read(&real).expect("read"), b"new");
    let mode = std::fs::metadata(&real).expect("meta").permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn an_unwritable_folder_reports_it() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().expect("tmp");
    let dir = d.path().join("locked");
    std::fs::create_dir(&dir).expect("mkdir");
    let target = dir.join("Ulanzi.AppImage");
    let new = d.path().join("new");
    std::fs::write(&target, b"old").expect("write");
    std::fs::write(&new, b"new").expect("write");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let r = download::swap_appimage(&new, &target);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert_eq!(r, Err(UpdateError::NotWritable));
    assert_eq!(std::fs::read(&target).expect("read"), b"old");
}

#[test]
fn state_is_kept_in_its_own_file() {
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    check::run(&st, &source(&base), false).expect("check");
    assert_eq!(UpdateState::load(&st).etag.as_deref(), Some("\"tag1\""));
    assert!(st.state_dir().join("updates.toml").exists());
}

#[test]
fn the_installed_release_stays_known_for_whats_new() {
    let d = tempfile::tempdir().expect("tmp");
    let st = store(&d);
    let (base, _) = release_server("v9.9.9", &ALL, 200);
    check::run(&st, &source(&base), false).expect("offered");
    let mut updated = source(&base);
    updated.current = "9.9.9".into();
    let s = check::run(&st, &updated, false).expect("after the update");
    assert_eq!(s.offered, None);
    assert_eq!(s.current_release.map(|r| r.tag), Some("v9.9.9".to_string()));
}
