//! Repairs for schema that only unreleased development builds wrote.
//!
//! Python's open was additive: a book a development build touched kept
//! whatever that build created, and the next release ignored it. The
//! structural gate refuses any object the current schema does not know, so
//! such a book would never open again. Each repair here names one known
//! leftover and matches it exactly, so an object a newer release creates is
//! never mistaken for one and still reaches the gate's refusal. A repair never
//! discards rows a build wrote: a leftover that holds data is left in place
//! and refused.
use crate::{map_sqlite_error, schema, Result};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeSet;

/// Run every repair, each in its own immediate transaction. The ordered pass
/// calls this after its last step, so the tables have their current columns.
pub(crate) fn repair(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(map_sqlite_error)?;
    }
    drop_metadata_sources(conn, observer)?;
    restore_chain_data_default(conn, observer)
}

/// `transaction_metadata_sources`: per-source note and label overlays from
/// an unmerged BTCPay follow-up (April 2026). No release created or read the
/// table. Two shapes were built: the first, and one with
/// `managed_labels_json` added by `ALTER TABLE`. Dropped only while empty.
const METADATA_SOURCES: &str = "transaction_metadata_sources";
const METADATA_SOURCE_COLUMNS: [&str; 5] = [
    "transaction_id",
    "source",
    "note",
    "labels_json",
    "updated_at",
];
const METADATA_SOURCE_LATER_COLUMN: &str = "managed_labels_json";

fn drop_metadata_sources(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    if in_manifest("table", METADATA_SOURCES) {
        return Ok(());
    }
    let Some(columns) = table_columns(conn, METADATA_SOURCES)? else {
        return Ok(());
    };
    let first: BTreeSet<String> = METADATA_SOURCE_COLUMNS.map(str::to_owned).into();
    let mut later = first.clone();
    later.insert(METADATA_SOURCE_LATER_COLUMN.to_owned());
    if columns != first && columns != later {
        return Ok(());
    }
    // Nothing else may depend on it: no trigger or view names it, and its only
    // index is the primary key's automatic one.
    let dependents: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name<>?1 AND name NOT GLOB 'sqlite_autoindex_*' AND (tbl_name=?1 OR instr(lower(COALESCE(sql,'')), ?1) > 0)",
            [METADATA_SOURCES],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    if dependents != 0 {
        return Ok(());
    }
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_sqlite_error)?;
    let populated: bool = tx
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {METADATA_SOURCES})"),
            [],
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    if populated {
        return Ok(());
    }
    tx.execute_batch(&format!("DROP TABLE {METADATA_SOURCES}"))
        .map_err(map_sqlite_error)?;
    observer("unreleased.metadata_sources.before_commit")?;
    tx.commit().map_err(map_sqlite_error)?;
    observer("unreleased.metadata_sources.after_commit")
}

/// `source_funds_links.chain_data_confirmed DEFAULT 1`: written by the
/// unmerged source-of-funds branch (9be8e2fd0, 2026-05-09) and changed to 0
/// there by 54d8fd795 without a migration; the squash merge to main
/// (fac1b64e8) and every release have 0. Only the default differs, so the
/// table is rebuilt under the current definition with every row copied as
/// stored; no row is reinterpreted.
const LINKS: &str = "source_funds_links";
const CHAIN_DEFAULT_CURRENT: &str = "chain_data_confirmed INTEGER NOT NULL DEFAULT 0,";
const CHAIN_DEFAULT_UNRELEASED: &str = "chain_data_confirmed INTEGER NOT NULL DEFAULT 1,";

fn restore_chain_data_default(
    conn: &mut Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let Some(current) = manifest_sql("table", LINKS) else {
        return Ok(());
    };
    if !current.contains(CHAIN_DEFAULT_CURRENT) {
        return Ok(());
    }
    let unreleased = current.replacen(CHAIN_DEFAULT_CURRENT, CHAIN_DEFAULT_UNRELEASED, 1);
    let Some(actual) = stored_sql(conn, "table", LINKS)? else {
        return Ok(());
    };
    if !crate::schema_structure::same_tokens(&actual, &unreleased) {
        return Ok(());
    }
    rebuild(conn, LINKS, current, observer)
}

/// Replace `table` with its current definition, copying every row as stored,
/// then reinstall the current schema's indexes and triggers on it. Foreign
/// keys stay off so no cascade fires, and `legacy_alter_table` keeps the
/// rename from rewriting other tables' references to the table.
fn rebuild(
    conn: &mut Connection,
    table: &str,
    definition: &str,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let pragma = |conn: &Connection, name: &str| -> Result<i64> {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .map_err(map_sqlite_error)
    };
    let foreign_keys = pragma(conn, "foreign_keys")?;
    let legacy_alter = pragma(conn, "legacy_alter_table")?;
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA legacy_alter_table=ON")
        .map_err(map_sqlite_error)?;
    let result = (|| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(map_sqlite_error)?;
        let legacy = format!("{table}__unreleased");
        let before: i64 = tx
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .map_err(map_sqlite_error)?;
        tx.execute_batch(&format!("ALTER TABLE {table} RENAME TO {legacy}"))
            .map_err(map_sqlite_error)?;
        tx.execute_batch(definition).map_err(map_sqlite_error)?;
        let columns = table_columns(&tx, table)?.expect("table just created");
        let list = columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(",");
        tx.execute_batch(&format!(
            "INSERT INTO {table}({list}) SELECT {list} FROM {legacy}; DROP TABLE {legacy};"
        ))
        .map_err(map_sqlite_error)?;
        let after: i64 = tx
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .map_err(map_sqlite_error)?;
        assert_eq!(before, after, "rebuild copies every row");
        for kind in ["index", "trigger"] {
            for sql in manifest_objects()
                .iter()
                .filter(|o| o[0] == kind && o[2] == table)
                .filter_map(|o| o[3].as_str())
            {
                tx.execute_batch(sql).map_err(map_sqlite_error)?;
            }
        }
        observer("unreleased.rebuild.before_commit")?;
        tx.commit().map_err(map_sqlite_error)?;
        observer("unreleased.rebuild.after_commit")
    })();
    let restored = conn
        .execute_batch(&format!(
            "PRAGMA legacy_alter_table={legacy_alter}; PRAGMA foreign_keys={foreign_keys}"
        ))
        .map_err(map_sqlite_error);
    result.and(restored)
}

fn manifest_objects() -> &'static Vec<Value> {
    static OBJECTS: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    OBJECTS.get_or_init(|| {
        let manifest: Value =
            serde_json::from_str(schema::MANIFEST).expect("checked-in schema manifest");
        manifest["objects"]
            .as_array()
            .expect("schema objects")
            .clone()
    })
}
fn manifest_sql(kind: &str, name: &str) -> Option<&'static str> {
    manifest_objects()
        .iter()
        .find(|o| o[0] == kind && o[1] == name)
        .and_then(|o| o[3].as_str())
}
fn in_manifest(kind: &str, name: &str) -> bool {
    manifest_objects()
        .iter()
        .any(|o| o[0] == kind && o[1] == name)
}
fn stored_sql(conn: &Connection, kind: &str, name: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
        [kind, name],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_sqlite_error)
}
fn table_columns(conn: &Connection, table: &str) -> Result<Option<BTreeSet<String>>> {
    if stored_sql(conn, "table", table)?.is_none() {
        return Ok(None);
    }
    let mut statement = conn
        .prepare("SELECT name FROM pragma_table_xinfo(?1)")
        .map_err(map_sqlite_error)?;
    let names = statement
        .query_map([table], |r| r.get(0))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<BTreeSet<String>>>()
        .map_err(map_sqlite_error)?;
    Ok(Some(names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encryption, Book, ErrorKind, OpenOptions};
    use std::path::Path;

    const PASSPHRASE: &str = "Synthetic passphrase 1";
    const FIRST_SHAPE: &str = "CREATE TABLE transaction_metadata_sources (
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    note TEXT,
    labels_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (transaction_id, source)
)";

    fn create(data: &Path, passphrase: Option<&str>) {
        Book::create(data, passphrase)
            .unwrap()
            .close()
            .map_err(|f| f.error)
            .unwrap();
    }
    /// Change the book behind Kassiber's back, as a development build did.
    fn tamper(data: &Path, passphrase: Option<&str>, sql: &str) {
        let conn =
            encryption::open_container(&kassiber_paths::resolve_database_path(data), passphrase)
                .unwrap();
        conn.execute_batch(sql).unwrap();
    }
    fn open(data: &Path, passphrase: Option<&str>) -> crate::Result<Book> {
        Book::open_migrating(
            data,
            OpenOptions {
                passphrase,
                ..Default::default()
            },
            true,
        )
    }
    fn query<T: rusqlite::types::FromSql>(book: &Book, sql: &str) -> T {
        book.connection().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn refused_object(error: &crate::Error) -> String {
        assert_eq!(error.kind, ErrorKind::RequiresPythonMigration, "{error:?}");
        error.details["object"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    /// The unreleased default, written the way 9be8e2fd0 created the table.
    fn unreleased_links() -> String {
        let current = manifest_sql("table", LINKS).unwrap();
        let mut sql = String::from("PRAGMA foreign_keys=OFF; PRAGMA legacy_alter_table=ON;");
        sql.push_str("ALTER TABLE source_funds_links RENAME TO links_old;");
        sql.push_str(&current.replacen(CHAIN_DEFAULT_CURRENT, CHAIN_DEFAULT_UNRELEASED, 1));
        sql.push_str(
            ";INSERT INTO source_funds_links SELECT * FROM links_old; DROP TABLE links_old;",
        );
        for o in manifest_objects()
            .iter()
            .filter(|o| o[0] == "index" && o[2] == LINKS)
        {
            sql.push_str(o[3].as_str().unwrap());
            sql.push(';');
        }
        sql
    }

    #[test]
    fn the_current_schema_still_has_the_fixed_default() {
        assert!(manifest_sql("table", LINKS)
            .unwrap()
            .contains(CHAIN_DEFAULT_CURRENT));
        assert!(!in_manifest("table", METADATA_SOURCES));
    }

    #[test]
    fn both_metadata_source_shapes_are_dropped_while_empty() {
        for passphrase in [None, Some(PASSPHRASE)] {
            for later in [false, true] {
                let root = tempfile::tempdir().unwrap();
                let data = root.path().join("data");
                create(&data, passphrase);
                let mut sql = format!("{FIRST_SHAPE};");
                if later {
                    sql.push_str("ALTER TABLE transaction_metadata_sources ADD COLUMN managed_labels_json TEXT NOT NULL DEFAULT '[]';");
                }
                tamper(&data, passphrase, &sql);
                let error = Book::open(
                    &data,
                    OpenOptions {
                        passphrase,
                        ..Default::default()
                    },
                )
                .err()
                .expect("the admission-only open does not repair");
                assert_eq!(refused_object(&error), METADATA_SOURCES);
                let book = open(&data, passphrase).unwrap();
                let left: i64 = query(
                    &book,
                    "SELECT COUNT(*) FROM sqlite_master WHERE tbl_name='transaction_metadata_sources'",
                );
                assert_eq!(left, 0);
                book.close().map_err(|f| f.error).unwrap();
                open(&data, passphrase).unwrap();
            }
        }
    }

    #[test]
    fn a_populated_or_different_metadata_sources_table_is_still_refused() {
        let populated = format!(
            "{FIRST_SHAPE}; PRAGMA foreign_keys=OFF; INSERT INTO transaction_metadata_sources(transaction_id,source,updated_at) VALUES('tx','btcpay','2026-04-23T00:00:00Z');"
        );
        let different = "CREATE TABLE transaction_metadata_sources (transaction_id TEXT NOT NULL, source TEXT NOT NULL, weight INTEGER, PRIMARY KEY (transaction_id, source))".to_owned();
        for sql in [populated, different] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("data");
            create(&data, None);
            tamper(&data, None, &sql);
            let error = open(&data, None).err().expect("refused");
            assert_eq!(refused_object(&error), METADATA_SOURCES);
            let conn =
                encryption::open_container(&kassiber_paths::resolve_database_path(&data), None)
                    .unwrap();
            let kept: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name='transaction_metadata_sources'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(kept, 1);
        }
    }

    #[test]
    fn the_unreleased_chain_default_is_rebuilt_keeping_rows_and_attachments() {
        for passphrase in [None, Some(PASSPHRASE)] {
            let root = tempfile::tempdir().unwrap();
            let data = root.path().join("data");
            create(&data, passphrase);
            let mut sql = unreleased_links();
            // Rows as stored, including one that took the unreleased default,
            // and an attachment whose cascade must not fire during the rebuild.
            sql.push_str("PRAGMA foreign_keys=OFF;
                INSERT INTO source_funds_links(id,workspace_id,profile_id,from_transaction_id,to_transaction_id,link_type,asset,created_at,updated_at)
                    VALUES('link-default','ws','pr','tx-a','tx-b','transfer','BTC','2026-05-09T13:00:00Z','2026-05-09T13:00:00Z');
                INSERT INTO source_funds_links(id,workspace_id,profile_id,from_transaction_id,to_transaction_id,link_type,asset,chain_data_confirmed,created_at,updated_at)
                    VALUES('link-explicit','ws','pr','tx-c','tx-d','transfer','BTC',0,'2026-05-09T13:00:00Z','2026-05-09T13:00:00Z');
                INSERT INTO source_funds_link_attachments(link_id,attachment_id,created_at)
                    VALUES('link-default','att','2026-05-09T13:00:00Z');");
            tamper(&data, passphrase, &sql);
            let error = Book::open(
                &data,
                OpenOptions {
                    passphrase,
                    ..Default::default()
                },
            )
            .err()
            .expect("the admission-only open does not repair");
            assert_eq!(refused_object(&error), LINKS);

            let book = open(&data, passphrase).unwrap();
            let stored: String = query(
                &book,
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='source_funds_links'",
            );
            assert_eq!(stored, manifest_sql("table", LINKS).unwrap());
            let rows: String = query(
                &book,
                "SELECT group_concat(id || '=' || chain_data_confirmed, ',') FROM (SELECT id, chain_data_confirmed FROM source_funds_links ORDER BY id)",
            );
            assert_eq!(
                rows, "link-default=1,link-explicit=0",
                "rows keep their stored values"
            );
            let attachments: i64 =
                query(&book, "SELECT COUNT(*) FROM source_funds_link_attachments");
            assert_eq!(attachments, 1);
            let indexes: i64 = query(
                &book,
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND tbl_name='source_funds_links' AND sql IS NOT NULL",
            );
            assert_eq!(indexes, 2);
            let leftover: i64 = query(
                &book,
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%__unreleased'",
            );
            assert_eq!(leftover, 0);
            let foreign_keys: i64 = query(&book, "PRAGMA foreign_keys");
            assert_eq!(foreign_keys, 1);
            book.close().map_err(|f| f.error).unwrap();
            open(&data, passphrase).unwrap();
        }
    }

    #[test]
    fn a_near_miss_links_definition_is_not_rebuilt() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        create(&data, None);
        let sql = unreleased_links().replacen(
            "uses_chain_observation INTEGER NOT NULL DEFAULT 0,",
            "uses_chain_observation INTEGER NOT NULL DEFAULT 1,",
            1,
        );
        tamper(&data, None, &sql);
        let error = open(&data, None).err().expect("refused");
        assert_eq!(refused_object(&error), LINKS);
    }
}
