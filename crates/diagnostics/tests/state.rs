use kassiber_diagnostics::state;
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::rusqlite::Connection;
#[test]
fn all_state_aggregates_match_planted_python_fixture() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!("state.sql")).unwrap();
    let config = kassiber_pytext::loads(include_str!("backends.json")).unwrap();
    let report = state::state(&conn, state::backends(&config)).unwrap();
    let bytes = dumps(
        &report,
        &Options {
            indent: Some(2.into()),
            ..Options::default()
        },
    )
    .unwrap()
        + "\n";
    assert_eq!(bytes, include_str!("state.json"));
    for secret in [
        "planted-wallet-secret",
        "private-txid",
        "private-note",
        "planted-credential",
        "private.example",
        "private-user",
        "private-profile-id",
        "planted-table-secret",
    ] {
        assert!(!bytes.contains(secret), "leaked {secret}");
    }
    for writer in [0, 1] {
        conn.execute("UPDATE profiles SET journal_semantics_version=?", [writer])
            .unwrap();
        let report = state::state(&conn, state::backends(&config)).unwrap();
        let profiles = kassiber_envelope::get(&report, "profiles").unwrap();
        assert_eq!(
            kassiber_envelope::get(profiles, "processed"),
            Some(&Value::Int(0.into()))
        );
        assert_eq!(
            kassiber_envelope::get(profiles, "stale_or_unprocessed"),
            Some(&Value::Int(2.into()))
        );
    }
}
#[test]
fn integrity_checks_match_reference_without_database_or_with_corruption() {
    assert_eq!(
        state::checks(None),
        kassiber_envelope::dict([("database_integrity", kassiber_envelope::s("unavailable"))])
    );
    let conn = Connection::open_in_memory().unwrap();
    assert_eq!(
        state::checks(Some(&conn)),
        kassiber_envelope::dict([("database_integrity", kassiber_envelope::s("ok"))])
    );
}
#[test]
fn backend_locality_is_syntactic_and_matches_python_address_registry() {
    use kassiber_envelope::{dict, get, s};
    let cases: serde_json::Value = serde_json::from_str(include_str!("localities.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let config = dict([(
            "backends",
            dict([(
                "private-name",
                dict([("url", s(case["url"].as_str().unwrap()))]),
            )]),
        )]);
        let summary = state::backends(&config);
        let Value::List(rows) = get(&summary, "by_locality").unwrap() else {
            panic!()
        };
        assert_eq!(
            get(&rows[0], "locality"),
            Some(&s(case["expected"].as_str().unwrap())),
            "{}",
            case["url"]
        );
    }
}
#[test]
fn empty_backend_config_discloses_no_credential_keys_or_counts() {
    use kassiber_envelope::{dict, get};
    let summary = state::backends(&dict::<&str>([]));
    assert_eq!(
        get(&summary, "credential_presence"),
        Some(&dict::<&str>([]))
    );
    assert_eq!(get(&summary, "total"), Some(&Value::Int(0.into())));
}
