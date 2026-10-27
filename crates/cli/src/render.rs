//! `emit` and `emit_error` from `kassiber/envelope.py` and
//! `core/runtime.py`: the four output formats and `--output`.

use std::io::Write;
use std::path::PathBuf;

use kassiber_envelope::{cli_text, json_ready, py_str, type_name, AppError};
use kassiber_pyjson::{dumps, float_repr, Key, Options, Value};

/// `--format`, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Table,
    Json,
    Plain,
    Csv,
}

impl Format {
    pub fn parse(text: &str) -> Option<Format> {
        match text {
            "table" => Some(Format::Table),
            "json" => Some(Format::Json),
            "plain" => Some(Format::Plain),
            "csv" => Some(Format::Csv),
            _ => None,
        }
    }
}

/// Where output goes: stdout, or the `--output` file.
pub struct Sink<'a> {
    pub output: Option<&'a str>,
    pub home: Option<&'a str>,
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
}

/// Expand the current user's home; refuse forms the shell cannot resolve
/// before touching the filesystem, as Path.expanduser does on failure.
fn expand_user(target: &str, home: Option<&str>) -> std::io::Result<PathBuf> {
    let Some(rest) = target.strip_prefix('~') else {
        return Ok(PathBuf::from(target));
    };
    let Some(home) = home.filter(|_| rest.is_empty() || rest.starts_with(std::path::is_separator))
    else {
        return Err(std::io::Error::other("Could not determine home directory."));
    };
    let home = if cfg!(windows) {
        home
    } else {
        home.trim_end_matches('/')
    };
    let expanded = format!("{home}{rest}");
    Ok(PathBuf::from(if expanded.is_empty() {
        if cfg!(windows) {
            "."
        } else {
            "/"
        }
    } else {
        &expanded
    }))
}

#[cfg(test)]
mod path_tests {
    use super::expand_user;
    use std::path::PathBuf;

    #[test]
    fn tilde_forms_expand_or_fail_before_writing() {
        assert_eq!(
            expand_user("plain/path", None).unwrap(),
            PathBuf::from("plain/path")
        );
        assert_eq!(
            expand_user("~/a", Some("/tmp/home")).unwrap(),
            PathBuf::from("/tmp/home/a")
        );
        assert_eq!(
            expand_user("~", Some("/tmp/home")).unwrap(),
            PathBuf::from("/tmp/home")
        );
        for target in ["~", "~/a", "~someone/a"] {
            assert_eq!(
                expand_user(target, None).unwrap_err().to_string(),
                "Could not determine home directory."
            );
        }
        assert!(expand_user("~someone/a", Some("/tmp/home")).is_err());
        #[cfg(unix)]
        for home in ["", "/"] {
            assert_eq!(expand_user("~", Some(home)).unwrap(), PathBuf::from("/"));
        }
        #[cfg(windows)]
        assert_eq!(
            expand_user(r"~\a", Some(r"C:\Users\example")).unwrap(),
            PathBuf::from(r"C:\Users\example\a")
        );
    }
}

impl Sink<'_> {
    /// `write_text(args, text)`: the text and a final newline, to stdout
    /// or to `--output` (creating its parent directories).
    pub fn write_text(&mut self, text: &str) -> std::io::Result<()> {
        let mut text = text.to_owned();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        match self.output.filter(|t| !t.is_empty() && *t != "-") {
            None => {
                self.stdout.write_all(text.as_bytes())?;
                self.stdout.flush()
            }
            Some(target) => {
                let path = expand_user(target, self.home)?;
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, text)
            }
        }
    }
}

/// `format_table_value(value)`.
fn table_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Float(f) => {
            let fixed = format!("{f:.8}");
            if f.is_finite() {
                fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
            } else {
                // `f"{inf:.8f}"` is `inf`, with no zeros to strip.
                float_repr(*f).to_lowercase().replace("infinity", "inf")
            }
        }
        other => py_str(other),
    }
}

/// `json.dumps(json_ready(value), sort_keys=False)`.
fn inline_json(value: &Value) -> String {
    dumps(&json_ready(value.clone()), &Options::new()).unwrap_or_default()
}

/// The rows of a dict or a list of dicts; a list item that is not a dict
/// fails as Python's attribute lookup on it does.
fn rows_of(payload: &Value, method: &str) -> Result<Vec<Vec<(String, Value)>>, AppError> {
    let row = |value: &Value| match value {
        Value::Dict(pairs) => Ok(pairs
            .iter()
            .map(|(key, item)| (key_text(key), item.clone()))
            .collect()),
        other => Err(AppError::new(
            "internal_error",
            format!("'{}' object has no attribute '{method}'", type_name(other)),
        )),
    };
    match payload {
        Value::List(items) => items.iter().map(row).collect(),
        other => Ok(vec![row(other)?]),
    }
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Str(text) => py_str(&Value::Str(text.clone())),
        Key::Int(n) => n.to_string(),
        Key::Float(f) => float_repr(*f),
        Key::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
        Key::Null => "None".to_owned(),
    }
}

fn len(text: &str) -> usize {
    text.chars().count()
}

/// `_table_text(rows)`.
fn table_text(rows: &[Vec<(String, Value)>]) -> String {
    if rows.is_empty() {
        return "(no rows)".to_owned();
    }
    let normalized: Vec<Vec<(String, String)>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|(k, v)| (k.clone(), table_value(v)))
                .collect()
        })
        .collect();
    let headers: Vec<String> = normalized[0].iter().map(|(k, _)| k.clone()).collect();
    let cell = |row: &Vec<(String, String)>, header: &str| {
        row.iter()
            .find(|(k, _)| k == header)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let widths: Vec<usize> = headers
        .iter()
        .map(|header| {
            normalized
                .iter()
                .map(|row| len(&cell(row, header)))
                .chain([len(header)])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let pad =
        |text: &str, width: usize| format!("{text}{}", " ".repeat(width.saturating_sub(len(text))));
    let mut lines = vec![
        headers
            .iter()
            .zip(&widths)
            .map(|(h, &w)| pad(h, w))
            .collect::<Vec<_>>()
            .join("  "),
        widths
            .iter()
            .map(|&w| "-".repeat(w))
            .collect::<Vec<_>>()
            .join("  "),
    ];
    for row in &normalized {
        lines.push(
            headers
                .iter()
                .zip(&widths)
                .map(|(h, &w)| pad(&cell(row, h), w))
                .collect::<Vec<_>>()
                .join("  "),
        );
    }
    lines.join("\n")
}

/// `_plain_dict(payload)`.
fn plain_dict(row: &[(String, Value)]) -> String {
    row.iter()
        .map(|(key, value)| match value {
            Value::Dict(_) | Value::List(_) => format!("{key}: {}", inline_json(value)),
            Value::Float(_) => format!("{key}: {}", table_value(value)),
            Value::Null => format!("{key}: "),
            other => format!("{key}: {}", py_str(other)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// csv's `QUOTE_MINIMAL` for one cell.
fn csv_cell(text: &str) -> String {
    if text.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// `_write_csv_rows(args, rows)` as text.
fn csv_text(rows: &[Value]) -> Result<String, AppError> {
    let Some(Value::Dict(first)) = rows.first() else {
        return Err(AppError::new(
            "format_unsupported",
            "CSV output requires a list of records; this command does not produce tabular output",
        ));
    };
    let headers: Vec<String> = first.iter().map(|(k, _)| key_text(k)).collect();
    let line = |cells: Vec<String>| {
        // csv writes a lone empty field as "" so the row is not blank.
        let line = if cells.len() == 1 && cells[0].is_empty() {
            "\"\"".to_owned()
        } else {
            cells
                .iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(",")
        };
        line + "\r\n"
    };
    let mut out = line(headers.clone());
    for row in rows_of(&Value::List(rows.to_vec()), "get")? {
        let cells = headers
            .iter()
            .map(
                |header| match row.iter().find(|(k, _)| k == header).map(|(_, v)| v) {
                    None | Some(Value::Null) => String::new(),
                    Some(value @ (Value::Dict(_) | Value::List(_))) => inline_json(value),
                    Some(value @ Value::Float(_)) => table_value(value),
                    Some(other) => py_str(other),
                },
            )
            .collect();
        out.push_str(&line(cells));
    }
    Ok(out)
}

/// `emit(args, payload)` with an already derived `kind`.
pub fn emit(
    sink: &mut Sink<'_>,
    format: Format,
    kind: &str,
    payload: Value,
) -> Result<(), AppError> {
    let io = |error: std::io::Error| AppError::new("internal_error", error.to_string());
    match format {
        Format::Json => {
            let envelope = kassiber_envelope::success(kind, payload);
            let text =
                cli_text(&envelope).map_err(|e| AppError::new("internal_error", e.to_string()))?;
            sink.write_text(&text).map_err(io)
        }
        Format::Csv => {
            let rows = match &payload {
                Value::Dict(_) => vec![payload.clone()],
                Value::List(items) => items.clone(),
                _ => Vec::new(),
            };
            let text = csv_text(&rows)?;
            match sink.output.filter(|t| !t.is_empty() && *t != "-") {
                None => sink.stdout.write_all(text.as_bytes()).map_err(io),
                Some(target) => {
                    let path = expand_user(target, sink.home).map_err(io)?;
                    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                        std::fs::create_dir_all(parent).map_err(io)?;
                    }
                    std::fs::write(path, text).map_err(io)
                }
            }
        }
        Format::Plain => {
            let text = match &payload {
                Value::List(items) if items.is_empty() => "(no rows)".to_owned(),
                Value::List(items) => items
                    .iter()
                    .map(|item| match item {
                        Value::Dict(_) => rows_of(item, "items").map(|rows| plain_dict(&rows[0])),
                        other => Ok(py_str(other)),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                Value::Dict(_) => plain_dict(&rows_of(&payload, "items")?[0]),
                Value::Null => String::new(),
                other => py_str(other),
            };
            sink.write_text(&text).map_err(io)
        }
        Format::Table => {
            let text = match &payload {
                Value::List(_) => table_text(&rows_of(&payload, "items")?),
                Value::Dict(pairs) => {
                    let rows: Vec<Vec<(String, Value)>> = pairs
                        .iter()
                        .map(|(key, value)| {
                            vec![
                                ("field".to_owned(), Value::Str(key_text(key).into())),
                                ("value".to_owned(), value.clone()),
                            ]
                        })
                        .collect();
                    table_text(&rows)
                }
                Value::Null => String::new(),
                other => py_str(other),
            };
            sink.write_text(&text).map_err(io)
        }
    }
}

/// `emit_error(args, exc)`: the error envelope for `json`, otherwise
/// `error:` and `hint:` lines on stderr.
pub fn emit_error(sink: &mut Sink<'_>, format: Format, error: &AppError) {
    if format == Format::Json {
        let envelope = error.envelope(None);
        let Ok(text) = cli_text(&envelope) else {
            return;
        };
        if sink.write_text(&text).is_err() {
            let _ = sink.stderr.write_all(text.as_bytes());
        }
        return;
    }
    let _ = writeln!(sink.stderr, "error: {}", error.message);
    if let Some(hint) = error.hint.as_deref().filter(|h| !h.is_empty()) {
        let _ = writeln!(sink.stderr, "hint: {hint}");
    }
    let _ = sink.stderr.flush();
}
