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
    "reports.filed-snapshots.create",
    "reports.tax-summary",
    "reports.journal-entries",
    "reports.austrian-e1kv",
    "reports.austrian-tax-summary",
    "reports.balance-history",
    "reports.export-csv",
    "reports.export-austrian-e1kv-csv",
    "transactions.export",
];
pub fn kind(command: &str) -> Option<&'static str> {
    match command {
        "reports.filed-snapshots.list" => Some("reports.filed-snapshots.list"),
        "reports.filed-snapshots.create" => Some("reports.filed-snapshots.create"),
        _ => None,
    }
}
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    if matches!(
        command,
        "reports.export-csv" | "reports.export-austrian-e1kv-csv" | "transactions.export"
    ) {
        return book.reports(
            command,
            &dict(
                [
                    "workspace",
                    "profile",
                    "wallet",
                    "wallets",
                    "file",
                    "dir",
                    "history_limit",
                    "year",
                    "export_format",
                    "period",
                    "from_day",
                    "to_day",
                    "timezone",
                ]
                .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
            ),
        );
    }
    if command == "reports.filed-snapshots.create" {
        return book.reports(command, &snapshot_request(args));
    }
    book.reports(command, &request(args))
}

fn snapshot_request(args: &Namespace) -> Value {
    dict(
        [
            "workspace",
            "profile",
            "report_kind",
            "state",
            "period_start_year",
            "period_end_year",
            "content_sha256",
            "saved_snapshot_id",
            "classification_summary_json",
            "gain_summary_json",
            "report_scope_json",
            "notes",
        ]
        .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
    )
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
    fn snapshot_create_transport_keeps_authored_fields() {
        let argv = [
            "reports",
            "filed-snapshots",
            "create",
            "--state",
            "filed",
            "--saved-snapshot-id",
            "saved",
            "--report-kind",
            "full-report.csv",
            "--period-start-year",
            "2024",
            "--period-end-year",
            "2025",
            "--content-sha256",
            "hash",
            "--classification-summary-json",
            "{}",
            "--gain-summary-json",
            "{}",
            "--report-scope-json",
            "{}",
            "--notes",
            "Review",
            "--workspace",
            "Main",
            "--profile",
            "Book",
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
        let request = snapshot_request(&args);
        for field in [
            "workspace",
            "profile",
            "report_kind",
            "state",
            "period_start_year",
            "period_end_year",
            "content_sha256",
            "saved_snapshot_id",
            "classification_summary_json",
            "gain_summary_json",
            "report_scope_json",
            "notes",
        ] {
            assert_eq!(get(&request, field), args.get(field), "{field}");
        }
        assert!(COMMANDS.contains(&"reports.filed-snapshots.create"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.filed-snapshots.create"));
        assert_eq!(
            kind("reports.filed-snapshots.create"),
            Some("reports.filed-snapshots.create")
        );
    }
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
