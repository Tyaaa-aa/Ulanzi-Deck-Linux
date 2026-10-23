//! Ulanzi Deck plugin contract (`ulanzi:plugin@1.0.0`). The WIT lives in `wit/plugin.wit`.
//!
//! Write a plugin by implementing [`Plugin`] and calling [`export_plugin!`] in the crate root;
//! build it with `cargo build --release --target wasm32-wasip2`. Test it natively with
//! [`testing::Harness`].
mod backend;
#[cfg(not(target_arch = "wasm32"))]
pub mod testing;

use std::time::Duration;

pub use serde_json::{Value, json};

/// The generated component bindings (wasm32 only). Used by [`export_plugin!`]; not a stable API.
#[cfg(target_arch = "wasm32")]
#[doc(hidden)]
#[allow(unsafe_code, clippy::all, missing_docs)]
pub mod bindings {
    // The generated export glue contains `unsafe`; it is machine-written, so it is allowed here only.
    wit_bindgen::generate!({
        path: "wit",
        world: "plugin",
        pub_export_macro: true,
        export_macro_name: "__export_plugin",
        default_bindings_module: "ulanzi_plugin::bindings",
    });
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

/// The plugin's connection status, shown next to it in the app.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Connecting,
    Problem,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Socket(pub u32);
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Timer(pub u32);
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Request(pub u32);

/// A plugin. One instance lives for the life of the component; every method is called by the host
/// one at a time.
#[allow(unused_variables)]
pub trait Plugin: Default + 'static {
    /// Called at start and whenever the plugin's settings change.
    fn init(&mut self, cx: &mut Cx, settings: Value) -> Result<(), String> {
        Ok(())
    }
    /// A key pressed one of the plugin's actions. An error is shown to the user.
    fn action(&mut self, cx: &mut Cx, action: &str, params: Value) -> Result<(), String>;
    fn opened(&mut self, cx: &mut Cx, socket: Socket) {}
    fn message(&mut self, cx: &mut Cx, socket: Socket, data: Payload) {}
    fn closed(&mut self, cx: &mut Cx, socket: Socket, reason: &str) {}
    fn http(&mut self, cx: &mut Cx, request: Request, response: Result<HttpResponse, String>) {}
    fn timer(&mut self, cx: &mut Cx, timer: Timer) {}
}

/// Handle to the host. Zero-sized: every call goes straight to the host (or the test fake).
pub struct Cx(());

impl Cx {
    pub(crate) fn new() -> Self {
        Cx(())
    }

    /// For [`export_plugin!`] only.
    #[doc(hidden)]
    pub fn __new() -> Self {
        Self::new()
    }

    pub fn set_timer(&mut self, after: Duration, repeat: bool) -> Timer {
        #[cfg(target_arch = "wasm32")]
        return Timer(backend::host::set_timer(after, repeat));
        #[cfg(not(target_arch = "wasm32"))]
        return Timer(backend::with(|f| f.new_timer(after, repeat)));
    }

    pub fn cancel_timer(&mut self, t: Timer) {
        #[cfg(target_arch = "wasm32")]
        backend::host::cancel_timer(t.0);
        #[cfg(not(target_arch = "wasm32"))]
        backend::with(|f| f.timers.retain(|(id, ..)| *id != t));
    }

    /// Live state; shown on keys as `{<plugin-id>.<key>}`.
    pub fn set_state(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        #[cfg(target_arch = "wasm32")]
        backend::host::set_state(key, &value);
        #[cfg(not(target_arch = "wasm32"))]
        backend::with(|f| f.state.insert(key.to_owned(), value));
    }

    pub fn secret(&mut self, key: &str) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        return backend::host::secret_get(key);
        #[cfg(not(target_arch = "wasm32"))]
        return backend::with(|f| f.secrets.get(key).cloned());
    }

    pub fn set_secret(&mut self, key: &str, value: &str) {
        #[cfg(target_arch = "wasm32")]
        backend::host::secret_set(key, value);
        #[cfg(not(target_arch = "wasm32"))]
        backend::with(|f| f.secrets.insert(key.to_owned(), value.to_owned()));
    }

    pub fn delete_secret(&mut self, key: &str) {
        #[cfg(target_arch = "wasm32")]
        backend::host::secret_delete(key);
        #[cfg(not(target_arch = "wasm32"))]
        backend::with(|f| f.secrets.remove(key));
    }

    pub fn set_status(&mut self, s: Status, message: &str) {
        #[cfg(target_arch = "wasm32")]
        backend::host::set_status(s, message);
        #[cfg(not(target_arch = "wasm32"))]
        backend::with(|f| f.status = Some((s, message.to_owned())));
    }

    pub fn log(&mut self, level: Level, message: &str) {
        #[cfg(target_arch = "wasm32")]
        backend::host::log(level, message);
        #[cfg(not(target_arch = "wasm32"))]
        backend::with(|f| f.logs.push((level, message.to_owned())));
    }

    /// Needs the `network` or `lan` permission. The result arrives as `opened` or `closed`.
    pub fn ws_open(&mut self, url: &str) -> Result<Socket, String> {
        #[cfg(target_arch = "wasm32")]
        return backend::host::ws_open(url).map(Socket);
        #[cfg(not(target_arch = "wasm32"))]
        return Ok(Socket(backend::with(|f| f.new_socket(url.to_owned()))));
    }

    pub fn tcp_open(&mut self, host: &str, port: u16, tls: bool) -> Result<Socket, String> {
        #[cfg(target_arch = "wasm32")]
        return backend::host::tcp_open(host, port, tls).map(Socket);
        #[cfg(not(target_arch = "wasm32"))]
        return Ok(Socket(backend::with(|f| {
            f.new_socket(format!("tcp://{host}:{port}{}", if tls { "+tls" } else { "" }))
        })));
    }

    pub fn send(&mut self, s: Socket, data: Payload) -> Result<(), String> {
        #[cfg(target_arch = "wasm32")]
        return backend::host::send(s.0, data);
        #[cfg(not(target_arch = "wasm32"))]
        return backend::with(|f| {
            if f.opened.iter().any(|(id, _)| *id == s) {
                f.sent.entry(s.0).or_default().push(data);
                Ok(())
            } else {
                Err(format!("unknown socket {}", s.0))
            }
        });
    }

    pub fn close(&mut self, s: Socket) {
        #[cfg(target_arch = "wasm32")]
        backend::host::close(s.0);
        #[cfg(not(target_arch = "wasm32"))]
        let _ = s;
    }

    /// Needs the `network` or `lan` permission. The result arrives as `http`.
    pub fn fetch(&mut self, req: HttpRequest) -> Result<Request, String> {
        #[cfg(target_arch = "wasm32")]
        return backend::host::fetch(req).map(Request);
        #[cfg(not(target_arch = "wasm32"))]
        return Ok(Request(backend::with(|f| f.new_request(req))));
    }
}

/// Plumbing for [`export_plugin!`].
#[doc(hidden)]
pub mod __private {
    use super::{Cx, Plugin, Value};
    use std::cell::RefCell;
    use std::thread::LocalKey;

    /// Run `f` on the plugin held in `slot`, creating it on first use.
    pub fn with<P: Plugin, R>(
        slot: &'static LocalKey<RefCell<Option<P>>>,
        f: impl FnOnce(&mut P, &mut Cx) -> R,
    ) -> R {
        slot.with(|cell| {
            let mut guard = cell.borrow_mut();
            f(guard.get_or_insert_with(P::default), &mut Cx::__new())
        })
    }

    pub fn parse(json: &str) -> Result<Value, String> {
        serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))
    }
}

/// Export a [`Plugin`] as the component. Call it once, in the plugin crate root:
/// `ulanzi_plugin::export_plugin!(MyPlugin);`. Expands to nothing on native targets.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        #[cfg(target_arch = "wasm32")]
        #[allow(unsafe_code)] // the generated export glue contains `unsafe`
        mod __ulanzi_plugin_export {
            #[allow(unused_imports)]
            use super::*;
            use $crate::__private::{parse, with};
            use $crate::bindings::ulanzi::plugin::types as wit;
            use $crate::{HttpResponse, Payload, Plugin, Request, Socket, Timer};
            use ::std::cell::RefCell;

            // The user's type, named from the crate root.
            type User = $plugin;

            ::std::thread_local! {
                static PLUGIN: RefCell<Option<User>> = const { RefCell::new(None) };
            }

            struct Adapter;

            impl $crate::bindings::Guest for Adapter {
                fn init(settings: String) -> Result<(), String> {
                    let settings = parse(&settings)?;
                    with(&PLUGIN, |p, cx| p.init(cx, settings))
                }
                fn on_action(action: String, params: String) -> Result<(), String> {
                    let params = parse(&params)?;
                    with(&PLUGIN, |p, cx| p.action(cx, &action, params))
                }
                fn on_open(socket: u32) {
                    with(&PLUGIN, |p, cx| p.opened(cx, Socket(socket)));
                }
                fn on_message(socket: u32, data: wit::Payload) {
                    let data = match data {
                        wit::Payload::Text(t) => Payload::Text(t),
                        wit::Payload::Binary(b) => Payload::Binary(b),
                    };
                    with(&PLUGIN, |p, cx| p.message(cx, Socket(socket), data));
                }
                fn on_closed(socket: u32, reason: String) {
                    with(&PLUGIN, |p, cx| p.closed(cx, Socket(socket), &reason));
                }
                fn on_http(request: u32, response: Result<wit::HttpResponse, String>) {
                    let response = response.map(|r| HttpResponse {
                        status: r.status,
                        headers: r.headers,
                        body: r.body,
                    });
                    with(&PLUGIN, |p, cx| p.http(cx, Request(request), response));
                }
                fn on_timer(timer: u32) {
                    with(&PLUGIN, |p, cx| p.timer(cx, Timer(timer)));
                }
            }

            $crate::bindings::__export_plugin!(Adapter with_types_in $crate::bindings);
        }
    };
}
