mod supervisor;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::path::PathBuf;
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
    "ui.journals.snapshot",
    "ui.journals.quarantine",
    "ui.journals.transfers.list",
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
fn daemon_invoke(
    app: tauri::AppHandle,
    state: State<'_, Arc<DaemonSupervisor>>,
    request: DaemonRequest,
) -> DaemonEnvelope {
    if !ALLOWED_DAEMON_KINDS.contains(&request.kind.as_str()) {
        return error_envelope(
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
        );
    }

    let request_id = request.request_id.clone();
    let streaming = STREAMING_DAEMON_KINDS.contains(&request.kind.as_str());
    match state.invoke(
        &request.kind,
        request.args,
        &app,
        streaming,
        request.request_id,
    ) {
        Ok(response) => match serde_json::from_value(response) {
            Ok(envelope) => envelope,
            Err(error) => error_envelope(
                "daemon_protocol_error",
                format!("Python daemon response did not match the envelope contract: {error}"),
                Some("Check daemon smoke tests before wiring more UI kinds."),
                None,
                request_id.clone(),
                false,
            ),
        },
        Err(error) => supervisor_error_envelope(error, request_id),
    }
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
        .invoke_handler(tauri::generate_handler![daemon_invoke])
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
