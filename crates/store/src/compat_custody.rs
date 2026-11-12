//! Durable custody identity and evidence backfills. Observations never grant
//! ownership here: only authored snapshots and anchors can create commitments.
use super::{columns, data, ensure_column, python_json as py, repairs};
use crate::{map_sqlite_error, Error, ErrorKind, Result};
use kassiber_pyjson::{Key, Value};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn identifier(parts: Vec<Value>) -> Result<String> {
    let encoded = kassiber_pyjson::dumps(
        &Value::List(parts),
        &kassiber_pyjson::Options::default().compact(),
    )
    .map_err(|error| {
        crate::stored_values::refusal(match error {
            kassiber_pyjson::Error::Surrogate => crate::stored_values::Shape::LoneSurrogate,
            _ => crate::stored_values::Shape::Other,
        })
    })?;
    Ok(hash(encoded.as_bytes()))
}
fn relation_id(review: &str, role: &str, transaction: &str) -> Result<String> {
    identifier(vec![
        "custody-gap-review-transaction-v2".into(),
        review.into(),
        role.into(),
        transaction.into(),
    ])
}
fn read_json(raw: &str, invalid_empty: bool) -> Result<Value> {
    match py::loads(if raw.is_empty() { "{}" } else { raw }) {
        Ok(value) => Ok(value),
        Err(py::ParseError::Invalid) if invalid_empty => Ok(Value::Null),
        Err(_) => Err(crate::stored_values::refusal(
            crate::stored_values::Shape::Other,
        )),
    }
}
fn sqls(name: &str) -> Vec<&'static str> {
    data()["functions"][name]["sql"]
        .as_array()
        .expect("checked custody SQL")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}
fn sql(name: &str, prefix: &str) -> &'static str {
    let values: Vec<_> = sqls(name)
        .into_iter()
        .filter(|sql| sql.trim_start().starts_with(prefix))
        .collect();
    assert_eq!(values.len(), 1, "ambiguous SQL in {name}: {prefix}");
    values[0]
}
fn execute(conn: &Connection, sql: &str) -> Result<()> {
    conn.execute_batch(sql).map_err(map_sqlite_error)
}

pub(crate) fn relation_schema(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let old = columns(conn, "custody_gap_review_transactions")?
        .iter()
        .any(|(name, _)| name == "ordinal");
    let legacy:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='custody_gap_review_transactions_v1')",[],|r|r.get(0)).map_err(map_sqlite_error)?;
    if !old && !legacy {
        return Ok(());
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    for (kind, name) in [
        ("TRIGGER", "trg_custody_gap_review_transaction_scope_insert"),
        ("TRIGGER", "trg_custody_gap_review_transactions_immutable"),
        ("INDEX", "idx_custody_gap_review_transactions_review"),
        ("INDEX", "idx_custody_gap_review_transactions_scope"),
    ] {
        execute(&tx, &format!("DROP {kind} IF EXISTS {name}"))?;
    }
    observer("review-schema.after_drop_objects")?;
    if old {
        if legacy {
            return Err(Error::new(
                ErrorKind::CustodyReviewRelationMigrationConflict,
                "Custody review relation migration state is ambiguous.",
            ));
        }
        execute(&tx,"ALTER TABLE custody_gap_review_transactions RENAME TO custody_gap_review_transactions_v1")?;
    }
    observer("review-schema.after_rename")?;
    for sql in sqls("_create_custody_gap_review_transaction_table") {
        execute(&tx, sql)?;
    }
    let mut statement = tx
        .prepare(sql(
            "_ensure_custody_gap_review_transaction_schema",
            "SELECT id, review_id",
        ))
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(statement);
    for (review, workspace, profile, role, transaction, created) in rows {
        tx.execute(
            sql(
                "_ensure_custody_gap_review_transaction_schema",
                "INSERT OR IGNORE",
            ),
            params![
                relation_id(&review, &role, &transaction)?,
                review,
                workspace,
                profile,
                role,
                transaction,
                created
            ],
        )
        .map_err(map_sqlite_error)?;
    }
    observer("review-schema.after_copy")?;
    execute(&tx, "DROP TABLE custody_gap_review_transactions_v1")?;
    for sql in sqls("_create_custody_gap_review_transaction_aux_schema") {
        execute(&tx, sql)?;
    }
    observer("review-schema.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("review-schema.after_commit")
}

pub(crate) fn review_relations(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let mut statement = tx
        .prepare(sql(
            "_backfill_custody_gap_review_transactions",
            "SELECT id, workspace_id",
        ))
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(statement);
    for (review, workspace, profile, component, snapshot, created) in rows {
        let snapshot = read_json(snapshot.as_deref().unwrap_or("{}"), true)?;
        let mut relations = [BTreeSet::new(), BTreeSet::new()];
        for (index, name) in ["source_ids", "return_ids"].into_iter().enumerate() {
            if let Some(Value::List(items)) = py::get(&snapshot, name) {
                for item in items {
                    if let Value::Str(text) = item {
                        let text = text.as_str().ok_or_else(|| {
                            crate::stored_values::refusal(
                                crate::stored_values::Shape::LoneSurrogate,
                            )
                        })?;
                        if !text.is_empty() {
                            relations[index].insert(text.to_owned());
                        }
                    }
                }
            }
        }
        if let Some(component) = component.filter(|text| !text.is_empty()) {
            let mut legs = tx
                .prepare(sql(
                    "_backfill_custody_gap_review_transactions",
                    "SELECT role, COALESCE",
                ))
                .map_err(map_sqlite_error)?;
            for row in legs
                .query_map(params![component, workspace, profile], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(map_sqlite_error)?
            {
                let (role, id) = row.map_err(map_sqlite_error)?;
                relations[usize::from(role == "destination")].insert(id);
            }
        }
        let mut statement=tx.prepare("SELECT role,transaction_id FROM custody_gap_review_transactions WHERE review_id=? ORDER BY role,transaction_id,id").map_err(map_sqlite_error)?;
        let existing = statement
            .query_map([&review], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .map_err(map_sqlite_error)?;
        drop(statement);
        for (role, ids) in ["source", "return"].into_iter().zip(&relations) {
            for id in ids {
                if existing.contains(&(role.to_owned(), id.clone())) {
                    continue;
                }
                let collision:bool=tx.query_row("SELECT 1 FROM transactions WHERE id=? AND (workspace_id!=? OR profile_id!=?) LIMIT 1",params![id,workspace,profile],|_|Ok(true)).optional().map_err(map_sqlite_error)?.unwrap_or(false);
                if collision {
                    continue;
                }
                tx.execute("INSERT OR IGNORE INTO custody_gap_review_transactions(id,review_id,workspace_id,profile_id,role,transaction_id,created_at) VALUES(?,?,?,?,?,?,?)",params![relation_id(&review,role,id)?,review,workspace,profile,role,id,created]).map_err(map_sqlite_error)?;
                observer("review-relations.after_insert")?;
            }
        }
        if !relations[0].is_empty() || !relations[1].is_empty() {
            let id = identifier(vec![
                "custody-gap-review-relation-set-v1".into(),
                review.clone().into(),
            ])?;
            tx.execute("INSERT OR IGNORE INTO custody_gap_review_relation_sets(id,review_id,workspace_id,profile_id,expected_source_count,expected_return_count,created_at) VALUES(?,?,?,?,?,?,?)",params![id,review,workspace,profile,relations[0].len() as i64,relations[1].len() as i64,created]).map_err(map_sqlite_error)?;
        }
    }
    observer("review-relations.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("review-relations.after_commit")
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn evidence(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<usize> {
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let migrated = evidence_rows(&tx, observer)?;
    observer("evidence.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("evidence.after_commit")?;
    Ok(migrated)
}
fn evidence_rows(tx: &Connection, observer: &mut impl FnMut(&str) -> Result<()>) -> Result<usize> {
    let function = "_backfill_local_custody_evidence_commitments";
    let mut statement = tx
        .prepare(sql(function, "SELECT c.id"))
        .map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(statement);
    let mut migrated = 0;
    for (component, workspace, profile, state, created) in rows {
        let mut statement = tx
            .prepare(sql(function, "SELECT quantity_hash"))
            .map_err(map_sqlite_error)?;
        let snapshots = statement
            .query_map(params![profile, component], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sqlite_error)?;
        drop(statement);
        if !snapshots.is_empty() {
            let mut commitments = Vec::new();
            for (quantity, payload, created) in snapshots {
                let Value::Dict(mut items) = read_json(&payload, false)? else {
                    return Err(Error::new(
                        ErrorKind::MigrationVerificationFailed,
                        "custody evidence payload is invalid",
                    ));
                };
                items.retain(|(key, _)| {
                    !["fingerprint", "occurred_at", "confirmed_at", "raw_json"]
                        .iter()
                        .any(|name| key == &Key::from(*name))
                });
                let detail = hash(py::dumps(&Value::Dict(items), false, true)?.as_bytes());
                commitments.push((quantity, detail, created));
            }
            commitments.sort();
            for (ordinal, (quantity, detail, snapshot_created)) in commitments.iter().enumerate() {
                let id = identifier(vec![
                    "custody-component-evidence-v1".into(),
                    component.clone().into(),
                    ordinal.into(),
                ])?;
                tx.execute(
                    sql(function, "INSERT OR IGNORE"),
                    params![
                        id,
                        component,
                        workspace,
                        profile,
                        ordinal as i64,
                        quantity,
                        detail,
                        snapshot_created
                            .as_deref()
                            .filter(|v| !v.is_empty())
                            .unwrap_or(&created)
                    ],
                )
                .map_err(map_sqlite_error)?;
                observer("evidence.after_insert")?;
            }
            tx.execute(
                "UPDATE custody_components SET expected_evidence_count=? WHERE id=?",
                params![commitments.len() as i64, component],
            )
            .map_err(map_sqlite_error)?;
            migrated += 1;
        } else if state == "active" {
            let anchored: bool = tx
                .query_row(
                    sql(function, "SELECT 1 FROM custody_component_legs"),
                    [&component],
                    |_| Ok(true),
                )
                .optional()
                .map_err(map_sqlite_error)?
                .unwrap_or(false);
            if !anchored {
                tx.execute(
                    "UPDATE custody_components SET expected_evidence_count=0 WHERE id=?",
                    [&component],
                )
                .map_err(map_sqlite_error)?;
                migrated += 1;
            }
        }
        observer("evidence.after_component")?;
    }
    Ok(migrated)
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn step(
    conn: &mut Connection,
    name: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    match name {
        "relation-schema" => relation_schema(conn, observer),
        "review-relations" => review_relations(conn, observer),
        "evidence" => evidence(conn, observer).map(|_| ()),
        "durable-evidence" => durable_evidence(conn, observer),
        _ => repairs::step(conn, name, observer),
    }
}

pub(crate) fn durable_evidence(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let anchor_before = columns(conn, "custody_component_legs")?
        .iter()
        .any(|(name, _)| name == "anchor_transaction_id");
    let evidence_before = columns(conn, "custody_components")?
        .iter()
        .any(|(name, _)| name == "expected_evidence_count");
    let anchored = |conn: &Connection| -> Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM custody_component_legs WHERE anchor_transaction_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)
    };
    let commitments = |conn: &Connection| -> Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM custody_component_evidence_commitments",
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)
    };
    let anchors_before = if anchor_before { anchored(conn)? } else { 0 };
    let commitments_before = commitments(conn)?;
    // Python's ensure_column commits occurred_at before the grouped backfill.
    ensure_column(conn, "custody_component_legs", "occurred_at", "TEXT")?;
    observer("durable-evidence.after_occurred_at")?;
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let added_anchor = ensure_column(
        &tx,
        "custody_component_legs",
        "anchor_transaction_id",
        "TEXT",
    )?;
    execute(&tx, "UPDATE custody_component_legs SET anchor_transaction_id=transaction_id WHERE anchor_transaction_id IS NULL AND transaction_id IS NOT NULL")?;
    observer("durable-evidence.after_anchors")?;
    for (column, table) in [
        ("expected_leg_count", "custody_component_legs"),
        ("expected_allocation_count", "custody_component_allocations"),
        (
            "expected_economic_term_count",
            "custody_component_economic_terms",
        ),
    ] {
        if ensure_column(&tx, "custody_components", column, "INTEGER")? {
            execute(&tx, &format!("UPDATE custody_components SET {column}=(SELECT COUNT(*) FROM {table} child WHERE child.component_id=custody_components.id) WHERE {column} IS NULL"))?;
        }
    }
    let added_evidence = ensure_column(
        &tx,
        "custody_components",
        "expected_evidence_count",
        "INTEGER",
    )?;
    if ensure_column(
        &tx,
        "custody_components",
        "authored_source",
        "TEXT DEFAULT 'user'",
    )? {
        execute(&tx, "UPDATE custody_components SET authored_source='user' WHERE authored_source IS NULL OR authored_source=''")?;
    }
    observer("durable-evidence.after_headers")?;
    // Author-snapshot commitments must remain in the same transaction group.
    let migrated = evidence_rows(&tx, observer)?;
    let anchors_after = anchored(&tx)?;
    let commitments_after = commitments(&tx)?;
    if added_anchor || added_evidence || migrated != 0 {
        let name = "custody-durable-evidence-v1";
        let impact = serde_json::json!({"schema_version":1,"migration":name,"changes":[
            {"name":"durable_transaction_anchors","before":{"column_present":anchor_before,"anchored_leg_count":anchors_before},"after":{"column_present":true,"anchored_leg_count":anchors_after},"rows_changed":(anchors_after-anchors_before).max(0),"explanation":data()["custody_migration_explanations"]["durable_transaction_anchors"]},
            {"name":"payload_free_evidence_commitments","before":{"header_column_present":evidence_before,"commitment_count":commitments_before},"after":{"header_column_present":true,"commitment_count":commitments_after},"rows_changed":(commitments_after-commitments_before).max(0),"explanation":data()["custody_migration_explanations"]["payload_free_evidence_commitments"]}
        ]});
        let encoded = py::dumps(&crate::layout::python_value(&impact), true, true)?;
        tx.execute(
            repairs::statement(
                "_record_custody_durable_evidence_migration",
                "INSERT OR IGNORE",
            ),
            params![name, name, encoded],
        )
        .map_err(map_sqlite_error)?;
        observer("durable-evidence.after_audit")?;
    }
    observer("durable-evidence.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("durable-evidence.after_commit")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn review_book() -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        execute(&book.connection,"INSERT INTO workspaces VALUES('ws','Synthetic','fixed'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed'); INSERT INTO custody_gap_reviews(id,workspace_id,profile_id,gap_id,revision,candidate_fingerprint,action,snapshot_json,created_at) VALUES('review','ws','pf','gap',1,'fingerprint','dismissed','{\"source_ids\":[\"missing\",\"missing\"],\"return_ids\":[\"return\"]}','fixed');").unwrap();
        (root, book)
    }
    #[test]
    fn review_relation_backfill_preserves_existing_ids_and_commits_expected_sets() {
        let (_root, mut book) = review_book();
        execute(&book.connection,"INSERT INTO custody_gap_review_transactions VALUES('authored-id','review','ws','pf','return','return','fixed')").unwrap();
        review_relations(&mut book.connection, &mut |_| Ok(())).unwrap();
        assert_eq!(
            book.connection
                .query_row(
                    "SELECT id FROM custody_gap_review_transactions WHERE role='return'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "authored-id"
        );
        assert_eq!(
            book.connection
                .query_row(
                    "SELECT id FROM custody_gap_review_transactions WHERE role='source'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            relation_id("review", "source", "missing").unwrap()
        );
        assert_eq!(book.connection.query_row("SELECT expected_source_count,expected_return_count FROM custody_gap_review_relation_sets",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(1,1));
        let changes = book.connection.total_changes();
        review_relations(&mut book.connection, &mut |_| Ok(())).unwrap();
        assert_eq!(book.connection.total_changes(), changes);
    }
    #[test]
    fn relation_rebuild_rolls_back_every_schema_boundary() {
        for target in [
            "review-schema.after_drop_objects",
            "review-schema.after_rename",
            "review-schema.after_copy",
            "review-schema.before_commit",
            "review-schema.after_commit",
        ] {
            let (_root, mut book) = review_book();
            let conn = &mut book.connection;
            execute(conn,"DROP TABLE custody_gap_review_transactions;CREATE TABLE custody_gap_review_transactions(id TEXT PRIMARY KEY,review_id TEXT,workspace_id TEXT,profile_id TEXT,ordinal INTEGER,role TEXT,transaction_id TEXT,created_at TEXT);INSERT INTO custody_gap_review_transactions VALUES('old-id','review','ws','pf',0,'source','missing','fixed');").unwrap();
            assert!(relation_schema(conn, &mut |label| if label == target {
                Err(Error::new(ErrorKind::Io, "injected relation failure"))
            } else {
                Ok(())
            })
            .is_err());
            assert!(conn.is_autocommit());
            assert_eq!(
                columns(conn, "custody_gap_review_transactions")
                    .unwrap()
                    .iter()
                    .any(|(name, _)| name == "ordinal"),
                target != "review-schema.after_commit"
            );
            relation_schema(conn, &mut |_| Ok(())).unwrap();
            assert_eq!(
                conn.query_row("SELECT id FROM custody_gap_review_transactions", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                relation_id("review", "source", "missing").unwrap()
            );
        }
    }
    #[test]
    fn current_custody_schemas_need_no_identity_or_evidence_changes() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let changes = book.connection.total_changes();
        for name in ["relation-schema", "review-relations", "evidence"] {
            step(&mut book.connection, name, &mut |_| Ok(())).unwrap();
            assert_eq!(book.connection.total_changes(), changes);
        }
    }
    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        execute(&conn,"CREATE TABLE custody_components(id TEXT,workspace_id TEXT,profile_id TEXT,state TEXT,created_at TEXT,expected_evidence_count INTEGER);CREATE TABLE custody_component_legs(component_id TEXT,transaction_id TEXT,anchor_transaction_id TEXT);CREATE TABLE custody_authored_evidence_snapshots(profile_id TEXT,subject_kind TEXT,subject_id TEXT,quantity_hash TEXT,detail_hash TEXT,payload_json TEXT,created_at TEXT);CREATE TABLE custody_component_evidence_commitments(id TEXT PRIMARY KEY,component_id TEXT,workspace_id TEXT,profile_id TEXT,ordinal INTEGER,quantity_hash TEXT,detail_hash TEXT,created_at TEXT);INSERT INTO custody_components VALUES('author','ws','pf','active','fixed',NULL),('unbound','ws','pf','active','fixed',NULL),('transactionless','ws','pf','active','fixed',NULL),('draft','ws','pf','draft','fixed',NULL);INSERT INTO custody_component_legs VALUES('author','tx','tx'),('unbound','tx','tx');INSERT INTO custody_authored_evidence_snapshots VALUES('pf','custody_component','author','quantity','old-detail','{\"stable\":1,\"fingerprint\":\"volatile\",\"raw_json\":{}}','');").unwrap();
        conn
    }
    #[test]
    fn evidence_uses_only_author_snapshots_and_transactionless_active_components() {
        let mut conn = fixture();
        assert_eq!(evidence(&mut conn, &mut |_| Ok(())).unwrap(), 2);
        assert_eq!(
            conn.query_row(
                "SELECT detail_hash FROM custody_component_evidence_commitments",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            hash(b"{\"stable\":1}")
        );
        assert_eq!(
            conn.query_row(
                "SELECT created_at FROM custody_component_evidence_commitments",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "fixed"
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM custody_components WHERE expected_evidence_count IS NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(evidence(&mut conn, &mut |_| Ok(())).unwrap(), 0);
    }
    #[test]
    fn evidence_failures_leave_no_partial_authored_commitments() {
        for target in [
            "evidence.after_insert",
            "evidence.after_component",
            "evidence.before_commit",
            "evidence.after_commit",
        ] {
            let mut conn = fixture();
            assert!(evidence(&mut conn, &mut |label| if label == target {
                Err(Error::new(ErrorKind::Io, "injected evidence failure"))
            } else {
                Ok(())
            })
            .is_err());
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM custody_component_evidence_commitments",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, i64::from(target == "evidence.after_commit"));
            evidence(&mut conn, &mut |_| Ok(())).unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM custody_component_evidence_commitments",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
    }
}
