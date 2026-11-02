//! `transaction_graph` / `onchain` leg helpers used before acquired
//! observations are stored. Witness stacks and preimages never survive.
use super::super::routing::py_text;
use super::super::*;
use kassiber_pyjson::BigInt;

pub(super) fn int(value: i64) -> Value {
    Value::Int(value.into())
}

/// `_string_or_none`: `str(value).strip() or None`.
pub(super) fn string_or_none(value: &Value) -> Option<String> {
    if value == &Value::Null {
        return None;
    }
    let text = kassiber_pytext::strip(&py_text(value)).to_owned();
    (!text.is_empty()).then_some(text)
}

/// `_int_or_none`: Python `int(value)`, or None for blanks and refusals.
pub(super) fn int_or_none(value: &Value) -> Option<BigInt> {
    match value {
        Value::Null => None,
        Value::Str(text) if text.code_points().next().is_none() => None,
        Value::Bool(flag) => Some(BigInt::from(i64::from(*flag))),
        Value::Int(n) => Some(n.clone()),
        Value::Float(f) if f.is_finite() => Some(BigInt::from(f.trunc() as i128)),
        Value::Str(_) => kassiber_pytext::parse_int(&py_text(value)).ok(),
        _ => None,
    }
}

fn lower(text: &str) -> String {
    kassiber_pytext::lower(text)
}

pub(super) fn is_txid(value: &Value) -> bool {
    super::super::sources::is_txid(value)
}

/// `onchain.output_script`.
pub(super) fn output_script(entry: &Value) -> Value {
    let mut script = get(entry, "scriptpubkey").cloned().unwrap_or(Value::Null);
    if script == Value::Null {
        script = get(entry, "script_hex").cloned().unwrap_or(Value::Null);
    }
    let nested = get(entry, "scriptPubKey").unwrap_or(&Value::Null);
    if script == Value::Null {
        if let Value::Dict(_) = nested {
            let hex = field(nested, "hex");
            script = if truthy(hex) {
                hex.clone()
            } else {
                get(nested, "scriptpubkey").cloned().unwrap_or(Value::Null)
            };
        }
    }
    if script == Value::Null {
        if let Value::Str(_) = nested {
            script = nested.clone();
        }
    }
    script
}

fn blank(value: &Value) -> bool {
    crate::publication::row::blank(value)
}

/// `onchain.output_address`.
pub(super) fn output_address(entry: &Value) -> Option<String> {
    let mut address = field(entry, "scriptpubkey_address");
    if !truthy(address) {
        address = field(entry, "address");
    }
    let nested = field(entry, "scriptPubKey");
    let mut owned = address.clone();
    if blank(&owned) {
        if let Value::Dict(_) = nested {
            owned = field(nested, "address").clone();
            if blank(&owned) {
                if let Value::List(items) = field(nested, "addresses") {
                    if let Some(first) = items.first() {
                        owned = first.clone();
                    }
                }
            }
        }
    }
    if blank(&owned) {
        return None;
    }
    let text = kassiber_pytext::strip(&py_text(&owned)).to_owned();
    (!text.is_empty()).then_some(text)
}

/// `onchain.normalized_script_hex`.
pub(super) fn normalized_script_hex(value: &Value) -> Option<String> {
    if blank(value) {
        return None;
    }
    let text = lower(kassiber_pytext::strip(&py_text(value)));
    if text.chars().count() % 2 == 1 {
        return None;
    }
    // bytes.fromhex permits whitespace only between complete byte pairs.
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if !c.is_ascii_hexdigit() || !chars.next().is_some_and(|d| d.is_ascii_hexdigit()) {
            return None;
        }
    }
    Some(text)
}

/// `onchain.output_value_sats`, through the ported feature extractor.
pub(super) fn output_value_sats(entry: &Value) -> Option<BigInt> {
    crate::analysis_features::value_sats(&to_serde(entry))
}

pub(super) fn to_serde(value: &Value) -> serde_json::Value {
    serde_json::from_str(
        &kassiber_pyjson::dumps(value, &Default::default()).unwrap_or_else(|_| "null".into()),
    )
    .unwrap_or(serde_json::Value::Null)
}

pub(super) fn from_serde(value: &serde_json::Value) -> Value {
    kassiber_pytext::loads(&value.to_string()).unwrap_or(Value::Null)
}

fn confidential_leg(entry: &Value) -> bool {
    let mut marker = field(entry, "value_state");
    if !truthy(marker) {
        marker = field(entry, "valueState");
    }
    let marker = if truthy(marker) {
        py_text(marker)
    } else {
        String::new()
    };
    if lower(kassiber_pytext::strip(&marker)) == "confidential" {
        return true;
    }
    [
        "valuecommitment",
        "assetcommitment",
        "surjectionproof",
        "rangeproof",
    ]
    .iter()
    .any(|key| get(entry, key).is_some_and(|v| v != &Value::Null))
}

fn first_truthy<'a>(entry: &'a Value, keys: &[&str]) -> &'a Value {
    for key in keys {
        let value = field(entry, key);
        if truthy(value) {
            return value;
        }
    }
    keys.last().map_or(&Value::Null, |key| field(entry, key))
}

fn liquid_fee_output(entry: &Value) -> bool {
    let explicit = string_or_none(first_truthy(entry, &["scriptpubkey_type", "type", "role"]));
    if explicit.is_some_and(|t| lower(&t) == "fee") {
        return true;
    }
    if confidential_leg(entry) || truthy(field(entry, "scriptpubkey_address")) {
        return false;
    }
    if string_or_none(first_truthy(entry, &["scriptpubkey", "script_hex"])).is_some() {
        return false;
    }
    output_value_sats(entry).is_some()
}

/// `wallet_descriptors.normalize_asset_code`.
fn asset_code(text: &str) -> String {
    let text = kassiber_pytext::strip(text);
    let lowered = lower(text);
    if lowered.len() == 64 && lowered.bytes().all(|b| b.is_ascii_hexdigit()) {
        return lowered;
    }
    if lowered == "l-btc" || lowered == "lbtc" {
        return "LBTC".into();
    }
    text.to_uppercase()
}

fn leg_asset(entry: &Value) -> Option<String> {
    if let Some(id) = string_or_none(first_truthy(entry, &["asset_id", "assetId"])) {
        let code = asset_code(&id);
        return Some(if code.is_empty() { lower(&id) } else { code });
    }
    let asset = string_or_none(field(entry, "asset"))?;
    if asset.chars().count() == 64 {
        let code = asset_code(&asset);
        return Some(if code.is_empty() { lower(&asset) } else { code });
    }
    Some(asset.to_uppercase())
}

fn pegout_address(entry: &Value) -> Option<String> {
    for key in ["pegout_address", "pegoutAddress"] {
        if let Some(address) = string_or_none(field(entry, key)) {
            return Some(address);
        }
    }
    let pegout = field(entry, "pegout");
    if let Value::Dict(_) = pegout {
        return string_or_none(first_truthy(
            pegout,
            &["scriptpubkey_address", "scriptpubkeyAddress"],
        ));
    }
    None
}

fn flag(entry: &Value, keys: &[&str]) -> Option<bool> {
    for key in keys {
        match field(entry, key) {
            Value::Bool(value) => return Some(*value),
            Value::Str(text)
                if ["true", "1"].contains(
                    &lower(kassiber_pytext::strip(&py_text(&Value::Str(text.clone())))).as_str(),
                ) =>
            {
                return Some(true)
            }
            _ => {}
        }
    }
    None
}

fn coinbase_leg(entry: &Value) -> bool {
    if let Some(value) = flag(entry, &["is_coinbase", "isCoinbase", "coinbase"]) {
        return value;
    }
    string_or_none(field(entry, "txid")).is_some_and(|t| lower(&t) == "00".repeat(32))
        && int_or_none(field(entry, "vout")) == Some(BigInt::from(0xFFFF_FFFFu32))
}

fn pegin_leg(entry: &Value) -> bool {
    flag(entry, &["is_pegin", "isPegin", "pegin"]).unwrap_or(false)
}

fn value_script(entry: &Value, chain: &str, include_n: bool) -> Value {
    let mut clean = dict::<&str>([]);
    if include_n {
        if let Some(n) = int_or_none(field(entry, "n")) {
            set(&mut clean, "n", Value::Int(n));
        }
    }
    if let Some(script) = string_or_none(&output_script(entry)) {
        set(&mut clean, "scriptpubkey", s(script.as_str()));
    }
    if let Some(kind) = string_or_none(first_truthy(entry, &["scriptpubkey_type", "type"])) {
        set(&mut clean, "scriptpubkey_type", s(kind.as_str()));
    }
    if let Some(address) = output_address(entry) {
        set(&mut clean, "scriptpubkey_address", s(address.as_str()));
    }
    if let Some(asset) = leg_asset(entry) {
        set(&mut clean, "asset", s(asset.as_str()));
    }
    if let Some(address) = pegout_address(entry) {
        set(&mut clean, "pegout_address", s(address.as_str()));
    }
    let value = output_value_sats(entry);
    let mut probe = clean.clone();
    set(
        &mut probe,
        "value",
        value.clone().map_or(Value::Null, Value::Int),
    );
    if confidential_leg(entry) || (chain == "liquid" && !liquid_fee_output(&probe)) {
        set(&mut clean, "value_state", s("confidential"));
    } else if let Some(value) = value {
        set(&mut clean, "value", Value::Int(value));
    }
    clean
}

/// `transaction_graph._sanitize_graph_lookup_raw`.
pub(super) fn sanitize(raw: &Value, chain: &str, txid: &str) -> Value {
    let mut clean = dict::<&str>([]);
    let raw_txid = string_or_none(field(raw, "txid")).unwrap_or_else(|| txid.to_owned());
    if !raw_txid.is_empty() {
        set(&mut clean, "txid", s(lower(&raw_txid).as_str()));
    }
    for key in ["version", "locktime", "size", "vsize", "weight"] {
        if let Some(value) = int_or_none(field(raw, key)) {
            set(&mut clean, key, Value::Int(value));
        }
    }
    if let Value::List(vin) = field(raw, "vin") {
        let inputs = vin
            .iter()
            .filter(|entry| matches!(entry, Value::Dict(_)))
            .map(|entry| {
                let mut leg = dict::<&str>([]);
                if let Some(txid) = string_or_none(field(entry, "txid")) {
                    set(&mut leg, "txid", s(lower(&txid).as_str()));
                }
                if let Some(vout) = int_or_none(field(entry, "vout")) {
                    set(&mut leg, "vout", Value::Int(vout));
                }
                if pegin_leg(entry) {
                    set(&mut leg, "is_pegin", Value::Bool(true));
                }
                if coinbase_leg(entry) {
                    set(&mut leg, "is_coinbase", Value::Bool(true));
                }
                if let prevout @ Value::Dict(_) = field(entry, "prevout") {
                    let prevout = value_script(prevout, chain, false);
                    if truthy(&prevout) {
                        set(&mut leg, "prevout", prevout);
                    }
                }
                leg
            })
            .collect();
        set(&mut clean, "vin", Value::List(inputs));
    }
    if let Value::List(vout) = field(raw, "vout") {
        let outputs = vout
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, Value::Dict(_)))
            .map(|(index, entry)| {
                let mut leg = value_script(entry, chain, true);
                if get(&leg, "n").is_none() {
                    set(&mut leg, "n", int(index as i64));
                }
                leg
            })
            .collect();
        set(&mut clean, "vout", Value::List(outputs));
    }
    clean
}

/// `onchain.input_outpoint`.
pub(super) fn input_outpoint(entry: &Value) -> Option<(String, BigInt)> {
    let txid = field(entry, "txid");
    let txid = if truthy(txid) {
        py_text(txid)
    } else {
        String::new()
    };
    let txid = lower(kassiber_pytext::strip(&txid));
    if txid.len() != 64
        || !txid.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        || txid == "0".repeat(64)
    {
        return None;
    }
    let vout = match field(entry, "vout") {
        Value::Null | Value::List(_) | Value::Dict(_) => return None,
        other => int_or_none(other)?,
    };
    (vout >= BigInt::from(0)).then_some((txid, vout))
}

/// `_bitcoinrpc_vout_to_graph`.
pub(super) fn rpc_vout(source: &Value) -> Value {
    let script = field(source, "scriptPubKey");
    let script_obj = if matches!(script, Value::Dict(_)) {
        script.clone()
    } else {
        dict::<&str>([])
    };
    let mut address = string_or_none(field(&script_obj, "address"));
    if address.is_none() {
        if let Value::List(items) = field(&script_obj, "addresses") {
            if let Some(first) = items.first() {
                address = string_or_none(first);
            }
        }
    }
    let either = |a: &str, b: &str| {
        let first = field(&script_obj, a);
        if truthy(first) {
            first.clone()
        } else {
            get(source, b).cloned().unwrap_or(Value::Null)
        }
    };
    let mut payload = dict([
        ("n", get(source, "n").cloned().unwrap_or(Value::Null)),
        (
            "value",
            get(source, "value").cloned().unwrap_or(Value::Null),
        ),
        ("scriptpubkey", either("hex", "scriptpubkey")),
        ("scriptpubkey_type", either("type", "scriptpubkey_type")),
    ]);
    if let Some(address) = address {
        set(&mut payload, "scriptpubkey_address", s(address.as_str()));
    }
    payload
}

/// `_bitcoinrpc_decoded_to_graph_raw`.
pub(super) fn rpc_graph(decoded: &Value) -> Value {
    let get_or = |key: &str| get(decoded, key).cloned().unwrap_or(Value::Null);
    let txid = string_or_none(field(decoded, "txid")).map_or(Value::Null, |t| s(t.as_str()));
    let mut inputs = Vec::new();
    if let Value::List(vin) = field(decoded, "vin") {
        for entry in vin.iter().filter(|e| matches!(e, Value::Dict(_))) {
            let mut input = dict([
                ("txid", get(entry, "txid").cloned().unwrap_or(Value::Null)),
                ("vout", get(entry, "vout").cloned().unwrap_or(Value::Null)),
                (
                    "sequence",
                    get(entry, "sequence").cloned().unwrap_or(Value::Null),
                ),
            ]);
            if matches!(field(entry, "coinbase"), Value::Str(t) if t.code_points().next().is_some())
            {
                set(&mut input, "is_coinbase", Value::Bool(true));
            }
            if let prevout @ Value::Dict(_) = field(entry, "prevout") {
                set(&mut input, "prevout", rpc_vout(prevout));
            }
            inputs.push(input);
        }
    }
    let outputs = match field(decoded, "vout") {
        Value::List(vout) => vout
            .iter()
            .filter(|e| matches!(e, Value::Dict(_)))
            .map(rpc_vout)
            .collect(),
        _ => vec![],
    };
    dict([
        ("txid", txid),
        ("version", get_or("version")),
        ("locktime", get_or("locktime")),
        ("size", get_or("size")),
        ("vsize", get_or("vsize")),
        ("weight", get_or("weight")),
        ("raw_hex", get_or("hex")),
        ("vin", Value::List(inputs)),
        ("vout", Value::List(outputs)),
    ])
}
