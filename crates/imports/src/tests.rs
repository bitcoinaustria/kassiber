use super::*;
use kassiber_envelope::get;

// Rust replacements for tests/test_import_batches.py::RollbackScaleTests and
// core.import_batches.demo. CLI subprocess cases remain hybrid boundary tests.
fn fixture(rows: usize) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;
        CREATE TABLE profiles(id TEXT PRIMARY KEY,last_processed_at TEXT,last_processed_tx_count INTEGER,journal_input_version INTEGER,ownership_review_counts_json TEXT);
        INSERT INTO profiles VALUES('p','fresh',12,7,'{}'),('q','fresh',1,20,'{}');
        CREATE TABLE accounts(id TEXT PRIMARY KEY,code TEXT);
        CREATE TABLE wallets(id TEXT PRIMARY KEY,profile_id TEXT,account_id TEXT,label TEXT);
        INSERT INTO wallets VALUES('w','p',NULL,'Source'),('v','p',NULL,'Other');
        CREATE TABLE transactions(id TEXT PRIMARY KEY,profile_id TEXT,wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE);
        CREATE TABLE import_batches(id TEXT PRIMARY KEY,profile_id TEXT,wallet_id TEXT REFERENCES wallets(id) ON DELETE CASCADE,source_format TEXT,source_filename TEXT,column_map_json TEXT,imported_at TEXT,rows_inserted INTEGER,rows_updated INTEGER,rows_skipped INTEGER);
        CREATE TABLE import_batch_transactions(batch_id TEXT REFERENCES import_batches(id) ON DELETE CASCADE,transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,PRIMARY KEY(batch_id,transaction_id));
        CREATE TABLE transaction_tags(transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,tag TEXT);
        CREATE TABLE loose_notes(transaction_id TEXT REFERENCES transactions(id) ON DELETE SET NULL,note TEXT);
        CREATE INDEX batch_transaction ON import_batch_transactions(transaction_id);
        CREATE INDEX tag_transaction ON transaction_tags(transaction_id);
        CREATE TABLE transaction_pairs(id TEXT PRIMARY KEY,in_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE,out_transaction_id TEXT REFERENCES transactions(id) ON DELETE CASCADE);
        CREATE INDEX pair_in ON transaction_pairs(in_transaction_id);
        CREATE INDEX pair_out ON transaction_pairs(out_transaction_id);
        CREATE TABLE pair_attachments(pair_id TEXT REFERENCES transaction_pairs(id) ON DELETE CASCADE,filename TEXT);
        CREATE INDEX attachment_pair ON pair_attachments(pair_id);
        INSERT INTO transactions VALUES('old','p','w'),('other','p','v');
        INSERT INTO import_batches VALUES('b','p','w','csv','source.csv','{\"date\": \"Datum\"}','2024-01-01',1,1,1),('c','p','v','json','other.json',NULL,'2024-02-01',1,0,0);
        INSERT INTO import_batch_transactions VALUES('c','other');").unwrap();
    conn.execute_batch("BEGIN").unwrap();
    for index in 0..rows {
        let id = format!("tx-{index}");
        conn.execute("INSERT INTO transactions VALUES(?,'p','w')", [&id])
            .unwrap();
        conn.execute("INSERT INTO import_batch_transactions VALUES('b',?)", [&id])
            .unwrap();
        conn.execute("INSERT INTO transaction_tags VALUES(?,'reviewed')", [&id])
            .unwrap();
    }
    for index in (0..rows.saturating_sub(1)).step_by(2) {
        let pair = format!("pair-{index}");
        conn.execute(
            "INSERT INTO transaction_pairs VALUES(?,?,?)",
            rusqlite::params![pair, format!("tx-{index}"), format!("tx-{}", index + 1)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO pair_attachments VALUES(?,'receipt.pdf')",
            [&pair],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
    conn
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {}", quote(table)), [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn rollback_large_run_uses_staged_ids_and_preserves_enriched_rows() {
    let conn = fixture(33_000);
    let plan = plan(&conn, "p", "b").unwrap();
    assert_eq!(
        get(&plan, "transactions_to_delete"),
        Some(&Value::from(33_000))
    );
    assert_eq!(
        get(get(&plan, "also_removed").unwrap(), "transaction_tags"),
        Some(&Value::from(33_000))
    );
    assert_eq!(count(&conn, "transactions"), 33_002);
    let result = rollback(&conn, "p", "b").unwrap();
    assert_eq!(
        get(&result, "transactions_deleted"),
        Some(&Value::from(33_000))
    );
    assert_eq!(count(&conn, "transactions"), 2);
    assert_eq!(count(&conn, "wallets"), 2);
    assert_eq!(count(&conn, "import_batches"), 1);
    assert_eq!(count(&conn, "import_batch_transactions"), 1);
    let readiness: (Option<String>,i64,i64,Option<String>) = conn.query_row("SELECT last_processed_at,last_processed_tx_count,journal_input_version,ownership_review_counts_json FROM profiles WHERE id='p'",[],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(readiness, (None, 0, 8, None));
    assert_eq!(
        conn.query_row(
            "SELECT journal_input_version FROM profiles WHERE id='q'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        20
    );
    assert_eq!(count(&conn, "pragma_foreign_key_check"), 0);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_temp_master WHERE name='_import_rollback_ids'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn collateral_counts_once_and_follows_transitive_cascades() {
    let conn = fixture(4);
    conn.execute_batch("INSERT INTO transaction_pairs VALUES('mixed','tx-0','old');INSERT INTO pair_attachments VALUES('mixed','mixed.pdf');
        INSERT INTO loose_notes VALUES('tx-0','retain');").unwrap();
    let plan = plan(&conn, "p", "b").unwrap();
    assert_eq!(
        get(&plan, "also_removed"),
        Some(&dict([
            ("transaction_tags", Value::from(4)),
            ("transaction_pairs", Value::from(3)),
            ("pair_attachments", Value::from(3)),
        ]))
    );
    let result = rollback(&conn, "p", "b").unwrap();
    assert_eq!(get(&result, "also_removed"), get(&plan, "also_removed"));
    assert_eq!(count(&conn, "loose_notes"), 1);
    assert!(conn
        .query_row("SELECT transaction_id IS NULL FROM loose_notes", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap());
}

#[test]
fn rollback_is_profile_bound_and_failures_are_atomic() {
    let conn = fixture(2);
    assert_eq!(plan(&conn, "q", "b").unwrap_err().code, "not_found");
    assert_eq!(rollback(&conn, "q", "b").unwrap_err().code, "not_found");
    assert_eq!(
        plan(&conn, "p", "missing").unwrap_err().message,
        "Import batch not found: missing"
    );
    conn.execute_batch("CREATE TRIGGER refuse_invalidation BEFORE UPDATE ON profiles BEGIN SELECT RAISE(ABORT,'failure');END;").unwrap();
    assert!(rollback(&conn, "p", "b").is_err());
    assert!(conn.is_autocommit());
    assert_eq!(count(&conn, "transactions"), 4);
    assert_eq!(count(&conn, "transaction_tags"), 2);
    assert_eq!(count(&conn, "transaction_pairs"), 1);
    assert_eq!(count(&conn, "pair_attachments"), 1);
    assert_eq!(count(&conn, "import_batches"), 2);
    assert_eq!(count(&conn, "import_batch_transactions"), 3);
    conn.execute_batch("DROP TRIGGER refuse_invalidation")
        .unwrap();
    rollback(&conn, "p", "b").unwrap();
    assert_eq!(count(&conn, "transactions"), 2);
}

#[test]
fn empty_run_does_not_invalidate_and_lists_actual_remaining_rows() {
    let conn = fixture(0);
    let plan = plan(&conn, "p", "b").unwrap();
    assert_eq!(
        get(&plan, "journals_invalidated"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        get(get(&plan, "batch").unwrap(), "rows_present"),
        Some(&Value::from(0))
    );
    assert_eq!(
        get(get(&plan, "batch").unwrap(), "rolled_back"),
        Some(&Value::Bool(true))
    );
    let result = rollback(&conn, "p", "b").unwrap();
    assert_eq!(
        get(&result, "journals_invalidated"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        conn.query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
}

#[test]
fn list_is_ordered_scoped_and_filters_wallet_without_exposing_config() {
    let conn = fixture(2);
    conn.execute_batch("UPDATE import_batches SET column_map_json='broken' WHERE id='c';
        INSERT INTO import_batches VALUES('d','q',NULL,'csv',NULL,NULL,'2025-01-01',0,0,0);
        INSERT INTO import_batches VALUES('z','p',NULL,'csv',NULL,'123456789012345678901234567890','2024-02-01',0,0,0);").unwrap();
    let Value::List(all) = list(&conn, "p", None, 0).unwrap() else {
        panic!()
    };
    assert_eq!(all.len(), 3);
    assert_eq!(get(&all[0], "id"), Some(&s("z")));
    assert_eq!(
        get(&all[0], "column_map"),
        Some(&kassiber_pytext::loads("123456789012345678901234567890").unwrap())
    );
    assert_eq!(get(&all[1], "id"), Some(&s("c")));
    assert_eq!(get(&all[1], "column_map"), Some(&Value::Null));
    let Value::List(one) = execute(
        &conn,
        Operation::List {
            profile_id: "p".into(),
            wallet: Some(" source ".into()),
            limit: Value::from(-1),
        },
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(one.len(), 1);
    assert_eq!(
        get(&one[0], "column_map"),
        Some(&dict([("date", s("Datum"))]))
    );
    assert_eq!(get(&one[0], "rows_inserted"), Some(&Value::from(1)));
    assert_eq!(get(&one[0], "rows_present"), Some(&Value::from(2)));
    assert_eq!(
        execute(
            &conn,
            Operation::List {
                profile_id: "q".into(),
                wallet: Some("w".into()),
                limit: Value::from(50)
            }
        )
        .unwrap_err()
        .code,
        "not_found"
    );
}

#[test]
fn python_list_limits_preserve_zero_types_and_integer_bounds() {
    for (raw, expected) in [
        ("null", 50),
        ("0", 50),
        ("false", 50),
        ("true", 1),
        ("\"0\"", 1),
        ("\" ２_０ \"", 20),
        ("2.9", 2),
        ("-100", 1),
        ("10000000000000000000000000000000000000000000", 200),
        ("\"\"", 50),
        ("[]", 50),
    ] {
        assert_eq!(
            list_limit(Some(&kassiber_pytext::loads(raw).unwrap())).unwrap(),
            expected,
            "{raw}"
        );
    }
    for raw in ["\"x\"", "[1]", "{\"a\":1}", "NaN", "Infinity"] {
        assert!(
            list_limit(Some(&kassiber_pytext::loads(raw).unwrap())).is_err(),
            "{raw}"
        );
    }
}

#[test]
fn column_map_decode_errors_are_null_but_recursion_errors_fail_closed() {
    let conn = fixture(0);
    for raw in ["broken".to_owned(), "1".repeat(4301)] {
        conn.execute(
            "UPDATE import_batches SET column_map_json=? WHERE id='b'",
            [raw],
        )
        .unwrap();
        let payload = plan(&conn, "p", "b").unwrap();
        assert_eq!(
            get(get(&payload, "batch").unwrap(), "column_map"),
            Some(&Value::Null)
        );
    }
    let raw = format!("{}0{}", "[".repeat(2000), "]".repeat(2000));
    conn.execute(
        "UPDATE import_batches SET column_map_json=? WHERE id='b'",
        [raw],
    )
    .unwrap();
    assert_eq!(plan(&conn, "p", "b").unwrap_err().code, "internal_error");
    assert_eq!(
        list(&conn, "p", None, 50).unwrap_err().code,
        "internal_error"
    );
    assert_eq!(count(&conn, "import_batches"), 2);
}
