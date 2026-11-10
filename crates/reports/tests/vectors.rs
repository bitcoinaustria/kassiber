use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::Book;
use serde_json::Value as Json;

fn check(group: &str) {
    let cases: Vec<Json> = serde_json::from_str(include_str!("data/vectors.json")).unwrap();
    assert!(
        cases.iter().any(|v| v["group"] == group),
        "empty oracle group: {group}"
    );
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    for case in cases.iter().filter(|v| v["group"] == group) {
        book.connection()
            .execute_batch("SAVEPOINT oracle_fixture")
            .unwrap();
        if case["base"].as_bool().unwrap_or(true) {
            book.connection()
                .execute_batch(include_str!("data/book.sql"))
                .unwrap();
        }
        book.connection()
            .execute_batch(case["sql"].as_str().unwrap())
            .unwrap();
        let args = kassiber_pytext::loads(&case["args"].to_string()).unwrap();
        let before = book.connection().total_changes();
        let result =
            kassiber_reports::execute(book.connection(), case["command"].as_str().unwrap(), &args);
        let value = match result {
            Ok(v) => dict([("ok", v)]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(e.retryable)),
                ]),
            )]),
        };
        let actual = dumps(
            &value,
            &Options {
                ensure_ascii: false,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            actual,
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            before,
            book.connection().total_changes(),
            "a report wrote to the book"
        );
        book.connection()
            .execute_batch("ROLLBACK TO oracle_fixture; RELEASE oracle_fixture")
            .unwrap();
    }
}
#[test]
fn capital_gains_vectors() {
    check("gains");
}
#[test]
fn holdings_vectors() {
    check("holdings");
}
#[test]
fn history_period_vectors() {
    check("history");
}
#[test]
fn history_quantity_vectors() {
    check("history-quantity");
}
#[test]
fn history_quote_vectors() {
    check("history-quotes");
}
#[test]
fn history_scope_vectors() {
    check("history-scope");
}
#[test]
fn history_readiness_vectors() {
    check("history-readiness");
}
#[test]
fn desktop_history_vectors() {
    check("desktop-history");
}
#[test]
fn summary_vectors() {
    check("summary");
}
#[test]
fn summary_text_vectors() {
    check("text");
}
#[test]
fn desktop_vectors() {
    check("desktop");
}
#[test]
fn readiness_vectors() {
    check("readiness");
}
#[test]
fn snapshots_vectors() {
    check("snapshots");
}

#[test]
fn components_vectors() {
    check("components");
}

#[test]
fn tax_summary_vectors() {
    check("tax");
}
#[test]
fn austrian_vectors() {
    check("austrian");
}
#[test]
fn austrian_text_vectors() {
    check("austrian-text");
}
#[test]
fn desktop_tax_vectors() {
    check("desktop-tax");
}

#[test]
fn journal_entry_vectors() {
    check("entries");
}

#[test]
fn report_csv_projection_vectors() {
    check("csv");
}
