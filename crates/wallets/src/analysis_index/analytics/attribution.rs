use super::*;
use std::collections::VecDeque;

pub(super) fn roots(ctx: &Context<'_>) -> BTreeSet<String> {
    if truthy(field(ctx.query, "_subject_node_ids")) {
        return ids(field(ctx.query, "_subject_node_ids"))
            .into_iter()
            .filter(|id| ctx.nodes.contains_key(id))
            .collect();
    }
    let subjects = if truthy(field(ctx.query, "subjects")) {
        field(ctx.query, "subjects").clone()
    } else {
        Value::List(vec![field(ctx.query, "subject").clone()])
    };
    let subjects = if matches!(subjects, Value::Str(_)) {
        vec![subjects]
    } else {
        list(&subjects).to_vec()
    };
    let mut resolved = BTreeSet::new();
    for subject in subjects.iter().filter_map(Value::as_str) {
        if ctx.nodes.contains_key(subject) {
            resolved.insert(subject.to_owned());
        }
        resolved.extend(ids(ctx
            .index
            .lookup("subjects", subject)
            .unwrap_or(&Value::Null)));
    }
    if resolved.is_empty() {
        ctx.nodes.keys().cloned().collect()
    } else {
        resolved
            .into_iter()
            .filter(|id| ctx.nodes.contains_key(id))
            .collect()
    }
}

fn exposure(ctx: &Context<'_>) -> (Vec<Value>, Vec<String>) {
    if ctx.labels.is_empty() {
        return (vec![], vec![]);
    }
    let roots = roots(ctx);
    let max_depth = super::patterns::bounded(field(ctx.query, "depth"), 6, 12) * 2;
    let mut forward = ctx.forward.clone();
    let mut reverse = ctx.reverse.clone();
    if ctx.observer != "public" && get(ctx.query, "include_relations").is_none_or(truthy) {
        for (id, e) in &ctx.edges {
            if text(e, "kind") == "custody"
                && !bad(e)
                && ctx.safe.contains(text(e, "source"))
                && ctx.safe.contains(text(e, "target"))
            {
                forward
                    .entry(text(e, "source").into())
                    .or_default()
                    .push((text(e, "target").into(), id.clone()));
                reverse
                    .entry(text(e, "target").into())
                    .or_default()
                    .push((text(e, "source").into(), id.clone()));
            }
        }
    }
    let mut remaining = 20_000usize;
    let mut stopped = false;
    let mut reasons = vec![];
    let mut results = vec![];
    for (label, targets) in &ctx.labels {
        let claim = dict(
            [
                "id",
                "revision",
                "label",
                "category",
                "source",
                "confidence",
                "dataset_id",
                "dataset_version",
                "content_sha256",
                "license",
                "attribution_method",
                "source_record",
                "valid_from",
                "valid_until",
                "visibility",
            ]
            .iter()
            .filter_map(|key| get(label, key).map(|v| (*key, v.clone()))),
        );
        for target in targets {
            if results.len() >= 250 || remaining == 0 {
                stopped = true;
                break;
            }
            if roots.contains(target) {
                results.push(dict([
                    (
                        "id",
                        s(ident(
                            "exposure",
                            vec![field(&claim, "id").clone(), s(target.as_str())],
                        )),
                    ),
                    ("kind", s("direct_label_match")),
                    ("claim", claim.clone()),
                    ("node_ids", strings([target])),
                    ("edge_ids", Value::List(vec![])),
                    ("observer", s(ctx.observer)),
                    ("authority", s("sourced_claim")),
                    ("taint_inference", Value::Bool(false)),
                ]));
                continue;
            }
            for (direction, graph) in [("descendant", &forward), ("ancestor", &reverse)] {
                let mut queue = VecDeque::from([(target.clone(), vec![target.clone()], vec![])]);
                let mut seen = BTreeSet::from([target.clone()]);
                let mut found = None;
                while !queue.is_empty() && remaining > 0 {
                    let (current, path, proof): (String, Vec<String>, Vec<String>) =
                        queue.pop_front().unwrap();
                    remaining -= 1;
                    if roots.contains(&current) {
                        found = Some((path, proof));
                        break;
                    }
                    if proof.len() >= max_depth {
                        if !ctx.adjacent(graph, &current).is_empty()
                            && !reasons.iter().any(|r| r == "exposure_depth_limit")
                        {
                            reasons.push("exposure_depth_limit".to_owned());
                        }
                        continue;
                    }
                    for (next, edge) in ctx.adjacent(graph, &current) {
                        if seen.insert(next.clone()) {
                            let mut p = path.clone();
                            p.push(next.clone());
                            let mut e = proof.clone();
                            e.push(edge.clone());
                            queue.push_back((next.clone(), p, e));
                        }
                    }
                }
                if let Some((path, proof)) = found {
                    results.push(dict([
                        (
                            "id",
                            s(ident(
                                "exposure",
                                vec![field(&claim, "id").clone(), strings(&path), s(direction)],
                            )),
                        ),
                        ("kind", s("indirect_topological_contact")),
                        ("direction_from_label", s(direction)),
                        ("claim", claim.clone()),
                        ("node_ids", strings(path)),
                        ("edge_ids", strings(&proof)),
                        ("observer", s(ctx.observer)),
                        ("authority", s("sourced_claim_with_evidence_path")),
                        (
                            "path_kinds",
                            strings(proof.iter().map(|id| text(ctx.edges[id], "kind"))),
                        ),
                        ("taint_inference", Value::Bool(false)),
                        (
                            "limitations",
                            strings([
                                "adjacency_does_not_prove_payment_or_ownership",
                                "selected_local_coverage_only",
                            ]),
                        ),
                    ]));
                }
            }
            if results.len() >= 250 || remaining == 0 {
                stopped = true;
                break;
            }
        }
        if stopped {
            break;
        }
    }
    if stopped {
        reasons.push("exposure_search_budget".into());
    }
    results.sort_by_key(|v| text(v, "id").to_owned());
    (results, reasons)
}

pub fn analyze_attribution(
    index: &AnalysisIndex,
    query: &Value,
    selected: &BTreeSet<String>,
) -> Result<Value, AppError> {
    let ctx = Context::new(index, query, selected);
    let (rows, reasons) = exposure(&ctx);
    Ok(dict([
        ("exposure", Value::List(rows)),
        (
            "coverage",
            dict([
                ("stopped_reasons", strings(reasons)),
                ("label_count", count(ctx.labels.len())),
            ]),
        ),
    ]))
}
