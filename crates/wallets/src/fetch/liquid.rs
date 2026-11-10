//! Python's Liquid compatibility observers (`record_components_from_liquid_tx`
//! and `_liquid_utxo_record_from_output` over the Esplora and Electrum
//! compatibility adapters). Pure: the cached acquisition fetched and verified
//! every transaction; blinding secrets are derived per script and never leave
//! this module.
use super::electrum::{explicit_time, header_time, positive_height, sort_key, target_metadata};
use super::*;
use crate::bitcoin_records::{self, btc, sorted_json, UNKNOWN};
use elements_miniscript::elements::{
    confidential, encode,
    hex::{FromHex, ToHex},
    secp256k1_zkp::Secp256k1,
    Transaction, TxOut,
};
use serde_json::Map;

fn invalid() -> AppError {
    AppError::new("protocol_error", "Invalid Liquid observation")
}

/// What the projection reads: the plan's blinding material, the observed
/// targets by script, and the fetched transactions.
pub(super) struct Context<'a> {
    pub plan: &'a Plan,
    pub targets: BTreeMap<String, &'a Value>,
    pub policy: &'a str,
    pub backend: &'a str,
    pub network: &'a str,
}

impl<'a> Context<'a> {
    pub(super) fn new(
        plan: &'a Plan,
        targets: &'a [Value],
        policy: &'a str,
        backend: &'a str,
        network: &'a str,
    ) -> Self {
        // `tracked_scripts`: a later target of the same script wins.
        let targets = targets
            .iter()
            .filter(|t| descriptors::truthy(&t["script_pubkey"]))
            .map(|t| (text(&t["script_pubkey"]).to_owned(), t))
            .collect();
        Self {
            plan,
            targets,
            policy,
            backend,
            network,
        }
    }

    /// `liquid_output_amount_asset_id`: an owned blinded output unblinds with
    /// its script's secret; any other output must be explicit.
    fn amount_asset(&self, output: &TxOut, target: Option<&Value>) -> Result<(u64, String)> {
        let label = match target {
            Some(t) if descriptors::truthy(&t["address"]) => text(&t["address"]).to_owned(),
            Some(t) => text(&t["script_pubkey"]).to_owned(),
            None => "fee output".into(),
        };
        let failed = || {
            AppError::new(
                "app_error",
                format!("Failed to decode Liquid output for {label}"),
            )
        };
        if output.nonce != confidential::Nonce::Null {
            let target = target.ok_or_else(|| {
                AppError::new(
                    "app_error",
                    "Unable to unblind Liquid output without wallet descriptor context",
                )
            })?;
            let index = |key: &str| {
                target[key]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(failed)
            };
            let secret = self
                .plan
                .liquid_blinding_secret(index("branch_index")?, index("address_index")?)
                .map_err(|_| failed())?;
            let secrets = output
                .unblind(&Secp256k1::new(), secret)
                .map_err(|_| failed())?;
            return Ok((secrets.value, secrets.asset.to_string()));
        }
        let confidential::Value::Explicit(value) = output.value else {
            return Err(AppError::new(
                "app_error",
                "Unsupported confidential Liquid value encoding while decoding transaction",
            ));
        };
        let confidential::Asset::Explicit(asset) = output.asset else {
            return Err(AppError::new(
                "app_error",
                "Unsupported Liquid asset encoding while decoding transaction",
            ));
        };
        Ok((value, asset.to_string()))
    }

    fn code(&self, asset: &str) -> String {
        crate::lwk::facts::asset_code(asset, self.policy)
    }

    /// `record_components_from_liquid_tx`: one record per asset whose net
    /// or (for a funding wallet) explicit fee is non-zero.
    fn records(
        &self,
        txid: &str,
        occurred: &str,
        tx: &Transaction,
        raw: &Map<String, Value>,
        mut context: Map<String, Value>,
        confirmed: Option<String>,
    ) -> Result<Vec<Value>> {
        let mut balance: BTreeMap<String, i128> = BTreeMap::new();
        let mut fees: BTreeMap<String, i128> = BTreeMap::new();
        let mut vout = vec![];
        for (n, output) in tx.output.iter().enumerate() {
            let script = output.script_pubkey.to_hex();
            let mut stored = json!({"n": n, "scriptpubkey": script});
            if script.is_empty() {
                let (value, asset) = self.amount_asset(output, None)?;
                *fees.entry(asset.clone()).or_default() += i128::from(value);
                stored["value_sats"] = json!(value);
                stored["asset"] = json!(self.code(&asset));
                stored["asset_id"] = json!(asset);
                stored["role"] = json!("fee");
            } else if let Some(target) = self.targets.get(&script) {
                let (value, asset) = self.amount_asset(output, Some(target))?;
                *balance.entry(asset.clone()).or_default() += i128::from(value);
                stored["value_sats"] = json!(value);
                stored["asset"] = json!(self.code(&asset));
                stored["asset_id"] = json!(asset);
                stored["role"] = json!("owned");
            } else if let Ok((value, asset)) = self.amount_asset(output, None) {
                // A confidential foreign output stays script-only.
                stored["value_sats"] = json!(value);
                stored["asset"] = json!(self.code(&asset));
                stored["asset_id"] = json!(asset);
                stored["role"] = json!("external");
            }
            vout.push(stored);
        }
        let mut vin = vec![];
        let mut evidence = vec![];
        let mut funded = false;
        for input in &tx.input {
            let previous = input.previous_output.txid.to_string();
            let index = input.previous_output.vout;
            let parent = decode(raw, &previous)?;
            let output = parent.output.get(index as usize).ok_or_else(|| {
                AppError::new(
                    "app_error",
                    format!(
                        "Liquid prevout index {index} is out of range for transaction {previous}"
                    ),
                )
            })?;
            let script = output.script_pubkey.to_hex();
            let prevout = if let Some(target) = self.targets.get(&script) {
                let (value, asset) = self.amount_asset(output, Some(target))?;
                funded = true;
                *balance.entry(asset.clone()).or_default() -= i128::from(value);
                json!({"scriptpubkey": script, "value_sats": value, "asset_id": asset,
                    "asset": self.code(&asset), "role": "owned"})
            } else {
                let mut prevout = json!({"scriptpubkey": script, "role": "external"});
                if let Ok((value, asset)) = self.amount_asset(output, None) {
                    prevout["value_sats"] = json!(value);
                    prevout["asset"] = json!(self.code(&asset));
                    prevout["asset_id"] = json!(asset);
                }
                prevout
            };
            vin.push(json!({"txid": previous, "vout": index, "prevout": prevout}));
            let witness: Vec<String> = input
                .witness
                .script_witness
                .iter()
                .map(|item| item.iter().map(|b| format!("{b:02x}")).collect())
                .collect();
            evidence.push(json!({"txid": previous, "vout": index, "witness": witness}));
        }
        // The claim and refund links Python reads from the input witnesses.
        let mut links = json!({});
        bitcoin_records::witness_evidence(&json!({"vin": evidence}), &mut links);
        // Only a wallet that funded the transaction paid its fee.
        if !funded {
            fees.clear();
        }
        let assets: BTreeSet<&String> = balance.keys().chain(fees.keys()).collect();
        let confirmed = confirmed.or_else(|| (occurred != UNKNOWN).then(|| occurred.to_owned()));
        context.insert("txid".into(), json!(txid));
        context.insert("chain".into(), json!("liquid"));
        context.insert("network".into(), json!(self.network));
        context.insert("ownership_graph_version".into(), json!(1));
        context.insert("vin".into(), json!(vin));
        context.insert("vout".into(), json!(vout));
        let mut records = vec![];
        for asset in assets {
            let net_value = balance.get(asset).copied().unwrap_or(0);
            let fee = fees.get(asset).copied().unwrap_or(0);
            if net_value == 0 && fee == 0 {
                continue;
            }
            let (direction, amount, row_fee, kind) = if net_value > 0 {
                ("inbound", net_value, 0, "deposit")
            } else {
                let amount = (-net_value - fee).max(0);
                (
                    "outbound",
                    amount,
                    fee,
                    if amount > 0 { "withdrawal" } else { "fee" },
                )
            };
            let code = self.code(asset);
            let mut raw_json = context.clone();
            raw_json.insert(
                "component".into(),
                json!({"asset_id": asset, "asset": code, "net_sats": net_value, "fee_sats": fee}),
            );
            let mut record = json!({"txid": txid, "occurred_at": occurred, "confirmed_at": confirmed,
                "direction": direction, "asset": code, "amount": btc(amount), "fee": btc(row_fee),
                "fiat_rate": null, "fiat_value": null, "kind": kind,
                "description": format!("Synced from {}", self.backend), "counterparty": null,
                "raw_json": sorted_json(&Value::Object(raw_json))?});
            for (key, value) in links.as_object().into_iter().flatten() {
                record[key] = value.clone();
            }
            records.push(record);
        }
        Ok(records)
    }

    /// `_liquid_utxo_record_from_output`.
    #[allow(clippy::too_many_arguments)]
    fn output(
        &self,
        txid: &Value,
        vout: &Value,
        status: &Value,
        block_time: Option<String>,
        raw: &Map<String, Value>,
        target: &Value,
        source: &str,
        tip: Option<i64>,
    ) -> Result<Value> {
        let id = text(txid);
        let tx = decode(raw, id)?;
        let index = crate::scan::integer(vout)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n < tx.output.len())
            .ok_or_else(|| {
                AppError::new(
                    "app_error",
                    format!(
                        "Liquid UTXO output index {} is out of range for transaction {id}",
                        descriptors::text(vout)
                    ),
                )
            })?;
        let output = &tx.output[index];
        if output.script_pubkey.to_hex() != text(&target["script_pubkey"]) {
            return Err(AppError::new(
                "app_error",
                format!(
                    "Liquid UTXO {id}:{} did not match the tracked script",
                    descriptors::text(vout)
                ),
            ));
        }
        let (value, asset) = self.amount_asset(output, Some(target))?;
        let height = positive_height(&status["block_height"]);
        let confirmed = descriptors::truthy(&status["confirmed"]);
        let confirmations = height
            .zip(tip)
            .filter(|(h, t)| *t >= *h)
            .map(|(h, t)| t - h + 1);
        let mut record = json!({"txid": txid, "vout": index, "asset": self.code(&asset),
            "amount_sats": value, "confirmation_status": if confirmed {"confirmed"} else {"mempool"},
            "confirmations": confirmations, "block_height": height, "block_time": block_time,
            "chain": "liquid", "network": self.network});
        let fields = record.as_object_mut().expect("object");
        fields.extend(target_metadata(target));
        fields.insert(
            "raw".into(),
            json!({"source": source, "asset_id": asset, "confirmed": confirmed}),
        );
        Ok(record)
    }
}

fn decode(raw: &Map<String, Value>, txid: &str) -> Result<Transaction> {
    let hex = raw
        .get(txid)
        .and_then(|tx| tx["raw_hex"].as_str())
        .ok_or_else(invalid)?;
    let bytes = Vec::<u8>::from_hex(hex).map_err(|_| invalid())?;
    encode::deserialize(&bytes).map_err(|_| invalid())
}

/// The record loop of `compatibility_electrum_records_for_wallet` for
/// Liquid: every changed history in Python's order, dated by its header.
pub(super) fn electrum_records(
    acquired: &Value,
    context: &Context<'_>,
    deadline: Instant,
    control: &dyn Control,
) -> Result<Vec<Value>> {
    let histories = acquired["histories"].as_object().ok_or_else(invalid)?;
    let raw = acquired["transactions"].as_object().ok_or_else(invalid)?;
    let headers = &acquired["freshness_checkpoint"]["electrum_headers"];
    let mut ordered: Vec<(&String, &Value)> = histories.iter().collect();
    ordered.sort_by_cached_key(|(txid, history)| sort_key(txid, history));
    let total = ordered.len();
    let mut records = vec![];
    for (index, (txid, history)) in ordered.into_iter().enumerate() {
        control.check(deadline)?;
        let occurred = match positive_height(&history["height"]) {
            Some(height) => {
                header_time(&headers[height.to_string()])?.unwrap_or_else(|| UNKNOWN.into())
            }
            None => explicit_time(history).unwrap_or_else(|| UNKNOWN.into()),
        };
        let tx = decode(raw, txid)?;
        let mut extra = Map::new();
        extra.insert("history".into(), history.clone());
        extra.insert("raw_hex".into(), raw[txid.as_str()]["raw_hex"].clone());
        extra.insert("status".into(), json!({"confirmed": occurred != UNKNOWN}));
        let confirmed = (occurred != UNKNOWN).then(|| occurred.clone());
        records.extend(context.records(txid, &occurred, &tx, raw, extra, confirmed)?);
        if (index + 1).is_multiple_of(100) || index + 1 == total {
            control.progress(json!({"phase":"decode_enrich","transactions_seen":index+1,
                "transactions_total":total,"records":records.len(),"processed":index+1,"total":total}))?;
        }
    }
    Ok(records)
}

/// The Liquid record loop of `compatibility_esplora_records_for_wallet`:
/// transactions by block time, then txid.
pub(super) fn esplora_records(
    acquired: &Value,
    context: &Context<'_>,
    deadline: Instant,
    control: &dyn Control,
) -> Result<Vec<Value>> {
    let raw = acquired["raw_transactions"]
        .as_object()
        .ok_or_else(invalid)?;
    let mut ordered: Vec<&Value> = acquired["transactions"]
        .as_object()
        .ok_or_else(invalid)?
        .values()
        .collect();
    ordered.sort_by_key(|tx| {
        (
            tx["status"]["block_time"].as_i64().unwrap_or(0),
            text(&tx["txid"]).to_owned(),
        )
    });
    let total = ordered.len();
    let mut records = vec![];
    for (index, tx) in ordered.into_iter().enumerate() {
        control.check(deadline)?;
        let txid = text(&tx["txid"]);
        let block_time = &tx["status"]["block_time"];
        let occurred = bitcoin_records::timestamp(block_time)?.unwrap_or_else(|| UNKNOWN.into());
        let mut extra = Map::new();
        extra.insert("tx".into(), tx.clone());
        extra.insert("raw_hex".into(), raw[txid]["raw_hex"].clone());
        let decoded = decode(raw, txid)?;
        let confirmed = bitcoin_records::timestamp(block_time)?;
        records.extend(context.records(txid, &occurred, &decoded, raw, extra, confirmed)?);
        if (index + 1).is_multiple_of(100) || index + 1 == total {
            control.progress(json!({"phase":"decode_enrich","transactions_seen":index+1,
                "transactions_total":total,"records":records.len(),"processed":index+1,"total":total}))?;
        }
    }
    Ok(records)
}

/// `compatibility_electrum_utxos_for_wallet` for Liquid, in target order.
pub(super) fn electrum_outputs(acquired: &Value, context: &Context<'_>) -> Result<Vec<Value>> {
    let tip = acquired["tip_height"].as_i64();
    let headers = &acquired["utxo_headers"];
    let raw = acquired["utxo_transactions"]
        .as_object()
        .ok_or_else(invalid)?;
    let mut outputs = vec![];
    for script in acquired["scripts"].as_array().ok_or_else(invalid)? {
        let listed = match &script["utxos"] {
            Value::Null => continue,
            Value::Array(listed) => listed,
            _ => return Err(invalid()),
        };
        for utxo in listed {
            if !utxo.is_object() {
                return Err(invalid());
            }
            // `_electrum_utxo_status`.
            let height = positive_height(&utxo["height"]);
            let (confirmed, block_time) = match height {
                Some(height) => (true, header_time(&headers[height.to_string()])?),
                None => {
                    let time = explicit_time(utxo);
                    (time.is_some(), time)
                }
            };
            let status = json!({"confirmed": confirmed, "block_height": height});
            outputs.push(context.output(
                &utxo["tx_hash"],
                &utxo["tx_pos"],
                &status,
                block_time,
                raw,
                &script["target"],
                "liquid_electrum_scripthash_listunspent",
                tip,
            )?);
        }
    }
    Ok(outputs)
}

/// `compatibility_esplora_utxos_for_wallet` for Liquid, in target order.
pub(super) fn esplora_outputs(acquired: &Value, context: &Context<'_>) -> Result<Vec<Value>> {
    let tip = acquired["tip_height"].as_i64();
    let raw = acquired["raw_transactions"]
        .as_object()
        .ok_or_else(invalid)?;
    let mut outputs = vec![];
    for script in acquired["scripts"].as_array().ok_or_else(invalid)? {
        for utxo in script["utxos"].as_array().ok_or_else(invalid)? {
            let status = if utxo["status"].is_object() {
                utxo["status"].clone()
            } else {
                json!({})
            };
            let block_time = bitcoin_records::timestamp(&status["block_time"]).unwrap_or(None);
            outputs.push(context.output(
                &utxo["txid"],
                &utxo["vout"],
                &status,
                block_time,
                raw,
                &script["target"],
                "liquid_esplora_scripthash_utxo",
                tip,
            )?);
        }
    }
    Ok(outputs)
}
