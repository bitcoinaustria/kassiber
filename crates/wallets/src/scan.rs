//! Preparation-only Bitcoin compatibility descriptor discovery. This ports
//! `discover_compatibility_descriptor_targets`, not BDK's ordinary Electrum
//! observer. Targets are candidate scripts; the coordinator owns observation,
//! ownership and custody policy.
//! Nothing in this module opens a database or persists a scan checkpoint.
use crate::descriptors::{self, Plan};
use kassiber_chain::{Backend, ElectrumClient, HttpClient};
use kassiber_envelope::AppError;
use kassiber_net::{Consent, Gate};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

type Result<T> = std::result::Result<T, AppError>;

/// The caller supplies the current scope again when starting the action. A
/// future apply coordinator must recheck it before publishing prepared facts.
#[derive(Clone, PartialEq, Eq)]
pub struct Scope {
    pub database_instance: String,
    pub profile_id: String,
    pub wallet_id: String,
    pub environment_id: String,
    pub source_revision: String,
}

/// Cooperative stop between derivation/protocol batches and around HTTP work.
/// In-flight I/O retains the supplied overall deadline and backend timeout.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

/// A request can add synchronous authority checks at protocol boundaries.
/// Database checks run on the connection's owning thread; parallel discovery
/// workers receive only the clonable cancellation flag.
pub trait Control {
    fn check(&self, deadline: Instant) -> Result<()>;
    fn cancellation(&self) -> &Cancellation;
    fn progress(&self, _event: Value) -> Result<()> {
        Ok(())
    }
}

pub struct Checked<'a> {
    pub cancel: &'a Cancellation,
    pub check: &'a dyn Fn() -> Result<()>,
}

impl Control for Checked<'_> {
    fn check(&self, deadline: Instant) -> Result<()> {
        self.cancel.check(deadline)?;
        (self.check)()
    }
    fn cancellation(&self) -> &Cancellation {
        self.cancel
    }
}

impl Control for Cancellation {
    fn check(&self, deadline: Instant) -> Result<()> {
        Cancellation::check(self, deadline)
    }
    fn cancellation(&self) -> &Cancellation {
        self
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub(crate) fn check(&self, deadline: Instant) -> Result<()> {
        if self.0.load(Ordering::SeqCst) {
            return Err(AppError::new("cancelled", "Wallet discovery was cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(kassiber_net::Error::InvalidTimeout.app_error());
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Esplora,
    Electrum,
}

/// An immutable, request-local snapshot. Constructing it derives no address and
/// opens no connection. It deliberately has no Debug/serialization projection.
pub struct CompatibilityDiscovery {
    scope: Scope,
    identity: Arc<()>,
    plan: Plan,
    backend: Backend,
    kind: Kind,
    checkpoint: Value,
    product_version: String,
}

/// A grant belongs to one exact action object, including its immutable wallet,
/// backend, checkpoint and scope. It cannot be reused for another scan, probe or
/// a reconfigured action, even if that action reaches the same endpoint.
pub struct DiscoveryConsent {
    identity: Arc<()>,
    transport: Consent,
    started: AtomicBool,
}
impl DiscoveryConsent {
    pub(crate) fn for_fetch(action: &CompatibilityDiscovery, transport: Consent) -> Self {
        Self {
            identity: action.identity.clone(),
            transport,
            started: AtomicBool::new(false),
        }
    }
    /// Call only after the user explicitly triggers this discovery action.
    pub fn explicit_trigger(action: &CompatibilityDiscovery) -> Self {
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

fn unsupported() -> AppError {
    AppError::new(
        "not_implemented",
        "Compatibility discovery supports Bitcoin and Liquid descriptor wallets over Esplora and Electrum only",
    )
}

impl CompatibilityDiscovery {
    /// This is an internal compatibility-route building block, not the product
    /// sync dispatcher. Ordinary Electrum must keep its BDK observer route.
    pub fn new(
        scope: Scope,
        wallet_config: &Value,
        backend_config: &Value,
        checkpoint: &Value,
        product_version: &str,
    ) -> Result<Self> {
        if !wallet_config.is_object() || !backend_config.is_object() {
            return Err(AppError::new(
                "validation",
                "Invalid discovery configuration",
            ));
        }
        let mut backend_config = backend_config.clone();
        if let Some(raw) = backend_config.get("config_json").and_then(Value::as_str) {
            let nested: Value = serde_json::from_str(raw)
                .map_err(|_| AppError::new("validation", "Invalid backend configuration JSON"))?;
            backend_config["config"] = nested;
        }
        if wallet_config
            .get("sp_descriptor")
            .is_some_and(descriptors::truthy)
            || wallet_config
                .get("silent_payment")
                .is_some_and(descriptors::truthy)
        {
            return Err(unsupported());
        }
        let plan = descriptors::load(wallet_config)
            .map_err(descriptors::Error::into_app)?
            .ok_or_else(unsupported)?;
        if !matches!(plan.chain.as_str(), "bitcoin" | "liquid") {
            return Err(unsupported());
        }
        let kind = match field(&backend_config, "kind").and_then(Value::as_str) {
            Some("esplora" | "mempool") => Kind::Esplora,
            Some("electrum") => Kind::Electrum,
            _ => return Err(unsupported()),
        };
        validate_backend(&plan, &backend_config, wallet_config)?;
        for key in ["esplora_scripthashes", "electrum_scripthash_statuses"] {
            if checkpoint
                .get(key)
                .is_some_and(|v| descriptors::truthy(v) && !v.is_object())
            {
                return Err(AppError::new(
                    "validation",
                    "Invalid descriptor discovery checkpoint",
                ));
            }
        }
        Ok(Self {
            scope,
            identity: Arc::new(()),
            plan,
            backend: Backend::new(&backend_config, matches!(kind, Kind::Electrum))?,
            kind,
            checkpoint: if checkpoint.is_object() {
                checkpoint.clone()
            } else {
                json!({})
            },
            product_version: product_version.into(),
        })
    }

    /// Returns Python's preparation shape, including an empty history_cache.
    /// No durable state is changed; a failed/cancelled scan returns no targets.
    pub fn run(
        &self,
        gate: &Gate,
        current_scope: &Scope,
        consent: Option<&DiscoveryConsent>,
        deadline: Instant,
        cancel: &dyn Control,
        mut progress: impl FnMut(Value),
    ) -> Result<Value> {
        cancel.check(deadline)?;
        if current_scope != &self.scope {
            return Err(AppError::new(
                "stale_context",
                "Wallet discovery scope changed",
            ));
        }
        let proof = consent
            .filter(|proof| Arc::ptr_eq(&proof.identity, &self.identity))
            .ok_or_else(|| {
                AppError::new(
                    "interaction_required",
                    "Wallet discovery requires consent for this exact action",
                )
            })?;
        // Even a fixed descriptor or an entirely cached scan must obey offline
        // policy. Authorization itself performs no DNS, socket or write.
        gate.authorize(
            self.backend.request().clone(),
            Some(proof.transport.clone()),
        )
        .map_err(|e| e.app_error())?;
        if proof.started.swap(true, Ordering::SeqCst) {
            return Err(AppError::new(
                "interaction_required",
                "Wallet discovery consent was already used",
            ));
        }
        let targets = match self.kind {
            Kind::Esplora => {
                let client =
                    HttpClient::new(self.backend.clone(), gate.clone(), &self.product_version);
                let workers = self.backend.batch_size().clamp(1, 8);
                scan_plan(
                    &self.plan,
                    &self.checkpoint,
                    workers,
                    cancel,
                    deadline,
                    &mut progress,
                    |targets| {
                        // All workers finish before returning an error. No worker
                        // or connection can outlive this request-local action.
                        std::thread::scope(|threads| {
                            let handles: Vec<_> = targets
                                .iter()
                                .map(|target| {
                                    let client = &client;
                                    let cancel = cancel.cancellation();
                                    let checkpoint = &self.checkpoint;
                                    let transport = &proof.transport;
                                    threads.spawn(move || {
                                        cancel.check(deadline)?;
                                        let hash = script_hash(
                                            target["script_pubkey"]
                                                .as_str()
                                                .expect("derived script"),
                                        )?;
                                        let cached =
                                            &checkpoint["esplora_scripthashes"][&hash]["tx_count"];
                                        if counter(cached)? > 0.into() {
                                            return Ok(true);
                                        }
                                        let stats = client.get_json(
                                            &format!("scripthash/{hash}"),
                                            Some(transport),
                                            deadline,
                                        )?;
                                        cancel.check(deadline)?;
                                        has_history(&stats)
                                    })
                                })
                                .collect();
                            handles
                                .into_iter()
                                .map(|h| {
                                    h.join()
                                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                                })
                                .collect()
                        })
                    },
                )?
            }
            Kind::Electrum => {
                let mut client = ElectrumClient::connect(
                    self.backend.clone(),
                    gate,
                    Some(proof.transport.clone()),
                    deadline,
                )?;
                let mut statuses: HashMap<String, Value> = self.checkpoint
                    ["electrum_scripthash_statuses"]
                    .as_object()
                    .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                scan_plan(
                    &self.plan,
                    &self.checkpoint,
                    self.backend.batch_size(),
                    cancel,
                    deadline,
                    &mut progress,
                    |targets| {
                        cancel.check(deadline)?;
                        let hashes = targets
                            .iter()
                            .map(|t| {
                                script_hash(t["script_pubkey"].as_str().expect("derived script"))
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let missing: Vec<_> = hashes
                            .iter()
                            .filter(|hash| statuses.get(*hash).is_none_or(Value::is_null))
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            let requests = missing
                                .iter()
                                .map(|hash| {
                                    ("blockchain.scripthash.subscribe".into(), json!([hash]))
                                })
                                .collect::<Vec<_>>();
                            let replies = client.batch_call(&requests, deadline)?;
                            cancel.check(deadline)?;
                            for (hash, status) in missing.into_iter().zip(replies) {
                                statuses.insert(hash, status);
                            }
                        }
                        Ok(hashes
                            .iter()
                            .map(|hash| statuses.get(hash).is_some_and(|v| !v.is_null()))
                            .collect())
                    },
                )?
            }
        };
        cancel.check(deadline)?;
        Ok(json!({"targets":targets,"history_cache":{}}))
    }
}

/// `validate_backend_for_wallet`'s chain and network agreement, plus the
/// local chain instance binding. Inert: names come from saved settings only.
pub(crate) fn validate_backend(
    plan: &Plan,
    backend_config: &Value,
    wallet_config: &Value,
) -> Result<()> {
    validate_backend_scope(&plan.chain, &plan.network, backend_config, wallet_config)
}

/// `validate_backend` for an explicit chain and network (an address list).
pub(crate) fn validate_backend_scope(
    chain: &str,
    network: &str,
    backend_config: &Value,
    wallet_config: &Value,
) -> Result<()> {
    let name = field(backend_config, "name")
        .and_then(Value::as_str)
        .unwrap_or("candidate");
    if let Some(configured) = field(backend_config, "chain") {
        let expected = descriptors::normalize_chain(&descriptors::text(configured))
            .map_err(descriptors::Error::into_app)?;
        if expected != chain {
            return Err(AppError::new(
                "app_error",
                format!(
                    "Backend '{name}' is configured for {expected}, but source refresh requires {}",
                    chain
                ),
            ));
        }
    }
    if let Some(configured) = field(backend_config, "network") {
        let expected = descriptors::normalize_network(chain, &descriptors::text(configured))
            .map_err(descriptors::Error::into_app)?;
        if expected != network {
            return Err(AppError::new(
                "app_error",
                format!(
                    "Backend '{name}' is configured for {expected}, but source refresh requires {}",
                    network
                ),
            ));
        }
    }
    if let Some(instance) = field(backend_config, "chain_instance_id") {
        if Some(instance) != wallet_config.get("chain_instance_id") {
            return Err(AppError::new(
                "book_network_mismatch",
                "Backend belongs to another local chain instance",
            ));
        }
    }
    Ok(())
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let usable = |v: &&Value| !v.is_null() && v.as_str() != Some("");
    value
        .get(key)
        .filter(usable)
        .or_else(|| value.get("config").and_then(|c| c.get(key)).filter(usable))
}

fn counter(value: &Value) -> Result<BigInt> {
    if !descriptors::truthy(value) {
        return Ok(0.into());
    }
    integer(value)
        .ok_or_else(|| AppError::new("protocol_error", "Invalid descriptor history counter"))
}

fn has_history(stats: &Value) -> Result<bool> {
    if !stats.is_object()
        || ["chain_stats", "mempool_stats"].iter().any(|key| {
            stats
                .get(key)
                .is_some_and(|v| descriptors::truthy(v) && !v.is_object())
        })
    {
        return Err(AppError::new(
            "protocol_error",
            "Invalid descriptor history statistics",
        ));
    }
    let count =
        counter(&stats["chain_stats"]["tx_count"])? + counter(&stats["mempool_stats"]["tx_count"])?;
    Ok(count > 0.into())
}

/// CPython int coercion used by discovery checkpoints and server counters.
pub(crate) fn integer(value: &Value) -> Option<BigInt> {
    match value {
        Value::String(s) => kassiber_pytext::parse_int(s).ok(),
        Value::Bool(b) => Some(u32::from(*b).into()),
        Value::Number(n) => kassiber_pytext::parse_int(&n.to_string()).ok().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite())
                .and_then(|f| kassiber_pytext::parse_int(&format!("{:.0}", f.trunc())).ok())
        }),
        _ => None,
    }
}

/// Electrum and Esplora both use reversed SHA-256 of the script bytes.
pub fn script_hash(script: &str) -> Result<String> {
    use bitcoin::hex::FromHex;
    let bytes = Vec::<u8>::from_hex(script)
        .map_err(|_| AppError::new("validation", "Invalid script encoding"))?;
    let mut digest = Sha256::digest(bytes);
    digest.reverse();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn scan_plan(
    plan: &Plan,
    checkpoint: &Value,
    batch_size: usize,
    cancel: &dyn Control,
    deadline: Instant,
    progress: &mut impl FnMut(Value),
    mut used: impl FnMut(&[Value]) -> Result<Vec<bool>>,
) -> Result<Vec<Value>> {
    let mut targets = Vec::new();
    let mut checked = 0;
    for branch in &plan.branches {
        cancel.check(deadline)?;
        let gap = if branch.ranged() { plan.gap_limit } else { 1 };
        if gap <= 1 {
            targets.extend(
                plan.derive(Some(branch.index), 0, 1)
                    .map_err(descriptors::Error::into_app)?,
            );
            continue;
        }
        let highest = integer(&checkpoint["highest_used"][branch.index.to_string()])
            .filter(|n| n >= &0.into());
        let mut index = 0u32;
        if let Some(highest) = highest {
            let highest = u32::try_from(highest)
                .ok()
                .filter(|n| *n < (1 << 31) - 1)
                .ok_or_else(|| {
                    AppError::new("validation", "Invalid descriptor discovery checkpoint")
                })?;
            // Derive known prefixes in bounded batches, checking cancellation
            // and deadlines even when no network request is needed.
            while index <= highest {
                cancel.check(deadline)?;
                let end = index.saturating_add(100).min(highest + 1);
                targets.extend(
                    plan.derive(Some(branch.index), index, end)
                        .map_err(descriptors::Error::into_app)?,
                );
                index = end;
            }
        }
        let mut unused = 0;
        while unused < gap {
            cancel.check(deadline)?;
            let end = index
                .checked_add(u32::try_from(batch_size).unwrap_or(u32::MAX))
                .filter(|n| *n <= 1 << 31)
                .ok_or_else(|| {
                    AppError::new("validation", "Descriptor discovery range exhausted")
                })?;
            let batch = plan
                .derive(Some(branch.index), index, end)
                .map_err(descriptors::Error::into_app)?;
            if batch.is_empty() {
                break;
            }
            let usage = used(&batch)?;
            cancel.check(deadline)?;
            if usage.len() != batch.len() {
                return Err(AppError::new("app_error", "Compatibility descriptor discovery returned an unexpected number of usage checks"));
            }
            checked += batch.len();
            for (target, is_used) in batch.into_iter().zip(usage) {
                targets.push(target);
                unused = if is_used { 0 } else { unused + 1 };
                index += 1;
                if unused >= gap {
                    break;
                }
            }
            let event = json!({"phase":"discovery","targets_checked":checked,"retained_targets":targets.len(),
                "branch_index":branch.index,"gap_limit":gap,"unused_streak":unused,"processed":checked});
            progress(event.clone());
            cancel.progress(event)?;
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests;
