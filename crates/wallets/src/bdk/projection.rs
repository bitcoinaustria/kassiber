//! JSON-only record projection of native BDK canonical history. No I/O.
use super::*;
use crate::{
    bitcoin_records,
    observer_state::{record_fingerprint, RecordReuse},
};
use bdk_chain::ChainPosition;
use bitcoin::{OutPoint, TxOut};

pub struct RecordProjection {
    pub records: Vec<Value>,
    pub fingerprints: BTreeMap<String, String>,
    pub reused_records: u64,
}

/// The stored plan's mapping from BDK keychains to Kassiber policy branches.
pub struct Branch {
    pub key: String,
    pub index: u32,
    pub label: String,
    pub internal: bool,
}

fn output(output: &TxOut) -> Value {
    json!({"value":output.value.to_sat(),"scriptpubkey":output.script_pubkey.to_hex_string()})
}
impl Prepared {
    /// Assemble the complete JSON-only observer update. Output inventory is a
    /// full snapshot even when every confirmed transaction record is reused.
    pub fn update(
        &self,
        branches: &[Branch],
        network: &str,
        backend_name: &str,
        fingerprint_version: &str,
        prior_state: Option<&Value>,
        current_txids: Option<BTreeSet<String>>,
    ) -> Result<Value> {
        if branches.is_empty()
            || branches.len() > 2
            || branches.iter().filter(|b| !b.internal).count() != 1
            || branches.iter().filter(|b| b.internal).count() > 1
        {
            return Err(AppError::new(
                "observer_capability_unsupported",
                "The Bitcoin descriptor source cannot be represented by one BDK observer",
            ));
        }
        let projection = self.records(
            backend_name,
            fingerprint_version,
            prior_state,
            current_txids,
        )?;
        let tip = self.wallet.latest_checkpoint();
        let position = |p: ChainPosition<ConfirmationBlockTime>| match p {
            ChainPosition::Confirmed { anchor, .. } => json!({
                "confirmation_status":"confirmed", "confirmations":tip.height().saturating_add(1).saturating_sub(anchor.block_id.height),
                "block_height":anchor.block_id.height,"block_time":anchor.confirmation_time}),
            ChainPosition::Unconfirmed { .. } => {
                json!({"confirmation_status":"mempool","confirmations":0,"block_height":null,"block_time":null})
            }
        };
        let mut highest = BTreeMap::<KeychainKind, u32>::new();
        for local in self.wallet.list_output() {
            highest
                .entry(local.keychain)
                .and_modify(|i| *i = (*i).max(local.derivation_index))
                .or_insert(local.derivation_index);
        }
        let mut outputs = vec![];
        for local in self.wallet.list_unspent() {
            let Some(branch) = branches
                .iter()
                .find(|b| b.internal == (local.keychain == KeychainKind::Internal))
            else {
                continue;
            };
            let mut output = position(local.chain_position);
            let confirmed = output["confirmation_status"] == "confirmed";
            let script = &local.txout.script_pubkey;
            let fields = json!({"txid":local.outpoint.txid.to_string(),"vout":local.outpoint.vout,"asset":"BTC",
                "amount_sats":local.txout.value.to_sat(),"chain":"bitcoin","network":network,
                "branch_index":branch.index,"branch_label":branch.label,"address_index":local.derivation_index,
                "address":bitcoin::Address::from_script(script,self.wallet.network()).ok().map(|a|a.to_string()),
                "script_pubkey":script.to_hex_string(),"derivation_path":null,"derivation_paths":[],"key_origins":[],
                "raw":{"source":"bdk_wallet","confirmed":confirmed}});
            output
                .as_object_mut()
                .expect("position")
                .extend(fields.as_object().expect("output").clone());
            outputs.push(output);
        }
        let mut coverage = vec![];
        let mut checkpoint_highest = serde_json::Map::new();
        for branch in branches {
            let keychain = if branch.internal {
                KeychainKind::Internal
            } else {
                KeychainKind::External
            };
            let used = highest.get(&keychain).copied();
            let scanned_to = self
                .wallet
                .derivation_index(keychain)
                .map_or(0u64, |i| u64::from(i) + 1);
            if used.is_some_and(|i| u64::from(i) >= scanned_to) {
                return Err(AppError::new(
                    "observer_accounting_ambiguous",
                    "BDK reported a used output outside its revealed scan horizon",
                ));
            }
            coverage.push(
                json!({"branch_key":branch.key,"scanned_to":scanned_to,"highest_used":used,
                "details":{"branch_index":branch.index,"observer":"bdk"}}),
            );
            if let Some(used) = used {
                checkpoint_highest.insert(branch.index.to_string(), json!(used));
            }
        }
        Ok(
            json!({"state":{"schema_version":1,"bdk_changeset":self.changeset(),"canonical_txids":self.canonical_txids,
                "record_fingerprint_version":fingerprint_version,"record_fingerprints":projection.fingerprints},
            "facts":{"transaction_records":projection.records,"retracted_external_ids":self.retracted_external_ids,
                "outputs":outputs,"coverage":coverage,"reused_records":projection.reused_records,
                "freshness_checkpoint":{"observer":"bdk","tip_height":tip.height(),"tip_hash":tip.hash().to_string(),
                    "highest_used":checkpoint_highest,"canonical_txids":self.canonical_txids}}}),
        )
    }
    /// The coordinator supplies reuse evidence from the same scoped snapshot.
    /// A fresh, forced or reorg scan always disables reuse independently of it.
    pub fn records(
        &self,
        backend_name: &str,
        fingerprint_version: &str,
        prior_state: Option<&Value>,
        current_txids: Option<BTreeSet<String>>,
    ) -> Result<RecordProjection> {
        let reuse = RecordReuse::new(
            prior_state,
            self.full_scan,
            fingerprint_version,
            current_txids,
        );
        let mut txouts = BTreeMap::new();
        for tx in &self.aggregate.tx_graph.txs {
            let id = tx.compute_txid();
            for (index, output) in tx.output.iter().enumerate() {
                txouts.insert(OutPoint::new(id, index as u32), output.clone());
            }
        }
        txouts.extend(
            self.aggregate
                .tx_graph
                .txouts
                .iter()
                .map(|(point, output)| (*point, output.clone())),
        );
        let tracked: BTreeSet<_> = txouts
            .values()
            .filter(|o| {
                self.wallet
                    .derivation_of_spk(o.script_pubkey.clone())
                    .is_some()
            })
            .map(|o| o.script_pubkey.to_hex_string())
            .collect();
        let mut projection = RecordProjection {
            records: vec![],
            fingerprints: BTreeMap::new(),
            reused_records: 0,
        };
        let tip = self.wallet.latest_checkpoint().height();
        // bdkpython's wallet projection sorts newest chain positions first.
        // The underlying Rust iterator itself has no presentation ordering.
        for canonical in self
            .wallet
            .transactions_sort_by(|a, b| b.chain_position.cmp(&a.chain_position))
        {
            let tx = &canonical.tx_node.tx;
            let id = tx.compute_txid();
            let vin: Vec<_> = tx.input.iter().map(|input| json!({"txid":input.previous_output.txid.to_string(),"vout":input.previous_output.vout,"prevout":txouts.get(&input.previous_output).map(output),"witness":input.witness.iter().map(|v| bitcoin::hex::DisplayHex::as_hex(v).to_string()).collect::<Vec<_>>()})).collect();
            let vout: Vec<_> = tx.output.iter().map(output).collect();
            let position = match canonical.chain_position {
                ChainPosition::Confirmed { anchor, .. } => {
                    json!({"status":{"confirmed":true,"block_height":anchor.block_id.height,"block_hash":anchor.block_id.hash.to_string(),"block_time":anchor.confirmation_time},"confirmation_status":"confirmed","block_height":anchor.block_id.height,"block_time":anchor.confirmation_time,"confirmations":tip.saturating_add(1).saturating_sub(anchor.block_id.height)})
                }
                ChainPosition::Unconfirmed { .. } => {
                    json!({"status":{"confirmed":false},"confirmation_status":"mempool","block_height":null,"block_time":null,"confirmations":0})
                }
            };
            let observed = self.aggregate.tx_graph.first_seen.get(&id).copied();
            let owned =
                bitcoin_records::graph_scoped_scripts(&json!({"vin":vin,"vout":vout}), &tracked);
            let fingerprint = record_fingerprint(
                fingerprint_version,
                &json!({"txid":id.to_string(),"vin":vin,"vout":vout,"position":position,"observed_at":observed,"owned_scripts":owned,"backend":backend_name}),
            )?;
            projection
                .fingerprints
                .insert(id.to_string(), fingerprint.clone());
            if reuse.can_reuse(
                &id.to_string(),
                position["status"]["confirmed"] == true,
                &fingerprint,
            ) {
                projection.reused_records += 1;
                continue;
            }
            let details = self.wallet.tx_details(id);
            let owns_input = vin
                .iter()
                .any(|v| tracked.contains(bitcoin_records::script(&v["prevout"])));
            let fee = details.as_ref().and_then(|d| d.fee);
            if owns_input && fee.is_none() {
                return Err(AppError::new("observer_accounting_ambiguous", "BDK could not prove the network fee for an owned spend").with_hint("Retry with a backend that serves every previous output; Kassiber will not book a zero fee.").with_details(kassiber_pytext::loads(&json!({"observer":"bdk","txid":id.to_string(),"missing":"network_fee"}).to_string()).expect("JSON details")));
            }
            let mut raw = position;
            raw["txid"] = json!(id.to_string());
            raw["vin"] = json!(vin);
            raw["vout"] = json!(vout);
            raw["fee"] = json!(fee.map_or(0, |f| f.to_sat()));
            raw["observed_at"] = json!(observed);
            raw["observer"] = json!("bdk");
            raw["observer_owned_scripts"] = json!(owned);
            if let Some(record) = bitcoin_records::from_esplora(&raw, &tracked, backend_name)? {
                projection.records.push(record);
            }
        }
        Ok(projection)
    }
}
