use kassiber_envelope::{get, s};
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::params, Book};
use kassiber_transactions::{
    metadata::{execute, Command},
    Scope,
};

fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn book() -> (tempfile::TempDir, Book) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025'); INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at,last_processed_at,last_processed_tx_count) VALUES('p','w','Book','EUR','at','2025','2025',3); INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','w','p','Cold','address','2025');").unwrap();
    for (id, external, direction) in [
        ("tx", "external", "inbound"),
        ("other", "tx", "outbound"),
        ("dup1", "duplicate", "inbound"),
        ("dup2", "duplicate", "inbound"),
    ] {
        book.connection().execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,kind,created_at) VALUES(?,'w','p','wal',?,?,'BTC',?,100000000001,1,'2025-01-01T00:00:00Z','deposit','2025')",params![id,id,external,direction]).unwrap();
    }
    (tmp, book)
}
fn run(book: &Book, command: Command) -> Value {
    execute(book.connection(), &scope(), &command).unwrap()
}
fn note(book: &Book, note: Option<&str>) -> Value {
    run(
        book,
        Command::Note {
            transaction: "tx".into(),
            note: note.map(str::to_owned),
            reason: Some("  reason  ".into()),
        },
    )
}
fn count(book: &Book, table: &str) -> i64 {
    book.connection()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
fn tag(book: &Book, remove: bool) -> Value {
    run(
        book,
        Command::Tag {
            transaction: "tx".into(),
            tag: "special".into(),
            remove,
            reason: None,
        },
    )
}

#[test]
fn resolver_prefers_id_and_refuses_duplicate_external_ids_and_cross_profile() {
    let (_tmp, book) = book();
    assert_eq!(
        get(
            &run(
                &book,
                Command::Get {
                    transaction: "tx".into()
                }
            ),
            "direction"
        ),
        Some(&s("inbound"))
    );
    assert_eq!(
        get(
            &run(
                &book,
                Command::Get {
                    transaction: "external".into()
                }
            ),
            "transaction_id"
        ),
        Some(&s("tx"))
    );
    assert_eq!(
        execute(
            book.connection(),
            &scope(),
            &Command::Get {
                transaction: "duplicate".into()
            }
        )
        .unwrap_err()
        .code,
        "ambiguous_reference"
    );
    let other_scope = Scope {
        workspace_id: "w",
        profile_id: "absent",
    };
    assert_eq!(
        execute(
            book.connection(),
            &other_scope,
            &Command::Get {
                transaction: "tx".into()
            }
        )
        .unwrap_err()
        .code,
        "app_error"
    );
    let record = run(
        &book,
        Command::Get {
            transaction: "tx".into(),
        },
    );
    assert_eq!(
        get(&record, "amount_msat"),
        Some(&Value::Int(100000000001_i64.into()))
    );
    assert_eq!(get(&record, "amount"), Some(&Value::Float(1.00000000001)));
}

#[test]
fn notes_append_exact_json_and_noops_do_not_invalidate() {
    let (_tmp, book) = book();
    let first = note(&book, Some("Invoice \"42\"\n€"));
    assert_eq!(get(&first, "updated"), Some(&Value::Bool(true)));
    assert_eq!(
        get(&note(&book, Some("Invoice \"42\"\n€")), "history_event_id"),
        Some(&Value::Null)
    );
    note(&book, None);
    assert_eq!(count(&book, "transaction_edit_events"), 2);
    let history: Vec<(String, String, String)> = book
        .connection()
        .prepare(
            "SELECT before_value,after_value,diff_json FROM transaction_edit_fields ORDER BY rowid",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        history,
        vec![
            (
                "null".into(),
                "\"Invoice \\\"42\\\"\\n\\u20ac\"".into(),
                "{}".into()
            ),
            (
                "\"Invoice \\\"42\\\"\\n\\u20ac\"".into(),
                "null".into(),
                "{}".into()
            )
        ]
    );
    let events: Vec<(String,i64,i64,Option<String>,i64)> = book.connection().prepare("SELECT reason,journal_input_version,journal_input_version_after,last_processed_at,last_processed_tx_count FROM transaction_edit_events ORDER BY rowid").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(
        events,
        vec![
            ("reason".into(), 0, 1, Some("2025".into()), 3),
            ("reason".into(), 1, 2, None, 0)
        ]
    );
    let version: i64 = book
        .connection()
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, 2);
}

#[test]
fn tags_use_label_normalization_preserve_existing_identity_and_append_diffs() {
    let (_tmp, book) = book();
    run(
        &book,
        Command::CreateTag {
            code: "special".into(),
            label: "  ΟΣ Team  ".into(),
        },
    );
    let first = tag(&book, false);
    assert_eq!(get(&first, "tag"), Some(&s("special")));
    assert_eq!(count(&book, "tags"), 2); // Authored code differs from normalized label.
    assert_eq!(get(&tag(&book, false), "status"), Some(&s("unchanged")));
    assert_eq!(count(&book, "tags"), 2);
    let code: String = book
        .connection()
        .query_row("SELECT code FROM tags WHERE code!='special'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(code, "ος-team");
    let diff: String = book
        .connection()
        .query_row("SELECT diff_json FROM transaction_edit_fields", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        diff,
        r#"{"added":["\u039f\u03a3 Team"],"after":["\u039f\u03a3 Team"],"before":[],"removed":[]}"#
    );
    // Removal compares untrimmed selected label, exactly as Python does.
    assert_eq!(get(&tag(&book, true), "updated"), Some(&Value::Bool(false)));
    run(
        &book,
        Command::Tag {
            transaction: "tx".into(),
            tag: code,
            remove: true,
            reason: None,
        },
    );
    assert_eq!(count(&book, "transaction_tags"), 0);
    assert_eq!(count(&book, "transaction_edit_events"), 2);
}

#[test]
fn failed_invalidation_rolls_back_rows_tags_and_history() {
    let (_tmp, book) = book();
    run(
        &book,
        Command::CreateTag {
            code: "special".into(),
            label: "New label".into(),
        },
    );
    book.connection().execute_batch("CREATE TRIGGER deny_invalidation BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'fail invalidation'); END;").unwrap();
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::Tag {
            transaction: "tx".into(),
            tag: "special".into(),
            remove: false,
            reason: None
        }
    )
    .is_err());
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::Note {
            transaction: "tx".into(),
            note: Some("No commit".into()),
            reason: None
        }
    )
    .is_err());
    assert_eq!(count(&book, "tags"), 1);
    assert_eq!(count(&book, "transaction_tags"), 0);
    assert_eq!(count(&book, "transaction_edit_events"), 0);
    assert_eq!(count(&book, "transaction_edit_fields"), 0);
    assert_eq!(
        get(
            &run(
                &book,
                Command::Get {
                    transaction: "tx".into()
                }
            ),
            "note"
        ),
        Some(&s(""))
    );
}

#[test]
fn tag_label_reuses_an_existing_profile_code() {
    let (_tmp, book) = book();
    run(
        &book,
        Command::CreateTag {
            code: "special".into(),
            label: "Receipt".into(),
        },
    );
    let existing = run(
        &book,
        Command::CreateTag {
            code: "receipt".into(),
            label: "Retained label".into(),
        },
    );
    tag(&book, false);
    let selected: String = book
        .connection()
        .query_row("SELECT tag_id FROM transaction_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(Some(&s(selected)), get(&existing, "id"));
    assert_eq!(count(&book, "tags"), 2);
    let history: String = book
        .connection()
        .query_row("SELECT after_value FROM transaction_edit_fields", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(history, r#"["Receipt"]"#);
}

#[test]
fn duplicate_case_labels_already_attached_are_a_canonical_noop() {
    let (_tmp, book) = book();
    run(
        &book,
        Command::CreateTag {
            code: "special".into(),
            label: "Receipt".into(),
        },
    );
    run(
        &book,
        Command::CreateTag {
            code: "second".into(),
            label: "receipt".into(),
        },
    );
    book.connection()
        .execute(
            "INSERT INTO transaction_tags(transaction_id,tag_id) SELECT 'tx',id FROM tags",
            [],
        )
        .unwrap();
    assert_eq!(
        get(&tag(&book, false), "updated"),
        Some(&Value::Bool(false))
    );
    assert_eq!(count(&book, "transaction_edit_events"), 0);
    assert_eq!(count(&book, "transaction_tags"), 2);
}

#[test]
fn kind_requires_direction_and_preserves_observed_kind() {
    let (_tmp, book) = book();
    let cmd = |tx: &str, kind: Option<&str>| Command::Kind {
        transaction: tx.into(),
        kind: kind.map(str::to_owned),
        reason: None,
    };
    assert_eq!(
        execute(book.connection(), &scope(), &cmd("other", Some("income")))
            .unwrap_err()
            .code,
        "validation"
    );
    let changed = run(&book, cmd("tx", Some("income")));
    assert_eq!(get(&changed, "kind"), Some(&s("deposit")));
    assert_eq!(get(&changed, "kind_override"), Some(&s("income")));
    assert_eq!(
        get(&run(&book, cmd("tx", Some("income"))), "updated"),
        Some(&Value::Bool(false))
    );
    let cleared = run(&book, cmd("tx", None));
    assert_eq!(get(&cleared, "kind"), Some(&s("deposit")));
    assert_eq!(get(&cleared, "kind_override"), Some(&Value::Null));
    assert_eq!(count(&book, "transaction_edit_events"), 2);
}

#[test]
fn exclusion_refuses_an_active_anchored_custody_leg() {
    let (_tmp, book) = book();
    book.connection().execute_batch("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('component','lineage','w','p',1,'transfer','active','2025'); INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,transaction_id,anchor_transaction_id,role,ordinal,rail,asset,exposure,conservation_unit,amount_msat,created_at) VALUES('leg','component','w','p','other','tx','source',0,'onchain','BTC','BTC','BTC',1000,'2025');").unwrap();
    let command = Command::Excluded {
        transaction: "tx".into(),
        excluded: true,
        reason: None,
    };
    let error = execute(book.connection(), &scope(), &command).unwrap_err();
    assert_eq!(error.code, "conflict");
    assert_eq!(count(&book, "transaction_edit_events"), 0);
    book.connection()
        .execute("UPDATE custody_components SET state='superseded'", [])
        .unwrap();
    assert_eq!(
        get(&run(&book, command), "excluded"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn stale_counts_use_version_after_and_preserve_processed_events() {
    let (_tmp, book) = book();
    note(&book, Some("First"));
    book.connection().execute("UPDATE profiles SET last_processed_input_version=1,last_processed_at='processed' WHERE id='p'",[]).unwrap();
    assert_eq!(
        get(&run(&book, Command::Stale), "edit_count"),
        Some(&Value::Int(0.into()))
    );
    note(&book, Some("Second"));
    let summary = run(&book, Command::Stale);
    assert_eq!(get(&summary, "edit_count"), Some(&Value::Int(1.into())));
    assert_eq!(
        get(get(&summary, "family_counts").unwrap(), "metadata"),
        Some(&Value::Int(1.into()))
    );
    assert_eq!(count(&book, "transaction_edit_events"), 2);
}
