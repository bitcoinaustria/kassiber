//! Streaming local attribution-file validation (`preview_dataset`).
//! Nothing is stored; matches remain sourced claims without accounting authority.
use super::*;
use kassiber_pyjson::{Key, PyStr};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::BufRead;

const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_ROWS: u64 = 50_000_000;
const BATCH_SIZE: u64 = 2000;
const MAX_SCRIPT_BYTES: usize = 10_000;
/// The reference iterates a `set` here, so its order follows `PYTHONHASHSEED`.
/// Native output uses this sorted order; see the recorded hash-seed variants.
pub(crate) const MANIFEST_REQUIRED: [&str; 9] = [
    "attribution_method",
    "chain",
    "dataset_key",
    "license",
    "name",
    "network",
    "source",
    "version",
    "visibility",
];
const MANIFEST_OPTIONAL: [&str; 6] = [
    "source_url",
    "observed_at",
    "valid_from",
    "valid_until",
    "category",
    "expected_active_id",
];
const CATEGORIES: [&str; 6] = ["exchange", "merchant", "mixer", "service", "self", "other"];
const CATEGORY_MAP: [(&str, &str); 9] = [
    ("mining", "service"),
    ("payment", "service"),
    ("gambling", "service"),
    ("darknet", "service"),
    ("scam", "other"),
    ("p2p", "service"),
    ("historical", "service"),
    ("historic", "service"),
    ("unknown", "other"),
];

fn error(message: impl Into<String>, code: &str, details: Option<Value>) -> AppError {
    let error = AppError::new(code, message);
    match details {
        Some(details) => error.with_details(details),
        None => error,
    }
}
fn validation(message: impl Into<String>) -> AppError {
    error(message, "validation", None)
}
fn line_details(line: u64) -> Option<Value> {
    Some(dict([("line", Value::Int(line.into()))]))
}

/// One bounded `readline(size)` from a local source.
pub trait LineSource {
    fn line(&mut self, size: usize) -> Result<Vec<u8>>;
}
impl LineSource for runtime::Source {
    fn line(&mut self, size: usize) -> Result<Vec<u8>> {
        self.read_line(size)
    }
}
/// A caller-opened file. Read failures surface as the CLI's open failure.
pub struct Stream(pub Box<dyn BufRead + Send>);
impl LineSource for Stream {
    fn line(&mut self, size: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while out.len() < size {
            let buffer = self.0.fill_buf().map_err(|_| unreadable())?;
            if buffer.is_empty() {
                break;
            }
            let want = (size - out.len()).min(buffer.len());
            let end = buffer[..want]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(want, |i| i + 1);
            out.extend_from_slice(&buffer[..end]);
            self.0.consume(end);
            if out.last() == Some(&b'\n') {
                break;
            }
        }
        Ok(out)
    }
}
pub fn unreadable() -> AppError {
    AppError::new(
        "chain_analysis_source_unavailable",
        "Cannot read selected dataset file",
    )
}

/// `_Lines`: bound each record and the whole input before decoding.
struct Lines<'a> {
    source: &'a mut dyn LineSource,
    byte_count: u64,
    record_bytes: usize,
    line_number: u64,
    sha256: Sha256,
    cancelled: &'a dyn Fn() -> bool,
}
impl Lines<'_> {
    fn next(&mut self) -> Result<Option<String>> {
        if (self.cancelled)() {
            return Err(error("Dataset import cancelled", "cancelled", None));
        }
        let raw = self.source.line(MAX_RECORD_BYTES + 1)?;
        if raw.is_empty() {
            return Ok(None);
        }
        self.byte_count += raw.len() as u64;
        self.record_bytes += raw.len();
        self.line_number += 1;
        if self.byte_count > MAX_BYTES || self.record_bytes > MAX_RECORD_BYTES {
            return Err(error(
                "Dataset byte or record limit exceeded",
                "chain_analysis_dataset_limit",
                line_details(self.line_number),
            ));
        }
        self.sha256.update(&raw);
        let body = if self.line_number == 1 {
            raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&raw)
        } else {
            &raw
        };
        String::from_utf8(body.to_vec()).map(Some).map_err(|_| {
            validation("Dataset must be UTF-8")
                .with_details(dict([("line", Value::Int(self.line_number.into()))]))
        })
    }
}

/// CPython 3.11 `_csv` reader states for the strict excel dialect.
#[derive(Clone, Copy, PartialEq)]
enum State {
    StartRecord,
    StartField,
    InField,
    InQuoted,
    QuoteInQuoted,
    EatNewline,
}
struct Csv {
    state: State,
    fields: Vec<String>,
    field: String,
}
/// `csv.Error`: reported with the reader's current line.
struct Malformed;
enum Failure {
    App(AppError),
    Csv(Malformed),
}
impl From<AppError> for Failure {
    fn from(e: AppError) -> Self {
        Failure::App(e)
    }
}
impl Csv {
    fn save(&mut self) {
        self.fields.push(std::mem::take(&mut self.field));
    }
    fn process(&mut self, c: Option<char>) -> std::result::Result<(), Malformed> {
        let newline = matches!(c, Some('\n' | '\r'));
        let ending = if c.is_none() {
            State::StartRecord
        } else {
            State::EatNewline
        };
        if self.state == State::StartRecord {
            if c.is_none() {
                return Ok(());
            }
            if newline {
                self.state = State::EatNewline;
                return Ok(());
            }
            self.state = State::StartField;
        }
        match self.state {
            State::StartField => match c {
                None | Some('\n' | '\r') => {
                    self.save();
                    self.state = ending;
                }
                Some('"') => self.state = State::InQuoted,
                Some(',') => self.save(),
                Some(c) => {
                    self.field.push(c);
                    self.state = State::InField;
                }
            },
            State::InField => match c {
                None | Some('\n' | '\r') => {
                    self.save();
                    self.state = ending;
                }
                Some(',') => {
                    self.save();
                    self.state = State::StartField;
                }
                Some(c) => self.field.push(c),
            },
            State::InQuoted => match c {
                None => {}
                Some('"') => self.state = State::QuoteInQuoted,
                Some(c) => self.field.push(c),
            },
            State::QuoteInQuoted => {
                if c == Some('"') {
                    self.field.push('"');
                    self.state = State::InQuoted;
                } else if c == Some(',') {
                    self.save();
                    self.state = State::StartField;
                } else if newline || c.is_none() {
                    self.save();
                    self.state = ending;
                } else {
                    return Err(Malformed);
                }
            }
            State::EatNewline => {
                if c.is_none() {
                    self.state = State::StartRecord;
                } else if !newline {
                    return Err(Malformed);
                }
            }
            State::StartRecord => unreachable!(),
        }
        Ok(())
    }
    /// `next(reader)`: `None` at the end of input.
    fn row(&mut self, lines: &mut Lines<'_>) -> std::result::Result<Option<Vec<String>>, Failure> {
        self.fields.clear();
        self.field.clear();
        self.state = State::StartRecord;
        loop {
            let Some(line) = lines.next()? else {
                if !self.field.is_empty() || self.state == State::InQuoted {
                    return Err(Failure::Csv(Malformed));
                }
                return Ok(None);
            };
            for c in line.chars() {
                self.process(Some(c)).map_err(Failure::Csv)?;
            }
            self.process(None).map_err(Failure::Csv)?;
            if self.state == State::StartRecord {
                return Ok(Some(std::mem::take(&mut self.fields)));
            }
        }
    }
}

type Row = Vec<(PyStr, Value)>;

fn lower_key(key: &PyStr) -> PyStr {
    let stripped = kassiber_pytext::strip_py(key);
    match stripped.as_str() {
        Some(text) => PyStr::from(kassiber_pytext::lower(text)),
        None => PyStr::from_code_points(stripped.code_points().map(|p| {
            char::from_u32(p)
                .map(|c| {
                    let mut lowered = c.to_lowercase();
                    match (lowered.next(), lowered.next()) {
                        (Some(one), None) => u32::from(one),
                        _ => p,
                    }
                })
                .unwrap_or(p)
        }))
        .unwrap_or_default(),
    }
}

/// `_records`: one bounded row at a time; rows are yielded to `each`.
fn records(
    lines: &mut Lines<'_>,
    format: &str,
    each: &mut dyn FnMut(Row, u64) -> Result<()>,
) -> Result<()> {
    if format == "csv" {
        let mut csv = Csv {
            state: State::StartRecord,
            fields: Vec::new(),
            field: String::new(),
        };
        let malformed = |line: u64| {
            validation("Malformed dataset CSV")
                .with_details(dict([("line", Value::Int(line.into()))]))
        };
        let header = match csv.row(lines) {
            Ok(v) => v,
            Err(Failure::App(e)) => return Err(e),
            Err(Failure::Csv(_)) => return Err(malformed(lines.line_number)),
        };
        let bounded = |h: &Vec<String>| {
            !h.is_empty()
                && h.len() <= 64
                && h.iter()
                    .all(|k| !kassiber_pytext::strip(k).is_empty() && k.chars().count() <= 64)
        };
        let Some(header) = header.filter(bounded) else {
            return Err(validation("Dataset CSV requires a bounded header"));
        };
        let headers: Vec<String> = header
            .iter()
            .map(|k| kassiber_pytext::lower(kassiber_pytext::strip(k)))
            .collect();
        if headers.iter().collect::<BTreeSet<_>>().len() != headers.len() {
            return Err(validation("Dataset CSV contains duplicate column names"));
        }
        lines.record_bytes = 0;
        loop {
            let row = match csv.row(lines) {
                Ok(Some(row)) if row.is_empty() => continue,
                Ok(Some(row)) => row,
                Ok(None) => return Ok(()),
                Err(Failure::App(e)) => return Err(e),
                Err(Failure::Csv(_)) => return Err(malformed(lines.line_number)),
            };
            if row.len() != headers.len() {
                return Err(validation("Dataset CSV row does not match its header")
                    .with_details(dict([("line", Value::Int(lines.line_number.into()))])));
            }
            each(
                headers
                    .iter()
                    .zip(row)
                    .map(|(k, v)| (PyStr::from(k.as_str()), s(v)))
                    .collect(),
                lines.byte_count,
            )?;
            lines.record_bytes = 0;
        }
    }
    while let Some(line) = lines.next()? {
        if kassiber_pytext::strip(&line).is_empty() {
            lines.record_bytes = 0;
            continue;
        }
        let at = lines.line_number;
        let row = kassiber_pytext::loads(&line).map_err(|_| {
            validation("Malformed dataset JSONL")
                .with_details(dict([("line", Value::Int(at.into()))]))
        })?;
        let bounded = match &row {
            Value::Dict(items) => {
                items.len() <= 64
                    && items
                        .iter()
                        .all(|(k, _)| matches!(k, Key::Str(k) if k.code_points().count() <= 64))
            }
            _ => false,
        };
        if !bounded {
            return Err(
                validation("Dataset JSONL requires one bounded object per line")
                    .with_details(dict([("line", Value::Int(at.into()))])),
            );
        }
        let Value::Dict(items) = row else {
            unreachable!()
        };
        let count = items.len();
        let mut lowered: Row = Vec::new();
        for (key, value) in items {
            let Key::Str(key) = key else { unreachable!() };
            let key = lower_key(&key);
            match lowered.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = value,
                None => lowered.push((key, value)),
            }
        }
        if lowered.len() != count {
            return Err(validation("Dataset JSONL contains duplicate column names")
                .with_details(dict([("line", Value::Int(at.into()))])));
        }
        each(lowered, lines.byte_count)?;
        lines.record_bytes = 0;
    }
    Ok(())
}

fn row_get<'a>(row: &'a Row, key: &str) -> Option<&'a Value> {
    row.iter().find(|(k, _)| k.is(key)).map(|(_, v)| v)
}
/// `value not in (None, "")`.
fn present(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null)) && v != Some(&s(""))
}

/// `_text`: nonempty bounded text without control characters, stripped.
fn text_field(value: Option<&Value>, name: &str, limit: usize) -> Result<PyStr> {
    let refused = || {
        validation(format!(
            "{name} must be nonempty text up to {limit} characters"
        ))
    };
    let Some(Value::Str(v)) = value else {
        return Err(refused());
    };
    let stripped = kassiber_pytext::strip_py(v);
    if stripped.code_points().next().is_none()
        || v.code_points().count() > limit
        || v.code_points().any(|p| p < 32)
    {
        return Err(refused());
    }
    Ok(stripped)
}

/// `_date`: an ISO date or timestamp, canonical UTC seconds.
fn date(value: Option<&Value>, name: &str) -> Result<Value> {
    if !present(value) {
        return Ok(Value::Null);
    }
    text_field(value, name, 40)?;
    let value = value.unwrap();
    if matches!(value, Value::Str(v) if v.has_surrogates()) {
        return Err(validation(format!(
            "{name} must be an ISO date or timestamp"
        )));
    }
    crate::publication::insert::parse_timestamp(value)
        .map(s)
        .map_err(|_| validation(format!("{name} must be an ISO date or timestamp")))
}

fn window(start: &Value, end: &Value) -> Result<()> {
    if let (Value::Str(a), Value::Str(b)) = (start, end) {
        if a.as_str() >= b.as_str() {
            return Err(validation("valid_until must be later than valid_from"));
        }
    }
    Ok(())
}

/// Code points whose NFKC form introduces a URL delimiter (Unicode 14.0).
const NFKC_DELIMITERS: [u32; 19] = [
    0x2047, 0x2048, 0x2049, 0x2100, 0x2101, 0x2105, 0x2106, 0x2a74, 0xfe13, 0xfe16, 0xfe55, 0xfe56,
    0xfe5f, 0xfe6b, 0xff03, 0xff0f, 0xff1a, 0xff1f, 0xff20,
];

/// `ipaddress.ip_address` for a bracketed host: IPv6 only.
fn bracketed_ok(host: &[u32]) -> bool {
    let text: Option<String> = host.iter().map(|p| char::from_u32(*p)).collect();
    let Some(text) = text else { return false };
    if let Some(rest) = text.strip_prefix('v') {
        let Some((version, tail)) = rest.split_once('.') else {
            return false;
        };
        return !version.is_empty()
            && version.bytes().all(|b| b.is_ascii_hexdigit())
            && !tail.is_empty();
    }
    let (address, scope) = match text.split_once('%') {
        Some((address, scope)) => (address, Some(scope)),
        None => (text.as_str(), None),
    };
    if scope.is_some_and(|scope| scope.is_empty() || scope.contains('%')) {
        return false;
    }
    address.parse::<std::net::Ipv6Addr>().is_ok()
}

/// `urlsplit` checks for a public HTTP(S) reference.
fn url_ok(value: &PyStr) -> bool {
    let mut url: Vec<u32> = value.code_points().collect();
    let start = url.iter().position(|p| *p > 0x20).unwrap_or(url.len());
    url.drain(..start);
    url.retain(|p| ![0x09, 0x0a, 0x0d].contains(p));
    let ascii_alpha = |p: u32| p < 128 && (p as u8).is_ascii_alphabetic();
    let mut scheme = String::new();
    if let Some(i) = url.iter().position(|p| *p == u32::from(b':')) {
        if i > 0
            && ascii_alpha(url[0])
            && url[..i].iter().all(|p| {
                *p < 128 && ((*p as u8).is_ascii_alphanumeric() || b"+-.".contains(&(*p as u8)))
            })
        {
            scheme = url[..i]
                .iter()
                .map(|p| (*p as u8 as char).to_ascii_lowercase())
                .collect();
            url.drain(..=i);
        }
    }
    let mut netloc: Vec<u32> = Vec::new();
    if url.starts_with(&[u32::from(b'/'), u32::from(b'/')]) {
        let end = url[2..]
            .iter()
            .position(|p| b"/?#".iter().any(|d| u32::from(*d) == *p))
            .map_or(url.len(), |i| i + 2);
        netloc = url[2..end].to_vec();
        url.drain(..end);
        let open = netloc.iter().position(|p| *p == u32::from(b'['));
        let close = netloc.contains(&u32::from(b']'));
        if open.is_some() != close {
            return false;
        }
        if let Some(open) = open {
            let rest = &netloc[open + 1..];
            let end = rest
                .iter()
                .position(|p| *p == u32::from(b']'))
                .unwrap_or(rest.len());
            if !bracketed_ok(&rest[..end]) {
                return false;
            }
        }
    }
    let (mut query, mut fragment) = (false, false);
    if let Some(i) = url.iter().position(|p| *p == u32::from(b'#')) {
        fragment = i + 1 < url.len();
        url.truncate(i);
    }
    if let Some(i) = url.iter().position(|p| *p == u32::from(b'?')) {
        query = i + 1 < url.len();
    }
    if netloc.iter().any(|p| *p >= 128) && netloc.iter().any(|p| NFKC_DELIMITERS.contains(p)) {
        return false;
    }
    let at = netloc.iter().rposition(|p| *p == u32::from(b'@'));
    let (userinfo, hostinfo) = match at {
        Some(at) => (Some(&netloc[..at]), &netloc[at + 1..]),
        None => (None, &netloc[..]),
    };
    let (username, password) = match userinfo {
        Some(info) => match info.iter().position(|p| *p == u32::from(b':')) {
            Some(i) => (!info[..i].is_empty(), i + 1 < info.len()),
            None => (!info.is_empty(), false),
        },
        None => (false, false),
    };
    let host = match hostinfo.iter().position(|p| *p == u32::from(b'[')) {
        Some(open) => {
            let rest = &hostinfo[open + 1..];
            !rest.is_empty() && rest[0] != u32::from(b']')
        }
        None => hostinfo
            .iter()
            .position(|p| *p == u32::from(b':'))
            .map_or(!hostinfo.is_empty(), |i| i > 0),
    };
    ["http", "https"].contains(&scheme.as_str())
        && host
        && !username
        && !password
        && !query
        && !fragment
}

/// `_url`.
fn url(value: Option<&Value>) -> Result<Value> {
    if !present(value) {
        return Ok(Value::Null);
    }
    let text = text_field(value, "source_url", 1024)?;
    if !url_ok(&text) {
        return Err(validation(
            "source_url must be a public HTTP(S) reference without credentials, query or fragment",
        ));
    }
    Ok(Value::Str(text))
}

fn domain(chain: &Value, network: &Value) -> Result<(String, String)> {
    let list = engine::datasets::canonical_subject(chain, network, &s("0".repeat(64)))?;
    let parts = super::list(&list);
    Ok((
        parts[0].as_str().unwrap().to_owned(),
        parts[1].as_str().unwrap().to_owned(),
    ))
}

/// `normalize_manifest`.
pub(crate) fn normalize_manifest(manifest: &Value) -> Result<Value> {
    let Value::Dict(items) = manifest else {
        return Err(validation(
            "Unsupported or missing attribution manifest fields",
        ));
    };
    let has = |k: &str| {
        items
            .iter()
            .any(|(key, _)| matches!(key, Key::Str(v) if v.is(k)))
    };
    if MANIFEST_REQUIRED.iter().any(|k| !has(k))
        || items.iter().any(|(key, _)| {
            !matches!(key, Key::Str(v) if v.as_str().is_some_and(|k| MANIFEST_REQUIRED.contains(&k) || MANIFEST_OPTIONAL.contains(&k)))
        })
    {
        return Err(validation("Unsupported or missing attribution manifest fields"));
    }
    let (chain, network) = domain(field(manifest, "chain"), field(manifest, "network"))?;
    let mut result = Vec::new();
    for name in MANIFEST_REQUIRED {
        let limit = if ["source", "license"].contains(&name) {
            512
        } else {
            256
        };
        result.push((
            name,
            Value::Str(text_field(get(manifest, name), name, limit)?),
        ));
    }
    let mut result = dict(result);
    set(&mut result, "chain", s(chain));
    set(&mut result, "network", s(network));
    let key = text(&result, "dataset_key");
    let mut chars = key.chars();
    let key_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && key.chars().count() <= 128
        && chars.all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !key_ok {
        return Err(validation(
            "dataset_key must be a stable identifier using letters, digits, dot, underscore or hyphen",
        ));
    }
    if !["public", "private"].contains(&text(&result, "visibility")) {
        return Err(validation("Dataset visibility must be public or private"));
    }
    set(&mut result, "source_url", url(get(manifest, "source_url"))?);
    for name in ["observed_at", "valid_from", "valid_until"] {
        set(&mut result, name, date(get(manifest, name), name)?);
    }
    window(field(&result, "valid_from"), field(&result, "valid_until"))?;
    let category = get(manifest, "category")
        .cloned()
        .unwrap_or_else(|| s("unknown"));
    set(
        &mut result,
        "category",
        Value::Str(text_field(Some(&category), "category", 64)?),
    );
    let active = match get(manifest, "expected_active_id") {
        None | Some(Value::Null) => Value::Null,
        value => Value::Str(text_field(value, "expected_active_id", 64)?),
    };
    set(&mut result, "expected_active_id", active);
    Ok(result)
}

/// `_claim`.
fn claim(row: &Row, manifest: &Value, adapter: &str, number: u64) -> Result<Value> {
    let first = |keys: &[&str]| {
        keys.iter()
            .map(|k| row_get(row, k))
            .find(|v| present(*v))
            .flatten()
            .cloned()
    };
    let subject = first(&["subject", "address", "hashadd"]);
    let mut category = first(&["category", "original_category"])
        .filter(truthy)
        .unwrap_or_else(|| field(manifest, "category").clone());
    let mut label = first(&["label", "entity", "name"]);
    if adapter == "maru92" {
        let fields: Vec<&str> = ["exchange", "gambling", "mining", "service", "historic"]
            .into_iter()
            .filter(|k| present(row_get(row, k)))
            .collect();
        if fields.len() != 1 {
            return Err(
                validation("Maru92 rows require exactly one entity category column")
                    .with_details(dict([("record_number", Value::Int(number.into()))])),
            );
        }
        category = s(fields[0]);
        label = row_get(row, fields[0]).cloned();
    }
    if present(row_get(row, "chain")) || present(row_get(row, "network")) {
        let chain = row_get(row, "chain").unwrap_or(field(manifest, "chain"));
        let network = row_get(row, "network").unwrap_or(field(manifest, "network"));
        if domain(chain, network)?
            != (
                text(manifest, "chain").to_owned(),
                text(manifest, "network").to_owned(),
            )
        {
            return Err(
                validation("Dataset row belongs to a different chain or network")
                    .with_details(dict([("record_number", Value::Int(number.into()))])),
            );
        }
    }
    let subject = subject.unwrap_or(Value::Null);
    let key = engine::datasets::canonical_subject(
        field(manifest, "chain"),
        field(manifest, "network"),
        &subject,
    )?;
    let key = super::list(&key)[2].clone();
    let original_category = text_field(Some(&category), "category", 64)?;
    let lowered = kassiber_pytext::lower(original_category.as_str().unwrap_or(""));
    let mapped = if CATEGORIES.contains(&lowered.as_str()) {
        lowered
    } else {
        CATEGORY_MAP
            .iter()
            .find(|(k, _)| *k == lowered)
            .map_or("other", |(_, v)| v)
            .to_owned()
    };
    let mut source = dict([
        ("record_number", Value::Int(number.into())),
        (
            "original_subject",
            Value::Str(text_field(
                Some(&subject),
                "subject",
                MAX_SCRIPT_BYTES * 2 + 64,
            )?),
        ),
        ("original_category", Value::Str(original_category)),
    ]);
    for (name, limit) in [
        ("source", 512),
        ("source_id", 256),
        ("attribution_method", 256),
    ] {
        if present(row_get(row, name)) {
            set(
                &mut source,
                name,
                Value::Str(text_field(row_get(row, name), name, limit)?),
            );
        }
    }
    if present(row_get(row, "source_url")) {
        set(&mut source, "source_url", url(row_get(row, "source_url"))?);
    }
    for name in ["observed_at", "date_first_tx"] {
        if present(row_get(row, name)) {
            set(&mut source, name, date(row_get(row, name), name)?);
        }
    }
    let start = date(row_get(row, "valid_from"), "valid_from")?;
    let end = date(row_get(row, "valid_until"), "valid_until")?;
    window(&start, &end)?;
    let label = Value::Str(text_field(label.as_ref(), "label", 256)?);
    Ok(dict([
        ("record_number", Value::Int(number.into())),
        ("subject", key),
        ("label", label),
        ("category", s(mapped)),
        ("source_record", source),
        ("valid_from", start),
        ("valid_until", end),
    ]))
}

/// `preview_dataset`: validate a whole local stream without storing it.
pub(crate) fn preview(
    manifest: &Value,
    source: &mut dyn LineSource,
    format: &Value,
    adapter: &Value,
    progress: &mut dyn FnMut(Value),
    cancelled: &dyn Fn() -> bool,
) -> Result<Value> {
    let manifest = normalize_manifest(manifest)?;
    let format_text = format.as_str().filter(|f| ["csv", "jsonl"].contains(f));
    let adapter_text = adapter
        .as_str()
        .filter(|a| ["generic", "am_i_exposed", "maru92"].contains(a));
    let (Some(format_text), Some(adapter_text)) = (format_text, adapter_text) else {
        return Err(validation(
            "Dataset format must be csv/jsonl and adapter generic/am_i_exposed/maru92",
        ));
    };
    let mut lines = Lines {
        source,
        byte_count: 0,
        record_bytes: 0,
        line_number: 0,
        sha256: Sha256::new(),
        cancelled,
    };
    let (mut sample, mut count) = (Vec::new(), 0u64);
    let mut each = |row: Row, lines_bytes: u64| -> Result<()> {
        count += 1;
        if count > MAX_ROWS {
            return Err(error(
                "Dataset record limit exceeded",
                "chain_analysis_dataset_limit",
                None,
            ));
        }
        let claim = claim(&row, &manifest, adapter_text, count).map_err(|e| {
            if e.details.is_none() {
                e.with_details(dict([("record_number", Value::Int(count.into()))]))
            } else {
                e
            }
        })?;
        if sample.len() < 10 {
            sample.push(claim);
        }
        if count % BATCH_SIZE == 0 {
            progress(dict([
                ("row_count", Value::Int(count.into())),
                ("byte_count", Value::Int(lines_bytes.into())),
                ("status", s("validating")),
            ]));
        }
        Ok(())
    };
    records(&mut lines, format_text, &mut each)?;
    if count == 0 {
        return Err(validation("Dataset contains no attribution claims"));
    }
    let digest = lines.sha256.clone().finalize();
    Ok(dict([
        ("manifest", manifest.clone()),
        ("format", s(format_text)),
        ("adapter", s(adapter_text)),
        (
            "sha256",
            s(digest
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()),
        ),
        ("byte_count", Value::Int(lines.byte_count.into())),
        ("row_count", Value::Int(count.into())),
        ("sample_claims", Value::List(sample)),
        ("validation", s("complete")),
        ("accounting_authority", Value::Bool(false)),
    ]))
}
