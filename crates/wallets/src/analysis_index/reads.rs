use crate::analysis_index::{self as engine, field, list, remove, Text};
use crate::database_error;
use crate::publication::{row::rows, wallet::Sources};
use engine::views::GraphRead;
use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};
use std::collections::BTreeMap;

mod cursor;
mod datasets;
#[cfg(any(test, feature = "test-harness"))]
pub mod fixture;
#[cfg(test)]
mod tests;

type Result<T> = std::result::Result<T, AppError>;

pub const OPERATIONS: &[&str] = &[
    "query",
    "entropy",
    "cases.list",
    "cases.get",
    "cases.compare",
    "labels.list",
    "datasets.list",
    "datasets.get",
    "datasets.query",
];

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    field(v, key).as_str().unwrap_or("")
}
#[cfg(test)]
fn json(v: &Value) -> Result<String> {
    dumps(
        v,
        &Options::new().sort_keys(true).compact().allow_nan(false),
    )
    .map_err(|_| invalid("Invalid chain analysis JSON document"))
}
fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Dict(a), Value::Dict(b)) => {
            a.len() == b.len()
                && a.iter().all(|(k, v)| {
                    b.iter()
                        .find(|(key, _)| key == k)
                        .is_some_and(|(_, other)| equivalent(v, other))
                })
        }
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b))
        }
        (Value::Bool(a), Value::Int(b)) | (Value::Int(b), Value::Bool(a)) => {
            *b == i64::from(*a).into()
        }
        (Value::Int(a), Value::Float(b)) | (Value::Float(b), Value::Int(a)) => {
            b.is_finite()
                && b.fract() == 0.0
                && kassiber_pytext::parse_int(&format!("{b:.0}")).is_ok_and(|n| n == *a)
        }
        (Value::Bool(a), Value::Float(b)) | (Value::Float(b), Value::Bool(a)) => {
            *b == f64::from(u8::from(*a))
        }
        _ => a == b,
    }
}
fn arguments(args: &Value, allowed: &[&str], required: &[&str]) -> Result<()> {
    let Value::Dict(fields) = args else {
        return Err(invalid("Unsupported or missing chain analysis arguments"));
    };
    if fields
        .iter()
        .any(|(k, _)| !k.as_str().is_some_and(|k| allowed.contains(&k)))
        || required.iter().any(|key| get(args, key).is_none())
    {
        return Err(invalid("Unsupported or missing chain analysis arguments"));
    }
    Ok(())
}
fn text_value<'a>(v: &'a Value, name: &str, max: usize) -> Result<&'a str> {
    v.as_str()
        .filter(|v| {
            !kassiber_pytext::strip(v).is_empty()
                && v.chars().count() <= max
                && !v.chars().any(|c| c < ' ')
        })
        .map(kassiber_pytext::strip)
        .ok_or_else(|| {
            invalid(format!(
                "{name} must be nonempty text up to {max} characters"
            ))
        })
}
fn limit(args: &Value, name: &str, default: i64, max: i64, message: &str) -> Result<i64> {
    let v = get(args, name)
        .cloned()
        .unwrap_or(Value::Int(default.into()));
    if let Value::Int(v) = v {
        if let Ok(n) = i64::try_from(v) {
            if (1..=max).contains(&n) {
                return Ok(n);
            }
        }
    }
    Err(invalid(message))
}
fn atomic<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT chain_analysis_reads")
        .map_err(database_error)?;
    let result = f();
    if result.is_err() {
        conn.execute_batch("ROLLBACK TO chain_analysis_reads")
            .map_err(database_error)?;
    }
    conn.execute_batch("RELEASE chain_analysis_reads")
        .map_err(database_error)?;
    result
}

pub fn execute(
    conn: &Connection,
    workspace: &str,
    profile: &str,
    operation: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    if !OPERATIONS.contains(&operation) {
        return Err(AppError::new(
            "not_implemented",
            "Unsupported chain analysis operation",
        ));
    }
    if !matches!(args, Value::Dict(_)) {
        return Err(invalid("Chain analysis arguments must be an object"));
    }
    let mut args = args.clone();
    if let Some(expected) = get(&args, "expected_scope") {
        if *expected != Value::Null
            && !(matches!(expected, Value::Dict(fields) if fields.len() == 2)
                && field(expected, "workspace_id") == &s(workspace)
                && field(expected, "profile_id") == &s(profile))
        {
            return Err(AppError::new(
                "scope_changed",
                "The active book changed; reopen the investigation in its original book",
            ));
        }
    }
    remove(&mut args, "expected_scope");
    match operation {
        "query" => query(conn, profile, &args, sources),
        "entropy" => entropy(conn, profile, &args, sources),
        "cases.list" => page(conn, profile, &args, false),
        "cases.get" => {
            arguments(&args, &["id"], &["id"])?;
            get_case(conn, profile, field(&args, "id"))
        }
        "cases.compare" => compare(conn, profile, &args, sources),
        "labels.list" => {
            arguments(&args, &[], &[])?;
            let items = rows(conn, "SELECT * FROM chain_analysis_labels WHERE profile_id=? AND deleted=0 ORDER BY chain,network,subject,id", [profile])?
                .into_iter().map(|mut row| {
                    for key in ["cluster_defining", "deleted"] {
                        let v = Value::Bool(truthy(field(&row, key)));
                        set(&mut row, key, v);
                    }
                    remove(&mut row, "profile_id");
                    row
                }).collect();
            Ok(dict([("items", Value::List(items))]))
        }
        "datasets.list" => page(conn, profile, &args, true),
        "datasets.get" => {
            arguments(&args, &["id"], &["id"])?;
            datasets::get_pack(conn, profile, field(&args, "id"))
        }
        "datasets.query" => datasets::query(conn, profile, &args),
        _ => unreachable!(),
    }
}

fn query(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    let query = engine::traversal::normalize_query(args)?;
    atomic(conn, || {
        engine::durable::synchronize_with_sources(conn, profile, false, sources)?;
        engine::views::with_read(conn, profile, text(&query, "observer"), |view| {
            engine::mirror::analyze_snapshot(view, &query)
        })
    })
}

fn entropy(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(
        args,
        &[
            "subject",
            "chain",
            "network",
            "observer",
            "max_states",
            "max_duration_ms",
            "scenario",
        ],
        &[],
    )
    .map_err(|_| invalid("Unsupported entropy arguments"))?;
    let mut query = dict([("mode", s("trace"))]);
    for key in ["subject", "chain", "network", "observer"] {
        if let Some(v) = get(args, key) {
            set(&mut query, key, v.clone());
        }
    }
    let query = engine::traversal::normalize_query(&query)?;
    let states = Value::Int(
        limit(
            args,
            "max_states",
            200000,
            2000000,
            "max_states must be between 1 and 2000000",
        )?
        .into(),
    );
    let duration = Value::Int(
        limit(
            args,
            "max_duration_ms",
            1000,
            30000,
            "max_duration_ms must be between 1 and 30000",
        )?
        .into(),
    );
    let scenario =
        engine::entropy::normalize_scenario(field(args, "scenario")).map_err(|reason| {
            invalid("Invalid entropy scenario").with_details(dict([("reason", s(reason))]))
        })?;
    atomic(conn, || {
        engine::durable::synchronize_with_sources(conn, profile, false, sources)?;
        engine::views::with_read(conn, profile, text(&query, "observer"), |view| {
            let mut context = dict([
                ("schema_version", Value::Int(1.into())),
                ("snapshot_id", view.snapshot_id()),
                ("subject", field(&query, "subject").clone()),
                ("observer", field(&query, "observer").clone()),
            ]);
            let subjects =
                match engine::traversal::resolve_read(view, text(&query, "subject"), &query) {
                    Ok(v) => v,
                    Err(e) if e.code == "subject_ambiguous" => {
                        set(&mut context, "status", s("unavailable"));
                        set(&mut context, "reason", s("ambiguous_domain"));
                        return Ok(context);
                    }
                    Err(e) => return Err(e),
                };
            if subjects.len() != 1
                || field(&view.get("nodes", &subjects[0])?, "kind") != &s("transaction")
            {
                set(&mut context, "status", s("unavailable"));
                set(
                    &mut context,
                    "reason",
                    s("single_physical_transaction_required"),
                );
                return Ok(context);
            }
            let id = &subjects[0];
            set(&mut context, "subject", s(id.as_str()));
            let node = view.get("nodes", id)?;
            let facts = view.get("transaction_facts", id)?;
            let result = engine::entropy::analyze_transaction_entropy(
                &facts,
                text(&node, "chain"),
                &states,
                &duration,
                &scenario,
                engine::entropy::Hooks::default(),
            );
            if let Value::Dict(fields) = result {
                for (k, v) in fields {
                    set(&mut context, k.as_str().unwrap(), v);
                }
            }
            Ok(context)
        })
    })
}

fn case(row: &Value, detail: bool) -> Result<Value> {
    let failed = || {
        AppError::new(
            "integrity_error",
            "Stored investigation failed its content integrity check",
        )
    };
    let result = kassiber_pytext::loads(text(row, "result_json")).map_err(|_| failed())?;
    let query = kassiber_pytext::loads(text(row, "query_json")).map_err(|_| failed())?;
    if !matches!(result, Value::Dict(_))
        || !equivalent(&query, field(&result, "query"))
        || field(row, "snapshot_id") != field(&result, "snapshot_id")
        || engine::digest(&result).map_err(|_| failed())? != text(row, "result_digest")
    {
        return Err(failed());
    }
    let mut value =
        dict(["id", "title", "created_at", "snapshot_id"].map(|k| (k, field(row, k).clone())));
    set(&mut value, "query", query);
    set(
        &mut value,
        "summary",
        get(&result, "summary")
            .cloned()
            .unwrap_or_else(|| dict::<&str>([])),
    );
    if detail {
        set(&mut value, "result", result);
    }
    Ok(value)
}
fn get_case(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    text_value(id, "id", 64)?;
    let found = rows(
        conn,
        "SELECT * FROM chain_analysis_cases WHERE profile_id=? AND id=?",
        [profile, id.as_str().unwrap()],
    )?;
    case(
        found
            .first()
            .ok_or_else(|| AppError::new("not_found", "Investigation not found in this book"))?,
        true,
    )
}

fn page(conn: &Connection, profile: &str, args: &Value, dataset: bool) -> Result<Value> {
    arguments(args, &["limit", "cursor"], &[]).map_err(|e| {
        if dataset {
            invalid("Unsupported dataset list arguments")
        } else {
            e
        }
    })?;
    let size = limit(
        args,
        "limit",
        50,
        100,
        if dataset {
            "Dataset list limit must be between 1 and 100"
        } else {
            "limit must be between 1 and 100"
        },
    )?;
    let mut params = vec![Sql::Text(profile.to_owned())];
    let mut clause = "";
    if truthy(field(args, "cursor")) {
        let invalid_cursor = || {
            invalid(if dataset {
                "Invalid dataset cursor"
            } else {
                "Invalid investigation cursor"
            })
        };
        let raw =
            text_value(field(args, "cursor"), "cursor", 2048).map_err(|_| invalid_cursor())?;
        let v = cursor::decode(raw).ok_or_else(invalid_cursor)?;
        let v = list(&v);
        if v.len() != 3 || v[0] != s(profile) || v.iter().any(|v| v.as_str().is_none()) {
            return Err(invalid_cursor());
        }
        params.extend(
            v[1..]
                .iter()
                .map(|v| Sql::Text(v.as_str().unwrap().to_owned())),
        );
        clause = " AND (created_at,id)<(?,?)";
    }
    params.push(Sql::Integer(size + 1));
    let table = if dataset {
        "chain_analysis_datasets"
    } else {
        "chain_analysis_cases"
    };
    let found = rows(conn, &format!("SELECT * FROM {table} WHERE profile_id=?{clause} ORDER BY created_at DESC,id DESC LIMIT ?"), params_from_iter(params))?;
    let mut next = Value::Null;
    if found.len() > size as usize {
        let last = &found[size as usize - 1];
        next = cursor::encode(
            &Value::List(vec![
                s(profile),
                field(last, "created_at").clone(),
                field(last, "id").clone(),
            ]),
            !dataset,
        )?;
    }
    let items = found
        .iter()
        .take(size as usize)
        .map(|row| {
            if dataset {
                datasets::pack(row)
            } else {
                case(row, false)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(dict([("items", Value::List(items)), ("next_cursor", next)]))
}

fn compare(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(args, &["id", "other_id"], &["id"])?;
    atomic(conn, || {
        let base = get_case(conn, profile, field(args, "id"))?;
        let saved = truthy(field(args, "other_id"));
        let current = if saved {
            field(&get_case(conn, profile, field(args, "other_id"))?, "result").clone()
        } else {
            query(conn, profile, field(&base, "query"), sources)?
        };
        let old = field(&base, "result");
        let mut value = dict([
            ("base_id", field(&base, "id").clone()),
            ("base_snapshot_id", field(&base, "snapshot_id").clone()),
            (
                "current_snapshot_id",
                field(&current, "snapshot_id").clone(),
            ),
            (
                "changed",
                Value::Bool(engine::digest(old)? != engine::digest(&current)?),
            ),
            (
                "comparison",
                s(if saved {
                    "saved_to_saved"
                } else {
                    "saved_to_current"
                }),
            ),
            (
                "summary",
                dict([
                    ("base", field(&base, "summary").clone()),
                    (
                        "current",
                        get(&current, "summary")
                            .cloned()
                            .unwrap_or_else(|| dict::<&str>([])),
                    ),
                ]),
            ),
        ]);
        for kind in [
            "nodes",
            "edges",
            "findings",
            "clusters",
            "patterns",
            "exposure",
            "transaction_features",
        ] {
            let key = if kind == "transaction_features" {
                "subject"
            } else {
                "id"
            };
            let map = |v| {
                list(v)
                    .iter()
                    .map(|row| (text(row, key).to_owned(), row.clone()))
                    .collect::<BTreeMap<_, _>>()
            };
            let before = map(field(old, kind));
            let after = map(field(&current, kind));
            let added = after
                .iter()
                .filter(|(k, _)| !before.contains_key(*k))
                .map(|(_, v)| v.clone())
                .collect();
            let removed = before
                .iter()
                .filter(|(k, _)| !after.contains_key(*k))
                .map(|(_, v)| v.clone())
                .collect();
            let mut changed = Vec::new();
            for (id, row) in &before {
                if let Some(new) = after.get(id) {
                    if !equivalent(row, new) {
                        changed.push(dict([
                            ("id", s(id.as_str())),
                            ("before", row.clone()),
                            ("after", new.clone()),
                        ]));
                    }
                }
            }
            for (prefix, rows) in [("added", added), ("removed", removed), ("changed", changed)] {
                set(&mut value, &format!("{prefix}_{kind}"), Value::List(rows));
            }
        }
        if !equivalent(field(old, "coverage"), field(&current, "coverage")) {
            set(
                &mut value,
                "coverage_changed",
                dict([
                    ("before", field(old, "coverage").clone()),
                    ("after", field(&current, "coverage").clone()),
                ]),
            );
        }
        Ok(value)
    })
}
