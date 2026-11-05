//! Shared CSV artifact preparation for native export transports.
//! Preparation holds one read snapshot and never publishes files or writes rows.
use super::*;

#[derive(Debug)]
pub struct File {
    pub name: String,
    pub sheet: String,
    pub bytes: Vec<u8>,
    pub rows: usize,
}
#[derive(Debug)]
pub struct Prepared {
    pub files: Vec<File>,
    pub metadata: Value,
    /// The saved marker publication must register, if any.
    pub completion: Option<crate::export_completion::Completion>,
}

pub(super) fn history_limit(raw: &Value) -> Result<Value> {
    let parsed = match raw {
        Value::Null => return Ok(Value::Null),
        Value::Int(n) => n.clone(),
        Value::Bool(v) => i64::from(*v).into(),
        Value::Float(v) if v.is_finite() => kassiber_pytext::parse_int(&format!("{:.0}",v.trunc())).map_err(|_| AppError::new("internal_error","invalid report history limit"))?,
        Value::Float(v) => return Err(AppError::new("internal_error",if v.is_nan(){"cannot convert float NaN to integer"}else{"cannot convert float infinity to integer"})),
        Value::Str(raw) => kassiber_pytext::parse_int(raw.as_str().unwrap_or_default()).map_err(|error| {
            AppError::new("internal_error",match error {
                kassiber_pytext::NumberError::TooManyDigits(n) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
                _ => format!("invalid literal for int() with base 10: {}",kassiber_pytext::repr_py(raw)),
            })
        })?,
        _ => return Err(AppError::new("internal_error",format!("int() argument must be a string, a bytes-like object or a real number, not '{}'",kassiber_envelope::type_name(raw)))),
    };
    if parsed < 0.into() {
        return Err(AppError::new(
            "validation",
            "--history-limit must be zero or positive",
        ));
    }
    Ok(Value::Int(parsed))
}

/// Prepare the complete artifact before the caller starts publication.
pub fn prepare(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn kassiber_setup::Sources,
) -> Result<Prepared> {
    conn.execute_batch("SAVEPOINT report_artifact_read")
        .map_err(sql_error)?;
    let result = (|| {
        let mut args = args.clone();
        if matches!(command, "reports.export-csv" | "reports.export-pdf") {
            let limit = history_limit(value(&args, "history_limit"))?;
            replace(&mut args, "history_limit", limit);
        }
        if command == "reports.export-platform-csv" {
            platform_csv::platform(value(&args, "platform"))?;
        }
        // A saved source-of-funds case is admitted by its own rules before
        // the book scope, without the journal freshness reports need.
        if command == "reports.export-source-funds-pdf" {
            return source_funds_pdf::prepare(conn, &args, sources);
        }
        let (workspace, profile) = kassiber_setup::references::scope(conn, &args)?;
        if command == "reports.export-transactions-csv" {
            return standalone_csv::prepare(conn, &profile, &args);
        }
        if command == "reports.export-platform-csv" {
            return platform_csv::prepare(conn, &profile, &args);
        }
        if command == "reports.export-exit-tax-pdf" {
            return exit_pdf::prepare(conn, &args, sources);
        }
        if command == "reports.export-summary-pdf" {
            return report_design::prepare_summary(conn, &args, sources);
        }
        require_context(conn, &profile, false)?;
        match command {
            "reports.export-capital-gains-csv" => capital_csv::prepare(conn, &profile, &args),
            "reports.export-csv" => full_csv::prepare(conn, &workspace, &profile, &args, sources),
            "reports.export-pdf" => full_pdf::prepare(conn, &workspace, &profile, &args, sources),
            "reports.export-austrian-e1kv-pdf" => {
                austrian_pdf::prepare(conn, &workspace, &profile, &args, sources)
            }
            "reports.export-austrian-e1kv-csv" => {
                austrian_csv::prepare(conn, &workspace, &profile, &args)
            }
            _ => Err(AppError::new(
                "not_implemented",
                format!("{command} is not implemented in the Rust backend yet"),
            )),
        }
    })();
    let release = conn
        .execute_batch("ROLLBACK TO report_artifact_read; RELEASE report_artifact_read")
        .map_err(sql_error);
    match result {
        Ok(v) => {
            release?;
            Ok(v)
        }
        Err(e) => Err(e),
    }
}

pub(super) fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(v) => v,
        _ => &[],
    }
}
pub(super) fn cell(v: &Value) -> String {
    if matches!(v, Value::Null) {
        String::new()
    } else {
        py_str(v)
    }
}
/// Python csv.writer's excel dialect, including CR/LF and a lone empty cell.
pub(super) fn encode(rows: &[Vec<Value>]) -> Vec<u8> {
    let mut out = String::new();
    for row in rows {
        for (i, v) in row.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let raw = cell(v);
            if raw.contains([',', '"', '\r', '\n']) || (row.len() == 1 && raw.is_empty()) {
                out.push('"');
                out.push_str(&raw.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(&raw);
            }
        }
        out.push_str("\r\n");
    }
    out.into_bytes()
}

pub(super) fn label(header: &str) -> String {
    header
        .split('_')
        .map(|part| match part {
            "id" => "ID".into(),
            "msat" => "msat".into(),
            "sat" => "sat".into(),
            "pnl" => "PnL".into(),
            "fmv" => "FMV".into(),
            _ => {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().to_string() + chars.as_str())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) struct Section {
    pub name: &'static str,
    pub title: String,
    pub headers: &'static [&'static str],
    pub rows: Vec<Value>,
}
pub(super) fn document(title: &str, sections: &[Section]) -> Vec<u8> {
    let mut rows = vec![vec![s(title)], vec![]];
    for section in sections {
        rows.push(vec![s(section.title.as_str())]);
        rows.push(section.headers.iter().map(|h| s(label(h))).collect());
        if section.rows.is_empty() {
            rows.push(vec![s("No rows in scope.")]);
        } else {
            rows.extend(section.rows.iter().map(|row| {
                section
                    .headers
                    .iter()
                    .map(|h| value(row, h).clone())
                    .collect()
            }));
        }
        rows.push(vec![]);
    }
    encode(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artifact_csv_excel_dialect() {
        let rows = vec![
            vec![],
            vec![s("")],
            vec![Value::Null],
            vec![s("one,two"), s("\"quote\""), s("CR\rLF\n")],
            vec![
                Value::Bool(true),
                Value::Float(1e-7),
                Value::Int(1001.into()),
                Value::Null,
            ],
        ];
        assert_eq!(encode(&rows),b"\r\n\"\"\r\n\"\"\r\n\"one,two\",\"\"\"quote\"\"\",\"CR\rLF\n\"\r\nTrue,1e-07,1001,\r\n");
        assert_eq!(
            encode(&[vec![s("only\rCR")], vec![s("only\nLF")]]),
            b"\"only\rCR\"\r\n\"only\nLF\"\r\n"
        );
    }
}
