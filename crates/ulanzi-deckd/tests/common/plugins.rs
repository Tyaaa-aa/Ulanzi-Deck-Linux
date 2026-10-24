//! Builds example/probe plugins to wasm components for tests.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::plugins::{Event, Exit, InstanceHandle, Output, PluginEngine, spawn_instance};

/// Generous bound for anything that should happen "promptly"; tests never sleep unbounded.
const PROMPTLY: Duration = Duration::from_secs(5);

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

/// A running instance plus everything that must outlive it.
pub struct Running {
    pub handle: InstanceHandle,
    pub out: mpsc::UnboundedReceiver<Output>,
    pub _engine: PluginEngine,
    pub _dir: tempfile::TempDir,
    pub _cache: tempfile::TempDir,
}

pub fn start(package: &str, manifest: &str, secrets: BTreeMap<String, String>) -> Running {
    let cache = tempfile::tempdir().expect("cache");
    let dir = fixture(package, manifest);
    let engine = PluginEngine::new(cache.path()).expect("engine");
    let manifest = Manifest::parse(manifest).expect("manifest");
    let loaded = engine
        .load(&manifest, &dir.path().join("plugin.wasm"))
        .expect("load");
    let (tx, out) = mpsc::unbounded_channel();
    let handle =
        spawn_instance(&loaded, &engine, secrets, tx, tokio::runtime::Handle::current()).expect("spawn");
    Running {
        handle,
        out,
        _engine: engine,
        _dir: dir,
        _cache: cache,
    }
}

impl Running {
    pub fn send(&self, event: Event) {
        // The queue holds 1024 events and tests send a handful: try_send never blocks the runtime.
        if self.handle.events.try_send(event).is_err() {
            panic!("event queue refused an event");
        }
    }

    /// Queue an action; the receiver resolves with the plugin's reply.
    pub fn queue(&self, action: &str, params: Value) -> oneshot::Receiver<Result<(), String>> {
        let (reply, rx) = oneshot::channel();
        self.send(Event::Action {
            action: action.into(),
            params,
            reply,
        });
        rx
    }

    pub async fn act(&self, action: &str, params: Value) -> Result<(), String> {
        let rx = self.queue(action, params);
        tokio::time::timeout(PROMPTLY, rx)
            .await
            .expect("action reply in time")
            .expect("reply sent")
    }

    /// Read outputs until `want` shows up; false when `within` runs out first.
    pub async fn saw(&mut self, want: &Output, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            match tokio::time::timeout_at(deadline, self.out.recv()).await {
                Ok(Some(o)) if &o == want => return true,
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return false,
            }
        }
    }

    pub async fn exit(mut self) -> Exit {
        // The handle stops the instance when dropped, so `exited` is awaited by reference.
        tokio::time::timeout(PROMPTLY, &mut self.handle.exited)
            .await
            .expect("instance exits in time")
            .expect("exit reported")
    }
}

/// Read outputs until a state for `key` shows up; its value, or None when `within` runs out.
pub async fn state_of(p: &mut Running, key: &str, within: Duration) -> Option<String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, p.out.recv()).await {
            Ok(Some(Output::State { key: k, value })) if k == key => return Some(value),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}
