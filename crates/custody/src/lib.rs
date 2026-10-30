//! Pure custody computation. No storage, network, clock, logging or Python calls.
pub mod codec;
pub mod components;
pub mod confirmation;
pub mod evidence;
pub mod finalized_projection;
pub mod gaps;
mod gates;
mod graph;
pub mod interpretation;
pub mod ledger_request;
mod liquid;
mod par;
mod python_text;
pub mod quantity;
pub mod snapshot;
pub mod timing;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STAGES: [&str; 7] = [
    "components",
    "evidence",
    "interpretation",
    "gaps",
    "quantity",
    "finalized_projection",
    "fusion",
];

/// Versioned JSON operation; missing stages are explicit and never fabricated.
pub fn custody(request: &str) -> String {
    custody_with_finalized(request, true)
}

/// Decision-only callers explicitly skip finalization, including its refusals.
/// The control is outside snapshot v1; the snapshot remains raw facts only.
pub fn custody_with_finalized(request: &str, finalized: bool) -> String {
    struct Memos;
    impl Drop for Memos {
        fn drop(&mut self) {
            codec::clear_memos();
        }
    }
    let _memos = Memos;
    let parse = timing::span("parse");
    let parsed = serde_json::from_str::<snapshot::Snapshot>(request)
        .map_err(|e| e.to_string())
        .and_then(|snapshot| {
            snapshot.validate()?;
            Ok(snapshot)
        });
    drop(parse);
    match parsed {
        Ok(snapshot) => {
            let admission = timing::span("admission");
            let gate = timing::span("admission.accounting");
            if let Err((code, message)) = gates::require_accounting(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":message}}).to_string();
            }
            drop(gate);
            let gate = timing::span("admission.confirmation");
            // Python captures confirmation provenance before rates and channel
            // admission, retaining the original rows rather than enriched rows.
            let projection_rows = match finalized_projection::runtime::confirmation_rows(&snapshot) {
                Ok(value) => value,
                Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":code}}).to_string(),
            };
            drop(gate);
            let gate = timing::span("admission.rates_channels");
            if let Err((code, message)) = gates::require_rates(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":message}}).to_string();
            }
            if let Err(code) = interpretation::admit_channels(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":code}}).to_string();
            }
            drop(gate);
            drop(admission);
            let span = timing::span("components+migration");
            let components = match components::compute(&snapshot) {
                Ok(value) => value,
                Err(error) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":error.code,"message":error.message}}).to_string(),
            };
            let migration = match interpretation::migration_quarantines(&snapshot) {
                Ok(value) => value,
                Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":code}}).to_string(),
            };
            drop(span);
            let span = timing::span("evidence");
            let evidence = match evidence::compute(&snapshot) {
                Ok(value) => value,
                Err(message) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"evidence","code":"custody_evidence_invalid","message":message}}).to_string(),
            };
            drop(span);
            let span = timing::span("interpretation");
            let interpretation = match interpretation::stage(&snapshot, &components, &evidence, migration) {
                Ok(value) => value,
                Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"interpretation","code":code,"message":code}}).to_string(),
            };
            drop(span);
            let span = timing::span("gaps");
            let gaps = match gaps::stage(&snapshot, &components, &evidence, &interpretation) {
                Ok(value) => value,
                Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"gaps","code":code,"message":code}}).to_string(),
            };
            drop(span);
            let span = timing::span("quantity");
            let quantity = match quantity::runtime::stage(&snapshot, &components, &evidence, &interpretation, &gaps) {
                Ok(value) => value,
                Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"quantity","code":code,"message":code}}).to_string(),
            };
            drop(span);
            let span = timing::span("finalized_projection");
            let projection = if finalized {
                match finalized_projection::runtime::stage_with_rows(&snapshot, projection_rows, &quantity, &interpretation) {
                    Ok(value) => Some(value),
                    Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"finalized_projection","code":code,"message":code}}).to_string(),
                }
            } else { None };
            drop(span);
            let _span = timing::span("response");
            if let Some(text) = response_text(
                snapshot.transactions.len() >= 256,
                &components,
                &evidence,
                &gaps,
                &interpretation,
                &quantity,
                projection.as_ref(),
            ) {
                // Freeing a large book's values takes a noticeable share of the
                // operation and changes no output; release them off the
                // caller's path when a thread is available.
                if snapshot.transactions.len() >= 256 {
                    let values = (snapshot, components, evidence, interpretation, gaps, quantity, projection);
                    let _ = std::thread::Builder::new().spawn(move || drop(values));
                }
                return text;
            }
            let response = serde_json::json!({
                "schema_version":1,
                "engine_version":ENGINE_VERSION,
                "stages":{"components":components,"evidence":evidence,"gaps":gaps},
                "skipped_stages":if finalized { vec!["fusion"] } else { vec!["finalized_projection", "fusion"] }
            });
            let mut response = codec::from_json(&response).expect("computed custody response");
            let mut stages = graph::stored::get(&response, "stages").clone();
            graph::stored::put(&mut stages, "interpretation", interpretation);
            graph::stored::put(&mut stages, "quantity", quantity);
            if let Some(projection) = projection {
                graph::stored::put(&mut stages, "finalized_projection", projection);
            }
            graph::stored::put(&mut response, "stages", stages);
            kassiber_pyjson::dumps(&response, &kassiber_pyjson::Options::new())
                .expect("computed JSON values")
        },
        Err(message) => serde_json::json!({"schema_version": 1, "engine_version": ENGINE_VERSION, "error": {"stage":"snapshot","code": "custody_snapshot_invalid", "message": message}}).to_string(),
    }
}

/// The successful response exactly as `kassiber_pyjson::dumps` writes the
/// composed document (serde stages convert through `codec::from_json`, so
/// object keys are sorted), without materializing it; `None` defers to that
/// path for any value it would treat differently.
fn response_text(
    concurrent: bool,
    components: &serde_json::Value,
    evidence: &serde_json::Value,
    gaps: &serde_json::Value,
    interpretation: &kassiber_pyjson::Value,
    quantity: &kassiber_pyjson::Value,
    projection: Option<&kassiber_pyjson::Value>,
) -> Option<String> {
    enum Stage<'a> {
        Json(&'a serde_json::Value),
        Py(&'a kassiber_pyjson::Value),
    }
    // Stages sit two containers deep: the response, then its stage map.
    fn encode(stage: &Stage) -> Option<String> {
        let options = kassiber_pyjson::Options::new();
        let mut out = String::new();
        match stage {
            Stage::Json(value) => codec::dumps_json_into(value, &options, 2, &mut out)?,
            Stage::Py(value) => kassiber_pyjson::dumps_into(value, &options, 2, &mut out).ok()?,
        }
        Some(out)
    }
    let mut stages = vec![
        ("components", Stage::Json(components)),
        ("evidence", Stage::Json(evidence)),
        ("gaps", Stage::Json(gaps)),
    ];
    stages.push(("interpretation", Stage::Py(interpretation)));
    stages.push(("quantity", Stage::Py(quantity)));
    if let Some(projection) = projection {
        stages.push(("finalized_projection", Stage::Py(projection)));
    }
    // Stage texts are independent; encode a large book's concurrently when
    // threads are available, and in order otherwise.
    let texts: Vec<Option<String>> = if !concurrent {
        stages.iter().map(|(_, stage)| encode(stage)).collect()
    } else {
        std::thread::scope(|scope| {
            let workers: Vec<_> = stages
                .iter()
                .map(|(_, stage)| {
                    std::thread::Builder::new()
                        .spawn_scoped(scope, move || encode(stage))
                        .ok()
                })
                .collect();
            stages
                .iter()
                .zip(workers)
                .map(|((_, stage), worker)| match worker {
                    Some(worker) => worker
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                    None => encode(stage),
                })
                .collect()
        })
    };
    let options = kassiber_pyjson::Options::new();
    let mut out =
        String::with_capacity(texts.iter().flatten().map(String::len).sum::<usize>() + 256);
    out.push_str("{\"engine_version\": ");
    kassiber_pyjson::dumps_str_into(ENGINE_VERSION, &options, &mut out);
    out.push_str(", \"schema_version\": 1, \"skipped_stages\": ");
    out.push_str(if projection.is_some() {
        "[\"fusion\"]"
    } else {
        "[\"finalized_projection\", \"fusion\"]"
    });
    out.push_str(", \"stages\": {");
    for (index, ((name, _), text)) in stages.iter().zip(texts).enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        kassiber_pyjson::dumps_str_into(name, &options, &mut out);
        out.push_str(": ");
        out.push_str(&text?);
    }
    out.push_str("}}");
    Some(out)
}

/// Comparison-only fusion. The caller supplies the existing ledger engine;
/// custody itself retains no tax-engine dependency or persistence authority.
pub fn custody_with_fusion(request: &str, finalized: bool, ledger: fn(&str) -> String) -> String {
    let response = custody_with_finalized(request, finalized);
    let mut original = codec::loads(&response).expect("custody response");
    let response = codec::to_json(&original);
    if !finalized || response.get("error").is_some() {
        return codec::dumps(&original, true).expect("response");
    }
    let snapshot = codec::to_json(&codec::loads(request).expect("admitted snapshot"));
    let wallets = snapshot["wallets"]
        .as_array()
        .expect("wallets")
        .iter()
        .map(|wallet| (wallet["id"].clone(), wallet.clone()))
        .collect::<Vec<_>>();
    let fusion = match ledger_request::build_request(
        &snapshot["profile"],
        &response["stages"]["finalized_projection"],
        &wallets,
        "ledger",
    ) {
        Ok(ledger_request::Outcome::Empty) => serde_json::json!({"outcome":"empty"}),
        Ok(ledger_request::Outcome::Request(request)) => {
            let text = ledger_request::request_json(&request);
            let result: serde_json::Value =
                serde_json::from_str(&ledger(&text)).expect("ledger response");
            serde_json::json!({"outcome":"request","request":request,"request_text":text,"response":result})
        }
        Err(error) => serde_json::json!({"outcome":"refusal","error":error.document()}),
    };
    // The builder reads named JSON fields, but existing stage diagnostics may
    // contain Python surrogate code points. Preserve their lossless values.
    let mut stages = graph::stored::get(&original, "stages").clone();
    graph::stored::put(
        &mut stages,
        "fusion",
        codec::from_json(&fusion).expect("fusion"),
    );
    graph::stored::put(&mut original, "stages", stages);
    graph::stored::put(
        &mut original,
        "skipped_stages",
        kassiber_pyjson::Value::List(vec![]),
    );
    codec::dumps(&original, true).expect("fusion response")
}
