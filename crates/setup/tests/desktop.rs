mod support;
use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_setup::{profile_reset, sensitive};
use kassiber_store::rusqlite::types::ValueRef;
use support::*;

fn canonical(value: &Value) -> String {
    dumps(
        value,
        &Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn frozen_python_desktop_responses_and_database_states() {
    let fixture = kassiber_pytext::loads(include_str!("../fixtures/python_desktop.json")).unwrap();
    let Value::List(records) = get(&fixture, "records").unwrap() else {
        panic!()
    };
    let f = Fixture::new();
    for (index, record) in records.iter().enumerate() {
        let sql = text(record, "sql");
        if !sql.is_empty() {
            f.book.connection().execute_batch(&sql).unwrap();
        }
        let kind = text(record, "kind");
        let args = get(record, "args").unwrap();
        let result = if kind == "ui.profiles.reset_data" {
            profile_reset::execute(
                f.book.connection(),
                args,
                &f._root.path().join("attachments"),
            )
        } else {
            kassiber_setup::execute(f.book.connection(), &kind, args, &f.sources)
        };
        let outcome = match result {
            Ok(v) => dict([("ok", v)]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map_or(Value::Null, s)),
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
        assert_eq!(canonical(&outcome), canonical(&expected), "{index} {kind}");
        let Value::Dict(tables) = get(record, "state").unwrap() else {
            panic!()
        };
        for (table, expected) in tables {
            let kassiber_pyjson::Key::Str(table) = table else {
                panic!()
            };
            let table = table.as_str().unwrap();
            let sql = if table == "context" {
                "SELECT * FROM settings WHERE key IN ('context_workspace','context_profile') ORDER BY key".into()
            } else {
                format!("SELECT * FROM {table} ORDER BY rowid")
            };
            let mut stmt = f.book.connection().prepare(&sql).unwrap();
            let values = stmt
                .query_map([], |r| {
                    Ok(dict(r.as_ref().column_names().iter().enumerate().map(
                        |(i, k)| {
                            (
                                *k,
                                match r.get_ref(i).unwrap() {
                                    ValueRef::Null => Value::Null,
                                    ValueRef::Integer(n) => Value::Int(n.into()),
                                    ValueRef::Real(n) => Value::Float(n),
                                    ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                                    _ => panic!(),
                                },
                            )
                        },
                    )))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(
                canonical(&Value::List(values)),
                canonical(expected),
                "state {index} {kind} {table}"
            );
        }
    }
}

#[test]
fn sensitive_selection_requires_exact_current_names_and_boolean_rate_scope() {
    let context = dict([("workspace_label", s("Main")), ("profile_label", s("Book"))]);
    for (kind, confirm, key, name) in [
        ("ui.workspace.delete", "DELETE", "confirm_workspace", "Main"),
        ("ui.profiles.reset_data", "RESET", "confirm_profile", "Book"),
    ] {
        for args in [
            dict::<&str>([]),
            dict([("confirm", s(confirm)), (key, s(name.to_lowercase()))]),
            dict([("confirm", s(confirm)), (key, Value::Int(1.into()))]),
        ] {
            assert!(sensitive::prepare(kind, &args, &context).is_err());
        }
        let args = dict([("confirm", s(confirm)), (key, s(name))]);
        assert_eq!(
            sensitive::prepare(kind, &args, &context).unwrap().ack_value,
            "DELETE LOCAL DATA"
        );
    }
    for flag in [s("false"), Value::Int(1.into()), Value::List(vec![])] {
        let args = dict([
            ("confirm", s("RESET")),
            ("confirm_profile", s("Book")),
            ("clear_shared_rates", flag),
        ]);
        assert!(sensitive::prepare("ui.profiles.reset_data", &args, &context).is_err());
    }
    for args in [
        dict::<&str>([]),
        dict([("name", Value::Int(1.into())), ("backend", s("private"))]),
    ] {
        assert!(sensitive::prepare("backends.reveal_token", &args, &context).is_err());
    }
    let a = sensitive::prepare(
        "backends.reveal_token",
        &dict([("name", s("one")), ("backend", s("two"))]),
        &context,
    )
    .unwrap();
    assert!(a.label.ends_with("'one'"));
    assert_eq!(a.ack_value, "COPY LOCAL SECRET");
}

#[test]
fn archived_ledger_blocks_only_its_profile_and_unscoped_rows_block_all() {
    let f = Fixture::seed();
    let pid = text(&f.profile(), "id");
    f.book
        .connection()
        .execute_batch(
            "CREATE TABLE gl_archive(profile_id TEXT); INSERT INTO gl_archive VALUES('other');",
        )
        .unwrap();
    sensitive::require_no_legacy(
        f.book.connection(),
        std::slice::from_ref(&pid),
        "resetting its data",
    )
    .unwrap();
    f.book
        .connection()
        .execute("INSERT INTO gl_archive VALUES(?)", [&pid])
        .unwrap();
    assert_eq!(
        sensitive::require_no_legacy(
            f.book.connection(),
            std::slice::from_ref(&pid),
            "resetting its data"
        )
        .unwrap_err()
        .code,
        "legacy_ledger_present"
    );
    f.book.connection().execute_batch("DELETE FROM gl_archive;CREATE TABLE gl_global(value TEXT);INSERT INTO gl_global VALUES('retained');").unwrap();
    assert!(
        sensitive::require_no_legacy(f.book.connection(), &[pid], "deleting this books set")
            .is_err()
    );
}

#[test]
fn profile_reset_keeps_other_books_and_confines_attachment_cleanup() {
    let f = Fixture::seed();
    let p = f.profile();
    let pid = text(&p, "id");
    let wid = text(&p, "workspace_id");
    let other = f.run("profiles.create", dict([("label", s("Other"))]));
    let oid = text(&other, "id");
    f.run(
        "ui.profiles.switch",
        dict([("profile_id", s(pid.as_str()))]),
    );
    let attachments = f._root.path().join("attachments");
    std::fs::create_dir_all(attachments.join("managed")).unwrap();
    std::fs::write(attachments.join("managed/receipt"), "synthetic").unwrap();
    let outside = f._root.path().join("outside");
    std::fs::write(&outside, "preserved").unwrap();
    for (id, profile, path) in [
        ("owned", &pid, "managed/receipt"),
        ("other", &oid, "other/receipt"),
        ("escape", &pid, "../outside"),
    ] {
        f.book.connection().execute("INSERT INTO attachments(id,workspace_id,profile_id,attachment_type,stored_relpath,created_at) VALUES(?,?,?,'file',?,'t')",[id,&wid,profile,path]).unwrap();
    }
    let result =
        profile_reset::execute(f.book.connection(), &dict::<&str>([]), &attachments).unwrap();
    assert_eq!(
        get(get(&result, "removed").unwrap(), "attachment_files"),
        Some(&Value::Int(1.into()))
    );
    assert!(!attachments.join("managed").exists());
    assert!(outside.exists());
    assert_eq!(
        f.book
            .connection()
            .query_row("SELECT profile_id FROM attachments", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        oid
    );
    assert_eq!(
        f.book
            .connection()
            .query_row(
                "SELECT count(*) FROM accounts WHERE profile_id=?",
                [&pid],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn profile_display_uses_the_stored_method_for_each_country() {
    use kassiber_setup::profile_snapshot::{policy_method, tax_policy_label};
    for (country, method, expected) in [
        ("at", "FIFO", "Austria - FIFO - EUR"),
        ("at", "MOVING_AVERAGE_AT", "Austria - ATM - EUR"),
        (
            "generic",
            "HIFO",
            "Generic - HIFO - EUR - 730 day long-term",
        ),
    ] {
        let p = dict([
            ("tax_country", s(country)),
            ("gains_algorithm", s(method)),
            ("fiat_currency", s("EUR")),
            ("tax_long_term_days", Value::Int(730.into())),
        ]);
        assert_eq!(tax_policy_label(&p), expected);
        assert_eq!(policy_method(&p), method.to_lowercase());
    }
}
