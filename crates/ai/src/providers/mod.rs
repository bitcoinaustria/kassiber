//! The AI provider store (`kassiber/ai/providers.py`): the `ai_providers`
//! rows and their secret references, the one-shot seeding of the built-in
//! providers, the redacted output shape, and the gates every path that
//! contacts a provider passes. Chat, model discovery, connection tests and
//! document OCR all resolve providers here, so none of them can reach a
//! provider the others would refuse.
//!
//! A provider is held as Python's dict (`_row_to_dict`), in its key order,
//! so payloads built from it keep Python's bytes. Nothing here opens a
//! socket or starts a process.

use std::path::Path;

use kassiber_envelope::{dict, set, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{
    params, params_from_iter, types::ValueRef, Connection, OptionalExtension,
};
use sha2::{Digest, Sha256};

use crate::clock::now_iso;
use crate::pyval::{field, opt, s, str_or_none};

pub mod gates;
pub use gates::*;

pub const PROVIDER_KINDS: &[&str] = &["local", "remote", "tee"];
pub const DEFAULT_PROVIDER_SETTING: &str = "default_ai_provider";
pub const PROVIDERS_SEEDED_SETTING: &str = "ai_providers_seeded";
const NATIVE_SEED_GENERATIONS: &[(&str, &[&str])] = &[
    (
        "ai_native_broker_providers_seeded",
        &[
            "codex-cli://default",
            "claude-cli://default",
            "opencode-cli://default",
        ],
    ),
    ("ai_acp_broker_providers_seeded", &["copilot-cli://default"]),
];
const DESKTOP_BUNDLE_ID: &str = "at.bitcoinaustria.kassiber";
pub const SECRET_STORE_INLINE: &str = "sqlcipher_inline";
pub const SECRET_STORES: &[&str] = &[
    "macos_keychain",
    "windows_dpapi",
    "linux_secret_service",
    SECRET_STORE_INLINE,
];
pub const SECRET_STATES: &[&str] = &["ok", "missing", "needs_reauth", "unavailable"];
pub const REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
const DEFAULT_BOOTSTRAP_PROVIDER: &str = "ollama";
const SAFE_OUTPUT_FIELDS: &[&str] = &[
    "name",
    "display_name",
    "base_url",
    "default_model",
    "kind",
    "notes",
    "acknowledged_at",
    "enabled",
    "default_reasoning_effort",
    "api_format",
    "detected_api_format",
    "omit_store_field",
    "created_at",
    "updated_at",
];
pub const CLEARABLE_FIELDS: &[&str] = &[
    "api_key",
    "default_model",
    "notes",
    "default_reasoning_effort",
    "detected_api_format",
];

/// `CLI_PROVIDER_BY_LOCATOR`, in Python's order.
pub const CLI_PROVIDERS: &[(&str, &str)] = &[
    ("claude-cli://default", "claude"),
    ("codex-cli://default", "codex"),
    ("opencode-cli://default", "opencode"),
    ("copilot-cli://default", "copilot"),
];
pub const API_FORMAT_AUTO: &str = "auto";
pub const API_FORMAT_RESPONSES: &str = "responses";
pub const API_FORMAT_CHAT_COMPLETIONS: &str = "chat_completions";
pub const WIRE_API_FORMATS: &[&str] = &[API_FORMAT_RESPONSES, API_FORMAT_CHAT_COMPLETIONS];
pub const API_FORMATS: &[&str] = &[
    API_FORMAT_AUTO,
    API_FORMAT_RESPONSES,
    API_FORMAT_CHAT_COMPLETIONS,
];

/// The built-in rows (`DEFAULT_BOOTSTRAP_PROVIDERS`).
pub fn bootstrap_providers() -> Vec<Value> {
    match kassiber_pytext::loads(include_str!("defaults.json")) {
        Ok(Value::List(rows)) => rows,
        _ => unreachable!("defaults.json is a list"),
    }
}

/// Database failures without SQL, bound values or secrets, as the wallet
/// services map them.
pub(crate) fn database_error(error: kassiber_store::rusqlite::Error) -> AppError {
    let error = kassiber_store::map_sqlite_error(error);
    AppError {
        code: error.code.to_owned(),
        message: error.message.to_owned(),
        hint: error.hint.map(Box::from),
        details: kassiber_pytext::loads(&error.details.to_string()).ok(),
        retryable: error.retryable,
        message_codepoints: None,
        sqlite_origin: true,
    }
}

/// `is_cli_provider_locator(value)`.
pub fn is_cli_locator(value: &Value) -> bool {
    cli_provider_for_locator(value).is_some()
}

/// `cli_provider_for_locator(value)`.
pub fn cli_provider_for_locator(value: &Value) -> Option<&'static str> {
    let Value::Str(text) = value else {
        return None;
    };
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&crate::pyval::lossy(text)));
    CLI_PROVIDERS
        .iter()
        .find(|(locator, _)| *locator == text)
        .map(|(_, runtime)| *runtime)
}

fn cli_locator_text(text: &str) -> bool {
    is_cli_locator(&s(text))
}

fn setting(conn: &Connection, key: &str) -> Result<Option<Value>, AppError> {
    conn.query_row("SELECT value FROM settings WHERE key = ?", [key], |row| {
        Ok(value_of(row.get_ref(0)?))
    })
    .optional()
    .map_err(database_error)
}

fn setting_text(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    Ok(setting(conn, key)?.and_then(|value| match value {
        Value::Null => None,
        Value::Str(text) => Some(crate::pyval::lossy(&text)),
        other => Some(kassiber_envelope::py_str(&other)),
    }))
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )
    .map_err(database_error)?;
    Ok(())
}

/// `conn.commit()`; a no-op outside a transaction.
fn commit(conn: &Connection) -> Result<(), AppError> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(database_error)?;
    }
    Ok(())
}

/// Runs `body` as one transaction, as Python's implicit transaction up to
/// its `commit()` does.
fn atomically<T>(
    conn: &Connection,
    body: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let began = conn.is_autocommit();
    if began {
        conn.execute_batch("BEGIN").map_err(database_error)?;
    }
    match body() {
        Ok(value) => {
            commit(conn)?;
            Ok(value)
        }
        Err(error) => {
            if began && !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            Err(error)
        }
    }
}

fn value_of(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => Value::from(n),
        ValueRef::Real(n) => Value::Float(n),
        ValueRef::Text(t) => s(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

fn exists(conn: &Connection, sql: &str, values: &[&str]) -> Result<bool, AppError> {
    conn.query_row(sql, params_from_iter(values), |_| Ok(()))
        .optional()
        .map(|found| found.is_some())
        .map_err(database_error)
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

fn not_found(name: &str) -> AppError {
    AppError::new("not_found", format!("AI provider '{name}' not found"))
}

/// `_normalize_name(value)`.
pub fn normalize_name(value: &Value) -> Result<String, AppError> {
    let name = str_or_none(value).ok_or_else(|| validation("AI provider name is required"))?;
    Ok(kassiber_pytext::lower(&name))
}

fn normalize_name_text(name: &str) -> Result<String, AppError> {
    normalize_name(&s(name))
}

fn normalize_kind(value: &Value) -> Result<String, AppError> {
    let Some(kind) = str_or_none(value) else {
        return Ok("local".into());
    };
    let kind = kassiber_pytext::lower(&kind);
    if !PROVIDER_KINDS.contains(&kind.as_str()) {
        return Err(validation(format!("Unsupported AI provider kind '{kind}'"))
            .with_hint(format!("Choose one of: {}", PROVIDER_KINDS.join(", "))));
    }
    Ok(kind)
}

fn normalize_secret_store_id(value: &Value) -> Result<String, AppError> {
    let store = str_or_none(value).unwrap_or_else(|| SECRET_STORE_INLINE.into());
    let store = kassiber_pytext::lower(kassiber_pytext::strip(&store));
    if !SECRET_STORES.contains(&store.as_str()) {
        return Err(
            validation(format!("Unsupported AI provider secret store '{store}'"))
                .with_hint(format!("Choose one of: {}", SECRET_STORES.join(", "))),
        );
    }
    Ok(store)
}

fn normalize_secret_state(value: &Value) -> Result<String, AppError> {
    let state = str_or_none(value).unwrap_or_else(|| "missing".into());
    let state = kassiber_pytext::lower(kassiber_pytext::strip(&state));
    if !SECRET_STATES.contains(&state.as_str()) {
        return Err(
            validation(format!("Unsupported AI provider secret state '{state}'"))
                .with_hint(format!("Choose one of: {}", SECRET_STATES.join(", "))),
        );
    }
    Ok(state)
}

/// `normalize_reasoning_effort(value)`; empty clears it.
pub fn normalize_reasoning_effort(value: &Value) -> Result<Option<String>, AppError> {
    let Some(effort) = str_or_none(value) else {
        return Ok(None);
    };
    let effort = kassiber_pytext::lower(&effort);
    if !REASONING_EFFORTS.contains(&effort.as_str()) {
        return Err(
            validation(format!("Unsupported reasoning effort '{effort}'"))
                .with_hint(format!("Use one of: {}", REASONING_EFFORTS.join(", "))),
        );
    }
    Ok(Some(effort))
}

/// `normalize_api_format(value)`; empty means `auto`, `-` reads as `_`.
pub fn normalize_api_format(value: &Value) -> Result<String, AppError> {
    let Some(format) = str_or_none(value) else {
        return Ok(API_FORMAT_AUTO.into());
    };
    let format = kassiber_pytext::lower(&format).replace('-', "_");
    if !API_FORMATS.contains(&format.as_str()) {
        return Err(validation(format!("Unsupported AI API format '{format}'"))
            .with_hint(format!("Use one of: {}", API_FORMATS.join(", "))));
    }
    Ok(format)
}

/// `normalize_detected_api_format(value)`; empty means not detected.
pub fn normalize_detected_api_format(value: &Value) -> Result<Option<String>, AppError> {
    let Some(format) = str_or_none(value) else {
        return Ok(None);
    };
    let format = kassiber_pytext::lower(&format).replace('-', "_");
    if !WIRE_API_FORMATS.contains(&format.as_str()) {
        return Err(
            validation(format!("Unsupported detected AI API format '{format}'")).with_hint(
                format!("Use one of: {}, or null", WIRE_API_FORMATS.join(", ")),
            ),
        );
    }
    Ok(Some(format))
}

fn is_wire(value: &Value) -> Option<&'static str> {
    WIRE_API_FORMATS
        .iter()
        .find(|format| *value == s(format))
        .copied()
}

/// `resolve_ai_api_format(provider)`: `None` for a CLI locator.
pub fn resolve_api_format(provider: &Value) -> Option<&'static str> {
    if is_cli_locator(field(provider, "base_url")) {
        return None;
    }
    let format = field(provider, "api_format");
    if let Some(format) = is_wire(format) {
        return Some(format);
    }
    if let Some(detected) = is_wire(field(provider, "detected_api_format")) {
        return Some(detected);
    }
    Some(API_FORMAT_RESPONSES)
}

/// `http_reasoning_effort(value)`: the effort an HTTP request carries.
pub fn http_reasoning_effort(value: &Value) -> Option<&'static str> {
    let Value::Str(text) = value else {
        return None;
    };
    let effort = kassiber_pytext::lower(kassiber_pytext::strip(&crate::pyval::lossy(text)));
    match effort.as_str() {
        "low" => Some("low"),
        "medium" => Some("medium"),
        "high" | "xhigh" | "max" | "ultra" => Some("high"),
        _ => None,
    }
}

fn validate_locator_omit_store(base_url: &str, omit: bool) -> Result<(), AppError> {
    if omit && cli_locator_text(base_url) {
        return Err(
            validation("CLI agent providers do not take omit_store_field").with_hint(
                "Only an HTTP provider's Chat Completions requests carry the store field.",
            ),
        );
    }
    Ok(())
}

fn validate_locator_api_format(
    base_url: &str,
    format: &str,
    detected: Option<&str>,
) -> Result<(), AppError> {
    if !cli_locator_text(base_url) {
        return Ok(());
    }
    if format != API_FORMAT_AUTO || detected.is_some() {
        return Err(validation("CLI agent providers do not take an API format")
            .with_hint("Leave the API format on auto for a *-cli://default locator."));
    }
    Ok(())
}

/// `urlparse(url).hostname`, lowercased, and the port as Python reads it.
pub(crate) struct UrlParts {
    pub scheme: String,
    pub host: String,
    /// `None`, `Some(Ok(port))`, or `Some(Err(()))` for a port
    /// `urlparse(...).port` refuses with `ValueError`.
    pub port: Option<Result<u16, ()>>,
}

/// `urllib.parse.urlparse`'s scheme, hostname and port.
pub(crate) fn url_parts(url: &str) -> UrlParts {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest))
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            (kassiber_pytext::lower(scheme), rest)
        }
        _ => {
            return UrlParts {
                scheme: url
                    .split_once(':')
                    .map(|(scheme, _)| kassiber_pytext::lower(scheme))
                    .unwrap_or_default(),
                host: String::new(),
                port: None,
            }
        }
    };
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or("");
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, host)| host);
    let (host, port) = if let Some(bracketed) = hostinfo.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((host, rest)) => (host, rest.strip_prefix(':')),
            None => (bracketed, None),
        }
    } else {
        match hostinfo.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (hostinfo, None),
        }
    };
    let port = port.and_then(|port| {
        if port.is_empty() {
            return None;
        }
        Some(
            if port.bytes().all(|b| b.is_ascii_digit()) {
                port.parse::<u32>()
                    .ok()
                    .filter(|p| *p <= 65535)
                    .map(|p| p as u16)
            } else {
                None
            }
            .ok_or(()),
        )
    });
    UrlParts {
        scheme,
        host: kassiber_pytext::lower(host),
        port,
    }
}

/// `_is_loopback_ai_http_url(base_url)`.
pub fn is_loopback_http_url(base_url: &str) -> bool {
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return false;
    }
    let host = url_parts(base_url).host;
    let host = kassiber_pytext::strip(&host);
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `ai_base_url_origin(base_url)`: scheme, host and non-default port.
pub fn base_url_origin(base_url: &Value) -> String {
    let text = match base_url {
        Value::Null => String::new(),
        Value::Str(text) => crate::pyval::lossy(text),
        other => kassiber_envelope::py_str(other),
    };
    let text = kassiber_pytext::strip(&text).to_owned();
    if cli_locator_text(&text) {
        return kassiber_pytext::lower(&text);
    }
    let parts = url_parts(&text);
    let port = match parts.port {
        Some(Err(())) => return kassiber_pytext::lower(&text),
        Some(Ok(port)) => Some(port),
        None => None,
    };
    let netloc = if parts.host.contains(':') {
        format!("[{}]", parts.host)
    } else {
        parts.host.clone()
    };
    let default = match parts.scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    let netloc = match port {
        Some(port) if Some(port) != default => format!("{netloc}:{port}"),
        _ => netloc,
    };
    format!("{}://{netloc}", parts.scheme)
}

/// `infer_ai_provider_kind(base_url)`.
pub fn infer_kind(base_url: &str) -> &'static str {
    if is_loopback_http_url(base_url) {
        "local"
    } else {
        "remote"
    }
}

fn validate_locator_kind(base_url: &str, kind: &str) -> Result<(), AppError> {
    if kind != "local" {
        return Ok(());
    }
    if cli_locator_text(base_url) {
        return Err(validation("CLI agent providers cannot be marked local").with_hint(
            "Use --kind remote (or tee if your configured CLI path has documented confidential inference). These CLIs may send prompts to external model providers.",
        ));
    }
    if !is_loopback_http_url(base_url) {
        return Err(validation("Local AI providers must use a loopback HTTP endpoint").with_hint(
            "Use http://localhost, http://127.0.0.1, or http://[::1] for local providers. Mark off-device endpoints as remote and acknowledge remote use.",
        ));
    }
    Ok(())
}

/// `_data_root_from_connection(conn)`.
fn data_root(conn: &Connection) -> String {
    let file: Option<String> = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    match file.filter(|file| !file.is_empty()) {
        Some(file) => kassiber_paths::canonicalize_allow_missing(Path::new(&file))
            .ok()
            .and_then(|path| path.parent().map(|p| p.to_string_lossy().into_owned()))
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// `ai_provider_secret_service_id(data_root)`.
pub fn secret_service_id(data_root: &str) -> String {
    let root = if data_root.is_empty() {
        String::new()
    } else {
        kassiber_paths::canonicalize_allow_missing(Path::new(data_root))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| data_root.to_owned())
    };
    Sha256::digest(format!("{DESKTOP_BUNDLE_ID}:{root}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `ai_provider_secret_ref_namespace(conn, provider_name)`.
pub fn secret_ref_namespace(conn: &Connection, name: &str) -> Result<(String, String), AppError> {
    Ok((
        secret_service_id(&data_root(conn)),
        normalize_name_text(name)?,
    ))
}

fn default_secret_ref(conn: &Connection, name: &str, has_secret: bool) -> Value {
    dict([
        ("store_id", s(SECRET_STORE_INLINE)),
        ("service", s(secret_service_id(&data_root(conn)))),
        ("account", s(name)),
        ("state", s(if has_secret { "ok" } else { "missing" })),
        ("created_at", Value::Null),
        ("rotated_at", Value::Null),
    ])
}

fn row_secret_ref(conn: &Connection, row: &Value) -> Result<Value, AppError> {
    let has_secret = str_or_none(field(row, "api_key")).is_some();
    let name = normalize_name(field(row, "name"))?;
    let (expected_service, expected_account) = secret_ref_namespace(conn, &name)?;
    if !truthy(field(row, "secret_store_id")) {
        return Ok(default_secret_ref(conn, &name, has_secret));
    }
    let store = normalize_secret_store_id(field(row, "secret_store_id"))?;
    let or = |key: &str, fallback: &str| {
        let value = field(row, key);
        if truthy(value) {
            kassiber_envelope::py_str(value)
        } else {
            fallback.to_owned()
        }
    };
    let mut service = or("secret_service", &expected_service);
    let mut account = or("secret_account", &expected_account);
    let state_value = if truthy(field(row, "secret_state")) {
        field(row, "secret_state").clone()
    } else {
        s(if has_secret { "ok" } else { "missing" })
    };
    let mut state = normalize_secret_state(&state_value)?;
    if store != SECRET_STORE_INLINE && (service != expected_service || account != expected_account)
    {
        // Rows can come from restored or imported books: never surface or
        // use a native ref outside this data root's and provider's namespace.
        service = expected_service;
        account = expected_account;
        state = "unavailable".into();
    }
    Ok(dict([
        ("store_id", s(store)),
        ("service", s(service)),
        ("account", s(account)),
        ("state", s(state)),
        ("created_at", field(row, "secret_created_at").clone()),
        ("rotated_at", field(row, "secret_rotated_at").clone()),
    ]))
}

fn upsert_secret_ref(
    conn: &Connection,
    name: &str,
    store_id: &str,
    state: &str,
    service: Option<&str>,
    account: Option<&str>,
) -> Result<Value, AppError> {
    let name = normalize_name_text(name)?;
    let store = normalize_secret_store_id(&s(store_id))?;
    let state = normalize_secret_state(&s(state))?;
    let service = service
        .and_then(|v| str_or_none(&s(v)))
        .unwrap_or_else(|| secret_service_id(&data_root(conn)));
    let account = account
        .and_then(|v| str_or_none(&s(v)))
        .unwrap_or_else(|| name.clone());
    let timestamp = now_iso();
    let created: Option<Value> = conn
        .query_row(
            "SELECT created_at FROM ai_provider_secret_refs WHERE provider_name = ?",
            [&name],
            |row| Ok(value_of(row.get_ref(0)?)),
        )
        .optional()
        .map_err(database_error)?;
    let created = created.unwrap_or_else(|| s(&timestamp));
    let created_text = match &created {
        Value::Str(text) => Some(crate::pyval::lossy(text)),
        Value::Null => None,
        other => Some(kassiber_envelope::py_str(other)),
    };
    conn.execute(
        "INSERT INTO ai_provider_secret_refs(provider_name, store_id, service, account, state, created_at, rotated_at) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(provider_name) DO UPDATE SET store_id = excluded.store_id, service = excluded.service, account = excluded.account, state = excluded.state, rotated_at = excluded.rotated_at",
        params![name, store, service, account, state, created_text, timestamp],
    )
    .map_err(database_error)?;
    Ok(dict([
        ("store_id", s(store)),
        ("service", s(service)),
        ("account", s(account)),
        ("state", s(state)),
        ("created_at", created),
        ("rotated_at", s(timestamp)),
    ]))
}

/// `mark_ai_provider_secret_ref_state(conn, name, state)`.
pub fn mark_secret_ref_state(
    conn: &Connection,
    name: &str,
    state: &str,
) -> Result<Value, AppError> {
    let name = normalize_name_text(name)?;
    let provider = get(conn, &name)?;
    let reference = field(&provider, "secret_ref");
    let pick =
        |key: &str| str_or_none(field(reference, key)).filter(|_| truthy(field(reference, key)));
    let store = if truthy(field(reference, "store_id")) {
        kassiber_envelope::py_str(field(reference, "store_id"))
    } else {
        SECRET_STORE_INLINE.into()
    };
    upsert_secret_ref(
        conn,
        &name,
        &store,
        state,
        pick("service").as_deref(),
        pick("account").as_deref(),
    )
}

fn secret_ref_for_error(provider: &Value) -> Value {
    let reference = field(provider, "secret_ref");
    let store = if truthy(field(reference, "store_id")) {
        field(reference, "store_id").clone()
    } else {
        s(SECRET_STORE_INLINE)
    };
    let mut state = if truthy(field(reference, "state")) {
        field(reference, "state").clone()
    } else {
        s("missing")
    };
    if store != s(SECRET_STORE_INLINE) && state == s("ok") {
        state = s("unavailable");
    }
    let account = if truthy(field(reference, "account")) {
        field(reference, "account").clone()
    } else {
        field(provider, "name").clone()
    };
    dict([
        ("provider_name", field(provider, "name").clone()),
        ("store_id", store),
        ("service", field(reference, "service").clone()),
        ("account", account),
        ("state", state),
    ])
}

/// How a native secret reference is read: the desktop's keychain over the
/// daemon's control channel. `Ok(None)` is a missing secret.
pub type SecretResolver<'a> = dyn FnMut(&Value) -> Result<Option<String>, AppError> + 'a;

fn unavailable_secret(
    provider: &Value,
    state: &str,
    message: String,
    extra: Option<(&str, Value)>,
    hint: Option<String>,
) -> AppError {
    let mut reference = secret_ref_for_error(provider);
    set(&mut reference, "state", s(state));
    let mut details = vec![("refs", Value::List(vec![reference])), ("state", s(state))];
    if let Some(extra) = extra {
        details.push(extra);
    }
    AppError::new("secret_ref_unavailable", message)
        .with_hint(hint.unwrap_or_else(|| {
            "Open Settings -> AI providers and re-enter or repair the provider API key.".into()
        }))
        .with_details(dict(details))
        .retryable(true)
}

/// `get_ai_provider_api_key_for_use(provider, conn=..., secret_resolver=...)`.
pub fn api_key_for_use(
    provider: &Value,
    conn: Option<&Connection>,
    resolver: Option<&mut SecretResolver<'_>>,
) -> Result<Option<String>, AppError> {
    let reference = field(provider, "secret_ref");
    let store = if truthy(field(reference, "store_id")) {
        kassiber_envelope::py_str(field(reference, "store_id"))
    } else {
        SECRET_STORE_INLINE.into()
    };
    let state = if truthy(field(reference, "state")) {
        kassiber_envelope::py_str(field(reference, "state"))
    } else if str_or_none(field(provider, "api_key")).is_some() {
        "ok".into()
    } else {
        "missing".into()
    };
    if store == SECRET_STORE_INLINE {
        return Ok(str_or_none(field(provider, "api_key")));
    }
    let name = kassiber_envelope::py_str(field(provider, "name"));
    let has_name = truthy(field(provider, "name"));
    let message = |native: bool| {
        if native {
            format!("AI provider '{name}' secret is not available in the OS credential store")
        } else {
            format!("AI provider '{name}' secret is not available")
        }
    };
    if state != "ok" {
        if let Some(conn) = conn {
            if has_name && state == "unavailable" {
                mark_secret_ref_state(conn, &name, &state)?;
                commit(conn)?;
            }
        }
        let reference = secret_ref_for_error(provider);
        return Err(AppError::new("secret_ref_unavailable", message(false))
            .with_hint("Open Settings -> AI providers and re-enter or repair the provider API key.")
            .with_details(dict([
                ("refs", Value::List(vec![reference])),
                ("state", s(state)),
            ]))
            .retryable(true));
    }
    let state = if let Some(resolver) = resolver {
        let mut request = reference.clone();
        set(
            &mut request,
            "provider_name",
            field(provider, "name").clone(),
        );
        match resolver(&request) {
            Err(error) => {
                if let Some(conn) = conn {
                    if has_name {
                        mark_secret_ref_state(conn, &name, "unavailable")?;
                    }
                    commit(conn)?;
                }
                return Err(unavailable_secret(
                    provider,
                    "unavailable",
                    message(true),
                    Some(("cause_code", s(&error.code))),
                    error.hint.as_deref().map(str::to_owned),
                ));
            }
            Ok(secret) => {
                if let Some(secret) = secret.and_then(|secret| str_or_none(&s(secret))) {
                    return Ok(Some(secret));
                }
                "missing"
            }
        }
    } else {
        "unavailable"
    };
    if let Some(conn) = conn {
        if has_name {
            mark_secret_ref_state(conn, &name, state)?;
            commit(conn)?;
        }
    }
    Err(unavailable_secret(
        provider,
        state,
        message(true),
        None,
        None,
    ))
}

/// `normalize_base_url(value)`.
pub fn normalize_base_url(value: &Value) -> Result<String, AppError> {
    let Some(base) = str_or_none(value) else {
        return Err(validation("AI provider base_url is required").with_hint(
            "Use an OpenAI-compatible API root, e.g. http://localhost:11434/v1, or a built-in *-cli://default locator.",
        ));
    };
    let base = kassiber_pytext::strip(&base)
        .trim_end_matches('/')
        .to_owned();
    if base.is_empty() {
        return Err(validation("AI provider base_url is required"));
    }
    if cli_locator_text(&base) {
        return Ok(kassiber_pytext::lower(&base));
    }
    if !base.contains("://") {
        return Err(
            validation(format!("AI provider base_url '{base}' is missing a scheme"))
                .with_hint("Include http://, https://, or a built-in *-cli:// scheme."),
        );
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(
            validation(format!("Unsupported AI provider locator '{base}'"))
                .with_hint("Use http(s):// or a built-in *-cli://default locator."),
        );
    }
    Ok(base)
}

/// `redact_ai_provider_for_output(provider, default_name=...)`: never the
/// API key. `is_default` is present only with a default name, as Python
/// adds it only when `default_name is not None`.
pub fn redact_for_output(provider: &Value, default: Option<&str>) -> Value {
    let mut payload = Vec::new();
    for key in SAFE_OUTPUT_FIELDS {
        if let Some(value) = kassiber_envelope::get(provider, key) {
            payload.push((*key, value.clone()));
        }
    }
    let reference = field(provider, "secret_ref");
    let store = if truthy(field(reference, "store_id")) {
        field(reference, "store_id").clone()
    } else {
        s(SECRET_STORE_INLINE)
    };
    let mut state = field(reference, "state").clone();
    let inline = str_or_none(field(provider, "api_key")).is_some();
    if store == s(SECRET_STORE_INLINE) {
        state = s(if inline { "ok" } else { "missing" });
    }
    let has_key = inline || (store != s(SECRET_STORE_INLINE) && state == s("ok"));
    payload.push(("has_api_key", Value::Bool(has_key)));
    let state = if truthy(&state) {
        state
    } else {
        s(if has_key { "ok" } else { "missing" })
    };
    payload.push(("secret_ref", dict([("store_id", store), ("state", state)])));
    payload.push((
        "supports_reasoning_effort",
        Value::Bool(is_cli_locator(field(provider, "base_url"))),
    ));
    payload.push(("effective_api_format", opt(resolve_api_format(provider))));
    if let Some(default) = default {
        payload.push((
            "is_default",
            Value::Bool(*field(provider, "name") == s(default)),
        ));
    }
    dict(payload)
}

fn stored_api_format(value: &Value) -> Value {
    if API_FORMATS.iter().any(|format| *value == s(format)) {
        value.clone()
    } else {
        s(API_FORMAT_AUTO)
    }
}

fn stored_detected_api_format(value: &Value) -> Value {
    is_wire(value).map_or(Value::Null, s)
}

const SELECT: &str = "SELECT p.*, r.store_id AS secret_store_id, r.service AS secret_service, r.account AS secret_account, r.state AS secret_state, r.created_at AS secret_created_at, r.rotated_at AS secret_rotated_at FROM ai_providers p LEFT JOIN ai_provider_secret_refs r ON r.provider_name = p.name";

fn rows(conn: &Connection, sql: &str, values: &[&str]) -> Result<Vec<Value>, AppError> {
    let mut statement = conn.prepare(sql).map_err(database_error)?;
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            let mut pairs = Vec::new();
            for (index, name) in names.iter().enumerate() {
                pairs.push((name.as_str(), value_of(row.get_ref(index)?)));
            }
            Ok(dict(pairs))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(rows)
}

/// `_row_to_dict(conn, row)`.
fn row_to_dict(conn: &Connection, row: &Value) -> Result<Value, AppError> {
    let display = if truthy(field(row, "display_name")) {
        field(row, "display_name").clone()
    } else {
        field(row, "name").clone()
    };
    let enabled = match field(row, "enabled") {
        Value::Null => Value::Bool(true),
        other => Value::Bool(truthy(other)),
    };
    Ok(dict([
        ("name", field(row, "name").clone()),
        ("display_name", display),
        ("base_url", field(row, "base_url").clone()),
        ("api_key", field(row, "api_key").clone()),
        ("default_model", field(row, "default_model").clone()),
        ("kind", field(row, "kind").clone()),
        ("notes", field(row, "notes").clone()),
        ("acknowledged_at", field(row, "acknowledged_at").clone()),
        ("enabled", enabled),
        (
            "default_reasoning_effort",
            field(row, "default_reasoning_effort").clone(),
        ),
        ("api_format", stored_api_format(field(row, "api_format"))),
        (
            "detected_api_format",
            stored_detected_api_format(field(row, "detected_api_format")),
        ),
        (
            "omit_store_field",
            Value::Bool(truthy(field(row, "omit_store_field"))),
        ),
        ("created_at", field(row, "created_at").clone()),
        ("updated_at", field(row, "updated_at").clone()),
        ("secret_ref", row_secret_ref(conn, row)?),
    ]))
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// `seed_default_ai_provider_if_empty(conn)`.
pub fn seed_default_if_empty(conn: &Connection) -> Result<(), AppError> {
    if setting_text(conn, PROVIDERS_SEEDED_SETTING)?.is_some_and(|v| !v.is_empty()) {
        return Ok(());
    }
    if exists(conn, "SELECT 1 FROM ai_providers LIMIT 1", &[])? {
        set_setting(conn, PROVIDERS_SEEDED_SETTING, "1")?;
        return commit(conn);
    }
    let defaults = bootstrap_providers();
    let default = kassiber_pytext::lower(kassiber_pytext::strip(
        &env("KASSIBER_DEFAULT_AI_PROVIDER").unwrap_or_else(|| DEFAULT_BOOTSTRAP_PROVIDER.into()),
    ));
    let mut names: Vec<String> = defaults
        .iter()
        .map(|row| kassiber_envelope::py_str(field(row, "name")))
        .collect();
    if !names.contains(&default) {
        names.sort();
        return Err(
            validation(format!("Unsupported default AI provider '{default}'"))
                .with_hint(format!("Choose one of: {}", names.join(", "))),
        );
    }
    atomically(conn, || {
        let timestamp = now_iso();
        for row in &defaults {
            let name = kassiber_envelope::py_str(field(row, "name"));
            let base = env(&format!(
                "KASSIBER_{}_AI_BASE_URL",
                name.to_uppercase().replace('-', "_")
            ))
            .or_else(|| {
                (name == default)
                    .then(|| env("KASSIBER_DEFAULT_AI_BASE_URL"))
                    .flatten()
            })
            .unwrap_or_else(|| kassiber_envelope::py_str(field(row, "base_url")));
            let local = *field(row, "kind") == s("local");
            conn.execute(
                "INSERT INTO ai_providers(name, display_name, base_url, api_key, default_model, kind, notes, acknowledged_at, created_at, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    name,
                    text_param(field(row, "display_name")),
                    base,
                    text_param(field(row, "api_key")),
                    text_param(field(row, "default_model")),
                    text_param(field(row, "kind")),
                    text_param(field(row, "notes")),
                    local.then(now_iso),
                    timestamp,
                    timestamp
                ],
            )
            .map_err(database_error)?;
        }
        if !setting_text(conn, DEFAULT_PROVIDER_SETTING)?.is_some_and(|v| !v.is_empty()) {
            set_setting(conn, DEFAULT_PROVIDER_SETTING, &default)?;
        }
        set_setting(conn, PROVIDERS_SEEDED_SETTING, "1")
    })
}

fn text_param(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(kassiber_envelope::py_str(other)),
    }
}

/// `_free_provider_name(conn, name)`.
fn free_name(conn: &Connection, name: &str) -> Result<String, AppError> {
    let (mut candidate, mut suffix) = (name.to_owned(), 2);
    while exists(
        conn,
        "SELECT 1 FROM ai_providers WHERE name = ? LIMIT 1",
        &[&candidate],
    )? {
        candidate = format!("{name}-{suffix}");
        suffix += 1;
    }
    Ok(candidate)
}

/// `seed_native_ai_providers(conn)`: each generation once per book.
pub fn seed_native(conn: &Connection) -> Result<(), AppError> {
    let mut pending = Vec::new();
    for (setting, locators) in NATIVE_SEED_GENERATIONS {
        if !setting_text(conn, setting)?.is_some_and(|v| !v.is_empty()) {
            pending.push((*setting, *locators));
        }
    }
    if pending.is_empty() {
        return Ok(());
    }
    atomically(conn, || {
        let timestamp = now_iso();
        let locators: Vec<&str> = pending
            .iter()
            .flat_map(|(_, l)| l.iter().copied())
            .collect();
        let mut rows = Vec::new();
        for row in bootstrap_providers() {
            let base = kassiber_envelope::py_str(field(&row, "base_url"));
            if locators.contains(&base.as_str())
                && !exists(
                    conn,
                    "SELECT 1 FROM ai_providers WHERE lower(base_url) = lower(?) LIMIT 1",
                    &[&base],
                )?
            {
                rows.push(row);
            }
        }
        for row in rows {
            let name = free_name(conn, &kassiber_envelope::py_str(field(&row, "name")))?;
            conn.execute(
                "INSERT INTO ai_providers(name, display_name, base_url, api_key, default_model, kind, notes, acknowledged_at, created_at, updated_at) VALUES(?, ?, ?, NULL, ?, ?, ?, NULL, ?, ?)",
                params![
                    name,
                    text_param(field(&row, "display_name")),
                    text_param(field(&row, "base_url")),
                    text_param(field(&row, "default_model")),
                    text_param(field(&row, "kind")),
                    text_param(field(&row, "notes")),
                    timestamp,
                    timestamp
                ],
            )
            .map_err(database_error)?;
        }
        for (setting, _) in &pending {
            set_setting(conn, setting, "1")?;
        }
        Ok(())
    })
}

/// `_ensure_seeded(conn)`: every seed generation, before any read or change.
pub fn ensure_seeded(conn: &Connection) -> Result<(), AppError> {
    seed_default_if_empty(conn)?;
    seed_native(conn)
}

/// `list_db_ai_providers(conn)`, sorted by name.
pub fn list(conn: &Connection) -> Result<Vec<Value>, AppError> {
    ensure_seeded(conn)?;
    rows(conn, &format!("{SELECT} ORDER BY p.name ASC"), &[])?
        .iter()
        .map(|row| row_to_dict(conn, row))
        .collect()
}

/// `get_db_ai_provider(conn, name)`.
pub fn get(conn: &Connection, name: &str) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    let found = rows(conn, &format!("{SELECT} WHERE p.name = ?"), &[&name])?;
    let row = found.first().ok_or_else(|| {
        not_found(&name).with_hint("Use `kassiber ai providers list` to see configured providers.")
    })?;
    row_to_dict(conn, row)
}

/// `get_default_ai_provider_name(conn)`.
pub fn default_name(conn: &Connection) -> Result<Option<String>, AppError> {
    ensure_seeded(conn)?;
    setting_text(conn, DEFAULT_PROVIDER_SETTING)
}

/// `resolve_ai_provider(conn, name)`.
pub fn resolve(conn: &Connection, name: Option<&str>) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = match name {
        Some(name) => Some(name.to_owned()),
        None => setting_text(conn, DEFAULT_PROVIDER_SETTING)?,
    };
    match name.filter(|name| !name.is_empty()) {
        Some(name) => get(conn, &name),
        None => Err(AppError::new("ai_provider_not_configured", "No AI provider is configured")
            .with_hint("Run `kassiber ai providers create` or set a default with `kassiber ai providers set-default`.")),
    }
}

/// The arguments of `create_db_ai_provider`.
pub struct Create {
    pub name: String,
    pub base_url: String,
    pub display_name: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Value,
    pub kind: Value,
    pub notes: Value,
    pub acknowledged: bool,
    pub enabled: bool,
    pub default_reasoning_effort: Value,
    pub api_format: Value,
    pub detected_api_format: Value,
    pub omit_store_field: bool,
}

impl Default for Create {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_url: String::new(),
            display_name: None,
            api_key: None,
            default_model: Value::Null,
            kind: Value::Null,
            notes: Value::Null,
            acknowledged: false,
            enabled: true,
            default_reasoning_effort: Value::Null,
            api_format: Value::Null,
            detected_api_format: Value::Null,
            omit_store_field: false,
        }
    }
}

/// `create_db_ai_provider(conn, ...)`.
pub fn create(conn: &Connection, request: Create) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let raw_name = str_or_none(&s(&request.name));
    let name = normalize_name_text(&request.name)?;
    let display = request
        .display_name
        .as_deref()
        .and_then(|display| str_or_none(&s(display)))
        .unwrap_or_else(|| raw_name.clone().unwrap_or_else(|| name.clone()));
    let base_url = normalize_base_url(&s(&request.base_url))?;
    let kind = if str_or_none(&request.kind).is_some() {
        normalize_kind(&request.kind)?
    } else {
        infer_kind(&base_url).into()
    };
    validate_locator_kind(&base_url, &kind)?;
    let api_format = normalize_api_format(&request.api_format)?;
    let detected = normalize_detected_api_format(&request.detected_api_format)?;
    validate_locator_api_format(&base_url, &api_format, detected.as_deref())?;
    validate_locator_omit_store(&base_url, request.omit_store_field)?;
    let api_key = request
        .api_key
        .as_deref()
        .and_then(|key| str_or_none(&s(key)));
    let default_model = str_or_none(&request.default_model);
    let notes = str_or_none(&request.notes);
    let effort = normalize_reasoning_effort(&request.default_reasoning_effort)?;
    if exists(conn, "SELECT 1 FROM ai_providers WHERE name = ?", &[&name])? {
        return Err(
            AppError::new("conflict", format!("AI provider '{name}' already exists"))
                .with_hint("Use `kassiber ai providers update` to change an existing provider."),
        );
    }
    atomically(conn, || {
        let timestamp = now_iso();
        let acknowledged = (request.acknowledged || kind == "local").then(|| timestamp.clone());
        conn.execute(
            "INSERT INTO ai_providers(name, display_name, base_url, api_key, default_model, kind, notes, acknowledged_at, enabled, default_reasoning_effort, api_format, detected_api_format, omit_store_field, created_at, updated_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                name,
                display,
                base_url,
                api_key,
                default_model,
                kind,
                notes,
                acknowledged,
                i64::from(request.enabled),
                effort,
                api_format,
                detected,
                i64::from(request.omit_store_field),
                timestamp,
                timestamp
            ],
        )
        .map_err(database_error)?;
        if api_key.is_some() {
            upsert_secret_ref(conn, &name, SECRET_STORE_INLINE, "ok", None, None)?;
        }
        Ok(())
    })?;
    get(conn, &name)
}

/// The `updates` of `update_db_ai_provider`; `None` (null) is "not given",
/// as Python's `updates.get(field) is not None` reads it.
pub struct Update {
    pub display_name: Value,
    pub base_url: Value,
    pub api_key: Value,
    pub default_model: Value,
    pub kind: Value,
    pub notes: Value,
    pub default_reasoning_effort: Value,
    pub enabled: Value,
    pub api_format: Value,
    pub detected_api_format: Value,
    pub omit_store_field: Value,
    pub clear: Vec<String>,
    /// Whether `acknowledged` / `acknowledge_clear` keys were given, and
    /// their truth.
    pub acknowledged: Option<bool>,
    pub acknowledge_clear: Option<bool>,
}

impl Default for Update {
    fn default() -> Self {
        Self {
            display_name: Value::Null,
            base_url: Value::Null,
            api_key: Value::Null,
            default_model: Value::Null,
            kind: Value::Null,
            notes: Value::Null,
            default_reasoning_effort: Value::Null,
            enabled: Value::Null,
            api_format: Value::Null,
            detected_api_format: Value::Null,
            omit_store_field: Value::Null,
            clear: Vec::new(),
            acknowledged: None,
            acknowledge_clear: None,
        }
    }
}

/// `update_db_ai_provider(conn, name, updates)`.
pub fn update(conn: &Connection, name: &str, updates: Update) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    let found = rows(conn, "SELECT * FROM ai_providers WHERE name = ?", &[&name])?;
    let Some(row) = found.into_iter().next() else {
        return Err(not_found(&name));
    };
    let mut clear: std::collections::BTreeSet<String> = updates
        .clear
        .iter()
        .filter(|field| CLEARABLE_FIELDS.contains(&field.as_str()))
        .cloned()
        .collect();
    let mut unsupported: Vec<&String> = updates
        .clear
        .iter()
        .filter(|field| !CLEARABLE_FIELDS.contains(&field.as_str()))
        .collect();
    unsupported.sort();
    unsupported.dedup();
    if !unsupported.is_empty() {
        let mut clearable = CLEARABLE_FIELDS.to_vec();
        clearable.sort_unstable();
        return Err(validation(format!(
            "Unsupported AI provider clear field(s): {}",
            unsupported
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .with_hint(format!("Clearable fields: {}", clearable.join(", "))));
    }
    let present = |value: &Value| *value != Value::Null;
    let nontrivial = [
        &updates.display_name,
        &updates.base_url,
        &updates.api_key,
        &updates.default_model,
        &updates.kind,
        &updates.notes,
        &updates.default_reasoning_effort,
        &updates.enabled,
        &updates.api_format,
        &updates.detected_api_format,
        &updates.omit_store_field,
    ]
    .into_iter()
    .any(present);
    if !nontrivial
        && clear.is_empty()
        && updates.acknowledged.is_none()
        && updates.acknowledge_clear.is_none()
    {
        return Err(validation("ai providers update requires at least one field to change").with_hint(
            "Pass --display-name, --base-url, --api-key, --default-model, --default-effort, --kind, --api-format, --omit-store-field, --send-store-field, --notes, --enable, --disable, --acknowledge, --clear FIELD, or --revoke-acknowledge",
        ));
    }
    let enabled = match &updates.enabled {
        Value::Null => None,
        Value::Bool(enabled) => Some(*enabled),
        _ => return Err(validation("AI provider enabled must be a boolean")),
    };
    if enabled == Some(false)
        && setting_text(conn, DEFAULT_PROVIDER_SETTING)?.as_deref() == Some(name.as_str())
    {
        return Err(AppError::new(
            "conflict",
            format!("AI provider '{name}' is the stored default and cannot be disabled"),
        )
        .with_hint("Set another provider as the default first, or clear it with `kassiber ai providers clear-default`."));
    }
    let omit_store = match &updates.omit_store_field {
        Value::Null => None,
        Value::Bool(omit) => Some(*omit),
        _ => return Err(validation("AI provider omit_store_field must be a boolean")),
    };
    let effort = if present(&updates.default_reasoning_effort) {
        normalize_reasoning_effort(&updates.default_reasoning_effort)?
    } else {
        None
    };
    let mut kind = if present(&updates.kind) {
        Some(normalize_kind(&updates.kind)?)
    } else {
        None
    };
    let base_url = if present(&updates.base_url) {
        Some(normalize_base_url(&updates.base_url)?)
    } else {
        None
    };
    let api_format = if present(&updates.api_format) {
        Some(normalize_api_format(&updates.api_format)?)
    } else {
        None
    };
    let detected = if present(&updates.detected_api_format) {
        normalize_detected_api_format(&updates.detected_api_format)?
    } else {
        None
    };
    let row_kind = field(&row, "kind").clone();
    if kind.is_none()
        && base_url
            .as_deref()
            .is_some_and(|base| row_kind == s("local") && !is_loopback_http_url(base))
    {
        kind = Some(infer_kind(base_url.as_deref().unwrap()).into());
    }
    let mut acknowledged_at = field(&row, "acknowledged_at").clone();
    if updates.acknowledge_clear == Some(true) {
        acknowledged_at = Value::Null;
    }
    if row_kind == s("local") && matches!(kind.as_deref(), Some("remote" | "tee")) {
        // A local provider is acknowledged implicitly; that never covered
        // sending prompts off the device.
        acknowledged_at = Value::Null;
    }
    if updates.acknowledged == Some(true) {
        acknowledged_at = s(now_iso());
    }
    if kind.as_deref() == Some("local") && !truthy(&acknowledged_at) {
        acknowledged_at = s(now_iso());
    }
    let row_base = field(&row, "base_url").clone();
    let base_changed = base_url.as_ref().is_some_and(|base| s(base) != row_base);
    let origin_changed = base_changed
        && base_url_origin(&s(base_url.as_deref().unwrap())) != base_url_origin(&row_base);
    if origin_changed && !present(&updates.api_key) {
        // API keys are origin-bound: a retarget to another origin drops the
        // key so the next request cannot send it there.
        clear.insert("api_key".into());
    }
    if base_changed && detected.is_none() {
        clear.insert("detected_api_format".into());
    }
    let resolved = |key: &str, provided: &Value, fallback: Value| -> Value {
        if clear.contains(key) {
            Value::Null
        } else if present(provided) {
            opt(str_or_none(provided))
        } else {
            fallback
        }
    };
    let display = resolved(
        "display_name",
        &updates.display_name,
        if truthy(field(&row, "display_name")) {
            field(&row, "display_name").clone()
        } else {
            field(&row, "name").clone()
        },
    );
    let merged_base = base_url.clone().map_or(row_base.clone(), s);
    let api_key = resolved("api_key", &updates.api_key, field(&row, "api_key").clone());
    let default_model = resolved(
        "default_model",
        &updates.default_model,
        field(&row, "default_model").clone(),
    );
    let merged_kind = kind.clone().map_or(row_kind.clone(), s);
    let notes = resolved("notes", &updates.notes, field(&row, "notes").clone());
    let merged_enabled = match enabled {
        Some(enabled) => Value::from(i64::from(enabled)),
        None => field(&row, "enabled").clone(),
    };
    let merged_effort = if clear.contains("default_reasoning_effort") {
        Value::Null
    } else if let Some(effort) = effort {
        s(effort)
    } else {
        field(&row, "default_reasoning_effort").clone()
    };
    let merged_format = match api_format {
        Some(format) => s(format),
        None if base_changed && is_cli_locator(&merged_base) => s(API_FORMAT_AUTO),
        None => stored_api_format(field(&row, "api_format")),
    };
    let merged_detected = match detected {
        Some(detected) => s(detected),
        None if clear.contains("detected_api_format") => Value::Null,
        None => stored_detected_api_format(field(&row, "detected_api_format")),
    };
    let merged_omit = match omit_store {
        Some(omit) => omit,
        None if base_changed => false,
        None => truthy(field(&row, "omit_store_field")),
    };
    let base_text = kassiber_envelope::py_str(&merged_base);
    validate_locator_kind(&base_text, &kassiber_envelope::py_str(&merged_kind))?;
    validate_locator_api_format(
        &base_text,
        &kassiber_envelope::py_str(&merged_format),
        crate::pyval::string(&merged_detected).as_deref(),
    )?;
    validate_locator_omit_store(&base_text, merged_omit)?;
    atomically(conn, || {
        conn.execute(
            "UPDATE ai_providers SET display_name = ?, base_url = ?, api_key = ?, default_model = ?, kind = ?, notes = ?, acknowledged_at = ?, enabled = ?, default_reasoning_effort = ?, api_format = ?, detected_api_format = ?, omit_store_field = ?, updated_at = ? WHERE name = ?",
            params![
                SqlValue(&display),
                SqlValue(&merged_base),
                SqlValue(&api_key),
                SqlValue(&default_model),
                SqlValue(&merged_kind),
                SqlValue(&notes),
                SqlValue(&acknowledged_at),
                SqlValue(&merged_enabled),
                SqlValue(&merged_effort),
                SqlValue(&merged_format),
                SqlValue(&merged_detected),
                i64::from(merged_omit),
                now_iso(),
                name
            ],
        )
        .map_err(database_error)?;
        if clear.contains("api_key") {
            upsert_secret_ref(conn, &name, SECRET_STORE_INLINE, "missing", None, None)?;
        } else if present(&updates.api_key) {
            upsert_secret_ref(conn, &name, SECRET_STORE_INLINE, "ok", None, None)?;
        }
        Ok(())
    })?;
    get(conn, &name)
}

/// A Python value bound as SQLite would bind it.
struct SqlValue<'a>(&'a Value);

impl kassiber_store::rusqlite::ToSql for SqlValue<'_> {
    fn to_sql(
        &self,
    ) -> kassiber_store::rusqlite::Result<kassiber_store::rusqlite::types::ToSqlOutput<'_>> {
        use kassiber_store::rusqlite::types::{ToSqlOutput, Value as Sql};
        Ok(ToSqlOutput::Owned(match self.0 {
            Value::Null => Sql::Null,
            Value::Bool(value) => Sql::Integer(i64::from(*value)),
            Value::Int(value) => Sql::Integer(i64::try_from(value).unwrap_or(i64::MAX)),
            Value::Float(value) => Sql::Real(*value),
            Value::Bytes(value) => Sql::Blob(value.clone()),
            Value::Str(text) => Sql::Text(crate::pyval::lossy(text)),
            other => Sql::Text(kassiber_envelope::py_str(other)),
        }))
    }
}

/// `record_detected_api_format(conn, name, base_url, detected)`: true when
/// the row, still at `base_url`, now records the format.
pub fn record_detected_api_format(
    conn: &Connection,
    name: &str,
    base_url: &str,
    detected: &str,
) -> Result<bool, AppError> {
    let Some(detected) = normalize_detected_api_format(&s(detected))? else {
        return Ok(false);
    };
    if cli_locator_text(base_url) {
        return Ok(false);
    }
    let name = normalize_name_text(name)?;
    let changed = conn
        .execute(
            "UPDATE ai_providers SET detected_api_format = ? WHERE name = ? AND base_url = ? AND (detected_api_format IS NULL OR detected_api_format != ?)",
            params![detected, name, base_url, detected],
        )
        .map_err(database_error)?;
    commit(conn)?;
    if changed != 0 {
        return Ok(true);
    }
    exists(
        conn,
        "SELECT 1 FROM ai_providers WHERE name = ? AND base_url = ? AND detected_api_format = ?",
        &[&name, base_url, &detected],
    )
}

fn require_row(conn: &Connection, name: &str) -> Result<(), AppError> {
    if !exists(conn, "SELECT 1 FROM ai_providers WHERE name = ?", &[name])? {
        return Err(not_found(name));
    }
    Ok(())
}

/// `set_db_ai_provider_api_key(conn, name, api_key)`.
pub fn set_api_key(
    conn: &Connection,
    name: &str,
    api_key: Option<&str>,
) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    require_row(conn, &name)?;
    let key = api_key.and_then(|key| str_or_none(&s(key)));
    let state = if key.is_some() { "ok" } else { "missing" };
    atomically(conn, || {
        conn.execute(
            "UPDATE ai_providers SET api_key = ?, updated_at = ? WHERE name = ?",
            params![key, now_iso(), name],
        )
        .map_err(database_error)?;
        upsert_secret_ref(conn, &name, SECRET_STORE_INLINE, state, None, None).map(drop)
    })?;
    get(conn, &name)
}

/// `set_db_ai_provider_native_secret_ref(conn, name, ...)`.
pub fn set_native_secret_ref(
    conn: &Connection,
    name: &str,
    store_id: &str,
    service: &str,
    account: &str,
    state: &str,
) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    require_row(conn, &name)?;
    let (expected_service, expected_account) = secret_ref_namespace(conn, &name)?;
    if service != expected_service || account != expected_account {
        return Err(validation(
            "AI provider native secret ref is outside this project's namespace",
        )
        .with_hint(
            "Re-enter the provider API key so Kassiber can create a project-scoped native secret.",
        ));
    }
    atomically(conn, || {
        conn.execute(
            "UPDATE ai_providers SET api_key = NULL, updated_at = ? WHERE name = ?",
            params![now_iso(), name],
        )
        .map_err(database_error)?;
        upsert_secret_ref(conn, &name, store_id, state, Some(service), Some(account)).map(drop)
    })?;
    get(conn, &name)
}

/// `delete_db_ai_provider(conn, name)`.
pub fn delete(conn: &Connection, name: &str) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    require_row(conn, &name)?;
    if setting_text(conn, DEFAULT_PROVIDER_SETTING)?.as_deref() == Some(name.as_str()) {
        return Err(AppError::new(
            "conflict",
            format!("AI provider '{name}' is the stored default; clear it with `kassiber ai providers clear-default` first"),
        ));
    }
    conn.execute("DELETE FROM ai_providers WHERE name = ?", [&name])
        .map_err(database_error)?;
    commit(conn)?;
    Ok(dict([("name", s(name)), ("deleted", Value::Bool(true))]))
}

/// `set_default_ai_provider(conn, name)`.
pub fn set_default(conn: &Connection, name: &str) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    let enabled: Option<Value> = conn
        .query_row(
            "SELECT enabled FROM ai_providers WHERE name = ?",
            [&name],
            |row| Ok(value_of(row.get_ref(0)?)),
        )
        .optional()
        .map_err(database_error)?;
    let Some(enabled) = enabled else {
        return Err(not_found(&name)
            .with_hint("Use `kassiber ai providers list` to see available providers."));
    };
    if !truthy(&enabled) {
        return Err(AppError::new(
            "conflict",
            format!("AI provider '{name}' is disabled and cannot be the default"),
        )
        .with_hint(format!(
            "Enable it first with `kassiber ai providers update {name} --enable`."
        )));
    }
    set_setting(conn, DEFAULT_PROVIDER_SETTING, &name)?;
    commit(conn)?;
    Ok(dict([("default", s(name))]))
}

/// `clear_default_ai_provider(conn)`.
pub fn clear_default(conn: &Connection) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    conn.execute(
        "DELETE FROM settings WHERE key = ?",
        [DEFAULT_PROVIDER_SETTING],
    )
    .map_err(database_error)?;
    commit(conn)?;
    Ok(dict([
        ("default", Value::Null),
        ("cleared", Value::Bool(true)),
    ]))
}

/// `list_with_default(conn)`.
pub fn list_with_default(conn: &Connection) -> Result<Value, AppError> {
    let providers = list(conn)?;
    let default = default_name(conn)?;
    Ok(dict([
        (
            "providers",
            Value::List(
                providers
                    .iter()
                    .map(|provider| redact_for_output(provider, default.as_deref()))
                    .collect(),
            ),
        ),
        ("default", opt(default)),
    ]))
}

/// `acknowledge_remote_use(conn, name)`.
pub fn acknowledge(conn: &Connection, name: &str) -> Result<Value, AppError> {
    ensure_seeded(conn)?;
    let name = normalize_name_text(name)?;
    require_row(conn, &name)?;
    let timestamp = now_iso();
    conn.execute(
        "UPDATE ai_providers SET acknowledged_at = ?, updated_at = ? WHERE name = ?",
        params![timestamp, timestamp, name],
    )
    .map_err(database_error)?;
    commit(conn)?;
    Ok(dict([("name", s(name)), ("acknowledged_at", s(timestamp))]))
}

/// `contactable_cli_runtimes(conn)`: CLI runtimes some enabled and
/// acknowledged provider row allows, in locator order.
pub fn contactable_cli_runtimes(conn: &Connection) -> Result<Vec<&'static str>, AppError> {
    let mut allowed = Vec::new();
    for provider in list(conn)? {
        let Some(runtime) = cli_provider_for_locator(field(&provider, "base_url")) else {
            continue;
        };
        if allowed.contains(&runtime) || require_contactable(&provider).is_err() {
            continue;
        }
        allowed.push(runtime);
    }
    let mut ordered: Vec<&'static str> = Vec::new();
    for (_, runtime) in CLI_PROVIDERS {
        if !ordered.contains(runtime) && allowed.contains(runtime) {
            ordered.push(runtime);
        }
    }
    Ok(ordered)
}

#[cfg(test)]
pub(crate) mod tests;
