use super::*;
use crate::publication::testbook::{load, snapshot, text};
use kassiber_envelope::get;
use kassiber_net::{Ledger, Offline};
use serde_json::json;
use std::time::Duration;

struct Fixed {
    prefix: String,
    ids: u64,
    now: i64,
}
impl Sources for Fixed {
    fn id(&mut self) -> String {
        self.ids += 1;
        format!("{}-{:04}", self.prefix, self.ids)
    }
    fn now(&mut self) -> String {
        self.now += 1;
        kassiber_chain::format_unix_time(1_893_456_000 + self.now).unwrap()
    }
}
fn vectors() -> Json {
    serde_json::from_str(include_str!(
        "../../tests/vectors/python_wallet_publication.json"
    ))
    .unwrap()
}
fn sample() -> Json {
    vectors()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/esplora/0")
        .unwrap()
        .clone()
}
fn book(case: &Json) -> Connection {
    load(&kassiber_pytext::loads(&case["before"].to_string()).unwrap())
}
fn backend(case: &Json) -> Json {
    case["fetch"]["backend"].clone()
}
fn make(conn: &Connection, case: &Json, all: bool) -> Sync {
    Sync::new(
        conn,
        &Request {
            profile_id: "p",
            wallet: (!all).then_some("a"),
            all,
            force_full: case["fetch"]["force_full"] == true,
            version: "fixture",
        },
        |_, _| Ok(backend(case)),
    )
    .unwrap()
}
fn fixed(case: &Json) -> Fixed {
    Fixed {
        prefix: case["prefix"].as_str().unwrap().into(),
        ids: 0,
        now: case["now"].as_i64().unwrap(),
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn online() -> Gate {
    Gate::new(|| Offline::Online, Ledger::default())
}
fn run(
    action: &Sync,
    conn: &mut Connection,
    case: &Json,
    proof: Option<&SyncConsent>,
    cancel: &Cancellation,
) -> Result<Vec<Value>> {
    action.execute(
        conn,
        &online(),
        proof,
        cancel,
        deadline(),
        &mut fixed(case),
        &mut |_, _| Ok(backend(case)),
        |_, _| Ok(case["fetch"].clone()),
        |_| Ok(()),
    )
}
fn strip_elapsed(value: &mut Value) {
    match value {
        Value::Dict(pairs) => {
            pairs.retain(|(k, _)| *k != "elapsed_ms".into());
            for (_, v) in pairs {
                strip_elapsed(v);
            }
        }
        Value::List(items) => {
            for item in items {
                strip_elapsed(item);
            }
        }
        _ => (),
    }
}

#[test]
fn sync_service_composes_all_102_python_publication_rounds() {
    let vectors = vectors();
    let mut successes = 0;
    for case in vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["outcome"]["result"].is_array())
    {
        for all in [false, true] {
            let mut conn = book(case);
            let action = make(&conn, case, all);
            assert_eq!(action.wallet_ids(), vec!["a"]);
            let proof = SyncConsent::explicit_trigger(&action);
            let mut actual = Value::List(
                run(
                    &action,
                    &mut conn,
                    case,
                    Some(&proof),
                    &Cancellation::default(),
                )
                .unwrap(),
            );
            strip_elapsed(&mut actual);
            assert_eq!(
                text(&actual),
                text(&kassiber_pytext::loads(&case["outcome"]["result"].to_string()).unwrap()),
                "{} all={all}",
                case["name"]
            );
            assert_eq!(
                text(&snapshot(&conn)),
                text(&kassiber_pytext::loads(&case["after"].to_string()).unwrap()),
                "{} all={all}",
                case["name"]
            );
        }
        successes += 1;
    }
    assert_eq!(successes, 102);
}

#[test]
fn sync_service_consent_is_exact_revocable_single_use_and_inert() {
    let case = sample();
    let mut conn = book(&case);
    let before = snapshot(&conn);
    let action = make(&conn, &case, false);
    let other = make(&conn, &case, false);
    assert_eq!(snapshot(&conn), before);
    let foreign = SyncConsent::explicit_trigger(&other);
    let revoked = SyncConsent::explicit_trigger(&action);
    revoked.revoke();
    for proof in [None, Some(&foreign), Some(&revoked)] {
        assert_eq!(
            run(&action, &mut conn, &case, proof, &Cancellation::default())
                .unwrap_err()
                .code,
            "interaction_required"
        );
        assert_eq!(snapshot(&conn), before);
        if let Some(proof) = proof {
            assert!(!proof.started.load(Ordering::SeqCst));
        }
    }
    let consent = SyncConsent::explicit_trigger(&action);
    let error = AppError::new("upstream_error", "failed");
    let mut called = 0;
    for expected in ["upstream_error", "interaction_required"] {
        let result = action.execute(
            &mut conn,
            &online(),
            Some(&consent),
            &Cancellation::default(),
            deadline(),
            &mut fixed(&case),
            &mut |_, _| Ok(backend(&case)),
            |_, _| {
                called += 1;
                Err(error.clone())
            },
            |_| Ok(()),
        );
        assert_eq!(result.unwrap_err().code, expected);
    }
    assert_eq!(called, 1);
    assert_eq!(snapshot(&conn), before);
}

#[test]
fn sync_service_scope_changes_refuse_before_fetch_and_do_not_consume_consent() {
    let case = sample();
    for sql in [
        "UPDATE wallets SET label='edited' WHERE id='a'",
        "UPDATE wallets SET kind='addresses' WHERE id='a'",
        "UPDATE wallets SET config_json='{}' WHERE id='a'",
        "UPDATE wallets SET profile_id='elsewhere' WHERE id='a'",
        "INSERT OR REPLACE INTO settings VALUES('database_instance_id','changed')",
        "UPDATE profiles SET workspace_id='elsewhere' WHERE id='p'",
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','B','addresses','{}','fixed')",
    ] {
        let mut conn=book(&case);
        conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        let action=make(&conn,&case,true);
        let proof=SyncConsent::explicit_trigger(&action);
        conn.execute_batch(sql).unwrap();
        let before=snapshot(&conn);
        assert!(run(&action,&mut conn,&case,Some(&proof),&Cancellation::default()).is_err(),"{sql}");
        assert!(!proof.started.load(Ordering::SeqCst));
        assert_eq!(snapshot(&conn),before);
    }
    let mut conn = book(&case);
    let action = make(&conn, &case, false);
    let proof = SyncConsent::explicit_trigger(&action);
    let before = snapshot(&conn);
    let gate = online();
    let error = action
        .execute(
            &mut conn,
            &gate,
            Some(&proof),
            &Cancellation::default(),
            deadline(),
            &mut fixed(&case),
            &mut |_, _| {
                Ok(json!({"name":"fixture","kind":"esplora","url":"http://changed.invalid"}))
            },
            |_, _| panic!("stale backend must not fetch"),
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "stale_context");
    assert!(!proof.started.load(Ordering::SeqCst));
    assert_eq!(snapshot(&conn), before);
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn sync_service_offline_early_cancel_and_deadline_make_no_traffic_or_writes() {
    let mut case = sample();
    for url in ["http://127.0.0.1:1", "http://unresolvable.invalid:80"] {
        case["fetch"]["backend"]["url"] = json!(url);
        for mode in ["offline", "unavailable", "cancel", "deadline"] {
            let mut conn = book(&case);
            let before = snapshot(&conn);
            let action = make(&conn, &case, false);
            let proof = SyncConsent::explicit_trigger(&action);
            let cancel = Cancellation::default();
            if mode == "cancel" {
                cancel.cancel();
            }
            let offline = match mode {
                "offline" => Offline::Offline,
                "unavailable" => Offline::Unavailable,
                _ => Offline::Online,
            };
            let gate = Gate::new(move || offline, Ledger::default());
            let result = action.run(
                &mut conn,
                &gate,
                Some(&proof),
                &cancel,
                if mode == "deadline" {
                    Instant::now()
                } else {
                    deadline()
                },
                100,
                &mut fixed(&case),
                |_, _| Ok(backend(&case)),
            );
            assert!(result.is_err(), "{mode}");
            if matches!(mode, "cancel" | "deadline") {
                assert!(!proof.started.load(Ordering::SeqCst));
            }
            assert!(gate.attempts().records.is_empty());
            assert!(gate.ledger().snapshot(0, 100).records.is_empty());
            assert_eq!(snapshot(&conn), before);
        }
    }
}

#[test]
fn sync_service_every_apply_stage_rolls_back_on_cancel_revocation_and_offline() {
    let case = sample();
    for stage in [
        "begin",
        "observer_persistence",
        "retractions",
        "transaction_insertion",
        "output_inventory",
        "derivation_coverage",
        "freshness_checkpoint",
        "commit",
    ] {
        for mode in ["cancel", "revoke", "offline"] {
            let mut conn = book(&case);
            let before = snapshot(&conn);
            let action = make(&conn, &case, false);
            let proof = SyncConsent::explicit_trigger(&action);
            let cancel = Cancellation::default();
            let offline = Arc::new(AtomicBool::new(false));
            let status = offline.clone();
            let gate = Gate::new(
                move || {
                    if status.load(Ordering::SeqCst) {
                        Offline::Offline
                    } else {
                        Offline::Online
                    }
                },
                Ledger::default(),
            );
            let result = action.execute(
                &mut conn,
                &gate,
                Some(&proof),
                &cancel,
                deadline(),
                &mut fixed(&case),
                &mut |_, _| Ok(backend(&case)),
                |_, _| Ok(case["fetch"].clone()),
                |current| {
                    if current == stage {
                        match mode {
                            "cancel" => cancel.cancel(),
                            "revoke" => proof.revoke(),
                            _ => offline.store(true, Ordering::SeqCst),
                        }
                    }
                    Ok(())
                },
            );
            assert!(result.is_err(), "{stage} {mode}");
            assert_eq!(snapshot(&conn), before, "{stage} {mode}");
            assert!(gate.attempts().records.is_empty());
        }
    }
}

#[test]
fn sync_service_selection_skips_deprecated_only_for_all_and_validates_flags() {
    let case = sample();
    let mut conn = book(&case);
    for (reference, all, code) in [
        (None, false, "app_error"),
        (Some("a"), true, "validation"),
        (Some("missing"), false, "not_found"),
    ] {
        let result = Sync::new(
            &conn,
            &Request {
                profile_id: "p",
                wallet: reference,
                all,
                force_full: false,
                version: "test",
            },
            |_, _| panic!("selection before backend"),
        );
        assert_eq!(result.err().unwrap().code, code);
    }
    conn.execute(
        "UPDATE wallets SET config_json='{\"deprecated\":\"false\"}' WHERE id='a'",
        [],
    )
    .unwrap();
    let action = make(&conn, &case, true);
    assert_eq!(action.wallet_ids(), vec!["a"]);
    conn.execute(
        "UPDATE wallets SET config_json='{\"deprecated\":\"true\"}' WHERE id='a'",
        [],
    )
    .unwrap();
    let action = make(&conn, &case, true);
    let consent = SyncConsent::explicit_trigger(&action);
    assert!(run(
        &action,
        &mut conn,
        &case,
        Some(&consent),
        &Cancellation::default()
    )
    .unwrap()
    .is_empty());
    let action = make(&conn, &case, false);
    let consent = SyncConsent::explicit_trigger(&action);
    let results = run(
        &action,
        &mut conn,
        &case,
        Some(&consent),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(get(&results[0], "status"), Some(&s("skipped")));
}

#[test]
fn sync_service_all_preserves_label_order_and_redacts_isolated_errors() {
    let case = sample();
    let mut conn = book(&case);
    let action = make(&conn, &case, true);
    let proof = SyncConsent::explicit_trigger(&action);
    let before = snapshot(&conn);
    let error = AppError::new(
        "upstream_error",
        "failed https://user:pass@host.invalid/api?q=secret.",
    )
    .with_hint("retry ssl://u:p@host.invalid:50002/?secret=yes!")
    .with_details(dict([(
        "nested",
        Value::List(vec![s("http://u:p@host.invalid/a?token=x")]),
    )]))
    .retryable(true);
    let result = action
        .execute(
            &mut conn,
            &online(),
            Some(&proof),
            &Cancellation::default(),
            deadline(),
            &mut fixed(&case),
            &mut |_, _| Ok(backend(&case)),
            |_, _| Err(error.clone()),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(get(&result[0], "wallet"), Some(&s("Cold")));
    assert_eq!(get(&result[0], "code"), Some(&s("upstream_error")));
    assert_eq!(
        get(&result[0], "message"),
        Some(&s("failed https://<redacted>@host.invalid/api."))
    );
    assert_eq!(
        get(&result[0], "hint"),
        Some(&s("retry ssl://<redacted>@host.invalid:50002/!"))
    );
    assert_eq!(get(&result[0], "retryable"), Some(&Value::Bool(true)));
    assert!(!text(&result[0]).contains("token="));
    assert_eq!(snapshot(&conn), before);
    conn.execute(
        "UPDATE wallets SET config_json='{}',label='Zulu' WHERE id='a'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','Alpha','addresses','{}','fixed')",[]).unwrap();
    let action = make(&conn, &case, true);
    assert_eq!(action.wallet_ids(), vec!["b", "a"]);
    let proof = SyncConsent::explicit_trigger(&action);
    assert_eq!(
        run(
            &action,
            &mut conn,
            &case,
            Some(&proof),
            &Cancellation::default()
        )
        .unwrap()
        .len(),
        2
    );
}

#[test]
fn sync_service_unsupported_invocation_refuses_during_inert_construction() {
    let case = sample();
    for c in [
        json!({"source_file":"a","source_format":"csv"}),
        json!({"chain":"liquid","descriptor":"wpkh(bad)"}),
        json!({"addresses":["a"]}),
    ] {
        let conn = book(&case);
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [c.to_string()],
        )
        .unwrap();
        let before = snapshot(&conn);
        assert_eq!(
            Sync::new(
                &conn,
                &Request {
                    profile_id: "p",
                    wallet: Some("a"),
                    all: false,
                    force_full: false,
                    version: "test"
                },
                |_, _| panic!("unsupported wallet must refuse before backend resolution")
            )
            .err()
            .unwrap()
            .code,
            "native_fallback_required"
        );
        assert_eq!(snapshot(&conn), before);
    }
    let conn = book(&case);
    let before = snapshot(&conn);
    assert_eq!(
        Sync::new(
            &conn,
            &Request {
                profile_id: "p",
                wallet: Some("a"),
                all: false,
                force_full: false,
                version: "test"
            },
            |_, _| Ok(json!({"name":"fixture","kind":"bitcoinrpc","url":"http://127.0.0.1:1"}))
        )
        .err()
        .unwrap()
        .code,
        "native_fallback_required"
    );
    assert_eq!(snapshot(&conn), before);
}

#[test]
fn sync_service_revalidates_observer_checkpoint_overlap_and_connection_before_traffic() {
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "checkpoint/electrum/1")
        .unwrap();
    for sql in [
        "UPDATE chain_observer_instances SET state_epoch=state_epoch+1",
        "UPDATE chain_observer_instances SET state_json='{}'",
        "UPDATE freshness_source_states SET checkpoint_json='{\"changed\":true}'",
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','B','addresses','{}','fixed')",
    ] {
        let mut conn=book(case);
        let action=make(&conn,case,false);
        let proof=SyncConsent::explicit_trigger(&action);
        assert!(conn.execute(sql,[]).unwrap()>0);
        let before=snapshot(&conn);
        assert!(run(&action,&mut conn,case,Some(&proof),&Cancellation::default()).is_err(),"{sql}");
        assert!(!proof.started.load(Ordering::SeqCst));
        assert_eq!(snapshot(&conn),before);
    }
    let mut a = book(case);
    let mut b = book(case);
    let action = make(&a, case, false);
    let proof = SyncConsent::explicit_trigger(&action);
    assert_eq!(
        run(
            &action,
            &mut b,
            case,
            Some(&proof),
            &Cancellation::default()
        )
        .unwrap_err()
        .code,
        "stale_context"
    );
    assert!(!proof.started.load(Ordering::SeqCst));
    a.execute_batch("BEGIN").unwrap();
    assert_eq!(
        run(
            &action,
            &mut a,
            case,
            Some(&proof),
            &Cancellation::default()
        )
        .unwrap_err()
        .code,
        "observer_prepare_in_transaction"
    );
    assert!(!proof.started.load(Ordering::SeqCst));
    a.execute_batch("ROLLBACK").unwrap();
    a.execute("UPDATE wallets SET config_json='{}'", [])
        .unwrap();
    let action = make(&a, case, false);
    let proof = SyncConsent::explicit_trigger(&action);
    a.execute_batch("BEGIN").unwrap();
    assert_eq!(
        run(
            &action,
            &mut a,
            case,
            Some(&proof),
            &Cancellation::default()
        )
        .unwrap_err()
        .code,
        "observer_prepare_in_transaction"
    );
    assert!(!proof.started.load(Ordering::SeqCst));
    a.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn sync_service_constructor_failures_keep_single_and_all_error_contracts() {
    let case = sample();
    for all in [false, true] {
        let mut conn = book(&case);
        let before = snapshot(&conn);
        let req = Request {
            profile_id: "p",
            wallet: (!all).then_some("a"),
            all,
            force_full: false,
            version: "test",
        };
        let error = AppError::new("app_error", "Unknown backend 'fixture'");
        let action = Sync::new(&conn, &req, |_, _| Err(error.clone())).unwrap();
        let proof = SyncConsent::explicit_trigger(&action);
        let result = run(
            &action,
            &mut conn,
            &case,
            Some(&proof),
            &Cancellation::default(),
        );
        if all {
            let rows = result.unwrap();
            assert_eq!(get(&rows[0], "code"), Some(&s("app_error")));
            assert_eq!(get(&rows[0], "hint"), Some(&s("")));
            assert_eq!(get(&rows[0], "details"), Some(&Value::Null));
            assert_eq!(get(&rows[0], "retryable"), Some(&Value::Bool(false)));
        } else {
            assert_eq!(result.unwrap_err(), error);
        }
        assert_eq!(snapshot(&conn), before);
        assert_eq!(
            Sync::new(&conn, &req, |_, _| Err(wallet::fallback("test")))
                .err()
                .unwrap()
                .code,
            "native_fallback_required"
        );
        let mut invalid = backend(&case);
        invalid["url"] = json!("invalid");
        let action = Sync::new(&conn, &req, |_, _| Ok(invalid.clone())).unwrap();
        let proof = SyncConsent::explicit_trigger(&action);
        let result = action.execute(
            &mut conn,
            &online(),
            Some(&proof),
            &Cancellation::default(),
            deadline(),
            &mut fixed(&case),
            &mut |_, _| Ok(invalid.clone()),
            |_, _| panic!("invalid backend must not fetch"),
            |_| Ok(()),
        );
        if all {
            assert_eq!(get(&result.unwrap()[0], "status"), Some(&s("error")));
        } else {
            assert!(result.is_err());
        }
        assert_eq!(snapshot(&conn), before);
    }
}

#[test]
fn sync_service_preflight_rejects_malformed_inputs_and_node_routes_without_io() {
    let case = sample();
    let conn = book(&case);
    let req = Request {
        profile_id: "missing",
        wallet: Some("a"),
        all: false,
        force_full: false,
        version: "test",
    };
    assert_eq!(
        Sync::new(&conn, &req, |_, _| panic!("scope before backend"))
            .err()
            .unwrap(),
        AppError::new("not_found", "Profile not found")
    );
    conn.execute_batch("BEGIN").unwrap();
    assert_eq!(
        Sync::new(&conn, &req, |_, _| panic!("transaction before backend"))
            .err()
            .unwrap()
            .code,
        "observer_prepare_in_transaction"
    );
    conn.execute_batch("ROLLBACK").unwrap();
    for raw in [
        "broken",
        "[]",
        "null",
        "42",
        "false",
        "\"text\"",
        "{\"deprecated\":\"invalid\"}",
    ] {
        conn.execute("UPDATE wallets SET config_json=?", [raw])
            .unwrap();
        let error = Sync::new(
            &conn,
            &Request {
                profile_id: "p",
                wallet: None,
                all: true,
                force_full: false,
                version: "test",
            },
            |_, _| panic!("malformed config before backend"),
        )
        .err()
        .unwrap();
        if raw.starts_with('{') {
            continue;
        }
        // tests/rust_port/repro_ai_wallets_sync_bad_config.py: the typed
        // refusal names the wallet, in the CLI selection and the planner.
        let label: String = conn
            .query_row("SELECT label FROM wallets WHERE id='a'", [], |row| row.get(0))
            .unwrap();
        let expected = AppError::new(
            "invalid_wallet_config",
            format!("Wallet '{label}' has an unreadable stored configuration"),
        )
        .with_hint("Edit or remove this wallet, then sync again.")
        .with_details(kassiber_envelope::dict([("wallet", s(label.as_str()))]));
        assert_eq!(error, expected, "{raw}");
        for wallet in [None, Some("a".to_owned())] {
            let options = crate::freshness::sources::Options {
                wallet,
                ..Default::default()
            };
            assert_eq!(
                crate::freshness::sources::build(&conn, "p", &options).err(),
                Some(expected.clone()),
                "{raw}"
            );
        }
    }
    for kind in ["coreln", "lnd"] {
        conn.execute(
            "UPDATE wallets SET kind=?,config_json='{\"backend\":\"fixture\"}'",
            [kind],
        )
        .unwrap();
        assert_eq!(
            Sync::new(
                &conn,
                &Request {
                    profile_id: "p",
                    wallet: Some("a"),
                    all: false,
                    force_full: false,
                    version: "test"
                },
                |_, _| panic!("unsupported shape must refuse before backend resolution")
            )
            .err()
            .unwrap()
            .code,
            "native_fallback_required"
        );
        conn.execute("UPDATE wallets SET config_json=''", [])
            .unwrap();
        assert_eq!(make(&conn, &case, true).wallet_ids(), vec!["a"]);
    }
}

#[test]
fn sync_service_transport_permission_rejects_missing_foreign_and_revoked_grants() {
    let case = sample();
    let conn = book(&case);
    let service = make(&conn, &case, false);
    let action = service.selected[0]
        .fetch
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    let other = Fetch::new(&conn, "p", "a", &backend(&case), false, "fixture").unwrap();
    let foreign = FetchConsent::explicit_trigger(&other);
    let revoked = FetchConsent::explicit_trigger(action);
    revoked.revoke();
    let gate = online();
    for grant in [None, Some(&foreign), Some(&revoked)] {
        assert_eq!(
            action.check_permission(&gate, grant).unwrap_err().code,
            "interaction_required"
        );
    }
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 100).records.is_empty());
}

#[test]
fn sync_service_native_driver_fetches_and_publishes_one_explicit_action() {
    use kassiber_net::test_support::FakeProvider;
    let v = vectors();
    let case = v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "fixed/esplora/0")
        .unwrap();
    let stats = r#"{"chain_stats":{"tx_count":0},"mempool_stats":{"tx_count":0}}"#;
    let server = FakeProvider::replies(vec![
        (200, "0".repeat(64)),
        (200, stats.into()),
        (200, "[]".into()),
        (200, "[]".into()),
        (200, stats.into()),
        (200, "25".into()),
        (200, "[]".into()),
    ])
    .unwrap();
    let mut b = backend(case);
    b["url"] = json!(server.base());
    let mut conn = book(case);
    let action = Sync::new(
        &conn,
        &Request {
            profile_id: "p",
            wallet: Some("a"),
            all: false,
            force_full: true,
            version: "test",
        },
        |_, _| Ok(b.clone()),
    )
    .unwrap();
    assert!(server.requests().is_empty());
    let consent = SyncConsent::explicit_trigger(&action);
    let result = action
        .run(
            &mut conn,
            &online(),
            Some(&consent),
            &Cancellation::default(),
            deadline(),
            100,
            &mut fixed(case),
            |_, _| Ok(b.clone()),
        )
        .unwrap();
    assert_eq!(get(&result[0], "status"), Some(&s("synced")));
    assert_eq!(get(&result[0], "force_full"), Some(&Value::Bool(true)));
    assert_eq!(server.requests().len(), 7);
    let cp:String=conn.query_row("SELECT checkpoint_json FROM freshness_source_states WHERE profile_id='p' AND source_key='onchain_wallet:a'",[],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Json>(&cp).unwrap()["esplora_tip_hash"],
        json!("0".repeat(64))
    );
}

#[test]
fn sync_service_rechecks_backend_at_prefetch_and_publication() {
    let case = sample();
    for change_at in [2, 3] {
        let mut conn = book(&case);
        let before = snapshot(&conn);
        let action = make(&conn, &case, false);
        let proof = SyncConsent::explicit_trigger(&action);
        let mut checks = 0;
        let mut fetches = 0;
        let result = action.execute(
            &mut conn,
            &online(),
            Some(&proof),
            &Cancellation::default(),
            deadline(),
            &mut fixed(&case),
            &mut |_, _| {
                checks += 1;
                let mut b = backend(&case);
                if checks >= change_at {
                    b["url"] = json!("http://changed.invalid");
                }
                Ok(b)
            },
            |_, _| {
                fetches += 1;
                Ok(case["fetch"].clone())
            },
            |_| Ok(()),
        );
        assert_eq!(result.unwrap_err().code, "stale_context");
        assert_eq!(fetches, change_at - 2);
        assert_eq!(snapshot(&conn), before);
    }
}

#[test]
fn sync_service_network_scope_is_checked_for_wallets_without_sources() {
    let case = sample();
    let mut conn = book(&case);
    conn.execute("UPDATE wallets SET config_json='{}'", [])
        .unwrap();
    let action = make(&conn, &case, false);
    let proof = SyncConsent::explicit_trigger(&action);
    conn.execute("INSERT INTO book_network_bindings(profile_id,environment_id,environment,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','bound','main','[]','[]','fixed','fixed')",[]).unwrap();
    let before = snapshot(&conn);
    assert_eq!(
        run(
            &action,
            &mut conn,
            &case,
            Some(&proof),
            &Cancellation::default()
        )
        .unwrap_err()
        .code,
        "stale_context"
    );
    assert!(!proof.started.load(Ordering::SeqCst));
    assert_eq!(snapshot(&conn), before);
    conn.execute(
        "UPDATE wallets SET config_json='{\"chain\":\"bitcoin\",\"network\":\"test\"}'",
        [],
    )
    .unwrap();
    assert!(Sync::new(
        &conn,
        &Request {
            profile_id: "p",
            wallet: Some("a"),
            all: false,
            force_full: false,
            version: "test"
        },
        |_, _| panic!("scope before backend")
    )
    .is_err());
}

#[test]
fn sync_service_resolves_saved_and_default_backends_and_rechecks_default_selection() {
    let case = sample();
    let mut conn = book(&case);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors/sync-service-missing.env");
    assert!(!path.exists());
    let config =
        kassiber_setup::backends::Config::load_with_env(&path, &Default::default()).unwrap();
    assert!(config.resolve(&conn, "unknown").is_err());
    conn.execute_batch(
        "INSERT INTO backends(name,kind,url,created_at,updated_at) VALUES
         ('fixture','esplora','http://127.0.0.1:1','fixed','fixed'),
         ('other','esplora','http://127.0.0.1:2','fixed','fixed');
         INSERT OR REPLACE INTO settings VALUES('default_backend','fixture');",
    )
    .unwrap();
    assert_eq!(
        resolve_backend(&conn, &config, "").unwrap()["name"],
        "fixture"
    );
    assert_eq!(
        resolve_backend(&conn, &config, " Other ").unwrap()["name"],
        "other"
    );
    let mut c: Json = serde_json::from_str(
        &conn
            .query_row("SELECT config_json FROM wallets WHERE id='a'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
    )
    .unwrap();
    for backend in [json!(" FIXTURE "), json!(42)] {
        c["backend"] = backend.clone();
        conn.execute(
            "UPDATE wallets SET config_json=? WHERE id='a'",
            [c.to_string()],
        )
        .unwrap();
        let result = Sync::new(
            &conn,
            &Request {
                profile_id: "p",
                wallet: Some("a"),
                all: false,
                force_full: false,
                version: "test",
            },
            |conn, name| resolve_backend(conn, &config, name),
        );
        if backend.is_string() {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.err().unwrap().code, "native_fallback_required");
        }
    }
    c.as_object_mut().unwrap().remove("backend");
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='a'",
        [c.to_string()],
    )
    .unwrap();
    let action = Sync::new(
        &conn,
        &Request {
            profile_id: "p",
            wallet: Some("a"),
            all: false,
            force_full: false,
            version: "test",
        },
        |conn, name| resolve_backend(conn, &config, name),
    )
    .unwrap();
    let consent = SyncConsent::explicit_trigger(&action);
    conn.execute(
        "UPDATE settings SET value='other' WHERE key='default_backend'",
        [],
    )
    .unwrap();
    let before = snapshot(&conn);
    let gate = online();
    assert_eq!(
        action
            .execute(
                &mut conn,
                &gate,
                Some(&consent),
                &Cancellation::default(),
                deadline(),
                &mut fixed(&case),
                &mut |conn, name| resolve_backend(conn, &config, name),
                |_, _| panic!("changed default must refuse before fetch"),
                |_| Ok(()),
            )
            .unwrap_err()
            .code,
        "stale_context"
    );
    assert!(!consent.started.load(Ordering::SeqCst));
    assert_eq!(snapshot(&conn), before);
    assert!(gate.attempts().records.is_empty());
    conn.execute_batch(
        "DELETE FROM backends;
         DELETE FROM settings WHERE key='default_backend';
         INSERT OR REPLACE INTO settings VALUES('backend_bootstrap_mode','manual');
         INSERT OR REPLACE INTO settings VALUES('bootstrap_backend_tombstones',
         '[\"mempool\",\"fulcrum\",\"liquid\",\"liquid-blockstream\"]');",
    )
    .unwrap();
    assert_eq!(
        resolve_backend(&conn, &config, "").unwrap_err(),
        AppError::new("backend_not_configured", "No sync backend is configured")
            .with_hint("Add a backend in Settings before refreshing this wallet.")
    );
}
