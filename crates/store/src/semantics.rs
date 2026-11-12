//! One-shot projection invalidation. Authored evidence and retained journals
//! are untouched; audit insertion and invalidation commit together.
use crate::{layout::python_value, map_sqlite_error, Error, ErrorKind, Result};
use kassiber_pyjson::{dumps, Key, Options, Value as PyValue};
use rusqlite::{Connection, TransactionBehavior};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const WAGES: &str = "lower(trim(coalesce(kind, ''))) = 'wages'";
const PARTIAL_PAYOUT: &str = "custody-partial-payout-pricing-v1";
const ADOPTION: &str = "journal-calculation-semantics-adoption-v1";
const OUTFLOW: &str = "at-outflow-consumed-regime-v2";
const LEGACY_PAIR_FEE: &str = "custody-legacy-pair-fee-review-v1";
const SAME_TIME_ORDER: &str = "same-time-source-order-v1";
const PRICE_CURRENCY: &str = "price-currency-mismatch-hold-v1";
/// The semantics version that holds a price stored in another currency than
/// the book's.
const PRICE_CURRENCY_REQUIRED: i64 = 10;
/// The semantics version that orders same-time rows by their source order.
const SAME_TIME_ORDER_REQUIRED: i64 = 8;
/// The semantics version that includes the legacy pair fee blocker; an
/// older writer stamping the Austrian version 2 lacks it.
const LEGACY_PAIR_FEE_REQUIRED: i64 = 7;
/// Python's `JOURNAL_CALCULATION_SEMANTICS_VERSION`: main's Austrian outflow
/// regime (2), 8f's import identities and legacy pair fee blocker (3 to 7),
/// the same-time source order (8), and the price-currency hold (10; 9 stays
/// free for a concurrent fix). Retained journals are current only when
/// stamped with exactly this.
pub const JOURNAL_CALCULATION_SEMANTICS_VERSION: i64 = 10;

/// Compatibility shared by all retained-journal readers.
pub fn compatible(required: i64, built: i64) -> bool {
    required == 0 || (built == JOURNAL_CALCULATION_SEMANTICS_VERSION && built >= required)
}

/// SQL equivalent for internal profile aliases, never user-supplied SQL.
pub fn current_sql(profile: &str) -> String {
    format!("({profile}.required_journal_semantics_version=0 OR ({profile}.journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION} AND {profile}.journal_semantics_version>={profile}.required_journal_semantics_version))")
}

pub fn journal_current(conn: &Connection, profile: &str) -> Result<bool> {
    conn.query_row(
        &format!("SELECT {} FROM profiles p WHERE id=?", current_sql("p")),
        [profile],
        |r| r.get(0),
    )
    .map_err(map_sqlite_error)
}

pub(crate) fn adopt(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    migrate_selected(conn, "adoption", observer)
}

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
    adopt(conn, observer)?;
    migrate_stage(conn, false, observer)?;
    same_time_order(conn, observer)?;
    migrate_stage(conn, true, observer)?;
    partial_payout(conn, observer)?;
    // Python's ensure_schema_compat order: partial payouts, legacy pair
    // fee reviews, then the stored-error credential scrub.
    legacy_pair_fee(conn, observer)?;
    price_currency(conn, observer)?;
    crate::stored_errors::migrate(conn, observer)
}

/// The valuation migration belongs after the msat/transaction shape repair;
/// the other four markers precede it in Python's compatibility pass.
pub(crate) fn migrate_stage(
    conn: &mut Connection,
    valuation_stage: bool,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    migrate_selected(
        conn,
        if valuation_stage {
            "valuation"
        } else {
            "ordinary"
        },
        observer,
    )
}

fn migrate_selected(
    conn: &mut Connection,
    stage: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let migrations: Vec<Value> = serde_json::from_str(include_str!("../fixtures/semantics.json"))
        .expect("semantic manifest");
    for migration in migrations {
        let name = migration["name"].as_str().expect("migration name");
        if name == PARTIAL_PAYOUT {
            continue;
        }
        let selected_stage = if name == ADOPTION {
            "adoption"
        } else if name == "at-acquisition-valuation-support-v1" {
            "valuation"
        } else {
            "ordinary"
        };
        if stage != selected_stage {
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
        } else if name == ADOPTION || name == OUTFLOW {
            impact["affected_profile_count"] = count.into();
            dumps(
                &python_value(&impact),
                &if name == OUTFLOW {
                    Options::default().sort_keys(true)
                } else {
                    Options::default().sort_keys(true).compact()
                },
            )
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
            let required = if name == OUTFLOW { 2 } else { 1 };
            let invalidate = if name == ADOPTION {
                ""
            } else {
                "last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL,"
            };
            tx.execute(&format!("UPDATE profiles SET {invalidate}required_journal_semantics_version=MAX(required_journal_semantics_version,{required}) WHERE {affected}"), []).map_err(map_sqlite_error)?;
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
        tx.execute("UPDATE profiles SET last_processed_at=NULL,last_processed_tx_count=0,journal_input_version=journal_input_version+1,ownership_review_counts_json=NULL,required_journal_semantics_version=MAX(required_journal_semantics_version,1) WHERE id=?", [profile]).map_err(map_sqlite_error)?;
    }
    observer(&format!("{PARTIAL_PAYOUT}.before_commit"))?;
    tx.commit().map_err(map_sqlite_error)?;
    observer(&format!("{PARTIAL_PAYOUT}.after_commit"))
}

/// `mark_legacy_pair_fee_reviews`, for books without a candidate pair.
///
/// Python decides each candidate with the custody component validator, which
/// the store does not carry. A book holding an active quantity component with
/// a `reviewed_residual:network_fee` fee leg and no audit is refused for
/// Python to migrate; any other book records the zero-count audit exactly.
/// `_migrate_same_time_order_semantics`: require the source-order version of
/// every processed book, once, keeping its input markers.
pub(crate) fn same_time_order(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    const AFFECTED: &str = "last_processed_at IS NOT NULL OR EXISTS (\n        SELECT 1 FROM journal_entries WHERE profile_id = profiles.id)";
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    if tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migration_audits WHERE migration_name=?)",
            [SAME_TIME_ORDER],
            |r| r.get::<_, bool>(0),
        )
        .map_err(map_sqlite_error)?
    {
        return Ok(());
    }
    let count: i64 = tx
        .query_row(
            &format!("SELECT COUNT(*) FROM profiles WHERE {AFFECTED}"),
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    let impact = python_value(&serde_json::json!({
        "schema_version": 1, "migration": SAME_TIME_ORDER,
        "affected_profile_count": count,
        "required_journal_semantics_version": SAME_TIME_ORDER_REQUIRED,
    }));
    let impact = dumps(&impact, &Options::default().sort_keys(true))
        .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
    observer(&format!("{SAME_TIME_ORDER}.before_audit"))?;
    let inserted = tx.execute("INSERT OR IGNORE INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES(?1,?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [SAME_TIME_ORDER, &impact]).map_err(map_sqlite_error)?;
    if inserted != 0 {
        tx.execute(&format!("UPDATE profiles\n        SET required_journal_semantics_version = MAX(required_journal_semantics_version, {SAME_TIME_ORDER_REQUIRED})\n        WHERE {AFFECTED}"), []).map_err(map_sqlite_error)?;
    }
    observer(&format!("{SAME_TIME_ORDER}.before_commit"))?;
    tx.commit().map_err(map_sqlite_error)?;
    observer(&format!("{SAME_TIME_ORDER}.after_commit"))
}

/// `_migrate_price_currency_semantics`: invalidate, once, every processed
/// book with an active priced transaction whose currency is not the book's,
/// and require the version that holds those rows.
pub(crate) fn price_currency(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    if tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migration_audits WHERE migration_name=?)",
            [PRICE_CURRENCY],
            |r| r.get::<_, bool>(0),
        )
        .map_err(map_sqlite_error)?
    {
        return Ok(());
    }
    let priced = [
        "fiat_rate_exact",
        "fiat_rate",
        "fiat_value_exact",
        "fiat_value",
    ]
    .map(|column| format!("COALESCE(CAST(t.{column} AS TEXT), '') <> ''"))
    .join(" OR ");
    let affected = format!(
        "(last_processed_at IS NOT NULL OR EXISTS (\n        SELECT 1 FROM journal_entries WHERE profile_id = profiles.id))\n        AND EXISTS (SELECT 1 FROM transactions t WHERE t.profile_id = profiles.id\n            AND t.excluded = 0 AND ({priced}) AND NOT {})",
        crate::price_currency::price_in_book_currency_sql("t")
    );
    let count: i64 = tx
        .query_row(
            &format!("SELECT COUNT(*) FROM profiles WHERE {affected}"),
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    let impact = python_value(&serde_json::json!({
        "schema_version": 1, "migration": PRICE_CURRENCY,
        "affected_profile_count": count,
        "required_journal_semantics_version": PRICE_CURRENCY_REQUIRED,
    }));
    let impact = dumps(&impact, &Options::default().sort_keys(true))
        .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
    observer(&format!("{PRICE_CURRENCY}.before_audit"))?;
    let inserted = tx.execute("INSERT OR IGNORE INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES(?1,?1,1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [PRICE_CURRENCY, &impact]).map_err(map_sqlite_error)?;
    if inserted != 0 {
        tx.execute(&format!("UPDATE profiles SET last_processed_at = NULL,\n        last_processed_tx_count = 0, journal_input_version = journal_input_version + 1,\n        ownership_review_counts_json = NULL,\n        required_journal_semantics_version = MAX(required_journal_semantics_version, {PRICE_CURRENCY_REQUIRED})\n        WHERE {affected}"), []).map_err(map_sqlite_error)?;
    }
    observer(&format!("{PRICE_CURRENCY}.before_commit"))?;
    tx.commit().map_err(map_sqlite_error)?;
    observer(&format!("{PRICE_CURRENCY}.after_commit"))
}

pub(crate) fn legacy_pair_fee(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    if tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migration_audits WHERE migration_name=?)",
            [LEGACY_PAIR_FEE],
            |r| r.get::<_, bool>(0),
        )
        .map_err(map_sqlite_error)?
    {
        return Ok(());
    }
    // Python's candidate query in custody_authored_migration.py.
    let candidates: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM custody_components component
             JOIN custody_component_legs fee ON fee.component_id = component.id
             WHERE component.state = 'active'
               AND component.conservation_mode = 'quantity'
               AND fee.role = 'fee' AND fee.notes = 'reviewed_residual:network_fee')",
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    if candidates {
        return Err(Error::new(
            ErrorKind::UnsupportedSchemaLayout,
            "legacy pair fee review requires the custody component validator; migrate this book with Python",
        )
        .hint("Open this book with the Python build of Kassiber to migrate it first, then retry the Rust binary.")
        .details(serde_json::json!({
            "migration_step": "mark_legacy_pair_fee_reviews",
            "reason": "legacy_pair_fee_candidates",
        })));
    }
    let impact = python_value(&serde_json::json!({
        "schema_version": 1, "migration": LEGACY_PAIR_FEE,
        "affected_component_count": 0, "affected_profile_count": 0,
        "required_journal_semantics_version": LEGACY_PAIR_FEE_REQUIRED,
        "explanation": "Legacy pair fee allocations require review; authored economics and retained journals are unchanged.",
    }));
    let impact = dumps(&impact, &Options::default().sort_keys(true))
        .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
    observer(&format!("{LEGACY_PAIR_FEE}.before_audit"))?;
    // Python stamps now_iso(): whole seconds with a Z suffix.
    tx.execute("INSERT INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES(?1,?1,1,?2,strftime('%Y-%m-%dT%H:%M:%SZ','now'))", [LEGACY_PAIR_FEE, &impact]).map_err(map_sqlite_error)?;
    observer(&format!("{LEGACY_PAIR_FEE}.before_commit"))?;
    tx.commit().map_err(map_sqlite_error)?;
    observer(&format!("{LEGACY_PAIR_FEE}.after_commit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_version_refuses_older_stamps_even_with_a_lower_requirement() {
        let conn = Connection::open_in_memory().unwrap();
        for required in [
            1,
            2,
            LEGACY_PAIR_FEE_REQUIRED,
            SAME_TIME_ORDER_REQUIRED,
            PRICE_CURRENCY_REQUIRED,
        ] {
            assert!(!compatible(required, 0));
            assert!(!compatible(required, 1));
            // A writer stamping main's version 2 lacks 8f's semantics.
            assert!(!compatible(required, 2));
            assert!(!compatible(required, 6));
            // Version 7 ordered same-time lots by row id.
            assert!(!compatible(required, 7));
            // Version 8 read a price in another currency as the book's.
            assert!(!compatible(required, 8));
            assert!(!compatible(required, 9));
            assert!(compatible(required, 10));
            assert!(!compatible(required, 11));
        }
        assert!(compatible(0, 0));
        assert!(!compatible(11, 10));
        for required in 0..=11 {
            for built in 0..=11 {
                let sql = format!("SELECT {} FROM (SELECT ?1 AS required_journal_semantics_version, ?2 AS journal_semantics_version) p", current_sql("p"));
                let actual: bool = conn
                    .query_row(&sql, [required, built], |r| r.get(0))
                    .unwrap();
                assert_eq!(
                    actual,
                    compatible(required, built),
                    "required={required} built={built}"
                );
            }
        }
    }
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
    fn outflow_v2_preserves_evidence_and_rejects_old_writer_publication() {
        for kind in ["disposal", "fee", "transfer_fee"] {
            let (_root, mut book) = book();
            seed(&book.connection);
            book.connection.execute("INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,transaction_id,entry_type,occurred_at,asset,quantity,created_at) VALUES('out','ws','at','wal','wage',?,'2024-01-01','BTC',-1,'fixed')",[kind]).unwrap();
            book.connection.execute_batch("INSERT INTO schema_migration_audits VALUES('at-fee-consumed-regime-v1','at-fee-consumed-regime-v1',1,'{}','fixed'); DELETE FROM schema_migration_audits WHERE migration_name='at-outflow-consumed-regime-v2'; UPDATE profiles SET journal_semantics_version=1,required_journal_semantics_version=1;").unwrap();
            migrate(&mut book.connection).unwrap();
            let conn = &book.connection;
            assert_eq!(conn.query_row("SELECT required_journal_semantics_version,journal_input_version FROM profiles WHERE id='at'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(2,1));
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM journal_entries WHERE id='out' AND quantity=-1",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM transactions", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            // Python's current calculation semantics are version 7; the
            // Austrian consumed-regime writer's version 2 is no longer current.
            for writer in [0, 1, 2, 7, 1, 7, 2] {
                conn.execute_batch("UPDATE profiles SET last_processed_at='same',last_processed_input_version=journal_input_version WHERE id='at'").unwrap();
                assert!(!journal_current(conn, "at").unwrap());
                conn.execute(
                    "UPDATE profiles SET journal_semantics_version=? WHERE id='at'",
                    [writer],
                )
                .unwrap();
                assert_eq!(
                    journal_current(conn, "at").unwrap(),
                    writer == JOURNAL_CALCULATION_SEMANTICS_VERSION
                );
            }
            let changes = conn.total_changes();
            migrate(&mut book.connection).unwrap();
            assert_eq!(book.connection.total_changes(), changes);
        }
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

    #[test]
    fn price_currency_invalidates_only_processed_books_holding_another_currency() {
        let (_root, mut book) = book();
        book.connection.execute_batch("INSERT INTO workspaces VALUES('ws','synthetic','fixed');
            INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,last_processed_at,journal_semantics_version,required_journal_semantics_version,created_at) VALUES
              ('switched','ws','Switched','USD','generic','FIFO','fixed',8,8,'fixed'),
              ('austria','ws','Austria','EUR','at','FIFO','fixed',8,8,'fixed'),
              ('spelled','ws','Spelled',' usd ','generic','FIFO','fixed',8,8,'fixed'),
              ('unpriced','ws','Unpriced','USD','generic','FIFO','fixed',8,8,'fixed'),
              ('excluded','ws','Excluded','USD','generic','FIFO','fixed',8,8,'fixed'),
              ('unprocessed','ws','Unprocessed','USD','generic','FIFO',NULL,0,0,'fixed');
            INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','switched','Wallet','custom','fixed');
            INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fiat_currency,fiat_rate,fiat_value_exact,excluded,created_at) VALUES
              ('a','ws','switched','wal','a','fixed','inbound','BTC',1,'EUR',40000,NULL,0,'fixed'),
              ('b','ws','austria','wal','b','fixed','inbound','BTC',1,'USD',NULL,'5',0,'fixed'),
              ('c','ws','spelled','wal','c','fixed','inbound','BTC',1,'USD ',40000,NULL,0,'fixed'),
              ('d','ws','unpriced','wal','d','fixed','inbound','BTC',1,'EUR',NULL,'',0,'fixed'),
              ('e','ws','excluded','wal','e','fixed','inbound','BTC',1,'EUR',40000,NULL,1,'fixed'),
              ('f','ws','unprocessed','wal','f','fixed','inbound','BTC',1,'EUR',40000,NULL,0,'fixed');
            DELETE FROM schema_migration_audits WHERE migration_name='price-currency-mismatch-hold-v1';").unwrap();
        for _ in 0..2 {
            price_currency(&mut book.connection, &mut |_| Ok(())).unwrap();
        }
        let profiles: Vec<(String, Option<String>, i64, i64)> = book
            .connection
            .prepare("SELECT id,last_processed_at,journal_input_version,required_journal_semantics_version FROM profiles ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        let invalidated = |id: &str| (id.to_owned(), None, 1, PRICE_CURRENCY_REQUIRED);
        let kept = |id: &str| (id.to_owned(), Some("fixed".to_owned()), 0, 8);
        assert_eq!(
            profiles,
            [
                invalidated("austria"),
                kept("excluded"),
                kept("spelled"),
                invalidated("switched"),
                kept("unpriced"),
                ("unprocessed".to_owned(), None, 0, 0),
            ]
        );
        let audit: String = book
            .connection
            .query_row("SELECT impact_json FROM schema_migration_audits WHERE id='price-currency-mismatch-hold-v1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            audit,
            r#"{"affected_profile_count": 2, "migration": "price-currency-mismatch-hold-v1", "required_journal_semantics_version": 10, "schema_version": 1}"#
        );
    }
}
