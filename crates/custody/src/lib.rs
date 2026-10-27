//! Pure custody computation. No storage, network, clock, logging or Python calls.
pub mod codec;
pub mod evidence;
mod gates;
mod graph;
mod python_text;
pub mod snapshot;

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
    let parsed = serde_json::from_str::<snapshot::Snapshot>(request)
        .map_err(|e| e.to_string())
        .and_then(|snapshot| {
            snapshot.validate()?;
            Ok(snapshot)
        });
    match parsed {
        Ok(snapshot) => {
            if let Err((code, message)) = gates::require_accounting(&snapshot) {
                return serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"code":code,"message":message}}).to_string();
            }
            match evidence::compute(&snapshot) {
            Ok(evidence) => serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"stages":{"evidence":evidence},"skipped_stages":STAGES.into_iter().filter(|s|*s!="evidence").collect::<Vec<_>> ()}).to_string(),
            Err(message) => serde_json::json!({"schema_version":1,"engine_version":ENGINE_VERSION,"error":{"code":"custody_evidence_invalid","message":message}}).to_string(),
        }
        },
        Err(message) => serde_json::json!({"schema_version": 1, "engine_version": ENGINE_VERSION, "error": {"code": "custody_snapshot_invalid", "message": message}}).to_string(),
    }
}
