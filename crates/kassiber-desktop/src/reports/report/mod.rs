//! Reports (`/reports`): routes/Reports.tsx and components/reports/**.
//!
//! - `model`: the report read, its breakdown, readiness and formatting (reportModel.ts);
//! - `view`: the header, metric strip, breakdown and rules cards;
//! - `files`: the Files panel and the export's status;
//! - `extra`: the Lightning profitability panel and the capital-gain explanation
//!   (`/reports?result=<reference>`);
//! - `widgets`: selects, info notes, checkboxes and links shared by the above.
//!
//! Reads on mount (all local): `ui.reports.capital_gains {year?}`, `ui.wallets.list`,
//! `ui.source_funds.cases.list`; completeness comes from the shell's `ui.overview.snapshot`.
//! Exports run only from a press and write into the book's `exports/reports` folder, then ask
//! where to keep a copy with the native Save dialog (`save_exported_file_as`).
//! `ui.reports.lightning_profitability` contacts the node, so it runs only from "Run report".

pub mod extra;
mod files;
pub mod model;
mod view;
mod widgets;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use rune::{Cx, El, FileDialogOptions, KeyEvent, Tx};
use serde_json::{Value, json};

use crate::app::{App, Load, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

/// Which Reports popup is open (one at a time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Year,
    AuditScope,
    AuditCase,
    LnConnection,
    /// The tax report's formats menu (PDF, XLSX, CSV) under its Export button.
    TaxFormats,
    /// An `InfoNote`, by its key.
    Note(String),
}

/// The folded options under a file row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Summary,
    Audit,
}

/// `ReportExportFormatId`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Pdf,
    Xlsx,
    Csv,
    YearHandoff,
    SummaryPdf,
    AuditPackage,
}

#[derive(Clone, Debug)]
pub struct Audit {
    pub case_scope: bool,
    pub case_id: String,
    /// copiedAttachments, urlReferences, journalState, reviewState, editHistory.
    pub include: [bool; 5],
}

impl Default for Audit {
    fn default() -> Self {
        Audit {
            case_scope: false,
            case_id: String::new(),
            include: [true, true, true, true, false],
        }
    }
}

/// The audit package's parts (translation keys), in `Audit::include` order.
pub const AUDIT_PARTS: [&str; 5] = [
    "reports:files.audit.part.copiedAttachments",
    "reports:files.audit.part.urlReferences",
    "reports:files.audit.part.journalState",
    "reports:files.audit.part.reviewState",
    "reports:files.audit.part.editHistory",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTone {
    Success,
    Warning,
    Error,
}

/// `QuarantineExportBlock`.
#[derive(Clone, Debug, Default)]
pub struct Quarantine {
    pub count: usize,
    pub rows: Vec<Value>,
}

/// `ReportExportStatus`: the last export's outcome, kept beside the files.
#[derive(Clone, Debug)]
pub struct Status {
    pub year: i32,
    pub tone: StatusTone,
    pub message: String,
    pub notes: Vec<String>,
    pub path: Option<String>,
    pub open_path: Option<String>,
    pub quarantine: Option<Quarantine>,
    /// `toastopen`: Show in Finder for the managed file (beside the strip's Open), or Open +
    /// Show in Finder for a saved copy; empty for errors and unknown paths.
    pub actions: Vec<crate::toastopen::ToastAction>,
}

#[derive(Clone, Debug)]
pub struct WalletChoice {
    pub id: String,
    pub label: String,
    pub chain: String,
}

/// A written export waiting for its Save As answer.
#[derive(Clone, Debug)]
pub struct Written {
    pub fmt: Fmt,
    pub year: i32,
    pub path: String,
    pub payload: Value,
}

#[derive(Default)]
pub struct Ui {
    /// `?year=`; None follows the daemon's default year.
    pub year: Option<i32>,
    pub route_seen: String,
    /// Capital-gains reads by year (0 = the daemon's default).
    pub reports: HashMap<i32, Load<Value>>,
    /// Years whose cached read is stale and must be fetched again (kept on screen meanwhile).
    pub stale: Vec<i32>,
    /// The years of the reads in flight, in send order. None: asked before a book switch,
    /// dropped when it answers.
    pub queue: VecDeque<Option<i32>>,
    pub wallets: Option<Vec<WalletChoice>>,
    pub wallets_requested: bool,
    pub cases: Vec<(String, String)>,
    pub cases_requested: bool,
    pub popup: Option<Popup>,
    pub fold: Option<Fold>,
    pub no_cover: bool,
    pub summary_wallets: Vec<String>,
    pub audit: Audit,
    pub active: Option<Fmt>,
    /// The year the running export belongs to (fixed at its start).
    pub export_year: i32,
    /// The format that just finished, its year and the clock time its check goes away.
    pub done: Option<(Fmt, i32, f64)>,
    pub status: Option<Status>,
    pub pending_save: Option<Written>,
    pub book: Option<String>,
    pub ln_selected: Option<String>,
    /// `ui.reports.lightning_profitability` for (connection id): None until "Run report".
    pub ln: Option<(String, Load<Value>)>,
    /// The connection of each profitability read and the reference of each explanation read in
    /// flight, in send order (replies come in order): a reply lands only on what asked for it.
    /// Kept through a book switch, since the replies still come.
    pub asked: Asked,
    /// Bumped each time the page is left (`leave`): an export started on an earlier visit
    /// finishes without a strip, as the web's `setExportStatus` on an unmounted view does.
    pub visit: u64,
    /// The visit the running export was started on.
    pub export_visit: u64,
    /// `/reports?result=…`: (reference JSON, explanation read).
    pub explain: Option<(String, Load<Value>)>,
    /// Open `<details>` nodes of the explanation.
    pub explain_open: Vec<String>,
    /// A select's trigger to focus again once the page shows it (key, with its ring): picking a
    /// year reloads the report, and the trigger is gone while it loads.
    pub refocus: Option<(String, bool)>,
}

impl Ui {
    /// Leaving `/reports` unmounts `ReportOverview`: what it keeps in `useState` (the export's
    /// status strip, the 5 s check, folded options, summary and audit choices, the Lightning
    /// selection, open menus) starts over on the next visit. The reads (React Query's cache) and
    /// a running export stay: its Save As and notification follow the promise, and its busy
    /// state keeps a second export from starting meanwhile (the web loses `activeExport` here).
    pub fn leave(&mut self) {
        self.visit += 1;
        self.popup = None;
        self.fold = None;
        self.status = None;
        self.done = None;
        self.no_cover = false;
        self.audit = Audit::default();
        // `summaryWalletIds` starts empty and its effect selects every wallet.
        self.summary_wallets = self
            .wallets
            .as_ref()
            .map(|w| w.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default();
        self.ln_selected = None;
        self.explain_open.clear();
        self.route_seen.clear();
    }

    /// The running export belongs to this visit of the page (its outcome may show).
    fn same_visit(&self) -> bool {
        self.visit == self.export_visit
    }

    fn key(&self) -> i32 {
        self.year.unwrap_or(0)
    }

    /// The read for the selected year.
    pub fn current(&self) -> Option<&Load<Value>> {
        self.reports.get(&self.key())
    }
}

pub fn owns(path: &str) -> bool {
    path == "/reports"
}

// ---- Loading --------------------------------------------------------------------------------

fn send_report(app: &mut App, key: i32) {
    let args = if key == 0 {
        Value::Null
    } else {
        json!({"year": key})
    };
    app.reports.report.queue.push_back(Some(key));
    app.send("ui.reports.capital_gains", args, "reports.report.cg");
}

/// The reads the page makes on mount and after the route changes. No egress.
fn step(app: &mut App) -> bool {
    if app.route.path() != "/reports" {
        return false;
    }
    let mut changed = false;
    // A book switch drops the per-book state (a running export keeps its outcome).
    let book = app
        .read("ui.profiles.snapshot")
        .and_then(|p| p.get("activeProfileId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if book.is_some() && app.reports.report.book.is_some() && book != app.reports.report.book {
        let ui = std::mem::take(&mut app.reports.report);
        app.reports.report = Ui {
            active: ui.active,
            export_year: ui.export_year,
            pending_save: ui.pending_save,
            visit: ui.visit,
            export_visit: ui.export_visit,
            asked: ui.asked,
            queue: ui.queue.iter().map(|_| None).collect(),
            ..Ui::default()
        };
        changed = true;
    }
    if book.is_some() {
        app.reports.report.book = book;
    }
    if app.route.0 != app.reports.report.route_seen {
        let route = app.route.0.clone();
        let ui = &mut app.reports.report;
        ui.route_seen = route.clone();
        ui.year = model::year_from_route(&route);
        match model::result_param(&route) {
            Some(r) if ui.explain.as_ref().map(|(k, _)| k) != Some(&r) => {
                ui.explain = Some((r.clone(), Load::Loading));
                ui.explain_open.clear();
                let reference: Value = serde_json::from_str(&r).unwrap_or(Value::Null);
                app.reports.report.asked.explain.push_back(r);
                app.send(
                    "ui.reports.explain_capital_gain",
                    json!({"reference": reference}),
                    "reports.report.explain",
                );
            }
            Some(_) => {}
            None => ui.explain = None,
        }
        changed = true;
    }
    if app.reports.report.explain.is_some() {
        return changed;
    }
    let key = app.reports.report.key();
    if let std::collections::hash_map::Entry::Vacant(e) = app.reports.report.reports.entry(key) {
        e.insert(Load::Loading);
        send_report(app, key);
        changed = true;
    } else if let Some(i) = app.reports.report.stale.iter().position(|&y| y == key) {
        app.reports.report.stale.remove(i);
        send_report(app, key);
    }
    if !app.reports.report.wallets_requested {
        app.reports.report.wallets_requested = true;
        app.send("ui.wallets.list", Value::Null, "reports.report.wallets");
    }
    if !app.reports.report.cases_requested {
        app.reports.report.cases_requested = true;
        app.send(
            "ui.source_funds.cases.list",
            Value::Null,
            "reports.report.cases",
        );
    }
    changed
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    // The Save As for a finished export runs even after leaving Reports, as the web's promise does.
    if let Some(w) = app.reports.report.pending_save.take() {
        ask_save(w, tx);
        changed = true;
    }
    if let Some((_, _, until)) = app.reports.report.done {
        if app.clock >= until {
            app.reports.report.done = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                (until - app.clock).max(0.05),
            ));
        }
    }
    // Radix Select hands focus back to its trigger after a pick: once the page is back.
    if app
        .reports
        .report
        .current()
        .is_some_and(|l| matches!(l, Load::Ready(_) | Load::Failed(_)))
        && let Some((key, ring)) = app.reports.report.refocus.take()
    {
        tx.focus(key, ring);
        changed = true;
    }
    changed | step(app)
}

pub fn prime(app: &mut App) {
    step(app);
    super::pump_until(app, |a| a.pending.is_empty());
}

/// The shell re-reads the overview after every sync, journal run and book switch; the web
/// invalidates every daemon query then, so the cached reports go stale (kept while refetching).
pub fn on_any_reply(app: &mut App, tag: &str, _r: &Reply) {
    if tag == "app.overview" {
        let ui = &mut app.reports.report;
        ui.stale = ui.reports.keys().copied().collect();
        ui.cases_requested = false;
        ui.wallets_requested = false;
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "reports.report.cg" => {
            let ui = &mut app.reports.report;
            let Some(Some(key)) = ui.queue.pop_front() else {
                return;
            };
            let next = match (&r.error, r.data.is_object()) {
                (Some(e), _) => Load::Failed(e.clone()),
                (None, true) => Load::Ready(r.data.clone()),
                (None, false) => Load::Failed(t("reports:page.noData")),
            };
            // A failed refetch replaces the shown read: figures the book no longer produces must
            // not stay up as current (the next book refresh reads the year again).
            ui.reports.insert(key, next);
        }
        "reports.report.wallets" if r.error.is_none() => {
            let choices: Vec<WalletChoice> = r.data["wallets"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|w| {
                            let label = w["label"].as_str().unwrap_or("").to_owned();
                            let id = w["id"]
                                .as_str()
                                .map(str::to_owned)
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| label.clone());
                            (!id.is_empty()).then(|| WalletChoice {
                                id,
                                label,
                                chain: w["chain"].as_str().unwrap_or("").to_owned(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            // Keep the selection's survivors; a selection with none left takes every wallet.
            let ui = &mut app.reports.report;
            let all: Vec<String> = choices.iter().map(|w| w.id.clone()).collect();
            ui.summary_wallets = if all.len() <= 1 {
                all
            } else {
                let kept: Vec<String> = all
                    .iter()
                    .filter(|id| ui.summary_wallets.contains(id))
                    .cloned()
                    .collect();
                if kept.is_empty() { all } else { kept }
            };
            ui.wallets = Some(choices);
        }
        "reports.report.cases" if r.error.is_none() => {
            let ui = &mut app.reports.report;
            ui.cases = r.data["cases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            let id = c["id"].as_str()?.to_owned();
                            let label = [&c["label"], &c["target_external_id"]]
                                .iter()
                                .find_map(|v| v.as_str().filter(|s| !s.is_empty()))
                                .unwrap_or(&id)
                                .to_owned();
                            Some((id, label))
                        })
                        .collect()
                })
                .unwrap_or_default();
            fill_case(ui);
        }
        "reports.report.export" => on_export(app, r),
        "reports.report.ln" => {
            let asked = app.reports.report.asked.ln.pop_front();
            if let Some((_, load)) = app
                .reports
                .report
                .ln
                .as_mut()
                .filter(|(c, _)| Some(c) == asked.as_ref())
            {
                *load = match &r.error {
                    Some(e) => Load::Failed(e.clone()),
                    None => Load::Ready(r.data.clone()),
                };
            }
        }
        "reports.report.explain" => {
            let asked = app.reports.report.asked.explain.pop_front();
            if let Some((_, load)) = app
                .reports
                .report
                .explain
                .as_mut()
                .filter(|(k, _)| Some(k) == asked.as_ref())
            {
                *load = match &r.error {
                    Some(e) => Load::Failed(e.clone()),
                    None => Load::Ready(r.data.clone()),
                };
            }
        }
        _ => {}
    }
}

/// What the profitability and explanation reads in flight were asked for, in send order.
#[derive(Default)]
pub struct Asked {
    pub ln: VecDeque<String>,
    pub explain: VecDeque<String>,
}

/// The audit package's case scope takes the first saved case when none is chosen.
fn fill_case(ui: &mut Ui) {
    if ui.audit.case_scope
        && ui.audit.case_id.is_empty()
        && let Some((id, _)) = ui.cases.first()
    {
        ui.audit.case_id = id.clone();
    }
}

pub fn set_case_scope(app: &mut App, on: bool) {
    app.reports.report.audit.case_scope = on;
    fill_case(&mut app.reports.report);
}

// ---- Year -----------------------------------------------------------------------------------

/// `handleYearChange`: writes `?year=` in place (`history.replaceState`) and loads that year.
pub fn set_year(app: &mut App, year: i32) {
    app.reports.report.popup = None;
    app.reports.report.year = Some(year);
    app.route.0 = format!("/reports?year={year}");
    app.reports.report.route_seen = app.route.0.clone();
}

// ---- Export ---------------------------------------------------------------------------------

/// The year the page shows: the selection, else the report's.
pub fn effective_year(app: &App) -> Option<i32> {
    let ui = &app.reports.report;
    ui.year.or_else(|| {
        ui.current()
            .and_then(Load::ready)
            .and_then(|d| d["year"].as_i64())
            .map(|y| y as i32)
    })
}

/// `handleExport`: the daemon writes the file into the book's exports folder.
pub fn export(app: &mut App, fmt: Fmt) {
    if app.reports.report.active.is_some() {
        return;
    }
    let Some(year) = effective_year(app) else {
        return;
    };
    let austrian = app
        .reports
        .report
        .current()
        .and_then(Load::ready)
        .is_some_and(|d| {
            model::region(d["jurisdictionCode"].as_str().unwrap_or("")) == model::Region::At
        });
    let (kind, args) = request(&app.reports.report, fmt, year, austrian);
    let ui = &mut app.reports.report;
    ui.status = None;
    ui.done = None;
    ui.active = Some(fmt);
    ui.export_year = year;
    ui.export_visit = ui.visit;
    if app.send(kind, args, "reports.report.export").is_none() {
        app.reports.report.active = None;
    }
}

/// The export request for `fmt` over the page's `year`. Year-based exports send the year
/// itself; the core cuts it with the book's tax calendar, as the page's own report does.
fn request(ui: &Ui, fmt: Fmt, year: i32, austrian: bool) -> (&'static str, Value) {
    let picker = ui.wallets.as_ref().is_some_and(|w| w.len() > 1);
    match fmt {
        Fmt::SummaryPdf => {
            let mut a = json!({
                "year": year,
                "include_snapshot": !ui.no_cover,
            });
            if picker {
                a["wallets"] = json!(ui.summary_wallets);
            }
            ("ui.reports.export_summary_pdf", a)
        }
        Fmt::AuditPackage => {
            let i = ui.audit.include;
            let mut a = json!({
                "include_copied_attachments": i[0],
                "include_url_references": i[1],
                "include_journal_state": i[2],
                "include_review_state": i[3],
                "include_edit_history": i[4],
            });
            if ui.audit.case_scope && !ui.audit.case_id.is_empty() {
                a["source_funds_case"] = json!(ui.audit.case_id);
            }
            ("ui.reports.export_audit_package", a)
        }
        Fmt::YearHandoff => ("ui.reports.export_year_handoff", json!({"year": year})),
        Fmt::Csv if austrian => ("ui.reports.export_austrian_e1kv_csv", json!({"year": year})),
        Fmt::Xlsx if austrian => (
            "ui.reports.export_austrian_e1kv_xlsx",
            json!({"year": year}),
        ),
        Fmt::Pdf if austrian => ("ui.reports.export_austrian_e1kv_pdf", json!({"year": year})),
        Fmt::Csv => ("ui.reports.export_csv", json!({})),
        Fmt::Xlsx => ("ui.reports.export_xlsx", json!({})),
        Fmt::Pdf => ("ui.reports.export_pdf", json!({})),
    }
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// `{count} <unit>` in the UI language (`reports:files.written.<unit>`).
fn plural(n: usize, unit: &str) -> String {
    tc(&format!("reports:files.written.{unit}"), n, &[])
}

/// What a written export holds, as the notification says it.
fn detail(p: &Value) -> String {
    let f = p["format"].as_str().unwrap_or("");
    let n = |k: &str| p[k].as_u64().unwrap_or(0) as usize;
    let len = |k: &str| p[k].as_array().map(Vec::len).unwrap_or(0);
    if p["scope"].as_str() == Some("audit_package") {
        format!(
            "{} · {} · {}",
            plural(n("transaction_count"), "transactions"),
            plural(n("evidence_file_count"), "files"),
            plural(n("url_reference_count"), "links")
        )
    } else if f == "pdf" && n("pages") > 0 {
        plural(n("pages"), "pages")
    } else if f == "xlsx" && len("sheets") > 0 {
        plural(len("sheets"), "sheets")
    } else if f == "csv" && len("files") > 0 {
        plural(len("files"), "files")
    } else if (f == "xlsx" || f == "csv") && p["rows"].is_u64() {
        plural(n("rows"), "rows")
    } else {
        t("reports:files.written.fallback")
    }
}

fn on_export(app: &mut App, r: &Reply) {
    let Some(fmt) = app.reports.report.active else {
        return;
    };
    let year = app.reports.report.export_year;
    if let Some(message) = &r.error {
        let e = &r.error_detail;
        let code = e["code"].as_str().unwrap_or("");
        let quarantine = matches!(
            code,
            "transaction_export_quarantined" | "year_handoff_quarantined"
        )
        .then(|| Quarantine {
            count: e["details"]["count"].as_u64().unwrap_or(0) as usize,
            rows: e["details"]["transactions"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        });
        let title = t(if quarantine.is_some() {
            "chrome:notifications.feedback.reportExportBlocked"
        } else {
            "chrome:notifications.feedback.reportExportFailed"
        });
        let message = if message.is_empty() {
            t("chrome:notifications.feedback.reportExportFailed")
        } else {
            message.clone()
        };
        let ui = &mut app.reports.report;
        ui.active = None;
        if ui.same_visit() {
            ui.status = Some(Status {
                year,
                tone: StatusTone::Error,
                message: message.clone(),
                notes: vec![],
                path: None,
                open_path: None,
                quarantine,
                actions: vec![],
            });
        }
        app.toast(
            ToastTone::Error,
            &title,
            Some(message),
            Some("report-export-error"),
        );
        return;
    }
    let path = r.data["file"]
        .as_str()
        .or_else(|| r.data["dir"].as_str())
        .unwrap_or("")
        .to_owned();
    let w = Written {
        fmt,
        year,
        path: path.clone(),
        payload: r.data.clone(),
    };
    if path.is_empty() {
        finish(app, w, Ok(None));
    } else {
        // `saveExportedFileAs`: the desktop asks where to keep a copy (the next poll has a `Tx`).
        app.reports.report.pending_save = Some(w);
    }
}

/// The Save As for a written export: a native Save dialog named after the managed file.
fn ask_save(w: Written, tx: &mut Tx) {
    let source = PathBuf::from(&w.path);
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_owned();
    let mut options = FileDialogOptions {
        title: t("reports:files.saveDialogTitle"),
        file_name: Some(name),
        ..Default::default()
    };
    if source.is_file() {
        options = match source
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("pdf") => options.filter("PDF", &["pdf"]),
            Some("xlsx") => {
                options.filter(t("transactions:dashboard.export.formats.xlsx"), &["xlsx"])
            }
            Some("csv") => options.filter("CSV", &["csv"]),
            _ => options,
        };
    }
    tx.save_file(options, move |a: &mut App, result, _tx| {
        let answer = match result {
            Ok(Some(dest)) => save_copy(a, &source, &dest).map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        finish(a, w.clone(), answer);
    });
}

/// `state_root_for_data_root`: the book's root above its `data` folder.
fn state_root(app: &App) -> Option<PathBuf> {
    let data = app.data_root()?;
    Some(
        if data.file_name().and_then(|n| n.to_str()) == Some("data") {
            data.parent().map(Path::to_path_buf).unwrap_or(data)
        } else {
            data
        },
    )
}

/// A managed-export error in the UI language, with the OS error appended where it has one.
fn err(key: &str, e: impl std::fmt::Display) -> String {
    tr(
        &format!("reports:files.error.{key}"),
        &[("error", &e.to_string())],
    )
}

/// `validated_report_export_target`: an absolute path directly in the book's
/// `exports/reports` folder, naming a PDF/XLSX/CSV file, an Austrian CSV bundle folder or an
/// audit package folder (Reports and the exit-tax handoff share it).
pub(crate) fn managed(app: &App, path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(t("reports:files.error.notAbsolute"));
    }
    let root = state_root(app).ok_or_else(|| t("reports:files.error.noManagedFolder"))?;
    let managed = std::fs::canonicalize(root.join("exports").join("reports"))
        .map_err(|e| err("noManagedFolderDetail", e))?;
    let canonical = std::fs::canonicalize(path).map_err(|e| err("notFound", e))?;
    if canonical.parent() != Some(managed.as_path()) {
        return Err(t("reports:files.error.otherProject"));
    }
    let meta = canonical.metadata().map_err(|e| err("inspect", e))?;
    if !supported_target(&canonical, meta.is_file(), meta.is_dir()) {
        return Err(t("reports:files.error.unsupported"));
    }
    Ok(canonical)
}

/// `is_supported_report_export_target`: a `.pdf`/`.xlsx`/`.csv` file, or a folder named as an
/// Austrian CSV bundle or an audit package.
fn supported_target(path: &Path, is_file: bool, is_dir: bool) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if is_file {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        return matches!(ext.as_deref(), Some("pdf" | "xlsx" | "csv"));
    }
    is_dir
        && ((name.starts_with("kassiber-austrian-e1kv-") && name.contains("-csv-"))
            || name.starts_with("kassiber-audit-package-"))
}

fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists()
        && dest
            .read_dir()
            .map_err(|e| err("inspectDestination", e))?
            .next()
            .is_some()
    {
        return Err(t("reports:files.error.destinationNotEmpty"));
    }
    std::fs::create_dir_all(dest).map_err(|e| err("createDestination", e))?;
    for entry in std::fs::read_dir(src).map_err(|e| err("readSource", e))? {
        let entry = entry.map_err(|e| err("readEntry", e))?;
        let to = dest.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to).map_err(|e| err("saveEntry", e))?;
        }
    }
    Ok(())
}

/// `save_exported_file_as`: copies the managed export to a destination outside the managed
/// folder (a file keeps a `.pdf`/`.xlsx`/`.csv` name; a bundle folder copies whole).
pub(crate) fn save_copy(app: &App, source: &Path, dest: &Path) -> Result<String, String> {
    let source = managed(app, source)?;
    let parent = dest
        .parent()
        .ok_or_else(|| t("reports:files.error.noParent"))?;
    std::fs::create_dir_all(parent).map_err(|e| err("createParent", e))?;
    let parent = std::fs::canonicalize(parent).map_err(|e| err("inspectParent", e))?;
    if source.parent().is_some_and(|m| parent.starts_with(m)) {
        return Err(t("reports:files.error.insideManaged"));
    }
    if source.is_file() {
        let ext = dest
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(ext.as_deref(), Some("pdf" | "xlsx" | "csv")) {
            return Err(t("reports:files.error.extension"));
        }
        std::fs::copy(&source, dest).map_err(|e| err("save", e))?;
    } else {
        copy_dir(&source, dest)?;
    }
    Ok(dest.to_string_lossy().into_owned())
}

/// The export's outcome once the Save As answered (`Ok(None)`: cancelled, the managed file stays).
pub fn finish(app: &mut App, w: Written, answer: Result<Option<String>, String>) {
    let p = &w.payload;
    let filename = p["filename"]
        .as_str()
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| basename(&w.path));
    let filename = if filename.is_empty() {
        "report".to_owned()
    } else {
        filename
    };
    let saved = match answer {
        Ok(saved) => saved.unwrap_or_else(|| w.path.clone()),
        Err(message) => {
            let ui = &mut app.reports.report;
            ui.active = None;
            if ui.same_visit() {
                ui.status = Some(Status {
                    year: w.year,
                    tone: StatusTone::Error,
                    message: message.clone(),
                    notes: vec![],
                    path: Some(w.path.clone()),
                    open_path: Some(w.path.clone()),
                    quarantine: None,
                    actions: vec![],
                });
            }
            app.toast(
                ToastTone::Error,
                &t("chrome:notifications.feedback.reportSaveFailed"),
                Some(message),
                Some("report-export-error"),
            );
            return;
        }
    };
    let warnings: Vec<String> = p["warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let flagged = p["reconciled"].as_bool() == Some(false) || !warnings.is_empty();
    let kept = saved == w.path;
    let ui = &mut app.reports.report;
    ui.active = None;
    let shown = ui.same_visit();
    if shown && (kept || flagged) {
        ui.status = Some(Status {
            year: w.year,
            tone: if flagged {
                StatusTone::Warning
            } else {
                StatusTone::Success
            },
            message: tr(
                if flagged {
                    "reports:files.savedWithNotes"
                } else {
                    "reports:files.saved"
                },
                &[("file", &filename)],
            ),
            notes: if flagged { warnings.clone() } else { vec![] },
            path: Some(saved.clone()).filter(|s| !s.is_empty()),
            open_path: kept.then(|| w.path.clone()).filter(|s| !s.is_empty()),
            quarantine: None,
            actions: if kept {
                crate::toastopen::for_path(&w.path, true)
                    .into_iter()
                    .filter(|a| a.id == "reveal")
                    .collect()
            } else {
                crate::toastopen::for_path(&saved, false)
            },
        });
    }
    if shown {
        ui.done = Some((w.fmt, w.year, app.clock + 5.0));
    }
    // `recordReportExported`: a Recent entry that opens Reports, and the toast.
    let file = {
        let b = basename(&saved);
        if b.is_empty() { filename.clone() } else { b }
    };
    let title = t(if flagged {
        "chrome:notifications.result.reportExported.notesTitle"
    } else {
        "chrome:notifications.result.reportExported.title"
    });
    let at = app.clock;
    app.shell.note(
        &title,
        Some(format!("{file} · {}", detail(p))),
        if flagged {
            ToastTone::Warning
        } else {
            ToastTone::Info
        },
        at,
    );
    // `toastopen`: Open / Show in Finder for the file just written (the managed one when the
    // Save As was cancelled).
    let tone = if flagged {
        ToastTone::Warning
    } else {
        ToastTone::Success
    };
    crate::toastopen::saved_toast(
        app,
        tone,
        &title,
        Some(file),
        Some("report-export"),
        &saved,
        kept,
    );
}

/// `handleOpenExport`: opens a managed export with the system's default app; only a failure
/// says so.
pub fn open_export(app: &mut App, path: &str) {
    // `toastopen`: the managed checks, then the shared runner (never spawns in tooling runs; a
    // failed open shows `reportOpenFailed`).
    crate::toastopen::open_managed(app, path);
}

/// "Run report" (Lightning): contacts the node, so only from that press.
pub fn run_ln(app: &mut App, connection: &str) {
    app.reports.report.ln = Some((connection.to_owned(), Load::Loading));
    app.reports.report.asked.ln.push_back(connection.to_owned());
    app.send(
        "ui.reports.lightning_profitability",
        json!({"connection": connection}),
        "reports.report.ln",
    );
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if matches!(ev.key, rune::Key::Escape) && app.reports.report.popup.is_some() {
        app.reports.report.popup = None;
        return true;
    }
    false
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    if app.reports.report.explain.is_some() {
        return extra::explanation_page(app, tk);
    }
    view::page(app, cx, tk, layer)
}

// ---- Scenes ---------------------------------------------------------------------------------

fn open(a: &mut App, route: &str) {
    a.navigate(route);
    prime(a);
}

/// Runs an export headless and answers the Save As with "cancel" (the managed file stays).
fn scene_export(a: &mut App, fmt: Fmt) {
    open(a, "/reports");
    export(a, fmt);
    super::pump_until(a, |a| {
        a.reports.report.active.is_none() || a.reports.report.pending_save.is_some()
    });
    if let Some(w) = a.reports.report.pending_save.take() {
        finish(a, w, Ok(None));
    }
}

/// SYNTHETIC: rewrites the loaded read in place (the demo books are all generic and clean), so
/// the Austrian and not-ready layouts can be looked at without touching the book.
fn scene_patch(a: &mut App, f: impl FnOnce(&mut Value)) {
    open(a, "/reports");
    let key = a.reports.report.key();
    if let Some(Load::Ready(d)) = a.reports.report.reports.get_mut(&key) {
        f(d);
    }
}

fn scene_austrian(a: &mut App) {
    scene_patch(a, |d| {
        let gain = d["totals"]["gainLossEur"].as_f64().unwrap_or(0.0);
        d["jurisdictionCode"] = json!("AT");
        d["method"] = json!("moving_average_at");
        d["e1kvApplies"] = json!(true);
        d["kennzahlRows"] = json!([
            {"code": "172", "label": "Foreign ongoing crypto income", "form": "E 1kv", "amount": 0.0, "rowCount": 0},
            {"code": "174", "label": "Foreign realized crypto gains", "form": "E 1kv", "amount": gain, "rowCount": 74},
            {"code": "176", "label": "Foreign realized crypto losses", "form": "E 1kv", "amount": 0.0, "rowCount": 0},
            {"code": "801", "label": "Altbestand speculation gains", "form": "E 1", "amount": 0.0, "rowCount": 0},
        ]);
        d["indicativeTax"] = json!({
            "rate": "0.275",
            "estimatedTaxEurCents": (gain * 27.5).round(),
            "progressiveRateAmountsEurCents": {},
            "notes": ["Indicative only: the special rate of 27.5 % on KZ 172 + 174 − 176."],
        });
    });
}

pub const SCENES: &[super::Scene] = &[
    // SYNTHETIC (see `scene_patch`): an Austrian E 1kv year.
    ("reports-reports-at", scene_austrian),
    ("reports-reports-at-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_austrian(a);
    }),
    // SYNTHETIC: journals stale and two rows held, so the readiness tile and "Provisional" show.
    ("reports-reports-not-ready", |a| {
        scene_patch(a, |d| {
            d["status"]["needsJournals"] = json!(true);
            d["status"]["quarantines"] = json!(2);
        })
    }),
    ("reports-reports-formats", |a| {
        open(a, "/reports");
        a.reports.report.popup = Some(Popup::TaxFormats);
    }),
    ("reports-reports", |a| open(a, "/reports")),
    ("reports-reports-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, "/reports");
    }),
    ("reports-reports-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open(a, "/reports");
    }),
    ("reports-reports-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        open(a, "/reports");
    }),
    ("reports-reports-2022", |a| open(a, "/reports?year=2022")),
    ("reports-reports-year", |a| {
        open(a, "/reports");
        a.reports.report.popup = Some(Popup::Year);
    }),
    ("reports-reports-summary", |a| {
        open(a, "/reports");
        a.reports.report.fold = Some(Fold::Summary);
    }),
    ("reports-reports-audit", |a| {
        open(a, "/reports");
        a.reports.report.fold = Some(Fold::Audit);
    }),
    ("reports-reports-audit-scope", |a| {
        open(a, "/reports");
        a.reports.report.fold = Some(Fold::Audit);
        a.reports.report.popup = Some(Popup::AuditScope);
    }),
    ("reports-reports-narrow", |a| open(a, "/reports")),
    // Writes the bookkeeping handoff into the book's exports folder (disposable book only).
    ("reports-reports-exported", |a| {
        scene_export(a, Fmt::YearHandoff)
    }),
    // SYNTHETIC: a quarantine-blocked export, as the daemon reports one (the demo book has none).
    ("reports-reports-blocked", |a| {
        open(a, "/reports");
        let year = effective_year(a).unwrap_or(2025);
        a.reports.report.status = Some(Status {
            year,
            tone: StatusTone::Error,
            message: "Year handoff blocked: 2 transactions are quarantined.".into(),
            notes: vec![],
            path: None,
            open_path: None,
            actions: vec![],
            quarantine: Some(Quarantine {
                count: 2,
                rows: vec![
                    json!({"transaction_id": "5f0db9cd8a1b2c3d4e5f60718293a4b5c6d7e8f9", "wallet": "Regtest Spending Wallet", "occurred_at": "2025-03-02T10:00:00Z", "reason": "missing_price"}),
                    json!({"transaction_id": "ef6b457e9a8b7c6d5e4f30211203f4e5d6c7b8a9", "wallet": "Regtest Treasury", "occurred_at": "2025-04-11T08:15:00Z", "reason": "unpaired_transfer"}),
                ],
            }),
        });
    }),
    // The capital-gain explanation for the current year's first lot.
    ("reports-reports-explain", |a| {
        open(a, "/reports");
        let reference = a
            .reports
            .report
            .current()
            .and_then(Load::ready)
            .and_then(|d| d["lots"].get(0))
            .map(|l| l["explanationReference"].clone());
        if let Some(r) = reference {
            open(
                a,
                &format!("/reports?result={}", model::encode(&r.to_string())),
            );
        }
    }),
];

#[cfg(test)]
mod transient_tests {
    use super::*;

    /// A refresh that fails after a good read shows the failure, not the old figures as current.
    #[test]
    fn a_failed_refresh_replaces_the_shown_report() {
        let mut app = crate::app::tests::quiet_app();
        app.reports
            .report
            .reports
            .insert(2025, Load::Ready(json!({ "year": 2025 })));
        app.reports.report.queue.push_back(Some(2025));
        let failed = Reply {
            kind: "ui.reports.capital_gains".into(),
            data: Value::Null,
            error: Some("journals are stale".into()),
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "reports.report.cg", &failed);
        assert!(matches!(
            app.reports.report.reports.get(&2025),
            Some(Load::Failed(_))
        ));
    }

    /// Another book opens (outside a session start) while the old book's year is being read,
    /// and the page reads the same year of the new one: the old figures never show as the new
    /// book's, and the new read lands.
    #[test]
    fn a_book_switch_drops_the_old_books_report() {
        let mut app = crate::app::tests::quiet_app();
        app.route = crate::app::Route("/reports".into());
        let reply = |data: Value| Reply {
            kind: "ui.reports.capital_gains".into(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        app.reads.insert(
            "ui.profiles.snapshot".into(),
            reply(json!({ "activeProfileId": "b" })),
        );
        app.reports.report.book = Some("a".into());
        app.reports.report.queue.push_back(Some(0));
        step(&mut app);
        on_reply(
            &mut app,
            "reports.report.cg",
            &reply(json!({ "book": "a" })),
        );
        assert!(matches!(
            app.reports.report.reports.get(&0),
            Some(Load::Loading)
        ));
        on_reply(
            &mut app,
            "reports.report.cg",
            &reply(json!({ "book": "b" })),
        );
        match app.reports.report.reports.get(&0) {
            Some(Load::Ready(v)) => assert_eq!(v["book"], "b"),
            _ => panic!("the new book's report lands"),
        }
    }

    /// Profitability run for A, then for B before A answers: A's numbers never show as B's,
    /// and B stays loading until its own reply.
    #[test]
    fn a_profitability_reply_lands_only_on_its_connection() {
        let mut app = crate::app::tests::quiet_app();
        app.reports
            .report
            .asked
            .ln
            .extend(["a".to_owned(), "b".to_owned()]);
        app.reports.report.ln = Some(("b".into(), Load::Loading));
        let reply = |n: u64| Reply {
            kind: "ui.reports.lightning_profitability".into(),
            data: json!({ "revenue_msat": n }),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "reports.report.ln", &reply(1));
        assert!(matches!(app.reports.report.ln, Some((_, Load::Loading))));
        on_reply(&mut app, "reports.report.ln", &reply(2));
        match &app.reports.report.ln {
            Some((c, Load::Ready(v))) => {
                assert_eq!((c.as_str(), &v["revenue_msat"]), ("b", &json!(2)))
            }
            _ => panic!("B's own reply lands"),
        }
        assert!(app.reports.report.asked.ln.is_empty());
    }

    #[test]
    fn leaving_reports_drops_the_export_strip_and_keeps_reads() {
        let mut ui = Ui {
            year: Some(2025),
            ..Ui::default()
        };
        ui.reports.insert(2025, Load::Ready(json!({"year": 2025})));
        ui.wallets = Some(vec![
            WalletChoice {
                id: "a".into(),
                label: "A".into(),
                chain: "bitcoin".into(),
            },
            WalletChoice {
                id: "b".into(),
                label: "B".into(),
                chain: "bitcoin".into(),
            },
        ]);
        ui.summary_wallets = vec!["a".into()];
        ui.status = Some(Status {
            year: 2025,
            tone: StatusTone::Warning,
            message: "x.xlsx saved with notes.".into(),
            notes: vec!["3 rows".into()],
            path: None,
            open_path: None,
            quarantine: None,
            actions: vec![],
        });
        ui.done = Some((Fmt::YearHandoff, 2025, 5.0));
        ui.popup = Some(Popup::Year);
        ui.fold = Some(Fold::Audit);
        ui.no_cover = true;
        ui.audit.include = [false; 5];
        ui.active = Some(Fmt::Pdf);
        ui.export_visit = ui.visit;
        ui.leave();
        assert!(
            ui.status.is_none() && ui.done.is_none() && ui.popup.is_none() && ui.fold.is_none()
        );
        assert!(!ui.no_cover);
        assert_eq!(ui.audit.include, Audit::default().include);
        assert_eq!(ui.summary_wallets, vec!["a".to_owned(), "b".to_owned()]);
        // Cached reads, the year and the running export stay; its outcome no longer shows.
        assert!(matches!(ui.reports.get(&2025), Some(Load::Ready(_))));
        assert_eq!(ui.year, Some(2025));
        assert_eq!(ui.active, Some(Fmt::Pdf));
        assert!(!ui.same_visit());
    }

    #[test]
    fn the_summary_export_sends_the_pages_tax_year_not_utc_bounds() {
        let mut ui = Ui::default();
        let (kind, args) = request(&ui, Fmt::SummaryPdf, 2024, false);
        assert_eq!(kind, "ui.reports.export_summary_pdf");
        assert_eq!(args, json!({"year": 2024, "include_snapshot": true}));
        // A wallet pick and the cover choice ride along; still no hand-built range.
        ui.wallets = Some(vec![
            WalletChoice {
                id: "a".into(),
                label: "A".into(),
                chain: "bitcoin".into(),
            },
            WalletChoice {
                id: "b".into(),
                label: "B".into(),
                chain: "bitcoin".into(),
            },
        ]);
        ui.summary_wallets = vec!["b".into()];
        ui.no_cover = true;
        let (_, args) = request(&ui, Fmt::SummaryPdf, 2025, true);
        assert_eq!(
            args,
            json!({"year": 2025, "include_snapshot": false, "wallets": ["b"]})
        );
    }
}
