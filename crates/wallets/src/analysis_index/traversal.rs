use super::{field, fields, list, AnalysisIndex, Text};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::{BigInt, Value};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn count(n: usize) -> Value {
    Value::Int(BigInt::from(n as u64))
}
fn strings(values: impl IntoIterator<Item = impl AsRef<str>>) -> Value {
    Value::List(values.into_iter().map(|v| s(v.as_ref())).collect())
}
fn integer(value: &Value) -> Option<BigInt> {
    let text = match value {
        Value::Int(n) => n.to_string(),
        Value::Str(v) => kassiber_pytext::strip(v.as_str()?).to_owned(),
        _ => return None,
    };
    if text.len() > kassiber_pyjson::INT_MAX_STR_DIGITS {
        return None;
    }
    let n: BigInt = text.parse().ok()?;
    (n >= BigInt::from(0) && n.to_string() == text).then_some(n)
}
fn size(value: &Value) -> usize {
    if let Value::Int(n) = value {
        n.to_string().parse().unwrap_or(0)
    } else {
        0
    }
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).as_str().unwrap_or("")
}
fn instant(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|v| crate::time::instant(v).ok().flatten())
}

pub fn normalize_query(args: &Value) -> Result<Value, AppError> {
    if !matches!(args, Value::Null | Value::Dict(_)) {
        return Err(invalid("Chain analysis arguments must be an object"));
    }
    let allowed = [
        "mode",
        "subject",
        "target",
        "chain",
        "network",
        "direction",
        "depth",
        "node_limit",
        "edge_limit",
        "include_relations",
        "include_hypotheses",
        "observer",
        "min_amount_msat",
        "start",
        "end",
    ];
    let mut unknown: Vec<_> = fields(args)
        .map(|(k, _)| k)
        .filter(|k| !allowed.contains(k))
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(invalid("Unsupported chain analysis arguments")
            .with_details(dict([("fields", strings(unknown))])));
    }
    let mut q = dict([
        ("mode", s("overview")),
        ("direction", s("both")),
        ("depth", count(4)),
        ("node_limit", count(400)),
        ("edge_limit", count(1200)),
        ("include_relations", Value::Bool(true)),
        ("include_hypotheses", Value::Bool(false)),
        ("observer", s("owner")),
    ]);
    for (key, value) in fields(args) {
        set(&mut q, key, value.clone());
    }
    for (key, choices) in [
        ("mode", vec!["overview", "trace", "path"]),
        ("direction", vec!["backward", "forward", "both"]),
        ("observer", vec!["public", "owner", "disclosed"]),
        ("chain", vec!["bitcoin", "liquid"]),
        ("network", vec!["main", "test", "signet", "regtest"]),
    ] {
        if get(&q, key).is_some()
            && !field(&q, key)
                .as_str()
                .is_some_and(|v| choices.contains(&v))
        {
            let mut choices = choices;
            choices.sort();
            return Err(invalid(format!("Invalid {key}"))
                .with_details(dict([("choices", strings(choices))])));
        }
    }
    for (key, low, high) in [
        ("depth", 1, 50),
        ("node_limit", 25, 2000),
        ("edge_limit", 50, 6000),
    ] {
        if !matches!(field(&q, key), Value::Int(_)) || !(low..=high).contains(&size(field(&q, key)))
        {
            return Err(invalid(format!(
                "{key} must be an integer between {low} and {high}"
            )));
        }
    }
    for key in ["include_relations", "include_hypotheses"] {
        if !matches!(field(&q, key), Value::Bool(_)) {
            return Err(invalid(format!("{key} must be a boolean")));
        }
    }
    for key in ["subject", "target"] {
        if let Some(value) = get(&q, key) {
            let Some(value) = value
                .as_str()
                .filter(|v| v.chars().count() <= 1024 && !kassiber_pytext::strip(v).is_empty())
            else {
                return Err(invalid(format!(
                    "{key} must be a nonempty identifier up to 1024 characters"
                )));
            };
            let value = s(kassiber_pytext::strip(value));
            set(&mut q, key, value);
        }
    }
    if ["trace", "path"].contains(&text(&q, "mode")) && !truthy(field(&q, "subject")) {
        return Err(invalid("A subject is required for trace/path mode"));
    }
    if text(&q, "mode") == "path" && !truthy(field(&q, "target")) {
        return Err(invalid("A target is required for path mode"));
    }
    if let Some(v) = get(&q, "min_amount_msat") {
        if v.as_str().is_none() || integer(v).is_none() {
            return Err(invalid(
                "min_amount_msat must be an exact nonnegative decimal integer string",
            ));
        }
    }
    let timestamp =
        Regex::new(r"\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})\z")
            .unwrap();
    for key in ["start", "end"] {
        if let Some(v) = get(&q, key) {
            if !v.as_str().is_some_and(|v| timestamp.is_match(v)) {
                return Err(invalid(format!("{key} must be an RFC3339 timestamp")));
            }
            if instant(v).is_none() {
                return Err(invalid(format!("{key} must include a valid timezone")));
            }
        }
    }
    if let (Some(start), Some(end)) = (instant(field(&q, "start")), instant(field(&q, "end"))) {
        if start > end {
            return Err(invalid("start must not be later than end"));
        }
    }
    Ok(q)
}

fn domain(node: &Value, query: &Value) -> bool {
    if let Some(chain) = get(query, "chain") {
        if field(node, "chain") != chain {
            return false;
        }
    }
    if let Some(network) = get(query, "network") {
        let Ok(wanted) = crate::descriptors::normalize_network(
            if ["lightning", "ln"].contains(&text(node, "chain")) {
                "bitcoin"
            } else {
                text(node, "chain")
            },
            network.as_str().unwrap_or(""),
        ) else {
            return false;
        };
        return field(node, "network") == &s(wanted);
    }
    true
}

pub fn resolve_subject(
    index: &AnalysisIndex,
    subject: &str,
    query: &Value,
) -> Result<Vec<String>, AppError> {
    let mut candidates = index.lookup("subjects", subject).unwrap_or(&Value::Null);
    let lower = subject.to_lowercase();
    if !truthy(candidates) && subject != lower {
        candidates = index.lookup("subjects", &lower).unwrap_or(&Value::Null);
    }
    let candidates: Vec<String> = list(candidates)
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| {
            index
                .lookup("nodes", id)
                .is_some_and(|node| domain(node, query))
        })
        .map(str::to_owned)
        .collect();
    if candidates.is_empty() {
        return Err(AppError::new(
            "not_found",
            "No locally observed graph subject matches this identifier",
        )
        .with_details(dict([
            ("subject", s(subject)),
            ("local_only", Value::Bool(true)),
        ])));
    }
    let domains: BTreeSet<_> = candidates
        .iter()
        .map(|id| {
            let node = index.lookup("nodes", id).unwrap();
            (text(node, "chain"), text(node, "network"))
        })
        .collect();
    let transaction_key = Regex::new(r"\A(?:tx:|out:)?[a-fA-F0-9]{64}(?::[0-9]+)?\z")
        .unwrap()
        .is_match(subject);
    let physical = transaction_key
        || subject.split(':').count() <= 2
            && candidates
                .iter()
                .any(|id| field(index.lookup("nodes", id).unwrap(), "address") == &s(subject));
    if physical && domains.len() > 1 {
        return Err(AppError::new(
            "subject_ambiguous",
            "Subject is observed in multiple chain/network domains",
        )
        .with_details(dict([(
            "domains",
            Value::List(
                domains
                    .into_iter()
                    .map(|(c, n)| dict([("chain", s(c)), ("network", s(n))]))
                    .collect(),
            ),
        )])));
    }
    let instance = Regex::new(r":domain:([^:]+):").unwrap();
    let instances: BTreeSet<_> = candidates
        .iter()
        .map(|id| instance.captures(id).map(|c| c[1].to_owned()))
        .collect();
    if physical && (instances.len() > 1 || transaction_key && candidates.len() > 1) {
        return Err(AppError::new(
            "subject_ambiguous",
            "Subject is observed in multiple chain instances or block occurrences",
        )
        .with_details(dict([("subjects", strings(&candidates))])));
    }
    Ok(candidates)
}

struct Traversal<'a> {
    index: &'a AnalysisIndex,
    query: &'a Value,
    nodes: BTreeSet<String>,
    edges: BTreeSet<String>,
    frontier: BTreeMap<(String, String), Value>,
    inspections: usize,
    budget: bool,
    filtered: BTreeMap<String, usize>,
    omitted: usize,
}
impl<'a> Traversal<'a> {
    fn new(index: &'a AnalysisIndex, query: &'a Value) -> Self {
        Self {
            index,
            query,
            nodes: BTreeSet::new(),
            edges: BTreeSet::new(),
            frontier: BTreeMap::new(),
            inspections: 0,
            budget: false,
            filtered: BTreeMap::new(),
            omitted: 0,
        }
    }
    fn stop(&mut self, id: &str, reason: &str, detail: Option<(&str, Value)>) {
        let key = (id.to_owned(), reason.to_owned());
        if !self.frontier.contains_key(&key)
            && self.frontier.len() >= size(field(self.query, "node_limit"))
        {
            self.omitted += 1;
            return;
        }
        let mut v = dict([("node_id", s(id)), ("reason", s(reason))]);
        if let Some((key, value)) = detail {
            set(&mut v, key, value);
        }
        self.frontier.insert(key, v);
    }
    fn admit(&mut self, id: &str) -> bool {
        if self.nodes.contains(id) {
            return true;
        }
        if self.nodes.len() >= size(field(self.query, "node_limit")) {
            self.budget = true;
            self.stop(id, "node_limit", None);
            return false;
        }
        self.nodes.insert(id.to_owned());
        true
    }
    fn neighbors(&mut self, id: &str, blocked: &BTreeSet<String>) -> Vec<(String, String)> {
        let node = self.index.lookup("nodes", id).unwrap();
        let status = text(node, "status");
        if ["conflicting", "stale"].contains(&status) {
            self.stop(id, status, None);
            return vec![];
        }
        if status == "missing" {
            self.stop(id, "missing_observation", None);
        }
        if text(node, "kind") == "transaction"
            && !truthy(field(
                self.index
                    .lookup("transaction_facts", id)
                    .unwrap_or(&Value::Null),
                "complete",
            ))
        {
            self.stop(id, "incomplete_transaction", None);
        }
        let direction = text(self.query, "direction");
        let outgoing = list(self.index.lookup("outgoing", id).unwrap_or(&Value::Null));
        if text(node, "kind") == "output"
            && ["forward", "both"].contains(&direction)
            && !outgoing.iter().any(|eid| {
                text(
                    self.index.lookup("edges", eid.as_str().unwrap()).unwrap(),
                    "kind",
                ) == "spends"
            })
        {
            self.stop(id, "unobserved_successor", None);
        }
        let mut edge_ids = BTreeSet::new();
        if ["forward", "both"].contains(&direction) {
            edge_ids.extend(outgoing.iter().filter_map(Value::as_str));
        }
        if ["backward", "both"].contains(&direction) {
            edge_ids.extend(
                list(self.index.lookup("incoming", id).unwrap_or(&Value::Null))
                    .iter()
                    .filter_map(Value::as_str),
            );
        }
        let mut result = vec![];
        for eid in edge_ids {
            if blocked.contains(eid) {
                continue;
            }
            if self.inspections >= size(field(self.query, "edge_limit")) {
                self.budget = true;
                self.stop(id, "edge_limit", None);
                break;
            }
            self.inspections += 1;
            let edge = self.index.lookup("edges", eid).unwrap();
            let other = if text(edge, "source") == id {
                text(edge, "target")
            } else {
                text(edge, "source")
            };
            let custody = text(edge, "kind") == "custody";
            if custody
                && (!truthy(field(self.query, "include_relations"))
                    || text(self.query, "observer") == "public")
            {
                *self
                    .filtered
                    .entry("relations_disabled".into())
                    .or_default() += 1;
                continue;
            }
            let status = text(edge, "status");
            if ["stale", "conflicting"].contains(&status) {
                self.stop(
                    id,
                    &if custody {
                        format!("{status}_relation")
                    } else {
                        status.to_owned()
                    },
                    Some(("edge_id", s(eid))),
                );
                continue;
            }
            let other_node = self.index.lookup("nodes", other).unwrap();
            let amount_filter = integer(field(self.query, "min_amount_msat"))
                .zip(integer(field(edge, "amount_msat")))
                .is_some_and(|(min, amount)| amount < min);
            let when = instant(field(other_node, "occurred_at"));
            let time_filter = when.is_some_and(|when| {
                instant(field(self.query, "start")).is_some_and(|start| when < start)
                    || instant(field(self.query, "end")).is_some_and(|end| when > end)
            });
            let filter = if !domain(other_node, self.query) {
                Some("domain")
            } else if amount_filter {
                Some("amount")
            } else if time_filter {
                Some("time")
            } else {
                None
            };
            if let Some(filter) = filter {
                *self.filtered.entry(filter.into()).or_default() += 1;
                self.stop(id, &format!("{filter}_filter"), Some(("edge_id", s(eid))));
                continue;
            }
            if self.admit(other) {
                self.edges.insert(eid.to_owned());
                result.push((other.to_owned(), eid.to_owned()));
            }
        }
        result
    }
    fn walk(&mut self, seeds: &[String]) {
        let mut queue = VecDeque::new();
        let mut distance = BTreeMap::new();
        for seed in seeds {
            if self.admit(seed) {
                queue.push_back(seed.clone());
                distance.insert(seed.clone(), 0);
            }
            if self.budget {
                break;
            }
        }
        while let Some(id) = queue.pop_front() {
            let depth = distance[&id];
            if depth >= size(field(self.query, "depth")) {
                self.stop(&id, "depth_limit", Some(("depth", count(depth))));
                continue;
            }
            for (other, _) in self.neighbors(&id, &BTreeSet::new()) {
                if !distance.contains_key(&other) {
                    distance.insert(other.clone(), depth + 1);
                    queue.push_back(other);
                }
            }
        }
    }
    fn path(
        &mut self,
        seeds: &[String],
        targets: &BTreeSet<String>,
        blocked: &BTreeSet<String>,
    ) -> Option<Value> {
        let mut queue = VecDeque::new();
        let mut prior: BTreeMap<String, Option<(String, String)>> = BTreeMap::new();
        let mut depth = BTreeMap::new();
        for seed in seeds {
            if self.admit(seed) {
                prior.insert(seed.clone(), None);
                depth.insert(seed.clone(), 0);
                queue.push_back(seed.clone());
            }
        }
        while let Some(id) = queue.pop_front() {
            let status = text(self.index.lookup("nodes", &id).unwrap(), "status");
            if ["stale", "conflicting"].contains(&status) {
                self.stop(&id, status, None);
                continue;
            }
            if targets.contains(&id) {
                let mut ids = vec![id.clone()];
                let mut edges = vec![];
                let mut current = id;
                while let Some((parent, eid)) = &prior[&current] {
                    edges.push(eid.clone());
                    ids.push(parent.clone());
                    current = parent.clone();
                }
                edges.reverse();
                ids.reverse();
                let relation = edges
                    .iter()
                    .any(|eid| text(self.index.lookup("edges", eid).unwrap(), "kind") == "custody");
                return Some(dict([
                    ("kind", s(if relation { "relation" } else { "physical" })),
                    ("node_ids", strings(ids)),
                    ("edge_ids", strings(edges)),
                ]));
            }
            if depth[&id] >= size(field(self.query, "depth")) {
                self.stop(&id, "depth_limit", Some(("depth", count(depth[&id]))));
                continue;
            }
            for (other, eid) in self.neighbors(&id, blocked) {
                if !prior.contains_key(&other) {
                    prior.insert(other.clone(), Some((id.clone(), eid)));
                    depth.insert(other.clone(), depth[&id] + 1);
                    queue.push_back(other);
                }
            }
        }
        None
    }
}

pub fn query_index(index: &AnalysisIndex, args: &Value) -> Result<Value, AppError> {
    let query = normalize_query(args)?;
    let index = index.observer(text(&query, "observer"))?;
    let seeds = if let Some(subject) = field(&query, "subject").as_str() {
        resolve_subject(&index, subject, &query)?
    } else {
        list(field(index.snapshot(), "profile_seeds"))
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| domain(index.lookup("nodes", id).unwrap(), &query))
            .map(str::to_owned)
            .collect()
    };
    let mut walk = Traversal::new(&index, &query);
    let mut paths = vec![];
    if text(&query, "mode") == "path" {
        let targets = resolve_subject(&index, text(&query, "target"), &query)?
            .into_iter()
            .collect();
        if let Some(primary) = walk.path(&seeds, &targets, &BTreeSet::new()) {
            let mut seen = BTreeSet::from([list(field(&primary, "edge_ids"))
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()]);
            paths.push(primary.clone());
            for eid in list(field(&primary, "edge_ids")) {
                if walk.budget || paths.len() >= 3 {
                    break;
                }
                if let Some(alternative) = walk.path(
                    &seeds,
                    &targets,
                    &BTreeSet::from([eid.as_str().unwrap().to_owned()]),
                ) {
                    let ids = list(field(&alternative, "edge_ids"))
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect::<Vec<_>>();
                    if seen.insert(ids) {
                        paths.push(alternative);
                    }
                }
            }
            paths.sort_by_key(|p| {
                let ids: Vec<_> = list(field(p, "edge_ids"))
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                (ids.len(), ids)
            });
        }
    } else {
        walk.walk(&seeds);
    }
    let nodes: Vec<_> = walk
        .nodes
        .iter()
        .map(|id| index.lookup("nodes", id).unwrap().clone())
        .collect();
    let mut edges: Vec<_> = walk
        .edges
        .iter()
        .map(|id| index.lookup("edges", id).unwrap().clone())
        .collect();
    let mut edge_ids = walk.edges.clone();
    if truthy(field(&query, "include_relations")) && text(&query, "observer") != "public" {
        for id in &walk.nodes {
            for eid in list(index.lookup("outgoing", id).unwrap_or(&Value::Null))
                .iter()
                .filter_map(Value::as_str)
            {
                let edge = index.lookup("edges", eid).unwrap();
                if text(edge, "kind") == "custody"
                    && walk.nodes.contains(text(edge, "target"))
                    && !edge_ids.contains(eid)
                    && edges.len() < size(field(&query, "edge_limit"))
                {
                    edges.push(edge.clone());
                    edge_ids.insert(eid.to_owned());
                }
            }
        }
    }
    let findings = list(field(index.snapshot(), "findings"))
        .iter()
        .filter(|f| {
            !truthy(field(f, "node_ids"))
                || list(field(f, "node_ids"))
                    .iter()
                    .any(|id| id.as_str().is_some_and(|id| walk.nodes.contains(id)))
        })
        .cloned()
        .collect();
    let frontier: Vec<_> = walk.frontier.values().cloned().collect();
    let missing = nodes.iter().any(|n| text(n, "status") == "missing")
        || frontier.iter().any(|f| {
            [
                "missing_observation",
                "incomplete_transaction",
                "unobserved_successor",
            ]
            .contains(&text(f, "reason"))
        });
    let mut coverage = field(index.snapshot(), "coverage").clone();
    let stale = nodes.iter().any(|n| text(n, "status") == "stale")
        || frontier.iter().any(|f| text(f, "reason").contains("stale"))
        || truthy(field(&coverage, "reference_reconciling_count"));
    let complete = frontier.is_empty()
        && !missing
        && !stale
        && !truthy(field(&coverage, "missing_tables"))
        && !truthy(field(&coverage, "invalid_observations"));
    for (key, value) in [
        ("observer_knowledge", s(match text(&query, "observer") { "public" => "public_chain_facts", "disclosed" => "assumes_all_local_owner_evidence_disclosed", _ => "local_owner_evidence" })),
        ("complete", Value::Bool(complete)), ("budget_exhausted", Value::Bool(walk.budget)), ("missing", Value::Bool(missing)), ("stale", Value::Bool(stale)), ("frontier_omitted_count", count(walk.omitted)),
        ("pruning", Value::List(walk.filtered.iter().map(|(key,n)| dict([("filter", s(key.as_str())), ("count", count(*n))])).collect())),
        ("depth_unit", s("graph_edges")), ("scope", s("locally_observed_subgraph")),
        ("unobserved_successors", s("An output without a local spend observation is not proven unspent.")),
        ("unknown_filter_values", s("Unknown amounts and times remain visible; filters prune known values only.")),
        ("path_semantics", s("Connectivity only. No input-to-output allocation or common ownership is implied.")),
        ("alternative_paths", s("Up to three distinct bounded alternatives; absence is not proof that no other path exists.")),
    ] { set(&mut coverage, key, value); }
    let summary = dict([
        ("node_count", count(nodes.len())),
        ("edge_count", count(edges.len())),
        (
            "transaction_count",
            count(
                nodes
                    .iter()
                    .filter(|n| text(n, "kind") == "transaction")
                    .count(),
            ),
        ),
        (
            "output_count",
            count(nodes.iter().filter(|n| text(n, "kind") == "output").count()),
        ),
        (
            "record_count",
            count(nodes.iter().filter(|n| text(n, "kind") == "record").count()),
        ),
        ("path_count", count(paths.len())),
        ("visited_node_count", count(walk.nodes.len())),
        ("inspected_edge_count", count(walk.inspections)),
    ]);
    Ok(dict([
        ("schema_version", count(1)),
        (
            "snapshot_id",
            field(index.snapshot(), "snapshot_id").clone(),
        ),
        ("query", query.clone()),
        ("summary", summary),
        ("nodes", Value::List(nodes)),
        ("edges", Value::List(edges)),
        ("findings", Value::List(findings)),
        ("clusters", Value::List(vec![])),
        ("paths", Value::List(paths)),
        ("frontier", Value::List(frontier)),
        ("coverage", coverage),
        (
            "capabilities",
            dict([
                ("local_graph", Value::Bool(true)),
                ("backward_forward_trace", Value::Bool(true)),
                ("bounded_alternative_paths", Value::Bool(true)),
                ("cross_rail_custody_relations", Value::Bool(true)),
                ("network_acquisition", Value::Bool(false)),
                ("taint_attribution", Value::Bool(false)),
                ("global_chain_completeness", Value::Bool(false)),
            ]),
        ),
    ]))
}

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_fixture(case: &Value, snapshots: &Value) -> Value {
    let result = if text(case, "operation") == "normalize" {
        normalize_query(field(case, "args"))
    } else {
        AnalysisIndex::from_snapshot(field(snapshots, text(case, "snapshot")).clone()).and_then(
            |index| {
                if text(case, "operation") == "resolve" {
                    let q = normalize_query(field(case, "args"))?;
                    resolve_subject(&index, text(case, "subject"), &q).map(strings)
                } else {
                    query_index(&index, field(case, "args"))
                }
            },
        )
    };
    result.unwrap_or_else(|e| {
        dict([(
            "error",
            dict([
                ("code", s(e.code)),
                ("message", s(e.message)),
                ("details", e.details.unwrap_or(Value::Null)),
                ("retryable", Value::Bool(e.retryable)),
            ]),
        )])
    })
}
