//! Inert, test-only observer-state oracle. Never registered by the product.
use kassiber_store::rusqlite::{Connection, OpenFlags};
use kassiber_wallets::observer_state::{self as state, Identity};
use serde_json::{json, Value};
use std::io::Read;

fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let input: Value = serde_json::from_str(&raw).unwrap();
    let result = match input["operation"].as_str().unwrap() {
        "changeset" => state::normalize_changeset(&input["payload"]),
        "fingerprint" => {
            state::record_fingerprint(input["version"].as_str().unwrap(), &input["record"])
                .map(|v| json!(v))
        }
        "load" => {
            let conn = Connection::open_with_flags(
                input["database"].as_str().unwrap(),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let i = &input["identity"];
            let identity = Identity {
                id: i["id"].as_str().unwrap(),
                observer_kind: i["observer_kind"].as_str().unwrap(),
                chain: i["chain"].as_str().unwrap(),
                network: i["network"].as_str().unwrap(),
                logical_wallet_id: i["logical_wallet_id"].as_str().unwrap(),
                source_wallet_id: i["source_wallet_id"].as_str().unwrap(),
                source_key: i["source_key"].as_str().unwrap(),
            };
            state::load_state(&conn,&identity,input["force_full"] == true)
                .map(|v| json!({"prior":v.prior,"coverage":v.coverage,"expected_epoch":v.expected_epoch,"autocommit":conn.is_autocommit()}))
        }
        "current" => {
            let conn = Connection::open_with_flags(
                input["database"].as_str().unwrap(),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            state::current_observed_txids(
                &conn,
                input["wallet_id"].as_str().unwrap(),
                input["observer_id"].as_str().unwrap(),
            )
            .map(|v| json!(v))
        }
        _ => panic!("unknown fixture operation"),
    };
    println!(
        "{}",
        match result {
            Ok(result) => json!({"result":result}),
            Err(error) => json!({"error":{"code":error.code,"message":error.message}}),
        }
    );
}
