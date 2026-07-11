#[allow(dead_code)]
mod secret_store;
mod supervisor;

use base64::Engine;
use secret_store::{
    secret_store_policy_status, touch_id_delete_passphrase, touch_id_get_passphrase,
    touch_id_passphrase_status, touch_id_store_passphrase, TouchIdPassphraseStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(target_os = "macos")]
use std::{fs::File, os::fd::AsRawFd};
use supervisor::{DaemonSupervisor, SupervisorError};
use tauri::menu::{AboutMetadata, Menu, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager, State, Url};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::DialogExt;

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_STATE_DIR: &str = ".kassiber";
const DEFAULT_PROJECTS_DIR: &str = "projects";
const DEFAULT_PROJECT_ID: &str = "default";
const DEFAULT_DATA_DIR: &str = "data";
const CLI_LEGACY_UNLOCK_QUARANTINED_SETTING: &str = "cli_legacy_unlock_quarantined";
const DESKTOP_BIOMETRIC_STALE_SETTING: &str = "desktop_biometric_stale";
const DB_FILENAMES: &[&str] = &["kassiber.sqlite3", "satbooks.sqlite3"];
const LEDGER_PREVIEW_EXTENSIONS: &[&str] = &["csv", "tsv", "xlsx", "xlsm"];
const DOCUMENT_IMPORT_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "pdf"];
const DOCUMENT_IMPORT_STAGE_KIND: &str = "internal.document_import.stage";
const IMPORT_PICKER_TIMEOUT: Duration = Duration::from_secs(300);
const TERMINAL_COMMAND_NAME: &str = "kassiber";
const TERMINAL_COMMAND_MARKER: &str =
    "Kassiber desktop CLI launcher. Managed by Kassiber Settings.";
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
const MENU_UI_SCALE_DECREASE: &str = "kassiber:ui-scale:decrease";
const MENU_UI_SCALE_INCREASE: &str = "kassiber:ui-scale:increase";
const MENU_UI_SCALE_RESET: &str = "kassiber:ui-scale:reset";
const MENU_TOGGLE_FULLSCREEN: &str = "kassiber:window:toggle-fullscreen";
const MENU_WINDOW_CLOSE: &str = "kassiber:window:close";
const MENU_WINDOW_MINIMIZE: &str = "kassiber:window:minimize";
const MENU_WINDOW_ZOOM: &str = "kassiber:window:zoom";
const MENU_WINDOW_FOCUS: &str = "kassiber:window:focus";
const MENU_QUIT: &str = "kassiber:quit";
const MENU_HELP_DOCS: &str = "kassiber:help:docs";
const MENU_HELP_ISSUES: &str = "kassiber:help:issues";
const MENU_WORKFLOW_ADD_WALLET: &str = "kassiber:workflow:add-wallet";
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
const MENU_NAV_QUARANTINE: &str = "kassiber:navigate:quarantine";
const MENU_NAV_ASSISTANT: &str = "kassiber:navigate:assistant";
const MENU_NAV_LOGS: &str = "kassiber:navigate:logs";
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
//   kassiber://workflow/add-wallet            opens wallet-source setup
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
    ("tax-events", "/journals"),
    ("quarantine", "/quarantine"),
    ("assistant", "/assistant"),
    ("logs", "/logs"),
    ("diagnostics", "/logs"),
];

// Mirrors the React `settingsSectionForHash` map in
// `ui-tauri/src/components/kb/settingsSections.ts`. Aliases (`sync` →
// bitcoin, `assistant` → ai) are accepted at the deep-link boundary so the
// Rust allowlist matches the section-resolution logic on the React side; the
// React helper does the final hash → section-id lookup.
const DEEP_LINK_SETTINGS_SECTIONS: &[&str] = &[
    "appearance",
    "privacy",
    "developer",
    "logs",
    "display",
    "explorer",
    "explorers",
    "bitcoin",
    "lightning",
    "liquid",
    "market",
    "desktop",
    "terminal",
    "security",
    "lock",
    "backends",
    "sync",
    "replication",
    "rates",
    "ai",
    "assistant",
    "data",
    "storage",
];

const ALLOWED_DAEMON_KINDS: &[&str] = &[
    "status",
    "ui.egress.snapshot",
    "ui.overview.snapshot",
    "ui.workspace.overview.snapshot",
    "ui.transactions.list",
    "ui.transactions.metadata.update",
    "ui.transactions.resolve",
    "ui.transactions.graph",
    "ui.transactions.history",
    "ui.transactions.history.revert",
    "ui.activity.history",
    "ui.activity.stale",
    "ui.attachments.list",
    "ui.attachments.add",
    "ui.attachments.copy",
    "ui.attachments.rename",
    "ui.attachments.remove",
    "ui.attachments.open",
    "ui.wallets.list",
    "ui.backends.list",
    "ui.backends.options",
    "ui.backends.public_defaults",
    "ui.backends.settings.list",
    "ui.backends.create",
    "ui.backends.update",
    "ui.backends.delete",
    "ui.backends.set_default",
    "ui.backends.bitcoinrpc.test",
    "ui.backends.detect_core",
    "ui.backends.electrum.test",
    "ui.backends.http.test",
    "ui.backends.lightning.test",
    "ui.profiles.snapshot",
    "ui.onboarding.complete",
    "ui.profiles.create",
    "ui.profiles.rename",
    "ui.profiles.update",
    "ui.profiles.switch",
    "ui.profiles.reset_data",
    "ui.reports.capital_gains",
    "ui.reports.summary",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
    "ui.reports.balance_history",
    "ui.reports.tax_summary",
    "ui.reports.privacy_hygiene",
    "ui.reports.privacy_mirror",
    "ui.reports.psbt_privacy",
    "ui.reports.exit_tax_preview",
    "ui.reports.export_exit_tax_pdf",
    "ui.reports.export_exit_tax_xlsx",
    "ui.reports.export_pdf",
    "ui.reports.export_summary_pdf",
    "ui.reports.export_csv",
    "ui.reports.export_xlsx",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_pdf",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.reports.export_austrian_e1kv_csv",
    "ui.reports.export_audit_package",
    "ui.transactions.export_csv",
    "ui.transactions.export_xlsx",
    "ui.transactions.ledger_template",
    "ui.journals.snapshot",
    "ui.journals.events.list",
    "ui.journals.quarantine",
    "ui.journals.transfers.list",
    "ui.journals.process",
    "ui.transfers.suggest",
    "ui.transfers.list",
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.create",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.update",
    "ui.transfers.bulk_pair",
    "ui.transfers.dismiss",
    "ui.transfers.rules.list",
    "ui.transfers.rules.create",
    "ui.transfers.rules.delete",
    "ui.transfers.rules.set_enabled",
    "ui.transfers.rules.apply",
    "ui.saved_views.list",
    "ui.saved_views.create",
    "ui.saved_views.delete",
    "ui.rates.summary",
    "ui.rates.coverage",
    "ui.rates.kraken_csv.import",
    "ui.rates.latest",
    "ui.rates.rebuild",
    "ui.maintenance.settings",
    "ui.maintenance.configure",
    "ui.maintenance.run",
    "ui.sync.status",
    "ui.sync.enable",
    "ui.sync.disable",
    "ui.sync.transports.list",
    "ui.sync.transports.configure",
    "ui.sync.transports.delete",
    "ui.sync.push",
    "ui.sync.pull",
    "ui.sync.join_request",
    "ui.sync.invite",
    "ui.sync.join",
    "ui.sync.members.list",
    "ui.sync.members.revoke",
    "ui.sync.devices.list",
    "ui.sync.devices.revoke",
    "ui.sync.conflicts.list",
    "ui.sync.conflicts.resolve",
    "ui.workspace.health",
    "ui.workspace.freshness.run",
    "ui.audit.evidence.summary",
    "ui.workspace.create",
    "ui.workspace.rename",
    "ui.workspace.delete",
    "ui.projects.list",
    "ui.projects.create",
    "ui.projects.select",
    "ui.secrets.init",
    "ui.secrets.change_passphrase",
    "ui.secrets.forget_cli_unlock",
    "ui.next_actions",
    "ui.review.badges",
    "ui.wallets.utxos",
    "ui.privacy_hygiene.snapshot",
    "ui.loans.list",
    "ui.loans.link",
    "ui.loans.mark",
    "ui.loans.unmark",
    "ui.wallets.create",
    "ui.wallets.import_file",
    "ui.wallets.document_import.preview",
    "ui.wallets.document_import.import",
    "ui.wallets.import_samourai",
    "ui.wallets.ledger_preview",
    "ui.wallets.preview_descriptor",
    "ui.wallets.detect_script_types",
    "ui.wallets.identify",
    "ui.wallets.identify_onchain",
    "ui.connections.sources",
    "ui.connections.btcpay.create",
    "ui.connections.btcpay.discover",
    "ui.connections.btcpay.test",
    "ui.connections.node.snapshot",
    "ui.reports.lightning_profitability",
    "ui.metadata.bip329.preview",
    "ui.metadata.bip329.import",
    "ui.metadata.bip329.export",
    "ui.wallets.update",
    "ui.wallets.delete",
    "ui.wallets.sync",
    "ui.freshness.status",
    "ui.freshness.configure",
    "ui.freshness.run",
    "ui.freshness.cancel",
    "ui.freshness.pause",
    "ui.freshness.resume",
    "wallets.reveal_descriptor",
    "daemon.lock",
    "daemon.unlock",
    "ai.providers.list",
    "ai.providers.get",
    "ai.providers.create",
    "ai.providers.update",
    "ai.providers.set_api_key",
    "ai.providers.move_api_key",
    "ai.providers.delete",
    "ai.providers.set_default",
    "ai.providers.clear_default",
    "ai.providers.acknowledge",
    "ai.list_models",
    "ai.test_connection",
    "ai.chat",
    "ai.chat.cancel",
    "ai.tool_call.consent",
    // Stored chat history stays manageable even while the AI runtime
    // toggle is off — these are privacy controls, not AI runtime kinds.
    "ui.chat.sessions.list",
    "ui.chat.sessions.get",
    "ui.chat.sessions.delete",
    "ui.chat.sessions.clear",
    "ui.chat.history.configure",
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
    "ui.source_funds.assemble",
    "ui.source_funds.evidence.list",
    "ui.source_funds.export_pdf",
    "ui.source_funds.export_bundle",
    "ui.source_funds.coverage",
    "ui.source_funds.recipients.list",
    "ui.source_funds.recipients.create",
    "ui.source_funds.recipients.update",
    "ui.source_funds.recipients.delete",
    "ui.btcpay.provenance.sync",
    "ui.btcpay.provenance.list",
    "ui.btcpay.provenance.suggest",
    "ui.btcpay.provenance.links",
    "ui.btcpay.provenance.review",
    "ui.transactions.commercial_context",
    "ui.documents.list",
    "ui.documents.create",
    "ui.documents.attach",
    "ui.logs.snapshot",
];

/// Kinds that may emit intermediate stream records (kind = "<request_kind>.delta",
/// "<request_kind>.tool_call", etc.) before the terminal envelope. The supervisor
/// forwards intermediate records to the webview as Tauri events
/// `daemon://stream` and switches to a per-record inactivity
/// timeout. Other kinds keep the existing total-budget (15s) behavior.
///
/// This list also covers long-running, result-bearing kinds the UI invokes that
/// run heavy sync/RP2 work synchronously on the daemon's single serial loop.
/// They may not emit intermediate records yet, but they legitimately exceed the
/// 15s non-streaming budget, so the inactivity timeout lets a sub-window run
/// finish and return its result instead of the caller being abandoned at 15s. A
/// fully-silent run beyond the inactivity window returns `daemon_busy` to the
/// caller without killing the shared daemon; the UX follow-up is to emit real
/// progress from those handlers. `ui.maintenance.run` is intentionally absent:
/// it is only ever an AI tool call run inside `ai.chat`, never a top-level
/// supervisor request, so its classification here would be inert.
const STREAMING_DAEMON_KINDS: &[&str] = &[
    "ai.chat",
    "ui.wallets.sync",
    "ui.freshness.run",
    "ui.workspace.freshness.run",
    "ui.journals.process",
    "ui.rates.rebuild",
    "ui.wallets.document_import.preview",
    "ui.wallets.document_import.import",
    "ui.sync.push",
    "ui.sync.pull",
    "ui.sync.join",
];

// Daemon kinds that exercise the AI runtime (model calls, chat sessions, tool
// consent prompts). Gated server-side by the global AI features toggle so the
// switch is a real privacy promise instead of just hiding the UI — every
// future caller can't accidentally bypass the guard. `ai.providers.*` is
// deliberately excluded: providers stay configurable while AI is off so the
// user can wire keys before turning the feature on.
const AI_RUNTIME_KINDS: &[&str] = &[
    "ai.list_models",
    "ai.test_connection",
    "ui.wallets.document_import.preview",
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalCommandStatus {
    platform: &'static str,
    available: bool,
    installed: bool,
    managed: bool,
    needs_repair: bool,
    conflict: bool,
    path_on_path: bool,
    command: String,
    bin_dir: String,
    command_path: String,
    target_path: String,
    path_hint: String,
    message: String,
}

struct TerminalCommandPaths {
    platform: &'static str,
    bin_dir: PathBuf,
    command_path: PathBuf,
    target_path: PathBuf,
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
            Ok(mut response) => {
                attach_secret_store_policy_status(&mut response);
                match serde_json::from_value(response) {
                    Ok(envelope) => envelope,
                    Err(error) => error_envelope(
                        "daemon_protocol_error",
                        format!(
                            "Python daemon response did not match the envelope contract: {error}"
                        ),
                        Some("Check daemon smoke tests before wiring more UI kinds."),
                        None,
                        task_request_id.clone(),
                        false,
                    ),
                }
            }
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
async fn pick_document_import_source(
    app: tauri::AppHandle,
    state: State<'_, Arc<DaemonSupervisor>>,
) -> Result<Option<Value>, String> {
    // This command deliberately accepts no path or filter arguments from the
    // webview. Only the native picker may mint the daemon's opaque document
    // session, so a compromised renderer cannot turn OCR into a local-file
    // read oracle.
    let selection = app
        .dialog()
        .file()
        .add_filter("Images and PDF", DOCUMENT_IMPORT_EXTENSIONS)
        .blocking_pick_file();
    let Some(selection) = selection else {
        return Ok(None);
    };
    let source_path = selection
        .into_path()
        .map_err(|_| "The selected document path is unavailable.".to_string())?;
    let supervisor = Arc::clone(state.inner());
    let response = tauri::async_runtime::spawn_blocking(move || {
        supervisor.invoke(
            DOCUMENT_IMPORT_STAGE_KIND,
            Some(json!({ "source_file": source_path.to_string_lossy() })),
            &app,
            false,
            None,
        )
    })
    .await
    .map_err(|error| format!("Document picker task failed: {error}"))?
    .map_err(|error| format!("Could not stage the selected document: {}", error.message))?;

    match response.get("kind").and_then(Value::as_str) {
        Some(DOCUMENT_IMPORT_STAGE_KIND) => response
            .get("data")
            .cloned()
            .map(Some)
            .ok_or_else(|| "Document staging returned no session.".to_string()),
        Some("error") | Some("auth_required") => Err(response
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("Could not stage the selected document.")
            .to_string()),
        _ => Err("Document staging returned an unexpected response.".to_string()),
    }
}

fn attach_secret_store_policy_status(response: &mut Value) {
    if response.get("kind").and_then(Value::as_str) != Some("ai.providers.list") {
        return;
    }
    let Some(data) = response.get_mut("data").and_then(Value::as_object_mut) else {
        return;
    };
    data.insert(
        "secret_store_policy".to_string(),
        secret_store_policy_status(),
    );
}

#[tauri::command]
fn daemon_lifecycle_snapshot(
    state: State<'_, Arc<DaemonSupervisor>>,
    after_id: u64,
) -> Result<Value, String> {
    let (records, last_id) = state.lifecycle_snapshot(after_id);
    Ok(json!({
        "records": records,
        "lastId": last_id,
    }))
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
    if !is_supported_report_export_target(&canonical, &metadata) {
        return Err(
            "Only managed PDF, XLSX, CSV files, Austrian CSV bundle folders, and audit package folders can be opened."
                .to_string(),
        );
    }

    open_with_default_app(&canonical)
}

#[tauri::command]
fn open_attachment_file(
    state: State<'_, Arc<DaemonSupervisor>>,
    path: String,
) -> Result<(), String> {
    let requested = PathBuf::from(path);
    if !requested.is_absolute() {
        return Err("Attachment paths must be absolute.".to_string());
    }

    let data_root = state
        .current_data_root()
        .map_err(|error| error.message)?
        .unwrap_or_else(default_state_data_root);
    let canonical = validated_attachment_file_path(&data_root, &requested)?;
    // Validation and spawning are intentionally adjacent; this is a local desktop
    // open path, so the remaining TOCTOU surface is limited to same-user races.
    open_path_with_default_app(&canonical, "attachment")
}

fn validated_attachment_file_path(data_root: &Path, requested: &Path) -> Result<PathBuf, String> {
    if !requested.is_absolute() {
        return Err("Attachment paths must be absolute.".to_string());
    }
    let state_root =
        if data_root.file_name().and_then(|name| name.to_str()) == Some(DEFAULT_DATA_DIR) {
            data_root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| data_root.to_path_buf())
        } else {
            data_root.to_path_buf()
        };
    let attachments_root = std::fs::canonicalize(state_root.join("attachments"))
        .map_err(|error| format!("Attachments folder could not be found: {error}"))?;
    let canonical = std::fs::canonicalize(requested)
        .map_err(|error| format!("Attachment file could not be found: {error}"))?;
    if !canonical.starts_with(&attachments_root) {
        return Err("Only managed Kassiber attachment files can be opened.".to_string());
    }
    let metadata = canonical
        .metadata()
        .map_err(|error| format!("Attachment file could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("Only attachment files can be opened.".to_string());
    }
    Ok(canonical)
}

#[tauri::command]
fn save_exported_file_as(source_path: String, destination_path: String) -> Result<String, String> {
    let source = PathBuf::from(source_path);
    if !source.is_absolute() {
        return Err("Report export source paths must be absolute.".to_string());
    }
    let destination = PathBuf::from(destination_path);
    if !destination.is_absolute() {
        return Err("Report export destination paths must be absolute.".to_string());
    }

    let canonical_source = std::fs::canonicalize(&source)
        .map_err(|error| format!("Report export file could not be found: {error}"))?;
    let metadata = canonical_source
        .metadata()
        .map_err(|error| format!("Report export file could not be inspected: {error}"))?;
    if !is_supported_report_export_target(&canonical_source, &metadata) {
        return Err(
            "Only managed PDF, XLSX, CSV files, Austrian CSV bundle folders, and audit package folders can be saved."
                .to_string(),
        );
    }
    ensure_export_destination_outside_managed_root(&canonical_source, &destination)?;

    if metadata.is_file() {
        copy_report_export_file(&canonical_source, &destination)?;
    } else {
        copy_report_export_directory(&canonical_source, &destination)?;
    }
    Ok(destination.to_string_lossy().into_owned())
}

/// Write ``contents`` to ``destination_path`` after validating that the
/// caller-supplied path is absolute and uses one of ``permitted_extensions``.
///
/// The extension allow-list is intentionally **not** a parameter of any
/// ``#[tauri::command]`` — it must be hard-coded per command in this file.
/// Otherwise any code with WebView ``invoke`` access (a compromised
/// renderer, a malicious dependency, an XSS) could pass its own list and
/// write arbitrary file types to any absolute path.
fn write_text_export(
    destination_path: String,
    contents: String,
    permitted_extensions: &[&str],
) -> Result<String, String> {
    debug_assert!(
        !permitted_extensions.is_empty(),
        "permitted_extensions must be hard-coded with at least one entry",
    );
    let destination = PathBuf::from(destination_path);
    if !destination.is_absolute() {
        return Err("Export destination paths must be absolute.".to_string());
    }
    let actual = destination
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    let allowed = actual
        .as_deref()
        .map(|ext| {
            permitted_extensions
                .iter()
                .any(|p| p.eq_ignore_ascii_case(ext))
        })
        .unwrap_or(false);
    if !allowed {
        let expected = permitted_extensions
            .iter()
            .map(|ext| format!(".{ext}"))
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(format!("Export destination must use {expected}."));
    }
    let Some(parent) = destination.parent() else {
        return Err("Export destination must include a parent folder.".to_string());
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create export destination folder: {error}"))?;
    std::fs::write(&destination, contents)
        .map_err(|error| format!("Could not save export: {error}"))?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
fn save_chat_export_as(destination_path: String, contents: String) -> Result<String, String> {
    write_text_export(destination_path, contents, &["md"])
}

#[tauri::command]
fn save_logs_export_as(destination_path: String, contents: String) -> Result<String, String> {
    write_text_export(destination_path, contents, &["jsonl", "log", "md"])
}

#[tauri::command]
fn read_ledger_preview_file_base64(path: String) -> Result<String, String> {
    let requested = PathBuf::from(path);
    if !requested.is_absolute() {
        return Err("Ledger preview path must be absolute.".to_string());
    }
    let extension = requested
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .ok_or_else(|| "Ledger preview file must use .csv, .tsv, .xlsx, or .xlsm.".to_string())?;
    if !LEDGER_PREVIEW_EXTENSIONS
        .iter()
        .any(|allowed| *allowed == extension)
    {
        return Err("Ledger preview file must use .csv, .tsv, .xlsx, or .xlsm.".to_string());
    }
    let canonical = std::fs::canonicalize(&requested)
        .map_err(|error| format!("Ledger preview file could not be found: {error}"))?;
    let metadata = canonical
        .metadata()
        .map_err(|error| format!("Ledger preview file could not be inspected: {error}"))?;
    if !metadata.is_file() {
        return Err("Ledger preview selection must be a file.".to_string());
    }
    let bytes = std::fs::read(&canonical)
        .map_err(|error| format!("Ledger preview file could not be read: {error}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn ensure_export_destination_outside_managed_root(
    source: &Path,
    destination: &Path,
) -> Result<(), String> {
    let Some(managed_root) = managed_report_exports_root(source) else {
        return Err("Report export source is not in the managed exports folder.".to_string());
    };
    let Some(destination_parent) = destination.parent() else {
        return Err("Report export destination must include a parent folder.".to_string());
    };
    std::fs::create_dir_all(destination_parent)
        .map_err(|error| format!("Could not create report export destination folder: {error}"))?;
    let canonical_parent = std::fs::canonicalize(destination_parent)
        .map_err(|error| format!("Could not inspect report export destination folder: {error}"))?;
    let canonical_managed_root = std::fs::canonicalize(managed_root)
        .map_err(|error| format!("Could not inspect managed report export folder: {error}"))?;
    if canonical_parent.starts_with(canonical_managed_root) {
        return Err("Choose a destination outside Kassiber's managed exports folder.".to_string());
    }
    Ok(())
}

fn copy_report_export_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !is_supported_export_file(destination) {
        return Err("Report export destination must use .pdf, .xlsx, or .csv.".to_string());
    }
    let Some(parent) = destination.parent() else {
        return Err("Report export destination must include a parent folder.".to_string());
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create report export destination folder: {error}"))?;
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| format!("Could not save report export: {error}"))
}

fn copy_report_export_directory(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists()
        && destination
            .read_dir()
            .map_err(|error| format!("Could not inspect report export destination: {error}"))?
            .next()
            .is_some()
    {
        return Err("Choose a new or empty folder for the report export.".to_string());
    }
    std::fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create report export destination: {error}"))?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("Could not read managed report export: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("Could not read report export entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let entry_metadata = entry
            .metadata()
            .map_err(|error| format!("Could not inspect report export entry: {error}"))?;
        if entry_metadata.is_dir() {
            copy_report_export_directory(&source_path, &destination_path)?;
        } else {
            std::fs::copy(&source_path, &destination_path)
                .map(|_| ())
                .map_err(|error| format!("Could not save report export entry: {error}"))?;
        }
    }
    Ok(())
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

#[tauri::command]
fn touch_id_passphrase_status_command(
    state: State<'_, Arc<DaemonSupervisor>>,
    data_root: Option<String>,
) -> Result<TouchIdPassphraseStatus, String> {
    let scope = touch_id_scope_for_data_root(&state, data_root)?;
    touch_id_passphrase_status_with_managed_guard(&scope)
}

#[tauri::command]
fn touch_id_store_passphrase_command(
    state: State<'_, Arc<DaemonSupervisor>>,
    data_root: Option<String>,
    passphrase_secret: String,
    stale_generation: Option<String>,
) -> Result<TouchIdPassphraseStatus, String> {
    let scope = touch_id_scope_for_data_root(&state, data_root)?;
    let expected_generation = stale_generation.filter(|value| !value.trim().is_empty());
    let _lifecycle = touch_id_credential_lifecycle_lock()?;
    if desktop_biometric_stale_generation(&scope.data_root)?.as_deref()
        != expected_generation.as_deref()
    {
        return Err(
            "Touch ID enrollment state changed; refresh status and verify the current passphrase again."
                .to_string(),
        );
    }
    touch_id_store_passphrase(&scope.account, &passphrase_secret)?;
    clear_desktop_biometric_stale_guard_if_matches(
        &scope.data_root,
        expected_generation.as_deref(),
    )?;
    touch_id_passphrase_status_with_managed_guard(&scope)
}

#[tauri::command]
async fn touch_id_unlock_passphrase_command(
    app: tauri::AppHandle,
    state: State<'_, Arc<DaemonSupervisor>>,
    data_root: Option<String>,
    require_existing_project: Option<bool>,
    project_id: Option<String>,
) -> Result<DaemonEnvelope, String> {
    let scope = touch_id_scope_for_data_root(&state, data_root)?;
    let (cli_owns_legacy, stale_generation) = touch_id_managed_unlock_state(&scope.data_root)?;
    if stale_generation.is_some() {
        return Ok(error_envelope(
            "touch_id_passphrase_not_found",
            "The saved Touch ID passphrase is stale for these books.",
            Some("Unlock once with the current passphrase and re-enroll Touch ID."),
            None,
            None,
            false,
        ));
    }
    let Some(passphrase_secret) = touch_id_get_passphrase(&scope.account, cli_owns_legacy)? else {
        return Ok(error_envelope(
            "touch_id_passphrase_not_found",
            "No Touch ID passphrase was found for these books.",
            Some("Unlock once with the passphrase to save it again."),
            None,
            None,
            false,
        ));
    };
    let mut args = json!({
        "auth_response": { "passphrase_secret": passphrase_secret },
        "require_existing_project": require_existing_project.unwrap_or(false),
    });
    let kind = if let Some(project_id) = project_id.filter(|value| !value.trim().is_empty()) {
        args["project_id"] = json!(project_id);
        "ui.projects.select"
    } else {
        "daemon.unlock"
    };
    let supervisor = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        match supervisor.invoke(kind, Some(args), &app, false, None) {
            Ok(mut response) => {
                attach_secret_store_policy_status(&mut response);
                serde_json::from_value(response).map_err(|error| {
                    format!("Python daemon response did not match the envelope contract: {error}")
                })
            }
            Err(error) => Ok(supervisor_error_envelope(error, None)),
        }
    })
    .await
    .map_err(|error| format!("Touch ID unlock task failed: {error}"))?
}

#[tauri::command]
fn touch_id_forget_passphrase_command(
    state: State<'_, Arc<DaemonSupervisor>>,
    data_root: Option<String>,
) -> Result<TouchIdPassphraseStatus, String> {
    let scope = touch_id_scope_for_data_root(&state, data_root)?;
    let _lifecycle = touch_id_credential_lifecycle_lock()?;
    let (cli_owns_legacy, _stale_generation) = touch_id_managed_unlock_state(&scope.data_root)?;
    touch_id_delete_passphrase(&scope.account, cli_owns_legacy)?;
    clear_desktop_biometric_stale_guard(&scope.data_root)?;
    touch_id_passphrase_status_with_managed_guard(&scope)
}

#[tauri::command]
fn terminal_command_status_command() -> Result<TerminalCommandStatus, String> {
    terminal_command_status()
}

#[tauri::command]
fn terminal_command_install_command() -> Result<TerminalCommandStatus, String> {
    install_terminal_command()?;
    terminal_command_status()
}

#[tauri::command]
fn terminal_command_remove_command() -> Result<TerminalCommandStatus, String> {
    remove_terminal_command()?;
    terminal_command_status()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TouchIdScope {
    account: String,
    data_root: String,
}

fn touch_id_scope_for_selected(selected: PathBuf) -> TouchIdScope {
    let normalized = std::fs::canonicalize(&selected).unwrap_or_else(|_| selected.clone());
    TouchIdScope {
        account: normalized.to_string_lossy().into_owned(),
        data_root: selected.to_string_lossy().into_owned(),
    }
}

fn touch_id_scope_for_data_root(
    state: &Arc<DaemonSupervisor>,
    data_root: Option<String>,
) -> Result<TouchIdScope, String> {
    let selected = if let Some(explicit) = data_root.filter(|value| !value.trim().is_empty()) {
        PathBuf::from(explicit)
    } else if let Some(active) = state.current_data_root().map_err(|error| error.message)? {
        active
    } else {
        default_state_data_root()
    };
    // The normalized data-root path is the Keychain account namespace. Fall
    // back to the selected path for first-run/default roots that may not exist
    // before the daemon creates them.
    Ok(touch_id_scope_for_selected(selected))
}

fn managed_settings_path(data_root: &str) -> PathBuf {
    let data_root = Path::new(data_root);
    let state_root =
        if data_root.file_name().and_then(|name| name.to_str()) == Some(DEFAULT_DATA_DIR) {
            data_root.parent().unwrap_or(data_root)
        } else {
            data_root
        };
    state_root.join("config").join("settings.json")
}

fn read_managed_settings(data_root: &str) -> Result<Option<Value>, String> {
    let settings_path = managed_settings_path(data_root);
    let raw = match fs::read_to_string(&settings_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Managed settings could not be read: {error}")),
    };
    let value = serde_json::from_str::<Value>(&raw)
        .map_err(|error| format!("Managed settings JSON is invalid: {error}"))?;
    if !value.is_object() {
        return Err("Managed settings must contain a JSON object.".to_string());
    }
    Ok(Some(value))
}

fn stale_generation_from_settings(value: &Value) -> Option<String> {
    let raw = value.get(DESKTOP_BIOMETRIC_STALE_SETTING)?;
    if let Some(generation) = raw.as_str().filter(|value| !value.trim().is_empty()) {
        return Some(generation.to_string());
    }
    // Compatibility for short-lived development builds that wrote a boolean
    // guard before generation-bound compare-and-clear landed.
    raw.as_bool()
        .filter(|enabled| *enabled)
        .map(|_| "legacy-boolean-guard".to_string())
}

fn desktop_biometric_stale_generation(data_root: &str) -> Result<Option<String>, String> {
    Ok(touch_id_managed_unlock_state(data_root)?.1)
}

fn touch_id_managed_unlock_state(data_root: &str) -> Result<(bool, Option<String>), String> {
    let settings = read_managed_settings(data_root)?;
    let cli_owns_legacy = settings.as_ref().is_some_and(|value| {
        value
            .get("cli_remembered_unlock")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || value
                .get(CLI_LEGACY_UNLOCK_QUARANTINED_SETTING)
                .and_then(Value::as_bool)
                .unwrap_or(false)
    });
    let stale_generation = settings.as_ref().and_then(stale_generation_from_settings);
    Ok((cli_owns_legacy, stale_generation))
}

fn touch_id_passphrase_status_with_managed_guard(
    scope: &TouchIdScope,
) -> Result<TouchIdPassphraseStatus, String> {
    let (cli_owns_legacy, stale_generation) = touch_id_managed_unlock_state(&scope.data_root)?;
    let mut status = touch_id_passphrase_status(&scope.account, cli_owns_legacy);
    let Some(generation) = stale_generation else {
        return Ok(status);
    };
    status.configured = false;
    status.stale = true;
    status.protection = None;
    status.stale_generation = Some(generation);
    Ok(status)
}

static TOUCH_ID_CREDENTIAL_LIFECYCLE: OnceLock<Mutex<()>> = OnceLock::new();

fn touch_id_credential_lifecycle_lock() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    TOUCH_ID_CREDENTIAL_LIFECYCLE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Touch ID credential lifecycle lock is poisoned.".to_string())
}

#[cfg(target_os = "macos")]
struct ManagedSettingsLock(File);

#[cfg(target_os = "macos")]
impl Drop for ManagedSettingsLock {
    fn drop(&mut self) {
        // SAFETY: this descriptor stays owned by the guard until after the
        // unlock attempt. A failed unlock is harmless because closing the
        // descriptor also releases flock locks.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(target_os = "macos")]
fn lock_managed_settings(settings_path: &Path) -> Result<ManagedSettingsLock, String> {
    let lock_path = settings_path.with_file_name("settings.json.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("Managed settings lock could not be opened: {error}"))?;
    // SAFETY: flock only borrows the live file descriptor for this call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!(
            "Managed settings lock could not be acquired: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(ManagedSettingsLock(file))
}

#[cfg(not(target_os = "macos"))]
fn lock_managed_settings(_settings_path: &Path) -> Result<(), String> {
    Ok(())
}

fn write_managed_settings_payload(
    settings_path: &Path,
    parent: &Path,
    payload: &Value,
) -> Result<(), String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock could not create a settings nonce: {error}"))?
        .as_nanos();
    let temp_path = parent.join(format!(".settings.json.{}.{nonce}.tmp", std::process::id()));
    let encoded = serde_json::to_vec_pretty(payload)
        .map_err(|error| format!("Managed settings could not be encoded: {error}"))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .map_err(|error| format!("Managed settings temporary file could not be opened: {error}"))?;
    let write_result = file
        .write_all(&encoded)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Managed settings could not be written: {error}"));
    }
    #[cfg(target_os = "windows")]
    if settings_path.exists() {
        fs::remove_file(settings_path)
            .map_err(|error| format!("Managed settings could not be replaced: {error}"))?;
    }
    if let Err(error) = fs::rename(&temp_path, settings_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("Managed settings could not be replaced: {error}"));
    }
    Ok(())
}

fn update_desktop_biometric_stale_guard(
    data_root: &str,
    expected_generation: Option<Option<&str>>,
) -> Result<bool, String> {
    let settings_path = managed_settings_path(data_root);
    if !settings_path.exists() {
        return Ok(matches!(expected_generation, None | Some(None)));
    }
    let parent = settings_path
        .parent()
        .ok_or_else(|| "Managed settings path has no parent directory.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Managed settings directory could not be created: {error}"))?;
    let _settings_lock = lock_managed_settings(&settings_path)?;
    let mut payload = match fs::read_to_string(&settings_path) {
        Ok(raw) => serde_json::from_str::<Value>(&raw)
            .map_err(|error| format!("Managed settings JSON is invalid: {error}"))?,
        Err(error) if error.kind() == ErrorKind::NotFound => json!({}),
        Err(error) => return Err(format!("Managed settings could not be read: {error}")),
    };
    let current_generation = stale_generation_from_settings(&payload);
    if let Some(expected) = expected_generation {
        if current_generation.as_deref() != expected {
            return Ok(false);
        }
    }
    if current_generation.is_none() {
        return Ok(true);
    }
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "Managed settings must contain a JSON object.".to_string())?;
    object.remove(DESKTOP_BIOMETRIC_STALE_SETTING);
    write_managed_settings_payload(&settings_path, parent, &payload)?;
    Ok(true)
}

fn clear_desktop_biometric_stale_guard_if_matches(
    data_root: &str,
    expected_generation: Option<&str>,
) -> Result<bool, String> {
    update_desktop_biometric_stale_guard(data_root, Some(expected_generation))
}

fn clear_desktop_biometric_stale_guard(data_root: &str) -> Result<bool, String> {
    update_desktop_biometric_stale_guard(data_root, None)
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
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
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
    #[cfg(target_os = "windows")]
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"));
    #[cfg(not(target_os = "windows"))]
    let home = env::var_os("HOME");

    home.map(PathBuf::from)
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

fn default_state_data_root() -> PathBuf {
    home_dir()
        .map(|home| {
            home.join(DEFAULT_STATE_DIR)
                .join(DEFAULT_PROJECTS_DIR)
                .join(DEFAULT_PROJECT_ID)
                .join(DEFAULT_DATA_DIR)
        })
        .unwrap_or_else(|| {
            PathBuf::from(DEFAULT_STATE_DIR)
                .join(DEFAULT_PROJECTS_DIR)
                .join(DEFAULT_PROJECT_ID)
                .join(DEFAULT_DATA_DIR)
        })
}

fn terminal_command_paths() -> Result<TerminalCommandPaths, String> {
    let target_path = terminal_command_target_path()?;
    let home = home_dir().ok_or_else(|| {
        "Could not locate your home folder for a user-owned terminal command.".to_string()
    })?;

    #[cfg(target_os = "windows")]
    let platform = "windows";
    #[cfg(target_os = "macos")]
    let platform = "macos";
    #[cfg(target_os = "linux")]
    let platform = "linux";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let platform = "unsupported";

    #[cfg(target_os = "windows")]
    let candidates = terminal_command_windows_candidate_dirs(&home);
    #[cfg(not(target_os = "windows"))]
    let candidates = vec![home.join(".local").join("bin"), home.join("bin")];

    let bin_dir = candidates
        .iter()
        .find(|path| path_is_on_path(path))
        .cloned()
        .or_else(|| candidates.first().cloned())
        .ok_or_else(|| "Could not choose a user-owned terminal command directory.".to_string())?;
    let command_path = bin_dir.join(terminal_command_filename());
    Ok(TerminalCommandPaths {
        platform,
        bin_dir,
        command_path,
        target_path,
    })
}

fn terminal_command_target_path() -> Result<PathBuf, String> {
    terminal_command_target_path_for(env::var_os("APPIMAGE"), || {
        env::current_exe()
            .map_err(|error| format!("Could not locate the Kassiber desktop executable: {error}"))
    })
}

fn terminal_command_target_path_for(
    appimage: Option<std::ffi::OsString>,
    current_exe: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(appimage) = appimage
            .as_ref()
            .filter(|value| !value.as_os_str().is_empty())
        {
            return Ok(PathBuf::from(appimage));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = appimage;
    current_exe()
}

#[cfg(target_os = "windows")]
fn terminal_command_windows_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local_app_data) =
        env::var_os("LOCALAPPDATA").filter(|value| !value.as_os_str().is_empty())
    {
        candidates.push(PathBuf::from(local_app_data).join("Kassiber").join("bin"));
    }
    candidates.push(home.join(".kassiber").join("bin"));
    candidates
}

#[cfg(target_os = "windows")]
fn terminal_command_filename() -> &'static str {
    "kassiber.cmd"
}

#[cfg(not(target_os = "windows"))]
fn terminal_command_filename() -> &'static str {
    TERMINAL_COMMAND_NAME
}

fn terminal_command_status() -> Result<TerminalCommandStatus, String> {
    let paths = terminal_command_paths()?;
    let existing = inspect_terminal_command(&paths)?;
    let path_on_path = path_is_on_path(&paths.bin_dir);
    let installed = matches!(existing, TerminalCommandFileState::Current);
    let managed = matches!(
        existing,
        TerminalCommandFileState::Current | TerminalCommandFileState::ManagedStale
    );
    let needs_repair = matches!(existing, TerminalCommandFileState::ManagedStale);
    let conflict = matches!(existing, TerminalCommandFileState::Conflict);
    let message = terminal_command_message(installed, needs_repair, conflict, path_on_path);
    Ok(TerminalCommandStatus {
        platform: paths.platform,
        available: true,
        installed,
        managed,
        needs_repair,
        conflict,
        path_on_path,
        command: TERMINAL_COMMAND_NAME.to_string(),
        bin_dir: paths.bin_dir.to_string_lossy().into_owned(),
        command_path: paths.command_path.to_string_lossy().into_owned(),
        target_path: paths.target_path.to_string_lossy().into_owned(),
        path_hint: terminal_command_path_hint(&paths.bin_dir),
        message,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalCommandFileState {
    Missing,
    Current,
    ManagedStale,
    Conflict,
}

fn inspect_terminal_command(
    paths: &TerminalCommandPaths,
) -> Result<TerminalCommandFileState, String> {
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(metadata) = fs::symlink_metadata(&paths.command_path) {
            if metadata.file_type().is_symlink() {
                return fs::read_link(&paths.command_path)
                    .map(|target| {
                        if target == paths.target_path {
                            // A direct symlink works today, but the Settings
                            // installer normalizes managed launchers to the
                            // script form so future dispatch stays explicit.
                            TerminalCommandFileState::ManagedStale
                        } else {
                            TerminalCommandFileState::Conflict
                        }
                    })
                    .map_err(|error| {
                        format!("Could not inspect terminal command symlink: {error}")
                    });
            }
        }
    }

    match fs::read_to_string(&paths.command_path) {
        Ok(contents) => {
            if contents == terminal_command_contents(&paths.target_path) {
                return Ok(TerminalCommandFileState::Current);
            }
            if contents.contains(TERMINAL_COMMAND_MARKER) {
                return Ok(TerminalCommandFileState::ManagedStale);
            }
            Ok(TerminalCommandFileState::Conflict)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(TerminalCommandFileState::Missing),
        Err(error) => Err(format!("Could not inspect terminal command: {error}")),
    }
}

fn install_terminal_command() -> Result<(), String> {
    let paths = terminal_command_paths()?;
    match inspect_terminal_command(&paths)? {
        TerminalCommandFileState::Missing => {}
        TerminalCommandFileState::Current => return Ok(()),
        TerminalCommandFileState::ManagedStale => {
            fs::remove_file(&paths.command_path)
                .map_err(|error| format!("Could not replace terminal command: {error}"))?;
        }
        TerminalCommandFileState::Conflict => {
            return Err(format!(
                "{} already exists and is not managed by Kassiber. Move it aside first if you want Kassiber to install this command there.",
                paths.command_path.display()
            ));
        }
    }

    fs::create_dir_all(&paths.bin_dir)
        .map_err(|error| format!("Could not create terminal command directory: {error}"))?;
    fs::write(
        &paths.command_path,
        terminal_command_contents(&paths.target_path),
    )
    .map_err(|error| format!("Could not write terminal command: {error}"))?;
    set_terminal_command_permissions(&paths.command_path)
        .map_err(|error| format!("Could not make terminal command executable: {error}"))?;
    Ok(())
}

fn remove_terminal_command() -> Result<(), String> {
    let paths = terminal_command_paths()?;
    match inspect_terminal_command(&paths)? {
        TerminalCommandFileState::Missing => Ok(()),
        TerminalCommandFileState::Current | TerminalCommandFileState::ManagedStale => {
            fs::remove_file(&paths.command_path)
                .map_err(|error| format!("Could not remove terminal command: {error}"))
        }
        TerminalCommandFileState::Conflict => Err(format!(
            "{} is not managed by Kassiber, so it was left untouched.",
            paths.command_path.display()
        )),
    }
}

#[cfg(not(target_os = "windows"))]
fn set_terminal_command_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
}

#[cfg(target_os = "windows")]
fn set_terminal_command_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn terminal_command_contents(target_path: &Path) -> String {
    let target = target_path.to_string_lossy().replace('%', "%%");
    format!(
        "@echo off\r\nREM {TERMINAL_COMMAND_MARKER}\r\nREM target: {}\r\n\"{}\" --cli %*\r\n",
        target, target
    )
}

#[cfg(not(target_os = "windows"))]
fn terminal_command_contents(target_path: &Path) -> String {
    format!(
        "#!/bin/sh\n# {TERMINAL_COMMAND_MARKER}\n# target: {}\nexec {} --cli \"$@\"\n",
        target_path.to_string_lossy(),
        shell_single_quote(&target_path.to_string_lossy())
    )
}

#[cfg(not(target_os = "windows"))]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn terminal_command_message(
    installed: bool,
    needs_repair: bool,
    conflict: bool,
    path_on_path: bool,
) -> String {
    if conflict {
        "A different command already exists at the install path.".to_string()
    } else if needs_repair {
        "The terminal command is managed by Kassiber but points at an older app path.".to_string()
    } else if installed && path_on_path {
        "The terminal command is installed and appears on PATH.".to_string()
    } else if installed {
        "The terminal command is installed; add its folder to PATH if your shell cannot find it."
            .to_string()
    } else {
        "Install the user-local terminal command to run kassiber from a shell.".to_string()
    }
}

fn terminal_command_path_hint(bin_dir: &Path) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("Add {} to your user PATH.", bin_dir.display())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = home_dir();
        let display_dir = home
            .as_ref()
            .and_then(|home| bin_dir.strip_prefix(home).ok())
            .map(|relative| format!("$HOME/{}", relative.to_string_lossy()))
            .unwrap_or_else(|| bin_dir.to_string_lossy().into_owned());
        format!("export PATH=\"{display_dir}:$PATH\"")
    }
}

fn path_is_on_path(path: &Path) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|candidate| same_path_text(&candidate, path))
}

fn same_path_text(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        left.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['\\', '/']))
    }
    #[cfg(not(target_os = "windows"))]
    {
        left.to_string_lossy().trim_end_matches('/')
            == right.to_string_lossy().trim_end_matches('/')
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

fn is_supported_austrian_csv_bundle_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("kassiber-austrian-e1kv-") && name.contains("-csv-")
}

fn is_supported_audit_package_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("kassiber-audit-package-")
}

fn is_managed_report_export_path(path: &Path) -> bool {
    managed_report_exports_root(path).is_some()
}

fn managed_report_exports_root(path: &Path) -> Option<&Path> {
    let Some(parent) = path.parent() else {
        return None;
    };
    let Some(grandparent) = parent.parent() else {
        return None;
    };
    if parent.file_name().and_then(|name| name.to_str()) == Some("reports")
        && grandparent.file_name().and_then(|name| name.to_str()) == Some("exports")
    {
        Some(grandparent)
    } else {
        None
    }
}

fn is_supported_report_export_target(path: &Path, metadata: &std::fs::Metadata) -> bool {
    if !is_managed_report_export_path(path) {
        return false;
    }
    if metadata.is_file() {
        return is_supported_export_file(path);
    }
    metadata.is_dir()
        && (is_supported_austrian_csv_bundle_dir(path) || is_supported_audit_package_dir(path))
}

fn open_with_default_app(path: &Path) -> Result<(), String> {
    open_path_with_default_app(path, "report export")
}

fn open_path_with_default_app(path: &Path, label: &str) -> Result<(), String> {
    let mut command = default_app_command(path);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open {label} with the default app: {error}"))
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

    // Single-instance must come before the deep-link plugin so GUI/deep-link
    // relaunches are forwarded to the existing window instead of forking a new
    // GUI process. CLI-mode launches intentionally skip the plugin: they run a
    // short-lived sidecar command and exit, so they do not start a second
    // desktop supervisor or hold a competing GUI daemon open.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if cli_args.is_none() {
            builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }));
        }
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
            let supervisor = Arc::new(DaemonSupervisor::new(resource_dir));
            // Unsolicited daemon events (`event: true`, no request_id) —
            // e.g. background freshness records — fan out to the webview
            // on their own channel, separate from per-request
            // `daemon://stream` records.
            let event_app_handle = app.handle().clone();
            supervisor.set_event_sink(move |record| {
                if let Err(error) = event_app_handle.emit("daemon://event", record) {
                    eprintln!("kassiber: failed to emit daemon event: {error}");
                }
            });
            app.manage(supervisor);

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
                            eprintln!("kassiber: ignoring unrecognized launch deep link: {url}");
                        }
                    }
                });
            }
            Ok(())
        })
        .on_menu_event(handle_app_menu_event)
        .invoke_handler(tauri::generate_handler![
            daemon_invoke,
            pick_document_import_source,
            daemon_lifecycle_snapshot,
            open_exported_file,
            open_attachment_file,
            save_exported_file_as,
            save_chat_export_as,
            save_logs_export_as,
            read_ledger_preview_file_base64,
            open_external_url,
            select_import_project_directory,
            activate_import_project,
            clear_import_project,
            touch_id_passphrase_status_command,
            touch_id_store_passphrase_command,
            touch_id_unlock_passphrase_command,
            touch_id_forget_passphrase_command,
            terminal_command_status_command,
            terminal_command_install_command,
            terminal_command_remove_command,
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
            Some("add-wallet") => Some(menu_action("add-wallet")),
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
    let ui_scale_decrease_item = menu_item(
        app,
        MENU_UI_SCALE_DECREASE,
        "Smaller UI",
        Some("CmdOrCtrl+Minus"),
    )?;
    let ui_scale_increase_item = menu_item(
        app,
        MENU_UI_SCALE_INCREASE,
        "Larger UI",
        Some("CmdOrCtrl+Equal"),
    )?;
    let ui_scale_reset_item = menu_item(
        app,
        MENU_UI_SCALE_RESET,
        "Default UI Scale",
        Some("CmdOrCtrl+Digit0"),
    )?;
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
    let logs_item = menu_item(app, MENU_NAV_LOGS, "Logs", None)?;
    let add_wallet_item = menu_item(
        app,
        MENU_WORKFLOW_ADD_WALLET,
        "Add Wallet Connection...",
        Some("CmdOrCtrl+Shift+A"),
    )?;
    let sync_all_item = menu_item(
        app,
        MENU_WORKFLOW_SYNC_ALL,
        "Sync All Wallets",
        Some("CmdOrCtrl+R"),
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
        .item(&quarantine_item)
        .item(&assistant_item)
        .separator()
        .item(&toggle_sensitive)
        .separator()
        .item(&ui_scale_decrease_item)
        .item(&ui_scale_increase_item)
        .item(&ui_scale_reset_item)
        .separator()
        .item(&toggle_fullscreen)
        .build()?;

    let workflow_menu = SubmenuBuilder::new(app, "Workflows")
        .item(&add_wallet_item)
        .separator()
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
        .item(&logs_item)
        .separator()
        .item(&issues_item)
        .build()?;

    #[cfg(not(target_os = "macos"))]
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&docs_item)
        .item(&logs_item)
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
        // them out instead. Logs is gated behind an unlocked workspace and
        // Developer tools; Settings has its own no-identity render.
        overview_item.clone(),
        transactions_item.clone(),
        connections_item.clone(),
        books_item.clone(),
        reports_item.clone(),
        source_funds_item.clone(),
        journals_item.clone(),
        quarantine_item.clone(),
        logs_item.clone(),
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
        MENU_UI_SCALE_DECREASE => Some(menu_action("ui-scale-decrease")),
        MENU_UI_SCALE_INCREASE => Some(menu_action("ui-scale-increase")),
        MENU_UI_SCALE_RESET => Some(menu_action("ui-scale-reset")),
        MENU_WORKFLOW_ADD_WALLET => Some(menu_action("add-wallet")),
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
        MENU_NAV_QUARANTINE => Some(navigate_action("/quarantine")),
        MENU_NAV_ASSISTANT => Some(navigate_action("/assistant")),
        MENU_NAV_LOGS => Some(navigate_action("/logs")),
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
        clear_desktop_biometric_stale_guard_if_matches, copy_report_export_directory,
        database_is_encrypted, desktop_biometric_stale_generation,
        ensure_export_destination_outside_managed_root, inspect_import_project_directory,
        inspect_terminal_command, is_managed_report_export_path, is_supported_audit_package_dir,
        is_supported_austrian_csv_bundle_dir, is_supported_export_file,
        is_supported_report_export_target, managed_settings_path, menu_action,
        menu_action_for_deep_link, menu_action_for_id, navigate_action, open_settings_action,
        path_is_on_path, terminal_command_contents, terminal_command_path_hint,
        touch_id_managed_unlock_state, touch_id_scope_for_selected, validated_attachment_file_path,
        validated_external_url, TerminalCommandFileState, TerminalCommandPaths,
        ALLOWED_DAEMON_KINDS, DEEP_LINK_SETTINGS_SECTIONS, DOCUMENT_IMPORT_STAGE_KIND,
        MENU_HELP_DOCS, MENU_LOCK_APP, MENU_NAV_ASSISTANT, MENU_NAV_REPORTS, MENU_OPEN_SETTINGS,
        MENU_SETTINGS_AI, MENU_SETTINGS_BACKENDS, MENU_SETTINGS_DATA, MENU_SETTINGS_DISPLAY,
        MENU_SETTINGS_GENERAL, MENU_SETTINGS_PRIVACY, MENU_SETTINGS_SECURITY,
        MENU_TOGGLE_FULLSCREEN, MENU_UI_SCALE_DECREASE, MENU_UI_SCALE_INCREASE,
        MENU_UI_SCALE_RESET, MENU_WORKFLOW_ADD_WALLET, MENU_WORKFLOW_CONNECTIONS_IMPORTS,
        MENU_WORKFLOW_OPEN_REPORTS, MENU_WORKFLOW_PROCESS_JOURNALS, MENU_WORKFLOW_SYNC_ALL,
        TERMINAL_COMMAND_MARKER,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tauri::Url;

    #[test]
    fn older_stale_generation_cannot_clear_a_newer_guard() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let state_root = std::env::temp_dir().join(format!(
            "kassiber-touch-id-stale-{}-{suffix}",
            std::process::id()
        ));
        let data_root = state_root.join("data");
        fs::create_dir_all(&data_root).unwrap();
        let data_root = data_root.to_string_lossy().to_string();
        let settings_path = managed_settings_path(&data_root);
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(
            &settings_path,
            b"{\"cli_legacy_unlock_quarantined\":true,\"desktop_biometric_stale\":\"generation-new\",\"keep\":true}\n",
        )
        .unwrap();

        assert_eq!(
            desktop_biometric_stale_generation(&data_root)
                .unwrap()
                .as_deref(),
            Some("generation-new")
        );
        assert!(!clear_desktop_biometric_stale_guard_if_matches(
            &data_root,
            Some("generation-old")
        )
        .unwrap());
        assert_eq!(
            desktop_biometric_stale_generation(&data_root)
                .unwrap()
                .as_deref(),
            Some("generation-new")
        );
        assert!(
            clear_desktop_biometric_stale_guard_if_matches(&data_root, Some("generation-new"))
                .unwrap()
        );
        assert_eq!(
            desktop_biometric_stale_generation(&data_root).unwrap(),
            None
        );
        assert!(touch_id_managed_unlock_state(&data_root).unwrap().0);
        let settings: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(settings_path).unwrap()).unwrap();
        assert_eq!(settings["keep"], serde_json::json!(true));

        let _ = fs::remove_dir_all(state_root);
    }

    #[test]
    fn stale_guard_read_errors_fail_closed() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let state_root = std::env::temp_dir().join(format!(
            "kassiber-touch-id-invalid-settings-{}-{suffix}",
            std::process::id()
        ));
        let data_root = state_root.join("data");
        let settings_path = state_root.join("config").join("settings.json");
        fs::create_dir_all(&data_root).unwrap();
        fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        fs::write(&settings_path, b"not-json\n").unwrap();
        let data_root_text = data_root.to_string_lossy().to_string();
        assert!(desktop_biometric_stale_generation(&data_root_text).is_err());

        fs::remove_file(&settings_path).unwrap();
        fs::create_dir(&settings_path).unwrap();
        assert!(desktop_biometric_stale_generation(&data_root_text).is_err());

        let _ = fs::remove_dir_all(state_root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_data_root_keeps_lexical_settings_scope() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "kassiber-touch-id-symlink-{}-{suffix}",
            std::process::id()
        ));
        let state_root = root.join("state");
        let target = root.join("bookdb");
        fs::create_dir_all(&state_root).unwrap();
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, state_root.join("data")).unwrap();

        let scope = touch_id_scope_for_selected(state_root.join("data"));
        assert_eq!(Path::new(&scope.account), target.canonicalize().unwrap());
        assert_eq!(Path::new(&scope.data_root), state_root.join("data"));
        assert_eq!(
            super::managed_settings_path(&scope.data_root),
            state_root.join("config").join("settings.json")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn document_import_stage_is_not_renderer_allowlisted() {
        assert!(!ALLOWED_DAEMON_KINDS.contains(&DOCUMENT_IMPORT_STAGE_KIND));
        assert!(ALLOWED_DAEMON_KINDS.contains(&"ui.wallets.document_import.preview"));
        assert!(ALLOWED_DAEMON_KINDS.contains(&"ui.wallets.document_import.import"));
    }

    #[test]
    fn terminal_command_launcher_targets_desktop_executable() {
        let target = Path::new("/Applications/Kassiber.app/Contents/MacOS/kassiber-ui");
        let contents = terminal_command_contents(target);
        assert!(contents.contains("Managed by Kassiber Settings"));
        assert!(contents.contains("--cli"));
        assert!(contents.contains("kassiber-ui"));
    }

    #[test]
    fn terminal_command_path_hint_uses_user_shell_path_syntax() {
        let hint = terminal_command_path_hint(Path::new("/Users/alice/.local/bin"));
        #[cfg(target_os = "windows")]
        assert!(hint.contains("PATH"));
        #[cfg(not(target_os = "windows"))]
        assert!(hint.contains("export PATH="));
    }

    #[test]
    fn path_detection_tolerates_missing_path_env() {
        assert!(!path_is_on_path(Path::new(
            "/path/that/should/not/exist/in/tests"
        )));
    }

    #[test]
    fn open_attachment_validation_accepts_managed_file() {
        let root = unique_temp_dir("attachment-open-happy");
        let data_root = root.join("data");
        let attachments_root = root.join("attachments");
        fs::create_dir_all(&attachments_root).expect("create attachments root");
        fs::create_dir_all(&data_root).expect("create data root");
        let file = attachments_root.join("receipt.txt");
        fs::write(&file, "invoice 42").expect("write attachment");

        let validated =
            validated_attachment_file_path(&data_root, &file).expect("validate attachment");

        assert_eq!(validated, file.canonicalize().expect("canonical file"));
    }

    #[test]
    fn open_attachment_validation_rejects_absolute_escape() {
        let root = unique_temp_dir("attachment-open-escape");
        let data_root = root.join("data");
        let attachments_root = root.join("attachments");
        fs::create_dir_all(&attachments_root).expect("create attachments root");
        fs::create_dir_all(&data_root).expect("create data root");
        let outside = root.join("outside.txt");
        fs::write(&outside, "secret").expect("write outside file");

        let error =
            validated_attachment_file_path(&data_root, &outside).expect_err("reject escape");

        assert!(error.contains("managed Kassiber attachment"));
    }

    #[cfg(unix)]
    #[test]
    fn open_attachment_validation_rejects_symlink_escape() {
        let root = unique_temp_dir("attachment-open-symlink");
        let data_root = root.join("data");
        let attachments_root = root.join("attachments");
        fs::create_dir_all(&attachments_root).expect("create attachments root");
        fs::create_dir_all(&data_root).expect("create data root");
        let outside = root.join("outside.txt");
        fs::write(&outside, "secret").expect("write outside file");
        let link = attachments_root.join("linked-outside.txt");
        std::os::unix::fs::symlink(&outside, &link).expect("create symlink");

        let error =
            validated_attachment_file_path(&data_root, &link).expect_err("reject symlink escape");

        assert!(error.contains("managed Kassiber attachment"));
    }

    #[test]
    fn open_attachment_validation_rejects_directory() {
        let root = unique_temp_dir("attachment-open-directory");
        let data_root = root.join("data");
        let attachments_root = root.join("attachments");
        fs::create_dir_all(&attachments_root).expect("create attachments root");
        fs::create_dir_all(&data_root).expect("create data root");

        let error = validated_attachment_file_path(&data_root, &attachments_root)
            .expect_err("reject directory");

        assert!(error.contains("Only attachment files"));
    }

    #[test]
    fn open_attachment_validation_rejects_missing_root() {
        let root = unique_temp_dir("attachment-open-missing-root");
        let data_root = root.join("data");
        fs::create_dir_all(&data_root).expect("create data root");
        let file = root.join("attachments").join("receipt.txt");

        let error =
            validated_attachment_file_path(&data_root, &file).expect_err("reject missing root");

        assert!(error.contains("Attachments folder"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn terminal_command_target_prefers_stable_appimage_path() {
        let appimage = std::ffi::OsString::from("/home/alice/Downloads/Kassiber.AppImage");
        let target = super::terminal_command_target_path_for(Some(appimage), || {
            Ok(PathBuf::from("/tmp/.mount_Kassiber/usr/bin/kassiber-ui"))
        })
        .expect("resolve terminal target");

        assert_eq!(
            target,
            PathBuf::from("/home/alice/Downloads/Kassiber.AppImage")
        );
    }

    #[test]
    fn terminal_command_inspection_tracks_managed_conflict_and_stale_files() {
        let root = unique_temp_dir("terminal-command-inspect");
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let target_path = root
            .join("Kassiber.app")
            .join("Contents")
            .join("MacOS")
            .join("kassiber-ui");
        let paths = TerminalCommandPaths {
            platform: "macos",
            command_path: bin_dir.join("kassiber"),
            bin_dir,
            target_path,
        };

        assert_eq!(
            inspect_terminal_command(&paths).expect("inspect missing command"),
            TerminalCommandFileState::Missing
        );

        fs::write(
            &paths.command_path,
            terminal_command_contents(&paths.target_path),
        )
        .expect("write managed command");
        assert_eq!(
            inspect_terminal_command(&paths).expect("inspect managed command"),
            TerminalCommandFileState::Current
        );

        fs::write(&paths.command_path, "#!/bin/sh\necho elsewhere\n").expect("write conflict");
        assert_eq!(
            inspect_terminal_command(&paths).expect("inspect conflict command"),
            TerminalCommandFileState::Conflict
        );

        fs::write(
            &paths.command_path,
            format!("#!/bin/sh\n# {TERMINAL_COMMAND_MARKER}\nexec /old/kassiber-ui --cli \"$@\"\n"),
        )
        .expect("write stale managed command");
        assert_eq!(
            inspect_terminal_command(&paths).expect("inspect stale command"),
            TerminalCommandFileState::ManagedStale
        );
    }

    #[cfg(unix)]
    #[test]
    fn terminal_command_inspection_normalizes_matching_symlink_to_managed_script() {
        let root = unique_temp_dir("terminal-command-symlink");
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        let target_path = root
            .join("Kassiber.app")
            .join("Contents")
            .join("MacOS")
            .join("kassiber-ui");
        let paths = TerminalCommandPaths {
            platform: "macos",
            command_path: bin_dir.join("kassiber"),
            bin_dir,
            target_path,
        };
        std::os::unix::fs::symlink(&paths.target_path, &paths.command_path)
            .expect("create launcher symlink");

        assert_eq!(
            inspect_terminal_command(&paths).expect("inspect symlink command"),
            TerminalCommandFileState::ManagedStale
        );
    }

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
            "ui.source_funds.assemble",
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
    fn xpub_script_type_daemon_kinds_are_in_allowlist() {
        // The add-wallet auto-detect flow probes script types through this kind;
        // the packaged desktop shell blocks any kind not in ALLOWED_DAEMON_KINDS,
        // so a missing entry would silently break detection in production.
        let required: &[&str] = &[
            "ui.wallets.detect_script_types",
            "ui.wallets.preview_descriptor",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn loans_daemon_kinds_are_in_allowlist() {
        // Pin the collateral-mark daemon surface so the Transactions-screen mark
        // actions come with an explicit allowlist update; otherwise packaged
        // desktop mode returns kind_not_allowed and the feature breaks silently.
        let required: &[&str] = &[
            "ui.loans.list",
            "ui.loans.link",
            "ui.loans.mark",
            "ui.loans.unmark",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn swap_matching_daemon_kinds_are_in_allowlist() {
        // SwapMatching.tsx drives candidate review through these explicit
        // daemon kinds. The packaged desktop shell blocks any kind not listed
        // in ALLOWED_DAEMON_KINDS, so keep this surface pinned.
        let required: &[&str] = &[
            "ui.transfers.suggest",
            "ui.transfers.list",
            "ui.transfers.payouts.list",
            "ui.transfers.payouts.create",
            "ui.transfers.payouts.delete",
            "ui.transfers.pair",
            "ui.transfers.unpair",
            "ui.transfers.update",
            "ui.transfers.bulk_pair",
            "ui.transfers.dismiss",
            "ui.transfers.rules.list",
            "ui.transfers.rules.create",
            "ui.transfers.rules.delete",
            "ui.transfers.rules.set_enabled",
            "ui.transfers.rules.apply",
            "ui.saved_views.list",
            "ui.saved_views.create",
            "ui.saved_views.delete",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn transaction_detail_daemon_kinds_are_in_allowlist() {
        // AppShell tx resolution, the transactions table, and the overview tx
        // detail drive these through the supervisor; packaged desktop mode
        // rejects any unlisted kind (these were missing, causing kind_not_allowed).
        let required: &[&str] = &[
            "ui.transactions.resolve",
            "ui.transactions.graph",
            "ui.transactions.history",
            "ui.transactions.history.revert",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn transaction_attachment_daemon_kinds_are_in_allowlist() {
        // TransactionDetailSheet routes attachment mutations through the
        // daemon supervisor. Packaged desktop mode rejects any unlisted kind.
        let required: &[&str] = &[
            "ui.attachments.list",
            "ui.attachments.add",
            "ui.attachments.copy",
            "ui.attachments.rename",
            "ui.attachments.remove",
            "ui.attachments.open",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn transactions_export_daemon_kinds_are_in_allowlist() {
        // The Transactions screen Export button invokes these directly from the
        // webview; packaged desktop mode rejects any unlisted kind.
        let required: &[&str] = &["ui.transactions.export_csv", "ui.transactions.export_xlsx"];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn report_read_daemon_kinds_are_in_allowlist() {
        // Report panels and post-sync query invalidations call these read
        // kinds directly from the webview. Missing entries show up as
        // kind_not_allowed after a refresh, leaving balances/report cards stale.
        let required: &[&str] = &[
            "ui.reports.capital_gains",
            "ui.reports.summary",
            "ui.reports.balance_sheet",
            "ui.reports.portfolio_summary",
            "ui.reports.balance_history",
            "ui.reports.tax_summary",
            "ui.reports.privacy_hygiene",
            "ui.reports.privacy_mirror",
            "ui.reports.psbt_privacy",
            "ui.reports.lightning_profitability",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn maintenance_daemon_kinds_are_in_allowlist() {
        // SettingsScreen and MarketDataSettingsPanel use these profile
        // maintenance endpoints to expose rate provider and refresh controls.
        let required: &[&str] = &[
            "ui.maintenance.settings",
            "ui.maintenance.configure",
            "ui.maintenance.run",
        ];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn logs_daemon_kinds_are_in_allowlist() {
        // The Logs screen polls the daemon ring through this read-only kind;
        // packaged desktop mode rejects any unlisted kind.
        let required: &[&str] = &["ui.logs.snapshot"];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn wallet_detail_daemon_kinds_are_in_allowlist() {
        // ConnectionDetail.tsx reads wallet-detail panes through these daemon
        // kinds. Missing entries fail at the Tauri shell before the Python
        // daemon can return the feature payload.
        let required: &[&str] = &["ui.wallets.utxos"];
        for kind in required {
            assert!(
                ALLOWED_DAEMON_KINDS.contains(kind),
                "daemon kind missing from Tauri allowlist: {kind}"
            );
        }
    }

    #[test]
    fn native_menu_ids_map_to_webview_actions() {
        let settings_items = [
            (MENU_OPEN_SETTINGS, None),
            (MENU_SETTINGS_GENERAL, None),
            (MENU_SETTINGS_PRIVACY, Some("privacy")),
            (MENU_SETTINGS_DISPLAY, Some("display")),
            (MENU_SETTINGS_SECURITY, Some("security")),
            (MENU_SETTINGS_BACKENDS, Some("backends")),
            (MENU_SETTINGS_AI, Some("ai")),
            (MENU_SETTINGS_DATA, Some("data")),
        ];
        for (menu_id, section) in settings_items {
            assert_eq!(
                menu_action_for_id(menu_id),
                Some(open_settings_action(section)),
                "settings menu id {menu_id} should route to {section:?}"
            );
        }
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
            menu_action_for_id(MENU_UI_SCALE_DECREASE),
            Some(menu_action("ui-scale-decrease"))
        );
        assert_eq!(
            menu_action_for_id(MENU_UI_SCALE_INCREASE),
            Some(menu_action("ui-scale-increase"))
        );
        assert_eq!(
            menu_action_for_id(MENU_UI_SCALE_RESET),
            Some(menu_action("ui-scale-reset"))
        );
        assert_eq!(
            menu_action_for_id(MENU_WORKFLOW_ADD_WALLET),
            Some(menu_action("add-wallet"))
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
        for section in DEEP_LINK_SETTINGS_SECTIONS {
            assert_eq!(
                parse(&format!("kassiber://settings/{section}")),
                Some(open_settings_action(Some(*section))),
                "settings deep link section {section} should be accepted"
            );
        }
        // Unknown sections degrade to "open settings" without a section
        // rather than failing — the user still arrives at the right surface
        // and the menu fallback already handles the missing-hash case.
        assert_eq!(
            parse("kassiber://settings/wallet-of-satoshi"),
            Some(open_settings_action(None))
        );
        assert_eq!(
            parse("kassiber://workflow/add-wallet"),
            Some(menu_action("add-wallet"))
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
    fn austrian_csv_bundle_dirs_are_narrowly_recognized() {
        assert!(is_supported_austrian_csv_bundle_dir(Path::new(
            "kassiber-austrian-e1kv-2026-csv-20260512-101010"
        )));
        assert!(!is_supported_austrian_csv_bundle_dir(Path::new(
            "kassiber-report-20260512-101010"
        )));
        assert!(is_supported_audit_package_dir(Path::new(
            "kassiber-audit-package-20260512-101010"
        )));
        assert!(!is_supported_audit_package_dir(Path::new(
            "kassiber-report-20260512-101010"
        )));

        let root = unique_temp_dir("report-export-target");
        let reports = root.join("exports").join("reports");
        fs::create_dir_all(&reports).expect("create reports dir");
        let csv_file = reports.join("report.csv");
        fs::write(&csv_file, b"header\n").expect("write csv file");
        let bundle_dir = reports.join("kassiber-austrian-e1kv-2026-csv-20260512-101010");
        fs::create_dir_all(&bundle_dir).expect("create bundle dir");
        let audit_dir = reports.join("kassiber-audit-package-20260512-101010");
        fs::create_dir_all(&audit_dir).expect("create audit package dir");
        let nested_dir = reports.join("kassiber-report-20260512-101010");
        fs::create_dir_all(&nested_dir).expect("create unrelated dir");

        assert!(is_supported_report_export_target(
            &csv_file,
            &csv_file.metadata().expect("csv metadata")
        ));
        assert!(is_supported_report_export_target(
            &bundle_dir,
            &bundle_dir.metadata().expect("bundle metadata")
        ));
        assert!(is_supported_report_export_target(
            &audit_dir,
            &audit_dir.metadata().expect("audit metadata")
        ));
        assert!(!is_supported_report_export_target(
            &nested_dir,
            &nested_dir.metadata().expect("nested metadata")
        ));
    }

    #[test]
    fn export_save_destination_must_stay_outside_managed_exports() {
        let root = unique_temp_dir("report-export-destination");
        let reports = root.join("exports").join("reports");
        fs::create_dir_all(&reports).expect("create reports dir");
        let source = reports.join("report.pdf");
        fs::write(&source, b"%PDF").expect("write source file");

        let outside = root.join("downloads").join("report.pdf");
        ensure_export_destination_outside_managed_root(&source, &outside)
            .expect("outside managed exports is allowed");

        let inside = reports.join("copy.pdf");
        let error = ensure_export_destination_outside_managed_root(&source, &inside).unwrap_err();
        assert!(error.contains("outside Kassiber's managed exports"));
    }

    #[test]
    fn csv_bundle_copy_refuses_non_empty_destination() {
        let root = unique_temp_dir("report-export-copy");
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).expect("create source dir");
        fs::create_dir_all(&destination).expect("create destination dir");
        fs::write(source.join("overview.csv"), b"a,b\n1,2\n").expect("write source csv");
        fs::write(destination.join("keep.csv"), b"existing\n").expect("write destination csv");

        let error = copy_report_export_directory(&source, &destination).unwrap_err();
        assert!(error.contains("new or empty folder"));
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
