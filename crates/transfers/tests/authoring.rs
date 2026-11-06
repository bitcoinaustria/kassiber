use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{
    rusqlite::{types::ValueRef, Connection},
    Book,
};
use kassiber_transfers::{execute_with_sources, Scope, Sources};

fn array(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        panic!()
    }
}
struct Fixed<'a> {
    id: &'a str,
    now: &'a str,
}
impl Sources for Fixed<'_> {
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok(self.id.into())
    }
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok(self.now.into())
    }
}
fn scope() -> Scope<'static> {
    Scope {
        workspace_id: "w",
        profile_id: "p",
    }
}
fn book() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let fixture = kassiber_pytext::loads(include_str!("python.json")).unwrap();
    for sql in array(get(&fixture, "sql").unwrap()) {
        book.connection().execute_batch(&py_str(sql)).unwrap();
    }
    (dir, book)
}
fn rows(c: &Connection, table: &str) -> Value {
    let quoted = table.replace('"', "\"\"");
    let mut q = c.prepare(&format!("SELECT * FROM \"{quoted}\"")).unwrap();
    let order = (1..=q.column_count())
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    q = c
        .prepare(&format!("SELECT * FROM \"{quoted}\" ORDER BY {order}"))
        .unwrap();
    let names: Vec<_> = q.column_names().iter().map(|s| s.to_string()).collect();
    let mut cursor = q.query([]).unwrap();
    let mut rows = vec![];
    while let Some(row) = cursor.next().unwrap() {
        rows.push(dict(names.iter().enumerate().map(|(i, k)| {
            (
                k.as_str(),
                match row.get_ref(i).unwrap() {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => n.into(),
                    ValueRef::Real(n) => Value::Float(n),
                    ValueRef::Text(s) => Value::Str(std::str::from_utf8(s).unwrap().into()),
                    _ => panic!(),
                },
            )
        })));
    }
    Value::List(rows)
}
fn bytes(v: &Value) -> String {
    dumps(v, &Options::default()).unwrap()
}

#[test]
fn authoring_sequences_match_python_rows_and_errors_byte_for_byte() {
    let (_dir, book) = book();
    let fixture = kassiber_pytext::loads(include_str!("authoring.json")).unwrap();
    // Marking journals stale updates profiles, which the chain-analysis
    // overlay triggers record in the chain index clock and dirty set.
    let names:Vec<String>=book.connection().prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('swap_matching_rules','transaction_pair_dismissals','profiles','chain_index_clock','chain_index_dirty') ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    let original: Vec<_> = names
        .iter()
        .map(|name| (name, rows(book.connection(), name)))
        .collect();
    for case in array(&fixture) {
        let id = py_str(get(case, "id").unwrap());
        let now = py_str(get(case, "now").unwrap());
        let result = execute_with_sources(
            book.connection(),
            &scope(),
            &py_str(get(case, "operation").unwrap()),
            get(case, "args").unwrap(),
            &Fixed { id: &id, now: &now },
        );
        if let Some(expected) = get(case, "result") {
            assert_eq!(bytes(&result.unwrap()), bytes(expected), "{id}");
        } else {
            let error = result.unwrap_err();
            let expected = get(case, "error").unwrap();
            assert_eq!(
                s(error.code),
                get(expected, "code").unwrap().clone(),
                "{id}"
            );
            assert_eq!(
                s(error.message),
                get(expected, "message").unwrap().clone(),
                "{id}"
            );
            assert_eq!(
                Value::from(error.hint),
                get(expected, "hint").unwrap().clone(),
                "{id}"
            );
            assert_eq!(
                error.details.unwrap_or(Value::Null),
                get(expected, "details").unwrap().clone(),
                "{id}"
            );
        }
        let Value::Dict(tables) = get(case, "rows").unwrap() else {
            panic!()
        };
        for (name, expected) in tables {
            let kassiber_pyjson::Key::Str(name) = name else {
                panic!()
            };
            assert_eq!(
                bytes(&rows(book.connection(), name.as_str().unwrap())),
                bytes(expected),
                "{id}: {name:?}"
            );
        }
        for (name, before) in &original {
            assert_eq!(
                &rows(book.connection(), name),
                before,
                "{id}: unchanged table {name}"
            );
        }
    }
}

#[test]
fn rule_scope_dismissal_reference_priority_and_rollback_are_preserved() {
    let (_dir, book) = book();
    let c = book.connection();
    let source = Fixed {
        id: "scoped",
        now: "2025-06-01T00:00:00Z",
    };
    let other = Scope {
        workspace_id: "w",
        profile_id: "other",
    };
    assert_eq!(
        execute_with_sources(
            c,
            &other,
            "transfers.rules.delete",
            &dict([("rule_id", s("invalid"))]),
            &source
        )
        .unwrap_err()
        .code,
        "not_found"
    );
    assert_eq!(
        execute_with_sources(
            c,
            &other,
            "transfers.dismiss",
            &dict([("tx_out", s("tx0")), ("tx_in", s("tx1"))]),
            &source
        )
        .unwrap_err()
        .code,
        "app_error"
    );
    c.execute_batch("UPDATE transactions SET external_id='shared' WHERE id IN ('tx1','tx2')")
        .unwrap();
    let args = dict([("tx_out", s("shared")), ("tx_in", s("tx0"))]);
    assert_eq!(
        execute_with_sources(c, &scope(), "transfers.dismiss", &args, &source)
            .unwrap_err()
            .code,
        "ambiguous_reference"
    );
    c.execute_batch("UPDATE transactions SET external_id='tx1' WHERE id='tx2'")
        .unwrap();
    let args = dict([
        ("tx_out", s("tx1")),
        ("tx_in", s("tx0")),
        ("expires_in_days", 0.into()),
    ]);
    let result = execute_with_sources(c, &scope(), "transfers.dismiss", &args, &source).unwrap();
    assert_eq!(get(&result, "out_transaction_id"), Some(&s("tx1")));
    // A readback failure after an INSERT must roll the INSERT back as well.
    c.execute_batch("CREATE TRIGGER remove_new_rule AFTER INSERT ON swap_matching_rules BEGIN DELETE FROM swap_matching_rules WHERE id='invalid' OR id=NEW.id; END;").unwrap();
    let before = rows(c, "swap_matching_rules");
    assert!(execute_with_sources(
        c,
        &scope(),
        "transfers.rules.create",
        &dict::<&str>([]),
        &source
    )
    .is_err());
    assert_eq!(rows(c, "swap_matching_rules"), before);
    assert!(c.is_autocommit()); // service savepoints are always released
    c.execute_batch("BEGIN").unwrap();
    execute_with_sources(
        c,
        &scope(),
        "transfers.rules.disable",
        &dict([("rule_id", s("invalid"))]),
        &source,
    )
    .unwrap();
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn profile_policy_defaults_do_not_author_ownership_or_journal_freshness() {
    let (_dir, book) = book();
    let c = book.connection();
    for (i, country, rail, predicate, expected) in [
        (
            0,
            "generic",
            true,
            r#"{"out_asset":"BTC","in_asset":"LBTC"}"#,
            "carrying-value",
        ),
        (
            1,
            "generic",
            false,
            r#"{"out_asset":"BTC","in_asset":"LBTC"}"#,
            "taxable",
        ),
        (
            2,
            "generic",
            true,
            r#"{"out_asset":"BTC","in_asset":"ETH"}"#,
            "taxable",
        ),
        (
            3,
            "at",
            false,
            r#"{"out_asset":"BTC","in_asset":"ETH"}"#,
            "carrying-value",
        ),
        (
            4,
            "generic",
            false,
            r#"{"out_asset":" btc ","in_asset":"BTC"}"#,
            "carrying-value",
        ),
        (
            5,
            "generic",
            false,
            r#"{"out_asset":"ß","in_asset":"SS"}"#,
            "carrying-value",
        ),
        (
            6,
            "generic",
            false,
            "{\"out_asset\":\"\u{1c8a}\",\"in_asset\":\"\u{1c89}\"}",
            "taxable",
        ),
    ] {
        c.execute(
            "UPDATE profiles SET tax_country=?,bitcoin_rail_carrying_value=? WHERE id='p'",
            kassiber_store::rusqlite::params![country, rail],
        )
        .unwrap();
        let result = execute_with_sources(
            c,
            &scope(),
            "transfers.rules.create",
            &dict([("predicate", kassiber_pytext::loads(predicate).unwrap())]),
            &Fixed {
                id: &format!("policy-{i}"),
                now: "2025-06-01T00:00:00Z",
            },
        )
        .unwrap();
        assert_eq!(get(&result, "policy"), Some(&s(expected)));
    }
}
