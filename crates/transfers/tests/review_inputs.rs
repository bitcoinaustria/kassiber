#![cfg(feature = "test-harness")]
//! Review input requests against the frozen Python outcomes of
//! `scripts/export_review_inputs.py`: packets, digests and refusal order.
use kassiber_envelope::{get, py_str};
use kassiber_journals::review_workflow::{self, Error};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_transfers::Scope;

const CLOCK: &str = "2026-10-31T12:00:00Z";

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap()
}
fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(values) => values,
        _ => panic!("expected list"),
    }
}
fn bytes(value: &Value) -> String {
    dumps(value, &Options::default().sort_keys(true)).unwrap()
}

fn book(fixture: &Value, scenario: &Value) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA temp_store=MEMORY")
        .unwrap();
    for statements in [
        field(fixture, "schema"),
        field(scenario, "sql"),
        field(fixture, "objects"),
    ] {
        for statement in list(statements) {
            conn.execute_batch(&py_str(statement)).unwrap();
        }
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    conn
}

fn changes(conn: &Connection) -> i64 {
    conn.query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap()
}

fn outcome(result: Result<Value, Error>) -> Value {
    match result {
        Ok(value) => kassiber_envelope::dict([("result", value)]),
        Err(Error::Service(error)) => kassiber_envelope::dict([(
            "error",
            kassiber_envelope::dict([
                ("code", kassiber_envelope::s(error.code.as_str())),
                ("message", kassiber_envelope::s(error.message.as_str())),
                ("details", error.details.unwrap_or(Value::Null)),
            ]),
        )]),
        Err(Error::Build(refusal)) => panic!("unexpected build refusal {refusal}"),
    }
}

#[test]
fn review_input_requests_match_python_packets_digests_and_refusals_without_writes() {
    let fixture = kassiber_pytext::loads(include_str!("review_cases.json")).unwrap();
    let cases = kassiber_pytext::loads(include_str!("review_inputs.json")).unwrap();
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let mut successes = 0;
    for case in list(&cases) {
        let scenario = list(field(&fixture, "scenarios"))
            .iter()
            .find(|row| field(row, "name") == field(case, "scenario"))
            .unwrap();
        let conn = book(&fixture, scenario);
        let before = changes(&conn);
        let actual = outcome(review_workflow::request_input(
            &conn,
            &scope,
            field(case, "args"),
            CLOCK,
        ));
        let mut expected = case.clone();
        if let Value::Dict(entries) = &mut expected {
            entries.retain(|(key, _)| *key == "result".into() || *key == "error".into());
        }
        assert_eq!(bytes(&actual), bytes(&expected), "{}", bytes(case));
        assert!(conn.is_autocommit(), "the owned read snapshot is released");
        assert_eq!(changes(&conn), before, "{}", bytes(case));
        successes += usize::from(get(case, "result").is_some());
    }
    assert!(successes >= 40, "{successes} successful packets");
}

#[test]
fn review_input_requests_keep_a_caller_transaction_open_and_see_its_edits() {
    let fixture = kassiber_pytext::loads(include_str!("review_cases.json")).unwrap();
    let cases = kassiber_pytext::loads(include_str!("review_inputs.json")).unwrap();
    let case = list(&cases)
        .iter()
        .find(|case| get(case, "result").is_some())
        .unwrap();
    let scenario = list(field(&fixture, "scenarios"))
        .iter()
        .find(|row| field(row, "name") == field(case, "scenario"))
        .unwrap();
    let conn = book(&fixture, scenario);
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    conn.execute_batch("BEGIN").unwrap();
    let packet = review_workflow::request_input(&conn, &scope, field(case, "args"), CLOCK).unwrap();
    assert_eq!(bytes(&packet), bytes(field(case, "result")));
    assert!(!conn.is_autocommit(), "a caller transaction stays open");
    // An uncommitted version change in the caller's transaction is current.
    conn.execute_batch("UPDATE profiles SET journal_input_version=journal_input_version+1")
        .unwrap();
    let Err(Error::Service(error)) =
        review_workflow::request_input(&conn, &scope, field(case, "args"), CLOCK)
    else {
        panic!("a stale version must refuse");
    };
    assert_eq!(error.code, "custody_review_plan_stale");
    conn.execute_batch("ROLLBACK").unwrap();
}
