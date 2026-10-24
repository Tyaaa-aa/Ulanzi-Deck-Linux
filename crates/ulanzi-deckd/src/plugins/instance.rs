//! Plugin instances. Each instance is one thread (`plugin-<id>`) that owns the wasmtime `Store`
//! and feeds it events from a bounded queue, one guest call at a time. Host functions never block:
//! state and status go out over an unbounded channel, timers are tokio tasks on the daemon's runtime.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use ulanzi_core::plugin::{Manifest, PickItem};
use wasmtime::component::{ComponentExportIndex, Resource, ResourceTable, TypedFunc};
use wasmtime::{Store, StoreLimits, StoreLimitsBuilder, Trap, UpdateDeadline};
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_io::bindings::wasi::io::poll as async_poll;
use wasmtime_wasi_io::bytes::Bytes;
use wasmtime_wasi_io::poll::{DynPollable, Pollable};
use wasmtime_wasi_io::streams::{OutputStream, StreamResult};

use super::engine::Ticker;
use super::net::{Budget, Net};
use super::ulanzi::plugin::types::{HttpResponse, LogLevel, Payload, Status};
use super::ulanzi::plugin::{core, types};
use super::{LoadedPlugin, PluginBase, PluginBasePre, PluginEngine, PluginError};

/// Events waiting for a plugin; when full, new events are dropped (one warning per episode).
const EVENT_QUEUE: usize = 1024;
/// Per guest call: 50 epoch ticks of 10 ms = 500 ms.
const CALL_DEADLINE: u64 = 50;
/// The same budget in wall time, waiting included: blocking WASI waits are not counted in ticks.
const CALL_BUDGET: Duration = Duration::from_millis(500);
/// A blocking WASI wait checks the stop flag and the call budget this often.
const WAIT_SLICE: Duration = Duration::from_millis(10);
/// A running guest call checks for a stop request this often (in epoch ticks).
const STOP_CHECK: u64 = 5;
/// An idle instance re-checks its stop flag this often, in case the wake-up event was lost.
const STOP_POLL: Duration = Duration::from_millis(200);
const MAX_SECRETS: usize = 256;
/// Most distinct `set-state` keys per instance; new keys beyond it are ignored.
const MAX_STATE_KEYS: usize = 256;
/// Longest key for `set-state` and secrets, in bytes.
const MAX_KEY: usize = 256;
/// Longest `set-state` value, in bytes. Values are label text (`{<plugin>.<key>}`).
const MAX_STATE_VALUE: usize = 1024;
/// Longest secret value, in bytes.
const MAX_SECRET_VALUE: usize = 64 * 1024;
const MEMORY_LIMIT: usize = 64 << 20;
/// Live timers per instance. Far above what a sane plugin needs, low enough to bound memory.
const MAX_TIMERS: usize = 4096;
/// Repeating timers fire at most this often.
const MIN_REPEAT: Duration = Duration::from_millis(10);
/// A guest stdout/stderr line longer than this is logged in pieces; a longer `log` or `set-status`
/// message is truncated.
const MAX_LOG_LINE: usize = 4096;
/// Most entries of a `list-items` answer; the rest are dropped.
const MAX_ITEMS: usize = 5000;
/// An entry whose id is longer than this (or empty) is dropped.
const MAX_ITEM_ID: usize = 1024;
/// Longest label, subtitle, group and state of an entry; longer ones are cut.
const MAX_ITEM_TEXT: usize = 256;
const MAX_ITEM_ICON: usize = 128;
/// Longest `list-items` error text; longer ones are cut.
const MAX_ITEM_ERROR: usize = 4096;

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
    /// Contract 1.1: the entries of a picker (already bounded when they arrive). A plugin without
    /// `list-items` answers `<name> has no list to pick from`.
    ListItems {
        picker: String,
        query: String,
        group: String,
        reply: oneshot::Sender<Result<Vec<PickItem>, String>>,
    },
    Open(u32),
    Message(u32, Payload),
    Closed(u32, String),
    Http(u32, Result<HttpResponse, String>),
    Timer(u32),
    Stop,
}

/// What a plugin instance reports back. `Debug` never shows secret values.
#[derive(Clone, PartialEq)]
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

impl fmt::Debug for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Output::State { key, value } => f
                .debug_struct("State")
                .field("key", key)
                .field("value", value)
                .finish(),
            Output::Status { status, message } => f
                .debug_struct("Status")
                .field("status", status)
                .field("message", message)
                .finish(),
            Output::SecretsChanged(secrets) => {
                f.debug_tuple("SecretsChanged").field(&Redacted(secrets)).finish()
            }
        }
    }
}

/// Secret keys with their values hidden.
struct Redacted<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|k| (k, "<redacted>")))
            .finish()
    }
}

/// Why an instance's thread ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Stopped,
    /// A trap (panic, endless loop, out of memory, ...), with a one-line reason.
    Crashed(String),
}

/// A running instance. The thread ends on [`InstanceHandle::stop`] (also called on drop),
/// [`Event::Stop`] or a crash; `exited` then resolves.
///
/// The instance is gone (treat it as `<name> stopped working`) when an action's reply receiver
/// errs with `RecvError` (the event was dropped because the instance already ended) or when
/// `exited` errs (the thread ended without reporting, which should not happen). An action
/// [`InstanceHandle::post`] refuses with [`Dropped::Full`] found the plugin busy instead.
/// Await `exited` through `&mut handle.exited`: the handle stops the instance when dropped.
pub struct InstanceHandle {
    /// Bounded (1024). Prefer [`InstanceHandle::post`], which never blocks.
    pub events: SyncSender<Event>,
    pub exited: oneshot::Receiver<Exit>,
    queue: Queue,
    stop: Arc<AtomicBool>,
    net_budget: Arc<Budget>,
}

impl Drop for InstanceHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl InstanceHandle {
    /// Ask the instance to stop, without blocking. A guest call in progress is cut short within
    /// about 50 ms; queued actions are answered `the <name> plugin is turned off`. The instance
    /// then exits with `Exit::Stopped`, even when its queue is full.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // Best effort wake-up; when the queue is full the flag is seen before the next event.
        let _ = self.queue.tx.try_send(Event::Stop);
    }

    /// Bytes of network data (socket messages, HTTP bodies) waiting in the event queue. Bounded:
    /// sockets stop reading while the budget (16 MiB) is used up.
    pub fn net_queued(&self) -> usize {
        self.net_budget.used()
    }

    /// Queue an event without blocking. When the queue is full the event is dropped (an action's
    /// reply sender with it) and one warning is logged per overflow episode.
    pub fn post(&self, event: Event) -> Result<(), Dropped> {
        self.queue.try_post(event)
    }
}

/// Why an event was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// The queue is full: the plugin is busy.
    Full,
    /// The instance has ended.
    Gone,
}

/// The sending side of an instance's event queue, shared by the handle, timers and sockets.
#[derive(Clone)]
pub(crate) struct Queue {
    tx: SyncSender<Event>,
    overflowing: Arc<AtomicBool>,
    plugin: Arc<str>,
}

impl Queue {
    /// [`Queue::try_post`], true when queued.
    pub(crate) fn post(&self, event: Event) -> bool {
        self.try_post(event).is_ok()
    }

    pub(crate) fn try_post(&self, event: Event) -> Result<(), Dropped> {
        match self.tx.try_send(event) {
            Ok(()) => {
                if self.overflowing.load(Ordering::Relaxed) && self.overflowing.swap(false, Ordering::Relaxed)
                {
                    tracing::info!(target: "plugin", plugin = %self.plugin, "event queue has room again");
                }
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                if !self.overflowing.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        target: "plugin", plugin = %self.plugin,
                        "event queue full ({EVENT_QUEUE}); dropping events until it drains"
                    );
                }
                Err(Dropped::Full)
            }
            Err(TrySendError::Disconnected(_)) => Err(Dropped::Gone),
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
    /// The daemon's runtime: timers, sockets and HTTP requests run here.
    pub(crate) rt: Handle,
    pub(crate) queue: Queue,
    /// Sockets and HTTP requests; dropped (and so closed) with the Store.
    pub(crate) net: Net,
    out: mpsc::UnboundedSender<Output>,
    secrets: BTreeMap<String, String>,
    /// Secrets changed during the current guest call; reported once when it returns.
    secrets_dirty: bool,
    timers: HashMap<u32, TimerEntry>,
    last_timer: u32,
    timers_full_warned: bool,
    /// The `set-state` keys used so far (at most `MAX_STATE_KEYS`).
    state_keys: HashSet<String>,
    /// The last `set-status` reported; a repeat is not reported again.
    last_status: Option<(Status, String)>,
    /// Limits already warned about (one warning per instance and kind).
    warned: BTreeSet<&'static str>,
    /// Set by `InstanceHandle::stop`; checked between events and during guest calls.
    stop: Arc<AtomicBool>,
    /// Epoch ticks left in the current guest call's budget.
    ticks_left: u64,
    /// When the current guest call started; bounds blocking WASI waits.
    call_started: Instant,
    /// A running instance keeps the epoch ticker alive.
    _ticker: Arc<Ticker>,
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

    fn warn_once(&mut self, kind: &'static str, message: &str) {
        if self.warned.insert(kind) {
            tracing::warn!(target: "plugin", plugin = %self.plugin, "{message}");
        }
    }

    /// Report the secret map once per guest call, not once per change.
    fn flush_secrets(&mut self) {
        if std::mem::take(&mut self.secrets_dirty) {
            self.emit(Output::SecretsChanged(self.secrets.clone()));
        }
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
        if key.len() > MAX_KEY || value.len() > MAX_STATE_VALUE {
            let msg = format!("state over {MAX_KEY} B key / {MAX_STATE_VALUE} B value is ignored");
            self.warn_once("state-size", &msg);
            return;
        }
        if !self.state_keys.contains(&key) {
            if self.state_keys.len() >= MAX_STATE_KEYS {
                let msg = format!("more than {MAX_STATE_KEYS} state keys; new keys are ignored");
                self.warn_once("state-count", &msg);
                return;
            }
            self.state_keys.insert(key.clone());
        }
        self.emit(Output::State { key, value });
    }

    fn secret_get(&mut self, key: String) -> Option<String> {
        self.secrets.get(&key).cloned()
    }

    fn secret_set(&mut self, key: String, value: String) {
        if key.len() > MAX_KEY || value.len() > MAX_SECRET_VALUE {
            let msg = format!("secret over {MAX_KEY} B key / {MAX_SECRET_VALUE} B value is ignored");
            self.warn_once("secret-size", &msg);
            return;
        }
        if !self.secrets.contains_key(&key) && self.secrets.len() >= MAX_SECRETS {
            let msg = format!("more than {MAX_SECRETS} secrets; new secrets are ignored");
            self.warn_once("secret-count", &msg);
            return;
        }
        if self.secrets.get(&key) != Some(&value) {
            self.secrets.insert(key, value);
            self.secrets_dirty = true;
        }
    }

    fn secret_delete(&mut self, key: String) {
        if self.secrets.remove(&key).is_some() {
            self.secrets_dirty = true;
        }
    }

    fn set_status(&mut self, status: Status, message: String) {
        let message = truncate(&message, MAX_LOG_LINE).to_string();
        if self
            .last_status
            .as_ref()
            .is_some_and(|(s, m)| *s == status && *m == message)
        {
            return;
        }
        self.last_status = Some((status, message.clone()));
        self.emit(Output::Status { status, message });
    }

    fn log(&mut self, level: LogLevel, message: String) {
        let message = truncate(&message, MAX_LOG_LINE);
        let plugin = &*self.plugin;
        match level {
            LogLevel::Debug => tracing::debug!(target: "plugin", plugin = %plugin, "{message}"),
            LogLevel::Info => tracing::info!(target: "plugin", plugin = %plugin, "{message}"),
            LogLevel::Warn => tracing::warn!(target: "plugin", plugin = %plugin, "{message}"),
            LogLevel::Error => tracing::error!(target: "plugin", plugin = %plugin, "{message}"),
        }
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
    let list_items = loaded.list_items;
    let pre = PluginBasePre::new(loaded.pre.clone())
        .map_err(|e| PluginError::Load(format!("{} is not a valid plugin: {e:#}", manifest.name)))?;

    let (tx, rx) = std::sync::mpsc::sync_channel(EVENT_QUEUE);
    let stop = Arc::new(AtomicBool::new(false));
    let net_budget = Arc::new(Budget::default());
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
        net: Net::new(Arc::clone(&net_budget)),
        out,
        secrets,
        secrets_dirty: false,
        timers: HashMap::new(),
        last_timer: 0,
        timers_full_warned: false,
        state_keys: HashSet::new(),
        last_status: None,
        warned: BTreeSet::new(),
        stop: Arc::clone(&stop),
        ticks_left: CALL_DEADLINE,
        call_started: Instant::now(),
        _ticker: Arc::clone(&loaded.ticker),
    };
    let mut store = Store::new(engine.engine(), state);
    store.limiter(|s| &mut s.limits);
    // The deadline fires every STOP_CHECK ticks: stop when asked, trap when the budget is spent.
    store.epoch_deadline_callback(|mut cx| {
        let s = cx.data_mut();
        if s.stop.load(Ordering::Relaxed) {
            return Err(wasmtime::Error::new(Stopping));
        }
        s.ticks_left = s.ticks_left.saturating_sub(STOP_CHECK);
        // Ticks pass only while wasm runs; blocking WASI waits count through the wall clock.
        if s.ticks_left == 0 || s.call_started.elapsed() >= CALL_BUDGET {
            return Ok(UpdateDeadline::Interrupt);
        }
        Ok(UpdateDeadline::Continue(STOP_CHECK.min(s.ticks_left)))
    });

    let (exit_tx, exited) = oneshot::channel();
    std::thread::Builder::new()
        .name(format!("plugin-{plugin}"))
        .spawn(move || {
            let exit = run(store, &pre, list_items, rx);
            let _ = exit_tx.send(exit);
        })
        .map_err(|e| PluginError::Io(format!("cannot start the {plugin} plugin: {e}")))?;
    Ok(InstanceHandle {
        events: tx,
        exited,
        queue,
        stop,
        net_budget,
    })
}

/// Returned from the epoch callback (or a blocking WASI wait) when the instance is asked to stop
/// during a guest call.
#[derive(Debug)]
struct Stopping;

impl fmt::Display for Stopping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("stopped")
    }
}

impl std::error::Error for Stopping {}

/// Give the next guest call its full budget, checked for stop requests every `STOP_CHECK` ticks.
fn arm(store: &mut Store<HostState>) {
    let s = store.data_mut();
    s.ticks_left = CALL_DEADLINE;
    s.call_started = Instant::now();
    store.set_epoch_deadline(STOP_CHECK);
}

/// `wasi:io/poll` whose blocking waits (`poll`, `pollable.block`; std's `thread::sleep` is one)
/// give up when the instance is asked to stop or the call's budget is spent. wasmtime-wasi's own
/// version blocks the instance thread until the pollable is ready, and epoch interruption cannot
/// cut that short: it only fires while wasm code runs.
pub(crate) struct BoundedPoll<'a> {
    table: &'a mut ResourceTable,
    stop: &'a AtomicBool,
    call_started: Instant,
}

impl HostState {
    pub(crate) fn bounded_poll(&mut self) -> BoundedPoll<'_> {
        BoundedPoll {
            table: &mut self.table,
            stop: &self.stop,
            call_started: self.call_started,
        }
    }
}

/// Wait for `ready` in slices of `WAIT_SLICE`. Ends like the epoch callback would: `Stopping` when
/// asked to stop, an interrupt trap ("took longer than 500 ms") when the call's budget is spent.
fn bounded<T>(
    stop: &AtomicBool,
    call_started: Instant,
    ready: impl Future<Output = wasmtime::Result<T>>,
) -> wasmtime::Result<T> {
    wasmtime_wasi::runtime::in_tokio(async {
        let mut ready = std::pin::pin!(ready);
        loop {
            if let Ok(done) = tokio::time::timeout(WAIT_SLICE, ready.as_mut()).await {
                return done;
            }
            if stop.load(Ordering::Relaxed) {
                return Err(wasmtime::Error::new(Stopping));
            }
            if call_started.elapsed() >= CALL_BUDGET {
                return Err(wasmtime::Error::new(Trap::Interrupt));
            }
        }
    })
}

impl wasmtime_wasi::p2::bindings::sync::io::poll::Host for BoundedPoll<'_> {
    fn poll(&mut self, pollables: Vec<Resource<DynPollable>>) -> wasmtime::Result<Vec<u32>> {
        let ready = async_poll::Host::poll(&mut *self.table, pollables);
        bounded(self.stop, self.call_started, ready)
    }
}

impl wasmtime_wasi::p2::bindings::sync::io::poll::HostPollable for BoundedPoll<'_> {
    fn ready(&mut self, pollable: Resource<DynPollable>) -> wasmtime::Result<bool> {
        // Does not wait: it checks readiness once.
        wasmtime_wasi::runtime::in_tokio(async_poll::HostPollable::ready(&mut *self.table, pollable))
    }
    fn block(&mut self, pollable: Resource<DynPollable>) -> wasmtime::Result<()> {
        let ready = async_poll::HostPollable::block(&mut *self.table, pollable);
        bounded(self.stop, self.call_started, ready)
    }
    fn drop(&mut self, pollable: Resource<DynPollable>) -> wasmtime::Result<()> {
        async_poll::HostPollable::drop(&mut *self.table, pollable)
    }
}

/// How a failed guest call ends the instance.
fn ended(e: &wasmtime::Error) -> Exit {
    if e.downcast_ref::<Stopping>().is_some() {
        Exit::Stopped
    } else {
        Exit::Crashed(crash_reason(e))
    }
}

/// `list-items(picker, query, group)`.
type ListItems = TypedFunc<(String, String, String), (Result<Vec<types::Item>, String>,)>;

/// Instantiate the component: its 1.0 exports, and `list-items` when `list_items` names it.
/// A `list-items` of the wrong type counts as missing (with a warning).
fn instantiate(
    store: &mut Store<HostState>,
    pre: &PluginBasePre<HostState>,
    list_items: Option<ComponentExportIndex>,
) -> wasmtime::Result<(PluginBase, Option<ListItems>)> {
    let instance = pre.instance_pre().instantiate(&mut *store)?;
    let guest = PluginBase::new(&mut *store, &instance)?;
    let list_items = list_items.and_then(|index| {
        instance
            .get_typed_func(&mut *store, index)
            .inspect_err(|e| {
                let plugin = &store.data().plugin;
                tracing::warn!(target: "plugin", plugin = %plugin, "list-items has the wrong type: {e:#}");
            })
            .ok()
    });
    Ok((guest, list_items))
}

/// Keep at most `MAX_ITEMS` entries with a usable id, every field cut to its limit.
fn bound_items(items: Vec<types::Item>) -> Vec<PickItem> {
    let cut = |s: String, max: usize| match s.len() <= max {
        true => s,
        false => truncate(&s, max).to_string(),
    };
    let text = |s: Option<String>| s.map(|s| cut(s, MAX_ITEM_TEXT));
    items
        .into_iter()
        .filter(|i| !i.id.is_empty() && i.id.len() <= MAX_ITEM_ID)
        .take(MAX_ITEMS)
        .map(|i| PickItem {
            id: i.id,
            label: cut(i.label, MAX_ITEM_TEXT),
            subtitle: text(i.subtitle),
            group: text(i.group),
            icon: i.icon.map(|s| cut(s, MAX_ITEM_ICON)),
            state: text(i.state),
        })
        .collect()
}

/// The instance thread: instantiate, then one guest call per event until stopped or a trap.
fn run(
    mut store: Store<HostState>,
    pre: &PluginBasePre<HostState>,
    list_items: Option<ComponentExportIndex>,
    rx: Receiver<Event>,
) -> Exit {
    let plugin = Arc::clone(&store.data().plugin);
    let stop = Arc::clone(&store.data().stop);
    let name = store.data().manifest.name.clone();
    let refusal = |exit: &Exit| match exit {
        Exit::Stopped => PluginError::Off(name.clone()).to_string(),
        Exit::Crashed(_) => PluginError::Failed(name.clone()).to_string(),
    };

    arm(&mut store);
    let instantiated = instantiate(&mut store, pre, list_items);
    store.data_mut().flush_secrets();
    let mut unanswered = None;
    let exit = match instantiated {
        Err(e) => ended(&e),
        Ok((guest, list_items)) => loop {
            if stop.load(Ordering::Relaxed) {
                break Exit::Stopped;
            }
            let event = match rx.recv_timeout(STOP_POLL) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break Exit::Stopped,
            };
            if stop.load(Ordering::Relaxed) {
                unanswered = Some(event);
                break Exit::Stopped;
            }
            arm(&mut store);
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
                        let _ = reply.send(Err(refusal(&ended(&e))));
                        Err(e)
                    }
                },
                Event::ListItems {
                    picker,
                    query,
                    group,
                    reply,
                } => match &list_items {
                    None => {
                        let _ = reply.send(Err(format!("{name} has no list to pick from")));
                        Ok(())
                    }
                    Some(f) => match f.call(s, (picker, query, group)) {
                        Ok((answer,)) => {
                            let answer = answer
                                .map(bound_items)
                                .map_err(|e| truncate(&e, MAX_ITEM_ERROR).to_string());
                            let _ = reply.send(answer);
                            Ok(())
                        }
                        Err(e) => {
                            let _ = reply.send(Err(refusal(&ended(&e))));
                            Err(e)
                        }
                    },
                },
                // Socket events reach the plugin only while it has the socket open.
                Event::Open(socket) => match s.data().net.socket_open(socket) {
                    true => guest.call_on_open(s, socket),
                    false => Ok(()),
                },
                Event::Message(socket, data) => match s.data().net.message_taken(socket, &data) {
                    true => guest.call_on_message(s, socket, &data),
                    false => Ok(()),
                },
                Event::Closed(socket, reason) => match s.data_mut().net.socket_closed(socket) {
                    true => guest.call_on_closed(s, socket, &reason),
                    false => Ok(()),
                },
                Event::Http(id, response) => {
                    s.data_mut().net.fetch_done(id, &response);
                    guest.call_on_http(s, id, response.as_ref().map_err(String::as_str))
                }
                Event::Timer(id) => match s.data_mut().timer_due(id) {
                    true => guest.call_on_timer(s, id),
                    false => Ok(()),
                },
            };
            // Secrets set before a trap still count.
            store.data_mut().flush_secrets();
            if let Err(e) = called {
                break ended(&e);
            }
        },
    };

    if let Exit::Crashed(why) = &exit {
        tracing::warn!(target: "plugin", plugin = %plugin, "crashed: {why}");
    }
    // Nothing queued runs any more; actions waiting on a reply get the reason.
    let message = refusal(&exit);
    for event in unanswered.into_iter().chain(rx.try_iter()) {
        match event {
            Event::Action { reply, .. } => {
                let _ = reply.send(Err(message.clone()));
            }
            Event::ListItems { reply, .. } => {
                let _ = reply.send(Err(message.clone()));
            }
            _ => {}
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

/// At most `max` bytes of `s`, cut at a character boundary.
pub(super) fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
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
