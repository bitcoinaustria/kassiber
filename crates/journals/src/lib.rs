//! Native journal composition, rendering and persistence (R4-11/R4-12/R4-13).
//! The scoped service binds local transactions and lifecycle sources; no stage
//! performs network I/O. Raw custody operands retain the transitional JSON
//! boundary; the engine result and rendered entries have native types.
pub mod boundary;
pub mod engine;
mod integer;
pub mod loader;
mod model;
pub mod native;
pub mod persistence;
#[cfg(any(test, feature = "test-harness"))]
pub mod pipeline;
pub mod prebuild;
pub mod process;
mod quarantine;
mod render;
#[cfg(feature = "test-harness")]
mod review_apply;
#[cfg(feature = "test-harness")]
pub mod review_effects;
#[cfg(feature = "test-harness")]
mod review_simulation;
#[cfg(feature = "test-harness")]
pub mod review_workflow;
pub mod scalar;
pub mod service;
pub mod storage;
pub mod timing;
pub mod views;
pub use model::*;
pub use render::{enrich_quarantine_detail, render_ledger, LedgerInputs, LedgerState};
use serde_json::Value;

/// A Python refusal, including the exception family and AppError attributes.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(transparent)]
pub struct Refusal(Box<RefusalDetails>);

#[derive(Clone, Debug, serde::Serialize)]
pub struct RefusalDetails {
    pub family: String,
    pub message: String,
    pub code: Option<String>,
    pub hint: Option<String>,
    pub details: Value,
    pub retryable: Option<bool>,
    pub error_class: Option<String>,
    pub invariant: Option<String>,
    pub cause: Option<Box<Refusal>>,
    /// Driver metadata used by daemon retry/error projection, outside oracle JSON.
    #[serde(skip)]
    pub sqlite_code: Option<i32>,
    /// `details` in Python's insertion order for the transport envelope;
    /// `details` itself is a sorted JSON map.
    #[serde(skip)]
    pub ordered_details: Option<kassiber_pyjson::Value>,
}
impl Refusal {
    pub(crate) fn new(family: &str, message: impl Into<String>) -> Self {
        Self(Box::new(RefusalDetails {
            family: family.into(),
            message: message.into(),
            code: None,
            hint: None,
            details: Value::Null,
            retryable: (family == "AppError").then_some(false),
            error_class: None,
            invariant: None,
            cause: None,
            sqlite_code: None,
            ordered_details: None,
        }))
    }
}
impl std::ops::Deref for Refusal {
    type Target = RefusalDetails;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Refusal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Refusal {}
impl From<String> for Refusal {
    fn from(message: String) -> Self {
        Self::new("RuntimeError", message)
    }
}
impl From<kassiber_envelope::AppError> for Refusal {
    fn from(error: kassiber_envelope::AppError) -> Self {
        prebuild::app(error)
    }
}
impl From<kassiber_pydecimal::DecimalError> for Refusal {
    fn from(error: kassiber_pydecimal::DecimalError) -> Self {
        Self::new(error.name(), error.to_string())
    }
}

pub(crate) fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
#[inline]
pub(crate) fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    match value {
        Value::Object(fields) => fields.get(key).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}
pub(crate) fn string(value: &Value) -> String {
    match value {
        Value::Array(_) | Value::Object(_) => scalar::repr(
            &kassiber_custody::codec::from_json(value).expect("JSON scalar conversion"),
        ),
        _ => kassiber_custody::codec::pystr(value),
    }
}
pub(crate) fn truth(value: &Value) -> bool {
    kassiber_custody::codec::truth(value)
}
pub(crate) fn fallback<'a>(a: &'a Value, b: &'a Value) -> &'a Value {
    if truth(a) {
        a
    } else {
        b
    }
}
pub(crate) fn number(text: &str) -> Result<Value, Refusal> {
    serde_json::from_str(text).map_err(|e| Refusal::new("ValueError", e.to_string()))
}
pub(crate) fn pyjson(value: &Value, compact: bool) -> Result<String, Refusal> {
    let options = kassiber_pyjson::Options {
        sort_keys: true,
        separators: compact.then_some((",".into(), ":".into())),
        ..Default::default()
    };
    kassiber_pyjson::dumps(&kassiber_custody::codec::from_json(value)?, &options)
        .map_err(|e| Refusal::new("ValueError", e.to_string()))
}

mod compute;
pub use compute::{compute, compute_loaded, compute_snapshot, Computed, Fusion};
pub use kassiber_custody::Stages;
