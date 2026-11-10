//! Apply the public knowledge boundary before any query or analytics.
use super::{field, fields, list, remove, AnalysisIndex, Text};
use kassiber_envelope::{dict, s, set, truthy, AppError};
use kassiber_pyjson::{PyStr, Value};
use std::collections::{BTreeMap, BTreeSet};

fn refs(values: &Value, subject: &Value) -> Value {
    Value::List(
        list(values)
            .iter()
            .map(|v| {
                dict([
                    ("source", field(v, "source").clone()),
                    ("reference", subject.clone()),
                    ("level", field(v, "level").clone()),
                ])
            })
            .collect(),
    )
}

pub(super) fn public(index: &AnalysisIndex) -> Result<AnalysisIndex, AppError> {
    let owner = index.snapshot();
    let mut nodes = BTreeMap::new();
    let mut node_order = Vec::new();
    let mut outputs = Vec::new();
    for (id, node) in fields(field(owner, "nodes")) {
        if ![s("transaction"), s("output")].contains(field(node, "kind"))
            || ![s("bitcoin"), s("liquid")].contains(field(node, "chain"))
        {
            continue;
        }
        let mut public = node.clone();
        set(&mut public, "wallet_ids", Value::List(vec![]));
        remove(&mut public, "transaction_id");
        remove(&mut public, "occurred_at");
        set(
            &mut public,
            "evidence",
            refs(field(node, "evidence"), &s(id)),
        );
        let private = index.lookup("output_facts", id).unwrap_or(&Value::Null);
        if field(node, "kind") == &s("output") {
            outputs.push((
                id.to_owned(),
                dict([
                    ("script", field(private, "script").clone()),
                    ("branch_role", Value::Null),
                    ("ownership_known", Value::Bool(false)),
                ]),
            ));
            if field(node, "chain") == &s("liquid") {
                let amount = field(private, "public_amount_msat").clone();
                set(&mut public, "amount_msat", amount.clone());
                set(&mut public, "asset", field(private, "public_asset").clone());
                remove(&mut public, "address");
                set(
                    &mut public,
                    "amount_visibility",
                    s(if amount == Value::Null {
                        "unknown_for_observer"
                    } else {
                        "public_explicit"
                    }),
                );
            }
        }
        nodes.insert(id.to_owned(), public);
        node_order.push(id.to_owned());
    }
    let mut edges = Vec::new();
    let mut edge_ids = BTreeSet::new();
    let mut outgoing: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut incoming: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (id, edge) in fields(field(owner, "edges")) {
        let creates = field(edge, "kind") == &s("creates");
        let source = field(edge, "source").as_str().unwrap_or("");
        let target = field(edge, "target").as_str().unwrap_or("");
        if !(creates || field(edge, "kind") == &s("spends"))
            || !nodes.contains_key(source)
            || !nodes.contains_key(target)
        {
            continue;
        }
        let mut public = edge.clone();
        let output = &nodes[if creates { target } else { source }];
        set(
            &mut public,
            "amount_msat",
            field(output, "amount_msat").clone(),
        );
        set(&mut public, "asset", field(output, "asset").clone());
        set(
            &mut public,
            "evidence",
            refs(field(edge, "evidence"), field(output, "id")),
        );
        edges.push((id.to_owned(), public));
        edge_ids.insert(id.to_owned());
        outgoing.entry(source.to_owned()).or_default().push(s(id));
        incoming.entry(target.to_owned()).or_default().push(s(id));
    }
    let mut facts = Vec::new();
    for (id, fact) in fields(field(owner, "transaction_facts")) {
        let Some(node) = nodes.get(id) else { continue };
        let mut public = fact.clone();
        for side in ["inputs", "outputs"] {
            let values = list(field(fact, side))
                .iter()
                .map(|item| {
                    let mut item = item.clone();
                    let amount = field(&item, "output_id")
                        .as_str()
                        .and_then(|id| nodes.get(id))
                        .map(|n| field(n, "amount_msat").clone())
                        .unwrap_or(Value::Null);
                    set(&mut item, "amount_msat", amount);
                    item
                })
                .collect();
            set(&mut public, side, Value::List(values));
        }
        remove(&mut public, "directions");
        remove(&mut public, "transaction_ids");
        if field(node, "chain") == &s("liquid") {
            set(&mut public, "fee_msat", Value::Null);
        }
        let marker = field(&public, "collaboration");
        if truthy(marker)
            && !crate::coinjoin_shape::SOURCES
                .iter()
                .any(|source| field(marker, "source") == &s(*source))
        {
            set(
                &mut public,
                "collaboration",
                dict([
                    ("kind", s("unknown")),
                    ("source", s("observer_exclusion")),
                    ("evidence_level", s("unavailable")),
                ]),
            );
        }
        facts.push((id.to_owned(), public));
    }
    let mut subjects: BTreeMap<PyStr, BTreeSet<String>> = BTreeMap::new();
    let output_map: BTreeMap<_, _> = outputs.iter().map(|(k, v)| (k.as_str(), v)).collect();
    for id in &node_order {
        let node = &nodes[id];
        for alias in [
            s(id.as_str()),
            if field(node, "kind") == &s("transaction") {
                field(node, "txid").clone()
            } else {
                Value::Null
            },
            field(node, "outpoint").clone(),
            field(node, "address").clone(),
        ] {
            if truthy(&alias) {
                subjects
                    .entry(match &alias {
                        Value::Str(v) => v.clone(),
                        _ => kassiber_envelope::py_str(&alias).into(),
                    })
                    .or_default()
                    .insert(id.clone());
            }
        }
        if let Some(script) = output_map
            .get(id.as_str())
            .map(|v| field(v, "script"))
            .filter(|v| truthy(v))
        {
            subjects
                .entry(format!("script:{}", kassiber_envelope::py_str(script)).into())
                .or_default()
                .insert(id.clone());
        }
    }
    let mut findings = Vec::new();
    for finding in list(field(owner, "findings")) {
        let ids: Vec<_> = list(field(finding, "node_ids"))
            .iter()
            .filter(|id| id.as_str().is_some_and(|id| nodes.contains_key(id)))
            .cloned()
            .collect();
        if ids.is_empty() {
            continue;
        }
        let mut public = finding.clone();
        let visible_edges = list(field(finding, "edge_ids"))
            .iter()
            .filter(|id| id.as_str().is_some_and(|id| edge_ids.contains(id)))
            .cloned()
            .collect();
        set(
            &mut public,
            "evidence",
            refs(field(finding, "evidence"), &ids[0]),
        );
        set(&mut public, "node_ids", Value::List(ids));
        set(&mut public, "edge_ids", Value::List(visible_edges));
        findings.push(public);
    }
    let mut coverage = field(owner, "coverage").clone();
    if let Value::Dict(_) = field(&coverage, "datasets") {
        let mut datasets = field(&coverage, "datasets").clone();
        let public = field(field(&datasets, "visibility_coverage"), "public").clone();
        remove(&mut datasets, "visibility_coverage");
        for (k, v) in fields(&public) {
            set(&mut datasets, k, v.clone());
        }
        set(&mut coverage, "datasets", datasets);
    }
    for (key, value) in [
        ("observer_knowledge", s("public_chain_facts")),
        (
            "hidden_private_node_count",
            Value::from(fields(field(owner, "nodes")).count() - nodes.len()),
        ),
        (
            "hidden_private_relation_count",
            Value::from(
                fields(field(owner, "edges"))
                    .filter(|(_, e)| field(e, "kind") == &s("custody"))
                    .count(),
            ),
        ),
        (
            "public_liquid_value_policy",
            s("explicit_public_values_only"),
        ),
        ("private_labels_withheld", Value::Bool(true)),
    ] {
        set(&mut coverage, key, value);
    }
    let mut result = owner.clone();
    for (key, value) in [
        (
            "nodes",
            dict(node_order.iter().map(|id| (id.as_str(), nodes[id].clone()))),
        ),
        ("edges", dict(edges)),
        (
            "outgoing",
            dict(outgoing.into_iter().map(|(k, v)| (k, Value::List(v)))),
        ),
        (
            "incoming",
            dict(incoming.into_iter().map(|(k, v)| (k, Value::List(v)))),
        ),
        (
            "subjects",
            dict(
                subjects
                    .into_iter()
                    .map(|(k, v)| (k, Value::List(v.into_iter().map(s).collect()))),
            ),
        ),
        ("transaction_facts", dict(facts)),
        ("output_facts", dict(outputs)),
        ("findings", Value::List(findings)),
        ("coverage", coverage),
        (
            "labels",
            Value::List(
                list(field(owner, "labels"))
                    .iter()
                    .filter(|v| {
                        truthy(field(v, "dataset_id")) && field(v, "visibility") == &s("public")
                    })
                    .cloned()
                    .collect(),
            ),
        ),
        (
            "profile_seeds",
            Value::List(
                list(field(owner, "profile_seeds"))
                    .iter()
                    .filter(|v| v.as_str().is_some_and(|id| nodes.contains_key(id)))
                    .cloned()
                    .collect(),
            ),
        ),
    ] {
        set(&mut result, key, value);
    }
    AnalysisIndex::from_snapshot(result)
}
