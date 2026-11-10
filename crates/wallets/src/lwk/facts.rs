//! `LwkObserver._records` and `._facts`: JSON-only accounting projection of
//! one scanned wollet. No I/O. Unblinded values and assets come only from LWK;
//! a wrongly blinded or unrelated output stays external evidence.
use super::Result;
use crate::bitcoin_records::{btc, hashlock, sorted_json};
use kassiber_envelope::AppError;
use lwk_wollet::{
    elements::{
        confidential::{AssetBlindingFactor, ValueBlindingFactor},
        hashes::{hash160, sha256, Hash},
        hex::ToHex,
        BlockHeader, Transaction, TxOut, TxOutSecrets,
    },
    Chain, WalletTx, WalletTxOut, Wollet,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";

/// Request-constant inputs of the projection.
pub(crate) struct Source<'a> {
    pub network: &'a str,
    pub policy_asset: &'a str,
    pub backend_name: &'a str,
    pub branch_keys: &'a [String],
}

pub(crate) fn dependency_error(error: lwk_wollet::Error) -> AppError {
    let _ = error;
    AppError::new("protocol_error", "LWK could not project the scanned wallet")
}

/// `wallet_descriptors.liquid_asset_code`: the policy asset reads as LBTC.
pub(crate) fn asset_code(asset_id: &str, policy_asset: &str) -> String {
    let normalized = asset_id.to_lowercase();
    if !policy_asset.is_empty() && normalized == policy_asset.to_lowercase() {
        "LBTC".into()
    } else {
        normalized
    }
}

/// LWK's `is_explicit`: both blinding factors are zero.
fn explicit(secrets: &TxOutSecrets) -> bool {
    secrets.asset_bf == AssetBlindingFactor::zero()
        && secrets.value_bf == ValueBlindingFactor::zero()
}

fn owned(output: &WalletTxOut) -> Map<String, Value> {
    let mut item = Map::new();
    item.insert(
        "scriptpubkey".into(),
        json!(output.script_pubkey.as_bytes().to_hex()),
    );
    item.insert("value_sats".into(), json!(output.unblinded.value));
    item.insert("asset_id".into(), json!(output.unblinded.asset.to_string()));
    item.insert("role".into(), json!("owned"));
    item
}

/// `_fee_sats_by_asset` over Elements' explicit fee outputs.
fn fees(outputs: &[TxOut]) -> Result<BTreeMap<String, i128>> {
    let mut fees = BTreeMap::<String, i128>::new();
    for output in outputs.iter().filter(|o| o.is_fee()) {
        let (Some(value), Some(asset)) = (output.value.explicit(), output.asset.explicit()) else {
            continue;
        };
        let total = fees.entry(asset.to_string()).or_default();
        *total = total.checked_add(i128::from(value)).ok_or_else(overflow)?;
    }
    Ok(fees)
}

fn overflow() -> AppError {
    AppError::new(
        "observer_accounting_ambiguous",
        "LWK reported an unrepresentable amount",
    )
}

/// `htlc_parser.extract_from_claim_witness`: SHA-256 of the revealed preimage.
fn claim_hash(items: &[Vec<u8>]) -> Option<String> {
    let (script, rest) = items.split_last().filter(|_| items.len() >= 2)?;
    let lock = hashlock(script)?;
    rest.iter()
        .find(|item| {
            item.len() == 32 && hash160::Hash::hash(item).as_byte_array().as_slice() == lock
        })
        .map(|preimage| sha256::Hash::hash(preimage).to_string())
}

/// `htlc_parser.extract_from_refund_witness`: the timeout branch, no preimage.
fn refund_spend(items: &[Vec<u8>]) -> bool {
    let Some((script, rest)) = items.split_last().filter(|_| items.len() >= 2) else {
        return false;
    };
    let Some(lock) = hashlock(script) else {
        return false;
    };
    !rest.iter().any(|item| {
        item.len() == 32 && hash160::Hash::hash(item).as_byte_array().as_slice() == lock
    })
}

/// `(payment_hash, funding_txid, funding_vout)` for exactly one claim input,
/// and the one unique refund funding outpoint, as in `sync_backends`.
/// `(payment_hash, funding_txid, funding_vout)` of a claim input.
type Claim = (String, String, u32);
/// `(funding_txid, funding_vout)` of a refund input.
type Refund = (String, u32);

fn htlc_evidence(tx: &Transaction) -> (Option<Claim>, Option<Refund>) {
    let claim = match tx.input.as_slice() {
        [input] => claim_hash(&input.witness.script_witness).map(|hash| {
            (
                hash,
                input.previous_output.txid.to_string(),
                input.previous_output.vout,
            )
        }),
        _ => None,
    };
    let mut refunds: Vec<Refund> = vec![];
    for input in &tx.input {
        if refund_spend(&input.witness.script_witness) {
            let point = (
                input.previous_output.txid.to_string(),
                input.previous_output.vout,
            );
            if !refunds.contains(&point) {
                refunds.push(point);
            }
        }
    }
    let refund = match refunds.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    };
    (claim, refund)
}

/// `htlc_evidence.htlc_spend_attestation`.
fn attestation(claim: &Option<Claim>, refund: &Option<Refund>, inputs: usize) -> Option<Value> {
    if inputs != 1 || claim.is_some() == refund.is_some() {
        return None;
    }
    let mut result = json!({"version": 1, "input_count": 1});
    if let Some((hash, txid, vout)) = claim {
        result["role"] = json!("claim");
        result["funding_txid"] = json!(txid);
        result["funding_vout"] = json!(vout);
        result["payment_hash"] = json!(hash);
    } else if let Some((txid, vout)) = refund {
        result["role"] = json!("refund");
        result["funding_txid"] = json!(txid);
        result["funding_vout"] = json!(vout);
    }
    Some(result)
}

/// `_allocated_liquid_fee`: wallet-attributed fee, and whether it remains
/// folded into the exact wallet delta of a mixed-input spend.
pub(crate) fn allocated_fee(
    delta: i128,
    transaction_fee: i128,
    owns_input: bool,
    mixed: bool,
) -> (i128, bool) {
    if !owns_input || delta >= 0 || transaction_fee <= 0 {
        return (0, false);
    }
    if mixed {
        return (0, true);
    }
    (transaction_fee, false)
}

pub(crate) fn records(source: &Source<'_>, wallet_tx: &WalletTx) -> Result<Vec<Value>> {
    let tx = &wallet_tx.tx;
    let txid = wallet_tx.txid.to_string();
    let fees = fees(&tx.output)?;
    let mut vin = vec![];
    for (index, input) in tx.input.iter().enumerate() {
        let mut item = Map::new();
        item.insert("txid".into(), json!(input.previous_output.txid.to_string()));
        item.insert("vout".into(), json!(input.previous_output.vout));
        if let Some(Some(owned_input)) = wallet_tx.inputs.get(index) {
            item.insert("prevout".into(), Value::Object(owned(owned_input)));
        }
        vin.push(Value::Object(item));
    }
    let mut vout = vec![];
    for (index, output) in tx.output.iter().enumerate() {
        let mut item = Map::new();
        item.insert("n".into(), json!(index));
        item.insert(
            "scriptpubkey".into(),
            json!(output.script_pubkey.as_bytes().to_hex()),
        );
        if let Some(Some(owned_output)) = wallet_tx.outputs.get(index) {
            item.extend(owned(owned_output));
        } else if output.is_fee() {
            item.insert(
                "value_sats".into(),
                json!(output.value.explicit().unwrap_or(0)),
            );
            item.insert(
                "asset_id".into(),
                json!(output.asset.explicit().map(|a| a.to_string())),
            );
            item.insert("role".into(), json!("fee"));
        } else if let (Some(value), Some(asset)) =
            (output.value.explicit(), output.asset.explicit())
        {
            item.insert("value_sats".into(), json!(value));
            item.insert("asset_id".into(), json!(asset.to_string()));
            item.insert("role".into(), json!("external"));
        }
        vout.push(Value::Object(item));
    }
    let (claim, refund) = htlc_evidence(tx);
    let htlc_spend = attestation(&claim, &refund, tx.input.len());
    let occurred_at = match wallet_tx.timestamp {
        Some(0) | None => UNKNOWN_OCCURRED_AT.to_owned(),
        Some(seconds) => kassiber_chain::format_unix_time(i64::from(seconds))
            .ok_or_else(|| AppError::new("protocol_error", "Invalid Liquid block time"))?,
    };
    let confirmed_at = wallet_tx.timestamp.map(|_| occurred_at.clone());
    let owned_inputs = wallet_tx.inputs.iter().filter(|i| i.is_some()).count();
    let owns_input = owned_inputs > 0;
    let mixed = owns_input && owned_inputs < tx.input.len();
    // `_wallet_tx_balance`: LWK's confidential balance plus the explicit
    // owned deltas it omits, sorted by the asset id's display text.
    let mut deltas: BTreeMap<String, i128> = wallet_tx
        .balance
        .iter()
        .map(|(asset, value)| (asset.to_string(), i128::from(*value)))
        .collect();
    for (sign, items) in [(1i128, &wallet_tx.outputs), (-1, &wallet_tx.inputs)] {
        for owned in items.iter().flatten().filter(|o| explicit(&o.unblinded)) {
            let entry = deltas.entry(owned.unblinded.asset.to_string()).or_default();
            *entry = entry
                .checked_add(sign * i128::from(owned.unblinded.value))
                .ok_or_else(overflow)?;
        }
    }
    let balance: Vec<(String, i128)> = deltas.into_iter().collect();
    let mut records = vec![];
    for (asset_id, delta) in balance {
        let transaction_fee = fees.get(&asset_id).copied().unwrap_or(0);
        let (fee, includes_fee) = allocated_fee(delta, transaction_fee, owns_input, mixed);
        if delta == 0 && fee == 0 {
            continue;
        }
        let (direction, amount, record_fee, kind) = if delta > 0 {
            ("inbound", delta, 0, "deposit")
        } else {
            let amount = if includes_fee {
                delta.abs()
            } else {
                (delta.abs() - fee).max(0)
            };
            (
                "outbound",
                amount,
                fee,
                if amount > 0 { "withdrawal" } else { "fee" },
            )
        };
        let asset = asset_code(&asset_id, source.policy_asset);
        let mut raw = json!({
            "txid": txid, "chain": "liquid", "network": source.network, "observer": "lwk",
            "ownership_graph_version": 1, "vin": vin, "vout": vout,
            "component": {
                "asset_id": asset_id, "asset": asset, "net_sats": delta, "fee_sats": fee,
                "transaction_fee_sats": transaction_fee,
                "fee_attribution": if includes_fee { "implicit_wallet_delta" } else { "exact" },
            },
        });
        if let Some(proof) = &htlc_spend {
            raw["htlc_spend"] = proof.clone();
        }
        let mut record = json!({
            "txid": txid, "occurred_at": occurred_at, "confirmed_at": confirmed_at,
            "direction": direction, "asset": asset, "amount": btc(amount), "fee": btc(record_fee),
            "amount_includes_fee": includes_fee, "fiat_rate": null, "fiat_value": null,
            "kind": kind, "description": format!("Synced from {}", source.backend_name),
            "counterparty": null, "raw_json": sorted_json(&raw)?,
        });
        if let Some((hash, _, _)) = &claim {
            record["payment_hash"] = json!(hash);
            record["payment_hash_source"] = json!("chain_script_unique_outpoint");
        }
        if let Some((funding_txid, funding_vout)) = &refund {
            record["swap_refund_funding_txid"] = json!(funding_txid);
            record["swap_refund_funding_vout"] = json!(funding_vout);
        }
        records.push(record);
    }
    Ok(records)
}

fn branch_key(chain: Chain) -> &'static str {
    match chain {
        Chain::Internal => "change",
        Chain::External => "receive",
    }
}

/// `_lwk_coverage`: the exact exclusive boundary submitted to the full scan,
/// widened only by an index LWK itself reported as used.
pub(crate) fn coverage(
    branch_keys: &[String],
    scan_to: u32,
    highest: &BTreeMap<&'static str, u32>,
) -> Vec<Value> {
    branch_keys
        .iter()
        .map(|key| {
            let used = highest.get(key.as_str()).copied();
            let scanned_to = (u64::from(scan_to) + 1).max(used.map_or(0, |u| u64::from(u) + 1));
            json!({"branch_key": key, "scanned_to": scanned_to, "highest_used": used,
                "details": {"observer": "lwk"}})
        })
        .collect()
}

/// The prepared update: JSON state, facts and the canonical txid set.
pub(crate) fn facts(
    source: &Source<'_>,
    wollet: &Wollet,
    tip: &BlockHeader,
    previous: &BTreeSet<String>,
    scan_to: u32,
) -> Result<Value> {
    let transactions = wollet.transactions().map_err(dependency_error)?;
    let mut records = vec![];
    for wallet_tx in &transactions {
        records.extend(self::records(source, wallet_tx)?);
    }
    let current: BTreeSet<String> = transactions.iter().map(|t| t.txid.to_string()).collect();
    let tip_height = tip.height;
    let mut highest = BTreeMap::<&'static str, u32>::new();
    for output in wollet.txos().map_err(dependency_error)? {
        let key = branch_key(output.ext_int);
        let entry = highest.entry(key).or_insert(output.wildcard_index);
        *entry = (*entry).max(output.wildcard_index);
    }
    let mut outputs = vec![];
    let mut utxos = wollet.utxos().map_err(dependency_error)?;
    // LWK's UTXO set omits explicit outputs; an explicit TXO no wallet
    // transaction spends is still held.
    let mut listed_or_spent: BTreeSet<_> = utxos.iter().map(|o| o.outpoint).collect();
    listed_or_spent.extend(
        transactions
            .iter()
            .flat_map(|t| t.tx.input.iter().map(|i| i.previous_output)),
    );
    utxos.extend(
        wollet
            .txos()
            .map_err(dependency_error)?
            .into_iter()
            .filter(|o| explicit(&o.unblinded) && !listed_or_spent.contains(&o.outpoint)),
    );
    // LWK iterates its unspent set as a hash map; publish outpoint order.
    utxos.sort_by_key(|o| (o.outpoint.txid.to_string(), o.outpoint.vout));
    for output in utxos {
        let key = branch_key(output.ext_int);
        let secrets = &output.unblinded;
        outputs.push(json!({
            "txid": output.outpoint.txid.to_string(), "vout": output.outpoint.vout,
            "asset": asset_code(&secrets.asset.to_string(), source.policy_asset),
            "amount_sats": secrets.value,
            "confirmation_status": if output.height.is_some() { "confirmed" } else { "mempool" },
            "confirmations": output.height.map_or(0, |h| (i64::from(tip_height) - i64::from(h) + 1).max(0)),
            "block_height": output.height, "block_time": null,
            "chain": "liquid", "network": source.network,
            "branch_index": u32::from(key == "change"), "branch_label": key,
            "address_index": output.wildcard_index, "address": output.address.to_string(),
            "script_pubkey": output.script_pubkey.as_bytes().to_hex(),
            "derivation_path": null, "derivation_paths": [], "key_origins": [],
            "raw": {"source": "lwk_wollet", "confidential": !explicit(secrets)},
        }));
    }
    let mut checkpoint_highest = Map::new();
    // Python sorts the branch keys ("change" before "receive").
    for (key, value) in &highest {
        checkpoint_highest.insert(u32::from(*key == "change").to_string(), json!(value));
    }
    Ok(json!({
        "state": {"schema_version": 1, "canonical_txids": current},
        "facts": {
            "transaction_records": records,
            "retracted_external_ids": previous.difference(&current).collect::<Vec<_>>(),
            "outputs": outputs,
            "coverage": coverage(source.branch_keys, scan_to, &highest),
            "freshness_checkpoint": {
                "observer": "lwk", "tip_height": tip_height,
                "tip_hash": tip.block_hash().to_string(),
                "highest_used": checkpoint_highest, "canonical_txids": current,
            },
        },
    }))
}
