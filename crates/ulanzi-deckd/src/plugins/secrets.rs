//! Where plugins' secrets live: the system keyring (Secret Service), else a private file.
//! Secret values are never logged, and error messages never quote them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How long daemon start waits for the Secret Service before using the file instead.
const KEYRING_TIMEOUT: Duration = Duration::from_secs(3);
/// Keyring item attribute naming this application.
const APPLICATION: &str = "ulanzi-deck";

/// A plugin's secrets, keyed by plugin id. Saving an empty map forgets the plugin.
#[async_trait::async_trait]
pub trait SecretStore: Send + Sync {
    async fn load(&self, plugin: &str) -> Result<BTreeMap<String, String>, String>;
    async fn save(&self, plugin: &str, secrets: &BTreeMap<String, String>) -> Result<(), String>;
}

/// The fallback: a TOML file `{ <plugin> = { key = value } }` written with mode 0600.
pub struct FileSecrets {
    path: PathBuf,
    /// Serializes read-modify-write cycles within this process.
    lock: tokio::sync::Mutex<()>,
}

type AllSecrets = BTreeMap<String, BTreeMap<String, String>>;

impl FileSecrets {
    pub fn new(path: PathBuf) -> FileSecrets {
        FileSecrets {
            path,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    fn read_all(path: &Path) -> Result<AllSecrets, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AllSecrets::new()),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        // toml's error quotes the offending line, which may hold a secret: say only where.
        toml::from_str(&text).map_err(|e| {
            let at = e
                .span()
                .map(|s| format!(" (at byte {})", s.start))
                .unwrap_or_default();
            format!("{} is not valid{at}", path.display())
        })
    }

    async fn blocking<T: Send + 'static>(
        f: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| format!("secret file task failed: {e}"))?
    }
}

#[async_trait::async_trait]
impl SecretStore for FileSecrets {
    async fn load(&self, plugin: &str) -> Result<BTreeMap<String, String>, String> {
        let _guard = self.lock.lock().await;
        let path = self.path.clone();
        let mut all = Self::blocking(move || Self::read_all(&path)).await?;
        Ok(all.remove(plugin).unwrap_or_default())
    }

    async fn save(&self, plugin: &str, secrets: &BTreeMap<String, String>) -> Result<(), String> {
        let _guard = self.lock.lock().await;
        let path = self.path.clone();
        let plugin = plugin.to_string();
        let secrets = secrets.clone();
        Self::blocking(move || {
            let mut all = Self::read_all(&path)?;
            if secrets.is_empty() {
                if all.remove(&plugin).is_none() {
                    return Ok(());
                }
            } else {
                all.insert(plugin, secrets);
            }
            let text = toml::to_string(&all).map_err(|e| format!("cannot encode secrets: {e}"))?;
            ulanzi_core::store::write_atomic_mode(&path, text.as_bytes(), 0o600)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))
        })
        .await
    }
}

/// The system keyring: one item per plugin, attributes `{application = "ulanzi-deck", plugin =
/// <id>}`, label `Ulanzi Deck: <id> plugin`, secret = the map as JSON.
pub struct KeyringSecrets {
    keyring: oo7::Keyring,
}

impl KeyringSecrets {
    fn attributes(plugin: &str) -> [(&'static str, &str); 2] {
        [("application", APPLICATION), ("plugin", plugin)]
    }
}

#[async_trait::async_trait]
impl SecretStore for KeyringSecrets {
    async fn load(&self, plugin: &str) -> Result<BTreeMap<String, String>, String> {
        let keyring_err = |e: oo7::Error| format!("keyring: {e}");
        let items = self
            .keyring
            .search_items(&Self::attributes(plugin))
            .await
            .map_err(keyring_err)?;
        let Some(item) = items.first() else {
            return Ok(BTreeMap::new());
        };
        if item.is_locked().await.map_err(keyring_err)? {
            item.unlock().await.map_err(keyring_err)?;
        }
        let secret = item.secret().await.map_err(keyring_err)?;
        // serde_json's message can quote the content; say only where.
        serde_json::from_slice(secret.as_bytes()).map_err(|e| {
            format!(
                "the keyring item for {plugin} is not valid (line {}, column {})",
                e.line(),
                e.column()
            )
        })
    }

    async fn save(&self, plugin: &str, secrets: &BTreeMap<String, String>) -> Result<(), String> {
        let keyring_err = |e: oo7::Error| format!("keyring: {e}");
        let attributes = Self::attributes(plugin);
        if secrets.is_empty() {
            return self.keyring.delete(&attributes).await.map_err(keyring_err);
        }
        let json = serde_json::to_string(secrets).map_err(|e| format!("cannot encode secrets: {e}"))?;
        let label = format!("Ulanzi Deck: {plugin} plugin");
        self.keyring
            .create_item(&label, &attributes, oo7::Secret::text(json), true)
            .await
            .map_err(keyring_err)
    }
}

/// The system keyring when the Secret Service answers within 3 s, else `<config_dir>/plugin-secrets.toml`.
pub async fn system_secrets(config_dir: &Path) -> Arc<dyn SecretStore> {
    let why = match tokio::time::timeout(KEYRING_TIMEOUT, oo7::Keyring::new()).await {
        Ok(Ok(keyring)) => return Arc::new(KeyringSecrets { keyring }),
        Ok(Err(e)) => e.to_string(),
        Err(_) => format!("no answer within {} s", KEYRING_TIMEOUT.as_secs()),
    };
    let path = config_dir.join("plugin-secrets.toml");
    tracing::warn!(
        "system keyring unavailable ({why}); plugin secrets are kept in {}",
        path.display()
    );
    Arc::new(FileSecrets::new(path))
}
