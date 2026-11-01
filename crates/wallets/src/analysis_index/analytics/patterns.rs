use super::*;
use std::collections::VecDeque;

pub(super) fn bounded(v: &Value, default: usize, maximum: usize) -> usize {
    amount(v)
        .map(|n| n.to_string().parse::<usize>().unwrap_or(maximum))
        .unwrap_or(default)
        .clamp(1, maximum)
}
struct Patterns {
    rows: Vec<Value>,
    stopped: Vec<String>,
}
impl Patterns {
    fn stop(&mut self, reason: &str) {
        if !self.stopped.iter().any(|r| r == reason) {
            self.stopped.push(reason.into());
        }
    }
    fn add(
        &mut self,
        ctx: &Context<'_>,
        code: &str,
        ids: &[String],
        edges: &[String],
        detail: &str,
        level: &str,
    ) {
        if self.rows.len() >= 250 {
            self.stop("pattern_result_budget");
            return;
        }
        let unique: BTreeSet<_> = edges.iter().collect();
        let evidence = unique
            .into_iter()
            .flat_map(|id| {
                ctx.physical
                    .get(id)
                    .map(|e| list(field(e, "evidence")).to_vec())
                    .unwrap_or_default()
            })
            .collect();
        let mut row = finding(code, ids, evidence, detail, "public", level, edges);
        set(&mut row, "scope", s("selected_local_topology"));
        let id = text(&row, "id").to_owned();
        if let Some(at) = self.rows.iter().position(|r| text(r, "id") == id) {
            self.rows[at] = row;
        } else {
            self.rows.push(row);
        }
    }
}
fn patterns(ctx: &Context<'_>) -> Patterns {
    let max_steps = bounded(
        get(ctx.query, "pattern_depth").unwrap_or(field(ctx.query, "depth")),
        6,
        12,
    );
    let mut remaining = 20_000usize;
    let mut result = Patterns {
        rows: vec![],
        stopped: vec![],
    };
    for txid in &ctx.txids {
        let incoming: Vec<_> = ctx
            .adjacent(&ctx.reverse, txid)
            .iter()
            .flat_map(|(output, eid2)| {
                ctx.adjacent(&ctx.reverse, output)
                    .iter()
                    .map(move |(parent, eid1)| {
                        (parent.clone(), eid1.clone(), output.clone(), eid2.clone())
                    })
            })
            .collect();
        let outgoing: Vec<_> = ctx
            .adjacent(&ctx.forward, txid)
            .iter()
            .flat_map(|(output, eid1)| {
                ctx.adjacent(&ctx.forward, output)
                    .iter()
                    .map(move |(child, eid2)| {
                        (child.clone(), eid2.clone(), output.clone(), eid1.clone())
                    })
            })
            .collect();
        for (code,rows,detail) in [("fan_in",&incoming,"Inputs from multiple observed parent transactions are spent together. This is topology, not common ownership."),("fan_out",&outgoing,"Different outputs have locally observed spends in different later transactions.")] {
            if rows.iter().map(|r| &r.0).collect::<BTreeSet<_>>().len()>1 {
                let mut ids=vec![txid.clone()]; ids.extend(rows.iter().map(|r| r.0.clone()));
                let proof=rows.iter().flat_map(|r| [r.1.clone(),r.3.clone()]).collect::<Vec<_>>();
                result.add(ctx,code,&ids,&proof,detail,"observed");
            }
        }
    }
    for origin in ctx.txids.iter().take(100) {
        type Branch = (String, Vec<String>, Vec<String>);
        let mut reached: Vec<(String, Vec<Branch>)> = vec![];
        for (first_output, first_edge) in ctx.adjacent(&ctx.forward, origin) {
            let mut queue = VecDeque::from([(
                first_output.clone(),
                vec![origin.clone(), first_output.clone()],
                vec![first_edge.clone()],
                0,
            )]);
            let mut seen = BTreeSet::from([origin.clone(), first_output.clone()]);
            while !queue.is_empty() && remaining > 0 {
                let (current, path, proof, hops) = queue.pop_front().unwrap();
                remaining -= 1;
                if text(ctx.nodes[&current], "kind") == "transaction" {
                    let at = reached
                        .iter()
                        .position(|(id, _)| id == &current)
                        .unwrap_or_else(|| {
                            reached.push((current.clone(), vec![]));
                            reached.len() - 1
                        });
                    if !reached[at].1.iter().any(|r| &r.0 == first_output) {
                        reached[at]
                            .1
                            .push((first_output.clone(), path.clone(), proof.clone()));
                    }
                }
                if hops >= max_steps * 2 {
                    if !ctx.adjacent(&ctx.forward, &current).is_empty() {
                        result.stop("pattern_depth_limit");
                    }
                    continue;
                }
                for (next, edge) in ctx.adjacent(&ctx.forward, &current) {
                    if seen.insert(next.clone()) {
                        let mut p = path.clone();
                        p.push(next.clone());
                        let mut e = proof.clone();
                        e.push(edge.clone());
                        queue.push_back((next.clone(), p, e, hops + 1));
                    }
                }
            }
        }
        for (_, branches) in reached {
            if branches.len() > 1 {
                let ids = branches
                    .iter()
                    .take(2)
                    .flat_map(|r| r.1.clone())
                    .collect::<Vec<_>>();
                let proof = branches
                    .iter()
                    .take(2)
                    .flat_map(|r| r.2.clone())
                    .collect::<Vec<_>>();
                let boundary = field(ctx.fact(origin), "collaboration");
                let code = if text(boundary, "kind") == "coinjoin" {
                    "postmix_reconvergence"
                } else {
                    "reconvergence"
                };
                result.add(ctx,code,&ids,&proof,"Distinct output branches reach the same later transaction through observed spends. No unique value allocation or common owner is inferred.","observed");
                if code == "postmix_reconvergence" {
                    let key = ident(
                        code,
                        vec![strings(ids.iter().cloned().collect::<BTreeSet<_>>())],
                    );
                    if let Some(row) = result.rows.iter_mut().find(|r| text(r, "id") == key) {
                        set(row, "boundary_evidence", boundary.clone());
                        set(
                            row,
                            "conditional_on_boundary_interpretation",
                            Value::Bool(true),
                        );
                    }
                }
            }
        }
        if remaining == 0 {
            break;
        }
    }
    if ctx.txids.len() > 100 || remaining == 0 {
        result.stopped.push("pattern_search_budget".into());
    }
    let mut next_peel = BTreeMap::new();
    for txid in &ctx.txids {
        let fact = ctx.fact(txid);
        let ins = list(field(fact, "inputs"));
        let outs = list(field(fact, "outputs"));
        if text(ctx.nodes[txid], "chain") != "bitcoin"
            || truthy(field(fact, "collaboration"))
            || !truthy(field(fact, "complete"))
            || ins.len() != 1
            || outs.len() != 2
        {
            continue;
        }
        let (Some(input), Some(a), Some(b)) = (
            amount(field(&ins[0], "amount_msat")),
            amount(field(&outs[0], "amount_msat")),
            amount(field(&outs[1], "amount_msat")),
        ) else {
            continue;
        };
        if &a + &b > input || a == b {
            continue;
        }
        let output = text(&outs[usize::from(a < b)], "output_id");
        let spenders = ctx.adjacent(&ctx.forward, output);
        if spenders.len() == 1 {
            if let Some((_, creates)) = ctx
                .adjacent(&ctx.forward, txid)
                .iter()
                .find(|(id, _)| id == output)
            {
                next_peel.insert(
                    txid.clone(),
                    (
                        spenders[0].0.clone(),
                        vec![creates.clone(), spenders[0].1.clone()],
                    ),
                );
            }
        }
    }
    if field(ctx.query, "include_hypotheses") == &Value::Bool(true) {
        for txid in next_peel.keys() {
            if next_peel.values().any(|(child, _)| child == txid) {
                continue;
            }
            let mut path = vec![txid.clone()];
            let mut proof = vec![];
            while path.len() < 12 {
                let Some((child, edges)) = next_peel.get(path.last().unwrap()) else {
                    break;
                };
                if path.contains(child) || !next_peel.contains_key(child) {
                    break;
                }
                path.push(child.clone());
                proof.extend(edges.iter().cloned());
            }
            if path.len() >= 3 {
                result.add(ctx,"peel_chain_candidate",&path,&proof,"At least three non-marked 1-input/2-output transactions continue through the larger output. Repeated payments and ownership remain hypotheses; undetected collaboration can invalidate them.","heuristic");
            }
        }
    }
    result.rows.sort_by_key(|row| text(row, "id").to_owned());
    result
}

pub fn analyze_patterns(
    index: &AnalysisIndex,
    query: &Value,
    selected: &BTreeSet<String>,
) -> Result<Value, AppError> {
    let ctx = Context::new(index, query, selected);
    let result = patterns(&ctx);
    Ok(dict([
        ("patterns", Value::List(result.rows.clone())),
        (
            "coverage",
            dict([
                ("stopped_reasons", strings(&result.stopped)),
                ("pattern_count", count(result.rows.len())),
            ]),
        ),
    ]))
}
