//! The Transactions screen (ui-tauri/src/routes/Transactions.tsx + components/transactions/dashboard).
//!
//! `model` holds rows, filters, periods and the server args; `workbench` the period nav, metric
//! tiles and flow chart; `table` the table card (toolbar, filters, chips, rows, footer, row menu);
//! `export` the export dialog. State lives in `TxUi`; loads start from `poll`, which also reads
//! the URL-style params carried in `app.route` (`/transactions?period=…&tx=…`).

mod export;
mod menu;
pub mod model;
mod style;
mod table;
mod workbench;

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use rune::{Cx, El, KeyEvent, Tx, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply, ToastTone};
use crate::i18n::t;
use crate::theme::Tk;
use crate::ui::surface;
use model::*;

/// Which transactions popup is open (one at a time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Year,
    Filters,
    FiltersSub(&'static str),
    StatusHeader,
    NetworkHeader,
    RowMenu(String),
    ExportFormat,
    ExportPeriod,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ListState {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

/// What the workbench and the period nav read from `ui.transactions.dashboard`.
#[derive(Clone, Debug, Default)]
pub struct Dash {
    pub years: Vec<BookYear>,
    pub earliest: String,
    pub latest: String,
    pub timezone: String,
    pub auto: Option<Period>,
    pub summary: Value,
    pub buckets: Vec<Value>,
    pub bucket_size: String,
    pub swap_candidate_total: Option<f64>,
}

pub struct TxUi {
    pub period: Period,
    pub filters: Filters,
    pub rows: Vec<Row>,
    /// Indices into `rows` after the client filter and sort.
    pub filtered: Vec<usize>,
    pub total: usize,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub list: ListState,
    pub refreshing: bool,
    pub loading_more: bool,
    /// The args (JSON) of the rows on screen; a change starts a fresh load.
    pub list_key: String,
    /// In-flight list requests in send order: (args key, is a next-page load). None: asked
    /// before a book switch, dropped when it answers.
    pub queue: VecDeque<(Option<String>, bool)>,
    pub dash: Option<Dash>,
    pub dash_key: String,
    pub dash_queue: VecDeque<Option<String>>,
    pub dash_failed: bool,
    pub candidates: Candidates,
    /// transaction id → loan role.
    pub loans: HashMap<String, String>,
    pub loans_requested: bool,
    pub popup: Option<Popup>,
    pub expanded: bool,
    pub chart_count: bool,
    pub chart_external: bool,
    pub hover_bar: Option<(String, Segment)>,
    pub route_seen: String,
    pub confirm_exclude: Option<String>,
    pub export: export::ExportUi,
    /// Ids a book refresh brought in, highlighted until `highlight_until`.
    pub highlighted: HashSet<String>,
    pub highlight_until: f64,
    pub seen_tx_count: Option<f64>,
    pub book: Option<String>,
    /// UI CPU of the last refilter/sort (µs), for the pilot's measurements.
    pub last_refilter_us: u128,
    /// Scroll the table back to the top on the next poll (filter or sort changed).
    pub scroll_top: bool,
    /// Bumped whenever the loaded rows change, so memoised table rows rebuild.
    pub generation: u64,
}

impl Default for TxUi {
    fn default() -> Self {
        TxUi {
            period: Period::Auto,
            filters: Filters::default(),
            rows: vec![],
            filtered: vec![],
            total: 0,
            next_cursor: None,
            has_more: false,
            list: ListState::Idle,
            refreshing: false,
            loading_more: false,
            list_key: String::new(),
            queue: VecDeque::new(),
            dash: None,
            dash_key: String::new(),
            dash_queue: VecDeque::new(),
            dash_failed: false,
            candidates: Candidates::default(),
            loans: HashMap::new(),
            loans_requested: false,
            popup: None,
            expanded: false,
            chart_count: false,
            chart_external: false,
            hover_bar: None,
            route_seen: String::new(),
            confirm_exclude: None,
            export: Default::default(),
            highlighted: HashSet::new(),
            highlight_until: 0.0,
            seen_tx_count: None,
            book: None,
            last_refilter_us: 0,
            scroll_top: false,
            generation: 0,
        }
    }
}

impl TxUi {
    /// The period actually shown: `auto` resolves through the daemon's `history.autoPeriod`.
    pub fn resolved(&self) -> Period {
        match self.period {
            Period::Auto => self
                .dash
                .as_ref()
                .and_then(|d| d.auto)
                .unwrap_or(Period::Range(Range::Y1)),
            p => p,
        }
    }

    pub fn years(&self) -> &[BookYear] {
        self.dash
            .as_ref()
            .map(|d| d.years.as_slice())
            .unwrap_or(&[])
    }

    /// Re-runs the client filter and sort over the loaded rows.
    pub fn refilter(&mut self, fiat: bool) {
        let started = Instant::now();
        let mut idx: Vec<usize> = (0..self.rows.len())
            .filter(|&i| matches(&self.rows[i], &self.filters, &self.candidates))
            .collect();
        sort_indices(
            &self.rows,
            &mut idx,
            self.filters.sort,
            fiat,
            &self.candidates,
        );
        self.filtered = idx;
        self.last_refilter_us = started.elapsed().as_micros();
    }

    /// Filter options of the Wallet submenu: the distinct legs of loaded rows' wallets.
    pub fn wallet_options(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for r in &self.rows {
            for l in r.wallet_legs() {
                if !v.contains(&l) {
                    v.push(l);
                }
            }
        }
        if let Some(k) = self.filters.leg_wallet()
            && !v.iter().any(|x| x == k)
        {
            v.push(k.to_owned());
        }
        v.sort_by_key(|a| a.to_lowercase());
        v
    }
}

pub fn fiat_mode(app: &App) -> bool {
    app.prefs.currency == crate::app::Currency::Fiat
}

/// Applies a filter change: refilter, sync the URL, close popups.
pub fn changed(app: &mut App) {
    let fiat = fiat_mode(app);
    app.txs.refilter(fiat);
    app.txs.popup = None;
    app.txs.scroll_top = true;
    write_url(app);
}

// ---- URL ----------------------------------------------------------------------------------

fn query_params(route: &str) -> Vec<(String, String)> {
    let q = route.split_once('?').map(|(_, q)| q).unwrap_or("");
    let q = q.split('#').next().unwrap_or("");
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(v) => {
                    out.push(v);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|c| match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                (c as char).to_string()
            }
            _ => format!("%{c:02X}"),
        })
        .collect()
}

/// Reads the route's params into the filter state (the `TransactionsDashboard` URL codec). An
/// absent param keeps the period and sort (they are remembered); other filters follow the URL.
fn apply_route(app: &mut App) {
    let route = app.route.0.clone();
    app.txs.route_seen = route.clone();
    let p = query_params(&route);
    let get = |k: &str| {
        p.iter()
            .find(|(x, _)| x == k)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.trim().is_empty())
    };
    if let Some(per) = get("period").and_then(|v| Period::parse(&v)) {
        app.txs.period = per;
    }
    let sort = app.txs.filters.sort;
    let mut f = Filters {
        quick: get("quick").and_then(|v| Quick::parse(&v)),
        ..Filters::default()
    };
    if let Some(w) = get("wallet") {
        f.breakdown = Some(Breakdown {
            network: false,
            key: w.trim().to_owned(),
            leg: true,
        });
    } else if let Some(n) = get("network") {
        f.breakdown = Some(Breakdown {
            network: true,
            key: n,
            leg: false,
        });
    }
    if let Some(ids) = get("txids") {
        let mut v: Vec<String> = Vec::new();
        for id in ids.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            if !v.iter().any(|x| x == id) && v.len() < 128 {
                v.push(id.to_owned());
            }
        }
        f.txids = v;
    }
    f.status = get("status").and_then(|v| Status::ALL.into_iter().find(|s| s.param() == v));
    f.flow = get("flow").and_then(|v| Flow::parse(&v));
    f.payment = get("payment").filter(|v| PAYMENTS.contains(&v.as_str()));
    f.with_fees = get("fees").is_some_and(|v| matches!(v.as_str(), "with-fees" | "true" | "1"));
    f.sort = match (get("sort").as_deref(), get("order").as_deref()) {
        (Some(k @ ("date" | "amount")), Some(o @ ("asc" | "desc"))) => Some((
            if k == "date" {
                SortKey::Date
            } else {
                SortKey::Amount
            },
            if o == "asc" { Dir::Asc } else { Dir::Desc },
        )),
        _ => sort,
    };
    app.txs.filters = f;
    let fiat = fiat_mode(app);
    app.txs.refilter(fiat);
    let tx_param = get("tx")
        .or_else(|| get("transaction"))
        .or_else(|| get("transactionId"));
    // Normalise the route (legacy keys dropped, period always written) without re-reading it.
    write_url(app);
    if let Some(id) = tx_param {
        let tab = get("tab")
            .filter(|t| ["details", "classify", "pricing", "tax", "linked"].contains(&t.as_str()))
            .unwrap_or_else(|| "details".into());
        set_detail_param(app, Some(&id), &tab);
        crate::txdetail::open(app, &id, &tab);
    }
}

fn build_route(q: &[(String, String)]) -> String {
    let s = q
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("/transactions?{s}")
}

/// Writes the filter state back into `app.route` in place (`history.replaceState`), keeping
/// the detail params.
pub fn write_url(app: &mut App) {
    let keep: Vec<(String, String)> = query_params(&app.route.0)
        .into_iter()
        .filter(|(k, _)| ["tx", "tab", "qrow"].contains(&k.as_str()))
        .collect();
    let ui = &app.txs;
    let f = &ui.filters;
    let mut q: Vec<(String, String)> = vec![("period".into(), ui.period.param())];
    if let Some(x) = f.quick {
        q.push(("quick".into(), x.param().into()));
    }
    if let Some(b) = &f.breakdown {
        q.push((
            if b.network { "network" } else { "wallet" }.into(),
            b.key.clone(),
        ));
    }
    if !f.txids.is_empty() {
        q.push(("txids".into(), f.txids.join(",")));
    }
    if let Some(s) = f.status {
        q.push(("status".into(), s.param().into()));
    }
    if let Some(fl) = f.flow {
        q.push(("flow".into(), fl.param().into()));
    }
    if let Some(p) = &f.payment {
        q.push(("payment".into(), p.clone()));
    }
    if f.with_fees {
        q.push(("fees".into(), "with-fees".into()));
    }
    if let Some((k, d)) = f.sort {
        q.push((
            "sort".into(),
            if k == SortKey::Date { "date" } else { "amount" }.into(),
        ));
        q.push(("order".into(), d.param().into()));
    }
    q.extend(keep);
    app.route.0 = build_route(&q);
    app.txs.route_seen = app.route.0.clone();
}

/// Sets or clears the detail params (`updateTransactionDetailParams`).
pub fn set_detail_param(app: &mut App, id: Option<&str>, tab: &str) {
    let mut q: Vec<(String, String)> = query_params(&app.route.0)
        .into_iter()
        .filter(|(k, _)| {
            !["tx", "transaction", "transactionId", "tab", "qrow"].contains(&k.as_str())
        })
        .collect();
    if let Some(id) = id {
        q.push(("tx".into(), id.to_owned()));
        if tab != "details" {
            q.push(("tab".into(), tab.to_owned()));
        }
    }
    app.route.0 = build_route(&q);
    app.txs.route_seen = app.route.0.clone();
}

/// Opens the detail panel on a row (`openTransactionDetail`).
pub fn open_detail(app: &mut App, id: &str, tab: &str) {
    app.txs.popup = None;
    set_detail_param(app, Some(id), tab);
    crate::txdetail::open(app, id, tab);
    // "Save & next" walks the table's current order.
    let ids: Vec<String> = app
        .txs
        .filtered
        .iter()
        .map(|&i| app.txs.rows[i].id.clone())
        .collect();
    crate::txdetail::set_nav_list(app, ids);
}

// ---- Loading ------------------------------------------------------------------------------

fn dash_args(ui: &TxUi) -> Value {
    let resolved = ui.resolved();
    let mut m = serde_json::Map::new();
    let period = match resolved {
        // The daemon knows calendar ranges by their window; `since`/`until` below carry it.
        Period::Year(_) | Period::Range(crate::transactions::model::Range::Ytd) => {
            "1year".to_owned()
        }
        p => p.param(),
    };
    m.insert("period".into(), json!(period));
    if let Some(w) = period_window(resolved, ui.years()) {
        m.insert("since".into(), json!(w.since));
        m.insert("until".into(), json!(w.until));
    }
    m.insert("resolveAuto".into(), json!(ui.period == Period::Auto));
    m.insert("timezone".into(), json!("UTC"));
    if let Some(w) = ui.filters.leg_wallet() {
        m.insert("wallet".into(), json!(w));
    }
    Value::Object(m)
}

/// Leaving `/transactions` (`crate::shell::on_leave`): `TransactionsDashboard` unmounts, so its
/// `useState`s start over next visit: the expanded table, the chart's metric and mode, open
/// menus and prompts, and the export dialog's choices (format, period, wallets). Filters live
/// in the URL, the sort in `kb.ui`, the reads in React Query's cache: kept. A running export
/// keeps its dialog until it answers.
pub fn on_leave(app: &mut App) {
    let ui = &mut app.txs;
    ui.popup = None;
    ui.confirm_exclude = None;
    ui.expanded = false;
    ui.chart_count = false;
    ui.chart_external = false;
    ui.hover_bar = None;
    if !ui.export.busy {
        let e = std::mem::take(&mut ui.export);
        ui.export = export::ExportUi {
            queue: e.queue,
            wallets: e.wallets,
            viewer_tz: e.viewer_tz,
            pending_save: e.pending_save,
            ..Default::default()
        };
    }
}

/// Starts whatever the screen needs. Called every frame while Transactions is shown.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if app.route.path() != "/transactions" {
        return false;
    }
    if app.txs.scroll_top {
        app.txs.scroll_top = false;
        tx.scroll_to_instant("txs-scroll", 0.0, 0.0);
    }
    let changed = step(app);
    export::poll(app, tx) || changed
}

/// Headless runs render on a virtual clock: load the screen's reads before frame 0 (scenes).
/// With `all_pages`, follows the cursor to the last page.
pub fn prime(app: &mut App, all_pages: bool) {
    let deadline = Instant::now() + std::time::Duration::from_secs(30);
    loop {
        step(app);
        if all_pages && app.txs.queue.is_empty() && app.txs.has_more {
            load_more(app);
        }
        let idle =
            app.txs.queue.is_empty() && app.txs.dash_queue.is_empty() && app.pending.is_empty();
        if (idle && (!all_pages || !app.txs.has_more) && !app.txs.list_key.is_empty())
            || Instant::now() > deadline
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        app.pump();
    }
}

/// The work of `poll` that needs no `Tx`.
fn step(app: &mut App) -> bool {
    if app.route.path() != "/transactions" {
        return false;
    }
    let mut changed = false;
    // A book switch resets the screen's per-book state (the sort is remembered globally).
    let book = app
        .read("ui.profiles.snapshot")
        .and_then(|p| p.get("activeProfileId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if book.is_some() && app.txs.book.is_some() && book != app.txs.book {
        let sort = app.txs.filters.sort;
        let old = std::mem::take(&mut app.txs);
        app.txs = TxUi {
            filters: Filters {
                sort,
                ..Filters::default()
            },
            // Reads still out keep their place, so their answers are dropped rather than taken
            // as the new book's (the args do not name the book).
            queue: old
                .queue
                .into_iter()
                .map(|(_, more)| (None, more))
                .collect(),
            dash_queue: old.dash_queue.iter().map(|_| None).collect(),
            ..TxUi::default()
        };
        changed = true;
    }
    if book.is_some() {
        app.txs.book = book;
    }
    if app.route.0 != app.txs.route_seen {
        apply_route(app);
        changed = true;
    }
    if !app.txs.loans_requested {
        app.txs.loans_requested = true;
        app.send("ui.loans.list", Value::Null, "txs.loans");
    }
    // A book refresh that changed the transaction count reloads, highlighting the new rows.
    let count = app
        .snapshot()
        .and_then(|s| s.status.as_ref())
        .and_then(|s| s.transaction_count);
    if count.is_some()
        && app.txs.seen_tx_count.is_some()
        && count != app.txs.seen_tx_count
        && !app.txs.rows.is_empty()
    {
        app.txs.list_key.clear();
        app.txs.dash_key.clear();
        app.txs.highlight_until = -1.0; // armed: the next fresh page marks its new ids
    }
    if count.is_some() {
        app.txs.seen_tx_count = count;
    }
    if !app.txs.highlighted.is_empty()
        && app.txs.highlight_until > 0.0
        && app.clock > app.txs.highlight_until
    {
        app.txs.highlighted.clear();
        changed = true;
    }
    let dash = dash_args(&app.txs).to_string();
    if dash != app.txs.dash_key {
        app.txs.dash_key = dash.clone();
        app.txs.dash_queue.push_back(Some(dash.clone()));
        app.send(
            "ui.transactions.dashboard",
            serde_json::from_str(&dash).unwrap_or(Value::Null),
            "txs.dash",
        );
    }
    // `auto` waits for the daemon's resolution before the first list load.
    let waiting_auto =
        app.txs.period == Period::Auto && app.txs.dash.is_none() && !app.txs.dash_failed;
    if !waiting_auto {
        let fiat = fiat_mode(app);
        let ui = &app.txs;
        let args =
            list_args(&ui.filters, ui.resolved(), ui.years(), fiat, &ui.candidates).to_string();
        if args != app.txs.list_key {
            app.txs.list_key = args.clone();
            app.txs.queue.push_back((Some(args.clone()), false));
            app.txs.refreshing = !app.txs.rows.is_empty();
            app.txs.loading_more = false;
            if app.txs.rows.is_empty() {
                app.txs.list = ListState::Loading;
            }
            app.send(
                "ui.transactions.list",
                serde_json::from_str(&args).unwrap_or(Value::Null),
                "txs.list",
            );
            changed = true;
        }
    }
    changed
}

/// Loads the next page (infinite scroll).
pub fn load_more(app: &mut App) {
    let ui = &app.txs;
    if !ui.has_more || ui.loading_more || ui.refreshing || app.busy.syncing {
        return;
    }
    let Some(cursor) = ui.next_cursor.clone() else {
        return;
    };
    let mut args: Value = serde_json::from_str(&ui.list_key).unwrap_or(json!({}));
    args["cursor"] = json!(cursor);
    let key = ui.list_key.clone();
    app.txs.loading_more = true;
    app.txs.queue.push_back((Some(key), true));
    app.send("ui.transactions.list", args, "txs.list");
}

/// Re-reads after a mutation (the web invalidates every daemon query).
pub fn refresh_all(app: &mut App) {
    app.txs.list_key.clear();
    app.txs.dash_key.clear();
    app.txs.loans_requested = false;
    app.refresh_reads();
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "txs.list" => on_list(app, r),
        "txs.dash" => on_dash(app, r),
        "txs.loans" if r.error.is_none() => {
            app.txs.loans = r.data["marks"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            Some((
                                m["transaction_id"].as_str()?.to_owned(),
                                m["role"].as_str().unwrap_or("collateral_lock").to_owned(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
        "txs.loan_mark" | "txs.loan_unmark" => {
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("transactions:notification.loanMarkFailed.title"),
                    Some(e.clone()),
                    Some("loan-mark"),
                ),
                None if tag == "txs.loan_mark" => app.toast(
                    ToastTone::Success,
                    &t("transactions:notification.loanMarked.title"),
                    Some(t("transactions:notification.loanMarked.body")),
                    Some("loan-mark"),
                ),
                None => app.toast(
                    ToastTone::Success,
                    &t("transactions:notification.loanUnmarked.title"),
                    Some(t("transactions:notification.loanUnmarked.body")),
                    Some("loan-mark"),
                ),
            }
            refresh_all(app);
        }
        "txs.exclude" => {
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("common:state.somethingWentWrong"),
                    Some(e.clone()),
                    Some("tx-exclude"),
                ),
                None => app.toast(
                    ToastTone::Info,
                    &t("transactions:notification.transactionExcluded.title"),
                    Some(t("transactions:notification.transactionExcluded.body")),
                    Some("tx-exclude"),
                ),
            }
            refresh_all(app);
        }
        other if other.starts_with("txs.export") => export::on_reply(app, other, r),
        _ => {}
    }
}

fn on_dash(app: &mut App, r: &Reply) {
    let key = app.txs.dash_queue.pop_front().flatten();
    if key.as_ref() != Some(&app.txs.dash_key) {
        return;
    }
    if r.error.is_some() {
        app.txs.dash_failed = true;
        return;
    }
    let d = &r.data;
    let h = &d["history"];
    let years = h["years"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|y| BookYear {
                    year: y["year"].as_i64().unwrap_or(0) as i32,
                    count: y["count"].as_u64().unwrap_or(0) as usize,
                    since: y["since"].as_str().unwrap_or("").to_owned(),
                    until: y["until"].as_str().unwrap_or("").to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    let auto = h["autoPeriod"]
        .as_str()
        .and_then(Period::parse)
        .or_else(|| app.txs.dash.as_ref().and_then(|x| x.auto));
    let candidates: Vec<Value> = d["candidates"].as_array().cloned().unwrap_or_default();
    app.txs.candidates = Candidates::from_dashboard(&candidates);
    app.txs.dash = Some(Dash {
        years,
        earliest: h["earliest"].as_str().unwrap_or("").to_owned(),
        latest: h["latest"].as_str().unwrap_or("").to_owned(),
        timezone: h["accountingTimezone"].as_str().unwrap_or("UTC").to_owned(),
        auto,
        summary: d["summary"].clone(),
        buckets: d["buckets"].as_array().cloned().unwrap_or_default(),
        bucket_size: d["bucketSize"].as_str().unwrap_or("month").to_owned(),
        swap_candidate_total: d["swapCandidateTotal"].as_f64(),
    });
    app.txs.dash_failed = false;
    let fiat = fiat_mode(app);
    app.txs.refilter(fiat);
}

fn on_list(app: &mut App, r: &Reply) {
    let (key, more) = app.txs.queue.pop_front().unwrap_or_default();
    if key.as_ref() != Some(&app.txs.list_key) {
        return; // a superseded filter's answer
    }
    if more {
        app.txs.loading_more = false;
    } else {
        app.txs.refreshing = false;
    }
    if let Some(e) = &r.error {
        if app.txs.rows.is_empty() {
            app.txs.list = ListState::Failed(e.clone());
        } else {
            app.toast(
                ToastTone::Error,
                &t("transactions:route.unavailable.title"),
                Some(e.clone()),
                Some("txs-list"),
            );
        }
        return;
    }
    let d = &r.data;
    let new_rows: Vec<Row> = d["txs"]
        .as_array()
        .map(|a| a.iter().map(Row::parse).collect())
        .unwrap_or_default();
    if more {
        let have: HashSet<String> = app.txs.rows.iter().map(|r| r.id.clone()).collect();
        app.txs
            .rows
            .extend(new_rows.into_iter().filter(|r| !have.contains(&r.id)));
    } else {
        if app.txs.highlight_until < 0.0 {
            let before: HashSet<&str> = app.txs.rows.iter().map(|r| r.id.as_str()).collect();
            let fresh: HashSet<String> = new_rows
                .iter()
                .filter(|r| !before.contains(r.id.as_str()))
                .map(|r| r.id.clone())
                .collect();
            app.txs.highlighted = fresh;
            app.txs.highlight_until = app.clock + 6.0;
        }
        app.txs.rows = new_rows;
    }
    app.txs.total = d["total"]
        .as_u64()
        .or_else(|| d["count"].as_u64())
        .unwrap_or(app.txs.rows.len() as u64) as usize;
    app.txs.next_cursor = d["nextCursor"].as_str().map(str::to_owned);
    app.txs.has_more = d["hasMore"].as_bool().unwrap_or(false) && app.txs.next_cursor.is_some();
    app.txs.list = ListState::Ready;
    app.txs.generation += 1;
    let fiat = fiat_mode(app);
    app.txs.refilter(fiat);
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if app.route.path() != "/transactions" || !matches!(ev.key, rune::Key::Escape) {
        return false;
    }
    if app.txs.popup.is_some() {
        app.txs.popup = None;
        return true;
    }
    if app.txs.confirm_exclude.is_some() {
        app.txs.confirm_exclude = None;
        return true;
    }
    if app.txs.export.open && !app.txs.export.busy {
        app.txs.export.open = false;
        return true;
    }
    if app.txs.expanded {
        app.txs.expanded = false;
        return true;
    }
    false
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.txs;
    let page = col()
        .key("txs-page")
        .w_full()
        .px(tk.page_pad())
        .pb(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    if let ListState::Failed(msg) = &ui.list
        && ui.rows.is_empty()
    {
        return page.pt(tk.page_pad()).child(
            surface(tk)
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.25))
                .role("alert")
                .label(t("transactions:route.unavailable.title"))
                .child(
                    text(t("transactions:route.unavailable.title"))
                        .font(tk.tsm(600.0))
                        .color(tk.fg),
                )
                .child(
                    text(if msg.is_empty() {
                        t("transactions:route.unavailable.body")
                    } else {
                        msg.clone()
                    })
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .w_full(),
                ),
        );
    }
    let first_load = ui.rows.is_empty()
        && ui.dash.is_none()
        && matches!(ui.list, ListState::Idle | ListState::Loading);
    let mut page = page.child(workbench::period_nav(app, tk, layer));
    if first_load {
        let bar = |w: f32, h: f32| row().w(w).h(h).radius(tk.r(0.375)).fill(tk.muted);
        return page.child(
            surface(tk)
                .w_full()
                .padding(tk.r(0.75))
                .gap(tk.r(0.5))
                .label(t("common:state.loading"))
                .child(bar(tk.r(11.0), tk.r(1.25)))
                .children((0..8).map(|_| bar(0.0, tk.r(2.5)).w_full())),
        );
    }
    if !ui.expanded {
        page = page.child(workbench::view(app, cx, tk, layer));
    }
    page = page.child(table::view(app, cx, tk, layer));
    if ui.confirm_exclude.is_some() {
        table::confirm_dialog(app, tk, layer);
    }
    if ui.export.open {
        export::dialog(app, tk, layer);
    }
    page
}

/// Headless scenes: the export dialog with its first preview loaded.
pub fn open_export_primed(app: &mut App) {
    export::prime(app);
}

/// The export dialog's headless scenes (registered in main.rs's scene list).
pub const EXPORT_SCENES: &[crate::app::Scene] = export::SCENES;

#[cfg(test)]
mod switch_tests {
    use super::*;
    use crate::app::Route;

    fn reply(data: Value) -> Reply {
        Reply {
            kind: "ui.transactions.list".into(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// Another book opens (outside a session start) while the old book's reads are out, and
    /// the screen asks the same of the new one: the old answers are dropped, the new ones land.
    #[test]
    fn a_book_switch_drops_the_old_books_reads() {
        let mut app = crate::app::tests::quiet_app();
        app.route = Route("/transactions".into());
        app.reads.insert(
            "ui.profiles.snapshot".into(),
            reply(json!({ "activeProfileId": "b" })),
        );
        app.txs.book = Some("a".into());
        app.txs.loans_requested = true;
        let dash = dash_args(&app.txs).to_string();
        app.txs.dash_queue.push_back(Some(dash));
        app.txs.queue.push_back((Some("{}".into()), false));
        step(&mut app);
        assert_eq!(app.txs.book.as_deref(), Some("b"));
        on_reply(&mut app, "txs.dash", &reply(json!({ "summary": "a" })));
        assert!(app.txs.dash.is_none(), "the old book's dashboard");
        on_reply(&mut app, "txs.dash", &reply(json!({ "summary": "b" })));
        assert_eq!(app.txs.dash.as_ref().unwrap().summary, json!("b"));
        // The list goes out once the period is resolved.
        step(&mut app);
        on_reply(
            &mut app,
            "txs.list",
            &reply(json!({ "txs": [{ "id": "a1" }] })),
        );
        assert!(app.txs.rows.is_empty(), "the old book's rows");
        on_reply(
            &mut app,
            "txs.list",
            &reply(json!({ "txs": [{ "id": "b1" }] })),
        );
        assert_eq!(app.txs.rows.len(), 1);
        assert_eq!(app.txs.rows[0].id, "b1");
    }
}
