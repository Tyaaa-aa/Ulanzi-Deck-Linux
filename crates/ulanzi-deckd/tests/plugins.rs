mod common;

use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::plugins::{PluginEngine, PluginError};

const HELLO_PACKAGE: &str = "ulanzi-plugin-hello";

fn hello_manifest() -> Manifest {
    let text = include_str!("../../../examples/plugins/hello/plugin.toml");
    Manifest::parse(text).expect("hello manifest")
}

fn has_file(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| match e.file_type() {
        Ok(t) if t.is_dir() => has_file(&e.path()),
        Ok(t) => t.is_file(),
        Err(_) => false,
    })
}

#[tokio::test]
async fn loads_hello() {
    let cache = tempfile::tempdir().expect("cache");
    let dir = common::plugins::fixture(
        HELLO_PACKAGE,
        include_str!("../../../examples/plugins/hello/plugin.toml"),
    );
    let engine = PluginEngine::new(cache.path()).expect("engine");
    let loaded = engine
        .load(&hello_manifest(), &dir.path().join("plugin.wasm"))
        .expect("load hello");
    assert_eq!(loaded.manifest.id, "hello");
    // The cache worker writes in the background; give it a moment.
    for _ in 0..100 {
        if has_file(cache.path()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("compile cache stayed empty");
}

#[tokio::test]
async fn refuses_garbage_wasm() {
    let cache = tempfile::tempdir().expect("cache");
    let dir = tempfile::tempdir().expect("dir");
    let wasm = dir.path().join("plugin.wasm");
    std::fs::write(&wasm, b"").expect("write");
    let engine = PluginEngine::new(cache.path()).expect("engine");
    let err = engine.load(&hello_manifest(), &wasm).err().expect("must fail");
    assert!(matches!(err, PluginError::Load(_)), "{err:?}");
}
