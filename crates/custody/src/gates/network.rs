//! Network inventory admission over typed observation columns. Unknown payload
//! keys are skipped; the complete input remains available at the oracle seam.
use crate::python_text::PythonText;
#[cfg(test)]
use crate::snapshot::BookSnapshot;
use crate::snapshot::{NetworkArray, NetworkFacts, NetworkObject, NetworkRow, SqlValue, WalletRef};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;

type Evidence = (Option<String>, Option<String>, bool);
#[derive(Default, Deserialize)]
#[serde(default)]
struct Payload {
    chain: SqlValue,
    network: SqlValue,
    bitcoin_network: SqlValue,
    chain_network: SqlValue,
    chain_instance_id: SqlValue,
    source: SqlValue,
}
impl Payload {
    fn parse(value: &SqlValue) -> Self {
        match value {
            SqlValue::Text(s) if s.as_ref() == "{}" || s.is_empty() => Self::default(),
            // Serde structs accept positional arrays, while the historical
            // object reader treats every non-object payload as an empty map.
            SqlValue::Text(s) if !s.trim_start().starts_with('{') => Self::default(),
            SqlValue::Text(s) => serde_json::from_str(s).unwrap_or_else(|_| {
                serde_json::from_value(crate::codec::object(&value.json())).unwrap_or_default()
            }),
            SqlValue::Object(_) => serde_json::from_value(value.json()).unwrap_or_default(),
            _ => Self::default(),
        }
    }
}
fn text(v: &SqlValue) -> String {
    if !v.truth() {
        String::new()
    } else if let Some(s) = v.as_str() {
        s.to_owned()
    } else {
        crate::codec::text(&v.json())
    }
}
fn nonempty(v: &SqlValue) -> bool {
    !v.is_null() && v.as_str() != Some("")
}
fn normalized(v: &SqlValue) -> String {
    text(v).py_strip().py_lower()
}
fn domains(raw: &Payload, config: &Payload, asset: &SqlValue) -> (Option<String>, bool) {
    let mut families = BTreeSet::new();
    let mut domains = BTreeSet::new();
    for payload in [raw, config] {
        if nonempty(&payload.chain) {
            let family = match normalized(&payload.chain).replace('_', "-").as_str() {
                "bitcoin" | "btc" | "onchain" | "lightning" | "core-ln" | "core-lightning"
                | "lnd" | "coreln" | "cln" | "nwc" | "phoenix" => "bitcoin",
                "liquid" | "lbtc" | "elements" => "liquid",
                _ => return (None, false),
            };
            families.insert(family);
        }
        for value in [
            &payload.network,
            &payload.bitcoin_network,
            &payload.chain_network,
        ] {
            if nonempty(value) {
                let domain = match normalized(value).as_str() {
                    "main" | "mainnet" | "bitcoin" | "liquid" | "liquidv1" => "main",
                    "test" | "testnet" | "liquidtestnet" => "test",
                    "regtest" | "elements" | "elementsregtest" => "regtest",
                    "signet" => "signet",
                    _ => return (None, false),
                };
                domains.insert(domain);
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
pub(super) fn evidence(row: &NetworkRow) -> Evidence {
    let mut raw = Payload::parse(row.raw_json.value());
    let config = Payload::parse(if row.config_json.truth() {
        row.config_json.value()
    } else {
        row.wallet_config_json.value()
    });
    let instance_config = Payload::parse(if row.wallet_config_json.truth() {
        row.wallet_config_json.value()
    } else {
        row.config_json.value()
    });
    if raw.source.as_str() == Some("bullbitcoin_wallet_csv")
        && ["bitcoin", "liquid", "lightning"].contains(&text(&raw.network).py_lower().as_str())
        || row.wallet_kind.value().as_str() == Some("coinbase")
            && matches!(raw.network, SqlValue::Object(_))
    {
        raw.network = SqlValue::Null;
    }
    let (mut environment, mut valid) = domains(&raw, &config, row.asset.value());
    let columns = Payload {
        chain: row.chain.value().clone(),
        network: row.network.value().clone(),
        bitcoin_network: row.bitcoin_network.value().clone(),
        chain_network: row.chain_network.value().clone(),
        ..Payload::default()
    };
    if [
        &columns.chain,
        &columns.network,
        &columns.bitcoin_network,
        &columns.chain_network,
    ]
    .into_iter()
    .any(nonempty)
    {
        for payload in [&raw, &instance_config] {
            let (other, ok) = domains(&columns, payload, row.asset.value());
            valid &= ok && (environment.is_none() || other.is_none() || environment == other);
            environment = environment.or(other);
        }
    }
    let instances: BTreeSet<_> = [
        row.chain_instance_id.value(),
        &raw.chain_instance_id,
        &instance_config.chain_instance_id,
    ]
    .into_iter()
    .filter(|v| v.truth())
    .map(|v| text(v).py_strip().to_owned())
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
pub(super) fn require(
    book_network: &Option<NetworkObject<NetworkFacts>>,
    wallets: &[WalletRef],
) -> Result<(), crate::Refusal> {
    let Some(NetworkObject::Object(facts)) = book_network else {
        return if book_network.is_none() {
            Ok(())
        } else {
            Err(crate::Refusal::new(
                "components",
                "custody_snapshot_invalid",
                "missing network observations",
            ))
        };
    };
    let Some(NetworkArray::Array(observations)) = &facts.observations else {
        return Err(crate::Refusal::new(
            "components",
            "custody_snapshot_invalid",
            "missing network observations",
        ));
    };
    let references = facts.references.as_array().ok_or_else(|| {
        crate::Refusal::new(
            "components",
            "custody_snapshot_invalid",
            "missing network references",
        )
    })?;
    let mut environments = BTreeSet::new();
    let mut instances = BTreeSet::new();
    let mut conflict = false;
    let mut add = |(environment, instance, valid): Evidence, reference: bool| {
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
    for wallet in wallets {
        let wallet_id = wallet.id.json();
        let mut found = false;
        let empty_row = NetworkRow::default();
        for observation in observations {
            let row = match observation {
                NetworkObject::Object(row) => row,
                NetworkObject::Other(_) => &empty_row,
            };
            if row.wallet_id.json() == wallet_id {
                found = true;
                add(evidence(row), false);
            }
        }
        if !found {
            let row = serde_json::from_value(
                json!({"wallet_kind":wallet.kind.json(),"config_json":wallet.config_json.json()}),
            )
            .expect("network row");
            add(evidence(&row), false);
        }
    }
    for source in references {
        let row: NetworkRow = if source["table"] == "wallet_policy_epochs"
            && !crate::codec::truth(&source["private_material_present"])
            && !crate::codec::truth(&source["source_present"])
        {
            NetworkRow::default()
        } else {
            serde_json::from_value(json!({"raw_json":crate::codec::object(&source["payload_json"]),"config_json":{"chain":source["chain"],"network":source["network"]}})).expect("network reference")
        };
        add(evidence(&row), true);
    }
    if conflict || environments.len() > 1 || instances.len() > 1 {
        let state = if conflict { "conflicted" } else { "mixed" };
        return Err(review_refusal(
            "This book contains incompatible network histories",
            json!({"state":state}),
        ));
    }
    let binding = &facts.binding;
    if !binding.is_null()
        && (environments
            .iter()
            .any(|e| Some(e.as_str()) != binding["environment"].as_str())
            || instances
                .iter()
                .any(|i| Some(i.as_str()) != binding["chain_instance_id"].as_str()))
    {
        return Err(review_refusal(
            "Book observations no longer match its network binding",
            json!({}),
        ));
    }
    Ok(())
}

fn review_refusal(message: &str, details: serde_json::Value) -> crate::Refusal {
    let mut refusal = crate::Refusal::new("components", "book_network_review_required", message);
    refusal.hint = Some(
        "Review the active book's network in Settings. Existing history cannot be relabeled."
            .into(),
    );
    refusal.details = details;
    refusal
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_network_evidence_matches_historical_semantics() {
        let vectors: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for case in vectors.iter().filter(|v| v.get("network_row").is_some()) {
            let row = serde_json::from_value(case["network_row"].clone()).unwrap();
            assert_eq!(
                json!(evidence(&row)),
                case["network_evidence"],
                "{}",
                case["case"]
            );
        }
        for chain in [
            json!(null),
            json!("bitcoin"),
            json!("liquid"),
            json!("lightning"),
            json!("oops"),
            json!(false),
            json!(0),
            json!({}),
        ] {
            for network in [
                json!(null),
                json!("main"),
                json!("regtest"),
                json!("signet"),
                json!("invalid"),
                json!(false),
                json!({}),
            ] {
                for asset in ["BTC", "LBTC", "other"] {
                    let value = json!({"asset":asset,"raw_json":{"chain":chain,"network":network}});
                    let row = serde_json::from_value(value.clone()).unwrap();
                    assert_eq!(evidence(&row), super::super::evidence(&value));
                }
            }
        }
    }

    #[test]
    fn typed_network_payload_arrays_are_not_object_facts() {
        for payload in [
            json!(["bitcoin", "regtest", null, null, null, null]),
            json!(["invalid", "invalid", null, null, null, null]),
        ] {
            let value = json!({"asset":"BTC","raw_json":payload.to_string()});
            let row = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(evidence(&row), super::super::evidence(&value));
            assert_eq!(evidence(&row), (None, None, true));
        }
    }
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    fn snapshot(facts: serde_json::Value) -> BookSnapshot {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_projection_stage.json"
        ))
        .unwrap();
        let mut input = cases[0]["input"].clone();
        input["wallets"] = json!([{"id":"wallet","kind":"custom","config_json":"{}"}]);
        input["book_network"] = facts;
        crate::parse(&input.to_string()).unwrap()
    }
    fn require(snapshot: &BookSnapshot) -> Result<(), crate::Refusal> {
        super::require(&snapshot.book_network, &snapshot.wallets)
    }
    #[test]
    fn typed_network_inventory_admission() {
        let observation = json!({"wallet_id":"wallet","asset":"BTC","raw_json":{"chain":"bitcoin","network":"main"}});
        let clean = json!({"observations":[observation],"references":[],"binding":null});
        assert!(require(&snapshot(clean.clone())).is_ok());
        let mut conflict = clean.clone();
        conflict["observations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"wallet_id":"wallet","raw_json":{"chain":"bitcoin","network":"regtest"}}));
        let conflict = require(&snapshot(conflict)).unwrap_err();
        assert_eq!(conflict.details, json!({"state":"mixed"}));
        assert_eq!(
            conflict.message,
            "This book contains incompatible network histories"
        );
        let mut binding = clean.clone();
        binding["binding"] = json!({"environment":"test","chain_instance_id":null});
        assert_eq!(
            require(&snapshot(binding)).unwrap_err().message,
            "Book observations no longer match its network binding"
        );
        let mut missing = clean.clone();
        missing.as_object_mut().unwrap().remove("references");
        assert_eq!(
            require(&snapshot(missing)).unwrap_err(),
            crate::Refusal::new(
                "components",
                "custody_snapshot_invalid",
                "missing network references"
            )
        );
        let epoch = json!({"table":"wallet_policy_epochs","private_material_present":false,"source_present":false,"chain":"bitcoin","network":"regtest"});
        let mut ignored = clean.clone();
        ignored["references"] = json!([epoch]);
        assert!(require(&snapshot(ignored.clone())).is_ok());
        for key in ["private_material_present", "source_present"] {
            let mut present = ignored.clone();
            present["references"][0][key] = json!(true);
            assert!(require(&snapshot(present)).is_err());
        }
        let mut unobserved = clean.clone();
        unobserved["observations"] = json!([]);
        unobserved["binding"] = json!({"environment":"main"});
        let mut input = snapshot(unobserved);
        input.wallets[0].config_json.0 = Some(SqlValue::Text(
            r#"{"chain":"bitcoin","network":"regtest"}"#.into(),
        ));
        assert!(require(&input).is_err());
    }
}
