//! `ulanzi-deckd`: the Ulanzi Deck background service.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{broadcast, mpsc};
use ulanzi_core::store::Store;
use ulanzi_deckd::actions::Executor;
use ulanzi_deckd::dbus::{BUS_NAME, DeckService, OBJECT_PATH, forward_signals, reload};
use ulanzi_deckd::deck::{DeckEvent, DeckMsg, spawn_deck};
use ulanzi_deckd::screen::Sampler;
use ulanzi_deckd::transport::{DeviceState, Hidraw, probe, watch_dev};

const EXIT_ALREADY_RUNNING: u8 = 3;
const SAMPLE_EVERY: Duration = Duration::from_secs(2);

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

    // Take the bus name first so a second instance exits before touching anything.
    let conn = match zbus::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .map(|b| b.allow_name_replacements(true).replace_existing_names(replace))
    {
        Ok(b) => match b.build().await {
            Ok(c) => c,
            Err(zbus::Error::NameTaken) => {
                eprintln!("ulanzi-deckd is already running (use --replace to take over)");
                return ExitCode::from(EXIT_ALREADY_RUNNING);
            }
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
    tracing::info!("ulanzi-deckd {} starting", env!("CARGO_PKG_VERSION"));

    first_run(&store);
    let settings = store.load_settings().unwrap_or_else(|e| {
        tracing::error!("settings unreadable, using defaults: {e}");
        Default::default()
    });

    let executor = Arc::new(Executor::new(Some(conn.clone())));
    let media = executor.media().cloned();
    let (tx, events) = spawn_deck(store.clone(), settings, executor);

    if let Err(e) = conn
        .object_server()
        .at(OBJECT_PATH, DeckService::new(tx.clone(), store.clone()))
        .await
    {
        tracing::error!("cannot publish the D-Bus API: {e}");
        return ExitCode::FAILURE;
    }
    tokio::spawn(forward_signals(conn.clone(), events.resubscribe()));
    tokio::spawn(hotplug(tx.clone(), events));
    tokio::spawn(sample_loop(
        tx.clone(),
        Sampler::new(media, store.cache_dir().to_path_buf()),
    ));

    let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        tracing::error!("cannot install signal handlers");
        return ExitCode::FAILURE;
    };
    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = int.recv() => break,
            _ = hup.recv() => {
                tracing::info!("reloading configuration");
                reload(&tx, &store).await;
            }
        }
    }
    tracing::info!("stopping");
    let _ = tx.send(DeckMsg::Shutdown).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    ExitCode::SUCCESS
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

/// Watch for the deck appearing, disappearing or changing permissions, and (re)connect.
async fn hotplug(tx: mpsc::Sender<DeckMsg>, mut events: broadcast::Receiver<DeckEvent>) {
    let mut ticks = watch_dev("/dev".into());
    let mut connected = false;
    let mut last = DeviceState::Error {
        message: "starting".into(),
    };
    loop {
        tokio::select! {
            t = ticks.recv() => if t.is_none() { break },
            ev = events.recv() => match ev {
                Ok(DeckEvent::State(DeviceState::Ready { .. })) => connected = true,
                Ok(DeckEvent::State(_)) => connected = false,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
        if connected {
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

async fn sample_loop(tx: mpsc::Sender<DeckMsg>, mut sampler: Sampler) {
    let mut t = tokio::time::interval(SAMPLE_EVERY);
    loop {
        t.tick().await;
        let d = sampler.sample().await;
        if tx.send(DeckMsg::Dynamic(d)).await.is_err() {
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
