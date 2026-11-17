//! Exit tax (routes/ExitTax.tsx): the Wegzugsbesteuerung "what if I left now" estimate. The
//! departure date (a month-grid `DatePicker`) and destination class pick the
//! `ui.reports.exit_tax_preview` read; the body shows readiness, the headline liability, the
//! collection timing, the deemed-disposal lots, wallet holdings, the Steuerberater handoff
//! (PDF / XLSX export, then the native Save As), the FMV sources (ExitTaxFmvCard.tsx) and the
//! assumptions. The screen does no tax math. Its copy lives under `overview:exitTax`; the
//! assumptions and review gate are the daemon's own (English) text.

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use rune::{Color, Cx, El, FileDialogOptions, KeyEvent, Tx, col, el, row, text};
use rune_kit::Placement;
use serde_json::{Value, json};

use super::common::{self as c, Col, sens, thead, tnum, trow};
use crate::app::{App, Load, Reply};
use crate::i18n::{Lang, lang, t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Variant};
use crate::ui::{ic, surface};

/// The destination classes the preview takes, with their label keys.
const DESTINATIONS: [(&str, &str); 2] = [
    ("eu_eea", "overview:exitTax.destination.euEea"),
    ("third_country", "overview:exitTax.destination.thirdCountry"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    Date,
    Destination,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tone {
    Success,
    Error,
}

/// `ExitTaxExportStatus`.
#[derive(Clone, Debug)]
pub struct ExportStatus {
    pub tone: Tone,
    pub message: String,
    pub path: Option<String>,
    /// `toastopen`: Show in Finder for the file the message names (beside the card's Open).
    pub actions: Vec<crate::toastopen::ToastAction>,
}

pub struct ExitUi {
    /// The picked departure day (`yyyy-mm-dd`); empty = the core's current civil day.
    pub date: String,
    pub destination: String,
    /// Preview reads by their args (`JSON`), as the query cache keys them.
    pub cache: HashMap<String, Load<Value>>,
    /// Keys whose cached answer is stale (re-read, still shown meanwhile).
    stale: Vec<String>,
    /// Preview requests in flight, in send order (the daemon answers in order).
    queue: VecDeque<String>,
    pub popup: Option<Popup>,
    /// The calendar's month on screen: (year, month 0–11).
    pub month: (i32, u32),
    pub active_export: Option<&'static str>,
    pub status: Option<ExportStatus>,
    /// A written export waiting for its Save As dialog: (format, managed path).
    pending_save: Option<(&'static str, String)>,
}

impl Default for ExitUi {
    fn default() -> Self {
        ExitUi {
            date: String::new(),
            destination: "eu_eea".into(),
            cache: HashMap::new(),
            stale: vec![],
            queue: VecDeque::new(),
            popup: None,
            month: (1970, 0),
            active_export: None,
            status: None,
            pending_save: None,
        }
    }
}

impl ExitUi {
    /// Leaving `/exit-tax`: the departure day, destination, calendar, open menus and the export
    /// strip are `ExitTax`'s `useState`s. The preview cache and a running export stay.
    pub fn leave(&mut self) {
        let k = std::mem::take(self);
        *self = ExitUi {
            cache: k.cache,
            stale: k.stale,
            queue: k.queue,
            active_export: k.active_export,
            pending_save: k.pending_save,
            ..ExitUi::default()
        };
    }

    pub fn invalidate(&mut self) {
        self.cache.retain(|_, v| matches!(v, Load::Ready(_)));
        self.stale = self.cache.keys().cloned().collect();
    }

    fn today_key(&self) -> String {
        json!({"destination": self.destination}).to_string()
    }

    fn preview_key(&self) -> String {
        if self.date.is_empty() {
            self.today_key()
        } else {
            json!({"departure_date": self.date, "destination": self.destination}).to_string()
        }
    }

    fn report(&self) -> Option<&Value> {
        self.cache.get(&self.preview_key()).and_then(Load::ready)
    }
}

// ---- Loading ------------------------------------------------------------------------------

fn want(app: &mut App, key: String) -> bool {
    let ui = &mut app.reports.ledger.exit;
    let fresh = !ui.cache.contains_key(&key);
    let stale = ui.stale.contains(&key);
    if (!fresh && !stale) || ui.queue.contains(&key) {
        return false;
    }
    ui.stale.retain(|k| k != &key);
    if fresh {
        ui.cache.insert(key.clone(), Load::Loading);
    }
    ui.queue.push_back(key.clone());
    let args: Value = serde_json::from_str(&key).unwrap_or(Value::Null);
    app.send(
        "ui.reports.exit_tax_preview",
        args,
        "reports.ledger.exit.preview",
    );
    true
}

fn step(app: &mut App) -> bool {
    let today = app.reports.ledger.exit.today_key();
    let preview = app.reports.ledger.exit.preview_key();
    want(app, today) | want(app, preview)
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let changed = step(app);
    if let Some((fmt, path)) = app.reports.ledger.exit.pending_save.take() {
        ask_save(app, fmt, path, tx);
    }
    changed
}

pub fn prime(app: &mut App) {
    step(app);
    crate::reports::pump_until(app, |a| a.reports.ledger.exit.queue.is_empty());
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "reports.ledger.exit.preview" => {
            let ui = &mut app.reports.ledger.exit;
            let Some(key) = ui.queue.pop_front() else {
                return;
            };
            let load = match &r.error {
                Some(e) => Load::Failed(e.clone()),
                None if r.data.is_object() => Load::Ready(r.data.clone()),
                // Empty: the view says "Unknown error." in the reader's language.
                None => Load::Failed(String::new()),
            };
            // A failed re-read of a shown answer keeps the answer, as React Query does.
            if !(matches!(load, Load::Failed(_))
                && matches!(ui.cache.get(&key), Some(Load::Ready(_))))
            {
                ui.cache.insert(key, load);
            }
        }
        "reports.ledger.exit.export" => {
            let ui = &mut app.reports.ledger.exit;
            let fmt = ui.active_export.unwrap_or("pdf");
            match &r.error {
                Some(e) => {
                    ui.active_export = None;
                    ui.status = Some(ExportStatus {
                        tone: Tone::Error,
                        message: if e.is_empty() {
                            t("overview:exitTax.exportFailed")
                        } else {
                            e.clone()
                        },
                        path: None,
                        actions: vec![],
                    });
                }
                None => {
                    let path = r.data["file"].as_str().unwrap_or("").to_owned();
                    if path.is_empty() {
                        saved(app, fmt, path, Ok(None));
                    } else {
                        // `saveExportedFileAs`: the desktop asks where to keep a copy.
                        ui.pending_save = Some((fmt, path));
                    }
                }
            }
        }
        _ => {}
    }
}

/// `handleExport`: writes the handoff into the book's exports folder.
fn export(app: &mut App, fmt: &'static str) {
    if app.reports.ledger.exit.active_export.is_some() {
        return;
    }
    let resolved = resolved_date(&app.reports.ledger.exit);
    let ui = &mut app.reports.ledger.exit;
    ui.status = None;
    ui.active_export = Some(fmt);
    let kind = if fmt == "pdf" {
        "ui.reports.export_exit_tax_pdf"
    } else {
        "ui.reports.export_exit_tax_xlsx"
    };
    let dest = ui.destination.clone();
    if app
        .send(
            kind,
            json!({"departure_date": resolved, "destination": dest}),
            "reports.ledger.exit.export",
        )
        .is_none()
    {
        app.reports.ledger.exit.active_export = None;
    }
}

/// The Save As for a written export (`save_exported_file_as`): the source must pass the managed
/// export checks Reports use (`validated_report_export_target`) before the native Save dialog
/// opens; the chosen copy is written outside the managed folder.
fn ask_save(app: &mut App, fmt: &'static str, path: String, tx: &mut Tx) {
    let source = match crate::reports::report::managed(app, Path::new(&path)) {
        Ok(p) => p,
        Err(e) => return saved(app, fmt, path, Err(e)),
    };
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
    options = match source
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => options.filter("PDF", &["pdf"]),
        Some("xlsx") => options.filter(t("transactions:dashboard.export.formats.xlsx"), &["xlsx"]),
        Some("csv") => options.filter("CSV", &["csv"]),
        _ => options,
    };
    tx.save_file(options, move |a: &mut App, result, _tx| {
        let answer = match result {
            // Checked again at copy time: the folder may have changed while the dialog was up.
            Ok(Some(dest)) => {
                crate::reports::report::save_copy(a, Path::new(&path), &dest).map(Some)
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e.to_string()),
        };
        saved(a, fmt, path, answer);
    });
}

/// `openExportedFile` (`open_exported_file`): only a managed export of the open book opens,
/// with the system's default app; a refusal or failure shows as the Reports one does.
fn open_export(app: &mut App, path: &str) {
    crate::reports::report::open_export(app, path);
}

/// The export's outcome once the Save As answered (`None` = cancelled: the managed file stays).
fn saved(app: &mut App, fmt: &str, export_path: String, answer: Result<Option<String>, String>) {
    let ui = &mut app.reports.ledger.exit;
    ui.active_export = None;
    ui.status = Some(match answer {
        Ok(saved) => {
            let saved = saved.unwrap_or_else(|| export_path.clone());
            let kept = saved == export_path;
            let actions = crate::toastopen::for_path(&saved, kept)
                .into_iter()
                .filter(|a| a.id == "reveal")
                .collect();
            ExportStatus {
                tone: Tone::Success,
                message: tr(
                    "overview:exitTax.handoff.saved",
                    &[("format", &fmt.to_uppercase()), ("file", basename(&saved))],
                ),
                path: Some(export_path).filter(|p| !p.is_empty()),
                actions,
            }
        }
        Err(message) => ExportStatus {
            tone: Tone::Error,
            message,
            path: Some(export_path),
            actions: vec![],
        },
    });
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

fn resolved_date(ui: &ExitUi) -> String {
    if !ui.date.is_empty() {
        return ui.date.clone();
    }
    ui.report()
        .and_then(|r| r["departureDate"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if matches!(ev.key, rune::Key::Escape) && app.reports.ledger.exit.popup.is_some() {
        app.reports.ledger.exit.popup = None;
        return true;
    }
    false
}

// ---- Calendar -----------------------------------------------------------------------------

/// `parseISO`: (year, month 0–11, day); today (UTC) when it doesn't parse.
fn parse_iso(v: &str) -> (i32, u32, u32) {
    let b = v.as_bytes();
    if b.len() >= 10
        && b[4] == b'-'
        && b[7] == b'-'
        && let (Ok(y), Ok(m), Ok(d)) = (
            v[0..4].parse::<i32>(),
            v[5..7].parse::<u32>(),
            v[8..10].parse::<u32>(),
        )
        && (1..=12).contains(&m)
    {
        return (y, m - 1, d);
    }
    let days = (crate::format::now_secs() / 86_400.0).floor() as i64;
    let (y, m, d) = civil(days);
    (y, m - 1, d)
}

/// Days since 1970-01-01 → (year, month 1–12, day) (Hinnant's `civil_from_days`).
fn civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

/// (year, month 1–12, day) → days since 1970-01-01.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i32, m0: u32) -> u32 {
    let (ny, nm) = if m0 == 11 { (y + 1, 1) } else { (y, m0 + 2) };
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m0 + 1, 1)) as u32
}

/// Monday-first weekday of the month's first day (0 = Monday).
fn first_weekday(y: i32, m0: u32) -> u32 {
    // 1970-01-01 was a Thursday (Monday-first index 3).
    ((days_from_civil(y, m0 + 1, 1) + 3).rem_euclid(7)) as u32
}

/// The calendar's month names and Monday-first weekday initials (`Intl` in the web's UI
/// locale: en-US, de-AT with `Jänner`).
fn months(l: Lang) -> [&'static str; 12] {
    match l {
        Lang::En => [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        Lang::De => [
            "Jänner",
            "Februar",
            "März",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ],
    }
}

fn weekdays(l: Lang) -> [&'static str; 7] {
    match l {
        Lang::En => ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"],
        Lang::De => ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
    }
}

/// `formatDisplay`: `Intl.DateTimeFormat(uiLocale, {day: "2-digit", month: "short", year:
/// "numeric"})`, en-US `Nov 15, 2026`, de-AT `15. Nov. 2026`.
pub fn format_display(v: &str, l: Lang) -> String {
    if v.is_empty() {
        return t("overview:exitTax.pickDate");
    }
    let (y, m, d) = parse_iso(v);
    match l {
        Lang::En => format!(
            "{} {d:02}, {y}",
            [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
            ][m as usize]
        ),
        Lang::De => format!(
            "{d:02}. {} {y}",
            [
                "Jän.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.",
                "Nov.", "Dez."
            ][m as usize]
        ),
    }
}

fn open_picker(app: &mut App) {
    let ui = &mut app.reports.ledger.exit;
    if ui.popup == Some(Popup::Date) {
        ui.popup = None;
        return;
    }
    let (y, m, _) = parse_iso(&resolved_date(ui));
    ui.month = (y, m);
    ui.popup = Some(Popup::Date);
}

fn step_month(app: &mut App, delta: i32) {
    let (y, m) = app.reports.ledger.exit.month;
    let total = y * 12 + m as i32 + delta;
    app.reports.ledger.exit.month = (total.div_euclid(12), total.rem_euclid(12) as u32);
}

fn calendar(app: &App, tk: &Tk) -> El {
    let ui = &app.reports.ledger.exit;
    let (vy, vm) = ui.month;
    let selected = parse_iso(&resolved_date(ui));
    let today_src = ui
        .cache
        .get(&ui.today_key())
        .and_then(Load::ready)
        .and_then(|r| r["departureDate"].as_str())
        .unwrap_or("")
        .to_owned();
    let today = parse_iso(&today_src);
    let nav = |key: &str, glyph: &'static rune::Icon, label: &str, delta: i32| {
        w::icon_button(
            tk,
            key.to_owned(),
            Variant::Ghost,
            tk.r(2.0),
            glyph,
            tk.r(1.0),
            label.to_owned(),
        )
        .on_press(move |a: &mut App| step_month(a, delta))
    };
    let cell_w = tk.r(2.0);
    let gap = tk.r(0.25);
    let grid_w = cell_w * 7.0 + gap * 6.0;
    let header = row()
        .w(grid_w)
        .pb(tk.r(0.5))
        .center_y()
        .justify_between()
        .child(nav(
            "et-cal-prev",
            &I::CHEVRON_LEFT,
            &t("overview:exitTax.calendar.previousMonth"),
            -1,
        ))
        .child(
            text(format!("{} {vy}", months(lang())[vm as usize]))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(nav(
            "et-cal-next",
            &I::CHEVRON_RIGHT,
            &t("overview:exitTax.calendar.nextMonth"),
            1,
        ));
    let weekdays = row().w(grid_w).gap(gap).children(weekdays(lang()).map(|d| {
        row()
            .w(cell_w)
            .py(tk.r(0.25))
            .center()
            .child(text(d).font(tk.txs(500.0)).color(tk.muted_fg))
    }));
    let first = first_weekday(vy, vm);
    let n = days_in_month(vy, vm);
    let mut weeks = col()
        .gap(gap)
        .role("grid")
        .label(format!("{} {vy}", months(lang())[vm as usize]))
        .focus_group();
    let mut week = row().gap(gap);
    let mut slot = 0;
    for _ in 0..first {
        week = week.child(el().size(cell_w, cell_w));
        slot += 1;
    }
    for d in 1..=n {
        let is_sel = (d, vm, vy) == (selected.2, selected.1, selected.0);
        let is_today = (d, vm, vy) == (today.2, today.1, today.0);
        let mut b = row()
            .key(format!("et-day-{d}"))
            .size(cell_w, cell_w)
            .center()
            .radius(tk.r(0.375))
            .role("button")
            .label(d.to_string())
            .checked(is_sel)
            .focusable(true)
            .focus_selected(is_sel)
            .on_press(move |a: &mut App| {
                let ui = &mut a.reports.ledger.exit;
                let (y, m) = ui.month;
                ui.date = format!("{y:04}-{:02}-{d:02}", m + 1);
                ui.popup = None;
            });
        b = if is_sel {
            b.fill(tk.primary)
        } else {
            let b = b.hover(tk.accent);
            if is_today {
                b.border(1.0, tk.ring).border_layout()
            } else {
                b
            }
        };
        week = week.child(
            b.child(
                text(d.to_string())
                    .font(tnum(tk.tsm(400.0)))
                    .color(if is_sel { tk.primary_fg } else { tk.fg }),
            ),
        );
        slot += 1;
        if slot % 7 == 0 {
            weeks = weeks.child(week);
            week = row().gap(gap);
        }
    }
    if slot % 7 != 0 {
        weeks = weeks.child(week);
    }
    crate::shell::glass(
        tk,
        col()
            .padding(tk.r(0.75))
            .child(header)
            .child(weekdays)
            .child(el().h(gap))
            .child(weeks),
    )
    .radius(tk.r(0.375))
    .role("dialog")
    .label(t("overview:exitTax.departureDate"))
}

// ---- View ---------------------------------------------------------------------------------

/// `localeForFiat` money with two decimals; `—` when unpriced.
fn money(v: &Value, ccy: &str) -> String {
    match v.as_f64() {
        Some(x) => crate::format::fiat(x, ccy),
        None => "—".into(),
    }
}

fn btc(sats: &Value) -> String {
    format!("{:.8} BTC", sats.as_f64().unwrap_or(0.0) / 1e8)
}

/// A tax rate (a fraction) with one decimal in the UI language: en `27.5%`, de-AT `27,5 %`.
fn percent(rate: f64) -> String {
    let v = format!("{:.1}", rate * 100.0);
    match lang() {
        Lang::En => format!("{v}%"),
        Lang::De => format!("{} %", v.replace('.', ",")),
    }
}

/// shadcn `Select` trigger (`h-8 w-[220px]`).
fn destination_trigger(app: &App, tk: &Tk) -> El {
    let ui = &app.reports.ledger.exit;
    let label = DESTINATIONS
        .iter()
        .find(|(v, _)| *v == ui.destination)
        .map(|(_, key)| t(key))
        .unwrap_or_default();
    let open = ui.popup == Some(Popup::Destination);
    // `SelectTrigger` sets no text colour either: it takes the label's muted ink.
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, tk.accent)
    };
    let t = row()
        .key("et-destination")
        .h(tk.r(2.0))
        .w(220.0)
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(fill)
        .hover(hover)
        .role("combobox")
        .label(label.clone())
        .a11y_value(label.clone())
        .focusable(true)
        .child(
            text(label)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .truncate(),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    crate::ui::popup::trigger(t, crate::ui::popup::Kind::Listbox, open, |a: &mut App| {
        let p = &mut a.reports.ledger.exit.popup;
        *p = if *p == Some(Popup::Destination) {
            None
        } else {
            Some(Popup::Destination)
        };
    })
}

/// The option list, item-aligned over its trigger as Radix places it.
fn destination_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let ui = &app.reports.ledger.exit;
    let current = DESTINATIONS
        .iter()
        .position(|(v, _)| *v == ui.destination)
        .unwrap_or(0);
    let mut p = crate::ui::popup::Popup::listbox("et-destination", |a: &mut App| {
        a.reports.ledger.exit.popup = None
    });
    for (v, key) in DESTINATIONS {
        let label = t(key);
        let checked = v == ui.destination;
        p.item(
            row()
                .w_full()
                .h(tk.r(2.0))
                .pl(tk.r(0.5))
                .pr(tk.r(2.0))
                .center_y()
                .radius(tk.r(0.25))
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .role("option")
                .checked(checked)
                .label(label.clone())
                .on_press_tx(move |a: &mut App, tx: &mut rune::Tx| {
                    let ui = &mut a.reports.ledger.exit;
                    ui.destination = v.into();
                    ui.popup = None;
                    // The destination re-lays out the form around the trigger.
                    crate::ui::popup::refocus(tx, "et-destination");
                })
                .child(
                    text(label.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .grow(1.0),
                )
                .child(if checked {
                    row()
                        .absolute(220.0 - tk.r(0.5) * 2.0 - tk.r(1.5), 0.0)
                        .h(tk.r(2.0))
                        .center_y()
                        .child(ic(&I::CHECK, tk.r(1.0), tk.fg))
                } else {
                    el()
                }),
            format!("et-destination-{v}"),
            label,
            true,
            checked,
        );
    }
    // The selected row lands over the trigger: back up by the trigger, the list's padding and
    // the rows above it.
    let gap = -(tk.r(2.0) + tk.r(0.25) + tk.r(2.0) * current as f32);
    let list = col()
        .w(220.0)
        .padding(tk.r(0.25))
        .label(t("overview:exitTax.destination.label"));
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, l).radius(tk.r(0.375)),
        Placement::BottomStart,
        gap,
    );
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.reports.ledger.exit;
    let ww = c::win_w(app, cx);
    let report = ui.report();
    let resolved = resolved_date(ui);
    let field_label = |s: &str| text(s.to_owned()).font(tk.txs(500.0)).color(tk.muted_fg);
    let date_button = w::button_surface(
        tk,
        row()
            .key("et-date")
            .h(tk.r(2.0))
            .w(180.0)
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y(),
        Variant::Outline,
    )
    .role("button")
    .label(format_display(&resolved, lang()))
    .focusable(true)
    .on_press(open_picker)
    // The outline button sets no ink, so it inherits the wrapping label's muted colour.
    .child(ic(&I::CALENDAR_DAYS, tk.r(1.0), tk.muted_fg.alpha(0.7)))
    .child(
        text(format_display(&resolved, lang()))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg),
    );
    let mut controls = row().gap(tk.r(0.75)).items_end().flex_wrap().child(
        col()
            .gap(tk.r(0.25))
            .child(field_label(&t("overview:exitTax.departureDate")))
            .child(date_button),
    );
    if let Some(r) = report {
        controls = controls.child(
            text(tr(
                "overview:exitTaxDepartureTimezone",
                &[("timezone", r["departureTimezone"].as_str().unwrap_or(""))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    // Keyed: the timezone note before it comes and goes with the report, and an unkeyed field
    // would remount (losing the trigger's focus) as its index shifts.
    controls = controls.child(
        col()
            .key("et-destination-field")
            .gap(tk.r(0.25))
            .child(field_label(&t("overview:exitTax.destination.label")))
            .child(destination_trigger(app, tk)),
    );
    let description = text(t("overview:exitTax.description"))
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .wrap()
        .max_w(tk.r(48.0))
        .shrink();
    let header = if ww >= 640.0 {
        // The description keeps its `max-w-3xl`; the controls wrap into what is left, as the
        // browser's flex row resolves it.
        let avail = c::page_w(app, cx, tk);
        let desc_w = tk.r(48.0).min(avail - tk.r(0.75) - 220.0).max(0.0);
        row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .child(description.w(desc_w))
            .child(controls.w(avail - desc_w - tk.r(0.75)))
    } else {
        col()
            .w_full()
            .gap(tk.r(0.625))
            .child(description)
            .child(controls)
    };
    match ui.popup {
        Some(Popup::Date) => {
            layer.push(rune_kit::scrim("et-date-scrim", |a: &mut App| {
                a.reports.ledger.exit.popup = None
            }));
            let surf = calendar(app, tk)
                .modal(true, |a: &mut App| a.reports.ledger.exit.popup = None)
                .focus_return("et-date");
            layer.push(crate::shell::anchored(
                tk,
                surf,
                "et-date",
                Placement::BottomStart,
                4.0,
            ));
        }
        Some(Popup::Destination) => destination_menu(app, tk, layer),
        None => {}
    }
    let page = c::shell(tk).key("et-page").child(header);
    match ui.cache.get(&ui.preview_key()) {
        Some(Load::Ready(r)) => page.child(body(app, cx, tk, r, ww)),
        Some(Load::Failed(e)) => page.child(
            col()
                .w_full()
                .padding(tk.r(1.0))
                .gap(tk.r(0.25))
                .radius(tk.r(0.5))
                .fill(tk.destructive.alpha(0.05))
                .border(1.0, tk.destructive.alpha(0.3))
                .border_layout()
                .role("alert")
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.destructive))
                        .child(
                            text(t("overview:exitTax.loadFailed"))
                                .font(tk.tsm(500.0))
                                .color(tk.destructive),
                        ),
                )
                .child(
                    text(if e.is_empty() {
                        t("overview:exitTax.unknownError")
                    } else {
                        e.clone()
                    })
                    .font(tk.tsm(400.0))
                    .color(tk.destructive.alpha(0.9))
                    .wrap()
                    .w_full(),
                ),
        ),
        _ => page.child(c::screen_skeleton(tk, tk.r(10.0), ww >= 1536.0)),
    }
}

fn section_title(tk: &Tk, glyph: &'static rune::Icon, title: &str) -> El {
    row()
        .gap(tk.r(0.5))
        .center_y()
        .child(ic(glyph, tk.r(1.0), tk.muted_fg))
        .child(text(title.to_owned()).font(tk.tsm(500.0)).color(tk.fg))
}

fn body(app: &App, cx: &Cx<App>, tk: &Tk, r: &Value, ww: f32) -> El {
    let ui = &app.reports.ledger.exit;
    let hidden = app.prefs.hide_sensitive();
    let ccy = r["fiatCurrency"].as_str().unwrap_or("EUR");
    let m = |v: &Value| money(v, ccy);
    let status = &r["status"];
    let missing: Vec<String> = status["missingFmvAssets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let needs = status["needsJournals"].as_bool().unwrap_or(false);
    let quarantines = status["quarantines"].as_f64().unwrap_or(0.0);
    let totals = &r["totals"];
    let deferred = totals["collectionTiming"].as_str() == Some("deferred");
    let incomplete = needs || quarantines > 0.0 || !missing.is_empty();
    let is_at = r["jurisdictionCode"].as_str() == Some("AT");
    let amber = c::amber();
    let mut out = col().w_full().gap(tk.card_gap());

    if incomplete {
        let mut lines = col().min_w(0.0).shrink().child(
            text(t("overview:exitTaxReadiness.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        );
        if needs {
            lines = lines.child(
                text(t("overview:exitTaxReadiness.journals"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        if quarantines > 0.0 {
            // `<Trans>` with the `<queue>` link to the review queue.
            let raw = tc(
                "overview:exitTaxReadiness.quarantines",
                quarantines as usize,
                &[],
            );
            let (before, rest) = raw.split_once("<queue>").unwrap_or((&raw, ""));
            let (link, after) = rest.split_once("</queue>").unwrap_or((rest, ""));
            lines = lines.child(
                row()
                    .flex_wrap()
                    .child(
                        text(before.to_owned())
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    )
                    .child(
                        c::underlined(
                            tk,
                            text(link.to_owned()).font(tk.tsm(400.0)).color(tk.muted_fg),
                            tk.muted_fg,
                        )
                        .role("link")
                        .label(link.to_owned())
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|a: &mut App| crate::shell::go(a, "/review")),
                    )
                    .child(
                        text(after.to_owned())
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        if !missing.is_empty() {
            lines = lines.child(
                text(tr(
                    "overview:exitTaxFmv.incomplete",
                    &[("assets", &missing.join(", "))],
                ))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
            );
        }
        out = out.child(
            c::banner(tk, &amber)
                .role("status")
                .items_start()
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), c::hue_ink(tk, &amber)).mt(tk.r(0.125)))
                .child(lines),
        );
    }

    // Headline liability.
    let rate = totals["estimatedTaxRate"].as_f64();
    let key = match (rate.is_some(), is_at) {
        (true, true) => "overview:exitTax.headline.rateNeu",
        (true, false) => "overview:exitTax.headline.rate",
        (false, true) => "overview:exitTax.headline.onNeu",
        (false, false) => "overview:exitTax.headline.on",
    };
    // `{{gain}}` stays in the sentence so the (possibly blurred) amount sits where the language
    // puts it.
    let sentence = tr(key, &[("rate", &rate.map(percent).unwrap_or_default())]);
    let (lead, tail) = sentence
        .split_once("{{gain}}")
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .unwrap_or((sentence.clone(), String::new()));
    let gain = m(&totals["taxableGain"]);
    let sub = if hidden {
        row()
            .flex_wrap()
            .mt(tk.r(0.25))
            .child(text(lead).font(tk.tsm(400.0)).color(tk.muted_fg))
            .child(sens(gain, tk.tsm(400.0), tk.muted_fg, true))
            .child(text(tail).font(tk.tsm(400.0)).color(tk.muted_fg))
    } else {
        row().mt(tk.r(0.25)).child(
            text(format!("{lead}{gain}{tail}"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        )
    };
    let headline_left = col()
        .min_w(0.0)
        .child(
            text(t("overview:exitTax.headline.title").to_uppercase())
                .font(tk.txs(500.0).tracking(0.025 * tk.r(0.75)))
                .color(tk.muted_fg)
                .label(t("overview:exitTax.headline.title")),
        )
        .child(sens(m(&totals["estimatedTax"]), tnum(t4xl(tk)), tk.fg, hidden).mt(tk.r(0.25)))
        .child(sub);
    let dd = |label: &str, value: El| {
        col()
            .w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .child(
                text(label.to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
            .child(value)
    };
    let dval = |s: String, sensitive: bool| {
        if sensitive {
            sens(s, tnum(tk.tsm(500.0)), tk.fg, hidden)
        } else {
            text(s).font(tk.tsm(500.0)).color(tk.fg)
        }
    };
    let dl = col()
        .gap(tk.r(0.75))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .child(dd(
                    &t(if is_at {
                        "overview:exitTax.headline.neuGain"
                    } else {
                        "overview:exitTax.headline.taxableGain"
                    }),
                    dval(m(&totals["neuGain"]), true),
                ))
                .child(dd(
                    &t(if is_at {
                        "overview:exitTax.headline.altExcluded"
                    } else {
                        "overview:exitTax.headline.excludedHoldings"
                    }),
                    dval(m(&totals["altMarketValue"]), true),
                )),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .child(dd(
                    &t("overview:exitTax.headline.method"),
                    dval(r["method"].as_str().unwrap_or("").to_owned(), false),
                ))
                .child(dd(
                    &t("overview:exitTax.headline.jurisdiction"),
                    dval(
                        r["jurisdictionCode"].as_str().unwrap_or("").to_owned(),
                        false,
                    ),
                )),
        );
    let headline = if ww >= 640.0 {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .center_y()
            .child(headline_left.w(0.0).grow(1.4))
            .child(dl.w(0.0).grow(1.0))
    } else {
        col().w_full().gap(tk.r(1.0)).child(headline_left).child(dl)
    };
    out = out.child(surface(tk).w_full().padding(tk.card_pad()).child(headline));

    // Collection timing.
    let (hue, glyph, title, body_text) = if deferred {
        (
            c::sky(),
            &I::CLOCK,
            t("overview:exitTax.timing.deferredTitle"),
            t("overview:exitTax.timing.deferredBody"),
        )
    } else {
        (
            amber,
            &I::TRIANGLE_ALERT,
            t("overview:exitTax.timing.dueTitle"),
            t("overview:exitTax.timing.dueBody"),
        )
    };
    out = out.child(
        c::banner(tk, &hue)
            .items_start()
            .child(ic(glyph, tk.r(1.0), c::hue_ink(tk, &hue)).mt(tk.r(0.125)))
            .child(
                col()
                    .min_w(0.0)
                    .shrink()
                    .child(text(title).font(tk.tsm(500.0)).color(tk.fg))
                    .child(
                        text(body_text)
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            ),
    );

    let main = col()
        .min_w(0.0)
        .gap(tk.card_gap())
        .child(lots(app, cx, tk, r, is_at, ccy));
    let main = if r["walletHoldings"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        main.child(holdings(app, cx, tk, r, ccy))
    } else {
        main
    };
    let side = col()
        .min_w(0.0)
        .gap(tk.card_gap())
        .child(handoff(app, tk, ui, incomplete))
        .child(fmv_card(tk, r, ccy))
        .child(assumptions(tk, r));
    out = if ww >= 1536.0 {
        out.child(
            row()
                .w_full()
                .gap(tk.card_gap())
                .items_start()
                .child(main.w(0.0).grow(1.0))
                .child(side.w(360.0)),
        )
    } else {
        out.child(
            col()
                .w_full()
                .gap(tk.card_gap())
                .child(main.w_full())
                .child(side.w_full()),
        )
    };

    if is_at {
        out = out.child(
            row()
                .w_full()
                .padding(tk.r(0.75))
                .radius(tk.r(0.5))
                .fill(tk.muted.alpha(0.2))
                .border(1.0, tk.border)
                .border_layout()
                .child(
                    text(t("overview:exitTax.historicalNote"))
                        .font(tk.txs(400.0).lh(tk.r(0.75) * 1.625))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    out
}

/// `text-4xl` (2.25 rem / 2.5 rem), semibold, `tracking-tight`.
fn t4xl(tk: &Tk) -> rune::TextStyle {
    rune::TextStyle::new(tk.r(2.25), 600.0)
        .lh(tk.r(2.5))
        .tracking(-0.025 * tk.r(2.25))
}

/// A cell of the exit-tax tables: plain or blurred text, or a badge.
enum Cell {
    Text(String, rune::TextStyle, Color),
    Sens(String, rune::TextStyle, Color),
    Badge(String, Color, Color, Color),
}

impl Cell {
    fn width(&self, tk: &Tk) -> f32 {
        match self {
            Cell::Text(s, st, _) | Cell::Sens(s, st, _) => c::tw(s, *st),
            Cell::Badge(s, ..) => c::tw(s, tk.txs(500.0)) + tk.r(1.0) + 2.0,
        }
    }
    fn el(self, tk: &Tk, hidden: bool) -> El {
        match self {
            Cell::Text(s, st, col) => text(s).font(st).color(col),
            Cell::Sens(s, st, col) => sens(s, st, col, hidden),
            Cell::Badge(s, ink, fill, border) => c::badge(tk, &s, ink, fill, border),
        }
    }
}

/// A `kb-surface` table section: the icon title (`px-5 pt-4`), then the table (`px-1 pb-2
/// sm:px-2`) laid out as the browser sizes it.
#[allow(clippy::too_many_arguments)]
fn table_section(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    key: &str,
    glyph: &'static rune::Icon,
    title: &str,
    cols: &[Col],
    labels: &[&str],
    body: Vec<Vec<Cell>>,
    min: f32,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let mut content = vec![0.0f32; cols.len()];
    for r in &body {
        for (k, cell) in r.iter().enumerate() {
            content[k] = content[k].max(cell.width(tk));
        }
    }
    let label_w: Vec<f32> = labels.iter().map(|l| c::tw(l, tk.tsm(500.0))).collect();
    let avail = c::avail(cx, key, c::page_w(app, cx, tk) - 2.0 - tk.r(1.0));
    let table = c::Table::auto(tk, cols, &label_w, &content, avail, min);
    let header = thead(
        tk,
        labels
            .iter()
            .enumerate()
            .map(|(k, l)| table.th(tk, k, l))
            .collect(),
    );
    let n = body.len();
    let rows = body
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            trow(
                tk,
                i + 1 == n,
                r.into_iter()
                    .enumerate()
                    .map(|(k, cell)| table.td(tk, k, cell.el(tk, hidden)))
                    .collect(),
            )
            .key(format!("{key}-{i}"))
        })
        .collect();
    surface(tk)
        .w_full()
        .min_w(0.0)
        .clip_rounded(tk.r(0.75))
        .child(
            row()
                .px(tk.r(1.25))
                .pt(tk.r(1.0))
                .child(section_title(tk, glyph, title)),
        )
        .child(col().w_full().px(tk.r(0.5)).pb(tk.r(0.5)).child(table.view(
            key,
            title.to_owned(),
            header,
            rows,
        )))
}

fn lots(app: &App, cx: &Cx<App>, tk: &Tk, r: &Value, is_at: bool, ccy: &str) -> El {
    const COLS: [Col; 7] = [
        c::c(0.0),
        c::c(0.0),
        c::cr(0.0),
        c::cr(0.0),
        c::cr(0.0),
        c::cr(0.0),
        c::cr(0.0),
    ];
    let num = |s: String| Cell::Sens(s, tnum(tk.tsm(400.0)), tk.fg);
    let body = r["lots"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|lot| {
            let neu = lot["regime"].as_str() == Some("neu");
            let regime = if neu {
                Cell::Badge(
                    t(if is_at {
                        "overview:exitTax.lots.neuTaxable"
                    } else {
                        "overview:exitTax.lots.taxable"
                    }),
                    tk.primary,
                    tk.primary.alpha(0.15),
                    Color::TRANSPARENT,
                )
            } else {
                Cell::Badge(
                    t(if is_at {
                        "overview:exitTax.lots.altTaxFree"
                    } else {
                        "overview:exitTax.lots.taxFree"
                    }),
                    tk.muted_fg,
                    Color::TRANSPARENT,
                    tk.border,
                )
            };
            let kz = lot["kennzahl"]
                .as_i64()
                .map(|k| k.to_string())
                .unwrap_or_else(|| "—".into());
            vec![
                Cell::Text(
                    lot["asset"].as_str().unwrap_or("").to_owned(),
                    tk.tsm(500.0),
                    tk.fg,
                ),
                regime,
                num(btc(&lot["quantitySats"])),
                num(if neu {
                    money(&lot["costBasis"], ccy)
                } else {
                    "—".into()
                }),
                num(money(&lot["marketValue"], ccy)),
                num(if neu {
                    money(&lot["gain"], ccy)
                } else {
                    t("overview:exitTax.lots.excluded")
                }),
                Cell::Text(kz, tnum(tk.tsm(400.0)), tk.muted_fg),
            ]
        })
        .collect();
    let labels = [
        t("overview:exitTax.columns.asset"),
        t("overview:exitTax.columns.regime"),
        t("overview:exitTax.columns.quantity"),
        t("overview:exitTax.columns.costBasis"),
        t("overview:exitTax.columns.marketValue"),
        t("overview:exitTax.columns.gain"),
        t("overview:exitTax.columns.kennzahl"),
    ];
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    table_section(
        app,
        cx,
        tk,
        "et-lots",
        &I::LANDMARK,
        &t("overview:exitTax.lots.title"),
        &COLS,
        &labels,
        body,
        680.0,
    )
}

fn holdings(app: &App, cx: &Cx<App>, tk: &Tk, r: &Value, ccy: &str) -> El {
    const COLS: [Col; 4] = [c::c(0.0), c::c(0.0), c::cr(0.0), c::cr(0.0)];
    let body = r["walletHoldings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|h| {
            vec![
                Cell::Text(
                    h["wallet"].as_str().unwrap_or("").to_owned(),
                    tk.tsm(500.0),
                    tk.fg,
                ),
                Cell::Text(
                    h["asset"].as_str().unwrap_or("").to_owned(),
                    tk.tsm(400.0),
                    tk.fg,
                ),
                Cell::Sens(btc(&h["quantitySats"]), tnum(tk.tsm(400.0)), tk.fg),
                Cell::Sens(money(&h["marketValue"], ccy), tnum(tk.tsm(400.0)), tk.fg),
            ]
        })
        .collect();
    let labels = [
        t("overview:exitTax.columns.wallet"),
        t("overview:exitTax.columns.asset"),
        t("overview:exitTax.columns.quantity"),
        t("overview:exitTax.columns.marketValue"),
    ];
    table_section(
        app,
        cx,
        tk,
        "et-holdings",
        &I::INFO,
        &t("overview:exitTax.holdings.title"),
        &COLS,
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        body,
        480.0,
    )
}

fn handoff(app: &App, tk: &Tk, ui: &ExitUi, incomplete: bool) -> El {
    let busy = ui.active_export;
    let button = |fmt: &'static str, variant: Variant, glyph: &'static rune::Icon, label: &str| {
        let ink = if variant == Variant::Default {
            tk.primary_fg
        } else {
            tk.fg
        };
        let mut b = w::button_surface(
            tk,
            row()
                .key(format!("et-export-{fmt}"))
                .w_full()
                .h(tk.r(2.25))
                .px(tk.r(0.75))
                .gap(tk.r(0.5))
                .center(),
            variant,
        )
        .role("button")
        .label(label.to_owned())
        .focusable(busy.is_none())
        .disabled(busy.is_some())
        .on_press(move |a: &mut App| export(a, fmt));
        if busy.is_some() {
            b = b.opacity(0.5);
        }
        b = if busy == Some(fmt) {
            b.child(ic(&I::LOADER_CIRCLE, tk.r(1.0), ink).looping(rune::LoopSpec::rotation(0.9)))
        } else {
            b.child(ic(glyph, tk.r(1.0), ink))
        };
        b.child(text(label.to_owned()).font(tk.tsm(500.0)).color(ink))
    };
    let mut card = surface(tk)
        .key("et-handoff")
        .w_full()
        .padding(tk.card_pad())
        .child(section_title(
            tk,
            &I::DOWNLOAD,
            &t("overview:exitTax.handoff.title"),
        ))
        .child(
            text(t("overview:exitTax.handoff.body"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .mt(tk.r(0.25)),
        );
    if incomplete {
        let amber = c::amber();
        card = card.child(
            text(t("overview:exitTaxReadiness.exportDraft"))
                .font(tk.txs(400.0))
                .color(if tk.dark { amber.c400 } else { amber.c700 })
                .wrap()
                .mt(tk.r(0.5)),
        );
    }
    card = card.child(
        col()
            .w_full()
            .mt(tk.r(0.75))
            .gap(tk.r(0.5))
            .child(button(
                "pdf",
                Variant::Default,
                &I::FILE_TEXT,
                &t("overview:exitTax.handoff.exportPdf"),
            ))
            .child(button(
                "xlsx",
                Variant::Outline,
                &I::FILE_SPREADSHEET,
                &t("overview:exitTax.handoff.exportXlsx"),
            )),
    );
    if let Some(st) = &ui.status {
        let ok = st.tone == Tone::Success;
        let em = c::emerald();
        let (fill, border, ink) = if ok {
            (em.c500.alpha(0.1), em.c500.alpha(0.3), tk.fg)
        } else {
            (
                tk.destructive.alpha(0.05),
                tk.destructive.alpha(0.3),
                tk.destructive,
            )
        };
        let glyph = if ok {
            ic(&I::CIRCLE_CHECK, tk.r(0.875), c::hue_ink(tk, &em))
        } else {
            ic(&I::TRIANGLE_ALERT, tk.r(0.875), ink)
        };
        let mut msg = col().min_w(0.0).shrink().child(
            text(st.message.clone())
                .font(tk.txs(400.0))
                .color(ink)
                .wrap_anywhere(),
        );
        // `canOpenExportedFiles()`: the desktop opens the managed file with the system handler.
        let open = t("common:actions.open");
        let mut acts = row().mt(tk.r(0.25)).gap(tk.r(0.75)).center_y();
        if let (true, Some(path)) = (ok, st.path.clone()) {
            acts = acts.child(
                row()
                    .key("et-export-open")
                    .gap(tk.r(0.25))
                    .center_y()
                    .role("button")
                    .label(open.clone())
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |a: &mut App| open_export(a, &path))
                    .child(ic(&I::FOLDER_OPEN, tk.r(0.75), ink))
                    .child(c::underlined(
                        tk,
                        text(open).font(tk.txs(400.0)).color(ink),
                        ink,
                    )),
            );
        }
        // `toastopen`: Show in Finder beside the Open.
        if ok && !st.actions.is_empty() {
            acts = acts.child(crate::toastopen::links(
                tk,
                &st.actions,
                |a| format!("et-export-{}", a.id),
                tk.muted_fg,
                ink,
            ));
        }
        if ok && (st.path.is_some() || !st.actions.is_empty()) {
            msg = msg.child(acts);
        }
        card = card.child(
            row()
                .key("et-export-status")
                .w_full()
                .mt(tk.r(0.75))
                .padding(tk.r(0.5))
                .gap(tk.r(0.5))
                .items_start()
                .radius(tk.r(0.5))
                .fill(fill)
                .border(1.0, border)
                .border_layout()
                .child(glyph.mt(tk.r(0.125)))
                .child(msg.grow(1.0))
                .child(
                    crate::shell::transient::dismiss(
                        tk,
                        "et-export-status-dismiss",
                        ink,
                        |a: &mut App| a.reports.ledger.exit.status = None,
                    )
                    .mt(tk.r(-0.25))
                    .mr(tk.r(-0.25)),
                ),
        );
    }
    let _ = app;
    card
}

/// ExitTaxFmvCard.tsx.
fn fmv_card(tk: &Tk, r: &Value, ccy: &str) -> El {
    let quotes = r["fmvSource"].as_array().cloned().unwrap_or_default();
    let missing: Vec<&str> = quotes
        .iter()
        .filter(|q| q["source"].as_str() == Some("missing"))
        .filter_map(|q| q["asset"].as_str())
        .collect();
    let mut card = surface(tk)
        .key("et-fmv")
        .w_full()
        .padding(tk.card_pad())
        .child(
            text(t("overview:exitTaxFmv.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        );
    if !missing.is_empty() {
        let amber = c::amber();
        card = card.child(
            text(tr(
                "overview:exitTaxFmv.incomplete",
                &[("assets", &missing.join(", "))],
            ))
            .font(tk.txs(400.0))
            .color(if tk.dark { amber.c400 } else { amber.c700 })
            .wrap()
            .mt(tk.r(0.5))
            .role("status"),
        );
    }
    let mut list = col().w_full().mt(tk.r(0.5)).gap(tk.r(0.25));
    for q in &quotes {
        let source = q["source"].as_str().unwrap_or("missing");
        let as_of = q["asOf"].as_str().and_then(utc_timestamp);
        let right = col()
            .items_end()
            .child(
                row()
                    .center_y()
                    .child(
                        text(money(&q["rate"], ccy))
                            .font(tnum(tk.txs(400.0)))
                            .color(tk.muted_fg),
                    )
                    .child(
                        c::outline_badge(
                            tk,
                            &t(&format!("overview:exitTaxFmv.source.{source}")),
                            tk.fg,
                        )
                        .ml(tk.r(0.5)),
                    ),
            )
            .child(
                text(format!(
                    "{} {}",
                    t("overview:exitTaxFmv.asOf"),
                    as_of.unwrap_or_else(|| t("overview:exitTaxFmv.unavailable"))
                ))
                .font(tnum(tk.txs(400.0)))
                .color(tk.muted_fg)
                .mt(tk.r(0.25)),
            );
        list = list.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .items_start()
                .justify_between()
                .child(
                    text(q["pair"].as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
                .child(right),
        );
    }
    card.child(list)
}

/// `formatUtcTimestamp`: an ISO instant with a `Z` or `±hh:mm` offset, re-expressed in UTC,
/// keeping its fractional seconds (`2024-12-31T12:34:56.123456+01:00` → `…11:34:56.123456Z`).
pub fn utc_timestamp(v: &str) -> Option<String> {
    let (body, offset) = if let Some(b) = v.strip_suffix('Z') {
        (b, 0i64)
    } else {
        let (b, off) = v.split_at(v.len().checked_sub(6)?);
        let sign = match off.as_bytes().first()? {
            b'+' => 1,
            b'-' => -1,
            _ => return None,
        };
        let (h, m) = off[1..].split_once(':')?;
        (
            b,
            sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?),
        )
    };
    let (main, frac) = match body.split_once('.') {
        Some((m, f)) if !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()) => {
            (m, format!(".{f}"))
        }
        Some(_) => return None,
        None => (body, String::new()),
    };
    let (date, time) = main.split_once('T')?;
    let (y, m, d) = (
        date.get(0..4)?.parse::<i32>().ok()?,
        date.get(5..7)?.parse::<u32>().ok()?,
        date.get(8..10)?.parse::<u32>().ok()?,
    );
    let mut parts = time.split(':');
    let (hh, mm) = (
        parts.next()?.parse::<i64>().ok()?,
        parts.next()?.parse::<i64>().ok()?,
    );
    let ss = parts
        .next()
        .map(|s| s.parse::<i64>())
        .transpose()
        .ok()?
        .unwrap_or(0);
    let total = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - offset * 60;
    let (day, sec) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let (y, m, d) = civil(day);
    // `toISOString()` always prints milliseconds; `.000` gives way to the original fraction.
    let frac = if frac.is_empty() { String::new() } else { frac };
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{frac}Z",
        sec / 3600,
        sec / 60 % 60,
        sec % 60
    ))
}

fn assumptions(tk: &Tk, r: &Value) -> El {
    let notes: Vec<String> = r["assumptions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut list = col().w_full().mt(tk.r(0.5)).gap(tk.r(0.25)).role("list");
    for n in notes {
        list = list.child(
            row()
                .w_full()
                .role("listitem")
                .child(
                    row()
                        .w(tk.r(1.0))
                        .h(tk.r(1.0))
                        .child(text("•").font(tk.txs(400.0)).color(tk.muted_fg)),
                )
                .child(
                    text(n)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w(0.0)
                        .grow(1.0),
                ),
        );
    }
    surface(tk)
        .key("et-assumptions")
        .w_full()
        .padding(tk.card_pad())
        .child(section_title(
            tk,
            &I::SHIELD_CHECK,
            &t("overview:exitTax.assumptions.title"),
        ))
        .child(list)
        .child(
            row()
                .w_full()
                .mt(tk.r(0.75))
                .padding(tk.r(0.5))
                .radius(tk.r(0.5))
                .fill(tk.muted.alpha(0.3))
                .border(1.0, tk.border)
                .border_layout()
                .child(
                    text(r["reviewGate"].as_str().unwrap_or("").to_owned())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .w_full(),
                ),
        )
}

// ---- Scenes -------------------------------------------------------------------------------

pub fn scene_set_date(app: &mut App, day: &str) {
    app.reports.ledger.exit.date = day.into();
    prime(app);
}

pub fn scene_set_destination(app: &mut App, dest: &str) {
    app.reports.ledger.exit.destination = dest.into();
    prime(app);
}

pub fn scene_open_picker(app: &mut App) {
    open_picker(app);
}

/// Writes the PDF handoff into the book's exports folder and settles as if the Save As was
/// cancelled (the managed file stays, as the web reports it).
pub fn scene_exported(app: &mut App) {
    export(app, "pdf");
    crate::reports::pump_until(app, |a| {
        a.reports.ledger.exit.pending_save.is_some()
            || a.reports.ledger.exit.active_export.is_none()
    });
    if let Some((fmt, path)) = app.reports.ledger.exit.pending_save.take() {
        saved(app, fmt, path, Ok(None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_timestamps_match_the_web() {
        assert_eq!(
            utc_timestamp("2024-03-31T23:59:59.500000Z").as_deref(),
            Some("2024-03-31T23:59:59.500000Z")
        );
        assert_eq!(
            utc_timestamp("2024-12-31T12:34:56.123456+01:00").as_deref(),
            Some("2024-12-31T11:34:56.123456Z")
        );
        assert_eq!(
            utc_timestamp("2024-12-31T12:34:56-02:00").as_deref(),
            Some("2024-12-31T14:34:56Z")
        );
        assert_eq!(
            utc_timestamp("2025-11-10T00:00:00Z").as_deref(),
            Some("2025-11-10T00:00:00Z")
        );
        assert_eq!(utc_timestamp("garbage"), None);
    }

    #[test]
    fn calendar_math() {
        assert_eq!(days_in_month(2024, 1), 29);
        assert_eq!(days_in_month(2025, 10), 30);
        // 2026-11-01 is a Sunday (Monday-first index 6); 2025-11-01 a Saturday.
        assert_eq!(first_weekday(2026, 10), 6);
        assert_eq!(first_weekday(2025, 10), 5);
        assert_eq!(civil(days_from_civil(2026, 11, 15)), (2026, 11, 15));
    }

    #[test]
    fn display_dates() {
        assert_eq!(format_display("2026-11-15", Lang::En), "Nov 15, 2026");
        assert_eq!(format_display("2026-03-05", Lang::De), "05. März 2026");
        assert_eq!(format_display("", Lang::En), "Pick a date");
    }
}
