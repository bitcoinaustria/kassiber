//! The book set rollup (`ui.workspace.overview.snapshot`): the set's name and totals, what needs
//! attention (only then), one list of its books (a row opens its book; its other pages are in
//! the row's menu; fiat basis and YTD on hover), the refresh panel while a run lasts, and the
//! shared BTC activity chart and recent activity once there is any.
//!
//! "Refresh book set" (`ui.workspace.freshness.run`) syncs every book from its sources: network
//! egress, sent only from that button. Opening a book or a transaction switches the active book
//! (`ui.profiles.switch`) and then navigates, as the web does.

use rune::{Color, Cx, El, Icon, TextStyle, col, el, row, text};
use serde_json::{Value, json};

use super::kit::amber;
use super::list::{page, skeleton};
use crate::app::{App, Currency, Reply, ToastTone};
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::model::{self, Flow, RecentRow, Snapshot, TxStatus};
use crate::theme::Tk;
use crate::txdetail::widgets::{Variant, spinner};
use crate::ui::{hairline, ic, sensitive, surface};

/// One refresh progress line.
#[derive(Clone, Debug)]
pub struct Progress {
    pub book: Option<String>,
    pub phase: Option<String>,
    pub source: Option<String>,
    /// The shell's formatted line, when no structured record reached this part.
    pub line: Option<String>,
}

#[derive(Default)]
pub struct State {
    /// The set the data below belongs to (or is being read for).
    pub ws: String,
    pub data: Option<Value>,
    pub chart: Option<Box<Snapshot>>,
    /// The recent list's rows (`activityTxs`, else `txs`) as the overview table reads them.
    pub recent: Option<Box<Snapshot>>,
    pub error: Option<String>,
    pub loading: bool,
    pub refreshing: bool,
    pub progress: Vec<Progress>,
    pub summary: Option<Value>,
    pub last_line: Option<String>,
    pub switching: bool,
    /// After a switch: (route, toast title, toast body).
    pub then: Option<(String, String, String)>,
    /// `App::book_generation` the shown read belongs to; a book refresh reads it again.
    pub generation: u64,
    /// The generation of each snapshot read in flight, oldest first (kept across a set
    /// change, so an earlier answer is recognized and dropped).
    pub asked: std::collections::VecDeque<u64>,
}

/// Mount and set change: read the set's overview (local).
pub fn step(app: &mut App, id: &str) -> bool {
    let generation = app.book_generation;
    let s = &mut app.reports.books.be;
    if s.ws == id && s.generation == generation {
        return false;
    }
    if s.ws == id {
        // A book refresh since the read (an exclusion, a classification, a sync): read the
        // set again, showing the old figures as loading until it lands.
        s.loading = true;
    } else {
        let asked = std::mem::take(&mut s.asked);
        *s = State {
            ws: id.to_owned(),
            loading: true,
            asked,
            ..State::default()
        };
    }
    s.generation = generation;
    s.asked.push_back(generation);
    app.send(
        "ui.workspace.overview.snapshot",
        json!({"workspace_id": id}),
        "reports.books.be.snapshot",
    );
    true
}

/// While a refresh runs, the shell's progress line becomes a panel row when no structured
/// record arrived (the host hands stream progress to the shell only).
pub fn follow_refresh(app: &mut App) -> bool {
    let s = &mut app.reports.books.be;
    if !s.refreshing {
        return false;
    }
    let line = app.busy.progress.clone();
    if line.is_some() && line != s.last_line {
        s.last_line = line.clone();
        if s.progress.iter().all(|p| p.line.is_some()) {
            s.progress.insert(
                0,
                Progress {
                    book: None,
                    phase: None,
                    source: None,
                    line,
                },
            );
            s.progress.truncate(8);
        }
        return true;
    }
    false
}

pub fn on_progress(app: &mut App, d: &Value) {
    let s = &mut app.reports.books.be;
    s.progress.retain(|p| p.line.is_none());
    s.progress.insert(
        0,
        Progress {
            book: d["profile"]["label"].as_str().map(str::to_owned),
            phase: d["phase"].as_str().map(str::to_owned),
            source: d["source_label"].as_str().map(str::to_owned),
            line: None,
        },
    );
    s.progress.truncate(8);
}

/// `handleRefresh`: egress, so only from the button.
pub fn refresh(app: &mut App) {
    let s = &app.reports.books.be;
    if s.refreshing || s.ws.is_empty() {
        return;
    }
    let ws = s.ws.clone();
    if app
        .send(
            "ui.workspace.freshness.run",
            json!({"workspace_id": ws, "journals": true, "run": true}),
            "reports.books.be.refresh",
        )
        .is_some()
    {
        let s = &mut app.reports.books.be;
        s.refreshing = true;
        s.progress.clear();
        s.summary = None;
        s.last_line = app.busy.progress.clone();
    }
}

fn switch_then(app: &mut App, profile_id: &str, tag: &'static str, then: (String, String, String)) {
    if app.reports.books.be.switching {
        return;
    }
    if app
        .send("ui.profiles.switch", json!({"profile_id": profile_id}), tag)
        .is_some()
    {
        app.reports.books.be.switching = true;
        app.reports.books.be.then = Some(then);
    }
}

/// `BOOK_ROUTES`: (id, nav label key, route, icon).
const BOOK_ROUTES: [(&str, &str, &str, &Icon); 6] = [
    ("overview", "nav:book.overview", "/overview", &I::GAUGE),
    (
        "transactions",
        "nav:book.transactions",
        "/transactions",
        &I::TABLE_PROPERTIES,
    ),
    ("ledger", "nav:book.ledger", "/journals", &I::BOOK_OPEN),
    ("review", "nav:book.review", "/review", &I::INBOX),
    (
        "wallets",
        "nav:book.wallets",
        "/connections",
        &I::WALLET_CARDS,
    ),
    ("reports", "nav:book.reports", "/reports", &I::CHART_COLUMN),
];

/// `openBookRoute`.
fn open_book_route(app: &mut App, profile_id: &str, book: &str, label_key: &str, route: &str) {
    let book = if book.is_empty() {
        t("nav:openBook.fallbackBook")
    } else {
        book.to_owned()
    };
    let body = tr(
        "nav:openBook.body",
        &[("book", &book), ("page", &t(label_key))],
    );
    switch_then(
        app,
        profile_id,
        "reports.books.be.switch",
        (route.to_owned(), t("nav:openBook.title"), body),
    );
}

/// `openWorkspaceTransaction`.
fn open_transaction(app: &mut App, profile_id: &str, book: &str, tx_id: &str) {
    if profile_id.is_empty() {
        return;
    }
    let book = if book.is_empty() {
        t("overview:birdsEye.toast.fallbackBook")
    } else {
        book.to_owned()
    };
    let body = tr(
        "overview:birdsEye.toast.transactionBody",
        &[("book", &book)],
    );
    switch_then(
        app,
        profile_id,
        "reports.books.be.switch_tx",
        (
            format!("/transactions?tx={tx_id}"),
            t("overview:birdsEye.toast.activeBookChangedTitle"),
            body,
        ),
    );
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "reports.books.be.snapshot" => {
            let ws = r.data["workspace"]["id"].as_str().unwrap_or_default();
            let s = &mut app.reports.books.be;
            let asked = s.asked.pop_front();
            if (!ws.is_empty() && ws != s.ws) || asked != Some(s.generation) {
                return; // a superseded set's, or an earlier generation's, answer
            }
            s.loading = false;
            match &r.error {
                Some(e) => {
                    s.error = Some(if e.trim().is_empty() {
                        t("overview:birdsEye.couldNotLoad")
                    } else {
                        e.clone()
                    })
                }
                None => {
                    s.error = None;
                    s.chart = Snapshot::parse(&chart_snapshot(&r.data)).ok().map(Box::new);
                    s.recent = s.chart.as_ref().map(|c| {
                        let mut x = (**c).clone();
                        if !x.activity_txs.is_empty() {
                            x.txs = x.activity_txs.clone();
                        }
                        Box::new(x)
                    });
                    s.data = Some(r.data.clone());
                }
            }
        }
        "reports.books.be.refresh" => {
            let ws = app.reports.books.be.ws.clone();
            let s = &mut app.reports.books.be;
            s.refreshing = false;
            app.busy.progress = None;
            match &r.error {
                Some(e) => {
                    let title = t("chrome:notifications.condition.workspaceRefreshFailed.title");
                    app.toast(ToastTone::Error, &title, None, Some("workspace-refresh"));
                    let at = app.clock;
                    app.shell
                        .note(&title, Some(e.clone()), ToastTone::Error, at);
                }
                None => {
                    let summary = r.data.get("summary").filter(|v| v.is_object()).cloned();
                    let problems = summary.as_ref().is_some_and(|x| {
                        x["errors"].as_f64().unwrap_or(0.0) > 0.0
                            || x["reports_blocked"].as_f64().unwrap_or(0.0) > 0.0
                    });
                    app.reports.books.be.summary = summary;
                    let (title, tone) = if problems {
                        (
                            t("chrome:notifications.condition.workspaceBooks.title"),
                            ToastTone::Warning,
                        )
                    } else {
                        (
                            t("chrome:notifications.result.workspaceRefreshed.title"),
                            ToastTone::Success,
                        )
                    };
                    app.toast(tone, &title, None, Some("workspace-refresh"));
                    let at = app.clock;
                    app.shell.note(&title, None, tone, at);
                }
            }
            // Mutations invalidate every read: the rollup and the shell's book state.
            if !ws.is_empty() {
                app.send(
                    "ui.workspace.overview.snapshot",
                    json!({"workspace_id": ws}),
                    "reports.books.be.snapshot",
                );
            }
            app.refresh_reads();
        }
        "reports.books.be.switch" | "reports.books.be.switch_tx" => {
            let s = &mut app.reports.books.be;
            s.switching = false;
            let then = s.then.take();
            match &r.error {
                Some(e) => {
                    let (title, fallback) = if tag == "reports.books.be.switch" {
                        (
                            "overview:birdsEye.toast.couldNotOpenBookTitle",
                            "overview:birdsEye.toast.couldNotOpenBookBody",
                        )
                    } else {
                        (
                            "overview:birdsEye.toast.couldNotOpenTransactionTitle",
                            "overview:birdsEye.toast.couldNotOpenTransactionBody",
                        )
                    };
                    let body = if e.trim().is_empty() {
                        t(fallback)
                    } else {
                        e.clone()
                    };
                    let key = if tag == "reports.books.be.switch" {
                        "birds-eye-open-book"
                    } else {
                        "birds-eye-open-transaction"
                    };
                    app.toast(ToastTone::Error, &t(title), Some(body), Some(key));
                }
                None => {
                    // `App::deliver` starts the new book's session after this.
                    if let Some((route, title, body)) = then {
                        app.toast(
                            ToastTone::Info,
                            &title,
                            Some(body),
                            Some("birds-eye-active-book"),
                        );
                        app.navigate(&route);
                    }
                }
            }
        }
        _ => {}
    }
}

/// `workspaceChartSnapshot`: the rollup in the shape of `ui.overview.snapshot`.
pub fn chart_snapshot(d: &Value) -> Value {
    let books = d["books"].as_array().cloned().unwrap_or_default();
    let mr = books
        .iter()
        .map(|b| &b["marketRate"])
        .find(|m| m.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    let fiat = &d["fiat"];
    let fc = fiat["fiatCurrency"]
        .as_str()
        .or_else(|| mr["fiatCurrency"].as_str())
        .unwrap_or("EUR")
        .to_owned();
    let series = d["portfolioSeries"].as_array().cloned().unwrap_or_default();
    let fallback = mr["rate"]
        .as_f64()
        .or_else(|| {
            series
                .iter()
                .filter_map(|p| p["priceEur"].as_f64())
                .find(|r| r.is_finite() && *r > 0.0)
        })
        .unwrap_or(0.0);
    let portfolio: Vec<Value> = series
        .iter()
        .map(|p| {
            json!({
                "date": p["date"], "label": p["label"], "balanceBtc": p["balanceBtc"],
                "valueEur": p["valueEur"].as_f64().unwrap_or(0.0),
                "costBasisEur": p["costBasisEur"].as_f64().unwrap_or(0.0),
                "priceEur": p["priceEur"], "priceTimestamp": p["priceTimestamp"], "priceSource": p["priceSource"],
            })
        })
        .collect();
    let mixed = fiat["mixed"].as_bool().unwrap_or(false);
    let market = if mixed {
        Value::Null
    } else {
        json!({
            "asset": "BTC", "fiatCurrency": fc,
            "pair": mr["pair"].as_str().map(str::to_owned).unwrap_or_else(|| format!("BTC-{fc}")),
            "rate": if fallback > 0.0 { json!(fallback) } else { Value::Null },
            "timestamp": mr["timestamp"], "source": mr["source"], "fetchedAt": mr["fetchedAt"],
            "granularity": mr["granularity"], "method": mr["method"],
        })
    };
    json!({
        "priceEur": if fc == "EUR" { fallback } else { 0.0 },
        "priceUsd": 0.0,
        "marketRate": market,
        "connections": d["connections"],
        "txs": d["txs"],
        "activityTxs": d["activityTxs"],
        "balanceSeries": d["balanceSeries"],
        "portfolioSeries": portfolio,
        "fiat": {
            "fiatCurrency": fc,
            "eurBalance": fiat["eurBalance"].as_f64().unwrap_or(0.0),
            "eurCostBasis": fiat["eurCostBasis"].as_f64().unwrap_or(0.0),
            "eurUnrealized": fiat["eurUnrealized"].as_f64().unwrap_or(0.0),
            "eurRealizedYTD": fiat["eurRealizedYTD"].as_f64().unwrap_or(0.0),
            "completeness": fiat["completeness"],
        },
        "status": {
            "workspace": d["workspace"]["label"],
            "profile": "Book set",
            "transactionCount": d["status"]["transactionCount"],
            "needsJournals": d["status"]["needsJournals"],
            "quarantines": d["status"]["quarantines"],
        },
    })
}

/// `normalizeFiatCompleteness` as far as the rollup reads it: (state, basis complete, market
/// rate missing). A missing object is "unavailable", never exact.
fn completeness(c: &Value) -> (String, bool, bool) {
    let raw = c["state"].as_str().unwrap_or("unavailable");
    let state = if matches!(raw, "complete" | "incomplete" | "stale" | "unavailable") {
        raw
    } else {
        "unavailable"
    };
    let complete = c["costBasisComplete"].as_bool().unwrap_or(false)
        && !matches!(state, "stale" | "incomplete");
    (
        state.to_owned(),
        complete,
        c["marketRateMissing"].as_bool().unwrap_or(false),
    )
}

/// `completenessBadgeKey`.
fn badge_key(c: &(String, bool, bool)) -> Option<&'static str> {
    if c.1 {
        return c.2.then_some("overview:completeness.badge.noMarketRate");
    }
    Some(match c.0.as_str() {
        "stale" => "overview:completeness.badge.stale",
        "unavailable" => "overview:completeness.badge.unavailable",
        _ => "overview:completeness.badge.incomplete",
    })
}

/// A quiet attention line: amber ink and its icon, no box.
fn attention(tk: &Tk, label: String) -> El {
    crate::ui::tag(tk, amber(tk), Some(&I::TRIANGLE_ALERT), label)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, ws: &str, layer: &mut Vec<El>) -> El {
    let s = &app.reports.books.be;
    if s.ws != ws || (s.loading && s.data.is_none()) {
        return skeleton(app, tk, 0, tk.r(11.0));
    }
    if let Some(e) = &s.error {
        return page(tk).child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .role("alert")
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.destructive))
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    let empty = json!({});
    let d = s.data.as_ref().unwrap_or(&empty);
    let hide = app.prefs.hide_sensitive();
    let books: Vec<Value> = d["books"].as_array().cloned().unwrap_or_default();
    let fiat = &d["fiat"];
    let mixed = fiat["mixed"].as_bool().unwrap_or(false);
    let roll = completeness(&fiat["completeness"]);
    let fiat_books: Vec<Value> = fiat["books"].as_array().cloned().unwrap_or_default();
    let basis_incomplete = fiat_books
        .iter()
        .filter(|r| !completeness(&r["completeness"]).1)
        .count();
    let ready = d["status"]["readyBooks"]
        .as_u64()
        .map(|v| v as usize)
        .unwrap_or_else(|| {
            books
                .iter()
                .filter(|b| b["readiness"]["ready"].as_bool() == Some(true))
                .count()
        });
    let blocked = d["status"]["blockedBooks"]
        .as_u64()
        .map(|v| v as usize)
        .unwrap_or(books.len().saturating_sub(ready));
    let quarantines = d["status"]["quarantines"].as_u64().unwrap_or(0) as usize;
    let title = d["workspace"]["label"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| t("overview:birdsEye.fallbackTitle"));

    // Header: the set's name (the read-only note on hover) and its totals; then only what needs
    // attention; the set-wide refresh as a quiet icon on the right.
    let fc = fiat["fiatCurrency"].as_str().unwrap_or("EUR").to_owned();
    let btc = format::btc(fiat["btcBalance"].as_f64().unwrap_or(0.0), 8, false);
    let fiat_value = if mixed {
        None
    } else if roll.2 {
        Some("—".to_owned())
    } else {
        Some(format::fiat(
            fiat["eurBalance"].as_f64().unwrap_or(0.0),
            &fc,
        ))
    };
    let totals_style = tk.tsm(400.0).features("'tnum' 1");
    let dot = || {
        text("·")
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .a11y_hidden()
    };
    let mut totals = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(sensitive(btc, totals_style, tk.fg, hide).key("be-total-btc"));
    match fiat_value {
        Some(v) => {
            totals = totals
                .child(dot())
                .child(sensitive(v, totals_style, tk.fg, hide).key("be-total-fiat"));
        }
        None => {
            totals = totals.child(dot()).child(crate::ui::tag(
                tk,
                tk.muted_fg,
                None,
                t("overview:birdsEye.mixedFiatBadge"),
            ));
        }
    }
    totals = totals.child(dot()).child(
        text(tc("overview:birdsEye.metric.books", books.len(), &[]))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg),
    );
    let name = col()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.25))
        .child(
            text(title)
                .font(tk.tlg(600.0).lh(tk.r(1.125) * 1.25))
                .color(tk.fg)
                .role("heading")
                .help(t("overview:birdsEye.readonlyNote")),
        )
        .child(totals);
    let header = row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .justify_between()
        .child(name)
        .child(refresh_button(tk, s.refreshing));
    let mut signals: Vec<El> = Vec::new();
    if blocked > 0 {
        signals.push(attention(
            tk,
            tc("overview:birdsEye.metric.booksNeedAttention", blocked, &[]),
        ));
    }
    if basis_incomplete > 0 {
        signals.push(attention(
            tk,
            tc(
                "overview:birdsEye.metric.basisIncompleteBooks",
                basis_incomplete,
                &[],
            ),
        ));
    } else if roll.2 && !mixed {
        signals.push(attention(tk, t("overview:completeness.valueUnavailable")));
    }
    if quarantines > 0 {
        signals.push(attention(
            tk,
            tc("overview:birdsEye.bookRow.quarantines", quarantines, &[]),
        ));
    }
    if d["status"]["needsJournals"].as_bool().unwrap_or(false) {
        signals.push(attention(
            tk,
            t("overview:birdsEye.metric.journalsNeedProcessing"),
        ));
    }

    let mut out = page(tk).busy(s.loading || s.refreshing).child(header);
    if !signals.is_empty() {
        out = out.child(
            row()
                .key("be-signals")
                .w_full()
                .gap(tk.r(1.0))
                .flex_wrap()
                .center_y()
                .children(signals),
        );
    }

    // The books: one list; a row opens its book, its other pages are in the row's menu.
    let disabled = s.switching;
    let mut list = surface(tk)
        .key("be-books-card")
        .w_full()
        .min_w(0.0)
        .clip_rounded(tk.r(0.75))
        .role("list")
        .label(t("overview:birdsEye.booksCard.title"));
    if books.is_empty() {
        list = list.child(
            row().w_full().padding(tk.card_pad()).child(
                text(t("overview:birdsEye.booksCard.empty"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    for (i, b) in books.iter().enumerate() {
        if i > 0 {
            list = list.child(hairline(tk.border));
        }
        let pid = b["profile"]["id"].as_str().unwrap_or_default();
        let fr = fiat_books
            .iter()
            .find(|r| r["profileId"].as_str() == Some(pid));
        list = list.child(book_row(app, tk, layer, b, fr, hide, disabled, mixed));
    }
    out = out.child(list);
    if s.refreshing || !s.progress.is_empty() || s.summary.is_some() {
        let win = if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        };
        out = out.child(refresh_panel(tk, s, win));
    }
    // The shared chart and the recent activity, once there is any.
    if let Some(snap) = s.chart.as_deref()
        && (!snap.portfolio_series.is_empty()
            || !snap.txs.is_empty()
            || !snap.activity_txs.is_empty())
    {
        out = out.child(crate::chart::view(app, cx, tk, snap, layer));
    }
    if let Some(rs) = s.recent.as_deref()
        && let Some(table) = recent_table(app, tk, d, rs)
    {
        out = out.child(table);
    }
    out
}

/// The set-wide refresh (`ui.workspace.freshness.run`, network egress: only from here): a quiet
/// icon button, the spinner while the run lasts.
fn refresh_button(tk: &Tk, running: bool) -> El {
    let label = t("overview:birdsEye.refresh");
    let side = tk.r(2.0);
    let glyph = if running {
        ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.muted_fg).looping(rune::LoopSpec::rotation(0.9))
    } else {
        ic(&I::REFRESH_CW, tk.r(1.0), tk.muted_fg)
    };
    let mut b = crate::ui::button::surface(
        tk,
        row().key("be-refresh").size(side, side).center(),
        Variant::Ghost,
    )
    .radius(crate::ui::button::radius(tk, side))
    .role("button")
    .label(label.clone())
    .help(label)
    .focusable(!running)
    .disabled(running)
    .busy(running)
    .child(glyph);
    if !running {
        b = b.on_press(|a: &mut App| refresh(a));
    }
    b
}

fn refresh_panel(tk: &Tk, s: &State, win: f32) -> El {
    let head = row()
        .w_full()
        .px(tk.card_pad())
        .pb(tk.card_pad())
        .gap(tk.r(0.5))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(if s.refreshing {
            spinner(tk, tk.r(1.0))
        } else {
            ic(&I::CIRCLE_CHECK, tk.r(1.0), tk.fg)
        })
        .child(
            text(t("overview:birdsEye.refreshPanel.title"))
                .font(tk.tbase(600.0).lh(tk.r(1.0)))
                .color(tk.fg),
        );
    let boxed = |e: El| {
        e.radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.background.alpha(0.8))
    };
    let mut lines = col().grow(1.0).min_w(0.0).gap(tk.r(0.5));
    if s.progress.is_empty() {
        lines = lines.child(
            boxed(row().w_full().px(tk.r(0.75)).py(tk.r(0.5))).child(
                text(t("overview:birdsEye.refreshPanel.waiting"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    for p in &s.progress {
        let book = p
            .book
            .clone()
            .unwrap_or_else(|| t("overview:birdsEye.refreshPanel.fallbackBook"));
        let rest = match &p.line {
            Some(l) => format!(" · {l}"),
            None => {
                let phase = p.phase.as_deref().map(phase_label).unwrap_or("In progress");
                format!(
                    " · {phase}{}",
                    p.source
                        .as_ref()
                        .map(|s| format!(" · {s}"))
                        .unwrap_or_default()
                )
            }
        };
        lines = lines.child(
            boxed(row().w_full().px(tk.r(0.75)).py(tk.r(0.5)))
                .child(text(book).font(tk.tsm(500.0)).color(tk.fg))
                .child(text(rest).font(tk.tsm(400.0)).color(tk.muted_fg).truncate()),
        );
    }
    let summary = match &s.summary {
        Some(x) => {
            let n = |k: &str| x[k].as_u64().unwrap_or(0).to_string();
            col()
                .gap(tk.r(0.25))
                .child(
                    text(tr(
                        "overview:birdsEye.refreshPanel.booksRefreshed",
                        &[("synced", &n("synced_books")), ("total", &n("books"))],
                    ))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
                )
                .child(
                    text(tr(
                        "overview:birdsEye.refreshPanel.summaryDetail",
                        &[
                            ("blocked", &n("reports_blocked")),
                            ("rateLimited", &n("rate_limited")),
                        ],
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .w_full(),
                )
        }
        None => col().child(
            text(t("overview:birdsEye.refreshPanel.placeholder"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        ),
    };
    let summary = boxed(col().padding(tk.r(0.75))).child(summary);
    let body = if win >= 768.0 {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .child(lines)
            .child(summary.w(280.0))
    } else {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(lines)
            .child(summary.w_full())
    };
    surface(tk)
        .key("be-refresh-panel")
        .w_full()
        .py(tk.card_pad())
        .gap(tk.r(1.0))
        .fill(tk.primary.alpha(0.05))
        .border(1.0, tk.primary.alpha(0.2))
        .live_region(rune::LiveRegion::Polite)
        .child(head)
        .child(body.px(tk.card_pad()))
}

/// `PHASE_LABELS` (lib/syncProgress.ts), hard-coded English in the web too.
fn phase_label(phase: &str) -> &str {
    match phase {
        "discovery" => "Discovering wallet history",
        "backend_fetch" => "Fetching source history",
        "decode_enrich" => "Decoding and enriching transactions",
        "import" | "importing" => "Importing transactions",
        "rate_coverage" => "Checking market-rate coverage",
        "auto_pair" => "Pairing swaps and transfers",
        "journal_refresh" => "Refreshing journals",
        "rate_limited" => "Waiting out rate limit",
        "done" => "Refresh complete",
        "error" => "Refresh needs attention",
        other => other,
    }
}

/// One book: name, its sources and transactions, what needs attention (only then), BTC and fiat
/// (basis and YTD on hover); pressing it opens the book, its other pages are in the "⋯" menu.
#[allow(clippy::too_many_arguments)]
fn book_row(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    b: &Value,
    fr: Option<&Value>,
    hide: bool,
    disabled: bool,
    mixed: bool,
) -> El {
    let ready = b["readiness"]["ready"].as_bool().unwrap_or(false);
    let pid = b["profile"]["id"].as_str().unwrap_or_default().to_owned();
    let label = b["profile"]["label"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let fc = [&b["profile"]["fiatCurrency"], &b["fiat"]["fiatCurrency"]]
        .iter()
        .filter_map(|v| v.as_str())
        .find(|s| !s.is_empty())
        .unwrap_or("EUR")
        .to_owned();
    let comp = completeness(&b["fiat"]["completeness"]);
    let btc = b["balanceSummary"]["totalBtc"].as_f64().unwrap_or_else(|| {
        b["connections"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c["balance"].as_f64()).sum())
            .unwrap_or(0.0)
    });
    let n = |v: &Value| v.as_u64().unwrap_or(0) as usize;
    let mut name = row().min_w(0.0).gap(tk.r(0.75)).center_y().child(
        text(label.clone())
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .truncate(),
    );
    if mixed {
        name = name.child(crate::ui::tag(tk, tk.muted_fg, None, fc.clone()));
    }
    if let Some(k) = badge_key(&comp) {
        name = name.child(crate::ui::tag(tk, amber(tk), None, t(k)));
    }
    let small = tk.txs(400.0);
    let sep = || text("·").font(small).color(tk.muted_fg).a11y_hidden();
    let mut meta = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(
            text(tc(
                "overview:birdsEye.bookRow.wallets",
                b["connections"].as_array().map(|a| a.len()).unwrap_or(0),
                &[],
            ))
            .font(small)
            .color(tk.muted_fg),
        )
        .child(sep())
        .child(
            text(tc(
                "overview:birdsEye.bookRow.txs",
                n(&b["status"]["transactionCount"]),
                &[],
            ))
            .font(small)
            .color(tk.muted_fg),
        );
    let q = n(&b["journals"]["quarantine_count"]);
    if q > 0 {
        meta = meta.child(sep()).child(
            text(tc("overview:birdsEye.bookRow.quarantines", q, &[]))
                .font(small)
                .color(amber(tk)),
        );
    }
    let mut left = col()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.25))
        .child(name)
        .child(meta);
    if !ready {
        let hint = b["readiness"]["hints"][0]
            .as_str()
            .or_else(|| b["journals"]["reason"].as_str())
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| t("overview:birdsEye.bookRow.attention"));
        left = left.child(attention(tk, hint));
    }
    let fiat_s = if comp.2 {
        "—".to_owned()
    } else {
        format::fiat(b["fiat"]["eurBalance"].as_f64().unwrap_or(0.0), &fc)
    };
    // The fiat row's basis and YTD (basis-derived figures "—" while the basis isn't exact).
    let fiat_help = fr.map(|r| {
        let c = completeness(&r["completeness"]);
        let money = |v: f64| {
            if hide {
                t("common:state.hidden")
            } else if !c.1 {
                "—".to_owned()
            } else {
                format::fiat(v, &fc)
            }
        };
        format!(
            "{} · {}",
            tr(
                "overview:birdsEye.fiatCard.basis",
                &[("value", &money(r["costBasis"].as_f64().unwrap_or(0.0)))],
            ),
            tr(
                "overview:birdsEye.fiatCard.ytd",
                &[("value", &money(r["realizedYTD"].as_f64().unwrap_or(0.0)))],
            )
        )
    });
    let mut fiat_el = sensitive(fiat_s, small.features("'tnum' 1"), tk.muted_fg, hide);
    if let Some(h) = fiat_help {
        fiat_el = fiat_el.help(h);
    }
    let amounts = col()
        .items_end()
        .gap(tk.r(0.125))
        .child(sensitive(
            format::btc(btc, 8, false),
            tk.tsm(400.0).features("'tnum' 1"),
            tk.fg,
            hide,
        ))
        .child(fiat_el);
    let (p, l) = (pid.clone(), label.clone());
    let mut main = row()
        .key(format!("be-book-{pid}"))
        .grow(1.0)
        .min_w(0.0)
        .px(tk.card_pad())
        .py(tk.r(0.75))
        .gap(tk.r(1.0))
        .center_y()
        .role("button")
        .label(tr(
            "nav:openBook.body",
            &[("book", &label), ("page", &t("nav:book.overview"))],
        ))
        .focusable(!disabled)
        .disabled(disabled)
        .child(left)
        .child(amounts);
    if !disabled {
        main = main
            .hover(tk.muted.alpha(0.4))
            .on_press(move |a: &mut App| {
                open_book_route(a, &p, &l, "nav:book.overview", "/overview")
            });
    }
    // The book's other pages.
    let menu_key = format!("be-book-actions-{pid}");
    let pop = format!("be:{pid}");
    let is_open = app.reports.books.popup.as_deref() == Some(pop.as_str());
    let more = crate::ui::button::icon_button(
        tk,
        menu_key.clone(),
        Variant::Ghost,
        tk.r(1.75),
        &I::ELLIPSIS,
        tk.r(1.0),
        tr("onboarding:books.row.actions", &[("name", &label)]),
    );
    let toggle_key = pop.clone();
    let more = crate::ui::popup::trigger(
        more,
        crate::ui::popup::Kind::Menu,
        is_open,
        move |a: &mut App| {
            a.reports.books.popup = if a.reports.books.popup.as_deref() == Some(toggle_key.as_str())
            {
                None
            } else {
                Some(toggle_key.clone())
            };
        },
    );
    if is_open {
        let items = BOOK_ROUTES
            .iter()
            .map(|&(id, key, route, glyph)| {
                let k = format!("book-route-{pid}-{id}");
                let name = t(key);
                let (p, l) = (pid.clone(), label.clone());
                let mut item = super::kit::menu_item(tk, &k, glyph, &name);
                if !disabled {
                    item = item.on_press(move |a: &mut App| {
                        a.reports.books.popup = None;
                        open_book_route(a, &p, &l, key, route)
                    });
                }
                (item, k, name)
            })
            .collect();
        super::kit::dropdown(
            tk,
            layer,
            &menu_key,
            tk.r(13.0),
            rune_kit::Placement::BottomEnd,
            items,
        );
    }
    row()
        .w_full()
        .min_w(0.0)
        .center_y()
        .pr(tk.r(0.75))
        .role("listitem")
        .label(label)
        .child(main)
        .child(more)
}

// ---- Recent activity by book (RecentTransactionsTable with `scopeLabel`, no Show all) --------

/// `RECENT_TX_REVEAL_STEP`: the web's list starts at 8 rows, and 8 rows fit its 340 px cap, so
/// it never scrolls to reveal more.
const RECENT_ROWS: usize = 8;

fn h(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::WHITE)
}

fn flow_chip(tk: &Tk, f: Flow) -> (Color, Color, Color) {
    let (l50, l700, ring_l, d900, d_alpha, d300, ring_d) = match f {
        Flow::Incoming => (
            "#ecfdf5",
            "#047857",
            h("#059669").alpha(0.20),
            "#064e3b",
            0.25,
            "#6ee7b7",
            h("#34d399").alpha(0.2),
        ),
        Flow::Outgoing => (
            "#fef2f2",
            "#b91c1c",
            h("#dc2626").alpha(0.15),
            "#7f1d1d",
            0.25,
            "#fca5a5",
            h("#f87171").alpha(0.2),
        ),
        Flow::Transfer => (
            "#fafafa",
            "#3f3f46",
            h("#71717a").alpha(0.20),
            "#27272a",
            0.70,
            "#d4d4d8",
            h("#a1a1aa").alpha(0.2),
        ),
        Flow::Swap => (
            "#f0f9ff",
            "#0369a1",
            h("#0284c7").alpha(0.20),
            "#0c4a6e",
            0.25,
            "#7dd3fc",
            h("#38bdf8").alpha(0.2),
        ),
    };
    if tk.dark {
        (h(d900).alpha(d_alpha), h(d300), ring_d)
    } else {
        (h(l50), h(l700), ring_l)
    }
}

fn status_chip(tk: &Tk, s: TxStatus) -> (Color, Color, Color) {
    let (l50, l700, ring_l, d900, d400) = match s {
        TxStatus::Confirmed => (
            "#ecfdf5",
            "#047857",
            h("#059669").alpha(0.2),
            "#064e3b",
            "#34d399",
        ),
        TxStatus::Pending => (
            "#fffbeb",
            "#b45309",
            h("#d97706").alpha(0.2),
            "#78350f",
            "#fbbf24",
        ),
        TxStatus::Review => (
            "#eff6ff",
            "#1d4ed8",
            h("#1d4ed8").alpha(0.1),
            "#1e3a8a",
            "#60a5fa",
        ),
        TxStatus::Failed => (
            "#fef2f2",
            "#b91c1c",
            h("#dc2626").alpha(0.1),
            "#7f1d1d",
            "#f87171",
        ),
    };
    if tk.dark {
        (h(d900).alpha(0.3), h(d400), h(d400).alpha(0.2))
    } else {
        (h(l50), h(l700), ring_l)
    }
}

fn flow_text(f: Flow) -> String {
    t(match f {
        Flow::Incoming => "overview:txFlow.incoming",
        Flow::Outgoing => "overview:txFlow.outgoing",
        Flow::Transfer => "overview:txFlow.transfer",
        Flow::Swap => "overview:txFlow.swap",
    })
}

fn status_text(s: TxStatus) -> String {
    t(match s {
        TxStatus::Confirmed => "overview:txStatus.confirmed",
        TxStatus::Pending => "overview:txStatus.pending",
        TxStatus::Review => "overview:txStatus.review",
        TxStatus::Failed => "overview:txStatus.failed",
    })
}

fn recent_table(app: &App, tk: &Tk, d: &Value, s: &Snapshot) -> Option<El> {
    let rows = model::recent_rows(s);
    if rows.is_empty() {
        return None;
    }
    let source = if d["activityTxs"].as_array().is_some_and(|a| !a.is_empty()) {
        &d["activityTxs"]
    } else {
        &d["txs"]
    };
    let owner = |id: &str| -> (String, String) {
        source
            .as_array()
            .and_then(|a| a.iter().find(|x| x["id"].as_str() == Some(id)))
            .map(|x| {
                (
                    x["profileId"].as_str().unwrap_or_default().to_owned(),
                    x["book"]["label"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .unwrap_or_default()
    };
    let header = row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.625))
        .center_y()
        .child(
            text(t("overview:birdsEye.recentActivityTitle"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        );
    let body = {
        let mut list = col().key("be-recent-list").w_full();
        for (i, r) in rows.iter().take(RECENT_ROWS).enumerate() {
            if i > 0 {
                list = list.child(hairline(tk.border));
            }
            let (pid, scope) = owner(&r.id);
            list = list.child(recent_row(app, tk, s, r, i, pid, scope));
        }
        list
    };
    Some(
        surface(tk)
            .border_layout()
            .key("be-recent")
            .w_full()
            .clip_rounded(tk.r(0.75))
            .child(header)
            .child(hairline(tk.border))
            .child(body),
    )
}

fn recent_row(
    app: &App,
    tk: &Tk,
    s: &Snapshot,
    r: &RecentRow,
    i: usize,
    pid: String,
    scope: String,
) -> El {
    let hide = app.prefs.hide_sensitive();
    let (bg, fg, ring) = flow_chip(tk, r.flow);
    let flow_label = flow_text(r.flow);
    let type_label = if r.type_label == model::flow_label(r.flow) {
        flow_label
    } else {
        crate::overview::transaction_type_label(&r.type_label)
    };
    let small = TextStyle::new(tk.r(0.694), 400.0).lh(tk.r(0.694) * 1.25);
    let mut meta = row()
        .min_w(0.0)
        .mt(tk.r(0.125))
        .gap(tk.r(0.375))
        .center_y()
        .child(
            text(r.date.clone())
                .font(small.features("'tnum' 1"))
                .color(tk.muted_fg),
        )
        .child(text("·").font(small).color(tk.muted_fg).a11y_hidden())
        .child(
            text(if r.extra_tags > 0 {
                format!("{type_label} +{}", r.extra_tags)
            } else {
                type_label
            })
            .font(small)
            .color(tk.muted_fg)
            .truncate()
            .min_w(0.0),
        );
    if r.status != TxStatus::Confirmed {
        let (_, sf, _) = status_chip(tk, r.status);
        meta = meta.child(
            row().child(
                text(status_text(r.status))
                    .font(TextStyle {
                        weight: 500.0,
                        ..small
                    })
                    .color(sf),
            ),
        );
    }
    if !scope.is_empty() {
        meta = meta.child(
            sensitive(
                scope.clone(),
                TextStyle {
                    weight: 500.0,
                    ..small
                },
                tk.muted_fg,
                hide,
            )
            .truncate()
            .min_w(0.0),
        );
    }
    let cur = r.fiat_currency.clone().unwrap_or_else(|| s.fiat_currency());
    let (primary, secondary) = match app.prefs.currency {
        Currency::Btc => (
            format::btc(r.amount_btc, 8, true),
            r.amount_fiat
                .map(|v| format::fiat(v.abs(), &cur))
                .unwrap_or_else(|| t("common:state.unpriced")),
        ),
        Currency::Fiat => (
            r.amount_fiat
                .map(|v| format::signed_fiat(v, &cur))
                .unwrap_or_else(|| t("common:state.unpriced")),
            format::btc(r.amount_btc, 8, false),
        ),
    };
    let ink = match (r.flow, tk.dark) {
        (Flow::Incoming, false) => h("#047857"),
        (Flow::Incoming, true) => h("#6ee7b7"),
        (Flow::Outgoing, false) => h("#b91c1c"),
        (Flow::Outgoing, true) => h("#fca5a5"),
        _ => tk.muted_fg,
    };
    let toggle = t(if app.prefs.currency == Currency::Btc {
        "chrome:currencyToggleText.showFiat"
    } else {
        "chrome:currencyToggleText.showBitcoin"
    });
    let amount = sensitive(
        primary.clone(),
        TextStyle::new(13.0, 600.0)
            .lh(13.0 * 1.25)
            .features("'tnum' 1"),
        ink,
        hide,
    )
    .key(format!("be-recent-amount-{}", r.id))
    .role("button")
    .label(primary)
    .description(toggle.clone())
    .help(toggle)
    .focusable(true)
    .on_press(|a: &mut App| {
        a.prefs.currency = if a.prefs.currency == Currency::Btc {
            Currency::Fiat
        } else {
            Currency::Btc
        }
    });
    let counterparty = if r.counterparty == "Unassigned" {
        t("overview:recentTx.unassigned")
    } else {
        r.counterparty.clone()
    };
    let (id, book) = (r.id.clone(), scope);
    row()
        .key(format!("be-recent-row-{i}"))
        .w_full()
        .min_w(0.0)
        .px(tk.r(1.0))
        .py(tk.r(0.375))
        .gap(tk.r(0.625))
        .center_y()
        .hover(tk.muted.alpha(0.45))
        .role("button")
        .label(format!("{counterparty} {}", r.date))
        .focusable(true)
        .on_press(move |a: &mut App| open_transaction(a, &pid, &book, &id))
        .child(
            row()
                .size(tk.r(1.5), tk.r(1.5))
                .center()
                .radius(tk.r(0.375))
                .fill(bg)
                .border(1.0, ring)
                .a11y_hidden()
                .child(ic(flow_icon(r.flow), tk.r(0.875), fg)),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(
                    sensitive(
                        if r.counterparty.is_empty() {
                            r.txid.clone()
                        } else {
                            counterparty
                        },
                        TextStyle::new(13.0, 500.0).lh(13.0 * 1.25),
                        tk.fg,
                        hide,
                    )
                    .truncate(),
                )
                .child(meta),
        )
        .child(col().items_end().pl(tk.r(0.5)).child(amount).child(
            sensitive(secondary, small.features("'tnum' 1"), tk.muted_fg, hide).mt(tk.r(0.125)),
        ))
}

fn flow_icon(f: Flow) -> &'static Icon {
    match f {
        Flow::Incoming => &I::ARROW_DOWN_RIGHT,
        Flow::Outgoing => &I::ARROW_UP_RIGHT,
        Flow::Transfer | Flow::Swap => &I::ARROW_LEFT_RIGHT,
    }
}

/// SYNTHETIC scene state: a finished run's panel (two progress records and a summary), without
/// sending the refresh.
pub fn scene_refresh_panel(app: &mut App) {
    let s = &mut app.reports.books.be;
    s.progress = vec![
        Progress {
            book: Some("Exchange Reconciliation".into()),
            phase: Some("journal_refresh".into()),
            source: None,
            line: None,
        },
        Progress {
            book: Some("Full Accounting".into()),
            phase: Some("backend_fetch".into()),
            source: Some("Regtest Spending Wallet".into()),
            line: None,
        },
    ];
    s.summary = Some(
        json!({"books": 2, "synced_books": 2, "reports_blocked": 0, "rate_limited": 0, "errors": 0}),
    );
    let _ = el;
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    fn snapshot(ws: &str, books: u64) -> Reply {
        Reply {
            kind: "ui.workspace.overview.snapshot".into(),
            data: json!({"workspace": {"id": ws}, "books": books}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// A book refresh (an exclusion elsewhere) reads the same set again; the answer from
    /// before the refresh does not land as current.
    #[test]
    fn a_book_refresh_reads_the_set_again() {
        let mut app = crate::app::tests::quiet_app();
        assert!(step(&mut app, "ws-1"));
        assert!(!step(&mut app, "ws-1"), "nothing changed: no new read");
        app.book_generation += 1;
        assert!(step(&mut app, "ws-1"), "refreshed: read again");
        on_reply(&mut app, "reports.books.be.snapshot", &snapshot("ws-1", 1));
        assert!(
            app.reports.books.be.data.is_none(),
            "the earlier read is dropped"
        );
        on_reply(&mut app, "reports.books.be.snapshot", &snapshot("ws-1", 2));
        assert!(app.reports.books.be.data.is_some());
        assert!(!app.reports.books.be.loading);
    }
}
