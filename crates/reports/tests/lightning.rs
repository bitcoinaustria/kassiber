use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{BigInt, PyStr, Value};
use kassiber_reports::lightning::{self, Channel, ConnectionRef, Routing, Snapshot};
use kassiber_store::{
    rusqlite::{types::ValueRef, Connection},
    Book,
};

fn v<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap()
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    match v(value, key) {
        Value::Str(s) => s.as_str().unwrap(),
        _ => panic!("expected fixture text: {key}"),
    }
}
fn string(value: &Value, key: &str) -> PyStr {
    match v(value, key) {
        Value::Str(s) => s.clone(),
        encoded @ Value::Dict(_) => {
            PyStr::from_code_points(list(v(encoded, "codepoints")).iter().map(|p| {
                if let Value::Int(n) = p {
                    u32::try_from(n.clone()).unwrap()
                } else {
                    panic!("expected code point")
                }
            }))
            .unwrap()
        }
        _ => panic!("expected fixture string: {key}"),
    }
}
fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(v) => v,
        _ => panic!("expected fixture list"),
    }
}
fn integer(value: &Value, key: &str) -> BigInt {
    text(value, key).parse().unwrap()
}
fn wire(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap() + "\n"
}
fn error(error: AppError) -> Value {
    dict([
        ("code", s(error.code)),
        ("message", s(error.message)),
        ("hint", error.hint.map(s).unwrap_or(Value::Null)),
        ("details", error.details.unwrap_or(Value::Null)),
        ("retryable", Value::Bool(error.retryable)),
    ])
}
fn connection() -> ConnectionRef {
    ConnectionRef {
        id: s("node"),
        label: s("Café node"),
        kind: "coreln".into(),
        profile_id: s("p"),
        backend_name: None,
    }
}
fn report(case: &Value) -> lightning::Report {
    let raw = v(case, "snapshot");
    let routing = match v(raw, "routing") {
        Value::Null => None,
        r => Some(Routing {
            window_label: string(r, "window_label"),
            routing_revenue_sat: integer(r, "routing_revenue_sat"),
            payment_cost_sat: integer(r, "payment_cost_sat"),
            rebalance_cost_sat: integer(r, "rebalance_cost_sat"),
            onchain_cost_sat: integer(r, "onchain_cost_sat"),
            net_profit_sat: integer(r, "net_profit_sat"),
            forward_count: integer(r, "forward_count"),
            payment_count: integer(r, "payment_count"),
            rebalance_count: integer(r, "rebalance_count"),
        }),
    };
    let channels = list(v(raw, "channels"))
        .iter()
        .map(|c| Channel {
            id: string(c, "id"),
            peer_alias: string(c, "peer_alias"),
            capacity_sat: integer(c, "capacity_sat"),
            earned_routing_sat: (!matches!(v(c, "earned_routing_sat"), Value::Null))
                .then(|| integer(c, "earned_routing_sat")),
        })
        .collect();
    lightning::build_report(
        connection(),
        Snapshot { routing, channels },
        integer(case, "open_cost_sat"),
    )
}

/// Compare every typed value, including empty tables and SQL NULLs, without
/// rerunning compatibility code or normalizing identities after a read/write.
fn tables(conn: &Connection) -> Value {
    let names: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(names.len(), 113);
    dict(names.iter().map(|name| {
        let mut statement = conn.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
        let width = statement.column_count();
        let mut records: Vec<_> = statement
            .query_map([], |r| {
                Ok(Value::List(
                    (0..width)
                        .map(|i| match r.get_ref(i).unwrap() {
                            ValueRef::Null => Value::Null,
                            ValueRef::Integer(n) => Value::Int(n.into()),
                            ValueRef::Real(n) => Value::Float(n),
                            ValueRef::Text(t) => s(std::str::from_utf8(t).unwrap()),
                            ValueRef::Blob(_) => panic!("unexpected fixture blob"),
                        })
                        .collect(),
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        records.sort_by_key(wire);
        (name.as_str(), Value::List(records))
    }))
}

#[test]
fn lightning_report_vectors_pin_wire_csv_rounding_and_all_tables() {
    let corpus = kassiber_pytext::loads(include_str!("data/lightning.json")).unwrap();
    let cases = list(v(&corpus, "reports"));
    assert!(cases.len() >= 140);
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    conn.execute_batch(include_str!("data/book.sql")).unwrap();
    conn.execute_batch(text(&corpus, "seed")).unwrap();
    let expected_tables = text(&corpus, "tables_wire");
    assert_eq!(wire(&tables(conn)), expected_tables);
    let path = root.path().join("export.csv");
    for case in cases {
        let name = text(case, "name");
        let expected = v(case, "expected");
        let report = report(case);
        assert_eq!(
            wire(&report.payload()),
            text(expected, "payload_wire"),
            "{name}"
        );
        let envelope = kassiber_envelope::build_envelope(
            "reports.lightning-profitability",
            report.payload(),
            vec![],
        )
        .unwrap();
        assert_eq!(wire(&envelope), text(expected, "envelope_wire"), "{name}");
        if get(expected, "rows_wire").is_some() {
            let rows = Value::List(
                report
                    .csv_rows()
                    .unwrap()
                    .into_iter()
                    .map(|row| Value::List(row.into_iter().map(Value::Str).collect()))
                    .collect(),
            );
            assert_eq!(wire(&rows), text(expected, "rows_wire"), "{name}");
        }
        std::fs::write(&path, b"previous contents must be truncated\n").unwrap();
        let result = report.export_csv(&path);
        match result {
            Ok(metadata) => {
                assert!(
                    get(expected, "export_wire").is_some(),
                    "{name}: unexpected export success: {}",
                    wire(&metadata)
                );
                assert_eq!(
                    wire(&metadata).replace(root.path().to_str().unwrap(), "<root>"),
                    text(expected, "export_wire"),
                    "{name}"
                );
            }
            Err(e) => {
                assert!(
                    get(expected, "error_wire").is_some(),
                    "{name}: unexpected export refusal: {}",
                    e.message
                );
                assert_eq!(wire(&error(e)), text(expected, "error_wire"), "{name}");
            }
        }
        let expected_bytes = text(expected, "file_hex")
            .as_bytes()
            .chunks(2)
            .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(std::fs::read(&path).unwrap(), expected_bytes, "{name}");
        assert_eq!(wire(&tables(conn)), expected_tables, "{name}");
    }
}

#[test]
fn lightning_connection_vectors_pin_scope_ambiguity_config_and_all_tables() {
    let corpus = kassiber_pytext::loads(include_str!("data/lightning.json")).unwrap();
    let cases = list(v(&corpus, "connections"));
    assert!(cases.len() >= 30);
    let root = tempfile::tempdir().unwrap();
    let book = Book::create(&root.path().join("data"), None).unwrap();
    let conn = book.connection();
    for case in cases {
        let name = text(case, "name");
        conn.execute_batch("SAVEPOINT lightning_fixture").unwrap();
        conn.execute_batch(include_str!("data/book.sql")).unwrap();
        conn.execute_batch(text(&corpus, "seed")).unwrap();
        conn.execute_batch(text(case, "sql")).unwrap();
        let before = wire(&tables(conn));
        assert_eq!(before, text(case, "tables_wire"), "{name}");
        let args = kassiber_pytext::loads(text(case, "args_wire")).unwrap();
        let actual = match lightning::resolve_connection(conn, &args) {
            Ok(c) => dict([(
                "ok",
                dict([
                    ("id", c.id),
                    ("label", c.label),
                    ("kind", s(c.kind)),
                    ("profile_id", c.profile_id),
                    (
                        "backend_name",
                        c.backend_name.map(Value::Str).unwrap_or(Value::Null),
                    ),
                ]),
            )]),
            Err(e) => dict([("error", error(e))]),
        };
        assert_eq!(wire(&actual), text(case, "expected_wire"), "{name}");
        assert_eq!(wire(&tables(conn)), before, "{name}");
        conn.execute_batch("ROLLBACK TO lightning_fixture; RELEASE lightning_fixture")
            .unwrap();
    }
}

#[test]
fn lightning_daemon_window_vectors_preserve_coercion_and_infinity_error() {
    let corpus = kassiber_pytext::loads(include_str!("data/lightning.json")).unwrap();
    for case in list(v(&corpus, "windows")) {
        let input = kassiber_pytext::loads(text(case, "input")).unwrap();
        let result = match lightning::daemon_window_days(&input) {
            Ok(n) => dict([("ok", Value::Int(n.into()))]),
            Err(e) => dict([("error", error(e))]),
        };
        assert_eq!(
            wire(&result),
            text(case, "expected_wire"),
            "{}",
            text(case, "input")
        );
    }
}

#[test]
fn lightning_csv_publication_preserves_direct_open_and_no_registration() {
    let corpus = kassiber_pytext::loads(include_str!("data/lightning.json")).unwrap();
    let report = report(&list(v(&corpus, "reports"))[0]);
    assert_eq!(lightning::DEFAULT_OPEN_COST_SAT, 2500);
    let root = tempfile::tempdir().unwrap();
    let absent = root.path().join("missing/export.csv");
    let error = report.export_csv(&absent).unwrap_err();
    assert_eq!(error.code, "internal_error");
    assert!(error
        .message
        .starts_with("[Errno 2] No such file or directory:"));
    assert!(!absent.parent().unwrap().exists());
    let error = report.export_csv(root.path()).unwrap_err();
    assert!(error.message.starts_with("[Errno 21] Is a directory:"));
    #[cfg(unix)]
    {
        let target = root.path().join("target.csv");
        let link = root.path().join("link.csv");
        std::fs::write(&target, b"old data").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let metadata = report.export_csv(&link).unwrap();
        assert!(link.is_symlink());
        assert_eq!(
            get(&metadata, "file"),
            Some(&s(link.to_string_lossy().into_owned()))
        );
        assert_eq!(std::fs::read(target).unwrap(), std::fs::read(link).unwrap());
    }
}
