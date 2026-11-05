//! Desktop report transports; all computation lives in kassiber-reports.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const KINDS: &[&str] = &[
    "ui.transactions.export_csv",
    "ui.transactions.export_xlsx",
    "ui.transactions.export_platform_csv",
    "ui.privacy_hygiene.snapshot",
    "ui.reports.capital_gains",
    "ui.reports.explain_capital_gain",
    "ui.reports.summary",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
    "ui.reports.tax_summary",
    "ui.reports.balance_history",
    "ui.reports.privacy_hygiene",
    "ui.reports.psbt_privacy",
    "ui.reports.exit_tax_preview",
    "ui.reports.export_pdf",
    "ui.reports.export_summary_pdf",
    "ui.reports.export_csv",
    "ui.reports.export_xlsx",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.reports.export_austrian_e1kv_csv",
    "ui.reports.export_exit_tax_pdf",
    "ui.reports.export_exit_tax_xlsx",
    "ui.reports.export_year_handoff",
    "ui.reports.export_cross_check",
    "ui.source_funds.preview",
    "ui.source_funds.review_context",
    "ui.source_funds.request_input",
    "ui.source_funds.cases.save",
    "ui.source_funds.cases.list",
    "ui.source_funds.sources.list",
    "ui.source_funds.sources.create",
    "ui.source_funds.sources.attach",
    "ui.source_funds.links.list",
    "ui.source_funds.links.create",
    "ui.source_funds.links.review",
    "ui.source_funds.links.bulk_review",
    "ui.source_funds.links.attach",
    "ui.source_funds.suggest",
    "ui.source_funds.assemble",
    "ui.source_funds.evidence.list",
    "ui.source_funds.export_pdf",
    "ui.source_funds.export_bundle",
    "ui.source_funds.coverage",
    "ui.source_funds.recipients.list",
    "ui.source_funds.recipients.create",
    "ui.source_funds.recipients.update",
    "ui.source_funds.recipients.delete",
    "ui.reports.lightning_profitability",
    "ui.exports.preview",
    "ui.reports.cross_check",
];
pub fn execute(book: &mut dyn Book, kind: &str, raw: Option<&Value>) -> Result<Value, AppError> {
    if kind == "ui.privacy_hygiene.snapshot" {
        return book.reports(kind, raw.unwrap_or(&Value::Null));
    }
    if kind == "ui.exports.preview" {
        let empty = dict::<&str>([]);
        return book.reports(
            kind,
            raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty),
        );
    }
    if kind == "ui.reports.psbt_privacy" {
        return book.reports(kind, &psbt_args(raw)?);
    }
    if kind.starts_with("ui.source_funds.") {
        return book.reports(kind, &cross_check_args(raw)?);
    }
    if matches!(
        kind,
        "ui.reports.privacy_hygiene" | "ui.reports.privacy_mirror"
    ) {
        return book.reports(kind, &privacy_args(kind, raw)?);
    }
    if matches!(
        kind,
        "ui.reports.cross_check"
            | "ui.reports.export_cross_check"
            | "ui.reports.lightning_profitability"
    ) {
        return book.reports(kind, &cross_check_args(raw)?);
    }
    if matches!(
        kind,
        "ui.reports.capital_gains" | "ui.reports.explain_capital_gain"
    ) {
        return book.reports(kind, &gain_args(kind, raw)?);
    }
    if matches!(
        kind,
        "ui.reports.export_csv"
            | "ui.reports.export_xlsx"
            | "ui.reports.export_pdf"
            | "ui.reports.export_summary_pdf"
            | "ui.reports.export_exit_tax_pdf"
            | "ui.reports.export_exit_tax_xlsx"
            | "ui.reports.exit_tax_preview"
            | "ui.reports.export_capital_gains_csv"
            | "ui.reports.export_austrian_e1kv_csv"
            | "ui.reports.export_austrian_e1kv_xlsx"
            | "ui.reports.export_austrian_e1kv_pdf"
            | "ui.transactions.export_csv"
            | "ui.transactions.export_xlsx"
            | "ui.reports.export_year_handoff"
            | "ui.transactions.export_platform_csv"
    ) {
        let empty = dict::<&str>([]);
        let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
        if !matches!(args, Value::Dict(_)) {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(args)))])));
        }
        if kind == "ui.reports.exit_tax_preview" {
            return book.reports(
                kind,
                &dict([
                    (
                        "departure_date",
                        get(args, "departure_date").cloned().unwrap_or(Value::Null),
                    ),
                    (
                        "destination",
                        get(args, "destination").cloned().unwrap_or(Value::Null),
                    ),
                ]),
            );
        }
        return book.reports(kind, args);
    }
    if kind == "ui.reports.balance_history" {
        return book.reports(kind, &history_args(raw)?);
    }
    if !matches!(kind, "ui.reports.summary" | "ui.reports.tax_summary") {
        return book.reports(kind, &dict::<&str>([]));
    }
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let mut unknown: Vec<_> = fields
        .iter()
        .filter(|(k, _)| {
            *k != if kind == "ui.reports.tax_summary" {
                "year"
            } else {
                "wallet"
            }
            .into()
        })
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) => Some(k.clone()),
            _ => None, // JSON object keys are strings.
        })
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("{kind} received unsupported arguments"),
        )
        .with_details(dict([(
            "unknown",
            Value::List(unknown.into_iter().map(Value::Str).collect()),
        )])));
    }
    if kind == "ui.reports.tax_summary" {
        return book.reports(kind, &tax_summary_args(args)?);
    }
    let wallet = match get(args, "wallet") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Str(t))
            if t.as_str()
                .is_some_and(|t| !kassiber_pytext::strip(t).is_empty()) =>
        {
            s(kassiber_pytext::strip(t.as_str().unwrap()))
        }
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.reports.summary wallet must be a non-empty string",
            ))
        }
    };
    book.reports(kind, &dict([("wallet", wallet)]))
}
fn cross_check_args(raw: Option<&Value>) -> Result<Value, AppError> {
    let args = raw
        .filter(|v| !matches!(v, Value::Null))
        .cloned()
        .unwrap_or_else(|| dict::<&str>([]));
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(&args)))])));
    }
    Ok(args)
}

fn gain_args(kind: &str, raw: Option<&Value>) -> Result<Value, AppError> {
    if kind == "ui.reports.explain_capital_gain" {
        return Ok(dict([("raw_args", raw.cloned().unwrap_or(Value::Null))]));
    }
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    Ok(dict([(
        "year",
        get(args, "year")
            .or_else(|| get(args, "tax_year"))
            .cloned()
            .unwrap_or(Value::Null),
    )]))
}

fn psbt_args(raw: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let mut unknown = fields
        .iter()
        .filter(|(key, _)| *key != "psbt".into())
        .map(|(key, _)| match key {
            kassiber_pyjson::Key::Str(key) => Value::Str(key.clone()),
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    unknown.sort_by(|a, b| match (a, b) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => unreachable!(),
    });
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.reports.psbt_privacy received unsupported arguments",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let psbt = get(args, "psbt")
        .filter(|value| match value {
            Value::Str(text) => text
                .as_str()
                .is_none_or(|text| !kassiber_pytext::strip(text).is_empty()),
            _ => false,
        })
        .ok_or_else(|| {
            AppError::new("validation", "ui.reports.psbt_privacy requires psbt text")
                .with_details(dict([("required", Value::List(vec![s("psbt")]))]))
        })?;
    Ok(dict([("psbt", psbt.clone())]))
}

fn privacy_args(kind: &str, raw: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    if !fields.is_empty() {
        let mut unknown: Vec<Value> = fields
            .iter()
            .map(|(k, _)| match k {
                kassiber_pyjson::Key::Str(k) => Value::Str(k.clone()),
                _ => unreachable!(),
            })
            .collect();
        unknown.sort_by(|a, b| match (a, b) {
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            _ => unreachable!(),
        });
        return Err(AppError::new(
            "validation",
            format!("{kind} received unsupported arguments"),
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    Ok(empty)
}

fn history_args(raw: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let allowed = [
        "interval", "start", "end", "wallet", "account", "asset", "limit",
    ];
    let mut unknown: Vec<_> = fields
        .iter()
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) if !allowed.iter().any(|a| k == &(*a).into()) => {
                Some(Value::Str(k.clone()))
            }
            _ => None,
        })
        .collect();
    unknown.sort_by(|a, b| match (a, b) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => unreachable!(), // JSON object keys are strings.
    });
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.reports.balance_history received unsupported arguments",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let interval = get(args, "interval").cloned().unwrap_or_else(|| s("month"));
    if !["hour", "day", "week", "month"]
        .iter()
        .any(|i| interval == s(*i))
    {
        return Err(AppError::new(
            "validation",
            "ui.reports.balance_history interval is unsupported",
        )
        .with_details(dict([
            ("interval", interval),
            (
                "supported",
                Value::List(["hour", "day", "week", "month"].map(s).into()),
            ),
        ])));
    }
    let raw_limit = get(args, "limit")
        .cloned()
        .unwrap_or(Value::Int(120.into()));
    let limit = match &raw_limit {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(i64::from(*b).into()),
        Value::Str(t) => t.as_str().and_then(|s| kassiber_pytext::parse_int(s).ok()),
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc())).ok()
        }
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        _ => None,
    }
    .ok_or_else(|| {
        AppError::new(
            "validation",
            "ui.reports.balance_history limit must be an integer",
        )
        .with_details(dict([("value", raw_limit.clone())]))
    })?;
    if limit < 1.into() {
        return Err(AppError::new(
            "validation",
            "ui.reports.balance_history limit must be positive",
        )
        .with_details(dict([("value", raw_limit)])));
    }
    let limit = limit.min(500.into());
    let mut normalized = dict(allowed.map(|k| (k, get(args, k).cloned().unwrap_or(Value::Null))));
    if let Value::Dict(ref mut fields) = normalized {
        for (key, v) in fields {
            if key == &"limit".into() {
                *v = Value::Int(limit.clone());
            }
            if key == &"interval".into() {
                *v = interval.clone();
            }
        }
    }
    Ok(normalized)
}

fn tax_summary_args(args: &Value) -> Result<Value, AppError> {
    let raw = get(args, "year").unwrap_or(&Value::Null);
    let parsed = match raw {
        Value::Null => Some(Value::Null),
        Value::Int(_) => Some(raw.clone()),
        Value::Bool(v) => Some(Value::Int(i64::from(*v).into())),
        Value::Str(v) => v
            .as_str()
            .and_then(|s| kassiber_pytext::parse_int(s).ok())
            .map(Value::Int),
        Value::Float(v) if v.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", v.trunc()))
                .ok()
                .map(Value::Int)
        }
        Value::Float(v) if v.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        _ => None,
    };
    let year = parsed.ok_or_else(|| {
        AppError::new(
            "validation",
            "ui.reports.tax_summary year must be an integer",
        )
        .with_details(dict([("year", raw.clone())]))
    })?;
    Ok(dict([("year", year)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct SourceFundsCapture;
    impl Book for SourceFundsCapture {
        fn reports(&mut self, command: &str, args: &Value) -> Result<Value, AppError> {
            Ok(dict([("command", s(command)), ("args", args.clone())]))
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
    fn source_funds_internal_diagnostics_preserve_native_debug_presence_and_redaction() {
        let id = crate::output::RequestId::Value(s("cursor"));
        let error = AppError::new("internal_error", "invalid literal for int()");
        for kind in KINDS
            .iter()
            .filter(|kind| kind.starts_with("ui.source_funds."))
        {
            let envelope = crate::app_error_envelope(&error, &id, Some(kind));
            let fields = get(&envelope, "error").unwrap();
            assert_eq!(get(fields, "debug"), Some(&s("")));
            assert_eq!(get(fields, "code"), Some(&s("internal_error")));
            assert_eq!(get(fields, "message"), Some(&s(error.message.as_str())));
            assert_eq!(get(&envelope, "request_id"), Some(&s("cursor")));
            let typed = AppError::new("validation", "cursor is invalid");
            let envelope = crate::app_error_envelope(&typed, &id, Some(kind));
            assert_eq!(
                get(get(&envelope, "error").unwrap(), "debug"),
                Some(&Value::Null)
            );
        }
        for kind in [
            None,
            Some("ui.reports.summary"),
            Some("ui.source_funds.unregistered"),
        ] {
            let envelope = crate::app_error_envelope(&error, &id, kind);
            assert_eq!(
                get(get(&envelope, "error").unwrap(), "debug"),
                Some(&Value::Null)
            );
        }
        let secret = AppError::new("internal_error", "Bearer planted-secret-value");
        let envelope =
            crate::app_error_envelope(&secret, &id, Some("ui.source_funds.evidence.list"));
        assert!(!kassiber_pyjson::dumps(&envelope, &Default::default())
            .unwrap()
            .contains("planted-secret-value"));
    }

    #[test]
    fn source_funds_batch_daemon_preserves_bounded_selection() {
        let kind = "ui.source_funds.links.bulk_review";
        assert!(KINDS.contains(&kind));
        let args = dict([
            ("target_transaction_ref", s(" out ")),
            ("link_ids", Value::List(vec![s(" l-z ")])),
        ]);
        let result = execute(&mut SourceFundsCapture, kind, Some(&args)).unwrap();
        assert_eq!(get(&result, "command"), Some(&s(kind)));
        assert_eq!(get(&result, "args"), Some(&args));
        let empty = dict::<&str>([]);
        assert_eq!(
            get(
                &execute(&mut SourceFundsCapture, kind, None).unwrap(),
                "args"
            ),
            Some(&empty)
        );
        assert_eq!(
            execute(&mut SourceFundsCapture, kind, Some(&Value::List(vec![])))
                .unwrap_err()
                .code,
            "validation"
        );
    }
    #[test]
    fn source_funds_link_daemon_preserves_arguments_and_registers_mutations() {
        let args = dict([
            ("link", s(" l-z ")),
            ("attachment_id", s(" a-a ")),
            ("allocation_amount", s("0.00000100001")),
            ("from_allocation_amount", s("0.00000100002")),
            ("expected_allocation_amount_msat", Value::Int(100001.into())),
            (
                "expected_from_allocation_amount_msat",
                Value::Int(100002.into()),
            ),
            ("uses_chain_observation", Value::Bool(false)),
            ("chain_data_confirmed", Value::Bool(false)),
        ]);
        for kind in [
            "ui.source_funds.links.create",
            "ui.source_funds.links.review",
            "ui.source_funds.links.attach",
        ] {
            assert!(KINDS.contains(&kind));
            let actual = execute(&mut SourceFundsCapture, kind, Some(&args)).unwrap();
            assert_eq!(get(&actual, "command"), Some(&s(kind)));
            assert_eq!(get(&actual, "args"), Some(&args));
            assert_eq!(
                execute(&mut SourceFundsCapture, kind, Some(&Value::Bool(false)))
                    .unwrap_err()
                    .code,
                "validation"
            );
        }
    }
    #[test]
    fn source_funds_recipient_daemon_preserves_arguments_and_registers_mutations() {
        let args = dict([
            ("recipient", s(" r-a ")),
            ("label", s("Renamed")),
            ("kind", s("bank")),
            ("default_reveal_mode", s("full")),
            ("notes", s("")),
        ]);
        for kind in [
            "ui.source_funds.recipients.create",
            "ui.source_funds.recipients.update",
            "ui.source_funds.recipients.delete",
        ] {
            assert!(KINDS.contains(&kind));
            let actual = execute(&mut SourceFundsCapture, kind, Some(&args)).unwrap();
            assert_eq!(get(&actual, "command"), Some(&s(kind)));
            assert_eq!(get(&actual, "args"), Some(&args));
            assert_eq!(
                execute(&mut SourceFundsCapture, kind, Some(&Value::Bool(false)))
                    .unwrap_err()
                    .code,
                "validation"
            );
        }
    }
    #[test]
    fn source_funds_stored_daemon_preserves_arguments_and_registers_only_lists() {
        let args = dict([
            ("target_transaction", s(" external-in ")),
            ("state", s(" accepted ")),
            ("include_inactive", Value::Bool(true)),
            ("cursor", s("1")),
            ("limit", Value::Int(2.into())),
        ]);
        for kind in [
            "ui.source_funds.sources.list",
            "ui.source_funds.links.list",
            "ui.source_funds.cases.list",
            "ui.source_funds.recipients.list",
            "ui.source_funds.evidence.list",
            "ui.source_funds.preview",
            "ui.source_funds.review_context",
            "ui.source_funds.request_input",
            "ui.source_funds.coverage",
            "ui.source_funds.suggest",
            "ui.source_funds.assemble",
        ] {
            assert!(KINDS.contains(&kind));
            assert_eq!(cross_check_args(Some(&args)).unwrap(), args);
            let actual = execute(&mut SourceFundsCapture, kind, Some(&args)).unwrap();
            assert_eq!(get(&actual, "command"), Some(&s(kind)));
            assert_eq!(get(&actual, "args"), Some(&args));
        }
        assert!(KINDS.contains(&"ui.source_funds.export_pdf"));
        assert!(KINDS.contains(&"ui.source_funds.export_bundle"));
        assert_eq!(cross_check_args(None).unwrap(), dict::<&str>([]));
        assert_eq!(
            cross_check_args(Some(&Value::Null)).unwrap(),
            dict::<&str>([])
        );
        let error = cross_check_args(Some(&Value::Bool(false))).unwrap_err();
        assert_eq!(error.message, "daemon args must be an object");
        assert_eq!(error.details, Some(dict([("type", s("bool"))])));
    }

    #[test]
    fn source_funds_case_save_daemon_preserves_recipe_fingerprint_and_registration() {
        let args = dict([
            ("target_transaction", s(" in ")),
            ("expected_review_fingerprint", s("0".repeat(64))),
            ("case_label", s("Reviewed Ω")),
            ("recipient", s("r-a")),
            ("target_amount", s("0.1")),
            (
                "report_options",
                dict([("mask_recipient", Value::Bool(true))]),
            ),
        ]);
        assert!(KINDS.contains(&"ui.source_funds.cases.save"));
        let actual = execute(
            &mut SourceFundsCapture,
            "ui.source_funds.cases.save",
            Some(&args),
        )
        .unwrap();
        assert_eq!(
            get(&actual, "command"),
            Some(&s("ui.source_funds.cases.save"))
        );
        assert_eq!(get(&actual, "args"), Some(&args));
        assert_eq!(
            execute(
                &mut SourceFundsCapture,
                "ui.source_funds.cases.save",
                Some(&Value::Bool(false))
            )
            .unwrap_err()
            .code,
            "validation"
        );
    }

    #[test]
    fn source_funds_source_daemon_preserves_mutation_arguments_and_admission() {
        let args = dict([
            ("source_type", s("income")),
            ("label", s("Origin")),
            ("amount", s("1.00000000001")),
            ("asset", s("LBTC")),
            ("fiat_value", s("12.34")),
            ("fiat_currency", s("EUR")),
            ("acquired_at", s("2024-01-01")),
            ("description", s("Evidence")),
            ("attachment_ids", Value::List(vec![s("a-a"), s("a-z")])),
            ("source", s("s-z")),
            ("attachment_id", s("a-a")),
        ]);
        for kind in [
            "ui.source_funds.sources.create",
            "ui.source_funds.sources.attach",
        ] {
            assert!(KINDS.contains(&kind));
            let actual = execute(&mut SourceFundsCapture, kind, Some(&args)).unwrap();
            assert_eq!(get(&actual, "command"), Some(&s(kind)));
            assert_eq!(get(&actual, "args"), Some(&args));
            assert_eq!(
                execute(&mut SourceFundsCapture, kind, Some(&Value::List(vec![])))
                    .unwrap_err()
                    .code,
                "validation"
            );
        }
    }
    #[test]
    fn cross_check_daemon_preserves_uploaded_bytes_and_registration() {
        let args = kassiber_pytext::loads(r#"{"tool":"cointracking","filename":"history.csv","source_bytes_base64":"YQ==","wallets":["cold"],"year":2024,"tool_sources":["Exchange"],"include_tool_wallets":true}"#).unwrap();
        for kind in ["ui.reports.cross_check", "ui.reports.export_cross_check"] {
            assert!(KINDS.contains(&kind));
            assert_eq!(cross_check_args(Some(&args)).unwrap(), args);
        }
        assert_eq!(cross_check_args(None).unwrap(), dict::<&str>([]));
        assert_eq!(
            cross_check_args(Some(&Value::Null)).unwrap(),
            dict::<&str>([])
        );
        let error = cross_check_args(Some(&Value::List(vec![]))).unwrap_err();
        assert_eq!(error.message, "daemon args must be an object");
        assert_eq!(error.details, Some(dict([("type", s("list"))])));
    }
    #[test]
    fn gain_daemon_arguments_preserve_reference_year_alias_and_context() {
        let args = kassiber_pytext::loads(
            r#"{"year":null,"tax_year":2024,"workspace":"ignored","reference":{"entry_id":"z"}}"#,
        )
        .unwrap();
        assert_eq!(
            gain_args("ui.reports.capital_gains", Some(&args)).unwrap(),
            dict([("year", Value::Null)])
        );
        let alias = dict([("tax_year", s("2024"))]);
        assert_eq!(
            gain_args("ui.reports.capital_gains", Some(&alias)).unwrap(),
            dict([("year", s("2024"))])
        );
        assert_eq!(
            get(
                &gain_args("ui.reports.explain_capital_gain", Some(&args)).unwrap(),
                "raw_args"
            ),
            Some(&args)
        );
        for kind in [
            "ui.reports.capital_gains",
            "ui.reports.explain_capital_gain",
        ] {
            assert!(KINDS.contains(&kind));
            if kind == "ui.reports.capital_gains" {
                assert_eq!(
                    gain_args(kind, Some(&Value::List(vec![])))
                        .unwrap_err()
                        .code,
                    "validation"
                );
            }
            assert!(gain_args(kind, None).is_ok());
        }
    }
    #[test]
    fn history_daemon_arguments() {
        for (raw, expected) in [
            ("true", 1),
            ("1", 1),
            ("2.9", 2),
            ("500", 500),
            ("501", 500),
            ("999999999999999999999999999999", 500),
            ("\" ５０１ \"", 500),
        ] {
            let input = dict([("limit", kassiber_pytext::loads(raw).unwrap())]);
            let parsed = history_args(Some(&input)).unwrap();
            assert_eq!(get(&parsed, "limit"), Some(&Value::Int(expected.into())));
            assert_eq!(get(&parsed, "interval"), Some(&s("month")));
        }
        for raw in ["0", "false", "-1", "null", "[]", "{}", "\"bad\"", "NaN"] {
            let input = dict([("limit", kassiber_pytext::loads(raw).unwrap())]);
            assert_eq!(history_args(Some(&input)).unwrap_err().code, "validation");
        }
        for raw in ["null", "[]", "false", "\"bad\""] {
            let input = dict([("interval", kassiber_pytext::loads(raw).unwrap())]);
            assert_eq!(
                history_args(Some(&input)).unwrap_err().message,
                "ui.reports.balance_history interval is unsupported"
            );
        }
        let unknown = dict([
            ("z", Value::Null),
            ("a", Value::Null),
            ("limit", Value::Null),
        ]);
        assert_eq!(
            get(
                history_args(Some(&unknown))
                    .unwrap_err()
                    .details
                    .as_ref()
                    .unwrap(),
                "unknown"
            ),
            Some(&Value::List(vec![s("a"), s("z")]))
        );
        assert!(history_args(Some(&Value::List(vec![]))).is_err());
        let defaults = history_args(None).unwrap();
        assert_eq!(get(&defaults, "limit"), Some(&Value::Int(120.into())));
        assert_eq!(get(&defaults, "interval"), Some(&s("month")));
        let unicode = kassiber_pytext::loads(r#"{"\ue000":null,"z":null,"\ud800":null}"#).unwrap();
        let expected = kassiber_pytext::loads(r#"["z","\ud800","\ue000"]"#).unwrap();
        assert_eq!(
            get(
                history_args(Some(&unicode))
                    .unwrap_err()
                    .details
                    .as_ref()
                    .unwrap(),
                "unknown"
            ),
            Some(&expected)
        );
    }
    #[test]
    fn tax_summary_daemon_arguments() {
        for (raw, expected) in [
            ("null", "null"),
            ("true", "1"),
            ("false", "0"),
            ("2024", "2024"),
            ("2024.9", "2024"),
            ("-2024.9", "-2024"),
            ("\" ２０２４ \"", "2024"),
            ("\"2_024\"", "2024"),
            ("99999999999999999999999999", "99999999999999999999999999"),
        ] {
            let args = dict([("year", kassiber_pytext::loads(raw).unwrap())]);
            let result = tax_summary_args(&args).unwrap();
            assert_eq!(
                get(&result, "year"),
                Some(&kassiber_pytext::loads(expected).unwrap())
            );
        }
        for raw in ["[]", "{}", "\"bad\"", "\"2024.5\"", "NaN"] {
            let args = dict([("year", kassiber_pytext::loads(raw).unwrap())]);
            let error = tax_summary_args(&args).unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                "ui.reports.tax_summary year must be an integer"
            );
            if raw == "NaN" {
                assert!(
                    matches!(get(error.details.as_ref().unwrap(),"year"),Some(Value::Float(n)) if n.is_nan())
                );
            } else {
                assert_eq!(error.details, Some(args));
            }
        }
    }
    #[test]
    fn platform_csv_daemon_kind_is_registered() {
        assert!(KINDS.contains(&"ui.transactions.export_platform_csv"));
        let kinds = crate::supported_kinds();
        let position = |kind| kinds.iter().position(|k| *k == kind).unwrap();
        assert!(
            position("ui.transactions.export_platform_csv")
                < position("ui.transactions.metadata.update")
        );
        assert_eq!(
            kinds
                .iter()
                .copied()
                .filter(|k| k.starts_with("ui.transactions.export_"))
                .collect::<Vec<_>>(),
            [
                "ui.transactions.export_csv",
                "ui.transactions.export_xlsx",
                "ui.transactions.export_platform_csv"
            ]
        );
        assert_eq!(
            kinds
                .iter()
                .copied()
                .filter(|k| k.starts_with("ui.reports."))
                .collect::<Vec<_>>(),
            [
                "ui.reports.capital_gains",
                "ui.reports.explain_capital_gain",
                "ui.reports.summary",
                "ui.reports.balance_sheet",
                "ui.reports.portfolio_summary",
                "ui.reports.tax_summary",
                "ui.reports.balance_history",
                "ui.reports.privacy_hygiene",
                "ui.reports.psbt_privacy",
                "ui.reports.exit_tax_preview",
                "ui.reports.export_pdf",
                "ui.reports.export_summary_pdf",
                "ui.reports.export_csv",
                "ui.reports.export_xlsx",
                "ui.reports.export_capital_gains_csv",
                "ui.reports.export_austrian_e1kv_pdf",
                "ui.reports.export_austrian_e1kv_xlsx",
                "ui.reports.export_austrian_e1kv_csv",
                "ui.reports.export_exit_tax_pdf",
                "ui.reports.export_exit_tax_xlsx",
                "ui.reports.export_year_handoff",
                "ui.reports.export_cross_check",
                "ui.reports.lightning_profitability",
                "ui.reports.cross_check",
            ]
        );
    }

    #[test]
    fn privacy_mirror_daemon_accepts_only_an_empty_object() {
        for raw in [None, Some(Value::Null), Some(dict::<&str>([]))] {
            assert_eq!(
                privacy_args("ui.reports.privacy_mirror", raw.as_ref()).unwrap(),
                dict::<&str>([])
            );
        }
        for raw in [s(""), Value::Bool(false), Value::List(vec![])] {
            assert_eq!(
                privacy_args("ui.reports.privacy_mirror", Some(&raw))
                    .unwrap_err()
                    .message,
                "daemon args must be an object"
            );
        }
        let error = privacy_args(
            "ui.reports.privacy_mirror",
            Some(&dict([
                ("wallet", s("cold")),
                ("redacted", Value::Bool(true)),
            ])),
        )
        .unwrap_err();
        assert_eq!(error.code, "validation");
        assert_eq!(
            error.message,
            "ui.reports.privacy_mirror received unsupported arguments"
        );
        assert_eq!(
            error.details,
            Some(dict([(
                "unknown",
                Value::List(vec![s("redacted"), s("wallet")])
            )]))
        );
    }
    #[test]
    fn privacy_hygiene_daemon_rejects_unknown_fields_and_non_objects() {
        assert_eq!(
            privacy_args("ui.reports.privacy_hygiene", None).unwrap(),
            dict::<&str>([])
        );
        assert_eq!(
            privacy_args("ui.reports.privacy_hygiene", Some(&Value::Null)).unwrap(),
            dict::<&str>([])
        );
        for raw in [
            Value::List(vec![]),
            Value::Bool(false),
            s(""),
            Value::Int(0.into()),
        ] {
            let error = privacy_args("ui.reports.privacy_hygiene", Some(&raw)).unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(error.message, "daemon args must be an object");
            assert_eq!(error.details, Some(dict([("type", s(type_name(&raw)))])));
        }
        let error = privacy_args(
            "ui.reports.privacy_hygiene",
            Some(&dict([("z", Value::Null), ("a", Value::Null)])),
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "ui.reports.privacy_hygiene received unsupported arguments"
        );
        assert_eq!(
            error.details,
            Some(dict([("unknown", Value::List(vec![s("a"), s("z")]))]))
        );
        assert!(KINDS.contains(&"ui.reports.privacy_hygiene"));
        assert!(!KINDS.contains(&"ui.reports.privacy_mirror"));
        assert!(KINDS.contains(&"ui.reports.psbt_privacy"));
    }
    #[test]
    fn lightning_daemon_preserves_alias_scope_window_and_registration() {
        assert!(KINDS.contains(&"ui.reports.lightning_profitability"));
        let args = dict([
            ("label", s("node")),
            ("workspace", s("Main")),
            ("profile", s("Book")),
            ("window_days", s("7")),
        ]);
        assert_eq!(cross_check_args(Some(&args)).unwrap(), args);
        assert_eq!(cross_check_args(None).unwrap(), dict::<&str>([]));
        assert!(cross_check_args(Some(&Value::List(vec![]))).is_err());
    }
}
