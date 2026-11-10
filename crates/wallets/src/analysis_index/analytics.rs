use super::{field, fields, list, AnalysisIndex, Text};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::{dumps, BigInt, Options, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

mod attribution;
pub use attribution::analyze_attribution;
mod patterns;
pub use patterns::analyze_patterns;

const RULE_VERSION: &str = "local-analytics-v1";
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    field(v, key).as_str().unwrap_or("")
}
fn count(n: usize) -> Value {
    Value::Int(BigInt::from(n as u64))
}
fn strings(values: impl IntoIterator<Item = impl AsRef<str>>) -> Value {
    Value::List(values.into_iter().map(|v| s(v.as_ref())).collect())
}
fn ids(v: &Value) -> Vec<String> {
    list(v)
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn amount(v: &Value) -> Option<BigInt> {
    let raw = match v {
        Value::Int(n) => n.to_string(),
        Value::Str(v) => v.as_str()?.to_owned(),
        _ => return None,
    };
    let n: BigInt = raw.parse().ok()?;
    (n >= BigInt::from(0) && raw == n.to_string()).then_some(n)
}
fn bad(v: &Value) -> bool {
    ["stale", "conflicting", "retracted"].contains(&text(v, "status"))
}
fn domain(v: &Value) -> Option<(String, String)> {
    let (chain, network) = (text(v, "chain"), text(v, "network"));
    if chain == "lightning" {
        return ["main", "test", "signet", "regtest"]
            .contains(&network)
            .then(|| (chain.into(), network.into()));
    }
    if !["bitcoin", "liquid"].contains(&chain) {
        return None;
    }
    (crate::descriptors::normalize_network(chain, network)
        .ok()
        .as_deref()
        == Some(network))
    .then(|| (chain.into(), network.into()))
}
fn ident(kind: &str, parts: Vec<Value>) -> String {
    let encoded = dumps(
        &Value::List(parts),
        &Options::new().sort_keys(true).default_str(true),
    )
    .expect("captured JSON values");
    let hex: String = Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{kind}:{}", &hex[..24])
}
fn finding(
    code: &str,
    members: &[String],
    evidence: Vec<Value>,
    detail: &str,
    observer: &str,
    level: &str,
    edges: &[String],
) -> Value {
    let members: BTreeSet<_> = members.iter().cloned().collect();
    let edges: BTreeSet<_> = edges.iter().cloned().collect();
    let title = code.replace('_', " ");
    let title = format!("{}{}", title[..1].to_uppercase(), &title[1..]);
    dict([
        ("id", s(ident(code, vec![strings(&members)]))),
        ("code", s(code)),
        ("rule_version", s(RULE_VERSION)),
        ("title", s(title)),
        ("detail", s(detail)),
        ("severity", s("info")),
        ("evidence_level", s(level)),
        ("observer", s(observer)),
        ("node_ids", strings(members)),
        ("edge_ids", strings(edges)),
        ("evidence", Value::List(evidence)),
        ("accounting_authority", Value::Bool(false)),
    ])
}

type Graph = BTreeMap<String, Vec<(String, String)>>;
type LabelGroup<'a> = (Vec<Value>, Vec<(&'a Value, String)>);
struct Context<'a> {
    index: &'a AnalysisIndex,
    query: &'a Value,
    nodes: BTreeMap<String, &'a Value>,
    edges: BTreeMap<String, &'a Value>,
    safe: BTreeSet<String>,
    physical: BTreeMap<String, &'a Value>,
    forward: Graph,
    reverse: Graph,
    txids: Vec<String>,
    labels: Vec<(&'a Value, Vec<String>)>,
    observer: &'a str,
    stopped: Vec<String>,
}
impl<'a> Context<'a> {
    fn new(index: &'a AnalysisIndex, query: &'a Value, selected: &BTreeSet<String>) -> Self {
        let nodes: BTreeMap<_, _> = selected
            .iter()
            .filter_map(|id| index.lookup("nodes", id).map(|v| (id.clone(), v)))
            .collect();
        let selected_edges =
            get(query, "_selected_edge_ids").map(|v| ids(v).into_iter().collect::<BTreeSet<_>>());
        let edges: BTreeMap<_, _> = fields(field(index.snapshot(), "edges"))
            .filter(|(id, e)| {
                nodes.contains_key(text(e, "source"))
                    && nodes.contains_key(text(e, "target"))
                    && selected_edges.as_ref().is_none_or(|set| set.contains(*id))
            })
            .map(|(id, v)| (id.to_owned(), v))
            .collect();
        let safe: BTreeSet<_> = nodes
            .iter()
            .filter(|(_, n)| !bad(n) && domain(n).is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let physical: BTreeMap<_, _> = edges
            .iter()
            .filter(|(_, e)| {
                ["creates", "spends"].contains(&text(e, "kind"))
                    && !bad(e)
                    && safe.contains(text(e, "source"))
                    && safe.contains(text(e, "target"))
                    && domain(nodes[text(e, "source")]) == domain(nodes[text(e, "target")])
            })
            .map(|(id, v)| (id.clone(), *v))
            .collect();
        let mut forward = Graph::new();
        let mut reverse = Graph::new();
        for (id, e) in &physical {
            forward
                .entry(text(e, "source").into())
                .or_default()
                .push((text(e, "target").into(), id.clone()));
            reverse
                .entry(text(e, "target").into())
                .or_default()
                .push((text(e, "source").into(), id.clone()));
        }
        let txids = safe
            .iter()
            .filter(|id| {
                text(nodes[*id], "kind") == "transaction"
                    && index.lookup("transaction_facts", id).is_some()
            })
            .cloned()
            .collect();
        let observer = match text(query, "observer") {
            "owner" => "owner",
            "disclosed" => "disclosed",
            _ => "public",
        };
        let mut labels = vec![];
        let mut stopped = vec![];
        for label in list(field(index.snapshot(), "labels")) {
            if !matches!(label, Value::Dict(_)) || truthy(field(label, "deleted")) {
                continue;
            }
            if observer == "public"
                && !(truthy(field(label, "dataset_id")) && text(label, "visibility") == "public")
            {
                continue;
            }
            let matched = if truthy(field(label, "node_ids")) {
                field(label, "node_ids")
            } else {
                index
                    .lookup("subjects", text(label, "subject"))
                    .unwrap_or(&Value::Null)
            };
            let mut matched: Vec<_> = ids(matched)
                .into_iter()
                .filter(|id| {
                    safe.contains(id)
                        && domain(nodes[id])
                            == Some((text(label, "chain").into(), text(label, "network").into()))
                })
                .collect();
            matched.sort();
            if !matched.is_empty() {
                labels.push((label, matched));
                if labels.len() >= 2000 {
                    stopped.push("label_analysis_budget".into());
                    break;
                }
            }
        }
        Self {
            index,
            query,
            nodes,
            edges,
            safe,
            physical,
            forward,
            reverse,
            txids,
            labels,
            observer,
            stopped,
        }
    }
    fn fact(&self, id: &str) -> &Value {
        self.index
            .lookup("transaction_facts", id)
            .unwrap_or(&Value::Null)
    }
    fn output(&self, id: &str) -> &Value {
        self.index
            .lookup("output_facts", id)
            .unwrap_or(&Value::Null)
    }
    fn adjacent<'b>(&'b self, graph: &'b Graph, id: &str) -> &'b [(String, String)] {
        graph.get(id).map(Vec::as_slice).unwrap_or(&[])
    }
    fn coverage(&self, edges: usize) -> Value {
        dict([
            ("selected_node_count", count(self.nodes.len())),
            (
                "hypotheses_enabled",
                Value::Bool(field(self.query, "include_hypotheses") == &Value::Bool(true)),
            ),
            ("observer", s(self.observer)),
            ("topology_complete", Value::Bool(false)),
            ("scope", s("selected_local_observations")),
            ("stopped_reasons", strings(&self.stopped)),
            ("transaction_count", count(self.txids.len())),
            (
                "amount_complete_transaction_count",
                count(
                    self.txids
                        .iter()
                        .filter(|id| {
                            let fact = self.fact(id);
                            truthy(field(fact, "complete"))
                                && list(field(fact, "inputs"))
                                    .iter()
                                    .chain(list(field(fact, "outputs")))
                                    .all(|r| amount(field(r, "amount_msat")).is_some())
                        })
                        .count(),
                ),
            ),
            ("hypothesis_count", count(edges)),
            ("label_count", count(self.labels.len())),
        ])
    }
}

struct Hypotheses {
    rows: Vec<Value>,
    stopped: bool,
}
impl Hypotheses {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        ctx: &Context<'_>,
        code: &str,
        a: &str,
        b: &str,
        premises: &[&str],
        evidence: Vec<Value>,
        scope: &str,
    ) {
        if field(ctx.query, "include_hypotheses") != &Value::Bool(true)
            || a == b
            || !ctx.safe.contains(a)
            || !ctx.safe.contains(b)
            || domain(ctx.nodes[a]) != domain(ctx.nodes[b])
        {
            return;
        }
        if self.rows.len() >= 500 {
            self.stopped = true;
            return;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        self.rows.push(dict([
            ("id", s(ident(code, vec![s(a), s(b)]))),
            ("source", s(a)),
            ("target", s(b)),
            ("kind", s("hypothesis")),
            ("rule", s(code)),
            ("rule_version", s(RULE_VERSION)),
            ("label", s(code.replace('_', " "))),
            ("status", s("hypothesis")),
            ("evidence_level", s("heuristic")),
            ("observer", s(scope)),
            ("premises", strings(premises)),
            ("evidence", Value::List(evidence)),
            ("reversible", Value::Bool(true)),
            ("accounting_authority", Value::Bool(false)),
        ]));
    }
}

pub fn analyze_clusters(
    index: &AnalysisIndex,
    query: &Value,
    selected: &BTreeSet<String>,
) -> Result<Value, AppError> {
    let mut ctx = Context::new(index, query, selected);
    let mut findings = vec![];
    let mut hypotheses = Hypotheses {
        rows: vec![],
        stopped: false,
    };
    let mut scripts: Vec<((String, String, String), Vec<String>)> = vec![];
    for id in &ctx.safe {
        if let Some(script) = field(ctx.output(id), "script")
            .as_str()
            .filter(|v| !v.is_empty() && !v.starts_with("6a"))
        {
            let (chain, network) = domain(ctx.nodes[id]).unwrap();
            let key = (chain, network, script.to_owned());
            if let Some((_, members)) = scripts.iter_mut().find(|(k, _)| *k == key) {
                members.push(id.clone());
            } else {
                scripts.push((key, vec![id.clone()]));
            }
        }
    }
    for (_, members) in scripts {
        if members.len() > 1 {
            findings.push(finding("script_reuse",&members,members.iter().flat_map(|id| list(field(ctx.nodes[id],"evidence")).to_vec()).collect(),"The same locking script occurs on distinct locally observed outputs. Common control remains a hypothesis.","public","observed",&[]));
            for id in &members[1..] {
                hypotheses.add(
                    &ctx,
                    "reused_script_control",
                    &members[0],
                    id,
                    &[
                        "same_locking_script",
                        "script_control_is_not_entity_identity",
                    ],
                    list(field(ctx.nodes[id], "evidence")).to_vec(),
                    "public",
                );
            }
        }
    }
    for txid in &ctx.txids {
        let fact = ctx.fact(txid);
        let observed_inputs: BTreeSet<_> = ctx
            .adjacent(&ctx.reverse, txid)
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        let observed_outputs: BTreeSet<_> = ctx
            .adjacent(&ctx.forward, txid)
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        let ins: Vec<_> = list(field(fact, "inputs"))
            .iter()
            .map(|r| text(r, "output_id"))
            .filter(|id| observed_inputs.contains(id))
            .collect();
        let outs: Vec<_> = list(field(fact, "outputs"))
            .iter()
            .map(|r| text(r, "output_id"))
            .filter(|id| observed_outputs.contains(id))
            .collect();
        let collab = field(fact, "collaboration");
        if ins.len() > 1 {
            let mut members = vec![txid.clone()];
            members.extend(ins.iter().map(|id| (*id).to_owned()));
            let edges = ctx
                .adjacent(&ctx.reverse, txid)
                .iter()
                .filter(|(id, _)| ins.contains(&id.as_str()))
                .map(|(_, e)| e.clone())
                .collect::<Vec<_>>();
            findings.push(finding("observed_co_spend",&members,list(field(ctx.nodes[txid],"evidence")).to_vec(),"These observed outputs are spent in one transaction. Common ownership is a separate hypothesis and can fail under collaboration.","public","observed",&edges));
        }
        if truthy(collab) {
            findings.push(finding("collaborative_boundary",std::slice::from_ref(txid),vec![collab.clone()],"Collaboration evidence suppresses common-input and change ownership assumptions; physical spends remain observable.",ctx.observer,get(collab,"evidence_level").and_then(Value::as_str).unwrap_or("heuristic"),&[]));
            continue;
        }
        if text(ctx.nodes[txid], "chain") != "bitcoin" {
            continue;
        }
        for other in ins.iter().skip(1) {
            hypotheses.add(
                &ctx,
                "common_input_control",
                ins[0],
                other,
                &["observed_co_spend", "assumes_no_undetected_collaboration"],
                list(field(ctx.nodes[txid], "evidence")).to_vec(),
                "public",
            );
        }
        if truthy(field(fact, "complete"))
            && outs.len() == 2
            && list(field(fact, "outputs")).len() == 2
            && !ins.is_empty()
        {
            let scripts: BTreeSet<_> = ins
                .iter()
                .map(|id| text(ctx.output(id), "script"))
                .filter(|v| !v.is_empty())
                .collect();
            let candidates: Vec<_> = outs
                .iter()
                .filter(|id| scripts.contains(text(ctx.output(id), "script")))
                .collect();
            if candidates.len() == 1 {
                hypotheses.add(
                    &ctx,
                    "change_script_return",
                    ins[0],
                    candidates[0],
                    &[
                        "two_output_transaction",
                        "one_output_reuses_input_script",
                        "assumes_no_undetected_collaboration",
                        "payment_to_same_script_remains_alternative",
                    ],
                    list(field(ctx.nodes[txid], "evidence")).to_vec(),
                    "public",
                );
            }
        }
        if ["owner", "disclosed"].contains(&ctx.observer) {
            for output in &outs {
                if text(ctx.output(output), "branch_role") != "change" {
                    continue;
                }
                let owners: BTreeSet<_> = ids(field(ctx.nodes[*output], "wallet_ids"))
                    .into_iter()
                    .collect();
                for input in &ins {
                    if ids(field(ctx.nodes[*input], "wallet_ids"))
                        .iter()
                        .any(|id| owners.contains(id))
                    {
                        hypotheses.add(
                            &ctx,
                            "private_wallet_change",
                            input,
                            output,
                            &[
                                "local_wallet_branch_evidence",
                                "not_public_observer_knowledge",
                                "assumes_no_undetected_collaboration",
                            ],
                            list(field(ctx.nodes[*output], "evidence")).to_vec(),
                            ctx.observer,
                        );
                    }
                }
            }
        }
    }
    let mut groups: Vec<LabelGroup<'_>> = vec![];
    for (label, members) in &ctx.labels {
        if field(label, "cluster_defining") == &Value::Bool(true)
            || (field(label, "cluster_defining") == &count(1)
                || field(label, "cluster_defining") == &Value::Float(1.0))
        {
            let key = ["chain", "network", "source", "label"]
                .iter()
                .map(|k| field(label, k).clone())
                .collect::<Vec<_>>();
            let at = groups
                .iter()
                .position(|(k, _)| *k == key)
                .unwrap_or_else(|| {
                    groups.push((key, vec![]));
                    groups.len() - 1
                });
            groups[at]
                .1
                .extend(members.iter().map(|id| (*label, id.clone())));
        }
    }
    for (_, members) in groups {
        for (label, id) in &members[1..] {
            hypotheses.add(
                &ctx,
                "authored_label_cluster",
                &members[0].1,
                id,
                &[
                    "explicit_cluster_defining_label",
                    "attribution_claim_not_ownership_proof",
                ],
                vec![dict([
                    ("source", s("local_label")),
                    ("reference", field(label, "id").clone()),
                    ("revision", field(label, "revision").clone()),
                    ("confidence", field(label, "confidence").clone()),
                ])],
                ctx.observer,
            );
        }
    }
    if hypotheses.stopped {
        ctx.stopped.push("hypothesis_budget".into());
    }
    let mut by_id = BTreeMap::new();
    for edge in hypotheses.rows {
        by_id.insert(text(&edge, "id").to_owned(), edge);
    }
    let edges: Vec<_> = by_id.into_values().collect();
    let clusters = clusters(&edges, &ctx);
    Ok(dict([
        ("rule_version", s(RULE_VERSION)),
        ("findings", Value::List(findings)),
        ("clusters", Value::List(clusters)),
        ("hypothesis_edges", Value::List(edges.clone())),
        ("coverage", ctx.coverage(edges.len())),
    ]))
}

fn clusters(edges: &[Value], ctx: &Context<'_>) -> Vec<Value> {
    let mut neighbors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in edges {
        neighbors
            .entry(text(e, "source").into())
            .or_default()
            .insert(text(e, "target").into());
        neighbors
            .entry(text(e, "target").into())
            .or_default()
            .insert(text(e, "source").into());
    }
    let mut remaining: BTreeSet<_> = neighbors.keys().cloned().collect();
    let mut result = vec![];
    while let Some(first) = remaining.pop_first() {
        let mut members = BTreeSet::from([first.clone()]);
        let mut pending = vec![first];
        while let Some(id) = pending.pop() {
            for next in &neighbors[&id] {
                if members.insert(next.clone()) {
                    remaining.remove(next);
                    pending.push(next.clone());
                }
            }
        }
        let refs: Vec<_> = edges
            .iter()
            .filter(|e| members.contains(text(e, "source")))
            .collect();
        let rules: BTreeSet<_> = refs.iter().map(|e| text(e, "rule")).collect();
        let node = ctx.nodes[members.first().unwrap()];
        result.push(dict([
            (
                "id",
                s(ident("cluster", vec![strings(&members), s(ctx.observer)])),
            ),
            ("node_ids", strings(members)),
            ("edge_ids", strings(refs.iter().map(|e| text(e, "id")))),
            ("observer", s(ctx.observer)),
            ("kind", s("possible_common_control")),
            ("evidence_level", s("heuristic")),
            ("rules", strings(rules)),
            ("rule_version", s(RULE_VERSION)),
            ("chain", field(node, "chain").clone()),
            ("network", field(node, "network").clone()),
            ("reversible", Value::Bool(true)),
            ("accounting_authority", Value::Bool(false)),
        ]));
    }
    result.sort_by_key(|v| text(v, "id").to_owned());
    result
}

#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_fixture(case: &Value, snapshots: &Value) -> Value {
    let empty = dict::<&str>([]);
    let query = get(case, "query").unwrap_or(&empty);
    let observer = get(query, "observer")
        .and_then(Value::as_str)
        .unwrap_or("public");
    let index = AnalysisIndex::from_snapshot(field(snapshots, text(case, "snapshot")).clone())
        .unwrap()
        .observer(observer)
        .unwrap();
    let selected = get(case, "selected")
        .map(|v| ids(v).into_iter().collect())
        .unwrap_or_else(|| {
            fields(field(index.snapshot(), "nodes"))
                .map(|(id, _)| id.to_owned())
                .collect()
        });
    if text(case, "group") == "full" {
        analyze_index(&index, query, &selected).unwrap()
    } else if text(case, "group") == "attribution" {
        analyze_attribution(&index, query, &selected).unwrap()
    } else if text(case, "group") == "patterns" {
        analyze_patterns(&index, query, &selected).unwrap()
    } else {
        analyze_clusters(&index, query, &selected).unwrap()
    }
}

/// Compose analyses over the supplied observer snapshot and selected subgraph.
/// This kernel neither sources observations nor registers a report route.
pub fn analyze_index(
    index: &AnalysisIndex,
    query: &Value,
    selected: &BTreeSet<String>,
) -> Result<Value, AppError> {
    use super::entropy::{analyze_transaction_entropy, Hooks, MODEL};
    let ctx = Context::new(index, query, selected);
    let mut result = analyze_clusters(index, query, selected)?;
    let patterns = analyze_patterns(index, query, selected)?;
    let attribution = analyze_attribution(index, query, selected)?;
    set(
        &mut result,
        "patterns",
        field(&patterns, "patterns").clone(),
    );
    set(
        &mut result,
        "exposure",
        field(&attribution, "exposure").clone(),
    );
    let mut coverage = field(&result, "coverage").clone();
    let mut reasons = list(field(&coverage, "stopped_reasons")).to_vec();
    for analysis in [&patterns, &attribution] {
        reasons.extend(list(field(field(analysis, "coverage"), "stopped_reasons")).to_vec());
    }
    set(
        &mut coverage,
        "pattern_count",
        field(field(&patterns, "coverage"), "pattern_count").clone(),
    );
    let label_count = field(&coverage, "label_count").clone();
    super::remove(&mut coverage, "label_count");
    set(&mut coverage, "label_count", label_count);
    let mut entropy = vec![];
    if text(query, "mode") == "entropy"
        || text(query, "action") == "entropy"
        || field(query, "entropy") == &Value::Bool(true)
    {
        let roots = attribution::roots(&ctx);
        let candidates: Vec<_> = ctx.txids.iter().filter(|id| roots.contains(*id)).collect();
        if candidates.is_empty() {
            entropy.push(dict([
                ("status", s("unsupported")),
                ("reason", s("select_transaction_subject")),
                ("model", s(MODEL)),
            ]));
        }
        if let Some(txid) = candidates.first() {
            let mut row = analyze_transaction_entropy(
                ctx.fact(txid),
                text(ctx.nodes[*txid], "chain"),
                field(query, "entropy_max_states"),
                field(query, "entropy_max_duration_ms"),
                &Value::Null,
                Hooks::default(),
            );
            set(&mut row, "transaction_node_id", s(txid.as_str()));
            entropy.push(row);
        }
        if candidates.len() > 1 {
            reasons.push(s("entropy_single_transaction_limit"));
        }
    }
    set(&mut result, "entropy", Value::List(entropy));
    set(&mut coverage, "stopped_reasons", Value::List(reasons));
    set(&mut result, "coverage", coverage);
    Ok(result)
}

#[cfg(test)]
mod tests;
