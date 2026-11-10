//! Desktop BTCPay connection actions: saved or inline instance resolution
//! (`_resolve_btcpay_backend_for_setup`) and the explicit wallet test. Each
//! network action mints one grant bound to its exact kind, arguments, book
//! and backend configuration, and revokes it when the action returns.
use super::args::{opt_str, req_str};
use super::Result;
use kassiber_chain::btcpay::{client::Greenfield, fetch, payment_methods as methods, url};
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::{PyStr, Value};
use kassiber_setup::backends::Config;
use kassiber_store::rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

/// What a BTCPay service may use from the open book.
pub struct Env<'a> {
    pub conn: &'a Connection,
    pub backends: &'a Config,
    pub gate: Gate,
    pub version: &'a str,
    pub book_id: String,
    /// `emit_sync_progress` for page reads (freshness jobs only).
    pub progress: Option<kassiber_chain::btcpay::fetch::Progress<'a>>,
}

/// A resolved instance: the credential-bearing configuration stays here;
/// `safe` is the redacted display row.
pub struct Instance {
    pub backend: serde_json::Value,
    pub safe: Value,
    pub name: String,
}

/// Revokes the action's grant when the action ends, however it ends.
pub struct Grant(pub Consent);

impl Drop for Grant {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

impl Env<'_> {
    /// One client for one action; the binding covers every input that
    /// could change what the request reads or where it goes.
    pub fn client(
        &self,
        kind: &str,
        args: &Value,
        backend: &serde_json::Value,
        missing_token_hint: Option<&str>,
    ) -> Result<(Greenfield, Grant)> {
        let encoded = kassiber_pyjson::dumps(args, &Default::default())
            .map_err(|_| AppError::new("validation", "Invalid BTCPay arguments"))?;
        let binding = serde_json::json!(["btcpay-action-v1", kind, self.book_id, encoded, backend]);
        let binding: [u8; 32] = Sha256::digest(binding.to_string().as_bytes()).into();
        let client = Greenfield::from_backend(
            backend,
            self.gate.clone(),
            self.version,
            binding,
            missing_token_hint,
        )?;
        let grant = Grant(Consent::explicit_trigger(client.request()));
        Ok((client, grant))
    }
}

pub fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(600)
}

fn text(value: &PyStr) -> String {
    url::from_py(value)
}

/// `_slug_btcpay_backend_label`.
pub fn slug(label: &str) -> Result<String> {
    let name = kassiber_chain::btcpay::slug_label(label);
    if name.is_empty() {
        return Err(
            AppError::new("validation", "BTCPay instance label is required.")
                .with_hint("Enter a short local name for this BTCPay instance."),
        );
    }
    Ok(name)
}

/// `_inline_btcpay_backend_args`: `(name, normalized url, api key)`.
pub fn inline_args(args: &Value) -> Result<(String, String, String)> {
    let label = match opt_str(args, "backend_label")? {
        Some(v) => Some(v),
        None => opt_str(args, "instance_label")?,
    };
    let name = slug(&label.map(|v| text(&v)).unwrap_or_else(|| "btcpay".into()))?;
    let server = match opt_str(args, "server_url")? {
        Some(v) => v,
        None => match opt_str(args, "url")? {
            Some(v) => v,
            None => req_str(args, "server_url", "BTCPay server URL")?,
        },
    };
    let server = url::normalize_server_url(&text(&server))?;
    let key = match opt_str(args, "api_key")? {
        Some(v) => v,
        None => match opt_str(args, "token")? {
            Some(v) => v,
            None => req_str(args, "api_key", "BTCPay API key")?,
        },
    };
    Ok((name, server, kassiber_pytext::strip(&text(&key)).to_owned()))
}

fn has_inline(args: &Value) -> Result<bool> {
    for key in ["server_url", "url", "api_key", "token"] {
        if opt_str(args, key)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `_existing_btcpay_backend(ctx, ref, reveal=True)`.
pub fn existing(env: &Env<'_>, reference: &str) -> Result<Instance> {
    let normalized = kassiber_pytext::lower(reference);
    let raw = env
        .backends
        .configured(&normalized)
        .filter(|v| matches!(v, Value::Dict(_)))
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                format!("BTCPay instance '{reference}' is not configured"),
            )
            .with_hint("Choose a saved BTCPay instance or add a new one here.")
        })?;
    let kind = get(raw, "kind")
        .filter(|v| kassiber_envelope::truthy(v))
        .map(kassiber_envelope::py_str)
        .unwrap_or_default();
    if kassiber_pytext::lower(kassiber_pytext::strip(&kind)) != "btcpay" {
        return Err(AppError::new(
            "validation",
            format!("Backend '{reference}' is not a BTCPay instance"),
        )
        .with_hint("Choose a backend whose kind is btcpay."));
    }
    let safe = env.backends.details(env.conn, &normalized)?;
    let backend = env.backends.resolve(env.conn, &normalized)?;
    let name = get(&safe, "name")
        .map(kassiber_envelope::py_str)
        .unwrap_or(normalized);
    Ok(Instance {
        backend,
        safe,
        name,
    })
}

/// `_resolve_btcpay_backend_for_setup(..., create_if_inline=False, reveal=True)`.
pub fn resolve_without_create(env: &Env<'_>, args: &Value) -> Result<Instance> {
    let reference = opt_str(args, "backend")?;
    let inline = has_inline(args)?;
    if let Some(reference) = reference {
        if inline {
            return Err(AppError::new(
                "validation",
                "BTCPay setup received both a saved instance and inline credentials",
            )
            .with_hint("Choose a saved BTCPay instance or enter a new instance, not both."));
        }
        return existing(env, &text(&reference));
    }
    let (name, server, key) = inline_args(args)?;
    Ok(Instance {
        backend: serde_json::json!({"name": name, "kind": "btcpay", "url": server, "token": key}),
        safe: dict([
            ("name", s(name.as_str())),
            ("kind", s("btcpay")),
            ("has_url", Value::Bool(true)),
            ("has_token", Value::Bool(true)),
        ]),
        name,
    })
}

/// `ui.connections.btcpay.test` (`_test_btcpay_connection_payload`): one
/// wallet-history request for the chosen store and payment method.
pub fn test(env: &Env<'_>, args: &Value) -> Result<Value> {
    let store = req_str(args, "store_id", "BTCPay store ID")?;
    let method = opt_str(args, "payment_method_id")?.unwrap_or_else(|| "BTC-CHAIN".into());
    let instance = resolve_without_create(env, args)?;
    let shown = text(&method);
    let canonical = methods::require_wallet_history(&shown, &shown)?;
    let (client, grant) =
        env.client("ui.connections.btcpay.test", args, &instance.backend, None)?;
    let reads = fetch::Reads {
        client: &client,
        consent: &grant.0,
        deadline: deadline(),
        progress: env.progress,
    };
    fetch::probe_wallet(reads, &text(&store), &canonical)?;
    Ok(dict([
        ("backend", s(instance.name.as_str())),
        ("store_id", Value::Str(store)),
        ("payment_method_id", Value::Str(method)),
        ("ok", Value::Bool(true)),
    ]))
}

fn inspect_and_plan(
    env: &Env<'_>,
    kind: &str,
    args: &Value,
    instance: &Instance,
    profile_id: &str,
    server_url: Option<&str>,
) -> Result<Value> {
    let (client, grant) = env.client(
        kind,
        args,
        &instance.backend,
        Some("Enter a Greenfield API key for this BTCPay instance."),
    )?;
    let reads = fetch::Reads {
        client: &client,
        consent: &grant.0,
        deadline: deadline(),
        progress: env.progress,
    };
    let inspection = kassiber_chain::btcpay::discovery::inspect(reads)?;
    drop(grant);
    let mut plan = super::setup::plan(
        env.conn,
        profile_id,
        &inspection,
        &instance.name,
        env.backends,
        server_url,
    )?;
    let detected = super::setup::detected_network(&plan);
    kassiber_envelope::set(&mut plan, "detected_network", detected);
    Ok(plan)
}

fn profile_of(env: &Env<'_>, args: &Value) -> Result<(String, String)> {
    let (workspace, profile) = kassiber_setup::references::scope(env.conn, args)?;
    let id = |v: &Value| kassiber_envelope::py_str(get(v, "id").unwrap_or(&Value::Null));
    Ok((id(&workspace), id(&profile)))
}

/// `ui.connections.btcpay.discover` (`_discover_btcpay_connection_payload`).
pub fn discover(env: &Env<'_>, args: &Value) -> Result<Value> {
    let instance = resolve_without_create(env, args)?;
    let (_, profile) = profile_of(env, &dict::<&str>([]))?;
    let server = instance.backend["url"]
        .as_str()
        .map(kassiber_pytext::strip)
        .filter(|u| !u.is_empty())
        .map(str::to_owned);
    let mut plan = inspect_and_plan(
        env,
        "ui.connections.btcpay.discover",
        args,
        &instance,
        &profile,
        server.as_deref(),
    )?;
    use kassiber_chain::btcpay::permissions::{server_version, setup_guide, wallet_history_preset};
    // BTCPay 2.4 serves wallet history to the read-only key; 2.3 needs the
    // store-modifying one.
    let version = kassiber_envelope::get(&plan, "server")
        .filter(|server| kassiber_envelope::truthy(server))
        .and_then(|server| kassiber_envelope::get(server, "version"));
    let version = server_version(version.unwrap_or(&Value::Null));
    kassiber_envelope::set(
        &mut plan,
        "key_upgrade",
        setup_guide(server.as_deref(), wallet_history_preset(version), None)?,
    );
    kassiber_envelope::set(
        &mut plan,
        "key_read_only",
        setup_guide(server.as_deref(), "read_only", None)?,
    );
    Ok(plan)
}

/// `btcpay inspect` (`handlers.inspect_btcpay_setup`).
pub fn inspect(env: &Env<'_>, args: &Value) -> Result<Value> {
    inspect_for(env, args, "btcpay.inspect")
}

/// The inspection plan for `kind` (`btcpay inspect` or `btcpay setup`).
pub fn inspect_for(env: &Env<'_>, args: &Value, kind: &str) -> Result<Value> {
    let (_, profile) = profile_of(env, args)?;
    let reference = get(args, "backend")
        .map(kassiber_envelope::py_str)
        .unwrap_or_default();
    let backend = env
        .backends
        .resolve_configured(env.conn, Some(&reference))?;
    let name = backend["name"].as_str().unwrap_or_default().to_owned();
    let raw_kind = backend["kind"].as_str().unwrap_or_default().to_owned();
    if super::provenance::backend_kind(&raw_kind) != "btcpay" {
        return Err(AppError::new(
            "validation",
            format!("Backend '{name}' has kind '{raw_kind}', expected 'btcpay'"),
        ));
    }
    let server = backend["url"].as_str().map(str::to_owned);
    let instance = Instance {
        backend: backend.clone(),
        safe: Value::Null,
        name,
    };
    let mut plan = inspect_and_plan(env, kind, args, &instance, &profile, server.as_deref())?;
    kassiber_envelope::set(
        &mut plan,
        "backend_url",
        s(url::redact_backend_url(server.as_deref().unwrap_or(""))),
    );
    Ok(plan)
}
