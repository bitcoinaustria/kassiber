use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_wallets::file_import::document;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

pub(crate) fn required() -> AppError {
    AppError::new("interaction_required", "Document OCR import requires an interactive review")
        .with_hint("Run wallets preview-document first, then rerun wallets import-document in a terminal and explicitly select the reviewed rows.")
}

fn cancelled() -> AppError {
    AppError::new(
        "document_import_review_cancelled",
        "Document import review was cancelled",
    )
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn display(value: &Value, fallback: &str) -> String {
    if truthy(value) {
        review_str(value)
    } else {
        fallback.into()
    }
}

fn review_str(value: &Value) -> String {
    match value {
        Value::Str(text) => text
            .code_points()
            .map(|point| {
                char::from_u32(point).map_or_else(|| format!("\\u{point:04x}"), |c| c.to_string())
            })
            .collect(),
        _ => py_str(value),
    }
}

fn io_error(_: std::io::Error) -> AppError {
    AppError::new("internal_error", "Could not write document review")
}

fn digit(c: char) -> bool {
    kassiber_pytext::decimal_value(c as u32).is_some()
        || matches!(c as u32, 0xb2..=0xb3 | 0xb9 | 0x1369..=0x1371 | 0x19da
            | 0x2070 | 0x2074..=0x2079 | 0x2080..=0x2089 | 0x2460..=0x2468
            | 0x2474..=0x247c | 0x2488..=0x2490 | 0x24ea | 0x24f5..=0x24fd
            | 0x24ff | 0x2776..=0x277e | 0x2780..=0x2788 | 0x278a..=0x2792
            | 0x10a40..=0x10a43 | 0x10e60..=0x10e68 | 0x11052..=0x1105a
            | 0x1f100..=0x1f10a)
}

pub(crate) fn select(
    draft: &Value,
    currency: &str,
    include_quarantined: bool,
    interactive: bool,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<Vec<String>, AppError> {
    if !interactive {
        return Err(required());
    }
    let (Value::Dict(_), Value::List(rows)) = (field(draft, "source"), field(draft, "rows")) else {
        return Err(AppError::new("validation", "Document OCR draft is invalid"));
    };
    let hash = display(field(field(draft, "source"), "sha256"), "");
    let threshold = document::threshold(field(draft, "confidence_threshold"))?;
    let mut selectable = BTreeMap::new();
    writeln!(
        stderr,
        "OCR accounting draft (nothing is selected by default):"
    )
    .map_err(io_error)?;
    for (offset, row) in rows.iter().enumerate() {
        if !matches!(row, Value::Dict(_)) {
            continue;
        }
        let index = offset + 1;
        let record = field(row, "record");
        let permitted = document::reviewed_record(row, &hash, currency).is_some()
            && (document::confidence_flags(row, &threshold).is_empty() || include_quarantined);
        if permitted {
            selectable.insert(index, display(field(row, "id"), ""));
        }
        let region = field(row, "source_region");
        let page = if matches!(region, Value::Dict(_)) {
            py_str(field(region, "page"))
        } else {
            "—".into()
        };
        let confidence = match field(row, "confidence") {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(v.to_string().parse::<f64>().unwrap_or(f64::INFINITY)),
            Value::Bool(v) => Some(f64::from(*v)),
            _ => None,
        };
        let confidence = confidence.map_or_else(
            || "—".into(),
            |v| {
                if v.is_nan() {
                    "nan%".into()
                } else {
                    format!("{:.0}%", v * 100.0)
                }
            },
        );
        let cells = field(row, "cell_confidences");
        let cells = if truthy(cells) {
            cells.clone()
        } else {
            dict::<&str>([])
        };
        let cells =
            kassiber_pyjson::dumps(&cells, &kassiber_pyjson::Options::new().sort_keys(true))
                .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        let flags = match field(row, "flags") {
            Value::List(v) => v.iter().map(py_str).collect::<Vec<_>>().join(","),
            _ => String::new(),
        };
        writeln!(stderr, "[{index}] {} | {} | confidence {confidence} | page {page}\n    date={} | side={} | asset={} | amount={} | fee={}{} | fiat={} {} | rate={}\n    counterparty={} | description={}\n    cell_confidences={cells} | flags={}",
            if permitted { "selectable" } else { "blocked" }, get(row, "status").map_or_else(|| "unknown".into(), py_str),
            display(field(record, "occurred_at"), "—"), display(field(record, "direction"), "—"), display(field(record, "asset"), "—"),
            display(field(record, "amount_btc"), "—"), display(field(record, "fee_btc"), "—"),
            if truthy(field(record, "fee_defaulted")) { " (not provided; records 0)" } else { "" },
            display(field(record, "fiat_value"), "—"), display(field(record, "fiat_currency"), ""), display(field(record, "fiat_rate"), "—"),
            display(field(record, "counterparty"), "—"), display(field(record, "description"), "—"), if flags.is_empty() { "none" } else { &flags }
        ).map_err(io_error)?;
    }
    stderr.flush().map_err(io_error)?;
    if selectable.is_empty() {
        return Err(AppError::new(
            "document_import_no_ready_rows",
            "Document draft has no structurally valid rows to review",
        ));
    }
    let mut terminal = kassiber_secrets::terminal::ReviewInput::install()
        .map_err(|_| AppError::new("internal_error", "Could not prepare document review input"))?;
    write!(
        stdout,
        "Enter comma-separated row numbers to import, or press Enter to cancel: "
    )
    .map_err(io_error)?;
    stdout.flush().map_err(io_error)?;
    let mut response = Vec::new();
    loop {
        let mut byte = [0];
        let read = match terminal.as_mut() {
            Some(terminal) => terminal.read(stdin, &mut byte),
            None => stdin.read(&mut byte),
        };
        match read {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => response.push(byte[0]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => return Err(cancelled()),
            Err(_) => {
                return Err(AppError::new(
                    "internal_error",
                    "Could not read document review",
                ))
            }
        }
    }
    let response = String::from_utf8(response)
        .map_err(|_| AppError::new("validation", "Document row selection must be valid UTF-8"))?;
    let response = kassiber_pytext::strip(&response);
    if response.is_empty() {
        return Err(cancelled());
    }
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for token in response.split(',') {
        let token = kassiber_pytext::strip(token);
        if token.is_empty() || !token.chars().all(digit) {
            return Err(AppError::new(
                "validation",
                "Document row selection must contain comma-separated row numbers",
            )
            .with_details(dict([("selection", s(response))])));
        }
        let index = kassiber_pytext::parse_int(token).map_err(|e| {
            AppError::new("internal_error", match e {
                kassiber_pytext::NumberError::Invalid => format!("invalid literal for int() with base 10: {}", kassiber_pytext::repr_str(token)),
                kassiber_pytext::NumberError::TooManyDigits(n) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
            })
        })?;
        let id = index
            .to_string()
            .parse::<usize>()
            .ok()
            .and_then(|i| selectable.get(&i));
        let Some(id) = id else {
            return Err(AppError::new(
                "validation",
                format!("Document row {index} is not selectable"),
            )
            .with_details(dict([
                ("selection", s(response)),
                (
                    "selectable_rows",
                    Value::List(selectable.keys().map(|i| Value::from(*i as i64)).collect()),
                ),
            ])));
        };
        if seen.insert(index) {
            selected.push(id.clone());
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests;
