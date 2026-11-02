use super::*;
use crate::analysis_index::reads::preview::{normalize_manifest, stream_claims, LineSource};
use sha2::{Digest, Sha256};

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
    discard_with(conn, profile, field(args, "id"), &|_| {}, &|| false)
}

/// `discard_dataset`: remove only incomplete imports in bounded commits.
pub fn discard_with(
    conn: &Connection,
    profile: &str,
    id: &Value,
    progress: &dyn Fn(Value),
    cancelled: &dyn Fn() -> bool,
) -> Result<Value> {
    if !conn.is_autocommit() {
        return Err(AppError::new(
            "chain_analysis_dataset_busy",
            "Dataset cleanup requires an idle connection",
        ));
    }
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
    let mut removed = 0u64;
    loop {
        if cancelled() {
            return Err(AppError::new(
                "chain_analysis_cancelled",
                "Dataset cleanup cancelled",
            ));
        }
        let deleted = atomic(conn, || {
            conn.execute("DELETE FROM chain_analysis_dataset_claims WHERE dataset_id=? AND record_number IN (SELECT record_number FROM chain_analysis_dataset_claims WHERE dataset_id=? LIMIT ?)",
                params![sql(id, 1)?, sql(id, 2)?, 2000]).map_err(database_error)
        })?;
        removed += deleted as u64;
        progress(dict([
            ("rows_deleted", Value::Int(removed.into())),
            ("status", s("discarding")),
        ]));
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

/// `_json`: sorted, compact, non-ASCII retained, no NaN.
fn compact(value: &Value) -> Result<String> {
    dumps(
        value,
        &Options::new()
            .sort_keys(true)
            .compact()
            .ensure_ascii(false)
            .allow_nan(false),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

fn check_replacement(conn: &Connection, profile: &str, manifest: &Value) -> Result<()> {
    let active = first(
        conn,
        "SELECT id FROM chain_analysis_datasets WHERE profile_id=? AND dataset_key=? AND status='active'",
        params![profile, sql(field(manifest, "dataset_key"), 2)?],
    )?
    .map(|row| field(&row, "id").clone())
    .unwrap_or(Value::Null);
    if !py_eq(&active, field(manifest, "expected_active_id")) {
        return Err(stale(
            "Active dataset changed; preview the replacement again",
        ));
    }
    let reused = first(
        conn,
        "SELECT 1 FROM chain_analysis_datasets WHERE profile_id=? AND dataset_key=? AND version=? AND status IN ('active','superseded','revoked') LIMIT 1",
        params![profile, sql(field(manifest, "dataset_key"), 2)?, sql(field(manifest, "version"), 3)?],
    )?;
    if reused.is_some() {
        return Err(invalid(
            "Dataset versions are immutable; choose a new version",
        ));
    }
    Ok(())
}

/// One import's local inputs; the bytes come from a caller-opened source.
pub struct Import<'a> {
    pub manifest: &'a Value,
    pub format: &'a Value,
    pub adapter: &'a Value,
    pub expected_sha256: Option<&'a Value>,
    pub progress: &'a dyn Fn(Value),
    pub cancelled: &'a dyn Fn() -> bool,
}

/// `import_dataset`: stage at most 2,000 claims per commit on an idle
/// connection and activate only a complete, unchanged pack.
pub fn import(
    conn: &Connection,
    profile: &str,
    request: &Import<'_>,
    source: &mut dyn LineSource,
    sources: &mut dyn Sources,
) -> Result<Value> {
    let manifest = normalize_manifest(request.manifest)?;
    if !conn.is_autocommit() {
        return Err(AppError::new(
            "chain_analysis_dataset_busy",
            "Dataset imports require an idle connection; finish caller-owned writes first",
        ));
    }
    if let Some(expected) = request.expected_sha256.filter(|v| v != &&Value::Null) {
        if !matches!(expected, Value::Str(t) if t.as_str().is_some_and(|t| t.len() == 64 && t.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))))
        {
            return Err(invalid(
                "expected_sha256 must be a lowercase SHA-256 digest",
            ));
        }
    }
    if first(conn, "SELECT 1 FROM profiles WHERE id=?", [profile])?.is_none() {
        return Err(AppError::new("not_found", "Book not found"));
    }
    let format = request
        .format
        .as_str()
        .filter(|f| ["csv", "jsonl"].contains(f));
    let adapter = request
        .adapter
        .as_str()
        .filter(|a| ["generic", "am_i_exposed", "maru92"].contains(a));
    let (Some(format), Some(adapter)) = (format, adapter) else {
        return Err(invalid(
            "Dataset format must be csv/jsonl and adapter generic/am_i_exposed/maru92",
        ));
    };
    let id = sources.id();
    let timestamp = sources.now();
    let mut stored = manifest.clone();
    set(&mut stored, "format", s(format));
    set(&mut stored, "adapter", s(adapter));
    atomic(conn, || {
        check_replacement(conn, profile, &manifest)?;
        conn.execute("INSERT INTO chain_analysis_datasets(id,profile_id,dataset_key,version,chain,network,visibility,status,manifest_json,created_at,updated_at) VALUES(?,?,?,?,?,?,?,'staging',?,?,?)",
            params![id, profile, sql(field(&manifest, "dataset_key"), 3)?, sql(field(&manifest, "version"), 4)?, sql(field(&manifest, "chain"), 5)?,
                sql(field(&manifest, "network"), 6)?, sql(field(&manifest, "visibility"), 7)?, compact(&stored)?, timestamp, timestamp])
            .map_err(database_error)?;
        Ok(())
    })?;
    let staged = (|| -> Result<Value> {
        let mut batch: Vec<Value> = Vec::new();
        let mut count = 0u64;
        let mut claims = Sha256::new();
        let flush = |batch: &mut Vec<Value>,
                     count: u64,
                     bytes: u64,
                     sources: &mut dyn Sources|
         -> Result<()> {
            atomic(conn, || {
                // A reset or discard during reading cannot resurrect the pack.
                let row = first(
                    conn,
                    "SELECT status FROM chain_analysis_datasets WHERE id=? AND profile_id=?",
                    params![id, profile],
                )?;
                if !row.is_some_and(|row| field(&row, "status") == &s("staging")) {
                    return Err(stale("Staged dataset changed during import"));
                }
                let mut insert = conn
                    .prepare("INSERT INTO chain_analysis_dataset_claims VALUES(?,?,?,?,?,?,?,?)")
                    .map_err(database_error)?;
                for claim in batch.iter() {
                    insert
                        .execute(params![
                            id,
                            sql(field(claim, "record_number"), 2)?,
                            sql(field(claim, "subject"), 3)?,
                            sql(field(claim, "label"), 4)?,
                            sql(field(claim, "category"), 5)?,
                            compact(field(claim, "source_record"))?,
                            sql(field(claim, "valid_from"), 7)?,
                            sql(field(claim, "valid_until"), 8)?
                        ])
                        .map_err(database_error)?;
                }
                conn.execute("UPDATE chain_analysis_datasets SET row_count=?,byte_count=?,updated_at=? WHERE id=?",
                    params![count as i64, bytes as i64, sources.now(), id]).map_err(database_error)?;
                Ok(())
            })?;
            batch.clear();
            (request.progress)(dict([
                ("id", s(id.as_str())),
                ("row_count", Value::Int(count.into())),
                ("byte_count", Value::Int(bytes.into())),
                ("status", s("staging")),
            ]));
            Ok(())
        };
        let (content, bytes) = stream_claims(
            &manifest,
            source,
            format,
            adapter,
            request.cancelled,
            &mut |claim, number, bytes| {
                count = number;
                claims.update(format!("{}\n", compact(&claim)?).as_bytes());
                batch.push(claim);
                if batch.len() >= 2000 {
                    flush(&mut batch, count, bytes, sources)?;
                }
                Ok(())
            },
        )?;
        if count == 0 {
            return Err(invalid("Dataset contains no attribution claims"));
        }
        if !batch.is_empty() {
            flush(&mut batch, count, bytes, sources)?;
        }
        if request
            .expected_sha256
            .is_some_and(|e| e != &Value::Null && e != &s(content.as_str()))
        {
            return Err(stale("Dataset content changed since preview"));
        }
        if (request.cancelled)() {
            return Err(AppError::new(
                "chain_analysis_cancelled",
                "Dataset import cancelled before activation",
            ));
        }
        let claims_hash: String = claims
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        atomic(conn, || {
            check_replacement(conn, profile, &manifest)?;
            let previous = field(&manifest, "expected_active_id");
            if truthy(previous) {
                conn.execute("UPDATE chain_analysis_datasets SET status='superseded',revision=revision+1,updated_at=? WHERE id=?",
                    params![sources.now(), sql(previous, 2)?]).map_err(database_error)?;
            }
            let changed = conn.execute("UPDATE chain_analysis_datasets SET status='active',content_sha256=?,claims_sha256=?,byte_count=?,updated_at=? WHERE id=? AND profile_id=? AND status='staging'",
                params![content, claims_hash, bytes as i64, sources.now(), id, profile]).map_err(database_error)?;
            if changed != 1 {
                return Err(stale("Staged dataset changed before activation"));
            }
            Ok(())
        })?;
        get_dataset(conn, profile, &s(id.as_str()))
    })();
    match staged {
        Ok(value) => Ok(value),
        Err(error) => {
            atomic(conn, || {
                conn.execute("UPDATE chain_analysis_datasets SET status='failed',updated_at=? WHERE id=? AND profile_id=? AND status='staging'",
                    params![sources.now(), id, profile]).map_err(database_error)?;
                Ok(())
            })?;
            Err(error)
        }
    }
}
