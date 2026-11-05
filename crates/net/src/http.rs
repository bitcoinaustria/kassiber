//! Deliberately one HTTP/1.1 request per connection. No pool, background work,
//! retries, cookies, proxy discovery, decompression, or implicit credentials.
use crate::{remaining, Connection, Endpoint, Error, Proxy, ProxyKind, Route, TlsTrust, Transport};
use base64::Engine;
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_HEADERS: usize = 100;
const MAX_INFORMATIONAL: usize = 8;
const MAX_CHUNKS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RedirectPolicy {
    /// Backend, rates, BTCPay and updates: no redirect authority.
    #[default]
    Refuse,
    /// AI HTTP: exact origin only, with urllib's repeat/destination limits.
    SameOrigin,
}

#[derive(Clone, Copy, Debug)]
pub struct HttpOptions {
    /// Python transport default: timeout for each blocking operation (30 seconds).
    pub timeout: Duration,
    /// An optional acquisition budget also bounds redirects and all reads/writes.
    pub deadline: Option<Instant>,
    /// Includes error responses; callers such as updates can tighten to 2 MiB.
    /// Values above the hard 64 MiB cap are clamped.
    pub max_response_bytes: usize,
    pub redirects: RedirectPolicy,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            deadline: None,
            max_response_bytes: 16 * 1024 * 1024,
            redirects: RedirectPolicy::Refuse,
        }
    }
}

/// Contains secrets. Intentionally no Debug; nothing is added to the RAM ledger
/// except the route's closed metadata and encrypted/on-wire byte counts.
pub struct HttpRequest {
    url: Url,
    method: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    upgrade: bool,
    cancel: Option<Cancel>,
}

impl HttpRequest {
    /// Parse configuration without DNS, sockets, trust loading or probing.
    pub fn new(method: &str, url: &str) -> Result<Self, Error> {
        let url = parse_url(url)?;
        if !matches!(
            method,
            "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS"
        ) {
            return Err(Error::HttpInvalid);
        }
        Ok(Self {
            url,
            method: method.into(),
            headers: Vec::new(),
            body: Vec::new(),
            upgrade: false,
            cancel: None,
        })
    }

    pub fn header(mut self, name: &str, value: &str) -> Result<Self, Error> {
        if !token(name)
            || value
                .bytes()
                .any(|b| b < 32 && b != b'\t' || b == 127 || b > 126)
            || matches!(
                name.to_ascii_lowercase().as_str(),
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "proxy-authorization"
                    | "proxy-connection"
                    | "expect"
                    | "trailer"
                    | "te"
            )
        {
            return Err(Error::HttpInvalid);
        }
        self.headers
            .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
        self.headers.push((name.into(), value.into()));
        Ok(self)
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    /// Permit a 101 response. A future WebSocket codec supplies/validates the
    /// protocol's headers (including Sec-WebSocket-Accept) before using the stream.
    pub fn allow_upgrade(mut self) -> Self {
        self.upgrade = true;
        self
    }

    /// Makes `cancel` able to stop this request: [`Cancel::cancel`] shuts the
    /// socket down, so a blocked connect, write or read returns at once.
    pub fn cancellable(mut self, cancel: &Cancel) -> Self {
        self.cancel = Some(cancel.clone());
        self
    }

    pub fn send(self, route: Route, options: HttpOptions) -> Result<HttpResponse, Error> {
        let cancel = self.cancel.clone();
        let response = self.exchange(route, options, |connection, method, upgrade, limit| {
            let response = receive(connection, method, upgrade, limit)?;
            let location = response.header("location").map(str::to_owned);
            Ok((response.status, location, response))
        });
        if let Some(cancel) = cancel {
            cancel.release();
        }
        response
    }

    /// Like [`HttpRequest::send`], but returns once the response head has
    /// arrived; the body is read from the returned [`HttpStream`] as the
    /// server sends it. Redirect bodies are still read whole, as `send`
    /// reads them, before the next hop. A 101 is refused.
    pub fn open(self, route: Route, options: HttpOptions) -> Result<HttpStream, Error> {
        if self.upgrade {
            return Err(Error::HttpInvalid);
        }
        let cancel = self.cancel.clone();
        let stream = self.exchange(route, options, |connection, method, _, limit| {
            let mut stream = HttpStream::receive(connection, method, limit)?;
            let location = stream.header("location").map(str::to_owned);
            if matches!(stream.status, 301 | 302 | 303 | 307 | 308) && location.is_some() {
                // Read the body before following, as `receive` does.
                stream.drain()?;
            }
            Ok((stream.status, location, stream))
        });
        match stream {
            Ok(stream) => Ok(stream.attach(cancel)),
            Err(error) => {
                if let Some(cancel) = cancel {
                    cancel.release();
                }
                Err(error)
            }
        }
    }

    fn exchange<T>(
        mut self,
        route: Route,
        options: HttpOptions,
        receive: impl Fn(Connection, &str, bool, usize) -> Result<(u16, Option<String>, T), Error>,
    ) -> Result<T, Error> {
        route.check()?;
        self.bind(&route)?;
        if options.timeout.is_zero() {
            return Err(Error::InvalidTimeout);
        }
        let mut redirects = RedirectHistory::default();
        loop {
            route.check()?;
            let mut wire = self.encode(&route)?;
            let timeout = options
                .deadline
                .map(remaining)
                .transpose()?
                .map_or(options.timeout, |budget| budget.min(options.timeout));
            // Keep the original route immutable across same-origin hops.
            let next = Route {
                gate: route.gate.clone(),
                request: route.request.clone(),
                consent: route.consent.clone(),
            };
            if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "request cancelled",
                )));
            }
            let mut connection = next.connect_inner(timeout, true)?;
            if let Some(cancel) = &self.cancel {
                cancel.register(&mut connection)?;
            }
            connection.set_deadline(options.deadline);
            connection.transport().write_context =
                crate::WriteContext::Http(method_label(&self.method)?);
            connection.write_all(&wire)?;
            connection.flush()?;
            wire.fill(0);
            let (status, location, response) = receive(
                connection,
                &self.method,
                self.upgrade,
                options.max_response_bytes.min(MAX_RESPONSE_BYTES),
            )?;
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let Some(location) = location else {
                return Ok(response);
            };
            drop(response);
            if options.redirects == RedirectPolicy::Refuse {
                return Err(Error::RedirectRefused);
            }
            let target = self.url.join(&location).map_err(|_| Error::HttpInvalid)?;
            validate_url(&target)?;
            if target.origin() != self.url.origin() {
                return Err(Error::RedirectRefused);
            }
            // urllib does not redirect POST on 307/308 or arbitrary mutating verbs.
            if !matches!(self.method.as_str(), "GET" | "HEAD") {
                if self.method != "POST" || !matches!(status, 301..=303) {
                    return Err(Error::RedirectRefused);
                }
                self.method = "GET".into();
                self.body.clear();
                self.headers.retain(|(name, _)| {
                    !matches!(
                        name.to_ascii_lowercase().as_str(),
                        "content-type" | "content-length"
                    )
                });
            }
            redirects.visit(&target)?;
            self.url = target;
            self.bind(&route)?;
            // receive has closed the previous stream; credentials remain origin-bound.
        }
    }

    fn bind(&self, route: &Route) -> Result<(), Error> {
        let target = Endpoint::new(
            self.url
                .host_str()
                .ok_or(Error::InvalidEndpoint)?
                .trim_matches(['[', ']']),
            self.url
                .port_or_known_default()
                .ok_or(Error::InvalidEndpoint)?,
        )?;
        if target != route.request.endpoint
            || (self.url.scheme() == "http") != (route.request.tls == TlsTrust::Plaintext)
        {
            return Err(Error::ConsentScope);
        }
        Ok(())
    }

    fn encode(&self, route: &Route) -> Result<Vec<u8>, Error> {
        let forward = route.request.proxy.as_ref().filter(|proxy| {
            proxy.kind == ProxyKind::Http && route.request.tls == TlsTrust::Plaintext
        });
        let path = if forward.is_some() {
            self.url.as_str()
        } else {
            &self.url[url::Position::BeforePath..url::Position::AfterQuery]
        };
        let mut wire = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: {}\r\n",
            self.method,
            path,
            host_header(&route.request.endpoint, self.url.scheme()),
            if self.upgrade { "Upgrade" } else { "close" }
        );
        if !self
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept-encoding"))
        {
            wire.push_str("Accept-Encoding: identity\r\n");
        }
        if let Some(proxy) = forward {
            wire.push_str(&proxy_authorization(proxy));
        }
        for (name, value) in &self.headers {
            wire.push_str(name);
            wire.push_str(": ");
            wire.push_str(value);
            wire.push_str("\r\n");
        }
        if !self.body.is_empty() || matches!(self.method.as_str(), "POST" | "PUT" | "PATCH") {
            wire.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        wire.push_str("\r\n");
        if wire.len() > MAX_HEADER_BYTES {
            return Err(Error::HttpInvalid);
        }
        let mut bytes = wire.into_bytes();
        bytes.extend_from_slice(&self.body);
        Ok(bytes)
    }
}

#[derive(Default)]
struct RedirectHistory(BTreeMap<String, usize>);

impl RedirectHistory {
    fn visit(&mut self, target: &Url) -> Result<(), Error> {
        let count = self.0.get(target.as_str()).copied().unwrap_or_default();
        // urllib.HTTPRedirectHandler.max_repeats / max_redirections.
        if count >= 4 || self.0.len() >= 10 {
            return Err(Error::RedirectLimit);
        }
        self.0.insert(target.as_str().to_owned(), count + 1);
        Ok(())
    }
}

fn method_label(method: &str) -> Result<&'static str, Error> {
    match method {
        "GET" => Ok("GET"),
        "HEAD" => Ok("HEAD"),
        "POST" => Ok("POST"),
        "PUT" => Ok("PUT"),
        "PATCH" => Ok("PATCH"),
        "DELETE" => Ok("DELETE"),
        "OPTIONS" => Ok("OPTIONS"),
        _ => Err(Error::HttpInvalid),
    }
}

fn token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn parse_url(raw: &str) -> Result<Url, Error> {
    // The URL parser repairs some control characters; refuse before normalization.
    if raw.bytes().any(|b| b <= 32 || b == 127) || raw.contains('\\') {
        return Err(Error::HttpInvalid);
    }
    // urllib does not provide uniform IDN normalization (non-latin-1 hosts
    // fail header encoding). Require the same reviewed ASCII Endpoint identity.
    let authority = raw
        .split_once("://")
        .ok_or(Error::HttpInvalid)?
        .1
        .split(['/', '?', '#'])
        .next()
        .ok_or(Error::HttpInvalid)?;
    if !authority.is_ascii() {
        return Err(Error::HttpInvalid);
    }
    let url = Url::parse(raw).map_err(|_| Error::HttpInvalid)?;
    validate_url(&url)?;
    Ok(url)
}

fn validate_url(url: &Url) -> Result<(), Error> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::HttpInvalid);
    }
    Ok(())
}

fn authority(endpoint: &Endpoint) -> String {
    if endpoint.host.contains(':') {
        format!("[{}]:{}", endpoint.host, endpoint.port)
    } else {
        format!("{}:{}", endpoint.host, endpoint.port)
    }
}

fn host_header(endpoint: &Endpoint, scheme: &str) -> String {
    if matches!((scheme, endpoint.port), ("https", 443) | ("http", 80)) {
        if endpoint.host.contains(':') {
            format!("[{}]", endpoint.host)
        } else {
            endpoint.host.clone()
        }
    } else {
        authority(endpoint)
    }
}

fn proxy_authorization(proxy: &Proxy) -> String {
    proxy
        .credentials
        .as_ref()
        .map_or(String::new(), |(user, password)| {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
            format!("Proxy-Authorization: Basic {encoded}\r\n")
        })
}

/// CONNECT sends only proxy credentials, never target Authorization or cookies.
/// Target DNS belongs to the proxy, and target SNI/trust apply after the tunnel.
pub(crate) fn tunnel(transport: &mut Transport) -> Result<(), Error> {
    let proxy = transport
        .route
        .request
        .proxy
        .as_ref()
        .ok_or(Error::ProxyUnsupported)?;
    let authority = authority(&transport.route.request.endpoint);
    let request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{}\r\n",
        proxy_authorization(proxy)
    );
    transport.write_context = crate::WriteContext::ProxyHandshake;
    transport.write_all(request.as_bytes())?;
    transport.flush()?;
    let (header, pending) = read_tunnel_head(transport)?;
    transport.pending = io::Cursor::new(pending);
    let (status, _, _) = parse_head(&header)?;
    if status != 200 {
        return Err(Error::ProxyUnsupported);
    }
    transport.write_context = crate::WriteContext::Raw;
    Ok(())
}

fn read_tunnel_head(reader: &mut impl Read) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let mut header = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let search = header.len().saturating_sub(3);
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Err(Error::HttpInvalid);
        }
        header.extend_from_slice(&buffer[..count]);
        if let Some(index) = header[search..]
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
        {
            let end = search + index + 4;
            if end > MAX_HEADER_BYTES {
                return Err(Error::HttpInvalid);
            }
            let pending = header.split_off(end);
            return Ok((header, pending));
        }
        if header.len() >= MAX_HEADER_BYTES {
            return Err(Error::HttpInvalid);
        }
    }
}

/// Untrusted response data: deliberately no Debug/Display or error-source exposure.
pub struct HttpResponse {
    pub status: u16,
    /// Untrusted HTTP reason phrase, for explicit probe output only. Never
    /// included in a transport error or ledger record.
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    upgraded: Option<Upgrade>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn into_upgrade(self) -> Option<Upgrade> {
        self.upgraded
    }
}

/// Preserves any bytes received with 101 and keeps all subsequent writes gated.
pub struct Upgrade(BufReader<Connection>);

impl Read for Upgrade {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}

impl Write for Upgrade {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.get_mut().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.get_mut().flush()
    }
}

/// Stops an [`HttpRequest`] made [`HttpRequest::cancellable`] from another
/// thread. Clones share the request. Cancelling before the request connects
/// makes it fail before any socket is opened; cancelling while it waits on
/// the server shuts its socket down, as Python's AI client closes the
/// response from its cancel thread.
#[derive(Clone, Default)]
pub struct Cancel(Arc<CancelState>);

#[derive(Default)]
struct CancelState {
    cancelled: AtomicBool,
    /// The in-flight socket (a duplicate used only for shutdown) and the
    /// generation of the stream that owns it.
    socket: Mutex<Option<(u64, std::net::TcpStream)>>,
    generation: AtomicU64,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        let socket = self
            .0
            .socket
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some((_, socket)) = socket {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    fn register(&self, connection: &mut Connection) -> Result<(), Error> {
        let socket = connection.transport().stream.try_clone()?;
        let mut slot = self.0.socket.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_cancelled() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "request cancelled",
            )));
        }
        let generation = self.0.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *slot = Some((generation, socket));
        Ok(())
    }

    /// Releases the duplicate so the connection closes with its stream.
    fn release(&self) {
        let generation = self.0.generation.load(Ordering::SeqCst);
        let mut slot = self.0.socket.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|(owner, _)| *owner == generation) {
            *slot = None;
        }
    }
}

/// How the body of an [`HttpStream`] ends.
enum Framing {
    Empty,
    Length(usize),
    Chunked {
        remaining: usize,
        started: bool,
        done: bool,
        chunks: usize,
    },
    Close {
        done: bool,
    },
}

/// A response whose body is read incrementally: chunked transfer coding is
/// decoded as chunks arrive. Untrusted, like [`HttpResponse`]: no Debug.
/// Every byte, framing included, counts against the response limit.
pub struct HttpStream {
    pub status: u16,
    /// Untrusted HTTP reason phrase; see [`HttpResponse::reason`].
    pub reason: String,
    pub headers: Vec<(String, String)>,
    reader: BufReader<Box<dyn Read + Send>>,
    framing: Framing,
    limit: usize,
    consumed: usize,
    /// Decoded bytes read ahead by [`HttpStream::read_line`].
    pending: Vec<u8>,
    cancel: Option<Cancel>,
}

impl HttpStream {
    fn receive(connection: Connection, method: &str, limit: usize) -> Result<Self, Error> {
        Self::from_reader(Box::new(connection), method, limit)
    }

    fn from_reader(
        reader: Box<dyn Read + Send>,
        method: &str,
        limit: usize,
    ) -> Result<Self, Error> {
        let mut reader = BufReader::new(reader);
        let (status, reason, headers) = read_head(&mut reader)?;
        check_content_encoding(&headers)?;
        if status == 101 {
            return Err(Error::HttpInvalid);
        }
        let framing = if method == "HEAD" || matches!(status, 204 | 304) {
            Framing::Empty
        } else {
            framing(&headers)?
        };
        if matches!(framing, Framing::Length(length) if length > limit) {
            // As `read_body`: never wait for a body announced over the limit.
            return Err(Error::ResponseTooLarge);
        }
        Ok(Self {
            status,
            reason,
            headers,
            reader,
            framing,
            limit,
            consumed: 0,
            pending: Vec::new(),
            cancel: None,
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn charge(&mut self, count: usize) -> Result<(), Error> {
        charge(&mut self.consumed, count, self.limit)
    }

    /// Reads decoded body bytes; 0 at the end of the body.
    pub fn read_body(&mut self, bytes: &mut [u8]) -> Result<usize, Error> {
        if !self.pending.is_empty() {
            let count = bytes.len().min(self.pending.len());
            bytes[..count].copy_from_slice(&self.pending[..count]);
            self.pending.drain(..count);
            return Ok(count);
        }
        self.decode(bytes)
    }

    fn decode(&mut self, bytes: &mut [u8]) -> Result<usize, Error> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            match &mut self.framing {
                Framing::Empty => return Ok(0),
                Framing::Close { done: true } => return Ok(0),
                Framing::Close { done } => {
                    let count = match self.reader.read(bytes) {
                        Ok(count) => count,
                        // Close framing only: see `read_body`'s ragged EOF.
                        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => 0,
                        Err(error) => return Err(error.into()),
                    };
                    if count == 0 {
                        *done = true;
                        return Ok(0);
                    }
                    if count > self.limit.saturating_sub(self.consumed) {
                        return Err(Error::ResponseTooLarge);
                    }
                    self.consumed += count;
                    return Ok(count);
                }
                Framing::Length(0) => return Ok(0),
                Framing::Length(remaining) => {
                    let want = bytes.len().min(*remaining);
                    let count = self.reader.read(&mut bytes[..want])?;
                    if count == 0 {
                        return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
                    }
                    *remaining -= count;
                    self.charge(count)?;
                    return Ok(count);
                }
                Framing::Chunked { done: true, .. } => return Ok(0),
                Framing::Chunked {
                    remaining: 0,
                    started,
                    chunks,
                    ..
                } => {
                    if *started {
                        let mut end = [0; 2];
                        self.reader.read_exact(&mut end)?;
                        if end != *b"\r\n" {
                            return Err(Error::HttpInvalid);
                        }
                    }
                    if *chunks >= MAX_CHUNKS {
                        return Err(Error::HttpInvalid);
                    }
                    *chunks += 1;
                    let line = read_line(&mut self.reader, 4096)?;
                    let length = line.split(|b| *b == b';').next().unwrap();
                    let length = std::str::from_utf8(length)
                        .map_err(|_| Error::HttpInvalid)?
                        .trim();
                    if length.is_empty() || !length.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(Error::HttpInvalid);
                    }
                    let length =
                        usize::from_str_radix(length, 16).map_err(|_| Error::HttpInvalid)?;
                    self.charge(line.len())?;
                    if length == 0 {
                        let mut trailers = 0;
                        loop {
                            let trailer = read_line(
                                &mut self.reader,
                                MAX_HEADER_BYTES.saturating_sub(trailers),
                            )?;
                            trailers += trailer.len();
                            self.charge(trailer.len())?;
                            if trailer == b"\r\n" {
                                break;
                            }
                        }
                        self.framing = Framing::Chunked {
                            remaining: 0,
                            started: false,
                            done: true,
                            chunks: 0,
                        };
                        return Ok(0);
                    }
                    self.charge(length)?;
                    self.charge(2)?;
                    let Framing::Chunked {
                        remaining, started, ..
                    } = &mut self.framing
                    else {
                        unreachable!()
                    };
                    *remaining = length;
                    *started = true;
                }
                Framing::Chunked { remaining, .. } => {
                    let want = bytes.len().min(*remaining);
                    let count = self.reader.read(&mut bytes[..want])?;
                    if count == 0 {
                        return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
                    }
                    *remaining -= count;
                    return Ok(count);
                }
            }
        }
    }

    /// The next line of the body, ending at (and including) `\n`, or the
    /// unterminated rest at the end of the body; `None` once the body is
    /// read. Like iterating a Python response, only `\n` ends a line. A line
    /// longer than `max` bytes fails with [`Error::ResponseTooLarge`].
    pub fn read_line(&mut self, max: usize) -> Result<Option<Vec<u8>>, Error> {
        let mut searched = 0;
        loop {
            if let Some(index) = self.pending[searched..].iter().position(|b| *b == b'\n') {
                let end = searched + index + 1;
                if end > max {
                    return Err(Error::ResponseTooLarge);
                }
                return Ok(Some(self.pending.drain(..end).collect()));
            }
            searched = self.pending.len();
            if searched > max {
                return Err(Error::ResponseTooLarge);
            }
            let mut buffer = [0; 8192];
            let count = self.decode(&mut buffer)?;
            if count == 0 {
                return Ok((!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending)));
            }
            self.pending.extend_from_slice(&buffer[..count]);
        }
    }

    /// The rest of the body.
    pub fn read_to_end(&mut self) -> Result<Vec<u8>, Error> {
        let mut body = std::mem::take(&mut self.pending);
        let mut buffer = [0; 8192];
        loop {
            let count = self.decode(&mut buffer)?;
            if count == 0 {
                return Ok(body);
            }
            body.extend_from_slice(&buffer[..count]);
        }
    }

    fn drain(&mut self) -> Result<(), Error> {
        self.read_to_end().map(drop)
    }

    fn attach(mut self, cancel: Option<Cancel>) -> Self {
        self.cancel = cancel;
        self
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.release();
        }
    }
}

/// The body framing `read_body` accepts, decided from the head alone.
fn framing(headers: &Headers) -> Result<Framing, Error> {
    let lengths: Vec<_> = headers
        .iter()
        .filter(|(key, _)| key == "content-length")
        .collect();
    let encodings: Vec<_> = headers
        .iter()
        .filter(|(key, _)| key == "transfer-encoding")
        .collect();
    if lengths.windows(2).any(|pair| pair[0].1 != pair[1].1)
        || encodings.len() > 1
        || (!lengths.is_empty() && !encodings.is_empty())
    {
        return Err(Error::HttpInvalid);
    }
    if let Some((_, encoding)) = encodings.first() {
        if !encoding.eq_ignore_ascii_case("chunked") {
            return Err(Error::HttpInvalid);
        }
        return Ok(Framing::Chunked {
            remaining: 0,
            started: false,
            done: false,
            chunks: 0,
        });
    }
    if let Some((_, length)) = lengths.first() {
        if length.is_empty() || !length.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::HttpInvalid);
        }
        return Ok(Framing::Length(
            length.parse().map_err(|_| Error::HttpInvalid)?,
        ));
    }
    Ok(Framing::Close { done: false })
}

fn read_line(reader: &mut impl BufRead, limit: usize) -> Result<Vec<u8>, Error> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Err(Error::HttpInvalid);
        }
        let count = buffer
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buffer.len(), |i| i + 1);
        if count > limit.saturating_sub(line.len()) {
            return Err(Error::HttpInvalid);
        }
        line.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if line.ends_with(b"\n") {
            if !line.ends_with(b"\r\n") {
                return Err(Error::HttpInvalid);
            }
            return Ok(line);
        }
    }
}

type Headers = Vec<(String, String)>;

fn parse_head(head: &[u8]) -> Result<(u16, String, Headers), Error> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut response = httparse::Response::new(&mut headers);
    if !response
        .parse(head)
        .map_err(|_| Error::HttpInvalid)?
        .is_complete()
    {
        return Err(Error::HttpInvalid);
    }
    let status = response
        .code
        .filter(|status| (100..=599).contains(status))
        .ok_or(Error::HttpInvalid)?;
    let headers = response
        .headers
        .iter()
        .map(|header| {
            // http.client decodes field values with ISO-8859-1, not UTF-8.
            let value: String = header.value.iter().map(|&byte| char::from(byte)).collect();
            Ok((
                header.name.to_ascii_lowercase(),
                value.trim_matches([' ', '\t']).to_string(),
            ))
        })
        .collect::<Result<Headers, Error>>()?;
    Ok((status, response.reason.unwrap_or("").to_owned(), headers))
}

fn receive(
    connection: Connection,
    method: &str,
    upgrade: bool,
    limit: usize,
) -> Result<HttpResponse, Error> {
    let mut reader = BufReader::new(connection);
    let (status, reason, headers) = read_head(&mut reader)?;
    let mut response = HttpResponse {
        status,
        reason,
        headers,
        body: Vec::new(),
        upgraded: None,
    };
    check_content_encoding(&response.headers)?;
    if status == 101 {
        if !upgrade {
            return Err(Error::HttpInvalid);
        }
        reader.get_mut().transport().write_context = crate::WriteContext::Raw;
        response.upgraded = Some(Upgrade(reader));
        return Ok(response);
    }
    if method == "HEAD" || matches!(status, 204 | 304) {
        return Ok(response);
    }
    response.body = read_body(&mut reader, &response.headers, limit)?;
    Ok(response)
}

fn read_head(reader: &mut impl BufRead) -> Result<(u16, String, Headers), Error> {
    let mut total = 0;
    for _ in 0..=MAX_INFORMATIONAL {
        let mut head = Vec::new();
        loop {
            let line = read_line(reader, MAX_HEADER_BYTES.saturating_sub(total))?;
            total += line.len();
            head.extend_from_slice(&line);
            if line == b"\r\n" {
                break;
            }
        }
        let (status, reason, headers) = parse_head(&head)?;
        if status >= 200 || status == 101 {
            return Ok((status, reason, headers));
        }
    }
    Err(Error::HttpInvalid)
}

fn check_content_encoding(headers: &Headers) -> Result<(), Error> {
    if headers
        .iter()
        .any(|(name, value)| name == "content-encoding" && !value.eq_ignore_ascii_case("identity"))
    {
        return Err(Error::HttpInvalid);
    }
    Ok(())
}

fn read_body(reader: &mut impl BufRead, headers: &Headers, limit: usize) -> Result<Vec<u8>, Error> {
    check_content_encoding(headers)?;
    let lengths: Vec<_> = headers
        .iter()
        .filter(|(key, _)| key == "content-length")
        .collect();
    let encodings: Vec<_> = headers
        .iter()
        .filter(|(key, _)| key == "transfer-encoding")
        .collect();
    if lengths.windows(2).any(|pair| pair[0].1 != pair[1].1)
        || encodings.len() > 1
        || (!lengths.is_empty() && !encodings.is_empty())
    {
        return Err(Error::HttpInvalid);
    }
    let mut body = Vec::new();
    if let Some((_, encoding)) = encodings.first() {
        if !encoding.eq_ignore_ascii_case("chunked") {
            return Err(Error::HttpInvalid);
        }
        let mut consumed = 0usize;
        for _ in 0..MAX_CHUNKS {
            let line = read_line(reader, 4096)?;
            charge(&mut consumed, line.len(), limit)?;
            let length = line.split(|b| *b == b';').next().unwrap();
            let length = std::str::from_utf8(length)
                .map_err(|_| Error::HttpInvalid)?
                .trim();
            if length.is_empty() || !length.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::HttpInvalid);
            }
            let length = usize::from_str_radix(length, 16).map_err(|_| Error::HttpInvalid)?;
            if length == 0 {
                let mut trailers = 0;
                loop {
                    let trailer = read_line(reader, MAX_HEADER_BYTES.saturating_sub(trailers))?;
                    trailers += trailer.len();
                    charge(&mut consumed, trailer.len(), limit)?;
                    if trailer == b"\r\n" {
                        return Ok(body);
                    }
                }
            }
            charge(&mut consumed, length, limit)?;
            charge(&mut consumed, 2, limit)?;
            append_exact(reader, &mut body, length, limit)?;
            let mut end = [0; 2];
            reader.read_exact(&mut end)?;
            if end != *b"\r\n" {
                return Err(Error::HttpInvalid);
            }
        }
        return Err(Error::HttpInvalid);
    }
    if let Some((_, length)) = lengths.first() {
        if length.is_empty() || !length.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::HttpInvalid);
        }
        let length = length.parse().map_err(|_| Error::HttpInvalid)?;
        append_exact(reader, &mut body, length, limit)?;
    } else {
        let mut buffer = [0; 8192];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(count) => count,
                // Python SSL sockets default to suppress_ragged_eofs. This is
                // EOF only for close framing, never an announced body length.
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error.into()),
            };
            if count == 0 {
                break;
            }
            if count > limit.saturating_sub(body.len()) {
                return Err(Error::ResponseTooLarge);
            }
            body.extend_from_slice(&buffer[..count]);
        }
    }
    Ok(body)
}

fn charge(consumed: &mut usize, count: usize, limit: usize) -> Result<(), Error> {
    if count > limit.saturating_sub(*consumed) {
        return Err(Error::ResponseTooLarge);
    }
    *consumed += count;
    Ok(())
}

fn append_exact(
    reader: &mut impl Read,
    body: &mut Vec<u8>,
    count: usize,
    limit: usize,
) -> Result<(), Error> {
    if count > limit.saturating_sub(body.len()) {
        return Err(Error::ResponseTooLarge);
    }
    // Grow only with bytes actually received, never allocate an announced length.
    let mut remaining = count;
    let mut buffer = [0; 8192];
    while remaining != 0 {
        let count = remaining.min(buffer.len());
        reader.read_exact(&mut buffer[..count])?;
        body.extend_from_slice(&buffer[..count]);
        remaining -= count;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn redirect_loops_obey_urllib_repeat_and_destination_limits() {
        let target = Url::parse("https://provider.invalid/loop").unwrap();
        let mut history = RedirectHistory::default();
        for _ in 0..4 {
            history.visit(&target).unwrap();
        }
        assert!(matches!(history.visit(&target), Err(Error::RedirectLimit)));
        let mut history = RedirectHistory::default();
        for n in 0..10 {
            history
                .visit(&target.join(&format!("/{n}")).unwrap())
                .unwrap();
        }
        assert!(matches!(history.visit(&target), Err(Error::RedirectLimit)));
    }

    #[test]
    fn framing_and_limits_fail_closed_for_success_and_error_responses() {
        for (header, bytes, limit, expected) in [
            ("Content-Length: 3\r\n", "abc", 3, Some(b"abc".as_slice())),
            (
                "Transfer-Encoding: chunked\r\n",
                "3\r\nabc\r\n0\r\n\r\n",
                32,
                Some(b"abc".as_slice()),
            ),
            ("", "abc", 3, Some(b"abc".as_slice())),
            ("Content-Length: 4\r\n", "abcd", 3, None),
            (
                "Transfer-Encoding: chunked\r\n",
                "4\r\nabcd\r\n0\r\n\r\n",
                3,
                None,
            ),
            ("", "abcd", 3, None),
            (
                "Content-Length: 3\r\nContent-Length: 3\r\n",
                "abc",
                3,
                Some(b"abc".as_slice()),
            ),
            ("Content-Length: 3\r\nContent-Length: 4\r\n", "abc", 3, None),
            (
                "Content-Length: 3\r\nTransfer-Encoding: chunked\r\n",
                "abc",
                3,
                None,
            ),
            ("Transfer-Encoding: gzip\r\n", "abc", 3, None),
            ("Content-Length: 3\r\n", "ab", 3, None),
        ] {
            let head = format!("HTTP/1.1 500 Error\r\n{header}\r\n");
            let (_, _, headers) = parse_head(head.as_bytes()).unwrap();
            let result = read_body(&mut Cursor::new(bytes), &headers, limit);
            match expected {
                Some(body) => assert_eq!(result.unwrap(), body),
                None => assert!(result.is_err()),
            }
        }
    }

    #[test]
    fn parsing_is_bounded_and_rejects_injection() {
        for raw in [
            "http://u:secret@host/",
            "http://host/\r\nsecret",
            "http://host/#secret",
            "file:///tmp/file",
        ] {
            assert!(HttpRequest::new("GET", raw).is_err());
        }
        for (name, value) in [
            ("Authorization", "secret\r\nX: injected"),
            ("Host", "other"),
            ("Proxy-Authorization", "secret"),
            ("Content-Length", "5"),
        ] {
            assert!(HttpRequest::new("GET", "https://host/")
                .unwrap()
                .header(name, value)
                .is_err());
        }
        assert!(read_head(&mut Cursor::new(format!(
            "HTTP/1.1 200 OK\r\nX: {}\r\n\r\n",
            "x".repeat(MAX_HEADER_BYTES)
        )))
        .is_err());
    }
    #[test]
    fn many_small_chunks_fit_body_budget_with_individual_framing_bounds() {
        let mut wire = Vec::new();
        for _ in 0..15 * 1024 {
            wire.extend_from_slice(b"400\r\n");
            wire.extend_from_slice(&[b'x'; 1024]);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b"0\r\n\r\n");
        let headers = vec![("transfer-encoding".into(), "chunked".into())];
        assert_eq!(
            read_body(&mut Cursor::new(wire), &headers, 16 * 1024 * 1024)
                .unwrap()
                .len(),
            15 * 1024 * 1024
        );
        for wire in [
            format!("1;{}\r\nx\r\n0\r\n\r\n", "x".repeat(4096)),
            format!("0\r\nX: {}\r\n\r\n", "x".repeat(MAX_HEADER_BYTES)),
            "1;extension=counts-against-budget\r\nx\r\n0\r\n\r\n".into(),
        ] {
            assert!(read_body(&mut Cursor::new(wire), &headers, 32).is_err());
        }
    }

    #[test]
    fn response_headers_decode_latin1_and_refuse_content_coding() {
        let (_, _, headers) = parse_head(b"HTTP/1.1 200 OK\r\nX-Name: caf\xe9\r\n\r\n").unwrap();
        assert_eq!(headers[0].1, "caf\u{e9}");
        for coding in ["gzip", "br", "identity, gzip"] {
            let headers = vec![("content-encoding".into(), coding.into())];
            assert!(read_body(&mut Cursor::new(b"compressed"), &headers, 100).is_err());
        }
    }

    #[test]
    fn request_defaults_match_python_host_and_accept_encoding() {
        use crate::{Consent, Gate, Ledger, Offline, Request, Subsystem};
        for (url, host, port, expected) in [
            (
                "https://api.example.com/",
                "api.example.com",
                443,
                "api.example.com",
            ),
            (
                "http://api.example.com/",
                "api.example.com",
                80,
                "api.example.com",
            ),
            (
                "https://api.example.com:8443/",
                "api.example.com",
                8443,
                "api.example.com:8443",
            ),
            ("https://[::1]/", "::1", 443, "[::1]"),
        ] {
            let tls = if url.starts_with("https:") {
                TlsTrust::BundledRoots
            } else {
                TlsTrust::Plaintext
            };
            let req = Request::new(Subsystem::Sync, Endpoint::new(host, port).unwrap(), tls);
            let proof = Consent::explicit_trigger(&req);
            let route = Gate::new(|| Offline::Online, Ledger::default())
                .authorize(req, Some(proof))
                .unwrap();
            let wire = HttpRequest::new("GET", url)
                .unwrap()
                .encode(&route)
                .unwrap();
            let wire = String::from_utf8(wire).unwrap();
            assert!(wire.contains(&format!("Host: {expected}\r\n")));
            assert!(wire.contains("Accept-Encoding: identity\r\n"));
        }
    }
    #[test]
    fn ragged_tls_eof_is_accepted_only_for_close_delimited_bodies() {
        struct Ragged(Cursor<Vec<u8>>);
        impl Read for Ragged {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let count = self.0.read(bytes)?;
                if count == 0 {
                    Err(io::ErrorKind::UnexpectedEof.into())
                } else {
                    Ok(count)
                }
            }
        }
        let reader = || BufReader::new(Ragged(Cursor::new(b"abc".to_vec())));
        assert_eq!(read_body(&mut reader(), &vec![], 3).unwrap(), b"abc");
        assert!(read_body(
            &mut reader(),
            &vec![("content-length".into(), "4".into())],
            4
        )
        .is_err());
        assert!(read_body(
            &mut reader(),
            &vec![("transfer-encoding".into(), "chunked".into())],
            100
        )
        .is_err());
    }
    #[test]
    fn urls_require_reviewed_ascii_hosts_and_allow_unicode_paths() {
        for raw in [
            "https://münich.example/",
            "https://例.example/",
            "https://%c3%bc.example/",
        ] {
            assert!(HttpRequest::new("GET", raw).is_err());
        }
        for raw in [
            "https://xn--mnich-kva.example/",
            "http://docker_service/",
            "https://host/ü",
        ] {
            assert!(HttpRequest::new("GET", raw).is_ok());
        }
        assert!(Endpoint::new("docker_service", 80).is_ok());
    }
    fn stream(head: &str, body: &[u8], limit: usize) -> Result<HttpStream, Error> {
        let mut wire = head.as_bytes().to_vec();
        wire.extend_from_slice(body);
        HttpStream::from_reader(Box::new(Cursor::new(wire)), "POST", limit)
    }

    fn lines(stream: &mut HttpStream, max: usize) -> Result<Vec<Vec<u8>>, Error> {
        let mut lines = Vec::new();
        while let Some(line) = stream.read_line(max)? {
            lines.push(line);
        }
        Ok(lines)
    }

    #[test]
    fn streamed_bodies_decode_like_buffered_ones_and_split_lines_only_on_lf() {
        let body = b"data: a\r\n\r\ndata: b\rc\n\nlast";
        let mut chunked = Vec::new();
        for piece in body.chunks(3) {
            chunked.extend_from_slice(format!("{:x};ext=1\r\n", piece.len()).as_bytes());
            chunked.extend_from_slice(piece);
            chunked.extend_from_slice(b"\r\n");
        }
        chunked.extend_from_slice(b"0\r\nTrailer: x\r\n\r\n");
        let expected: Vec<&[u8]> = vec![b"data: a\r\n", b"\r\n", b"data: b\rc\n", b"\n", b"last"];
        for (head, wire) in [
            (
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
                &chunked[..],
            ),
            (
                &*format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()),
                &body[..],
            ),
            ("HTTP/1.1 200 OK\r\n\r\n", &body[..]),
        ] {
            let mut stream = stream(head, wire, 1024).unwrap();
            assert_eq!(lines(&mut stream, 64).unwrap(), expected);
            assert!(stream.read_line(64).unwrap().is_none());
            let mut stream = self::stream(head, wire, 1024).unwrap();
            assert_eq!(stream.read_to_end().unwrap(), body);
        }
    }

    #[test]
    fn streamed_bodies_fail_closed_on_framing_limits_and_long_lines() {
        for (head, wire, limit) in [
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n",
                &b"abc"[..],
                100,
            ),
            ("HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n", b"abcd", 3),
            (
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
                b"4\r\nab",
                100,
            ),
            (
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
                b"2\r\nabXX0\r\n\r\n",
                100,
            ),
            (
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
                b"zz\r\nab\r\n",
                100,
            ),
            (
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
                b"4\r\nabcd\r\n0\r\n\r\n",
                6,
            ),
            ("HTTP/1.1 200 OK\r\n\r\n", b"abcd", 3),
        ] {
            let result = stream(head, wire, limit).and_then(|mut s| s.read_to_end());
            assert!(result.is_err(), "{head:?} {wire:?}");
        }
        for head in [
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n",
            "HTTP/1.1 101 Switching\r\n\r\n",
        ] {
            assert!(stream(head, b"", 100).is_err(), "{head:?}");
        }
        let mut long = stream("HTTP/1.1 200 OK\r\n\r\n", b"abcdef\nx", 100).unwrap();
        assert!(matches!(long.read_line(5), Err(Error::ResponseTooLarge)));
        let mut unterminated = stream("HTTP/1.1 200 OK\r\n\r\n", b"abcdef", 100).unwrap();
        assert!(matches!(
            unterminated.read_line(5),
            Err(Error::ResponseTooLarge)
        ));
        let mut exact = stream("HTTP/1.1 200 OK\r\n\r\n", b"abcd\n", 100).unwrap();
        assert_eq!(exact.read_line(5).unwrap().unwrap(), b"abcd\n");
    }

    #[test]
    fn a_live_stream_yields_lines_as_they_arrive_and_cancel_shuts_it_down() {
        use crate::{Consent, Gate, Ledger, Offline, Request, Subsystem};
        use std::net::TcpListener;
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sent, release) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n6\r\nfirst\n\r\n")
                .unwrap();
            // Hold the rest until the client has read the first line.
            let _ = release.recv();
            let mut rest = Vec::new();
            let _ = socket.read_to_end(&mut rest);
        });
        let request = Request::new(
            Subsystem::Ai,
            Endpoint::new("127.0.0.1", port).unwrap(),
            TlsTrust::Plaintext,
        );
        let proof = Consent::explicit_trigger(&request);
        let route = Gate::new(|| Offline::Online, Ledger::default())
            .authorize(request, Some(proof))
            .unwrap();
        let cancel = Cancel::new();
        let mut stream = HttpRequest::new("POST", &format!("http://127.0.0.1:{port}/v1/responses"))
            .unwrap()
            .cancellable(&cancel)
            .open(route, HttpOptions::default())
            .unwrap();
        assert_eq!(stream.status, 200);
        assert_eq!(stream.read_line(64).unwrap().unwrap(), b"first\n");
        let canceller = cancel.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            canceller.cancel();
        });
        let started = Instant::now();
        assert!(stream.read_line(64).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
        stopper.join().unwrap();
        drop(stream);
        sent.send(()).unwrap();
        server.join().unwrap();
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn a_cancel_before_connect_opens_no_socket() {
        use crate::{Consent, Gate, Ledger, Offline, Request, Subsystem};
        let request = Request::new(
            Subsystem::Ai,
            Endpoint::new("127.0.0.1", 9).unwrap(),
            TlsTrust::Plaintext,
        );
        let proof = Consent::explicit_trigger(&request);
        let gate = Gate::new(|| Offline::Online, Ledger::default());
        let route = gate.authorize(request, Some(proof)).unwrap();
        let cancel = Cancel::new();
        cancel.cancel();
        let result = HttpRequest::new("GET", "http://127.0.0.1:9/v1/models")
            .unwrap()
            .cancellable(&cancel)
            .open(route, HttpOptions::default());
        assert!(matches!(result, Err(Error::Io(_))));
        assert!(gate.ledger().snapshot(0, 100).records.is_empty());
    }

    #[test]
    fn connect_headers_read_in_blocks_and_preserve_tunnel_bytes() {
        struct Counted {
            data: Cursor<Vec<u8>>,
            calls: usize,
        }
        impl Read for Counted {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.data.read(bytes)
            }
        }
        let header = format!("HTTP/1.1 200 Connected\r\nX: {}\r\n\r\n", "x".repeat(5000));
        let mut data = header.as_bytes().to_vec();
        data.extend_from_slice(b"early-tunnel-bytes");
        let mut reader = Counted {
            data: Cursor::new(data),
            calls: 0,
        };
        let (parsed, pending) = read_tunnel_head(&mut reader).unwrap();
        assert_eq!(parsed, header.as_bytes());
        assert_eq!(pending, b"early-tunnel-bytes");
        assert!(reader.calls <= 2, "{} CONNECT reads", reader.calls);
    }
}
