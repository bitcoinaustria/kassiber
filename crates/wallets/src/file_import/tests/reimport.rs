//! Re-importing a newer export (docs/reference/imports.md#re-importing-a-newer-export):
//! the same file again and an overlapping newer export add only new rows;
//! rows that look like a stored row under another id or amount refuse the
//! import until the user chooses `skip` or `import`; values the stored row
//! keeps are reported; identical ID-less rows of a generic file stay
//! separate rows, compatibly with books that stored them merged.
use super::*;

const HEADER: &str = "date,txid,direction,asset,amount,fee,kind\n";

fn csv(rows: &[&str]) -> String {
    format!("{HEADER}{}\n", rows.join("\n"))
}

fn run(conn: &Connection, content: &str, choice: Option<ChangedRows>) -> Result<Py, AppError> {
    import_with(conn, content, Format::Csv, "Source", choice)
}

fn int(result: &Py, key: &str) -> i64 {
    match kassiber_envelope::get(result, key) {
        Some(Py::Int(n)) => n.to_string().parse().unwrap(),
        other => panic!("{key}: {other:?}"),
    }
}

fn detail(error: &AppError, key: &str) -> i64 {
    match error
        .details
        .as_ref()
        .and_then(|d| kassiber_envelope::get(d, key))
    {
        Some(Py::Int(n)) => n.to_string().parse().unwrap(),
        other => panic!("{key}: {other:?}"),
    }
}

fn dumped(result: &Py, key: &str) -> String {
    kassiber_pyjson::dumps(
        kassiber_envelope::get(result, key).unwrap_or(&Py::Null),
        &kassiber_pyjson::Options::new().sort_keys(true),
    )
    .unwrap()
}

fn rows(conn: &Connection) -> i64 {
    scalar(
        conn,
        "SELECT count(*) FROM transactions WHERE wallet_id='wallet'",
    )
}

#[test]
fn the_same_file_and_a_newer_overlapping_export_add_only_new_rows() {
    let conn = fixture();
    let first = csv(&[
        "2025-01-01,a,in,BTC,0.1,0,",
        "2025-01-02,b,out,BTC,0.05,0.0001,",
    ]);
    assert_eq!(int(&run(&conn, &first, None).unwrap(), "imported"), 2);
    let again = run(&conn, &first, None).unwrap();
    assert_eq!((int(&again, "imported"), int(&again, "unchanged")), (0, 2));
    // Nothing differs: no new keys in the result.
    for key in ["corrections_ignored", "changed_rows", "updated"] {
        assert_eq!(kassiber_envelope::get(&again, key), None, "{key}");
    }
    let newer = csv(&[
        "2025-01-01,a,in,BTC,0.1,0,",
        "2025-01-02,b,out,BTC,0.05,0.0001,",
        "2025-01-03,c,in,BTC,0.2,0,",
    ]);
    let result = run(&conn, &newer, None).unwrap();
    assert_eq!((int(&result, "imported"), int(&result, "skipped")), (1, 2));
    assert_eq!(rows(&conn), 3);
}

#[test]
fn an_id_column_that_appears_or_disappears_waits_for_the_users_choice() {
    for (before, after) in [("", "tx-"), ("tx-", "")] {
        let conn = fixture();
        let id = |prefix: &str, n: &str| {
            if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}{n}")
            }
        };
        let file = |prefix: &str, extra: bool| {
            let mut lines = vec![
                format!("2025-01-01,{},in,BTC,0.1,0,", id(prefix, "1")),
                format!("2025-01-02,{},out,BTC,0.05,0.0001,", id(prefix, "2")),
            ];
            if extra {
                lines.push(format!("2025-01-03,{},in,BTC,0.2,0,", id(prefix, "3")));
            }
            csv(&lines.iter().map(String::as_str).collect::<Vec<_>>())
        };
        run(&conn, &file(before, false), None).unwrap();
        let newer = file(after, true);
        let error = run(&conn, &newer, None).unwrap_err();
        assert_eq!(error.code, "import_changed_rows");
        assert_eq!(
            error.message,
            "This export identifies 2 rows differently from the earlier import (same date, direction, asset, amount and fee as a stored row under another id)"
        );
        assert!(error
            .hint
            .as_deref()
            .unwrap()
            .contains("--changed-rows skip"));
        assert!(error.hint.as_deref().unwrap().contains("imports rollback"));
        assert_eq!(
            (detail(&error, "reidentified"), detail(&error, "corrected")),
            (2, 0)
        );
        // Nothing was written.
        assert_eq!(rows(&conn), 2);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 1);
        // Skip: the stored rows stay, only the new one is imported.
        let skipped = run(&conn, &newer, Some(ChangedRows::Skip)).unwrap();
        assert_eq!(
            (int(&skipped, "imported"), int(&skipped, "skipped")),
            (1, 2)
        );
        assert_eq!(
            dumped(&skipped, "changed_rows"),
            r#"{"action": "skip", "corrected": 0, "reidentified": 2}"#
        );
        assert_eq!(rows(&conn), 3);
        // The answer is remembered: the same export again asks nothing.
        assert_eq!(int(&run(&conn, &newer, None).unwrap(), "imported"), 0);
        // Import: they are separate transactions.
        let imported = run(&conn, &newer, Some(ChangedRows::Import)).unwrap();
        assert_eq!(int(&imported, "imported"), 2);
        assert_eq!(rows(&conn), 5);
    }
}

#[test]
fn a_changed_date_format_behind_a_synthetic_id_waits_for_the_users_choice() {
    let conn = fixture();
    let file = |date: &str| {
        format!("Order ID,Type,Date,Amount EUR,Amount Crypto,Crypto,Rate EUR,Mining Fee Crypto,Total Fee EUR,Transaction\n,buy,{date},100,0.002,BTC,,0,0,\n")
    };
    let coinfinity = |content: &str, choice| {
        let path = std::env::temp_dir().join(format!(
            "kassiber-reimport-{}-{:?}.csv",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, content).unwrap();
        let result = execute(
            &conn,
            Request {
                profile_id: "p".into(),
                wallet: "Source".into(),
                file: path.to_str().unwrap().into(),
                format: Format::Coinfinity,
                mode: Some("full".into()),
                companion: None,
                timezone: None,
                changed_rows: choice,
            },
            None,
        );
        std::fs::remove_file(path).unwrap();
        result
    };
    conn.execute("UPDATE profiles SET fiat_currency='EUR' WHERE id='p'", [])
        .unwrap();
    assert_eq!(
        int(&coinfinity(&file("2025-01-02"), None).unwrap(), "imported"),
        1
    );
    // The same instant, written differently, names the row differently.
    let error = coinfinity(&file("2025-01-02T00:00:00Z"), None).unwrap_err();
    assert_eq!(
        (error.code.as_str(), detail(&error, "reidentified")),
        ("import_changed_rows", 1)
    );
    let skipped = coinfinity(&file("2025-01-02T00:00:00Z"), Some(ChangedRows::Skip)).unwrap();
    assert_eq!(int(&skipped, "imported"), 0);
    assert_eq!(rows(&conn), 1);
}

#[test]
fn a_provider_correction_under_the_same_id_is_never_a_second_row_silently() {
    let conn = fixture();
    run(
        &conn,
        &csv(&[
            "2025-01-01,a,in,BTC,0.1,0,",
            "2025-01-02,b,out,BTC,0.05,0.0001,",
        ]),
        None,
    )
    .unwrap();
    let corrected = csv(&[
        "2025-01-01,a,in,BTC,0.11,0,",
        "2025-01-02,b,out,BTC,0.05,0.0002,",
        "2025-01-03,c,in,BTC,0.2,0,",
    ]);
    let error = run(&conn, &corrected, None).unwrap_err();
    assert_eq!(
        error.message,
        "2 rows have the id of a stored row with another amount or fee; first 'a': amount 0.1 in the book, 0.11 in the file"
    );
    assert_eq!(
        (detail(&error, "reidentified"), detail(&error, "corrected")),
        (0, 2)
    );
    assert_eq!(rows(&conn), 2);
    // Skip keeps the stored values, imports the rest and names what it kept.
    let skipped = run(&conn, &corrected, Some(ChangedRows::Skip)).unwrap();
    assert_eq!(int(&skipped, "imported"), 1);
    let ignored = dumped(&skipped, "corrections_ignored");
    assert!(
        ignored
            .contains(r#""external_id": "a", "field": "amount", "file": "0.11", "stored": "0.1""#),
        "{ignored}"
    );
    assert!(
        ignored.contains(
            r#""external_id": "b", "field": "fee", "file": "0.0002", "stored": "0.0001""#
        ),
        "{ignored}"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT amount FROM transactions WHERE external_id='a'"
        ),
        10_000_000_000
    );
    // Import keeps both under the id (a second row the user asked for).
    let imported = run(&conn, &corrected, Some(ChangedRows::Import)).unwrap();
    assert_eq!(int(&imported, "imported"), 2);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id='a'"
        ),
        2
    );
}

#[test]
fn a_new_row_under_an_id_whose_stored_rows_all_match_is_imported() {
    // Two payments of one transaction: the stored one matches, the second is new.
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,tx,in,BTC,0.1,0,"]), None).unwrap();
    let result = run(
        &conn,
        &csv(&["2025-01-01,tx,in,BTC,0.1,0,", "2025-01-01,tx,in,BTC,0.2,0,"]),
        None,
    )
    .unwrap();
    assert_eq!(int(&result, "imported"), 1);
    // Within one file, no row counts as a correction of another.
    let conn = fixture();
    let result = run(
        &conn,
        &csv(&["2025-01-01,tx,in,BTC,0.1,0,", "2025-01-01,tx,in,BTC,0.2,0,"]),
        None,
    )
    .unwrap();
    assert_eq!(int(&result, "imported"), 2);
}

#[test]
fn values_the_stored_row_keeps_are_reported() {
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,a,in,BTC,0.1,0,income"]), None).unwrap();
    // A later export moves the date and relabels the row under the same id.
    let result = run(&conn, &csv(&["2025-01-02,a,in,BTC,0.1,0,interest"]), None).unwrap();
    assert_eq!(int(&result, "imported"), 0);
    let ignored = dumped(&result, "corrections_ignored");
    assert!(ignored.contains(r#""field": "occurred_at", "file": "2025-01-02T00:00:00Z", "stored": "2025-01-01T00:00:00Z""#), "{ignored}");
    assert!(
        ignored.contains(r#""field": "kind", "file": "interest", "stored": "income""#),
        "{ignored}"
    );
    // An exchange price relaxes the fee match: the stored fee stays, said so.
    let priced = |fee: &str| {
        format!("date,txid,direction,asset,amount,fee,fiat_value,pricing_source_kind\n2025-02-01,p,in,BTC,0.1,{fee},100,exchange_execution\n")
    };
    run(&conn, &priced("0"), None).unwrap();
    let result = run(&conn, &priced("0.0001"), None).unwrap();
    assert_eq!(int(&result, "imported"), 0);
    assert!(dumped(&result, "corrections_ignored")
        .contains(r#""field": "fee", "file": "0.0001", "stored": "0""#));
}

#[test]
fn enriched_rows_are_counted_for_every_format() {
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,a,in,BTC,0.1,0,"]), None).unwrap();
    let priced = "date,txid,direction,asset,amount,fee,fiat_value\n2025-01-01,a,in,BTC,0.1,0,100\n";
    let result = run(&conn, priced, None).unwrap();
    assert_eq!((int(&result, "imported"), int(&result, "updated")), (0, 1));
}

#[test]
fn identical_idless_rows_stay_separate_and_an_earlier_merged_import_gains_only_the_copies() {
    let conn = fixture();
    let three = csv(&[
        "2025-01-01,,in,BTC,0.1,0,",
        "2025-01-01,,in,BTC,0.1,0,",
        "2025-01-01,,in,BTC,0.1,0,",
    ]);
    // An earlier build stored such a file as one row: the first occurrence
    // keeps that row's identity.
    run(&conn, &csv(&["2025-01-01,,in,BTC,0.1,0,"]), None).unwrap();
    let result = run(&conn, &three, None).unwrap();
    assert_eq!(
        (int(&result, "imported"), int(&result, "unchanged")),
        (2, 1)
    );
    assert_eq!(rows(&conn), 3);
    // Again: nothing new, never a fourth copy.
    assert_eq!(int(&run(&conn, &three, None).unwrap(), "imported"), 0);
    assert_eq!(rows(&conn), 3);
    // Generic JSON and the generic ledger number them the same way.
    let conn = fixture();
    let json = r#"[{"date":"2025-01-01","amount":1},{"date":"2025-01-01","amount":1}]"#;
    assert_eq!(
        int(
            &import_with(&conn, json, Format::Json, "Source", None).unwrap(),
            "imported"
        ),
        2
    );
}

#[test]
fn every_identical_copy_of_a_corrected_row_gets_the_same_answer() {
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,a,in,BTC,0.1,0,"]), None).unwrap();
    let file = csv(&[
        "2025-01-01,a,in,BTC,0.2,0,",
        "2025-01-01,a,in,BTC,0.2,0,",
        "2025-01-02,c,in,BTC,0.3,0,",
    ]);
    let error = run(&conn, &file, None).unwrap_err();
    assert_eq!(detail(&error, "corrected"), 2);
    let skipped = run(&conn, &file, Some(ChangedRows::Skip)).unwrap();
    assert_eq!(int(&skipped, "imported"), 1);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id='a'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT amount FROM transactions WHERE external_id='a'"
        ),
        10_000_000_000
    );
    // The same file again: the remembered answer covers both copies.
    let again = run(&conn, &file, None).unwrap();
    assert_eq!(int(&again, "imported"), 0);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM transactions WHERE external_id='a'"
        ),
        1
    );
}

#[test]
fn a_kept_difference_does_not_ask_again_but_a_new_difference_does() {
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,a,in,BTC,0.1,0,"]), None).unwrap();
    let corrected = csv(&["2025-01-01,a,in,BTC,0.11,0,"]);
    run(&conn, &corrected, Some(ChangedRows::Skip)).unwrap();
    let config: String = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id='wallet'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(config.contains("\"changed_rows_kept\""), "{config}");
    // Next import or sync of the same difference: no question, the kept value is named.
    let again = run(&conn, &corrected, None).unwrap();
    assert_eq!(int(&again, "imported"), 0);
    assert!(dumped(&again, "corrections_ignored")
        .contains(r#""field": "amount", "file": "0.11", "stored": "0.1""#));
    assert_eq!(kassiber_envelope::get(&again, "changed_rows"), None);
    // A new difference asks.
    let newer = csv(&["2025-01-01,a,in,BTC,0.12,0,"]);
    assert_eq!(
        detail(&run(&conn, &newer, None).unwrap_err(), "corrected"),
        1
    );
}

/// A remembered `skip` answer is bound to the import run that recorded it:
/// once that run is rolled back (`imports rollback` deletes its rows and the
/// run), the same difference asks again instead of being skipped unseen.
#[test]
fn a_kept_difference_asks_again_once_its_import_run_is_rolled_back() {
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,a,in,BTC,0.1,0,"]), None).unwrap();
    // The answering import also stores a new row, so it is a run to roll back.
    let corrected = csv(&["2025-01-01,a,in,BTC,0.11,0,", "2025-01-03,c,in,BTC,0.2,0,"]);
    let answered = run(&conn, &corrected, Some(ChangedRows::Skip)).unwrap();
    let batch =
        kassiber_envelope::py_str(kassiber_envelope::get(&answered, "import_batch_id").unwrap());
    let roll_back = |batch: &str| {
        conn.execute("DELETE FROM transactions WHERE id IN (SELECT transaction_id FROM import_batch_transactions WHERE batch_id=?)", [batch]).unwrap();
        conn.execute("DELETE FROM import_batches WHERE id=?", [batch])
            .unwrap();
    };
    roll_back(&batch);
    assert_eq!(rows(&conn), 1);
    let error = run(&conn, &corrected, None).unwrap_err();
    assert_eq!(error.code, "import_changed_rows");
    assert_eq!(detail(&error, "corrected"), 1);
    // A new answer binds the new run and stands while it does.
    let again = run(&conn, &corrected, Some(ChangedRows::Skip)).unwrap();
    assert_eq!(int(&again, "imported"), 1);
    let kept = run(&conn, &corrected, None).unwrap();
    assert_eq!(int(&kept, "imported"), 0);
    let config: String = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id='wallet'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // The retired answer is gone; only the one bound to the live run stays.
    let live =
        kassiber_envelope::py_str(kassiber_envelope::get(&again, "import_batch_id").unwrap());
    assert!(
        config.contains(&format!("\"batch\": \"{live}\"")),
        "{config}"
    );
    assert!(!config.contains(&batch), "{config}");
}

/// The refusal's advice names the CLI steps of the run it refused: a file
/// import is imported again, an exchange sync synced again.
#[test]
fn the_refusal_names_the_steps_of_an_import_or_a_sync() {
    let conn = fixture();
    run(&conn, &csv(&["2025-01-01,a,in,BTC,0.1,0,"]), None).unwrap();
    let error = run(&conn, &csv(&["2025-01-01,a,in,BTC,0.11,0,"]), None).unwrap_err();
    assert_eq!(error.hint.as_deref(), Some(changed::HINT));
    assert!(changed::HINT.contains("import this file again"));
    assert!(changed::SYNC_HINT.contains("Run the sync again with --changed-rows skip"));
    assert!(changed::SYNC_HINT.contains(
        "roll back the earlier sync (kassiber imports list, then imports rollback) and sync again"
    ));
    assert!(!changed::SYNC_HINT.contains("file"));
}

/// A corrected swap imported as new rows (`--changed-rows import`) pairs
/// the legs this import stored, never the earlier rows that share their id,
/// direction and asset.
#[test]
fn a_corrected_swap_imported_as_new_rows_pairs_its_new_legs() {
    let conn = fixture();
    let swap = |amount: &str| {
        format!(
            "date,txid,direction,asset,amount,fee,kind,swap_ref\n2025-01-01,s-out,out,BTC,{amount},0,swap,S1\n2025-01-01,s-in,in,LBTC,{amount},0,swap,S1\n"
        )
    };
    let first = run(&conn, &swap("0.1"), None).unwrap();
    assert_eq!(
        kassiber_envelope::get(&first, "swap_pairs"),
        Some(&Py::from(1))
    );
    let corrected = run(&conn, &swap("0.2"), Some(ChangedRows::Import)).unwrap();
    assert_eq!(int(&corrected, "imported"), 2);
    assert_eq!(
        kassiber_envelope::get(&corrected, "swap_pairs"),
        Some(&Py::from(1))
    );
    let scope = kassiber_transfers::Scope {
        workspace_id: "ws",
        profile_id: "p",
    };
    let paired = kassiber_transfers::paired_transaction_ids(&conn, &scope).unwrap();
    let new_legs: Vec<String> = conn
        .prepare("SELECT id FROM transactions WHERE wallet_id='wallet' AND amount=20000000000")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(new_legs.len(), 2);
    for id in new_legs {
        assert!(paired.contains(&id), "{id} unpaired: {paired:?}");
    }
}
