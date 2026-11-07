//! `_analyze_attached_file_for_ai` (`kassiber/daemon.py`) and
//! `core.file_analysis.redact_for_egress`: the model never chooses a file,
//! and an off-device provider sees headers, counts and the column plan only.

use std::sync::OnceLock;

use fancy_regex::Regex;
use kassiber_envelope::{get, s, AppError};
use kassiber_pyjson::{Key, PyStr, Value};

use crate::executor::Book;
use crate::pyre;
use crate::ToolError;

/// `importers.LEDGER_ASSET_HINT_FIELDS`: what a column is denominated in.
pub const LEDGER_ASSET_HINT_FIELDS: [&str; 4] = [
    "received_header_asset",
    "sent_header_asset",
    "amount_header_asset",
    "fee_header_asset",
];

/// `WITHHELD_HEADER`.
pub const WITHHELD_HEADER: &str = "[withheld]";

/// `_EGRESS_DROPPED_KEYS`: per-row messages and normalized rows.
const DROPPED_KEYS: [&str; 2] = ["problems", "preview"];
/// `_EGRESS_DROPPED_SOURCE_KEYS`.
const DROPPED_SOURCE_KEYS: [&str; 2] = ["path", "filename"];
/// `_HEADERISH_LONG`.
const HEADERISH_LONG: usize = 64;

fn patterns() -> &'static [Regex; 5] {
    static PATTERNS: OnceLock<[Regex; 5]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // `_HEADERISH_NUMBER` is used with `match`: anchored at the start.
            pyre::compile(r"^[+-]?[\d.,\s']*\d[\d.,\s']*$"),
            pyre::compile(r"\d{4}-\d{2}-\d{2}|\d{1,2}[./]\d{1,2}[./]\d{2,4}"),
            pyre::compile(r"[0-9a-fA-F]{32,}"),
            pyre::compile(r"\b[A-Z]{2}\d{2}[A-Z0-9]{10,}\b"),
            pyre::compile(r"[^@\s]+@[^@\s]+\.[A-Za-z]{2,}"),
        ]
    })
}

/// `_header_is_safe_to_disclose(value)`.
pub fn header_is_safe_to_disclose(value: &PyStr) -> bool {
    let text = pyre::lossy(value);
    let text = kassiber_pytext::strip(&text);
    if text.is_empty() {
        return true;
    }
    if text.chars().count() > HEADERISH_LONG {
        return false;
    }
    !patterns()
        .iter()
        .any(|pattern| pattern.is_match(text).unwrap_or(true))
}

fn str_of(value: &Value) -> PyStr {
    match value {
        Value::Str(text) => text.clone(),
        other => PyStr::from(kassiber_envelope::py_str(other)),
    }
}

/// `redact_headers_for_egress(headers, is_preamble=...)`: a row is a header
/// only as a whole; one data-shaped cell withholds every non-blank cell.
pub fn redact_headers_for_egress(headers: &[Value], is_preamble: bool) -> Vec<Value> {
    let cells: Vec<PyStr> = headers.iter().map(str_of).collect();
    if !is_preamble && cells.iter().all(header_is_safe_to_disclose) {
        return cells.into_iter().map(Value::Str).collect();
    }
    cells
        .into_iter()
        .map(|cell| {
            if kassiber_pytext::strip(&pyre::lossy(&cell)).is_empty() {
                Value::Str(cell)
            } else {
                s(WITHHELD_HEADER)
            }
        })
        .collect()
}

fn named(key: &Key, names: &[&str]) -> bool {
    matches!(key, Key::Str(name) if names.iter().any(|candidate| name.is(candidate)))
}

/// `os.path.splitext(filename)[1].lower() or None`.
fn extension(filename: &str) -> Value {
    let suffix = splitext(filename);
    if suffix.is_empty() {
        Value::Null
    } else {
        s(kassiber_pytext::lower(suffix))
    }
}

/// `posixpath.splitext`: the last dot after the last separator, ignoring
/// leading dots of the basename.
fn splitext(path: &str) -> &str {
    let base_start = path.rfind('/').map_or(0, |index| index + 1);
    let base = &path[base_start..];
    let Some(dot) = base.rfind('.') else {
        return "";
    };
    if base[..dot].chars().all(|c| c == '.') {
        return "";
    }
    &base[dot..]
}

fn safe_column(value: &Value, is_preamble: bool) -> Value {
    match value {
        Value::Str(text) if is_preamble || !header_is_safe_to_disclose(text) => s(WITHHELD_HEADER),
        other => other.clone(),
    }
}

/// `redact_for_egress(analysis)`: column names and counts survive; every cell
/// value, the local path and the file name do not.
pub fn redact_for_egress(analysis: &Value) -> Value {
    let Value::Dict(pairs) = analysis else {
        return analysis.clone();
    };
    let mut safe: Vec<(Key, Value)> = pairs
        .iter()
        .filter(|(key, _)| !named(key, &DROPPED_KEYS))
        .cloned()
        .collect();
    let is_preamble = get(analysis, "header_is_preamble").is_some_and(kassiber_envelope::truthy);
    for (key, value) in safe.iter_mut() {
        if *key == Key::from("source") {
            if let Value::Dict(source) = &*value {
                let filename = match get(&Value::Dict(source.clone()), "filename") {
                    Some(name) if kassiber_envelope::truthy(name) => {
                        kassiber_envelope::py_str(name)
                    }
                    _ => String::new(),
                };
                let mut kept: Vec<(Key, Value)> = source
                    .iter()
                    .filter(|(key, _)| !named(key, &DROPPED_SOURCE_KEYS))
                    .cloned()
                    .collect();
                let suffix = extension(&filename);
                match kept
                    .iter_mut()
                    .find(|(key, _)| *key == Key::from("extension"))
                {
                    Some((_, slot)) => *slot = suffix,
                    None => kept.push((Key::from("extension"), suffix)),
                }
                *value = Value::Dict(kept);
            }
        }
    }
    let mut withheld = None;
    for (key, value) in safe.iter_mut() {
        if *key == Key::from("headers") {
            if let Value::List(headers) = &*value {
                let headers = redact_headers_for_egress(headers, is_preamble);
                withheld = Some(
                    headers
                        .iter()
                        .filter(|header| **header == s(WITHHELD_HEADER))
                        .count(),
                );
                *value = Value::List(headers);
            }
        }
    }
    if let Some(count) = withheld {
        put(&mut safe, "headers_withheld", Value::from(count));
    }
    for (key, value) in safe.iter_mut() {
        if named(key, &["inferred_plan", "applied_plan"]) {
            if let Value::Dict(plan) = &*value {
                *value = Value::Dict(
                    plan.iter()
                        .map(|(field, column)| (field.clone(), safe_column(column, is_preamble)))
                        .collect(),
                );
            }
        }
    }
    for (key, value) in safe.iter_mut() {
        if *key == Key::from("detected") {
            if let Value::List(entries) = &*value {
                *value = Value::List(
                    entries
                        .iter()
                        .map(|entry| match entry {
                            Value::Dict(fields) => {
                                let column = match get(entry, "column") {
                                    Some(column) if kassiber_envelope::truthy(column) => {
                                        str_of(column)
                                    }
                                    _ => PyStr::new(),
                                };
                                let shown = if !is_preamble && header_is_safe_to_disclose(&column) {
                                    get(entry, "column").cloned().unwrap_or(Value::Null)
                                } else {
                                    s(WITHHELD_HEADER)
                                };
                                let mut fields = fields.clone();
                                match fields
                                    .iter_mut()
                                    .find(|(key, _)| *key == Key::from("column"))
                                {
                                    Some((_, slot)) => *slot = shown,
                                    None => fields.push((Key::from("column"), shown)),
                                }
                                Value::Dict(fields)
                            }
                            other => other.clone(),
                        })
                        .collect(),
                );
            }
        }
    }
    if let Some(problems) = get(analysis, "problems") {
        let count = match problems {
            Value::List(items) => items.len(),
            Value::Dict(items) => items.len(),
            _ => 0,
        };
        put(&mut safe, "problem_rows", Value::from(count));
    }
    put(&mut safe, "cell_values_withheld", Value::Bool(true));
    Value::Dict(safe)
}

fn put(pairs: &mut Vec<(Key, Value)>, key: &str, value: Value) {
    match pairs.iter_mut().find(|(name, _)| *name == Key::from(key)) {
        Some((_, slot)) => *slot = value,
        None => pairs.push((Key::from(key), value)),
    }
}

/// `_analyze_attached_file_for_ai(runtime, arguments)`.
pub fn analyze_attached_file(
    arguments: &Value,
    state: &Value,
    book: &mut dyn Book,
) -> Result<Value, ToolError> {
    let source_file = match get(state, "attachment_source_file") {
        Some(file) if kassiber_envelope::truthy(file) => kassiber_envelope::py_str(file),
        _ => {
            return Err(AppError::new("ai_attachment_missing", "No file is attached to this chat")
                .with_hint("Ask the user to attach the export, then analyze it. In the CLI that is `kassiber chat --file <path>`.")
                .into())
        }
    };
    let column_map = match get(arguments, "column_map") {
        None | Some(Value::Null) => None,
        Some(map @ Value::Dict(_)) => Some(map),
        Some(_) => {
            return Err(AppError::new("validation", "column_map must be an object")
                .with_hint("Map ledger fields to this file's column names.")
                .into())
        }
    };
    if let Some(Value::Dict(fields)) = column_map {
        let mut smuggled: Vec<&'static str> = LEDGER_ASSET_HINT_FIELDS
            .iter()
            .copied()
            .filter(|hint| fields.iter().any(|(key, _)| *key == Key::from(*hint)))
            .collect();
        smuggled.sort_unstable();
        if !smuggled.is_empty() {
            return Err(AppError::new("validation", "Asset hints cannot be set from chat")
                .with_hint("Map the columns only. If a numeric column does not say which asset it holds, ask the user which rail it is instead of declaring one.")
                .with_details(kassiber_envelope::dict([(
                    "rejected_fields",
                    Value::List(smuggled.into_iter().map(s).collect()),
                )]))
                .into());
        }
    }
    let mut analysis = book.analyze_file(&source_file, arguments)?;
    if let Some(label) =
        get(state, "attachment_label").filter(|label| kassiber_envelope::truthy(label))
    {
        kassiber_envelope::set(&mut analysis, "user_description", label.clone());
    }
    if !crate::executor::full_access(state) {
        return Ok(redact_for_egress(&analysis));
    }
    // The local path is never useful to the model and is an exfiltration hint.
    let source = match get(&analysis, "source") {
        Some(Value::Dict(fields)) => Value::Dict(
            fields
                .iter()
                .filter(|(key, _)| *key != Key::from("path"))
                .cloned()
                .collect(),
        ),
        // `analysis.get("source", {}).items()`.
        None => Value::Dict(Vec::new()),
        Some(_) => return Err(ToolError::Unexpected),
    };
    kassiber_envelope::set(&mut analysis, "source", source);
    Ok(analysis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitext_matches_posixpath() {
        for (path, suffix) in [
            ("export.CSV", ".CSV"),
            (".hidden", ""),
            ("..csv", ""),
            ("a.b.tsv", ".tsv"),
            ("dir.d/file", ""),
            ("noext", ""),
            ("", ""),
        ] {
            assert_eq!(splitext(path), suffix, "{path}");
        }
    }
}
