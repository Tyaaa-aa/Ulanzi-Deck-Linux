//! A tokio runtime beside GTK's main loop. Async work (the D-Bus client) runs here; results come back
//! to the GTK thread through `glib::spawn_future_local`.

use std::future::Future;
use std::sync::OnceLock;

use gtk::glib;

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap_or_else(|e| {
                eprintln!("ulanzi-deck: cannot start the async runtime: {e}");
                std::process::exit(1)
            })
    })
}

/// Run `fut` on tokio and hand its output to `done` on the GTK thread.
pub fn run<T, F, D>(fut: F, done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let handle = runtime().spawn(fut);
    glib::spawn_future_local(async move {
        if let Ok(v) = handle.await {
            done(v);
        }
    });
}
