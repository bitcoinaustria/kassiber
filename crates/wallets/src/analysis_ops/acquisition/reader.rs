//! Bounded, consented reads from exactly the reviewed backend. Every request
//! passes the explicit-action grant through `kassiber_net`; no retries, no
//! redirects and no ambient proxies. Errors never carry server prose.
use super::super::routing::{backend_value, py_text};
use super::super::*;
use super::legs::*;
use super::{MAX_RESPONSE_BYTES as MAX_RESPONSE, MAX_TOTAL_BYTES as MAX_TOTAL};
use kassiber_net::{Connection, Consent, Gate, HttpOptions, HttpRequest, Request};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

fn fail(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}

pub(super) struct Budget {
    pub maximum: i64,
    pub count: i64,
    pub bytes_read: i64,
    pub deadline: Instant,
}

impl Budget {
    pub fn new(maximum: i64) -> Self {
        Self {
            maximum,
            count: 0,
            bytes_read: 0,
            deadline: Instant::now() + Duration::from_secs(45),
        }
    }

    /// Covers HTTP requests and the Electrum handshake before any connect.
    pub fn request(&mut self, runtime: &dyn Runtime) -> Result<Duration> {
        if runtime.no_egress() {
            return Err(fail(
                "network_egress_disabled",
                "Outbound chain acquisition is disabled by KASSIBER_NO_EGRESS",
            ));
        }
        if runtime.offline() {
            return Err(fail(
                "network_egress_disabled",
                "Outbound chain acquisition is disabled: Kassiber is in offline mode",
            )
            .with_hint("Turn off offline mode to connect."));
        }
        let now = Instant::now();
        if self.count >= self.maximum || self.bytes_read >= MAX_TOTAL || now >= self.deadline {
            return Err(fail("acquisition_budget", "Acquisition budget reached"));
        }
        self.count += 1;
        let left = self.deadline.saturating_duration_since(now);
        Ok(left.clamp(Duration::from_millis(50), Duration::from_secs(8)))
    }

    pub fn received(&mut self, size: usize) -> Result<()> {
        self.bytes_read += size as i64;
        if self.bytes_read > MAX_TOTAL {
            return Err(fail(
                "acquisition_budget",
                "Acquisition total response byte budget reached",
            ));
        }
        Ok(())
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

/// A recurring grant's durable lifetime quota around each counted request.
pub(super) trait Quota {
    /// Refund the unused part of the previous successful reservation.
    fn settle(&mut self) -> Result<()>;
    /// Revalidate the leased grant and reserve one response before transport.
    fn reserve(&mut self) -> Result<()>;
    fn received(&mut self, size: usize) -> Result<()>;
}

pub(super) struct Reader<'a> {
    pub backend: Value,
    pub kind: String,
    pub budget: Budget,
    pub quota: Option<Box<dyn Quota + 'a>>,
    request: Request,
    gate: Gate,
    consent: Consent,
    runtime: &'a dyn Runtime,
    electrum: Option<Electrum>,
}

struct Electrum {
    connection: Connection,
    buffer: Vec<u8>,
    request_id: u64,
}

/// Python's `URLError` text for a connection that never produced a response.
fn transport_error(error: kassiber_net::Error) -> AppError {
    use kassiber_net::Error as E;
    match error {
        E::ResponseTooLarge => fail(
            "invalid_observation",
            "Acquisition response exceeds the byte limit",
        ),
        E::RedirectRefused | E::RedirectLimit => fail(
            "history_unavailable",
            "Backend could not supply the requested observation",
        ),
        E::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => AppError::new(
            "internal_error",
            "<urlopen error [Errno 111] Connection refused>",
        ),
        E::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            AppError::new("internal_error", "timed out")
        }
        other => other.app_error(),
    }
}

/// Standard padded base64 for the Basic credential header.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                TABLE[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

fn dumps(value: &Value) -> Result<String> {
    canonical(value)
}

fn loads(text: &str) -> Result<Value> {
    kassiber_pytext::loads(text).map_err(|_| {
        fail(
            "invalid_observation",
            "Acquisition returned malformed response data",
        )
    })
}

impl<'a> Reader<'a> {
    /// The route and grant are minted only after the reviewed plan matched.
    pub fn new(backend: Value, runtime: &'a dyn Runtime) -> Result<Self> {
        Self::build(backend, runtime, Consent::explicit_trigger)
    }

    /// An authorized recurring source: the enabled grant, not a click.
    pub fn scoped(backend: Value, runtime: &'a dyn Runtime) -> Result<Self> {
        Self::build(backend, runtime, Consent::narrowly_scoped)
    }

    fn build(
        backend: Value,
        runtime: &'a dyn Runtime,
        grant: fn(&Request) -> Consent,
    ) -> Result<Self> {
        let kind = py_text(field(&backend, "kind"));
        let mut config = backend.clone();
        set(&mut config, "batch_size", Value::Null);
        let route = kassiber_chain::Backend::new(&to_serde(&config), kind == "electrum")?;
        let request = route.request().clone();
        let consent = grant(&request);
        Ok(Self {
            backend,
            kind,
            budget: Budget::new(0),
            quota: None,
            request,
            gate: runtime.gate(),
            consent,
            runtime,
            electrum: None,
        })
    }

    pub fn runtime(&self) -> &'a dyn Runtime {
        self.runtime
    }

    pub fn settle(&mut self) -> Result<()> {
        match self.quota.as_mut() {
            Some(quota) => quota.settle(),
            None => Ok(()),
        }
    }

    pub fn revoke(&self) {
        self.consent.revoke();
    }

    fn route(&self) -> Result<kassiber_net::Route> {
        self.gate
            .authorize(self.request.clone(), Some(self.consent.clone()))
            .map_err(|error| error.app_error())
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = format!("chain-analysis-{method}");
        let payload = dict([
            ("jsonrpc", s("1.0")),
            ("id", s(id.as_str())),
            ("method", s(method)),
            (
                "params",
                if truthy(&params) {
                    params
                } else {
                    Value::List(vec![])
                },
            ),
        ]);
        let url = py_text(field(&self.backend, "url"));
        let headers = vec![("Authorization".to_owned(), self.core_auth()?)];
        let response = self.http(&url, headers, false, Some(&payload))?;
        if !matches!(response, Value::Dict(_)) || field(&response, "id") != &s(id.as_str()) {
            return Err(fail(
                "invalid_observation",
                "Invalid Bitcoin Core response identity",
            ));
        }
        if truthy(field(&response, "error")) {
            return Err(fail(
                "history_unavailable",
                "Bitcoin Core could not supply the requested observation",
            ));
        }
        Ok(field(&response, "result").clone())
    }

    /// `bitcoinrpc_auth_headers`, evaluated before the request is counted.
    fn core_auth(&self) -> Result<String> {
        let token = if let Some(path) = backend_value(&self.backend, &["cookiefile", "cookie_file"])
        {
            let expanded = match (path.strip_prefix('~'), std::env::var("HOME")) {
                (Some(rest), Ok(home)) if rest.is_empty() || rest.starts_with('/') => {
                    format!("{home}{rest}")
                }
                _ => path.clone(),
            };
            let text = std::fs::read_to_string(&expanded).map_err(|error| {
                AppError::new(
                    "internal_error",
                    match error.raw_os_error() {
                        Some(2) => format!(
                            "[Errno 2] No such file or directory: {}",
                            kassiber_pytext::repr_str(&expanded)
                        ),
                        _ => "Cannot read the Bitcoin Core cookie file".into(),
                    },
                )
            })?;
            kassiber_pytext::strip(&text).to_owned()
        } else {
            let username = backend_value(&self.backend, &["username", "rpcuser", "rpc_user"]);
            let password =
                backend_value(&self.backend, &["password", "rpcpassword", "rpc_password"]);
            match (username, password) {
                (Some(username), Some(password)) => format!("{username}:{password}"),
                _ => {
                    return Err(AppError::new(
                        "app_error",
                        format!(
                            "Bitcoin Core backend '{}' requires USERNAME/PASSWORD or COOKIEFILE configuration",
                            py_text(field(&self.backend, "name"))
                        ),
                    ))
                }
            }
        };
        Ok(format!("Basic {}", base64(token.as_bytes())))
    }

    fn esplora_auth(&self) -> Vec<(String, String)> {
        if let Some(header) = backend_value(&self.backend, &["auth_header"]) {
            return vec![("Authorization".into(), header)];
        }
        if let Some(token) = backend_value(&self.backend, &["token"]) {
            return vec![("Authorization".into(), format!("Bearer {token}"))];
        }
        vec![]
    }

    pub fn get(&mut self, path: &str, text: bool) -> Result<Value> {
        let base = py_text(field(&self.backend, "url"));
        let url = format!("{}{path}", base.trim_end_matches('/'));
        let headers = self.esplora_auth();
        self.http(&url, headers, text, None)
    }

    fn http(
        &mut self,
        url: &str,
        headers: Vec<(String, String)>,
        text: bool,
        payload: Option<&Value>,
    ) -> Result<Value> {
        self.settle()?;
        let timeout = self.budget.request(self.runtime)?;
        if let Some(quota) = self.quota.as_mut() {
            quota.reserve()?;
        }
        let mut request = HttpRequest::new(if payload.is_some() { "POST" } else { "GET" }, url)
            .map_err(|e| e.app_error())?;
        for (name, value) in [
            (
                "Accept",
                if text {
                    "text/plain"
                } else {
                    "application/json"
                },
            ),
            ("Content-Type", "application/json"),
            ("User-Agent", "Kassiber-chain-analysis"),
        ] {
            request = request.header(name, value).map_err(|e| e.app_error())?;
        }
        for (name, value) in &headers {
            request = request.header(name, value).map_err(|e| e.app_error())?;
        }
        if let Some(payload) = payload {
            request = request.body(dumps(payload)?.into_bytes());
        }
        let total_left = (MAX_TOTAL - self.budget.bytes_read).max(0) as usize;
        let cap = (MAX_RESPONSE as usize + 1).min(total_left + 1);
        let response = request.send(
            self.route()?,
            HttpOptions {
                timeout,
                deadline: Some(self.budget.deadline),
                max_response_bytes: cap,
                ..Default::default()
            },
        );
        let response = match response {
            Ok(response) => response,
            Err(kassiber_net::Error::ResponseTooLarge) => {
                self.budget.received(cap)?;
                return Err(transport_error(kassiber_net::Error::ResponseTooLarge));
            }
            Err(_) if self.budget.expired() => {
                return Err(fail(
                    "acquisition_budget",
                    "Acquisition response exceeded its scheduling deadline",
                ))
            }
            Err(error) => return Err(transport_error(error)),
        };
        if let Some(quota) = self.quota.as_mut() {
            quota.received(response.body.len())?;
        }
        self.budget.received(response.body.len())?;
        if response.body.len() > MAX_RESPONSE as usize {
            return Err(transport_error(kassiber_net::Error::ResponseTooLarge));
        }
        // Core's typed RPC envelope may arrive with HTTP 400/404/500.
        if response.status >= 300
            && !(payload.is_some() && [400, 404, 500].contains(&response.status))
        {
            return Err(fail(
                "history_unavailable",
                "Backend could not supply the requested observation",
            ));
        }
        let body = String::from_utf8(response.body).map_err(|_| {
            fail(
                "invalid_observation",
                "Acquisition returned malformed response data",
            )
        })?;
        if text {
            Ok(s(body.as_str()))
        } else {
            loads(&body)
        }
    }

    /// Opens the reviewed Electrum route and negotiates its version.
    pub fn connect_electrum(&mut self) -> Result<()> {
        let timeout = match backend_value(&self.backend, &["timeout"]) {
            Some(text) => kassiber_pytext::parse_int(&text)
                .ok()
                .and_then(|n| u64::try_from(n).ok())
                .unwrap_or(8)
                .min(8),
            None => 8,
        };
        let mut connection = self
            .route()?
            .connect(Duration::from_secs(timeout.max(1)))
            .map_err(transport_error)?;
        connection.set_deadline(Some(self.budget.deadline));
        self.electrum = Some(Electrum {
            connection,
            buffer: Vec::new(),
            request_id: 0,
        });
        self.electrum_call("server.version", Value::List(vec![s("Kassiber"), s("1.6")]))?;
        Ok(())
    }

    pub fn electrum(&mut self, method: &str, params: Value) -> Result<Value> {
        self.budget.request(self.runtime)?;
        self.electrum_call(method, params)
    }

    fn electrum_call(&mut self, method: &str, params: Value) -> Result<Value> {
        let Some(client) = self.electrum.as_mut() else {
            return Err(fail(
                "history_unavailable",
                "Electrum connection is unavailable",
            ));
        };
        if Instant::now() >= self.budget.deadline {
            return Err(fail(
                "acquisition_budget",
                "Electrum request exceeded its deadline",
            ));
        }
        client.request_id += 1;
        let id = client.request_id;
        let payload = dict([
            ("jsonrpc", s("2.0")),
            ("id", Value::Int(id.into())),
            ("method", s(method)),
            (
                "params",
                if truthy(&params) {
                    params
                } else {
                    Value::List(vec![])
                },
            ),
        ]);
        let mut bytes = dumps(&payload)?.into_bytes();
        bytes.push(b'\n');
        client
            .connection
            .write_all(&bytes)
            .map_err(|e| transport_error(e.into()))?;
        for _ in 0..128 {
            while !client.buffer.contains(&b'\n') {
                if Instant::now() >= self.budget.deadline {
                    return Err(fail(
                        "acquisition_budget",
                        "Electrum response exceeded its deadline",
                    ));
                }
                let want = 65536.min(MAX_RESPONSE as usize + 1 - client.buffer.len());
                let mut chunk = vec![0; want];
                let count = match client.connection.read(&mut chunk) {
                    Ok(count) => count,
                    Err(_) if Instant::now() >= self.budget.deadline => {
                        return Err(fail(
                            "acquisition_budget",
                            "Electrum response exceeded its deadline",
                        ))
                    }
                    Err(error) => return Err(transport_error(error.into())),
                };
                if count == 0 {
                    return Err(fail(
                        "history_unavailable",
                        "Electrum connection closed before its response",
                    ));
                }
                self.budget.received(count)?;
                client.buffer.extend_from_slice(&chunk[..count]);
                if client.buffer.len() > MAX_RESPONSE as usize {
                    return Err(fail(
                        "invalid_observation",
                        "Electrum response exceeds the byte limit",
                    ));
                }
            }
            let end = client.buffer.iter().position(|b| *b == b'\n').unwrap();
            let line: Vec<u8> = client.buffer.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line[..end]);
            let message = kassiber_pytext::loads(&text)
                .map_err(|error| AppError::new("internal_error", error.to_string()))?;
            if !matches!(message, Value::Dict(_)) {
                return Err(fail("invalid_observation", "Malformed Electrum response"));
            }
            if !py_eq(field(&message, "id"), &Value::Int(id.into())) {
                continue;
            }
            if truthy(field(&message, "error")) {
                return Err(fail(
                    "history_unavailable",
                    "Electrum could not supply this observation",
                ));
            }
            return Ok(field(&message, "result").clone());
        }
        Err(fail(
            "acquisition_budget",
            "Electrum response notification limit reached",
        ))
    }

    pub fn genesis(&mut self) -> Result<Value> {
        if self.kind == "bitcoinrpc" {
            return self.rpc("getblockhash", Value::List(vec![int(0)]));
        }
        if self.electrum.is_some() {
            let features = self.electrum("server.features", Value::Null)?;
            return Ok(match features {
                Value::Dict(_) => field(&features, "genesis_hash").clone(),
                _ => Value::Null,
            });
        }
        let text = self.get("/block-height/0", true)?;
        Ok(s(kassiber_pytext::strip(&py_text(&text))))
    }
}
