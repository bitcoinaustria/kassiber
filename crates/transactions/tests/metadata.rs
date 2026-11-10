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

fn grouped() -> kassiber_transactions::metadata::GroupedUpdate {
    kassiber_transactions::metadata::GroupedUpdate {
        transaction: "tx".into(),
        source: " GUI ".into(),
        reason: Some("  grouped review  ".into()),
        ..Default::default()
    }
}

#[test]
fn grouped_raw_quantities_return_python_integers_without_rewriting_observations() {
    use kassiber_store::rusqlite::types::Value as Sql;
    let (_tmp, book) = book();
    for (amount, fee, amount_msat, fee_msat) in [
        (Sql::Real(1.75), Sql::Real(-2.75), 1, -2),
        (Sql::Blob(b"77".to_vec()), Sql::Blob(b"0".to_vec()), 77, 0),
        (
            Sql::Blob(b" 7_7 ".to_vec()),
            Sql::Blob(b"-3".to_vec()),
            77,
            -3,
        ),
    ] {
        book.connection()
            .execute(
                "UPDATE transactions SET amount=?,fee=? WHERE id='tx'",
                params![amount, fee],
            )
            .unwrap();
        let result = run(&book, Command::Update(Box::new(grouped())));
        assert_eq!(get(&result, "amount_msat"), Some(&Value::from(amount_msat)));
        assert_eq!(get(&result, "fee_msat"), Some(&Value::from(fee_msat)));
        kassiber_pyjson::dumps(&result, &kassiber_pyjson::Options::new()).unwrap();
        let stored: (Sql, Sql) = book
            .connection()
            .query_row(
                "SELECT amount,fee FROM transactions WHERE id='tx'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, (amount, fee));
    }
}

#[test]
fn grouped_save_merges_only_present_fields_and_appends_one_ordered_event() {
    use kassiber_transactions::metadata::PricingUpdate;
    let (_tmp, book) = book();
    book.connection()
        .execute(
            "UPDATE transactions SET description='observation',note='old' WHERE id='tx'",
            [],
        )
        .unwrap();
    run(
        &book,
        Command::CreateTag {
            code: "team-name".into(),
            label: "Existing identity".into(),
        },
    );
    let first = run(
        &book,
        Command::Update(Box::new(kassiber_transactions::metadata::GroupedUpdate {
            note: Some(s("")),
            tags: Some(Value::List(vec![
                s(" Team Name "),
                s("team name"),
                s("ΟΣ"),
                s("ος"),
                s(" "),
            ])),
            excluded: Some(Value::Bool(true)),
            review_status: Some(s(" review ")),
            taxable: Some(Value::Bool(false)),
            at_regime: Some(s("outside")),
            at_category: Some(s("none")),
            kind: Some(s("income")),
            pricing: Some(PricingUpdate {
                fiat_currency: s(" eur "),
                fiat_rate: s("65000.00100"),
                source_kind: s("manual_override"),
                external_ref: s(" invoice=42 "),
                ..Default::default()
            }),
            ..grouped()
        })),
    );
    assert_eq!(get(&first, "note"), Some(&s("")));
    assert_eq!(get(&first, "kind"), Some(&s("deposit")));
    assert_eq!(get(&first, "kind_override"), Some(&s("income")));
    assert_eq!(get(&first, "fiat_rate_exact"), Some(&s("65000.00100")));
    assert_eq!(
        get(&first, "fiat_value_exact"),
        Some(&s("65000.0010006500000100"))
    );
    assert_eq!(get(&first, "pricing_quality"), Some(&s("exact")));
    assert_eq!(count(&book, "transaction_edit_events"), 1);
    assert_eq!(count(&book, "transaction_tags"), 2);
    assert_eq!(count(&book, "tags"), 2);
    let conn = book.connection();
    let event: (String, String, i64, i64, i64, String, i64) = conn.query_row("SELECT source,reason,journal_input_version,journal_input_version_after,last_processed_input_version,last_processed_at,last_processed_tx_count FROM transaction_edit_events", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    assert_eq!(
        event,
        (
            "gui".into(),
            "grouped review".into(),
            0,
            1,
            0,
            "2025".into(),
            3
        )
    );
    let fields: Vec<String> = conn
        .prepare("SELECT field FROM transaction_edit_fields ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        &fields[..8],
        [
            "note",
            "tags",
            "excluded",
            "review_status",
            "taxable",
            "at_regime",
            "at_category",
            "kind"
        ]
    );
    let tags: (String, String) = conn
        .query_row(
            "SELECT before_value,after_value FROM transaction_edit_fields WHERE field='tags'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(tags.0, "[]");
    assert_eq!(tags.1, "[\"Team Name\",\"\\u039f\\u03a3\"]");
    let second = run(
        &book,
        Command::Update(Box::new(kassiber_transactions::metadata::GroupedUpdate {
            note: Some(Value::Null),
            taxable: Some(Value::Null),
            kind: Some(Value::Null),
            ..grouped()
        })),
    );
    assert_eq!(get(&second, "note"), Some(&s("")));
    assert_eq!(get(&second, "taxable"), Some(&Value::Null));
    assert_eq!(get(&second, "kind_override"), Some(&Value::Null));
    assert_eq!(get(&second, "review_status"), Some(&s("review")));
    assert_eq!(count(&book, "transaction_tags"), 2);
    assert!(conn
        .query_row(
            "SELECT note IS NULL FROM transactions WHERE id='tx'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(
        get(&second, "fiat_rate_exact"),
        get(&first, "fiat_rate_exact")
    );
    assert_eq!(count(&book, "transaction_edit_events"), 2);
    let observed: (i64, i64, String) = conn
        .query_row(
            "SELECT amount,fee,description FROM transactions WHERE id='tx'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(observed, (100000000001, 1, "observation".into()));
}

#[test]
fn grouped_prices_derive_exact_amounts_and_clear_all_provenance_when_missing() {
    use kassiber_transactions::metadata::PricingUpdate;
    let (_tmp, book) = book();
    let result = run(
        &book,
        Command::Update(Box::new(kassiber_transactions::metadata::GroupedUpdate {
            pricing: Some(PricingUpdate {
                fiat_value: s("1.234567890123456789012345678901"),
                source_kind: s("generic_import"),
                ..Default::default()
            }),
            ..grouped()
        })),
    );
    assert_eq!(
        get(&result, "fiat_rate_exact"),
        Some(&s("1.2345678901111111101112345677999"))
    );
    assert_eq!(
        get(&result, "fiat_value_exact"),
        Some(&s("1.234567890123456789012345678901"))
    );
    assert_eq!(get(&result, "pricing_quality"), Some(&s("provider_sample")));
    let result = run(
        &book,
        Command::Update(Box::new(kassiber_transactions::metadata::GroupedUpdate {
            pricing: Some(PricingUpdate {
                fiat_currency: s("usd"),
                fiat_rate: s("ignored junk"),
                external_ref: Value::Bool(true),
                ..Default::default()
            }),
            ..grouped()
        })),
    );
    assert_eq!(get(&result, "fiat_currency"), Some(&s("USD")));
    assert_eq!(get(&result, "pricing_quality"), Some(&s("missing")));
    for column in [
        "fiat_rate",
        "fiat_value",
        "fiat_rate_exact",
        "fiat_value_exact",
        "fiat_price_source",
        "pricing_source_kind",
        "pricing_provider",
        "pricing_pair",
        "pricing_timestamp",
        "pricing_fetched_at",
        "pricing_granularity",
        "pricing_method",
        "pricing_external_ref",
    ] {
        let null: bool = book
            .connection()
            .query_row(
                &format!("SELECT {column} IS NULL FROM transactions WHERE id='tx'"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(null, "{column}");
    }
    let result = run(
        &book,
        Command::Update(Box::new(kassiber_transactions::metadata::GroupedUpdate {
            pricing: Some(PricingUpdate {
                source_kind: s("manual_override"),
                quality: s("missing"),
                ..Default::default()
            }),
            ..grouped()
        })),
    );
    assert_eq!(get(&result, "updated"), Some(&Value::Bool(false)));
}

#[test]
fn grouped_canonical_tags_and_capture_time_only_changes_are_noops() {
    use kassiber_transactions::metadata::{GroupedUpdate, PricingUpdate};
    let (_tmp, book) = book();
    let request = || GroupedUpdate {
        note: Some(s("Receipt")),
        tags: Some(Value::List(vec![s("Zulu"), s("Alpha")])),
        pricing: Some(PricingUpdate {
            fiat_rate: s("42"),
            source_kind: s("wallet_export"),
            ..Default::default()
        }),
        ..grouped()
    };
    run(&book, Command::Update(Box::new(request())));
    book.connection()
        .execute(
            "UPDATE transactions SET pricing_fetched_at='2000' WHERE id='tx'",
            [],
        )
        .unwrap();
    let mut repeated = request();
    repeated.tags = Some(Value::List(vec![s("Alpha"), s("Zulu"), s(" Zulu "), s("")]));
    let result = run(&book, Command::Update(Box::new(repeated)));
    assert_eq!(get(&result, "updated"), Some(&Value::Bool(false)));
    assert_eq!(get(&result, "history_event_id"), Some(&Value::Null));
    assert_eq!(count(&book, "transaction_edit_events"), 1);
    let (time, version): (String,i64) = book.connection().query_row("SELECT t.pricing_fetched_at,p.journal_input_version FROM transactions t JOIN profiles p ON p.id=t.profile_id WHERE t.id='tx'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((time, version), ("2000".into(), 1));
}

#[test]
fn grouped_validation_and_failed_invalidation_preserve_all_rows() {
    use kassiber_transactions::metadata::{GroupedUpdate, PricingUpdate};
    let (_tmp, book) = book();
    let request = || GroupedUpdate {
        note: Some(s("New")),
        tags: Some(Value::List(vec![s("New tag")])),
        pricing: Some(PricingUpdate {
            fiat_rate: s("42"),
            source_kind: s("manual_override"),
            ..Default::default()
        }),
        ..grouped()
    };
    let mut invalids = Vec::new();
    for raw in [Value::Bool(true), Value::Int(1.into()), Value::List(vec![])] {
        invalids.push(GroupedUpdate {
            note: Some(raw.clone()),
            ..request()
        });
        invalids.push(GroupedUpdate {
            review_status: Some(raw.clone()),
            ..request()
        });
        invalids.push(GroupedUpdate {
            excluded: Some(raw.clone()),
            ..request()
        });
        invalids.push(GroupedUpdate {
            taxable: Some(raw),
            ..request()
        });
    }
    // Boolean true is valid for these fields, so use an explicit invalid value.
    invalids
        .retain(|r| r.excluded != Some(Value::Bool(true)) && r.taxable != Some(Value::Bool(true)));
    invalids.extend([
        GroupedUpdate {
            kind: Some(s("sell")),
            ..request()
        },
        GroupedUpdate {
            at_regime: Some(s("invented")),
            ..request()
        },
        GroupedUpdate {
            at_category: Some(s("invented")),
            ..request()
        },
        GroupedUpdate {
            review_status: Some(s("invented")),
            ..request()
        },
        GroupedUpdate {
            tags: Some(Value::List(vec![Value::Bool(true)])),
            ..request()
        },
        GroupedUpdate {
            note: Some(s("x".repeat(20001))),
            ..request()
        },
        GroupedUpdate {
            reason: Some("x".repeat(1001).into()),
            ..request()
        },
        GroupedUpdate {
            source: "unknown".into(),
            ..request()
        },
    ]);
    for invalid in invalids {
        assert_eq!(
            execute(
                book.connection(),
                &scope(),
                &Command::Update(Box::new(invalid))
            )
            .unwrap_err()
            .code,
            "validation"
        );
    }
    assert_eq!(count(&book, "transaction_edit_events"), 0);
    assert_eq!(count(&book, "tags"), 0);
    book.connection().execute_batch("CREATE TRIGGER refuse_grouped_invalidation BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'refuse invalidation'); END;").unwrap();
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::Update(Box::new(request()))
    )
    .is_err());
    assert_eq!(count(&book, "tags"), 0);
    assert_eq!(count(&book, "transaction_edit_events"), 0);
    assert_eq!(count(&book, "transaction_edit_fields"), 0);
    let unchanged: bool = book
        .connection()
        .query_row(
            "SELECT note IS NULL AND fiat_rate IS NULL FROM transactions WHERE id='tx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(unchanged);
}

#[test]
fn grouped_nonfinite_prices_fail_before_writes_and_exclusion_preserves_custody() {
    use kassiber_transactions::metadata::{GroupedUpdate, PricingUpdate};
    let (_tmp, book) = book();
    for field in ["fiat_rate", "fiat_value"] {
        for number in ["Infinity", "-Infinity", "NaN", "sNaN"] {
            let mut pricing = PricingUpdate {
                source_kind: s("manual_override"),
                ..Default::default()
            };
            if field == "fiat_rate" {
                pricing.fiat_rate = s(number);
            } else {
                pricing.fiat_value = s(number);
            }
            let error = execute(
                book.connection(),
                &scope(),
                &Command::Update(Box::new(GroupedUpdate {
                    note: Some(s("must not change")),
                    pricing: Some(pricing),
                    ..grouped()
                })),
            )
            .unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                format!("{field} must be a finite decimal number")
            );
        }
    }
    book.connection().execute_batch("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('component','lineage','w','p',1,'transfer','active','2025'); INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,transaction_id,anchor_transaction_id,role,ordinal,rail,asset,exposure,conservation_unit,amount_msat,created_at) VALUES('leg','component','w','p','other','tx','source',0,'onchain','BTC','BTC','BTC',1000,'2025');").unwrap();
    assert_eq!(
        execute(
            book.connection(),
            &scope(),
            &Command::Update(Box::new(GroupedUpdate {
                excluded: Some(Value::Bool(true)),
                note: Some(s("No")),
                ..grouped()
            }))
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    assert_eq!(count(&book, "transaction_edit_events"), 0);
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

fn list_values<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match get(value, key).unwrap() {
        Value::List(items) => items,
        _ => panic!("not a list"),
    }
}
fn cursor_text(value: &Value) -> String {
    match get(value, "next_cursor").unwrap() {
        Value::Str(s) => s.as_str().unwrap().into(),
        _ => panic!("no cursor"),
    }
}
fn revert(book: &Book, event: &str, field: Option<&str>) -> Value {
    run(
        book,
        Command::Revert {
            transaction: "tx".into(),
            event: event.into(),
            field: field.map(str::to_owned),
            source: "cli".into(),
            reason: None,
        },
    )
}

#[test]
fn record_pages_bind_filters_and_keep_all_three_descending_tiebreakers() {
    use kassiber_transactions::metadata::RecordOptions;
    let (_tmp, book) = book();
    book.connection()
        .execute(
            "UPDATE transactions SET created_at='2026' WHERE id='other'",
            [],
        )
        .unwrap();
    let options = || RecordOptions {
        limit: Some(1),
        ..Default::default()
    };
    let first = run(&book, Command::Records(options()));
    assert_eq!(
        get(&list_values(&first, "records")[0], "transaction_id"),
        Some(&s("other"))
    );
    let cursor = cursor_text(&first);
    let second = run(
        &book,
        Command::Records(RecordOptions {
            cursor: Some(cursor.clone()),
            ..options()
        }),
    );
    assert_eq!(
        get(&list_values(&second, "records")[0], "transaction_id"),
        Some(&s("tx"))
    );
    let third = run(
        &book,
        Command::Records(RecordOptions {
            cursor: Some(cursor_text(&second)),
            ..options()
        }),
    );
    assert_eq!(
        get(&list_values(&third, "records")[0], "transaction_id"),
        Some(&s("dup2"))
    );
    assert_eq!(
        execute(
            book.connection(),
            &scope(),
            &Command::Records(RecordOptions {
                cursor: Some(cursor),
                excluded: Some(false),
                ..options()
            })
        )
        .unwrap_err()
        .code,
        "validation"
    );
    note(&book, Some(""));
    let without = run(
        &book,
        Command::Records(RecordOptions {
            has_note: Some(false),
            ..Default::default()
        }),
    );
    assert_eq!(list_values(&without, "records").len(), 4);
    note(&book, Some("Receipt"));
    let with = run(
        &book,
        Command::Records(RecordOptions {
            has_note: Some(true),
            ..Default::default()
        }),
    );
    assert_eq!(list_values(&with, "records").len(), 1);
    assert_eq!(
        get(&list_values(&with, "records")[0], "amount_msat"),
        Some(&Value::Int(100000000001i64.into()))
    );
    for limit in [0, 1001] {
        assert!(execute(
            book.connection(),
            &scope(),
            &Command::Records(RecordOptions {
                limit: Some(limit),
                ..Default::default()
            })
        )
        .is_err());
    }
    run(
        &book,
        Command::CreateTag {
            code: "receipt".into(),
            label: "Receipt".into(),
        },
    );
    run(
        &book,
        Command::Tag {
            transaction: "tx".into(),
            tag: "receipt".into(),
            remove: false,
            reason: None,
        },
    );
    let tagged = run(
        &book,
        Command::Records(RecordOptions {
            tag: Some("Receipt".into()),
            ..Default::default()
        }),
    );
    assert_eq!(list_values(&tagged, "records").len(), 1);
    assert_eq!(
        get(&list_values(&tagged, "records")[0], "transaction_id"),
        Some(&s("tx"))
    );
}

#[test]
fn wallet_filters_prefer_ids_and_refuse_ambiguous_labels() {
    use kassiber_transactions::metadata::RecordOptions;
    let (_tmp, book) = book();
    book.connection().execute_batch("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('second','w','p','cold','address','2025'),('third','w','p','wal','address','2025');").unwrap();
    let list = |wallet: &str| {
        Command::Records(RecordOptions {
            wallet: Some(wallet.into()),
            ..Default::default()
        })
    };
    assert_eq!(list_values(&run(&book, list(" wal ")), "records").len(), 4);
    assert_eq!(
        execute(book.connection(), &scope(), &list("Cold"))
            .unwrap_err()
            .code,
        "validation"
    );
    assert_eq!(
        execute(book.connection(), &scope(), &list("missing"))
            .unwrap_err()
            .code,
        "not_found"
    );
}

#[test]
fn history_pages_filter_events_preserve_all_fields_and_redact_values() {
    use kassiber_transactions::metadata::HistoryOptions;
    let (_tmp, book) = book();
    for event in ["a", "b"] {
        book.connection().execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,source,changed_at,journal_input_version,journal_input_version_after) VALUES(?,'w','p','tx','wal','cli','2026-01-01T00:00:00Z',0,?)", params![event, if event == "a" { 1 } else { 2 }]).unwrap();
        book.connection().execute("INSERT INTO transaction_edit_fields(id,event_id,field,before_value,after_value,diff_json) VALUES(?,?,'note','null','\"Receipt\"','{}')",params![event,event]).unwrap();
    }
    let page = run(
        &book,
        Command::History(HistoryOptions {
            limit: Some(1),
            ..Default::default()
        }),
    );
    let first = &list_values(&page, "events")[0];
    let next = run(
        &book,
        Command::History(HistoryOptions {
            limit: Some(1),
            cursor: Some(cursor_text(&page)),
            ..Default::default()
        }),
    );
    let second = &list_values(&next, "events")[0];
    assert_ne!(get(first, "id"), get(second, "id"));
    assert!(format!("{:?}", get(first, "id")) > format!("{:?}", get(second, "id")));
    assert_eq!(get(&next, "has_more"), Some(&Value::Bool(false)));
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::History(HistoryOptions {
            cursor: Some(cursor_text(&page)),
            stale_only: true,
            ..Default::default()
        })
    )
    .is_err());
    let none = run(
        &book,
        Command::History(HistoryOptions {
            pricing_only: true,
            ..Default::default()
        }),
    );
    assert!(list_values(&none, "events").is_empty());
    book.connection()
        .execute(
            "UPDATE profiles SET last_processed_input_version=1 WHERE id='p'",
            [],
        )
        .unwrap();
    let stale = run(
        &book,
        Command::History(HistoryOptions {
            stale_only: true,
            ..Default::default()
        }),
    );
    assert_eq!(list_values(&stale, "events").len(), 1);
    assert_eq!(get(&list_values(&stale, "events")[0], "id"), Some(&s("b")));
    let note_id = match get(first, "id").unwrap() {
        Value::Str(t) => t.as_str().unwrap(),
        _ => unreachable!(),
    };
    book.connection().execute("INSERT INTO transaction_edit_fields(id,event_id,field,before_value,after_value,diff_json) VALUES('secret',?,'pricing_external_ref','null','{\"api_key\":\"synthetic-secret\"}','{}')",[note_id]).unwrap();
    let filtered = run(
        &book,
        Command::History(HistoryOptions {
            field: Some("pricing_external_ref".into()),
            include_stale: false,
            ..Default::default()
        }),
    );
    assert_eq!(list_values(&filtered, "events").len(), 1);
    let event = &list_values(&filtered, "events")[0];
    assert_eq!(list_values(event, "fields").len(), 2); // Selection is per event.
    assert_eq!(
        get(event, "summary"),
        Some(&s("Pricing provenance updated"))
    );
    assert_eq!(get(&filtered, "stale"), None);
    assert_eq!(
        get(&list_values(event, "fields")[1], "redacted"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn reverts_append_one_forward_event_and_repeating_is_a_noop() {
    let (_tmp, book) = book();
    let edit = note(&book, Some("Invoice"));
    let event = match get(&edit, "history_event_id").unwrap() {
        Value::Str(t) => t.as_str().unwrap(),
        _ => unreachable!(),
    };
    let before: String = book
        .connection()
        .query_row(
            "SELECT before_value FROM transaction_edit_fields WHERE event_id=?",
            [event],
            |r| r.get(0),
        )
        .unwrap();
    let result = revert(&book, event, None);
    assert_eq!(get(&result, "updated"), Some(&Value::Bool(true)));
    assert_eq!(
        get(get(&result, "transaction").unwrap(), "note"),
        Some(&s(""))
    );
    assert_eq!(count(&book, "transaction_edit_events"), 2);
    let retained: String = book
        .connection()
        .query_row(
            "SELECT before_value FROM transaction_edit_fields WHERE event_id=?",
            [event],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, retained);
    assert_eq!(
        get(&revert(&book, event, None), "updated"),
        Some(&Value::Bool(false))
    );
    assert_eq!(count(&book, "transaction_edit_events"), 2);
    let newest: String = book
        .connection()
        .query_row(
            "SELECT id FROM transaction_edit_events WHERE rowid=2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        get(
            get(&revert(&book, &newest, None), "transaction").unwrap(),
            "note"
        ),
        Some(&s("Invoice"))
    );
    let invalid = Command::Revert {
        transaction: "other".into(),
        event: event.into(),
        field: None,
        source: "gui".into(),
        reason: None,
    };
    assert_eq!(
        execute(book.connection(), &scope(), &invalid)
            .unwrap_err()
            .code,
        "not_found"
    );
    assert!(execute(
        book.connection(),
        &scope(),
        &Command::Revert {
            transaction: "tx".into(),
            event: event.into(),
            field: Some("tags".into()),
            source: "cli".into(),
            reason: None
        }
    )
    .is_err());
}

#[test]
fn grouped_revert_restores_tax_pricing_tags_and_rolls_back_invalidation_failure() {
    let (_tmp, book) = book();
    let edit = note(&book, Some("new"));
    let event = match get(&edit, "history_event_id").unwrap() {
        Value::Str(t) => t.as_str().unwrap(),
        _ => unreachable!(),
    };
    for (field, before, after) in [
        ("tags", r#"["Receipt","receipt"]"#, "[]"),
        ("taxable", "false", "null"),
        ("kind", r#""income""#, "null"),
        ("at_regime", r#""alt""#, "null"),
        ("at_category", r#""none""#, "null"),
        ("review_status", r#""review""#, "null"),
        ("fiat_rate", r#""1.000000000001""#, "null"),
        ("fiat_value", r#""-0.00000001""#, "null"),
        ("pricing_fetched_at", r#""2025""#, "null"),
    ] {
        book.connection().execute("INSERT INTO transaction_edit_fields(id,event_id,field,before_value,after_value,diff_json) VALUES(?,?,?,?,?,'{}')",params![field,event,field,before,after]).unwrap();
    }
    book.connection().execute_batch("CREATE TRIGGER deny_history_invalidation BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'fail invalidation'); END;").unwrap();
    let command = || Command::Revert {
        transaction: "tx".into(),
        event: event.into(),
        field: None,
        source: "gui".into(),
        reason: Some(" grouped ".into()),
    };
    assert!(execute(book.connection(), &scope(), &command()).is_err());
    assert_eq!(count(&book, "tags"), 0);
    assert_eq!(count(&book, "transaction_edit_events"), 1);
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
        Some(&s("new"))
    );
    book.connection()
        .execute_batch("DROP TRIGGER deny_history_invalidation")
        .unwrap();
    let reverted = run(&book, command());
    let tx = get(&reverted, "transaction").unwrap();
    assert_eq!(get(tx, "kind"), Some(&s("deposit")));
    assert_eq!(get(tx, "kind_override"), Some(&s("income")));
    assert_eq!(get(tx, "taxable"), Some(&Value::Bool(false)));
    assert_eq!(get(tx, "fiat_rate_exact"), Some(&s("1.000000000001")));
    assert_eq!(get(tx, "fiat_value_exact"), Some(&s("-0.00000001")));
    assert_eq!(list_values(tx, "tags").len(), 1);
    assert_eq!(count(&book, "transaction_edit_events"), 2);
    let rows: i64 = book
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM transaction_edit_fields WHERE event_id!=?",
            [event],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 10);
    let (before,after,source,reason): (i64,i64,String,String) = book.connection().query_row("SELECT journal_input_version,journal_input_version_after,source,reason FROM transaction_edit_events WHERE id!=?",[event],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        (before, after, source, reason),
        (1, 2, "gui".into(), "grouped".into())
    );
}

#[test]
fn every_history_field_reverts_and_fetch_timestamp_alone_remains_a_noop() {
    let (_tmp, book) = book();
    let fields = [
        ("note", r#""Old note""#),
        ("tags", r#"["Receipt"]"#),
        ("excluded", "true"),
        ("review_status", r#""review""#),
        ("taxable", "false"),
        ("kind", r#""income""#),
        ("at_regime", r#""alt""#),
        ("at_category", r#""none""#),
        ("fiat_currency", r#""USD""#),
        ("fiat_rate", r#""1.000000000001""#),
        ("fiat_value", r#""0.00000001""#),
        ("fiat_price_source", r#""manual""#),
        ("pricing_source_kind", r#""manual_override""#),
        ("pricing_quality", r#""exact""#),
        ("pricing_provider", r#""provider""#),
        ("pricing_pair", r#""BTC-USD""#),
        ("pricing_timestamp", r#""2025-01-01T00:00:00Z""#),
        ("pricing_granularity", r#""exact""#),
        ("pricing_method", r#""manual""#),
        ("pricing_external_ref", r#""invoice-42""#),
        ("pricing_fetched_at", r#""2025-01-01T00:00:00Z""#),
    ];
    for (field, old) in fields {
        book.connection().execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,source,changed_at) VALUES(?,'w','p','tx','wal','gui','2025')",[field]).unwrap();
        book.connection().execute("INSERT INTO transaction_edit_fields(id,event_id,field,before_value,after_value,diff_json) VALUES(?,?,?,?,'null','{}')",params![field,field,field,old]).unwrap();
        let result = revert(&book, field, None);
        assert_eq!(
            get(&result, "updated"),
            Some(&Value::Bool(field != "pricing_fetched_at")),
            "{field}"
        );
        assert_eq!(
            get(&revert(&book, field, None), "updated"),
            Some(&Value::Bool(false)),
            "{field}"
        );
    }
    assert_eq!(count(&book, "transaction_edit_events"), 41);
    let legacy = run(
        &book,
        Command::Get {
            transaction: "tx".into(),
        },
    );
    assert_eq!(get(&legacy, "kind"), Some(&s("deposit")));
    assert_eq!(get(&legacy, "kind_override"), Some(&s("income")));
}

#[test]
fn revert_cannot_exclude_a_newly_reviewed_custody_leg() {
    let (_tmp, book) = book();
    run(
        &book,
        Command::Excluded {
            transaction: "tx".into(),
            excluded: true,
            reason: None,
        },
    );
    let included = run(
        &book,
        Command::Excluded {
            transaction: "tx".into(),
            excluded: false,
            reason: None,
        },
    );
    let event = match get(&included, "history_event_id").unwrap() {
        Value::Str(t) => t.as_str().unwrap(),
        _ => unreachable!(),
    };
    book.connection().execute_batch("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,created_at) VALUES('revert-component','revert-lineage','w','p',1,'transfer','active','2025'); INSERT INTO custody_component_legs(id,component_id,workspace_id,profile_id,transaction_id,anchor_transaction_id,role,ordinal,rail,asset,exposure,conservation_unit,amount_msat,created_at) VALUES('revert-leg','revert-component','w','p','tx','tx','source',0,'onchain','BTC','BTC','BTC',1000,'2025');").unwrap();
    let command = Command::Revert {
        transaction: "tx".into(),
        event: event.into(),
        field: None,
        source: "gui".into(),
        reason: None,
    };
    assert_eq!(
        execute(book.connection(), &scope(), &command)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(count(&book, "transaction_edit_events"), 2);
    assert_eq!(
        get(
            &run(
                &book,
                Command::Get {
                    transaction: "tx".into()
                }
            ),
            "excluded"
        ),
        Some(&Value::Bool(false))
    );
}

#[test]
fn coarse_price_acceptance_binds_the_price_labels_history_and_reverts() {
    use kassiber_transactions::metadata::{GroupedUpdate, HistoryOptions, PricingUpdate};
    let (_tmp, book) = book();
    let accept = |book: &Book, pricing: bool| {
        execute(
            book.connection(),
            &scope(),
            &Command::Update(Box::new(GroupedUpdate {
                accept_coarse_price: true,
                pricing: pricing.then(|| PricingUpdate {
                    fiat_rate: s("1"),
                    ..Default::default()
                }),
                ..grouped()
            })),
        )
    };
    let refused = |error: kassiber_envelope::AppError| (error.code, error.message);
    assert_eq!(
        refused(accept(&book, false).unwrap_err()),
        (
            "validation".into(),
            "only a coarse price can be accepted".into()
        )
    );
    book.connection()
        .execute(
            "UPDATE transactions SET pricing_quality='coarse_fallback' WHERE id='tx'",
            [],
        )
        .unwrap();
    assert_eq!(
        refused(accept(&book, true).unwrap_err()),
        (
            "validation".into(),
            "accept the current coarse price or set a new one, not both".into()
        )
    );
    assert_eq!(
        refused(accept(&book, false).unwrap_err()),
        (
            "validation".into(),
            "the coarse price has no value to accept".into()
        )
    );
    // The rate prices the row first; a zero rate falls back to the value.
    book.connection()
        .execute(
            "UPDATE transactions SET fiat_rate=0.0,fiat_rate_exact='0',fiat_value=12.5,fiat_value_exact='12.50' WHERE id='tx'",
            [],
        )
        .unwrap();
    let saved = accept(&book, false).unwrap();
    assert_eq!(get(&saved, "updated"), Some(&Value::Bool(true)));
    let stored = |book: &Book| -> Option<String> {
        book.connection()
            .query_row(
                "SELECT pricing_review_accepted_price FROM transactions WHERE id='tx'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(stored(&book).as_deref(), Some("value:12.50"));
    book.connection()
        .execute(
            "UPDATE transactions SET fiat_rate=3197.6,fiat_rate_exact='3197.60' WHERE id='tx'",
            [],
        )
        .unwrap();
    accept(&book, false).unwrap();
    assert_eq!(stored(&book).as_deref(), Some("rate:3197.60"));
    // Accepting the same price again changes nothing.
    let again = accept(&book, false).unwrap();
    assert_eq!(get(&again, "updated"), Some(&Value::Bool(false)));
    let history = run(
        &book,
        Command::History(HistoryOptions {
            transaction: Some("tx".into()),
            field_family: Some("pricing".into()),
            ..Default::default()
        }),
    );
    let Some(Value::List(events)) = get(&history, "events") else {
        panic!("{history:?}")
    };
    // Both edits may share a second; the newest-first order then follows
    // their random event ids, so compare the set.
    let mut labels: Vec<_> = events
        .iter()
        .map(|event| {
            let Some(Value::List(fields)) = get(event, "fields") else {
                panic!()
            };
            (
                get(&fields[0], "label").cloned(),
                get(&fields[0], "after_label").cloned(),
                get(&fields[0], "family").cloned(),
            )
        })
        .collect();
    labels.sort_by_key(|(_, after, _)| format!("{after:?}"));
    let row = |after: &str| {
        (
            Some(s("Approximate price accepted")),
            Some(s(after)),
            Some(s("pricing")),
        )
    };
    assert_eq!(
        labels,
        [
            row("Accepted price per unit 3197.60"),
            row("Accepted value 12.50")
        ]
    );
    let stale = run(&book, Command::Stale);
    assert_eq!(
        get(&stale, "family_counts"),
        Some(&kassiber_envelope::dict([("pricing", Value::from(2))]))
    );
    let latest: String = book
        .connection()
        .query_row(
            "SELECT event_id FROM transaction_edit_fields WHERE field='pricing_review_accepted' AND after_value LIKE '%rate:%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let reverted = run(
        &book,
        Command::Revert {
            transaction: "tx".into(),
            event: latest,
            field: Some("pricing_review_accepted".into()),
            source: "cli".into(),
            reason: None,
        },
    );
    assert_eq!(get(&reverted, "updated"), Some(&Value::Bool(true)));
    assert_eq!(stored(&book).as_deref(), Some("value:12.50"));
}

/// d444f4db5 and f823b4729, as the merged Python answers them
/// (`tests/test_rust_grouped_metadata_services.py` compares the bytes).
#[test]
fn grouped_prices_bound_the_valuations_they_imply() {
    use kassiber_transactions::metadata::{GroupedUpdate, PricingUpdate};
    let (_tmp, book) = book();
    book.connection()
        .execute("UPDATE transactions SET amount=100000001 WHERE id='tx'", [])
        .unwrap();
    for (rate, total, message) in [
        (
            "",
            "1e18",
            "fiat_rate is outside the supported decimal range",
        ),
        (
            "1e10",
            "1e17",
            "transaction pricing input: derived fiat_rate is outside the supported decimal range",
        ),
        (
            "1e-12",
            "",
            "fiat_value is outside the supported decimal range",
        ),
        (
            "1e-11",
            "5",
            "transaction pricing input: derived fiat_value is outside the supported decimal range",
        ),
        (
            "9999999999999999999.5",
            "",
            "fiat_rate is outside the supported decimal range",
        ),
    ] {
        let error = execute(
            book.connection(),
            &scope(),
            &Command::Update(Box::new(GroupedUpdate {
                pricing: Some(PricingUpdate {
                    source_kind: s("manual_override"),
                    fiat_rate: if rate.is_empty() {
                        Value::Null
                    } else {
                        s(rate)
                    },
                    fiat_value: if total.is_empty() {
                        Value::Null
                    } else {
                        s(total)
                    },
                    ..Default::default()
                }),
                ..grouped()
            })),
        )
        .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("validation", message),
            "{rate} {total}"
        );
    }
    assert_eq!(count(&book, "transaction_edit_events"), 0);
    // The smallest implied total the engine compares as nonzero is accepted.
    execute(
        book.connection(),
        &scope(),
        &Command::Update(Box::new(GroupedUpdate {
            pricing: Some(PricingUpdate {
                source_kind: s("manual_override"),
                fiat_rate: s("1e-10"),
                fiat_value: s("5"),
                ..Default::default()
            }),
            ..grouped()
        })),
    )
    .unwrap();
}

#[test]
fn price_reverts_bound_restored_values_and_implied_valuations() {
    use kassiber_transactions::metadata::{GroupedUpdate, PricingUpdate};
    let (_tmp, book) = book();
    let mut event = String::new();
    for rate in ["1e15", "2"] {
        let edited = run(
            &book,
            Command::Update(Box::new(GroupedUpdate {
                pricing: Some(PricingUpdate {
                    source_kind: s("manual_override"),
                    fiat_currency: s("EUR"),
                    fiat_rate: s(rate),
                    ..Default::default()
                }),
                ..grouped()
            })),
        );
        event = match get(&edited, "history_event_id").unwrap() {
            Value::Str(t) => t.as_str().unwrap().to_owned(),
            _ => unreachable!(),
        };
    }
    // 10,000 BTC: restoring the 1e15 rate would imply a 1e19 total.
    book.connection()
        .execute(
            "UPDATE transactions SET amount=1000000000000000 WHERE id='tx'",
            [],
        )
        .unwrap();
    let refused = |field: Option<&str>| {
        execute(
            book.connection(),
            &scope(),
            &Command::Revert {
                transaction: "tx".into(),
                event: event.clone(),
                field: field.map(str::to_owned),
                source: "cli".into(),
                reason: None,
            },
        )
        .unwrap_err()
    };
    for field in [None, Some("fiat_rate")] {
        let error = refused(field);
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            (
                "validation",
                "transaction tx: derived fiat_value is outside the supported decimal range"
            )
        );
    }
    for (stored, message) in [
        (
            "\"1e40\"",
            "fiat_rate is outside the supported decimal range",
        ),
        ("\"NaN\"", "fiat_rate must be a finite decimal number"),
        ("\"bad\"", "fiat_rate must be a decimal number"),
        (
            "\"9999999999999999999.5\"",
            "fiat_rate is outside the supported decimal range",
        ),
    ] {
        book.connection()
            .execute(
                "UPDATE transaction_edit_fields SET before_value=? WHERE event_id=? AND field='fiat_rate'",
                params![stored, event],
            )
            .unwrap();
        let error = refused(Some("fiat_rate"));
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("validation", message),
            "{stored}"
        );
    }
    assert_eq!(count(&book, "transaction_edit_events"), 2);
}
