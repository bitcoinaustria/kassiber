use super::graph;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(PartialEq, Eq, Hash)]
enum Group {
    Physical(graph::Scope),
    Legacy(PyStr, PyStr),
}

static EMPTY: Value = Value::Dict(vec![]);

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => &[],
    }
}
fn int(value: &Value) -> Result<BigInt> {
    let Value::Int(n) = int_zero(value)? else {
        unreachable!();
    };
    Ok(n)
}
fn legacy_txid(value: &Value) -> PyStr {
    let value = graph::empty_text(value);
    graph::strict_txid(&Value::Str(value.clone()))
        .filter(|_| value.code_points().count() == 64)
        .unwrap_or(value)
}

fn downstream(reason: &str) -> bool {
    matches!(
        reason,
        "custody_basis_barrier"
            | "transfer_pair_dependency_blocked"
            | "basis_provenance_incomplete"
            | "derived_transfer_group_blocked"
            | "at_swap_basis_carry_unresolved"
            | "bitcoin_rail_carry_basis_unresolved"
    )
}
fn pairable(reason: &str) -> bool {
    matches!(
        reason,
        "owned_fanout_unresolved"
            | "ownership_transfer_destination_ambiguous"
            | "ownership_transfer_source_ambiguous"
    )
}
pub(super) fn ownership_review(c: &Value, qs: &[Value]) -> Result<(Value, Value)> {
    let mut blocked = BTreeMap::new();
    for q in qs {
        let reason = text(required(q, "reason")?);
        let entry = blocked
            .entry(text(required(q, "transaction_id")?))
            .or_insert_with(|| reason.clone());
        if downstream(entry.as_str().unwrap_or("")) && !downstream(reason.as_str().unwrap_or("")) {
            *entry = text(required(q, "reason")?);
        }
    }
    if blocked.is_empty() {
        return Ok((
            dict([("total", Value::from(0)), ("by_reason", dict([]))]),
            Value::List(vec![]),
        ));
    }
    let rows = iterable(required(c, "rows")?)?;
    let rows = rows.as_slice();
    let by_id: HashMap<_, _> = rows.iter().map(|r| (text(field(r, "id")), r)).collect();
    let mut active = BTreeSet::new();
    let mut unavailable = BTreeSet::new();
    for component in iterable(required(c, "active_components")?)? {
        let component = &component;
        let legs_values = iterable(get_default(component, "legs", &Value::List(vec![]))?)?;
        let legs: HashMap<_, _> = legs_values
            .iter()
            .map(|l| Ok((graph::empty_text(get(l, "id")?), l)))
            .collect::<Result<_>>()?;
        for a in iterable(get_default(component, "allocations", &Value::List(vec![]))?)? {
            let a = &a;
            let source = legs
                .get(&graph::empty_text(get(a, "source_leg_id")?))
                .copied()
                .unwrap_or(&EMPTY);
            let sink = legs
                .get(&graph::empty_text(get(a, "sink_leg_id")?))
                .copied()
                .unwrap_or(&EMPTY);
            let out = fallback(
                field(source, "anchor_transaction_id"),
                field(source, "transaction_id"),
            );
            let inbound = fallback(
                field(sink, "anchor_transaction_id"),
                field(sink, "transaction_id"),
            );
            if !graph::present(out) {
                continue;
            }
            let out = text(out);
            let inbound = if !graph::present(inbound) {
                PyStr::new()
            } else {
                text(inbound)
            };
            active.insert((out.clone(), inbound.clone()));
            if inbound.code_points().next().is_none() {
                unavailable.insert(out);
                continue;
            }
            let o = by_id.get(&out).copied().unwrap_or(&Value::Null);
            let i = by_id.get(&inbound).copied().unwrap_or(&Value::Null);
            let kind = graph::empty_text(fallback(
                get(component, "component_type")?,
                &Value::from("manual"),
            ));
            let policy = graph::empty_text(fallback(
                get(component, "conversion_policy")?,
                &Value::from("carrying-value"),
            ));
            if matches!(o, Value::Null)
                || matches!(i, Value::Null)
                || graph::upper(&graph::empty_text(field(o, "asset")))
                    != graph::upper(&graph::empty_text(field(i, "asset")))
                || !policy.is("carrying-value")
                || !matches!(kind.as_str(), Some("manual" | "coinjoin" | "whirlpool"))
            {
                unavailable.insert(out);
                unavailable.insert(inbound);
            }
        }
    }
    let mut inbounds: HashMap<PyStr, Vec<&Value>> = HashMap::new();
    let mut groups: HashMap<Group, Vec<&Value>> = HashMap::new();
    let mut scopes = HashMap::<usize, Option<graph::Scope>>::new();
    fn row_scope(
        scopes: &mut HashMap<usize, Option<graph::Scope>>,
        row: &Value,
    ) -> Option<graph::Scope> {
        scopes
            .entry(row as *const Value as usize)
            .or_insert_with(|| graph::onchain_scope(row))
            .clone()
    }
    let mut physical_groups_built = false;
    let mut legacy_groups_built = false;
    for r in rows {
        if is(field(r, "direction"), "inbound") && int(field(r, "amount"))? > BigInt::from(0) {
            inbounds
                .entry(text(field(r, "wallet_id")))
                .or_default()
                .push(r);
        }
    }
    let mut candidates = vec![];
    let mut counts = BTreeMap::<PyStr, usize>::new();
    for (id, reason) in blocked {
        if !pairable(reason.as_str().unwrap_or("")) {
            continue;
        }
        let Some(source) = by_id.get(&id).copied() else {
            continue;
        };
        if unavailable.contains(&id)
            || !is(field(source, "direction"), "outbound")
            || int(field(source, "amount"))? <= BigInt::from(0)
        {
            continue;
        }
        let parsed = graph::parse(field(source, "raw_json"))?;
        let mut scope = None;
        let mut destinations = BTreeMap::<PyStr, BigInt>::new();
        if let Some(parsed) = &parsed {
            scope = graph::ownership_scope(source, parsed, field(c, "owned_index"))?;
            let Some(s) = &scope else {
                continue;
            };
            let mut ambiguous = false;
            for output in list(field(parsed, "outputs")) {
                let owners = graph::script_owners(
                    field(output, "script"),
                    &(s.0.clone(), s.1.clone()),
                    field(c, "owned_index"),
                )?;
                if owners.contains(&text(field(source, "wallet_id"))) {
                    continue;
                }
                if owners.len() > 1 {
                    ambiguous = true;
                    break;
                }
                if let Some(owner) = owners.first() {
                    *destinations.entry(owner.clone()).or_default() +=
                        int(field(output, "value_sats"))? * 1000;
                }
            }
            if ambiguous {
                continue;
            }
        } else if reason.is("owned_fanout_unresolved") {
            scope = row_scope(&mut scopes, source);
            if scope.is_some() && !physical_groups_built {
                for row in rows {
                    if let Some(s) = row_scope(&mut scopes, row) {
                        groups.entry(Group::Physical(s)).or_default().push(row);
                    }
                }
                physical_groups_built = true;
            }
            let legacy = legacy_txid(field(source, "external_id"));
            if scope.is_none() && legacy.code_points().next().is_none() {
                continue;
            }
            if scope.is_none() && !legacy_groups_built {
                for row in rows {
                    let legacy = legacy_txid(field(row, "external_id"));
                    if legacy.code_points().next().is_some() {
                        groups
                            .entry(Group::Legacy(
                                legacy,
                                graph::upper(&graph::empty_text(field(row, "asset"))),
                            ))
                            .or_default()
                            .push(row);
                    }
                }
                legacy_groups_built = true;
            }
            let key = scope.clone().map(Group::Physical).unwrap_or_else(|| {
                Group::Legacy(
                    legacy,
                    graph::upper(&graph::empty_text(field(source, "asset"))),
                )
            });
            let group = groups.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let mut outs = vec![];
            for row in group {
                if is(field(row, "direction"), "outbound")
                    && int(field(row, "amount"))? > BigInt::from(0)
                {
                    outs.push(row);
                }
            }
            if outs.len() != 1 || text(field(outs[0], "id")) != id {
                continue;
            }
            for inbound in group {
                if is(field(inbound, "direction"), "inbound")
                    && int(field(inbound, "amount"))? > BigInt::from(0)
                    && text(field(inbound, "wallet_id")) != text(field(source, "wallet_id"))
                {
                    *destinations
                        .entry(text(field(inbound, "wallet_id")))
                        .or_default() += int(field(inbound, "amount"))?;
                }
            }
        }
        let txid = parsed
            .as_ref()
            .map(|p| fallback(field(p, "txid"), field(source, "external_id")))
            .unwrap_or(field(source, "external_id"));
        let txid = graph::normalized(txid);
        let scope = scope.or_else(|| row_scope(&mut scopes, source));
        for (wallet, amount) in destinations {
            let mut exact = vec![];
            let mut compatible = vec![];
            for inbound in inbounds.get(&wallet).into_iter().flatten() {
                let iid = text(field(inbound, "id"));
                if graph::upper(&graph::empty_text(field(inbound, "asset")))
                    != graph::upper(&graph::empty_text(field(source, "asset")))
                    || unavailable.contains(&iid)
                    || active.contains(&(id.clone(), iid.clone()))
                {
                    continue;
                }
                let received = int(required(inbound, "amount")?)?;
                let diff = &received - &amount;
                let abs = if diff < BigInt::from(0) { -diff } else { diff };
                let magnitude = if amount < BigInt::from(0) {
                    -amount.clone()
                } else {
                    amount.clone()
                };
                let tolerance = std::cmp::max(magnitude / 100, BigInt::from(2_500_000));
                if abs > tolerance {
                    continue;
                }
                let iscope = row_scope(&mut scopes, inbound);
                if scope.is_some() && iscope.is_some() && scope != iscope {
                    continue;
                }
                if graph::strict_txid(&Value::Str(txid.clone())).is_some()
                    && iscope.as_ref().is_some_and(|s| s.2 != txid)
                {
                    continue;
                }
                if parsed.is_some()
                    && !reason.is("ownership_transfer_source_ambiguous")
                    && scope.is_some()
                    && iscope == scope
                    && int(field(source, "amount"))? == amount
                    && received == amount
                {
                    exact.push(*inbound);
                } else {
                    compatible.push(*inbound);
                }
            }
            let (selected, confidence) = if exact.is_empty() {
                (compatible, "strong")
            } else {
                (exact, "exact")
            };
            let size = selected.len();
            for inbound in selected {
                candidates.push(candidate(
                    source,
                    inbound,
                    &amount,
                    &reason,
                    &join(&[
                        "ownership-review:".into(),
                        id.clone(),
                        ":".into(),
                        wallet.clone(),
                    ]),
                    confidence,
                    size,
                )?);
                *counts.entry(reason.clone()).or_default() += 1;
            }
        }
    }
    Ok((
        dict([
            ("total", Value::from(candidates.len())),
            (
                "by_reason",
                Value::Dict(
                    counts
                        .into_iter()
                        .map(|(k, v)| (Key::Str(k), Value::from(v)))
                        .collect(),
                ),
            ),
        ]),
        Value::List(candidates),
    ))
}

fn candidate(
    out: &Value,
    inbound: &Value,
    amount: &BigInt,
    reason: &PyStr,
    conflict: &PyStr,
    confidence: &str,
    size: usize,
) -> Result<Value> {
    let received = int(required(inbound, "amount")?)?;
    let mut v = dict([
        ("out_id", Value::Str(text(required(out, "id")?))),
        ("in_id", Value::Str(text(required(inbound, "id")?))),
        ("out_amount_msat", Value::Int(amount.clone())),
        ("in_amount_msat", Value::Int(received.clone())),
        ("confidence", Value::from(confidence)),
        ("method", Value::from("ownership_graph")),
        ("swap_fee_msat", Value::Int(amount - received)),
        ("swap_fee_kind", Value::from("ownership_graph_delta")),
        ("default_kind", Value::from("manual")),
        ("default_policy", Value::from("carrying-value")),
        ("conflict_set_id", Value::Str(conflict.clone())),
        ("conflict_size", Value::from(size)),
        ("evidence_provider", Value::from("ownership_graph")),
        ("evidence_id", Value::Str(reason.clone())),
        ("evidence_kind", Value::from("owned_output")),
        ("evidence_conflicts", Value::List(vec![])),
        ("evidence_send_amount_msat", Value::Null),
        ("evidence_receive_amount_msat", Value::Null),
    ]);
    for (prefix, row) in [("out", out), ("in", inbound)] {
        for key in [
            "asset",
            "wallet_id",
            "wallet_label",
            "wallet_kind",
            "occurred_at",
        ] {
            let value = if key == "wallet_kind" && !truth(required(row, key)?) {
                PyStr::new()
            } else {
                text(required(row, key)?)
            };
            set(&mut v, &format!("{prefix}_{key}"), Value::Str(value));
        }
    }
    for k in [
        "evidence_status",
        "evidence_version",
        "evidence_taproot",
        "evidence_cooperative",
        "evidence_spend_path",
    ] {
        set(&mut v, k, Value::from(""));
    }
    Ok(v)
}
