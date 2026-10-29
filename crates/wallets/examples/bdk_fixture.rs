//! Gated native BDK driver; no product registration or database access.
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_wallets::{
    bdk::{Execution, ExecutionConsent, Refresh},
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
    let result = Execution::new_refresh(
        scope.clone(),
        &i["backend"],
        i["descriptor"].as_str().unwrap(),
        i["change_descriptor"].as_str(),
        i["network"].as_str().unwrap_or("main"),
        i["gap"].as_u64().unwrap_or(3) as u32,
        i.get("prior").filter(|v| !v.is_null()),
        Refresh {
            force_full: i["force_full"] == true,
            canonical_txids: i["canonical_txids"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect(),
            checkpoint_tip: i["checkpoint_tip"]
                .as_u64()
                .map(|h| u32::try_from(h).unwrap()),
        },
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
    })
    .and_then(|p| {
        let projection = p.records(
            i["backend"]["name"].as_str().unwrap_or("fixture"),
            i["fingerprint_version"].as_str().unwrap_or("1:fixture"),
            i.get("prior_state"),
            i["current_txids"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect()),
        )?;
        Ok((p, projection))
    });
    println!(
        "{}",
        match result {
            Ok((p, records)) =>
                json!({"result":{"bdk_changeset":p.changeset(),"canonical_txids":p.canonical_txids(),"retracted_external_ids":p.retracted_external_ids(),"rebuilt":p.rebuilt(),"records":records.records,"record_fingerprints":records.fingerprints,"reused_records":records.reused_records}}),
            Err(e) => json!({"error":{"code":e.code,"message":e.message}}),
        }
    );
}
