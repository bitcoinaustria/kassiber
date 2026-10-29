//! Pure custody computation. No storage, network, clock, logging or Python calls.
pub mod codec;
pub mod components;
pub mod evidence;
pub mod finalized_projection;
pub mod gaps;
mod gates;
mod graph;
pub mod interpretation;
mod liquid;
mod python_text;
pub mod quantity;
pub mod snapshot;
pub mod timing;

pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STAGES: [&str; 6] = [
    "components",
    "evidence",
    "interpretation",
    "gaps",
    "quantity",
    "finalized_projection",
];

/// Versioned JSON operation; missing stages are explicit and never fabricated.
pub fn custody(request: &str) -> String {
    custody_with_finalized(request, true)
}

/// Decision-only callers explicitly skip finalization, including its refusals.
/// The control is outside snapshot v1; the snapshot remains raw facts only.
pub fn custody_with_finalized(request: &str, finalized: bool) -> String {
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
            if let Err((code, message)) = gates::require_accounting(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":message}}).to_string();
            }
            // Python captures confirmation provenance before rates and channel
            // admission, retaining the original rows rather than enriched rows.
            let projection_rows = match finalized_projection::runtime::confirmation_rows(&snapshot) {
                Ok(value) => value,
                Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":code}}).to_string(),
            };
            if let Err((code, message)) = gates::require_rates(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":message}}).to_string();
            }
            if let Err(code) = interpretation::admit_channels(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"components","code":code,"message":code}}).to_string();
            }
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
                match finalized_projection::runtime::stage_with_rows(&snapshot, &projection_rows, &quantity, &interpretation) {
                    Ok(value) => Some(value),
                    Err(code) => return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"stage":"finalized_projection","code":code,"message":code}}).to_string(),
                }
            } else { None };
            drop(span);
            let _span = timing::span("response");
            let response = serde_json::json!({
                "schema_version":1,
                "engine_version":ENGINE_VERSION,
                "stages":{"components":components,"evidence":evidence,"gaps":gaps},
                "skipped_stages":if finalized { Vec::<&str>::new() } else { vec!["finalized_projection"] }
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
