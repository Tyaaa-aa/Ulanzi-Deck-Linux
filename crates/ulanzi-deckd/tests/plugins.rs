mod common;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::plugins::{Event, Exit, InstanceHandle, Output, PluginEngine, PluginError, spawn_instance};

const HELLO_PACKAGE: &str = "ulanzi-plugin-hello";
const HELLO_MANIFEST: &str = include_str!("../../../examples/plugins/hello/plugin.toml");
const PROBE_PACKAGE: &str = "ulanzi-plugin-probe";
const PROBE_MANIFEST: &str = include_str!("plugins/probe/plugin.toml");
/// Generous bound for anything that should happen "promptly"; tests never sleep unbounded.
const PROMPTLY: Duration = Duration::from_secs(5);

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

/// A running instance plus everything that must outlive it.
struct Running {
    handle: InstanceHandle,
    out: mpsc::UnboundedReceiver<Output>,
    _engine: PluginEngine,
    _dir: tempfile::TempDir,
    _cache: tempfile::TempDir,
}

fn start(package: &str, manifest: &str, secrets: BTreeMap<String, String>) -> Running {
    let cache = tempfile::tempdir().expect("cache");
    let dir = common::plugins::fixture(package, manifest);
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

fn probe() -> Running {
    start(PROBE_PACKAGE, PROBE_MANIFEST, BTreeMap::new())
}

impl Running {
    fn send(&self, event: Event) {
        // The queue holds 1024 events and tests send a handful: try_send never blocks the runtime.
        if self.handle.events.try_send(event).is_err() {
            panic!("event queue refused an event");
        }
    }

    /// Queue an action; the receiver resolves with the plugin's reply.
    fn queue(&self, action: &str, params: Value) -> oneshot::Receiver<Result<(), String>> {
        let (reply, rx) = oneshot::channel();
        self.send(Event::Action {
            action: action.into(),
            params,
            reply,
        });
        rx
    }

    async fn act(&self, action: &str, params: Value) -> Result<(), String> {
        let rx = self.queue(action, params);
        tokio::time::timeout(PROMPTLY, rx)
            .await
            .expect("action reply in time")
            .expect("reply sent")
    }

    /// Read outputs until `want` shows up; false when `within` runs out first.
    async fn saw(&mut self, want: &Output, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            match tokio::time::timeout_at(deadline, self.out.recv()).await {
                Ok(Some(o)) if &o == want => return true,
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return false,
            }
        }
    }

    async fn exit(self) -> Exit {
        tokio::time::timeout(PROMPTLY, self.handle.exited)
            .await
            .expect("instance exits in time")
            .expect("exit reported")
    }
}

fn state(key: &str, value: &str) -> Output {
    Output::State {
        key: key.into(),
        value: value.into(),
    }
}

#[tokio::test]
async fn hello_counts() {
    let mut p = start(HELLO_PACKAGE, HELLO_MANIFEST, BTreeMap::new());
    p.send(Event::Init {
        settings: json!({"start": 1}),
    });
    assert_eq!(p.act("count", json!({})).await, Ok(()));
    assert!(p.saw(&state("count", "1"), PROMPTLY).await);
    assert!(p.saw(&state("count", "2"), PROMPTLY).await);
}

#[tokio::test]
async fn action_errors_are_returned() {
    let mut p = start(HELLO_PACKAGE, HELLO_MANIFEST, BTreeMap::new());
    assert_eq!(
        p.act("greet", json!({})).await,
        Err("greet needs a name".to_string())
    );
    // Still running: the next action works.
    assert_eq!(p.act("count", json!({})).await, Ok(()));
    assert!(p.saw(&state("count", "1"), PROMPTLY).await);
}

#[tokio::test]
async fn settings_reach_the_plugin() {
    let mut p = probe();
    p.send(Event::Init {
        settings: json!({"room": "office"}),
    });
    assert_eq!(p.act("settings", json!({})).await, Ok(()));
    assert!(p.saw(&state("settings", r#"{"room":"office"}"#), PROMPTLY).await);
}

#[tokio::test]
async fn timer_fires() {
    let mut p = probe();
    assert_eq!(p.act("timer", json!({"ms": 50})).await, Ok(()));
    assert!(p.saw(&state("timer", "fired"), Duration::from_secs(1)).await);
}

#[tokio::test]
async fn secrets() {
    let secrets = BTreeMap::from([("k".to_string(), "old".to_string())]);
    let mut p = start(PROBE_PACKAGE, PROBE_MANIFEST, secrets);
    assert_eq!(p.act("secret_get", json!({"k": "k"})).await, Ok(()));
    assert!(p.saw(&state("secret", "old"), PROMPTLY).await);
    assert_eq!(p.act("secret_get", json!({"k": "nope"})).await, Ok(()));
    assert!(p.saw(&state("secret", "none"), PROMPTLY).await);
    assert_eq!(p.act("secret_set", json!({"k": "k", "v": "new"})).await, Ok(()));
    let changed = Output::SecretsChanged(BTreeMap::from([("k".into(), "new".into())]));
    assert!(p.saw(&changed, PROMPTLY).await);
}

#[tokio::test]
async fn panic_spin_alloc_crash_cleanly() {
    for action in ["panic", "spin", "alloc"] {
        let p = probe();
        let started = Instant::now();
        assert_eq!(
            p.act(action, json!({})).await,
            Err("Probe stopped working".to_string()),
            "{action}"
        );
        if action == "spin" {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "spin took {:?}",
                started.elapsed()
            );
        }
        let why = match p.exit().await {
            Exit::Crashed(why) => why,
            other => panic!("{action}: {other:?}"),
        };
        assert!(!why.contains('\n'), "one line: {why}");
        if action == "spin" {
            assert_eq!(why, "took longer than 500 ms");
        }
    }
}

#[tokio::test]
async fn actions_queued_behind_a_crash_get_the_error() {
    let p = probe();
    let first = p.queue("panic", json!({}));
    let second = p.queue("state", json!({"k": "a", "v": "b"}));
    for rx in [first, second] {
        let reply = tokio::time::timeout(PROMPTLY, rx).await.expect("reply in time");
        assert_eq!(reply, Ok(Err("Probe stopped working".to_string())));
    }
    assert!(matches!(p.exit().await, Exit::Crashed(_)));
}

#[tokio::test]
async fn stop_ends_the_instance() {
    let p = probe();
    assert_eq!(p.act("state", json!({"k": "a", "v": "b"})).await, Ok(()));
    p.send(Event::Stop);
    assert_eq!(p.exit().await, Exit::Stopped);
}

#[tokio::test]
async fn flood_is_bounded() {
    let mut p = probe();
    let reply = p.queue("flood", json!({}));
    // The test's own runtime stays responsive while the plugin floods the host.
    let tick = Instant::now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        tick.elapsed() < Duration::from_millis(500),
        "runtime stalled for {:?}",
        tick.elapsed()
    );

    let started = Instant::now();
    let reply = tokio::time::timeout(Duration::from_secs(2), reply)
        .await
        .expect("flood finishes or crashes within 2 s")
        .expect("reply sent");
    match reply {
        Ok(()) => assert!(p.saw(&state("flood", "9999"), PROMPTLY).await),
        Err(e) => assert_eq!(e, "Probe stopped working"),
    }
    assert!(started.elapsed() < Duration::from_secs(2));
    // Still responsive after 2 000 pending timers.
    let tick = Instant::now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        tick.elapsed() < Duration::from_millis(500),
        "runtime stalled for {:?}",
        tick.elapsed()
    );
}

#[tokio::test]
async fn refuses_undeclared_network() {
    let cache = tempfile::tempdir().expect("cache");
    let manifest = PROBE_MANIFEST.replace("network = [\"127.0.0.1\"]", "");
    let dir = common::plugins::fixture(PROBE_PACKAGE, &manifest);
    let manifest = Manifest::parse(&manifest).expect("manifest");
    assert!(!manifest.has_network());
    let engine = PluginEngine::new(cache.path()).expect("engine");
    let err = engine
        .load(&manifest, &dir.path().join("plugin.wasm"))
        .err()
        .expect("must fail");
    assert!(matches!(err, PluginError::Permission(_)), "{err:?}");
}
