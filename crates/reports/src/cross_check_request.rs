//! Upload validation and export transport without a renderer filesystem grant.
use super::*;
use std::path::Path;

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}
fn optional(args: &Value, key: &str) -> Result<Option<String>> {
    match value(args, key) {
        Value::Null => Ok(None),
        Value::Str(v) => {
            let text = py_str(&Value::Str(v.clone()));
            let text = kassiber_pytext::strip(&text);
            Ok((!text.is_empty()).then(|| text.to_owned()))
        }
        v => Err(validation(format!("{key} must be a string"))
            .with_details(dict([("type", s(kassiber_envelope::type_name(v)))]))),
    }
}
fn decode(raw: &str) -> Option<Vec<u8>> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::new();
    let mut bits = 0_u32;
    let mut count = 0;
    let mut padding = false;
    let mut pads = 0;
    for c in raw.bytes() {
        if c == b'=' {
            if count == 0 {
                return None;
            }
            padding = true;
            pads += 1;
            continue;
        }
        if padding {
            return None;
        }
        let value = alphabet.iter().position(|v| *v == c)?;
        bits = (bits << 6) | value as u32;
        count += 1;
        if count % 4 == 0 {
            bytes.extend_from_slice(&[(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
            bits = 0;
        }
    }
    match count % 4 {
        0 => (),
        2 if pads >= 2 => bytes.push((bits >> 4) as u8),
        3 if pads >= 1 => bytes.extend_from_slice(&[(bits >> 10) as u8, (bits >> 2) as u8]),
        _ => return None,
    }
    if count % 4 != 0 && pads != 4 - count % 4 {
        return None;
    }
    Some(bytes)
}
fn upload(kind: &str, args: &Value) -> Result<(Value, Vec<u8>, String)> {
    let allowed = [
        "tool",
        "filename",
        "source_bytes_base64",
        "wallet",
        "wallets",
        "year",
        "tool_sources",
        "include_tool_wallets",
    ];
    let fields = match args {
        Value::Dict(fields) => fields,
        _ => {
            return Err(validation("daemon args must be an object")
                .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])))
        }
    };
    let mut unknown = fields
        .iter()
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) if !allowed.iter().any(|a| k.as_str() == Some(a)) => {
                Some(Value::Str(k.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    unknown.sort_by_key(|v| {
        if let Value::Str(s) = v {
            s.clone()
        } else {
            unreachable!()
        }
    });
    if !unknown.is_empty() {
        return Err(validation(format!("{kind} received unsupported arguments"))
            .with_details(dict([("unsupported", Value::List(unknown))])));
    }
    if !matches!(value(args, "tool"), Value::Str(_)) {
        return Err(validation(format!("{kind} requires a tool")));
    }
    if !matches!(value(args, "year"), Value::Null | Value::Int(_)) {
        return Err(validation(format!("{kind} year must be an integer")));
    }
    if !matches!(value(args, "tool_sources"), Value::Null)
        && !matches!(value(args,"tool_sources"),Value::List(v) if v.iter().all(|v|matches!(v,Value::Str(_))))
    {
        return Err(validation(format!(
            "{kind} tool_sources must be an array of strings"
        )));
    }
    let encoded = optional(args, "source_bytes_base64")?;
    let filename = optional(args, "filename")?.unwrap_or_else(|| "export.csv".into());
    let filename = Path::new(&filename)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let encoded = encoded.filter(|s| !s.is_empty()).ok_or_else(|| {
        validation("source_bytes_base64 is required")
            .with_hint("Choose the tool's export with the file picker first.")
    })?;
    if Path::new(&filename)
        .extension()
        .is_none_or(|s| s.to_string_lossy().to_lowercase() != "csv")
    {
        return Err(validation("Choose the tool's CSV export")
            .with_hint("CoinTracking and Blockpit both export their transactions as CSV."));
    }
    let bytes = decode(&encoded).ok_or_else(|| {
        validation("Could not decode the selected file.")
            .with_hint("Choose the file again and retry.")
    })?;
    let wallets = super::export_request::wallet_args(kind, args)?;
    if !matches!(
        get(args, "include_tool_wallets"),
        None | Some(Value::Bool(_))
    ) {
        return Err(
            validation("include_tool_wallets must be a boolean").with_details(dict([(
                "type",
                s(kassiber_envelope::type_name(value(
                    args,
                    "include_tool_wallets",
                ))),
            )])),
        );
    }
    let mut normalized = dict(
        ["tool", "year", "tool_sources", "include_tool_wallets"]
            .map(|k| (k, value(args, k).clone())),
    );
    replace(&mut normalized, "wallets", wallets);
    Ok((normalized, bytes, filename))
}
/// The book's tax country picks the workbook's language.
fn language(conn: &Connection, args: &Value) -> Result<report_design::Lang> {
    let (_, profile) = kassiber_setup::references::scope(conn, args)?;
    Ok(report_design::Lang::for_country(&text(
        &profile,
        "tax_country",
    )))
}

pub(super) fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    exports_root: &Path,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Value> {
    if command == "reports.cross-check" {
        let report = cross_check::read_cli(conn, args)?;
        if truthy(value(args, "xlsx")) {
            return cross_check_xlsx::publish(
                &report,
                Path::new(&text(args, "xlsx")),
                sources.now(conn)?,
                language(conn, args)?,
            );
        }
        return Ok(report);
    }
    let empty = dict::<&str>([]);
    let args = if matches!(args, Value::Null) {
        &empty
    } else {
        args
    };
    let (args, bytes, filename) = upload(command, args)?;
    if command == "ui.reports.cross_check" {
        return cross_check::report(conn, &args, &bytes, &filename);
    }
    let tool = text(&args, "tool");
    let stem = if truthy(value(&args, "year")) {
        format!("kassiber-vs-{tool}-{}", text(&args, "year"))
    } else {
        format!("kassiber-vs-{tool}")
    };
    let path = super::export_request::managed(exports_root, &stem, ".xlsx", &sources.now(conn)?)?;
    let report = cross_check::report(conn, &args, &bytes, &filename)?;
    let mut result =
        cross_check_xlsx::publish(&report, &path, sources.now(conn)?, language(conn, &args)?)?;
    replace(&mut result, "format", s("xlsx"));
    replace(&mut result, "scope", s("cross_check"));
    replace(
        &mut result,
        "filename",
        s(path.file_name().unwrap().to_string_lossy().into_owned()),
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_check_publication_preserves_bytes_and_diagnostic_history() {
        struct Fixed;
        impl kassiber_setup::Sources for Fixed {
            fn now(&self, _: &Connection) -> Result<String> {
                Ok("2026-01-02T03:04:05Z".into())
            }
            fn uuid(&self, _: &Connection) -> Result<String> {
                unreachable!()
            }
        }
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch("UPDATE profiles SET journal_input_version=99;")
            .unwrap();
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/data/cross_check.json")).unwrap();
        let source = root.path().join("export.csv");
        std::fs::write(&source, cases[0]["csv"].as_str().unwrap()).unwrap();
        let target = root.path().join("nested/report.xlsx");
        let mut args = dict([
            ("tool", s("cointracking")),
            ("file", s(source.to_string_lossy().into_owned())),
            ("xlsx", s(target.to_string_lossy().into_owned())),
        ]);
        let before = conn.total_changes();
        assert!(export_request::handles("reports.cross-check"));
        assert!(export_request::handles("ui.reports.export_cross_check"));
        let result = execute(conn, "reports.cross-check", &args, root.path(), &Fixed).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(value(&result, "bytes"), &Value::Int(bytes.len().into()));
        assert_eq!(
            text(&result, "file"),
            target.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(value(&result, "tool"), &s("cointracking"));
        assert!(get(&result, "report_snapshot").is_none());
        assert_eq!(conn.total_changes(), before);
        replace(&mut args, "xlsx", Value::Null);
        let report = execute(conn, "reports.cross-check", &args, root.path(), &Fixed).unwrap();
        assert!(artifact_csv::list(value(&report, "insights"))
            .iter()
            .any(|r| text(r, "code") == "journals_stale"));
        assert_eq!(value(&result, "counts"), value(&report, "counts"));
        assert_eq!(conn.total_changes(), before);
    }
    #[test]
    fn cross_check_upload_validates_bytes_and_scope_before_reads() {
        let valid = dict([
            ("tool", s("cointracking")),
            ("filename", s("../history.CSV")),
            ("source_bytes_base64", s("YSwgYg0K")),
            ("wallets", Value::List(vec![s("cold")])),
            ("year", Value::Int(2024.into())),
        ]);
        let (args, bytes, name) = upload("ui.reports.cross_check", &valid).unwrap();
        assert_eq!(bytes, b"a, b\r\n");
        assert_eq!(name, "history.CSV");
        assert_eq!(value(&args, "wallets"), &Value::List(vec![s("cold")]));
        assert!(get(&args, "file").is_none());
        for (key, v, message) in [
            (
                "file",
                s("/must/not/read.csv"),
                "ui.reports.cross_check received unsupported arguments",
            ),
            (
                "year",
                Value::Bool(true),
                "ui.reports.cross_check year must be an integer",
            ),
            (
                "tool_sources",
                Value::List(vec![Value::Int(1.into())]),
                "ui.reports.cross_check tool_sources must be an array of strings",
            ),
            (
                "filename",
                s("history.xlsx"),
                "Choose the tool's CSV export",
            ),
            (
                "source_bytes_base64",
                s("YQ="),
                "Could not decode the selected file.",
            ),
            (
                "include_tool_wallets",
                Value::Null,
                "include_tool_wallets must be a boolean",
            ),
        ] {
            let mut input = valid.clone();
            replace(&mut input, key, v);
            assert_eq!(
                upload("ui.reports.cross_check", &input)
                    .err()
                    .unwrap()
                    .message,
                message
            );
        }
        for (raw, expected) in [
            ("", Some(vec![])),
            ("YQ==", Some(b"a".to_vec())),
            ("YWI=", Some(b"ab".to_vec())),
            ("YWJj", Some(b"abc".to_vec())),
            ("YWJj====", Some(b"abc".to_vec())),
            ("====", None),
            ("=", None),
            ("YQ===", None),
            ("YWI==", None),
            ("YQ=", None),
            ("a===", None),
            ("YWJj\n", None),
            ("YQ==x", None),
        ] {
            assert_eq!(decode(raw), expected, "{raw}");
        }
    }
}
