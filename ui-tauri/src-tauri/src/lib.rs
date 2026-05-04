mod supervisor;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::time::Duration;
use supervisor::{DaemonSupervisor, SupervisorError};
use tauri::{Manager, State};

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_STATE_DIR: &str = ".kassiber";
const DEFAULT_DATA_DIR: &str = "data";
const DB_FILENAMES: &[&str] = &["kassiber.sqlite3", "satbooks.sqlite3"];
const IMPORT_PICKER_TIMEOUT: Duration = Duration::from_secs(300);

const ALLOWED_DAEMON_KINDS: &[&str] = &[
    "status",
    "ui.overview.snapshot",
    "ui.transactions.list",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.profiles.snapshot",
    "ui.profiles.switch",
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProjectSelection {
    state_root: String,
    data_root: String,
    database: String,
    encrypted: bool,
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

#[tauri::command]
async fn select_import_project_directory() -> Result<Option<ImportProjectSelection>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let Some(selected) = choose_import_project_directory()? else {
            return Ok(None);
        };
        inspect_import_project_directory(&selected).map(Some)
    })
    .await
    .map_err(|error| format!("Project folder picker task failed: {error}"))?
}

#[tauri::command]
fn activate_import_project(
    state: State<'_, Arc<DaemonSupervisor>>,
    data_root: String,
) -> Result<ImportProjectSelection, String> {
    let selection = inspect_import_project_directory(Path::new(&data_root))?;
    state
        .set_data_root(PathBuf::from(&selection.data_root))
        .map_err(|error| error.message)?;
    Ok(selection)
}

#[tauri::command]
fn clear_import_project(state: State<'_, Arc<DaemonSupervisor>>) -> Result<(), String> {
    state.clear_data_root().map_err(|error| error.message)
}

fn inspect_import_project_directory(path: &Path) -> Result<ImportProjectSelection, String> {
    let canonical = path
        .expanduser()
        .canonicalize()
        .map_err(|error| format!("Kassiber project folder could not be opened: {error}"))?;
    let (data_root, database, encrypted) = resolve_import_data_root(&canonical)?.ok_or_else(|| {
        "Choose a Kassiber project folder containing data/kassiber.sqlite3, or choose the data folder itself."
            .to_string()
    })?;
    let state_root =
        if data_root.file_name().and_then(|name| name.to_str()) == Some(DEFAULT_DATA_DIR) {
            data_root
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| data_root.clone())
        } else {
            data_root.clone()
        };
    Ok(ImportProjectSelection {
        state_root: state_root.to_string_lossy().to_string(),
        data_root: data_root.to_string_lossy().to_string(),
        database: database.to_string_lossy().to_string(),
        encrypted,
    })
}

fn resolve_import_data_root(path: &Path) -> Result<Option<(PathBuf, PathBuf, bool)>, String> {
    let mut direct = None;
    for filename in DB_FILENAMES {
        let database = path.join(filename);
        if let Some(encrypted) = inspect_database_candidate(&database)? {
            direct = Some((path.to_path_buf(), database, encrypted));
            break;
        }
    }

    let nested_data_root = path.join(DEFAULT_DATA_DIR);
    let nested = if let Some(data_root) = inspect_data_root_candidate(&nested_data_root)? {
        let mut nested = None;
        for filename in DB_FILENAMES {
            let database = data_root.join(filename);
            if let Some(encrypted) = inspect_database_candidate(&database)? {
                nested = Some((data_root.clone(), database, encrypted));
                break;
            }
        }
        nested
    } else {
        None
    };

    match (direct, nested) {
        (Some(_), Some(_)) => Err(
            "Selected folder contains Kassiber databases both directly and under data/. Choose the exact data folder to import."
                .to_string(),
        ),
        (Some(selection), None) | (None, Some(selection)) => Ok(Some(selection)),
        (None, None) => Ok(None),
    }
}

fn command_output_with_timeout(mut command: Command, label: &str) -> Result<Output, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                format!("{label} program was not found.")
            } else {
                format!("Could not open {label}: {error}")
            }
        })?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("Could not read {label} output: {error}"));
            }
            Ok(None) => {
                if started.elapsed() >= IMPORT_PICKER_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{label} timed out."));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Could not inspect {label}: {error}"));
            }
        }
    }
}

fn inspect_data_root_candidate(path: &Path) -> Result<Option<PathBuf>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Kassiber data folder candidate could not be inspected: {error}"
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err("Kassiber data folders must not be symlinks.".to_string());
    }
    if !metadata.file_type().is_dir() {
        return Ok(None);
    }
    path.canonicalize()
        .map(Some)
        .map_err(|error| format!("Kassiber data folder could not be opened: {error}"))
}

fn inspect_database_candidate(path: &Path) -> Result<Option<bool>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Kassiber database candidate could not be inspected: {error}"
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err("Kassiber database files must not be symlinks.".to_string());
    }
    if !metadata.file_type().is_file() {
        return Ok(None);
    }
    if metadata.len() == 0 {
        return Err("Kassiber database file is empty.".to_string());
    }

    let encrypted = database_is_encrypted(path)
        .map_err(|error| format!("Kassiber database could not be inspected: {error}"))?;
    if !encrypted && !plaintext_database_looks_like_kassiber(path)? {
        return Err(
            "Selected SQLite file does not contain Kassiber workspace/profile tables.".to_string(),
        );
    }
    Ok(Some(encrypted))
}

fn database_is_encrypted(path: &Path) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0_u8; 16];
    let count = file.read(&mut header)?;
    if count == 0 {
        return Ok(false);
    }
    if count < header.len() {
        return Ok(true);
    }
    Ok(header != *b"SQLite format 3\0")
}

fn plaintext_database_looks_like_kassiber(path: &Path) -> Result<bool, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Kassiber database could not be read: {error}"))?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Kassiber database could not be read: {error}"))?;
    Ok(contains_ascii_case_insensitive(&bytes, b"create table")
        && contains_ascii_case_insensitive(&bytes, b"settings")
        && contains_ascii_case_insensitive(&bytes, b"workspaces")
        && contains_ascii_case_insensitive(&bytes, b"profiles")
        && contains_ascii_case_insensitive(&bytes, b"workspace_id")
        && contains_ascii_case_insensitive(&bytes, b"fiat_currency"))
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.to_ascii_lowercase() == right.to_ascii_lowercase())
    })
}

trait ExpandUser {
    fn expanduser(&self) -> PathBuf;
}

impl ExpandUser for Path {
    fn expanduser(&self) -> PathBuf {
        let raw = self.to_string_lossy();
        if raw == "~" || raw.starts_with("~/") {
            if let Some(home) = home_dir() {
                if raw == "~" {
                    return home;
                }
                return home.join(&raw[2..]);
            }
        }
        self.to_path_buf()
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn default_import_picker_root() -> PathBuf {
    let state_root = home_dir()
        .map(|home| home.join(DEFAULT_STATE_DIR))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_DIR));
    if state_root.exists() {
        state_root
    } else {
        home_dir().unwrap_or(state_root)
    }
}

fn choose_import_project_directory() -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        choose_import_project_directory_macos(&default_import_picker_root())
    }

    #[cfg(target_os = "windows")]
    {
        choose_import_project_directory_windows(&default_import_picker_root())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        choose_import_project_directory_unix(&default_import_picker_root())
    }
}

#[cfg(target_os = "macos")]
fn choose_import_project_directory_macos(default_root: &Path) -> Result<Option<PathBuf>, String> {
    let prompt = apple_script_string("Choose a Kassiber project folder");
    let default_clause = if default_root.exists() {
        format!(
            " default location POSIX file {}",
            apple_script_string(&default_root.to_string_lossy())
        )
    } else {
        String::new()
    };
    let script = format!(
        "try\nset chosenFolder to choose folder with prompt {prompt}{default_clause}\nreturn POSIX path of chosenFolder\non error number -128\nreturn \"__KASSIBER_CANCELLED__\"\nend try"
    );
    let mut command = Command::new("osascript");
    command.arg("-e").arg(script);
    let output = command_output_with_timeout(command, "the macOS folder picker")?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout == "__KASSIBER_CANCELLED__" {
        return Ok(None);
    }
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "The macOS folder picker failed.".to_string()
        } else {
            stderr
        });
    }
    if stdout.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(stdout)))
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "windows")]
fn choose_import_project_directory_windows(default_root: &Path) -> Result<Option<PathBuf>, String> {
    let default_path = powershell_single_quoted(&default_root.to_string_lossy());
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $dialog = New-Object System.Windows.Forms.FolderBrowserDialog; \
         $dialog.Description = 'Choose a Kassiber project folder'; \
         $dialog.ShowNewFolderButton = $false; \
         $dialog.SelectedPath = {default_path}; \
         if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) {{ \
           [Console]::Out.Write($dialog.SelectedPath) \
         }}"
    );
    let mut command = Command::new("powershell.exe");
    command
        .arg("-NoProfile")
        .arg("-STA")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script);
    let output = command_output_with_timeout(command, "the Windows folder picker")?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return if stdout.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(stdout)))
        };
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        "The Windows folder picker failed.".to_string()
    } else {
        stderr
    })
}

#[cfg(target_os = "windows")]
fn powershell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn choose_import_project_directory_unix(default_root: &Path) -> Result<Option<PathBuf>, String> {
    let title = "Choose a Kassiber project folder";
    let default_dir = picker_default_dir_arg(default_root);
    let attempts: [(&str, Vec<String>); 3] = [
        (
            "zenity",
            vec![
                "--file-selection".to_string(),
                "--directory".to_string(),
                "--title".to_string(),
                title.to_string(),
                "--filename".to_string(),
                default_dir.clone(),
            ],
        ),
        (
            "kdialog",
            vec![
                "--title".to_string(),
                title.to_string(),
                "--getexistingdirectory".to_string(),
                default_root.to_string_lossy().to_string(),
            ],
        ),
        (
            "yad",
            vec![
                "--file".to_string(),
                "--directory".to_string(),
                format!("--title={title}"),
                format!("--filename={default_dir}"),
            ],
        ),
    ];

    for (program, args) in attempts {
        if let Some(selection) = try_unix_folder_picker(program, &args)? {
            return Ok(selection);
        }
    }

    Err("No supported folder picker is available. Install zenity, kdialog, or yad to import a Kassiber project from the desktop app.".to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn try_unix_folder_picker(
    program: &str,
    args: &[String],
) -> Result<Option<Option<PathBuf>>, String> {
    let mut command = Command::new(program);
    command.args(args);
    let output = match command_output_with_timeout(command, &format!("{program} folder picker")) {
        Ok(output) => output,
        Err(error) if error.contains("program was not found") => return Ok(None),
        Err(error) => return Err(error),
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(Some(if stdout.is_empty() {
            None
        } else {
            Some(PathBuf::from(stdout))
        }));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stdout.is_empty() && stderr.is_empty() {
        return Ok(Some(None));
    }
    Err(if stderr.is_empty() {
        format!("The {program} folder picker failed.")
    } else {
        stderr
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn picker_default_dir_arg(path: &Path) -> String {
    let mut value = path.to_string_lossy().to_string();
    if !value.ends_with(std::path::MAIN_SEPARATOR) {
        value.push(std::path::MAIN_SEPARATOR);
    }
    value
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
        .invoke_handler(tauri::generate_handler![
            daemon_invoke,
            open_exported_file,
            select_import_project_directory,
            activate_import_project,
            clear_import_project
        ])
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
    use super::{
        database_is_encrypted, inspect_import_project_directory, is_managed_report_export_path,
        is_supported_export_file,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[test]
    fn import_project_accepts_state_root_with_data_dir() {
        let root = unique_temp_dir("state-root");
        let data = root.join("data");
        fs::create_dir_all(&data).expect("create data dir");
        fs::write(data.join("kassiber.sqlite3"), fake_kassiber_sqlite_bytes())
            .expect("write sqlite header");
        let root = root.canonicalize().expect("canonical root");
        let data = data.canonicalize().expect("canonical data");

        let selection = inspect_import_project_directory(&root).expect("inspect project");
        assert_eq!(selection.state_root, root.to_string_lossy().to_string());
        assert_eq!(selection.data_root, data.to_string_lossy().to_string());
        assert_eq!(
            selection.database,
            data.join("kassiber.sqlite3").to_string_lossy().to_string()
        );
        assert!(!selection.encrypted);
    }

    #[test]
    fn import_project_rejects_ambiguous_direct_and_nested_databases() {
        let root = unique_temp_dir("ambiguous-root");
        let data = root.join("data");
        fs::create_dir_all(&data).expect("create data dir");
        fs::write(root.join("kassiber.sqlite3"), fake_kassiber_sqlite_bytes())
            .expect("write root sqlite");
        fs::write(data.join("kassiber.sqlite3"), fake_kassiber_sqlite_bytes())
            .expect("write nested sqlite");

        let error = inspect_import_project_directory(&root)
            .expect_err("ambiguous import roots should be rejected");
        assert!(error.contains("both directly and under data/"));
    }

    #[test]
    fn import_project_accepts_data_root_directly() {
        let data = unique_temp_dir("data-root");
        fs::write(data.join("satbooks.sqlite3"), b"not a sqlite header")
            .expect("write encrypted-looking database");
        let data = data.canonicalize().expect("canonical data");

        let selection = inspect_import_project_directory(&data).expect("inspect project");
        assert_eq!(selection.state_root, data.to_string_lossy().to_string());
        assert_eq!(selection.data_root, data.to_string_lossy().to_string());
        assert_eq!(
            selection.database,
            data.join("satbooks.sqlite3").to_string_lossy().to_string()
        );
        assert!(selection.encrypted);
    }

    #[test]
    fn import_project_rejects_folders_without_kassiber_database() {
        let root = unique_temp_dir("missing-db");
        let error = inspect_import_project_directory(&root)
            .expect_err("folder without database should be rejected");
        assert!(error.contains("data/kassiber.sqlite3"));
    }

    #[test]
    fn import_project_rejects_empty_database_file() {
        let data = unique_temp_dir("empty-db");
        fs::write(data.join("kassiber.sqlite3"), b"").expect("write empty database");

        let error = inspect_import_project_directory(&data)
            .expect_err("empty database file should be rejected");
        assert!(error.contains("empty"));
    }

    #[test]
    fn import_project_rejects_unrelated_plaintext_sqlite() {
        let data = unique_temp_dir("unrelated-db");
        fs::write(
            data.join("kassiber.sqlite3"),
            b"SQLite format 3\0CREATE TABLE unrelated (id TEXT)",
        )
        .expect("write unrelated sqlite");

        let error = inspect_import_project_directory(&data)
            .expect_err("unrelated sqlite file should be rejected");
        assert!(error.contains("workspace/profile"));
    }

    #[cfg(unix)]
    #[test]
    fn import_project_rejects_database_symlink() {
        let data = unique_temp_dir("symlink-db");
        let target = data.join("target.sqlite3");
        fs::write(&target, fake_kassiber_sqlite_bytes()).expect("write target sqlite");
        std::os::unix::fs::symlink(&target, data.join("kassiber.sqlite3"))
            .expect("create database symlink");

        let error = inspect_import_project_directory(&data)
            .expect_err("database symlink should be rejected");
        assert!(error.contains("symlinks"));
    }

    #[cfg(unix)]
    #[test]
    fn import_project_rejects_data_folder_symlink() {
        let root = unique_temp_dir("symlink-data-root");
        let target = unique_temp_dir("symlink-data-target");
        fs::write(
            target.join("kassiber.sqlite3"),
            fake_kassiber_sqlite_bytes(),
        )
        .expect("write target sqlite");
        std::os::unix::fs::symlink(&target, root.join("data")).expect("create data folder symlink");

        let error = inspect_import_project_directory(&root)
            .expect_err("data folder symlink should be rejected");
        assert!(error.contains("data folders must not be symlinks"));

        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn sqlite_header_detection_treats_non_sqlite_as_encrypted() {
        let root = unique_temp_dir("header-detection");
        let sqlite = root.join("plain.sqlite3");
        let encrypted = root.join("encrypted.sqlite3");
        fs::write(&sqlite, b"SQLite format 3\0rest").expect("write sqlite");
        fs::write(&encrypted, b"ciphertext").expect("write encrypted");

        assert!(!database_is_encrypted(&sqlite).expect("read sqlite"));
        assert!(database_is_encrypted(&encrypted).expect("read encrypted"));
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "kassiber-ui-import-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn fake_kassiber_sqlite_bytes() -> &'static [u8] {
        b"SQLite format 3\0CREATE TABLE IF NOT EXISTS settings (key TEXT, value TEXT); CREATE TABLE IF NOT EXISTS workspaces (id TEXT, label TEXT); CREATE TABLE IF NOT EXISTS profiles (id TEXT, workspace_id TEXT, label TEXT, fiat_currency TEXT);"
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
