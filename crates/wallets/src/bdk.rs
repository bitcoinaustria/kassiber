//! Native, request-local BDK execution. No BDK connector or file persistence.
//! This preparatory seam is intentionally not registered as product sync.
use crate::{
    bdk_state,
    scan::{Cancellation, Scope},
};
use bdk_chain::{BlockId, CheckPoint, ConfirmationBlockTime, Merge};
use bdk_core::{spk_client::SpkWithExpectedTxids, TxUpdate};
use bdk_wallet::{ChangeSet, KeychainKind, Update, Wallet};
use bitcoin::{
    block::Header,
    consensus,
    hashes::{sha256d, Hash, HashEngine},
    hex::FromHex,
    Transaction, Txid,
};
use kassiber_chain::{Backend, ElectrumClient};
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

type Result<T> = std::result::Result<T, AppError>;
fn invalid() -> AppError {
    AppError::new("protocol_error", "Invalid BDK Bitcoin observation response")
}
fn failed() -> AppError {
    AppError::new(
        "observer_state_rebuild_required",
        "Stored BDK wallet does not match its descriptor source",
    )
}

/// Immutable scope, source, state and transport snapshot. No Debug projection.
pub struct Execution {
    scope: Scope,
    identity: Arc<()>,
    backend: Backend,
    external: String,
    internal: Option<String>,
    network: bitcoin::Network,
    gap: u32,
    prior: Option<ChangeSet>,
}
pub struct ExecutionConsent {
    identity: Arc<()>,
    transport: Consent,
    started: AtomicBool,
}
impl ExecutionConsent {
    pub fn explicit_trigger(action: &Execution) -> Self {
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
/// Private in-memory result. The coordinator must still fence scope/epoch and
/// persist all facts atomically; this type conveys no absence authority.
pub struct Prepared {
    wallet: Wallet,
    aggregate: ChangeSet,
}
impl Prepared {
    pub fn changeset(&self) -> Value {
        bdk_state::encode(&self.aggregate)
    }
    pub fn wallet(&self) -> &Wallet {
        &self.wallet
    }
}
impl Execution {
    /// Inputs must come from one preflighted Bitcoin observer source. Parse only
    /// public descriptors, before handing anything to BDK's wallet constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: Scope,
        backend: &Value,
        external: &str,
        internal: Option<&str>,
        network: &str,
        gap: u32,
        prior: Option<&Value>,
    ) -> Result<Self> {
        if backend["kind"] != "electrum" {
            return Err(AppError::new(
                "not_implemented",
                "Native BDK execution requires Electrum; Esplora uses compatibility observation",
            ));
        }
        for descriptor in std::iter::once(external).chain(internal) {
            descriptor
                .parse::<miniscript::Descriptor<miniscript::DescriptorPublicKey>>()
                .map_err(|_| failed())?;
        }
        if gap == 0 {
            return Err(AppError::new(
                "validation",
                "Wallet gap limit must be positive",
            ));
        }
        let action = Self {
            scope,
            identity: Arc::new(()),
            backend: Backend::new(backend, true)?,
            external: external.into(),
            internal: internal.map(str::to_owned),
            network: bdk_state::network(network)?,
            gap,
            prior: prior.map(bdk_state::decode).transpose()?,
        };
        // Validate the source binding before any consent or network access.
        action.wallet()?;
        Ok(action)
    }
    fn wallet(&self) -> Result<Wallet> {
        if let Some(prior) = &self.prior {
            Wallet::load()
                .descriptor(KeychainKind::External, Some(self.external.clone()))
                .descriptor(KeychainKind::Internal, self.internal.clone())
                .check_network(self.network)
                .lookahead(self.gap)
                .load_wallet_no_persist(prior.clone())
                .map_err(|_| failed())?
                .ok_or_else(failed)
        } else {
            let create = match &self.internal {
                Some(change) => Wallet::create(self.external.clone(), change.clone()),
                None => Wallet::create_single(self.external.clone()),
            };
            create
                .network(self.network)
                .lookahead(self.gap)
                .create_wallet_no_persist()
                .map_err(|_| failed())
        }
    }
    pub fn run(
        &self,
        gate: &Gate,
        scope: &Scope,
        consent: Option<&ExecutionConsent>,
        deadline: Instant,
        cancel: &Cancellation,
        start_time: u64,
    ) -> Result<Prepared> {
        cancel.check(deadline)?;
        if scope != &self.scope {
            return Err(AppError::new(
                "stale_context",
                "BDK execution scope changed",
            ));
        }
        let proof = consent
            .filter(|p| Arc::ptr_eq(&p.identity, &self.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "BDK execution requires consent for this exact action",
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
                "BDK execution consent was already used",
            ));
        }
        let client = ElectrumClient::connect(
            self.backend.clone(),
            gate,
            Some(proof.transport.clone()),
            deadline,
        )?;
        let result = self.execute(
            &mut Wire {
                client,
                deadline,
                cancel,
            },
            start_time,
        )?;
        self.finish(gate, proof, deadline, cancel, result)
    }
    fn finish(
        &self,
        gate: &Gate,
        proof: &ExecutionConsent,
        deadline: Instant,
        cancel: &Cancellation,
        result: Prepared,
    ) -> Result<Prepared> {
        cancel.check(deadline)?;
        gate.authorize(
            self.backend.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| e.app_error())?;
        Ok(result)
    }
    fn execute(&self, rpc: &mut impl Rpc, start_time: u64) -> Result<Prepared> {
        let mut wallet = self.wallet()?;
        let mut aggregate = self.prior.clone().unwrap_or_default();
        let mut protocol = Protocol {
            rpc,
            txs: BTreeMap::new(),
            headers: BTreeMap::new(),
            anchors: BTreeMap::new(),
        };
        // Python's binding creates a fresh BdkElectrumClient for each refresh;
        // its transaction/header cache is request-local, not seeded from state.
        if self.prior.is_none() {
            let chain = protocol.chain_snapshot(wallet.latest_checkpoint())?;
            let mut request = wallet.start_full_scan_at(start_time).build();
            let mut update = Update::default();
            for key in request.keychains() {
                let last = protocol.scan(
                    request
                        .iter_spks(key)
                        .map(|(i, spk)| (i, SpkWithExpectedTxids::from(spk))),
                    self.gap as usize,
                    self.backend.batch_size(),
                    start_time,
                    &mut update.tx_update,
                )?;
                if let Some(last) = last {
                    update.last_active_indices.insert(key, last);
                }
            }
            protocol.complete(&mut update, chain)?;
            wallet.apply_update(update).map_err(|_| invalid())?;
            reveal(&mut wallet, self.gap, self.internal.is_some())?;
        } else {
            reveal(&mut wallet, self.gap, self.internal.is_some())?;
            let mut finished = false;
            for _ in 0..64 {
                let chain = protocol.chain_snapshot(wallet.latest_checkpoint())?;
                let mut request = wallet.start_sync_with_revealed_spks_at(start_time).build();
                let mut update = Update::default();
                protocol.scan(
                    request
                        .iter_spks_with_expected_txids()
                        .enumerate()
                        .map(|(i, s)| (i as u32, s)),
                    usize::MAX,
                    self.backend.batch_size(),
                    start_time,
                    &mut update.tx_update,
                )?;
                protocol.complete(&mut update, chain)?;
                wallet.apply_update(update).map_err(|_| invalid())?;
                if !reveal(&mut wallet, self.gap, self.internal.is_some())? {
                    finished = true;
                    break;
                }
            }
            if !finished {
                return Err(AppError::new(
                    "observer_scan_limit",
                    "BDK descriptor discovery exceeded the bounded widening limit",
                ));
            }
        }
        if let Some(staged) = wallet.take_staged() {
            aggregate.merge(staged);
        }
        Ok(Prepared { wallet, aggregate })
    }
}
fn reveal(wallet: &mut Wallet, gap: u32, internal: bool) -> Result<bool> {
    let mut highest = BTreeMap::<KeychainKind, u32>::new();
    for o in wallet.list_output() {
        highest
            .entry(o.keychain)
            .and_modify(|i| *i = (*i).max(o.derivation_index))
            .or_insert(o.derivation_index);
    }
    let mut changed = false;
    for key in [KeychainKind::External, KeychainKind::Internal] {
        if key == KeychainKind::Internal && !internal {
            continue;
        }
        let target = highest
            .get(&key)
            .map_or(Some(gap - 1), |i| i.checked_add(gap))
            .ok_or_else(invalid)?;
        if wallet.derivation_index(key).is_none_or(|i| i < target) {
            let before = wallet.derivation_index(key);
            wallet.reveal_addresses_to(key, target).for_each(drop);
            // Fixed descriptors always stop at index zero. They must not make
            // every widening pass appear to change the revealed set.
            changed |= wallet.derivation_index(key) != before;
        }
    }
    Ok(changed)
}
trait Rpc {
    fn call(&mut self, method: &str, params: Value) -> Result<Value>;
    fn batch(&mut self, method: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        params.into_iter().map(|p| self.call(method, p)).collect()
    }
}
struct Wire<'a> {
    client: ElectrumClient,
    deadline: Instant,
    cancel: &'a Cancellation,
}
impl Rpc for Wire<'_> {
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.cancel.check(self.deadline)?;
        let result = self.client.call(method, Some(&params), self.deadline)?;
        self.cancel.check(self.deadline)?;
        Ok(result)
    }
    fn batch(&mut self, method: &str, params: Vec<Value>) -> Result<Vec<Value>> {
        let mut result = vec![];
        for chunk in params.chunks(self.client.batch_size()) {
            self.cancel.check(self.deadline)?;
            result.extend(
                self.client.batch_call(
                    &chunk
                        .iter()
                        .map(|p| (method.into(), p.clone()))
                        .collect::<Vec<_>>(),
                    self.deadline,
                )?,
            );
        }
        self.cancel.check(self.deadline)?;
        Ok(result)
    }
}
struct Protocol<'a, R> {
    rpc: &'a mut R,
    txs: BTreeMap<Txid, Arc<Transaction>>,
    headers: BTreeMap<u32, Header>,
    anchors: BTreeMap<(Txid, bitcoin::BlockHash), ConfirmationBlockTime>,
}
impl<R: Rpc> Protocol<'_, R> {
    fn tx(&mut self, id: Txid) -> Result<Arc<Transaction>> {
        if let Some(tx) = self.txs.get(&id) {
            return Ok(tx.clone());
        }
        let raw = self
            .rpc
            .call("blockchain.transaction.get", json!([id.to_string()]))?;
        let tx: Transaction = consensus::deserialize(
            &Vec::from_hex(raw.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())?;
        if tx.compute_txid() != id {
            return Err(invalid());
        }
        let tx = Arc::new(tx);
        self.txs.insert(id, tx.clone());
        Ok(tx)
    }
    fn header(&mut self, height: u32) -> Result<Header> {
        if let Some(h) = self.headers.get(&height) {
            return Ok(*h);
        }
        let raw = self.rpc.call("blockchain.block.header", json!([height]))?;
        let header = consensus::deserialize(
            &Vec::from_hex(raw.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())?;
        self.headers.insert(height, header);
        Ok(header)
    }
    fn anchor(&mut self, id: Txid, height: u32) -> Result<Option<ConfirmationBlockTime>> {
        let mut header = self.header(height)?;
        if let Some(anchor) = self.anchors.get(&(id, header.block_hash())) {
            return Ok(Some(*anchor));
        }
        let proof = self.rpc.call(
            "blockchain.transaction.get_merkle",
            json!([id.to_string(), height]),
        )?;
        if !merkle(id, header, &proof)? {
            self.headers.remove(&height);
            header = self.header(height)?;
        }
        if !merkle(id, header, &proof)? {
            return Ok(None);
        }
        let anchor = ConfirmationBlockTime {
            block_id: BlockId {
                height,
                hash: header.block_hash(),
            },
            confirmation_time: header.time.into(),
        };
        self.anchors.insert((id, header.block_hash()), anchor);
        Ok(Some(anchor))
    }
    fn scan(
        &mut self,
        mut spks: impl Iterator<Item = (u32, SpkWithExpectedTxids)>,
        gap: usize,
        batch: usize,
        time: u64,
        update: &mut TxUpdate<ConfirmationBlockTime>,
    ) -> Result<Option<u32>> {
        let mut unused = 0usize;
        let mut last = None;
        loop {
            let chunk: Vec<_> = spks.by_ref().take(batch).collect();
            if chunk.is_empty() {
                break;
            }
            let histories = self.rpc.batch(
                "blockchain.scripthash.get_history",
                chunk
                    .iter()
                    .map(|(_, s)| {
                        crate::scan::script_hash(&s.spk.to_hex_string()).map(|h| json!([h]))
                    })
                    .collect::<Result<_>>()?,
            )?;
            if histories.len() != chunk.len() {
                return Err(invalid());
            }
            for ((i, script), history) in chunk.into_iter().zip(histories) {
                let history = history.as_array().ok_or_else(invalid)?;
                if history.is_empty() {
                    unused = unused.saturating_add(1);
                    if unused >= gap {
                        return Ok(last);
                    }
                } else {
                    unused = 0;
                    last = Some(i);
                }
                let mut present = BTreeSet::new();
                for item in history {
                    let id = Txid::from_str(item["tx_hash"].as_str().ok_or_else(invalid)?)
                        .map_err(|_| invalid())?;
                    present.insert(id);
                    update.txs.push(self.tx(id)?);
                    let height = crate::scan::integer(&item["height"])
                        .and_then(|n| i64::try_from(n).ok())
                        .ok_or_else(invalid)?;
                    if height > 0 {
                        let height = u32::try_from(height).map_err(|_| invalid())?;
                        if let Some(anchor) = self.anchor(id, height)? {
                            update.anchors.insert((anchor, id));
                        }
                    } else {
                        update.seen_ats.insert((id, time));
                    }
                }
                update.evicted_ats.extend(
                    script
                        .expected_txids
                        .into_iter()
                        .filter(|id| !present.contains(id))
                        .map(|id| (id, time)),
                );
            }
        }
        Ok(last)
    }
    fn complete(&mut self, update: &mut Update, mut chain: CheckPoint) -> Result<()> {
        // Same floating previous-output model as bdk_electrum. Whole parent
        // transactions do not enter the wallet graph unless independently seen.
        for tx in update.tx_update.txs.clone() {
            if tx.is_coinbase() {
                continue;
            }
            for input in &tx.input {
                let op = input.previous_output;
                let parent = self.tx(op.txid)?;
                let output = parent.output.get(op.vout as usize).ok_or_else(invalid)?;
                update.tx_update.txouts.insert(op, output.clone());
            }
        }
        for (a, _) in &update.tx_update.anchors {
            // The pre-scan suffix is authoritative over anchors fetched later,
            // as in bdk_electrum's chain_update. Reorg recovery is step 2.
            if a.block_id.height <= chain.height() && chain.get(a.block_id.height).is_none() {
                chain = chain.insert(a.block_id);
            }
        }
        update.chain = Some(chain);
        Ok(())
    }
    fn chain_snapshot(&mut self, prior: CheckPoint) -> Result<CheckPoint> {
        // BDK snapshots the tip before histories, using one suffix response.
        let notification = self.rpc.call("blockchain.headers.subscribe", json!([]))?;
        let height = notification["height"]
            .as_u64()
            .and_then(|h| u32::try_from(h).ok())
            .ok_or_else(invalid)?;
        if height < prior.height() {
            return Err(AppError::new(
                "backend_tip_behind",
                "The selected Bitcoin backend is behind the persisted wallet state",
            )
            .retryable(true));
        }
        let start = height.saturating_sub(7);
        let response = self
            .rpc
            .call("blockchain.block.headers", json!([start, 8]))?;
        let raws = response["headers"].as_array().ok_or_else(invalid)?;
        if raws.len() != (height - start + 1) as usize
            || response["count"].as_u64() != Some(raws.len() as u64)
        {
            return Err(invalid());
        }
        let suffix = raws
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                let header: Header = consensus::deserialize(
                    &Vec::from_hex(raw.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?,
                )
                .map_err(|_| invalid())?;
                Ok((start + i as u32, header.block_hash()))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let prior_hash = match suffix.get(&prior.height()) {
            Some(hash) => *hash,
            None => self.header(prior.height())?.block_hash(),
        };
        // Step 2 adds stale-anchor rebuilding and point-of-agreement recovery.
        // Until then refuse disconnected state before preparing any update.
        if prior_hash != prior.hash() {
            return Err(failed());
        }
        let mut chain = prior;
        for (h, hash) in suffix {
            if h > chain.height() {
                chain = chain.insert(BlockId { height: h, hash });
            }
        }
        Ok(chain)
    }
}
fn merkle(id: Txid, header: Header, proof: &Value) -> Result<bool> {
    let mut position = proof["pos"].as_u64().ok_or_else(invalid)?;
    let mut current = id.to_raw_hash();
    for node in proof["merkle"].as_array().ok_or_else(invalid)? {
        let next =
            sha256d::Hash::from_str(node.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
        let mut engine = sha256d::Hash::engine();
        let (left, right) = if position % 2 == 0 {
            (current, next)
        } else {
            (next, current)
        };
        engine.input(left.as_byte_array());
        engine.input(right.as_byte_array());
        current = sha256d::Hash::from_engine(engine);
        position /= 2;
    }
    Ok(current == header.merkle_root.to_raw_hash())
}

#[cfg(test)]
mod tests;
