use kassiber_envelope::{get, s};
use kassiber_pyjson::{Key, Value};
use kassiber_store::Book;
use kassiber_transactions::{desktop, Scope};

fn vectors(group: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(include_str!("data/desktop.sql"))
        .unwrap();
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("data/desktop-vectors.json")).unwrap()
    else {
        panic!("cases")
    };
    let utc_year: String = conn
        .query_row("SELECT strftime('%Y','now')", [], |row| row.get(0))
        .unwrap();
    let mut count = 0;
    for case in cases {
        if get(&case, "group") != Some(&s(group)) {
            continue;
        }
        count += 1;
        conn.execute_batch("SAVEPOINT vector").unwrap();
        let string = |key| match get(&case, key).unwrap() {
            Value::Str(t) => t.as_str().unwrap(),
            _ => panic!("string"),
        };
        conn.execute_batch(string("setup")).unwrap();
        let actual = execute(
            conn,
            &Scope {
                workspace_id: "w",
                profile_id: "p",
            },
            string("kind"),
            get(&case, "args"),
        );
        let actual = match actual {
            Ok(value) => value,
            Err(error) => error.envelope(None),
        };
        let mut expected = get(&case, "expected").unwrap().clone();
        if let Value::Dict(pairs) = &mut expected {
            if let Some((_, value)) = pairs.iter_mut().find(|(key, _)| *key == Key::from("year")) {
                if *value == s("$utc_year") {
                    *value = Value::Int(utc_year.parse::<i64>().unwrap().into());
                }
            }
        }
        assert_eq!(
            actual,
            expected,
            "{group} {} {:?}",
            string("kind"),
            get(&case, "args")
        );
        conn.execute_batch("ROLLBACK TO vector; RELEASE vector")
            .unwrap();
    }
    assert!(count > 0, "missing vector group {group}");
}

#[test]
fn desktop_filters_preserve_wallet_population_query_ids_and_exact_instants() {
    vectors("filters");
}
#[test]
fn desktop_flow_payment_network_fee_quick_and_status_filters_match_python() {
    vectors("flows");
}
#[test]
fn desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors() {
    vectors("pages");
}
#[test]
fn desktop_extremes_apply_filters_before_both_amount_limits() {
    vectors("extremes");
}
#[test]
fn desktop_search_requires_query_and_clamps_its_own_limit() {
    vectors("search");
}
#[test]
fn desktop_refusals_keep_typed_validation_and_cursor_errors() {
    vectors("refusals");
}
#[test]
fn desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs() {
    vectors("pairs");
}
#[test]
fn desktop_freshness_requires_processed_count_version_and_timestamp() {
    vectors("freshness");
}

#[test]
fn desktop_moves_use_stored_decisions_and_journal_fallback_without_inventing_fees() {
    vectors("moves");
}

#[test]
fn desktop_resolve_preserves_alias_precedence_focused_principal_and_queue_status() {
    vectors("resolve");
}

#[test]
fn desktop_relative_periods_use_midnight_and_alias_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(include_str!("data/desktop.sql"))
        .unwrap();
    for (period, days) in [
        ("30-days", 29),
        ("3months", 92),
        ("6months", 183),
        ("1year", 365),
        ("4years", 365 * 4),
        ("4y", 365 * 4),
        ("5years", 365 * 5),
        ("10years", 365 * 10),
        ("15years", 365 * 15),
    ] {
        let page = execute(
            conn,
            &Scope {
                workspace_id: "w",
                profile_id: "p",
            },
            "ui.transactions.list",
            Some(&kassiber_envelope::dict([("period", s(period))])),
        )
        .unwrap();
        let expected: String = conn
            .query_row(
                "SELECT strftime('%Y-%m-%dT00:00:00Z','now',?)",
                [format!("-{days} days")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            get(get(&page, "filters").unwrap(), "since"),
            Some(&s(expected)),
            "{period}"
        );
    }
    let page = execute(
        conn,
        &Scope {
            workspace_id: "w",
            profile_id: "p",
        },
        "ui.transactions.list",
        Some(&kassiber_envelope::dict([
            ("since", s("2025-01-01")),
            ("start", Value::List(vec![])),
            ("until", s("2025-02-01")),
            ("end", Value::Bool(false)),
        ])),
    )
    .unwrap();
    assert_eq!(
        get(get(&page, "filters").unwrap(), "since"),
        Some(&s("2025-01-01T00:00:00Z"))
    );
}

#[test]
fn desktop_missing_context_short_circuits_list_search_but_extremes_validates() {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    let scope = Scope {
        workspace_id: "",
        profile_id: "",
    };
    for kind in ["ui.transactions.list", "ui.transactions.search"] {
        let page = execute(book.connection(), &scope, kind, Some(&Value::List(vec![]))).unwrap();
        assert_eq!(get(&page, "txs"), Some(&Value::List(vec![])));
        assert_eq!(
            get(get(&page, "filters").unwrap(), "limit"),
            Some(&Value::Int(0.into()))
        );
    }
    let resolved = execute(
        book.connection(),
        &scope,
        "ui.transactions.resolve",
        Some(&Value::List(vec![])),
    )
    .unwrap();
    assert_eq!(get(&resolved, "transaction"), Some(&Value::Null));
    assert_eq!(get(&resolved, "query"), Some(&s("")));
    assert_eq!(
        execute(
            book.connection(),
            &scope,
            "ui.transactions.extremes",
            Some(&Value::List(vec![]))
        )
        .unwrap_err()
        .code,
        "validation"
    );
}

fn execute(
    conn: &kassiber_store::rusqlite::Connection,
    scope: &Scope<'_>,
    kind: &str,
    args: Option<&Value>,
) -> Result<Value, kassiber_envelope::AppError> {
    desktop::execute(
        conn,
        scope,
        desktop::Projection::from_kind(kind).unwrap(),
        args,
        &|_, _| {
            Ok(kassiber_envelope::dict([
                ("reviewPairId", Value::Null),
                ("pairFingerprint", Value::Null),
            ]))
        },
    )
}
