use super::*;
use sha2::{Digest, Sha256};

fn digest(value: &Value) -> Result<String> {
    Ok(Sha256::digest(canonical(value)?.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub(super) fn get_case(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    text_value(id, "id", 64)?;
    let row = first(
        conn,
        "SELECT * FROM chain_analysis_cases WHERE profile_id=? AND id=?",
        params![profile, sql(id, 2)?],
    )?
    .ok_or_else(|| AppError::new("not_found", "Investigation not found in this book"))?;
    let integrity = || {
        AppError::new(
            "integrity_error",
            "Stored investigation failed its content integrity check",
        )
    };
    let result = kassiber_pytext::loads(field(&row, "result_json").as_str().ok_or_else(integrity)?)
        .map_err(|_| integrity())?;
    let query = kassiber_pytext::loads(field(&row, "query_json").as_str().ok_or_else(integrity)?)
        .map_err(|_| integrity())?;
    if !matches!(result, Value::Dict(_))
        || digest(&result)? != field(&row, "result_digest").as_str().unwrap_or("")
        || !py_eq(&query, field(&result, "query"))
        || !py_eq(field(&row, "snapshot_id"), field(&result, "snapshot_id"))
    {
        return Err(integrity());
    }
    let mut output = dict(
        ["id", "title", "created_at", "snapshot_id"].map(|key| (key, field(&row, key).clone())),
    );
    set(&mut output, "query", query);
    set(
        &mut output,
        "summary",
        get(&result, "summary")
            .cloned()
            .unwrap_or_else(|| dict::<&str>([])),
    );
    set(&mut output, "result", result);
    Ok(output)
}

pub(super) fn save(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(
        args,
        &["title", "query", "expected_snapshot_id"],
        &["title", "query", "expected_snapshot_id"],
    )?;
    let title = text_value(field(args, "title"), "title", 256)?;
    atomic(conn, || {
        let query = crate::analysis_index::traversal::normalize_query(field(args, "query"))?;
        crate::analysis_index::durable::synchronize_with_sources(conn, profile, false, sources)?;
        let result = crate::analysis_index::views::with_read(
            conn,
            profile,
            field(&query, "observer").as_str().unwrap(),
            |view| crate::analysis_index::mirror::analyze_snapshot(view, &query),
        )?;
        if field(&result, "snapshot_id") != field(args, "expected_snapshot_id") {
            return Err(stale(
                "Analysis inputs changed; run the query again before saving",
            ));
        }
        let payload = canonical(&result)?;
        if payload.len() > 10_000_000 {
            return Err(invalid(
                "Investigation exceeds the 10 MB snapshot limit; narrow the query",
            ));
        }
        let id = sources.id();
        conn.execute(
            "INSERT INTO chain_analysis_cases VALUES(?,?,?,?,?,?,?,?)",
            params![
                id,
                profile,
                sql(&Value::Str(title), 3)?,
                canonical(field(&result, "query"))?,
                payload,
                sql(field(&result, "snapshot_id"), 6)?,
                digest(&result)?,
                sources.now()
            ],
        )
        .map_err(database_error)?;
        get_case(conn, profile, &s(id))
    })
}

pub(super) fn delete(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(args, &["id"], &["id"])?;
    let id = field(args, "id");
    get_case(conn, profile, id)?;
    conn.execute(
        "DELETE FROM chain_analysis_cases WHERE profile_id=? AND id=?",
        params![profile, sql(id, 2)?],
    )
    .map_err(database_error)?;
    Ok(dict([("id", id.clone()), ("deleted", Value::Bool(true))]))
}
