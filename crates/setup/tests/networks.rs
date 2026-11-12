//! Ports of book-network service policy and consistent-read regressions.
mod support;
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_setup::networks::{self, partition};
use support::*;
fn member<'a>(v: &'a Value, k: &str) -> &'a Value {
    get(v, k).unwrap()
}
fn canonical(v: &Value) -> String {
    dumps(
        v,
        &Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .unwrap()
}
fn data() -> Value {
    kassiber_pytext::loads(include_str!("../fixtures/python_networks.json")).unwrap()
}
fn seed(index: usize) -> Fixture {
    let f = Fixture::new();
    let v = data();
    let Value::List(scenarios) = member(&v, "scenarios") else {
        panic!()
    };
    let Value::List(sql) = member(&scenarios[index], "seed") else {
        panic!()
    };
    f.book
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF; BEGIN")
        .unwrap();
    for sql in sql {
        f.book.connection().execute_batch(&py_str(sql)).unwrap();
    }
    f.book
        .connection()
        .execute_batch("COMMIT; PRAGMA foreign_keys=ON")
        .unwrap();
    f
}
fn args(environment: &str) -> Value {
    dict([
        ("environment", s(environment)),
        (
            "declared_wallet_ids",
            Value::List(
                ["wallet-a", "wallet-b", "wallet-c"]
                    .into_iter()
                    .map(s)
                    .collect(),
            ),
        ),
    ])
}
fn bound(f: &Fixture, environment: &str, instance: Option<&str>) -> Value {
    let mut args = args(environment);
    if let Value::Dict(v) = &mut args {
        v.push(("chain_instance_id".into(), instance.map_or(Value::Null, s)));
    }
    let plan = networks::plan(f.book.connection(), "profile-1", &args).unwrap();
    assert_eq!(member(&plan, "can_apply"), &Value::Bool(true));
    if let Value::Dict(v) = &mut args {
        v.push(("plan_id".into(), member(&plan, "plan_id").clone()));
    }
    networks::bind(f.book.connection(), "profile-1", &args, &f.sources).unwrap()
}
#[test]
fn network_reference_vectors_match_python() {
    let v = data();
    let Value::List(scenarios) = member(&v, "scenarios") else {
        panic!()
    };
    for (index, scenario) in scenarios.iter().enumerate() {
        let f = seed(index);
        let Value::List(cases) = member(scenario, "cases") else {
            panic!()
        };
        for case in cases {
            let args = member(case, "args");
            let result = match text(case, "operation").as_str() {
                "inventory" => networks::inventory(f.book.connection(), "profile-1"),
                "plan" => networks::plan(f.book.connection(), "profile-1", args),
                "split-plan" => partition::plan(f.book.connection(), "profile-1", args),
                _ => panic!(),
            };
            match result {
                Ok(actual) => assert_eq!(
                    canonical(&actual),
                    canonical(member(case, "ok")),
                    "{} {} {}",
                    text(scenario, "scenario"),
                    text(case, "operation"),
                    canonical(args)
                ),
                Err(error) => {
                    let expected = member(case, "error");
                    assert_eq!(error.code, text(expected, "code"));
                    assert_eq!(error.message, text(expected, "message"));
                    assert_eq!(error.hint.as_deref(), Some(text(expected, "hint").as_str()));
                    assert_eq!(
                        canonical(&error.details.unwrap_or(Value::Null)),
                        canonical(member(expected, "details"))
                    );
                }
            }
        }
    }
}
#[test]
fn network_evidence_matches_python_including_absence_and_contradictions() {
    let v = data();
    let Value::List(cases) = member(&v, "evidence") else {
        panic!()
    };
    for case in cases {
        let (environment, instance, valid) = networks::evidence(member(case, "row"));
        assert_eq!(
            Value::List(vec![
                environment.map_or(Value::Null, s),
                instance.map_or(Value::Null, s),
                Value::Bool(valid)
            ]),
            *member(case, "expected"),
            "{}",
            canonical(member(case, "row"))
        );
    }
}
#[test]
fn network_bind_recomputes_history_and_invalidates_journals_atomically() {
    let f = seed(1);
    let conn = f.book.connection();
    let mut args = args("main");
    let plan = networks::plan(conn, "profile-1", &args).unwrap();
    if let Value::Dict(v) = &mut args {
        v.push(("plan_id".into(), member(&plan, "plan_id").clone()));
    }
    conn.execute("UPDATE wallets SET label='Changed' WHERE id='wallet-a'", [])
        .unwrap();
    assert_eq!(
        networks::bind(conn, "profile-1", &args, &f.sources)
            .unwrap_err()
            .code,
        "stale_context"
    );
    assert_eq!(
        text(&networks::resolve(conn, "profile-1").unwrap(), "state"),
        "unbound"
    );
    conn.execute("UPDATE profiles SET last_processed_at='old',last_processed_tx_count=10,ownership_review_counts_json='{}'",[]).unwrap();
    let binding = bound(&f, "main", None);
    assert_eq!(text(&binding, "environment"), "main");
    let facts=conn.query_row("SELECT last_processed_at,last_processed_tx_count,journal_input_version,ownership_review_counts_json FROM profiles",[],|r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<String>>(3)?))).unwrap();
    assert_eq!(facts, (None, 0, 1, None));
    assert!(conn
        .execute("UPDATE book_network_bindings SET environment='test'", [])
        .is_err());
    let plan = networks::plan(conn, "profile-1", &args).unwrap();
    if let Value::Dict(v) = &mut args {
        v.retain(|(k, _)| k != &"plan_id".into());
        v.push(("plan_id".into(), member(&plan, "plan_id").clone()));
    }
    assert_eq!(
        networks::bind(conn, "profile-1", &args, &f.sources)
            .unwrap_err()
            .code,
        "book_network_review_required"
    );
}
#[test]
fn network_unknown_declaration_cannot_override_conflicts_or_other_book_ids() {
    let f = seed(0);
    let conn = f.book.connection();
    let before: String = conn
        .query_row("SELECT config_json FROM wallets LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let plan = networks::plan(conn, "profile-1", &dict([("environment", s("main"))])).unwrap();
    assert_eq!(member(&plan, "can_apply"), &Value::Bool(false));
    bound(&f, "main", None);
    assert_eq!(
        conn.query_row("SELECT config_json FROM wallets LIMIT 1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        before
    );
    let f = seed(3);
    assert_eq!(
        member(
            &networks::plan(f.book.connection(), "profile-1", &args("main")).unwrap(),
            "can_apply"
        ),
        &Value::Bool(false)
    );
    assert_eq!(
        networks::require_accounting(f.book.connection(), "profile-1")
            .unwrap_err()
            .code,
        "book_network_review_required"
    );
}
#[test]
fn network_regtest_cache_requires_exact_instance_and_never_relabels_wallet() {
    let f = seed(0);
    let conn = f.book.connection();
    let binding = bound(&f, "regtest", Some("00000000-0000-0000-0000-000000000001"));
    let row = dict([("config_json", dict([("network", s("regtest"))]))]);
    assert!(networks::observation_matches_binding(&binding, &row, false));
    assert!(!networks::observation_matches_binding(&binding, &row, true));
    let wrong = dict([(
        "config_json",
        dict([
            ("network", s("regtest")),
            ("chain_instance_id", s("different")),
        ]),
    )]);
    assert!(!networks::observation_matches_binding(
        &binding, &wrong, false
    ));
    assert_eq!(
        networks::guard_observation(conn, "profile-1", &wrong, "import")
            .unwrap_err()
            .code,
        "book_network_mismatch"
    );
    let old = dict([("network", s("main"))]);
    let new = dict([("network", s("regtest"))]);
    assert!(networks::guard_wallet(conn, "profile-1", &new, Some(&old), "wallet").is_err());
}
#[test]
fn network_chain_domains_preserve_bitcoin_lightning_liquid_and_signet_rules() {
    let f = seed(0);
    let conn = f.book.connection();
    assert_eq!(
        networks::require_chain_domain(conn, "profile-1", "bitcoin", &s("main"), None, "analysis")
            .unwrap_err()
            .code,
        "book_network_unbound"
    );
    bound(&f, "main", None);
    let btc =
        networks::require_chain_domain(conn, "profile-1", "bitcoin", &s("main"), None, "analysis")
            .unwrap();
    let ln = networks::require_chain_domain(
        conn,
        "profile-1",
        "lightning",
        &s("mainnet"),
        None,
        "analysis",
    )
    .unwrap();
    let liquid = networks::require_chain_domain(
        conn,
        "profile-1",
        "liquid",
        &s("liquidv1"),
        None,
        "analysis",
    )
    .unwrap();
    assert_eq!(member(&btc, "domain_id"), member(&ln, "domain_id"));
    assert_ne!(member(&btc, "domain_id"), member(&liquid, "domain_id"));
    assert!(networks::require_chain_domain(
        conn,
        "profile-1",
        "bitcoin",
        &s("regtest"),
        None,
        "analysis"
    )
    .is_err());
    conn.execute(
        "UPDATE wallets SET config_json='{\"network\":\"regtest\"}' WHERE id='wallet-a'",
        [],
    )
    .unwrap();
    assert_eq!(
        networks::require_accounting(conn, "profile-1")
            .unwrap_err()
            .code,
        "book_network_review_required"
    );
    let f = seed(0);
    bound(&f, "signet", None);
    assert!(networks::require_chain_domain(
        f.book.connection(),
        "profile-1",
        "liquid",
        &s("liquidtestnet"),
        None,
        "analysis"
    )
    .is_err());
    let f = seed(0);
    bound(&f, "regtest", Some("00000000-0000-0000-0000-000000000001"));
    assert!(networks::require_chain_domain(
        f.book.connection(),
        "profile-1",
        "bitcoin",
        &s("regtest"),
        Some("another-instance"),
        "analysis"
    )
    .is_err());
}
#[test]
fn network_read_snapshot_preserves_live_cursor_and_caller_transactions() {
    let f = seed(0);
    let conn = f.book.connection();
    for fail in [false, true] {
        let mut stmt = conn.prepare("SELECT id FROM wallets ORDER BY id").unwrap();
        let mut cursor = stmt.query([]).unwrap();
        assert_eq!(
            cursor.next().unwrap().unwrap().get::<_, String>(0).unwrap(),
            "wallet-a"
        );
        let result = networks::read_snapshot(conn, || {
            conn.execute("UPDATE wallets SET label='discard' WHERE id='wallet-a'", [])
                .unwrap();
            if fail {
                Err(kassiber_envelope::AppError::new("test", "test"))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.is_err(), fail);
        assert!(conn.is_autocommit());
        assert_eq!(
            cursor.next().unwrap().unwrap().get::<_, String>(0).unwrap(),
            "wallet-b"
        );
        assert_eq!(
            conn.query_row("SELECT label FROM wallets WHERE id='wallet-a'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "Cold"
        );
    }
    conn.execute_batch("BEGIN; UPDATE wallets SET label='caller' WHERE id='wallet-a'")
        .unwrap();
    networks::read_snapshot(conn, || networks::read_snapshot(conn, || Ok(()))).unwrap();
    assert!(!conn.is_autocommit());
    conn.execute_batch("ROLLBACK").unwrap();
}
#[test]
fn network_read_snapshot_is_stable_across_concurrent_commit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("snapshot.sqlite3");
    let conn = kassiber_store::rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE observations(id); INSERT INTO observations VALUES(1)").unwrap();
    let writer = kassiber_store::rusqlite::Connection::open(&path).unwrap();
    let count = || {
        Ok(conn
            .query_row("SELECT count(*) FROM observations", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap())
    };
    networks::read_snapshot(&conn, || {
        assert_eq!(count()?, 1);
        writer
            .execute("INSERT INTO observations VALUES(2)", [])
            .unwrap();
        assert_eq!(networks::read_snapshot(&conn, count)?, 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(count().unwrap(), 2);
}
#[test]
fn partition_closure_and_legacy_rows_block_before_files() {
    let f = seed(5);
    let conn = f.book.connection();
    let args = dict([
        ("environment", s("main")),
        ("wallet_ids", Value::List(vec![s("wallet-a")])),
        ("declared_wallet_ids", Value::List(vec![])),
    ]);
    let plan = partition::plan(conn, "profile-1", &args).unwrap();
    assert_eq!(member(&plan, "can_apply"), &Value::Bool(true));
    conn.execute_batch("CREATE TABLE gl_books(profile_id); INSERT INTO gl_books VALUES('other')")
        .unwrap();
    assert_eq!(
        member(
            &partition::plan(conn, "profile-1", &args).unwrap(),
            "can_apply"
        ),
        &Value::Bool(true)
    );
    conn.execute("INSERT INTO gl_books VALUES('profile-1')", [])
        .unwrap();
    let blocked = partition::plan(conn, "profile-1", &args).unwrap();
    assert_eq!(member(&blocked, "can_apply"), &Value::Bool(false));
    assert_ne!(member(&blocked, "plan_id"), member(&plan, "plan_id"));
    conn.execute_batch("DELETE FROM gl_books; UPDATE transactions SET raw_json='{\"related\":\"other\",\"network\":\"main\"}' WHERE id='main'").unwrap();
    // Raw provider IDs are evidence and do not create authored relationships.
    assert_eq!(
        member(
            &partition::plan(conn, "profile-1", &args).unwrap(),
            "can_apply"
        ),
        &Value::Bool(true)
    );
    conn.execute("INSERT INTO transaction_pairs(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,created_at) VALUES('pair','ws-1','profile-1','main','other','manual','2026-01-01')",[]).unwrap();
    assert_eq!(
        member(
            &partition::plan(conn, "profile-1", &args).unwrap(),
            "can_apply"
        ),
        &Value::Bool(false)
    );
}
#[test]
fn partition_public_wallet_material_drops_credentials_and_private_keys() {
    for private in [
        "xprvSYNTHETIC",
        "ct(slip77(secret),public)",
        "slip77(secret)",
        &"a".repeat(64),
        &format!("K{}", "1".repeat(51)),
    ] {
        let input = dict([
            ("descriptor", s(private)),
            ("token", s("synthetic-secret")),
            ("network", s("main")),
            ("xpub", s("xpub-public")),
        ]);
        let output = partition::public_wallet_config(&input).unwrap();
        assert!(get(&output, "descriptor").is_none());
        assert!(get(&output, "token").is_none());
        assert_eq!(member(&output, "xpub"), &s("xpub-public"));
    }
}
#[test]
fn partition_identity_types_match_python_without_coercing_numbers() {
    let f = seed(0);
    let conn = f.book.connection();
    conn.execute("UPDATE wallets SET id='123' WHERE id='wallet-a'", [])
        .unwrap();
    let args = dict([
        ("environment", s("main")),
        ("wallet_ids", Value::List(vec![Value::Int(123.into())])),
    ]);
    assert_eq!(
        partition::plan(conn, "profile-1", &args).unwrap_err().code,
        "validation"
    );
    let args = dict([
        ("environment", s("main")),
        ("wallet_ids", dict([("123", Value::Bool(true))])),
        ("declared_wallet_ids", dict([("123", Value::Bool(false))])),
    ]);
    assert_eq!(
        member(
            &partition::plan(conn, "profile-1", &args).unwrap(),
            "can_apply"
        ),
        &Value::Bool(true)
    );
}
#[test]
fn partition_export_preserves_ids_encryption_attachments_and_source() {
    use sha2::{Digest, Sha256};
    use std::{fs, io::Read};
    for encrypted in [false, true] {
        let mut f = seed(5);
        if encrypted {
            let path = f._root.path().join("keyed");
            f.book = kassiber_store::Book::create(&path, Some("inner-project-password")).unwrap();
            let v = data();
            let Value::List(scenarios) = member(&v, "scenarios") else {
                panic!()
            };
            let Value::List(sql) = member(&scenarios[5], "seed") else {
                panic!()
            };
            f.book
                .connection()
                .execute_batch("PRAGMA foreign_keys=OFF; BEGIN")
                .unwrap();
            for sql in sql {
                f.book.connection().execute_batch(&py_str(sql)).unwrap();
            }
            f.book
                .connection()
                .execute_batch("COMMIT; PRAGMA foreign_keys=ON")
                .unwrap();
        }
        let conn = f.book.connection();
        conn.execute_batch("ALTER TABLE transaction_edit_events ADD COLUMN sync_signature TEXT; INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,source,changed_at,sync_signature) VALUES('edit-main','ws-1','profile-1','main','user','2026-01-01','old-device-signature')").unwrap();
        let attachments = f._root.path().join("attachments");
        fs::create_dir(&attachments).unwrap();
        fs::write(attachments.join("document"), b"synthetic attachment").unwrap();
        let hash: String = Sha256::digest(b"synthetic attachment")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        conn.execute("INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,stored_relpath,sha256,created_at) VALUES('attachment','ws-1','profile-1','main','file','document',?,'2026-01-01')",[hash]).unwrap();
        let mut args = dict([
            ("environment", s("main")),
            ("wallet_ids", Value::List(vec![s("wallet-a")])),
        ]);
        let plan = partition::plan(conn, "profile-1", &args).unwrap();
        if let Value::Dict(v) = &mut args {
            v.push(("plan_id".into(), member(&plan, "plan_id").clone()));
        }
        let destination = f._root.path().join("export.kassiber");
        let source = f
            ._root
            .path()
            .join(if encrypted { "keyed" } else { "data" });
        let options = || partition::Export {
            data_root: &source,
            attachments_root: &attachments,
            destination: &destination,
            recipient: None,
            backup_passphrase: Some("partition-test-password"),
            db_passphrase: encrypted.then_some("inner-project-password"),
        };
        fs::write(attachments.join("document"), b"changed").unwrap();
        assert_eq!(
            partition::export(conn, "profile-1", &args, options(), &f.sources)
                .unwrap_err()
                .code,
            "attachment_changed"
        );
        assert!(!destination.exists());
        fs::write(attachments.join("document"), b"synthetic attachment").unwrap();
        let before = conn.total_changes();
        partition::export(conn, "profile-1", &args, options(), &f.sources).unwrap();
        assert_eq!(conn.total_changes(), before);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM wallets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            partition::export(conn, "profile-1", &args, options(), &f.sources)
                .unwrap_err()
                .code,
            "conflict"
        );
        let identity = age::scrypt::Identity::new("partition-test-password".to_owned().into());
        let decryptor = age::Decryptor::new(fs::File::open(&destination).unwrap()).unwrap();
        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let mut plaintext = Vec::new();
        reader.read_to_end(&mut plaintext).unwrap();
        let restored = f._root.path().join("restored");
        fs::create_dir(&restored).unwrap();
        tar::Archive::new(plaintext.as_slice())
            .unpack(&restored)
            .unwrap();
        assert_eq!(
            fs::read(restored.join("attachments/document")).unwrap(),
            b"synthetic attachment"
        );
        let target = kassiber_store::Book::open(
            &restored,
            kassiber_store::OpenOptions {
                passphrase: encrypted.then_some("inner-project-password"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(target.encrypted(), encrypted);
        assert_eq!(
            target
                .connection()
                .query_row("SELECT id FROM transaction_edit_events", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "edit-main"
        );
        assert!(target
            .connection()
            .prepare("SELECT sync_signature FROM transaction_edit_events")
            .is_err());
        assert_ne!(target.instance_id(), f.book.instance_id());
        assert_eq!(
            target
                .connection()
                .query_row("SELECT id FROM transactions", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "main"
        );
        assert_eq!(
            target
                .connection()
                .query_row("SELECT id FROM wallets", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wallet-a"
        );
        assert_eq!(
            target
                .connection()
                .query_row("SELECT count(*) FROM backends", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            text(
                &networks::resolve(target.connection(), "profile-1").unwrap(),
                "environment"
            ),
            "main"
        );
        assert!(!target
            .connection()
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap());
    }
}
