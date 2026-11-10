//! Guarded protocol acquisition oracle. No product command or database access.
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_wallets::{
    acquisition::{Acquisition, AcquisitionConsent},
    scan::{Cancellation, Scope},
};
use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};

fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let input: Value = serde_json::from_str(&raw).unwrap();
    let scope = Scope {
        database_instance: "synthetic".into(),
        profile_id: "p".into(),
        wallet_id: "w".into(),
        environment_id: "fixture".into(),
        source_revision: "1".into(),
    };
    let cancel = Cancellation::default();
    let mut progress = vec![];
    let offline = input["offline"] == true;
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
    let action = if input["cached"] == true {
        Acquisition::new_cached(
            scope.clone(),
            &input["backend"],
            input["targets"].as_array().unwrap(),
            input["version"].as_str().unwrap_or("fixture"),
            &input["checkpoint"],
            &input["history_cache"],
            input["network"].as_str().unwrap_or("main"),
        )
    } else {
        Acquisition::new(
            scope.clone(),
            &input["backend"],
            input["targets"].as_array().unwrap(),
            input["version"].as_str().unwrap_or("fixture"),
        )
    };
    let result = action.and_then(|action| {
        let grant =
            (input["consent"] == true).then(|| AcquisitionConsent::explicit_trigger(&action));
        action.run(
            &gate,
            &scope,
            grant.as_ref(),
            Instant::now() + Duration::from_secs(5),
            &cancel,
            |event| {
                progress.push(event);
                if input["cancel_after_progress"] == true {
                    cancel.cancel();
                }
            },
        )
    });
    println!(
        "{}",
        match result {
            Ok(result) => json!({"result":result}),
            Err(error) =>
                json!({"error":{"code":error.code,"message":error.message},"progress":progress}),
        }
    );
}
