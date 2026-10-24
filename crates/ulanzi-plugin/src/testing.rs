//! Native test harness: drives a [`Plugin`] against a fake host, no WebAssembly needed.
use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

use crate::backend::{self, Fake};
use crate::{Cx, HttpRequest, HttpResponse, Item, Payload, Plugin, Request, Socket, Status, Timer};

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
    /// Ask for the entries of picker `picker` (contract 1.1), as the host does when a picker opens.
    pub fn list_items(&mut self, picker: &str, query: &str, group: &str) -> Result<Vec<Item>, String> {
        self.run(|p, cx| p.list_items(cx, picker, query, group))
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
    /// The host loses a socket and the `on-closed` notice never arrives (its event queue was full):
    /// the plugin is not told, and every later `send` on it fails.
    pub fn lose_socket(&mut self, s: Socket) {
        self.fake.lost.push(s);
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

    /// Every state key set so far, with its latest value.
    pub fn states(&self) -> &BTreeMap<String, String> {
        &self.fake.state
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
        fn list_items(
            &mut self,
            cx: &mut Cx,
            picker: &str,
            query: &str,
            group: &str,
        ) -> Result<Vec<Item>, String> {
            cx.set_state("asked", picker);
            Ok(vec![
                Item::new("a", format!("{query}/{group}"))
                    .group(group)
                    .state("on"),
            ])
        }
    }

    /// Implements only `action`: `list_items` has the default answer.
    #[derive(Default)]
    struct Bare;

    impl Plugin for Bare {
        fn action(&mut self, _cx: &mut Cx, _action: &str, _params: Value) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn list_items_comes_back() {
        let mut h = Harness::<Demo>::new();
        let items = h.list_items("things", "q", "g").unwrap();
        let want = Item {
            id: "a".into(),
            label: "q/g".into(),
            subtitle: None,
            group: Some("g".into()),
            icon: None,
            state: Some("on".into()),
        };
        assert_eq!(items, [want]);
        assert_eq!(h.state("asked"), Some("things"));
        let mut bare = Harness::<Bare>::new();
        assert_eq!(
            bare.list_items("things", "", ""),
            Err("no list to pick from".to_string())
        );
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
