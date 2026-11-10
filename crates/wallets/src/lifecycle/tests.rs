use super::*;
use kassiber_envelope::get as field;

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let transcript: Value =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in transcript["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
        INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01'),('q','ws','Other','2025-01-01');
        INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('a','ws','p','treasury','Treasury','asset','2025-01-01'),('b','ws','q','treasury','Foreign','asset','2025-01-01');").unwrap();
    conn
}
fn create(conn: &Connection, label: &str, config: Value) -> String {
    let result = execute(
        conn,
        Operation::Create {
            workspace_id: "ws".into(),
            profile_id: "p".into(),
            label: label.into(),
            kind: "custom".into(),
            account: None,
            config,
        },
    )
    .unwrap();
    kassiber_envelope::py_str(field(&result, "id").unwrap())
}
fn update(conn: &Connection, id: &str, config: Value) -> Result<Py, AppError> {
    execute(
        conn,
        Operation::Update {
            profile_id: "p".into(),
            wallet: id.into(),
            label: None,
            account: None,
            config,
            clear: vec![],
        },
    )
}
fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}
fn descriptor_config() -> Value {
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/python_descriptors.json")).unwrap();
    vectors["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "kind-descriptor")
        .unwrap()["config"]
        .clone()
}
fn history(conn: &Connection, id: &str, raw: &str) {
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,occurred_at,direction,asset,amount,raw_json,created_at) VALUES('tx','ws','p',?,'fp','2025-01-01','inbound','BTC',1,?,'2025-01-01')",[id,raw]).unwrap();
}
#[test]
fn lifecycle_selection_storage_and_redaction() {
    let conn = fixture();
    let config = descriptor_config();
    let descriptor = config["descriptor"].as_str().unwrap();
    let id = create(&conn, "Vault", config.clone());
    let result = execute(
        &conn,
        Operation::Get {
            profile_id: "p".into(),
            wallet: " vAuLt ".into(),
        },
    )
    .unwrap();
    assert_eq!(field(&result, "id"), Some(&s(id.as_str())));
    assert_eq!(field(&result, "descriptor"), Some(&Py::Bool(true)));
    assert_eq!(
        field(field(&result, "config").unwrap(), "descriptor"),
        Some(&s("[redacted]"))
    );
    assert!(!kassiber_pyjson::dumps(&result, &Default::default())
        .unwrap()
        .contains(descriptor));
    let stored: String = conn
        .query_row("SELECT config_json FROM wallets WHERE id=?", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        stored,
        config::stored_json(&config::validate("custom", &config).unwrap())
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='q'"
        ),
        0
    );
    assert!(execute(
        &conn,
        Operation::Get {
            profile_id: "q".into(),
            wallet: id
        }
    )
    .is_err());
    let result = execute(
        &conn,
        Operation::List {
            profile_id: "q".into(),
        },
    )
    .unwrap();
    assert_eq!(result, Py::List(vec![]));
}
#[test]
fn spending_private_create_and_persisted_update_fail_without_writes() {
    let conn = fixture();
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/python_descriptors.json")).unwrap();
    let private = vectors["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "private-wpkh")
        .unwrap()["config"]
        .clone();
    let result = execute(
        &conn,
        Operation::Create {
            workspace_id: "ws".into(),
            profile_id: "p".into(),
            label: "Unsafe".into(),
            kind: "descriptor".into(),
            account: None,
            config: private.clone(),
        },
    )
    .unwrap_err();
    assert_eq!(result.code, "wallet_spending_private_material");
    assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM wallets"), 0);
    let id = create(&conn, "Legacy", json!({}));
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id=?",
        [config::stored_json(&private), id.clone()],
    )
    .unwrap();
    assert_eq!(
        update(&conn, &id, json!({"backend":"other"}))
            .unwrap_err()
            .code,
        "wallet_spending_private_material"
    );
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM wallet_policy_epochs"),
        0
    );
}
#[test]
fn policy_rotation_retains_material_and_is_atomic() {
    let conn = fixture();
    let id = create(&conn, "Vault", descriptor_config());
    let old = load(&conn, "p", &id).unwrap().config;
    update(&conn, &id, json!({"addresses":["different"]})).unwrap();
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM wallet_policy_epochs WHERE status='retired'"
        ),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM wallet_policy_epochs WHERE status='active'"
        ),
        1
    );
    let material: String = conn
        .query_row(
            "SELECT private_material_json FROM wallet_policy_epochs WHERE status='retired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        material,
        config::stored_json(&config::private_material(&old))
    );
    conn.execute_batch("CREATE TRIGGER fail_epoch BEFORE INSERT ON wallet_policy_epochs BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let before = load(&conn, "p", &id).unwrap().config;
    assert!(update(&conn, &id, json!({"addresses":["third"]})).is_err());
    assert_eq!(load(&conn, "p", &id).unwrap().config, before);
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM wallet_policy_epochs WHERE status='active'"
        ),
        1
    );
    assert!(conn.is_autocommit());
}
#[test]
fn scan_depth_gap_aliases_and_script_order_do_not_rotate_policy() {
    let conn = fixture();
    let mut config = descriptor_config();
    config["script_types"] = json!(["p2wpkh", "p2tr"]);
    let id = create(&conn, "Vault", config);
    update(
        &conn,
        &id,
        json!({"ownership_scan_to_index":50,"gap_limit":30}),
    )
    .unwrap();
    update(
        &conn,
        &id,
        json!({"chain":"BTC","network":"mainnet","script_types":["p2tr","p2wpkh","p2tr"]}),
    )
    .unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM wallet_policy_epochs"),
        0
    );
    update(&conn, &id, json!({"script_types":["p2tr"]})).unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM wallet_policy_epochs"),
        2
    );
}
#[test]
fn config_changes_reset_freshness_but_deprecation_and_scan_depth_preserve_checkpoint() {
    let conn = fixture();
    let id = create(&conn, "Vault", descriptor_config());
    let source = format!("onchain_wallet:{id}");
    conn.execute("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,state,checkpoint_json,updated_at) VALUES('p',?,'onchain_wallet','Vault','fresh','fresh','{\"scan\":12}','old')",[&source]).unwrap();
    update(&conn, &id, json!({"ownership_scan_to_index":100})).unwrap();
    let checkpoint: String = conn
        .query_row(
            "SELECT checkpoint_json FROM freshness_source_states",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint, "{\"scan\":12}");
    update(&conn, &id, json!({"backend":"changed"})).unwrap();
    let (status, checkpoint, stale): (String, String, String) = conn
        .query_row(
            "SELECT status,checkpoint_json,stale_reason FROM freshness_source_states",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (status.as_str(), checkpoint.as_str(), stale.as_str()),
        ("partially_stale", "{}", "wallet_config_changed")
    );
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM wallet_policy_epochs"),
        0
    );
    update(&conn, &id, json!({"deprecated":true})).unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM freshness_source_states"),
        0
    );
}
#[test]
fn history_and_book_binding_reject_relabeling_without_side_effects() {
    let conn = fixture();
    let id = create(&conn, "Vault", json!({"chain":"bitcoin","network":"main"}));
    history(&conn, &id, "{\"network\":\"main\"}");
    let error = update(&conn, &id, json!({"network":"regtest"})).unwrap_err();
    assert_eq!(
        error.message,
        "A connection's historical network cannot be changed; create a new connection"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        1
    );
    conn.execute_batch("DELETE FROM transactions").unwrap();
    update(&conn, &id, json!({"network":"regtest"})).unwrap();
    conn.execute_batch("INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES('p','e','test',1,NULL,'[]','[]','digest','now');").unwrap();
    assert_eq!(
        update(&conn, &id, json!({"backend":"local"}))
            .unwrap_err()
            .code,
        "book_network_mismatch"
    );
    let other = create(&conn, "New", json!({"addresses":["public"]}));
    assert_eq!(load(&conn, "p", &other).unwrap().config["network"], "test");
}
#[test]
fn contradictory_observation_history_never_becomes_configured_fact() {
    let conn = fixture();
    let id = create(&conn, "Vault", json!({}));
    history(&conn, &id, "{\"network\":\"test\"}");
    let error = update(&conn, &id, json!({"chain":"bitcoin","network":"main"})).unwrap_err();
    assert_eq!(
        error.message,
        "Observation contradicts its configured network"
    );
    assert_eq!(load(&conn, "p", &id).unwrap().config, json!({}));
}
#[test]
fn delete_guard_cascade_and_invalidation_are_profile_scoped() {
    let conn = fixture();
    let id = create(&conn, "Vault", json!({}));
    history(&conn, &id, "{}");
    let delete = |profile: &str, cascade| Operation::Delete {
        profile_id: profile.into(),
        wallet: id.clone(),
        cascade,
    };
    assert_eq!(
        execute(&conn, delete("q", true)).unwrap_err().code,
        "not_found"
    );
    assert_eq!(
        execute(&conn, delete("p", false)).unwrap_err().code,
        "conflict"
    );
    assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM transactions"), 1);
    let result = execute(&conn, delete("p", true)).unwrap();
    assert_eq!(field(&result, "cascaded_transactions"), Some(&Py::from(1)));
    assert_eq!(scalar(&conn, "SELECT COUNT(*) FROM transactions"), 0);
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        2
    );
}
#[test]
fn conflicting_labels_clear_fields_accounts_and_failed_invalidation_roll_back() {
    let conn = fixture();
    let id = create(&conn, "Vault", json!({"altbestand":true}));
    create(&conn, "Other", json!({}));
    let change = |label, account, clear| Operation::Update {
        profile_id: "p".into(),
        wallet: id.clone(),
        label,
        account,
        config: json!({}),
        clear,
    };
    assert_eq!(
        execute(&conn, change(Some("Other".into()), None, vec![]))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        execute(&conn, change(None, Some("b".into()), vec![]))
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(
        execute(&conn, change(None, None, vec!["unset".into()]))
            .unwrap_err()
            .code,
        "validation"
    );
    conn.execute_batch("CREATE TRIGGER fail_invalidation BEFORE UPDATE OF journal_input_version ON profiles BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(update(&conn, &id, json!({"backend":"local"})).is_err());
    assert_eq!(
        load(&conn, "p", &id).unwrap().config,
        json!({"altbestand":true})
    );
    assert!(conn.is_autocommit());
}

#[test]
fn alias_rewrites_preserve_observer_state_but_policy_changes_discard_it() {
    let conn = fixture();
    let id = create(&conn, "Vault", descriptor_config());
    let mut original = load(&conn, "p", &id).unwrap().config;
    original["chain"] = json!("btc");
    original["network"] = json!("mainnet");
    original["script_types"] = json!(["p2tr", "p2wpkh", "p2tr"]);
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id=?",
        params![config::stored_json(&original), id],
    )
    .unwrap();
    let w = load(&conn, "p", &id).unwrap();
    let epoch = ensure_epoch(&conn, &w, &config::private_material(&original)).unwrap();
    conn.execute("INSERT INTO chain_observer_instances(id,workspace_id,profile_id,logical_wallet_id,source_wallet_id,source_key,observer_kind,chain,network,state_version,state_json,created_at,updated_at) VALUES('observer','ws','p',?,?,'descriptor','bdk','bitcoin','main',1,'{}','old','old')",[&id,&id]).unwrap();
    update(
        &conn,
        &id,
        json!({"chain":"bitcoin","network":"main","script_types":["p2wpkh","p2tr"]}),
    )
    .unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM chain_observer_instances"),
        1
    );
    let active: String = conn
        .query_row(
            "SELECT id FROM wallet_policy_epochs WHERE status='active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, epoch);
    let material: String = conn
        .query_row(
            "SELECT private_material_json FROM wallet_policy_epochs WHERE status='active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        material,
        config::stored_json(&config::private_material(&original))
    );
    update(&conn, &id, json!({"script_types":["p2tr"]})).unwrap();
    assert_eq!(
        scalar(&conn, "SELECT COUNT(*) FROM chain_observer_instances"),
        0
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM wallet_policy_epochs WHERE status='retired'"
        ),
        1
    );
}

#[test]
fn malformed_config_and_payment_rail_evidence_preserve_reference_guards() {
    let conn = fixture();
    for input in [json!(true), json!(0), json!([]), Value::Null] {
        assert_eq!(
            network::new_config(&conn, "p", "coreln", input)
                .unwrap_err()
                .code,
            "validation"
        );
    }
    let id = create(
        &conn,
        "Rail",
        json!({"chain":"bitcoin", "network":"regtest"}),
    );
    history(
        &conn,
        &id,
        r#"{"source":"bullbitcoin_wallet_csv","network":"lightning"}"#,
    );
    update(&conn, &id, json!({"backend":"local"})).unwrap();
    conn.execute(
        "UPDATE transactions SET raw_json=?",
        [r#"{"network":"lightning"}"#],
    )
    .unwrap();
    assert_eq!(
        update(&conn, &id, json!({"backend":"other"}))
            .unwrap_err()
            .code,
        "book_network_mismatch"
    );
    assert_eq!(load(&conn, "p", &id).unwrap().config["backend"], "local");
}

#[test]
fn change_only_private_and_unsafe_legacy_reads_fail_closed() {
    let conn = fixture();
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/python_descriptors.json")).unwrap();
    let private = vectors["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "private-wpkh")
        .unwrap()["config"]["descriptor"]
        .clone();
    let config = json!({"change_descriptor":private});
    assert_eq!(
        config::validate("custom", &config).unwrap_err().code,
        "wallet_spending_private_material"
    );
    let id = create(&conn, "Legacy", json!({}));
    assert_eq!(
        update(&conn, &id, config).unwrap_err().code,
        "wallet_spending_private_material"
    );
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id=?",
        params![config::stored_json(&json!({"descriptor": private})), id],
    )
    .unwrap();
    for op in [
        Operation::List {
            profile_id: "p".into(),
        },
        Operation::Get {
            profile_id: "p".into(),
            wallet: id,
        },
    ] {
        assert_eq!(
            execute(&conn, op).unwrap_err().code,
            "wallet_spending_private_material"
        );
    }
}
#[test]
fn descriptor_list_default_gap_matches_reference() {
    let conn = fixture();
    let mut config = descriptor_config();
    config.as_object_mut().unwrap().remove("gap_limit");
    create(&conn, "Vault", config);
    let Py::List(rows) = execute(
        &conn,
        Operation::List {
            profile_id: "p".into(),
        },
    )
    .unwrap() else {
        panic!("list")
    };
    assert_eq!(field(&rows[0], "gap_limit"), Some(&Py::from(100)));
}

#[test]
fn policy_rotation_preserves_existing_coverage() {
    let conn = fixture();
    let id = create(&conn, "Vault", descriptor_config());
    let wallet = load(&conn, "p", &id).unwrap();
    let old = wallet.config.clone();
    let epoch = ensure_epoch(&conn, &wallet, &config::private_material(&old)).unwrap();
    conn.execute("INSERT INTO wallet_policy_sources(id,epoch_id,source_wallet_id,source_key,observer_kind,branch_keys_json,created_at,updated_at) VALUES('policy-source',?,?,'descriptor','bdk','[\"receive\"]','old','old')", [&epoch,&id]).unwrap();
    conn.execute("INSERT INTO wallet_policy_coverage_witnesses(source_id,branch_key,scanned_to_exclusive,highest_used,observer_kind,observed_at) VALUES('policy-source','receive',50,7,'bdk','old')", []).unwrap();
    update(&conn, &id, json!({"addresses":["different"]})).unwrap();
    assert_eq!(scalar(&conn,"SELECT scanned_to_exclusive FROM wallet_policy_coverage_witnesses WHERE source_id='policy-source'"),50);
    let retained_epoch: String = conn
        .query_row(
            "SELECT epoch_id FROM wallet_policy_sources WHERE id='policy-source'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained_epoch, epoch);
    assert_eq!(
        scalar(
            &conn,
            "SELECT COUNT(*) FROM wallet_policy_epochs WHERE status='retired'"
        ),
        1
    );
}

#[test]
fn birthdays_and_silent_payment_scan_keys_persist_without_spending_authority() {
    let conn = fixture();
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/python_descriptors.json")).unwrap();
    let config = vectors["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "sp-two-key")
        .unwrap()["config"]
        .clone();
    let mut c = config.clone();
    c["birthday"] = json!("2025-01-02T01:02:03 +01:00");
    let result = execute(
        &conn,
        Operation::Create {
            workspace_id: "ws".into(),
            profile_id: "p".into(),
            label: "SP".into(),
            kind: "silent-payment".into(),
            account: None,
            config: c,
        },
    )
    .unwrap();
    let id = kassiber_envelope::py_str(field(&result, "id").unwrap());
    assert_eq!(
        field(field(&result, "config").unwrap(), "sp_descriptor"),
        Some(&s("[redacted]"))
    );
    let raw: String = conn
        .query_row("SELECT config_json FROM wallets WHERE id=?", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    let stored: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["sp_descriptor"], config["sp_descriptor"]);
    assert_eq!(stored["birthday"], "2025-01-02T00:02:03Z");
    let before = scalar(
        &conn,
        "SELECT journal_input_version FROM profiles WHERE id='p'",
    );
    let private = vectors["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "sp-spend-private-nested")
        .unwrap()["config"]
        .clone();
    assert_eq!(
        update(&conn, &id, private).unwrap_err().code,
        "wallet_spending_private_material"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        before
    );
    let after: String = conn
        .query_row("SELECT config_json FROM wallets WHERE id=?", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(after, raw);
    assert_eq!(
        update(&conn, &id, json!({"birthday":"bad"}))
            .unwrap_err()
            .code,
        "validation"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT journal_input_version FROM profiles WHERE id='p'"
        ),
        before
    );
}
