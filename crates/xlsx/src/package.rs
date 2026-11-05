//! rust_xlsxwriter stores row heights as integer pixels. Python stores exact
//! points (28 becomes 27.75 in the Rust API). Restore only the generated ht
//! attributes; all ZIP members remain in memory with fixed ZIP timestamps.
use crate::{Cell, Operation, Sheet};
use rust_xlsxwriter::XlsxError;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub(super) fn restore_heights(bytes: Vec<u8>, sheets: &[Sheet]) -> Result<Vec<u8>, XlsxError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut out = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for index in 0..archive.len() {
        let mut member = archive.by_index(index)?;
        let name = member.name()?.into_owned();
        let mut content = Vec::new();
        member.read_to_end(&mut content)?;
        if let Some(sheet) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|s| s.strip_suffix(".xml"))
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|n| n.checked_sub(1))
            .and_then(|n| sheets.get(n))
        {
            let heights: BTreeMap<u32, f64> = sheet
                .operations
                .iter()
                .filter_map(|op| match op {
                    Operation::Row { row, height } => Some((row + 1, *height)),
                    _ => None,
                })
                .collect();
            let xml =
                String::from_utf8(content).map_err(|e| XlsxError::ParameterError(e.to_string()))?;
            let mut fixed = String::with_capacity(xml.len());
            let mut tail = xml.as_str();
            while let Some(start) = tail.find("<row ") {
                let (prefix, row) = tail.split_at(start);
                let end = row
                    .find('>')
                    .ok_or_else(|| XlsxError::ParameterError("invalid generated row".into()))?;
                let (tag, remainder) = row.split_at(end);
                fixed.push_str(prefix);
                let number = attribute(tag, "r").and_then(|n| n.parse::<u32>().ok());
                if let Some(height) = number.and_then(|n| heights.get(&n)) {
                    if let Some(original) = attribute(tag, "ht") {
                        fixed.push_str(
                            &tag.replace(
                                &format!("ht=\"{original}\""),
                                &format!("ht=\"{height}\""),
                            ),
                        );
                    } else {
                        fixed.push_str(tag);
                    }
                } else {
                    fixed.push_str(tag);
                }
                tail = remainder;
            }
            fixed.push_str(tail);
            // Rust emits a 200 DPI hint where Python leaves the printer
            // resolution unspecified. The reports never set one.
            let mut numbers = BTreeMap::new();
            let mut empty_strings = BTreeSet::new();
            for operation in &sheet.operations {
                match operation {
                    Operation::Cell {
                        row, col, value, ..
                    } => {
                        let key = reference(*row, *col);
                        if matches!(value, Cell::String(s) if s.is_empty()) {
                            empty_strings.insert(key.clone());
                        } else {
                            empty_strings.remove(&key);
                        }
                        if let Cell::Number(value) = value {
                            numbers.insert(key, number16(*value));
                        } else {
                            numbers.remove(&key);
                        }
                    }
                    Operation::Merge {
                        first_row,
                        first_col,
                        last_row,
                        last_col,
                        ..
                    } if first_row != last_row || first_col != last_col => {
                        for row in *first_row..=*last_row {
                            for col in *first_col..=*last_col {
                                numbers.remove(&reference(row, col));
                                empty_strings.remove(&reference(row, col));
                            }
                        }
                    }
                    _ => (),
                }
            }
            let fixed = restore_numbers(&fixed, &numbers, &empty_strings)?;
            content = fixed
                .replace(" horizontalDpi=\"200\"", "")
                .replace(" verticalDpi=\"200\"", "")
                .replace(" orientation=\"portrait\"", "")
                .into_bytes();
        }
        out.start_file(name, options)?;
        out.write_all(&content)?;
    }
    Ok(out.finish()?.into_inner())
}

fn reference(row: u32, col: u16) -> String {
    let mut col = u32::from(col) + 1;
    let mut letters = Vec::new();
    while col > 0 {
        letters.push((b'A' + ((col - 1) % 26) as u8) as char);
        col = (col - 1) / 26;
    }
    format!(
        "{}{row}",
        letters.iter().rev().collect::<String>(),
        row = row + 1
    )
}

/// Python XlsxWriter's %.16G, including scientific notation for huge/tiny
/// values. Scientific notation matters to readers which otherwise parse a
/// huge integer literally instead of the spreadsheet's IEEE-754 number.
fn number16(value: f64) -> String {
    let scientific = format!("{value:.15e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    if !(-4..16).contains(&exponent) {
        return format!("{mantissa}E{exponent:+03}");
    }
    let sign = if mantissa.starts_with('-') { "-" } else { "" };
    let digits = mantissa.trim_start_matches('-').replace('.', "");
    let point = exponent + 1;
    if point <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!(
            "{sign}{digits}{}",
            "0".repeat(point as usize - digits.len())
        )
    } else {
        format!(
            "{sign}{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    }
}

fn restore_numbers(
    xml: &str,
    numbers: &BTreeMap<String, String>,
    empty_strings: &BTreeSet<String>,
) -> Result<String, XlsxError> {
    let mut out = String::with_capacity(xml.len());
    let mut tail = xml;
    while let Some(start) = tail.find("<c ") {
        let (prefix, cell) = tail.split_at(start);
        let tag_end = cell
            .find('>')
            .ok_or_else(|| XlsxError::ParameterError("invalid generated cell".into()))?;
        let end = if cell[..tag_end].ends_with('/') {
            tag_end + 1
        } else {
            cell.find("</c>")
                .map(|n| n + 4)
                .ok_or_else(|| XlsxError::ParameterError("invalid generated cell".into()))?
        };
        let (cell, remainder) = cell.split_at(end);
        out.push_str(prefix);
        let number = attribute(cell, "r").and_then(|r| numbers.get(r));
        if attribute(cell, "r").is_some_and(|r| empty_strings.contains(r)) {
            // Python write_string("") is a typed empty string. The Rust API
            // turns it into a blank; restore the type with an inline string.
            out.push_str(cell[..tag_end].trim_end_matches('/'));
            out.push_str(" t=\"inlineStr\"><is><t></t></is></c>");
        } else if let Some(number) = number {
            let start = cell
                .find("<v>")
                .ok_or_else(|| XlsxError::ParameterError("missing generated value".into()))?
                + 3;
            let end = cell[start..].find("</v>").unwrap() + start;
            out.push_str(&cell[..start]);
            out.push_str(number);
            out.push_str(&cell[end..]);
        } else {
            out.push_str(cell);
        }
        tail = remainder;
    }
    out.push_str(tail);
    Ok(out)
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let (_, value) = tag.split_once(&format!(" {name}=\""))?;
    value.split_once('"').map(|(value, _)| value)
}

/// A deflated ZIP of `members` in order, with fixed timestamps, so the same
/// files always give the same bytes. Report bundles share it.
pub fn archive(members: &[(String, Vec<u8>)]) -> Result<Vec<u8>, XlsxError> {
    let mut out = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, bytes) in members {
        out.start_file(name.as_str(), options)?;
        out.write_all(bytes)?;
    }
    Ok(out.finish()?.into_inner())
}
