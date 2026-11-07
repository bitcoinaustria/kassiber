use super::*;
use artifact_csv::list;
use kassiber_xlsx::{Cell, Document};
fn s(raw: impl AsRef<str>) -> Value {
    super::s(raw.as_ref())
}

fn numeric_cell(value: &Value) -> Result<Cell> {
    let raw = py_str(value);
    let non_finite = raw.trim_start_matches(['+', '-']).starts_with("NaN")
        || raw.parse::<f64>().is_ok_and(|v| !v.is_finite());
    if non_finite {
        return Err(AppError::new(
            "internal_error",
            "NAN/INF not supported in write_number() without 'nan_inf_to_errors' Workbook() option",
        ));
    }
    artifact_xlsx::cell(&float(&decimal(value)?)?)
}
/// The cross-check workbook in the shared design. Python's writer refuses a
/// non-finite figure, so this refuses it the same way first.
pub(super) fn document(
    report: &Value,
    created: String,
    lang: report_design::Lang,
) -> Result<Document> {
    for total in list(value(report, "totals")) {
        for key in ["tool", "kassiber", "delta"] {
            if value(total, key) != &s("") {
                numeric_cell(value(total, key))?;
            }
        }
    }
    for row in list(value(report, "rows")) {
        for (side, keys) in [
            ("tool", ["amount", "fee", "value"]),
            ("kassiber", ["amount", "fee", "value"]),
        ] {
            for key in keys {
                let v = value(value(row, side), key);
                if !matches!(v, Value::Null) && v != &s("") {
                    numeric_cell(v)?;
                }
            }
        }
    }
    Ok(report_design::xlsx_document(&report_design::cross_check(
        report, &created, lang,
    )))
}
pub(super) fn publish(
    report: &Value,
    path: &std::path::Path,
    created: String,
    lang: report_design::Lang,
) -> Result<Value> {
    export::create_directory(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new(".")),
    )?;
    let bytes = kassiber_xlsx::render(&document(report, created, lang)?)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    export::write_synced(path, &bytes).map_err(|e| export::io_error(e, Some(path)))?;
    let absolute = path
        .canonicalize()
        .map_err(|e| export::io_error(e, Some(path)))?;
    Ok(dict([
        ("file", s(absolute.to_string_lossy())),
        ("bytes", Value::Int(bytes.len().into())),
        ("tool", value(report, "tool").clone()),
        ("year", value(report, "year").clone()),
        ("counts", value(report, "counts").clone()),
        ("tax_differences", value(report, "tax_differences").clone()),
        ("agrees", value(report, "agrees").clone()),
    ]))
}
