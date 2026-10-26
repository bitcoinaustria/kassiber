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
//! TLS and HTTP adapters are deferred: TLS routes fail before DNS, and this
//! crate never follows redirects. Future adapters must stay inside this crate,
//! consume Route, disable ambient proxies and redirects, enforce the selected
//! trust store, and retain the pre-I/O policy checks. No raw socket is exposed.

mod ledger;
mod socks;

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
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proxy")
            .field("endpoint", &self.endpoint)
            .field("authenticated", &self.credentials.is_some())
            .finish()
    }
}

impl Proxy {
    /// SOCKS5 always sends a domain target to the proxy, including `socks5` spelling.
    /// HTTP CONNECT is deferred with HTTP integration; HTTPS proxies are refused.
    /// Supply decoded credentials separately so they never enter diagnostic URLs.
    pub fn new(
        scheme: &str,
        endpoint: Endpoint,
        credentials: Option<(String, String)>,
    ) -> Result<Self, Error> {
        match scheme {
            "socks5" | "socks5h" => {}
            "https" => return Err(Error::HttpsProxyUnsupported),
            _ => return Err(Error::ProxyUnsupported),
        }
        if let Some((user, password)) = &credentials {
            if user.is_empty() || password.is_empty() || user.len() > 255 || password.len() > 255 {
                return Err(Error::InvalidProxyCredentials);
            }
        }
        if endpoint.host.ends_with(".onion") {
            return Err(Error::ProxyUnsupported); // never resolve a proxy's onion locally
        }
        Ok(Self {
            endpoint,
            credentials,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsTrust {
    Plaintext,
    SystemRoots,
    OperatorRoots,
    BackendCertificate,
    ExplicitInsecure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    subsystem: Subsystem,
    endpoint: Endpoint,
    proxy: Option<Proxy>,
    tls: TlsTrust,
    on_device_ai: bool,
}

impl Request {
    pub fn new(subsystem: Subsystem, endpoint: Endpoint, tls: TlsTrust) -> Self {
        Self {
            subsystem,
            endpoint,
            tls,
            proxy: None,
            on_device_ai: false,
        }
    }

    pub fn with_proxy(mut self, proxy: Proxy) -> Self {
        self.proxy = Some(proxy);
        self
    }

    /// Only providers explicitly marked local qualify, never a remote gateway
    /// merely hosted on loopback. Validation also requires loopback and no proxy.
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
    /// blocks loopback. Product NO_EGRESS is always destination-blind.
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
        }
    }

    pub fn ledger(&self) -> Ledger {
        self.ledger.clone()
    }

    /// Observer attempts are independent of the product ledger. Policy refusals
    /// do not count as network attempts; tests assert both buffers independently.
    pub fn attempts(&self) -> Snapshot {
        self.attempts.snapshot(0, 2_000)
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
        if no_egress() {
            return Err(Error::NoEgress);
        }
        let on_device = self.request.on_device_ai
            && self.request.subsystem == Subsystem::Ai
            && self.request.endpoint.loopback()
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
        if self.request.endpoint.host.ends_with(".onion") && self.request.proxy.is_none() {
            return Err(Error::OnionProxyRequired);
        }
        Ok(())
    }

    /// The only connection constructor. TLS requests refuse before DNS until
    /// a compliant TLS adapter exists; plaintext is never substituted for TLS.
    pub fn connect(self, timeout: Duration) -> Result<Connection, Error> {
        self.check()?;
        if self.request.tls != TlsTrust::Plaintext {
            return Err(Error::TlsUnsupported);
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
                    let mut connection = Connection {
                        stream,
                        route: self,
                    };
                    connection
                        .stream
                        .set_read_timeout(Some(remaining(deadline)?))?;
                    connection
                        .stream
                        .set_write_timeout(Some(remaining(deadline)?))?;
                    if connection.route.request.proxy.is_some() {
                        socks::handshake(&mut connection, deadline)?;
                    }
                    connection.stream.set_read_timeout(Some(timeout))?;
                    connection.stream.set_write_timeout(Some(timeout))?;
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
pub struct Connection {
    stream: TcpStream,
    route: Route,
}

impl Read for Connection {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.read(bytes)
    }
}

impl Write for Connection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.route.check().map_err(io::Error::other)?;
        let physical = self
            .route
            .request
            .proxy
            .as_ref()
            .map_or(&self.route.request.endpoint, |p| &p.endpoint);
        self.route
            .gate
            .observe(&self.route.request, physical, Operation::Write);
        let count = self.stream.write(bytes)?;
        self.route.gate.ledger.record(
            &self.route.request,
            &self.route.request.endpoint,
            Operation::Write,
            count,
        );
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.route.check().map_err(io::Error::other)?;
        self.stream.flush()
    }
}

#[derive(Debug)]
pub enum Error {
    InvalidEndpoint,
    InvalidTimeout,
    InvalidProxyCredentials,
    InvalidLocalProvider,
    HttpsProxyUnsupported,
    ProxyUnsupported,
    TlsUnsupported,
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

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Fixed diagnostics never include raw server replies, credentials or payloads.
        if matches!(self, Self::Io(_)) {
            f.write_str("network I/O failed")
        } else {
            write!(f, "{self:?}")
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[cfg(test)]
mod tests;
