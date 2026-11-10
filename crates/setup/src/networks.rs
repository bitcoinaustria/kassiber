//! Book-wide authored network scope. Inspection never grants ownership or egress.
use super::*;
use kassiber_pyjson::{dumps, Options};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeSet;
#[path = "network_partition.rs"]
pub mod partition;

const HINT: &str =
    "Review the active book's network in Settings. Existing history cannot be relabeled.";
const REFERENCE_TABLES: &[&str] = &[
    "chain_analysis_observations",
    "chain_observer_instances",
    "chain_observation_provenance",
    "wallet_policy_epochs",
    "wallet_utxos",
];
pub(super) fn error(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
        .with_hint(HINT)
        .with_details(dict::<&str>([]))
}
fn mismatch(message: &str) -> AppError {
    error("book_network_mismatch", message)
}
fn value<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}
fn list(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        &[]
    }
}
fn object(v: &Value) -> Value {
    match v {
        Value::Dict(_) => v.clone(),
        Value::Str(v) => v
            .as_str()
            .and_then(|v| kassiber_pytext::loads(v).ok())
            .filter(|v| matches!(v, Value::Dict(_)))
            .unwrap_or_else(|| dict::<&str>([])),
        _ => dict::<&str>([]),
    }
}
// A negative lexical test only: without escapes, a JSON object cannot
// contain a relevant key unless its quoted spelling occurs in the input.
// Invalid JSON and nonobjects also become {} in object(). Possible evidence,
// including escaped keys, always takes the Python parser.
fn irrelevant_json(v: &Value) -> bool {
    matches!(v, Value::Str(text) if text.as_str().is_some_and(|raw| {
        !raw.contains('\\') && ![
            "\"chain\"", "\"network\"", "\"bitcoin_network\"",
            "\"chain_network\"", "\"chain_instance_id\"",
        ].iter().any(|key| raw.contains(key))
    }))
}
fn evidence_object(v: &Value) -> Cow<'_, Value> {
    match v {
        Value::Dict(_) => Cow::Borrowed(v),
        v if irrelevant_json(v) => Cow::Owned(dict::<&str>([])),
        _ => Cow::Owned(object(v)),
    }
}
pub(crate) fn json(v: &Value, compact: bool, sorted: bool) -> Result<String> {
    dumps(
        v,
        &Options {
            sort_keys: sorted,
            separators: compact.then_some((",".into(), ":".into())),
            default_str: true,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))
}
pub(crate) fn digest(v: &Value) -> Result<String> {
    Ok(Sha256::digest(json(v, true, true)?.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn normalize(v: &Value) -> String {
    kassiber_pytext::strip(&py_str(v)).to_lowercase()
}
fn domain(v: &Value) -> Option<&'static str> {
    match normalize(v).as_str() {
        "main" | "mainnet" | "bitcoin" | "liquid" | "liquidv1" => Some("main"),
        "test" | "testnet" | "liquidtestnet" => Some("test"),
        "regtest" | "elements" | "elementsregtest" => Some("regtest"),
        "signet" => Some("signet"),
        _ => None,
    }
}
fn nonempty(v: &Value) -> bool {
    !matches!(v, Value::Null) && v != &s("")
}
fn domain_evidence(raw: &Value, config: &Value, asset: &Value) -> (Option<String>, bool) {
    let mut families = BTreeSet::new();
    let mut domains = BTreeSet::new();
    for payload in [raw, config] {
        if nonempty(value(payload, "chain")) {
            let family = match normalize(value(payload, "chain"))
                .replace('_', "-")
                .as_str()
            {
                "bitcoin" | "btc" | "onchain" | "lightning" | "core-ln" | "core-lightning"
                | "lnd" | "coreln" | "cln" | "nwc" | "phoenix" => "bitcoin",
                "liquid" | "lbtc" | "elements" => "liquid",
                _ => return (None, false),
            };
            families.insert(family);
        }
        for key in ["network", "bitcoin_network", "chain_network"] {
            if nonempty(value(payload, key)) {
                let Some(d) = domain(value(payload, key)) else {
                    return (None, false);
                };
                domains.insert(d);
            }
        }
    }
    let asset = normalize(asset).replace("l-btc", "lbtc");
    if families.len() > 1
        || domains.len() > 1
        || asset == "btc" && families.contains("liquid")
        || asset == "lbtc" && families.contains("bitcoin")
    {
        return (None, false);
    }
    (domains.first().map(|v| (*v).into()), true)
}
type EvidenceFacts = (Option<String>, Option<String>, bool);
/// Distinguish absent evidence from contradictory evidence; never infer mainnet.
pub fn evidence(row: &Value) -> EvidenceFacts {
    let mut raw = evidence_object(value(row, "raw_json"));
    let config_input = optional(row, "config_json")
        .filter(|v| truthy(v))
        .unwrap_or_else(|| value(row, "wallet_config_json"));
    let config = evidence_object(config_input);
    let instance_input = optional(row, "wallet_config_json")
        .filter(|v| truthy(v))
        .unwrap_or_else(|| value(row, "config_json"));
    let other_config;
    let instance_config = if std::ptr::eq(config_input, instance_input) {
        config.as_ref()
    } else {
        other_config = evidence_object(instance_input);
        other_config.as_ref()
    };
    if value(&raw, "source") == &s("bullbitcoin_wallet_csv")
        && ["bitcoin", "liquid", "lightning"]
            .contains(&py_str(value(&raw, "network")).to_lowercase().as_str())
        || value(row, "wallet_kind") == &s("coinbase")
            && matches!(value(&raw, "network"), Value::Dict(_))
    {
        if let Value::Dict(fields) = raw.to_mut() {
            fields.retain(|(k, _)| *k != "network".into());
        }
    }
    // Binance names the coin's transfer rail; "BTC" is Bitcoin on chain, and
    // Binance runs on mainnet only. Other rails stay unsupported evidence.
    if value(row, "wallet_kind") == &s("binance")
        && value(row, "asset") == &s("BTC")
        && value(&raw, "network") == &s("BTC")
    {
        if let Value::Dict(fields) = raw.to_mut() {
            for (key, field) in fields.iter_mut() {
                if *key == "network".into() {
                    *field = s("main");
                }
            }
        }
    }
    let (mut environment, mut valid) = domain_evidence(&raw, &config, value(row, "asset"));
    let columns = dict(
        ["chain", "network", "bitcoin_network", "chain_network"]
            .into_iter()
            .filter(|k| nonempty(value(row, k)))
            .map(|k| (k, value(row, k).clone())),
    );
    if truthy(&columns) {
        for payload in [raw.as_ref(), instance_config] {
            let (other, ok) = domain_evidence(&columns, payload, value(row, "asset"));
            valid &= ok && (environment.is_none() || other.is_none() || environment == other);
            environment = environment.or(other);
        }
    }
    let instances: BTreeSet<String> = [row, raw.as_ref(), instance_config]
        .into_iter()
        .map(|v| value(v, "chain_instance_id"))
        .filter(|v| truthy(v))
        .map(|v| kassiber_pytext::strip(&py_str(v)).to_owned())
        .collect();
    (
        environment,
        if instances.len() == 1 {
            instances.first().cloned()
        } else {
            None
        },
        valid && instances.len() <= 1,
    )
}
pub fn guard_observation(
    conn: &Connection,
    profile: &str,
    row: &Value,
    operation: &str,
) -> Result<()> {
    let (environment, instance, valid) = evidence(row);
    if !valid {
        return Err(mismatch("Observation contradicts its configured network")
            .with_details(dict([("operation", s(operation))])));
    }
    if let Some(binding) = binding(conn, profile)? {
        for (actual, key, message) in [
            (
                environment,
                "environment",
                "Observation belongs to a different book environment",
            ),
            (
                instance,
                "chain_instance_id",
                "Observation belongs to a different local chain instance",
            ),
        ] {
            if let Some(actual) = actual {
                if value(&binding, key) != &s(actual) {
                    return Err(mismatch(message).with_details(dict([("operation", s(operation))])));
                }
            }
        }
    }
    Ok(())
}
pub fn guard_wallet(
    conn: &Connection,
    profile: &str,
    config: &Value,
    previous: Option<&Value>,
    operation: &str,
) -> Result<()> {
    if let Some(previous) = previous {
        let old = evidence(&dict([("config_json", previous.clone())]));
        let new = evidence(&dict([("config_json", config.clone())]));
        if old != new && (old.0.is_some() || old.1.is_some()) {
            return Err(mismatch(
                "A connection's historical network cannot be changed; create a new connection",
            ));
        }
    }
    guard_observation(
        conn,
        profile,
        &dict([("config_json", config.clone())]),
        operation,
    )
}
pub fn observation_matches_binding(binding: &Value, row: &Value, unscoped: bool) -> bool {
    if value(binding, "state") != &s("bound") {
        return true;
    }
    let (environment, instance, valid) = evidence(row);
    if !valid || environment.map(s).as_ref() != Some(value(binding, "environment")) {
        return false;
    }
    let expected = value(binding, "chain_instance_id");
    if instance
        .as_ref()
        .is_some_and(|v| &s(v.as_str()) != expected)
    {
        return false;
    }
    !(unscoped && truthy(expected) && instance.map(s).as_ref() != Some(expected))
}
pub fn require_accounting(conn: &Connection, profile: &str) -> Result<()> {
    if !table_exists(conn, "book_network_bindings")? {
        return Ok(());
    }
    // Admission needs all the evidence checks, but no review commitment or
    // per-wallet presentation. Keep both paths on the same interpreter.
    let inventory = read_snapshot(conn, || inventory_snapshot(conn, profile, false))?;
    if ["mixed", "conflicted"].contains(&text(&inventory, "state").as_str()) {
        return Err(error(
            "book_network_review_required",
            "This book contains incompatible network histories",
        )
        .with_details(dict([("state", value(&inventory, "state").clone())])));
    }
    if let Some(binding) = binding(conn, profile)? {
        if list(value(&inventory, "environments"))
            .iter()
            .any(|v| v != value(&binding, "environment"))
            || list(value(&inventory, "chain_instances"))
                .iter()
                .any(|v| v != value(&binding, "chain_instance_id"))
        {
            return Err(error(
                "book_network_review_required",
                "Book observations no longer match its network binding",
            ));
        }
    }
    Ok(())
}
pub fn require_chain_domain(
    conn: &Connection,
    profile: &str,
    chain: &str,
    network: &Value,
    instance: Option<&str>,
    operation: &str,
) -> Result<Value> {
    let binding = resolve(conn, profile)?;
    if value(&binding, "state") != &s("bound") {
        return Err(error(
            "book_network_unbound",
            "Bind this book to a network environment first",
        ));
    }
    let chain = if chain == "lightning" {
        "bitcoin"
    } else {
        chain
    };
    let normalized = if truthy(network) && !normalize(network).is_empty() {
        normalize(network)
    } else if chain == "liquid" {
        "liquidv1".into()
    } else {
        "main".into()
    };
    let normalized = if chain == "liquid" {
        match normalized.as_str() {
            "liquid" | "liquidv1" | "main" | "mainnet" => "liquidv1",
            "liquidtestnet" | "test" | "testnet" => "liquidtestnet",
            "elements" | "elementsregtest" | "regtest" => "elementsregtest",
            _ => return Err(error("validation", "Unsupported chain domain")),
        }
    } else if chain == "bitcoin" {
        match normalized.as_str() {
            "main" | "mainnet" | "bitcoin" => "main",
            "test" | "testnet" => "test",
            "signet" => "signet",
            "regtest" => "regtest",
            _ => return Err(error("validation", "Unsupported chain domain")),
        }
    } else {
        return Err(error("validation", "Unsupported chain domain"));
    };
    let domain = list(value(&binding, "domains"))
        .iter()
        .find(|d| value(d, "chain") == &s(chain) && value(d, "network") == &s(normalized));
    let Some(domain) = domain
        .filter(|d| instance.is_none_or(|instance| value(d, "chain_instance_id") == &s(instance)))
    else {
        return Err(
            mismatch("Source belongs to a different book network").with_details(dict([
                ("operation", s(operation)),
                ("profile_id", s(profile)),
            ])),
        );
    };
    let mut result = domain.clone();
    for k in ["environment_id", "revision"] {
        replace(&mut result, k, value(&binding, k).clone());
    }
    Ok(result)
}
fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        [table],
        |r| r.get(0),
    )
    .map_err(sql_error)
}
fn binding(conn: &Connection, profile: &str) -> Result<Option<Value>> {
    if !table_exists(conn, "book_network_bindings")? {
        return Ok(None);
    }
    Ok(rows(
        conn,
        "SELECT * FROM book_network_bindings WHERE profile_id=?",
        [profile],
    )?
    .into_iter()
    .next())
}
/// Python's read savepoint: borrow caller transactions; discard only owned writes.
pub fn read_snapshot<T>(conn: &Connection, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    if !conn.is_autocommit() {
        return operation();
    }
    let savepoint = conn.unchecked_transaction().map_err(sql_error)?;
    let result = operation();
    savepoint.rollback().map_err(sql_error)?;
    result
}
pub fn inventory(conn: &Connection, profile: &str) -> Result<Value> {
    read_snapshot(conn, || inventory_snapshot(conn, profile, true))
}
// The accounting gate needs only unions and a conflict flag. Decode the same
// ordered projection as review, without retaining raw observations or cloning
// wallet configuration for every neutral row. This cache lasts one read only.
#[derive(Default)]
struct AccountingFacts {
    environments: BTreeSet<String>,
    instances: BTreeSet<String>,
    conflicted: bool,
    count: usize,
}
fn accounting_observations(
    conn: &Connection,
    profile: &str,
    wallets: &[Value],
) -> Result<AccountingFacts> {
    let mut result = AccountingFacts::default();
    let mut seen = vec![false; wallets.len()];
    let mut neutral: Vec<Vec<Value>> = vec![Vec::new(); wallets.len()];
    let wallet_indices: std::collections::BTreeMap<_, _> = wallets
        .iter()
        .enumerate()
        .filter_map(|(i, row)| match value(row, "id") {
            Value::Str(id) => id.as_str().map(|id| (id, i)),
            _ => None,
        })
        .collect();
    let mut merge = |facts: EvidenceFacts| {
        let (environment, instance, valid) = facts;
        if !valid {
            result.conflicted = true;
        } else if let Some(environment) = environment {
            if environment == "regtest" {
                if let Some(instance) = instance {
                    result.instances.insert(instance);
                }
            }
            result.environments.insert(environment);
        }
    };
    visit_rows(conn, "SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id", [profile], |row| {
        result.count += 1;
        let index = match value(row,"wallet_id") {
            Value::Str(id) => id.as_str().and_then(|id| wallet_indices.get(id).copied()),
            id => wallets.iter().position(|w| value(w,"id") == id),
        };
        let Some(index) = index else { return; };
        seen[index] = true;
        let facts = if irrelevant_json(value(row,"raw_json")) {
            let cache = &mut neutral[index];
            let asset = value(row,"asset");
            // Unioning the same facts twice cannot change admission. There
            // are no per-row review counts on this path.
            if cache.contains(asset) { return; }
            cache.push(asset.clone());
            evidence(row)
        } else { evidence(row) };
        merge(facts);
    })?;
    for (wallet, seen) in wallets.iter().zip(seen) {
        if !seen {
            merge(evidence(&dict([
                ("wallet_kind", value(wallet, "kind").clone()),
                ("config_json", value(wallet, "config_json").clone()),
            ])));
        }
    }
    Ok(result)
}
fn inventory_snapshot(conn: &Connection, profile: &str, review: bool) -> Result<Value> {
    let profiles = rows(
        conn,
        "SELECT id,journal_input_version FROM profiles WHERE id=?",
        [profile],
    )?;
    let profile_row = profiles
        .first()
        .ok_or_else(|| error("not_found", "Book not found"))?;
    let wallets = rows(
        conn,
        "SELECT id,label,kind,config_json FROM wallets WHERE profile_id=? ORDER BY id",
        [profile],
    )?;
    let observations = if review {
        rows(conn, "SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id", [profile])?
    } else {
        Vec::new()
    };
    let accounting = if review {
        AccountingFacts::default()
    } else {
        accounting_observations(conn, profile, &wallets)?
    };
    let transaction_count = if review {
        observations.len()
    } else {
        accounting.count
    };
    let mut fingerprint = if review {
        vec![
            profile_row.clone(),
            Value::List(wallets.clone()),
            Value::List(observations.clone()),
        ]
    } else {
        Vec::new()
    };
    let mut all_environments = accounting.environments;
    let mut all_instances = accounting.instances;
    let mut wallet_rows = Vec::new();
    let mut wallet_conflicts = accounting.conflicted;
    for wallet in wallets.into_iter().filter(|_| review) {
        let transactions: Vec<_> = observations
            .iter()
            .filter(|r| value(r, "wallet_id") == value(&wallet, "id"))
            .collect();
        let fallback = dict([
            ("wallet_kind", value(&wallet, "kind").clone()),
            ("config_json", value(&wallet, "config_json").clone()),
        ]);
        let mut known = BTreeSet::new();
        let mut instances = BTreeSet::new();
        let mut conflicts = Vec::new();
        let mut unknown = 0;
        // Joined wallet kind/config are constant within this wallet. When the
        // raw observation provably contains no network facts, only its asset
        // can change the evidence result. Cache that result, not admission.
        let mut neutral_assets: Vec<(Value, EvidenceFacts)> = Vec::new();
        for row in transactions
            .iter()
            .copied()
            .chain(transactions.is_empty().then_some(&fallback))
        {
            let facts = if irrelevant_json(value(row, "raw_json")) {
                let asset = value(row, "asset");
                if let Some((_, facts)) = neutral_assets.iter().find(|(key, _)| key == asset) {
                    facts.clone()
                } else {
                    let facts = evidence(row);
                    neutral_assets.push((asset.clone(), facts.clone()));
                    facts
                }
            } else {
                evidence(row)
            };
            let (environment, instance, valid) = facts;
            if !valid {
                conflicts.push(value(row, "id").clone());
            } else if let Some(environment) = environment {
                if environment == "regtest" {
                    if let Some(instance) = instance {
                        instances.insert(instance);
                    }
                }
                known.insert(environment);
            } else {
                unknown += 1;
            }
        }
        all_environments.extend(known.iter().cloned());
        all_instances.extend(instances.iter().cloned());
        wallet_conflicts |= !conflicts.is_empty();
        if review {
            wallet_rows.push(dict([
                ("wallet_id", value(&wallet, "id").clone()),
                ("label", value(&wallet, "label").clone()),
                ("environments", strings(&known)),
                ("chain_instances", strings(&instances)),
                ("transaction_count", Value::Int(transactions.len().into())),
                ("unknown_count", Value::Int(unknown.into())),
                ("conflict_count", Value::Int(conflicts.len().into())),
                (
                    "conflicting_transaction_ids",
                    Value::List(conflicts.into_iter().filter(truthy).collect()),
                ),
                (
                    "requires_declaration",
                    Value::Bool(unknown != 0 || known.contains("regtest") && instances.is_empty()),
                ),
            ]));
        }
    }
    let mut references = Vec::new();
    let mut reference_conflicts = false;
    for &table in REFERENCE_TABLES {
        if !table_exists(conn, table)? {
            continue;
        }
        let columns = rows(conn, &format!("PRAGMA table_info(\"{table}\")"), [])?;
        if !["profile_id", "chain", "network"]
            .into_iter()
            .all(|k| columns.iter().any(|r| value(r, "name") == &s(k)))
        {
            continue;
        }
        let source_rows = rows(
            conn,
            &format!("SELECT * FROM \"{table}\" WHERE profile_id=?"),
            [profile],
        )?;
        if review {
            fingerprint.push(Value::List(vec![
                s(table),
                Value::List(source_rows.clone()),
            ]));
        }
        for source in source_rows {
            let mut row = dict([
                ("raw_json", object(value(&source, "payload_json"))),
                (
                    "config_json",
                    dict([
                        ("chain", value(&source, "chain").clone()),
                        ("network", value(&source, "network").clone()),
                    ]),
                ),
            ]);
            if table == "wallet_policy_epochs"
                && !truthy(&object(value(&source, "private_material_json")))
                && !conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM wallet_policy_sources WHERE epoch_id=?)",
                        [text(&source, "id")],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(sql_error)?
            {
                row = dict::<&str>([]);
            }
            let (environment, instance, valid) = evidence(&row);
            if !valid {
                reference_conflicts = true;
            } else if let Some(environment) = &environment {
                all_environments.insert(environment.clone());
                if let Some(instance) = &instance {
                    all_instances.insert(instance.clone());
                }
            }
            if review {
                references.push(dict([
                    ("source", s(table)),
                    ("environment", environment.map_or(Value::Null, s)),
                    ("chain_instance_id", instance.map_or(Value::Null, s)),
                    ("valid", Value::Bool(valid)),
                ]));
            }
        }
    }
    let state = if wallet_conflicts || reference_conflicts {
        "conflicted"
    } else if all_environments.len() > 1 || all_instances.len() > 1 {
        "mixed"
    } else {
        "unbound"
    };
    Ok(dict([
        ("profile_id", s(profile)),
        ("state", s(state)),
        ("environments", strings(&all_environments)),
        ("chain_instances", strings(&all_instances)),
        ("wallets", Value::List(wallet_rows)),
        ("reference_scopes", Value::List(references)),
        (
            "inventory_digest",
            if review {
                s(digest(&Value::List(fingerprint))?)
            } else {
                Value::Null
            },
        ),
        ("transaction_count", Value::Int(transaction_count.into())),
    ]))
}
fn strings(values: &BTreeSet<String>) -> Value {
    Value::List(values.iter().map(|v| s(v.as_str())).collect())
}
pub fn resolve(conn: &Connection, profile: &str) -> Result<Value> {
    let Some(binding) = binding(conn, profile)? else {
        return Ok(dict([
            ("profile_id", s(profile)),
            ("state", s("unbound")),
            ("environment_id", Value::Null),
            ("revision", Value::Int(0.into())),
            ("domains", Value::List(vec![])),
        ]));
    };
    let mut result = dict([("profile_id", s(profile)), ("state", s("bound"))]);
    for key in [
        "environment_id",
        "environment",
        "revision",
        "chain_instance_id",
    ] {
        replace(&mut result, key, value(&binding, key).clone());
    }
    for (source, target) in [
        ("domains_json", "domains"),
        ("acknowledgements_json", "declared_wallet_ids"),
    ] {
        replace(
            &mut result,
            target,
            kassiber_pytext::loads(&text(&binding, source))
                .map_err(|e| AppError::new("internal_error", e.to_string()))?,
        );
    }
    Ok(result)
}
fn networks(environment: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match environment {
        "main" => Some(&[("bitcoin", "main"), ("liquid", "liquidv1")]),
        "test" => Some(&[("bitcoin", "test"), ("liquid", "liquidtestnet")]),
        "signet" => Some(&[("bitcoin", "signet")]),
        "regtest" => Some(&[("bitcoin", "regtest"), ("liquid", "elementsregtest")]),
        _ => None,
    }
}
pub fn plan(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    read_snapshot(conn, || plan_snapshot(conn, profile, args))
}
fn plan_snapshot(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    let environment = text(args, "environment");
    let networks = networks(environment.as_str())
        .ok_or_else(|| error("validation", "Choose a supported book environment"))?;
    let mut instance = optional(args, "chain_instance_id")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or(Value::Null);
    if environment == "regtest" {
        let input = py_str(&instance).replace("urn:", "").replace("uuid:", "");
        let input = input.trim_matches(['{', '}']).replace('-', "");
        instance = s(uuid::Uuid::parse_str(&input)
            .map_err(|_| {
                error(
                    "validation",
                    "A local regtest instance needs an explicit UUID",
                )
            })?
            .to_string());
    } else if !matches!(instance, Value::Null) {
        return Err(error(
            "validation",
            "Standard networks cannot have a local instance identity",
        ));
    }
    let empty = Value::List(vec![]);
    let declared = get(args, "declared_wallet_ids").unwrap_or(&empty);
    if !matches!(declared, Value::List(v) if v.iter().all(|v| matches!(v, Value::Str(_)))) {
        return Err(error(
            "validation",
            "Wallet declarations must be wallet IDs",
        ));
    }
    let declared: BTreeSet<_> = list(declared).iter().map(py_str).collect();
    let inventory = inventory(conn, profile)?;
    let wallets = list(value(&inventory, "wallets"));
    let ids: BTreeSet<_> = wallets.iter().map(|w| text(w, "wallet_id")).collect();
    if !declared.is_subset(&ids) {
        return Err(error(
            "validation",
            "Wallet declaration is outside this book",
        ));
    }
    let mut blockers = Vec::new();
    if binding(conn, profile)?.is_some() {
        blockers.push(dict([("code", s("already_bound"))]));
    }
    for wallet in wallets {
        for (blocked, code) in [
            (
                truthy(value(wallet, "conflict_count")),
                "contradictory_evidence",
            ),
            (
                list(value(wallet, "environments"))
                    .iter()
                    .any(|v| v != &s(environment.as_str())),
                "different_environment",
            ),
            (
                list(value(wallet, "chain_instances"))
                    .iter()
                    .any(|v| v != &instance),
                "different_instance",
            ),
            (
                flag(wallet, "requires_declaration")
                    && !declared.contains(&text(wallet, "wallet_id")),
                "scope_declaration_required",
            ),
        ] {
            if blocked {
                blockers.push(dict([
                    ("code", s(code)),
                    ("wallet_id", value(wallet, "wallet_id").clone()),
                ]));
            }
        }
    }
    if list(value(&inventory, "reference_scopes")).iter().any(|r| {
        !flag(r, "valid")
            || nonempty(value(r, "environment"))
                && value(r, "environment") != &s(environment.as_str())
            || nonempty(value(r, "chain_instance_id")) && value(r, "chain_instance_id") != &instance
    }) {
        blockers.push(dict([("code", s("reference_domain_mismatch"))]));
    }
    let mut recipe = dict([
        ("profile_id", s(profile)),
        ("environment", s(environment.as_str())),
        ("chain_instance_id", instance.clone()),
        ("declared_wallet_ids", strings(&declared)),
        (
            "inventory_digest",
            value(&inventory, "inventory_digest").clone(),
        ),
    ]);
    let plan_id = digest(&recipe)?;
    replace(&mut recipe, "plan_id", s(plan_id));
    let domains = networks
        .iter()
        .map(|&(chain, network)| {
            Ok(dict([
                (
                    "domain_id",
                    s(digest(&Value::List(vec![
                        s(chain),
                        s(network),
                        instance.clone(),
                    ]))?),
                ),
                ("chain", s(chain)),
                ("network", s(network)),
                ("chain_instance_id", instance.clone()),
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    replace(&mut recipe, "domains", Value::List(domains));
    let can_apply = blockers.is_empty();
    replace(&mut recipe, "blockers", Value::List(blockers));
    replace(&mut recipe, "can_apply", Value::Bool(can_apply));
    replace(&mut recipe, "inventory", inventory);
    Ok(recipe)
}
pub fn bind(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    kassiber_store::begin_write_savepoint(conn, "setup_network_bind").map_err(sql_error)?;
    struct Rollback<'a>(&'a Connection, bool);
    impl Drop for Rollback<'_> {
        fn drop(&mut self) {
            if self.1 {
                let _ = self
                    .0
                    .execute_batch("ROLLBACK TO setup_network_bind; RELEASE setup_network_bind");
            }
        }
    }
    let mut rollback = Rollback(conn, true);
    let plan = plan(conn, profile, args)?;
    if value(args, "plan_id") != value(&plan, "plan_id") {
        return Err(error("stale_context", "Book network preview is stale"));
    }
    if !flag(&plan, "can_apply") {
        return Err(error(
            "book_network_review_required",
            "Book network binding needs review",
        )
        .with_details(dict([("blockers", value(&plan, "blockers").clone())])));
    }
    conn.execute("INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES(?,?,?,1,?,?,?,?,?)", rusqlite::params![profile, sources.uuid(conn)?, text(&plan,"environment"), optional(&plan,"chain_instance_id").map(py_str), json(value(&plan,"domains"),false,true)?, json(value(&plan,"declared_wallet_ids"),false,false)?, text(&plan,"inventory_digest"), sources.now(conn)?]).map_err(sql_error)?;
    conn.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?", [profile]).map_err(sql_error)?;
    let result = resolve(conn, profile)?;
    conn.execute_batch("RELEASE setup_network_bind")
        .map_err(sql_error)?;
    rollback.1 = false;
    Ok(result)
}
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let scope = if command.starts_with("ui.") {
        dict::<&str>([])
    } else {
        args.clone()
    };
    let (_, profile) = references::scope(conn, &scope)?;
    let profile = text(&profile, "id");
    let mut payload = args.clone();
    if let Some(declarations) = get(args, "declare_wallet") {
        replace(&mut payload, "declared_wallet_ids", declarations.clone());
    }
    if let Some(wallets) = get(args, "wallet") {
        replace(&mut payload, "wallet_ids", wallets.clone());
    }
    match command {
        "networks.inventory" | "ui.networks.inventory" => {
            let mut result = inventory(conn, &profile)?;
            replace(&mut result, "binding", resolve(conn, &profile)?);
            Ok(result)
        }
        "ui.networks.binding" => resolve(conn, &profile),
        "networks.plan" | "ui.networks.plan" => plan(conn, &profile, &payload),
        "networks.bind" | "ui.networks.bind" => bind(conn, &profile, &payload, sources),
        "networks.split-plan" | "ui.networks.partition_plan" => {
            partition::plan(conn, &profile, &payload)
        }
        _ => Err(AppError::new(
            "not_implemented",
            "network operation is not implemented in the Rust backend yet",
        )),
    }
}

#[cfg(test)]
mod read_tests {
    use super::*;

    #[test]
    fn lexical_negative_filter_keeps_escaped_nested_and_conflicting_evidence() {
        for (raw, expected) in [
            (r#"{"txid":"generic","amount":"1"}"#, (None, None, true)),
            (r#"{"invalid": "unterminated"#, (None, None, true)),
            (r#"{"nested":{"network":"test"}}"#, (None, None, true)),
            (r#"{"\u006eetwork":"test"}"#, (Some("test"), None, true)),
            (
                r#"{"network":"test","network":"main"}"#,
                (Some("main"), None, true),
            ),
            (
                r#"{"network":"regtest","chain_instance_id":"one"}"#,
                (Some("regtest"), Some("one"), true),
            ),
            (
                r#"{"network":"regtest","chain":"liquid"}"#,
                (None, None, false),
            ),
            (r#"{"network":"bad"}"#, (None, None, false)),
        ] {
            let row = dict([("raw_json", s(raw)), ("asset", s("BTC"))]);
            let (domain, instance, valid) = evidence(&row);
            assert_eq!(
                (domain.as_deref(), instance.as_deref(), valid),
                expected,
                "{raw}"
            );
        }
        let row = dict([
            ("raw_json", s(r#"{"network":"main"}"#)),
            ("wallet_config_json", s(r#"{"network":"test"}"#)),
            ("asset", s("BTC")),
        ]);
        assert!(!evidence(&row).2);
    }

    #[test]
    fn accounting_neutral_cache_is_wallet_and_asset_scoped() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE profiles(id TEXT,journal_input_version INTEGER); CREATE TABLE wallets(id TEXT,label TEXT,kind TEXT,config_json TEXT,profile_id TEXT); CREATE TABLE transactions(id TEXT,wallet_id TEXT,asset TEXT,raw_json TEXT,profile_id TEXT); INSERT INTO profiles VALUES('p',1); INSERT INTO wallets VALUES('a','A','custom','{\"network\":\"main\"}','p'),('b','B','custom','{\"network\":\"test\"}','p'); INSERT INTO transactions VALUES('a1','a','BTC','{}','p'),('a2','a','BTC','{}','p'),('b1','b','BTC','{}','p');").unwrap();
        let compact = inventory_snapshot(&conn, "p", false).unwrap();
        assert_eq!(value(&compact, "state"), &s("mixed"));
        assert_eq!(
            value(&compact, "environments"),
            &Value::List(vec![s("main"), s("test")])
        );
        conn.execute_batch("UPDATE wallets SET config_json='{\"chain\":\"bitcoin\"}'; UPDATE transactions SET asset='LBTC' WHERE id='a2';").unwrap();
        let compact = inventory_snapshot(&conn, "p", false).unwrap();
        assert_eq!(value(&compact, "state"), &s("conflicted"));
        // Even an otherwise redundant neutral row must still be decoded.
        conn.execute_batch("UPDATE transactions SET raw_json=x'aa' WHERE id='b1';")
            .unwrap();
        assert!(inventory_snapshot(&conn, "p", false).is_err());
    }

    #[test]
    fn accounting_and_review_read_the_same_network_facts_without_writes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE profiles(id TEXT,journal_input_version INTEGER); CREATE TABLE wallets(id TEXT,label TEXT,kind TEXT,config_json TEXT,profile_id TEXT); CREATE TABLE transactions(id TEXT,wallet_id TEXT,asset TEXT,raw_json TEXT,profile_id TEXT); INSERT INTO profiles VALUES('p',1); INSERT INTO wallets VALUES('w','Wallet','custom','{}','p'),('empty','Empty','custom','{}','p'); INSERT INTO transactions VALUES('t','w','BTC','{}','p');").unwrap();
        // Include multiple wallets, repeated neutral assets, a foreign-wallet
        // observation and configuration-only evidence on an empty wallet.
        conn.execute_batch("INSERT INTO wallets VALUES('other','Other','custom','{\"network\":\"test\"}','p'),('foreign','Foreign','custom','{}','other-profile'); INSERT INTO transactions VALUES('other-t','other','BTC','{}','p'),('foreign-t','foreign','BTC','{\"network\":\"signet\"}','p'); UPDATE wallets SET config_json='{\"network\":\"main\"}' WHERE id='empty';").unwrap();
        for raw in [
            "{}",
            r#"{"network":"main"}"#,
            r#"{"network":"unknown"}"#,
            r#"{"network":"regtest","chain_instance_id":"one"}"#,
            r#"{"\u006eetwork":"test"}"#,
        ] {
            conn.execute("UPDATE transactions SET raw_json=?", [raw])
                .unwrap();
            let before = conn.total_changes();
            let full = inventory_snapshot(&conn, "p", true).unwrap();
            let compact = inventory_snapshot(&conn, "p", false).unwrap();
            for key in [
                "profile_id",
                "state",
                "environments",
                "chain_instances",
                "transaction_count",
            ] {
                assert_eq!(value(&full, key), value(&compact, key), "{key}: {raw}");
            }
            assert_eq!(conn.total_changes(), before);
            assert!(truthy(value(&full, "inventory_digest")));
            assert_eq!(value(&compact, "inventory_digest"), &Value::Null);
        }
        conn.execute_batch("UPDATE wallets SET config_json='{\"chain\":\"bitcoin\"}' WHERE id='w'; UPDATE transactions SET raw_json='{}'; INSERT INTO transactions VALUES('t2','w','LBTC','{}','p'),('t3','w','BTC','{}','p');").unwrap();
        let inventory = inventory_snapshot(&conn, "p", true).unwrap();
        assert_eq!(value(&inventory, "state"), &s("conflicted"));
        let wallet = list(value(&inventory, "wallets"))
            .iter()
            .find(|row| value(row, "wallet_id") == &s("w"))
            .unwrap();
        assert_eq!(value(wallet, "unknown_count"), &Value::Int(2.into()));
        assert_eq!(value(wallet, "conflict_count"), &Value::Int(1.into()));
    }
}
