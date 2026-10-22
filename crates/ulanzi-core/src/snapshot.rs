//! Automatic backups: everything (profiles and settings) as a `.ulanzideck` in the state directory,
//! taken before risky changes. The newest [`KEEP`] are kept.

use std::path::PathBuf;

use crate::bundle::{self, BundleError, Conflict, ImportOptions};
use crate::store::{ProfileLoad, Store};

pub const KEEP: usize = 20;
const EXT: &str = "ulanzideck";
const STAMP: &str = "%Y%m%d-%H%M%S";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    /// File name without the extension, e.g. `20261023-101500-000-before-import`.
    pub name: String,
    /// RFC 3339.
    pub created: String,
    pub reason: String,
    pub profiles: Vec<String>,
}

fn dir(store: &Store) -> PathBuf {
    store.state_dir().join("snapshots")
}

/// `before-import`, `daily`…: lowercase letters, digits and single dashes.
fn sanitise(reason: &str) -> String {
    let mut out = String::new();
    for c in reason.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "manual".into() } else { out }
}

/// (timestamp, reason) from a snapshot name.
fn parse_name(name: &str) -> Option<(chrono::NaiveDateTime, String)> {
    let mut parts = name.splitn(4, '-');
    let (date, time, _seq, reason) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let ts = chrono::NaiveDateTime::parse_from_str(&format!("{date}-{time}"), STAMP).ok()?;
    Some((ts, reason.to_string()))
}

fn info(path: &std::path::Path) -> Option<SnapshotInfo> {
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let (ts, reason) = parse_name(&name)?;
    let manifest = std::fs::read(path)
        .ok()
        .and_then(|b| bundle::read_manifest(&b).ok());
    Some(SnapshotInfo {
        created: ts.and_utc().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        reason,
        profiles: manifest
            .map(|m| m.profiles.into_iter().map(|p| p.name).collect())
            .unwrap_or_default(),
        name,
    })
}

/// Every snapshot, newest first.
pub fn list(store: &Store) -> Vec<SnapshotInfo> {
    let Ok(entries) = std::fs::read_dir(dir(store)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXT))
        .collect();
    paths.sort();
    paths.reverse();
    paths.iter().filter_map(|p| info(p)).collect()
}

/// Back up everything now; older snapshots beyond [`KEEP`] are removed.
pub fn take(store: &Store, reason: &str) -> Result<SnapshotInfo, BundleError> {
    let bytes = bundle::export(store, &[], true)?;
    let d = dir(store);
    std::fs::create_dir_all(&d)?;
    let stamp = chrono::Utc::now().format(STAMP).to_string();
    let reason = sanitise(reason);
    // Several in the same second get increasing sequence numbers (and still sort by age).
    let taken: Vec<String> = std::fs::read_dir(&d)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let n = (0..1000)
        .find(|n| !taken.iter().any(|t| t.starts_with(&format!("{stamp}-{n:03}-"))))
        .ok_or_else(|| BundleError::Io("too many snapshots in one second".into()))?;
    let path = d.join(format!("{stamp}-{n:03}-{reason}.{EXT}"));
    crate::store::write_atomic(&path, &bytes)?;
    for old in list(store).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(d.join(format!("{}.{EXT}", old.name)));
    }
    info(&path).ok_or_else(|| BundleError::Io("snapshot unreadable".into()))
}

/// Put everything back as it was in snapshot `name`. The current state is snapshotted first;
/// that snapshot is returned so the restore can be undone. The OBS password is kept.
pub fn restore(store: &Store, name: &str) -> Result<SnapshotInfo, BundleError> {
    if name.contains('/') || name.contains("..") {
        return Err(BundleError::UnsafePath(name.into()));
    }
    let bytes = std::fs::read(dir(store).join(format!("{name}.{EXT}")))?;
    let snap = bundle::read(&bytes)?;
    let before = take(store, "before-restore")?;
    let keep: Vec<uuid::Uuid> = snap.profiles.iter().map(|p| p.profile.id).collect();
    for l in store.list_profiles() {
        if let ProfileLoad::Ok(p) = l
            && !keep.contains(&p.id)
        {
            store.delete_profile(p.id)?;
        }
    }
    bundle::import(
        store,
        &snap,
        &ImportOptions {
            conflict: Conflict::Replace,
            keep_rules: true,
            settings: true,
            device: None,
        },
    )?;
    Ok(before)
}

/// True when no snapshot was taken in the last 24 hours.
pub fn due_daily(store: &Store) -> bool {
    let day_ago = chrono::Utc::now().naive_utc() - chrono::Duration::hours(24);
    !list(store)
        .iter()
        .filter_map(|s| parse_name(&s.name))
        .any(|(ts, _)| ts > day_ago)
}
