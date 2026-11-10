use crate::{
    config,
    descriptors::truthy as json_truthy,
    publication::row::{field, or_text, rows, Result},
};
use kassiber_envelope::{dict, get, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use serde_json::Value as Json;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub wallet: Option<String>,
    pub rates: bool,
    pub journals: bool,
    pub auto_pair: bool,
    pub force_full: bool,
}

pub(crate) fn wallets(
    conn: &Connection,
    profile: &str,
    reference: Option<&str>,
) -> Result<Vec<Value>> {
    if let Some(reference) = reference.filter(|v| !v.is_empty()) {
        let found = rows(
            conn,
            "SELECT * FROM wallets WHERE profile_id=? AND (id=? OR lower(label)=lower(?)) ORDER BY label ASC",
            [profile, reference, reference],
        )?;
        if found.is_empty() {
            return Err(AppError::new(
                "not_found",
                format!("Wallet '{reference}' was not found"),
            ));
        }
        return Ok(found);
    }
    rows(
        conn,
        "SELECT * FROM wallets WHERE profile_id=? ORDER BY label ASC",
        [profile],
    )
}

pub(crate) fn wallet_config(row: &Value) -> Result<Json> {
    let raw = or_text(field(row, "config_json"));
    let parsed: Json = serde_json::from_str(if raw.is_empty() { "{}" } else { &raw })
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))?;
    if !parsed.is_object() {
        return Err(AppError::new(
            "internal_error",
            "Wallet configuration must be an object",
        ));
    }
    Ok(parsed)
}

/// `core_wallets.stored_wallet_config(wallet)`: a wallet row's stored
/// configuration, refusing one that is not an object. Planning a sync from a
/// guessed configuration could skip or misroute the wallet's sources.
pub(crate) fn stored_wallet_config(row: &Value) -> Result<Json> {
    wallet_config(row).map_err(|_| {
        let label = field(row, "label");
        AppError::new(
            "invalid_wallet_config",
            format!(
                "Wallet '{}' has an unreadable stored configuration",
                kassiber_envelope::py_str(label)
            ),
        )
        .with_hint("Edit or remove this wallet, then sync again.")
        .with_details(dict([("wallet", label.clone())]))
    })
}

fn wallet_payload(wallet: &Value, force: bool) -> Value {
    let mut payload = vec![
        ("wallet_id", field(wallet, "id").clone()),
        ("wallet_label", field(wallet, "label").clone()),
    ];
    if force {
        payload.push(("force_full", Value::Bool(true)));
    }
    dict(payload)
}

fn spec(
    kind: &str,
    class: &str,
    key: String,
    label: String,
    payload: Value,
    priority: i64,
) -> Value {
    dict([
        ("job_type", s(kind)),
        ("source_type", s(class)),
        ("source_key", s(key)),
        ("source_label", s(label)),
        ("payload", payload),
        ("priority", Value::from(priority)),
    ])
}

fn wallet_spec(
    wallet: &Value,
    kind: &str,
    class: &str,
    label: &str,
    priority: i64,
    force: bool,
) -> Value {
    let mut result = spec(
        kind,
        class,
        format!("{class}:{}", or_text(field(wallet, "id"))),
        format!("{} {label}", or_text(field(wallet, "label"))),
        wallet_payload(wallet, force),
        priority,
    );
    kassiber_envelope::set(&mut result, "single_flight", Value::Bool(!force));
    result
}

pub fn build(conn: &Connection, profile: &str, options: &Options) -> Result<Vec<Value>> {
    let mut specs = Vec::new();
    for wallet in wallets(conn, profile, options.wallet.as_deref())? {
        let mut config = stored_wallet_config(&wallet)?;
        if options.wallet.is_none() && !config::in_sync_all(&config, options.force_full)? {
            continue;
        }
        config::normalize_btcpay(&mut config)?;
        if config::optional_text(config.get("sync_source")).is_some() {
            specs.push(wallet_spec(
                &wallet,
                "btcpay_wallet_source",
                "btcpay_wallet",
                "BTCPay wallet source",
                20,
                options.force_full,
            ));
        }
        let file = config.get("source_file").is_some_and(json_truthy)
            && config.get("source_format").is_some_and(json_truthy);
        if field(&wallet, "kind") != &s("coreln") && !file {
            let descriptor = config::optional_text(config.get("descriptor")).is_some()
                || (config::optional_text(config.get("xpub")).is_some()
                    && config.get("script_types").is_some_and(json_truthy));
            if descriptor || !config::addresses(config.get("addresses"))?.is_empty() {
                specs.push(wallet_spec(
                    &wallet,
                    "onchain_wallet_history",
                    "onchain_wallet",
                    "on-chain history",
                    30,
                    options.force_full,
                ));
            }
        }
        if !config::provenance(config.get("btcpay_provenance").unwrap_or(&Json::Null))?
            .as_array()
            .is_none_or(|v| v.is_empty())
        {
            specs.push(wallet_spec(
                &wallet,
                "btcpay_provenance",
                "btcpay_provenance",
                "BTCPay provenance",
                40,
                options.force_full,
            ));
        }
    }
    if options.wallet.is_none() {
        for route in rows(conn, "SELECT * FROM btcpay_account_routes WHERE profile_id=? ORDER BY backend_name ASC,store_id ASC,payment_method_id ASC,action ASC", [profile])? {
            let mut payload = dict([
                ("account_route_id", field(&route, "id").clone()),
                ("backend", field(&route, "backend_name").clone()),
                ("store_id", field(&route, "store_id").clone()),
                ("payment_method_id", field(&route, "payment_method_id").clone()),
            ]);
            if options.force_full {
                kassiber_envelope::set(&mut payload, "force_full", Value::Bool(true));
            }
            let mut result = spec(
                "btcpay_provenance", "btcpay_provenance",
                format!("btcpay_provenance:account:{}", or_text(field(&route, "id"))),
                format!("BTCPay provenance {} {}", or_text(field(&route, "backend_name")), or_text(field(&route, "store_id"))),
                payload, 40,
            );
            kassiber_envelope::set(&mut result, "single_flight", Value::Bool(!options.force_full));
            specs.push(result);
        }
    }
    if options.rates {
        specs.push(spec(
            "market_rate_coverage",
            "market_rates",
            format!("market_rates:{profile}"),
            "Market-rate coverage".into(),
            dict::<&str>([]),
            70,
        ));
    }
    if options.journals {
        let payload = if options.auto_pair {
            dict([("auto_pair", Value::Bool(true))])
        } else {
            dict::<&str>([])
        };
        specs.push(journal(profile, payload));
    }
    Ok(specs)
}

pub(super) fn journal(profile: &str, payload: Value) -> Value {
    spec(
        "journal_refresh",
        "journals",
        format!("journals:{profile}"),
        "Journal refresh".into(),
        payload,
        80,
    )
}

pub fn filter_by_policy(specs: Vec<Value>, policy: &Value, force: bool) -> Vec<Value> {
    specs
        .into_iter()
        .filter(|spec| {
            force
                || get(
                    field(policy, "source_classes"),
                    &or_text(field(spec, "source_type")),
                )
                .is_some_and(truthy)
        })
        .collect()
}
