//! Handwritten compatibility steps backed by exact generated Python SQL.
//! This module is private until the complete ordered pass proves parity.
use crate::{map_sqlite_error, Result};
use rusqlite::Connection;
use serde_json::Value;
use std::{collections::BTreeSet, sync::OnceLock};

#[path = "compat_backfill.rs"]
pub(crate) mod backfills;
#[path = "compat_custody.rs"]
pub(crate) mod custody;
#[path = "compat_liquid.rs"]
pub(crate) mod liquid;
#[path = "compat_ordered.rs"]
pub(crate) mod ordered;
#[path = "compat_ownership.rs"]
pub(crate) mod ownership;
#[path = "compat_json.rs"]
pub(crate) mod python_json;
#[path = "compat_schema.rs"]
pub(crate) mod repairs;

pub(super) fn data() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../fixtures/compat.json"))
            .expect("checked compatibility SQL")
    })
}
fn sql(function: &str, index: usize) -> &'static str {
    data()["functions"][function]["sql"][index]
        .as_str()
        .expect("checked SQL statement")
}
pub(super) fn columns(conn: &Connection, table: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(map_sqlite_error)?;
    let result = stmt
        .query_map([], |r| Ok((r.get(1)?, r.get(2)?)))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error);
    result
}
pub(super) fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<bool> {
    if columns(conn, table)?.iter().any(|(name, _)| name == column) {
        return Ok(false);
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition}"
    ))
    .map_err(map_sqlite_error)?;
    Ok(true)
}
pub(crate) fn install_base(conn: &Connection) -> Result<()> {
    let existing = columns(conn, "journal_tax_summary")?;
    if !existing.is_empty() {
        ensure_column(conn, "journal_tax_summary", "capital_gains_type", "TEXT")?;
    }
    for table in ["transaction_pairs", "direct_swap_payouts"] {
        if !columns(conn, table)?.is_empty() {
            ensure_column(
                conn,
                table,
                "component_id",
                "TEXT REFERENCES custody_components(id) ON DELETE SET NULL",
            )?;
        }
    }
    let empty: bool = conn
        .query_row("SELECT NOT EXISTS(SELECT 1 FROM sqlite_master)", [], |r| {
            r.get(0)
        })
        .map_err(map_sqlite_error)?;
    let base = data()["base_schema"].as_str().expect("base schema");
    let script = if empty {
        format!("BEGIN IMMEDIATE;\n{base}\nCOMMIT;")
    } else {
        // Existing books add this index at the transactional ordered step.
        base.replace("CREATE INDEX IF NOT EXISTS idx_transactions_profile_page ON transactions(profile_id, occurred_at, created_at, id, wallet_id);", "")
    };
    conn.execute_batch(&script).map_err(map_sqlite_error)
}

pub(crate) fn migrate_msat(
    conn: &Connection,
    observer: &mut impl FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut steps = Vec::new();
    for (table, column, key) in [
        ("transactions", "amount", "migrate_transactions"),
        ("journal_entries", "quantity", "migrate_journal_entries"),
        (
            "journal_tax_summary",
            "quantity",
            "migrate_journal_tax_summary",
        ),
        (
            "journal_account_holdings",
            "quantity",
            "migrate_journal_account_holdings",
        ),
        (
            "journal_wallet_holdings",
            "quantity",
            "migrate_journal_wallet_holdings",
        ),
    ] {
        let columns = columns(conn, table)?;
        if columns.iter().any(|(name, kind)| {
            (name == column || (table == "transactions" && name == "fee"))
                && kind.eq_ignore_ascii_case("REAL")
        }) {
            steps.push(key);
        }
    }
    if steps.is_empty() {
        return Ok(());
    }
    if !conn.is_autocommit() {
        conn.execute_batch("COMMIT").map_err(map_sqlite_error)?;
    }
    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .map_err(map_sqlite_error)?;
    conn.execute_batch("PRAGMA foreign_keys=OFF")
        .map_err(map_sqlite_error)?;
    let result = (|| {
        for key in &steps {
            for (index, statement) in data()["msat"][key]
                .as_array()
                .expect("msat scripts")
                .iter()
                .enumerate()
            {
                observer(&format!("msat.before_script.{key}.{index}"))?;
                conn.execute_batch(statement.as_str().expect("SQL script"))
                    .map_err(map_sqlite_error)?;
                observer(&format!("msat.after_script.{key}.{index}"))?;
            }
        }
        conn.execute_batch(sql("_recreate_msat_migration_indexes", 0))
            .map_err(map_sqlite_error)?;
        if steps.contains(&"migrate_transactions") {
            conn.execute_batch(data()["custody_schema"].as_str().expect("custody schema"))
                .map_err(map_sqlite_error)?;
            for statement in data()["functions"]
                ["_create_custody_gap_review_transaction_aux_schema"]["sql"]
                .as_array()
                .expect("review auxiliary schema")
            {
                conn.execute_batch(statement.as_str().expect("SQL"))
                    .map_err(map_sqlite_error)?;
            }
        }
        Ok(())
    })();
    if result.is_err() && !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    conn.execute_batch(&format!("PRAGMA foreign_keys={foreign_keys}"))
        .map_err(map_sqlite_error)?;
    result
}

pub(crate) fn install_graph(conn: &mut Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','trigger')")
        .map_err(map_sqlite_error)?;
    let objects: BTreeSet<String> = stmt
        .query_map([], |r| r.get(0))
        .map_err(map_sqlite_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(map_sqlite_error)?;
    drop(stmt);
    let graph = &data()["graph"];
    let required: BTreeSet<String> = graph["ddl"]
        .as_array()
        .expect("graph DDL")
        .iter()
        .filter_map(|value| {
            let statement = value.as_str().expect("SQL");
            if !statement.starts_with("CREATE TABLE") {
                return None;
            }
            Some(
                statement
                    .split_whitespace()
                    .nth(5)
                    .expect("table name")
                    .split('(')
                    .next()
                    .expect("table name")
                    .to_owned(),
            )
        })
        .collect();
    // Unlike executescript, a savepoint never commits an outer import/review.
    let tx = conn.savepoint().map_err(map_sqlite_error)?;
    if !required.is_subset(&objects) {
        for statement in graph["ddl"].as_array().expect("graph DDL") {
            let statement = statement
                .as_str()
                .expect("SQL")
                .replace(
                    "profile_id TEXT NOT NULL",
                    "profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE",
                )
                .replace(
                    "profile_id TEXT PRIMARY KEY",
                    "profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE",
                );
            tx.execute_batch(&statement).map_err(map_sqlite_error)?;
        }
        tx.execute("UPDATE chain_index_state SET version=0", [])
            .map_err(map_sqlite_error)?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_chain_index_label_subject ON chain_index_labels(profile_id,json_extract(payload_json,'$.subject'))").map_err(map_sqlite_error)?;
    if objects.contains("chain_analysis_dataset_claims") {
        for field in ["valid_from", "valid_until"] {
            tx.execute_batch(&format!("CREATE INDEX IF NOT EXISTS idx_chain_index_claim_{field} ON chain_analysis_dataset_claims({field},dataset_id) WHERE {field} IS NOT NULL")).map_err(map_sqlite_error)?;
        }
    }
    for value in graph["sources"].as_array().expect("graph source allowlist") {
        let table = value.as_str().expect("source table");
        if !objects.contains(table) {
            continue;
        }
        let mut added = false;
        for (operation, prefix) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
            let name = format!("chain_index_track_{table}_{}", operation.to_lowercase());
            if objects.contains(&name) {
                continue;
            }
            let physical = graph["physical"]
                .as_array()
                .expect("physical sources")
                .iter()
                .any(|v| v == table);
            let keys = if physical {
                if operation == "UPDATE" {
                    vec!["OLD.rowid".to_owned(), "NEW.rowid".to_owned()]
                } else {
                    vec![format!("{prefix}.rowid")]
                }
            } else {
                vec!["-1".to_owned()]
            };
            let writes=keys.iter().map(|key|format!("INSERT INTO chain_index_dirty VALUES('{table}',{key},(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;")).collect::<String>();
            let when = if table == "chain_analysis_acquisition_grants" && operation == "UPDATE" {
                let existing = columns(&tx, table)?;
                let fields: Vec<_> = graph["tip_fields"]
                    .as_array()
                    .expect("tip fields")
                    .iter()
                    .filter_map(|v| v.as_str())
                    .filter(|field| existing.iter().any(|(name, _)| name == field))
                    .map(|field| format!("OLD.{field} IS NOT NEW.{field}"))
                    .collect();
                format!(" WHEN {}", fields.join(" OR "))
            } else {
                String::new()
            };
            tx.execute_batch(&format!("CREATE TRIGGER {name} AFTER {operation} ON {table}{when} BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; {writes} END")).map_err(map_sqlite_error)?;
            added = true;
        }
        if added {
            tx.execute(
                "UPDATE chain_index_clock SET revision=revision+1 WHERE id=1",
                [],
            )
            .map_err(map_sqlite_error)?;
            tx.execute("INSERT OR REPLACE INTO chain_index_dirty SELECT ?,-1,revision FROM chain_index_clock WHERE id=1",[table]).map_err(map_sqlite_error)?;
        }
    }
    tx.commit().map_err(map_sqlite_error)
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn finish_msat_columns(conn: &Connection) -> Result<()> {
    for (column, definition) in [
        ("pricing_review_accepted_price", "TEXT"),
        ("amount_includes_fee", "INTEGER NOT NULL DEFAULT 0"),
        ("kind_override", "TEXT"),
        (
            "external_id_kind",
            "TEXT CHECK(external_id_kind IS NULL OR external_id_kind = 'txid')",
        ),
    ] {
        ensure_column(conn, "transactions", column, definition)?;
    }
    // Python adds the compact calculation column right after the msat rebuild.
    ensure_column(conn, "journal_entries", "calculation_compact", "TEXT")?;
    for values in data()["functions"]["_ensure_swap_matching_schema"]["columns"]
        .as_array()
        .expect("swap columns")
    {
        if values[0] == "transactions" {
            ensure_column(
                conn,
                "transactions",
                values[1].as_str().expect("column name"),
                values[2].as_str().expect("column definition"),
            )?;
        }
    }
    // Recreate the index whose table was rebuilt above, at Python's later
    // swap-schema step rather than assuming the original index survived.
    let statements = data()["functions"]["_ensure_swap_matching_schema"]["sql"]
        .as_array()
        .expect("swap schema");
    for statement in statements
        .iter()
        .filter_map(Value::as_str)
        .filter(|s| s.contains("CREATE INDEX") && s.contains("payment_hash"))
    {
        conn.execute_batch(statement).map_err(map_sqlite_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn legacy_book(table: &str, key: &str) -> (tempfile::TempDir, crate::Book) {
        let root = tempfile::tempdir().unwrap();
        let book = crate::Book::create(root.path(), None).unwrap();
        let conn = &book.connection;
        conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        if table == "transactions" {
            conn.execute_batch(data()["msat"]["migrate_transactions"][0].as_str().unwrap())
                .unwrap();
        }
        let scripts = data()["msat"][key].as_array().unwrap();
        let script = scripts.last().unwrap().as_str().unwrap();
        let declaration = script
            .split_once("CREATE TABLE ")
            .unwrap()
            .1
            .split_once(';')
            .unwrap()
            .0;
        let declaration = format!(
            "CREATE TABLE {}",
            declaration
                .replace(&format!("{table}__msat_new"), table)
                .replace("quantity INTEGER", "quantity REAL")
                .replace("amount INTEGER", "amount REAL")
                .replace("fee INTEGER", "fee REAL")
        );
        conn.execute_batch(&format!(
            "DROP TABLE {table};{declaration};PRAGMA foreign_keys=ON;"
        ))
        .unwrap();
        conn.execute_batch("INSERT INTO workspaces VALUES('ws','Synthetic','fixed');INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed');INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','pf','Wallet','manual','fixed');").unwrap();
        let tx_id = if table == "transactions" {
            "legacy"
        } else {
            "anchor"
        };
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fee,created_at) VALUES(?1,'ws','pf','wal',?1,'fixed','inbound','BTC',?2,?3,'fixed')",rusqlite::params![tx_id,if table=="transactions"{1.25}else{1.0},if table=="transactions"{0.00001}else{0.0}]).unwrap();
        match table {
            "journal_entries" => {
                conn.execute_batch("INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,created_at) VALUES('legacy','ws','pf','anchor','wal','fixed','acquisition','BTC',1.25,'fixed')").unwrap();
            }
            "journal_tax_summary" => {
                conn.execute_batch("INSERT INTO journal_tax_summary(id,workspace_id,profile_id,year,asset,transaction_type,quantity,created_at) VALUES('legacy','ws','pf',2024,'BTC','sell',0.5,'fixed')").unwrap();
            }
            "journal_account_holdings" => {
                conn.execute_batch("INSERT INTO journal_account_holdings(id,workspace_id,profile_id,asset,quantity,created_at) VALUES('legacy','ws','pf','BTC',1.25,'fixed')").unwrap();
            }
            "journal_wallet_holdings" => {
                conn.execute_batch("INSERT INTO journal_wallet_holdings(id,workspace_id,profile_id,asset,quantity,created_at) VALUES('legacy','ws','pf','BTC',0.75,'fixed')").unwrap();
            }
            _ => (),
        }
        (root, book)
    }
    #[test]
    fn each_legacy_real_quantity_shape_converts_once_with_integer_results() {
        for (table, key, column, expected) in [
            (
                "transactions",
                "migrate_transactions",
                "amount",
                125_000_000_000i64,
            ),
            (
                "journal_entries",
                "migrate_journal_entries",
                "quantity",
                125_000_000_000,
            ),
            (
                "journal_tax_summary",
                "migrate_journal_tax_summary",
                "quantity",
                50_000_000_000,
            ),
            (
                "journal_account_holdings",
                "migrate_journal_account_holdings",
                "quantity",
                125_000_000_000,
            ),
            (
                "journal_wallet_holdings",
                "migrate_journal_wallet_holdings",
                "quantity",
                75_000_000_000,
            ),
        ] {
            let (_root, mut book) = legacy_book(table, key);
            let conn = &mut book.connection;
            install_base(conn).unwrap();
            migrate_msat(conn, &mut |_| Ok(())).unwrap();
            finish_msat_columns(conn).unwrap();
            install_graph(conn).unwrap();
            let actual: (i64, String) = conn
                .query_row(
                    &format!("SELECT {column},typeof({column}) FROM {table} WHERE id='legacy'"),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(actual, (expected, "integer".into()), "{table}");
            let changes = conn.total_changes();
            migrate_msat(conn, &mut |_| Ok(())).unwrap();
            assert_eq!(conn.total_changes(), changes);
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
    #[test]
    fn ordered_pass_restores_the_accepted_price_the_msat_rebuild_drops() {
        // The rebuild's fixed column list omits the accepted coarse price, as
        // Python's does; the pass adds it before and again after the rebuild.
        let (_root, mut book) = legacy_book("transactions", "migrate_transactions");
        let conn = &mut book.connection;
        ordered::migrate(conn, &mut |_| panic!("identity must survive"), &mut |_| {
            Ok(())
        })
        .unwrap();
        crate::schema::verify(conn).unwrap();
        assert!(columns(conn, "transactions")
            .unwrap()
            .iter()
            .any(|(name, _)| name == "pricing_review_accepted_price"));
    }
    #[test]
    fn interrupted_msat_steps_resume_without_double_scaling() {
        for target in [
            "msat.before_script.migrate_transactions.0",
            "msat.after_script.migrate_transactions.0",
            "msat.before_script.migrate_transactions.1",
            "msat.after_script.migrate_transactions.1",
        ] {
            let (_root, mut book) = legacy_book("transactions", "migrate_transactions");
            let conn = &mut book.connection;
            install_base(conn).unwrap();
            assert!(migrate_msat(conn, &mut |label| if label == target {
                Err(crate::Error::new(
                    crate::ErrorKind::Io,
                    "injected migration failure",
                ))
            } else {
                Ok(())
            })
            .is_err());
            assert!(conn.is_autocommit());
            assert_eq!(
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                1
            );
            install_base(conn).unwrap();
            migrate_msat(conn, &mut |_| Ok(())).unwrap();
            finish_msat_columns(conn).unwrap();
            install_graph(conn).unwrap();
            assert_eq!(
                conn.query_row(
                    "SELECT amount FROM transactions WHERE id='legacy'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                125_000_000_000
            );
        }
    }
    #[test]
    fn graph_repair_preserves_outer_transactions_and_tracks_physical_rowids() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        book.connection.execute_batch("DROP TABLE chain_index_edges; BEGIN; INSERT INTO workspaces VALUES('ws','Synthetic','fixed'); INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('pf','ws','Synthetic','fixed'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','ws','pf','Wallet','manual','fixed');").unwrap();
        install_graph(&mut book.connection).unwrap();
        assert!(!book.connection.is_autocommit());
        book.connection.execute_batch("INSERT INTO transactions(rowid,id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at) VALUES(123,'tx','ws','pf','wal','tx','fixed','inbound','BTC',1,'fixed'); DELETE FROM chain_index_dirty; UPDATE transactions SET rowid=124 WHERE id='tx';").unwrap();
        let keys: Vec<i64> = book.connection.prepare("SELECT source_key FROM chain_index_dirty WHERE source_table='transactions' ORDER BY source_key").unwrap().query_map([], |row| row.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(keys, [123, 124]);
        book.connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            book.connection
                .query_row("SELECT COUNT(*) FROM workspaces WHERE id='ws'", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }
    #[test]
    fn graph_install_is_idempotent_and_does_not_seed_an_existing_clock_table() {
        let root = tempfile::tempdir().unwrap();
        let mut book = crate::Book::create(root.path(), None).unwrap();
        let changes = book.connection.total_changes();
        install_graph(&mut book.connection).unwrap();
        assert_eq!(book.connection.total_changes(), changes);
        book.connection
            .execute("DELETE FROM chain_index_clock", [])
            .unwrap();
        install_graph(&mut book.connection).unwrap();
        assert_eq!(
            book.connection
                .query_row("SELECT count(*) FROM chain_index_clock", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
