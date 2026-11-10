//! Native, request-local LWK execution for Liquid descriptor wallets.
//! `lwk_wollet` 0.18.0 owns gap discovery, unblinding and wollet state, as in
//! the Python binding. It never opens a connection: its `BlockchainBackend`
//! is Kassiber's Electrum client (`backend`). Its opaque store is buffered in
//! memory and becomes durable only inside the caller's apply savepoint.
use crate::{
    descriptors::Plan,
    scan::{Control, Scope},
};
use kassiber_chain::{Backend, ElectrumClient};
use kassiber_envelope::{dict, redaction, s, AppError};
use kassiber_net::{Consent, Gate};
use lwk_common::ElementsParamsBuilder;
use lwk_wollet::{
    clients::blocking::BlockchainBackend, elements::AssetId, BoxError, DynStore, Network, Wollet,
    WolletBuilder, WolletDescriptor,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
mod backend;
pub mod descriptor;
pub(crate) mod facts;
pub use backend::Rpc;
use backend::Wire;

type Result<T> = std::result::Result<T, AppError>;

/// The Python store's `LWK_OBSERVER_STATE_VERSION`.
pub const STATE_VERSION: i64 = 1;

/// Opaque string-keyed bytes, exactly as Python's `SqlCipherForeignStore`
/// buffers them. Not persisted (so LWK does not encrypt) and never printed.
#[derive(Default)]
struct Buffered(Mutex<BTreeMap<String, Vec<u8>>>);
impl std::fmt::Debug for Buffered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Buffered(..)")
    }
}
impl DynStore for Buffered {
    fn get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, BoxError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "store poisoned")?
            .get(key)
            .cloned())
    }
    fn put(&self, key: &str, value: &[u8]) -> std::result::Result<(), BoxError> {
        self.0
            .lock()
            .map_err(|_| "store poisoned")?
            .insert(key.to_owned(), value.to_vec());
        Ok(())
    }
    fn remove(&self, key: &str) -> std::result::Result<(), BoxError> {
        self.0.lock().map_err(|_| "store poisoned")?.remove(key);
        Ok(())
    }
}

/// `lwk_network`: Liquid mainnet, testnet, or Elements regtest with the
/// wallet's policy asset.
pub(crate) fn network(network: &str, policy_asset: &str) -> Result<Network> {
    match kassiber_pytext::strip(network).to_lowercase().as_str() {
        "liquidv1" => Ok(Network::Liquid),
        "liquidtestnet" => Ok(Network::TestnetLiquid),
        // `lwk.Network.regtest(policy_asset)`: default regtest genesis.
        "elementsregtest" => Ok(Network::CustomElements(
            ElementsParamsBuilder::new()
                .with_policy_asset(AssetId::from_str(policy_asset).map_err(|_| {
                    AppError::new("observer_capability_unsupported", "Unsupported LWK network")
                })?)
                .build()
                .map_err(|_| {
                    AppError::new("observer_capability_unsupported", "Unsupported LWK network")
                })?,
        )),
        _ => Err(AppError::new(
            "observer_capability_unsupported",
            "Unsupported LWK network",
        )),
    }
}

/// Identity-scoped inputs captured with the stored state, before consent.
pub struct Refresh {
    pub force_full: bool,
    /// The stored JSON payload (`schema_version`, `canonical_txids`).
    pub prior: Option<Value>,
    /// Stored coverage points (`highest_used`).
    pub prior_coverage: Vec<Value>,
    /// This identity's freshness checkpoint.
    pub checkpoint: Value,
    /// Opaque wollet values; empty for a forced rebuild.
    pub values: BTreeMap<String, Vec<u8>>,
}

/// Immutable scope, source, state and transport snapshot. No Debug projection.
pub struct Execution {
    scope: Scope,
    identity: Arc<()>,
    backend: Backend,
    backend_name: String,
    descriptor: WolletDescriptor,
    network: Network,
    network_name: String,
    policy_asset: String,
    branch_keys: Vec<String>,
    gap_limit: u32,
    refresh: Refresh,
}

pub struct ExecutionConsent {
    identity: Arc<()>,
    transport: Consent,
    started: AtomicBool,
}
impl ExecutionConsent {
    pub(crate) fn for_fetch(action: &Execution, transport: Consent) -> Self {
        Self {
            identity: action.identity.clone(),
            transport,
            started: AtomicBool::new(false),
        }
    }
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

/// The prepared update and its buffered opaque values. The coordinator must
/// still fence scope/epoch and persist both inside one savepoint.
pub struct Prepared {
    pub update: Value,
    pub values: BTreeMap<String, Vec<u8>>,
}

fn rebuild_required() -> AppError {
    AppError::new(
        "observer_state_rebuild_required",
        "Stored LWK observer state must be rebuilt",
    )
}

impl Execution {
    /// Inputs come from one preflighted Liquid observer identity: `branch_keys`
    /// from the identity, `plan` from the same captured wallet configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: Scope,
        backend: &Value,
        plan: &Plan,
        network_name: &str,
        policy_asset: &str,
        branch_keys: Vec<String>,
        refresh: Refresh,
    ) -> Result<Self> {
        if backend["kind"] != "electrum" {
            return Err(AppError::new(
                "not_implemented",
                "Native LWK execution requires Electrum",
            ));
        }
        if let Some(prior) = &refresh.prior {
            if prior["schema_version"].as_i64() != Some(STATE_VERSION) && !refresh.force_full {
                return Err(rebuild_required());
            }
        }
        Ok(Self {
            scope,
            identity: Arc::new(()),
            backend: Backend::new(backend, true)?,
            backend_name: backend["name"].as_str().unwrap_or("").to_owned(),
            descriptor: descriptor::wollet_descriptor(plan)?,
            network: network(network_name, policy_asset)?,
            network_name: network_name.to_owned(),
            policy_asset: policy_asset.to_owned(),
            branch_keys,
            gap_limit: plan.gap_limit.max(1),
            refresh,
        })
    }

    /// `_lwk_scan_to_index`: keep the explicit horizon ahead of every prior
    /// discovery, from stored coverage and this identity's checkpoint.
    fn scan_to(&self) -> Result<u32> {
        let gap = i128::from(self.gap_limit);
        let mut highest: Vec<i128> = vec![];
        if self.refresh.prior.is_some() && self.prior_compatible() {
            highest.extend(
                self.refresh
                    .prior_coverage
                    .iter()
                    .filter_map(|p| p["highest_used"].as_i64())
                    .filter(|v| *v >= 0)
                    .map(i128::from),
            );
        }
        if let Some(values) = self.refresh.checkpoint["highest_used"].as_object() {
            for value in values.values() {
                if let Some(parsed) =
                    crate::scan::integer(value).and_then(|n| i128::try_from(n).ok())
                {
                    if parsed >= 0 {
                        highest.push(parsed);
                    }
                }
            }
        }
        let target = highest
            .into_iter()
            .map(|v| v + gap)
            .fold(gap - 1, i128::max);
        u32::try_from(target).map_err(|_| {
            self.sync_failed("The LWK scan horizon exceeds the supported derivation range")
        })
    }

    /// A forced rebuild discards an incompatible payload in memory only.
    fn prior_compatible(&self) -> bool {
        self.refresh
            .prior
            .as_ref()
            .is_some_and(|p| p["schema_version"].as_i64() == Some(STATE_VERSION))
    }

    /// Retractions compare against the stored payload, or for a forced
    /// rebuild without one, against this identity's checkpoint.
    fn previous_txids(&self) -> BTreeSet<String> {
        let source = match (&self.refresh.prior, self.prior_compatible()) {
            (Some(prior), true) => &prior["canonical_txids"],
            _ if self.refresh.force_full => &self.refresh.checkpoint["canonical_txids"],
            _ => return BTreeSet::new(),
        };
        source
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => kassiber_envelope::py_str(
                    &kassiber_pytext::loads(&other.to_string())
                        .unwrap_or(kassiber_pyjson::Value::Null),
                ),
            })
            .collect()
    }

    fn sync_failed(&self, error: &str) -> AppError {
        AppError::new(
            "backend_sync_failed",
            format!(
                "LWK Liquid observation failed for backend '{}'",
                self.backend_name
            ),
        )
        .with_hint("Check the backend connection and retry; Kassiber did not apply partial observer state.")
        .with_details(dict([
            ("dependency", s("lwk")),
            ("error", s(redaction::redact_operational_text(&redaction::redact_secret_text(error)))),
        ]))
        .retryable(true)
    }

    /// Kassiber's own fences pass through; transport and dependency failures
    /// become Python's redacted `backend_sync_failed`.
    fn transport_failed(&self, error: AppError) -> AppError {
        if matches!(
            error.code.as_str(),
            "cancelled" | "interaction_required" | "network_egress_disabled" | "stale_context"
        ) {
            error
        } else {
            self.sync_failed(&error.message)
        }
    }

    pub fn run(
        &self,
        gate: &Gate,
        scope: &Scope,
        consent: Option<&ExecutionConsent>,
        deadline: Instant,
        cancel: &dyn Control,
    ) -> Result<Prepared> {
        cancel.check(deadline)?;
        if scope != &self.scope {
            return Err(AppError::new(
                "stale_context",
                "LWK execution scope changed",
            ));
        }
        let proof = consent
            .filter(|p| Arc::ptr_eq(&p.identity, &self.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "LWK execution requires consent for this exact action",
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
                "LWK execution consent was already used",
            ));
        }
        // Restore before connecting, as Python constructs its wollet first:
        // a corrupt store fails without any request.
        let restored = self.restore()?;
        let client = ElectrumClient::connect(
            self.backend.clone(),
            gate,
            Some(proof.transport.clone()),
            deadline,
        )
        .map_err(|e| self.transport_failed(e))?;
        let prepared = self.execute(
            restored,
            &mut Wire {
                client,
                deadline,
                cancel,
            },
        )?;
        cancel.check(deadline)?;
        gate.authorize(
            self.backend.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| e.app_error())?;
        Ok(prepared)
    }

    /// Restoring replays the stored updates; a corrupt or foreign value is a
    /// dependency failure, as when the Python binding constructs its wollet.
    fn restore(&self) -> Result<(Wollet, Arc<Buffered>)> {
        let store = Arc::new(Buffered(Mutex::new(self.refresh.values.clone())));
        let wollet = WolletBuilder::new(self.network, self.descriptor.clone())
            .with_updates_store(store.clone())
            .build()
            .map_err(|e| self.sync_failed(&e.to_string()))?;
        Ok((wollet, store))
    }

    fn execute(&self, restored: (Wollet, Arc<Buffered>), rpc: &mut impl Rpc) -> Result<Prepared> {
        let (mut wollet, store) = restored;
        let mut backend = backend::Backend::new(rpc);
        let fail = |backend: &backend::Backend<'_, _>, error: lwk_wollet::Error| match backend
            .take_failure()
        {
            Some(transport) => self.transport_failed(transport),
            None => self.sync_failed(&error.to_string()),
        };
        // LWK's Electrum client subscribes to headers when it connects.
        backend.subscribe().map_err(|e| fail(&backend, e))?;
        if self.refresh.force_full {
            if let Some(persisted) = self.refresh.checkpoint["tip_height"].as_i64() {
                // Rebuilding the opaque wollet must not let a lagging backend
                // retract facts newer than its current chain view.
                let remote = backend.tip().map_err(|e| fail(&backend, e))?.height;
                if i64::from(remote) < persisted {
                    return Err(AppError::new(
                        "backend_tip_behind",
                        "The selected Liquid backend is behind the persisted wallet state",
                    )
                    .with_hint("Wait for the backend to catch up or select a fully synchronized backend, then retry.")
                    .with_details(dict([
                        ("observer", s("lwk")),
                        ("backend_height", kassiber_pyjson::Value::from(remote)),
                        ("persisted_height", kassiber_pyjson::Value::from(persisted)),
                    ]))
                    .retryable(true));
                }
            }
        }
        let scan_to = self.scan_to()?;
        let update = backend
            .full_scan_to_index(&wollet, scan_to)
            .map_err(|e| fail(&backend, e))?;
        if let Some(update) = update {
            wollet
                .apply_update(update)
                .map_err(|e| self.sync_failed(&e.to_string()))?;
        }
        let tip = backend.tip().map_err(|e| fail(&backend, e))?;
        let source = facts::Source {
            network: &self.network_name,
            policy_asset: &self.policy_asset,
            backend_name: &self.backend_name,
            branch_keys: &self.branch_keys,
        };
        let update = facts::facts(&source, &wollet, &tip, &self.previous_txids(), scan_to)?;
        drop(wollet);
        let values = store
            .0
            .lock()
            .map_err(|_| self.sync_failed("store poisoned"))?
            .clone();
        Ok(Prepared { update, values })
    }
}

/// Fixture/test entry point over an in-memory RPC; no transport or consent.
#[cfg(any(test, feature = "test-harness"))]
pub fn execute_offline(action: &Execution, rpc: &mut impl Rpc) -> Result<Prepared> {
    action.execute(action.restore()?, rpc)
}

/// Canonical, order-independent projection of opaque values for oracles:
/// LWK serializes hash-map iteration order, so bytes differ between runs.
#[cfg(any(test, feature = "test-harness"))]
pub fn canonical_values(values: &BTreeMap<String, Vec<u8>>) -> Result<Value> {
    use lwk_wollet::elements::{encode::serialize, hex::ToHex};
    let mut out = serde_json::Map::new();
    for (key, bytes) in values {
        let update = lwk_wollet::Update::deserialize(bytes).map_err(|_| rebuild_required())?;
        let mut txs: Vec<String> = update
            .new_txs
            .txs
            .iter()
            .map(|(id, tx)| format!("{id}:{}", serialize(tx).to_hex()))
            .collect();
        txs.sort();
        let mut unblinds: Vec<String> = update
            .new_txs
            .unblinds
            .iter()
            .map(|(point, secrets)| format!("{point}:{}:{}", secrets.asset, secrets.value))
            .collect();
        unblinds.sort();
        let mut heights: Vec<String> = update
            .txid_height_new
            .iter()
            .map(|(id, h)| format!("{id}:{h:?}"))
            .collect();
        heights.sort();
        let mut deleted: Vec<String> = update
            .txid_height_delete
            .iter()
            .map(|id| id.to_string())
            .collect();
        deleted.sort();
        let mut timestamps: Vec<String> = update
            .timestamps
            .iter()
            .map(|(h, t)| format!("{h}:{t}"))
            .collect();
        timestamps.sort();
        let mut scripts: Vec<String> = update
            .scripts_with_blinding_pubkey
            .iter()
            .map(|(chain, child, script, key)| {
                format!(
                    "{chain:?}:{child}:{}:{}",
                    script.as_bytes().to_hex(),
                    key.map(|k| k.to_string()).unwrap_or_default()
                )
            })
            .collect();
        scripts.sort();
        out.insert(
            key.clone(),
            serde_json::json!({"version": update.version, "wollet_status": update.wollet_status.to_string(),
                "txs": txs, "unblinds": unblinds, "txid_height_new": heights,
                "txid_height_delete": deleted, "timestamps": timestamps, "scripts": scripts,
                "tip": serialize(&update.tip).to_hex(),
                "last_unused": [update.last_unused.external, update.last_unused.internal]}),
        );
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests;
