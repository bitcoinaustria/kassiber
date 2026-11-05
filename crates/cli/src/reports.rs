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
    "reports.export-pdf",
    "reports.export-summary-pdf",
    "reports.export-exit-tax-pdf",
    "reports.export-austrian-e1kv-pdf",
    "reports.export-austrian",
    "reports.export-source-funds-pdf",
    "reports.export-source-funds-bundle",
    "reports.export-xlsx",
    "reports.exit-tax",
    "reports.export-exit-tax-xlsx",
    "reports.export-austrian-e1kv-csv",
    "reports.export-austrian-e1kv-xlsx",
    "transactions.export",
    "reports.cross-check",
    "reports.lightning-profitability",
    "reports.export-lightning-profitability-csv",
    "reports.export-year-handoff",
    "reports.privacy-hygiene",
    "reports.source-funds",
    "source-funds.sources.list",
    "source-funds.links.list",
    "source-funds.cases.list",
    "source-funds.recipients.list",
    "source-funds.sources.create",
    "source-funds.sources.attach",
    "source-funds.recipients.create",
    "source-funds.recipients.update",
    "source-funds.recipients.delete",
    "source-funds.links.create",
    "source-funds.links.review",
    "source-funds.links.attach",
    "source-funds.links.bulk-review",
    "source-funds.review-context",
    "source-funds.request-input",
    "source-funds.coverage",
    "source-funds.suggest",
    "source-funds.assemble",
    "reports.psbt-privacy",
];
pub fn kind(command: &str) -> Option<&'static str> {
    match command {
        "reports.filed-snapshots.list" => Some("reports.filed-snapshots.list"),
        "reports.filed-snapshots.create" => Some("reports.filed-snapshots.create"),
        _ => None,
    }
}
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    if command == "reports.psbt-privacy" {
        return book.reports(command, &psbt_request(args)?);
    }
    if matches!(
        command,
        "reports.lightning-profitability" | "reports.export-lightning-profitability-csv"
    ) {
        return book.reports(command, &lightning_request(args));
    }
    if command == "reports.source-funds" {
        return book.reports(command, &source_funds_report_request(args));
    }
    if command.starts_with("source-funds.") {
        if matches!(command, "source-funds.suggest" | "source-funds.assemble") {
            return book.reports(
                command,
                &dict(
                    [
                        "workspace",
                        "profile",
                        "target_transaction",
                        "include_broad_hints",
                        "max_suggestions",
                        "max_passes",
                        "dry_run",
                    ]
                    .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
                ),
            );
        }
        if command == "source-funds.coverage" {
            return book.reports(
                command,
                &dict(
                    [
                        "workspace",
                        "profile",
                        "max_depth",
                        "max_transactions",
                        "format",
                    ]
                    .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
                ),
            );
        }
        if matches!(
            command,
            "source-funds.review-context" | "source-funds.request-input"
        ) {
            return book.reports(
                command,
                &dict(
                    [
                        "workspace",
                        "profile",
                        "target_transaction",
                        "recipe_file",
                        "action",
                        "expected_review_fingerprint",
                        "explanation",
                    ]
                    .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
                ),
            );
        }
        return book.reports(command, &source_funds_request(args));
    }
    if command == "reports.cross-check" {
        return book.reports(command, &cross_check_request(args));
    }
    if matches!(
        command,
        "reports.export-csv"
            | "reports.export-pdf"
            | "reports.export-summary-pdf"
            | "reports.export-exit-tax-pdf"
            | "reports.export-austrian-e1kv-pdf"
            | "reports.export-austrian"
            | "reports.export-source-funds-pdf"
            | "reports.export-source-funds-bundle"
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

fn source_funds_report_request(args: &Namespace) -> Value {
    dict(
        [
            "workspace",
            "profile",
            "target_transaction",
            "target_amount",
            "purpose",
            "planned_destination",
            "planned_note",
            "reveal_mode",
            "max_depth",
            "recipient",
            "diagram_detail",
            "amount_precision",
            "mask_recipient",
            "omit_section",
            "reveal_override",
            "format",
            "save_case",
            "case_label",
        ]
        .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null))),
    )
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
            "from_transaction",
            "from_source",
            "to_transaction",
            "link_type",
            "confidence",
            "method",
            "allocation_amount",
            "from_asset",
            "allocation_policy",
            "explanation",
            "uses_chain_observation",
            "chain_data_confirmed",
            "link",
            "dry_run",
        ]
        .map(|key| (key, args.get(key).cloned().unwrap_or(Value::Null)))
        .into_iter()
        .chain([(
            "from_allocation_amount",
            args.get("from_amount").cloned().unwrap_or(Value::Null),
        )]),
    )
}

fn lightning_request(args: &Namespace) -> Value {
    dict(
        ["connection", "window_days", "file", "workspace", "profile"]
            .map(|k| (k, args.get(k).cloned().unwrap_or(Value::Null))),
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

fn psbt_request(args: &Namespace) -> Result<Value, AppError> {
    let psbt = match args.text("psbt") {
        Some(text) => text.to_owned(),
        None => read_psbt_file(args.text("psbt_file").unwrap_or(""))?,
    };
    Ok(dict([
        (
            "workspace",
            args.get("workspace").cloned().unwrap_or(Value::Null),
        ),
        (
            "profile",
            args.get("profile").cloned().unwrap_or(Value::Null),
        ),
        ("psbt", kassiber_envelope::s(psbt)),
        (
            "table_rows",
            Value::Bool(matches!(
                args.text("format"),
                Some("table" | "plain" | "csv")
            )),
        ),
    ]))
}

fn read_psbt_file(path: &str) -> Result<String, AppError> {
    use kassiber_envelope::s;
    if path.is_empty() {
        return Ok(String::new());
    }
    if path.contains('\0') {
        return Err(AppError::new("internal_error", "embedded null byte"));
    }
    // Match the existing literal UTF-8 text-file boundary, including refusal
    // precedence. This report's CLI file option does not expand user paths.
    let bytes = std::fs::read(path).map_err(|_| {
        AppError::new("validation", "Could not read PSBT file")
            .with_hint("Choose a readable local file path.")
            .with_details(dict([("path", s(path))]))
    })?;
    let text = String::from_utf8(bytes).map_err(|error| {
        let bytes = error.as_bytes();
        let invalid = error.utf8_error();
        let at = invalid.valid_up_to();
        let (end, reason) = match invalid.error_len() {
            Some(1) if matches!(bytes[at], 0x80..=0xC1 | 0xF5..=0xFF) => (at, "invalid start byte"),
            Some(n) => (at + n - 1, "invalid continuation byte"),
            None => (bytes.len() - 1, "unexpected end of data"),
        };
        AppError::new(
            "internal_error",
            if end == at {
                format!(
                    "'utf-8' codec can't decode byte 0x{:02x} in position {at}: {reason}",
                    bytes[at]
                )
            } else {
                format!("'utf-8' codec can't decode bytes in position {at}-{end}: {reason}")
            },
        )
    })?;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = kassiber_pytext::strip(&text);
    for (refused, message) in [
        (text.is_empty(), "PSBT file is empty"),
        (text.contains('\0'), "PSBT file contains NUL bytes"),
    ] {
        if refused {
            return Err(
                AppError::new("validation", message).with_details(dict([("path", s(path))]))
            );
        }
    }
    Ok(text.to_owned())
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
            "start",
            "end",
            "include_snapshot",
            "case",
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
    fn source_funds_link_cli_forwards_fields_and_registers_mutations() {
        for tail in [
            vec![
                "create",
                "--from-transaction",
                "in",
                "--to-transaction",
                "out",
                "--type",
                "self_transfer",
                "--state",
                "suggested",
                "--confidence",
                "weak",
                "--method",
                "exact method",
                "--asset",
                "BTC",
                "--from-asset",
                "BTC",
                "--allocation-amount",
                "0.00000100001",
                "--from-amount",
                "0.00000100002",
                "--allocation-policy",
                "explicit",
                "--explanation",
                "Provenance",
                "--uses-chain-observation",
                "--chain-data-confirmed",
                "--attachment",
                "a-a",
                "--attachment",
                "a-z",
            ],
            vec![
                "review",
                "--link",
                "l-z",
                "--state",
                "reviewed",
                "--from-amount",
                "0.00000100002",
                "--no-chain-observation",
                "--unconfirmed-chain-data",
            ],
            vec!["attach", "--link", "l-z", "--attachment", "a-a"],
        ] {
            let command = format!("source-funds.links.{}", tail[0]);
            let mut argv = vec!["source-funds", "links"];
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
                "from_transaction",
                "from_source",
                "to_transaction",
                "link_type",
                "state",
                "confidence",
                "method",
                "asset",
                "allocation_amount",
                "from_asset",
                "allocation_policy",
                "explanation",
                "uses_chain_observation",
                "chain_data_confirmed",
                "attachments",
                "link",
                "attachment",
            ] {
                assert_eq!(
                    get(payload, key),
                    Some(args.get(key).unwrap_or(&Value::Null)),
                    "{key}"
                );
            }
            assert_eq!(
                get(payload, "from_allocation_amount"),
                Some(args.get("from_amount").unwrap_or(&Value::Null))
            );
        }
    }
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
    fn source_funds_batch_cli_forwards_target_scope_and_dry_run() {
        let command = "source-funds.links.bulk-review";
        assert!(COMMANDS.contains(&command));
        assert!(crate::describe::IMPLEMENTED.contains(&command));
        for dry_run in [false, true] {
            let mut argv = vec![
                "source-funds",
                "links",
                "bulk-review",
                "--target-transaction",
                "out",
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ];
            if dry_run {
                argv.push("--dry-run")
            }
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let result = execute(&mut SourceFundsCapture, command, &args).unwrap();
            let payload = get(&result, "args").unwrap();
            for (key, expected) in [
                ("target_transaction", "out"),
                ("workspace", "Main"),
                ("profile", "Book"),
            ] {
                assert_eq!(get(payload, key), Some(&kassiber_envelope::s(expected)));
            }
            assert_eq!(get(payload, "dry_run"), Some(&Value::Bool(dry_run)));
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
        // The source-of-funds exports are native report exports.
        for command in [
            "reports.export-source-funds-pdf",
            "reports.export-source-funds-bundle",
        ] {
            assert!(COMMANDS.contains(&command));
            assert!(crate::describe::IMPLEMENTED.contains(&command));
        }
    }
    #[test]
    fn source_funds_report_cli_forwards_recipe_and_keeps_mutation_boundary() {
        let argv = [
            "--format",
            "plain",
            "reports",
            "source-funds",
            "--target-transaction",
            "in",
            "--target-amount",
            "0.1",
            "--purpose",
            "planned_exchange_sale",
            "--planned-destination",
            "Exchange",
            "--planned-note",
            "Note",
            "--recipient",
            "Bank",
            "--reveal-mode",
            "full",
            "--max-depth",
            "12",
            "--diagram-detail",
            "detailed",
            "--amount-precision",
            "sats",
            "--mask-recipient",
            "--omit-section",
            "flow_links",
            "--reveal-override",
            "in=hide",
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
        assert!(COMMANDS.contains(&"reports.source-funds"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.source-funds"));
        assert!(crate::run::implemented("reports.source-funds", &args));
        let actual = execute(&mut SourceFundsCapture, "reports.source-funds", &args).unwrap();
        let payload = get(&actual, "args").unwrap();
        for key in [
            "workspace",
            "profile",
            "target_transaction",
            "target_amount",
            "purpose",
            "planned_destination",
            "planned_note",
            "reveal_mode",
            "max_depth",
            "recipient",
            "diagram_detail",
            "amount_precision",
            "mask_recipient",
            "omit_section",
            "reveal_override",
            "format",
            "save_case",
            "case_label",
        ] {
            assert_eq!(get(payload, key), args.get(key));
        }
        let mut saving = argv.to_vec();
        saving.push("--save-case".into());
        let saving = crate::argparse::parse_args(
            &saving,
            crate::argparse::Context {
                width: 78,
                version: "test",
            },
        )
        .unwrap();
        assert!(crate::run::implemented("reports.source-funds", &saving));
    }
    #[test]
    fn lightning_cli_preserves_action_scope_window_file_and_registration() {
        for command in [
            "lightning-profitability",
            "export-lightning-profitability-csv",
        ] {
            let mut argv = vec![
                "reports",
                command,
                "--connection",
                "node",
                "--window-days",
                "7",
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ];
            if command.starts_with("export") {
                argv.extend(["--file", "target/report.csv"]);
            }
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let payload = lightning_request(&args);
            for key in ["connection", "window_days", "workspace", "profile"] {
                assert_eq!(get(&payload, key), args.get(key));
                assert!(!matches!(get(&payload, key), None | Some(Value::Null)));
            }
            if command.starts_with("export") {
                assert_eq!(
                    get(&payload, "file"),
                    Some(&kassiber_envelope::s("target/report.csv"))
                );
            }
            let full = format!("reports.{command}");
            assert!(COMMANDS.contains(&full.as_str()));
            assert!(crate::describe::IMPLEMENTED.contains(&full.as_str()));
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
    fn privacy_mirror_cli_preserves_scope_and_output_audience() {
        for format in ["json", "table", "plain", "csv"] {
            let args = crate::argparse::parse_args(
                &[
                    "--format",
                    format,
                    "reports",
                    "privacy-mirror",
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
        assert!(COMMANDS.contains(&"reports.psbt-privacy"));
        assert!(crate::describe::IMPLEMENTED.contains(&"reports.psbt-privacy"));
    }
    #[test]
    fn source_funds_review_cli_forwards_fingerprint_file_and_handoff_fields() {
        for command in ["review-context", "request-input"] {
            let mut argv = vec![
                "source-funds",
                command,
                "--target-transaction",
                "in",
                "--recipe-file",
                "inspection.json",
                "--workspace",
                "Main",
                "--profile",
                "Book",
            ];
            if command == "request-input" {
                argv.extend([
                    "--action",
                    "attach_evidence",
                    "--expected-review-fingerprint",
                    "hash",
                    "--explanation",
                    "Evidence",
                ]);
            }
            let args = crate::argparse::parse_args(
                &argv.into_iter().map(String::from).collect::<Vec<_>>(),
                crate::argparse::Context {
                    width: 78,
                    version: "test",
                },
            )
            .unwrap();
            let command = format!("source-funds.{command}");
            assert!(COMMANDS.contains(&command.as_str()));
            assert!(crate::describe::IMPLEMENTED.contains(&command.as_str()));
            assert!(crate::run::implemented(&command, &args));
            let actual = execute(&mut SourceFundsCapture, &command, &args).unwrap();
            for key in [
                "workspace",
                "profile",
                "target_transaction",
                "recipe_file",
                "action",
                "expected_review_fingerprint",
                "explanation",
            ] {
                assert_eq!(
                    get(get(&actual, "args").unwrap(), key),
                    Some(&args.get(key).cloned().unwrap_or(Value::Null))
                );
            }
        }
    }

    #[test]
    fn source_funds_coverage_cli_forwards_scope_limits_and_format() {
        let args = crate::argparse::parse_args(
            &[
                "--format",
                "plain",
                "source-funds",
                "coverage",
                "--workspace",
                "Main",
                "--profile",
                "Book",
                "--max-depth",
                "64",
                "--max-transactions",
                "0",
            ]
            .map(String::from),
            crate::argparse::Context {
                width: 78,
                version: "test",
            },
        )
        .unwrap();
        let command = "source-funds.coverage";
        assert!(COMMANDS.contains(&command));
        assert!(crate::describe::IMPLEMENTED.contains(&command));
        assert!(crate::run::implemented(command, &args));
        let actual = execute(&mut SourceFundsCapture, command, &args).unwrap();
        for key in [
            "workspace",
            "profile",
            "max_depth",
            "max_transactions",
            "format",
        ] {
            assert_eq!(get(get(&actual, "args").unwrap(), key), args.get(key));
        }
    }
    #[test]
    fn source_funds_suggest_cli_forwards_preview_and_persistence() {
        for preview in [false, true] {
            let mut argv: Vec<String> = [
                "source-funds",
                "suggest",
                "--target-transaction",
                "target",
                "--workspace",
                "workspace",
                "--profile",
                "profile",
                "--max-suggestions",
                "2",
                "--include-broad-hints",
            ]
            .map(String::from)
            .into();
            if preview {
                argv.push("--dry-run".into());
            }
            let args = crate::argparse::parse_args(
                &argv,
                crate::argparse::Context {
                    width: 80,
                    version: "test",
                },
            )
            .unwrap();
            assert!(crate::run::implemented("source-funds.suggest", &args));
            let actual = execute(&mut SourceFundsCapture, "source-funds.suggest", &args).unwrap();
            let request = get(&actual, "args").unwrap();
            for key in [
                "workspace",
                "profile",
                "target_transaction",
                "max_suggestions",
                "include_broad_hints",
                "dry_run",
            ] {
                assert_eq!(get(request, key), args.get(key));
            }
        }
    }
    #[test]
    fn source_funds_passes_cli_forwards_assembly_scope_and_cap() {
        let argv = [
            "source-funds",
            "assemble",
            "--target-transaction",
            "target",
            "--max-passes",
            "3",
            "--include-broad-hints",
            "--workspace",
            "Main",
            "--profile",
            "Book",
        ]
        .map(String::from);
        let args = crate::argparse::parse_args(
            &argv,
            crate::argparse::Context {
                width: 80,
                version: "test",
            },
        )
        .unwrap();
        assert!(COMMANDS.contains(&"source-funds.assemble"));
        assert!(crate::run::implemented("source-funds.assemble", &args));
        let actual = execute(&mut SourceFundsCapture, "source-funds.assemble", &args).unwrap();
        let request = get(&actual, "args").unwrap();
        for key in [
            "target_transaction",
            "max_passes",
            "include_broad_hints",
            "workspace",
            "profile",
        ] {
            assert_eq!(get(request, key), args.get(key));
        }
    }
}
