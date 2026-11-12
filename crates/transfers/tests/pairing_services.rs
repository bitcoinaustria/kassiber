use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{
    rusqlite::{types::ValueRef, Connection},
    Book,
};
use kassiber_transfers::{pairing, Scope, Sources};
use std::cell::Cell;
fn list(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        &[]
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
    next: Cell<u128>,
}
impl Sources for Fixed {
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        let n = self.next.get();
        self.next.set(n + 1);
        let h = format!("{n:032x}");
        Ok(format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        ))
    }
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-10-30T12:00:00Z".into())
    }
}
fn dump(c: &Connection) -> Value {
    let tables:Vec<String>=c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    dict(tables.iter().map(|table| {
        let mut stmt = c
            .prepare(&format!("SELECT * FROM \"{}\"", table.replace('"', "\"\"")))
            .unwrap();
        let names: Vec<_> = stmt.column_names().iter().map(|n| n.to_string()).collect();
        let mut cur = stmt.query([]).unwrap();
        let mut rows = Vec::new();
        while let Some(r) = cur.next().unwrap() {
            rows.push(dict(names.iter().enumerate().map(|(i, n)| {
                (
                    n.as_str(),
                    match r.get_ref(i).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => Value::Int(n.into()),
                        ValueRef::Real(n) => Value::Float(n),
                        ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                        ValueRef::Blob(_) => panic!("binary fixture"),
                    },
                )
            })));
        }
        rows.sort_by_key(bytes);
        (table.as_str(), Value::List(rows))
    }))
}
fn options() -> pairing::PairingOptions {
    let mut o = pairing::PairingOptions::default();
    o.matching.now = "2026-10-30T12:00:00Z".into();
    o
}
fn restore(c: &Connection, scenario: &Value) {
    let name = py_str(field(scenario, "name"));
    c.execute_batch("PRAGMA foreign_keys=OFF; BEGIN").unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for t in tables {
        c.execute_batch(&format!("DELETE FROM \"{}\"", t.replace('"', "\"\"")))
            .unwrap();
    }
    for q in list(field(scenario, "sql")) {
        c.execute_batch(&py_str(q))
            .unwrap_or_else(|e| panic!("{name}: {e}: {q:?}"));
    }
    c.execute_batch("COMMIT; PRAGMA foreign_keys=ON;").unwrap();
    assert!(c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}
#[test]
fn pairing_services_match_python_seeded_complete_database_rows_and_ordered_results() {
    assert_services(false, false);
}

#[test]
fn pairing_stage_matches_python_complete_database_dumps_on_sqlcipher() {
    assert_services(false, true);
}

#[test]
fn pairing_command_routes_match_python_complete_database_rows_and_ordered_results() {
    assert_services(true, false);
}

fn assert_services(commands: bool, encrypted: bool) {
    let fixture = kassiber_pytext::loads(include_str!("pairing_services.json")).unwrap();
    assert_eq!(list(field(&fixture, "scenarios")).len(), 281);
    assert_eq!(
        list(field(&fixture, "scenarios"))
            .iter()
            .map(|s| list(field(s, "cases")).len())
            .sum::<usize>(),
        2349
    );
    assert_eq!(
        py_str(field(&fixture, "vacuous_failure")),
        "seed 1, scenario deliberately-vacuous: distinct-input activity 0 < 1"
    );
    let dir = tempfile::tempdir().unwrap();
    let template = Book::create(
        dir.path(),
        encrypted.then_some("synthetic-pairing-test-only"),
    )
    .unwrap();
    for scenario in list(field(&fixture, "scenarios")) {
        let name = py_str(field(scenario, "name"));
        let mut connection = Connection::open_in_memory().unwrap();
        let c = if encrypted {
            template.connection()
        } else {
            kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut connection)
                .unwrap()
                .run_to_completion(128, std::time::Duration::ZERO, None)
                .unwrap();
            &connection
        };
        restore(c, scenario);
        let source = Fixed {
            next: Cell::new(py_str(field(scenario, "uuid_start")).parse().unwrap()),
        };
        let scope = Scope {
            workspace_id: "w",
            profile_id: "p",
        };
        for case in list(field(scenario, "cases")) {
            let op = py_str(field(case, "operation"));
            let mut opts = options();
            if let Some(fields) = get(case, "options") {
                for (k, target) in [
                    ("confidence", &mut opts.confidence),
                    ("asset_pair", &mut opts.asset_pair),
                    ("route_pair", &mut opts.route_pair),
                    ("method", &mut opts.method),
                    ("candidate_type", &mut opts.candidate_type),
                ] {
                    *target = get(fields, k).map(py_str);
                }
            }
            let result = if commands && op != "auto" {
                let args = get(case, "options")
                    .cloned()
                    .unwrap_or_else(|| dict::<&str>([]));
                kassiber_transfers::execute_with_sources(
                    c,
                    &scope,
                    match op.as_str() {
                        "suggest" => "transfers.suggest",
                        "rules" => "transfers.rules.apply",
                        "bulk" => "transfers.bulk-pair",
                        _ => panic!("operation"),
                    },
                    &args,
                    &source,
                )
            } else {
                match op.as_str() {
                    "suggest" => pairing::suggestions(c, &scope, &opts),
                    "auto" => pairing::auto_pair_before_journals(c, &scope, &source),
                    "rules" => pairing::apply_rules(c, &scope, &opts, None, &source),
                    "bulk" => pairing::bulk_pair(c, &scope, &opts, None, &source),
                    _ => panic!("operation"),
                }
            };
            if let Some(expected) = get(case, "result") {
                let actual = result.unwrap_or_else(|e| panic!("{name} {op}: {e:?}"));
                assert_eq!(
                    dumps(&actual, &Options::default()).unwrap(),
                    dumps(expected, &Options::default()).unwrap(),
                    "{name} {op}: result"
                );
                if !c.is_autocommit() {
                    c.execute_batch("COMMIT").unwrap();
                }
            } else {
                let e = result.unwrap_err();
                let actual = dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                ]);
                assert_eq!(
                    bytes(&actual),
                    bytes(field(case, "error")),
                    "{name} {op}: error"
                );
                if !c.is_autocommit() {
                    c.execute_batch("ROLLBACK").unwrap();
                }
            }
            let expected = field(field(&fixture, "states"), &py_str(field(case, "state")));
            let actual = dump(c);
            // Compare each named table so a failure stays concise and actionable.
            if let Value::Dict(fields) = expected {
                for (k, v) in fields {
                    let kassiber_pyjson::Key::Str(k) = k else {
                        panic!()
                    };
                    let k = k.as_str().unwrap();
                    assert_eq!(bytes(field(&actual, k)), bytes(v), "{name} {op}: {k}");
                }
            }
            assert_eq!(
                bytes(&actual),
                bytes(expected),
                "{name} {op}: complete dump"
            );
        }
    }
}

#[test]
fn pairing_commands_preserve_dry_runs_caller_transactions_and_late_failure_rollback() {
    let fixture = kassiber_pytext::loads(include_str!("pairing_services.json")).unwrap();
    let scenario = list(field(&fixture, "scenarios"))
        .iter()
        .find(|s| py_str(field(s, "name")) == "1-multi-wallet")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    restore(c, scenario);
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let source = Fixed { next: Cell::new(1) };
    let before = bytes(&dump(c));
    let preview = kassiber_transfers::execute_with_sources(
        c,
        &scope,
        "transfers.bulk-pair",
        &dict([("dry_run", Value::Bool(true))]),
        &source,
    )
    .unwrap();
    assert_eq!(field(field(&preview, "summary"), "count"), &2.into());
    assert_eq!(get(&preview, "dry_run"), Some(&Value::Bool(true)));
    assert_eq!(bytes(&dump(c)), before);
    assert!(c.is_autocommit());
    c.execute_batch("BEGIN; UPDATE profiles SET label='Outer edit' WHERE id='p'; CREATE TEMP TRIGGER fail_second_pair BEFORE INSERT ON custody_components WHEN (SELECT COUNT(*) FROM custody_components)>0 BEGIN SELECT RAISE(ABORT,'late second pair failure'); END;").unwrap();
    let outer = bytes(&dump(c));
    let error = kassiber_transfers::execute_with_sources(
        c,
        &scope,
        "transfers.bulk-pair",
        &dict::<&str>([]),
        &source,
    )
    .unwrap_err();
    assert_eq!(error.message, "late second pair failure");
    assert_eq!(bytes(&dump(c)), outer);
    assert!(!c.is_autocommit());
    c.execute_batch("DROP TRIGGER fail_second_pair").unwrap();
    kassiber_transfers::execute_with_sources(
        c,
        &scope,
        "transfers.bulk-pair",
        &dict::<&str>([]),
        &source,
    )
    .unwrap();
    assert!(!c.is_autocommit());
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(bytes(&dump(c)), before);
    kassiber_transfers::execute_with_sources(
        c,
        &scope,
        "transfers.bulk-pair",
        &dict::<&str>([]),
        &source,
    )
    .unwrap();
    assert!(c.is_autocommit());
    assert_ne!(bytes(&dump(c)), before);
}

#[test]
fn pairing_command_filters_numeric_options_and_authorship_match_python() {
    let fixture = kassiber_pytext::loads(include_str!("pairing_services.json")).unwrap();
    let scenario = list(field(&fixture, "scenarios"))
        .iter()
        .find(|s| py_str(field(s, "name")) == "1-tolerance")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    restore(c, scenario);
    c.execute_batch(
        "UPDATE transactions SET occurred_at='2026-03-14T17:30:01Z' WHERE direction='inbound'",
    )
    .unwrap();
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let source = Fixed { next: Cell::new(1) };
    let run = |args| {
        kassiber_transfers::execute_with_sources(c, &scope, "transfers.suggest", &args, &source)
            .unwrap()
    };
    for (args, message) in [
        (
            dict([(
                "time_window_seconds",
                Value::Int(format!("1{}", "0".repeat(400)).parse().unwrap()),
            )]),
            "int too large to convert to float",
        ),
        (
            dict([("asset_pair", Value::List(vec![s("BTC")]))]),
            "'list' object has no attribute 'split'",
        ),
        (
            dict([("route_pair", 1.into())]),
            "'int' object has no attribute 'split'",
        ),
    ] {
        let error = kassiber_transfers::execute_with_sources(
            c,
            &scope,
            "transfers.suggest",
            &args,
            &source,
        )
        .unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(error.message, message);
    }
    assert_eq!(
        field(field(&run(dict::<&str>([])), "counts"), "total"),
        &1.into()
    );
    assert_eq!(
        field(
            field(
                &run(dict([
                    ("fee_pct_max", Value::Float(1.0)),
                    ("fee_sats_min", 0.into())
                ])),
                "counts"
            ),
            "total"
        ),
        &2.into()
    );
    for args in [
        dict([("time_window_seconds", 0.into())]),
        dict([
            ("fee_pct_max", Value::Float(0.0)),
            ("fee_sats_min", 0.into()),
        ]),
        dict([("confidence", s("exact"))]),
        dict([("method", s("payment_hash"))]),
        dict([("asset_pair", s("BTC-LBTC"))]),
        dict([("route_pair", s("BTC-LNBTC"))]),
        dict([("candidate_type", s("swap"))]),
    ] {
        assert_eq!(
            field(field(&run(args.clone()), "counts"), "total"),
            &0.into(),
            "{args:?}"
        );
    }
    let result = kassiber_transfers::execute_with_sources(
        c,
        &scope,
        "transfers.bulk-pair",
        &dict([("confidence", s("strong")), ("authored_source", s("gui"))]),
        &source,
    )
    .unwrap();
    assert_eq!(field(field(&result, "summary"), "count"), &1.into());
    assert_eq!(
        c.query_row("SELECT authored_source FROM custody_components", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "gui"
    );
}

#[test]
fn pairing_auto_pair_rolls_back_late_failure_and_leaves_commit_to_caller() {
    let fixture = kassiber_pytext::loads(include_str!("pairing_services.json")).unwrap();
    let scenario = list(field(&fixture, "scenarios"))
        .iter()
        .find(|s| py_str(field(s, "name")) == "1-multi-wallet")
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let book = Book::create(dir.path(), None).unwrap();
    let c = book.connection();
    restore(c, scenario);
    c.execute_batch("BEGIN IMMEDIATE; UPDATE profiles SET label='Outer edit' WHERE id='p'; CREATE TEMP TRIGGER fail_pairing_cache BEFORE UPDATE OF swap_candidate_count ON profiles BEGIN SELECT RAISE(ABORT,'late pairing failure'); END;").unwrap();
    let before = bytes(&dump(c));
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let source = Fixed { next: Cell::new(1) };
    let error = pairing::auto_pair_before_journals(c, &scope, &source).unwrap_err();
    assert_eq!(error.message, "late pairing failure");
    assert!(!c.is_autocommit());
    assert_eq!(before, bytes(&dump(c)));
    c.execute_batch("DROP TRIGGER fail_pairing_cache; COMMIT")
        .unwrap();
    let before = bytes(&dump(c));
    let result =
        pairing::auto_pair_before_journals(c, &scope, &Fixed { next: Cell::new(1) }).unwrap();
    assert_eq!(py_str(field(&result, "applied")), "2");
    assert!(!c.is_autocommit());
    assert_ne!(before, bytes(&dump(c)));
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(before, bytes(&dump(c)));
}

#[test]
fn pairing_stage_repeats_match_python_and_outer_journal_failure_reverts_every_table() {
    let fixture = kassiber_pytext::loads(include_str!("pairing_services.json")).unwrap();
    let scenario = list(field(&fixture, "scenarios"))
        .iter()
        .find(|s| py_str(field(s, "name")) == "1-multi-wallet")
        .unwrap();
    for encrypted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let book = Book::create(
            dir.path(),
            encrypted.then_some("synthetic-pairing-test-only"),
        )
        .unwrap();
        let c = book.connection();
        restore(c, scenario);
        let before = bytes(&dump(c));
        let scope = Scope {
            workspace_id: "w",
            profile_id: "p",
        };
        let source = Fixed { next: Cell::new(1) };
        c.execute_batch("BEGIN IMMEDIATE; SAVEPOINT journal_pipeline")
            .unwrap();
        let first = pairing::auto_pair_before_journals(c, &scope, &source).unwrap();
        assert_eq!(field(&first, "applied"), &2.into());
        let second = pairing::auto_pair_before_journals(c, &scope, &source).unwrap();
        assert_eq!(field(&second, "applied"), &0.into());
        assert_eq!(field(field(&second, "remaining"), "total"), &0.into());
        let expected = &list(field(scenario, "cases"))[2];
        assert_eq!(bytes(&second), bytes(field(expected, "result")));
        let expected_state = field(field(&fixture, "states"), &py_str(field(expected, "state")));
        assert_eq!(bytes(&dump(c)), bytes(expected_state));
        assert!(!c.is_autocommit());
        c.execute_batch("ROLLBACK TO journal_pipeline; RELEASE journal_pipeline; COMMIT")
            .unwrap();
        assert_eq!(before, bytes(&dump(c)));
    }
}
