//! Builds example/probe plugins to wasm components for tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Build `package` for wasm32-wasip2 (once per test binary) and return the `.wasm` path.
pub fn wasm(package: &str) -> PathBuf {
    static BUILT: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let mut built = BUILT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(p) = built.get(package) {
        return p.clone();
    }
    let root = workspace_root();
    let target = root.join("target/test-plugins");
    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "--target", "wasm32-wasip2", "-p", package])
        .arg("--target-dir")
        .arg(&target)
        .current_dir(&root)
        .status()
        .expect("run cargo");
    assert!(status.success(), "building plugin {package} failed");
    let path = target
        .join("wasm32-wasip2/release")
        .join(format!("{}.wasm", package.replace('-', "_")));
    assert!(path.exists(), "missing {}", path.display());
    built.insert(package.to_string(), path.clone());
    path
}

/// A plugin folder: `plugin.toml` with `manifest` plus the built `plugin.wasm`.
pub fn fixture(package: &str, manifest: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plugin.toml"), manifest).expect("write manifest");
    std::fs::copy(wasm(package), dir.path().join("plugin.wasm")).expect("copy wasm");
    dir
}
