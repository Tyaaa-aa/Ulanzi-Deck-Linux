//! Plugin instances. Each instance is one thread (`plugin-<id>`) that owns the wasmtime `Store`
//! and feeds it events from a bounded queue, one guest call at a time. Host functions never block:
//! state and status go out over an unbounded channel, timers are tokio tasks on the daemon's runtime.

use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use ulanzi_core::plugin::Manifest;
use wasmtime::component::ResourceTable;
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_io::bytes::Bytes;
use wasmtime_wasi_io::poll::Pollable;
use wasmtime_wasi_io::streams::{OutputStream, StreamResult};

use super::ulanzi::plugin::types::{HttpResponse, LogLevel, Payload, Status};
use super::ulanzi::plugin::{core, net, types};
use super::{LoadedPlugin, PluginEngine, PluginError, PluginPre};

/// Events waiting for a plugin; when full, new events are dropped (one warning per episode).
const EVENT_QUEUE: usize = 1024;
/// Per guest call: 50 epoch ticks of 10 ms = 500 ms.
const CALL_DEADLINE: u64 = 50;
const MEMORY_LIMIT: usize = 64 << 20;
/// Live timers per instance. Far above what a sane plugin needs, low enough to bound memory.
const MAX_TIMERS: usize = 4096;
/// Repeating timers fire at most this often.
const MIN_REPEAT: Duration = Duration::from_millis(10);
/// A guest stdout/stderr line longer than this is logged in pieces.
const MAX_LOG_LINE: usize = 4096;

/// What the host asks a plugin instance to do, in order.
pub enum Event {
    /// Settings as JSON; at start and whenever they change.
    Init {
        settings: serde_json::Value,
    },
    Action {
        action: String,
        params: serde_json::Value,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Open(u32),
    Message(u32, Payload),
    Closed(u32, String),
    Http(u32, Result<HttpResponse, String>),
    Timer(u32),
    Stop,
}

/// What a plugin instance reports back.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    State {
        key: String,
        value: String,
    },
    Status {
        status: Status,
        message: String,
    },
    /// The plugin's full secret map after a change; the caller persists it.
    SecretsChanged(BTreeMap<String, String>),
}

/// Why an instance's thread ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Stopped,
    /// A trap (panic, endless loop, out of memory, ...), with a one-line reason.
    Crashed(String),
}

/// A running instance. The thread ends on [`Event::Stop`] or a crash; `exited` then resolves.
pub struct InstanceHandle {
    /// Bounded (1024). Prefer [`InstanceHandle::post`], which never blocks.
    pub events: SyncSender<Event>,
    pub exited: oneshot::Receiver<Exit>,
    queue: Queue,
}

impl InstanceHandle {
    /// Queue an event without blocking. When the queue is full the event is dropped (an action's
    /// reply sender with it) and one warning is logged per overflow episode. False when dropped.
    pub fn post(&self, event: Event) -> bool {
        self.queue.post(event)
    }
}

/// The sending side of an instance's event queue, shared by the handle, timers (and sockets).
#[derive(Clone)]
pub(crate) struct Queue {
    tx: SyncSender<Event>,
    overflowing: Arc<AtomicBool>,
    plugin: Arc<str>,
}

impl Queue {
    pub(crate) fn post(&self, event: Event) -> bool {
        match self.tx.try_send(event) {
            Ok(()) => {
                if self.overflowing.load(Ordering::Relaxed) && self.overflowing.swap(false, Ordering::Relaxed)
                {
                    tracing::info!(target: "plugin", plugin = %self.plugin, "event queue has room again");
                }
                true
            }
            Err(TrySendError::Full(_)) => {
                if !self.overflowing.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        target: "plugin", plugin = %self.plugin,
                        "event queue full ({EVENT_QUEUE}); dropping events until it drains"
                    );
                }
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

struct TimerEntry {
    task: AbortHandle,
    repeat: bool,
}

/// The `Store` data of a plugin instance.
pub struct HostState {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: StoreLimits,
    /// Plugin id; log lines carry it as the `plugin` field.
    pub(crate) plugin: Arc<str>,
    pub(crate) manifest: Manifest,
    /// The daemon's runtime: timers (and sockets) run here.
    pub(crate) rt: Handle,
    pub(crate) queue: Queue,
    out: mpsc::UnboundedSender<Output>,
    secrets: BTreeMap<String, String>,
    timers: HashMap<u32, TimerEntry>,
    last_timer: u32,
    timers_full_warned: bool,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl Drop for HostState {
    fn drop(&mut self) {
        for (_, t) in self.timers.drain() {
            t.task.abort();
        }
    }
}

impl HostState {
    fn emit(&self, output: Output) {
        // The receiver is gone only while the daemon shuts down; nothing to do then.
        let _ = self.out.send(output);
    }

    fn next_timer_id(&mut self) -> u32 {
        loop {
            self.last_timer = self.last_timer.wrapping_add(1);
            if self.last_timer != 0 && !self.timers.contains_key(&self.last_timer) {
                return self.last_timer;
            }
        }
    }

    /// Should `Event::Timer(id)` reach the guest? Cancelled timers are skipped; a one-shot timer
    /// is forgotten once it fired.
    fn timer_due(&mut self, id: u32) -> bool {
        match self.timers.get(&id) {
            None => false,
            Some(t) if t.repeat => true,
            Some(_) => {
                self.timers.remove(&id);
                true
            }
        }
    }
}

impl types::Host for HostState {}

impl core::Host for HostState {
    fn set_timer(&mut self, ms: u32, repeat: bool) -> u32 {
        if self.timers.len() >= MAX_TIMERS {
            // One-shot timers whose event was dropped (queue overflow) never fire; prune them.
            self.timers.retain(|_, t| !t.task.is_finished());
        }
        if self.timers.len() >= MAX_TIMERS {
            if !self.timers_full_warned {
                self.timers_full_warned = true;
                tracing::warn!(
                    target: "plugin", plugin = %self.plugin,
                    "more than {MAX_TIMERS} timers; new timers are ignored"
                );
            }
            return 0;
        }
        self.timers_full_warned = false;
        let id = self.next_timer_id();
        let queue = self.queue.clone();
        let after = Duration::from_millis(ms.into());
        let task = self.rt.spawn(async move {
            if repeat {
                let every = after.max(MIN_REPEAT);
                let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    queue.post(Event::Timer(id));
                }
            } else {
                tokio::time::sleep(after).await;
                queue.post(Event::Timer(id));
            }
        });
        self.timers.insert(
            id,
            TimerEntry {
                task: task.abort_handle(),
                repeat,
            },
        );
        id
    }

    fn cancel_timer(&mut self, id: u32) {
        if let Some(t) = self.timers.remove(&id) {
            t.task.abort();
        }
    }

    fn set_state(&mut self, key: String, value: String) {
        self.emit(Output::State { key, value });
    }

    fn secret_get(&mut self, key: String) -> Option<String> {
        self.secrets.get(&key).cloned()
    }

    fn secret_set(&mut self, key: String, value: String) {
        if self.secrets.get(&key) != Some(&value) {
            self.secrets.insert(key, value);
            self.emit(Output::SecretsChanged(self.secrets.clone()));
        }
    }

    fn secret_delete(&mut self, key: String) {
        if self.secrets.remove(&key).is_some() {
            self.emit(Output::SecretsChanged(self.secrets.clone()));
        }
    }

    fn set_status(&mut self, status: Status, message: String) {
        self.emit(Output::Status { status, message });
    }

    fn log(&mut self, level: LogLevel, message: String) {
        let plugin = &*self.plugin;
        match level {
            LogLevel::Debug => tracing::debug!(target: "plugin", plugin = %plugin, "{message}"),
            LogLevel::Info => tracing::info!(target: "plugin", plugin = %plugin, "{message}"),
            LogLevel::Warn => tracing::warn!(target: "plugin", plugin = %plugin, "{message}"),
            LogLevel::Error => tracing::error!(target: "plugin", plugin = %plugin, "{message}"),
        }
    }
}

// The network arrives in Task 6 (net.rs), which replaces these bodies. Permission checks read
// `self.manifest.permissions`; socket tasks run on `self.rt` and report through `self.queue`.
impl net::Host for HostState {
    fn ws_open(&mut self, _url: String) -> Result<u32, String> {
        Err("not available".into())
    }
    fn tcp_open(&mut self, _host: String, _port: u16, _tls: bool) -> Result<u32, String> {
        Err("not available".into())
    }
    fn send(&mut self, _socket: u32, _data: Payload) -> Result<(), String> {
        Err("not available".into())
    }
    fn close(&mut self, _socket: u32) {}
    fn fetch(&mut self, _request: types::HttpRequest) -> Result<u32, String> {
        Err("not available".into())
    }
}

/// Start an instance of `loaded` on its own thread. `secrets` are the plugin's stored secrets;
/// `out` receives its state, status and secret changes; timers run on `rt`. Instantiation happens
/// on the new thread, so a guest that traps while starting shows up as `Exit::Crashed`.
pub fn spawn_instance(
    loaded: &LoadedPlugin,
    engine: &PluginEngine,
    secrets: BTreeMap<String, String>,
    out: mpsc::UnboundedSender<Output>,
    rt: Handle,
) -> Result<InstanceHandle, PluginError> {
    let manifest = loaded.manifest.clone();
    let plugin: Arc<str> = manifest.id.as_str().into();
    let pre = PluginPre::new(loaded.pre.clone())
        .map_err(|e| PluginError::Load(format!("{} is not a valid plugin: {e:#}", manifest.name)))?;

    let (tx, rx) = std::sync::mpsc::sync_channel(EVENT_QUEUE);
    let queue = Queue {
        tx: tx.clone(),
        overflowing: Arc::new(AtomicBool::new(false)),
        plugin: Arc::clone(&plugin),
    };
    let wasi = WasiCtxBuilder::new()
        .stdout(GuestOutput {
            plugin: Arc::clone(&plugin),
            stderr: false,
        })
        .stderr(GuestOutput {
            plugin: Arc::clone(&plugin),
            stderr: true,
        })
        .build();
    // A failed `memory.grow` returns -1 to the guest (wasm semantics) instead of trapping at
    // once; a Rust guest then aborts in `handle_alloc_error`, which is an `unreachable` trap.
    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .trap_on_grow_failure(false)
        .build();
    let state = HostState {
        wasi,
        table: ResourceTable::new(),
        limits,
        plugin: Arc::clone(&plugin),
        manifest,
        rt,
        queue: queue.clone(),
        out,
        secrets,
        timers: HashMap::new(),
        last_timer: 0,
        timers_full_warned: false,
    };
    let mut store = Store::new(engine.engine(), state);
    store.limiter(|s| &mut s.limits);

    let (exit_tx, exited) = oneshot::channel();
    std::thread::Builder::new()
        .name(format!("plugin-{plugin}"))
        .spawn(move || {
            let exit = run(store, &pre, rx);
            let _ = exit_tx.send(exit);
        })
        .map_err(|e| PluginError::Io(format!("cannot start the {plugin} plugin: {e}")))?;
    Ok(InstanceHandle {
        events: tx,
        exited,
        queue,
    })
}

/// The instance thread: instantiate, then one guest call per event until `Stop` or a trap.
fn run(mut store: Store<HostState>, pre: &PluginPre<HostState>, rx: Receiver<Event>) -> Exit {
    let plugin = Arc::clone(&store.data().plugin);
    let failed = PluginError::Failed(store.data().manifest.name.clone()).to_string();

    store.set_epoch_deadline(CALL_DEADLINE);
    let exit = match pre.instantiate(&mut store) {
        Err(e) => Exit::Crashed(crash_reason(&e)),
        Ok(guest) => loop {
            let Ok(event) = rx.recv() else {
                break Exit::Stopped;
            };
            store.set_epoch_deadline(CALL_DEADLINE);
            let s = &mut store;
            let called = match event {
                Event::Stop => break Exit::Stopped,
                Event::Init { settings } => guest.call_init(s, &settings.to_string()).map(|r| {
                    if let Err(e) = r {
                        tracing::warn!(target: "plugin", plugin = %plugin, "settings refused: {e}");
                    }
                }),
                Event::Action {
                    action,
                    params,
                    reply,
                } => match guest.call_on_action(s, &action, &params.to_string()) {
                    Ok(r) => {
                        let _ = reply.send(r);
                        Ok(())
                    }
                    Err(e) => {
                        let _ = reply.send(Err(failed.clone()));
                        Err(e)
                    }
                },
                Event::Open(socket) => guest.call_on_open(s, socket),
                Event::Message(socket, data) => guest.call_on_message(s, socket, &data),
                Event::Closed(socket, reason) => guest.call_on_closed(s, socket, &reason),
                Event::Http(id, response) => {
                    guest.call_on_http(s, id, response.as_ref().map_err(String::as_str))
                }
                Event::Timer(id) => match s.data_mut().timer_due(id) {
                    true => guest.call_on_timer(s, id),
                    false => Ok(()),
                },
            };
            if let Err(e) = called {
                break Exit::Crashed(crash_reason(&e));
            }
        },
    };

    if let Exit::Crashed(why) = &exit {
        tracing::warn!(target: "plugin", plugin = %plugin, "crashed: {why}");
        // Nothing queued behind the crash runs; actions waiting on a reply get the error.
        while let Ok(event) = rx.try_recv() {
            if let Event::Action { reply, .. } = event {
                let _ = reply.send(Err(failed.clone()));
            }
        }
    }
    // Dropping the store aborts the instance's timers before anyone learns it exited.
    drop(store);
    exit
}

/// A one-line, user-presentable reason for a failed guest call.
fn crash_reason(e: &wasmtime::Error) -> String {
    match e.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => "took longer than 500 ms".into(),
        // How a Rust guest panics or fails to allocate (it aborts); the details went to stderr.
        Some(Trap::UnreachableCodeReached) => "panicked or ran out of memory".into(),
        Some(trap) => trap.to_string(),
        None => e
            .root_cause()
            .to_string()
            .lines()
            .next()
            .unwrap_or("unknown error")
            .to_string(),
    }
}

/// Guest stdout (info) or stderr (warn), logged line by line with the plugin's id.
#[derive(Clone)]
struct GuestOutput {
    plugin: Arc<str>,
    stderr: bool,
}

impl IsTerminal for GuestOutput {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for GuestOutput {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(LineLog::new(self.clone()))
    }
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(LineLog::new(self.clone()))
    }
}

struct LineLog {
    to: GuestOutput,
    buf: Vec<u8>,
}

impl LineLog {
    fn new(to: GuestOutput) -> Self {
        LineLog { to, buf: Vec::new() }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            self.emit(&line);
        }
        if self.buf.len() > MAX_LOG_LINE {
            self.flush_partial();
        }
    }

    fn flush_partial(&mut self) {
        let rest = std::mem::take(&mut self.buf);
        self.emit(&rest);
    }

    fn emit(&self, line: &[u8]) {
        let text = String::from_utf8_lossy(line);
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        let plugin = &*self.to.plugin;
        if self.to.stderr {
            tracing::warn!(target: "plugin", plugin = %plugin, "stderr: {text}");
        } else {
            tracing::info!(target: "plugin", plugin = %plugin, "stdout: {text}");
        }
    }
}

impl Drop for LineLog {
    fn drop(&mut self) {
        self.flush_partial();
    }
}

impl OutputStream for LineLog {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.push(&bytes);
        Ok(())
    }
    fn flush(&mut self) -> StreamResult<()> {
        self.flush_partial();
        Ok(())
    }
    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(64 * 1024)
    }
}

#[wasmtime_wasi_io::async_trait]
impl Pollable for LineLog {
    async fn ready(&mut self) {}
}

impl tokio::io::AsyncWrite for LineLog {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        self.get_mut().push(buf);
        Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.get_mut().flush_partial();
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.poll_flush(cx)
    }
}
