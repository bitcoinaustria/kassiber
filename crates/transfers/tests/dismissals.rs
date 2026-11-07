//! Transfer dismissals against CPython (`scripts/export_dismissals.py`):
//! dismiss marks journals stale, list and delete read and undo the vetoes,
//! and suggestions explain what the matcher skipped.
use kassiber_envelope::{get, py_str, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{
    rusqlite::{types::ValueRef, Connection},
    Book,
};
use kassiber_transfers::pairing::{exclusion_counts, Population};
use kassiber_transfers::{execute_with_sources, Scope, Sources};
use std::collections::HashSet;

fn fixture() -> Value {
    kassiber_pytext::loads(include_str!("dismissals.json")).unwrap()
}
fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(items) => items,
        _ => &[],
    }
}
fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}
fn bytes(v: &Value) -> String {
    dumps(v, &Options::default()).unwrap()
}
struct Fixed {
    id: String,
    now: String,
}
impl Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, AppError> {
        Ok(self.id.clone())
    }
    fn now(&self, _: &Connection) -> Result<String, AppError> {
        Ok(self.now.clone())
    }
}
fn table(c: &Connection, sql: &str) -> Value {
    let mut statement = c.prepare(sql).unwrap();
    let names: Vec<_> = statement
        .column_names()
        .iter()
        .map(|n| n.to_string())
        .collect();
    let mut cursor = statement.query([]).unwrap();
    let mut rows = Vec::new();
    while let Some(row) = cursor.next().unwrap() {
        rows.push(kassiber_envelope::dict(names.iter().enumerate().map(
            |(i, name)| {
                (
                    name.as_str(),
                    match row.get_ref(i).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => n.into(),
                        ValueRef::Real(n) => Value::Float(n),
                        ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                        ValueRef::Blob(_) => panic!("blob"),
                    },
                )
            },
        )));
    }
    Value::List(rows)
}

#[test]
fn dismissal_services_match_python_results_errors_and_rows() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    for sql in list(field(&fixture, "sql")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let now = py_str(field(&fixture, "now"));
    let cases = list(field(&fixture, "cases"));
    assert!(cases.len() >= 25);
    for (index, case) in cases.iter().enumerate() {
        let operation = format!(
            "transfers.{}",
            match py_str(field(case, "operation")).as_str() {
                "dismiss" => "dismiss",
                "suggest" => "suggest",
                "review_context" => "review-context",
                "list" => "dismissals.list",
                _ => "dismissals.delete",
            }
        );
        let sources = Fixed {
            id: py_str(field(case, "id")),
            now: now.clone(),
        };
        let result = if operation == "transfers.review-context" {
            // The daemon's shaping: the suggestion, the default limit and no
            // saved views (`crates/daemon/src/transfers.rs`).
            execute_with_sources(
                c,
                &scope,
                "transfers.suggest",
                &kassiber_envelope::dict::<&str>([]),
                &sources,
            )
            .and_then(|suggestion| {
                let args = field(case, "args");
                let payload = kassiber_envelope::dict([
                    ("args", args.clone()),
                    ("suggestion", suggestion),
                    ("limit", Value::Int(8.into())),
                    ("saved_views", Value::List(vec![])),
                    ("workspace", field(args, "workspace").clone()),
                    ("profile", field(args, "profile").clone()),
                    ("fee_pct_max", Value::Float(0.01)),
                    ("fee_sats_min", Value::Int(2500.into())),
                ]);
                execute_with_sources(c, &scope, &operation, &payload, &sources)
            })
        } else {
            execute_with_sources(c, &scope, &operation, field(case, "args"), &sources)
        };
        let label = format!("{index} {operation} {}", bytes(field(case, "args")));
        match get(case, "result") {
            Some(expected) => assert_eq!(bytes(&result.unwrap()), bytes(expected), "{label}"),
            None => {
                let error = result.unwrap_err();
                let expected = field(case, "error");
                assert_eq!(s(error.code.as_str()), *field(expected, "code"), "{label}");
                assert_eq!(
                    s(error.message.as_str()),
                    *field(expected, "message"),
                    "{label}"
                );
                assert_eq!(
                    error.hint.as_deref().map_or(Value::Null, s),
                    *field(expected, "hint"),
                    "{label}"
                );
                assert_eq!(
                    error.details.unwrap_or(Value::Null),
                    *field(expected, "details"),
                    "{label}"
                );
            }
        }
        let rows = field(case, "rows");
        assert_eq!(
            bytes(&table(
                c,
                "SELECT * FROM transaction_pair_dismissals ORDER BY id"
            )),
            bytes(field(rows, "transaction_pair_dismissals")),
            "{label}: dismissals"
        );
        assert_eq!(
            bytes(&table(c, "SELECT id,last_processed_at,last_processed_tx_count,journal_input_version,ownership_review_counts_json FROM profiles ORDER BY id")),
            bytes(field(rows, "profiles")),
            "{label}: journal staleness"
        );
    }
}

#[test]
fn dismissal_exclusion_counts_match_python() {
    let fixture = fixture();
    let exclusions = field(&fixture, "exclusions");
    let population = Population::new(list(field(exclusions, "rows"))).unwrap();
    let cases = list(field(exclusions, "cases"));
    assert_eq!(cases.len(), 15);
    for (index, case) in cases.iter().enumerate() {
        let options = field(case, "options");
        let booked: HashSet<String> = list(field(options, "booked_move_transaction_ids"))
            .iter()
            .map(py_str)
            .collect();
        let actual = exclusion_counts(
            &population,
            list(field(options, "pair_records")),
            list(field(options, "dismissals")),
            &booked,
            &py_str(field(options, "now_iso")),
        );
        assert_eq!(bytes(&actual), bytes(field(case, "result")), "case {index}");
    }
}
