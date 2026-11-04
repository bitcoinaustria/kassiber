//! `core.file_analysis.analyze_file`: what an export is and whether the
//! generic ledger importer can read it, before anything is imported.
//!
//! Tabular files are previewed through the real per-row normalizer; no model
//! output ever becomes an amount, a date or an asset. Photos and PDFs are
//! only routed to the local document import.
use super::ledger::{self, py, ColumnMap};
use super::*;
use kassiber_envelope::get;

pub const TABULAR_EXTENSIONS: [&str; 5] = [".csv", ".tsv", ".txt", ".xlsx", ".xlsm"];
pub const DOCUMENT_EXTENSIONS: [&str; 6] = [".jpg", ".jpeg", ".png", ".webp", ".gif", ".pdf"];
const MAX_ANALYZE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_HEADER_COLUMNS: usize = 256;
pub const DEFAULT_SAMPLE_ROWS: usize = 5;
pub const MAX_SAMPLE_ROWS: usize = 25;
const COLUMN_FIELDS: [&str; 21] = [
    "date",
    "type",
    "direction",
    "received",
    "sent",
    "amount",
    "received_asset",
    "sent_asset",
    "asset",
    "fee",
    "fee_asset",
    "fiat_currency",
    "fiat_value",
    "fiat_rate",
    "note",
    "txid",
    "counterparty",
    "payment_hash",
    "payment_hash_source",
    "swap_refund_funding_txid",
    "swap_refund_funding_vout",
];
const HINT_FIELDS: [&str; 4] = [
    "received_header_asset",
    "sent_header_asset",
    "amount_header_asset",
    "fee_header_asset",
];
const ASSET_STATING_FIELDS: [&str; 6] = [
    "asset",
    "received_asset",
    "sent_asset",
    "amount_header_asset",
    "received_header_asset",
    "sent_header_asset",
];

/// Every extension `analyze_file` accepts, sorted as Python reports them.
pub fn supported_extensions() -> Vec<&'static str> {
    let mut all: Vec<_> = TABULAR_EXTENSIONS
        .iter()
        .chain(&DOCUMENT_EXTENSIONS)
        .copied()
        .collect();
    all.sort_unstable();
    all
}

/// `os.path.splitext(path)[1]`: leading dots of the name are not a suffix.
pub fn splitext(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let body = name.trim_start_matches('.');
    match body.rfind('.') {
        Some(at) => &body[at..],
        None => "",
    }
}

/// `_analyze_path`.
fn analyze_path(raw: &str) -> Result<(std::path::PathBuf, u64), AppError> {
    let raw = kassiber_pytext::strip(raw);
    if raw.is_empty() {
        return Err(AppError::new("validation", "A file path is required"));
    }
    let candidate = ledger::map_path(raw)?;
    let candidate = std::path::absolute(&candidate)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let shown = candidate.to_string_lossy().into_owned();
    if !candidate.exists() {
        return Err(
            AppError::new("not_found", format!("File not found: {shown}"))
                .with_hint("Check the path."),
        );
    }
    if !candidate.is_file() {
        return Err(AppError::new(
            "validation",
            "Analysis source must be a file",
        ));
    }
    let size = std::fs::metadata(&candidate)
        .map_err(|e| AppError::new("internal_error", e.to_string()))?
        .len();
    if size > MAX_ANALYZE_BYTES {
        return Err(AppError::new("validation", "File is too large to analyze")
            .with_hint("Split the export, or import it in per-year files.")
            .with_details(dict([
                ("size_bytes", Py::from(size as i64)),
                ("max_bytes", Py::from(MAX_ANALYZE_BYTES as i64)),
            ])));
    }
    Ok((candidate, size))
}

/// `_delimiter_name`: the sniffed delimiter of a delimited file's first 8192
/// characters, or `None`.
fn delimiter_name(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .ok()?
        .take(8192 * 4 + 3)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let sample: String = text.chars().take(8192).collect();
    if sample.is_empty() {
        return None;
    }
    super::platform_csv::sniffed_delimiter(&sample).map(String::from)
}

fn file_kind(path: &str) -> &'static str {
    let extension = splitext(path).to_lowercase();
    if [".xlsx", ".xlsm"].contains(&extension.as_str()) {
        "spreadsheet"
    } else if TABULAR_EXTENSIONS.contains(&extension.as_str()) {
        "delimited"
    } else if extension == ".pdf" {
        "pdf"
    } else if DOCUMENT_EXTENSIONS.contains(&extension.as_str()) {
        "image"
    } else {
        "unsupported"
    }
}

fn field<'a>(value: &'a Py, key: &str) -> &'a Py {
    get(value, key).unwrap_or(&Py::Null)
}

fn truthy(value: &Py) -> bool {
    kassiber_envelope::truthy(value)
}

/// A JSON plan in `infer_ledger_columns` / `normalize_column_map` key order.
fn ordered_plan(plan: &Json, keys: impl IntoIterator<Item = &'static str>) -> Py {
    Py::Dict(
        keys.into_iter()
            .map(|key| (kassiber_pyjson::Key::from(key), py(&plan[key])))
            .collect(),
    )
}

/// `normalize_column_map(raw, headers)` in Python's key order: explicit asset
/// hints, the type map, mapped columns, then header-derived hints.
fn applied_plan(mapping: &ColumnMap, headers: &[String]) -> Result<Py, AppError> {
    let plan = ledger::validate_map(mapping, headers)?;
    let raw = mapping.raw.as_object().expect("validated column map");
    let mut keys: Vec<&'static str> = HINT_FIELDS
        .iter()
        .copied()
        .filter(|k| raw.contains_key(*k))
        .collect();
    let mut ordered = vec![];
    for key in &keys {
        ordered.push((kassiber_pyjson::Key::from(*key), py(&plan[*key])));
    }
    if raw.contains_key("type_map") {
        ordered.push((
            kassiber_pyjson::Key::from("type_map"),
            ledger::ordered_type_map(mapping, &plan["type_map"]),
        ));
    }
    keys = COLUMN_FIELDS
        .iter()
        .copied()
        .filter(|k| raw.contains_key(*k))
        .collect();
    keys.extend(
        HINT_FIELDS
            .iter()
            .copied()
            .filter(|k| !raw.contains_key(*k)),
    );
    for key in keys {
        ordered.push((kassiber_pyjson::Key::from(key), py(&plan[key])));
    }
    Ok(Py::Dict(ordered))
}

/// `_implausible_as_btc`: `Decimal(str(amount_max)) >= 1000`. The preview
/// writes `amount_max` as a plain fixed-point string, or omits it.
fn implausible_as_btc(amount_max: &Py) -> bool {
    let Py::Str(text) = amount_max else {
        return false;
    };
    let text =
        kassiber_pytext::strip(&kassiber_envelope::py_str(&Py::Str(text.clone()))).to_owned();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(&text)),
    };
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return false;
    }
    let significant = whole.trim_start_matches('0');
    !negative && significant.len() >= 4
}

fn step(action: &str, reason: &str) -> Py {
    dict([("action", s(action)), ("reason", s(reason))])
}

/// `_next_step`.
fn next_step(
    native: bool,
    confident: bool,
    preview: &Py,
    kinds_from_sign: bool,
    units_unconfirmed: bool,
) -> Py {
    if truthy(field(preview, "header_is_preamble")) {
        return step("strip_preamble", "The first row of this file is a preamble (account holder, IBAN, date range), not a header — it is narrower than the rows below it. Kassiber read it as the column names, so no mapping can work. Delete the lines above the real header row, or re-export without them.");
    }
    if !confident {
        return step("supply_column_map", "The columns in this file were not recognized. Map them with column_map (at least a date column and one amount column), or fill in the blank ledger template instead.");
    }
    if !truthy(field(preview, "mapped")) {
        return step(
            "fix_rows",
            "The columns were recognized but no row could be normalized.",
        );
    }
    if units_unconfirmed {
        return step("confirm_amount_units", "Nothing in this file says what the amounts are denominated in, and they are too large to be BTC — this export is probably in satoshis. Importing as-is would be off by 1e8 with nothing rejected. Ask the user which rail it is, then map an asset column or pass amount_header_asset with --column-map.");
    }
    if kinds_from_sign {
        return step("map_row_kinds", "No Type or direction column was recognized, so every row would import as a plain transfer (Deposit/Withdrawal) chosen by the amount's sign — a sale would book as a deposit. Map the file's own Type column with column_map.type (plus type_map for its vocabulary), or confirm the file really is transfers only.");
    }
    if truthy(field(preview, "errors")) {
        return step("review_problems", "Some rows would be rejected. Import the rest, or correct the mapping and preview again.");
    }
    step(
        "import",
        if native {
            "This is the native ledger template; import it as generic_ledger."
        } else {
            "Every row normalized. Import with source_format=generic_ledger, passing the same column_map."
        },
    )
}

/// `analyze_file(source_file, column_map=..., sample_rows=...)`.
pub fn analyze(
    source_file: &str,
    mapping: Option<&ColumnMap>,
    sample_rows: usize,
) -> Result<Py, AppError> {
    let (path, size) = analyze_path(source_file)?;
    let shown = path.to_string_lossy().into_owned();
    let kind = file_kind(&shown);
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let source = dict([
        ("path", s(shown.as_str())),
        ("filename", s(filename.as_str())),
        ("kind", s(kind)),
        ("size_bytes", Py::from(size as i64)),
        (
            "delimiter",
            if kind == "delimited" {
                delimiter_name(&path).map_or(Py::Null, s)
            } else {
                Py::Null
            },
        ),
    ]);
    if ["pdf", "image"].contains(&kind) {
        return Ok(dict([
            ("source", source),
            ("route", s("document_import")),
            ("supported", Py::Bool(true)),
            (
                "next_step",
                step("document_import_preview", "Photos and PDFs are read by a local vision model, which never leaves this device. Preview the draft rows, then import the ones you confirm."),
            ),
        ]));
    }
    if kind == "unsupported" {
        let extension = splitext(&filename);
        return Err(AppError::new(
            "validation",
            format!(
                "Cannot analyze '{}' files",
                if extension.is_empty() {
                    "(no extension)"
                } else {
                    extension
                }
            ),
        )
        .with_hint("Export the history as CSV or XLSX, or supply a PDF/photo statement.")
        .with_details(dict([(
            "supported_extensions",
            Py::List(supported_extensions().into_iter().map(s).collect()),
        )])));
    }
    let bound = sample_rows.min(MAX_SAMPLE_ROWS);
    let preview = ledger::preview(&shown, mapping, bound)?;
    let headers: Vec<String> = match field(&preview, "headers") {
        Py::List(items) => items
            .iter()
            .take(MAX_HEADER_COLUMNS)
            .map(kassiber_envelope::py_str)
            .collect(),
        _ => vec![],
    };
    let native = truthy(field(&preview, "template"));
    let plan = mapping
        .map(|mapping| applied_plan(mapping, &headers))
        .transpose()?;
    let (inferred, detected) = ledger::infer_columns(&headers);
    let named = |key: &str| inferred[key].as_str().is_some_and(|v| !v.is_empty());
    let inferred_confident =
        named("date") && ["received", "sent", "amount"].iter().any(|k| named(k));
    let inferred_plan = ordered_plan(&inferred, COLUMN_FIELDS.into_iter().chain(HINT_FIELDS));
    let confident = get(&preview, "confident").is_none_or(truthy);
    let effective = match &plan {
        Some(plan) if truthy(plan) => plan.clone(),
        _ => inferred_plan.clone(),
    };
    let kinds_from_sign =
        !native && !(truthy(field(&effective, "type")) || truthy(field(&effective, "direction")));
    let units_unconfirmed = !native
        && !ASSET_STATING_FIELDS
            .iter()
            .any(|key| truthy(field(&effective, key)))
        && implausible_as_btc(field(&preview, "amount_max"));
    let step = next_step(
        native,
        confident,
        &preview,
        kinds_from_sign,
        units_unconfirmed,
    );
    let count = |key: &str| get(&preview, key).cloned().unwrap_or(Py::from(0));
    Ok(dict([
        ("source", source),
        ("route", s("generic_ledger")),
        ("supported", Py::Bool(true)),
        ("template", Py::Bool(native)),
        ("headers", Py::List(headers.into_iter().map(s).collect())),
        ("inferred_plan", inferred_plan),
        ("inferred_confident", Py::Bool(inferred_confident)),
        ("detected", py(&detected)),
        ("applied_plan", plan.unwrap_or(Py::Null)),
        ("confident", Py::Bool(confident)),
        ("rows_read", count("rows_read")),
        ("mapped", count("mapped")),
        ("errors", count("errors")),
        (
            "problems",
            get(&preview, "problems")
                .cloned()
                .unwrap_or(Py::List(vec![])),
        ),
        (
            "preview",
            get(&preview, "preview")
                .cloned()
                .unwrap_or(Py::List(vec![])),
        ),
        ("truncated", Py::Bool(truthy(field(&preview, "truncated")))),
        ("row_kinds_from_amount_sign", Py::Bool(kinds_from_sign)),
        ("amount_units_unconfirmed", Py::Bool(units_unconfirmed)),
        (
            "header_is_preamble",
            Py::Bool(truthy(field(&preview, "header_is_preamble"))),
        ),
        ("next_step", step),
    ]))
}

/// `_analyze_file_payload` for uploaded bytes: analyzed from a private
/// temporary copy with the upload's extension, removed afterwards.
pub fn analyze_upload(
    bytes: &[u8],
    extension: &str,
    mapping: Option<&ColumnMap>,
    sample_rows: usize,
) -> Result<Py, AppError> {
    let temp = super::document::files::Temp::new("analyze-file")?;
    let path = temp.path().join(format!("upload{extension}"));
    std::fs::write(&path, bytes).map_err(super::document::files::io)?;
    analyze(&path.to_string_lossy(), mapping, sample_rows)
}
