//! One-shot projection invalidation. Authored evidence and retained journals
//! are untouched; audit insertion and invalidation commit together.
use crate::{layout::python_value, map_sqlite_error, Error, ErrorKind, Result};
use kassiber_pyjson::{dumps, Key, Options, Value as PyValue};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const WAGES: &str = "lower(trim(coalesce(kind, ''))) = 'wages'";
const PARTIAL_PAYOUT: &str = "custody-partial-payout-pricing-v1";

/// Run only after the compatibility pass has established the referenced
/// columns. The order is the order in Python ensure_schema_compat, with the
/// valuation marker after transaction shape normalization.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    migrate_with_observer(conn, &mut |_| Ok(()))
}

pub(crate) fn migrate_with_observer(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    migrate_stage(conn, false, observer)?;
    migrate_stage(conn, true, observer)?;
    partial_payout(conn, observer)
}

/// The valuation migration belongs after the msat/transaction shape repair;
/// the other four markers precede it in Python's compatibility pass.
pub(crate) fn migrate_stage(
    conn: &mut Connection,
    valuation_stage: bool,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let migrations: Vec<Value> = serde_json::from_str(include_str!("../fixtures/semantics.json"))
        .expect("semantic manifest");
    for migration in migrations {
        let name = migration["name"].as_str().expect("migration name");
        if name == PARTIAL_PAYOUT {
            continue;
        }
        if (name == "at-acquisition-valuation-support-v1") != valuation_stage {
            continue;
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let applied: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM schema_migration_audits WHERE migration_name=?)",
                [name],
                |r| r.get(0),
            )
            .map_err(map_sqlite_error)?;
        if applied {
            continue;
        }
        let affected = migration["affected"].as_str().map(str::to_owned).unwrap_or_else(|| format!("EXISTS(SELECT 1 FROM transactions transaction_row WHERE transaction_row.profile_id=profiles.id AND {WAGES})"));
        let count: i64 = tx
            .query_row(
                &format!("SELECT COUNT(*) FROM profiles WHERE {affected}"),
                [],
                |r| r.get(0),
            )
            .map_err(map_sqlite_error)?;
        let mut impact = migration["impact"].clone();
        let valuation = name == "at-acquisition-valuation-support-v1";
        let impact_json = if valuation {
            // This migration uses json.dumps defaults, including dict order.
            let fields = vec![
                (Key::Str("schema_version".into()), PyValue::Int(1.into())),
                (
                    Key::Str("affected_profile_count".into()),
                    PyValue::Int(count.into()),
                ),
                (
                    Key::Str("explanation".into()),
                    python_value(&impact["explanation"]),
                ),
            ];
            dumps(&PyValue::Dict(fields), &Options::default())
        } else {
            impact["changes"][0]["affected_profile_count"] = count.into();
            if name == "wages-acquisition-semantics-v1" {
                let transactions: i64 = tx
                    .query_row(
                        &format!("SELECT COUNT(*) FROM transactions WHERE {WAGES}"),
                        [],
                        |r| r.get(0),
                    )
                    .map_err(map_sqlite_error)?;
                impact["changes"][0]["affected_transaction_count"] = transactions.into();
            }
            dumps(
                &python_value(&impact),
                &Options::default().sort_keys(true).compact(),
            )
        }
        .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
        observer(&format!("{name}.before_audit"))?;
        let inserted = tx.execute("INSERT OR IGNORE INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES(?1,?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [name, &impact_json]).map_err(map_sqlite_error)?;
        observer(&format!("{name}.after_audit"))?;
        if inserted != 0 {
            tx.execute(&format!("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE {affected}"), []).map_err(map_sqlite_error)?;
        }
        observer(&format!("{name}.before_commit"))?;
        tx.commit().map_err(map_sqlite_error)?;
        observer(&format!("{name}.after_commit"))?;
    }
    Ok(())
}

/// Python runs this after authored memberships and transaction shapes are ready.
pub(crate) fn partial_payout(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    if tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migration_audits WHERE migration_name=?)",
            [PARTIAL_PAYOUT],
            |r| r.get::<_, bool>(0),
        )
        .map_err(map_sqlite_error)?
    {
        return Ok(());
    }
    // Match list_payout_review_records: effective memberships, both legs, then
    // last term in Python's order per legacy review identity. A source anchor
    // is evidence, so a missing transaction must not silently disappear.
    let mut reviews = BTreeMap::new();
    {
        let mut statement = tx.prepare("SELECT term.profile_id,term.legacy_source_id,
            term.reviewed_source_amount_msat,t.amount,t.fee,t.direction,t.amount_includes_fee
            FROM custody_component_economic_terms term
            JOIN custody_components c ON c.id=term.component_id
            JOIN custody_component_legs source ON source.id=term.source_leg_id
            JOIN custody_component_legs target ON target.id=term.target_leg_id
            LEFT JOIN transactions t ON t.id=COALESCE(source.anchor_transaction_id,source.transaction_id)
            WHERE term.term_kind='direct_swap_payout' AND term.component_id IN
              (SELECT component_id FROM custody_component_transaction_memberships WHERE profile_id=term.profile_id)
            ORDER BY term.created_at DESC,term.component_id,term.ordinal").map_err(map_sqlite_error)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    r.get::<_, String>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })
            .map_err(map_sqlite_error)?;
        for row in rows {
            let (profile, review, reviewed, amount, fee, direction, included) =
                row.map_err(map_sqlite_error)?;
            let principal = i128::from(amount)
                - if included && direction.trim().eq_ignore_ascii_case("outbound") {
                    i128::from(fee)
                } else {
                    0
                };
            let reviewed = i128::from(reviewed.unwrap_or(amount));
            reviews.insert((profile, review), reviewed > 0 && reviewed < principal);
        }
    }
    let affected: BTreeSet<_> = reviews
        .into_iter()
        .filter_map(|((profile, _), partial)| partial.then_some(profile))
        .collect();
    let impact = crate::layout::python_value(&serde_json::json!({
        "schema_version":1, "migration":PARTIAL_PAYOUT, "affected_profile_count":affected.len(),
    }));
    let impact = dumps(&impact, &Options::default().sort_keys(true))
        .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
    observer(&format!("{PARTIAL_PAYOUT}.before_audit"))?;
    tx.execute("INSERT INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES(?1,?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [PARTIAL_PAYOUT, &impact]).map_err(map_sqlite_error)?;
    observer(&format!("{PARTIAL_PAYOUT}.after_audit"))?;
    for profile in affected {
        tx.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL WHERE id=?", [profile]).map_err(map_sqlite_error)?;
    }
    observer(&format!("{PARTIAL_PAYOUT}.before_commit"))?;
    tx.commit().map_err(map_sqlite_error)?;
    observer(&format!("{PARTIAL_PAYOUT}.after_commit"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn book() -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        (root, book)
    }
    fn seed(conn: &Connection) {
        conn.execute_batch("INSERT INTO workspaces VALUES('ws','synthetic','fixed');
            INSERT INTO profiles(id,workspace_id,label,tax_country,gains_algorithm,last_processed_at,created_at) VALUES('at','ws','AT','at','FIFO','fixed','fixed'),('pool','ws','Pool','generic','moving_average','fixed','fixed'),('fifo','ws','FIFO','generic','FIFO','fixed','fixed');
            INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','at','Wallet','custom','fixed');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,kind,created_at) VALUES('wage','ws','at','wal','wage','fixed','inbound','BTC',1,' WAGES ','fixed'),('airdrop','ws','at','wal','airdrop','fixed','inbound','BTC',1,'airdrop','fixed');").unwrap();
    }
    #[test]
    fn every_semantic_step_is_one_shot_and_preserves_retained_rows() {
        let (_root, mut book) = book();
        seed(&book.connection);
        book.connection
            .execute("DELETE FROM schema_migration_audits", [])
            .unwrap();
        migrate(&mut book.connection).unwrap();
        let versions: Vec<i64> = book
            .connection
            .prepare("SELECT journal_input_version FROM profiles ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(versions, [1, 1, 1]); // Earlier markers clear processed_at.
        let before = book.connection.total_changes();
        migrate(&mut book.connection).unwrap();
        assert_eq!(book.connection.total_changes(), before);
        assert_eq!(
            book.connection
                .query_row("SELECT count(*) FROM transactions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert!(book
            .connection
            .execute("UPDATE schema_migration_audits SET impact_json='{}'", [])
            .is_err());
    }
    #[test]
    fn each_audit_and_profile_update_roll_back_together() {
        let migrations: Vec<Value> =
            serde_json::from_str(include_str!("../fixtures/semantics.json")).unwrap();
        for migration in migrations {
            for boundary in ["before_audit", "after_audit", "before_commit"] {
                let (_root, mut book) = book();
                seed(&book.connection);
                let name = migration["name"].as_str().unwrap();
                book.connection
                    .execute(
                        "DELETE FROM schema_migration_audits WHERE migration_name=?",
                        [name],
                    )
                    .unwrap();
                let target = format!("{name}.{boundary}");
                assert!(migrate_with_observer(&mut book.connection, &mut |label| {
                    if label == target {
                        Err(Error::new(ErrorKind::Io, "injected migration failure"))
                    } else {
                        Ok(())
                    }
                })
                .is_err());
                assert_eq!(
                    book.connection
                        .query_row(
                            "SELECT count(*) FROM schema_migration_audits WHERE migration_name=?",
                            [name],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    0
                );
                assert_eq!(
                    book.connection
                        .query_row("SELECT max(journal_input_version) FROM profiles", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
                migrate(&mut book.connection).unwrap();
            }
        }
    }
}
