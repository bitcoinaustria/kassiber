//! `ui.logs.snapshot` and `ui.egress.snapshot`: bounded, redacted RAM rings.
//!
//! Nothing here touches disk. Log records pass the shared secret floor
//! (`kassiber_envelope::redaction`) at insert, as `kassiber/log_ring.py`
//! does; egress rows come from the process-wide `kassiber_net` ledger and
//! carry only host, port, subsystem, operation and byte counts.
use std::collections::{BTreeMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use kassiber_envelope::redaction::{is_sensitive_key, redact_secret_text};
use kassiber_envelope::{dict, get, py_str, s, set, type_name, AppError};
use kassiber_pyjson::{BigInt, Key, Value};

pub const KINDS: &[&str] = &["ui.logs.snapshot", "ui.egress.snapshot"];

const MAX_RECORDS: usize = 5000;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_MODULE: &str = "kassiber.daemon.request";

/// UTC civil time for a Unix second count.
fn civil(seconds: i64) -> (i64, u32, u32, i64) {
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rem)
}
fn stamp(time: SystemTime, millis: bool) -> String {
    let elapsed = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (year, month, day, rem) = civil(elapsed.as_secs() as i64);
    let base = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    );
    if millis {
        format!("{base}.{:03}Z", elapsed.subsec_millis())
    } else {
        format!("{base}Z")
    }
}
/// `time_utils.now_iso()`: whole seconds with a `Z` suffix.
pub(crate) fn now_iso() -> String {
    stamp(SystemTime::now(), false)
}

/// `LogRing` from `kassiber/log_ring.py`.
pub(crate) struct LogRing {
    records: VecDeque<(Value, usize)>,
    next_id: u64,
    evicted_through: u64,
    buffer_bytes: usize,
    created_at: String,
}

impl Default for LogRing {
    fn default() -> Self {
        LogRing {
            records: VecDeque::new(),
            next_id: 1,
            evicted_through: 0,
            buffer_bytes: 0,
            created_at: now_iso(),
        }
    }
}

/// `_coerce_field(value, field_name)` for the typed `{"type","value"}` shape.
fn coerce_field(name: &str, field: Value) -> Value {
    if is_sensitive_key(name) {
        return dict([("type", s("text")), ("value", s("[redacted]"))]);
    }
    let kind = get(&field, "type").map(py_str).unwrap_or_default();
    let value = match get(&field, "value").cloned().unwrap_or(Value::Null) {
        Value::Str(text) => s(redact_secret_text(&py_str(&Value::Str(text)))),
        other @ (Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null) => other,
        other => s(redact_secret_text(&py_str(&other))),
    };
    dict([("type", s(kind)), ("value", value)])
}

impl LogRing {
    /// `LogRing.append`. `fields` are already typed; `request_id` is the
    /// registry key Python keeps in `current_request_id`.
    pub(crate) fn append(
        &mut self,
        level: &str,
        line: u32,
        msg: &str,
        fields: Vec<(&str, Value)>,
        request_id: Option<&str>,
    ) {
        let mut clean: Vec<(String, Value)> = fields
            .into_iter()
            .map(|(name, field)| (name.to_owned(), coerce_field(name, field)))
            .collect();
        if let Some(id) = request_id.filter(|id| !id.is_empty()) {
            if !clean.iter().any(|(name, _)| name == "request_id") {
                clean.push((
                    "request_id".into(),
                    dict([("type", s("text")), ("value", s(id))]),
                ));
            }
        }
        let record = dict([
            ("id", Value::Int(self.next_id.into())),
            ("ts", s(stamp(SystemTime::now(), true))),
            ("level", s(level)),
            ("module", s(REQUEST_MODULE)),
            ("file", s("crates/daemon/src/lib.rs")),
            ("line", Value::Int(line.into())),
            ("msg", s(redact_secret_text(msg))),
            ("fields", dict(clean)),
        ]);
        self.next_id += 1;
        let size = kassiber_pyjson::dumps(&record, &Default::default())
            .map(|text| text.len())
            .unwrap_or(0);
        self.records.push_back((record, size));
        self.buffer_bytes += size;
        while self.records.len() > 1
            && (self.records.len() > MAX_RECORDS || self.buffer_bytes > MAX_BYTES)
        {
            if let Some((evicted, size)) = self.records.pop_front() {
                self.buffer_bytes -= size;
                if let Some(Value::Int(id)) = get(&evicted, "id") {
                    self.evicted_through = u64::try_from(id.clone()).unwrap_or(u64::MAX);
                }
            }
        }
    }

    fn snapshot(&self, after_id: &BigInt, limit: usize) -> Value {
        let records: Vec<Value> = self
            .records
            .iter()
            .filter(|(record, _)| match get(record, "id") {
                Some(Value::Int(id)) => id > after_id,
                _ => false,
            })
            .take(limit)
            .map(|(record, _)| record.clone())
            .collect();
        dict([
            ("records", Value::List(records)),
            ("last_id", Value::Int((self.next_id - 1).into())),
            (
                "gap",
                Value::Bool(*after_id < BigInt::from(self.evicted_through)),
            ),
            ("started_at", s(self.created_at.as_str())),
            ("buffer_bytes", Value::Int(self.buffer_bytes.into())),
            ("max_bytes", Value::Int(MAX_BYTES.into())),
        ])
    }
}

/// `{"type": "text", "value": str(kind) if kind is not None else ""}`.
pub(crate) fn kind_field(kind: Option<&Value>) -> Value {
    let text = match kind {
        None | Some(Value::Null) => String::new(),
        Some(kind) => py_str(kind),
    };
    dict([("type", s("text")), ("value", s(text))])
}
pub(crate) fn duration_field(started: std::time::Instant) -> Value {
    dict([
        ("type", s("duration_ms")),
        (
            "value",
            Value::Int((started.elapsed().as_millis() as u64).into()),
        ),
    ])
}
/// `_request_id_registry_key(request_id)`.
pub(crate) fn request_key(request: &Value) -> Option<String> {
    match get(request, "request_id") {
        None | Some(Value::Null) => None,
        Some(Value::Str(text)) => Some(py_str(&Value::Str(text.clone()))).filter(|t| !t.is_empty()),
        Some(other) => Some(py_str(other)),
    }
}

/// `_coerce_args_dict` and the unknown-field check of both snapshot kinds.
fn snapshot_args(kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let args = match args {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(args @ Value::Dict(_)) => args.clone(),
        Some(other) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(other)))])))
        }
    };
    let Value::Dict(pairs) = &args else {
        unreachable!()
    };
    let mut unknown: Vec<kassiber_pyjson::PyStr> = pairs
        .iter()
        .filter_map(|(key, _)| match key {
            Key::Str(key) if !key.is("after_id") && !key.is("limit") => Some(key.clone()),
            _ => None,
        })
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        unknown.dedup();
        return Err(
            AppError::new("validation", format!("{kind} received unsupported fields"))
                .with_details(dict([(
                    "unknown",
                    Value::List(unknown.into_iter().map(Value::Str).collect()),
                )])),
        );
    }
    Ok(args)
}

/// `_bounded_snapshot_int(args, key, request_kind, default, minimum, maximum)`.
fn bounded(
    args: &Value,
    kind: &str,
    key: &str,
    default: i64,
    minimum: i64,
    maximum: Option<i64>,
) -> Result<BigInt, AppError> {
    let value = match get(args, key) {
        None | Some(Value::Null) => return Ok(default.into()),
        Some(Value::Int(value)) => value.clone(),
        Some(other) => {
            return Err(
                AppError::new("validation", format!("{kind} {key} must be an integer"))
                    .with_details(dict([("type", s(type_name(other)))])),
            )
        }
    };
    if value < BigInt::from(minimum) || maximum.is_some_and(|max| value > BigInt::from(max)) {
        return Err(
            AppError::new("validation", format!("{kind} {key} is out of range")).with_details(
                dict([
                    ("min", Value::Int(minimum.into())),
                    (
                        "max",
                        maximum.map_or(Value::Null, |max| Value::Int(max.into())),
                    ),
                ]),
            ),
        );
    }
    Ok(value)
}

fn limit(args: &Value, kind: &str) -> Result<(BigInt, usize), AppError> {
    let after = bounded(args, kind, "after_id", 0, 0, None)?;
    let limit = bounded(args, kind, "limit", 500, 1, Some(2000))?;
    Ok((after, usize::try_from(limit).unwrap_or(500)))
}

/// `ui.logs.snapshot`.
pub(crate) fn logs(ring: &LogRing, args: Option<&Value>) -> Result<Value, AppError> {
    let args = snapshot_args("ui.logs.snapshot", args)?;
    let (after, limit) = limit(&args, "ui.logs.snapshot")?;
    Ok(ring.snapshot(&after, limit))
}

/// One `EgressAllowlistEntry`.
#[derive(Clone)]
struct Allowed {
    host: String,
    port: Option<i64>,
    subsystem: String,
    label: String,
    source: String,
    user: bool,
}
impl Allowed {
    fn payload(&self) -> Value {
        dict([
            ("host", s(self.host.as_str())),
            (
                "port",
                self.port.map_or(Value::Null, |p| Value::Int(p.into())),
            ),
            ("subsystem", s(self.subsystem.as_str())),
            ("label", s(self.label.as_str())),
            ("source", s(self.source.as_str())),
            ("user_allowlisted", Value::Bool(self.user)),
        ])
    }
    fn matches(&self, host: &str, port: Option<i64>, subsystem: &str) -> bool {
        self.host == normalize_host(host)
            && self.port.is_none_or(|p| Some(p) == port)
            && (self.subsystem == "any" || self.subsystem == subsystem)
    }
}

/// `_normalize_host(host)`.
fn normalize_host(host: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(host).trim_end_matches('.'))
}

/// `endpoint_from_url(url)` over `urllib.parse.urlsplit`: `(host, port,
/// scheme)`. An unparseable port fails like Python's `ValueError`.
pub(crate) fn endpoint(url: &str) -> Result<(String, Option<i64>, String), AppError> {
    let raw = kassiber_pytext::strip(url);
    if raw.is_empty() {
        return Ok((String::new(), None, String::new()));
    }
    let text: String = raw
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let text = if text.contains("://") {
        text
    } else {
        format!("//{text}")
    };
    let (scheme, rest) = match text.find(':') {
        Some(index)
            if index > 0
                && text.as_bytes()[0].is_ascii_alphabetic()
                && text[..index]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) =>
        {
            (text[..index].to_ascii_lowercase(), &text[index + 1..])
        }
        _ => (String::new(), text.as_str()),
    };
    let netloc = match rest.strip_prefix("//") {
        Some(after) => &after[..after.find(['/', '?', '#']).unwrap_or(after.len())],
        None => "",
    };
    if netloc.contains('[') != netloc.contains(']') {
        return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
    }
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
    let (hostname, port) = match hostinfo.split_once('[') {
        Some((_, bracketed)) => {
            let (host, tail) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            if !host.starts_with(['v', 'V'])
                && host
                    .split('%')
                    .next()
                    .unwrap_or("")
                    .parse::<std::net::Ipv6Addr>()
                    .is_err()
            {
                return Err(AppError::new("internal_error", "Invalid IPv6 URL"));
            }
            (host, tail.split_once(':').map_or("", |(_, p)| p))
        }
        None => hostinfo.split_once(':').unwrap_or((hostinfo, "")),
    };
    let port = if port.is_empty() {
        None
    } else if port.bytes().all(|b| b.is_ascii_digit()) {
        let value: BigInt = port.parse().unwrap_or_default();
        if value > BigInt::from(65_535) {
            return Err(AppError::new("internal_error", "Port out of range 0-65535"));
        }
        Some(i64::try_from(value).unwrap_or_default())
    } else {
        return Err(AppError::new(
            "internal_error",
            format!(
                "Port could not be cast to integer value as {}",
                kassiber_pytext::repr_str(port)
            ),
        ));
    };
    let hostname = match hostname.split_once('%') {
        Some((host, zone)) => format!("{}%{zone}", kassiber_pytext::lower(host)),
        None => kassiber_pytext::lower(hostname),
    };
    let default = match scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        "ssl" => Some(50002),
        "tcp" => Some(50001),
        _ => None,
    };
    Ok((
        normalize_host(&hostname),
        port.filter(|p| *p != 0).or(default),
        scheme,
    ))
}

/// `db_header_proof(path)`.
fn db_header(path: &std::path::Path) -> Value {
    match std::fs::read(path) {
        Ok(bytes) => {
            let prefix = &bytes[..bytes.len().min(16)];
            let plaintext = prefix == b"SQLite format 3\0";
            dict([
                ("exists", Value::Bool(true)),
                (
                    "classification",
                    s(if plaintext {
                        "plaintext-sqlite"
                    } else {
                        "ciphertext-like"
                    }),
                ),
                ("sqlite_plaintext_header", Value::Bool(plaintext)),
                (
                    "encrypted_like",
                    Value::Bool(!prefix.is_empty() && !plaintext),
                ),
                (
                    "prefix_hex",
                    s(prefix
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()),
                ),
            ])
        }
        Err(error) => {
            let missing = error.kind() == std::io::ErrorKind::NotFound;
            let mut proof = dict([
                ("exists", Value::Bool(false)),
                (
                    "classification",
                    s(if missing { "missing" } else { "unreadable" }),
                ),
                ("sqlite_plaintext_header", Value::Bool(false)),
                ("encrypted_like", Value::Bool(false)),
                ("prefix_hex", s("")),
            ]);
            if !missing {
                let errno = error.raw_os_error().unwrap_or(0);
                let text = error.to_string();
                let text = text
                    .strip_suffix(&format!(" (os error {errno})"))
                    .unwrap_or(&text)
                    .to_owned();
                set(
                    &mut proof,
                    "error",
                    s(format!(
                        "[Errno {errno}] {text}: {}",
                        kassiber_pytext::repr_str(&path.to_string_lossy())
                    )),
                );
            }
            proof
        }
    }
}

/// Inputs Python's `_egress_allowlist(ctx)` reads: runtime backends in
/// dict order as `(name, url, source)`, and AI providers when a book is open.
pub(crate) struct EgressInputs {
    pub backends: Vec<(String, Value, Value)>,
    pub ai_providers: Option<Vec<(String, Value, Value)>>,
}

fn allowlist(inputs: &EgressInputs) -> Result<Vec<Allowed>, AppError> {
    let mut entries = vec![
        Allowed {
            host: "api.exchange.coinbase.com".into(),
            port: Some(443),
            subsystem: "pricing".into(),
            label: "Coinbase Exchange".into(),
            source: "built-in".into(),
            user: false,
        },
        Allowed {
            host: "api.coingecko.com".into(),
            port: Some(443),
            subsystem: "pricing".into(),
            label: "CoinGecko".into(),
            source: "built-in".into(),
            user: false,
        },
    ];
    for (name, url, source) in &inputs.backends {
        // `backend_value(raw, "url")`: the stripped, non-empty text.
        let url = match url {
            Value::Null => continue,
            other => kassiber_pytext::strip(&py_str(other)).to_owned(),
        };
        if url.is_empty() {
            continue;
        }
        let (host, port, _) = endpoint(&url)?;
        if host.is_empty() {
            continue;
        }
        let source = if kassiber_envelope::truthy(source) {
            py_str(source)
        } else {
            "configured".into()
        };
        entries.push(Allowed {
            host,
            port,
            subsystem: "any".into(),
            label: format!("backend:{name}"),
            user: source != "built-in default",
            source,
        });
    }
    for (name, base_url, kind) in inputs.ai_providers.iter().flatten() {
        let url = match base_url {
            Value::Null => String::new(),
            other => py_str(other),
        };
        let (host, port, scheme) = endpoint(&url)?;
        if !matches!(scheme.as_str(), "http" | "https") || host.is_empty() {
            continue;
        }
        entries.push(Allowed {
            host,
            port,
            subsystem: "ai".into(),
            label: format!("ai:{name}"),
            source: if kassiber_envelope::truthy(kind) {
                py_str(kind)
            } else {
                "ai-provider".into()
            },
            user: true,
        });
    }
    let mut seen = std::collections::BTreeSet::new();
    entries.retain(|e| seen.insert((e.host.clone(), e.port, e.subsystem.clone(), e.label.clone())));
    Ok(entries)
}

/// Python's operation names for native ledger rows.
fn operation(record: &kassiber_net::Record) -> &'static str {
    use kassiber_net::Operation;
    match record.operation {
        Operation::Dns => "dns.resolve",
        Operation::Connect => "socket.connect",
        Operation::ProxyHandshake => "proxy.connect",
        Operation::Write if record.method.is_some() => "http.request",
        Operation::Write => "socket.write",
        Operation::Process => "process.spawn",
    }
}
fn subsystem(value: kassiber_net::Subsystem) -> &'static str {
    use kassiber_net::Subsystem;
    match value {
        Subsystem::Sync => "sync",
        Subsystem::Pricing => "pricing",
        Subsystem::Ai => "ai",
        Subsystem::Update => "update",
    }
}

/// `get_egress_ledger().snapshot(after_id=..., limit=...)` reduced to what the
/// AI-safe view counts: each record's subsystem and bytes, never its host.
pub(crate) fn ledger_counts(after: i64, limit: i64) -> Value {
    let all = kassiber_net::Ledger::process().retained();
    let records = all
        .records
        .iter()
        .filter(|record| BigInt::from(record.id) > BigInt::from(after))
        .take(usize::try_from(limit).unwrap_or(0))
        .map(|record| {
            dict([
                ("subsystem", s(subsystem(record.subsystem))),
                ("bytes_out", Value::Int(record.bytes_out.into())),
            ])
        })
        .collect();
    dict([
        ("records", Value::List(records)),
        ("last_id", Value::Int(all.last_id.into())),
        (
            "gap",
            Value::Bool(BigInt::from(after) < BigInt::from(all.evicted_through)),
        ),
    ])
}

/// `ui.egress.snapshot` over the process ledger.
pub(crate) fn egress(
    args: Option<&Value>,
    database: &std::path::Path,
    inputs: &EgressInputs,
) -> Result<Value, AppError> {
    let args = snapshot_args("ui.egress.snapshot", args)?;
    let (after, limit) = limit(&args, "ui.egress.snapshot")?;
    let complete = inputs.ai_providers.is_some();
    let entries = allowlist(inputs)?;
    let ledger = kassiber_net::Ledger::process();
    let all = ledger.retained();
    let annotate = |record: &kassiber_net::Record| {
        let subsystem = subsystem(record.subsystem);
        let port = Some(i64::from(record.port));
        let mut row = dict([
            ("id", Value::Int(record.id.into())),
            ("ts", s(stamp(record.ts, false))),
            ("subsystem", s(subsystem)),
            ("host", s(record.host.as_str())),
            ("port", Value::Int(record.port.into())),
            ("scheme", s(record.scheme)),
            ("operation", s(operation(record))),
            (
                "method",
                record
                    .method
                    .map_or(Value::Null, |m| s(m.to_ascii_uppercase())),
            ),
            ("bytes_out", Value::Int(record.bytes_out.into())),
            ("via_proxy", Value::Bool(record.via_proxy)),
        ]);
        let found = complete
            .then(|| {
                entries
                    .iter()
                    .find(|e| e.matches(&record.host, port, subsystem))
            })
            .flatten();
        let (status, label, source, user) = match (complete, found) {
            (false, _) => ("unknown", Value::Null, Value::Null, false),
            (true, Some(e)) => (
                "expected",
                s(e.label.as_str()),
                s(e.source.as_str()),
                e.user,
            ),
            (true, None) => ("unexpected", Value::Null, Value::Null, false),
        };
        set(&mut row, "allowlist_status", s(status));
        set(&mut row, "allowlist_label", label);
        set(&mut row, "allowlist_source", source);
        set(&mut row, "user_allowlisted", Value::Bool(user));
        row
    };
    let records: Vec<Value> = all
        .records
        .iter()
        .filter(|r| BigInt::from(r.id) > after)
        .take(limit)
        .map(annotate)
        .collect();
    let mut totals: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let mut unexpected = 0;
    for record in &all.records {
        let entry = totals.entry(subsystem(record.subsystem)).or_default();
        entry.0 += 1;
        entry.1 += record.bytes_out;
        if get(&annotate(record), "allowlist_status") == Some(&s("unexpected")) {
            unexpected += 1;
        }
    }
    Ok(dict([
        ("records", Value::List(records)),
        ("last_id", Value::Int(all.last_id.into())),
        (
            "gap",
            Value::Bool(after < BigInt::from(all.evicted_through)),
        ),
        ("started_at", s(kassiber_net::Ledger::process_started_at())),
        ("buffer_bytes", Value::Int(all.buffer_bytes.into())),
        ("max_bytes", Value::Int(kassiber_net::MAX_BYTES.into())),
        ("allowlist_complete", Value::Bool(complete)),
        (
            "allowlist",
            Value::List(entries.iter().map(Allowed::payload).collect()),
        ),
        ("db_header", db_header(database)),
        (
            "summary",
            dict([
                ("total_records", Value::Int(all.records.len().into())),
                ("unexpected", Value::Int(unexpected.into())),
                (
                    "update",
                    Value::Int(totals.get("update").map_or(0, |t| t.0).into()),
                ),
                (
                    "by_subsystem",
                    dict(totals.iter().map(|(key, (records, bytes))| {
                        (
                            *key,
                            dict([
                                ("records", Value::Int((*records).into())),
                                ("bytes_out", Value::Int((*bytes).into())),
                            ]),
                        )
                    })),
                ),
            ]),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_floors_secrets_and_sensitive_fields_and_evicts_with_gap() {
        let mut ring = LogRing::default();
        ring.append(
            "debug",
            1,
            "api_key=synthetic-secret",
            vec![
                ("kind", kind_field(Some(&s("token=synthetic-secret")))),
                ("api_key", dict([("type", s("text")), ("value", s("x"))])),
            ],
            Some("7"),
        );
        let snapshot = logs(&ring, None).unwrap();
        let text = kassiber_pyjson::dumps(&snapshot, &Default::default()).unwrap();
        assert!(!text.contains("synthetic-secret"), "{text}");
        assert!(text.contains("\"request_id\": {\"type\": \"text\", \"value\": \"7\"}"));
        assert!(text.contains("\"api_key\": {\"type\": \"text\", \"value\": \"[redacted]\"}"));
        for _ in 0..MAX_RECORDS + 3 {
            ring.append("debug", 1, "m", vec![], None);
        }
        assert_eq!(ring.records.len(), MAX_RECORDS);
        let snapshot = logs(&ring, Some(&dict([("after_id", Value::Int(1.into()))]))).unwrap();
        assert_eq!(get(&snapshot, "gap"), Some(&Value::Bool(true)));
    }

    #[test]
    fn endpoint_matches_urlsplit_hosts_ports_and_errors() {
        assert_eq!(
            endpoint("https://Node.Example.:8443/x").unwrap(),
            ("node.example".into(), Some(8443), "https".into())
        );
        assert_eq!(
            endpoint("ssl://h").unwrap(),
            ("h".into(), Some(50002), "ssl".into())
        );
        assert_eq!(
            endpoint("user:pw@h:0").unwrap(),
            ("h".into(), None, String::new())
        );
        assert_eq!(
            endpoint("http://[::1]:80").unwrap(),
            ("::1".into(), Some(80), "http".into())
        );
        assert!(endpoint("http://h:99999").is_err());
        assert!(endpoint("http://h:x1").is_err());
        assert!(endpoint("http://[::1").is_err());
    }
}
