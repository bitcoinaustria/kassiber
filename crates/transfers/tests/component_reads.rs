use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_store::{
    rusqlite::{types::ValueRef, Connection},
    Book,
};
use kassiber_transfers::{execute, Scope};

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
fn dump(c: &Connection) -> Value {
    let tables:Vec<String>=c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    dict(tables.iter().map(|t| {
        let mut stmt = c.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
        let names: Vec<_> = stmt.column_names().iter().map(|n| n.to_string()).collect();
        let mut cursor = stmt.query([]).unwrap();
        let mut result = Vec::new();
        while let Some(row) = cursor.next().unwrap() {
            result.push(dict(names.iter().enumerate().map(|(i, n)| {
                (
                    n.as_str(),
                    match row.get_ref(i).unwrap() {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => n.into(),
                        ValueRef::Real(n) => Value::Float(n),
                        ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                        ValueRef::Blob(_) => panic!("binary fixture"),
                    },
                )
            })));
        }
        result.sort_by_key(bytes);
        (t.as_str(), Value::List(result))
    }))
}

#[test]
fn component_reads_match_python_ordered_views_filters_and_complete_unchanged_database_rows() {
    assert_reads(false);
}

#[test]
fn component_read_filters_privacy_and_paging_match_python() {
    assert_reads(true);
}

fn restore(template: &Book, scenario: &Value) -> Connection {
    let name = py_str(field(scenario, "name"));
    let mut c = Connection::open_in_memory().unwrap();
    kassiber_store::rusqlite::backup::Backup::new(template.connection(), &mut c)
        .unwrap()
        .run_to_completion(1000, std::time::Duration::ZERO, None)
        .unwrap();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for t in tables {
        c.execute_batch(&format!("DELETE FROM \"{t}\"")).unwrap();
    }
    for q in list(field(scenario, "sql")) {
        c.execute_batch(&py_str(q))
            .unwrap_or_else(|e| panic!("{name}: {e}: {q:?}"));
    }
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    assert!(
        c.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none(),
        "{name}"
    );
    c
}

#[test]
fn component_desktop_boolean_validation_follows_transaction_resolution() {
    let fixture = kassiber_pytext::loads(include_str!("component_reads.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let template = Book::create(dir.path(), None).unwrap();
    let c = restore(&template, &list(field(&fixture, "scenarios"))[0]);
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    for (reference, code) in [
        ("absent", "app_error"),
        ("j-ext", "ambiguous_reference"),
        ("d-ext", "validation"),
    ] {
        let args = dict([
            ("transaction", s(reference)),
            ("effective_only", 1.into()),
            ("desktop_kind", s("ui.transfers.components.list")),
            ("limit", 0.into()),
        ]);
        assert_eq!(
            execute(&c, &scope, "transfers.components.list", &args)
                .unwrap_err()
                .code,
            code
        );
    }
}

fn assert_reads(focused: bool) {
    let fixture = kassiber_pytext::loads(include_str!("component_reads.json")).unwrap();
    let all = list(field(&fixture, "scenarios"));
    assert_eq!(all.len(), 51);
    let scenarios: Vec<_> = if focused {
        [
            "locations",
            "revise",
            "batch",
            "conflict",
            "native",
            "conversion",
            "draft",
            "draft-revision",
        ]
        .into_iter()
        .map(|prefix| {
            all.iter()
                .rev()
                .find(|s| py_str(field(s, "name")).starts_with(&format!("{prefix}-")))
                .unwrap()
        })
        .collect()
    } else {
        all.iter().collect()
    };
    let dir = tempfile::tempdir().unwrap();
    let template = Book::create(dir.path(), None).unwrap();
    let mut successes = 0;
    let mut refusals = 0;
    for scenario in scenarios {
        let name = py_str(field(scenario, "name"));
        let c = restore(&template, scenario);
        let before = bytes(&dump(&c));
        for case in list(field(scenario, "cases")) {
            let operation = py_str(field(case, "operation"));
            let profile = py_str(field(case, "profile"));
            let result = execute(
                &c,
                &Scope {
                    workspace_id: "w",
                    profile_id: &profile,
                },
                &operation,
                field(case, "args"),
            );
            if let Some(expected) = get(case, "result") {
                let actual = result.unwrap_or_else(|e| panic!("{name} {operation}: {e:?}"));
                assert_eq!(
                    dumps(&actual, &Options::default()).unwrap(),
                    dumps(expected, &Options::default()).unwrap(),
                    "{name} {operation} {:?}",
                    field(case, "args")
                );
                successes += 1;
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
                    "{name} {operation} {:?}",
                    field(case, "args")
                );
                refusals += 1;
            }
            assert_eq!(
                bytes(&dump(&c)),
                before,
                "{name} {operation}: read changed database"
            );
        }
    }
    assert!(successes >= if focused { 60 } else { 300 });
    assert!(refusals >= if focused { 30 } else { 100 });
}
