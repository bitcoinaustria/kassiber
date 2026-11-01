use super::*;
use crate::analysis_index::fixture::tables;
use kassiber_pyjson::{dumps, Options};
fn canonical(v: &Value) -> String {
    dumps(v, &Options::new().sort_keys(true).compact()).unwrap()
}
fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(include_str!(
        "../../../../reports/tests/data/privacy_hygiene_seed.sql"
    ))
    .unwrap();
    conn.execute_batch(include_str!("../../../../reports/tests/data/book.sql"))
        .unwrap();
    conn
}
#[test]
fn handles_match_python_tokens_and_replay_all_112_tables() {
    let cases =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_handles.json")).unwrap();
    assert!(list(&cases).len() >= 48);
    for case in list(&cases) {
        let conn = database();
        conn.execute_batch(field(case, "sql").as_str().unwrap())
            .unwrap();
        let before = tables(&conn);
        assert_eq!(
            canonical(&evaluate_fixture(case, &conn).unwrap()),
            canonical(field(case, "expected")),
            "{}",
            field(case, "name").as_str().unwrap()
        );
        assert_eq!(tables(&conn), before);
        assert!(conn.is_autocommit());
    }
}
#[test]
fn handles_bind_process_profile_connection_and_book_identity() {
    let conn = database();
    let other = database();
    let before = tables(&conn);
    let scope = BookScope::read(&conn, "p").unwrap();
    assert!(scope != BookScope::read(&conn, "other").unwrap());
    assert!(scope != BookScope::read(&other, "p").unwrap());
    let mut refs = References::new(&conn).unwrap();
    let token = refs.issue(&scope, "subject").unwrap();
    assert_eq!(refs.resolve(&scope, &token).unwrap(), "subject");
    assert!(References::new(&conn)
        .unwrap()
        .resolve(&scope, &token)
        .is_err());
    refs.process = refs.process.wrapping_add(1);
    assert!(refs.resolve(&scope, &token).is_err());
    assert!(refs.issue(&scope, "subject").is_err());
    assert_eq!(tables(&conn), before);
    conn.execute(
        "INSERT OR REPLACE INTO settings(key,value) VALUES('database_instance_id',?)",
        ["a".repeat(32)],
    )
    .unwrap();
    let first = BookScope::read(&conn, "p").unwrap();
    conn.execute(
        "UPDATE settings SET value=? WHERE key='database_instance_id'",
        ["b".repeat(32)],
    )
    .unwrap();
    assert!(first != BookScope::read(&conn, "p").unwrap());
}
#[test]
fn handles_revalidate_current_subject_and_reject_unissued_tokens() {
    let cases =
        kassiber_pytext::loads(include_str!("../../../tests/data/analysis_handles.json")).unwrap();
    let conn = database();
    conn.execute_batch(field(&list(&cases)[0], "sql").as_str().unwrap())
        .unwrap();
    let scope = BookScope::read(&conn, "p").unwrap();
    let mut refs = References::new(&conn).unwrap();
    let subject = "a".repeat(64);
    let unissued = refs.token(&scope, &subject);
    assert!(refs.resolve(&scope, &unissued).is_err());
    let token = refs.issue(&scope, &subject).unwrap();
    let args = dict([("subject", s(token))]);
    super::super::views::with_read(&conn, "p", "owner", |view| {
        assert_eq!(
            field(&refs.decode(&args, view)?, "subject"),
            &s(subject.as_str())
        );
        Ok(())
    })
    .unwrap();
    conn.execute(
        "DELETE FROM chain_index_aliases WHERE profile_id='p' AND alias=?",
        [subject],
    )
    .unwrap();
    let before = tables(&conn);
    super::super::views::with_read(&conn, "p", "owner", |view| {
        assert_eq!(
            refs.decode(&args, view).unwrap_err().code,
            "chain_analysis_reference_stale"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(tables(&conn), before);
}
