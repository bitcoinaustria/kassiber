use kassiber_envelope::{dict, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::Book;

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(
        &self,
        _: &kassiber_store::rusqlite::Connection,
    ) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(
        &self,
        _: &kassiber_store::rusqlite::Connection,
    ) -> Result<String, kassiber_envelope::AppError> {
        unreachable!()
    }
}
#[test]
fn gain_reads_pin_retained_calculations_custody_versions_and_desktop() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/gain_reads.json")).unwrap();
    assert!(cases.len() > 100);
    check(cases);
}

/// Round 8 (7c7fcc0a8): malformed fragments, lots and lot pricing in the
/// explained or an inherited calculation are a mismatch, not an error.
#[test]
fn gain_reads_refuse_malformed_retained_lot_pricing_as_mismatch() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/retained_pricing.json")).unwrap();
    assert_eq!(cases.len(), 45);
    check(cases);
}

fn check(cases: Vec<serde_json::Value>) {
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    for case in cases {
        let conn = book.connection();
        conn.execute_batch("SAVEPOINT gain_fixture").unwrap();
        if case["base"].as_bool().unwrap_or(true) {
            conn.execute_batch(include_str!("data/book.sql")).unwrap();
        }
        conn.execute_batch(case["sql"].as_str().unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let before = conn.total_changes();
        let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
        let actual = match kassiber_reports::execute_with_sources(
            conn,
            case["command"].as_str().unwrap(),
            &args,
            &Fixed,
        ) {
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
        let wire = dumps(
            &actual,
            &Options {
                sort_keys: true,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            wire,
            case["expected_json"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            before,
            conn.total_changes(),
            "{} wrote to the database",
            case["name"]
        );
        conn.execute_batch("ROLLBACK TO gain_fixture; RELEASE gain_fixture")
            .unwrap();
    }
}
