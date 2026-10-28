//! Reports family transport. Accounting stays in kassiber-reports.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const COMMANDS: &[&str] = &[
    "reports.summary",
    "reports.balance-sheet",
    "reports.portfolio-summary",
    "reports.capital-gains",
    "reports.filed-snapshots.list",
];
pub fn kind(command: &str) -> Option<&'static str> {
    (command == "reports.filed-snapshots.list").then_some("reports.filed-snapshots.list")
}
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    book.reports(
        command,
        &dict([
            (
                "text",
                Value::Bool(
                    !args.flag("machine")
                        && matches!(args.text("format"), None | Some("table" | "plain")),
                ),
            ),
            ("wallet", args.get("wallet").cloned().unwrap_or(Value::Null)),
            (
                "workspace",
                args.get("workspace").cloned().unwrap_or(Value::Null),
            ),
            (
                "profile",
                args.get("profile").cloned().unwrap_or(Value::Null),
            ),
        ]),
    )
}
