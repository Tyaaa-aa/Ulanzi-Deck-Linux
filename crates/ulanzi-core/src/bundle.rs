//! `.ulanzideck` files: one or more profiles (with their pictures) and optionally the settings, in a
//! ZIP. Used for sharing, templates and snapshots. Reading checks everything before anything is
//! written: paths, sizes and every picture.

use std::io::{Cursor, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{Model, Profile, Settings};
use crate::store::{ProfileLoad, Store, parse_profile};

pub const FORMAT: u32 = 1;
/// Largest total (uncompressed) size accepted.
pub const MAX_TOTAL: u64 = 50 * 1024 * 1024;
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "svg"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    /// RFC 3339.
    pub created: String,
    #[serde(default)]
    pub includes_settings: bool,
    #[serde(default)]
    pub profiles: Vec<ManifestProfile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestProfile {
    pub id: Uuid,
    pub name: String,
    pub model: Model,
    pub pages: u32,
    pub keys: u32,
}

/// A profile and its pictures (`icons/<name>`, bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct BundleProfile {
    pub profile: Profile,
    pub icons: Vec<(String, Vec<u8>)>,
}

/// A checked bundle, ready to import.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub manifest: Manifest,
    pub profiles: Vec<BundleProfile>,
    pub settings: Option<Settings>,
}

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("That file isn't an Ulanzi Deck profile file")]
    NotABundle,
    #[error("That file is too large (over 50 MB)")]
    TooLarge,
    #[error("That file contains something it shouldn't: {0}")]
    UnsafePath(String),
    #[error("{0} isn't a valid picture")]
    BadImage(String),
    #[error("A profile in the file can't be read: {0}")]
    BadProfile(String),
    #[error("That file was made by a newer Ulanzi Deck (format {0}); update to open it")]
    NewerFormat(u32),
    #[error("{0}")]
    Io(String),
}

impl From<crate::error::CoreError> for BundleError {
    fn from(e: crate::error::CoreError) -> Self {
        BundleError::Io(e.to_string())
    }
}

impl From<std::io::Error> for BundleError {
    fn from(e: std::io::Error) -> Self {
        BundleError::Io(e.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Import as a copy with a new id and name.
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    pub conflict: Conflict,
    /// Keep the profiles' auto-switch apps.
    pub keep_rules: bool,
    /// Also take the bundle's settings (when it has them). The current OBS password is kept.
    pub settings: bool,
    /// The connected deck, to warn about profiles made for the other model.
    pub device: Option<Model>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: Vec<Uuid>,
    pub replaced: Vec<Uuid>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
}

fn all_profiles(store: &Store) -> Vec<Profile> {
    store
        .list_profiles()
        .into_iter()
        .filter_map(|l| match l {
            ProfileLoad::Ok(p) => Some(*p),
            _ => None,
        })
        .collect()
}

fn image_ext(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, e)| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Pack `ids` (all profiles when empty), optionally with the settings (OBS password removed).
pub fn export(store: &Store, ids: &[Uuid], include_settings: bool) -> Result<Vec<u8>, BundleError> {
    export_with(store, ids, include_settings, &Default::default())
}

/// Like [`export`], also removing the plugin secrets in `declared` (secret keys by plugin id).
/// Keys that look like credentials are always removed.
pub fn export_with(
    store: &Store,
    ids: &[Uuid],
    include_settings: bool,
    declared: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> Result<Vec<u8>, BundleError> {
    let profiles: Vec<Profile> = if ids.is_empty() {
        all_profiles(store)
    } else {
        ids.iter()
            .map(|id| store.load_profile(*id))
            .collect::<Result<_, _>>()?
    };
    let mut out = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let mut icons = Vec::new();
        if let Ok(entries) = std::fs::read_dir(store.profile_dir(profile.id).join("icons")) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().is_ok_and(|t| t.is_file()) && image_ext(&name) {
                    icons.push((format!("icons/{name}"), std::fs::read(e.path())?));
                }
            }
        }
        out.push(BundleProfile { profile, icons });
    }
    let settings = if include_settings {
        let mut s = store.load_settings()?;
        s.obs.password = None;
        s.strip_plugin_secrets(declared);
        Some(s)
    } else {
        None
    };
    let manifest = Manifest {
        format: FORMAT,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        includes_settings: include_settings,
        profiles: out
            .iter()
            .map(|bp| ManifestProfile {
                id: bp.profile.id,
                name: bp.profile.name.clone(),
                model: bp.profile.model,
                pages: bp.profile.pages.len() as u32,
                keys: bp.profile.pages.iter().map(|pg| pg.keys.len() as u32).sum(),
            })
            .collect(),
    };
    to_bytes(&Bundle {
        manifest,
        profiles: out,
        settings,
    })
}

/// Write a bundle to bytes (the OBS password is never written).
pub fn to_bytes(b: &Bundle) -> Result<Vec<u8>, BundleError> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut put = |name: &str, data: &[u8]| -> Result<(), BundleError> {
        zip.start_file(name, opts)
            .map_err(|e| BundleError::Io(e.to_string()))?;
        zip.write_all(data)?;
        Ok(())
    };
    put("manifest.toml", to_toml(&b.manifest)?.as_bytes())?;
    if let Some(s) = &b.settings {
        let mut s = s.clone();
        s.obs.password = None;
        s.strip_plugin_secrets(&Default::default());
        put("settings.toml", to_toml(&s)?.as_bytes())?;
    }
    for bp in &b.profiles {
        let dir = format!("profiles/{}", bp.profile.id);
        put(&format!("{dir}/profile.toml"), to_toml(&bp.profile)?.as_bytes())?;
        for (rel, data) in &bp.icons {
            put(&format!("{dir}/{rel}"), data)?;
        }
    }
    Ok(zip
        .finish()
        .map_err(|e| BundleError::Io(e.to_string()))?
        .into_inner())
}

fn to_toml<T: Serialize>(v: &T) -> Result<String, BundleError> {
    toml::to_string_pretty(v).map_err(|e| BundleError::Io(e.to_string()))
}

/// What an entry of the ZIP is.
enum Entry {
    Dir,
    Manifest,
    Settings,
    Profile(Uuid),
    Icon(Uuid, String),
}

fn classify(name: &str) -> Result<Entry, BundleError> {
    let unsafe_path = || BundleError::UnsafePath(name.to_string());
    if name.starts_with('/') || name.contains('\\') || name.split('/').any(|c| c == ".." || c == ".") {
        return Err(unsafe_path());
    }
    if name.ends_with('/') {
        return Ok(Entry::Dir);
    }
    match name.split('/').collect::<Vec<_>>().as_slice() {
        ["manifest.toml"] => Ok(Entry::Manifest),
        ["settings.toml"] => Ok(Entry::Settings),
        ["profiles", id, "profile.toml"] => {
            Uuid::parse_str(id).map(Entry::Profile).map_err(|_| unsafe_path())
        }
        ["profiles", id, "icons", file] if !file.is_empty() && image_ext(file) => {
            let id = Uuid::parse_str(id).map_err(|_| unsafe_path())?;
            Ok(Entry::Icon(id, (*file).to_string()))
        }
        _ => Err(unsafe_path()),
    }
}

fn check_image(name: &str, data: &[u8]) -> Result<(), BundleError> {
    let bad = || BundleError::BadImage(name.to_string());
    if name.to_ascii_lowercase().ends_with(".svg") {
        let opts = resvg::usvg::Options {
            resources_dir: None,
            ..Default::default()
        };
        resvg::usvg::Tree::from_data(data, &opts).map_err(|_| bad())?;
        return Ok(());
    }
    let mut reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| bad())?;
    if reader.format().is_none() {
        return Err(bad());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| bad())?;
    Ok(())
}

/// Only the manifest (for listing snapshots and previews), without checking the rest.
pub fn read_manifest(bytes: &[u8]) -> Result<Manifest, BundleError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| BundleError::NotABundle)?;
    let f = zip
        .by_name("manifest.toml")
        .map_err(|_| BundleError::NotABundle)?;
    let mut text = String::new();
    f.take(1024 * 1024).read_to_string(&mut text)?;
    toml::from_str(&text).map_err(|_| BundleError::NotABundle)
}

/// Read and check a bundle. Nothing is written.
pub fn read(bytes: &[u8]) -> Result<Bundle, BundleError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| BundleError::NotABundle)?;
    // Sizes and names first, before decompressing anything.
    let mut total = 0u64;
    let mut kinds = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let f = zip.by_index_raw(i).map_err(|_| BundleError::NotABundle)?;
        let name = f.name().map_err(|_| BundleError::NotABundle)?.into_owned();
        let kind = classify(&name)?;
        total = total.saturating_add(f.size());
        if total > MAX_TOTAL {
            return Err(BundleError::TooLarge);
        }
        kinds.push((kind, name));
    }
    let mut manifest = None;
    let mut settings = None;
    let mut profiles: Vec<BundleProfile> = Vec::new();
    let mut icons: Vec<(Uuid, String, Vec<u8>)> = Vec::new();
    let mut read_total = 0u64;
    for (i, (kind, name)) in kinds.into_iter().enumerate() {
        if matches!(kind, Entry::Dir) {
            continue;
        }
        let f = zip.by_index(i).map_err(|_| BundleError::NotABundle)?;
        // Never trust the declared size: cap what is actually read.
        let mut data = Vec::new();
        f.take(MAX_TOTAL - read_total + 1).read_to_end(&mut data)?;
        read_total += data.len() as u64;
        if read_total > MAX_TOTAL {
            return Err(BundleError::TooLarge);
        }
        let text = || String::from_utf8(data.clone()).map_err(|_| BundleError::NotABundle);
        match kind {
            Entry::Manifest => {
                let m: Manifest = toml::from_str(&text()?).map_err(|_| BundleError::NotABundle)?;
                if m.format > FORMAT {
                    return Err(BundleError::NewerFormat(m.format));
                }
                manifest = Some(m);
            }
            Entry::Settings => {
                settings = Some(
                    toml::from_str::<Settings>(&text()?)
                        .map_err(|e| BundleError::BadProfile(e.to_string()))?,
                );
            }
            Entry::Profile(id) => {
                let p = parse_profile(&text()?).map_err(|e| BundleError::BadProfile(e.to_string()))?;
                if p.id != id {
                    return Err(BundleError::BadProfile(format!(
                        "{name} has a different id inside"
                    )));
                }
                profiles.push(BundleProfile {
                    profile: p,
                    icons: Vec::new(),
                });
            }
            Entry::Icon(id, file) => {
                check_image(&name, &data)?;
                icons.push((id, file, data));
            }
            Entry::Dir => {}
        }
    }
    let manifest = manifest.ok_or(BundleError::NotABundle)?;
    for (id, file, data) in icons {
        if let Some(bp) = profiles.iter_mut().find(|bp| bp.profile.id == id) {
            bp.icons.push((format!("icons/{file}"), data));
        }
    }
    Ok(Bundle {
        manifest,
        profiles,
        settings,
    })
}

/// `name`, or `name (2)`, `name (3)`… so it differs from every name in `taken`.
fn unique_name(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|c| !taken.contains(c))
        .unwrap_or_else(|| name.to_string())
}

/// Write a profile and its pictures into a fresh directory, then move it into place.
fn write_profile(store: &Store, p: &Profile, icons: &[(String, Vec<u8>)]) -> Result<(), BundleError> {
    let tmp = store.profiles_dir().join(format!(".import-{}", Uuid::new_v4()));
    std::fs::create_dir_all(tmp.join("icons"))?;
    let result = (|| -> Result<(), BundleError> {
        std::fs::write(
            tmp.join("profile.toml"),
            toml::to_string_pretty(p).map_err(|e| BundleError::Io(e.to_string()))?,
        )?;
        for (rel, data) in icons {
            let file = Path::new(rel)
                .file_name()
                .ok_or_else(|| BundleError::UnsafePath(rel.clone()))?;
            std::fs::write(tmp.join("icons").join(file), data)?;
        }
        let dest = store.profile_dir(p.id);
        if dest.exists() {
            store.delete_profile(p.id)?;
        }
        std::fs::rename(&tmp, &dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result
}

/// Import a checked bundle.
pub fn import(store: &Store, bundle: &Bundle, opts: &ImportOptions) -> Result<ImportReport, BundleError> {
    let existing = all_profiles(store);
    let mut names: Vec<String> = existing.iter().map(|p| p.name.clone()).collect();
    let mut report = ImportReport::default();
    for bp in &bundle.profiles {
        let mut p = bp.profile.clone();
        if !opts.keep_rules {
            p.r#match = Default::default();
        }
        let clash = existing.iter().any(|e| e.id == p.id);
        if clash {
            match opts.conflict {
                Conflict::Skip => {
                    report.skipped.push(p.name.clone());
                    continue;
                }
                Conflict::Replace => {}
                Conflict::KeepBoth => {
                    p.id = Uuid::new_v4();
                    p.name = unique_name(&p.name, &names);
                }
            }
        }
        if let Some(dev) = opts.device
            && p.model != dev
        {
            report.warnings.push(format!(
                "“{}” was made for the {:?}; anything your {:?} doesn't have is kept but not shown",
                p.name, p.model, dev
            ));
        }
        write_profile(store, &p, &bp.icons)?;
        names.push(p.name.clone());
        if clash && opts.conflict == Conflict::Replace {
            report.replaced.push(p.id);
        } else {
            report.imported.push(p.id);
        }
    }
    if opts.settings
        && let Some(s) = &bundle.settings
    {
        let mut s = s.clone();
        s.obs.password = store.load_settings()?.obs.password;
        store.save_settings(&s)?;
    }
    Ok(report)
}

/// Every shell command the bundle's profiles would run (Run Command, Launch App by command line,
/// the Command Output widget), for the import preview: imported files come from other people.
pub fn commands(b: &Bundle) -> Vec<String> {
    use crate::model::{Action, ScreenConfig, Widget};
    let mut out = Vec::new();
    for bp in &b.profiles {
        for a in bp.profile.actions() {
            match a {
                Action::Command { cmd, .. } => out.push(cmd.clone()),
                Action::LaunchApp { exec: Some(e), .. } => out.push(e.clone()),
                _ => {}
            }
        }
        if let ScreenConfig::Custom { widgets, .. } = &bp.profile.screen {
            for w in widgets {
                if let Widget::Command { cmd, .. } = w {
                    out.push(cmd.clone());
                }
            }
        }
    }
    out.retain(|c| !c.trim().is_empty());
    out
}
