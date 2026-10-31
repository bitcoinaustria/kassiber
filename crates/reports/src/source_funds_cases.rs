use super::*;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::params;
use sha2::{Digest, Sha256};

pub struct FrozenCase<'a> {
    pub workspace_id: &'a str,
    pub profile_id: &'a str,
    pub target_transaction_id: &'a str,
    pub target_amount_msat: i64,
    pub asset: &'a str,
    pub reveal_mode: &'a str,
    pub status: &'a str,
    pub snapshot: &'a Value,
    pub label: Option<&'a str>,
    pub target_external_id: Option<&'a str>,
    pub recipient_id: Option<&'a str>,
    pub recipient_label_snapshot: Option<&'a str>,
    pub recipient_kind_snapshot: Option<&'a str>,
    pub recipient_reveal_mode_snapshot: Option<&'a str>,
}

struct Write<'a> {
    conn: &'a Connection,
    owns_transaction: bool,
    committed: bool,
}

impl Drop for Write<'_> {
    fn drop(&mut self) {
        if !self.committed {
            if self.owns_transaction {
                let _ = self.conn.execute_batch("ROLLBACK");
            } else {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO source_funds_case_write; RELEASE source_funds_case_write",
                );
            }
        }
    }
}

pub fn save_snapshot(
    conn: &Connection,
    case: &FrozenCase<'_>,
    sources: &dyn Sources,
) -> Result<Value> {
    let case_id = sources.uuid(conn)?;
    let snapshot_id = sources.uuid(conn)?;
    let created_at = sources.now(conn)?;
    let snapshot = kassiber_envelope::json_ready(case.snapshot.clone());
    let stored =
        kassiber_pyjson::dumps(&snapshot, &kassiber_pyjson::Options::new().sort_keys(true))
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let compact = kassiber_pyjson::dumps(
        &snapshot,
        &kassiber_pyjson::Options::new().sort_keys(true).compact(),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let digest = format!("{:x}", Sha256::digest(compact.as_bytes()));
    let owns_transaction = conn.is_autocommit();
    conn.execute_batch("SAVEPOINT source_funds_case_write")
        .map_err(sql_error)?;
    let mut write = Write {
        conn,
        owns_transaction,
        committed: false,
    };
    conn.execute(
        "INSERT INTO source_funds_cases(id,workspace_id,profile_id,target_transaction_id,target_external_id,target_amount,asset,label,reveal_mode,status,snapshot_hash,snapshot_json,created_at,updated_at,recipient_id,recipient_label_snapshot,recipient_kind_snapshot,recipient_reveal_mode_snapshot) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![case_id,case.workspace_id,case.profile_id,case.target_transaction_id,case.target_external_id,case.target_amount_msat,case.asset,case.label,case.reveal_mode,case.status,digest,stored,created_at,created_at,case.recipient_id,case.recipient_label_snapshot,case.recipient_kind_snapshot,case.recipient_reveal_mode_snapshot],
    ).map_err(sql_error)?;
    conn.execute(
        "INSERT INTO source_funds_snapshots(id,case_id,snapshot_hash,snapshot_json,created_at) VALUES(?,?,?,?,?)",
        params![snapshot_id,case_id,digest,stored,created_at],
    ).map_err(sql_error)?;
    conn.execute_batch("RELEASE source_funds_case_write")
        .map_err(sql_error)?;
    write.committed = true;
    Ok(dict([
        ("id", s(case_id)),
        ("snapshot_id", s(snapshot_id)),
        ("snapshot_hash", s(digest)),
        ("status", s(case.status)),
        ("created_at", s(created_at)),
    ]))
}
