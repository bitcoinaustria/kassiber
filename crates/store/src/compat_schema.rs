//! Ordered schema repairs. SQL comes from the reference; control flow is Rust.
use super::{columns, data, ensure_column};
use crate::{map_sqlite_error, Error, ErrorKind, Result};
use rusqlite::{Connection, OptionalExtension};

pub(super) fn statements(function: &str) -> Vec<&'static str> {
    data()["functions"][function]["sql"]
        .as_array()
        .expect("checked SQL")
        .iter()
        .map(|value| value.as_str().expect("checked SQL"))
        .collect()
}
pub(super) fn statement(function: &str, prefix: &str) -> &'static str {
    let values: Vec<_> = statements(function)
        .into_iter()
        .filter(|sql| sql.trim_start().starts_with(prefix))
        .collect();
    assert_eq!(values.len(), 1, "ambiguous SQL: {function}: {prefix}");
    values[0]
}
fn execute(conn: &Connection, sql: &str) -> Result<()> {
    conn.execute_batch(sql).map_err(map_sqlite_error)
}
/// SQLite stores CREATE definitions without IF NOT EXISTS or a final semicolon.
/// Only exact matches avoid replacement; historical spellings are still repaired.
pub(super) fn definition_matches(
    conn: &Connection,
    kind: &str,
    name: &str,
    sql: &str,
) -> Result<bool> {
    let actual: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type=? AND name=?",
            [kind, name],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite_error)?;
    let expected = sql
        .trim()
        .trim_end_matches(';')
        .replacen("IF NOT EXISTS ", "", 1);
    Ok(actual.as_deref().map(str::trim_end) == Some(expected.as_str()))
}
fn created_index_name(sql: &str) -> Option<&str> {
    let tail = sql.trim_start().strip_prefix("CREATE ")?;
    let tail = tail.strip_prefix("UNIQUE ").unwrap_or(tail);
    let tail = tail.strip_prefix("INDEX ")?;
    let tail = tail.strip_prefix("IF NOT EXISTS ").unwrap_or(tail);
    tail.split_whitespace().next()
}
pub(super) fn unchanged_index_statement(
    conn: &Connection,
    sql: &str,
    script: &[&str],
) -> Result<bool> {
    if let Some(name) = created_index_name(sql) {
        return definition_matches(conn, "index", name, sql);
    }
    let Some(name) = sql.trim().strip_prefix("DROP INDEX IF EXISTS ") else {
        return Ok(false);
    };
    let name = name.trim_end_matches(';');
    match script
        .iter()
        .find(|sql| created_index_name(sql) == Some(name))
    {
        Some(sql) => definition_matches(conn, "index", name, sql),
        None => Ok(false), // An obsolete index still needs removal.
    }
}

fn table_sql(conn: &Connection, table: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT COALESCE(sql,'') FROM sqlite_master WHERE type='table' AND name=?",
        [table],
        |row| row.get(0),
    )
    .optional()
    .map_err(map_sqlite_error)
}
fn install_columns(conn: &Connection, function: &str) -> Result<()> {
    for column in data()["functions"][function]["columns"]
        .as_array()
        .expect("columns")
    {
        ensure_column(
            conn,
            column[0].as_str().unwrap(),
            column[1].as_str().unwrap(),
            column[2].as_str().unwrap(),
        )?;
    }
    Ok(())
}
// Keep every rebuild atomic and restore connection-local pragmas on all errors.
fn rebuild(
    conn: &mut Connection,
    legacy: bool,
    observer: &mut impl FnMut(&str) -> Result<()>,
    f: impl FnOnce(&Connection, &mut dyn FnMut(&str) -> Result<()>) -> Result<()>,
) -> Result<()> {
    if !conn.is_autocommit() {
        execute(conn, "COMMIT")?;
    }
    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    let previous: i64 = conn
        .query_row("PRAGMA legacy_alter_table", [], |row| row.get(0))
        .map_err(map_sqlite_error)?;
    execute(conn, "PRAGMA foreign_keys=OFF")?;
    if legacy {
        execute(conn, "PRAGMA legacy_alter_table=ON")?;
    }
    let result = (|| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        f(&tx, observer)?;
        observer("repair.before_commit")?;
        tx.commit().map_err(map_sqlite_error)?;
        observer("repair.after_commit")
    })();
    let restored = execute(
        conn,
        &format!("PRAGMA legacy_alter_table={previous};PRAGMA foreign_keys={fk}"),
    );
    result.and(restored)
}

pub(crate) fn migrate_attachments(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let function = "_migrate_attachment_table_shape";
    let current = table_sql(conn, "attachments")?.unwrap_or_default();
    let (legacy, present) = if table_sql(conn, "attachments_legacy_shape")?.is_some() {
        ("attachments_legacy_shape", true)
    } else if table_sql(conn, "attachments_legacy_notnull_tx")?.is_some() {
        ("attachments_legacy_notnull_tx", true)
    } else {
        ("attachments_legacy_notnull_tx", false)
    };
    let mut fks = conn
        .prepare("PRAGMA foreign_key_list(attachments)")
        .map_err(map_sqlite_error)?;
    let copied = fks
        .query_map([], |row| row.get::<_, String>(3))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sqlite_error)?;
    drop(fks);
    if present
        || current.contains("transaction_id TEXT NOT NULL")
        || current.contains("label TEXT NOT NULL")
        || copied
            .iter()
            .any(|name| name == "copied_from_attachment_id" || name == "copied_from_transaction_id")
    {
        rebuild(conn, true, observer, |conn, observe| {
            observe("attachment.before_rename")?;
            execute(
                conn,
                &if present {
                    "DROP TABLE IF EXISTS attachments".into()
                } else {
                    format!("ALTER TABLE attachments RENAME TO {legacy}")
                },
            )?;
            observe("attachment.after_rename")?;
            execute(conn, statement(function, "CREATE TABLE attachments"))?;
            observe("attachment.after_create")?;
            let old = columns(conn, legacy)?;
            let source = |field: &str| {
                if old.iter().any(|(name, _)| name == field) {
                    field.to_owned()
                } else {
                    "NULL".into()
                }
            };
            execute(conn,&format!("INSERT OR IGNORE INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,copied_from_transaction_id,created_at) SELECT id,workspace_id,profile_id,transaction_id,attachment_type,CASE WHEN attachment_type='url' AND label=source_url THEN NULL ELSE label END,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,{},{},created_at FROM {legacy}",source("copied_from_attachment_id"),source("copied_from_transaction_id")))?;
            observe("attachment.after_copy")?;
            execute(conn, &format!("DROP TABLE {legacy}"))?;
            execute(conn, statement(function, "CREATE INDEX"))?;
            observe("attachment.after_drop")
        })?;
    }
    repair_attachment_children(conn, observer)
}
fn repair_attachment_children(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut broken = Vec::new();
    for child in data()["functions"]["_repair_attachment_child_fks"]["constants"]["child_tables"]
        .as_array()
        .expect("attachment children")
    {
        let table = child[0].as_str().unwrap();
        let mut stmt = conn
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .map_err(map_sqlite_error)?;
        let targets = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(2)?, row.get::<_, String>(3)?))
            })
            .map_err(map_sqlite_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(map_sqlite_error)?;
        if targets
            .iter()
            .any(|(target, field)| field == "attachment_id" && target != "attachments")
        {
            broken.push(child);
        }
    }
    if broken.is_empty() {
        return Ok(());
    }
    rebuild(conn, true, observer, |conn, observe| {
        for child in broken {
            let table = child[0].as_str().unwrap();
            let fields = child[2].as_str().unwrap();
            execute(
                conn,
                &format!("ALTER TABLE {table} RENAME TO {table}__legacy_attachment_fk"),
            )?;
            observe("attachment_child.after_rename")?;
            execute(conn, child[1].as_str().unwrap())?;
            execute(conn,&format!("INSERT OR IGNORE INTO {table} ({fields}) SELECT {fields} FROM {table}__legacy_attachment_fk"))?;
            observe("attachment_child.after_copy")?;
            execute(conn, &format!("DROP TABLE {table}__legacy_attachment_fk"))?;
            observe("attachment_child.after_drop")?;
        }
        Ok(())
    })
}
pub(crate) fn migrate_pair_uniques(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if !table_sql(conn, "transaction_pairs")?
        .unwrap_or_default()
        .contains("UNIQUE (profile_id, out_transaction_id)")
    {
        return Ok(());
    }
    let has_component = columns(conn, "transaction_pairs")?
        .iter()
        .any(|(name, _)| name == "component_id");
    rebuild(conn, false, observer, |conn, observe| {
        execute(
            conn,
            "ALTER TABLE transaction_pairs RENAME TO transaction_pairs_legacy",
        )?;
        observe("pair.after_rename")?;
        execute(
            conn,
            statement("_migrate_legacy_transaction_pairs_uniques", "CREATE TABLE"),
        )?;
        let component = if has_component {
            "component_id"
        } else {
            "NULL"
        };
        execute(conn,&format!("INSERT INTO transaction_pairs(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,policy,notes,component_id,created_at) SELECT id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,policy,notes,{component},created_at FROM transaction_pairs_legacy"))?;
        observe("pair.after_copy")?;
        execute(conn, "DROP TABLE transaction_pairs_legacy")?;
        observe("pair.after_drop")
    })
}
pub(crate) fn migrate_recipient_unique(conn: &mut Connection) -> Result<()> {
    if !table_sql(conn, "source_funds_recipients")?
        .unwrap_or_default()
        .contains("UNIQUE (profile_id, label)")
    {
        return Ok(());
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    for sql in statements("_drop_legacy_source_funds_recipients_unique")
        .into_iter()
        .skip(1)
    {
        execute(&tx, sql)?;
    }
    tx.commit().map_err(map_sqlite_error)
}
pub(crate) fn projection_shapes(conn: &mut Connection) -> Result<()> {
    let mut pending = Vec::new();
    for (table, candidates) in data()["functions"]["_ensure_custody_projection_table_shapes"]
        ["constants"]["semantic_columns"]
        .as_object()
        .expect("projection columns")
    {
        let existing = columns(conn, table)?;
        for column in candidates
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
        {
            if existing.iter().any(|(name, _)| name == column) {
                pending.push((table, column));
            }
        }
    }
    if pending.is_empty() {
        return Ok(());
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    execute(
        &tx,
        "DROP VIEW IF EXISTS journal_custody_projection_relations",
    )?;
    for (table, column) in pending {
        execute(
            &tx,
            &format!("ALTER TABLE \"{table}\" DROP COLUMN \"{column}\""),
        )?;
    }
    tx.commit().map_err(map_sqlite_error)
}
pub(crate) fn economic_notes(conn: &mut Connection) -> Result<()> {
    if columns(conn, "custody_component_economic_terms")?
        .iter()
        .any(|(name, _)| name == "review_notes")
    {
        return Ok(());
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    for sql in statements("_ensure_custody_economic_term_review_notes")
        .into_iter()
        .skip(1)
    {
        execute(&tx, sql)?;
    }
    tx.commit().map_err(map_sqlite_error)
}
fn immutability_sql(conn: &Connection) -> Result<()> {
    let function = "_ensure_custody_revision_immutability_triggers";
    for sql in statements(function) {
        let trigger = sql.split_whitespace().nth(2).expect("trigger name");
        execute(conn, &format!("DROP TRIGGER IF EXISTS {trigger}"))?;
    }
    for sql in statements(function) {
        execute(conn, sql)?;
    }
    Ok(())
}
pub(crate) fn immutability(conn: &mut Connection) -> Result<()> {
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    let mut current = tx
        .prepare("SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?")
        .map_err(map_sqlite_error)?;
    let mut matches = true;
    for sql in statements("_ensure_custody_revision_immutability_triggers") {
        let trigger = sql.split_whitespace().nth(2).expect("trigger name");
        let actual: Option<String> = current
            .query_row([trigger], |row| row.get(0))
            .optional()
            .map_err(map_sqlite_error)?;
        if actual.as_deref() != Some(sql.trim().trim_end_matches(';')) {
            matches = false;
            break;
        }
    }
    drop(current);
    if !matches {
        immutability_sql(&tx)?;
    }
    tx.commit().map_err(map_sqlite_error)
}
fn custody_fk_check(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA foreign_key_check")
        .map_err(map_sqlite_error)?;
    for table in stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(map_sqlite_error)?
    {
        if [
            "custody_component_legs",
            "custody_component_allocations",
            "custody_component_economic_terms",
        ]
        .contains(&table.map_err(map_sqlite_error)?.as_str())
        {
            return Err(Error::new(
                ErrorKind::CustodySchemaMigrationFailed,
                "custody component schema migration left invalid references",
            )
            .details(serde_json::json!({"migration":"custody_leg_suspense_role"})));
        }
    }
    Ok(())
}
pub(crate) fn leg_roles(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if table_sql(conn, "custody_component_legs")?.is_some_and(|text| text.contains("'suspense'")) {
        return Ok(());
    }
    let function = "_rebuild_custody_leg_role_schema";
    let tables = [
        ("custody_component_legs", "leg_columns"),
        ("custody_component_allocations", "allocation_columns"),
        ("custody_component_economic_terms", "term_columns"),
    ];
    let counts = |conn: &Connection| -> Result<Vec<i64>> {
        tables
            .iter()
            .map(|(table, _)| {
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(map_sqlite_error)
            })
            .collect()
    };
    let before = counts(conn)?;
    let result = rebuild(conn, false, observer, |conn, observe| {
        for item in data()["functions"][function]["loops"].as_array().unwrap() {
            let kind = match item["target"].as_str() {
                Some("trigger") => "TRIGGER",
                Some("index") => "INDEX",
                _ => continue,
            };
            for name in item["values"].as_array().unwrap() {
                execute(
                    conn,
                    &format!("DROP {kind} IF EXISTS {}", name.as_str().unwrap()),
                )?;
            }
        }
        observe("leg-roles.after_drop_objects")?;
        for sql in statements(function)
            .into_iter()
            .filter(|s| s.trim_start().starts_with("ALTER TABLE"))
        {
            execute(conn, sql)?;
        }
        observe("leg-roles.after_rename")?;
        for sql in statements(function)
            .into_iter()
            .filter(|s| s.trim_start().starts_with("CREATE TABLE"))
        {
            execute(conn, sql)?;
        }
        observe("leg-roles.after_create")?;
        for (table, key) in tables {
            let fields = data()["functions"][function]["constants"][key]
                .as_str()
                .unwrap();
            execute(
                conn,
                &format!(
                    "INSERT INTO {table}({fields}) SELECT {fields} FROM {table}__pre_suspense"
                ),
            )?;
        }
        observe("leg-roles.after_copy")?;
        if counts(conn)? != before {
            return Err(Error::new(
                ErrorKind::CustodySchemaMigrationFailed,
                "custody child row counts changed",
            ));
        }
        for sql in statements(function)
            .into_iter()
            .filter(|s| s.trim_start().starts_with("DROP TABLE"))
        {
            execute(conn, sql)?;
        }
        for sql in statements(function).into_iter().filter(|s| {
            s.trim_start().starts_with("CREATE INDEX")
                || s.trim_start().starts_with("CREATE TRIGGER")
        }) {
            execute(conn, sql)?;
        }
        let surrounding = "_create_custody_leg_indexes_and_scope_triggers";
        for item in data()["functions"][surrounding]["loops"]
            .as_array()
            .unwrap()
        {
            if item["target"] == "statement" {
                for sql in item["values"].as_array().unwrap() {
                    execute(conn, sql.as_str().unwrap())?;
                }
            }
        }
        for sql in statements(surrounding) {
            execute(conn, sql)?;
        }
        immutability_sql(conn)?;
        observe("leg-roles.after_restore_objects")?;
        custody_fk_check(conn)
    });
    if result.is_err() {
        return Err(Error::new(
            ErrorKind::CustodySchemaMigrationFailed,
            "custody component schema migration failed",
        )
        .hint("Restore the database backup or retry with a compatible Kassiber build.")
        .details(serde_json::json!({"migration":"custody_leg_suspense_role"})));
    }
    custody_fk_check(conn)
}
/// Widen the custody AI audit's consent CHECK to the permission-mode and
/// `mcp_host` decisions. Rows are copied unchanged; a current table is left
/// alone, so a repeat open writes nothing.
pub(crate) fn ai_audit_consent(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let function = "_ensure_custody_ai_audit_consent_schema";
    let Some(current) = table_sql(conn, "custody_ai_assistance_audits")? else {
        return Ok(());
    };
    if ["mode_auto", "mode_full_access", "mcp_host"]
        .iter()
        .all(|decision| current.contains(&format!("'{decision}'")))
    {
        return Ok(());
    }
    let count = |conn: &Connection| -> Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM custody_ai_assistance_audits",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite_error)
    };
    let before = count(conn)?;
    rebuild(conn, false, observer, |conn, observe| {
        for prefix in ["DROP TRIGGER", "DROP INDEX", "ALTER TABLE"] {
            execute(conn, statement(function, prefix))?;
        }
        observe("ai-audit.after_rename")?;
        execute(conn, statement(function, "CREATE TABLE"))?;
        let fields = data()["custody_ai_audit_columns"]
            .as_str()
            .expect("custody AI audit columns");
        execute(
            conn,
            &format!(
                "INSERT INTO custody_ai_assistance_audits ({fields}) SELECT {fields} FROM custody_ai_assistance_audits__pre_consent_widening"
            ),
        )?;
        observe("ai-audit.after_copy")?;
        let after = count(conn)?;
        if after != before {
            return Err(Error::new(
                ErrorKind::CustodySchemaMigrationFailed,
                "custody AI audit migration changed the number of records",
            )
            .details(serde_json::json!({
                "migration": "custody_ai_audit_consent_decisions",
                "before": before,
                "after": after,
            })));
        }
        for prefix in ["DROP TABLE", "CREATE INDEX", "CREATE TRIGGER"] {
            execute(conn, statement(function, prefix))?;
        }
        observe("ai-audit.after_restore_objects")
    })
}
pub(crate) fn direct_payout(conn: &Connection) -> Result<()> {
    let function = "_ensure_direct_swap_payout_schema";
    execute(conn, statement(function, "CREATE TABLE"))?;
    install_columns(conn, function)?;
    for sql in statements(function).into_iter().skip(1) {
        execute(conn, sql)?;
    }
    Ok(())
}
pub(crate) fn commercial(conn: &mut Connection) -> Result<()> {
    let function = "_ensure_commercial_reconciliation_schema";
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    // origin_source is an appended compatibility column even on new tables.
    let script = statements(function);
    for sql in &script {
        let sql_trim = sql.trim_start();
        if sql_trim.starts_with("SELECT") {
            continue;
        }
        if sql_trim.starts_with(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_external_documents_profile_external_ref_unique",
        ) {
            let duplicate: bool = tx
                .query_row(statement(function, "SELECT 1"), [], |_| Ok(true))
                .optional()
                .map_err(map_sqlite_error)?
                .unwrap_or(false);
            if duplicate {
                continue;
            }
        }
        if unchanged_index_statement(&tx, sql, &script)? {
            continue;
        }
        execute(&tx, sql)?;
        if sql_trim.starts_with("CREATE TABLE IF NOT EXISTS btcpay_provenance_records") {
            for column in data()["functions"][function]["columns"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|column| column[0] == "btcpay_provenance_records")
            {
                ensure_column(
                    &tx,
                    "btcpay_provenance_records",
                    column[1].as_str().unwrap(),
                    column[2].as_str().unwrap(),
                )?;
            }
        }
        if sql_trim.starts_with("CREATE TABLE IF NOT EXISTS commercial_links") {
            for column in data()["functions"][function]["columns"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|column| column[0] == "commercial_links")
            {
                ensure_column(
                    &tx,
                    "commercial_links",
                    column[1].as_str().unwrap(),
                    column[2].as_str().unwrap(),
                )?;
            }
        }
    }
    tx.commit().map_err(map_sqlite_error)
}
pub(crate) fn freshness(conn: &Connection) -> Result<()> {
    for sql in statements("_ensure_freshness_schema") {
        execute(conn, sql)?;
    }
    for table in ["freshness_source_states", "freshness_jobs"] {
        for (column, definition) in [
            ("source_type", "TEXT NOT NULL DEFAULT 'source'"),
            ("source_label", "TEXT NOT NULL DEFAULT ''"),
            ("updated_at", "TEXT NOT NULL DEFAULT ''"),
        ] {
            ensure_column(conn, table, column, definition)?;
        }
    }
    Ok(())
}

pub(crate) fn step(
    conn: &mut Connection,
    name: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    match name {
        "leg-roles" => leg_roles(conn, observer),
        "attachments" => migrate_attachments(conn, observer),
        "pair-uniques" => migrate_pair_uniques(conn, observer),
        "recipient-unique" => migrate_recipient_unique(conn),
        "projection-shapes" => projection_shapes(conn),
        "economic-notes" => economic_notes(conn),
        "immutability" => immutability(conn),
        "direct-payout" => direct_payout(conn),
        "commercial" => commercial(conn),
        "freshness" => freshness(conn),
        "ai-audit" => ai_audit_consent(conn, observer),
        "ai-refs" | "ai-checks" | "graph-cache" | "projection-view" | "legacy-freeze"
        | "source-target" => {
            let function = match name {
                "ai-refs" => "_ensure_ai_provider_secret_refs_schema",
                "ai-checks" => "_ensure_ai_provider_checks_schema",
                "graph-cache" => "_ensure_transaction_graph_cache_schema",
                "projection-view" => "_ensure_custody_projection_relation_view",
                "legacy-freeze" => "_ensure_legacy_custody_write_freeze_triggers",
                _ => "_backfill_source_funds_target_external_id",
            };
            let tx = conn.savepoint().map_err(map_sqlite_error)?;
            if name == "projection-view"
                && definition_matches(
                    &tx,
                    "view",
                    "journal_custody_projection_relations",
                    statement(function, "CREATE VIEW"),
                )?
            {
                return tx.commit().map_err(map_sqlite_error);
            }
            for sql in statements(function) {
                execute(&tx, sql)?;
            }
            tx.commit().map_err(map_sqlite_error)
        }
        _ => Err(Error::new(
            ErrorKind::Validation,
            "unknown private compatibility step",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_immutability_is_read_only_but_missing_or_changed_triggers_are_repaired() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let conn = &mut book.connection;
        let version = |conn: &Connection| {
            conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        let original = crate::schema::fingerprint(conn).unwrap();
        let before = version(conn);
        immutability(conn).unwrap();
        assert_eq!(version(conn), before);
        for replacement in [
            "",
            "CREATE TRIGGER trg_custody_component_revision_immutable BEFORE UPDATE ON custody_components BEGIN SELECT 1; END",
        ] {
            execute(conn, "DROP TRIGGER trg_custody_component_revision_immutable").unwrap();
            execute(conn, replacement).unwrap();
            immutability(conn).unwrap();
            assert_eq!(crate::schema::fingerprint(conn).unwrap(), original);
            let before = version(conn);
            immutability(conn).unwrap();
            assert_eq!(version(conn), before);
        }
    }
    #[test]
    fn review_notes_backfill_restores_immutable_authored_terms() {
        let mut conn = Connection::open_in_memory().unwrap();
        execute(&conn,"CREATE TABLE transaction_pairs(id TEXT,profile_id TEXT,notes TEXT);CREATE TABLE direct_swap_payouts(id TEXT,profile_id TEXT,notes TEXT);CREATE TABLE custody_component_economic_terms(id TEXT,profile_id TEXT,term_kind TEXT,legacy_source_id TEXT);INSERT INTO transaction_pairs VALUES('pair','pf','pair note');INSERT INTO direct_swap_payouts VALUES('payout','pf','payout note');INSERT INTO custody_component_economic_terms VALUES('a','pf','transaction_pair','pair'),('b','pf','direct_swap_payout','payout');").unwrap();
        economic_notes(&mut conn).unwrap();
        let notes: Vec<String> = conn
            .prepare("SELECT review_notes FROM custody_component_economic_terms ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(notes, ["pair note", "payout note"]);
        let changes = conn.total_changes();
        economic_notes(&mut conn).unwrap();
        assert_eq!(conn.total_changes(), changes);
        assert!(conn
            .execute(
                "UPDATE custody_component_economic_terms SET review_notes='changed' WHERE id='a'",
                []
            )
            .is_err());
    }
    #[test]
    fn custody_leg_rebuild_rolls_back_and_restores_fk_enforcement() {
        for target in [
            "leg-roles.after_drop_objects",
            "leg-roles.after_rename",
            "leg-roles.after_create",
            "leg-roles.after_copy",
            "leg-roles.after_restore_objects",
            "repair.before_commit",
            "repair.after_commit",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut book = crate::Book::create(root.path(), None).unwrap();
            let conn = &mut book.connection;
            execute(conn,"PRAGMA foreign_keys=OFF;DROP TABLE custody_component_economic_terms;DROP TABLE custody_component_allocations;DROP TABLE custody_component_legs;").unwrap();
            for sql in statements("_rebuild_custody_leg_role_schema")
                .into_iter()
                .filter(|sql| sql.trim_start().starts_with("CREATE TABLE"))
            {
                execute(conn, &sql.replace(", 'suspense'", "")).unwrap();
            }
            execute(conn, "PRAGMA foreign_keys=ON").unwrap();
            assert!(!table_sql(conn, "custody_component_legs")
                .unwrap()
                .unwrap()
                .contains("'suspense'"));
            assert_eq!(
                leg_roles(conn, &mut |label| if label == target {
                    Err(Error::new(ErrorKind::Io, "injected leg rebuild failure"))
                } else {
                    Ok(())
                })
                .unwrap_err()
                .kind,
                ErrorKind::CustodySchemaMigrationFailed
            );
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                table_sql(conn, "custody_component_legs")
                    .unwrap()
                    .unwrap()
                    .contains("'suspense'"),
                target == "repair.after_commit"
            );
            leg_roles(conn, &mut |_| Ok(())).unwrap();
            custody_fk_check(conn).unwrap();
        }
    }
    #[test]
    fn ai_audit_consent_rebuild_copies_rows_and_rolls_back_on_failure() {
        let function = "_ensure_custody_ai_audit_consent_schema";
        // #654's CHECK, and one that already has the permission modes but
        // not `mcp_host`; either needs the rebuild.
        let narrowings = [
            "'mode_auto', 'mode_full_access',\n                        'mcp_host', ",
            "\n                        'mcp_host',",
        ];
        let targets = [
            "ai-audit.after_rename",
            "ai-audit.after_copy",
            "ai-audit.after_restore_objects",
            "repair.before_commit",
            "repair.after_commit",
            "",
        ];
        for (removed, target) in narrowings
            .iter()
            .flat_map(|removed| targets.iter().map(move |target| (*removed, *target)))
        {
            let root = tempfile::tempdir().unwrap();
            let mut book = crate::Book::create(root.path(), None).unwrap();
            let conn = &mut book.connection;
            let current = crate::schema::fingerprint(conn).unwrap();
            execute(conn, "DROP TABLE custody_ai_assistance_audits").unwrap();
            execute(
                conn,
                &statement(function, "CREATE TABLE").replace(removed, ""),
            )
            .unwrap();
            for prefix in ["CREATE INDEX", "CREATE TRIGGER"] {
                execute(conn, statement(function, prefix)).unwrap();
            }
            execute(conn, "INSERT INTO workspaces VALUES('ws','Synthetic','fixed');INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');").unwrap();
            let facts = "f".repeat(64);
            for (id, decision) in [("a", "allow_once"), ("b", "deny")] {
                conn.execute("INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,facts_sha256,consent_decision,consent_requested_at,consent_decided_at,execution_status,created_at) VALUES(?1,'ws','pf','tool','kind','call','local','m',?2,?3,'t','t','executed','t')", [id, &facts, decision]).unwrap();
            }
            let insert_mode = "INSERT INTO custody_ai_assistance_audits(id,workspace_id,profile_id,tool_name,daemon_kind,call_id,provider_kind,model,facts_sha256,consent_decision,consent_requested_at,consent_decided_at,execution_status,created_at) VALUES('c','ws','pf','tool','kind','call','local','m',?1,'mcp_host','t','t','executed','t')";
            assert!(conn.execute(insert_mode, [&facts]).is_err());
            let rows = |conn: &Connection| -> Vec<(String, String)> {
                conn.prepare(
                    "SELECT id,consent_decision FROM custody_ai_assistance_audits ORDER BY id",
                )
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
            };
            let before = rows(conn);
            let result = ai_audit_consent(conn, &mut |label| {
                if label == target {
                    Err(Error::new(ErrorKind::Io, "injected audit rebuild failure"))
                } else {
                    Ok(())
                }
            });
            assert_eq!(result.is_err(), !target.is_empty(), "{target}");
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(rows(conn), before, "{target}");
            let widened = table_sql(conn, "custody_ai_assistance_audits")
                .unwrap()
                .unwrap()
                .contains("'mcp_host'");
            assert_eq!(
                widened,
                target.is_empty() || target == "repair.after_commit",
                "{target}"
            );
            ai_audit_consent(conn, &mut |_| Ok(())).unwrap();
            let version = conn
                .query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                .unwrap();
            ai_audit_consent(conn, &mut |_| Ok(())).unwrap();
            assert_eq!(
                conn.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                version
            );
            assert_eq!(rows(conn), before);
            // The rebuilt table keeps the migration's own spelling, as in Python;
            // the structural gate admits it.
            assert_ne!(crate::schema::fingerprint(conn).unwrap(), current);
            crate::schema::verify(conn).unwrap();
            conn.execute(insert_mode, [&facts]).unwrap();
            assert!(conn
                .execute(
                    "UPDATE custody_ai_assistance_audits SET model='x' WHERE id='a'",
                    []
                )
                .is_err());
        }
    }
    #[test]
    fn every_schema_step_is_idempotent_on_a_current_book() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        for name in [
            "leg-roles",
            "attachments",
            "pair-uniques",
            "recipient-unique",
            "projection-shapes",
            "economic-notes",
            "immutability",
            "direct-payout",
            "commercial",
            "freshness",
            "ai-audit",
            "ai-refs",
            "ai-checks",
            "graph-cache",
            "projection-view",
            "legacy-freeze",
            "source-target",
        ] {
            step(&mut book.connection, name, &mut |_| Ok(())).unwrap();
            step(&mut book.connection, name, &mut |_| Ok(())).unwrap();
            assert!(book.connection.is_autocommit(), "{name}");
        }
    }
    #[test]
    fn attachment_rebuild_rolls_back_before_publication_and_restores_pragmas() {
        for target in [
            "attachment.before_rename",
            "attachment.after_rename",
            "attachment.after_create",
            "attachment.after_copy",
            "attachment.after_drop",
            "repair.before_commit",
            "repair.after_commit",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut book = crate::Book::create(root.path(), None).unwrap();
            let conn = &mut book.connection;
            execute(conn, "PRAGMA foreign_keys=OFF;DROP TABLE attachments;").unwrap();
            let ddl = statement(
                "_migrate_attachment_table_shape",
                "CREATE TABLE attachments",
            )
            .replace(
                "transaction_id TEXT REFERENCES",
                "transaction_id TEXT NOT NULL REFERENCES",
            );
            execute(conn, &ddl).unwrap();
            execute(conn, "PRAGMA foreign_keys=ON").unwrap();
            let mut seen = false;
            assert!(
                migrate_attachments(conn, &mut |label| if label == target {
                    seen = true;
                    Err(Error::new(ErrorKind::Io, "injected repair failure"))
                } else {
                    Ok(())
                })
                .is_err(),
                "{target}"
            );
            assert!(seen);
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                conn.query_row("PRAGMA legacy_alter_table", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            let old = table_sql(conn, "attachments")
                .unwrap()
                .unwrap()
                .contains("transaction_id TEXT NOT NULL");
            assert_eq!(old, target != "repair.after_commit", "{target}");
            migrate_attachments(conn, &mut |_| Ok(())).unwrap();
            assert!(!table_sql(conn, "attachments")
                .unwrap()
                .unwrap()
                .contains("transaction_id TEXT NOT NULL"));
        }
    }
}
