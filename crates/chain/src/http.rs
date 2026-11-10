use super::*;
use kassiber_net::{Gate, HttpOptions, HttpRequest, HttpResponse, HttpUrl};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

/// Esplora/mempool (Bitcoin or Liquid) and shared Core HTTP transport.
/// The caller owns action/book binding; a saved backend never mints Consent.
pub(crate) enum StatusPolicy {
    Backend,
    Core,
    Return,
    /// As `Backend`, but these statuses are answered rather than raised.
    Accept(&'static [u16]),
}

pub struct HttpClient {
    pub(crate) backend: Backend,
    pub(crate) gate: Gate,
    pub(crate) version: String,
    pub(crate) withheld: Vec<String>,
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
                StatusPolicy::Backend,
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
            StatusPolicy::Accept(accepted),
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
        Self {
            backend,
            gate,
            version: version.into(),
            withheld: vec![],
        }
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
    /// Optional cache anchor, read only under the caller's exact sync grant.
    /// A missing or malformed tip disables reuse; it never means an empty chain.
    pub fn esplora_tip_hash(&self, consent: Option<&Consent>, deadline: Instant) -> Option<String> {
        let text = self.get_text("blocks/tip/hash", consent, deadline).ok()?;
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
        let response = self.get(path, "text/plain", consent, deadline)?;
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
                let auth = config::value(&self.backend.config, &["auth_header"])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        config::value(&self.backend.config, &["token"])
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(|s| format!("Bearer {s}"))
                    });
                if let Some(auth) = auth {
                    request = request
                        .header("Authorization", &auth)
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
        let route = self
            .gate
            .authorize(self.backend.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        remaining(deadline)?;
        request
            .send(
                route,
                HttpOptions {
                    timeout: self.backend.timeout,
                    deadline: Some(deadline),
                    max_response_bytes: cap,
                    ..Default::default()
                },
            )
            .map_err(|e| e.app_error())
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
    fn retry_checked(
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
        let limiter = host_limiter(url)?;
        let mut cumulative = Duration::ZERO;
        for attempt in 0..3 {
            check()?;
            let slot = limiter.acquire(deadline)?;
            check()?;
            let request = build()?;
            let headers = request.headers().to_vec();
            let response = self.send(request, consent, deadline, cap)?;
            drop(slot);
            if !matches!(response.status, 429 | 503) {
                if (200..300).contains(&response.status)
                    || (matches!(policy, StatusPolicy::Core)
                        && matches!(response.status, 400 | 404 | 500))
                    || matches!(policy, StatusPolicy::Return)
                    || matches!(policy, StatusPolicy::Accept(statuses) if statuses.contains(&response.status))
                {
                    return Ok(response);
                }
                // The body is the server's text: withhold every credential
                // the request carried, and the client's own (an exchange key
                // and secret), before the cut, as Python does.
                let mut credentials = kassiber_credentials::request_credentials(
                    Some(url),
                    &headers,
                    self.backend.proxy_url().as_deref(),
                );
                credentials.extend(self.withheld.iter().cloned());
                let withheld = kassiber_credentials::credential_forms(credentials);
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
struct Limiter {
    active: Mutex<usize>,
    wake: Condvar,
}
struct Slot(Arc<Limiter>);
impl Limiter {
    fn acquire(self: &Arc<Self>, deadline: Instant) -> Result<Slot> {
        let mut active = self.active.lock().unwrap();
        while *active >= 8 {
            active = self
                .wake
                .wait_timeout(active, remaining(deadline)?)
                .unwrap()
                .0;
        }
        remaining(deadline)?;
        *active += 1;
        Ok(Slot(self.clone()))
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        *self.0.active.lock().unwrap() -= 1;
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
