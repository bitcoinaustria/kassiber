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
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
struct Payload {
    chain: SqlValue,
    network: SqlValue,
    bitcoin_network: SqlValue,
    chain_network: SqlValue,
    chain_instance_id: SqlValue,
    source: SqlValue,
}
/// Whether `Payload::parse` reads `value` as the empty payload without
/// parsing it.
fn empty_payload(value: &SqlValue) -> bool {
    match value {
        SqlValue::Text(s) => {
            s.as_ref() == "{}"
                || s.is_empty()
                || !s.trim_start().starts_with('{')
                || !s.contains('\\')
                    && !["chain", "network", "source"]
                        .iter()
                        .any(|name| s.contains(name))
        }
        SqlValue::Object(_) => false,
        _ => true,
    }
}
impl Payload {
    fn parse(value: &SqlValue) -> Self {
        match value {
            SqlValue::Text(s) if s.as_ref() == "{}" || s.is_empty() => Self::default(),
            // Serde structs accept positional arrays, while the historical
            // object reader treats every non-object payload as an empty map.
            SqlValue::Text(s) if !s.trim_start().starts_with('{') => Self::default(),
            // Every field name contains "chain", "network" or "source"; a text
            // without them or escapes names none of the fields.
            SqlValue::Text(s)
                if !s.contains('\\')
                    && !["chain", "network", "source"]
                        .iter()
                        .any(|name| s.contains(name)) =>
            {
                Self::default()
            }
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
/// Parsed configuration texts; a book repeats each wallet's on every row.
type Configs = crate::fxhash::HashMap<String, Payload>;

fn parse_config(value: &SqlValue, configs: &mut Configs) -> Payload {
    let SqlValue::Text(text) = value else {
        return Payload::parse(value);
    };
    if let Some(payload) = configs.get(&**text) {
        return payload.clone();
    }
    let payload = Payload::parse(value);
    configs.insert(text.to_string(), payload.clone());
    payload
}

pub(super) fn evidence(row: &NetworkRow) -> Evidence {
    evidence_with(row, &mut Configs::default())
}

fn evidence_with(row: &NetworkRow, configs: &mut Configs) -> Evidence {
    let mut raw = Payload::parse(row.raw_json.value());
    let config = parse_config(
        if row.config_json.truth() {
            row.config_json.value()
        } else {
            row.wallet_config_json.value()
        },
        configs,
    );
    let instance_config = parse_config(
        if row.wallet_config_json.truth() {
            row.wallet_config_json.value()
        } else {
            row.config_json.value()
        },
        configs,
    );
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
/// The evidence of observations whose payload is empty, by the columns it
/// then depends on. Evidence is a pure function of those columns, and the
/// gate only collects it into sets, so a repeated row adds nothing new.
#[derive(Default)]
struct Remembered<'a> {
    seen: Vec<([&'a SqlValue; 9], Evidence)>,
}
impl<'a> Remembered<'a> {
    /// Distinct column sets beyond this are computed without remembering.
    const LIMIT: usize = 64;

    fn evidence(&mut self, row: &'a NetworkRow, configs: &mut Configs) -> Evidence {
        if !empty_payload(row.raw_json.value()) {
            return evidence_with(row, configs);
        }
        let key = [
            row.config_json.value(),
            row.wallet_config_json.value(),
            row.wallet_kind.value(),
            row.asset.value(),
            row.chain.value(),
            row.network.value(),
            row.bitcoin_network.value(),
            row.chain_network.value(),
            row.chain_instance_id.value(),
        ];
        if let Some((_, evidence)) = self.seen.iter().find(|(known, _)| *known == key) {
            return evidence.clone();
        }
        let evidence = evidence_with(row, configs);
        if self.seen.len() < Self::LIMIT {
            self.seen.push((key, evidence.clone()));
        }
        evidence
    }
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
    let mut configs = Configs::default();
    let empty_row = NetworkRow::default();
    let mut remembered = Remembered::default();
    for wallet in wallets {
        let wallet_id = wallet.id.json();
        // Text identifiers compare as the strings their JSON holds.
        let same = |column: &crate::snapshot::Column| match (
            column.value().as_str(),
            wallet_id.as_str(),
        ) {
            (Some(a), Some(b)) => a == b,
            _ => column.json() == wallet_id,
        };
        let mut found = false;
        for observation in observations {
            let row = match observation {
                NetworkObject::Object(row) => row,
                NetworkObject::Other(_) => &empty_row,
            };
            if same(&row.wallet_id) {
                found = true;
                add(remembered.evidence(row, &mut configs), false);
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
    fn remembered_evidence_matches_each_row() {
        let rows: Vec<NetworkRow> = [
            json!({"wallet_id":"a","config_json":"{\"chain\":\"bitcoin\",\"network\":\"regtest\"}","raw_json":"{}"}),
            json!({"wallet_id":"b","config_json":"{\"chain\":\"liquid\",\"network\":\"liquidv1\"}","raw_json":"{}"}),
            json!({"wallet_id":"a","config_json":"{\"chain\":\"bitcoin\",\"network\":\"regtest\"}","raw_json":"{\"x\":1}"}),
            json!({"wallet_id":"c","config_json":"{}","raw_json":"{\"network\":\"testnet\"}"}),
            json!({"wallet_id":"c","config_json":"{}","raw_json":"{}","network":"main"}),
            json!({"wallet_id":"b","config_json":"{\"chain\":\"liquid\",\"network\":\"liquidv1\"}","raw_json":"{}","chain_instance_id":"i"}),
        ]
        .into_iter()
        .map(|row| serde_json::from_value(row).unwrap())
        .collect();
        let mut remembered = Remembered::default();
        let mut configs = Configs::default();
        for _ in 0..2 {
            for row in &rows {
                assert_eq!(remembered.evidence(row, &mut configs), evidence(row));
            }
        }
    }
    #[test]
    fn empty_payloads_parse_empty() {
        let texts = [
            "{}",
            "",
            "[1]",
            "  {\"a\":1}",
            "{\"chain\":\"bitcoin\"}",
            "{\"ch\\u0061in\":\"bitcoin\"}",
            "{\"network\":\"regtest\",\"source\":\"x\"}",
            "{\"chained\":1}",
            "not json",
        ];
        let mut values: Vec<SqlValue> = texts.iter().map(|t| SqlValue::Text((*t).into())).collect();
        values.extend([
            SqlValue::Null,
            SqlValue::Int(3),
            SqlValue::Object(vec![("chain".into(), SqlValue::Text("liquid".into()))]),
        ]);
        let mut empty = 0;
        for value in &values {
            if empty_payload(value) {
                empty += 1;
                assert_eq!(Payload::parse(value), Payload::default(), "{value:?}");
            }
        }
        assert_eq!(empty, 7);
    }
    #[test]
    fn network_config_cache_reads_as_parse() {
        let row: NetworkRow = serde_json::from_value(json!({
            "wallet_id": "w",
            "config_json": "{\"chain\":\"bitcoin\",\"network\":\"regtest\"}",
        }))
        .unwrap();
        let expected = evidence(&row);
        assert_ne!(expected, evidence(&NetworkRow::default()));
        let mut configs = Configs::default();
        for _ in 0..2 {
            assert_eq!(evidence_with(&row, &mut configs), expected);
        }
    }
    #[test]
    fn typed_network_evidence_matches_historical_semantics() {
        let vectors: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        // One configuration cache serves every row, as the gate shares it.
        let mut configs = Configs::default();
        for case in vectors.iter().filter(|v| v.get("network_row").is_some()) {
            let row = serde_json::from_value(case["network_row"].clone()).unwrap();
            for _ in 0..2 {
                assert_eq!(
                    json!(evidence_with(&row, &mut configs)),
                    case["network_evidence"],
                    "{}",
                    case["case"]
                );
            }
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
            // Field names in a positional array still name no fields.
            json!(["liquid", "network", null, null, "chain", "source"]),
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
        // Wallet ids match observations by their JSON value.
        let regtest = json!({"wallet_id":7,"raw_json":{"chain":"bitcoin","network":"regtest"}});
        let mut numeric = clean.clone();
        numeric["observations"] =
            json!([{"wallet_id":7,"raw_json":{"chain":"bitcoin","network":"main"}}, regtest]);
        let mut input = snapshot(numeric.clone());
        input.wallets[0].id.0 = Some(SqlValue::Int(7));
        assert!(require(&input).is_err());
        numeric["observations"][1]["wallet_id"] = json!("7");
        let mut input = snapshot(numeric);
        input.wallets[0].id.0 = Some(SqlValue::Int(7));
        assert!(require(&input).is_ok());
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
