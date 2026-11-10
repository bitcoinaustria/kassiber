//! `core/transaction_lookup.py` vectors: every reference resolves, or
//! refuses, exactly as CPython does (`scripts/gen_lookup_vectors.py`).
use kassiber_envelope::{dict, get, py_repr, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::Book;
use kassiber_transactions::lookup::{find_transaction, resolve_transaction};

fn vectors() -> Value {
    kassiber_pytext::loads(include_str!("data/lookup-vectors.json")).unwrap()
}

fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(items) => items,
        _ => &[],
    }
}

fn book(vectors: &Value) -> (tempfile::TempDir, Book) {
    let tmp = tempfile::tempdir().unwrap();
    let book = Book::create(tmp.path(), None).unwrap();
    for statement in list(get(vectors, "sql").unwrap()) {
        let Value::Str(sql) = statement else { panic!() };
        book.connection()
            .execute(sql.as_str().unwrap(), [])
            .unwrap();
    }
    (tmp, book)
}

fn outcome(result: Result<Option<Value>, AppError>) -> Value {
    match result {
        Ok(row) => dict([(
            "id",
            row.map_or(Value::Null, |row| get(&row, "id").cloned().unwrap()),
        )]),
        Err(error) => dict([(
            "error",
            dict([
                ("code", s(error.code.as_str())),
                ("message", s(error.message.as_str())),
                ("hint", error.hint.as_deref().map_or(Value::Null, s)),
                ("details", error.details.unwrap_or(Value::Null)),
            ]),
        )]),
    }
}

fn check(mode: &str, run: impl Fn(&Book, &Value, Option<&str>) -> Result<Option<Value>, AppError>) {
    let vectors = vectors();
    let (_tmp, book) = book(&vectors);
    let mut compared = 0;
    for case in list(get(&vectors, "cases").unwrap()) {
        let reference = get(case, "reference").unwrap();
        for (key, direction) in [
            ("any", None),
            ("outbound", Some("outbound")),
            ("inbound", Some("inbound")),
        ] {
            let Some(expected) = get(case, &format!("{mode}_{key}")) else {
                continue;
            };
            let actual = outcome(run(&book, reference, direction));
            assert_eq!(&actual, expected, "{mode}_{key} {}", py_repr(reference));
            compared += 1;
        }
    }
    assert!(compared > 150, "{compared}");
}

#[test]
fn lookup_find_matches_python_for_every_reference_and_direction() {
    check("find", |book, reference, direction| {
        find_transaction(book.connection(), "profile", reference, direction)
    });
}

#[test]
fn lookup_resolve_matches_python_not_found_messages() {
    check("resolve", |book, reference, direction| {
        resolve_transaction(book.connection(), "profile", reference, direction).map(Some)
    });
}

/// `ui.transactions.resolve` on the shared-txid book: the qualified fallback
/// and the rows sharing the shown row's txid.
#[test]
fn lookup_resolve_snapshot_lists_rows_sharing_the_txid_like_python() {
    let vectors = vectors();
    let (_tmp, book) = book(&vectors);
    let scope = kassiber_transactions::Scope {
        workspace_id: "ws",
        profile_id: "profile",
    };
    let mut compared = 0;
    for case in list(get(&vectors, "snapshots").unwrap()) {
        let args = dict([("query", get(case, "query").unwrap().clone())]);
        let actual = kassiber_transactions::desktop::execute(
            book.connection(),
            &scope,
            kassiber_transactions::desktop::Projection::Resolve,
            Some(&args),
            &|_, _| {
                Ok(dict([
                    ("reviewPairId", Value::Null),
                    ("pairFingerprint", Value::Null),
                ]))
            },
        )
        .unwrap_or_else(|error| error.envelope(None));
        assert_eq!(
            &actual,
            get(case, "expected").unwrap(),
            "{}",
            py_repr(&args)
        );
        compared += 1;
    }
    assert!(compared >= 12);
}
