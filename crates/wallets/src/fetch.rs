//! Native preparation for the supported single-wallet Bitcoin publication
//! slice. Construction reads a scoped snapshot without transport or writes;
//! running requires consent for this exact immutable action. Not registered
//! as a CLI or daemon service until their acceptance proofs are complete.
use crate::{
    acquisition::{Acquisition, AcquisitionConsent},
    bdk::{self, Execution, ExecutionConsent, Refresh},
    descriptors::{self, Plan},
    observer_state::{self, Identity},
    publication::{
        row::{field, first, or_text},
        wallet::{fallback, network_scope, require_supported},
    },
    scan::{Cancellation, CompatibilityDiscovery, Control, DiscoveryConsent, Scope},
};
use kassiber_chain::Backend;
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

type Result<T> = std::result::Result<T, AppError>;

struct ProgressControl<'a> {
    control: &'a dyn Control,
    progress: RefCell<&'a mut dyn FnMut(Value) -> Result<()>>,
}

impl Control for ProgressControl<'_> {
    fn check(&self, deadline: Instant) -> Result<()> {
        self.control.check(deadline)
    }
    fn cancellation(&self) -> &Cancellation {
        self.control.cancellation()
    }
    fn progress(&self, event: Value) -> Result<()> {
        (self.progress.borrow_mut())(event)
    }
}

struct Observer {
    identity: Value,
    action: Execution,
    branches: Vec<bdk::Branch>,
    prior: Option<Value>,
    current: Option<BTreeSet<String>>,
    epoch: Option<i64>,
}

#[cfg(test)]
mod tests;

pub struct Fetch {
    identity: Arc<()>,
    scope: Scope,
    captured_config: String,
    backend: Value,
    transport: Backend,
    plan: Plan,
    checkpoint: Value,
    environment: Value,
    force_full: bool,
    version: String,
    observers: Vec<Observer>,
    compatibility: Option<CompatibilityDiscovery>,
    target_count: usize,
}

/// Revoking this grant also revokes every protocol stage of the fetch.
pub struct FetchConsent {
    identity: Arc<()>,
    transport: Consent,
    started: AtomicBool,
}
impl FetchConsent {
    /// Only an explicit sync action may call this, never a saved configuration.
    pub fn explicit_trigger(action: &Fetch) -> Self {
        Self {
            identity: action.identity.clone(),
            transport: Consent::explicit_trigger(action.transport.request()),
            started: AtomicBool::new(false),
        }
    }
    /// A feature the user enabled for this exact kind of action (report-read
    /// refresh), re-checked against the book's policy before each fetch.
    pub fn narrowly_scoped(action: &Fetch) -> Self {
        Self {
            identity: action.identity.clone(),
            transport: Consent::narrowly_scoped(action.transport.request()),
            started: AtomicBool::new(false),
        }
    }
    pub fn revoke(&self) {
        self.transport.revoke();
    }
}

fn encode(value: &kassiber_pyjson::Value) -> Result<Value> {
    let text =
        kassiber_pyjson::dumps(value, &Default::default()).map_err(|_| fallback("invalid_json"))?;
    serde_json::from_str(&text).map_err(|_| fallback("invalid_json"))
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn identity(value: &Value) -> Identity<'_> {
    Identity {
        id: text(&value["id"]),
        observer_kind: "bdk",
        chain: "bitcoin",
        network: text(&value["network"]),
        logical_wallet_id: text(&value["logical_wallet_id"]),
        source_wallet_id: text(&value["source_wallet_id"]),
        source_key: text(&value["source_key"]),
    }
}

impl Fetch {
    /// Capture local preflight, observer epochs, reuse evidence and freshness
    /// together, before the caller can create a network grant. The backend is
    /// selected by the caller; no alternate endpoint is probed on failure.
    pub fn new(
        conn: &Connection,
        profile_id: &str,
        wallet_id: &str,
        backend: &Value,
        force_full: bool,
        version: &str,
    ) -> Result<Self> {
        if !conn.is_autocommit() {
            return Err(AppError::new(
                "observer_prepare_in_transaction",
                "Observer network preparation must finish before the wallet transaction begins",
            ));
        }
        let wallet = first(
            conn,
            "SELECT * FROM wallets WHERE id=? AND profile_id=?",
            [wallet_id, profile_id],
        )?
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                "Sync wallet does not belong to the selected book",
            )
        })?;
        require_supported(conn, profile_id, &wallet)?;
        let captured_config = or_text(field(&wallet, "config_json"));
        let kind = or_text(field(&wallet, "kind"));
        let (routed, _) = network_scope(conn, profile_id, &kind, &captured_config)?;
        let (routed, environment) = network_scope(conn, profile_id, &kind, &routed)?;
        let environment = encode(&environment)?;
        let config: Value = serde_json::from_str(&routed).map_err(|_| fallback("wallet_config"))?;
        if config.get("samourai").is_some() {
            return Err(fallback("wallet_route"));
        }
        if !matches!(text(&backend["kind"]), "electrum" | "esplora") {
            return Err(fallback("backend_kind"));
        }
        let requested = match &config["backend"] {
            Value::Null => "",
            Value::String(name) => name.trim(),
            _ => return Err(fallback("backend_selection")),
        };
        if (!requested.is_empty() && requested.to_lowercase() != text(&backend["name"]))
            || text(&backend["name"]).is_empty()
        {
            return Err(fallback("backend_selection"));
        }
        // Preserve the ordinary BDK versus compatibility source boundary.
        // Additional compatibility reasons need their own parity witnesses.
        for key in ["certificate", "tor_proxy", "proxy"] {
            if backend.get(key).is_some_and(descriptors::truthy)
                || backend["config"].get(key).is_some_and(descriptors::truthy)
            {
                return Err(fallback("compatibility_transport"));
            }
        }
        if backend.get("config_json").is_some() || text(&backend["url"]).contains(".onion") {
            return Err(fallback("compatibility_transport"));
        }
        if backend["kind"] == "esplora"
            && ["insecure", "token", "auth_header", "timeout"]
                .iter()
                .any(|k| {
                    backend.get(k).is_some_and(descriptors::truthy)
                        || backend["config"].get(k).is_some_and(descriptors::truthy)
                })
        {
            return Err(fallback("compatibility_transport"));
        }
        if backend["kind"] == "esplora"
            && text(&backend["url"])
                .to_ascii_lowercase()
                .starts_with("https:")
            && std::env::var_os("SSL_CERT_FILE").is_some_and(|v| !v.is_empty())
        {
            return Err(fallback("compatibility_transport"));
        }
        let plan = descriptors::load(&config)
            .map_err(descriptors::Error::into_app)?
            .ok_or_else(|| fallback("descriptor"))?;
        let checkpoint: Option<String> = conn.query_row("SELECT checkpoint_json FROM freshness_source_states WHERE profile_id=? AND source_type='onchain_wallet' AND source_key=?",[profile_id,&format!("onchain_wallet:{wallet_id}")],|r|r.get(0)).optional().map_err(crate::database_error)?;
        let mut checkpoint = checkpoint
            .and_then(|v| serde_json::from_str::<Value>(&v).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if force_full {
            checkpoint = crate::acquisition::full_scan_checkpoint(&checkpoint);
        }
        let scope = Scope {
            database_instance: conn.path().unwrap_or("").into(),
            profile_id: profile_id.into(),
            wallet_id: wallet_id.into(),
            environment_id: environment.to_string(),
            source_revision: captured_config.clone(),
        };
        // This inert constructor validates backend chain/network/instance too.
        let discovery =
            CompatibilityDiscovery::new(scope.clone(), &config, backend, &checkpoint, version)?;
        let transport = Backend::new(backend, backend["kind"] == "electrum")?;
        let mut observers = vec![];
        let mut target_count = 0;
        for branch in &plan.branches {
            let observed = checkpoint["highest_used"][branch.index.to_string()].as_u64();
            let end = observed
                .map_or(Some(plan.gap_limit), |i| {
                    u32::try_from(i)
                        .ok()
                        .and_then(|i| i.checked_add(plan.gap_limit)?.checked_add(1))
                })
                .ok_or_else(|| fallback("scan_horizon"))?
                .max(plan.gap_limit);
            target_count += plan
                .derive(Some(branch.index), 0, end)
                .map_err(descriptors::Error::into_app)?
                .len();
        }
        if backend["kind"] == "electrum" {
            let keys: BTreeSet<String> = if config["xpub"].is_string() {
                config["script_types"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| format!("xpub:{}", text(v).trim().to_lowercase()))
                    .collect()
            } else {
                BTreeSet::from(["descriptor:default".into()])
            };
            let source_count = keys.len();
            for source_key in keys {
                let selected: Vec<_> = plan
                    .branches
                    .iter()
                    .filter(|b| {
                        !source_key.starts_with("xpub:")
                            || b.label
                                .to_lowercase()
                                .starts_with(&format!("{} ", &source_key[5..]))
                    })
                    .collect();
                if selected.is_empty() || selected.len() > 2 {
                    return Err(fallback("observer_branches"));
                }
                let branches: Vec<_> = selected
                    .iter()
                    .enumerate()
                    .map(|(position, b)| {
                        let internal = b.label.to_lowercase().contains("change") || position == 1;
                        bdk::Branch {
                            key: if internal { "change" } else { "receive" }.into(),
                            index: b.index,
                            label: b.label.clone(),
                            internal,
                        }
                    })
                    .collect();
                if branches.iter().filter(|b| !b.internal).count() != 1
                    || branches.iter().filter(|b| b.internal).count() > 1
                {
                    return Err(fallback("observer_branches"));
                }
                let id = observer_state::instance_id(wallet_id, wallet_id, &source_key);
                let identity_value = json!({"id":id,"workspace_id":encode(field(&wallet,"workspace_id"))?,"profile_id":profile_id,
                    "logical_wallet_id":wallet_id,"source_wallet_id":wallet_id,"source_key":source_key,"observer_kind":"bdk",
                    "chain":"bitcoin","network":plan.network,"branch_keys":branches.iter().map(|b|b.key.clone()).collect::<Vec<_>>()});
                let loaded =
                    observer_state::load_state(conn, &identity(&identity_value), force_full)?;
                let cp = observer_state::identity_checkpoint(&checkpoint, &id);
                let canonical = loaded
                    .prior
                    .as_ref()
                    .map(|v| &v["canonical_txids"])
                    .unwrap_or(&cp["canonical_txids"])
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                let prior = loaded.bdk_changeset(force_full)?;
                let action = Execution::new_refresh(
                    scope.clone(),
                    backend,
                    &selected[0].selected_descriptor(),
                    selected.get(1).map(|b| b.selected_descriptor()).as_deref(),
                    &plan.network,
                    plan.gap_limit,
                    prior.as_ref(),
                    Refresh {
                        force_full,
                        canonical_txids: canonical,
                        checkpoint_tip: cp["tip_height"]
                            .as_u64()
                            .and_then(|h| u32::try_from(h).ok()),
                    },
                )?;
                let current = if source_count > 1 {
                    None
                } else {
                    Some(observer_state::current_observed_txids(
                        conn, wallet_id, &id,
                    )?)
                };
                observers.push(Observer {
                    identity: identity_value,
                    action,
                    branches,
                    prior: loaded.prior,
                    current,
                    epoch: loaded.expected_epoch,
                });
            }
            if observers.is_empty() {
                return Err(fallback("observer_identity"));
            }
        }
        Ok(Self {
            identity: Arc::new(()),
            scope,
            captured_config,
            backend: backend.clone(),
            transport,
            plan,
            checkpoint,
            environment,
            force_full,
            version: version.into(),
            observers,
            compatibility: (backend["kind"] == "esplora").then_some(discovery),
            target_count,
        })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn captured_config(&self) -> &str {
        &self.captured_config
    }

    fn permission_error(&self, error: kassiber_net::Error) -> AppError {
        let mut envelope = error.app_error();
        if self.compatibility.is_none()
            && matches!(
                error,
                kassiber_net::Error::NoEgress
                    | kassiber_net::Error::Offline
                    | kassiber_net::Error::OfflineUnavailable
            )
        {
            envelope.message = envelope.message.replacen(
                "Outbound requests are",
                "Outbound chain observation is",
                1,
            );
        }
        envelope
    }

    pub(crate) fn revalidate(&self, conn: &Connection) -> Result<()> {
        let current = Self::new(
            conn,
            &self.scope.profile_id,
            &self.scope.wallet_id,
            &self.backend,
            self.force_full,
            &self.version,
        )?;
        let same = current.scope == self.scope
            && current.checkpoint == self.checkpoint
            && current.observers.len() == self.observers.len()
            && current.observers.iter().zip(&self.observers).all(|(a, b)| {
                a.identity == b.identity
                    && a.epoch == b.epoch
                    && a.prior == b.prior
                    && a.current == b.current
            });
        if !same {
            return Err(AppError::new(
                "stale_context",
                "Wallet fetch inputs changed",
            ));
        }
        Ok(())
    }

    pub(crate) fn check_permission(
        &self,
        gate: &Gate,
        consent: Option<&FetchConsent>,
    ) -> Result<()> {
        let proof = consent
            .filter(|grant| Arc::ptr_eq(&self.identity, &grant.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "Wallet fetch requires consent for this exact action",
                )
            })?;
        gate.authorize(
            self.transport.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| self.permission_error(e))?;
        Ok(())
    }

    pub fn run(
        &self,
        gate: &Gate,
        scope: &Scope,
        consent: Option<&FetchConsent>,
        deadline: Instant,
        cancel: &Cancellation,
        start_time: u64,
    ) -> Result<Value> {
        self.run_with_progress(
            gate,
            scope,
            consent,
            deadline,
            cancel,
            start_time,
            &mut |_| Ok(()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_with_progress(
        &self,
        gate: &Gate,
        scope: &Scope,
        consent: Option<&FetchConsent>,
        deadline: Instant,
        control: &dyn Control,
        start_time: u64,
        progress: &mut dyn FnMut(Value) -> Result<()>,
    ) -> Result<Value> {
        let cancel = &ProgressControl {
            control,
            progress: RefCell::new(progress),
        };
        cancel.check(deadline)?;
        if scope != &self.scope {
            return Err(AppError::new("stale_context", "Wallet fetch scope changed"));
        }
        let proof = consent
            .filter(|p| Arc::ptr_eq(&p.identity, &self.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "Wallet fetch requires consent for this exact action",
                )
            })?;
        gate.authorize(
            self.transport.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| self.permission_error(e))?;
        if proof.started.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Wallet fetch consent was already used",
            ));
        }
        let mut updates = vec![];
        let mut records = vec![];
        let mut target_count = self.target_count;
        let meta = if let Some(discovery) = &self.compatibility {
            let grant = DiscoveryConsent::for_fetch(discovery, proof.transport.clone());
            let discovered = discovery.run(gate, scope, Some(&grant), deadline, cancel, |_| {})?;
            let targets = discovered["targets"]
                .as_array()
                .ok_or_else(|| fallback("discovery_targets"))?;
            target_count = targets.len();
            let action = Acquisition::new_cached(
                scope.clone(),
                &self.backend,
                targets,
                &self.version,
                &self.checkpoint,
                &discovered["history_cache"],
                &self.plan.network,
            )?;
            let grant = AcquisitionConsent::for_fetch(&action, proof.transport.clone());
            let tracked = targets
                .iter()
                .filter_map(|t| t["script_pubkey"].as_str().map(str::to_owned))
                .collect();
            let acquired = action.run_with_history(gate, scope, Some(&grant), deadline, cancel, |_| {}, &mut |history| {
                let mut txs: Vec<_> = history["transactions"].as_object().into_iter().flatten()
                    .map(|(_, v)| v).collect();
                txs.sort_by_key(|tx| (tx["status"]["block_time"].as_i64().unwrap_or(0), text(&tx["txid"])));
                let total = txs.len();
                for (index, tx) in txs.into_iter().enumerate() {
                    cancel.check(deadline)?;
                    if let Some(record) = crate::bitcoin_records::from_esplora(tx, &tracked, text(&self.backend["name"]))? {
                        records.push(record);
                    }
                    if (index + 1).is_multiple_of(100) || index + 1 == total {
                        cancel.progress(json!({"phase":"decode_enrich", "transactions_seen":index+1,
                            "transactions_total":total,"records":records.len(),"processed":index+1,"total":total}))?;
                    }
                }
                Ok(())
            })?;
            esplora_metadata(&acquired, &self.plan.network)?
        } else {
            for observer in &self.observers {
                cancel.check(deadline)?;
                cancel.progress(
                    json!({"phase":"backend_fetch","observer":"bdk","status":"scanning"}),
                )?;
                let grant = ExecutionConsent::for_fetch(&observer.action, proof.transport.clone());
                let prepared =
                    observer
                        .action
                        .run(gate, scope, Some(&grant), deadline, cancel, start_time)?;
                let update = prepared.update(
                    &observer.branches,
                    &self.plan.network,
                    text(&self.backend["name"]),
                    &format!("1:{}", self.version),
                    observer.prior.as_ref(),
                    observer.current.clone(),
                )?;
                cancel.progress(json!({"phase":"decode_enrich","observer":"bdk",
                    "transactions_seen":update["facts"]["transaction_records"].as_array().map_or(0,Vec::len),
                    "utxos_seen":update["facts"]["outputs"].as_array().map_or(0,Vec::len)}))?;
                updates.push(json!({"identity":observer.identity,"expected_epoch":observer.epoch,"update":update}));
            }
            json!({"observer_route":"bdk"})
        };
        cancel.check(deadline)?;
        gate.authorize(
            self.transport.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| self.permission_error(e))?;
        Ok(
            json!({"backend":self.backend,"kind":self.backend["kind"],"force_full":self.force_full,"authoritative_chain_observer":true,
            "book_environment_id":self.environment,"skip_outcome":null,"sync_state":{"chain":"bitcoin","network":self.plan.network,
                "target_count":target_count,"gap_limit":self.plan.gap_limit,"descriptor_kind":null,
                "policy_asset_id":"","checkpoint":self.checkpoint},"normalized_records":records,"adapter_meta":meta,"observer_updates":updates,"elapsed_ms":0}),
        )
    }
}

fn esplora_metadata(acquired: &Value, network: &str) -> Result<Value> {
    let mut meta = json!({"observer_route":"compatibility","observer_compatibility_reason":"http_route_policy",
            "freshness_checkpoint":acquired["freshness_checkpoint"],"scripts_changed":acquired["scripts_changed"],"scripts_unchanged":acquired["scripts_unchanged"],
            "known_txids":acquired["transactions"].as_object().map_or(0,|t|t.len()),"esplora_history_complete":acquired["esplora_history_complete"],
            "observer_retracted_external_ids":acquired["observer_retracted_external_ids"]});
    if acquired["utxos_skipped_unchanged"] == true {
        meta["utxos_skipped_unchanged"] = json!(true);
        return Ok(meta);
    }
    let tip = acquired["tip_height"].as_i64();
    let mut outputs = vec![];
    let scripts = acquired["scripts"]
        .as_array()
        .ok_or_else(|| fallback("output_inventory"))?;
    for script in scripts {
        let target = &script["target"];
        let utxos = script["utxos"]
            .as_array()
            .ok_or_else(|| fallback("output_inventory"))?;
        for raw in utxos {
            let status = &raw["status"];
            let height = status["block_height"].as_i64().filter(|h| *h > 0);
            let confirmations = height
                .zip(tip)
                .filter(|(h, t)| *t >= *h)
                .map(|(h, t)| t - h + 1);
            let block_time = status["block_time"]
                .as_i64()
                .and_then(kassiber_chain::format_unix_time);
            let label = text(&target["branch_label"]);
            let index = target["address_index"].as_u64();
            let amount = raw["value"]
                .as_u64()
                .ok_or_else(|| AppError::new("protocol_error", "Invalid Bitcoin output amount"))?;
            outputs.push(json!({"txid":raw["txid"],"vout":raw["vout"],"asset":"BTC","amount_sats":amount,
                    "confirmation_status":if status["confirmed"]==true{"confirmed"}else{"mempool"},"confirmations":confirmations,"block_height":height,"block_time":block_time,
                    "chain":"bitcoin","network":network,"address":target["address"],"script_pubkey":target["script_pubkey"],
                    "address_label":index.map_or_else(||label.into(),|i|format!("{label} #{i}")),"branch_label":label,"branch_index":target["branch_index"],"address_index":target["address_index"],
                    "raw":{"source":"esplora_scripthash_utxo","confirmed":status["confirmed"]==true}}));
        }
    }
    meta["utxos"] = json!(outputs);
    Ok(meta)
}
