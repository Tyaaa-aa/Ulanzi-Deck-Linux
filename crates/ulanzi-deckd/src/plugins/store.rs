//! The plugin store: the signed index from Ulanzi-Deck-Plugins and installs from it. The index is
//! trusted only after its signature checks out against a built-in key; it lists each package's
//! SHA-256, and each package must also carry its own valid signature. The last good index is kept,
//! so the store still lists plugins offline.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::PluginError;
use super::package::MAX_PACKAGE_BYTES;
use super::signature;

/// Where the index is published (the plugins repo's `index` release).
pub const INDEX_URL: &str =
    "https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins/releases/download/index/index.json";
const INDEX_NAME: &str = "index.json";
/// The kept copy, in the plugins folder (hidden, so the plugin scan ignores it).
const CACHE: &str = ".store-index.json";
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4096;
/// Index formats this app reads.
const FORMAT: u32 = 1;

/// The store's list of plugins, as the plugins repo's CI writes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreIndex {
    pub format: u32,
    pub plugins: Vec<StoreEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub categories: Vec<String>,
    pub contract: String,
    #[serde(default)]
    pub min_app: Option<String>,
    #[serde(default)]
    pub tested_on_hardware: bool,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub readme: Option<String>,
    pub package: StorePackage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorePackage {
    pub url: String,
    pub signature: String,
    pub sha256: String,
    pub size: u64,
}

/// What was read: the index, and whether it came from the kept copy (offline).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub index: StoreIndex,
    pub offline: bool,
}

fn net_err(what: &str) -> impl Fn(String) -> PluginError + '_ {
    move |e| PluginError::Io(format!("Can't reach the plugin store ({what}): {e}"))
}

/// `url` (https only), at most `max` bytes.
fn get(url: &str, max: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!("{url} is not an https address"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    agent
        .get(url)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(c) => format!("the server said {c}"),
            _ => "no connection".to_string(),
        })?
        .body_mut()
        .with_config()
        .limit(max)
        .read_to_vec()
        .map_err(|_| "the download was cut short or too large".to_string())
}

/// Parse and check an index and its signature.
pub fn read_index(data: &[u8], sig: &str) -> Result<StoreIndex, PluginError> {
    signature::verify(data, sig, INDEX_NAME)?;
    let index: StoreIndex = serde_json::from_slice(data)
        .map_err(|e| PluginError::Invalid(format!("The plugin store's list is not valid: {e}")))?;
    if index.format != FORMAT {
        return Err(PluginError::Invalid(
            "The plugin store needs a newer Ulanzi Deck".into(),
        ));
    }
    Ok(index)
}

/// The store's index: fresh when the store answers (and kept in `plugins`), else the kept copy.
pub fn fetch_index(plugins: &Path) -> Result<Listing, PluginError> {
    let fresh = (|| {
        let data = get(INDEX_URL, MAX_INDEX_BYTES).map_err(net_err("list"))?;
        let sig = get(&format!("{INDEX_URL}.minisig"), MAX_SIGNATURE_BYTES).map_err(net_err("signature"))?;
        let sig =
            String::from_utf8(sig).map_err(|_| PluginError::Invalid("The signature is not text".into()))?;
        let index = read_index(&data, &sig)?;
        Ok::<_, PluginError>((index, data, sig))
    })();
    let cache = plugins.join(CACHE);
    match fresh {
        Ok((index, data, sig)) => {
            let kept = serde_json::json!({ "data": String::from_utf8_lossy(&data), "sig": sig });
            if fs::create_dir_all(plugins).is_ok() {
                let _ = fs::write(&cache, kept.to_string());
            }
            Ok(Listing {
                index,
                offline: false,
            })
        }
        Err(e) => {
            // Checked again: the kept copy is a file anyone with access could edit.
            let kept = fs::read(&cache)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| {
                    let data = v.get("data")?.as_str()?.as_bytes().to_vec();
                    let sig = v.get("sig")?.as_str()?.to_string();
                    read_index(&data, &sig).ok()
                });
            match kept {
                Some(index) => Ok(Listing { index, offline: true }),
                None => Err(e),
            }
        }
    }
}

/// Download `entry`'s package and signature and check the package against the index's hash.
pub fn download(entry: &StoreEntry) -> Result<(Vec<u8>, String), PluginError> {
    let p = &entry.package;
    let bytes = get(&p.url, MAX_PACKAGE_BYTES).map_err(net_err("download"))?;
    if !hex_sha256(&bytes).eq_ignore_ascii_case(&p.sha256) {
        return Err(PluginError::Invalid(format!(
            "The download of {} does not match the plugin store's list; try again",
            entry.name
        )));
    }
    let sig = get(&p.signature, MAX_SIGNATURE_BYTES).map_err(net_err("signature"))?;
    let sig = String::from_utf8(sig).map_err(|_| PluginError::Invalid("The signature is not text".into()))?;
    Ok((bytes, sig))
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &[u8] = include_bytes!("../../tests/fixtures/store/index.json");
    const SIG: &str = include_str!("../../tests/fixtures/store/index.json.minisig");

    #[test]
    fn the_signed_index_reads_and_a_changed_one_does_not() {
        let index = read_index(INDEX, SIG).expect("index");
        assert_eq!(index.format, FORMAT);
        let ha = index.plugins.iter().find(|p| p.id == "ha").expect("ha");
        assert!(
            ha.package
                .url
                .starts_with("https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins/")
        );
        assert_eq!(ha.package.sha256.len(), 64);
        let mut changed = INDEX.to_vec();
        changed[10] ^= 1;
        assert!(read_index(&changed, SIG).is_err());
    }

    #[test]
    fn the_kept_copy_is_used_offline_only_while_its_signature_holds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let kept = serde_json::json!({ "data": String::from_utf8_lossy(INDEX), "sig": SIG });
        fs::write(dir.path().join(CACHE), kept.to_string()).expect("write");
        // No network in tests is not guaranteed; read_index on the kept copy is what offline uses.
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join(CACHE)).expect("read")).expect("json");
        let data = v["data"].as_str().expect("data").as_bytes();
        assert!(read_index(data, v["sig"].as_str().expect("sig")).is_ok());
        assert!(
            read_index(b"{\"format\":1,\"plugins\":[]}", SIG).is_err(),
            "a list the key did not sign"
        );
        assert_eq!(hex_sha256(b"abc").len(), 64);
    }

    #[test]
    fn only_https_is_fetched() {
        assert!(get("http://example.com/x", 10).is_err());
        assert!(get("file:///etc/passwd", 10).is_err());
    }
}
