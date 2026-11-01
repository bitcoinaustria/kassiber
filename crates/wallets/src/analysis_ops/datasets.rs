use super::*;

fn get_dataset(conn: &Connection, profile: &str, id: &Value) -> Result<Value> {
    let mut result = first(
        conn,
        "SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND id=?",
        params![profile, sql(id, 2)?],
    )?
    .ok_or_else(|| AppError::new("not_found", "Dataset not found in this book"))?;
    let manifest = kassiber_pytext::loads(field(&result, "manifest_json").as_str().unwrap_or(""))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    remove(&mut result, "profile_id");
    remove(&mut result, "manifest_json");
    set(&mut result, "manifest", manifest);
    set(&mut result, "accounting_authority", Value::Bool(false));
    Ok(result)
}

pub(super) fn revoke(
    conn: &Connection,
    profile: &str,
    args: &Value,
    sources: &mut dyn Sources,
) -> Result<Value> {
    arguments(
        args,
        &["id", "expected_revision"],
        &["id", "expected_revision"],
    )?;
    atomic(conn, || {
        let id = field(args, "id");
        let old = get_dataset(conn, profile, id)?;
        if !matches!(get(args, "expected_revision"), Some(Value::Int(n)) if field(&old, "revision") == &Value::Int(n.clone()))
        {
            return Err(stale("Dataset changed; reload before revoking"));
        }
        if ![s("active"), s("superseded")].contains(field(&old, "status")) {
            return Err(invalid("Only a complete dataset can be revoked"));
        }
        conn.execute("UPDATE chain_analysis_datasets SET status='revoked',revision=revision+1,updated_at=? WHERE id=? AND profile_id=?",
            params![sources.now(), sql(id, 2)?, profile]).map_err(database_error)?;
        get_dataset(conn, profile, id)
    })
}

pub(super) fn discard(conn: &Connection, profile: &str, args: &Value) -> Result<Value> {
    arguments(args, &["id"], &["id"])?;
    if !conn.is_autocommit() {
        return Err(AppError::new(
            "chain_analysis_dataset_busy",
            "Dataset cleanup requires an idle connection",
        ));
    }
    let id = field(args, "id");
    atomic(conn, || {
        let old = get_dataset(conn, profile, id)?;
        if ![s("staging"), s("failed")].contains(field(&old, "status")) {
            return Err(invalid(
                "Complete versions are retained as provenance; revoke them instead",
            ));
        }
        conn.execute(
            "UPDATE chain_analysis_datasets SET status='failed' WHERE id=? AND profile_id=?",
            params![sql(id, 1)?, profile],
        )
        .map_err(database_error)?;
        Ok(())
    })?;
    loop {
        let deleted = atomic(conn, || {
            conn.execute("DELETE FROM chain_analysis_dataset_claims WHERE dataset_id=? AND record_number IN (SELECT record_number FROM chain_analysis_dataset_claims WHERE dataset_id=? LIMIT ?)",
                params![sql(id, 1)?, sql(id, 2)?, 2000]).map_err(database_error)
        })?;
        if deleted == 0 {
            break;
        }
    }
    atomic(conn, || {
        conn.execute(
            "DELETE FROM chain_analysis_datasets WHERE id=? AND profile_id=? AND status='failed'",
            params![sql(id, 1)?, profile],
        )
        .map_err(database_error)?;
        Ok(())
    })?;
    Ok(dict([("id", id.clone()), ("discarded", Value::Bool(true))]))
}
