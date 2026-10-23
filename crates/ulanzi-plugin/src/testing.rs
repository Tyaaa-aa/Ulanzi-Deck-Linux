//! Native test harness: drives a [`Plugin`] against a fake host, no WebAssembly needed.
use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::backend::{self, Fake};
use crate::{Cx, HttpRequest, HttpResponse, Payload, Plugin, Request, Socket, Status, Timer};

pub struct Harness<P: Plugin> {
    plugin: P,
    fake: Fake,
}

impl<P: Plugin> Default for Harness<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: Plugin> Harness<P> {
    pub fn new() -> Self {
        Self {
            plugin: P::default(),
            fake: Fake::default(),
        }
    }

    /// Run `f` with this harness's fake host installed, then take the fake back.
    fn run<R>(&mut self, f: impl FnOnce(&mut P, &mut Cx) -> R) -> R {
        backend::install(std::mem::take(&mut self.fake));
        let result = f(&mut self.plugin, &mut Cx::new());
        self.fake = backend::take();
        result
    }

    pub fn init(&mut self, settings: Value) -> Result<(), String> {
        self.run(|p, cx| p.init(cx, settings))
    }
    pub fn action(&mut self, action: &str, params: Value) -> Result<(), String> {
        self.run(|p, cx| p.action(cx, action, params))
    }
    /// Deliver `on-open` for a socket.
    pub fn open(&mut self, s: Socket) {
        self.run(|p, cx| p.opened(cx, s));
    }
    pub fn message(&mut self, s: Socket, data: Payload) {
        self.run(|p, cx| p.message(cx, s, data));
    }
    pub fn closed(&mut self, s: Socket, reason: &str) {
        self.run(|p, cx| p.closed(cx, s, reason));
    }
    pub fn http(&mut self, r: Request, response: Result<HttpResponse, String>) {
        self.run(|p, cx| p.http(cx, r, response));
    }
    /// Fire a timer; a one-shot timer is gone afterwards.
    pub fn fire(&mut self, t: Timer) {
        if let Some(i) = self.fake.timers.iter().position(|(id, ..)| *id == t) {
            if !self.fake.timers[i].2 {
                self.fake.timers.remove(i);
            }
            self.run(|p, cx| p.timer(cx, t));
        }
    }

    pub fn state(&self, key: &str) -> Option<&str> {
        self.fake.state.get(key).map(String::as_str)
    }
    /// Sockets opened so far: the URL, or `tcp://host:port` (`+tls` when encrypted).
    pub fn opened(&self) -> &[(Socket, String)] {
        &self.fake.opened
    }
    pub fn sent(&self, s: Socket) -> &[Payload] {
        self.fake.sent.get(&s.0).map_or(&[], Vec::as_slice)
    }
    pub fn fetched(&self) -> &[(Request, HttpRequest)] {
        &self.fake.fetched
    }
    /// Active timers only.
    pub fn timers(&self) -> Vec<(Timer, Duration, bool)> {
        self.fake.timers.clone()
    }
    pub fn status(&self) -> Option<(Status, String)> {
        self.fake.status.clone()
    }
    pub fn logs(&self) -> &[(crate::Level, String)] {
        &self.fake.logs
    }
    pub fn secrets(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.fake.secrets
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Demo;

    impl Plugin for Demo {
        fn init(&mut self, cx: &mut Cx, _settings: Value) -> Result<(), String> {
            cx.ws_open("ws://x")?;
            cx.set_timer(Duration::from_millis(1000), true);
            cx.set_secret("k", "v");
            Ok(())
        }
        fn action(&mut self, _cx: &mut Cx, _action: &str, _params: Value) -> Result<(), String> {
            Ok(())
        }
        fn opened(&mut self, cx: &mut Cx, socket: Socket) {
            let _ = cx.send(socket, Payload::Text("hi".into()));
        }
    }

    #[test]
    fn fake_host_records_what_the_plugin_does() {
        let mut h = Harness::<Demo>::new();
        h.init(json!({})).unwrap();
        assert_eq!(h.opened()[0], (Socket(1), "ws://x".into()));
        h.open(Socket(1));
        assert_eq!(h.sent(Socket(1)), [Payload::Text("hi".into())]);
        assert_eq!(h.timers(), [(Timer(1), Duration::from_millis(1000), true)]);
        assert_eq!(h.secrets()["k"], "v");
    }

    #[test]
    fn one_shot_timers_are_removed_when_fired() {
        let mut h = Harness::<Demo>::new();
        h.run(|_, cx| {
            cx.set_timer(Duration::from_millis(5), false);
        });
        assert_eq!(h.timers().len(), 1);
        h.fire(Timer(1));
        assert!(h.timers().is_empty());
    }
}
