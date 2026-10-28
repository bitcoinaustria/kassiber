//! Guarded synthetic discovery oracle, never registered as a product command.
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_wallets::scan::{Cancellation, CompatibilityDiscovery, DiscoveryConsent, Scope};
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
        source_revision: "v1".into(),
    };
    let cancel = Cancellation::default();
    let mut progress = vec![];
    let result = CompatibilityDiscovery::new(
        scope.clone(),
        &input["wallet"],
        &input["backend"],
        &input["checkpoint"],
        input["version"].as_str().unwrap_or("fixture"),
    )
    .and_then(|action| {
        let grant = (input["consent"] == true).then(|| DiscoveryConsent::explicit_trigger(&action));
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
    let output = match result {
        Ok(result) => json!({"result":result,"progress":progress}),
        Err(error) => {
            json!({"error":{"code":error.code,"message":error.message},"progress":progress})
        }
    };
    println!("{output}");
}
