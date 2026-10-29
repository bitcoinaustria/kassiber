//! Gated native BDK driver; no product registration or database access.
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_wallets::{
    bdk::{Execution, ExecutionConsent},
    scan::{Cancellation, Scope},
};
use serde_json::{json, Value};
use std::{
    io::Read,
    time::{Duration, Instant},
};
fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(16 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let i: Value = serde_json::from_str(&raw).unwrap();
    let scope = Scope {
        database_instance: "fixture".into(),
        profile_id: "p".into(),
        wallet_id: "w".into(),
        environment_id: "main".into(),
        source_revision: "1".into(),
    };
    let offline = i["offline"] == true;
    let gate = Gate::new(
        move || {
            if offline {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    );
    let result = Execution::new(
        scope.clone(),
        &i["backend"],
        i["descriptor"].as_str().unwrap(),
        i["change_descriptor"].as_str(),
        i["network"].as_str().unwrap_or("main"),
        i["gap"].as_u64().unwrap_or(3) as u32,
        i.get("prior").filter(|v| !v.is_null()),
    )
    .and_then(|a| {
        let c = (i["consent"] == true).then(|| ExecutionConsent::explicit_trigger(&a));
        a.run(
            &gate,
            &scope,
            c.as_ref(),
            Instant::now() + Duration::from_secs(10),
            &Cancellation::default(),
            i["time"].as_u64().unwrap_or(1700000000),
        )
    });
    println!(
        "{}",
        match result {
            Ok(p) =>
                json!({"result":{"bdk_changeset":p.changeset(),"canonical_txids":p.wallet().transactions().map(|t|t.tx_node.tx.compute_txid().to_string()).collect::<Vec<_>>()}}),
            Err(e) => json!({"error":{"code":e.code,"message":e.message}}),
        }
    );
}
