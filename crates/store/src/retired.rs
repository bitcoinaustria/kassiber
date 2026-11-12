//! Retire only removed device-sync storage and empty general-ledger archives.
//! Authored book rows and populated ledger retention guards are preserved.
use crate::{map_sqlite_error, Error, ErrorKind, Result};
use kassiber_pyjson::{dumps, Options};
use rusqlite::{params_from_iter, Connection};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn drop_device_sync(conn: &mut Connection) -> Result<bool> {
    drop_device_sync_with_observer(conn, &mut |_| Ok(()))
}
pub(crate) fn drop_device_sync_with_observer(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<bool> {
    let manifest: Value = serde_json::from_str(include_str!("../fixtures/retired.json"))
        .expect("retired schema contract");
    let tables: Vec<&str> = manifest["tables"]
        .as_array()
        .expect("table names")
        .iter()
        .map(|v| v.as_str().expect("table name"))
        .collect();
    let indexes: Vec<&str> = manifest["indexes"]
        .as_array()
        .expect("index names")
        .iter()
        .map(|v| v.as_str().expect("index name"))
        .collect();
    let names: Vec<&str> = tables.iter().chain(indexes.iter()).copied().collect();
    let marks = vec!["?"; names.len()].join(",");
    let present: BTreeMap<String, String> = conn
        .prepare(&format!(
            "SELECT name,type FROM sqlite_master WHERE name IN ({marks})"
        ))
        .map_err(map_sqlite_error)?
        .query_map(params_from_iter(names), |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    if present.is_empty() {
        return Ok(false);
    }
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    let mut impact = manifest["impact"].clone();
    for (field, table, condition) in [
        ("sync_enabled_book_count", "sync_books", ""),
        ("signed_event_count", "sync_events", ""),
        (
            "open_conflict_count",
            "sync_conflicts",
            "WHERE status='open'",
        ),
        ("transport_count", "sync_transports", ""),
    ] {
        let count: i64 = if present.get(table).is_some_and(|kind| kind == "table") {
            tx.query_row(
                &format!("SELECT COUNT(*) FROM {table} {condition}"),
                [],
                |r| r.get(0),
            )
            .map_err(map_sqlite_error)?
        } else {
            0
        };
        impact["changes"][0][field] = count.into();
    }
    let encoded = dumps(
        &crate::layout::python_value(&impact),
        &Options::default().sort_keys(true).compact(),
    )
    .map_err(|_| Error::new(ErrorKind::Validation, "invalid migration audit JSON"))?;
    observer("device-sync.before_audit")?;
    tx.execute("INSERT OR IGNORE INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at) VALUES('device-sync-removal-v1','device-sync-removal-v1',1,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",[encoded]).map_err(map_sqlite_error)?;
    observer("device-sync.after_audit")?;
    for (kind, names) in [("INDEX", indexes), ("TABLE", tables)] {
        for name in names {
            observer(&format!("device-sync.before_drop.{name}"))?;
            tx.execute_batch(&format!("DROP {kind} IF EXISTS {name}"))
                .map_err(map_sqlite_error)?;
            observer(&format!("device-sync.after_drop.{name}"))?;
        }
    }
    observer("device-sync.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("device-sync.after_commit")?;
    Ok(true)
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn ledger_objects(conn: &Connection) -> Result<(BTreeSet<String>, Vec<String>)> {
    let mut tables = BTreeSet::new();
    let mut triggers = Vec::new();
    let mut stmt=conn.prepare("SELECT type,name FROM sqlite_master WHERE type IN ('table','trigger') AND name LIKE 'gl\\_%' ESCAPE '\\' ORDER BY name").map_err(map_sqlite_error)?;
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(map_sqlite_error)?
    {
        let (kind, name) = row.map_err(map_sqlite_error)?;
        if kind == "table" {
            tables.insert(name);
        } else {
            triggers.push(name);
        }
    }
    Ok((tables, triggers))
}
pub fn populated_legacy_ledger_tables(conn: &Connection) -> Result<Vec<String>> {
    let (tables, _) = ledger_objects(conn)?;
    let mut found = Vec::new();
    for table in tables {
        let populated: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {})", quote(&table)),
                [],
                |r| r.get(0),
            )
            .map_err(map_sqlite_error)?;
        if populated {
            found.push(table);
        }
    }
    Ok(found)
}
/// Read-only refusal for callers implementing delete/reset/partition.
pub fn require_legacy_ledger_absent(conn: &Connection) -> Result<()> {
    if !populated_legacy_ledger_tables(conn)?.is_empty() {
        return Err(Error::new(ErrorKind::LegacyLedgerPresent,"this book holds archived general-ledger data").hint("Back up the project, then review kassiber maintenance purge-legacy-ledger before deleting, resetting or partitioning this book."));
    }
    Ok(())
}
pub fn retire_empty_legacy_ledger(conn: &mut Connection) -> Result<bool> {
    retire_ledger_with_observer(conn, &mut |_| Ok(()))
}
pub(crate) fn retire_ledger_with_observer(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<bool> {
    let (tables, triggers) = ledger_objects(conn)?;
    if tables.is_empty() && triggers.is_empty() {
        return Ok(false);
    }
    if !populated_legacy_ledger_tables(conn)?.is_empty() {
        return Ok(false);
    }
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    drop_ledger(&tx, tables, triggers, observer)?;
    observer("legacy-ledger.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("legacy-ledger.after_commit")?;
    Ok(true)
}

/// `_drop_all`: the triggers first (their retention guards included), then
/// the tables, children before the tables they reference; a reference cycle
/// falls back to deferred foreign-key checks.
fn drop_ledger(
    conn: &Connection,
    mut tables: BTreeSet<String>,
    triggers: Vec<String>,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    for trigger in triggers {
        observer(&format!("legacy-ledger.before_drop.{trigger}"))?;
        conn.execute_batch(&format!("DROP TRIGGER IF EXISTS {}", quote(&trigger)))
            .map_err(map_sqlite_error)?;
        observer(&format!("legacy-ledger.after_drop.{trigger}"))?;
    }
    conn.execute_batch("PRAGMA defer_foreign_keys=ON")
        .map_err(map_sqlite_error)?;
    while !tables.is_empty() {
        let mut referenced = BTreeSet::new();
        for table in &tables {
            let mut stmt = conn
                .prepare(&format!("PRAGMA foreign_key_list({})", quote(table)))
                .map_err(map_sqlite_error)?;
            for row in stmt
                .query_map([], |r| r.get::<_, String>(2))
                .map_err(map_sqlite_error)?
            {
                let target = row.map_err(map_sqlite_error)?;
                if target != *table {
                    referenced.insert(target);
                }
            }
        }
        let mut batch: Vec<_> = tables.difference(&referenced).cloned().collect();
        if batch.is_empty() {
            batch = tables.iter().cloned().collect();
        }
        for table in batch {
            observer(&format!("legacy-ledger.before_drop.{table}"))?;
            conn.execute_batch(&format!("DROP TABLE IF EXISTS {}", quote(&table)))
                .map_err(map_sqlite_error)?;
            observer(&format!("legacy-ledger.after_drop.{table}"))?;
            tables.remove(&table);
        }
    }
    Ok(())
}

/// `legacy_ledger.purge_plan`: the leftover tables that hold rows with their
/// counts (in name order), and the table, trigger and row totals.
pub struct LedgerPurgePlan {
    pub tables: Vec<(String, i64)>,
    pub table_count: usize,
    pub trigger_count: usize,
    pub row_count: i64,
}

pub fn legacy_ledger_purge_plan(conn: &Connection) -> Result<LedgerPurgePlan> {
    let (tables, triggers) = ledger_objects(conn)?;
    let mut counted = Vec::new();
    for table in &tables {
        let rows: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {}", quote(table)), [], |r| {
                r.get(0)
            })
            .map_err(map_sqlite_error)?;
        counted.push((table.clone(), rows));
    }
    Ok(LedgerPurgePlan {
        row_count: counted.iter().map(|(_, rows)| rows).sum(),
        tables: counted.into_iter().filter(|(_, rows)| *rows != 0).collect(),
        table_count: tables.len(),
        trigger_count: triggers.len(),
    })
}

/// `legacy_ledger.purge`: the user-confirmed removal of every leftover ledger
/// table and trigger, rows included, in one reserved write. Returns the plan
/// it removed.
pub fn purge_legacy_ledger(conn: &Connection) -> Result<LedgerPurgePlan> {
    let tx = crate::write_transaction(conn).map_err(map_sqlite_error)?;
    let plan = legacy_ledger_purge_plan(&tx)?;
    let (tables, triggers) = ledger_objects(&tx)?;
    if !tables.is_empty() || !triggers.is_empty() {
        drop_ledger(&tx, tables, triggers, &mut |_| Ok(()))?;
    }
    tx.commit().map_err(map_sqlite_error)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ledger_retirement_is_atomic_and_preserves_nonempty_storage() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE gl_parent(id INTEGER PRIMARY KEY);CREATE TABLE gl_child(parent INTEGER REFERENCES gl_parent(id));CREATE TRIGGER gl_guard BEFORE DELETE ON gl_parent BEGIN SELECT RAISE(ABORT,'retained'); END;").unwrap();
        let error = retire_ledger_with_observer(&mut conn, &mut |label| {
            if label == "legacy-ledger.after_drop.gl_parent" {
                Err(Error::new(ErrorKind::Io, "injected failure"))
            } else {
                Ok(())
            }
        });
        assert!(error.is_err());
        assert_eq!(ledger_objects(&conn).unwrap().0.len(), 2);
        conn.execute("INSERT INTO gl_parent VALUES(1)", []).unwrap();
        assert!(!retire_empty_legacy_ledger(&mut conn).unwrap());
        assert_eq!(
            require_legacy_ledger_absent(&conn).unwrap_err().kind,
            ErrorKind::LegacyLedgerPresent
        );
        assert!(conn.execute("DELETE FROM gl_parent", []).is_err());
    }
    #[test]
    fn device_sync_audit_rolls_back_with_every_drop() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        book.connection.execute_batch("CREATE TABLE sync_books(id);CREATE TABLE sync_events(id);INSERT INTO sync_events VALUES(1),(2);").unwrap();
        assert!(
            drop_device_sync_with_observer(&mut book.connection, &mut |label| if label
                == "device-sync.after_drop.sync_events"
            {
                Err(Error::new(ErrorKind::Io, "injected failure"))
            } else {
                Ok(())
            })
            .is_err()
        );
        assert_eq!(
            book.connection
                .query_row("SELECT count(*) FROM sync_events", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(book.connection.query_row("SELECT count(*) FROM schema_migration_audits WHERE migration_name='device-sync-removal-v1'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(drop_device_sync(&mut book.connection).unwrap());
        let changes = book.connection.total_changes();
        assert!(!drop_device_sync(&mut book.connection).unwrap());
        assert_eq!(book.connection.total_changes(), changes);
    }
}
