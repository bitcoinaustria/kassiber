//! `handlers.sync_btcpay_commercial_provenance`: read one store's invoices
//! and payouts, then store them as provenance and refresh link suggestions.
//! Every read finishes before the first write; a failed refresh keeps the
//! stored provenance and records only the attempt.
use super::connect::{deadline, Env};
use super::Result;
use kassiber_chain::btcpay::{fetch, url};
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_transactions::{btcpay_records as records, commercial, Scope};

/// `redact_backend_text`: every URL in `text` reduced to its location.
pub fn redact_backend_text(text: &str) -> String {
    use std::sync::OnceLock;
    static URLS: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = URLS.get_or_init(|| {
        regex::Regex::new(r#"\b[a-zA-Z][a-zA-Z0-9+.-]*://(?:\[[^\]\s]+\][^\s,;)"']*|[^\s,;)"']+)"#)
            .expect("backend URL pattern")
    });
    pattern
        .replace_all(text, |m: &regex::Captures<'_>| {
            let raw = m.get(0).expect("match").as_str();
            let core = raw.trim_end_matches([':', '.', '!', '?']);
            format!("{}{}", url::redact_backend_url(core), &raw[core.len()..])
        })
        .into_owned()
}

/// `normalize_backend_kind`.
pub fn backend_kind(kind: &str) -> String {
    let value = kassiber_pytext::lower(kassiber_pytext::strip(kind));
    match value.as_str() {
        "bitcoin-core" | "bitcoincore" | "core" => "bitcoinrpc".into(),
        "core-ln" | "core-lightning" => "coreln".into(),
        "liquid-esplora" | "mempool" => "esplora".into(),
        _ => value,
    }
}

pub struct Sync<'a> {
    /// The resolved book scope (`resolve_scope`).
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub backend: &'a str,
    pub store_id: &'a PyStr,
    pub page_size: i64,
    pub include_payouts: bool,
    /// Refresh local link suggestions afterwards (`suggest`).
    pub suggest: bool,
    /// The action this read belongs to, bound into its grant.
    pub kind: &'a str,
    pub args: &'a Value,
    /// The previous `freshness_checkpoint` (a dict, else empty): its known
    /// invoice pages stop the read early, and the next checkpoint updates it.
    pub checkpoint: &'a Value,
}

fn atomic<T>(env: &Env<'_>, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let conn = env.conn;
    kassiber_store::begin_write_savepoint(conn, "btcpay_provenance")
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT btcpay_provenance")
                .map_err(|e| AppError::new("internal_error", e.to_string()))?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT btcpay_provenance; RELEASE SAVEPOINT btcpay_provenance",
            );
            Err(error)
        }
    }
}

/// One store's refresh; `Ok` carries the reference's result payload.
pub fn sync(env: &Env<'_>, request: Sync<'_>) -> Result<Value> {
    let backend = env
        .backends
        .resolve_configured(env.conn, Some(request.backend))?;
    let name = backend["name"].as_str().unwrap_or_default().to_owned();
    let kind = backend_kind(backend["kind"].as_str().unwrap_or_default());
    if kind != "btcpay" {
        return Err(AppError::new(
            "validation",
            format!(
                "Backend '{name}' has kind '{}', expected 'btcpay'",
                backend["kind"].as_str().unwrap_or_default()
            ),
        )
        .with_hint("Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token-stdin` or `--token-fd FD`."));
    }
    let store = url::from_py(request.store_id);
    let previous = match request.checkpoint {
        value @ Value::Dict(_) => value.clone(),
        _ => dict::<&str>([]),
    };
    let fetched = (|| {
        let (client, grant) = env.client(request.kind, request.args, &backend, None)?;
        let reads = fetch::Reads {
            client: &client,
            consent: &grant.0,
            deadline: deadline(),
            progress: env.progress,
        };
        let (invoices, meta) =
            fetch::invoice_provenance(reads, request.store_id, request.page_size, &previous)?;
        let payouts = if request.include_payouts {
            Some(fetch::payouts(reads, request.store_id)?)
        } else {
            None
        };
        Ok::<_, AppError>((invoices, meta, payouts))
    })();
    let (invoices, meta, payouts) = match fetched {
        Ok(fetched) => fetched,
        // A bare Python exception escapes before the attempt is recorded.
        Err(error) if error.code == "internal_error" => return Err(error),
        Err(error) => {
            // Committed on its own, as the reference commits it before raising.
            let code = if error.code.is_empty() {
                "error"
            } else {
                &error.code
            };
            records::record_store_sync(
                env.conn,
                request.profile_id,
                &name,
                &store,
                Some((code, &redact_backend_text(&error.message))),
                (None, None),
            )?;
            return Err(error);
        }
    };
    let scope = Scope {
        workspace_id: request.workspace_id,
        profile_id: request.profile_id,
    };
    let (mut outcome, suggestions) = atomic(env, || {
        let mut outcome = records::upsert_provenance(env.conn, &scope, &name, &invoices)?;
        if let Some((rows, _)) = &payouts {
            let payout_outcome = records::upsert_payouts(env.conn, &scope, &name, rows)?;
            if let (Value::Dict(base), Value::Dict(extra)) = (&mut outcome, payout_outcome) {
                base.extend(extra);
            }
        }
        records::record_store_sync(
            env.conn,
            request.profile_id,
            &name,
            &store,
            None,
            (
                Some(invoices.len() as i64),
                payouts.as_ref().map(|(rows, _)| rows.len() as i64),
            ),
        )?;
        if !request.suggest {
            return Ok((outcome, Value::Null));
        }
        let suggested = commercial::execute(
            env.conn,
            &scope,
            &commercial::Command::Suggest { limit: 500 },
        )?;
        let total = match get(&suggested, "suggestions") {
            Some(Value::List(items)) => items.len(),
            _ => 0,
        };
        Ok((
            outcome,
            dict([
                (
                    "created",
                    get(&suggested, "created").cloned().unwrap_or(Value::Null),
                ),
                ("total", Value::Int(total.into())),
            ]),
        ))
    })?;
    let field = |key: &str| get(&meta, key).cloned().unwrap_or(Value::Null);
    let payout_meta = payouts.as_ref().map(|(_, meta)| meta);
    // `checkpoint.update(...)`: earlier keys keep their place.
    let mut checkpoint = previous;
    for (key, value) in [
        (
            "backend",
            dict([("name", s(name.as_str())), ("kind", s(kind.as_str()))]),
        ),
        ("btcpay_invoice_pages", field("btcpay_invoice_pages")),
        (
            "btcpay_invoice_pagination",
            field("btcpay_invoice_pagination"),
        ),
        ("store_id", Value::Str(request.store_id.clone())),
    ] {
        kassiber_envelope::set(&mut checkpoint, key, value);
    }
    let Value::Dict(pairs) = &mut outcome else {
        unreachable!("outcome dict")
    };
    let mut push = |key: &str, value: Value| pairs.push((key.into(), value));
    push("backend", s(name.as_str()));
    push("backend_kind", s(kind.as_str()));
    push(
        "backend_url",
        s(url::redact_backend_url(
            backend["url"].as_str().unwrap_or_default(),
        )),
    );
    push("store_id", Value::Str(request.store_id.clone()));
    push("page_size", Value::Int(request.page_size.into()));
    push("pages_fetched", field("pages_fetched"));
    push("invoices_hydrated", field("invoices_hydrated"));
    push(
        "stopped_by_known_page",
        Value::Bool(kassiber_envelope::truthy(&field("stopped_by_known_page"))),
    );
    push("stop_reason", field("stop_reason"));
    push("deep_audit", field("deep_audit"));
    push(
        "payouts_available",
        payout_meta
            .and_then(|m| get(m, "payouts_available").cloned())
            .unwrap_or(Value::Null),
    );
    push(
        "payouts_permission_missing",
        Value::Bool(payout_meta.is_some_and(|m| {
            get(m, "payouts_permission_missing").is_some_and(kassiber_envelope::truthy)
        })),
    );
    push("suggestions", suggestions);
    push("freshness_checkpoint", checkpoint);
    Ok(outcome)
}
