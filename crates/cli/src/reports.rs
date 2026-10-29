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
    "reports.tax-summary",
    "reports.journal-entries",
    "reports.austrian-e1kv",
    "reports.austrian-tax-summary",
    "reports.balance-history",
];
pub fn kind(command: &str) -> Option<&'static str> {
    (command == "reports.filed-snapshots.list").then_some("reports.filed-snapshots.list")
}
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    book.reports(command, &request(args))
}

fn request(args: &Namespace) -> Value {
    dict([
        ("csv", Value::Bool(args.text("format") == Some("csv"))),
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
            "interval",
            args.get("interval").cloned().unwrap_or(Value::Null),
        ),
        ("start", args.get("start").cloned().unwrap_or(Value::Null)),
        ("end", args.get("end").cloned().unwrap_or(Value::Null)),
        (
            "account",
            args.get("account").cloned().unwrap_or(Value::Null),
        ),
        ("asset", args.get("asset").cloned().unwrap_or(Value::Null)),
        (
            "workspace",
            args.get("workspace").cloned().unwrap_or(Value::Null),
        ),
        (
            "profile",
            args.get("profile").cloned().unwrap_or(Value::Null),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;
    #[test]
    fn report_csv_arguments_select_payload_without_terminal_text() {
        for format in ["csv", "table", "plain", "json"] {
            let argv = [
                "--format",
                format,
                "reports",
                "austrian-tax-summary",
                "--year",
                "2024",
            ]
            .map(String::from);
            let args = crate::argparse::parse_args(
                &argv,
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let request = request(&args);
            assert_eq!(get(&request, "csv"), Some(&Value::Bool(format == "csv")));
            assert_eq!(
                get(&request, "text"),
                Some(&Value::Bool(matches!(format, "table" | "plain")))
            );
            assert_eq!(get(&request, "year"), Some(&Value::Int(2024.into())));
        }
    }
}
