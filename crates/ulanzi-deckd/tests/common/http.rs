//! A small web server for update tests.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Reply {
    pub path: &'static str,
    pub status: u16,
    pub body: Vec<u8>,
    /// A larger size than the body, so the connection ends early.
    pub claim: Option<usize>,
}

pub fn reply(path: &'static str, status: u16, body: &[u8]) -> Reply {
    Reply {
        path,
        status,
        body: body.to_vec(),
        claim: None,
    }
}

/// Serve `replies` until the test ends; returns the address and every request's text.
pub fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<String>>>) {
    serve_late(move |_| replies)
}

/// Like [`serve`], with the replies made once the address is known.
pub fn serve_late(
    make: impl FnOnce(&str) -> Vec<Reply> + Send + 'static,
) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let replies = make(&base);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            log.lock().expect("log").push(text.clone());
            let path = text.split_whitespace().nth(1).unwrap_or("").to_string();
            let r = replies
                .iter()
                .find(|r| r.path == path)
                .cloned()
                .unwrap_or_else(|| reply("", 404, b""));
            let head = format!(
                "HTTP/1.1 {} X\r\nContent-Length: {}\r\nETag: \"tag1\"\r\nConnection: close\r\n\r\n",
                r.status,
                r.claim.unwrap_or(r.body.len())
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&r.body);
        }
    });
    (base, seen)
}
