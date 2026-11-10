//! Transaction primitives for the unregistered Bitcoin publication path.
//! These do not yet implement the complete wallet coordinator. No acquisition,
//! progress emitter, product routing or implicit accounting authority lives here.
use crate::{database_error, invalidate_journals};
use kassiber_envelope::AppError;
use kassiber_store::rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(crate) mod balance;
pub mod insert;
pub mod inventory;
pub(crate) mod physical;
pub mod policy;
pub mod provenance;
pub mod reports;
pub(crate) mod row;
#[cfg(test)]
pub(crate) mod testbook;
pub mod wallet;

type Result<T> = std::result::Result<T, AppError>;

/// Own one rollback boundary, including cancellation and Rust unwinding.
/// Preparation must finish before entry. `apply` is local persistence only;
/// it must neither commit nor invoke progress callbacks. Nested use releases
/// only this savepoint and leaves the caller's transaction open.
pub fn with_wallet_savepoint<T>(
    conn: &mut Connection,
    mut check_cancelled: impl FnMut() -> Result<()>,
    apply: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let savepoint = kassiber_store::write_savepoint(conn).map_err(database_error)?;
    check_cancelled()?;
    let value = apply(&savepoint)?;
    check_cancelled()?;
    savepoint.commit().map_err(database_error)?;
    Ok(value)
}

/// Python's `retract_wallet_records(commit=False)`. Input is the composed
/// retraction set, after live-observer vetoes; a backend absence alone is not
/// sufficient authority to call this. SQL foreign keys own collateral deletion.
pub fn retract_records(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    wallet_label: &str,
    external_ids: &[String],
    source_label: &str,
) -> Result<Value> {
    if conn.is_autocommit() {
        return Err(AppError::new(
            "observer_apply_outside_transaction",
            "Observation retraction requires the caller's wallet transaction",
        ));
    }
    let candidates: Vec<_> = external_ids
        .iter()
        .map(|id| kassiber_pytext::strip(id).to_lowercase())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut retracted = Vec::new();
    for chunk in candidates.chunks(500) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT id,external_id,occurred_at,direction,asset,amount,fee
             FROM transactions WHERE profile_id=? AND wallet_id=?
             AND external_id IS NOT NULL AND LOWER(external_id) IN ({placeholders})"
        );
        let mut statement = conn.prepare(&sql).map_err(database_error)?;
        let parameters = [profile_id, wallet_id]
            .into_iter()
            .chain(chunk.iter().map(String::as_str));
        let rows = statement
            .query_map(params_from_iter(parameters), |row| {
                Ok(json!({
                    "transaction_id": row.get::<_, String>(0)?,
                    "wallet": wallet_label,
                    "external_id": row.get::<_, String>(1)?,
                    "occurred_at": row.get::<_, String>(2)?,
                    "direction": row.get::<_, String>(3)?,
                    "asset": row.get::<_, String>(4)?,
                    "amount": row.get::<_, i64>(5)?,
                    "fee": row.get::<_, i64>(6)?,
                }))
            })
            .map_err(database_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(database_error)?;
        if rows.is_empty() {
            continue;
        }
        let placeholders = vec!["?"; rows.len()].join(", ");
        conn.execute(
            &format!("DELETE FROM transactions WHERE id IN ({placeholders})"),
            params_from_iter(
                rows.iter()
                    .map(|row| row["transaction_id"].as_str().expect("selected text id")),
            ),
        )
        .map_err(database_error)?;
        retracted.extend(rows);
    }
    let invalidated = !retracted.is_empty();
    if invalidated {
        invalidate_journals(conn, profile_id)?;
    }
    Ok(json!({
        "wallet": wallet_label, "source": source_label,
        "retracted": retracted.len(), "journal_invalidated": invalidated,
        "retracted_records": retracted,
    }))
}

/// The final successful-sync metadata/checkpoint writes from
/// `_mark_wallet_synced_from_results`. The caller supplies the already-composed
/// checkpoint; this helper never infers success or contacts a provider.
pub struct Synced<'a> {
    pub profile_id: &'a str,
    pub wallet_id: &'a str,
    pub wallet_label: &'a str,
    pub checkpoint: Option<&'a Value>,
    pub partial_success: bool,
    pub degraded_reason: Option<&'a str>,
    pub blocking_reports: bool,
    pub timestamp: &'a str,
    pub updated_at: &'a str,
}

fn python_json(value: &Value, compact: bool) -> Result<String> {
    let value = kassiber_pytext::loads(&value.to_string())
        .map_err(|_| AppError::new("internal_error", "Invalid publication JSON"))?;
    let mut options = kassiber_pyjson::Options::default().sort_keys(true);
    if compact {
        options = options.compact();
    }
    kassiber_pyjson::dumps(&value, &options)
        .map_err(|_| AppError::new("internal_error", "Invalid publication JSON"))
}

pub fn mark_synced(conn: &Connection, synced: &Synced<'_>) -> Result<()> {
    if conn.is_autocommit() {
        return Err(AppError::new(
            "observer_apply_outside_transaction",
            "Sync metadata application requires the caller's wallet transaction",
        ));
    }
    let raw: String = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=? AND profile_id=?",
            [synced.wallet_id, synced.profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                "Sync wallet does not belong to the selected book",
            )
        })?;
    let mut config: Value = serde_json::from_str(if raw.is_empty() { "{}" } else { &raw })
        .map_err(|_| AppError::new("internal_error", "Invalid wallet configuration JSON"))?;
    config
        .as_object_mut()
        .ok_or_else(|| AppError::new("internal_error", "Invalid wallet configuration JSON"))?
        .insert("last_synced_at".into(), json!(synced.timestamp));
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id=? AND profile_id=?",
        params![
            python_json(&config, false)?,
            synced.wallet_id,
            synced.profile_id
        ],
    )
    .map_err(database_error)?;
    let Some(checkpoint) = synced.checkpoint.filter(|value| value.is_object()) else {
        return Ok(());
    };
    let source_key = format!("onchain_wallet:{}", synced.wallet_id);
    let (paused, retry_count): (bool, i64) = conn
        .query_row(
            "SELECT paused,retry_count FROM freshness_source_states WHERE profile_id=? AND source_key=?",
            [synced.profile_id, &source_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database_error)?
        .unwrap_or((false, 0));
    let status = if paused {
        "paused"
    } else if synced.blocking_reports {
        "blocking_reports"
    } else if synced.partial_success {
        "partially_stale"
    } else {
        "fresh"
    };
    conn.execute(
        "INSERT INTO freshness_source_states(
         profile_id,source_key,source_type,source_label,status,state,stale_reason,
         blocking_reports,paused,rate_limited_until,cooldown_reason,retry_count,
         last_success_at,last_error_at,last_error_code,last_error_message,
         last_phase,progress_json,checkpoint_json,updated_at)
         VALUES(?,?,'onchain_wallet',?,?,?,?,?,?,NULL,NULL,?,?,NULL,NULL,NULL,'done',?,?,?)
         ON CONFLICT(profile_id,source_key) DO UPDATE SET
         source_type=excluded.source_type,source_label=excluded.source_label,
         status=excluded.status,state=excluded.state,stale_reason=excluded.stale_reason,
         blocking_reports=excluded.blocking_reports,paused=excluded.paused,
         rate_limited_until=excluded.rate_limited_until,cooldown_reason=excluded.cooldown_reason,
         retry_count=excluded.retry_count,last_success_at=excluded.last_success_at,
         last_error_at=excluded.last_error_at,last_error_code=excluded.last_error_code,
         last_error_message=excluded.last_error_message,last_phase=excluded.last_phase,
         progress_json=excluded.progress_json,checkpoint_json=excluded.checkpoint_json,
         updated_at=excluded.updated_at",
        params![
            synced.profile_id,
            source_key,
            synced.wallet_label,
            status,
            status,
            if synced.partial_success {
                synced.degraded_reason
            } else {
                None
            },
            synced.blocking_reports,
            paused,
            retry_count,
            synced.timestamp,
            python_json(&json!({"phase":"done"}), true)?,
            python_json(checkpoint, true)?,
            synced.updated_at,
        ],
    )
    .map_err(database_error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
