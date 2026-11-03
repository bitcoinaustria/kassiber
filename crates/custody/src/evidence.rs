//! Port of custody_evidence's canonical identities, commitments and quantities.
use crate::codec::{self, int, integer, text, text_ref, truth, wide};
use crate::components::Error;
use crate::fields::Fields;
use crate::python_text::PythonText;
use crate::python_text::{isalnum, strip};
use crate::snapshot::Record;
use crate::snapshot::Snapshot;
use kassiber_pyjson::{Key, Value as Py};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
pub mod canonical;
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
        let mut value = serde_json::Map::new();
        value.insert("$type".into(), Value::String("CanonicalEventKey".into()));
        value.insert("chain".into(), Value::String(self.chain.clone()));
        value.insert("network".into(), Value::String(self.network.clone()));
        value.insert(
            "native_namespace".into(),
            Value::String(self.native_namespace.clone()),
        );
        value.insert(
            "native_event_id".into(),
            Value::String(self.native_event_id.clone()),
        );
        Value::Object(value)
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

fn scope_of<F: Fields + ?Sized>(row: &F) -> Result<(String, String), Error> {
    let raw = match row.raw_text() {
        Some(raw) => crate::graph::raw_object(raw),
        None => crate::graph::transfer_object_ref(row.at("raw_json")),
    };
    scope_with(|key| row.at(key), raw)
}

pub(crate) fn scope_fields<'a>(row: impl Fn(&str) -> &'a Value) -> Result<(String, String), Error> {
    scope_with(&row, crate::graph::transfer_object_ref(row("raw_json")))
}

/// `scope_fields` of a row whose `raw_json` object is `raw`.
fn scope_with<'a>(
    row: impl Fn(&str) -> &'a Value,
    raw: crate::graph::TransferValue<'_>,
) -> Result<(String, String), Error> {
    let config = crate::graph::transfer_object_ref(if truth(row("config_json")) {
        row("config_json")
    } else {
        row("wallet_config_json")
    });
    // Borrowed where no case changes; the readers below only compare them.
    let kind = text_ref(row("wallet_kind"));
    let kind = crate::python_text::lower(strip(&kind));
    let asset = text_ref(row("asset"));
    let asset = crate::python_text::upper(strip(&asset));
    let raw_chain = raw.field("chain");
    let selected = if !raw_chain.is_null() && *raw_chain != "" {
        text(&raw_chain)
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
    let raw_network = raw.field("network");
    let n = if truth(&raw_network) {
        &*raw_network
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
    event_key_of(row)
}

/// Whether a payload key names a transaction id once lowercased and reduced
/// to its alphanumeric characters.
fn txid_name(key: &str) -> bool {
    const NAMES: [&str; 5] = [
        "txid",
        "txhash",
        "transactionid",
        "transactionhash",
        "onchaintxid",
    ];
    if key.is_ascii() {
        // ASCII lowercasing and `isalnum` act per byte; no name is longer
        // than 15 characters.
        let mut name = [0; 15];
        let mut len = 0;
        for byte in key.bytes().filter(u8::is_ascii_alphanumeric) {
            if len == name.len() {
                return false;
            }
            name[len] = byte.to_ascii_lowercase();
            len += 1;
        }
        return NAMES.iter().any(|known| known.as_bytes() == &name[..len]);
    }
    let name: String = key.py_lower().chars().filter(|c| isalnum(*c)).collect();
    NAMES.contains(&name.as_str())
}

pub(crate) fn event_key_of<F: Fields + ?Sized>(row: &F) -> Result<EventKey, Error> {
    // The JSON object of `raw_json` is read only by some branches below.
    let raw = std::cell::OnceCell::new();
    let raw = || {
        raw.get_or_init(|| match row.raw_text() {
            Some(raw) => crate::graph::raw_object(raw),
            None => crate::graph::transfer_object_ref(row.at("raw_json")),
        })
    };
    let (chain, network) = scope_of(row)?;
    let null = Py::Null;
    let parsed;
    let raw_ordered = match (row.at("raw_json").as_str(), row.loaded_raw()) {
        (Some(_), Some(loaded)) => loaded.as_ref().unwrap_or(&null),
        (Some(s), None) => {
            parsed = codec::loads(s).unwrap_or(Py::Null);
            &parsed
        }
        (None, _) => {
            parsed = codec::from_json(row.at("raw_json"))?;
            &parsed
        }
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
        Some(raw_ordered),
        member(raw_ordered, "tx"),
        member(raw_ordered, "ownership_graph"),
    ]
    .into_iter()
    .flatten()
    {
        if let Py::Dict(items) = payload {
            for (key, value) in items {
                if let Key::Str(key) = key {
                    if txid_name(key.as_str().unwrap_or_default()) {
                        candidates.push(codec::to_json(value));
                    }
                }
            }
        }
    }
    let kind = if truth(row.at("external_id_kind")) {
        text(row.at("external_id_kind"))
    } else {
        text(&raw().field("external_id_kind"))
    };
    if kind.py_strip().py_lower() == "txid" {
        candidates.extend([
            row.at("native_event_id").clone(),
            row.at("external_id").clone(),
        ]);
    }
    let physical = candidates.iter().find_map(txid);
    let (namespace, id) = if let Some(physical) =
        physical.filter(|_| ["bitcoin", "liquid"].contains(&chain.as_str()))
    {
        ("chain".into(), physical)
    } else {
        let ns = [
            row.at("native_namespace"),
            row.at("source_ref"),
            row.at("backend_name"),
            &raw().field("_kassiber_provenance")["import_source"],
            row.at("wallet_id"),
        ]
        .into_iter()
        .find(|v| truth(v))
        .map(text)
        .unwrap_or_default()
        .py_strip()
        .py_lower();
        let raw_txid = raw().field("txid");
        let declared = [row.at("native_event_id"), &*raw_txid, row.at("external_id")]
            .into_iter()
            .find(|v| !v.is_null() && **v != Value::String(String::new()));
        let native = declared
            .filter(|v| truth(v))
            .or_else(|| {
                [row.at("fingerprint"), row.at("id")]
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
    quantity_payload_of(row, key)
}

fn quantity_payload_of<F: Fields + ?Sized>(row: &F, key: &EventKey) -> Result<Value, Error> {
    Ok(
        json!({"schema_version":1,"event":key.payload(),"wallet_id":text(row.at("wallet_id")),"direction":text(row.at("direction")).py_lower(),"asset":text(row.at("asset")).py_upper(),"amount_msat":integer_or_zero(row.at("amount"))? ,"fee_msat":integer_or_zero(row.at("fee"))? ,"amount_includes_fee":truth(row.at("amount_includes_fee"))}),
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
    observed_hash_of(row)
}

fn observed_hash_of<F: Fields + ?Sized>(row: &F) -> Result<(String, String), Error> {
    let a = strip(&text(row.at("asset"))).to_owned();
    let asset = if a.len() == 64 && a.chars().all(|c| c.is_ascii_hexdigit()) {
        a.py_lower()
    } else if ["l-btc", "lbtc"].contains(&a.py_lower().as_str()) {
        "LBTC".into()
    } else {
        a.py_upper()
    };
    codec::hash(
        &json!({"schema_version":1,"wallet_id":text(row.at("wallet_id")),"external_id":strip(&text(row.at("external_id"))).py_lower(),"direction":strip(&text(row.at("direction"))).py_lower(),"asset":asset,"amount_msat":integer_or_zero(row.at("amount"))? ,"fee_msat":integer_or_zero(row.at("fee"))? ,"amount_includes_fee":truth(row.at("amount_includes_fee"))}),
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
    current_observation_of(row)
}

pub(crate) fn current_observation_of<F: Fields + ?Sized>(row: &F) -> Result<bool, Error> {
    if integer(row.at("observation_authority_version")).ok() != Some(json!(1)) {
        return Ok(false);
    }
    Ok(
        *row.at("observation_graph_hash") == graph_hash(row.at("raw_json"))?.0
            && *row.at("observation_quantity_hash") == observed_hash_of(row)?.0,
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

fn authority_facts<F: Fields + ?Sized>(row: &F) -> Result<Authority, Error> {
    let version = row.at("observation_authority_version");
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
    let key = match event_key_of(row) {
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
    let q = observed_hash_of(row)?.0;
    if text(row.at("observation_quantity_hash"))
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
    let g = graph_hash(row.at("raw_json"))?.0;
    if strip(&text(row.at("observation_graph_hash"))).py_lower() != g {
        return Ok(Authority::new(
            false,
            "graph_hash_mismatch",
            Some(1),
            Some(g),
            Some(q),
        ));
    }
    if *row.at("observation_graph_hash") != g || *row.at("observation_quantity_hash") != q {
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

/// The row fields enrichment retains.
fn retained(key: &str) -> bool {
    matches!(
        key,
        "id" | "journal_transaction_id"
            | "profile_id"
            | "wallet_id"
            | "wallet_kind"
            | "fingerprint"
            | "external_id"
            | "external_id_kind"
            | "native_event_id"
            | "native_namespace"
            | "source_ref"
            | "backend_name"
            | "occurred_at"
            | "confirmed_at"
            | "direction"
            | "asset"
            | "amount"
            | "fee"
            | "amount_includes_fee"
            | "kind"
            | "payment_hash"
            | "payment_hash_source"
            | "swap_refund_funding_txid"
            | "swap_refund_funding_vout"
            | "raw_json"
            | "config_json"
            | "wallet_config_json"
            | "chain"
            | "network"
            | "observation_authority_version"
            | "observation_graph_hash"
            | "observation_quantity_hash"
            | "observation_fee_attribution"
            | "observation_application_revision"
            | "observation_observed_at"
    )
}
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

/// The retained fields of a source row, before enrichment adds its own.
struct Retained<'a, R>(&'a R);
impl<R: RowRecord> Fields for Retained<'_, R> {
    fn get(&self, key: &str) -> Option<&Value> {
        if retained(key) {
            self.0.field(key)
        } else {
            None
        }
    }
}

/// One enriched row: its source row's index and the fields enrichment and
/// fee normalization set. The source row is never copied.
#[derive(Clone, Debug)]
pub struct Enriched {
    row: usize,
    verified: Option<Value>,
    chain: Value,
    network: Value,
    amount: Option<Value>,
    fee: Option<Value>,
    attribution: Option<Value>,
    native_normalized: Option<Value>,
    duplicate_normalized: Option<Value>,
    /// `codec::loads` of a string `raw_json`, parsed once for every reader.
    raw: Option<std::sync::Arc<crate::graph::RawText>>,
}
impl Enriched {
    pub fn row(&self) -> usize {
        self.row
    }
    pub(crate) fn view<'a, R>(&'a self, rows: &'a [R]) -> EnrichedRow<'a, R> {
        EnrichedRow {
            facts: self,
            row: &rows[self.row],
        }
    }
}

/// An enriched row read through its source row.
pub(crate) struct EnrichedRow<'a, R> {
    facts: &'a Enriched,
    row: &'a R,
}
impl<R> Clone for EnrichedRow<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R> Copy for EnrichedRow<'_, R> {}
impl<'a, R: RowRecord> EnrichedRow<'a, R> {
    fn overrides(&self) -> [(&'static str, Option<&'a Value>); 8] {
        let f = self.facts;
        [
            ("_kassiber_verified_chain_observation", f.verified.as_ref()),
            ("chain", Some(&f.chain)),
            ("network", Some(&f.network)),
            ("amount", f.amount.as_ref()),
            ("fee", f.fee.as_ref()),
            ("observation_fee_attribution", f.attribution.as_ref()),
            (
                "custody_native_fee_normalized",
                f.native_normalized.as_ref(),
            ),
            (
                "custody_duplicate_event_fee_normalized",
                f.duplicate_normalized.as_ref(),
            ),
        ]
    }
    /// The enriched row as the JSON object Python builds.
    pub fn value(&self) -> Value {
        let mut item: serde_json::Map<String, Value> = self
            .row
            .fields()
            .filter(|(key, _)| retained(key) && key.as_str() != "config_json")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (key, value) in self.overrides() {
            if let Some(value) = value {
                item.insert(key.into(), value.clone());
            }
        }
        Value::Object(item)
    }
}
impl<R: RowRecord> Fields for EnrichedRow<'_, R> {
    fn get(&self, key: &str) -> Option<&Value> {
        let f = self.facts;
        match key {
            "_kassiber_verified_chain_observation" => f.verified.as_ref(),
            "chain" => Some(&f.chain),
            "network" => Some(&f.network),
            "config_json" => None,
            "amount" => f.amount.as_ref().or_else(|| self.row.field(key)),
            "fee" => f.fee.as_ref().or_else(|| self.row.field(key)),
            "observation_fee_attribution" => f.attribution.as_ref().or_else(|| self.row.field(key)),
            "custody_native_fee_normalized" => f.native_normalized.as_ref(),
            "custody_duplicate_event_fee_normalized" => f.duplicate_normalized.as_ref(),
            _ if retained(key) => self.row.field(key),
            _ => None,
        }
    }

    fn loaded_raw(&self) -> Option<&Result<Py, ()>> {
        self.facts.raw.as_deref().map(crate::graph::RawText::loaded)
    }

    fn raw_text(&self) -> Option<&std::sync::Arc<crate::graph::RawText>> {
        self.facts.raw.as_ref()
    }
}

pub fn enriched_rows<R: RowRecord>(rows: &[R]) -> Result<Vec<Value>, Error> {
    let (enriched, _) = enrich_with_keys(rows)?;
    Ok(enriched.iter().map(|e| e.view(rows).value()).collect())
}

fn enrich_with_keys<R: RowRecord>(
    rows: &[R],
) -> Result<(Vec<Enriched>, Vec<Option<EventKey>>), Error> {
    // Rows are enriched independently; keep them, or the first refusal, in order.
    let enrich = |(index, row): (usize, &R)| -> Result<Enriched, Error> {
        // Indexing a JSON object yields null for an absent field.
        let original = |key: &str| row.field(key).unwrap_or(&Value::Null);
        let auth = authority_facts(&Retained(row))?;
        let config = crate::graph::transfer_object_ref(original("config_json"));
        // The loaded text is shared with every later reader of it.
        let loaded = original("raw_json").as_str().map(crate::graph::raw_text);
        let raw = match &loaded {
            Some(loaded) => crate::graph::raw_object(loaded),
            None => crate::graph::transfer_object_ref(original("raw_json")),
        };
        let raw_chain = raw.field("chain");
        let mut chain = if truth(&raw_chain) {
            raw_chain.into_owned()
        } else {
            config["chain"].clone()
        };
        if !truth(&chain) && lightning(text(original("wallet_kind")).py_strip().py_lower().as_str())
        {
            chain = json!("lightning");
        }
        let raw_network = raw.field("network");
        let network = if truth(&raw_network) {
            raw_network.into_owned()
        } else {
            config["network"].clone()
        };
        Ok(Enriched {
            row: index,
            verified: auth.authoritative.then(|| auth.value()),
            chain,
            network,
            amount: None,
            fee: None,
            attribution: None,
            native_normalized: None,
            duplicate_normalized: None,
            raw: loaded,
        })
    };
    let mut result = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        result.push(enrich((index, row))?);
    }
    // Owned-fee normalization reads and writes only verified rows; run it on
    // their JSON objects and keep the fields it may set.
    let verified: Vec<usize> = (0..result.len())
        .filter(|i| result[*i].verified.as_ref().is_some_and(Value::is_object))
        .collect();
    if !verified.is_empty() {
        let mut values: Vec<Value> = verified
            .iter()
            .map(|i| result[*i].view(rows).value())
            .collect();
        crate::graph::normalize_owned_fees(&mut values)?;
        for (i, value) in verified.into_iter().zip(values) {
            let e = &mut result[i];
            e.amount = value.get("amount").cloned();
            e.fee = value.get("fee").cloned();
            e.attribution = value.get("observation_fee_attribution").cloned();
            e.native_normalized = value.get("custody_native_fee_normalized").cloned();
        }
    }
    let keys: Vec<_> = result
        .iter()
        .map(|e| {
            let row = e.view(rows);
            (*row.at("direction") == "outbound")
                .then(|| event_key_of(&row).ok())
                .flatten()
        })
        .collect();
    let mut groups: BTreeMap<&EventKey, Vec<usize>> = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        if let Some(key) = key {
            if key.native_namespace == "chain" {
                groups.entry(key).or_default().push(i);
            }
        }
    }
    fn field<R: RowRecord>(result: &[Enriched], rows: &[R], i: usize, key: &str) -> Value {
        result[i].view(rows).at(key).clone()
    }
    for group in groups.values() {
        let positive: Vec<usize> = group
            .iter()
            .copied()
            .filter(|i| {
                int(&field(&result, rows, *i, "fee")) > 0
                    && !truth(&field(&result, rows, *i, "custody_native_fee_normalized"))
            })
            .collect();
        let fees: BTreeSet<i64> = positive
            .iter()
            .map(|i| int(&field(&result, rows, *i, "fee")))
            .collect();
        if positive.len() <= 1 || fees.len() != 1 {
            continue;
        }
        let keeper = *positive
            .iter()
            .min_by_key(|i| text(&field(&result, rows, **i, "id")))
            .unwrap();
        for i in positive {
            if i != keeper {
                let fee = int(&field(&result, rows, i, "fee"));
                if !truth(&field(&result, rows, i, "amount_includes_fee")) {
                    let amount = integer_or_zero(&field(&result, rows, i, "amount"))?;
                    result[i].amount = Some(
                        match codec::checked_wide(&amount)
                            .and_then(|amount| amount.checked_add(i128::from(fee)))
                        {
                            Some(sum) => json!(sum),
                            None => codec::number(&(codec::big(&amount) + fee)),
                        },
                    );
                }
                result[i].fee = Some(json!(0));
                result[i].duplicate_normalized = Some(json!(true));
            }
        }
    }
    Ok((result, keys))
}

pub(crate) fn observation<F: Fields + ?Sized>(row: &F, key: &EventKey) -> Result<Value, Error> {
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
fn issue<F: Fields>(
    key: &EventKey,
    code: &str,
    message: &str,
    rows: &[F],
    details: Value,
) -> Value {
    let ids: BTreeSet<String> = rows
        .iter()
        .filter(|row| !row.at("id").is_null() && *row.at("id") != "")
        .map(|row| text(row.at("id")))
        .collect();
    json!({"$type":"CanonicalEventIssue","event_key":key.value(),"code":code,"message":message,"transaction_ids":ids,"details":details})
}
/// `canonical_input` of `rows`' enriched rows, typed and without the
/// enriched rows' JSON objects.
pub(crate) fn canonical_of_rows<R: RowRecord>(rows: &[R]) -> Result<canonical::Canonical, Error> {
    let (enriched, keys) = enrich_with_keys(rows)?;
    let views: Vec<_> = enriched.iter().map(|e| e.view(rows)).collect();
    canonical_with_keys(&views, &json!({}), &BTreeSet::new(), keys)
}

pub fn canonical_input(
    rows: &[Value],
    reconciliations: &Value,
    non_events: &BTreeSet<String>,
) -> Result<Value, Error> {
    canonical_with_keys(rows, reconciliations, non_events, Vec::new())
        .map(canonical::Canonical::into_value)
}

fn canonical_with_keys<F: Fields + Sync>(
    rows: &[F],
    reconciliations: &Value,
    non_events: &BTreeSet<String>,
    mut prepared: Vec<Option<EventKey>>,
) -> Result<canonical::Canonical, Error> {
    let mut keyed: Vec<(EventKey, &F)> = Vec::with_capacity(rows.len());
    let mut rejected = Vec::new();
    let mut events = Vec::new();
    for (ordinal, row) in rows.iter().enumerate() {
        if !non_events.is_empty() && non_events.contains(&codec::pystr(row.at("id"))) {
            continue;
        }
        let key = prepared
            .get_mut(ordinal)
            .and_then(Option::take)
            .map_or_else(|| event_key_of(row), Ok);
        match key {
            Ok(key) => keyed.push((key, row)),
            Err(message) if message.is_value_or_type() => {
                let id = if truth(row.at("id")) {
                    text(row.at("id"))
                } else if truth(row.at("fingerprint")) {
                    text(row.at("fingerprint"))
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
    // Groups are independent; build them in key order, so the first refusal
    // and every output keep their order.
    let span = crate::timing::span("evidence.canonical.events");
    // A stable sort keeps each group's rows in row order.
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let mut keyed = keyed.into_iter().peekable();
    let mut group = Vec::new();
    while let Some((key, row)) = keyed.next() {
        group.clear();
        group.push(row);
        while let Some((_, row)) = keyed.next_if(|(next, _)| *next == key) {
            group.push(row);
        }
        match canonical_event(key, &group, reconciliations)? {
            Ok(event) => events.push(event),
            Err(issue) => rejected.push(issue),
        }
    }
    drop(span);
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
    Ok(canonical::Canonical::new(events, rejected))
}

/// One canonical event group: its event, a rejection issue, or a refusal.
fn canonical_event<F: Fields>(
    key: EventKey,
    group: &[&F],
    reconciliations: &Value,
) -> Result<Result<canonical::Event, Value>, Error> {
    // The typed key and its object, shared by the event and its legs.
    let keys = canonical::Keys::new(key);
    let key = &keys.key;
    let reconciled = |row: &F| {
        if reconciliations.as_object().is_some_and(|m| m.is_empty()) {
            None
        } else {
            reconciliations.get(text(row.at("id")))
        }
    };
    if group
        .iter()
        .any(|r| reconciled(r).is_some_and(Value::is_null))
    {
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
            let mut observation = observation::Observation::new(*row, key)?;
            if let Some(proof) = reconciled(row).filter(|v| !v.is_null()) {
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
    // Observations by slot, in slot order and then in observation order.
    let mut by_slot: Vec<usize> = (0..observations.len()).collect();
    by_slot.sort_by(|a, b| observations[*a].slot().cmp(&observations[*b].slot()));
    let slots: Vec<&[usize]> = by_slot
        .chunk_by(|a, b| observations[*a].slot() == observations[*b].slot())
        .collect();
    let mut contradictions = Vec::new();
    for items in &slots {
        let semantics: Vec<_> = items
            .iter()
            .map(|index| observations[*index].semantics())
            .collect();
        if semantics.iter().any(|s| *s != semantics[0]) {
            let (wallet, direction, asset) = observations[items[0]].slot();
            let semantics: BTreeSet<(i128, i128, bool)> = semantics.into_iter().collect();
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
    // Each slot's selected observation becomes its leg, in slot order.
    let mut selected = vec![0; observations.len()];
    let mut leg_of = vec![usize::MAX; observations.len()];
    for (at, items) in slots.iter().copied().enumerate() {
        let keeper = *items
            .iter()
            .min_by_key(|index| observations[**index].selection())
            .unwrap();
        for index in items {
            selected[*index] = keeper;
        }
        leg_of[keeper] = at;
    }
    let leg_count = slots.len();
    drop(slots);
    let mut aliases = Vec::with_capacity(observations.len());
    let mut ids = Vec::with_capacity(observations.len());
    for (index, item) in observations.iter().enumerate() {
        aliases.push((
            item.transaction_id.as_ref(),
            observations[selected[index]]
                .commitment
                .quantity_hash
                .as_str(),
        ));
        ids.push(item.transaction_id.as_ref());
    }
    aliases.sort();
    ids.sort();
    ids.dedup();
    let string = |value: &str| Value::String(value.to_owned());
    let aliases = aliases
        .into_iter()
        .map(|(id, hash)| Value::Array(vec![string(id), string(hash)]))
        .collect();
    let ids = ids.into_iter().map(string).collect();
    // In detail hash order, a later commitment with the same detail hash
    // replaces an earlier one.
    let detail = |index: &usize| observations[*index].commitment.detail_hash.as_str();
    let mut by_detail: Vec<usize> = (0..observations.len()).collect();
    by_detail.sort_by(|a, b| detail(a).cmp(detail(b)));
    let mut snapshot_of = vec![usize::MAX; observations.len()];
    for (at, same) in by_detail
        .chunk_by(|a, b| detail(a) == detail(b))
        .enumerate()
    {
        snapshot_of[*same.last().expect("a nonempty run")] = at;
    }
    // Selected observations move into their legs, which then hold their
    // snapshots; the other snapshots keep their commitments.
    let mut legs: Vec<Option<canonical::Leg>> = vec![None; leg_count];
    let mut snapshots: Vec<Option<canonical::Snapshot>> =
        vec![None; snapshot_of.iter().filter(|at| **at != usize::MAX).count()];
    for (index, item) in observations.into_iter().enumerate() {
        let leg = leg_of[index];
        let snapshot = snapshot_of[index];
        if leg != usize::MAX {
            if snapshot != usize::MAX {
                snapshots[snapshot] = Some(canonical::Snapshot::Leg(leg));
            }
            legs[leg] = Some(item.into_leg(&keys));
        } else if snapshot != usize::MAX {
            let commitment = item.commitment;
            snapshots[snapshot] = Some(canonical::Snapshot::Owned {
                quantity_hash: commitment.quantity_hash,
                detail_hash: commitment.detail_hash,
                payload_json: commitment.payload_json,
            });
        }
    }
    let legs = legs.into_iter().map(|leg| leg.expect("slot leg")).collect();
    let snapshots = snapshots
        .into_iter()
        .map(|snapshot| snapshot.expect("winning snapshot"))
        .collect();
    Ok(Ok(canonical::Event::new(
        keys, legs, snapshots, ids, aliases,
    )))
}

/// The evidence stage: enriched rows by source-row index, the canonical input
/// and the receipt and zero-delta facts. Its JSON is built only on request.
#[derive(Clone, Debug)]
pub struct Evidence {
    enriched: Vec<Enriched>,
    canonical: std::sync::Arc<canonical::Canonical>,
    reconciliations: Value,
    non_events: BTreeSet<String>,
}
impl Evidence {
    /// The canonical input's JSON object, built on first request.
    pub fn canonical(&self) -> &Value {
        self.canonical.value()
    }
    pub fn canonical_input(&self) -> &canonical::Canonical {
        &self.canonical
    }
    pub(crate) fn canonical_arc(&self) -> std::sync::Arc<canonical::Canonical> {
        self.canonical.clone()
    }
    pub fn non_event_ids(&self) -> &BTreeSet<String> {
        &self.non_events
    }
    pub fn enriched(&self) -> &[Enriched] {
        &self.enriched
    }
    /// The stage object, given the snapshot rows the enriched rows read.
    pub fn value(&self, rows: &[Record]) -> Value {
        let mut stage = serde_json::Map::new();
        stage.insert(
            "enriched_rows".into(),
            Value::Array(self.enriched.iter().map(|e| e.view(rows).value()).collect()),
        );
        stage.insert("canonical_input".into(), self.canonical.value().clone());
        stage.insert(
            "receipt_reconciliations".into(),
            self.reconciliations.clone(),
        );
        stage.insert(
            "native_non_event_ids".into(),
            Value::Array(self.non_events.iter().cloned().map(Value::String).collect()),
        );
        Value::Object(stage)
    }
}

/// Enriched snapshot rows and the outbound rows' event keys, computed once
/// for evidence and for the components' native observations.
pub struct Prepared {
    enriched: Vec<Enriched>,
    keys: Vec<Option<EventKey>>,
    /// Keys a reader computed for the other rows; only successes are kept.
    computed: Vec<std::sync::OnceLock<EventKey>>,
}
impl Prepared {
    pub(crate) fn row<'a>(&'a self, at: usize, rows: &'a [Record]) -> EnrichedRow<'a, Record> {
        self.enriched[at].view(rows)
    }
    /// The row's event key, as `event_key_of` of its enriched row.
    pub(crate) fn key(&self, at: usize, rows: &[Record]) -> Result<EventKey, Error> {
        if let Some(key) = self.keys[at].as_ref().or_else(|| self.computed[at].get()) {
            return Ok(key.clone());
        }
        let key = event_key_of(&self.row(at, rows))?;
        Ok(self.computed[at].get_or_init(|| key).clone())
    }
}

/// Enrich the snapshot rows; `stage_prepared` reports the outcome in order.
pub fn prepare(snapshot: &Snapshot) -> Result<Prepared, Error> {
    let _span = crate::timing::span("evidence.enriched");
    let (enriched, keys) = enrich_with_keys(&snapshot.transactions)?;
    let computed = (0..keys.len())
        .map(|_| std::sync::OnceLock::new())
        .collect();
    Ok(Prepared {
        enriched,
        keys,
        computed,
    })
}

pub fn stage(snapshot: &Snapshot) -> Result<Evidence, Error> {
    stage_prepared(snapshot, prepare(snapshot))
}

/// The evidence stage over an earlier enrichment, whose refusal still
/// follows the zero-delta refusal.
pub fn stage_prepared(
    snapshot: &Snapshot,
    prepared: Result<Prepared, Error>,
) -> Result<Evidence, Error> {
    // The snapshot records hold the rows' fields.
    let records = &snapshot.transactions;
    let span = crate::timing::span("evidence.zero_delta");
    let non_events = crate::graph::native_zero_delta_ids(records, snapshot)?;
    drop(span);
    let Prepared {
        enriched,
        keys,
        computed,
    } = prepared?;
    // Keys the components stage computed are the keys this stage would.
    let keys = keys
        .into_iter()
        .zip(computed)
        .map(|(key, computed)| key.or_else(|| computed.into_inner()))
        .collect();
    let span = crate::timing::span("evidence.receipts");
    let reconciliations = crate::graph::reconcile_receipts(records, snapshot)?;
    drop(span);
    let span = crate::timing::span("evidence.canonical");
    let views: Vec<_> = enriched.iter().map(|e| e.view(records)).collect();
    let canonical = canonical_with_keys(&views, &reconciliations, &non_events, keys)?;
    drop(span);
    Ok(Evidence {
        enriched,
        canonical: std::sync::Arc::new(canonical),
        reconciliations,
        non_events,
    })
}

pub fn compute(snapshot: &Snapshot) -> Result<Value, Error> {
    stage(snapshot).map(|evidence| evidence.value(&snapshot.transactions))
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    #[test]
    fn canonical_events_follow_key_order_with_unique_sources() {
        let row = |id: &str, event: &str| json!({"id":id,"profile_id":"p","wallet_id":"w","wallet_kind":"descriptor","direction":"inbound","asset":"BTC","amount":1000,"fee":0,"occurred_at":"2024-01-01T00:00:00+00:00","source_ref":"src","external_id":event,"raw_json":"{}","network":"regtest"});
        // Rows out of key order, and one transaction observed twice.
        let rows = [row("t2", "e2"), row("t1", "e1"), row("t1", "e1")];
        let canonical = canonical_input(&rows, &json!({}), &BTreeSet::new()).unwrap();
        let events = canonical["events"].as_array().unwrap();
        let ids: Vec<_> = events
            .iter()
            .map(|event| event["event_key"]["native_event_id"].clone())
            .collect();
        assert_eq!(ids, [json!("e1"), json!("e2")]);
        assert_eq!(events[0]["source_transaction_ids"], json!(["t1"]));
    }
    #[test]
    fn txid_names_match_python_normalization() {
        let names = [
            "txid",
            "TxID",
            "tx_hash",
            "Transaction-Hash",
            "onChainTxId",
            "transaction id",
            "txid1",
            "abcdefghijklmnopqrstuvwxyz",
            "transactionhashx",
            "TX\u{130}D",
            "t\u{e9}xid",
            "tx\u{0}id",
            "",
        ];
        for name in names {
            let reference: String = name.py_lower().chars().filter(|c| isalnum(*c)).collect();
            let expected = [
                "txid",
                "txhash",
                "transactionid",
                "transactionhash",
                "onchaintxid",
            ]
            .contains(&reference.as_str());
            assert_eq!(txid_name(name), expected, "{name:?}");
        }
    }
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
