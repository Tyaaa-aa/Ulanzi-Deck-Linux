mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::plugins::{PROMPTLY, Running, start, state_of};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use ulanzi_core::model::{PluginEntry, Settings};
use ulanzi_core::plugin::Manifest;
use ulanzi_deckd::deck::DeckMsg;
use ulanzi_deckd::plugins::{
    Event, Exit, FileSecrets, LazySecrets, Output, PluginEngine, PluginError, PluginHost, PluginSlot,
    PluginStatus, SecretStore,
};

const HELLO_PACKAGE: &str = "ulanzi-plugin-hello";
const HELLO_MANIFEST: &str = include_str!("../../../examples/plugins/hello/plugin.toml");
const PROBE_PACKAGE: &str = "ulanzi-plugin-probe";
const PROBE_MANIFEST: &str = include_str!("plugins/probe/plugin.toml");

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
async fn unwritable_cache_still_loads() {
    // The cache "folder" sits under a regular file, so it can never be created.
    let file = tempfile::NamedTempFile::new().expect("file");
    let dir = common::plugins::fixture(HELLO_PACKAGE, HELLO_MANIFEST);
    let engine = PluginEngine::new(&file.path().join("cache")).expect("engine without a cache");
    engine
        .load(&hello_manifest(), &dir.path().join("plugin.wasm"))
        .expect("load hello");
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

fn probe() -> Running {
    start(PROBE_PACKAGE, PROBE_MANIFEST, BTreeMap::new())
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
async fn state_keys_are_capped() {
    let mut p = probe();
    assert_eq!(p.act("keys", json!({"n": 1000})).await, Ok(()));
    let mut keys = BTreeSet::new();
    let mut updated = false;
    // Everything the action produced, until a short quiet spell.
    while let Ok(Some(o)) = tokio::time::timeout(Duration::from_millis(200), p.out.recv()).await {
        if let Output::State { key, value } = o {
            updated |= key == "key0" && value == "updated";
            keys.insert(key);
        }
    }
    assert_eq!(keys.len(), 256, "distinct keys are capped");
    assert!(keys.contains("key255") && !keys.contains("key256"));
    assert!(updated, "a known key still updates at the limit");
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
async fn a_short_sleep_is_fine() {
    let p = probe();
    assert_eq!(p.act("sleep", json!({"ms": 100})).await, Ok(()));
    assert_eq!(p.act("state", json!({"k": "a", "v": "b"})).await, Ok(()));
}

#[tokio::test]
async fn a_long_sleep_is_over_budget() {
    // A blocking WASI wait (std's sleep) counts against the 500 ms call budget.
    let p = probe();
    let started = Instant::now();
    assert_eq!(
        p.act("sleep", json!({"ms": 2000})).await,
        Err("Probe stopped working".to_string())
    );
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(p.exit().await, Exit::Crashed("took longer than 500 ms".into()));
}

#[tokio::test]
async fn waiting_counts_toward_the_budget_of_busy_work() {
    // 490 ms asleep, then a busy loop: the call ends at about 550 ms in all. Before waits counted,
    // the loop got 45 more ticks of at least 10 ms each, so it could not end before 940 ms.
    let p = probe();
    let started = Instant::now();
    assert_eq!(
        p.act("sleep_spin", json!({"ms": 490})).await,
        Err("Probe stopped working".to_string())
    );
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(p.exit().await, Exit::Crashed("took longer than 500 ms".into()));
}

#[tokio::test]
async fn stop_cuts_a_sleep_short() {
    let mut p = probe();
    let busy = p.queue("sleep", json!({"ms": 10_000}));
    // Let the call start sleeping.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    p.handle.stop();
    let exit = tokio::time::timeout(Duration::from_secs(1), &mut p.handle.exited)
        .await
        .expect("stops within 1 s")
        .expect("exit reported");
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(exit, Exit::Stopped);
    let reply = tokio::time::timeout(PROMPTLY, busy).await.expect("reply in time");
    assert_eq!(reply, Ok(Err("the Probe plugin is turned off".to_string())));
}

#[tokio::test]
async fn status_is_capped_and_not_repeated() {
    let mut p = probe();
    let long = "é".repeat(3000); // 6000 bytes
    assert_eq!(p.act("status", json!({"message": long, "n": 3})).await, Ok(()));
    let mut seen = Vec::new();
    while let Ok(Some(o)) = tokio::time::timeout(Duration::from_millis(200), p.out.recv()).await {
        if let Output::Status { message, .. } = o {
            seen.push(message);
        }
    }
    assert_eq!(seen.len(), 1, "a repeated status is reported once");
    assert_eq!(seen[0].len(), 4096);
    assert!(long.starts_with(&seen[0]));
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
    // Values are label text: 1 KiB at most.
    let big = "x".repeat(1024 + 1);
    let most = "x".repeat(1024);
    assert_eq!(p.act("state", json!({"k": "big", "v": big})).await, Ok(()));
    assert_eq!(p.act("state", json!({"k": "most", "v": most})).await, Ok(()));
    assert_eq!(p.act("state", json!({"k": "after", "v": "1"})).await, Ok(()));
    let mut saw_most = false;
    loop {
        let o = tokio::time::timeout(PROMPTLY, p.out.recv())
            .await
            .expect("output in time")
            .expect("output");
        match o {
            Output::State { key, .. } if key == "big" => panic!("oversized state was sent"),
            Output::State { key, .. } if key == "most" => saw_most = true,
            o if o == state("after", "1") => break,
            _ => {}
        }
    }
    assert!(saw_most, "a 1 KiB value is fine");
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

/// Payload `i` of the probe's `burst` action (kept in step with the probe).
fn burst_payload(i: usize) -> String {
    format!("{i:04}{}", ".".repeat(4092))
}

#[tokio::test]
async fn data_sent_before_close_arrives() {
    const N: usize = 16;
    const ROUNDS: usize = 50;
    let expected: Vec<String> = (0..N).map(burst_payload).collect();
    // What each peer received, in order, once the probe's close reached it.
    let (got_tx, mut got) = mpsc::unbounded_channel::<Vec<String>>();
    let (opened_tx, mut opened) = mpsc::unbounded_channel::<()>();

    let ws_url = ws_server({
        let (got_tx, opened_tx) = (got_tx.clone(), opened_tx.clone());
        move |mut ws| {
            let (got_tx, opened_tx) = (got_tx.clone(), opened_tx.clone());
            async move {
                let _ = ws.next().await; // "ping"
                let _ = opened_tx.send(());
                let mut seen = Vec::new();
                while let Some(Ok(m)) = ws.next().await {
                    match m {
                        Message::Text(t) => seen.push(t.as_str().to_owned()),
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                let _ = got_tx.send(seen);
            }
        }
    })
    .await;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let tcp_port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let (got_tx, opened_tx) = (got_tx.clone(), opened_tx.clone());
            tokio::spawn(async move {
                let mut ping = [0u8; 4];
                if tcp.read_exact(&mut ping).await.is_err() {
                    return;
                }
                let _ = opened_tx.send(());
                let mut all = Vec::new();
                let _ = tcp.read_to_end(&mut all).await;
                let text = String::from_utf8_lossy(&all);
                let pieces = text
                    .as_bytes()
                    .chunks(4096)
                    .map(|c| String::from_utf8_lossy(c).into_owned())
                    .collect();
                let _ = got_tx.send(pieces);
            });
        }
    });

    let p = probe();
    let started = Instant::now();
    for round in 0..ROUNDS {
        let ws = round % 2 == 0;
        let open = match ws {
            true => json!({"url": ws_url}),
            false => json!({"host": "127.0.0.1", "port": tcp_port}),
        };
        assert_eq!(p.act(if ws { "ws" } else { "tcp" }, open).await, Ok(()));
        tokio::time::timeout(PROMPTLY, opened.recv())
            .await
            .expect("socket opens")
            .expect("peer");
        assert_eq!(p.act("burst", json!({"n": N})).await, Ok(()));
        let seen = tokio::time::timeout(PROMPTLY, got.recv())
            .await
            .expect("peer sees the close")
            .expect("peer");
        let kind = if ws { "ws" } else { "tcp" };
        assert_eq!(
            seen.len(),
            N,
            "round {round} ({kind}): {} payloads arrived",
            seen.len()
        );
        assert!(
            seen == expected,
            "round {round} ({kind}): payloads out of order or cut"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "took {:?}",
        started.elapsed()
    );
}

// ---- Plugin host: secrets, supervisor, install ----

/// Secrets kept in memory, for tests that must not touch the system keyring.
#[derive(Default)]
struct MemSecrets(std::sync::Mutex<BTreeMap<String, BTreeMap<String, String>>>);

#[async_trait::async_trait]
impl SecretStore for MemSecrets {
    async fn load(&self, plugin: &str) -> Result<BTreeMap<String, String>, String> {
        let all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Ok(all.get(plugin).cloned().unwrap_or_default())
    }
    async fn save(&self, plugin: &str, secrets: &BTreeMap<String, String>) -> Result<(), String> {
        let mut all = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Like the real stores: saving nothing forgets the plugin.
        if secrets.is_empty() {
            all.remove(plugin);
        } else {
            all.insert(plugin.to_string(), secrets.clone());
        }
        Ok(())
    }
}

/// Put a plugin folder `<dir>/<folder>/` with `manifest` and `package`'s wasm (`None`: no wasm).
fn plugin_folder(dir: &std::path::Path, folder: &str, manifest: &str, package: Option<&str>) {
    let to = dir.join(folder);
    std::fs::create_dir_all(&to).expect("mkdir");
    std::fs::write(to.join("plugin.toml"), manifest).expect("write manifest");
    if let Some(package) = package {
        std::fs::copy(common::plugins::wasm(package), to.join("plugin.wasm")).expect("copy wasm");
    }
}

/// A host over a temporary plugins dir, with its deck channel and the temp dirs it needs.
struct Host {
    host: PluginHost,
    deck: mpsc::Receiver<DeckMsg>,
    dir: tempfile::TempDir,
    _cache: tempfile::TempDir,
}

async fn host_with(
    setup: impl FnOnce(&std::path::Path),
    settings: &Settings,
    used: &[&str],
    secrets: Arc<dyn SecretStore>,
) -> Host {
    let dir = tempfile::tempdir().expect("dir");
    let cache = tempfile::tempdir().expect("cache");
    setup(dir.path());
    let (tx, deck) = mpsc::channel(4096);
    let used: BTreeSet<String> = used.iter().map(|s| s.to_string()).collect();
    let host = PluginHost::start(
        dir.path().to_path_buf(),
        cache.path().to_path_buf(),
        secrets,
        settings,
        &used,
        tx,
    )
    .await;
    Host {
        host,
        deck,
        dir,
        _cache: cache,
    }
}

fn hello_folder(dir: &std::path::Path) {
    plugin_folder(dir, "hello", HELLO_MANIFEST, Some(HELLO_PACKAGE));
}

impl Host {
    fn status(&self, id: &str) -> Option<PluginStatus> {
        self.host
            .list()
            .into_iter()
            .find(|p| p.id == id)
            .map(|p| p.status)
    }

    /// Wait until `id`'s status satisfies `want`; panics after `PROMPTLY`.
    async fn wait_status(&self, id: &str, want: impl Fn(&PluginStatus) -> bool) -> PluginStatus {
        let mut rx = self.host.watch();
        let deadline = tokio::time::Instant::now() + PROMPTLY;
        loop {
            rx.borrow_and_update();
            if let Some(s) = self.status(id).filter(|s| want(s)) {
                return s;
            }
            if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
                panic!("{id} never reached the wanted status; now {:?}", self.status(id));
            }
        }
    }

    /// Read deck messages until `want` matches one; false when `PROMPTLY` runs out.
    async fn deck_saw(&mut self, want: impl Fn(&DeckMsg) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + PROMPTLY;
        loop {
            match tokio::time::timeout_at(deadline, self.deck.recv()).await {
                Ok(Some(m)) if want(&m) => return true,
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return false,
            }
        }
    }

    async fn saw_state(&mut self, plugin: &str, key: &str, value: &str) -> bool {
        self.deck_saw(|m| {
            matches!(m, DeckMsg::PluginState { plugin: p, key: k, value: v }
                if p == plugin && k == key && v == value)
        })
        .await
    }
}

fn mem_secrets() -> Arc<dyn SecretStore> {
    Arc::new(MemSecrets::default())
}

#[tokio::test]
async fn file_secrets_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("plugin-secrets.toml");
    let store = FileSecrets::new(path.clone());
    assert_eq!(store.load("one").await, Ok(BTreeMap::new()));
    let one = BTreeMap::from([("token".to_string(), "s3cret".to_string())]);
    let two = BTreeMap::from([("key".to_string(), "other".to_string())]);
    store.save("one", &one).await.expect("save one");
    store.save("two", &two).await.expect("save two");
    assert_eq!(store.load("one").await, Ok(one.clone()));
    assert_eq!(store.load("two").await, Ok(two));
    let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    // A second store over the same file sees the same data.
    assert_eq!(FileSecrets::new(path).load("one").await, Ok(one));
    // Saving nothing forgets the plugin.
    store.save("one", &BTreeMap::new()).await.expect("clear");
    assert_eq!(store.load("one").await, Ok(BTreeMap::new()));
}

#[tokio::test]
async fn broken_folders_do_not_stop_others() {
    let setup = |dir: &std::path::Path| {
        hello_folder(dir);
        plugin_folder(dir, "bad_toml", "id = \"bad_toml\"\nname = ", Some(HELLO_PACKAGE));
        let manifest = |id: &str, name: &str, contract: &str| {
            format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\ncontract = \"{contract}\"\n")
        };
        plugin_folder(dir, "no_wasm", &manifest("no_wasm", "No wasm", "1.0"), None);
        plugin_folder(dir, "garbage", &manifest("garbage", "Garbage", "1.0"), None);
        std::fs::write(dir.join("garbage/plugin.wasm"), b"").expect("garbage wasm");
        // A future contract may come with a manifest shape this host does not know.
        let future = format!("{}[future_table]\nx = 1\n", manifest("future", "Future", "2.0"));
        plugin_folder(dir, "future", &future, Some(HELLO_PACKAGE));
        plugin_folder(
            dir,
            "mismatch",
            &manifest("hello2", "Hello 2", "1.0"),
            Some(HELLO_PACKAGE),
        );
        // Leftovers of an interrupted install are not plugins.
        plugin_folder(dir, "hello.new", HELLO_MANIFEST, Some(HELLO_PACKAGE));
        plugin_folder(dir, ".hidden", HELLO_MANIFEST, Some(HELLO_PACKAGE));
    };
    let h = host_with(setup, &Settings::default(), &["hello"], mem_secrets()).await;
    h.wait_status("hello", |s| *s == PluginStatus::Running).await;
    let list = h.host.list();
    let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["bad_toml", "future", "garbage", "hello", "mismatch", "no_wasm"]
    );
    for p in &list {
        if p.id == "hello" {
            assert_eq!(p.name, "Hello");
            assert_eq!(p.version, "1.0.0");
            continue;
        }
        assert!(
            matches!(&p.status, PluginStatus::Incompatible(why) if !why.is_empty()),
            "{p:?}"
        );
        assert_eq!(p.status.id(), "incompatible");
        assert!(!p.status.detail().is_empty());
    }
    let future = list.iter().find(|p| p.id == "future").expect("future");
    assert_eq!(future.status.detail(), "Future needs a newer Ulanzi Deck");
}

#[tokio::test]
async fn action_messages() {
    let mut settings = Settings::default();
    settings.plugins.insert(
        "hello".into(),
        PluginEntry {
            enabled: false,
            ..PluginEntry::default()
        },
    );
    let h = host_with(hello_folder, &settings, &["hello"], mem_secrets()).await;
    let err = h
        .host
        .action("nope", "count", json!({}))
        .await
        .expect_err("unknown");
    assert_eq!(err.to_string(), "the nope plugin is not installed");
    assert_eq!(h.status("hello"), Some(PluginStatus::Off));
    let err = h.host.action("hello", "count", json!({})).await.expect_err("off");
    assert_eq!(err.to_string(), "the Hello plugin is turned off");
    // The plugin's own refusal comes back as its message.
    h.host.apply_settings(&Settings::default()).await;
    let err = h
        .host
        .action("hello", "greet", json!({}))
        .await
        .expect_err("refused");
    assert_eq!(err.to_string(), "greet needs a name");
}

#[tokio::test]
async fn lazy_start() {
    let mut h = host_with(hello_folder, &Settings::default(), &[], mem_secrets()).await;
    assert_eq!(h.status("hello"), Some(PluginStatus::Ready));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(h.deck.try_recv().is_err(), "nothing runs before the first action");
    assert_eq!(h.host.action("hello", "count", json!({})).await, Ok(()));
    assert_eq!(h.status("hello"), Some(PluginStatus::Running));
    assert!(h.saw_state("hello", "count", "1").await);
}

#[tokio::test]
async fn crash_policy() {
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let mut h = host_with(setup, &Settings::default(), &["probe"], mem_secrets()).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    let cleared = |m: &DeckMsg| matches!(m, DeckMsg::PluginCleared(id) if id == "probe");

    let err = h
        .host
        .action("probe", "panic", json!({}))
        .await
        .expect_err("panics");
    assert_eq!(err.to_string(), "Probe stopped working");
    assert!(h.deck_saw(cleared).await, "a crash clears the plugin's state");
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    let kv = json!({"k": "a", "v": "b"});
    assert_eq!(h.host.action("probe", "state", kv.clone()).await, Ok(()));

    let err = h
        .host
        .action("probe", "panic", json!({}))
        .await
        .expect_err("panics");
    assert_eq!(err.to_string(), "Probe stopped working");
    assert!(h.deck_saw(cleared).await);
    let failed = h
        .wait_status("probe", |s| matches!(s, PluginStatus::Failed(_)))
        .await;
    assert_eq!(failed.id(), "failed");
    assert!(!failed.detail().is_empty());
    let err = h
        .host
        .action("probe", "state", kv.clone())
        .await
        .expect_err("failed");
    assert_eq!(err.to_string(), "Probe stopped working");

    assert_eq!(h.host.restart("probe").await, Ok(()));
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    assert_eq!(h.host.action("probe", "state", kv).await, Ok(()));
    assert!(h.saw_state("probe", "a", "b").await);
    h.host.shutdown().await;
}

#[tokio::test]
async fn a_full_queue_is_busy_not_broken() {
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let h = host_with(setup, &Settings::default(), &["probe"], mem_secrets()).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    // Hold the plugin, then queue more actions than the queue takes.
    let held = h.host.action("probe", "sleep", json!({"ms": 300}));
    let kv = json!({"k": "a", "v": "b"});
    let more = (0..1100).map(|_| h.host.action("probe", "state", kv.clone()));
    let (held, more) = tokio::join!(held, futures_util::future::join_all(more));
    assert_eq!(held, Ok(()));
    let busy = more
        .iter()
        .filter(|r| **r == Err(PluginError::Action("Probe is busy".into())))
        .count();
    let ok = more.iter().filter(|r| r.is_ok()).count();
    assert!(busy > 0, "some actions found the queue full");
    assert_eq!(busy + ok, more.len(), "{more:?}");
    assert_eq!(h.status("probe"), Some(PluginStatus::Running));
    h.host.shutdown().await;
}

#[tokio::test]
async fn reinstall_keeps_secrets() {
    let mut h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let src = common::plugins::fixture(PROBE_PACKAGE, PROBE_MANIFEST);
    assert_eq!(
        h.host.install_dir(src.path(), true).await,
        Ok("probe".to_string())
    );
    assert!(h.dir.path().join("probe/plugin.wasm").is_file());
    let kv = json!({"k": "k", "v": "v"});
    assert_eq!(h.host.action("probe", "secret_set", kv).await, Ok(()));
    assert_eq!(
        h.host.install_dir(src.path(), true).await,
        Ok("probe".to_string())
    );
    assert!(
        h.deck_saw(|m| matches!(m, DeckMsg::PluginCleared(id) if id == "probe"))
            .await
    );
    assert_eq!(
        h.host.action("probe", "secret_get", json!({"k": "k"})).await,
        Ok(())
    );
    assert!(h.saw_state("probe", "secret", "v").await);
    let left: Vec<String> = std::fs::read_dir(h.dir.path())
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["probe"], "no leftovers of the replaced install");

    assert_eq!(h.host.remove("probe").await, Ok(()));
    assert!(!h.dir.path().join("probe").exists());
    assert!(h.status("probe").is_none());
    let err = h
        .host
        .action("probe", "state", json!({}))
        .await
        .expect_err("gone");
    assert_eq!(err.to_string(), "the probe plugin is not installed");
}

#[tokio::test]
async fn install_needs_developer_mode() {
    let h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let src = common::plugins::fixture(HELLO_PACKAGE, HELLO_MANIFEST);
    let err = h.host.install_dir(src.path(), false).await.expect_err("refused");
    assert_eq!(
        err.to_string(),
        "Turn on Developer mode to install plugins from a folder"
    );
    assert!(h.host.list().is_empty());
    // A broken folder is refused before anything is installed.
    let bad = tempfile::tempdir().expect("bad");
    std::fs::write(bad.path().join("plugin.toml"), HELLO_MANIFEST).expect("manifest");
    std::fs::write(bad.path().join("plugin.wasm"), b"").expect("wasm");
    assert!(h.host.install_dir(bad.path(), true).await.is_err());
    assert_eq!(std::fs::read_dir(h.dir.path()).expect("dir").count(), 0);
}

#[tokio::test]
async fn state_reaches_the_deck() {
    let mut h = host_with(hello_folder, &Settings::default(), &["hello"], mem_secrets()).await;
    assert_eq!(h.host.action("hello", "count", json!({})).await, Ok(()));
    assert!(h.saw_state("hello", "count", "1").await);
}

#[tokio::test]
async fn changed_settings_reach_a_running_plugin() {
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let mut h = host_with(setup, &Settings::default(), &["probe"], mem_secrets()).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    let mut settings = Settings::default();
    let mut table = toml::Table::new();
    table.insert("room".into(), toml::Value::String("office".into()));
    settings.plugins.insert(
        "probe".into(),
        PluginEntry {
            enabled: true,
            settings: table,
            ..PluginEntry::default()
        },
    );
    h.host.apply_settings(&settings).await;
    assert_eq!(h.host.action("probe", "settings", json!({})).await, Ok(()));
    assert!(h.saw_state("probe", "settings", r#"{"room":"office"}"#).await);
    // Turned off: stopped, and its state cleared from the deck.
    settings.plugins.get_mut("probe").expect("entry").enabled = false;
    h.host.apply_settings(&settings).await;
    assert_eq!(h.status("probe"), Some(PluginStatus::Off));
    assert!(
        h.deck_saw(|m| matches!(m, DeckMsg::PluginCleared(id) if id == "probe"))
            .await
    );
}

#[tokio::test]
async fn remove_wins_over_a_racing_action() {
    let secrets = Arc::new(MemSecrets::default());
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let store: Arc<dyn SecretStore> = secrets.clone();
    let h = host_with(setup, &Settings::default(), &[], store).await;
    let kv = json!({"k": "k", "v": "v"});
    assert_eq!(h.host.action("probe", "secret_set", kv.clone()).await, Ok(()));
    let (removed, raced) = tokio::join!(h.host.remove("probe"), async {
        // `remove` runs first and gets as far as stopping the instance.
        tokio::task::yield_now().await;
        h.host.action("probe", "secret_set", kv.clone()).await
    });
    assert_eq!(removed, Ok(()));
    assert_eq!(
        raced.expect_err("removed").to_string(),
        "the probe plugin is not installed"
    );
    assert!(
        secrets.0.lock().expect("lock").get("probe").is_none(),
        "secrets deleted"
    );
    assert!(h.host.list().is_empty());
}

#[tokio::test]
async fn install_refuses_symlinks() {
    let h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    // plugin.wasm is a link.
    let src = common::plugins::fixture(HELLO_PACKAGE, HELLO_MANIFEST);
    let real = src.path().join("real.wasm");
    std::fs::rename(src.path().join("plugin.wasm"), &real).expect("move");
    std::os::unix::fs::symlink(&real, src.path().join("plugin.wasm")).expect("link");
    assert!(h.host.install_dir(src.path(), true).await.is_err());
    // A link inside assets/.
    let src = common::plugins::fixture(HELLO_PACKAGE, HELLO_MANIFEST);
    std::fs::create_dir(src.path().join("assets")).expect("assets");
    std::fs::write(src.path().join("assets/icon.svg"), "<svg/>").expect("icon");
    std::os::unix::fs::symlink("/etc/passwd", src.path().join("assets/passwd")).expect("link");
    let err = h
        .host
        .install_dir(src.path(), true)
        .await
        .expect_err("link in assets");
    assert!(err.to_string().contains("link"), "{err}");
    assert_eq!(
        std::fs::read_dir(h.dir.path()).expect("dir").count(),
        0,
        "nothing left behind"
    );
    assert!(h.host.list().is_empty());
}

#[tokio::test]
async fn a_slow_reply_is_busy_not_broken() {
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let h = host_with(setup, &Settings::default(), &["probe"], mem_secrets()).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    // Eight 300 ms calls queued at once (each well within its 500 ms): the last replies come
    // after the 2 s an action waits.
    let sleeps = (0..8).map(|_| h.host.action("probe", "sleep", json!({"ms": 300})));
    let late = async {
        tokio::task::yield_now().await;
        h.host.action("probe", "state", json!({"k": "a", "v": "b"})).await
    };
    let (sleeps, late) = tokio::join!(futures_util::future::join_all(sleeps), late);
    assert_eq!(
        late,
        Err(PluginError::Action("Probe is busy".into())),
        "{sleeps:?}"
    );
    assert!(sleeps.iter().any(|r| r.is_ok()), "{sleeps:?}");
    assert_eq!(h.status("probe"), Some(PluginStatus::Running));
    h.host.shutdown().await;
}

/// A lazy store that counts how often its real store is made (each one takes `delay`).
fn counted(made: Arc<std::sync::atomic::AtomicUsize>, delay: Duration) -> LazySecrets {
    LazySecrets::new(move || {
        let made = made.clone();
        async move {
            made.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(delay).await;
            Arc::new(MemSecrets::default()) as Arc<dyn SecretStore>
        }
    })
}

#[tokio::test]
async fn secrets_store_is_made_on_first_use_once() {
    let made = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lazy = counted(made.clone(), Duration::from_millis(100));
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let h = host_with(setup, &Settings::default(), &[], Arc::new(lazy)).await;
    assert_eq!(
        made.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "not at host start"
    );
    let kv = json!({"k": "k", "v": "v"});
    let (a, b) = tokio::join!(
        h.host.action("probe", "secret_set", kv.clone()),
        h.host.action("probe", "secret_get", json!({"k": "k"}))
    );
    assert_eq!((a, b), (Ok(()), Ok(())));
    assert_eq!(h.host.restart("probe").await, Ok(()));
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    assert_eq!(made.load(std::sync::atomic::Ordering::SeqCst), 1, "made once");
    h.host.shutdown().await;
}

#[tokio::test]
async fn shutdown_while_a_plugin_waits_for_its_secrets() {
    let made = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lazy = counted(made.clone(), Duration::from_secs(2));
    let setup = |dir: &std::path::Path| plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
    let h = host_with(setup, &Settings::default(), &["probe"], Arc::new(lazy)).await;
    assert_eq!(h.status("probe"), Some(PluginStatus::Starting));
    let started = Instant::now();
    h.host.shutdown().await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    // The secrets arrive after the shutdown: the instance never starts.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(h.status("probe"), Some(PluginStatus::Off));
}

fn press(plugin: &str, action: &str) -> ulanzi_core::model::Action {
    ulanzi_core::model::Action::Plugin {
        plugin: plugin.into(),
        action: action.into(),
        params: toml::Table::new(),
    }
}

#[tokio::test]
async fn keys_work_before_the_plugins_are_ready() {
    use ulanzi_deckd::actions::{ActionError, ActionRunner, Executor};
    let dir = tempfile::tempdir().expect("dir");
    let cache = tempfile::tempdir().expect("cache");
    hello_folder(dir.path());
    let (tx, _deck) = mpsc::channel(4096);
    let (go, ready) = oneshot::channel::<()>();
    let slot = {
        let (dir, cache) = (dir.path().to_path_buf(), cache.path().to_path_buf());
        PluginSlot::start(async move {
            let _ = ready.await;
            PluginHost::start(
                dir,
                cache,
                mem_secrets(),
                &Settings::default(),
                &BTreeSet::new(),
                tx,
            )
            .await
        })
    };
    let ex = Executor::new(None).with_plugin_slot(slot.clone());
    // Other actions run as usual; a plugin key says why it does nothing yet.
    let media = ulanzi_core::model::Action::Media {
        control: ulanzi_core::model::MediaControl::PlayPause,
        player: None,
    };
    assert!(matches!(ex.run(&media).await, Err(ActionError::Unavailable(_))));
    assert_eq!(
        ex.run(&press("hello", "count")).await,
        Err(ActionError::Unavailable("plugins are still starting".into()))
    );
    assert!(slot.get().is_none());
    go.send(()).expect("start");
    let host = tokio::time::timeout(PROMPTLY, slot.wait())
        .await
        .expect("ready in time")
        .expect("started");
    assert_eq!(ex.run(&press("hello", "count")).await, Ok(()));
    assert_eq!(
        ex.run(&press("nope", "count")).await,
        Err(ActionError::Unavailable(
            "the nope plugin is not installed".into()
        ))
    );
    slot.shutdown().await;
    assert_eq!(host.list()[0].status, PluginStatus::Off, "shut down");
}

#[tokio::test]
async fn shutdown_while_the_plugins_start() {
    let slot = PluginSlot::start(std::future::pending());
    let started = Instant::now();
    tokio::time::timeout(PROMPTLY, slot.shutdown())
        .await
        .expect("shutdown in time");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert!(slot.wait().await.is_none(), "never ready");
    assert!(slot.get().is_none());
}

#[tokio::test]
async fn a_start_that_panics_is_not_starting_forever() {
    use ulanzi_deckd::actions::{ActionError, ActionRunner, Executor};
    let slot = PluginSlot::start(async { panic!("the start panics on purpose") });
    let ready = tokio::time::timeout(PROMPTLY, slot.wait())
        .await
        .expect("wait ends");
    assert!(ready.is_none());
    let ex = Executor::new(None).with_plugin_slot(slot.clone());
    assert_eq!(
        ex.run(&press("hello", "count")).await,
        Err(ActionError::Unavailable("plugins are not running".into()))
    );
    tokio::time::timeout(PROMPTLY, slot.shutdown())
        .await
        .expect("shutdown in time");
}

// ---- Plugin host: manifests, folder inspection, secret writes ----

fn probe_folder(dir: &std::path::Path) {
    plugin_folder(dir, "probe", PROBE_MANIFEST, Some(PROBE_PACKAGE));
}

#[tokio::test]
async fn manifests_list_loaded_plugins_only() {
    let setup = |dir: &std::path::Path| {
        probe_folder(dir);
        plugin_folder(dir, "broken", PROBE_MANIFEST, None);
    };
    let h = host_with(setup, &Settings::default(), &[], mem_secrets()).await;
    let manifests = h.host.manifests();
    assert_eq!(manifests.keys().collect::<Vec<_>>(), ["probe"]);
    assert_eq!(manifests["probe"].secret_keys(), ["password"]);
    h.host.shutdown().await;
}

#[tokio::test]
async fn inspect_dir_reads_without_installing() {
    let h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let src = common::plugins::fixture(PROBE_PACKAGE, PROBE_MANIFEST);
    let inspected = h.host.inspect_dir(src.path()).await.expect("inspects");
    assert_eq!(inspected.manifest.id, "probe");
    assert_eq!(inspected.installed_version, None);
    assert!(!h.dir.path().join("probe").exists());
    assert!(h.status("probe").is_none());

    assert_eq!(
        h.host.install_dir(src.path(), true).await,
        Ok("probe".to_string())
    );
    let inspected = h.host.inspect_dir(src.path()).await.expect("inspects again");
    assert_eq!(inspected.installed_version.as_deref(), Some("1.0.0"));

    std::fs::write(src.path().join("plugin.wasm"), b"").expect("empty wasm");
    let err = h.host.inspect_dir(src.path()).await.expect_err("refused");
    assert!(matches!(err, PluginError::Load(_)), "{err:?}");
    h.host.shutdown().await;
}

fn probe_package(dir: &std::path::Path) -> std::path::PathBuf {
    common::plugins::package_file(
        dir,
        "probe-1.0.0.ulanziplugin",
        PROBE_PACKAGE,
        PROBE_MANIFEST,
        &[("assets/icons/a.png", b"png")],
    )
}

#[tokio::test]
async fn install_file_needs_developer_mode() {
    let h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let pkg = tempfile::tempdir().expect("pkg");
    let file = probe_package(pkg.path());
    let err = h.host.install_file(&file, false).await.expect_err("refused");
    assert_eq!(
        err.to_string(),
        "Turn on Developer mode to install unverified plugins"
    );
    assert!(h.host.list().is_empty());
    assert_eq!(std::fs::read_dir(h.dir.path()).expect("dir").count(), 0);
}

#[tokio::test]
async fn install_file_installs_and_starts() {
    let mut h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let pkg = tempfile::tempdir().expect("pkg");
    let file = probe_package(pkg.path());
    assert_eq!(h.host.install_file(&file, true).await, Ok("probe".to_string()));
    assert!(h.dir.path().join("probe/plugin.wasm").is_file());
    assert!(h.dir.path().join("probe/plugin.toml").is_file());
    assert_eq!(
        std::fs::read(h.dir.path().join("probe/assets/icons/a.png")).expect("asset"),
        b"png"
    );
    let kv = json!({"k": "a", "v": "b"});
    assert_eq!(h.host.action("probe", "state", kv).await, Ok(()));
    assert!(h.saw_state("probe", "a", "b").await);
    // Installing it again replaces it.
    assert_eq!(h.host.install_file(&file, true).await, Ok("probe".to_string()));
    h.host.shutdown().await;
}

#[tokio::test]
async fn inspect_file_works_without_developer_mode() {
    let h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let pkg = tempfile::tempdir().expect("pkg");
    let file = probe_package(pkg.path());
    let inspected = h.host.inspect_file(&file).await.expect("inspects");
    assert_eq!(inspected.manifest.id, "probe");
    assert_eq!(inspected.installed_version, None);
    assert!(h.host.list().is_empty());
    assert_eq!(std::fs::read_dir(h.dir.path()).expect("dir").count(), 0);
    h.host.shutdown().await;
}

#[tokio::test]
async fn install_file_leaves_no_temp_dir() {
    let h = host_with(|_| {}, &Settings::default(), &[], mem_secrets()).await;
    let pkg = tempfile::tempdir().expect("pkg");
    let good = probe_package(pkg.path());
    assert!(h.host.inspect_file(&good).await.is_ok());
    assert!(h.host.install_file(&good, true).await.is_ok());
    // Refused while unpacking, and refused while validating the unpacked plugin.
    let junk = pkg.path().join("junk.ulanziplugin");
    std::fs::write(&junk, b"nope").expect("junk");
    assert!(h.host.install_file(&junk, true).await.is_err());
    assert!(h.host.inspect_file(&junk).await.is_err());
    let bad = common::plugins::package_file(
        pkg.path(),
        "bad.ulanziplugin",
        PROBE_PACKAGE,
        "not a manifest",
        &[],
    );
    assert!(h.host.install_file(&bad, true).await.is_err());
    assert!(h.host.inspect_file(&bad).await.is_err());
    let left: Vec<String> = std::fs::read_dir(h.dir.path())
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["probe"]);
    h.host.shutdown().await;
}

/// A host with the probe running and a handle on the secrets it uses.
async fn probe_with_secrets(settings: &Settings, used: &[&str]) -> (Host, Arc<MemSecrets>) {
    let secrets = Arc::new(MemSecrets::default());
    let h = host_with(probe_folder, settings, used, secrets.clone()).await;
    (h, secrets)
}

fn stored(secrets: &MemSecrets, plugin: &str) -> BTreeMap<String, String> {
    let all = secrets.0.lock().unwrap_or_else(|e| e.into_inner());
    all.get(plugin).cloned().unwrap_or_default()
}

#[tokio::test]
async fn set_secret_restarts_with_the_new_value() {
    let (mut h, _) = probe_with_secrets(&Settings::default(), &["probe"]).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    assert_eq!(h.host.secret_keys("probe").await, Ok(vec![]));

    assert_eq!(
        h.host
            .set_secret("probe", "password", Some("s3cret".into()))
            .await,
        Ok(())
    );
    let get = json!({"k": "password"});
    assert_eq!(h.host.action("probe", "secret_get", get.clone()).await, Ok(()));
    assert!(h.saw_state("probe", "secret", "s3cret").await);
    assert_eq!(
        h.host.secret_keys("probe").await,
        Ok(vec!["password".to_string()])
    );

    assert_eq!(h.host.set_secret("probe", "password", None).await, Ok(()));
    assert_eq!(h.host.action("probe", "secret_get", get).await, Ok(()));
    assert!(h.saw_state("probe", "secret", "none").await);
    assert_eq!(h.host.secret_keys("probe").await, Ok(vec![]));
    h.host.shutdown().await;
}

#[tokio::test]
async fn set_secret_keeps_other_secrets() {
    let (mut h, _) = probe_with_secrets(&Settings::default(), &["probe"]).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    let kv = json!({"k": "k", "v": "v"});
    assert_eq!(h.host.action("probe", "secret_set", kv).await, Ok(()));
    assert_eq!(
        h.host
            .set_secret("probe", "password", Some("s3cret".into()))
            .await,
        Ok(())
    );
    let get = json!({"k": "k"});
    assert_eq!(h.host.action("probe", "secret_get", get).await, Ok(()));
    assert!(h.saw_state("probe", "secret", "v").await);
    h.host.shutdown().await;
}

#[tokio::test]
async fn set_secret_does_not_start_an_off_plugin() {
    let mut settings = Settings::default();
    settings.plugins.insert(
        "probe".into(),
        PluginEntry {
            enabled: false,
            ..PluginEntry::default()
        },
    );
    let (h, secrets) = probe_with_secrets(&settings, &["probe"]).await;
    assert_eq!(h.status("probe"), Some(PluginStatus::Off));
    assert_eq!(
        h.host
            .set_secret("probe", "password", Some("s3cret".into()))
            .await,
        Ok(())
    );
    assert_eq!(h.status("probe"), Some(PluginStatus::Off));
    assert_eq!(
        stored(&secrets, "probe").get("password").map(String::as_str),
        Some("s3cret")
    );
    h.host.shutdown().await;
}

#[tokio::test]
async fn set_secret_refuses_undeclared_keys() {
    let (h, secrets) = probe_with_secrets(&Settings::default(), &[]).await;
    assert_eq!(
        h.host.set_secret("probe", "other", Some("x".into())).await,
        Err(PluginError::Invalid("Probe has no secret setting “other”".into()))
    );
    assert_eq!(
        h.host.set_secret("nope", "password", Some("x".into())).await,
        Err(PluginError::NotInstalled("nope".into()))
    );
    assert!(stored(&secrets, "probe").is_empty());
    h.host.shutdown().await;
}

#[tokio::test]
async fn secret_keys_see_the_settings_copy() {
    let mut settings = Settings::default();
    let mut table = toml::Table::new();
    table.insert("password".into(), toml::Value::String("x".into()));
    table.insert("greeting".into(), toml::Value::String("hi".into()));
    settings.plugins.insert(
        "probe".into(),
        PluginEntry {
            settings: table,
            ..PluginEntry::default()
        },
    );
    let (h, _) = probe_with_secrets(&settings, &[]).await;
    assert_eq!(
        h.host.secret_keys("probe").await,
        Ok(vec!["password".to_string()])
    );
    h.host.shutdown().await;
}

const LEGACY_PACKAGE: &str = "ulanzi-plugin-legacy";
const LEGACY_MANIFEST: &str = include_str!("plugins/legacy/plugin.toml");

#[tokio::test]
async fn legacy_1_0_component_loads_and_runs() {
    // Built against the frozen 1.0 WIT: imports `ulanzi:plugin/core@1.0.0`, no `list-items`.
    let cache = tempfile::tempdir().expect("cache");
    let dir = common::plugins::fixture(LEGACY_PACKAGE, LEGACY_MANIFEST);
    let engine = PluginEngine::new(cache.path()).expect("engine");
    let manifest = Manifest::parse(LEGACY_MANIFEST).expect("manifest");
    assert_eq!(manifest.contract, "1.0");
    let loaded = engine
        .load(&manifest, &dir.path().join("plugin.wasm"))
        .expect("a 1.0 component loads");
    assert!(!loaded.lists_items);

    let setup = |dir: &std::path::Path| plugin_folder(dir, "legacy", LEGACY_MANIFEST, Some(LEGACY_PACKAGE));
    let mut h = host_with(setup, &Settings::default(), &[], mem_secrets()).await;
    assert_eq!(h.host.action("legacy", "ping", json!({})).await, Ok(()));
    assert!(h.saw_state("legacy", "ping", "pong").await);
    let err = h
        .host
        .list_items("legacy", "anything", "", "")
        .await
        .expect_err("no list");
    assert_eq!(err.to_string(), "Legacy has no list to pick from");
    // Still running: the refusal is not a crash.
    assert_eq!(h.status("legacy"), Some(PluginStatus::Running));
    h.host.shutdown().await;
}

#[tokio::test]
async fn probe_reports_that_it_lists_items() {
    let cache = tempfile::tempdir().expect("cache");
    let dir = common::plugins::fixture(PROBE_PACKAGE, PROBE_MANIFEST);
    let engine = PluginEngine::new(cache.path()).expect("engine");
    let manifest = Manifest::parse(PROBE_MANIFEST).expect("manifest");
    let loaded = engine
        .load(&manifest, &dir.path().join("plugin.wasm"))
        .expect("load");
    assert!(loaded.lists_items);
}

#[tokio::test]
async fn probe_lists_items() {
    let h = host_with(probe_folder, &Settings::default(), &["probe"], mem_secrets()).await;
    let items = h
        .host
        .list_items("probe", "things", "lamp", "kitchen")
        .await
        .expect("items");
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0].id, "one");
    assert_eq!(items[0].label, "One lamp kitchen");
    assert_eq!(items[0].subtitle.as_deref(), Some("the first"));
    assert_eq!(items[0].group.as_deref(), Some("kitchen"));
    assert_eq!(items[0].icon.as_deref(), Some("lightbulb"));
    assert_eq!(items[0].state.as_deref(), Some("on"));
    assert_eq!(items[1].id, "two");
    assert_eq!(items[1].subtitle, None);
    assert_eq!(items[1].state, None);
    // The plugin's own refusal comes back as its message.
    let err = h
        .host
        .list_items("probe", "refuse", "", "")
        .await
        .expect_err("refused");
    assert_eq!(err, PluginError::Action("not connected".into()));
    h.host.shutdown().await;
}

#[tokio::test]
async fn list_items_rejects_undeclared_picker() {
    let h = host_with(probe_folder, &Settings::default(), &[], mem_secrets()).await;
    let err = h
        .host
        .list_items("probe", "nope", "", "")
        .await
        .expect_err("undeclared");
    assert_eq!(err, PluginError::Invalid("Probe has no picker \"nope\"".into()));
    // Refused before anything starts.
    assert_eq!(h.status("probe"), Some(PluginStatus::Ready));
    let err = h
        .host
        .list_items("ghost", "things", "", "")
        .await
        .expect_err("unknown plugin");
    assert_eq!(err, PluginError::NotInstalled("ghost".into()));
}

#[tokio::test]
async fn list_items_starts_a_ready_plugin() {
    let h = host_with(probe_folder, &Settings::default(), &[], mem_secrets()).await;
    assert_eq!(h.status("probe"), Some(PluginStatus::Ready));
    let items = h.host.list_items("probe", "things", "", "").await.expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(h.status("probe"), Some(PluginStatus::Running));
    h.host.shutdown().await;
}

#[tokio::test]
async fn list_items_is_bounded() {
    let h = host_with(probe_folder, &Settings::default(), &["probe"], mem_secrets()).await;
    let items = h.host.list_items("probe", "flood", "", "").await.expect("items");
    assert_eq!(items.len(), 5000);
    for i in &items {
        assert!(!i.id.is_empty() && i.id.len() <= 1024, "id {}", i.id.len());
        assert!(i.label.len() <= 256, "label {}", i.label.len());
        for f in [&i.subtitle, &i.group, &i.state] {
            assert!(f.as_ref().is_none_or(|s| s.len() <= 256));
        }
        assert!(i.icon.as_ref().is_none_or(|s| s.len() <= 128));
    }
    // Cut at a character boundary: byte 256 falls inside an "é", so the cut backs up one byte.
    assert!(items[0].label.starts_with('a'));
    assert!(items[0].label[1..].chars().all(|c| c == 'é'));
    assert_eq!(items[0].label.len(), 255);
    assert_eq!(items[1].label.len(), 256);
    assert_eq!(
        items[0].id, "item0",
        "items with an empty or overlong id are dropped"
    );
    let err = h
        .host
        .list_items("probe", "loud", "", "")
        .await
        .expect_err("refused");
    assert!(err.to_string().len() <= 4096, "{}", err.to_string().len());
    assert_eq!(h.status("probe"), Some(PluginStatus::Running));
    h.host.shutdown().await;
}

#[tokio::test]
async fn list_items_trap_counts_as_crash() {
    let mut h = host_with(probe_folder, &Settings::default(), &["probe"], mem_secrets()).await;
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    let cleared = |m: &DeckMsg| matches!(m, DeckMsg::PluginCleared(id) if id == "probe");
    let err = h
        .host
        .list_items("probe", "panic", "", "")
        .await
        .expect_err("panics");
    assert_eq!(err, PluginError::Failed("Probe".into()));
    assert!(h.deck_saw(cleared).await, "restarted once like any crash");
    h.wait_status("probe", |s| *s == PluginStatus::Running).await;
    let err = h
        .host
        .list_items("probe", "panic", "", "")
        .await
        .expect_err("panics");
    assert_eq!(err, PluginError::Failed("Probe".into()));
    h.wait_status("probe", |s| matches!(s, PluginStatus::Failed(_)))
        .await;
    h.host.shutdown().await;
}
