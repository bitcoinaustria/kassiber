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
                None,
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
        "UPDATE wallets SET config_json='{\"deprecated\":\"true\",\"last_synced_at\":\"2026-01-01T00:00:00Z\"}' WHERE id='a'",
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

/// `wallet_in_sync_all`: a retired wallet is skipped only once its history
/// is imported. A never-synced (or reset) one joins sync-all and keeps its
/// place through validation after it publishes; `--force-full` always
/// includes it.
#[test]
fn sync_all_includes_retired_wallets_until_their_history_is_imported() {
    let case = sample();
    let mut conn = book(&case);
    conn.execute(
        "UPDATE wallets SET config_json=json_set(json_remove(config_json,'$.last_synced_at'),'$.deprecated',json('true')) WHERE id='a'",
        [],
    )
    .unwrap();
    // A file wallet labelled after it revalidates the selection once the
    // retired wallet has published and recorded its sync time.
    let csv =
        std::env::temp_dir().join(format!("kassiber-retired-sync-{}.csv", std::process::id()));
    std::fs::write(
        &csv,
        "txid,occurred_at,direction,asset,amount,fee\nfile-1,2026-01-02T10:00:00Z,inbound,BTC,0.001,0\n",
    )
    .unwrap();
    let config = serde_json::json!({"source_file": csv.to_str().unwrap(), "source_format": "csv"});
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('z','w','p','Z File','custom',?,'fixed')",
        [config.to_string()],
    )
    .unwrap();
    let action = make(&conn, &case, true);
    assert_eq!(action.wallet_ids(), vec!["a", "z"]);
    let consent = SyncConsent::explicit_trigger(&action);
    let results = run(
        &action,
        &mut conn,
        &case,
        Some(&consent),
        &Cancellation::default(),
    );
    std::fs::remove_file(&csv).unwrap();
    let results = results.unwrap();
    assert_eq!(get(&results[0], "status"), Some(&s("synced")));
    assert_eq!(get(&results[1], "status"), Some(&s("synced")));
    let synced: String = conn
        .query_row(
            "SELECT json_extract(config_json,'$.last_synced_at') FROM wallets WHERE id='a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!synced.is_empty());
    assert_eq!(make(&conn, &case, true).wallet_ids(), vec!["z"]);
    let forced = Sync::new(
        &conn,
        &Request {
            profile_id: "p",
            wallet: None,
            all: true,
            force_full: true,
            version: "fixture",
        },
        |_, _| Ok(backend(&case)),
    )
    .unwrap();
    assert_eq!(forced.wallet_ids(), vec!["a", "z"]);
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
    {
        let c = json!({"chain":"lightning","descriptor":"wpkh(bad)"});
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
    // Liquid descriptor wallets are a native LWK route now: they reach
    // backend resolution instead of refusing during inert construction.
    let conn = book(&case);
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='a'",
        [json!({"chain":"liquid","descriptor":"wpkh(bad)","backend":"fixture"}).to_string()],
    )
    .unwrap();
    let mut resolved = false;
    let _ = Sync::new(
        &conn,
        &Request {
            profile_id: "p",
            wallet: Some("a"),
            all: false,
            force_full: false,
            version: "test",
        },
        |_, _| {
            resolved = true;
            Err(AppError::new("backend_not_configured", "fixture"))
        },
    );
    assert!(resolved);
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
            |_, _| Ok(json!({"name":"fixture","kind":"custom","url":"http://127.0.0.1:1"}))
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
        "UPDATE wallets SET label='Renamed'",
    ] {
        let mut conn = book(case);
        let action = make(&conn, case, false);
        let proof = SyncConsent::explicit_trigger(&action);
        assert!(conn.execute(sql, []).unwrap() > 0);
        let before = snapshot(&conn);
        assert!(
            run(
                &action,
                &mut conn,
                case,
                Some(&proof),
                &Cancellation::default()
            )
            .is_err(),
            "{sql}"
        );
        assert!(!proof.started.load(Ordering::SeqCst));
        assert_eq!(snapshot(&conn), before);
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
            .query_row("SELECT label FROM wallets WHERE id='a'", [], |row| {
                row.get(0)
            })
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
    // Node wallets with a backend take the native node route, which
    // resolves that backend while construction stays inert.
    for kind in ["coreln", "lnd"] {
        conn.execute(
            "UPDATE wallets SET kind=?,config_json='{\"backend\":\"fixture\"}'",
            [kind],
        )
        .unwrap();
        let before = snapshot(&conn);
        let mut resolved = vec![];
        Sync::new(
            &conn,
            &Request {
                profile_id: "p",
                wallet: Some("a"),
                all: false,
                force_full: false,
                version: "test",
            },
            |_, name| {
                resolved.push(name.to_owned());
                Ok(json!({"name": name, "kind": kind}))
            },
        )
        .unwrap();
        assert_eq!(resolved, vec!["fixture"]);
        assert_eq!(snapshot(&conn), before);
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
    // The tip hash, its height bound to that hash, then the histories.
    let server = FakeProvider::replies(vec![
        (200, "0".repeat(64)),
        (200, "25".into()),
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
            None,
        )
        .unwrap();
    assert_eq!(get(&result[0], "status"), Some(&s("synced")));
    assert_eq!(get(&result[0], "force_full"), Some(&Value::Bool(true)));
    assert_eq!(server.requests().len(), 9);
    let cp:String=conn.query_row("SELECT checkpoint_json FROM freshness_source_states WHERE profile_id='p' AND source_key='onchain_wallet:a'",[],|r|r.get(0)).unwrap();
    let cp = serde_json::from_str::<Json>(&cp).unwrap();
    assert_eq!(cp["esplora_tip_hash"], json!("0".repeat(64)));
    assert_eq!(cp["esplora_tip_height"], json!(25));
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

/// Python's direct `sync_wallet` answers a selection without sources with
/// `skipped` rows; anything it would refresh keeps the selection out.
#[test]
fn sync_service_sourceless_selections_answer_python_skipped_rows() {
    let case = sample();
    let conn = book(&case);
    let rows = |kind: &str, config: &str| {
        conn.execute(
            "UPDATE wallets SET kind=?, config_json=? WHERE id='a'",
            [kind, config],
        )
        .unwrap();
        Sync::new(
            &conn,
            &Request {
                profile_id: "p",
                wallet: Some("a"),
                all: false,
                force_full: false,
                version: "fixture",
            },
            |_, _| Ok(backend(&case)),
        )
        .ok()
        .and_then(|action| action.sourceless_results())
    };
    let skipped = rows("kraken", r#"{"backend":"kraken-live"}"#).unwrap();
    assert_eq!(
        skipped,
        vec![dict([
            ("wallet", get(&skipped[0], "wallet").unwrap().clone()),
            ("status", s("skipped")),
            (
                "reason",
                s("no file source, descriptor, or backend addresses configured")
            ),
        ])]
    );
    assert!(rows("custom", r#"{"addresses":[],"descriptor":""}"#).is_some());
    for (kind, config) in [
        ("custom", r#"{"sp_descriptor":"sp1"}"#),
        (
            "custom",
            r#"{"source_file":"/x.csv","source_format":"csv"}"#,
        ),
        ("custom", r#"{"sync_source":"btcpay"}"#),
        ("phoenix", "{}"),
        ("nwc", "{}"),
    ] {
        assert!(rows(kind, config).is_none(), "{kind} {config}");
    }
}

#[test]
fn sync_service_parallel_multi_wallet_fetches_concurrently_without_skipping_and_preserves_order() {
    use bitcoin::hex::FromHex;
    use kassiber_net::test_support::{FakeService, Reply};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // Six distinct valid mainnet P2WPKH addresses, one per wallet.
    let plan = crate::descriptors::load(&json!({
        "chain": "bitcoin",
        "network": "main",
        "descriptor": "wpkh(xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8/0/*)",
    }))
    .map_err(|e| e.into_app())
    .unwrap()
    .unwrap();
    let targets = plan
        .derive(Some(0), 0, 6)
        .map_err(|e| e.into_app())
        .unwrap();
    assert_eq!(targets.len(), 6);

    let tip_hash = "0000000000000000000123456789abcdef0123456789abcdef0123456789abcd";
    let block_hash = "0000000000000000000fedcba9876543210fedcba9876543210fedcba9876543";
    let mut tx_by_hash = std::collections::HashMap::<String, serde_json::Value>::new();
    for (idx, target) in targets.iter().enumerate() {
        let script_pubkey = target["script_pubkey"].as_str().unwrap();
        let address = target["address"].as_str().unwrap();
        let sh: String = Sha256::digest(Vec::<u8>::from_hex(script_pubkey).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let txid = format!("{:064x}", idx + 1);
        let amount = 100_000 + (idx as u64) * 10_000;
        let tx = json!({
            "txid": txid,
            "version": 2,
            "locktime": 0,
            "size": 191,
            "weight": 764,
            "fee": 1000,
            "status": {
                "confirmed": true,
                "block_height": 100,
                "block_hash": block_hash,
                "block_time": 1_700_000_000 + idx as u64 * 60,
            },
            "vin": [{
                "txid": format!("{:064x}", idx + 100),
                "vout": 0,
                "is_coinbase": false,
                "scriptsig": "",
                "sequence": 4294967294u64,
                "prevout": {
                    "scriptpubkey": "00143333333333333333333333333333333333333333",
                    "value": amount + 1000,
                },
            }],
            "vout": [{
                "scriptpubkey": script_pubkey,
                "scriptpubkey_address": address,
                "value": amount,
            }],
        });
        tx_by_hash.insert(sh, tx);
    }

    let in_flight = AtomicUsize::new(0);
    let peak_in_flight = Arc::new(AtomicUsize::new(0));
    let first_hash = tx_by_hash
        .iter()
        .find(|(_, tx)| tx["vout"][0]["scriptpubkey"] == targets[0]["script_pubkey"])
        .unwrap()
        .0;
    let first_script = format!("/scripthash/{first_hash}");
    let cancel_book = Arc::new(Mutex::new(None::<std::path::PathBuf>));
    let cancelled = Arc::new(AtomicBool::new(false));
    let requests_after_cancel = Arc::new(AtomicUsize::new(0));

    let peak = peak_in_flight.clone();
    let cancel_path = cancel_book.clone();
    let did_cancel = cancelled.clone();
    let after_cancel = requests_after_cancel.clone();
    let server = FakeService::start_keep_alive(move |request| {
        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(15));
        let path = request.target.as_str();
        if let Some(suffix) = path.strip_prefix(&first_script) {
            if did_cancel.load(Ordering::SeqCst) {
                after_cancel.fetch_add(1, Ordering::SeqCst);
            } else if suffix == "/txs/chain" {
                if let Some(book) = cancel_path.lock().unwrap().as_ref() {
                    let control = Connection::open(book).unwrap();
                    assert_eq!(control.execute(
                        "UPDATE freshness_jobs SET cancel_requested=1 WHERE source_key='onchain_wallet:w1'",
                        [],
                    ).unwrap(), 1);
                    did_cancel.store(true, Ordering::SeqCst);
                }
            }
        }
        let reply = if path == "/blocks/tip/hash" {
            Reply::raw(200, "text/plain", tip_hash.as_bytes())
        } else if path == "/blocks/tip/height" {
            Reply::raw(200, "text/plain", b"105")
        } else if let Some(rest) = path.strip_prefix("/scripthash/") {
            let response = if let Some(hash) = rest.strip_suffix("/txs/chain") {
                json!(tx_by_hash.get(hash).into_iter().collect::<Vec<_>>())
            } else if rest.ends_with("/txs/mempool") {
                json!([])
            } else if let Some(hash) = rest.strip_suffix("/utxo") {
                let outputs: Vec<_> = tx_by_hash.get(hash).into_iter().map(|tx| json!({
                    "txid": tx["txid"],
                    "vout": 0,
                    "value": tx["vout"][0]["value"],
                    "status": tx["status"],
                })).collect();
                json!(outputs)
            } else {
                let count = usize::from(tx_by_hash.contains_key(rest));
                json!({
                    "chain_stats": {
                        "funded_txo_count": count,
                        "funded_txo_sum": count * 100_000,
                        "spent_txo_count": 0,
                        "spent_txo_sum": 0,
                        "tx_count": count,
                    },
                    "mempool_stats": {
                        "funded_txo_count": 0,
                        "funded_txo_sum": 0,
                        "spent_txo_count": 0,
                        "spent_txo_sum": 0,
                        "tx_count": 0,
                    },
                })
            };
            Reply::json(200, &response.to_string())
        } else {
            Reply::json(200, "{}")
        };
        in_flight.fetch_sub(1, Ordering::SeqCst);
        reply
    }).unwrap();
    let port = server.port();

    let case = sample();
    let mut conn = book(&case);
    conn.execute("DELETE FROM wallets", []).unwrap();
    for (idx, target) in targets.iter().enumerate() {
        let id = format!("w{}", idx + 1);
        let label = format!("W{}", idx + 1);
        let cfg = json!({
            "chain": "bitcoin",
            "network": "main",
            "backend": "fixture",
            "addresses": [target["address"].as_str().unwrap()],
        })
        .to_string();
        conn.execute(
            "INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
             VALUES(?, 'w', 'p', ?, 'addresses', ?, 'fixed')",
            kassiber_store::rusqlite::params![id, label, cfg],
        )
        .unwrap();
    }
    let backend_cfg = json!({
        "name": "fixture",
        "kind": "esplora",
        "chain": "bitcoin",
        "network": "main",
        "url": format!("http://127.0.0.1:{port}"),
        "batch_size": 4,
        "timeout": 5,
    });
    let req = Request {
        profile_id: "p",
        wallet: None,
        all: true,
        force_full: false,
        version: "test",
    };
    let scratch = std::env::temp_dir().join(format!(
        "kassiber-parallel-cancel-{}-{port}",
        std::process::id()
    ));
    std::fs::create_dir(&scratch).unwrap();
    let path = scratch.join("book.sqlite3");
    conn.execute_batch(
        "INSERT INTO settings(key,value) VALUES ('context_workspace','w'),('context_profile','p')",
    )
    .unwrap();
    conn.execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();

    // 1. Initial sync of all 6 wallets concurrently.
    let action = Sync::new(&conn, &req, |_, _| Ok(backend_cfg.clone())).unwrap();
    let consent = SyncConsent::explicit_trigger(&action);
    let results = action
        .run(
            &mut conn,
            &online(),
            Some(&consent),
            &Cancellation::default(),
            deadline(),
            1_700_010_000,
            &mut fixed(&case),
            |_, _| Ok(backend_cfg.clone()),
            None,
        )
        .unwrap();
    let labels: Vec<String> = results
        .iter()
        .map(|row| py_str(get(row, "wallet").unwrap()))
        .collect();
    assert_eq!(labels, vec!["W1", "W2", "W3", "W4", "W5", "W6"]);
    for row in &results {
        assert_eq!(get(row, "status"), Some(&s("synced")), "{row:?}");
    }
    let tx_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tx_count, 6, "every wallet transaction must be imported");
    assert!(
        peak_in_flight.load(Ordering::SeqCst) >= 2,
        "multi-wallet Sync::run must fetch concurrently (peak={})",
        peak_in_flight.load(Ordering::SeqCst)
    );
    let initial_requests = server.requests().len();
    let initial_conns = server.connection_count();
    assert!(
        initial_conns < initial_requests,
        "HTTP keep-alive pool must reuse sockets ({initial_conns} conns for {initial_requests} reqs)"
    );

    // 2. Incremental sync reuses complete membership checkpoints (far fewer requests).
    let reqs_before_inc = server.requests().len();
    let action2 = Sync::new(&conn, &req, |_, _| Ok(backend_cfg.clone())).unwrap();
    let consent2 = SyncConsent::explicit_trigger(&action2);
    let results2 = action2
        .run(
            &mut conn,
            &online(),
            Some(&consent2),
            &Cancellation::default(),
            deadline(),
            1_700_010_060,
            &mut fixed(&case),
            |_, _| Ok(backend_cfg.clone()),
            None,
        )
        .unwrap();
    assert_eq!(results2.len(), 6);
    let inc_requests = server.requests().len() - reqs_before_inc;
    assert!(
        inc_requests < initial_requests,
        "incremental sync ({inc_requests} reqs) must reuse checkpoints vs initial ({initial_requests} reqs)"
    );

    // 3. Offline gate enforces zero egress across all parallel wallet slots.
    let reqs_before_offline = server.requests().len();
    let action3 = Sync::new(&conn, &req, |_, _| Ok(backend_cfg.clone())).unwrap();
    let consent3 = SyncConsent::explicit_trigger(&action3);
    let offline_gate = Gate::new(|| Offline::Offline, Ledger::default());
    let offline_results = action3
        .run(
            &mut conn,
            &offline_gate,
            Some(&consent3),
            &Cancellation::default(),
            deadline(),
            1_700_010_120,
            &mut fixed(&case),
            |_, _| Ok(backend_cfg.clone()),
            None,
        )
        .unwrap();
    assert_eq!(offline_results.len(), 6);
    for row in &offline_results {
        assert_eq!(get(row, "status"), Some(&s("error")));
    }
    assert_eq!(
        server.requests().len(),
        reqs_before_offline,
        "offline mode must make zero network requests"
    );

    // Cancel a single queued job through another database connection while its
    // first history response is in flight. The other five jobs must finish.
    use crate::freshness::{execution, native, planning};
    let mut conn = Connection::open(&path).unwrap();
    let request = planning::Request {
        scope: planning::Scope::ActiveProfile,
        wallet: None,
        rates: Some(false),
        journals: Some(false),
        force_full: false,
        auto_pair: false,
        wallets_sync: true,
    };
    let plan =
        planning::Plan::new(&conn, &request, "test", |_, _| Ok(backend_cfg.clone())).unwrap();
    let proof = native::Consent::explicit_trigger(&plan);
    *cancel_book.lock().unwrap() = Some(path.clone());
    peak_in_flight.store(0, Ordering::SeqCst);
    let mut first_wallet_progress = false;
    let outcome = plan
        .execute_native(
            &mut conn,
            &execution::Options {
                run: true,
                limit: None,
            },
            &mut fixed(&case),
            &mut |event| {
                if get(event, "phase") == Some(&s("backend_fetch"))
                    && get(event, "wallet") == Some(&s("W1"))
                    && !cancelled.load(Ordering::SeqCst)
                {
                    first_wallet_progress = true;
                }
                Ok(())
            },
            &online(),
            Some(&proof),
            &Cancellation::default(),
            deadline(),
            1_700_010_180,
            "test",
            &kassiber_setup::backends::Config::load_with_env(
                &scratch.join("backends.json"),
                &Default::default(),
            )
            .unwrap(),
            |_, _| Ok(backend_cfg.clone()),
            None,
        )
        .unwrap();
    assert!(cancelled.load(Ordering::SeqCst), "{outcome:?}");
    assert!(
        first_wallet_progress,
        "fetch progress must arrive during the wave"
    );
    assert!(peak_in_flight.load(Ordering::SeqCst) >= 2);
    assert_eq!(requests_after_cancel.load(Ordering::SeqCst), 0);
    let mut query = conn.prepare("SELECT external_id FROM transactions WHERE profile_id='p' AND excluded=0 ORDER BY external_id").unwrap();
    let imported: Vec<String> = query
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(
        imported,
        (2..=6).map(|id| format!("{id:064x}")).collect::<Vec<_>>()
    );

    drop(server);
    drop(query);
    drop(conn);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&scratch).unwrap();
}
