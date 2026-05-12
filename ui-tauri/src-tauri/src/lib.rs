mod supervisor;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use supervisor::{DaemonSupervisor, SupervisorError};
use tauri::menu::{AboutMetadata, Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager, State, Url};
use tauri_plugin_deep_link::DeepLinkExt;

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_STATE_DIR: &str = ".kassiber";
const DEFAULT_DATA_DIR: &str = "data";
const DB_FILENAMES: &[&str] = &["kassiber.sqlite3", "satbooks.sqlite3"];
const IMPORT_PICKER_TIMEOUT: Duration = Duration::from_secs(300);
// Event name kept on the existing `kassiber:` colon-prefixed convention so the
// channel can be reused for OS-level deep links (e.g. `kassiber://transaction/...`)
// without colliding with their URL form.
const MENU_EVENT: &str = "kassiber:intent";
const MENU_OPEN_SETTINGS: &str = "kassiber:settings";
const MENU_SETTINGS_GENERAL: &str = "kassiber:settings:general";
const MENU_SETTINGS_PRIVACY: &str = "kassiber:settings:privacy";
const MENU_SETTINGS_DISPLAY: &str = "kassiber:settings:display";
const MENU_SETTINGS_SECURITY: &str = "kassiber:settings:security";
const MENU_SETTINGS_BACKENDS: &str = "kassiber:settings:backends";
const MENU_SETTINGS_AI: &str = "kassiber:settings:ai";
const MENU_SETTINGS_DATA: &str = "kassiber:settings:data";
const MENU_LOCK_APP: &str = "kassiber:lock";
const MENU_TOGGLE_SENSITIVE: &str = "kassiber:toggle-sensitive";
const MENU_TOGGLE_FULLSCREEN: &str = "kassiber:window:toggle-fullscreen";
const MENU_WINDOW_CLOSE: &str = "kassiber:window:close";
const MENU_WINDOW_MINIMIZE: &str = "kassiber:window:minimize";
const MENU_WINDOW_ZOOM: &str = "kassiber:window:zoom";
const MENU_WINDOW_FOCUS: &str = "kassiber:window:focus";
const MENU_QUIT: &str = "kassiber:quit";
const MENU_HELP_DOCS: &str = "kassiber:help:docs";
const MENU_HELP_ISSUES: &str = "kassiber:help:issues";
const MENU_WORKFLOW_SYNC_ALL: &str = "kassiber:workflow:sync-all";
const MENU_WORKFLOW_PROCESS_JOURNALS: &str = "kassiber:workflow:process-journals";
const MENU_WORKFLOW_OPEN_REPORTS: &str = "kassiber:workflow:open-reports";
const MENU_WORKFLOW_CONNECTIONS_IMPORTS: &str = "kassiber:workflow:connections-imports";
const MENU_WORKFLOW_DATA_BACKUP: &str = "kassiber:workflow:data-backup";
const MENU_NAV_OVERVIEW: &str = "kassiber:navigate:overview";
const MENU_NAV_TRANSACTIONS: &str = "kassiber:navigate:transactions";
const MENU_NAV_CONNECTIONS: &str = "kassiber:navigate:connections";
const MENU_NAV_BOOKS: &str = "kassiber:navigate:books";
const MENU_NAV_REPORTS: &str = "kassiber:navigate:reports";
const MENU_NAV_SOURCE_FUNDS: &str = "kassiber:navigate:source-funds";
const MENU_NAV_JOURNALS: &str = "kassiber:navigate:journals";
const MENU_NAV_TAX_EVENTS: &str = "kassiber:navigate:tax-events";
const MENU_NAV_QUARANTINE: &str = "kassiber:navigate:quarantine";
const MENU_NAV_ASSISTANT: &str = "kassiber:navigate:assistant";
const MENU_NAV_DIAGNOSTICS: &str = "kassiber:navigate:diagnostics";
const DOCS_URL: &str = "https://github.com/bitcoinaustria/kassiber#readme";
const ISSUES_URL: &str = "https://github.com/bitcoinaustria/kassiber/issues";

const DEEP_LINK_SCHEME: &str = "kassiber";

// Public URL contract for `kassiber://`. Once `bundle.active` flips to true,
// every form below becomes part of the app's external API — emails, websites,
// and third-party tools will start linking against it. Treat additions as
// non-breaking and removals as breaking changes; rename via deprecation, not
// in-place edits.
//
// Currently supported forms (case-insensitive, host + first segment are
// normalized to ASCII lowercase before matching):
//   kassiber://<route>                        navigates to /<route>
//   kassiber://settings                       opens Settings (no section)
//   kassiber://settings/<section>             opens Settings, focuses panel
//   kassiber://workflow/sync-all              triggers wallet sync
//   kassiber://workflow/process-journals      rebuilds journal state
//   kassiber://lock                           locks the workspace
//
// Restricting hosts and sections to fixed allowlists keeps a malicious URL
// from deep-linking the user into an unintended route or section.

const DEEP_LINK_ROUTE_HOSTS: &[(&str, &str)] = &[
    ("overview", "/overview"),
    ("transactions", "/transactions"),
    ("connections", "/connections"),
    ("books", "/books"),
    ("reports", "/reports"),
    ("source-of-funds", "/source-of-funds"),
    ("journals", "/journals"),
    ("tax-events", "/tax-events"),
    ("quarantine", "/quarantine"),
    ("assistant", "/assistant"),
    ("diagnostics", "/diagnostics"),
];

// Mirrors the React `SETTINGS_SECTION_INTEGRATION` map in
// `ui-tauri/src/components/kb/settingsSections.ts`. Aliases (`sync` →
// backends, `assistant` → ai) are accepted at the deep-link boundary so the
// Rust allowlist matches the panel-resolution logic on the React side; the
// React helper does the final hash → integration-id lookup.
const DEEP_LINK_SETTINGS_SECTIONS: &[&str] = &[
    "privacy",
    "display",
    "security",
    "backends",
    "sync",
    "rates",
    "ai",
    "assistant",
    "data",
];

const ALLOWED_DAEMON_KINDS: &[&str] = &[
    "status",
    "ui.overview.snapshot",
    "ui.transactions.list",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.backends.options",
    "ui.backends.electrum.test",
    "ui.profiles.snapshot",
    "ui.profiles.create",
    "ui.profiles.switch",
    "ui.reports.capital_gains",
    "ui.reports.export_pdf",
    "ui.reports.export_csv",
    "ui.reports.export_xlsx",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.journals.snapshot",
    "ui.journals.quarantine",
    "ui.journals.transfers.list",
    "ui.journals.process",
    "ui.rates.summary",
    "ui.workspace.health",
    "ui.workspace.create",
    "ui.workspace.delete",
    "ui.secrets.init",
    "ui.secrets.change_passphrase",
    "ui.next_actions",
    "ui.wallets.create",
    "ui.wallets.preview_descriptor",
    "ui.connections.sources",
    "ui.connections.btcpay.create",
    "ui.connections.btcpay.discover",
    "ui.connections.btcpay.test",
    "ui.metadata.bip329.import",
    "ui.wallets.update",
    "ui.wallets.delete",
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
    "ui.source_funds.preview",
    "ui.source_funds.cases.save",
    "ui.source_funds.cases.list",
    "ui.source_funds.sources.list",
    "ui.source_funds.sources.create",
    "ui.source_funds.sources.attach",
    "ui.source_funds.links.list",
    "ui.source_funds.links.create",
    "ui.source_funds.links.review",
    "ui.source_funds.links.bulk_review",
    "ui.source_funds.links.attach",
    "ui.source_funds.suggest",
    "ui.source_funds.evidence.list",
    "ui.source_funds.export_pdf",
    "ui.source_funds.coverage",
    "ui.source_funds.recipients.list",
    "ui.source_funds.recipients.create",
    "ui.source_funds.recipients.update",
    "ui.source_funds.recipients.delete",
];

/// Kinds that may emit intermediate stream records (kind = "<request_kind>.delta",
/// "<request_kind>.tool_call", etc.) before the terminal envelope. The supervisor
/// forwards intermediate records to the webview as Tauri events
/// `daemon://stream` and switches to a per-record inactivity
/// timeout. Other kinds keep the existing total-budget behavior.
const STREAMING_DAEMON_KINDS: &[&str] = &["ai.chat", "ui.wallets.sync"];

// Daemon kinds that exercise the AI runtime (model calls, chat sessions, tool
// consent prompts). Gated server-side by the global AI features toggle so the
// switch is a real privacy promise instead of just hiding the UI — every
// future caller can't accidentally bypass the guard. `ai.providers.*` is
// deliberately excluded: providers stay configurable while AI is off so the
// user can wire keys before turning the feature on.
const AI_RUNTIME_KINDS: &[&str] = &[
    "ai.list_models",
    "ai.test_connection",
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
];

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct MenuActionPayload {
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    route: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    section: Option<&'static str>,
}

struct AppMenuHandles {
    assistant: MenuItem<tauri::Wry>,
    // Menu items that only make sense once the user has unlocked a workspace
    // (`identity` is set). React notifies us via `set_menu_state` so the
    // corresponding native menu items grey out instead of bouncing the user
    // back to the Welcome screen mid-action.
    workspace_gated: Vec<MenuItem<tauri::Wry>>,
}

struct AppRuntimeState {
    ai_features_enabled: AtomicBool,
}

impl AppRuntimeState {
    fn new() -> Self {
        // Fail-safe default: assume AI is off until React explicitly tells
        // us otherwise via `set_menu_state`. A few `ai_features_disabled`
        // envelopes during the first ~50ms after startup are recoverable;
        // a silently-broken kill switch (bad invoke binding, JS error
        // before the effect runs) defaulting to "AI on" is not. The user's
        // explicit "AI on" intent must always cross the React→Rust
        // boundary before AI runtime kinds run.
        Self {
            ai_features_enabled: AtomicBool::new(false),
        }
    }
}

#[tauri::command]
async fn daemon_invoke(
    app: tauri::AppHandle,
    state: State<'_, Arc<DaemonSupervisor>>,
    runtime: State<'_, AppRuntimeState>,
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

    // Server-side enforcement of the global AI features toggle. Even if a
    // future caller forgets the React-side guard, AI runtime kinds will get
    // refused while the toggle is off — including streaming `ai.chat` and
    // its cancel/consent helpers. Provider configuration (`ai.providers.*`)
    // stays available so users can wire keys before turning AI on.
    if AI_RUNTIME_KINDS.contains(&request.kind.as_str())
        && !runtime.ai_features_enabled.load(Ordering::Relaxed)
    {
        return Ok(error_envelope(
            "ai_features_disabled",
            "AI features are disabled in Settings.",
            Some("Enable AI features in Settings before invoking AI runtime kinds."),
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
fn open_external_url(url: String) -> Result<(), String> {
    let validated = validated_external_url(&url)?;
    open_url_with_default_browser(&validated)
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
        (Some(_), Some(selection)) if is_managed_state_root(path) => Ok(Some(selection)),
        (Some(_), Some(_)) => Err(
            "Selected folder contains Kassiber databases both directly and under data/. Choose the exact data folder to import."
                .to_string(),
        ),
        (Some(selection), None) | (None, Some(selection)) => Ok(Some(selection)),
        (None, None) => Ok(None),
    }
}

// A managed Kassiber state root looks like `<...>/.kassiber/{config,data}/...`,
// so it always has a sibling `data/kassiber.sqlite3` *and* may carry a legacy
// `kassiber.sqlite3` at the top level from earlier daemon versions. The strict
// "ambiguous selection" error is meant for ad-hoc folders the user assembled
// by hand — when we recognize the managed layout we transparently prefer the
// nested `data/` database instead of asking them to drill in by one level.
fn is_managed_state_root(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some(DEFAULT_STATE_DIR)
        || path.join("config").join("settings.json").is_file()
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

fn validated_external_url(url: &str) -> Result<String, String> {
    let parsed = Url::parse(url.trim())
        .map_err(|_| "Only absolute HTTP or HTTPS explorer URLs can be opened.".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Only HTTP or HTTPS explorer URLs can be opened.".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("Explorer URLs must include a host.".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Explorer URLs with embedded credentials cannot be opened.".to_string());
    }
    Ok(parsed.as_str().to_string())
}

fn open_url_with_default_browser(url: &str) -> Result<(), String> {
    let mut command = default_browser_command(url);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open explorer URL with the default browser: {error}"))
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

#[cfg(target_os = "macos")]
fn default_browser_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.arg(url);
    command
}

#[cfg(target_os = "windows")]
fn default_browser_command(url: &str) -> Command {
    let mut command = Command::new("explorer");
    command.arg(url);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_browser_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(url);
    command
}

pub fn run() {
    let cli_args = desktop_cli_args();
    let mut builder = tauri::Builder::default();

    // Single-instance must come before the deep-link plugin so a second
    // launch (`open kassiber://settings/privacy` while the app is already
    // running) is forwarded to the existing window instead of forking a new
    // process. With a separate process we'd race the SQLite/SQLCipher
    // database — the daemon assumes one writer at a time.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(
            |app, _args, _cwd| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            },
        ));
    }

    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(move |app| {
            let resource_dir = app.path().resource_dir().ok();
            if let Some(args) = cli_args.as_ref() {
                let code = supervisor::run_cli(resource_dir.as_deref(), args.clone());
                std::process::exit(code);
            }
            let (menu, menu_handles) = build_app_menu(app.handle())?;
            app.set_menu(menu)?;
            app.manage(menu_handles);
            app.manage(AppRuntimeState::new());
            app.manage(Arc::new(DaemonSupervisor::new(resource_dir)));

            // Linux/Windows need an explicit runtime register; macOS uses
            // CFBundleURLTypes from the bundle config. `register` is a no-op
            // on macOS and harmless when the scheme is already registered.
            #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
            {
                let _ = app.deep_link().register(DEEP_LINK_SCHEME);
            }

            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    if let Some(payload) = menu_action_for_deep_link(&url) {
                        emit_menu_action(&app_handle, payload);
                    } else {
                        eprintln!("kassiber: ignoring unrecognized deep link: {url}");
                    }
                }
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            });

            // Cold-start case: if Kassiber was launched *from* a deep link
            // (`open kassiber://settings/privacy` while not running),
            // `on_open_url` is never called for that URL — Tauri delivers
            // launch URLs through `get_current` instead. Defer the emit
            // until the webview has had a chance to mount the intent
            // listener; otherwise the event fires into the void.
            if let Ok(Some(initial_urls)) = app.deep_link().get_current() {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tauri::async_runtime::spawn_blocking(|| {
                        std::thread::sleep(Duration::from_millis(800));
                    })
                    .await
                    .ok();
                    for url in initial_urls {
                        if let Some(payload) = menu_action_for_deep_link(&url) {
                            emit_menu_action(&app_handle, payload);
                        } else {
                            eprintln!(
                                "kassiber: ignoring unrecognized launch deep link: {url}"
                            );
                        }
                    }
                });
            }
            Ok(())
        })
        .on_menu_event(handle_app_menu_event)
        .invoke_handler(tauri::generate_handler![
            daemon_invoke,
            open_exported_file,
            open_external_url,
            select_import_project_directory,
            activate_import_project,
            clear_import_project,
            set_menu_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kassiber desktop shell");
}

fn menu_action_for_deep_link(url: &Url) -> Option<MenuActionPayload> {
    if !url.scheme().eq_ignore_ascii_case(DEEP_LINK_SCHEME) {
        return None;
    }

    // `url::Url` already normalizes the host to lowercase; the path is left
    // case-sensitive so we lowercase ourselves. URLs in emails / chat /
    // mailing lists are routinely auto-capitalized, so treating
    // `kassiber://settings/Privacy` differently from `.../privacy` would be
    // a footgun once the scheme is public API.
    let host = url.host_str()?;
    let segments: Vec<String> = url
        .path_segments()
        .map(|iter| {
            iter.filter(|segment| !segment.is_empty())
                .map(|segment| segment.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let first_segment = segments.first().map(String::as_str);

    match host {
        "lock" if segments.is_empty() => Some(menu_action("lock-app")),
        "settings" => {
            let section = first_segment.and_then(deep_link_settings_section);
            Some(open_settings_action(section))
        }
        "workflow" => match first_segment {
            Some("sync-all") | Some("sync") => Some(menu_action("sync-all-wallets")),
            Some("process-journals") => Some(menu_action("process-journals")),
            _ => None,
        },
        host if segments.is_empty() => deep_link_route(host).map(navigate_action),
        _ => None,
    }
}

fn deep_link_route(host: &str) -> Option<&'static str> {
    DEEP_LINK_ROUTE_HOSTS
        .iter()
        .find(|(slug, _)| *slug == host)
        .map(|(_, route)| *route)
}

fn deep_link_settings_section(section: &str) -> Option<&'static str> {
    DEEP_LINK_SETTINGS_SECTIONS
        .iter()
        .copied()
        .find(|known| *known == section)
}

fn build_app_menu(
    app: &tauri::AppHandle<tauri::Wry>,
) -> tauri::Result<(Menu<tauri::Wry>, AppMenuHandles)> {
    let settings_item = menu_item(app, MENU_OPEN_SETTINGS, "Settings...", Some("CmdOrCtrl+,"))?;
    let general_settings = menu_item(app, MENU_SETTINGS_GENERAL, "General", None)?;
    let privacy_settings = menu_item(app, MENU_SETTINGS_PRIVACY, "Privacy", None)?;
    let display_settings = menu_item(app, MENU_SETTINGS_DISPLAY, "Display", None)?;
    let security_settings = menu_item(app, MENU_SETTINGS_SECURITY, "Security", None)?;
    let backends_settings = menu_item(app, MENU_SETTINGS_BACKENDS, "Sync Backends", None)?;
    let ai_settings = menu_item(app, MENU_SETTINGS_AI, "AI Providers", None)?;
    let data_settings = menu_item(app, MENU_SETTINGS_DATA, "Local Data", None)?;
    let lock_item = menu_item(app, MENU_LOCK_APP, "Lock Kassiber", Some("CmdOrCtrl+L"))?;
    let close_item = menu_item(app, MENU_WINDOW_CLOSE, "Close Window", Some("CmdOrCtrl+W"))?;
    #[cfg(not(target_os = "macos"))]
    let quit_item = menu_item(app, MENU_QUIT, "Quit Kassiber", Some("CmdOrCtrl+Q"))?;
    let toggle_sensitive = menu_item(app, MENU_TOGGLE_SENSITIVE, "Toggle Sensitive Values", None)?;
    let toggle_fullscreen = menu_item(
        app,
        MENU_TOGGLE_FULLSCREEN,
        "Toggle Full Screen",
        Some("F11"),
    )?;
    let minimize_item = menu_item(app, MENU_WINDOW_MINIMIZE, "Minimize", Some("CmdOrCtrl+M"))?;
    let zoom_item = menu_item(app, MENU_WINDOW_ZOOM, "Zoom", None)?;
    let focus_item = menu_item(app, MENU_WINDOW_FOCUS, "Bring Main Window to Front", None)?;
    let docs_item = menu_item(app, MENU_HELP_DOCS, "Kassiber Documentation", None)?;
    let issues_item = menu_item(app, MENU_HELP_ISSUES, "Report an Issue", None)?;
    let diagnostics_item = menu_item(app, MENU_NAV_DIAGNOSTICS, "Diagnostics", None)?;
    let sync_all_item = menu_item(
        app,
        MENU_WORKFLOW_SYNC_ALL,
        "Sync All Wallets",
        Some("CmdOrCtrl+Shift+S"),
    )?;
    let process_journals_item = menu_item(
        app,
        MENU_WORKFLOW_PROCESS_JOURNALS,
        "Process Journals",
        Some("CmdOrCtrl+Shift+J"),
    )?;
    let open_reports_item = menu_item(
        app,
        MENU_WORKFLOW_OPEN_REPORTS,
        "Reports & Export...",
        Some("CmdOrCtrl+Shift+E"),
    )?;
    let workflow_connections_item = menu_item(
        app,
        MENU_WORKFLOW_CONNECTIONS_IMPORTS,
        "Connections & Imports...",
        None,
    )?;
    let workflow_data_item = menu_item(
        app,
        MENU_WORKFLOW_DATA_BACKUP,
        "Local Data & Backup...",
        None,
    )?;

    let overview_item = menu_item(app, MENU_NAV_OVERVIEW, "Overview", Some("CmdOrCtrl+1"))?;
    let transactions_item = menu_item(
        app,
        MENU_NAV_TRANSACTIONS,
        "Transactions",
        Some("CmdOrCtrl+2"),
    )?;
    let connections_item = menu_item(
        app,
        MENU_NAV_CONNECTIONS,
        "Connections",
        Some("CmdOrCtrl+3"),
    )?;
    let books_item = menu_item(app, MENU_NAV_BOOKS, "Books", Some("CmdOrCtrl+4"))?;
    let reports_item = menu_item(app, MENU_NAV_REPORTS, "Reports", Some("CmdOrCtrl+5"))?;
    let source_funds_item = menu_item(
        app,
        MENU_NAV_SOURCE_FUNDS,
        "Source of Funds",
        Some("CmdOrCtrl+6"),
    )?;
    let journals_item = menu_item(app, MENU_NAV_JOURNALS, "Journals", Some("CmdOrCtrl+7"))?;
    let tax_events_item = menu_item(app, MENU_NAV_TAX_EVENTS, "Tax Events", None)?;
    let quarantine_item = menu_item(app, MENU_NAV_QUARANTINE, "Quarantine", Some("CmdOrCtrl+8"))?;
    let assistant_item = menu_item(app, MENU_NAV_ASSISTANT, "Assistant", Some("CmdOrCtrl+9"))?;

    #[cfg(target_os = "macos")]
    let app_menu = SubmenuBuilder::new(app, "Kassiber")
        .about(Some(about_metadata(app)))
        .separator()
        .item(&settings_item)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    #[cfg(target_os = "macos")]
    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&lock_item)
        .separator()
        .item(&close_item)
        .build()?;

    #[cfg(not(target_os = "macos"))]
    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&settings_item)
        .separator()
        .item(&lock_item)
        .separator()
        .item(&close_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&overview_item)
        .item(&transactions_item)
        .item(&connections_item)
        .item(&books_item)
        .item(&reports_item)
        .item(&source_funds_item)
        .separator()
        .item(&journals_item)
        .item(&tax_events_item)
        .item(&quarantine_item)
        .item(&assistant_item)
        .separator()
        .item(&toggle_sensitive)
        .separator()
        .item(&toggle_fullscreen)
        .build()?;

    let workflow_menu = SubmenuBuilder::new(app, "Workflows")
        .item(&sync_all_item)
        .item(&process_journals_item)
        .separator()
        .item(&open_reports_item)
        .item(&workflow_connections_item)
        .item(&workflow_data_item)
        .build()?;

    let settings_menu = SubmenuBuilder::new(app, "Settings")
        .item(&general_settings)
        .separator()
        .item(&privacy_settings)
        .item(&display_settings)
        .item(&security_settings)
        .item(&backends_settings)
        .item(&ai_settings)
        .item(&data_settings)
        .build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .item(&minimize_item)
        .item(&zoom_item)
        .separator()
        .item(&focus_item)
        .build()?;

    #[cfg(target_os = "macos")]
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&docs_item)
        .item(&diagnostics_item)
        .separator()
        .item(&issues_item)
        .build()?;

    #[cfg(not(target_os = "macos"))]
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&docs_item)
        .item(&diagnostics_item)
        .separator()
        .item(&issues_item)
        .separator()
        .about(Some(about_metadata(app)))
        .build()?;

    let mut menu_builder = MenuBuilder::new(app);
    #[cfg(target_os = "macos")]
    {
        menu_builder = menu_builder.item(&app_menu);
    }
    let menu = menu_builder
        .item(&file_menu)
        .item(&edit_menu)
        .item(&view_menu)
        .item(&workflow_menu)
        .item(&settings_menu)
        .item(&window_menu)
        .item(&help_menu)
        .build()?;

    let workspace_gated = vec![
        lock_item.clone(),
        sync_all_item.clone(),
        process_journals_item.clone(),
        open_reports_item.clone(),
        workflow_connections_item.clone(),
        workflow_data_item.clone(),
        // View-menu navigation items: clicking these from the Welcome screen
        // would redirect right back via the identity-guard effect, so grey
        // them out instead. Diagnostics + Settings items remain enabled
        // because Diagnostics is meant to be reachable while triaging a
        // broken workspace and Settings has its own no-identity render.
        overview_item.clone(),
        transactions_item.clone(),
        connections_item.clone(),
        books_item.clone(),
        reports_item.clone(),
        source_funds_item.clone(),
        journals_item.clone(),
        tax_events_item.clone(),
        quarantine_item.clone(),
    ];

    let handles = AppMenuHandles {
        assistant: assistant_item,
        workspace_gated,
    };

    Ok((menu, handles))
}

#[tauri::command]
fn set_menu_state(
    handles: tauri::State<'_, AppMenuHandles>,
    runtime: tauri::State<'_, AppRuntimeState>,
    ai_features_enabled: bool,
    has_workspace: bool,
    locked: bool,
) -> Result<(), String> {
    // While locked the daemon refuses mutating calls, so workflow menu items
    // would only ever produce auth_required errors. Disable them alongside
    // the no-workspace case to keep the menu state aligned with what the user
    // can actually do right now.
    let workflows_enabled = has_workspace && !locked;
    let assistant_enabled = ai_features_enabled && workflows_enabled;
    handles
        .assistant
        .set_enabled(assistant_enabled)
        .map_err(|error| error.to_string())?;
    for item in &handles.workspace_gated {
        item.set_enabled(workflows_enabled)
            .map_err(|error| error.to_string())?;
    }
    runtime
        .ai_features_enabled
        .store(ai_features_enabled, Ordering::Relaxed);
    Ok(())
}

fn menu_item(
    app: &tauri::AppHandle<tauri::Wry>,
    id: &'static str,
    text: &'static str,
    accelerator: Option<&'static str>,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    let mut builder = MenuItemBuilder::with_id(id, text);
    if let Some(accelerator) = accelerator {
        builder = builder.accelerator(accelerator);
    }
    builder.build(app)
}

fn about_metadata(app: &tauri::AppHandle<tauri::Wry>) -> AboutMetadata<'static> {
    let pkg_info = app.package_info();
    let config = app.config();
    AboutMetadata {
        name: Some("Kassiber".to_string()),
        version: Some(pkg_info.version.to_string()),
        authors: Some(vec!["Bitcoin Austria".to_string()]),
        comments: Some("Local-first Bitcoin accounting.".to_string()),
        copyright: config.bundle.copyright.clone(),
        license: Some("AGPL-3.0-only".to_string()),
        website: Some("https://github.com/bitcoinaustria/kassiber".to_string()),
        website_label: Some("Kassiber on GitHub".to_string()),
        ..Default::default()
    }
}

fn handle_app_menu_event(app: &tauri::AppHandle<tauri::Wry>, event: tauri::menu::MenuEvent) {
    let id = event.id().as_ref();
    if let Some(payload) = menu_action_for_id(id) {
        emit_menu_action(app, payload);
        return;
    }

    match id {
        MENU_TOGGLE_FULLSCREEN => toggle_main_window_fullscreen(app),
        MENU_WINDOW_CLOSE => with_main_window(app, |window| window.close()),
        MENU_WINDOW_MINIMIZE => with_main_window(app, |window| window.minimize()),
        MENU_WINDOW_ZOOM => with_main_window(app, |window| {
            if window.is_maximized().unwrap_or(false) {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }),
        MENU_WINDOW_FOCUS => with_main_window(app, |window| window.set_focus()),
        MENU_QUIT => app.exit(0),
        MENU_HELP_DOCS => open_menu_url(DOCS_URL),
        MENU_HELP_ISSUES => open_menu_url(ISSUES_URL),
        _ => {}
    }
}

fn menu_action_for_id(id: &str) -> Option<MenuActionPayload> {
    match id {
        MENU_OPEN_SETTINGS | MENU_SETTINGS_GENERAL => Some(open_settings_action(None)),
        MENU_SETTINGS_PRIVACY => Some(open_settings_action(Some("privacy"))),
        MENU_SETTINGS_DISPLAY => Some(open_settings_action(Some("display"))),
        MENU_SETTINGS_SECURITY => Some(open_settings_action(Some("security"))),
        MENU_SETTINGS_BACKENDS => Some(open_settings_action(Some("backends"))),
        MENU_SETTINGS_AI => Some(open_settings_action(Some("ai"))),
        MENU_SETTINGS_DATA => Some(open_settings_action(Some("data"))),
        MENU_LOCK_APP => Some(menu_action("lock-app")),
        MENU_TOGGLE_SENSITIVE => Some(menu_action("toggle-sensitive")),
        MENU_WORKFLOW_SYNC_ALL => Some(menu_action("sync-all-wallets")),
        MENU_WORKFLOW_PROCESS_JOURNALS => Some(menu_action("process-journals")),
        MENU_WORKFLOW_OPEN_REPORTS => Some(navigate_action("/reports")),
        MENU_WORKFLOW_CONNECTIONS_IMPORTS => Some(navigate_action("/connections")),
        MENU_WORKFLOW_DATA_BACKUP => Some(open_settings_action(Some("data"))),
        MENU_NAV_OVERVIEW => Some(navigate_action("/overview")),
        MENU_NAV_TRANSACTIONS => Some(navigate_action("/transactions")),
        MENU_NAV_CONNECTIONS => Some(navigate_action("/connections")),
        MENU_NAV_BOOKS => Some(navigate_action("/books")),
        MENU_NAV_REPORTS => Some(navigate_action("/reports")),
        MENU_NAV_SOURCE_FUNDS => Some(navigate_action("/source-of-funds")),
        MENU_NAV_JOURNALS => Some(navigate_action("/journals")),
        MENU_NAV_TAX_EVENTS => Some(navigate_action("/tax-events")),
        MENU_NAV_QUARANTINE => Some(navigate_action("/quarantine")),
        MENU_NAV_ASSISTANT => Some(navigate_action("/assistant")),
        MENU_NAV_DIAGNOSTICS => Some(navigate_action("/diagnostics")),
        _ => None,
    }
}

fn menu_action(action: &'static str) -> MenuActionPayload {
    MenuActionPayload {
        action,
        route: None,
        section: None,
    }
}

fn open_settings_action(section: Option<&'static str>) -> MenuActionPayload {
    MenuActionPayload {
        action: "open-settings",
        route: None,
        section,
    }
}

fn navigate_action(route: &'static str) -> MenuActionPayload {
    MenuActionPayload {
        action: "navigate",
        route: Some(route),
        section: None,
    }
}

fn emit_menu_action(app: &tauri::AppHandle<tauri::Wry>, payload: MenuActionPayload) {
    if let Err(error) = app.emit(MENU_EVENT, payload) {
        eprintln!("kassiber: failed to emit menu action: {error}");
    }
}

fn toggle_main_window_fullscreen(app: &tauri::AppHandle<tauri::Wry>) {
    with_main_window(app, |window| {
        let fullscreen = window.is_fullscreen().unwrap_or(false);
        window.set_fullscreen(!fullscreen)
    });
}

fn with_main_window<F>(app: &tauri::AppHandle<tauri::Wry>, action: F)
where
    F: FnOnce(tauri::WebviewWindow<tauri::Wry>) -> tauri::Result<()>,
{
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = action(window) {
        eprintln!("kassiber: menu window action failed: {error}");
    }
}

fn open_menu_url(url: &str) {
    if let Err(error) = open_url_with_default_browser(url) {
        eprintln!("kassiber: failed to open menu URL: {error}");
    }
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
        is_supported_export_file, menu_action, menu_action_for_deep_link, menu_action_for_id,
        navigate_action, open_settings_action, validated_external_url, ALLOWED_DAEMON_KINDS,
        MENU_HELP_DOCS, MENU_LOCK_APP, MENU_NAV_ASSISTANT, MENU_NAV_REPORTS, MENU_SETTINGS_SECURITY,
        MENU_TOGGLE_FULLSCREEN, MENU_WORKFLOW_CONNECTIONS_IMPORTS, MENU_WORKFLOW_OPEN_REPORTS,
        MENU_WORKFLOW_PROCESS_JOURNALS, MENU_WORKFLOW_SYNC_ALL,
    };
    use tauri::Url;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn source_funds_daemon_kinds_are_in_allowlist() {
        // The Tauri shell forwards only kinds in ALLOWED_DAEMON_KINDS to
        // the daemon. If a UI daemon kind is missing here, packaged
        // desktop mode returns kind_not_allowed and the feature breaks
        // silently. Pin the source-funds set so future additions to
        // SourceFunds.tsx come with an explicit allowlist update.
        let required: &[&str] = &[
            "ui.source_funds.preview",
            "ui.source_funds.cases.save",
            "ui.source_funds.cases.list",
            "ui.source_funds.sources.list",
            "ui.source_funds.sources.create",
            "ui.source_funds.sources.attach",
            "ui.source_funds.links.list",
            "ui.source_funds.links.create",
            "ui.source_funds.links.review",
            "ui.source_funds.links.bulk_review",
            "ui.source_funds.links.attach",
            "ui.source_funds.suggest",
            "ui.source_funds.evidence.list",
            "ui.source_funds.export_pdf",
            "ui.source_funds.coverage",
            "ui.source_funds.recipients.list",
            "ui.source_funds.recipients.create",
            "ui.source_funds.recipients.update",
            "ui.source_funds.recipients.delete",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn native_menu_ids_map_to_webview_actions() {
        assert_eq!(
            menu_action_for_id(MENU_SETTINGS_SECURITY),
            Some(open_settings_action(Some("security")))
        );
        assert_eq!(
            menu_action_for_id(MENU_NAV_REPORTS),
            Some(navigate_action("/reports"))
        );
        assert_eq!(
            menu_action_for_id(MENU_NAV_ASSISTANT),
            Some(navigate_action("/assistant"))
        );
        assert_eq!(
            menu_action_for_id(MENU_LOCK_APP),
            Some(menu_action("lock-app"))
        );
        assert_eq!(
            menu_action_for_id(MENU_WORKFLOW_SYNC_ALL),
            Some(menu_action("sync-all-wallets"))
        );
        assert_eq!(
            menu_action_for_id(MENU_WORKFLOW_PROCESS_JOURNALS),
            Some(menu_action("process-journals"))
        );
        // Reports & Export navigates to the Reports screen instead of running
        // a context-free export from the menu, so the user keeps tax-year
        // context and the Open step stays explicit.
        assert_eq!(
            menu_action_for_id(MENU_WORKFLOW_OPEN_REPORTS),
            Some(navigate_action("/reports"))
        );
        assert_eq!(
            menu_action_for_id(MENU_WORKFLOW_CONNECTIONS_IMPORTS),
            Some(navigate_action("/connections"))
        );
        assert_eq!(menu_action_for_id(MENU_TOGGLE_FULLSCREEN), None);
        assert_eq!(menu_action_for_id(MENU_HELP_DOCS), None);
    }

    #[test]
    fn deep_links_route_to_known_actions() {
        let parse = |s: &str| menu_action_for_deep_link(&Url::parse(s).unwrap());

        assert_eq!(
            parse("kassiber://transactions"),
            Some(navigate_action("/transactions"))
        );
        assert_eq!(
            parse("kassiber://source-of-funds"),
            Some(navigate_action("/source-of-funds"))
        );
        assert_eq!(
            parse("kassiber://settings"),
            Some(open_settings_action(None))
        );
        assert_eq!(
            parse("kassiber://settings/privacy"),
            Some(open_settings_action(Some("privacy")))
        );
        // Unknown sections degrade to "open settings" without a section
        // rather than failing — the user still arrives at the right surface
        // and the menu fallback already handles the missing-hash case.
        assert_eq!(
            parse("kassiber://settings/wallet-of-satoshi"),
            Some(open_settings_action(None))
        );
        assert_eq!(
            parse("kassiber://workflow/sync-all"),
            Some(menu_action("sync-all-wallets"))
        );
        assert_eq!(
            parse("kassiber://workflow/process-journals"),
            Some(menu_action("process-journals"))
        );
        assert_eq!(parse("kassiber://lock"), Some(menu_action("lock-app")));
    }

    #[test]
    fn deep_links_normalize_segment_case() {
        // URLs auto-capitalized by mail clients, chat apps, or the user
        // mistyping a segment must still resolve. Without this normalization
        // `kassiber://settings/Privacy` silently falls through to the
        // sectionless Settings open and the user wonders why their link
        // doesn't focus the panel.
        let parse = |s: &str| menu_action_for_deep_link(&Url::parse(s).unwrap());

        assert_eq!(
            parse("kassiber://settings/Privacy"),
            Some(open_settings_action(Some("privacy")))
        );
        assert_eq!(
            parse("kassiber://workflow/Sync-All"),
            Some(menu_action("sync-all-wallets"))
        );
        assert_eq!(
            parse("kassiber://workflow/PROCESS-JOURNALS"),
            Some(menu_action("process-journals"))
        );
    }

    #[test]
    fn deep_links_reject_unknown_routes() {
        let parse = |s: &str| menu_action_for_deep_link(&Url::parse(s).unwrap());

        // Unknown top-level slug is rejected outright — never silently
        // navigate the user somewhere they didn't ask for.
        assert_eq!(parse("kassiber://wallet-of-satoshi"), None);
        // Unknown workflow command does NOT fall back to settings or nav.
        assert_eq!(parse("kassiber://workflow/drain-everything"), None);
        // `lock` only works as the bare host, not with extra segments — guards
        // against future route collisions like `lock/...`.
        assert_eq!(parse("kassiber://lock/maybe"), None);
        // Wrong scheme is always rejected, even if the path otherwise looks fine.
        assert_eq!(parse("https://transactions/abc"), None);
    }

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
    fn external_url_validation_accepts_http_and_https_urls() {
        assert_eq!(
            validated_external_url(" https://mempool.space/tx/abc123 ").unwrap(),
            "https://mempool.space/tx/abc123"
        );
        assert_eq!(
            validated_external_url("http://127.0.0.1:3002/tx/abc123").unwrap(),
            "http://127.0.0.1:3002/tx/abc123"
        );
    }

    #[test]
    fn external_url_validation_rejects_non_browser_urls() {
        for url in [
            "",
            "/tx/abc123",
            "file:///tmp/report.pdf",
            "ftp://example.test/tx/abc123",
            "javascript:alert(1)",
            "mailto:dev@example.test",
        ] {
            assert!(
                validated_external_url(url).is_err(),
                "{url:?} should be rejected"
            );
        }
    }

    #[test]
    fn external_url_validation_rejects_embedded_credentials() {
        for url in [
            "https://dev@example.test/tx/abc123",
            "https://dev:secret@example.test/tx/abc123",
        ] {
            assert!(
                validated_external_url(url).is_err(),
                "{url:?} should be rejected"
            );
        }
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
    fn import_project_prefers_data_dir_for_managed_state_root() {
        let parent = unique_temp_dir("managed-parent");
        let root = parent.join(".kassiber");
        let data = root.join("data");
        fs::create_dir_all(&data).expect("create data dir");
        fs::write(root.join("kassiber.sqlite3"), fake_kassiber_sqlite_bytes())
            .expect("write legacy root sqlite");
        fs::write(data.join("kassiber.sqlite3"), fake_kassiber_sqlite_bytes())
            .expect("write nested sqlite");
        let root = root.canonicalize().expect("canonical root");
        let data = data.canonicalize().expect("canonical data");

        let selection = inspect_import_project_directory(&root).expect("inspect project");
        assert_eq!(selection.state_root, root.to_string_lossy().to_string());
        assert_eq!(selection.data_root, data.to_string_lossy().to_string());
        assert_eq!(
            selection.database,
            data.join("kassiber.sqlite3").to_string_lossy().to_string()
        );
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
