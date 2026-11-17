//! The book set rollup (routes/BirdsEye.tsx): `ui.workspace.overview.snapshot` as four metric
//! cards, the refresh panel, the read-only note, the shared BTC activity chart, the per-book
//! cards with their drill-down buttons, the per-book fiat rows and the recent activity list.
//!
//! "Refresh book set" (`ui.workspace.freshness.run`) syncs every book from its sources: network
//! egress, sent only from that button. Opening a book or a transaction switches the active book
//! (`ui.profiles.switch`) and then navigates, as the web does.

use rune::{Color, Cx, El, Icon, TextStyle, col, el, row, text};
use serde_json::{Value, json};

use super::kit::{amber, badge_outline, badge_secondary};
use super::list::{page, skeleton};
use crate::app::{App, Currency, Reply, ToastTone};
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::model::{self, Flow, RecentRow, Snapshot, TxStatus};
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, button_surface, button_with, spinner};
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

fn amber_badge(tk: &Tk, label: &str, glyph: Option<&'static Icon>) -> El {
    badge_outline(
        tk,
        label,
        glyph,
        amber(tk),
        rune::color::hex("#f59e0b")
            .unwrap_or(Color::WHITE)
            .alpha(0.4),
    )
}

/// `MetricCard`: `py-4 px-4 space-y-2`, icon + `text-xs` label, `text-xl` value, detail.
fn metric(tk: &Tk, key: &str, glyph: &'static Icon, label: &str, value: El, detail: El) -> El {
    surface(tk)
        .key(key.to_owned())
        .grow(1.0)
        .min_w(0.0)
        .px(tk.r(1.0))
        .py(tk.r(1.0))
        .gap(tk.r(0.5))
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(glyph, tk.r(1.0), tk.muted_fg))
                .child(
                    text(label.to_owned())
                        .font(tk.txs(500.0))
                        .color(tk.muted_fg),
                ),
        )
        .child(value)
        .child(detail)
}

fn metric_value(tk: &Tk, s: String, hidden: bool) -> El {
    // `text-xl font-semibold tracking-tight tabular-nums`; "Hidden" is a word, not a blur.
    let _ = hidden;
    text(s)
        .font(
            tk.txl(600.0)
                .features("'tnum' 1")
                .tracking(-tk.r(1.25) * 0.025),
        )
        .color(tk.fg)
}

fn detail(tk: &Tk, s: String, warn: bool) -> El {
    text(s)
        .font(tk.txs(400.0))
        .color(if warn { amber(tk) } else { tk.muted_fg })
        .wrap()
        .w_full()
}

/// A shadcn `Card` with a `border-b` header (`text-base` title) and a `grid gap-3` body.
fn card(tk: &Tk, key: &str, title: &str, body: El) -> El {
    surface(tk)
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .py(tk.card_pad())
        .gap(tk.r(1.0))
        .child(
            row()
                .w_full()
                .px(tk.card_pad())
                .pb(tk.card_pad())
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .child(
                    text(title.to_owned())
                        .font(tk.tbase(600.0).lh(tk.r(1.0)))
                        .color(tk.fg)
                        .role("heading"),
                ),
        )
        .child(body.px(tk.card_pad()).pt(tk.r(1.0)))
}

fn dashed(tk: &Tk, s: String) -> El {
    row()
        .w_full()
        .padding(tk.r(1.0))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_dash(4.0, 3.0)
        .child(
            text(s)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        )
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, ws: &str, layer: &mut Vec<El>) -> El {
    let s = &app.reports.books.be;
    if s.ws != ws || (s.loading && s.data.is_none()) {
        return skeleton(app, tk, 4, tk.r(11.0));
    }
    if let Some(e) = &s.error {
        return page(tk).child(
            surface(tk)
                .w_full()
                .px(tk.r(1.25))
                .py(tk.r(1.0))
                .fill(tk.destructive.alpha(0.1))
                .border(1.0, tk.destructive.alpha(0.3))
                .role("alert")
                .child(
                    text(e.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.destructive)
                        .wrap()
                        .w_full(),
                ),
        );
    }
    let empty = json!({});
    let d = s.data.as_ref().unwrap_or(&empty);
    let hide = app.prefs.hide_sensitive();
    let hidden = t("common:state.hidden");
    let win = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
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
    let title = d["workspace"]["label"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| t("overview:birdsEye.fallbackTitle"));

    // Header: the set's name and badges; Books and Refresh book set.
    let mut name = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(
            text(title)
                .font(tk.tlg(600.0).lh(tk.r(1.125) * 1.25))
                .color(tk.fg)
                .role("heading"),
        )
        .child(badge_secondary(tk, &t("overview:birdsEye.badge"), None));
    if mixed {
        name = name.child(badge_outline(
            tk,
            &t("overview:birdsEye.mixedFiatBadge"),
            None,
            tk.fg,
            tk.border,
        ));
    }
    let refreshing = s.refreshing;
    let refresh = refresh_button(tk, refreshing);
    let header = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(name)
        .child(
            row()
                .gap(tk.r(0.75))
                .center_y()
                .child(
                    button(
                        tk,
                        "be-books",
                        Variant::Outline,
                        Size::Sm,
                        None,
                        &t("overview:birdsEye.booksLink"),
                        true,
                    )
                    .px(tk.r(1.0))
                    .role("link")
                    .on_press(|a: &mut App| a.navigate("/books")),
                )
                .child(refresh),
        );

    // Metrics.
    let fc = fiat["fiatCurrency"].as_str().unwrap_or("EUR").to_owned();
    let btc = if hide {
        hidden.clone()
    } else {
        format::btc(fiat["btcBalance"].as_f64().unwrap_or(0.0), 8, false)
    };
    let fiat_value = if hide {
        hidden.clone()
    } else if mixed {
        t("overview:birdsEye.metric.mixed")
    } else if roll.2 {
        "—".into()
    } else {
        format::fiat(fiat["eurBalance"].as_f64().unwrap_or(0.0), &fc)
    };
    let fiat_detail = if basis_incomplete > 0 {
        detail(
            tk,
            tc(
                "overview:birdsEye.metric.basisIncompleteBooks",
                basis_incomplete,
                &[],
            ),
            true,
        )
    } else if roll.2 && !mixed {
        detail(tk, t("overview:completeness.valueUnavailable"), true)
    } else if mixed {
        detail(
            tk,
            fiat["label"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("overview:birdsEye.metric.perBookFiatOnly")),
            false,
        )
    } else {
        detail(
            tk,
            fiat["fiatCurrency"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("overview:birdsEye.metric.noFiatCurrency")),
            false,
        )
    };
    let cards = vec![
        metric(
            tk,
            "be-m-btc",
            &I::GAUGE,
            &t("overview:birdsEye.metric.btcTotal"),
            metric_value(tk, btc, hide),
            detail(
                tk,
                tc("overview:birdsEye.metric.books", books.len(), &[]),
                false,
            ),
        ),
        metric(
            tk,
            "be-m-fiat",
            &I::CHART_COLUMN,
            &t(if mixed {
                "overview:birdsEye.metric.fiatRollup"
            } else {
                "overview:birdsEye.metric.fiatTotal"
            }),
            metric_value(tk, fiat_value, hide),
            fiat_detail,
        ),
        metric(
            tk,
            "be-m-ready",
            &I::CIRCLE_CHECK,
            &t("overview:birdsEye.metric.readiness"),
            metric_value(tk, format!("{ready}/{}", books.len()), false),
            detail(
                tk,
                if blocked > 0 {
                    tc("overview:birdsEye.metric.booksNeedAttention", blocked, &[])
                } else {
                    t("overview:birdsEye.metric.allBooksReady")
                },
                false,
            ),
        ),
        metric(
            tk,
            "be-m-quarantine",
            &I::SHIELD_ALERT,
            &t("overview:birdsEye.metric.quarantine"),
            metric_value(
                tk,
                d["status"]["quarantines"].as_u64().unwrap_or(0).to_string(),
                false,
            ),
            detail(
                tk,
                t(if d["status"]["needsJournals"].as_bool().unwrap_or(false) {
                    "overview:birdsEye.metric.journalsNeedProcessing"
                } else {
                    "overview:birdsEye.metric.noStaleJournals"
                }),
                false,
            ),
        ),
    ];
    // `grid gap-3 md:grid-cols-2 xl:grid-cols-4`.
    let per_row = if win >= 1280.0 {
        4
    } else if win >= 768.0 {
        2
    } else {
        1
    };
    let mut grid = col().w_full().gap(tk.card_gap());
    let mut it = cards.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().gap(tk.card_gap()).items_stretch();
        for _ in 0..per_row {
            if let Some(c) = it.next() {
                r = r.child(c);
            }
        }
        grid = grid.child(r);
    }

    let mut out = page(tk)
        .busy(s.loading || refreshing)
        .child(header)
        .child(grid);
    if refreshing || !s.progress.is_empty() || s.summary.is_some() {
        out = out.child(refresh_panel(tk, s, win));
    }
    // The read-only note: `rounded-lg border bg-muted/25 px-3 py-2 text-xs`.
    out = out.child(
        row()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .gap(tk.r(0.375))
            .items_start()
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .fill(tk.muted.alpha(0.25))
            .child(ic(&I::INFO, tk.r(0.875), tk.muted_fg).mt(tk.r(0.0625)))
            .child(
                text(t("overview:birdsEye.readonlyNote"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            ),
    );
    if let Some(snap) = s.chart.as_deref() {
        out = out.child(crate::chart::view(app, cx, tk, snap, layer));
    }

    // Books and Fiat rows: `xl:grid-cols-[minmax(0,1fr)_360px]`.
    let disabled = s.switching;
    let mut book_list = col().w_full().gap(tk.r(0.75));
    if books.is_empty() {
        book_list = book_list.child(dashed(tk, t("overview:birdsEye.booksCard.empty")));
    }
    for b in &books {
        book_list = book_list.child(book_row(tk, b, hide, disabled, win));
    }
    let mut fiat_list = col().w_full().gap(tk.r(0.5));
    if fiat_books.is_empty() {
        fiat_list = fiat_list.child(dashed(tk, t("overview:birdsEye.fiatCard.empty")));
    }
    for r in &fiat_books {
        fiat_list = fiat_list.child(fiat_row(tk, r, hide));
    }
    let books_card = card(
        tk,
        "be-books-card",
        &t("overview:birdsEye.booksCard.title"),
        book_list,
    );
    let fiat_card = card(
        tk,
        "be-fiat-card",
        &t("overview:birdsEye.fiatCard.title"),
        fiat_list,
    );
    out = out.child(if win >= 1280.0 {
        row()
            .w_full()
            .gap(tk.card_gap())
            .items_start()
            .child(books_card.grow(1.0))
            .child(fiat_card.w(360.0))
    } else {
        col()
            .w_full()
            .gap(tk.card_gap())
            .child(books_card)
            .child(fiat_card)
    });
    if let Some(rs) = s.recent.as_deref() {
        out = out.child(recent_table(app, tk, d, rs));
    }
    out
}

/// The primary `h-8 gap-2` Refresh book set button; a spinner replaces its glyph while the run
/// lasts, and it is disabled then.
fn refresh_button(tk: &Tk, running: bool) -> El {
    let label = t("overview:birdsEye.refresh");
    let glyph = if running {
        ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.primary_fg).looping(rune::LoopSpec::rotation(0.9))
    } else {
        ic(&I::REFRESH_CW, tk.r(1.0), tk.primary_fg)
    };
    let mut b = button_surface(
        tk,
        row()
            .key("be-refresh")
            .h(tk.r(2.0))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center(),
        Variant::Default,
    )
    .role("button")
    .label(label.clone())
    .focusable(!running)
    .disabled(running)
    .busy(running)
    .child(glyph)
    .child(text(label).font(tk.tsm(500.0)).color(tk.primary_fg));
    if running {
        b = b.opacity(0.5);
    } else {
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

/// `BookRow` (the per-book card): name and badges, counts, the hint when not ready, BTC and
/// fiat, then one button per book page.
fn book_row(tk: &Tk, b: &Value, hide: bool, disabled: bool, win: f32) -> El {
    let hidden = t("common:state.hidden");
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
    let mut badges = row()
        .gap(tk.r(0.5))
        .center_y()
        .child(text(label.clone()).font(tk.tsm(500.0)).color(tk.fg));
    // A badge only when the book needs attention; a ready book shows none.
    if !ready {
        badges = badges.child(badge_outline(
            tk,
            &t("overview:birdsEye.bookRow.attention"),
            Some(&I::TRIANGLE_ALERT),
            tk.fg,
            tk.border,
        ));
    }
    badges = badges.child(badge_outline(tk, &fc, None, tk.fg, tk.border));
    if let Some(k) = badge_key(&comp) {
        badges = badges.child(amber_badge(tk, &t(k), None));
    }
    let n = |v: &Value| v.as_u64().unwrap_or(0) as usize;
    let counts = row()
        .gap(tk.r(1.0))
        .child(
            text(tc(
                "overview:birdsEye.bookRow.wallets",
                b["connections"].as_array().map(|a| a.len()).unwrap_or(0),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        )
        .child(
            text(tc(
                "overview:birdsEye.bookRow.txs",
                n(&b["status"]["transactionCount"]),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        )
        .child(
            text(tc(
                "overview:birdsEye.bookRow.journalRows",
                n(&b["journals"]["journal_entry_count"]),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        )
        .child(
            text(tc(
                "overview:birdsEye.bookRow.quarantines",
                n(&b["journals"]["quarantine_count"]),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    let mut left = col().min_w(0.0).gap(tk.r(0.5)).child(badges).child(counts);
    if !ready {
        let hint = b["readiness"]["hints"][0]
            .as_str()
            .or_else(|| b["journals"]["reason"].as_str())
            .unwrap_or_default()
            .to_owned();
        left = left.child(text(hint).font(tk.txs(400.0)).color(amber(tk)).wrap());
    }
    let btc_s = if hide {
        hidden.clone()
    } else {
        format::btc(btc, 8, false)
    };
    let fiat_s = if hide {
        hidden
    } else if comp.2 {
        "—".into()
    } else {
        format::fiat(b["fiat"]["eurBalance"].as_f64().unwrap_or(0.0), &fc)
    };
    // `grid gap-1 text-sm sm:grid-cols-2 lg:min-w-[260px]`.
    let amounts = row()
        .gap(tk.r(0.25))
        .child(
            text(btc_s)
                .font(tk.tsm(400.0).features("'tnum' 1"))
                .color(tk.fg)
                .grow(1.0),
        )
        .child(
            text(fiat_s)
                .font(tk.tsm(400.0).features("'tnum' 1"))
                .color(tk.fg)
                .grow(1.0),
        );
    let top = if win >= 1024.0 {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .justify_between()
            .child(left.grow(1.0))
            .child(amounts.min_w(260.0))
    } else {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(left)
            .child(amounts.w_full())
    };
    let mut buttons = row().w_full().gap(tk.r(0.5)).flex_wrap().mt(tk.r(0.75));
    for (id, key, route, glyph) in BOOK_ROUTES {
        let (p, l) = (pid.clone(), label.clone());
        let mut btn = button_with(
            tk,
            format!("book-route-{pid}-{id}"),
            Variant::Outline,
            Size::Sm,
            Some(glyph),
            Some(tk.r(0.875)),
            &t(key),
            !disabled,
        );
        if !disabled {
            btn = btn.on_press(move |a: &mut App| open_book_route(a, &p, &l, key, route));
        }
        buttons = buttons.child(btn);
    }
    surface(tk)
        .key(format!("be-book-{pid}"))
        .w_full()
        .padding(tk.r(0.75))
        .role("group")
        .label(label)
        .child(top)
        .child(buttons)
}

/// `FiatBookRow`: the book's fiat balance, basis and YTD (basis-derived figures "—" while the
/// basis isn't exact; the value needs a market rate). Its `title` hint becomes the help text.
fn fiat_row(tk: &Tk, r: &Value, hide: bool) -> El {
    let comp = completeness(&r["completeness"]);
    let fc = r["fiatCurrency"].as_str().unwrap_or("EUR").to_owned();
    let basis_hidden = !comp.1;
    let hint = if !comp.1 {
        Some(t(if comp.0 == "stale" {
            "overview:completeness.hint.stale"
        } else {
            "overview:completeness.hint.incomplete"
        }))
    } else if comp.2 {
        Some(t("overview:completeness.hint.unavailable"))
    } else {
        None
    };
    let money = |v: f64, h: bool| {
        if hide {
            t("common:state.hidden")
        } else if h {
            "—".to_owned()
        } else {
            format::fiat(v, &fc)
        }
    };
    let label = r["profileLabel"].as_str().unwrap_or_default().to_owned();
    let mut name = row().gap(tk.r(0.5)).center_y().child(
        text(label.clone())
            .font(tk.tsm(500.0))
            .color(tk.fg)
            .truncate(),
    );
    if let Some(k) = badge_key(&comp) {
        name = name.child(amber_badge(tk, &t(k), Some(&I::TRIANGLE_ALERT)));
    }
    let mut el = col()
        .key(format!(
            "be-fiat-{}",
            r["profileId"].as_str().unwrap_or_default()
        ))
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.2))
        .role("group")
        .label(label)
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_start()
                .justify_between()
                .child(
                    col().min_w(0.0).grow(1.0).child(name).child(
                        text(fc.clone())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .mt(tk.r(0.25)),
                    ),
                )
                .child(
                    text(money(r["balance"].as_f64().unwrap_or(0.0), comp.2))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                ),
        )
        .child(
            row()
                .w_full()
                .mt(tk.r(0.75))
                .gap(tk.r(0.5))
                .child(
                    text(tr(
                        "overview:birdsEye.fiatCard.basis",
                        &[(
                            "value",
                            &money(r["costBasis"].as_f64().unwrap_or(0.0), basis_hidden),
                        )],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .grow(1.0),
                )
                .child(
                    text(tr(
                        "overview:birdsEye.fiatCard.ytd",
                        &[(
                            "value",
                            &money(r["realizedYTD"].as_f64().unwrap_or(0.0), basis_hidden),
                        )],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .grow(1.0),
                ),
        );
    if let Some(h) = hint {
        el = el.help(h.clone()).description(h);
    }
    el
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

fn recent_table(app: &App, tk: &Tk, d: &Value, s: &Snapshot) -> El {
    let rows = model::recent_rows(s);
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
    let body = if rows.is_empty() {
        col().w_full().padding(tk.r(1.0)).child(
            row()
                .w_full()
                .h(96.0)
                .center()
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_dash(4.0, 3.0)
                .child(
                    text(t("overview:recentTx.empty"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        )
    } else {
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
    surface(tk)
        .border_layout()
        .key("be-recent")
        .w_full()
        .clip_rounded(tk.r(0.75))
        .child(header)
        .child(hairline(tk.border))
        .child(body)
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
        let (sb, sf, sr) = status_chip(tk, r.status);
        meta = meta.child(
            row()
                .px(tk.r(0.25))
                .radius(tk.r(0.375))
                .fill(sb)
                .border(1.0, sr)
                .child(
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
