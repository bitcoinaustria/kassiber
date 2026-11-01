use super::*;

fn index(value: &Value, fallback: Option<usize>) -> Result<Option<BigInt>> {
    match integer(value) {
        Ok(Value::Int(n)) => Ok(Some(n)),
        Err(e) if is(field(&e.0, "family"), "OverflowError") => Err(e),
        _ => Ok(fallback.map(BigInt::from)),
    }
}

fn sats(entry: &Value) -> Value {
    let explicit = !matches!(field(entry, "value_sats"), Value::Null);
    let value = field(entry, if explicit { "value_sats" } else { "value" });
    if !present(value) || matches!(value, Value::Bool(_)) {
        return Value::Null;
    }
    let spelling = text(value);
    if explicit
        || matches!(value, Value::Int(_))
        || matches!(value, Value::Str(_)) && !spelling.code_points().any(|p| p == u32::from('.'))
    {
        return integer(value).unwrap_or(Value::Null);
    }
    spelling
        .as_str()
        .and_then(|s| s.parse::<Decimal>().ok())
        .and_then(|d| d.mul(&Decimal::from(100_000_000), &Context::RP2).ok())
        .and_then(|d| {
            d.to_fixed_places(0, kassiber_pydecimal::Rounding::HalfEven)
                .ok()
        })
        .and_then(|s| s.parse().ok())
        .map(Value::Int)
        .unwrap_or(Value::Null)
}

fn script(entry: &Value) -> Value {
    for key in ["scriptpubkey", "script_hex"] {
        if !matches!(field(entry, key), Value::Null) {
            return field(entry, key).clone();
        }
    }
    let nested = field(entry, "scriptPubKey");
    if matches!(nested, Value::Str(_)) {
        nested.clone()
    } else {
        fallback(field(nested, "hex"), field(nested, "scriptpubkey")).clone()
    }
}

fn mapping(payload: &Value, chain: &PyStr, network: &PyStr) -> Result<Option<Value>> {
    let (Value::List(vin), Value::List(vout)) = (field(payload, "vin"), field(payload, "vout"))
    else {
        return Ok(None);
    };
    let mut inputs = vec![];
    for entry in vin.iter().filter(|v| matches!(v, Value::Dict(_))) {
        let txid = strict_txid(field(entry, "txid")).filter(|s| !s.is(&"0".repeat(64)));
        let outpoint = if let Some(txid) = txid {
            index(field(entry, "vout"), None)?
                .filter(|n| n >= &BigInt::from(0))
                .map(|n| Value::Str(join(&[txid, ":".into(), n.to_string().into()])))
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let prevout = field(entry, "prevout");
        let value = sats(prevout);
        inputs.push(dict([
            ("outpoint", outpoint),
            (
                "script",
                if matches!(prevout, Value::Dict(_)) {
                    script(prevout)
                } else {
                    Value::Null
                },
            ),
            (
                "value_sats",
                if matches!(value, Value::Null) {
                    sats(entry)
                } else {
                    value
                },
            ),
        ]));
    }
    let mut outputs = vec![];
    for (position, entry) in vout
        .iter()
        .enumerate()
        .filter(|(_, v)| matches!(v, Value::Dict(_)))
    {
        let n = index(field(entry, "n"), Some(position))?.unwrap();
        outputs.push(dict([
            ("n", Value::Int(n)),
            ("script", script(entry)),
            ("value_sats", sats(entry)),
        ]));
    }
    let c = normalized(fallback(
        field(payload, "chain"),
        &Value::Str(chain.clone()),
    ));
    let n = normalized(fallback(
        field(payload, "network"),
        &Value::Str(network.clone()),
    ));
    Ok(Some(dict([
        ("txid", field(payload, "txid").clone()),
        ("chain", Value::Str(c)),
        ("network", Value::Str(n)),
        ("inputs", Value::List(inputs)),
        ("outputs", Value::List(outputs)),
    ])))
}

fn merge_leg(target: &mut Value, incoming: &Value) {
    for key in ["script", "value_sats"] {
        if !present(field(target, key)) && present(field(incoming, key)) {
            set(target, key, field(incoming, key).clone());
        }
    }
}

fn merge(items: &[Value]) -> Option<Value> {
    let mut merged = items.first()?.clone();
    let mut inputs = BTreeMap::<PyStr, Value>::new();
    let mut positions = BTreeMap::<usize, PyStr>::new();
    let mut outputs = BTreeMap::<BigInt, Value>::new();
    for item in items {
        for key in ["txid", "chain", "network"] {
            if !present(field(&merged, key)) && present(field(item, key)) {
                set(&mut merged, key, field(item, key).clone());
            }
        }
        for (position, leg) in list(field(item, "inputs")).iter().enumerate() {
            let outpoint = strip(&empty_text(field(leg, "outpoint")));
            let previous = positions.get(&position).cloned();
            let key = if outpoint.code_points().next().is_some() {
                outpoint.clone()
            } else {
                previous
                    .clone()
                    .unwrap_or_else(|| format!("position:{position}").into())
            };
            if outpoint.code_points().next().is_some()
                && previous.as_ref().is_some_and(|v| v != &outpoint)
            {
                if let Some(old) = inputs.remove(previous.as_ref().unwrap()) {
                    if let Some(target) = inputs.get_mut(&outpoint) {
                        merge_leg(target, &old);
                    } else {
                        inputs.insert(outpoint.clone(), old);
                    }
                }
            }
            positions.insert(position, key.clone());
            if let Some(target) = inputs.get_mut(&key) {
                merge_leg(target, leg);
            } else {
                inputs.insert(key.clone(), leg.clone());
            }
            if outpoint.code_points().next().is_some() {
                set(
                    inputs.get_mut(&key).unwrap(),
                    "outpoint",
                    Value::Str(outpoint),
                );
            }
        }
        for leg in list(field(item, "outputs")) {
            let Value::Int(n) = field(leg, "n") else {
                unreachable!();
            };
            if let Some(target) = outputs.get_mut(n) {
                merge_leg(target, leg);
            } else {
                outputs.insert(n.clone(), leg.clone());
            }
        }
    }
    set(
        &mut merged,
        "inputs",
        Value::List(inputs.into_values().collect()),
    );
    set(
        &mut merged,
        "outputs",
        Value::List(outputs.into_values().collect()),
    );
    Some(merged)
}

pub(in crate::native) fn parse(raw: &Value) -> Result<Option<Value>> {
    let outer = object(raw);
    let c = normalized(field(&outer, "chain"));
    let n = normalized(field(&outer, "network"));
    let mut items = vec![];
    for payload in [
        &outer,
        field(&outer, "tx"),
        field(&outer, "ownership_graph"),
    ] {
        if let Some(parsed) = mapping(payload, &c, &n)? {
            items.push(parsed);
        }
    }
    if (c.is("liquid") || truth(field(field(&outer, "component"), "asset_id")))
        && truth(field(&outer, "raw_hex"))
    {
        if let Some(mut decoded) =
            kassiber_custody::journal_support::native_liquid_graph(field(&outer, "raw_hex"))
        {
            set(&mut decoded, "network", Value::Str(n.clone()));
            items.push(decoded);
        }
    }
    let Some(mut graph) = merge(&items) else {
        return Ok(None);
    };
    if truth(field(&outer, "txid")) {
        set(&mut graph, "txid", field(&outer, "txid").clone());
    }
    if c.code_points().next().is_some() {
        set(&mut graph, "chain", Value::Str(c));
    } else if !truth(field(&graph, "chain")) {
        set(
            &mut graph,
            "chain",
            Value::from(if truth(field(field(&outer, "component"), "asset_id")) {
                "liquid"
            } else {
                "bitcoin"
            }),
        );
    }
    if n.code_points().next().is_some() {
        set(&mut graph, "network", Value::Str(n));
    }
    set(
        &mut graph,
        "component",
        if matches!(field(&outer, "component"), Value::Dict(_)) {
            field(&outer, "component").clone()
        } else {
            dict([])
        },
    );
    if list(field(&graph, "outputs"))
        .iter()
        .any(|o| matches!(field(o, "value_sats"), Value::Null))
    {
        return Ok(None);
    }
    Ok(Some(graph))
}
