use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{
    rusqlite::{types::ValueRef, Connection},
    Book,
};
use kassiber_transfers::{execute_with_sources, Scope, Sources};
fn list(v: &Value) -> &[Value] {
    if let Value::List(items) = v {
        items
    } else {
        panic!("not list")
    }
}
fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    get(v, k).unwrap()
}
fn bytes(v: &Value) -> String {
    dumps(
        v,
        &Options {
            sort_keys: true,
            ..Options::default()
        },
    )
    .unwrap()
}
struct Fixed {
    id: String,
    now: String,
}
impl Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok(self.id.clone())
    }
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok(self.now.clone())
    }
}
fn dump(c: &Connection, table: &str) -> Value {
    let mut stmt = c
        .prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
        .unwrap();
    let names: Vec<_> = stmt.column_names().iter().map(|n| n.to_string()).collect();
    let mut cursor = stmt.query([]).unwrap();
    let mut out = vec![];
    while let Some(r) = cursor.next().unwrap() {
        out.push(dict(names.iter().enumerate().map(|(i, n)| {
            (
                n.as_str(),
                match r.get_ref(i).unwrap() {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::Int(n.into()),
                    ValueRef::Real(n) => Value::Float(n),
                    ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                    ValueRef::Blob(_) => panic!("unexpected blob"),
                },
            )
        })));
    }
    out.sort_by_key(bytes);
    Value::List(out)
}
#[test]
fn pair_sequences_match_python_complete_database_rows_and_refusals() {
    run_sequences(include_str!("pairs.json"));
}
#[test]
fn payout_creations_match_python_complete_database_rows_and_refusals() {
    run_sequences(include_str!("payout_create.json"));
}
#[test]
fn payout_deletions_match_python_complete_database_rows_and_refusals() {
    run_sequences(include_str!("payout_delete.json"));
}
#[test]
fn component_reviews_match_python_complete_database_rows_and_refusals() {
    run_sequences(include_str!("components.json"));
}
#[test]
fn gap_reviews_match_python_complete_database_rows_and_refusals() {
    run_sequences(include_str!("gaps.json"));
}
#[test]
fn gap_worklists_match_python_without_changing_database_rows() {
    run_sequences(include_str!("gap_worklists.json"));
}
#[test]
fn gap_apply_rolls_back_late_failure_and_preserves_outer_transaction() {
    let fixture = kassiber_pytext::loads(include_str!("gaps.json")).unwrap();
    let scenario = &list(field(&fixture, "scenarios"))[0];
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    for sql in list(field(scenario, "sql")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    let case = &list(field(scenario, "cases"))[1];
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    c.execute_batch("BEGIN; INSERT INTO settings(key,value) VALUES('caller-gap','pending'); CREATE TEMP TRIGGER refuse_gap_review BEFORE INSERT ON custody_gap_reviews BEGIN SELECT RAISE(ABORT,'late review failure'); END;").unwrap();
    let before: Vec<_> = tables.iter().map(|t| (t, bytes(&dump(c, t)))).collect();
    let error = call(c, "transfers.gaps.apply", field(case, "args").clone()).unwrap_err();
    assert_eq!(error.message, "late review failure");
    assert!(!c.is_autocommit());
    for (t, rows) in before {
        assert_eq!(bytes(&dump(c, t)), rows, "rollback {t}");
    }
    c.execute_batch("DROP TRIGGER refuse_gap_review").unwrap();
    call(c, "transfers.gaps.apply", field(case, "args").clone()).unwrap();
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    assert!(list(&dump(c, "custody_components")).is_empty());
    assert!(list(&dump(c, "custody_gap_reviews")).is_empty());
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM settings WHERE key='caller-gap'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

fn run_sequences(input: &str) {
    let fixture = kassiber_pytext::loads(input).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    for scenario in list(field(&fixture, "scenarios")) {
        c.execute_batch("SAVEPOINT pair_oracle_scenario").unwrap();
        for sql in list(field(scenario, "sql")) {
            c.execute_batch(&py_str(sql)).unwrap();
        }
        let tables: Vec<String> = c
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let default = [
            "chain_index_clock",
            "chain_index_dirty",
            "profiles",
            "custody_components",
            "custody_component_legs",
            "custody_component_allocations",
            "custody_component_economic_terms",
            "custody_component_transaction_memberships",
            "custody_authored_evidence_snapshots",
            "custody_component_evidence_commitments",
        ];
        let changed: Vec<String> = match get(&fixture, "tables") {
            Some(tables) => list(tables).iter().map(py_str).collect(),
            None => default.iter().map(|t| t.to_string()).collect(),
        };
        for case in list(field(scenario, "cases")) {
            for sql in list(field(case, "setup")) {
                c.execute_batch(&py_str(sql)).unwrap();
            }
            let untouched: Vec<_> = tables
                .iter()
                .filter(|n| !changed.contains(n))
                .map(|n| (n.clone(), bytes(&dump(c, n))))
                .collect();
            let id = py_str(field(case, "id"));
            let profile = py_str(field(case, "profile"));
            let fixed = Fixed {
                id: id.clone(),
                now: py_str(field(case, "now")),
            };
            let result = execute_with_sources(
                c,
                &Scope {
                    workspace_id: "w",
                    profile_id: &profile,
                },
                &py_str(field(case, "operation")),
                field(case, "args"),
                &fixed,
            );
            if let Some(expected) = get(case, "result") {
                let actual = result.unwrap_or_else(|e| panic!("{id}: {e:?}"));
                assert_eq!(
                    dumps(&actual, &Options::default()).unwrap(),
                    dumps(expected, &Options::default()).unwrap(),
                    "{id}: result"
                );
            } else {
                let e = result.unwrap_err();
                let expected = field(case, "error");
                assert_eq!(s(e.code), field(expected, "code").clone(), "{id}: code");
                assert_eq!(
                    s(e.message),
                    field(expected, "message").clone(),
                    "{id}: message"
                );
                assert_eq!(
                    Value::from(e.hint),
                    field(expected, "hint").clone(),
                    "{id}: hint"
                );
                assert_eq!(
                    dumps(&e.details.unwrap_or(Value::Null), &Options::default()).unwrap(),
                    dumps(field(expected, "details"), &Options::default()).unwrap(),
                    "{id}: details"
                );
            }
            for (table, before) in &untouched {
                assert_eq!(&bytes(&dump(c, table)), before, "{id}: untouched {table}");
            }
            let Value::Dict(tables) = field(case, "rows") else {
                panic!()
            };
            for (table, key) in tables {
                let kassiber_pyjson::Key::Str(table) = table else {
                    panic!()
                };
                let table = table.as_str().unwrap();
                let expected = field(field(&fixture, "states"), &py_str(key));
                let actual = dump(c, table);
                assert_eq!(bytes(&actual), bytes(expected), "{id}: table {table}");
            }
        }
        c.execute_batch("ROLLBACK TO pair_oracle_scenario; RELEASE pair_oracle_scenario")
            .unwrap();
    }
}

fn fresh() -> (tempfile::TempDir, Book) {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let fixture = kassiber_pytext::loads(include_str!("pairs.json")).unwrap();
    for sql in list(field(&list(field(&fixture, "scenarios"))[0], "sql")) {
        book.connection().execute_batch(&py_str(sql)).unwrap();
    }
    (dir, book)
}
fn call(
    c: &Connection,
    operation: &str,
    args: Value,
) -> Result<Value, kassiber_envelope::AppError> {
    execute_with_sources(
        c,
        &Scope {
            workspace_id: "w",
            profile_id: "p",
        },
        operation,
        &args,
        &Fixed {
            id: "guarded".into(),
            now: "2025-06-01T00:00:00Z".into(),
        },
    )
}
#[test]
fn pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership() {
    let (_dir, book) = fresh();
    let c = book.connection();
    let created = call(
        c,
        "transfers.pair",
        dict([("tx_out", s("d")), ("tx_in", s("e"))]),
    )
    .unwrap();
    call(
        c,
        "transfers.update",
        dict([
            ("pair_id", field(&created, "id").clone()),
            ("notes", s("revised")),
        ]),
    )
    .unwrap();
    for (sql, trigger) in [
        (
            "UPDATE custody_components SET notes='tampered'",
            "custody_component_revision_immutable",
        ),
        (
            "UPDATE custody_component_legs SET amount_msat=amount_msat+1",
            "custody_component_leg_revision_immutable",
        ),
        (
            "UPDATE custody_component_allocations SET source_amount_msat=source_amount_msat+1",
            "custody_component_allocation_revision_immutable",
        ),
        (
            "UPDATE custody_component_economic_terms SET review_notes='tampered'",
            "custody_component_terms_immutable",
        ),
        (
            "UPDATE custody_component_evidence_commitments SET detail_hash='tampered'",
            "custody_component_evidence_revision_immutable",
        ),
        (
            "UPDATE custody_authored_evidence_snapshots SET payload_json='{}'",
            "custody_authored_evidence_immutable",
        ),
        (
            "DELETE FROM custody_components",
            "custody_component_revision_delete_immutable",
        ),
        (
            "DELETE FROM custody_component_legs",
            "custody_component_leg_revision_delete_immutable",
        ),
        (
            "DELETE FROM custody_component_allocations",
            "custody_component_allocation_revision_delete_immutable",
        ),
        (
            "DELETE FROM custody_component_economic_terms",
            "custody_component_terms_delete_immutable",
        ),
        (
            "DELETE FROM custody_component_evidence_commitments",
            "custody_component_evidence_revision_delete_immutable",
        ),
    ] {
        let error = c.execute(sql, []).unwrap_err();
        assert!(error.to_string().contains(trigger), "{sql}: {error}");
    }

    let before = bytes(&dump(c, "custody_components"));
    let profile = bytes(&dump(c, "profiles"));
    c.execute_batch("BEGIN;INSERT INTO settings(key,value) VALUES('caller','retained');CREATE TRIGGER reject_pair_evidence BEFORE INSERT ON custody_component_evidence_commitments BEGIN SELECT RAISE(ABORT,'injected evidence refusal');END").unwrap();
    assert!(call(
        c,
        "transfers.pair",
        dict([("tx_out", s("f")), ("tx_in", s("g"))])
    )
    .is_err());
    assert!(!c.is_autocommit());
    assert_eq!(bytes(&dump(c, "custody_components")), before);
    assert_eq!(bytes(&dump(c, "profiles")), profile);
    assert_eq!(
        c.query_row("SELECT value FROM settings WHERE key='caller'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "retained"
    );
    c.execute_batch("ROLLBACK").unwrap();
    c.execute_batch("BEGIN").unwrap();
    call(
        c,
        "transfers.pair",
        dict([("tx_out", s("f")), ("tx_in", s("g"))]),
    )
    .unwrap();
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(bytes(&dump(c, "custody_components")), before);
}
#[test]
fn pair_writes_refuse_other_review_terms_and_profile_references() {
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    let fixture = kassiber_pytext::loads(include_str!("python.json")).unwrap();
    for sql in list(field(&fixture, "sql")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    assert_eq!(
        call(
            c,
            "transfers.pair",
            dict([("tx_out", s("tx3")), ("tx_in", s("tx1"))])
        )
        .unwrap_err()
        .code,
        "conflict"
    );
    let error = execute_with_sources(
        c,
        &Scope {
            workspace_id: "w",
            profile_id: "other",
        },
        "transfers.pair",
        &dict([("tx_out", s("tx0")), ("tx_in", s("tx1"))]),
        &Fixed {
            id: "no".into(),
            now: "2025-06-01T00:00:00Z".into(),
        },
    )
    .unwrap_err();
    assert_eq!(error.message, "Outbound transaction 'tx0' not found");
}

#[test]
fn pair_final_invalidation_refusals_are_atomic() {
    let (_dir, book) = fresh();
    let c = book.connection();
    call(
        c,
        "transfers.pair",
        dict([("tx_out", s("d")), ("tx_in", s("e"))]),
    )
    .unwrap();
    let version: i64 = c
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch(&format!("CREATE TRIGGER fail_final_invalidation BEFORE UPDATE ON profiles WHEN NEW.journal_input_version={} BEGIN SELECT RAISE(ABORT,'injected final invalidation refusal');END", version + 2)).unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let before: Vec<_> = tables.iter().map(|name| bytes(&dump(c, name))).collect();
    for (operation, args) in [
        (
            "transfers.pair",
            dict([("tx_out", s("f")), ("tx_in", s("g"))]),
        ),
        (
            "transfers.update",
            dict([("pair_id", s("guarded")), ("notes", s("revised"))]),
        ),
        ("transfers.unpair", dict([("pair_id", s("guarded"))])),
    ] {
        assert!(c.is_autocommit());
        assert!(call(c, operation, args).is_err(), "{operation}");
        assert!(c.is_autocommit());
        for (name, expected) in tables.iter().zip(&before) {
            assert_eq!(&bytes(&dump(c, name)), expected, "{operation}: {name}");
        }
    }
}

#[test]
fn payout_deletion_rolls_back_late_failure_and_preserves_outer_transaction() {
    let fixture = kassiber_pytext::loads(include_str!("payout_delete.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    let scenario = &list(field(&fixture, "scenarios"))[1];
    for sql in list(field(scenario, "sql")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let before: Vec<_> = tables.iter().map(|t| bytes(&dump(c, t))).collect();
    let version: i64 = c
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch(&format!("CREATE TEMP TRIGGER refuse_payout_invalidation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={} BEGIN SELECT RAISE(ABORT,'late payout invalidation'); END; BEGIN; INSERT INTO settings(key,value) VALUES('outer-payout','keep');",version+2)).unwrap();
    let case = &list(field(scenario, "cases"))[0];
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let fixed = Fixed {
        id: "unused".into(),
        now: "2025-06-01T00:00:00Z".into(),
    };
    assert!(execute_with_sources(
        c,
        &scope,
        "transfers.payouts.delete",
        field(case, "args"),
        &fixed
    )
    .is_err());
    assert!(!c.is_autocommit());
    c.execute("DELETE FROM settings WHERE key='outer-payout'", [])
        .unwrap();
    for (table, expected) in tables.iter().zip(&before) {
        assert_eq!(&bytes(&dump(c, table)), expected, "{table}");
    }
    c.execute_batch("DROP TRIGGER refuse_payout_invalidation")
        .unwrap();
    execute_with_sources(
        c,
        &scope,
        "transfers.payouts.delete",
        field(case, "args"),
        &fixed,
    )
    .unwrap();
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    for (table, expected) in tables.iter().zip(&before) {
        assert_eq!(&bytes(&dump(c, table)), expected, "{table}");
    }
}

#[test]
fn payout_creation_rolls_back_late_failure_and_preserves_outer_transaction() {
    let fixture = kassiber_pytext::loads(include_str!("payout_create.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    let scenario = &list(field(&fixture, "scenarios"))[0];
    for sql in list(field(scenario, "sql")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let before: Vec<_> = tables.iter().map(|t| bytes(&dump(c, t))).collect();
    let version: i64 = c
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id='p'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch(&format!("CREATE TEMP TRIGGER refuse_payout_creation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={} BEGIN SELECT RAISE(ABORT,'late payout creation invalidation'); END",version+2)).unwrap();
    let case = &list(field(scenario, "cases"))[0];
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let fixed = Fixed {
        id: py_str(field(case, "id")),
        now: py_str(field(case, "now")),
    };
    // The fixed Python oracle also pins complete rollback on this failure path.
    for outer in [false, true] {
        if outer {
            c.execute_batch("BEGIN; INSERT INTO settings(key,value) VALUES('outer-create','keep')")
                .unwrap();
        }
        let error = execute_with_sources(
            c,
            &scope,
            "transfers.payouts.create",
            field(case, "args"),
            &fixed,
        )
        .unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(c.is_autocommit(), !outer);
        if outer {
            assert_eq!(
                c.query_row(
                    "SELECT value FROM settings WHERE key='outer-create'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "keep"
            );
            c.execute_batch("ROLLBACK").unwrap();
        }
        for (table, expected) in tables.iter().zip(&before) {
            assert_eq!(&bytes(&dump(c, table)), expected, "{table}");
        }
    }
    c.execute_batch("DROP TRIGGER refuse_payout_creation; BEGIN")
        .unwrap();
    execute_with_sources(
        c,
        &scope,
        "transfers.payouts.create",
        field(case, "args"),
        &fixed,
    )
    .unwrap();
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    for (table, expected) in tables.iter().zip(&before) {
        assert_eq!(&bytes(&dump(c, table)), expected, "{table}");
    }
}

#[test]
fn component_apply_rolls_back_late_failure_and_preserves_outer_transaction() {
    let fixture = kassiber_pytext::loads(include_str!("components.json")).unwrap();
    let scenario = list(field(&fixture, "scenarios"))
        .iter()
        .find(|s| field(s, "name") == &s_("late-failure"))
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    for sql in list(field(scenario, "sql")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let before: Vec<_> = tables.iter().map(|t| bytes(&dump(c, t))).collect();
    let case = &list(field(scenario, "cases"))[0];
    for sql in list(field(case, "setup")) {
        c.execute_batch(&py_str(sql)).unwrap();
    }
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let fixed = Fixed {
        id: py_str(field(case, "id")),
        now: py_str(field(case, "now")),
    };
    // The first activation writes rows before the second invalidation fails.
    for outer in [false, true] {
        if outer {
            c.execute_batch(
                "BEGIN; INSERT INTO settings(key,value) VALUES('outer-component','keep')",
            )
            .unwrap();
        }
        let error = execute_with_sources(
            c,
            &scope,
            "transfers.components.apply",
            field(case, "args"),
            &fixed,
        )
        .unwrap_err();
        assert_eq!(error.message, "late component activation invalidation");
        assert_eq!(c.is_autocommit(), !outer);
        if outer {
            assert_eq!(
                c.query_row(
                    "SELECT value FROM settings WHERE key='outer-component'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "keep"
            );
            c.execute_batch("ROLLBACK").unwrap();
        }
        for (table, expected) in tables.iter().zip(&before) {
            assert_eq!(&bytes(&dump(c, table)), expected, "{table}");
        }
    }
    c.execute_batch("DROP TRIGGER refuse_component_activation; BEGIN")
        .unwrap();
    let applied = execute_with_sources(
        c,
        &scope,
        "transfers.components.apply",
        field(case, "args"),
        &fixed,
    )
    .unwrap();
    assert_eq!(list(field(&applied, "components")).len(), 2);
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    for (table, expected) in tables.iter().zip(&before) {
        assert_eq!(&bytes(&dump(c, table)), expected, "{table}");
    }
}
fn s_(text: &str) -> Value {
    s(text)
}
