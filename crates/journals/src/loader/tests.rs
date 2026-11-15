use super::*;

#[test]
fn native_loader_scalar_helpers_preserve_types_and_missing_values() {
    use kassiber_pyjson::PyStr;
    for (source, expected) in [
        ("null", "null"),
        ("true", "true"),
        ("9223372036854775808", "9223372036854775808"),
        ("1.25", "1.25"),
        ("\"钱\"", "\"钱\""),
        ("[1, false]", "[1,false]"),
        ("{\"a\": [1]}", "{\"a\":[1]}"),
    ] {
        assert_eq!(
            convert(kassiber_pytext::loads(source).unwrap())
                .unwrap()
                .to_string(),
            expected
        );
    }
    assert!(convert(Value::Str(PyStr::from_code_points(vec![0xd800]).unwrap())).is_err());
    assert!(convert(Value::Dict(vec![(Key::Int(1.into()), Value::Null)])).is_err());
    assert!(convert(Value::Dict(vec![(
        Key::Str(PyStr::from_code_points(vec![0xd800]).unwrap()),
        Value::Null
    )]))
    .is_err());
    assert!(convert(Value::Decimal(kassiber_pydecimal::Decimal::zero())).is_err());
    assert!(!truth(&Value::Decimal(kassiber_pydecimal::Decimal::zero())));
    assert!(truth(&Value::Decimal("0.01".parse().unwrap())));
    for (source, expected) in [
        ("null", false),
        ("false", false),
        ("true", true),
        ("0", false),
        ("1", true),
        ("0.0", false),
        ("-0.1", true),
        ("\"\"", false),
        ("\"x\"", true),
        ("[]", false),
        ("[0]", true),
        ("{}", false),
        ("{\"a\": 0}", true),
    ] {
        assert_eq!(
            truth(&kassiber_pytext::loads(source).unwrap()),
            expected,
            "{source}"
        );
    }
    let mut row = dict([("first", Value::from(1))]);
    set(&mut row, "first", Value::from(2));
    set(&mut row, "second", Value::from(3));
    assert_eq!(
        row,
        dict([("first", Value::from(2)), ("second", Value::from(3))])
    );
    assert_eq!(get(&row, "missing"), &Value::Null);
    assert_eq!(get(&row, "first"), &Value::from(2));
    assert_eq!(get(&Value::Null, "first"), &Value::Null);
    for value in [Value::Null, Value::from("bad JSON"), Value::from("null")] {
        assert_eq!(parsed(&value), Value::Null);
    }
    assert_eq!(
        parsed(&Value::from("{\"a\": 1}")),
        dict([("a", Value::from(1))])
    );
    assert_eq!(sql_value(ValueRef::Null).unwrap(), Value::Null);
}

#[test]
fn native_loader_refusal_attributes_and_display_keep_private_values_out() {
    let error = kassiber_envelope::AppError::new("validation", "synthetic")
        .with_hint("hint")
        .with_details(dict([("field", Value::from("amount"))]))
        .retryable(true);
    let Error::Reference(refusal) = app_refusal(error) else {
        panic!("reference")
    };
    assert_eq!(refusal.family, "AppError");
    assert_eq!(refusal.code.as_deref(), Some("validation"));
    assert_eq!(refusal.message, "synthetic");
    assert_eq!(refusal.hint.as_deref(), Some("hint"));
    assert_eq!(refusal.retryable, Some(true));
    assert_eq!(refusal.details, serde_json::json!({"field": "amount"}));
    let Error::Reference(empty) = app_refusal(kassiber_envelope::AppError::new("x", "y")) else {
        panic!("reference")
    };
    assert!(empty.details.is_null());
    assert_eq!(empty.retryable, Some(false));
    for (error, expected) in [
        (
            Error::TransactionRequired,
            "custody loading requires the caller's write transaction",
        ),
        (
            Error::Database(rusqlite::Error::InvalidParameterName("private".into())),
            "custody snapshot database read failed",
        ),
        (Error::Unsupported("synthetic"), "synthetic"),
        (
            Error::Invalid("private".into()),
            "custody snapshot contains unsupported facts",
        ),
        (
            Error::Reference(refusal),
            "custody snapshot reference refused",
        ),
        (
            Error::Json(
                kassiber_pyjson::dumps(&Value::Float(f64::NAN), &Options::new().allow_nan(false))
                    .unwrap_err(),
            ),
            "custody snapshot v1 serialization failed",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
    let conn = Connection::open_in_memory().unwrap();
    assert!(operational(
        &query(&conn, "SELECT absent FROM missing", []).unwrap_err()
    ));
    assert!(!operational(&Error::Invalid("private".into())));
    assert!(!operational(&Error::Database(
        rusqlite::Error::InvalidQuery
    )));
}

#[test]
fn native_loader_utf8_refusals_preserve_python_column_order_and_message() {
    let conn = Connection::open_in_memory().unwrap();
    let refusal = |sql: &str, direct: bool| {
        let error = if direct {
            super::direct::Reader::default()
                .rows(&conn, "rows[]", sql, [], true)
                .unwrap_err()
        } else {
            query(&conn, sql, []).unwrap_err()
        };
        let Error::Reference(refusal) = error else {
            panic!("expected reference refusal")
        };
        assert_eq!(refusal.family, "OperationalError");
        refusal.message.clone()
    };
    for direct in [false, true] {
        for (sql, expected) in [
            (
                "SELECT CAST(x'ff00' AS TEXT) AS note",
                "Could not decode to UTF-8 column 'note' with text '�'",
            ),
            (
                "SELECT CAST(x'ff' AS TEXT) AS z, CAST(x'fe' AS TEXT) AS a",
                "Could not decode to UTF-8 column 'z' with text '�'",
            ),
            (
                "SELECT 'first' AS duplicate, CAST(x'ff' AS TEXT) AS duplicate",
                "Could not decode to UTF-8 column 'duplicate' with text '�'",
            ),
            (
                "SELECT CAST(x'61626300ff' AS TEXT) AS x",
                "Could not decode to UTF-8 column 'x' with text 'abc'",
            ),
            (
                "SELECT CAST(x'ff' AS TEXT) AS 钱",
                "Could not decode to UTF-8 column '���' with text '�'",
            ),
            (
                "SELECT CAST(x'ff' AS TEXT) AS _closed_confirmation_observation",
                "Could not decode to UTF-8 column '_closed_confirmation_observation' with text '�'",
            ),
        ] {
            assert_eq!(refusal(sql, direct), expected);
        }
        let sql = format!("SELECT CAST(x'{}ff' AS TEXT) AS x", "61".repeat(250));
        assert_eq!(
            refusal(&sql, direct),
            format!(
                "Could not decode to UTF-8 column 'x' with text '{}",
                "a".repeat(150)
            )
        );
    }
}

#[test]
fn native_loader_utf8_optional_reviews_preserve_python_fallback() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&text(get(case, "sql"))).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; UPDATE custody_gap_reviews SET reason=CAST(x'ff00' AS TEXT) WHERE revision=2").unwrap();
    let changes = conn.total_changes();
    let mut expected = kassiber_pytext::loads(&text(get(case, "expected"))).unwrap();
    set(&mut expected, "gap_reviews", list(vec![]));
    let snapshot = load(
        &conn,
        get(case, "profile").clone(),
        &text(get(case, "as_of")),
    )
    .unwrap();
    assert_eq!(
        snapshot.v1().unwrap(),
        kassiber_pyjson::dumps(&expected, &Options::new().allow_nan(true)).unwrap()
    );
    assert_eq!(changes, conn.total_changes());
    assert!(!conn.is_autocommit());
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn native_loader_refuses_mixed_revision_references_before_output() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    for field in ["supersedes_component_id", "superseded_by_component_id"] {
        for scalar in ["7", "1.5", "1e999", "-1e999"] {
            let conn = Connection::open_in_memory().unwrap();
            let sql = text(get(case, "sql"))
                .replace(&format!("\"{field}\" TEXT"), &format!("\"{field}\""));
            conn.execute_batch(&sql).unwrap();
            conn.execute_batch(&format!("BEGIN IMMEDIATE; UPDATE custody_components SET {field}={scalar} WHERE id='active-a'" )).unwrap();
            let changes = conn.total_changes();
            for _ in 0..2 {
                let Err(Error::Reference(refusal)) = load(
                    &conn,
                    get(case, "profile").clone(),
                    &text(get(case, "as_of")),
                ) else {
                    panic!("expected mixed revision validation refusal")
                };
                assert_eq!(refusal.family, "AppError");
                assert_eq!(refusal.code.as_deref(), Some("validation"));
                assert_eq!(
                    refusal.message,
                    "Custody revision references contain incompatible types"
                );
            }
            assert_eq!(changes, conn.total_changes());
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn native_loader_revision_queries_keep_sqlite_binding_types() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    let mut sql = text(get(case, "sql"));
    for column in [
        "id",
        "supersedes_component_id",
        "superseded_by_component_id",
    ] {
        sql = sql.replace(&format!("\"{column}\" TEXT"), &format!("\"{column}\""));
    }
    for number in ["7", "7.0", "1.5", "9007199254740993", "1e999", "-1e999"] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&sql).unwrap();
        conn.execute_batch(&format!("BEGIN IMMEDIATE;
            UPDATE custody_components SET supersedes_component_id=NULL,superseded_by_component_id=NULL;
            UPDATE custody_components SET id={number} WHERE state='superseded';
            UPDATE custody_components SET supersedes_component_id={number} WHERE id='active-a';
            UPDATE custody_components SET superseded_by_component_id={number} WHERE id='active-b';")).unwrap();
        let changes = conn.total_changes();
        let snapshot = load(
            &conn,
            get(case, "profile").clone(),
            &text(get(case, "as_of")),
        )
        .unwrap();
        let actual = kassiber_pytext::loads(&snapshot.v1().unwrap()).unwrap();
        let Value::List(headers) = get(&actual, "revision_headers") else {
            panic!("headers")
        };
        assert_eq!(headers.len(), 1, "{number}");
        let expected = query(
            &conn,
            "SELECT id FROM custody_components WHERE state='superseded'",
            [],
        )
        .unwrap();
        assert_eq!(get(&headers[0], "id"), get(&expected[0], "id"));
        assert_eq!(conn.total_changes(), changes);
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn native_loader_matches_python_database_bytes() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    for case in cases {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        conn.execute_batch(&text(get(&case, "sql"))).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let changes = conn.total_changes();
        let snapshot = load(
            &conn,
            get(&case, "profile").clone(),
            &text(get(&case, "as_of")),
        )
        .unwrap();
        assert_eq!(
            snapshot.v1().unwrap(),
            text(get(&case, "expected")),
            "{}",
            text(get(&case, "name"))
        );
        // The same raw records as Python's bytes, with no wire text. Database
        // admission retains legacy quantities; v1's strict quantity admission
        // is independently pinned in the snapshot tests.
        let adapter = snapshot.into_custody().unwrap();
        // Python's v1 uses Infinity constants, while serde's text grammar
        // accepts only JSON numbers. Preserve their float value as overflowing
        // exponents for this separate admission/record comparison. The exact
        // Python v1 bytes have already been checked above.
        let reference_text = serde_json::to_string(
            &convert(kassiber_pytext::loads(&text(get(&case, "expected"))).unwrap()).unwrap(),
        )
        .unwrap();
        let reference: kassiber_custody::snapshot::Snapshot =
            serde_json::from_str(&reference_text).unwrap();
        // Typed rows serialize REALs from f64; the native compatibility rows
        // retain Python's exponent spelling. Compare the exact Python scalar
        // model (including integer/float distinctions and signed zero), while
        // the assertion above independently keeps the raw v1 byte contract.
        let scalar_text = |value| {
            kassiber_pyjson::dumps(
                &kassiber_custody::codec::from_json(&value).unwrap(),
                &Options::new(),
            )
            .unwrap()
        };
        assert_eq!(
            scalar_text(serde_json::to_value(adapter).unwrap()),
            scalar_text(serde_json::to_value(reference).unwrap()),
            "{}",
            text(get(&case, "name"))
        );
        assert_eq!(conn.total_changes(), changes);
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn native_loader_requires_caller_write_transaction() {
    let conn = Connection::open_in_memory().unwrap();
    for begin in [None, Some("BEGIN DEFERRED")] {
        if let Some(sql) = begin {
            conn.execute_batch(sql).unwrap();
        }
        assert!(matches!(
            load(&conn, dict([("id", Value::from("p"))]), "now"),
            Err(Error::TransactionRequired)
        ));
        if begin.is_some() {
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
    conn.execute_batch(
        "CREATE TABLE canary(value INTEGER); BEGIN IMMEDIATE; INSERT INTO canary VALUES(1)",
    )
    .unwrap();
    assert!(matches!(
        load(&conn, dict([]), "now"),
        Err(Error::Database(_))
    ));
    assert!(!conn.is_autocommit());
    assert_eq!(
        conn.query_row("SELECT count(*) FROM canary", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM canary", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn native_loader_keeps_sql_values_and_refuses_blob_coercion() {
    let conn = Connection::open_in_memory().unwrap();
    let rows=query(&conn,"SELECT NULL AS n,9223372036854775807 AS i,1.0 AS f,'钱' AS s,1 AS duplicate,2 AS duplicate",[]).unwrap();
    assert_eq!(kassiber_pyjson::dumps(&list(rows),&Options::new()).unwrap(),"[{\"n\": null, \"i\": 9223372036854775807, \"f\": 1.0, \"s\": \"\\u94b1\", \"duplicate\": 1}]");
    let rows = query(&conn, "SELECT x'ff00' AS blob", []).unwrap();
    assert_eq!(get(&rows[0], "blob"), &Value::Bytes(vec![255, 0]));
    assert_eq!(
        kassiber_pyjson::dumps(&list(rows), &Options::new()),
        Err(kassiber_pyjson::Error::BytesNotSerializable)
    );
}

#[test]
fn native_loader_preserves_nonfinite_real_until_downstream() {
    for number in [f64::INFINITY, f64::NEG_INFINITY, -0.0, 1.0, 1.234e-7] {
        let stored = super::direct::scalar(ValueRef::Real(number)).unwrap();
        let expected = Value::Float(number);
        assert_eq!(convert(expected.clone()).unwrap(), stored);
        // Gate::encode first crosses serde's Value serializer. The overflowing
        // exponent must survive that round trip too, never becoming null.
        let serialized = serde_json::to_value(&stored).unwrap();
        let restored = kassiber_custody::codec::from_json(&serialized).unwrap();
        assert_eq!(restored, expected);
        assert_eq!(
            kassiber_pyjson::dumps(&restored, &Options::new().allow_nan(true)).unwrap(),
            kassiber_pyjson::dumps(&expected, &Options::new().allow_nan(true)).unwrap()
        );
    }
    assert!(float_storage(f64::NAN).is_err());
}

#[test]
fn native_loader_legacy_scalars_keep_downstream_admission() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&text(get(case, "sql"))).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; UPDATE transactions SET amount=1.5,fee=''")
        .unwrap();
    let changes = conn.total_changes();
    let loaded = load(
        &conn,
        get(case, "profile").clone(),
        &text(get(case, "as_of")),
    )
    .unwrap();
    assert_eq!(loaded.project().err().unwrap().stage, "components");
    let snapshot = loaded.into_custody().unwrap();
    for row in &snapshot.transactions {
        assert_eq!(row["amount"], serde_json::json!(1.5));
        assert_eq!(row["fee"], "");
    }
    assert_eq!(changes, conn.total_changes());
    assert!(!conn.is_autocommit());
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn review_assembler_preserves_raw_admission_and_refusal_ownership() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    for encrypted in [false, true] {
        let conn = Connection::open_in_memory().unwrap();
        if encrypted {
            conn.pragma_update(None, "key", "synthetic-strict-admission-key")
                .unwrap();
        }
        conn.execute_batch(&text(get(case, "sql"))).unwrap();
        conn.execute_batch("UPDATE transactions SET amount=1.5")
            .unwrap();
        for begin in ["BEGIN", "BEGIN IMMEDIATE"] {
            conn.execute_batch(begin).unwrap();
            let changes = conn.total_changes();
            let loaded = load_read_snapshot(
                &conn,
                get(case, "profile").clone(),
                &text(get(case, "as_of")),
            )
            .unwrap()
            .into_custody()
            .unwrap();
            assert!(!loaded.transactions.is_empty());
            assert!(loaded
                .transactions
                .iter()
                .all(|row| row["amount"] == serde_json::json!(1.5)));
            let state = conn.transaction_state(Some("main")).unwrap();
            let reference = kassiber_custody::run_loaded(&loaded, true).err().unwrap();
            let Err(error) = crate::pipeline::load_review_state(
                &conn,
                get(case, "profile").clone(),
                &text(get(case, "as_of")),
                |_| (),
            ) else {
                panic!("review assembler must preserve the consuming stage refusal")
            };
            assert_eq!(error.message, reference.message);
            assert_eq!(
                error.code.as_deref().unwrap_or(&error.family),
                reference.code
            );
            assert_eq!(conn.total_changes(), changes);
            assert_eq!(conn.transaction_state(Some("main")).unwrap(), state);
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn native_loader_caught_integer_errors_still_propagate_overflow() {
    assert_eq!(
        ownership::legacy_integer(&Value::from("invalid")).unwrap(),
        None
    );
    assert_eq!(ownership::legacy_integer(&Value::Null).unwrap(), None);
    assert_eq!(
        ownership::legacy_integer(&Value::Float(1.9)).unwrap(),
        Some(1.into())
    );
    for real in [f64::INFINITY, f64::NEG_INFINITY] {
        let Err(Error::Reference(error)) = ownership::legacy_integer(&Value::Float(real)) else {
            panic!("int(inf) must refuse")
        };
        assert_eq!(error.family, "OverflowError");
        assert_eq!(error.message, "cannot convert float infinity to integer");
    }
}

#[test]
fn native_loader_integer_overflow_refusals_match_python() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "receipts-counts-0")
        .unwrap();
    for (table, column) in [
        ("chain_observation_provenance", "authority_version"),
        ("wallet_utxos", "branch_index"),
        ("wallet_utxos", "address_index"),
    ] {
        for number in ["1e999", "-1e999"] {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch(&text(get(case, "sql"))).unwrap();
            conn.execute_batch(&format!(
                "BEGIN IMMEDIATE; UPDATE {table} SET {column}={number}"
            ))
            .unwrap();
            let changes = conn.total_changes();
            let Err(Error::Reference(error)) = load(
                &conn,
                get(case, "profile").clone(),
                &text(get(case, "as_of")),
            ) else {
                panic!("{table}.{column} {number} must refuse")
            };
            assert_eq!(error.family, "OverflowError");
            assert_eq!(error.message, "cannot convert float infinity to integer");
            assert_eq!(conn.total_changes(), changes);
            assert!(!conn.is_autocommit());
            conn.execute_batch("ROLLBACK").unwrap();
        }
    }
}

#[test]
fn native_loader_bounds_gap_worklist() {
    for structured in [false, true] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE wallets(id TEXT,profile_id TEXT,label TEXT,kind TEXT,config_json TEXT);
          CREATE TABLE transactions(id TEXT,profile_id TEXT,wallet_id TEXT,occurred_at TEXT,created_at TEXT,direction TEXT,asset TEXT,amount INTEGER,fee INTEGER,amount_includes_fee INTEGER,excluded INTEGER,kind TEXT,kind_override TEXT,privacy_boundary TEXT,external_id TEXT,raw_json TEXT);
          INSERT INTO wallets VALUES('a','p','A','custom','{}'),('b','p','B','custom','{}');
          WITH RECURSIVE n(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM n WHERE i<50000)
          INSERT INTO transactions(id,profile_id,wallet_id,excluded) SELECT 'orphan-'||i,'p','absent',0 FROM n;").unwrap();
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [r#"{"chain":"bitcoin"}"#],
        )
        .unwrap();
        for i in 0..270 {
            conn.execute("INSERT INTO transactions VALUES(?,'p','a','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','outbound','BTC',?,0,0,0,'withdrawal',NULL,?,'acq','{}')",rusqlite::params![format!("out-{i:03}"),1_000_000+i,if structured { "whirlpool" } else { "" }]).unwrap();
        }
        for i in 0..513 {
            conn.execute("INSERT INTO transactions VALUES(?,'p','b','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','inbound','BTC',900000,0,0,0,'deposit',NULL,NULL,'acq','{}')",[format!("in-{i:03}")]).unwrap();
        }
        let result = worklist::load(&conn, "p", &mut HashMap::new()).unwrap();
        assert_eq!(get(&result, "large_book"), &Value::Bool(true));
        let Value::List(rows) = get(&result, "rows") else {
            panic!("rows")
        };
        let outbound = rows
            .iter()
            .filter(|row| get(row, "direction") == &Value::from("outbound"))
            .collect::<Vec<_>>();
        assert_eq!(outbound.len(), if structured { 256 } else { 64 });
        for row in &outbound {
            assert_eq!(
                get(row, "wallet_config_json"),
                &Value::from(r#"{"chain": "bitcoin"}"#)
            );
        }
        assert_eq!(
            text(get(outbound[0], "id")),
            if structured { "out-000" } else { "out-206" }
        );
        assert_eq!(
            text(get(outbound.last().unwrap(), "id")),
            if structured { "out-255" } else { "out-269" }
        );
        assert_eq!(rows.len() - outbound.len(), 512);
        assert_eq!(text(get(&rows[0], "id")), "in-000");
    }
}

#[test]
fn native_loader_worklist_ranks_sources_across_assets_before_budgeting() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE wallets(id TEXT,profile_id TEXT,label TEXT,kind TEXT,config_json TEXT);
      CREATE TABLE transactions(id TEXT,profile_id TEXT,wallet_id TEXT,occurred_at TEXT,created_at TEXT,direction TEXT,asset TEXT,amount INTEGER,fee INTEGER,amount_includes_fee INTEGER,excluded INTEGER,kind TEXT,kind_override TEXT,privacy_boundary TEXT,external_id TEXT,raw_json TEXT);
      INSERT INTO wallets VALUES('a','p','A','custom','{}');
      WITH RECURSIVE n(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM n WHERE i<50000)
      INSERT INTO transactions(id,profile_id,wallet_id,excluded) SELECT 'orphan-'||i,'p','absent',0 FROM n;").unwrap();
    for (asset, amount) in [("BTC", 100_000), ("L-BTC", 200_000)] {
        for i in 0..64 {
            conn.execute("INSERT INTO transactions VALUES(?,'p','a','2026-01-01','2026-01-01','outbound',?,?,0,0,0,'withdrawal',NULL,NULL,'acq','{}')", rusqlite::params![format!("{asset}-{i:03}"),asset,amount+i]).unwrap();
        }
    }
    let result = worklist::load(&conn, "p", &mut HashMap::new()).unwrap();
    let Value::List(rows) = get(&result, "rows") else {
        panic!("rows")
    };
    assert_eq!(rows.len(), 64);
    assert!(rows
        .iter()
        .all(|row| get(row, "asset") == &Value::from("L-BTC")));
    assert_eq!(get(&rows[0], "id"), &Value::from("L-BTC-000"));
    assert_eq!(get(&rows[63], "id"), &Value::from("L-BTC-063"));
}

#[test]
fn native_loader_preserves_receipt_error_precedence() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader_errors.json")).unwrap()
    else {
        panic!("cases")
    };
    for case in cases {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&text(get(&case, "sql"))).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let changes = conn.total_changes();
        let result = load(
            &conn,
            get(&case, "profile").clone(),
            &text(get(&case, "as_of")),
        );
        let Err(Error::Reference(refusal)) = result else {
            panic!("expected reference refusal: {}", text(get(&case, "name")))
        };
        let actual = serde_json::to_value(refusal).unwrap();
        let expected = convert(get(&case, "expected").clone()).unwrap();
        for key in ["family", "message", "code"] {
            assert_eq!(
                actual[key],
                expected[key],
                "{}: {key}",
                text(get(&case, "name"))
            );
        }
        assert!(!conn.is_autocommit());
        assert_eq!(changes, conn.total_changes());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn native_loader_selects_bounded_worklist_from_active_count() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "config-0")
        .unwrap();
    for excluded in [0, 1] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&text(get(case, "sql"))).unwrap();
        conn.execute_batch(&format!("WITH RECURSIVE n(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM n WHERE i<50000) INSERT INTO transactions(id,profile_id,wallet_id,excluded) SELECT 'orphan-'||i,'profile-1','absent',{excluded} FROM n")).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let snapshot = load(
            &conn,
            get(case, "profile").clone(),
            &text(get(case, "as_of")),
        )
        .unwrap()
        .into_custody()
        .unwrap();
        assert_eq!(snapshot.gap_worklist.large_book, excluded == 0);
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn review_loader_preserves_read_and_writer_snapshots_without_writes() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    for encrypted in [false, true] {
        for case in &cases {
            let conn = Connection::open_in_memory().unwrap();
            if encrypted {
                conn.pragma_update(None, "key", "synthetic-read-loader-key")
                    .unwrap();
            }
            conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
            conn.execute_batch(&text(get(case, "sql"))).unwrap();
            for begin in ["BEGIN", "BEGIN IMMEDIATE"] {
                conn.execute_batch(begin).unwrap();
                let changes = conn.total_changes();
                let snapshot = load_read_snapshot(
                    &conn,
                    get(case, "profile").clone(),
                    &text(get(case, "as_of")),
                )
                .unwrap();
                assert_eq!(snapshot.v1().unwrap(), text(get(case, "expected")));
                assert_eq!(conn.total_changes(), changes);
                assert!(!conn.is_autocommit());
                conn.execute_batch("ROLLBACK").unwrap();
            }
        }
    }
    let conn = Connection::open_in_memory().unwrap();
    assert!(matches!(
        load_read_snapshot(&conn, dict([]), "now"),
        Err(Error::TransactionRequired)
    ));
    assert!(conn.is_autocommit());
    assert_eq!(conn.total_changes(), 0);
}

#[test]
fn native_loader_blob_conversions_preserve_bytes_and_consuming_policy() {
    for bytes in [
        vec![],
        vec![255, 0],
        b"77".to_vec(),
        b"  +7_7\n".to_vec(),
        b"{\"chain\":\"bitcoin\",\"private\":\"secret\"}".to_vec(),
    ] {
        let value = Value::Bytes(bytes.clone());
        assert_eq!(sql_value(ValueRef::Blob(&bytes)).unwrap(), value);
        let native = direct::scalar(ValueRef::Blob(&bytes)).unwrap();
        assert_eq!(kassiber_custody::codec::from_json(&native).unwrap(), value);
        assert_eq!(truth(&value), !bytes.is_empty());
        assert_eq!(text(&value), kassiber_pyjson::bytes_repr(&bytes));
        let safe = safe_row(dict([("config_json", value.clone())]), &mut HashMap::new()).unwrap();
        assert_eq!(get(&safe, "config_json"), &Value::from("{}"));
        assert_eq!(
            get(&safe, "config_text_mentions_samourai"),
            &Value::Bool(false)
        );
        assert_eq!(
            kassiber_pyjson::dumps(&value, &Options::new())
                .unwrap_err()
                .to_string(),
            "Object of type bytes is not JSON serializable"
        );
    }
    assert_eq!(
        ownership::integer(&Value::Bytes(b"77".to_vec())),
        Some(77.into())
    );
    assert_eq!(
        ownership::outpoint_integer(&Value::Bytes(b"77".to_vec())).unwrap(),
        77.into()
    );
    assert!(ownership::integer(&Value::Bytes("٧٧".as_bytes().to_vec())).is_none());
    assert!(ownership::outpoint_integer(&Value::Bytes("٧٧".as_bytes().to_vec())).is_err());
    let Err(Error::Reference(error)) = ownership::outpoint_integer(&Value::Bytes(vec![255, 0]))
    else {
        panic!("byte int must refuse")
    };
    assert_eq!(error.family, "ValueError");
    assert_eq!(
        error.message,
        "invalid literal for int() with base 10: b'\\xff\\x00'"
    );
    assert_eq!(
        parsed(&Value::Bytes(b"{\"chain\":\"bitcoin\"}".to_vec())),
        dict([("chain", "bitcoin".into())])
    );
    assert_eq!(
        ownership::config(&dict([(
            "config_json",
            Value::Bytes(b"{\"chain\":\"bitcoin\"}".to_vec())
        )]))
        .unwrap(),
        dict([("chain", "bitcoin".into())])
    );
    let Err(Error::Reference(error)) =
        ownership::config(&dict([("config_json", Value::Bytes(b"77".to_vec()))]))
    else {
        panic!("parsed int must refuse as int")
    };
    assert_eq!(error.message, "'int' object has no attribute 'get'");
}

#[test]
fn native_loader_blob_projection_retains_byte_identity() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "config-0")
        .unwrap();
    for bytes in [vec![], vec![255, 0], b"77".to_vec()] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&text(get(case, "sql"))).unwrap();
        conn.execute("UPDATE transactions SET note=?", [&bytes])
            .unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let snapshot = load(
            &conn,
            get(case, "profile").clone(),
            &text(get(case, "as_of")),
        )
        .unwrap();
        let projection = snapshot
            .project()
            .unwrap()
            .finalized_projection
            .unwrap()
            .py(&snapshot.custody().transactions)
            .unwrap();
        let Value::List(rows) = get(&projection, "rows") else {
            panic!("projection rows")
        };
        assert!(!rows.is_empty());
        for row in rows {
            assert_eq!(get(row, "note"), &Value::Bytes(bytes.clone()));
        }
        assert!(matches!(
            snapshot.v1(),
            Err(Error::Json(kassiber_pyjson::Error::BytesNotSerializable))
        ));
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
}

#[test]
fn native_loader_blob_revision_references_bind_and_refuse_before_output() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    let sql = text(get(case, "sql"));
    for expression in ["x'ff00'", "x'3737'", "x''"] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&sql).unwrap();
        conn.execute_batch(&format!("BEGIN IMMEDIATE;
            UPDATE custody_components SET supersedes_component_id=NULL,superseded_by_component_id=NULL;
            UPDATE custody_components SET id={expression} WHERE state='superseded';
            UPDATE custody_components SET supersedes_component_id={expression} WHERE id='active-a';
            UPDATE custody_components SET superseded_by_component_id={expression} WHERE id='active-b';")).unwrap();
        let changes = conn.total_changes();
        let loaded = load(
            &conn,
            get(case, "profile").clone(),
            &text(get(case, "as_of")),
        )
        .unwrap();
        assert!(matches!(
            loaded.v1(),
            Err(Error::Json(kassiber_pyjson::Error::BytesNotSerializable))
        ));
        let snapshot = loaded.into_custody().unwrap();
        assert_eq!(snapshot.revision_headers.len(), 1);
        let expected = query(
            &conn,
            "SELECT id FROM custody_components WHERE state='superseded'",
            [],
        )
        .unwrap();
        assert_eq!(
            kassiber_custody::codec::from_json(&snapshot.revision_headers[0]["id"]).unwrap(),
            get(&expected[0], "id").clone()
        );
        assert_eq!(changes, conn.total_changes());
        assert!(!conn.is_autocommit());
        conn.execute_batch("ROLLBACK").unwrap();
    }
    for field in ["supersedes_component_id", "superseded_by_component_id"] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&sql).unwrap();
        conn.execute_batch(&format!(
            "BEGIN IMMEDIATE; UPDATE custody_components SET {field}=x'ff00' WHERE id='active-a'"
        ))
        .unwrap();
        let changes = conn.total_changes();
        for _ in 0..2 {
            let Err(Error::Reference(error)) = load(
                &conn,
                get(case, "profile").clone(),
                &text(get(case, "as_of")),
            ) else {
                panic!("mixed bytes must refuse")
            };
            assert_eq!(error.family, "AppError");
            assert_eq!(error.code.as_deref(), Some("validation"));
            assert_eq!(
                error.message,
                "Custody revision references contain incompatible types"
            );
        }
        assert_eq!(changes, conn.total_changes());
        assert!(!conn.is_autocommit());
    }
}

#[test]
fn native_loader_refuses_mixed_blob_numeric_revisions_before_queries() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/loader.json")).unwrap()
    else {
        panic!("cases")
    };
    let case = cases
        .iter()
        .find(|case| text(get(case, "name")) == "structural-columns")
        .unwrap();
    let sql = text(get(case, "sql")).replace(
        "\"superseded_by_component_id\" TEXT",
        "\"superseded_by_component_id\"",
    );
    for scalar in ["7", "1.5", "1e999", "-1e999"] {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&sql).unwrap();
        conn.execute_batch(&format!("BEGIN IMMEDIATE; UPDATE custody_components SET supersedes_component_id=NULL,superseded_by_component_id=NULL; UPDATE custody_components SET supersedes_component_id=x'3737' WHERE id='active-a'; UPDATE custody_components SET superseded_by_component_id={scalar} WHERE id='active-b';")).unwrap();
        let changes = conn.total_changes();
        let Err(Error::Reference(error)) = load(
            &conn,
            get(case, "profile").clone(),
            &text(get(case, "as_of")),
        ) else {
            panic!("mixed bytes must refuse")
        };
        assert_eq!(error.family, "AppError");
        assert_eq!(error.code.as_deref(), Some("validation"));
        assert_eq!(
            error.message,
            "Custody revision references contain incompatible types"
        );
        assert_eq!(changes, conn.total_changes());
        assert!(!conn.is_autocommit());
    }
}

/// A book from the store's creation transcript with three same-time receipts
/// whose ids, creation times and file positions disagree.
fn same_time_book() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let creation: serde_json::Value =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','W','t');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','P','t');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('w','ws','p','Cold','custom','t');
        INSERT INTO import_batches(id,workspace_id,profile_id,source_format,imported_at) VALUES('b','ws','p','csv','t');").unwrap();
    // File order z, y, x; the ids and creation times say x, y, z.
    for (position, id, created, rate) in [
        (1, "z", "2026-01-03", "30000"),
        (2, "y", "2026-01-02", "20000"),
        (3, "x", "2026-01-01", "10000"),
    ] {
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,fiat_rate,fiat_rate_exact,created_at) VALUES(?1,'ws','p','w',?1,'2026-01-01T00:00:00Z','inbound','BTC',1000,?3,?3,?2)", rusqlite::params![id, created, rate]).unwrap();
        conn.execute("INSERT INTO import_batch_transactions(batch_id,transaction_id,source_position) VALUES('b',?,?)", rusqlite::params![id, position]).unwrap();
    }
    conn
}

#[test]
fn native_loader_orders_same_time_rows_by_source_position() {
    let conn = same_time_book();
    let ids = |sql: &str, column: usize| -> Vec<String> {
        conn.prepare(sql)
            .unwrap()
            .query_map(["p"], |r| r.get(column))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    // Composition reads the file order; the latest rate is the file's last row.
    assert_eq!(ids(TRANSACTIONS.as_str(), 0), ["z", "y", "x"]);
    assert_eq!(ids(RATE_ROWS.as_str(), 3), ["10000", "20000", "30000"]);
}

#[test]
fn rate_rows_leave_out_prices_in_another_currency() {
    let conn = same_time_book();
    conn.execute_batch("UPDATE profiles SET fiat_currency='USD'; UPDATE transactions SET fiat_currency='EUR' WHERE id='x'; UPDATE transactions SET fiat_currency=' usd' WHERE id='y'")
        .unwrap();
    let rates = |sql: &str| -> Vec<String> {
        conn.prepare(sql)
            .unwrap()
            .query_map(["p"], |r| r.get(3))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    // x is priced in EUR in a USD book; y and the unrecorded z stay rates.
    assert_eq!(rates(RATE_ROWS.as_str()), ["20000", "30000"]);
    for shape in 0..2 {
        assert_eq!(rates(&rate_rows_sql(shape, true)).len(), 2, "{shape}");
        assert_eq!(rates(&rate_rows_sql(shape, false)).len(), 3, "{shape}");
    }
}
