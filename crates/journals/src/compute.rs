//! The `journals process` pipeline of plan 21, R4: custody, fusion and the
//! tax engine in one process (docs/reference/r4-journal-pipeline.md).
//!
//! No command calls it yet. Until fusion is typed (R4-8), it reads the
//! finalized projection, the profile and the wallets in exactly the JSON
//! model `kassiber_custody::custody_with_fusion` reads from the request and
//! response texts, so both build the same ledger request.
use kassiber_custody::{
    codec, ledger_request,
    snapshot::{BookSnapshot, Snapshot},
    timing,
};
use serde_json::Value;

pub use kassiber_custody::{Refusal, Stages};

/// What fusion did with the finalized projection.
#[derive(Debug, PartialEq)]
pub enum Fusion {
    /// A decision-only build: no finalized projection, so no fusion.
    Skipped,
    /// An empty projection; the engine is not called.
    Empty,
    /// The builder's refusal document.
    Refused(Value),
    /// The request's wire text and the engine's response text.
    Ledger { request: String, response: String },
}

/// One computed journal build.
pub struct Computed {
    /// The stage snapshot whose rows the stages index.
    pub snapshot: Snapshot,
    pub stages: Stages,
    pub fusion: Fusion,
}

/// Custody and fusion for one v1 snapshot. `finalized` false is the
/// decision-only build, which skips finalization and fusion.
pub fn compute(request: &str, finalized: bool) -> Result<Computed, Refusal> {
    let snapshot = kassiber_custody::parse(request)?;
    compute_snapshot(snapshot, finalized)
}

pub fn compute_snapshot(snapshot: BookSnapshot, finalized: bool) -> Result<Computed, Refusal> {
    let (rows, stages) = kassiber_custody::run_with_rows(&snapshot, finalized)?;
    finish(rows, stages)
}

pub fn compute_loaded(snapshot: Snapshot, finalized: bool) -> Result<Computed, Refusal> {
    let stages = kassiber_custody::run_loaded(&snapshot, finalized)?;
    finish(snapshot, stages)
}

fn finish(snapshot: Snapshot, stages: Stages) -> Result<Computed, Refusal> {
    let fusion = match &stages.finalized_projection {
        Some(projection) => fuse(&snapshot, projection)?,
        None => Fusion::Skipped,
    };
    Ok(Computed {
        snapshot,
        stages,
        fusion,
    })
}

fn fuse(snapshot: &Snapshot, projection: &kassiber_pyjson::Value) -> Result<Fusion, Refusal> {
    let span = timing::span("fusion.input");
    let projection =
        kassiber_custody::journal_support::exact_operand(projection).map_err(|message| {
            Refusal {
                stage: "fusion",
                code: "ValueError".into(),
                message,
                hint: None,
                details: Value::Null,
            }
        })?;
    let python = |record: &kassiber_custody::snapshot::Record| {
        codec::from_json(&Value::Object(record.clone().into_map()))
            .map(|value| codec::to_json(&value))
            .map_err(|message| Refusal {
                stage: "fusion",
                code: "ValueError".into(),
                message,
                hint: None,
                details: Value::Null,
            })
    };
    let profile = python(&snapshot.profile)?;
    let wallets = snapshot
        .wallets
        .iter()
        .map(|wallet| {
            let wallet = python(wallet)?;
            Ok((wallet["id"].clone(), wallet))
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    drop(span);
    let span = timing::span("fusion.request");
    let outcome = ledger_request::build_request(&profile, &projection, &wallets, "ledger");
    drop(span);
    Ok(match outcome {
        Ok(ledger_request::Outcome::Empty) => Fusion::Empty,
        Err(error) => Fusion::Refused(error.document()),
        Ok(ledger_request::Outcome::Request(request)) => {
            let span = timing::span("fusion.text");
            let request = ledger_request::request_json(&request);
            drop(span);
            let _span = timing::span("engine");
            let response = kassiber_tax_core::ledger(&request);
            Fusion::Ledger { request, response }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::dict;
    use kassiber_pyjson::Value as Py;

    #[test]
    fn fusion_preserves_unused_bytes_and_refuses_lossy_scalars_without_panicking() {
        let snapshot = Snapshot::default();
        let projection = |value| dict([("rows", Py::List(vec![])), ("diagnostic", value)]);
        assert_eq!(
            fuse(&snapshot, &projection(Py::Bytes(vec![255, 0]))).unwrap(),
            Fusion::Empty
        );
        for raw in ["\"\\ud800\"", "NaN", "Infinity"] {
            let error =
                fuse(&snapshot, &projection(kassiber_pytext::loads(raw).unwrap())).unwrap_err();
            assert_eq!(error.code, "ValueError");
            assert_eq!(
                error.message,
                "journal operands cannot represent custody facts losslessly"
            );
        }
    }
}
