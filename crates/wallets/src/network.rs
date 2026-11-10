//! Book binding admission, without borrowing network or ownership authority.
use crate::{config, database_error, descriptors};
use kassiber_envelope::{dict, s, AppError};
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn error(message: &str, operation: Option<&str>) -> AppError {
    AppError::new("book_network_mismatch", message)
        .with_hint(
            "Review the active book's network in Settings. Existing history cannot be relabeled.",
        )
        .with_details(operation.map_or_else(|| dict::<&str>([]), |v| dict([("operation", s(v))])))
}
fn binding(conn: &Connection, profile: &str) -> Result<Option<(String, Option<String>)>, AppError> {
    conn.query_row(
        "SELECT environment,chain_instance_id FROM book_network_bindings WHERE profile_id=?",
        [profile],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(database_error)
}
fn standard_network(environment: &str, chain: &str) -> Option<&'static str> {
    match (environment, chain) {
        ("main", "bitcoin") => Some("main"),
        ("main", "liquid") => Some("liquidv1"),
        ("test", "bitcoin") => Some("test"),
        ("test", "liquid") => Some("liquidtestnet"),
        ("signet", "bitcoin") => Some("signet"),
        ("regtest", "bitcoin") => Some("regtest"),
        ("regtest", "liquid") => Some("elementsregtest"),
        _ => None,
    }
}
pub fn new_config(
    conn: &Connection,
    profile: &str,
    kind: &str,
    mut value: Value,
) -> Result<Value, AppError> {
    if !value.is_object() {
        return Err(AppError::new(
            "validation",
            "Wallet config must be an object",
        ));
    }
    if let Some((environment, instance)) = binding(conn, profile)? {
        if [
            "chain",
            "network",
            "descriptor",
            "change_descriptor",
            "xpub",
            "addresses",
        ]
        .iter()
        .any(|k| value.get(k).is_some_and(descriptors::truthy))
            || ["coreln", "lnd", "lightning", "nwc", "phoenix"].contains(&kind)
        {
            let chain = if ["liquid", "lbtc", "elements"]
                .contains(&descriptors::field(&value, "chain").as_str())
                || kind == "liquid-electrum"
            {
                "liquid"
            } else {
                "bitcoin"
            };
            let network = standard_network(&environment, chain)
                .ok_or_else(|| error("This chain has no domain in the book's environment", None))?;
            let object = value.as_object_mut().expect("configuration object");
            object.entry("chain").or_insert(json!(chain));
            object.entry("network").or_insert(json!(network));
            if let Some(instance) = instance {
                object.entry("chain_instance_id").or_insert(json!(instance));
            }
        }
    }
    Ok(value)
}
#[derive(PartialEq)]
struct Evidence {
    environment: Option<String>,
    instance: Option<String>,
    valid: bool,
}
fn evidence(config: &Value, raw: &Value, asset: &str) -> Evidence {
    let mut raw = raw.clone();
    if raw.get("source") == Some(&json!("bullbitcoin_wallet_csv"))
        && ["bitcoin", "liquid", "lightning"]
            .contains(&descriptors::field(&raw, "network").to_lowercase().as_str())
    {
        raw.as_object_mut()
            .expect("source field implies object")
            .remove("network");
    }
    let mut chains = BTreeSet::new();
    let mut domains = BTreeSet::new();
    let mut instances = BTreeSet::new();
    let mut valid = true;
    for payload in [&raw, config] {
        if let Some(chain) = config::optional_text(payload.get("chain")) {
            let chain = match chain.to_lowercase().as_str() {
                "bitcoin" | "btc" | "onchain" => "bitcoin",
                "liquid" | "lbtc" | "elements" => "liquid",
                "lightning" | "ln" | "cln" | "coreln" | "lnd" | "nwc" | "phoenix" => "bitcoin",
                _ => {
                    valid = false;
                    "invalid"
                }
            };
            chains.insert(chain);
        }
        for key in ["network", "bitcoin_network", "chain_network"] {
            if let Some(network) = config::optional_text(payload.get(key)) {
                let domain = match network.to_lowercase().as_str() {
                    "main" | "mainnet" | "bitcoin" | "liquid" | "liquidv1" => "main",
                    "test" | "testnet" | "liquidtestnet" => "test",
                    "signet" => "signet",
                    "regtest" | "elements" | "elementsregtest" => "regtest",
                    _ => {
                        valid = false;
                        "invalid"
                    }
                };
                domains.insert(domain.to_owned());
            }
        }
        if let Some(instance) = config::optional_text(payload.get("chain_instance_id")) {
            instances.insert(instance);
        }
    }
    valid &= chains.len() <= 1 && domains.len() <= 1 && instances.len() <= 1;
    valid &= !(asset == "BTC" && chains.contains("liquid")
        || ["LBTC", "L-BTC"].contains(&asset) && chains.contains("bitcoin"));
    Evidence {
        environment: domains.into_iter().next(),
        instance: instances.into_iter().next(),
        valid,
    }
}
pub fn guard(
    conn: &Connection,
    profile: &str,
    config: &Value,
    previous: Option<&Value>,
) -> Result<(), AppError> {
    let new = evidence(config, &Value::Null, "");
    if let Some(previous) = previous {
        let old = evidence(previous, &Value::Null, "");
        if old != new && (old.environment.is_some() || old.instance.is_some()) {
            return Err(error(
                "A connection's historical network cannot be changed; create a new connection",
                None,
            ));
        }
    }
    guard_evidence(conn, profile, new, "wallet")
}
fn guard_evidence(
    conn: &Connection,
    profile: &str,
    evidence: Evidence,
    operation: &str,
) -> Result<(), AppError> {
    if !evidence.valid {
        return Err(error(
            "Observation contradicts its configured network",
            Some(operation),
        ));
    }
    if let Some((environment, instance)) = binding(conn, profile)? {
        if evidence.environment.is_some_and(|e| e != environment) {
            return Err(error(
                "Observation belongs to a different book environment",
                Some(operation),
            ));
        }
        if evidence.instance.is_some() && evidence.instance != instance {
            return Err(error(
                "Observation belongs to a different local chain instance",
                Some(operation),
            ));
        }
    }
    Ok(())
}
pub fn guard_import(
    conn: &Connection,
    profile: &str,
    kind: &str,
    config: &Value,
    raw: &Value,
    asset: &str,
) -> Result<(), AppError> {
    let mut raw = raw.clone();
    if kind == "coinbase" && raw.get("network").is_some_and(Value::is_object) {
        // Coinbase's transaction network object holds hash/fee/status data,
        // not a main/test/regtest declaration.
        raw.as_object_mut()
            .expect("network implies object")
            .remove("network");
    }
    if kind == "binance" && asset == "BTC" && raw.get("network") == Some(&json!("BTC")) {
        // Binance names the coin's transfer rail; "BTC" is Bitcoin on chain,
        // and Binance runs on mainnet only. Other rails stay unsupported.
        raw["network"] = json!("main");
    }
    guard_evidence(conn, profile, evidence(config, &raw, asset), "import")
}
pub fn guard_history(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    config: &Value,
) -> Result<(), AppError> {
    let mut stmt = conn
        .prepare("SELECT asset,raw_json FROM transactions WHERE wallet_id=?")
        .map_err(database_error)?;
    let rows = stmt
        .query_map([wallet], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(database_error)?;
    for row in rows {
        let (asset, raw) = row.map_err(database_error)?;
        let raw = raw
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or(Value::Null);
        guard_evidence(
            conn,
            profile,
            evidence(config, &raw, &asset),
            "wallet_update",
        )?;
    }
    Ok(())
}
