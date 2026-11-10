//! Native preparation for the supported single-wallet Bitcoin publication
//! slice. Construction reads a scoped snapshot without transport or writes;
//! running requires consent for this exact immutable action. Not registered
//! as a CLI or daemon service until their acceptance proofs are complete.
use crate::{
    acquisition::{Acquisition, AcquisitionConsent},
    bdk::{self, Execution, ExecutionConsent, Refresh},
    descriptors::{self, Plan},
    lwk,
    observer_state::{self, Identity},
    publication::{
        row::{field, first, or_text},
        wallet::{fallback, network_scope, require_supported},
    },
    scan::{Cancellation, CompatibilityDiscovery, Control, DiscoveryConsent, Scope},
    source_overlap::{self, Overlap},
};
use kassiber_chain::Backend;
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
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

enum Action {
    Bdk {
        action: Execution,
        branches: Vec<bdk::Branch>,
    },
    Lwk(lwk::Execution),
}

struct Observer {
    identity: Value,
    action: Action,
    prior: Option<Value>,
    current: Option<BTreeSet<String>>,
    epoch: Option<i64>,
    /// LWK's opaque store as captured with the state; a change is stale input.
    values: Option<BTreeMap<String, Vec<u8>>>,
}

mod core;
mod electrum;
mod liquid;
mod silent;
#[cfg(test)]
mod tests;

pub struct Fetch {
    identity: Arc<()>,
    scope: Scope,
    captured_config: String,
    /// The backend as resolved, which the fetch reports and a repair,
    /// rescan or revalidation prepares from again.
    given: Value,
    /// The same backend with its kind normalized (`normalize_backend_kind`):
    /// what the observers route by.
    backend: Value,
    /// `None` for a silent-payment scan that makes no request.
    transport: Option<Backend>,
    /// `None` for an address-list or silent-payment wallet.
    plan: Option<Plan>,
    /// A silent-payment wallet's scan route.
    silent: Option<silent::Route>,
    chain: String,
    network: String,
    checkpoint: Value,
    environment: Value,
    force_full: bool,
    version: String,
    observers: Vec<Observer>,
    /// Online discovery for a Bitcoin descriptor: the compatibility route's,
    /// or the one the dependency route falls back to.
    compatibility: Option<CompatibilityDiscovery>,
    /// Python's Bitcoin Core adapter (`bitcoinrpc_sync_adapter`).
    core: bool,
    /// `_stored_wallet_chain_history`: Electrum graphs the wallet already
    /// stored, keyed by txid, which the compatibility Electrum route reuses.
    history: Value,
    /// The offline preflight targets the dependency route reports.
    targets: Vec<Value>,
    /// Ownership evidence captured with the snapshot. Not part of
    /// revalidation: publication re-checks it against the current book.
    overlap: Overlap,
    /// Every preflight target belongs to a better canonical wallet: the
    /// original count, and no request is made.
    skip: Option<usize>,
    label: String,
    /// `bdk_compatibility_reason`, which selects Python's compatibility
    /// observer; empty for the dependency (BDK or LWK) route.
    reason: &'static str,
    /// The widened gap limit of a negative-balance repair fetch.
    gap: Option<u32>,
    policy_asset: String,
}

/// Revoking this grant also revokes every protocol stage of the fetch.
pub struct FetchConsent {
    identity: Arc<()>,
    /// `None` for a fetch without a transport.
    transport: Option<Consent>,
    started: AtomicBool,
}
impl FetchConsent {
    /// Only an explicit sync action may call this, never a saved configuration.
    pub fn explicit_trigger(action: &Fetch) -> Self {
        Self {
            identity: action.identity.clone(),
            transport: action
                .transport
                .as_ref()
                .map(|t| Consent::explicit_trigger(t.request())),
            started: AtomicBool::new(false),
        }
    }
    /// A feature the user enabled for this exact kind of action (report-read
    /// refresh), re-checked against the book's policy before each fetch.
    pub fn narrowly_scoped(action: &Fetch) -> Self {
        Self {
            identity: action.identity.clone(),
            transport: action
                .transport
                .as_ref()
                .map(|t| Consent::narrowly_scoped(t.request())),
            started: AtomicBool::new(false),
        }
    }
    /// The widened repair fetch Python runs within the same consented sync;
    /// revoking the original grant revokes it too.
    pub(crate) fn for_repair(repair: &Fetch, original: &FetchConsent) -> Self {
        Self {
            identity: repair.identity.clone(),
            transport: original.transport.clone(),
            started: AtomicBool::new(false),
        }
    }
    pub fn revoke(&self) {
        if let Some(transport) = &self.transport {
            transport.revoke();
        }
    }
    /// The transport grant of a fetch that has a transport.
    fn transport(&self) -> Result<Consent> {
        self.transport
            .clone()
            .ok_or_else(|| AppError::new("internal_error", "Wallet fetch has no transport grant"))
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
/// `normalize_backend_kind`: Python routes `mempool` and `liquid-esplora`
/// as Esplora and the Core spellings as `bitcoinrpc`.
fn routed(backend: &Value) -> Value {
    let raw = kassiber_pytext::lower(kassiber_pytext::strip(&descriptors::text(&backend["kind"])));
    let kind = match raw.as_str() {
        "bitcoin-core" | "bitcoincore" | "core" => "bitcoinrpc",
        "core-ln" | "core-lightning" => "coreln",
        "liquid-esplora" | "mempool" => "esplora",
        other => other,
    };
    let mut routed = backend.clone();
    routed["kind"] = json!(kind);
    routed
}
fn identity(value: &Value) -> Identity<'_> {
    Identity {
        id: text(&value["id"]),
        observer_kind: text(&value["observer_kind"]),
        chain: text(&value["chain"]),
        network: text(&value["network"]),
        logical_wallet_id: text(&value["logical_wallet_id"]),
        source_wallet_id: text(&value["source_wallet_id"]),
        source_key: text(&value["source_key"]),
    }
}

/// One BDK observer instance of `identities_for_wallet`; `branch_keys` is
/// set where Python fixes them apart from the descriptor plan (Samourai).
struct BdkIdentity {
    logical_wallet_id: String,
    source_key: String,
    branch_keys: Option<Vec<String>>,
}

/// `identities_for_wallet`'s source keys: a Samourai child observes under its
/// parent's logical wallet, a parent observes nothing, an xpub one instance
/// per script type, a descriptor one default instance.
fn bdk_identities(wallet_id: &str, config: &Value) -> Vec<BdkIdentity> {
    use crate::config::optional_text;
    let own = |source_key: String| BdkIdentity {
        logical_wallet_id: wallet_id.into(),
        source_key,
        branch_keys: None,
    };
    if let Some(samourai) = config.get("samourai").filter(|v| v.is_object()) {
        match optional_text(samourai.get("role")).as_deref() {
            Some("parent") => return vec![],
            Some("child") => {
                let part = |key: &str, default: &str| {
                    optional_text(samourai.get(key)).unwrap_or_else(|| default.into())
                };
                let mut branch_keys = vec!["receive".to_owned()];
                if config
                    .get("change_descriptor")
                    .is_some_and(descriptors::truthy)
                {
                    branch_keys.push("change".into());
                }
                return vec![BdkIdentity {
                    logical_wallet_id: part("parent_wallet_id", wallet_id),
                    source_key: format!(
                        "samourai:{}:{}:{}",
                        part("section", "source"),
                        part("script_type", "descriptor"),
                        part("root_path", "account"),
                    ),
                    branch_keys: Some(branch_keys),
                }];
            }
            _ => {}
        }
    }
    let script_types: BTreeSet<String> = config
        .get("script_types")
        .filter(|v| descriptors::truthy(v))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| optional_text(Some(v)))
        .map(|v| v.to_lowercase())
        .collect();
    if optional_text(config.get("xpub")).is_some() && !script_types.is_empty() {
        return script_types
            .into_iter()
            .map(|script_type| own(format!("xpub:{script_type}")))
            .collect();
    }
    if optional_text(config.get("descriptor")).is_some() {
        return vec![own("descriptor:default".into())];
    }
    vec![]
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
        Self::prepare(
            conn,
            profile_id,
            wallet_id,
            backend,
            force_full,
            version,
            Preflight::Index,
            Overrides::default(),
        )
    }

    /// `_prepare_negative_balance_repairs`' widened fetch: the same wallet
    /// and backend, forced, with `negative_balance_rescan_gap_limit`. It keeps
    /// the original's stored Electrum graphs, as Python's repair wallet does.
    pub(crate) fn repair(&self, conn: &Connection) -> Result<Self> {
        Self::prepare(
            conn,
            &self.scope.profile_id,
            &self.scope.wallet_id,
            &self.given,
            true,
            &self.version,
            Preflight::Index,
            Overrides {
                gap: self.rescan_gap(),
                history: Some(self.history.clone()),
                ..Overrides::default()
            },
        )
    }

    /// `sync_wallet_from_backend`'s rescan when a deficit shows only once a
    /// compatibility fetch is published: the same wallet, forced, at the
    /// widened gap (an address list keeps its targets), from the published
    /// `checkpoint`. Python prepares and fetches it inside the publication's
    /// savepoint, so this does too; Python's plain wallet row offers no
    /// stored graphs.
    pub(crate) fn rescan(&self, conn: &Connection, checkpoint: &Value) -> Result<Self> {
        Self::prepare(
            conn,
            &self.scope.profile_id,
            &self.scope.wallet_id,
            &self.given,
            true,
            &self.version,
            Preflight::Index,
            Overrides {
                gap: self.rescan_gap(),
                history: Some(json!({})),
                checkpoint: Some(checkpoint.clone()),
                in_savepoint: true,
            },
        )
    }

    /// `negative_balance_rescan_gap_limit`: none without a descriptor plan.
    pub(crate) fn rescan_gap(&self) -> Option<u32> {
        match (&self.plan, &self.silent) {
            (Some(plan), _) => Some(rescan_gap_limit(plan.gap_limit)),
            // A silent-payment plan exists, at gap 0.
            (None, Some(_)) => Some(rescan_gap_limit(0)),
            (None, None) => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare(
        conn: &Connection,
        profile_id: &str,
        wallet_id: &str,
        backend: &Value,
        force_full: bool,
        version: &str,
        preflight: Preflight,
        overrides: Overrides,
    ) -> Result<Self> {
        let Overrides {
            gap,
            history,
            checkpoint: given,
            in_savepoint,
        } = overrides;
        let resolved = backend;
        let routed = routed(resolved);
        let backend = &routed;
        if !in_savepoint && !conn.is_autocommit() {
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
        require_supported(&wallet)?;
        let captured_config = or_text(field(&wallet, "config_json"));
        let kind = or_text(field(&wallet, "kind"));
        let (routed, _) = network_scope(conn, profile_id, &kind, &captured_config)?;
        let (routed, environment) = network_scope(conn, profile_id, &kind, &routed)?;
        let environment = encode(&environment)?;
        let mut config: Value =
            serde_json::from_str(&routed).map_err(|_| fallback("wallet_config"))?;
        if let Some(gap) = gap {
            // `wallet_with_temporary_gap_limit`.
            config["gap_limit"] = json!(gap);
        }
        let mut checkpoint = match given {
            Some(given) => given,
            None => {
                let stored: Option<String> = conn.query_row("SELECT checkpoint_json FROM freshness_source_states WHERE profile_id=? AND source_type='onchain_wallet' AND source_key=?",[profile_id,&format!("onchain_wallet:{wallet_id}")],|r|r.get(0)).optional().map_err(crate::database_error)?;
                stored
                    .and_then(|v| serde_json::from_str::<Value>(&v).ok())
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}))
            }
        };
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
        // `resolve_wallet_sync_targets`: silent-payment material takes its
        // scanner route ahead of any descriptor, on any SP-capable backend.
        if crate::silent_payments::has_material(&config) {
            let requested = config["backend"].as_str().map_or("", str::trim);
            if (!requested.is_empty() && requested.to_lowercase() != text(&backend["name"]))
                || text(&backend["name"]).is_empty()
            {
                return Err(fallback("backend_selection"));
            }
            let label = kassiber_envelope::py_str(field(&wallet, "label"));
            let (route, transport) = silent::Route::prepare(backend, &config, wallet_id, &label)?;
            let overlap = Overlap::capture(conn, profile_id, wallet_id, &label, &kind, &config)?;
            return Ok(Self {
                identity: Arc::new(()),
                scope,
                captured_config,
                given: resolved.clone(),
                backend: backend.clone(),
                transport,
                plan: None,
                chain: route.chain().into(),
                network: route.network().into(),
                targets: vec![route.target()],
                silent: Some(route),
                checkpoint,
                environment,
                force_full,
                version: version.into(),
                observers: vec![],
                compatibility: None,
                core: false,
                history: json!({}),
                overlap,
                skip: None,
                label,
                reason: "silent_payment",
                gap,
                policy_asset: String::new(),
            });
        }
        if !matches!(
            text(&backend["kind"]),
            "electrum" | "esplora" | "bitcoinrpc"
        ) {
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
        if backend.get("config_json").is_some() {
            return Err(fallback("compatibility_transport"));
        }
        // Esplora always takes Python's compatibility observer, as does
        // Electrum over a transport `bdk_electrum` cannot carry; the reason
        // labels the route.
        let electrum = backend["kind"] == "electrum";
        // Bitcoin Core is Python's own adapter, neither BDK nor a
        // compatibility observer.
        let core = backend["kind"] == "bitcoinrpc";
        let reason = if core {
            ""
        } else if electrum {
            electrum_reason(backend)
        } else {
            esplora_reason(backend)?
        };
        let history = match history {
            Some(history) => history,
            None if force_full => json!({}),
            None => stored_history(conn, profile_id, wallet_id, &config)?,
        };
        let plan = descriptors::load(&config).map_err(descriptors::Error::into_app)?;
        let Some(plan) = plan else {
            let label = kassiber_envelope::py_str(field(&wallet, "label"));
            let overlap = Overlap::capture(conn, profile_id, wallet_id, &label, &kind, &config)?;
            return Self::address_list(AddressList {
                config: &config,
                given: resolved,
                backend,
                preflight,
                scope,
                captured_config,
                checkpoint,
                environment,
                force_full,
                version,
                overlap,
                label,
                history,
            });
        };
        let liquid = plan.chain == "liquid";
        let policy_asset = if liquid {
            policy_asset(&config, &plan.network)
        } else {
            String::new()
        };
        // `resolve_wallet_sync_targets`: Liquid's own checks, or the inert
        // discovery constructor, which validates backend chain, network and
        // instance too.
        if liquid {
            liquid_preflight(&config, &plan, backend)?;
            if core {
                return Err(AppError::new(
                    "app_error",
                    "Liquid live refresh currently requires an Esplora-compatible or Electrum backend",
                ));
            }
        }
        let discovery = if core {
            crate::scan::validate_backend(&plan, backend, &config)?;
            None
        } else {
            Some(CompatibilityDiscovery::new(
                scope.clone(),
                &config,
                backend,
                &checkpoint,
                version,
            )?)
        };
        let transport = Backend::new(backend, backend["kind"] == "electrum")?;
        // Core imports its own range (`discover_bitcoinrpc_descriptor_targets`).
        let targets = if core {
            self::core::checkpoint_targets(&plan, &checkpoint)?
        } else {
            offline_targets(&plan, &checkpoint)?
        };
        let label = kassiber_envelope::py_str(field(&wallet, "label"));
        let overlap = Overlap::capture(conn, profile_id, wallet_id, &label, &kind, &config)?;
        // `_apply_source_overlap_preflight` with the operation's index.
        let dropped = match preflight {
            Preflight::Index => overlap.dropped(&targets, &plan.chain, &plan.network, true),
            Preflight::Skipped | Preflight::Prepared(_) => Default::default(),
        };
        let kept = source_overlap::keep(&targets, &dropped);
        if preflight == Preflight::Skipped || (kept.is_empty() && !targets.is_empty()) {
            return Ok(Self {
                identity: Arc::new(()),
                scope,
                captured_config,
                given: resolved.clone(),
                backend: backend.clone(),
                transport: Some(transport),
                silent: None,
                chain: plan.chain.clone(),
                network: plan.network.clone(),
                plan: Some(plan),
                checkpoint,
                environment,
                force_full,
                version: version.into(),
                observers: vec![],
                compatibility: None,
                core,
                history,
                skip: Some(targets.len()),
                targets,
                overlap,
                label,
                reason,
                gap,
                policy_asset,
            });
        }
        if core {
            // Python's Core adapter keeps the trimmed targets it was given.
            return Ok(Self {
                identity: Arc::new(()),
                scope,
                captured_config,
                given: resolved.clone(),
                backend: backend.clone(),
                transport: Some(transport),
                silent: None,
                chain: plan.chain.clone(),
                network: plan.network.clone(),
                plan: Some(plan),
                checkpoint,
                environment,
                force_full,
                version: version.into(),
                observers: vec![],
                compatibility: None,
                core,
                history,
                targets: kept,
                overlap,
                skip: None,
                label,
                reason,
                gap,
                policy_asset,
            });
        }
        // Liquid takes LWK unless `lwk_compatibility_reason` names a route LWK
        // cannot carry; Python then runs its Liquid compatibility observer.
        let mut reason = if liquid {
            lwk::descriptor::compatibility_reason(backend, &plan)?.unwrap_or("")
        } else {
            reason
        };
        // A whole-descriptor scanner (BDK, LWK) cannot leave out a trimmed
        // script, so a trimmed descriptor takes the compatibility observer,
        // which re-filters after online discovery.
        if !dropped.is_empty() && reason.is_empty() {
            reason = "source_overlap_partial_descriptor";
        }
        // Revalidation keeps the route the ownership index chose.
        if let Preflight::Prepared(prepared) = preflight {
            if reason.is_empty() {
                reason = prepared;
            }
        }
        let liquid_observers = if liquid && reason.is_empty() {
            Some(liquid_observers(
                conn,
                &wallet,
                &config,
                &plan,
                backend,
                &checkpoint,
                &scope,
                force_full,
            )?)
        } else {
            None
        };
        let mut observers = vec![];
        if let Some(liquid) = liquid_observers {
            observers = liquid;
        } else if electrum && reason.is_empty() {
            let identities = bdk_identities(wallet_id, &config);
            if identities.is_empty() {
                return Err(AppError::new(
                    "observer_identity_invalid",
                    "The dependency observer route resolved no wallet identities",
                )
                .with_hint("Review the wallet descriptor source; Kassiber will not treat an empty scan as authoritative.")
                .with_details(kassiber_envelope::dict([
                    ("observer", kassiber_envelope::s("bdk")),
                    ("wallet_id", kassiber_envelope::s(wallet_id)),
                ])));
            }
            let source_count = identities.len();
            for BdkIdentity {
                logical_wallet_id,
                source_key,
                branch_keys,
            } in identities
            {
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
                let id = observer_state::instance_id(&logical_wallet_id, wallet_id, &source_key);
                let branch_keys =
                    branch_keys.unwrap_or_else(|| branches.iter().map(|b| b.key.clone()).collect());
                let identity_value = json!({"id":id,"workspace_id":encode(field(&wallet,"workspace_id"))?,"profile_id":profile_id,
                    "logical_wallet_id":logical_wallet_id,"source_wallet_id":wallet_id,"source_key":source_key,"observer_kind":"bdk",
                    "chain":"bitcoin","network":plan.network,"branch_keys":branch_keys});
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
                    action: Action::Bdk { action, branches },
                    prior: loaded.prior,
                    current,
                    epoch: loaded.expected_epoch,
                    values: None,
                });
            }
        }
        Ok(Self {
            identity: Arc::new(()),
            scope,
            captured_config,
            given: resolved.clone(),
            backend: backend.clone(),
            transport: Some(transport),
            silent: None,
            chain: plan.chain.clone(),
            network: plan.network.clone(),
            plan: Some(plan),
            checkpoint,
            environment,
            force_full,
            version: version.into(),
            observers,
            compatibility: discovery,
            core,
            history,
            targets,
            overlap,
            skip: None,
            label,
            reason,
            gap,
            policy_asset,
        })
    }
    /// `_overlap_filtered_skip_outcome`: no target is left to fetch.
    fn skipped(&self, original: usize) -> Value {
        let mut outcome = json!({"wallet":if self.label.is_empty() {"Wallet"} else {&self.label},
            "status":"skipped","reason":"all sync targets are already covered by canonical wallet sources",
            "target_count":0,"filtered_overlap_targets":original,"elapsed_ms":0});
        if self.force_full {
            outcome["force_full"] = json!(true);
        }
        json!({"backend":self.given,"kind":self.backend["kind"],"force_full":self.force_full,
            "authoritative_chain_observer":false,"book_environment_id":self.environment,
            "skip_outcome":outcome,"sync_state":null,"normalized_records":[],"adapter_meta":{},
            "observer_updates":[],"elapsed_ms":0})
    }

    /// `_wrap_sync_progress_for_wallet` around the events Python's
    /// coordinator emits before a silent-payment scan: discovery, then
    /// `backend_fetch` outside the compatibility route.
    pub(crate) fn emit_silent_progress(&self) {
        let Some(route) = &self.silent else {
            return;
        };
        let label = kassiber_envelope::s(self.label.as_str());
        crate::progress::for_wallet(&label, || {
            let phase =
                |name: &str| kassiber_envelope::dict([("phase", kassiber_envelope::s(name))]);
            crate::progress::emit(phase("discovery"));
            if !matches!(route.kind.as_str(), "esplora" | "electrum") {
                crate::progress::emit(phase("backend_fetch"));
            }
        });
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn captured_config(&self) -> &str {
        &self.captured_config
    }

    fn permission_error(&self, error: kassiber_net::Error) -> AppError {
        let mut envelope = error.app_error();
        if self.reason.is_empty()
            && !self.core
            && self.plan.is_some()
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
        let current = Self::prepare(
            conn,
            &self.scope.profile_id,
            &self.scope.wallet_id,
            &self.given,
            self.force_full,
            &self.version,
            if self.skip.is_some() {
                Preflight::Skipped
            } else {
                Preflight::Prepared(self.reason)
            },
            Overrides {
                gap: self.gap,
                history: Some(self.history.clone()),
                ..Overrides::default()
            },
        )?;
        let same = current.scope == self.scope
            && current.checkpoint == self.checkpoint
            && current.skip == self.skip
            && current.observers.len() == self.observers.len()
            && current.observers.iter().zip(&self.observers).all(|(a, b)| {
                a.identity == b.identity
                    && a.epoch == b.epoch
                    && a.prior == b.prior
                    && a.current == b.current
                    && a.values == b.values
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
        if self.skip.is_some() {
            // Python's overlap skip makes no request, so no transport check.
            return Ok(());
        }
        self.authorize(gate, proof)
    }

    /// Authorizes this fetch's transport under its grant. A fetch without
    /// one (a local scanner file) makes no request to authorize.
    fn authorize(&self, gate: &Gate, proof: &FetchConsent) -> Result<()> {
        if let Some(transport) = &self.transport {
            gate.authorize(transport.request().clone(), Some(proof.transport()?))
                .map_err(|e| self.permission_error(e))?;
        }
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
        // `fetch_wallet_backend`'s `started`: Python reports the fetch's own
        // wall time, which publication extends to the whole sync.
        let started = Instant::now();
        let mut fetched = self.run_timed(
            gate, scope, consent, deadline, control, start_time, progress,
        )?;
        let elapsed = json!(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        fetched["elapsed_ms"] = elapsed.clone();
        if fetched["skip_outcome"].is_object() {
            fetched["skip_outcome"]["elapsed_ms"] = elapsed;
        }
        Ok(fetched)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_timed(
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
        // Python's overlap skip makes no request, so no transport check.
        if self.skip.is_none() {
            self.authorize(gate, proof)?;
        }
        if proof.started.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Wallet fetch consent was already used",
            ));
        }
        if let Some(original) = self.skip {
            return Ok(self.skipped(original));
        }
        if let Some(route) = &self.silent {
            let (records, meta, authoritative) = route.run(
                self.transport.as_ref(),
                gate,
                proof.transport.as_ref(),
                &self.version,
                deadline,
                &self.checkpoint,
            )?;
            cancel.check(deadline)?;
            self.authorize(gate, proof)?;
            return Ok(
                json!({"backend":self.given,"kind":route.kind,"force_full":self.force_full,
                "authoritative_chain_observer":authoritative,"book_environment_id":self.environment,
                "skip_outcome":null,"sync_state":silent::sync_state(route, &self.checkpoint),
                "normalized_records":records,"adapter_meta":meta,"observer_updates":[],"elapsed_ms":0}),
            );
        }
        let mut updates = vec![];
        let mut records = vec![];
        let mut reported = self.targets.clone();
        let meta = if self.core {
            let transport = self
                .transport
                .as_ref()
                .ok_or_else(|| AppError::new("internal_error", "Wallet fetch has no transport"))?;
            let consent = proof.transport()?;
            let node = self::core::Node::new(
                &self.backend,
                transport,
                gate,
                &self.version,
                &consent,
                deadline,
                cancel,
            );
            let config: Value = serde_json::from_str(&self.captured_config)
                .map_err(|_| fallback("wallet_config"))?;
            let (found, meta) = self::core::sync(
                &node,
                &self::core::Input {
                    wallet_id: &self.scope.wallet_id,
                    config: &config,
                    plan: self.plan.as_ref(),
                    targets: &self.targets,
                    checkpoint: &self.checkpoint,
                    chain: &self.chain,
                    network: &self.network,
                },
            )?;
            records = found;
            meta
        } else if !self.reason.is_empty() {
            match self.compatible(
                self.reason,
                gate,
                scope,
                proof,
                deadline,
                cancel,
                &mut records,
            )? {
                Compatible::Skipped(skipped) => return Ok(skipped),
                Compatible::Fetched { meta, targets } => {
                    reported = targets;
                    meta
                }
            }
        } else {
            let mut route = "bdk";
            for observer in &self.observers {
                cancel.check(deadline)?;
                match &observer.action {
                    Action::Bdk { action, branches } => {
                        cancel.progress(
                            json!({"phase":"backend_fetch","observer":"bdk","status":"scanning"}),
                        )?;
                        let grant = ExecutionConsent::for_fetch(action, proof.transport()?);
                        let prepared =
                            action.run(gate, scope, Some(&grant), deadline, cancel, start_time)?;
                        let update = prepared.update(
                            branches,
                            &self.network,
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
                    Action::Lwk(action) => {
                        route = "lwk";
                        cancel.progress(
                            json!({"phase":"backend_fetch","observer":"lwk","status":"scanning"}),
                        )?;
                        let grant = lwk::ExecutionConsent::for_fetch(action, proof.transport()?);
                        let prepared = action.run(gate, scope, Some(&grant), deadline, cancel)?;
                        // In-process only: the publication savepoint persists
                        // these bytes; they are never logged or projected.
                        let values: serde_json::Map<String, Value> = prepared
                            .values
                            .iter()
                            .map(|(k, v)| (k.clone(), json!(hex(v))))
                            .collect();
                        updates.push(
                            json!({"identity":observer.identity,"expected_epoch":observer.epoch,
                            "update":prepared.update,"opaque_values":values}),
                        );
                    }
                }
            }
            // `_observer_discovered_targets`: an owned script past the
            // preflight horizon that a better wallet owns discards the
            // prepared updates for Python's compatibility observer.
            let prepared: Vec<&Value> = updates.iter().map(|u| &u["update"]).collect();
            let found = source_overlap::discovered(&prepared);
            let overlapping =
                source_overlap::expanded(&self.targets, &found).is_some_and(|expanded| {
                    !self
                        .overlap
                        .dropped(&expanded, &self.chain, &self.network, false)
                        .is_empty()
                });
            if overlapping {
                // Every dependency route prepares its online discovery;
                // without one the compatibility route would keep the fixed
                // horizon, so refuse instead.
                if self.compatibility.is_none() {
                    return Err(fallback("source_overlap_dependency_discovery"));
                }
                updates.clear();
                let reason = "source_overlap_dependency_discovery";
                match self.compatible(reason, gate, scope, proof, deadline, cancel, &mut records)? {
                    Compatible::Skipped(skipped) => return Ok(skipped),
                    Compatible::Fetched { meta, targets } => {
                        reported = targets;
                        meta
                    }
                }
            } else {
                json!({"observer_route":route})
            }
        };
        let target_count = reported.len();
        cancel.check(deadline)?;
        self.authorize(gate, proof)?;
        Ok(
            json!({"backend":self.given,"kind":self.backend["kind"],"force_full":self.force_full,"authoritative_chain_observer":true,
            "book_environment_id":self.environment,"skip_outcome":null,"sync_state":{"chain":self.chain,"network":self.network,
                "target_count":target_count,"gap_limit":self.plan.as_ref().map(|p| p.gap_limit),"descriptor_kind":null,
                "policy_asset_id":self.policy_asset,"checkpoint":self.checkpoint,
                "targets":reported.iter().map(target_summary).collect::<Vec<_>>()},"normalized_records":records,"adapter_meta":meta,"observer_updates":updates,"elapsed_ms":0}),
        )
    }
}

/// What `compatibility_fetch` produced: an overlap skip, or the adapter's
/// metadata and the targets it observed.
enum Compatible {
    Skipped(Value),
    Fetched { meta: Value, targets: Vec<Value> },
}

impl Fetch {
    /// `compatibility_fetch(reason)`: online discovery for a descriptor (an
    /// address list has its targets), the ownership re-filter, then Python's
    /// compatibility Esplora or Electrum adapter. Records go to `records`.
    #[allow(clippy::too_many_arguments)]
    fn compatible(
        &self,
        reason: &str,
        gate: &Gate,
        scope: &Scope,
        proof: &FetchConsent,
        deadline: Instant,
        cancel: &dyn Control,
        records: &mut Vec<Value>,
    ) -> Result<Compatible> {
        let targets = if let Some(discovery) =
            self.compatibility.as_ref().filter(|_| self.plan.is_some())
        {
            let grant = DiscoveryConsent::for_fetch(discovery, proof.transport()?);
            let discovered = discovery.run(gate, scope, Some(&grant), deadline, cancel, |_| {})?;
            let online = discovered["targets"]
                .as_array()
                .ok_or_else(|| fallback("discovery_targets"))?;
            // Deeper online targets must not introduce an overlap the
            // finite preflight horizon could not see.
            let dropped = self
                .overlap
                .dropped(online, &self.chain, &self.network, false);
            let targets = source_overlap::keep(online, &dropped);
            if targets.is_empty() && !online.is_empty() {
                cancel.check(deadline)?;
                return Ok(Compatible::Skipped(self.skipped(online.len())));
            }
            // Discovery's `history_cache` is always empty, so the stored
            // graphs are the whole cache Python merges.
            targets
        } else {
            self.targets.clone()
        };
        let action = Acquisition::new_cached(
            scope.clone(),
            &self.backend,
            &targets,
            &self.version,
            &self.checkpoint,
            &self.history,
            &self.network,
        )?;
        let grant = AcquisitionConsent::for_fetch(&action, proof.transport()?);
        let tracked = targets
            .iter()
            .filter_map(|t| t["script_pubkey"].as_str().map(str::to_owned))
            .collect();
        let backend = text(&self.backend["name"]);
        let liquid = self.chain == "liquid";
        let acquired = action.run_with_history(gate, scope, Some(&grant), deadline, cancel, |_| {}, &mut |history| {
            // Liquid records decode the raw transactions after acquisition.
            if liquid {
                return Ok(());
            }
            let mut txs: Vec<_> = history["transactions"].as_object().into_iter().flatten()
                .map(|(_, v)| v).collect();
            txs.sort_by_key(|tx| (tx["status"]["block_time"].as_i64().unwrap_or(0), text(&tx["txid"])));
            let total = txs.len();
            for (index, tx) in txs.into_iter().enumerate() {
                cancel.check(deadline)?;
                if let Some(record) = crate::bitcoin_records::from_esplora(tx, &tracked, backend)? {
                    records.push(record);
                }
                if (index + 1).is_multiple_of(100) || index + 1 == total {
                    cancel.progress(json!({"phase":"decode_enrich", "transactions_seen":index+1,
                        "transactions_total":total,"records":records.len(),"processed":index+1,"total":total}))?;
                }
            }
            Ok(())
        })?;
        let route = if reason == "address_list" {
            "bitcoin_script"
        } else {
            "compatibility"
        };
        let electrum = self.backend["kind"] == "electrum";
        let meta = if liquid {
            let plan = self.plan.as_ref().ok_or_else(|| fallback("descriptor"))?;
            let context =
                liquid::Context::new(plan, &targets, &self.policy_asset, backend, &self.network);
            if electrum {
                records.extend(liquid::electrum_records(
                    &acquired, &context, deadline, cancel,
                )?);
                electrum::metadata(&acquired, route, reason, &|| {
                    liquid::electrum_outputs(&acquired, &context)
                })?
            } else {
                records.extend(liquid::esplora_records(
                    &acquired, &context, deadline, cancel,
                )?);
                esplora_metadata(&acquired, route, reason, &|| {
                    liquid::esplora_outputs(&acquired, &context)
                })?
            }
        } else if electrum {
            records.extend(electrum::records(
                &acquired, &tracked, backend, deadline, cancel,
            )?);
            electrum::metadata(&acquired, route, reason, &|| {
                electrum::outputs(&acquired, &self.chain, &self.network)
            })?
        } else {
            esplora_metadata(&acquired, route, reason, &|| {
                esplora_outputs(&acquired, &self.network)
            })?
        };
        Ok(Compatible::Fetched { meta, targets })
    }
}

/// `bdk_compatibility_reason` for a Bitcoin Electrum backend: `bdk_electrum`
/// takes no proxy (Kassiber's `socks5h` keeps DNS inside Tor) and no custom
/// trust store. Python reads the flat backend keys.
fn electrum_reason(backend: &Value) -> &'static str {
    let value = |key: &str| {
        [backend.get(key), backend["config"].get(key)]
            .into_iter()
            .flatten()
            .filter(|v| !v.is_null())
            .any(|v| !kassiber_pytext::strip(&descriptors::text(v)).is_empty())
    };
    if value("tor_proxy") || value("proxy") || onion(text(&backend["url"])) {
        "proxy_transport"
    } else if value("certificate") {
        "custom_ca"
    } else {
        ""
    }
}

/// `_electrum_stored_graph_wallets` and `_stored_wallet_chain_history`: the
/// Electrum graphs this wallet stored, by txid, when the backend its
/// configuration names is a stored Electrum backend. Each must still match
/// its provenance hash.
fn stored_history(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    config: &Value,
) -> Result<Value> {
    let name = if descriptors::truthy(&config["backend"]) {
        descriptors::text(&config["backend"])
    } else {
        String::new()
    };
    let kind: Option<String> = conn
        .query_row("SELECT kind FROM backends WHERE name=?", [&name], |r| {
            r.get(0)
        })
        .optional()
        .map_err(crate::database_error)?;
    if kind.is_none_or(|kind| kassiber_pytext::lower(kassiber_pytext::strip(&kind)) != "electrum") {
        return Ok(json!({}));
    }
    let mut statement = conn
        .prepare(
            "SELECT transactions.external_id, transactions.raw_json, chain_observation_provenance.graph_hash,
                chain_observation_provenance.observer_kinds_json
             FROM transactions
             JOIN chain_observation_provenance
               ON chain_observation_provenance.transaction_id = transactions.id
             WHERE transactions.profile_id = ?
               AND transactions.wallet_id IN (?)
               AND transactions.external_id_kind = 'txid'
               AND transactions.raw_json IS NOT NULL",
        )
        .map_err(crate::database_error)?;
    let rows = statement
        .query_map([profile_id, wallet_id], |r| {
            Ok((
                r.get::<_, kassiber_store::rusqlite::types::Value>(0)?,
                r.get::<_, kassiber_store::rusqlite::types::Value>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(crate::database_error)?;
    let mut history = serde_json::Map::new();
    for row in rows {
        use kassiber_store::rusqlite::types::Value as Sql;
        let (external_id, raw, hash, kinds) = row.map_err(crate::database_error)?;
        let kinds: Option<Value> =
            serde_json::from_str(kinds.as_deref().filter(|k| !k.is_empty()).unwrap_or("[]")).ok();
        if !kinds
            .as_ref()
            .and_then(Value::as_array)
            .is_some_and(|kinds| kinds.contains(&json!("electrum")))
        {
            continue;
        }
        let Sql::Text(raw) = raw else { continue };
        if raw.is_empty() || hash.unwrap_or_default() != observer_state::graph_hash(&raw)? {
            continue;
        }
        // A txid-kind row holds its txid as text; no other key can match one.
        let key = match external_id {
            Sql::Text(id) => id,
            Sql::Integer(id) => id.to_string(),
            _ => continue,
        };
        history.insert(key, Value::String(raw));
    }
    Ok(Value::Object(history))
}

/// `_prepare_negative_balance_repairs` for one prepared fetch: a prospective
/// deficit replaces it with one forced fetch at the widened gap, before any
/// publication savepoint opens. `run` performs that fetch under its grant.
pub(crate) fn with_repair(
    conn: &Connection,
    action: &Fetch,
    grant: &FetchConsent,
    fetch: Value,
    run: &mut dyn FnMut(&Fetch, &FetchConsent) -> Result<Value>,
) -> Result<Value> {
    if fetch["skip_outcome"].is_object() || !fetch["sync_state"].is_object() {
        return Ok(fetch);
    }
    let updates = fetch["observer_updates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let events = crate::publication::wallet::prospective_negative_events(
        conn,
        &action.scope.profile_id,
        &action.scope.wallet_id,
        &fetch,
        &updates,
        text(&action.backend["name"]),
    )?;
    // `negative_balance_rescan_gap_limit` has no gap to widen without a
    // plan; a silent-payment plan's gap is zero.
    let original = match (&action.plan, &action.silent) {
        (Some(plan), _) => plan.gap_limit,
        (None, Some(_)) => 0,
        (None, None) => return Ok(fetch),
    };
    if events.is_empty() {
        return Ok(fetch);
    }
    let repair = action.repair(conn)?;
    let repair_grant = FetchConsent::for_repair(&repair, grant);
    let mut repaired = run(&repair, &repair_grant)?;
    let events = events.iter().map(encode).collect::<Result<Vec<_>>>()?;
    if !repaired["adapter_meta"].is_object() {
        repaired["adapter_meta"] = json!({});
    }
    // Enrichments still read the wallet's own checkpoint, not the repair's.
    repaired["wallet_checkpoint"] = fetch["sync_state"]["checkpoint"].clone();
    repaired["adapter_meta"]["_prepared_negative_balance_rescan"] = json!({
        "triggered": true,
        "initial_negative_events": events,
        "original_gap_limit": original,
        "rescan_gap_limit": repair.plan.as_ref().map(|p| p.gap_limit).or(repair.gap),
    });
    Ok(repaired)
}

/// `bdk_compatibility_reason`'s label for an Esplora backend, which always
/// takes the compatibility observer. Python reads only the flat backend keys.
fn esplora_reason(backend: &Value) -> Result<&'static str> {
    let value = |key: &str| {
        backend
            .get(key)
            .filter(|v| !v.is_null())
            .map(descriptors::text)
            .map(|v| kassiber_pytext::strip(&v).to_owned())
            .filter(|v| !v.is_empty())
    };
    if crate::config::boolean(value("insecure").map(Value::String).as_ref())? {
        return Ok("insecure_tls");
    }
    if value("certificate").is_some() {
        return Ok("custom_ca");
    }
    let url = text(&backend["url"]);
    if kassiber_pytext::strip(url)
        .to_ascii_lowercase()
        .starts_with("https:")
        && std::env::var("KASSIBER_HOST_CA_BUNDLE")
            .is_ok_and(|v| kassiber_pytext::strip(&v).eq_ignore_ascii_case("explicit"))
    {
        return Ok("system_ca_trust");
    }
    if value("tor_proxy").is_some() || value("proxy").is_some() || onion(url) {
        return Ok("proxy_transport");
    }
    if value("auth_header").is_some() || value("token").is_some() {
        return Ok("custom_http_auth");
    }
    if let Some(timeout) = value("timeout") {
        if kassiber_pytext::parse_int(&timeout).ok() != Some(30.into()) {
            return Ok("custom_timeout");
        }
    }
    Ok("http_route_policy")
}

/// `is_onion_endpoint`: the URL's host ends in `.onion`.
fn onion(url: &str) -> bool {
    let raw = kassiber_pytext::strip(url);
    let rest = raw.split_once("://").map_or(raw, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if host.starts_with('[') {
        host.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    kassiber_pytext::lower(host.trim_end_matches('.')).ends_with(".onion")
}

/// `negative_balance_rescan_gap_limit`: at least 500, otherwise double the
/// (at least default) gap, at most the descriptor maximum.
pub(crate) fn rescan_gap_limit(current: u32) -> u32 {
    current.max(100).saturating_mul(2).clamp(500, 5_000)
}

/// `_offline_descriptor_targets`: each branch up to its checkpointed highest
/// use plus the gap, derived without a request.
fn offline_targets(plan: &Plan, checkpoint: &Value) -> Result<Vec<Value>> {
    let mut targets = vec![];
    for branch in &plan.branches {
        let observed = crate::scan::integer(&checkpoint["highest_used"][branch.index.to_string()])
            .unwrap_or_else(|| (-1).into());
        let gap = kassiber_pyjson::BigInt::from(plan.gap_limit);
        let end = (observed + gap.clone() + kassiber_pyjson::BigInt::from(1)).max(gap);
        let end = u32::try_from(end).map_err(|_| fallback("scan_horizon"))?;
        targets.extend(
            plan.derive(Some(branch.index), 0, end)
                .map_err(descriptors::Error::into_app)?,
        );
    }
    Ok(targets)
}

/// What a repair fetch overrides in preparation.
#[derive(Default)]
struct Overrides {
    /// `wallet_with_temporary_gap_limit`.
    gap: Option<u32>,
    /// The stored Electrum graphs; read from the book when absent.
    history: Option<Value>,
    /// Python's explicit `checkpoint`, instead of the stored one.
    checkpoint: Option<Value>,
    /// Preparation inside the publication's savepoint (the after-apply
    /// rescan), which the ordinary route forbids.
    in_savepoint: bool,
}

/// How preparation treats the profile's ownership evidence. Revalidation
/// leaves the index out: Python re-checks ownership only when it publishes,
/// and a fetch it skipped stays skipped, or keeps its prepared route.
#[derive(Clone, Copy, PartialEq)]
enum Preflight {
    Index,
    Prepared(&'static str),
    Skipped,
}

/// The target fields publication reads: the ownership re-check's, and the
/// address an address-list outcome reports.
fn target_summary(target: &Value) -> Value {
    json!({"script_pubkey":target["script_pubkey"],"branch_index":target["branch_index"],
        "address_index":target["address_index"],"address":target["address"]})
}

/// An address-list wallet's preparation, `resolve_wallet_sync_targets`
/// without a descriptor plan.
struct AddressList<'a> {
    config: &'a Value,
    given: &'a Value,
    backend: &'a Value,
    preflight: Preflight,
    scope: Scope,
    captured_config: String,
    checkpoint: Value,
    environment: Value,
    force_full: bool,
    version: &'a str,
    overlap: Overlap,
    label: String,
    history: Value,
}

impl Fetch {
    /// Python serves an address list with its compatibility observer
    /// (`address_list`, route `bitcoin_script`) on Esplora and Electrum.
    fn address_list(input: AddressList<'_>) -> Result<Self> {
        let AddressList {
            config,
            given,
            backend,
            preflight,
            scope,
            captured_config,
            checkpoint,
            environment,
            force_full,
            version,
            overlap,
            label,
            history,
        } = input;
        let addresses = crate::config::addresses(config.get("addresses"))?;
        if addresses.is_empty() {
            return Err(fallback("descriptor"));
        }
        let app = |e: descriptors::Error| e.into_app();
        let chain =
            descriptors::normalize_chain(&descriptors::field(config, "chain")).map_err(app)?;
        let network =
            descriptors::normalize_network(&chain, &descriptors::field(config, "network"))
                .map_err(app)?;
        if chain == "liquid" {
            return Err(AppError::new(
                "app_error",
                "Liquid live refresh currently requires descriptor-backed wallets so outputs can be unblinded locally",
            ));
        }
        crate::scan::validate_backend_scope(&chain, &network, backend, config)?;
        let transport = Backend::new(backend, backend["kind"] == "electrum")?;
        let core = backend["kind"] == "bitcoinrpc";
        // `sync_target_from_address`.
        let targets = addresses
            .iter()
            .enumerate()
            .map(|(index, address)| {
                let script = crate::config::bitcoin_address_script(address).ok_or_else(|| {
                    AppError::new("app_error", format!("Invalid Bitcoin address: {address}"))
                })?;
                Ok(json!({"chain":chain,"network":network,"branch_index":0,"branch_label":"address",
                    "address_index":index,"address":address,"unconfidential_address":null,
                    "script_pubkey":script}))
            })
            .collect::<Result<Vec<_>>>()?;
        let dropped = match preflight {
            Preflight::Index => overlap.dropped(&targets, &chain, &network, true),
            Preflight::Skipped | Preflight::Prepared(_) => Default::default(),
        };
        let kept = source_overlap::keep(&targets, &dropped);
        let skip = (preflight == Preflight::Skipped || (kept.is_empty() && !targets.is_empty()))
            .then_some(targets.len());
        Ok(Self {
            identity: Arc::new(()),
            scope,
            captured_config,
            given: given.clone(),
            backend: backend.clone(),
            transport: Some(transport),
            silent: None,
            plan: None,
            chain,
            network,
            checkpoint,
            environment,
            force_full,
            version: version.into(),
            observers: vec![],
            compatibility: None,
            core,
            history,
            targets: if skip.is_some() { targets } else { kept },
            overlap,
            skip,
            label,
            reason: if core { "" } else { "address_list" },
            gap: None,
            policy_asset: String::new(),
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `wallets.wallet_policy_asset_id` for a Liquid wallet: an explicit asset,
/// with a symbolic LBTC resolved to the network's policy asset.
fn policy_asset(config: &Value, network: &str) -> String {
    let default = match network {
        "liquidv1" => "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
        "liquidtestnet" => "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
        "elementsregtest" => "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
        _ => "",
    };
    let raw = descriptors::field(config, "policy_asset");
    let text = kassiber_pytext::strip(&raw);
    let lowered = text.to_lowercase();
    // `wallet_descriptors.normalize_asset_code`.
    let explicit = if text.is_empty() {
        String::new()
    } else if lowered.len() == 64 && lowered.bytes().all(|b| b.is_ascii_hexdigit()) {
        lowered
    } else if matches!(lowered.as_str(), "l-btc" | "lbtc") {
        "LBTC".into()
    } else {
        text.to_uppercase()
    };
    match explicit.as_str() {
        "" => default.to_owned(),
        "LBTC" if !default.is_empty() => default.to_owned(),
        _ => explicit,
    }
}

/// `resolve_wallet_sync_targets` for a Liquid descriptor wallet.
fn liquid_preflight(config: &Value, plan: &Plan, backend: &Value) -> Result<()> {
    if !plan.can_unblind() {
        return Err(AppError::new(
            "app_error",
            "Liquid descriptor wallets require private blinding keys for full sync and fee accounting",
        ));
    }
    if !descriptors::truthy(&config["backend"]) {
        return Err(AppError::new(
            "app_error",
            "Liquid wallets must name a backend explicitly; no public Liquid default is built in",
        ));
    }
    crate::scan::validate_backend(plan, backend, config)
}

/// `prepare_dependency_observer_fetch` for a Liquid descriptor wallet, up to
/// (not including) any network access.
#[allow(clippy::too_many_arguments)]
fn liquid_observers(
    conn: &Connection,
    wallet: &kassiber_pyjson::Value,
    config: &Value,
    plan: &Plan,
    backend: &Value,
    checkpoint: &Value,
    scope: &Scope,
    force_full: bool,
) -> Result<Vec<Observer>> {
    if let Some(reason) = lwk::descriptor::compatibility_reason(backend, plan)? {
        return Err(fallback(reason));
    }
    let wallet_id = scope.wallet_id.as_str();
    // `identities_for_wallet(..., observer_kind="lwk")`.
    let sources: Vec<(String, Vec<String>)> = if descriptors::truthy(&config["descriptor"]) {
        let mut keys: Vec<String> = vec![];
        for branch in &plan.branches {
            let key = if branch.label.to_lowercase().contains("change") {
                "change"
            } else {
                "receive"
            };
            if !keys.iter().any(|k| k == key) {
                keys.push(key.to_owned());
            }
        }
        if keys.is_empty() {
            keys.push("receive".into());
        }
        vec![("descriptor:default".into(), keys)]
    } else {
        let types: BTreeSet<String> = config["script_types"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| kassiber_pytext::strip(text(v)).to_lowercase())
            .filter(|v| !v.is_empty())
            .collect();
        types
            .into_iter()
            .map(|t| (format!("xpub:{t}"), vec!["receive".into(), "change".into()]))
            .collect()
    };
    if sources.is_empty() {
        return Err(AppError::new(
            "observer_identity_invalid",
            "The dependency observer route resolved no wallet identities",
        ));
    }
    let policy = policy_asset(config, &plan.network);
    let mut observers = vec![];
    for (source_key, branch_keys) in sources {
        let id = observer_state::instance_id(wallet_id, wallet_id, &source_key);
        let identity_value = json!({"id":id,"workspace_id":encode(field(wallet,"workspace_id"))?,
            "profile_id":scope.profile_id,"logical_wallet_id":wallet_id,"source_wallet_id":wallet_id,
            "source_key":source_key,"observer_kind":"lwk","chain":"liquid","network":plan.network,
            "branch_keys":branch_keys});
        // A forced rebuild never loads the opaque store; its rows stay until
        // the replacement is applied.
        let values = if force_full {
            BTreeMap::new()
        } else {
            observer_state::load_values(conn, &id)?
        };
        let loaded = observer_state::load_state(conn, &identity(&identity_value), force_full)?;
        let action = lwk::Execution::new(
            scope.clone(),
            backend,
            plan,
            &plan.network,
            &policy,
            branch_keys,
            lwk::Refresh {
                force_full,
                prior: loaded.prior.clone(),
                prior_coverage: loaded.coverage.clone(),
                checkpoint: observer_state::identity_checkpoint(checkpoint, &id),
                values: values.clone(),
            },
        )?;
        observers.push(Observer {
            identity: identity_value,
            action: Action::Lwk(action),
            prior: loaded.prior,
            current: None,
            epoch: loaded.expected_epoch,
            values: Some(values),
        });
    }
    Ok(observers)
}

fn esplora_metadata(
    acquired: &Value,
    route: &str,
    reason: &str,
    outputs: &dyn Fn() -> Result<Vec<Value>>,
) -> Result<Value> {
    let mut meta = json!({"observer_route":route,"observer_compatibility_reason":reason,
            "freshness_checkpoint":acquired["freshness_checkpoint"],"scripts_changed":acquired["scripts_changed"],"scripts_unchanged":acquired["scripts_unchanged"],
            "known_txids":acquired["transactions"].as_object().map_or(0,|t|t.len()),"esplora_history_complete":acquired["esplora_history_complete"],
            "observer_retracted_external_ids":acquired["observer_retracted_external_ids"]});
    if acquired["utxos_skipped_unchanged"] == true {
        meta["utxos_skipped_unchanged"] = json!(true);
        return Ok(meta);
    }
    meta["utxos"] = json!(outputs()?);
    Ok(meta)
}

/// `_esplora_bitcoin_utxo_record` for every listed output, in target order.
fn esplora_outputs(acquired: &Value, network: &str) -> Result<Vec<Value>> {
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
    Ok(outputs)
}
