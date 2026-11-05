//! Shared export request admission, calendar resolution and managed filenames.
use super::*;
use kassiber_setup::Sources;
use std::path::{Path, PathBuf};

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "reports.cross-check"
            | "ui.reports.export_cross_check"
            | "reports.export-csv"
            | "reports.export-pdf"
            | "reports.export-summary-pdf"
            | "reports.export-exit-tax-pdf"
            | "reports.export-xlsx"
            | "reports.export-exit-tax-xlsx"
            | "reports.export-commercial-subledger-csv"
            | "reports.export-austrian-e1kv-csv"
            | "reports.export-austrian-e1kv-xlsx"
            | "reports.export-austrian-e1kv-pdf"
            | "reports.export-austrian"
            | "reports.export-source-funds-pdf"
            | "transactions.export"
            | "ui.transactions.export_csv"
            | "ui.transactions.export_xlsx"
            | "ui.transactions.export_platform_csv"
            | "ui.reports.export_xlsx"
            | "ui.reports.export_pdf"
            | "ui.reports.export_summary_pdf"
            | "ui.reports.export_exit_tax_pdf"
            | "reports.export-year-handoff"
            | "ui.reports.export_year_handoff"
            | "ui.reports.export_exit_tax_xlsx"
            | "ui.reports.export_csv"
            | "ui.reports.export_austrian_e1kv_csv"
            | "ui.reports.export_austrian_e1kv_xlsx"
            | "ui.reports.export_austrian_e1kv_pdf"
            | "ui.reports.export_capital_gains_csv"
            | "ui.source_funds.export_pdf"
    )
}
fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn now(conn: &Connection, sources: &dyn Sources) -> Result<(String, i64)> {
    let stamp = sources.now(conn)?;
    let instant = kassiber_rates::time::parse(&stamp)
        .ok_or_else(|| AppError::new("internal_error", "invalid export clock"))?;
    let epoch = kassiber_rates::time::parse("1970-01-01T00:00:00Z").unwrap();
    Ok((stamp, (instant - epoch).div_euclid(1_000_000)))
}
pub(super) fn range(
    conn: &Connection,
    profile: &Value,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Option<export_range::Range>> {
    let tzpath = crate::zone::tzpath(std::env::var("PYTHONTZPATH").ok().as_deref());
    export_range::resolve(
        profile,
        value(args, "range"),
        value(args, "from"),
        value(args, "to"),
        value(args, "timezone"),
        now(conn, sources)?.1,
        &tzpath,
    )
}
pub(super) fn managed(root: &Path, stem: &str, suffix: &str, stamp: &str) -> Result<PathBuf> {
    let root = root.join("reports");
    std::fs::create_dir_all(&root).map_err(|e| export::io_error(e, Some(&root)))?;
    let stamp = stamp
        .get(..19)
        .unwrap_or(stamp)
        .replace(['-', ':'], "")
        .replace('T', "-");
    let base = format!("{stem}-{stamp}");
    let mut candidate = root.join(format!("{base}{suffix}"));
    let mut counter = 2;
    while candidate.exists() {
        candidate = root.join(format!("{base}-{counter}{suffix}"));
        counter += 1;
    }
    Ok(candidate)
}
pub(super) fn freshness(conn: &Connection, profile: &Value) -> Result<()> {
    let id = text(profile, "id");
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM freshness_source_states WHERE profile_id=? AND blocking_reports=1", [&id], |r| r.get(0)).map_err(sql_error)?;
    if count == 0 {
        return Ok(());
    }
    let sources = rows(conn,"SELECT source_type,source_label,status,stale_reason,last_error_code FROM freshness_source_states WHERE profile_id=? AND blocking_reports=1 ORDER BY source_type,source_label,source_key LIMIT 20",[&id])?;
    Err(AppError::new(
        "report_freshness_blocked",
        "Final report export is blocked by source freshness failures",
    )
    .with_hint("Refresh or repair the failed source before exporting a final report.")
    .with_details(dict([
        ("source_count", Value::Int(count.into())),
        ("sources", Value::List(sources)),
    ])))
}
pub(super) fn wallet_args(kind: &str, args: &Value) -> Result<Value> {
    let wallets = value(args, "wallets");
    let wallet = value(args, "wallet");
    if !matches!(wallets, Value::Null) {
        if !matches!(wallet, Value::Null) {
            return Err(validation(format!(
                "{kind} takes wallet or wallets, not both"
            )));
        }
        if !matches!(wallets,Value::List(v) if v.iter().all(|v| matches!(v,Value::Str(_)))) {
            return Err(validation(format!(
                "{kind} wallets must be an array of strings"
            )));
        }
        return Ok(wallets.clone());
    }
    if !matches!(wallet, Value::Null | Value::Str(_)) {
        return Err(validation(format!("{kind} wallet must be a string")));
    }
    Ok(if truthy(wallet) {
        Value::List(vec![wallet.clone()])
    } else {
        Value::Null
    })
}
/// `ui.reports.export_summary_pdf` admits only its own four arguments.
fn summary_args(raw: &Value) -> Result<()> {
    const KIND: &str = "ui.reports.export_summary_pdf";
    if let Value::Dict(fields) = raw {
        let mut unknown: Vec<_> = fields
            .iter()
            .filter_map(|(k, _)| match k {
                kassiber_pyjson::Key::Str(k)
                    if !["start", "end", "wallets", "include_snapshot"]
                        .iter()
                        .any(|a| k.as_str() == Some(a)) =>
                {
                    Some(k.clone())
                }
                _ => None,
            })
            .collect();
        unknown.sort();
        if !unknown.is_empty() {
            return Err(
                validation(format!("{KIND} received unsupported arguments")).with_details(dict([
                    (
                        "unsupported",
                        Value::List(unknown.into_iter().map(Value::Str).collect()),
                    ),
                ])),
            );
        }
    }
    match value(raw, "wallets") {
        Value::Null => (),
        Value::List(v) if v.iter().all(|v| matches!(v, Value::Str(_))) => {
            if v.is_empty() {
                return Err(validation(format!(
                    "{KIND} requires at least one selected wallet"
                )));
            }
        }
        _ => {
            return Err(validation(format!(
                "{KIND} wallets must be an array of strings"
            )))
        }
    }
    if !matches!(get(raw, "include_snapshot"), None | Some(Value::Bool(_))) {
        return Err(validation(format!(
            "{KIND} include_snapshot must be a boolean"
        )));
    }
    Ok(())
}

/// The full report's CSV bundle is a ZIP: a requested path without the
/// `.zip` suffix gets it, as Python's source-of-funds bundle does.
fn csv_bundle_path(command: &str, path: PathBuf) -> PathBuf {
    if command == "reports.export-csv"
        && !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        path.with_extension("zip")
    } else {
        path
    }
}

/// Paths supplied by the CLI have already been expanded by crates/paths.
/// Managed desktop roots come from that same resolved book, never request args.
pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    exports_root: &Path,
    sources: &dyn Sources,
) -> Result<Value> {
    if matches!(
        command,
        "reports.cross-check" | "ui.reports.export_cross_check"
    ) {
        return cross_check_request::execute(conn, command, args, exports_root, sources);
    }
    if command == "reports.export-commercial-subledger-csv" {
        return commercial::export(conn, args);
    }
    export::admitted(conn, || {
        execute_admitted(conn, command, args, exports_root, sources)
    })
}
fn execute_admitted(
    conn: &Connection,
    command: &str,
    raw: &Value,
    exports_root: &Path,
    sources: &dyn Sources,
) -> Result<Value> {
    // `reports export-austrian` is Python's alias of the Austrian PDF.
    let command = if command == "reports.export-austrian" {
        "reports.export-austrian-e1kv-pdf"
    } else {
        command
    };
    let desktop = command.starts_with("ui.");
    let xlsx = command == "ui.reports.export_xlsx"
        || command == "ui.reports.export_austrian_e1kv_xlsx"
        || command == "ui.reports.export_exit_tax_xlsx"
        || command == "ui.transactions.export_xlsx"
        || (command == "transactions.export" && text(raw, "export_format") == "xlsx")
        || command == "ui.reports.export_year_handoff";
    if !desktop && command != "transactions.export" {
        // The CLI admits final accounting exports against source freshness
        // before it reads anything else, as Python's command handler does.
        if matches!(
            command,
            "reports.export-year-handoff"
                | "reports.export-pdf"
                | "reports.export-summary-pdf"
                | "reports.export-exit-tax-pdf"
                | "reports.export-austrian-e1kv-pdf"
        ) {
            let (_, profile) = kassiber_setup::references::scope(conn, raw)?;
            freshness(conn, &profile)?;
        }
        let target = csv_bundle_path(
            command,
            PathBuf::from(text(
                raw,
                if command == "reports.export-austrian-e1kv-csv" {
                    "dir"
                } else {
                    "file"
                },
            )),
        );
        return export::export(conn, command, raw, &target, sources);
    }
    if command == "transactions.export" {
        let days = !matches!(value(raw, "from_day"), Value::Null)
            || !matches!(value(raw, "to_day"), Value::Null);
        if !matches!(value(raw, "period"), Value::Null) && days {
            return Err(validation("Use --period or --from/--to, not both"));
        }
    }
    let mut args = raw.clone();
    if desktop {
        replace(&mut args, "workspace", Value::Null);
        replace(&mut args, "profile", Value::Null);
    }
    let profile = if command == "ui.transactions.export_platform_csv" {
        Value::Null
    } else {
        kassiber_setup::references::scope(conn, &args)?.1
    };
    let mut range_label = None;
    if desktop
        && !matches!(
            command,
            "ui.transactions.export_csv"
                | "ui.transactions.export_xlsx"
                | "ui.transactions.export_platform_csv"
                | "ui.source_funds.export_pdf"
        )
    {
        freshness(conn, &profile)?;
    }
    let (native, stem, suffix, scope) = match command {
        "reports.export-csv"
        | "ui.reports.export_csv"
        | "ui.reports.export_xlsx"
        | "ui.reports.export_pdf" => {
            if desktop {
                if get(raw, "year").is_some() || get(raw, "tax_year").is_some() {
                    return Err(validation(format!(
                        "{command} does not support a tax year; use an annual tax export instead"
                    )));
                }
                if !matches!(value(raw, "wallet"), Value::Null | Value::Str(_)) {
                    return Err(validation(format!("{command} wallet must be a string")));
                }
                if command == "ui.reports.export_xlsx"
                    && !matches!(get(raw, "verify"), None | Some(Value::Bool(_)))
                {
                    return Err(validation(format!("{command} verify must be a boolean")));
                }
                if get(raw, "history_limit").is_none() {
                    replace(&mut args, "history_limit", Value::Int(0.into()));
                }
            }
            (
                if xlsx {
                    "reports.export-xlsx"
                } else if command == "ui.reports.export_pdf" {
                    "reports.export-pdf"
                } else {
                    "reports.export-csv"
                },
                "kassiber-report".into(),
                if xlsx {
                    ".xlsx"
                } else if command == "ui.reports.export_pdf" {
                    ".pdf"
                } else {
                    // The full report's CSV is a bundle of tables.
                    ".zip"
                },
                "report",
            )
        }
        "ui.reports.export_summary_pdf" => {
            summary_args(raw)?;
            (
                "reports.export-summary-pdf",
                "kassiber-summary-report".into(),
                ".pdf",
                "summary_report",
            )
        }
        "ui.reports.export_year_handoff" => {
            if let Value::Dict(fields) = raw {
                let mut unknown: Vec<_> = fields
                    .iter()
                    .filter_map(|(k, _)| match k {
                        kassiber_pyjson::Key::Str(k) if k.as_str() != Some("year") => {
                            Some(k.clone())
                        }
                        _ => None,
                    })
                    .collect();
                unknown.sort();
                if !unknown.is_empty() {
                    return Err(validation(
                        "ui.reports.export_year_handoff received unsupported arguments",
                    )
                    .with_details(dict([(
                        "unsupported",
                        Value::List(unknown.into_iter().map(Value::Str).collect()),
                    )])));
                }
            }
            if !matches!(value(raw, "year"), Value::Int(_)) {
                return Err(validation(
                    "ui.reports.export_year_handoff requires an integer year",
                ));
            }
            (
                "reports.export-year-handoff",
                format!("kassiber-year-handoff-{}", py_str(value(raw, "year"))),
                ".xlsx",
                "year_handoff",
            )
        }
        "ui.reports.export_exit_tax_xlsx" | "ui.reports.export_exit_tax_pdf" => {
            let report = execute_with_sources(conn, "reports.exit-tax", &args, sources)?;
            let departure = value(&report, "departureDate").clone();
            replace(&mut args, "departure_date", departure.clone());
            let pdf = command == "ui.reports.export_exit_tax_pdf";
            (
                if pdf {
                    "reports.export-exit-tax-pdf"
                } else {
                    "reports.export-exit-tax-xlsx"
                },
                format!("kassiber-exit-tax-{}", py_str(&departure)),
                if pdf { ".pdf" } else { ".xlsx" },
                "exit_tax",
            )
        }
        "ui.source_funds.export_pdf" => {
            if !matches!(value(raw, "case"), Value::Null | Value::Str(_)) {
                return Err(validation(
                    "ui.source_funds.export_pdf case must be a string",
                ));
            }
            (
                "reports.export-source-funds-pdf",
                "kassiber-source-funds".into(),
                ".pdf",
                "source_funds",
            )
        }
        "ui.reports.export_austrian_e1kv_pdf" => (
            "reports.export-austrian-e1kv-pdf",
            format!("kassiber-austrian-e1kv-{}", py_str(value(raw, "year"))),
            ".pdf",
            "austrian_e1kv",
        ),
        "ui.reports.export_austrian_e1kv_xlsx" => (
            "reports.export-austrian-e1kv-xlsx",
            format!("kassiber-austrian-e1kv-{}", py_str(value(raw, "year"))),
            ".xlsx",
            "austrian_e1kv",
        ),
        "reports.export-austrian-e1kv-csv" | "ui.reports.export_austrian_e1kv_csv" => (
            "reports.export-austrian-e1kv-csv",
            format!("kassiber-austrian-e1kv-{}-csv", py_str(value(raw, "year"))),
            "",
            "austrian_e1kv",
        ),
        "ui.reports.export_capital_gains_csv" => {
            let year = value(raw, "year");
            let stem = if matches!(year, Value::Null) {
                "kassiber-capital-gains".into()
            } else {
                format!("kassiber-capital-gains-{}", py_str(year))
            };
            (
                "reports.export-capital-gains-csv",
                stem,
                ".csv",
                "capital_gains",
            )
        }
        "transactions.export"
        | "ui.transactions.export_csv"
        | "ui.transactions.export_xlsx"
        | "ui.transactions.export_platform_csv" => {
            if desktop {
                let mut allowed = vec!["wallet", "wallets", "range", "from", "to", "timezone"];
                if command == "ui.transactions.export_platform_csv" {
                    allowed.push("platform");
                }
                if let Value::Dict(fields) = raw {
                    let mut unknown: Vec<_> = fields
                        .iter()
                        .filter_map(|(k, _)| match k {
                            kassiber_pyjson::Key::Str(k)
                                if !allowed.iter().any(|a| k.as_str() == Some(a)) =>
                            {
                                Some(k.clone())
                            }
                            _ => None,
                        })
                        .collect();
                    unknown.sort();
                    if !unknown.is_empty() {
                        return Err(validation(format!(
                            "{command} received unsupported arguments"
                        ))
                        .with_details(dict([(
                            "unsupported",
                            Value::List(unknown.into_iter().map(Value::Str).collect()),
                        )])));
                    }
                }
                if command == "ui.transactions.export_platform_csv"
                    && !matches!(value(raw, "platform"), Value::Str(_))
                {
                    return Err(validation(format!("{command} requires a platform")));
                }
                replace(&mut args, "wallets", wallet_args(command, raw)?);
                for name in ["range", "from", "to", "timezone"] {
                    let v = value(raw, name);
                    if !matches!(v, Value::Null | Value::Str(_)) {
                        return Err(validation(format!("{command} {name} must be a string")));
                    }
                    if matches!(v,Value::Str(v) if v.as_str().is_some_and(|v| kassiber_pytext::strip(v).is_empty()))
                    {
                        return Err(validation(format!("{command} {name} must not be empty")));
                    }
                }
            } else {
                if !["csv", "xlsx", "cointracking-csv", "blockpit-csv"]
                    .contains(&text(raw, "export_format").as_str())
                {
                    return Err(AppError::new(
                        "not_implemented",
                        "This transaction export format is not implemented in the Rust backend yet",
                    ));
                }
                let period = value(raw, "period");
                let days = !matches!(value(raw, "from_day"), Value::Null)
                    || !matches!(value(raw, "to_day"), Value::Null);
                replace(
                    &mut args,
                    "range",
                    if truthy(period) {
                        s(py_str(period).replace('-', "_"))
                    } else {
                        Value::Null
                    },
                );
                replace(&mut args, "from", value(raw, "from_day").clone());
                replace(&mut args, "to", value(raw, "to_day").clone());
                // Python ignores a timezone alone on the CLI.
                if matches!(period, Value::Null) && !days {
                    replace(&mut args, "timezone", Value::Null);
                }
            }
            let profile = if command == "ui.transactions.export_platform_csv"
                && ["range", "from", "to", "timezone"]
                    .iter()
                    .any(|key| !matches!(value(raw, key), Value::Null))
            {
                kassiber_setup::references::scope(conn, &args)?.1
            } else {
                profile.clone()
            };
            if let Some(range) = range(conn, &profile, &args, sources)? {
                range_label = Some(
                    range
                        .label()
                        .map_err(|e| AppError::new("internal_error", e))?,
                );
                replace(&mut args, "export_range", range.payload()?);
            }
            let platform = if command == "ui.transactions.export_platform_csv" {
                Some(platform_csv::platform(value(raw, "platform"))?)
            } else if command == "transactions.export" {
                match text(raw, "export_format").as_str() {
                    "cointracking-csv" => Some("cointracking"),
                    "blockpit-csv" => Some("blockpit"),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(platform) = platform {
                replace(&mut args, "platform", s(platform));
            }
            let stem = format!(
                "{}{}",
                platform
                    .map(|p| format!("kassiber-{p}"))
                    .unwrap_or_else(|| "kassiber-transactions".into()),
                range_label.map(|l| format!("-{l}")).unwrap_or_default()
            );
            (
                if platform.is_some() {
                    "reports.export-platform-csv"
                } else if xlsx {
                    "reports.export-transactions-xlsx"
                } else {
                    "reports.export-transactions-csv"
                },
                stem,
                if xlsx { ".xlsx" } else { ".csv" },
                "transactions",
            )
        }
        _ => {
            return Err(AppError::new(
                "not_implemented",
                format!("{command} is not implemented in the Rust backend yet"),
            ))
        }
    };
    let target = if desktop {
        managed(exports_root, &stem, suffix, &now(conn, sources)?.0)?
    } else {
        PathBuf::from(text(
            raw,
            if native == "reports.export-austrian-e1kv-csv" {
                "dir"
            } else {
                "file"
            },
        ))
    };
    let mut result = export::export(conn, native, &args, &target, sources)?;
    if desktop {
        let format = if xlsx {
            "xlsx"
        } else if native.ends_with("-pdf") {
            "pdf"
        } else {
            "csv"
        };
        replace(&mut result, "format", s(format));
        replace(&mut result, "scope", s(scope));
        if scope == "capital_gains" {
            replace(&mut result, "tax_year", value(raw, "year").clone());
        }
        replace(
            &mut result,
            "filename",
            s(target.file_name().unwrap().to_string_lossy().into_owned()),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl Sources for Fixed {
        fn uuid(&self, _: &Connection) -> Result<String> {
            Ok("00000000-0000-0000-0000-000000000001".into())
        }
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
    }
    fn encode(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                sort_keys: true,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn masked(v: &Value, kind: &str) -> Value {
        let layout = matches!(
            kind,
            "ui.reports.export_csv" | "ui.reports.export_austrian_e1kv_csv"
        );
        match v {
            Value::Dict(fields) => Value::Dict(
                fields
                    .iter()
                    .map(|(k, v)| {
                        let key = match k {
                            kassiber_pyjson::Key::Str(k) => k.as_str().unwrap_or_default(),
                            _ => "",
                        };
                        let hide = matches!(key, "content_sha256" | "bytes")
                            || (layout
                                && matches!(
                                    key,
                                    "sections" | "sheets" | "files" | "rows" | "file" | "filename"
                                ));
                        (
                            k.clone(),
                            if hide { s("<design>") } else { masked(v, kind) },
                        )
                    })
                    .collect(),
            ),
            Value::List(items) => Value::List(items.iter().map(|v| masked(v, kind)).collect()),
            other => other.clone(),
        }
    }
    fn files(root: &Path, at: &Path, out: &mut Vec<(String, Value)>) {
        if !at.exists() {
            return;
        }
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    s(String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned()),
                ));
            }
        }
    }
    #[test]
    fn managed_csv_export_requests_match_python() {
        let cases =
            kassiber_pytext::loads(include_str!("../tests/data/export_requests.json")).unwrap();
        for case in artifact_csv::list(&cases) {
            let root = tempfile::tempdir().unwrap();
            let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
            let conn = book.connection();
            conn.execute_batch(include_str!("../tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(&text(case, "sql")).unwrap();
            let result = execute(
                conn,
                &text(case, "kind"),
                value(case, "args"),
                &root.path().join("exports"),
                &Fixed,
            );
            let actual = match result {
                Ok(v) => dict([("ok", v)]),
                Err(e) => dict([(
                    "error",
                    dict([
                        ("code", s(e.code)),
                        ("message", s(e.message)),
                        ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                        ("details", e.details.unwrap_or(Value::Null)),
                        ("retryable", Value::Bool(e.retryable)),
                    ]),
                )]),
            };
            // The CSVs are the shared design's machine format: their bytes,
            // hashes, file names and layout counts differ from Python's.
            let kind = text(case, "kind");
            assert_eq!(
                encode(&masked(&actual, &kind)).replace(root.path().to_str().unwrap(), "<root>"),
                encode(&masked(value(case, "expected"), &kind)),
                "{}",
                text(case, "name")
            );
            let Value::Dict(tables) = value(case, "tables") else {
                panic!()
            };
            for (table, expected) in tables {
                let kassiber_pyjson::Key::Str(table) = table else {
                    panic!()
                };
                let actual = Value::List(
                    rows(
                        conn,
                        &format!("SELECT * FROM {} ORDER BY 1,2,3", table.as_str().unwrap()),
                        [],
                    )
                    .unwrap(),
                );
                assert_eq!(
                    encode(&masked(&actual, &kind)),
                    encode(&masked(expected, &kind)),
                    "{}: {table:?}",
                    text(case, "name")
                );
            }
            let mut actual = Vec::new();
            files(root.path(), &root.path().join("exports"), &mut actual);
            let expected = match value(case, "files") {
                Value::Dict(files) => files.len(),
                _ => 0,
            };
            if kind != "ui.reports.export_austrian_e1kv_csv" {
                assert_eq!(actual.len(), expected, "{} files", text(case, "name"));
            } else {
                assert_eq!(
                    actual.is_empty(),
                    expected == 0,
                    "{} files",
                    text(case, "name")
                );
            }
            if matches!(
                kind.as_str(),
                "ui.reports.export_capital_gains_csv" | "ui.transactions.export_csv"
            ) {
                for (name, content) in &actual {
                    same_values(
                        &py_str(content),
                        &text(value(case, "files"), name),
                        &format!("{} {name}", text(case, "name")),
                    );
                }
            }
        }
    }

    /// RFC 4180 records; quoted cells may hold line breaks.
    fn records(text: &str) -> Vec<Vec<String>> {
        let mut records = Vec::new();
        let mut record = Vec::new();
        let mut cell = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    cell.push('"');
                    chars.next();
                }
                ('"', _) => quoted = !quoted,
                (',', false) => record.push(std::mem::take(&mut cell)),
                ('\r', false) => (),
                ('\n', false) => {
                    record.push(std::mem::take(&mut cell));
                    records.push(std::mem::take(&mut record));
                }
                _ => cell.push(c),
            }
        }
        records
    }

    /// A machine-format CSV against Python's file: the same rows, and every
    /// column Python has (`amount` for `amount_btc`) equal by value, so the
    /// exact decimals match Python's floats.
    fn same_values(actual: &str, expected: &str, name: &str) {
        // Python's transaction file heads its columns with labels
        // (`Occurred At`); they name the same keys.
        let old: Vec<Vec<String>> = records(expected)
            .into_iter()
            .map(|r| {
                if r.first().is_some_and(|c| c == "Occurred At") {
                    r.iter()
                        .map(|c| c.to_lowercase().replace(' ', "_"))
                        .collect()
                } else {
                    r
                }
            })
            .collect();
        let start = old
            .iter()
            .position(|r| r.first().map(String::as_str) == Some("occurred_at"))
            .unwrap_or_else(|| panic!("{name}: no header in Python's file"));
        let old_rows: Vec<&Vec<String>> = old[start + 1..]
            .iter()
            .take_while(|r| !(r.len() == 1 && r[0].is_empty()))
            // Python writes a placeholder row where the new file has none.
            .filter(|r| !(r.len() == 1 && r[0] == "No rows in scope."))
            .collect();
        let new = records(actual);
        assert_eq!(new.len() - 1, old_rows.len(), "{name}: rows");
        for (row, before) in new[1..].iter().zip(old_rows) {
            for (column, value) in new[0].iter().zip(row) {
                let source = column.trim_end_matches("_btc");
                let Some(index) = old[start].iter().position(|h| h == source) else {
                    continue;
                };
                let was = &before[index];
                let equal = value == was
                    || matches!(
                        (value.parse::<f64>(), was.parse::<f64>()),
                        (Ok(a), Ok(b)) if a == b
                    );
                assert!(equal, "{name}: {column} {value:?} != {was:?}");
            }
        }
    }

    #[test]
    fn managed_csv_paths_do_not_overwrite_existing_actions() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        let exports = root.path().join("exports");
        let args = dict::<&str>([]);
        let first = execute(conn, "ui.transactions.export_csv", &args, &exports, &Fixed).unwrap();
        let first_path = PathBuf::from(text(&first, "file"));
        let bytes = std::fs::read(&first_path).unwrap();
        let reserved = exports.join("reports/kassiber-transactions-20260102-030405-2.csv");
        std::fs::create_dir(&reserved).unwrap();
        let second = execute(conn, "ui.transactions.export_csv", &args, &exports, &Fixed).unwrap();
        assert_eq!(
            text(&second, "filename"),
            "kassiber-transactions-20260102-030405-3.csv"
        );
        assert_eq!(std::fs::read(first_path).unwrap(), bytes);
        assert!(reserved.is_dir());
    }
}
