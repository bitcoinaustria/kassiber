mod supervisor;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;
use supervisor::{DaemonSupervisor, SupervisorError};
use tauri::State;

const SCHEMA_VERSION: u8 = 1;

const ALLOWED_DAEMON_KINDS: &[&str] = &[
    "status",
    "ui.overview.snapshot",
    "ui.transactions.list",
    "ui.profiles.snapshot",
    "ui.reports.capital_gains",
];

#[derive(Debug, Deserialize)]
pub struct DaemonRequest {
    kind: String,
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
    state: State<'_, Mutex<DaemonSupervisor>>,
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
            false,
        );
    }

    let mut supervisor = match state.lock() {
        Ok(supervisor) => supervisor,
        Err(_) => {
            return error_envelope(
                "daemon_lock_poisoned",
                "daemon supervisor lock is poisoned",
                Some("Restart the Tauri shell."),
                None,
                true,
            )
        }
    };

    match supervisor.invoke(&request.kind, request.args) {
        Ok(response) => match serde_json::from_value(response) {
            Ok(envelope) => envelope,
            Err(error) => error_envelope(
                "daemon_protocol_error",
                format!("Python daemon response did not match the envelope contract: {error}"),
                Some("Check daemon smoke tests before wiring more UI kinds."),
                None,
                false,
            ),
        },
        Err(error) => supervisor_error_envelope(error),
    }
}

fn error_envelope(
    code: &str,
    message: impl Into<String>,
    hint: Option<&str>,
    details: Option<Value>,
    retryable: bool,
) -> DaemonEnvelope {
    DaemonEnvelope {
        kind: "error".to_string(),
        schema_version: SCHEMA_VERSION,
        request_id: None,
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

fn supervisor_error_envelope(error: SupervisorError) -> DaemonEnvelope {
    error_envelope(
        error.code,
        error.message,
        error.hint,
        error.details,
        error.retryable,
    )
}

pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(DaemonSupervisor::new()))
        .invoke_handler(tauri::generate_handler![daemon_invoke])
        .run(tauri::generate_context!())
        .expect("error while running Kassiber desktop shell");
}
