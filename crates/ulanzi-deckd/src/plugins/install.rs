//! Installing a plugin from a folder (developer mode): check it, copy it next to the installed
//! plugins as `<id>.new/`, stop the old instance, swap the folders, start the new one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::package;
use super::signature;
use super::supervisor::{Host, read_manifest, stop_run};
use ulanzi_core::plugin::Inspected;

use super::{LoadedPlugin, PluginEngine, PluginError};

const DEVELOPER_ONLY: &str = "Turn on Developer mode to install plugins from a folder";
const DEVELOPER_ONLY_FILE: &str = "Turn on Developer mode to install unverified plugins";

/// What a plugin folder may hold; everything else is left behind.
const FILES: [&str; 2] = ["plugin.toml", "plugin.wasm"];
const ASSETS: &str = "assets";

/// Run blocking file work off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, PluginError> + Send + 'static,
) -> Result<T, PluginError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| PluginError::Io(format!("install task failed: {e}")))?
}

fn io_err(what: &str, path: &Path) -> impl FnOnce(io::Error) -> PluginError {
    let at = format!("{what} {}", path.display());
    move |e| PluginError::Io(format!("{at}: {e}"))
}

/// A regular file, not a symlink.
fn regular_file(path: &Path) -> Result<(), PluginError> {
    let meta = fs::symlink_metadata(path).map_err(io_err("cannot read", path))?;
    if meta.file_type().is_file() {
        Ok(())
    } else {
        Err(PluginError::Io(format!(
            "{} is not a regular file",
            path.display()
        )))
    }
}

/// Check `src` as a plugin folder and compile it. Nothing is written.
fn validate(src: &Path, engine: &PluginEngine) -> Result<LoadedPlugin, PluginError> {
    for file in FILES {
        regular_file(&src.join(file))?;
    }
    let fallback = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let manifest = read_manifest(src, &fallback).map_err(|bad| PluginError::Load(bad.reason))?;
    check_icons(src, &manifest)?;
    engine.load(&manifest, &src.join("plugin.wasm"))
}

/// Every `assets/` icon the manifest names is a file in the plugin.
fn check_icons(src: &Path, m: &ulanzi_core::plugin::Manifest) -> Result<(), PluginError> {
    for icon in m.icon_files() {
        if !fs::symlink_metadata(src.join(icon)).is_ok_and(|m| m.is_file()) {
            return Err(PluginError::Invalid(format!(
                "plugin.toml names the icon {icon}, which is not in the plugin"
            )));
        }
    }
    Ok(())
}

/// Copy the plugin's files from `src` into `to` (made empty first). Symlinks are refused.
fn copy_plugin(src: &Path, to: &Path) -> Result<(), PluginError> {
    if fs::symlink_metadata(to).is_ok() {
        fs::remove_dir_all(to).map_err(io_err("cannot clear", to))?;
    }
    fs::create_dir_all(to).map_err(io_err("cannot create", to))?;
    for file in FILES {
        regular_file(&src.join(file))?;
        fs::copy(src.join(file), to.join(file)).map_err(io_err("cannot copy", &src.join(file)))?;
    }
    let assets = src.join(ASSETS);
    match fs::symlink_metadata(&assets) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("cannot read", &assets)(e)),
        Ok(meta) if meta.file_type().is_dir() => copy_tree(&assets, &to.join(ASSETS)),
        Ok(_) => Err(PluginError::Io(format!("{} is not a folder", assets.display()))),
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), PluginError> {
    fs::create_dir_all(to).map_err(io_err("cannot create", to))?;
    for entry in fs::read_dir(from).map_err(io_err("cannot read", from))? {
        let entry = entry.map_err(io_err("cannot read", from))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(io_err("cannot read", &path))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&path, &target)?;
        } else if kind.is_file() {
            fs::copy(&path, &target).map_err(io_err("cannot copy", &path))?;
        } else {
            return Err(PluginError::Io(format!(
                "{} is a link or special file; plugins may only hold files and folders",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Move `<dir>/<id>.new/` to `<dir>/<id>/`, removing the old folder. On failure the old folder
/// is put back.
fn swap_in(dir: &Path, id: &str) -> Result<(), PluginError> {
    let target = dir.join(id);
    let staged = dir.join(format!("{id}.new"));
    let aside = dir.join(format!(".{id}.old"));
    if fs::symlink_metadata(&aside).is_ok() {
        fs::remove_dir_all(&aside).map_err(io_err("cannot clear", &aside))?;
    }
    let had_old = fs::symlink_metadata(&target).is_ok();
    if had_old && let Err(e) = fs::rename(&target, &aside) {
        let _ = fs::remove_dir_all(&staged);
        return Err(io_err("cannot move", &target)(e));
    }
    if let Err(e) = fs::rename(&staged, &target) {
        if had_old && let Err(e) = fs::rename(&aside, &target) {
            // The next daemon start puts it back (see the plugin scan).
            tracing::warn!(plugin = %id, "cannot put the old plugin back: {e}");
        }
        let _ = fs::remove_dir_all(&staged);
        return Err(io_err("cannot move", &staged)(e));
    }
    if had_old && let Err(e) = fs::remove_dir_all(&aside) {
        tracing::warn!("cannot delete the replaced plugin {}: {e}", aside.display());
    }
    Ok(())
}

/// A checked package: its unpacked folder (removed when dropped), the plugin and whether a trusted
/// key signed it.
pub(super) struct Unpacked {
    pub tmp: tempfile::TempDir,
    pub loaded: LoadedPlugin,
    pub verified: bool,
}

/// The signature next to a package file (`<file>.minisig`), if there is one.
fn sidecar_signature(file: &Path) -> Result<Option<String>, PluginError> {
    let mut path = file.as_os_str().to_owned();
    path.push(".minisig");
    let path = PathBuf::from(path);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err("cannot read", &path)(e)),
        Ok(m) if !m.is_file() || m.len() > 4096 => Err(PluginError::Invalid(format!(
            "{} is not a signature file",
            path.display()
        ))),
        Ok(_) => fs::read_to_string(&path)
            .map(Some)
            .map_err(io_err("cannot read", &path)),
    }
}

/// Unpack the package `file` (signed by `<file>.minisig` when that exists) into a fresh hidden
/// folder inside `plugins` and [`validate`] it.
fn unpack_and_validate(file: &Path, plugins: &Path, engine: &PluginEngine) -> Result<Unpacked, PluginError> {
    let meta = fs::metadata(file).map_err(io_err("cannot read", file))?;
    if !meta.is_file() {
        return Err(PluginError::Io(format!(
            "{} is not a regular file",
            file.display()
        )));
    }
    if meta.len() > package::MAX_PACKAGE_BYTES {
        return Err(PluginError::Invalid("The package is too large".into()));
    }
    let bytes = fs::read(file).map_err(io_err("cannot read", file))?;
    let sig = sidecar_signature(file)?;
    unpack_bytes(&bytes, sig.as_deref(), plugins, engine)
}

/// Unpack and [`validate`] a package held in memory; with `signature`, it must check out for this
/// plugin's id and version (a bad signature is refused, never ignored). The bytes checked are the
/// bytes unpacked: they are written to a private file first.
pub(super) fn unpack_bytes(
    bytes: &[u8],
    signature: Option<&str>,
    plugins: &Path,
    engine: &PluginEngine,
) -> Result<Unpacked, PluginError> {
    fs::create_dir_all(plugins).map_err(io_err("cannot create", plugins))?;
    // Hidden, so the plugin scan ignores it, and inside the plugins folder so it is never
    // somewhere unexpected. Removed when the guard drops, whatever happens.
    let tmp = tempfile::Builder::new()
        .prefix(".unpack-")
        .tempdir_in(plugins)
        .map_err(io_err("cannot make a temporary folder in", plugins))?;
    let copy = tempfile::Builder::new()
        .prefix(".package-")
        .tempfile_in(plugins)
        .map_err(io_err("cannot make a temporary file in", plugins))?;
    fs::write(copy.path(), bytes).map_err(io_err("cannot write", copy.path()))?;
    package::unpack(copy.path(), tmp.path())?;
    let loaded = validate(tmp.path(), engine)?;
    let verified = match signature {
        Some(sig) => {
            let m = &loaded.manifest;
            signature::verify(bytes, sig, &signature::package_name(&m.id, &m.version))?;
            true
        }
        None => false,
    };
    Ok(Unpacked {
        tmp,
        loaded,
        verified,
    })
}

impl Host {
    /// [`validate`] a folder without installing it or checking Developer mode.
    pub(super) async fn inspect_dir(&self, src: &Path) -> Result<Inspected, PluginError> {
        let engine = self.engine.clone().map_err(PluginError::Load)?;
        let src = src.to_path_buf();
        let loaded = blocking(move || validate(&src, &engine)).await?;
        Ok(self.inspected(loaded))
    }

    /// Unpack and [`validate`] a `.ulanziplugin` file without installing it or checking
    /// Developer mode.
    pub(super) async fn inspect_file(&self, file: &Path) -> Result<Inspected, PluginError> {
        let engine = self.engine.clone().map_err(PluginError::Load)?;
        let (file, dir) = (file.to_path_buf(), self.dir.clone());
        let u = blocking(move || unpack_and_validate(&file, &dir, &engine)).await?;
        let mut i = self.inspected(u.loaded);
        i.verified = u.verified;
        Ok(i)
    }

    fn inspected(&self, loaded: LoadedPlugin) -> Inspected {
        let installed_version = self.installed_version(&loaded.manifest.id);
        Inspected {
            manifest: loaded.manifest.clone(),
            installed_version,
            verified: false,
        }
    }

    pub(super) async fn install_dir(&self, src: &Path, developer_mode: bool) -> Result<String, PluginError> {
        if !developer_mode {
            return Err(PluginError::Permission(DEVELOPER_ONLY.into()));
        }
        let engine = self.engine.clone().map_err(PluginError::Load)?;
        let src = src.to_path_buf();
        let loaded = {
            let src = src.clone();
            blocking(move || validate(&src, &engine)).await?
        };
        self.install_checked(src, loaded, false).await
    }

    pub(super) async fn store_index(&self) -> Result<super::store::Listing, PluginError> {
        let dir = self.dir.clone();
        blocking(move || super::store::fetch_index(&dir)).await
    }

    pub(super) async fn store_install(&self, id: &str) -> Result<String, PluginError> {
        let engine = self.engine.clone().map_err(PluginError::Load)?;
        let listing = self.store_index().await?;
        let entry = listing
            .index
            .plugins
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| PluginError::Invalid(format!("The plugin store has no plugin \"{id}\"")))?;
        let dir = self.dir.clone();
        let u = blocking(move || {
            let (bytes, sig) = super::store::download(&entry)?;
            unpack_bytes(&bytes, Some(&sig), &dir, &engine)
        })
        .await?;
        self.install_unpacked(u, false).await
    }

    pub(super) fn verified(&self) -> Vec<String> {
        self.list()
            .into_iter()
            .filter(|p| self.dir.join(&p.id).join(signature::VERIFIED_MARKER).is_file())
            .map(|p| p.id)
            .collect()
    }

    /// Install (or replace) the plugin in a `.ulanziplugin` file.
    pub(super) async fn install_file(
        &self,
        file: &Path,
        developer_mode: bool,
    ) -> Result<String, PluginError> {
        let engine = self.engine.clone().map_err(PluginError::Load)?;
        let (file, dir) = (file.to_path_buf(), self.dir.clone());
        let u = blocking(move || unpack_and_validate(&file, &dir, &engine)).await?;
        self.install_unpacked(u, developer_mode).await
    }

    /// Install a checked package: signed ones always, unsigned ones in Developer mode only.
    pub(super) async fn install_unpacked(
        &self,
        u: Unpacked,
        developer_mode: bool,
    ) -> Result<String, PluginError> {
        if !u.verified && !developer_mode {
            return Err(PluginError::Permission(DEVELOPER_ONLY_FILE.into()));
        }
        let Unpacked {
            tmp,
            loaded,
            verified,
        } = u;
        let installed = self
            .install_checked(tmp.path().to_path_buf(), loaded, verified)
            .await;
        // Removing the unpacked copy may take a moment; keep it off the async runtime.
        let _ = blocking(move || {
            drop(tmp);
            Ok(())
        })
        .await;
        installed
    }

    /// Install the already validated plugin folder `src` (copied, so `src` stays as it is).
    async fn install_checked(
        &self,
        src: PathBuf,
        loaded: LoadedPlugin,
        verified: bool,
    ) -> Result<String, PluginError> {
        let id = loaded.manifest.id.clone();

        let _ops = self.ops.lock().await;
        let dir = self.dir.clone();
        let staged = dir.join(format!("{id}.new"));
        blocking(move || {
            copy_plugin(&src, &staged).inspect_err(|_| {
                let _ = fs::remove_dir_all(&staged);
            })
        })
        .await?;

        if let Some(run) = self.detach(&id) {
            stop_run(run).await;
        }
        let swapped = {
            let id = id.clone();
            blocking(move || swap_in(&dir, &id)).await
        };
        if let Err(e) = swapped {
            self.resume(&id);
            return Err(e);
        }
        // Says the package was signed; written by the host, never taken from a package.
        let marker = self.dir.join(&id).join(signature::VERIFIED_MARKER);
        let _ = if verified {
            fs::write(&marker, "signed with a key Ulanzi Deck trusts\n")
        } else {
            fs::remove_file(&marker).or_else(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })
        }
        .inspect_err(|e| tracing::warn!(plugin = %id, "cannot record whether it is signed: {e}"));
        tracing::info!(plugin = %id, verified, "installed");
        self.installed(loaded);
        Ok(id)
    }
}
