use super::{
    field, list,
    views::{GraphRead, SqlView},
    Text,
};
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const MAX_FINDINGS: usize = 100;
const MAX_ENTROPY: usize = 12;
const UNSAFE: [&str; 3] = ["missing", "stale", "conflicting"];
const LINK_RULES: [&str; 3] = [
    "common_input_control",
    "change_script_return",
    "reused_script_control",
];
const ASSUMPTIONS: [&str; 7] = [
    "public_chain_observer_with_locally_available_history",
    "private_ownership_selects_relevance_not_observer_knowledge",
    "common_control_is_conditional_not_identity_proof",
    "collaboration_blocks_ownership_assumptions_not_physical_spends",
    "topological_contact_is_not_payment_allocation_or_taint",
    "local_coverage_is_not_whole_chain_completeness",
    "entropy_counts_conditional_models_not_ownership_probabilities",
];
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    field(v, key).as_str().unwrap_or("")
}
fn count(v: usize) -> Value {
    Value::Int(v.into())
}
fn strings(v: impl IntoIterator<Item = impl AsRef<str>>) -> Value {
    Value::List(v.into_iter().map(|v| s(v.as_ref())).collect())
}
fn ids(v: &Value) -> BTreeSet<String> {
    list(v)
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn unique(v: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut found = Vec::new();
    for v in v {
        if !found.contains(&v) {
            found.push(v);
        }
    }
    found
}
pub fn query() -> Value {
    dict([
        ("mode", s("overview")),
        ("observer", s("public")),
        ("direction", s("both")),
        ("depth", count(12)),
        ("node_limit", count(2000)),
        ("edge_limit", count(6000)),
        ("include_relations", Value::Bool(false)),
        ("include_hypotheses", Value::Bool(true)),
    ])
}
fn investigation(public: &SqlView<'_>, subjects: &[Value]) -> Result<Value, AppError> {
    let mut query = query();
    for subject in subjects {
        let Some(id) = subject.as_str() else {
            continue;
        };
        let node = public.get("nodes", id)?;
        if node == Value::Null {
            continue;
        }
        let network = match text(&node, "network") {
            "liquidv1" => "main",
            "liquidtestnet" => "test",
            "elementsregtest" => "regtest",
            v => v,
        };
        for (k, v) in [
            ("mode", s("trace")),
            ("subject", s(id)),
            ("chain", field(&node, "chain").clone()),
            ("network", s(network)),
        ] {
            set(&mut query, k, v);
        }
        break;
    }
    Ok(dict([
        ("query", query),
        ("snapshot_id", public.snapshot_id()),
    ]))
}

pub fn analyze_snapshot(view: &SqlView<'_>, args: &Value) -> Result<Value, AppError> {
    let query = super::traversal::normalize_query(args)?;
    let view = view.for_observer(text(&query, "observer"))?;
    let mut result = super::traversal::query_read(&view, &query)?;
    let selected: BTreeSet<_> = list(field(&result, "nodes"))
        .iter()
        .map(|v| text(v, "id").to_owned())
        .collect();
    let selected_edges: BTreeSet<_> = list(field(&result, "edges"))
        .iter()
        .map(|v| text(v, "id").to_owned())
        .collect();
    let index = view.selected_index(&selected, &selected_edges)?;
    let mut analytics_query = query.clone();
    let roots = if let Some(subject) = field(&query, "subject").as_str() {
        super::traversal::resolve_read(&view, subject, &query)?
    } else {
        view.seeds(&query)?
    };
    set(&mut analytics_query, "_subject_node_ids", strings(roots));
    set(
        &mut analytics_query,
        "_selected_edge_ids",
        strings(&selected_edges),
    );
    let analytics = super::analytics::analyze_index(&index, &analytics_query, &selected)?;
    let mut findings = list(field(&result, "findings")).to_vec();
    findings.extend(list(field(&analytics, "findings")).iter().cloned());
    let mut coverage = field(&result, "coverage").clone();
    set(
        &mut coverage,
        "analytics",
        field(&analytics, "coverage").clone(),
    );
    set(&mut result, "coverage", coverage);
    let mut capabilities = field(&result, "capabilities").clone();
    set(
        &mut capabilities,
        "hypothesis_scope",
        s("overlay_only; hypotheses never establish physical or custody reachability"),
    );
    set(&mut result, "capabilities", capabilities);
    for key in ["clusters", "patterns", "exposure", "entropy"] {
        if let Some(v) = get(&analytics, key) {
            set(&mut result, key, v.clone());
        }
    }
    let mut features = Vec::new();
    for node in list(field(&result, "nodes")) {
        if ["missing", "stale", "conflicting", "retracted"].contains(&text(node, "status")) {
            continue;
        }
        let id = text(node, "id");
        let fact = view.get("transaction_facts", id)?;
        let snapshot = field(&fact, "features");
        if !truthy(snapshot) {
            continue;
        }
        features.push(dict([("subject", s(id)), ("features", snapshot.clone())]));
        let typed = super::value::Value::from_py(snapshot).serde();
        let collaboration = super::value::Value::from_py(field(&fact, "collaboration")).serde();
        for finding in crate::analysis_features::evaluate(&typed, Some(&collaboration)) {
            let mut finding = kassiber_pytext::loads(
                &serde_json::to_string(&finding).map_err(|_| super::invalid())?,
            )
            .map_err(|_| super::invalid())?;
            let code = text(&finding, "code");
            let title = capitalize(&code.replace('_', " "));
            let id = super::digest(&Value::List(vec![
                s(id),
                field(&finding, "code").clone(),
                field(&finding, "rule_version").clone(),
            ]))?;
            for (k, v) in [
                ("id", s(id)),
                ("title", s(title)),
                ("detail", field(&finding, "message").clone()),
                ("node_ids", strings([text(node, "id")])),
                ("edge_ids", Value::List(vec![])),
                ("accounting_authority", Value::Bool(false)),
                ("observer", field(&query, "observer").clone()),
                ("evidence_level", field(&finding, "authority").clone()),
            ] {
                set(&mut finding, k, v);
            }
            findings.push(finding);
        }
    }
    set(&mut result, "findings", Value::List(findings));
    set(&mut result, "transaction_features", Value::List(features));
    if truthy(field(&query, "include_hypotheses")) {
        let mut edges = list(field(&result, "edges")).to_vec();
        let limit = text_number(field(&query, "edge_limit"));
        for edge in list(field(&analytics, "hypothesis_edges")) {
            if edges.len() >= limit {
                let mut coverage = field(&result, "coverage").clone();
                set(&mut coverage, "budget_exhausted", Value::Bool(true));
                set(&mut result, "coverage", coverage);
                break;
            }
            if selected.contains(text(edge, "source")) && selected.contains(text(edge, "target")) {
                edges.push(edge.clone());
            }
        }
        let mut summary = field(&result, "summary").clone();
        set(&mut summary, "edge_count", count(edges.len()));
        set(&mut result, "summary", summary);
        set(&mut result, "edges", Value::List(edges));
    }
    Ok(result)
}
fn capitalize(v: &str) -> String {
    let mut chars = v.chars();
    match chars.next() {
        Some(first) => format!(
            "{}{}",
            first.to_uppercase(),
            kassiber_pytext::lower(chars.as_str())
        ),
        None => String::new(),
    }
}
fn text_number(v: &Value) -> usize {
    match v {
        Value::Int(n) => n.to_string().parse().unwrap_or(0),
        _ => 0,
    }
}

fn relevance(
    owner: &SqlView<'_>,
    owned: &BTreeSet<String>,
    subjects: &[Value],
    edges: Option<&[Value]>,
) -> Result<(&'static str, usize), AppError> {
    let mut related = ids(&Value::List(subjects.to_vec()));
    if let Some(edges) = edges {
        for id in edges.iter().filter_map(Value::as_str) {
            let edge = owner.get("edges", id)?;
            for k in ["source", "target"] {
                if let Some(id) = field(&edge, k).as_str() {
                    related.insert(id.to_owned());
                }
            }
        }
    }
    let mut spends = BTreeSet::new();
    let mut receipts = BTreeSet::new();
    for id in subjects.iter().filter_map(Value::as_str) {
        let fact = owner.get("transaction_facts", id)?;
        for (side, found) in [("inputs", &mut spends), ("outputs", &mut receipts)] {
            for row in list(field(&fact, side)) {
                let output = text(row, "output_id");
                if owned.contains(output) && (edges.is_none() || related.contains(output)) {
                    found.insert(output.to_owned());
                }
            }
        }
    }
    let direct = related
        .intersection(owned)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut affected = direct.clone();
    affected.extend(spends.iter().cloned());
    affected.extend(receipts.iter().cloned());
    Ok((
        if !spends.is_empty() {
            "own_spend"
        } else if !direct.is_empty() {
            "owned_output"
        } else if !receipts.is_empty() {
            "received_context"
        } else {
            "nearby_context"
        },
        affected.len(),
    ))
}
struct Findings<'a, 'o, 'p> {
    owner: &'a SqlView<'o>,
    public: &'a SqlView<'p>,
    owned: &'a BTreeSet<String>,
    rows: BTreeMap<String, Value>,
}
impl Findings<'_, '_, '_> {
    fn add(
        &mut self,
        row: &Value,
        category: &str,
        code: Option<&str>,
        assumptions: &[&str],
        title: Option<&str>,
        detail: Option<&str>,
    ) -> Result<(), AppError> {
        let code = code.unwrap_or_else(|| {
            if truthy(field(row, "code")) {
                text(row, "code")
            } else {
                text(row, "kind")
            }
        });
        let mut visible = Vec::new();
        for id in unique(list(field(row, "node_ids")).iter().cloned()) {
            let Some(key) = id.as_str() else {
                continue;
            };
            let node = self.public.get("nodes", key)?;
            if node != Value::Null {
                if UNSAFE.contains(&text(&node, "status")) {
                    return Ok(());
                }
                visible.push(id);
            }
        }
        if visible.is_empty() {
            return Ok(());
        }
        let edges = if category == "pattern" || text(row, "kind") == "indirect_topological_contact"
        {
            Some(list(field(row, "edge_ids")))
        } else {
            None
        };
        let (relevance, affected) = relevance(self.owner, self.owned, &visible, edges)?;
        let actionable = ["own_spend", "owned_output"].contains(&relevance)
            && (LINK_RULES.contains(&code)
                || ["script_reuse", "postmix_reconvergence"].contains(&code));
        let identity = if truthy(field(row, "id")) {
            text(row, "id").to_owned()
        } else {
            super::digest(&Value::List(vec![
                s(code),
                strings(ids(&Value::List(visible.clone()))),
            ]))?
        };
        let title = title
            .map(str::to_owned)
            .or_else(|| {
                field(row, "title")
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| capitalize(&code.replace('_', " ")));
        let detail = detail.map(str::to_owned).unwrap_or_else(|| {
            ["detail", "message"]
                .into_iter()
                .map(|k| text(row, k))
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .to_owned()
        });
        let authority = ["authority", "evidence_level"]
            .into_iter()
            .map(|k| text(row, k))
            .find(|s| !s.is_empty())
            .unwrap_or("observed");
        let assumptions = unique(
            assumptions
                .iter()
                .map(|v| s(*v))
                .chain(list(field(row, "assumptions")).iter().cloned())
                .chain(list(field(row, "premises")).iter().cloned()),
        );
        let mut value = dict([
            ("id", s(identity.as_str())),
            ("code", s(code)),
            ("category", s(category)),
            ("severity", s(if actionable { "warning" } else { "info" })),
            ("authority", s(authority)),
            ("relevance", s(relevance)),
            ("title", s(title)),
            ("detail", s(detail)),
            ("assumptions", Value::List(assumptions)),
            (
                "limitations",
                Value::List(list(field(row, "limitations")).to_vec()),
            ),
            ("affected_output_count", count(affected)),
            (
                "source_ids",
                get(row, "source_ids")
                    .cloned()
                    .unwrap_or_else(|| strings([identity.as_str()])),
            ),
            ("investigation", investigation(self.public, &visible)?),
        ]);
        if truthy(field(row, "claim")) {
            set(&mut value, "claim", field(row, "claim").clone());
        }
        if code == "script_reuse" {
            let fact = self
                .public
                .get("output_facts", visible[0].as_str().unwrap())?;
            if let Some(script) = field(&fact, "script").as_str().filter(|v| !v.is_empty()) {
                let mut handoff = field(&value, "investigation").clone();
                let mut query = field(&handoff, "query").clone();
                if script.chars().count() + 7 <= 1024 {
                    set(&mut query, "subject", s(format!("script:{script}")));
                } else {
                    query = self::query();
                }
                set(&mut handoff, "query", query);
                set(&mut value, "investigation", handoff);
            }
        }
        self.rows.insert(identity, value);
        Ok(())
    }
}
fn findings(
    owner: &SqlView<'_>,
    public: &SqlView<'_>,
    owned: &BTreeSet<String>,
    analysis: &Value,
) -> Result<Vec<Value>, AppError> {
    let mut output = Findings {
        owner,
        public,
        owned,
        rows: BTreeMap::new(),
    };
    for row in list(field(analysis, "findings")) {
        if ["script_reuse", "collaborative_boundary"].contains(&text(row, "code")) {
            output.add(row, "linkage", None, &[], None, None)?;
        } else if truthy(field(row, "rule_version")) {
            output.add(row, "structure", None, &[], None, None)?;
        }
    }
    let mut groups: BTreeMap<(String, String), Value> = BTreeMap::new();
    for edge in list(field(analysis, "edges")) {
        let rule = text(edge, "rule");
        if text(edge, "kind") != "hypothesis"
            || !LINK_RULES.contains(&rule)
            || rule == "reused_script_control"
        {
            continue;
        }
        let mut anchor = text(edge, "source").to_owned();
        for reference in list(field(edge, "evidence")) {
            let id = text(reference, "reference");
            if public.get("transaction_facts", id)? != Value::Null {
                anchor = id.to_owned();
                break;
            }
        }
        let group = groups
            .entry((rule.to_owned(), anchor.clone()))
            .or_insert_with(|| {
                dict([
                    ("node_ids", strings([anchor])),
                    ("source_ids", Value::List(vec![])),
                    ("premises", Value::List(vec![])),
                    ("evidence_level", s("hypothesis")),
                ])
            });
        for (key, values) in [
            (
                "node_ids",
                vec![field(edge, "source").clone(), field(edge, "target").clone()],
            ),
            ("source_ids", vec![field(edge, "id").clone()]),
            ("premises", list(field(edge, "premises")).to_vec()),
        ] {
            let mut rows = list(field(group, key)).to_vec();
            rows.extend(values);
            set(group, key, Value::List(rows));
        }
    }
    for ((code, anchor), mut row) in groups {
        let id = super::digest(&Value::List(vec![
            s("privacy_mirror"),
            s(code.as_str()),
            s(anchor),
            strings(ids(field(&row, "source_ids"))),
        ]))?;
        set(&mut row, "id", s(id));
        let detail = if code == "common_input_control" {
            "Observed inputs are spent together. Common control is conditional on this being an ordinary, non-collaborative transaction."
        } else {
            "One output returns to an input script. Change is a hypothesis; a payment to the same script remains an alternative."
        };
        output.add(&row, "linkage", Some(&code), &[], None, Some(detail))?;
    }
    for row in list(field(analysis, "patterns")) {
        output.add(
            row,
            "pattern",
            None,
            &["connectivity_does_not_allocate_value"],
            None,
            None,
        )?;
    }
    for row in list(field(analysis, "exposure")) {
        let detail = if text(row, "kind") == "direct_label_match" {
            "A locally imported public dataset names this subject. The claim is not verified ownership."
        } else {
            "Observed transaction links reach a subject named by a locally imported public dataset. This is not evidence of payment, ownership or taint."
        };
        output.add(
            row,
            "attribution",
            None,
            &["source_claim_requires_independent_review"],
            Some("Public attribution claim"),
            Some(detail),
        )?;
    }
    let mut rows: Vec<_> = output.rows.into_values().collect();
    rows.sort_by_key(|row| {
        (
            text(row, "severity") != "warning",
            ["received_context", "nearby_context"].contains(&text(row, "relevance")),
            text(row, "code").to_owned(),
            text(row, "id").to_owned(),
        )
    });
    Ok(rows)
}

fn entropy(
    owner: &SqlView<'_>,
    public: &SqlView<'_>,
    owned: &BTreeSet<String>,
    selected: &BTreeSet<String>,
    deterministic: bool,
    mut elapsed: impl FnMut() -> f64,
) -> Result<Value, AppError> {
    let mut candidates = Vec::new();
    for id in selected {
        let node = public.get("nodes", id)?;
        let fact = public.get("transaction_facts", id)?;
        if !UNSAFE.contains(&text(&node, "status")) && truthy(field(&fact, "inputs")) {
            let (relevance, _) = relevance(owner, owned, &[s(id.as_str())], None)?;
            candidates.push((
                relevance != "own_spend",
                list(field(&fact, "inputs")).len() < 2,
                id.clone(),
            ));
        }
    }
    candidates.sort();
    let start = Instant::now();
    let mut results = Vec::new();
    for (_, _, id) in candidates.iter().take(MAX_ENTROPY) {
        let remaining = (300.0
            - if deterministic {
                elapsed()
            } else {
                start.elapsed().as_secs_f64() * 1000.0
            })
        .max(0.0) as usize;
        if remaining == 0 {
            break;
        }
        let node = public.get("nodes", id)?;
        let fact = public.get("transaction_facts", id)?;
        #[cfg(any(test, feature = "test-harness"))]
        let mut clock = || 0.0;
        #[cfg(any(test, feature = "test-harness"))]
        let hooks = super::entropy::Hooks {
            elapsed_ms: if deterministic {
                Some(&mut clock)
            } else {
                None
            },
            ..Default::default()
        };
        #[cfg(not(any(test, feature = "test-harness")))]
        let hooks = {
            let _ = deterministic;
            super::entropy::Hooks::default()
        };
        let value = super::entropy::analyze_transaction_entropy(
            &fact,
            text(&node, "chain"),
            &count(5000),
            &count(remaining.min(25)),
            &Value::Null,
            hooks,
        );
        // Python's key order: the subject, the flag, the entropy fields,
        // then the investigation handoff.
        let mut row = dict([
            ("subject", s(id.as_str())),
            ("conditional_on_model", Value::Bool(true)),
        ]);
        for key in [
            "status",
            "reason",
            "model",
            "rule_version",
            "interpretation_count",
            "interpretation_count_lower_bound",
            "entropy_bits",
            "conditional_on_model",
            "assumptions",
            "limitations",
            "limits",
        ] {
            if let Some(v) = get(&value, key) {
                set(&mut row, key, v.clone());
            }
        }
        set(
            &mut row,
            "investigation",
            investigation(public, &[s(id.as_str())])?,
        );
        results.push(row);
    }
    Ok(dict([
        ("evaluated", count(results.len())),
        ("eligible", count(candidates.len())),
        ("omitted", count(candidates.len() - results.len())),
        ("results", Value::List(results)),
        (
            "limits",
            dict([
                ("transactions", count(MAX_ENTROPY)),
                ("total_duration_ms", count(300)),
                ("states_per_transaction", count(5000)),
            ]),
        ),
    ]))
}
fn coverage(
    public: &SqlView<'_>,
    analysis: &Value,
    entropy: &Value,
    finding_count: usize,
) -> Result<Value, AppError> {
    let nodes = list(field(analysis, "nodes"));
    let selected: Vec<_> = nodes
        .iter()
        .filter(|n| text(n, "kind") == "transaction")
        .collect();
    let available = public.available_transaction_count()?;
    let safe: Vec<_> = selected
        .iter()
        .filter(|n| !UNSAFE.contains(&text(n, "status")))
        .collect();
    let selected_available = selected
        .iter()
        .filter(|n| text(n, "status") != "missing")
        .count();
    let source = field(analysis, "coverage");
    let analytics = field(source, "analytics");
    let datasets = field(source, "datasets");
    let mut reasons = ids(field(analytics, "stopped_reasons"));
    reasons.remove("entropy_single_transaction_limit");
    for row in list(field(analysis, "frontier")) {
        reasons.insert(text(row, "reason").to_owned());
    }
    for (condition, reason) in [
        (finding_count > MAX_FINDINGS, "finding_display_limit"),
        (
            truthy(field(entropy, "omitted")),
            "entropy_transaction_budget",
        ),
        (
            selected_available < available,
            "local_transactions_outside_selected_subgraph",
        ),
        (
            truthy(field(datasets, "truncated")) || truthy(field(datasets, "subjects_truncated")),
            "dataset_match_budget",
        ),
        (
            truthy(field(source, "invalid_observations")),
            "invalid_observations",
        ),
        (
            truthy(field(source, "reference_reconciling_count")),
            "reference_source_reconciling",
        ),
        (
            truthy(field(source, "missing_tables")),
            "missing_local_tables",
        ),
    ] {
        if condition {
            reasons.insert(reason.to_owned());
        }
    }
    for row in list(field(entropy, "results")) {
        if text(row, "status") != "exact" {
            let reason = if truthy(field(row, "reason")) {
                text(row, "reason")
            } else {
                text(row, "status")
            };
            reasons.insert(format!("entropy_{reason}"));
        }
    }
    let missing = nodes
        .iter()
        .filter(|n| text(n, "status") == "missing")
        .count();
    let stale = nodes
        .iter()
        .filter(|n| text(n, "status") == "stale")
        .count();
    let conflicting = nodes
        .iter()
        .filter(|n| text(n, "status") == "conflicting")
        .count();
    let truncated = truthy(field(source, "budget_exhausted"))
        || finding_count > MAX_FINDINGS
        || truthy(field(entropy, "omitted"))
        || selected_available < available
        || reasons
            .iter()
            .any(|r| r.contains("budget") || r.contains("limit"));
    let partial = !reasons.is_empty()
        || missing > 0
        || stale > 0
        || conflicting > 0
        || !truthy(field(source, "complete"));
    let mut feature_count = 0;
    for node in &safe {
        feature_count += usize::from(truthy(field(
            &public.get("transaction_facts", text(node, "id"))?,
            "features",
        )));
    }
    let public_claims = public.label_count()?;
    let examined = text_number(field(analytics, "label_count"));
    let status = if truncated {
        "bounded"
    } else if partial {
        "partial"
    } else {
        "complete"
    };
    let mut checks = vec![
        check(
            "physical_topology",
            if partial { "partial" } else { "complete" },
            safe.len(),
            available,
        ),
        check("ownership_hypotheses", status, safe.len(), available),
        check(
            "structural_features",
            if feature_count < selected.len() {
                "partial"
            } else {
                "complete"
            },
            feature_count,
            selected.len(),
        ),
        check("multi_hop_patterns", status, safe.len(), available),
        check(
            "public_attribution",
            if public_claims == 0 {
                "unavailable"
            } else if truncated {
                "bounded"
            } else if partial || examined < public_claims {
                "partial"
            } else {
                "complete"
            },
            examined,
            public_claims,
        ),
        check(
            "conditional_entropy",
            if !truthy(field(entropy, "eligible")) {
                "not_applicable"
            } else if truthy(field(entropy, "omitted")) {
                "bounded"
            } else if list(field(entropy, "results"))
                .iter()
                .any(|r| text(r, "status") != "exact")
            {
                "partial"
            } else {
                "complete"
            },
            text_number(field(entropy, "evaluated")),
            text_number(field(entropy, "eligible")),
        ),
    ];
    if public_claims == 0 {
        set(&mut checks[4], "reason", s("no_local_public_claims"));
    }
    if safe.is_empty() {
        for check in &mut checks {
            if text(check, "code") != "conditional_entropy" {
                set(check, "status", s("unavailable"));
                set(check, "reason", s("no_analyzable_transactions"));
            }
        }
    }
    Ok(dict([
        (
            "status",
            s(if safe.is_empty() {
                "unavailable"
            } else if partial {
                "partial"
            } else {
                "complete"
            }),
        ),
        ("examined_transactions", count(safe.len())),
        ("available_transactions", count(available)),
        (
            "amount_complete_transactions",
            count(text_number(field(
                analytics,
                "amount_complete_transaction_count",
            ))),
        ),
        ("missing_nodes", count(missing)),
        ("stale_nodes", count(stale)),
        ("conflicting_nodes", count(conflicting)),
        ("truncated", Value::Bool(truncated)),
        ("stopped_reasons", strings(reasons)),
        ("checks", Value::List(checks)),
    ]))
}
fn check(code: &str, status: &str, evaluated: usize, eligible: usize) -> Value {
    dict([
        ("code", s(code)),
        ("status", s(status)),
        ("evaluated", count(evaluated)),
        ("eligible", count(eligible)),
    ])
}
fn snapshot(
    owner: &SqlView<'_>,
    deterministic: bool,
    elapsed: impl FnMut() -> f64,
) -> Result<Value, AppError> {
    let public = owner.for_observer("public")?;
    let analysis = analyze_snapshot(owner, &query())?;
    let selected: BTreeSet<_> = list(field(&analysis, "nodes"))
        .iter()
        .map(|n| text(n, "id").to_owned())
        .collect();
    let mut relevant = selected.clone();
    for id in &selected {
        let fact = owner.get("transaction_facts", id)?;
        for side in ["inputs", "outputs"] {
            relevant.extend(
                list(field(&fact, side))
                    .iter()
                    .map(|r| text(r, "output_id").to_owned()),
            );
        }
    }
    let owned = owner.owned_outputs_for_nodes(&relevant)?;
    let findings = findings(owner, &public, &owned, &analysis)?;
    let entropy = entropy(owner, &public, &owned, &selected, deterministic, elapsed)?;
    let coverage = coverage(&public, &analysis, &entropy, findings.len())?;
    let attention = findings
        .iter()
        .filter(|r| text(r, "severity") == "warning")
        .count();
    let owned_count = owner.owned_output_count()?;
    Ok(dict([
        ("payload_schema_version", count(2)),
        ("observer", s("public")),
        ("local_only", Value::Bool(true)),
        ("read_only", Value::Bool(true)),
        ("advisory_only", Value::Bool(true)),
        ("redaction", s("desktop_local")),
        ("investigation", investigation(&public, &[])?),
        (
            "summary",
            dict([
                (
                    "status",
                    s(
                        if owned_count == 0 || text(&coverage, "status") == "unavailable" {
                            "unavailable"
                        } else if attention > 0 {
                            "findings"
                        } else {
                            "no_observed_exposure"
                        },
                    ),
                ),
                ("finding_count", count(findings.len())),
                ("attention_count", count(attention)),
                ("owned_output_count", count(owned_count)),
                (
                    "analyzed_transaction_count",
                    field(&coverage, "examined_transactions").clone(),
                ),
                (
                    "local_transaction_count",
                    field(&coverage, "available_transactions").clone(),
                ),
                ("domain_count", count(owner.owned_domain_count()?)),
            ]),
        ),
        (
            "findings",
            Value::List(findings.into_iter().take(MAX_FINDINGS).collect()),
        ),
        ("coverage", coverage),
        ("entropy", entropy),
        ("assumptions", strings(ASSUMPTIONS)),
    ]))
}
/// Test builds only: `KASSIBER_RUST_TEST_ENTROPY_CLOCK=frozen` runs the
/// entropy evaluation on a clock that never advances, so a loaded machine
/// cannot cut its time budget short and the report reads the same every
/// run. Production always measures the wall clock.
fn frozen_entropy_clock() -> bool {
    #[cfg(any(test, feature = "test-harness"))]
    {
        std::env::var("KASSIBER_RUST_TEST_ENTROPY_CLOCK").as_deref() == Ok("frozen")
    }
    #[cfg(not(any(test, feature = "test-harness")))]
    {
        false
    }
}

pub fn build_local(
    conn: &kassiber_store::rusqlite::Connection,
    profile: &str,
) -> Result<Value, AppError> {
    super::views::with_read(conn, profile, "owner", |view| {
        snapshot(view, frozen_entropy_clock(), || 0.0)
    })
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn opaque(
    refs: &mut super::handles::References,
    scope: &super::handles::BookScope,
    value: &Value,
) -> Result<Value, AppError> {
    let value = value.as_str().ok_or_else(super::invalid)?;
    Ok(s(refs.issue(scope, value)?))
}
fn project_handoff(
    value: &Value,
    refs: &mut super::handles::References,
    scope: &super::handles::BookScope,
) -> Result<Value, AppError> {
    let mut query = dict([] as [(&str, Value); 0]);
    for (key, value) in super::fields(field(value, "query")) {
        match key {
            "subject" | "target" => set(&mut query, key, opaque(refs, scope, value)?),
            "mode" | "observer" | "direction" | "chain" | "network" | "depth" | "node_limit"
            | "edge_limit" | "include_relations" | "include_hypotheses" => {
                set(&mut query, key, value.clone())
            }
            _ => return Err(super::invalid()),
        }
    }
    Ok(dict([
        ("query", query),
        ("snapshot_id", field(value, "snapshot_id").clone()),
        ("ai_reference_scope", s("current_process_and_book")),
        (
            "ai_projection",
            s("opaque_graph_references_no_public_chain_identifiers"),
        ),
    ]))
}
fn project_claim(
    claim: &Value,
    refs: &mut super::handles::References,
    scope: &super::handles::BookScope,
) -> Result<Value, AppError> {
    let mut result = dict([] as [(&str, Value); 0]);
    let code = regex::Regex::new(r"\A[a-z][a-z0-9_]*(?:-[a-z0-9_]+)*\z").unwrap();
    let date = regex::Regex::new(r"\A[0-9T:.+Z-]{10,40}\z").unwrap();
    for (key, value) in super::fields(claim) {
        if ![
            "id",
            "revision",
            "category",
            "source",
            "confidence",
            "dataset_id",
            "dataset_version",
            "content_sha256",
            "valid_from",
            "valid_until",
            "visibility",
        ]
        .contains(&key)
        {
            continue;
        }
        let projected = match value {
            Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => Some(value.clone()),
            Value::Str(value) => {
                let Some(value) = value.as_str() else {
                    return Err(super::invalid());
                };
                match key {
                    "id" => Some(if uuid(value) {
                        s(value)
                    } else {
                        s(refs.issue(scope, value)?)
                    }),
                    "dataset_id" | "content_sha256" => Some(s(value)),
                    "source" => [
                        "stored_transaction",
                        "wallet_inventory",
                        "reference_cache",
                        "local_label",
                        "journal_custody_decisions",
                        "journal_custody_economic_relations",
                        "reviewed_or_imported_privacy_boundary",
                        "invalid_privacy_metadata",
                        "equal_output_coinjoin",
                        "large_equal_output_coinjoin",
                        "distinct_value_coinjoin",
                        "local_acquisition",
                        "observer_exclusion",
                        "psbt",
                        "psbt_proposal",
                        "psbt_supplied",
                        "local_dataset",
                    ]
                    .contains(&value)
                    .then(|| s(value)),
                    "category" | "confidence" | "visibility" => {
                        let identifier = value.len() == 64
                            && value
                                .bytes()
                                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
                        (value.len() <= 100
                            && code.is_match(value)
                            && !identifier
                            && !([
                                "bc1", "tb1", "bcrt1", "ex1", "lq1", "tex1", "tlq1", "ert1", "el1",
                                "xpub", "tpub", "ypub", "zpub",
                            ]
                            .iter()
                            .any(|prefix| value.starts_with(prefix))))
                        .then(|| s(value))
                    }
                    "valid_from" | "valid_until" => date.is_match(value).then(|| s(value)),
                    _ => None,
                }
            }
            _ => return Err(super::invalid()),
        };
        if let Some(value) = projected {
            set(&mut result, key, value);
        }
    }
    Ok(result)
}
fn project_report(
    payload: &Value,
    refs: &mut super::handles::References,
    scope: &super::handles::BookScope,
) -> Result<Value, AppError> {
    let mut result = payload.clone();
    set(&mut result, "redaction", s("ai_export_safe"));
    set(
        &mut result,
        "investigation",
        project_handoff(field(payload, "investigation"), refs, scope)?,
    );
    let mut findings = list(field(payload, "findings")).to_vec();
    for finding in &mut findings {
        let identity = text(finding, "id").to_owned();
        set(
            finding,
            "id",
            if uuid(&identity) {
                s(identity)
            } else {
                s(refs.issue(scope, &identity)?)
            },
        );
        let sources = list(field(finding, "source_ids"))
            .iter()
            .map(|v| opaque(refs, scope, v))
            .collect::<Result<Vec<_>, _>>()?;
        // Python pops `source_ids` and assigns it again, so it moves last.
        if let Value::Dict(pairs) = finding {
            pairs.retain(|(key, _)| *key != "source_ids".into());
            pairs.push(("source_ids".into(), Value::List(sources)));
        }
        let handoff = project_handoff(field(finding, "investigation"), refs, scope)?;
        set(finding, "investigation", handoff);
        if let Some(claim) = get(finding, "claim") {
            let claim = project_claim(claim, refs, scope)?;
            set(finding, "claim", claim);
        }
    }
    set(&mut result, "findings", Value::List(findings));
    let mut entropy = field(payload, "entropy").clone();
    let mut results = list(field(&entropy, "results")).to_vec();
    for row in &mut results {
        let subject = opaque(refs, scope, field(row, "subject"))?;
        let handoff = project_handoff(field(row, "investigation"), refs, scope)?;
        set(row, "subject", subject);
        set(row, "investigation", handoff);
    }
    set(&mut entropy, "results", Value::List(results));
    set(&mut result, "entropy", entropy);
    Ok(result)
}
pub fn build_redacted(
    conn: &kassiber_store::rusqlite::Connection,
    profile: &str,
) -> Result<Value, AppError> {
    super::views::with_read(conn, profile, "owner", |view| {
        let scope = view.book_scope()?;
        let payload = snapshot(view, frozen_entropy_clock(), || 0.0)?;
        super::handles::with_process_references(conn, |refs| project_report(&payload, refs, &scope))
    })
}

#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_fixture(
    case: &Value,
    conn: &kassiber_store::rusqlite::Connection,
) -> Result<Value, AppError> {
    super::views::with_read(conn, "p", "owner", |view| {
        let times = list(field(case, "elapsed_ms"));
        let mut cursor = 0;
        let elapsed = || {
            let v = times
                .get(cursor)
                .or_else(|| times.last())
                .map(|v| text_number(v) as f64)
                .unwrap_or(0.0);
            cursor += 1;
            v
        };
        let payload = snapshot(view, true, elapsed)?;
        if truthy(field(case, "redacted")) {
            project_report(
                &payload,
                &mut super::handles::References::from_key([7; 32]),
                &super::handles::BookScope::fixture("book-p"),
            )
        } else {
            Ok(payload)
        }
    })
}

#[cfg(test)]
mod tests;
