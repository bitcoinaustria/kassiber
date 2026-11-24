//! components/kb/AddConnectionDialog.tsx: the Add connection dialog, both steps.
//!
//! Open state is `app.ov.add_open` (shared with the Overview, which mounts the dialog through
//! `overview::add_connection`); the picker's search / category / selection are `app.ov.add_*`;
//! everything else lives in `app.wallets.add`.
//!
//! - `picker`: the source step (catalog columns and summary).
//! - `setup`: every setup form (`renderSetupFields`), the import summary and progress.
//! - `submit`: validation (`validateSetupForm`) and the submit pipeline (`onSetupSubmit`) as
//!   a small step machine over daemon replies.
//! - `meta`: the catalog's setup fields per source.
//! - `btcpay`, `exchange`: the BTCPay / exchange API panels; `ledger_preview`: the generic
//!   ledger import preview; `scanner`: the wallet-material scanner; `libs`: address list,
//!   wallet material, Samourai, Wasabi and backend-row helpers.

pub mod bbqr;
pub mod bisq;
pub mod btcpay;
pub mod camera;
pub mod exchange;
pub mod ledger_preview;
pub mod libs;
pub mod meta;
pub mod picker;
pub mod scanner;
mod setup;
mod submit;

use std::collections::HashMap;

use rune::{Color, Cx, El, Key, KeyEvent, Shadow, TextEdit, Tx, col, el, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::overview::add_connection::Source;
use crate::theme::Tk;
use crate::ui::ic;
use meta::{SetupKind, meta, source};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Source,
    Setup,
}

/// `SetupFormState`.
#[derive(Clone)]
pub struct Form {
    pub label: TextEdit,
    pub report_provider: TextEdit,
    pub tax_year: TextEdit,
    pub backend: String,
    pub birthday: TextEdit,
    pub core_url: TextEdit,
    /// "cookiefile" | "basic".
    pub core_auth: &'static str,
    pub core_cookiefile: TextEdit,
    pub core_user: TextEdit,
    pub core_pass: TextEdit,
    pub core_credential_ref: String,
    pub core_network: String,
    /// "wallet_source" | "existing_wallet".
    pub btcpay_csv_mode: &'static str,
    /// "wallet_sources" | "existing_wallets".
    pub bull_mode: &'static str,
    pub bull_networks: Vec<&'static str>,
    pub bull_routes: HashMap<&'static str, String>,
    pub wallet_material: TextEdit,
    pub script_type: String,
    pub sp_descriptor: TextEdit,
    /// "local_index" | "server_assisted".
    pub sp_scan_mode: &'static str,
    pub sp_start_height: TextEdit,
    pub sp_start_date: TextEdit,
    pub sp_full_history: bool,
    pub sp_ack_full: bool,
    pub sp_ack_server: bool,
    pub address_list: TextEdit,
    pub gap_limit: TextEdit,
    pub target_wallet: String,
    pub source_file: TextEdit,
    /// Bisq's Funds > Transactions export, read beside the trade history.
    pub companion_file: TextEdit,
    /// The IANA zone of the computer that wrote a local-time export (Bisq).
    pub source_timezone: TextEdit,
    /// Deposit, Badbank, Premix, Postmix.
    pub samourai: [TextEdit; 4],
    /// "rpc" | "bundle-file".
    pub wasabi_mode: &'static str,
    /// History, Coins, Wallet info, Additional.
    pub wasabi: [TextEdit; 4],
    /// "csv" | "json" (the generic CSV source's format select).
    pub source_format: &'static str,
    /// "relevant" | "full".
    pub bull_import_mode: &'static str,
    pub bip329_wallet: String,
    pub bip329_file: TextEdit,
    /// "stored" | "synthesized" | "all".
    pub bip329_export_mode: &'static str,
    pub sync_after: bool,
}

pub const CORE_DEFAULT_RPC_URLS: [(&str, &str); 4] = [
    ("main", "http://127.0.0.1:8332"),
    ("test", "http://127.0.0.1:18332"),
    ("signet", "http://127.0.0.1:38332"),
    ("regtest", "http://127.0.0.1:18443"),
];

pub fn core_default_url(network: &str) -> &'static str {
    CORE_DEFAULT_RPC_URLS
        .iter()
        .find(|(n, _)| *n == network)
        .map(|(_, u)| *u)
        .unwrap_or(CORE_DEFAULT_RPC_URLS[0].1)
}

/// This computer's IANA zone, the usual source of a Bisq export (the web's
/// `Intl.DateTimeFormat().resolvedOptions().timeZone`): `TZ` when it names a zone, else the zone
/// `/etc/localtime` links to; empty when neither says (the owner types it).
pub fn local_time_zone() -> String {
    let tz = std::env::var("TZ").unwrap_or_default();
    let from_env = zone_name(tz.trim_start_matches(':'));
    if !from_env.is_empty() {
        return from_env;
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_str().map(zone_from_link))
        .unwrap_or_default()
}

/// The `Area/City` part of a zoneinfo path, or a bare name that looks like one.
fn zone_from_link(path: &str) -> String {
    path.rsplit_once("zoneinfo/")
        .map(|(_, z)| zone_name(z))
        .unwrap_or_default()
}

fn zone_name(z: &str) -> String {
    let ok = !z.is_empty()
        && !z.starts_with('/')
        && (z.contains('/') || z == "UTC")
        && z.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
    if ok { z.to_owned() } else { String::new() }
}

impl Form {
    /// `formDefaultsFor(source)`.
    pub fn defaults(src: &Source) -> Form {
        let m = meta(src.id);
        let label = if src.id == "csv" {
            t("connections:add.defaultLabel.importedFile")
        } else if src.id == "bip329" {
            String::new()
        } else if let Some(ex) = m.exchange {
            exchange::provider_name(ex).to_owned()
        } else {
            src.title()
        };
        let te = |s: &str| TextEdit::new(s);
        Form {
            label: te(&label),
            report_provider: te(""),
            tax_year: te(""),
            backend: String::new(),
            birthday: te(""),
            core_url: te(CORE_DEFAULT_RPC_URLS[0].1),
            core_auth: "cookiefile",
            core_cookiefile: te(""),
            core_user: te(""),
            core_pass: te(""),
            core_credential_ref: String::new(),
            core_network: m.network.unwrap_or("main").to_owned(),
            btcpay_csv_mode: "wallet_source",
            bull_mode: "wallet_sources",
            bull_networks: vec!["bitcoin", "liquid", "lightning"],
            bull_routes: HashMap::new(),
            wallet_material: te(""),
            script_type: String::new(),
            sp_descriptor: te(""),
            sp_scan_mode: "local_index",
            sp_start_height: te(""),
            sp_start_date: te(""),
            sp_full_history: false,
            sp_ack_full: false,
            sp_ack_server: false,
            address_list: te(""),
            gap_limit: te("40"),
            target_wallet: String::new(),
            source_file: te(""),
            companion_file: te(""),
            source_timezone: te(&local_time_zone()),
            samourai: [te(""), te(""), te(""), te("")],
            wasabi_mode: if m.source_format == Some("wasabi_bundle") {
                "rpc"
            } else {
                "bundle-file"
            },
            wasabi: [te(""), te(""), te(""), te("")],
            source_format: "csv",
            bull_import_mode: "relevant",
            bip329_wallet: String::new(),
            bip329_file: te(""),
            bip329_export_mode: "stored",
            // An exchange sync waits for an explicit opt-in: saving alone never contacts it.
            sync_after: matches!(
                m.kind(),
                SetupKind::Descriptor
                    | SetupKind::FileWallet
                    | SetupKind::BullbitcoinWallet
                    | SetupKind::AddressList
            ),
        }
    }
}

impl Default for Form {
    fn default() -> Self {
        Form::defaults(&crate::overview::catalog::SOURCES[0])
    }
}

/// `WalletCoreCoverageState`.
#[derive(Clone, Debug, Default)]
pub enum Coverage {
    #[default]
    Idle,
    Checking,
    Available(Value),
    BirthdayRequired(Value),
    VerificationRequired(Value),
    Blocked {
        message: String,
        details: Value,
    },
    Error {
        message: String,
        details: Value,
    },
}

/// The Bitcoin Core "Test connection" outcome (`coreTestStatus`).
#[derive(Clone, Debug)]
pub enum CoreTest {
    Ok(Value),
    Failed {
        message: String,
        hint: Option<String>,
    },
}

/// What a save dialog does with the chosen path (`None` when cancelled).
pub type SaveDone = Box<dyn Fn(&mut App, Option<String>)>;

pub struct PendingSave {
    pub file: String,
    pub title: String,
    pub default_name: String,
    pub filter: (String, Vec<&'static str>),
    pub done: SaveDone,
}

#[derive(Default)]
pub struct State {
    pub btcpay: btcpay::State,
    pub exchange: exchange::State,
    pub ledger: ledger_preview::State,
    pub scanner: scanner::State,
    pub step: Step,
    pub form: Form,
    /// The source the form was built for.
    pub form_for: &'static str,
    pub errors: HashMap<&'static str, String>,
    pub setup_error: Option<String>,
    /// An import refused for rows that look like rows the book has, waiting for an answer.
    pub refused: Option<submit::Refused>,
    /// The asset preview of the source a new provider wallet imports, and the user's answer.
    pub asset_ask: Option<crate::wallets::asset_choice::Ask>,
    /// The source of the asset preview in flight.
    pub asset_ask_pending: Option<crate::wallets::asset_choice::Source>,
    /// An exchange's error from the asset preview or the first sync after it, in the
    /// exchange's own wording.
    pub asset_error: Option<exchange::model::ErrorInfo>,
    /// The source changed after the asset question; it was asked again.
    pub asset_notice: Option<String>,
    /// The last import's result (`lastImportResult`, an `ImportFileResult`).
    pub last_import: Option<Value>,
    /// The arguments of the file import behind `last_import`, when this dialog chose its path
    /// (`ImportFileResult.localReplay`): tracking other crypto assets imports it again.
    pub last_import_replay: Option<crate::wallets::assets::model::Replay>,
    pub bip329_preview: Option<Value>,
    /// Derived addresses (`previewAddresses`): branch, index, address.
    pub preview_addresses: Option<Vec<(String, u64, String)>>,
    pub preview_error: Option<String>,
    pub preview_pending: bool,
    /// Bumped whenever wallet material / script type / form resets, dropping stale preview replies.
    pub preview_gen: u64,
    pub preview_req: Option<(u64, Option<String>)>,
    /// Key material removed from the address list: (private, public).
    pub purged: Option<(usize, usize)>,
    pub core_detection: Option<Vec<Value>>,
    pub core_detect_pending: bool,
    pub core_detect_req: Option<(u64, Option<String>)>,
    pub core_test: Option<CoreTest>,
    pub core_test_pending: bool,
    pub core_test_req: Option<(u64, Option<String>)>,
    pub coverage: Coverage,
    /// Bumped whenever the coverage inputs change, so a stale probe reply is dropped.
    pub coverage_gen: u64,
    pub coverage_req: Option<(u64, Option<String>)>,
    /// Bumped on open/reset so stale Core detect/test replies are dropped.
    pub form_gen: u64,
    pub copied_address: Option<String>,
    pub template_pending: bool,
    pub bip329_export_pending: bool,
    /// The open native select (its trigger key).
    pub select: Option<String>,
    /// The open date + year picker popover.
    pub date_picker: Option<crate::ui::date_picker::PickerState>,
    /// The running submit (`isSubmitting`) shown in the open dialog.
    pub job: Option<submit::Job>,
    /// Dismissed submits still finishing required writes or awaiting cancellation exit.
    pub background_jobs: Vec<submit::Job>,
    /// A daemon export to copy to a path the user picks next poll.
    pub pending_save: Option<PendingSave>,
    /// `deferredConnectionSetup`: the source to reopen when Wallets is shown again.
    pub deferred: Option<&'static str>,
    /// Focus the search field on the next poll (`onOpenAutoFocus`).
    pub focus_search: bool,
    /// Live scan/sync progress during `Stage::Sync`.
    pub progress: Option<SyncProgress>,
    /// A review history import (`initialTargetWalletId` defined): the file formats only, into
    /// an existing wallet or a new one.
    pub history: Option<History>,
    /// The browser's validation bubble on the first empty required control.
    pub required_hint: Option<RequiredHint>,
    /// Focus that control on the next poll.
    pub focus_required: Option<&'static str>,
    /// The dialog's open-time setup ran (an Overview button opens it through the shared
    /// `overview::open_add_connection`, which knows nothing of this state).
    pub opened: bool,
}

/// Live progress for a wallet scan/sync started by Add connection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncProgress {
    pub wallet: String,
    pub job_id: Option<String>,
    pub phase: Option<String>,
    pub processed: u64,
    pub total: u64,
    pub percent: f64,
}

/// Review's "Import history" (`initialTargetWalletId`): the wallet the rows go into (`""`: none
/// chosen yet, `"__new__"`: a new wallet), and whether the format was seeded or chosen.
#[derive(Clone, Debug, Default)]
pub struct History {
    pub target: String,
    pub initial: Option<String>,
    pub chosen: bool,
}

/// The setup fields drawn as a multi-line box (`setup::textarea`).
const TEXTAREAS: [&str; 11] = [
    "connection-wallet-material",
    "connection-address-list",
    "connection-sp-descriptor",
    "connection-wasabi-history",
    "connection-wasabi-coins",
    "connection-wasabi-info",
    "connection-wasabi-additional",
    "connection-samourai-deposit",
    "connection-samourai-badbank",
    "connection-samourai-premix",
    "connection-samourai-postmix",
];

/// How long the validation bubble stays up (WebKit hides it after a few seconds).
pub const REQUIRED_HINT_SECS: f64 = 5.0;

#[derive(Clone, Debug)]
pub struct RequiredHint {
    pub key: &'static str,
    pub message: &'static str,
    pub until: f64,
}

/// `isHistoryImportSource`: a file format that imports rows into a wallet.
pub fn is_history_source(src: &Source) -> bool {
    let m = meta(src.id);
    m.source_format.is_some()
        && m.forward_to.is_none()
        && matches!(m.kind(), SetupKind::FileWallet | SetupKind::FileEnrichment)
}

/// `historyImport`.
pub fn history_import(app: &App) -> bool {
    st(app).history.is_some()
}

/// `historyUsesExisting`.
pub fn history_uses_existing(app: &App) -> bool {
    st(app)
        .history
        .as_ref()
        .is_some_and(|h| h.target != "__new__")
}

/// `historyTarget`: the chosen wallet in `ui.wallets.list`.
pub fn history_target(app: &App) -> Option<Value> {
    let id = st(app).history.as_ref()?.target.clone();
    submit::wallets(app)
        .into_iter()
        .find(|w| w.get("id").and_then(Value::as_str) == Some(id.as_str()))
}

/// `knownWalletImportSource`: a stored provider kind selects its parser; a generic wallet
/// (or a kind several formats share) does not.
pub fn known_wallet_import_source(kind: Option<&str>) -> Option<&'static Source> {
    let kind = kind.filter(|k| !["custom", "address", "descriptor"].contains(k))?;
    let matches: Vec<&'static Source> = crate::overview::catalog::SOURCES
        .iter()
        .filter(|s| meta(s.id).wallet_kind == Some(kind) && !s.planned && is_history_source(s))
        .collect();
    (matches.len() == 1).then(|| matches[0])
}

/// `sourceForConnectionCategory`.
pub fn source_for_category(category: &str, history: bool) -> Option<&'static Source> {
    crate::overview::catalog::SOURCES
        .iter()
        .find(|s| s.category == category && (!history || is_history_source(s)))
}

/// Review's "Import history" for `wallet` (None: the user picks the target): the dialog opens on
/// the CSV format among the file formats, and once the wallet list is in, a wallet whose kind
/// names one parser opens straight on that format's setup.
pub fn open_history(app: &mut App, wallet: Option<String>) {
    open(app, None);
    let csv = source("csv").unwrap_or(&crate::overview::catalog::SOURCES[0]);
    app.ov.add_category = Some("files");
    app.ov.add_selected = Some(csv.id);
    reset_form(app, csv);
    let s = stm(app);
    s.step = Step::Source;
    s.history = Some(History {
        target: wallet.clone().unwrap_or_default(),
        initial: wallet,
        chosen: false,
    });
    seed_history_source(app);
}

/// Seeds the format once, after the wallet list arrives (later refetches and target changes
/// keep what the user chose).
pub fn seed_history_source(app: &mut App) {
    let Some(h) = st(app).history.clone() else {
        return;
    };
    if h.chosen || app.read("ui.wallets.list").is_none() || !app.ov.add_open {
        return;
    }
    if let Some(h) = stm(app).history.as_mut() {
        h.chosen = true;
    }
    let kind = submit::wallets(app)
        .into_iter()
        .find(|w| {
            h.initial.is_some() && w.get("id").and_then(Value::as_str) == h.initial.as_deref()
        })
        .and_then(|w| w.get("kind").and_then(Value::as_str).map(str::to_owned));
    let Some(src) = known_wallet_import_source(kind.as_deref()) else {
        return;
    };
    app.ov.add_category = Some(src.category);
    app.ov.add_selected = Some(src.id);
    reset_form(app, src);
    stm(app).step = Step::Setup;
}

/// The user picked a format or category: the seed no longer applies.
pub fn mark_history_chosen(app: &mut App) {
    if let Some(h) = stm(app).history.as_mut() {
        h.chosen = true;
    }
}

pub fn st(app: &App) -> &State {
    &app.wallets.add
}
pub fn stm(app: &mut App) -> &mut State {
    &mut app.wallets.add
}

/// The selected source (`selected`).
pub fn selected(app: &App) -> &'static Source {
    picker::selected(app)
}

pub fn submitting(app: &App) -> bool {
    st(app).job.is_some()
}

/// Opens the dialog; `initial_source` opens straight on that source's setup step
/// (`initialSourceId`, with the `xpub` → `descriptor` alias).
pub fn open(app: &mut App, initial_source: Option<&str>) {
    // A running submit keeps its dialog, as `close` does: its answers still need the job (an
    // assistant request can open the dialog at any time).
    if submitting(app) {
        app.ov.add_open = true;
        return;
    }
    crate::overview::open_add_connection(app);
    let btcpay = std::mem::take(&mut app.wallets.add.btcpay);
    let exchange = std::mem::take(&mut app.wallets.add.exchange);
    let ledger = ledger_preview::restart(std::mem::take(&mut app.wallets.add.ledger));
    let background_jobs = std::mem::take(&mut app.wallets.add.background_jobs);
    let form_gen = app.wallets.add.form_gen + 1;
    let preview_gen = app.wallets.add.preview_gen + 1;
    let coverage_gen = app.wallets.add.coverage_gen + 1;
    let deferred = app.wallets.add.deferred;
    app.wallets.add = State {
        btcpay,
        exchange,
        ledger,
        background_jobs,
        form_gen,
        preview_gen,
        coverage_gen,
        focus_search: true,
        opened: true,
        deferred,
        ..State::default()
    };
    let resolved = initial_source.map(|s| if s == "xpub" { "descriptor" } else { s });
    let requested = resolved.and_then(source);
    let src = requested.unwrap_or(&crate::overview::catalog::SOURCES[0]);
    app.ov.add_category = Some(src.category);
    app.ov.add_selected = Some(src.id);
    reset_form(app, src);
    app.wallets.add.step =
        if requested.is_some() && !src.planned && meta(src.id).forward_to.is_none() {
            Step::Setup
        } else {
            Step::Source
        };
    load_reads(app);
}

/// The dialog's local reads on open. Local only.
fn load_reads(app: &mut App) {
    app.send("ui.backends.options", Value::Null, "wallets.add.backends");
    app.send(
        "ui.backends.settings.list",
        Value::Null,
        "wallets.add.settings",
    );
    app.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
}

/// The web's `useEffect([selected])`: a fresh form and cleared results for `src`.
pub fn reset_form(app: &mut App, src: &'static Source) {
    let s = stm(app);
    s.form = Form::defaults(src);
    s.form_for = src.id;
    s.errors.clear();
    s.setup_error = None;
    s.last_import = None;
    s.refused = None;
    s.asset_ask = None;
    s.asset_error = None;
    s.asset_notice = None;
    s.bip329_preview = None;
    s.preview_addresses = None;
    s.preview_error = None;
    s.preview_pending = false;
    s.preview_gen += 1;
    s.preview_req = None;
    s.core_detection = None;
    s.core_detect_pending = false;
    s.core_detect_req = None;
    s.core_test = None;
    s.core_test_pending = false;
    s.core_test_req = None;
    s.coverage = Coverage::Idle;
    s.coverage_gen += 1;
    s.coverage_req = None;
    s.form_gen += 1;
    s.template_pending = false;
    s.bip329_export_pending = false;
    s.progress = None;
    s.purged = None;
    s.select = None;
    s.date_picker = None;
    s.required_hint = None;
    s.focus_required = None;
    ledger_preview::set_source(app, None);
    btcpay::reset(app);
    if let Some(ex) = meta(src.id).exchange {
        exchange::reset(app, ex);
    }
    submit::apply_default_backend(app);
}

/// A source, format or target changed: the last result and an unanswered refusal belong to the
/// old inputs.
pub fn inputs_changed(app: &mut App) {
    let s = stm(app);
    s.last_import = None;
    s.refused = None;
    s.asset_notice = None;
}

pub fn close(app: &mut App) {
    if submitting(app) {
        submit::dismiss_job(app);
    }
    app.ov.add_open = false;
    let s = stm(app);
    s.select = None;
    s.date_picker = None;
    s.required_hint = None;
    s.focus_required = None;
}

/// `selectSourceForSetup`.
pub fn select_source_for_setup(app: &mut App, id: &str) {
    mark_history_chosen(app);
    let Some(src) = source(id) else { return };
    if history_import(app) && !is_history_source(src) {
        return;
    }
    app.ov.add_category = Some(src.category);
    app.ov.add_selected = Some(src.id);
    app.ov.add_search = TextEdit::new("");
    reset_form(app, src);
    stm(app).step = if !src.planned && meta(src.id).forward_to.is_none() {
        Step::Setup
    } else {
        Step::Source
    };
}

/// The scanner's result (`onMaterialScanned`).
pub fn on_material_scanned(app: &mut App, material: String) {
    let s = stm(app);
    s.form.wallet_material = TextEdit::new(&material);
    s.form.script_type.clear();
    s.preview_addresses = None;
    s.preview_error = None;
    s.preview_pending = false;
    s.preview_gen += 1;
    s.preview_req = None;
    s.errors.remove("walletMaterial");
}

/// `canContinue`.
fn can_continue(app: &App, src: &Source) -> bool {
    let m = meta(src.id);
    !src.planned
        && (m.kind() != SetupKind::Planned || m.forward_to.is_some())
        && (!history_import(app) || is_history_source(src))
}

/// Continue: advance to the setup step (the forward target's for brand sources) without
/// submitting or showing validation errors on an untouched form.
fn continue_to_setup(app: &mut App) {
    let src = selected(app);
    if !can_continue(app, src) {
        return;
    }
    let target = meta(src.id).forward_to.and_then(source).unwrap_or(src);
    app.ov.add_selected = Some(target.id);
    if st(app).form_for != target.id {
        reset_form(app, target);
    }
    let s = stm(app);
    s.step = Step::Setup;
    s.required_hint = None;
    s.focus_required = None;
}

pub fn dialog(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) {
    // `--kb-dialog-top: max(1rem, 8vh)`, `sm:h-[740px] sm:max-w-[960px] lg:max-w-[1040px]`,
    // never past `--kb-dialog-max-h` (the window less the top and 1 rem).
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let top = crate::shell::dialog_top(tk, win_h);
    let max_w: f32 = if win_w >= 1024.0 { 1040.0 } else { 960.0 };
    let width = max_w.min(win_w - tk.r(2.0));
    let height = 740.0f32.min(win_h - top - tk.r(1.0));
    let src = selected(app);
    let s = st(app);
    let setup_step = s.step == Step::Setup;
    let busy = submitting(app);

    // Header: `px-5 pt-5 pb-4 pr-12`, title `text-lg leading-none font-semibold`, `gap-2`.
    let title = if setup_step {
        tr("connections:add.setupTitle", &[("title", &src.title())])
    } else {
        t("connections:add.title")
    };
    let desc = if setup_step {
        t("connections:add.setupDescription")
    } else {
        t("connections:add.description")
    };
    let header = col()
        .w_full()
        .pl(tk.r(1.25))
        .pr(tk.r(3.0))
        .pt(tk.r(1.25))
        .pb(tk.r(1.0))
        .gap(tk.r(0.5))
        .child(
            text(title.clone())
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading"),
        )
        .child(text(desc).font(tk.tsm(400.0)).color(tk.muted_fg));
    // Open select lists mount above the dialog (after it), or its modal would cover them.
    let mut popups = Vec::new();
    let body_inner = if setup_step {
        setup::body(app, cx, tk, &mut popups)
    } else {
        picker::body(app, cx, tk)
    };
    let mut body = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout();
    if history_import(app) && setup_step && meta(src.id).kind() == SetupKind::FileWallet {
        body = body.child(setup::history_target_block(app, tk, &mut popups));
    }
    let body = body.child(col().w_full().grow(1.0).min_h(0.0).child(body_inner));

    // Footer: `gap-2 border-t px-5 py-3 sm:justify-between`.
    let mut footer = row()
        .w_full()
        .px(tk.r(1.25))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .justify_between()
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout();
    footer = if setup_step {
        let b = picker::button(tk, "add-back", &t("connections:add.back"), false, !busy);
        footer.child(if busy {
            b
        } else {
            b.on_press(|a: &mut App| stm(a).step = Step::Source)
        })
    } else {
        footer.child(el())
    };
    let cancel =
        picker::button(tk, "add-cancel", &t("common:actions.cancel"), false, true).on_press(close);
    let primary = if setup_step {
        let m = meta(src.id);
        let enabled = !busy
            && can_continue(app, src)
            && m.kind() != SetupKind::Planned
            && !submit::missing_backend(app)
            && !(m.source_format == Some("generic_ledger") && ledger_preview::blocks_submit(app));
        let b = picker::button(tk, "add-submit", &submit::submit_label(app), true, enabled);
        if enabled {
            b.on_press(submit::submit)
        } else {
            b
        }
    } else {
        let ready = can_continue(app, src);
        let b = picker::button(
            tk,
            "add-continue",
            &t(if !src.planned {
                "connections:add.continue"
            } else {
                "connections:add.plannedLabel"
            }),
            true,
            ready,
        );
        if ready {
            b.on_press(continue_to_setup)
        } else {
            b
        }
    };
    footer = footer.child(row().gap(tk.r(0.5)).child(cancel).child(primary));

    // Close: `absolute top-3 right-3`, a 25.2 pt ghost square at 70 % with an X.
    let close_btn = row()
        .key("add-close")
        .absolute(width - 2.0 - tk.r(0.75) - 25.2, tk.r(0.75))
        .size(25.2, 25.2)
        .center()
        .radius(tk.r(0.125))
        .opacity(0.7)
        .role("button")
        .label(t("common:actions.close"))
        .focusable(true)
        .on_press(close)
        .child(ic(&I::X, tk.r(1.0), tk.fg));

    // `.kb-glass-dialog`: the background at 80 % over a 12–16 pt backdrop blur, a hairline
    // and a long soft shadow; 1 rem corners.
    let (rim, shadow) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            Shadow {
                y: 24.0,
                blur: 72.0,
                spread: -20.0,
                color: Color::BLACK.alpha(0.9),
                ..Shadow::default()
            },
        )
    } else {
        (
            tk.fg.alpha(0.1),
            Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Shadow::default()
            },
        )
    };
    let panel = col()
        .key("add-connection-dialog")
        .role("dialog")
        .label(title)
        .w(width)
        .h(height)
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(if tk.dark { 16.0 } else { 12.0 })
        .transition(crate::shell::dialog_enter())
        .border(1.0, rim)
        .border_layout()
        .box_shadow(shadow)
        .clip_rounded(tk.r(1.0))
        .modal(true, close)
        .child(header)
        .child(body)
        .child(footer)
        .child(close_btn);
    // `.kb-glass-backdrop`: black at 22 % in dark, the foreground at 6 % in light.
    layer.push(crate::shell::window_backdrop("add-scrim", tk.glass_backdrop).a11y_hidden());
    layer.push(crate::shell::centered(panel, win_w, width, top));
    layer.extend(popups);
    setup::selects(app, tk, layer);
    if let Some(h) = &st(app).required_hint {
        layer.push(required_bubble(tk, h));
    }
    if scanner::is_open(app) {
        scanner::view(app, cx, tk, layer);
    }
}

/// The browser's constraint-validation bubble under the first empty required control: a
/// warning glyph and the message, announced as an alert. It goes on the next key or press, or
/// after a few seconds.
fn required_bubble(tk: &Tk, h: &RequiredHint) -> El {
    row()
        .key("add-required-hint")
        // Under the control's box: a textarea's text field is only its first line.
        .anchored(
            if TEXTAREAS.contains(&h.key) {
                format!("{}-box", h.key)
            } else {
                h.key.to_owned()
            },
            rune::Side::Bottom,
            rune::Align::Start,
            6.0,
        )
        .px(tk.r(0.625))
        .py(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .border_layout()
        .box_shadow(Shadow {
            y: 4.0,
            blur: 12.0,
            color: Color::BLACK.alpha(0.18),
            ..Shadow::default()
        })
        .role("alert")
        .label(h.message)
        .on_press(|a: &mut App| stm(a).required_hint = None)
        .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.warn.solid).a11y_hidden())
        .child(text(h.message).font(tk.tsm(400.0)).color(tk.fg))
}

/// Every frame (any route): the BTCPay authorize URL, a pending export save, search focus,
/// and reopening a deferred setup when Wallets is shown again.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    if app.ov.add_open && !st(app).opened {
        // Opened from the Overview: the same setup `open` does, on the picker.
        let (search, cat) = (std::mem::take(&mut app.ov.add_search), app.ov.add_category);
        open(app, None);
        app.ov.add_search = search;
        app.ov.add_category = cat;
        changed = true;
    }
    if !app.ov.add_open && st(app).opened && !submitting(app) {
        stm(app).opened = false;
        // The dialog's content unmounts with it: typed exchange keys, the test outcome and the
        // link error go (the sync status the connection page shows stays).
        let ex = std::mem::take(&mut stm(app).exchange.exchange);
        exchange::reset(app, &ex);
    }
    if !app.ov.add_open && scanner::is_open(app) {
        // The dialog went away under the scanner: its camera stops with it.
        stm(app).scanner = scanner::State::default();
        changed = true;
    }
    if app.ov.add_open {
        // The web's `useEffect([selected])`: a new selection gets a fresh form.
        let sel = selected(app);
        if st(app).form_for != sel.id {
            reset_form(app, sel);
            changed = true;
        }
        changed |= btcpay::poll(app, tx);
        changed |= scanner::poll(app);
        if let Some(key) = stm(app).focus_required.take() {
            tx.focus(key.to_owned(), true);
            changed = true;
        }
        if let Some(until) = st(app).required_hint.as_ref().map(|h| h.until) {
            if app.clock >= until {
                stm(app).required_hint = None;
                changed = true;
            } else {
                tx.wake_after(std::time::Duration::from_secs_f64(
                    (until - app.clock).max(0.05),
                ));
            }
        }
        if st(app).focus_search && st(app).step == Step::Source {
            stm(app).focus_search = false;
            tx.focus("add-search".to_owned(), false);
        }
        if let Some(p) = stm(app).pending_save.take() {
            let opts = rune::FileDialogOptions {
                title: p.title.clone(),
                file_name: Some(p.default_name.clone()),
                ..Default::default()
            }
            .filter(p.filter.0.clone(), &p.filter.1);
            let file = p.file.clone();
            let done = p.done;
            tx.save_file(
                opts,
                move |a: &mut App, result, _tx: &mut Tx| match result {
                    Ok(Some(path)) => match std::fs::copy(&file, &path) {
                        Ok(_) => done(a, Some(path.to_string_lossy().into_owned())),
                        Err(e) => stm(a).setup_error = Some(e.to_string()),
                    },
                    // Cancelled: the export stays in the book's exports folder.
                    Ok(None) => done(a, None),
                    Err(e) => stm(a).setup_error = Some(e.0),
                },
            );
            changed = true;
        }
    } else if app.route.path() == "/connections"
        && let Some(id) = stm(app).deferred.take()
    {
        open(app, Some(id));
        changed = true;
    }
    changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    btcpay::on_reply(app, tag, r);
    exchange::on_reply(app, tag, r);
    ledger_preview::on_reply(app, tag, r);
    if tag.starts_with("wallets.add.") {
        submit::on_reply(app, tag, r);
    }
}

pub fn on_progress(app: &mut App, tag: &str, request_id: &str, env: &Value) {
    submit::on_progress(app, tag, request_id, env);
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if !app.ov.add_open {
        return false;
    }
    if scanner::is_open(app) && scanner::on_key(app, ev, tx) {
        return true;
    }
    // Any key dismisses the validation bubble (and still does what it does).
    stm(app).required_hint = None;
    if matches!(ev.key, Key::Escape) {
        if let Some(p) = stm(app).date_picker.take() {
            tx.focus(p.key, true);
        } else if st(app).select.is_some() {
            stm(app).select = None;
        } else {
            close(app);
        }
        return true;
    }
    false
}

/// Headless: pump until the dialog's reads (and any preview read) are in.
pub fn settle(app: &mut App) {
    crate::wallets::common::settle(app);
    ledger_preview::prime(app);
}

fn scene_setup(a: &mut App, id: &str) {
    crate::wallets::scene_route(a, "/connections");
    open(a, Some(id));
    settle(a);
}

/// The source step on a category with the given source selected.
fn scene_pick(a: &mut App, id: &str) {
    crate::wallets::scene_route(a, "/connections");
    open(a, None);
    if let Some(s) = source(id) {
        a.ov.add_category = Some(s.category);
        a.ov.add_selected = Some(s.id);
        reset_form(a, s);
    }
    settle(a);
    submit::apply_default_backend(a);
}

/// As the web gets there: pick, then Continue (which also submits, see `continue_to_setup`).
fn scene_continue(a: &mut App, id: &str) {
    scene_pick(a, id);
    continue_to_setup(a);
    settle(a);
}

/// The generic ledger form with a synthetic BTC-only file picked (its preview read is local).
fn scene_ledger(a: &mut App, file: &str) {
    scene_setup(a, "generic-ledger");
    // Synthetic, BTC-only sample files kept beside this module.
    let path = format!(
        "{}/src/wallets/add/fixtures/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    setup::pick_ledger_file(a, path);
    settle(a);
}

fn scene_scanner(a: &mut App) {
    scene_setup(a, "descriptor");
    scanner::open(
        a,
        tr(
            "connections:scanner.scanTitle",
            &[("title", &selected(a).title())],
        ),
    );
}

/// Review's "Import history" for the book's first wallet (or no wallet: the user picks one).
fn scene_history(a: &mut App, with_wallet: bool) {
    crate::wallets::scene_route(a, "/review");
    a.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
    settle(a);
    let first = submit::wallets(a)
        .first()
        .and_then(|w| w.get("id").and_then(Value::as_str))
        .map(str::to_owned);
    open_history(a, if with_wallet { first } else { None });
    settle(a);
    seed_history_source(a);
}

pub const SCENES: &[crate::app::Scene] = &[
    ("wallets-add-history", |a| scene_history(a, true)),
    ("wallets-add-history-pick", |a| scene_history(a, false)),
    ("wallets-add-history-setup", |a| {
        scene_history(a, true);
        let csv = source("csv").expect("csv");
        a.ov.add_selected = Some(csv.id);
        reset_form(a, csv);
        stm(a).step = Step::Setup;
    }),
    // As the web's parity recipes reach the panels: pick, then Continue (which submits).
    ("wallets-add-btcpay-continue", |a| {
        scene_continue(a, "btcpay")
    }),
    ("wallets-add-btcpay-manual-key-continue", |a| {
        scene_continue(a, "btcpay");
        a.wallets.add.btcpay.preset = "wallet_history";
        a.wallets.add.btcpay.manual_open = true;
    }),
    ("wallets-add-exchange-kraken-continue", |a| {
        scene_continue(a, "kraken")
    }),
    ("wallets-add-exchange-coinbase-continue", |a| {
        scene_continue(a, "coinbase")
    }),
    ("wallets-add-exchange-binance-continue", |a| {
        scene_continue(a, "binance-api")
    }),
    ("wallets-add-exchange-bitpanda-continue", |a| {
        scene_continue(a, "bitpanda")
    }),
    ("wallets-add-exchange-kraken-light-continue", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_continue(a, "kraken");
    }),
    ("wallets-add-ledger-preview", |a| {
        scene_ledger(a, "ledger.csv")
    }),
    ("wallets-add-ledger-preview-unrecognized", |a| {
        scene_ledger(a, "unknown.csv")
    }),
    ("wallets-add-ledger-preview-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_ledger(a, "ledger.csv");
    }),
    ("wallets-add-scanner", scene_scanner),
    // A BBQR set read one frame in three, and a single descriptor QR read (fed as the camera
    // would hand them over; headless runs open no camera).
    ("wallets-add-scanner-bbqr", |a| {
        scene_scanner(a);
        // "B$2U0300" + Base32 of `wpkh([d34db33f/84h/0h/0h]xpub6C.../0/*)`, first of three frames.
        scanner::scene_feed(a, &["B$2U0300O5YGW2BIGZRDINDCGMZWMZRPHA2GQL3"]);
    }),
    ("wallets-add-scanner-scanned", |a| {
        scene_scanner(a);
        scanner::scene_feed(a, &["wpkh([d34db33f/84h/0h/0h]xpub6C.../0/*)"]);
    }),
    ("wallets-add-scanner-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_scanner(a);
    }),
    ("wallets-add", |a| scene_pick(a, "descriptor")),
    ("wallets-add-nodes", |a| scene_pick(a, "bitcoin-core")),
    ("wallets-add-lightning", |a| scene_pick(a, "phoenix")),
    ("wallets-add-merchant", |a| scene_pick(a, "btcpay")),
    ("wallets-add-exchanges", |a| scene_pick(a, "river")),
    ("wallets-add-files", |a| scene_pick(a, "generic-ledger")),
    ("wallets-add-planned", |a| scene_pick(a, "zeus")),
    ("wallets-add-search", |a| {
        scene_pick(a, "descriptor");
        a.ov.add_search = TextEdit::new("csv");
    }),
    ("wallets-add-search-empty", |a| {
        scene_pick(a, "descriptor");
        a.ov.add_search = TextEdit::new("zzzz");
    }),
    ("wallets-add-descriptor", |a| {
        scene_continue(a, "descriptor")
    }),
    ("wallets-add-descriptor-clean", |a| {
        scene_setup(a, "descriptor")
    }),
    ("wallets-add-descriptor-birthday-picker", |a| {
        scene_setup(a, "descriptor");
        stm(a).date_picker = Some(crate::ui::date_picker::PickerState {
            key: "connection-birthday".into(),
            year: 2024,
            month: 2,
            year_page_start: 2015,
            view: crate::ui::date_picker::View::Days,
        });
    }),
    ("wallets-add-descriptor-birthday-years", |a| {
        scene_setup(a, "descriptor");
        stm(a).date_picker = Some(crate::ui::date_picker::PickerState {
            key: "connection-birthday".into(),
            year: 2024,
            month: 2,
            year_page_start: 2015,
            view: crate::ui::date_picker::View::Years,
        });
    }),
    // "Create connection" pressed with the label empty: the validation bubble on that field.
    ("wallets-add-descriptor-required", |a| {
        scene_setup(a, "descriptor");
        submit::submit(a);
    }),
    ("wallets-add-sparrow", |a| scene_continue(a, "sparrow")),
    ("wallets-add-address-list", |a| {
        scene_continue(a, "address-list")
    }),
    ("wallets-add-silent-payment", |a| {
        scene_continue(a, "silent-payment")
    }),
    ("wallets-add-liquid", |a| {
        scene_continue(a, "liquid-descriptor")
    }),
    ("wallets-add-samourai", |a| scene_continue(a, "samourai")),
    ("wallets-add-wasabi", |a| scene_continue(a, "wasabi")),
    ("wallets-add-bitcoin-core", |a| {
        scene_continue(a, "bitcoin-core")
    }),
    ("wallets-add-electrum", |a| scene_setup(a, "electrum")),
    ("wallets-add-ledger", |a| scene_continue(a, "ledger")),
    ("wallets-add-phoenix", |a| scene_continue(a, "phoenix")),
    ("wallets-add-bull-wallet", |a| {
        scene_continue(a, "bullbitcoin-wallet")
    }),
    ("wallets-add-btcpay-csv", |a| {
        scene_continue(a, "btcpay-csv")
    }),
    ("wallets-add-river", |a| scene_continue(a, "river")),
    ("wallets-add-bisq", |a| scene_continue(a, "bisq")),
    ("wallets-add-bullbitcoin", |a| {
        scene_continue(a, "bullbitcoin")
    }),
    ("wallets-add-prior-report", |a| {
        scene_continue(a, "prior-tax-report")
    }),
    ("wallets-add-generic-ledger", |a| {
        scene_continue(a, "generic-ledger")
    }),
    ("wallets-add-csv", |a| scene_continue(a, "csv")),
    ("wallets-add-bip329", |a| scene_continue(a, "bip329")),
    ("wallets-add-descriptor-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_continue(a, "descriptor");
    }),
    ("wallets-add-descriptor-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_continue(a, "descriptor");
    }),
    ("wallets-add-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_pick(a, "descriptor");
    }),
    ("wallets-add-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_pick(a, "descriptor");
    }),
];

/// The args every book-bound mutation carries.
pub fn scoped(app: &App, mut args: Value) -> Value {
    if !args.is_object() {
        args = json!({});
    }
    args["expected_scope"] = app.expected_scope();
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_zone_comes_from_a_zoneinfo_link_or_name() {
        assert_eq!(
            zone_from_link("/usr/share/zoneinfo/Europe/Vienna"),
            "Europe/Vienna"
        );
        assert_eq!(
            zone_from_link("/var/db/timezone/zoneinfo/America/Argentina/Buenos_Aires"),
            "America/Argentina/Buenos_Aires"
        );
        assert_eq!(zone_from_link("/etc/localtime.real"), "");
        assert_eq!(zone_name("UTC"), "UTC");
        assert_eq!(zone_name("Etc/GMT+1"), "Etc/GMT+1");
        // POSIX rules and paths are not IANA names: the owner types the zone instead.
        assert_eq!(zone_name("CET-1CEST,M3.5.0,M10.5.0/3"), "");
        assert_eq!(zone_name("/usr/share/zoneinfo/Europe/Vienna"), "");
        assert_eq!(zone_name(""), "");
    }

    #[test]
    fn continue_to_setup_does_not_submit_or_raise_validation_notice() {
        for id in ["descriptor", "address-list", "electrum"] {
            let mut app = crate::app::tests::quiet_app();
            open(&mut app, None);
            let src = source(id).expect("source");
            app.ov.add_category = Some(src.category);
            app.ov.add_selected = Some(src.id);
            reset_form(&mut app, src);
            continue_to_setup(&mut app);
            assert!(app.ov.add_open, "{id}: dialog should remain open");
            assert_eq!(st(&app).step, Step::Setup, "{id}: should enter setup step");
            assert!(
                st(&app).required_hint.is_none(),
                "{id}: must not show required notice on untouched form"
            );
            assert!(
                st(&app).errors.is_empty(),
                "{id}: must not show validation errors on untouched form"
            );
            assert!(
                !submitting(&app),
                "{id}: must not start submit job on Continue"
            );
        }
    }

    #[cfg(feature = "tooling")]
    fn headless_descriptor_runtime() -> rune::Runtime<App> {
        let spec = rune::App::new(rune::AppSpec::<App> {
            fonts: crate::FONTS,
            families: (crate::sans_stack(), crate::MONO),
            setup: Some(crate::setup),
            poll: Some(crate::poll),
            on_key: Some(crate::on_key),
            ..rune::AppSpec::new(
                "Kassiber",
                (1440.0, 900.0),
                crate::app::tests::quiet_app,
                crate::view,
            )
        });
        let mut rt = spec.headless_runtime((1440.0, 900.0), rune::Env::new(true, 1.0));
        rt.state.route = crate::app::Route("/connections".into());
        open(&mut rt.state, Some("descriptor"));
        for _ in 0..4 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    #[cfg(feature = "tooling")]
    fn click_point(rt: &mut rune::Runtime<App>, x: f32, y: f32) {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
    }

    #[cfg(feature = "tooling")]
    fn el_rect(rt: &rune::Runtime<App>, key: &str) -> [f32; 4] {
        let els = rt.elements();
        els.iter()
            .find(|e| e.key.as_deref() == Some(key))
            .unwrap_or_else(|| {
                let keys: Vec<_> = els.iter().filter_map(|e| e.key.as_deref()).collect();
                panic!("missing element {key:?}; present keys: {keys:?}")
            })
            .rect
    }

    /// Clicking near the bottom of an empty Wallet export textarea focuses it, long spaceless
    /// descriptors wrap within the box without horizontal overflow, tall pastes clamp height
    /// and scroll vertically, and pressing Enter inserts a newline.
    #[cfg(feature = "tooling")]
    #[test]
    fn textarea_focuses_on_bottom_click_wraps_long_descriptors_and_clamps_tall_pastes() {
        let mut rt = headless_descriptor_runtime();
        let box_rect = el_rect(&rt, "connection-wallet-material-box");
        assert!(box_rect[3] >= 100.0, "empty textarea box should keep min_h");
        // Click near the bottom of the empty box (well below line 1).
        click_point(
            &mut rt,
            box_rect[0] + 40.0,
            box_rect[1] + box_rect[3] - 12.0,
        );
        assert!(
            st(&rt.state).form.wallet_material.focused,
            "clicking lower half of empty textarea must focus it"
        );

        // Enter inserts a newline.
        rt.key(KeyEvent {
            key: Key::Enter,
            mods: rune::Mods::default(),
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(st(&rt.state).form.wallet_material.text, "\n");

        // Long spaceless descriptor must hard-wrap inside the box width (no horizontal overflow).
        let long_desc = format!(
            "wsh(sortedmulti(2,[d34db33f/48h/0h/0h/2h]xpub6{a}/<0;1>/*,[cafebabe/48h/0h/0h/2h]xpub6{b}/<0;1>/*,[deadbeef/48h/0h/0h/2h]xpub6{c}/<0;1>/*))#checksum",
            a = "A".repeat(100),
            b = "B".repeat(100),
            c = "C".repeat(100),
        );
        stm(&mut rt.state).form.wallet_material = TextEdit::new(&long_desc);
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        let box_rect = el_rect(&rt, "connection-wallet-material-box");
        let text_rect = el_rect(&rt, "connection-wallet-material");
        assert!(
            text_rect[0] >= box_rect[0] - 1.0
                && text_rect[0] + text_rect[2] <= box_rect[0] + box_rect[2] + 1.0,
            "long descriptor must wrap inside box: box={box_rect:?} text={text_rect:?}"
        );

        // Tall 40-line paste clamps the visible box height and scrolls inside the viewport.
        let tall = (0..40)
            .map(|i| format!("bc1qexampleaddressline{i:02}xxxxxxxxxxxxxxxxxxxxxxxxxx"))
            .collect::<Vec<_>>()
            .join("\n");
        stm(&mut rt.state).form.wallet_material = TextEdit::new(&tall);
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        let box_rect = el_rect(&rt, "connection-wallet-material-box");
        let text_rect = el_rect(&rt, "connection-wallet-material");
        assert!(
            box_rect[3] <= 265.0,
            "tall paste must stay vertically clamped: box={box_rect:?}"
        );
        assert!(
            text_rect[3] > box_rect[3] + 20.0,
            "inner text height must exceed scroll viewport so it scrolls: box={box_rect:?} text={text_rect:?}"
        );
    }

    /// Selecting a wallet birthday via Year -> Month -> Day writes canonical ISO `YYYY-MM-DD`,
    /// closes the popover, and Clear resets it back to empty.
    #[cfg(feature = "tooling")]
    #[test]
    fn date_picker_selects_year_month_day_and_clears() {
        let mut rt = headless_descriptor_runtime();
        let trig = el_rect(&rt, "connection-birthday");
        click_point(&mut rt, trig[0] + 40.0, trig[1] + trig[3] / 2.0);
        assert!(st(&rt.state).date_picker.is_some(), "picker should open");

        let yr_btn = el_rect(&rt, "connection-birthday-pick-year");
        click_point(
            &mut rt,
            yr_btn[0] + yr_btn[2] / 2.0,
            yr_btn[1] + yr_btn[3] / 2.0,
        );
        assert_eq!(
            st(&rt.state).date_picker.as_ref().map(|p| p.view),
            Some(crate::ui::date_picker::View::Years)
        );

        let y2021 = el_rect(&rt, "connection-birthday-year-2021");
        click_point(
            &mut rt,
            y2021[0] + y2021[2] / 2.0,
            y2021[1] + y2021[3] / 2.0,
        );
        assert_eq!(
            st(&rt.state).date_picker.as_ref().map(|p| p.view),
            Some(crate::ui::date_picker::View::Months)
        );

        // Month 4 = May (0-indexed).
        let may = el_rect(&rt, "connection-birthday-month-4");
        click_point(&mut rt, may[0] + may[2] / 2.0, may[1] + may[3] / 2.0);
        assert_eq!(
            st(&rt.state).date_picker.as_ref().map(|p| p.view),
            Some(crate::ui::date_picker::View::Days)
        );

        let day15 = el_rect(&rt, "connection-birthday-day-15");
        click_point(
            &mut rt,
            day15[0] + day15[2] / 2.0,
            day15[1] + day15[3] / 2.0,
        );
        assert!(
            st(&rt.state).date_picker.is_none(),
            "picker should close on pick"
        );
        assert_eq!(st(&rt.state).form.birthday.text, "2021-05-15");

        let clear = el_rect(&rt, "connection-birthday-clear");
        click_point(
            &mut rt,
            clear[0] + clear[2] / 2.0,
            clear[1] + clear[3] / 2.0,
        );
        assert_eq!(st(&rt.state).form.birthday.text, "");
    }

    /// Every Add connection state touched here (setup form, date picker Days/Months/Years,
    /// live sync progress + Stop scan, required bubble, scanner) renders with zero a11y
    /// findings, zero missing translations in English and German, and visible scanner controls.
    #[cfg(feature = "tooling")]
    #[test]
    fn add_connection_states_have_no_a11y_findings_or_missing_translations() {
        for lang in [crate::i18n::Lang::En, crate::i18n::Lang::De] {
            let check = |rt: &mut rune::Runtime<App>, label: &str| {
                for _ in 0..3 {
                    rt.frame(1.0 / 60.0);
                }
                let (tree, elements) = (rt.accessibility_json(), rt.elements());
                let findings = crate::checks::a11y_findings(&tree, &elements);
                assert!(
                    findings.is_empty(),
                    "{label} ({}): a11y findings: {findings:#?}",
                    lang.code()
                );
            };
            // 1. Clean Descriptor setup form (with birthday set so Clear button renders too).
            let mut rt = headless_descriptor_runtime();
            rt.state.prefs.lang = lang;
            rt.state.reads.insert(
                "ui.backends.options".into(),
                Reply {
                    kind: "ui.backends.options".into(),
                    data: json!({
                        "backends": [
                            {"name": "esplora-btc", "kind": "esplora", "chain": "bitcoin", "is_default": true}
                        ]
                    }),
                    error: None,
                    error_detail: Value::Null,
                    elapsed: std::time::Duration::ZERO,
                },
            );
            stm(&mut rt.state).form.backend = "esplora-btc".into();
            stm(&mut rt.state).form.birthday = TextEdit::new("2021-05-15");
            // Open date picker via real click (forces render) and verify Days -> Years -> Months.
            let trig = el_rect(&rt, "connection-birthday");
            click_point(&mut rt, trig[0] + 40.0, trig[1] + trig[3] / 2.0);
            check(&mut rt, "date-picker-days");
            let yr_btn = el_rect(&rt, "connection-birthday-pick-year");
            click_point(
                &mut rt,
                yr_btn[0] + yr_btn[2] / 2.0,
                yr_btn[1] + yr_btn[3] / 2.0,
            );
            check(&mut rt, "date-picker-years");
            let y2021 = el_rect(&rt, "connection-birthday-year-2021");
            click_point(
                &mut rt,
                y2021[0] + y2021[2] / 2.0,
                y2021[1] + y2021[3] / 2.0,
            );
            check(&mut rt, "date-picker-months");
            // Close picker via Escape.
            rt.key(KeyEvent {
                key: Key::Escape,
                mods: rune::Mods::default(),
            });
            check(&mut rt, "descriptor-setup");
            // 2. Required validation bubble via real Submit click.
            let sub = el_rect(&rt, "add-submit");
            click_point(&mut rt, sub[0] + sub[2] / 2.0, sub[1] + sub[3] / 2.0);
            assert!(st(&rt.state).required_hint.is_some());
            check(&mut rt, "required-bubble");
            // 3. Live sync progress card + Stop scan button (cancelling state + terminal exit).
            let spec = rune::App::new(rune::AppSpec::<App> {
                fonts: crate::FONTS,
                families: (crate::sans_stack(), crate::MONO),
                setup: Some(crate::setup),
                poll: Some(crate::poll),
                on_key: Some(crate::on_key),
                ..rune::AppSpec::new(
                    "Kassiber",
                    (1440.0, 900.0),
                    crate::app::tests::quiet_app,
                    crate::view,
                )
            });
            let mut rt_sync = spec.headless_runtime((1440.0, 900.0), rune::Env::new(true, 1.0));
            rt_sync.state.prefs.lang = lang;
            rt_sync.state.route = crate::app::Route("/connections".into());
            open(&mut rt_sync.state, Some("descriptor"));
            stm(&mut rt_sync.state).job = Some(submit::Job {
                stage: submit::Stage::Sync,
                source_id: "descriptor",
                label: "Cold".into(),
                request_id: Some("req-sync-ui".into()),
                sync_wallet: Some("Cold".into()),
                syncs: std::collections::VecDeque::new(),
                done: None,
                stay_open: false,
                dismissed: false,
                cancelling: false,
                form: st(&rt_sync.state).form.clone(),
                replay: None,
                sent: None,
                import_after: None,
            });
            stm(&mut rt_sync.state).progress = Some(SyncProgress {
                wallet: "Cold".into(),
                job_id: Some("job-1".into()),
                phase: Some("backend_fetch".into()),
                processed: 10,
                total: 40,
                percent: 25.0,
            });
            check(&mut rt_sync, "sync-progress");
            let stop = el_rect(&rt_sync, "add-cancel-scan");
            click_point(
                &mut rt_sync,
                stop[0] + stop[2] / 2.0,
                stop[1] + stop[3] / 2.0,
            );
            assert!(
                st(&rt_sync.state)
                    .job
                    .as_ref()
                    .is_some_and(|j| j.cancelling),
                "Stop scan must transition to cancelling until the worker exits"
            );
            check(&mut rt_sync, "sync-progress-cancelling");
            rt_sync.state.deliver_as(
                "wallets.add.job",
                "req-sync-ui",
                Reply {
                    kind: "ui.wallets.sync".into(),
                    data: Value::Null,
                    error: Some("cancelled".into()),
                    error_detail: Value::Null,
                    elapsed: std::time::Duration::ZERO,
                },
            );
            assert!(st(&rt_sync.state).progress.is_none());
            // 4. QR scanner overlay opened via real Scan button click: zero a11y findings and
            // Cancel button stays visible without scrolling.
            let scan_btn = el_rect(&rt, "add-scan");
            click_point(
                &mut rt,
                scan_btn[0] + scan_btn[2] / 2.0,
                scan_btn[1] + scan_btn[3] / 2.0,
            );
            assert!(scanner::is_open(&rt.state));
            assert!(
                st(&rt.state).required_hint.is_none(),
                "opening scanner clears any lingering validation bubble"
            );
            check(&mut rt, "scanner");
            let cancel_rect = el_rect(&rt, "scanner-cancel");
            assert!(
                cancel_rect[1] + cancel_rect[3] <= 900.0,
                "scanner Cancel button must stay visible without scrolling: {cancel_rect:?}"
            );
        }
        let missing = crate::i18n::missing();
        assert!(missing.is_empty(), "missing i18n keys: {missing:?}");
    }

    #[cfg(feature = "tooling")]
    #[test]
    fn scanner_completed_multipart_keeps_cancel_and_use_visible_at_laptop_height() {
        let size = (1280.0, 768.0);
        let spec = rune::App::new(rune::AppSpec::<App> {
            fonts: crate::FONTS,
            families: (crate::sans_stack(), crate::MONO),
            setup: Some(crate::setup),
            poll: Some(crate::poll),
            on_key: Some(crate::on_key),
            ..rune::AppSpec::new("Kassiber", size, crate::app::tests::quiet_app, crate::view)
        });
        let mut rt = spec.headless_runtime(size, rune::Env::new(true, 1.0));
        rt.state.shell.window_w = size.0;
        rt.state.shell.window_h = size.1;
        rt.state.route = crate::app::Route("/connections".into());
        open(&mut rt.state, Some("descriptor"));
        scanner::open(&mut rt.state, "Scan descriptor".into());
        {
            let sc = &mut rt.state.wallets.add.scanner;
            sc.progress = Some(bbqr::Progress {
                received: 2,
                total: 2,
                file_type: "U".into(),
            });
            sc.scanned = Some("wpkh(xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz/0/*)".into());
            sc.error = Some("Camera preview paused after capture.".into());
        }
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        let (tree, elements) = (rt.accessibility_json(), rt.elements());
        let findings = crate::checks::a11y_findings(&tree, &elements);
        assert!(findings.is_empty(), "scanner a11y findings: {findings:#?}");

        let cancel_rect = el_rect(&rt, "scanner-cancel");
        let use_rect = el_rect(&rt, "scanner-use");
        assert!(
            cancel_rect[1] >= 0.0 && cancel_rect[1] + cancel_rect[3] <= size.1,
            "scanner-cancel must stay inside laptop viewport 768px: {cancel_rect:?}"
        );
        assert!(
            use_rect[1] >= 0.0 && use_rect[1] + use_rect[3] <= size.1,
            "scanner-use must stay inside laptop viewport 768px: {use_rect:?}"
        );

        // Clicking Use scanned text applies the descriptor and closes the scanner.
        click_point(
            &mut rt,
            use_rect[0] + use_rect[2] / 2.0,
            use_rect[1] + use_rect[3] / 2.0,
        );
        assert!(!scanner::is_open(&rt.state));
        assert!(
            st(&rt.state).form.wallet_material.text.starts_with("wpkh("),
            "clicking pinned scanner-use must apply scanned material"
        );
    }

    #[cfg(feature = "tooling")]
    #[test]
    fn date_picker_keyboard_week_navigation_and_focus_return_in_add_and_edit_dialogs() {
        fn focused_key(rt: &rune::Runtime<App>) -> Option<String> {
            rt.elements()
                .into_iter()
                .find(|e| e.focused)
                .map(|e| e.path.rsplit('/').next().unwrap_or(&e.path).to_owned())
        }
        fn press(rt: &mut rune::Runtime<App>, key: Key) {
            rt.key(KeyEvent {
                key,
                mods: rune::Mods::default(),
            });
            for _ in 0..3 {
                rt.frame(1.0 / 60.0);
            }
        }

        // 1. Add Connection dialog (`connection-birthday`).
        let mut rt = headless_descriptor_runtime();
        stm(&mut rt.state).form.birthday = TextEdit::new("2024-05-15");
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        let trig = el_rect(&rt, "connection-birthday");
        click_point(&mut rt, trig[0] + 40.0, trig[1] + trig[3] / 2.0);
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday-day-15"),
            "opening picker should autofocus selected day"
        );
        // Down moves a full week (+7 days), including crossing into June.
        press(&mut rt, Key::Down);
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday-day-22")
        );
        press(&mut rt, Key::Down);
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday-day-29")
        );
        press(&mut rt, Key::Down);
        assert_eq!(
            st(&rt.state)
                .date_picker
                .as_ref()
                .map(|p| (p.year, p.month)),
            Some((2024, 5)),
            "Down past month end must advance to June"
        );
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday-day-5")
        );
        // Up moves back a full week (-7 days) across the month boundary to May 29.
        press(&mut rt, Key::Up);
        assert_eq!(
            st(&rt.state)
                .date_picker
                .as_ref()
                .map(|p| (p.year, p.month)),
            Some((2024, 4))
        );
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday-day-29")
        );
        press(&mut rt, Key::Right);
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday-day-30")
        );
        // Enter picks the day, closes the popover, and returns focus to the trigger.
        press(&mut rt, Key::Enter);
        assert!(st(&rt.state).date_picker.is_none());
        assert_eq!(st(&rt.state).form.birthday.text, "2024-05-30");
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday"),
            "Enter selection must return focus to trigger"
        );
        // Reopen with Enter on the trigger, then dismiss with Escape and check focus return.
        press(&mut rt, Key::Enter);
        assert!(st(&rt.state).date_picker.is_some());
        press(&mut rt, Key::Escape);
        assert!(st(&rt.state).date_picker.is_none());
        assert_eq!(
            focused_key(&rt).as_deref(),
            Some("connection-birthday"),
            "Escape must return focus to trigger"
        );

        // 2. Edit Connection dialog (`wd-edit-birthday`).
        let spec = rune::App::new(rune::AppSpec::<App> {
            fonts: crate::FONTS,
            families: (crate::sans_stack(), crate::MONO),
            setup: Some(crate::setup),
            poll: Some(crate::poll),
            on_key: Some(crate::on_key),
            ..rune::AppSpec::new(
                "Kassiber",
                (1440.0, 900.0),
                crate::app::tests::quiet_app,
                crate::view,
            )
        });
        let mut rt_edit = spec.headless_runtime((1440.0, 900.0), rune::Env::new(true, 1.0));
        rt_edit.state.route = crate::app::Route("/connections/w1".into());
        rt_edit.state.reads.insert(
            "ui.overview.snapshot".into(),
            Reply {
                kind: "ui.overview.snapshot".into(),
                data: json!({"connections": [{"id": "w1", "label": "Cold", "kind": "descriptor"}]}),
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            },
        );
        rt_edit.state.wallets.detail.wallet = "w1".into();
        rt_edit.state.wallets.detail.wallets = crate::wallets::detail::Read::Ready(vec![json!({
            "id": "w1",
            "label": "Cold",
            "kind": "descriptor",
            "birthday": "2024-05-15"
        })]);
        rt_edit.state.wallets.detail.edit = Some(crate::wallets::detail::EditForm {
            serial: 1,
            label: TextEdit::new("Cold"),
            ack: TextEdit::new(crate::wallets::detail::CHANGE_ACK),
            birthday: TextEdit::new("2024-05-15"),
            ..Default::default()
        });
        for _ in 0..3 {
            rt_edit.frame(1.0 / 60.0);
        }
        let edit_trig = el_rect(&rt_edit, "wd-edit-birthday");
        click_point(
            &mut rt_edit,
            edit_trig[0] + 40.0,
            edit_trig[1] + edit_trig[3] / 2.0,
        );
        assert_eq!(
            focused_key(&rt_edit).as_deref(),
            Some("wd-edit-birthday-day-15")
        );
        press(&mut rt_edit, Key::Down);
        assert_eq!(
            focused_key(&rt_edit).as_deref(),
            Some("wd-edit-birthday-day-22")
        );
        press(&mut rt_edit, Key::Enter);
        assert!(rt_edit.state.wallets.detail.date_picker.is_none());
        assert_eq!(
            rt_edit
                .state
                .wallets
                .detail
                .edit
                .as_ref()
                .unwrap()
                .birthday
                .text,
            "2024-05-22"
        );
        assert_eq!(
            focused_key(&rt_edit).as_deref(),
            Some("wd-edit-birthday"),
            "Edit Connection picker must return focus to trigger on Enter"
        );
        press(&mut rt_edit, Key::Enter);
        assert!(rt_edit.state.wallets.detail.date_picker.is_some());
        press(&mut rt_edit, Key::Escape);
        assert!(rt_edit.state.wallets.detail.date_picker.is_none());
        assert_eq!(
            focused_key(&rt_edit).as_deref(),
            Some("wd-edit-birthday"),
            "Edit Connection picker must return focus to trigger on Escape"
        );

        // While Save is pending, the birthday picker trigger is disabled and clicking it does
        // not open the popover.
        rt_edit.state.wallets.detail.edit.as_mut().unwrap().pending = true;
        rt_edit.invalidate();
        for _ in 0..2 {
            rt_edit.frame(1.0 / 60.0);
        }
        let edit_trig = el_rect(&rt_edit, "wd-edit-birthday");
        click_point(
            &mut rt_edit,
            edit_trig[0] + 40.0,
            edit_trig[1] + edit_trig[3] / 2.0,
        );
        assert!(
            rt_edit.state.wallets.detail.date_picker.is_none(),
            "Edit Connection date picker must stay disabled while Save is pending"
        );
    }
}
