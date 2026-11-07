//! CPython's default, non-strict csv.reader dialect, with newline="".
//! In particular BOM is data, CR/LF inside quotes are retained, short rows
//! get None, duplicate headings overwrite, and blank physical rows are skipped.
use super::*;

#[derive(Debug)]
pub(super) struct Row {
    pub data: Json,
    pub headers: Vec<String>,
    pub rest: Option<Vec<String>>,
    pub original: Option<Py>,
}
pub(super) fn rows(text: &str) -> Result<Vec<Row>, AppError> {
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut count = 0;
    let mut quoted = false;
    let mut start = true;
    let mut touched = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                } else {
                    quoted = false;
                    continue;
                }
            }
        } else {
            if ch == '"' && start {
                quoted = true;
                start = false;
                touched = true;
                continue;
            }
            if ch == ',' || ch == '\r' || ch == '\n' {
                if ch == ',' || touched || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                }
                if ch != ',' {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    // The first blank row is the empty fieldnames list;
                    // later blank rows are ignored by DictReader.
                    if !row.is_empty() || records.is_empty() {
                        records.push(std::mem::take(&mut row));
                    }
                    touched = false;
                } else {
                    touched = true;
                }
                start = true;
                count = 0;
                continue;
            }
        }
        field.push(ch);
        count += 1;
        if count > 131_072 {
            return Err(AppError::new(
                "internal_error",
                "field larger than field limit (131072)",
            ));
        }
        start = false;
        touched = true;
    }
    if touched || !row.is_empty() {
        row.push(field);
        records.push(row);
    }
    let mut records = records.into_iter();
    let Some(header) = records.next() else {
        return Ok(vec![]);
    };
    records
        .map(|row| {
            let rest = (row.len() > header.len()).then(|| row[header.len()..].to_vec());
            let mut object = serde_json::Map::new();
            for (i, key) in header.iter().enumerate() {
                object.insert(key.clone(), row.get(i).map_or(Json::Null, |v| json!(v)));
            }
            Ok(Row {
                data: Json::Object(object),
                headers: header.clone(),
                rest,
                original: None,
            })
        })
        .collect()
}

pub(super) fn load(file: &str) -> Result<Vec<Row>, AppError> {
    rows(&read_text(file)?)
}

pub(super) fn read_text(file: &str) -> Result<String, AppError> {
    read_encoded(file, false)
}
pub(super) fn read_sig(file: &str) -> Result<String, AppError> {
    read_encoded(file, true)
}
fn read_encoded(file: &str, sig: bool) -> Result<String, AppError> {
    exists(file)?;
    read_encoded_direct(file, sig)
}
fn exists(file: &str) -> Result<(), AppError> {
    if !std::path::Path::new(file).exists() {
        return Err(
            AppError::new("not_found", format!("Import file not found: {file}"))
                .with_hint("Check the wallet source_file or pass an existing --file path."),
        );
    }
    Ok(())
}

pub(super) fn read_text_direct(file: &str) -> Result<String, AppError> {
    read_encoded_direct(file, false)
}

/// `_read_json_file(file_path, label)`: parse a JSON import file, refusing
/// one that is not UTF-8 JSON. A CSV export imported with a JSON format, or a
/// file in another encoding, is a wrong-format choice, not an internal error.
/// `checked` is the coordinator's missing-file check, which the Wasabi loader
/// does not make.
pub(super) fn read_json(file: &str, label: &str, checked: bool) -> Result<Py, AppError> {
    if checked {
        exists(file)?;
    }
    let refuse = || {
        AppError::new("validation", format!("{label} is not valid UTF-8 JSON"))
            .with_hint("Choose the import format that matches the file.")
    };
    let bytes = read_bytes(file)?;
    // open(..., encoding="utf-8") uses universal newlines.
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| refuse())?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    kassiber_pytext::loads(&text).map_err(|error| match error.kind {
        // RecursionError is not a ValueError.
        kassiber_pytext::JsonErrorKind::TooDeep { .. } => {
            AppError::new("internal_error", error.to_string())
        }
        _ => refuse(),
    })
}

fn read_bytes(file: &str) -> Result<Vec<u8>, AppError> {
    std::fs::read(file).map_err(|e| {
        let reason = e
            .raw_os_error()
            .map(|n| std::io::Error::from_raw_os_error(n).to_string());
        let reason = reason
            .as_deref()
            .unwrap_or("I/O error")
            .split(" (os error")
            .next()
            .unwrap();
        AppError::new(
            "internal_error",
            format!(
                "[Errno {}] {reason}: {}",
                e.raw_os_error().unwrap_or(5),
                kassiber_pytext::repr_str(file)
            ),
        )
    })
}

fn read_encoded_direct(file: &str, sig: bool) -> Result<String, AppError> {
    let bytes = read_bytes(file)?;
    let bytes = if sig {
        bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)
    } else {
        &bytes
    };
    let text = std::str::from_utf8(bytes).map_err(|e| {
        let at = e.valid_up_to();
        let n = e.error_len().unwrap_or(bytes.len() - at);
        let position = if n == 1 {
            format!("byte 0x{:02x} in position {at}", bytes[at])
        } else {
            format!("bytes in position {at}-{}", at + n - 1)
        };
        let reason = if e.error_len().is_none() {
            "unexpected end of data"
        } else if bytes[at] < 0xc2 || bytes[at] > 0xf4 {
            "invalid start byte"
        } else {
            "invalid continuation byte"
        };
        AppError::new(
            "internal_error",
            format!("'utf-8' codec can't decode {position}: {reason}"),
        )
    })?;
    Ok(text.to_owned())
}
