//! Checking a downloaded release. `SHA256SUMS` is signed with a key that stays with the maintainer,
//! and the file must match its line there. Every failure stops the update.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

use super::UpdateError;

/// Keys that sign app releases (minisign, Ed25519). Separate from the plugin store's key, which is
/// held in CI: plugins are sandboxed and the app is not. A list, so a key can be rotated.
pub const TRUSTED_KEYS: &[&str] = &[
    // packaging/release-key.pub, which scripts/publish.sh checks releases against.
    "RWRggS6K7GTDoxAnEXcIlq/wDo8kR579Lh8q5cv1ztbDwtiKyaCWLlKR",
];

/// The signed comment of a release's checksum file.
const SUMS_COMMENT: &str = "file:SHA256SUMS";

/// Largest signature file read.
const MAX_SIGNATURE_BYTES: usize = 4096;

/// A test key for the fake release in `scripts/fake-release.sh`. Debug builds only.
#[cfg(debug_assertions)]
fn trusted() -> Vec<String> {
    let extra = std::env::var("ULANZI_TEST_RELEASE_KEY").ok();
    TRUSTED_KEYS.iter().map(|k| k.to_string()).chain(extra).collect()
}

#[cfg(not(debug_assertions))]
fn trusted() -> Vec<String> {
    TRUSTED_KEYS.iter().map(|k| k.to_string()).collect()
}

/// Whether `asset` is the file named `asset_name` in the signed `sums`.
pub fn check(sums: &[u8], signature: &str, asset_name: &str, asset: &Path) -> Result<(), UpdateError> {
    let keys = trusted();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    check_with(&keys, sums, signature, asset_name, asset)
}

fn check_with(
    keys: &[&str],
    sums: &[u8],
    signature: &str,
    asset_name: &str,
    asset: &Path,
) -> Result<(), UpdateError> {
    if signature.len() > MAX_SIGNATURE_BYTES {
        return Err(UpdateError::Unverified);
    }
    let sig = Signature::decode(signature).map_err(|_| UpdateError::Unverified)?;
    let signed = keys
        .iter()
        .filter_map(|k| PublicKey::from_base64(k).ok())
        .any(|k| k.verify(sums, &sig, false).is_ok());
    if !signed || sig.trusted_comment() != SUMS_COMMENT {
        return Err(UpdateError::Unverified);
    }
    let sums = std::str::from_utf8(sums).map_err(|_| UpdateError::Unverified)?;
    let want = sums
        .lines()
        .filter_map(|l| l.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim_start().trim_start_matches('*') == asset_name)
        .map(|(hash, _)| hash)
        .ok_or(UpdateError::Unverified)?;
    if hash_of(asset)
        .map_err(|_| UpdateError::Unverified)?
        .eq_ignore_ascii_case(want)
    {
        Ok(())
    } else {
        Err(UpdateError::Unverified)
    }
}

/// The SHA-256 of a file, in hex. Reads in pieces: a release file can be large.
fn hash_of(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buf)? {
            0 => break,
            n => hasher.update(&buf[..n]),
        }
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key (`minisign -G -W`). `SUMS` holds the SHA-256 of "asset bytes".
    const KEY: &str = "RWSs/atOYGdNly+XXewka2b44HRpYWS3v+8YvPtCQRRCzq2cw/8hVmRJ";
    const OTHER_KEY: &str = "RWT2/HtBKUuAWImA/92lW+GmNaz2nfsNJrxbGrfVmSj/sPyQlngs0pCf";
    const NAME: &str = "Ulanzi_Deck-9.9.9-x86_64.AppImage";
    const SUMS: &str = "84293ed06cb3210e7d549afec3140d0c48494416ad25b7f25196afffaa5eb796  Ulanzi_Deck-9.9.9-x86_64.AppImage\n";
    const SIG: &str = "untrusted comment: signature from minisign secret key
RUSs/atOYGdNl4/T6e0ANX1ZNjzAj99eG41ifrnMSPXk7oI56tOYLznY46X4ThsNc3mjcsjPLzsdCr5qmXhS2Cm81zdt+saSlgs=
trusted comment: file:SHA256SUMS
6oGLrfaCxJlDrCBCLbzjjfkgKcH3KzE3640CbHDEyNNWd7XNiweYbG/Neacs+qB085vpcaQil9606E3hJ4X5Ag==
";
    /// The same bytes signed with another file's comment.
    const OTHER_SIG: &str = "untrusted comment: signature from minisign secret key
RUSs/atOYGdNl2Sg0buxSkjdI3DLwzPK8hrKTov3BB9iEbGjTj8hM261Mtw3ZXu0vGDvns7EI8SsDgx4KROzLgiE9r08YT9lMAc=
trusted comment: file:other
XrS8yrkr9afS43d1eof1ELBODWs6jLc1s+xAsm3q1skN98XiL6dafSC8JiD+rR4B+6pkJRXHX6shPaIbYmocBg==
";

    fn asset(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().expect("tmp");
        let p = d.path().join("asset");
        std::fs::write(&p, bytes).expect("write");
        (d, p)
    }

    #[test]
    fn a_signed_matching_file_passes() {
        let (_d, p) = asset(b"asset bytes");
        assert_eq!(check_with(&[KEY], SUMS.as_bytes(), SIG, NAME, &p), Ok(()));
    }

    #[test]
    fn a_changed_file_fails() {
        let (_d, p) = asset(b"asset bytez");
        assert_eq!(
            check_with(&[KEY], SUMS.as_bytes(), SIG, NAME, &p),
            Err(UpdateError::Unverified)
        );
    }

    #[test]
    fn a_changed_sums_file_fails() {
        let (_d, p) = asset(b"asset bytes");
        let sums = SUMS.replace("84293", "84294");
        assert_eq!(
            check_with(&[KEY], sums.as_bytes(), SIG, NAME, &p),
            Err(UpdateError::Unverified)
        );
    }

    #[test]
    fn an_untrusted_key_fails() {
        let (_d, p) = asset(b"asset bytes");
        assert_eq!(
            check_with(&[OTHER_KEY], SUMS.as_bytes(), SIG, NAME, &p),
            Err(UpdateError::Unverified)
        );
        assert_eq!(
            check_with(&[], SUMS.as_bytes(), SIG, NAME, &p),
            Err(UpdateError::Unverified)
        );
    }

    #[test]
    fn a_file_not_listed_in_the_sums_fails() {
        let (_d, p) = asset(b"asset bytes");
        assert_eq!(
            check_with(
                &[KEY],
                SUMS.as_bytes(),
                SIG,
                "ulanzi-deck-9.9.9-x86_64.tar.gz",
                &p
            ),
            Err(UpdateError::Unverified)
        );
    }

    #[test]
    fn a_signature_for_another_file_fails() {
        let (_d, p) = asset(b"asset bytes");
        assert_eq!(
            check_with(&[KEY], SUMS.as_bytes(), OTHER_SIG, NAME, &p),
            Err(UpdateError::Unverified)
        );
    }

    #[test]
    fn the_built_in_key_is_the_one_releases_are_checked_against() {
        let file = include_str!("../../../../packaging/release-key.pub");
        assert_eq!(file.lines().last(), TRUSTED_KEYS.last().copied());
    }

    #[test]
    fn a_missing_file_fails() {
        let d = tempfile::tempdir().expect("tmp");
        assert_eq!(
            check_with(&[KEY], SUMS.as_bytes(), SIG, NAME, &d.path().join("gone")),
            Err(UpdateError::Unverified)
        );
    }
}
