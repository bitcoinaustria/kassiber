//! Python binding for Kassiber's tax engine.
//!
//! Entry points take and return JSON text, so boundaries stay narrow and
//! versioned. `ledger` is schema 2 (`docs/reference/tax-engine.md`); `custody`
//! is the raw-fact schema 1 (`docs/reference/custody-engine.md`). The core's
//! schema-1 operations (`check_entry`,
//! `compute`) stay its lot-engine fixture API and are not exposed here. The
//! engine runs with the GIL released. A Rust panic is converted into
//! `RuntimeError` instead of crossing into Python as a partial result.

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

/// Runs one schema-2 request (`ledger`, `prepare`, or `manifest`); returns
/// the response document.
#[pyfunction]
fn ledger(py: Python<'_>, request: &str) -> PyResult<String> {
    run(py, request, kassiber_tax_core::ledger)
}

/// Runs the versioned raw-fact custody operation independently of `ledger`.
/// `loaded` reads the snapshot in the database loader's record encoding
/// (SQLite BLOBs as the codec's byte tag) and runs the loaded path, for books
/// the strict v1 adapter cannot carry.
#[pyfunction(signature = (request, *, finalized=true, loaded=false))]
fn custody(py: Python<'_>, request: &str, finalized: bool, loaded: bool) -> PyResult<String> {
    fn decisions(request: &str) -> String {
        kassiber_custody::custody_with_fusion(request, false, kassiber_tax_core::ledger)
    }
    fn fusion(request: &str) -> String {
        kassiber_custody::custody_with_fusion(request, true, kassiber_tax_core::ledger)
    }
    fn loaded_decisions(request: &str) -> String {
        kassiber_custody::custody_loaded_with_fusion(request, false, kassiber_tax_core::ledger)
    }
    fn loaded_fusion(request: &str) -> String {
        kassiber_custody::custody_loaded_with_fusion(request, true, kassiber_tax_core::ledger)
    }
    let call = match (loaded, finalized) {
        (false, false) => decisions,
        (false, true) => fusion,
        (true, false) => loaded_decisions,
        (true, true) => loaded_fusion,
    };
    run(py, request, call)
}

#[pymodule]
fn kassiber_tax(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(engine_version, m)?)?;
    m.add_function(wrap_pyfunction!(ledger, m)?)?;
    m.add_function(wrap_pyfunction!(custody, m)?)?;
    Ok(())
}
