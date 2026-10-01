//! Python binding for Kassiber's tax engine.
//!
//! The only entry points take and return JSON text, so the boundary stays
//! narrow and versioned. A Rust panic is converted into `RuntimeError`
//! instead of crossing into Python as a partial result.

use pyo3::prelude::*;

/// Version of the linked engine.
#[pyfunction]
fn engine_version() -> &'static str {
    kassiber_tax_core::ENGINE_VERSION
}

#[pymodule]
fn kassiber_tax(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(engine_version, m)?)?;
    Ok(())
}
