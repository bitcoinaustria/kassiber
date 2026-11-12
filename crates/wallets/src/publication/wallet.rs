//! The fenced, atomic wallet publication coordinator. It applies one prepared
//! Bitcoin descriptor-wallet fetch as Python's
//! `handlers._apply_wallet_sync_atomically` → `sync.sync_wallets` →
//! `sync_wallet_from_backend` → `_mark_wallet_synced_from_results` does, under
//! one savepoint. Preparation (all network I/O) must finish before entry, and
//! no progress callback runs inside it. Unsupported wallet shapes, routes and
//! repairs refuse with `native_fallback_required`, rolling back. This internal
//! refusal never authorizes a Python retry after native execution starts. The
//! product router must select an unsupported whole invocation before bootstrap.
use super::row::{field, first, or_text, rows, Result};
use super::{insert, inventory, policy, provenance, reports};
use crate::{database_error, observer_apply};
use kassiber_envelope::{dict, get, py_str, s, set, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::Connection;
use serde_json::Value as Json;
use std::cell::RefCell;

/// Deterministic seams for `uuid.uuid4()` and `time_utils.now_iso()`.
pub trait Sources {
    fn id(&mut self) -> String;
    fn now(&mut self) -> String;
}

/// Random version-four identities and second-resolution UTC timestamps.
#[derive(Default)]
pub struct SystemSources {
    counter: u64,
}

impl Sources for SystemSources {
    fn id(&mut self) -> String {
        use std::hash::{BuildHasher, Hasher};
        // Each RandomState draws fresh process randomness; the counter keeps
        // identities distinct even if two states shared keys.
        let mut bytes = [0u8; 16];
        for half in 0..2 {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u64(self.counter);
            hasher.write_u8(half);
            self.counter = self.counter.wrapping_add(1);
            bytes[half as usize * 8..half as usize * 8 + 8]
                .copy_from_slice(&hasher.finish().to_le_bytes());
        }
        bytes[6] = (bytes[6] & 15) | 64;
        bytes[8] = (bytes[8] & 63) | 128;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )
    }
    fn now(&mut self) -> String {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        kassiber_chain::format_unix_time(seconds).unwrap_or_default()
    }
}

/// The publication's sources and when it began, for `elapsed_ms`.
struct Clock<'a>(RefCell<&'a mut dyn Sources>, std::time::Instant);

impl Clock<'_> {
    fn id(&self) -> String {
        self.0.borrow_mut().id()
    }
    fn now(&self) -> String {
        self.0.borrow_mut().now()
    }
}

/// Python's `APPLY_STAGE_*` seams, in order. `stage` sees each after its writes.
pub const STAGES: [&str; 6] = [
    "observer_persistence",
    "retractions",
    "transaction_insertion",
    "output_inventory",
    "derivation_coverage",
    "freshness_checkpoint",
];

pub struct Request<'a> {
    pub profile_id: &'a str,
    pub wallet_id: &'a str,
    /// The wallet `config_json` captured when the fetch was prepared.
    pub captured_config: &'a str,
    /// The serialized prepared fetch; see `docs/reference/rust-shell.md`.
    pub fetch: &'a Json,
    /// Greenfield access for the wallet's BTCPay provenance routes; a wallet
    /// with routes refuses without it.
    pub btcpay: Option<&'a crate::sync::btcpay::Context<'a>>,
}

pub(crate) fn fallback(reason: &str) -> AppError {
    AppError::new(
        "native_fallback_required",
        "This wallet refresh is not supported by the native publication path",
    )
    .with_details(dict([("reason", s(reason))]))
}

fn invalid(message: &str) -> AppError {
    AppError::new("observer_state_invalid", message)
}

fn py(value: &Json) -> Result<Value> {
    kassiber_pytext::loads(&value.to_string())
        .map_err(|_| invalid("Prepared fetch is not valid JSON"))
}

fn json(value: &Value) -> Result<Json> {
    let text = kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new())
        .map_err(|_| invalid("Publication value cannot be encoded"))?;
    serde_json::from_str(&text).map_err(|_| invalid("Publication value cannot be encoded"))
}

fn config_mapping(text: &str) -> Value {
    match kassiber_pytext::loads(if text.is_empty() { "{}" } else { text }) {
        Ok(value @ Value::Dict(_)) => value,
        _ => Value::Dict(vec![]),
    }
}

/// `book_network.new_wallet_config(conn, profile_id, kind, config)`.
fn network_wallet_config(
    conn: &Connection,
    profile_id: &str,
    kind: &str,
    config: &Value,
) -> Result<Value> {
    let mut config = config.clone();
    let Some(binding) = first(
        conn,
        "SELECT * FROM book_network_bindings WHERE profile_id=?",
        [profile_id],
    )?
    else {
        return Ok(config);
    };
    let material = [
        "chain",
        "network",
        "descriptor",
        "change_descriptor",
        "xpub",
        "addresses",
    ]
    .iter()
    .any(|key| get(&config, key).is_some_and(truthy));
    if !(material || matches!(kind, "coreln" | "lnd" | "lightning" | "nwc" | "phoenix")) {
        return Ok(config);
    }
    let liquid = matches!(get(&config, "chain"), Some(Value::Str(c)) if matches!(c.as_str(), Some("liquid" | "lbtc" | "elements")))
        || kind == "liquid-electrum";
    let environment = py_str(field(&binding, "environment"));
    let network = match (environment.as_str(), liquid) {
        ("main", false) => "main",
        ("main", true) => "liquidv1",
        ("test", false) => "test",
        ("test", true) => "liquidtestnet",
        ("signet", false) => "signet",
        ("regtest", false) => "regtest",
        ("regtest", true) => "elementsregtest",
        _ => {
            return Err(AppError::new(
                "book_network_mismatch",
                "This chain has no domain in the book's environment",
            ))
        }
    };
    if get(&config, "chain").is_none() {
        set(
            &mut config,
            "chain",
            s(if liquid { "liquid" } else { "bitcoin" }),
        );
    }
    if get(&config, "network").is_none() {
        set(&mut config, "network", s(network));
    }
    let instance = field(&binding, "chain_instance_id");
    if truthy(instance) && get(&config, "chain_instance_id").is_none() {
        set(&mut config, "chain_instance_id", instance.clone());
    }
    Ok(config)
}

/// `wallet_for_network_sync`: guard the authored scope, then route from it.
pub(crate) fn network_scope(
    conn: &Connection,
    profile_id: &str,
    kind: &str,
    config_json: &str,
) -> Result<(String, Value)> {
    let config = config_mapping(config_json);
    kassiber_setup::networks::guard_wallet(conn, profile_id, &config, None, "sync")?;
    let environment = kassiber_setup::networks::resolve(conn, profile_id)?;
    let routed = network_wallet_config(conn, profile_id, kind, &config)?;
    let routed = kassiber_pyjson::dumps(&routed, &kassiber_pyjson::Options::new())
        .map_err(|_| AppError::new("app_error", "Wallet configuration cannot be encoded"))?;
    Ok((routed, field(&environment, "environment_id").clone()))
}

/// The supported native shape: one Bitcoin or Liquid descriptor wallet, or
/// a silent-payment wallet, refreshed from a named backend, with no
/// alternate source routes. Liquid routes other than LWK over Electrum
/// refuse at fetch.
pub(crate) fn require_supported(wallet: &Value) -> Result<()> {
    let config = config_mapping(&or_text(field(wallet, "config_json")));
    let kind = or_text(field(wallet, "kind"));
    // `sync_wallets` dispatches a BTCPay source and a file source (both
    // `source_file` and `source_format`) before any chain route; a stray key
    // alone does not change the route.
    let truthy_key = |key: &str| get(&config, key).is_some_and(truthy);
    let btcpay = get(&config, "sync_source").is_some_and(|v| {
        !matches!(v, Value::Null) && !kassiber_pytext::strip(&py_str(v)).is_empty()
    });
    if btcpay || (truthy_key("source_file") && truthy_key("source_format")) {
        return Err(fallback("wallet_route"));
    }
    if crate::silent_payments::has_material(&json(&config)?) {
        return Ok(());
    }
    // A node-kind wallet reaches a chain route only without a node backend,
    // as `sync_wallets` dispatches it.
    let node =
        matches!(kind.as_str(), "coreln" | "lnd") && get(&config, "backend").is_some_and(truthy);
    let chain = kassiber_pytext::lower(kassiber_pytext::strip(&or_text(field(&config, "chain"))));
    if node
        || !matches!(
            chain.as_str(),
            "" | "bitcoin" | "btc" | "onchain" | "liquid" | "lbtc" | "elements"
        )
    {
        return Err(fallback("wallet_route"));
    }
    let present = |key: &str| {
        get(&config, key).is_some_and(|v| !kassiber_pytext::strip(&or_text(v)).is_empty())
    };
    if !(present("descriptor")
        || (present("xpub") && get(&config, "script_types").is_some_and(truthy))
        || !crate::config::addresses(json(&config)?.get("addresses"))?.is_empty())
    {
        return Err(fallback("wallet_route"));
    }
    Ok(())
}

/// `outcome["negative_balance_rescan"]` for a prefetch repair, in Python's
/// key order (the marker crossed the fetch contract as plain JSON).
fn rescan_outcome(prepared: &Json, remaining: Vec<Value>) -> Result<Value> {
    let event = |value: &Json| -> Result<Value> {
        Ok(dict(
            [
                "asset",
                "transaction_id",
                "external_id",
                "occurred_at",
                "delta_msat",
                "running_balance_msat",
            ]
            .map(|key| (key, value[key].clone()))
            .into_iter()
            .map(|(key, value)| py(&value).map(|value| (key, value)))
            .collect::<Result<Vec<_>>>()?,
        ))
    };
    let initial = prepared["initial_negative_events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(event)
        .collect::<Result<Vec<_>>>()?;
    Ok(dict([
        ("triggered", py(&prepared["triggered"])?),
        ("initial_negative_events", Value::List(initial)),
        ("original_gap_limit", py(&prepared["original_gap_limit"])?),
        ("rescan_gap_limit", py(&prepared["rescan_gap_limit"])?),
        ("resolved", Value::Bool(remaining.is_empty())),
        ("remaining_negative_events", Value::List(remaining)),
    ]))
}

/// `sync_wallet_from_backend`'s ownership re-check against the current book:
/// an earlier wallet of the same operation may have published inventory that
/// now makes a better owner of a fetched target. A prepared fetch always
/// carries a payload by now, so Python refuses rather than trimming it.
/// Fetches serialized without targets (Python's own, in the replay oracles)
/// skip the check.
fn recheck_overlap(
    conn: &Connection,
    request: &Request<'_>,
    wallet: &Value,
    routed: &str,
    sync_state: &Json,
) -> Result<()> {
    let Some(targets) = sync_state["targets"].as_array() else {
        return Ok(());
    };
    let label = py_str(field(wallet, "label"));
    let config: Json = serde_json::from_str(routed).unwrap_or_else(|_| serde_json::json!({}));
    let overlap = crate::source_overlap::Overlap::capture(
        conn,
        request.profile_id,
        request.wallet_id,
        &label,
        &or_text(field(wallet, "kind")),
        &config,
    )?;
    let chain = sync_state["chain"].as_str().unwrap_or("");
    let network = sync_state["network"].as_str().unwrap_or("");
    if overlap.dropped(targets, chain, network, false).is_empty() {
        return Ok(());
    }
    let name = if label.is_empty() {
        "Wallet".into()
    } else {
        label
    };
    Err(AppError::new(
        "source_overlap_retry",
        format!("Wallet source overlap changed during refresh for {name}"),
    )
    .with_hint("Retry refresh so Kassiber can filter overlapping sync targets before contacting the backend and importing history.")
    .retryable(true))
}

fn plain_records(facts: &Json, key: &str) -> Result<Vec<Json>> {
    let items = facts[key].as_array().cloned().unwrap_or_default();
    for (index, item) in items.iter().enumerate() {
        if !item.is_object() {
            return Err(
                invalid("Observer facts must use explicit objects").with_details(dict([
                    ("field", s(key)),
                    ("index", Value::Int(index.into())),
                ])),
            );
        }
    }
    Ok(items)
}

fn text<'a>(value: &'a Json, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// `apply_fetch_observer_updates`: persist each observer's state and policy
/// coverage in order, then compose the normalized facts.
fn apply_observer_updates(
    conn: &Connection,
    profile: &Value,
    updates: &[Json],
    adapter_meta: &Json,
    backend_name: &str,
    clock: &Clock<'_>,
) -> Result<(Vec<Json>, Json)> {
    let mut observers = Vec::new();
    for update in updates {
        let identity = &update["identity"];
        let branch_keys: Vec<String> = identity["branch_keys"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().unwrap_or("").to_owned())
            .collect();
        let branch_refs: Vec<&str> = branch_keys.iter().map(String::as_str).collect();
        let facts = &update["update"]["facts"];
        let records = plain_records(facts, "transaction_records")?;
        let outputs = plain_records(facts, "outputs")?;
        let mut retracted: Vec<String> = Vec::new();
        for value in facts["retracted_external_ids"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let value = kassiber_pytext::strip(&py_str(&py(value)?)).to_owned();
            if !value.is_empty() && !retracted.contains(&value) {
                retracted.push(value);
            }
        }
        let mut coverage = Vec::new();
        let mut points = Vec::new();
        for point in facts["coverage"].as_array().into_iter().flatten() {
            let scanned_to = point["scanned_to"]
                .as_i64()
                .ok_or_else(|| invalid("Invalid observer coverage"))?;
            let highest_used = point["highest_used"].as_i64();
            coverage.push(observer_apply::Coverage {
                branch_key: text(point, "branch_key").into(),
                highest_used,
                scanned_to,
                details: point["details"].clone(),
            });
            points.push(policy::Point {
                branch_key: text(point, "branch_key").into(),
                scanned_to,
                highest_used,
            });
        }
        let state = &update["update"]["state"];
        if !state.is_object() || !facts["freshness_checkpoint"].is_object() {
            return Err(invalid(
                "Observer application must expose normalized chain facts",
            ));
        }
        let timestamp = clock.now();
        observer_apply::persist(
            conn,
            &observer_apply::Prepared {
                identity: crate::observer_state::Identity {
                    id: text(identity, "id"),
                    observer_kind: text(identity, "observer_kind"),
                    chain: text(identity, "chain"),
                    network: text(identity, "network"),
                    logical_wallet_id: text(identity, "logical_wallet_id"),
                    source_wallet_id: text(identity, "source_wallet_id"),
                    source_key: text(identity, "source_key"),
                },
                workspace_id: &py_str(field(profile, "workspace_id")),
                profile_id: &py_str(field(profile, "id")),
                branch_keys: &branch_refs,
                state: state.clone(),
                coverage,
                expected_epoch: update["expected_epoch"].as_i64(),
            },
            &timestamp,
        )?;
        if let Some(values) = update.get("opaque_values").filter(|v| !v.is_null()) {
            // `persist_opaque_state`: the dependency's buffered store becomes
            // durable only here, with its own clock read like Python's.
            observer_apply::persist_values(conn, text(identity, "id"), values, &clock.now())?;
        }
        policy::record_coverage(
            conn,
            &policy::Identity {
                source_wallet_id: text(identity, "source_wallet_id"),
                source_key: text(identity, "source_key"),
                observer_kind: text(identity, "observer_kind"),
                chain: text(identity, "chain"),
                network: text(identity, "network"),
                branch_keys: &branch_keys,
            },
            &points,
            &mut || clock.id(),
            &mut || clock.now(),
        )?;
        observers.push(crate::composition::ObserverFacts {
            observer_id: text(identity, "id").into(),
            observer_kind: text(identity, "observer_kind").into(),
            records,
            outputs,
            retracted_external_ids: retracted,
            checkpoint: facts["freshness_checkpoint"].clone(),
            reused_records: facts["reused_records"].as_u64().unwrap_or(0),
        });
    }
    let composed = crate::composition::compose(&observers, &[], adapter_meta, backend_name)?;
    let mut metadata = composed.metadata;
    if !composed.provenance_entries.is_empty() {
        metadata["_chain_observation_provenance"] = serde_json::json!({
            "application_revision": clock.id(),
            "entries": composed.provenance_entries,
        });
    }
    Ok((composed.records, metadata))
}

/// `sync._string_list`.
fn string_list(value: Option<&Json>) -> Result<Vec<String>> {
    let Some(Json::Array(items)) = value else {
        return Ok(vec![]);
    };
    let mut output: Vec<String> = Vec::new();
    for item in items {
        let item = py(item)?;
        if matches!(item, Value::Null) {
            continue;
        }
        let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&py_str(&item)));
        if !normalized.is_empty() && !output.contains(&normalized) {
            output.push(normalized);
        }
    }
    Ok(output)
}

fn put(outcome: &mut Value, key: &str, value: Value) {
    set(outcome, key, value);
}

/// `refresh_wallet_output_confirmations`: when unchanged histories let UTXO
/// membership be reused, advance the confirmation counts of this source's
/// confirmed outputs from the proven anchor height. No output is observed
/// anew and no request is made; a missing or invalid anchor changes nothing.
fn refresh_output_confirmations(
    conn: &Connection,
    request: &Request<'_>,
    fetch: &Json,
    meta: &Json,
    backend: &Json,
    sync_state: &Json,
) -> Result<()> {
    let Some(height) = proven_tip_height(fetch, meta) else {
        return Ok(());
    };
    let text = |value: &Json| -> Result<String> { Ok(or_text(&py(value)?)) };
    refresh_confirmations(
        conn,
        &OutputSource {
            profile_id: request.profile_id,
            wallet_id: request.wallet_id,
            backend_name: &text(&backend["name"])?,
            backend_kind: &text(&backend["kind"])?,
            chain: &text(&sync_state["chain"])?,
            network: &text(&sync_state["network"])?,
        },
        height,
    )
}

/// The anchor height of an Esplora or Electrum checkpoint: a JSON integer,
/// never a boolean, float or negative number.
pub(crate) fn proven_tip_height(fetch: &Json, meta: &Json) -> Option<i64> {
    let kind = fetch["kind"]
        .as_str()
        .map(|k| kassiber_pytext::strip(k).to_lowercase())
        .unwrap_or_default();
    let kind = match kind.as_str() {
        "mempool" | "liquid-esplora" => "esplora",
        kind => kind,
    };
    if !matches!(kind, "esplora" | "electrum") {
        return None;
    }
    meta["freshness_checkpoint"][format!("{kind}_tip_height")]
        .as_i64()
        .filter(|h| *h >= 0)
}

pub(crate) struct OutputSource<'a> {
    pub profile_id: &'a str,
    pub wallet_id: &'a str,
    pub backend_name: &'a str,
    pub backend_kind: &'a str,
    pub chain: &'a str,
    pub network: &'a str,
}

pub(crate) fn refresh_confirmations(
    conn: &Connection,
    source: &OutputSource<'_>,
    height: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE wallet_utxos SET confirmations = ?1 - block_height + 1
         WHERE profile_id = ?2 AND wallet_id = ?3
           AND backend_name = ?4 AND backend_kind = ?5 AND chain = ?6 AND network = ?7
           AND spent_at IS NULL AND confirmation_status = 'confirmed'
           AND block_height > 0 AND block_height <= ?1",
        kassiber_store::rusqlite::params![
            height,
            source.profile_id,
            source.wallet_id,
            source.backend_name,
            source.backend_kind,
            source.chain,
            source.network,
        ],
    )
    .map_err(database_error)?;
    Ok(())
}

fn take(meta: &mut Json, key: &str) -> Option<Json> {
    meta.as_object_mut().and_then(|object| object.remove(key))
}

/// `sync._wallet_negative_balance_events` for one wallet.
pub(crate) fn negative_balance_events(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
) -> Result<Vec<Value>> {
    let found = rows(
        conn,
        "SELECT id, external_id, occurred_at, confirmed_at, raw_json, direction, asset, amount, fee, created_at
         FROM transactions WHERE profile_id = ? AND wallet_id = ? AND excluded = 0",
        [profile_id, wallet_id],
    )?;
    super::balance::events(&found)
}

/// `handlers._prospective_negative_balance_events` for a dependency fetch:
/// stored rows overlaid with the prepared records, before any write. The
/// overlay is keyed by (txid, asset), the identity the apply path writes, so
/// one asset's record of a multi-asset Liquid transaction never hides another.
pub(crate) fn prospective_negative_events(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    fetch: &Json,
    updates: &[Json],
    backend_name: &str,
) -> Result<Vec<Value>> {
    let mut stored = rows(
        conn,
        "SELECT id, external_id, occurred_at, confirmed_at, raw_json, direction, asset, amount, fee, created_at
         FROM transactions WHERE profile_id = ? AND wallet_id = ? AND excluded = 0",
        [profile_id, wallet_id],
    )?;
    let key = |value: &Value| {
        if truthy(value) {
            kassiber_pytext::lower(kassiber_pytext::strip(&py_str(value)))
        } else {
            String::new()
        }
    };
    let mut retracted = Vec::new();
    let meta = &fetch["adapter_meta"];
    for list in [
        &meta["bitcoinrpc_retracted_txids"],
        &meta["observer_retracted_external_ids"],
    ]
    .into_iter()
    .chain(
        updates
            .iter()
            .map(|u| &u["update"]["facts"]["retracted_external_ids"]),
    ) {
        for value in list.as_array().into_iter().flatten() {
            let value = key(&py(value)?);
            if !value.is_empty() {
                retracted.push(value);
            }
        }
    }
    if !retracted.is_empty() {
        stored.retain(|row| !retracted.contains(&key(field(row, "external_id"))));
    }
    // `str(asset or "")`.
    let asset = |value: &Value| {
        if truthy(value) {
            py_str(value)
        } else {
            String::new()
        }
    };
    let mut by_external_id = std::collections::BTreeMap::new();
    for (index, row) in stored.iter().enumerate() {
        let id = (key(field(row, "external_id")), asset(field(row, "asset")));
        if !kassiber_pytext::strip(&or_text(field(row, "external_id"))).is_empty() {
            by_external_id.insert(id, index);
        }
    }
    let candidates = fetch["normalized_records"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(updates.iter().flat_map(|u| {
            u["update"]["facts"]["transaction_records"]
                .as_array()
                .into_iter()
                .flatten()
        }));
    let label = format!("backend:{backend_name}");
    for (index, record) in candidates.enumerate() {
        let normalized = insert::normalize(&py(record)?, &label)?;
        let external_id = key(field(&normalized, "external_id"));
        let mut candidate = dict([
            (
                "id",
                s(format!(
                    "prefetched:{}",
                    if external_id.is_empty() {
                        index.to_string()
                    } else {
                        external_id.clone()
                    }
                )),
            ),
            ("external_id", field(&normalized, "external_id").clone()),
            ("occurred_at", field(&normalized, "occurred_at").clone()),
            ("confirmed_at", field(&normalized, "confirmed_at").clone()),
            ("raw_json", field(&normalized, "raw_json").clone()),
            ("direction", field(&normalized, "direction").clone()),
            ("asset", field(&normalized, "asset").clone()),
            (
                "amount",
                Value::Int(insert::btc_to_msat(&insert::decimal(&normalized, "amount"))?.into()),
            ),
            (
                "fee",
                Value::Int(insert::btc_to_msat(&insert::decimal(&normalized, "fee"))?.into()),
            ),
            ("created_at", field(&normalized, "occurred_at").clone()),
        ]);
        let overlay = (external_id.clone(), asset(field(&normalized, "asset")));
        match (!external_id.is_empty())
            .then(|| by_external_id.get(&overlay).copied())
            .flatten()
        {
            Some(existing) => {
                set(&mut candidate, "id", field(&stored[existing], "id").clone());
                set(
                    &mut candidate,
                    "created_at",
                    field(&stored[existing], "created_at").clone(),
                );
                stored[existing] = candidate;
            }
            None => {
                stored.push(candidate);
                if !external_id.is_empty() {
                    by_external_id.insert(overlay, stored.len() - 1);
                }
            }
        }
    }
    super::balance::events(&stored)
}

/// Apply one prepared fetch atomically. `stage` runs after each Python apply
/// stage and before entry; an error from it (cancellation, injected failure)
/// rolls back every write, as does any other failure or a Rust unwind.
pub fn apply(
    conn: &mut Connection,
    request: &Request<'_>,
    sources: &mut dyn Sources,
    mut stage: impl FnMut(&str) -> Result<()>,
) -> Result<Vec<Value>> {
    apply_mode(
        conn,
        request,
        sources,
        |_, name| stage(name),
        AfterApply::Refuse,
    )
}

/// `rescan` performs Python's after-apply rescan fetch for the given
/// published checkpoint; without it such a deficit refuses.
pub(crate) fn apply_with_rescan(
    conn: &mut Connection,
    request: &Request<'_>,
    sources: &mut dyn Sources,
    mut stage: impl FnMut(&str) -> Result<()>,
    rescan: Rescan<'_>,
) -> Result<Vec<Value>> {
    apply_mode(
        conn,
        request,
        sources,
        |_, name| stage(name),
        AfterApply::Rescan(rescan),
    )
}

pub(crate) fn apply_job(
    conn: &mut Connection,
    request: &Request<'_>,
    sources: &mut dyn Sources,
    stage: impl FnMut(&Connection, &str) -> Result<()>,
    rescan: Rescan<'_>,
) -> Result<Vec<Value>> {
    apply_mode(conn, request, sources, stage, AfterApply::Rescan(rescan))
}

/// Fetches the after-apply rescan inside the publication's savepoint, from
/// the published checkpoint.
pub(crate) type Rescan<'a> = &'a mut dyn FnMut(&Connection, &Json) -> Result<Json>;

/// What a deficit left after applying a compatibility fetch does.
enum AfterApply<'a> {
    /// Python's in-savepoint rescan.
    Rescan(Rescan<'a>),
    /// No rescan can be fetched here: refuse and roll back.
    Refuse,
    /// The rescan pass itself (`_allow_negative_balance_rescan=False`).
    Skip,
}

fn apply_mode(
    conn: &mut Connection,
    request: &Request<'_>,
    sources: &mut dyn Sources,
    mut stage: impl FnMut(&Connection, &str) -> Result<()>,
    rescan: AfterApply<'_>,
) -> Result<Vec<Value>> {
    let savepoint = kassiber_store::write_savepoint(conn).map_err(database_error)?;
    let mut stage = |name: &str| stage(&savepoint, name);
    stage("begin")?;
    let clock = Clock(RefCell::new(sources), std::time::Instant::now());
    let results = publish(&savepoint, request, &clock, &mut stage, rescan)?;
    stage("commit")?;
    savepoint.commit().map_err(database_error)?;
    Ok(results)
}

/// What one pass of `sync_wallet_from_backend` produced.
enum Applied {
    Skipped(Value),
    Synced { label: String, outcome: Value },
}

fn publish(
    conn: &Connection,
    request: &Request<'_>,
    clock: &Clock<'_>,
    stage: &mut dyn FnMut(&str) -> Result<()>,
    rescan: AfterApply<'_>,
) -> Result<Vec<Value>> {
    let (label, mut outcome) = match apply_pass(conn, request, clock, stage, rescan)? {
        Applied::Skipped(skipped) => return Ok(vec![skipped]),
        Applied::Synced { label, outcome } => (label, outcome),
    };
    // `_merge_wallet_enrichments`, inside the wallet's savepoint: BTCPay
    // provenance routes first (Greenfield reads under their own grants),
    // then Bull Bitcoin export routes.
    let fetch = request.fetch;
    let captured: Json = serde_json::from_str(request.captured_config)
        .map_err(|_| invalid("Wallet configuration must be JSON"))?;
    match request.btcpay {
        Some(ctx) => {
            let workspace_id = first(
                conn,
                "SELECT workspace_id FROM profiles WHERE id = ?",
                [request.profile_id],
            )?
            .map(|row| py_str(field(&row, "workspace_id")))
            .ok_or_else(|| AppError::new("not_found", "Profile not found"))?;
            let wallet = first(
                conn,
                "SELECT * FROM wallets WHERE id = ? AND profile_id = ?",
                [request.wallet_id, request.profile_id],
            )?
            .ok_or_else(|| {
                AppError::new(
                    "not_found",
                    "Sync wallet does not belong to the selected book",
                )
            })?;
            let scope = crate::sync::btcpay::Scope {
                workspace_id: &workspace_id,
                profile_id: request.profile_id,
                wallet: &wallet,
            };
            // `wallet["_freshness_checkpoint"]`: the checkpoint the fetch
            // started from (a repair keeps the original one aside).
            let checkpoint = if fetch["wallet_checkpoint"].is_object() {
                &fetch["wallet_checkpoint"]
            } else {
                &fetch["sync_state"]["checkpoint"]
            };
            let checkpoint = py(checkpoint)?;
            if let Some(enriched) =
                crate::sync::btcpay::enrichment(conn, ctx, &scope, &captured, &checkpoint)?
            {
                put(&mut outcome, "btcpay_provenance", enriched);
            }
        }
        None if crate::sync::sources::has_btcpay_provenance(
            &crate::sync::sources::config_from(request.captured_config),
        ) =>
        {
            return Err(fallback("btcpay_provenance"));
        }
        None => {}
    }
    if let Some(enriched) = crate::sync::sources::bull_enrichment(
        conn,
        request.profile_id,
        request.wallet_id,
        &crate::sync::sources::config_from(request.captured_config),
    )? {
        put(&mut outcome, "bullbitcoin_wallet_exports", enriched);
    }
    let mut result = dict([("wallet", s(label.clone())), ("status", s("synced"))]);
    if let Value::Dict(pairs) = outcome {
        for (key, value) in pairs {
            if let Key::Str(name) = &key {
                set(&mut result, &py_str(&Value::Str(name.clone())), value);
            }
        }
    }
    let synced_at = clock.now();
    let checkpoint = get(&result, "freshness_checkpoint")
        .filter(|v| matches!(v, Value::Dict(_)))
        .cloned();
    let checkpoint_json = checkpoint.as_ref().map(json).transpose()?;
    let updated_at = if checkpoint.is_some() {
        clock.now()
    } else {
        String::new()
    };
    let degraded = get(&result, "silent_payment_degraded_reason")
        .filter(|v| !matches!(v, Value::Null))
        .map(py_str);
    super::mark_synced(
        conn,
        &super::Synced {
            profile_id: request.profile_id,
            wallet_id: request.wallet_id,
            wallet_label: &label,
            checkpoint: checkpoint_json.as_ref(),
            partial_success: get(&result, "partial_success").is_some_and(truthy),
            degraded_reason: degraded.as_deref(),
            blocking_reports: get(&result, "blocking_reports").is_some_and(truthy),
            timestamp: &synced_at,
            updated_at: &updated_at,
        },
    )?;
    stage("freshness_checkpoint")?;
    Ok(vec![result])
}

/// One `sync_wallet_from_backend` pass up to its outcome.
fn apply_pass(
    conn: &Connection,
    request: &Request<'_>,
    clock: &Clock<'_>,
    stage: &mut dyn FnMut(&str) -> Result<()>,
    rescan: AfterApply<'_>,
) -> Result<Applied> {
    let fetch = request.fetch;
    let profile = first(
        conn,
        "SELECT * FROM profiles WHERE id = ?",
        [request.profile_id],
    )?
    .ok_or_else(|| AppError::new("not_found", "Profile not found"))?;
    let wallet = first(
        conn,
        "SELECT * FROM wallets WHERE id = ? AND profile_id = ?",
        [request.wallet_id, request.profile_id],
    )?
    .ok_or_else(|| {
        AppError::new(
            "not_found",
            "Sync wallet does not belong to the selected book",
        )
    })?;
    let config_json = or_text(field(&wallet, "config_json"));
    if config_json != request.captured_config {
        return Err(AppError::new(
            "stale_context",
            "Wallet configuration changed during refresh",
        )
        .with_hint("Retry the wallet refresh against the current configuration.")
        .retryable(true));
    }
    require_supported(&wallet)?;
    let kind = or_text(field(&wallet, "kind"));
    let label = py_str(field(&wallet, "label"));
    let (routed, _) = network_scope(conn, request.profile_id, &kind, &config_json)?;
    let (routed, environment) = network_scope(conn, request.profile_id, &kind, &routed)?;
    if py(&fetch["book_environment_id"])? != environment {
        return Err(AppError::new(
            "stale_context",
            "Book network changed during refresh; retry in the selected scope",
        )
        .retryable(true));
    }
    let sync_state = &fetch["sync_state"];
    if sync_state.is_object() {
        kassiber_setup::networks::guard_observation(
            conn,
            request.profile_id,
            &dict([
                ("chain", py(&sync_state["chain"])?),
                ("network", py(&sync_state["network"])?),
                ("wallet_config_json", s(routed.clone())),
            ]),
            "sync_publish",
        )?;
    }
    let updates = fetch["observer_updates"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let dependency_prepared = !updates.is_empty();
    if fetch["skip_outcome"].is_object() {
        return Ok(Applied::Skipped(py(&fetch["skip_outcome"])?));
    }
    let backend = &fetch["backend"];
    let backend_name = py_str(&py(&backend["name"])?);
    let mut records: Vec<Json> = fetch["normalized_records"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut meta = if fetch["adapter_meta"].is_object() {
        fetch["adapter_meta"].clone()
    } else {
        serde_json::json!({})
    };
    // The coordinator runs Python's prefetch repair before this savepoint; a
    // fetch without its marker cannot have been checked.
    let repaired = fetch["adapter_meta"]["_prepared_negative_balance_rescan"].is_object();
    if dependency_prepared
        && !repaired
        && fetch["sync_state"].is_object()
        && !prospective_negative_events(
            conn,
            request.profile_id,
            request.wallet_id,
            fetch,
            &updates,
            &backend_name,
        )?
        .is_empty()
    {
        // Python widens to a forced repair scan before its savepoint opens.
        return Err(fallback("negative_balance_repair"));
    }
    if dependency_prepared {
        if !records.is_empty() || meta.get("utxos").is_some_and(|v| !v.is_null()) {
            return Err(AppError::new(
                "observer_projection_conflict",
                "A wallet refresh cannot apply dependency and compatibility projections together",
            )
            .with_hint("Select exactly one observer route before contacting the backend.")
            .with_details(dict([("conflict_kind", s("mixed_observer_routes"))])));
        }
        let (composed, composed_meta) =
            apply_observer_updates(conn, &profile, &updates, &meta, &backend_name, clock)?;
        if !composed.is_empty() {
            records = composed;
        }
        meta = composed_meta;
    }
    let authoritative = dependency_prepared || fetch["authoritative_chain_observer"] == true;
    // A silent-payment scan that is incomplete, untimestamped or from a
    // custom backend inserts without observer authority (R4).
    let silent = fetch["sync_state"]["descriptor_kind"] == "silent-payment";
    if !authoritative && !silent {
        return Err(fallback("non_authoritative_route"));
    }
    let mut observation = take(&mut meta, "_chain_observation_provenance").filter(|v| !v.is_null());
    if observation.is_none() && authoritative && !dependency_prepared && !records.is_empty() {
        let route = kassiber_pytext::lower(kassiber_pytext::strip(&{
            let kind = py_str(&py(&fetch["kind"])?);
            if !kind.is_empty() && fetch["kind"].as_str().is_some_and(|k| !k.is_empty()) {
                kind
            } else if backend["kind"].as_str().is_some_and(|k| !k.is_empty()) {
                py_str(&py(&backend["kind"])?)
            } else {
                "adapter".into()
            }
        }));
        let identity = format!("adapter:{route}:{}", request.wallet_id);
        let python_records: Vec<Value> = records.iter().map(py).collect::<Result<_>>()?;
        let entries = provenance::entries_for_facts(
            &[provenance::ObserverRecords {
                id: &identity,
                kind: &route,
                records: &python_records,
            }],
            Some(&python_records),
        );
        if !entries.is_empty() {
            observation = Some(serde_json::json!({
                "application_revision": clock.id(),
                "entries": json(&Value::List(entries))?,
            }));
        }
    }
    let prepared_rescan =
        take(&mut meta, "_prepared_negative_balance_rescan").filter(|v| !v.is_null());
    recheck_overlap(conn, request, &wallet, &routed, sync_state)?;
    stage("observer_persistence")?;
    let observed_utxos = take(&mut meta, "utxos");
    let mut retracted = string_list(take(&mut meta, "bitcoinrpc_retracted_txids").as_ref())?;
    for value in string_list(take(&mut meta, "observer_retracted_external_ids").as_ref())? {
        if !retracted.contains(&value) {
            retracted.push(value);
        }
    }
    let source_label = format!("backend:{backend_name}");
    let retraction = if retracted.is_empty() {
        None
    } else {
        Some(super::retract_records(
            conn,
            request.profile_id,
            request.wallet_id,
            &label,
            &retracted,
            &source_label,
        )?)
    };
    stage("retractions")?;
    let python_records: Vec<Value> = records.iter().map(py).collect::<Result<_>>()?;
    let workspace_id = py_str(field(&profile, "workspace_id"));
    let fiat = or_text(field(&profile, "fiat_currency"));
    let mut outcome = if authoritative {
        insert::insert_records(
            conn,
            &insert::Scope {
                workspace_id: &workspace_id,
                profile_id: request.profile_id,
                profile_fiat_currency: &fiat,
                wallet_id: request.wallet_id,
                wallet_label: &label,
            },
            &python_records,
            &source_label,
            &mut || clock.id(),
            &mut || clock.now(),
        )?
    } else {
        // `hooks.insert_records(...)` without `authoritative_chain_observer`.
        crate::file_import::import_records(
            conn,
            &crate::file_import::RecordImport {
                profile_id: request.profile_id,
                wallet_id: request.wallet_id,
                label: &source_label,
                match_existing_only: false,
                report_updates: false,
                btcpay: false,
                settlement_ids: &[],
            },
            &crate::file_import::record_rows(python_records.clone()),
        )?
    };
    let resolved = match get(&outcome, "_observer_resolved_records") {
        Some(Value::List(items)) => items.clone(),
        _ => vec![],
    };
    if let Value::Dict(pairs) = &mut outcome {
        pairs.retain(|(key, _)| !matches!(key, Key::Str(k) if k.as_str() == Some("_observer_resolved_records")));
    }
    if let Some(observation) = observation.filter(|v| v.is_object()) {
        let entries: Vec<Value> = match py(&observation["entries"])? {
            Value::List(items) => items,
            _ => vec![],
        };
        let current = provenance::journal_current(conn, request.profile_id)?;
        let previous = if current {
            Some(provenance::accounting_inputs(
                conn,
                request.profile_id,
                &entries,
            )?)
        } else {
            None
        };
        provenance::persist(
            conn,
            &provenance::Target {
                workspace_id: &workspace_id,
                profile_id: request.profile_id,
                wallet_id: request.wallet_id,
                application_revision: &py_str(&py(&observation["application_revision"])?),
                chain: &py_str(&py(&sync_state["chain"])?),
                network: &py_str(&py(&sync_state["network"])?),
            },
            &entries,
            &resolved,
            &mut || clock.now(),
        )?;
        if let Some(previous) = previous {
            if !previous.same(&provenance::accounting_inputs(
                conn,
                request.profile_id,
                &entries,
            )?) {
                crate::invalidate_journals(conn, request.profile_id)?;
                put(&mut outcome, "journal_invalidated", Value::Bool(true));
            }
        }
    }
    if sync_state.is_object() {
        let impacts = reports::record_wallet_changes(
            conn,
            request.profile_id,
            request.wallet_id,
            &retracted,
            &mut || clock.id(),
            &mut || clock.now(),
        )?;
        put(
            &mut outcome,
            "saved_report_impacts",
            Value::Int(impacts.into()),
        );
    }
    stage("transaction_insertion")?;
    if let Some(utxos) = &observed_utxos {
        let outputs: Vec<Value> = match py(utxos)? {
            Value::List(items) => items,
            _ => return Err(invalid("Observed outputs must be a list")),
        };
        let wallet_config = Value::from(routed.clone());
        let summary = inventory::update(
            conn,
            &inventory::Source {
                workspace_id: &workspace_id,
                profile_id: request.profile_id,
                wallet_id: request.wallet_id,
                wallet_config_json: &wallet_config,
                backend_name: &or_text(&py(&backend["name"])?),
                backend_kind: &or_text(&py(&backend["kind"])?),
                chain: &or_text(&py(&sync_state["chain"])?),
                network: &or_text(&py(&sync_state["network"])?),
            },
            &outputs,
            &clock.now(),
        )?;
        put(&mut outcome, "output_inventory", summary);
    } else if meta["utxos_skipped_unchanged"] == true {
        refresh_output_confirmations(conn, request, fetch, &meta, backend, sync_state)?;
    }
    stage("output_inventory")?;
    stage("derivation_coverage")?;
    describe(
        &mut outcome,
        fetch,
        &meta,
        &records,
        retracted,
        retraction,
        observed_utxos.is_some(),
        clock.1.elapsed(),
    )?;
    if let Some(prepared) = prepared_rescan.filter(Json::is_object) {
        let remaining = negative_balance_events(conn, request.profile_id, request.wallet_id)?;
        put(
            &mut outcome,
            "negative_balance_rescan",
            rescan_outcome(&prepared, remaining)?,
        );
    } else if !dependency_prepared && !matches!(rescan, AfterApply::Skip) {
        let events = negative_balance_events(conn, request.profile_id, request.wallet_id)?;
        if !events.is_empty() {
            // Python re-syncs inside this savepoint, forced, at the widened
            // gap, from the checkpoint this pass published.
            let AfterApply::Rescan(rescan) = rescan else {
                return Err(fallback("negative_balance_repair"));
            };
            let checkpoint = meta
                .get("freshness_checkpoint")
                .unwrap_or(&sync_state["checkpoint"]);
            let repair = rescan(conn, &crate::acquisition::full_scan_checkpoint(checkpoint))?;
            let nested = Request {
                fetch: &repair,
                ..*request
            };
            let mut repaired = match apply_pass(conn, &nested, clock, stage, AfterApply::Skip)? {
                Applied::Skipped(skipped) => skipped,
                Applied::Synced { outcome, .. } => outcome,
            };
            let remaining = negative_balance_events(conn, request.profile_id, request.wallet_id)?;
            // The repair replaces the outcome; carry forward the first pass's
            // projection retirement and superseded observations.
            if get(&outcome, "journal_invalidated").is_some_and(truthy) {
                put(&mut repaired, "journal_invalidated", Value::Bool(true));
            }
            if let Some(superseded) = get(&outcome, "observer_superseded").filter(|v| truthy(v)) {
                let count = |value: Option<&Value>| -> Result<kassiber_pyjson::BigInt> {
                    match value {
                        Some(Value::Int(n)) => Ok(n.clone()),
                        Some(Value::Bool(b)) => Ok(u8::from(*b).into()),
                        None | Some(Value::Null) => Ok(0.into()),
                        Some(_) => Err(invalid("Superseded counts must be integers")),
                    }
                };
                let total =
                    count(Some(superseded))? + count(get(&repaired, "observer_superseded"))?;
                put(&mut repaired, "observer_superseded", Value::Int(total));
                let mut records = Vec::new();
                for key in [&outcome, &repaired] {
                    if let Some(Value::List(items)) = get(key, "observer_superseded_records") {
                        records.extend(items.iter().cloned());
                    }
                }
                put(
                    &mut repaired,
                    "observer_superseded_records",
                    Value::List(records),
                );
            }
            let gap = |value: Option<u32>| value.map_or(Value::Null, |v| Value::Int(v.into()));
            let original = sync_state["gap_limit"]
                .as_u64()
                .and_then(|g| u32::try_from(g).ok());
            let resolved = remaining.is_empty();
            put(
                &mut repaired,
                "negative_balance_rescan",
                dict([
                    ("triggered", Value::Bool(true)),
                    ("resolved", Value::Bool(resolved)),
                    ("initial_negative_events", Value::List(events)),
                    ("remaining_negative_events", Value::List(remaining)),
                    ("original_gap_limit", gap(original)),
                    (
                        "rescan_gap_limit",
                        gap(original.map(crate::fetch::rescan_gap_limit)),
                    ),
                ]),
            );
            outcome = repaired;
        }
    }
    Ok(Applied::Synced { label, outcome })
}

/// The public outcome fields `sync_wallet_from_backend` adds after its writes.
#[allow(clippy::too_many_arguments)]
fn describe(
    outcome: &mut Value,
    fetch: &Json,
    meta: &Json,
    records: &[Json],
    retracted: Vec<String>,
    retraction: Option<Json>,
    utxos_refreshed: bool,
    published: std::time::Duration,
) -> Result<()> {
    let state = &fetch["sync_state"];
    let backend = &fetch["backend"];
    put(outcome, "backend", py(&backend["name"])?);
    put(outcome, "backend_kind", py(&fetch["kind"])?);
    put(
        outcome,
        "backend_url",
        s(redact_url(backend["url"].as_str().unwrap_or(""))),
    );
    put(outcome, "chain", py(&state["chain"])?);
    put(outcome, "network", py(&state["network"])?);
    // `gap_limit` is null only without a descriptor plan; a silent-payment
    // plan reports its target addresses like an address list.
    let addresses = state["gap_limit"].is_null();
    let mode = if state["descriptor_kind"] == "silent-payment" {
        "silent_payment"
    } else if addresses {
        "addresses"
    } else {
        "descriptor"
    };
    put(outcome, "sync_mode", s(mode));
    put(outcome, "target_count", py(&state["target_count"])?);
    put(outcome, "records_fetched", Value::Int(records.len().into()));
    let mut meta = meta.clone();
    let reused = take(&mut meta, "observer_records_reused")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if reused != 0 {
        put(outcome, "records_reused", Value::Int(reused.into()));
        for key in ["unchanged", "skipped"] {
            let current = match get(outcome, key) {
                Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(0),
                _ => 0,
            };
            put(outcome, key, Value::Int((current + reused).into()));
        }
    }
    if get(outcome, "updated").is_none() {
        if let Some(Value::List(items)) = get(outcome, "updated_records") {
            let count = items.len();
            put(outcome, "updated", Value::Int(count.into()));
        }
    }
    if !retracted.is_empty() {
        put(
            outcome,
            "bitcoinrpc_retracted_txids",
            Value::List(retracted.into_iter().map(Value::from).collect()),
        );
    }
    if let Some(retraction) = retraction {
        put(outcome, "retracted", py(&retraction["retracted"])?);
        put(
            outcome,
            "retracted_records",
            py(&retraction["retracted_records"])?,
        );
        let invalidated = get(outcome, "journal_invalidated").is_some_and(truthy)
            || retraction["journal_invalidated"] == true;
        put(outcome, "journal_invalidated", Value::Bool(invalidated));
    }
    if fetch["force_full"] == true {
        put(outcome, "force_full", Value::Bool(true));
    }
    if addresses {
        let listed: Vec<&str> = state["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|target| target["address"].as_str().filter(|a| !a.is_empty()))
            .collect();
        put(outcome, "addresses", s(listed.join(",")));
    } else {
        put(outcome, "gap_limit", py(&state["gap_limit"])?);
    }
    if state["policy_asset_id"]
        .as_str()
        .is_some_and(|v| !v.is_empty())
    {
        put(outcome, "policy_asset", py(&state["policy_asset_id"])?);
    }
    if let Value::Dict(pairs) = py(&meta)? {
        for (key, value) in pairs {
            if let Key::Str(name) = &key {
                put(outcome, &py_str(&Value::Str(name.clone())), value);
            }
        }
    }
    let count = |key: &str| match get(outcome, key) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(0),
        _ => 0,
    };
    let (changed, unchanged) = (count("scripts_changed"), count("scripts_unchanged"));
    if changed != 0 || unchanged != 0 {
        put(
            outcome,
            "scripts_checked",
            Value::Int((changed + unchanged).into()),
        );
    }
    put(outcome, "utxos_refreshed", Value::Bool(utxos_refreshed));
    // Python measures from the fetch's start to here: the fetch's own time
    // plus this publication's so far.
    let fetched = fetch["elapsed_ms"].as_u64().unwrap_or(0);
    let published = u64::try_from(published.as_millis()).unwrap_or(u64::MAX);
    put(
        outcome,
        "elapsed_ms",
        Value::Int(fetched.saturating_add(published).into()),
    );
    Ok(())
}

/// `backends.redact_backend_url` for the `scheme://[user@]host[:port]/path` forms
/// Kassiber backends use; query, fragment and credentials are dropped.
pub(crate) fn redact_url(url: &str) -> String {
    let value = kassiber_pytext::strip(url);
    let Some((scheme, rest)) = value.split_once("://") else {
        return value.into();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (netloc, tail) = rest.split_at(end);
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..path_end];
    if netloc.is_empty() {
        return value.into();
    }
    let (userinfo, host) = match netloc.rsplit_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, netloc),
    };
    let host = host.to_ascii_lowercase();
    match userinfo {
        Some(_) => format!("{}://<redacted>@{host}{path}", scheme.to_ascii_lowercase()),
        None => format!("{}://{host}{path}", scheme.to_ascii_lowercase()),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod confirmation_tests {
    use super::*;

    /// Python's `refresh_wallet_output_confirmations` and its tip selection
    /// (round 5, 7e8c323b9), on every output scope the vectors seed.
    #[test]
    fn reused_utxo_confirmations_match_python() {
        let vectors: Json = serde_json::from_str(include_str!(
            "../../tests/vectors/python_utxo_confirmations.json"
        ))
        .unwrap();
        let manifest: Json =
            serde_json::from_str(include_str!("../../../store/fixtures/current-schema.json"))
                .unwrap();
        let cases = vectors["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 35);
        for case in cases {
            let conn = Connection::open_in_memory().unwrap();
            for kind in ["table", "index", "view", "trigger"] {
                for object in manifest["objects"].as_array().unwrap() {
                    if object[0] == kind && object[3].is_string() {
                        conn.execute_batch(object[3].as_str().unwrap()).unwrap();
                    }
                }
            }
            conn.execute_batch(vectors["seed"].as_str().unwrap())
                .unwrap();
            for o in vectors["outputs"].as_array().unwrap() {
                conn.execute(
                    "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,backend_name,backend_kind,chain,network,
                     asset,amount,txid,vout,outpoint,confirmation_status,confirmations,block_height,first_seen_at,last_seen_at,spent_at)
                     VALUES(?1,'w','p',?2,?3,?4,?5,?6,'BTC',1000,?1,0,?1||':0',?7,?10,?8,'fixed','fixed',?9)",
                    kassiber_store::rusqlite::params![
                        o[0].as_str(), o[1].as_str(), o[2].as_str(), o[3].as_str(), o[4].as_str(),
                        o[5].as_str(), o[6].as_str(), o[7].as_i64(), o[8].as_str(), o[9].as_i64()
                    ],
                )
                .unwrap();
            }
            let fetch = serde_json::json!({"kind": case["fetch_kind"]});
            let meta = serde_json::json!({"freshness_checkpoint": case["checkpoint"]});
            if let Some(height) = proven_tip_height(&fetch, &meta) {
                let source = OutputSource {
                    profile_id: "p",
                    wallet_id: "wallet",
                    backend_name: "fixture",
                    backend_kind: case["backend_kind"].as_str().unwrap(),
                    chain: "bitcoin",
                    network: "main",
                };
                refresh_confirmations(&conn, &source, height).unwrap();
            }
            let mut statement = conn
                .prepare("SELECT id, confirmations FROM wallet_utxos ORDER BY id")
                .unwrap();
            let actual: serde_json::Map<String, Json> = statement
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Json::from(r.get::<_, Option<i64>>(1)?),
                    ))
                })
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert_eq!(
                Json::Object(actual),
                case["confirmations"],
                "{}",
                case["name"]
            );
        }
    }
}
