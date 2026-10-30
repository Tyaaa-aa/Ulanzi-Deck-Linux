//! The plugin store's index, as the plugins repo's CI writes it and the app reads it.

use serde::{Deserialize, Serialize};

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
    /// Who makes it, from its manifest.
    #[serde(default)]
    pub author: Option<String>,
    /// Made by the Ulanzi Deck project: set by the store (its own list), never by a plugin.
    #[serde(default)]
    pub official: bool,
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

/// What was read: the index, whether it came from the kept copy (offline), and the plugins' icons
/// downloaded so far (id → a file the app can read).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listing {
    pub index: StoreIndex,
    pub offline: bool,
    #[serde(default)]
    pub icons: std::collections::BTreeMap<String, std::path::PathBuf>,
}

impl StoreEntry {
    /// What the store offers next to an installed version: nothing new, an update, or a version
    /// the app is too old for.
    pub fn offer(&self, installed: Option<&str>) -> Offer {
        let newer_app = self
            .min_app
            .as_deref()
            .is_some_and(|m| super::manifest::semver(m) > super::manifest::semver(crate::APP_VERSION));
        match installed {
            _ if newer_app => Offer::NeedsApp(self.min_app.clone().unwrap_or_default()),
            None => Offer::Install,
            Some(v) if super::manifest::semver(&self.version) > super::manifest::semver(v) => Offer::Update,
            Some(_) => Offer::Installed,
        }
    }
}

/// What a store entry's button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    Install,
    Update,
    Installed,
    /// The app is older than the plugin's `min_app`.
    NeedsApp(String),
}
