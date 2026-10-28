//! Deliberately one HTTP/1.1 request per connection. No pool, background work,
//! retries, cookies, proxy discovery, decompression, or implicit credentials.
use crate::{remaining, Connection, Endpoint, Error, Proxy, ProxyKind, Route, TlsTrust, Transport};
use base64::Engine;
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
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

    pub fn send(mut self, route: Route, options: HttpOptions) -> Result<HttpResponse, Error> {
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
            let mut connection = next.connect_inner(timeout, true)?;
            connection.set_deadline(options.deadline);
            connection.write_all(&wire)?;
            connection.flush()?;
            wire.fill(0);
            let response = receive(
                connection,
                &self.method,
                self.upgrade,
                options.max_response_bytes.min(MAX_RESPONSE_BYTES),
            )?;
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let Some(location) = response.header("location") else {
                return Ok(response);
            };
            if options.redirects == RedirectPolicy::Refuse {
                return Err(Error::RedirectRefused);
            }
            let target = self.url.join(location).map_err(|_| Error::HttpInvalid)?;
            validate_url(&target)?;
            if target.origin() != self.url.origin() {
                return Err(Error::RedirectRefused);
            }
            // urllib does not redirect POST on 307/308 or arbitrary mutating verbs.
            if !matches!(self.method.as_str(), "GET" | "HEAD") {
                if self.method != "POST" || !matches!(response.status, 301..=303) {
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
    transport.write_all(request.as_bytes())?;
    transport.flush()?;
    // Read exactly the headers; never buffer TLS handshake bytes past CONNECT.
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= MAX_HEADER_BYTES {
            return Err(Error::HttpInvalid);
        }
        let mut byte = [0];
        transport.read_exact(&mut byte)?;
        header.push(byte[0]);
    }
    let (status, _) = parse_head(&header)?;
    if status != 200 {
        return Err(Error::ProxyUnsupported);
    }
    Ok(())
}

/// Untrusted response data: deliberately no Debug/Display or error-source exposure.
pub struct HttpResponse {
    pub status: u16,
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

fn parse_head(head: &[u8]) -> Result<(u16, Headers), Error> {
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
    Ok((status, headers))
}

fn receive(
    connection: Connection,
    method: &str,
    upgrade: bool,
    limit: usize,
) -> Result<HttpResponse, Error> {
    let mut reader = BufReader::new(connection);
    let (status, headers) = read_head(&mut reader)?;
    let mut response = HttpResponse {
        status,
        headers,
        body: Vec::new(),
        upgraded: None,
    };
    check_content_encoding(&response.headers)?;
    if status == 101 {
        if !upgrade {
            return Err(Error::HttpInvalid);
        }
        response.upgraded = Some(Upgrade(reader));
        return Ok(response);
    }
    if method == "HEAD" || matches!(status, 204 | 304) {
        return Ok(response);
    }
    response.body = read_body(&mut reader, &response.headers, limit)?;
    Ok(response)
}

fn read_head(reader: &mut impl BufRead) -> Result<(u16, Headers), Error> {
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
        let (status, headers) = parse_head(&head)?;
        if status >= 200 || status == 101 {
            return Ok((status, headers));
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
            let count = reader.read(&mut buffer)?;
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
            let (_, headers) = parse_head(head.as_bytes()).unwrap();
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
        let (_, headers) = parse_head(b"HTTP/1.1 200 OK\r\nX-Name: caf\xe9\r\n\r\n").unwrap();
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
}
