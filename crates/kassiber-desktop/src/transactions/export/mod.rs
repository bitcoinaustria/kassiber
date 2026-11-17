//! The export dialog (owner redesign of TransactionsExportDialog.tsx; the Tauri dialog is the
//! reference for what it can do): three short choices on the left (format, period, wallets),
//! a small human preview of the file on the right, then the same write calls the web makes
//! (`ui.transactions.export_xlsx` / `_csv` / `_platform_csv`, `ui.metadata.bip329.export`) and
//! the web's Save As copy of the managed file.

mod strings;
mod summary;
mod view;

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use rune::{FileDialogOptions, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply, ToastTone};
use crate::format;
use crate::i18n::{t, tc, tr};

pub use view::dialog;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Xlsx,
    Csv,
    Labels,
    CoinTracking,
    Blockpit,
}

impl Format {
    pub const ALL: [Format; 5] = [
        Format::Xlsx,
        Format::Csv,
        Format::Labels,
        Format::CoinTracking,
        Format::Blockpit,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Format::Xlsx => "xlsx",
            Format::Csv => "csv",
            Format::Labels => "labels",
            Format::CoinTracking => "cointracking",
            Format::Blockpit => "blockpit",
        }
    }
    pub fn name(self) -> String {
        t(&format!(
            "transactions:dashboard.export.formats.{}",
            self.id()
        ))
    }
    pub fn hint(self) -> String {
        t(&format!(
            "transactions:dashboard.export.formatHints.{}",
            self.id()
        ))
    }
    pub fn badge(self) -> &'static str {
        match self {
            Format::Xlsx => "XLSX",
            Format::Labels => "JSONL",
            _ => "CSV",
        }
    }
    pub fn dated(self) -> bool {
        self != Format::Labels
    }
    pub fn tool(self) -> bool {
        matches!(self, Format::CoinTracking | Format::Blockpit)
    }
    /// The Save As filter (`exportFormats.ts` `filter`).
    fn filter(self) -> (&'static str, &'static str) {
        match self {
            Format::Xlsx => ("Excel workbook", "xlsx"),
            Format::Labels => ("BIP 329 labels", "jsonl"),
            _ => ("CSV", "csv"),
        }
    }
}

pub const PRESETS: [&str; 12] = [
    "all",
    "today",
    "yesterday",
    "this_week",
    "last_week",
    "this_month",
    "last_month",
    "this_quarter",
    "last_quarter",
    "this_year",
    "last_year",
    "custom",
];

#[derive(Clone, Debug)]
pub struct WalletOpt {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub chain: String,
    pub count: usize,
}

/// A written export waiting for its Save As (the next poll has the `Tx` the dialog needs).
#[derive(Clone, Debug)]
pub struct Written {
    format: Format,
    path: String,
    payload: Value,
    file_name: String,
}

#[derive(Default)]
pub struct ExportUi {
    pub open: bool,
    pub busy: bool,
    /// The running export's format and previewed file name, fixed when it was sent: the
    /// dialog's choice may change before it finishes.
    pub sent: Option<(Format, String)>,
    pub format: Format,
    pub preset: usize,
    pub from: TextEdit,
    pub to: TextEdit,
    /// None = the book's timezone.
    pub timezone: Option<String>,
    pub viewer_tz: Option<String>,
    pub tz_open: bool,
    pub wallets: Option<Vec<WalletOpt>>,
    pub excluded: HashSet<String>,
    pub wallet_filter: TextEdit,
    /// The wallet list is open (Edit); otherwise the section is one summary line.
    pub editing_wallets: bool,
    /// The raw preview of every column instead of the summary.
    pub all_columns: bool,
    pub preview: Option<Value>,
    pub preview_error: Option<Value>,
    pub preview_key: String,
    pub settled_key: String,
    pub queue: VecDeque<String>,
    pub due: f64,
    pub(crate) pending_save: Option<Written>,
}

pub fn open(app: &mut App) {
    let e = &mut app.txs.export;
    e.open = true;
    e.busy = false;
    e.editing_wallets = false;
    e.tz_open = false;
    if e.viewer_tz.is_none() {
        // The viewer's IANA zone, as `Intl.DateTimeFormat().resolvedOptions().timeZone`.
        e.viewer_tz = std::env::var("TZ")
            .ok()
            .filter(|z| z.contains('/'))
            .or_else(|| {
                std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                    p.to_str()
                        .and_then(|s| s.split("zoneinfo/").nth(1).map(str::to_owned))
                })
            });
    }
    if e.wallets.is_none() {
        app.send("ui.wallets.list", Value::Null, "txs.export.wallets");
    }
}

/// Closes the dialog unless an export is being written (`onOpenChange` ignores it then).
pub fn close(a: &mut App) {
    if !a.txs.export.busy {
        a.txs.export.open = false;
        a.txs.export.tz_open = false;
        a.txs.popup = None;
    }
}

fn book_tz(app: &App) -> String {
    app.txs
        .dash
        .as_ref()
        .map(|d| d.timezone.clone())
        .unwrap_or_else(|| "UTC".into())
}

fn wallets_arg(e: &ExportUi) -> Option<Vec<String>> {
    let w = e.wallets.as_ref()?;
    if e.excluded.is_empty() {
        None
    } else {
        Some(
            w.iter()
                .filter(|x| !e.excluded.contains(&x.id))
                .map(|x| x.id.clone())
                .collect(),
        )
    }
}

pub fn selected_count(e: &ExportUi) -> usize {
    e.wallets
        .as_ref()
        .map(|w| w.iter().filter(|x| !e.excluded.contains(&x.id)).count())
        .unwrap_or(0)
}

fn valid_day(s: &str) -> bool {
    s.len() == 10 && format::Day::parse(s).is_some()
}

/// The custom range typed so far, when both ends are dates and in order.
pub fn custom_range(e: &ExportUi) -> Option<(String, String)> {
    let (f, to) = (e.from.text.trim(), e.to.text.trim());
    (valid_day(f) && valid_day(to) && f <= to).then(|| (f.to_owned(), to.to_owned()))
}

pub fn is_custom(e: &ExportUi) -> bool {
    PRESETS[e.preset] == "custom"
}

/// `periodArgs`: nothing for all time, `range` for a preset, `from`/`to` for a custom range
/// (none until it is complete), and a timezone other than the book's.
fn period_args(e: &ExportUi, m: &mut serde_json::Map<String, Value>) {
    match PRESETS[e.preset] {
        "all" => return,
        "custom" => {
            let Some((f, t)) = custom_range(e) else {
                return;
            };
            m.insert("from".into(), json!(f));
            m.insert("to".into(), json!(t));
        }
        p => {
            m.insert("range".into(), json!(p));
        }
    }
    if let Some(tz) = &e.timezone {
        m.insert("timezone".into(), json!(tz));
    }
}

fn preview_args(e: &ExportUi) -> Option<Value> {
    if selected_count(e) == 0 {
        return None;
    }
    let mut m = serde_json::Map::new();
    if e.format == Format::Labels {
        m.insert("export".into(), json!("labels"));
    } else {
        m.insert("export".into(), json!("transactions"));
        m.insert("format".into(), json!(e.format.id()));
        period_args(e, &mut m);
    }
    if let Some(w) = wallets_arg(e) {
        m.insert("wallets".into(), json!(w));
    }
    Some(Value::Object(m))
}

/// The preview is behind the choices (debounce pending or a request in flight).
pub fn updating(e: &ExportUi) -> bool {
    e.preview_key != e.settled_key || !e.queue.is_empty()
}

/// Debounced (300 ms) preview loading and the Save As of a finished export; from the screen's
/// poll.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    // The Save As runs even if the dialog closed meanwhile, as the web's promise does.
    if let Some(w) = app.txs.export.pending_save.take() {
        ask_save(w, tx);
        return true;
    }
    let generation = app.book_generation;
    let e = &mut app.txs.export;
    if !e.open || e.wallets.is_none() {
        return false;
    }
    let Some(args) = preview_args(e) else {
        return false;
    };
    // The book refresh generation is part of the key: after an edit or exclusion the same
    // choices preview again, and an answer from before the refresh no longer matches.
    let key = preview_key(&args, generation);
    if key != e.preview_key {
        e.preview_key = key;
        e.due = app.clock + 0.3;
        // A different choice is a different file: forget the previous export's block.
        if e.preview_error
            .as_ref()
            .is_some_and(|x| x["from_write"].as_bool() == Some(true))
        {
            e.preview_error = None;
        }
        tx.wake_after(std::time::Duration::from_millis(310));
        return true;
    }
    if e.settled_key != e.preview_key && !e.queue.contains(&e.preview_key) && app.clock >= e.due {
        e.queue.push_back(e.preview_key.clone());
        app.send("ui.exports.preview", args, "txs.export.preview");
        return true;
    }
    if app.clock < e.due {
        tx.wake_after(std::time::Duration::from_secs_f64(
            (e.due - app.clock).max(0.01),
        ));
    }
    false
}

/// A preview's key: its arguments in the book generation they are read in.
fn preview_key(args: &Value, generation: u64) -> String {
    format!("{args}#{generation}")
}

/// `quarantineExportBlock`: the daemon's refusal to write a file with quarantined rows.
pub fn blocked(err: &Value) -> bool {
    matches!(
        err["code"].as_str(),
        Some("transaction_export_quarantined" | "year_handoff_quarantined")
    )
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "txs.export.wallets" => {
            let list = r.data["wallets"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|w| WalletOpt {
                            id: w["id"].as_str().unwrap_or("").to_owned(),
                            label: w["label"].as_str().unwrap_or("").to_owned(),
                            kind: w["kind"].as_str().unwrap_or("").to_owned(),
                            chain: w["chain"].as_str().unwrap_or("").to_owned(),
                            count: w["transaction_count"].as_u64().unwrap_or(0) as usize,
                        })
                        .collect()
                })
                .unwrap_or_default();
            app.txs.export.wallets = Some(list);
        }
        "txs.export.preview" => {
            let e = &mut app.txs.export;
            let key = e.queue.pop_front().unwrap_or_default();
            if key != e.preview_key {
                return;
            }
            e.settled_key = key;
            if r.error.is_some() {
                e.preview_error = Some(r.error_detail.clone());
            } else {
                e.preview_error = None;
                e.preview = Some(r.data.clone());
            }
        }
        "txs.export.write" => {
            app.txs.export.busy = false;
            let sent = app.txs.export.sent.take();
            match &r.error {
                Some(msg) => {
                    if blocked(&r.error_detail) {
                        let mut err = r.error_detail.clone();
                        err["from_write"] = json!(true);
                        app.txs.export.open = true;
                        app.txs.export.preview_error = Some(err);
                    } else {
                        app.toast(
                            ToastTone::Error,
                            &t("transactions:dashboard.export.failed"),
                            Some(msg.clone()),
                            Some("txs-export"),
                        );
                    }
                }
                None => {
                    let e = &mut app.txs.export;
                    let path = r.data["file"]
                        .as_str()
                        .or_else(|| r.data["path"].as_str())
                        .unwrap_or("")
                        .to_owned();
                    let (format, file_name) = sent.unwrap_or_else(|| {
                        let name = e.preview.as_ref().and_then(|p| p["file_name"].as_str());
                        (e.format, name.unwrap_or("").to_owned())
                    });
                    let w = Written {
                        format,
                        path,
                        payload: r.data.clone(),
                        file_name,
                    };
                    e.open = false;
                    e.tz_open = false;
                    app.txs.popup = None;
                    // `saveDaemonExport`: a managed file the desktop can copy gets a Save As;
                    // the labels' `.jsonl` isn't one the copy accepts, so it stays put.
                    if w.path.is_empty() || w.format == Format::Labels {
                        finish(app, w, Ok(None));
                    } else {
                        app.txs.export.pending_save = Some(w);
                    }
                }
            }
        }
        _ => {}
    }
}

fn ask_save(w: Written, tx: &mut Tx) {
    let source = PathBuf::from(&w.path);
    let name = if w.file_name.is_empty() {
        source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned()
    } else {
        w.file_name.clone()
    };
    let (label, ext) = w.format.filter();
    let title = t(if w.format == Format::Labels {
        "transactions:dashboard.export.labels.saveTitle"
    } else {
        "transactions:dashboard.export.saveTitle"
    });
    let options = FileDialogOptions {
        title,
        file_name: Some(name),
        ..Default::default()
    }
    .filter(label, &[ext]);
    tx.save_file(options, move |a: &mut App, result, _tx| {
        let answer = match result {
            Ok(Some(dest)) => crate::reports::report::save_copy(a, &source, &dest).map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        finish(a, w.clone(), answer);
    });
}

/// The web's `finish`: the toast naming where the file went (or how many rows it holds when it
/// stayed in the managed folder), plus any warnings.
fn finish(app: &mut App, w: Written, answer: Result<Option<String>, String>) {
    let saved = match answer {
        Ok(saved) => saved,
        Err(message) => {
            app.toast(
                ToastTone::Error,
                &t("transactions:dashboard.export.failed"),
                Some(message),
                Some("txs-export"),
            );
            return;
        }
    };
    let labels = w.format == Format::Labels;
    let warnings: Vec<String> = w.payload["warnings"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let title = if labels {
        t("transactions:dashboard.export.labels.done")
    } else if warnings.is_empty() {
        t("transactions:dashboard.export.done")
    } else {
        t("transactions:dashboard.export.doneWithWarnings")
    };
    let base = |p: &str| {
        p.rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or(p)
            .to_owned()
    };
    let mut body = match &saved {
        Some(dest) => tr(
            "transactions:dashboard.export.savedTo",
            &[("name", &base(dest))],
        ),
        None if labels => base(&w.path),
        None => tc(
            "transactions:dashboard.export.rows",
            w.payload["rows"].as_u64().unwrap_or(0) as usize,
            &[],
        ),
    };
    for x in &warnings {
        body.push('\n');
        body.push_str(x);
    }
    let tone = if warnings.is_empty() {
        ToastTone::Success
    } else {
        ToastTone::Warning
    };
    // `toastopen`: Open / Show in Finder for the saved copy, or the managed file when the Save
    // As was cancelled.
    let (path, managed) = match &saved {
        Some(dest) => (dest.as_str(), false),
        None => (w.path.as_str(), true),
    };
    crate::toastopen::saved_toast(
        app,
        tone,
        &title,
        Some(body),
        Some("txs-export"),
        path,
        managed,
    );
}

fn submit(app: &mut App) {
    let e = &app.txs.export;
    let mut m = serde_json::Map::new();
    if let Some(w) = wallets_arg(e) {
        m.insert("wallets".into(), json!(w));
    }
    // Write the exact days the preview resolved, so a relative preset can't drift past midnight.
    if e.format.dated() && PRESETS[e.preset] != "all" {
        match e
            .preview
            .as_ref()
            .and_then(|p| p.get("range"))
            .filter(|r| r.is_object())
        {
            Some(r) => {
                m.insert("from".into(), r["first_day"].clone());
                m.insert("to".into(), r["last_day"].clone());
                if let Some(tz) = r["timezone"].as_str() {
                    m.insert("timezone".into(), json!(tz));
                }
            }
            None => period_args(e, &mut m),
        }
    }
    let (kind, args) = match e.format {
        Format::Labels => {
            m.insert("mode".into(), json!("all"));
            ("ui.metadata.bip329.export", m)
        }
        Format::CoinTracking | Format::Blockpit => {
            m.insert("platform".into(), json!(e.format.id()));
            ("ui.transactions.export_platform_csv", m)
        }
        Format::Xlsx => ("ui.transactions.export_xlsx", m),
        Format::Csv => ("ui.transactions.export_csv", m),
    };
    let file_name = e
        .preview
        .as_ref()
        .and_then(|p| p["file_name"].as_str())
        .unwrap_or("")
        .to_owned();
    app.txs.export.sent = Some((e.format, file_name));
    app.txs.export.busy = true;
    if app.txs.export.preview_error.as_ref().is_some_and(blocked) {
        app.txs.export.preview_error = None;
    }
    app.send(kind, Value::Object(args), "txs.export.write");
}

/// Headless scenes: open the dialog and wait for the wallets and the first preview.
pub fn prime(app: &mut App) {
    open(app);
    settle(app);
}

fn pump(app: &mut App, done: fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !done(app) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.pump();
    }
}

/// Loads the preview for the current choices at once (no debounce), as a scene needs.
fn settle(app: &mut App) {
    pump(app, |a| a.txs.export.wallets.is_some());
    if let Some(args) = preview_args(&app.txs.export) {
        let key = preview_key(&args, app.book_generation);
        app.txs.export.preview_key = key.clone();
        app.txs.export.queue.push_back(key);
        app.send("ui.exports.preview", args, "txs.export.preview");
        pump(app, |a| a.txs.export.queue.is_empty());
    }
}

fn scene(a: &mut App, setup: fn(&mut App)) {
    a.navigate("/transactions");
    super::prime(a, false);
    open(a);
    pump(a, |a| a.txs.export.wallets.is_some());
    setup(a);
    settle(a);
}

/// SYNTHETIC: a quarantine-blocked export, as the daemon reports one (the demo book has none).
fn blocked_error() -> Value {
    json!({
        "code": "transaction_export_quarantined",
        "message": "2 quarantined transactions block this export.",
        "details": {"count": 2, "transactions": [
            {"transaction_id": "5f0db9cd8a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4f5061728394a5b", "wallet": "Regtest Spending Wallet", "occurred_at": "2025-03-02T10:00:00Z", "reason": "missing_price"},
            {"transaction_id": "ef6b457e9a8b7c6d5e4f30211203f4e5d6c7b8a9f0e1d2c3b4a5968778695a4b", "wallet": "Regtest Treasury", "occurred_at": "2025-04-11T08:15:00Z", "reason": "unpaired_transfer"}
        ]}
    })
}

pub const SCENES: &[crate::app::Scene] = &[
    ("transactions-export-csv", |a| {
        scene(a, |a| a.txs.export.format = Format::Csv)
    }),
    ("transactions-export-labels", |a| {
        scene(a, |a| a.txs.export.format = Format::Labels)
    }),
    ("transactions-export-cointracking", |a| {
        scene(a, |a| a.txs.export.format = Format::CoinTracking)
    }),
    ("transactions-export-blockpit", |a| {
        scene(a, |a| a.txs.export.format = Format::Blockpit)
    }),
    ("transactions-export-format-open", |a| {
        scene(a, |_| {});
        a.txs.popup = Some(super::Popup::ExportFormat);
    }),
    ("transactions-export-period-open", |a| {
        scene(a, |_| {});
        a.txs.popup = Some(super::Popup::ExportPeriod);
    }),
    ("transactions-export-custom", |a| {
        scene(a, |a| {
            a.txs.export.preset = PRESETS.iter().position(|p| *p == "custom").unwrap_or(0);
            a.txs.export.from = TextEdit::new("2024-01-01");
            a.txs.export.to = TextEdit::new("2024-12-31");
        })
    }),
    ("transactions-export-tz-open", |a| {
        scene(a, |a| {
            a.txs.export.preset = PRESETS.iter().position(|p| *p == "last_year").unwrap_or(0)
        });
        a.txs.export.tz_open = true;
    }),
    ("transactions-export-wallets", |a| {
        scene(a, |a| a.txs.export.editing_wallets = true)
    }),
    ("transactions-export-some-wallets", |a| {
        scene(a, |a| {
            let ids: Vec<String> = a
                .txs
                .export
                .wallets
                .iter()
                .flatten()
                .skip(3)
                .map(|w| w.id.clone())
                .collect();
            a.txs.export.excluded.extend(ids);
        })
    }),
    ("transactions-export-all-columns", |a| {
        scene(a, |a| a.txs.export.all_columns = true)
    }),
    ("transactions-export-blocked", |a| {
        scene(a, |_| {});
        a.txs.export.preview_error = Some(blocked_error());
    }),
    // Writes the export into the book's managed exports folder (disposable book only); the
    // Save As is skipped, as a cancelled one leaves the file there.
    ("transactions-export-written", |a| {
        scene(a, |a| a.txs.export.format = Format::Csv);
        submit(a);
        pump(a, |a| !a.txs.export.busy);
        if let Some(w) = a.txs.export.pending_save.take() {
            finish(a, w, Ok(None));
        }
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_range_needs_two_ordered_days() {
        let mut e = ExportUi {
            preset: 11,
            ..Default::default()
        };
        e.from = TextEdit::new("2024-01-01");
        e.to = TextEdit::new("2024-12-3");
        assert_eq!(custom_range(&e), None);
        e.to = TextEdit::new("2023-12-31");
        assert_eq!(custom_range(&e), None);
        e.to = TextEdit::new("2024-12-31");
        assert_eq!(
            custom_range(&e),
            Some(("2024-01-01".into(), "2024-12-31".into()))
        );
        let mut m = serde_json::Map::new();
        e.timezone = Some("UTC".into());
        period_args(&e, &mut m);
        assert_eq!(
            Value::Object(m),
            json!({"from": "2024-01-01", "to": "2024-12-31", "timezone": "UTC"})
        );
    }

    #[test]
    fn preview_args_follow_the_format() {
        let mut e = ExportUi {
            wallets: Some(vec![WalletOpt {
                id: "w1".into(),
                label: "A".into(),
                kind: String::new(),
                chain: String::new(),
                count: 1,
            }]),
            ..Default::default()
        };
        assert_eq!(
            preview_args(&e),
            Some(json!({"export": "transactions", "format": "xlsx"}))
        );
        e.format = Format::Labels;
        e.preset = 9;
        assert_eq!(preview_args(&e), Some(json!({"export": "labels"})));
        e.excluded.insert("w1".into());
        assert_eq!(preview_args(&e), None);
    }
}

#[cfg(test)]
mod preview_key_tests {
    /// The same choices after a book refresh are a different preview.
    #[test]
    fn a_refresh_changes_the_preview_key() {
        let args = serde_json::json!({"format": "csv"});
        assert_ne!(super::preview_key(&args, 1), super::preview_key(&args, 2));
        assert_eq!(super::preview_key(&args, 2), super::preview_key(&args, 2));
    }
}
