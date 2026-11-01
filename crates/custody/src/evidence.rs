//! Port of custody_evidence's canonical identities, commitments and quantities.
use crate::codec::{self, int, integer, text, truth, wide};
use crate::components::Error;
use crate::python_text::PythonText;
use crate::python_text::{isalnum, strip};
use crate::snapshot::Snapshot;
use kassiber_pyjson::{Key, Value as Py};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
mod observation;

fn integer_or_zero(value: &Value) -> Result<Value, Error> {
    if truth(value) {
        crate::components::integer(value)
    } else {
        Ok(json!(0))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventKey {
    pub chain: String,
    pub network: String,
    pub native_namespace: String,
    pub native_event_id: String,
}
impl EventKey {
    pub fn value(&self) -> Value {
        json!({"$type":"CanonicalEventKey", "chain":self.chain, "network":self.network, "native_namespace":self.native_namespace, "native_event_id":self.native_event_id})
    }
    pub fn payload(&self) -> Value {
        let mut v = self.value();
        v.as_object_mut().unwrap().remove("$type");
        v
    }
}
pub fn lightning(kind: &str) -> bool {
    ["lnd", "coreln", "cln", "lightning", "nwc", "phoenix"].contains(&kind)
}
pub fn chain(value: &str) -> Result<String, Error> {
    let normalized = match value {
        "" | "bitcoin" | "btc" | "onchain" => "bitcoin",
        "liquid" | "lbtc" | "elements" => "liquid",
        "lightning" | "ln" => "lightning",
        _ => return Err(format!("Unsupported wallet chain '{value}'").into()),
    };
    Ok(normalized.into())
}
pub fn network(chain: &str, original: &Value) -> Result<String, Error> {
    let v = strip(&text(original)).py_lower();
    let result = if chain == "liquid" {
        match v.as_str() {
            "" | "liquid" | "liquidv1" | "main" | "mainnet" => Some("liquidv1"),
            "liquidtestnet" | "test" | "testnet" => Some("liquidtestnet"),
            "elements" | "elementsregtest" | "regtest" => Some("elementsregtest"),
            _ => None,
        }
    } else {
        match v.as_str() {
            "" | "main" | "mainnet" | "bitcoin" => Some("main"),
            "test" | "testnet" => Some("test"),
            "regtest" => Some("regtest"),
            "signet" => Some("signet"),
            _ => None,
        }
    };
    result.map(str::to_owned).ok_or_else(|| {
        format!(
            "Unsupported {chain} network '{}'",
            if chain == "lightning" {
                codec::pystr(original)
            } else {
                v
            }
        )
        .into()
    })
}
pub fn scope(row: &Value) -> Result<(String, String), Error> {
    scope_fields(|key| &row[key])
}

pub(crate) fn scope_fields<'a>(row: impl Fn(&str) -> &'a Value) -> Result<(String, String), Error> {
    let raw = crate::graph::transfer_object_ref(row("raw_json"));
    let config = crate::graph::transfer_object_ref(if truth(row("config_json")) {
        row("config_json")
    } else {
        row("wallet_config_json")
    });
    let kind = strip(&text(row("wallet_kind"))).py_lower();
    let asset = strip(&text(row("asset"))).py_upper();
    let raw_chain = text(&raw["chain"]);
    let selected = if !raw["chain"].is_null() && raw["chain"] != "" {
        raw_chain
    } else if lightning(&kind) {
        "lightning".into()
    } else if !row("chain").is_null() && *row("chain") != "" {
        text(row("chain"))
    } else if truth(&config["chain"]) {
        text(&config["chain"])
    } else if asset == "LBTC" {
        "liquid".into()
    } else {
        "bitcoin".into()
    };
    let selected = strip(&selected).py_lower();
    let chain = chain(if selected.is_empty() && asset == "LBTC" {
        "liquid"
    } else {
        &selected
    })?;
    let n = if truth(&raw["network"]) {
        &raw["network"]
    } else if truth(row("network")) {
        row("network")
    } else {
        &config["network"]
    };
    let normalized = network(&chain, n)?;
    Ok((chain, normalized))
}
pub fn txid(value: &Value) -> Option<String> {
    let s = strip(&text(value)).py_lower();
    if s.chars().count() != 64 {
        return None;
    }
    let mut high = true;
    for c in s.chars() {
        if c.is_ascii_hexdigit() {
            high = !high;
        } else if !(high && matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')) {
            return None;
        }
    }
    high.then_some(s)
}

pub fn event_key(row: &Value) -> Result<EventKey, Error> {
    let raw = crate::graph::transfer_object_ref(&row["raw_json"]);
    let (chain, network) = scope(row)?;
    let raw_ordered = if let Some(s) = row["raw_json"].as_str() {
        codec::loads(s).unwrap_or(Py::Null)
    } else {
        codec::from_json(&row["raw_json"])?
    };
    fn member<'a>(value: &'a Py, name: &str) -> Option<&'a Py> {
        if let Py::Dict(items) = value {
            items.iter().find_map(|(key, value)| match key {
                Key::Str(key) if key.as_str() == Some(name) => Some(value),
                _ => None,
            })
        } else {
            None
        }
    }
    let mut candidates = Vec::new();
    for payload in [
        Some(&raw_ordered),
        member(&raw_ordered, "tx"),
        member(&raw_ordered, "ownership_graph"),
    ]
    .into_iter()
    .flatten()
    {
        if let Py::Dict(items) = payload {
            for (key, value) in items {
                if let Key::Str(key) = key {
                    let name: String = key
                        .as_str()
                        .unwrap_or_default()
                        .py_lower()
                        .chars()
                        .filter(|c| isalnum(*c))
                        .collect();
                    if [
                        "txid",
                        "txhash",
                        "transactionid",
                        "transactionhash",
                        "onchaintxid",
                    ]
                    .contains(&name.as_str())
                    {
                        candidates.push(codec::to_json(value));
                    }
                }
            }
        }
    }
    let kind = if truth(&row["external_id_kind"]) {
        text(&row["external_id_kind"])
    } else {
        text(&raw["external_id_kind"])
    };
    if kind.py_strip().py_lower() == "txid" {
        candidates.extend([row["native_event_id"].clone(), row["external_id"].clone()]);
    }
    let physical = candidates.iter().find_map(txid);
    let (namespace, id) = if let Some(physical) =
        physical.filter(|_| ["bitcoin", "liquid"].contains(&chain.as_str()))
    {
        ("chain".into(), physical)
    } else {
        let ns = [
            &row["native_namespace"],
            &row["source_ref"],
            &row["backend_name"],
            &raw["_kassiber_provenance"]["import_source"],
            &row["wallet_id"],
        ]
        .into_iter()
        .find(|v| truth(v))
        .map(text)
        .unwrap_or_default()
        .py_strip()
        .py_lower();
        let declared = [&row["native_event_id"], &raw["txid"], &row["external_id"]]
            .into_iter()
            .find(|v| !v.is_null() && **v != Value::String(String::new()));
        let native = declared
            .filter(|v| truth(v))
            .or_else(|| {
                [&row["fingerprint"], &row["id"]]
                    .into_iter()
                    .find(|v| truth(v))
            })
            .map(text)
            .unwrap_or_default()
            .py_strip()
            .py_lower();
        (ns, native)
    };
    if namespace.is_empty() || id.is_empty() {
        return Err("canonical events require chain, network, namespace, and id".into());
    }
    Ok(EventKey {
        chain,
        network,
        native_namespace: namespace,
        native_event_id: id,
    })
}
pub fn quantity_payload(row: &Value, key: &EventKey) -> Result<Value, Error> {
    Ok(
        json!({"schema_version":1,"event":key.payload(),"wallet_id":text(&row["wallet_id"]),"direction":text(&row["direction"]).py_lower(),"asset":text(&row["asset"]).py_upper(),"amount_msat":integer_or_zero(&row["amount"])? ,"fee_msat":integer_or_zero(&row["fee"])? ,"amount_includes_fee":truth(&row["amount_includes_fee"])}),
    )
}
pub fn raw_py(value: &Value, opaque_key: &str) -> Result<Py, Error> {
    if let Some(bytes) = codec::raw_bytes(value) {
        // Evidence canonicalization deliberately uses UTF-8 replacement;
        // json.loads(bytes) in graph parsing uses encoding detection instead.
        return raw_py(
            &Value::String(String::from_utf8_lossy(&bytes).into_owned()),
            opaque_key,
        );
    }
    if let Some(s) = value.as_str() {
        if s.is_empty() {
            return Ok(Py::Dict(vec![]));
        }
        Ok(codec::loads(s)
            .unwrap_or_else(|_| Py::Dict(vec![(Key::Str(opaque_key.into()), Py::Str(s.into()))])))
    } else {
        Ok(codec::from_json(value)?)
    }
}

pub fn observed_hash(row: &Value) -> Result<(String, String), Error> {
    let a = strip(&text(&row["asset"])).to_owned();
    let asset = if a.len() == 64 && a.chars().all(|c| c.is_ascii_hexdigit()) {
        a.py_lower()
    } else if ["l-btc", "lbtc"].contains(&a.py_lower().as_str()) {
        "LBTC".into()
    } else {
        a.py_upper()
    };
    codec::hash(
        &json!({"schema_version":1,"wallet_id":text(&row["wallet_id"]),"external_id":strip(&text(&row["external_id"])).py_lower(),"direction":strip(&text(&row["direction"])).py_lower(),"asset":asset,"amount_msat":integer_or_zero(&row["amount"])? ,"fee_msat":integer_or_zero(&row["fee"])? ,"amount_includes_fee":truth(&row["amount_includes_fee"])}),
        true,
    ).map_err(Error::from)
}
pub fn graph_hash(raw: &Value) -> Result<(String, String), Error> {
    codec::hash_py(&raw_py(raw, "opaque_text")?, false).map_err(Error::encoding)
}
pub fn evidence_snapshot(
    row: &Value,
    key: &EventKey,
    override_raw: Option<Py>,
) -> Result<Value, Error> {
    observation::Commitment::build(row, key, override_raw).map(|snapshot| snapshot.value())
}
/// Native zero-delta suppression uses the historical closed-row predicate,
/// which binds both hashes but does not additionally validate canonical scope.
pub fn current_observation(row: &Value) -> Result<bool, Error> {
    if integer(&row["observation_authority_version"]).ok() != Some(json!(1)) {
        return Ok(false);
    }
    Ok(
        row["observation_graph_hash"] == graph_hash(&row["raw_json"])?.0
            && row["observation_quantity_hash"] == observed_hash(row)?.0,
    )
}

struct Authority {
    authoritative: bool,
    reason: &'static str,
    version: Option<u8>,
    graph: Option<String>,
    quantity: Option<String>,
}

impl Authority {
    fn new(
        authoritative: bool,
        reason: &'static str,
        version: Option<u8>,
        graph: Option<String>,
        quantity: Option<String>,
    ) -> Self {
        Self {
            authoritative,
            reason,
            version,
            graph,
            quantity,
        }
    }

    fn value(self) -> Value {
        json!({"$type":"ChainObservationAuthority","authoritative":self.authoritative,"reason":self.reason,"authority_version":self.version,"graph_hash":self.graph,"quantity_hash":self.quantity})
    }
}

pub fn authority(row: &Value) -> Result<Value, Error> {
    authority_facts(row).map(Authority::value)
}

fn authority_facts(row: &Value) -> Result<Authority, Error> {
    let version = &row["observation_authority_version"];
    if version.is_null() || *version == json!("") {
        return Ok(Authority::new(
            false,
            "provenance_missing",
            None,
            None,
            None,
        ));
    }
    if integer(version).ok() != Some(json!(1)) {
        return Ok(Authority::new(
            false,
            "authority_version_unsupported",
            None,
            None,
            None,
        ));
    }
    let key = match event_key(row) {
        Ok(key) => key,
        Err(_) => {
            return Ok(Authority::new(
                false,
                "canonical_event_invalid",
                Some(1),
                None,
                None,
            ))
        }
    };
    if key.native_namespace != "chain" || !["bitcoin", "liquid"].contains(&key.chain.as_str()) {
        return Ok(Authority::new(
            false,
            "canonical_chain_scope_invalid",
            Some(1),
            None,
            None,
        ));
    }
    let q = observed_hash(row)?.0;
    if text(&row["observation_quantity_hash"])
        .py_strip()
        .py_lower()
        != q
    {
        return Ok(Authority::new(
            false,
            "quantity_hash_mismatch",
            Some(1),
            None,
            Some(q),
        ));
    }
    let g = graph_hash(&row["raw_json"])?.0;
    if strip(&text(&row["observation_graph_hash"])).py_lower() != g {
        return Ok(Authority::new(
            false,
            "graph_hash_mismatch",
            Some(1),
            Some(g),
            Some(q),
        ));
    }
    if row["observation_graph_hash"] != g || row["observation_quantity_hash"] != q {
        return Ok(Authority::new(
            false,
            "authoritative_observation_invalid",
            Some(1),
            Some(g),
            Some(q),
        ));
    }
    Ok(Authority::new(true, "matched", Some(1), Some(g), Some(q)))
}

const FIELDS:&str="id journal_transaction_id profile_id wallet_id wallet_kind fingerprint external_id external_id_kind native_event_id native_namespace source_ref backend_name occurred_at confirmed_at direction asset amount fee amount_includes_fee kind payment_hash payment_hash_source swap_refund_funding_txid swap_refund_funding_vout raw_json config_json wallet_config_json chain network observation_authority_version observation_graph_hash observation_quantity_hash observation_fee_attribution observation_application_revision observation_observed_at";
pub trait RowRecord: Sync {
    fn field(&self, key: &str) -> Option<&Value>;
    fn fields(&self) -> impl Iterator<Item = (&String, &Value)>;
}
impl RowRecord for Value {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
    fn fields(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.as_object().expect("snapshot row").iter()
    }
}
impl RowRecord for serde_json::Map<String, Value> {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
    fn fields(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.iter()
    }
}
impl RowRecord for crate::snapshot::Record {
    fn field(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
    fn fields(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.iter()
    }
}
pub fn enriched_rows<R: RowRecord>(rows: &[R]) -> Result<Vec<Value>, Error> {
    enrich_with_keys(rows).map(|(rows, _)| rows)
}

fn enrich_with_keys<R: RowRecord>(
    rows: &[R],
) -> Result<(Vec<Value>, Vec<Option<EventKey>>), Error> {
    let fields: BTreeSet<&str> = FIELDS.split_whitespace().collect();
    // Rows are enriched independently; keep them, or the first refusal, in order.
    let enrich = |value: &R| -> Result<Value, Error> {
        let row = value;
        // Indexing a JSON object yields null for an absent field.
        let original = |key: &str| row.field(key).unwrap_or(&Value::Null);
        let mut item = Value::Object(
            row.fields()
                .filter(|(key, _)| fields.contains(key.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );
        let auth = authority_facts(&item)?;
        let Value::Object(mut item) = std::mem::take(&mut item) else {
            unreachable!("row object")
        };
        if auth.authoritative {
            item.insert("_kassiber_verified_chain_observation".into(), auth.value());
        }
        let config = crate::graph::transfer_object_ref(original("config_json"));
        let raw = crate::graph::transfer_object_ref(original("raw_json"));
        let mut chain = if truth(&raw["chain"]) {
            raw["chain"].clone()
        } else {
            config["chain"].clone()
        };
        if !truth(&chain) && lightning(text(original("wallet_kind")).py_strip().py_lower().as_str())
        {
            chain = json!("lightning");
        }
        item.insert("chain".into(), chain);
        item.insert(
            "network".into(),
            if truth(&raw["network"]) {
                raw["network"].clone()
            } else {
                config["network"].clone()
            },
        );
        item.remove("config_json");
        Ok(Value::Object(item))
    };
    let mut result = Vec::with_capacity(rows.len());
    for item in crate::par::map(rows, enrich) {
        result.push(crate::par::unwrap(item)?);
    }
    crate::graph::normalize_owned_fees(&mut result)?;
    let keys: Vec<_> = crate::par::map(&result, |row| {
        (row["direction"] == "outbound")
            .then(|| event_key(row).ok())
            .flatten()
    })
    .into_iter()
    .map(crate::par::unwrap)
    .collect();
    let mut groups: BTreeMap<&EventKey, Vec<usize>> = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        if let Some(key) = key {
            if key.native_namespace == "chain" {
                groups.entry(key).or_default().push(i);
            }
        }
    }
    for group in groups.values() {
        let positive: Vec<usize> = group
            .iter()
            .copied()
            .filter(|i| {
                int(&result[*i]["fee"]) > 0 && !truth(&result[*i]["custody_native_fee_normalized"])
            })
            .collect();
        let fees: BTreeSet<i64> = positive.iter().map(|i| int(&result[*i]["fee"])).collect();
        if positive.len() <= 1 || fees.len() != 1 {
            continue;
        }
        let keeper = *positive
            .iter()
            .min_by_key(|i| text(&result[**i]["id"]))
            .unwrap();
        for i in positive {
            if i != keeper {
                let fee = int(&result[i]["fee"]);
                if !truth(&result[i]["amount_includes_fee"]) {
                    let amount = integer_or_zero(&result[i]["amount"])?;
                    result[i]["amount"] = match codec::checked_wide(&amount)
                        .and_then(|amount| amount.checked_add(i128::from(fee)))
                    {
                        Some(sum) => json!(sum),
                        None => codec::number(&(codec::big(&amount) + fee)),
                    };
                }
                result[i]["fee"] = json!(0);
                result[i]["custody_duplicate_event_fee_normalized"] = json!(true);
            }
        }
    }
    Ok((result, keys))
}

pub(crate) fn observation(row: &Value, key: &EventKey) -> Result<Value, Error> {
    observation::Observation::new(row, key).map(|observation| observation.value())
}

// Construction shared with exact native augmentation, whose admission uses BigInt.
pub(crate) fn observation_fields(row: &Value, key: &EventKey) -> Result<Value, Error> {
    let snapshot = evidence_snapshot(row, key, None)?;
    let cached = &row["_kassiber_verified_chain_observation"];
    let auth = if truth(&cached["authoritative"]) {
        cached.clone()
    } else {
        authority(row)?
    };
    let attr = text(&row["observation_fee_attribution"])
        .py_strip()
        .py_lower();
    let attr = if truth(&auth["authoritative"])
        && ["exact", "implicit_wallet_delta"].contains(&attr.as_str())
    {
        attr
    } else {
        "unknown".into()
    };
    let result = json!({"$type":"QuantityObservation","transaction_id":text(&row["id"]),"anchor_transaction_id":if truth(&row["journal_transaction_id"]) {text(&row["journal_transaction_id"])} else {text(&row["id"])},"quantity_hash":snapshot["quantity_hash"],"evidence_detail_hash":snapshot["detail_hash"],"evidence_payload_json":snapshot["payload_json"],"event_key":key.value(),"profile_id":text(&row["profile_id"]),"authoritative_chain_observation":auth["authoritative"],"observation_authority_reason":auth["reason"],"fee_attribution":attr,"wallet_id":text(&row["wallet_id"]),"asset":text(&row["asset"]).py_upper(),"direction":text(&row["direction"]).py_lower(),"amount_msat":integer_or_zero(&row["amount"])? ,"fee_msat":integer_or_zero(&row["fee"])? ,"amount_includes_fee":truth(&row["amount_includes_fee"]),"occurred_at":text(&row["occurred_at"])});
    Ok(result)
}
fn issue(key: &EventKey, code: &str, message: &str, rows: &[&Value], details: Value) -> Value {
    let ids: BTreeSet<String> = rows
        .iter()
        .filter(|row| !row["id"].is_null() && row["id"] != "")
        .map(|row| text(&row["id"]))
        .collect();
    json!({"$type":"CanonicalEventIssue","event_key":key.value(),"code":code,"message":message,"transaction_ids":ids,"details":details})
}
pub fn canonical_input(
    rows: &[Value],
    reconciliations: &Value,
    non_events: &BTreeSet<String>,
) -> Result<Value, Error> {
    canonical_with_keys(rows, reconciliations, non_events, Vec::new())
}

fn canonical_with_keys(
    rows: &[Value],
    reconciliations: &Value,
    non_events: &BTreeSet<String>,
    mut prepared: Vec<Option<EventKey>>,
) -> Result<Value, Error> {
    let mut groups: BTreeMap<EventKey, Vec<&Value>> = BTreeMap::new();
    let mut rejected = Vec::new();
    let mut events = Vec::new();
    for (ordinal, row) in rows.iter().enumerate() {
        if non_events.contains(&codec::pystr(&row["id"])) {
            continue;
        }
        let key = prepared
            .get_mut(ordinal)
            .and_then(Option::take)
            .map_or_else(|| event_key(row), Ok);
        match key {
            Ok(key) => groups.entry(key).or_default().push(row),
            Err(message) if message.is_value_or_type() => {
                let id = if truth(&row["id"]) {
                    text(&row["id"])
                } else if truth(&row["fingerprint"]) {
                    text(&row["fingerprint"])
                } else {
                    format!("row-{ordinal}")
                };
                let key = EventKey {
                    chain: "invalid".into(),
                    network: "invalid".into(),
                    native_namespace: "row".into(),
                    native_event_id: id.py_strip().py_lower(),
                };
                rejected.push(issue(
                    &key,
                    "canonical_event_identity_invalid",
                    &message.message,
                    &[row],
                    json!({}),
                ));
            }
            Err(error) => return Err(error),
        }
    }
    // Groups are independent; build them concurrently and apply the outcomes
    // in key order, so the first refusal and every output keep their order.
    let groups: Vec<_> = groups.into_iter().collect();
    let span = crate::timing::span("evidence.canonical.events");
    let outcomes = crate::par::map(&groups, |(key, group)| {
        canonical_event(key, group, reconciliations)
    });
    drop(span);
    for outcome in outcomes {
        match crate::par::unwrap(outcome)? {
            Ok(event) => events.push(event),
            Err(issue) => rejected.push(issue),
        }
    }
    rejected.sort_by_key(|v| {
        (
            text(&v["event_key"]["chain"]),
            text(&v["event_key"]["network"]),
            text(&v["event_key"]["native_namespace"]),
            text(&v["event_key"]["native_event_id"]),
            text(&v["code"]),
            v["transaction_ids"].to_string(),
        )
    });
    // The object json! would build, with the lists moved rather than copied.
    let mut input = serde_json::Map::new();
    input.insert("$type".into(), json!("CanonicalQuantityInput"));
    input.insert("events".into(), Value::Array(events));
    input.insert("rejected_events".into(), Value::Array(rejected));
    Ok(Value::Object(input))
}

/// One canonical event group: its event, a rejection issue, or a refusal.
fn canonical_event(
    key: &EventKey,
    group: &[&Value],
    reconciliations: &Value,
) -> Result<Result<Value, Value>, Error> {
    if group.iter().any(|r| {
        reconciliations
            .get(text(&r["id"]))
            .is_some_and(Value::is_null)
    }) {
        return Ok(Err(issue(
            key,
            "source_overlap_quantity_unresolved",
            "overlapping wallet receipts lack complete disjoint output coverage",
            group,
            json!({}),
        )));
    }
    let mut observations = Vec::new();
    let mut errors = BTreeSet::new();
    for row in group {
        let built = (|| {
            let mut observation = observation::Observation::new(row, key)?;
            if let Some(proof) = reconciliations
                .get(text(&row["id"]))
                .filter(|v| !v.is_null())
            {
                observation.reconcile(proof)?;
            }
            Ok::<_, Error>(observation)
        })();
        match built {
            Ok(o) => observations.push(o),
            Err(e) if e.is_value_or_type() => {
                errors.insert(e.message);
            }
            Err(error) => return Err(error),
        }
    }
    if !errors.is_empty() {
        return Ok(Err(issue(
            key,
            "canonical_event_leg_invalid",
            "one or more wallet-event aggregate legs are invalid",
            group,
            json!({"errors":errors}),
        )));
    }
    let mut slots: BTreeMap<(&str, &str, &str), Vec<usize>> = BTreeMap::new();
    for (index, observation) in observations.iter().enumerate() {
        slots.entry(observation.slot()).or_default().push(index);
    }
    let mut contradictions = Vec::new();
    for ((wallet, direction, asset), items) in &slots {
        let semantics: BTreeSet<(i128, i128, bool)> = items
            .iter()
            .map(|index| observations[*index].semantics())
            .collect();
        if semantics.len() > 1 {
            contradictions.push(json!({"wallet_id":wallet,"direction":direction,"asset":asset,"semantics":semantics}));
        }
    }
    if !contradictions.is_empty() {
        return Ok(Err(issue(
            key,
            "canonical_event_leg_contradiction",
            "wallet-event aggregate observations disagree",
            group,
            json!({"contradictory_slots":contradictions}),
        )));
    }
    let mut selected = vec![0; observations.len()];
    let mut legs = Vec::with_capacity(slots.len());
    for items in slots.values() {
        let keeper = *items
            .iter()
            .min_by_key(|index| observations[**index].selection())
            .unwrap();
        for index in items {
            selected[*index] = keeper;
        }
        legs.push(observations[keeper].value());
    }
    let mut aliases = Vec::with_capacity(observations.len());
    let mut snapshots = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for (index, item) in observations.iter().enumerate() {
        aliases.push((
            item.transaction_id.as_ref(),
            observations[selected[index]]
                .commitment
                .quantity_hash
                .as_str(),
        ));
        ids.insert(item.transaction_id.as_ref());
        snapshots.insert(item.commitment.detail_hash.as_str(), &item.commitment);
    }
    aliases.sort();
    let snapshots: Vec<_> = snapshots.into_values().map(|item| {
        json!({"$type":"EvidenceSnapshot","quantity_hash":item.quantity_hash,"detail_hash":item.detail_hash,"payload_json":item.payload_json})
    }).collect();
    let mut event = serde_json::Map::new();
    event.insert("$type".into(), json!("CanonicalQuantityEvent"));
    event.insert("event_key".into(), key.value());
    event.insert("legs".into(), Value::Array(legs));
    event.insert("evidence_snapshots".into(), Value::Array(snapshots));
    event.insert("source_transaction_ids".into(), json!(ids));
    event.insert("observation_aliases".into(), json!(aliases));
    Ok(Ok(Value::Object(event)))
}

pub fn compute(snapshot: &Snapshot) -> Result<Value, Error> {
    let span = crate::timing::span("evidence.rows");
    let rows = snapshot.rows();
    drop(span);
    let span = crate::timing::span("evidence.zero_delta");
    let non_events = crate::graph::native_zero_delta_ids(rows, snapshot)?;
    drop(span);
    let span = crate::timing::span("evidence.enriched");
    let (enriched, keys) = enrich_with_keys(&snapshot.transactions)?;
    drop(span);
    let span = crate::timing::span("evidence.receipts");
    let reconciliations = crate::graph::reconcile_receipts(rows, snapshot)?;
    drop(span);
    let span = crate::timing::span("evidence.canonical");
    let canonical = canonical_with_keys(&enriched, &reconciliations, &non_events, keys)?;
    drop(span);
    let _span = crate::timing::span("evidence.value");
    // The object json! would build, with the stage parts moved rather than copied.
    let mut stage = serde_json::Map::new();
    stage.insert("enriched_rows".into(), Value::Array(enriched));
    stage.insert("canonical_input".into(), canonical);
    stage.insert("receipt_reconciliations".into(), reconciliations);
    stage.insert(
        "native_non_event_ids".into(),
        Value::Array(non_events.into_iter().map(Value::String).collect()),
    );
    Ok(Value::Object(stage))
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    #[test]
    fn cpython_event_semantics() {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_semantics.json"
        ))
        .unwrap();
        for v in vectors.iter().filter(|v| v.get("row").is_some()) {
            let result = event_key(&v["row"]);
            if let Some(error) = v.get("error") {
                assert_eq!(json!(result.unwrap_err()), *error, "{}", v["case"]);
            } else {
                assert_eq!(result.unwrap().payload(), v["key"], "{}", v["case"]);
            }
        }
    }
}

#[cfg(test)]
mod byte_policy_tests {
    use super::*;

    #[test]
    fn evidence_bytes_use_utf8_replacement_before_canonical_json() {
        for (bytes, text) in [
            (b"{\"x\":7}".as_slice(), "{\"x\":7}"),
            (b"\xff\0".as_slice(), "\u{fffd}\0"),
            (b"\xff\xfe{\0}\0".as_slice(), "\u{fffd}\u{fffd}{\0}\0"),
            (b"".as_slice(), ""),
        ] {
            let value = codec::to_json(&Py::Bytes(bytes.to_vec()));
            assert_eq!(
                raw_py(&value, "opaque").unwrap(),
                raw_py(&Value::String(text.into()), "opaque").unwrap()
            );
        }
    }
}
