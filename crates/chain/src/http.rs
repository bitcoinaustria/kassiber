use super::*;
use kassiber_net::{Gate, HttpOptions, HttpPool, HttpRequest, HttpResponse, HttpUrl};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Esplora/mempool (Bitcoin or Liquid) and shared Core HTTP transport.
/// The caller owns action/book binding; a saved backend never mints Consent.
pub(crate) enum StatusPolicy {
    Backend,
    Core,
    Return,
    /// An exchange read: as `Backend`, but these statuses are answered
    /// rather than raised. The request's credentials are remembered for
    /// error display, a successful body that echoes one is noted (never
    /// rewritten), and error bodies are scrubbed after their JSON escapes
    /// are decoded. docs/reference/credential-echo.md.
    Exchange(&'static [u16]),
}

pub struct HttpClient {
    pub(crate) backend: Backend,
    pub(crate) gate: Gate,
    pub(crate) version: String,
    pub(crate) withheld: Vec<String>,
    pub(crate) met: Mutex<Met>,
    pub(crate) pool: Arc<HttpPool>,
    auth_header: Option<String>,
    limiter: Option<Arc<Limiter>>,
}
/// What this client's exchange reads met.
#[derive(Default)]
pub(crate) struct Met {
    /// Every credential the requests carried, at any length (URL and proxy
    /// userinfo, credential headers, computed signatures).
    seen: Vec<String>,
    /// A successful body carried one of at least
    /// [`kassiber_credentials::MIN_EXACT_CREDENTIAL_CHARS`] characters.
    echoed: bool,
}
impl HttpClient {
    /// `request_with_retry(url, opener, source_label=source, withheld=...)`
    /// around one signed `json.loads(body.decode("utf-8"))` read: the host
    /// limiter, bounded 429/503 retries and the exact-route gate. `build` must
    /// rebuild the same signed request on a retry.
    pub fn authenticated_json(
        &self,
        url: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        source: &str,
        build: impl Fn() -> Result<HttpRequest>,
    ) -> Result<kassiber_pyjson::Value> {
        let response = self
            .retry(
                url,
                consent,
                deadline,
                StatusPolicy::Exchange(&[]),
                build,
                RESPONSE_LIMIT,
            )
            .map_err(|mut error| {
                if error.code == "rate_limited" {
                    error.message = error.message.replacen("Backend", source, 1);
                }
                error
            })?;
        let text = std::str::from_utf8(&response.body)
            .map_err(|e| AppError::new("internal_error", utf8_error(&response.body, &e)))?;
        kassiber_pytext::loads(text).map_err(|e| AppError::new("internal_error", e.to_string()))
    }
    /// [`Self::authenticated_json`]'s retries, gate and error bodies, but the
    /// response itself: a 2xx status or one of `accepted` is the caller's to
    /// read, every other status fails with the withheld body prefix.
    pub fn authenticated_response(
        &self,
        url: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        source: &str,
        accepted: &'static [u16],
        build: impl Fn() -> Result<HttpRequest>,
    ) -> Result<HttpResponse> {
        self.retry(
            url,
            consent,
            deadline,
            StatusPolicy::Exchange(accepted),
            build,
            RESPONSE_LIMIT,
        )
        .map_err(|mut error| {
            if error.code == "rate_limited" {
                error.message = error.message.replacen("Backend", source, 1);
            }
            error
        })
    }
    pub fn new(backend: Backend, gate: Gate, version: &str) -> Self {
        let auth_header = config::value(&backend.config, &["auth_header"])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                config::value(&backend.config, &["token"])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("Bearer {s}"))
            });
        let limiter = host_limiter(&backend.url).ok();
        Self {
            backend,
            gate,
            version: version.into(),
            withheld: vec![],
            met: Mutex::default(),
            pool: Arc::new(HttpPool::new()),
            auth_header,
            limiter,
        }
    }
    /// Reuse a caller-scoped keep-alive pool across phases of the same wallet
    /// fetch action. Never share across wallets, books, or consents.
    pub fn with_pool(mut self, pool: Arc<HttpPool>) -> Self {
        self.pool = pool;
        self
    }
    /// The literal credentials this client's requests carry. An error body
    /// that echoes one shows `<redacted>`, before it is truncated.
    pub fn withholding(mut self, credentials: &[&str]) -> Self {
        self.withheld = credentials.iter().map(|c| (*c).to_owned()).collect();
        self
    }
    pub fn backend(&self) -> &Backend {
        &self.backend
    }
    /// Every credential an error from this client's exchange reads must
    /// withhold, at any length: its own and every one its requests carried.
    pub fn display_credentials(&self) -> Vec<String> {
        let met = self.met.lock().unwrap_or_else(|e| e.into_inner());
        let mut values: Vec<String> = self.withheld.iter().chain(&met.seen).cloned().collect();
        values.retain(|value| !value.is_empty());
        values.sort();
        values.dedup();
        values
    }
    /// Whether a successful exchange body echoed a request credential.
    pub fn echoed(&self) -> bool {
        self.met.lock().unwrap_or_else(|e| e.into_inner()).echoed
    }
    /// The credentials one request carries, with the client's own.
    fn credentials(&self, url: &str, headers: &[(String, String)]) -> Vec<String> {
        let proxy = self.backend.proxy_url();
        let mut found =
            kassiber_credentials::request_credentials(Some(url), headers, proxy.as_deref());
        found.extend(self.withheld.iter().cloned());
        found
    }
    /// Optional cache anchor, read only under the caller's exact sync grant.
    /// A missing or malformed tip disables reuse; it never means an empty chain.
    pub fn esplora_tip_hash(&self, consent: Option<&Consent>, deadline: Instant) -> Option<String> {
        self.esplora_tip_hash_checked(consent, deadline, &|| Ok(()))
    }
    pub fn esplora_tip_hash_checked(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
        check: &dyn Fn() -> Result<()>,
    ) -> Option<String> {
        let text = self
            .get_text_checked("blocks/tip/hash", consent, deadline, check)
            .ok()?;
        let text = kassiber_pytext::strip(&text).to_ascii_lowercase();
        (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())).then_some(text)
    }
    pub fn get_json(
        &self,
        path: &str,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<Value> {
        let response = self.get(path, "application/json", consent, deadline)?;
        decode_json(&response.body)
    }
    pub fn get_json_checked(
        &self,
        path: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Value> {
        let response = self.get_checked(path, "application/json", consent, deadline, check)?;
        decode_json(&response.body)
    }
    /// `http_post_json(append_url_path(url, path), payload)`'s request: one
    /// JSON body POSTed under the host limiter, bounded 429/503 retries and
    /// the exact-route gate. The body comes back undecoded; the caller
    /// decodes it as Python's `json.loads(body.decode("utf-8"))` would.
    pub fn post_json(
        &self,
        path: &str,
        body: &[u8],
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<Vec<u8>> {
        let url = config::append_path(&self.backend.url, path)?;
        let response = self.retry(
            &url,
            consent,
            deadline,
            StatusPolicy::Backend,
            || {
                Ok(HttpRequest::new("POST", &url)
                    .map_err(|e| e.app_error())?
                    .header("Accept", "application/json")
                    .map_err(|e| e.app_error())?
                    .header("Content-Type", "application/json")
                    .map_err(|e| e.app_error())?
                    .header("User-Agent", &format!("kassiber/{}", self.version))
                    .map_err(|e| e.app_error())?
                    .body(body.to_vec()))
            },
            RESPONSE_LIMIT,
        )?;
        Ok(response.body)
    }
    pub fn get_text(
        &self,
        path: &str,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<String> {
        self.get_text_checked(path, consent, deadline, &|| Ok(()))
    }
    pub fn get_text_checked(
        &self,
        path: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<String> {
        let response = self.get_checked(path, "text/plain", consent, deadline, check)?;
        String::from_utf8(response.body).map_err(|_| app("Backend returned invalid UTF-8"))
    }
    fn get(
        &self,
        path: &str,
        accept: &str,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<HttpResponse> {
        self.get_checked(path, accept, consent, deadline, &|| Ok(()))
    }
    fn get_checked(
        &self,
        path: &str,
        accept: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<HttpResponse> {
        let url = config::append_path(&self.backend.url, path)?;
        self.retry_checked(
            &url,
            consent,
            deadline,
            StatusPolicy::Backend,
            || {
                let mut request = HttpRequest::new("GET", &url)
                    .map_err(|e| e.app_error())?
                    .header("Accept", accept)
                    .map_err(|e| e.app_error())?
                    .header("User-Agent", &format!("kassiber/{}", self.version))
                    .map_err(|e| e.app_error())?;
                if let Some(auth) = &self.auth_header {
                    request = request
                        .header("Authorization", auth)
                        .map_err(|e| e.app_error())?;
                }
                Ok(request)
            },
            RESPONSE_LIMIT,
            check,
        )
    }
    /// One explicit HTTP probe: no retry, no follow-up, no redirect authority.
    pub fn probe(&self, consent: Option<&Consent>, deadline: Instant) -> Result<HttpResponse> {
        let request = HttpRequest::new("GET", &self.backend.url)
            .map_err(|e| e.app_error())?
            .header("Accept", "application/json")
            .map_err(|e| e.app_error())?
            .header("User-Agent", &format!("Kassiber/{}", self.version))
            .map_err(|e| e.app_error())?;
        self.send(request, consent, deadline, RESPONSE_LIMIT)
    }
    pub(crate) fn send(
        &self,
        request: HttpRequest,
        consent: Option<&Consent>,
        deadline: Instant,
        cap: usize,
    ) -> Result<HttpResponse> {
        self.send_checked(request, consent, deadline, cap, &|| Ok(()))
    }
    fn send_checked(
        &self,
        request: HttpRequest,
        consent: Option<&Consent>,
        deadline: Instant,
        cap: usize,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<HttpResponse> {
        let route = self
            .gate
            .authorize(self.backend.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        remaining(deadline)?;
        let refusal = std::cell::Cell::new(None);
        request
            .send_pooled_checked(
                route,
                HttpOptions {
                    timeout: self.backend.timeout,
                    deadline: Some(deadline),
                    max_response_bytes: cap,
                    ..Default::default()
                },
                &self.pool,
                &|| {
                    check().map_err(|error| {
                        refusal.set(Some(error));
                        kassiber_net::Error::ConsentRevoked
                    })
                },
            )
            .map_err(|e| refusal.take().unwrap_or_else(|| e.app_error()))
    }
    pub(crate) fn retry(
        &self,
        url: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        policy: StatusPolicy,
        build: impl Fn() -> Result<HttpRequest>,
        cap: usize,
    ) -> Result<HttpResponse> {
        self.retry_checked(url, consent, deadline, policy, build, cap, &|| Ok(()))
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn retry_checked(
        &self,
        url: &str,
        consent: Option<&Consent>,
        deadline: Instant,
        policy: StatusPolicy,
        build: impl Fn() -> Result<HttpRequest>,
        cap: usize,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<HttpResponse> {
        check()?;
        // Check before waiting for a host slot, building auth, or sleeping.
        self.gate
            .authorize(self.backend.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        let limiter = match &self.limiter {
            Some(cached) if url.starts_with(&self.backend.url) => cached.clone(),
            _ => host_limiter(url)?,
        };
        let mut cumulative = Duration::ZERO;
        for attempt in 0..3 {
            check()?;
            let slot = limiter.acquire(deadline, check)?;
            check()?;
            let request = build()?;
            let exchange = match policy {
                StatusPolicy::Exchange(accepted) => Some(accepted),
                _ => None,
            };
            let mut cached_credentials = None;
            if exchange.is_some() {
                let credentials = self.credentials(url, request.headers());
                let mut met = self.met.lock().unwrap_or_else(|e| e.into_inner());
                for value in &credentials {
                    if !value.is_empty() && !met.seen.contains(value) {
                        met.seen.push(value.clone());
                    }
                }
                cached_credentials = Some(credentials);
            }
            let headers_for_error = if cached_credentials.is_none() {
                Some(request.headers().to_vec())
            } else {
                None
            };
            let mut response = self.send_checked(request, consent, deadline, cap, check)?;
            drop(slot);
            if !matches!(response.status, 429 | 503) {
                if let Some(accepted) = exchange {
                    let credentials = cached_credentials.as_deref().unwrap_or(&[]);
                    if (200..300).contains(&response.status) {
                        if echoes(&response.body, credentials) {
                            self.met.lock().unwrap_or_else(|e| e.into_inner()).echoed = true;
                        }
                        return Ok(response);
                    }
                    let withheld = kassiber_credentials::credential_forms(credentials);
                    response.body = scrub_error_body(&response.body, &withheld);
                    if accepted.contains(&response.status) {
                        return Ok(response);
                    }
                } else if (200..300).contains(&response.status)
                    || (matches!(policy, StatusPolicy::Core)
                        && matches!(response.status, 400 | 404 | 500))
                    || matches!(policy, StatusPolicy::Return)
                {
                    return Ok(response);
                }
                let withheld = if let Some(credentials) = cached_credentials {
                    kassiber_credentials::credential_forms(&credentials)
                } else {
                    let headers = headers_for_error.unwrap_or_default();
                    let credentials = self.credentials(url, &headers);
                    kassiber_credentials::credential_forms(&credentials)
                };
                // The body is the server's text: withhold every credential
                // the request carried, and the client's own (an exchange key
                // and secret), before the cut, as Python does.
                let detail = kassiber_credentials::withhold(
                    &String::from_utf8_lossy(&response.body),
                    &withheld,
                );
                return Err(app(format!(
                    "HTTP {} from backend for {}: {}",
                    response.status,
                    label(url),
                    redacted(&detail.chars().take(200).collect::<String>())
                )));
            }
            let retry_after = response.header("Retry-After").and_then(retry_seconds);
            let delay = retry_after.map(Duration::from_secs).unwrap_or_else(|| {
                use std::hash::{BuildHasher, Hasher};
                let jitter = std::collections::hash_map::RandomState::new()
                    .build_hasher()
                    .finish()
                    % 500_001;
                Duration::from_micros(500_000 * (1 << attempt) + jitter)
            });
            if response.status == 429 {
                limiter.note_cooldown(delay.min(Duration::from_secs(5)));
            }
            if attempt == 2 || cumulative.saturating_add(delay) > Duration::from_secs(20) {
                let message = if response.status == 429 {
                    format!(
                        "Backend rate limited the request for {} (HTTP 429)",
                        label(url)
                    )
                } else {
                    format!(
                        "Backend is temporarily unavailable for {} (HTTP {})",
                        label(url),
                        response.status
                    )
                };
                return Err(AppError::new("rate_limited", message)
                    .retryable(true)
                    .with_details(details(json!({"retry_after_seconds":retry_after}))));
            }
            if delay >= remaining(deadline)? {
                return Err(kassiber_net::Error::InvalidTimeout.app_error());
            }
            check()?;
            self.gate
                .authorize(self.backend.request.clone(), consent.cloned())
                .map_err(|e| e.app_error())?;
            let wake = Instant::now() + delay;
            while Instant::now() < wake {
                check()?;
                self.gate
                    .authorize(self.backend.request.clone(), consent.cloned())
                    .map_err(|e| e.app_error())?;
                remaining(deadline)?;
                std::thread::sleep(
                    wake.saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(100)),
                );
            }
            check()?;
            cumulative += delay;
        }
        unreachable!()
    }
}
/// Authentication codecs use the transport's existing reviewed dependency.
pub fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
/// `redaction.withhold_credentials`'s spellings of each credential: literal,
/// percent-encoded (`quote` and `quote_plus` with no safe characters) and
/// JSON-escaped (with and without escaped slashes), longest first. Python
/// orders equal lengths by set iteration; these tie-break by text.
pub fn credential_forms(credentials: &[&str]) -> Vec<String> {
    let quote = |text: &str, plus: bool| {
        let mut out = String::new();
        for b in text.bytes() {
            if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) {
                out.push(char::from(b));
            } else if plus && b == b' ' {
                out.push('+');
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    };
    let mut forms = std::collections::BTreeSet::new();
    for credential in credentials.iter().filter(|c| !c.is_empty()) {
        let json = kassiber_pyjson::dumps(
            &kassiber_pyjson::Value::Str((*credential).into()),
            &Default::default(),
        )
        .expect("string JSON");
        let escaped = json[1..json.len() - 1].to_owned();
        forms.extend([
            (*credential).to_owned(),
            quote(credential, false),
            quote(credential, true),
            escaped.replace('/', "\\/"),
            escaped,
        ]);
    }
    let mut forms: Vec<String> = forms.into_iter().collect();
    forms.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then(a.cmp(b)));
    forms
}
/// `redaction.withhold_credentials(text, credentials)`.
pub fn withhold_credentials(text: &str, credentials: &[&str]) -> String {
    let mut text = text.to_owned();
    for form in credential_forms(credentials) {
        text = text.replace(&form, "<redacted>");
    }
    text
}
/// Whether a successful exchange body carries a request credential of at
/// least the exact-match length, as written or in its decoded JSON strings
/// and keys. Shorter values are ordinary data and never match.
fn echoes(body: &[u8], credentials: &[String]) -> bool {
    let forms: Vec<String> = credentials
        .iter()
        .flat_map(|c| kassiber_credentials::exact_forms(c))
        .collect();
    let found = |text: &str| forms.iter().any(|form| text.contains(form.as_str()));
    fn any(value: &kassiber_pyjson::Value, found: &dyn Fn(&str) -> bool) -> bool {
        use kassiber_pyjson::{Key, Value as Py};
        match value {
            Py::Str(text) => found(&lossy(text)),
            Py::List(items) => items.iter().any(|item| any(item, found)),
            Py::Dict(items) => items.iter().any(|(key, item)| {
                matches!(key, Key::Str(name) if found(&lossy(name))) || any(item, found)
            }),
            _ => false,
        }
    }
    let text = String::from_utf8_lossy(body);
    found(&text)
        || (text.contains('\\')
            && kassiber_pytext::loads(&text).is_ok_and(|parsed| any(&parsed, &found)))
}
fn lossy(text: &kassiber_pyjson::PyStr) -> String {
    let points = text.code_points();
    points
        .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}
/// An exchange error body for display: a JSON body has every decoded
/// string and key withheld (so an escape cannot hide a credential) and is
/// written back only when something changed; any other body is withheld
/// as text. A JSON body is then withheld as text too, for what its decoded
/// strings do not show (a duplicate key's dropped value), with forms of at
/// least [`TEXT_FORM_CHARS`] characters only: numbers stay, so an error code
/// survives a short credential.
fn scrub_error_body(body: &[u8], forms: &kassiber_credentials::Forms) -> Vec<u8> {
    fn walk(value: &mut kassiber_pyjson::Value, forms: &kassiber_credentials::Forms) -> bool {
        use kassiber_pyjson::{Key, PyStr, Value as Py};
        let clean = |text: &mut PyStr| {
            let plain = lossy(text);
            let withheld = kassiber_credentials::withhold(&plain, forms);
            let changed = withheld != plain;
            if changed {
                *text = PyStr::from(withheld.as_str());
            }
            changed
        };
        match value {
            Py::Str(text) => clean(text),
            Py::List(items) => items
                .iter_mut()
                .fold(false, |c, item| walk(item, forms) | c),
            Py::Dict(items) => items.iter_mut().fold(false, |c, (key, item)| {
                let key = match key {
                    Key::Str(name) => clean(name),
                    _ => false,
                };
                walk(item, forms) | key | c
            }),
            _ => false,
        }
    }
    let text = String::from_utf8_lossy(body);
    if let Ok(mut parsed) = kassiber_pytext::loads(&text) {
        let long = forms.at_least(TEXT_FORM_CHARS);
        if !walk(&mut parsed, forms) {
            let withheld = kassiber_credentials::withhold(&text, &long);
            return match withheld == text {
                true => body.to_vec(),
                false => withheld.into_bytes(),
            };
        }
        if let Ok(dumped) = kassiber_pyjson::dumps(&parsed, &Default::default()) {
            return kassiber_credentials::withhold(&dumped, &long).into_bytes();
        }
    }
    kassiber_credentials::withhold(&text, forms).into_bytes()
}
/// The shortest credential form a JSON error body is withheld of as text.
const TEXT_FORM_CHARS: usize = 4;
/// `str(UnicodeDecodeError)` for `bytes.decode("utf-8")`.
pub fn utf8_error(bytes: &[u8], error: &std::str::Utf8Error) -> String {
    let at = error.valid_up_to();
    let (end, reason) = match error.error_len() {
        Some(1) if matches!(bytes[at], 0x80..=0xC1 | 0xF5..=0xFF) => (at, "invalid start byte"),
        Some(n) => (at + n - 1, "invalid continuation byte"),
        None => (bytes.len() - 1, "unexpected end of data"),
    };
    if end == at {
        format!(
            "'utf-8' codec can't decode byte 0x{:02x} in position {at}: {reason}",
            bytes[at]
        )
    } else {
        format!("'utf-8' codec can't decode bytes in position {at}-{end}: {reason}")
    }
}
pub(crate) fn decode_json(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes)
        .map_err(|_| AppError::new("protocol_error", "Backend returned invalid JSON"))
}
fn label(url: &str) -> String {
    HttpUrl::parse(url)
        .map(|u| format!("{}://<redacted-host>", u.scheme()))
        .unwrap_or_else(|_| "backend endpoint".into())
}
fn retry_seconds(raw: &str) -> Option<u64> {
    // Numeric Retry-After is the common backend form. Date form is parsed in
    // UTC without locale or network access (email.utils semantics).
    let raw = raw.trim();
    if let Ok(n) = raw.parse::<i64>() {
        return Some(n.max(0) as u64);
    }
    let parts: Vec<_> = raw.split_whitespace().collect();
    let parts = if parts.first().is_some_and(|p| p.ends_with(',')) {
        &parts[1..]
    } else {
        &parts[..]
    };
    if parts.len() < 4 {
        return None;
    }
    let day: i64 = parts[0].parse().ok()?;
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| parts[1].eq_ignore_ascii_case(m))? as i64
        + 1;
    let mut year: i64 = parts[2].parse().ok()?;
    if year < 100 {
        year += if year >= 69 { 1900 } else { 2000 };
    }
    if !(1..=9999).contains(&year) {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let hms: Vec<i64> = parts[3]
        .split(':')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    if hms.len() != 3
        || !(1..=month_days[(month - 1) as usize]).contains(&day)
        || !(0..24).contains(&hms[0])
        || !(0..60).contains(&hms[1])
        || !(0..60).contains(&hms[2])
    {
        return None;
    }
    let zone = parts.get(4).copied().unwrap_or("GMT");
    let offset = match zone.to_ascii_uppercase().as_str() {
        "GMT" | "UTC" | "UT" | "-0000" | "+0000" => 0,
        _ => {
            if zone.len() != 5
                || !matches!(zone.as_bytes()[0], b'+' | b'-')
                || !zone.as_bytes()[1..].iter().all(u8::is_ascii_digit)
            {
                return None;
            }
            let hours: i64 = zone[1..3].parse().ok()?;
            let minutes: i64 = zone[3..].parse().ok()?;
            if hours >= 24 || minutes >= 60 {
                return None;
            }
            (hours * 3600 + minutes * 60) * if zone.starts_with('-') { -1 } else { 1 }
        }
    };
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let days =
        era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + (153 * m + 2) / 5 + day - 1 - 719468;
    let target = days
        .checked_mul(86400)?
        .checked_add(hms[0] * 3600 + hms[1] * 60 + hms[2] - offset)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    Some(target.saturating_sub(now).max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::retry_seconds;

    #[test]
    fn retry_after_dates_refuse_overflow_invalid_calendar_and_unicode_zone() {
        assert_eq!(retry_seconds("-1"), Some(0));
        assert_eq!(retry_seconds("Thu, 29 Feb 2024 00:00:00 GMT"), Some(0));
        for raw in [
            "Wed, 29 Feb 2023 00:00:00 GMT",
            "Fri, 01 Jan 9223372036854775807 00:00:00 GMT",
            "Thu, 01 Jan 1970 00:00:00 +0é0",
            "Thu, 01 Jan 1970 00:00:00 +2460",
        ] {
            assert_eq!(retry_seconds(raw), None, "{raw}");
        }
    }
}
#[derive(Default)]
struct LimiterState {
    active: usize,
    cooldown_until: Option<Instant>,
}
#[derive(Default)]
struct Limiter {
    state: Mutex<LimiterState>,
    wake: Condvar,
}
struct Slot(Arc<Limiter>);
impl Limiter {
    fn note_cooldown(&self, duration: Duration) {
        if duration.is_zero() {
            return;
        }
        let target = Instant::now() + duration;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.cooldown_until = Some(match state.cooldown_until {
            Some(existing) if existing > target => existing,
            _ => target,
        });
    }
    fn acquire(
        self: &Arc<Self>,
        deadline: Instant,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Slot> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            drop(state);
            check()?;
            let budget = remaining(deadline)?;
            state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let cooldown_wait = state
                .cooldown_until
                .and_then(|until| until.checked_duration_since(Instant::now()));
            if let Some(wait) = cooldown_wait {
                let slice = wait.min(budget).min(Duration::from_millis(100));
                state = self
                    .wake
                    .wait_timeout(state, slice)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
                continue;
            }
            if state.active < 8 {
                state.active += 1;
                return Ok(Slot(self.clone()));
            }
            let slice = budget.min(Duration::from_millis(100));
            state = self
                .wake
                .wait_timeout(state, slice)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active -= 1;
        self.0.wake.notify_one();
    }
}
fn host_limiter(url: &str) -> Result<Arc<Limiter>> {
    static HOSTS: OnceLock<Mutex<HashMap<String, Arc<Limiter>>>> = OnceLock::new();
    let parsed = HttpUrl::parse(url).map_err(|_| app("Invalid backend URL"))?;
    let host = parsed
        .host_str()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    Ok(HOSTS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(host)
        .or_default()
        .clone())
}

#[cfg(test)]
mod exchange_tests {
    use super::*;
    use kassiber_net::test_support::{HandledProvider, HandledRequest};
    use std::time::{Duration, Instant};

    const KEY: &str = "synthetic-key-0123456789";
    const SIGNATURE: &str = "0123456789abcdefsignature";

    /// What a JSON error body's decoded strings do not show is withheld as
    /// text too: a duplicate key's dropped value. Text-level forms are at
    /// least four characters, so a numeric code survives a short credential.
    #[test]
    fn json_error_bodies_withhold_what_the_decoded_strings_drop() {
        let forms = kassiber_credentials::credential_forms([KEY, "123"]);
        let scrub =
            |body: &str| String::from_utf8(scrub_error_body(body.as_bytes(), &forms)).unwrap();
        let duplicate = format!(r#"{{"error": "{KEY}", "error": "denied", "code": 123}}"#);
        assert_eq!(
            scrub(&duplicate),
            r#"{"error": "<redacted>", "error": "denied", "code": 123}"#
        );
        // Unchanged bodies keep their bytes; a short credential leaves the code.
        let plain = r#"{"error":"denied","code":123}"#;
        assert_eq!(scrub(plain), plain);
    }

    /// One exchange read carrying `KEY` and `SIGNATURE` headers from a
    /// loopback server answering `status` with `body`.
    fn read(
        status: u16,
        accepted: &'static [u16],
        withheld: &[&str],
        proxy: Option<&str>,
        body: impl Fn(&HandledRequest) -> String + Send + 'static,
    ) -> (HttpClient, Result<HttpResponse>) {
        let server = HandledProvider::start(move |request| (status, body(request))).unwrap();
        let mut config = json!({"name": "fixture", "url": server.base(), "timeout": 5});
        if let Some(proxy) = proxy {
            config["tor_proxy"] = json!(proxy);
        }
        let backend = Backend::new(&config, false).unwrap();
        let consent = kassiber_net::Consent::explicit_trigger(backend.request());
        let gate = Gate::new(
            || kassiber_net::Offline::Online,
            kassiber_net::Ledger::default(),
        );
        let client = HttpClient::new(backend, gate, "test").withholding(withheld);
        let url = format!("{}/v2/x", server.base());
        let deadline = Instant::now() + Duration::from_secs(30);
        let response = client.authenticated_response(
            &url,
            Some(&consent),
            deadline,
            "Fixture",
            accepted,
            || {
                let request = HttpRequest::new("GET", &url).map_err(|e| e.app_error())?;
                let request = request
                    .header("Bitvavo-Access-Key", KEY)
                    .map_err(|e| e.app_error())?;
                request
                    .header("Bitvavo-Access-Signature", SIGNATURE)
                    .map_err(|e| e.app_error())
            },
        );
        (client, response)
    }

    /// Accepted and raised error bodies withhold every request credential at
    /// any length (the client's short `pw7` too), after JSON escapes are
    /// decoded, so another hex case behind `\u0041` cannot pass. Numbers stay.
    #[test]
    fn exchange_error_bodies_withhold_every_request_credential_at_any_length() {
        let echo = |request: &HandledRequest| {
            let key = request.header("Bitvavo-Access-Key").unwrap();
            let signature = request.header("Bitvavo-Access-Signature").unwrap();
            format!(
                r#"{{"errorCode":305,"error":"bad {key} {signature} pw7 \u0041BCDEF0123456789ABCDEF"}}"#
            )
        };
        let withheld = ["pw7", "abcdef0123456789abcdef"];
        let (_, accepted) = read(403, &[403], &withheld, None, echo);
        assert_eq!(
            String::from_utf8(accepted.unwrap().body).unwrap(),
            r#"{"errorCode": 305, "error": "bad <redacted> <redacted> <redacted> <redacted>"}"#
        );
        let (_, raised) = read(500, &[], &withheld, None, echo);
        let message = raised.err().unwrap().message;
        assert!(
            message.ends_with(r#"bad <redacted> <redacted> <redacted> <redacted>"}"#),
            "{message}"
        );
        let (_, raised) = read(500, &[], &["pw7"], None, |_| "proxy says pw7".into());
        assert!(raised
            .err()
            .unwrap()
            .message
            .ends_with("proxy says <redacted>"));
    }

    /// A successful body is data: it comes back byte for byte. An echoed
    /// request credential (escaped too) is noted; short values never match.
    #[test]
    fn successful_exchange_bodies_are_kept_and_echoes_noted() {
        let plain = r#"{"id":"DEADBEEF-1","amount":1,"a":"x"}"#;
        let (client, response) = read(200, &[], &["1", "a", "deadbeef"], None, |_| plain.into());
        assert_eq!(response.unwrap().body, plain.as_bytes());
        assert!(!client.echoed());
        // The signature's `s` behind a JSON escape: only decoding finds it.
        let escaped = format!(
            r#"{{"address":"{}"}}"#,
            SIGNATURE.replacen('s', "\\u0073", 1)
        );
        let body = escaped.clone();
        let (client, response) = read(200, &[], &[], None, move |_| body.clone());
        assert_eq!(response.unwrap().body, escaped.as_bytes());
        assert!(client.echoed());
    }

    /// Every credential an exchange request carried (here the proxy's short
    /// password and the signature) is kept for error display, even when the
    /// request then fails.
    #[test]
    fn exchange_reads_keep_every_request_credential_for_display() {
        let proxy = "socks5h://proxyuser:pw@127.0.0.1:9";
        let (client, _) = read(200, &[], &[], Some(proxy), |_| "{}".into());
        let shown = client.display_credentials();
        assert!(shown.iter().any(|c| c == "pw"), "{shown:?}");
        assert!(shown.iter().any(|c| c == SIGNATURE), "{shown:?}");
    }
}
