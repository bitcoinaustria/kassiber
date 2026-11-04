//! Python binding for Kassiber's tax engine.
//!
//! The only entry points take and return JSON text, so the boundary stays
//! narrow and versioned (`docs/reference/tax-engine.md`). The engine runs
//! with the GIL released. A Rust panic is converted into `RuntimeError`
//! instead of crossing into Python as a partial result.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

/// Version of the linked engine.
#[pyfunction]
fn engine_version() -> &'static str {
    kassiber_tax_core::ENGINE_VERSION
}

fn panic_text(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "unknown panic payload"
    }
}

/// Runs `call` without the GIL and turns a panic into `RuntimeError`.
fn run(py: Python<'_>, request: &str, call: fn(&str) -> String) -> PyResult<String> {
    py.allow_threads(|| catch_unwind(AssertUnwindSafe(|| call(request))))
        .map_err(|payload| {
            PyRuntimeError::new_err(format!(
                "Internal error: the tax engine panicked: {}",
                panic_text(payload.as_ref())
            ))
        })
}

/// Validates one entry as RP2's constructor would; returns the response
/// document.
#[pyfunction]
fn check_entry(py: Python<'_>, request: &str) -> PyResult<String> {
    run(py, request, kassiber_tax_core::check_entry)
}

/// Runs one compute request; returns the response document.
#[pyfunction]
fn compute(py: Python<'_>, request: &str) -> PyResult<String> {
    run(py, request, kassiber_tax_core::compute)
}

#[pymodule]
fn kassiber_tax(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(engine_version, m)?)?;
    m.add_function(wrap_pyfunction!(check_entry, m)?)?;
    m.add_function(wrap_pyfunction!(compute, m)?)?;
    Ok(())
}
