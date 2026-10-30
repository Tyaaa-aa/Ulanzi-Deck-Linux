//! The plugin store: the signed index from Ulanzi-Deck-Plugins and installs from it. The index is
//! trusted only after its signature checks out against a built-in key; it lists each package's
//! SHA-256, and each package must also carry its own valid signature. The last good index is kept,
//! so the store still lists plugins offline.

use std::fs;
use std::path::Path;
use std::time::Duration;

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

pub use ulanzi_core::plugin::store::{Listing, StoreEntry, StoreIndex, StorePackage};

fn net_err(what: &str) -> impl Fn(String) -> PluginError + '_ {
    move |e| PluginError::Io(format!("Can't reach the plugin store ({what}): {e}"))
}

/// `url` (https only), at most `max` bytes.
fn get(url: &str, max: u64) -> Result<Vec<u8>, String> {
    get_with(url, max, &|_| {})
}

/// [`get`], telling `progress` how many bytes have arrived as they do.
fn get_with(url: &str, max: u64, progress: &dyn Fn(u64)) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if !url.starts_with("https://") {
        return Err(format!("{url} is not an https address"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut reader = agent
        .get(url)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(c) => format!("the server said {c}"),
            _ => "no connection".to_string(),
        })?
        .into_body()
        .into_with_config()
        .limit(max)
        .reader();
    let cut = |_| "the download was cut short or too large".to_string();
    let mut out = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(cut)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
        progress(out.len() as u64);
    }
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

/// How long a kept index is used without asking the store again.
const FRESH_FOR: Duration = Duration::from_secs(30 * 60);

/// The kept index, checked again (it is a file anyone with access could edit).
fn kept_index(cache: &Path) -> Option<StoreIndex> {
    let v: serde_json::Value = serde_json::from_slice(&fs::read(cache).ok()?).ok()?;
    let data = v.get("data")?.as_str()?.as_bytes().to_vec();
    let sig = v.get("sig")?.as_str()?.to_string();
    read_index(&data, &sig).ok()
}

/// The store's index: the kept copy while it is under [`FRESH_FOR`] old (unless `refresh`), else
/// a fresh one (kept in `plugins`), else the kept copy marked offline.
pub fn fetch_index(plugins: &Path, refresh: bool) -> Result<Listing, PluginError> {
    let cache = plugins.join(CACHE);
    let recent = fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < FRESH_FOR);
    if recent
        && !refresh
        && let Some(index) = kept_index(&cache)
    {
        let icons = icons(plugins, &index, false);
        return Ok(Listing {
            index,
            offline: false,
            icons,
        });
    }
    let fresh = (|| {
        let data = get(INDEX_URL, MAX_INDEX_BYTES).map_err(net_err("list"))?;
        let sig = get(&format!("{INDEX_URL}.minisig"), MAX_SIGNATURE_BYTES).map_err(net_err("signature"))?;
        let sig =
            String::from_utf8(sig).map_err(|_| PluginError::Invalid("The signature is not text".into()))?;
        let index = read_index(&data, &sig)?;
        Ok::<_, PluginError>((index, data, sig))
    })();
    match fresh {
        Ok((index, data, sig)) => {
            let kept = serde_json::json!({ "data": String::from_utf8_lossy(&data), "sig": sig });
            if fs::create_dir_all(plugins).is_ok() {
                let _ = fs::write(&cache, kept.to_string());
            }
            let icons = icons(plugins, &index, true);
            Ok(Listing {
                index,
                offline: false,
                icons,
            })
        }
        Err(e) => match kept_index(&cache) {
            Some(index) => {
                let icons = icons(plugins, &index, false);
                Ok(Listing {
                    index,
                    offline: true,
                    icons,
                })
            }
            None => Err(e),
        },
    }
}

/// Kept store icons, in the plugins folder (hidden).
const ICONS: &str = ".store-icons";
const MAX_ICON_BYTES: u64 = 512 * 1024;

/// The store plugins' icons as local files (id → file): those kept from before, plus, when
/// `download`, the missing ones fetched now. One file per version; best effort.
fn icons(
    plugins: &Path,
    index: &StoreIndex,
    download: bool,
) -> std::collections::BTreeMap<String, std::path::PathBuf> {
    let dir = plugins.join(ICONS);
    let mut out = std::collections::BTreeMap::new();
    for e in &index.plugins {
        let Some(url) = e.icon.as_deref() else { continue };
        let ext = match url.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
            Some("svg") => "svg",
            Some("png") => "png",
            _ => continue,
        };
        // Ids and versions are checked by the manifest rules; keep the name plain anyway.
        let plain = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        };
        if !plain(&e.id) || !plain(&e.version) {
            continue;
        }
        let file = dir.join(format!("{}-{}.{ext}", e.id, e.version));
        if !file.is_file()
            && download
            && let Ok(bytes) = get(url, MAX_ICON_BYTES)
            && fs::create_dir_all(&dir).is_ok()
        {
            let _ = fs::write(&file, bytes);
        }
        if file.is_file() {
            out.insert(e.id.clone(), file);
        }
    }
    out
}

/// Download `entry`'s package (telling `progress` the bytes so far; the index gives the total) and
/// signature, and check the package against the index's hash.
pub fn download(entry: &StoreEntry, progress: &dyn Fn(u64)) -> Result<(Vec<u8>, String), PluginError> {
    let p = &entry.package;
    let bytes = get_with(&p.url, MAX_PACKAGE_BYTES, progress).map_err(net_err("download"))?;
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
