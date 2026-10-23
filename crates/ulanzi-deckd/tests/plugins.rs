mod common;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
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

    async fn exit(mut self) -> Exit {
        // The handle stops the instance when dropped, so `exited` is awaited by reference.
        tokio::time::timeout(PROMPTLY, &mut self.handle.exited)
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
    let mut reply = p.queue("flood", json!({}));
    let started = Instant::now();
    // While the plugin floods the host, the test's own runtime keeps running 10 ms sleeps on time.
    let reply = loop {
        let tick = Instant::now();
        tokio::select! {
            r = &mut reply => break r.expect("reply sent"),
            () = tokio::time::sleep(Duration::from_millis(10)) => {
                let late = tick.elapsed();
                assert!(late < Duration::from_millis(500), "runtime stalled for {late:?}");
                let total = started.elapsed();
                assert!(total < Duration::from_secs(2), "flood still running after {total:?}");
            }
        }
    };
    match reply {
        Ok(()) => assert!(p.saw(&state("flood", "9999"), PROMPTLY).await),
        Err(e) => assert_eq!(e, "Probe stopped working"),
    }
    // Still responsive with 2 000 pending timers.
    let tick = Instant::now();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        tick.elapsed() < Duration::from_millis(500),
        "runtime stalled for {:?}",
        tick.elapsed()
    );
}

#[tokio::test]
async fn stop_works_with_a_full_queue() {
    let mut p = probe();
    let busy = p.queue("spin", json!({}));
    let queued = p.queue("state", json!({"k": "a", "v": "b"}));
    // Fill the queue behind the spinning call; the timer id is unknown, so these do nothing.
    while p.handle.events.try_send(Event::Timer(u32::MAX)).is_ok() {}
    p.handle.stop();
    let exit = tokio::time::timeout(Duration::from_secs(1), &mut p.handle.exited)
        .await
        .expect("stops within 1 s")
        .expect("exit reported");
    assert_eq!(exit, Exit::Stopped);
    let off = Err("the Probe plugin is turned off".to_string());
    for rx in [busy, queued] {
        let reply = tokio::time::timeout(PROMPTLY, rx).await.expect("reply in time");
        assert_eq!(reply, Ok(off.clone()));
    }
}

#[tokio::test]
async fn dropping_the_handle_ends_the_instance() {
    let Running {
        handle,
        mut out,
        _engine,
        _dir,
        _cache,
    } = probe();
    drop(handle);
    // The thread drops the store, and with it the output sender, when it ends.
    let closed = tokio::time::timeout(Duration::from_secs(1), async {
        while out.recv().await.is_some() {}
    })
    .await;
    assert!(closed.is_ok(), "instance still running");
}

#[tokio::test]
async fn oversized_state_is_ignored() {
    let mut p = probe();
    let big = "x".repeat(64 * 1024 + 1);
    assert_eq!(p.act("state", json!({"k": "big", "v": big})).await, Ok(()));
    assert_eq!(p.act("state", json!({"k": "after", "v": "1"})).await, Ok(()));
    loop {
        let o = tokio::time::timeout(PROMPTLY, p.out.recv())
            .await
            .expect("output in time")
            .expect("output");
        match o {
            Output::State { key, .. } if key == "big" => panic!("oversized state was sent"),
            o if o == state("after", "1") => break,
            _ => {}
        }
    }
}

#[test]
fn secrets_changed_debug_hides_values() {
    let o = Output::SecretsChanged(BTreeMap::from([("token".into(), "hunter2".into())]));
    let shown = format!("{o:?}");
    assert!(shown.contains("token") && !shown.contains("hunter2"), "{shown}");
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

// ---- Network ----

/// A local WebSocket server; each connection runs `serve`. Returns the `ws://` URL.
async fn ws_server<F, Fut>(serve: F) -> String
where
    F: Fn(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            if let Ok(ws) = tokio_tungstenite::accept_async(tcp).await {
                tokio::spawn(serve(ws));
            }
        }
    });
    format!("ws://127.0.0.1:{port}")
}

/// A local HTTP/1.1 server that answers every request with `response` (raw bytes) and closes.
async fn http_server(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let response = response.clone();
            tokio::spawn(async move {
                let mut seen = Vec::new();
                let mut buf = [0u8; 4096];
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tcp.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => seen.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = tcp.write_all(&response).await;
                let _ = tcp.shutdown().await;
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Read outputs until a state for `key` shows up; its value, or None when `within` runs out.
async fn state_of(p: &mut Running, key: &str, within: Duration) -> Option<String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, p.out.recv()).await {
            Ok(Some(Output::State { key: k, value })) if k == key => return Some(value),
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return None,
        }
    }
}

async fn echo(mut ws: WebSocketStream<TcpStream>) {
    while let Some(Ok(m)) = ws.next().await {
        if (m.is_text() || m.is_binary()) && ws.send(m).await.is_err() {
            return;
        }
    }
}

#[tokio::test]
async fn ws_round_trip() {
    let url = ws_server(echo).await;
    let mut p = probe();
    assert_eq!(p.act("ws", json!({"url": url})).await, Ok(()));
    assert!(p.saw(&state("ws", "ping"), PROMPTLY).await);
}

#[tokio::test]
async fn ws_close_reported() {
    let url = ws_server(|mut ws| async move {
        // Wait for the probe's "ping", then close with a reason.
        let _ = ws.next().await;
        let _ = ws
            .close(Some(CloseFrame {
                code: CloseCode::Normal,
                reason: "bye".into(),
            }))
            .await;
        while ws.next().await.is_some() {}
    })
    .await;
    let mut p = probe();
    assert_eq!(p.act("ws", json!({"url": url})).await, Ok(()));
    assert!(p.saw(&state("ws_closed", "bye"), PROMPTLY).await);
}

#[tokio::test]
async fn closed_sockets_refuse_sends() {
    let url = ws_server(echo).await;
    let mut p = probe();
    assert_eq!(p.act("ws", json!({"url": url})).await, Ok(()));
    assert!(p.saw(&state("ws", "ping"), PROMPTLY).await);
    // `close` closes twice (harmless), then a send must fail.
    assert_eq!(p.act("close", json!({})).await, Ok(()));
    let sent = state_of(&mut p, "send", PROMPTLY).await.expect("send state");
    assert_ne!(sent, "ok");
    // The plugin closed it itself: no `on-closed` for that.
    assert_eq!(
        state_of(&mut p, "ws_closed", Duration::from_millis(300)).await,
        None
    );
}

#[tokio::test]
async fn wss_handshake_failure_is_reported() {
    // A plain TCP server that answers the TLS hello with garbage: the TLS path runs and fails.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let _ = tcp.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        }
    });
    let mut p = probe();
    let url = format!("wss://127.0.0.1:{port}");
    assert_eq!(p.act("ws", json!({"url": url})).await, Ok(()));
    let reason = state_of(&mut p, "ws_closed", PROMPTLY).await.expect("on-closed");
    assert!(reason.starts_with("cannot open"), "{reason}");
}

#[tokio::test]
async fn tcp_round_trip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        if let Ok((mut tcp, _)) = listener.accept().await {
            let mut buf = [0u8; 64];
            while let Ok(n) = tcp.read(&mut buf).await {
                if n == 0 || tcp.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }
    });
    let mut p = probe();
    assert_eq!(
        p.act("tcp", json!({"host": "127.0.0.1", "port": port})).await,
        Ok(())
    );
    assert!(p.saw(&state("bin", "ping"), PROMPTLY).await);
}

#[tokio::test]
async fn fetch_status() {
    let url = http_server(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec()).await;
    let mut p = probe();
    assert_eq!(p.act("fetch", json!({"url": url})).await, Ok(()));
    assert!(p.saw(&state("http", "204"), PROMPTLY).await);
}

#[tokio::test]
async fn fetch_does_not_follow_redirects() {
    let url = http_server(
        b"HTTP/1.1 302 Found\r\nLocation: http://example.com/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec(),
    )
    .await;
    let mut p = probe();
    assert_eq!(p.act("fetch", json!({"url": url})).await, Ok(()));
    assert!(p.saw(&state("http", "302"), PROMPTLY).await);
}

#[tokio::test]
async fn fetch_caps_the_body() {
    let size = 8 * 1024 * 1024 + 1;
    let mut response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n").into_bytes();
    response.resize(response.len() + size, b'x');
    let url = http_server(response).await;
    let mut p = probe();
    assert_eq!(p.act("fetch", json!({"url": url})).await, Ok(()));
    assert!(p.saw(&state("http", "error: response too large"), PROMPTLY).await);
}

#[tokio::test]
async fn refuses_hosts_outside_permissions() {
    let mut p = probe();
    let refused = Err("not allowed: example.com".to_string());
    assert_eq!(p.act("ws", json!({"url": "ws://example.com"})).await, refused);
    assert_eq!(
        p.act("fetch", json!({"url": "https://Example.COM/x"})).await,
        Err("not allowed: example.com".to_string())
    );
    assert_eq!(
        p.act("tcp", json!({"host": "example.com", "port": 80})).await,
        refused
    );
    // Other schemes are refused too.
    assert!(p.act("ws", json!({"url": "http://127.0.0.1/"})).await.is_err());
    assert!(p.act("fetch", json!({"url": "ftp://127.0.0.1/"})).await.is_err());
    assert_eq!(state_of(&mut p, "ws", Duration::from_millis(300)).await, None);
}

#[tokio::test]
async fn sockets_close_when_instance_stops() {
    let (opened_tx, opened) = oneshot::channel::<()>();
    let (closed_tx, closed) = oneshot::channel::<()>();
    let slots = std::sync::Arc::new(std::sync::Mutex::new(Some((opened_tx, closed_tx))));
    let url = ws_server(move |mut ws| {
        let slot = slots.lock().ok().and_then(|mut s| s.take());
        async move {
            let Some((opened_tx, closed_tx)) = slot else {
                return;
            };
            // The probe's "ping" shows the socket is open.
            let _ = ws.next().await;
            let _ = opened_tx.send(());
            loop {
                match ws.next().await {
                    Some(Ok(m)) if !m.is_close() => {}
                    _ => break,
                }
            }
            let _ = closed_tx.send(());
        }
    })
    .await;
    let p = probe();
    assert_eq!(p.act("ws", json!({"url": url})).await, Ok(()));
    tokio::time::timeout(PROMPTLY, opened)
        .await
        .expect("socket opens")
        .expect("server saw ping");
    p.send(Event::Stop);
    tokio::time::timeout(Duration::from_secs(1), closed)
        .await
        .expect("server sees the close within 1 s")
        .expect("close reported");
    assert_eq!(p.exit().await, Exit::Stopped);
}

#[tokio::test]
async fn closing_sockets_end_with_the_instance() {
    const BLAST: usize = 7680 * 1024;
    // A peer with a tiny receive buffer that stops reading after the probe's "ping".
    let socket = tokio::net::TcpSocket::new_v4().expect("socket");
    socket.set_recv_buffer_size(4096).expect("rcvbuf");
    socket.bind("127.0.0.1:0".parse().expect("addr")).expect("bind");
    let port = socket.local_addr().expect("addr").port();
    let listener = socket.listen(1).expect("listen");
    let (opened_tx, opened) = oneshot::channel::<()>();
    let (go_tx, go) = oneshot::channel::<()>();
    let (seen_tx, seen) = oneshot::channel::<(usize, std::io::Result<()>, Duration)>();
    tokio::spawn(async move {
        let Ok((mut tcp, _)) = listener.accept().await else {
            return;
        };
        let mut ping = [0u8; 4];
        if tcp.read_exact(&mut ping).await.is_err() {
            return;
        }
        let _ = opened_tx.send(());
        let _ = go.await;
        // The instance is gone: poke the connection, then read until it ends.
        let started = Instant::now();
        let _ = tcp.write_all(b"x").await;
        let mut total = 0;
        let mut buf = vec![0u8; 64 * 1024];
        let end = loop {
            match tcp.read(&mut buf).await {
                Ok(0) => break Ok(()),
                Ok(n) => total += n,
                Err(e) => break Err(e),
            }
        };
        let _ = seen_tx.send((total, end, started.elapsed()));
    });
    let mut p = probe();
    assert_eq!(
        p.act("tcp", json!({"host": "127.0.0.1", "port": port})).await,
        Ok(())
    );
    tokio::time::timeout(PROMPTLY, opened)
        .await
        .expect("socket opens")
        .expect("peer saw ping");
    // Queue far more than the peer takes, then close: the socket is closing, not gone.
    assert_eq!(p.act("blast", json!({"kib": BLAST / 1024})).await, Ok(()));
    p.handle.stop();
    tokio::time::timeout(PROMPTLY, &mut p.handle.exited)
        .await
        .expect("instance exits")
        .expect("exit reported");
    let _ = go_tx.send(());
    let (total, end, took) = tokio::time::timeout(Duration::from_secs(1), seen)
        .await
        .expect("peer sees the connection end within 1 s")
        .expect("peer reported");
    // A socket left running would deliver everything and then close cleanly.
    assert!(
        end.is_err() || total < BLAST,
        "socket kept sending after the instance stopped: {total} B, {end:?}, {took:?}"
    );
}

#[tokio::test]
async fn queued_network_data_is_bounded() {
    const BUDGET: usize = 16 << 20;
    const PIECE: usize = 512 * 1024;
    const PIECES: usize = 100;
    let (opened_tx, opened) = oneshot::channel::<()>();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(opened_tx)));
    // After the probe's "ping", flood 50 MiB, then say "done".
    let url = ws_server(move |mut ws| {
        let opened_tx = slot.lock().ok().and_then(|mut s| s.take());
        async move {
            let _ = ws.next().await;
            if let Some(tx) = opened_tx {
                let _ = tx.send(());
            }
            for _ in 0..PIECES {
                if ws.send(Message::binary(vec![b'x'; PIECE])).await.is_err() {
                    return;
                }
            }
            let _ = ws.send(Message::text("done")).await;
            while ws.next().await.is_some() {}
        }
    })
    .await;
    let mut p = probe();
    assert_eq!(p.act("ws", json!({"url": url})).await, Ok(()));
    tokio::time::timeout(PROMPTLY, opened)
        .await
        .expect("socket opens")
        .expect("server saw ping");
    // Keep the plugin busy while the flood arrives.
    let busy: Vec<_> = (0..3).map(|_| p.queue("sleep", json!({"ms": 300}))).collect();
    let mut most = 0;
    let deadline = tokio::time::Instant::now() + PROMPTLY;
    loop {
        most = most.max(p.handle.net_queued());
        tokio::select! {
            o = p.out.recv() => match o {
                Some(o) if o == state("ws", "done") => break,
                Some(_) => {}
                None => panic!("instance ended"),
            },
            () = tokio::time::sleep(Duration::from_millis(2)) => {}
            () = tokio::time::sleep_until(deadline) => panic!("flood not delivered in time"),
        }
    }
    for rx in busy {
        let reply = tokio::time::timeout(PROMPTLY, rx).await.expect("reply in time");
        assert_eq!(reply, Ok(Ok(())));
    }
    assert!(most <= BUDGET, "{most} B of network data queued");
    // The budget was actually reached: the socket waited instead of reading on.
    assert!(
        most > BUDGET / 2,
        "only {most} B queued; the flood never backed up"
    );
    assert_eq!(p.handle.net_queued(), 0);
}
