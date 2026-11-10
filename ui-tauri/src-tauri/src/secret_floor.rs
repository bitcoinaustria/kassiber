//! The backend's secret floor, compiled from the same source as the
//! `kassiber-envelope` crate (`crates/envelope/src/redaction/`). Those files
//! use only the standard library, so the shell shares the exact rules without
//! depending on the backend workspace.

#[path = "../../../crates/envelope/src/redaction/scanner.rs"]
#[allow(dead_code)]
pub mod scanner;

#[path = "../../../crates/envelope/src/redaction/floor.rs"]
#[allow(dead_code)]
pub mod floor;
