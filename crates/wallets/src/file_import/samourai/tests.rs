use super::*;

fn corpus() -> Vec<Value> {
    let mut result: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/vectors/python_samourai_imports.json"
    ))
    .unwrap();
    let edges: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/vectors/python_samourai_codepoints.json"
    ))
    .unwrap();
    result.extend(edges);
    result
}
fn input(name: &str) -> String {
    corpus().into_iter().find(|c| c["name"] == name).unwrap()["text"]
        .as_str()
        .unwrap()
        .into()
}
fn fixture(encrypted: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    if encrypted {
        conn.execute_batch("PRAGMA key='disposable-test-key'")
            .unwrap();
    }
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let creation: Value =
        serde_json::from_str(include_str!("../../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
        INSERT INTO profiles(id,workspace_id,label,created_at,last_processed_at,last_processed_tx_count,journal_input_version) VALUES('p','ws','Book','2025-01-01','2025-01-02',3,7),('q','ws','Other','2025-01-01',NULL,0,0);
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,asset,created_at) VALUES('a','ws','p','treasury','Treasury','asset','BTC','2025-01-01'),('b','ws','q','treasury','Treasury','asset','BTC','2025-01-01');
        INSERT INTO settings(key,value) VALUES('context_workspace','ws'),('context_profile','p');").unwrap();
    conn
}
fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn file(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "samourai-{}-{:?}-{name}.json",
        std::process::id(),
        std::thread::current().id()
    ))
}
fn request(file: &std::path::Path) -> Request {
    Request {
        workspace: None,
        profile: None,
        label: "Samourai".into(),
        account: None,
        backend: Some(" MEMPOOL ".into()),
        network: None,
        gap_limit: Some(Py::from(10)),
        file: file.to_str().unwrap().into(),
        source_set: None,
    }
}

#[test]
fn python_samourai_import_vectors() {
    let path = file("vectors");
    for case in corpus() {
        let content = case
            .get("bytes")
            .map(|v| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| case["text"].as_str().unwrap().as_bytes().to_vec());
        std::fs::write(&path, content).unwrap();
        let gap = (!case["gap_limit"].is_null())
            .then(|| kassiber_pytext::loads(&case["gap_limit"].to_string()).unwrap());
        let actual = load_file(
            path.to_str().unwrap(),
            case["network"].as_str().unwrap(),
            gap.as_ref(),
        );
        if let Some(expected) = case["sources_json"].as_str() {
            let actual = actual.unwrap_or_else(|e| panic!("{}: {e:?}", case["name"]));
            if let Some(plans) = case["plans_json"].as_str() {
                let actual = actual
                    .iter()
                    .map(|v| {
                        load_plan(field(v, "config"))
                            .unwrap()
                            .unwrap()
                            .projection(0, 2)
                            .map_err(descriptors::Error::into_app)
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                let expected: Value = serde_json::from_str(plans).unwrap();
                assert_eq!(json!(actual), expected, "{}", case["name"]);
            }
            assert_eq!(
                kassiber_pyjson::dumps(&Py::List(actual), &Options::new().sort_keys(true)).unwrap(),
                expected,
                "{}",
                case["name"]
            );
        } else {
            let e = match actual {
                Err(e) => e,
                Ok(v) => panic!("{} accepted: {v:?}", case["name"]),
            };
            let actual = dict([
                ("code", s(e.code)),
                (
                    "message",
                    e.message_codepoints
                        .map_or_else(|| s(e.message), |v| Py::Str(*v)),
                ),
                ("hint", e.hint.map_or(Py::Null, |v| s(v.as_ref()))),
                ("details", e.details.unwrap_or(Py::Null)),
            ]);
            assert_eq!(
                kassiber_pyjson::dumps(&actual, &Options::new().sort_keys(true)).unwrap(),
                case["error_json"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn samourai_coordinator_preserves_source_scope_redaction_and_reference_readiness() {
    for encrypted in [false, true] {
        let conn = fixture(encrypted);
        let path = file("coordinator");
        std::fs::write(&path, input("mixed")).unwrap();
        let result = execute(&conn, request(&path), None).unwrap();
        let parent = field(&result, "group");
        let parent_id = py_str(field(parent, "id"));
        assert_eq!(field(parent, "kind"), &s("samourai"));
        assert_eq!(field(parent, "backend"), &s("mempool"));
        assert_eq!(
            field(field(field(parent, "config"), "samourai"), "watch_only"),
            &Py::Bool(true)
        );
        assert_eq!(field(parent, "gap_limit"), &Py::from(10));
        let Py::List(children) = field(&result, "children") else {
            panic!()
        };
        assert_eq!(children.len(), 2);
        for child in children {
            assert_eq!(field(child, "kind"), &s("descriptor"));
            assert_eq!(field(child, "backend"), &s("mempool"));
            let config = field(child, "config");
            assert_eq!(field(config, "descriptor"), &s("[redacted]"));
            assert_eq!(field(config, "change_descriptor"), &s("[redacted]"));
            assert_eq!(field(config, "gap_limit"), &Py::from(100));
            assert_eq!(
                field(field(config, "samourai"), "parent_wallet_id"),
                &s(&parent_id)
            );
            assert_eq!(
                field(field(config, "samourai"), "group_id"),
                field(field(field(parent, "config"), "samourai"), "group_id")
            );
            assert_eq!(field(child, "account_code"), &s("treasury"));
        }
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 3);
        let Py::List(warnings) = field(&result, "warnings") else {
            panic!()
        };
        assert_eq!(warnings.len(), 2);
        assert_eq!(field(&warnings[0], "code"), &s("bip47_not_auto_scanned"));
        assert_eq!(field(&warnings[1], "code"), &s("postmix_gap_watch"));
        assert_eq!(scalar(&conn, "SELECT count(*) FROM transactions"), 0);
        assert_eq!(scalar(&conn, "SELECT count(*) FROM import_batches"), 0);
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM wallet_policy_epochs"),
            0
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            7
        );
        assert_eq!(
            scalar(
                &conn,
                "SELECT last_processed_tx_count FROM profiles WHERE id='p'"
            ),
            3
        );
        let stored: String = conn
            .query_row(
                "SELECT config_json FROM wallets WHERE kind='descriptor' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stored.contains("xpub"));
        assert!(!kassiber_pyjson::dumps(&result, &Options::default())
            .unwrap()
            .contains("xpub"));
        assert_eq!(
            execute(&conn, request(&path), None).unwrap_err().code,
            "conflict"
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 3);
        let mut other = request(&path);
        other.profile = Some(s("q"));
        execute(&conn, other, None).unwrap();
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM wallets WHERE profile_id='q'"),
            3
        );
        std::fs::write(&path, input("duplicate-labels")).unwrap();
        let mut duplicate = request(&path);
        duplicate.label = "Duplicate".into();
        assert_eq!(
            execute(&conn, duplicate, None).unwrap_err().message,
            "Samourai import contains duplicate wallet labels"
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 6);
        std::fs::write(&path, input("descriptor-codepoints-label-14")).unwrap();
        let mut malformed = request(&path);
        malformed.label = "Unicode".into();
        let error = execute(&conn, malformed, None).unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert!(error.message.contains("surrogates not allowed"));
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 6);
        std::fs::write(&path, input("duplicate-label-codepoint-order")).unwrap();
        let error = execute(&conn, request(&path), None).unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(
            error.message,
            "Samourai import contains duplicate wallet labels"
        );
        assert!(
            matches!(field(error.details.as_ref().unwrap(), "duplicate_labels"), Py::List(v) if matches!(&v[0], Py::Str(label) if label.has_surrogates()))
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 6);
        std::fs::write(&path, input("label-binding-codepoint-order")).unwrap();
        let error = execute(&conn, request(&path), None).unwrap_err();
        assert_eq!(error.code, "internal_error");
        assert_eq!(
            error.message,
            "'utf-8' codec can't encode character '\\ud800' in position 15: surrogates not allowed"
        );
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 6);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn samourai_late_failure_rolls_back_group_children_and_readiness() {
    for encrypted in [false, true] {
        let conn = fixture(encrypted);
        let path = file("atomic");
        std::fs::write(&path, input("mixed")).unwrap();
        conn.execute_batch("CREATE TRIGGER refuse_child BEFORE INSERT ON wallets WHEN NEW.label LIKE '%Postmix' BEGIN SELECT RAISE(ABORT,'denied'); END;").unwrap();
        assert!(execute(&conn, request(&path), None).is_err());
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 0);
        assert_eq!(
            scalar(
                &conn,
                "SELECT journal_input_version FROM profiles WHERE id='p'"
            ),
            7
        );
        conn.execute_batch("DROP TRIGGER refuse_child").unwrap();
        execute(&conn, request(&path), None).unwrap();
        assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 3);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn samourai_file_and_source_refusals_precede_scope_and_never_write() {
    let conn = fixture(false);
    let path = file("missing");
    std::fs::remove_file(&path).ok();
    let mut r = request(&path);
    r.profile = Some(s("missing"));
    assert_eq!(execute(&conn, r, None).unwrap_err().code, "not_found");
    std::fs::write(&path, "{").unwrap();
    let mut r = request(&path);
    r.profile = Some(s("missing"));
    assert_eq!(
        execute(&conn, r, None).unwrap_err().message,
        "Samourai descriptor/xpub set is not valid JSON"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 0);
    std::fs::write(&path, input("mixed")).unwrap();
    if let Ok(home) = std::env::var("HOME") {
        if let Ok(tail) = path.strip_prefix(home) {
            let mut r = request(&path);
            r.file = format!("~/{}", tail.display());
            execute(&conn, r, None).unwrap();
            assert_eq!(scalar(&conn, "SELECT count(*) FROM wallets"), 3);
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn samourai_inline_source_sets_share_file_coordination_and_refusal_order() {
    for encrypted in [false, true] {
        let path = file("inline");
        let text = input("mixed");
        std::fs::write(&path, &text).unwrap();
        let payload = kassiber_pytext::loads(&text).unwrap();
        let identity = Identity {
            ids: ["group", "parent", "receive", "postmix"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            timestamp: "2025-01-02T03:04:05Z".into(),
        };
        let conn = fixture(encrypted);
        let file_result = execute(&conn, request(&path), Some(&identity)).unwrap();
        let inline_conn = fixture(encrypted);
        let mut inline = request(&path);
        inline.file.clear();
        inline.source_set = Some(payload.clone());
        let inline_result = execute(&inline_conn, inline, Some(&identity)).unwrap();
        assert_eq!(inline_result, file_result);
        let rows = |conn: &Connection| {
            let mut query = conn.prepare("SELECT * FROM wallets ORDER BY id").unwrap();
            let columns = query.column_count();
            query
                .query_map([], |row| {
                    (0..columns)
                        .map(|i| row.get::<_, kassiber_store::rusqlite::types::Value>(i))
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(rows(&inline_conn), rows(&conn));
        let mut both = request(&path);
        both.profile = Some(s("missing"));
        both.source_set = Some(payload);
        assert_eq!(
            execute(&inline_conn, both, None).unwrap_err().message,
            "Choose exactly one Samourai import source"
        );
        let mut missing = request(&path);
        missing.file.clear();
        missing.source_set = Some(Py::Null);
        assert_eq!(
            execute(&inline_conn, missing, None).unwrap_err().message,
            "Choose exactly one Samourai import source"
        );
        let mut invalid = request(&path);
        invalid.file.clear();
        invalid.profile = Some(s("missing"));
        invalid.source_set = Some(Py::List(vec![]));
        assert_eq!(
            execute(&inline_conn, invalid, None).unwrap_err().message,
            "Samourai descriptor/xpub set must be a JSON object"
        );
        assert_eq!(scalar(&inline_conn, "SELECT count(*) FROM wallets"), 3);
        assert_eq!(
            scalar(&inline_conn, "SELECT count(*) FROM import_batches"),
            0
        );
        std::fs::remove_file(path).unwrap();
    }
}
