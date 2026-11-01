use super::*;
use crate::{evidence, graph};

pub(super) fn rail(v: &Value) -> String {
    let r = text(v).py_strip().py_lower().replace('_', "-");
    match r.as_str() {
        "btc" | "onchain" => "bitcoin".into(),
        "elements" | "lbtc" => "liquid".into(),
        "cln" | "coreln" | "lnd" | "nwc" => "lightning".into(),
        _ => r,
    }
}
pub(super) fn kind(v: &Value) -> String {
    let k = text(v).py_strip().py_lower().replace('_', "-");
    if matches!(k.as_str(), "core-ln" | "core-lightning") {
        "coreln".into()
    } else {
        k
    }
}
pub(super) fn asset(v: &Value) -> String {
    let a = text(v).py_strip().to_owned();
    let lower = a.py_lower();
    if lower.len() == 64 && lower.bytes().all(|b| b.is_ascii_hexdigit()) {
        lower
    } else if matches!(lower.as_str(), "l-btc" | "lbtc") {
        "LBTC".into()
    } else {
        a.py_upper()
    }
}
fn canonical_chain(v: &Value) -> Option<String> {
    let c = text(v).py_strip().py_lower();
    match c.as_str() {
        "" | "bitcoin" | "btc" | "onchain" => Some("bitcoin".into()),
        "liquid" | "lbtc" | "elements" => Some("liquid".into()),
        _ => None,
    }
}
fn chain_or_none(v: &Value) -> Option<String> {
    if absent(v) {
        None
    } else {
        canonical_chain(v)
    }
}
fn network_or_none(c: Option<&str>, v: &Value) -> Option<String> {
    if absent(v) {
        None
    } else {
        evidence::network(c?, v).ok()
    }
}
pub(super) fn network_scope(leg: &Value) -> Value {
    let r = rail(&leg["rail"]);
    let expected = match r.as_str() {
        "bitcoin" | "lightning" => Some("bitcoin"),
        "liquid" => Some("liquid"),
        _ => None,
    };
    let mut c = None;
    if !absent(&leg["chain"]) {
        c = canonical_chain(&leg["chain"]);
        if c.is_none() {
            if expected.is_some() {
                return json!({"valid":false,"rail":r,"chain":leg["chain"],"network":leg["network"],"domain":null,"reason":"unsupported_chain"});
            }
            c = Some(text(&leg["chain"]).py_strip().py_lower());
        }
    }
    if c.is_some() && expected.is_some() && c.as_deref() != expected {
        return json!({"valid":false,"rail":r,"chain":c,"network":leg["network"],"domain":null,"reason":"rail_chain_mismatch"});
    }
    let c = c.or_else(|| expected.map(str::to_owned));
    if absent(&leg["network"]) {
        return json!({"valid":true,"rail":r,"chain":c,"network":null,"domain":null});
    }
    let n = pystr(&leg["network"]).py_strip().py_lower();
    if matches!(c.as_deref(), Some("bitcoin" | "liquid")) {
        let Ok(n) = evidence::network(c.as_deref().unwrap(), &json!(n)) else {
            return json!({"valid":false,"rail":r,"chain":c,"network":leg["network"],"domain":null,"reason":"network_invalid_for_chain"});
        };
        let d = match n.as_str() {
            "main" | "liquidv1" => Some("main"),
            "test" | "liquidtestnet" => Some("test"),
            "regtest" | "elementsregtest" => Some("regtest"),
            "signet" => Some("signet"),
            _ => None,
        };
        return json!({"valid":true,"rail":r,"chain":c,"network":n,"domain":d});
    }
    let d = match n.as_str() {
        "bitcoin" | "liquid" | "liquidv1" | "main" | "mainnet" => Some("main"),
        "test" | "testnet" | "liquidtestnet" => Some("test"),
        "elements" | "elementsregtest" | "regtest" => Some("regtest"),
        "signet" => Some("signet"),
        _ => None,
    };
    json!({"valid":true,"rail":r,"chain":c,"network":n,"domain":d})
}
pub(super) fn expected_scope(row: &Value) -> (Option<String>, Option<String>, Option<String>) {
    if let Some((c, n, _, _)) = graph::onchain_scope(row) {
        return (Some(c.clone()), Some(c), Some(n));
    }
    let config = codec::object(&row["config_json"]);
    let c = chain_or_none(&config["chain"]);
    let k = kind(&row["wallet_kind"]);
    let a = asset(&row["asset"]);
    let (r, c) = if evidence::lightning(&k) {
        (
            Some("lightning".into()),
            c.or_else(|| Some("bitcoin".into())),
        )
    } else if c.as_deref() == Some("liquid")
        || matches!(k.as_str(), "elements" | "liquid")
        || k.contains("liquid")
        || a == "LBTC"
    {
        (Some("liquid".into()), Some("liquid".into()))
    } else if matches!(
        k.as_str(),
        "address" | "descriptor" | "samourai" | "silent-payment" | "wasabi" | "xpub"
    ) || c.as_deref() == Some("bitcoin")
    {
        (Some("bitcoin".into()), Some("bitcoin".into()))
    } else {
        (c.clone(), c)
    };
    let n = network_or_none(c.as_deref(), &config["network"]);
    (r, c, n)
}
pub(super) fn anchor_scope_issues(leg: &Value, row: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    let (r, c, n) = expected_scope(row);
    let authored = rail(&leg["rail"]);
    if r.is_some() && Some(authored.as_str()) != r.as_deref() {
        issues.push(json!({"code":"anchor_rail_mismatch","leg_id":leg["id"],"transaction_id":text(&row["id"]),"leg_rail":authored,"transaction_rail":r}));
    }
    if c.is_some() && !absent(&leg["chain"]) && chain_or_none(&leg["chain"]) != c {
        issues.push(json!({"code":"anchor_chain_mismatch","leg_id":leg["id"],"transaction_id":text(&row["id"]),"leg_chain":leg["chain"],"transaction_chain":c}));
    }
    if n.is_some()
        && !absent(&leg["network"])
        && network_or_none(c.as_deref(), &leg["network"]) != n
    {
        issues.push(json!({"code":"anchor_network_mismatch","leg_id":leg["id"],"transaction_id":text(&row["id"]),"leg_network":leg["network"],"transaction_network":n}));
    }
    issues
}
pub(super) fn scoped_legs(snapshot: &Snapshot, legs: &[Value]) -> Result<Vec<Value>, Error> {
    legs.iter().map(|leg| {
        let tx=&leg["transaction_id"];
        let wallet=snapshot.anchor_wallets.iter().rev().find(|w|w.get("id")==leg.get("wallet_id"));
        let row = if !absent(tx) {snapshot.anchors.get(&pystr(tx)).filter(|r|!r.is_null()).cloned()} else {None};
        let row=row.or_else(||wallet.map(|w|json!({"id":format!("wallet:{}",text(&w["id"])),"wallet_kind":w["kind"],"config_json":w["config_json"],"asset":leg["asset"],"raw_json":"{}","external_id":null})));
        let mut result=leg.clone();
        if let Some(row)=row {
            let (r,c,n)=expected_scope(&row);
            for (key,v) in [("rail",r),("chain",c),("network",n)] {if absent(&result[key]) {if let Some(v)=v {result[key]=json!(v);}}}
            let canonical=&row["occurred_at"];
            if !absent(tx) && !absent(canonical) && !(!absent(&result["occurred_at"]) && result["occurred_at"]!=*canonical && evidence::current_observation(&row)?) {result["occurred_at"]=canonical.clone();}
        }
        Ok(result)
    }).collect()
}

pub(super) fn inferred_pairs<'a>(legs: &'a [Value], mode: &str) -> Vec<(&'a Value, &'a Value)> {
    let sources: Vec<_> = legs
        .iter()
        .filter(|l| l["role"] == "source" && big(&l["amount_msat"]) > BigInt::from(0))
        .collect();
    let sinks: Vec<_> = legs
        .iter()
        .filter(|l| sink(l) && big(&l["amount_msat"]) > BigInt::from(0))
        .collect();
    if mode == "conversion" {
        return if sources.len() == 1 && sinks.len() == 1 {
            vec![(sources[0], sinks[0])]
        } else {
            Vec::new()
        };
    }
    if sources.len() == 1 {
        return sinks.into_iter().map(|s| (sources[0], s)).collect();
    }
    let owned: Vec<_> = sinks.iter().filter(|s| owned(s)).collect();
    if owned.len() == 1
        && sinks
            .iter()
            .all(|s| !matches!(s["role"].as_str(), Some("fee" | "external" | "suspense")))
    {
        sources.iter().map(|s| (*s, *owned[0])).collect()
    } else {
        Vec::new()
    }
}
pub(super) fn pairs<'a>(
    legs: &'a [Value],
    allocs: &[Value],
    mode: &str,
) -> Vec<(Value, &'a Value, &'a Value, BigInt, BigInt)> {
    if allocs.is_empty() {
        return inferred_pairs(legs, mode)
            .into_iter()
            .map(|(s, t)| {
                (
                    Value::Null,
                    s,
                    t,
                    big(&s["amount_msat"]),
                    big(&t["amount_msat"]),
                )
            })
            .collect();
    }
    allocs
        .iter()
        .filter_map(|a| {
            Some((
                a["id"].clone(),
                by_id(legs, &a["source_leg_id"])?,
                by_id(legs, &a["sink_leg_id"])?,
                big(&a["source_amount_msat"]),
                big(&a["sink_amount_msat"]),
            ))
        })
        .collect()
}
pub(super) fn allocation_scope_issues(legs: &[Value], allocs: &[Value], mode: &str) -> Vec<Value> {
    if mode != "quantity" {
        return Vec::new();
    }
    pairs(legs,allocs,mode).into_iter().filter_map(|(id,s,t,_,_)| {
        let ss=network_scope(s);let ts=network_scope(t);
        if !truth(&ss["valid"]) || !truth(&ts["valid"]) {Some(json!({"code":"allocation_network_scope_invalid","allocation_id":id,"source_leg_id":s["id"],"sink_leg_id":t["id"],"source_scope":ss,"sink_scope":ts}))}
        else if !ss["domain"].is_null() && !ts["domain"].is_null() && ss["domain"]!=ts["domain"] {Some(json!({"code":"allocation_network_mismatch","allocation_id":id,"source_leg_id":s["id"],"sink_leg_id":t["id"],"source_network_domain":ss["domain"],"sink_network_domain":ts["domain"],"source_scope":ss,"sink_scope":ts}))} else {None}
    }).collect()
}
pub(super) fn chronology(
    legs: &[Value],
    allocs: &[Value],
    mode: &str,
) -> Result<Vec<Value>, Error> {
    let mut issues = Vec::new();
    for (id, s, t, sa, ta) in pairs(legs, allocs, mode) {
        if sa <= BigInt::from(0) && ta <= BigInt::from(0) {
            continue;
        }
        let source = time::instant(&s["occurred_at"])?;
        let target = time::instant(&t["occurred_at"])?;
        let (Some(source), Some(target)) = (source, target) else {
            continue;
        };
        if source <= time::skewed(target)? {
            continue;
        }
        issues.push(json!({"code":"allocation_chronology_mismatch","message":"a custody allocation destination predates its source","allocation_id":id,"source_leg_id":s["id"],"sink_leg_id":t["id"],"source_occurred_at":s["occurred_at"],"sink_occurred_at":t["occurred_at"]}));
    }
    Ok(issues)
}
pub(super) fn location_issues(legs: &[Value]) -> Result<Vec<Value>, Error> {
    let mut groups = group(
        legs.iter()
            .filter(|l| {
                big(&l["amount_msat"]) > BigInt::from(0)
                    && truth(&l["wallet_id"])
                    && (owned(l) || l["role"] == "source")
            })
            .cloned(),
        location,
    );
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let mut issues = Vec::new();
    for ((wallet, exposure, unit), legs) in groups {
        let sources: Vec<_> = legs.iter().filter(|l| l["role"] == "source").collect();
        let sinks: Vec<_> = legs.iter().filter(|l| owned(l)).collect();
        if sources.is_empty() || sinks.is_empty() {
            continue;
        }
        let parsed_sinks: Vec<_> = sinks
            .iter()
            .map(|l| Ok((time::instant(&l["occurred_at"])?, *l)))
            .collect::<Result<_, Error>>()?;
        let parsed_sources: Vec<_> = sources
            .iter()
            .map(|l| Ok((time::instant(&l["occurred_at"])?, *l)))
            .collect::<Result<_, Error>>()?;
        if parsed_sinks
            .iter()
            .chain(&parsed_sources)
            .any(|(when, _)| when.is_none())
        {
            continue;
        }
        let mut events: Vec<_> = parsed_sinks
            .iter()
            .map(|(when, l)| (when.unwrap(), 0, *l))
            .collect();
        for (when, s) in parsed_sources {
            let when = when.unwrap();
            let prior = parsed_sinks.iter().any(|(sink, _)| sink.unwrap() < when);
            if s["transaction_id"].is_null() || prior {
                events.push((
                    if s["transaction_id"].is_null() {
                        time::skewed(when)?
                    } else {
                        when
                    },
                    1,
                    s,
                ));
            }
        }
        events.sort_by_key(|(when, kind, l)| (*when, *kind, wide(&l["ordinal"])));
        let mut available = BigInt::from(0);
        for (_, kind, l) in events {
            let amount = big(&l["amount_msat"]);
            if kind == 0 {
                available += amount;
            } else if amount > available {
                issues.push(json!({"code":"custody_location_continuity_mismatch","message":"an intermediate custody location spends before or beyond the value this component credited to it","wallet_id":wallet,"exposure":exposure,"conservation_unit":unit,"source_leg_id":l["id"],"available_msat":number(&available),"source_msat":number(&amount)}));
                break;
            } else {
                available -= amount;
            }
        }
    }
    Ok(issues)
}
pub(super) fn connectivity(legs: &[Value], allocs: &[Value], mode: &str) -> Vec<Value> {
    if mode != "quantity" {
        return Vec::new();
    }
    let material: Vec<_> = legs
        .iter()
        .filter(|l| big(&l["amount_msat"]) > BigInt::from(0))
        .cloned()
        .collect();
    // Dicts replace duplicate ids without changing their insertion slot.
    let mut unique: Vec<&Value> = Vec::new();
    let mut positions = HashMap::new();
    let mut ids = Vec::new();
    for l in &material {
        let id = pystr(&l["id"]);
        if let Some(&at) = positions.get(&id) {
            unique[at] = l;
        } else {
            positions.insert(id.clone(), unique.len());
            ids.push(id);
            unique.push(l);
        }
    }
    let mut parent: Vec<usize> = (0..unique.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    let mut edges: Vec<(String, String)> = pairs(&material, allocs, mode)
        .into_iter()
        .filter(|(_, _, _, s, t)| *s > BigInt::from(0) || *t > BigInt::from(0))
        .map(|(_, s, t, _, _)| (pystr(&s["id"]), pystr(&t["id"])))
        .collect();
    for (_, items) in group(
        material
            .iter()
            .filter(|l| !absent(&l["wallet_id"]) && (owned(l) || l["role"] == "source"))
            .cloned(),
        location,
    ) {
        for l in items.iter().skip(1) {
            edges.push((pystr(&items[0]["id"]), pystr(&l["id"])));
        }
    }
    for (a, b) in edges {
        if let (Some(a), Some(b)) = (positions.get(&a).copied(), positions.get(&b).copied()) {
            let a = root(&mut parent, a);
            let b = root(&mut parent, b);
            if a != b {
                let (small, large) = if ids[a] < ids[b] { (a, b) } else { (b, a) };
                parent[large] = small;
            }
        }
    }
    let mut groups: Vec<(usize, Vec<Value>)> = Vec::new();
    let mut index = std::collections::HashMap::new();
    for (i, l) in unique.iter().enumerate() {
        let r = root(&mut parent, i);
        crate::codec::push_grouped(&mut groups, &mut index, r, (*l).clone());
    }
    let mut issues = Vec::new();
    for (_, ls) in groups {
        let mut domains: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for l in &ls {
            let scope = network_scope(l);
            if truth(&scope["valid"]) && !scope["domain"].is_null() {
                domains
                    .entry(pystr(&scope["domain"]))
                    .or_default()
                    .push(pystr(&l["id"]));
            }
        }
        if domains.len() <= 1 {
            continue;
        }
        for ids in domains.values_mut() {
            ids.sort();
        }
        let ids: BTreeSet<_> = ls
            .iter()
            .filter(|l| !absent(&l["component_id"]))
            .map(|l| pystr(&l["component_id"]))
            .collect();
        issues.push(json!({"code":"custody_network_scope_laundering","message":"unknown custody legs cannot connect incompatible known Bitcoin network domains","network_domains":domains.keys().collect::<Vec<_>>(),"leg_ids_by_domain":domains,"component_ids":ids}));
    }
    issues
}
pub(super) fn complete_allocations(legs: &[Value], allocs: &[Value]) -> Vec<Value> {
    let mut groups = group(legs.iter().cloned(), |l| {
        if truth(&l["component_id"]) {
            text(&l["component_id"])
        } else {
            "__single__".into()
        }
    });
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    let mut result = Vec::new();
    for (id, ls) in groups {
        let explicit: Vec<_> = allocs
            .iter()
            .filter(|a| {
                if truth(&a["component_id"]) {
                    text(&a["component_id"]) == id
                } else {
                    id == "__single__"
                }
            })
            .cloned()
            .collect();
        if !explicit.is_empty() {
            result.extend(explicit);
            continue;
        }
        for (i, (s, t)) in inferred_pairs(&ls, "quantity").into_iter().enumerate() {
            result.push(json!({"id":format!("inferred:{id}:{i}"),"component_id":id,"source_leg_id":s["id"],"sink_leg_id":t["id"],"source_amount_msat":s["amount_msat"],"sink_amount_msat":t["amount_msat"]}));
        }
    }
    result
}
pub(super) fn profile_routes(
    snapshot: &Snapshot,
    legs: &[Value],
    allocs: &[Value],
) -> Result<Vec<Value>, Error> {
    let ls = scoped_legs(snapshot, legs)?;
    let complete = complete_allocations(&ls, allocs);
    let mut issues = connectivity(&ls, &complete, "quantity");
    let untracked: BTreeSet<_> = snapshot
        .anchor_wallets
        .iter()
        .filter(|w| kind(&w["kind"]) == "untracked")
        .map(|w| text(&w["id"]))
        .collect();
    for (_, ls) in group(
        ls.into_iter()
            .filter(|l| untracked.contains(&text(&l["wallet_id"]))),
        location,
    ) {
        let ids: BTreeSet<_> = ls
            .iter()
            .filter(|l| !absent(&l["component_id"]))
            .map(|l| pystr(&l["component_id"]))
            .collect();
        if ids.len() < 2 {
            continue;
        }
        for mut issue in location_issues(&ls)? {
            issue["component_ids"] = json!(ids);
            issue["message"]=json!("a reused untracked custody location spends before or beyond value credited by the connected active route");
            issues.push(issue);
        }
    }
    Ok(issues)
}
