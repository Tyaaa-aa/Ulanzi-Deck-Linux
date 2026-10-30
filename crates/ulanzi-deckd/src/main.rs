//! `ulanzi-deckd`: the Ulanzi Deck background service.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{broadcast, mpsc};
use ulanzi_core::model::Settings;
use ulanzi_core::store::{ProfileLoad, Store};
use ulanzi_deckd::actions::Executor;
use ulanzi_deckd::dbus::{BUS_NAME, DeckService, OBJECT_PATH, forward_signals, reload};
use ulanzi_deckd::deck::{DeckEvent, DeckMsg, start_deck};
use ulanzi_deckd::plugins::{FileSecrets, LazySecrets, PluginHost, PluginSlot, SecretStore};
use ulanzi_deckd::screen::Sampler;
use ulanzi_deckd::transport::{DeviceState, Hidraw, probe, watch_dev};

const EXIT_ALREADY_RUNNING: u8 = 3;
const SAMPLE_EVERY: Duration = Duration::from_secs(1);

fn usage() -> ! {
    eprintln!(
        "usage: ulanzi-deckd [--replace]\n\n  --replace   take over from an already running ulanzi-deckd"
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut replace = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--replace" => replace = true,
            "--version" => {
                println!("ulanzi-deckd {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => usage(),
        }
    }

    let store = match Store::xdg() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ulanzi-deckd: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _log_guard = init_logging(store.state_dir());

    // Connect, publish the API, then take the bus name, so a call that D-Bus activation queued for
    // us never arrives before the object exists. A second instance exits at the name request,
    // before it touches the deck or the configuration.
    let conn = match zbus::connection::Builder::session()
        // A frozen media player must not stall the deck: every D-Bus call gets a time limit.
        .map(|b| b.method_timeout(std::time::Duration::from_secs(2)))
    {
        Ok(b) => match b.build().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("ulanzi-deckd: cannot connect to the session bus: {e}");
                return ExitCode::FAILURE;
            }
        },
        Err(e) => {
            eprintln!("ulanzi-deckd: cannot connect to the session bus: {e}");
            return ExitCode::FAILURE;
        }
    };

    let quit = Arc::new(tokio::sync::Notify::new());
    // Calls that arrive before the deck starts wait in this channel.
    let (tx, rx) = mpsc::channel(256);
    if let Err(e) = conn
        .object_server()
        .at(
            OBJECT_PATH,
            DeckService::new(tx.clone(), store.clone())
                .with_plugins_dir(plugins_dir(&store))
                .with_quit(quit.clone()),
        )
        .await
    {
        tracing::error!("cannot publish the D-Bus API: {e}");
        return ExitCode::FAILURE;
    }
    let mut flags = zbus::fdo::RequestNameFlags::AllowReplacement | zbus::fdo::RequestNameFlags::DoNotQueue;
    if replace {
        flags |= zbus::fdo::RequestNameFlags::ReplaceExisting;
    }
    match conn.request_name_with_flags(BUS_NAME, flags).await {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner) => {}
        Ok(_) | Err(zbus::Error::NameTaken) => {
            eprintln!("ulanzi-deckd is already running (use --replace to take over)");
            return ExitCode::from(EXIT_ALREADY_RUNNING);
        }
        Err(e) => {
            eprintln!("ulanzi-deckd: cannot take the bus name: {e}");
            return ExitCode::FAILURE;
        }
    }
    tracing::info!("ulanzi-deckd {} starting", env!("CARGO_PKG_VERSION"));

    first_run(&store);
    // Version 2.1 reads 2.0's files and saves them in its own format: keep the originals once.
    match store.keep_v1_copy() {
        Ok(Some(dir)) => tracing::info!(
            "kept a copy of the 2.0 settings and profiles in {}",
            dir.display()
        ),
        Ok(None) => {}
        Err(e) => tracing::warn!("cannot keep a copy of the 2.0 settings and profiles: {e}"),
    }
    if ulanzi_core::snapshot::due_daily(&store)
        && let Err(e) = ulanzi_core::snapshot::take(&store, "daily")
    {
        tracing::warn!("daily backup failed: {e}");
    }
    let settings = store.load_settings().unwrap_or_else(|e| {
        tracing::error!("settings unreadable, using defaults and leaving the file alone: {e}");
        store.protect_settings();
        Default::default()
    });
    ulanzi_deckd::actions::audio::set_sound_volume(settings.sound_volume);
    // Before the plugins start: nothing after them may return without shutting them down.
    let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        tracing::error!("cannot install signal handlers");
        return ExitCode::FAILURE;
    };
    // The deck does not wait for the plugins: they come up in the background, and a plugin key
    // pressed meanwhile says so.
    let plugins = {
        let (store, settings, deck) = (store.clone(), settings.clone(), tx.clone());
        PluginSlot::start(async move { start_plugins(&store, &settings, deck).await })
    };
    tokio::spawn(plugins_ready(conn.clone(), plugins.clone(), store.clone()));
    tokio::spawn(ulanzi_deckd::updates::run(store.clone()));
    let executor = Arc::new(Executor::new(Some(conn.clone())).with_plugin_slot(plugins.clone()));
    {
        let ex = executor.clone();
        tokio::task::spawn_blocking(move || ex.prepare_keyboard());
    }
    let media = executor.media().cloned();
    let events = start_deck(store.clone(), settings, executor, rx);

    tokio::spawn(forward_signals(conn.clone(), events.resubscribe()));
    tokio::spawn(ulanzi_deckd::tray::run(
        tx.clone(),
        events.resubscribe(),
        store.clone(),
        quit.clone(),
    ));
    if std::env::var_os("ULANZI_DEMO").is_some() {
        // A pretend deck: everything works but nothing touches the hardware (screenshots, tests).
        tracing::info!("demo mode: using a pretend deck");
        tokio::spawn(demo_deck(tx.clone()));
    } else {
        tokio::spawn(hotplug(tx.clone(), events));
    }
    tokio::spawn(ulanzi_deckd::focus::run(
        conn.clone(),
        store.state_dir().to_path_buf(),
        tx.clone(),
    ));
    tokio::spawn(sample_loop(
        tx.clone(),
        Sampler::new(media, store.cache_dir().to_path_buf()),
    ));

    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = int.recv() => break,
            _ = quit.notified() => {
                tracing::info!("quit from the tray");
                break;
            }
            _ = hup.recv() => {
                tracing::info!("reloading configuration");
                reload(&tx, &store).await;
                if let (Some(host), Ok(s)) = (plugins.get(), store.load_settings()) {
                    host.apply_settings(&s).await;
                }
            }
        }
    }
    tracing::info!("stopping");
    ulanzi_deckd::focus::kwin::unload(&conn).await;
    let _ = tx.send(DeckMsg::Shutdown).await;
    plugins.shutdown().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    ExitCode::SUCCESS
}

/// Once the plugin host is ready: offer it on D-Bus, signal the list, and give it settings saved
/// while it was starting.
async fn plugins_ready(conn: zbus::Connection, plugins: PluginSlot, store: Store) {
    let Some(host) = plugins.wait().await else {
        return;
    };
    // Holding the service while the settings are read and applied: an ApplySettings call waits,
    // so its newer settings cannot be overwritten by the ones read here.
    let iface = conn
        .object_server()
        .interface::<_, DeckService>(OBJECT_PATH)
        .await;
    let mut service = match &iface {
        Ok(iface) => Some(iface.get_mut().await),
        Err(e) => {
            tracing::error!("plugins are not on D-Bus: {e}");
            None
        }
    };
    if let Some(service) = service.as_mut() {
        service.set_plugins(host.clone());
    }
    if let Ok(mut s) = store.load_settings() {
        if host.adopt_secrets(&mut s).await
            && let Err(e) = store.save_settings(&s)
        {
            tracing::warn!("cannot save the settings without the moved secrets: {e}");
        }
        host.apply_settings(&s).await;
    }
    drop(service);
    ulanzi_deckd::dbus::forward_plugins(conn, host).await;
}

/// Where plugins are installed: `<data folder>/plugins`.
fn plugins_dir(store: &Store) -> PathBuf {
    match ulanzi_core::store::data_dir() {
        Ok(d) => d.join("plugins"),
        Err(e) => {
            tracing::error!("no data folder ({e}); plugins are kept in the config folder");
            store.root().join("plugins")
        }
    }
}

/// Where plugins keep their secrets: the system keyring, asked only when a plugin first needs its
/// secrets. A pretend deck (`ULANZI_DEMO`) runs in a throwaway sandbox whose private D-Bus session
/// could still start a keyring daemon that opens your own keyring, so it uses a file in the
/// sandbox's config folder instead.
fn plugin_secrets(store: &Store) -> Arc<dyn SecretStore> {
    if std::env::var_os("ULANZI_DEMO").is_some() {
        Arc::new(FileSecrets::new(store.root().join("plugin-secrets.toml")))
    } else {
        Arc::new(LazySecrets::system(store.root()))
    }
}

/// Load the installed plugins and start the ones that run in the background or that a profile
/// uses (the others start on their first action).
async fn start_plugins(store: &Store, settings: &Settings, deck: mpsc::Sender<DeckMsg>) -> PluginHost {
    let dir = plugins_dir(store);
    let used: BTreeSet<String> = store
        .list_profiles()
        .into_iter()
        .filter_map(|l| match l {
            ProfileLoad::Ok(p) => Some(p.plugins_used()),
            _ => None,
        })
        .flatten()
        .collect();
    PluginHost::start(
        dir,
        store.cache_dir().join("plugins"),
        plugin_secrets(store),
        settings,
        &used,
        deck,
    )
    .await
}

/// Import the Python version's config the first time, if there are no profiles yet.
fn first_run(store: &Store) {
    if !store.list_profiles().is_empty() || ulanzi_core::migrate::already_migrated(store) {
        return;
    }
    let Some(path) = ulanzi_core::migrate::legacy_config_path().filter(|p| p.is_file()) else {
        return;
    };
    match ulanzi_core::migrate::migrate_legacy(&path, store, false) {
        Ok(r) => {
            tracing::info!("imported {} keys from {}", r.keys, path.display());
            for w in r.warnings {
                tracing::warn!("import: {w}");
            }
        }
        Err(e) => tracing::error!("could not import {}: {e}", path.display()),
    }
}

/// Watch for the deck appearing, disappearing or changing permissions, and (re)connect. A connection
/// that dies within a few seconds backs off (1 s, 2 s, 4 s ... 30 s) so a wedged deck does not cause a
/// reconnect storm.
async fn hotplug(tx: mpsc::Sender<DeckMsg>, mut events: broadcast::Receiver<DeckEvent>) {
    const SHORT_LIVED: Duration = Duration::from_secs(5);
    const MAX_BACKOFF: Duration = Duration::from_secs(30);
    let mut ticks = watch_dev("/dev".into());
    let mut connected = false;
    let mut connected_at: Option<tokio::time::Instant> = None;
    let mut failures: u32 = 0;
    let mut retry_at = tokio::time::Instant::now();
    let mut last = DeviceState::Error {
        message: "starting".into(),
    };
    loop {
        tokio::select! {
            t = ticks.recv() => if t.is_none() { break },
            ev = events.recv() => match ev {
                Ok(DeckEvent::State(DeviceState::Ready { .. })) => connected = true,
                Ok(DeckEvent::State(_)) => {
                    if connected {
                        let short = connected_at.is_some_and(|t| t.elapsed() < SHORT_LIVED);
                        failures = if short { failures + 1 } else { 0 };
                        let wait = if failures == 0 {
                            Duration::ZERO
                        } else {
                            Duration::from_secs(1 << failures.min(5)).min(MAX_BACKOFF)
                        };
                        if !wait.is_zero() {
                            tracing::warn!("deck connection keeps dropping; retrying in {} s", wait.as_secs());
                        }
                        retry_at = tokio::time::Instant::now() + wait;
                    }
                    connected = false;
                }
                Ok(_) => continue,
                // Missed events: assume nothing and re-probe.
                Err(broadcast::error::RecvError::Lagged(_)) => connected = false,
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
        if connected || tokio::time::Instant::now() < retry_at {
            continue;
        }
        let state = probe(
            Path::new("/sys/class/hidraw"),
            Path::new("/dev"),
            Path::new("/proc"),
        );
        let state = match state {
            DeviceState::Ready { node } => match Hidraw::open(&node) {
                Ok(t) => {
                    tracing::info!("deck connected at {}", node.display());
                    connected = true;
                    connected_at = Some(tokio::time::Instant::now());
                    last = DeviceState::Ready { node: node.clone() };
                    if tx.send(DeckMsg::DeviceUp(Box::new(t), node)).await.is_err() {
                        break;
                    }
                    continue;
                }
                Err(s) => s,
            },
            other => other,
        };
        if state != last {
            match &state {
                DeviceState::Busy { .. } | DeviceState::NoPermission { .. } | DeviceState::Error { .. } => {
                    tracing::warn!("{}", state.detail())
                }
                _ => tracing::info!("{}", state.detail()),
            }
            last = state.clone();
            if tx.send(DeckMsg::DeviceDown(state)).await.is_err() {
                break;
            }
        }
    }
}

async fn demo_deck(tx: mpsc::Sender<DeckMsg>) {
    let (t, handle) = ulanzi_deckd::transport::MockTransport::new();
    let _ = tx.send(DeckMsg::DeviceUp(Box::new(t), "demo".into())).await;
    // The handle keeps the pretend deck open for as long as the service runs; what was "sent" to
    // it is dropped now and then so it doesn't pile up.
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        handle.clear();
    }
}

async fn sample_loop(tx: mpsc::Sender<DeckMsg>, mut sampler: Sampler) {
    let mut t = tokio::time::interval(SAMPLE_EVERY);
    loop {
        t.tick().await;
        let d = sampler.sample().await;
        if tx.send(DeckMsg::Dynamic(Box::new(d))).await.is_err() {
            break;
        }
    }
}

/// Journal-friendly stderr under systemd, a daily-rotated file otherwise.
fn init_logging(state_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("ULANZI_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    if std::env::var_os("JOURNAL_STREAM").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .without_time()
            .with_ansi(false)
            .init();
        return None;
    }
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("daemon")
        .filename_suffix("log")
        .max_log_files(3)
        .build(state_dir);
    match appender {
        Ok(a) => {
            let (w, guard) = tracing_appender::non_blocking(a);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(w)
                .with_ansi(false)
                .init();
            Some(guard)
        }
        Err(_) => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
            None
        }
    }
}
