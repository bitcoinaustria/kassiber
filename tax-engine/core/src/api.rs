//! JSON-in, JSON-out entry points (schema version 1).
//!
//! Each function parses one request document and returns one response
//! document. Malformed requests fail with the `RequestError` class; every
//! other failure carries RP2's class and message.
//!
//! `compute` runs one asset (RP2's `compute_tax`). `validate` and
//! `compute_multi` take every asset of a run, in the adapter's order: they
//! are the country hooks `validate_input_data` and `compute_tax_for_assets`,
//! which only Austria implements.

use serde::Serialize;

use crate::engine::{compute_asset, Country};
use crate::entry::Entry;
use crate::error::{EngineError, EngineResult, ErrorClass};
use crate::model::{
    AssetOutput, CheckEntryRequest, CheckEntryResponse, ComputeRequest, ComputeResponse, ErrorBody,
    ErrorResponse, Operation, SCHEMA_VERSION,
};
use crate::swaps;

/// Serializes a response; serializing these types cannot fail, but a
/// failure still becomes an error document rather than a panic.
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        error_json(&EngineError::new(
            ErrorClass::RuntimeError,
            format!("Internal error: response serialization failed: {error}"),
        ))
    })
}

fn error_json(error: &EngineError) -> String {
    let response = ErrorResponse {
        schema_version: SCHEMA_VERSION,
        ok: false,
        error: ErrorBody {
            class: error.class,
            message: error.message.clone(),
        },
    };
    // A plain struct of strings; fall back to a fixed document regardless.
    serde_json::to_string(&response).unwrap_or_else(|_| {
        String::from(
            r#"{"schema_version":1,"ok":false,"error":{"class":"RuntimeError","message":"Internal error: error serialization failed"}}"#,
        )
    })
}

fn respond<T: Serialize>(result: EngineResult<T>) -> String {
    match result {
        Ok(value) => to_json(&value),
        Err(error) => error_json(&error),
    }
}

fn parse<'a, T: serde::Deserialize<'a>>(request: &'a str) -> EngineResult<T> {
    serde_json::from_str(request)
        .map_err(|error| EngineError::request(format!("invalid engine request: {error}")))
}

fn check_version(version: u32) -> EngineResult<()> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(EngineError::request(format!(
            "unsupported schema_version {version}; this engine reads {SCHEMA_VERSION}"
        )))
    }
}

/// Validates one entry as RP2's constructor would and returns its stored
/// and derived values.
pub fn check_entry(request: &str) -> String {
    respond(check_entry_inner(request))
}

fn check_entry_inner(request: &str) -> EngineResult<CheckEntryResponse> {
    let request: CheckEntryRequest = parse(request)?;
    check_version(request.schema_version)?;
    if let Some(operation) = &request.operation {
        if operation != "check_entry" {
            return Err(EngineError::request(format!(
                "check_entry received operation {operation:?}"
            )));
        }
    }
    let entry = Entry::from_input(&request.entry, request.config.as_ref())?;
    Ok(CheckEntryResponse {
        schema_version: SCHEMA_VERSION,
        ok: true,
        entry: entry.derived(),
    })
}

/// Runs a `compute`, `compute_multi`, or `validate` request.
pub fn compute(request: &str) -> String {
    respond(compute_inner(request))
}

fn compute_inner(request: &str) -> EngineResult<ComputeResponse> {
    let request: ComputeRequest = parse(request)?;
    check_version(request.schema_version)?;
    let country = Country::from_spec(&request.country);
    let response = |handled: Option<bool>, assets: Option<Vec<AssetOutput>>| ComputeResponse {
        schema_version: SCHEMA_VERSION,
        ok: true,
        handled,
        assets,
    };
    match request.operation {
        Operation::Compute => {
            let [asset] = request.assets.as_slice() else {
                return Err(EngineError::request(format!(
                    "compute takes exactly one asset, got {}",
                    request.assets.len()
                )));
            };
            let output = compute_asset(asset, country, request.method)?;
            Ok(response(None, Some(vec![output])))
        }
        Operation::Validate => {
            swaps::validate(&request.assets, country)?;
            Ok(response(None, None))
        }
        Operation::ComputeMulti => {
            let outputs = swaps::compute_multi(&request.assets, country, request.method)?;
            Ok(response(Some(outputs.is_some()), outputs))
        }
    }
}
