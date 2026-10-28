use kassiber_envelope::{get, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::params, Book};
use kassiber_transactions::{
    loans::{execute, Command},
    Scope,
};

fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w1",
        profile_id: "p1",
    }
}

fn book() -> (tempfile::TempDir, Book) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w1','Main','2025-01-01T00:00:00Z'); INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p1','w1','Book','EUR','at','2025-01-01T00:00:00Z'); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal1','w1','p1','onchain','descriptor','2025-01-01T00:00:00Z');").unwrap();
    for (id, direction, when) in [
        ("tx1", "outbound", "2025-06-01T00:00:00Z"),
        ("tx2", "inbound", "2025-09-01T00:00:00Z"),
        ("tx3", "outbound", "2025-10-01T00:00:00Z"),
    ] {
        book.connection().execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES(?,'w1','p1','wal1',?,'BTC',?,100000000001,0,?,'deposit','2025-01-01T00:00:00Z')", params![id, format!("fp-{id}"),direction,when]).unwrap();
    }
    (tmp, book)
}

fn mark(book: &Book, tx: &str, role: &str, loan_id: Option<&str>) -> Value {
    execute(
        book.connection(),
        &scope(),
        &Command::Mark {
            txid: tx.into(),
            mark_as: role.into(),
            note: Value::Null,
            loan_id: loan_id.map(str::to_owned),
        },
    )
    .unwrap()
}

fn list(book: &Book, key: &str) -> Vec<Value> {
    let result = execute(
        book.connection(),
        &scope(),
        &Command::List {
            include_roles: false,
        },
    )
    .unwrap();
    match get(&result, key).unwrap() {
        Value::List(rows) => rows.clone(),
        _ => panic!(),
    }
}

#[test]
fn all_roles_require_the_exact_direction_and_refuse_channel_roles() {
    let (_tmp, book) = book();
    for (role, valid_tx, invalid_tx) in [
        ("collateral", "tx1", "tx2"),
        ("returned", "tx2", "tx1"),
        ("principal-received", "tx2", "tx1"),
        ("principal-repaid", "tx1", "tx2"),
    ] {
        mark(&book, valid_tx, role, None);
        let before = list(&book, "marks");
        let error = execute(
            book.connection(),
            &scope(),
            &Command::Mark {
                txid: invalid_tx.into(),
                mark_as: role.into(),
                note: Value::Null,
                loan_id: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(list(&book, "marks"), before);
    }
    for role in ["liquidation", "channel_open", "channel_close"] {
        assert_eq!(
            execute(
                book.connection(),
                &scope(),
                &Command::Mark {
                    txid: "tx1".into(),
                    mark_as: role.into(),
                    note: Value::Null,
                    loan_id: None
                }
            )
            .unwrap_err()
            .code,
            "validation"
        );
    }
}

#[test]
fn remark_retires_history_preserves_loan_and_observation_and_invalidates() {
    let (_tmp, book) = book();
    let first = mark(&book, "tx1", "collateral", Some("loan-a"));
    let second = mark(&book, "tx1", "principal-repaid", None);
    assert_ne!(get(&first, "id"), get(&second, "id"));
    assert_eq!(get(&second, "loan_id"), Some(&s("loan-a")));
    assert_eq!(list(&book, "marks").len(), 1);
    let counts: (i64,i64,i64,String,i64) = book.connection().query_row("SELECT (SELECT COUNT(*) FROM loan_legs),(SELECT COUNT(*) FROM loan_legs WHERE deleted_at IS NOT NULL),journal_input_version,(SELECT kind FROM transactions WHERE id='tx1'),(SELECT amount FROM transactions WHERE id='tx1') FROM profiles WHERE id='p1'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(counts, (2, 1, 2, "deposit".into(), 100000000001));
}

#[test]
fn unknown_and_cross_profile_transactions_fail_without_marks() {
    let (_tmp, book) = book();
    for tx in ["absent", ""] {
        assert_eq!(
            execute(
                book.connection(),
                &scope(),
                &Command::Mark {
                    txid: tx.into(),
                    mark_as: "collateral".into(),
                    note: Value::Null,
                    loan_id: None
                }
            )
            .unwrap_err()
            .code,
            "not_found"
        );
    }
    let other = Scope {
        workspace_id: "w1",
        profile_id: "other",
    };
    assert_eq!(
        execute(
            book.connection(),
            &other,
            &Command::Mark {
                txid: "tx1".into(),
                mark_as: "collateral".into(),
                note: Value::Null,
                loan_id: None
            }
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert!(list(&book, "marks").is_empty());
}

#[test]
fn linking_preserves_request_order_and_requires_two_distinct_active_marks() {
    let (_tmp, book) = book();
    mark(&book, "tx1", "principal-repaid", Some("loan-a"));
    mark(&book, "tx2", "principal-received", Some("loan-b"));
    let linked = execute(
        book.connection(),
        &scope(),
        &Command::Link {
            txids: vec!["tx2".into(), "tx1".into(), "tx2".into()],
            loan_id: None,
        },
    )
    .unwrap();
    assert_eq!(get(&linked, "loan_id"), Some(&s("loan-b")));
    assert_eq!(
        get(&linked, "transaction_ids"),
        Some(&Value::List(vec![s("tx2"), s("tx1")]))
    );
    for txids in [vec!["tx1".into(), "tx1".into()], vec![]] {
        assert_eq!(
            execute(
                book.connection(),
                &scope(),
                &Command::Link {
                    txids,
                    loan_id: None
                }
            )
            .unwrap_err()
            .code,
            "validation"
        );
    }
    let before = list(&book, "marks");
    assert_eq!(
        execute(
            book.connection(),
            &scope(),
            &Command::Link {
                txids: vec!["tx1".into(), "tx3".into()],
                loan_id: Some("new".into())
            }
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(list(&book, "marks"), before);
    execute(
        book.connection(),
        &scope(),
        &Command::Link {
            txids: vec!["tx1".into(), "tx2".into()],
            loan_id: Some(" explicit ".into()),
        },
    )
    .unwrap();
    assert!(list(&book, "marks")
        .iter()
        .all(|m| get(m, "loan_id") == Some(&s("explicit"))));
}

#[test]
fn unmark_retires_and_missing_mark_is_not_found() {
    let (_tmp, book) = book();
    mark(&book, "tx1", "collateral", None);
    let result = execute(
        book.connection(),
        &scope(),
        &Command::Unmark { txid: "tx1".into() },
    )
    .unwrap();
    assert_eq!(get(&result, "unmarked"), Some(&s("tx1")));
    assert!(list(&book, "marks").is_empty());
    assert_eq!(
        execute(
            book.connection(),
            &scope(),
            &Command::Unmark { txid: "tx1".into() }
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        book.connection()
            .query_row(
                "SELECT COUNT(*) FROM loan_legs WHERE deleted_at IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn open_locks_match_oldest_first_per_asset_and_ignore_principal() {
    let (_tmp, book) = book();
    mark(&book, "tx1", "collateral", None);
    mark(&book, "tx3", "collateral", None);
    assert_eq!(list(&book, "open_locks").len(), 2);
    mark(&book, "tx2", "principal-received", None);
    assert_eq!(list(&book, "open_locks").len(), 2);
    mark(&book, "tx2", "returned", None);
    assert_eq!(
        get(&list(&book, "open_locks")[0], "transaction_id"),
        Some(&s("tx3"))
    );
    book.connection()
        .execute("UPDATE transactions SET asset='L-BTC' WHERE id='tx2'", [])
        .unwrap();
    assert_eq!(list(&book, "open_locks").len(), 2);
}

#[test]
fn failed_invalidation_rolls_back_mark_and_retirement() {
    let (_tmp, book) = book();
    mark(&book, "tx1", "collateral", None);
    let before = list(&book, "marks");
    book.connection().execute_batch("CREATE TRIGGER refuse_invalidate BEFORE UPDATE ON profiles BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::Mark {
            txid: "tx1".into(),
            mark_as: "principal-repaid".into(),
            note: Value::Null,
            loan_id: None
        }
    )
    .is_err());
    assert_eq!(list(&book, "marks"), before);
}
