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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
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
const MAX_MESSAGE: usize = 8 << 20;
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

/// The network side of an instance: its sockets and HTTP requests. Dropping it (with the Store,
/// however the instance ends) aborts them all, which closes their connections.
#[derive(Default)]
pub(crate) struct Net {
    sockets: HashMap<u32, SocketEntry>,
    fetches: HashMap<u32, AbortHandle>,
    last_id: u32,
}

struct SocketEntry {
    task: AbortHandle,
    /// Dropping the sender (the entry) tells the task the plugin closed the socket.
    tx: mpsc::UnboundedSender<Payload>,
    /// Dropped with the entry; tells a socket that is still connecting to give up.
    _gone: oneshot::Sender<()>,
    /// Bytes queued on `tx` and not yet written.
    unsent: Arc<AtomicUsize>,
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
            // Sockets that ended while their `on-closed` was dropped (queue full) are gone.
            self.sockets.retain(|_, s| !s.task.is_finished());
        }
        if self.sockets.len() >= MAX_SOCKETS {
            return Err(format!("too many sockets (at most {MAX_SOCKETS})"));
        }
        Ok(())
    }

    /// The socket ended and the plugin is being told so; forget it.
    pub(crate) fn socket_closed(&mut self, id: u32) {
        self.sockets.remove(&id);
    }

    /// The request's answer is being delivered; forget it.
    pub(crate) fn fetch_done(&mut self, id: u32) {
        self.fetches.remove(&id);
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
    let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
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
        let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
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
        let len = payload_len(&data);
        if entry.unsent.load(Ordering::Relaxed) + len > MAX_UNSENT {
            return Err("too much data waiting to be sent".into());
        }
        entry.unsent.fetch_add(len, Ordering::Relaxed);
        entry.tx.send(data).map_err(|_| {
            entry.unsent.fetch_sub(len, Ordering::Relaxed);
            format!("socket {socket} is closed")
        })
    }

    fn close(&mut self, socket: u32) {
        // Dropping the entry ends the task once the queued data is sent; no `on-closed` follows.
        self.net.sockets.remove(&socket);
    }

    fn fetch(&mut self, request: HttpRequest) -> Result<u32, String> {
        let Target { uri, host, port, .. } = split_url(&request.url, ("http", 80), ("https", 443))?;
        check_literal(&self.manifest.permissions, &host)?;
        let method = ureq::http::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| format!("not an HTTP method: {}", request.method))?;
        if self.net.fetches.len() >= MAX_FETCHES {
            self.net.fetches.retain(|_, f| !f.is_finished());
        }
        if self.net.fetches.len() >= MAX_FETCHES {
            return Err(format!("too many requests at once (at most {MAX_FETCHES})"));
        }
        let id = self.net.next_id();
        let perms = self.manifest.permissions.clone();
        let queue = self.queue.clone();
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
            queue.post(Event::Http(id, answer.await));
        });
        self.net.fetches.insert(id, task.abort_handle());
        Ok(id)
    }
}

impl HostState {
    /// Register a socket and run `body` as its task. `body` gets the socket id, the data the
    /// plugin sends, the count of unsent bytes, and a signal that resolves when the plugin closes
    /// the socket. It returns why the socket ended, or None when the plugin closed it (then no
    /// `on-closed` is sent).
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
        let queue = self.queue.clone();
        let run = body(
            id,
            queue.clone(),
            Plugin {
                data,
                gone,
                unsent: Arc::clone(&unsent),
            },
        );
        let task = self.rt.spawn(async move {
            let reason = match run.await {
                Ok(Some(reason)) | Err(reason) => reason,
                Ok(None) => return,
            };
            queue.post(Event::Closed(id, reason));
        });
        self.net.sockets.insert(
            id,
            SocketEntry {
                task: task.abort_handle(),
                tx,
                _gone: gone_tx,
                unsent,
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

fn payload_len(data: &Payload) -> usize {
    match data {
        Payload::Text(t) => t.len(),
        Payload::Binary(b) => b.len(),
    }
}

/// Relay a WebSocket until either side closes it.
async fn pump_ws<S>(id: u32, mut ws: WebSocketStream<S>, mut plugin: Plugin, queue: &Queue) -> Option<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut close_reason = None;
    loop {
        tokio::select! {
            out = plugin.data.recv() => match out {
                Some(data) => {
                    plugin.sent(&data);
                    let msg = match data {
                        Payload::Text(t) => Message::text(t),
                        Payload::Binary(b) => Message::binary(b),
                    };
                    if let Err(e) = ws.send(msg).await {
                        return Some(format!("connection lost: {e}"));
                    }
                }
                None => {
                    // The plugin closed the socket and everything it sent is out.
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, ws.close(None)).await;
                    return None;
                }
            },
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    queue.post(Event::Message(id, Payload::Text(t.as_str().to_owned())));
                }
                Some(Ok(Message::Binary(b))) => {
                    queue.post(Event::Message(id, Payload::Binary(b.to_vec())));
                }
                Some(Ok(Message::Close(frame))) => {
                    // tungstenite answers the close; the stream ends after that.
                    let reason = frame.map(|f| f.reason.as_str().to_owned()).unwrap_or_default();
                    close_reason = Some(if reason.is_empty() { "closed by the server".into() } else { reason });
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Some(close_reason.unwrap_or_else(|| format!("connection lost: {e}"))),
                None => return Some(close_reason.unwrap_or_else(|| "connection lost".into())),
            },
        }
    }
}

/// Relay a TCP (or TLS) stream until either side closes it.
async fn pump_tcp<S>(id: u32, mut stream: S, mut plugin: Plugin, queue: &Queue) -> Option<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        tokio::select! {
            out = plugin.data.recv() => match out {
                Some(data) => {
                    plugin.sent(&data);
                    let bytes = match &data {
                        Payload::Text(t) => t.as_bytes(),
                        Payload::Binary(b) => b.as_slice(),
                    };
                    if let Err(e) = stream.write_all(bytes).await {
                        return Some(format!("connection lost: {e}"));
                    }
                }
                None => {
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, stream.shutdown()).await;
                    return None;
                }
            },
            read = stream.read(&mut buf) => match read {
                Ok(0) => return Some("closed by the server".into()),
                Ok(n) => {
                    queue.post(Event::Message(id, Payload::Binary(buf[..n].to_vec())));
                }
                Err(e) => return Some(format!("connection lost: {e}")),
            },
        }
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
