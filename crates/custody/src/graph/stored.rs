//! Stored mappings retain Python scalar types until parsing and merging finish.
//! In particular JSON NaN/Infinity must not become absent facts via serde nulls.
use crate::{codec, interpretation::python_value_string as py_string, python_text::PythonText};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{BigInt, Key, Value as Py};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

/// Whether a dict key is the string `key`.
fn is_key(k: &Key, key: &str) -> bool {
    matches!(k, Key::Str(s) if s.is(key))
}
pub(crate) fn get<'a>(value: &'a Py, key: &str) -> &'a Py {
    if let Py::Dict(entries) = value {
        if let Some((_, value)) = entries.iter().find(|(k, _)| is_key(k, key)) {
            return value;
        }
    }
    &Py::Null
}
pub(crate) fn put(value: &mut Py, key: &str, item: Py) {
    let Py::Dict(entries) = value else {
        unreachable!("graph dictionary")
    };
    if let Some((_, value)) = entries.iter_mut().find(|(k, _)| is_key(k, key)) {
        *value = item;
    } else {
        entries.push((Key::Str(key.into()), item));
    }
}
fn dict(entries: &[(&str, Py)]) -> Py {
    Py::Dict(
        entries
            .iter()
            .map(|(k, v)| (Key::Str((*k).into()), v.clone()))
            .collect(),
    )
}
fn string(value: impl Into<String>) -> Py {
    Py::Str(value.into().into())
}
pub(crate) fn list(value: &Py) -> &[Py] {
    if let Py::List(items) = value {
        items
    } else {
        &[]
    }
}
pub(crate) fn truth(value: &Py) -> bool {
    match value {
        Py::Null => false,
        Py::Bool(v) => *v,
        Py::Int(v) => *v != BigInt::from(0),
        Py::Float(v) => *v != 0.0,
        Py::Str(v) => v.code_points().next().is_some(),
        Py::Bytes(v) => !v.is_empty(),
        Py::List(v) => !v.is_empty(),
        Py::Dict(v) => !v.is_empty(),
        Py::Decimal(_) => unreachable!(),
    }
}
fn present(value: &Py) -> bool {
    !matches!(value, Py::Null) && !matches!(value,Py::Str(s) if s.code_points().next().is_none())
}
pub(crate) fn text(value: &Py) -> String {
    if truth(value) {
        py_string(value)
    } else {
        String::new()
    }
}

/// Equality of raw JSON scalars, including exact int/float comparisons. JSON's
/// NaN constant is shared by CPython's decoder: container comparisons use its
/// identity shortcut, while direct scalar comparison still reports unequal.
pub(crate) fn equal(left: &Py, right: &Py, nested: bool) -> bool {
    let integer = |v: &Py| match v {
        Py::Int(v) => Some(v.clone()),
        Py::Bool(v) => Some(BigInt::from(u8::from(*v))),
        _ => None,
    };
    match (left, right) {
        (Py::Float(a), Py::Float(b)) => a == b || nested && a.is_nan() && b.is_nan(),
        (Py::Float(f), other) | (other, Py::Float(f)) => {
            f.is_finite()
                && f.fract() == 0.0
                && integer(other)
                    .is_some_and(|n| BigInt::from_str(&format!("{f:.0}")).ok() == Some(n))
        }
        (Py::List(a), Py::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b, true))
        }
        (Py::Dict(a), Py::Dict(b)) => {
            a.len() == b.len()
                && a.iter().all(|(k, v)| {
                    b.iter()
                        .find(|(other, _)| other == k)
                        .is_some_and(|(_, other)| equal(v, other, true))
                })
        }
        _ => {
            if let (Some(a), Some(b)) = (integer(left), integer(right)) {
                a == b
            } else {
                left == right
            }
        }
    }
}
pub(crate) fn integer(value: &Py) -> Result<BigInt, &'static str> {
    match value {
        Py::Int(v) => Ok(v.clone()),
        Py::Bool(v) => Ok(BigInt::from(u8::from(*v))),
        Py::Float(v) if v.is_infinite() => Err("OverflowError"),
        Py::Float(v) if v.is_nan() => Err("ValueError"),
        Py::Float(v) => BigInt::from_str(&format!("{:.0}", v.trunc())).map_err(|_| "ValueError"),
        Py::Str(v) => {
            let s = v.as_str().ok_or("ValueError")?;
            if s.chars()
                .filter(|c| crate::python_text::decimal_digit(*c).is_some())
                .count()
                > kassiber_pyjson::INT_MAX_STR_DIGITS
            {
                return Err("ValueError");
            }
            let n = codec::integer(&json!(s)).map_err(|_| "ValueError")?;
            BigInt::from_str(&n.to_string()).map_err(|_| "ValueError")
        }
        _ => Err("TypeError"),
    }
}
fn index(value: &Py, fallback: Option<usize>) -> Result<Option<BigInt>, String> {
    match integer(value) {
        Ok(n) => Ok(Some(n)),
        Err("OverflowError") => Err("OverflowError".into()),
        Err(_) => Ok(fallback.map(BigInt::from)),
    }
}
fn script(entry: &Py) -> Py {
    for key in ["scriptpubkey", "script_hex"] {
        if !matches!(get(entry, key), Py::Null) {
            return get(entry, key).clone();
        }
    }
    let nested = get(entry, "scriptPubKey");
    if let Py::Str(_) = nested {
        return nested.clone();
    }
    if truth(get(nested, "hex")) {
        get(nested, "hex").clone()
    } else {
        get(nested, "scriptpubkey").clone()
    }
}
pub(crate) fn output_sats(entry: &Py) -> Option<BigInt> {
    match sats(entry) {
        Py::Int(n) => Some(n),
        _ => None,
    }
}
fn sats(entry: &Py) -> Py {
    let explicit = !matches!(get(entry, "value_sats"), Py::Null);
    let value = get(entry, if explicit { "value_sats" } else { "value" });
    if !present(value) || matches!(value, Py::Bool(_)) {
        return Py::Null;
    }
    let spelling = py_string(value);
    if explicit
        || matches!(value, Py::Int(_))
        || matches!(value, Py::Str(_)) && !spelling.contains('.')
    {
        return integer(value).map(Py::Int).unwrap_or(Py::Null);
    }
    Decimal::from_str(spelling.py_strip())
        .ok()
        .and_then(|d| d.mul(&Decimal::from(100_000_000), &Context::RP2).ok())
        .and_then(|d| d.to_fixed_places(0, Rounding::HalfEven).ok())
        .and_then(|s| BigInt::from_str(&s).ok())
        .map(Py::Int)
        .unwrap_or(Py::Null)
}
fn assets(entry: &Py) -> (Py, Py) {
    let mut id = if present(get(entry, "asset_id")) {
        string(py_string(get(entry, "asset_id")).py_strip().py_lower())
    } else {
        Py::Null
    };
    let mut asset = if present(get(entry, "asset")) {
        string(py_string(get(entry, "asset")).py_strip().py_upper())
    } else {
        Py::Null
    };
    let spelling = text(&asset);
    // Python bytes.fromhex accepts embedded ASCII whitespace. The 64-character
    // spelling test precedes decoding; it does not require 32 decoded bytes.
    if matches!(id, Py::Null) && spelling.chars().count() == 64 {
        // Rust's ASCII whitespace helpers omit vertical tab; bytes.fromhex
        // accepts all six ASCII whitespace characters, and no others.
        let hex_whitespace = |c: char| matches!(c, ' ' | '\t'..='\r');
        let compact: String = spelling.chars().filter(|c| !hex_whitespace(*c)).collect();
        // Whitespace is only legal between complete bytes.
        let valid = spelling.split(hex_whitespace).all(|part| {
            part.len().is_multiple_of(2) && part.bytes().all(|c| c.is_ascii_hexdigit())
        });
        if compact.len().is_multiple_of(2) && valid {
            id = string(spelling.py_lower());
            asset = Py::Null;
        }
    }
    (id, asset)
}
fn mapping(payload: &Py, chain: &str, network: &str) -> Result<Option<Py>, String> {
    let (Py::List(vin), Py::List(vout)) = (get(payload, "vin"), get(payload, "vout")) else {
        return Ok(None);
    };
    let mut inputs = Vec::new();
    for entry in vin.iter().filter(|v| matches!(v, Py::Dict(_))) {
        let txid = text(get(entry, "txid")).py_strip().py_lower();
        let valid = txid.len() == 64
            && txid.bytes().all(|c| c.is_ascii_hexdigit())
            && txid != "0".repeat(64);
        let outpoint = if valid {
            index(get(entry, "vout"), None)?
                .filter(|n| n >= &BigInt::from(0))
                .map(|n| string(format!("{txid}:{n}")))
                .unwrap_or(Py::Null)
        } else {
            Py::Null
        };
        let prevout = get(entry, "prevout");
        let (id, asset) = assets(if matches!(prevout, Py::Dict(_)) {
            prevout
        } else {
            entry
        });
        let value = sats(prevout);
        inputs.push(dict(&[
            ("outpoint", outpoint),
            (
                "script",
                if matches!(prevout, Py::Dict(_)) {
                    script(prevout)
                } else {
                    Py::Null
                },
            ),
            (
                "value_sats",
                if matches!(value, Py::Null) {
                    sats(entry)
                } else {
                    value
                },
            ),
            ("asset_id", id),
            ("asset", asset),
        ]));
    }
    let mut outputs = Vec::new();
    for (position, entry) in vout
        .iter()
        .enumerate()
        .filter(|(_, v)| matches!(v, Py::Dict(_)))
    {
        let n = if let Py::Dict(entries) = entry {
            if entries.iter().any(|(k, _)| is_key(k, "n")) {
                index(get(entry, "n"), Some(position))?.unwrap()
            } else {
                BigInt::from(position)
            }
        } else {
            unreachable!()
        };
        let (id, asset) = assets(entry);
        outputs.push(dict(&[
            ("n", Py::Int(n)),
            ("script", script(entry)),
            ("value_sats", sats(entry)),
            ("asset_id", id),
            ("asset", asset),
            (
                "role",
                if truth(get(entry, "role")) {
                    string(py_string(get(entry, "role")))
                } else {
                    Py::Null
                },
            ),
        ]));
    }
    let c = text(get(payload, "chain"));
    let n = text(get(payload, "network"));
    Ok(Some(dict(&[
        ("txid", get(payload, "txid").clone()),
        (
            "chain",
            string(if c.is_empty() {
                chain.to_owned()
            } else {
                c.py_strip().py_lower()
            }),
        ),
        (
            "network",
            string(if n.is_empty() {
                network.to_owned()
            } else {
                n.py_strip().py_lower()
            }),
        ),
        ("inputs", Py::List(inputs)),
        ("outputs", Py::List(outputs)),
        (
            "component",
            if matches!(get(payload, "component"), Py::Dict(_)) {
                get(payload, "component").clone()
            } else {
                Py::Dict(vec![])
            },
        ),
        ("evidence_conflicts", Py::List(vec![])),
    ])))
}
fn merge_leg(target: &mut Py, incoming: &Py, label: &str, conflicts: &mut BTreeSet<String>) {
    for key in ["script", "value_sats", "asset_id", "asset", "role"] {
        let old = get(target, key);
        let value = get(incoming, key);
        if present(value) {
            if !present(old) {
                put(target, key, value.clone());
            } else if !equal(old, value, false) {
                if key == "role"
                    && BTreeSet::from([py_string(old), py_string(value)])
                        == BTreeSet::from(["external".into(), "owned".into()])
                {
                    put(target, key, string("owned"));
                } else {
                    conflicts.insert(format!("{label}.{key}"));
                }
            }
        }
    }
}
fn bitcoin_assets(graph: &mut Py) {
    for side in ["inputs", "outputs"] {
        let Py::List(legs) = graph_get_mut(graph, side) else {
            unreachable!()
        };
        for leg in legs {
            for key in ["asset_id", "asset"] {
                if !truth(get(leg, key)) {
                    put(leg, key, string("BTC"));
                }
            }
        }
    }
}
fn graph_get_mut<'a>(value: &'a mut Py, key: &str) -> &'a mut Py {
    let Py::Dict(entries) = value else {
        unreachable!()
    };
    &mut entries.iter_mut().find(|(k, _)| is_key(k, key)).unwrap().1
}
pub(crate) fn merge(items: &[Py]) -> Option<Py> {
    let mut merged = items.first()?.clone();
    let mut inputs: BTreeMap<String, Py> = BTreeMap::new();
    let mut positions = BTreeMap::new();
    let mut outputs: BTreeMap<BigInt, Py> = BTreeMap::new();
    let mut conflicts: BTreeSet<String> = list(get(&merged, "evidence_conflicts"))
        .iter()
        .map(py_string)
        .collect();
    for item in items {
        for key in ["txid", "chain", "network"] {
            let old = get(&merged, key);
            let value = get(item, key);
            if present(value) {
                if !present(old) {
                    put(&mut merged, key, value.clone());
                } else if !equal(old, value, false) {
                    conflicts.insert(key.into());
                }
            }
        }
        for (position, leg) in list(get(item, "inputs")).iter().enumerate() {
            let outpoint = text(get(leg, "outpoint")).py_strip().to_owned();
            let previous = positions.get(&position).cloned();
            let key = if !outpoint.is_empty() {
                outpoint.clone()
            } else {
                previous
                    .clone()
                    .unwrap_or_else(|| format!("position:{position}"))
            };
            if !outpoint.is_empty() && previous.as_ref().is_some_and(|v| v != &outpoint) {
                if let Some(old) = inputs.remove(previous.as_ref().unwrap()) {
                    if let Some(target) = inputs.get_mut(&outpoint) {
                        merge_leg(target, &old, &format!("input:{outpoint}"), &mut conflicts);
                    } else {
                        inputs.insert(outpoint.clone(), old);
                    }
                }
            }
            positions.insert(position, key.clone());
            if let Some(target) = inputs.get_mut(&key) {
                merge_leg(target, leg, &format!("input:{key}"), &mut conflicts);
            } else {
                inputs.insert(key.clone(), leg.clone());
            }
            if !outpoint.is_empty() {
                put(inputs.get_mut(&key).unwrap(), "outpoint", string(outpoint));
            }
        }
        for leg in list(get(item, "outputs")) {
            let n = integer(get(leg, "n")).expect("parsed graph index");
            if let Some(target) = outputs.get_mut(&n) {
                merge_leg(target, leg, &format!("output:{n}"), &mut conflicts);
            } else {
                outputs.insert(n, leg.clone());
            }
        }
    }
    put(
        &mut merged,
        "inputs",
        Py::List(inputs.into_values().collect()),
    );
    put(
        &mut merged,
        "outputs",
        Py::List(outputs.into_values().collect()),
    );
    put(
        &mut merged,
        "evidence_conflicts",
        Py::List(conflicts.into_iter().map(string).collect()),
    );
    if text(get(&merged, "chain")).py_lower() == "bitcoin" {
        bitcoin_assets(&mut merged);
    }
    Some(merged)
}
pub(crate) fn object(raw: &Value) -> Py {
    if let Some(bytes) = codec::raw_bytes(raw) {
        return codec::load_json_bytes(&bytes)
            .ok()
            .filter(|v| matches!(v, Py::Dict(_)))
            .unwrap_or_else(|| Py::Dict(vec![]));
    }
    let value = raw
        .as_str()
        .and_then(|s| codec::loads(s).ok())
        .or_else(|| codec::from_json(raw).ok());
    match value {
        Some(value @ Py::Dict(_)) => value,
        _ => Py::Dict(vec![]),
    }
}
pub(crate) fn parse(raw: &Value) -> Result<Option<Py>, String> {
    let outer = if let Some(bytes) = codec::raw_bytes(raw) {
        codec::load_json_bytes(&bytes).ok()
    } else if let Some(raw) = raw.as_str() {
        codec::loads(raw).ok()
    } else {
        codec::from_json(raw).ok()
    };
    let Some(outer @ Py::Dict(_)) = outer else {
        return Ok(None);
    };
    parse_object(&outer)
}

pub(crate) fn parse_object(outer: &Py) -> Result<Option<Py>, String> {
    let chain = text(get(outer, "chain")).py_strip().py_lower();
    let network = text(get(outer, "network")).py_strip().py_lower();
    let mut items = Vec::new();
    for payload in [outer, get(outer, "tx"), get(outer, "ownership_graph")] {
        if let Some(parsed) = mapping(payload, &chain, &network)? {
            items.push(parsed);
        }
    }
    if (chain == "liquid" || truth(get(get(outer, "component"), "asset_id")))
        && truth(get(outer, "raw_hex"))
    {
        if let Some(decoded) = crate::liquid::decode(&codec::to_json(get(outer, "raw_hex"))) {
            let mut decoded = codec::from_json(&decoded)?;
            put(&mut decoded, "network", string(network.clone()));
            items.push(decoded);
        }
    }
    let Some(mut graph) = merge(&items) else {
        return Ok(None);
    };
    if truth(get(outer, "txid")) {
        put(&mut graph, "txid", get(outer, "txid").clone());
    }
    if !chain.is_empty() {
        put(&mut graph, "chain", string(chain));
    } else if !truth(get(&graph, "chain")) {
        put(
            &mut graph,
            "chain",
            string(if truth(get(get(outer, "component"), "asset_id")) {
                "liquid"
            } else {
                "bitcoin"
            }),
        );
    }
    if !network.is_empty() {
        put(&mut graph, "network", string(network));
    }
    put(
        &mut graph,
        "component",
        if matches!(get(outer, "component"), Py::Dict(_)) {
            get(outer, "component").clone()
        } else {
            Py::Dict(vec![])
        },
    );
    if text(get(&graph, "chain")) == "bitcoin" {
        bitcoin_assets(&mut graph);
    }
    Ok(Some(graph))
}
/// Match the test-only `to_jsonable` wire projection, after all scalar-sensitive
/// admission and merging. Never interpret a raw {"$float": ...} as a scalar.
pub(crate) fn project(value: &Py) -> Value {
    match value {
        Py::Float(n) if !n.is_finite() => json!({"$float":kassiber_pyjson::float_repr(*n)}),
        Py::List(items) => Value::Array(items.iter().map(project).collect()),
        Py::Dict(items) => Value::Object(
            items
                .iter()
                .filter_map(|(k, v)| {
                    if let Key::Str(k) = k {
                        Some((k.as_str()?.to_owned(), project(v)))
                    } else {
                        None
                    }
                })
                .collect(),
        ),
        _ => codec::to_json(value),
    }
}

#[cfg(test)]
mod byte_policy_tests {
    use super::*;

    #[test]
    fn stored_graph_json_bytes_decode_only_at_the_mapping_consumer() {
        let text = r#"{"vin":[],"vout":[],"chain":"bitcoin","network":"main"}"#;
        let mut utf16 = vec![255, 254];
        utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        for bytes in [text.as_bytes().to_vec(), utf16] {
            let value = codec::to_json(&Py::Bytes(bytes));
            assert_eq!(object(&value), object(&json!(text)));
            assert_eq!(parse(&value).unwrap(), parse(&json!(text)).unwrap());
            assert!(parse(&value).unwrap().is_some());
        }
        for bytes in [vec![], vec![255, 0]] {
            let value = codec::to_json(&Py::Bytes(bytes.clone()));
            assert_eq!(object(&value), Py::Dict(vec![]));
            assert_eq!(parse(&value).unwrap(), None);
            assert_eq!(truth(&Py::Bytes(bytes.clone())), !bytes.is_empty());
        }
    }
}
