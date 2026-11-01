//! Pure presentation of an already reviewed, disclosure-filtered report.
//! This module reads no book and grants no ownership or accounting authority.

use super::*;
use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn list<'a>(row: &'a Value, key: &str) -> &'a [Value] {
    match value(row, key) {
        Value::List(items) => items,
        _ => &[],
    }
}
fn fallback(row: &Value, key: &str) -> String {
    get(row, key)
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default()
}
fn default_text(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or_else(|| s(""))
}
fn count(n: usize) -> Value {
    Value::Int(n.into())
}
fn strings(items: impl IntoIterator<Item = String>) -> Value {
    Value::List(items.into_iter().map(s).collect())
}
fn event_time(node: &Value) -> String {
    let occurred = fallback(node, "occurred_at");
    if occurred.is_empty() {
        fallback(node, "acquired_at")
    } else {
        occurred
    }
}
fn sort_nodes(nodes: &mut [&Value]) {
    nodes.sort_by_key(|node| {
        (
            text(node, "node_type") == "source",
            event_time(node),
            fallback(node, "label"),
        )
    });
}
fn number(v: &Value) -> Result<f64> {
    match v {
        Value::Float(n) => Ok(*n),
        Value::Bool(v) => Ok(if *v { 1.0 } else { 0.0 }),
        Value::Int(n) => n
            .to_string()
            .parse()
            .map_err(|_| AppError::new("internal_error", "int too large to convert to float")),
        _ => kassiber_pytext::parse_float(&py_str(v))
            .map_err(|_| AppError::new("internal_error", "invalid float in source-funds report")),
    }
}
fn rounded(n: f64, places: usize) -> f64 {
    if n.is_finite() {
        format!("{n:.places$}").parse().expect("formatted float")
    } else {
        n
    }
}
pub(super) fn integer(v: &Value) -> Result<BigInt> {
    match v {
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(v) => Ok(i32::from(*v).into()),
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc()))
        }
        _ => kassiber_pytext::parse_int(&py_str(v)),
    }
    .map_err(|_| AppError::new("internal_error", "invalid integer in source-funds report"))
}
fn zero_integer(row: &Value, key: &str) -> Result<BigInt> {
    if truthy(value(row, key)) {
        integer(value(row, key))
    } else {
        Ok(0.into())
    }
}
pub(super) fn amount(msat: &BigInt) -> Result<Value> {
    let decimal: Decimal = msat
        .to_string()
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid amount"))?;
    float(
        &decimal
            .div(&Decimal::from(100_000_000_000i64), &DECIMAL)
            .map_err(arithmetic)?,
    )
}
fn label(row: &Value, key: &str) -> String {
    fallback(row, key).replace('_', " ")
}
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}
pub(super) fn normalized_asset(v: &Value) -> String {
    let raw = if truthy(v) { py_str(v) } else { String::new() };
    let raw = kassiber_pytext::strip(&raw);
    let lowered = kassiber_pytext::lower(raw);
    if lowered.len() == 64 && lowered.bytes().all(|b| b.is_ascii_hexdigit()) {
        lowered
    } else if ["l-btc", "lbtc"].contains(&lowered.as_str()) {
        "LBTC".into()
    } else {
        kassiber_pytext::upper(raw)
    }
}
pub(super) fn ratio(numerator: &BigInt, denominator: &BigInt) -> Result<f64> {
    let zero = BigInt::from(0);
    if *denominator == zero {
        return Err(AppError::new("internal_error", "division by zero"));
    }
    let negative = (*numerator < zero) != (*denominator < zero);
    let sign = if negative { 1u64 << 63 } else { 0 };
    let numerator = if *numerator < zero {
        -numerator
    } else {
        numerator.clone()
    };
    let denominator = if *denominator < zero {
        -denominator
    } else {
        denominator.clone()
    };
    if numerator == zero {
        return Ok(f64::from_bits(sign));
    }
    let mut exponent = numerator.bits() as i64 - denominator.bits() as i64;
    if if exponent >= 0 {
        numerator < (&denominator << exponent as usize)
    } else {
        (&numerator << (-exponent) as usize) < denominator
    } {
        exponent -= 1;
    }
    if exponent > 1023 {
        return Err(AppError::new(
            "internal_error",
            "integer division result too large for a float",
        ));
    }
    if exponent < -1075 {
        return Ok(f64::from_bits(sign));
    }
    exponent = exponent.max(-1022);
    let shift = 52 - exponent;
    let (numerator, denominator) = if shift >= 0 {
        (numerator << shift as usize, denominator)
    } else {
        (numerator, denominator << (-shift) as usize)
    };
    let mut quotient = &numerator / &denominator;
    let remainder = numerator % &denominator;
    let twice = remainder << 1usize;
    if twice > denominator || twice == denominator && (&quotient & BigInt::from(1)) != zero {
        quotient += 1;
    }
    let mut significand = u64::try_from(quotient).expect("53-bit rounded quotient");
    if significand >= 1u64 << 53 {
        significand >>= 1;
        exponent += 1;
    }
    if exponent > 1023 {
        return Err(AppError::new(
            "internal_error",
            "integer division result too large for a float",
        ));
    }
    let bits = if significand < 1u64 << 52 {
        significand
    } else {
        ((exponent + 1023) as u64) << 52 | (significand - (1u64 << 52))
    };
    Ok(f64::from_bits(sign | bits))
}
fn privacy(edge: &Value) -> bool {
    matches!(text(edge, "link_type").as_str(), "coinjoin" | "payjoin")
}

/// Add overview, flow models, rollups and narrative to a finished report.
/// The caller owns the read snapshot and allocation/gate decisions.
/// Tied simplified-flow presentation keys remain an explicit unported boundary:
/// Python orders them by its process-randomized set rather than a stable key.
/// Failure leaves the envelope untouched.
pub fn add_shape(envelope: &mut Value) -> Result<()> {
    let graph = value(envelope, "graph");
    let nodes = list(graph, "nodes");
    let edges = list(graph, "edges");
    let tx_count = nodes
        .iter()
        .filter(|n| text(n, "node_type") == "transaction")
        .count();
    let source_count = nodes
        .iter()
        .filter(|n| text(n, "node_type") == "source")
        .count();
    let mut dates: Vec<_> = nodes
        .iter()
        .map(event_time)
        .filter(|s| !s.is_empty())
        .collect();
    dates.sort();
    let target = value(envelope, "target");
    let target_asset = get(target, "asset")
        .filter(|v| truthy(v))
        .or_else(|| get(value(envelope, "allocations"), "asset"))
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default();
    let data_sources = data_sources(nodes);
    let provenance = data_provenance(nodes);
    let source_mix = list(envelope, "source_mix");
    let categories: BTreeSet<_> = source_mix
        .iter()
        .map(|row| fallback(row, "source_type"))
        .collect();
    let blocker_count = list(value(envelope, "explain_gates"), "blockers").len();
    let warning_count = list(value(envelope, "explain_gates"), "warnings").len();
    let start = dates.first().cloned().unwrap_or_default();
    let end = dates.last().cloned().unwrap_or_default();
    let overview = dict([
        ("target_label", default_text(target, "label")),
        ("target_asset", s(target_asset.as_str())),
        ("target_amount", value(target, "required_amount").clone()),
        (
            "target_amount_msat",
            value(target, "required_amount_msat").clone(),
        ),
        ("target_fiat_value", value(target, "fiat_value").clone()),
        (
            "target_fiat_currency",
            default_text(target, "fiat_currency"),
        ),
        ("target_date", default_text(target, "occurred_at")),
        ("target_wallet", default_text(target, "wallet")),
        (
            "time_range",
            dict([("start", s(start.as_str())), ("end", s(end.as_str()))]),
        ),
        ("transaction_count", count(tx_count)),
        ("link_count", count(edges.len())),
        ("root_source_count", count(source_count)),
        ("source_category_count", count(categories.len())),
        ("data_source_count", count(data_sources.len())),
        ("blocker_count", count(blocker_count)),
        ("warning_count", count(warning_count)),
    ]);
    let levels = flow_levels(envelope)?;
    let simplified = simplified_flow(envelope)?;
    let explanation = route_difference_explanation(envelope)?;
    let allocations = value(envelope, "allocations");
    let gross_phrase = gross_requirement_phrase(allocations)?;
    let target_amount = if truthy(value(target, "required_amount")) {
        number(value(target, "required_amount"))?
    } else {
        0.0
    };
    let mut gross_sentence = String::new();
    if !gross_phrase.is_empty() && !truthy(value(allocations, "gross_matches_target")) {
        gross_sentence = format!("Selected target: {target_amount:.8} {target_asset}. Gross upstream requirement: {gross_phrase}. ");
        if let Some(explanation) = &explanation {
            let hops = int(explanation, "hop_count") as usize;
            let fee = number(value(explanation, "fee"))?;
            gross_sentence.push_str(&format!("That is {target_amount:.8} {target_asset} plus {fee:.8} {target_asset} of network fees recorded on {hops} disclosed transaction{}. ", plural(hops)));
        } else if !matches!(value(allocations, "route_difference"), Value::Null) {
            let difference = number(value(allocations, "route_difference"))?;
            if difference != 0.0 {
                gross_sentence.push_str(&format!("The route difference of {:.8} {target_asset} is not automatically classified as a fee. ", difference.abs()));
            }
        }
    }
    let edge_count = edges.len();
    let ds_count = data_sources.len();
    let target_label = get(target, "label")
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_else(|| "the selected target".into());
    let mix_phrase = source_mix_phrase(source_mix, &target_asset)?;
    let start_label = if start.is_empty() {
        "unknown start"
    } else {
        &start
    };
    let end_label = if end.is_empty() { "unknown end" } else { &end };
    let narrative = dict([
        ("generated_by", s("local_rule_summary")),
        ("paragraphs", strings([
            format!("This report traces {target_amount:.8} {target_asset} for {target_label} through {edge_count} reviewed link{} across {tx_count} transaction{}.", plural(edge_count), plural(tx_count)),
            format!("The reviewed source mix is {mix_phrase}. {gross_sentence}The path spans {start_label} to {end_label} and uses {ds_count} local data source{}.", plural(ds_count)),
            if blocker_count == 0 { "Export is currently clear under the current review gates.".into() } else { format!("Export is blocked by {blocker_count} unresolved review gate{}.", plural(blocker_count)) },
        ])),
    ]);
    replace(envelope, "overview", overview);
    replace(envelope, "data_sources", Value::List(data_sources));
    replace(envelope, "data_provenance_summary", Value::List(provenance));
    replace(envelope, "flow_levels", Value::List(levels));
    replace(envelope, "simplified_flow", simplified);
    if let Some(explanation) = explanation {
        let mut allocations = value(envelope, "allocations").clone();
        replace(
            &mut allocations,
            "route_difference_explanation",
            explanation,
        );
        replace(envelope, "allocations", allocations);
    }
    replace(envelope, "narrative", narrative);
    Ok(())
}

fn data_sources(nodes: &[Value]) -> Vec<Value> {
    struct Group {
        row: Value,
        assets: BTreeSet<String>,
        start: String,
        end: String,
        tx: usize,
        source: usize,
    }
    let mut groups: BTreeMap<(String, String), Group> = BTreeMap::new();
    for node in nodes {
        let tx = text(node, "node_type") == "transaction";
        let (name, kind, provenance) = if tx {
            let name = fallback(node, "wallet");
            (
                if name.is_empty() {
                    "Unlabeled wallet".into()
                } else {
                    name
                },
                "wallet".into(),
                fallback(node, "data_provenance"),
            )
        } else {
            let mut name = fallback(node, "label");
            if name.is_empty() {
                name = label(node, "source_type");
            }
            if name.is_empty() {
                name = "Reviewed source".into();
            }
            let kind = fallback(node, "source_type");
            (
                name,
                if kind.is_empty() {
                    "source".into()
                } else {
                    kind
                },
                "attested_source".into(),
            )
        };
        let group = groups
            .entry((kind.clone(), name.clone()))
            .or_insert_with(|| Group {
                row: dict([
                    ("label", s(name)),
                    ("kind", s(kind)),
                    ("provenance", s(provenance)),
                    ("transaction_count", count(0)),
                    ("source_count", count(0)),
                    ("assets", Value::List(vec![])),
                    ("first_seen", s("")),
                    ("last_seen", s("")),
                ]),
                assets: BTreeSet::new(),
                start: String::new(),
                end: String::new(),
                tx: 0,
                source: 0,
            });
        if tx {
            group.tx += 1;
        } else {
            group.source += 1;
        }
        if truthy(value(node, "asset")) {
            group.assets.insert(text(node, "asset"));
        }
        let time = event_time(node);
        if !time.is_empty() {
            if group.start.is_empty() || time < group.start {
                group.start = time.clone();
            }
            if group.end.is_empty() || time > group.end {
                group.end = time;
            }
        }
    }
    groups
        .into_values()
        .map(|mut group| {
            replace(&mut group.row, "transaction_count", count(group.tx));
            replace(&mut group.row, "source_count", count(group.source));
            replace(&mut group.row, "assets", strings(group.assets));
            replace(&mut group.row, "first_seen", s(group.start));
            replace(&mut group.row, "last_seen", s(group.end));
            group.row
        })
        .collect()
}

fn data_provenance(nodes: &[Value]) -> Vec<Value> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for node in nodes
        .iter()
        .filter(|node| text(node, "node_type") == "transaction")
    {
        let key = fallback(node, "data_provenance");
        *counts
            .entry(if key.is_empty() {
                "manual_import".into()
            } else {
                key
            })
            .or_default() += 1;
    }
    let total: usize = counts.values().sum();
    [
        ("chain_sync", "Blockchain (watch-only sync)"),
        ("platform_export", "Platform export / API"),
        ("manual_import", "Manual / custom import"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        counts.get(key).map(|&n| {
            dict([
                ("provenance", s(key)),
                ("label", s(label)),
                ("count", count(n)),
                (
                    "percent",
                    Value::Float(rounded(n as f64 / total as f64 * 100.0, 2)),
                ),
            ])
        })
    })
    .collect()
}

fn allocated_fiat(node: &Value) -> Result<Value> {
    let fiat = value(node, "fiat_value");
    if matches!(fiat, Value::Null) {
        return Ok(Value::Null);
    }
    let fiat = number(fiat)?;
    let amount = value(node, "amount_msat");
    let required = value(node, "required_amount_msat");
    let result = if !truthy(amount) || matches!(required, Value::Null) {
        fiat
    } else {
        fiat * ratio(&integer(required)?, &integer(amount)?)?
    };
    Ok(Value::Float(rounded(result, 2)))
}
fn compact(node: &Value) -> Result<Value> {
    let mut row = dict(
        [
            "id",
            "node_type",
            "transaction_id",
            "label",
            "wallet",
            "source_type",
            "direction",
            "asset",
        ]
        .map(|key| (key, default_text(node, key))),
    );
    for key in [
        "required_amount",
        "required_amount_msat",
        "amount",
        "amount_msat",
        "fee",
        "fee_msat",
    ] {
        replace(&mut row, key, value(node, key).clone());
    }
    for key in ["occurred_at", "acquired_at", "external_id", "fiat_currency"] {
        replace(&mut row, key, default_text(node, key));
    }
    replace(&mut row, "fiat_value", value(node, "fiat_value").clone());
    replace(&mut row, "fiat_value_allocated", allocated_fiat(node)?);
    for key in ["data_provenance", "review_state"] {
        replace(&mut row, key, default_text(node, key));
    }
    Ok(row)
}

fn flow_levels(report: &Value) -> Result<Vec<Value>> {
    let graph = value(report, "graph");
    let nodes: BTreeMap<_, _> = list(graph, "nodes")
        .iter()
        .filter(|n| truthy(value(n, "id")))
        .map(|n| (text(n, "id"), n))
        .collect();
    let target = fallback(value(report, "target"), "id");
    if target.is_empty() {
        return Ok(vec![]);
    }
    let mut parents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in list(graph, "edges") {
        let parent = fallback(edge, "from");
        let child = fallback(edge, "to");
        if !parent.is_empty() && !child.is_empty() {
            parents.entry(child).or_default().push(parent);
        }
    }
    let mut depths = BTreeMap::from([(target.clone(), 0usize)]);
    let mut discovery = vec![target.clone()];
    let mut queue = VecDeque::from([target]);
    while let Some(child) = queue.pop_front() {
        for parent in parents.get(&child).into_iter().flatten() {
            let next = depths[&child] + 1;
            if depths.get(parent).is_none_or(|&old| next < old) {
                if !depths.contains_key(parent) {
                    discovery.push(parent.clone());
                }
                depths.insert(parent.clone(), next);
                queue.push_back(parent.clone());
            }
        }
    }
    let fallback = depths.values().max().copied().unwrap_or(0) + 1;
    for node in list(graph, "nodes") {
        let id = text(node, "id");
        if !depths.contains_key(&id) && nodes.contains_key(&id) {
            discovery.push(id.clone());
            depths.insert(id, fallback);
        }
    }
    let mut levels: BTreeMap<usize, Vec<&Value>> = BTreeMap::new();
    for id in discovery {
        if let Some(node) = nodes.get(&id) {
            levels.entry(depths[&id]).or_default().push(node);
        }
    }
    let mut result = vec![];
    for (depth, mut nodes) in levels {
        sort_nodes(&mut nodes);
        let compact: Vec<_> = nodes
            .iter()
            .map(|node| compact(node))
            .collect::<Result<_>>()?;
        let currencies: BTreeSet<_> = compact
            .iter()
            .map(|node| self::fallback(node, "fiat_currency"))
            .collect();
        let fiat = if !compact.is_empty()
            && currencies.len() == 1
            && compact
                .iter()
                .all(|node| !matches!(value(node, "fiat_value_allocated"), Value::Null))
        {
            Some(rounded(
                compact
                    .iter()
                    .map(|node| number(value(node, "fiat_value_allocated")))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .fold(0.0, |total, value| total + value),
                2,
            ))
        } else {
            None
        };
        result.push(dict([
            ("level", count(depth + 1)),
            ("role", s(if depth == 0 { "target" } else { "upstream" })),
            ("nodes", Value::List(compact)),
            (
                "transaction_count",
                count(
                    nodes
                        .iter()
                        .filter(|n| text(n, "node_type") == "transaction")
                        .count(),
                ),
            ),
            (
                "source_count",
                count(
                    nodes
                        .iter()
                        .filter(|n| text(n, "node_type") == "source")
                        .count(),
                ),
            ),
            (
                "assets",
                strings(
                    nodes
                        .iter()
                        .filter(|n| truthy(value(n, "asset")))
                        .map(|n| text(n, "asset"))
                        .collect::<BTreeSet<_>>(),
                ),
            ),
            (
                "fiat_currency",
                s(if fiat.is_some() {
                    currencies.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                }),
            ),
            (
                "fiat_value_total",
                fiat.map(Value::Float).unwrap_or(Value::Null),
            ),
        ]));
    }
    Ok(result)
}

fn simplified_node(node: &Value, deferred: bool) -> Value {
    let kind = ["source_type", "direction", "node_type"]
        .iter()
        .map(|key| value(node, key))
        .find(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| s(""));
    let mut row =
        dict(["id", "node_type", "transaction_id"].map(|key| (key, default_text(node, key))));
    replace(&mut row, "kind", kind);
    for key in ["label", "wallet", "asset"] {
        replace(&mut row, key, default_text(node, key));
    }
    for (key, required) in [
        ("amount", "required_amount"),
        ("amount_msat", "required_amount_msat"),
    ] {
        replace(
            &mut row,
            key,
            if matches!(value(node, required), Value::Null) {
                value(node, key)
            } else {
                value(node, required)
            }
            .clone(),
        );
    }
    replace(&mut row, "occurred_at", s(event_time(node)));
    replace(&mut row, "deferred_privacy_hop", Value::Bool(deferred));
    row
}
fn simplified_flow(report: &Value) -> Result<Value> {
    let graph = value(report, "graph");
    let nodes: BTreeMap<_, _> = list(graph, "nodes")
        .iter()
        .filter(|n| truthy(value(n, "id")))
        .map(|n| (text(n, "id"), n))
        .collect();
    let target_id = fallback(value(report, "target"), "id");
    if target_id.is_empty() || !nodes.contains_key(&target_id) {
        return Ok(dict([
            ("levels", Value::List(vec![])),
            ("edges", Value::List(vec![])),
            ("deferred_privacy_hops", Value::List(vec![])),
            ("note", s("")),
        ]));
    }
    let mut parents: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for edge in list(graph, "edges") {
        if truthy(value(edge, "from")) && truthy(value(edge, "to")) {
            parents.entry(text(edge, "to")).or_default().push(edge);
        }
    }
    let mut included = BTreeSet::from([target_id.clone()]);
    let mut inclusion_order = vec![target_id.clone()];
    let mut deferred = BTreeSet::new();
    let mut hops = vec![];
    let mut depths = BTreeMap::from([(target_id.clone(), 0usize)]);
    let mut queue = VecDeque::from([target_id]);
    let mut traversed = BTreeSet::new();
    while let Some(child) = queue.pop_front() {
        let depth = depths[&child];
        for &edge in parents.get(&child).into_iter().flatten() {
            if !traversed.insert(fallback(edge, "id")) {
                continue;
            }
            let parent = fallback(edge, "from");
            if !nodes.contains_key(&parent) {
                continue;
            }
            let next = depth + 1;
            if depths.get(&parent).is_none_or(|&old| next < old) {
                depths.insert(parent.clone(), next);
            }
            if included.insert(parent.clone()) {
                inclusion_order.push(parent.clone());
            }
            if privacy(edge) {
                deferred.insert(parent.clone());
                hops.push(dict([
                    ("link_id", default_text(edge, "id")), ("link_type", default_text(edge, "link_type")),
                    ("from", s(parent)), ("to", s(child.as_str())),
                    ("message", s("CoinJoin/PayJoin traversal is deferred; this reviewed hop is shown as a privacy boundary, not as ownership proof through unrelated participant inputs.")),
                ]));
            } else if !deferred.contains(&parent) {
                queue.push_back(parent);
            }
        }
    }
    let mut levels: BTreeMap<usize, Vec<&Value>> = BTreeMap::new();
    for id in super::source_funds_order::zero_seed_set(inclusion_order) {
        levels.entry(depths[&id]).or_default().push(nodes[&id]);
    }
    let max_depth = *levels.keys().max().expect("target depth");
    let levels = levels
        .into_iter()
        .rev()
        .map(|(depth, mut nodes)| {
            sort_nodes(&mut nodes);
            let mut role = if depth == 0 { "target" } else { "upstream" };
            if nodes.iter().all(|node| text(node, "node_type") == "source") {
                role = "source";
            }
            if role != "target"
                && nodes
                    .iter()
                    .any(|node| deferred.contains(&text(node, "id")))
            {
                role = "privacy_boundary";
            }
            dict([
                ("level", count(max_depth - depth + 1)),
                ("distance_to_target", count(depth)),
                ("role", s(role)),
                (
                    "nodes",
                    Value::List(
                        nodes
                            .into_iter()
                            .map(|node| simplified_node(node, deferred.contains(&text(node, "id"))))
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    let target = value(report, "target");
    let target_amount = if truthy(value(target, "required_amount_msat")) {
        value(target, "required_amount_msat")
    } else {
        value(target, "amount_msat")
    };
    let allocations = value(report, "allocations");
    let target_asset = if truthy(value(allocations, "asset")) {
        value(allocations, "asset")
    } else {
        value(target, "asset")
    };
    let target_asset = normalized_asset(target_asset);
    let mut edges = vec![];
    for edge in list(graph, "edges") {
        let parent = fallback(edge, "from");
        let child = fallback(edge, "to");
        if !included.contains(&parent) || !included.contains(child.as_str()) {
            continue;
        }
        let allocation = value(edge, "allocation_amount_msat");
        let share = if truthy(value(allocations, "gross_matches_target"))
            && truthy(target_amount)
            && !matches!(allocation, Value::Null)
            && integer(allocation)? <= integer(target_amount)?
            && normalized_asset(value(edge, "asset")) == target_asset
        {
            Value::Float(rounded(
                ratio(&integer(allocation)?, &integer(target_amount)?)? * 100.0,
                4,
            ))
        } else {
            Value::Null
        };
        edges.push(dict([
            ("id", default_text(edge, "id")),
            ("from", s(parent)),
            ("to", s(child)),
            ("link_type", default_text(edge, "link_type")),
            ("asset", default_text(edge, "asset")),
            ("amount", value(edge, "allocation_amount").clone()),
            ("amount_msat", allocation.clone()),
            ("share_of_target", share),
            ("deferred_privacy_hop", Value::Bool(privacy(edge))),
        ]));
    }
    Ok(dict([
        ("levels", Value::List(levels)), ("edges", Value::List(edges)), ("deferred_privacy_hops", Value::List(hops)),
        ("note", s("Simplified flow follows reviewed local links. Wallet transfers and consolidations are shown as reviewed hops; CoinJoin/PayJoin traversal is deferred and shown only as a privacy boundary.")),
    ]))
}

fn source_mix_phrase(mix: &[Value], fallback_asset: &str) -> Result<String> {
    if mix.is_empty() {
        return Ok("no reviewed root sources".into());
    }
    let mut parts = vec![];
    for row in mix.iter().take(4) {
        let amount = number(&amount(&integer(value(row, "amount_msat"))?)?)?;
        let suffix = if matches!(value(row, "percent_of_target"), Value::Null) {
            String::new()
        } else {
            format!(
                ", {:.1}% of target",
                number(value(row, "percent_of_target"))?
            )
        };
        let asset = fallback(row, "asset");
        let asset = if asset.is_empty() {
            fallback_asset
        } else {
            &asset
        };
        parts.push(format!(
            "{}: {amount:.8} {asset}{suffix}",
            label(row, "source_type")
        ));
    }
    if mix.len() > 4 {
        parts.push(format!("{} more source categories", mix.len() - 4));
    }
    Ok(parts.join("; "))
}
fn gross_requirement_phrase(allocations: &Value) -> Result<String> {
    list(allocations, "gross_source_requirement")
        .iter()
        .map(|row| {
            let amount = if truthy(value(row, "amount")) {
                number(value(row, "amount"))?
            } else {
                0.0
            };
            Ok(format!("{amount:.8} {}", fallback(row, "asset"))
                .trim()
                .to_owned())
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join(", "))
}

fn route_difference_explanation(report: &Value) -> Result<Option<Value>> {
    let Value::Int(difference) = value(value(report, "allocations"), "route_difference_msat")
    else {
        return Ok(None);
    };
    if *difference <= BigInt::from(0) {
        return Ok(None);
    }
    let graph = value(report, "graph");
    let nodes: BTreeMap<_, _> = list(graph, "nodes")
        .iter()
        .filter(|n| text(n, "node_type") == "transaction")
        .map(|n| (fallback(n, "id"), n))
        .collect();
    let mut consumed: BTreeMap<String, BigInt> = BTreeMap::new();
    for edge in list(graph, "edges") {
        let child = fallback(edge, "to");
        if !nodes.contains_key(&child) {
            continue;
        }
        let allocation = value(edge, "allocation_amount_msat");
        if matches!(allocation, Value::Null) {
            return Ok(None);
        }
        let from = if truthy(value(edge, "from_allocation_amount_msat")) {
            value(edge, "from_allocation_amount_msat")
        } else {
            allocation
        };
        *consumed.entry(child).or_default() += integer(from)? - integer(allocation)?;
    }
    let mut hops = 0;
    let mut total = BigInt::from(0);
    for (id, consumed) in consumed {
        if consumed == BigInt::from(0) {
            continue;
        }
        let node = nodes[&id];
        if consumed < BigInt::from(0)
            || truthy(value(node, "privacy_boundary"))
            || consumed != zero_integer(node, "fee_msat")?
        {
            return Ok(None);
        }
        hops += 1;
        total += consumed;
    }
    if hops == 0 || total != *difference {
        return Ok(None);
    }
    Ok(Some(dict([
        ("kind", s("network_fees")),
        ("fee_msat", Value::Int(total.clone())),
        ("fee", amount(&total)?),
        ("hop_count", count(hops)),
    ])))
}
