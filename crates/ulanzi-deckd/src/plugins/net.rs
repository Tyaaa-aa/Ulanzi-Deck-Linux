//! The `net` host functions. The host owns every connection: each WebSocket, TCP socket and HTTP
//! request is a tokio task on the daemon's runtime that reports through the instance's queue
//! (`on-open`, `on-message`, `on-closed`, `on-http`). All of them end with the instance.
//!
//! Permissions are checked twice. The call itself checks the scheme and the literal host and
//! refuses at once (`not allowed: <host>`). A name that can only pass through `lan` is accepted
//! there and checked again in the task, against the addresses it resolved to. The task then
//! connects to exactly those addresses (no second lookup), so DNS cannot answer differently
//! between the check and the connection.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{Connector, WebSocketStream};
use ulanzi_core::plugin::{Permissions, host_allowed};
use ureq::http::Uri;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use super::instance::{Event, HostState, Queue};
use super::ulanzi::plugin::net;
use super::ulanzi::plugin::types::{HttpRequest, HttpResponse, Payload};

/// Open sockets (WebSocket and TCP) per instance.
const MAX_SOCKETS: usize = 64;
/// HTTP requests in flight per instance.
const MAX_FETCHES: usize = 64;
/// Largest incoming WebSocket message (and frame); a bigger one closes the socket.
const MAX_MESSAGE: usize = 1 << 20;
/// Network data (messages, response bodies) an instance may have waiting in its event queue.
const QUEUED_NET: usize = 16 << 20;
/// Largest HTTP response body; a bigger one fails with `response too large`.
const MAX_BODY: u64 = 8 << 20;
/// Bytes a socket may have waiting to be sent; `send` fails beyond that.
const MAX_UNSENT: usize = 8 << 20;
/// Largest `on-message` for a TCP socket (one read).
const READ_CHUNK: usize = 64 << 10;
/// Resolving, connecting and the TLS/WebSocket handshake together.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A whole HTTP request, from resolving to the last body byte.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// A socket the plugin closed gets this long to say goodbye.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// TLS for WebSocket and TCP sockets: the webpki roots with ring, set per config so no
/// process-wide crypto provider is needed. (HTTP goes through ureq's own rustls setup.)
static TLS: LazyLock<Option<Arc<rustls::ClientConfig>>> = LazyLock::new(|| {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    match rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions() {
        Ok(b) => Some(Arc::new(b.with_root_certificates(roots).with_no_client_auth())),
        Err(e) => {
            tracing::error!("TLS for plugins is unavailable: {e}");
            None
        }
    }
});

/// Network data (message payloads, response bodies) waiting in an instance's event queue. Socket
/// and HTTP tasks reserve room before posting and wait while there is none, so a flood backs up
/// into the peer's TCP window instead of the daemon's memory; the instance thread releases the
/// room as it takes each event.
#[derive(Default)]
pub(crate) struct Budget {
    used: AtomicUsize,
    freed: Notify,
}

impl Budget {
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    /// Wait until `n` more bytes fit, then count them.
    async fn reserve(&self, n: usize) {
        let n = n.min(QUEUED_NET);
        loop {
            let freed = self.freed.notified();
            tokio::pin!(freed);
            // Register before checking, so a release in between is not missed.
            freed.as_mut().enable();
            let mut used = self.used.load(Ordering::Acquire);
            while used + n <= QUEUED_NET {
                match self
                    .used
                    .compare_exchange_weak(used, used + n, Ordering::AcqRel, Ordering::Acquire)
                {
                    Ok(_) => return,
                    Err(now) => used = now,
                }
            }
            freed.await;
        }
    }

    /// Saturating: releasing more than was reserved cannot wrap the count.
    fn release(&self, n: usize) {
        let n = n.min(QUEUED_NET);
        if n == 0 {
            return;
        }
        let mut used = self.used.load(Ordering::Acquire);
        loop {
            match self.used.compare_exchange_weak(
                used,
                used.saturating_sub(n),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(now) => used = now,
            }
        }
        self.freed.notify_waiters();
    }

    /// Post `event` carrying `n` bytes once they fit; the room is returned if it is dropped.
    async fn post(&self, queue: &Queue, event: Event, n: usize) {
        self.reserve(n).await;
        if !queue.post(event) {
            self.release(n);
        }
    }
}

/// The network side of an instance: its sockets and HTTP requests. Dropping it (with the Store,
/// however the instance ends) aborts them all, which closes their connections.
pub(crate) struct Net {
    sockets: HashMap<u32, SocketEntry>,
    fetches: HashMap<u32, AbortHandle>,
    last_id: u32,
    budget: Arc<Budget>,
}

struct SocketEntry {
    /// Kept after the plugin closes the socket, until the task ends: a closing socket still
    /// counts toward `MAX_SOCKETS` and is aborted with the instance.
    task: AbortHandle,
    /// The plugin's end; None once the plugin closed the socket.
    open: Option<PluginEnd>,
    /// Bytes queued by `send` and not yet written.
    unsent: Arc<AtomicUsize>,
    /// The task ended but its `on-closed` was dropped (queue full): nothing more will come.
    lost: Arc<AtomicBool>,
}

/// Dropping this tells the socket's task the plugin closed it.
struct PluginEnd {
    tx: mpsc::UnboundedSender<Payload>,
    _gone: oneshot::Sender<()>,
}

impl Drop for Net {
    fn drop(&mut self) {
        for (_, s) in self.sockets.drain() {
            s.task.abort();
        }
        for (_, f) in self.fetches.drain() {
            f.abort();
        }
    }
}

impl Net {
    pub(crate) fn new(budget: Arc<Budget>) -> Self {
        Net {
            sockets: HashMap::new(),
            fetches: HashMap::new(),
            last_id: 0,
            budget,
        }
    }

    /// Socket and request ids share one counter, so an id names one thing at a time.
    fn next_id(&mut self) -> u32 {
        loop {
            self.last_id = self.last_id.wrapping_add(1);
            let id = self.last_id;
            if id != 0 && !self.sockets.contains_key(&id) && !self.fetches.contains_key(&id) {
                return id;
            }
        }
    }

    fn room_for_socket(&mut self) -> Result<(), String> {
        if self.sockets.len() >= MAX_SOCKETS {
            // Ended sockets with nothing left to report: closed by the plugin, or their
            // `on-closed` was dropped. One whose `on-closed` is still queued stays.
            self.sockets.retain(|_, s| {
                !(s.task.is_finished() && (s.open.is_none() || s.lost.load(Ordering::Relaxed)))
            });
        }
        if self.sockets.len() >= MAX_SOCKETS {
            return Err(format!("too many sockets (at most {MAX_SOCKETS})"));
        }
        Ok(())
    }

    /// Is the socket open as far as the plugin knows? Its events reach the plugin only then.
    pub(crate) fn socket_open(&self, id: u32) -> bool {
        self.sockets.get(&id).is_some_and(|s| s.open.is_some())
    }

    /// The instance thread took a message; true when it should reach the plugin.
    pub(crate) fn message_taken(&self, id: u32, data: &Payload) -> bool {
        self.budget.release(payload_len(data));
        self.socket_open(id)
    }

    /// The socket ended; forget it. True when the plugin should hear of it (it did not close the
    /// socket itself).
    pub(crate) fn socket_closed(&mut self, id: u32) -> bool {
        self.sockets.remove(&id).is_some_and(|s| s.open.is_some())
    }

    /// The request's answer is being delivered; forget it.
    pub(crate) fn fetch_done(&mut self, id: u32, response: &Result<HttpResponse, String>) {
        if let Ok(r) = response {
            self.budget.release(r.body.len());
        }
        self.fetches.remove(&id);
    }
}

/// Headers the host sets itself; a plugin may not supply them.
const RESERVED_HEADERS: [&str; 4] = ["host", "connection", "transfer-encoding", "content-length"];

fn check_headers(headers: &[(String, String)]) -> Result<(), String> {
    match headers.iter().find(|(name, _)| {
        RESERVED_HEADERS
            .iter()
            .any(|r| name.trim().eq_ignore_ascii_case(r))
    }) {
        Some((name, _)) => Err(format!("header not allowed: {name}")),
        None => Ok(()),
    }
}

/// A host as given, without IPv6 brackets; `1.2.3.4.` is the address `1.2.3.4`, never a name to
/// look up.
fn host_of(host: &str) -> String {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.strip_suffix('.') {
        Some(ip) if ip.parse::<IpAddr>().is_ok() => ip.to_string(),
        _ => host.to_string(),
    }
}

fn not_allowed(host: &str) -> String {
    format!("not allowed: {}", host.to_ascii_lowercase())
}

/// The check before any network access: literal host names and addresses only. A name that could
/// still pass through `lan` once resolved is let through here and checked again after DNS.
fn check_literal(p: &Permissions, host: &str) -> Result<(), String> {
    if host_allowed(p, host, &[]) {
        return Ok(());
    }
    if p.lan && !host.is_empty() && host.parse::<IpAddr>().is_err() {
        return Ok(());
    }
    Err(not_allowed(host))
}

/// Resolve once and check what the name resolved to; the caller connects to exactly these.
async fn resolve(p: &Permissions, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("cannot find {host}: {e}"))?
        .collect();
    let ips: Vec<IpAddr> = addrs.iter().map(SocketAddr::ip).collect();
    if addrs.is_empty() {
        return Err(format!("cannot find {host}"));
    }
    if !host_allowed(p, host, &ips) {
        return Err(not_allowed(host));
    }
    Ok(addrs)
}

/// Resolve, check and connect to the first address that answers.
async fn connect_tcp(p: &Permissions, host: &str, port: u16) -> Result<TcpStream, String> {
    let mut last = String::from("no address");
    for addr in resolve(p, host, port).await? {
        match TcpStream::connect(addr).await {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                return Ok(s);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("cannot connect to {host}: {last}"))
}

/// A URL taken apart: the host is without IPv6 brackets, the port filled in.
struct Target {
    uri: Uri,
    host: String,
    port: u16,
    secure: bool,
}

/// `plain` and `secure` are the two schemes allowed, with their default ports.
fn split_url(url: &str, plain: (&str, u16), secure: (&str, u16)) -> Result<Target, String> {
    let uri: Uri = url.parse().map_err(|_| format!("not a valid URL: {url}"))?;
    let scheme = uri.scheme_str().unwrap_or("").to_ascii_lowercase();
    let (default_port, is_secure) = if scheme == plain.0 {
        (plain.1, false)
    } else if scheme == secure.0 {
        (secure.1, true)
    } else {
        return Err(format!(
            "only {}:// and {}:// URLs are supported",
            plain.0, secure.0
        ));
    };
    let host = uri
        .host()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| format!("no host in {url}"))?;
    let host = host_of(host);
    let port = uri.port_u16().unwrap_or(default_port);
    Ok(Target {
        uri,
        host,
        port,
        secure: is_secure,
    })
}

impl net::Host for HostState {
    fn ws_open(&mut self, url: String) -> Result<u32, String> {
        let Target {
            host, port, secure, ..
        } = split_url(&url, ("ws", 80), ("wss", 443))?;
        check_literal(&self.manifest.permissions, &host)?;
        let tls = match secure {
            true => Some(TLS.clone().ok_or("TLS is unavailable")?),
            false => None,
        };
        let perms = self.manifest.permissions.clone();
        self.open_socket(move |id, queue, mut plugin| async move {
            let opening = async {
                let tcp = connect_tcp(&perms, &host, port).await?;
                let config = WebSocketConfig::default()
                    .max_message_size(Some(MAX_MESSAGE))
                    .max_frame_size(Some(MAX_MESSAGE));
                // The URL keeps the original host for the Host header and SNI.
                let connector = tls.map_or(Connector::Plain, Connector::Rustls);
                tokio_tungstenite::client_async_tls_with_config(
                    url.as_str(),
                    tcp,
                    Some(config),
                    Some(connector),
                )
                .await
                .map(|(ws, _)| ws)
                .map_err(|e| format!("cannot open {url}: {e}"))
            };
            let Some(ws) = plugin.connecting(opening).await? else {
                return Ok(None);
            };
            queue.post(Event::Open(id));
            Ok(pump_ws(id, ws, plugin, &queue).await)
        })
    }

    fn tcp_open(&mut self, host: String, port: u16, tls: bool) -> Result<u32, String> {
        let host = host_of(&host);
        if host.is_empty() {
            return Err("no host".into());
        }
        check_literal(&self.manifest.permissions, &host)?;
        let tls = match tls {
            true => Some(TLS.clone().ok_or("TLS is unavailable")?),
            false => None,
        };
        let name = match &tls {
            Some(_) => Some(
                rustls::pki_types::ServerName::try_from(host.clone())
                    .map_err(|_| format!("not a valid host name: {host}"))?,
            ),
            None => None,
        };
        let perms = self.manifest.permissions.clone();
        self.open_socket(move |id, queue, mut plugin| async move {
            let opening = async {
                let tcp = connect_tcp(&perms, &host, port).await?;
                match (tls, name) {
                    (Some(config), Some(name)) => tokio_rustls::TlsConnector::from(config)
                        .connect(name, tcp)
                        .await
                        .map(|s| Stream::Tls(Box::new(s)))
                        .map_err(|e| format!("TLS with {host} failed: {e}")),
                    _ => Ok(Stream::Plain(tcp)),
                }
            };
            let Some(stream) = plugin.connecting(opening).await? else {
                return Ok(None);
            };
            queue.post(Event::Open(id));
            Ok(match stream {
                Stream::Plain(s) => pump_tcp(id, s, plugin, &queue).await,
                Stream::Tls(s) => pump_tcp(id, *s, plugin, &queue).await,
            })
        })
    }

    fn send(&mut self, socket: u32, data: Payload) -> Result<(), String> {
        let entry = self
            .net
            .sockets
            .get(&socket)
            .ok_or_else(|| format!("unknown socket {socket}"))?;
        let end = entry
            .open
            .as_ref()
            .ok_or_else(|| format!("socket {socket} is closed"))?;
        let len = payload_len(&data);
        if entry.unsent.load(Ordering::Relaxed) + len > MAX_UNSENT {
            return Err("too much data waiting to be sent".into());
        }
        entry.unsent.fetch_add(len, Ordering::Relaxed);
        end.tx.send(data).map_err(|_| {
            entry.unsent.fetch_sub(len, Ordering::Relaxed);
            format!("socket {socket} is closed")
        })
    }

    fn close(&mut self, socket: u32) {
        // The task sends what is queued and closes, within CLOSE_TIMEOUT; no `on-closed` follows.
        if let Some(entry) = self.net.sockets.get_mut(&socket) {
            entry.open = None;
        }
    }

    fn fetch(&mut self, request: HttpRequest) -> Result<u32, String> {
        let Target { uri, host, port, .. } = split_url(&request.url, ("http", 80), ("https", 443))?;
        check_literal(&self.manifest.permissions, &host)?;
        let method = ureq::http::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| format!("not an HTTP method: {}", request.method))?;
        check_headers(&request.headers)?;
        if self.net.fetches.len() >= MAX_FETCHES {
            self.net.fetches.retain(|_, f| !f.is_finished());
        }
        if self.net.fetches.len() >= MAX_FETCHES {
            return Err(format!("too many requests at once (at most {MAX_FETCHES})"));
        }
        let id = self.net.next_id();
        let perms = self.manifest.permissions.clone();
        let queue = self.queue.clone();
        let budget = Arc::clone(&self.net.budget);
        let task = self.rt.spawn(async move {
            let answer = async {
                let addrs = tokio::time::timeout(CONNECT_TIMEOUT, resolve(&perms, &host, port))
                    .await
                    .map_err(|_| format!("timed out looking up {host}"))??;
                // ureq blocks; its own timeouts bound the thread even when this task is aborted.
                tokio::task::spawn_blocking(move || blocking_fetch(method, uri, request, addrs))
                    .await
                    .map_err(|_| "the request failed".to_string())?
            };
            let answer = answer.await;
            let size = answer.as_ref().map_or(0, |r| r.body.len());
            budget.post(&queue, Event::Http(id, answer), size).await;
        });
        self.net.fetches.insert(id, task.abort_handle());
        Ok(id)
    }
}

impl HostState {
    /// Register a socket and run `body` as its task. It returns why the socket ended, or None
    /// when the plugin closed it (then no `on-closed` is sent).
    fn open_socket<F, Fut>(&mut self, body: F) -> Result<u32, String>
    where
        F: FnOnce(u32, Queue, Plugin) -> Fut,
        Fut: Future<Output = Result<Option<String>, String>> + Send + 'static,
    {
        self.net.room_for_socket()?;
        let id = self.net.next_id();
        let (tx, data) = mpsc::unbounded_channel();
        let (gone_tx, gone) = oneshot::channel();
        let unsent = Arc::new(AtomicUsize::new(0));
        let lost = Arc::new(AtomicBool::new(false));
        let queue = self.queue.clone();
        let run = body(
            id,
            queue.clone(),
            Plugin {
                data,
                gone,
                unsent: Arc::clone(&unsent),
                budget: Arc::clone(&self.net.budget),
            },
        );
        let task = self.rt.spawn({
            let lost = Arc::clone(&lost);
            async move {
                let reason = match run.await {
                    Ok(Some(reason)) | Err(reason) => reason,
                    Ok(None) => return,
                };
                if !queue.post(Event::Closed(id, reason)) {
                    lost.store(true, Ordering::Relaxed);
                }
            }
        });
        self.net.sockets.insert(
            id,
            SocketEntry {
                task: task.abort_handle(),
                open: Some(PluginEnd { tx, _gone: gone_tx }),
                unsent,
                lost,
            },
        );
        Ok(id)
    }
}

/// The plugin's side of a socket, as its task sees it.
struct Plugin {
    /// What the plugin sends; ends (after the queued data) when the plugin closes the socket.
    data: mpsc::UnboundedReceiver<Payload>,
    /// Resolves when the plugin closes the socket.
    gone: oneshot::Receiver<()>,
    unsent: Arc<AtomicUsize>,
    budget: Arc<Budget>,
}

/// One step of a socket's relay.
enum Step {
    Go,
    /// The plugin closed the socket: send what is queued, close, report nothing.
    Finish,
    /// The connection ended; report why.
    Ended(String),
}

impl Plugin {
    /// Run `opening` (resolve, connect, handshake) within `CONNECT_TIMEOUT`; Ok(None) when the
    /// plugin closed the socket first.
    async fn connecting<T>(
        &mut self,
        opening: impl Future<Output = Result<T, String>>,
    ) -> Result<Option<T>, String> {
        tokio::select! {
            r = tokio::time::timeout(CONNECT_TIMEOUT, opening) => match r {
                Ok(r) => r.map(Some),
                Err(_) => Err("timed out connecting".into()),
            },
            _ = &mut self.gone => Ok(None),
        }
    }

    fn sent(&self, data: &Payload) {
        self.unsent.fetch_sub(payload_len(data), Ordering::Relaxed);
    }
}

/// Run `step` unless the plugin closes the socket first.
async fn unless_gone(gone: &mut oneshot::Receiver<()>, step: impl Future<Output = Step>) -> Step {
    tokio::select! {
        s = step => s,
        _ = gone => Step::Finish,
    }
}

/// Hand a message to the plugin once the instance's network budget has room for it.
async fn deliver(budget: &Budget, queue: &Queue, id: u32, data: Payload) -> Step {
    let n = payload_len(&data);
    budget.post(queue, Event::Message(id, data), n).await;
    Step::Go
}

fn payload_len(data: &Payload) -> usize {
    match data {
        Payload::Text(t) => t.len(),
        Payload::Binary(b) => b.len(),
    }
}

fn ws_message(data: Payload) -> Message {
    match data {
        Payload::Text(t) => Message::text(t),
        Payload::Binary(b) => Message::binary(b),
    }
}

/// Relay a WebSocket until either side closes it.
///
/// Data the plugin sent before closing is never lost: the idle wait prefers queued data over the
/// close, and a message caught mid-send by the close is finished first when draining.
async fn pump_ws<S>(id: u32, mut ws: WebSocketStream<S>, mut plugin: Plugin, queue: &Queue) -> Option<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut close_reason: Option<String> = None;
    // A message taken from the plugin but not yet handed to the WebSocket.
    let mut pending: Option<Message> = None;
    loop {
        let step = tokio::select! {
            biased;
            out = plugin.data.recv() => match out {
                Some(data) => {
                    plugin.sent(&data);
                    let msg = ws_message(data);
                    // Wait for room without giving up the message: a close keeps it for the drain.
                    let ready = tokio::select! {
                        biased;
                        r = std::future::poll_fn(|cx| ws.poll_ready_unpin(cx)) => Some(r),
                        _ = &mut plugin.gone => None,
                    };
                    match ready {
                        None => {
                            pending = Some(msg);
                            Step::Finish
                        }
                        Some(Err(e)) => Step::Ended(format!("connection lost: {e}")),
                        Some(Ok(())) => match ws.start_send_unpin(msg) {
                            Err(e) => Step::Ended(format!("connection lost: {e}")),
                            // Handed over: a close during the flush leaves it for `close` to flush.
                            Ok(()) => tokio::select! {
                                biased;
                                r = std::future::poll_fn(|cx| ws.poll_flush_unpin(cx)) => match r {
                                    Ok(()) => Step::Go,
                                    Err(e) => Step::Ended(format!("connection lost: {e}")),
                                },
                                _ = &mut plugin.gone => Step::Finish,
                            },
                        },
                    }
                }
                None => Step::Finish,
            },
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let data = Payload::Text(t.as_str().to_owned());
                    unless_gone(&mut plugin.gone, deliver(&plugin.budget, queue, id, data)).await
                }
                Some(Ok(Message::Binary(b))) => {
                    let data = Payload::Binary(b.to_vec());
                    unless_gone(&mut plugin.gone, deliver(&plugin.budget, queue, id, data)).await
                }
                Some(Ok(Message::Close(frame))) => {
                    // tungstenite answers the close; the stream ends after that.
                    let reason = frame.map(|f| f.reason.as_str().to_owned()).unwrap_or_default();
                    close_reason = Some(if reason.is_empty() { "closed by the server".into() } else { reason });
                    Step::Go
                }
                Some(Ok(_)) => Step::Go,
                Some(Err(e)) => Step::Ended(format!("connection lost: {e}")),
                None => Step::Ended("connection lost".into()),
            },
            _ = &mut plugin.gone => Step::Finish,
        };
        match step {
            Step::Go => {}
            // A close the server already announced explains the end best.
            Step::Ended(why) => return Some(close_reason.unwrap_or(why)),
            Step::Finish => {
                let data = &mut plugin.data;
                let unsent = &plugin.unsent;
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
                    if let Some(msg) = pending.take()
                        && ws.feed(msg).await.is_err()
                    {
                        return;
                    }
                    while let Some(d) = data.recv().await {
                        unsent.fetch_sub(payload_len(&d), Ordering::Relaxed);
                        if ws.feed(ws_message(d)).await.is_err() {
                            return;
                        }
                    }
                    // Flushes everything fed, then the close frame.
                    let _ = ws.close(None).await;
                })
                .await;
                return None;
            }
        }
    }
}

/// Relay a TCP (or TLS) stream until either side closes it. Like `pump_ws`, data sent before the
/// plugin closed goes out in full and in order: a write caught by the close resumes at the byte
/// where it stopped.
async fn pump_tcp<S>(id: u32, mut stream: S, mut plugin: Plugin, queue: &Queue) -> Option<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; READ_CHUNK];
    // A payload taken from the plugin and how much of it is written.
    let mut pending: Option<(Payload, usize)> = None;
    loop {
        let step = tokio::select! {
            biased;
            out = plugin.data.recv() => match out {
                Some(data) => {
                    plugin.sent(&data);
                    let bytes = payload_bytes(&data);
                    let mut done = 0;
                    let mut step = Step::Go;
                    // One `write` at a time: each either writes some bytes or none, so a close
                    // in between knows exactly where to resume.
                    while done < bytes.len() {
                        tokio::select! {
                            biased;
                            w = stream.write(&bytes[done..]) => match w {
                                Ok(0) => {
                                    step = Step::Ended("connection lost".into());
                                    break;
                                }
                                Ok(n) => done += n,
                                Err(e) => {
                                    step = Step::Ended(format!("connection lost: {e}"));
                                    break;
                                }
                            },
                            _ = &mut plugin.gone => {
                                step = Step::Finish;
                                break;
                            }
                        }
                    }
                    if matches!(step, Step::Finish) {
                        pending = Some((data, done));
                    }
                    step
                }
                None => Step::Finish,
            },
            read = stream.read(&mut buf) => match read {
                Ok(0) => Step::Ended("closed by the server".into()),
                Ok(n) => {
                    let data = Payload::Binary(buf[..n].to_vec());
                    unless_gone(&mut plugin.gone, deliver(&plugin.budget, queue, id, data)).await
                }
                Err(e) => Step::Ended(format!("connection lost: {e}")),
            },
            _ = &mut plugin.gone => Step::Finish,
        };
        match step {
            Step::Go => {}
            Step::Ended(why) => return Some(why),
            Step::Finish => {
                let data = &mut plugin.data;
                let unsent = &plugin.unsent;
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
                    if let Some((d, done)) = pending.take()
                        && stream.write_all(&payload_bytes(&d)[done..]).await.is_err()
                    {
                        return;
                    }
                    while let Some(d) = data.recv().await {
                        unsent.fetch_sub(payload_len(&d), Ordering::Relaxed);
                        if stream.write_all(payload_bytes(&d)).await.is_err() {
                            return;
                        }
                    }
                    let _ = stream.shutdown().await;
                })
                .await;
                return None;
            }
        }
    }
}

fn payload_bytes(data: &Payload) -> &[u8] {
    match data {
        Payload::Text(t) => t.as_bytes(),
        Payload::Binary(b) => b.as_slice(),
    }
}

/// A plain or TLS TCP stream.
enum Stream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

/// Hands ureq the addresses that were already resolved and checked, so it cannot look the name
/// up again (and get a different answer) before connecting.
#[derive(Debug)]
struct Checked(Vec<SocketAddr>);

impl Resolver for Checked {
    fn resolve(
        &self,
        _uri: &Uri,
        _config: &ureq::config::Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let mut out = self.empty();
        for addr in &self.0 {
            if out.try_push(*addr).is_err() {
                break;
            }
        }
        if out.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(out)
    }
}

/// One HTTP request with ureq, on a blocking thread. No redirects (a 3xx goes to the plugin, so
/// it can never be led to a host outside its permissions), no proxy, at most `MAX_BODY` back.
fn blocking_fetch(
    method: ureq::http::Method,
    uri: Uri,
    request: HttpRequest,
    addrs: Vec<SocketAddr>,
) -> Result<HttpResponse, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .proxy(None)
        .allow_non_standard_methods(true)
        .max_idle_connections(0)
        .build();
    let agent = ureq::Agent::with_parts(config, DefaultConnector::new(), Checked(addrs));
    let mut builder = ureq::http::Request::builder().method(method).uri(uri);
    for (name, value) in &request.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    let sent = if request.body.is_empty() {
        let req = builder.body(ureq::SendBody::none()).map_err(|e| e.to_string())?;
        agent.run(req)
    } else {
        let req = builder.body(request.body).map_err(|e| e.to_string())?;
        agent.run(req)
    };
    let mut response = sent.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => "response too large".to_string(),
            e => e.to_string(),
        })?;
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(network: &[&str], lan: bool) -> Permissions {
        Permissions {
            network: network.iter().map(|s| s.to_string()).collect(),
            lan,
            ..Permissions::default()
        }
    }

    #[test]
    fn literal_check() {
        let listed = perms(&["api.example.com"], false);
        assert_eq!(check_literal(&listed, "API.example.com."), Ok(()));
        assert_eq!(
            check_literal(&listed, "Other.com"),
            Err("not allowed: other.com".into())
        );
        let lan = perms(&[], true);
        assert_eq!(check_literal(&lan, "192.168.1.2"), Ok(()));
        assert_eq!(check_literal(&lan, "8.8.8.8"), Err("not allowed: 8.8.8.8".into()));
        // A name may still resolve to the local network: checked again after DNS.
        assert_eq!(check_literal(&lan, "printer.lan"), Ok(()));
        assert!(check_literal(&listed, "192.168.1.2").is_err());
        // An address with a trailing dot is still an address, not a name for DNS.
        assert_eq!(host_of("8.8.8.8."), "8.8.8.8");
        assert_eq!(
            check_literal(&lan, &host_of("8.8.8.8.")),
            Err("not allowed: 8.8.8.8".into())
        );
        assert_eq!(host_of("[::1]"), "::1");
        assert_eq!(host_of("printer.lan."), "printer.lan.");
    }

    #[test]
    fn reserved_headers_are_refused() {
        let h = |name: &str| vec![(name.to_string(), "x".to_string())];
        assert_eq!(check_headers(&h("Accept")), Ok(()));
        for name in ["Host", "connection", "Transfer-Encoding", "CONTENT-LENGTH"] {
            assert_eq!(
                check_headers(&h(name)),
                Err(format!("header not allowed: {name}"))
            );
        }
    }

    #[tokio::test]
    async fn budget_waits_for_room() {
        let budget = Arc::new(Budget::default());
        budget.reserve(QUEUED_NET - 10).await;
        let waiting = tokio::spawn({
            let budget = Arc::clone(&budget);
            async move { budget.reserve(20).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiting.is_finished(), "reserved past the budget");
        budget.release(QUEUED_NET - 10);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("room after release")
            .expect("task");
        assert_eq!(budget.used(), 20);
        // Releasing more than is reserved stops at zero.
        budget.release(1000);
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test]
    async fn names_are_checked_after_resolving() {
        let lan = perms(&[], true);
        assert!(resolve(&lan, "localhost", 80).await.is_ok());
        let none = perms(&["example.com"], false);
        assert_eq!(
            resolve(&none, "localhost", 80).await,
            Err("not allowed: localhost".into())
        );
    }

    #[test]
    fn tls_config_builds_without_a_global_provider() {
        assert!(TLS.is_some());
    }

    #[test]
    fn urls() {
        let t = split_url("WSS://[::1]/x", ("ws", 80), ("wss", 443)).expect("url");
        assert_eq!((t.host.as_str(), t.port, t.secure), ("::1", 443, true));
        let t = split_url("ws://h:9/", ("ws", 80), ("wss", 443)).expect("url");
        assert_eq!((t.host.as_str(), t.port, t.secure), ("h", 9, false));
        assert!(split_url("http://h/", ("ws", 80), ("wss", 443)).is_err());
        assert!(split_url("nonsense", ("ws", 80), ("wss", 443)).is_err());
    }
}
