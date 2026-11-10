//! `ui.connections.btcpay.create` (`_create_btcpay_connection_payload`) and
//! the CLI `btcpay setup`, `wallets attach-btcpay` and `provenance sync --all`.
//! A store mapped onto an existing settlement wallet becomes provenance on
//! that wallet; the wallet stays the only balance source.
use super::args::{opt_str, py_text, req_str};
use super::connect::{existing, inline_args, Env, Instance};
use super::provenance::{self, redact_backend_text, Sync};
use super::setup::{self, field};
use super::Result;
use kassiber_chain::btcpay::{payment_methods as methods, url};
use kassiber_envelope::{dict, get, s, truthy, type_name, AppError};
use kassiber_net::Gate;
use kassiber_pyjson::{PyStr, Value};
use kassiber_setup::backends::Config;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::Connection;

/// The open book with mutable backend configuration (inline instances are
/// saved before routes are created, as the reference commits them).
pub struct Book<'a> {
    pub conn: &'a Connection,
    pub backends: &'a mut Config,
    pub sources: &'a dyn Sources,
    pub gate: Gate,
    pub version: &'a str,
    pub book_id: String,
}

impl Book<'_> {
    fn env(&self) -> Env<'_> {
        Env {
            conn: self.conn,
            backends: self.backends,
            gate: self.gate.clone(),
            version: self.version,
            book_id: self.book_id.clone(),
            progress: None,
        }
    }
}

fn text(v: &PyStr) -> String {
    url::from_py(v)
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// `_btcpay_setup_network`.
fn setup_network(args: &Value) -> Result<String> {
    let network = opt_str(args, "network")?
        .map(|n| kassiber_pytext::lower(kassiber_pytext::strip(&text(&n))))
        .unwrap_or_else(|| "main".into());
    if !["main", "test", "signet", "regtest"].contains(&network.as_str()) {
        return Err(
            validation(format!("Unsupported BTCPay network '{network}'"))
                .with_hint("Choose one of: main, test, signet, regtest."),
        );
    }
    Ok(network)
}

/// `_resolve_btcpay_backend_for_setup(..., reveal=False)`; with `create`,
/// an inline instance is saved and merged into the runtime configuration.
fn resolve(book: &mut Book<'_>, args: &Value, create: bool) -> Result<Instance> {
    let reference = opt_str(args, "backend")?;
    let mut inline = false;
    for key in ["server_url", "url", "api_key", "token"] {
        if opt_str(args, key)?.is_some() {
            inline = true;
            break;
        }
    }
    if let Some(reference) = reference {
        if inline {
            return Err(validation(
                "BTCPay setup received both a saved instance and inline credentials",
            )
            .with_hint("Choose a saved BTCPay instance or enter a new instance, not both."));
        }
        let mut instance = existing(&book.env(), &text(&reference))?;
        // `reveal=False` keeps the runtime view; only its URL is read here.
        if let Some(view) = book
            .backends
            .configured(&kassiber_pytext::lower(&text(&reference)))
        {
            let encoded = kassiber_pyjson::dumps(view, &Default::default())
                .map_err(|e| AppError::new("internal_error", e.to_string()))?;
            instance.backend = serde_json::from_str(&encoded)
                .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        }
        return Ok(instance);
    }
    let (name, server, key) = inline_args(args)?;
    if !create {
        return Ok(Instance {
            backend: serde_json::json!({"name": name, "kind": "btcpay", "url": server, "token": key}),
            safe: dict([
                ("name", s(name.as_str())),
                ("kind", s("btcpay")),
                ("has_url", Value::Bool(true)),
                ("has_token", Value::Bool(true)),
            ]),
            name,
        });
    }
    if book.backends.configured(&name).is_some() {
        return Err(AppError::new(
            "conflict",
            format!("A BTCPay instance named '{name}' already exists"),
        )
        .with_hint("Pick that saved instance, or enter a different instance name.")
        .with_details(dict([("existing_backend", s(name.as_str()))])));
    }
    let identity = url::server_url_identity(&server)?;
    for other in book.backends.names() {
        if other == name {
            continue;
        }
        let Some(raw) = book
            .backends
            .configured(&other)
            .filter(|v| matches!(v, Value::Dict(_)))
        else {
            continue;
        };
        let value = |key: &str| {
            get(raw, key)
                .filter(|v| !matches!(v, Value::Null))
                .map(|v| kassiber_pytext::strip(&kassiber_envelope::py_str(v)).to_owned())
                .filter(|v| !v.is_empty())
        };
        if kassiber_pytext::lower(kassiber_pytext::strip(&value("kind").unwrap_or_default()))
            != "btcpay"
        {
            continue;
        }
        if url::server_url_identity(&value("url").unwrap_or_default())? != identity {
            continue;
        }
        if value("token").as_deref() != Some(key.as_str()) {
            continue;
        }
        let existing_name = get(raw, "name")
            .filter(|v| truthy(v))
            .map(kassiber_envelope::py_str)
            .unwrap_or(other);
        return Err(AppError::new(
            "conflict",
            format!("BTCPay instance '{existing_name}' already uses this server URL and API key"),
        )
        .with_hint("Pick that saved instance, then choose or discover another store.")
        .with_details(dict([("existing_backend", s(existing_name.as_str()))])));
    }
    let network = setup_network(args)?;
    let created = book.backends.execute(
        book.conn,
        "backends.create",
        &dict([
            ("name", s(name.as_str())),
            ("kind", s("btcpay")),
            ("url", s(server.as_str())),
            ("chain", s("bitcoin")),
            ("network", s(network)),
            ("token", s(key.as_str())),
        ]),
        book.sources,
    )?;
    book.backends.merge(book.conn)?;
    let created_name = get(&created, "name")
        .map(kassiber_envelope::py_str)
        .unwrap_or(name);
    let safe = book.backends.details(book.conn, &created_name)?;
    let backend = book.backends.resolve(book.conn, &created_name)?;
    Ok(Instance {
        backend,
        safe,
        name: created_name,
    })
}

/// `_btcpay_payment_method_ids`.
fn payment_method_ids(args: &Value) -> Result<Vec<String>> {
    let raw = get(args, "payment_method_ids").unwrap_or(&Value::Null);
    let normalize = |value: &str| -> Result<String> {
        let canonical = methods::canonical(value);
        if canonical.is_empty() {
            return Err(validation("BTCPay payment method id cannot be empty"));
        }
        methods::require_wallet_history(&canonical, &canonical)
    };
    match raw {
        Value::Null => {
            let single = opt_str(args, "payment_method_id")?
                .map(|v| text(&v))
                .unwrap_or_else(|| "BTC-CHAIN".into());
            Ok(vec![normalize(&single)?])
        }
        Value::List(items) => {
            let mut out: Vec<String> = Vec::new();
            for item in items {
                let valid = matches!(item, Value::Str(t) if !kassiber_pytext::strip_py(t).is(""));
                if !valid {
                    return Err(validation(
                        "BTCPay payment_method_ids entries must be non-empty strings",
                    )
                    .with_details(dict([("type", s(type_name(item)))])));
                }
                let method = normalize(&text(&py_text(item)))?;
                if !out.contains(&method) {
                    out.push(method);
                }
            }
            if out.is_empty() {
                return Err(validation("Select at least one BTCPay payment method"));
            }
            Ok(out)
        }
        other => Err(validation("BTCPay payment_method_ids must be an array")
            .with_details(dict([("type", s(type_name(other)))]))),
    }
}

/// `_btcpay_existing_wallet_routes`: `(wallet ref, payment method)`.
fn existing_wallet_routes(args: &Value) -> Result<Vec<(String, String)>> {
    let raw = get(args, "routes").unwrap_or(&Value::Null);
    if matches!(raw, Value::Null) {
        let target = opt_str(args, "target_wallet")?.ok_or_else(|| {
            validation("Existing-wallet BTCPay setup requires routes")
                .with_hint("Choose which Kassiber wallet each BTCPay payment method settles into.")
        })?;
        return payment_method_ids(args)?
            .into_iter()
            .map(|m| Ok((text(&target), methods::require_settlement(&m, &m)?)))
            .collect();
    }
    let Value::List(items) = raw else {
        return Err(validation(
            "Existing-wallet BTCPay routes must be a non-empty array",
        ));
    };
    if items.is_empty() {
        return Err(validation(
            "Existing-wallet BTCPay routes must be a non-empty array",
        ));
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for item in items {
        if !matches!(item, Value::Dict(_)) {
            return Err(validation("Existing-wallet BTCPay routes must be objects"));
        }
        let wallet = match opt_str(item, "wallet")? {
            Some(w) => Some(w),
            None => opt_str(item, "target_wallet")?,
        };
        let method = opt_str(item, "payment_method_id")?
            .map(|m| text(&m))
            .unwrap_or_else(|| "BTC-CHAIN".into());
        let Some(wallet) = wallet else {
            return Err(validation("Existing-wallet BTCPay routes require a wallet"));
        };
        let method = methods::canonical(&method);
        if method.is_empty() {
            return Err(validation("BTCPay payment method id cannot be empty"));
        }
        methods::require_settlement(&method, &method)?;
        let key = (text(&wallet), method);
        if !out.contains(&key) {
            out.push(key);
        }
    }
    Ok(out)
}

fn wallet_details(conn: &Connection, profile_id: &str, reference: &str) -> Result<Value> {
    crate::lifecycle::execute(
        conn,
        crate::Operation::Get {
            profile_id: profile_id.into(),
            wallet: reference.into(),
        },
    )
}

fn scope(conn: &Connection, args: &Value) -> Result<(String, String, String)> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let id = |v: &Value, key: &str| kassiber_envelope::py_str(get(v, key).unwrap_or(&Value::Null));
    Ok((
        id(&workspace, "id"),
        id(&profile, "id"),
        id(&profile, "label"),
    ))
}

/// Append one provenance route per mapping (`_attach_btcpay_provenance_payload`).
fn attach_routes(
    conn: &Connection,
    profile_id: &str,
    backend: &str,
    store: &str,
    routes: &[(String, String)],
) -> Result<Vec<Value>> {
    let mut updated = Vec::new();
    for (wallet_ref, method) in routes {
        let wallet = wallet_details(conn, profile_id, wallet_ref)?;
        let mut existing = match field(field(&wallet, "config"), "btcpay_provenance") {
            Value::List(items) => items.clone(),
            _ => Vec::new(),
        };
        let next = dict([
            ("backend", s(backend)),
            ("store_id", s(store)),
            ("payment_method_id", s(method.as_str())),
        ]);
        if !existing.contains(&next) {
            existing.push(next);
        }
        let encoded = kassiber_pyjson::dumps(&Value::List(existing), &Default::default())
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        updated.push(crate::lifecycle::execute(
            conn,
            crate::Operation::Update {
                profile_id: profile_id.into(),
                wallet: kassiber_envelope::py_str(field(&wallet, "id")),
                label: None,
                account: None,
                config: serde_json::json!({"btcpay_provenance": serde_json::from_str::<serde_json::Value>(&encoded).expect("routes")}),
                clear: Vec::new(),
            },
        )?);
    }
    Ok(updated)
}

fn store_id(args: &Value) -> Result<String> {
    let store = req_str(args, "store_id", "BTCPay store ID")?;
    Ok(text(&store))
}

/// `ui.connections.btcpay.create`.
pub fn create(book: &mut Book<'_>, args: &Value) -> Result<Value> {
    let label = req_str(args, "label", "Connection label")?;
    let mode = opt_str(args, "mode")?
        .map(|m| kassiber_pytext::lower(kassiber_pytext::strip(&text(&m))))
        .unwrap_or_else(|| "wallet_sources".into());
    if ![
        "wallet_sources",
        "existing_wallets",
        "map_existing",
        "provenance",
        "account",
        "account_wizard",
    ]
    .contains(&mode.as_str())
    {
        return Err(validation(format!(
            "Unsupported BTCPay setup mode '{mode}'"
        )));
    }
    if mode.starts_with("account") {
        return account(book, args, &text(&label));
    }
    let empty = dict::<&str>([]);
    if ["existing_wallets", "map_existing", "provenance"].contains(&mode.as_str()) {
        store_id(args)?;
        let routes = existing_wallet_routes(args)?;
        let (_, profile, _) = scope(book.conn, &empty)?;
        for (wallet, _) in &routes {
            wallet_details(book.conn, &profile, wallet)?;
        }
        let instance = resolve(book, args, true)?;
        let store = store_id(args)?;
        let routes = existing_wallet_routes(args)?;
        let updated = attach_routes(book.conn, &profile, &instance.name, &store, &routes)?;
        return Ok(dict([
            ("mode", s("existing_wallets")),
            ("backend", instance.safe),
            ("wallet", updated.first().cloned().unwrap_or(Value::Null)),
            ("wallets", Value::List(updated)),
            (
                "routes",
                Value::List(
                    routes
                        .into_iter()
                        .map(|(w, m)| dict([("wallet", s(w)), ("payment_method_id", s(m))]))
                        .collect(),
                ),
            ),
        ]));
    }
    let (workspace, profile, profile_label) = scope(book.conn, &empty)?;
    let ids = payment_method_ids(args)?;
    let label = text(&label);
    let labels: Vec<String> = if ids.len() == 1 {
        vec![label.clone()]
    } else {
        ids.iter().map(|m| format!("{label} - {m}")).collect()
    };
    let mut taken = Vec::new();
    for candidate in &labels {
        let exists: bool = book
            .conn
            .query_row(
                "SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?",
                [profile.as_str(), candidate.as_str()],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists && !taken.contains(candidate) {
            taken.push(candidate.clone());
        }
    }
    if !taken.is_empty() {
        taken.sort();
        return Err(AppError::new(
            "conflict",
            format!(
                "Wallet '{}' already exists in profile '{profile_label}'",
                taken[0]
            ),
        )
        .with_hint("Choose a different connection label.")
        .with_details(dict([(
            "existing_labels",
            Value::List(taken.into_iter().map(s).collect()),
        )])));
    }
    let store = store_id(args)?;
    let instance = resolve(book, args, true)?;
    let mut wallets = Vec::new();
    for (label, method) in labels.iter().zip(&ids) {
        wallets.push(crate::lifecycle::execute(
            book.conn,
            crate::Operation::Create {
                workspace_id: workspace.clone(),
                profile_id: profile.clone(),
                label: label.clone(),
                kind: "custom".into(),
                account: None,
                config: serde_json::json!({
                    "backend": instance.name,
                    "store_id": store,
                    "payment_method_id": method,
                    "sync_source": "btcpay",
                }),
            },
        )?);
    }
    Ok(dict([
        ("backend", instance.safe),
        ("wallet", wallets[0].clone()),
        ("wallets", Value::List(wallets)),
    ]))
}

/// `parse_bool(value, default=True)`.
fn flag(value: Option<&Value>) -> Result<bool> {
    match value {
        None | Some(Value::Null) => Ok(true),
        Some(v) => {
            match kassiber_pytext::lower(kassiber_pytext::strip(&kassiber_envelope::py_str(v)))
                .as_str()
            {
                "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(AppError::new(
                    "app_error",
                    format!("Invalid boolean value: {}", kassiber_envelope::py_str(v)),
                )),
            }
        }
    }
}

fn error_summary(error: &AppError, store: Option<&str>) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    if let Some(store) = store {
        out.push(("store_id", s(store)));
    }
    out.extend([
        ("status", s("error")),
        ("code", s(error.code.as_str())),
        ("message", s(redact_backend_text(&error.message))),
        (
            "hint",
            s(error
                .hint
                .as_deref()
                .map(redact_backend_text)
                .unwrap_or_default()),
        ),
        ("retryable", Value::Bool(error.retryable)),
    ]);
    out
}

/// `_btcpay_error_summary` with its `status` replaced by `error`.
pub fn cli_error_summary(error: &AppError) -> Vec<(&'static str, Value)> {
    let missing = match &error.details {
        Some(details @ Value::Dict(_)) => get(details, "missing_permission")
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    vec![
        ("status", s("error")),
        ("code", s(error.code.as_str())),
        ("message", s(redact_backend_text(&error.message))),
        (
            "hint",
            s(error
                .hint
                .as_deref()
                .map(redact_backend_text)
                .unwrap_or_default()),
        ),
        ("missing_permission", missing),
    ]
}

fn without_checkpoint(mut value: Value) -> Value {
    if let Value::Dict(pairs) = &mut value {
        pairs.retain(|(k, _)| *k != kassiber_pyjson::Key::from("freshness_checkpoint"));
    }
    value
}

/// `_create_btcpay_account_setup_payload`.
fn account(book: &mut Book<'_>, args: &Value, label: &str) -> Result<Value> {
    let saved = opt_str(args, "backend")?.is_some();
    let routes = setup::normalize_routes(get(args, "routes").unwrap_or(&Value::Null), saved)?;
    let empty = dict::<&str>([]);
    let (workspace, profile, profile_label) = scope(book.conn, &empty)?;
    let candidate = resolve(book, args, false)?;
    let server = candidate.backend["url"]
        .as_str()
        .map(kassiber_pytext::strip)
        .filter(|u| !u.is_empty())
        .map(str::to_owned);
    setup::validate_routes(
        book.conn,
        &profile,
        &routes,
        &candidate.name,
        book.backends,
        server.as_deref(),
    )?;
    let instance = resolve(book, args, true)?;
    let applied = setup::apply_routes(
        book.conn,
        &setup::Scope {
            workspace_id: &workspace,
            profile_id: &profile,
            profile_label: &profile_label,
        },
        &instance.name,
        &routes,
        label,
    )?;
    let mut provenance_results = Vec::new();
    if flag(get(args, "sync_provenance"))? {
        let page_size = page_size(get(args, "page_size"))?;
        if let Value::List(stores) = field(&applied, "provenance_store_ids") {
            for store in stores {
                let store = py_text(store);
                let env = book.env();
                let outcome = provenance::sync(
                    &env,
                    Sync {
                        workspace_id: &workspace,
                        profile_id: &profile,
                        backend: &instance.name,
                        store_id: &store,
                        page_size,
                        include_payouts: true,
                        suggest: true,
                        kind: "ui.connections.btcpay.create",
                        args,
                        checkpoint: &Value::Null,
                    },
                );
                provenance_results.push(match outcome {
                    Ok(result) => without_checkpoint(result),
                    Err(error) if error.code == "internal_error" => return Err(error),
                    Err(error) => dict(error_summary(&error, Some(&text(&store)))),
                });
            }
        }
    }
    Ok(dict([
        ("mode", s("account")),
        ("backend", instance.safe),
        ("wallet", field(&applied, "wallet").clone()),
        ("wallets", field(&applied, "wallets").clone()),
        ("routes", Value::List(routes)),
        ("wallet_sources", field(&applied, "wallet_sources").clone()),
        (
            "payment_ledgers",
            field(&applied, "payment_ledgers").clone(),
        ),
        ("reused_wallets", field(&applied, "reused_wallets").clone()),
        ("mappings", field(&applied, "mappings").clone()),
        ("account_routes", field(&applied, "account_routes").clone()),
        ("provenance", Value::List(provenance_results)),
        ("skipped", field(&applied, "skipped").clone()),
    ]))
}

/// `int(value or DEFAULT_PAGE_SIZE)`.
pub fn page_size(value: Option<&Value>) -> Result<i64> {
    let invalid = || AppError::new("internal_error", "invalid literal for int() with base 10");
    match value.filter(|v| truthy(v)) {
        None => Ok(100),
        Some(Value::Int(n)) => Ok(i64::try_from(n.clone()).unwrap_or(i64::MAX)),
        Some(Value::Bool(b)) => Ok(i64::from(*b)),
        Some(Value::Float(f)) if f.is_finite() => Ok(f.trunc() as i64),
        Some(Value::Str(t)) => t
            .as_str()
            .and_then(|t| kassiber_pytext::parse_int(t).ok())
            .and_then(|n| i64::try_from(n).ok())
            .ok_or_else(invalid),
        Some(_) => Err(invalid()),
    }
}

/// `wallets attach-btcpay` (`attach_btcpay_provenance_to_wallet`).
pub fn attach(book: &mut Book<'_>, args: &Value) -> Result<Value> {
    let raw = |key: &str| {
        get(args, key)
            .filter(|v| !matches!(v, Value::Null))
            .map(kassiber_envelope::py_str)
    };
    let store = raw("store_id")
        .map(|v| kassiber_pytext::strip(&v).to_owned())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| validation("BTCPay store id cannot be empty"))?;
    let method = raw("payment_method_id")
        .map(|v| kassiber_pytext::strip(&v).to_owned())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| validation("BTCPay payment method id cannot be empty"))?;
    let method = methods::canonical(&method);
    methods::require_settlement(&method, &method)?;
    let (_, profile, _) = scope(book.conn, args)?;
    let wallet = crate::resolve(book.conn, &profile, &raw("wallet").unwrap_or_default())?;
    let backend = book
        .backends
        .resolve_configured(book.conn, raw("backend").as_deref())?;
    let name = backend["name"].as_str().unwrap_or_default().to_owned();
    let kind = backend["kind"].as_str().unwrap_or_default().to_owned();
    if provenance::backend_kind(&kind) != "btcpay" {
        return Err(validation(format!(
            "Backend '{name}' has kind '{kind}', expected 'btcpay'"
        ))
        .with_hint("Use a BTCPay backend for BTCPay provenance enrichment."));
    }
    let config: serde_json::Value = book
        .conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id = ?",
            [&wallet.id],
            |r| r.get::<_, Option<String>>(0),
        )
        .map_err(|e| AppError::new("internal_error", e.to_string()))
        .map(|raw| {
            serde_json::from_str(raw.as_deref().unwrap_or("{}")).unwrap_or(serde_json::json!({}))
        })?;
    let mut routes = crate::config::provenance(
        config
            .get("btcpay_provenance")
            .unwrap_or(&serde_json::Value::Null),
    )?;
    let next = serde_json::json!({"backend": name.to_lowercase(), "store_id": store, "payment_method_id": method});
    let list = routes.as_array_mut().expect("normalized routes");
    if !list.contains(&next) {
        list.push(next);
    }
    crate::lifecycle::execute(
        book.conn,
        crate::Operation::Update {
            profile_id: profile,
            wallet: wallet.id,
            label: None,
            account: None,
            config: serde_json::json!({"btcpay_provenance": routes}),
            clear: Vec::new(),
        },
    )
}

/// `btcpay provenance sync --all` (`sync_all_btcpay_commercial_provenance`).
pub fn sync_all(
    book: &mut Book<'_>,
    args: &Value,
    page_size: i64,
    include_payouts: bool,
) -> Result<Value> {
    let (workspace, profile, _) = scope(book.conn, args)?;
    let mut names = Vec::new();
    for name in book.backends.names() {
        if let Some(backend) = book
            .backends
            .configured(&name)
            .filter(|v| matches!(v, Value::Dict(_)))
        {
            let kind = get(backend, "kind")
                .filter(|v| truthy(v))
                .map(kassiber_envelope::py_str)
                .unwrap_or_default();
            if kassiber_pytext::lower(&kind) == "btcpay" {
                let label = get(backend, "name")
                    .filter(|v| truthy(v))
                    .map(kassiber_envelope::py_str)
                    .unwrap_or(name);
                names.push(kassiber_pytext::lower(&label));
            }
        }
    }
    let routes = setup::existing_routes(book.conn, &profile, &names)?;
    let mut stores: Vec<(String, String)> = routes
        .iter()
        .filter(|r| !truthy(field(r, "deprecated")))
        .map(|r| {
            (
                kassiber_envelope::py_str(field(r, "backend")),
                kassiber_envelope::py_str(field(r, "store_id")),
            )
        })
        .collect();
    stores.sort();
    stores.dedup();
    let mut results = Vec::new();
    for (backend, store) in &stores {
        let env = book.env();
        let store_py = PyStr::from(store.as_str());
        let outcome = provenance::sync(
            &env,
            Sync {
                workspace_id: &workspace,
                profile_id: &profile,
                backend,
                store_id: &store_py,
                page_size,
                include_payouts,
                suggest: true,
                kind: "btcpay.provenance.sync",
                args,
                checkpoint: &Value::Null,
            },
        );
        results.push(match outcome {
            Ok(result) => {
                let mut pairs = vec![("status".to_owned(), s("synced"))];
                if let Value::Dict(items) = without_checkpoint(result) {
                    for (k, v) in items {
                        if let kassiber_pyjson::Key::Str(k) = k {
                            pairs.push((url::from_py(&k), v));
                        }
                    }
                }
                dict(pairs)
            }
            Err(error) if error.code == "internal_error" => return Err(error),
            Err(error) => {
                let mut pairs = vec![
                    ("backend", s(backend.as_str())),
                    ("store_id", s(store.as_str())),
                ];
                pairs.extend(cli_error_summary(&error));
                dict(pairs)
            }
        });
    }
    let synced = results
        .iter()
        .filter(|r| field(r, "status") == &s("synced"))
        .count();
    Ok(dict([
        ("stores", Value::Int(stores.len().into())),
        ("synced", Value::Int(synced.into())),
        ("failed", Value::Int((results.len() - synced).into())),
        ("results", Value::List(results)),
    ]))
}

/// `parse_btcpay_route_spec`: `STORE[:METHOD]=ACTION[@WALLET]`.
fn route_spec(spec: &str) -> Result<super::setup::RouteOverride> {
    let raw = kassiber_pytext::strip(spec);
    let invalid = || {
        validation(format!("Invalid --route '{spec}'"))
            .with_hint("Use STORE[:METHOD]=ACTION[@WALLET], for example AbC123:BTC-CHAIN=existing_wallet@Shop Trezor.")
    };
    let (target, choice) = raw.split_once('=').ok_or_else(invalid)?;
    if kassiber_pytext::strip(target).is_empty() || kassiber_pytext::strip(choice).is_empty() {
        return Err(invalid());
    }
    let target = kassiber_pytext::strip(target);
    let (store, method) = target.split_once(':').unwrap_or((target, ""));
    let choice = kassiber_pytext::strip(choice);
    let (action, wallet) = choice.split_once('@').unwrap_or((choice, ""));
    let store = kassiber_pytext::strip(store);
    if store.is_empty() {
        return Err(validation("BTCPay store id cannot be empty"));
    }
    let method = kassiber_pytext::strip(method);
    let method = methods::canonical(if method.is_empty() {
        "BTC-CHAIN"
    } else {
        method
    });
    let wallet = kassiber_pytext::strip(wallet);
    Ok((
        (store.to_owned(), method),
        (
            kassiber_pytext::lower(kassiber_pytext::strip(action)),
            (!wallet.is_empty()).then(|| wallet.to_owned()),
        ),
    ))
}

/// `btcpay setup` (`handlers.apply_btcpay_setup`).
pub fn setup_cli(book: &mut Book<'_>, args: &Value) -> Result<Value> {
    let (workspace, profile, profile_label) = scope(book.conn, args)?;
    let mut overrides = Vec::new();
    if let Some(Value::List(specs)) = get(args, "route_specs") {
        for spec in specs {
            overrides.push(route_spec(&kassiber_envelope::py_str(spec))?);
        }
    }
    let recommended = get(args, "recommended").is_some_and(truthy);
    if overrides.is_empty() && !recommended {
        return Err(validation("Choose what to set up").with_hint(
            "Pass --recommended to apply the suggested plan, or --route STORE[:METHOD]=ACTION for each payment method.",
        ));
    }
    let plan = super::connect::inspect_for(&book.env(), args, "btcpay.setup")?;
    let reference = get(args, "backend")
        .map(kassiber_envelope::py_str)
        .unwrap_or_default();
    let backend = book
        .backends
        .resolve_configured(book.conn, Some(&reference))?;
    let name = backend["name"].as_str().unwrap_or_default().to_owned();
    let server = backend["url"].as_str().map(str::to_owned);
    let routes = setup::routes_from_plan(&plan, &overrides, recommended)?;
    let routes = setup::normalize_routes(&Value::List(routes), true)?;
    setup::validate_routes(
        book.conn,
        &profile,
        &routes,
        &name,
        book.backends,
        server.as_deref(),
    )?;
    let dry_run = get(args, "dry_run").is_some_and(truthy);
    let warnings = match field(&plan, "warnings") {
        v if truthy(v) => v.clone(),
        _ => Value::List(Vec::new()),
    };
    let mut result = vec![
        ("backend", s(name.as_str())),
        ("dry_run", Value::Bool(dry_run)),
        ("routes", Value::List(routes.clone())),
        ("warnings", warnings),
        ("api_key", field(&plan, "api_key").clone()),
    ];
    if dry_run {
        return Ok(dict(result));
    }
    let label = get(args, "label")
        .map(kassiber_envelope::py_str)
        .unwrap_or_default();
    let applied = setup::apply_routes(
        book.conn,
        &setup::Scope {
            workspace_id: &workspace,
            profile_id: &profile,
            profile_label: &profile_label,
        },
        &name,
        &routes,
        &label,
    )?;
    let mut provenance_results = Vec::new();
    if get(args, "sync_provenance").is_none_or(truthy) {
        if let Value::List(stores) = field(&applied, "provenance_store_ids") {
            for store in stores {
                let store = py_text(store);
                let env = book.env();
                let outcome = provenance::sync(
                    &env,
                    Sync {
                        workspace_id: &workspace,
                        profile_id: &profile,
                        backend: &name,
                        store_id: &store,
                        page_size: 100,
                        include_payouts: true,
                        suggest: true,
                        kind: "btcpay.setup",
                        args,
                        checkpoint: &Value::Null,
                    },
                );
                provenance_results.push(match outcome {
                    Ok(value) => {
                        let mut pairs = vec![("status".to_owned(), s("synced"))];
                        if let Value::Dict(items) = without_checkpoint(value) {
                            for (k, v) in items {
                                if let kassiber_pyjson::Key::Str(k) = k {
                                    pairs.push((url::from_py(&k), v));
                                }
                            }
                        }
                        dict(pairs)
                    }
                    Err(error) if error.code == "internal_error" => return Err(error),
                    Err(error) => {
                        let mut pairs = vec![("store_id", Value::Str(store.clone()))];
                        pairs.extend(cli_error_summary(&error));
                        dict(pairs)
                    }
                });
            }
        }
    }
    for key in [
        "wallet_sources",
        "payment_ledgers",
        "reused_wallets",
        "mappings",
        "account_routes",
        "skipped",
    ] {
        result.push((key, field(&applied, key).clone()));
    }
    result.push(("provenance", Value::List(provenance_results)));
    Ok(dict(result))
}
