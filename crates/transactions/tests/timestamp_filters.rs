use kassiber_envelope::get;
use kassiber_pyjson::Value;
use kassiber_store::{rusqlite::params, Book};
use kassiber_transactions::{
    listing::{list, Options},
    metadata::{execute, Command, HistoryOptions, RecordOptions},
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
    book.connection().execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');
        INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025');
        INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('wal','w','p','Cold','address','2025');").unwrap();
    for (id, stamp) in [
        ("before", "2025-02-01T00:00:00Z"),
        ("after", "2025-02-01T00:00:01Z"),
        ("later", "2025-02-01T00:00:02Z"),
    ] {
        book.connection().execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at) VALUES(?,'w','p','wal',?,'BTC','inbound',1,0,?,?)", params![id,id,stamp,stamp]).unwrap();
        book.connection().execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,source,changed_at) VALUES(?,'w','p',?,'wal','cli',?)", params![id,id,stamp]).unwrap();
    }
    (tmp, book)
}

fn ids(page: &Value, family: &str, key: &str) -> Vec<String> {
    let Some(Value::List(rows)) = get(page, family) else {
        panic!("missing {family}")
    };
    rows.iter()
        .map(|row| {
            let Some(Value::Str(value)) = get(row, key) else {
                panic!("missing {key}")
            };
            value.as_str().unwrap().to_owned()
        })
        .collect()
}

const BOUNDS: &[&str] = &[
    "2025-02-01T00:00:00.5Z",
    "2025-02-01T00:00:00.000001Z",
    "2025-02-01T00:00:00.999999Z",
    "2025-02-01T01:00:00.5+01:00",
];

#[test]
fn transaction_fractional_bounds_compare_instants_before_counting_and_pagination() {
    let (_tmp, book) = book();
    for bound in BOUNDS {
        let mut options = Options {
            start: Some((*bound).into()),
            limit: Some(1),
            ..Default::default()
        };
        let first = list(book.connection(), &scope(), &options).unwrap();
        assert_eq!(ids(&first, "records", "id"), ["later"]);
        assert_eq!(get(&first, "count"), Some(&Value::Int(2.into())));
        let Some(Value::Str(cursor)) = get(&first, "next_cursor") else {
            panic!("missing next cursor")
        };
        options.cursor = Some(cursor.as_str().unwrap().into());
        let last = list(book.connection(), &scope(), &options).unwrap();
        assert_eq!(ids(&last, "records", "id"), ["after"]);
        assert_eq!(get(&last, "count"), Some(&Value::Int(2.into())));
        assert_eq!(get(&last, "has_more"), Some(&Value::Bool(false)));
        let end = list(
            book.connection(),
            &scope(),
            &Options {
                end: Some((*bound).into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ids(&end, "records", "id"), ["before"]);
    }
}

#[test]
fn metadata_record_and_history_bounds_keep_exact_microseconds() {
    let (_tmp, book) = book();
    for bound in BOUNDS {
        for start in [true, false] {
            let (lower, upper) = if start {
                (Some((*bound).into()), None)
            } else {
                (None, Some((*bound).into()))
            };
            let record = execute(
                book.connection(),
                &scope(),
                &Command::Records(RecordOptions {
                    start: lower.clone(),
                    end: upper.clone(),
                    ..Default::default()
                }),
            )
            .unwrap();
            let history = execute(
                book.connection(),
                &scope(),
                &Command::History(HistoryOptions {
                    start: lower,
                    end: upper,
                    ..Default::default()
                }),
            )
            .unwrap();
            let expected = if start {
                vec!["later", "after"]
            } else {
                vec!["before"]
            };
            assert_eq!(ids(&record, "records", "transaction_id"), expected);
            assert_eq!(ids(&history, "events", "id"), expected);
        }
    }
}

#[test]
fn persisted_time_bounds_normalize_offsets_and_exclude_invalid_instants() {
    let (_tmp, book) = book();
    let conn = book.connection();
    conn.execute(
        "UPDATE transactions SET occurred_at='2025-02-01T01:00:00+01:00' WHERE id='before'",
        [],
    )
    .unwrap();
    for (id, stamp) in [
        ("unknown", "bad"),
        ("overflow", "0001-01-01T00:00:00+01:00"),
    ] {
        conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at) VALUES(?,'w','p','wal',?,'BTC','inbound',1,0,?,'2025')", params![id,id,stamp]).unwrap();
    }
    let options = Options {
        start: Some("2025-02-01T00:00:00.000000Z".into()),
        end: Some("2025-02-01T00:00:00.000001Z".into()),
        ..Default::default()
    };
    assert_eq!(
        ids(&list(conn, &scope(), &options).unwrap(), "records", "id"),
        ["before"]
    );
    // CPython replaces terminal Z before parsing, including this date spelling.
    conn.execute(
        "UPDATE transactions SET occurred_at='2025-02-01Z' WHERE id='before'",
        [],
    )
    .unwrap();
    assert_eq!(
        ids(&list(conn, &scope(), &options).unwrap(), "records", "id"),
        ["before"]
    );
    // Keep a reading statement active while a second bounded service read
    // reuses the registered function on the same connection.
    let mut statement = conn
        .prepare("SELECT kassiber_timestamp_us(occurred_at) FROM transactions")
        .unwrap();
    let mut active = statement.query([]).unwrap();
    assert!(active.next().unwrap().is_some());
    assert_eq!(
        ids(&list(conn, &scope(), &options).unwrap(), "records", "id"),
        ["before"]
    );
    let end_only = Options {
        end: options.end,
        ..Default::default()
    };
    assert_eq!(
        ids(&list(conn, &scope(), &end_only).unwrap(), "records", "id"),
        ["before"]
    );
}
