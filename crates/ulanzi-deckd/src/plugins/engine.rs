//! The wasmtime engine, the shared linker and plugin loading.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use ulanzi_core::plugin::Manifest;
use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, ComponentExportIndex, HasData, Linker, ResourceTable};
use wasmtime::{Cache, CacheConfig, Config, Engine};
use wasmtime_wasi::WasiView;
use wasmtime_wasi::p2::bindings::{cli, clocks, random};

use super::PluginError;
use super::instance::{BoundedPoll, HostState};
use super::ulanzi::plugin::{core, net};

/// How often the epoch advances; guest call deadlines are counted in these ticks.
const EPOCH_TICK: Duration = Duration::from_millis(10);

struct HasIo;
impl HasData for HasIo {
    type Data<'a> = &'a mut ResourceTable;
}

struct HasPoll;
impl HasData for HasPoll {
    type Data<'a> = BoundedPoll<'a>;
}

struct HasHost;
impl HasData for HasHost {
    type Data<'a> = &'a mut HostState;
}

/// Link the WASI p2 interfaces std needs, and nothing else: clocks, random, cli stdio/env/exit,
/// io. No filesystem, sockets or http (a component importing them fails to link). `wasi:io/poll`
/// is the host's own [`BoundedPoll`], so a blocking wait cannot outlast the call budget or a stop.
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
    io::poll::add_to_linker::<HostState, HasPoll>(l, HostState::bounded_poll)?;
    io::streams::add_to_linker::<HostState, HasIo>(l, |t| t.ctx().table)?;
    Ok(())
}

/// Stops the epoch ticker when the last `PluginEngine` clone is dropped.
pub(crate) struct Ticker(JoinHandle<()>);

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
    /// passes the final directory, e.g. `<Store::cache_dir()>/plugins`; when it cannot be used,
    /// with a warning, nothing is cached). Spawns the 10 ms epoch
    /// ticker on the current tokio runtime, so it must be called inside one.
    pub fn new(cache_dir: &Path) -> Result<PluginEngine, PluginError> {
        let load = |e: wasmtime::Error| PluginError::Load(format!("{e:#}"));
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|e| PluginError::Load(format!("no tokio runtime: {e}")))?;

        let mut cache_cfg = CacheConfig::new();
        cache_cfg.with_directory(cache_dir);
        // Without a usable cache folder plugins still run; they are compiled at every start.
        let cache = Cache::new(cache_cfg)
            .inspect_err(|e| {
                let dir = cache_dir.display();
                tracing::warn!("no compile cache for plugins in {dir}: {e:#}");
            })
            .ok();
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.cache(cache);
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
        // Contract 1.1: a component built against 1.0 has no `list-items`.
        let list_items = match component.get_export(None, "list-items") {
            Some((ComponentItem::ComponentFunc(_), index)) => Some(index),
            _ => None,
        };
        Ok(LoadedPlugin {
            manifest: manifest.clone(),
            lists_items: list_items.is_some(),
            list_items,
            component,
            pre,
            ticker: Arc::clone(&self.ticker),
        })
    }

    pub(crate) fn engine(&self) -> &Engine {
        &self.engine
    }
}

/// A compiled plugin that links against the host; `spawn_instance` instantiates it.
pub struct LoadedPlugin {
    pub manifest: Manifest,
    /// The component exports `list-items` (built against contract 1.1 or later).
    pub lists_items: bool,
    pub(crate) list_items: Option<ComponentExportIndex>,
    #[allow(dead_code)] // kept with its pre-instance; nothing reads it directly yet
    pub(crate) component: Component,
    pub(crate) pre: wasmtime::component::InstancePre<HostState>,
    /// Keeps the epoch ticker alive for as long as the plugin is.
    pub(crate) ticker: Arc<Ticker>,
}
