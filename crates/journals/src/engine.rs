use crate::{boundary, Refusal};
use kassiber_tax_core::ledger::{request::LedgerRequest, response::LedgerResponse};
use serde::Deserialize;
use serde_json::Value;

pub fn ledger(request: &Value) -> Result<LedgerResponse, Refusal> {
    let request_span = crate::timing::span("request_conversion");
    let typed = LedgerRequest::deserialize(request);
    drop(request_span);
    match typed {
        Ok(request) => {
            let _span = crate::timing::span("engine_compute");
            // On the calling thread: the engine's workers cost process CPU
            // time without shortening anything that counts.
            kassiber_tax_core::sequential(|| kassiber_tax_core::ledger_typed(&request))
                .map_err(boundary::staged_error)
        }
        Err(_) => {
            let _span = crate::timing::span("engine_text_refusal");
            let text = kassiber_custody::ledger_request::request_json(request);
            let response = kassiber_tax_core::ledger(&text);
            let response = kassiber_custody::codec::loads(&response)?;
            boundary::ledger_response(&kassiber_custody::codec::to_json(&response))
        }
    }
}
