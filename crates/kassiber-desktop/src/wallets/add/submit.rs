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
    CoreTest, Coverage, PendingSave, history_import, history_target, history_uses_existing,
    is_history_source, scoped, selected, st, stm,
};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::wallets::assets::model::Replay;

pub const MAX_GAP: i64 = 5000;

/// The submit in flight.
pub struct Job {
    pub stage: Stage,
    pub label: String,
    /// Wallets still to `ui.wallets.sync`.
    pub syncs: VecDeque<String>,
    /// The toast once everything is done.
    pub done: Option<(String, String, ToastTone)>,
    /// Keep the dialog open when done (an import into an existing wallet shows its summary).
    pub stay_open: bool,
    /// The form as submitted: later steps (detection, coverage, the create after them) use
    /// it, not the form's current values, which stay editable while the steps run.
    pub form: super::Form,
    /// The import this job runs, kept for the result (`withReplay`).
    pub replay: Option<crate::wallets::assets::model::Replay>,
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
    let f = form(app);
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
    crate::format::Day::parse(s.trim()).is_some()
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

fn send_job(app: &mut App, stage: Stage, kind: &str, args: Value) {
    if let Some(j) = stm(app).job.as_mut() {
        j.stage = stage;
    }
    let args = scoped(app, args);
    app.send(kind, args, "wallets.add.job");
}

fn start(app: &mut App, label: &str) {
    let form = st(app).form.clone();
    stm(app).job = Some(Job {
        stage: Stage::Create,
        label: label.to_owned(),
        syncs: VecDeque::new(),
        done: None,
        stay_open: false,
        form,
        replay: None,
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
            } else {
                after_coverage(app);
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
        SetupKind::Descriptor => {
            select(backend_required(address_options(app)), "connection-backend")
                .or_else(|| text(empty(&f.wallet_material), "connection-wallet-material"))
        }
        SetupKind::AddressList | SetupKind::Samourai => select(
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

/// A date field's value as `YYYY-MM-DD` (`type="date"` gives ISO; a typed `mm/dd/yyyy` or
/// `dd.mm.yyyy` is converted).
pub fn iso_date(s: &str) -> String {
    let s = s.trim();
    if crate::format::Day::parse(s).is_some() {
        return s.to_owned();
    }
    let parts: Vec<&str> = s.split(['/', '.']).collect();
    if parts.len() == 3 {
        let (a, b, y) = (parts[0], parts[1], parts[2]);
        let (m, d) = if s.contains('/') { (a, b) } else { (b, a) };
        if let (Ok(m), Ok(d), Ok(y)) = (m.parse::<u32>(), d.parse::<u32>(), y.parse::<i32>()) {
            return format!("{y:04}-{m:02}-{d:02}");
        }
    }
    s.to_owned()
}

fn after_coverage(app: &mut App) {
    let src = selected(app);
    let m = meta(src.id);
    let f = form(app);
    if m.kind() == SetupKind::Descriptor {
        let bare = wallet_material::detect(&f.wallet_material.text).kind
            == wallet_material::Kind::BareXpub;
        if bare && f.script_type.is_empty() {
            let args = json!({"wallet_material": txt(&f.wallet_material), "backend": opt(&f.backend), "chain": m.chain, "network": m.network});
            send_job(
                app,
                Stage::DetectScripts,
                "ui.wallets.detect_script_types",
                strip_nulls(args),
            );
            return;
        }
        let types = if bare {
            Some(vec![f.script_type.clone()])
        } else {
            None
        };
        create_descriptor(app, types);
    } else {
        let parsed = address_list::parse(&f.address_list.text);
        let label = st(app)
            .job
            .as_ref()
            .map(|j| j.label.clone())
            .unwrap_or_default();
        let args = json!({
            "label": label,
            "kind": m.wallet_kind.unwrap_or("address"),
            "backend": opt(&f.backend),
            "chain": m.chain,
            "network": m.network,
            "addresses": parsed.valid,
            "birthday": opt(&iso_date(&f.birthday.text)),
        });
        let sync = f.sync_after;
        if let Some(j) = stm(app).job.as_mut() {
            if sync {
                j.syncs.push_back(label.clone());
            }
            j.done = Some((
                t("connections:add.added.title"),
                tc(
                    "connections:add.addressList.addedBody",
                    parsed.valid.len(),
                    &[("label", &label)],
                ),
                ToastTone::Success,
            ));
        }
        send_job(app, Stage::Create, "ui.wallets.create", strip_nulls(args));
    }
}

fn create_descriptor(app: &mut App, script_types: Option<Vec<String>>) {
    let m = meta(selected(app).id);
    let f = form(app);
    let label = st(app)
        .job
        .as_ref()
        .map(|j| j.label.clone())
        .unwrap_or_default();
    let args = json!({
        "label": label,
        "kind": m.wallet_kind.unwrap_or("descriptor"),
        "backend": opt(&f.backend),
        "chain": m.chain,
        "network": m.network,
        "wallet_material": txt(&f.wallet_material),
        "script_types": script_types,
        "gap_limit": txt(&f.gap_limit).parse::<i64>().ok(),
        "birthday": opt(&iso_date(&f.birthday.text)),
    });
    let sync = f.sync_after;
    if let Some(j) = stm(app).job.as_mut() {
        if sync {
            j.syncs.push_back(label.clone());
        }
        j.done = Some((
            t("connections:add.added.title"),
            tr("connections:add.added.body", &[("label", &label)]),
            ToastTone::Success,
        ));
    }
    send_job(app, Stage::Create, "ui.wallets.create", strip_nulls(args));
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
        // A path the user chose here is kept for importing it again (`withReplay`); a bundle
        // is not.
        let replay = bundle.is_none().then(|| Replay {
            wallet: id.as_str().unwrap_or_default().to_owned(),
            source_format: fmt.to_owned(),
            source_file: txt(&f.source_file),
        });
        match bundle {
            Some(b) => args["source_bundle"] = b,
            None => args["source_file"] = json!(txt(&f.source_file)),
        }
        bisq_args(f, fmt, &mut args);
        if let Some(j) = stm(app).job.as_mut() {
            j.replay = replay;
        }
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
    // The sync imports the file the user chose here; its result keeps the path for importing
    // it again (`withReplay`).
    let replay = sync.then(|| Replay {
        wallet: label.to_owned(),
        source_format: fmt.to_owned(),
        source_file: txt(&f.source_file),
    });
    if let Some(j) = stm(app).job.as_mut() {
        if sync {
            j.syncs.push_back(label.to_owned());
        }
        j.replay = replay;
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
fn core_config(app: &App) -> Value {
    let f = form(app);
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
        json!({"url": txt(&f.core_url), "network": if f.core_network.is_empty() { "main" } else { f.core_network.as_str() }, "config": core_config(app), "timeout": 10})
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

fn core_backend_args(app: &App) -> Value {
    let f = form(app);
    let mut a = json!({
        "name": backend_name_from_label(&f.label.text),
        "kind": "bitcoinrpc",
        "url": txt(&f.core_url),
        "chain": "bitcoin",
        "network": if f.core_network.is_empty() { "main" } else { f.core_network.as_str() },
        "config": core_config(app),
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
    let s = stm(app);
    s.job = None;
    s.progress = None;
    s.setup_error = Some(message);
    app.refresh_reads();
    app.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
}

fn finish(app: &mut App) {
    let Some(job) = stm(app).job.take() else {
        return;
    };
    stm(app).progress = None;
    if let Some((title, body, tone)) = job.done {
        app.toast(tone, &title, Some(body), Some("connection-setup"));
    }
    app.refresh_reads();
    app.send("ui.wallets.list", Value::Null, "wallets.add.wallets");
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

fn next_sync(app: &mut App) {
    let next = stm(app).job.as_mut().and_then(|j| j.syncs.pop_front());
    match next {
        Some(w) => {
            stm(app).progress = Some((w.clone(), 0, 0));
            send_job(app, Stage::Sync, "ui.wallets.sync", json!({"wallet": w}));
        }
        None => finish(app),
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

/// An import result for the summary, with the replay of the job that produced it; the asset
/// scope notice under it starts over.
fn set_last_import(app: &mut App, result: Value) {
    let replay = stm(app).job.as_mut().and_then(|j| j.replay.take());
    let s = stm(app);
    s.last_import = Some(result);
    s.last_import_replay = replay;
    crate::wallets::assets::reset(app, crate::wallets::assets::Surface::Import);
}

fn on_job(app: &mut App, r: &Reply) {
    let Some(stage) = st(app).job.as_ref().map(|j| j.stage.clone()) else {
        return;
    };
    if let Some(e) = &r.error {
        fail(app, e.clone());
        return;
    }
    let d = &r.data;
    match stage {
        Stage::CoreTest => {
            if d.get("reachable").and_then(Value::as_bool) != Some(true) {
                return fail(
                    app,
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
                return fail(app, msg);
            }
            let label = st(app)
                .job
                .as_ref()
                .map(|j| j.label.clone())
                .unwrap_or_default();
            if let Some(j) = stm(app).job.as_mut() {
                j.done = Some((
                    t("connections:add.added.title"),
                    tr("connections:add.core.addedBody", &[("label", &label)]),
                    ToastTone::Success,
                ));
            }
            let args = core_backend_args(app);
            send_job(app, Stage::CoreCreate, "ui.backends.create", args);
        }
        Stage::CoreCreate => finish(app),
        Stage::Coverage => {
            if d.get("reachable").and_then(Value::as_bool) != Some(true) {
                return fail(
                    app,
                    d.pointer("/error/message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| t("connections:add.core.coverageProbeFailed")),
                );
            }
            if d.get("pruned").and_then(Value::as_bool) == Some(true)
                && txt(&form(app).birthday).is_empty()
            {
                return fail(app, t("connections:add.core.birthdayRequiredForPruned"));
            }
            after_coverage(app);
        }
        Stage::DetectScripts => match wallet_material::script_types_from_detection_payload(Some(d))
        {
            Ok(types) => create_descriptor(app, Some(types)),
            Err(reason) => {
                let reason = reason.unwrap_or_else(|| {
                    t("connections:add.descriptor.autoDetectUnavailableReasonUnknown")
                });
                fail(
                    app,
                    tr(
                        "connections:add.descriptor.autoDetectUnavailable",
                        &[("reason", &reason)],
                    ),
                );
            }
        },
        Stage::Create => {
            let rpc = meta(selected(app).id).source_format == Some("wasabi_bundle")
                && form(app).wasabi_mode == "rpc";
            if rpc {
                let w = &form(app).wasabi;
                let b = wasabi::build_bundle(&w[0].text, &w[1].text, &w[2].text, &w[3].text);
                let label = st(app)
                    .job
                    .as_ref()
                    .map(|j| j.label.clone())
                    .unwrap_or_default();
                send_job(
                    app,
                    Stage::WasabiImport,
                    "ui.wallets.import_file",
                    json!({"wallet": label, "source_format": "wasabi_bundle", "source_bundle": b.bundle}),
                );
            } else {
                next_sync(app);
            }
        }
        Stage::WasabiImport | Stage::ImportFile => {
            set_last_import(app, d.clone());
            if meta(selected(app).id).kind() == SetupKind::FileEnrichment {
                let book = is_exchange_evidence(meta(selected(app).id).source_format);
                let scope = if book {
                    t("connections:add.enrichment.bookScope")
                } else {
                    form(app).target_wallet.clone()
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
                if let Some(j) = stm(app).job.as_mut() {
                    j.done = Some((
                        t("connections:add.enrichment.finishedTitle"),
                        body,
                        ToastTone::Success,
                    ));
                }
            }
            finish(app);
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
            let label = st(app)
                .job
                .as_ref()
                .map(|j| j.label.clone())
                .unwrap_or_default();
            let sync = form(app).sync_after;
            let value = crate::format::count(children.len());
            let body = if sync {
                tr(
                    "connections:add.samourai.importedBodyScanning",
                    &[("label", &label), ("value", &value)],
                )
            } else {
                tr(
                    "connections:add.samourai.importedBody",
                    &[("label", &label), ("value", &value)],
                )
            };
            if let Some(j) = stm(app).job.as_mut() {
                if sync {
                    j.syncs.extend(children);
                }
                j.done = Some((
                    t("connections:add.samourai.importedTitle"),
                    body,
                    ToastTone::Success,
                ));
            }
            for e in stm(app).form.samourai.iter_mut() {
                *e = rune::TextEdit::new("");
            }
            next_sync(app);
        }
        Stage::PriorReport => {
            let label = d
                .pointer("/document/label")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    st(app)
                        .job
                        .as_ref()
                        .map(|j| j.label.clone())
                        .unwrap_or_default()
                });
            if let Some(j) = stm(app).job.as_mut() {
                j.done = Some((
                    t("connections:add.priorReport.archivedTitle"),
                    tr(
                        "connections:add.priorReport.archivedBody",
                        &[("label", &label)],
                    ),
                    ToastTone::Success,
                ));
            }
            finish(app);
        }
        Stage::BullCreate => {
            let existing = form(app).bull_mode == "existing_wallets";
            let labels: Vec<String> = if existing {
                let mut v: Vec<String> = Vec::new();
                for (_, w) in bull_routes(app) {
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
            let sync = form(app).sync_after;
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
            if let Some(j) = stm(app).job.as_mut() {
                if sync {
                    j.syncs.extend(labels);
                }
                j.done = Some((title, tc(key, n, &[]), ToastTone::Success));
            }
            next_sync(app);
        }
        Stage::BtcpayCreate => {
            let created = super::btcpay::created(app, d);
            let sync = form(app).sync_after;
            if let Some(j) = stm(app).job.as_mut() {
                if sync {
                    j.syncs.extend(created.sync);
                }
                j.done = Some((created.title, created.body, created.tone));
            }
            next_sync(app);
        }
        Stage::ExchangeCreate => {
            let wallet = d.get("wallet").cloned().unwrap_or(Value::Null);
            let label = wallet
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    st(app)
                        .job
                        .as_ref()
                        .map(|j| j.label.clone())
                        .unwrap_or_default()
                });
            let provider =
                super::exchange::provider_name(meta(selected(app).id).exchange.unwrap_or(""));
            if form(app).sync_after && wallet.is_object() {
                if let Some(j) = stm(app).job.as_mut() {
                    j.label = label;
                }
                let id = wallet.get("id").cloned().unwrap_or(Value::Null);
                send_job(
                    app,
                    Stage::ExchangeSync,
                    "ui.connections.exchange.sync",
                    json!({"wallet": id}),
                );
            } else {
                if let Some(j) = stm(app).job.as_mut() {
                    j.done = Some((
                        tr(
                            "connections:exchange.added.title",
                            &[("provider", provider)],
                        ),
                        tr("connections:exchange.added.body", &[("label", &label)]),
                        ToastTone::Success,
                    ));
                }
                finish(app);
            }
        }
        Stage::ExchangeSync => {
            let label = st(app)
                .job
                .as_ref()
                .map(|j| j.label.clone())
                .unwrap_or_default();
            let provider =
                super::exchange::provider_name(meta(selected(app).id).exchange.unwrap_or(""));
            let n = d.get("imported").and_then(Value::as_u64).unwrap_or(0) as usize;
            if let Some(j) = stm(app).job.as_mut() {
                j.done = Some((
                    tr(
                        "connections:exchange.added.title",
                        &[("provider", provider)],
                    ),
                    tc(
                        "connections:exchange.added.importedBody",
                        n,
                        &[("label", &label)],
                    ),
                    ToastTone::Success,
                ));
            }
            finish(app);
        }
        Stage::Bip329Preview => {
            let s = stm(app);
            s.bip329_preview = Some(d.clone());
            s.job = None;
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
            if let Some(j) = stm(app).job.as_mut() {
                j.done = Some((
                    t("connections:add.bip329.labelsImportedTitle"),
                    body,
                    ToastTone::Success,
                ));
            }
            finish(app);
        }
        Stage::Sync => {
            // `recordConnectionSetupMutation`: a sync that is not fully "synced" leaves the
            // connection saved but partial.
            let wallet = st(app)
                .progress
                .as_ref()
                .map(|p| p.0.clone())
                .unwrap_or_default();
            let results = d
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(res) = results
                .iter()
                .find(|x| x.get("wallet").and_then(Value::as_str) == Some(wallet.as_str()))
                && let Some(summary) = import_result_from_sync(res)
            {
                set_last_import(app, summary);
            }
            if results.is_empty()
                || results
                    .iter()
                    .any(|x| x.get("status").and_then(Value::as_str) != Some("synced"))
            {
                return fail(app, t("connections:add.setupFailed.notFullySynced"));
            }
            next_sync(app);
        }
    }
}

// ---- Setup actions ---------------------------------------------------------------------------

/// "Preview addresses": detect script types first for a bare xpub on auto, then derive five.
/// Script-type detection asks the backend (egress), as the web's does.
pub fn preview_addresses(app: &mut App) {
    let m = meta(selected(app).id);
    let f = &st(app).form;
    if txt(&f.wallet_material).is_empty() || st(app).preview_pending {
        return;
    }
    stm(app).preview_error = None;
    stm(app).preview_pending = true;
    let f = &st(app).form;
    let bare =
        wallet_material::detect(&f.wallet_material.text).kind == wallet_material::Kind::BareXpub;
    if bare && f.script_type.is_empty() {
        let args = json!({"wallet_material": txt(&f.wallet_material), "backend": opt(&f.backend), "chain": m.chain, "network": m.network});
        app.send(
            "ui.wallets.detect_script_types",
            strip_nulls(args),
            "wallets.add.preview_detect",
        );
    } else {
        let types = if bare {
            Some(vec![f.script_type.clone()])
        } else {
            None
        };
        derive(app, types);
    }
}

fn derive(app: &mut App, script_types: Option<Vec<String>>) {
    let m = meta(selected(app).id);
    let args = json!({"wallet_material": txt(&st(app).form.wallet_material), "script_types": script_types, "chain": m.chain, "network": m.network, "count": 5});
    app.send(
        "ui.wallets.preview_descriptor",
        strip_nulls(args),
        "wallets.add.preview",
    );
}

/// "Detect" (`ui.backends.detect_core`): looks for a local node (loopback probes) — only on
/// its press.
pub fn detect_core(app: &mut App) {
    let s = stm(app);
    s.core_test = None;
    s.core_detect_pending = true;
    app.send(
        "ui.backends.detect_core",
        json!({}),
        "wallets.add.core_detect",
    );
}

/// "Test connection" (`ui.backends.bitcoinrpc.test`) — only on its press.
pub fn test_core(app: &mut App) {
    let s = stm(app);
    s.core_test = None;
    s.core_test_pending = true;
    let args = core_probe_args(app);
    app.send("ui.backends.bitcoinrpc.test", args, "wallets.add.core_test");
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
    stm(app).coverage = Coverage::Checking;

    app.send("ui.backends.bitcoinrpc.test", args, "wallets.add.coverage");
}

/// The coverage inputs changed: drop a verdict about the old ones.
pub fn coverage_inputs_changed(app: &mut App) {
    let s = stm(app);
    s.coverage_gen += 1;
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
        "wallets.add.preview_detect" => match (
            &r.error,
            wallet_material::script_types_from_detection_payload(Some(&r.data)),
        ) {
            (Some(e), _) => {
                let s = stm(app);
                s.preview_pending = false;
                s.preview_addresses = None;
                s.preview_error = Some(e.clone());
            }
            (None, Ok(types)) => derive(app, Some(types)),
            (None, Err(reason)) => {
                let reason = reason.unwrap_or_else(|| {
                    t("connections:add.descriptor.autoDetectUnavailableReasonUnknown")
                });
                let s = stm(app);
                s.preview_pending = false;
                s.preview_addresses = None;
                s.preview_error = Some(tr(
                    "connections:add.descriptor.autoDetectUnavailable",
                    &[("reason", &reason)],
                ));
            }
        },
        "wallets.add.preview" => {
            let s = stm(app);
            s.preview_pending = false;
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
            stm(app).core_detect_pending = false;
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
            let s = stm(app);
            s.core_test_pending = false;
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
            if !matches!(st(app).coverage, Coverage::Checking) {
                return; // inputs changed meanwhile
            }
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
            label: "Cold".into(),
            syncs: VecDeque::new(),
            done: None,
            stay_open: false,
            form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
            replay: None,
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
                label: "Bisq".into(),
                syncs: VecDeque::new(),
                done: None,
                stay_open: false,
                form: super::super::Form::defaults(&crate::overview::catalog::SOURCES[0]),
                replay: None,
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
    }
}
