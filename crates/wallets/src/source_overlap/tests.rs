use super::*;
use crate::publication::testbook::load;

fn book() -> Connection {
    let cases: Value = serde_json::from_str(include_str!(
        "../../tests/vectors/python_wallet_publication.json"
    ))
    .unwrap();
    load(&kassiber_pytext::loads(&cases["cases"][0]["before"].to_string()).unwrap())
}

fn config(conn: &Connection) -> String {
    conn.query_row("SELECT config_json FROM wallets WHERE id='a'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

fn add_wallet(conn: &Connection, id: &str, label: &str, kind: &str, config: &str) {
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,?,?,'fixed')",
        [id, label, kind, config],
    )
    .unwrap();
}

fn activity(conn: &Connection, wallet: &str) {
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,
         direction,asset,amount,fee,raw_json,created_at) VALUES(?1,'w','p',?2,?1,'f','2025-01-01T00:00:00Z',
         'inbound','BTC',1000,0,'{}','fixed')",
        [format!("tx-{wallet}"), wallet.to_owned()],
    )
    .unwrap();
}

fn overlap(conn: &Connection, label: &str) -> (Overlap, Vec<Value>, Plan) {
    let text = config(conn);
    let config: Value = serde_json::from_str(&text).unwrap();
    let plan = descriptors::load(&config).ok().unwrap().unwrap();
    let targets = plan.derive(None, 0, plan.gap_limit).ok().unwrap();
    let overlap = Overlap::capture(conn, "p", "a", label, "descriptor", &config).unwrap();
    (overlap, targets, plan)
}

fn source(wallet: &str, label: &str, kind: &str, source: &'static str) -> Source {
    Source {
        wallet: wallet.into(),
        label: label.into(),
        kind: kind.into(),
        chain: "bitcoin".into(),
        network: "main".into(),
        script: "51".into(),
        source,
        deprecated: false,
        count: 0,
        branch: None,
        index: None,
    }
}

#[test]
fn canonical_owner_ranks_like_python() {
    let a = source("a", "Zed", "descriptor", "descriptor_config");
    let b = source("b", "Alpha", "descriptor", "descriptor_config");
    // Equal evidence: the label decides.
    assert_eq!(recommended(&[&a, &b]), "b");
    // Activity beats the label.
    let active = Source {
        count: 2,
        ..a.clone()
    };
    assert_eq!(recommended(&[&active, &b]), "a");
    // A stored source beats a sync target, even with activity.
    let target = Source {
        count: 0,
        ..source("b", "Alpha", "descriptor", "sync_target")
    };
    assert_eq!(recommended(&[&a, &target]), "a");
    // An address-list wallet is not a canonical kind, even with inventory.
    let list = source("b", "Alpha", "address", "address_list");
    assert_eq!(recommended(&[&a, &list]), "a");
    let listed = source("b", "Alpha", "address", "inventory");
    assert_eq!(recommended(&[&a, &listed]), "a");
    // A deprecated wallet never wins.
    let retired = Source {
        deprecated: true,
        count: 5,
        ..b.clone()
    };
    assert_eq!(recommended(&[&a, &retired]), "a");
    // A wallet's row aggregates every source, in any order: a sync target
    // (no activity) seen first does not hide the wallet's activity, and one
    // retired source retires the wallet.
    let first = Source {
        count: 0,
        ..source("a", "Zed", "descriptor", "sync_target")
    };
    assert_eq!(recommended(&[&first, &active, &b]), "a");
    assert_eq!(recommended(&[&active, &first, &b]), "a");
    assert_eq!(recommended(&[&b, &first, &active]), "a");
    let retired_target = Source {
        deprecated: true,
        ..first.clone()
    };
    assert_eq!(recommended(&[&active, &retired_target, &b]), "b");
    assert_eq!(recommended(&[&retired_target, &active, &b]), "b");
}

#[test]
fn a_lone_wallet_and_distinct_wallets_keep_every_target() {
    let conn = book();
    let (lone, targets, plan) = overlap(&conn, "Cold");
    for indexed in [true, false] {
        assert!(lone
            .dropped(&targets, &plan.chain, &plan.network, indexed)
            .is_empty());
    }
    add_wallet(
        &conn,
        "b",
        "Alpha",
        "descriptor",
        r#"{"addresses":[],"chain":"bitcoin"}"#,
    );
    let (other, targets, plan) = overlap(&conn, "Cold");
    assert!(other
        .dropped(&targets, &plan.chain, &plan.network, true)
        .is_empty());
}

#[test]
fn a_duplicate_descriptor_drops_every_target_of_the_later_label() {
    let conn = book();
    add_wallet(&conn, "b", "Alpha", "descriptor", &config(&conn));
    let (cold, targets, plan) = overlap(&conn, "Cold");
    for indexed in [true, false] {
        let dropped = cold.dropped(&targets, &plan.chain, &plan.network, indexed);
        assert_eq!(dropped.len(), targets.len());
        assert!(keep(&targets, &dropped).is_empty());
    }
}

#[test]
fn deprecated_wallets_without_activity_are_ignored() {
    let conn = book();
    let mut retired: Value = serde_json::from_str(&config(&conn)).unwrap();
    retired["deprecated"] = json!(true);
    add_wallet(&conn, "b", "Alpha", "descriptor", &retired.to_string());
    let (cold, targets, plan) = overlap(&conn, "Cold");
    assert!(cold
        .dropped(&targets, &plan.chain, &plan.network, false)
        .is_empty());
    // Even against a retired candidate, which would lose to it on the label.
    let retired_cold = Overlap::capture(&conn, "p", "a", "Cold", "descriptor", &retired).unwrap();
    for indexed in [true, false] {
        assert!(retired_cold
            .dropped(&targets, &plan.chain, &plan.network, indexed)
            .is_empty());
    }
    // With activity the retired wallet is evidence again, but never canonical.
    activity(&conn, "b");
    let (cold, targets, plan) = overlap(&conn, "Cold");
    assert!(cold
        .dropped(&targets, &plan.chain, &plan.network, false)
        .is_empty());
    retired["deprecated"] = json!("maybe");
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id='b'",
        [retired.to_string()],
    )
    .unwrap();
    let text = config(&conn);
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let error = Overlap::capture(&conn, "p", "a", "Cold", "descriptor", &parsed)
        .err()
        .unwrap();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("app_error", "Invalid boolean value: maybe")
    );
}

#[test]
fn prefetch_index_and_publish_recheck_name_one_owner() {
    // tests/rust_port/repro_source_overlap_index_order.py: the two passes
    // list the sources in different orders and must still agree.
    let conn = book();
    add_wallet(&conn, "b", "Alpha", "descriptor", &config(&conn));
    activity(&conn, "a");
    let (cold, targets, plan) = overlap(&conn, "Cold");
    for indexed in [true, false] {
        assert!(cold
            .dropped(&targets, &plan.chain, &plan.network, indexed)
            .is_empty());
    }
    let alpha_config: Value = serde_json::from_str(&config(&conn)).unwrap();
    let alpha = Overlap::capture(&conn, "p", "b", "Alpha", "descriptor", &alpha_config).unwrap();
    for indexed in [true, false] {
        assert_eq!(
            alpha
                .dropped(&targets, &plan.chain, &plan.network, indexed)
                .len(),
            targets.len()
        );
    }
}

#[test]
fn inventory_and_address_lists_are_evidence() {
    let conn = book();
    let (_, targets, plan) = overlap(&conn, "Cold");
    let script = targets[1]["script_pubkey"].as_str().unwrap().to_owned();
    add_wallet(&conn, "b", "Alpha", "descriptor", r#"{"chain":"bitcoin"}"#);
    conn.execute(
        "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,txid,vout,outpoint,asset,amount,chain,network,
         script_pubkey,confirmation_status,first_seen_at,last_seen_at) VALUES('u','w','p','b','t',0,'t:0','BTC',1,
         'bitcoin','main',?,'confirmed','fixed','fixed')",
        [script.to_uppercase()],
    )
    .unwrap();
    let (cold, _, _) = overlap(&conn, "Cold");
    for indexed in [true, false] {
        let dropped = cold.dropped(&targets, &plan.chain, &plan.network, indexed);
        assert_eq!(dropped, BTreeSet::from([script.clone()]));
        assert_eq!(keep(&targets, &dropped).len(), targets.len() - 1);
    }
    // Another network's evidence is a different script group.
    conn.execute("UPDATE wallet_utxos SET network='test'", [])
        .unwrap();
    let (cold, _, _) = overlap(&conn, "Cold");
    assert!(cold
        .dropped(&targets, &plan.chain, &plan.network, true)
        .is_empty());
    // An address list never outranks a descriptor wallet.
    conn.execute("DELETE FROM wallet_utxos", []).unwrap();
    let address = targets[1]["address"].as_str().unwrap();
    add_wallet(
        &conn,
        "c",
        "Aardvark",
        "address",
        &json!({"addresses":[address],"chain":"bitcoin","network":"main"}).to_string(),
    );
    let (cold, _, _) = overlap(&conn, "Cold");
    assert!(cold
        .dropped(&targets, &plan.chain, &plan.network, false)
        .is_empty());
}

#[test]
fn descriptor_evidence_stops_at_each_wallet_horizon() {
    // Alpha watches the same descriptor with a smaller gap, so a target past
    // Alpha's horizon has no other owner.
    let conn = book();
    let mut narrow: Value = serde_json::from_str(&config(&conn)).unwrap();
    narrow["gap_limit"] = json!(1);
    add_wallet(&conn, "b", "Alpha", "descriptor", &narrow.to_string());
    let (cold, _, plan) = overlap(&conn, "Cold");
    let deep = plan.derive(Some(0), 5, 6).ok().unwrap();
    for indexed in [true, false] {
        // Index 5 is past Alpha's horizon (gap 1): no overlap.
        assert!(cold
            .dropped(&deep, &plan.chain, &plan.network, indexed)
            .is_empty());
    }
    let shallow = plan.derive(Some(0), 0, 1).ok().unwrap();
    for indexed in [true, false] {
        assert_eq!(
            cold.dropped(&shallow, &plan.chain, &plan.network, indexed)
                .len(),
            1
        );
    }
}

#[test]
fn discovered_targets_follow_python_sources_and_order() {
    let outputs = json!({"facts":{"outputs":[{"script_pubkey":" AB ","branch_index":0,"address_index":7},
        {"scriptpubkey":"cd"},{"script_pubkey":"zz"}],
        "transaction_records":[{"raw_json":"{\"observer_owned_scripts\":[\"ab\",\"ef\"],\"vin\":[{\"prevout\":{\"role\":\"owned\",\"script_pubkey\":\"0102\"}}],\"vout\":[{\"role\":\"external\",\"script_pubkey\":\"99\"},{\"role\":\"owned\",\"script_pubkey\":\"0304\"}]}"},
        {"raw_json":"not json"},"skip"]}});
    let found = discovered(&[&outputs]);
    let scripts: Vec<&str> = found
        .iter()
        .map(|t| t["script_pubkey"].as_str().unwrap())
        .collect();
    assert_eq!(scripts, ["ab", "cd", "ef", "0102", "0304"]);
    assert_eq!(found[0]["address_index"], 7);
    let targets = vec![json!({"script_pubkey":"ab"}), json!({"script_pubkey":"AB"})];
    assert!(expanded(&targets, &found[..1]).is_none());
    let grown = expanded(&targets, &found).unwrap();
    let scripts: Vec<&str> = grown
        .iter()
        .map(|t| t["script_pubkey"].as_str().unwrap())
        .collect();
    // The second preflight spelling replaces the first in place.
    assert_eq!(scripts, ["AB", "cd", "ef", "0102", "0304"]);
}
