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
    c.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
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
    c.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
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
    let fixture = kassiber_pytext::loads(include_str!("pairing_services.json")).unwrap();
    assert_eq!(list(field(&fixture, "scenarios")).len(), 255);
    assert_eq!(
        list(field(&fixture, "scenarios"))
            .iter()
            .map(|s| list(field(s, "cases")).len())
            .sum::<usize>(),
        1860
    );
    assert_eq!(
        py_str(field(&fixture, "vacuous_failure")),
        "seed 1, scenario deliberately-vacuous: distinct-input activity 0 < 1"
    );
    let dir = tempfile::tempdir().unwrap();
    let template = Book::create(dir.path(), None).unwrap();
    for scenario in list(field(&fixture, "scenarios")) {
        let name = py_str(field(scenario, "name"));
        let mut connection = Connection::open_in_memory().unwrap();
        kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut connection)
            .unwrap()
            .run_to_completion(128, std::time::Duration::ZERO, None)
            .unwrap();
        let c = &connection;
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
            let result = match op.as_str() {
                "suggest" => pairing::suggestions(c, &scope, &opts),
                "auto" => pairing::auto_pair_before_journals(c, &scope, &source),
                "rules" => pairing::apply_rules(c, &scope, &opts, None, &source),
                "bulk" => pairing::bulk_pair(c, &scope, &opts, None, &source),
                _ => panic!("operation"),
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
