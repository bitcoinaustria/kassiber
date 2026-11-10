//! Python's Bitcoin Core adapter (`bitcoinrpc_sync_adapter`): one watch-only
//! node wallet per Kassiber wallet, ranged descriptor or address imports with
//! a rescan, then the wallet's transaction details and outputs. Every request
//! goes through `CoreClient` under the fetch's grant; the node never receives
//! key material (`CoreClient` refuses it).
use super::*;
use crate::bitcoin_records::{self, sorted_json};
use kassiber_chain::CoreClient;
use kassiber_pydecimal::{Context as DecimalContext, Decimal, Rounding};
use serde_json::Map;
use sha2::{Digest, Sha256};
use std::time::Duration;

const ATTESTATION_VERSION: u64 = 1;
const GRAPH_NORMALIZATION_VERSION: u64 = 3;
const CONTEXT: DecimalContext = DecimalContext::RP2;

fn app(message: impl Into<String>) -> AppError {
    AppError::new("app_error", message)
}
fn unexpected(message: &str) -> AppError {
    AppError::new("bitcoinrpc_unexpected_response", message).retryable(true)
}

/// `backend_value`: the first non-blank value among `keys`.
fn backend_value(backend: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        backend
            .get(*key)
            .filter(|v| !v.is_null())
            .map(descriptors::text)
            .map(|v| kassiber_pytext::strip(&v).to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// `msat.dec`: `Decimal(str(value))`, with JSON floats spelled as Python's
/// `repr` and a default for `None` or `""`.
fn dec(value: &Value, default: &str) -> Result<Decimal> {
    let text = match value {
        Value::Null => default.to_owned(),
        Value::String(s) if s.is_empty() => default.to_owned(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
        Value::Number(n) => {
            let raw = n.to_string();
            if raw.contains(['.', 'e', 'E']) {
                let float: f64 = raw.parse().map_err(|_| invalid_decimal(value))?;
                kassiber_pyjson::dumps(&kassiber_pyjson::Value::Float(float), &Default::default())
                    .map_err(|_| invalid_decimal(value))?
            } else {
                raw
            }
        }
        other => other.to_string(),
    };
    text.parse().map_err(|_| invalid_decimal(value))
}
fn invalid_decimal(value: &Value) -> AppError {
    app(format!(
        "Invalid decimal value: {}",
        descriptors::text(value)
    ))
}
fn decimal_json(value: &Decimal) -> Value {
    json!(value.to_string())
}

/// `timestamp_to_iso(value, default)`.
fn iso(value: &Value) -> Result<Option<String>> {
    bitcoin_records::timestamp(value)
}

/// `sanitize_wallet_segment`.
fn segment(value: &str) -> String {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(value));
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "kassiber".into()
    } else {
        trimmed.into()
    }
}

fn digest(value: &Value) -> Result<String> {
    // `json.dumps(payload, sort_keys=True, separators=(",", ":"))`.
    let py = kassiber_pytext::loads(&value.to_string()).map_err(|_| unexpected("Invalid JSON"))?;
    let text = kassiber_pyjson::dumps(
        &py,
        &kassiber_pyjson::Options::new()
            .sort_keys(true)
            .separators(",", ":")
            .ensure_ascii(true),
    )
    .map_err(|_| unexpected("Invalid JSON"))?;
    Ok(Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// `_int_mapping`: non-negative integer values by string key.
fn int_mapping(value: &Value) -> BTreeMap<String, i64> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, raw)| {
            crate::scan::integer(raw)
                .and_then(|n| i64::try_from(n).ok())
                .filter(|n| *n >= 0)
                .map(|n| (key.clone(), n))
        })
        .collect()
}

/// `_highest_used_branch_index`.
fn highest_index(highest: &Value, branch: u32) -> Option<i64> {
    if !highest.is_object() {
        return None;
    }
    let value = highest.get(branch.to_string()).unwrap_or(&Value::Null);
    crate::scan::integer(value)
        .and_then(|n| i64::try_from(n).ok())
        .filter(|n| *n >= 0)
}

/// `_merge_highest_used`.
fn merge_highest(highest: &mut Map<String, Value>, target: &Value) {
    let branch = descriptors::text(target.get("branch_index").unwrap_or(&json!(0)));
    let index = if descriptors::truthy(&target["address_index"]) {
        crate::scan::integer(&target["address_index"])
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(0)
    } else {
        0
    };
    let previous = highest
        .get(&branch)
        .filter(|v| descriptors::truthy(v))
        .and_then(crate::scan::integer)
        .and_then(|n| i64::try_from(n).ok())
        .unwrap_or(-1);
    highest.insert(branch, json!(previous.max(index)));
}

/// `str(detail.get("category") or "").lower()`.
fn category(detail: &Value) -> String {
    if descriptors::truthy(&detail["category"]) {
        kassiber_pytext::lower(&descriptors::text(&detail["category"]))
    } else {
        String::new()
    }
}
/// `_bitcoinrpc_detail_is_retracted`.
fn retracted(detail: &Value) -> bool {
    category(detail) == "orphan"
        || crate::scan::integer(&detail["confirmations"]).is_some_and(|n| n < 0.into())
}
fn detail_txid(detail: &Value) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(&descriptors::text(&detail["txid"])))
}

/// The node, its long-running import client, and this fetch's grant.
pub(super) struct Node<'a> {
    client: CoreClient,
    import: CoreClient,
    consent: &'a Consent,
    deadline: Instant,
    control: &'a dyn Control,
    backend: &'a Value,
}

impl<'a> Node<'a> {
    pub(super) fn new(
        backend: &'a Value,
        transport: &Backend,
        gate: &Gate,
        version: &str,
        consent: &'a Consent,
        deadline: Instant,
        control: &'a dyn Control,
    ) -> Self {
        // `bitcoinrpc_import_timeout`: at least 30 minutes for a rescan.
        let import_timeout = transport.timeout().max(Duration::from_secs(1800));
        Self {
            client: CoreClient::new(transport.clone(), gate.clone(), version),
            import: CoreClient::new(
                transport.clone().with_timeout(import_timeout),
                gate.clone(),
                version,
            ),
            consent,
            deadline,
            control,
            backend,
        }
    }

    fn call(&self, method: &str, params: Value, wallet: Option<&str>) -> Result<Value> {
        self.control.check(self.deadline)?;
        self.client.call(
            method,
            Some(&params),
            wallet,
            Some(self.consent),
            self.deadline,
        )
    }

    fn name(&self) -> String {
        descriptors::text(&self.backend["name"])
    }

    /// `bitcoinrpc_ensure_watchonly_wallet_state`.
    fn wallet(&self, wallet_id: &str) -> Result<(String, bool)> {
        let name = backend_value(self.backend, &["wallet"]).unwrap_or_else(|| {
            let prefix = backend_value(self.backend, &["walletprefix", "wallet_prefix"])
                .unwrap_or_else(|| "kassiber".into());
            format!("{}-{}", segment(&prefix), segment(wallet_id))
        });
        let loaded = self.call("listwallets", json!([]), None)?;
        if loaded
            .as_array()
            .is_some_and(|names| names.contains(&json!(name)))
        {
            return Ok((name, false));
        }
        let load = match self.call("loadwallet", json!([name, true]), None) {
            Ok(_) => return Ok((name, false)),
            Err(error) => error,
        };
        for params in [
            json!([name, true, true, "", false, true, true]),
            json!([name, true, true, "", false, true]),
            json!([name, true, true]),
        ] {
            if self.call("createwallet", params, None).is_ok() {
                return Ok((name, true));
            }
        }
        Err(app(format!(
            "Unable to load or create Bitcoin Core wallet '{name}' for backend '{}'. Last load error: {}",
            self.name(),
            load.message
        )))
    }

    /// `bitcoinrpc_prune_coverage`.
    fn coverage(&self, birthday: i64) -> Result<Value> {
        let info = self.call("getblockchaininfo", json!([]), None)?;
        if !info.is_object() {
            return Err(unexpected(
                "Bitcoin Core getblockchaininfo returned an unexpected payload",
            ));
        }
        let pruned = descriptors::truthy(&info["pruned"]);
        let birthday_iso = if birthday > 0 {
            iso(&json!(birthday))?
        } else {
            None
        };
        let mut coverage = json!({"pruned": pruned, "pruneheight": null, "earliest_retained_at": null,
            "birthday": birthday_iso, "rescan_start_at": null, "rescan_start_height": null,
            "birthday_covered": if pruned {Value::Null} else {json!(true)},
            "rescan_possible": if pruned {Value::Null} else {json!(true)}});
        if !pruned {
            return Ok(coverage);
        }
        let Some(height) = crate::scan::integer(&info["pruneheight"]) else {
            return Ok(coverage);
        };
        coverage["pruneheight"] = serde_json::from_str(&height.to_string()).unwrap_or(Value::Null);
        let time = self
            .call("getblockhash", json!([coverage["pruneheight"]]), None)
            .and_then(|hash| self.call("getblockheader", json!([hash]), None))
            .ok()
            .filter(Value::is_object)
            .and_then(|header| {
                if descriptors::truthy(&header["time"]) {
                    crate::scan::integer(&header["time"]).and_then(|n| i64::try_from(n).ok())
                } else {
                    Some(0)
                }
            })
            .unwrap_or(0);
        if time > 0 {
            coverage["earliest_retained_at"] = json!(iso(&json!(time))?);
        }
        if birthday <= 0 {
            return Ok(coverage);
        }
        // Core starts a timestamp import up to two hours early.
        coverage["rescan_start_at"] = json!(iso(&json!((birthday - 7200).max(0)))?);
        Ok(coverage)
    }

    /// `bitcoinrpc_require_rescan_coverage`.
    fn require_coverage(&self, birthday: i64) -> Result<()> {
        let coverage = self.coverage(birthday)?;
        if coverage["pruned"] == true && birthday <= 0 {
            return Err(AppError::new(
                "bitcoinrpc_birthday_required",
                "A wallet birthday is required when Bitcoin Core is pruned",
            )
            .with_hint("Set the wallet's real first-use date, or use an unpruned Core node or archival backend.")
            .with_details(details(json!({"pruneheight": coverage["pruneheight"],
                "earliest_retained_at": coverage["earliest_retained_at"]}))));
        }
        Ok(())
    }

    /// `_raise_bitcoinrpc_import_failure`.
    fn import_failure(&self, birthday: i64, operation: &str, failure: &Value) -> AppError {
        // Core's result text is the server's: withhold what the request
        // carried (`_bitcoinrpc_credentials`).
        let value = |key: &str| match self.backend.get(key) {
            Some(Value::String(text)) => kassiber_pytext::strip(text).to_owned(),
            Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
            _ => String::new(),
        };
        let mut credentials = kassiber_credentials::backend_credentials(value);
        if let (Some(user), Some(password)) = (
            backend_value(self.backend, &["username", "rpcuser", "rpc_user"]),
            backend_value(self.backend, &["password", "rpcpassword", "rpc_password"]),
        ) {
            credentials.extend(kassiber_credentials::authorization_credentials(&format!(
                "Basic {}",
                kassiber_chain::encode_base64(format!("{user}:{password}").as_bytes())
            )));
        }
        let text = kassiber_credentials::withhold_credentials(
            &sorted_json(failure).unwrap_or_default(),
            credentials,
        );
        let normalized = text.to_lowercase();
        if normalized.contains("rescan")
            && (normalized.contains("fail") || normalized.contains("prun"))
        {
            let coverage = match self.coverage(birthday) {
                Ok(coverage) => coverage,
                Err(error) => return error,
            };
            let pruned = coverage["pruned"] == true;
            return AppError::new(
                if pruned {
                    "bitcoinrpc_pruned_below_birthday"
                } else {
                    "bitcoinrpc_rescan_failed"
                },
                "Bitcoin Core could not complete the required wallet history rescan",
            )
            .with_hint(if pruned {
                "Use an unpruned Core node or archival backend. Only change the birthday if the wallet genuinely had no earlier activity."
            } else {
                "Check Bitcoin Core's block data and retry the rescan."
            })
            .with_details(details(json!({"operation": operation, "birthday": coverage["birthday"],
                "rescan_start_at": coverage["rescan_start_at"], "pruneheight": coverage["pruneheight"],
                "earliest_retained_at": coverage["earliest_retained_at"]})))
            .retryable(!pruned);
        }
        app(format!("{operation} failed: {text}"))
    }

    fn import_into(&self, method: &str, params: Value, wallet: &str) -> Result<Value> {
        self.control.check(self.deadline)?;
        self.import.call(
            method,
            Some(&params),
            Some(wallet),
            Some(self.consent),
            self.deadline,
        )
    }
}

fn details(value: Value) -> kassiber_pyjson::Value {
    kassiber_pytext::loads(&value.to_string()).unwrap_or(kassiber_pyjson::Value::Null)
}
fn first_failure(results: &Value) -> Option<Value> {
    results
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| !descriptors::truthy(&r["success"]))
        .cloned()
}

/// What the fetch hands the adapter.
pub(super) struct Input<'a> {
    pub wallet_id: &'a str,
    pub config: &'a Value,
    pub plan: Option<&'a Plan>,
    pub targets: &'a [Value],
    pub checkpoint: &'a Value,
    pub chain: &'a str,
    pub network: &'a str,
}

/// `bitcoinrpc_sync_adapter`: the records and adapter metadata.
pub(super) fn sync(node: &Node<'_>, input: &Input<'_>) -> Result<(Vec<Value>, Value)> {
    let (wallet, created) = node.wallet(input.wallet_id)?;
    // A recreated node wallet has no history, whatever the checkpoint says.
    let checkpoint = if created {
        json!({})
    } else {
        input.checkpoint.clone()
    };
    let birthday = birthday(input.config)?;
    let addresses: Vec<&str> = input
        .targets
        .iter()
        .filter_map(|t| t["address"].as_str().filter(|a| !a.is_empty()))
        .collect();
    let fingerprint = match input.plan {
        Some(plan) => digest(&json!({"descriptor_fingerprint": plan.fingerprint,
            "gap_limit": plan.gap_limit,
            "branches": plan.branches.iter().map(|b| json!({"branch": b.index,
                "descriptor": b.selected_descriptor()})).collect::<Vec<_>>()}))?,
        None => {
            let unique: BTreeSet<&str> = addresses.iter().copied().collect();
            digest(&json!({"addresses": unique}))?
        }
    };
    let proxy = backend_value(node.backend, &["tor_proxy", "proxy"]).unwrap_or_default();
    let url = if descriptors::truthy(&node.backend["url"]) {
        descriptors::text(&node.backend["url"])
    } else {
        String::new()
    };
    let endpoint = digest(&json!({"kind": "bitcoinrpc", "url": url, "proxy_url": proxy}))?;
    let attestation = json!({"version": ATTESTATION_VERSION, "backend": node.name(),
        "backend_endpoint_fingerprint": endpoint, "core_wallet": wallet,
        "birthday": birthday, "target_fingerprint": fingerprint});
    let attested = checkpoint["bitcoinrpc_history_attestation"] == attestation;
    let mut targets = input.targets.to_vec();
    let mut range_ends = None;
    let imported = if let Some(plan) = input.plan {
        let (count, ends) = import_ranges(node, &wallet, plan, &checkpoint, birthday, !attested)?;
        targets = targets_for_ends(plan, &ends)?;
        range_ends = Some(ends);
        count
    } else {
        import_addresses(
            node,
            &wallet,
            input.wallet_id,
            &addresses,
            birthday,
            !attested,
        )?
    };
    let addresses: Vec<String> = targets
        .iter()
        .filter_map(|t| t["address"].as_str().filter(|a| !a.is_empty()))
        .map(str::to_owned)
        .collect();
    let (records, mut meta, cache) = records(node, &wallet, imported, &checkpoint, &targets)?;
    let outputs = outputs(node, &wallet, &addresses, &targets, input, cache)?;
    meta.insert("utxos".into(), json!(outputs));
    meta.insert("observer_route".into(), json!("bitcoin_core_rpc"));
    if created {
        meta.insert("core_wallet_created".into(), json!(true));
    }
    let mut highest = Map::new();
    if input.plan.is_some() {
        highest = checkpoint["highest_used"]
            .as_object()
            .filter(|h| !h.is_empty())
            .cloned()
            .unwrap_or_default();
        let found = meta
            .get("bitcoinrpc_highest_used")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (branch, index) in found {
            let index = crate::scan::integer(&index)
                .and_then(|n| i64::try_from(n).ok())
                .unwrap_or(0);
            // `_highest_used_branch_index(highest_used, branch)`.
            let previous = highest
                .get(&branch)
                .and_then(crate::scan::integer)
                .and_then(|n| i64::try_from(n).ok())
                .filter(|n| *n >= 0);
            if previous.is_none_or(|p| index > p) {
                highest.insert(branch, json!(index));
            }
        }
        for output in &outputs {
            merge_highest(&mut highest, output);
        }
        meta.insert("imported_descriptors".into(), json!(imported));
    }
    meta.remove("bitcoinrpc_highest_used");
    if meta
        .get("bitcoinrpc_last_block")
        .is_some_and(descriptors::truthy)
        || range_ends.is_some()
    {
        let mut next = checkpoint.as_object().cloned().unwrap_or_default();
        let batch = backend_value(node.backend, &["batch_size"])
            .and_then(|v| kassiber_pytext::parse_int(&v).ok())
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(100);
        next.insert(
            "backend".into(),
            json!({"name": node.backend["name"], "kind": "bitcoinrpc", "chain": input.chain,
                "network": input.network, "batch_size": batch}),
        );
        next.insert("bitcoinrpc_history_attestation".into(), attestation);
        if let Some(block) = meta
            .get("bitcoinrpc_last_block")
            .filter(|v| descriptors::truthy(v))
        {
            next.insert("bitcoinrpc_last_block".into(), block.clone());
        }
        if let Some(version) = meta
            .get("bitcoinrpc_graph_normalization_version")
            .filter(|v| descriptors::truthy(v))
        {
            next.insert(
                "bitcoinrpc_graph_normalization_version".into(),
                version.clone(),
            );
        }
        if meta
            .get("bitcoinrpc_pending_maturity")
            .is_some_and(descriptors::truthy)
        {
            next.insert("bitcoinrpc_pending_maturity".into(), json!(true));
        } else {
            next.remove("bitcoinrpc_pending_maturity");
        }
        if let Some(ends) = range_ends {
            next.insert("bitcoinrpc_descriptor_range_ends".into(), json!(ends));
            next.insert(
                "highest_used".into(),
                Value::Object(
                    highest
                        .into_iter()
                        .collect::<BTreeMap<_, _>>()
                        .into_iter()
                        .collect(),
                ),
            );
        }
        meta.insert("freshness_checkpoint".into(), Value::Object(next));
    }
    Ok((records, Value::Object(meta)))
}

/// `iso_to_unix(config.get("birthday"))`: 0 without a birthday.
fn birthday(config: &Value) -> Result<i64> {
    let value = &config["birthday"];
    if value.is_null() || value.as_str() == Some("") {
        return Ok(0);
    }
    let raw = kassiber_pytext::strip(&descriptors::text(value)).to_owned();
    let invalid = || {
        AppError::new(
            "validation",
            format!(
                "Invalid occurred_at/date value '{}'",
                descriptors::text(value)
            ),
        )
        .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    };
    if !descriptors::truthy(value) {
        return Err(AppError::new("app_error", "Missing occurred_at/date value"));
    }
    let us = kassiber_chain::parse_iso_time(&raw).ok_or_else(invalid)?;
    Ok((us - 62_135_596_800_000_000).div_euclid(1_000_000))
}

/// `_bitcoinrpc_descriptor_end`.
fn descriptor_end(
    plan: &Plan,
    branch: &descriptors::Branch,
    highest: &Value,
    previous: Option<i64>,
) -> i64 {
    if !branch.ranged() {
        return 0;
    }
    let previous = previous.unwrap_or(-1);
    match highest_index(highest, branch.index) {
        None => 0.max(previous).max(i64::from(plan.gap_limit) - 1),
        Some(known) => 0.max(previous).max(known + i64::from(plan.gap_limit)),
    }
}

/// `_bitcoinrpc_descriptor_targets_for_range_ends`.
pub(super) fn targets_for_ends(plan: &Plan, ends: &BTreeMap<String, i64>) -> Result<Vec<Value>> {
    let mut targets = vec![];
    for branch in &plan.branches {
        let end = ends.get(&branch.index.to_string()).copied().unwrap_or(0);
        let end = u32::try_from(end.max(0)).map_err(|_| fallback("scan_horizon"))?;
        let derived = if end == 0 {
            plan.derive(Some(branch.index), 0, 1)
        } else {
            plan.derive(
                Some(branch.index),
                0,
                end.checked_add(1).ok_or_else(|| fallback("scan_horizon"))?,
            )
        };
        targets.extend(derived.map_err(descriptors::Error::into_app)?);
    }
    Ok(targets)
}

/// `_bitcoinrpc_descriptor_targets_for_checkpoint`: Core's import range,
/// derived locally.
pub(super) fn checkpoint_targets(plan: &Plan, checkpoint: &Value) -> Result<Vec<Value>> {
    let previous = int_mapping(&checkpoint["bitcoinrpc_descriptor_range_ends"]);
    let ends = plan
        .branches
        .iter()
        .map(|branch| {
            let key = branch.index.to_string();
            let end = descriptor_end(
                plan,
                branch,
                &checkpoint["highest_used"],
                previous.get(&key).copied(),
            );
            (key, end)
        })
        .collect();
    targets_for_ends(plan, &ends)
}

/// `bitcoinrpc_import_ranged_descriptors`.
fn import_ranges(
    node: &Node<'_>,
    wallet: &str,
    plan: &Plan,
    checkpoint: &Value,
    birthday: i64,
    force: bool,
) -> Result<(usize, BTreeMap<String, i64>)> {
    let previous = int_mapping(&checkpoint["bitcoinrpc_descriptor_range_ends"]);
    let mut next = previous.clone();
    let mut requests = vec![];
    for branch in &plan.branches {
        let key = branch.index.to_string();
        let end = descriptor_end(
            plan,
            branch,
            &checkpoint["highest_used"],
            previous.get(&key).copied(),
        );
        let merged = next.get(&key).copied().unwrap_or(-1).max(end);
        next.insert(key.clone(), merged);
        if !force && end <= previous.get(&key).copied().unwrap_or(-1) {
            continue;
        }
        // `bitcoinrpc_ranged_descriptor`.
        let info = node.call(
            "getdescriptorinfo",
            json!([branch.selected_descriptor()]),
            None,
        )?;
        let mut request = json!({"desc": info["descriptor"], "timestamp": birthday,
            "internal": branch.index % 2 == 1, "active": false});
        if branch.ranged() {
            request["range"] = json!([0, end]);
        }
        requests.push(request);
    }
    if requests.is_empty() {
        return Ok((0, next));
    }
    node.require_coverage(birthday)?;
    let count = requests.len();
    let results = node.import_into("importdescriptors", json!([requests]), wallet)?;
    if let Some(failure) = first_failure(&results) {
        return Err(node.import_failure(birthday, "ranged descriptor import", &failure));
    }
    Ok((count, next))
}

/// `bitcoinrpc_import_addresses`.
fn import_addresses(
    node: &Node<'_>,
    wallet: &str,
    wallet_id: &str,
    addresses: &[&str],
    birthday: i64,
    force: bool,
) -> Result<usize> {
    let label = format!("kassiber:{wallet_id}");
    let mut missing = vec![];
    let mut descriptors = vec![];
    for address in addresses {
        let info = node.call("getaddressinfo", json!([address]), Some(wallet))?;
        if (descriptors::truthy(&info["iswatchonly"]) || descriptors::truthy(&info["ismine"]))
            && !force
        {
            continue;
        }
        let descriptor = node.call(
            "getdescriptorinfo",
            json!([format!("addr({address})")]),
            None,
        )?;
        descriptors
            .push(json!({"desc": descriptor["descriptor"], "timestamp": birthday, "label": label}));
        missing.push(*address);
    }
    if missing.is_empty() {
        return Ok(0);
    }
    node.require_coverage(birthday)?;
    match node.import_into("importdescriptors", json!([descriptors]), wallet) {
        Ok(results) => {
            if let Some(failure) = first_failure(&results) {
                return Err(node.import_failure(birthday, "descriptor import", &failure));
            }
        }
        Err(error) => {
            let message = error.message.to_lowercase();
            if message.contains("rescan") && (message.contains("fail") || message.contains("prun"))
            {
                return Err(node.import_failure(
                    birthday,
                    "descriptor import",
                    &json!({"error": error.message}),
                ));
            }
            let requests: Vec<Value> = missing
                .iter()
                .map(|address| {
                    json!({"scriptPubKey": {"address": address}, "timestamp": birthday,
                    "watchonly": true, "label": label})
                })
                .collect();
            let results =
                node.import_into("importmulti", json!([requests, {"rescan": true}]), wallet)?;
            if let Some(failure) = first_failure(&results) {
                return Err(node.import_failure(birthday, "address import", &failure));
            }
        }
    }
    Ok(missing.len())
}

/// `fetch_bitcoinrpc_wallet_transactions`.
fn transactions(
    node: &Node<'_>,
    wallet: &str,
    checkpoint: Option<&Value>,
) -> Result<(Vec<Value>, Map<String, Value>)> {
    let checkpoint = checkpoint
        .filter(|c| c.is_object())
        .cloned()
        .unwrap_or(json!({}));
    let pending = descriptors::truthy(&checkpoint["bitcoinrpc_pending_maturity"]);
    let last = checkpoint["bitcoinrpc_last_block"]
        .as_str()
        .map(kassiber_pytext::strip)
        .filter(|s| !s.is_empty() && !pending)
        .map(str::to_owned);
    let mut fallback_reason = pending
        .then(|| "Bitcoin Core wallet has immature transactions awaiting maturity".to_owned());
    let mut removed: BTreeSet<String> = BTreeSet::new();
    let retracted_of = |details: &[Value]| -> BTreeSet<String> {
        details
            .iter()
            .filter(|d| d.is_object() && descriptors::truthy(&d["txid"]) && retracted(d))
            .map(detail_txid)
            .collect()
    };
    let immature = |details: &[Value]| {
        details
            .iter()
            .any(|d| d.is_object() && category(d) == "immature")
    };
    if let Some(last) = &last {
        match node.call("listsinceblock", json!([last, 1, true, true]), Some(wallet)) {
            Err(error)
                if error
                    .message
                    .contains("Bitcoin Core RPC listsinceblock failed") =>
            {
                fallback_reason = Some(error.message);
            }
            Err(error) => return Err(error),
            Ok(payload) if !payload.is_object() => {
                fallback_reason =
                    Some("Bitcoin Core RPC listsinceblock returned an unexpected payload".into());
            }
            Ok(payload) => {
                let listed = if payload["transactions"].is_null() {
                    Some(vec![])
                } else {
                    payload["transactions"].as_array().cloned()
                };
                let dropped = payload["removed"].as_array().cloned().unwrap_or_default();
                match listed {
                    None => {
                        fallback_reason = Some(
                            "Bitcoin Core RPC listsinceblock returned unexpected transactions"
                                .into(),
                        )
                    }
                    Some(_) if descriptors::truthy(&payload["removed"]) => {
                        removed = dropped
                            .iter()
                            .filter(|d| d.is_object())
                            .map(detail_txid)
                            .filter(|t| !t.is_empty())
                            .collect();
                        fallback_reason = Some(
                            "Bitcoin Core RPC listsinceblock reported removed transactions".into(),
                        );
                    }
                    Some(listed) => {
                        let retracted = retracted_of(&listed);
                        let mut meta = Map::new();
                        meta.insert("bitcoinrpc_sync_mode".into(), json!("sinceblock"));
                        meta.insert(
                            "bitcoinrpc_last_block".into(),
                            if descriptors::truthy(&payload["lastblock"]) {
                                payload["lastblock"].clone()
                            } else {
                                json!(last)
                            },
                        );
                        meta.insert("bitcoinrpc_removed".into(), json!(0));
                        meta.insert(
                            "bitcoinrpc_pending_maturity".into(),
                            json!(immature(&listed)),
                        );
                        if !retracted.is_empty() {
                            meta.insert("bitcoinrpc_retracted_txids".into(), json!(retracted));
                        }
                        return Ok((listed, meta));
                    }
                }
            }
        }
    }
    let mut listed = vec![];
    let mut skip = 0;
    loop {
        let page = node.call(
            "listtransactions",
            json!(["*", 1000, skip, true]),
            Some(wallet),
        )?;
        let Some(page) = page.as_array().filter(|p| !p.is_empty()) else {
            break;
        };
        let full = page.len() >= 1000;
        listed.extend(page.iter().cloned());
        if !full {
            break;
        }
        skip += 1000;
    }
    let mut retracted = removed;
    retracted.extend(retracted_of(&listed));
    let mut meta = Map::new();
    meta.insert("bitcoinrpc_sync_mode".into(), json!("full_scan"));
    meta.insert(
        "bitcoinrpc_pending_maturity".into(),
        json!(immature(&listed)),
    );
    if !retracted.is_empty() {
        meta.insert("bitcoinrpc_retracted_txids".into(), json!(retracted));
    }
    if let Some(reason) = fallback_reason {
        meta.insert("bitcoinrpc_sinceblock_fallback".into(), json!(reason));
    }
    if let Ok(best) = node.call("getbestblockhash", json!([]), None) {
        if descriptors::truthy(&best) {
            meta.insert("bitcoinrpc_last_block".into(), best);
        }
    }
    Ok((listed, meta))
}

/// `_bitcoinrpc_normalized_graph`: Core's verbose transaction as the public
/// graph Kassiber stores, with a curated HTLC spend proof.
fn normalized_graph(txid: &str, payload: &Value) -> Option<Value> {
    let decoded = payload.get("decoded").filter(|d| d.is_object())?;
    let vin = decoded["vin"].as_array()?;
    let vout = decoded["vout"].as_array()?;
    let mut evidence = vec![];
    let mut inputs = vec![];
    for entry in vin.iter().filter(|e| e.is_object()) {
        let mut item = Map::new();
        if !entry["txid"].is_null() {
            item.insert(
                "txid".into(),
                json!(kassiber_pytext::lower(&descriptors::text(&entry["txid"]))),
            );
        }
        if let Some(index) =
            crate::scan::integer(&entry["vout"]).filter(|_| !entry["vout"].is_null())
        {
            item.insert(
                "vout".into(),
                serde_json::from_str(&index.to_string()).unwrap_or(Value::Null),
            );
        }
        let prevout = prevout_of(&entry["prevout"]);
        if !prevout.is_empty() {
            item.insert("prevout".into(), Value::Object(prevout));
        }
        inputs.push(Value::Object(item));
    }
    for entry in vin {
        evidence.push(json!({"txid": entry["txid"], "vout": entry["vout"],
            "witness": entry.get("txinwitness").cloned().unwrap_or(Value::Null)}));
    }
    let mut outputs = vec![];
    for (position, entry) in vout.iter().enumerate().filter(|(_, e)| e.is_object()) {
        let (Ok(Some(value)), Some(script)) = (
            bitcoin_records::value_sats(entry),
            normalized_script(bitcoin_records::script(entry)),
        ) else {
            continue;
        };
        let index = crate::scan::integer(entry.get("n").unwrap_or(&json!(position)))
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(position as i64);
        outputs.push(json!({"n": index, "scriptpubkey": script, "value": value}));
    }
    if inputs.is_empty() || outputs.is_empty() {
        return None;
    }
    let id = if descriptors::truthy(&decoded["txid"]) {
        descriptors::text(&decoded["txid"])
    } else {
        txid.to_owned()
    };
    let mut graph = json!({"txid": kassiber_pytext::lower(&id), "vin": inputs, "vout": outputs,
        "source": "bitcoinrpc_gettransaction"});
    if let Some(proof) = htlc_spend(&evidence) {
        graph["observer"] = json!("bitcoinrpc");
        graph["htlc_spend"] = proof;
    }
    Some(graph)
}

/// `htlc_spend_attestation` over the claim and refund evidence of the
/// witnesses: one input revealing exactly one of them.
fn htlc_spend(inputs: &[Value]) -> Option<Value> {
    if inputs.len() != 1 {
        return None;
    }
    let mut links = json!({});
    bitcoin_records::witness_evidence(&json!({"vin": inputs}), &mut links);
    let claim = links.get("payment_hash").cloned();
    let refund = links.get("swap_refund_funding_txid").is_some();
    if claim.is_some() == refund {
        return None;
    }
    let (txid, vout) = if claim.is_some() {
        (&inputs[0]["txid"], &inputs[0]["vout"])
    } else {
        (
            &links["swap_refund_funding_txid"],
            &links["swap_refund_funding_vout"],
        )
    };
    let txid = crate::publication::physical::canonical_txid(
        &kassiber_pytext::loads(&txid.to_string()).ok()?,
    )?;
    let vout = vout.as_u64()?;
    let mut proof = json!({"version": 1, "role": if claim.is_some() {"claim"} else {"refund"},
        "funding_txid": txid, "funding_vout": vout, "input_count": 1});
    if let Some(hash) = claim {
        proof["payment_hash"] = hash;
    }
    Some(proof)
}

/// `normalized_script_hex`.
fn normalized_script(value: &str) -> Option<String> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(value));
    (!text.is_empty()
        && text.len().is_multiple_of(2)
        && text.bytes().all(|b| b.is_ascii_hexdigit()))
    .then_some(text)
}

/// `_bitcoinrpc_prevout_from_vin`'s prevout fields.
fn prevout_of(prevout: &Value) -> Map<String, Value> {
    let mut result = Map::new();
    if !prevout.is_object() {
        return result;
    }
    if let Some(script) = normalized_script(bitcoin_records::script(prevout)) {
        result.insert("scriptpubkey".into(), json!(script));
    }
    if let Ok(Some(value)) = bitcoin_records::value_sats(prevout) {
        result.insert("value".into(), json!(value));
    }
    result
}

type Cache = Map<String, Value>;

/// `_bitcoinrpc_fetch_normalized_graph`.
fn fetch_graph(node: &Node<'_>, wallet: &str, txid: &str, cache: &mut Cache) -> Option<Value> {
    let cached = cache
        .get(txid)
        .filter(|p| p["decoded"].is_object())
        .cloned();
    let payload = match cached {
        Some(payload) => payload,
        None => {
            let payload = node
                .call("gettransaction", json!([txid, true, true]), Some(wallet))
                .ok()?;
            cache.insert(txid.to_owned(), payload.clone());
            payload
        }
    };
    normalized_graph(txid, &payload)
}

/// `_bitcoinrpc_enrich_wallet_prevouts`: previous outputs the node's wallet
/// can see; `None` retries history next time.
fn enrich(
    node: &Node<'_>,
    wallet: &str,
    graph: &Value,
    cache: &mut Cache,
) -> Result<Option<Value>> {
    let mut inputs = vec![];
    for source in graph["vin"].as_array().into_iter().flatten() {
        let mut entry = source.clone();
        let complete = bitcoin_records::value_sats(&entry["prevout"])
            .ok()
            .flatten()
            .is_some()
            && !bitcoin_records::script(&entry["prevout"]).is_empty();
        if complete {
            inputs.push(entry);
            continue;
        }
        let parent = crate::publication::physical::canonical_txid(
            &kassiber_pytext::loads(&entry["txid"].to_string())
                .unwrap_or(kassiber_pyjson::Value::Null),
        );
        let index = entry["vout"].as_u64();
        let (Some(parent), Some(index)) = (parent, index) else {
            inputs.push(entry);
            continue;
        };
        let refetch = match cache.get(&parent) {
            None => true,
            Some(cached) => {
                cached.is_object()
                    && !cached["decoded"].is_object()
                    && cached["_wallet_parent_unavailable"] != true
            }
        };
        if refetch {
            let fetched =
                match node.call("gettransaction", json!([parent, true, true]), Some(wallet)) {
                    Ok(payload) => payload,
                    Err(error) => {
                        let code = error
                            .details
                            .as_ref()
                            .and_then(|d| kassiber_envelope::get(d, "rpc_error_code"))
                            .and_then(|c| match c {
                                kassiber_pyjson::Value::Int(n) => i64::try_from(n.clone()).ok(),
                                _ => None,
                            });
                        if code == Some(-5) {
                            // A foreign or pruned parent stays an explicit unknown.
                            json!({"_wallet_parent_unavailable": true})
                        } else {
                            return Ok(None);
                        }
                    }
                };
            cache.insert(parent.clone(), fetched);
        }
        let cached = &cache[&parent];
        let Some(outputs) = cached["decoded"]["vout"].as_array() else {
            if cached["_wallet_parent_unavailable"] == true {
                inputs.push(entry);
                continue;
            }
            return Ok(None);
        };
        let output = outputs.iter().enumerate().find(|(position, item)| {
            item.is_object()
                && crate::scan::integer(item.get("n").unwrap_or(&json!(*position)))
                    .is_some_and(|n| n == index.into())
        });
        let Some((_, output)) = output else {
            return Ok(None);
        };
        let prevout = prevout_of(output);
        if !prevout.contains_key("value") || !prevout.contains_key("scriptpubkey") {
            return Ok(None);
        }
        entry["prevout"] = Value::Object(prevout);
        inputs.push(entry);
    }
    let mut enriched = graph.clone();
    enriched["vin"] = json!(inputs);
    Ok(Some(enriched))
}

/// `record_from_bitcoinrpc_details`.
fn record(
    txid: &str,
    details: &[Value],
    backend: &str,
    graph: Option<&Value>,
    tracked: &BTreeSet<String>,
) -> Result<Option<Value>> {
    let graph = graph.map(|graph| {
        let mut graph = graph.clone();
        graph["observer_owned_scripts"] =
            json!(bitcoin_records::graph_scoped_scripts(&graph, tracked));
        graph
    });
    let mut amount = Decimal::zero();
    let mut fee = Decimal::zero();
    let mut sent = false;
    let mut fees: Vec<Decimal> = vec![];
    let mut occurred = bitcoin_records::UNKNOWN.to_owned();
    let mut confirmed: Option<String> = None;
    for detail in details {
        let kind = category(detail);
        if kind == "immature" || retracted(detail) {
            continue;
        }
        amount = amount
            .add(&dec(&detail["amount"], "0")?, &CONTEXT)
            .map_err(|_| invalid_decimal(&detail["amount"]))?;
        // Core repeats one signed wallet-relative fee on every send.
        sent |= kind == "send";
        let detail_fee = if kind == "send" {
            dec(&detail["fee"], "0")?
        } else {
            Decimal::zero()
        };
        if !detail_fee.is_zero() {
            if !fees.contains(&detail_fee) {
                fees.push(detail_fee.clone());
            }
            if fees.len() > 1 {
                return Err(AppError::new(
                    "bitcoinrpc_inconsistent_wallet_details",
                    "Bitcoin Core returned inconsistent wallet fee details",
                ));
            }
            fee = detail_fee;
        }
        if descriptors::truthy(&detail["blocktime"]) && detail["blocktime"] != "0" {
            confirmed = iso(&detail["blocktime"])?;
        }
        let when = if descriptors::truthy(&detail["blocktime"]) {
            &detail["blocktime"]
        } else {
            &detail["time"]
        };
        if let Some(when) = iso(when)? {
            occurred = when;
        }
    }
    if amount.is_zero() && fee.is_zero() {
        return Ok(None);
    }
    let (direction, value, row_fee, kind) = if amount.is_positive() {
        ("inbound", amount.clone(), Decimal::zero(), "deposit")
    } else {
        let gross = amount.copy_abs();
        let kind = if gross.is_positive() {
            "withdrawal"
        } else {
            "fee"
        };
        ("outbound", gross, fee.copy_abs(), kind)
    };
    let raw = match &graph {
        Some(graph) => graph.clone(),
        None => json!(details),
    };
    let mut record = json!({"txid": txid, "occurred_at": occurred, "confirmed_at": confirmed,
        "direction": direction, "asset": "BTC", "amount": decimal_json(&value), "fee": decimal_json(&row_fee),
        "fiat_rate": null, "fiat_value": null, "kind": kind, "description": format!("Synced from {backend}"),
        "counterparty": null, "raw_json": sorted_json(&raw)?});
    let mut boundary: Option<String> = None;
    let mut observed = None;
    if let Some(graph) = &graph {
        let inputs = graph["vin"].as_array().filter(|v| !v.is_empty());
        let complete = inputs.is_some_and(|inputs| {
            inputs.iter().all(|item| {
                bitcoin_records::value_sats(&item["prevout"])
                    .ok()
                    .flatten()
                    .or_else(|| bitcoin_records::value_sats(item).ok().flatten())
                    .is_some()
                    && !bitcoin_records::script(&item["prevout"]).is_empty()
            })
        });
        if !tracked.is_empty() && complete {
            observed = bitcoin_records::from_graph(
                graph,
                tracked,
                backend,
                &json!(txid),
                &occurred,
                confirmed.as_deref(),
                None,
            )?;
            if let Some(observed) = &observed {
                record = observed.clone();
                boundary = observed["privacy_boundary"].as_str().map(str::to_owned);
            }
        }
    }
    if observed.is_none() && (direction == "outbound" || sent) {
        // Without every previous output, a transaction-wide fee is not this
        // wallet's to claim: book the observed wallet delta.
        let mut payload = match &graph {
            Some(graph) => graph.clone(),
            None => json!({"source": "bitcoinrpc_wallet_details", "details": details}),
        };
        payload["component"] = json!({"fee_attribution": "implicit_wallet_delta"});
        payload["observer_owned_scripts"] = match &graph {
            Some(graph) => json!(bitcoin_records::graph_scoped_scripts(graph, tracked)),
            None => json!(tracked),
        };
        let found =
            bitcoin_records::privacy_boundary(&payload)?.unwrap_or_else(|| "collaborative".into());
        payload["privacy_boundary"] = json!(found);
        boundary = Some(found);
        let delta = amount
            .add(&fee, &CONTEXT)
            .map_err(|_| invalid_decimal(&json!(null)))?;
        record["direction"] = json!(if delta.is_positive() {
            "inbound"
        } else {
            "outbound"
        });
        record["amount"] = decimal_json(&delta.copy_abs());
        record["fee"] = json!("0");
        record["kind"] = json!(if delta.is_positive() {
            "deposit"
        } else {
            "withdrawal"
        });
        record["raw_json"] = json!(sorted_json(&payload)?);
    }
    // Hints only; exact matching needs the closed observation authority.
    if let Some(proof) = graph
        .as_ref()
        .map(|g| &g["htlc_spend"])
        .filter(|p| p.is_object())
    {
        if proof["role"] == "claim" {
            record["payment_hash"] = proof["payment_hash"].clone();
            record["payment_hash_source"] = json!("chain_script_unique_outpoint");
        } else if proof["role"] == "refund" {
            record["swap_refund_funding_txid"] = proof["funding_txid"].clone();
            if proof["funding_vout"].is_u64() {
                record["swap_refund_funding_vout"] = proof["funding_vout"].clone();
            }
        }
    }
    if let Some(boundary) = boundary {
        record["privacy_boundary"] = json!(boundary);
    }
    Ok(Some(record))
}

/// `bitcoinrpc_records_for_wallet`.
fn records(
    node: &Node<'_>,
    wallet: &str,
    imported: usize,
    checkpoint: &Value,
    targets: &[Value],
) -> Result<(Vec<Value>, Map<String, Value>, Cache)> {
    // Core's existing history is replayed once after normalization changes.
    let current = checkpoint["bitcoinrpc_graph_normalization_version"].as_u64()
        == Some(GRAPH_NORMALIZATION_VERSION);
    let since = (imported == 0 && current).then_some(checkpoint);
    let (details, mut meta) = transactions(node, wallet, since)?;
    let mut grouped: Vec<(String, Vec<Value>)> = vec![];
    for detail in &details {
        let Some(txid) = detail["txid"].as_str().filter(|t| !t.is_empty()) else {
            continue;
        };
        match grouped.iter_mut().find(|(id, _)| id == txid) {
            Some((_, list)) => list.push(detail.clone()),
            None => grouped.push((txid.to_owned(), vec![detail.clone()])),
        }
    }
    let time = |detail: &Value| -> i64 {
        let raw = if descriptors::truthy(&detail["blocktime"]) {
            &detail["blocktime"]
        } else if descriptors::truthy(&detail["time"]) {
            &detail["time"]
        } else {
            return 0;
        };
        crate::scan::integer(raw)
            .and_then(|n| i64::try_from(n).ok())
            .unwrap_or(0)
    };
    grouped.sort_by(|(a, left), (b, right)| {
        let l = left.iter().map(time).max().unwrap_or(0);
        let r = right.iter().map(time).max().unwrap_or(0);
        (l, a).cmp(&(r, b))
    });
    let tracked: BTreeSet<String> = targets
        .iter()
        .filter_map(|t| {
            t["script_pubkey"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .collect();
    let backend = node.name();
    let mut records = vec![];
    let mut cache = Cache::new();
    let mut unavailable: BTreeSet<String> = BTreeSet::new();
    for (txid, list) in &grouped {
        node.control.check(node.deadline)?;
        let mut record_value = record(txid, list, &backend, None, &tracked)?;
        if record_value.is_some() {
            let mut graph = fetch_graph(node, wallet, txid, &mut cache);
            if let Some(found) = graph
                .clone()
                .filter(|_| list.iter().any(|d| category(d) == "send"))
            {
                match enrich(node, wallet, &found, &mut cache)? {
                    Some(enriched) => graph = Some(enriched),
                    None => {
                        unavailable.insert(txid.to_lowercase());
                    }
                }
            }
            match &graph {
                Some(graph) => record_value = record(txid, list, &backend, Some(graph), &tracked)?,
                None => {
                    unavailable.insert(txid.to_lowercase());
                }
            }
        }
        if let Some(found) = record_value {
            records.push(found);
        }
    }
    if unavailable.is_empty() {
        meta.insert(
            "bitcoinrpc_graph_normalization_version".into(),
            json!(GRAPH_NORMALIZATION_VERSION),
        );
    } else {
        meta.insert(
            "bitcoinrpc_graph_unavailable_txids".into(),
            json!(unavailable),
        );
        meta.remove("bitcoinrpc_last_block");
    }
    if let Some(retracted) = meta.get("bitcoinrpc_retracted_txids").cloned() {
        let active: BTreeSet<String> = records
            .iter()
            .map(|r| descriptors::text(&r["txid"]).to_lowercase())
            .collect();
        let left: BTreeSet<String> = retracted
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|t| !active.contains(*t))
            .map(str::to_owned)
            .collect();
        if left.is_empty() {
            meta.remove("bitcoinrpc_retracted_txids");
        } else {
            meta.insert("bitcoinrpc_retracted_txids".into(), json!(left));
        }
    }
    // `_bitcoinrpc_highest_used_from_details`.
    let mut highest = Map::new();
    for detail in &details {
        if category(detail) == "immature" || retracted(detail) {
            continue;
        }
        if let Some(target) = targets
            .iter()
            .rev()
            .find(|t| descriptors::truthy(&t["address"]) && t["address"] == detail["address"])
        {
            merge_highest(&mut highest, target);
        }
    }
    let mut ordered = Map::new();
    ordered.insert("core_wallet".into(), json!(wallet));
    ordered.insert("imported_addresses".into(), json!(imported));
    ordered.insert("bitcoinrpc_highest_used".into(), Value::Object(highest));
    ordered.extend(meta);
    Ok((records, ordered, cache))
}

/// `bitcoinrpc_utxos_for_wallet_name`.
fn outputs(
    node: &Node<'_>,
    wallet: &str,
    addresses: &[String],
    targets: &[Value],
    input: &Input<'_>,
    mut cache: Cache,
) -> Result<Vec<Value>> {
    let listed = node.call(
        "listunspent",
        json!([0, 9_999_999, addresses, true]),
        Some(wallet),
    )?;
    let mut outputs = vec![];
    let mut metadata: Map<String, Value> = Map::new();
    for raw in listed.as_array().into_iter().flatten() {
        let target = targets
            .iter()
            .rev()
            .find(|t| descriptors::truthy(&t["address"]) && t["address"] == raw["address"])
            .or_else(|| {
                let script = kassiber_pytext::lower(&descriptors::text(&raw["scriptPubKey"]));
                targets.iter().rev().find(|t| {
                    descriptors::truthy(&t["script_pubkey"]) && t["script_pubkey"] == json!(script)
                })
            });
        let Some(target) = target else {
            continue;
        };
        let txid = descriptors::text(&raw["txid"]);
        let confirmations = if descriptors::truthy(&raw["confirmations"]) {
            crate::scan::integer(&raw["confirmations"])
                .and_then(|n| i64::try_from(n).ok())
                .unwrap_or(0)
        } else {
            0
        };
        if !metadata.contains_key(&txid) {
            let found = transaction_metadata(node, wallet, &txid, &mut cache)?;
            metadata.insert(txid.clone(), found);
        }
        let found = &metadata[&txid];
        let sats = dec(&raw["amount"], "0")?
            .mul(&Decimal::from(100_000_000_u64), &CONTEXT)
            .and_then(|d| d.to_fixed_places(0, Rounding::HalfEven))
            .map_err(|_| invalid_decimal(&raw["amount"]))?;
        let mut output = json!({"txid": raw["txid"], "vout": raw["vout"], "asset": "BTC",
            "amount_sats": serde_json::from_str::<Value>(&sats).map_err(|_| invalid_decimal(&raw["amount"]))?,
            "confirmation_status": if confirmations > 0 {"confirmed"} else {"mempool"},
            "confirmations": confirmations, "block_height": found["block_height"],
            "block_time": found["block_time"], "chain": input.chain, "network": input.network});
        let fields = output.as_object_mut().expect("object");
        fields.extend(super::electrum::target_metadata(target));
        fields.insert(
            "raw".into(),
            json!({"source": "bitcoinrpc_listunspent", "confirmed": confirmations > 0,
                "safe": raw.get("safe").is_none_or(descriptors::truthy)}),
        );
        outputs.push(output);
    }
    Ok(outputs)
}

/// `_bitcoinrpc_transaction_metadata`.
fn transaction_metadata(
    node: &Node<'_>,
    wallet: &str,
    txid: &str,
    cache: &mut Cache,
) -> Result<Value> {
    let tx = match cache.get(txid).filter(|t| t.is_object()) {
        Some(tx) => tx.clone(),
        None => match node.call("gettransaction", json!([txid, true]), Some(wallet)) {
            Ok(tx) => {
                cache.insert(txid.to_owned(), tx.clone());
                tx
            }
            Err(_) => return Ok(json!({"block_height": null, "block_time": null})),
        },
    };
    let when = if descriptors::truthy(&tx["blocktime"]) {
        &tx["blocktime"]
    } else {
        &tx["time"]
    };
    let block_time = iso(when)?;
    let mut height = Value::Null;
    if descriptors::truthy(&tx["blockhash"]) {
        if let Ok(header) = node.call("getblockheader", json!([tx["blockhash"]]), None) {
            height = header["height"].clone();
        }
    }
    Ok(json!({"block_height": height, "block_time": block_time}))
}
