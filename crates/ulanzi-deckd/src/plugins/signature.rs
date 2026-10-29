//! Signed plugins: a `.ulanziplugin` next to a `.minisig` from Ulanzi-Deck-Plugins installs without
//! Developer mode. The signature must check out against a key built into the app, and its signed
//! comment must name this plugin's id and version, so a signature cannot be moved to another package
//! or version.

use minisign_verify::{PublicKey, Signature};

use super::PluginError;

/// Keys the app trusts (minisign, Ed25519). A list, so a key can be rotated: ship the new key in an
/// update, sign with it, then drop the old one.
pub const TRUSTED_KEYS: &[&str] = &[
    // Ulanzi-Deck-Plugins, key id 58804B29417BFCF6.
    "RWT2/HtBKUuAWImA/92lW+GmNaz2nfsNJrxbGrfVmSj/sPyQlngs0pCf",
];

/// Largest signature file read.
const MAX_SIGNATURE_BYTES: usize = 4096;

/// Written into an installed plugin's folder when its package was signed.
pub const VERIFIED_MARKER: &str = ".verified";

/// The signed comment the plugins repo writes for a package or the index.
pub fn comment_for(file: &str) -> String {
    format!("file:{file}")
}

/// Whether `signature` (the text of a `.minisig`) signs `data` with a trusted key and names `file`
/// in its signed comment. Every failure is an error worded for the user.
pub fn verify(data: &[u8], signature: &str, file: &str) -> Result<(), PluginError> {
    verify_with(TRUSTED_KEYS, data, signature, file)
}

fn verify_with(keys: &[&str], data: &[u8], signature: &str, file: &str) -> Result<(), PluginError> {
    let bad = |why: &str| PluginError::Invalid(format!("The signature does not match: {why}"));
    if signature.len() > MAX_SIGNATURE_BYTES {
        return Err(bad("the signature file is too large"));
    }
    let sig = Signature::decode(signature).map_err(|_| bad("the signature file is not valid"))?;
    let trusted = keys
        .iter()
        .filter_map(|k| PublicKey::from_base64(k).ok())
        .any(|k| k.verify(data, &sig, false).is_ok());
    if !trusted {
        return Err(bad(
            "it was not made by Ulanzi Deck's plugin key, or the file was changed",
        ));
    }
    if sig.trusted_comment() != comment_for(file) {
        return Err(bad("it was made for another file"));
    }
    Ok(())
}

/// The package file name a plugin version is signed under.
pub fn package_name(id: &str, version: &str) -> String {
    format!("{id}-{version}.ulanziplugin")
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key (`minisign -G -W`) and its signature of "plugin bytes" with the signed comment
    // `file:hello-1.0.0.ulanziplugin`.
    const TEST_KEY: &str = "RWTX7ld6rvu/ztcZIhB7vCxu9cKQKd3AgffEr5Sg4BAyARZJyp7l+LRH";
    const TEST_SIG: &str = "untrusted comment: signature from minisign secret key
RUTX7ld6rvu/zpWUM2kxvliq1zY/IGml4hZd+4BjXq6qdEvrJXb87wUudG4ffSba0n5b3AeWX+7j5sgg2rupTVvjLeZAyQoqoAc=
trusted comment: file:hello-1.0.0.ulanziplugin
3RNNB2URCzK+muYH6nBDW/5QiElEhUR9X98Mf0yLu20qHi0SyCyz21DdiSMMXgP92r7uu82ErEd4AgUu1WJ0DA==
";
    const FILE: &str = "hello-1.0.0.ulanziplugin";

    #[test]
    fn a_trusted_key_and_the_named_file_pass() {
        assert!(verify_with(&[TEST_KEY], b"plugin bytes", TEST_SIG, FILE).is_ok());
        // Changed data, another name, a key the app does not trust, garbage.
        assert!(verify_with(&[TEST_KEY], b"plugin bytez", TEST_SIG, FILE).is_err());
        let moved = verify_with(&[TEST_KEY], b"plugin bytes", TEST_SIG, "hello-1.0.1.ulanziplugin");
        assert!(moved.expect_err("moved").to_string().contains("another file"));
        let e = verify(b"plugin bytes", TEST_SIG, FILE).expect_err("untrusted key");
        assert!(e.to_string().contains("plugin key"), "{e}");
        assert!(verify(b"x", "not a signature", FILE).is_err());
        assert!(verify(b"x", &"a".repeat(MAX_SIGNATURE_BYTES + 1), FILE).is_err());
    }

    #[test]
    fn the_real_index_checks_out_with_the_built_in_key() {
        let data = include_bytes!("../../tests/fixtures/store/index.json");
        let sig = include_str!("../../tests/fixtures/store/index.json.minisig");
        assert!(verify(data, sig, "index.json").is_ok());
        assert_eq!(package_name("ha", "1.2.0"), "ha-1.2.0.ulanziplugin");
    }
}
