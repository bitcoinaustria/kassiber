//! The `ai.providers.*`, `ai.provider_runtime.status`, `ai.list_models` and
//! `ai.test_connection` daemon kinds, and the `ai providers` and `ai models`
//! CLI leaves (`kassiber/daemon.py`, `kassiber/cli/main.py`).
//!
//! Listing, reading and editing providers contacts nothing; the desktop's
//! keychain is reached only through [`Host::secret_store`], the daemon's
//! control channel. Only `ai.list_models`, `ai.test_connection`,
//! `ai providers test` and `ai models` contact a provider, after the
//! provider gates, through the [`Gate`] the host supplies. CLI agent
//! runtimes go through the provider broker ([`crate::broker`]), launched
//! on a process route of the same gate: `ai.provider_runtime.status`
//! without `cached_only`, and model listing and connection tests on a
//! `*-cli://default` row.

use std::time::Duration;

use kassiber_envelope::{dict, set, truthy, AppError};
pub use kassiber_net::Gate;
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

use crate::broker::{BrokerClient, Locate, Scope};
use crate::discovery::{unchecked_metadata, Cache};
use crate::providers::{self as store, cli_provider_for_locator, is_cli_locator};
use crate::pyval::{field, opt, s, str_or_none, string};
use crate::transport::{run_connection_test, ClientConfig, HttpClient};

/// The daemon kinds, in Python's `supported_kinds` order.
pub const KINDS: &[&str] = &[
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.set_api_key",
    "ai.providers.move_api_key",
    "ai.providers.delete",
    "ai.providers.set_default",
    "ai.providers.clear_default",
    "ai.providers.acknowledge",
    "ai.provider_runtime.status",
    "ai.list_models",
    "ai.test_connection",
];

/// The CLI leaves, as dotted command paths.
pub const COMMANDS: &[&str] = &[
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.delete",
    "ai.providers.set-default",
    "ai.providers.clear-default",
    "ai.providers.test",
    "ai.models",
];

const SECRET_STORE_IDS: &[&str] = &[
    "sqlcipher_inline",
    "macos_keychain",
    "windows_dpapi",
    "linux_secret_service",
];

/// What the daemon or the CLI lends a provider operation.
pub trait Host {
    /// One `supervisor.ai_secret_store.request` round trip; its `data`.
    fn secret_store(
        &mut self,
        op: &str,
        provider_name: &str,
        store_id: &str,
        service: &str,
        account: &str,
        secret: Option<&str>,
    ) -> Result<Value, AppError>;
    /// The RAM-only discovery cache of this process.
    fn discovery(&self) -> &Cache;
    /// The gate every provider request and broker launch is authorized by.
    fn gate(&self) -> Gate;
    /// How the provider broker is found: the environment overrides, else
    /// the bundled script.
    fn broker(&self) -> Locate {
        crate::broker::default_locate()
    }
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// `_coerce_args_dict(request_id, args)`.
pub fn coerce_args(args: Option<&Value>) -> Result<Value, AppError> {
    match args {
        None | Some(Value::Null) => Ok(dict::<&str>([])),
        Some(args @ Value::Dict(_)) => Ok(args.clone()),
        Some(other) => Err(validation("daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(other)))]))),
    }
}

fn present(args: &Value, key: &str) -> bool {
    kassiber_envelope::get(args, key).is_some()
}

fn name_arg(args: &Value, kind: &str) -> Result<String, AppError> {
    string(field(args, "name")).ok_or_else(|| validation(format!("{kind} requires a name string")))
}

/// `_optional_bool_arg(args, key, default)`.
fn optional_bool(args: &Value, key: &str) -> Result<bool, AppError> {
    match kassiber_envelope::get(args, key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(validation(format!("{key} must be a boolean"))
            .with_details(dict([("type", s(kassiber_envelope::type_name(other)))]))),
    }
}

/// `_ai_provider_enabled_arg` / `_ai_provider_omit_store_arg`.
fn bool_or_none(args: &Value, key: &str, kind: &str) -> Result<Value, AppError> {
    match field(args, key) {
        Value::Null => Ok(Value::Null),
        value @ Value::Bool(_) => Ok(value.clone()),
        _ => Err(validation(format!("{kind} {key} must be a boolean"))),
    }
}

/// `_ai_provider_effort_arg`.
fn effort_arg(args: &Value, kind: &str) -> Result<Value, AppError> {
    match field(args, "default_reasoning_effort") {
        Value::Null => Ok(Value::Null),
        value @ Value::Str(_) => Ok(value.clone()),
        _ => Err(validation(format!(
            "{kind} default_reasoning_effort must be a string"
        ))),
    }
}

/// `_ai_provider_format_args`.
fn format_args(args: &Value, kind: &str) -> Result<(Value, Value), AppError> {
    let mut values = Vec::new();
    for key in ["api_format", "detected_api_format"] {
        match field(args, key) {
            value @ (Value::Null | Value::Str(_)) => values.push(value.clone()),
            _ => return Err(validation(format!("{kind} {key} must be a string or null"))),
        }
    }
    Ok((values[0].clone(), values[1].clone()))
}

fn redacted(conn: &Connection, provider: &Value) -> Result<Value, AppError> {
    let default = store::default_name(conn)?;
    Ok(store::redact_for_output(provider, default.as_deref()))
}

fn bridge_enabled(args: &Value) -> bool {
    truthy(field(args, "_desktop_secret_store_bridge"))
}

/// `_provider_secret_ref_for_bridge(ctx, provider)`.
fn bridge_ref(
    conn: &Connection,
    provider: &Value,
) -> Result<(String, String, String, String), AppError> {
    let name = kassiber_envelope::py_str(field(provider, "name"));
    let name = if truthy(field(provider, "name")) {
        name
    } else {
        String::new()
    };
    let (service, account) = store::secret_ref_namespace(conn, &name)?;
    let reference = field(provider, "secret_ref");
    let text = |key: &str, fallback: &str| {
        let value = field(reference, key);
        if truthy(value) {
            kassiber_envelope::py_str(value)
        } else {
            fallback.to_owned()
        }
    };
    let store_id = text("store_id", "sqlcipher_inline");
    let state = text("state", "missing");
    if store_id == "sqlcipher_inline"
        || state != "ok"
        || text("service", "") != service
        || text("account", &name) != account
    {
        return Err(AppError::new(
            "secret_ref_unavailable",
            "AI provider secret ref is outside this project's native secret namespace",
        )
        .with_details(dict([(
            "refs",
            Value::List(vec![dict([
                ("provider_name", s(&name)),
                ("store_id", s(&store_id)),
                ("state", s("unavailable")),
            ])]),
        )]))
        .retryable(true));
    }
    Ok((name, store_id, service, account))
}

/// `_resolve_ai_provider_api_key(ctx, provider, args)`.
fn resolve_key(
    conn: &Connection,
    host: &mut dyn Host,
    provider: &Value,
    args: &Value,
) -> Result<Option<String>, AppError> {
    if !bridge_enabled(args) {
        return store::api_key_for_use(provider, Some(conn), None);
    }
    let mut resolver = |reference: &Value| -> Result<Option<String>, AppError> {
        let text = |first: &str, second: &str| {
            let value = if truthy(field(reference, first)) {
                field(reference, first)
            } else {
                field(reference, second)
            };
            if truthy(value) {
                kassiber_envelope::py_str(value)
            } else {
                String::new()
            }
        };
        let data = host.secret_store(
            "get",
            &text("provider_name", "account"),
            &text("store_id", "store_id"),
            &text("service", "service"),
            &text("account", "provider_name"),
            None,
        )?;
        let state = if truthy(field(&data, "state")) {
            kassiber_envelope::py_str(field(&data, "state"))
        } else {
            String::new()
        };
        if state != "ok" {
            return Ok(None);
        }
        Ok(string(field(&data, "secret")))
    };
    store::api_key_for_use(provider, Some(conn), Some(&mut resolver))
}

/// `_ai_provider_has_stored_api_key(provider)`.
fn has_stored_key(provider: &Value) -> bool {
    let reference = field(provider, "secret_ref");
    let store_id = field(reference, "store_id");
    if !truthy(store_id) || *store_id == s("sqlcipher_inline") {
        return str_or_none(field(provider, "api_key")).is_some();
    }
    *field(reference, "state") == s("ok")
}

/// `_validate_ai_provider_secret_store_id(store_id)`.
fn store_id_arg(store_id: &str) -> Result<String, AppError> {
    let store_id = kassiber_pytext::strip(store_id);
    if !SECRET_STORE_IDS.contains(&store_id) {
        return Err(validation("unsupported AI provider secret store")
            .with_details(dict([("store_id", s(store_id))])));
    }
    Ok(store_id.to_owned())
}

fn native_unavailable(hint: bool) -> AppError {
    let error = AppError::new(
        "secret_store_unavailable",
        "native AI provider secret storage is available only in the desktop app",
    )
    .retryable(true);
    if hint {
        error.with_hint("Use SQLCipher inline storage or reopen this project in the desktop app.")
    } else {
        error
    }
}

/// `_set_ai_provider_key_with_selected_store(ctx, args, name, api_key)`.
fn set_key(
    conn: &Connection,
    host: &mut dyn Host,
    args: &Value,
    name: &str,
    key: Option<&str>,
) -> Result<Value, AppError> {
    let target = match field(args, "store_id") {
        Value::Str(text) if text.code_points().next().is_some() => crate::pyval::lossy(text),
        _ => match field(args, "_desktop_secret_store_default") {
            Value::Str(text) if text.code_points().next().is_some() => crate::pyval::lossy(text),
            _ => "sqlcipher_inline".into(),
        },
    };
    let target = store_id_arg(&target)?;
    if target == "sqlcipher_inline" {
        return store::set_api_key(conn, name, key);
    }
    if !bridge_enabled(args) {
        return Err(native_unavailable(true));
    }
    let provider = store::get(conn, name)?;
    let provider_name = kassiber_envelope::py_str(field(&provider, "name"));
    let (service, account) = store::secret_ref_namespace(conn, &provider_name)?;
    match key {
        None => {
            host.secret_store("delete", &provider_name, &target, &service, &account, None)?;
            store::set_api_key(conn, &provider_name, None)
        }
        Some(key) => {
            host.secret_store(
                "set",
                &provider_name,
                &target,
                &service,
                &account,
                Some(key),
            )?;
            store::set_native_secret_ref(conn, &provider_name, &target, &service, &account, "ok")
        }
    }
}

/// `_move_ai_provider_key(ctx, args, name, target_store_id, api_key)`.
fn move_key(
    conn: &Connection,
    host: &mut dyn Host,
    args: &Value,
    name: &str,
    target: &str,
    key: Option<&str>,
) -> Result<Value, AppError> {
    let provider = store::get(conn, name)?;
    let provider_name = kassiber_envelope::py_str(field(&provider, "name"));
    let reference = field(&provider, "secret_ref");
    let current = if truthy(field(reference, "store_id")) {
        kassiber_envelope::py_str(field(reference, "store_id"))
    } else {
        "sqlcipher_inline".into()
    };
    let target = store_id_arg(target)?;
    let mut moving = key
        .and_then(|key| str_or_none(&s(key)))
        .or_else(|| str_or_none(field(&provider, "api_key")));
    if target != "sqlcipher_inline" && !bridge_enabled(args) {
        return Err(native_unavailable(false));
    }
    if moving.is_none() && current != "sqlcipher_inline" {
        moving = resolve_key(conn, host, &provider, args)?;
    }
    let Some(moving) = moving else {
        let reference = bridge_ref(conn, &provider).map(|(name, store_id, service, account)| {
            dict([
                ("provider_name", s(name)),
                ("store_id", s(store_id)),
                ("service", s(service)),
                ("account", s(account)),
                ("state", s("ok")),
            ])
        })?;
        return Err(AppError::new(
            "secret_ref_unavailable",
            "AI provider key must be re-entered before it can be moved",
        )
        .with_hint("Re-enter the provider API key in Settings, then retry the storage move.")
        .with_details(dict([("refs", Value::List(vec![reference]))]))
        .retryable(true));
    };
    let (service, account) = store::secret_ref_namespace(conn, &provider_name)?;
    if target == "sqlcipher_inline" {
        let updated = store::set_api_key(conn, &provider_name, Some(&moving))?;
        if current != "sqlcipher_inline" && bridge_enabled(args) {
            host.secret_store("delete", &provider_name, &current, &service, &account, None)?;
        }
        return Ok(updated);
    }
    host.secret_store(
        "set",
        &provider_name,
        &target,
        &service,
        &account,
        Some(&moving),
    )?;
    store::set_native_secret_ref(conn, &provider_name, &target, &service, &account, "ok")
}

/// `_delete_native_ai_provider_secret(ctx, args, provider)`: best effort.
fn delete_native_secret(
    conn: &Connection,
    host: &mut dyn Host,
    args: &Value,
    provider: &Value,
) -> Result<(), AppError> {
    let reference = field(provider, "secret_ref");
    let store_id = if truthy(field(reference, "store_id")) {
        kassiber_envelope::py_str(field(reference, "store_id"))
    } else {
        "sqlcipher_inline".into()
    };
    if store_id == "sqlcipher_inline" || !bridge_enabled(args) {
        return Ok(());
    }
    let name = kassiber_envelope::py_str(field(provider, "name"));
    let (service, account) = store::secret_ref_namespace(conn, &name)?;
    let label = if truthy(field(provider, "name")) {
        name
    } else {
        account.clone()
    };
    let _ = host.secret_store("delete", &label, &store_id, &service, &account, None);
    Ok(())
}

/// `_refresh_ai_provider_native_secret_states(ctx, args)`: ask the
/// desktop whether native secrets still exist; a missing one is marked.
fn refresh_native_states(
    conn: &Connection,
    host: &mut dyn Host,
    args: &Value,
) -> Result<(), AppError> {
    if !bridge_enabled(args) {
        return Ok(());
    }
    for provider in store::list(conn)? {
        let reference = field(&provider, "secret_ref");
        let store_id = field(reference, "store_id");
        if !truthy(store_id)
            || *store_id == s("sqlcipher_inline")
            || *field(reference, "state") != s("ok")
        {
            continue;
        }
        let Ok((name, store_id, service, account)) = bridge_ref(conn, &provider) else {
            continue;
        };
        let Ok(data) = host.secret_store("exists", &name, &store_id, &service, &account, None)
        else {
            continue;
        };
        if *field(&data, "state") == s("missing") {
            store::mark_secret_ref_state(conn, &name, "missing")?;
        }
    }
    Ok(())
}

/// The HTTP client for a provider row or a tested form.
fn http_client(
    host: &dyn Host,
    base_url: &str,
    key: Option<String>,
    kind: Option<&str>,
    timeout: Duration,
) -> HttpClient {
    let mut config = ClientConfig::new(base_url);
    config.api_key = key;
    config.timeout = timeout;
    config.on_device = kind == Some("local");
    HttpClient::new(config, host.gate())
}

/// The cache key of a discovery probe, unambiguous for any provider name.
fn cache_key(parts: &[&str]) -> String {
    kassiber_pyjson::dumps(
        &Value::List(parts.iter().map(s).collect()),
        &Default::default(),
    )
    .unwrap_or_default()
}

fn with_metadata(
    mut payload: Vec<(&'static str, Value)>,
    metadata: Vec<(&'static str, Value)>,
) -> Value {
    payload.extend(metadata);
    dict(payload)
}

/// One daemon kind; the payload of its success envelope.
pub fn daemon(
    conn: &Connection,
    host: &mut dyn Host,
    kind: &str,
    args: Option<&Value>,
) -> Result<Value, AppError> {
    let args = coerce_args(args)?;
    let cache = |host: &dyn Host| host.discovery().invalidate();
    match kind {
        "ai.providers.list" => {
            refresh_native_states(conn, host, &args)?;
            store::list_with_default(conn)
        }
        "ai.providers.create" => {
            let (Some(name), Some(base_url)) = (
                string(field(&args, "name")),
                string(field(&args, "base_url")),
            ) else {
                return Err(validation(
                    "ai.providers.create requires name and base_url strings",
                ));
            };
            if present(&args, "api_key") {
                return Err(validation("ai.providers.create does not accept api_key; use ai.providers.set_api_key")
                    .with_hint("Save provider metadata first, then send the key through ai.providers.set_api_key."));
            }
            let display = match field(&args, "display_name") {
                Value::Null => None,
                Value::Str(text) => Some(crate::pyval::lossy(text)),
                _ => {
                    return Err(validation(
                        "ai.providers.create display_name must be a string",
                    ))
                }
            };
            let enabled = bool_or_none(&args, "enabled", kind)?;
            let effort = effort_arg(&args, kind)?;
            let (api_format, detected) = format_args(&args, kind)?;
            let omit = bool_or_none(&args, "omit_store_field", kind)?;
            let created = store::create(
                conn,
                store::Create {
                    name,
                    base_url,
                    display_name: display,
                    api_key: None,
                    default_model: field(&args, "default_model").clone(),
                    kind: field(&args, "kind").clone(),
                    notes: field(&args, "notes").clone(),
                    acknowledged: truthy(field(&args, "acknowledged")),
                    enabled: enabled != Value::Bool(false),
                    default_reasoning_effort: effort,
                    api_format,
                    detected_api_format: detected,
                    omit_store_field: omit == Value::Bool(true),
                },
            )?;
            cache(host);
            redacted(conn, &created)
        }
        "ai.providers.update" => {
            let name = name_arg(&args, kind)?;
            let clear = match field(&args, "clear") {
                Value::Null => Vec::new(),
                Value::List(items) => items.iter().map(kassiber_envelope::py_str).collect(),
                _ => return Err(validation("ai.providers.update clear must be a list")),
            };
            if present(&args, "api_key") {
                return Err(validation("ai.providers.update does not accept api_key; use ai.providers.set_api_key")
                    .with_hint("Update provider metadata separately, then send the key through ai.providers.set_api_key."));
            }
            let display = match field(&args, "display_name") {
                value @ (Value::Null | Value::Str(_)) => value.clone(),
                _ => {
                    return Err(validation(
                        "ai.providers.update display_name must be a string",
                    ))
                }
            };
            let enabled = bool_or_none(&args, "enabled", kind)?;
            let effort = effort_arg(&args, kind)?;
            let (api_format, detected) = format_args(&args, kind)?;
            let omit = bool_or_none(&args, "omit_store_field", kind)?;
            let updated = store::update(
                conn,
                &name,
                store::Update {
                    display_name: display,
                    base_url: field(&args, "base_url").clone(),
                    api_key: Value::Null,
                    default_model: field(&args, "default_model").clone(),
                    kind: field(&args, "kind").clone(),
                    notes: field(&args, "notes").clone(),
                    default_reasoning_effort: effort,
                    enabled,
                    api_format,
                    detected_api_format: detected,
                    omit_store_field: omit,
                    clear: clear.clone(),
                    acknowledged: Some(truthy(field(&args, "acknowledged"))),
                    acknowledge_clear: Some(truthy(field(&args, "acknowledge_clear"))),
                },
            )?;
            // The enable switch, default effort, wire format and store
            // choice do not change what a provider lists.
            let preference_only = crate::pyval::entries(&args).all(|(key, _)| {
                [
                    "name",
                    "enabled",
                    "default_reasoning_effort",
                    "api_format",
                    "detected_api_format",
                    "omit_store_field",
                    "clear",
                ]
                .contains(&key.as_str())
            }) && clear
                .iter()
                .all(|field| field == "default_reasoning_effort" || field == "detected_api_format");
            if !preference_only {
                cache(host);
            }
            redacted(conn, &updated)
        }
        "ai.providers.set_api_key" => {
            let name = name_arg(&args, kind)?;
            let key = match field(&args, "api_key") {
                Value::Null => None,
                Value::Str(text) => Some(crate::pyval::lossy(text)),
                _ => {
                    return Err(validation(
                        "ai.providers.set_api_key api_key must be a string or null",
                    ))
                }
            };
            let updated = set_key(conn, host, &args, &name, key.as_deref())?;
            cache(host);
            redacted(conn, &updated)
        }
        "ai.providers.move_api_key" => {
            let name = name_arg(&args, kind)?;
            let target = match field(&args, "store_id") {
                Value::Str(text)
                    if !kassiber_pytext::strip(&crate::pyval::lossy(text)).is_empty() =>
                {
                    crate::pyval::lossy(text)
                }
                _ => return Err(validation("ai.providers.move_api_key requires store_id")),
            };
            let key = match field(&args, "api_key") {
                Value::Null => None,
                Value::Str(text) => Some(crate::pyval::lossy(text)),
                _ => {
                    return Err(validation(
                        "ai.providers.move_api_key api_key must be a string or null",
                    ))
                }
            };
            let updated = move_key(conn, host, &args, &name, &target, key.as_deref())?;
            cache(host);
            redacted(conn, &updated)
        }
        "ai.providers.delete" => {
            let name = name_arg(&args, kind)?;
            let provider = store::get(conn, &name)?;
            delete_native_secret(conn, host, &args, &provider)?;
            let deleted = store::delete(conn, &name)?;
            cache(host);
            Ok(deleted)
        }
        "ai.providers.set_default" => store::set_default(conn, &name_arg(&args, kind)?),
        "ai.providers.clear_default" => store::clear_default(conn),
        "ai.providers.get" => {
            let name = name_arg(&args, kind)?;
            refresh_native_states(conn, host, &args)?;
            redacted(conn, &store::get(conn, &name)?)
        }
        "ai.providers.acknowledge" => store::acknowledge(conn, &name_arg(&args, kind)?),
        "ai.provider_runtime.status" => runtime_status(conn, host, &args),
        "ai.list_models" => list_models(conn, host, &args),
        "ai.test_connection" => test_connection(conn, host, &args),
        other => Err(AppError::new(
            "unsupported_kind",
            format!("unsupported AI provider kind {other}"),
        )),
    }
}

/// `ai.provider_runtime.status`: CLI runtimes. A display read
/// (`cached_only`, or nothing a provider row allows) never probes.
fn runtime_status(conn: &Connection, host: &mut dyn Host, args: &Value) -> Result<Value, AppError> {
    let refresh = optional_bool(args, "refresh")?;
    let cached_only = optional_bool(args, "cached_only")?;
    if refresh && cached_only {
        return Err(validation(
            "ai.provider_runtime.status refresh and cached_only are exclusive",
        ));
    }
    let provider_name = match field(args, "provider") {
        Value::Null => None,
        Value::Str(text) => Some(crate::pyval::lossy(text)),
        _ => {
            return Err(validation(
                "ai.provider_runtime.status provider must be a string",
            ))
        }
    };
    let expected = field(args, "base_url");
    if *expected != Value::Null && provider_name.is_none() {
        return Err(validation(
            "ai.provider_runtime.status base_url requires provider",
        ));
    }
    let mut scoped = None;
    let mut probe: Vec<&'static str> = Vec::new();
    let mut single = None;
    let (key, nothing_to_probe) = match &provider_name {
        None => {
            // Only runtimes an enabled, acknowledged row allows; a seeded
            // or switched-off CLI is never started.
            let allowed = store::contactable_cli_runtimes(conn)?;
            let mut parts = vec!["runtime", "*"];
            parts.extend(allowed.iter().copied());
            probe = allowed;
            (cache_key(&parts), probe.is_empty())
        }
        Some(name) => {
            let provider = store::get(conn, name)?;
            store::require_endpoint(&provider, expected)?;
            let Some(runtime) = cli_provider_for_locator(field(&provider, "base_url")) else {
                return Err(validation(format!(
                    "AI provider '{}' is not a CLI provider",
                    kassiber_envelope::py_str(field(&provider, "name"))
                ))
                .with_hint("Use ai.list_models to check an HTTP provider."));
            };
            scoped = Some(field(&provider, "name").clone());
            if !cached_only {
                store::require_contactable(&provider)?;
            }
            // Its probe starts only that runtime's CLI.
            single = Some(runtime);
            (cache_key(&["runtime", runtime]), false)
        }
    };
    let mut payload = if cached_only || nothing_to_probe {
        match host.discovery().peek(&key) {
            Some(snapshot) => with_metadata(
                vec![("providers", snapshot.value.clone())],
                snapshot.metadata(),
            ),
            None => with_metadata(
                vec![("providers", Value::List(Vec::new()))],
                unchecked_metadata(),
            ),
        }
    } else {
        let gate = host.gate();
        let locate = host.broker();
        let snapshot = host.discovery().get(&key, refresh, || {
            let scope = match single {
                Some(runtime) => Scope::One(runtime),
                None => Scope::Many(&probe),
            };
            crate::broker::runtime_status(&gate, &locate, scope)
        })?;
        with_metadata(
            vec![("providers", snapshot.value.clone())],
            snapshot.metadata(),
        )
    };
    if let Some(name) = scoped {
        set(&mut payload, "provider", name);
    }
    Ok(payload)
}

/// `ai.list_models`: probes on a cold or expired cache, or on `refresh`.
fn list_models(conn: &Connection, host: &mut dyn Host, args: &Value) -> Result<Value, AppError> {
    let name = match field(args, "provider") {
        Value::Null => None,
        Value::Str(text) => Some(crate::pyval::lossy(text)),
        _ => return Err(validation("ai.list_models provider must be a string")),
    };
    let provider = store::resolve(conn, name.as_deref())?;
    store::require_endpoint(&provider, field(args, "base_url"))?;
    store::require_contactable(&provider)?;
    let base_url = kassiber_envelope::py_str(field(&provider, "base_url"));
    let key = resolve_key(conn, host, &provider, args)?;
    let provider_name = kassiber_envelope::py_str(field(&provider, "name"));
    let refresh = optional_bool(args, "refresh")?;
    let kind = string(field(&provider, "kind"));
    let cli = is_cli_locator(field(&provider, "base_url"));
    let host: &dyn Host = host;
    let snapshot = host.discovery().get(
        &cache_key(&["models", &provider_name, &base_url]),
        refresh,
        || {
            if cli {
                return broker_client(host, &base_url, crate::transport::DEFAULT_TIMEOUT)?
                    .list_models()
                    .map(Value::List);
            }
            http_client(
                host,
                &base_url,
                key,
                kind.as_deref(),
                crate::transport::DEFAULT_TIMEOUT,
            )
            .list_models(false)
            .map(Value::List)
        },
    )?;
    Ok(with_metadata(
        vec![
            ("provider", field(&provider, "name").clone()),
            ("models", snapshot.value.clone()),
        ],
        snapshot.metadata(),
    ))
}

/// `ai.test_connection`: caller-supplied form values; only a detected
/// format is ever persisted, onto the saved provider that was tested.
fn test_connection(
    conn: &Connection,
    host: &mut dyn Host,
    args: &Value,
) -> Result<Value, AppError> {
    let raw = field(args, "base_url");
    let canonical = match raw {
        Value::Str(text) if !kassiber_pytext::strip(&crate::pyval::lossy(text)).is_empty() => {
            store::normalize_base_url(raw)?
        }
        _ => {
            return Err(validation(
                "ai.test_connection requires a non-empty base_url string",
            ))
        }
    };
    let saved = |name: &Value| -> Option<Value> {
        match name {
            Value::Str(text) if !kassiber_pytext::strip(&crate::pyval::lossy(text)).is_empty() => {
                store::get(conn, &crate::pyval::lossy(text)).ok()
            }
            _ => None,
        }
    };
    let named = saved(field(args, "provider"));
    if let Some(row) = &named {
        // Testing a saved provider's form still contacts it.
        store::require_enabled(row)?;
    }
    let (format_arg, _) = format_args(args, "ai.test_connection")?;
    let mut model = match field(args, "model") {
        Value::Null => None,
        Value::Str(text) => Some(crate::pyval::lossy(text)),
        _ => return Err(validation("ai.test_connection model must be a string")),
    };
    let mut key = match field(args, "api_key") {
        Value::Null => String::new(),
        value @ Value::Str(_) => str_or_none(value).unwrap_or_default(),
        _ => return Err(validation("ai.test_connection api_key must be a string")),
    };
    if key.is_empty() {
        if let Some(stored) = saved(field(args, "provider")) {
            // Stored keys stay bound to their origin.
            let same_origin = store::base_url_origin(&s(&canonical))
                == store::base_url_origin(&s(store::normalize_base_url(field(
                    &stored, "base_url",
                ))?));
            if has_stored_key(&stored) && !same_origin {
                return Err(validation("ai.test_connection cannot reuse a stored API key for a different origin")
                    .with_hint("Save the provider URL first, then test it, so stored credentials are only sent to their configured origin."));
            }
            if same_origin {
                key = resolve_key(conn, host, &stored, args)?.unwrap_or_default();
            }
        }
    }
    let mut kind = match field(args, "kind") {
        Value::Null => None,
        value => match string(value).filter(|k| store::PROVIDER_KINDS.contains(&k.as_str())) {
            Some(kind) => Some(kind),
            None => {
                return Err(validation(format!(
                    "ai.test_connection kind must be one of: {}",
                    store::PROVIDER_KINDS.join(", ")
                )))
            }
        },
    };
    if kind.is_none() {
        if let Some(row) = saved(field(args, "provider")) {
            kind = string(field(&row, "kind"));
        }
    }
    let kind = kind.unwrap_or_else(|| store::infer_kind(&canonical).to_owned());
    let acknowledged = match field(args, "acknowledged") {
        Value::Null => false,
        Value::Bool(value) => *value,
        _ => {
            return Err(validation(
                "ai.test_connection acknowledged must be a boolean",
            ))
        }
    };
    store::require_test_target_acknowledged(&canonical, Some(&kind), acknowledged, named.as_ref())?;
    let cli = cli_provider_for_locator(&s(&canonical)).is_some();
    let api_format = store::normalize_api_format(if format_arg != Value::Null {
        &format_arg
    } else {
        named
            .as_ref()
            .map_or(&Value::Null, |row| field(row, "api_format"))
    })?;
    if cli && !matches!(&format_arg, Value::Null) && format_arg != s("auto") {
        return Err(validation("CLI agent providers do not take an API format")
            .with_hint("Leave the API format on auto for a *-cli://default locator."));
    }
    if model.is_none() {
        if let Some(row) = &named {
            model = match field(row, "default_model") {
                Value::Str(text) => Some(crate::pyval::lossy(text)),
                _ => None,
            };
        }
    }
    let omit = match bool_or_none(args, "omit_store_field", "ai.test_connection")? {
        Value::Bool(omit) => omit,
        _ => {
            let same_address = named
                .as_ref()
                .is_some_and(|row| kassiber_envelope::py_str(field(row, "base_url")) == canonical);
            !cli && same_address && truthy(field(named.as_ref().unwrap(), "omit_store_field"))
        }
    };
    if cli && omit {
        return Err(validation(
            "CLI agent providers do not take omit_store_field",
        ));
    }
    if cli {
        let models = broker_client(host, &canonical, Duration::from_secs(10))?.list_models()?;
        let mut payload = vec![("base_url".to_owned(), s(&canonical))];
        payload.extend(cli_connection_test(models));
        payload.push(("detection_saved".into(), Value::Bool(false)));
        return Ok(dict(payload));
    }
    // Discovery fails fast; the one test request has its own, longer
    // timeout so a local model can load.
    let mut config = ClientConfig::new(&canonical);
    config.api_key = (!key.is_empty()).then_some(key);
    config.timeout = Duration::from_secs(10);
    config.on_device = kind == "local";
    config.api_format = api_format;
    config.omit_store_field = omit;
    let mut client = HttpClient::new(config, host.gate());
    let result = run_connection_test(&mut client, model.as_deref())?;
    let mut saved_detection = false;
    if let (Some(detected), Some(row)) = (string(field(&result, "detected_api_format")), &named) {
        if store::normalize_base_url(field(row, "base_url"))? == canonical {
            saved_detection = store::record_detected_api_format(
                conn,
                &kassiber_envelope::py_str(field(row, "name")),
                &kassiber_envelope::py_str(field(row, "base_url")),
                &detected,
            )?;
        }
    }
    let mut payload = vec![("base_url".to_owned(), s(&canonical))];
    payload.extend(crate::pyval::entries(&result).map(|(k, v)| (k, v.clone())));
    payload.push(("detection_saved".into(), Value::Bool(saved_detection)));
    Ok(dict(payload))
}

/// The arguments of one `ai` CLI leaf, already read from argparse: the
/// API key from its safe channel, never argv echoed anywhere.
pub fn cli(
    conn: &Connection,
    host: &mut dyn Host,
    command: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let text = |key: &str| string(field(args, key));
    let name = || text("name").unwrap_or_default();
    match command {
        "ai.providers.list" => store::list_with_default(conn),
        "ai.providers.get" => redacted(conn, &store::get(conn, &name())?),
        "ai.providers.create" => {
            let created = store::create(
                conn,
                store::Create {
                    name: name(),
                    base_url: text("base_url").unwrap_or_default(),
                    display_name: text("display_name"),
                    api_key: text("api_key"),
                    default_model: field(args, "default_model").clone(),
                    kind: field(args, "kind").clone(),
                    notes: field(args, "notes").clone(),
                    acknowledged: truthy(field(args, "acknowledge")),
                    enabled: !truthy(field(args, "disabled")),
                    default_reasoning_effort: field(args, "default_reasoning_effort").clone(),
                    api_format: field(args, "api_format").clone(),
                    detected_api_format: Value::Null,
                    omit_store_field: truthy(field(args, "omit_store_field")),
                },
            )?;
            redacted(conn, &created)
        }
        "ai.providers.update" => {
            let clear = match field(args, "clear") {
                Value::List(items) => items.iter().map(kassiber_envelope::py_str).collect(),
                _ => Vec::new(),
            };
            let updated = store::update(
                conn,
                &name(),
                store::Update {
                    display_name: field(args, "display_name").clone(),
                    base_url: field(args, "base_url").clone(),
                    api_key: opt(text("api_key")),
                    default_model: field(args, "default_model").clone(),
                    kind: field(args, "kind").clone(),
                    notes: field(args, "notes").clone(),
                    default_reasoning_effort: field(args, "default_reasoning_effort").clone(),
                    enabled: field(args, "enabled").clone(),
                    api_format: field(args, "api_format").clone(),
                    detected_api_format: Value::Null,
                    omit_store_field: field(args, "omit_store_field").clone(),
                    clear,
                    acknowledged: Some(truthy(field(args, "acknowledge"))),
                    acknowledge_clear: Some(truthy(field(args, "revoke_acknowledge"))),
                },
            )?;
            redacted(conn, &updated)
        }
        "ai.providers.delete" => store::delete(conn, &name()),
        "ai.providers.set-default" => store::set_default(conn, &name()),
        "ai.providers.clear-default" => store::clear_default(conn),
        "ai.providers.test" => cli_test(conn, host, &name(), text("model")),
        "ai.models" => {
            let provider = store::resolve(conn, text("provider").as_deref())?;
            // Listing contacts the provider and sends its key: the same gate
            // as `ai.list_models`.
            store::require_contactable(&provider)?;
            let key = store::api_key_for_use(&provider, None, None)?;
            let base_url = kassiber_envelope::py_str(field(&provider, "base_url"));
            let models = if is_cli_locator(field(&provider, "base_url")) {
                broker_client(host, &base_url, crate::transport::DEFAULT_TIMEOUT)?.list_models()?
            } else {
                let kind = string(field(&provider, "kind"));
                http_client(
                    host,
                    &base_url,
                    key,
                    kind.as_deref(),
                    crate::transport::DEFAULT_TIMEOUT,
                )
                .list_models(false)?
            };
            Ok(dict([
                ("provider", field(&provider, "name").clone()),
                ("models", Value::List(models)),
            ]))
        }
        other => Err(AppError::new(
            "unknown_command",
            format!("unknown AI command {other}"),
        )),
    }
}

/// `_test_ai_provider(conn, name, model=...)`.
fn cli_test(
    conn: &Connection,
    host: &mut dyn Host,
    name: &str,
    model: Option<String>,
) -> Result<Value, AppError> {
    let provider = store::get(conn, name)?;
    store::require_contactable(&provider)?;
    let key = store::api_key_for_use(&provider, None, None)?;
    let base_url = kassiber_envelope::py_str(field(&provider, "base_url"));
    if is_cli_locator(field(&provider, "base_url")) {
        // A listing alone: the probe is skipped, so no model is required and
        // no format is detected.
        let models = broker_client(host, &base_url, Duration::from_secs(10))?.list_models()?;
        let mut payload = vec![
            ("provider".to_owned(), field(&provider, "name").clone()),
            ("base_url".to_owned(), field(&provider, "base_url").clone()),
        ];
        payload.extend(cli_connection_test(models));
        payload.push(("detection_saved".into(), Value::Bool(false)));
        return Ok(dict(payload));
    }
    let mut config = ClientConfig::new(&base_url);
    config.api_key = key;
    config.timeout = Duration::from_secs(10);
    config.on_device = *field(&provider, "kind") == s("local");
    config.api_format = string(field(&provider, "api_format")).unwrap_or_else(|| "auto".into());
    config.omit_store_field = truthy(field(&provider, "omit_store_field"));
    let mut client = HttpClient::new(config, host.gate());
    let model =
        model
            .filter(|m| !m.is_empty())
            .or_else(|| match field(&provider, "default_model") {
                Value::Str(text) if text.code_points().next().is_some() => {
                    Some(crate::pyval::lossy(text))
                }
                _ => None,
            });
    let result = run_connection_test(&mut client, model.as_deref())?;
    let provider_name = kassiber_envelope::py_str(field(&provider, "name"));
    if *field(field(&result, "probe"), "reason") == s("no_model") {
        return Err(AppError::new(
            "ai_model_required",
            format!(
                "AI provider '{provider_name}' lists {} models, but no model was given to test a reply with",
                kassiber_envelope::py_str(field(&result, "model_count"))
            ),
        )
        .with_details(dict([
            ("reason", s("no_model")),
            ("model_count", field(&result, "model_count").clone()),
        ]))
        .with_hint(format!(
            "Pass --model, or set one with `kassiber ai providers update {provider_name} --default-model <id>`."
        )));
    }
    let saved = match string(field(&result, "detected_api_format")) {
        Some(detected) if !detected.is_empty() => {
            store::record_detected_api_format(conn, &provider_name, &base_url, &detected)?
        }
        _ => false,
    };
    let mut payload = vec![
        ("provider".to_owned(), field(&provider, "name").clone()),
        ("base_url".to_owned(), field(&provider, "base_url").clone()),
    ];
    payload.extend(crate::pyval::entries(&result).map(|(k, v)| (k, v.clone())));
    payload.push(("detection_saved".into(), Value::Bool(saved)));
    Ok(dict(payload))
}

/// The gate the product uses: the persisted offline switch, read afresh
/// before every connection.
/// `ai_client_for_locator(...)` for a CLI locator.
fn broker_client(
    host: &dyn Host,
    locator: &str,
    timeout: Duration,
) -> Result<BrokerClient, AppError> {
    Ok(BrokerClient::new(locator, timeout, host.gate())?.with_locate(host.broker()))
}

/// `run_connection_test(client, http=False)`: the listing, with the probe
/// skipped because a CLI provider has no HTTP format to detect.
fn cli_connection_test(models: Vec<Value>) -> Vec<(String, Value)> {
    let binary_presence = models
        .iter()
        .any(|row| *field(row, "check_kind") == s("binary_presence"));
    vec![
        ("model_count".into(), Value::from(models.len())),
        ("models".into(), Value::List(models)),
        (
            "check_kind".into(),
            s(if binary_presence {
                "binary_presence"
            } else {
                "models"
            }),
        ),
        ("api_format".into(), Value::Null),
        ("detected_api_format".into(), Value::Null),
        ("tools_accepted".into(), Value::Null),
        ("store_accepted".into(), Value::Null),
        (
            "probe".into(),
            dict([
                ("status", s("skipped")),
                ("reason", s("cli_provider")),
                ("model", Value::Null),
                ("finish_reason", Value::Null),
            ]),
        ),
    ]
}

pub fn system_gate() -> Gate {
    use kassiber_net::{Ledger, Offline};
    let gate = Gate::new(
        || match kassiber_paths::Context::from_env()
            .ok()
            .and_then(|c| kassiber_setup::offline::preference_path(&c).ok())
        {
            Some(path) if kassiber_setup::offline::enabled(&path) => Offline::Offline,
            Some(_) => Offline::Online,
            None => Offline::Unavailable,
        },
        Ledger::default(),
    );
    // Test builds: the one fake broker an armed observer admits.
    #[cfg(feature = "test-harness")]
    let gate = match std::env::var_os("KASSIBER_RUST_TEST_AI_BROKER_FIXTURE") {
        Some(script) => gate.with_broker_fixture(script.into()),
        None => gate,
    };
    gate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::tests::{book, text};

    /// A desktop keychain in RAM, recording each round trip.
    #[derive(Default)]
    struct Desk {
        secrets: Vec<(String, String)>,
        calls: Vec<String>,
        fail: bool,
        cache: Cache,
        broker: Option<(Gate, Locate)>,
    }

    /// A Python script run as the broker, logging each request it reads.
    struct FakeBroker {
        dir: std::path::PathBuf,
        script: std::path::PathBuf,
        gate: Gate,
    }

    impl FakeBroker {
        fn new(body: &str) -> Self {
            static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "kassiber-service-broker-{}-{}",
                std::process::id(),
                COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let script = dir.join("broker.py");
            let log = dir.join("requests.jsonl");
            let logged = body.replacen(
                "request = json.loads(sys.stdin.readline())\n",
                &format!(
                    "line = sys.stdin.readline()\nopen({:?}, 'a').write(line)\nrequest = json.loads(line)\n",
                    log.to_str().unwrap()
                ),
                1,
            );
            std::fs::write(&script, logged).unwrap();
            let gate = Gate::new(|| kassiber_net::Offline::Online, Default::default())
                .with_test_observer(false)
                .with_broker_fixture(script.clone());
            Self { dir, script, gate }
        }

        fn host(&self) -> (Gate, Locate) {
            let program = kassiber_net::BrokerProgram::new(
                kassiber_net::which("python3"),
                self.script.clone(),
            );
            (
                self.gate.clone(),
                std::sync::Arc::new(move || program.clone()),
            )
        }

        fn launches(&self) -> usize {
            self.gate.attempts().records.len()
        }

        fn requests(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.join("requests.jsonl"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    impl Drop for FakeBroker {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Host for Desk {
        fn secret_store(
            &mut self,
            op: &str,
            name: &str,
            store_id: &str,
            _service: &str,
            account: &str,
            secret: Option<&str>,
        ) -> Result<Value, AppError> {
            self.calls.push(format!("{op}:{name}:{store_id}:{account}"));
            if self.fail {
                return Err(AppError::new(
                    "secret_ref_unavailable",
                    "desktop secret store did not answer",
                ));
            }
            let found = self.secrets.iter().position(|(a, _)| a == account);
            Ok(match op {
                "set" => {
                    self.secrets.retain(|(a, _)| a != account);
                    self.secrets.push((account.into(), secret.unwrap().into()));
                    dict([("state", s("ok"))])
                }
                "get" => match found {
                    Some(i) => dict([("state", s("ok")), ("secret", s(&self.secrets[i].1))]),
                    None => dict([("state", s("missing"))]),
                },
                "exists" => dict([("state", s(if found.is_some() { "ok" } else { "missing" }))]),
                _ => {
                    self.secrets.retain(|(a, _)| a != account);
                    dict([("state", s("missing"))])
                }
            })
        }
        fn discovery(&self) -> &Cache {
            &self.cache
        }
        fn gate(&self) -> Gate {
            match &self.broker {
                Some((gate, _)) => gate.clone(),
                None => panic!("provider configuration must not reach the network"),
            }
        }
        fn broker(&self) -> Locate {
            match &self.broker {
                Some((_, locate)) => locate.clone(),
                None => panic!("provider configuration must not start the broker"),
            }
        }
    }

    fn call(conn: &Connection, host: &mut Desk, kind: &str, args: &str) -> Result<Value, AppError> {
        daemon(
            conn,
            host,
            kind,
            Some(&kassiber_pytext::loads(args).unwrap()),
        )
    }

    #[test]
    fn native_keys_move_through_the_desktop_keychain_only() {
        let book = book();
        let mut desk = Desk::default();
        call(
            &book.conn,
            &mut desk,
            "ai.providers.create",
            r#"{"name": "cloud", "base_url": "https://c.test/v1"}"#,
        )
        .unwrap();
        // Without the bridge a native store is refused before any round trip.
        let error = call(
            &book.conn,
            &mut desk,
            "ai.providers.set_api_key",
            r#"{"name": "cloud", "api_key": "sk-n", "store_id": "macos_keychain"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, "secret_store_unavailable");
        assert!(desk.calls.is_empty());
        let row = call(&book.conn, &mut desk, "ai.providers.set_api_key", r#"{"name": "cloud", "api_key": "sk-native", "store_id": "macos_keychain", "_desktop_secret_store_bridge": true}"#).unwrap();
        assert_eq!(
            *field(&row, "secret_ref"),
            dict([("store_id", s("macos_keychain")), ("state", s("ok"))])
        );
        let raw: Option<String> = book
            .conn
            .query_row(
                "SELECT api_key FROM ai_providers WHERE name='cloud'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, None);
        // A display read asks whether it still exists, and nothing else.
        call(
            &book.conn,
            &mut desk,
            "ai.providers.list",
            r#"{"_desktop_secret_store_bridge": true}"#,
        )
        .unwrap();
        assert_eq!(
            desk.calls,
            [
                "set:cloud:macos_keychain:cloud",
                "exists:cloud:macos_keychain:cloud"
            ]
        );
        // Moving it inline reads it back, stores it, then deletes the native copy.
        let row = call(&book.conn, &mut desk, "ai.providers.move_api_key", r#"{"name": "cloud", "store_id": "sqlcipher_inline", "_desktop_secret_store_bridge": true}"#).unwrap();
        assert_eq!(
            *field(&row, "secret_ref"),
            dict([("store_id", s("sqlcipher_inline")), ("state", s("ok"))])
        );
        assert_eq!(
            &desk.calls[2..],
            [
                "get:cloud:macos_keychain:cloud",
                "delete:cloud:macos_keychain:cloud"
            ]
        );
        assert!(desk.secrets.is_empty());
        // And back to the keychain; a vanished secret is marked missing.
        call(&book.conn, &mut desk, "ai.providers.move_api_key", r#"{"name": "cloud", "store_id": "linux_secret_service", "_desktop_secret_store_bridge": true}"#).unwrap();
        desk.secrets.clear();
        let row = call(
            &book.conn,
            &mut desk,
            "ai.providers.get",
            r#"{"name": "cloud", "_desktop_secret_store_bridge": true}"#,
        )
        .unwrap();
        assert_eq!(text(field(&row, "secret_ref"), "state").unwrap(), "missing");
        assert_eq!(*field(&row, "has_api_key"), Value::Bool(false));
        // Deleting a provider deletes its native secret, best effort.
        desk.fail = true;
        call(
            &book.conn,
            &mut desk,
            "ai.providers.delete",
            r#"{"name": "cloud", "_desktop_secret_store_bridge": true}"#,
        )
        .unwrap();
        assert!(desk
            .calls
            .last()
            .unwrap()
            .starts_with("delete:cloud:linux_secret_service"));
        assert!(!format!("{:?}", desk.calls).contains("sk-native"));
    }

    #[test]
    fn a_keychain_failure_marks_the_ref_unavailable() {
        let book = book();
        let mut desk = Desk::default();
        call(
            &book.conn,
            &mut desk,
            "ai.providers.create",
            r#"{"name": "cloud", "base_url": "https://c.test/v1", "acknowledged": true}"#,
        )
        .unwrap();
        call(&book.conn, &mut desk, "ai.providers.set_api_key", r#"{"name": "cloud", "api_key": "sk-native", "store_id": "macos_keychain", "_desktop_secret_store_bridge": true}"#).unwrap();
        desk.fail = true;
        let provider = store::get(&book.conn, "cloud").unwrap();
        let args = kassiber_pytext::loads(r#"{"_desktop_secret_store_bridge": true}"#).unwrap();
        let error = resolve_key(&book.conn, &mut desk, &provider, &args).unwrap_err();
        assert_eq!(error.code, "secret_ref_unavailable");
        assert_eq!(
            *field(error.details.as_ref().unwrap(), "cause_code"),
            s("secret_ref_unavailable")
        );
        assert_eq!(
            text(
                field(&store::get(&book.conn, "cloud").unwrap(), "secret_ref"),
                "state"
            )
            .unwrap(),
            "unavailable"
        );
    }

    #[test]
    fn edits_invalidate_discovery_except_preferences() {
        let book = book();
        let mut desk = Desk::default();
        let seed = || Ok(s("models"));
        desk.cache.get("k", false, seed).unwrap();
        call(
            &book.conn,
            &mut desk,
            "ai.providers.update",
            r#"{"name": "codex", "enabled": false, "clear": ["default_reasoning_effort"]}"#,
        )
        .unwrap();
        assert!(desk.cache.peek("k").is_some());
        call(
            &book.conn,
            &mut desk,
            "ai.providers.update",
            r#"{"name": "codex", "notes": "x"}"#,
        )
        .unwrap();
        assert!(desk.cache.peek("k").is_none());
        for (kind, args) in [
            (
                "ai.providers.create",
                r#"{"name": "n", "base_url": "https://n.test/v1"}"#,
            ),
            (
                "ai.providers.set_api_key",
                r#"{"name": "n", "api_key": "k"}"#,
            ),
            ("ai.providers.delete", r#"{"name": "n"}"#),
        ] {
            desk.cache.get("k", true, seed).unwrap();
            call(&book.conn, &mut desk, kind, args).unwrap();
            assert!(desk.cache.peek("k").is_none(), "{kind}");
        }
        // Reads and defaults keep it.
        desk.cache.get("k", true, seed).unwrap();
        for (kind, args) in [
            ("ai.providers.list", "{}"),
            ("ai.providers.get", r#"{"name": "codex"}"#),
            ("ai.providers.set_default", r#"{"name": "ollama"}"#),
        ] {
            call(&book.conn, &mut desk, kind, args).unwrap();
        }
        assert!(desk.cache.peek("k").is_some());
    }

    #[test]
    fn display_reads_never_probe_and_cli_runtimes_go_through_the_broker() {
        let book = book();
        let mut desk = Desk::default();
        let empty = call(&book.conn, &mut desk, "ai.provider_runtime.status", "{}").unwrap();
        assert_eq!(
            empty,
            kassiber_pytext::loads(
                r#"{"providers": [], "checked_at": null, "stale": false, "error": null}"#
            )
            .unwrap()
        );
        call(
            &book.conn,
            &mut desk,
            "ai.providers.acknowledge",
            r#"{"name": "codex"}"#,
        )
        .unwrap();
        let cached = call(
            &book.conn,
            &mut desk,
            "ai.provider_runtime.status",
            r#"{"cached_only": true}"#,
        )
        .unwrap();
        assert_eq!(*field(&cached, "checked_at"), Value::Null);
        // Each probe and listing starts the broker once, through the gate.
        let fake = FakeBroker::new(
            r#"import json, sys
request = json.loads(sys.stdin.readline())
if request["command"] == "status":
    rows = [{"provider": name, "state": "ready"} for name in request.get("providers") or [request.get("provider")]]
    print(json.dumps({"type": "result", "data": rows + [{"provider": "claude", "state": "ready"}]}), flush=True)
else:
    print(json.dumps({"type": "result", "data": [{"id": "gpt-5.4", "check_kind": "binary_presence"}]}), flush=True)
"#,
        );
        desk.broker = Some(fake.host());
        let probed = call(
            &book.conn,
            &mut desk,
            "ai.provider_runtime.status",
            r#"{"provider": "codex"}"#,
        )
        .unwrap();
        assert_eq!(
            *field(&probed, "providers"),
            kassiber_pytext::loads(r#"[{"provider": "codex", "state": "ready"}]"#).unwrap()
        );
        assert_eq!(*field(&probed, "provider"), s("codex"));
        let all = call(&book.conn, &mut desk, "ai.provider_runtime.status", "{}").unwrap();
        assert_eq!(
            *field(&all, "providers"),
            kassiber_pytext::loads(r#"[{"provider": "codex", "state": "ready"}]"#).unwrap()
        );
        // The cache answers a display read now, and a repeat within the TTL.
        call(&book.conn, &mut desk, "ai.provider_runtime.status", "{}").unwrap();
        let cached = call(
            &book.conn,
            &mut desk,
            "ai.provider_runtime.status",
            r#"{"cached_only": true}"#,
        )
        .unwrap();
        assert_eq!(*field(&cached, "providers"), *field(&all, "providers"));
        assert_eq!(fake.launches(), 2);
        let models = call(
            &book.conn,
            &mut desk,
            "ai.list_models",
            r#"{"provider": "codex"}"#,
        )
        .unwrap();
        assert_eq!(
            *field(&models, "models"),
            kassiber_pytext::loads(r#"[{"id": "gpt-5.4", "check_kind": "binary_presence"}]"#)
                .unwrap()
        );
        let tested = call(
            &book.conn,
            &mut desk,
            "ai.test_connection",
            r#"{"base_url": "codex-cli://default", "provider": "codex"}"#,
        )
        .unwrap();
        assert_eq!(
            kassiber_pyjson::dumps(&tested, &Default::default()).unwrap(),
            r#"{"base_url": "codex-cli://default", "model_count": 1, "models": [{"id": "gpt-5.4", "check_kind": "binary_presence"}], "check_kind": "binary_presence", "api_format": null, "detected_api_format": null, "tools_accepted": null, "store_accepted": null, "probe": {"status": "skipped", "reason": "cli_provider", "model": null, "finish_reason": null}, "detection_saved": false}"#
        );
        assert_eq!(fake.launches(), 4);
        assert_eq!(
            fake.requests(),
            [
                r#"{"command": "status", "provider": "codex"}"#,
                r#"{"command": "status", "providers": ["codex"]}"#,
                r#"{"command": "models", "provider": "codex"}"#,
                r#"{"command": "models", "provider": "codex"}"#,
            ]
        );
    }
}
