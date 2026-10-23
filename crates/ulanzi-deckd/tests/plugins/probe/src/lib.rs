//! A test plugin for the host: each action exercises one host function, or misbehaves on purpose
//! (panic, endless loop, huge allocation, flood of host calls).
use std::hint::black_box;
use std::time::Duration;

use ulanzi_plugin::{Cx, HttpRequest, HttpResponse, Payload, Plugin, Request, Socket, Timer, Value};

#[derive(Default)]
pub struct Probe {
    settings: Value,
    /// The most recently opened socket.
    socket: Option<Socket>,
}

fn text<'a>(params: &'a Value, key: &str, action: &str) -> Result<&'a str, String> {
    params[key]
        .as_str()
        .ok_or_else(|| format!("{action} needs {key}"))
}

/// Payload `i` of the `burst` action: its number, padded to 4 KiB.
pub fn burst_payload(i: u64) -> String {
    format!("{i:04}{}", ".".repeat(4092))
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
            "tcp" => {
                let host = text(&params, "host", action)?;
                let port = params["port"].as_u64().ok_or("tcp needs port")?;
                let port = u16::try_from(port).map_err(|_| "bad port")?;
                cx.tcp_open(host, port, false)?;
            }
            "close" => {
                // Close twice (must be harmless), then sending must fail.
                let socket = self.socket.ok_or("no socket")?;
                cx.close(socket);
                cx.close(socket);
                let sent = cx.send(socket, Payload::Text("late".into()));
                cx.set_state("send", sent.err().unwrap_or_else(|| "ok".into()));
            }
            "blast" => {
                // Send `kib` KiB in 64 KiB pieces on the last socket, then close it.
                let socket = self.socket.ok_or("no socket")?;
                let kib = params["kib"].as_u64().ok_or("blast needs kib")?;
                for _ in 0..kib / 64 {
                    cx.send(socket, Payload::Binary(vec![b'x'; 64 * 1024]))?;
                }
                cx.close(socket);
            }
            "burst" => {
                // `n` text payloads of 4 KiB (`burst_payload`) on the last socket, then close.
                let socket = self.socket.ok_or("no socket")?;
                let n = params["n"].as_u64().ok_or("burst needs n")?;
                for i in 0..n {
                    cx.send(socket, Payload::Text(burst_payload(i)))?;
                }
                cx.close(socket);
            }
            "sleep" => {
                let ms = params["ms"].as_u64().ok_or("sleep needs ms")?;
                std::thread::sleep(Duration::from_millis(ms));
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
        self.socket = Some(socket);
        if let Err(e) = cx.send(socket, Payload::Text("ping".into())) {
            cx.set_state("ws_error", e);
        }
    }

    fn message(&mut self, cx: &mut Cx, _socket: Socket, data: Payload) {
        match data {
            Payload::Text(t) => cx.set_state("ws", t),
            Payload::Binary(b) => cx.set_state("bin", String::from_utf8_lossy(&b)),
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
