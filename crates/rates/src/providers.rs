//! Every provider request consumes an exact-action grant through kassiber_net.
use super::*;
use kassiber_net::HttpUrl as Url;
use kassiber_net::{
    Consent, Endpoint, Gate, HttpOptions, HttpRequest, Ledger, Offline, Proxy, Request, Subsystem,
    TlsTrust,
};
use std::{path::Path, time::Duration};

// urllib.parse.unquote preserves malformed escapes and replaces invalid UTF-8.
pub(crate) fn unquote(raw: &str) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut input = raw.as_bytes().iter().copied().peekable();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let mut tail = input.clone();
            if let (Some(a), Some(b)) = (tail.next(), tail.next()) {
                if let (Some(a), Some(b)) = ((a as char).to_digit(16), (b as char).to_digit(16)) {
                    bytes.push((a * 16 + b) as u8);
                    input = tail;
                    continue;
                }
            }
        }
        bytes.push(byte);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchAction {
    Sync,
    Rebuild,
    Latest,
    EnabledRefresh,
}
/// Service authority: provider, pair and operation cannot be reused for another
/// action. The trusted CLI/desktop boundary constructs this only after checking
/// its user action or per-book source feature. A provider/config is not consent.
pub struct FetchConsent {
    action: FetchAction,
    source: String,
    pair: Option<String>,
    book_id: String,
    profile: Option<String>,
}
impl FetchConsent {
    pub fn explicit(
        conn: &Connection,
        action: FetchAction,
        source: &str,
        pair: Option<&str>,
    ) -> Result<Self> {
        if action == FetchAction::EnabledRefresh {
            return Err(validation(
                "Background refresh requires a scoped feature grant",
            ));
        }
        Ok(Self {
            action,
            source: source.to_owned(),
            pair: pair.map(require_supported_pair).transpose()?,
            book_id: setting(conn, "database_instance_id")?
                .ok_or_else(|| validation("Book identity is unavailable"))?,
            profile: None,
        })
    }
    pub fn enabled_refresh(
        conn: &Connection,
        profile_id: &str,
        source: &str,
        pair: &str,
    ) -> Result<Self> {
        require_live_enabled(conn, profile_id)?;
        Ok(Self {
            action: FetchAction::EnabledRefresh,
            source: source.into(),
            pair: Some(require_supported_pair(pair)?),
            book_id: setting(conn, "database_instance_id")?
                .ok_or_else(|| validation("Book identity is unavailable"))?,
            profile: Some(profile_id.to_owned()),
        })
    }
    fn verify(
        &self,
        conn: &Connection,
        action: FetchAction,
        source: &str,
        pair: &str,
    ) -> Result<()> {
        if !self.matches(action, source, pair)
            || setting(conn, "database_instance_id")?.as_deref() != Some(&self.book_id)
        {
            return Err(AppError::new(
                "interaction_required",
                "Network action requires current consent",
            ));
        }
        if let Some(profile) = &self.profile {
            require_live_enabled(conn, profile)?;
        }
        Ok(())
    }
    fn matches(&self, action: FetchAction, source: &str, pair: &str) -> bool {
        self.action == action
            && self.source == source
            && self.pair.as_ref().is_none_or(|p| p == pair)
    }
}
/// Stateless client; construction and local reads never open a connection.
pub struct Providers {
    gate: Gate,
    version: String,
    #[cfg(test)]
    pub(crate) test_base: Option<String>,
}
impl Providers {
    pub fn new(gate: Gate, version: &str) -> Self {
        Self {
            gate,
            version: version.into(),
            #[cfg(test)]
            test_base: None,
        }
    }
    #[cfg(test)]
    pub(crate) fn gate_for_test(&mut self, offline: Offline) {
        self.gate = Gate::new(move || offline, Ledger::process());
    }
    pub fn system(version: &str) -> Self {
        Self::new(Gate::new(offline_preference, Ledger::process()), version)
    }
    fn json(
        &self,
        conn: &Connection,
        url: &str,
        proxy: Option<&str>,
        timeout: i64,
        consent: Option<&FetchConsent>,
        scope: (FetchAction, &str, &str),
    ) -> Result<Value> {
        let (action, source, pair) = scope;
        if let Some(c) = consent {
            c.verify(conn, action, source, pair)?;
        }
        #[cfg(test)]
        let replaced = self.test_base.as_ref().map(|base| {
            let parsed = Url::parse(url).unwrap();
            format!(
                "{base}{}{}",
                parsed.path(),
                parsed.query().map(|q| format!("?{q}")).unwrap_or_default()
            )
        });
        #[cfg(test)]
        let url = replaced.as_deref().unwrap_or(url);
        let parsed = Url::parse(url).map_err(|_| validation("Invalid rate provider URL"))?;
        let trust = if parsed.scheme() == "https" {
            TlsTrust::python_default().map_err(|e| e.app_error())?
        } else {
            TlsTrust::Plaintext
        };
        let mut request = Request::new(
            Subsystem::Pricing,
            Endpoint::new(
                parsed
                    .host_str()
                    .ok_or_else(|| validation("Invalid rate provider URL"))?,
                parsed.port_or_known_default().unwrap_or(80),
            )
            .map_err(|e| e.app_error())?,
            trust,
        );
        if let Some(proxy) = proxy.filter(|p| !p.is_empty()) {
            let proxy = proxy.trim();
            let normalized = if proxy.contains("://") {
                proxy.to_owned()
            } else {
                format!("socks5h://{proxy}")
            };
            let url =
                Url::parse(&normalized).map_err(|_| validation("Invalid rate provider proxy"))?;
            if url.scheme().starts_with("socks5")
                && !url.username().is_empty()
                && url.password().is_none()
            {
                return Err(validation(
                    "SOCKS5 proxy credentials must include both username and password",
                )
                .with_hint(
                    "Use socks5h://USER:PASS@HOST:PORT, percent-encoding any special characters.",
                ));
            }
            let credentials = if url.username().is_empty() && url.password().is_none() {
                None
            } else {
                Some((
                    unquote(url.username()),
                    unquote(url.password().unwrap_or("")),
                ))
            };
            request = request.with_proxy(
                Proxy::new(
                    url.scheme(),
                    Endpoint::new(
                        url.host_str()
                            .ok_or_else(|| validation("Invalid rate provider proxy"))?,
                        url.port()
                            .unwrap_or(if url.scheme() == "http" { 80 } else { 9050 }),
                    )
                    .map_err(|e| e.app_error())?,
                    credentials,
                )
                .map_err(|e| e.app_error())?,
            );
        }
        let retry_attempt = std::cell::Cell::new(0);
        let mut cumulative = 0.0;
        let response = loop {
            let proof = consent.map(|c| {
                if c.action == FetchAction::EnabledRefresh {
                    Consent::narrowly_scoped(&request)
                } else {
                    Consent::explicit_trigger(&request)
                }
            });
            if let Some(c) = consent {
                c.verify(conn, action, source, pair)?;
            }
            let route = self
                .gate
                .authorize(request.clone(), proof)
                .map_err(|e| e.app_error())?;
            let permit = HostPermit::acquire(parsed.host_str().unwrap_or(""));
            if let Some(c) = consent {
                c.verify(conn, action, source, pair)?;
            }
            let response = HttpRequest::new("GET", url)
                .map_err(|e| e.app_error())?
                .header("Accept", "application/json")
                .map_err(|e| e.app_error())?
                .header("User-Agent", &format!("kassiber/{}", self.version))
                .map_err(|e| e.app_error())?
                .send(
                    route,
                    HttpOptions {
                        timeout: Duration::from_secs(timeout.max(1) as u64),
                        ..Default::default()
                    },
                )
                .map_err(|e| e.app_error())?;
            drop(permit);
            if ![429, 503].contains(&response.status) {
                break response;
            }
            let after = response
                .header("Retry-After")
                .and_then(|s| kassiber_pytext::parse_int(s).ok())
                .and_then(|n| i64::try_from(n).ok())
                .map(|n| n.max(0));
            let attempt = retry_attempt.get();
            let jitter = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as f64
                / 1_000_000_000.0)
                * 0.5;
            let delay = after
                .map(|n| n as f64)
                .unwrap_or((0.5 * 2.0f64.powi(attempt)).min(8.0) + jitter);
            if attempt >= 2 || cumulative + delay > 20.0 {
                return Err(AppError::new("rate_limited",if response.status==429 {format!("Rate provider rate limited the request for {}://<redacted-host> (HTTP 429)",parsed.scheme())}else{format!("Rate provider is temporarily unavailable for {}://<redacted-host> (HTTP {})",parsed.scheme(),response.status)}).retryable(true).with_details(dict([("retry_after_seconds",after.map_or(Value::Null,Value::from))])));
            }
            std::thread::sleep(Duration::from_secs_f64(delay));
            cumulative += delay;
            retry_attempt.set(attempt + 1);
        };
        if !(200..300).contains(&response.status) {
            return Err(AppError::new(
                "app_error",
                format!(
                    "HTTP {} from backend for {}://<redacted-host>",
                    response.status,
                    parsed.scheme()
                ),
            ));
        }
        let raw = std::str::from_utf8(&response.body).map_err(|_| {
            AppError::new("upstream_error", "Rate provider returned invalid UTF-8").retryable(true)
        })?;
        kassiber_pytext::loads(raw).map_err(|_| {
            AppError::new("upstream_error", "Rate provider returned invalid JSON").retryable(true)
        })
    }
}
fn offline_preference() -> Offline {
    let path = if let Some(p) =
        std::env::var_os("KASSIBER_OFFLINE_PREFERENCE_FILE").filter(|v| !v.is_empty())
    {
        match kassiber_paths::Context::from_env()
            .and_then(|context| context.expand_user(Path::new(&p)))
        {
            Ok(p) => Some(p),
            Err(_) => return Offline::Unavailable,
        }
    } else {
        (|| {
            kassiber_paths::Context::from_env()
                .ok()?
                .default_state_root()
                .ok()
                .map(|p| p.join("config/offline-mode.json"))
        })()
    };
    let Some(path) = path else {
        return Offline::Unavailable;
    };
    let metadata = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Offline::Online,
        Err(_) => return Offline::Unavailable,
        Ok(m) => m,
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4096 {
        return Offline::Unavailable;
    }
    let payload = match kassiber_paths::read_private_json(&path) {
        Ok(Some(v)) => v,
        _ => return Offline::Unavailable,
    };
    if payload.get("schema_version").and_then(|v| v.as_i64()) != Some(1) {
        return Offline::Unavailable;
    }
    match payload.get("enabled").and_then(|v| v.as_bool()) {
        Some(false) => Offline::Online,
        Some(true) => Offline::Offline,
        None => Offline::Unavailable,
    }
}
pub fn require_live_enabled(conn: &Connection, profile_id: &str) -> Result<()> {
    let raw = setting(conn, &format!("freshness.policy.profile.{profile_id}"))?;
    let enabled = raw
        .and_then(|r| kassiber_pytext::loads(&r).ok())
        .and_then(|v| get(&v, "source_classes").cloned())
        .and_then(|v| get(&v, "market_rates").cloned())
        .as_ref()
        .is_some_and(truthy);
    if enabled {
        Ok(())
    } else {
        Err(AppError::new("live_market_rates_disabled","Live market-rate provider lookups are disabled for this book").with_hint("Enable live market-rate lookups in Settings > Market data, or use the bundled/local Kraken history for offline pricing.").with_details(dict([("profile_id",s(profile_id))])))
    }
}

#[derive(Clone)]
struct Candle {
    timestamp: String,
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
}
fn parse_candles(payload: Value, seconds: i64) -> Result<Vec<Candle>> {
    let Value::List(rows) = payload else {
        return Err(AppError::new(
            "upstream_error",
            "Coinbase Exchange response did not contain candle rows",
        )
        .retryable(true));
    };
    let mut output = Vec::new();
    for row in rows {
        let Value::List(cells) = row else { continue };
        if cells.len() < 6 {
            continue;
        }
        let Some(open) = py_integer(&cells[0]) else {
            continue;
        };
        let Some(ts) = open.checked_add(seconds).and_then(time::epoch) else {
            continue;
        };
        let values: Vec<_> = cells[1..6].iter().map(py_str).collect();
        if values.iter().any(|v| decimal(v).is_err()) {
            continue;
        }
        output.push(Candle {
            timestamp: ts,
            low: values[0].clone(),
            high: values[1].clone(),
            open: values[2].clone(),
            close: values[3].clone(),
            volume: values[4].clone(),
        });
    }
    output.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(output)
}
fn py_integer(v: &Value) -> Option<i64> {
    match v {
        Value::Float(f) if f.is_finite() => Some(*f as i64),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => kassiber_pytext::parse_int(&py_str(v))
            .ok()
            .and_then(|n| i64::try_from(n).ok()),
    }
}
fn insert_candle(conn: &Connection, pair: &str, c: &Candle, fetched: &str) -> Result<()> {
    upsert(
        conn,
        &Sample {
            pair,
            timestamp: &c.timestamp,
            rate: &c.close,
            source: "coinbase-exchange",
            fetched_at: fetched,
            granularity: Some("minute"),
            method: Some("product_candles"),
            open: Some(&c.open),
            high: Some(&c.high),
            low: Some(&c.low),
            close: Some(&c.close),
            volume: Some(&c.volume),
            ..Default::default()
        },
    )?;
    Ok(())
}
#[derive(Clone)]
struct Mempool {
    base: String,
    proxy: Option<String>,
    timeout: i64,
}
fn mempool_backend(conn: &Connection) -> Result<Mempool> {
    let default = setting(conn, "default_backend")?;
    let backends=query(conn,"SELECT name,kind,chain,network,url,timeout,tor_proxy,config_json FROM backends ORDER BY name",[])?;
    let mut candidates = Vec::new();
    for row in backends {
        if !["esplora", "mempool", "liquid-esplora"]
            .contains(&text(&row, "kind").unwrap_or("").to_lowercase().as_str())
        {
            continue;
        }
        let kind = text(&row, "kind").unwrap_or("");
        let chain =
            text(&row, "chain")
                .filter(|v| !v.is_empty())
                .unwrap_or(if kind == "liquid-esplora" {
                    "liquid"
                } else {
                    "bitcoin"
                });
        if chain != "bitcoin" {
            continue;
        }
        let Some(raw) = text(&row, "url") else {
            continue;
        };
        let Ok(mut url) = Url::parse(raw) else {
            continue;
        };
        if !["http", "https"].contains(&url.scheme()) {
            continue;
        }
        // Python strips queries/fragments and appends /api when absent.
        url.set_query(None);
        url.set_fragment(None);
        let base = url.as_str().trim_end_matches('/').to_owned();
        let base = if base.ends_with("/api") {
            base
        } else {
            format!("{base}/api")
        };
        let name = text(&row, "name").unwrap_or("").to_owned();
        let config = text(&row, "config_json")
            .and_then(|v| kassiber_pytext::loads(v).ok())
            .unwrap_or(Value::Null);
        let rank = (
            text(&config, "infrastructure_owner") != Some("self"),
            ["mempool", "fulcrum", "liquid", "liquid-blockstream"].contains(&name.as_str()),
            default.as_ref() != Some(&name),
            name,
        );
        candidates.push((
            rank,
            text(&row, "network")
                .filter(|v| !v.is_empty())
                .unwrap_or("main")
                .to_owned(),
            Mempool {
                base,
                proxy: text(&row, "tor_proxy").map(str::to_owned),
                timeout: integer(&row, "timeout", 30)?.max(1),
            },
        ));
    }
    let has_regtest = (|| -> Result<bool> {
        let Some(profile) = setting(conn, "context_profile")? else {
            return Ok(false);
        };
        for row in query(
            conn,
            "SELECT config_json FROM wallets WHERE profile_id=?",
            [profile],
        )? {
            let Some(config) =
                text(&row, "config_json").and_then(|s| kassiber_pytext::loads(s).ok())
            else {
                continue;
            };
            if ["regtest", "elementsregtest", "liquidregtest"].contains(
                &text(&config, "network")
                    .unwrap_or("")
                    .trim()
                    .to_lowercase()
                    .as_str(),
            ) {
                return Ok(true);
            }
            if let Some(name) = text(&config, "backend") {
                let row = query(
                    conn,
                    "SELECT network FROM backends WHERE lower(name)=?",
                    [name.trim().to_lowercase()],
                )?;
                if row.first().is_some_and(|r| {
                    ["regtest", "elementsregtest", "liquidregtest"].contains(
                        &text(r, "network")
                            .unwrap_or("")
                            .trim()
                            .to_lowercase()
                            .as_str(),
                    )
                }) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    })()?;
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    for network in ["main", if has_regtest { "regtest" } else { "main" }] {
        if let Some((_, _, backend)) = candidates.iter().find(|(_, n, _)| n == network) {
            return Ok(backend.clone());
        }
    }
    Err(AppError::new("config_error","No HTTP mempool/esplora Bitcoin backend is configured for market prices").with_hint("Create a Bitcoin mempool/esplora backend that points at your mempool instance before selecting the mempool price source."))
}
fn mempool_price(payload: &Value, fiat: &str, default: &str) -> Result<(String, String)> {
    let row_price = |row: &Value| {
        let price = get(row, fiat)
            .filter(|v| !matches!(v, Value::Null))
            .or_else(|| get(row, &fiat.to_lowercase()).filter(|v| !matches!(v, Value::Null)))?;
        let ts = get(row, "time")
            .filter(|v| truthy(v))
            .or_else(|| get(row, "timestamp"))
            .and_then(py_integer)
            .and_then(time::epoch)
            .unwrap_or_else(|| default.into());
        Some((ts, py_str(price)))
    };
    if let Some(Value::List(prices)) = get(payload, "prices") {
        for row in prices.iter().rev() {
            if let Some(p) = row_price(row) {
                return Ok(p);
            }
        }
    }
    row_price(payload).ok_or_else(|| {
        AppError::new(
            "upstream_error",
            format!("mempool prices response did not contain a {fiat} BTC price"),
        )
        .retryable(true)
    })
}
impl Providers {
    fn coinbase(
        &self,
        conn: &Connection,
        pair: &str,
        start: i64,
        end: i64,
        consent: Option<&FetchConsent>,
        action: FetchAction,
    ) -> Result<Vec<Candle>> {
        let mut url = Url::parse(&format!(
            "https://api.exchange.coinbase.com/products/{pair}/candles"
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("granularity", "60")
            .append_pair(
                "start",
                &time::format(start).ok_or_else(|| validation("Rate window out of range"))?,
            )
            .append_pair(
                "end",
                &time::format(end).ok_or_else(|| validation("Rate window out of range"))?,
            );
        parse_candles(
            self.json(
                conn,
                url.as_str(),
                None,
                30,
                consent,
                (action, "coinbase-exchange", pair),
            )?,
            60,
        )
    }
    fn coingecko(
        &self,
        conn: &Connection,
        pair: &str,
        days: i64,
        consent: Option<&FetchConsent>,
        action: FetchAction,
    ) -> Result<Vec<(String, String)>> {
        let fiat = pair.split_once('-').unwrap().1.to_lowercase();
        let url=format!("https://api.coingecko.com/api/v3/coins/bitcoin/market_chart?vs_currency={fiat}&days={days}");
        let payload = self.json(conn, &url, None, 30, consent, (action, "coingecko", pair))?;
        let Some(Value::List(prices)) = get(&payload, "prices") else {
            return Err(AppError::new(
                "upstream_error",
                "CoinGecko response did not contain a prices array",
            )
            .retryable(true));
        };
        let mut out = Vec::new();
        for row in prices {
            let Value::List(c) = row else { continue };
            if c.len() < 2 {
                continue;
            }
            let parse = |v: &Value| kassiber_pytext::parse_float(&py_str(v)).ok();
            let (Some(ms), Some(rate)) = (parse(&c[0]), parse(&c[1])) else {
                continue;
            };
            if !ms.is_finite() {
                continue;
            }
            if let Some(ts) = time::epoch((ms / 1000.0).floor() as i64) {
                out.push((ts, kassiber_pyjson::float_repr(rate)))
            }
        }
        Ok(out)
    }
    fn mempool(
        &self,
        conn: &Connection,
        pair: &str,
        at: Option<&str>,
        consent: Option<&FetchConsent>,
        action: FetchAction,
    ) -> Result<(String, String, String)> {
        let backend = mempool_backend(conn)?;
        let fetched = time::iso_now();
        let fiat = pair.split_once('-').unwrap().1;
        let mut url = Url::parse(&format!(
            "{}/{}",
            backend.base,
            if at.is_some() {
                "v1/historical-price"
            } else {
                "v1/prices"
            }
        ))
        .map_err(|_| validation("Invalid mempool URL"))?;
        let default = if let Some(at) = at {
            let us = time::parse(at).ok_or_else(|| validation("Invalid rate timestamp"))?
                / 60_000_000
                * 60_000_000;
            url.query_pairs_mut()
                .append_pair("currency", fiat)
                .append_pair("timestamp", &((us - time::UNIX) / 1_000_000).to_string());
            time::format(us).unwrap()
        } else {
            fetched.clone()
        };
        let payload = self.json(
            conn,
            url.as_str(),
            backend.proxy.as_deref(),
            backend.timeout,
            consent,
            (action, "mempool", pair),
        )?;
        let (timestamp, rate) = mempool_price(&payload, fiat, &default)?;
        Ok((
            if at.is_some() { default } else { timestamp },
            rate,
            fetched,
        ))
    }
    pub fn latest(
        &self,
        conn: &Connection,
        pair: Option<&str>,
        source: Option<&str>,
        consent: Option<&FetchConsent>,
        action: FetchAction,
    ) -> Result<Value> {
        if !matches!(action, FetchAction::Latest | FetchAction::EnabledRefresh) {
            return Err(AppError::new(
                "interaction_required",
                "Latest rates require their matching action grant",
            ));
        }
        let source = source
            .map(normalize_market_provider)
            .transpose()?
            .unwrap_or(market_provider(conn)?);
        let pairs = selected_pairs(pair)?;
        let mut fetched = time::iso_now();
        let mut result = Vec::new();
        for pair in pairs {
            let (granularity, method, lookback) = match source.as_str() {
                "coinbase-exchange" => ("minute", "product_candles", 5),
                "mempool" => ("latest", "mempool_prices", 0),
                _ => ("five_minute", "market_chart", 1440),
            };
            let timestamp = match source.as_str() {
                "coinbase-exchange" => {
                    let end = time::now() / 60_000_000 * 60_000_000;
                    let samples =
                        self.coinbase(conn, &pair, end - 5 * 60_000_000, end, consent, action)?;
                    let latest = samples
                        .iter()
                        .rfind(|c| time::parse(&c.timestamp).is_some_and(|ts| ts <= end));
                    if let Some(c) = latest {
                        insert_candle(conn, &pair, c, &fetched)?;
                    }
                    latest.map(|c| c.timestamp.clone())
                }
                "mempool" => {
                    let (ts, rate, f) = self.mempool(conn, &pair, None, consent, action)?;
                    fetched = f;
                    upsert(
                        conn,
                        &Sample {
                            pair: &pair,
                            timestamp: &ts,
                            rate: &rate,
                            source: &source,
                            fetched_at: &fetched,
                            granularity: Some(granularity),
                            method: Some(method),
                            ..Default::default()
                        },
                    )?;
                    Some(ts)
                }
                _ => {
                    let samples = self.coingecko(conn, &pair, 1, consent, action)?;
                    if let Some((ts, rate)) = samples.last() {
                        upsert(
                            conn,
                            &Sample {
                                pair: &pair,
                                timestamp: ts,
                                rate,
                                source: &source,
                                fetched_at: &fetched,
                                granularity: Some(granularity),
                                method: Some(method),
                                ..Default::default()
                            },
                        )?;
                    }
                    samples.last().map(|(ts, _)| ts.clone())
                }
            };
            commit(conn)?;
            result.push(dict([
                ("pair", s(pair)),
                ("source", s(source.as_str())),
                ("samples", Value::from(i64::from(timestamp.is_some()))),
                ("granularity", s(granularity)),
                ("method", s(method)),
                ("mode", s("latest_quote")),
                ("lookback_minutes", Value::from(lookback)),
                ("timestamp", optional(timestamp.as_deref())),
                ("fetched_at", s(fetched.as_str())),
            ]));
        }
        Ok(Value::List(result))
    }
}
fn selected_pairs(pair: Option<&str>) -> Result<Vec<String>> {
    pair.filter(|p| !p.is_empty())
        .map(|p| require_supported_pair(p).map(|p| vec![p]))
        .unwrap_or_else(|| Ok(PAIRS.iter().map(|p| p.to_string()).collect()))
}
pub fn missing_price_sql(prefix: &str) -> String {
    format!("(({prefix}fiat_rate IS NULL OR {prefix}fiat_rate<=0) AND ({prefix}fiat_rate_exact IS NULL OR CAST({prefix}fiat_rate_exact AS REAL)<=0) AND ({prefix}fiat_value IS NULL OR {prefix}fiat_value<=0) AND ({prefix}fiat_value_exact IS NULL OR CAST({prefix}fiat_value_exact AS REAL)<=0))")
}
fn needed_minutes(
    conn: &Connection,
    pairs: &[String],
) -> Result<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>> {
    let mut needed: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> = pairs
        .iter()
        .map(|p| (p.clone(), Default::default()))
        .collect();
    let sql=format!("SELECT t.occurred_at,t.confirmed_at,t.asset,t.fiat_currency,p.fiat_currency AS profile_fiat_currency FROM transactions t JOIN profiles p ON p.id=t.profile_id WHERE t.excluded=0 AND ({} OR (t.fiat_price_source='rates_cache' AND t.pricing_source_kind IS NULL AND t.pricing_quality IS NULL)) ORDER BY COALESCE(t.confirmed_at,t.occurred_at),t.created_at,t.id",missing_price_sql("t."));
    let now = time::now() / 60_000_000 * 60_000_000;
    for row in query(conn, &sql, [])? {
        let fiat = text(&row, "fiat_currency")
            .filter(|s| !s.is_empty())
            .or_else(|| text(&row, "profile_fiat_currency"))
            .unwrap_or("");
        let Some(pair) = transaction_rate_pair(text(&row, "asset").unwrap_or(""), fiat) else {
            continue;
        };
        let Some(target) = needed.get_mut(&pair) else {
            continue;
        };
        let at = text(&row, "confirmed_at")
            .filter(|s| !s.is_empty())
            .or_else(|| text(&row, "occurred_at"))
            .unwrap_or("");
        if let Some(us) = time::parse(at) {
            let minute = us / 60_000_000 * 60_000_000;
            if minute <= now {
                target.insert(time::format(minute).unwrap());
            }
        }
    }
    Ok(needed)
}
pub(crate) fn windows(minutes: &std::collections::BTreeSet<String>, now: i64) -> Vec<(i64, i64)> {
    let mut windows = std::collections::BTreeSet::new();
    let now = now / 60_000_000 * 60_000_000;
    for minute in minutes {
        let Some(close) = time::parse(minute) else {
            continue;
        };
        if close > now {
            continue;
        }
        let day = close / 86_400_000_000 * 86_400_000_000;
        let block = (close - day - 60_000_000).div_euclid(18_000_000_000);
        let start = day + block * 18_000_000_000;
        let end = (start + 18_000_000_000).min(now);
        windows.insert((start, end));
    }
    windows.into_iter().collect()
}
impl Providers {
    pub fn sync(
        &self,
        conn: &Connection,
        args: &Value,
        consent: Option<&FetchConsent>,
        action: FetchAction,
    ) -> Result<Value> {
        if !matches!(
            action,
            FetchAction::Sync | FetchAction::Rebuild | FetchAction::EnabledRefresh
        ) {
            return Err(AppError::new(
                "interaction_required",
                "Historical rates require their matching action grant",
            ));
        }
        if action == FetchAction::EnabledRefresh
            && get(args, "warm_cache_when_idle").is_some_and(truthy)
        {
            return Err(AppError::new(
                "interaction_required",
                "Background rates sync cannot warm an idle cache",
            ));
        }
        let raw = text(args, "source").unwrap_or("coinbase-exchange");
        let source = raw.trim().to_lowercase();
        if !SOURCES.contains(&source.as_str()) {
            return Err(
                validation(format!("Unknown rate source '{raw}'")).with_hint(
                    "Supported sources: coinbase-exchange, mempool, kraken-csv, coingecko",
                ),
            );
        }
        let path = text(args, "path").filter(|s| !s.is_empty());
        if source == "kraken-csv" {
            let summary=csv::sync(conn,text(args,"pair").filter(|s|!s.is_empty()),path.ok_or_else(||validation("--path is required for --source kraken-csv").with_hint("Download Kraken's OHLCVT archive yourself and pass the local .zip or .csv path"))?)?;
            if action != FetchAction::Rebuild {
                commit(conn)?;
            }
            return Ok(summary);
        }
        if path.is_some() {
            return Err(validation(
                "--path is only supported for --source kraken-csv",
            ));
        }
        let days = integer(args, "days", 30)?;
        if days <= 0 {
            return Err(validation("--days must be positive"));
        }
        let pairs = selected_pairs(text(args, "pair"))?;
        let fetched = time::iso_now();
        if source == "coingecko" {
            let granularity = if days > 90 {
                "daily"
            } else if days > 1 {
                "hourly"
            } else {
                "five_minute"
            };
            let mut result = Vec::new();
            for pair in pairs {
                let samples = self.coingecko(conn, &pair, days, consent, action)?;
                for (ts, rate) in &samples {
                    upsert(
                        conn,
                        &Sample {
                            pair: &pair,
                            timestamp: ts,
                            rate,
                            source: &source,
                            fetched_at: &fetched,
                            granularity: Some(granularity),
                            method: Some("market_chart"),
                            ..Default::default()
                        },
                    )?;
                }
                invalidate(conn, &pair)?;
                if action != FetchAction::Rebuild {
                    commit(conn)?;
                }
                result.push(dict([
                    ("pair", s(pair)),
                    ("source", s(source.as_str())),
                    ("samples", Value::from(samples.len())),
                    ("days", Value::from(days)),
                    ("granularity", s(granularity)),
                    ("fetched_at", s(fetched.as_str())),
                ]));
            }
            return Ok(Value::List(result));
        }
        let needed = needed_minutes(conn, &pairs)?;
        let any_needed = needed.values().any(|v| !v.is_empty());
        let warm = action != FetchAction::EnabledRefresh
            && get(args, "warm_cache_when_idle").is_none_or(truthy);
        let mut summaries = Vec::new();
        for pair in pairs {
            let minutes = &needed[&pair];
            let mut cached = 0;
            let mut checked = 0;
            let mut missing = std::collections::BTreeSet::new();
            for minute in minutes {
                let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM rates_cache WHERE pair=? AND timestamp=? AND (granularity='minute' OR source IN ('manual','coinbase-exchange','kraken-csv','mempool')))",params![pair,minute],|r|r.get(0)).map_err(sql_error)?;
                if exists {
                    cached += 1;
                    continue;
                }
                let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM rates_checked_minutes WHERE pair=? AND timestamp=? AND source=?)",params![pair,minute,source],|r|r.get(0)).map_err(sql_error)?;
                if exists {
                    checked += 1;
                } else {
                    missing.insert(minute.clone());
                }
            }
            let windows = if source == "coinbase-exchange" {
                windows(&missing, time::now())
            } else {
                Vec::new()
            };
            let mut inserted = 0;
            let mut newly_checked = 0;
            let mut mode = "transaction_need";
            if !minutes.is_empty() {
                if source == "mempool" {
                    for minute in &missing {
                        let (ts, rate, at) =
                            self.mempool(conn, &pair, Some(minute), consent, action)?;
                        upsert(
                            conn,
                            &Sample {
                                pair: &pair,
                                timestamp: &ts,
                                rate: &rate,
                                source: &source,
                                fetched_at: &at,
                                granularity: Some("daily"),
                                method: Some("historical_price"),
                                ..Default::default()
                            },
                        )?;
                        inserted += 1;
                    }
                    if inserted > 0 {
                        invalidate(conn, &pair)?
                    }
                } else {
                    for (start, end) in &windows {
                        let samples = self.coinbase(conn, &pair, *start, *end, consent, action)?;
                        for candle in &samples {
                            insert_candle(conn, &pair, candle, &fetched)?;
                            inserted += 1;
                        }
                        begin_write(conn)?;
                        let mut minute = start + 60_000_000;
                        while minute <= *end {
                            conn.execute("INSERT INTO rates_checked_minutes(pair,timestamp,source,checked_at,granularity,method) VALUES(?,?,?,?,'minute','product_candles') ON CONFLICT(pair,timestamp,source) DO UPDATE SET checked_at=excluded.checked_at,granularity=excluded.granularity,method=excluded.method",params![pair,time::format(minute).unwrap(),source,fetched]).map_err(sql_error)?;
                            newly_checked += 1;
                            minute += 60_000_000;
                        }
                        if !samples.is_empty() {
                            invalidate(conn, &pair)?
                        }
                        if action != FetchAction::Rebuild {
                            commit(conn)?;
                        }
                    }
                }
            } else if !any_needed && warm {
                if source == "mempool" {
                    mode = "latest_quote";
                    let (ts, rate, at) = self.mempool(conn, &pair, None, consent, action)?;
                    upsert(
                        conn,
                        &Sample {
                            pair: &pair,
                            timestamp: &ts,
                            rate: &rate,
                            source: &source,
                            fetched_at: &at,
                            granularity: Some("latest"),
                            method: Some("mempool_prices"),
                            ..Default::default()
                        },
                    )?;
                    inserted = 1;
                } else {
                    mode = "continuous_days";
                    let end = time::now() / 60_000_000 * 60_000_000;
                    let mut cursor = end
                        - days
                            .checked_mul(86_400_000_000)
                            .ok_or_else(|| validation("--days is too large"))?;
                    let mut samples = Vec::new();
                    while cursor < end {
                        let chunk = (cursor + 18_000_000_000).min(end);
                        samples.extend(self.coinbase(conn, &pair, cursor, chunk, consent, action)?);
                        cursor = chunk;
                    }
                    samples.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
                    samples.dedup_by(|a, b| a.timestamp == b.timestamp);
                    for candle in &samples {
                        insert_candle(conn, &pair, candle, &fetched)?;
                        inserted += 1;
                    }
                    if inserted > 0 {
                        invalidate(conn, &pair)?
                    }
                }
            } else if !any_needed {
                mode = "idle_no_missing_minutes";
            }
            let (granularity, method) = if source == "coinbase-exchange" {
                ("minute", "product_candles")
            } else if minutes.is_empty() {
                ("latest", "mempool_prices")
            } else {
                ("daily", "historical_price")
            };
            if action != FetchAction::Rebuild {
                commit(conn)?;
            }
            summaries.push(dict([
                ("pair", s(pair)),
                ("source", s(source.as_str())),
                ("samples", Value::from(inserted)),
                ("days", Value::from(days)),
                ("granularity", s(granularity)),
                ("method", s(method)),
                ("mode", s(mode)),
                ("needed_minutes", Value::from(minutes.len())),
                ("cached_minutes", Value::from(cached)),
                ("already_checked_minutes", Value::from(checked)),
                ("missing_minutes", Value::from(missing.len())),
                ("windows", Value::from(windows.len())),
                ("checked_minutes", Value::from(newly_checked)),
                ("fetched_at", s(fetched.as_str())),
            ]));
        }
        Ok(Value::List(summaries))
    }
}

// Match Python's per-host concurrency bound. No client construction or slot
// acquisition can open a connection; transport still consumes a guarded route.
type HostCounts = std::collections::BTreeMap<String, usize>;
static HOSTS: std::sync::OnceLock<(std::sync::Mutex<HostCounts>, std::sync::Condvar)> =
    std::sync::OnceLock::new();
struct HostPermit(String);
impl HostPermit {
    fn acquire(host: &str) -> Self {
        let key = host.trim_end_matches('.').to_lowercase();
        let (lock, changed) = HOSTS.get_or_init(Default::default);
        let mut counts = lock.lock().unwrap();
        while counts.get(&key).copied().unwrap_or(0) >= 8 {
            counts = changed.wait(counts).unwrap();
        }
        *counts.entry(key.clone()).or_default() += 1;
        Self(key)
    }
}
impl Drop for HostPermit {
    fn drop(&mut self) {
        let (lock, changed) = HOSTS.get().unwrap();
        let mut counts = lock.lock().unwrap();
        *counts.get_mut(&self.0).unwrap() -= 1;
        changed.notify_one();
    }
}
