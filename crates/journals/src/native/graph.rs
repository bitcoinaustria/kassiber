use super::*;
use std::collections::{BTreeMap, BTreeSet};

mod parsed;
pub(super) use parsed::parse;

pub(super) type Scope = (PyStr, PyStr, PyStr, PyStr);
type Physical = (PyStr, PyStr);

pub(super) fn empty_text(value: &Value) -> PyStr {
    if truth(value) {
        text(value)
    } else {
        PyStr::new()
    }
}

pub(super) fn lower(value: &PyStr) -> PyStr {
    case(value, kassiber_pytext::lower)
}

pub(super) fn upper(value: &PyStr) -> PyStr {
    case(value, kassiber_pytext::upper)
}

pub(super) fn case(value: &PyStr, convert: fn(&str) -> String) -> PyStr {
    let mut points = vec![];
    let mut chunk = String::new();
    for point in value.code_points() {
        if let Some(c) = char::from_u32(point) {
            chunk.push(c);
        } else {
            points.extend(convert(&chunk).chars().map(u32::from));
            chunk.clear();
            points.push(point);
        }
    }
    points.extend(convert(&chunk).chars().map(u32::from));
    PyStr::from_code_points(points).unwrap()
}

pub(super) fn strip(value: &PyStr) -> PyStr {
    kassiber_pytext::strip_py(value)
}

pub(super) fn normalized(value: &Value) -> PyStr {
    lower(&strip(&empty_text(value)))
}

pub(super) fn asset(value: &Value) -> PyStr {
    let value = upper(&strip(&empty_text(value)));
    replace(&value, "L-BTC", "LBTC")
}

pub(super) fn replace(value: &PyStr, from: &str, to: &str) -> PyStr {
    let points: Vec<_> = value.code_points().collect();
    let from: Vec<_> = from.chars().map(u32::from).collect();
    let to: Vec<_> = to.chars().map(u32::from).collect();
    let mut result = vec![];
    let mut at = 0;
    while at < points.len() {
        if points[at..].starts_with(&from) {
            result.extend(&to);
            at += from.len();
        } else {
            result.push(points[at]);
            at += 1;
        }
    }
    PyStr::from_code_points(result).unwrap()
}

pub(super) fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => &[],
    }
}

pub(super) fn object(value: &Value) -> Value {
    let value = decoded(value);
    if matches!(value, Value::Dict(_)) {
        value
    } else {
        dict([])
    }
}

pub(super) fn decoded(value: &Value) -> Value {
    match value {
        Value::Str(s) => s
            .as_str()
            .and_then(|s| scalar::decode(s).ok())
            .unwrap_or(Value::Null),
        _ => value.clone(),
    }
}

pub(super) fn strict_txid(value: &Value) -> Option<PyStr> {
    let value = normalized(value);
    (value.code_points().count() == 64
        && value
            .code_points()
            .all(|p| char::from_u32(p).is_some_and(|c| c.is_ascii_hexdigit())))
    .then_some(value)
}

pub(super) fn chain(value: &Value) -> Option<PyStr> {
    Some(
        match normalized(value).as_str()? {
            "" | "bitcoin" | "btc" | "onchain" => "bitcoin",
            "liquid" | "lbtc" | "elements" => "liquid",
            "lightning" | "ln" => "lightning",
            _ => return None,
        }
        .into(),
    )
}

pub(super) fn network(c: &PyStr, value: &Value) -> Option<PyStr> {
    let value = normalized(value);
    Some(
        if c.is("liquid") {
            match value.as_str()? {
                "" | "liquid" | "liquidv1" | "main" | "mainnet" => "liquidv1",
                "liquidtestnet" | "test" | "testnet" => "liquidtestnet",
                "elements" | "elementsregtest" | "regtest" => "elementsregtest",
                _ => return None,
            }
        } else {
            match value.as_str()? {
                "" | "main" | "mainnet" | "bitcoin" => "main",
                "test" | "testnet" => "test",
                "regtest" => "regtest",
                "signet" => "signet",
                _ => return None,
            }
        }
        .into(),
    )
}

pub(super) fn match_scope(item: &Value) -> Physical {
    let c = field(item, "chain");
    let n = field(item, "network");
    if let Some(c) = chain(c) {
        if let Some(n) = network(&c, n) {
            return (c, n);
        }
    }
    (normalized(c), normalized(n))
}

pub(super) fn onchain_scope(row: &Value) -> Option<Scope> {
    let id = empty_text(field(row, "id"));
    if [
        "custody-tax:",
        "cross-split:",
        "direct-payout:",
        "multi-consol:",
        "owned-derive:",
        "recorded-fanout:",
    ]
    .iter()
    .any(|p| {
        id.code_points()
            .collect::<Vec<_>>()
            .starts_with(&p.chars().map(u32::from).collect::<Vec<_>>())
    }) {
        return None;
    }
    let raw_value = field(row, "raw_json");
    let raw = object(raw_value);
    let config = object(fallback(
        field(row, "config_json"),
        field(row, "wallet_config_json"),
    ));
    let decoded = decoded(raw_value);
    let mut maps = vec![&raw];
    maps.extend(
        list(&decoded)
            .iter()
            .filter(|v| matches!(v, Value::Dict(_))),
    );
    for key in ["tx", "ownership_graph"] {
        if matches!(field(&raw, key), Value::Dict(_)) {
            maps.push(field(&raw, key));
        }
    }
    let mut txids = BTreeSet::new();
    for payload in &maps {
        if truth(&Value::Str(strip(&empty_text(field(payload, "txid"))))) {
            txids.insert(strict_txid(field(payload, "txid"))?);
        }
    }
    if txids.len() > 1 {
        return None;
    }
    let external = strict_txid(field(row, "external_id"));
    let graph = txids.into_iter().next();
    if graph.is_some() && external.is_some() && graph != external {
        return None;
    }
    if graph.is_none() && external.is_some() {
        let kind = normalized(fallback(
            field(row, "external_id_kind"),
            field(&raw, "external_id_kind"),
        ));
        let typed = if kind.code_points().next().is_some() {
            ["txid", "transaction_hash", "onchain_txid"]
                .iter()
                .any(|v| kind.is(v))
        } else {
            maps.iter().any(|m| {
                let Value::Dict(items) = m else {
                    return false;
                };
                items.iter().any(|(k, v)| {
                    let name = lower(&empty_text(&scalar::key_value(k)));
                    let name: String = name
                        .code_points()
                        .filter_map(char::from_u32)
                        .filter(|c| kassiber_custody::journal_support::native_alphanumeric(*c))
                        .collect();
                    [
                        "txid",
                        "txhash",
                        "transactionid",
                        "transactionhash",
                        "onchaintxid",
                        "sendtxid",
                        "lockuptxid",
                        "receivetxid",
                        "claimtxid",
                        "refundtxid",
                    ]
                    .contains(&name.as_str())
                        && strict_txid(v) == external
                })
            })
        };
        if !typed {
            return None;
        }
    }
    let txid = graph.or(external)?;
    let kind = replace(&normalized(field(row, "wallet_kind")), "_", "-");
    if [
        "lnd",
        "coreln",
        "core-ln",
        "core-lightning",
        "cln",
        "lightning",
        "nwc",
        "phoenix",
    ]
    .iter()
    .any(|v| kind.is(v))
        && !maps.iter().any(|m| {
            matches!(field(m, "vin"), Value::List(_)) && matches!(field(m, "vout"), Value::List(_))
        })
    {
        return None;
    }
    let mut chains = BTreeSet::new();
    for payload in maps.iter().copied().chain([&config]) {
        let value = field(payload, "chain");
        if strip(&empty_text(value)).code_points().next().is_some() {
            let c = chain(value)?;
            if c.is("lightning") {
                return None;
            }
            chains.insert(c);
        }
    }
    if chains.len() > 1 {
        return None;
    }
    let asset = asset(field(row, "asset"));
    let c = chains.into_iter().next().or_else(|| {
        if asset.is("BTC") {
            Some("bitcoin".into())
        } else if asset.is("LBTC") {
            Some("liquid".into())
        } else {
            None
        }
    })?;
    if asset.is("BTC") && !c.is("bitcoin") || asset.is("LBTC") && !c.is("liquid") {
        return None;
    }
    let mut networks = BTreeSet::new();
    for payload in maps.iter().copied().chain([&config]) {
        let value = field(payload, "network");
        if strip(&empty_text(value)).code_points().next().is_some() {
            networks.insert(network(&c, value)?);
        }
    }
    if networks.len() > 1 {
        return None;
    }
    let n = networks
        .into_iter()
        .next()
        .or_else(|| c.is("bitcoin").then(|| "main".into()))?;
    let identity = if c.is("liquid") {
        let mut ids = BTreeSet::new();
        let mut displays = BTreeSet::new();
        for payload in &maps {
            for component in [*payload, field(payload, "component")]
                .into_iter()
                .filter(|v| matches!(v, Value::Dict(_)))
            {
                let id = field(component, "asset_id");
                if present(id) {
                    ids.insert(strict_txid(id)?);
                }
                let value = field(component, "asset");
                if present(value) {
                    displays.insert(self::asset(value));
                }
            }
        }
        if ids.len() != 1 || displays.len() > 1 || displays.iter().any(|v| v != &asset) {
            return None;
        }
        let id = ids.into_iter().next()?;
        if !asset.is("LBTC") && asset != upper(&id) {
            return None;
        }
        id
    } else {
        asset
    };
    Some((c, n, txid, identity))
}

pub(super) fn present(value: &Value) -> bool {
    !matches!(value, Value::Null) && !is(value, "")
}

pub(super) fn lookup<'a>(mapping: &'a Value, key: &PyStr) -> &'a Value {
    if let Value::Dict(items) = mapping {
        if let Some((_, value)) = items
            .iter()
            .find(|(k, _)| matches!(k, Key::Str(s) if s == key))
        {
            return value;
        }
    }
    &Value::Null
}

pub(super) fn scripts<'a>(script: &Value, index: &'a Value) -> Result<&'a [Value]> {
    if !truth(script) {
        return Ok(&[]);
    }
    let Value::Str(script) = script else {
        return Err(error(
            "AttributeError",
            format!("'{}' object has no attribute 'lower'", kind(script)),
        ));
    };
    Ok(list(lookup(field(index, "by_script"), &lower(script))))
}

pub(super) fn script_owners(
    script: &Value,
    scope: &Physical,
    index: &Value,
) -> Result<BTreeSet<PyStr>> {
    Ok(scripts(script, index)?
        .iter()
        .filter(|m| match_scope(m) == *scope)
        .map(|m| text(field(m, "wallet_id")))
        .collect())
}

pub(super) fn outpoints<'a>(outpoint: &Value, index: &'a Value) -> &'a [Value] {
    list(lookup(
        field(index, "by_outpoint"),
        &lower(&empty_text(outpoint)),
    ))
}

pub(super) fn ownership_scope(row: &Value, parsed: &Value, index: &Value) -> Result<Option<Scope>> {
    if let Some(scope) = onchain_scope(row) {
        return Ok(Some(scope));
    }
    let raw_id = strip(&empty_text(field(parsed, "txid")));
    let parsed_id = strict_txid(&Value::Str(raw_id.clone()));
    if raw_id.code_points().next().is_some() && parsed_id.is_none() {
        return Ok(None);
    }
    let external = strict_txid(field(row, "external_id"));
    if parsed_id.is_some() && external.is_some() && parsed_id != external {
        return Ok(None);
    }
    let txid = parsed_id.or(external);
    let physical = parsed_scope(parsed, index, row)?;
    let (Some(txid), Some((c, n))) = (txid, physical) else {
        return Ok(None);
    };
    let component = field(parsed, "component");
    let asset_id = normalized(field(component, "asset_id"));
    let display = asset(fallback(field(component, "asset"), field(row, "asset")));
    if display.is("BTC") && !c.is("bitcoin") || display.is("LBTC") && !c.is("liquid") {
        return Ok(None);
    }
    let identity = if c.is("liquid") {
        if strict_txid(&Value::Str(asset_id.clone())).is_none() {
            return Ok(None);
        }
        asset_id
    } else {
        display
    };
    Ok(Some((c, n, txid, identity)))
}

pub(super) fn parsed_scope(parsed: &Value, index: &Value, row: &Value) -> Result<Option<Physical>> {
    let wallet = empty_text(field(row, "wallet_id"));
    let mut inferred = BTreeSet::new();
    for entry in list(field(parsed, "inputs")) {
        let mut matches = if truth(field(entry, "outpoint")) {
            outpoints(field(entry, "outpoint"), index)
        } else {
            &[]
        };
        if matches.is_empty() {
            matches = scripts(field(entry, "script"), index)?;
        }
        for m in matches
            .iter()
            .filter(|m| text(field(m, "wallet_id")) == wallet)
        {
            inferred.insert(match_scope(m));
        }
    }
    if inferred.len() != 1 {
        inferred.clear();
        for output in list(field(parsed, "outputs")) {
            for m in scripts(field(output, "script"), index)?
                .iter()
                .filter(|m| text(field(m, "wallet_id")) == wallet)
            {
                inferred.insert(match_scope(m));
            }
        }
    }
    let inferred = if inferred.len() == 1 {
        inferred.into_iter().next()
    } else {
        None
    };
    let config = object(fallback(
        field(row, "config_json"),
        field(row, "wallet_config_json"),
    ));
    let mut chains = BTreeSet::new();
    for value in [field(parsed, "chain"), field(&config, "chain")] {
        if strip(&empty_text(value)).code_points().next().is_some() {
            let Some(c) = chain(value) else {
                return Ok(None);
            };
            chains.insert(c);
        }
    }
    if chains.len() > 1 {
        return Ok(None);
    }
    let explicit_chain = chains.into_iter().next();
    let network_chain = explicit_chain
        .clone()
        .or_else(|| inferred.as_ref().map(|s| s.0.clone()))
        .unwrap_or_else(|| {
            if asset(field(row, "asset")).is("LBTC") {
                "liquid"
            } else {
                "bitcoin"
            }
            .into()
        });
    let mut networks = BTreeSet::new();
    for value in [field(parsed, "network"), field(&config, "network")] {
        if strip(&empty_text(value)).code_points().next().is_some() {
            let Some(n) = network(&network_chain, value) else {
                return Ok(None);
            };
            networks.insert(n);
        }
    }
    if networks.len() > 1 {
        return Ok(None);
    }
    let explicit_network = networks.into_iter().next();
    if let Some(inferred) = inferred {
        if explicit_chain.as_ref().is_some_and(|v| v != &inferred.0)
            || explicit_network.as_ref().is_some_and(|v| v != &inferred.1)
        {
            return Ok(None);
        }
        return Ok(Some(inferred));
    }
    if let Some(c) = explicit_chain {
        if let Some(n) = explicit_network {
            return Ok(Some((c, n)));
        }
        if c.is("bitcoin") {
            return Ok(Some((c, "main".into())));
        }
    }
    Ok(None)
}
