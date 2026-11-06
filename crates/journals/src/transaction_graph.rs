//! `build_transaction_graph_snapshot` (`ui.transactions.graph`): the curated
//! transaction-flow graph for the desktop detail view.
//!
//! Physical input/output ownership comes from local observation evidence and
//! the bounded owned index; booked custody annotations come only from the
//! current stored journal projection. A graph connection or an owned output is
//! never ownership authority, a booked transfer or a tax fact. The payload
//! never exposes raw transaction JSON, wallet configuration, descriptors,
//! xpubs, backend URLs or credentials. This read uses local references and the
//! stored lookup cache only; a public reference lookup is refused before any
//! backend is selected.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use kassiber_envelope::{dict, get, py_str, s, set, truthy, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};

use crate::report_blockers::{rows, sql_error, val};

type Result<T> = std::result::Result<T, AppError>;

const MAX_NODES_PER_SIDE: usize = 250;
const COINBASE_TXID: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const COINBASE_VOUT: i64 = 0xFFFF_FFFF;
const POLICY_ASSET_IDS: &[(&str, &str)] = &[
    (
        "liquidv1",
        "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
    ),
    (
        "main",
        "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
    ),
    (
        "liquidtestnet",
        "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
    ),
    (
        "test",
        "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
    ),
    (
        "elementsregtest",
        "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
    ),
    (
        "regtest",
        "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
    ),
];

// ------------------------------------------------------------ scalars

fn g<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn is_dict(value: &Value) -> bool {
    matches!(value, Value::Dict(_))
}

fn list(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::List(items) => Some(items),
        _ => None,
    }
}

/// `str(value)` for a present value, `""` for `None`.
fn text(value: &Value) -> String {
    if *value == Value::Null {
        String::new()
    } else {
        py_str(value)
    }
}

/// `str(value or "")`.
fn text_or(value: &Value) -> String {
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

fn lower(text: &str) -> String {
    kassiber_pytext::lower(text)
}

fn upper(text: &str) -> String {
    kassiber_pytext::upper(text)
}

fn strip(text: &str) -> String {
    kassiber_pytext::strip(text).to_owned()
}

/// `_string_or_none`.
fn string_or_none(value: &Value) -> Option<String> {
    if *value == Value::Null {
        return None;
    }
    Some(strip(&py_str(value))).filter(|v| !v.is_empty())
}

/// `_int_or_none`: Python's `int(value)`, `None` for blank or unconvertible.
fn int_or_none(value: &Value) -> Option<i128> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(i128::from(*b)),
        Value::Int(n) => i128::try_from(n.clone()).ok(),
        Value::Float(f) if f.is_finite() => Some(f.trunc() as i128),
        Value::Float(_) => None,
        Value::Str(text) if text.as_str() == Some("") => None,
        Value::Str(text) => kassiber_pytext::parse_int(text.as_str().unwrap_or_default())
            .ok()
            .and_then(|n| i128::try_from(n).ok()),
        _ => None,
    }
}

fn looks_like_txid(value: &Value) -> bool {
    let text = strip(&text_or(value));
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `_json_obj`.
fn json_obj(value: &Value) -> Value {
    match value {
        Value::Dict(_) => value.clone(),
        Value::Str(text) => {
            let raw = text.as_str().unwrap_or_default();
            kassiber_pytext::loads(if raw.is_empty() { "{}" } else { raw })
                .ok()
                .filter(is_dict)
                .unwrap_or_else(|| dict::<&str>([]))
        }
        Value::Null => dict::<&str>([]),
        _ => dict::<&str>([]),
    }
}

fn sats_to_btc(value: Option<i128>) -> Value {
    value.map_or(Value::Null, |v| Value::Float(v as f64 / 100_000_000.0))
}

fn msat_to_btc(value: i128) -> Value {
    format!("{value}e-11")
        .parse::<f64>()
        .map_or(Value::Null, Value::Float)
}

fn int_value(value: i128) -> Value {
    Value::Int(value.into())
}

fn opt_int(value: Option<i128>) -> Value {
    value.map_or(Value::Null, int_value)
}

/// `{key: value for ... if value is not None}`.
fn without_none(value: Value) -> Value {
    match value {
        Value::Dict(fields) => Value::Dict(
            fields
                .into_iter()
                .filter(|(_, v)| *v != Value::Null)
                .collect(),
        ),
        other => other,
    }
}

fn humanize(code: &str) -> String {
    let replaced = strip(&code.replace(['_', '-'], " "));
    let mut chars = replaced.chars();
    match chars.next() {
        Some(first) => format!("{}{}", upper(&first.to_string()), lower(chars.as_str())),
        None => String::new(),
    }
}

fn annotation(code: &str, label: &str, group: Option<Value>) -> Value {
    let mut item = dict([("code", s(code)), ("label", s(label))]);
    if let Some(group) = group {
        set(&mut item, "groupId", group);
    }
    item
}

/// `bytes.fromhex(text)`: whitespace may separate whole bytes.
fn from_hex(text: &str) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_whitespace() {
            continue;
        }
        let high = c.to_digit(16)?;
        let low = chars.next()?.to_digit(16)?;
        output.push((high * 16 + low) as u8);
    }
    Some(output)
}

/// `round(value, 2)`.
fn round2(value: f64) -> f64 {
    format!("{value:.2}").parse().unwrap_or(value)
}

// -------------------------------------------------------- chain scope

fn serde(value: &Value) -> serde_json::Value {
    kassiber_custody::codec::to_json(value)
}

fn normalize_chain(value: &str) -> Option<&'static str> {
    match value {
        "" | "bitcoin" | "btc" | "onchain" => Some("bitcoin"),
        "liquid" | "lbtc" | "elements" => Some("liquid"),
        _ => None,
    }
}

fn normalize_network(chain: &str, value: &str) -> Option<String> {
    kassiber_custody::evidence::network(chain, &serde_json::Value::String(value.to_owned())).ok()
}

/// `ownership_transfers._norm_chain_network`.
fn norm_chain_network(chain: &Value, network: &Value) -> (String, String) {
    let raw_chain = lower(&strip(&text_or(chain)));
    if let Some(canonical) = normalize_chain(if raw_chain.is_empty() {
        "bitcoin"
    } else {
        &raw_chain
    }) {
        if let Some(network) = normalize_network(canonical, &text_or(network)) {
            return (canonical.into(), network);
        }
    }
    (raw_chain, lower(&strip(&text_or(network))))
}

fn stored_mapping(value: &Value) -> Value {
    kassiber_custody::confirmation::stored_mapping(&serde(value), false)
        .and_then(|v| kassiber_custody::codec::from_json(&v).ok())
        .unwrap_or_else(|| dict::<&str>([]))
}

/// `transaction_references.reference_scope`.
fn reference_scope(
    row: &Value,
    default_chain: Option<&str>,
    default_network: Option<&str>,
) -> Option<(String, String)> {
    let (domain, _, valid) = kassiber_setup::networks::evidence(row);
    if !valid {
        return None;
    }
    let mut scoped = row.clone();
    let raw = stored_mapping(g(row, "raw_json"));
    let config_input = if truthy(g(row, "config_json")) {
        g(row, "config_json")
    } else {
        g(row, "wallet_config_json")
    };
    let config = stored_mapping(config_input);
    if let Some(chain) = default_chain.filter(|c| !c.is_empty()) {
        if !(truthy(g(&raw, "chain")) || truthy(g(&config, "chain")) || truthy(g(row, "chain"))) {
            set(&mut scoped, "chain", s(chain));
        }
    }
    let (protocol, network) = kassiber_custody::evidence::scope(&serde(&scoped)).ok()?;
    let base = if protocol == "lightning" {
        "bitcoin".to_owned()
    } else {
        protocol
    };
    let selected = if let Some(domain) = domain {
        if base == "liquid" {
            match domain.as_str() {
                "main" => "liquidv1".to_owned(),
                "test" => "liquidtestnet".to_owned(),
                "regtest" => "regtest".to_owned(),
                _ => return None,
            }
        } else {
            domain
        }
    } else if truthy(g(&raw, "network"))
        || truthy(g(&config, "network"))
        || truthy(g(row, "network"))
    {
        network
    } else {
        default_network
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .unwrap_or(network)
    };
    let normalized = normalize_network(&base, &selected)?;
    Some((base, normalized))
}

/// `_row_chain_network`.
fn row_chain_network(row: &Value, default_chain: &str, default_network: &str) -> (String, String) {
    let config = json_obj(g(row, "wallet_config_json"));
    let raw = json_obj(g(row, "raw_json"));
    let pick = |key: &str, default: &str| {
        let value = [g(&raw, key), g(&config, key)]
            .into_iter()
            .find(|v| truthy(v))
            .map(py_str)
            .unwrap_or_else(|| default.to_owned());
        lower(&value)
    };
    let mut chain = pick("chain", default_chain);
    let mut network = pick("network", default_network);
    let asset = upper(&text_or(g(row, "asset")));
    let wallet_kind = lower(&text_or(g(row, "wallet_kind")));
    if wallet_kind.contains("liquid") || ["LBTC", "L-BTC", "LIQUID-BTC"].contains(&asset.as_str()) {
        chain = "liquid".into();
        if ["", "main", "mainnet"].contains(&network.as_str()) {
            network = "liquidv1".into();
        }
    } else if ["", "btc"].contains(&chain.as_str()) {
        chain = "bitcoin".into();
    }
    reference_scope(row, Some(&chain), Some(&network))
        .unwrap_or_else(|| ("unknown".into(), "unknown".into()))
}

fn default_row_chain_network(row: &Value) -> (String, String) {
    row_chain_network(row, "bitcoin", "main")
}

// ------------------------------------------------------ onchain legs

/// `onchain.output_script`.
fn output_script(entry: &Value) -> Value {
    for key in ["scriptpubkey", "script_hex", "script_pubkey"] {
        if let Some(value) = get(entry, key).filter(|v| **v != Value::Null) {
            return value.clone();
        }
    }
    let nested = g(entry, "scriptPubKey");
    if is_dict(nested) {
        let hex = g(nested, "hex");
        return if truthy(hex) {
            hex.clone()
        } else {
            g(nested, "scriptpubkey").clone()
        };
    }
    if matches!(nested, Value::Str(_)) {
        return nested.clone();
    }
    Value::Null
}

/// `onchain.output_address`.
fn output_address(entry: &Value) -> Option<String> {
    let mut address = [g(entry, "scriptpubkey_address"), g(entry, "address")]
        .into_iter()
        .find(|v| truthy(v))
        .cloned()
        .unwrap_or(Value::Null);
    let nested = g(entry, "scriptPubKey");
    if (address == Value::Null || address == s("")) && is_dict(nested) {
        address = g(nested, "address").clone();
        if address == Value::Null || address == s("") {
            if let Some(first) = list(g(nested, "addresses")).and_then(|a| a.first()) {
                address = first.clone();
            }
        }
    }
    if address == Value::Null || address == s("") {
        return None;
    }
    Some(strip(&py_str(&address))).filter(|v| !v.is_empty())
}

/// `onchain.output_value_sats`.
fn output_value_sats(entry: &Value) -> Option<i128> {
    let explicit = *g(entry, "value_sats") != Value::Null;
    let value = if explicit {
        g(entry, "value_sats")
    } else {
        g(entry, "value")
    };
    match value {
        Value::Null | Value::Bool(_) => None,
        Value::Str(t) if t.as_str() == Some("") => None,
        Value::Int(n) => i128::try_from(n.clone()).ok(),
        _ if explicit => match value {
            Value::Float(f) if f.is_finite() => Some(f.trunc() as i128),
            Value::Str(t) => kassiber_pytext::parse_int(t.as_str().unwrap_or_default())
                .ok()
                .and_then(|n| i128::try_from(n).ok()),
            _ => None,
        },
        Value::Str(t) if !t.as_str().unwrap_or_default().contains('.') => {
            kassiber_pytext::parse_int(t.as_str().unwrap_or_default())
                .ok()
                .and_then(|n| i128::try_from(n).ok())
        }
        _ => {
            use kassiber_pydecimal::{Context, Decimal, Rounding};
            let spelling = py_str(value);
            let decimal: Decimal = strip(&spelling).parse().ok()?;
            let product = decimal
                .mul(&Decimal::from(100_000_000), &Context::RP2)
                .ok()?;
            product
                .to_fixed_places(0, Rounding::HalfEven)
                .ok()?
                .parse()
                .ok()
        }
    }
}

/// `_confidential_leg`.
fn confidential_leg(entry: &Value) -> bool {
    let marker = [g(entry, "value_state"), g(entry, "valueState")]
        .into_iter()
        .find(|v| truthy(v))
        .map(|v| lower(&strip(&py_str(v))))
        .unwrap_or_default();
    if marker == "confidential" {
        return true;
    }
    [
        "valuecommitment",
        "assetcommitment",
        "surjectionproof",
        "rangeproof",
    ]
    .iter()
    .any(|key| *g(entry, key) != Value::Null)
}

fn raw_has_confidential_legs(raw: &Value) -> bool {
    ["vin", "vout"].iter().any(|key| {
        list(g(raw, key)).is_some_and(|legs| {
            legs.iter().filter(|leg| is_dict(leg)).any(|leg| {
                confidential_leg(leg)
                    || (is_dict(g(leg, "prevout")) && confidential_leg(g(leg, "prevout")))
            })
        })
    })
}

fn looks_liquid_or_confidential(row: &Value, raw: &Value) -> bool {
    let asset = upper(&text_or(g(row, "asset")));
    if ["LBTC", "L-BTC", "LIQUID-BTC"].contains(&asset.as_str()) {
        return true;
    }
    if lower(&text_or(g(row, "wallet_kind"))).contains("liquid") {
        return true;
    }
    raw_has_confidential_legs(raw)
}

fn is_liquid_fee_output(entry: &Value) -> bool {
    let explicit = [
        g(entry, "scriptpubkey_type"),
        g(entry, "type"),
        g(entry, "role"),
    ]
    .into_iter()
    .find(|v| truthy(v))
    .and_then(string_or_none);
    if explicit.is_some_and(|t| lower(&t) == "fee") {
        return true;
    }
    if confidential_leg(entry) {
        return false;
    }
    if truthy(g(entry, "scriptpubkey_address")) {
        return false;
    }
    let script = [g(entry, "scriptpubkey"), g(entry, "script_hex")]
        .into_iter()
        .find(|v| truthy(v))
        .and_then(string_or_none);
    if script.is_some() {
        return false;
    }
    output_value_sats(entry).is_some()
}

fn canonical_bitcoin_asset(value: &str) -> Option<&'static str> {
    match upper(&strip(value)).as_str() {
        "BTC" | "XBT" | "XXBT" => Some("BTC"),
        "LBTC" | "L-BTC" => Some("LBTC"),
        _ => None,
    }
}

fn normalize_asset_code(value: &str) -> String {
    let text = strip(value);
    if text.is_empty() {
        return String::new();
    }
    let lowered = lower(&text);
    if lowered.len() == 64
        && lowered
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return lowered;
    }
    if ["l-btc", "lbtc"].contains(&lowered.as_str()) {
        return "LBTC".into();
    }
    upper(&text)
}

fn liquid_asset_code(asset_id: &str, policy: Option<&str>) -> String {
    let normalized = normalize_asset_code(asset_id);
    let policy = normalize_asset_code(policy.unwrap_or_default());
    if !normalized.is_empty() && !policy.is_empty() && normalized == policy {
        return "LBTC".into();
    }
    normalized
}

/// `_leg_asset`.
fn leg_asset(entry: &Value, policy: Option<&str>) -> Option<String> {
    let id = [g(entry, "asset_id"), g(entry, "assetId")]
        .into_iter()
        .find(|v| truthy(v))
        .and_then(string_or_none);
    if let Some(id) = id {
        let code = liquid_asset_code(&id, policy);
        return Some(if code.is_empty() { lower(&id) } else { code });
    }
    let asset = string_or_none(g(entry, "asset"))?;
    if asset.chars().count() == 64 {
        let code = liquid_asset_code(&asset, policy);
        return Some(if code.is_empty() { lower(&asset) } else { code });
    }
    Some(upper(&asset))
}

fn policy_asset_id(row: &Value) -> Option<String> {
    let config = json_obj(g(row, "wallet_config_json"));
    if let Some(configured) = string_or_none(g(&config, "policy_asset")) {
        if canonical_bitcoin_asset(&configured).is_none() && configured.chars().count() == 64 {
            return Some(lower(&configured));
        }
    }
    let (_, network) = row_chain_network(row, "liquid", "liquidv1");
    let key = lower(&strip(&network));
    POLICY_ASSET_IDS
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, id)| (*id).to_owned())
}

fn axis_is_liquid(row: &Value, confidential: bool) -> bool {
    default_row_chain_network(row).0 == "liquid" || confidential
}

fn same_asset(left: Option<&str>, right: Option<&str>) -> bool {
    let (Some(left), Some(right)) = (
        left.filter(|v| !v.is_empty()),
        right.filter(|v| !v.is_empty()),
    ) else {
        return true;
    };
    if left == right {
        return true;
    }
    let a = canonical_bitcoin_asset(left);
    a.is_some() && a == canonical_bitcoin_asset(right)
}

fn short_asset(asset: Option<&str>) -> Value {
    match asset.filter(|v| !v.is_empty()) {
        None => Value::Null,
        Some(asset) if asset.chars().count() == 64 => {
            s(format!("{}…", asset.chars().take(8).collect::<String>()))
        }
        Some(asset) => s(asset),
    }
}

fn flag(entry: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        match g(entry, key) {
            Value::Bool(b) => return Some(*b),
            Value::Str(t)
                if ["true", "1"]
                    .contains(&lower(&strip(t.as_str().unwrap_or_default())).as_str()) =>
            {
                return Some(true)
            }
            _ => {}
        }
    }
    None
}

fn is_coinbase_leg(entry: &Value) -> bool {
    for key in ["is_coinbase", "isCoinbase", "coinbase"] {
        match g(entry, key) {
            Value::Bool(b) => return *b,
            Value::Str(t)
                if ["true", "1"]
                    .contains(&lower(&strip(t.as_str().unwrap_or_default())).as_str()) =>
            {
                return true
            }
            _ => {}
        }
    }
    let txid = string_or_none(g(entry, "txid"));
    txid.is_some_and(|t| lower(&t) == COINBASE_TXID)
        && int_or_none(g(entry, "vout")) == Some(COINBASE_VOUT.into())
}

fn is_pegin_leg(entry: &Value) -> bool {
    flag(entry, &["is_pegin", "isPegin", "pegin"]).unwrap_or(false)
}

fn pegout_address(entry: &Value) -> Option<String> {
    for key in ["pegout_address", "pegoutAddress"] {
        if let Some(address) = string_or_none(g(entry, key)) {
            return Some(address);
        }
    }
    let pegout = g(entry, "pegout");
    if is_dict(pegout) {
        let value = [
            g(pegout, "scriptpubkey_address"),
            g(pegout, "scriptpubkeyAddress"),
        ]
        .into_iter()
        .find(|v| truthy(v))
        .cloned()
        .unwrap_or(Value::Null);
        return string_or_none(&value);
    }
    None
}

/// `_outpoint`: `"{txid}:{vout}"`, `None` when either is missing.
fn outpoint(entry: &Value) -> Option<String> {
    let (txid, vout) = (g(entry, "txid"), g(entry, "vout"));
    if *txid == Value::Null || *vout == Value::Null {
        return None;
    }
    let vout = match vout {
        Value::Str(t) => kassiber_pytext::parse_int(t.as_str().unwrap_or_default())
            .ok()
            .map(|n| n.to_string()),
        other => int_or_none(other).map(|n| n.to_string()),
    }?;
    Some(format!("{}:{vout}", lower(&py_str(txid))))
}

fn script_type(source: &Value, script: Option<&str>) -> String {
    let explicit = [g(source, "scriptpubkey_type"), g(source, "type")]
        .into_iter()
        .find(|v| truthy(v))
        .and_then(string_or_none);
    if let Some(explicit) = explicit {
        return explicit;
    }
    let script = lower(script.unwrap_or_default());
    if script.is_empty() {
        return if output_script(source) == s("") {
            "empty".into()
        } else {
            "unknown".into()
        };
    }
    for (prefix, kind) in [
        ("6a", "op_return"),
        ("0014", "p2wpkh"),
        ("0020", "p2wsh"),
        ("5120", "p2tr"),
    ] {
        if script.starts_with(prefix) {
            return kind.into();
        }
    }
    "script".into()
}

fn is_unspendable(script: Option<&str>) -> bool {
    lower(&strip(script.unwrap_or_default())).starts_with("6a")
}

fn txid_from_row(row: &Value) -> Option<String> {
    let raw = json_obj(g(row, "raw_json"));
    if let Some(txid) = string_or_none(g(&raw, "txid")) {
        return Some(lower(&txid));
    }
    let external = string_or_none(g(row, "external_id"))?;
    Some(if external.chars().count() == 64 {
        lower(&external)
    } else {
        external
    })
}

// ------------------------------------------------- transaction sizes

fn le(bytes: &[u8], start: u128, len: u128) -> u128 {
    let start = start.min(bytes.len() as u128) as usize;
    let end = (start as u128 + len).min(bytes.len() as u128) as usize;
    bytes[start..end]
        .iter()
        .rev()
        .fold(0u128, |acc, b| acc.wrapping_shl(8) | u128::from(*b))
}

fn varint(bytes: &[u8], offset: u128) -> Option<(u128, u128)> {
    let prefix = *bytes.get(usize::try_from(offset).ok()?)?;
    let offset = offset + 1;
    Some(match prefix {
        0..=0xFC => (u128::from(prefix), offset),
        0xFD => (le(bytes, offset, 2), offset + 2),
        0xFE => (le(bytes, offset, 4), offset + 4),
        0xFF => (le(bytes, offset, 8), offset + 8),
    })
}

/// `decode_raw_transaction(raw_hex)` sizes: only when the walk consumes the
/// payload exactly; `None` where CPython raises `IndexError`.
fn decoded_sizes(payload: &[u8]) -> Option<Vec<(&'static str, i128)>> {
    let mut offset: u128 = 4;
    let witness = payload.len() as u128 > offset + 1 && payload[4] == 0 && payload[5] != 0;
    if witness {
        offset += 2;
    }
    let (inputs, next) = varint(payload, offset)?;
    offset = next;
    let mut count = 0u128;
    while count < inputs {
        offset += 36;
        let (length, next) = varint(payload, offset)?;
        offset = next + length + 4;
        count += 1;
    }
    let (outputs, next) = varint(payload, offset)?;
    offset = next;
    count = 0;
    while count < outputs {
        offset += 8;
        let (length, next) = varint(payload, offset)?;
        offset = next + length;
        count += 1;
    }
    let witness_start = offset;
    if witness {
        count = 0;
        while count < inputs {
            let (items, next) = varint(payload, offset)?;
            offset = next;
            let mut item = 0u128;
            while item < items {
                let (length, next) = varint(payload, offset)?;
                offset = next + length;
                item += 1;
            }
            count += 1;
        }
    }
    if offset + 4 != payload.len() as u128 {
        return Some(Vec::new());
    }
    let size = payload.len() as i128;
    let stripped = size
        - if witness {
            2 + (offset - witness_start) as i128
        } else {
            0
        };
    let weight = stripped * 3 + size;
    Some(vec![
        ("size", size),
        ("vsize", (weight + 3) / 4),
        ("weight", weight),
    ])
}

fn byte_size(raw_hex: &str) -> Vec<(&'static str, i128)> {
    from_hex(raw_hex).map_or_else(Vec::new, |bytes| vec![("size", bytes.len() as i128)])
}

/// `_transaction_metadata`.
fn transaction_metadata(raw: &Value, confidential: bool) -> Value {
    let mut metadata = dict(
        ["version", "locktime", "size", "vsize", "weight"]
            .map(|key| (key, opt_int(int_or_none(g(raw, key))))),
    );
    let raw_hex = [g(raw, "raw_hex"), g(raw, "hex")]
        .into_iter()
        .find(|v| truthy(v))
        .and_then(string_or_none);
    if let Some(raw_hex) = raw_hex {
        let derived = if confidential {
            byte_size(&raw_hex)
        } else {
            match from_hex(&raw_hex).map(|bytes| decoded_sizes(&bytes)) {
                Some(Some(sizes)) if !sizes.is_empty() => sizes,
                _ => byte_size(&raw_hex),
            }
        };
        for (key, value) in derived {
            if *g(&metadata, key) == Value::Null {
                set(&mut metadata, key, int_value(value));
            }
        }
    }
    metadata
}

// ------------------------------------------------------------- graph

fn lookup_warnings(raw: &Value) -> Vec<Value> {
    let warning = g(raw, "_graphLookupWarning");
    if !is_dict(warning) {
        return Vec::new();
    }
    vec![dict([
        (
            "code",
            s(if truthy(g(warning, "code")) {
                py_str(g(warning, "code"))
            } else {
                "graph_lookup_warning".into()
            }),
        ),
        (
            "level",
            s(if truthy(g(warning, "level")) {
                py_str(g(warning, "level"))
            } else {
                "warning".into()
            }),
        ),
        (
            "message",
            s(if truthy(g(warning, "message")) {
                py_str(g(warning, "message"))
            } else {
                "Public transaction reference lookup did not return a complete graph.".into()
            }),
        ),
    ])]
}

fn with_lookup_warning(raw: &Value, code: &str, message: &str) -> Value {
    let mut output = raw.clone();
    set(
        &mut output,
        "_graphLookupWarning",
        dict([
            ("code", s(code)),
            ("level", s("warning")),
            ("message", s(message)),
        ]),
    );
    output
}

fn needs_prevout_lookup(entry: &Value) -> bool {
    if !is_dict(entry) {
        return false;
    }
    let prevout = g(entry, "prevout");
    let empty = dict::<&str>([]);
    let prevout = if is_dict(prevout) { prevout } else { &empty };
    if confidential_leg(prevout) {
        return false;
    }
    output_value_sats(prevout).is_none()
}

fn can_lookup_public_bitcoin(row: &Value, raw: &Value) -> bool {
    if looks_liquid_or_confidential(row, raw) {
        return false;
    }
    let asset = upper(&text_or(g(row, "asset")));
    if !asset.is_empty() && asset != "BTC" {
        return false;
    }
    match (list(g(raw, "vin")), list(g(raw, "vout"))) {
        (Some(vin), Some(_)) => vin.iter().any(needs_prevout_lookup),
        _ => true,
    }
}

fn has_required_prevouts(raw: &Value) -> bool {
    let (Some(vin), Some(_)) = (list(g(raw, "vin")), list(g(raw, "vout"))) else {
        return false;
    };
    for entry in vin {
        if !is_dict(entry) {
            return false;
        }
        let txid = string_or_none(g(entry, "txid"));
        let vout = int_or_none(g(entry, "vout"));
        if txid.is_none() && vout.is_none() {
            continue;
        }
        if txid.is_none() || vout.is_none() {
            return false;
        }
        let prevout = g(entry, "prevout");
        if !is_dict(prevout)
            || output_value_sats(prevout).is_none()
            || string_or_none(&output_script(prevout)).is_none()
        {
            return false;
        }
    }
    true
}

fn graph_wire(value: &serde_json::Value) -> Result<Value> {
    kassiber_custody::codec::from_json(value).map_err(|e| AppError::new("internal_error", e))
}

/// `_enrich_reference_graph_raw` without a backend: stored local and cached
/// references only.
fn enrich(
    conn: &Connection,
    profile_id: &str,
    row: &Value,
    raw: Value,
    allow_public_lookup: bool,
) -> Result<Value> {
    let liquid = looks_liquid_or_confidential(row, &raw);
    let (label, prefix) = if liquid {
        ("Liquid", "liquid")
    } else {
        ("Bitcoin", "bitcoin")
    };
    if liquid {
        if list(g(&raw, "vin")).is_some() && list(g(&raw, "vout")).is_some() {
            return Ok(raw);
        }
    } else if !can_lookup_public_bitcoin(row, &raw) {
        return Ok(raw);
    }
    let txid = string_or_none(g(&raw, "txid")).or_else(|| txid_from_row(row));
    let Some(txid) = txid.filter(|t| looks_like_txid(&s(t.as_str()))) else {
        return Ok(raw);
    };
    let (chain, network) = if liquid {
        row_chain_network(row, "liquid", "liquidv1")
    } else {
        default_row_chain_network(row)
    };
    if chain == "unknown" {
        return Ok(with_lookup_warning(
            &raw,
            "invalid_reference_scope",
            "Conflicting or unsupported network metadata prevents transaction reference lookup.",
        ));
    }
    let binding = kassiber_setup::networks::resolve(conn, &text_or(g(row, "profile_id")))?;
    if !kassiber_setup::networks::observation_matches_binding(&binding, row, false) {
        return Ok(with_lookup_warning(
            &raw,
            "book_network_mismatch",
            "This observation does not match the active book network; no reference lookup was performed.",
        ));
    }
    let txid = lower(&txid);
    let mut raw = raw;
    if ["bitcoin", "liquid"].contains(&chain.as_str()) {
        match kassiber_wallets::freshness::graph::local_reference(
            conn,
            profile_id,
            &chain,
            &network,
            &txid,
            &serde(&raw),
        )? {
            kassiber_wallets::freshness::graph::LocalReference::Conflict => {
                return Ok(with_lookup_warning(
                    &raw,
                    "local_reference_conflict",
                    "Local transaction references disagree or exceed the inspection limit; the stored transaction is shown unchanged.",
                ))
            }
            kassiber_wallets::freshness::graph::LocalReference::Payload(payload) => {
                raw = graph_wire(&payload)?;
                set(
                    &mut raw,
                    "_graphLookupWarning",
                    dict([
                        ("code", s("local_transaction_reference")),
                        ("level", s("info")),
                        (
                            "message",
                            s("Uses transaction references already stored in this book. References do not establish wallet ownership or booked custody."),
                        ),
                    ]),
                );
                if liquid || has_required_prevouts(&raw) {
                    return Ok(raw);
                }
            }
            kassiber_wallets::freshness::graph::LocalReference::Absent => {}
        }
    }
    let cached = if looks_like_txid(&s(txid.as_str())) {
        kassiber_wallets::freshness::graph::cached_reference(conn, &chain, &network, &txid)?
            .map(|v| graph_wire(&v))
            .transpose()?
    } else {
        None
    };
    if let Some(mut cached) = cached.filter(|c| liquid || has_required_prevouts(c)) {
        set(
            &mut cached,
            "_graphLookupWarning",
            dict([
                ("code", s("cached_reference_graph")),
                ("level", s("info")),
                (
                    "message",
                    s("Uses previously fetched public transaction references. These references do not establish wallet ownership or booked custody."),
                ),
            ]),
        );
        return Ok(cached);
    }
    if !allow_public_lookup {
        return Ok(raw);
    }
    // Public lookups are network work behind kassiber-net; this read refuses
    // instead of falling back.
    let _ = (label, prefix);
    Err(AppError::new(
        "not_implemented",
        "Public transaction reference lookup is not implemented in the Rust daemon yet",
    )
    .with_details(dict([("kind", s("ui.transactions.graph"))])))
}

/// `_observed_liquid_leg_sats`: legs this book's own Liquid observations
/// recorded, and the legs they disagree on.
fn observed_liquid_legs(
    conn: &Connection,
    profile_id: &str,
    txid: &str,
    wanted: &(String, String),
    policy: Option<&str>,
    axis: &str,
) -> Result<(BTreeMap<String, i128>, BTreeSet<String>)> {
    let txid = lower(txid);
    let mut parsed = Vec::new();
    for sibling in rows(
        conn,
        "SELECT raw_json FROM transactions WHERE profile_id = ? AND external_id IN (?, ?)",
        [profile_id, &txid, &txid.to_uppercase()],
    )? {
        let observation =
            kassiber_custody::onchain::parse_ownership_tx(&serde(g(&sibling, "raw_json")))
                .map_err(|e| AppError::new("internal_error", e))?;
        let Some(observation) = observation else {
            continue;
        };
        if lower(&text_or(g(&observation, "txid"))) != txid {
            continue;
        }
        if norm_chain_network(g(&observation, "chain"), g(&observation, "network")) != *wanted {
            continue;
        }
        parsed.push(observation);
    }
    let Some(merged) = kassiber_custody::onchain::merge_ownership_txs(&parsed) else {
        return Ok((BTreeMap::new(), BTreeSet::new()));
    };
    let conflicted_labels: BTreeSet<String> = list(g(&merged, "evidence_conflicts"))
        .map(|items| {
            items
                .iter()
                .map(|label| {
                    let label = py_str(label);
                    label
                        .rsplit_once('.')
                        .map_or(label.clone(), |(head, _)| head.to_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    let mut legs = Vec::new();
    for leg in list(g(&merged, "inputs")).into_iter().flatten() {
        legs.push((
            format!("input:{}", text(g(leg, "outpoint"))),
            lower(&text_or(g(leg, "outpoint"))),
            leg,
        ));
    }
    for leg in list(g(&merged, "outputs")).into_iter().flatten() {
        legs.push((
            format!("output:{}", text(g(leg, "n"))),
            format!("{txid}:{}", text(g(leg, "n"))),
            leg,
        ));
    }
    let mut amounts = BTreeMap::new();
    let mut conflicted = BTreeSet::new();
    for (label, outpoint, leg) in legs {
        let value = int_or_none(g(leg, "value_sats"));
        let Some((prev, vout)) = outpoint.rsplit_once(':') else {
            continue;
        };
        if !(looks_like_txid(&s(prev))
            && !vout.is_empty()
            && vout
                .chars()
                .all(|c| kassiber_pytext::decimal_value(c as u32).is_some()))
        {
            continue;
        }
        if conflicted_labels.contains(&label) {
            conflicted.insert(outpoint);
            continue;
        }
        let Some(value) = value.filter(|v| *v >= 0) else {
            continue;
        };
        if !same_asset(leg_asset(leg, policy).as_deref(), Some(axis)) {
            continue;
        }
        amounts.insert(outpoint, value);
    }
    Ok((amounts, conflicted))
}

/// `_local_wallet_outpoint_amounts`.
fn local_outpoint_amounts(
    conn: &Connection,
    profile_id: &str,
    row: &Value,
    raw: &Value,
) -> Result<BTreeMap<String, i128>> {
    if !is_dict(raw) {
        return Ok(BTreeMap::new());
    }
    let mut outpoints = BTreeSet::new();
    if let Some(vin) = list(g(raw, "vin")) {
        for entry in vin.iter().filter(|e| is_dict(e)) {
            if let Some(op) = outpoint(entry).filter(|o| !o.is_empty()) {
                outpoints.insert(lower(&op));
            }
        }
    }
    let txid = string_or_none(g(raw, "txid")).or_else(|| txid_from_row(row));
    if let (Some(txid), Some(vout)) = (&txid, list(g(raw, "vout"))) {
        for (index, entry) in vout.iter().enumerate() {
            if !is_dict(entry) {
                continue;
            }
            let n = int_or_none(g(entry, "n")).unwrap_or(index as i128);
            outpoints.insert(format!("{}:{n}", lower(txid)));
        }
    }
    if outpoints.is_empty() {
        return Ok(BTreeMap::new());
    }
    let confidential = looks_liquid_or_confidential(row, raw);
    let wanted = if confidential {
        row_chain_network(row, "liquid", "liquidv1")
    } else {
        default_row_chain_network(row)
    };
    let liquid_axis = axis_is_liquid(row, confidential);
    let policy = if liquid_axis {
        policy_asset_id(row)
    } else {
        None
    };
    let axis = if liquid_axis { "LBTC" } else { "BTC" };
    let list_ids: Vec<String> = outpoints.iter().cloned().collect();
    let mut bound = vec![Sql::Text(profile_id.into())];
    bound.extend(list_ids.iter().cloned().map(Sql::Text));
    let found = rows(
        conn,
        &format!(
            "SELECT lower(outpoint) AS outpoint, amount, chain, network, UPPER(asset) AS asset FROM wallet_utxos WHERE profile_id = ? AND lower(outpoint) IN ({})",
            vec!["?"; list_ids.len()].join(", ")
        ),
        params_from_iter(bound),
    )?;
    let mut amounts = BTreeMap::new();
    for item in &found {
        let asset = leg_asset(&dict([("asset", val(item, "asset"))]), policy.as_deref());
        if !same_asset(asset.as_deref(), Some(axis)) {
            continue;
        }
        if norm_chain_network(g(item, "chain"), g(item, "network")) != wanted {
            continue;
        }
        let point = string_or_none(g(item, "outpoint"));
        let amount = int_or_none(g(item, "amount"));
        if let (Some(point), Some(amount)) = (point, amount) {
            if amount >= 0 {
                amounts.insert(lower(&point), amount.div_euclid(1000));
            }
        }
    }
    if liquid_axis {
        if let Some(txid) = &txid {
            let (observed, conflicted) =
                observed_liquid_legs(conn, profile_id, txid, &wanted, policy.as_deref(), axis)?;
            for (point, value) in observed {
                amounts.entry(point).or_insert(value);
            }
            for point in conflicted {
                amounts.remove(&point);
            }
        }
    }
    Ok(amounts)
}

struct Graph {
    support: &'static str,
    reason: Option<&'static str>,
    metadata: Value,
    inputs: Vec<Value>,
    outputs: Vec<Value>,
    fee: Value,
    warnings: Vec<Value>,
}

fn graphless_message(reason: &str) -> &'static str {
    if reason == "liquid_reference_graph_not_local" {
        "Kassiber has only the imported Liquid row locally. Add a Liquid explorer backend to inspect public input/output references; confidential amounts may remain hidden."
    } else {
        "This source record does not contain Bitcoin input/output references for a flow diagram."
    }
}

fn partial_message(reason: &str) -> &'static str {
    if reason == "input_prevout_values_missing" {
        "Bitcoin outputs in this transaction have values. Some input values are not stored locally because an input amount comes from the spent previous output, not from the transaction input itself. Kassiber can still draw the references and outputs; fee and fee rate need those prevout amounts."
    } else {
        "Amounts are missing for one or more inputs/outputs in the source data. Kassiber can still show references and ownership hints, but fee and fee rate may be incomplete."
    }
}

/// `_apply_foreign_asset`.
fn apply_foreign_asset(node: &mut Value, asset: Option<String>, axis: &str) -> bool {
    if same_asset(asset.as_deref(), Some(axis)) {
        return false;
    }
    set(node, "asset", short_asset(asset.as_deref()));
    // `asset` may be a shortened label for a 64-hex Liquid asset id; the full
    // id travels separately so nothing passes the label on as an id.
    set(node, "assetId", asset.map_or(Value::Null, s));
    set(node, "valueSats", Value::Null);
    set(node, "valueBtc", Value::Null);
    set(node, "valueState", s("other_asset"));
    true
}

fn value_state(hidden: bool, value: Option<i128>) -> Value {
    s(if hidden {
        "confidential"
    } else if value.is_none() {
        "missing"
    } else {
        "known"
    })
}

fn node_sats(node: &Value) -> Option<i128> {
    match g(node, "valueSats") {
        Value::Int(n) => i128::try_from(n.clone()).ok(),
        _ => None,
    }
}

/// `_fee_from_graph`.
fn fee_from_graph(
    inputs: &[Value],
    outputs: &[Value],
    metadata: &Value,
    explicit: Option<i128>,
) -> Value {
    let mut value = explicit.filter(|v| *v >= 0);
    if value.is_none() {
        let ins: Vec<_> = inputs.iter().map(node_sats).collect();
        let outs: Vec<_> = outputs.iter().map(node_sats).collect();
        if !ins.is_empty() && ins.iter().all(Option::is_some) && outs.iter().all(Option::is_some) {
            let computed = ins.iter().flatten().sum::<i128>() - outs.iter().flatten().sum::<i128>();
            if computed >= 0 {
                value = Some(computed);
            }
        }
    }
    let Some(value) = value else {
        return Value::Null;
    };
    let mut fee = dict([
        ("id", s("fee")),
        ("label", s("Miner fee")),
        ("role", s("fee")),
        ("ownership", s("network_fee")),
        ("valueSats", int_value(value)),
        ("valueBtc", sats_to_btc(Some(value))),
        (
            "annotations",
            Value::List(vec![annotation("miner_fee", "Network fee", None)]),
        ),
    ]);
    let vsize = g(metadata, "vsize");
    if truthy(vsize) {
        let size = match vsize {
            Value::Int(n) => i128::try_from(n.clone()).unwrap_or(0) as f64,
            Value::Float(f) => *f,
            _ => 0.0,
        };
        set(
            &mut fee,
            "rateSatVb",
            Value::Float(round2(value as f64 / size)),
        );
    }
    fee
}

/// `_parse_graph`.
fn parse_graph(row: &Value, raw: &Value, local: &BTreeMap<String, i128>) -> Result<Graph> {
    let mut warnings = lookup_warnings(raw);
    let confidential = looks_liquid_or_confidential(row, raw);
    let liquid_axis = axis_is_liquid(row, confidential);
    let policy = if liquid_axis {
        policy_asset_id(row)
    } else {
        None
    };
    let axis = if liquid_axis { "LBTC" } else { "BTC" };
    let metadata = transaction_metadata(raw, confidential);
    let (Some(vin), Some(vout)) = (list(g(raw, "vin")), list(g(raw, "vout"))) else {
        let reason = if confidential {
            "liquid_reference_graph_not_local"
        } else {
            "graphless_import"
        };
        let mut graphless = vec![dict([
            ("code", s(reason)),
            ("level", s("info")),
            ("message", s(graphless_message(reason))),
        ])];
        let lookup = g(raw, "_graphLookupWarning");
        if is_dict(lookup) {
            let field = |key: &str, default: &str| {
                if truthy(g(lookup, key)) {
                    py_str(g(lookup, key))
                } else {
                    default.to_owned()
                }
            };
            graphless.push(dict([
                ("code", s(field("code", "graph_lookup_failed"))),
                ("level", s(field("level", "warning"))),
                (
                    "message",
                    s(field(
                        "message",
                        "Public transaction reference lookup failed.",
                    )),
                ),
            ]));
        }
        return Ok(Graph {
            support: "graphless",
            reason: Some(reason),
            metadata,
            inputs: Vec::new(),
            outputs: Vec::new(),
            fee: Value::Null,
            warnings: graphless,
        });
    };
    let parsed = kassiber_custody::onchain::parse_ownership_tx(&serde(raw))
        .map_err(|e| AppError::new("internal_error", e))?;
    let valued = parsed.is_some_and(|p| {
        list(g(&p, "outputs"))
            .is_some_and(|outs| outs.iter().all(|o| *g(o, "value_sats") != Value::Null))
    });
    let empty = dict::<&str>([]);
    let mut inputs = Vec::new();
    let mut inputs_complete = true;
    for (index, entry) in vin.iter().enumerate() {
        if !is_dict(entry) {
            continue;
        }
        let prevout = if is_dict(g(entry, "prevout")) {
            g(entry, "prevout")
        } else {
            &empty
        };
        let point = outpoint(entry);
        let local_value = point
            .as_deref()
            .filter(|p| !p.is_empty())
            .and_then(|p| local.get(&lower(p)).copied())
            .filter(|v| *v >= 0);
        let hidden = local_value.is_none() && (confidential || confidential_leg(prevout));
        let value = if local_value.is_some() {
            local_value
        } else if hidden {
            None
        } else {
            output_value_sats(prevout)
        };
        if value.is_none() {
            inputs_complete = false;
        }
        let script = string_or_none(&output_script(prevout));
        let mut node = dict([
            ("id", s(format!("in-{index}"))),
            ("index", int_value(index as i128)),
            ("outpoint", point.clone().map_or(Value::Null, s)),
            (
                "txid",
                if truthy(g(entry, "txid")) {
                    s(py_str(g(entry, "txid")))
                } else {
                    Value::Null
                },
            ),
            ("vout", opt_int(int_or_none(g(entry, "vout")))),
            ("address", output_address(prevout).map_or(Value::Null, s)),
            ("scriptType", s(script_type(prevout, script.as_deref()))),
            ("valueSats", opt_int(value)),
            ("valueBtc", sats_to_btc(value)),
            ("valueState", value_state(hidden, value)),
            (
                "label",
                s(point
                    .clone()
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| format!("Input {}", index + 1))),
            ),
            ("ownership", s("unknown")),
            ("role", s("input")),
            ("annotations", Value::List(vec![])),
            ("_script", script.map_or(Value::Null, s)),
        ]);
        if is_pegin_leg(entry) {
            set(&mut node, "_pegin", Value::Bool(true));
        }
        if is_coinbase_leg(entry) {
            set(&mut node, "_coinbase", Value::Bool(true));
        }
        if apply_foreign_asset(&mut node, leg_asset(prevout, policy.as_deref()), axis) {
            inputs_complete = false;
        }
        inputs.push(node);
    }
    let mut outputs = Vec::new();
    let mut outputs_complete = true;
    let mut liquid_fee: Option<i128> = None;
    for (index, entry) in vout.iter().enumerate() {
        if !is_dict(entry) {
            continue;
        }
        if confidential && is_liquid_fee_output(entry) {
            if let Some(fee) = output_value_sats(entry).filter(|v| *v >= 0) {
                liquid_fee = Some(liquid_fee.unwrap_or(0) + fee);
            }
            continue;
        }
        let script = string_or_none(&output_script(entry));
        let n = int_or_none(g(entry, "n")).unwrap_or(index as i128);
        let tx = [g(raw, "txid"), g(row, "external_id")]
            .into_iter()
            .find(|v| truthy(v))
            .map(py_str)
            .unwrap_or_default();
        let point = format!("{}:{n}", lower(&tx));
        let local_value = local.get(&lower(&point)).copied().filter(|v| *v >= 0);
        let hidden = local_value.is_none() && (confidential || confidential_leg(entry));
        let value = if local_value.is_some() {
            local_value
        } else if hidden {
            None
        } else {
            output_value_sats(entry)
        };
        if value.is_none() {
            outputs_complete = false;
        }
        let pegout = pegout_address(entry);
        let mut node = dict([
            ("id", s(format!("out-{n}"))),
            ("index", int_value(n)),
            ("outpoint", s(point)),
            (
                "address",
                output_address(entry)
                    .or(pegout.clone())
                    .map_or(Value::Null, s),
            ),
            ("scriptType", s(script_type(entry, script.as_deref()))),
            ("valueSats", opt_int(value)),
            ("valueBtc", sats_to_btc(value)),
            ("valueState", value_state(hidden, value)),
            ("label", s(format!("Output {n}"))),
            ("ownership", s("unknown")),
            ("role", s("output")),
            ("annotations", Value::List(vec![])),
            ("_script", script.map_or(Value::Null, s)),
            ("_pegoutAddress", pegout.map_or(Value::Null, s)),
        ]);
        if apply_foreign_asset(&mut node, leg_asset(entry, policy.as_deref()), axis) {
            outputs_complete = false;
        }
        outputs.push(node);
    }
    let (support, reason) = if inputs_complete && outputs_complete && (valued || confidential) {
        ("full", None)
    } else {
        let reason = if confidential {
            "confidential_values_hidden"
        } else if !inputs_complete && outputs_complete {
            "input_prevout_values_missing"
        } else {
            "partial_graph_values_missing"
        };
        warnings.push(dict([
            ("code", s(reason)),
            ("level", s("info")),
            (
                "message",
                s(if confidential {
                    "Amounts are confidential on at least one Liquid input/output. Kassiber can show public references and ownership hints, but not value-sized dots or fee rate."
                } else {
                    partial_message(reason)
                }),
            ),
        ]));
        ("partial", Some(reason))
    };
    let mut metadata = metadata;
    set(&mut metadata, "inputCount", inputs.len().into());
    set(&mut metadata, "outputCount", outputs.len().into());
    let fee = fee_from_graph(&inputs, &outputs, &metadata, liquid_fee);
    Ok(Graph {
        support,
        reason,
        metadata,
        inputs,
        outputs,
        fee,
        warnings,
    })
}

// --------------------------------------------------------- ownership

/// One owned-index match.
struct Owned {
    wallet_id: String,
    wallet_label: String,
    branch_label: String,
}

fn owned_matches(index: &Value, node: &Value, wanted: &(String, String)) -> Vec<Owned> {
    let keep = |items: &[Value]| -> Vec<Value> {
        items
            .iter()
            .filter(|m| norm_chain_network(g(m, "chain"), g(m, "network")) == *wanted)
            .cloned()
            .collect()
    };
    let mut matches = Vec::new();
    if let Some(point) = string_or_none(g(node, "outpoint")).or_else(|| match g(node, "outpoint") {
        Value::Str(t) if t.as_str() == Some("") => None,
        _ => None,
    }) {
        let mut found = list(g(g(index, "by_outpoint"), &lower(&point)))
            .cloned()
            .unwrap_or_default();
        found.sort_by_key(|m| {
            let canonical = ["descriptor", "xpub", "silent-payment"]
                .contains(&text(g(m, "wallet_kind")).as_str());
            let priority = match text(g(m, "source")).as_str() {
                "derived" => 0,
                "inventory" => 1,
                "address_list" => 2,
                "derived_history" => 3,
                "address_history" => 4,
                _ => 9,
            };
            (
                !canonical,
                priority,
                text(g(m, "wallet_label")),
                text(g(m, "wallet_id")),
            )
        });
        matches = keep(&found);
    }
    if matches.is_empty() {
        let script = g(node, "_script");
        if truthy(script) {
            let found = list(g(g(index, "by_script"), &lower(&py_str(script))))
                .cloned()
                .unwrap_or_default();
            matches = keep(&found);
        }
    }
    matches
        .iter()
        .map(|m| Owned {
            wallet_id: text(g(m, "wallet_id")),
            wallet_label: text(g(m, "wallet_label")),
            branch_label: text(g(m, "branch_label")),
        })
        .collect()
}

fn apply_match(node: &mut Value, matches: &[Owned], owned_code: &str) {
    if matches.is_empty() {
        set(node, "ownership", s("unknown"));
        return;
    }
    let wallets: BTreeSet<&str> = matches.iter().map(|m| m.wallet_label.as_str()).collect();
    let ids: BTreeSet<&str> = matches.iter().map(|m| m.wallet_id.as_str()).collect();
    let ambiguous = ids.len() > 1;
    set(
        node,
        "ownership",
        s(if ambiguous { "ambiguous" } else { "owned" }),
    );
    set(
        node,
        "wallet",
        s(wallets.iter().copied().collect::<Vec<_>>().join(", ")),
    );
    set(
        node,
        "walletId",
        if ids.len() == 1 {
            s(*ids.iter().next().unwrap())
        } else {
            Value::Null
        },
    );
    push_annotation(
        node,
        annotation(
            if ambiguous {
                "ambiguous_ownership"
            } else {
                owned_code
            },
            if ambiguous {
                "Owned by multiple wallets"
            } else {
                "Owned wallet"
            },
            None,
        ),
    );
}

fn push_annotation(node: &mut Value, item: Value) {
    let mut annotations = list(g(node, "annotations")).cloned().unwrap_or_default();
    annotations.push(item);
    set(node, "annotations", Value::List(annotations));
}

fn row_is_inbound(row: &Value) -> bool {
    ["inbound", "receive", "received", "deposit", "income", "buy"]
        .contains(&lower(&text_or(g(row, "direction"))).as_str())
}

/// `branch_evidence(label, None)` reduced to the change predicate.
fn exact_change(label: &str) -> bool {
    let normalized: String = lower(&strip(label))
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let mut tokens: BTreeSet<String> = normalized.split_whitespace().map(str::to_owned).collect();
    tokens.insert(normalized.replace(' ', ""));
    tokens.contains("change") || tokens.contains("internal")
}

fn privacy_boundary(value: &Value) -> Option<String> {
    let text = strip(&text_or(value));
    if text.is_empty() {
        return None;
    }
    let normalized = lower(&text).replace('-', "_");
    [
        "coinjoin",
        "collaborative",
        "payjoin",
        "payment_in_coinjoin",
        "sweep",
    ]
    .contains(&normalized.as_str())
    .then_some(normalized)
}

fn boolish(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Str(t) if t.as_str() == Some("") => false,
        other => ["1", "true", "yes", "on"].contains(&lower(&strip(&py_str(other))).as_str()),
    }
}

const BOUNDARY_KEYS: &[&str] = &[
    "privacy_boundary",
    "privacy_boundary_kind",
    "privacyBoundary",
    "privacyBoundaryKind",
    "privacy_hop",
    "privacy_hop_kind",
    "privacyHop",
    "privacyHopKind",
];
const COINJOIN_KEYS: &[&str] = &[
    "islikelycoinjoin",
    "is_likely_coinjoin",
    "isLikelyCoinJoin",
    "likely_coinjoin",
    "likelyCoinJoin",
];

/// `privacy_boundary_from_import_record` for stored data: invalid markers
/// read as `unknown`.
fn stored_boundary(record: &Value) -> Option<String> {
    let scan = |payload: &Value| -> Option<Option<String>> {
        for key in BOUNDARY_KEYS {
            let Some(value) = get(payload, key) else {
                continue;
            };
            if strip(&text_or(value)).is_empty() {
                continue;
            }
            return Some(Some(
                privacy_boundary(value).unwrap_or_else(|| "unknown".into()),
            ));
        }
        if COINJOIN_KEYS.iter().any(|key| boolish(g(payload, key))) {
            return Some(Some("coinjoin".into()));
        }
        None
    };
    if let Some(found) = scan(record) {
        return found;
    }
    let raw = g(record, "raw_json");
    let payload = match raw {
        Value::Dict(_) => raw.clone(),
        _ if !truthy(raw) => return None,
        Value::Str(t) => match kassiber_pytext::loads(t.as_str().unwrap_or_default()) {
            Ok(v @ Value::Dict(_)) => v,
            _ => return None,
        },
        _ => return None,
    };
    scan(&payload).flatten()
}

/// `collaborative_transaction_evidence(row, raw) is not None`.
fn collaborative(row: &Value, raw: &Value) -> bool {
    let marker = g(row, "privacy_boundary");
    let mut boundary = privacy_boundary(marker);
    if boundary.is_none() && *marker != Value::Null && *marker != s("") {
        boundary = Some("unknown".into());
    }
    if boundary.is_none() {
        boundary = stored_boundary(raw);
    }
    if let Some(boundary) = &boundary {
        if boundary == "unknown"
            || [
                "coinjoin",
                "payjoin",
                "collaborative",
                "payment_in_coinjoin",
            ]
            .contains(&boundary.as_str())
        {
            return true;
        }
    }
    let tx = kassiber_custody::confirmation::stored_mapping(&serde(raw), true)
        .and_then(|v| kassiber_custody::codec::from_json(&v).ok())
        .unwrap_or_else(|| dict::<&str>([]));
    let (Some(vin), Some(vout)) = (list(g(&tx, "vin")), list(g(&tx, "vout"))) else {
        return false;
    };
    let mut counts: HashMap<i128, usize> = HashMap::new();
    let mut outputs = 0;
    for item in vout.iter().filter(|v| is_dict(v)) {
        let script = string_or_none(&output_script(item))
            .map(|s| lower(&s))
            .filter(|s| {
                s.len() % 2 == 0 && from_hex(s).is_some() && !s.contains(char::is_whitespace)
            });
        let declared = lower(&text_or(
            [g(item, "scriptpubkey_type"), g(item, "type")]
                .into_iter()
                .find(|v| truthy(v))
                .unwrap_or(&Value::Null),
        ));
        let op_return =
            declared == "op_return" || script.as_deref().is_some_and(|s| s.starts_with("6a"));
        if op_return {
            continue;
        }
        outputs += 1;
        if let Some(value) = output_value_sats(item).filter(|v| *v > 0) {
            *counts.entry(value).or_default() += 1;
        }
    }
    let common = counts.values().copied().max().unwrap_or(0);
    (vin.len() >= 5 && (5..=12).contains(&outputs) && common >= 5)
        || (vin.len() >= 20 && outputs >= 20 && common >= 5)
}

/// `_annotate_graph`.
fn annotate(graph: &mut Graph, row: &Value, index: &Value, by_row: &[Value], raw: &Value) {
    let source_wallet = text_or(g(row, "wallet_id"));
    let (c, n) = default_row_chain_network(row);
    let wanted = norm_chain_network(&s(c), &s(n));
    let mut input_owners = BTreeSet::new();
    for node in &mut graph.inputs {
        let matches = owned_matches(index, node, &wanted);
        apply_match(node, &matches, "owned_input");
        input_owners.extend(matches.iter().map(|m| m.wallet_id.clone()));
        if truthy(g(node, "_coinbase")) {
            set(node, "ownership", s("coinbase"));
            set(node, "role", s("coinbase"));
            push_annotation(
                node,
                annotation("coinbase", "Newly issued (coinbase)", None),
            );
        } else if truthy(g(node, "_pegin")) {
            set(node, "role", s("peg_in"));
            push_annotation(node, annotation("peg_in", "Peg-in from Bitcoin", None));
        }
    }
    let collaboration =
        collaborative(row, &json_obj(g(row, "raw_json"))) || collaborative(row, raw);
    let ordinary = !graph.inputs.is_empty()
        && !collaboration
        && graph
            .inputs
            .iter()
            .all(|n| text(g(n, "ownership")) == "owned")
        && input_owners.len() == 1;
    let incoming = inferred_incoming(row, &graph.outputs);
    for node in &mut graph.outputs {
        let script = string_or_none(g(node, "_script"));
        if truthy(g(node, "_pegoutAddress")) {
            set(node, "ownership", s("peg_out"));
            set(node, "role", s("peg_out"));
            push_annotation(node, annotation("peg_out", "Peg-out to Bitcoin", None));
            continue;
        }
        if is_unspendable(script.as_deref()) {
            set(node, "ownership", s("unspendable"));
            set(node, "role", s("op_return"));
            push_annotation(
                node,
                annotation("op_return", "OP_RETURN / non-address output", None),
            );
            continue;
        }
        let matches = owned_matches(index, node, &wanted);
        let owners: BTreeSet<String> = matches.iter().map(|m| m.wallet_id.clone()).collect();
        if matches.is_empty() {
            if incoming.as_ref() == Some(g(node, "id")) {
                set(node, "ownership", s("unknown"));
                set(node, "role", s("incoming_payment_candidate"));
                push_annotation(
                    node,
                    annotation(
                        "recorded_incoming_amount",
                        "Matches imported incoming amount; output ownership is unverified",
                        None,
                    ),
                );
                continue;
            }
            set(node, "ownership", s("unknown"));
            set(node, "role", s("output"));
            continue;
        }
        apply_match(node, &matches, "owned_output");
        if owners.len() > 1 {
            set(node, "role", s("ambiguous_owned_output"));
            push_annotation(
                node,
                annotation("ambiguous_owned_output", "Owned by multiple wallets", None),
            );
        } else if owners.iter().any(|o| input_owners.contains(o)) {
            let change = ordinary && matches.iter().all(|m| exact_change(&m.branch_label));
            let role = if change { "change" } else { "owned_return" };
            set(node, "role", s(role));
            push_annotation(
                node,
                annotation(
                    role,
                    if change {
                        "Change to an observed wallet change branch"
                    } else {
                        "Output to a wallet that owns an observed input"
                    },
                    None,
                ),
            );
        } else if row_is_inbound(row) && owners.contains(&source_wallet) {
            set(node, "role", s("incoming_payment"));
            push_annotation(
                node,
                annotation("incoming_payment", "Incoming payment to this wallet", None),
            );
        } else {
            set(node, "role", s("owned_destination"));
            push_annotation(
                node,
                annotation("owned_destination", "Owned destination wallet", None),
            );
        }
    }
    let mut groups: Vec<Value> = Vec::new();
    for item in by_row {
        let group = g(item, "groupId");
        if truthy(group) && !groups.contains(group) {
            groups.push(group.clone());
        }
    }
    // Python iterates a set of strings; order them as CPython's string hash
    // would not be reproducible, so a single group keeps parity and several
    // groups are sorted.
    groups.sort_by_key(py_str);
    for node in &mut graph.outputs {
        if text(g(node, "role")) == "owned_destination" {
            for group in &groups {
                push_annotation(
                    node,
                    annotation(
                        "linked_transfer_group",
                        "Linked transfer group",
                        Some(group.clone()),
                    ),
                );
            }
        }
    }
}

fn inferred_incoming(row: &Value, outputs: &[Value]) -> Option<Value> {
    if !row_is_inbound(row) {
        return None;
    }
    let amount = int_or_none(g(row, "amount")).filter(|a| *a > 0 && a % 1000 == 0)?;
    let sats = amount / 1000;
    let candidates: Vec<&Value> = outputs
        .iter()
        .filter(|n| *g(n, "id") != Value::Null && node_sats(n) == Some(sats))
        .map(|n| g(n, "id"))
        .collect();
    (candidates.len() == 1).then(|| candidates[0].clone())
}

// ------------------------------------------------------- local links

/// `_profile_outpoint_spends`: domain-qualified outpoints to local spends.
fn profile_spends(
    conn: &Connection,
    profile_id: &str,
) -> Result<HashMap<(String, String, String), Value>> {
    type Spenders = Vec<((String, String, String), Vec<(String, Value)>)>;
    let mut candidates: Spenders = Vec::new();
    let mut positions: HashMap<(String, String, String), usize> = HashMap::new();
    fn add(
        candidates: &mut Spenders,
        positions: &mut HashMap<(String, String, String), usize>,
        key: (String, String, String),
        txid: String,
        value: Value,
    ) {
        let index = *positions.entry(key.clone()).or_insert_with(|| {
            candidates.push((key, Vec::new()));
            candidates.len() - 1
        });
        let spenders = &mut candidates[index].1;
        if !spenders.iter().any(|(id, _)| *id == txid) {
            spenders.push((txid, value));
        }
    }
    for row in rows(
        conn,
        "SELECT t.id, t.external_id, t.raw_json, t.asset, w.kind AS wallet_kind, w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? ORDER BY t.occurred_at, t.created_at, t.id",
        [profile_id],
    )? {
        let graph = json_obj(g(&row, "raw_json"));
        let Some(vin) = list(g(&graph, "vin")) else { continue };
        let spending = string_or_none(g(&graph, "txid")).or_else(|| string_or_none(g(&row, "external_id")));
        let Some(spending) = spending.filter(|t| looks_like_txid(&s(t.as_str()))) else {
            continue;
        };
        let (chain, network) = default_row_chain_network(&row);
        let spending = lower(&spending);
        for entry in vin.iter().filter(|e| is_dict(e)) {
            if let Some(point) = outpoint(entry).filter(|p| !p.is_empty()) {
                add(
                    &mut candidates,
                    &mut positions,
                    (chain.clone(), network.clone(), lower(&point)),
                    spending.clone(),
                    dict([
                        ("txid", s(spending.as_str())),
                        ("transactionId", s(text(g(&row, "id")))),
                    ]),
                );
            }
        }
    }
    let local: BTreeSet<(String, String, String)> = positions.keys().cloned().collect();
    for row in rows(
        conn,
        "SELECT lower(outpoint) AS outpoint, spent_by, chain, network FROM wallet_utxos WHERE profile_id = ? AND spent_by IS NOT NULL AND spent_by != '' ORDER BY outpoint",
        [profile_id],
    )? {
        let point = string_or_none(g(&row, "outpoint"));
        let spent = string_or_none(g(&row, "spent_by"));
        let (Some(point), Some(spent)) = (point, spent) else { continue };
        if !looks_like_txid(&s(spent.as_str())) {
            continue;
        }
        let (chain, network) = norm_chain_network(g(&row, "chain"), g(&row, "network"));
        let key = (chain, network, lower(&point));
        if !local.contains(&key) {
            let spent = lower(&spent);
            add(&mut candidates, &mut positions, key, spent.clone(), dict([("txid", s(spent))]));
        }
    }
    Ok(candidates
        .into_iter()
        .map(|(key, spenders)| {
            let value = if spenders.len() == 1 {
                spenders.into_iter().next().unwrap().1
            } else {
                dict([("conflicting", Value::Bool(true))])
            };
            (key, value)
        })
        .collect())
}

fn annotate_spends(
    row: &Value,
    graph: &mut Graph,
    spends: &HashMap<(String, String, String), Value>,
) {
    if graph.outputs.is_empty() || spends.is_empty() {
        return;
    }
    let row_id = text_or(g(row, "id"));
    let (chain, network) = default_row_chain_network(row);
    for node in &mut graph.outputs {
        let Some(point) = string_or_none(g(node, "outpoint")) else {
            continue;
        };
        let Some(spend) = spends.get(&(chain.clone(), network.clone(), lower(&point))) else {
            continue;
        };
        if truthy(g(spend, "conflicting")) {
            graph.warnings.push(dict([
                ("code", s("conflicting_local_spends")),
                ("level", s("warning")),
                (
                    "message",
                    s("Local observations contain competing spends of an output. Refresh the connection to resolve which transaction is current."),
                ),
            ]));
            continue;
        }
        set(node, "spentByTxid", val(spend, "txid"));
        let transaction = g(spend, "transactionId");
        if truthy(transaction) && py_str(transaction) != row_id {
            set(node, "spentByTransactionId", s(py_str(transaction)));
        }
    }
}

fn annotate_funding(
    conn: &Connection,
    profile_id: &str,
    row: &Value,
    graph: &mut Graph,
) -> Result<()> {
    if graph.inputs.is_empty() {
        return Ok(());
    }
    let txids: BTreeSet<String> = graph
        .inputs
        .iter()
        .filter(|n| looks_like_txid(g(n, "txid")))
        .map(|n| lower(&py_str(g(n, "txid"))))
        .collect();
    if txids.is_empty() {
        return Ok(());
    }
    let row_id = text_or(g(row, "id"));
    let wanted = default_row_chain_network(row);
    let ids: Vec<String> = txids.into_iter().collect();
    let mut bound = vec![Sql::Text(profile_id.into())];
    bound.extend(ids.iter().cloned().map(Sql::Text));
    let mut funding: HashMap<String, String> = HashMap::new();
    for candidate in rows(
        conn,
        &format!("SELECT t.id, lower(t.external_id) AS txid, t.asset, t.raw_json, w.kind AS wallet_kind, w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND lower(t.external_id) IN ({}) ORDER BY t.occurred_at, t.created_at, t.id", vec!["?"; ids.len()].join(", ")),
        params_from_iter(bound),
    )? {
        let id = text(g(&candidate, "id"));
        if id == row_id || default_row_chain_network(&candidate) != wanted {
            continue;
        }
        funding.entry(text(g(&candidate, "txid"))).or_insert(id);
    }
    for node in &mut graph.inputs {
        let txid = lower(&text_or(g(node, "txid")));
        if let Some(id) = funding.get(&txid) {
            set(node, "fundedByTransactionId", s(id.as_str()));
        }
    }
    Ok(())
}

fn row_block_height(row: &Value, raw: Option<&Value>) -> Option<i128> {
    let stored = json_obj(g(row, "raw_json"));
    for candidate in raw.into_iter().chain([&stored]) {
        if !is_dict(candidate) {
            continue;
        }
        let status = g(candidate, "status");
        if is_dict(status) {
            let value = [g(status, "block_height"), g(status, "blockHeight")]
                .into_iter()
                .find(|v| truthy(v))
                .unwrap_or(g(status, "blockHeight"));
            if let Some(height) = int_or_none(value).filter(|h| *h > 0) {
                return Some(height);
            }
        }
        let value = [g(candidate, "block_height"), g(candidate, "blockHeight")]
            .into_iter()
            .find(|v| truthy(v))
            .unwrap_or(g(candidate, "blockHeight"));
        if let Some(height) = int_or_none(value).filter(|h| *h > 0) {
            return Some(height);
        }
    }
    None
}

fn annotate_heights(
    conn: &Connection,
    profile_id: &str,
    row: &Value,
    graph: &mut Graph,
) -> Result<()> {
    let mut wanted: Vec<String> = graph
        .inputs
        .iter()
        .filter_map(|n| string_or_none(g(n, "txid")))
        .collect();
    wanted.extend(
        graph
            .outputs
            .iter()
            .filter_map(|n| string_or_none(g(n, "spentByTxid"))),
    );
    if wanted.is_empty() {
        return Ok(());
    }
    let normalized: Vec<String> = wanted
        .iter()
        .map(|t| lower(&strip(t)))
        .filter(|t| !t.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if normalized.is_empty() {
        return Ok(());
    }
    let scope = default_row_chain_network(row);
    let placeholders = vec!["?"; normalized.len()].join(", ");
    let mut bound = vec![Sql::Text(profile_id.into())];
    bound.extend(normalized.iter().cloned().map(Sql::Text));
    let mut observed: BTreeMap<String, BTreeSet<i128>> = BTreeMap::new();
    for item in rows(
        conn,
        &format!("SELECT lower(t.external_id) AS txid, t.raw_json, t.asset, w.kind AS wallet_kind, w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND lower(t.external_id) IN ({placeholders})"),
        params_from_iter(bound.clone()),
    )? {
        if default_row_chain_network(&item) != scope {
            continue;
        }
        if let (Some(txid), Some(height)) = (string_or_none(g(&item, "txid")), row_block_height(&item, None)) {
            observed.entry(txid).or_default().insert(height);
        }
    }
    for item in rows(
        conn,
        &format!("SELECT lower(txid) AS txid, block_height, chain, network FROM wallet_utxos WHERE profile_id = ? AND lower(txid) IN ({placeholders}) AND block_height IS NOT NULL"),
        params_from_iter(bound),
    )? {
        if norm_chain_network(g(&item, "chain"), g(&item, "network")) != scope {
            continue;
        }
        if let (Some(txid), Some(height)) = (
            string_or_none(g(&item, "txid")),
            int_or_none(g(&item, "block_height")).filter(|h| *h > 0),
        ) {
            observed.entry(txid).or_default().insert(height);
        }
    }
    let heights: HashMap<String, i128> = observed
        .into_iter()
        .filter(|(_, h)| h.len() == 1)
        .map(|(t, h)| (t, *h.iter().next().unwrap()))
        .collect();
    if heights.is_empty() {
        return Ok(());
    }
    for node in &mut graph.inputs {
        if let Some(height) = string_or_none(g(node, "txid")).and_then(|t| heights.get(&lower(&t)))
        {
            set(node, "prevoutBlockHeight", int_value(*height));
        }
    }
    for node in &mut graph.outputs {
        if let Some(height) =
            string_or_none(g(node, "spentByTxid")).and_then(|t| heights.get(&lower(&t)))
        {
            set(node, "spentByBlockHeight", int_value(*height));
        }
    }
    Ok(())
}

// ------------------------------------------------ custody semantics

struct Semantics {
    current: bool,
    by_row: HashMap<String, Vec<Value>>,
}

/// `_stored_custody_semantics`.
fn semantics(conn: &Connection, profile: &Value) -> Result<Semantics> {
    let id = text(g(profile, "id"));
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
            [&id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let freshness = kassiber_workspace::readiness::evaluate_freshness(profile, active)?;
    let mut output = Semantics {
        current: truthy(g(&freshness, "is_current")),
        by_row: HashMap::new(),
    };
    if !output.current {
        return Ok(output);
    }
    let mut add = |transaction: String, item: &Value| {
        output
            .by_row
            .entry(transaction)
            .or_default()
            .push(without_none(item.clone()));
    };
    for row in rows(
        conn,
        "SELECT d.decision_id, d.source_transaction_id, d.target_transaction_id, d.source_wallet_id, source_wallet.label AS source_wallet_label, d.target_wallet_id, target_wallet.label AS target_wallet_label, d.source_end_msat - d.source_start_msat AS amount_msat, d.state, d.basis_state, d.reason, d.atomic_group_id, d.component_id FROM journal_custody_decisions d LEFT JOIN wallets source_wallet ON source_wallet.id = d.source_wallet_id LEFT JOIN wallets target_wallet ON target_wallet.id = d.target_wallet_id WHERE d.profile_id = ? ORDER BY d.occurred_at, d.decision_id",
        [&id],
    )? {
        let source = text(g(&row, "source_transaction_id"));
        let target = text(g(&row, "target_transaction_id"));
        let group = if truthy(g(&row, "atomic_group_id")) {
            py_str(g(&row, "atomic_group_id"))
        } else {
            text(g(&row, "decision_id"))
        };
        let amount = int_or_none(g(&row, "amount_msat"))
            .ok_or_else(|| AppError::new("internal_error", "int() argument must be a string, a bytes-like object or a real number, not 'NoneType'"))?;
        let item = dict([
            ("code", s("booked_custody_move")),
            ("label", s("Booked custody move")),
            ("severity", s("info")),
            ("groupId", s(format!("custody-decision:{group}"))),
            ("outTransactionId", s(source.as_str())),
            ("inTransactionId", s(target.as_str())),
            ("amountMsat", int_value(amount)),
            ("amountBtc", msat_to_btc(amount)),
            ("custodyState", val(&row, "state")),
            ("basisState", val(&row, "basis_state")),
            ("reason", val(&row, "reason")),
            ("componentId", val(&row, "component_id")),
            ("sourceWallet", val(&row, "source_wallet_label")),
            ("targetWallet", val(&row, "target_wallet_label")),
            ("booked", Value::Bool(true)),
        ]);
        add(source, &item);
        add(target, &item);
    }
    for row in rows(
        conn,
        "SELECT id AS relation_id, relation_kind, out_transaction_id AS source_transaction_id, in_transaction_id AS target_transaction_id, component_id, out_asset AS source_asset, in_asset AS target_asset, out_amount AS source_amount_msat, in_amount AS target_amount_msat, kind AS review_kind, policy, basis_state FROM journal_custody_projection_relations WHERE profile_id = ? AND relation_kind IN ('conversion', 'direct_payout') ORDER BY occurred_at, id",
        [&id],
    )? {
        let source = text(g(&row, "source_transaction_id"));
        let target = match g(&row, "target_transaction_id") {
            Value::Null => None,
            v if *v == s("") => None,
            v => Some(py_str(v)),
        };
        let kind = text(g(&row, "relation_kind"));
        let amount = int_or_none(g(&row, "source_amount_msat")).unwrap_or(0);
        let item = dict([
            ("code", s(format!("booked_custody_{kind}"))),
            (
                "label",
                s(if kind == "conversion" {
                    "Booked custody conversion"
                } else {
                    "Booked direct payout"
                }),
            ),
            ("severity", s("info")),
            ("groupId", s(format!("custody-relation:{}", text(g(&row, "relation_id"))))),
            ("outTransactionId", s(source.as_str())),
            ("inTransactionId", target.clone().map_or(Value::Null, s)),
            ("amountMsat", int_value(amount)),
            ("amountBtc", msat_to_btc(amount)),
            (
                "targetAmountMsat",
                int_value(int_or_none(g(&row, "target_amount_msat")).unwrap_or(0)),
            ),
            ("sourceAsset", val(&row, "source_asset")),
            ("targetAsset", val(&row, "target_asset")),
            ("basisState", val(&row, "basis_state")),
            ("policy", val(&row, "policy")),
            ("reviewKind", val(&row, "review_kind")),
            ("componentId", val(&row, "component_id")),
            ("booked", Value::Bool(true)),
        ]);
        add(source, &item);
        if let Some(target) = target {
            add(target, &item);
        }
    }
    Ok(output)
}

// ----------------------------------------------------- swap route

fn route_kind(pair: &Value) -> &'static str {
    let kind = lower(&strip(&text_or(g(pair, "pair_kind"))));
    if kind == "coinjoin" || kind.contains("coinjoin") || kind.contains("whirlpool") {
        return "coinjoin";
    }
    let out_asset = upper(&strip(&text_or(g(pair, "out_asset"))));
    let in_asset = upper(&strip(&text_or(g(pair, "in_asset"))));
    if (!out_asset.is_empty() && !in_asset.is_empty() && out_asset != in_asset)
        || ["peg-in", "peg-out", "submarine-swap", "swap-refund"].contains(&kind.as_str())
        || kind.contains("swap")
        || kind.starts_with("peg-")
    {
        return "swap";
    }
    if lower(&strip(&text_or(g(pair, "pair_policy")))) == "carrying-value" {
        return "transfer";
    }
    "pair"
}

fn network_label(asset: &Value, wallet_kind: &Value) -> Value {
    let kind = lower(&text_or(wallet_kind));
    let asset = upper(&text_or(asset));
    if kind.contains("liquid") || ["LBTC", "L-BTC", "LIQUID-BTC"].contains(&asset.as_str()) {
        return s("Liquid");
    }
    if asset == "BTC" {
        return s("Bitcoin");
    }
    if asset.is_empty() {
        Value::Null
    } else {
        s(asset)
    }
}

fn route_leg(pair: &Value, prefix: &str, amount: Option<i128>) -> Value {
    let field = |key: &str| val(pair, &format!("{prefix}_{key}"));
    let fee = int_or_none(&field("fee_msat"));
    let asset = string_or_none(&field("asset"));
    let external = string_or_none(&field("external_id"));
    let role = if prefix == "out" {
        let joined = [field("kind"), field("description")]
            .iter()
            .filter_map(string_or_none)
            .collect::<Vec<_>>()
            .join(" ");
        let raw = json_obj(&field("raw_json"));
        if lower(&joined).contains("consolidat")
            || list(g(&raw, "vin")).is_some_and(|v| v.len() > 1)
        {
            s("consolidation")
        } else {
            s("spend")
        }
    } else {
        s("receive")
    };
    without_none(dict([
        ("id", field("id")),
        ("externalId", external.clone().map_or(Value::Null, s)),
        (
            "txid",
            external.map_or(Value::Null, |e| {
                if e.chars().count() == 64 {
                    s(lower(&e))
                } else {
                    s(e)
                }
            }),
        ),
        ("direction", field("direction")),
        ("asset", asset.clone().map_or(Value::Null, s)),
        (
            "network",
            network_label(&asset.map_or(Value::Null, s), &field("wallet_kind")),
        ),
        ("amountMsat", opt_int(amount)),
        ("amountBtc", amount.map_or(Value::Null, msat_to_btc)),
        ("feeMsat", opt_int(fee)),
        ("feeBtc", fee.map_or(Value::Null, msat_to_btc)),
        ("occurredAt", field("occurred_at")),
        ("confirmedAt", field("confirmed_at")),
        ("kind", field("kind")),
        ("role", role),
        (
            "counterparty",
            string_or_none(&field("counterparty")).map_or(Value::Null, s),
        ),
        (
            "description",
            string_or_none(&field("description")).map_or(Value::Null, s),
        ),
        (
            "wallet",
            dict([
                ("id", field("wallet_id")),
                ("label", field("wallet_label")),
                ("kind", field("wallet_kind")),
            ]),
        ),
    ]))
}

fn swap_route(conn: &Connection, profile_id: &str, row: &Value) -> Result<Value> {
    let row_id = text_or(g(row, "id"));
    if row_id.is_empty() {
        return Ok(Value::Null);
    }
    let Some(pair) = rows(
        conn,
        "SELECT p.id AS pair_id, p.kind AS pair_kind, p.policy AS pair_policy, p.swap_fee_msat, p.swap_fee_kind, p.confidence_at_pair, p.pair_source, p.out_amount AS pair_out_amount_msat, p.created_at AS pair_created_at, out_t.id AS out_id, out_t.external_id AS out_external_id, out_t.direction AS out_direction, out_t.asset AS out_asset, out_t.amount AS out_full_amount_msat, p.out_amount AS out_amount_msat, out_t.fee AS out_fee_msat, out_t.occurred_at AS out_occurred_at, out_t.confirmed_at AS out_confirmed_at, out_t.kind AS out_kind, out_t.counterparty AS out_counterparty, out_t.description AS out_description, out_t.raw_json AS out_raw_json, out_w.id AS out_wallet_id, out_w.label AS out_wallet_label, out_w.kind AS out_wallet_kind, in_t.id AS in_id, in_t.external_id AS in_external_id, in_t.direction AS in_direction, in_t.asset AS in_asset, p.in_amount AS in_amount_msat, in_t.fee AS in_fee_msat, in_t.occurred_at AS in_occurred_at, in_t.confirmed_at AS in_confirmed_at, in_t.kind AS in_kind, in_t.counterparty AS in_counterparty, in_t.description AS in_description, in_t.raw_json AS in_raw_json, in_w.id AS in_wallet_id, in_w.label AS in_wallet_label, in_w.kind AS in_wallet_kind FROM journal_custody_projection_relations p JOIN transactions out_t ON out_t.id = p.out_transaction_id JOIN transactions in_t ON in_t.id = p.in_transaction_id JOIN wallets out_w ON out_w.id = out_t.wallet_id JOIN wallets in_w ON in_w.id = in_t.wallet_id WHERE p.profile_id = ? AND p.relation_kind IN ('move', 'conversion') AND (p.out_transaction_id = ? OR p.in_transaction_id = ?) ORDER BY p.created_at DESC, p.id DESC LIMIT 1",
        [profile_id, &row_id, &row_id],
    )?
    .into_iter()
    .next() else {
        return Ok(Value::Null);
    };
    let fee = int_or_none(g(&pair, "swap_fee_msat"));
    let out_amount = int_or_none(g(&pair, "out_amount_msat"));
    let out_full = int_or_none(g(&pair, "out_full_amount_msat"));
    Ok(without_none(dict([
        ("id", val(&pair, "pair_id")),
        ("kind", val(&pair, "pair_kind")),
        ("routeKind", s(route_kind(&pair))),
        ("policy", val(&pair, "pair_policy")),
        ("pairSource", val(&pair, "pair_source")),
        ("confidence", val(&pair, "confidence_at_pair")),
        ("createdAt", val(&pair, "pair_created_at")),
        (
            "currentLeg",
            s(if text(g(&pair, "out_id")) == row_id {
                "out"
            } else {
                "in"
            }),
        ),
        ("swapFeeMsat", opt_int(fee)),
        ("swapFeeBtc", fee.map_or(Value::Null, msat_to_btc)),
        ("swapFeeKind", val(&pair, "swap_fee_kind")),
        ("outAmountMsat", opt_int(out_amount)),
        ("outAmountBtc", out_amount.map_or(Value::Null, msat_to_btc)),
        ("outFullAmountMsat", opt_int(out_full)),
        (
            "outFullAmountBtc",
            out_full.map_or(Value::Null, msat_to_btc),
        ),
        ("out", route_leg(&pair, "out", out_amount)),
        (
            "in",
            route_leg(&pair, "in", int_or_none(g(&pair, "in_amount_msat"))),
        ),
    ])))
}

// ----------------------------------------------------------- payload

fn public_node(node: &Value) -> Value {
    match node {
        Value::Dict(fields) => Value::Dict(
            fields
                .iter()
                .filter(|(k, v)| {
                    !matches!(k, Key::Str(name) if name.as_str().is_some_and(|n| n.starts_with('_')))
                        && *v != Value::Null
                })
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

fn public_nodes(nodes: &[Value], side: &str) -> Value {
    let public: Vec<Value> = nodes.iter().map(public_node).collect();
    if public.len() <= MAX_NODES_PER_SIDE {
        return Value::List(public);
    }
    let keep = MAX_NODES_PER_SIDE - 1;
    let hidden = &public[keep..];
    let known: Vec<i128> = hidden.iter().filter_map(node_sats).collect();
    let total =
        (!known.is_empty() && known.len() == hidden.len()).then(|| known.iter().sum::<i128>());
    let mut overflow = dict([
        ("id", s(format!("{side}-overflow"))),
        ("role", s("overflow")),
        ("ownership", s("overflow")),
        ("overflow", Value::Bool(true)),
        ("overflowCount", hidden.len().into()),
        ("label", s(format!("+{} more", hidden.len()))),
        (
            "annotations",
            Value::List(vec![annotation(
                "overflow",
                &format!("{} aggregated {side} legs", hidden.len()),
                None,
            )]),
        ),
    ]);
    if let Some(total) = total {
        set(&mut overflow, "valueSats", int_value(total));
        set(&mut overflow, "valueBtc", sats_to_btc(Some(total)));
    }
    let mut output = public[..keep].to_vec();
    output.push(overflow);
    Value::List(output)
}

fn dedupe(warnings: Vec<Value>) -> Value {
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for warning in warnings {
        let code = text_or(g(&warning, "code"));
        let message = if truthy(g(&warning, "message")) {
            py_str(g(&warning, "message"))
        } else {
            humanize(&code)
        };
        if !seen.insert((code.clone(), message.clone())) {
            continue;
        }
        output.push(dict([
            ("code", s(code)),
            (
                "level",
                s(if truthy(g(&warning, "level")) {
                    py_str(g(&warning, "level"))
                } else {
                    "info".into()
                }),
            ),
            ("message", s(message)),
        ]));
    }
    Value::List(output)
}

fn empty_payload(reference: &str, reason: &str) -> Value {
    dict([
        ("transaction", Value::Null),
        ("supportLevel", s("graphless")),
        ("unsupportedReason", s(reason)),
        (
            "warnings",
            Value::List(vec![dict([
                ("code", s(reason)),
                ("level", s("warning")),
                (
                    "message",
                    s("Transaction graph is unavailable for the current profile."),
                ),
            ])]),
        ),
        ("inputs", Value::List(vec![])),
        ("outputs", Value::List(vec![])),
        ("fee", Value::Null),
        ("annotations", Value::List(vec![])),
        (
            "accounting",
            dict([
                ("quarantine", Value::Null),
                ("linkedPairs", Value::List(vec![])),
                ("transferGroupIds", Value::List(vec![])),
            ]),
        ),
        (
            "context",
            dict([("workspace", Value::Null), ("profile", Value::Null)]),
        ),
        ("swapRoute", Value::Null),
        ("query", s(reference)),
    ])
}

/// `_parse_public_lookup_arg`.
fn public_lookup(args: &Value) -> Result<bool> {
    let camel = get(args, "allowPublicLookup");
    let snake = get(args, "allow_public_lookup");
    if let (Some(camel), Some(snake)) = (camel, snake) {
        if !py_equal(camel, snake) {
            return Err(AppError::new(
                "validation",
                "ui.transactions.graph received conflicting public lookup flags",
            )
            .with_details(dict([
                ("allowPublicLookup", camel.clone()),
                ("allow_public_lookup", snake.clone()),
            ])));
        }
    }
    let Some(value) = camel.or(snake) else {
        return Ok(false);
    };
    match value {
        Value::Null => Ok(false),
        Value::Bool(b) => Ok(*b),
        Value::Str(t) if lower(&strip(t.as_str().unwrap_or_default())) == "true" => Ok(true),
        Value::Str(t) if lower(&strip(t.as_str().unwrap_or_default())) == "false" => Ok(false),
        other => Err(AppError::new(
            "validation",
            "ui.transactions.graph allowPublicLookup must be a boolean",
        )
        .with_details(dict([("allowPublicLookup", other.clone())]))),
    }
}

/// Python `==` across JSON values: numbers compare by value.
fn py_equal(left: &Value, right: &Value) -> bool {
    let number = |v: &Value| match v {
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Int(n) => n.to_string().parse::<f64>().ok(),
        Value::Float(f) => Some(*f),
        _ => None,
    };
    match (number(left), number(right)) {
        (Some(a), Some(b)) => match (left, right) {
            (Value::Int(a), Value::Int(b)) => a == b,
            _ => a == b,
        },
        _ => left == right,
    }
}

fn safe_detail(detail: &Value) -> Value {
    let Value::Dict(fields) = detail else {
        return dict::<&str>([]);
    };
    Value::Dict(
        fields
            .iter()
            .filter(|(k, v)| {
                let key = lower(&key_text(k));
                !["descriptor", "xpub", "config", "token", "secret", "raw"]
                    .iter()
                    .any(|t| key.contains(t))
                    && matches!(
                        v,
                        Value::Str(_)
                            | Value::Int(_)
                            | Value::Float(_)
                            | Value::Bool(_)
                            | Value::Null
                    )
            })
            .map(|(k, v)| (Key::Str(key_text(k).as_str().into()), v.clone()))
            .collect(),
    )
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Str(t) => py_str(&Value::Str(t.clone())),
        Key::Int(n) => n.to_string(),
        Key::Bool(b) => py_str(&Value::Bool(*b)),
        Key::Float(f) => py_str(&Value::Float(*f)),
        Key::Null => "None".into(),
    }
}

/// `build_transaction_graph_snapshot(conn, args, runtime_config)`.
pub fn snapshot(conn: &Connection, args: Option<&Value>) -> Result<Value> {
    let args = match args {
        Some(v @ Value::Dict(_)) => v.clone(),
        _ => dict::<&str>([]),
    };
    let Value::Dict(fields) = &args else {
        unreachable!()
    };
    let mut unknown: Vec<String> = fields
        .iter()
        .map(|(k, _)| key_text(k))
        .filter(|k| {
            !["transaction", "allowPublicLookup", "allow_public_lookup"].contains(&k.as_str())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(AppError::new(
            "validation",
            "ui.transactions.graph received unsupported fields",
        )
        .with_details(dict([(
            "unknown",
            Value::List(unknown.into_iter().map(s).collect()),
        )])));
    }
    let reference = strip(&text_or(g(&args, "transaction")));
    if reference.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.transactions.graph requires args.transaction",
        ));
    }
    let context = kassiber_setup::context::snapshot(conn)?;
    let profile_id = text_or(g(&context, "profile_id"));
    if profile_id.is_empty() {
        return Ok(empty_payload(&reference, "no_active_profile"));
    }
    let Some(row) = rows(
        conn,
        "SELECT t.*, w.label AS wallet_label, w.kind AS wallet_kind, w.config_json AS wallet_config_json, w.account_id AS wallet_account_id, COALESCE(a.code, 'treasury') AS account_code, COALESCE(a.label, 'Treasury') AS account_label, jq.reason AS quarantine_reason, jq.detail_json AS quarantine_detail_json FROM transactions t JOIN wallets w ON w.id = t.wallet_id LEFT JOIN accounts a ON a.id = w.account_id LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id WHERE t.profile_id = ? AND (t.id = ? OR t.external_id = ?) ORDER BY CASE WHEN t.id = ? THEN 0 ELSE 1 END, t.occurred_at DESC, t.created_at DESC, t.id DESC LIMIT 1",
        [&profile_id, &reference, &reference, &reference],
    )?
    .into_iter()
    .next() else {
        return Ok(empty_payload(&reference, "not_found"));
    };
    let profile = rows(conn, "SELECT * FROM profiles WHERE id = ?", [&profile_id])?
        .into_iter()
        .next()
        .unwrap_or_else(|| dict([("id", s(profile_id.as_str()))]));
    let semantics = semantics(conn, &profile)?;
    let spends = profile_spends(conn, &profile_id)?;
    let raw = json_obj(g(&row, "raw_json"));
    let allow = public_lookup(&args)?;
    let enriched = enrich(conn, &profile_id, &row, raw, allow)?;
    let local = local_outpoint_amounts(conn, &profile_id, &row, &enriched)?;
    let mut graph = parse_graph(&row, &enriched, &local)?;
    let mut references = BTreeSet::new();
    for node in graph.inputs.iter().chain(&graph.outputs) {
        if let Some(point) = string_or_none(g(node, "outpoint")) {
            references.insert(lower(&point));
        }
        if truthy(g(node, "_script")) {
            references.insert(lower(&py_str(g(node, "_script"))));
        }
    }
    let (index, index_warnings) = crate::loader::owned_index(conn, &profile_id, &references)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let tx_id = text(g(&row, "id"));
    let by_row = semantics.by_row.get(&tx_id).cloned().unwrap_or_default();
    annotate(&mut graph, &row, &index, &by_row, &enriched);
    annotate_spends(&row, &mut graph, &spends);
    annotate_funding(conn, &profile_id, &row, &mut graph)?;
    annotate_heights(conn, &profile_id, &row, &mut graph)?;
    let mut warnings = std::mem::take(&mut graph.warnings);
    warnings.extend(index_warnings.iter().map(|message| {
        dict([
            ("code", s("ownership_index")),
            ("level", s("info")),
            ("message", s(py_str(message))),
        ])
    }));
    if !semantics.current {
        warnings.push(dict([
            ("code", s("custody_projection_stale")),
            ("level", s("warning")),
            (
                "message",
                s("Custody annotations are unavailable until journals are rebuilt."),
            ),
        ]));
    }
    for item in &by_row {
        let code = text_or(g(item, "code"));
        if text_or(g(item, "severity")) == "warning" || code.ends_with("ambiguous") {
            warnings.push(dict([
                ("code", val(item, "code")),
                ("level", s("warning")),
                (
                    "message",
                    if truthy(g(item, "label")) {
                        val(item, "label")
                    } else {
                        s(humanize(&code))
                    },
                ),
            ]));
        }
    }
    let quarantine_reason = g(&row, "quarantine_reason");
    if truthy(quarantine_reason) {
        let reason = py_str(quarantine_reason);
        warnings.push(dict([
            ("code", s(reason.as_str())),
            ("level", s("warning")),
            (
                "message",
                s(format!("Journal blocker: {}", humanize(&reason))),
            ),
        ]));
    }
    let amount = int_or_none(g(&row, "amount")).unwrap_or(0);
    let fee = int_or_none(g(&row, "fee")).unwrap_or(0);
    let (chain, network) = default_row_chain_network(&row);
    let transaction = dict([
        ("id", s(tx_id.as_str())),
        ("chain", s(chain)),
        ("network", s(network)),
        (
            "blockHeight",
            opt_int(row_block_height(&row, Some(&enriched))),
        ),
        (
            "externalId",
            string_or_none(g(&row, "external_id")).map_or(Value::Null, s),
        ),
        ("txid", txid_from_row(&row).map_or(Value::Null, s)),
        ("occurredAt", val(&row, "occurred_at")),
        ("confirmedAt", val(&row, "confirmed_at")),
        ("direction", val(&row, "direction")),
        ("asset", val(&row, "asset")),
        ("amountMsat", int_value(amount)),
        ("amountBtc", msat_to_btc(amount)),
        ("feeMsat", int_value(fee)),
        ("feeBtc", msat_to_btc(fee)),
        (
            "wallet",
            dict([
                ("id", val(&row, "wallet_id")),
                ("label", val(&row, "wallet_label")),
                ("kind", val(&row, "wallet_kind")),
            ]),
        ),
        ("inputCount", val(&graph.metadata, "inputCount")),
        ("outputCount", val(&graph.metadata, "outputCount")),
        ("version", val(&graph.metadata, "version")),
        ("locktime", val(&graph.metadata, "locktime")),
        ("size", val(&graph.metadata, "size")),
        ("vsize", val(&graph.metadata, "vsize")),
        ("weight", val(&graph.metadata, "weight")),
        ("feeRateSatVb", val(&graph.fee, "rateSatVb")),
    ]);
    let route = if semantics.current {
        swap_route(conn, &profile_id, &row)?
    } else {
        Value::Null
    };
    let quarantine = if truthy(quarantine_reason) {
        dict([
            ("reason", quarantine_reason.clone()),
            (
                "detail",
                safe_detail(&json_obj(g(&row, "quarantine_detail_json"))),
            ),
        ])
    } else {
        Value::Null
    };
    let group_ids: BTreeSet<String> = by_row
        .iter()
        .filter(|a| truthy(g(a, "groupId")))
        .map(|a| py_str(g(a, "groupId")))
        .collect();
    let context_text = |key: &str| {
        if truthy(g(&context, key)) {
            val(&context, key)
        } else {
            Value::Null
        }
    };
    Ok(dict([
        ("transaction", transaction),
        ("supportLevel", s(graph.support)),
        ("unsupportedReason", graph.reason.map_or(Value::Null, s)),
        ("warnings", dedupe(warnings)),
        ("inputs", public_nodes(&graph.inputs, "input")),
        ("outputs", public_nodes(&graph.outputs, "output")),
        ("fee", graph.fee),
        ("annotations", Value::List(by_row.clone())),
        (
            "accounting",
            dict([
                ("quarantine", quarantine),
                (
                    "custodyProjection",
                    s(if semantics.current {
                        "current"
                    } else {
                        "stale"
                    }),
                ),
                ("linkedPairs", Value::List(by_row)),
                (
                    "transferGroupIds",
                    Value::List(group_ids.into_iter().map(s).collect()),
                ),
            ]),
        ),
        (
            "context",
            dict([
                ("workspace", context_text("workspace_label")),
                ("profile", context_text("profile_label")),
            ]),
        ),
        ("swapRoute", route),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_transaction_sizes_follow_the_bip141_walk() {
        // A one-input, one-output legacy transaction: 4+1+41+1+9+4 bytes.
        let mut tx = vec![1, 0, 0, 0, 1];
        tx.extend([0u8; 36]);
        tx.extend([0, 0xff, 0xff, 0xff, 0xff, 1]);
        tx.extend([0u8; 8]);
        tx.extend([0, 0, 0, 0, 0]);
        assert_eq!(
            decoded_sizes(&tx).unwrap(),
            vec![("size", 60), ("vsize", 60), ("weight", 240)]
        );
        assert_eq!(decoded_sizes(&tx[..20]), None);
        assert_eq!(
            decoded_sizes(&[tx.clone(), vec![0]].concat()).unwrap(),
            vec![]
        );
    }

    #[test]
    fn helpers_match_python_spellings() {
        assert_eq!(humanize("missing_fee-price"), "Missing fee price");
        assert_eq!(round2(1.005), 1.0);
        assert_eq!(from_hex("aa bb"), Some(vec![0xaa, 0xbb]));
        assert_eq!(from_hex("a ab"), None);
        assert!(exact_change("m/1 Change"));
        assert!(!exact_change("receive"));
    }

    #[test]
    fn foreign_asset_nodes_carry_the_full_asset_id_beside_the_label() {
        let full = "ce".repeat(32);
        let mut node = dict([
            ("asset", s("BTC")),
            ("valueSats", Value::Int(5.into())),
            ("valueBtc", Value::Null),
            ("valueState", s("known")),
        ]);
        assert!(apply_foreign_asset(&mut node, Some(full.clone()), "BTC"));
        let Value::Dict(pairs) = &node else { panic!() };
        let keys: Vec<_> = pairs.iter().map(|(k, _)| format!("{k:?}")).collect();
        assert_eq!(keys.len(), 5);
        assert!(keys[4].contains("assetId"), "{keys:?}");
        assert_eq!(get(&node, "assetId"), Some(&s(full.as_str())));
        assert_ne!(get(&node, "asset"), Some(&s(full.as_str())));
        let mut same = dict([("asset", s("BTC"))]);
        assert!(!apply_foreign_asset(&mut same, Some("BTC".into()), "BTC"));
        assert_eq!(get(&same, "assetId"), None);
    }
}
