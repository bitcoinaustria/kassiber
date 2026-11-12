//! Gated native LWK driver; no product registration or database access.
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_wallets::{
    descriptors,
    lwk::{self, Execution, ExecutionConsent, Refresh},
    scan::{Cancellation, Scope},
};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    io::Read,
    time::{Duration, Instant},
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn error(e: kassiber_envelope::AppError) -> Value {
    let details = e.details.as_ref().map(|d| {
        serde_json::from_str::<Value>(&kassiber_pyjson::dumps(d, &Default::default()).unwrap())
            .unwrap()
    });
    json!({"error":{"code":e.code,"message":e.message,"hint":e.hint.as_deref(),"details":details,"retryable":e.retryable}})
}
fn values(input: &Value) -> BTreeMap<String, Vec<u8>> {
    input
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), unhex(v.as_str().unwrap())))
        .collect()
}

fn main() {
    let mut raw = String::new();
    std::io::stdin()
        .take(64 * 1024 * 1024)
        .read_to_string(&mut raw)
        .unwrap();
    let i: Value = serde_json::from_str(&raw).unwrap();
    let plan = match descriptors::load(&i["config"]) {
        Ok(Some(plan)) => Some(plan),
        Ok(None) => None,
        Err(e) => {
            println!("{}", error(e.into_app()));
            return;
        }
    };
    let output = match i["op"].as_str().unwrap_or("run") {
        "descriptor" => {
            let plan = plan.expect("descriptor plan");
            json!({"result":{
                "text": lwk::descriptor::descriptor_text(&plan).map_err(|e| e.code).ok(),
                "wollet": lwk::descriptor::wollet_descriptor_string(&plan).map_err(|e| e.code).ok(),
                "reasons": i["backends"].as_array().into_iter().flatten().map(|b| match lwk::descriptor::compatibility_reason(b, &plan) {
                    Ok(reason) => json!(reason),
                    Err(e) => json!({"error": e.code, "message": e.message}),
                }).collect::<Vec<_>>(),
            }})
        }
        "canonical" => match lwk::canonical_values(&values(&i["values"])) {
            Ok(v) => json!({"result": v}),
            Err(e) => error(e),
        },
        _ => {
            let scope = Scope {
                database_instance: "fixture".into(),
                profile_id: "p".into(),
                wallet_id: "w".into(),
                environment_id: "regtest".into(),
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
            let cancel = Cancellation::default();
            if i["cancelled"] == true {
                cancel.cancel();
            }
            let result = Execution::new(
                scope.clone(),
                &i["backend"],
                &plan.expect("descriptor plan"),
                i["network"].as_str().unwrap(),
                i["policy_asset"].as_str().unwrap(),
                i["branch_keys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect(),
                Refresh {
                    force_full: i["force_full"] == true,
                    prior: i.get("prior").filter(|v| !v.is_null()).cloned(),
                    prior_coverage: i["prior_coverage"].as_array().cloned().unwrap_or_default(),
                    checkpoint: i.get("checkpoint").cloned().unwrap_or(json!({})),
                    values: values(&i["values"]),
                },
            )
            .and_then(|a| {
                let c = (i["consent"] == true).then(|| ExecutionConsent::explicit_trigger(&a));
                a.run(
                    &gate,
                    &scope,
                    c.as_ref(),
                    Instant::now() + Duration::from_secs(20),
                    &cancel,
                )
            });
            match result {
                Ok(p) => {
                    let stored: Map<String, Value> = p
                        .values
                        .iter()
                        .map(|(k, v)| (k.clone(), json!(hex(v))))
                        .collect();
                    json!({"result":{"update":p.update,"values":stored,"canonical_values":lwk::canonical_values(&p.values).unwrap()}})
                }
                Err(e) => error(e),
            }
        }
    };
    println!("{output}");
}
