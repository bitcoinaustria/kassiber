//! Admission gates run before every ported stage; no partial results on refusal.
use crate::codec::{object, text, truth};
use crate::python_text::strip;
use crate::python_text::PythonText;
use crate::snapshot::Snapshot;
use serde_json::{json, Value};
use std::collections::BTreeSet;

type Error = (&'static str, String);
fn nonempty(v: &Value) -> bool {
    !v.is_null() && v != ""
}
fn normalized(v: &Value) -> String {
    strip(&text(v)).py_lower()
}
fn domain(v: &Value) -> Option<&'static str> {
    match normalized(v).as_str() {
        "main" | "mainnet" | "bitcoin" | "liquid" | "liquidv1" => Some("main"),
        "test" | "testnet" | "liquidtestnet" => Some("test"),
        "regtest" | "elements" | "elementsregtest" => Some("regtest"),
        "signet" => Some("signet"),
        _ => None,
    }
}
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
pub fn require_accounting(snapshot: &Snapshot) -> Result<(), Error> {
    if let Some(facts) = &snapshot.book_network {
        let mut environments = BTreeSet::new();
        let mut instances = BTreeSet::new();
        let mut conflict = false;
        let observations = facts["observations"].as_array().ok_or((
            "custody_snapshot_invalid",
            "missing network observations".into(),
        ))?;
        let references = facts["references"].as_array().ok_or((
            "custody_snapshot_invalid",
            "missing network references".into(),
        ))?;
        let mut add = |row: &Value, reference: bool| {
            let (environment, instance, valid) = evidence(row);
            conflict |= !valid;
            if valid {
                if let Some(environment) = environment {
                    if reference || environment == "regtest" {
                        if let Some(instance) = instance {
                            instances.insert(instance);
                        }
                    }
                    environments.insert(environment);
                }
            }
        };
        for wallet in &snapshot.wallets {
            let rows: Vec<_> = observations
                .iter()
                .filter(|r| r["wallet_id"] == wallet["id"])
                .collect();
            if rows.is_empty() {
                add(
                    &json!({"wallet_kind":wallet["kind"],"config_json":wallet["config_json"]}),
                    false,
                );
            } else {
                for row in rows {
                    add(row, false);
                }
            }
        }
        for source in references {
            let row = if source["table"] == "wallet_policy_epochs"
                && !truth(&source["private_material_present"])
                && !truth(&source["source_present"])
            {
                json!({})
            } else {
                json!({"raw_json":object(&source["payload_json"]),"config_json":{"chain":source["chain"],"network":source["network"]}})
            };
            add(&row, true);
        }
        if conflict || environments.len() > 1 || instances.len() > 1 {
            return Err((
                "book_network_review_required",
                "This book contains incompatible network histories".into(),
            ));
        }
        let binding = &facts["binding"];
        if !binding.is_null()
            && (environments
                .iter()
                .any(|e| Some(e.as_str()) != binding["environment"].as_str())
                || instances
                    .iter()
                    .any(|i| Some(i.as_str()) != binding["chain_instance_id"].as_str()))
        {
            return Err((
                "book_network_review_required",
                "Book observations no longer match its network binding".into(),
            ));
        }
    }
    let original = snapshot.profile.get("tax_country").unwrap_or(&Value::Null);
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

#[cfg(test)]
mod tests {
    use super::*;
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
