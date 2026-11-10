//! Desktop projections: one daemon session per comparison on a book seeded
//! with the desktop fixture, as `tests/rust_desktop_parity.py` ran them.
//!
//! The seeds are the SQL the Python modules built, frozen under
//! `tests/black_box/sql/`. A read session must leave every table unchanged;
//! a mutating one records the rows it changed.

use serde_json::{json, Value};

use crate::support::{Sandbox, Transcript};

pub const DESKTOP: &str = include_str!("../../../transactions/tests/data/desktop.sql");
pub const ACTIVITY: &str = include_str!("sql/activity.sql");
pub const DECADE: &str = include_str!("sql/decade.sql");
pub const CURRENT: &str = include_str!("sql/current.sql");
pub const CAPITAL_BOOK: &str = include_str!("../../../reports/tests/data/book.sql");
pub const CAPITAL_BASE: &str = include_str!("sql/capital_gains_base.sql");
pub const SYNC_BOOK: &str = include_str!("sql/sync_book.sql");
pub const LOANS: &str = include_str!("sql/loans.sql");
pub const WALLETS: &str = include_str!("sql/wallets.sql");

/// `tests/test_rust_report_blockers.py::BARRIERS`.
pub fn barriers() -> String {
    format!("{ACTIVITY}{}", include_str!("sql/barriers.sql"))
}

/// `tests/test_rust_transaction_graph.py::GRAPH`.
pub fn graph() -> String {
    format!("{ACTIVITY}{}", include_str!("sql/graph.sql"))
}

/// `tests/test_rust_evidence_summary.py::EVIDENCE`.
pub fn evidence() -> String {
    format!("{}{}", barriers(), include_str!("sql/evidence.sql"))
}

/// `tests/test_rust_audit_package.py::ATTACHMENTS`.
pub fn attachments() -> String {
    format!("{}{}", evidence(), include_str!("sql/attachments.sql"))
}

/// `tests/test_rust_wallet_completeness.py::VERIFY`.
pub fn verify() -> String {
    format!("{}{}", graph(), include_str!("sql/verify.sql"))
}

/// `tests/test_rust_report_capital_gains.py::BOOK`.
pub fn capital_book() -> String {
    format!("{CAPITAL_BOOK}{CAPITAL_BASE}")
}

/// A desktop book: `seed` (the desktop fixture unless given), with the
/// daemon's environment.
pub fn book_with(seed: &str) -> Sandbox {
    let mut sandbox = Sandbox::book(seed);
    sandbox.set("KASSIBER_NO_EGRESS", "1");
    sandbox
}

pub fn book() -> Sandbox {
    book_with(DESKTOP)
}

/// `None` leaves `args` out of the request, as Python's `...` did.
pub fn requests(kind: &str, args: &[Option<Value>]) -> Vec<Value> {
    args.iter()
        .enumerate()
        .map(|(index, arg)| {
            let mut request = json!({"request_id": index.to_string(), "kind": kind});
            if let Some(arg) = arg {
                request["args"] = arg.clone();
            }
            request
        })
        .collect()
}

/// A read session: the responses, with every table unchanged.
pub fn compare(
    t: &mut Transcript,
    sandbox: &Sandbox,
    kind: &str,
    args: &[Option<Value>],
) -> Vec<Value> {
    let before = sandbox.dump();
    let responses = t.daemon(sandbox, &requests(kind, args));
    assert_eq!(sandbox.dump(), before, "a {kind} read changed the book");
    responses
}

/// A session that may write: the responses and the rows it changed.
pub fn mutating(
    t: &mut Transcript,
    sandbox: &Sandbox,
    kind: &str,
    args: &[Option<Value>],
) -> Vec<Value> {
    let before = sandbox.dump();
    let responses = t.daemon(sandbox, &requests(kind, args));
    t.changes(sandbox, &before, &format!("after the {kind} session"));
    responses
}

/// Shorthand for an argument list where every entry is present.
pub fn some(args: Vec<Value>) -> Vec<Option<Value>> {
    args.into_iter().map(Some).collect()
}

pub const NO_CONTEXT: &str =
    "DELETE FROM settings WHERE key IN ('context_workspace','context_profile');";
