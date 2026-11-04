//! Desktop wallet onboarding kinds: script-type detection, ownership
//! identification and the uploaded-ledger preview. Only detection contacts a
//! backend, as the user's explicit action; the other two stay local.
use crate::{
    imports::optional_string,
    output::{with_request_id, RequestId},
    Daemon, Handled,
};
use kassiber_envelope::{dict, get, s, success, type_name, AppError};
use kassiber_pyjson::{BigInt, Value};
use kassiber_wallets::{catalog_detection, file_import::ledger};

pub const KINDS: &[&str] = &[
    "ui.wallets.identify",
    "ui.wallets.ledger_preview",
    "ui.wallets.detect_script_types",
];

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let payload = match kind {
        // Python hands the raw args to the snapshot, which checks them itself.
        "ui.wallets.identify" => daemon
            .book_mut()?
            .service(kind, args.unwrap_or(&Value::Null))?,
        "ui.wallets.ledger_preview" => ledger_preview(&object(args)?)?,
        _ => {
            let args = detection_args(&object(args)?)?;
            daemon.book_mut()?.service(kind, &args)?
        }
    };
    Ok((
        Some(with_request_id(success(kind, payload), request_id)),
        false,
    ))
}

/// `_coerce_args_dict`.
fn object(args: Option<&Value>) -> Result<Value, AppError> {
    match args {
        None | Some(Value::Null) => Ok(dict::<&str>([])),
        Some(value @ Value::Dict(_)) => Ok(value.clone()),
        Some(value) => Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(value)))]))),
    }
}

/// `_detect_script_types_payload` up to the backend lookup.
fn detection_args(args: &Value) -> Result<Value, AppError> {
    let material = optional_string(args, "wallet_material")?.ok_or_else(|| {
        AppError::new("validation", "Wallet export is required.").with_hint("Enter wallet export.")
    })?;
    catalog_detection::validate_material(&material)?;
    let mut normalized = vec![("wallet_material", s(material))];
    for key in ["chain", "network", "backend"] {
        normalized.push((key, optional_string(args, key)?.map_or(Value::Null, s)));
    }
    Ok(dict(normalized))
}

/// `_ledger_preview_payload`.
fn ledger_preview(args: &Value) -> Result<Value, AppError> {
    let encoded = optional_string(args, "source_bytes_base64")?;
    let filename = optional_string(args, "filename")?.unwrap_or_else(|| "ledger.csv".into());
    let Some(encoded) = encoded else {
        return Err(
            AppError::new("validation", "source_bytes_base64 is required").with_hint(
                "Choose the ledger file with the desktop file picker before previewing it.",
            ),
        );
    };
    let extension = suffix(&filename).to_lowercase();
    if ![".csv", ".tsv", ".xlsx", ".xlsm"].contains(&extension.as_str()) {
        return Err(
            AppError::new("validation", "Unsupported ledger preview file type.")
                .with_hint("Choose a CSV, TSV, XLSX, or XLSM ledger file.")
                .with_details(dict([(
                    "extension",
                    if extension.is_empty() {
                        Value::Null
                    } else {
                        s(extension.as_str())
                    },
                )])),
        );
    }
    let bytes = strict_base64(&encoded).ok_or_else(|| {
        AppError::new("validation", "Could not decode selected ledger file.")
            .with_hint("Choose the file again and retry the preview.")
    })?;
    let mapping = match get(args, "column_map") {
        None | Some(Value::Null) => None,
        Some(value @ Value::Dict(_)) => Some(ledger::ColumnMap::from_python(value)?),
        Some(_) => {
            return Err(AppError::new("validation", "column_map must be an object")
                .with_hint("Map ledger fields to this file's column names."))
        }
    };
    let limit = preview_limit(get(args, "limit"))?;
    ledger::preview_upload(&bytes, &extension, mapping.as_ref(), limit)
}

/// `PurePosixPath(name).suffix`.
fn suffix(filename: &str) -> &str {
    let name = filename
        .split('/')
        .rfind(|part| !part.is_empty() && *part != ".")
        .unwrap_or("");
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => &name[at..],
        _ => "",
    }
}

/// `max(0, int(limit))`, with Python's fallback of 200 for values `int()`
/// refuses with TypeError/ValueError. An infinite float overflows in Python
/// and is refused here.
fn preview_limit(value: Option<&Value>) -> Result<usize, AppError> {
    let parsed = match value {
        None | Some(Value::Null) => Some(BigInt::from(200)),
        Some(Value::Bool(flag)) => Some(BigInt::from(u8::from(*flag))),
        Some(Value::Int(number)) => Some(number.clone()),
        Some(Value::Float(number)) if number.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Some(Value::Float(number)) if number.is_nan() => None,
        Some(Value::Float(number)) => {
            kassiber_pytext::parse_int(&format!("{:.0}", number.trunc())).ok()
        }
        Some(Value::Str(text)) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok()),
        Some(_) => None,
    };
    let limit = parsed.unwrap_or_else(|| BigInt::from(200));
    Ok(if limit < BigInt::from(0) {
        0
    } else {
        usize::try_from(limit).unwrap_or(usize::MAX)
    })
}

/// `base64.b64decode(value, validate=True)`: CPython's strict decoder.
fn strict_base64(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !text.is_ascii() || bytes.first() == Some(&b'=') {
        return None;
    }
    let (mut quad, mut pads, mut padding, mut left) = (0, 0, false, 0u32);
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (at, &byte) in bytes.iter().enumerate() {
        if byte == b'=' {
            padding = true;
            if quad >= 2 {
                pads += 1;
                if quad + pads >= 4 {
                    return (at + 1 == bytes.len()).then_some(out);
                }
            }
            continue;
        }
        let value = u32::from(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        });
        if padding {
            return None;
        }
        match quad {
            0 => left = value,
            1 => {
                out.push((left << 2 | value >> 4) as u8);
                left = value & 0xf;
            }
            2 => {
                out.push((left << 4 | value >> 2) as u8);
                left = value & 0x3;
            }
            _ => out.push((left << 6 | value) as u8),
        }
        quad = (quad + 1) % 4;
    }
    (quad == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_base64_matches_cpython_validate_mode() {
        for (text, expected) in [
            ("", Some(vec![])),
            ("AAAA=", Some(vec![0, 0, 0])),
            ("AAAA====", Some(vec![0, 0, 0])),
            ("AA==", Some(vec![0])),
            ("AAA=", Some(vec![0, 0])),
            ("aGk=", Some(b"hi".to_vec())),
            ("AA=", None),
            ("AA=A", None),
            ("AAA==", None),
            ("=AAA", None),
            ("A", None),
            ("AB", None),
            ("AA==AA==", None),
            ("AA== ", None),
            ("AAAA=A", None),
            ("AB==\n", None),
            ("AA-_", None),
            ("ÄAAA", None),
        ] {
            assert_eq!(strict_base64(text), expected, "{text:?}");
        }
    }

    #[test]
    fn upload_suffix_and_limit_follow_python() {
        for (name, expected) in [
            ("x.CSV", ".CSV"),
            (".csv", ""),
            ("a.", ""),
            ("dir/x.tar.xlsx", ".xlsx"),
            ("x.csv/", ".csv"),
            ("x.csv/.", ".csv"),
            ("", ""),
        ] {
            assert_eq!(suffix(name), expected, "{name:?}");
        }
        let limit = |value: Value| preview_limit(Some(&value)).unwrap();
        assert_eq!(limit(Value::Null), 200);
        assert_eq!(limit(Value::Bool(true)), 1);
        assert_eq!(limit(Value::from(-3)), 0);
        assert_eq!(limit(Value::Float(2.9)), 2);
        assert_eq!(limit(Value::Float(f64::NAN)), 200);
        assert_eq!(limit(s(" 7 ")), 7);
        assert_eq!(limit(s("x")), 200);
        assert_eq!(limit(Value::List(vec![])), 200);
        assert!(preview_limit(Some(&Value::Float(f64::INFINITY))).is_err());
    }
}
