//! Installing a plugin from a folder (developer mode): check it, copy it next to the installed
//! plugins as `<id>.new/`, stop the old instance, swap the folders, start the new one.

use std::fs;
use std::io;
use std::path::Path;

use super::supervisor::{Host, read_manifest, stop_run};
use super::{LoadedPlugin, PluginEngine, PluginError};

const DEVELOPER_ONLY: &str = "Turn on Developer mode to install plugins from a folder";

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
    engine.load(&manifest, &src.join("plugin.wasm"))
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

impl Host {
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
        tracing::info!(plugin = %id, "installed from a folder");
        self.installed(loaded);
        Ok(id)
    }
}
