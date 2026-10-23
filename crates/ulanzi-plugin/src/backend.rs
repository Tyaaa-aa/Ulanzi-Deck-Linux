//! Where `Cx` calls go: the wit-bindgen imports on wasm32, a thread-local fake host elsewhere.
#[cfg(target_arch = "wasm32")]
pub(crate) mod host {
    use crate::bindings::ulanzi::plugin::{core, net, types};
    use crate::{HttpRequest, Level, Payload, Status};
    use std::time::Duration;

    fn level(l: Level) -> types::LogLevel {
        match l {
            Level::Debug => types::LogLevel::Debug,
            Level::Info => types::LogLevel::Info,
            Level::Warn => types::LogLevel::Warn,
            Level::Error => types::LogLevel::Error,
        }
    }

    pub fn set_timer(after: Duration, repeat: bool) -> u32 {
        core::set_timer(u32::try_from(after.as_millis()).unwrap_or(u32::MAX), repeat)
    }
    pub fn cancel_timer(id: u32) {
        core::cancel_timer(id);
    }
    pub fn set_state(key: &str, value: &str) {
        core::set_state(key, value);
    }
    pub fn secret_get(key: &str) -> Option<String> {
        core::secret_get(key)
    }
    pub fn secret_set(key: &str, value: &str) {
        core::secret_set(key, value);
    }
    pub fn secret_delete(key: &str) {
        core::secret_delete(key);
    }
    pub fn set_status(s: Status, message: &str) {
        let s = match s {
            Status::Ok => types::Status::Ok,
            Status::Connecting => types::Status::Connecting,
            Status::Problem => types::Status::Problem,
        };
        core::set_status(s, message);
    }
    pub fn log(l: Level, message: &str) {
        core::log(level(l), message);
    }
    pub fn ws_open(url: &str) -> Result<u32, String> {
        net::ws_open(url)
    }
    pub fn tcp_open(host: &str, port: u16, tls: bool) -> Result<u32, String> {
        net::tcp_open(host, port, tls)
    }
    pub fn send(socket: u32, data: Payload) -> Result<(), String> {
        net::send(
            socket,
            &match data {
                Payload::Text(t) => types::Payload::Text(t),
                Payload::Binary(b) => types::Payload::Binary(b),
            },
        )
    }
    pub fn close(socket: u32) {
        net::close(socket);
    }
    pub fn fetch(req: HttpRequest) -> Result<u32, String> {
        net::fetch(&types::HttpRequest {
            method: req.method,
            url: req.url,
            headers: req.headers,
            body: req.body,
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use fake::*;

#[cfg(not(target_arch = "wasm32"))]
mod fake {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::time::Duration;

    use crate::{HttpRequest, Level, Payload, Request, Socket, Status, Timer};

    /// Everything the fake host remembers. Ids start at 1 and count up per kind.
    #[derive(Default)]
    pub struct Fake {
        pub state: BTreeMap<String, String>,
        pub secrets: BTreeMap<String, String>,
        pub opened: Vec<(Socket, String)>,
        pub sent: BTreeMap<u32, Vec<Payload>>,
        pub fetched: Vec<(Request, HttpRequest)>,
        pub timers: Vec<(Timer, Duration, bool)>,
        pub status: Option<(Status, String)>,
        pub logs: Vec<(Level, String)>,
        next_socket: u32,
        next_timer: u32,
        next_request: u32,
    }

    thread_local! {
        static FAKE: RefCell<Fake> = RefCell::new(Fake::default());
    }

    pub fn install(f: Fake) {
        FAKE.with(|c| *c.borrow_mut() = f);
    }
    pub fn take() -> Fake {
        FAKE.with(|c| std::mem::take(&mut *c.borrow_mut()))
    }
    pub fn with<R>(f: impl FnOnce(&mut Fake) -> R) -> R {
        FAKE.with(|c| f(&mut c.borrow_mut()))
    }

    impl Fake {
        pub fn new_socket(&mut self, label: String) -> u32 {
            self.next_socket += 1;
            self.opened.push((Socket(self.next_socket), label));
            self.next_socket
        }
        pub fn new_timer(&mut self, after: Duration, repeat: bool) -> u32 {
            self.next_timer += 1;
            self.timers.push((Timer(self.next_timer), after, repeat));
            self.next_timer
        }
        pub fn new_request(&mut self, req: HttpRequest) -> u32 {
            self.next_request += 1;
            self.fetched.push((Request(self.next_request), req));
            self.next_request
        }
    }
}
