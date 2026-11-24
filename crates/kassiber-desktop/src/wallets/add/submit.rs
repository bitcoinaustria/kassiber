//! `validateSetupForm` and `onSetupSubmit`: the submit pipeline as a step machine over daemon
//! replies, plus the setup step's other actions (address preview, Bitcoin Core detect / test /
//! coverage check, ledger template, BIP329 export).
//!
//! Every request here runs only from a press. Those that leave the machine (sync after create,
//! Bitcoin Core probes, script-type detection, exchange sync) are the web's own, on the same
//! presses.

use std::collections::VecDeque;

use serde_json::{Value, json};

use super::libs::{address_list, samourai, wallet_material, wasabi};
use super::meta::{SetupKind, is_exchange_evidence, meta};
use super::{
    CoreTest, Coverage, PendingSave, SyncProgress, history_import, history_target,
    history_uses_existing, is_history_source, scoped, selected, st, stm,
};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};

pub const MAX_GAP: i64 = 5000;

/// The submit in flight.
pub struct Job {
    pub stage: Stage,
    /// The catalog source id submitted (`selected(app).id` at submit time).
    pub source_id: &'static str,
    pub label: String,
    /// Daemon request id of the step currently in flight (its cancel and progress name it).
    pub request_id: Option<String>,
    /// The claim on that step's answer: dropped with the job, the answer reaches nobody.
    pub ticket: Option<crate::app::Ticket>,
    /// Immutable wallet label submitted to `ui.wallets.sync` for the current `Stage::Sync`.
    pub sync_wallet: Option<String>,
    /// Wallets still to `ui.wallets.sync`.
    pub syncs: VecDeque<String>,
    /// The toast once everything is done.
    pub done: Option<(String, String, ToastTone)>,
    /// Keep the dialog open when done (an import into an existing wallet shows its summary).
    pub stay_open: bool,
    /// The user closed the dialog while this submit was still in flight.
    pub dismissed: bool,
    /// Cancellation has been requested for the active `Stage::Sync`; waiting for worker exit.
    pub cancelling: bool,
    /// The form as submitted: later steps (detection, coverage, the create after them) use
    /// it, not the form's current values, which stay editable while the steps run.
    pub form: super::Form,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    CoreTest,
    CoreCreate,
    /// `requireSelectedCoreCoverage` before the create.
    Coverage,
    DetectScripts,
    Create,
    ImportSamourai,
    ImportFile,
    WasabiImport,
    PriorReport,
    BullCreate,
    BtcpayCreate,
    ExchangeCreate,
    ExchangeSync,
    Bip329Preview,
    Bip329Import,
    Sync,
}

fn kind(app: &App) -> SetupKind {
    meta(selected(app).id).kind()
}

fn txt(e: &rune::TextEdit) -> String {
    e.text.trim().to_owned()
}

/// Bisq's Funds > Transactions export and the zone its local times were written in, beside the
/// trade history (`ui.wallets.create` and `ui.wallets.import_file` take both).
fn bisq_args(f: &super::Form, fmt: &str, args: &mut Value) {
    if fmt == "bisq_csv" {
        args["companion_file"] = json!(txt(&f.companion_file));
        args["timezone"] = json!(txt(&f.source_timezone));
    }
}

// ---- Backends ------------------------------------------------------------------------------

/// The `ui.backends.options` backends.
pub fn backends(app: &App) -> Vec<Value> {
    app.read("ui.backends.options")
        .and_then(|d| d.get("backends"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn kind_of(b: &Value) -> &str {
    b.get("kind").and_then(Value::as_str).unwrap_or("")
}
fn chain_of(b: &Value) -> Option<&str> {
    b.get("chain")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
}
pub fn name_of(b: &Value) -> &str {
    b.get("name").and_then(Value::as_str).unwrap_or("")
}

/// `backendOptionLabel` plus the select's suffixes.
pub fn backend_option_label(b: &Value) -> String {
    let name = name_of(b);
    let display = b
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    let mut s = if display == name {
        display.to_owned()
    } else {
        format!("{display} ({name})")
    };
    if b.get("is_default").and_then(Value::as_bool) == Some(true) {
        s.push_str(&t("connections:add.field.backendOptionDefault"));
    }
    let k = kind_of(b);
    if !k.is_empty() {
        s.push_str(&format!(" · {k}"));
    }
    s
}

fn descriptor_capable(b: &Value) -> bool {
    matches!(kind_of(b), "esplora" | "electrum" | "bitcoinrpc")
}

/// `descriptorBackendOptions` (liquid or bitcoin by the source's chain).
pub fn descriptor_options(app: &App) -> Vec<Value> {
    let liquid = meta(selected(app).id).chain == Some("liquid");
    backends(app)
        .into_iter()
        .filter(|b| {
            descriptor_capable(b)
                && if liquid {
                    chain_of(b) == Some("liquid")
                } else {
                    chain_of(b).is_none_or(|c| c == "bitcoin")
                }
        })
        .collect()
}

/// `addressBackendOptions`.
pub fn address_options(app: &App) -> Vec<Value> {
    if meta(selected(app).id).chain == Some("bitcoin") {
        backends(app)
            .into_iter()
            .filter(|b| descriptor_capable(b) && chain_of(b).is_none_or(|c| c == "bitcoin"))
            .collect()
    } else {
        descriptor_options(app)
    }
}

/// `silentPaymentBackendOptions`.
pub fn silent_payment_options(app: &App) -> Vec<Value> {
    let server = st(app).form.sp_scan_mode == "server_assisted";
    backends(app)
        .into_iter()
        .filter(|b| {
            b.get("silent_payments").and_then(Value::as_bool) == Some(true)
                && chain_of(b).is_none_or(|c| c == "bitcoin")
                && (!server || kind_of(b) != "electrum")
        })
        .collect()
}

/// `selectedBackendOptions`.
pub fn selected_options(app: &App) -> Vec<Value> {
    match kind(app) {
        SetupKind::AddressList => address_options(app),
        SetupKind::SilentPayment => silent_payment_options(app),
        _ => descriptor_options(app),
    }
}

pub fn selected_backend(app: &App) -> Option<Value> {
    let name = &st(app).form.backend;
    selected_options(app)
        .into_iter()
        .find(|b| name_of(b) == name)
}

fn selected_is_core(app: &App) -> bool {
    selected_backend(app).is_some_and(|b| kind_of(&b) == "bitcoinrpc")
}

/// `missingBackend`.
pub fn missing_backend(app: &App) -> bool {
    matches!(
        kind(app),
        SetupKind::Descriptor
            | SetupKind::Samourai
            | SetupKind::SilentPayment
            | SetupKind::AddressList
    ) && selected_options(app).is_empty()
}

/// The effect that fills the backend from the default when the options arrive.
pub fn apply_default_backend(app: &mut App) {
    let k = kind(app);
    if !matches!(
        k,
        SetupKind::Descriptor
            | SetupKind::Samourai
            | SetupKind::AddressList
            | SetupKind::SilentPayment
    ) {
        return;
    }
    let opts = selected_options(app);
    let Some(default) = opts
        .iter()
        .find(|b| b.get("is_default").and_then(Value::as_bool) == Some(true))
        .or(opts.first())
        .map(|b| name_of(b).to_owned())
    else {
        return;
    };
    let current = st(app).form.backend.clone();
    let keep = if k == SetupKind::SilentPayment {
        opts.iter().any(|b| name_of(b) == current)
    } else {
        !current.is_empty()
    };
    if !keep {
        stm(app).form.backend = default;
    }
}

/// `ui.wallets.list` wallets.
pub fn wallets(app: &App) -> Vec<Value> {
    app.read("ui.wallets.list")
        .and_then(|d| d.get("wallets"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `existingWalletOptions`: wallets not fed by BTCPay.
pub fn existing_wallets(app: &App) -> Vec<Value> {
    wallets(app)
        .into_iter()
        .filter(|w| w.get("sync_source").and_then(Value::as_str) != Some("btcpay"))
        .collect()
}

/// `walletForBullNetwork`.
pub fn wallet_for_bull_network(app: &App, network: &str) -> String {
    let ws = existing_wallets(app);
    let label = |w: &Value| {
        w.get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let k = |w: &Value| {
        w.get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let c = |w: &Value| {
        w.get("chain")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    if network == "lightning" {
        return ws
            .iter()
            .find(|w| ["phoenix", "lnd", "coreln", "nwc"].contains(&k(w).as_str()))
            .map(label)
            .unwrap_or_default();
    }
    ws.iter()
        .find(|w| c(w) == network && k(w) != "bullbitcoin")
        .or_else(|| ws.iter().find(|w| c(w) == network))
        .map(label)
        .unwrap_or_default()
}

/// `selectedBullWalletRoutes`.
pub fn bull_routes(app: &App) -> Vec<(&'static str, String)> {
    bull_routes_for(app, form(app))
}

fn bull_routes_for(app: &App, f: &super::Form) -> Vec<(&'static str, String)> {
    f.bull_networks
        .iter()
        .map(|n| {
            (
                *n,
                f.bull_routes
                    .get(n)
                    .filter(|w| !w.is_empty())
                    .cloned()
                    .unwrap_or_else(|| wallet_for_bull_network(app, n)),
            )
        })
        .collect()
}

// ---- Labels ----------------------------------------------------------------------------------

/// `submitLabel`.
pub fn submit_label(app: &App) -> String {
    let src = selected(app);
    let k = kind(app);
    let s = st(app);
    let f = &s.form;
    let stage = s.job.as_ref().map(|j| j.stage.clone());
    let key = if k == SetupKind::BackendSettings && src.id != "bitcoin-core" {
        "add.submit.openBackendSettings"
    } else if matches!(stage, Some(Stage::Sync | Stage::ExchangeSync)) {
        "add.submit.refreshing"
    } else if stage == Some(Stage::ImportSamourai) {
        "add.submit.importingSamourai"
    } else if matches!(stage, Some(Stage::ImportFile | Stage::WasabiImport)) {
        "add.submit.importing"
    } else if stage == Some(Stage::Bip329Preview) {
        "add.submit.previewingLabels"
    } else if stage == Some(Stage::Bip329Import) {
        "add.submit.importingLabels"
    } else if k == SetupKind::Bip329 && s.bip329_preview.is_none() {
        "add.submit.previewLabels"
    } else if stage.is_some()
        || s.core_detect_pending
        || s.core_test_pending
        || s.bip329_export_pending
        || matches!(s.coverage, Coverage::Checking)
    {
        "add.submit.saving"
    } else if k == SetupKind::Btcpay {
        "add.submit.setupBtcpayAccount"
    } else if k == SetupKind::Descriptor && f.sync_after {
        "add.submit.createAndScan"
    } else if k == SetupKind::BullbitcoinWallet && f.bull_mode == "existing_wallets" {
        "add.submit.saveWalletMapping"
    } else if k == SetupKind::BullbitcoinWallet && f.bull_networks.len() > 1 {
        "add.submit.createConnections"
    } else if k == SetupKind::FileWallet
        && (history_uses_existing(app)
            || (meta(src.id).source_format == Some("btcpay_csv")
                && f.btcpay_csv_mode == "existing_wallet"))
    {
        "add.submit.importTransactions"
    } else if k == SetupKind::FileEnrichment {
        "add.submit.importPricing"
    } else if k == SetupKind::PriorReport {
        "add.submit.archiveReport"
    } else {
        "add.submit.createConnection"
    };
    t(&format!("connections:{key}"))
}

// ---- Validation ------------------------------------------------------------------------------

fn parse_date(s: &str) -> bool {
    crate::ui::date_picker::is_valid_date(s)
}

/// `validateSetupForm`: field key → message.
pub fn validate(app: &mut App) -> Vec<(&'static str, String)> {
    let src = selected(app);
    let m = meta(src.id);
    let k = m.kind();
    let mut e: Vec<(&'static str, String)> = Vec::new();
    let f = form(app);
    let btcpay_existing =
        m.source_format == Some("btcpay_csv") && f.btcpay_csv_mode == "existing_wallet";
    let needs_label =
        matches!(
            k,
            SetupKind::Descriptor
                | SetupKind::SilentPayment
                | SetupKind::AddressList
                | SetupKind::Samourai
                | SetupKind::Btcpay
                | SetupKind::ExchangeApi
                | SetupKind::BullbitcoinWallet
                | SetupKind::PriorReport
        ) || (k == SetupKind::FileWallet && !btcpay_existing && !history_uses_existing(app))
            || (k == SetupKind::BackendSettings && src.id == "bitcoin-core");
    if needs_label && txt(&f.label).is_empty() {
        e.push(("label", t("connections:add.validation.labelRequired")));
    }
    let coverage_errors = |e: &mut Vec<(&'static str, String)>| {
        if selected_is_core(app) && form(app).sync_after {
            match &st(app).coverage {
                Coverage::Checking => {
                    e.push(("birthday", t("connections:add.core.coverageChecking")))
                }
                Coverage::BirthdayRequired(_) => e.push((
                    "birthday",
                    t("connections:add.core.birthdayRequiredForPruned"),
                )),
                Coverage::Blocked { message, .. } => e.push(("birthday", message.clone())),
                Coverage::Error { message, .. } => e.push(("backend", message.clone())),
                _ => {}
            }
        }
    };
    let gap_errors =
        |e: &mut Vec<(&'static str, String)>, gap: &str| match gap.trim().parse::<i64>() {
            Ok(g) if g > MAX_GAP => e.push((
                "gapLimit",
                tr(
                    "connections:add.validation.gapMax",
                    &[("max", &crate::format::count(MAX_GAP as usize))],
                ),
            )),
            Ok(g) if g > 0 => {}
            _ => e.push(("gapLimit", t("connections:add.validation.gapPositive"))),
        };
    match k {
        SetupKind::AddressList => {
            if address_list::parse(&f.address_list.text).valid.is_empty() {
                e.push((
                    "addressList",
                    t("connections:add.addressList.errorNeedAddress"),
                ));
            }
            if !address_options(app).is_empty() && f.backend.trim().is_empty() {
                e.push(("backend", t("connections:add.validation.chooseBackend")));
            }
            if !txt(&f.birthday).is_empty() && !parse_date(&f.birthday.text) {
                e.push(("birthday", t("connections:add.validation.birthdayInvalid")));
            }
            coverage_errors(&mut e);
        }
        SetupKind::Descriptor => {
            if txt(&f.wallet_material).is_empty() {
                e.push((
                    "walletMaterial",
                    t("connections:add.validation.pasteWalletMaterial"),
                ));
            } else {
                let d = wallet_material::detect(&f.wallet_material.text);
                if d.kind == wallet_material::Kind::Unknown {
                    e.push(("walletMaterial", d.hint.unwrap_or(d.label)));
                }
            }
            gap_errors(&mut e, &f.gap_limit.text);
            if !descriptor_options(app).is_empty() && f.backend.trim().is_empty() {
                e.push(("backend", t("connections:add.validation.chooseBackend")));
            }
            if !txt(&f.birthday).is_empty() && !parse_date(&f.birthday.text) {
                e.push(("birthday", t("connections:add.validation.birthdayInvalid")));
            }
            coverage_errors(&mut e);
        }
        SetupKind::BackendSettings if src.id == "bitcoin-core" => {
            if txt(&f.core_url).is_empty() {
                e.push(("coreRpcUrl", t("connections:add.core.errorUrl")));
            }
            if f.core_auth == "cookiefile"
                && txt(&f.core_cookiefile).is_empty()
                && f.core_credential_ref.is_empty()
            {
                e.push((
                    "coreRpcCookiefile",
                    t("connections:add.core.errorCookiefile"),
                ));
            }
            if f.core_auth == "basic" && f.core_credential_ref.is_empty() {
                if txt(&f.core_user).is_empty() {
                    e.push(("coreRpcUsername", t("connections:add.core.errorUsername")));
                }
                if txt(&f.core_pass).is_empty() {
                    e.push(("coreRpcPassword", t("connections:add.core.errorPassword")));
                }
            }
        }
        SetupKind::SilentPayment => {
            if txt(&f.sp_descriptor).is_empty() {
                e.push((
                    "spDescriptor",
                    t("connections:add.silentPayments.errorMaterial"),
                ));
            }
            if f.backend.trim().is_empty() {
                e.push(("backend", t("connections:add.validation.chooseBackend")));
            }
            let h = txt(&f.sp_start_height);
            let has_h = !h.is_empty();
            let has_d = !txt(&f.sp_start_date).is_empty();
            if has_h && h.parse::<u64>().map(|v| v.to_string() != h).unwrap_or(true) {
                e.push((
                    "spScanStartHeight",
                    t("connections:add.silentPayments.errorStartHeight"),
                ));
            }
            if !f.sp_full_history && has_d && !parse_date(&f.sp_start_date.text) {
                e.push((
                    "spScanStartDate",
                    t("connections:add.silentPayments.errorStartDate"),
                ));
            }
            if !has_h && !has_d && !f.sp_full_history {
                e.push((
                    "spScanStartHeight",
                    t("connections:add.silentPayments.errorStartPoint"),
                ));
            }
            if f.sp_full_history && !f.sp_ack_full {
                e.push((
                    "spAcknowledgeFullHistoryWarning",
                    t("connections:add.silentPayments.errorFullHistoryAck"),
                ));
            }
            if f.sp_scan_mode == "server_assisted" && !f.sp_ack_server {
                e.push((
                    "spAcknowledgeServerWarning",
                    t("connections:add.silentPayments.errorServerAck"),
                ));
            }
        }
        SetupKind::Samourai => {
            gap_errors(&mut e, &f.gap_limit.text);
            if !descriptor_options(app).is_empty() && f.backend.trim().is_empty() {
                e.push(("backend", t("connections:add.validation.chooseBackend")));
            }
            let s = &f.samourai;
            let built = samourai::build_source_set(
                samourai::Fields {
                    deposit: &s[0].text,
                    badbank: &s[1].text,
                    premix: &s[2].text,
                    postmix: &s[3].text,
                },
                m.network,
            );
            for (section, msg) in &built.errors {
                let key = match *section {
                    "deposit" => "samouraiDeposit",
                    "badbank" => "samouraiBadbank",
                    "premix" => "samouraiPremix",
                    _ => "samouraiPostmix",
                };
                e.push((key, msg.clone()));
            }
            if built.children == 0 && built.xpubs == 0 && built.errors.is_empty() {
                e.push((
                    "samouraiDeposit",
                    t("connections:add.samourai.errorNeedMaterial"),
                ));
            }
        }
        SetupKind::FileWallet => {
            if history_uses_existing(app) && history_target(app).is_none() {
                e.push((
                    "targetWallet",
                    t("connections:add.enrichment.errorChooseWallet"),
                ));
            }
            if m.source_format == Some("wasabi_bundle") && f.wasabi_mode == "rpc" {
                let w = &f.wasabi;
                let b = wasabi::build_bundle(&w[0].text, &w[1].text, &w[2].text, &w[3].text);
                for (key, v) in [
                    ("wasabiHistory", b.errors.history),
                    ("wasabiCoins", b.errors.coins),
                    ("wasabiWalletInfo", b.errors.wallet_info),
                    ("wasabiAdditional", b.errors.additional),
                ] {
                    if let Some(msg) = v {
                        e.push((key, msg));
                    }
                }
            } else if txt(&f.source_file).is_empty() {
                e.push((
                    "sourceFile",
                    t("connections:add.enrichment.errorPickExportFile"),
                ));
            } else if m.source_format == Some("generic_ledger")
                && super::ledger_preview::blocks_submit(app)
            {
                e.push((
                    "sourceFile",
                    t("connections:add.genericLedger.preview.submitBlocked"),
                ));
            }
            if m.source_format == Some("bisq_csv") {
                if txt(&f.companion_file).is_empty() {
                    e.push((
                        "companionFile",
                        t("connections:add.exportFile.bisqTransactionsError"),
                    ));
                }
                if txt(&f.source_timezone).is_empty() {
                    e.push((
                        "sourceTimezone",
                        t("connections:add.exportFile.bisqTimezoneError"),
                    ));
                }
            }
            if btcpay_existing {
                if existing_wallets(app).is_empty() {
                    e.push((
                        "targetWallet",
                        t("connections:add.btcpayCsv.errorCreateWalletFirst"),
                    ));
                } else if f.target_wallet.trim().is_empty() {
                    e.push((
                        "targetWallet",
                        t("connections:add.btcpayCsv.errorChooseWallet"),
                    ));
                }
            }
        }
        SetupKind::FileEnrichment => {
            if !is_exchange_evidence(m.source_format) && f.target_wallet.trim().is_empty() {
                e.push((
                    "targetWallet",
                    t("connections:add.enrichment.errorChooseWallet"),
                ));
            }
            if txt(&f.source_file).is_empty() {
                e.push((
                    "sourceFile",
                    t("connections:add.enrichment.errorPickExportFile"),
                ));
            }
        }
        SetupKind::PriorReport => {
            if txt(&f.report_provider).is_empty() {
                e.push((
                    "reportProvider",
                    t("connections:add.priorReport.errorProvider"),
                ));
            }
            if txt(&f.source_file).is_empty() {
                e.push(("sourceFile", t("connections:add.priorReport.errorFile")));
            }
            let y = txt(&f.tax_year);
            if !y.is_empty() && !(y.len() == 4 && y.chars().all(|c| c.is_ascii_digit())) {
                e.push(("taxYear", t("connections:add.priorReport.errorYear")));
            }
        }
        SetupKind::BullbitcoinWallet => {
            if txt(&f.source_file).is_empty() {
                e.push((
                    "sourceFile",
                    t("connections:add.bullWallet.errorPickExport"),
                ));
            }
            if f.bull_networks.is_empty() {
                e.push((
                    "sourceFile",
                    t("connections:add.bullWallet.errorSelectNetwork"),
                ));
            }
            if f.bull_mode == "existing_wallets" {
                if existing_wallets(app).is_empty() {
                    e.push((
                        "sourceFile",
                        t("connections:add.bullWallet.errorCreateWalletsFirst"),
                    ));
                } else if bull_routes(app).iter().any(|(_, w)| w.is_empty()) {
                    e.push((
                        "sourceFile",
                        t("connections:add.bullWallet.errorChooseWalletEach"),
                    ));
                }
            }
        }
        SetupKind::Btcpay => {
            let label = form(app).label.text.trim().to_owned();
            if let Err(msg) = super::btcpay::submit(app, &label) {
                e.push(("backend", msg));
            }
        }
        SetupKind::ExchangeApi => {
            let label = form(app).label.text.trim().to_owned();
            if let Err(msg) = super::exchange::submit(app, &label) {
                e.push(("backend", msg));
            }
        }
        SetupKind::Bip329 if txt(&f.bip329_file).is_empty() => {
            e.push(("bip329File", t("connections:add.bip329.errorPickFile")));
        }
        _ => {}
    }
    e
}

// ---- Submit ----------------------------------------------------------------------------------

fn dispatch_job(app: &mut App, mut job: Job, stage: Stage, kind: &str, args: Value) {
    job.stage = stage;
    let args = scoped(app, args);
    let (id, ticket) = app.send_ticket_id(kind, args, "wallets.add.job");
    job.request_id = Some(id);
    job.ticket = Some(ticket);
    if job.dismissed {
        stm(app).background_jobs.push(job);
    } else {
        stm(app).job = Some(job);
    }
}

fn send_job(app: &mut App, stage: Stage, kind: &str, args: Value) {
    if let Some(job) = stm(app).job.take() {
        dispatch_job(app, job, stage, kind, args);
    }
}

fn start(app: &mut App, label: &str) {
    let source_id = selected(app).id;
    let form = st(app).form.clone();
    stm(app).job = Some(Job {
        stage: Stage::Create,
        source_id,
        label: label.to_owned(),
        request_id: None,
        ticket: None,
        sync_wallet: None,
        syncs: VecDeque::new(),
        done: None,
        stay_open: false,
        dismissed: false,
        cancelling: false,
        form,
    });
}

/// The form a step reads: the submitted copy while a submit runs, else the live form.
fn form(app: &App) -> &super::Form {
    match &st(app).job {
        Some(j) => &j.form,
        None => &st(app).form,
    }
}

/// `onSetupSubmit`.
pub fn submit(app: &mut App) {
    if super::submitting(app) {
        return;
    }
    let src = selected(app);
    let m = meta(src.id);
    let k = m.kind();
    if history_import(app) && !is_history_source(src) {
        stm(app).setup_error = Some(t("connections:add.enrichment.errorNoFormat"));
        return;
    }
    if k == SetupKind::Planned {
        stm(app).setup_error = Some(tr(
            "connections:add.planned.body",
            &[("title", &src.title())],
        ));
        return;
    }
    // The browser's own constraint validation runs first: an empty `required` field stops the
    // submit before `onSetupSubmit` sees it.
    if let Some((key, is_select)) = first_required_missing(app) {
        report_validity(app, key, is_select);
        return;
    }
    stm(app).required_hint = None;
    {
        let s = stm(app);
        s.setup_error = None;
        s.last_import = None;
    }
    let errors = validate(app);
    stm(app).errors = errors.iter().cloned().collect();
    if !errors.is_empty() {
        return;
    }
    let f = form(app);
    let label = txt(&f.label);
    match k {
        SetupKind::BackendSettings if src.id == "bitcoin-core" => {
            start(app, &label);
            let args = core_probe_args(app);
            send_job(app, Stage::CoreTest, "ui.backends.bitcoinrpc.test", args);
        }
        SetupKind::BackendSettings => open_backend_settings(app),
        SetupKind::Descriptor | SetupKind::AddressList => {
            start(app, &label);
            if form(app).sync_after && selected_is_core(app) {
                let mut args = json!({"backend": form(app).backend});
                if !txt(&form(app).birthday).is_empty() {
                    args["birthday"] = json!(iso_date(&form(app).birthday.text));
                }
                send_job(app, Stage::Coverage, "ui.backends.bitcoinrpc.test", args);
            } else if let Some(job) = stm(app).job.take() {
                after_coverage(app, job);
            }
        }
        SetupKind::SilentPayment => {
            start(app, &label);
            let f = form(app);
            let full = f.sp_full_history;
            let h = txt(&f.sp_start_height).parse::<u64>().ok();
            let args = json!({
                "label": label,
                "kind": "silent-payment",
                "backend": opt(&f.backend),
                "chain": "bitcoin",
                "network": m.network,
                "sp_descriptor": txt(&f.sp_descriptor),
                "sp_scan_mode": f.sp_scan_mode,
                "sp_scan_start_height": if full { None } else { h },
                "sp_scan_start_date": if full { None } else { opt(&iso_date(&f.sp_start_date.text)) },
                "sp_full_history": full,
                "sp_acknowledge_full_history_warning": f.sp_ack_full,
                "sp_acknowledge_server_warning": f.sp_ack_server,
            });
            if let Some(j) = stm(app).job.as_mut() {
                // The scan always runs right after a Silent Payments wallet is saved.
                j.syncs.push_back(label.clone());
                j.done = Some((
                    t("connections:add.added.title"),
                    tr(
                        "connections:add.silentPayments.addedBodyScanning",
                        &[("label", &label)],
                    ),
                    ToastTone::Success,
                ));
            }
            send_job(app, Stage::Create, "ui.wallets.create", strip_nulls(args));
        }
        SetupKind::Samourai => {
            start(app, &label);
            let f = form(app);
            let s = &f.samourai;
            let built = samourai::build_source_set(
                samourai::Fields {
                    deposit: &s[0].text,
                    badbank: &s[1].text,
                    premix: &s[2].text,
                    postmix: &s[3].text,
                },
                m.network,
            );
            let args = json!({
                "label": label,
                "backend": opt(&f.backend),
                "network": m.network,
                "gap_limit": txt(&f.gap_limit).parse::<i64>().ok(),
                "source_set": built.source_set,
            });
            send_job(
                app,
                Stage::ImportSamourai,
                "ui.wallets.import_samourai",
                strip_nulls(args),
            );
        }
        SetupKind::FileWallet => submit_file_wallet(app, &label),
        SetupKind::FileEnrichment => {
            start(app, &label);
            let f = form(app);
            let fmt = m.source_format.unwrap_or("");
            let book = is_exchange_evidence(m.source_format);
            let mut args = json!({"source_file": txt(&f.source_file), "source_format": fmt});
            if book {
                args["mode"] = json!(f.bull_import_mode);
            } else {
                args["wallet"] = json!(f.target_wallet);
            }
            if let Some(j) = stm(app).job.as_mut() {
                j.stay_open = false;
            }
            send_job(app, Stage::ImportFile, "ui.wallets.import_file", args);
        }
        SetupKind::PriorReport => {
            start(app, &label);
            let f = form(app);
            let args = json!({
                "provider": txt(&f.report_provider),
                "file_path": txt(&f.source_file),
                "tax_year": opt(&txt(&f.tax_year)),
                "label": if label == src.title() { None } else { Some(label.clone()) },
            });
            send_job(
                app,
                Stage::PriorReport,
                "ui.documents.import_report",
                strip_nulls(args),
            );
        }
        SetupKind::BullbitcoinWallet => {
            start(app, &label);
            let f = form(app);
            let args = if f.bull_mode == "existing_wallets" {
                let routes: Vec<Value> = bull_routes(app)
                    .into_iter()
                    .map(|(n, w)| json!({"wallet": w, "network": n}))
                    .collect();
                json!({"mode": "existing_wallets", "label": label, "source_file": txt(&f.source_file), "routes": routes})
            } else {
                json!({"mode": "wallet_sources", "label": label, "source_file": txt(&f.source_file), "networks": f.bull_networks})
            };
            send_job(
                app,
                Stage::BullCreate,
                "ui.connections.bullbitcoin_wallet.create",
                args,
            );
        }
        SetupKind::Btcpay => {
            if let Ok(args) = super::btcpay::submit(app, &label) {
                start(app, &label);
                send_job(
                    app,
                    Stage::BtcpayCreate,
                    "ui.connections.btcpay.create",
                    args,
                );
            }
        }
        SetupKind::ExchangeApi => {
            if let Ok(args) = super::exchange::submit(app, &label) {
                start(app, &label);
                send_job(
                    app,
                    Stage::ExchangeCreate,
                    "ui.connections.exchange.create",
                    args,
                );
            }
        }
        SetupKind::Bip329 => {
            start(app, &label);
            let file = txt(&form(app).bip329_file);
            if st(app).bip329_preview.is_none() {
                send_job(
                    app,
                    Stage::Bip329Preview,
                    "ui.metadata.bip329.preview",
                    json!({"file": file}),
                );
            } else {
                send_job(
                    app,
                    Stage::Bip329Import,
                    "ui.metadata.bip329.import",
                    json!({"file": file}),
                );
            }
        }
        SetupKind::Planned => {}
    }
}

/// The first empty field the web marks `required` (and not read-only or disabled), in the
/// form's order, as the browser's constraint validation finds it: (control key, is a select).
pub fn first_required_missing(app: &App) -> Option<(&'static str, bool)> {
    let src = selected(app);
    let m = meta(src.id);
    let f = &st(app).form;
    let empty = |e: &rune::TextEdit| e.text.is_empty();
    let backend_required = |opts: Vec<Value>| !opts.is_empty() && f.backend.is_empty();
    let label_shown = matches!(
        m.kind(),
        SetupKind::Descriptor
            | SetupKind::AddressList
            | SetupKind::SilentPayment
            | SetupKind::Samourai
            | SetupKind::PriorReport
            | SetupKind::BullbitcoinWallet
            | SetupKind::Btcpay
            | SetupKind::ExchangeApi
    ) || (m.kind() == SetupKind::FileWallet
        && !(m.source_format == Some("btcpay_csv") && f.btcpay_csv_mode == "existing_wallet")
        && !history_uses_existing(app))
        || (m.kind() == SetupKind::BackendSettings && src.id == "bitcoin-core");
    if label_shown && empty(&f.label) {
        return Some(("connection-label", false));
    }
    let text = |missing: bool, key: &'static str| missing.then_some((key, false));
    let select = |missing: bool, key: &'static str| missing.then_some((key, true));
    // The source-file input is `required` except for the generic ledger (read-only there).
    let file_required = m.source_format != Some("generic_ledger");
    match m.kind() {
        SetupKind::Descriptor => select(
            backend_required(descriptor_options(app)),
            "connection-backend",
        )
        .or_else(|| text(empty(&f.wallet_material), "connection-wallet-material")),
        SetupKind::AddressList => {
            select(backend_required(address_options(app)), "connection-backend")
        }
        SetupKind::Samourai => select(
            backend_required(descriptor_options(app)),
            "connection-backend",
        ),
        SetupKind::SilentPayment => select(
            backend_required(silent_payment_options(app)),
            "connection-backend",
        )
        .or_else(|| text(empty(&f.sp_descriptor), "connection-sp-descriptor")),
        SetupKind::FileWallet => {
            if m.source_format == Some("wasabi_bundle") && f.wasabi_mode == "rpc" {
                text(empty(&f.wasabi[0]), "connection-wasabi-history")
            } else if m.source_format == Some("btcpay_csv")
                && f.btcpay_csv_mode == "existing_wallet"
            {
                select(
                    !existing_wallets(app).is_empty() && f.target_wallet.is_empty(),
                    "connection-btcpay-csv-target-wallet",
                )
                .or_else(|| text(empty(&f.source_file), "connection-source-file"))
            } else {
                let bisq = m.source_format == Some("bisq_csv");
                text(
                    file_required && empty(&f.source_file),
                    "connection-source-file",
                )
                .or_else(|| {
                    text(
                        bisq && empty(&f.companion_file),
                        "connection-companion-file",
                    )
                })
                .or_else(|| {
                    text(
                        bisq && empty(&f.source_timezone),
                        "connection-source-timezone",
                    )
                })
            }
        }
        SetupKind::FileEnrichment => select(
            !is_exchange_evidence(m.source_format) && f.target_wallet.is_empty(),
            "connection-target-wallet",
        )
        .or_else(|| text(empty(&f.source_file), "connection-source-file")),
        SetupKind::PriorReport => text(empty(&f.report_provider), "connection-report-provider")
            .or_else(|| text(empty(&f.source_file), "connection-source-file")),
        SetupKind::BullbitcoinWallet => text(empty(&f.source_file), "connection-source-file"),
        SetupKind::Bip329 => text(empty(&f.bip329_file), "connection-bip329-file"),
        _ => None,
    }
}

/// The browser's validation bubble for an empty `required` control (WebKit's wording, as the
/// desktop webview shows it): the control takes focus and the bubble names what is missing.
fn report_validity(app: &mut App, key: &'static str, is_select: bool) {
    let de = app.prefs.lang == crate::i18n::Lang::De;
    let message = match (is_select, de) {
        (false, false) => "Fill out this field",
        (false, true) => "Füllen Sie dieses Feld aus.",
        (true, false) => "Select an item in the list",
        (true, true) => "Wählen Sie ein Objekt in der Liste aus.",
    };
    let until = app.clock + super::REQUIRED_HINT_SECS;
    let s = stm(app);
    s.required_hint = Some(super::RequiredHint {
        key,
        message,
        until,
    });
    s.focus_required = Some(key);
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// Removes `null` members (the web's `undefined` args are not sent).
fn strip_nulls(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.retain(|_, x| !x.is_null());
    }
    v
}

/// A date field's value as `YYYY-MM-DD` (`YYYY-MM-DD`, `mm/dd/yyyy`, or `dd.mm.yyyy` is
/// converted with strict calendar validation).
pub fn iso_date(s: &str) -> String {
    crate::ui::date_picker::normalize_date(s).unwrap_or_else(|| s.trim().to_owned())
}

fn after_coverage(app: &mut App, mut job: Job) {
    let m = meta(job.source_id);
    if m.kind() == SetupKind::Descriptor {
        let bare = wallet_material::detect(&job.form.wallet_material.text).kind
            == wallet_material::Kind::BareXpub;
        if bare && job.form.script_type.is_empty() {
            let args = json!({
                "wallet_material": txt(&job.form.wallet_material),
                "backend": opt(&job.form.backend),
                "chain": m.chain,
                "network": m.network,
            });
            dispatch_job(
                app,
                job,
                Stage::DetectScripts,
                "ui.wallets.detect_script_types",
                strip_nulls(args),
            );
            return;
        }
        let types = if bare {
            Some(vec![job.form.script_type.clone()])
        } else {
            None
        };
        create_descriptor(app, job, types);
    } else {
        let parsed = address_list::parse(&job.form.address_list.text);
        let label = job.label.clone();
        let args = json!({
            "label": label,
            "kind": m.wallet_kind.unwrap_or("address"),
            "backend": opt(&job.form.backend),
            "chain": m.chain,
            "network": m.network,
            "addresses": parsed.valid,
            "birthday": opt(&iso_date(&job.form.birthday.text)),
        });
        if job.form.sync_after {
            job.syncs.push_back(label.clone());
        }
        job.done = Some((
            t("connections:add.added.title"),
            tc(
                "connections:add.addressList.addedBody",
                parsed.valid.len(),
                &[("label", &label)],
            ),
            ToastTone::Success,
        ));
        dispatch_job(
            app,
            job,
            Stage::Create,
            "ui.wallets.create",
            strip_nulls(args),
        );
    }
}

fn create_descriptor(app: &mut App, mut job: Job, script_types: Option<Vec<String>>) {
    let m = meta(job.source_id);
    let label = job.label.clone();
    let args = json!({
        "label": label,
        "kind": m.wallet_kind.unwrap_or("descriptor"),
        "backend": opt(&job.form.backend),
        "chain": m.chain,
        "network": m.network,
        "wallet_material": txt(&job.form.wallet_material),
        "script_types": script_types,
        "gap_limit": txt(&job.form.gap_limit).parse::<i64>().ok(),
        "birthday": opt(&iso_date(&job.form.birthday.text)),
    });
    if job.form.sync_after {
        job.syncs.push_back(label.clone());
    }
    job.done = Some((
        t("connections:add.added.title"),
        tr("connections:add.added.body", &[("label", &label)]),
        ToastTone::Success,
    ));
    dispatch_job(
        app,
        job,
        Stage::Create,
        "ui.wallets.create",
        strip_nulls(args),
    );
}

fn submit_file_wallet(app: &mut App, label: &str) {
    let src = selected(app);
    let m = meta(src.id);
    let f = form(app);
    let fmt = if src.id == "csv" {
        f.source_format
    } else {
        m.source_format.unwrap_or("")
    };
    if fmt.is_empty() {
        stm(app).setup_error = Some(t("connections:add.enrichment.errorNoFormat"));
        return;
    }
    start(app, label);
    let f = form(app);
    if history_uses_existing(app) {
        // Review's history import into the wallet it named: rows only, no new wallet.
        let Some(target) = history_target(app) else {
            return fail(app, t("connections:add.enrichment.errorChooseWallet"));
        };
        let id = target.get("id").cloned().unwrap_or(Value::Null);
        let mut args = json!({"wallet": id, "source_format": fmt});
        let w = &f.wasabi;
        let bundle = if m.source_format == Some("wasabi_bundle") && f.wasabi_mode == "rpc" {
            wasabi::build_bundle(&w[0].text, &w[1].text, &w[2].text, &w[3].text).bundle
        } else {
            None
        };
        match bundle {
            Some(b) => args["source_bundle"] = b,
            None => args["source_file"] = json!(txt(&f.source_file)),
        }
        bisq_args(f, fmt, &mut args);
        send_job(app, Stage::ImportFile, "ui.wallets.import_file", args);
        return;
    }
    if m.source_format == Some("btcpay_csv")
        && f.btcpay_csv_mode == "existing_wallet"
        && !history_import(app)
    {
        let target = f.target_wallet.trim().to_owned();
        let args =
            json!({"wallet": target, "source_file": txt(&f.source_file), "source_format": fmt});
        if let Some(j) = stm(app).job.as_mut() {
            j.done = Some((
                t("connections:add.btcpayCsv.importedTitle"),
                tr(
                    "connections:add.btcpayCsv.importedBody",
                    &[("wallet", &target)],
                ),
                ToastTone::Success,
            ));
        }
        send_job(app, Stage::ImportFile, "ui.wallets.import_file", args);
        return;
    }
    let rpc = m.source_format == Some("wasabi_bundle") && f.wasabi_mode == "rpc";
    let mut args =
        json!({"label": label, "kind": m.wallet_kind.unwrap_or("custom"), "source_format": fmt});
    if !rpc {
        args["source_file"] = json!(txt(&f.source_file));
    }
    bisq_args(f, fmt, &mut args);
    let sync = f.sync_after && !rpc;
    if let Some(j) = stm(app).job.as_mut() {
        if sync {
            j.syncs.push_back(label.to_owned());
        }
        let body = if f_sync(sync) {
            tr("connections:add.added.bodyImported", &[("label", label)])
        } else {
            tr("connections:add.added.body", &[("label", label)])
        };
        j.done = Some((t("connections:add.added.title"), body, ToastTone::Success));
    }
    send_job(app, Stage::Create, "ui.wallets.create", args);
}

fn f_sync(b: bool) -> bool {
    b
}

/// `coreRpcConfig` / `coreRpcProbeArgs`.
fn core_config_for(f: &super::Form) -> Value {
    let mut c = json!({"display_name": txt(&f.label)});
    if f.core_auth == "cookiefile" {
        c["cookiefile"] = json!(txt(&f.core_cookiefile));
    } else {
        c["username"] = json!(txt(&f.core_user));
        c["password"] = json!(f.core_pass.text);
    }
    c
}

pub fn core_probe_args(app: &App) -> Value {
    let f = &st(app).form;
    if !f.core_credential_ref.is_empty() {
        json!({"credential_ref": f.core_credential_ref, "timeout": 10})
    } else {
        json!({"url": txt(&f.core_url), "network": if f.core_network.is_empty() { "main" } else { f.core_network.as_str() }, "config": core_config_for(f), "timeout": 10})
    }
}

/// `backendNameFromLabel`.
fn backend_name_from_label(label: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in label.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    let s = out.trim_matches('-').to_owned();
    if s.is_empty() {
        "bitcoin-core".into()
    } else {
        s
    }
}

fn core_backend_args_for(f: &super::Form) -> Value {
    let mut a = json!({
        "name": backend_name_from_label(&f.label.text),
        "kind": "bitcoinrpc",
        "url": txt(&f.core_url),
        "chain": "bitcoin",
        "network": if f.core_network.is_empty() { "main" } else { f.core_network.as_str() },
        "config": core_config_for(f),
    });
    if !f.core_credential_ref.is_empty() {
        a["credential_ref"] = json!(f.core_credential_ref);
    }
    a
}

/// `openBackendSettings`: remember the setup, close, open Settings → Backends.
pub fn open_backend_settings(app: &mut App) {
    let id = selected(app).id;
    stm(app).deferred = Some(id);
    app.ov.add_open = false;
    app.navigate("/settings/backends");
}

/// `openConfiguredBackendSettings`.
pub fn open_configured_backend(app: &mut App, settings_hash: Option<String>) {
    app.ov.add_open = false;
    let hash = settings_hash.unwrap_or_else(|| "bitcoin".into());
    app.navigate(&format!("/settings/{hash}"));
}

fn fail(app: &mut App, message: String) {
    let job = stm(app).job.take();
    match job {
        Some(j) => fail_job(app, j, message),
        None => {
            let s = stm(app);
            s.progress = None;
            s.setup_error = Some(message);
            app.refresh_reads();
            app.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
        }
    }
}

fn fail_job(app: &mut App, job: Job, message: String) {
    if job.dismissed {
        let body = if job.label.trim().is_empty() {
            message
        } else {
            tr(
                "connections:add.dismissed.failedBody",
                &[("label", &job.label), ("error", &message)],
            )
        };
        app.toast(
            ToastTone::Error,
            &t("connections:add.dismissed.failedTitle"),
            Some(body),
            Some("connection-setup"),
        );
    } else {
        let s = stm(app);
        s.progress = None;
        s.setup_error = Some(message);
    }
    app.refresh_reads();
    app.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
}

fn finish_job(app: &mut App, job: Job) {
    if !job.dismissed {
        stm(app).progress = None;
    }
    if let Some((title, body, tone)) = job.done {
        app.toast(tone, &title, Some(body), Some("connection-setup"));
    }
    app.refresh_reads();
    app.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
    if !job.dismissed {
        // A Bisq import that held trades keeps its summary open: those trades booked nothing, and
        // the toast alone would read as success.
        let held = st(app)
            .last_import
            .as_ref()
            .is_some_and(|r| !super::bisq::held_lines(r).is_empty());
        if !job.stay_open && !held {
            app.ov.add_open = false;
        }
    }
}

fn send_freshness_cancel(
    app: &mut App,
    target_request_id: Option<&str>,
    job_id: Option<&str>,
) -> bool {
    let mut args = serde_json::Map::new();
    if let Some(req_id) = target_request_id.map(str::trim).filter(|s| !s.is_empty()) {
        args.insert("target_request_id".into(), json!(req_id));
    }
    if let Some(jid) = job_id.map(str::trim).filter(|s| !s.is_empty()) {
        args.insert("job_id".into(), json!(jid));
    }
    if args.is_empty() {
        return false;
    }
    let args = scoped(app, Value::Object(args));
    app.send("ui.freshness.cancel", args, "wallets.add.cancel");
    true
}

/// Requests cancellation of an active scan during `Stage::Sync` and keeps tracking the job until
/// the daemon's freshness worker exits and replies (so reads refresh only once the slot is free).
pub fn stop_scan(app: &mut App) {
    let Some((stage, req_id, cancelling)) = st(app)
        .job
        .as_ref()
        .map(|j| (j.stage.clone(), j.request_id.clone(), j.cancelling))
    else {
        stm(app).progress = None;
        return;
    };
    if stage != Stage::Sync || cancelling {
        return;
    }
    let job_id = st(app).progress.as_ref().and_then(|p| p.job_id.clone());
    send_freshness_cancel(app, req_id.as_deref(), job_id.as_deref());
    if let Some(j) = stm(app).job.as_mut() {
        j.cancelling = true;
        j.syncs.clear();
    }
}

/// Dismisses the dialog mid-submit without leaving half-configured connections behind:
/// required multi-step writes continue in `background_jobs`, optional post-create scans are
/// skipped with a summary toast, and an active `Stage::Sync` is cancelled and tracked through
/// worker exit before reads refresh.
pub fn dismiss_job(app: &mut App) {
    let Some(mut job) = stm(app).job.take() else {
        stm(app).progress = None;
        return;
    };
    let prog_job_id = stm(app).progress.take().and_then(|p| p.job_id);
    if job.stage == Stage::Bip329Preview {
        return;
    }
    job.dismissed = true;
    if job.stage == Stage::Sync {
        job.syncs.clear();
        if !job.cancelling {
            send_freshness_cancel(app, job.request_id.as_deref(), prog_job_id.as_deref());
            job.cancelling = true;
        }
    }
    stm(app).background_jobs.push(job);
}

/// Live `ui.wallets.sync.progress` updates while Add connection is running `Stage::Sync`.
pub fn on_progress(app: &mut App, tag: &str, request_id: &str, env: &Value) {
    if tag != "wallets.add.job" {
        return;
    }
    let Some((expected_id, expected_wallet)) = st(app).job.as_ref().and_then(|j| {
        (j.stage == Stage::Sync).then(|| {
            (
                j.request_id.clone(),
                j.sync_wallet.clone().unwrap_or_else(|| j.label.clone()),
            )
        })
    }) else {
        return;
    };
    if let Some(exp) = expected_id.as_deref()
        && !request_id.is_empty()
        && exp != request_id
    {
        return;
    }
    let data = &env["data"];
    let p = if data.get("phase").and_then(Value::as_str).is_none()
        && data.get("progress").is_some_and(Value::is_object)
    {
        &data["progress"]
    } else {
        data
    };
    let prev = st(app)
        .progress
        .as_ref()
        .map(|pr| pr.percent)
        .unwrap_or(crate::shell::maint::STARTING_VALUE);
    let percent = crate::shell::maint::progress_value(p, prev);
    let slot = stm(app).progress.get_or_insert_with(|| SyncProgress {
        wallet: expected_wallet.clone(),
        ..SyncProgress::default()
    });
    if slot.wallet.is_empty() {
        slot.wallet = expected_wallet;
    }
    if let Some(id) = p
        .get("job_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        slot.job_id = Some(id.to_owned());
    }
    if let Some(ph) = p
        .get("phase")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        slot.phase = Some(ph.to_owned());
    }
    if let Some(done) = p.get("processed").and_then(Value::as_u64) {
        slot.processed = done;
    }
    if let Some(total) = p.get("total").and_then(Value::as_u64) {
        slot.total = total;
    }
    slot.percent = percent;
}

fn next_sync_job(app: &mut App, mut job: Job) {
    if job.dismissed && !job.syncs.is_empty() {
        job.syncs.clear();
        job.done = Some((
            t("connections:add.dismissed.savedScanSkippedTitle"),
            tr(
                "connections:add.dismissed.savedScanSkippedBody",
                &[("label", &job.label)],
            ),
            ToastTone::Info,
        ));
        return finish_job(app, job);
    }
    match job.syncs.pop_front() {
        Some(w) => {
            job.sync_wallet = Some(w.clone());
            if !job.dismissed {
                stm(app).progress = Some(SyncProgress {
                    wallet: w.clone(),
                    percent: crate::shell::maint::STARTING_VALUE,
                    ..SyncProgress::default()
                });
            }
            dispatch_job(
                app,
                job,
                Stage::Sync,
                "ui.wallets.sync",
                json!({"wallet": w}),
            );
        }
        None => finish_job(app, job),
    }
}

/// `importResultFromSyncResult`: an import-shaped sync result becomes the summary.
fn import_result_from_sync(result: &Value) -> Option<Value> {
    let has = ["imported", "skipped", "updated"]
        .iter()
        .any(|k| result.get(k).is_some_and(Value::is_number))
        || result.get("inserted_records").is_some_and(Value::is_array)
        || result.get("updated_records").is_some_and(Value::is_array);
    has.then(|| result.clone())
}

fn sync_failure_detail(results: &[Value]) -> Option<String> {
    let bad = results
        .iter()
        .find(|x| x.get("status").and_then(Value::as_str) != Some("synced"))?;
    let pick = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    pick(bad.get("message"))
        .or_else(|| pick(bad.pointer("/error/message")))
        .or_else(|| pick(bad.get("reason")))
        .or_else(|| pick(bad.get("error")))
        .or_else(|| pick(bad.get("status")))
}

/// Whether `job`'s step is the one answering now: by its ticket, or for an answer or a job
/// without one, any.
fn answers_job(app: &App, job: &Job) -> bool {
    match &job.ticket {
        Some(ticket) if app.delivering_ticketed() => ticket.answers(app),
        _ => true,
    }
}

fn take_matching_job(app: &mut App) -> Option<Job> {
    if st(app).job.as_ref().is_some_and(|j| answers_job(app, j)) {
        return stm(app).job.take();
    }
    let bg = &st(app).background_jobs;
    let idx = if app.delivering_ticketed() {
        bg.iter()
            .position(|j| j.ticket.as_ref().is_some_and(|ticket| ticket.answers(app)))
    } else if bg.len() == 1 {
        Some(0)
    } else {
        None
    };
    idx.map(|i| stm(app).background_jobs.remove(i))
}

fn on_job(app: &mut App, r: &Reply) {
    let Some(mut job) = take_matching_job(app) else {
        return;
    };
    if job.stage == Stage::Sync && job.cancelling {
        job.done = Some((
            t("connections:add.dismissed.scanStoppedTitle"),
            tr(
                "connections:add.dismissed.savedScanStopped",
                &[("label", &job.label)],
            ),
            ToastTone::Info,
        ));
        job.stay_open = false;
        finish_job(app, job);
        return;
    }
    if let Some(e) = &r.error {
        fail_job(app, job, e.clone());
        return;
    }
    let d = &r.data;
    let stage = job.stage.clone();
    match stage {
        Stage::CoreTest => {
            if d.get("reachable").and_then(Value::as_bool) != Some(true) {
                return fail_job(
                    app,
                    job,
                    d.pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| t("connections:add.core.testFailed")),
                );
            }
            if d.pointer("/wallet_rpc/available").and_then(Value::as_bool) == Some(false) {
                let msg = d
                    .pointer("/wallet_rpc/error/hint")
                    .or(d.pointer("/wallet_rpc/error/message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| t("connections:add.core.walletRpcUnavailable"));
                return fail_job(app, job, msg);
            }
            job.done = Some((
                t("connections:add.added.title"),
                tr("connections:add.core.addedBody", &[("label", &job.label)]),
                ToastTone::Success,
            ));
            let args = core_backend_args_for(&job.form);
            dispatch_job(app, job, Stage::CoreCreate, "ui.backends.create", args);
        }
        Stage::CoreCreate => finish_job(app, job),
        Stage::Coverage => {
            if d.get("reachable").and_then(Value::as_bool) != Some(true) {
                return fail_job(
                    app,
                    job,
                    d.pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| t("connections:add.core.coverageProbeFailed")),
                );
            }
            if d.get("pruned").and_then(Value::as_bool) == Some(true)
                && txt(&job.form.birthday).is_empty()
            {
                return fail_job(
                    app,
                    job,
                    t("connections:add.core.birthdayRequiredForPruned"),
                );
            }
            after_coverage(app, job);
        }
        Stage::DetectScripts => match wallet_material::script_types_from_detection_payload(Some(d))
        {
            Ok(types) => create_descriptor(app, job, Some(types)),
            Err(reason) => {
                let reason = reason.unwrap_or_else(|| {
                    t("connections:add.descriptor.autoDetectUnavailableReasonUnknown")
                });
                fail_job(
                    app,
                    job,
                    tr(
                        "connections:add.descriptor.autoDetectUnavailable",
                        &[("reason", &reason)],
                    ),
                );
            }
        },
        Stage::Create => {
            let rpc = meta(job.source_id).source_format == Some("wasabi_bundle")
                && job.form.wasabi_mode == "rpc";
            if rpc {
                let w = &job.form.wasabi;
                let b = wasabi::build_bundle(&w[0].text, &w[1].text, &w[2].text, &w[3].text);
                let label = job.label.clone();
                dispatch_job(
                    app,
                    job,
                    Stage::WasabiImport,
                    "ui.wallets.import_file",
                    json!({"wallet": label, "source_format": "wasabi_bundle", "source_bundle": b.bundle}),
                );
            } else {
                next_sync_job(app, job);
            }
        }
        Stage::WasabiImport | Stage::ImportFile => {
            if !job.dismissed {
                stm(app).last_import = Some(d.clone());
            }
            let m = meta(job.source_id);
            if m.kind() == SetupKind::FileEnrichment {
                let book = is_exchange_evidence(m.source_format);
                let scope = if book {
                    t("connections:add.enrichment.bookScope")
                } else {
                    job.form.target_wallet.clone()
                };
                let n = |k: &str| {
                    crate::format::count(d.get(k).and_then(Value::as_u64).unwrap_or(0) as usize)
                };
                let body = tr(
                    "connections:add.enrichment.finishedBody",
                    &[
                        ("scope", &scope),
                        ("updated", &n("updated")),
                        ("skipped", &n("skipped")),
                    ],
                );
                job.done = Some((
                    t("connections:add.enrichment.finishedTitle"),
                    body,
                    ToastTone::Success,
                ));
            } else if job.dismissed && !super::bisq::held_lines(d).is_empty() {
                job.done = Some((
                    t("connections:add.dismissed.bisqHeldTitle"),
                    tr(
                        "connections:add.dismissed.bisqHeldBody",
                        &[("label", &job.label)],
                    ),
                    ToastTone::Warning,
                ));
            }
            finish_job(app, job);
        }
        Stage::ImportSamourai => {
            let children: Vec<String> = d
                .get("children")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.get("label").and_then(Value::as_str).map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let sync = job.form.sync_after;
            let value = crate::format::count(children.len());
            let body = if sync {
                tr(
                    "connections:add.samourai.importedBodyScanning",
                    &[("label", &job.label), ("value", &value)],
                )
            } else {
                tr(
                    "connections:add.samourai.importedBody",
                    &[("label", &job.label), ("value", &value)],
                )
            };
            if sync {
                job.syncs.extend(children);
            }
            job.done = Some((
                t("connections:add.samourai.importedTitle"),
                body,
                ToastTone::Success,
            ));
            if !job.dismissed {
                for e in stm(app).form.samourai.iter_mut() {
                    *e = rune::TextEdit::new("");
                }
            }
            next_sync_job(app, job);
        }
        Stage::PriorReport => {
            let label = d
                .pointer("/document/label")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| job.label.clone());
            job.done = Some((
                t("connections:add.priorReport.archivedTitle"),
                tr(
                    "connections:add.priorReport.archivedBody",
                    &[("label", &label)],
                ),
                ToastTone::Success,
            ));
            finish_job(app, job);
        }
        Stage::BullCreate => {
            let existing = job.form.bull_mode == "existing_wallets";
            let labels: Vec<String> = if existing {
                let mut v: Vec<String> = Vec::new();
                for (_, w) in bull_routes_for(app, &job.form) {
                    if !v.contains(&w) {
                        v.push(w);
                    }
                }
                v
            } else {
                d.get("wallets")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|w| {
                                w.get("label").and_then(Value::as_str).map(str::to_owned)
                            })
                            .collect()
                    })
                    .filter(|v: &Vec<String>| !v.is_empty())
                    .unwrap_or_else(|| {
                        d.pointer("/wallet/label")
                            .and_then(Value::as_str)
                            .map(|l| vec![l.to_owned()])
                            .unwrap_or_default()
                    })
            };
            let sync = job.form.sync_after;
            let n = labels.len();
            let title = if existing {
                t("connections:add.bullWallet.mappingSavedTitle")
            } else if n > 1 {
                t("connections:add.added.titlePlural")
            } else {
                t("connections:add.added.title")
            };
            let key = match (existing, sync) {
                (true, true) => "connections:add.bullWallet.mappedBodyRefreshed",
                (true, false) => "connections:add.bullWallet.mappedBody",
                (false, true) => "connections:add.bullWallet.sourcesBodyImported",
                (false, false) => "connections:add.bullWallet.sourcesBody",
            };
            if sync {
                job.syncs.extend(labels);
            }
            job.done = Some((title, tc(key, n, &[]), ToastTone::Success));
            next_sync_job(app, job);
        }
        Stage::BtcpayCreate => {
            let created = super::btcpay::created(app, d);
            if job.form.sync_after {
                job.syncs.extend(created.sync);
            }
            job.done = Some((created.title, created.body, created.tone));
            next_sync_job(app, job);
        }
        Stage::ExchangeCreate => {
            let wallet = d.get("wallet").cloned().unwrap_or(Value::Null);
            let label = wallet
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| job.label.clone());
            let provider =
                super::exchange::provider_name(meta(job.source_id).exchange.unwrap_or(""));
            if job.form.sync_after && wallet.is_object() {
                job.label = label;
                if job.dismissed {
                    job.done = Some((
                        t("connections:add.dismissed.savedScanSkippedTitle"),
                        tr(
                            "connections:add.dismissed.savedScanSkippedBody",
                            &[("label", &job.label)],
                        ),
                        ToastTone::Info,
                    ));
                    finish_job(app, job);
                } else {
                    let id = wallet.get("id").cloned().unwrap_or(Value::Null);
                    dispatch_job(
                        app,
                        job,
                        Stage::ExchangeSync,
                        "ui.connections.exchange.sync",
                        json!({"wallet": id}),
                    );
                }
            } else {
                job.done = Some((
                    tr(
                        "connections:exchange.added.title",
                        &[("provider", provider)],
                    ),
                    tr("connections:exchange.added.body", &[("label", &label)]),
                    ToastTone::Success,
                ));
                finish_job(app, job);
            }
        }
        Stage::ExchangeSync => {
            let provider =
                super::exchange::provider_name(meta(job.source_id).exchange.unwrap_or(""));
            let n = d.get("imported").and_then(Value::as_u64).unwrap_or(0) as usize;
            job.done = Some((
                tr(
                    "connections:exchange.added.title",
                    &[("provider", provider)],
                ),
                tc(
                    "connections:exchange.added.importedBody",
                    n,
                    &[("label", &job.label)],
                ),
                ToastTone::Success,
            ));
            finish_job(app, job);
        }
        Stage::Bip329Preview => {
            if !job.dismissed {
                stm(app).bip329_preview = Some(d.clone());
            }
        }
        Stage::Bip329Import => {
            let n = |p: &str| {
                d.pointer(p)
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .to_string()
            };
            let body = tr(
                "connections:add.bip329.labelsImportedBody",
                &[
                    ("count", &n("/records")),
                    ("added", &n("/transaction_tags_added")),
                    ("skipped", &n("/preview/counts/tag_skipped_ambiguous")),
                ],
            );
            job.done = Some((
                t("connections:add.bip329.labelsImportedTitle"),
                body,
                ToastTone::Success,
            ));
            finish_job(app, job);
        }
        Stage::Sync => {
            // `recordConnectionSetupMutation`: a sync that is not fully "synced" leaves the
            // connection saved but partial. Match against the submitted wallet identity rather
            // than the live progress slot (whose label can change during journal follow-up).
            let wallet = job.sync_wallet.clone().unwrap_or_else(|| job.label.clone());
            let results = d
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if !job.dismissed
                && let Some(res) = results
                    .iter()
                    .find(|x| x.get("wallet").and_then(Value::as_str) == Some(wallet.as_str()))
                && let Some(summary) = import_result_from_sync(res)
            {
                stm(app).last_import = Some(summary);
            }
            if results.is_empty()
                || results
                    .iter()
                    .any(|x| x.get("status").and_then(Value::as_str) != Some("synced"))
            {
                let msg = match sync_failure_detail(&results) {
                    Some(detail) => tr(
                        "connections:add.setupFailed.notFullySyncedDetail",
                        &[("detail", &detail)],
                    ),
                    None => t("connections:add.setupFailed.notFullySynced"),
                };
                return fail_job(app, job, msg);
            }
            next_sync_job(app, job);
        }
    }
}

// ---- Setup actions ---------------------------------------------------------------------------

fn req_matches(slot: &Option<(u64, Option<Sent>)>, expected_gen: u64, app: &App) -> bool {
    let Some((g, sent)) = slot else {
        return false;
    };
    if *g != expected_gen {
        return false;
    }
    match sent {
        Some((_, ticket)) if app.delivering_ticketed() => ticket.answers(app),
        _ => true,
    }
}

/// A setup check's request id and the claim on its answer.
pub type Sent = (String, crate::app::Ticket);

/// "Preview addresses": detect script types first for a bare xpub on auto, then derive five.
/// Script-type detection asks the backend (egress), as the web's does.
pub fn preview_addresses(app: &mut App) {
    let m = meta(selected(app).id);
    let f = &st(app).form;
    if txt(&f.wallet_material).is_empty() || st(app).preview_pending {
        return;
    }
    let epoch = {
        let s = stm(app);
        s.preview_gen += 1;
        s.preview_error = None;
        s.preview_pending = true;
        s.preview_gen
    };
    let f = &st(app).form;
    let bare =
        wallet_material::detect(&f.wallet_material.text).kind == wallet_material::Kind::BareXpub;
    if bare && f.script_type.is_empty() {
        let args = json!({"wallet_material": txt(&f.wallet_material), "backend": opt(&f.backend), "chain": m.chain, "network": m.network});
        let req_id = app.send_ticket_id(
            "ui.wallets.detect_script_types",
            strip_nulls(args),
            "wallets.add.preview_detect",
        );
        stm(app).preview_req = Some((epoch, Some(req_id)));
    } else {
        let types = if bare {
            Some(vec![f.script_type.clone()])
        } else {
            None
        };
        derive(app, types, epoch);
    }
}

fn derive(app: &mut App, script_types: Option<Vec<String>>, epoch: u64) {
    let m = meta(selected(app).id);
    let args = json!({"wallet_material": txt(&st(app).form.wallet_material), "script_types": script_types, "chain": m.chain, "network": m.network, "count": 5});
    let req_id = app.send_ticket_id(
        "ui.wallets.preview_descriptor",
        strip_nulls(args),
        "wallets.add.preview",
    );
    stm(app).preview_req = Some((epoch, Some(req_id)));
}

/// "Detect" (`ui.backends.detect_core`): looks for a local node (loopback probes) — only on
/// its press.
pub fn detect_core(app: &mut App) {
    let epoch = {
        let s = stm(app);
        s.core_test = None;
        s.core_detect_pending = true;
        s.form_gen
    };
    let req_id = app.send_ticket_id(
        "ui.backends.detect_core",
        json!({}),
        "wallets.add.core_detect",
    );
    stm(app).core_detect_req = Some((epoch, Some(req_id)));
}

/// "Test connection" (`ui.backends.bitcoinrpc.test`) — only on its press.
pub fn test_core(app: &mut App) {
    let epoch = {
        let s = stm(app);
        s.core_test = None;
        s.core_test_pending = true;
        s.form_gen
    };
    let args = core_probe_args(app);
    let req_id = app.send_ticket_id("ui.backends.bitcoinrpc.test", args, "wallets.add.core_test");
    stm(app).core_test_req = Some((epoch, Some(req_id)));
}

/// "Check coverage" (`checkWalletCoreCoverage`) — only on its press.
pub fn check_coverage(app: &mut App) {
    let Some(b) = selected_backend(app) else {
        return;
    };
    let mut args = json!({"backend": name_of(&b)});
    if !txt(&form(app).birthday).is_empty() {
        args["birthday"] = json!(iso_date(&form(app).birthday.text));
    }
    let epoch = {
        let s = stm(app);
        s.coverage_gen += 1;
        s.coverage = Coverage::Checking;
        s.coverage_gen
    };
    let req_id = app.send_ticket_id("ui.backends.bitcoinrpc.test", args, "wallets.add.coverage");
    stm(app).coverage_req = Some((epoch, Some(req_id)));
}

/// The coverage inputs changed: drop a verdict about the old ones.
pub fn coverage_inputs_changed(app: &mut App) {
    let s = stm(app);
    s.coverage_gen += 1;
    s.coverage_req = None;
    s.coverage = Coverage::Idle;
}

/// `applyCoreCandidateToForm`.
pub fn apply_core_candidate(app: &mut App, c: &Value) {
    let f = &mut stm(app).form;
    let auth = c.get("auth_source").and_then(Value::as_str);
    if let Some(u) = c
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        f.core_url = rune::TextEdit::new(u);
    }
    if let Some(n) = c
        .get("network")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        f.core_network = n.to_owned();
    }
    f.core_auth = match auth {
        Some("basic") => "basic",
        Some("cookiefile") => "cookiefile",
        _ => f.core_auth,
    };
    if let Some(p) = c
        .get("cookiefile")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        f.core_cookiefile = rune::TextEdit::new(p);
    }
    if auth == Some("basic") {
        f.core_user = rune::TextEdit::new("");
        f.core_pass = rune::TextEdit::new("");
    }
    f.core_credential_ref = c
        .get("credential_ref")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
}

/// "Download XLSX/CSV template" (`ui.transactions.ledger_template`, local), then a Save dialog.
pub fn download_template(app: &mut App, format: &'static str) {
    stm(app).template_pending = true;
    let tag = if format == "csv" {
        "wallets.add.template_csv"
    } else {
        "wallets.add.template_xlsx"
    };
    app.send(
        "ui.transactions.ledger_template",
        json!({"format": format}),
        tag,
    );
}

/// "Export labels" (`ui.metadata.bip329.export`, local), then a Save dialog.
pub fn export_bip329(app: &mut App) {
    let f = &st(app).form;
    let args = strip_nulls(json!({"mode": f.bip329_export_mode, "wallet": opt(&f.bip329_wallet)}));
    stm(app).bip329_export_pending = true;
    app.send(
        "ui.metadata.bip329.export",
        args,
        "wallets.add.bip329_export",
    );
}

fn probe_message(d: &Value, fallback: &str) -> String {
    d.pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| t(fallback))
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "wallets.add.job" => on_job(app, r),
        "wallets.add.backends" | "wallets.add.settings" | "wallets.add.wallets" => {
            if tag == "wallets.add.backends" {
                apply_default_backend(app);
            }
            if tag == "wallets.add.wallets" && r.error.is_none() {
                super::seed_history_source(app);
            }
        }
        "wallets.add.preview_detect" => {
            if !req_matches(&st(app).preview_req, st(app).preview_gen, app) {
                return;
            }
            let epoch = st(app).preview_gen;
            match (
                &r.error,
                wallet_material::script_types_from_detection_payload(Some(&r.data)),
            ) {
                (Some(e), _) => {
                    let s = stm(app);
                    s.preview_pending = false;
                    s.preview_req = None;
                    s.preview_addresses = None;
                    s.preview_error = Some(e.clone());
                }
                (None, Ok(types)) => derive(app, Some(types), epoch),
                (None, Err(reason)) => {
                    let reason = reason.unwrap_or_else(|| {
                        t("connections:add.descriptor.autoDetectUnavailableReasonUnknown")
                    });
                    let s = stm(app);
                    s.preview_pending = false;
                    s.preview_req = None;
                    s.preview_addresses = None;
                    s.preview_error = Some(tr(
                        "connections:add.descriptor.autoDetectUnavailable",
                        &[("reason", &reason)],
                    ));
                }
            }
        }
        "wallets.add.preview" => {
            if !req_matches(&st(app).preview_req, st(app).preview_gen, app) {
                return;
            }
            let s = stm(app);
            s.preview_pending = false;
            s.preview_req = None;
            match &r.error {
                Some(e) => {
                    s.preview_addresses = None;
                    s.preview_error = Some(e.clone());
                }
                None => {
                    s.preview_addresses = Some(
                        r.data
                            .get("addresses")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .map(|x| {
                                        (
                                            x["branch"].as_str().unwrap_or("").to_owned(),
                                            x["index"].as_u64().unwrap_or(0),
                                            x["address"].as_str().unwrap_or("").to_owned(),
                                        )
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                    );
                }
            }
        }
        "wallets.add.core_detect" => {
            if !req_matches(&st(app).core_detect_req, st(app).form_gen, app) {
                return;
            }
            stm(app).core_detect_pending = false;
            stm(app).core_detect_req = None;
            match &r.error {
                Some(e) => {
                    let s = stm(app);
                    s.core_detection = Some(vec![]);
                    s.core_test = Some(CoreTest::Failed {
                        message: e.clone(),
                        hint: None,
                    });
                }
                None => {
                    let c: Vec<Value> = r
                        .data
                        .get("candidates")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    if let Some(first) = c.first().cloned() {
                        apply_core_candidate(app, &first);
                    }
                    stm(app).core_detection = Some(c);
                }
            }
        }
        "wallets.add.core_test" => {
            if !req_matches(&st(app).core_test_req, st(app).form_gen, app) {
                return;
            }
            let s = stm(app);
            s.core_test_pending = false;
            s.core_test_req = None;
            s.core_test = Some(match &r.error {
                Some(e) => CoreTest::Failed {
                    message: e.clone(),
                    hint: None,
                },
                None if r.data.get("reachable").and_then(Value::as_bool) == Some(true) => {
                    CoreTest::Ok(r.data.clone())
                }
                None => CoreTest::Failed {
                    message: probe_message(&r.data, "connections:add.core.testFailed"),
                    hint: r
                        .data
                        .pointer("/error/hint")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                },
            });
        }
        "wallets.add.coverage" => {
            if !matches!(st(app).coverage, Coverage::Checking)
                || !req_matches(&st(app).coverage_req, st(app).coverage_gen, app)
            {
                return; // inputs changed or a newer check replaced this one
            }
            stm(app).coverage_req = None;
            let birthday = !txt(&form(app).birthday).is_empty();
            stm(app).coverage = match &r.error {
                Some(e) => {
                    let code = r
                        .error_detail
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let details = r
                        .error_detail
                        .get("details")
                        .cloned()
                        .unwrap_or(Value::Null);
                    if code == "bitcoinrpc_pruned_below_birthday"
                        || code == "bitcoinrpc_birthday_required"
                    {
                        Coverage::Blocked {
                            message: e.clone(),
                            details,
                        }
                    } else {
                        Coverage::Error {
                            message: e.clone(),
                            details,
                        }
                    }
                }
                None => {
                    let d = r.data.clone();
                    let pruned = d.get("pruned").and_then(Value::as_bool) == Some(true);
                    if d.get("reachable").and_then(Value::as_bool) != Some(true) {
                        Coverage::Error {
                            message: probe_message(&d, "connections:add.core.coverageProbeFailed"),
                            details: Value::Null,
                        }
                    } else if pruned && !birthday {
                        Coverage::BirthdayRequired(d)
                    } else if pruned
                        && d.get("rescan_possible").and_then(Value::as_bool) != Some(true)
                    {
                        Coverage::VerificationRequired(d)
                    } else {
                        Coverage::Available(d)
                    }
                }
            };
        }
        "wallets.add.template_csv" | "wallets.add.template_xlsx" => {
            stm(app).template_pending = false;
            match &r.error {
                Some(e) => {
                    stm(app).setup_error = Some(tr(
                        "connections:add.genericLedger.templateFailedBody",
                        &[("message", e)],
                    ))
                }
                None => {
                    let Some(file) = r
                        .data
                        .get("file")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    else {
                        return;
                    };
                    let ext: &'static str = if tag.ends_with("csv") { "csv" } else { "xlsx" };
                    let daemon_path = file.clone();
                    stm(app).pending_save = Some(PendingSave {
                        file,
                        title: t("connections:add.genericLedger.saveTemplateTitle"),
                        default_name: format!("kassiber-ledger-template.{ext}"),
                        filter: (t("connections:add.fileFilter.genericLedger"), vec![ext]),
                        done: Box::new(move |a: &mut App, saved: Option<String>| {
                            // `toastopen`: Open / Show in Finder for the template just written.
                            let path = saved.unwrap_or_else(|| daemon_path.clone());
                            crate::toastopen::saved_toast(
                                a,
                                ToastTone::Success,
                                &t("connections:add.genericLedger.templateReadyTitle"),
                                Some(t("connections:add.genericLedger.templateReadyBody")),
                                Some("ledger-template"),
                                &path,
                                false,
                            );
                        }),
                    });
                }
            }
        }
        "wallets.add.bip329_export" => {
            stm(app).bip329_export_pending = false;
            match &r.error {
                Some(e) => stm(app).setup_error = Some(e.clone()),
                None => {
                    let Some(file) = r
                        .data
                        .get("file")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    else {
                        return;
                    };
                    let name = r
                        .data
                        .get("filename")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("kassiber-bip329-labels.jsonl")
                        .to_owned();
                    let count =
                        r.data.get("exported").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let daemon_path = file.clone();
                    stm(app).pending_save = Some(PendingSave {
                        file,
                        title: t("connections:add.bip329.exportSaveTitle"),
                        default_name: name,
                        filter: (
                            t("connections:add.bip329.labelFileFilter"),
                            vec!["jsonl", "json"],
                        ),
                        done: Box::new(move |a: &mut App, saved: Option<String>| {
                            let path = saved.unwrap_or_else(|| daemon_path.clone());
                            let body = tc(
                                "connections:add.bip329.exportReadyBody",
                                count,
                                &[("path", &path)],
                            );
                            crate::toastopen::saved_toast(
                                a,
                                ToastTone::Success,
                                &t("connections:add.bip329.exportReadyTitle"),
                                Some(body),
                                Some("bip329-export"),
                                &path,
                                false,
                            );
                        }),
                    });
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Editing the form while a multi-step submit runs (script detection, coverage) does not
    /// change what the later steps create: they read the form as submitted.
    #[test]
    fn later_steps_read_the_form_as_submitted() {
        let mut app = crate::app::tests::quiet_app();
        stm(&mut app).form.wallet_material = rune::TextEdit::new("xpubA");
        start(&mut app, "Cold");
        stm(&mut app).form.wallet_material = rune::TextEdit::new("xpubB");
        assert_eq!(form(&app).wallet_material.text, "xpubA");
        stm(&mut app).job = None;
        assert_eq!(form(&app).wallet_material.text, "xpubB");
    }

    /// The assistant opens the dialog while a submit runs: the job and its dialog stay, so the
    /// running request's answer still finds it.
    #[test]
    fn opening_the_dialog_keeps_a_running_submit() {
        let mut app = crate::app::tests::quiet_app();
        app.ov.add_open = true;
        stm(&mut app).job = Some(Job {
            stage: Stage::Create,
            source_id: "descriptor",
            label: "Cold".into(),
            request_id: None,
            ticket: None,
            sync_wallet: None,
            syncs: VecDeque::new(),
            done: None,
            stay_open: false,
            dismissed: false,
            cancelling: false,
            form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
        });
        super::super::open(&mut app, Some("descriptor"));
        let job = st(&app).job.as_ref().expect("the job runs on");
        assert_eq!((&job.stage, job.label.as_str()), (&Stage::Create, "Cold"));
        assert!(app.ov.add_open);
    }

    /// An import that held Bisq trades leaves the dialog open on its summary; one that held
    /// none closes it as before.
    #[test]
    fn a_bisq_import_with_held_trades_keeps_its_summary_open() {
        for (held, open) in [
            (
                json!([{"trade_id": "AbC12", "status": "Mediated", "reason": "not_completed"}]),
                true,
            ),
            (json!([]), false),
        ] {
            let mut app = crate::app::tests::quiet_app();
            app.ov.add_open = true;
            stm(&mut app).job = Some(Job {
                stage: Stage::ImportFile,
                source_id: "bisq",
                label: "Bisq".into(),
                request_id: None,
                ticket: None,
                sync_wallet: None,
                syncs: VecDeque::new(),
                done: None,
                stay_open: false,
                dismissed: false,
                cancelling: false,
                form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
            });
            let reply = Reply {
                kind: "ui.wallets.import_file".into(),
                data: json!({"imported": 0, "bisq_review_required": held}),
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            };
            on_job(&mut app, &reply);
            assert!(st(&app).job.is_none());
            assert_eq!(app.ov.add_open, open);
            assert!(st(&app).last_import.is_some());
        }
    }

    #[test]
    fn backend_names_slug_like_the_web() {
        assert_eq!(backend_name_from_label("My Node #1"), "my-node-1");
        assert_eq!(backend_name_from_label("   "), "bitcoin-core");
        assert_eq!(backend_name_from_label("core.local_v2"), "core.local_v2");
    }

    #[test]
    fn bisq_sends_its_transactions_export_and_time_zone() {
        let src = crate::overview::catalog::SOURCES
            .iter()
            .find(|s| s.id == "bisq")
            .expect("bisq in the catalog");
        assert_eq!(meta(src.id).kind(), SetupKind::FileWallet);
        assert_eq!(meta(src.id).wallet_kind, Some("bisq"));
        let mut f = super::super::Form::defaults(src);
        f.companion_file = rune::TextEdit::new(" /exports/transactions.csv ");
        f.source_timezone = rune::TextEdit::new("Europe/Vienna");
        let mut args = json!({"source_file": "/exports/history.csv"});
        bisq_args(&f, "bisq_csv", &mut args);
        assert_eq!(args["companion_file"], "/exports/transactions.csv");
        assert_eq!(args["timezone"], "Europe/Vienna");
        // Every other format sends neither.
        let mut other = json!({"source_file": "/exports/strike.csv"});
        bisq_args(&f, "strike_csv", &mut other);
        assert_eq!(other, json!({"source_file": "/exports/strike.csv"}));
    }

    #[test]
    fn dates_normalise_to_iso() {
        assert_eq!(iso_date("2024-03-05"), "2024-03-05");
        assert_eq!(iso_date("03/05/2024"), "2024-03-05");
        assert_eq!(iso_date("05.03.2024"), "2024-03-05");
        assert!(parse_date("2024-03-05"));
        assert!(parse_date("03/05/2024"));
        assert!(parse_date("05.03.2024"));
        assert!(!parse_date("2024-02-30"));
        assert!(!parse_date("not-a-date"));
    }

    #[test]
    fn descriptor_and_silent_payment_validate_birthdays_strictly() {
        let mut app = crate::app::tests::quiet_app();
        super::super::open(&mut app, Some("descriptor"));
        stm(&mut app).form.label = rune::TextEdit::new("Cold");
        stm(&mut app).form.wallet_material = rune::TextEdit::new(
            "wpkh([d34db33f/84h/0h/0h]xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz/0/*)",
        );
        for ok in ["2024-03-05", "03/05/2024", "05.03.2024"] {
            stm(&mut app).form.birthday = rune::TextEdit::new(ok);
            let errs = validate(&mut app);
            assert!(
                !errs.iter().any(|(k, _)| *k == "birthday"),
                "{ok} should validate: {errs:?}"
            );
        }
        for bad in ["2024-02-30", "2023-02-29", "invalid"] {
            stm(&mut app).form.birthday = rune::TextEdit::new(bad);
            let errs = validate(&mut app);
            assert!(
                errs.iter().any(|(k, _)| *k == "birthday"),
                "{bad} should be rejected: {errs:?}"
            );
        }

        // Silent payment start date validation.
        super::super::open(&mut app, Some("silent-payment"));
        stm(&mut app).form.label = rune::TextEdit::new("SP");
        stm(&mut app).form.backend = "sp-local".into();
        stm(&mut app).form.sp_descriptor = rune::TextEdit::new("sp(spscan1qexample)");
        stm(&mut app).form.sp_start_date = rune::TextEdit::new("2024-02-30");
        let errs = validate(&mut app);
        assert!(
            errs.iter().any(|(k, _)| *k == "spScanStartDate"),
            "invalid SP start date must fail validation: {errs:?}"
        );
        stm(&mut app).form.sp_start_date = rune::TextEdit::new("2024-03-05");
        let errs = validate(&mut app);
        assert!(
            !errs.iter().any(|(k, _)| *k == "spScanStartDate"),
            "valid SP start date must pass: {errs:?}"
        );
    }

    #[test]
    fn stop_scan_without_job_id_cancels_by_target_request_id_and_waits_for_worker_exit() {
        let mut app = crate::app::tests::quiet_app();
        app.ov.add_open = true;
        stm(&mut app).job = Some(Job {
            stage: Stage::Sync,
            source_id: "descriptor",
            label: "Cold".into(),
            request_id: Some("req-sync-prefetch".into()),
            ticket: None,
            sync_wallet: Some("Cold".into()),
            syncs: VecDeque::from(["NextWallet".to_owned()]),
            done: None,
            stay_open: false,
            dismissed: false,
            cancelling: false,
            form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
        });
        stm(&mut app).progress = Some(SyncProgress {
            wallet: "Cold".into(),
            job_id: None,
            phase: Some("backend_fetch".into()),
            percent: 15.0,
            ..SyncProgress::default()
        });

        // Clicking Stop scan during prefetch (before any queue job_id exists) MUST still send
        // ui.freshness.cancel with target_request_id, keep tracking until worker exit, and NOT
        // trigger premature reads while the daemon slot is occupied.
        stop_scan(&mut app);
        let cancel_ids: Vec<String> = app
            .pending
            .iter()
            .filter(|(_, (tag, kind, _))| {
                *tag == "wallets.add.cancel" && kind == "ui.freshness.cancel"
            })
            .map(|(id, _)| id.clone())
            .collect();
        assert_eq!(cancel_ids.len(), 1, "must dispatch ui.freshness.cancel");
        let sent = app.sent_args.get(&cancel_ids[0]).expect("cancel args");
        assert_eq!(sent["target_request_id"], "req-sync-prefetch");
        assert!(sent.get("job_id").is_none());

        let job = st(&app)
            .job
            .as_ref()
            .expect("job stays tracked until terminal reply");
        assert!(job.cancelling);
        assert!(job.syncs.is_empty(), "queued follow-up syncs cleared");
        assert!(st(&app).progress.is_some());
        assert!(
            !app.pending
                .values()
                .any(|(tag, _, _)| *tag == "wallets.add.wallets"),
            "must not refresh reads before the freshness worker exits"
        );

        // Once the worker exits and replies, job + progress clear, dialog closes, reads refresh,
        // and no spurious setup_error is set.
        let reply = Reply {
            kind: "ui.wallets.sync".into(),
            data: Value::Null,
            error: Some("cancelled".into()),
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.job", "req-sync-prefetch", reply);
        assert!(st(&app).job.is_none());
        assert!(st(&app).progress.is_none());
        assert!(!app.ov.add_open);
        assert!(st(&app).setup_error.is_none());
        assert!(
            app.pending
                .values()
                .any(|(tag, _, _)| *tag == "wallets.add.wallets"),
            "reads refresh once the worker has exited"
        );
        assert!(
            app.toasts
                .iter()
                .any(|t| t.title == crate::i18n::t("connections:add.dismissed.scanStoppedTitle")),
            "expected scan stopped toast, got {:?}",
            app.toasts
        );
    }

    #[test]
    fn closing_mid_sync_cancels_and_unsticks_dialog_immediately() {
        let mut app = crate::app::tests::quiet_app();
        app.ov.add_open = true;
        stm(&mut app).job = Some(Job {
            stage: Stage::Sync,
            source_id: "descriptor",
            label: "Cold".into(),
            request_id: Some("req-sync-1".into()),
            ticket: None,
            sync_wallet: Some("Cold".into()),
            syncs: VecDeque::new(),
            done: None,
            stay_open: false,
            dismissed: false,
            cancelling: false,
            form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
        });
        stm(&mut app).progress = Some(SyncProgress {
            wallet: "Cold".into(),
            percent: crate::shell::maint::STARTING_VALUE,
            ..SyncProgress::default()
        });
        let env = json!({
            "kind": "ui.wallets.sync.progress",
            "data": {
                "job_id": "job-42",
                "wallet": "Cold",
                "phase": "backend_fetch",
                "processed": 12,
                "total": 40
            }
        });
        on_progress(&mut app, "wallets.add.job", "req-sync-1", &env);
        let p = st(&app).progress.as_ref().expect("progress present");
        assert_eq!(p.job_id.as_deref(), Some("job-42"));
        assert_eq!(p.phase.as_deref(), Some("backend_fetch"));
        assert_eq!((p.processed, p.total), (12, 40));
        assert!((p.percent - 30.0).abs() < 0.01);

        // Closing the dialog immediately closes the panel, sends both target_request_id and
        // job_id to ui.freshness.cancel, and tracks exit in background_jobs.
        super::super::close(&mut app);
        assert!(!app.ov.add_open);
        assert!(!super::super::submitting(&app));
        assert!(st(&app).progress.is_none());
        assert_eq!(st(&app).background_jobs.len(), 1);
        let cancel_id = app
            .pending
            .iter()
            .find(|(_, (tag, kind, _))| {
                *tag == "wallets.add.cancel" && kind == "ui.freshness.cancel"
            })
            .map(|(id, _)| id.clone())
            .expect("closing during an active sync must request ui.freshness.cancel");
        let sent = &app.sent_args[&cancel_id];
        assert_eq!(sent["target_request_id"], "req-sync-1");
        assert_eq!(sent["job_id"], "job-42");

        // Terminal reply clears background_jobs, refreshes reads, and never pollutes setup_error.
        let late = Reply {
            kind: "ui.wallets.sync".into(),
            data: Value::Null,
            error: Some("cancelled".into()),
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.job", "req-sync-1", late);
        assert!(st(&app).background_jobs.is_empty());
        assert!(st(&app).setup_error.is_none());
        assert!(
            app.pending
                .values()
                .any(|(tag, _, _)| *tag == "wallets.add.wallets")
        );
    }

    #[test]
    fn closing_mid_wasabi_rpc_create_completes_bundle_import_in_background() {
        let mut app = crate::app::tests::quiet_app();
        super::super::open(&mut app, Some("wasabi"));
        stm(&mut app).form.label = rune::TextEdit::new("WasabiMain");
        stm(&mut app).form.wasabi_mode = "rpc";
        stm(&mut app).form.wasabi[0] = rune::TextEdit::new(r#"[{"tx":"abc"}]"#);
        let wasabi_form = st(&app).form.clone();
        stm(&mut app).job = Some(Job {
            stage: Stage::Create,
            source_id: "wasabi",
            label: "WasabiMain".into(),
            request_id: Some("req-wasabi-create".into()),
            ticket: None,
            sync_wallet: None,
            syncs: VecDeque::new(),
            done: Some((
                "Added".into(),
                "WasabiMain added".into(),
                ToastTone::Success,
            )),
            stay_open: false,
            dismissed: false,
            cancelling: false,
            form: wasabi_form,
        });

        // Close dialog mid-create and even reopen on a totally different source.
        super::super::close(&mut app);
        assert!(!app.ov.add_open);
        assert_eq!(st(&app).background_jobs.len(), 1);
        super::super::open(&mut app, Some("descriptor"));
        assert_eq!(st(&app).background_jobs.len(), 1);

        // Stage::Create reply arrives -> must dispatch Stage::WasabiImport, not leave empty wallet.
        let create_reply = Reply {
            kind: "ui.wallets.create".into(),
            data: json!({"wallet": {"label": "WasabiMain"}}),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.job", "req-wasabi-create", create_reply);
        assert_eq!(st(&app).background_jobs.len(), 1);
        assert_eq!(st(&app).background_jobs[0].stage, Stage::WasabiImport);
        let import_req_id = st(&app).background_jobs[0]
            .request_id
            .clone()
            .expect("WasabiImport dispatched");
        let sent = &app.sent_args[&import_req_id];
        assert_eq!(sent["wallet"], "WasabiMain");
        assert_eq!(sent["source_format"], "wasabi_bundle");
        assert!(sent["source_bundle"].is_object());

        // Stage::WasabiImport finishes cleanly in background without closing the new dialog.
        let import_reply = Reply {
            kind: "ui.wallets.import_file".into(),
            data: json!({"imported": 3}),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.job", &import_req_id, import_reply);
        assert!(st(&app).background_jobs.is_empty());
        assert!(
            app.ov.add_open,
            "background completion must not close an unrelated reopened dialog"
        );
    }

    #[test]
    fn closing_mid_descriptor_create_skips_optional_sync_and_toasts() {
        let mut app = crate::app::tests::quiet_app();
        super::super::open(&mut app, Some("descriptor"));
        stm(&mut app).job = Some(Job {
            stage: Stage::Create,
            source_id: "descriptor",
            label: "Cold".into(),
            request_id: Some("req-desc-create".into()),
            ticket: None,
            sync_wallet: None,
            syncs: VecDeque::from(["Cold".to_owned()]),
            done: Some(("Added".into(), "Cold added".into(), ToastTone::Success)),
            stay_open: false,
            dismissed: false,
            cancelling: false,
            form: st(&app).form.clone(),
        });

        super::super::close(&mut app);
        let reply = Reply {
            kind: "ui.wallets.create".into(),
            data: json!({"wallet": {"label": "Cold"}}),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.job", "req-desc-create", reply);
        assert!(st(&app).background_jobs.is_empty());
        assert!(
            !app.pending
                .values()
                .any(|(_, kind, _)| kind == "ui.wallets.sync"),
            "dismissed create must skip optional post-create scan"
        );
        assert!(
            app.toasts.iter().any(|t| {
                t.title == crate::i18n::t("connections:add.dismissed.savedScanSkippedTitle")
            }),
            "must inform user that wallet was saved and scan skipped: {:?}",
            app.toasts
        );
    }

    #[test]
    fn journal_progress_retains_wallet_identity_for_final_import_summary() {
        let mut app = crate::app::tests::quiet_app();
        app.ov.add_open = true;
        stm(&mut app).job = Some(Job {
            stage: Stage::Sync,
            source_id: "descriptor",
            label: "Cold".into(),
            request_id: Some("req-sync-1".into()),
            ticket: None,
            sync_wallet: Some("Cold".into()),
            syncs: VecDeque::new(),
            done: None,
            stay_open: true,
            dismissed: false,
            cancelling: false,
            form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
        });
        stm(&mut app).progress = Some(SyncProgress {
            wallet: "Cold".into(),
            ..SyncProgress::default()
        });

        // Progress from an unrelated request ID is ignored.
        on_progress(
            &mut app,
            "wallets.add.job",
            "req-other",
            &json!({"data": {"source_label": "Other", "phase": "backend_fetch", "processed": 99, "total": 100}}),
        );
        assert_eq!(st(&app).progress.as_ref().unwrap().processed, 0);

        // Journal-stage progress under the same request ID emits source_label="Journals" and its
        // own job_id: wallet identity must stay "Cold".
        on_progress(
            &mut app,
            "wallets.add.job",
            "req-sync-1",
            &json!({"data": {"source_label": "Journals", "job_id": "journal-job-9", "phase": "journal_refresh", "processed": 5, "total": 10}}),
        );
        let p = st(&app).progress.as_ref().unwrap();
        assert_eq!(
            p.wallet, "Cold",
            "journal source_label must not overwrite wallet"
        );
        assert_eq!(p.phase.as_deref(), Some("journal_refresh"));
        assert_eq!(p.job_id.as_deref(), Some("journal-job-9"));

        // Terminal Stage::Sync reply still matches "Cold" for import summary.
        let reply = Reply {
            kind: "ui.wallets.sync".into(),
            data: json!({
                "results": [{"wallet": "Cold", "status": "synced", "imported": 7, "skipped": 1}]
            }),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.job", "req-sync-1", reply);
        let summary = st(&app)
            .last_import
            .as_ref()
            .expect("import summary matched Cold");
        assert_eq!(summary["imported"], 7);
    }

    #[test]
    fn stale_coverage_preview_and_core_detect_replies_are_dropped() {
        let mut app = crate::app::tests::quiet_app();
        app.reads.insert(
            "ui.backends.options".into(),
            Reply {
                kind: "ui.backends.options".into(),
                data: json!({
                    "backends": [
                        {"name": "core-1", "kind": "bitcoinrpc", "chain": "bitcoin", "is_default": true}
                    ]
                }),
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            },
        );
        super::super::open(&mut app, Some("descriptor"));
        stm(&mut app).form.backend = "core-1".into();
        stm(&mut app).form.birthday = rune::TextEdit::new("2020-01-01");

        // Start coverage check A.
        check_coverage(&mut app);
        let req_a = st(&app)
            .coverage_req
            .as_ref()
            .and_then(|(_, sent)| sent.as_ref().map(|(id, _)| id.clone()))
            .expect("coverage A id");

        // Change birthday and start coverage check B.
        stm(&mut app).form.birthday = rune::TextEdit::new("2024-01-01");
        coverage_inputs_changed(&mut app);
        check_coverage(&mut app);
        let req_b = st(&app)
            .coverage_req
            .as_ref()
            .and_then(|(_, sent)| sent.as_ref().map(|(id, _)| id.clone()))
            .expect("coverage B id");
        assert_ne!(req_a, req_b);

        // Stale reply A arrives first: must NOT overwrite Checking with A's verdict.
        let reply_a = Reply {
            kind: "ui.backends.bitcoinrpc.test".into(),
            data: json!({"reachable": true, "pruned": true, "rescan_possible": false}),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.coverage", &req_a, reply_a);
        assert!(
            matches!(st(&app).coverage, Coverage::Checking),
            "stale coverage reply A must be dropped"
        );

        // Fresh reply B arrives: wins cleanly.
        let reply_b = Reply {
            kind: "ui.backends.bitcoinrpc.test".into(),
            data: json!({"reachable": true, "pruned": true, "rescan_possible": true}),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        app.deliver_as("wallets.add.coverage", &req_b, reply_b);
        assert!(
            matches!(st(&app).coverage, Coverage::Available(_)),
            "fresh coverage reply B must apply"
        );

        // Preview + Core detect replies arriving after reset_form must be dropped.
        stm(&mut app).form.wallet_material = rune::TextEdit::new("wpkh(xpub123/0/*)");
        preview_addresses(&mut app);
        let prev_req = st(&app)
            .preview_req
            .as_ref()
            .and_then(|(_, sent)| sent.as_ref().map(|(id, _)| id.clone()))
            .expect("preview id");
        detect_core(&mut app);
        let detect_req = st(&app)
            .core_detect_req
            .as_ref()
            .and_then(|(_, sent)| sent.as_ref().map(|(id, _)| id.clone()))
            .expect("detect id");

        let src = super::super::meta::source("descriptor").unwrap();
        super::super::reset_form(&mut app, src);

        app.deliver_as(
            "wallets.add.preview",
            &prev_req,
            Reply {
                kind: "ui.wallets.preview_descriptor".into(),
                data: json!({"addresses": [{"branch": "external", "index": 0, "address": "bc1qstale"}]}),
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            },
        );
        app.deliver_as(
            "wallets.add.core_detect",
            &detect_req,
            Reply {
                kind: "ui.backends.detect_core".into(),
                data: json!({"candidates": [{"url": "http://127.0.0.1:8332"}]}),
                error: None,
                error_detail: Value::Null,
                elapsed: std::time::Duration::ZERO,
            },
        );
        assert!(
            st(&app).preview_addresses.is_none(),
            "stale preview reply after reset_form must be dropped"
        );
        assert!(
            st(&app).core_detection.is_none(),
            "stale core_detect reply after reset_form must be dropped"
        );
    }

    #[test]
    fn sync_failure_surfaces_daemon_error_detail() {
        let mut app = crate::app::tests::quiet_app();
        app.ov.add_open = true;
        stm(&mut app).job = Some(Job {
            stage: Stage::Sync,
            source_id: "descriptor",
            label: "Cold".into(),
            request_id: None,
            ticket: None,
            sync_wallet: Some("Cold".into()),
            syncs: VecDeque::new(),
            done: None,
            stay_open: false,
            dismissed: false,
            cancelling: false,
            form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
        });
        stm(&mut app).progress = Some(SyncProgress {
            wallet: "Cold".into(),
            ..SyncProgress::default()
        });
        let reply = Reply {
            kind: "ui.wallets.sync".into(),
            data: json!({
                "results": [{"wallet": "Cold", "status": "error", "message": "Offline mode is enabled"}]
            }),
            error: None,
            error_detail: Value::Null,
            elapsed: std::time::Duration::ZERO,
        };
        on_job(&mut app, &reply);
        assert!(!super::super::submitting(&app));
        let err = st(&app).setup_error.as_deref().unwrap_or("");
        assert!(
            err.contains("Offline mode is enabled"),
            "expected daemon error detail in {err:?}"
        );
    }

    #[test]
    fn liquid_descriptor_and_bitcoin_address_list_check_matching_backend_options() {
        let mut app = crate::app::tests::quiet_app();
        app.reads.insert(
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
        // Liquid descriptor has NO Liquid backend configured -> backend must not be flagged as missing select.
        super::super::open(&mut app, Some("liquid-descriptor"));
        stm(&mut app).form.backend.clear();
        stm(&mut app).form.wallet_material = rune::TextEdit::new("ct(slip77(...),elwpkh(...))");
        assert!(descriptor_options(&app).is_empty());
        assert_eq!(first_required_missing(&app), None);

        // Bitcoin address list DOES have a Bitcoin backend -> empty backend must flag connection-backend.
        super::super::open(&mut app, Some("address-list"));
        stm(&mut app).form.backend.clear();
        assert_eq!(address_options(&app).len(), 1);
        assert_eq!(
            first_required_missing(&app),
            Some(("connection-backend", true))
        );
    }
}
