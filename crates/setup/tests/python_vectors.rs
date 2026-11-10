//! Frozen Python answers, including exact identities, times, storage values and
//! error fields. No UUID/time/order normalization is used.
mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::types::ValueRef;
use support::*;
fn canonical(v: &Value) -> String {
    dumps(
        v,
        &Options {
            sort_keys: true,
            ..Options::default()
        },
    )
    .unwrap()
}
fn state(f: &Fixture) -> Value {
    dict(
        [
            "workspaces",
            "profiles",
            "accounts",
            "saved_views",
            "settings",
        ]
        .map(|table| {
            let sql = if table == "settings" {
                "SELECT * FROM settings WHERE key LIKE 'context_%' ORDER BY key".to_owned()
            } else {
                format!("SELECT * FROM {table} ORDER BY id")
            };
            let mut statement = f.book.connection().prepare(&sql).unwrap();
            let rows = statement
                .query_map([], |r| {
                    Ok(dict(r.as_ref().column_names().iter().enumerate().map(
                        |(i, key)| {
                            let v = match r.get_ref(i).unwrap() {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(n) => Value::Int(n.into()),
                                ValueRef::Real(n) => Value::Float(n),
                                ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                                ValueRef::Blob(_) => panic!("unexpected blob"),
                            };
                            (*key, v)
                        },
                    )))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            (table, Value::List(rows))
        }),
    )
}
#[test]
fn frozen_python_setup_responses_and_database_states() {
    let fixture = kassiber_pytext::loads(include_str!("../fixtures/python_setup.json")).unwrap();
    let Value::List(records) = get(&fixture, "records").unwrap() else {
        panic!()
    };
    let f = Fixture::new();
    for (i, record) in records.iter().enumerate() {
        let command = text(record, "command");
        let result = kassiber_setup::execute(
            f.book.connection(),
            &command,
            get(record, "args").unwrap(),
            &f.sources,
        );
        let outcome = match result {
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
        let expected = if let Some(v) = get(record, "ok") {
            dict([("ok", v.clone())])
        } else {
            dict([("error", get(record, "error").unwrap().clone())])
        };
        assert_eq!(
            canonical(&outcome),
            canonical(&expected),
            "record {i}: {command}"
        );
        assert_eq!(
            canonical(&state(&f)),
            canonical(get(record, "state").unwrap()),
            "state after record {i}: {command}"
        );
        assert_eq!(
            Value::Int(f.sources.0.get().into()),
            *get(record, "identity_calls").unwrap(),
            "entropy calls after record {i}: {command}"
        );
    }
}
