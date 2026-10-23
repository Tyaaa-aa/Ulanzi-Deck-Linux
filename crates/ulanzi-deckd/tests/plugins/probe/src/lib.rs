//! A test plugin for the host: each action exercises one host function, or misbehaves on purpose
//! (panic, endless loop, huge allocation, flood of host calls).
use std::hint::black_box;
use std::time::Duration;

use ulanzi_plugin::{Cx, HttpRequest, HttpResponse, Payload, Plugin, Request, Socket, Timer, Value};

#[derive(Default)]
pub struct Probe {
    settings: Value,
}

fn text<'a>(params: &'a Value, key: &str, action: &str) -> Result<&'a str, String> {
    params[key]
        .as_str()
        .ok_or_else(|| format!("{action} needs {key}"))
}

impl Plugin for Probe {
    fn init(&mut self, _cx: &mut Cx, settings: Value) -> Result<(), String> {
        self.settings = settings;
        Ok(())
    }

    fn action(&mut self, cx: &mut Cx, action: &str, params: Value) -> Result<(), String> {
        match action {
            "state" => {
                let (k, v) = (text(&params, "k", action)?, text(&params, "v", action)?);
                cx.set_state(k, v);
            }
            "timer" => {
                let ms = params["ms"].as_u64().ok_or("timer needs ms")?;
                cx.set_timer(Duration::from_millis(ms), false);
            }
            "secret_set" => {
                let (k, v) = (text(&params, "k", action)?, text(&params, "v", action)?);
                cx.set_secret(k, v);
            }
            "secret_get" => {
                let value = cx.secret(text(&params, "k", action)?);
                cx.set_state("secret", value.unwrap_or_else(|| "none".into()));
            }
            "panic" => panic!("the probe panics on request"),
            "spin" => {
                let mut n = 0u64;
                loop {
                    n = black_box(n).wrapping_add(1);
                }
            }
            "alloc" => {
                let big = vec![1u8; 200 * 1024 * 1024];
                black_box(&big);
            }
            "flood" => {
                for i in 0..10_000 {
                    cx.set_state("flood", i.to_string());
                }
                for _ in 0..2_000 {
                    cx.set_timer(Duration::from_secs(60), false);
                }
            }
            "ws" => {
                cx.ws_open(text(&params, "url", action)?)?;
            }
            "fetch" => {
                let request = HttpRequest {
                    method: "GET".into(),
                    url: text(&params, "url", action)?.into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                };
                if let Err(e) = cx.fetch(request) {
                    cx.set_state("http", format!("error: {e}"));
                    return Err(e);
                }
            }
            "settings" => cx.set_state("settings", self.settings.to_string()),
            other => return Err(format!("unknown action: {other}")),
        }
        Ok(())
    }

    fn opened(&mut self, cx: &mut Cx, socket: Socket) {
        if let Err(e) = cx.send(socket, Payload::Text("ping".into())) {
            cx.set_state("ws_error", e);
        }
    }

    fn message(&mut self, cx: &mut Cx, _socket: Socket, data: Payload) {
        if let Payload::Text(t) = data {
            cx.set_state("ws", t);
        }
    }

    fn closed(&mut self, cx: &mut Cx, _socket: Socket, reason: &str) {
        cx.set_state("ws_closed", reason);
    }

    fn http(&mut self, cx: &mut Cx, _request: Request, response: Result<HttpResponse, String>) {
        let value = match response {
            Ok(r) => r.status.to_string(),
            Err(e) => format!("error: {e}"),
        };
        cx.set_state("http", value);
    }

    fn timer(&mut self, cx: &mut Cx, _timer: Timer) {
        cx.set_state("timer", "fired");
    }
}

ulanzi_plugin::export_plugin!(Probe);
