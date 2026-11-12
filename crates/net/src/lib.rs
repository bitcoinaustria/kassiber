//! All Rust backend connections must start with an authorized [`Route`].
//! Construction and display of configuration do no I/O. The application calls
//! [`Consent::explicit_trigger`] only at a verified user-action boundary, or
//! [`Consent::narrowly_scoped`] after checking the corresponding feature setting.
//! These are attestations by trusted application code, not authentication of a user.
//!
//! ```no_run
//! use kassiber_net::{Consent, Endpoint, Gate, Ledger, Offline, Request, Subsystem, TlsTrust};
//! use std::io::Write;
//! use std::time::Duration;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Supply a fresh, fail-closed read of the machine's offline preference.
//! // Never infer consent from a saved backend or from creating this Gate.
//! let gate = Gate::new(|| Offline::Online, Ledger::default());
//! let request = Request::new(Subsystem::Sync,
//!     Endpoint::new("127.0.0.1", 50001)?, TlsTrust::Plaintext);
//! let proof = Consent::explicit_trigger(&request); // verified explicit sync click
//! let route = gate.authorize(request, Some(proof))?;
//! let mut connection = route.connect(Duration::from_secs(8))?;
//! connection.write_all(b"consented request\n")?; // gate is rechecked on writes
//! # Ok(()) }
//! ```
//!
//! [`HttpRequest::send`] consumes a route and uses HTTP/1.1 over the same guarded
//! stream, including TLS handshake and post-handshake writes. Ambient proxies
//! are never read. Redirects default to refusal. No raw socket is exposed.

mod broker;
mod http;
mod ledger;
mod offline;
mod pinned;
mod process;
mod socks;
mod tls;

#[cfg(feature = "test-support")]
pub mod test_support;

#[cfg(unix)]
pub use broker::temp_parent_for;
pub use broker::{
    broker_script, broker_temp_parent, mount_fstype, node_executable, splitlines, which,
    BrokerError, BrokerOutput, BrokerProcess, BrokerProgram, BrokerSignal, BROKER_GRACE,
    BROKER_NODE_ENV, BROKER_SCRIPT_ENV, BROKER_TMPDIR_ENV,
};
pub use http::{
    Cancel, HttpOptions, HttpRequest, HttpResponse, HttpStream, RedirectPolicy, Upgrade,
};
pub use offline::OfflineCache;
pub use process::{ClnConfig, ClnMethod, ClnProcessError};
pub use tls::{RootCertificates, TlsTrust};
// Shared pure URL parsing keeps the reviewed URL dependency in this crate.
pub use url::Url as HttpUrl;

use ledger::WriteContext;
pub use ledger::{Ledger, Operation, Record, Snapshot, Subsystem, MAX_BYTES, MAX_RECORDS};
use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    /// Accept only a host, never a URL, userinfo, path, query, fragment or zone ID.
    /// IDNs must be supplied as reviewed ASCII A-labels; no implicit IDNA conversion.
    pub fn new(host: &str, port: u16) -> Result<Self, Error> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let ip = host.parse::<IpAddr>().is_ok();
        let dns = !host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            });
        if port == 0 || (!ip && !dns) {
            return Err(Error::InvalidEndpoint);
        }
        Ok(Self { host, port })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn loopback(&self) -> bool {
        self.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
            || matches!(
                self.host.as_str(),
                "localhost" | "ip6-localhost" | "ip6-loopback"
            )
    }
}

/// Explicit configuration only. No environment/OS proxy or NO_PROXY is read.
#[derive(Clone, PartialEq, Eq)]
pub struct Proxy {
    endpoint: Endpoint,
    credentials: Option<(String, String)>,
    kind: ProxyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxyKind {
    Socks5,
    Http,
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proxy")
            .field("endpoint", &self.endpoint)
            .field("kind", &self.kind)
            .field("authenticated", &self.credentials.is_some())
            .finish()
    }
}

impl Proxy {
    /// SOCKS5 always sends a domain target to the proxy, including `socks5` spelling.
    /// HTTP proxies use CONNECT for TLS; HTTPS proxies are refused before I/O.
    /// Supply decoded credentials separately so they never enter diagnostic URLs.
    pub fn new(
        scheme: &str,
        endpoint: Endpoint,
        credentials: Option<(String, String)>,
    ) -> Result<Self, Error> {
        let kind = match scheme {
            "socks5" | "socks5h" => ProxyKind::Socks5,
            "http" => ProxyKind::Http,
            "https" => return Err(Error::HttpsProxyUnsupported),
            _ => return Err(Error::ProxyUnsupported),
        };
        if let Some((user, password)) = &credentials {
            if (kind == ProxyKind::Socks5
                && (user.is_empty()
                    || password.is_empty()
                    || user.len() > 255
                    || password.len() > 255))
                || user.len() > 8192
                || password.len() > 8192
            {
                return Err(Error::InvalidProxyCredentials);
            }
        }
        if endpoint.host.ends_with(".onion") {
            return Err(Error::ProxyUnsupported); // never resolve a proxy's onion locally
        }
        Ok(Self {
            endpoint,
            credentials,
            kind,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    subsystem: Subsystem,
    endpoint: Endpoint,
    proxy: Option<Proxy>,
    tls: TlsTrust,
    on_device_ai: bool,
    action_binding: Option<[u8; 32]>,
    cln: Option<ClnConfig>,
    broker: Option<BrokerProgram>,
}

impl Request {
    pub fn new(subsystem: Subsystem, endpoint: Endpoint, tls: TlsTrust) -> Self {
        Self {
            subsystem,
            endpoint,
            tls,
            proxy: None,
            on_device_ai: false,
            action_binding: None,
            cln: None,
            broker: None,
        }
    }

    pub fn with_proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    pub fn with_action_binding(mut self, binding: [u8; 32]) -> Self {
        self.action_binding = Some(binding);
        self
    }

    /// Classify an explicitly local AI provider, requiring loopback and no proxy.
    /// This classification grants no offline exception or network authority.
    pub fn on_device_ai(mut self) -> Self {
        self.on_device_ai = true;
        self
    }
}

/// An exact-route grant; construction is a trusted boundary attestation.
/// Clones share revocation. Never persist this token or restore it from jobs.
#[derive(Clone)]
pub struct Consent {
    request: Request,
    active: Arc<AtomicBool>,
    kind: ConsentKind,
}

#[derive(Clone, Copy, Debug)]
pub enum ConsentKind {
    ExplicitTrigger,
    NarrowlyScoped,
}

impl Consent {
    pub fn explicit_trigger(request: &Request) -> Self {
        Self::new(request, ConsentKind::ExplicitTrigger)
    }

    pub fn narrowly_scoped(request: &Request) -> Self {
        Self::new(request, ConsentKind::NarrowlyScoped)
    }

    fn new(request: &Request, kind: ConsentKind) -> Self {
        Self {
            request: request.clone(),
            active: Arc::new(AtomicBool::new(true)),
            kind,
        }
    }

    /// Call on feature disable, cancellation, project lock/switch or stale inputs.
    pub fn revoke(&self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offline {
    Online,
    Offline,
    /// Missing/unreadable/invalid preference resolution must fail closed.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Observer {
    Disabled,
    LoopbackAllowed,
    Strict,
}

/// Own one gate per application process and share its ledger across callers.
/// The closure must read the persisted offline switch afresh and return
/// Unavailable on read/validation failure. No production preference parser is
/// supplied in R0; storage integration must provide it before adopting the gate.
#[derive(Clone)]
pub struct Gate {
    offline: Arc<dyn Fn() -> Offline + Send + Sync>,
    ledger: Ledger,
    attempts: Ledger,
    observer: Observer,
    cln_fixture: Option<std::path::PathBuf>,
    broker_fixture: Option<std::path::PathBuf>,
}

fn env_value(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(_) => guard_configuration_failure(),
    }
}

fn guard_configuration_failure() -> ! {
    eprintln!("Kassiber test no-egress guard could not start");
    std::process::exit(78)
}

fn no_egress() -> bool {
    matches!(
        env_value("KASSIBER_NO_EGRESS")
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

impl Gate {
    /// An armed test environment requires a guard-capable build. Any nonempty
    /// test value arms it, matching Python child sitecustomize; `strict` also
    /// blocks loopback. The shared product override is destination-blind for
    /// Sync/Pricing; AI/Update retain Python's separate online gate.
    pub fn new(offline: impl Fn() -> Offline + Send + Sync + 'static, ledger: Ledger) -> Self {
        let test = env_value("KASSIBER_TEST_NO_EGRESS");
        let product = env_value("KASSIBER_NO_EGRESS");
        let mode = if test.is_empty() { product } else { test };
        let observer = if mode.is_empty() {
            Observer::Disabled
        } else if cfg!(any(test, feature = "test-observer")) {
            if mode.trim().eq_ignore_ascii_case("strict") {
                Observer::Strict
            } else {
                Observer::LoopbackAllowed
            }
        } else if !env_value("KASSIBER_TEST_NO_EGRESS").is_empty() {
            guard_configuration_failure()
        } else {
            // The product switch does not require a test build.
            Observer::Disabled
        };
        Self {
            offline: Arc::new(offline),
            ledger,
            attempts: Ledger::default(),
            observer,
            cln_fixture: None,
            broker_fixture: None,
        }
    }

    /// Use only with an authoritative cross-process revision/invalidation source.
    /// Otherwise retain `new` and its fresh preference reader.
    pub fn with_offline_cache(cache: OfflineCache, ledger: Ledger) -> Self {
        Self::new(move || cache.read(), ledger)
    }

    pub fn ledger(&self) -> Ledger {
        self.ledger.clone()
    }

    /// Observer attempts are independent of the product ledger. Policy refusals
    /// do not count as network attempts; tests assert both buffers independently.
    pub fn attempts(&self) -> Snapshot {
        self.attempts.snapshot(0, 2_000)
    }

    /// Arm the attempt observer whatever the environment says, for service
    /// tests that count launches and connections.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_test_observer(mut self, strict: bool) -> Self {
        self.observer = if strict {
            Observer::Strict
        } else {
            Observer::LoopbackAllowed
        };
        self
    }

    pub fn authorize(&self, request: Request, consent: Option<Consent>) -> Result<Route, Error> {
        let consent = consent.ok_or(Error::ConsentMissing)?;
        let route = Route {
            gate: self.clone(),
            request,
            consent,
        };
        route.check()?;
        Ok(route)
    }

    fn observe(&self, request: &Request, endpoint: &Endpoint, operation: Operation) {
        if self.observer == Observer::Disabled {
            return;
        }
        self.attempts.record(request, endpoint, operation, 0);
        if self.observer == Observer::Strict || !endpoint.loopback() {
            // Panic, rather than an io::Error that product retry/envelope code
            // can swallow. The harness also asserts the independent attempt log.
            std::panic::panic_any(TestEgressBlocked);
        }
    }
}

/// Harness-only trip marker; no host, URL or credentials in panic output.
#[derive(Debug)]
pub struct TestEgressBlocked;

/// Cannot be constructed or retargeted outside this crate. No Clone or raw socket escape.
pub struct Route {
    gate: Gate,
    request: Request,
    consent: Consent,
}

impl Route {
    pub fn consent_kind(&self) -> ConsentKind {
        self.consent.kind
    }

    fn check(&self) -> Result<(), Error> {
        // Python's AI/update paths use require_online, outside the product override.
        if !matches!(self.request.subsystem, Subsystem::Ai | Subsystem::Update) && no_egress() {
            return Err(Error::NoEgress);
        }
        let on_device = self.request.on_device_ai
            && self.request.subsystem == Subsystem::Ai
            && (self.request.endpoint.host == "localhost"
                || self
                    .request
                    .endpoint
                    .host
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()))
            && self.request.proxy.is_none();
        if self.request.on_device_ai && !on_device {
            return Err(Error::InvalidLocalProvider);
        }
        match (self.gate.offline)() {
            Offline::Online => {}
            Offline::Offline if on_device => {}
            Offline::Offline => return Err(Error::Offline),
            Offline::Unavailable => return Err(Error::OfflineUnavailable),
        }
        if !self.consent.active.load(Ordering::SeqCst) {
            return Err(Error::ConsentRevoked);
        }
        if self.consent.request != self.request {
            return Err(Error::ConsentScope);
        }
        // LND uses the Sync subsystem, as in the Python ledger.
        if self.request.tls == TlsTrust::ExplicitInsecure
            && self.request.subsystem != Subsystem::Sync
        {
            return Err(Error::TrustInvalid);
        }
        if self.request.endpoint.host.ends_with(".onion") && self.request.proxy.is_none() {
            return Err(Error::OnionProxyRequired);
        }
        Ok(())
    }

    /// The only connection constructor. TLS is layered over the private guarded
    /// transport so every handshake/record write rechecks the original route.
    pub fn connect(self, timeout: Duration) -> Result<Connection, Error> {
        self.connect_inner(timeout, false)
    }

    fn connect_inner(self, timeout: Duration, http_forward: bool) -> Result<Connection, Error> {
        self.check()?;
        if self.request.cln.is_some() || self.request.broker.is_some() {
            return Err(Error::InvalidEndpoint);
        }
        let config = tls::config(&self.request.tls, http_forward)?;
        if self
            .request
            .proxy
            .as_ref()
            .is_some_and(|p| p.kind == ProxyKind::Http)
            && !http_forward
        {
            // Python raw backend sockets accept only SOCKS5, even for TLS.
            return Err(Error::ProxyUnsupported);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(Error::InvalidTimeout)?;
        remaining(deadline)?;
        let endpoint = self
            .request
            .proxy
            .as_ref()
            .map_or(&self.request.endpoint, |p| &p.endpoint);
        let addresses = if let Ok(ip) = endpoint.host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, endpoint.port)]
        } else if endpoint.loopback() {
            // Never send localhost aliases to a nameserver.
            vec![
                SocketAddr::from(([127, 0, 0, 1], endpoint.port)),
                SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], endpoint.port)),
            ]
        } else {
            self.check()?;
            self.gate.observe(&self.request, endpoint, Operation::Dns);
            self.gate
                .ledger
                .record(&self.request, endpoint, Operation::Dns, 0);
            // This is the sole system resolver boundary. SOCKS target names
            // never reach it. std's blocking resolver cannot enforce a deadline.
            (endpoint.host.as_str(), endpoint.port)
                .to_socket_addrs()?
                .collect()
        };
        let mut last = None;
        for address in addresses {
            self.check()?;
            self.gate
                .observe(&self.request, endpoint, Operation::Connect);
            self.gate
                .ledger
                .record(&self.request, endpoint, Operation::Connect, 0);
            match TcpStream::connect_timeout(&address, remaining(deadline)?) {
                Ok(stream) => {
                    let mut connection = Transport {
                        stream,
                        route: self,
                        timeout,
                        deadline: Some(deadline),
                        write_context: WriteContext::Raw,
                        read_timeout: None,
                        write_timeout: None,
                        in_operation: false,
                        pending: io::Cursor::new(Vec::new()),
                    };
                    if let Some(proxy) = &connection.route.request.proxy {
                        match proxy.kind {
                            ProxyKind::Socks5 => socks::handshake(&mut connection, deadline)?,
                            ProxyKind::Http if config.is_some() => http::tunnel(&mut connection)?,
                            ProxyKind::Http => {}
                        }
                    }
                    let mut connection = tls::wrap(connection, config)?;
                    connection.set_deadline(None);
                    return Ok(connection);
                }
                Err(error) => last = Some(error),
            }
        }
        Err(Error::Io(last.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no route addresses")
        })))
    }
}

fn remaining(deadline: Instant) -> Result<Duration, Error> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or(Error::InvalidTimeout)
}

/// Guarded stream. All writes (including SOCKS negotiation) recheck policy.
/// There is deliberately no AsRawFd, IntoRawSocket, try_clone or TcpStream getter.
struct Transport {
    stream: TcpStream,
    route: Route,
    timeout: Duration,
    deadline: Option<Instant>,
    write_context: WriteContext,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
    in_operation: bool,
    pending: io::Cursor<Vec<u8>>,
}

impl Transport {
    fn deadline_error(&self, error: io::Error) -> io::Error {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            io::Error::other(Error::InvalidTimeout)
        } else {
            error
        }
    }

    fn timeout(&self) -> io::Result<Duration> {
        match self.deadline {
            Some(deadline) => Ok(remaining(deadline)
                .map_err(io::Error::other)?
                .min(self.timeout)),
            None => Ok(self.timeout),
        }
    }

    fn prepare_read(&mut self) -> io::Result<()> {
        if !self.in_operation {
            self.route.check().map_err(io::Error::other)?;
        } else if !self.route.consent.active.load(Ordering::SeqCst) {
            return Err(io::Error::other(Error::ConsentRevoked));
        }
        let timeout = self.timeout()?;
        if self.read_timeout != Some(timeout) {
            self.stream.set_read_timeout(Some(timeout))?;
            self.read_timeout = Some(timeout);
        }
        Ok(())
    }

    fn prepare_write(&mut self) -> io::Result<()> {
        // Every physical write checks current policy, even within a TLS read.
        // Revision-based OfflineCache makes these checks cheap without a TTL.
        self.route.check().map_err(io::Error::other)?;
        let timeout = self.timeout()?;
        if self.write_timeout != Some(timeout) {
            self.stream.set_write_timeout(Some(timeout))?;
            self.write_timeout = Some(timeout);
        }
        Ok(())
    }
}

impl Read for Transport {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.prepare_read()?;
        let count = self.pending.read(bytes)?;
        if count != 0 {
            return Ok(count);
        }
        self.stream
            .read(bytes)
            .map_err(|error| self.deadline_error(error))
    }
}

impl Write for Transport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.prepare_write()?;
        let physical = self
            .route
            .request
            .proxy
            .as_ref()
            .map_or(&self.route.request.endpoint, |p| &p.endpoint);
        self.route.gate.observe(
            &self.route.request,
            physical,
            if matches!(self.write_context, WriteContext::ProxyHandshake) {
                Operation::ProxyHandshake
            } else {
                Operation::Write
            },
        );
        let count = self
            .stream
            .write(bytes)
            .map_err(|error| self.deadline_error(error))?;
        self.route
            .gate
            .ledger
            .record_write(&self.route.request, self.write_context, count);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.route.check().map_err(io::Error::other)?;
        self.stream
            .flush()
            .map_err(|error| self.deadline_error(error))
    }
}

/// A blocking, watch-only byte stream suitable for a future WebSocket codec.
/// TLS internals, transport descriptors and retargeting are deliberately private.
pub struct Connection {
    channel: Channel,
}

// One per open connection, never collected in bulk, so the plain variant's
// size costs nothing worth an extra allocation on every connection.
#[allow(clippy::large_enum_variant)]
enum Channel {
    Plain(Transport),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, Transport>>),
}

impl Connection {
    fn transport(&mut self) -> &mut Transport {
        match &mut self.channel {
            Channel::Plain(transport) => transport,
            Channel::Tls(stream) => &mut stream.sock,
        }
    }

    /// Bound a raw protocol request, including reads of already buffered TLS
    /// plaintext. Callers must set a fresh overall deadline before each RPC.
    /// This changes no route, trust, proxy, or consent authority.
    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.transport().deadline = deadline;
    }
}

impl Read for Connection {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match &mut self.channel {
            Channel::Plain(transport) => transport.read(bytes),
            Channel::Tls(stream) => {
                stream.sock.route.check().map_err(io::Error::other)?;
                // Check the deadline even when rustls already buffered plaintext.
                stream.sock.timeout()?;
                stream.sock.in_operation = true;
                let result = stream.read(bytes);
                stream.sock.in_operation = false;
                result
            }
        }
    }
}

impl Write for Connection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &mut self.channel {
            Channel::Plain(transport) => transport.write(bytes),
            Channel::Tls(stream) => {
                stream.sock.route.check().map_err(io::Error::other)?;
                stream.sock.timeout()?;
                let count = stream.write(bytes)?;
                // Do not leave application bytes queued past a revocation check.
                stream.flush()?;
                Ok(count)
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.channel {
            Channel::Plain(transport) => transport.flush(),
            Channel::Tls(stream) => {
                stream.sock.route.check().map_err(io::Error::other)?;
                stream.sock.timeout()?;
                stream.flush()
            }
        }
    }
}

pub enum Error {
    InvalidEndpoint,
    InvalidTimeout,
    InvalidProxyCredentials,
    InvalidLocalProvider,
    HttpsProxyUnsupported,
    ProxyUnsupported,
    TrustInvalid,
    Tls,
    HttpInvalid,
    ResponseTooLarge,
    RedirectRefused,
    RedirectLimit,
    NoEgress,
    Offline,
    OfflineUnavailable,
    ConsentMissing,
    ConsentScope,
    ConsentRevoked,
    OnionProxyRequired,
    SocksProtocol,
    Io(io::Error),
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Fixed diagnostics never include raw server replies, credentials or payloads.
        if matches!(self, Self::Io(_)) {
            f.write_str("network I/O failed")
        } else {
            f.write_str(match self {
                Self::InvalidEndpoint => "InvalidEndpoint",
                Self::InvalidTimeout => "InvalidTimeout",
                Self::InvalidProxyCredentials => "InvalidProxyCredentials",
                Self::InvalidLocalProvider => "InvalidLocalProvider",
                Self::HttpsProxyUnsupported => "HTTPS proxy transport is not supported",
                Self::ProxyUnsupported => "ProxyUnsupported",
                Self::TrustInvalid => "TLS trust configuration is invalid",
                Self::Tls => "TLS certificate or handshake failed",
                Self::HttpInvalid => "Invalid HTTP message",
                Self::ResponseTooLarge => "HTTP response is too large",
                Self::RedirectRefused => "HTTP redirect refused",
                Self::RedirectLimit => "HTTP redirect limit exceeded",
                Self::NoEgress => "Outbound requests are disabled by KASSIBER_NO_EGRESS",
                Self::Offline | Self::OfflineUnavailable => {
                    "Outbound requests are disabled: Kassiber is in offline mode"
                }
                Self::ConsentMissing => "ConsentMissing",
                Self::ConsentScope => "ConsentScope",
                Self::ConsentRevoked => "ConsentRevoked",
                Self::OnionProxyRequired => ".onion backend URLs require a Tor/SOCKS proxy",
                Self::SocksProtocol => "SOCKS5 negotiation failed",
                Self::Io(_) => unreachable!(),
            })
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// Shared Python envelope fields. Service adapters own status/body mapping
    /// (AI, BTCPay, retries and updates have different contracts).
    pub fn app_error(&self) -> kassiber_envelope::AppError {
        use kassiber_envelope::AppError;
        match self {
            Self::NoEgress => AppError::new("network_egress_disabled", self.to_string()),
            Self::Offline | Self::OfflineUnavailable => {
                AppError::new("network_egress_disabled", self.to_string())
                    .with_hint("Turn off offline mode to connect.")
            }
            Self::HttpsProxyUnsupported => {
                AppError::new("network_proxy_unsupported", self.to_string())
                    .with_hint("Configure a SOCKS5 proxy, or an HTTP CONNECT proxy on a trusted route. Kassiber will not downgrade an HTTPS proxy to plaintext.")
            }
            Self::OnionProxyRequired => {
                AppError::new("network_proxy_required", self.to_string())
                    .with_hint("Configure a proxy for this endpoint; Kassiber will not connect to .onion hosts directly.")
            }
            Self::TrustInvalid => AppError::new("backend_tls_config_invalid", "Could not load the custom CA bundle")
                .with_hint("Choose a readable PEM CA bundle file and retry."),
            Self::InvalidTimeout => AppError::new("backend_timeout", "Backend request budget exhausted").retryable(true),
            Self::ConsentMissing | Self::ConsentScope | Self::ConsentRevoked => {
                AppError::new("interaction_required", "Network action requires current consent")
            }
            Self::HttpInvalid | Self::ResponseTooLarge | Self::RedirectRefused | Self::RedirectLimit => {
                AppError::new("protocol_error", self.to_string())
            }
            Self::Io(_) | Self::Tls | Self::SocksProtocol => {
                AppError::new("network_error", "network I/O failed").retryable(true)
            }
            _ => AppError::new("validation", self.to_string()),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        // Recover typed gate refusals wrapped by Read/Write/rustls.
        if let Some(inner) = error.get_ref().and_then(|e| e.downcast_ref::<Error>()) {
            return match inner {
                Self::NoEgress => Self::NoEgress,
                Self::Offline => Self::Offline,
                Self::OfflineUnavailable => Self::OfflineUnavailable,
                Self::ConsentMissing => Self::ConsentMissing,
                Self::ConsentScope => Self::ConsentScope,
                Self::ConsentRevoked => Self::ConsentRevoked,
                Self::InvalidTimeout => Self::InvalidTimeout,
                _ => Self::Io(error),
            };
        }
        if error.kind() == io::ErrorKind::InvalidData {
            Self::Tls
        } else {
            Self::Io(error)
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod adapter_tests;
