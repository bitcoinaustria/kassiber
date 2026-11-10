//! Request-local Bitcoin protocol observations, before observer interpretation.
//!
//! Cache-aware preparation retains Python's protocol checkpoints. There is no
//! canonical-chain selection, ownership or database access. Cache-aware Esplora
//! prepares absence candidates only after a complete source-bound round.
#[cfg(test)]
use crate::scan::Cancellation;
use crate::scan::{script_hash, Control, Scope};
use bitcoin::{consensus::deserialize, hex::FromHex, Transaction};
use kassiber_chain::{Backend, ElectrumClient, HttpClient};
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

type Result<T> = std::result::Result<T, AppError>;

/// Discard fetch caches for a forced scan, retaining only versioned Esplora
/// positive memberships. Source binding and completeness are rechecked later.
pub fn full_scan_checkpoint(checkpoint: &Value) -> Value {
    crate::compatibility_cache::full_scan_checkpoint(checkpoint)
}

#[derive(Clone, Copy)]
enum Protocol {
    Esplora,
    Electrum,
}

/// All targets and the transport are immutable and bound to this action's grant.
/// Targets are observed scripts supplied by discovery, never ownership evidence.
pub struct Acquisition {
    scope: Scope,
    identity: Arc<()>,
    backend: Backend,
    protocol: Protocol,
    /// `bitcoin`, or `liquid` for a cached compatibility acquisition.
    chain: String,
    targets: Vec<Value>,
    max_pages: Option<usize>,
    version: String,
    cache: Option<CacheInput>,
}

struct CacheInput {
    checkpoint: Value,
    history: Value,
    config: Value,
    network: String,
}

pub struct AcquisitionConsent {
    identity: Arc<()>,
    transport: Consent,
    started: AtomicBool,
}
impl AcquisitionConsent {
    pub(crate) fn for_fetch(action: &Acquisition, transport: Consent) -> Self {
        Self {
            identity: action.identity.clone(),
            transport,
            started: AtomicBool::new(false),
        }
    }
    /// Only the explicit user action may create this request-local grant.
    pub fn explicit_trigger(action: &Acquisition) -> Self {
        Self {
            identity: action.identity.clone(),
            transport: Consent::explicit_trigger(action.backend.request()),
            started: AtomicBool::new(false),
        }
    }
    pub fn revoke(&self) {
        self.transport.revoke();
    }
}

fn invalid() -> AppError {
    AppError::new("protocol_error", "Invalid Bitcoin observation response")
}
fn field<'a>(config: &'a Value, name: &str) -> Option<&'a Value> {
    config
        .get(name)
        .filter(|v| !v.is_null() && v.as_str() != Some(""))
        .or_else(|| config.get("config").and_then(|v| v.get(name)))
}

impl Acquisition {
    /// No connection or descriptor interpretation. The coordinator must perform
    /// book-network and source-overlap preflight before this seam. Compatibility
    /// cache anchors are read only in `run`, after its exact-action consent fence.
    pub fn new(scope: Scope, backend: &Value, targets: &[Value], version: &str) -> Result<Self> {
        Self::build(scope, backend, targets, version, "bitcoin")
    }

    fn build(
        scope: Scope,
        backend: &Value,
        targets: &[Value],
        version: &str,
        chain: &str,
    ) -> Result<Self> {
        let mut config = backend.clone();
        if let Some(raw) = config.get("config_json").and_then(Value::as_str) {
            config["config"] = serde_json::from_str(raw)
                .map_err(|_| AppError::new("validation", "Invalid backend configuration JSON"))?;
        }
        let protocol = match field(&config, "kind").and_then(Value::as_str) {
            Some("esplora" | "mempool") => Protocol::Esplora,
            Some("electrum") => Protocol::Electrum,
            _ => {
                return Err(AppError::new(
                    "not_implemented",
                    "Bitcoin acquisition requires Esplora or Electrum",
                ))
            }
        };
        for target in targets {
            if target["chain"] != chain {
                return Err(AppError::new(
                    "not_implemented",
                    "Only Bitcoin observations are supported",
                ));
            }
            script_hash(target["script_pubkey"].as_str().ok_or_else(invalid)?)?;
        }
        let max_pages = field(&config, "maxpages")
            .filter(|v| !v.is_null())
            .map(|v| {
                let n = match v {
                    Value::String(s) => s.parse::<usize>().ok(),
                    _ => v.as_u64().and_then(|v| usize::try_from(v).ok()),
                };
                n.ok_or_else(|| AppError::new("validation", "Invalid history page limit"))
            })
            .transpose()?
            .filter(|n| *n > 0);
        Ok(Self {
            scope,
            identity: Arc::new(()),
            backend: Backend::new(&config, matches!(protocol, Protocol::Electrum))?,
            protocol,
            chain: chain.into(),
            targets: targets.to_vec(),
            max_pages,
            version: version.into(),
            cache: None,
        })
    }

    /// Bind private Python compatibility caches to the same immutable action
    /// as the targets and transport, before any grant can be minted.
    pub fn new_cached(
        scope: Scope,
        backend: &Value,
        targets: &[Value],
        version: &str,
        checkpoint: &Value,
        history: &Value,
        network: &str,
    ) -> Result<Self> {
        // Python's Liquid compatibility observers share these protocol paths.
        let chain = if matches!(network, "liquidv1" | "liquidtestnet" | "elementsregtest") {
            "liquid"
        } else {
            crate::bdk_state::network(network)?;
            "bitcoin"
        };
        if targets.iter().any(|t| t["network"] != network) {
            return Err(AppError::new(
                "validation",
                "Cached observation targets must use the book network",
            ));
        }
        let mut action = Self::build(scope, backend, targets, version, chain)?;
        let mut config = backend.clone();
        if let Some(raw) = backend.get("config_json").and_then(Value::as_str) {
            config["config"] = serde_json::from_str(raw).map_err(|_| invalid())?;
        }
        action.cache = Some(CacheInput {
            checkpoint: checkpoint.clone(),
            history: history.clone(),
            config,
            network: network.into(),
        });
        Ok(action)
    }

    /// Returns a complete request-local acquisition or an error, never partial
    /// results. `history` is protocol data, not proof of an absent transaction.
    pub fn run(
        &self,
        gate: &Gate,
        scope: &Scope,
        consent: Option<&AcquisitionConsent>,
        deadline: Instant,
        cancel: &dyn Control,
        progress: impl FnMut(Value),
    ) -> Result<Value> {
        self.run_with_history(
            gate,
            scope,
            consent,
            deadline,
            cancel,
            progress,
            &mut |_| Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_with_history(
        &self,
        gate: &Gate,
        scope: &Scope,
        consent: Option<&AcquisitionConsent>,
        deadline: Instant,
        cancel: &dyn Control,
        mut progress: impl FnMut(Value),
        history: &mut dyn FnMut(&Value) -> Result<()>,
    ) -> Result<Value> {
        cancel.check(deadline)?;
        if scope != &self.scope {
            return Err(AppError::new(
                "stale_context",
                "Wallet acquisition scope changed",
            ));
        }
        let proof = consent
            .filter(|c| Arc::ptr_eq(&c.identity, &self.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "Wallet acquisition requires consent for this exact action",
                )
            })?;
        gate.authorize(
            self.backend.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| e.app_error())?;
        if proof.started.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Wallet acquisition consent was already used",
            ));
        }
        let result = match (self.protocol, self.cache.as_ref()) {
            (Protocol::Esplora, Some(cache)) => {
                self.cached_esplora(gate, &proof.transport, deadline, cancel, cache, history)
            }
            (Protocol::Electrum, Some(cache)) => {
                self.cached_electrum(gate, &proof.transport, deadline, cancel, cache)
            }
            (Protocol::Esplora, None) => {
                self.esplora(gate, &proof.transport, deadline, cancel, &mut progress)
            }
            (Protocol::Electrum, None) => {
                self.electrum(gate, &proof.transport, deadline, cancel, &mut progress)
            }
        }?;
        cancel.check(deadline)?;
        gate.authorize(
            self.backend.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| e.app_error())?;
        Ok(result)
    }

    fn esplora(
        &self,
        gate: &Gate,
        consent: &Consent,
        deadline: Instant,
        cancel: &dyn Control,
        progress: &mut impl FnMut(Value),
    ) -> Result<Value> {
        let client = HttpClient::new(self.backend.clone(), gate.clone(), &self.version);
        let mut scripts = vec![];
        let mut transactions = Map::new();
        for target in &self.targets {
            cancel.check(deadline)?;
            let hash = script_hash(target["script_pubkey"].as_str().ok_or_else(invalid)?)?;
            let path = format!("scripthash/{hash}");
            // Keep the pre/post statistics for the future membership authority.
            let before = client.get_json(&path, Some(consent), deadline)?;
            let history =
                esplora_history(&client, &path, self.max_pages, consent, deadline, cancel)?;
            cancel.check(deadline)?;
            let after = client.get_json(&path, Some(consent), deadline)?;
            cancel.check(deadline)?;
            let utxos =
                array(client.get_json(&format!("{path}/utxo"), Some(consent), deadline)?)?;
            for tx in &history {
                transactions.insert(tx["txid"].as_str().ok_or_else(invalid)?.into(), tx.clone());
            }
            scripts.push(json!({"target":target,"before":before,"after":after,"history":history,"utxos":utxos}));
            progress(
                json!({"phase":"acquisition","targets_checked":scripts.len(),"target_count":self.targets.len()}),
            );
        }
        cancel.check(deadline)?;
        // Python treats an unavailable tip as unknown depth, never zero depth.
        let tip = client
            .get_text("blocks/tip/height", Some(consent), deadline)
            .ok()
            .and_then(|s| kassiber_pytext::parse_int(kassiber_pytext::strip(&s)).ok())
            .and_then(|n| i64::try_from(n).ok());
        let positions: Vec<_> = transactions.iter().map(|(txid, tx)| {
            let status = &tx["status"];
            json!({"txid":txid,"confirmed":status["confirmed"].as_bool().unwrap_or(false),
                "block_height":positive_height(&status["block_height"]),"block_time":status["block_time"],
                "confirmations":confirmations(positive_height(&status["block_height"]),tip)})
        }).collect();
        Ok(
            json!({"scripts":scripts,"transactions":transactions,"headers":{},"block_headers":{},"tip_height":tip,"positions":positions}),
        )
    }

    fn electrum(
        &self,
        gate: &Gate,
        consent: &Consent,
        deadline: Instant,
        cancel: &dyn Control,
        progress: &mut impl FnMut(Value),
    ) -> Result<Value> {
        let mut client =
            ElectrumClient::connect(self.backend.clone(), gate, Some(consent.clone()), deadline)?;
        let hashes = self
            .targets
            .iter()
            .map(|t| script_hash(t["script_pubkey"].as_str().ok_or_else(invalid)?))
            .collect::<Result<Vec<_>>>()?;
        let statuses = calls(
            &mut client,
            "blockchain.scripthash.subscribe",
            &hashes.iter().map(|s| json!([s])).collect::<Vec<_>>(),
            deadline,
            cancel,
        )?;
        let histories = calls(
            &mut client,
            "blockchain.scripthash.get_history",
            &hashes.iter().map(|s| json!([s])).collect::<Vec<_>>(),
            deadline,
            cancel,
        )?;
        let unspent = calls(
            &mut client,
            "blockchain.scripthash.listunspent",
            &hashes.iter().map(|s| json!([s])).collect::<Vec<_>>(),
            deadline,
            cancel,
        )?;
        let mut scripts = vec![];
        let mut txids = BTreeMap::new();
        let mut heights = BTreeSet::new();
        for (index, target) in self.targets.iter().enumerate() {
            let history = array(histories[index].clone())?;
            let utxos = array(unspent[index].clone())?;
            for item in history.iter().chain(utxos.iter()) {
                if let Some(h) = positive_height(&item["height"]) {
                    heights.insert(h);
                }
            }
            for item in &history {
                let txid = item["tx_hash"].as_str().ok_or_else(invalid)?;
                validate_txid(txid)?;
                txids.insert(txid.to_owned(), item.clone());
            }
            scripts.push(
                json!({"target":target,"status":statuses[index],"history":history,"utxos":utxos}),
            );
        }
        let ordered: Vec<_> = txids.keys().cloned().collect();
        let mut transactions = Map::new();
        fetch_transactions(&mut client, &ordered, &mut transactions, deadline, cancel)?;
        let mut previous = BTreeSet::new();
        for tx in transactions.values() {
            for input in tx["vin"].as_array().ok_or_else(invalid)? {
                if coinbase_input(input) {
                    continue;
                }
                let txid = input["txid"].as_str().ok_or_else(invalid)?;
                if !transactions.contains_key(txid) {
                    previous.insert(txid.to_owned());
                }
            }
        }
        fetch_transactions(
            &mut client,
            &previous.into_iter().collect::<Vec<_>>(),
            &mut transactions,
            deadline,
            cancel,
        )?;
        // Inline only the current transactions, matching Python's stored graph.
        for txid in &ordered {
            let mut tx = transactions[txid].clone();
            normalize_graph(&mut tx, &transactions)?;
            transactions.insert(txid.clone(), tx);
        }
        let heights: Vec<_> = heights.into_iter().collect();
        let headers = calls(
            &mut client,
            "blockchain.block.header",
            &heights.iter().map(|h| json!([h])).collect::<Vec<_>>(),
            deadline,
            cancel,
        )?;
        let mut timestamps = Map::new();
        let mut block_headers = Map::new();
        for (height, header) in heights.iter().zip(headers) {
            timestamps.insert(
                height.to_string(),
                json!(header_timestamp(header.as_str().ok_or_else(invalid)?)?),
            );
            block_headers.insert(height.to_string(), header);
        }
        cancel.check(deadline)?;
        let tip = client
            .call("blockchain.headers.subscribe", None, deadline)
            .ok()
            .and_then(|v| integer(&v["height"]));
        let positions: Vec<_> = txids
            .iter()
            .map(|(txid, history)| {
                let height = positive_height(&history["height"]);
                json!({"txid":txid,"confirmed":height.is_some(),"block_height":height,
                "block_time":height.and_then(|h|timestamps.get(&h.to_string()).cloned()),
                "confirmations":confirmations(height,tip)})
            })
            .collect();
        progress(
            json!({"phase":"acquisition","targets_checked":scripts.len(),"target_count":self.targets.len()}),
        );
        Ok(
            json!({"scripts":scripts,"transactions":transactions,"headers":timestamps,"block_headers":block_headers,"tip_height":tip,"positions":positions}),
        )
    }
}

fn array(value: Value) -> Result<Vec<Value>> {
    value.as_array().cloned().ok_or_else(invalid)
}
fn validate_txid(txid: &str) -> Result<()> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    Ok(())
}
fn integer(value: &Value) -> Option<i64> {
    crate::scan::integer(value).and_then(|n| i64::try_from(n).ok())
}
fn positive_height(value: &Value) -> Option<i64> {
    integer(value).filter(|h| *h > 0)
}
fn confirmations(height: Option<i64>, tip: Option<i64>) -> Option<i64> {
    let (height, tip) = (height?, tip?);
    if height <= 0 || tip < height {
        return None;
    }
    tip.checked_sub(height)?.checked_add(1)
}
fn esplora_history(
    client: &HttpClient,
    path: &str,
    max_pages: Option<usize>,
    consent: &Consent,
    deadline: Instant,
    cancel: &dyn Control,
) -> Result<Vec<Value>> {
    let mut transactions = vec![];
    let mut seen = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut cursor = None;
    let mut pages = 0;
    while max_pages.is_none_or(|limit| pages < limit) {
        cancel.check(deadline)?;
        let path = cursor.as_ref().map_or_else(
            || format!("{path}/txs/chain"),
            |c| format!("{path}/txs/chain/{c}"),
        );
        let page = array(client.get_json(&path, Some(consent), deadline)?)?;
        for tx in &page {
            retain_transaction(tx, &mut seen, &mut transactions)?;
        }
        if page.len() < 25 {
            break;
        }
        let last = page.last().ok_or_else(invalid)?["txid"]
            .as_str()
            .ok_or_else(invalid)?
            .to_owned();
        if !cursors.insert(last.clone()) {
            return Err(invalid());
        }
        cursor = Some(last);
        pages += 1;
    }
    cancel.check(deadline)?;
    for tx in array(client.get_json(&format!("{path}/txs/mempool"), Some(consent), deadline)?)? {
        retain_transaction(&tx, &mut seen, &mut transactions)?;
    }
    Ok(transactions)
}
fn retain_transaction(
    tx: &Value,
    seen: &mut BTreeSet<String>,
    result: &mut Vec<Value>,
) -> Result<()> {
    let txid = tx["txid"].as_str().ok_or_else(invalid)?;
    validate_txid(txid)?;
    if seen.insert(txid.to_owned()) {
        result.push(tx.clone());
    }
    Ok(())
}
fn calls(
    client: &mut ElectrumClient,
    method: &str,
    params: &[Value],
    deadline: Instant,
    cancel: &dyn Control,
) -> Result<Vec<Value>> {
    let mut result = vec![];
    // The underlying client's own batches obey its backend limit. Check
    // cancellation between those batches too, not only after a full wallet.
    for chunk in params.chunks(client.batch_size()) {
        cancel.check(deadline)?;
        let requests: Vec<_> = chunk.iter().map(|p| (method.into(), p.clone())).collect();
        result.extend(client.batch_call(&requests, deadline)?);
    }
    cancel.check(deadline)?;
    Ok(result)
}
fn fetch_transactions(
    client: &mut ElectrumClient,
    txids: &[String],
    result: &mut Map<String, Value>,
    deadline: Instant,
    cancel: &dyn Control,
) -> Result<()> {
    let raw = calls(
        client,
        "blockchain.transaction.get",
        &txids.iter().map(|s| json!([s])).collect::<Vec<_>>(),
        deadline,
        cancel,
    )?;
    for (txid, raw) in txids.iter().zip(raw) {
        let (tx, hex) = verify_transaction_id(&raw, txid)?;
        result.insert(txid.clone(), decode_transaction(&tx, hex)?);
    }
    Ok(())
}
/// `verify_electrum_transaction_id`: bind fetched bytes to the requested txid.
/// Only a complete, canonical encoding counts; anything else, including a
/// non-string reply, is an invalid observation.
fn verify_transaction_id<'a>(raw: &'a Value, txid: &str) -> Result<(Transaction, &'a str)> {
    let invalid_bytes = || {
        AppError::new(
            "invalid_observation",
            "Electrum returned invalid transaction bytes",
        )
    };
    let hex = raw.as_str().ok_or_else(invalid_bytes)?;
    let bytes = Vec::<u8>::from_hex(hex).map_err(|_| invalid_bytes())?;
    let tx: Transaction = deserialize(&bytes).map_err(|_| invalid_bytes())?;
    if bitcoin::consensus::serialize(&tx) != bytes {
        return Err(invalid_bytes());
    }
    if tx.compute_txid().to_string() != txid.to_lowercase() {
        return Err(AppError::new(
            "invalid_observation",
            "Electrum returned different transaction bytes",
        ));
    }
    Ok((tx, hex))
}
/// `verify_electrum_transaction_id(raw, txid, "liquid")`: a complete,
/// canonical Elements encoding of the requested transaction.
pub(crate) fn liquid_transaction(
    hex: &str,
    txid: &str,
) -> Result<elements_miniscript::elements::Transaction> {
    use elements_miniscript::elements::{encode, Transaction};
    let invalid_bytes = || {
        AppError::new(
            "invalid_observation",
            "Electrum returned invalid transaction bytes",
        )
    };
    let bytes = Vec::<u8>::from_hex(hex).map_err(|_| invalid_bytes())?;
    let tx: Transaction = encode::deserialize(&bytes).map_err(|_| invalid_bytes())?;
    if encode::serialize(&tx) != bytes {
        return Err(invalid_bytes());
    }
    if tx.txid().to_string() != txid.to_lowercase() {
        return Err(AppError::new(
            "invalid_observation",
            "Electrum returned different transaction bytes",
        ));
    }
    Ok(tx)
}
/// Liquid transactions keep their verified raw hex.
fn fetch_liquid(
    client: &mut ElectrumClient,
    txids: &[String],
    result: &mut Map<String, Value>,
    deadline: Instant,
    cancel: &dyn Control,
) -> Result<()> {
    let raw = calls(
        client,
        "blockchain.transaction.get",
        &txids.iter().map(|s| json!([s])).collect::<Vec<_>>(),
        deadline,
        cancel,
    )?;
    for (txid, raw) in txids.iter().zip(raw) {
        let hex = raw.as_str().ok_or_else(|| {
            AppError::new(
                "invalid_observation",
                "Electrum returned invalid transaction bytes",
            )
        })?;
        liquid_transaction(hex, txid)?;
        result.insert(txid.clone(), json!({"raw_hex": hex}));
    }
    Ok(())
}
/// The previous txids of a fetched Liquid transaction's inputs.
fn liquid_parents(tx: &Value) -> Result<Vec<String>> {
    let hex = tx["raw_hex"].as_str().ok_or_else(invalid)?;
    let bytes = Vec::<u8>::from_hex(hex).map_err(|_| invalid())?;
    let tx: elements_miniscript::elements::Transaction =
        elements_miniscript::elements::encode::deserialize(&bytes).map_err(|_| invalid())?;
    Ok(tx
        .input
        .iter()
        .map(|input| input.previous_output.txid.to_string())
        .collect())
}
fn decode_transaction(tx: &Transaction, hex: &str) -> Result<Value> {
    let witness = tx.input.iter().any(|i| !i.witness.is_empty());
    let vin: Vec<_> = tx.input.iter().map(|i| {
        let mut v = json!({"txid":i.previous_output.txid.to_string(),"vout":i.previous_output.vout,"sequence":i.sequence.0});
        if witness { v["witness"] = json!(i.witness.iter().map(|w|w.iter().map(|b|format!("{b:02x}")).collect::<String>()).collect::<Vec<_>>()); }
        v
    }).collect();
    let mut total = 0u64;
    let mut vout = vec![];
    for (n, o) in tx.output.iter().enumerate() {
        let sats = o.value.to_sat();
        total = total.checked_add(sats).ok_or_else(invalid)?;
        let btc: Value =
            serde_json::from_str(&format!("{}.{:08}", sats / 100_000_000, sats % 100_000_000))
                .map_err(|_| invalid())?;
        vout.push(json!({"n":n,"value_sats":sats,"value":btc,"script_hex":o.script_pubkey.to_hex_string()}));
    }
    Ok(
        json!({"version":tx.version.0 as u32,"locktime":tx.lock_time.to_consensus_u32(),"vin":vin,"vout":vout,
        "total_output_sats":total,"raw_hex":hex,"size":tx.total_size(),"vsize":tx.vsize(),"weight":tx.weight().to_wu()}),
    )
}
fn normalize_graph(tx: &mut Value, lookup: &Map<String, Value>) -> Result<()> {
    tx["_kassiber_electrum_graph"] = json!({"kind":"bitcoin_electrum","version":2});
    for output in tx["vout"].as_array_mut().ok_or_else(invalid)? {
        output["scriptpubkey"] = output["script_hex"].clone();
        // Cached decoder fields have passed through Python json_ready and can
        // be floats. Normalized graph values remain integer sats on reuse.
        output["value"] = graph_sats(&output["value_sats"])?;
    }
    for input in tx["vin"].as_array_mut().ok_or_else(invalid)? {
        if coinbase_input(input) {
            continue;
        }
        if input["prevout"].is_object() {
            continue;
        }
        let prev = lookup
            .get(input["txid"].as_str().ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        if let Some(output) = input["vout"]
            .as_u64()
            .and_then(|n| prev["vout"].get(n as usize))
        {
            input["prevout"] = json!({"scriptpubkey":output["script_hex"],"value":graph_sats(&output["value_sats"])?});
        }
    }
    Ok(())
}
fn graph_sats(value: &Value) -> Result<Value> {
    use kassiber_pydecimal::{Decimal, Rounding};
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let value = kassiber_pytext::strip(&text)
        .parse::<Decimal>()
        .map_err(|_| invalid())?;
    let rounded = value
        .to_fixed_places(0, Rounding::HalfEven)
        .map_err(|_| invalid())?;
    let integer = rounded
        .parse::<kassiber_pyjson::BigInt>()
        .map_err(|_| invalid())?;
    serde_json::from_str(&integer.to_string()).map_err(|_| invalid())
}
fn coinbase_input(input: &Value) -> bool {
    input["txid"] == "0".repeat(64) && input["vout"] == u32::MAX
}
/// `block_header_timestamp`: bytes 68..72, which Bitcoin's 80-byte header and
/// Elements' longer one share.
fn header_timestamp(hex: &str) -> Result<u32> {
    let header = Vec::<u8>::from_hex(hex).map_err(|_| invalid())?;
    if header.len() < 72 {
        return Err(invalid());
    }
    Ok(u32::from_le_bytes(
        header[68..72].try_into().map_err(|_| invalid())?,
    ))
}

mod cached;
#[cfg(test)]
mod tests;
