//! Admission gates run before every ported stage; no partial results on refusal.
#[cfg(test)]
use crate::codec::object;
use crate::codec::text;
#[cfg(test)]
use crate::codec::truth;
mod network;
use crate::python_text::strip;
use crate::python_text::PythonText;
use crate::snapshot::BookSnapshot as Snapshot;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeSet;

type Error = (&'static str, String);
#[cfg(test)]
fn nonempty(v: &Value) -> bool {
    !v.is_null() && v != ""
}
fn normalized(v: &Value) -> String {
    strip(&text(v)).py_lower()
}
#[cfg(test)]
fn domain(v: &Value) -> Option<&'static str> {
    match normalized(v).as_str() {
        "main" | "mainnet" | "bitcoin" | "liquid" | "liquidv1" => Some("main"),
        "test" | "testnet" | "liquidtestnet" => Some("test"),
        "regtest" | "elements" | "elementsregtest" => Some("regtest"),
        "signet" => Some("signet"),
        _ => None,
    }
}
#[cfg(test)]
fn domain_evidence(raw: &Value, config: &Value, asset: &Value) -> (Option<String>, bool) {
    let mut families = BTreeSet::new();
    let mut domains = BTreeSet::new();
    for payload in [raw, config] {
        if nonempty(&payload["chain"]) {
            let family = match normalized(&payload["chain"]).replace('_', "-").as_str() {
                "bitcoin" | "btc" | "onchain" | "lightning" | "core-ln" | "core-lightning"
                | "lnd" | "coreln" | "cln" | "nwc" | "phoenix" => "bitcoin",
                "liquid" | "lbtc" | "elements" => "liquid",
                _ => return (None, false),
            };
            families.insert(family);
        }
        for key in ["network", "bitcoin_network", "chain_network"] {
            if nonempty(&payload[key]) {
                let Some(value) = domain(&payload[key]) else {
                    return (None, false);
                };
                domains.insert(value);
            }
        }
    }
    let asset = normalized(asset).replace("l-btc", "lbtc");
    if families.len() > 1
        || domains.len() > 1
        || asset == "btc" && families.contains("liquid")
        || asset == "lbtc" && families.contains("bitcoin")
    {
        return (None, false);
    }
    (domains.first().map(|s| (*s).to_owned()), true)
}
#[cfg(test)]
fn evidence(row: &Value) -> (Option<String>, Option<String>, bool) {
    let mut raw = object(&row["raw_json"]);
    let config = object(if truth(&row["config_json"]) {
        &row["config_json"]
    } else {
        &row["wallet_config_json"]
    });
    let instance_config = object(if truth(&row["wallet_config_json"]) {
        &row["wallet_config_json"]
    } else {
        &row["config_json"]
    });
    if raw["source"] == "bullbitcoin_wallet_csv"
        && ["bitcoin", "liquid", "lightning"].contains(&text(&raw["network"]).py_lower().as_str())
        || row["wallet_kind"] == "coinbase" && raw["network"].is_object()
    {
        raw.as_object_mut().unwrap().remove("network");
    }
    let (mut environment, mut valid) = domain_evidence(&raw, &config, &row["asset"]);
    let columns = ["chain", "network", "bitcoin_network", "chain_network"]
        .into_iter()
        .filter(|k| nonempty(&row[*k]))
        .map(|k| (k.to_owned(), row[k].clone()))
        .collect::<serde_json::Map<_, _>>();
    if !columns.is_empty() {
        for payload in [&raw, &instance_config] {
            let (other, ok) =
                domain_evidence(&Value::Object(columns.clone()), payload, &row["asset"]);
            valid &= ok && (environment.is_none() || other.is_none() || environment == other);
            environment = environment.or(other);
        }
    }
    let instances: BTreeSet<String> = [row, &raw, &instance_config]
        .iter()
        .filter(|v| truth(&v["chain_instance_id"]))
        .map(|v| strip(&text(&v["chain_instance_id"])).to_owned())
        .collect();
    (
        environment,
        if instances.len() == 1 {
            instances.first().cloned()
        } else {
            None
        },
        valid && instances.len() <= 1,
    )
}
pub fn require_accounting(snapshot: &Snapshot) -> Result<(), crate::Refusal> {
    require_accounting_facts(&snapshot.book_network, &snapshot.wallets, &snapshot.profile)
}

/// The accounting gate over its three inputs, shared by the v1 text adapter
/// and loaded snapshots, which keep their transactions as records.
pub(crate) fn require_accounting_facts(
    book_network: &Option<crate::snapshot::NetworkObject<crate::snapshot::NetworkFacts>>,
    wallets: &[crate::snapshot::WalletRef],
    profile: &crate::snapshot::Record,
) -> Result<(), crate::Refusal> {
    network::require(book_network, wallets)?;
    let original = profile.get("tax_country").unwrap_or(&Value::Null);
    require_tax_country(original)
        .map_err(|(code, message)| crate::Refusal::new("components", code, message))
}

/// `require_accounting_facts` of a loaded snapshot, reading its network
/// JSON without typing every observation.
pub(crate) fn require_accounting_loaded(
    book_network: Option<&Value>,
    wallets: &[crate::snapshot::WalletRef],
    profile: &crate::snapshot::Record,
) -> Result<(), crate::Refusal> {
    network::require_value(book_network, wallets)?;
    let original = profile.get("tax_country").unwrap_or(&Value::Null);
    require_tax_country(original)
        .map_err(|(code, message)| crate::Refusal::new("components", code, message))
}

pub(crate) fn require_tax_country(original: &Value) -> Result<(), Error> {
    let country = normalized(original);
    if !["", "generic", "at"].contains(&country.as_str()) {
        // normalize_tax_country currently raises ValueError before AppError.
        return Err((
            "ValueError",
            format!(
                "Unsupported tax country '{}'",
                crate::codec::pystr(original)
            ),
        ));
    }
    Ok(())
}

/// Python loads rates before component/evidence materialization. Parse both
/// candidate columns even when a rate already exists on this row, while
/// retaining the loader's first-successful-row precedence per asset.
pub(crate) fn require_rates(snapshot: &Snapshot) -> Result<(), Error> {
    require_rate_rows(&snapshot.rate_rows)
}

pub(crate) fn require_rate_rows(rows: &[crate::snapshot::RateRow]) -> Result<(), Error> {
    let mut seen = BTreeSet::new();
    for row in rows {
        rate_row(row, &mut seen)?;
    }
    Ok(())
}

/// `require_rate_rows` of loaded records, typing only the rows it reads: a
/// row whose text asset is already seen is skipped before any other column.
/// A record that cannot be typed is the outer error.
pub(crate) fn require_rate_records(
    records: &[crate::snapshot::Record],
) -> Result<Result<(), Error>, String> {
    let mut seen = BTreeSet::new();
    for record in records {
        if let Some(asset @ Value::String(_)) = record.get("asset") {
            if seen.contains(&asset.to_string()) {
                continue;
            }
        }
        let row: crate::snapshot::RateRow = record.typed()?;
        if let Err(error) = rate_row(&row, &mut seen) {
            return Ok(Err(error));
        }
    }
    Ok(Ok(()))
}

/// One rate row's admission, given the assets an earlier row settled.
fn rate_row(row: &crate::snapshot::RateRow, seen: &mut BTreeSet<String>) -> Result<(), Error> {
    use kassiber_pydecimal::{Decimal, DecimalError};
    use std::str::FromStr;
    {
        let asset = row.asset.present().ok_or(("KeyError", "'asset'".into()))?;
        if seen.contains(&asset.json().to_string()) {
            return Ok(());
        }
        let parse = |value: &crate::snapshot::SqlValue| -> Result<(), Error> {
            let raw = if value.is_null() || value.as_str() == Some("") {
                "0".into()
            } else {
                crate::codec::pystr(&value.json())
            };
            match Decimal::from_str(strip(&raw)) {
                Ok(_) | Err(DecimalError::NonFinite) => Ok(()),
                Err(_) => Err((
                    "app_error",
                    format!(
                        "Invalid decimal value: {}",
                        crate::codec::pystr(&value.json())
                    ),
                )),
            }
        };
        let mut rate = false;
        let mut value = false;
        for (column, present) in [
            (&row.fiat_rate_exact, &mut rate),
            (&row.fiat_value_exact, &mut value),
        ] {
            if let Some(v) = column.present().filter(|v| !v.is_null()) {
                parse(v)?;
                *present = true;
            }
        }
        for (key, column, present) in [
            ("fiat_rate", &row.fiat_rate, &mut rate),
            ("fiat_value", &row.fiat_value, &mut value),
        ] {
            if !*present {
                let v = column.present().ok_or(("KeyError", format!("'{key}'")))?;
                if !v.is_null() {
                    parse(v)?;
                    *present = true;
                }
            }
        }
        if rate {
            seen.insert(asset.json().to_string());
        } else if value {
            let amount = row
                .amount
                .present()
                .ok_or(("KeyError", "'amount'".into()))?;
            if amount.truth() {
                crate::codec::integer(&amount.json()).map_err(|message| ("ValueError", message))?;
                seen.insert(asset.json().to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_rate_records_gate_as_typed_rows() {
        let rows = [
            json!({"asset": "BTC", "fiat_rate": "1.5", "fiat_value": null, "amount": 1}),
            json!({"asset": "BTC", "fiat_rate": "bad", "fiat_value": "bad", "amount": 1}),
            json!({"asset": "EUR", "fiat_rate": null, "fiat_value": "2", "amount": 0}),
            json!({"asset": "EUR", "fiat_rate": null, "fiat_value": "3", "amount": "x"}),
            json!({"asset": "EUR", "fiat_rate": null, "fiat_value": "4", "amount": 2}),
            json!({"asset": "EUR", "fiat_rate": "bad", "fiat_value": "bad", "amount": 2}),
            json!({"asset": 7, "fiat_rate": "1", "fiat_value": null, "amount": 1}),
            json!({"asset": 7, "fiat_rate": "bad", "fiat_value": null, "amount": 1}),
            json!({"asset": "ü\"", "fiat_rate": "1", "fiat_value": null, "amount": 1}),
            json!({"asset": "ü\"", "fiat_rate": "bad", "fiat_value": null, "amount": 1}),
            json!({"asset": "LBTC", "fiat_rate": null, "fiat_value": null, "amount": 1}),
            json!({"asset": "LBTC", "fiat_rate_exact": "1e999999999", "fiat_value": null, "amount": 1}),
            json!({"fiat_rate": "1", "fiat_value": null, "amount": 1}),
            json!({"asset": "USD", "fiat_value": null, "amount": 1}),
            json!({"asset": "USD", "fiat_rate": "bad", "fiat_value": null, "amount": 1}),
        ];
        // Every prefix, so each row is the last one read at least once.
        for end in 0..=rows.len() {
            let records: Vec<crate::snapshot::Record> = rows[..end]
                .iter()
                .map(|row| serde_json::from_value(row.clone()).unwrap())
                .collect();
            let typed: Vec<crate::snapshot::RateRow> = records
                .iter()
                .map(|record| record.typed().unwrap())
                .collect();
            assert_eq!(
                require_rate_records(&records).unwrap(),
                require_rate_rows(&typed),
                "{end}"
            );
        }
    }
    #[test]
    fn cpython_network_evidence() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v.get("network_row").is_some()) {
            assert_eq!(
                json!(evidence(&v["network_row"])),
                v["network_evidence"],
                "{}",
                v["case"]
            );
        }
    }
}
