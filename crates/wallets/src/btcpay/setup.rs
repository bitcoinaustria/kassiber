//! `core/btcpay_setup.py`: join a server inspection with the local book into
//! a reviewable plan, validate chosen routes and apply them atomically.
//! Recommendations are advisory: a recognised wallet suggests a mapping but
//! never grants ownership, and every create path re-validates the routes so
//! one store wallet cannot become a second balance source.
use super::Result;
use kassiber_chain::btcpay::{
    discovery::Inspection,
    payment_methods::{self as methods, RAIL_LIGHTNING, RAIL_LNURL, RAIL_ONCHAIN},
    url::{self, from_py},
};
use kassiber_envelope::{dict, get, py_str, s, set, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_setup::backends::Config;
use kassiber_store::rusqlite::Connection;
use kassiber_transactions::btcpay_records as records;

pub const ACTIONS: [&str; 5] = [
    "wallet_source",
    "existing_wallet",
    "payment_ledger",
    "provenance_only",
    "skip",
];
const BALANCE_ACTIONS: [&str; 3] = ["wallet_source", "existing_wallet", "payment_ledger"];
pub const FINGERPRINT_CONFIG_KEY: &str = "btcpay_wallet_fingerprint";
const OWNERSHIP_SCAN_TO_INDEX: i64 = 25;
const LIGHTNING_WALLET_KINDS: [&str; 4] = ["lnd", "coreln", "nwc", "phoenix"];
const NULL: Value = Value::Null;

pub fn reason(code: &str) -> Option<&'static str> {
    Some(match code {
        "already_configured" => "Already set up in Kassiber.",
        "configured_via_other_key" => "Already set up through another API key for this server.",
        "non_bitcoin_asset" => "Kassiber tracks Bitcoin and Liquid Bitcoin only.",
        "disabled" => "This payment method is disabled in BTCPay.",
        "lightning_settles_elsewhere" => "A Lightning node is already connected to this book, so Kassiber keeps these invoices as provenance and links them by payment hash. Choose the payment ledger if that node is not this store's.",
        "lightning_payment_ledger" => "Kassiber cannot watch this store's Lightning node or wallet directly, so it books BTCPay's settled payments and completed payouts as the balance.",
        "plugin_payment_ledger" => "Plugin payment method: Kassiber books BTCPay's settled payments and completed payouts as the balance.",
        "plugin_rail" => "Plugin payment method in an unknown currency; invoices are kept as provenance.",
        "import_bark_wallet_history" => "Import the store's whole Bark wallet from BTCPay: payments, sends, refreshes and their fees.",
        "bark_payment_ledger" => "Kassiber books BTCPay's settled Bark payments and completed payouts as the balance. Once the Bark plugin serves wallet history (0.10 or later) to a key that can view the wallet, the next sync switches to the whole Bark wallet and keeps the payments already booked.",
        "bark_wallet_history_unavailable" => "This store's Bark plugin does not serve wallet history: it needs plugin 0.10 or later, and Bark set up for the store.",
        "not_ledger_rail" => "Only Lightning and Bitcoin plugin payment methods can be booked from BTCPay's payments.",
        "payouts_permission_missing" => "The key cannot read payouts, so BTCPay's payments alone would overstate the balance. Keep the invoices only, or use a key that can read payouts.",
        "configured_balance_source" => "Kassiber already takes this payment method's balance from a wallet set up here. Archive that wallet to set it up differently.",
        "mapped_to_wallet" => "This payment method is mapped to a wallet Kassiber tracks. Skip the mapping first so the wallet is not counted twice.",
        "network_mismatch" => "This store's wallet is on a different network than this book.",
        "wallet_recognised" => "Kassiber already tracks the wallet this store pays into.",
        "shared_wallet_already_imported" => "Another store already imports this same wallet.",
        "shared_wallet" => "Another store in this setup pays into the same wallet; it is imported once.",
        "import_btcpay_wallet_history" => "Import BTCPay's wallet history for this store.",
        "read_only_key_upgrade" => "This key cannot read BTCPay's wallet history. Create a key that can (on BTCPay Server 2.4 the read-only key with 'View wallets') to import the store wallet from BTCPay, or map it to a wallet Kassiber already tracks.",
        "wallet_history_permission_missing" => "Needs a key with BTCPay wallet-history access.",
        "invoices_permission_missing" => "The key cannot read invoices for this store.",
        "not_on_chain" => "Only on-chain Bitcoin and Liquid can be imported as a wallet.",
        _ => return None,
    })
}

fn reason_value(code: Option<&str>) -> Value {
    code.and_then(reason).map_or(Value::Null, s)
}

pub(crate) fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&NULL)
}

fn text(v: &Value) -> String {
    py_str(v)
}

/// `str(value or "")`.
fn or_empty(v: &Value) -> String {
    if truthy(v) {
        text(v)
    } else {
        String::new()
    }
}

fn lower_strip(v: &str) -> String {
    kassiber_pytext::lower(kassiber_pytext::strip(v))
}

/// `_config(row)`: the decoded wallet configuration, `{}` when unusable.
pub(crate) fn wallet_config(row: &Value) -> Value {
    let raw = field(row, "config_json");
    let decoded = if truthy(raw) {
        kassiber_pytext::loads(&text(raw)).ok()
    } else {
        kassiber_pytext::loads("{}").ok()
    };
    match decoded {
        Some(v @ Value::Dict(_)) => v,
        _ => dict::<&str>([]),
    }
}

fn rows(
    conn: &Connection,
    sql: &str,
    params: &[&dyn kassiber_store::rusqlite::ToSql],
) -> Result<Vec<Value>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let names: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
    let mut cursor = stmt
        .query(params)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let mut out = Vec::new();
    while let Some(row) = cursor
        .next()
        .map_err(|e| AppError::new("internal_error", e.to_string()))?
    {
        let mut pairs = Vec::new();
        for (index, name) in names.iter().enumerate() {
            use kassiber_store::rusqlite::types::ValueRef;
            let value = match row
                .get_ref(index)
                .map_err(|e| AppError::new("internal_error", e.to_string()))?
            {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::Int(n.into()),
                ValueRef::Real(f) => Value::Float(f),
                ValueRef::Text(t) => s(String::from_utf8_lossy(t).into_owned()),
                ValueRef::Blob(_) => Value::Null,
            };
            pairs.push((name.as_str(), value));
        }
        out.push(dict(pairs));
    }
    Ok(out)
}

/// `wallet_is_deprecated`: `parse_bool(config.get("deprecated"), False)`.
fn deprecated(config: &Value) -> Result<bool> {
    match field(config, "deprecated") {
        Value::Null => Ok(false),
        value => {
            let text = lower_strip(&text(value));
            match text.as_str() {
                "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(AppError::new(
                    "app_error",
                    format!("Invalid boolean value: {}", py_str(value)),
                )),
            }
        }
    }
}

/// `btcpay_ledger.is_payment_ledger_config`.
pub(crate) fn is_payment_ledger(config: &Value) -> bool {
    matches!(config, Value::Dict(_))
        && lower_strip(&or_empty(field(config, "source_mode"))) == "payments"
}

/// `btcpay_ledger.ledger_payment_method_ids`.
pub(crate) fn ledger_methods(config: &Value) -> Vec<String> {
    let values = match field(config, "payment_method_ids") {
        Value::List(items) => items.clone(),
        _ => vec![field(config, "payment_method_id").clone()],
    };
    let mut out: Vec<String> = Vec::new();
    for value in values {
        let method = methods::canonical(&or_empty(&value));
        if !method.is_empty() && methods::ledger_group(&method).is_some() && !out.contains(&method)
        {
            out.push(method);
        }
    }
    out
}

/// `wallet_btcpay_provenance_config`, raising as the reference does.
fn provenance_routes(config: &Value) -> Result<Vec<(String, String, String)>> {
    let raw = field(config, "btcpay_provenance");
    if matches!(raw, Value::Null) {
        return Ok(Vec::new());
    }
    let Value::List(items) = raw else {
        return Err(AppError::new(
            "validation",
            "BTCPay provenance config must be a list",
        ));
    };
    let mut out: Vec<(String, String, String)> = Vec::new();
    for item in items {
        if !matches!(item, Value::Dict(_)) {
            return Err(AppError::new(
                "validation",
                "BTCPay provenance routes must be objects",
            ));
        }
        let stripped = |key: &str| {
            let v = field(item, key);
            (!matches!(v, Value::Null))
                .then(|| kassiber_pytext::strip(&text(v)).to_owned())
                .filter(|t| !t.is_empty())
        };
        let (Some(backend), Some(store)) = (stripped("backend"), stripped("store_id")) else {
            return Err(AppError::new(
                "validation",
                "BTCPay provenance routes require backend and store_id",
            ));
        };
        let method = stripped("payment_method_id").unwrap_or_else(|| "BTC-CHAIN".into());
        let route = (
            kassiber_pytext::lower(&backend),
            store,
            methods::canonical(&method),
        );
        if !out.contains(&route) {
            out.push(route);
        }
    }
    Ok(out)
}

fn route(fields: Vec<(&str, Value)>) -> Value {
    dict(fields)
}

/// `same_server_backends`: saved BTCPay keys for the same server.
pub fn same_server_backends(backends: &Config, server_url: Option<&str>) -> Result<Vec<String>> {
    let Some(server_url) = server_url.filter(|u| !u.is_empty()) else {
        return Ok(Vec::new());
    };
    let identity = url::server_url_identity(server_url)?;
    let mut names = Vec::new();
    for name in backends.names() {
        let Some(backend) = backends
            .configured(&name)
            .filter(|v| matches!(v, Value::Dict(_)))
        else {
            continue;
        };
        if lower_strip(&or_empty(field(backend, "kind"))) != "btcpay" {
            continue;
        }
        let other = match field(backend, "url") {
            Value::Null => String::new(),
            v => text(v),
        };
        if url::server_url_identity(&other)? == identity {
            names.push(kassiber_pytext::lower(&if truthy(field(backend, "name")) {
                text(field(backend, "name"))
            } else {
                name.clone()
            }));
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// `list_existing_routes`.
pub fn existing_routes(
    conn: &Connection,
    profile_id: &str,
    backend_names: &[String],
) -> Result<Vec<Value>> {
    let mut wanted: Vec<String> = backend_names
        .iter()
        .filter(|n| !n.is_empty())
        .map(|n| lower_strip(n))
        .collect();
    wanted.sort();
    wanted.dedup();
    let mut routes = Vec::new();
    if wanted.is_empty() {
        return Ok(routes);
    }
    let wallets = rows(
        conn,
        "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ? ORDER BY label ASC",
        &[&profile_id],
    )?;
    for row in &wallets {
        let config = wallet_config(row);
        let source = or_empty(field(&config, "sync_source"));
        let backend = lower_strip(&text(field(&config, "backend")));
        let has_store = truthy(field(&config, "store_id"));
        let is_btcpay = source == "btcpay" && wanted.contains(&backend) && has_store;
        if is_btcpay && is_payment_ledger(&config) {
            for method in ledger_methods(&config) {
                routes.push(route(vec![
                    ("action", s("payment_ledger")),
                    ("backend", s(backend.as_str())),
                    ("wallet", field(row, "label").clone()),
                    ("wallet_id", field(row, "id").clone()),
                    ("store_id", s(text(field(&config, "store_id")))),
                    ("payment_method_id", s(method)),
                    ("wallet_fingerprint", Value::Null),
                    ("deprecated", Value::Bool(deprecated(&config)?)),
                ]));
            }
        } else if is_btcpay {
            routes.push(route(vec![
                ("action", s("wallet_source")),
                ("backend", s(backend.as_str())),
                ("wallet", field(row, "label").clone()),
                ("wallet_id", field(row, "id").clone()),
                ("store_id", s(text(field(&config, "store_id")))),
                (
                    "payment_method_id",
                    s(methods::canonical(
                        &if truthy(field(&config, "payment_method_id")) {
                            text(field(&config, "payment_method_id"))
                        } else {
                            "BTC-CHAIN".into()
                        },
                    )),
                ),
                (
                    "wallet_fingerprint",
                    field(&config, FINGERPRINT_CONFIG_KEY).clone(),
                ),
                ("deprecated", Value::Bool(deprecated(&config)?)),
            ]));
        }
        for (backend, store, method) in provenance_routes(&config).unwrap_or_default() {
            if !wanted.contains(&backend) {
                continue;
            }
            routes.push(route(vec![
                ("action", s("existing_wallet")),
                ("backend", s(backend)),
                ("wallet", field(row, "label").clone()),
                ("wallet_id", field(row, "id").clone()),
                ("store_id", s(store)),
                ("payment_method_id", s(method)),
                ("wallet_fingerprint", Value::Null),
                ("deprecated", Value::Bool(deprecated(&config)?)),
            ]));
        }
    }
    for backend in &wanted {
        for account in records::account_routes(conn, profile_id, Some(backend))? {
            let label = field(&account, "label");
            routes.push(route(vec![
                ("action", field(&account, "action").clone()),
                ("backend", field(&account, "backend").clone()),
                ("route_id", field(&account, "id").clone()),
                ("wallet", Value::Null),
                ("wallet_id", Value::Null),
                ("store_id", field(&account, "store_id").clone()),
                (
                    "payment_method_id",
                    field(&account, "payment_method_id").clone(),
                ),
                ("label", if truthy(label) { label.clone() } else { s("") }),
                ("wallet_fingerprint", Value::Null),
                ("deprecated", Value::Bool(false)),
            ]));
        }
    }
    Ok(routes)
}

/// `wallet_source_fingerprints`: imported store wallets by fingerprint.
pub fn source_fingerprints(
    conn: &Connection,
    profile_id: &str,
) -> Result<Vec<(String, Vec<Value>)>> {
    let mut grouped: Vec<(String, Vec<Value>)> = Vec::new();
    for row in rows(
        conn,
        "SELECT id, label, config_json FROM wallets WHERE profile_id = ?",
        &[&profile_id],
    )? {
        let config = wallet_config(&row);
        let fingerprint = field(&config, FINGERPRINT_CONFIG_KEY);
        if !truthy(fingerprint) || or_empty(field(&config, "sync_source")) != "btcpay" {
            continue;
        }
        if deprecated(&config)? {
            continue;
        }
        let entry = dict([
            ("wallet", field(&row, "label").clone()),
            ("wallet_id", field(&row, "id").clone()),
            (
                "backend",
                s(kassiber_pytext::lower(&or_empty(field(&config, "backend")))),
            ),
            ("store_id", s(or_empty(field(&config, "store_id")))),
            (
                "payment_method_id",
                s(methods::canonical(&{
                    let m = or_empty(field(&config, "payment_method_id"));
                    if m.is_empty() {
                        "BTC-CHAIN".to_owned()
                    } else {
                        m
                    }
                })),
            ),
        ]);
        let key = text(fingerprint);
        match grouped.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1.push(entry),
            None => grouped.push((key, vec![entry])),
        }
    }
    Ok(grouped)
}

/// `_book_network`.
pub(crate) fn book_network(conn: &Connection, profile_id: &str) -> Result<Value> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='book_network_bindings'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if exists {
        if let Some(row) = rows(
            conn,
            "SELECT * FROM book_network_bindings WHERE profile_id=?",
            &[&profile_id],
        )?
        .into_iter()
        .next()
        {
            return Ok(dict([
                ("state", s("bound")),
                ("environment", field(&row, "environment").clone()),
            ]));
        }
    }
    Ok(dict([
        ("state", s("unbound")),
        ("environment", Value::Null),
    ]))
}

/// `network_compatible`.
fn network_compatible(book: &Value, chain: &Value, network: &Value) -> Option<bool> {
    if !truthy(chain) || !truthy(network) || field(book, "state") != &s("bound") {
        return None;
    }
    let environment = or_empty(field(book, "environment"));
    let chain = text(chain);
    let allowed: Option<&[&str]> = match (environment.as_str(), chain.as_str()) {
        ("main", "bitcoin") => Some(&["main"]),
        ("main", "liquid") => Some(&["liquidv1"]),
        ("test", "bitcoin") => Some(&["test"]),
        ("test", "liquid") => Some(&["liquidtestnet"]),
        ("signet", "bitcoin") => Some(&["test", "signet"]),
        ("regtest", "bitcoin") => Some(&["regtest"]),
        ("regtest", "liquid") => Some(&["elementsregtest"]),
        _ => None,
    };
    Some(allowed.is_some_and(|a| a.contains(&text(network).as_str())))
}

/// `_ownership_matches`: one unambiguous owned wallet per previewed method.
fn ownership_matches(
    conn: &Connection,
    profile_id: &str,
    previews: &kassiber_chain::btcpay::discovery::Previews,
) -> Vec<((String, String), Value)> {
    let mut addresses: Vec<String> = previews
        .iter()
        .flat_map(|(_, m)| m.iter().flat_map(|(_, a)| a.iter().cloned()))
        .collect();
    if addresses.is_empty() {
        return Vec::new();
    }
    addresses.sort();
    addresses.dedup();
    let inputs = crate::identify::Inputs {
        addresses,
        scan_to_index: OWNERSHIP_SCAN_TO_INDEX,
        ..Default::default()
    };
    let Ok(report) = crate::identify::identify(conn, profile_id, &inputs) else {
        return Vec::new();
    };
    let mut owned: Vec<(String, Value)> = Vec::new();
    if let Value::List(results) = field(&report, "results") {
        for result in results {
            if field(result, "status") != &s("owned")
                || truthy(field(result, "ownership_ambiguous"))
            {
                continue;
            }
            let key = text(field(result, "input"));
            let hit = dict([
                ("wallet", field(result, "canonical_wallet").clone()),
                ("wallet_id", field(result, "canonical_wallet_id").clone()),
            ]);
            match owned.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = hit,
                None => owned.push((key, hit)),
            }
        }
    }
    let mut matches = Vec::new();
    for (store, methods_list) in previews {
        for (method, values) in methods_list {
            let hits: Vec<&Value> = values
                .iter()
                .filter_map(|a| owned.iter().find(|(k, _)| k == a).map(|(_, v)| v))
                .collect();
            let mut ids: Vec<String> = hits.iter().map(|h| text(field(h, "wallet_id"))).collect();
            ids.sort();
            ids.dedup();
            if ids.len() == 1 && !hits.is_empty() {
                let mut entry = hits[0].clone();
                set(
                    &mut entry,
                    "addresses_matched",
                    Value::Int(hits.len().into()),
                );
                set(
                    &mut entry,
                    "addresses_checked",
                    Value::Int(values.len().into()),
                );
                matches.push(((store.clone(), method.clone()), entry));
            }
        }
    }
    matches
}

fn lightning_wallets(conn: &Connection, profile_id: &str) -> Result<Value> {
    let found = rows(
        conn,
        "SELECT id, label, kind FROM wallets WHERE profile_id = ? AND kind IN (?,?,?,?) ORDER BY label",
        &[
            &profile_id,
            &LIGHTNING_WALLET_KINDS[0],
            &LIGHTNING_WALLET_KINDS[1],
            &LIGHTNING_WALLET_KINDS[2],
            &LIGHTNING_WALLET_KINDS[3],
        ],
    )?;
    Ok(Value::List(
        found
            .iter()
            .map(|row| {
                dict([
                    ("wallet", field(row, "label").clone()),
                    ("wallet_id", field(row, "id").clone()),
                    ("kind", field(row, "kind").clone()),
                ])
            })
            .collect(),
    ))
}

fn entry(available: bool, code: Option<&str>) -> Value {
    dict([
        ("available", Value::Bool(available)),
        ("reason", code.map_or(Value::Null, s)),
        ("reason_text", reason_value(code)),
    ])
}

/// `_action_availability`.
fn action_availability(method: &Value, caps: &Value) -> Value {
    let onchain =
        field(method, "rail") == &s(RAIL_ONCHAIN) && truthy(field(method, "sync_supported"));
    let bark = field(method, "payment_method_id") == &s(methods::BARK_PAYMENT_METHOD_ID);
    let invoices = field(caps, "invoices") != &Value::Bool(false);
    let history_denied = field(caps, "wallet_history") == &Value::Bool(false);
    let network_ok = field(method, "network_compatible") != &Value::Bool(false);
    let wallet_source = if !(onchain || bark) {
        entry(false, Some("not_on_chain"))
    } else if history_denied {
        entry(false, Some("wallet_history_permission_missing"))
    } else if bark && field(method, "wallet_history_available") != &Value::Bool(true) {
        entry(false, Some("bark_wallet_history_unavailable"))
    } else if !network_ok {
        entry(false, Some("network_mismatch"))
    } else {
        entry(true, None)
    };
    let existing = if onchain && network_ok {
        entry(true, None)
    } else if onchain {
        entry(false, Some("network_mismatch"))
    } else {
        entry(false, Some("not_on_chain"))
    };
    let provenance = if invoices {
        entry(true, None)
    } else {
        entry(false, Some("invoices_permission_missing"))
    };
    let ledger = if !truthy(field(method, "ledger_group")) {
        entry(false, Some("not_ledger_rail"))
    } else if !invoices {
        entry(false, Some("invoices_permission_missing"))
    } else if field(caps, "payouts") == &Value::Bool(false) {
        entry(false, Some("payouts_permission_missing"))
    } else {
        entry(true, None)
    };
    dict([
        ("wallet_source", wallet_source),
        ("existing_wallet", existing),
        ("payment_ledger", ledger),
        ("provenance_only", provenance),
        ("skip", entry(true, None)),
    ])
}

/// `_balance_conflict`.
fn balance_conflict(existing: &str, requested: &str, same_key: bool) -> Option<&'static str> {
    if ["wallet_source", "payment_ledger"].contains(&existing) {
        if same_key && requested != existing {
            return Some("configured_balance_source");
        }
        if !same_key && BALANCE_ACTIONS.contains(&requested) {
            return Some("configured_via_other_key");
        }
    }
    if existing == "existing_wallet" && ["wallet_source", "payment_ledger"].contains(&requested) {
        return Some(if same_key {
            "mapped_to_wallet"
        } else {
            "configured_via_other_key"
        });
    }
    None
}

fn lock_configured(actions: &mut Value, own: &[Value], other: &[Value]) {
    let Value::Dict(pairs) = actions else { return };
    let routes: Vec<(&Value, bool)> = own
        .iter()
        .map(|r| (r, true))
        .chain(other.iter().map(|r| (r, false)))
        .collect();
    for (key, availability) in pairs.iter_mut() {
        if !truthy(field(availability, "available")) {
            continue;
        }
        let Key::Str(action) = key else { continue };
        let action = from_py(action);
        for (route, same_key) in &routes {
            if truthy(field(route, "deprecated")) {
                continue;
            }
            if let Some(code) = balance_conflict(&text(field(route, "action")), &action, *same_key)
            {
                set(availability, "available", Value::Bool(false));
                set(availability, "reason", s(code));
                set(availability, "reason_text", reason_value(Some(code)));
                break;
            }
        }
    }
}

/// `detected_network`: the inspected server's Bitcoin network, if unambiguous.
pub fn detected_network(public: &Value) -> Value {
    let mut networks: Vec<String> = Vec::new();
    if let Value::List(methods_list) = field(public, "payment_methods") {
        for method in methods_list {
            if truthy(field(method, "network")) && field(method, "chain") == &s("bitcoin") {
                let network = text(field(method, "network"));
                if !networks.contains(&network) {
                    networks.push(network);
                }
            }
        }
    }
    if networks.len() == 1 {
        s(networks.remove(0))
    } else {
        Value::Null
    }
}

fn plan_summary(plan: &Value) -> Value {
    let mut counts: Vec<(&str, i64)> = ACTIONS.iter().map(|a| (*a, 0)).collect();
    let empty = Value::List(Vec::new());
    let list = |key: &str| match field(plan, key) {
        Value::List(items) => items.clone(),
        _ => match &empty {
            Value::List(items) => items.clone(),
            _ => Vec::new(),
        },
    };
    for method in list("payment_methods") {
        let action = text(field(field(&method, "recommendation"), "action"));
        if let Some(slot) = counts.iter_mut().find(|(a, _)| *a == action) {
            slot.1 += 1;
        }
    }
    dict([
        ("stores", Value::Int(list("stores").len().into())),
        (
            "payment_methods",
            Value::Int(list("payment_methods").len().into()),
        ),
        (
            "recommended",
            dict(counts.into_iter().map(|(a, n)| (a, Value::Int(n.into())))),
        ),
        ("key_risk", field(field(plan, "api_key"), "risk").clone()),
    ])
}

/// `plan_btcpay_setup`.
pub fn plan(
    conn: &Connection,
    profile_id: &str,
    inspection: &Inspection,
    backend_name: &str,
    backends: &Config,
    server_url: Option<&str>,
) -> Result<Value> {
    let mut public = inspection.public.clone();
    let backend_key = lower_strip(backend_name);
    let siblings: Vec<String> = same_server_backends(backends, server_url)?
        .into_iter()
        .filter(|n| *n != backend_key)
        .collect();
    let mut names = vec![backend_key.clone()];
    names.extend(siblings.iter().cloned());
    let existing = existing_routes(conn, profile_id, &names)?;
    let book = book_network(conn, profile_id)?;
    let ownership = ownership_matches(conn, profile_id, &inspection.previews);
    let imported = source_fingerprints(conn, profile_id)?;
    let lightning = lightning_wallets(conn, profile_id)?;
    let stores = match field(&public, "stores") {
        Value::List(items) => items.clone(),
        _ => Vec::new(),
    };
    let store_name = |id: &Value| -> String {
        stores
            .iter()
            .rev()
            .find(|store| field(store, "id") == id)
            .map(|store| {
                let name = field(store, "name");
                if truthy(name) {
                    text(name)
                } else {
                    text(id)
                }
            })
            .unwrap_or_default()
    };
    let caps_of = |id: &Value| -> Value {
        stores
            .iter()
            .rev()
            .find(|store| field(store, "id") == id)
            .map(|store| {
                let caps = field(store, "capabilities");
                if truthy(caps) {
                    caps.clone()
                } else {
                    dict::<&str>([])
                }
            })
            .unwrap_or_else(|| dict::<&str>([]))
    };
    let shared = field(&public, "shared_wallets").clone();
    let Some(Value::List(mut methods_list)) = get(&public, "payment_methods").cloned() else {
        return Err(AppError::new(
            "internal_error",
            "inspection without payment methods",
        ));
    };
    let mut order: Vec<usize> = (0..methods_list.len()).collect();
    order.sort_by_key(|&i| {
        let m = &methods_list[i];
        (
            kassiber_pytext::lower(&store_name(field(m, "store_id"))),
            text(field(m, "store_id")),
            text(field(m, "payment_method_id")),
        )
    });
    let mut planned: Vec<String> = Vec::new();
    for index in order {
        let method = &mut methods_list[index];
        let store_id = text(field(method, "store_id"));
        let method_id = text(field(method, "payment_method_id"));
        let caps = caps_of(field(method, "store_id"));
        let compatible =
            network_compatible(&book, field(method, "chain"), field(method, "network"));
        set(
            method,
            "network_compatible",
            compatible.map_or(Value::Null, Value::Bool),
        );
        let here: Vec<&Value> = existing
            .iter()
            .filter(|r| {
                text(field(r, "store_id")) == store_id
                    && text(field(r, "payment_method_id")) == method_id
            })
            .collect();
        let own: Vec<Value> = here
            .iter()
            .filter(|r| text(field(r, "backend")) == backend_key)
            .map(|r| (*r).clone())
            .collect();
        let other: Vec<Value> = here
            .iter()
            .filter(|r| text(field(r, "backend")) != backend_key)
            .map(|r| (*r).clone())
            .collect();
        set(method, "existing_routes", Value::List(own.clone()));
        let mut via: Vec<String> = other.iter().map(|r| text(field(r, "backend"))).collect();
        via.sort();
        via.dedup();
        set(
            method,
            "configured_via",
            Value::List(via.into_iter().map(s).collect()),
        );
        let owned = ownership
            .iter()
            .find(|((st, m), _)| *st == store_id && *m == method_id)
            .map(|(_, v)| v.clone());
        set(method, "owned_by", owned.clone().unwrap_or(Value::Null));
        let fingerprint = if truthy(field(method, "wallet_fingerprint")) {
            Some(text(field(method, "wallet_fingerprint")))
        } else {
            None
        };
        let imported_here: Vec<Value> = imported
            .iter()
            .find(|(k, _)| Some(k) == fingerprint.as_ref())
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| {
                if fingerprint.is_none() {
                    imported
                        .iter()
                        .find(|(k, _)| k.is_empty())
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            })
            .into_iter()
            .filter(|r| {
                !(text(field(r, "store_id")) == store_id
                    && text(field(r, "payment_method_id")) == method_id)
            })
            .collect();
        set(
            method,
            "same_wallet_imported_by",
            Value::List(imported_here.clone()),
        );
        let mut shared_with = Vec::new();
        if let Value::List(groups) = &shared {
            for group in groups {
                if field(group, "wallet_fingerprint") != field(method, "wallet_fingerprint") {
                    continue;
                }
                if let Value::List(members) = field(group, "members") {
                    for member in members {
                        if !(text(field(member, "store_id")) == store_id
                            && text(field(member, "payment_method_id")) == method_id)
                        {
                            shared_with.push(member.clone());
                        }
                    }
                }
            }
        }
        set(method, "shared_with", Value::List(shared_with));
        let bark_method = method_id == methods::BARK_PAYMENT_METHOD_ID;
        let mut actions = action_availability(method, &caps);
        lock_configured(&mut actions, &own, &other);
        set(method, "actions", actions);
        let rail = text(field(method, "rail"));
        let lightning_rail = rail == RAIL_LIGHTNING || rail == RAIL_LNURL;
        let has_lightning = matches!(&lightning, Value::List(items) if !items.is_empty());
        if lightning_rail {
            set(method, "lightning_wallets", lightning.clone());
        }
        let mut wallet = Value::Null;
        let (mut action, mut why): (&str, &str);
        if !own.is_empty() {
            let mut preferred = own.clone();
            preferred.sort_by_key(|r| {
                let a = text(field(r, "action"));
                ACTIONS.iter().position(|x| *x == a).unwrap_or(9)
            });
            let first = &preferred[0];
            let a = text(field(first, "action"));
            action = ACTIONS.iter().find(|x| **x == a).copied().unwrap_or("skip");
            wallet = field(first, "wallet").clone();
            why = "already_configured";
        } else if other
            .iter()
            .any(|r| BALANCE_ACTIONS.contains(&text(field(r, "action")).as_str()))
        {
            (action, why) = ("skip", "configured_via_other_key");
        } else if !truthy(field(method, "bitcoin_asset")) {
            (action, why) = ("skip", "non_bitcoin_asset");
        } else if get(method, "enabled").is_some_and(|v| !truthy(v)) {
            (action, why) = ("skip", "disabled");
        } else if lightning_rail {
            (action, why) = if has_lightning {
                ("provenance_only", "lightning_settles_elsewhere")
            } else {
                ("payment_ledger", "lightning_payment_ledger")
            };
        } else if bark_method && compatible == Some(false) {
            (action, why) = ("skip", "network_mismatch");
        } else if bark_method && field(method, "wallet_history_available") == &Value::Bool(true) {
            (action, why) = ("wallet_source", "import_bark_wallet_history");
        } else if bark_method {
            (action, why) = ("payment_ledger", "bark_payment_ledger");
        } else if rail != RAIL_ONCHAIN || !truthy(field(method, "sync_supported")) {
            (action, why) = if truthy(field(method, "ledger_group")) {
                ("payment_ledger", "plugin_payment_ledger")
            } else {
                ("provenance_only", "plugin_rail")
            };
        } else if compatible == Some(false) {
            (action, why) = ("skip", "network_mismatch");
        } else if let Some(owned) = &owned {
            action = "existing_wallet";
            wallet = field(owned, "wallet").clone();
            why = "wallet_recognised";
        } else if !imported_here.is_empty() {
            (action, why) = ("provenance_only", "shared_wallet_already_imported");
        } else if fingerprint.as_ref().is_some_and(|f| planned.contains(f)) {
            (action, why) = ("provenance_only", "shared_wallet");
        } else if field(&caps, "wallet_history") == &Value::Bool(false) {
            (action, why) = ("provenance_only", "read_only_key_upgrade");
        } else {
            (action, why) = ("wallet_source", "import_btcpay_wallet_history");
        }
        if action == "payment_ledger" && field(&caps, "payouts") == &Value::Bool(false) {
            (action, why) = ("provenance_only", "payouts_permission_missing");
        }
        if ["provenance_only", "payment_ledger"].contains(&action)
            && field(&caps, "invoices") == &Value::Bool(false)
        {
            (action, why) = ("skip", "invoices_permission_missing");
        }
        if let Some(f) = &fingerprint {
            if ["wallet_source", "existing_wallet"].contains(&action) && !planned.contains(f) {
                planned.push(f.clone());
            }
        }
        set(
            method,
            "recommendation",
            dict([
                ("action", s(action)),
                ("wallet", wallet),
                ("reason", if why.is_empty() { Value::Null } else { s(why) }),
                (
                    "reason_text",
                    reason_value((!why.is_empty()).then_some(why)),
                ),
            ]),
        );
    }
    set(&mut public, "payment_methods", Value::List(methods_list));
    let own_existing: Vec<&Value> = existing
        .iter()
        .filter(|r| text(field(r, "backend")) == backend_key && !truthy(field(r, "deprecated")))
        .collect();
    let mut configured: Vec<String> = own_existing
        .iter()
        .map(|r| text(field(r, "store_id")))
        .collect();
    configured.sort();
    configured.dedup();
    let pairs: Vec<(String, String)> = configured
        .iter()
        .map(|st| (backend_key.clone(), st.clone()))
        .collect();
    let states = records::store_sync_states(conn, profile_id, &pairs)?;
    let mut stale = Vec::new();
    if let Some(Value::List(mut store_list)) = get(&public, "stores").cloned() {
        for store in store_list.iter_mut() {
            let id = text(field(store, "id"));
            let state = states
                .iter()
                .find(|((b, st), _)| *b == backend_key && *st == id)
                .map(|(_, v)| v.clone());
            if state.as_ref().is_some_and(|st| truthy(field(st, "stale"))) {
                stale.push(field(store, "id").clone());
            }
            set(store, "sync_state", state.unwrap_or(Value::Null));
        }
        set(&mut public, "stores", Value::List(store_list));
    }
    if !stale.is_empty() {
        let warning = dict([
            ("code", s("stale_store_data")),
            ("severity", s("info")),
            (
                "message",
                s(format!("BTCPay does not notify Kassiber of new invoices or payouts. Data for {} configured store(s) is over a day old or its last refresh failed; sync to bring it up to date.", stale.len())),
            ),
            ("store_ids", Value::List(stale)),
        ]);
        let mut warnings = match field(&public, "warnings") {
            Value::List(items) => items.clone(),
            _ => Vec::new(),
        };
        warnings.push(warning);
        set(&mut public, "warnings", Value::List(warnings));
    }
    set(&mut public, "backend", s(backend_key.as_str()));
    set(
        &mut public,
        "sibling_backends",
        Value::List(siblings.into_iter().map(s).collect()),
    );
    set(
        &mut public,
        "existing_routes",
        Value::List(
            existing
                .iter()
                .filter(|r| text(field(r, "backend")) == backend_key)
                .cloned()
                .collect(),
        ),
    );
    set(&mut public, "book_network", book);
    set(&mut public, "lightning_wallets", lightning);
    let detected = detected_network(&public);
    set(&mut public, "detected_network", detected);
    let summary = plan_summary(&public);
    set(&mut public, "summary", summary);
    Ok(public)
}

// --- Validating and applying reviewed routes -------------------------------------------

const ROUTE_NETWORKS: [&str; 7] = [
    "main",
    "test",
    "signet",
    "regtest",
    "liquidv1",
    "liquidtestnet",
    "elementsregtest",
];

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// `_route_text`: the first non-blank string among `keys`.
fn route_text(raw: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match field(raw, key) {
        Value::Str(t) => {
            let t = kassiber_pytext::strip(&from_py(t)).to_owned();
            (!t.is_empty()).then_some(t)
        }
        _ => None,
    })
}

fn opt(v: Option<String>) -> Value {
    v.map_or(Value::Null, s)
}

/// `normalize_setup_routes`.
pub fn normalize_routes(raw: &Value, allow_only_skips: bool) -> Result<Vec<Value>> {
    let items = match raw {
        Value::List(items) if !items.is_empty() => items,
        _ => return Err(
            validation("BTCPay account setup requires at least one route").with_hint(
                "Discover stores, then choose what Kassiber should do with each payment method.",
            ),
        ),
    };
    let mut routes = Vec::new();
    let mut seen: Vec<(String, String, String, String)> = Vec::new();
    for item in items {
        if !matches!(item, Value::Dict(_)) {
            return Err(validation("BTCPay account setup routes must be objects"));
        }
        let action = kassiber_pytext::lower(
            &route_text(item, &["action", "mode"]).unwrap_or_else(|| "skip".into()),
        );
        let action = match action.as_str() {
            "create" | "create_wallet" | "wallet" => "wallet_source".to_owned(),
            "map" | "map_existing" | "settlement_wallet" => "existing_wallet".to_owned(),
            "provenance" | "invoice_provenance" => "provenance_only".to_owned(),
            "ledger" | "payments" => "payment_ledger".to_owned(),
            other => other.to_owned(),
        };
        if !ACTIONS.contains(&action.as_str()) {
            return Err(validation(format!(
                "Unsupported BTCPay account route action '{action}'"
            )));
        }
        let store = route_text(item, &["store_id"])
            .ok_or_else(|| validation("BTCPay store ID is required"))?;
        let method = methods::canonical(
            &route_text(item, &["payment_method_id"]).unwrap_or_else(|| "BTC-CHAIN".into()),
        );
        if method.is_empty() {
            return Err(validation("BTCPay payment method id cannot be empty"));
        }
        if action == "wallet_source" {
            methods::require_wallet_history(&method, &method)?;
        }
        if action == "existing_wallet" {
            methods::require_settlement(&method, &method)?;
        }
        if action == "payment_ledger" && methods::ledger_group(&method).is_none() {
            return Err(validation(format!(
                "BTCPay payment method '{method}' cannot be booked as a payment ledger"
            ))
            .with_hint("Import on-chain methods as wallet history or map them to a tracked wallet; payment ledgers are for Lightning and Bitcoin plugin rails."));
        }
        let wallet = route_text(item, &["wallet", "target_wallet"]);
        if action == "existing_wallet" && wallet.is_none() {
            return Err(validation(
                "BTCPay account settlement routes require a wallet",
            ));
        }
        let network = route_text(item, &["network"]);
        if let Some(network) = &network {
            if !ROUTE_NETWORKS.contains(&network.as_str()) {
                return Err(validation(format!(
                    "Unsupported BTCPay route network '{network}'"
                )));
            }
        }
        let key = (
            store.clone(),
            method.clone(),
            action.clone(),
            wallet.clone().unwrap_or_default(),
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        routes.push(dict([
            ("store_id", s(store)),
            ("store_name", opt(route_text(item, &["store_name"]))),
            ("payment_method_id", s(method)),
            ("label", opt(route_text(item, &["label"]))),
            ("action", s(action)),
            ("wallet", opt(wallet)),
            (
                "wallet_fingerprint",
                opt(route_text(item, &["wallet_fingerprint"])),
            ),
            ("network", opt(network)),
        ]));
    }
    if !allow_only_skips && routes.iter().all(|r| field(r, "action") == &s("skip")) {
        return Err(validation("BTCPay account setup has no selected routes")
            .with_hint("Choose at least one wallet source, settlement mapping, payment ledger, or provenance-only route."));
    }
    Ok(routes)
}

fn conflict(message: String, hint: &str, details: Vec<(&str, Value)>) -> AppError {
    AppError::new("conflict", message)
        .with_hint(hint)
        .with_details(dict(details))
}

/// `validate_setup_routes`: refuse route sets that would import one store
/// wallet twice or add a second balance source.
pub fn validate_routes(
    conn: &Connection,
    profile_id: &str,
    routes: &[Value],
    backend_name: &str,
    backends: &Config,
    server_url: Option<&str>,
) -> Result<()> {
    let backend_key = lower_strip(backend_name);
    let siblings: Vec<String> = same_server_backends(backends, server_url)?
        .into_iter()
        .filter(|n| *n != backend_key)
        .collect();
    let mut names = vec![backend_key.clone()];
    names.extend(siblings.iter().cloned());
    let existing: Vec<Value> = existing_routes(conn, profile_id, &names)?
        .into_iter()
        .filter(|r| !truthy(field(r, "deprecated")))
        .collect();
    for route in routes {
        for found in existing.iter().filter(|e| {
            field(e, "store_id") == field(route, "store_id")
                && field(e, "payment_method_id") == field(route, "payment_method_id")
        }) {
            let existing_action = text(field(found, "action"));
            let same_key = text(field(found, "backend")) == backend_key;
            if let Some(code) =
                balance_conflict(&existing_action, &text(field(route, "action")), same_key)
            {
                let wallet = field(found, "wallet");
                let suffix = if truthy(wallet) {
                    format!(" ('{}')", text(wallet))
                } else {
                    String::new()
                };
                return Err(conflict(
                    format!(
                        "Store {} {} is already set up as {existing_action}{suffix}",
                        text(field(route, "store_id")),
                        text(field(route, "payment_method_id"))
                    ),
                    reason(code).unwrap_or_default(),
                    vec![
                        ("reason", s(code)),
                        ("existing_action", s(existing_action.as_str())),
                        ("existing_wallet", wallet.clone()),
                        ("existing_backend", field(found, "backend").clone()),
                    ],
                ));
            }
        }
    }
    if !siblings.is_empty() {
        let sibling_routes: Vec<Value> = existing_routes(conn, profile_id, &siblings)?
            .into_iter()
            .filter(|r| !truthy(field(r, "deprecated")))
            .collect();
        let pick = |action: &str,
                    key: &dyn Fn(&Value) -> (String, Option<String>)|
         -> Vec<((String, Option<String>), Value)> {
            let mut out: Vec<((String, Option<String>), Value)> = Vec::new();
            for r in sibling_routes
                .iter()
                .filter(|r| field(r, "action") == &s(action))
            {
                let k = key(r);
                match out.iter_mut().find(|(existing, _)| *existing == k) {
                    Some(slot) => slot.1 = r.clone(),
                    None => out.push((k, r.clone())),
                }
            }
            out
        };
        let sources = pick("wallet_source", &|r| {
            (
                text(field(r, "store_id")),
                Some(text(field(r, "payment_method_id"))),
            )
        });
        let ledgers = pick("payment_ledger", &|r| {
            (
                text(field(r, "store_id")),
                methods::ledger_group(&text(field(r, "payment_method_id"))),
            )
        });
        for route in routes {
            let action = text(field(route, "action"));
            let store = text(field(route, "store_id"));
            let method = text(field(route, "payment_method_id"));
            if action == "payment_ledger" {
                let key = (store.clone(), methods::ledger_group(&method));
                if let Some((_, clash)) = ledgers.iter().find(|(k, _)| *k == key) {
                    return Err(conflict(
                        format!("Store {store} {method} payments are already booked by '{}' through another API key", text(field(clash, "wallet"))),
                        "Keep one payment ledger per store rail, or keep this key's routes as provenance only.",
                        vec![("existing_wallet", field(clash, "wallet").clone()), ("existing_backend", field(clash, "backend").clone())],
                    ));
                }
                continue;
            }
            if action != "wallet_source" {
                continue;
            }
            if let Some((_, clash)) = sources
                .iter()
                .find(|(k, _)| *k == (store.clone(), Some(method.clone())))
            {
                return Err(conflict(
                    format!("Store {store} {method} is already imported as '{}' through another API key", text(field(clash, "wallet"))),
                    "Keep one wallet source per store wallet. Map this store to the existing wallet or skip it.",
                    vec![("existing_wallet", field(clash, "wallet").clone()), ("existing_backend", field(clash, "backend").clone())],
                ));
            }
        }
    }
    let imported = source_fingerprints(conn, profile_id)?;
    let mut seen: Vec<(String, &Value)> = Vec::new();
    for route in routes {
        if field(route, "action") != &s("wallet_source") {
            continue;
        }
        let fingerprint = field(route, "wallet_fingerprint");
        if !truthy(fingerprint) {
            continue;
        }
        let fingerprint = text(fingerprint);
        if let Some((_, previous)) = seen.iter().find(|(k, _)| *k == fingerprint) {
            let mut stores = vec![
                text(field(previous, "store_id")),
                text(field(route, "store_id")),
            ];
            stores.sort();
            stores.dedup();
            return Err(conflict(
                "Two selected stores pay into the same wallet; importing both would double-count it".into(),
                "Import the wallet once and keep the other store as invoice provenance.",
                vec![
                    ("stores", Value::List(stores.into_iter().map(s).collect())),
                    ("wallet_fingerprint", s(fingerprint.as_str())),
                ],
            ));
        }
        seen.push((fingerprint.clone(), route));
        for existing in imported
            .iter()
            .filter(|(k, _)| *k == fingerprint)
            .flat_map(|(_, v)| v.iter())
        {
            if field(existing, "store_id") == field(route, "store_id")
                && field(existing, "payment_method_id") == field(route, "payment_method_id")
            {
                continue;
            }
            return Err(conflict(
                format!(
                    "This store's wallet is already imported as '{}'",
                    text(field(existing, "wallet"))
                ),
                "Keep this store as invoice provenance, or map it to that wallet.",
                vec![
                    ("existing_wallet", field(existing, "wallet").clone()),
                    ("wallet_fingerprint", s(fingerprint.as_str())),
                ],
            ));
        }
    }
    Ok(())
}

fn db(e: kassiber_store::rusqlite::Error) -> AppError {
    AppError::new("internal_error", e.to_string())
}

fn wallet_get(conn: &Connection, profile_id: &str, reference: &str) -> Result<Value> {
    crate::lifecycle::execute(
        conn,
        crate::Operation::Get {
            profile_id: profile_id.into(),
            wallet: reference.into(),
        },
    )
}

fn wallet_update(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    config: serde_json::Value,
    clear: Vec<String>,
) -> Result<Value> {
    crate::lifecycle::execute(
        conn,
        crate::Operation::Update {
            profile_id: profile_id.into(),
            wallet: wallet_id.into(),
            label: None,
            account: None,
            config,
            clear,
        },
    )
}

fn wallet_create(
    conn: &Connection,
    scope: &Scope<'_>,
    label: &str,
    config: serde_json::Value,
) -> Result<Value> {
    crate::lifecycle::execute(
        conn,
        crate::Operation::Create {
            workspace_id: scope.workspace_id.into(),
            profile_id: scope.profile_id.into(),
            label: label.into(),
            kind: "custom".into(),
            account: None,
            config,
        },
    )
}

pub struct Scope<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub profile_label: &'a str,
}

/// `delete_btcpay_account_route` with its freshness source.
fn delete_account_route(
    conn: &Connection,
    profile_id: &str,
    backend: &str,
    store: &str,
    method: &str,
) -> Result<()> {
    let removed: Option<String> = conn
        .query_row(
            "DELETE FROM btcpay_account_routes WHERE profile_id = ? AND backend_name = ? AND store_id = ? AND payment_method_id = ? AND action = 'provenance_only' RETURNING id",
            [profile_id, backend, store, method],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            kassiber_store::rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(db(e)),
        })?;
    if let Some(id) = removed {
        let key = format!("btcpay_provenance:account:{id}");
        for table in ["freshness_jobs", "freshness_source_states"] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE profile_id = ? AND source_key IN (?)"),
                [profile_id, &key],
            )
            .map_err(db)?;
        }
    }
    Ok(())
}

fn label_taken(conn: &Connection, profile_id: &str, label: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?",
        [profile_id, label],
        |_| Ok(true),
    )
    .or_else(|e| match e {
        kassiber_store::rusqlite::Error::QueryReturnedNoRows => Ok(false),
        e => Err(db(e)),
    })
}

fn label_conflict(label: &str, profile_label: &str) -> AppError {
    AppError::new(
        "conflict",
        format!("Wallet '{label}' already exists in profile '{profile_label}'"),
    )
    .with_hint("Choose a different connection label or skip this already-used route.")
    .with_details(dict([("existing_labels", Value::List(vec![s(label)]))]))
}

/// `wallet_source_config`.
pub fn wallet_source_config(
    backend: &str,
    store: &str,
    method: &str,
    fingerprint: Option<&str>,
    network: Option<&str>,
) -> serde_json::Value {
    let info = methods::classify(method);
    let mut config = serde_json::json!({
        "backend": backend,
        "store_id": store,
        "payment_method_id": info.payment_method_id,
        "sync_source": "btcpay",
    });
    if let Some(chain) = info.chain {
        config["chain"] = chain.into();
    }
    if let Some(network) = network.filter(|n| !n.is_empty()) {
        config["network"] = network.into();
    }
    if let Some(fingerprint) = fingerprint.filter(|f| !f.is_empty()) {
        config[FINGERPRINT_CONFIG_KEY] = fingerprint.into();
    }
    config
}

fn wallets_custom(conn: &Connection, profile_id: &str) -> Result<Vec<Value>> {
    rows(
        conn,
        "SELECT * FROM wallets WHERE profile_id = ? AND kind = 'custom' ORDER BY label ASC",
        &[&profile_id],
    )
}

fn store_part(route: &Value) -> String {
    let name = field(route, "store_name");
    let raw = if truthy(name) {
        text(name)
    } else {
        text(field(route, "store_id"))
    };
    let part = kassiber_pytext::strip(&raw).to_owned();
    if part.is_empty() {
        text(field(route, "store_id"))
    } else {
        part
    }
}

fn opt_str(v: &Value) -> Option<String> {
    truthy(v).then(|| text(v))
}

/// `apply_setup_routes`: wallet sources, mappings, ledgers and account routes
/// in one savepoint; no network I/O.
pub fn apply_routes(
    conn: &Connection,
    scope: &Scope<'_>,
    backend_name: &str,
    routes: &[Value],
    label: &str,
) -> Result<Value> {
    kassiber_store::begin_write_savepoint(conn, "btcpay_setup").map_err(db)?;
    let result = apply_inner(conn, scope, backend_name, routes, label);
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK TO btcpay_setup");
    }
    conn.execute_batch("RELEASE btcpay_setup").map_err(db)?;
    result
}

fn apply_inner(
    conn: &Connection,
    scope: &Scope<'_>,
    backend_name: &str,
    routes: &[Value],
    label: &str,
) -> Result<Value> {
    let profile = scope.profile_id;
    let backend_key = lower_strip(backend_name);
    let (mut sources, mut mappings, mut accounts, mut ledgers, mut skipped) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut ledger_routes: Vec<((String, String), Vec<&Value>)> = Vec::new();
    let mut reused = 0;
    let mut stores: Vec<String> = Vec::new();
    let add_store = |stores: &mut Vec<String>, store: &str| {
        if !stores.iter().any(|s| s == store) {
            stores.push(store.to_owned());
        }
    };
    for route in routes {
        let action = text(field(route, "action"));
        let store = text(field(route, "store_id"));
        let method = text(field(route, "payment_method_id"));
        if action != "provenance_only" {
            delete_account_route(conn, profile, &backend_key, &store, &method)?;
        }
        match action.as_str() {
            "skip" => {
                skipped.push(route.clone());
                if let Some(wallet_ref) = opt_str(field(route, "wallet")) {
                    let wallet = wallet_get(conn, profile, &wallet_ref)?;
                    let existing = match field(field(&wallet, "config"), "btcpay_provenance") {
                        Value::List(items) => items.clone(),
                        _ => Vec::new(),
                    };
                    let remaining: Vec<Value> = existing
                        .iter()
                        .filter(|item| {
                            !(lower_strip(&or_empty(field(item, "backend"))) == backend_key
                                && field(item, "store_id") == &s(store.as_str())
                                && methods::canonical(&{
                                    let m = or_empty(field(item, "payment_method_id"));
                                    if m.is_empty() {
                                        "BTC-CHAIN".into()
                                    } else {
                                        m
                                    }
                                }) == method)
                        })
                        .cloned()
                        .collect();
                    if remaining.len() != existing.len() {
                        set_wallet_routes(conn, profile, &text(field(&wallet, "id")), remaining)?;
                    }
                }
            }
            "wallet_source" => {
                let source_label = match opt_str(field(route, "label")) {
                    Some(l) => kassiber_pytext::strip(&l).to_owned(),
                    None => format!("{label} - {} - {method}", store_part(route)),
                };
                let mut found = None;
                for row in wallets_custom(conn, profile)? {
                    let config = wallet_config(&row);
                    if or_empty(field(&config, "sync_source")) == "btcpay"
                        && !is_payment_ledger(&config)
                        && kassiber_pytext::lower(&or_empty(field(&config, "backend")))
                            == backend_key
                        && or_empty(field(&config, "store_id")) == store
                        && methods::canonical(&{
                            let m = or_empty(field(&config, "payment_method_id"));
                            if m.is_empty() {
                                "BTC-CHAIN".into()
                            } else {
                                m
                            }
                        }) == method
                    {
                        found = Some(row);
                        break;
                    }
                }
                if let Some(row) = found {
                    sources.push(wallet_get(conn, profile, &text(field(&row, "id")))?);
                    reused += 1;
                } else {
                    if label_taken(conn, profile, &source_label)? {
                        return Err(label_conflict(&source_label, scope.profile_label));
                    }
                    let config = wallet_source_config(
                        &backend_key,
                        &store,
                        &method,
                        opt_str(field(route, "wallet_fingerprint")).as_deref(),
                        opt_str(field(route, "network")).as_deref(),
                    );
                    sources.push(wallet_create(conn, scope, &source_label, config)?);
                }
                add_store(&mut stores, &store);
            }
            "existing_wallet" => {
                let wallet = wallet_get(conn, profile, &or_empty(field(route, "wallet")))?;
                if or_empty(field(field(&wallet, "config"), "sync_source")) == "btcpay" {
                    return Err(validation(format!(
                        "Wallet '{}' is itself a BTCPay wallet source",
                        text(field(&wallet, "label"))
                    ))
                    .with_hint("Map BTCPay stores onto the wallet that holds the funds (a descriptor, xpub, or Liquid wallet)."));
                }
                let mut existing = match field(field(&wallet, "config"), "btcpay_provenance") {
                    Value::List(items) => items.clone(),
                    _ => Vec::new(),
                };
                let next = dict([
                    ("backend", s(backend_key.as_str())),
                    ("store_id", s(store.as_str())),
                    ("payment_method_id", s(method.as_str())),
                ]);
                if !existing.contains(&next) {
                    existing.push(next.clone());
                }
                let updated =
                    set_wallet_routes(conn, profile, &text(field(&wallet, "id")), existing)?;
                mappings.push(dict([("wallet", updated), ("route", next)]));
                add_store(&mut stores, &store);
            }
            "payment_ledger" => {
                let group = methods::ledger_group(&method).unwrap_or_else(|| method.clone());
                let key = (store.clone(), group);
                match ledger_routes.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1.push(route),
                    None => ledger_routes.push((key, vec![route])),
                }
                add_store(&mut stores, &store);
            }
            _ => {
                let label =
                    opt_str(field(route, "store_name")).or_else(|| opt_str(field(route, "label")));
                accounts.push(records::upsert_account_route(
                    conn,
                    &kassiber_transactions::Scope {
                        workspace_id: scope.workspace_id,
                        profile_id: profile,
                    },
                    &backend_key,
                    &store,
                    &method,
                    label.as_deref(),
                )?);
                add_store(&mut stores, &store);
            }
        }
    }
    ledger_routes.sort_by(|a, b| a.0.cmp(&b.0));
    for ((store, group), grouped) in ledger_routes {
        let ledger_label = match grouped.iter().find_map(|r| opt_str(field(r, "label"))) {
            Some(l) => kassiber_pytext::strip(&l).to_owned(),
            None => {
                let first = grouped[0];
                let rail = if group == "lightning" {
                    "Lightning".to_owned()
                } else {
                    methods::classify(&text(field(first, "payment_method_id"))).label
                };
                format!("{label} - {} - {rail}", store_part(first))
            }
        };
        let route_methods: Vec<String> = grouped
            .iter()
            .map(|r| text(field(r, "payment_method_id")))
            .collect();
        let network = grouped.iter().find_map(|r| opt_str(field(r, "network")));
        let mut found = None;
        for row in wallets_custom(conn, profile)? {
            let config = wallet_config(&row);
            if is_payment_ledger(&config)
                && kassiber_pytext::lower(&or_empty(field(&config, "backend"))) == backend_key
                && or_empty(field(&config, "store_id")) == store
                && ledger_methods(&config)
                    .iter()
                    .any(|m| methods::ledger_group(m).as_deref() == Some(group.as_str()))
            {
                found = Some((row, config));
                break;
            }
        }
        if let Some((row, config)) = found {
            let current = ledger_methods(&config);
            let mut merged = current.clone();
            merged.extend(route_methods.iter().cloned());
            merged.sort();
            merged.dedup();
            let mut current_sorted = current;
            current_sorted.sort();
            let id = text(field(&row, "id"));
            if merged == current_sorted {
                ledgers.push(wallet_get(conn, profile, &id)?);
            } else {
                ledgers.push(wallet_update(
                    conn,
                    profile,
                    &id,
                    serde_json::json!({"payment_method_ids": merged}),
                    Vec::new(),
                )?);
            }
            reused += 1;
            continue;
        }
        if label_taken(conn, profile, &ledger_label)? {
            return Err(label_conflict(&ledger_label, scope.profile_label));
        }
        let mut sorted = route_methods.clone();
        sorted.sort();
        sorted.dedup();
        let mut config = serde_json::json!({
            "backend": backend_key,
            "store_id": store,
            "payment_method_id": sorted[0],
            "sync_source": "btcpay",
            "source_mode": "payments",
            "payment_method_ids": sorted,
            "chain": "bitcoin",
        });
        if let Some(network) = network {
            config["network"] = network.into();
        }
        ledgers.push(wallet_create(conn, scope, &ledger_label, config)?);
    }
    stores.sort();
    let mut wallets: Vec<Value> = sources.iter().chain(ledgers.iter()).cloned().collect();
    wallets.extend(mappings.iter().map(|m| field(m, "wallet").clone()));
    Ok(dict([
        ("wallet", wallets.first().cloned().unwrap_or(Value::Null)),
        ("wallets", Value::List(wallets)),
        ("wallet_sources", Value::List(sources)),
        ("payment_ledgers", Value::List(ledgers)),
        ("reused_wallets", Value::Int(reused.into())),
        ("mappings", Value::List(mappings)),
        ("account_routes", Value::List(accounts)),
        ("skipped", Value::List(skipped)),
        (
            "provenance_store_ids",
            Value::List(stores.into_iter().map(s).collect()),
        ),
    ]))
}

fn set_wallet_routes(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    routes: Vec<Value>,
) -> Result<Value> {
    if routes.is_empty() {
        return wallet_update(
            conn,
            profile_id,
            wallet_id,
            serde_json::json!({}),
            vec!["btcpay_provenance".into()],
        );
    }
    let encoded = kassiber_pyjson::dumps(&Value::List(routes), &Default::default())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let routes: serde_json::Value = serde_json::from_str(&encoded)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    wallet_update(
        conn,
        profile_id,
        wallet_id,
        serde_json::json!({"btcpay_provenance": routes}),
        Vec::new(),
    )
}

/// A `--route` override: `(store, method)` to `(action, wallet)`.
pub type RouteOverride = ((String, String), (String, Option<String>));

/// `routes_from_plan`: recommendations plus `STORE[:METHOD]` overrides.
pub fn routes_from_plan(
    plan: &Value,
    overrides: &[RouteOverride],
    use_recommendations: bool,
) -> Result<Vec<Value>> {
    let mut routes = Vec::new();
    let methods_list = match field(plan, "payment_methods") {
        Value::List(items) => items.clone(),
        _ => Vec::new(),
    };
    for method in &methods_list {
        let key = (
            text(field(method, "store_id")),
            text(field(method, "payment_method_id")),
        );
        let recommendation = field(method, "recommendation");
        let override_ = overrides
            .iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v);
        let (action, wallet) = if let Some((action, wallet)) = override_ {
            let wallet = match wallet {
                Some(w) if !w.is_empty() => s(w.as_str()),
                _ if action == "existing_wallet" => field(recommendation, "wallet").clone(),
                _ => Value::Null,
            };
            (action.clone(), wallet)
        } else if use_recommendations
            && truthy(field(recommendation, "action"))
            && field(recommendation, "action") != &s("skip")
        {
            (
                text(field(recommendation, "action")),
                field(recommendation, "wallet").clone(),
            )
        } else {
            continue;
        };
        let availability = field(field(method, "actions"), &action);
        if truthy(availability) && field(availability, "available") == &Value::Bool(false) {
            let reason_text = field(availability, "reason_text");
            let hint = if truthy(reason_text) {
                text(reason_text)
            } else {
                reason(&or_empty(field(availability, "reason")))
                    .unwrap_or("")
                    .to_owned()
            };
            return Err(validation(format!(
                "{} for store {} cannot use '{action}'",
                key.1, key.0
            ))
            .with_hint(hint)
            .with_details(dict([("reason", field(availability, "reason").clone())])));
        }
        let store_name = match field(plan, "stores") {
            Value::List(stores) => stores
                .iter()
                .find(|st| field(st, "id") == field(method, "store_id"))
                .map(|st| field(st, "name").clone())
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let network = field(method, "network");
        // Lightning and plugin rails expose no addresses; ledgers and the
        // Bark wallet take the server's on-chain network.
        let bark_source = action == "wallet_source"
            && field(method, "payment_method_id") == &s(methods::BARK_PAYMENT_METHOD_ID);
        let network = if truthy(network) {
            network.clone()
        } else if action == "payment_ledger" || bark_source {
            field(plan, "detected_network").clone()
        } else {
            Value::Null
        };
        routes.push(dict([
            ("store_id", field(method, "store_id").clone()),
            ("store_name", store_name),
            (
                "payment_method_id",
                field(method, "payment_method_id").clone(),
            ),
            ("action", s(action)),
            ("wallet", wallet),
            (
                "wallet_fingerprint",
                field(method, "wallet_fingerprint").clone(),
            ),
            ("network", network),
        ]));
    }
    let known: Vec<(String, String)> = methods_list
        .iter()
        .map(|m| {
            (
                text(field(m, "store_id")),
                text(field(m, "payment_method_id")),
            )
        })
        .collect();
    let mut unknown: Vec<&(String, String)> = overrides
        .iter()
        .map(|(k, _)| k)
        .filter(|k| !known.contains(k))
        .collect();
    unknown.sort();
    if let Some((store, method)) = unknown.first() {
        return Err(AppError::new(
            "not_found",
            format!("Store {store} has no enabled payment method {method} visible to this key"),
        )
        .with_hint(
            "Run `kassiber btcpay inspect --backend NAME` to list the stores and payment methods.",
        ));
    }
    Ok(routes)
}
