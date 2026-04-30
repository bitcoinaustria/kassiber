mod supervisor;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use supervisor::{DaemonSupervisor, SupervisorError};
use tauri::{Manager, State};

const SCHEMA_VERSION: u8 = 1;

const ALLOWED_DAEMON_KINDS: &[&str] = &[
    "status",
    "ui.overview.snapshot",
    "ui.transactions.list",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.profiles.snapshot",
    "ui.reports.capital_gains",
    "ui.reports.export_pdf",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.journals.snapshot",
    "ui.journals.quarantine",
    "ui.journals.transfers.list",
    "ui.journals.process",
    "ui.rates.summary",
    "ui.workspace.health",
    "ui.workspace.delete",
    "ui.secrets.init",
    "ui.secrets.change_passphrase",
    "ui.next_actions",
    "ui.wallets.sync",
    "daemon.lock",
    "daemon.unlock",
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.delete",
    "ai.providers.set_default",
    "ai.providers.clear_default",
    "ai.providers.acknowledge",
    "ai.list_models",
    "ai.test_connection",
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
];

/// Kinds that may emit intermediate stream records (kind = "<request_kind>.delta",
/// "<request_kind>.tool_call", etc.) before the terminal envelope. The supervisor
/// forwards intermediate records to the webview as Tauri events
/// `daemon://stream` and switches to a per-record inactivity
/// timeout. Other kinds keep the existing total-budget behavior.
const STREAMING_DAEMON_KINDS: &[&str] = &["ai.chat"];

#[derive(Debug, Deserialize)]
pub struct DaemonRequest {
    kind: String,
    #[serde(default)]
    request_id: Option<Value>,
    #[serde(default)]
    args: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DaemonEnvelope {
    kind: String,
    schema_version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DaemonError>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DaemonError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Value>,
    retryable: bool,
}

#[tauri::command]
async fn daemon_invoke(
    app: tauri::AppHandle,
    state: State<'_, Arc<DaemonSupervisor>>,
    request: DaemonRequest,
) -> Result<DaemonEnvelope, DaemonEnvelope> {
    if !ALLOWED_DAEMON_KINDS.contains(&request.kind.as_str()) {
        return Ok(error_envelope(
            "kind_not_allowed",
            format!(
                "daemon kind {:?} is not allowed by the Tauri shell",
                request.kind
            ),
            Some(
                "Add the kind to the generated daemon allowlist before exposing it to the webview.",
            ),
            Some(json!({ "kind": request.kind })),
            request.request_id,
            false,
        ));
    }

    let request_id = request.request_id.clone();
    let streaming = STREAMING_DAEMON_KINDS.contains(&request.kind.as_str());
    let task_request_id = request_id.clone();
    let supervisor = Arc::clone(state.inner());
    let DaemonRequest {
        kind,
        request_id: client_request_id,
        args,
    } = request;
    match tauri::async_runtime::spawn_blocking(move || {
        match supervisor.invoke(&kind, args, &app, streaming, client_request_id) {
            Ok(response) => match serde_json::from_value(response) {
                Ok(envelope) => envelope,
                Err(error) => error_envelope(
                    "daemon_protocol_error",
                    format!("Python daemon response did not match the envelope contract: {error}"),
                    Some("Check daemon smoke tests before wiring more UI kinds."),
                    None,
                    task_request_id.clone(),
                    false,
                ),
            },
            Err(error) => supervisor_error_envelope(error, task_request_id),
        }
    })
    .await
    {
        Ok(envelope) => Ok(envelope),
        Err(error) => Ok(error_envelope(
            "daemon_task_failed",
            format!("Tauri daemon task failed before returning an envelope: {error}"),
            Some("Restart the desktop shell and check the daemon smoke tests."),
            None,
            request_id,
            true,
        )),
    }
}

#[tauri::command]
fn open_exported_file(path: String) -> Result<(), String> {
    let requested = PathBuf::from(path);
    if !requested.is_absolute() {
        return Err("Report export paths must be absolute.".to_string());
    }

    let canonical = std::fs::canonicalize(&requested)
        .map_err(|error| format!("Report export file could not be found: {error}"))?;
    let metadata = canonical
        .metadata()
        .map_err(|error| format!("Report export file could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("Only report export files can be opened.".to_string());
    }
    if !is_supported_export_file(&canonical) || !is_managed_report_export_path(&canonical) {
        return Err(
            "Only PDF, XLSX, and CSV files in Kassiber's managed report exports folder can be opened."
                .to_string(),
        );
    }

    open_with_default_app(&canonical)
}

fn error_envelope(
    code: &str,
    message: impl Into<String>,
    hint: Option<&str>,
    details: Option<Value>,
    request_id: Option<Value>,
    retryable: bool,
) -> DaemonEnvelope {
    DaemonEnvelope {
        kind: "error".to_string(),
        schema_version: SCHEMA_VERSION,
        request_id,
        data: None,
        error: Some(DaemonError {
            code: code.to_string(),
            message: message.into(),
            hint: hint.map(str::to_string),
            details,
            retryable,
        }),
    }
}

fn supervisor_error_envelope(error: SupervisorError, request_id: Option<Value>) -> DaemonEnvelope {
    error_envelope(
        error.code,
        error.message,
        error.hint,
        error.details,
        request_id,
        error.retryable,
    )
}

fn is_supported_export_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "pdf" | "xlsx" | "csv"
            )
        })
        .unwrap_or(false)
}

fn is_managed_report_export_path(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(grandparent) = parent.parent() else {
        return false;
    };
    parent.file_name().and_then(|name| name.to_str()) == Some("reports")
        && grandparent.file_name().and_then(|name| name.to_str()) == Some("exports")
}

fn open_with_default_app(path: &Path) -> Result<(), String> {
    let mut command = default_app_command(path);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open report export with the default app: {error}"))
}

#[cfg(target_os = "macos")]
fn default_app_command(path: &Path) -> Command {
    let mut command = Command::new("open");
    command.arg(path);
    command
}

#[cfg(target_os = "windows")]
fn default_app_command(path: &Path) -> Command {
    let mut command = Command::new("explorer");
    command.arg(path);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_app_command(path: &Path) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(path);
    command
}

pub fn run() {
    let cli_args = desktop_cli_args();
    tauri::Builder::default()
        .setup(move |app| {
            let resource_dir = app.path().resource_dir().ok();
            if let Some(args) = cli_args.as_ref() {
                let code = supervisor::run_cli(resource_dir.as_deref(), args.clone());
                std::process::exit(code);
            }
            app.manage(Arc::new(DaemonSupervisor::new(resource_dir)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![daemon_invoke, open_exported_file])
        .run(tauri::generate_context!())
        .expect("error while running Kassiber desktop shell");
}

fn desktop_cli_args() -> Option<Vec<String>> {
    let args: Vec<String> = env::args()
        .skip(1)
        .filter(|arg| !arg.starts_with("-psn_"))
        .collect();
    if args.is_empty() {
        return None;
    }

    if args[0] == "--cli" || args[0] == "cli" {
        return Some(args[1..].to_vec());
    }

    if exe_stem_is_kassiber() {
        return Some(args);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{is_managed_report_export_path, is_supported_export_file};
    use std::path::Path;

    #[test]
    fn managed_report_export_paths_are_narrowly_recognized() {
        assert!(is_managed_report_export_path(Path::new(
            "/Users/dev/.kassiber/exports/reports/report.pdf"
        )));
        assert!(!is_managed_report_export_path(Path::new(
            "/Users/dev/.kassiber/exports/report.pdf"
        )));
        assert!(!is_managed_report_export_path(Path::new(
            "/Users/dev/.kassiber/reports/export.pdf"
        )));
        assert!(!is_managed_report_export_path(Path::new(
            "/Users/dev/.kassiber/exports/reports/archive/report.pdf"
        )));
    }

    #[test]
    fn supported_export_files_are_limited_to_report_formats() {
        assert!(is_supported_export_file(Path::new("report.PDF")));
        assert!(is_supported_export_file(Path::new("report.xlsx")));
        assert!(is_supported_export_file(Path::new("report.csv")));
        assert!(!is_supported_export_file(Path::new("report.txt")));
        assert!(!is_supported_export_file(Path::new("report")));
    }
}

fn exe_stem_is_kassiber() -> bool {
    env::current_exe()
        .ok()
        .and_then(|path: PathBuf| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .map(|stem| stem.eq_ignore_ascii_case("kassiber"))
        .unwrap_or(false)
}
