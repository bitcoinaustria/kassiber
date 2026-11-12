use super::*;
use kassiber_net::{Gate, Ledger, Offline};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/vectors/python_observer_state.json"
    ))
    .unwrap()
}

#[test]
fn observer_bdk_all_components_roundtrip_python_serialized_order_and_exact_quantities() {
    for case in vectors()["changesets"].as_array().unwrap() {
        assert_eq!(
            normalize_changeset(&case["input"]).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
        assert_eq!(
            normalize_changeset(&case["expected"]).unwrap(),
            case["expected"]
        );
    }
}

#[test]
fn observer_bdk_duplicate_lists_and_hex_whitespace_match_python_without_collapsing_entries() {
    for case in vectors()["changesets"].as_array().unwrap() {
        if matches!(
            case["name"].as_str(),
            Some("duplicate-lists" | "hex-whitespace")
        ) {
            assert_eq!(
                normalize_changeset(&case["input"]).unwrap(),
                case["expected"]
            );
        }
    }
}

#[test]
fn observer_bdk_network_binds_every_extended_key_and_hex_refuses_split_bytes_or_unicode_spaces() {
    for case in vectors()["rejected_changesets"].as_array().unwrap() {
        assert_eq!(
            normalize_changeset(&case["input"]).unwrap_err().code,
            "observer_state_rebuild_required",
            "{}",
            case["name"]
        );
    }
}

#[test]
fn observer_bdk_unknown_missing_malformed_components_fail_closed_without_material_in_errors() {
    let vectors = vectors();
    let original = &vectors["changesets"][1]["input"];
    for field in original.as_object().unwrap().keys() {
        let mut payload = original.clone();
        payload.as_object_mut().unwrap().remove(field);
        assert_eq!(
            normalize_changeset(&payload).unwrap_err().code,
            "observer_state_rebuild_required"
        );
    }
    let mut bad = vec![];
    for (path, value) in [
        ("schema_version", json!(2)),
        ("network", json!("unknown")),
        ("descriptor", json!("private material")),
        ("local_chain", json!({})),
        (
            "indexer",
            json!([{"descriptor_id":"00","last_revealed":-1}]),
        ),
        (
            "locked_outpoints",
            json!([{"txid":"00".repeat(32),"vout":0,"locked":1}]),
        ),
    ] {
        let mut p = original.clone();
        p[path] = value;
        if path == "network" {
            // Isolate the network enum check: wrong-network descriptors would
            // otherwise mask a missing enum guard with a different refusal.
            p["descriptor"] = Value::Null;
            p["change_descriptor"] = Value::Null;
        }
        bad.push(p);
    }
    let mut p = original.clone();
    p["extra"] = json!(true);
    bad.push(p);
    let mut p = original.clone();
    p["tx_graph"]["txs"] = json!(["not a transaction"]);
    bad.push(p);
    let mut p = original.clone();
    p["tx_graph"]["txouts"][0]["value_sats"] = json!(-1);
    bad.push(p);
    let mut p = original.clone();
    p["tx_graph"]["anchors"][0]["block_height"] = json!(u64::MAX);
    bad.push(p);
    let mut p = original.clone();
    p["tx_graph"]["unknown"] = json!([]);
    bad.push(p);
    let mut p = original.clone();
    p["tx_graph"]["last_seen"][0]["timestamp"] = json!(-1);
    bad.push(p);
    let mut p = original.clone();
    p["local_chain"][0]["hash"] = json!("00");
    bad.push(p);
    for payload in bad {
        let error = normalize_changeset(&payload).unwrap_err();
        assert_eq!(error.code, "observer_state_rebuild_required");
        assert!(!format!("{error:?}").contains("private material"));
    }
}

#[test]
fn observer_commitments_match_python_unicode_floats_opaque_json_and_integer_msat() {
    let vectors = vectors();
    for case in vectors["graphs"].as_array().unwrap() {
        assert_eq!(
            graph_hash(case["input"].as_str().unwrap()).unwrap(),
            case["expected"]
        );
    }
    for case in vectors["quantities"].as_array().unwrap() {
        assert_eq!(
            quantity_hash(&py(&case["input"]).unwrap()).unwrap(),
            case["expected"]
        );
    }
    let i = &vectors["identity"];
    assert_eq!(
        instance_id(
            i["logical_wallet"].as_str().unwrap(),
            i["source_wallet"].as_str().unwrap(),
            i["source_key"].as_str().unwrap()
        ),
        i["expected"]
    );
}

#[test]
fn observer_fingerprints_match_python_and_bind_every_record_input_except_depth() {
    let vectors = vectors();
    for case in vectors["fingerprints"].as_array().unwrap() {
        assert_eq!(
            record_fingerprint(case["version"].as_str().unwrap(), &case["record"]).unwrap(),
            case["expected"]
        );
    }
    let case = &vectors["fingerprints"][0];
    let record = &case["record"];
    let version = case["version"].as_str().unwrap();
    for field in [
        "txid",
        "vin",
        "vout",
        "position",
        "observed_at",
        "owned_scripts",
        "backend",
    ] {
        let mut changed = record.clone();
        changed[field] = if field == "position" {
            json!({"status":{"confirmed":false}})
        } else {
            json!("changed")
        };
        assert_ne!(
            record_fingerprint(version, &changed).unwrap(),
            case["expected"],
            "{field}"
        );
    }
    assert_ne!(
        record_fingerprint("1:other-release", record).unwrap(),
        case["expected"]
    );
    assert_eq!(
        vectors["fingerprints"][0]["expected"],
        vectors["fingerprints"][1]["expected"]
    );
}

#[test]
fn observer_reuse_requires_confirmed_current_single_observer_same_release_incremental_state() {
    let prior =
        json!({"record_fingerprint_version":"1:release","record_fingerprints":{"a":"fingerprint"}});
    let current = || Some(BTreeSet::from(["a".to_string()]));
    let reuse = RecordReuse::new(Some(&prior), false, "1:release", current());
    assert!(reuse.can_reuse("a", true, "fingerprint"));
    assert!(!reuse.can_reuse("a", false, "fingerprint"));
    assert!(!reuse.can_reuse("a", true, "changed"));
    assert!(!reuse.can_reuse("b", true, "fingerprint"));
    for gate in [
        RecordReuse::new(None, false, "1:release", current()),
        RecordReuse::new(Some(&prior), true, "1:release", current()),
        RecordReuse::new(Some(&prior), false, "1:other", current()),
        RecordReuse::new(Some(&prior), false, "1:release", None),
        RecordReuse::new(Some(&prior), false, "1:release", Some(BTreeSet::new())),
    ] {
        assert!(!gate.can_reuse("a", true, "fingerprint"));
    }
    let malformed = json!({"record_fingerprint_version":"1:release","record_fingerprints":[]});
    assert!(
        !RecordReuse::new(Some(&malformed), false, "1:release", current()).can_reuse(
            "a",
            true,
            "fingerprint"
        )
    );
}

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE transactions(id TEXT,wallet_id TEXT,external_id TEXT,direction TEXT,asset TEXT,amount INTEGER,fee INTEGER,amount_includes_fee INTEGER,raw_json TEXT,excluded INTEGER);
        CREATE TABLE chain_observation_provenance(transaction_id TEXT,authority_version INTEGER,graph_hash TEXT,quantity_hash TEXT,observer_ids_json TEXT);
        CREATE TABLE chain_observer_instances(id TEXT,observer_kind TEXT,chain TEXT,network TEXT,logical_wallet_id TEXT,source_wallet_id TEXT,source_key TEXT,state_version INTEGER,state_epoch INTEGER,state_json TEXT);
        CREATE TABLE chain_observer_coverage(observer_id TEXT,branch_key TEXT,coverage_version INTEGER,highest_used INTEGER,scanned_to INTEGER,details_json TEXT);").unwrap();
    conn
}

fn insert_observed(conn: &Connection, id: &str, txid: &str, observer_ids: Value) {
    let row = json!({"wallet_id":"w","external_id":txid,"direction":"inbound","asset":"BTC","amount":9007199254740993i64,"fee":1001,"amount_includes_fee":true});
    conn.execute(
        "INSERT INTO transactions VALUES(?,'w',?,'inbound','BTC',9007199254740993,1001,1,'{}',0)",
        [id, txid],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chain_observation_provenance VALUES(?,1,?,?,?)",
        [
            id,
            &graph_hash("{}").unwrap(),
            &quantity_hash(&py(&row).unwrap()).unwrap(),
            &observer_ids.to_string(),
        ],
    )
    .unwrap();
}

#[test]
fn observer_current_rows_require_both_commitments_authority_scope_and_every_active_sibling() {
    let conn = database();
    let txid = "ab".repeat(32);
    insert_observed(&conn, "good", &txid, json!(["one"]));
    assert_eq!(
        current_observed_txids(&conn, "w", "one").unwrap(),
        BTreeSet::from([txid.clone()])
    );
    assert!(current_observed_txids(&conn, "other-wallet", "one")
        .unwrap()
        .is_empty());
    for sql in [
        "UPDATE transactions SET amount=amount+1",
        "UPDATE transactions SET fee=fee+1",
        "UPDATE transactions SET amount_includes_fee=0",
        "UPDATE transactions SET direction='outbound'",
        "UPDATE transactions SET asset='LBTC'",
        "UPDATE transactions SET raw_json='{\"changed\":true}'",
        "UPDATE transactions SET excluded=1",
        "UPDATE chain_observation_provenance SET authority_version=2",
        "UPDATE chain_observation_provenance SET observer_ids_json='[\"other\"]'",
        "UPDATE chain_observation_provenance SET observer_ids_json='[\"one\",\"two\"]'",
        "UPDATE chain_observation_provenance SET observer_ids_json='broken'",
        "DELETE FROM chain_observation_provenance",
    ] {
        conn.execute_batch("SAVEPOINT attempt").unwrap();
        conn.execute_batch(sql).unwrap();
        assert!(
            current_observed_txids(&conn, "w", "one")
                .unwrap()
                .is_empty(),
            "{sql}"
        );
        conn.execute_batch("ROLLBACK TO attempt;RELEASE attempt")
            .unwrap();
    }
    insert_observed(&conn, "stale", &txid.to_uppercase(), json!(["two"]));
    assert!(current_observed_txids(&conn, "w", "one")
        .unwrap()
        .is_empty());
    conn.execute("UPDATE transactions SET excluded=1 WHERE id='stale'", [])
        .unwrap();
    assert_eq!(
        current_observed_txids(&conn, "w", "one").unwrap(),
        BTreeSet::from([txid.clone()])
    );
    conn.execute("UPDATE transactions SET raw_json=NULL WHERE id='good'", [])
        .unwrap();
    conn.execute(
        "UPDATE chain_observation_provenance SET graph_hash=? WHERE transaction_id='good'",
        [hash(b"null")],
    )
    .unwrap();
    assert_eq!(
        current_observed_txids(&conn, "w", "one").unwrap(),
        BTreeSet::from([txid])
    );
}

fn identity() -> Identity<'static> {
    Identity {
        id: "i",
        observer_kind: "bdk",
        chain: "bitcoin",
        network: "main",
        logical_wallet_id: "w",
        source_wallet_id: "w",
        source_key: "descriptor:default",
    }
}

fn seed_state(conn: &Connection) {
    conn.execute_batch("INSERT INTO chain_observer_instances VALUES('i','bdk','bitcoin','main','w','w','descriptor:default',1,7,'{\"schema_version\":1,\"canonical_txids\":[\"known\"]}');
        INSERT INTO chain_observer_coverage VALUES('i','receive',2,4,25,'{\"observer\":\"bdk\"}');
        INSERT INTO chain_observer_coverage VALUES('i','change',2,NULL,20,'{}');").unwrap();
}

#[test]
fn observer_snapshot_restores_epoch_exact_exclusive_coverage_and_refuses_incompatible_state() {
    let conn = database();
    assert_eq!(
        load_state(&conn, &identity(), false)
            .unwrap()
            .expected_epoch,
        None
    );
    seed_state(&conn);
    let loaded = load_state(&conn, &identity(), false).unwrap();
    assert_eq!(loaded.expected_epoch, Some(7));
    assert_eq!(loaded.prior.unwrap()["canonical_txids"], json!(["known"]));
    assert_eq!(
        loaded.coverage,
        vec![
            json!({"branch_key":"change","highest_used":null,"scanned_to":20,"details":{}}),
            json!({"branch_key":"receive","highest_used":4,"scanned_to":25,"details":{"observer":"bdk"}})
        ]
    );
    assert!(conn.is_autocommit());
    for sql in [
        "UPDATE chain_observer_instances SET state_version=2",
        "UPDATE chain_observer_instances SET observer_kind='compatibility'",
        "UPDATE chain_observer_instances SET chain='liquid'",
        "UPDATE chain_observer_instances SET network='regtest'",
        "UPDATE chain_observer_instances SET logical_wallet_id='other'",
        "UPDATE chain_observer_instances SET source_wallet_id='other'",
        "UPDATE chain_observer_instances SET source_key='xpub:p2wpkh'",
        "UPDATE chain_observer_instances SET state_json='invalid'",
        "UPDATE chain_observer_instances SET state_json='[]'",
        "UPDATE chain_observer_coverage SET coverage_version=1",
        "UPDATE chain_observer_coverage SET details_json='[]'",
        "UPDATE chain_observer_coverage SET details_json='invalid'",
    ] {
        conn.execute_batch(sql).unwrap();
        let error = match load_state(&conn, &identity(), false) {
            Err(e) => e,
            Ok(_) => panic!("accepted {sql}"),
        };
        assert_eq!(error.code, "observer_state_rebuild_required", "{sql}");
        let forced = load_state(&conn, &identity(), true).unwrap();
        assert!(forced.prior.is_none());
        assert!(forced.coverage.is_empty());
        assert_eq!(forced.expected_epoch, Some(7));
        assert!(conn.is_autocommit());
        conn.execute_batch(
            "DELETE FROM chain_observer_instances;DELETE FROM chain_observer_coverage",
        )
        .unwrap();
        seed_state(&conn);
    }
    conn.execute_batch("BEGIN").unwrap();
    let error = match load_state(&conn, &identity(), true) {
        Err(e) => e,
        Ok(_) => panic!("accepted active transaction"),
    };
    assert_eq!(error.code, "observer_prepare_in_transaction");
    assert!(!conn.is_autocommit());
    conn.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn observer_state_load_and_reuse_are_inert_without_consent_online_and_offline() {
    let conn = database();
    seed_state(&conn);
    let writes = conn.total_changes();
    for offline in [Offline::Online, Offline::Offline] {
        let gate = Gate::new(move || offline, Ledger::default());
        let loaded = load_state(&conn, &identity(), false).unwrap();
        assert_eq!(loaded.expected_epoch, Some(7));
        assert!(current_observed_txids(&conn, "w", "one")
            .unwrap()
            .is_empty());
        for case in vectors()["changesets"].as_array().unwrap() {
            normalize_changeset(&case["input"]).unwrap();
        }
        assert!(gate.ledger().snapshot(0, 100).records.is_empty());
        assert!(gate.attempts().records.is_empty());
        assert_eq!(conn.total_changes(), writes);
    }
}

#[test]
fn observer_full_rebuild_discards_only_wallet_state_and_never_borrows_another_checkpoint() {
    let payload = vectors()["changesets"][1]["expected"].clone();
    let original = json!({"schema_version":1,"bdk_changeset":payload,"canonical_txids":["known"]});
    let mut loaded = LoadedState {
        prior: Some(original.clone()),
        coverage: vec![],
        expected_epoch: Some(7),
    };
    assert_eq!(loaded.bdk_changeset(false).unwrap(), Some(payload));
    assert!(loaded.bdk_changeset(true).unwrap().is_none());
    assert_eq!(loaded.prior, Some(original));
    assert_eq!(loaded.expected_epoch, Some(7));
    for prior in [
        json!({"schema_version":2,"bdk_changeset":vectors()["changesets"][1]["expected"]}),
        json!({"schema_version":1}),
        json!({"schema_version":1,"bdk_changeset":[]}),
    ] {
        loaded.prior = Some(prior.clone());
        let expected = if prior["schema_version"] == 2 {
            AppError::new(
                "observer_state_rebuild_required",
                "Stored BDK observer state must be rebuilt",
            )
            .with_hint("Run a full wallet refresh to rebuild derived BDK observer state.")
        } else {
            AppError::new(
                "observer_state_rebuild_required",
                "Stored BDK observer state is incomplete",
            )
        };
        assert_eq!(loaded.bdk_changeset(false).unwrap_err(), expected);
        assert!(loaded.bdk_changeset(true).unwrap().is_none());
        assert_eq!(loaded.prior, Some(prior));
    }
    assert_eq!(
        identity_checkpoint(&json!({"canonical_txids":["legacy"]}), "i"),
        json!({"canonical_txids":["legacy"]})
    );
    let checkpoint = json!({"canonical_txids":["aggregate"],"observer_instances":{"i":{"canonical_txids":["own"]},"other":{"canonical_txids":["foreign"]},"bad":[]}});
    assert_eq!(
        identity_checkpoint(&checkpoint, "i"),
        json!({"canonical_txids":["own"]})
    );
    assert_eq!(identity_checkpoint(&checkpoint, "absent"), json!({}));
    assert_eq!(identity_checkpoint(&checkpoint, "bad"), json!({}));
}
