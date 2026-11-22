use super::*;
use kassiber_envelope::get;
use serde_json::Value as Json;

fn book() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let creation: Json =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
         INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01'),('q','ws','Other','2025-01-01');
         INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
           ('w','ws','p','Exchange','kraken','{}','2025-01-01'),
           ('n','ws','p','Node','coreln','{}','2025-01-01'),
           ('x','ws','q','Elsewhere','custom','{}','2025-01-01');",
    )
    .unwrap();
    conn
}

/// One active transaction and its wallet posting (signed msat).
fn movement(conn: &Connection, id: &str, at: &str, msat: i64) {
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at)
         VALUES(?1,'ws','p','w',?1,?2,?3,'BTC',?4,'2025-01-01')",
        params![id, at, if msat >= 0 { "inbound" } else { "outbound" }, msat.abs()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal_quantity_postings(posting_id,workspace_id,profile_id,transaction_id,occurred_at,asset,location_kind,location_id,amount_msat,state,created_at)
         VALUES(?1,'ws','p',?1,?2,'BTC','wallet','w',?3,'booked','2025-01-01')",
        params![id, at, msat],
    )
    .unwrap();
}

/// Mark the journals as built from the current inputs.
fn processed(conn: &Connection) {
    conn.execute_batch(
        "UPDATE profiles SET last_processed_at='2025-06-01T00:00:00Z',
           last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),
           last_processed_input_version=journal_input_version WHERE id='p'",
    )
    .unwrap();
    conn.execute(
        "UPDATE profiles SET journal_semantics_version=? WHERE id='p'",
        [kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION],
    )
    .unwrap();
}

fn checkpoint(at: &str, msat: i64) -> Checkpoint {
    Checkpoint {
        id: None,
        wallet_id: "w".into(),
        asset: "BTC".into(),
        as_of: at.into(),
        amount_msat: msat,
        origin: "manual".into(),
        scope: None,
        origin_ref: None,
        attachment_id: None,
        note: None,
        created_at: None,
    }
}

fn compared(conn: &Connection, at: &str, msat: i64) -> Comparison {
    let journals = journal_status(conn, "p").unwrap();
    compare(conn, "p", &checkpoint(at, msat), journals).unwrap()
}

#[test]
fn btc_text_parses_to_exact_msat_and_refuses_everything_else() {
    for (text, msat) in [
        ("0", 0),
        ("1", 100_000_000_000),
        ("0.00000000001", 1),
        ("21000000", 2_100_000_000_000_000_000),
        (".5", 50_000_000_000),
        ("0.12345678901", 12_345_678_901),
        ("7.", 700_000_000_000),
    ] {
        assert_eq!(parse_btc_msat(text), Some(msat), "{text}");
    }
    for text in [
        "",
        ".",
        "-1",
        "+1",
        "1e3",
        "0.000000000001",
        " 1",
        "1,5",
        "NaN",
        "１",
        "9999999999999999",
    ] {
        assert_eq!(parse_btc_msat(text), None, "{text}");
    }
    assert_eq!(format_btc(1), "0.00000000001");
    assert_eq!(format_btc(100_000_000_000), "1.00000000");
    assert_eq!(format_btc(-12_300_000_000), "-0.12300000");
}

#[test]
fn dates_are_end_of_utc_day_and_date_times_need_an_offset() {
    assert_eq!(
        iso(parse_as_of("2024-12-31").unwrap()).unwrap(),
        "2024-12-31T23:59:59.999999Z"
    );
    assert_eq!(
        iso(parse_as_of("2024-12-31T10:00:00+01:00").unwrap()).unwrap(),
        "2024-12-31T09:00:00Z"
    );
    assert_eq!(
        parse_as_of("2024-12-31T10:00:00").unwrap_err().code,
        "validation"
    );
    assert_eq!(parse_as_of("yesterday").unwrap_err().code, "validation");
}

#[test]
fn comparison_is_exact_in_msat() {
    let conn = book();
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100_000_000_001);
    movement(&conn, "b", "2025-02-01T00:00:00Z", -30_000_000_000);
    processed(&conn);
    let before = compared(&conn, "2025-01-15T00:00:00Z", 100_000_000_001);
    assert_eq!(
        (before.status, before.book_msat),
        ("matches", Some(100_000_000_001))
    );
    // One msat off is a difference, signed reported minus book.
    let off = compared(&conn, "2025-01-15T00:00:00Z", 100_000_000_000);
    assert_eq!(off.status, "differs");
    assert_eq!(off.difference_msat, Some(-1));
    let after = compared(&conn, "2025-03-01T00:00:00Z", 80_000_000_001);
    assert_eq!(after.status, "differs");
    assert_eq!(after.book_msat, Some(70_000_000_001));
    assert_eq!(after.difference_msat, Some(10_000_000_000));
    assert!(after.message.contains("+0.10000000"), "{}", after.message);
    // A posting at exactly the checkpoint's time counts.
    let exact = compared(&conn, "2025-02-01T00:00:00Z", 70_000_000_001);
    assert_eq!(exact.status, "matches");
}

#[test]
fn stale_journals_and_uncovered_times_are_unknown() {
    let conn = book();
    // No history at all.
    assert_eq!(
        compared(&conn, "2025-01-15T00:00:00Z", 5).reason,
        "no_history"
    );
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100);
    // Journals never processed.
    let stale = compared(&conn, "2025-01-15T00:00:00Z", 100);
    assert_eq!((stale.status, stale.reason), ("unknown", "journals_stale"));
    processed(&conn);
    assert_eq!(
        compared(&conn, "2025-01-15T00:00:00Z", 100).status,
        "matches"
    );
    // A new input after processing makes them stale again.
    conn.execute_batch(
        "UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'",
    )
    .unwrap();
    assert_eq!(
        compared(&conn, "2025-01-15T00:00:00Z", 100).reason,
        "journals_stale"
    );
    processed(&conn);
    let early = compared(&conn, "2024-12-31T23:59:59Z", 0);
    assert_eq!((early.status, early.reason), ("unknown", "before_coverage"));
}

#[test]
fn quarantined_rows_are_named_in_a_difference() {
    let conn = book();
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100);
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,created_at)
         VALUES('q1','ws','p','w','q1','2025-01-02T00:00:00Z','inbound','BTC',50,'2025-01-01')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('q1','ws','p','missing_price','x')",
        [],
    )
    .unwrap();
    processed(&conn);
    let result = compared(&conn, "2025-01-03T00:00:00Z", 150);
    assert_eq!(result.status, "differs");
    assert_eq!(result.notes, vec!["quarantined_transactions"]);
    assert!(
        result.message.contains("1 quarantined"),
        "{}",
        result.message
    );
}

#[test]
fn records_deduplicate_validate_and_stay_in_their_profile() {
    let conn = book();
    let new = New {
        wallet_id: "w".into(),
        asset: "BTC".into(),
        as_of: "2025-01-01T01:00:00+01:00".into(),
        amount_msat: 5,
        origin: "api",
        scope: Some("kraken_spot".into()),
        origin_ref: Some("kraken:api:main".into()),
        attachment_id: None,
        note: None,
    };
    let (id, created) = record(&conn, "p", &new).unwrap();
    assert!(created);
    let (again, created) = record(&conn, "p", &new).unwrap();
    assert_eq!((again.as_str(), created), (id.as_str(), false));
    let stored = stored(&conn, "p", None).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].as_of, "2025-01-01T00:00:00Z");
    // Another profile's wallet, a negative amount, an altcoin and a
    // derived origin are all refused.
    let other = New {
        wallet_id: "x".into(),
        ..new.clone()
    };
    assert_eq!(record(&conn, "p", &other).unwrap_err().code, "not_found");
    let negative = New {
        amount_msat: -1,
        ..new.clone()
    };
    assert_eq!(
        record(&conn, "p", &negative).unwrap_err().code,
        "validation"
    );
    let altcoin = New {
        asset: "ETH".into(),
        ..new.clone()
    };
    assert_eq!(record(&conn, "p", &altcoin).unwrap_err().code, "validation");
    let derived = New {
        origin: "chain",
        ..new.clone()
    };
    assert!(record(&conn, "p", &derived).is_err());
    // The table refuses a fractional amount even from raw SQL.
    assert!(conn
        .execute_batch("INSERT INTO balance_checkpoints(id,workspace_id,profile_id,wallet_id,asset,as_of,amount_msat,origin,created_at) VALUES('r','ws','p','w','BTC','2025-01-01T00:00:00Z',1.5,'manual','x')")
        .is_err());
    // Removing works on stored rows of this profile only.
    assert_eq!(remove(&conn, "q", &id).unwrap_err().code, "not_found");
    remove(&conn, "p", &id).unwrap();
    assert!(super::stored(&conn, "p", None).unwrap().is_empty());
}

#[test]
fn node_checkpoints_read_the_last_successful_sync_only() {
    let conn = book();
    assert_eq!(node_checkpoint(&conn, "p", "n").unwrap(), None);
    conn.execute_batch(
        "INSERT INTO lightning_node_syncs(id,workspace_id,profile_id,wallet_id,backend_name,started_at,completed_at,status,created_at) VALUES
           ('s1','ws','p','n','cln','2025-01-01T00:00:00Z','2025-01-01T00:01:00Z','synced','x'),
           ('s2','ws','p','n','cln','2025-02-01T00:00:00Z','2025-02-01T00:01:00Z','error','x'),
           ('s3','ws','p','n','cln','2025-03-01T00:00:00Z','2025-03-01T00:01:00Z','partial','x');
         INSERT INTO lightning_node_records(id,workspace_id,profile_id,wallet_id,backend_name,record_type,external_id,occurred_at,amount_msat,sync_id,first_seen_at,updated_at) VALUES
           ('r1','ws','p','n','cln','balance_snapshot','wallet','2025-01-01T00:00:00Z',1000,'s1','x','x'),
           ('r2','ws','p','n','cln','balance_snapshot','chan','2025-01-01T00:00:00Z',234,'s1','x','x'),
           ('r3','ws','p','n','cln','invoice','i','2025-01-01T00:00:00Z',99,'s1','x','x');",
    )
    .unwrap();
    let node = node_checkpoint(&conn, "p", "n").unwrap().unwrap();
    assert_eq!(node.amount_msat, 1234);
    assert_eq!(node.as_of, "2025-01-01T00:01:00Z");
    assert_eq!(node.origin, "node");
    assert_eq!(node.id, None);
}

#[test]
fn reports_show_the_latest_per_origin_and_the_worst_state() {
    let conn = book();
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100);
    processed(&conn);
    for (at, msat) in [("2025-01-02T00:00:00Z", 100), ("2025-01-03T00:00:00Z", 90)] {
        record(
            &conn,
            "p",
            &New {
                wallet_id: "w".into(),
                asset: "BTC".into(),
                as_of: at.into(),
                amount_msat: msat,
                origin: "manual",
                scope: None,
                origin_ref: None,
                attachment_id: None,
                note: None,
            },
        )
        .unwrap();
    }
    let derived = vec![Checkpoint {
        origin: "chain".into(),
        as_of: "2025-01-04T00:00:00Z".into(),
        amount_msat: 100,
        ..checkpoint("x", 0)
    }];
    let latest = report(&conn, "p", None, derived.clone(), false).unwrap();
    let Some(Value::List(rows)) = get(&latest, "checkpoints") else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    let Some(Value::List(wallets)) = get(&latest, "wallets") else {
        panic!()
    };
    assert_eq!(wallets.len(), 1);
    assert_eq!(get(&wallets[0], "status"), Some(&s("differs")));
    assert_eq!(
        get(&latest, "counts"),
        Some(&dict([
            ("matches", Value::from(1)),
            ("differs", Value::from(1)),
            ("unknown", Value::from(0))
        ]))
    );
    let history = report(&conn, "p", Some("w"), derived, true).unwrap();
    let Some(Value::List(rows)) = get(&history, "checkpoints") else {
        panic!()
    };
    assert_eq!(rows.len(), 3);
    assert_eq!(
        report(&conn, "p", Some("x"), vec![], false)
            .unwrap_err()
            .code,
        "not_found"
    );
}

/// Two balances for one wallet, origin, asset and time recorded within the
/// same second: the later-recorded one is the latest, whatever the random ids.
#[test]
fn a_tie_on_time_and_creation_reports_the_later_recorded_balance() {
    let conn = book();
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100);
    processed(&conn);
    for msat in [90, 100] {
        record(
            &conn,
            "p",
            &New {
                wallet_id: "w".into(),
                asset: "BTC".into(),
                as_of: "2025-01-02T00:00:00Z".into(),
                amount_msat: msat,
                origin: "manual",
                scope: None,
                origin_ref: None,
                attachment_id: None,
                note: None,
            },
        )
        .unwrap();
    }
    // Same creation second, and ids that sort the older row last.
    conn.execute_batch(
        "UPDATE balance_checkpoints SET created_at='2025-06-01T00:00:00Z';
         UPDATE balance_checkpoints SET id='zz-older' WHERE amount_msat=90;
         UPDATE balance_checkpoints SET id='aa-newer' WHERE amount_msat=100;",
    )
    .unwrap();
    let latest = report(&conn, "p", None, vec![], false).unwrap();
    let Some(Value::List(rows)) = get(&latest, "checkpoints") else {
        panic!()
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(get(&rows[0], "id"), Some(&s("aa-newer")));
    let Some(Value::List(wallets)) = get(&latest, "wallets") else {
        panic!()
    };
    assert_eq!(get(&wallets[0], "status"), Some(&s("matches")));
    // History lists both, oldest recorded first.
    let history = super::stored(&conn, "p", None).unwrap();
    let ids: Vec<_> = history.iter().filter_map(|c| c.id.as_deref()).collect();
    assert_eq!(ids, ["zz-older", "aa-newer"]);
}

/// A date covers its whole UTC day: every instant before the next midnight,
/// fractions of the last second included, and nothing from the next day.
#[test]
fn a_date_includes_fractional_seconds_up_to_the_next_utc_midnight() {
    let conn = book();
    movement(&conn, "a", "2025-01-31T00:00:00Z", 100);
    movement(&conn, "b", "2025-01-31T23:59:59.5Z", 20);
    movement(&conn, "c", "2025-01-31T23:59:59.999999Z", 3);
    movement(&conn, "d", "2025-02-01T00:00:00Z", 4000);
    movement(&conn, "e", "2025-02-01T00:00:00.000001Z", 50000);
    processed(&conn);
    let end = iso(parse_as_of("2025-01-31").unwrap()).unwrap();
    let day = compared(&conn, &end, 123);
    assert_eq!(
        (day.status, day.book_msat),
        ("matches", Some(123)),
        "{day:?}"
    );
    // The next day starts at midnight exactly.
    let next = iso(parse_as_of("2025-02-01").unwrap()).unwrap();
    assert_eq!(compared(&conn, &next, 54_123).status, "matches");
    // A date-time with an offset stays an exact, inclusive instant.
    let instant = iso(parse_as_of("2025-02-01T01:00:00+01:00").unwrap()).unwrap();
    assert_eq!(compared(&conn, &instant, 4_123).status, "matches");
}

/// A chain checkpoint is as of its sync's refresh; a transaction of the
/// same sync dated after it (a block time ahead of the clock) makes the
/// comparison unknown until the next sync, not a missing history.
#[test]
fn a_chain_checkpoint_with_later_dated_transactions_is_unknown() {
    let conn = book();
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100);
    movement(&conn, "b", "2025-01-02T00:00:05Z", 50);
    processed(&conn);
    let chain = Checkpoint {
        origin: "chain".into(),
        ..checkpoint("2025-01-02T00:00:00Z", 150)
    };
    let journals = journal_status(&conn, "p").unwrap();
    let result = compare(&conn, "p", &chain, journals).unwrap();
    assert_eq!(
        (result.status, result.reason),
        ("unknown", "after_last_sync")
    );
    // The same figure from any other origin is a real difference.
    let manual = compare(
        &conn,
        "p",
        &checkpoint("2025-01-02T00:00:00Z", 150),
        journals,
    )
    .unwrap();
    assert_eq!(manual.status, "differs");
    // A matching chain figure still matches.
    let matching = Checkpoint {
        origin: "chain".into(),
        ..checkpoint("2025-01-02T00:00:00Z", 100)
    };
    assert_eq!(
        compare(&conn, "p", &matching, journals).unwrap().status,
        "matches"
    );
}

/// A later-dated transaction defers a chain comparison only while it could
/// explain the gap; otherwise the difference shows, with the same note.
#[test]
fn later_postings_defer_only_the_gap_they_can_explain() {
    let conn = book();
    movement(&conn, "a", "2025-01-01T00:00:00Z", 100);
    movement(&conn, "b", "2025-01-02T00:00:05Z", 50);
    processed(&conn);
    let journals = journal_status(&conn, "p").unwrap();
    let chain = |msat| Checkpoint {
        origin: "chain".into(),
        ..checkpoint("2025-01-02T00:00:00Z", msat)
    };
    // +30 or +50: within what the later +50 could add.
    for msat in [130, 150] {
        let deferred = compare(&conn, "p", &chain(msat), journals).unwrap();
        assert_eq!(
            (deferred.status, deferred.reason),
            ("unknown", "after_last_sync"),
            "{msat}"
        );
        assert!(deferred.notes.contains(&"after_last_sync"));
    }
    // +1000 or -40: the later postings cannot explain it.
    for msat in [1100, 60] {
        let shown = compare(&conn, "p", &chain(msat), journals).unwrap();
        assert_eq!(shown.status, "differs", "{msat}");
        assert!(shown.notes.contains(&"after_last_sync"), "{msat}");
        assert!(
            shown.message.contains("until the next sync"),
            "{}",
            shown.message
        );
    }
}
