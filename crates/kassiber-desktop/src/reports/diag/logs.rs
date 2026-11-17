//! Logs (routes/Logs.tsx + components/kb/LogsTableControls.tsx): the RAM-only log ring with
//! level/module/search filters (substring or regex), the redacted / raw / masked-amount views,
//! the 5-minute raw-view timeout, newest-N rendering, expandable rows, autoscroll with "Jump to
//! latest", Copy 200, Clear, and exports (Markdown / JSONL / Log, the support bundle dialog)
//! through the native Save dialog. Nothing is written unless the user exports.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rune::{Color, Cx, El, KeyEvent, ScrollPos, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::{Placement, TextArea};

use super::logstore::{self, LEVELS, Record, Store};
use super::redact::{Mode, Opts};
use super::widgets::{self, Entry};
use super::{Popup, time};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, button_surface};
use crate::ui::ic;

const RENDER_STEP: usize = 1000;
const MAX_RENDERED: usize = 8000;
const PREVIEW_LINES: usize = 30;
const RAW_SECONDS: f64 = 300.0;
/// The Tauri command only writes these extensions (`save_logs_export_as`).
const ALLOWED_EXT: [&str; 3] = ["md", "log", "jsonl"];

pub struct State {
    pub store: Store,
    pub level: Option<&'static str>,
    pub module: Option<String>,
    pub redacted: bool,
    pub mask_amounts: bool,
    /// (app clock, wall clock) the raw view turns back to redacted.
    pub raw_until: Option<(f64, f64)>,
    pub query: TextEdit,
    pub regex: bool,
    pub render_limit: usize,
    pub expanded: HashSet<String>,
    pub autoscroll: bool,
    pub new_while_paused: usize,
    seen_count: usize,
    pub support_open: bool,
    pub support_issue: TextEdit,
    pub support_mode: Mode,
    /// A raw export waiting for its confirm (`md` / `jsonl` / `log`).
    pub confirm_raw: Option<&'static str>,
    pub scroll_bottom: bool,
    mounted: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            store: Store::new(),
            level: None,
            module: None,
            redacted: true,
            mask_amounts: false,
            raw_until: None,
            query: TextEdit::new(""),
            regex: false,
            render_limit: RENDER_STEP,
            expanded: HashSet::new(),
            autoscroll: true,
            new_while_paused: 0,
            seen_count: 0,
            support_open: false,
            support_issue: TextEdit::new(""),
            support_mode: Mode::HighSignal,
            confirm_raw: None,
            scroll_bottom: true,
            mounted: false,
        }
    }
}

impl State {
    /// Leaving `/logs`: every filter, the raw-view window, expanded rows, autoscroll and the
    /// support-bundle dialog are the route's `useState`s. The RAM ring (`store`) is the app's
    /// and stays.
    pub fn leave(&mut self) {
        let store = std::mem::replace(&mut self.store, Store::new());
        let seen = self.seen_count;
        *self = State {
            store,
            seen_count: seen,
            ..State::default()
        };
    }
}

thread_local! {
    /// Where the record viewport sits (set from `on_scroll_offset`, which can't touch state).
    static AT_BOTTOM: Cell<bool> = const { Cell::new(true) };
    static NEAR_TOP: Cell<bool> = const { Cell::new(false) };
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

#[derive(Default)]
struct Cache {
    key: String,
    filtered: Rc<Vec<usize>>,
    rendered: HashMap<(String, bool, bool), Rc<Record>>,
}

fn opts(s: &State) -> Opts {
    Opts::screen(s.redacted, s.mask_amounts)
}

fn has_filters(s: &State) -> bool {
    !s.query.text.trim().is_empty() || s.level.is_some() || s.module.is_some()
}

/// The filtered ring (indices), cached per ring generation and filter.
fn filtered(s: &State) -> Rc<Vec<usize>> {
    let key = format!(
        "{}|{}|{}|{:?}|{:?}|{}|{}",
        s.store.generation, s.query.text, s.regex, s.level, s.module, s.redacted, s.mask_amounts
    );
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.key != key {
            c.key = key;
            c.filtered = Rc::new(logstore::filter(
                &s.store,
                &s.query.text,
                s.regex,
                s.level,
                s.module.as_deref(),
                &opts(s),
            ));
        }
        c.filtered.clone()
    })
}

fn rendered(r: &Record, o: &Opts) -> Rc<Record> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.rendered.len() > 20_000 {
            c.rendered.clear();
        }
        c.rendered
            .entry((r.id.clone(), o.redacted, o.mask_amounts))
            .or_insert_with(|| Rc::new(r.redacted(o)))
            .clone()
    })
}

/// Every frame on any route: fold the app's finished requests into the ring.
pub fn collect(app: &mut App) -> bool {
    let before = app.reports.diag.logs.store.generation;
    let pending: Vec<(String, String)> = app
        .pending
        .iter()
        .map(|(id, (_, kind, _))| (id.clone(), kind.clone()))
        .collect();
    app.reports
        .diag
        .logs
        .store
        .fold_pending(pending.iter().map(|(i, k)| (i, k.as_str())));
    let timings = std::mem::take(&mut app.timings);
    app.reports.diag.logs.store.fold_timings(&timings);
    app.timings = timings;
    app.reports.diag.logs.store.generation != before
        && (app.route.path() == "/logs"
            || crate::assistant::surface_open(app, crate::assistant::Surface::Logs))
}

/// The bridge's poll while only the panel's Logs tab is open (the page runs its own `poll`).
pub fn bridge_tick(app: &mut App, tx: &mut Tx) -> bool {
    poll_bridge(app);
    wake_for_bridge(app, tx);
    false
}

/// The next poll's wake-up: none while a poll is in flight (its answer wakes the app) or behind
/// the lock (nothing polls there; the unlock redraws), so a poll that is due never spins the
/// app awake every frame.
fn wake_for_bridge(app: &App, tx: &mut Tx) {
    let st = &app.reports.diag.logs.store;
    if st.in_flight || app.locked || !st.next_poll.is_finite() {
        return;
    }
    tx.wake_after(std::time::Duration::from_secs_f64(
        (st.next_poll - app.clock).clamp(0.05, 60.0),
    ));
}

fn poll_bridge(app: &mut App) {
    let locked = app.locked;
    let st = &mut app.reports.diag.logs.store;
    // Behind the lock the daemon's ring is not read; polling resumes after the unlock.
    if locked || st.in_flight || app.clock < st.next_poll {
        return;
    }
    st.in_flight = true;
    let args = st.poll_args();
    app.send("ui.logs.snapshot", args, "reports.diag.logs.snapshot");
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    let s = &mut app.reports.diag.logs;
    if !s.mounted {
        s.mounted = true;
        s.store.next_poll = 0.0;
        s.scroll_bottom = true;
    }
    // The raw view turns itself back to redacted after five minutes.
    if let Some((until, _)) = s.raw_until {
        if app.clock >= until {
            s.redacted = true;
            s.raw_until = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                (until - app.clock).max(0.05),
            ));
        }
    }
    // Scroll position: near the top grows the rendered window; leaving the bottom pauses.
    let at_bottom = AT_BOTTOM.with(Cell::get);
    if at_bottom != s.autoscroll {
        s.autoscroll = at_bottom;
        if at_bottom {
            s.new_while_paused = 0;
        }
        changed = true;
    }
    if NEAR_TOP.with(Cell::get)
        && s.render_limit < MAX_RENDERED
        && s.store.records.len() > s.render_limit
    {
        NEAR_TOP.with(|c| c.set(false));
        s.render_limit = (s.render_limit + RENDER_STEP).min(MAX_RENDERED);
        changed = true;
    }
    let n = s.store.records.len();
    if n > s.seen_count && !s.autoscroll {
        s.new_while_paused += n - s.seen_count;
        changed = true;
    }
    s.seen_count = n;
    if s.scroll_bottom {
        s.scroll_bottom = false;
        tx.scroll_to_bottom_instant("logs-viewport");
    }
    poll_bridge(app);
    wake_for_bridge(app, tx);
    changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag == "reports.diag.logs.snapshot" {
        let clock = app.clock;
        if r.error_detail["code"] == crate::app::LOCKED_CODE {
            // Refused behind the lock: not a failed poll; the next one runs after the unlock.
            let st = &mut app.reports.diag.logs.store;
            st.in_flight = false;
            st.next_poll = 0.0;
            return;
        }
        let again =
            app.reports
                .diag
                .logs
                .store
                .on_snapshot(Some(&r.data), r.error.as_deref(), clock);
        if again {
            app.reports.diag.logs.store.next_poll = 0.0;
            poll_bridge(app);
        }
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let s = &mut app.reports.diag.logs;
    match &ev.key {
        rune::Key::Escape if s.confirm_raw.is_some() => {
            s.confirm_raw = None;
            true
        }
        rune::Key::Escape if s.support_open => {
            s.support_open = false;
            true
        }
        rune::Key::Escape if has_filters(s) => {
            s.query = TextEdit::new("");
            s.level = None;
            s.module = None;
            true
        }
        rune::Key::Text(t)
            if t == "/"
                && !ev.mods.cmd
                && !ev.mods.ctrl
                && tx.focused().is_none_or(|k| !k.contains("logs-search")) =>
        {
            s.query.focused = true;
            true
        }
        _ => false,
    }
}

pub fn prime(app: &mut App) {
    app.reports.diag.logs.mounted = true;
    // Fold what the launch already did, then read the daemon's ring once.
    collect(app);
    let st = &mut app.reports.diag.logs.store;
    st.in_flight = true;
    let args = st.poll_args();
    app.send("ui.logs.snapshot", args, "reports.diag.logs.snapshot");
    super::super::pump_until(app, |a| !a.reports.diag.logs.store.in_flight);
    collect(app);
    app.reports.diag.logs.seen_count = app.reports.diag.logs.store.records.len();
}

pub fn set_redacted(app: &mut App, on: bool) {
    let s = &mut app.reports.diag.logs;
    s.redacted = on;
    s.raw_until = if on {
        None
    } else {
        Some((
            app.clock + RAW_SECONDS,
            crate::format::now_secs() + RAW_SECONDS,
        ))
    };
}

/// The `reports-logs-filtered` scene: the debug level and the daemon's request module.
pub fn scene_filter(app: &mut App) {
    let s = &mut app.reports.diag.logs;
    s.level = Some("debug");
    s.module = Some("kassiber.daemon.request".into());
}

pub fn scene_expand_first(app: &mut App) {
    let s = &mut app.reports.diag.logs;
    if let Some(r) = s.store.records.back() {
        s.expanded.insert(r.id.clone());
    }
}

// ---- Exports -------------------------------------------------------------------------------

fn os_label() -> &'static str {
    match std::env::consts::OS {
        "macos" => "MacIntel",
        "windows" => "Win32",
        "linux" => "Linux x86_64",
        other => other,
    }
}

fn app_version(app: &App) -> String {
    let v = app
        .daemon_ready
        .as_ref()
        .and_then(|d| d["version"].as_str())
        .unwrap_or("0.0.0");
    format!("{v} (unknown)")
}

fn header(app: &App, records: &[&Record], filter: String, redaction: String) -> logstore::Header {
    logstore::Header {
        app_version: app_version(app),
        os: os_label().into(),
        generated_at: logstore::now_iso(),
        time_range: logstore::time_range(records),
        active_filter: filter,
        redaction,
    }
}

fn write(path: &std::path::Path, contents: &str) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !ALLOWED_EXT.contains(&ext.as_str()) {
        return Err(format!(
            "Kassiber only saves .md, .log or .jsonl files here (not .{ext})."
        ));
    }
    std::fs::write(path, contents).map_err(|e| e.to_string())
}

fn export_format(app: &mut App, tx: &mut Tx, format: &'static str) {
    let s = &app.reports.diag.logs;
    let redaction = if !s.redacted {
        "raw"
    } else if s.mask_amounts {
        "redacted-amounts"
    } else {
        "redacted"
    };
    let o = opts(s);
    let idx = filtered(s);
    let records: Vec<&Record> = idx.iter().map(|&i| &s.store.records[i]).collect();
    let h = header(
        app,
        &records,
        logstore::filter_label(s.level, s.module.as_deref(), &s.query.text, s.regex),
        redaction.into(),
    );
    let contents = logstore::export(&records, format, &o, &h);
    let options = rune::FileDialogOptions {
        title: t("review:logs.exportTitle"),
        file_name: Some(logstore::filename(format, redaction)),
        ..Default::default()
    }
    .filter(format.to_uppercase(), &[format]);
    tx.save_file(
        options,
        move |a: &mut App, result, _tx: &mut Tx| match result {
            Ok(Some(path)) => match write(&path, &contents) {
                // `toastopen`: owner request over the web (which confirms nothing): "File saved"
                // with Open / Show in Finder.
                Ok(()) => crate::toastopen::file_saved(a, &path, "logs-export"),
                Err(e) => a.toast(
                    ToastTone::Error,
                    &t("review:logs.exportFailedTitle"),
                    Some(e),
                    None,
                ),
            },
            Ok(None) => {}
            Err(e) => a.toast(
                ToastTone::Error,
                &t("review:logs.exportFailedTitle"),
                Some(e.to_string()),
                None,
            ),
        },
    );
}

fn start_export(app: &mut App, tx: &mut Tx, format: &'static str) {
    if !app.reports.diag.logs.redacted {
        app.reports.diag.logs.confirm_raw = Some(format);
        return;
    }
    export_format(app, tx, format);
}

fn bundle(app: &App, issue: &str) -> String {
    let s = &app.reports.diag.logs;
    let records: Vec<&Record> = s.store.records.iter().collect();
    let filter = format!(
        "support_bundle=all_records, ui_filter=({})",
        logstore::filter_label(s.level, s.module.as_deref(), &s.query.text, s.regex)
    );
    let h = header(app, &records, filter, s.support_mode.id().into());
    logstore::support_bundle(&records, issue, s.support_mode, &h)
}

fn export_bundle(app: &mut App, tx: &mut Tx) {
    let issue = app.reports.diag.logs.support_issue.text.trim().to_owned();
    if issue.is_empty() {
        app.toast(
            ToastTone::Warning,
            &t("review:logs.support.notExportedTitle"),
            Some(t("review:logs.support.notExportedBody")),
            None,
        );
        return;
    }
    let contents = bundle(app, &issue);
    let options = rune::FileDialogOptions {
        title: t("review:logs.exportSupportTitle"),
        file_name: Some(logstore::support_filename()),
        ..Default::default()
    }
    .filter("JSONL", &["jsonl"]);
    tx.save_file(
        options,
        move |a: &mut App, result, _tx: &mut Tx| match result {
            Ok(Some(path)) => match write(&path, &contents) {
                Ok(()) => {
                    a.reports.diag.logs.support_open = false;
                    crate::toastopen::file_saved(a, &path, "logs-support-export");
                }
                Err(e) => a.toast(
                    ToastTone::Error,
                    &t("review:logs.support.exportFailedTitle"),
                    Some(e),
                    None,
                ),
            },
            Ok(None) => {}
            Err(e) => a.toast(
                ToastTone::Error,
                &t("review:logs.support.exportFailedTitle"),
                Some(e.to_string()),
                None,
            ),
        },
    );
}

fn copy_last(app: &mut App, tx: &mut Tx) {
    let s = &app.reports.diag.logs;
    let o = opts(s);
    let idx = filtered(s);
    let tail = &idx[idx.len().saturating_sub(200)..];
    let text: Vec<String> = tail
        .iter()
        .map(|&i| logstore::format_record(&s.store.records[i], &o))
        .collect();
    let n = tail.len();
    crate::osleaks::clipboard::copy(app, tx, text.join("\n"));
    app.toast(
        ToastTone::Success,
        &t("review:logs.copyExcerptTitle"),
        Some(tr(
            "review:logs.copyExcerptBody",
            &[("count", &n.to_string())],
        )),
        None,
    );
}

// ---- View ----------------------------------------------------------------------------------

/// A `size-8` icon button: outline, or `secondary` with `border-primary` and a dot when active.
fn menu_button(
    tk: &Tk,
    key: &'static str,
    glyph: &'static rune::Icon,
    label: String,
    active: bool,
    open: bool,
    popup: Popup,
) -> El {
    let side = tk.r(2.0);
    let mut b = if active {
        row()
            .key(key)
            .size(side, side)
            .center()
            .radius(tk.r(0.375))
            .fill(tk.muted)
            .border(1.0, tk.primary)
            .hover(tk.muted.alpha(0.8))
    } else {
        button_surface(
            tk,
            row().key(key).size(side, side).center(),
            Variant::Outline,
        )
    };
    b = b
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .child(ic(glyph, tk.r(1.0), tk.fg));
    b = crate::ui::popup::trigger(b, crate::ui::popup::Kind::Menu, open, move |a: &mut App| {
        widgets::toggle(a, popup)
    });
    if active {
        b = b.child(
            row()
                .absolute(side - tk.r(0.25) - tk.r(0.375), tk.r(0.25))
                .child(crate::ui::dot(tk.r(0.375), tk.primary)),
        );
    }
    b
}

/// `ActiveFilterChip`: `h-7 rounded-sm border border-primary/40 bg-primary/10 px-2 font-mono
/// text-xs text-primary`, an × after.
fn chip(
    tk: &Tk,
    key: &'static str,
    aria: String,
    label: String,
    on: impl Fn(&mut App) + 'static,
) -> El {
    row()
        .key(key)
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.375))
        .hover(tk.accent)
        .role("button")
        .label(aria)
        .focusable(true)
        .on_press(on)
        .child(
            text(label)
                .font(tk.txs(400.0).mono())
                .color(tk.fg)
                .truncate()
                .shrink(),
        )
        .child(ic(&I::X, tk.r(0.75), tk.muted_fg))
}

/// The card header's controls: the search (substring or regex) and one menu for the filters and
/// the view settings.
fn header_controls(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.reports.diag.logs;
    let popup = app.reports.diag.popup;
    let active = has_filters(s) || s.regex || !s.redacted || s.mask_amounts;
    if popup == Some(Popup::LogFilter) {
        filter_menu(app, tk, layer);
    }
    row()
        .gap(tk.r(0.375))
        .center_y()
        .child(
            widgets::input(
                tk,
                &s.query,
                "logs-search",
                t("review:logsControls.searchPlaceholder"),
                t("review:logsControls.searchPlaceholder"),
                true,
                tk.r(2.0),
                |a| &mut a.reports.diag.logs.query,
            )
            .w(tk.r(13.0)),
        )
        .child(menu_button(
            tk,
            "logs-filter",
            &I::SLIDERS_HORIZONTAL,
            t("review:logsControls.filterLogsAria"),
            active,
            popup == Some(Popup::LogFilter),
            Popup::LogFilter,
        ))
}

/// The active filters as removable chips (`ActiveFilterChip`), or "All captured records".
fn chips(app: &App, tk: &Tk) -> El {
    let s = &app.reports.diag.logs;
    let q = s.query.text.trim().to_owned();
    let mut left = row()
        .min_w(0.0)
        .shrink()
        .gap(tk.r(0.25))
        .center_y()
        .flex_wrap();
    if has_filters(s) {
        if let Some(l) = s.level {
            left = left.child(chip(
                tk,
                "logs-chip-level",
                tr("review:logsControls.clearLevelAria", &[("level", l)]),
                tr(
                    "review:logsControls.levelChip",
                    &[("level", &l.to_uppercase())],
                ),
                |a| a.reports.diag.logs.level = None,
            ));
        }
        if let Some(m) = &s.module {
            left = left.child(chip(
                tk,
                "logs-chip-module",
                tr("review:logsControls.clearModuleAria", &[("module", m)]),
                tr("review:logsControls.moduleChip", &[("module", m)]),
                |a| a.reports.diag.logs.module = None,
            ));
        }
        if !q.is_empty() {
            let shown = format!(
                "{}{q}",
                if s.regex {
                    t("review:logsControls.searchRegexPrefix")
                } else {
                    String::new()
                }
            );
            left = left.child(chip(
                tk,
                "logs-chip-search",
                t("review:logsControls.clearSearchAria"),
                tr("review:logsControls.searchChip", &[("query", &shown)]),
                |a| {
                    a.reports.diag.logs.query = TextEdit::new("");
                    a.reports.diag.logs.regex = false;
                },
            ));
        }
    }
    left
}

fn filter_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let s = &app.reports.diag.logs;
    let counts = s.store.level_counts();
    let total = s.store.records.len();
    let mut e = vec![
        Entry::Label(t("review:logsControls.filterLogsLabel"), false),
        Entry::Check {
            key: "logs-lvl-all".into(),
            glyph: None,
            label: t("review:logsControls.allLevels"),
            mono: false,
            upper: false,
            trailing: Some(total.to_string()),
            checked: s.level.is_none(),
            on: Rc::new(|a, _| a.reports.diag.logs.level = None),
        },
    ];
    for (i, l) in LEVELS.iter().enumerate() {
        e.push(Entry::Check {
            key: format!("logs-lvl-{l}"),
            glyph: None,
            label: (*l).to_owned(),
            mono: false,
            upper: true,
            trailing: Some(counts[i].to_string()),
            checked: s.level == Some(*l),
            on: Rc::new(move |a, _| a.reports.diag.logs.level = Some(*l)),
        });
    }
    e.push(Entry::Sep);
    e.push(Entry::Label(t("review:logsControls.module"), false));
    e.push(Entry::Check {
        key: "logs-mod-all".into(),
        glyph: None,
        label: t("review:logsControls.allModules"),
        mono: false,
        upper: false,
        trailing: Some(total.to_string()),
        checked: s.module.is_none(),
        on: Rc::new(|a, _| a.reports.diag.logs.module = None),
    });
    for (i, (m, n)) in s.store.module_counts().into_iter().enumerate() {
        let checked = s.module.as_deref() == Some(m.as_str());
        let mm = m.clone();
        e.push(Entry::Check {
            key: format!("logs-mod-{i}"),
            glyph: None,
            label: m,
            mono: true,
            upper: false,
            trailing: Some(n.to_string()),
            checked,
            on: Rc::new(move |a, _| {
                let s = &mut a.reports.diag.logs;
                s.module = if s.module.as_deref() == Some(mm.as_str()) {
                    None
                } else {
                    Some(mm.clone())
                };
            }),
        });
    }
    e.extend(settings_entries(s));
    if has_filters(s) {
        e.push(Entry::Sep);
        e.push(Entry::Item {
            key: "logs-clear-filters".into(),
            glyph: Some(&I::X),
            label: t("review:logsControls.clearFilters"),
            on: Rc::new(|a, _| {
                let s = &mut a.reports.diag.logs;
                s.query = TextEdit::new("");
                s.regex = false;
                s.level = None;
                s.module = None;
            }),
        });
    }
    widgets::menu(
        tk,
        layer,
        "logs-filter-menu",
        "logs-filter",
        Placement::BottomEnd,
        Some(tk.r(16.0)),
        e,
    );
}

/// The view settings, after the filters in the same menu.
fn settings_entries(s: &State) -> Vec<Entry> {
    vec![
        Entry::Sep,
        Entry::Label(t("review:logsControls.logSettingsLabel"), false),
        Entry::Check {
            key: "logs-set-regex".into(),
            glyph: Some(&I::REGEX),
            label: t("review:logsControls.regexSearch"),
            mono: false,
            upper: false,
            trailing: None,
            checked: s.regex,
            on: Rc::new(|a, _| a.reports.diag.logs.regex ^= true),
        },
        Entry::Check {
            key: "logs-set-redacted".into(),
            glyph: Some(&I::SHIELD),
            label: t("review:logsControls.redactedView"),
            mono: false,
            upper: false,
            trailing: None,
            checked: s.redacted,
            on: Rc::new(|a, _| {
                let on = !a.reports.diag.logs.redacted;
                set_redacted(a, on);
            }),
        },
        Entry::Check {
            key: "logs-set-mask".into(),
            glyph: Some(&I::EYE),
            label: t("review:logsControls.maskAmounts"),
            mono: false,
            upper: false,
            trailing: None,
            checked: s.mask_amounts,
            on: Rc::new(|a, _| a.reports.diag.logs.mask_amounts ^= true),
        },
    ]
}

fn export_menu(tk: &Tk, layer: &mut Vec<El>) {
    let e = vec![
        Entry::Item {
            key: "logs-copy".into(),
            glyph: Some(&I::COPY),
            label: t("review:logs.copy200"),
            on: Rc::new(copy_last),
        },
        Entry::Sep,
        Entry::Item {
            key: "logs-exp-bundle".into(),
            glyph: Some(&I::FILE_ARCHIVE),
            label: t("review:logs.exportMenu.supportBundle"),
            on: Rc::new(|a, _| a.reports.diag.logs.support_open = true),
        },
        Entry::Item {
            key: "logs-exp-md".into(),
            glyph: Some(&I::FILE_TEXT),
            label: t("review:logs.exportMenu.markdown"),
            on: Rc::new(|a, tx| start_export(a, tx, "md")),
        },
        Entry::Item {
            key: "logs-exp-jsonl".into(),
            glyph: Some(&I::FILE_JSON),
            label: t("review:logs.exportMenu.jsonl"),
            on: Rc::new(|a, tx| start_export(a, tx, "jsonl")),
        },
        Entry::Item {
            key: "logs-exp-log".into(),
            glyph: Some(&I::FILE_TEXT),
            label: t("review:logs.exportMenu.log"),
            on: Rc::new(|a, tx| start_export(a, tx, "log")),
        },
        Entry::Sep,
        Entry::Item {
            key: "logs-clear".into(),
            glyph: Some(&I::TRASH_2),
            label: t("review:logs.clearLogsAria"),
            on: Rc::new(|a, _| a.reports.diag.logs.store.clear()),
        },
    ];
    widgets::menu(
        tk,
        layer,
        "logs-export-menu",
        "logs-export",
        Placement::BottomEnd,
        None,
        e,
    );
}

/// One record: the panel's log line (`LogsSurface`'s `LogLine`) behind a disclosure chevron;
/// open, the record's JSON underneath.
fn log_row(tk: &Tk, r: &Record, o: &Opts, open: bool) -> El {
    let rr = rendered(r, o);
    let id = r.id.clone();
    let location = if rr.line > 0 {
        format!("{}:{}:{}", rr.module, rr.file, rr.line)
    } else {
        format!("{}:{}", rr.module, rr.file)
    };
    let lead = row()
        .h(tk.r(1.125))
        .center_y()
        .child(widgets::chevron(tk, open, tk.r(0.75)));
    let line = super::tail::line(tk, &rr, format!("logs-row-{id}"), Some(lead))
        .help(format!("{} {location}", rr.ts))
        .role("button")
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| {
            let e = &mut a.reports.diag.logs.expanded;
            if !e.remove(&id) {
                e.insert(id.clone());
            }
        });
    let mut article = col().w_full().child(line);
    if open {
        article = article.child(
            col()
                .w_full()
                .max_h(tk.r(24.0))
                .ml(tk.r(1.5))
                .mr(tk.r(0.75))
                .my(tk.r(0.25))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .fill(tk.background)
                .border(1.0, tk.border.alpha(tk.border.a * 0.7))
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(super::ordjson::pretty(&rr.to_j()))
                        .font(super::tail::mono(tk, 400.0))
                        .color(tk.fg)
                        .wrap_anywhere()
                        .wrap(),
                ),
        );
    }
    article
}

fn support_dialog(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let s = &app.reports.diag.logs;
    let close = |a: &mut App| a.reports.diag.logs.support_open = false;
    let issue = if s.support_issue.text.is_empty() {
        t("review:logs.support.previewIssue")
    } else {
        s.support_issue.text.clone()
    };
    let preview: String = bundle(app, &issue)
        .split('\n')
        .take(PREVIEW_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let mode_button = |key: &'static str, mode: Mode, title: String, desc: String| {
        let active = s.support_mode == mode;
        row()
            .key(key)
            .grow(1.0)
            .min_w(0.0)
            .min_h(tk.r(6.0))
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, if active { tk.primary } else { tk.border })
            .fill(if active {
                tk.primary.alpha(0.1)
            } else {
                tk.muted.alpha(0.2)
            })
            .hover(if active {
                tk.primary.alpha(0.1)
            } else {
                tk.muted.alpha(0.5)
            })
            .role("button")
            .checked(active)
            .label(title.clone())
            .focusable(true)
            .on_press(move |a: &mut App| a.reports.diag.logs.support_mode = mode)
            .child(
                col()
                    .w_full()
                    .items_start()
                    .child(text(title).font(tk.tsm(600.0)).color(if active {
                        tk.primary
                    } else {
                        tk.fg
                    }))
                    .child(
                        text(desc)
                            .font(tk.txs(400.0).lh(tk.r(1.25)))
                            .color(tk.muted_fg)
                            .wrap()
                            .mt(tk.r(0.25))
                            .w_full(),
                    ),
            )
    };
    let k = crate::txdetail::widgets::form_kit(tk);
    let area = TextArea::new(&s.support_issue)
        .placeholder(t("review:logs.support.issuePlaceholder"))
        .label(t("review:logs.support.issueDescription"))
        .on_edit(|a: &mut App| &mut a.reports.diag.logs.support_issue)
        .build(&k)
        .key("logs-issue")
        .w_full()
        .min_h(tk.r(6.0))
        .border(1.0, tk.input)
        .radius(tk.r(0.375));
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    // `max-h-[88vh] grid-rows-[auto_minmax(0,1fr)_auto]`: the body scrolls inside what the
    // header (`py-4`, title + description) and footer (`py-4`, `h-9` buttons) leave.
    let chrome =
        2.0 * tk.r(1.0) + tk.r(1.125) + tk.r(0.5) + tk.r(1.25) + 2.0 * tk.r(1.0) + tk.r(2.25) + 4.0;
    let body = col()
        .w_full()
        .max_h(win_h * 0.88 - chrome)
        .px(tk.r(1.25))
        .py(tk.r(1.0))
        .gap(tk.r(1.0))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    text(t("review:logs.support.issueDescription"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(area),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    text(t("review:logs.support.bundleMode"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.5))
                        .child(mode_button(
                            "logs-mode-high",
                            Mode::HighSignal,
                            t("review:logs.support.highSignalTitle"),
                            t("review:logs.support.highSignalDescription"),
                        ))
                        .child(mode_button(
                            "logs-mode-public",
                            Mode::PublicSafe,
                            t("review:logs.support.publicSafeTitle"),
                            t("review:logs.support.publicSafeDescription"),
                        )),
                ),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.5))
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.75))
                        .center_y()
                        .justify_between()
                        .child(
                            text(t("review:logs.support.preview"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(
                            text(tc(
                                "review:logs.support.firstLines",
                                PREVIEW_LINES,
                                &[("count", &PREVIEW_LINES.to_string())],
                            ))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                        ),
                )
                .child(
                    col()
                        .key("logs-bundle-preview")
                        .w_full()
                        .max_h(tk.r(20.0))
                        .padding(tk.r(0.75))
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border)
                        .fill(tk.muted.alpha(0.4))
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .child(
                            text(preview)
                                .font(tk.txs(400.0).mono())
                                .color(tk.fg)
                                .wrap_anywhere()
                                .wrap(),
                        ),
                ),
        );
    let can = !s.support_issue.text.trim().is_empty();
    let mut save = button(
        tk,
        "logs-bundle-save",
        Variant::Default,
        Size::Default,
        None,
        &t("review:logs.support.saveBundle"),
        can,
    );
    if can {
        save = save.on_press_tx(export_bundle);
    }
    // `max-w-3xl` loses to DialogContent's own `sm:max-w-lg` (32 rem).
    let dw = tk.r(32.0);
    let dialog = col()
        .key("logs-support")
        .w(dw)
        .max_h(win_h * 0.88)
        .radius(tk.r(0.5))
        .fill(tk.background)
        .border(1.0, tk.border)
        .clip_rounded(tk.r(0.5))
        .role("dialog")
        .label(t("review:logs.support.title"))
        .description(t("review:logs.support.description"))
        .modal(true, close)
        .on_press(|_a: &mut App| {})
        .child(
            col()
                .w_full()
                .px(tk.r(1.25))
                .py(tk.r(1.0))
                .pr(tk.r(3.0))
                .gap(tk.r(0.5))
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .child(
                    text(t("review:logs.support.title"))
                        .font(tk.tlg(600.0).lh(tk.r(1.125)))
                        .color(tk.fg),
                )
                .child(
                    text(t("review:logs.support.description"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        )
        .child(body)
        .child(
            row()
                .w_full()
                .px(tk.r(1.25))
                .py(tk.r(1.0))
                .gap(tk.r(0.5))
                .justify_end()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    button(
                        tk,
                        "logs-bundle-cancel",
                        Variant::Outline,
                        Size::Default,
                        None,
                        &t("common:actions.cancel"),
                        true,
                    )
                    .on_press(close),
                )
                .child(save),
        )
        .child(
            row()
                .key("logs-support-close")
                .absolute(dw - tk.r(1.0) - tk.r(1.0), tk.r(1.0))
                .size(tk.r(1.0), tk.r(1.0))
                .center()
                .opacity(0.7)
                .role("button")
                .label(t("common:actions.close"))
                .focusable(true)
                .on_press(close)
                .child(ic(&I::X, tk.r(1.0), tk.fg)),
        );
    layer.push(
        row()
            .key("logs-support-wrap")
            .label(t("common:actions.close"))
            .pin(
                Some(0.0),
                Some(crate::shell::window_top_inset()),
                Some(0.0),
                Some(0.0),
            )
            .center()
            .fill(Color::BLACK.alpha(0.5))
            .on_press(close)
            .child(dialog),
    );
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.reports.diag.logs;
    let o = opts(s);
    let idx = filtered(s);
    let limit = s.render_limit.min(MAX_RENDERED);
    let visible = &idx[idx.len().saturating_sub(limit)..];
    let popup = app.reports.diag.popup;
    // One toolbar: the search and its active filters; the count (the RAM note on hover); the
    // filter menu and Export (its menu also copies the newest records and clears). The log
    // fills the rest; the page title already names it.
    let export = button(
        tk,
        "logs-export",
        Variant::Outline,
        Size::Sm,
        Some(&I::DOWNLOAD),
        &t("common:actions.export"),
        true,
    )
    .help(t("review:logs.ramNote"));
    let export = crate::ui::popup::trigger(
        export,
        crate::ui::popup::Kind::Menu,
        popup == Some(Popup::LogExport),
        |a: &mut App| widgets::toggle(a, Popup::LogExport),
    );
    if popup == Some(Popup::LogExport) {
        export_menu(tk, layer);
    }
    let mut count = tc(
        "review:logs.recordsCount",
        idx.len(),
        &[("count", &idx.len().to_string())],
    );
    if visible.len() < idx.len() {
        count = format!(
            "{count} · {}",
            tc(
                "review:logs.renderingNewest",
                visible.len(),
                &[("count", &visible.len().to_string())],
            )
        );
    }
    let toolbar = row()
        .key("logs-toolbar")
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .child(header_controls(app, tk, layer))
        .children(has_filters(s).then(|| chips(app, tk)))
        .child(rune::el().grow(1.0))
        .child(
            text(count)
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg)
                .truncate(),
        )
        .child(export);
    let mut card = widgets::surface(tk)
        .key("logs-card")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .clip_rounded(tk.r(0.75));
    if !s.redacted {
        let time = s
            .raw_until
            .map(|(_, wall)| time::plain_time(wall))
            .unwrap_or_else(|| t("review:logs.rawWarningSoon"));
        card = card.child(
            row()
                .w_full()
                .px(tk.card_pad())
                .py(tk.r(0.5))
                .fill(tk.destructive.alpha(0.1))
                .child(
                    text(tr("review:logs.rawWarning", &[("time", &time)]))
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap(),
                ),
        );
    }
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let page_h = win_h - 40.0 - tk.r(0.375) - 2.0;
    let mut viewport = col()
        .key("logs-viewport")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .py(tk.r(0.25))
        .fill(tk.muted.alpha(0.2))
        .role("log")
        .label(t("nav:book.logs"))
        .scroll(ScrollSpec {
            stick_bottom: s.autoscroll,
        })
        .on_scroll_offset(|at: ScrollPos| {
            AT_BOTTOM.with(|c| c.set(at.max[1] - at.offset[1] < 12.0));
            if at.offset[1] < 24.0 && at.max[1] > 0.0 {
                NEAR_TOP.with(|c| c.set(true));
            }
        });
    if visible.is_empty() {
        let empty_filters = !s.store.records.is_empty() && idx.is_empty();
        let body = has_filters(s).then(|| {
            let mut parts = Vec::new();
            if let Some(l) = s.level {
                parts.push(tr("review:logs.filterLevel", &[("level", l)]));
            }
            if let Some(m) = &s.module {
                parts.push(tr("review:logs.filterModule", &[("module", m)]));
            }
            if !s.query.text.is_empty() {
                parts.push(tr("review:logs.filterSearch", &[("query", &s.query.text)]));
            }
            parts.join(" · ")
        });
        let title = if empty_filters {
            t("review:logs.noMatching")
        } else {
            t("review:logs.waiting")
        };
        viewport = viewport.child(super::surfaces::empty(
            tk,
            &I::SCROLL_TEXT,
            super::surfaces::EmptyTone::Muted,
            title,
            body,
            None,
        ));
    } else {
        let mut rows = col().w_full();
        for &i in visible {
            let r = &s.store.records[i];
            rows = rows.child(log_row(tk, r, &o, s.expanded.contains(&r.id)));
        }
        viewport = viewport.child(rows);
    }
    let mut body = col().w_full().grow(1.0).min_h(0.0).child(viewport);
    if !s.autoscroll && s.new_while_paused > 0 {
        let label = tr(
            "review:logs.jumpToLatest",
            &[("count", &s.new_while_paused.to_string())],
        );
        let [vw, vh] = cx.layout_size("logs-viewport").unwrap_or([800.0, 400.0]);
        let bw = tk.r(1.5) + rune::text::measure(&label, &[], tk.tsm(500.0), None).0;
        body = body.child(
            row()
                .absolute((vw - bw) / 2.0, vh - tk.r(1.0) - tk.r(2.0))
                .child(
                    button(
                        tk,
                        "logs-jump",
                        Variant::Default,
                        Size::Sm,
                        None,
                        &label,
                        true,
                    )
                    .box_shadow(rune::Shadow {
                        y: 10.0,
                        blur: 15.0,
                        color: Color::BLACK.alpha(0.2),
                        ..Default::default()
                    })
                    .on_press_tx(|a: &mut App, tx: &mut Tx| {
                        a.reports.diag.logs.autoscroll = true;
                        a.reports.diag.logs.new_while_paused = 0;
                        AT_BOTTOM.with(|c| c.set(true));
                        tx.scroll_to_bottom("logs-viewport");
                    }),
                ),
        );
    }
    card = card.child(body);
    if s.support_open {
        support_dialog(app, tk, layer);
    }
    if let Some(format) = s.confirm_raw {
        widgets::confirm(
            tk,
            layer,
            "logs-raw-confirm",
            t("review:logs.exportRawConfirm"),
            move |a: &mut App, tx: &mut Tx| {
                a.reports.diag.logs.confirm_raw = None;
                export_format(a, tx, format);
            },
            |a: &mut App| a.reports.diag.logs.confirm_raw = None,
        );
    }
    widgets::shell(tk)
        .h(page_h)
        .min_h(0.0)
        .child(toolbar)
        .child(card)
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_only_allowed_extensions() {
        let dir = std::env::temp_dir().join(format!("kb-diag-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(super::write(&dir.join("a.md"), "x").is_ok());
        assert!(super::write(&dir.join("a.sh"), "x").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod transient_tests {
    use super::*;

    #[test]
    fn leaving_logs_resets_the_view_but_keeps_the_ring() {
        let mut s = State {
            level: Some("error"),
            redacted: false,
            raw_until: Some((300.0, 0.0)),
            query: TextEdit::new("needle"),
            regex: true,
            autoscroll: false,
            support_open: true,
            seen_count: 7,
            ..State::default()
        };
        s.store.next_poll = 42.0;
        s.leave();
        assert_eq!(s.level, None);
        assert!(
            s.redacted
                && s.raw_until.is_none()
                && s.query.text.is_empty()
                && !s.regex
                && s.autoscroll
                && !s.support_open
        );
        assert_eq!(s.seen_count, 7);
        assert_eq!(s.store.next_poll, 42.0);
    }
}
