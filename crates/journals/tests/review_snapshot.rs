#![cfg(feature = "test-harness")]
use kassiber_journals::pipeline::memory_snapshot;
use kassiber_store::rusqlite::{types::Value, Connection};

fn seed(encrypted: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.pragma_update(None, "key", "synthetic-source-key")
            .unwrap();
    }
    conn.execute_batch("CREATE TABLE canary(id TEXT PRIMARY KEY,amount INTEGER,real_value REAL,payload BLOB);INSERT INTO canary VALUES('钱 🦀',9007199254740993,1.25,x'00ff')").unwrap();
    conn
}
fn row(conn: &Connection) -> Vec<Value> {
    conn.query_row("SELECT * FROM canary", [], |row| {
        (0..4).map(|index| row.get(index)).collect()
    })
    .unwrap()
}

#[test]
fn review_memory_snapshots_preserve_sqlcipher_and_plaintext_values_without_source_writes() {
    for encrypted in [false, true] {
        for read_transaction in [false, true] {
            let source = seed(encrypted);
            if read_transaction {
                source.execute_batch("BEGIN; SELECT * FROM canary").unwrap();
            }
            let changes = source.total_changes();
            let mut keys = 0;
            let clone = memory_snapshot(&source, || {
                keys += 1;
                Ok("synthetic-ephemeral-key".into())
            })
            .unwrap();
            assert_eq!(keys, usize::from(encrypted));
            assert_eq!(row(&source), row(&clone));
            for (pragma, expected) in [("temp_store", 2), ("foreign_keys", 1)] {
                assert_eq!(
                    clone
                        .pragma_query_value(None, pragma, |row| row.get::<_, i64>(0))
                        .unwrap(),
                    expected
                );
            }
            assert_eq!(
                clone
                    .pragma_query_value(None, "cipher_status", |row| row.get::<_, String>(0))
                    .unwrap(),
                if encrypted { "1" } else { "0" }
            );
            clone.execute("DELETE FROM canary", []).unwrap();
            assert_eq!(source.total_changes(), changes);
            assert!(!row(&source).is_empty());
            assert_eq!(source.is_autocommit(), !read_transaction);
        }
    }
}

#[test]
fn review_memory_snapshot_refuses_pending_writes_without_committing_or_consuming_a_key() {
    for encrypted in [false, true] {
        let source = seed(encrypted);
        let original = row(&source);
        source
            .execute_batch("BEGIN IMMEDIATE; UPDATE canary SET amount=7")
            .unwrap();
        let changes = source.total_changes();
        let error = memory_snapshot(&source, || {
            panic!("write refusal must precede key generation")
        })
        .unwrap_err();
        assert_eq!(error.code.as_deref(), Some("review_transaction_active"));
        assert_eq!(
            error.message,
            "Finish the current write transaction before planning a review"
        );
        assert!(!source.is_autocommit());
        assert_eq!(source.total_changes(), changes);
        assert_eq!(row(&source)[1], Value::Integer(7));
        source.execute_batch("ROLLBACK").unwrap();
        assert_eq!(row(&source), original);
    }
}
