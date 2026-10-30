use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::Book;
use kassiber_transfers::{execute, Scope};

fn array(value: &Value) -> &[Value] {
    if let Value::List(v) = value {
        v
    } else {
        panic!("expected array")
    }
}
fn text(value: &Value) -> String {
    kassiber_envelope::py_str(value)
}
fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("python.json")).unwrap()
}
fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    for sql in array(get(&fixture(), "sql").unwrap()) {
        book.connection().execute_batch(&text(sql)).unwrap();
    }
    (dir, book)
}
fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}

#[test]
fn stored_reads_match_python_bytes_without_touching_rows() {
    let (_dir, book) = book();
    for case in array(get(&fixture(), "reads").unwrap()) {
        let before = book.connection().total_changes();
        let result = execute(
            book.connection(),
            &scope(),
            &text(get(case, "operation").unwrap()),
            get(case, "args").unwrap(),
        )
        .unwrap();
        assert_eq!(
            dumps(&result, &Options::default()).unwrap(),
            dumps(get(case, "expected").unwrap(), &Options::default()).unwrap(),
            "{}",
            text(get(case, "operation").unwrap())
        );
        assert_eq!(book.connection().total_changes(), before);
    }
}
#[test]
fn stored_scope_limits_and_receipt_selectors_fail_closed() {
    let (_dir, book) = book();
    for (args, code) in [
        (dict::<&str>([]), "validation"),
        (
            dict([("receipt_id", s("receipt")), ("idempotency_key", s("key"))]),
            "validation",
        ),
        (dict([("receipt_id", s("missing"))]), "not_found"),
    ] {
        assert_eq!(
            execute(book.connection(), &scope(), "review.receipt", &args)
                .unwrap_err()
                .code,
            code
        );
    }
    assert_eq!(
        execute(
            book.connection(),
            &scope(),
            "review.receipt",
            &dict([("idempotency_key", s("key"))])
        )
        .unwrap(),
        get(
            array(get(&fixture(), "reads").unwrap()).last().unwrap(),
            "expected"
        )
        .unwrap()
        .clone()
    );
    assert_eq!(
        execute(
            book.connection(),
            &Scope {
                workspace_id: "w",
                profile_id: "other"
            },
            "review.receipt",
            &dict([("receipt_id", s("receipt"))])
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    for n in [Value::Bool(true), 0.into(), 501.into()] {
        assert_eq!(
            execute(
                book.connection(),
                &scope(),
                "transfers.gaps.history",
                &dict([("gap_id", s("gap")), ("limit", n)])
            )
            .unwrap_err()
            .code,
            "custody_gap_review_validation"
        );
    }
    for operation in [
        "transfers.list",
        "transfers.payouts.list",
        "transfers.rules.list",
    ] {
        assert_eq!(
            execute(
                book.connection(),
                &Scope {
                    workspace_id: "w",
                    profile_id: "other"
                },
                operation,
                &dict::<&str>([])
            )
            .unwrap(),
            Value::List(vec![])
        );
    }
}

#[test]
fn malformed_stored_rules_are_disabled_with_python_reasons() {
    let (_dir, book) = book();
    for (raw, reason) in [
        ("[]", "predicate must be an object"),
        (
            r#"{"zz":1,"aa":2}"#,
            "predicate has unsupported field(s): aa, zz",
        ),
        (
            r#"{"out_asset":1}"#,
            "predicate field 'out_asset' must be a string",
        ),
        (
            r#"{"max_fee_pct":true}"#,
            "predicate field 'max_fee_pct' must be a number",
        ),
        (
            r#"{"max_fee_pct":-1}"#,
            "predicate field 'max_fee_pct' must be a finite non-negative number",
        ),
        (
            r#"{"max_fee_pct":"NaN"}"#,
            "predicate field 'max_fee_pct' must be a finite non-negative number",
        ),
        (
            r#"{"min_confidence":1}"#,
            "predicate field 'min_confidence' must be a string",
        ),
        (
            r#"{"min_confidence":"weak"}"#,
            "predicate field 'min_confidence' must be one of: exact, strong",
        ),
    ] {
        book.connection().execute("UPDATE swap_matching_rules SET predicate_json=?,kind='manual',policy='taxable' WHERE id='invalid'",[raw]).unwrap();
        let result = execute(
            book.connection(),
            &scope(),
            "transfers.rules.list",
            &dict::<&str>([]),
        )
        .unwrap();
        let row = array(&result)
            .iter()
            .find(|r| get(r, "id") == Some(&s("invalid")))
            .unwrap();
        assert_eq!(get(row, "enabled"), Some(&Value::Bool(false)), "{raw}");
        assert_eq!(get(row, "invalid_reason"), Some(&s(reason)), "{raw}");
        assert_eq!(get(row, "predicate"), Some(&dict::<&str>([])));
    }
}

#[test]
fn stored_allocations_override_reviewed_amounts() {
    let (_dir, book) = book();
    // Deliberately divergent imported historical terms: reads preserve the
    // allocation rather than recomputing it from an observation or source term.
    book.connection().execute_batch("DROP TRIGGER trg_custody_component_terms_immutable; UPDATE custody_component_economic_terms SET reviewed_source_amount_msat=reviewed_source_amount_msat+1 WHERE term_kind='transaction_pair'").unwrap();
    let result = execute(
        book.connection(),
        &scope(),
        "transfers.list",
        &dict::<&str>([]),
    )
    .unwrap();
    for row in array(&result) {
        assert_ne!(
            get(row, "out_amount"),
            get(get(row, "out").unwrap(), "amount_msat")
        );
    }
}

#[test]
fn malformed_stored_payout_value_remains_recoverable() {
    let (_dir, book) = book();
    book.connection()
        .execute_batch("DROP TRIGGER trg_custody_component_terms_immutable")
        .unwrap();
    for price in ["invalid", "NaN", "Infinity", "1e999", "1e-20"] {
        book.connection().execute("UPDATE custody_component_economic_terms SET payout_fiat_value_exact=? WHERE term_kind='direct_swap_payout'", [price]).unwrap();
        let result = execute(
            book.connection(),
            &scope(),
            "transfers.payouts.list",
            &dict::<&str>([]),
        )
        .unwrap();
        assert!(!array(&result).is_empty());
        for row in array(&result) {
            assert_eq!(get(row, "invalid"), Some(&Value::Bool(true)));
            assert_eq!(
                get(row, "validation_error"),
                Some(&s(
                    "Invalid payout_fiat_value; delete and recreate this payout"
                ))
            );
            assert_eq!(get(row, "payout_fiat_value"), Some(&Value::Null));
        }
    }
}

#[test]
fn historical_snapshot_quantities_follow_python_integer_coercion() {
    let (_dir, book) = book();
    let conn = book.connection();
    conn.execute_batch("DROP TRIGGER trg_custody_gap_reviews_immutable")
        .unwrap();
    conn.execute(
        "UPDATE custody_gap_reviews SET snapshot_json=? WHERE id='review3'",
        [r#"{"retained_msat":"１２３","residual_msat":1.9}"#],
    )
    .unwrap();
    let args = dict([("gap_id", s("gap")), ("limit", 1.into())]);
    let result = execute(conn, &scope(), "transfers.gaps.history", &args).unwrap();
    let row = &array(get(&result, "history").unwrap())[0];
    assert_eq!(get(row, "retained_msat"), Some(&123.into()));
    assert_eq!(get(row, "residual_msat"), Some(&1.into()));
    for (snapshot, message) in [
        ("[]", "'list' object has no attribute 'get'"),
        (
            r#"{"retained_msat":"bad"}"#,
            "invalid literal for int() with base 10: 'bad'",
        ),
    ] {
        conn.execute(
            "UPDATE custody_gap_reviews SET snapshot_json=? WHERE id='review3'",
            [snapshot],
        )
        .unwrap();
        let error = execute(conn, &scope(), "transfers.gaps.history", &args).unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(error.message, message);
    }
}
