//! Downloading a release and checking it. Nothing is kept unless the whole file arrived and its
//! signature checks out.

use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ulanzi_core::store::Store;
use ulanzi_core::update::{self, InstallKind, Release};

use super::{UpdateError, verify};

/// Largest `SHA256SUMS` or signature read.
const MAX_SMALL_FILE: u64 = 64 * 1024;
const SUMS: &str = "SHA256SUMS";
const SIGNATURE: &str = "SHA256SUMS.minisig";

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        // Per read: a slow connection must be able to finish a large file.
        .timeout_per_read(Some(Duration::from_secs(60)))
        .build()
        .into()
}

fn url_of<'a>(release: &'a Release, name: &str) -> Result<&'a str, UpdateError> {
    release
        .asset(name)
        .map(|a| a.url.as_str())
        .ok_or(UpdateError::Unverified)
}

fn small_file(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, UpdateError> {
    agent
        .get(url)
        .call()
        .map_err(|_| UpdateError::Offline)?
        .into_body()
        .into_with_config()
        .limit(MAX_SMALL_FILE)
        .read_to_vec()
        .map_err(|_| UpdateError::Offline)
}

/// Write `url` to `path`, reporting bytes as they arrive. Errors when less than `total` arrives.
fn large_file(
    agent: &ureq::Agent,
    url: &str,
    path: &Path,
    total: u64,
    progress: &impl Fn(&str, u64, u64),
) -> Result<(), UpdateError> {
    let cut = |e: std::io::Error| match e.kind() {
        ErrorKind::StorageFull => UpdateError::Failed("There isn't enough disk space.".into()),
        _ => UpdateError::Offline,
    };
    let mut reader = agent
        .get(url)
        .call()
        .map_err(|_| UpdateError::Offline)?
        .into_body()
        .into_with_config()
        // One byte over, so a file of exactly the promised size is not cut off.
        .limit(total.saturating_add(1))
        .reader();
    let mut out = fs::File::create(path).map_err(cut)?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        match reader.read(&mut buf).map_err(cut)? {
            0 => break,
            n if done + n as u64 > total => return Err(UpdateError::Offline),
            n => {
                out.write_all(&buf[..n]).map_err(cut)?;
                done += n as u64;
                progress("downloading", done, total);
            }
        }
    }
    out.sync_all().map_err(cut)?;
    if done != total {
        return Err(UpdateError::Offline);
    }
    Ok(())
}

fn asset_for(release: &Release, kind: InstallKind) -> Result<String, UpdateError> {
    update::asset_name(kind, release.version(), std::env::consts::ARCH)
        .ok_or_else(|| UpdateError::Failed("This install can't update itself.".into()))
}

fn folder(store: &Store, release: &Release) -> PathBuf {
    store.cache_dir().join("updates").join(release.version())
}

/// The downloaded file for this release, checked again now. Needs no network.
pub fn ready_file(store: &Store, release: &Release, kind: InstallKind) -> Result<PathBuf, UpdateError> {
    let name = asset_for(release, kind)?;
    let dir = folder(store, release);
    let sums = fs::read(dir.join(SUMS)).map_err(|_| UpdateError::Unverified)?;
    let signature = fs::read_to_string(dir.join(SIGNATURE)).map_err(|_| UpdateError::Unverified)?;
    let path = dir.join(&name);
    verify::check(&sums, &signature, &name, &path)?;
    Ok(path)
}

/// Marks that a download is running; dropping it ends that.
#[derive(Debug)]
pub struct Downloading;

static BUSY: AtomicBool = AtomicBool::new(false);

/// Claim the one download allowed at a time: two would write the same files.
pub fn begin() -> Option<Downloading> {
    (!BUSY.swap(true, Ordering::SeqCst)).then_some(Downloading)
}

impl Drop for Downloading {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

/// Download and verify the file for an install of `kind`. Sends `downloading`, `verifying` and
/// `ready`. A file from an earlier try that failed to install needs no network; on any failure the
/// folder is removed.
pub fn fetch(
    store: &Store,
    release: &Release,
    kind: InstallKind,
    progress: impl Fn(&str, u64, u64),
) -> Result<PathBuf, UpdateError> {
    let name = asset_for(release, kind)?;
    let total = release.asset(&name).ok_or(UpdateError::Unverified)?.size;
    if let Ok(path) = ready_file(store, release, kind) {
        progress("ready", total, total);
        return Ok(path);
    }
    let dir = folder(store, release);
    let run = || -> Result<PathBuf, UpdateError> {
        let agent = agent();
        let failed = |e: std::io::Error| UpdateError::Failed(e.to_string());
        let sums = small_file(&agent, url_of(release, SUMS)?)?;
        let signature = small_file(&agent, url_of(release, SIGNATURE)?)?;
        fs::create_dir_all(&dir).map_err(failed)?;
        fs::write(dir.join(SUMS), &sums).map_err(failed)?;
        fs::write(dir.join(SIGNATURE), &signature).map_err(failed)?;
        let signature = String::from_utf8(signature).map_err(|_| UpdateError::Unverified)?;
        let part = dir.join(format!("{name}.part"));
        large_file(&agent, url_of(release, &name)?, &part, total, &progress)?;
        progress("verifying", total, total);
        verify::check(&sums, &signature, &name, &part)?;
        let path = dir.join(&name);
        fs::rename(&part, &path).map_err(failed)?;
        progress("ready", total, total);
        Ok(path)
    };
    let result = run();
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    result
}

/// Put `new_file` in place of the AppImage at `target`. The new file is complete beside the old
/// one before a rename swaps them, so a failure leaves the old one running. A symlink is followed:
/// the file it points to is what the menu entry runs.
pub fn swap_appimage(new_file: &Path, target: &Path) -> Result<(), UpdateError> {
    let denied = |e: std::io::Error| match e.kind() {
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => UpdateError::NotWritable,
        _ => UpdateError::Failed(e.to_string()),
    };
    let real = target.canonicalize().map_err(denied)?;
    let dir = real.parent().ok_or(UpdateError::NotWritable)?;
    let name = real
        .file_name()
        .ok_or(UpdateError::NotWritable)?
        .to_string_lossy();
    let next = dir.join(format!(".{name}.new"));
    let swap = || -> std::io::Result<()> {
        fs::copy(new_file, &next)?;
        fs::set_permissions(&next, fs::Permissions::from_mode(0o755))?;
        // On disk before the swap: a power cut must not leave half a program.
        fs::File::open(&next)?.sync_all()?;
        fs::rename(&next, &real)
    };
    swap().map_err(|e| {
        let _ = fs::remove_file(&next);
        denied(e)
    })
}
