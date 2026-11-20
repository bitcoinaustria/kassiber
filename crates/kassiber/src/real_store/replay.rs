//! Replay bundles (docs/reference/replay-bundle.md) for the CLI and the
//! daemon.
use super::reproduce::{arg, limit, py};
use kassiber_envelope::{py_str, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use std::path::{Path, PathBuf};

fn managed_path(data_root: &str, stamp: &str) -> Result<PathBuf, AppError> {
    let paths = kassiber_paths::Context::from_env()
        .and_then(|context| {
            context.resolve(&kassiber_paths::ResolveOptions {
                data_root: Some(PathBuf::from(data_root)),
                ..Default::default()
            })
        })
        .map_err(super::error)?;
    let root = Path::new(&paths.exports_root).join("diagnostics");
    std::fs::create_dir_all(&root).map_err(|error| {
        AppError::new(
            "io_error",
            format!("Could not create the exports directory: {error}"),
        )
    })?;
    let stamp: String = stamp
        .get(..19)
        .unwrap_or(stamp)
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'T')
        .map(|c| if c == 'T' { '-' } else { c })
        .collect();
    let base = format!("replay-{stamp}");
    let mut candidate = root.join(format!("{base}.{}", kassiber_replay::EXTENSION));
    let mut counter = 2;
    while candidate.exists() {
        candidate = root.join(format!("{base}-{counter}.{}", kassiber_replay::EXTENSION));
        counter += 1;
    }
    Ok(candidate)
}

/// Seal and write the selected profile's replay bundle. The CLI names
/// `file`; the desktop gets a managed file under `exports/diagnostics/`.
pub(super) fn export(
    conn: &Connection,
    data_root: &str,
    args: &Value,
    version: &str,
    sources: &impl kassiber_transfers::Sources,
) -> Result<Value, AppError> {
    if !arg(args, "confirm").is_some_and(kassiber_envelope::truthy) {
        return Err(
            AppError::new("confirmation_required", kassiber_replay::DISCLOSURE).with_hint(
                "Confirm that you understand what the bundle holds (--confirm) to export it.",
            ),
        );
    }
    let passphrase = match arg(args, "passphrase") {
        Some(Value::Str(text)) => text.as_str().unwrap_or_default().to_owned(),
        _ => {
            return Err(AppError::new(
                "invalid_passphrase",
                "A bundle passphrase is required",
            ))
        }
    };
    let passphrase = zeroize::Zeroizing::new(passphrase);
    kassiber_replay::check_passphrase(&passphrase)?;
    let now = sources.now(conn)?;
    let path = match arg(args, "file") {
        Some(file) => {
            let raw = py_str(file);
            let expanded = kassiber_paths::Context::from_env()
                .and_then(|context| context.expand_user(Path::new(&raw)))
                .map_err(super::error)?;
            std::path::absolute(expanded)
                .map_err(|error| AppError::new("io_error", error.to_string()))?
        }
        None => managed_path(data_root, &now)?,
    };
    if path.exists() {
        return Err(
            AppError::new("file_exists", "The bundle file already exists")
                .with_hint("Choose a new file name; Kassiber does not overwrite files."),
        );
    }
    let exported = kassiber_replay::export(
        conn,
        arg(args, "workspace"),
        arg(args, "profile"),
        &passphrase,
        &now,
        version,
        sources,
    )?;
    kassiber_replay::write_new(&path, &exported.sealed)?;
    let mut manifest = exported.manifest;
    // The digests the receiver checks; the parts stay inside the envelope.
    if let Some(object) = manifest.as_object_mut() {
        object.remove("includes");
        object.remove("excludes");
        if let Some(pseudonymization) = object
            .get_mut("pseudonymization")
            .and_then(|v| v.as_object_mut())
        {
            pseudonymization.remove("differences");
        }
    }
    py(&serde_json::json!({
        "file": path.to_string_lossy(),
        "bytes": exported.sealed.len(),
        "sha256": kassiber_replay::pseudonym::sha256_hex(&exported.sealed),
        "encrypted": true,
        "disclosure": kassiber_replay::DISCLOSURE,
        "manifest": manifest,
    }))
}

/// `diagnostics replay-bundle verify`: no book, configuration or network.
pub(super) fn verify_bundle(args: &Value) -> Result<Value, AppError> {
    let file = arg(args, "file").map(py_str).unwrap_or_default();
    let passphrase = match arg(args, "passphrase") {
        Some(Value::Str(text)) => {
            zeroize::Zeroizing::new(text.as_str().unwrap_or_default().to_owned())
        }
        _ => {
            return Err(AppError::new(
                "invalid_passphrase",
                "A bundle passphrase is required",
            ))
        }
    };
    let path = kassiber_paths::Context::from_env()
        .and_then(|context| context.expand_user(Path::new(&file)))
        .map_err(super::error)?;
    let sealed = std::fs::read(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => {
            AppError::new("not_found", "The bundle file does not exist")
        }
        _ => AppError::new("io_error", format!("Could not read the bundle: {error}")),
    })?;
    let report = kassiber_replay::verify(&sealed, &passphrase, limit(args)?)?;
    py(&report)
}
