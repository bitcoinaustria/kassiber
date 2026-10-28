//! Reports family transport. Accounting stays in kassiber-reports.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const COMMANDS: &[&str] = &[
    "reports.summary",
    "reports.balance-sheet",
    "reports.portfolio-summary",
    "reports.capital-gains",
    "reports.filed-snapshots.list",
    "reports.tax-summary",
    "reports.journal-entries",
    "reports.austrian-e1kv",
    "reports.austrian-tax-summary",
];
pub fn kind(command: &str) -> Option<&'static str> {
    (command == "reports.filed-snapshots.list").then_some("reports.filed-snapshots.list")
}
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    if args.text("format") == Some("csv")
        && matches!(
            command,
            "reports.austrian-e1kv"
                | "reports.austrian-tax-summary"
                | "reports.tax-summary"
                | "reports.journal-entries"
        )
    {
        return Err(AppError::new(
            "not_implemented",
            "CSV report exports are not implemented in this Rust read slice",
        )
        .with_details(dict([("component", s("cli")), ("command", s(command))])));
    }
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
            ("year", args.get("year").cloned().unwrap_or(Value::Null)),
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
