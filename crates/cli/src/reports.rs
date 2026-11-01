//! Reports family transport. Accounting stays in kassiber-reports.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const COMMANDS: &[&str] = &[
    "reports.summary",
    "reports.commercial-subledger",
    "reports.export-commercial-subledger-csv",
    "reports.balance-sheet",
    "reports.portfolio-summary",
    "reports.capital-gains",
    "reports.explain-capital-gain",
    "reports.filed-snapshots.list",
    "reports.filed-snapshots.create",
    "reports.tax-summary",
    "reports.journal-entries",
    "reports.austrian-e1kv",
    "reports.austrian-tax-summary",
    "reports.balance-history",
    "reports.export-csv",
    "reports.export-xlsx",
    "reports.exit-tax",
    "reports.export-exit-tax-xlsx",
    "reports.export-austrian-e1kv-csv",
    "reports.export-austrian-e1kv-xlsx",
    "transactions.export",
    "reports.cross-check",
    "reports.export-year-handoff",
    "reports.privacy-hygiene",
    "source-funds.sources.list",
    "source-funds.links.list",
    "source-funds.cases.list",
    "source-funds.recipients.list",
    "source-funds.sources.create",
    "source-funds.sources.attach",
    "source-funds.recipients.create",
    "source-funds.recipients.update",
    "source-funds.recipients.delete",
];
pub fn kind(command: &str) -> Option<&'static str> {
    match command {
        "reports.filed-snapshots.list" => Some("reports.filed-snapshots.list"),
        "reports.filed-snapshots.create" => Some("reports.filed-snapshots.create"),
        _ => None,
    }
}
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    if command.starts_with("source-funds.") {
        return book.reports(command, &source_funds_request(args));
    }
    if command == "reports.cross-check" {
        return book.reports(command, &cross_check_request(args));
    }
    if matches!(
        command,
        "reports.export-csv"
            | "reports.export-xlsx"
            | "reports.export-exit-tax-xlsx"
            | "reports.export-austrian-e1kv-csv"
            | "reports.export-austrian-e1kv-xlsx"
            | "reports.export-commercial-subledger-csv"
            | "transactions.export"
            | "reports.export-year-handoff"
    ) {
        return book.reports(command, &export_request(args));
    }
    if command == "reports.filed-snapshots.create" {
        return book.reports(command, &snapshot_request(args));
    }
    book.reports(command, &request(args))
}

fn source_funds_request(args: &Namespace) -> Value {
    dict(
        [
            "workspace",
            "profile",
            "target_transaction",
            "state",
            "source_type",
            "label",
            "asset",
            "amount",
            "fiat_currency",
            "fiat_value",
            "acquired_at",
            "description",
            "attachments",
            "source",
            "attachment",
            "recipient",
            "kind",
            "default_reveal_mode",
            "notes",
        ]
        .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
    )
}

fn cross_check_request(args: &Namespace) -> Value {
    dict(
        [
            "workspace",
            "profile",
            "tool",
            "file",
            "wallets",
            "year",
            "tool_sources",
            "include_tool_wallets",
            "xlsx",
        ]
        .map(|k| (k, args.get(k).cloned().unwrap_or(Value::Null))),
    )
}

fn export_request(args: &Namespace) -> Value {
    dict(
        [
            "workspace",
            "profile",
            "wallet",
            "wallets",
            "file",
            "dir",
            "history_limit",
            "verify",
            "year",
            "export_format",
            "period",
            "from_day",
            "to_day",
            "timezone",
            "departure_date",
            "destination",
        ]
        .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
    )
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
        (
            "reference",
            args.get("reference").cloned().unwrap_or(Value::Null),
        ),
        (
            "departure_date",
            args.get("departure_date").cloned().unwrap_or(Value::Null),
        ),
        (
            "destination",
            args.get("destination").cloned().unwrap_or(Value::Null),
        ),
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
    fn source_funds_recipient_cli_forwards_fields_and_registers_mutations() {
        for (action, fields) in [
            (
                "create",
                vec![
                    "--label",
                    "Recipient",
                    "--kind",
                    "bank",
                    "--default-reveal-mode",
                    "minimal",
                    "--notes",
                    "Evidence",
                ],
            ),
            (
                "update",
                vec![
                    "--recipient",
                    "r-a",
                    "--label",
                    "Renamed",
                    "--kind",
                    "lawyer",
                    "--default-reveal-mode",
                    "full",
                    "--notes",
                    "",
                ],
            ),
            ("delete", vec!["--recipient", "r-a"]),
        ] {
            let mut argv = vec![
                "source-funds",
                "recipients",
                action,
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ];
            argv.extend(fields);
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let command = format!("source-funds.recipients.{action}");
            assert!(COMMANDS.contains(&command.as_str()));
            assert!(crate::describe::IMPLEMENTED.contains(&command.as_str()));
            let actual = execute(&mut SourceFundsCapture, &command, &args).unwrap();
            assert_eq!(
                get(&actual, "command"),
                Some(&kassiber_envelope::s(command.as_str()))
            );
            let payload = get(&actual, "args").unwrap();
            for key in [
                "workspace",
                "profile",
                "recipient",
                "label",
                "kind",
                "default_reveal_mode",
                "notes",
            ] {
                assert_eq!(
                    get(payload, key),
                    Some(args.get(key).unwrap_or(&Value::Null)),
                    "{key}"
                );
            }
        }
    }
    #[test]
    fn source_funds_source_cli_forwards_mutations_and_managed_ids() {
        for tail in [
            vec![
                "create",
                "--type",
                "income",
                "--label",
                "Origin",
                "--asset",
                "LBTC",
                "--amount",
                "1.00000000001",
                "--fiat-value",
                "12.34",
                "--fiat-currency",
                "EUR",
                "--acquired-at",
                "2024-01-01",
                "--description",
                "Evidence",
                "--attachment",
                "a-a",
                "--attachment",
                "a-z",
            ],
            vec!["attach", "--source", "s-z", "--attachment", "a-a"],
        ] {
            let mut argv = vec!["source-funds", "sources"];
            argv.extend(tail);
            argv.extend(["--workspace", "Main", "--profile", "Book"]);
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let command = format!(
                "source-funds.sources.{}",
                args.text("source_funds_sources_command").unwrap()
            );
            assert!(COMMANDS.contains(&command.as_str()));
            assert!(crate::describe::IMPLEMENTED.contains(&command.as_str()));
            let actual = execute(&mut SourceFundsCapture, &command, &args).unwrap();
            let request = get(&actual, "args").unwrap();
            for key in [
                "workspace",
                "profile",
                "source_type",
                "label",
                "asset",
                "amount",
                "fiat_currency",
                "fiat_value",
                "acquired_at",
                "description",
                "attachments",
                "source",
                "attachment",
            ] {
                assert_eq!(
                    get(request, key).unwrap(),
                    args.get(key).unwrap_or(&Value::Null),
                    "{key}"
                );
            }
        }
    }
    struct SourceFundsCapture;
    impl Book for SourceFundsCapture {
        fn reports(&mut self, command: &str, args: &Value) -> Result<Value, AppError> {
            Ok(dict([
                ("command", kassiber_envelope::s(command)),
                ("args", args.clone()),
            ]))
        }
        fn is_initialized(&self) -> Result<bool, AppError> {
            unreachable!()
        }
        fn context(&self) -> Result<kassiber_runtime::store::BookContext, AppError> {
            unreachable!()
        }
        fn status_facts(&self) -> Result<kassiber_runtime::store::StatusFacts, AppError> {
            unreachable!()
        }
        fn resolve_scope(
            &self,
            _: Option<&Value>,
            _: Option<&Value>,
        ) -> Result<(String, String), AppError> {
            unreachable!()
        }
        fn rollback(&mut self) -> Result<(), AppError> {
            unreachable!()
        }
        fn close(&mut self) -> Result<(), AppError> {
            unreachable!()
        }
    }
    #[test]
    fn source_funds_stored_cli_forwards_scope_filters_and_registers_only_lists() {
        let argv = [
            "source-funds",
            "links",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Book",
            "--target-transaction",
            "external-in",
            "--state",
            "reviewed",
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
        let payload = source_funds_request(&args);
        for key in ["workspace", "profile", "target_transaction", "state"] {
            assert_eq!(get(&payload, key), args.get(key));
        }
        for command in [
            "source-funds.sources.list",
            "source-funds.links.list",
            "source-funds.cases.list",
            "source-funds.recipients.list",
        ] {
            assert!(COMMANDS.contains(&command));
            assert!(crate::describe::IMPLEMENTED.contains(&command));
            let actual = execute(&mut SourceFundsCapture, command, &args).unwrap();
            assert_eq!(
                get(&actual, "command"),
                Some(&kassiber_envelope::s(command))
            );
            assert_eq!(get(&actual, "args"), Some(&payload));
        }
        for command in [
            "reports.source-funds",
            "reports.export-source-funds-pdf",
            "reports.export-source-funds-bundle",
            "source-funds.suggest",
            "source-funds.assemble",
            "source-funds.links.create",
        ] {
            assert!(!COMMANDS.contains(&command));
            assert!(!crate::describe::IMPLEMENTED.contains(&command));
        }
    }
    #[test]
    fn cross_check_cli_forwards_filters_files_and_native_registration() {
        let args = crate::argparse::parse_args(
            &[
                "reports",
                "cross-check",
                "--tool",
                "cointracking",
                "--file",
                "source.csv",
                "--wallet",
                "cold",
                "--wallet",
                "hot",
                "--year",
                "2024",
                "--source",
                "Exchange",
                "--include-tool-wallets",
                "--xlsx",
                "out.xlsx",
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ]
            .map(String::from),
            crate::argparse::Context {
                width: 78,
                version: "test",
            },
        )
        .unwrap();
        let payload = cross_check_request(&args);
        for k in [
            "workspace",
            "profile",
            "tool",
            "file",
            "wallets",
            "year",
            "tool_sources",
            "include_tool_wallets",
            "xlsx",
        ] {
            assert_eq!(get(&payload, k), args.get(k));
        }
        assert!(COMMANDS.contains(&"reports.cross-check"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.cross-check"));
    }
    #[test]
    fn gain_cli_forwards_complete_reference_and_scope() {
        let args = crate::argparse::parse_args(
            &[
                "reports",
                "explain-capital-gain",
                "--reference",
                r#"{"entry_id":"z"}"#,
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ]
            .map(String::from),
            crate::argparse::Context {
                width: 78,
                version: "test",
            },
        )
        .unwrap();
        let payload = request(&args);
        for key in ["reference", "workspace", "profile"] {
            assert_eq!(get(&payload, key), args.get(key));
        }
        assert!(COMMANDS.contains(&"reports.explain-capital-gain"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.explain-capital-gain"));
    }
    #[test]
    fn exit_tax_transport_forwards_departure_and_destination() {
        for command in ["exit-tax", "export-exit-tax-xlsx"] {
            let mut argv = vec![
                "reports",
                command,
                "--departure-date",
                "2024-01-01",
                "--destination",
                "third_country",
            ];
            if command.ends_with("xlsx") {
                argv.extend(["--file", "exit.xlsx"]);
            }
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let request = if command.ends_with("xlsx") {
                export_request(&args)
            } else {
                request(&args)
            };
            assert_eq!(
                kassiber_envelope::get(&request, "departure_date"),
                Some(&kassiber_envelope::s("2024-01-01"))
            );
            assert_eq!(
                kassiber_envelope::get(&request, "destination"),
                Some(&kassiber_envelope::s("third_country"))
            );
            let full = format!("reports.{command}");
            assert!(COMMANDS.contains(&full.as_str()));
            assert!(crate::describe::IMPLEMENTED.contains(&full.as_str()));
        }
    }
    #[test]
    fn full_xlsx_transport_retains_verification_scope_and_history() {
        for verify in [true, false] {
            let mut argv = vec![
                "reports",
                "export-xlsx",
                "--file",
                "full.xlsx",
                "--wallet",
                "hot",
                "--history-limit",
                "1",
            ];
            if !verify {
                argv.push("--no-verify");
            }
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let request = export_request(&args);
            assert_eq!(get(&request, "verify"), Some(&Value::Bool(verify)));
            assert_eq!(get(&request, "wallet"), args.get("wallet"));
            assert_eq!(get(&request, "history_limit"), args.get("history_limit"));
            assert!(COMMANDS.contains(&"reports.export-xlsx"));
            assert!(crate::describe::IMPLEMENTED.contains(&"reports.export-xlsx"));
        }
    }

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
    #[test]
    fn year_handoff_cli_forwards_year_file_and_book_scope() {
        let args = crate::argparse::parse_args(
            &[
                "reports",
                "export-year-handoff",
                "--year",
                "2024",
                "--file",
                "book.xlsx",
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ]
            .map(str::to_owned),
            crate::argparse::Context {
                width: 78,
                version: "test",
            },
        )
        .unwrap();
        let request = export_request(&args);
        assert_eq!(get(&request, "year"), Some(&Value::Int(2024.into())));
        assert_eq!(
            get(&request, "file"),
            Some(&kassiber_envelope::s("book.xlsx"))
        );
        assert_eq!(
            get(&request, "workspace"),
            Some(&kassiber_envelope::s("Main"))
        );
        assert_eq!(
            get(&request, "profile"),
            Some(&kassiber_envelope::s("Book"))
        );
        assert!(COMMANDS.contains(&"reports.export-year-handoff"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.export-year-handoff"));
    }

    #[test]
    fn privacy_hygiene_cli_preserves_scope_and_output_shape() {
        for format in ["json", "table", "plain", "csv"] {
            let args = crate::argparse::parse_args(
                &[
                    "--format",
                    format,
                    "reports",
                    "privacy-hygiene",
                    "--workspace",
                    "Main",
                    "--profile",
                    "Book",
                ]
                .map(str::to_owned),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let payload = request(&args);
            assert_eq!(
                get(&payload, "workspace"),
                Some(&kassiber_envelope::s("Main"))
            );
            assert_eq!(
                get(&payload, "profile"),
                Some(&kassiber_envelope::s("Book"))
            );
            assert_eq!(get(&payload, "csv"), Some(&Value::Bool(format == "csv")));
            assert_eq!(
                get(&payload, "text"),
                Some(&Value::Bool(matches!(format, "table" | "plain")))
            );
        }
        assert!(COMMANDS.contains(&"reports.privacy-hygiene"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.privacy-hygiene"));
        assert!(!COMMANDS.contains(&"reports.privacy-mirror"));
        assert!(!COMMANDS.contains(&"reports.psbt-privacy"));
    }
}
