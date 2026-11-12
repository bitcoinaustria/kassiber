use super::*;
use crate::publication::testbook::{load, snapshot, text};

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
        "../../../tests/vectors/python_wallet_publication.json"
    ))
    .unwrap()
}

fn replay(case: &Json, fail: Option<&str>) -> (Value, Connection, Value) {
    let mut conn = load(&py(&case["before"]).unwrap());
    let before = snapshot(&conn);
    let mut sources = Fixed {
        prefix: case["prefix"].as_str().unwrap().into(),
        ids: 0,
        now: case["now"].as_i64().unwrap(),
    };
    let result = apply(
        &mut conn,
        &Request {
            profile_id: "p",
            wallet_id: "a",
            captured_config: case["captured_config"].as_str().unwrap(),
            fetch: &case["fetch"],
            btcpay: None,
        },
        &mut sources,
        |stage| match fail {
            Some(target) if target == stage => Err(AppError::new("injected_failure", stage)),
            _ => Ok(()),
        },
    );
    let outcome = match result {
        Ok(mut results) => {
            for result in &mut results {
                if let Value::Dict(pairs) = result {
                    pairs.retain(
                        |(key, _)| !matches!(key, Key::Str(k) if k.as_str() == Some("elapsed_ms")),
                    );
                }
            }
            dict([("result", Value::List(results))])
        }
        Err(error) => dict([(
            "error",
            dict([("code", s(error.code)), ("message", s(error.message))]),
        )]),
    };
    (outcome, conn, before)
}

#[test]
fn publication_books_match_python_fixture_and_sequence_rounds() {
    let vectors = vectors();
    let cases = vectors["cases"].as_array().unwrap();
    for protocol in ["electrum", "esplora"] {
        for (name, rounds) in [
            ("empty", 1),
            ("receive-change", 1),
            ("checkpoint", 3),
            ("multi-script", 1),
            ("fixed", 1),
            ("gap-one", 1),
            ("reorg", 2),
            ("retraction", 2),
            ("mempool", 2),
            ("coinbase", 1),
            ("reanchor", 3),
            ("reorg-drop", 3),
            ("preflight-race", 3),
            ("mempool-drop-return", 3),
            ("replacement", 3),
            ("height-change", 3),
            ("force-full", 3),
            ("force-without-state", 3),
            ("reviewed-checkpoint", 3),
            ("reviewed-reorg", 2),
            ("reviewed-retraction", 2),
            ("reviewed-mempool", 2),
            ("reviewed-authority", 3),
        ] {
            for round in 0..rounds {
                let key = format!("{name}/{protocol}/{round}");
                assert_eq!(
                    cases
                        .iter()
                        .filter(|c| c["name"] == key && c["outcome"].get("result").is_some())
                        .count(),
                    1,
                    "missing or duplicated successful round {key}"
                );
            }
        }
    }
    let mut published = 0;
    for case in cases.iter().filter(|c| c.get("before").is_some()) {
        let name = case["name"].as_str().unwrap();
        let (outcome, conn, _) = replay(case, None);
        let expected = py(&case["outcome"]).unwrap();
        if get(&expected, "error").is_some() {
            assert_eq!(
                get(get(&outcome, "error").expect(name), "code"),
                get(get(&expected, "error").unwrap(), "code"),
                "{name}"
            );
        } else {
            assert_eq!(text(&outcome), text(&expected), "{name} outcome");
            published += 1;
        }
        assert_eq!(
            text(&snapshot(&conn)),
            text(&py(&case["after"]).unwrap()),
            "{name} rows"
        );
    }
    assert_eq!(published, 102);
}

#[test]
fn persistence_writers_require_the_wallet_transaction() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/esplora/0")
        .unwrap();
    let mut sources = Fixed {
        prefix: "guard".into(),
        ids: 0,
        now: 0,
    };
    let clock = Clock(RefCell::new(&mut sources), std::time::Instant::now());
    let conn = load(&py(&case["before"]).unwrap());
    let before = text(&snapshot(&conn));
    let config = s(case["captured_config"].as_str().unwrap());
    let results = [
        inventory::update(
            &conn,
            &inventory::Source {
                workspace_id: "w",
                profile_id: "p",
                wallet_id: "a",
                wallet_config_json: &config,
                backend_name: "fixture",
                backend_kind: "esplora",
                chain: "bitcoin",
                network: "main",
            },
            &[],
            "now",
        )
        .map(|_| ()),
        provenance::persist(
            &conn,
            &provenance::Target {
                workspace_id: "w",
                profile_id: "p",
                wallet_id: "a",
                application_revision: "revision",
                chain: "bitcoin",
                network: "main",
            },
            &[],
            &[],
            &mut || clock.now(),
        )
        .map(|_| ()),
        policy::ensure_active_epoch(
            &conn,
            "a",
            "bitcoin",
            "main",
            &mut || clock.id(),
            &mut || clock.now(),
        )
        .map(|_| ()),
        reports::record_wallet_changes(&conn, "p", "a", &[], &mut || clock.id(), &mut || {
            clock.now()
        })
        .map(|_| ()),
    ];
    for result in results {
        assert_eq!(
            result.unwrap_err().code,
            "observer_apply_outside_transaction"
        );
    }
    assert_eq!(text(&snapshot(&conn)), before);
}

#[test]
fn every_stage_failure_and_cancellation_restores_the_book() {
    let vectors = vectors();
    let mut checked = 0;
    for case in vectors["cases"].as_array().unwrap().iter().filter(|c| {
        c.get("before").is_some()
            && matches!(
                c["name"].as_str(),
                Some(
                    "receive-change/electrum/0"
                        | "receive-change/esplora/0"
                        | "reorg-drop/electrum/1"
                        | "replacement/esplora/1"
                )
            )
    }) {
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
            let (outcome, conn, before) = replay(case, Some(stage));
            assert_eq!(
                get(get(&outcome, "error").unwrap(), "code"),
                Some(&s("injected_failure")),
                "{stage}"
            );
            assert_eq!(
                text(&snapshot(&conn)),
                text(&before),
                "{} {stage}",
                case["name"]
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 32);
}

#[test]
fn unsupported_shapes_and_stale_preparations_refuse_without_writes() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/electrum/0")
        .unwrap();
    for (sql, code) in [
        ("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('b','w','p','Other','descriptor','{}','fixed')", ""),
        ("UPDATE wallets SET kind='lnd'", "native_fallback_required"),
        ("UPDATE profiles SET id=id", ""),
    ] {
        let mut conn = load(&py(&case["before"]).unwrap());
        conn.execute_batch(sql).unwrap();
        let before = snapshot(&conn);
        let mut sources = Fixed { prefix: "x".into(), ids: 0, now: 0 };
        let result = apply(
            &mut conn,
            &Request {
                profile_id: "p",
                wallet_id: "a",
                captured_config: case["captured_config"].as_str().unwrap(),
                fetch: &case["fetch"],
                btcpay: None,
            },
            &mut sources,
            |_| Ok(()),
        );
        if code.is_empty() {
            assert!(result.is_ok());
            continue;
        }
        assert_eq!(result.unwrap_err().code, code, "{sql}");
        assert_eq!(text(&snapshot(&conn)), text(&before), "{sql}");
    }
    let mut conn = load(&py(&case["before"]).unwrap());
    let before = snapshot(&conn);
    let mut stale = case["fetch"].clone();
    stale["book_environment_id"] = Json::from("other-environment");
    for (fetch, captured, code) in [
        (&case["fetch"], "{}", "stale_context"),
        (
            &stale,
            case["captured_config"].as_str().unwrap(),
            "stale_context",
        ),
    ] {
        let mut sources = Fixed {
            prefix: "x".into(),
            ids: 0,
            now: 0,
        };
        let error = apply(
            &mut conn,
            &Request {
                profile_id: "p",
                wallet_id: "a",
                captured_config: captured,
                fetch,
                btcpay: None,
            },
            &mut sources,
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(text(&snapshot(&conn)), text(&before));
    }
}

#[test]
fn nested_publication_releases_only_its_savepoint() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/esplora/0")
        .unwrap();
    let mut conn = load(&py(&case["before"]).unwrap());
    conn.execute_batch("BEGIN").unwrap();
    let mut sources = Fixed {
        prefix: case["prefix"].as_str().unwrap().into(),
        ids: 0,
        now: 0,
    };
    apply(
        &mut conn,
        &Request {
            profile_id: "p",
            wallet_id: "a",
            captured_config: case["captured_config"].as_str().unwrap(),
            fetch: &case["fetch"],
            btcpay: None,
        },
        &mut sources,
        |_| Ok(()),
    )
    .unwrap();
    assert!(!conn.is_autocommit(), "the caller's transaction stays open");
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        text(&snapshot(&conn)),
        text(&snapshot(&load(&py(&case["before"]).unwrap())))
    );
}

#[test]
fn publication_fences_observer_epochs_and_rolls_back_on_unwind() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "multi-script/electrum/0")
        .unwrap();
    let mut fetch = case["fetch"].clone();
    assert!(fetch["observer_updates"].as_array().unwrap().len() > 1);
    // Fail the second observer, after the first state and policy writes.
    fetch["observer_updates"][1]["expected_epoch"] = Json::from(999);
    let mut conn = load(&py(&case["before"]).unwrap());
    conn.execute_batch("BEGIN; UPDATE profiles SET label='caller-pending' WHERE id='p'")
        .unwrap();
    let before = text(&snapshot(&conn));
    for panic in [false, true] {
        let mut sources = Fixed {
            prefix: "fence".into(),
            ids: 0,
            now: 0,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            apply(
                &mut conn,
                &Request {
                    profile_id: "p",
                    wallet_id: "a",
                    captured_config: case["captured_config"].as_str().unwrap(),
                    fetch: if panic { &case["fetch"] } else { &fetch },
                    btcpay: None,
                },
                &mut sources,
                |stage| {
                    if panic && stage == "transaction_insertion" {
                        panic!("injected coordinator unwind");
                    }
                    Ok(())
                },
            )
        }));
        if panic {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().unwrap_err().code, "observer_state_stale");
        }
        assert!(!conn.is_autocommit(), "the caller transaction stays open");
        assert_eq!(text(&snapshot(&conn)), before);
    }
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn publication_rolls_back_at_every_row_write_including_cascades_and_reports() {
    use crate::publication::testbook::{watch_writes, write_trace};
    let vectors = vectors();
    let mut positions = 0;
    let mut operations = std::collections::BTreeSet::new();
    for case in vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case.get("before").is_some() && case["outcome"].get("result").is_some())
    {
        let initial = py(&case["before"]).unwrap();
        let run = |conn: &mut Connection| {
            let mut sources = Fixed {
                prefix: case["prefix"].as_str().unwrap().into(),
                ids: 0,
                now: case["now"].as_i64().unwrap(),
            };
            apply(
                conn,
                &Request {
                    profile_id: "p",
                    wallet_id: "a",
                    captured_config: case["captured_config"].as_str().unwrap(),
                    fetch: &case["fetch"],
                    btcpay: None,
                },
                &mut sources,
                |_| Ok(()),
            )
        };
        let mut success = load(&initial);
        watch_writes(&success, 0);
        run(&mut success).unwrap();
        assert_eq!(
            text(&snapshot(&success)),
            text(&py(&case["after"]).unwrap())
        );
        let trace = write_trace(&success);
        let mut conn = load(&initial);
        let before = text(&snapshot(&conn));
        watch_writes(&conn, 0);
        for (index, operation) in trace.iter().enumerate() {
            operations.insert(operation.clone());
            conn.execute(
                "UPDATE publication_write_failure SET position=?",
                [(index + 1) as i64],
            )
            .unwrap();
            let error = run(&mut conn).expect_err("every injected write must refuse");
            assert_eq!(error.code, "internal_error", "{error:?}");
            assert_eq!(
                error
                    .details
                    .as_ref()
                    .and_then(|d| get(d, "sqlite_error_code")),
                Some(&Value::Int(1811.into())),
                "the injected SQLITE_CONSTRAINT_TRIGGER must reach the caller"
            );
            assert_eq!(
                text(&snapshot(&conn)),
                before,
                "{} write {} {operation:?}",
                case["name"],
                index + 1
            );
            assert!(conn.is_autocommit());
            assert!(
                write_trace(&conn).is_empty(),
                "the trace must roll back too"
            );
            positions += 1;
        }
    }
    for (table, operation) in [
        ("transactions", "INSERT"),
        ("transactions", "UPDATE"),
        ("transactions", "DELETE"),
        ("chain_observation_provenance", "INSERT"),
        ("chain_observation_provenance", "UPDATE"),
        ("chain_observation_provenance", "DELETE"),
        ("profiles", "UPDATE"),
        ("wallet_policy_coverage_witnesses", "INSERT"),
        ("wallet_policy_coverage_witnesses", "UPDATE"),
        ("chain_analysis_watch_inbox", "INSERT"),
        ("filed_report_chain_impacts", "INSERT"),
        ("wallet_utxos", "INSERT"),
        ("wallet_utxos", "UPDATE"),
        ("freshness_source_states", "INSERT"),
        ("freshness_source_states", "UPDATE"),
    ] {
        assert!(
            operations.contains(&(table.into(), operation.into())),
            "missing {table} {operation}"
        );
    }
    eprintln!(
        "publication rollback: {positions} write positions, {} table/operation pairs",
        operations.len()
    );
    assert!(positions > 500, "{positions}");
}

#[test]
fn prospective_overlay_is_keyed_by_txid_and_asset() {
    use serde_json::json;
    let conn = load(&py(&vectors()["cases"][0]["before"]).unwrap());
    conn.execute("DELETE FROM transactions WHERE wallet_id='a'", [])
        .unwrap();
    let raw = r#"{"txid":"t"}"#;
    let record = |txid: &str, at: &str, direction: &str, asset: &str, amount: &str| {
        json!({"txid":txid,"occurred_at":at,"confirmed_at":at,"direction":direction,"asset":asset,
            "amount":amount,"fee":"0","fiat_rate":null,"fiat_value":null,"kind":"deposit",
            "description":"d","counterparty":null,"raw_json":raw})
    };
    let fund = "a".repeat(64);
    let spend = "b".repeat(64);
    let receipt = record(&fund, "2023-01-01T00:00:00Z", "inbound", "LBTC", "0.001");
    let issued = record(
        &fund,
        "2023-01-01T00:00:00Z",
        "inbound",
        &"c".repeat(64),
        "0.00007",
    );
    let debit = record(&spend, "2023-01-02T00:00:00Z", "outbound", "LBTC", "0.0006");
    let fetch = |records: Vec<Json>| {
        (
            json!({"adapter_meta":{},"normalized_records":[]}),
            vec![
                json!({"update":{"facts":{"transaction_records":records,"retracted_external_ids":[]}}}),
            ],
        )
    };
    let (f, updates) = fetch(vec![receipt.clone(), debit.clone()]);
    assert!(
        prospective_negative_events(&conn, "p", "a", &f, &updates, "fixture")
            .unwrap()
            .is_empty()
    );
    // Another asset of the same transaction keeps the L-BTC receipt.
    let (f, updates) = fetch(vec![receipt.clone(), issued.clone(), debit.clone()]);
    assert!(
        prospective_negative_events(&conn, "p", "a", &f, &updates, "fixture")
            .unwrap()
            .is_empty()
    );
    // The same (txid, asset) still replaces: the smaller receipt wins.
    let smaller = record(&fund, "2023-01-01T00:00:00Z", "inbound", "LBTC", "0.0001");
    let (f, updates) = fetch(vec![receipt.clone(), smaller, debit.clone()]);
    let events = prospective_negative_events(&conn, "p", "a", &f, &updates, "fixture").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(py_str(get(&events[0], "external_id").unwrap()), spend);
    // A stored funding row covers the debit until it is retracted.
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
         confirmed_at,direction,asset,amount,fee,raw_json,created_at)
         VALUES('stored-fund','w','p','a',?1,'fp','2023-01-01T00:00:00Z','2023-01-01T00:00:00Z',
         'inbound','LBTC',100000000,0,'{}','fixed')",
        [&fund],
    )
    .unwrap();
    let (f, updates) = fetch(vec![debit.clone()]);
    assert!(
        prospective_negative_events(&conn, "p", "a", &f, &updates, "fixture")
            .unwrap()
            .is_empty()
    );
    // A re-observed (txid, asset) replaces the stored row instead of adding.
    let shrunk = record(&fund, "2023-01-01T00:00:00Z", "inbound", "LBTC", "0.0001");
    let (f, updates) = fetch(vec![shrunk, debit.clone()]);
    assert_eq!(
        prospective_negative_events(&conn, "p", "a", &f, &updates, "fixture")
            .unwrap()
            .len(),
        1
    );
    let (mut f, updates) = fetch(vec![debit]);
    f["adapter_meta"]["observer_retracted_external_ids"] = json!([fund.to_uppercase()]);
    assert_eq!(
        prospective_negative_events(&conn, "p", "a", &f, &updates, "fixture")
            .unwrap()
            .len(),
        1
    );
}

/// `sync_wallet_from_backend`'s ownership re-check: a better owner that
/// appeared after preparation refuses the publication and writes nothing.
#[test]
fn publication_rechecks_target_ownership_against_the_current_book() {
    let vectors = vectors();
    let case = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "receive-change/electrum/0")
        .unwrap();
    let captured = case["captured_config"].as_str().unwrap();
    let config: Json = serde_json::from_str(captured).unwrap();
    let plan = crate::descriptors::load(&config).ok().unwrap().unwrap();
    let targets = plan.derive(None, 0, plan.gap_limit).ok().unwrap();
    let mut fetch = case["fetch"].clone();
    fetch["sync_state"]["targets"] = Json::from(targets);
    for (sql, code) in [
        ("SELECT 1", ""),
        (
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
             SELECT 'b','w','p','Alpha','descriptor',config_json,'fixed' FROM wallets WHERE id='a'",
            "source_overlap_retry",
        ),
    ] {
        let mut conn = load(&py(&case["before"]).unwrap());
        conn.execute_batch(sql).unwrap();
        let before = snapshot(&conn);
        let mut sources = Fixed {
            prefix: "x".into(),
            ids: 0,
            now: 0,
        };
        let result = apply(
            &mut conn,
            &Request {
                profile_id: "p",
                wallet_id: "a",
                captured_config: captured,
                fetch: &fetch,
                btcpay: None,
            },
            &mut sources,
            |_| Ok(()),
        );
        if code.is_empty() {
            assert!(result.is_ok());
            continue;
        }
        let error = result.unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.retryable);
        assert_eq!(
            error.message,
            "Wallet source overlap changed during refresh for Cold"
        );
        assert_eq!(text(&snapshot(&conn)), text(&before));
    }
}

/// Round 5 (7e8c323b9) through publication: a proven chain extension reuses
/// every history, the adapter skips the UTXO listing, and applying Python's
/// prepared fetch advances the stored confirmation counts from the anchor.
#[test]
fn reused_utxo_membership_refreshes_confirmations_through_publication() {
    let vectors: Json = serde_json::from_str(include_str!(
        "../../../tests/vectors/python_confirmation_publication.json"
    ))
    .unwrap();
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    let mut refreshed = 0;
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let (outcome, conn, _) = replay(case, None);
        assert_eq!(
            text(&outcome),
            text(&py(&case["outcome"]).unwrap()),
            "{name} outcome"
        );
        assert_eq!(
            text(&snapshot(&conn)),
            text(&py(&case["after"]).unwrap()),
            "{name} rows"
        );
        let skipped = case["fetch"]["adapter_meta"]["utxos_skipped_unchanged"] == true;
        let confirmations = |rows: &Json| -> Vec<Json> {
            rows.as_array()
                .into_iter()
                .flatten()
                .map(|row| row[14].clone())
                .collect()
        };
        let before = confirmations(&case["before"]["wallet_utxos"]["rows"]);
        let after = confirmations(&case["after"]["wallet_utxos"]);
        if skipped && before != after {
            refreshed += 1;
        }
    }
    // The Esplora extension round; the same-tip round changes nothing.
    assert_eq!(refreshed, 1);
}

/// `_merge_btcpay_enrichment` inside the wallet's savepoint: a descriptor
/// wallet's BTCPay provenance routes are read after the chain publication,
/// and any failure rolls the whole wallet back. Without Greenfield access
/// the routes refuse instead of being skipped.
#[test]
fn btcpay_provenance_routes_enrich_inside_the_savepoint() {
    let vectors = vectors();
    let case = &vectors["cases"][0];
    let mut config: Json = serde_json::from_str(case["captured_config"].as_str().unwrap()).unwrap();
    config["btcpay_provenance"] = serde_json::json!([{"backend":"merchant","store_id":"shop","payment_method_id":"btc-chain"}]);
    let captured = config.to_string();
    let mut conn = load(&py(&case["before"]).unwrap());
    conn.execute("UPDATE wallets SET config_json=? WHERE id='a'", [&captured])
        .unwrap();
    let before = snapshot(&conn);
    let backends = kassiber_setup::backends::Config::load_with_env(
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/sync-service-missing.env"
        )),
        &Default::default(),
    )
    .unwrap();
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Offline,
        kassiber_net::Ledger::default(),
    );
    let ctx = crate::sync::btcpay::Context {
        backends: &backends,
        gate: &gate,
        version: "test",
        book_id: "book",
        progress: None,
    };
    for btcpay in [None, Some(&ctx)] {
        let mut sources = Fixed {
            prefix: "p".into(),
            ids: 0,
            now: 0,
        };
        let error = apply(
            &mut conn,
            &Request {
                profile_id: "p",
                wallet_id: "a",
                captured_config: &captured,
                fetch: &case["fetch"],
                btcpay,
            },
            &mut sources,
            |_| Ok(()),
        )
        .unwrap_err();
        if btcpay.is_none() {
            assert_eq!(error.code, "native_fallback_required");
        } else {
            // The route's backend is resolved before any Greenfield read.
            assert_eq!(error.code, "app_error", "{error:?}");
            assert!(error.message.contains("'merchant'"), "{error:?}");
        }
        assert_eq!(snapshot(&conn), before, "the wallet's savepoint rolls back");
    }
}
