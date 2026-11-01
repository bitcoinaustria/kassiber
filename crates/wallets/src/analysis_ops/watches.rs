//! Durable local evidence watches: no transports, model calls or accounting
//! writes. A watch compares semantic observations, not graph hashes; caller
//! transactions advance the baseline and append inbox events together.
use super::*;
use crate::analysis_index::{durable, mirror, traversal::normalize_query, views};
use kassiber_setup::networks;

const RULE_VERSION: i64 = 1;
const RULES: [&str; 5] = [
    "output_spent",
    "confirmations",
    "connection_supported",
    "attribution_changed",
    "findings_changed",
];

fn py_text(value: &Value) -> String {
    match value {
        Value::Str(text) => text
            .code_points()
            .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
            .collect(),
        other => kassiber_envelope::py_str(other),
    }
}

fn int(value: i64) -> Value {
    Value::Int(value.into())
}

fn loads(value: &Value) -> Result<Value> {
    kassiber_pytext::loads(&py_text(value))
        .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// Plaintext SQLite opened by SQLCipher still reports a cipher version.
pub(super) fn require_encrypted(conn: &Connection) -> Result<()> {
    let column = |sql: &str| -> Result<Option<Value>> {
        Ok(first(conn, sql, [])?.map(|row| match row {
            Value::Dict(pairs) => pairs.into_iter().next().map_or(Value::Null, |(_, v)| v),
            _ => Value::Null,
        }))
    };
    let keyed = match column("PRAGMA cipher_version")? {
        Some(_) => column("PRAGMA cipher_salt")?,
        None => None,
    };
    if !keyed.is_some_and(|value| truthy(&value)) {
        return Err(AppError::new(
            "watch_requires_encrypted_database",
            "Encrypt this book before enabling background evidence work",
        )
        .with_hint("Open Settings > Lock & encryption, or run secrets init for this book"));
    }
    Ok(())
}

/// `db.database_instance_id`.
pub(super) fn database_instance_id(conn: &Connection) -> Result<Value> {
    let row = first(
        conn,
        "SELECT value FROM settings WHERE key = ?",
        ["database_instance_id"],
    )?;
    match row.as_ref().map(|row| field(row, "value")) {
        Some(Value::Str(text))
            if text.as_str().is_some_and(|v| {
                v.len() == 32 && v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            }) =>
        {
            Ok(Value::Str(text.clone()))
        }
        _ => Err(AppError::new(
            "invalid_project_database",
            "database instance identity is missing or invalid",
        )),
    }
}

fn network_name(network: &str) -> String {
    match network {
        "liquidv1" => "main",
        "liquidtestnet" => "test",
        "elementsregtest" => "regtest",
        other => other,
    }
    .into()
}

/// Python's `sorted(..., key=domain_id)` over homogeneous JSON scalars.
fn sort_domains(domains: &mut [Value]) -> Result<()> {
    let key = |value: &Value| field(value, "domain_id").clone();
    let mut failure = None;
    domains.sort_by(|a, b| match (key(a), key(b)) {
        (Value::Str(x), Value::Str(y)) => x.code_points().cmp(y.code_points()),
        (Value::Int(x), Value::Int(y)) => x.cmp(&y),
        (x, y) => {
            failure.get_or_insert_with(|| {
                format!(
                    "'<' not supported between instances of '{}' and '{}'",
                    kassiber_envelope::type_name(&y),
                    kassiber_envelope::type_name(&x)
                )
            });
            std::cmp::Ordering::Equal
        }
    });
    match failure {
        Some(message) => Err(AppError::new("internal_error", message)),
        None => Ok(()),
    }
}

pub(super) fn domain_for(conn: &Connection, profile: &str, query: &Value) -> Result<Value> {
    if truthy(field(query, "chain")) {
        return networks::require_chain_domain(
            conn,
            profile,
            &py_text(field(query, "chain")),
            field(query, "network"),
            None,
            "analysis",
        );
    }
    let binding = networks::resolve(conn, profile)?;
    if field(&binding, "state") != &s("bound") || !truthy(field(&binding, "domains")) {
        return Err(AppError::new(
            "book_network_unbound",
            "Bind this book to a network environment first",
        ));
    }
    // Generic saved investigations retain every bound domain; never narrow.
    let mut domains = Vec::new();
    if let Value::List(items) = field(&binding, "domains") {
        for domain in items {
            domains.push(networks::require_chain_domain(
                conn,
                profile,
                &py_text(field(domain, "chain")),
                field(domain, "network"),
                None,
                "analysis",
            )?);
        }
    }
    sort_domains(&mut domains)?;
    Ok(dict([
        ("scope", s("book_environment")),
        ("environment_id", field(&binding, "environment_id").clone()),
        ("revision", field(&binding, "revision").clone()),
        ("domains", Value::List(domains)),
    ]))
}

fn read<T>(
    conn: &Connection,
    profile: &str,
    observer: &str,
    sources: &mut dyn Sources,
    f: impl for<'v> FnOnce(&views::SqlView<'v>) -> Result<T>,
) -> Result<T> {
    atomic(conn, || {
        durable::synchronize_with_sources(conn, profile, false, sources)?;
        views::with_read(conn, profile, observer, f)
    })
}

/// `projection.index_revision`: the local watch cursor.
pub(super) fn revision_for(
    conn: &Connection,
    profile: &str,
    sources: &mut dyn Sources,
) -> Result<Value> {
    read(conn, profile, "owner", sources, |view| {
        Ok(dict([
            ("schema_version", int(1)),
            ("revision", field(view.state(), "revision").clone()),
            ("snapshot_id", field(view.state(), "snapshot_id").clone()),
            ("next_expiry", field(view.state(), "next_expiry").clone()),
        ]))
    })
}

pub(super) fn run_analysis(
    conn: &Connection,
    profile: &str,
    query: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    let query = normalize_query(query)?;
    let observer = py_text(field(&query, "observer"));
    read(conn, profile, &observer, sources, |view| {
        mirror::analyze_snapshot(view, &query)
    })
}

fn normalize_definition(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(args, &["rule", "query", "case_id", "threshold"], &["rule"])?;
    let rule = match field(args, "rule") {
        Value::Str(rule) if rule.as_str().is_some_and(|r| RULES.contains(&r)) => {
            rule.as_str().unwrap().to_owned()
        }
        _ => return Err(invalid("Unsupported watch rule")),
    };
    let case_id = field(args, "case_id");
    if truthy(case_id) == truthy(field(args, "query")) {
        return Err(invalid("Choose one investigation or query"));
    }
    let query = if truthy(case_id) {
        field(&cases::get_case(conn, profile, case_id)?, "query").clone()
    } else {
        field(args, "query").clone()
    };
    let mut query = normalize_query(&query)?;
    // A fully qualified physical subject declares its domain; a missing chain
    // in a generic saved query does not mean Bitcoin.
    let subject = py_text(get(&query, "subject").unwrap_or(&s("")));
    let parts: Vec<&str> = subject.split(':').collect();
    let physical = parts.len() >= 4
        && ["bitcoin", "liquid"].contains(&parts[0])
        && ["tx", "out"].contains(&parts[2]);
    if physical {
        let contradicts = match get(&query, "chain") {
            Some(chain) if chain != &s(parts[0]) => true,
            _ if truthy(field(&query, "network")) => {
                let normalize = |network: &str| {
                    crate::descriptors::normalize_network(parts[0], network).map_err(|error| {
                        match error {
                            crate::descriptors::Error::Value(message) => {
                                AppError::new("internal_error", message)
                            }
                            crate::descriptors::Error::App(error) => error,
                        }
                    })
                };
                normalize(&py_text(field(&query, "network")))? != normalize(parts[1])?
            }
            _ => false,
        };
        if contradicts {
            return Err(invalid("Watch subject contradicts its selected domain"));
        }
    }
    if !truthy(field(&query, "chain")) && physical {
        set(&mut query, "chain", s(parts[0]));
        if get(&query, "network").is_none() {
            set(&mut query, "network", s(network_name(parts[1]).as_str()));
        }
    }
    if truthy(field(&query, "chain")) && !truthy(field(&query, "network")) {
        let binding = networks::resolve(conn, profile)?;
        let chain = field(&query, "chain").clone();
        let domain = match field(&binding, "domains") {
            Value::List(items) => items.iter().find(|d| py_eq(field(d, "chain"), &chain)),
            _ => None,
        };
        let (true, Some(domain)) = (field(&binding, "state") == &s("bound"), domain) else {
            return Err(AppError::new(
                "book_network_unbound",
                "Bind this book to a compatible network environment first",
            ));
        };
        let network = match field(domain, "network") {
            Value::Str(text) => s(network_name(&py_text(&Value::Str(text.clone()))).as_str()),
            other => other.clone(),
        };
        set(&mut query, "network", network);
    }
    let mut query = normalize_query(&query)?;
    if ["output_spent", "confirmations", "attribution_changed"].contains(&rule.as_str())
        && !truthy(field(&query, "subject"))
    {
        return Err(invalid("This watch requires a subject"));
    }
    if rule == "connection_supported" && field(&query, "mode") != &s("path") {
        return Err(invalid("A connection watch requires a path query"));
    }
    if ["output_spent", "confirmations"].contains(&rule.as_str()) {
        for (key, value) in [
            ("mode", s("trace")),
            ("direction", s("both")),
            ("depth", int(2)),
            ("include_relations", Value::Bool(false)),
            ("include_hypotheses", Value::Bool(false)),
        ] {
            set(&mut query, key, value);
        }
        remove(&mut query, "target");
    }
    let mut definition = dict([
        ("rule", s(rule.as_str())),
        ("rule_version", int(RULE_VERSION)),
        ("query", query),
    ]);
    if truthy(case_id) {
        set(
            &mut definition,
            "case_id",
            Value::Str(text_value(case_id, "case_id", 64)?),
        );
    }
    if rule == "confirmations" {
        let threshold = get(args, "threshold").cloned().unwrap_or(int(1));
        match &threshold {
            Value::Int(n) if *n >= 1.into() && *n <= 10000.into() => {}
            _ => {
                return Err(invalid(
                    "Confirmation threshold must be between 1 and 10000",
                ))
            }
        }
        set(&mut definition, "threshold", threshold);
    } else if get(args, "threshold").is_some() {
        return Err(invalid("Only confirmation watches accept a threshold"));
    }
    Ok(definition)
}

fn unavailable(reason: &str) -> Value {
    dict([
        ("status", s("unavailable")),
        ("reason", s(reason)),
        ("values", Value::List(vec![])),
    ])
}

fn observed(status: &str, values: Vec<Value>) -> Value {
    dict([("status", s(status)), ("values", Value::List(values))])
}

fn sorted_text(values: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut values: Vec<Value> = values.into_iter().collect();
    values.sort_by(|a, b| match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.code_points().cmp(y.code_points()),
        _ => std::cmp::Ordering::Equal,
    });
    values.dedup_by(|a, b| a == b);
    values
}

fn list_of(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => &[],
    }
}

/// Evidence disappearance is unknown, never evidence of absence.
fn observe(
    conn: &Connection,
    profile: &str,
    definition: &Value,
    previewing: bool,
    sources: &mut dyn Sources,
) -> Result<Value> {
    if !py_eq(field(definition, "rule_version"), &int(RULE_VERSION)) {
        return Ok(unavailable("rule_version_changed"));
    }
    let query = field(definition, "query");
    let result = match run_analysis(conn, profile, query, sources) {
        Ok(result) => result,
        Err(error) if error.code == "subject_ambiguous" && previewing => return Err(error),
        Err(error) if ["not_found", "subject_ambiguous"].contains(&error.code.as_str()) => {
            return Ok(unavailable("subject_unavailable"))
        }
        Err(error) => return Err(error),
    };
    let rule = field(definition, "rule");
    let subject = get(query, "subject").cloned().unwrap_or(Value::Null);
    let all_nodes = list_of(field(&result, "nodes"));
    let nodes: Vec<&Value> = all_nodes
        .iter()
        .filter(|node| {
            let txid = if field(node, "kind") == &s("transaction") {
                get(node, "txid").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            [
                field(node, "id").clone(),
                txid,
                get(node, "outpoint").cloned().unwrap_or(Value::Null),
                get(node, "address").cloned().unwrap_or(Value::Null),
            ]
            .iter()
            .any(|candidate| py_eq(candidate, &subject))
        })
        .collect();
    let status_in = |node: &Value, statuses: &[&str]| {
        statuses
            .iter()
            .any(|status| field(node, "status") == &s(*status))
    };
    let trusted: Vec<&Value> = all_nodes
        .iter()
        .filter(|node| !status_in(node, &["missing", "stale", "conflicting", "retracted"]))
        .map(|node| field(node, "id"))
        .collect();
    let complete = matches!(
        get(field(&result, "coverage"), "complete"),
        Some(Value::Bool(true))
    );
    if rule == &s("output_spent") {
        if nodes.len() != 1 || field(nodes[0], "kind") != &s("output") {
            return Ok(unavailable("output_required"));
        }
        // A spends edge is positive evidence unless its source is stale.
        let mut invalid_nodes: Vec<Value> = all_nodes
            .iter()
            .filter(|node| status_in(node, &["stale", "conflicting", "retracted"]))
            .map(|node| field(node, "id").clone())
            .collect();
        let edges = list_of(field(&result, "edges"));
        for edge in edges {
            if field(edge, "kind") == &s("creates") && invalid_nodes.contains(field(edge, "source"))
            {
                invalid_nodes.push(field(edge, "target").clone());
            }
        }
        let subject_id = field(nodes[0], "id");
        let values = sorted_text(
            edges
                .iter()
                .filter(|edge| {
                    let (source, target) = (field(edge, "source"), field(edge, "target"));
                    !invalid_nodes.contains(target)
                        && !invalid_nodes.contains(source)
                        && field(edge, "kind") == &s("spends")
                        && source == subject_id
                        && trusted.contains(&source)
                        && !status_in(edge, &["stale", "conflicting"])
                })
                .map(|edge| field(edge, "target").clone()),
        );
        let status = if values.is_empty() {
            "unknown"
        } else {
            "observed"
        };
        return Ok(observed(status, values));
    }
    if rule == &s("confirmations") {
        let values: Vec<&Value> = nodes
            .iter()
            .filter(|node| trusted.contains(&field(node, "id")))
            .map(|node| field(node, "confirmations"))
            .collect();
        let known = match values.as_slice() {
            [Value::Int(n)] if n >= &kassiber_pyjson::BigInt::from(0) => Some(n),
            _ => None,
        };
        return Ok(match known {
            Some(n) => {
                let Value::Int(threshold) = field(definition, "threshold") else {
                    return Err(AppError::new(
                        "internal_error",
                        "'>=' not supported for this confirmation threshold",
                    ));
                };
                observed("observed", vec![Value::Bool(n >= threshold)])
            }
            None => observed("unknown", vec![]),
        });
    }
    if rule == &s("connection_supported") {
        return Ok(if truthy(field(&result, "paths")) {
            observed("observed", vec![Value::Bool(true)])
        } else {
            observed("unknown", vec![])
        });
    }
    let mut values = Vec::new();
    if rule == &s("attribution_changed") {
        // Only semantic claim fields count, never ingestion timestamps.
        for exposure in list_of(field(&result, "exposure")) {
            if field(exposure, "kind") == &s("direct_label_match") {
                let claim = field(exposure, "claim");
                values.push(s(canonical(&dict(
                    [
                        "subject",
                        "label",
                        "category",
                        "confidence",
                        "source",
                        "dataset_id",
                        "visibility",
                    ]
                    .map(|key| (key, get(claim, key).cloned().unwrap_or(Value::Null))),
                ))?
                .as_str()));
            }
        }
    } else {
        for item in list_of(field(&result, "findings")) {
            let node_ids = sorted_text(list_of(field(item, "node_ids")).iter().cloned());
            values.push(s(canonical(&dict([
                ("code", field(item, "code").clone()),
                ("node_ids", Value::List(node_ids)),
                (
                    "rule_version",
                    get(item, "rule_version").cloned().unwrap_or(Value::Null),
                ),
            ]))?
            .as_str()));
        }
    }
    Ok(observed(
        if complete { "observed" } else { "partial" },
        sorted_text(values),
    ))
}

fn digest(value: &Value) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(Sha256::digest(canonical(value)?.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn preview(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    require_encrypted(conn)?;
    atomic(conn, || {
        let definition = normalize_definition(conn, profile, args)?;
        let domain = domain_for(conn, profile, field(&definition, "query"))?;
        let revision = revision_for(conn, profile, sources)?;
        let observation = observe(conn, profile, &definition, true, sources)?;
        let mut plan = dict([
            ("definition", definition),
            ("domain", domain),
            ("book_id", database_instance_id(conn)?),
            ("profile_id", s(profile)),
            ("revision", revision),
            ("baseline", observation),
        ]);
        let plan_id = digest(&plan)?;
        set(&mut plan, "plan_id", s(plan_id.as_str()));
        set(&mut plan, "local_only", Value::Bool(true));
        set(&mut plan, "egress", s("none"));
        Ok(plan)
    })
}

fn recipe(supplied: &Value, query: Value) -> Value {
    let mut recipe = dict(
        ["rule", "threshold"]
            .into_iter()
            .filter_map(|key| get(supplied, key).map(|v| (key, v.clone()))),
    );
    if truthy(field(supplied, "case_id")) {
        set(&mut recipe, "case_id", field(supplied, "case_id").clone());
    } else {
        set(&mut recipe, "query", query);
    }
    recipe
}

fn changed() -> AppError {
    stale("Evidence or book changed; preview this watch again")
}

fn create(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(args, &["plan", "definition", "expected_plan_id"], &[])?;
    if get(args, "plan").is_none() {
        arguments(
            args,
            &["definition", "expected_plan_id"],
            &["definition", "expected_plan_id"],
        )?;
        return atomic(conn, || {
            let supplied = field(args, "definition");
            let normalized =
                matches!(supplied, Value::Dict(_)) && get(supplied, "rule_version").is_some();
            let mut fields = supplied.clone();
            if normalized {
                arguments(
                    supplied,
                    &["rule", "rule_version", "query", "case_id", "threshold"],
                    &["rule", "rule_version", "query"],
                )?;
                if !matches!(field(supplied, "rule_version"), Value::Int(n) if n == &kassiber_pyjson::BigInt::from(RULE_VERSION))
                {
                    return Err(stale("Watch rule version changed; preview it again"));
                }
                fields = recipe(supplied, field(supplied, "query").clone());
            }
            let plan = preview(conn, profile, &fields, sources)?;
            if (normalized && canonical(field(&plan, "definition"))? != canonical(supplied)?)
                || !py_eq(field(&plan, "plan_id"), field(args, "expected_plan_id"))
            {
                return Err(changed());
            }
            create(conn, profile, &dict([("plan", plan)]), sources)
        });
    }
    arguments(args, &["plan"], &["plan"])?;
    let plan = field(args, "plan");
    if !matches!(plan, Value::Dict(_)) || !matches!(get(plan, "definition"), Some(Value::Dict(_))) {
        return Err(invalid("A reviewed watch preview is required"));
    }
    let supplied = field(plan, "definition");
    let fields = recipe(
        supplied,
        get(supplied, "query").cloned().unwrap_or(Value::Null),
    );
    atomic(conn, || {
        let current = preview(conn, profile, &fields, sources)?;
        if canonical(&current)? != canonical(plan)? {
            return Err(changed());
        }
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chain_analysis_watches WHERE profile_id=?",
                [profile],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if count >= 250 {
            return Err(invalid("This book has reached its 250 watch limit"));
        }
        let definition = canonical(field(&current, "definition"))?;
        let domain = canonical(field(&current, "domain"))?;
        let existing = first(
            conn,
            "SELECT id FROM chain_analysis_watches WHERE profile_id=? AND definition_json=? AND domain_json=?",
            params![profile, definition, domain],
        )?;
        if let Some(existing) = existing {
            let watch = get_watch(conn, profile, field(&existing, "id"))?;
            if !truthy(field(&watch, "enabled")) {
                return Err(AppError::new(
                    "watch_already_exists",
                    "This watch already exists and is paused; resume it in Watches",
                ));
            }
            return Ok(watch);
        }
        let id = sources.id();
        let now = sources.now();
        conn.execute(
            "INSERT INTO chain_analysis_watches(id,profile_id,book_id,domain_json,definition_json,baseline_json,checkpoint,checked_at,created_at) VALUES(?,?,?,?,?,?,?,?,?)",
            params![
                id,
                profile,
                sql(field(&current, "book_id"), 3)?,
                domain,
                definition,
                canonical(field(&current, "baseline"))?,
                canonical(field(&current, "revision"))?,
                now,
                now
            ],
        )
        .map_err(database_error)?;
        get_watch(conn, profile, &s(id))
    })
}

fn watch(row: &Value) -> Result<Value> {
    Ok(dict([
        ("id", field(row, "id").clone()),
        ("definition", loads(field(row, "definition_json"))?),
        ("enabled", Value::Bool(truthy(field(row, "enabled")))),
        ("revision", field(row, "revision").clone()),
        ("baseline", loads(field(row, "baseline_json"))?),
        ("checked_at", field(row, "checked_at").clone()),
        ("created_at", field(row, "created_at").clone()),
    ]))
}

fn get_watch(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    let id = Value::Str(text_value(id, "id", 64)?);
    let row = first(
        conn,
        "SELECT * FROM chain_analysis_watches WHERE profile_id=? AND id=?",
        params![profile, sql(&id, 2)?],
    )?
    .ok_or_else(|| AppError::new("not_found", "Watch not found in this book"))?;
    watch(&row)
}

fn list(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(args, &[], &[])?;
    let items = crate::publication::row::rows(
        conn,
        "SELECT * FROM chain_analysis_watches WHERE profile_id=? ORDER BY created_at,id",
        [profile],
    )?
    .iter()
    .map(watch)
    .collect::<Result<Vec<_>>>()?;
    Ok(dict([
        ("items", Value::List(items)),
        ("local_only", Value::Bool(true)),
    ]))
}

fn same_revision(args: &Value, watch: &Value) -> bool {
    matches!(field(args, "expected_revision"), Value::Int(n) if py_eq(field(watch, "revision"), &Value::Int(n.clone())))
}

fn configure(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(
        args,
        &["id", "expected_revision", "enabled"],
        &["id", "expected_revision", "enabled"],
    )?;
    let Value::Bool(enabled) = field(args, "enabled") else {
        return Err(invalid("enabled must be boolean"));
    };
    atomic(conn, || {
        let old = get_watch(conn, profile, field(args, "id"))?;
        if !same_revision(args, &old) {
            return Err(stale("Watch changed; reload it"));
        }
        conn.execute(
            "UPDATE chain_analysis_watches SET enabled=?,revision=revision+1,checkpoint='' WHERE id=?",
            params![i64::from(*enabled), sql(field(args, "id"), 2)?],
        )
        .map_err(database_error)?;
        get_watch(conn, profile, field(args, "id"))
    })
}

fn delete(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(
        args,
        &["id", "expected_revision"],
        &["id", "expected_revision"],
    )?;
    atomic(conn, || {
        let watch = get_watch(conn, profile, field(args, "id"))?;
        if !same_revision(args, &watch) {
            return Err(stale("Watch changed; reload it"));
        }
        // Explicit cleanup also removes local notification history.
        let id = sql(field(args, "id"), 1)?;
        conn.execute(
            "DELETE FROM chain_analysis_watch_inbox WHERE watch_id=? AND profile_id=?",
            params![id, profile],
        )
        .map_err(database_error)?;
        conn.execute(
            "DELETE FROM chain_analysis_watches WHERE id=? AND profile_id=?",
            params![id, profile],
        )
        .map_err(database_error)?;
        Ok(dict([
            ("id", field(args, "id").clone()),
            ("deleted", Value::Bool(true)),
        ]))
    })
}

fn transition(before: &Value, after: &Value, rule: &Value) -> Result<Option<&'static str>> {
    if py_eq(before, after) {
        return Ok(None);
    }
    let set_of = |value: &Value| -> Result<Vec<String>> {
        let mut items = list_of(field(value, "values"))
            .iter()
            .map(canonical)
            .collect::<Result<Vec<_>>>()?;
        items.sort();
        items.dedup();
        Ok(items)
    };
    let (previous, current) = (set_of(before)?, set_of(after)?);
    let lost = |value: &Value| {
        [s("unavailable"), s("unknown")]
            .iter()
            .any(|status| py_eq(field(value, "status"), status))
    };
    if lost(after) && !lost(before) {
        return Ok(Some("coverage_lost"));
    }
    let only_true = |value: &Value| {
        py_eq(
            field(value, "values"),
            &Value::List(vec![Value::Bool(true)]),
        )
    };
    if rule == &s("confirmations")
        && py_eq(field(after, "status"), &s("observed"))
        && !py_eq(field(before, "values"), field(after, "values"))
    {
        return Ok(if only_true(after) {
            Some("threshold_reached")
        } else if only_true(before) {
            Some("threshold_reversed")
        } else {
            None
        });
    }
    if current.iter().any(|item| !previous.contains(item)) {
        return Ok(match rule {
            Value::Str(rule) => match rule.as_str() {
                Some("output_spent") => Some("spend_observed"),
                Some("connection_supported") => Some("connection_supported"),
                Some("attribution_changed") => Some("attribution_changed"),
                Some("findings_changed") => Some("findings_changed"),
                _ => None,
            },
            _ => None,
        });
    }
    if previous.iter().any(|item| !current.contains(item)) {
        return Ok(Some(if py_eq(field(after, "status"), &s("observed")) {
            "evidence_changed"
        } else {
            "coverage_lost"
        }));
    }
    if py_eq(field(before, "status"), &s("observed"))
        && py_eq(field(after, "status"), &s("partial"))
    {
        return Ok(Some("coverage_lost"));
    }
    Ok(None)
}

pub(super) fn evaluate_due(
    conn: &Connection,
    profile: &str,
    cancelled: &dyn Fn() -> bool,
    sources: &mut dyn Sources,
) -> Result<Value> {
    require_encrypted(conn)?;
    let cancellation = || AppError::new("cancelled", "Local watch evaluation cancelled");
    let mut emitted = 0;
    atomic(conn, || {
        let revision = revision_for(conn, profile, sources)?;
        let checkpoint = canonical(&revision)?;
        for row in crate::publication::row::rows(
            conn,
            "SELECT * FROM chain_analysis_watches WHERE profile_id=? AND enabled=1 ORDER BY id",
            [profile],
        )? {
            if cancelled() {
                return Err(cancellation());
            }
            let definition = loads(field(&row, "definition_json"))?;
            // Resolve every tick: rebinding must never continue under another domain.
            let valid = match domain_for(conn, profile, field(&definition, "query")) {
                Ok(domain) => {
                    py_eq(&domain, &loads(field(&row, "domain_json"))?)
                        && database_instance_id(conn)
                            .is_ok_and(|book| py_eq(field(&row, "book_id"), &book))
                }
                Err(_) => false,
            };
            if valid && field(&row, "checkpoint") == &s(checkpoint.as_str()) {
                continue;
            }
            let after = if valid {
                observe(conn, profile, &definition, false, sources)?
            } else {
                unavailable("domain_changed")
            };
            let before = loads(field(&row, "baseline_json"))?;
            let code = transition(&before, &after, field(&definition, "rule"))?;
            let Value::Int(sequence) = field(&row, "sequence") else {
                return Err(AppError::new(
                    "internal_error",
                    "Stored watch sequence is invalid",
                ));
            };
            let sequence = Value::Int(sequence.clone() + i64::from(code.is_some()));
            if let Some(code) = code {
                let id = sources.id();
                let now = sources.now();
                conn.execute(
                    "INSERT INTO chain_analysis_watch_inbox(id,watch_id,profile_id,sequence,code,observation_json,created_at) VALUES(?,?,?,?,?,?,?)",
                    params![
                        id,
                        sql(field(&row, "id"), 2)?,
                        profile,
                        sql(&sequence, 4)?,
                        code,
                        canonical(&dict([("before", before.clone()), ("after", after.clone())]))?,
                        now
                    ],
                )
                .map_err(database_error)?;
                let shared = std::cell::RefCell::new(&mut *sources);
                crate::publication::reports::record_watch_change(
                    conn,
                    profile,
                    &definition,
                    &after,
                    code,
                    &mut || shared.borrow_mut().id(),
                    &mut || shared.borrow_mut().now(),
                )?;
                emitted += 1;
            }
            conn.execute(
                "UPDATE chain_analysis_watches SET baseline_json=?,checkpoint=?,sequence=?,checked_at=? WHERE id=?",
                params![
                    canonical(&after)?,
                    if valid { checkpoint.as_str() } else { "" },
                    sql(&sequence, 3)?,
                    sources.now(),
                    sql(field(&row, "id"), 5)?
                ],
            )
            .map_err(database_error)?;
        }
        if cancelled() {
            return Err(cancellation());
        }
        Ok(())
    })?;
    Ok(dict([
        ("evaluated", Value::Bool(true)),
        ("event_count", int(emitted)),
        ("local_only", Value::Bool(true)),
    ]))
}

fn inbox(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(args, &["limit", "before"], &[])?;
    let limit = match get(args, "limit").cloned().unwrap_or(int(50)) {
        Value::Int(n) => i64::try_from(n).ok().filter(|n| (1..=100).contains(n)),
        _ => None,
    }
    .ok_or_else(|| invalid("limit must be between 1 and 100"))?;
    // Monotonic SQLite rowid survives equal timestamps, UUID order and clock jumps.
    let before = get(args, "before").cloned().unwrap_or(int(i64::MAX));
    let before = match &before {
        Value::Int(n) if n >= &kassiber_pyjson::BigInt::from(1) => sql(&before, 2)?,
        _ => return Err(invalid("before must be a positive inbox cursor")),
    };
    let rows = crate::publication::row::rows(
        conn,
        "SELECT rowid AS cursor,* FROM chain_analysis_watch_inbox WHERE profile_id=? AND rowid<? ORDER BY rowid DESC LIMIT ?",
        params![profile, before, limit + 1],
    )?;
    let mut items = Vec::new();
    for row in rows.iter().take(limit as usize) {
        let mut item = dict(
            [
                "id",
                "watch_id",
                "code",
                "created_at",
                "acknowledged_at",
                "cursor",
            ]
            .map(|key| (key, field(row, key).clone())),
        );
        set(
            &mut item,
            "observation",
            loads(field(row, "observation_json"))?,
        );
        items.push(item);
    }
    for item in &mut items {
        if let Some(impact) =
            crate::publication::reports::inbox_report_impact(conn, profile, field(item, "id"))?
        {
            set(item, "report_impact", impact);
        }
    }
    let unread: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chain_analysis_watch_inbox WHERE profile_id=? AND acknowledged_at IS NULL",
            [profile],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    let next = if rows.len() as i64 > limit {
        field(items.last().unwrap(), "cursor").clone()
    } else {
        Value::Null
    };
    Ok(dict([
        ("items", Value::List(items)),
        ("next_cursor", next),
        ("unread_count", int(unread)),
    ]))
}

fn acknowledge(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(args, &["id"], &["id"])?;
    let id = Value::Str(text_value(field(args, "id"), "id", 64)?);
    // Python's driver opens a deferred transaction before this bare UPDATE;
    // a missing event leaves it open for the caller, exactly as there.
    if conn.is_autocommit() {
        conn.execute_batch("BEGIN").map_err(database_error)?;
    }
    let changed = conn
        .execute(
            "UPDATE chain_analysis_watch_inbox SET acknowledged_at=COALESCE(acknowledged_at,?) WHERE profile_id=? AND id=?",
            params![sources.now(), profile, sql(&id, 3)?],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err(AppError::new(
            "not_found",
            "Inbox event not found in this book",
        ));
    }
    Ok(dict([("id", id), ("acknowledged", Value::Bool(true))]))
}

pub(super) fn dispatch(
    conn: &Connection,
    profile: &str,
    operation: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    match operation {
        "preview" => preview(conn, profile, args, sources),
        "create" => create(conn, profile, args, sources),
        "list" => list(conn, profile, args),
        "configure" => configure(conn, profile, args),
        "delete" => delete(conn, profile, args),
        "inbox" => inbox(conn, profile, args),
        "acknowledge" => acknowledge(conn, profile, args, sources),
        _ => {
            arguments(args, &[], &[])?;
            evaluate_due(conn, profile, &|| false, sources)
        }
    }
}
