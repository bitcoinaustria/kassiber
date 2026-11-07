//! Kassiber's machine envelope, as `kassiber/envelope.py` builds it.
//!
//! Success: `{"kind", "schema_version": 1, "data"}` plus metadata keys that
//! may not shadow the reserved ones, with a `data.page` block when `data`
//! carries `next_cursor` and `has_more`. Error: `{"kind": "error",
//! "schema_version": 1, "error": {code, message, hint, details, retryable,
//! debug}}`. Events add `"event": true`. The CLI writes envelopes with
//! `json.dumps(indent=2)` and a trailing newline; the daemon writes one
//! compact line per record. Both escape non-ASCII, as Python's defaults do.
//!
//! [`derive_kind`] and [`command_path`] port the namespace-to-kind rules of
//! `envelope.py` and `cli/command_registry.py`, and [`redaction`] ports the
//! rules the daemon applies to error messages, hints, and details.

pub mod redaction;
pub mod remediation;

use std::fmt;

use kassiber_pyjson::{dumps, Key, Options, PyStr, Value};

/// The envelope's `schema_version`.
pub const SCHEMA_VERSION: i64 = 1;

/// The keys envelope metadata may not override.
pub const RESERVED_ENVELOPE_KEYS: [&str; 4] = ["kind", "schema_version", "data", "error"];

/// Python's `AppError`: a user-facing failure with its envelope fields.
#[derive(Clone, Debug, PartialEq)]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub hint: Option<Box<str>>,
    pub details: Option<Value>,
    pub retryable: bool,
    pub message_codepoints: Option<Box<PyStr>>,
    pub sqlite_origin: bool,
}

impl AppError {
    /// `AppError(message, code=code)`.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        AppError {
            code: code.into(),
            message: message.into(),
            hint: None,
            details: None,
            retryable: false,
            message_codepoints: None,
            sqlite_origin: false,
        }
    }

    /// Retain Python string code points in machine output, with stderr's
    /// backslash replacement for the UTF-8 display/diagnostic message.
    pub fn with_python_message(mut self, message: PyStr) -> Self {
        self.message = message
            .code_points()
            .map(|p| char::from_u32(p).map_or_else(|| format!("\\u{p:04x}"), |c| c.to_string()))
            .collect();
        self.message_codepoints = Some(Box::new(message));
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into().into_boxed_str());
        self
    }

    pub fn with_sqlite_origin(mut self) -> Self {
        self.sqlite_origin = true;
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// The error envelope `emit_error` writes for this error.
    pub fn envelope(&self, debug: Option<&str>) -> Value {
        let mut envelope = build_error_envelope(
            &self.code,
            &self.message,
            self.details.clone(),
            self.hint.as_deref(),
            self.retryable,
            debug,
        );
        if let Some(message) = &self.message_codepoints {
            if let Value::Dict(fields) = &mut envelope {
                if let Some((_, error)) = fields
                    .iter_mut()
                    .find(|(key, _)| *key == Key::from("error"))
                {
                    set(error, "message", Value::Str(message.as_ref().clone()));
                }
            }
        }
        envelope
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

/// Shorthand for a `str` value.
pub fn s(text: impl Into<PyStr>) -> Value {
    Value::Str(text.into())
}

/// Shorthand for a `dict` with `str` keys, in order.
pub fn dict<K: Into<PyStr>>(pairs: impl IntoIterator<Item = (K, Value)>) -> Value {
    Value::Dict(
        pairs
            .into_iter()
            .map(|(key, value)| (Key::Str(key.into()), value))
            .collect(),
    )
}

/// `dict.get(key)` for a `str` key.
pub fn get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Dict(pairs) => pairs.iter().rev().find_map(|(k, v)| match k {
            Key::Str(name) if name.as_str() == Some(key) => Some(v),
            _ => None,
        }),
        _ => None,
    }
}

/// `key in dict` for a `str` key.
pub fn contains_key(value: &Value, key: &str) -> bool {
    get(value, key).is_some()
}

/// Sets `key` in a dict, replacing its value in place or appending it, as
/// Python's `dict[key] = value` does. Does nothing to a non-dict.
pub fn set(value: &mut Value, key: &str, item: Value) {
    if let Value::Dict(pairs) = value {
        for (k, v) in pairs.iter_mut() {
            if matches!(k, Key::Str(name) if name.as_str() == Some(key)) {
                *v = item;
                return;
            }
        }
        pairs.push((Key::from(key), item));
    }
}

/// Python's `bool(value)` for the JSON-shaped values envelopes hold.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(n) => *n != kassiber_pyjson::BigInt::default(),
        Value::Float(f) => *f != 0.0,
        Value::Str(text) => text.code_points().next().is_some(),
        Value::Bytes(bytes) => !bytes.is_empty(),
        Value::Decimal(d) => !d.is_zero(),
        Value::List(items) => !items.is_empty(),
        Value::Dict(pairs) => !pairs.is_empty(),
    }
}

/// `repr(value)` for JSON-shaped values, as Python messages quote them.
pub fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => kassiber_pyjson::float_repr(*f),
        Value::Str(text) => kassiber_pytext::repr_py(text),
        Value::Bytes(bytes) => kassiber_pyjson::bytes_repr(bytes),
        Value::Decimal(d) => format!("Decimal('{d}')"),
        Value::List(items) => format!(
            "[{}]",
            items.iter().map(py_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Dict(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(key, item)| format!("{}: {}", key_repr(key), py_repr(item)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn key_repr(key: &Key) -> String {
    match key {
        Key::Null => "None".to_owned(),
        Key::Bool(true) => "True".to_owned(),
        Key::Bool(false) => "False".to_owned(),
        Key::Int(n) => n.to_string(),
        Key::Float(f) => kassiber_pyjson::float_repr(*f),
        Key::Str(text) => kassiber_pytext::repr_py(text),
    }
}

/// `str(value)`: strings as themselves (surrogates dropped), anything else
/// as its `repr`.
pub fn py_str(value: &Value) -> String {
    match value {
        Value::Str(text) => text.code_points().filter_map(char::from_u32).collect(),
        other => py_repr(other),
    }
}

/// `type(value).__name__`.
pub fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Bytes(_) => "bytes",
        Value::Decimal(_) => "Decimal",
        Value::List(_) => "list",
        Value::Dict(_) => "dict",
    }
}

/// `json_ready`: every `Decimal` becomes a float, recursively.
///
/// `float(Decimal)` is the correctly rounded value of `str(Decimal)`, which
/// Rust's float parsing reproduces.
pub fn json_ready(value: Value) -> Value {
    match value {
        Value::Decimal(d) => Value::Float(d.to_canonical().parse::<f64>().unwrap_or(f64::NAN)),
        Value::List(items) => Value::List(items.into_iter().map(json_ready).collect()),
        Value::Dict(pairs) => Value::Dict(
            pairs
                .into_iter()
                .map(|(key, item)| (key, json_ready(item)))
                .collect(),
        ),
        other => other,
    }
}

/// A programming error Python raises as `ValueError` from `build_envelope`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedKeyError(pub String);

impl fmt::Display for ReservedKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Envelope metadata cannot override reserved key '{}'",
            self.0
        )
    }
}

impl std::error::Error for ReservedKeyError {}

/// `build_envelope(kind, data, envelope_meta)`.
pub fn build_envelope(
    kind: &str,
    data: Value,
    envelope_meta: Vec<(String, Value)>,
) -> Result<Value, ReservedKeyError> {
    let mut data = json_ready(data);
    if let Value::Dict(_) = &data {
        if contains_key(&data, "next_cursor")
            && contains_key(&data, "has_more")
            && !contains_key(&data, "page")
        {
            let next_cursor = get(&data, "next_cursor").cloned().unwrap_or(Value::Null);
            let has_more = get(&data, "has_more").is_some_and(truthy);
            set(
                &mut data,
                "page",
                dict([
                    ("next_cursor", next_cursor),
                    ("has_more", Value::Bool(has_more)),
                ]),
            );
        }
    }
    let mut envelope = dict([
        ("kind", s(kind)),
        ("schema_version", Value::from(SCHEMA_VERSION)),
        ("data", data),
    ]);
    if let Some((key, _)) = envelope_meta
        .iter()
        .find(|(key, _)| RESERVED_ENVELOPE_KEYS.contains(&key.as_str()))
    {
        return Err(ReservedKeyError(key.clone()));
    }
    for (key, value) in envelope_meta {
        set(&mut envelope, &key, json_ready(value));
    }
    Ok(envelope)
}

/// `build_envelope(kind, data)` without metadata, which cannot fail.
pub fn success(kind: &str, data: Value) -> Value {
    match build_envelope(kind, data, Vec::new()) {
        Ok(envelope) => envelope,
        Err(_) => unreachable!("an envelope without metadata has no reserved key"),
    }
}

/// `build_event_envelope(kind, data)`: an unsolicited daemon record with a
/// top-level `event: true` and never a `request_id`.
pub fn build_event_envelope(kind: &str, data: Value) -> Value {
    match build_envelope(kind, data, vec![("event".to_owned(), Value::Bool(true))]) {
        Ok(envelope) => envelope,
        Err(_) => unreachable!("`event` is not a reserved key"),
    }
}

/// `build_error_envelope(code, message, details, hint, retryable, debug)`.
pub fn build_error_envelope(
    code: &str,
    message: &str,
    details: Option<Value>,
    hint: Option<&str>,
    retryable: bool,
    debug: Option<&str>,
) -> Value {
    dict([
        ("kind", s("error")),
        ("schema_version", Value::from(SCHEMA_VERSION)),
        (
            "error",
            dict([
                ("code", s(code)),
                ("message", s(message)),
                ("hint", hint.map_or(Value::Null, s)),
                ("details", details.map_or(Value::Null, json_ready)),
                ("retryable", Value::Bool(retryable)),
                ("debug", debug.map_or(Value::Null, s)),
            ]),
        ),
    ])
}

/// An envelope `json.dumps` cannot write (an int beyond 4,300 digits, or
/// nesting past 1,000 levels); Python raises and the caller reports an
/// internal error.
pub type TextError = kassiber_pyjson::Error;

/// The CLI's machine output: `json.dumps(envelope, indent=2)` and a newline.
pub fn cli_text(envelope: &Value) -> Result<String, TextError> {
    let mut text = dumps(envelope, &Options::new().indent(2))?;
    text.push('\n');
    Ok(text)
}

/// One daemon record: `json.dumps(json_ready(payload), separators=(",",
/// ":"))` and a newline.
pub fn daemon_line(record: &Value) -> Result<String, TextError> {
    let mut text = dumps(&json_ready(record.clone()), &Options::new().compact())?;
    text.push('\n');
    Ok(text)
}

/// The namespace attributes that name a nested subcommand, in the order
/// `derive_kind` joins them (`envelope.py` `_KIND_SUBCOMMAND_ATTRS`).
pub const KIND_SUBCOMMAND_ATTRS: &[&str] = &[
    "networks_command",
    "chain_analysis_command",
    "chain_analysis_action",
    "maintenance_command",
    "backends_command",
    "commands_command",
    "offline_command",
    "operator_command",
    "operator_operation_command",
    "operator_touch_id_command",
    "mcp_command",
    "context_command",
    "workspaces_command",
    "profiles_command",
    "accounts_command",
    "wallets_command",
    "transactions_command",
    "attachments_command",
    "metadata_command",
    "notes_command",
    "tags_command",
    "bip329_command",
    "review_command",
    "journals_command",
    "journal_transfers_command",
    "transfers_command",
    "transfers_components_command",
    "transfers_dismissals_command",
    "transfers_gaps_command",
    "loans_command",
    "payouts_command",
    "transfers_rules_command",
    "imports_command",
    "views_command",
    "chats_command",
    "secrets_command",
    "backup_command",
    "btcpay_command",
    "btcpay_provenance_command",
    "documents_command",
    "source_funds_command",
    "source_funds_sources_command",
    "source_funds_links_command",
    "source_funds_cases_command",
    "source_funds_recipients_command",
    "events_command",
    "quarantine_command",
    "quarantine_resolve_command",
    "records_command",
    "records_note_command",
    "history_command",
    "reports_command",
    "rates_command",
    "diagnostics_command",
    "ai_command",
    "ai_providers_command",
];

/// Subcommand attributes that extend the command path but not the kind
/// (`command_registry.py` `_COMMAND_PATH_ONLY_SUBCOMMAND_ATTRS`).
pub const COMMAND_PATH_ONLY_SUBCOMMAND_ATTRS: &[&str] = &[
    "projects_command",
    "records_tag_command",
    "records_excluded_command",
    "records_kind_command",
    "filed_snapshots_command",
];

/// `derive_kind(args, override)`. `attr` returns the namespace's value for
/// a subcommand destination (`command` included), if set and non-empty.
pub fn derive_kind<'a>(
    attr: impl Fn(&str) -> Option<&'a str>,
    kind_override: Option<&str>,
) -> String {
    if let Some(kind) = kind_override.filter(|k| !k.is_empty()) {
        return kind.to_owned();
    }
    let mut parts: Vec<&str> = Vec::new();
    if let Some(command) = attr("command").filter(|c| !c.is_empty()) {
        parts.push(command);
    }
    for name in KIND_SUBCOMMAND_ATTRS {
        if let Some(value) = attr(name).filter(|v| !v.is_empty()) {
            parts.push(value);
        }
    }
    if parts.is_empty() {
        "response".to_owned()
    } else {
        parts.join(".")
    }
}

/// `command_path(args)`: the kind plus the path-only subcommands.
pub fn command_path<'a>(attr: impl Fn(&str) -> Option<&'a str>) -> String {
    let mut path = derive_kind(&attr, None);
    for name in COMMAND_PATH_ONLY_SUBCOMMAND_ATTRS {
        if let Some(value) = attr(name).filter(|v| !v.is_empty()) {
            path.push('.');
            path.push_str(value);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_block_and_metadata() {
        let data = dict([
            ("items", Value::List(vec![])),
            ("next_cursor", s("c2")),
            ("has_more", Value::from(1)),
        ]);
        let envelope = build_envelope(
            "transactions.list",
            data,
            vec![("event".into(), Value::Bool(true))],
        )
        .unwrap();
        assert_eq!(
            cli_text(&envelope).unwrap(),
            "{\n  \"kind\": \"transactions.list\",\n  \"schema_version\": 1,\n  \"data\": {\n    \"items\": [],\n    \"next_cursor\": \"c2\",\n    \"has_more\": 1,\n    \"page\": {\n      \"next_cursor\": \"c2\",\n      \"has_more\": true\n    }\n  },\n  \"event\": true\n}\n"
        );
        assert_eq!(
            build_envelope("x", Value::Null, vec![("data".into(), Value::Null)]).unwrap_err(),
            ReservedKeyError("data".into())
        );
    }

    #[test]
    fn derive_kind_follows_the_attribute_order() {
        let ns = [
            ("command", "metadata"),
            ("metadata_command", "records"),
            ("records_command", "tag"),
            ("records_tag_command", "add"),
        ];
        let attr = |name: &str| ns.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
        assert_eq!(derive_kind(attr, None), "metadata.records.tag");
        assert_eq!(command_path(attr), "metadata.records.tag.add");
        assert_eq!(derive_kind(|_| None, None), "response");
        assert_eq!(
            derive_kind(|_| None, Some("projects.list")),
            "projects.list"
        );
    }

    #[test]
    fn daemon_lines_are_compact_and_ascii() {
        let record = dict([("kind", s("status")), ("data", s("caf\u{e9}"))]);
        assert_eq!(
            daemon_line(&record).unwrap(),
            "{\"kind\":\"status\",\"data\":\"caf\\u00e9\"}\n"
        );
    }
}
