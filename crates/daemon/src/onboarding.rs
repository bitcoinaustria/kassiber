//! Desktop wallet onboarding kinds: script-type detection, ownership
//! identification and the uploaded-ledger preview. Only detection contacts a
//! backend, as the user's explicit action; the other two stay local.
use crate::{
    imports::optional_string,
    output::{with_request_id, RequestId},
    Daemon, Handled,
};
use kassiber_envelope::redaction::redact_secret_text;
use kassiber_envelope::{build_error_envelope, dict, get, py_str, s, success, type_name, AppError};
use kassiber_pyjson::{BigInt, Value};
use kassiber_wallets::{
    catalog_detection,
    file_import::{self, analysis, ledger, DesktopImport},
    identify::onchain::Start,
};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const KINDS: &[&str] = &[
    "ui.wallets.identify",
    "ui.wallets.identify_onchain",
    "ui.wallets.identify_onchain.cancel",
    "ui.wallets.import_file",
    "ui.wallets.import_samourai",
    "ui.wallets.ledger_preview",
    "ui.wallets.analyze_file",
    "ui.wallets.detect_script_types",
    "wallets.reveal_descriptor",
];

const ONCHAIN: &str = "ui.wallets.identify_onchain";
const ONCHAIN_CANCEL: &str = "ui.wallets.identify_onchain.cancel";
const ONCHAIN_PROGRESS: &str = "ui.wallets.identify_onchain.progress";
/// A cancel can race ahead of the request it names; it is held this long.
const PENDING_CANCEL_TTL: Duration = Duration::from_secs(30);
const MAX_PENDING_CANCELS: usize = 32;

/// `ActiveVerifications`: cancel flags for running on-chain verifications,
/// keyed by request id, plus cancels that arrived first.
#[derive(Default)]
pub(crate) struct Verifications {
    state: Mutex<Registry>,
}

#[derive(Default)]
struct Registry {
    running: HashMap<String, Arc<AtomicBool>>,
    /// Cancels for runs not registered yet, oldest first, with their expiry.
    pending: Vec<(String, Instant)>,
}

impl Verifications {
    fn register(&self, key: Option<&str>) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let Some(key) = key else {
            return flag;
        };
        let now = Instant::now();
        let mut state = self.state.lock().expect("verification registry");
        state.pending.retain(|(_, deadline)| *deadline >= now);
        if let Some(at) = state.pending.iter().position(|(pending, _)| pending == key) {
            state.pending.remove(at);
            flag.store(true, Ordering::SeqCst);
        }
        state.running.insert(key.to_owned(), flag.clone());
        flag
    }

    fn unregister(&self, key: Option<&str>, flag: &Arc<AtomicBool>) {
        let Some(key) = key else {
            return;
        };
        let mut state = self.state.lock().expect("verification registry");
        if state
            .running
            .get(key)
            .is_some_and(|running| Arc::ptr_eq(running, flag))
        {
            state.running.remove(key);
        }
    }

    fn cancel(&self, key: &str) -> bool {
        let mut state = self.state.lock().expect("verification registry");
        if let Some(flag) = state.running.get(key) {
            flag.store(true, Ordering::SeqCst);
            return true;
        }
        let deadline = Instant::now() + PENDING_CANCEL_TTL;
        match state.pending.iter_mut().find(|(pending, _)| pending == key) {
            Some(entry) => entry.1 = deadline,
            None => {
                if state.pending.len() >= MAX_PENDING_CANCELS {
                    state.pending.remove(0);
                }
                state.pending.push((key.to_owned(), deadline));
            }
        }
        false
    }
}

/// `_request_id_registry_key`.
fn registry_key(request_id: &RequestId) -> Option<String> {
    match request_id {
        RequestId::Missing | RequestId::Value(Value::Null) => None,
        RequestId::Value(id @ Value::Str(_)) => Some(py_str(id)).filter(|key| !key.is_empty()),
        RequestId::Value(id) => Some(py_str(id)),
    }
}

/// `daemon_ownership_verify._error`: no details, redacted text.
fn worker_error(error: &AppError) -> Value {
    let hint = error.hint.as_deref().map(redact_secret_text);
    build_error_envelope(
        &error.code,
        &redact_secret_text(&error.message),
        None,
        hint.as_deref(),
        error.retryable,
        None,
    )
}

/// `daemon_ownership_verify.start`: answer now, or stream from a worker that
/// never touches the book.
fn identify_onchain(daemon: &mut Daemon, args: Option<&Value>, request_id: &RequestId) -> Handled {
    let start = daemon
        .book_mut()?
        .identify_onchain(args.unwrap_or(&Value::Null))?;
    let verification = match start {
        Start::Report(report) => {
            return Ok((
                Some(with_request_id(success(ONCHAIN, report), request_id)),
                false,
            ));
        }
        Start::Verify(verification) => verification,
    };
    let key = registry_key(request_id);
    let registry = daemon.verifications.clone();
    let cancelled = registry.register(key.as_deref());
    let output = daemon.output.clone();
    let request_id = request_id.clone();
    let spawned = std::thread::Builder::new()
        .name("kassiber-identify-onchain".into())
        .spawn(move || {
            let progress = |checked: usize, total: usize| {
                let data = dict([
                    ("checked", Value::from(checked as i64)),
                    ("total", Value::from(total as i64)),
                ]);
                output
                    .write(&with_request_id(
                        success(ONCHAIN_PROGRESS, data),
                        &request_id,
                    ))
                    .map_err(|_| AppError::new("cancelled", "Daemon output closed"))
            };
            let total = verification.pending();
            let result = progress(0, total).and_then(|()| {
                catch_unwind(AssertUnwindSafe(|| {
                    verification.run(&mut |checked, total| progress(checked, total), &|| {
                        cancelled.load(Ordering::SeqCst)
                    })
                }))
                .unwrap_or_else(|_| Err(AppError::new("internal", "On-chain verification failed")))
            });
            let record = match result {
                Ok(report) => success(ONCHAIN, report),
                Err(error) => worker_error(&error),
            };
            let _ = output.write(&with_request_id(record, &request_id));
            registry.unregister(key.as_deref(), &cancelled);
        });
    if spawned.is_err() {
        return Err(
            AppError::new("internal_error", "Could not start on-chain verification").into(),
        );
    }
    Ok((None, false))
}

/// `daemon_ownership_verify.cancel`.
fn cancel_onchain(daemon: &Daemon, args: &Value) -> Result<Value, AppError> {
    let target = match get(args, "target_request_id") {
        Some(Value::Str(target)) if target.code_points().next().is_some() => {
            py_str(&Value::Str(target.clone()))
        }
        _ => {
            return Err(AppError::new(
                "validation",
                format!("{ONCHAIN_CANCEL} requires target_request_id"),
            )
            .with_hint(format!(
                "Pass {{target_request_id: '<active {ONCHAIN} request_id>'}}."
            )))
        }
    };
    let running = daemon.verifications.cancel(&target);
    Ok(dict([
        ("cancelled", Value::Bool(true)),
        ("running", Value::Bool(running)),
    ]))
}

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    if kind == ONCHAIN {
        return identify_onchain(daemon, args, request_id);
    }
    if kind == "wallets.reveal_descriptor" {
        return reveal_descriptor(daemon, args, request_id);
    }
    if ["ui.wallets.import_file", "ui.wallets.import_samourai"].contains(&kind) {
        let args = object(args)?;
        let import = if kind == "ui.wallets.import_file" {
            import_file(daemon, &args)?
        } else {
            DesktopImport::Samourai(samourai(&args)?)
        };
        let payload = daemon.book_mut()?.desktop_import(import)?;
        // Python ends the session after a committed desktop import.
        return Ok((
            Some(with_request_id(success(kind, payload), request_id)),
            true,
        ));
    }
    let payload = match kind {
        "ui.wallets.analyze_file" => analyze_file(daemon, &object(args)?)?,
        ONCHAIN_CANCEL => cancel_onchain(daemon, &object(args)?)?,
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

/// `_import_wallet_file_payload` up to the importer: argument checks in
/// Python's order, the active book, then the per-format wallet and mode rules.
fn import_file(daemon: &mut Daemon, args: &Value) -> Result<DesktopImport, AppError> {
    let required = |key: &str, label: &str| {
        optional_string(args, key)?.ok_or_else(|| {
            AppError::new("validation", format!("{label} is required."))
                .with_hint(format!("Enter {}.", label.to_lowercase()))
        })
    };
    let source_format = required("source_format", "Source format")?;
    if let Some(bundle) = get(args, "source_bundle").filter(|v| !matches!(v, Value::Null)) {
        if source_format != "wasabi_bundle" {
            return Err(AppError::new(
                "validation",
                "Inline source_bundle is only supported for Wasabi imports",
            ));
        }
        let wallet = required("wallet", "Wallet")?;
        let (_, profile_id) = daemon.book_mut()?.resolve_scope(None, None)?;
        return Ok(DesktopImport::WasabiPayload {
            request: file_import::Request {
                profile_id,
                wallet,
                file: "inline:wasabi_bundle".into(),
                format: file_import::Format::Wasabi,
                mode: None,
            },
            payload: bundle.clone(),
        });
    }
    let Some(file) = crate::wallets::source_file(args)? else {
        return Err(AppError::new("validation", "source_file is required")
            .with_hint("Choose the local export file to import."));
    };
    if !crate::wallets::SOURCE_FORMATS.contains(&source_format.as_str()) {
        return Err(AppError::new(
            "validation",
            format!("Unsupported source format '{source_format}'"),
        )
        .with_hint("Choose a supported file format."));
    }
    let format = file_import::Format::from_source_format(&source_format);
    let mode = || -> Result<Option<String>, AppError> {
        Ok(match optional_string(args, "mode")? {
            Some(mode) => Some(mode),
            None => optional_string(args, "import_mode")?,
        })
    };
    let (wallet, mode, column_map) = match source_format.as_str() {
        "bullbitcoin_csv" | "coinfinity_csv" | "pocketbitcoin_csv" => {
            let wallet = optional_string(args, "wallet")?;
            (
                wallet,
                Some(mode()?.unwrap_or_else(|| "relevant".into())),
                None,
            )
        }
        "21bitcoin_csv" => {
            let wallet = optional_string(args, "wallet")?;
            (wallet, mode()?, None)
        }
        "binance_supplemental_csv" => {
            let wallet = optional_string(args, "wallet")?;
            (wallet, Some(mode()?.unwrap_or_else(|| "full".into())), None)
        }
        "strike_csv" => (optional_string(args, "wallet")?, Some("full".into()), None),
        _ => {
            let wallet = required("wallet", "Wallet")?;
            let column_map = match get(args, "column_map") {
                None | Some(Value::Null) => None,
                Some(value @ Value::Dict(_)) => Some(ledger::ColumnMap::from_python(value)?),
                Some(_) => {
                    return Err(AppError::new("validation", "column_map must be an object")
                        .with_hint("Map ledger fields to this file's column names."))
                }
            };
            (Some(wallet), None, column_map)
        }
    };
    // The Bull Bitcoin wallet-history importer is not native yet.
    let format = format.ok_or_else(|| {
        kassiber_runtime::store::not_implemented("wallets", "Bull Bitcoin wallet history import")
    })?;
    let (_, profile_id) = daemon.book_mut()?.resolve_scope(None, None)?;
    Ok(DesktopImport::File {
        request: file_import::Request {
            profile_id,
            wallet: wallet.unwrap_or_default(),
            file,
            format,
            mode,
        },
        column_map,
    })
}

/// `_import_samourai_payload`: watch-only source sets only.
fn samourai(args: &Value) -> Result<file_import::samourai::Request, AppError> {
    let label = optional_string(args, "label")?.ok_or_else(|| {
        AppError::new("validation", "Connection label is required.")
            .with_hint("Enter connection label.")
    })?;
    let file = optional_string(args, "source_set_file")?;
    let source_set = get(args, "source_set").filter(|v| !matches!(v, Value::Null));
    let forbidden: Vec<Value> = [
        "backup_file",
        "backup_passphrase",
        "mnemonic",
        "mnemonic_passphrase",
        "source_file",
    ]
    .into_iter()
    .filter(|key| get(args, key).is_some_and(|v| !matches!(v, Value::Null) && *v != s("")))
    .map(s)
    .collect();
    if !forbidden.is_empty() {
        return Err(AppError::new(
            "validation",
            "Samourai imports accept only watch-only descriptor/xpub source sets",
        )
        .with_hint("Paste public Deposit, Badbank, Premix, and Postmix descriptors or xpubs instead of backup or recovery material.")
        .with_details(dict([("unsupported_fields", Value::List(forbidden))])));
    }
    if source_set.is_some_and(|v| !matches!(v, Value::Dict(_))) {
        return Err(AppError::new(
            "validation",
            "source_set must be a JSON object",
        ));
    }
    let gap_limit = match get(args, "gap_limit") {
        None | Some(Value::Null) => None,
        Some(value) if *value == s("") => None,
        Some(value @ (Value::Int(_) | Value::Bool(_))) => Some(value.clone()),
        Some(value) => {
            return Err(AppError::new("validation", "gap_limit must be an integer")
                .with_details(dict([("type", s(type_name(value)))])))
        }
    };
    Ok(file_import::samourai::Request {
        workspace: None,
        profile: None,
        label,
        account: optional_string(args, "account")?,
        backend: optional_string(args, "backend")?,
        network: optional_string(args, "network")?,
        gap_limit,
        file: file.unwrap_or_default(),
        source_set: source_set.cloned(),
    })
}

/// `_analyze_sample_rows_arg`: bounded, and `0` means no rows.
fn sample_rows(args: &Value) -> Result<usize, AppError> {
    let invalid = || AppError::new("validation", "sample_rows must be a number");
    let number = match get(args, "sample_rows") {
        None | Some(Value::Null) => return Ok(analysis::DEFAULT_SAMPLE_ROWS),
        Some(Value::Int(number)) => number.clone(),
        Some(Value::Float(number)) if number.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", number.trunc())).map_err(|_| invalid())?
        }
        Some(Value::Float(number)) => {
            return Err(AppError::new(
                "internal_error",
                if number.is_nan() {
                    "cannot convert float NaN to integer"
                } else {
                    "cannot convert float infinity to integer"
                },
            ))
        }
        Some(_) => return Err(invalid()),
    };
    Ok(if number <= BigInt::from(0) {
        0
    } else {
        usize::try_from(number)
            .unwrap_or(usize::MAX)
            .min(analysis::MAX_SAMPLE_ROWS)
    })
}

/// `_analyze_file_payload`: a staged token or uploaded bytes, never a path.
fn analyze_file(daemon: &mut Daemon, args: &Value) -> Result<Value, AppError> {
    let token = optional_string(args, "token")?;
    let column_map = match get(args, "column_map") {
        None | Some(Value::Null) => None,
        Some(value @ Value::Dict(_)) => Some(value.clone()),
        Some(_) => {
            return Err(AppError::new("validation", "column_map must be an object")
                .with_hint("Map ledger fields to this file's column names."))
        }
    };
    let rows = sample_rows(args)?;
    let mapping = column_map
        .as_ref()
        .map(ledger::ColumnMap::from_python)
        .transpose()?;
    if let Some(token) = token {
        let source = daemon.document_source(&token)?;
        return analysis::analyze(&source, mapping.as_ref(), rows).map_err(|error| {
            if error.code == "not_found" && error.message.contains(&source) {
                crate::documents_import::source_unavailable()
            } else {
                error
            }
        });
    }
    let Some(encoded) = optional_string(args, "source_bytes_base64")? else {
        return Err(
            AppError::new("validation", "token or source_bytes_base64 is required")
                .with_hint("Choose the export file with the file picker before analyzing it."),
        );
    };
    let filename = optional_string(args, "filename")?.unwrap_or_else(|| "export.csv".into());
    let extension = suffix(&filename).to_lowercase();
    let supported = analysis::supported_extensions();
    if !supported.contains(&extension.as_str()) {
        return Err(
            AppError::new("validation", "Unsupported file type for analysis")
                .with_hint("Choose a CSV, TSV, XLSX, PDF, or image export.")
                .with_details(dict([
                    (
                        "extension",
                        if extension.is_empty() {
                            Value::Null
                        } else {
                            s(extension.as_str())
                        },
                    ),
                    (
                        "supported_extensions",
                        Value::List(supported.into_iter().map(s).collect()),
                    ),
                ])),
        );
    }
    let bytes = strict_base64(&encoded).ok_or_else(|| {
        AppError::new("validation", "Could not decode the selected file.")
            .with_hint("Choose the file again and retry.")
    })?;
    let mut result = analysis::analyze_upload(&bytes, &extension, mapping.as_ref(), rows)?;
    // The temporary path is gone; never hand it back as if it were importable.
    if let Value::Dict(fields) = &mut result {
        for (key, value) in fields.iter_mut() {
            if *key == kassiber_pyjson::Key::from("source") {
                if let Value::Dict(source) = value {
                    for (key, value) in source.iter_mut() {
                        if *key == kassiber_pyjson::Key::from("path") {
                            *value = Value::Null;
                        } else if *key == kassiber_pyjson::Key::from("filename") {
                            *value = s(filename.as_str());
                        }
                    }
                }
            }
        }
    }
    Ok(result)
}

/// `_handle_reveal_request` for wallets: fresh local authentication on every
/// request, then only the pasteable descriptor material. No AI tool or other
/// projection reaches this kind.
fn reveal_descriptor(daemon: &mut Daemon, args: Option<&Value>, request_id: &RequestId) -> Handled {
    if let Some(response) =
        daemon.authorize_sensitive("wallets.reveal_descriptor", args, request_id)?
    {
        return Ok((Some(response), false));
    }
    let empty = dict::<&str>([]);
    let args = args
        .filter(|value| matches!(value, Value::Dict(_)))
        .unwrap_or(&empty);
    let target = ["name", "wallet", "backend"]
        .into_iter()
        .filter_map(|key| get(args, key))
        .find(|value| kassiber_envelope::truthy(value))
        .map(py_str)
        .expect("authorization validated the target");
    let payload = daemon.book_mut()?.reveal_descriptor(
        get(args, "workspace"),
        get(args, "profile"),
        &target,
    )?;
    Ok((
        Some(with_request_id(
            success("wallets.reveal_descriptor", payload),
            request_id,
        )),
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
    fn verification_registry_keys_follow_python() {
        assert_eq!(registry_key(&RequestId::Missing), None);
        assert_eq!(registry_key(&RequestId::Value(Value::Null)), None);
        assert_eq!(registry_key(&RequestId::Value(s(""))), None);
        assert_eq!(registry_key(&RequestId::Value(s("v"))), Some("v".into()));
        assert_eq!(
            registry_key(&RequestId::Value(Value::from(7))),
            Some("7".into())
        );
        assert_eq!(
            registry_key(&RequestId::Value(Value::Bool(true))),
            Some("True".into())
        );
    }

    #[test]
    fn verification_cancels_reach_running_and_later_runs() {
        let registry = Verifications::default();
        let running = registry.register(Some("run"));
        assert!(registry.cancel("run"));
        assert!(running.load(Ordering::SeqCst));
        // A cancel that arrives first is held for the run it names.
        assert!(!registry.cancel("later"));
        assert!(registry.register(Some("later")).load(Ordering::SeqCst));
        assert!(!registry.register(Some("fresh")).load(Ordering::SeqCst));
        // A finished run never unregisters a newer one with the same key.
        let newer = registry.register(Some("run"));
        registry.unregister(Some("run"), &running);
        assert!(registry.cancel("run"));
        assert!(newer.load(Ordering::SeqCst));
        // Only the newest pending cancels are kept.
        for index in 0..=MAX_PENDING_CANCELS {
            assert!(!registry.cancel(&format!("pending-{index}")));
        }
        assert!(!registry.register(Some("pending-0")).load(Ordering::SeqCst));
        assert!(registry.register(Some("pending-1")).load(Ordering::SeqCst));
        // Without a request id a run can never be cancelled.
        assert!(!registry.register(None).load(Ordering::SeqCst));
    }

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
