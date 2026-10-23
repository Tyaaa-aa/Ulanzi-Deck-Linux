//! The wasmtime engine, the shared linker and plugin loading.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use ulanzi_core::plugin::Manifest;
use wasmtime::component::{Component, HasData, Linker, ResourceTable};
use wasmtime::{Cache, CacheConfig, Config, Engine, StoreLimits};
use wasmtime_wasi::p2::bindings::{cli, clocks, random};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use super::PluginError;
use super::ulanzi::plugin::{core, net, types};

/// How often the epoch advances; guest call deadlines are counted in these ticks.
const EPOCH_TICK: Duration = Duration::from_millis(10);

/// The `Store` data type every plugin instance runs with. Task 5 (instance.rs) extends this
/// struct (event queue, timers, sockets, secrets, state sink, plugin id, ...) and replaces the
/// stub `core`/`net` bodies below; `PluginEngine` only needs `HostState: WasiView + core::Host +
/// net::Host + Send`.
#[allow(dead_code)] // fields are read once instances exist (Task 5)
pub struct HostState {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: StoreLimits,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// Linking-only stubs: Task 5 implements the contract for real. The generated traits cannot
// trap, so until then every call is a harmless no-op returning a default (nothing is wired).
impl types::Host for HostState {}

impl core::Host for HostState {
    fn set_timer(&mut self, _ms: u32, _repeat: bool) -> u32 {
        0
    }
    fn cancel_timer(&mut self, _id: u32) {}
    fn set_state(&mut self, _key: String, _value: String) {}
    fn secret_get(&mut self, _key: String) -> Option<String> {
        None
    }
    fn secret_set(&mut self, _key: String, _value: String) {}
    fn secret_delete(&mut self, _key: String) {}
    fn set_status(&mut self, _status: types::Status, _message: String) {}
    fn log(&mut self, _level: types::LogLevel, _message: String) {}
}

impl net::Host for HostState {
    fn ws_open(&mut self, _url: String) -> Result<u32, String> {
        Err("not available yet".into())
    }
    fn tcp_open(&mut self, _host: String, _port: u16, _tls: bool) -> Result<u32, String> {
        Err("not available yet".into())
    }
    fn send(&mut self, _socket: u32, _data: types::Payload) -> Result<(), String> {
        Err("not available yet".into())
    }
    fn close(&mut self, _socket: u32) {}
    fn fetch(&mut self, _request: types::HttpRequest) -> Result<u32, String> {
        Err("not available yet".into())
    }
}

struct HasIo;
impl HasData for HasIo {
    type Data<'a> = &'a mut ResourceTable;
}

struct HasHost;
impl HasData for HasHost {
    type Data<'a> = &'a mut HostState;
}

/// Link the WASI p2 interfaces std needs, and nothing else: clocks, random, cli stdio/env/exit,
/// io. No filesystem, sockets or http (a component importing them fails to link).
fn add_wasi(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    use wasmtime_wasi::cli::{WasiCli, WasiCliView as _};
    use wasmtime_wasi::clocks::WasiClocks;
    use wasmtime_wasi::clocks::WasiClocksView as _;
    use wasmtime_wasi::p2::bindings::sync::io;
    use wasmtime_wasi::random::WasiRandom;

    let l = linker;
    clocks::wall_clock::add_to_linker::<HostState, WasiClocks>(l, HostState::clocks)?;
    clocks::monotonic_clock::add_to_linker::<HostState, WasiClocks>(l, HostState::clocks)?;
    random::random::add_to_linker::<HostState, WasiRandom>(l, |t| t.ctx().ctx.random())?;
    random::insecure::add_to_linker::<HostState, WasiRandom>(l, |t| t.ctx().ctx.random())?;
    random::insecure_seed::add_to_linker::<HostState, WasiRandom>(l, |t| t.ctx().ctx.random())?;
    cli::exit::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::environment::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::stdin::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::stdout::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::stderr::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::terminal_input::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::terminal_output::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::terminal_stdin::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::terminal_stdout::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    cli::terminal_stderr::add_to_linker::<HostState, WasiCli>(l, HostState::cli)?;
    wasmtime_wasi_io::bindings::wasi::io::error::add_to_linker::<HostState, HasIo>(l, |t| t.ctx().table)?;
    io::poll::add_to_linker::<HostState, HasIo>(l, |t| t.ctx().table)?;
    io::streams::add_to_linker::<HostState, HasIo>(l, |t| t.ctx().table)?;
    Ok(())
}

/// Stops the epoch ticker when the last `PluginEngine` clone is dropped.
struct Ticker(JoinHandle<()>);

impl Drop for Ticker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The shared wasmtime engine and linker. Cheap to clone.
#[derive(Clone)]
pub struct PluginEngine {
    engine: Engine,
    linker: Arc<Linker<HostState>>,
    ticker: Arc<Ticker>,
}

impl PluginEngine {
    /// Epoch interruption on, compiled code cached in `cache_dir` (used as given; the caller
    /// passes the final directory, e.g. `<Store::cache_dir()>/plugins`). Spawns the 10 ms epoch
    /// ticker on the current tokio runtime, so it must be called inside one.
    pub fn new(cache_dir: &Path) -> Result<PluginEngine, PluginError> {
        let load = |e: wasmtime::Error| PluginError::Load(format!("{e:#}"));
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|e| PluginError::Load(format!("no tokio runtime: {e}")))?;

        let mut cache_cfg = CacheConfig::new();
        cache_cfg.with_directory(cache_dir);
        let cache = Cache::new(cache_cfg).map_err(load)?;
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.cache(Some(cache));
        let engine = Engine::new(&config).map_err(load)?;

        let mut linker = Linker::<HostState>::new(&engine);
        add_wasi(&mut linker).map_err(load)?;
        core::add_to_linker::<HostState, HasHost>(&mut linker, |s| s).map_err(load)?;
        net::add_to_linker::<HostState, HasHost>(&mut linker, |s| s).map_err(load)?;

        let weak = engine.weak();
        let ticker = handle.spawn(async move {
            let mut tick = tokio::time::interval(EPOCH_TICK);
            // After a stall, do not fire a burst of catch-up ticks: they would trap healthy calls.
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                match weak.upgrade() {
                    Some(engine) => engine.increment_epoch(),
                    None => break,
                }
            }
        });
        Ok(PluginEngine {
            engine,
            linker: Arc::new(linker),
            ticker: Arc::new(Ticker(ticker)),
        })
    }

    /// Compile the component and check its imports against the manifest.
    pub fn load(&self, manifest: &Manifest, wasm: &Path) -> Result<LoadedPlugin, PluginError> {
        let bytes = std::fs::read(wasm)
            .map_err(|e| PluginError::Io(format!("cannot read {}: {e}", wasm.display())))?;
        let component = Component::new(&self.engine, &bytes)
            .map_err(|e| PluginError::Load(format!("{} is not a valid plugin: {e:#}", manifest.name)))?;

        let uses_net = component
            .component_type()
            .imports(&self.engine)
            .any(|(name, _)| name.starts_with("ulanzi:plugin/net@"));
        if uses_net && !manifest.has_network() {
            return Err(PluginError::Permission(format!(
                "{} uses the network but does not ask for it",
                manifest.name
            )));
        }

        let pre = self.linker.instantiate_pre(&component).map_err(|e| {
            PluginError::Load(format!(
                "{} needs something Ulanzi Deck does not provide: {e:#}",
                manifest.name
            ))
        })?;
        Ok(LoadedPlugin {
            manifest: manifest.clone(),
            component,
            pre,
            _ticker: Arc::clone(&self.ticker),
        })
    }

    #[allow(dead_code)] // used by instance creation (Task 5)
    pub(crate) fn engine(&self) -> &Engine {
        &self.engine
    }
}

/// A compiled plugin that links against the host; Task 5 instantiates it.
#[allow(dead_code)] // `component` and `pre` are used by instance creation (Task 5)
pub struct LoadedPlugin {
    pub manifest: Manifest,
    pub(crate) component: Component,
    pub(crate) pre: wasmtime::component::InstancePre<HostState>,
    /// Keeps the epoch ticker alive for as long as the plugin is.
    _ticker: Arc<Ticker>,
}
