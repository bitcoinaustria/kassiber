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

/// A reviewed same-asset move of 600,000 sat whose send paid a 420 sat
/// network fee, alone in the book so every total is the pair's.
fn reviewed_move(conn: &kassiber_store::rusqlite::Connection) {
    conn.execute_batch(include_str!("data/desktop.sql"))
        .unwrap();
    conn.execute_batch(
        "DELETE FROM transaction_tags; DELETE FROM journal_quarantines;
         DELETE FROM transactions WHERE id NOT IN ('g','b');
         UPDATE transactions SET amount=600000000,fee=420000,fiat_rate=2 WHERE id='g';
         UPDATE transactions SET amount=600000000,fee=0 WHERE id='b';
         INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,occurred_at,created_at) VALUES('bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','w','p','g','b','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,600000000,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0,600000000,'main','main','onchain','onchain','BTC','BTC','internal_reviewed','eligible','test','2025','2025');
         UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=2,journal_input_version=2,last_processed_input_version=2 WHERE id='p';
         UPDATE profiles SET journal_semantics_version=10 WHERE id='p';",
    )
    .unwrap();
}

#[test]
fn dashboard_transfer_volume_counts_the_amount_a_pair_moved() {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    let conn = book.connection();
    reviewed_move(conn);
    let display = |key: &str| Ok(kassiber_pytext::zone::Zone::utc(key));
    let book_zone = |_: Option<&str>| {
        Ok::<_, kassiber_envelope::AppError>(kassiber_pytext::zone::Zone::utc("UTC"))
    };
    let snapshot = desktop::dashboard(
        conn,
        &Scope {
            workspace_id: "w",
            profile_id: "p",
        },
        &kassiber_envelope::dict([("period", s("all"))]),
        &[],
        &desktop::Zones {
            display: &display,
            book: &book_zone,
        },
    )
    .unwrap();
    let summary = get(&snapshot, "summary").unwrap();
    let transfers = get(summary, "transfers").unwrap();
    assert_eq!(get(transfers, "count"), Some(&Value::Int(1.into())));
    // 600,000 sat moved, priced at the pair's display rate (the out leg's 2).
    assert_eq!(get(transfers, "btc"), Some(&Value::Float(0.006)));
    assert_eq!(get(transfers, "eur"), Some(&Value::Float(0.012)));
    for segment in ["incoming", "outgoing", "swaps"] {
        let stats = get(summary, segment).unwrap();
        assert_eq!(
            get(stats, "count"),
            Some(&Value::Int(0.into())),
            "{segment}"
        );
        assert_eq!(get(stats, "btc"), Some(&Value::Float(0.0)), "{segment}");
    }
    let Some(Value::List(buckets)) = get(&snapshot, "buckets") else {
        panic!("buckets")
    };
    let charted: f64 = buckets
        .iter()
        .map(|bucket| {
            match get(
                get(get(bucket, "stats").unwrap(), "transfers").unwrap(),
                "btc",
            ) {
                Some(Value::Float(n)) => *n,
                other => panic!("btc {other:?}"),
            }
        })
        .sum();
    assert_eq!(
        charted, 0.006,
        "the chart's transfer bars match the summary"
    );
}
