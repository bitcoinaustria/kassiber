//! Private Bitcoin observer state and record-reuse rules, following Python's
//! `chain_observer/bdk{,_persistence}.py` and `sync_backends.py`.
//!
//! This is an inert preparation boundary, not an observer implementation. It
//! neither selects a canonical chain nor grants absence/retraction authority.
//! In particular, it cannot replace BDK with the compatibility gap walker.
use bitcoin::{consensus, hex::FromHex, Transaction};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{dumps, Options, Value as PyValue};
use kassiber_store::rusqlite::Connection;
use miniscript::{Descriptor, DescriptorPublicKey, ForEachKey};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, AppError>;

/// Hash structural source keys only. Never pass descriptors/xpubs as a source
/// key: Python uses `descriptor:default`, `xpub:<family>` or a Samourai role.
pub fn instance_id(logical_wallet: &str, source_wallet: &str, source_key: &str) -> String {
    hash(
        format!("chain-observer-identity-v1\0{logical_wallet}\0{source_wallet}\0{source_key}")
            .as_bytes(),
    )
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn invalid(field: &str) -> AppError {
    AppError::new(
        "observer_state_rebuild_required",
        "Stored BDK observer state cannot be decoded",
    )
    .with_hint("Run a full wallet refresh to rebuild derived BDK observer state.")
    .with_details(dict([
        ("representation", s("bdk_changeset")),
        ("field", s(field)),
    ]))
}

fn object<'a>(value: &'a Value, keys: &[&str], field: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .filter(|v| v.len() == keys.len() && keys.iter().all(|k| v.contains_key(*k)))
        .ok_or_else(|| invalid(field))
}

fn items<'a>(value: &'a Value, keys: &[&str], field: &str) -> Result<Vec<&'a Value>> {
    value
        .as_array()
        .ok_or_else(|| invalid(field))?
        .iter()
        .map(|item| object(item, keys, field).map(|_| item))
        .collect()
}

fn unsigned(value: &Value, field: &str, max: u64) -> Result<u64> {
    crate::scan::integer(value)
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v <= max)
        .ok_or_else(|| invalid(field))
}

fn hex(value: &Value, field: &str, length: Option<usize>) -> Result<String> {
    // Python's bytes.fromhex permits ASCII whitespace between complete bytes,
    // including vertical tab, but never between a byte's two hex digits.
    let mut input = value.as_str().ok_or_else(|| invalid(field))?.bytes();
    let mut bytes = vec![];
    while let Some(first) = input.next() {
        if matches!(first, b'\t'..=b'\r' | b' ') {
            continue;
        }
        let high = char::from(first)
            .to_digit(16)
            .ok_or_else(|| invalid(field))?;
        let low = input
            .next()
            .and_then(|v| char::from(v).to_digit(16))
            .ok_or_else(|| invalid(field))?;
        bytes.push((high * 16 + low) as u8);
    }
    if length.is_some_and(|n| n != bytes.len()) {
        return Err(invalid(field));
    }
    Ok(bytes.iter().map(|v| format!("{v:02x}")).collect())
}

fn descriptor(value: &Value, field: &str, mainnet: bool) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let parsed: Descriptor<DescriptorPublicKey> = value
        .as_str()
        .ok_or_else(|| invalid(field))?
        .parse()
        .map_err(|_| invalid(field))?;
    if !parsed.for_each_key(|key| match key {
        DescriptorPublicKey::Single(_) => true,
        DescriptorPublicKey::XPub(key) => key.xkey.network.is_mainnet() == mainnet,
        DescriptorPublicKey::MultiXPub(key) => key.xkey.network.is_mainnet() == mainnet,
    }) {
        return Err(invalid(field));
    }
    Ok(json!(parsed.to_string()))
}

fn version_one(value: &Value) -> bool {
    // Python compares numeric equality here: 1, 1.0 and True are equal.
    value == true || value.as_f64() == Some(1.0)
}

/// Decode and canonically re-encode Python's complete version-one BDK aggregate.
/// Hash fields retain BDK's serialized byte order (not display-order txids).
/// This does not reconstruct a dependency wallet or interpret its graph.
pub fn normalize_changeset(payload: &Value) -> Result<Value> {
    object(
        payload,
        &[
            "schema_version",
            "descriptor",
            "change_descriptor",
            "network",
            "local_chain",
            "tx_graph",
            "indexer",
            "locked_outpoints",
        ],
        "$",
    )?;
    if !version_one(&payload["schema_version"]) {
        return Err(invalid("schema_version"));
    }
    let network = &payload["network"];
    if !network.is_null()
        && !matches!(
            network.as_str(),
            Some("BITCOIN" | "TESTNET" | "TESTNET4" | "SIGNET" | "REGTEST")
        )
    {
        return Err(invalid("network"));
    }
    let mut chain = vec![];
    for item in items(&payload["local_chain"], &["height", "hash"], "local_chain")? {
        let height = unsigned(&item["height"], "local_chain.height", u32::MAX.into())?;
        let block_hash = if item["hash"].is_null() {
            Value::Null
        } else {
            json!(hex(&item["hash"], "local_chain.hash", Some(32))?)
        };
        chain.push((height, json!({"height":height,"hash":block_hash})));
    }
    chain.sort_by_key(|(height, _)| *height);
    let graph = &payload["tx_graph"];
    object(
        graph,
        &[
            "txs",
            "txouts",
            "anchors",
            "last_seen",
            "first_seen",
            "last_evicted",
        ],
        "tx_graph",
    )?;
    let mut txs = vec![];
    for raw in graph["txs"]
        .as_array()
        .ok_or_else(|| invalid("tx_graph.txs"))?
    {
        let raw = hex(raw, "tx_graph.txs", None)?;
        let tx: Transaction = consensus::deserialize(
            &Vec::<u8>::from_hex(&raw).map_err(|_| invalid("tx_graph.txs"))?,
        )
        .map_err(|_| invalid("tx_graph.txs"))?;
        txs.push(consensus::encode::serialize_hex(&tx));
    }
    txs.sort();
    let mut txouts = BTreeMap::new();
    for item in items(
        &graph["txouts"],
        &["txid", "vout", "value_sats", "script_pubkey"],
        "tx_graph.txouts",
    )? {
        let txid = hex(&item["txid"], "tx_graph.txouts.txid", Some(32))?;
        let vout = unsigned(&item["vout"], "tx_graph.txouts.vout", u32::MAX.into())?;
        let value = unsigned(&item["value_sats"], "tx_graph.txouts.value_sats", u64::MAX)?;
        let script = hex(
            &item["script_pubkey"],
            "tx_graph.txouts.script_pubkey",
            None,
        )?;
        txouts.insert(
            (txid.clone(), vout),
            json!({"txid":txid,"vout":vout,"value_sats":value,"script_pubkey":script}),
        );
    }
    let mut anchors = vec![];
    for item in items(
        &graph["anchors"],
        &["txid", "block_height", "block_hash", "confirmation_time"],
        "tx_graph.anchors",
    )? {
        let txid = hex(&item["txid"], "tx_graph.anchors.txid", Some(32))?;
        let height = unsigned(
            &item["block_height"],
            "tx_graph.anchors.block_height",
            u32::MAX.into(),
        )?;
        let block_hash = hex(&item["block_hash"], "tx_graph.anchors.block_hash", Some(32))?;
        let time = unsigned(
            &item["confirmation_time"],
            "tx_graph.anchors.confirmation_time",
            u64::MAX,
        )?;
        anchors.push(((txid.clone(), height, block_hash.clone()), json!({"txid":txid,"block_height":height,"block_hash":block_hash,"confirmation_time":time})));
    }
    // Python sorts this list stably by txid/height/hash, excluding time.
    // These three components are lists in the binding, not maps or sets.
    anchors.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut normalized_graph = json!({"txs":txs,"txouts":txouts.into_values().collect::<Vec<_>>(),"anchors":anchors.into_iter().map(|(_, v)|v).collect::<Vec<_>>()});
    for name in ["last_seen", "first_seen", "last_evicted"] {
        let mut times = BTreeMap::new();
        for item in items(&graph[name], &["txid", "timestamp"], name)? {
            let txid = hex(&item["txid"], name, Some(32))?;
            let timestamp = unsigned(&item["timestamp"], name, u64::MAX)?;
            times.insert(txid.clone(), json!({"txid":txid,"timestamp":timestamp}));
        }
        normalized_graph[name] = json!(times.into_values().collect::<Vec<_>>());
    }
    let mut indexer = BTreeMap::new();
    for item in items(
        &payload["indexer"],
        &["descriptor_id", "last_revealed"],
        "indexer",
    )? {
        let id = hex(&item["descriptor_id"], "indexer.descriptor_id", Some(32))?;
        let index = unsigned(
            &item["last_revealed"],
            "indexer.last_revealed",
            u32::MAX.into(),
        )?;
        indexer.insert(
            id.clone(),
            json!({"descriptor_id":id,"last_revealed":index}),
        );
    }
    let mut locked = BTreeMap::new();
    for item in items(
        &payload["locked_outpoints"],
        &["txid", "vout", "locked"],
        "locked_outpoints",
    )? {
        let txid = hex(&item["txid"], "locked_outpoints.txid", Some(32))?;
        let vout = unsigned(&item["vout"], "locked_outpoints.vout", u32::MAX.into())?;
        let is_locked = item["locked"]
            .as_bool()
            .ok_or_else(|| invalid("locked_outpoints.locked"))?;
        locked.insert(
            (txid.clone(), vout),
            json!({"txid":txid,"vout":vout,"locked":is_locked}),
        );
    }
    Ok(json!({
        "schema_version":1,"descriptor":descriptor(&payload["descriptor"], "descriptor", network == "BITCOIN")?,
        "change_descriptor":descriptor(&payload["change_descriptor"], "change_descriptor", network == "BITCOIN")?,"network":network,
        "local_chain":chain.into_iter().map(|(_, v)|v).collect::<Vec<_>>(),"tx_graph":normalized_graph,
        "indexer":indexer.into_values().collect::<Vec<_>>(),"locked_outpoints":locked.into_values().collect::<Vec<_>>()
    }))
}

fn compact(value: &PyValue, ascii: bool) -> Result<String> {
    dumps(
        value,
        &Options::new()
            .sort_keys(true)
            .ensure_ascii(ascii)
            .separators(",", ":")
            .default_str(true),
    )
    .map_err(|_| {
        AppError::new(
            "observer_state_invalid",
            "Observer commitment cannot be encoded",
        )
    })
}

fn py(value: &Value) -> Result<PyValue> {
    kassiber_pytext::loads(&value.to_string())
        .map_err(|_| AppError::new("observer_state_invalid", "Invalid observer JSON"))
}

/// Python's fingerprint omits depth, which grows without changing a saved row.
/// The caller supplies this release's version (`1:<Kassiber version>`).
pub fn record_fingerprint(version: &str, record: &Value) -> Result<String> {
    let mut position = record["position"]
        .as_object()
        .cloned()
        .ok_or_else(|| invalid("position"))?;
    position.remove("confirmations");
    let payload = json!({"version":version,"txid":record["txid"],"vin":record["vin"],
        "vout":record["vout"],"position":position,"observed_at":record["observed_at"],
        "owned_scripts":record["owned_scripts"],"backend":record["backend"]});
    Ok(hash(compact(&py(&payload)?, true)?.as_bytes())[..32].into())
}

/// Reuse is request-local and must be disabled on a full scan (including a
/// reorg rebuild). The canonical observer, not a history response, chooses it.
pub struct RecordReuse {
    prior_fingerprints: Map<String, Value>,
    current_txids: BTreeSet<String>,
}

impl RecordReuse {
    pub fn new(
        prior_state: Option<&Value>,
        use_full_scan: bool,
        version: &str,
        current_txids: Option<BTreeSet<String>>,
    ) -> Self {
        let prior_fingerprints = prior_state
            .filter(|_| !use_full_scan && current_txids.is_some())
            .filter(|v| v["record_fingerprint_version"].as_str() == Some(version))
            .and_then(|v| v["record_fingerprints"].as_object())
            .cloned()
            .unwrap_or_default();
        Self {
            prior_fingerprints,
            current_txids: current_txids.unwrap_or_default(),
        }
    }

    pub fn can_reuse(&self, txid: &str, confirmed: bool, fingerprint: &str) -> bool {
        confirmed
            && self.current_txids.contains(txid)
            && self.prior_fingerprints.get(txid).and_then(Value::as_str) == Some(fingerprint)
    }
}

pub fn graph_hash(raw_json: &str) -> Result<String> {
    let parsed = kassiber_pytext::loads(if raw_json.is_empty() { "{}" } else { raw_json })
        .unwrap_or_else(|_| dict([("opaque_text", s(raw_json))]));
    Ok(hash(compact(&parsed, false)?.as_bytes()))
}

/// `canonical_graph_hash` for an already decoded (non-text) stored value.
pub fn graph_hash_value(value: &PyValue) -> Result<String> {
    Ok(hash(compact(value, false)?.as_bytes()))
}

fn field<'a>(row: &'a PyValue, name: &str) -> &'a PyValue {
    get(row, name).unwrap_or(&PyValue::Null)
}

fn text(row: &PyValue, name: &str) -> String {
    let value = field(row, name);
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

/// The quantity commitment uses integer msat, never a floating BTC quantity.
pub fn quantity_hash(row: &PyValue) -> Result<String> {
    let integer = |name| -> Result<PyValue> {
        let value = field(row, name);
        if !truthy(value) {
            return Ok(PyValue::Int(0.into()));
        }
        let normalized = kassiber_pytext::parse_int(&py_str(value)).map_err(|_| {
            AppError::new(
                "observer_state_invalid",
                "Invalid observed integer quantity",
            )
        })?;
        Ok(PyValue::Int(normalized))
    };
    let asset_text = text(row, "asset");
    let asset = kassiber_pytext::strip(&asset_text);
    let lowered = asset.to_ascii_lowercase();
    let asset = if lowered.len() == 64 && lowered.bytes().all(|b| b.is_ascii_hexdigit()) {
        lowered
    } else if matches!(lowered.as_str(), "l-btc" | "lbtc") {
        "LBTC".into()
    } else {
        asset.to_uppercase()
    };
    let payload = dict([
        ("schema_version", PyValue::Int(1.into())),
        ("wallet_id", s(text(row, "wallet_id"))),
        (
            "external_id",
            s(kassiber_pytext::lower(kassiber_pytext::strip(&text(
                row,
                "external_id",
            )))),
        ),
        (
            "direction",
            s(kassiber_pytext::lower(kassiber_pytext::strip(&text(
                row,
                "direction",
            )))),
        ),
        ("asset", s(asset)),
        ("amount_msat", integer("amount")?),
        ("fee_msat", integer("fee")?),
        (
            "amount_includes_fee",
            PyValue::Bool(truthy(field(row, "amount_includes_fee"))),
        ),
    ]);
    Ok(hash(compact(&payload, true)?.as_bytes()))
}

/// Every active projection of a txid must still have this observer's sole,
/// current graph AND quantity authority. One stale sibling poisons reuse.
pub fn current_observed_txids(
    conn: &Connection,
    wallet_id: &str,
    observer_id: &str,
) -> Result<BTreeSet<String>> {
    let mut current = BTreeSet::new();
    let mut stale = BTreeSet::new();
    for row in
        kassiber_store::observer::observed_rows(conn, wallet_id).map_err(crate::database_error)?
    {
        let external = row.external_id;
        let txid = kassiber_pytext::strip(external.as_deref().unwrap_or("")).to_ascii_lowercase();
        if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let observed = dict([
            ("external_id", s(external.unwrap_or_default())),
            ("wallet_id", s(row.wallet_id)),
            ("direction", s(row.direction)),
            ("asset", s(row.asset)),
            ("amount", PyValue::Int(row.amount.into())),
            ("fee", PyValue::Int(row.fee.into())),
            (
                "amount_includes_fee",
                PyValue::Bool(row.amount_includes_fee),
            ),
        ]);
        let raw = row.raw_json;
        let version = row.authority_version;
        let graph = row.graph_hash;
        let quantity = row.quantity_hash;
        let ids = row.observer_ids_json;
        let ids = ids.and_then(|v| serde_json::from_str::<Value>(&v).ok());
        if version == Some(1)
            && ids == Some(json!([observer_id]))
            && graph.as_deref()
                == Some(
                    match raw.as_deref() {
                        Some(raw) => graph_hash(raw)?,
                        None => hash(b"null"),
                    }
                    .as_str(),
                )
            && quantity.as_deref() == Some(quantity_hash(&observed)?.as_str())
        {
            current.insert(txid);
        } else {
            stale.insert(txid);
        }
    }
    Ok(current.difference(&stale).cloned().collect())
}

pub struct Identity<'a> {
    pub id: &'a str,
    pub observer_kind: &'a str,
    pub chain: &'a str,
    pub network: &'a str,
    pub logical_wallet_id: &'a str,
    pub source_wallet_id: &'a str,
    pub source_key: &'a str,
}

pub struct LoadedState {
    pub prior: Option<Value>,
    pub coverage: Vec<Value>,
    pub expected_epoch: Option<i64>,
}

impl LoadedState {
    /// A forced refresh discards the dependency aggregate in memory only. Keep
    /// `prior` intact for canonical/retraction comparison by the coordinator.
    pub fn bdk_changeset(&self, force_full: bool) -> Result<Option<Value>> {
        if force_full {
            return Ok(None);
        }
        let Some(prior) = &self.prior else {
            return Ok(None);
        };
        if !version_one(&prior["schema_version"]) {
            return Err(AppError::new(
                "observer_state_rebuild_required",
                "Stored BDK observer state must be rebuilt",
            )
            .with_hint("Run a full wallet refresh to rebuild derived BDK observer state."));
        }
        if !prior["bdk_changeset"].is_object() {
            return Err(AppError::new(
                "observer_state_rebuild_required",
                "Stored BDK observer state is incomplete",
            ));
        }
        normalize_changeset(&prior["bdk_changeset"]).map(Some)
    }
}

/// `OBSERVER_VALUES_NAMESPACE_VERSION`.
pub const VALUES_NAMESPACE_VERSION: i64 = 1;

/// `load_observer_values`: opaque dependency bytes for one identity. A foreign
/// namespace version requires a rebuild; the bytes are never decoded here.
pub fn load_values(
    conn: &Connection,
    observer_id: &str,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let mut values = std::collections::BTreeMap::new();
    for row in kassiber_store::observer::values(conn, observer_id).map_err(crate::database_error)? {
        if row.namespace_version != VALUES_NAMESPACE_VERSION {
            return Err(AppError::new(
                "observer_state_rebuild_required",
                "Stored chain observer state must be rebuilt before this wallet can refresh",
            )
            .with_hint("Run a full wallet refresh to rebuild derived observer state.")
            .with_details(dict([
                ("observer_id", s(observer_id)),
                ("representation", s("opaque_values")),
                ("stored_version", PyValue::Int(row.namespace_version.into())),
                (
                    "supported_version",
                    PyValue::Int(VALUES_NAMESPACE_VERSION.into()),
                ),
            ])));
        }
        values.insert(row.key, row.value);
    }
    Ok(values)
}

/// Accept a pre-observer checkpoint during migration, but never borrow another
/// instance's facts from an aggregate checkpoint (including a missing own id).
pub fn identity_checkpoint(checkpoint: &Value, observer_id: &str) -> Value {
    if let Some(instances) = checkpoint["observer_instances"].as_object() {
        instances
            .get(observer_id)
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}))
    } else {
        checkpoint.clone()
    }
}

/// Read the same version/identity-bound JSON snapshot as Python's prepare
/// contract. A forced rebuild retains the raw epoch even for incompatible
/// rows. The caller still owns the future atomic compare-and-swap apply.
pub fn load_state(
    conn: &Connection,
    identity: &Identity<'_>,
    force_full: bool,
) -> Result<LoadedState> {
    if !conn.is_autocommit() {
        return Err(AppError::new(
            "observer_prepare_in_transaction",
            "Observer network preparation must finish before the wallet transaction begins",
        ));
    }
    let snapshot =
        kassiber_store::observer::snapshot(conn, identity.id).map_err(crate::database_error)?;
    let Some((row, points)) = snapshot else {
        return Ok(LoadedState {
            prior: None,
            coverage: vec![],
            expected_epoch: None,
        });
    };
    let rebuild = |representation, stored_version: Value| {
        AppError::new(
            "observer_state_rebuild_required",
            "Stored chain observer state must be rebuilt before this wallet can refresh",
        )
        .with_hint("Run a full wallet refresh to rebuild derived observer state.")
        .with_details(dict([
            ("observer_id", s(identity.id)),
            ("representation", s(representation)),
            ("stored_version", py(&stored_version).expect("primitive")),
            (
                "supported_version",
                PyValue::Int(if representation == "coverage" {
                    2.into()
                } else {
                    1.into()
                }),
            ),
        ]))
    };
    let decode = || -> Result<(Value, Vec<Value>)> {
        if row.state_version != 1 {
            return Err(rebuild("state", json!(row.state_version)));
        }
        if row.observer_kind != identity.observer_kind
            || row.chain != identity.chain
            || row.network != identity.network
            || row.logical_wallet_id != identity.logical_wallet_id
            || row.source_wallet_id != identity.source_wallet_id
            || row.source_key != identity.source_key
        {
            return Err(rebuild("state", json!("identity_mismatch")));
        }
        let decode_object = |raw: &str, repr| -> Result<Value> {
            let parsed: Value = serde_json::from_str(if raw.is_empty() { "{}" } else { raw })
                .map_err(|_| rebuild(repr, json!("invalid_json")))?;
            if !parsed.is_object() {
                return Err(rebuild(repr, json!("invalid_shape")));
            }
            Ok(parsed)
        };
        let payload = decode_object(&row.state_json, "state")?;
        let mut coverage = vec![];
        for point in points {
            if point.coverage_version != 2 {
                return Err(rebuild("coverage", json!(point.coverage_version)));
            }
            coverage.push(json!({"branch_key":point.branch_key,"highest_used":point.highest_used,"scanned_to":point.scanned_to,
                "details":decode_object(&point.details_json,"coverage")?}));
        }
        Ok((payload, coverage))
    };
    let (prior, coverage) = match decode() {
        Ok((payload, coverage)) => (Some(payload), coverage),
        Err(error) if force_full && error.code == "observer_state_rebuild_required" => {
            (None, vec![])
        }
        Err(error) => return Err(error),
    };
    Ok(LoadedState {
        prior,
        coverage,
        expected_epoch: Some(row.state_epoch),
    })
}

#[cfg(test)]
mod tests;
