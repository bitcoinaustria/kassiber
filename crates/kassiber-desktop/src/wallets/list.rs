//! routes/Connections.tsx: the Wallets list — `WalletsDashboardHeader` (Add wallet),
//! `WalletsMetricGrid`, and the card with `WalletsFilters` (Filters menu with Status / Kind
//! radio submenus, active-filter chips) over `WalletsTable` (sortable, rows open the detail).

use std::rc::Rc;

use rune::{Color, Cx, El, Key, KeyEvent, Shadow, Tx, col, el, row, text};
use rune_kit::Placement;
use serde_json::Value;

use super::common::asset_badge;
use super::conn::{Conn, all, kind_category_label, snapshot};
use crate::app::{App, Currency, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::{self, TriggerExt};
use crate::ui::{ic, sensitive, surface};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Label,
    Kind,
    Transactions,
    Last,
    TaxFree,
    Balance,
}

impl SortKey {
    /// `defaultSortDir`: the first click starts in the most useful direction.
    fn default_asc(self) -> bool {
        matches!(self, SortKey::Label | SortKey::Kind)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    Filters,
    FiltersSub(&'static str),
}

pub struct State {
    /// `kindFilter`: a category label, or None for all.
    pub kind: Option<String>,
    /// `statusFilter`.
    pub status: Option<&'static str>,
    /// The remembered sort (`useRememberedTableSort("wallets")`, default kind ascending).
    pub sort: Option<(SortKey, bool)>,
    pub popup: Option<Popup>,
}

impl Default for State {
    fn default() -> Self {
        State {
            kind: None,
            status: None,
            sort: Some((SortKey::Kind, true)),
            popup: None,
        }
    }
}

const STATUSES: [&str; 4] = ["synced", "syncing", "idle", "error"];

/// `kindFilterOptions`: the distinct kind categories in declaration order, then "Liquid".
pub fn kind_options() -> Vec<&'static str> {
    const KINDS: [&str; 23] = [
        "xpub",
        "address",
        "descriptor",
        "silent-payment",
        "samourai",
        "core-ln",
        "lnd",
        "nwc",
        "cashu",
        "btcpay",
        "kraken",
        "bitstamp",
        "coinbase",
        "bitpanda",
        "river",
        "bullbitcoin",
        "coinfinity",
        "strike",
        "phoenix",
        "custom",
        "csv",
        "bip329",
        "backend",
    ];
    let mut v: Vec<&'static str> = Vec::new();
    for k in KINDS {
        let c = kind_category_label(k);
        if !v.contains(&c) {
            v.push(c);
        }
    }
    if !v.contains(&"Liquid") {
        v.push("Liquid");
    }
    v
}

fn win_w(app: &App) -> f32 {
    if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    }
}

/// A BTC balance in the book's currency at its market rate, as the wallet detail hero values it
/// (`wallets::detail::fiat`), so the list and the detail never disagree on unit or rate.
fn fiat_value(app: &App, btc: f64) -> String {
    let (cur, rate) = crate::wallets::detail::fiat(app);
    crate::format::fiat(btc * rate, &cur)
}

/// `formatBtc` (wallets/format.ts): eight decimals.
pub fn format_btc8(v: f64) -> String {
    format!("{v:.8}")
}

/// A share with `places` decimals in the UI language's notation (de-AT: decimal comma).
fn decimal(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    match crate::i18n::lang() {
        crate::i18n::Lang::En => s,
        crate::i18n::Lang::De => s.replace('.', ","),
    }
}

/// `formatBtc(value, {precision})` from lib/currency: `₿ 0.123`.
fn display_btc(v: f64, precision: usize) -> String {
    format!("₿ {:.*}", precision, v)
}

fn ts_ms(s: &str) -> Option<f64> {
    crate::format::parse_ts(s)
}

/// `activityLabel`: never / just now / Nm / Nh / Nd ago, then the ISO date.
fn activity_label(c: &Conn) -> String {
    let Some(secs) = c.last_transaction_at.as_deref().and_then(ts_ms) else {
        return t("connections:table.never");
    };
    let now = crate::format::now_secs();
    let diff = (now - secs).max(0.0).floor() as i64;
    if diff < 60 {
        return t("overview:marketRate.justNow");
    }
    let ago = |key: &str, n: i64| tr(key, &[("count", &n.to_string())]);
    let m = diff / 60;
    if m < 60 {
        return ago("overview:marketRate.minutesAgo", m);
    }
    let h = m / 60;
    if h < 24 {
        return ago("overview:marketRate.hoursAgo", h);
    }
    let d = h / 24;
    if d < 30 {
        return ago("overview:marketRate.daysAgo", d);
    }
    crate::format::Day::from_epoch_days((secs / 86400.0).floor() as i64).iso()
}

/// `Intl.Collator(undefined, {sensitivity: "base", numeric: true})`, close enough: case- and
/// accent-folded, digit runs compared numerically.
pub fn collate(a: &str, b: &str) -> std::cmp::Ordering {
    fn fold(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for ch in s.chars() {
            let d = ch.is_ascii_digit();
            let c = fold_char(ch);
            match out.last_mut() {
                Some((dd, buf)) if *dd == d => buf.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    }
    fn fold_char(c: char) -> char {
        let l = c.to_lowercase().next().unwrap_or(c);
        match l {
            'ä' | 'á' | 'à' | 'â' => 'a',
            'ö' | 'ó' | 'ò' | 'ô' => 'o',
            'ü' | 'ú' | 'ù' | 'û' => 'u',
            'é' | 'è' | 'ê' => 'e',
            'ß' => 's',
            x => x,
        }
    }
    let (fa, fb) = (fold(a), fold(b));
    for (x, y) in fa.iter().zip(fb.iter()) {
        let o = if x.0 && y.0 {
            let (nx, ny) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
            nx.len().cmp(&ny.len()).then(nx.cmp(ny))
        } else {
            // Punctuation and spaces sort before letters and digits, as ICU's do.
            let key = |s: &str| {
                s.chars()
                    .map(|c| (c.is_alphanumeric(), c))
                    .collect::<Vec<_>>()
            };
            key(&x.1).cmp(&key(&y.1))
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    fa.len().cmp(&fb.len())
}

/// `compareWalletsTableConnections` (ascending).
pub fn compare(a: &Conn, b: &Conn, key: SortKey, tax_free: &[String]) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    match key {
        SortKey::Label => collate(&a.label, &b.label),
        SortKey::Kind => a
            .category_sort_rank()
            .cmp(&b.category_sort_rank())
            .then_with(|| collate(a.category_label(), b.category_label()))
            .then_with(|| collate(&a.type_label(), &b.type_label()))
            .then_with(|| collate(&a.label, &b.label)),
        SortKey::Transactions => a
            .transaction_count
            .unwrap_or(0.0)
            .partial_cmp(&b.transaction_count.unwrap_or(0.0))
            .unwrap_or(Equal),
        SortKey::Last => {
            let ma = a.last_transaction_at.as_deref().and_then(ts_ms);
            let mb = b.last_transaction_at.as_deref().and_then(ts_ms);
            match (ma, mb) {
                (None, None) => Equal,
                (None, _) => Less,
                (_, None) => Greater,
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Equal),
            }
        }
        SortKey::TaxFree => {
            let ra = tax_free.contains(&a.id) as i32;
            let rb = tax_free.contains(&b.id) as i32;
            ra.cmp(&rb).then_with(|| collate(&a.label, &b.label))
        }
        SortKey::Balance => a.balance.partial_cmp(&b.balance).unwrap_or(Equal),
    }
}

/// The snapshot's `taxFreeBalance` status (`taxFreeStatusKey`).
fn tax_free_status(tfb: &Value) -> &str {
    if let Some(s) = tfb.get("status").and_then(Value::as_str) {
        return s;
    }
    if tfb.get("needsJournals").and_then(Value::as_bool) == Some(true) {
        return "needs_journals";
    }
    if tfb
        .get("quarantines")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        > 0.0
    {
        return "quarantines";
    }
    "current"
}

fn tax_free(app: &App) -> Option<&Value> {
    snapshot(app)
        .and_then(|s| s.get("taxFreeBalance"))
        .filter(|v| v.is_object())
}

/// Wallet ids with a tax-free balance, when the tax-free column shows (`taxFreeBalanceIsCurrent`).
fn tax_free_ids(app: &App) -> Option<Vec<String>> {
    let tfb = tax_free(app)?;
    if tax_free_status(tfb) != "current" {
        return None;
    }
    Some(
        tfb.get("wallets")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|w| w.get("hasTaxFreeBalance").and_then(Value::as_bool) == Some(true))
                    .filter_map(|w| w.get("walletId").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// `eurRateMissing(snapshot)`.
fn rate_missing(app: &App) -> bool {
    let Some(s) = snapshot(app) else { return true };
    let price = s.get("priceEur").and_then(Value::as_f64).unwrap_or(0.0);
    if price <= 0.0 || price.is_nan() {
        return true;
    }
    let book = s
        .pointer("/marketRate/fiatCurrency")
        .and_then(Value::as_str)
        .unwrap_or("EUR")
        .to_uppercase();
    book == "EUR"
        && s.pointer("/fiat/completeness/marketRateMissing")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// `walletOverviewTotalBtc`: the summary's total, else the sum of balances.
fn total_btc(app: &App, conns: &[Conn]) -> f64 {
    snapshot(app)
        .and_then(|s| s.pointer("/balanceSummary/totalBtc"))
        .and_then(Value::as_f64)
        .unwrap_or_else(|| conns.iter().map(|c| c.balance).sum())
}

fn filtered(app: &App, conns: &[Conn]) -> Vec<Conn> {
    let st = &app.wallets.list;
    conns
        .iter()
        .filter(|c| {
            st.kind.as_deref().is_none_or(|k| c.category_label() == k)
                && st.status.is_none_or(|s| c.status == s)
        })
        .cloned()
        .collect()
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let page = col()
        .key("wallets-page")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    let Some(_) = snapshot(app) else {
        return page.child(skeleton(tk));
    };
    let conns = all(app);
    let shown = filtered(app, &conns);
    let total = total_btc(app, &conns);
    page.child(header(app, tk))
        .child(metrics(app, cx, tk, &conns, total))
        .child(
            surface(tk)
                .w_full()
                .clip_rounded(tk.r(0.75))
                .child(filters(app, tk, layer, shown.len()))
                .child(table(app, cx, tk, &shown, total, conns.len())),
        )
}

/// The sync line on the header's left (an owner request; the web's header holds only `Add wallet`):
/// "Syncing…" while a refresh runs, else the newest successful sync across the sources, with the
/// exact time on hover, and the failed-source count when any failed.
pub fn sync_line(app: &App, tk: &Tk) -> El {
    let mut r = row()
        .key("wallets-sync-line")
        .gap(tk.r(0.5))
        .center_y()
        .min_w(0.0)
        .role("status");
    if app.busy.syncing {
        return r.child(
            text(t("chrome:shell.freshness.syncing"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    }
    let sources = app
        .read("ui.freshness.status")
        .and_then(|f| f["sources"].as_array().cloned())
        .unwrap_or_default();
    let latest = sources
        .iter()
        .filter_map(|x| x["last_success_at"].as_str())
        .filter(|s| !s.is_empty())
        .max()
        .map(str::to_owned);
    let failed = sources.iter().filter(|x| x["status"] == "failed").count();
    if let Some(at) = latest {
        let when = crate::overview::l10n::relative(&at, crate::format::now_secs())
            .unwrap_or_else(|| crate::format::short_date(Some(&at)));
        r = r.child(
            text(tr("overview:holdings.lastSyncedAt", &[("date", &when)]))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .truncate()
                .help(tr(
                    "overview:holdings.lastSyncedAt",
                    &[("date", &crate::format::short_date(Some(&at)))],
                )),
        );
    }
    if failed > 0 {
        r = r.child(crate::ui::dot(tk.r(0.375), tk.alert.solid)).child(
            text(t("chrome:shell.freshness.failed"))
                .font(tk.tsm(500.0))
                .color(tk.alert.solid),
        );
    }
    r
}

/// `WalletsDashboardHeader`: the sync line, then a right-aligned `Add wallet` (`size="sm" h-8 gap-2 min-w-[112px]`).
fn header(app: &App, tk: &Tk) -> El {
    let label = t("connections:dashboard.addWallet");
    row()
        .w_full()
        .justify_between()
        .center_y()
        .gap(tk.r(1.0))
        .child(sync_line(app, tk))
        .child(
            row()
                .key("wallets-add")
                .h(tk.r(2.0))
                .min_w(112.0)
                .px(tk.r(0.625))
                .gap(tk.r(0.5))
                .center()
                .radius(tk.r(0.375))
                .fill(tk.primary)
                .hover(tk.primary.alpha(0.9))
                .role("button")
                .label(label.clone())
                .focusable(true)
                .on_press(|a: &mut App| super::add::open(a, None))
                .child(ic(&I::PLUS, tk.r(1.0), tk.primary_fg))
                .child(text(label).font(tk.tsm(500.0)).color(tk.primary_fg)),
        )
}

/// `CurrencyToggleText`: the amount names the control, the switch it makes is its tooltip.
fn currency_toggle(
    app: &App,
    tk: &Tk,
    key: &str,
    s: String,
    style: rune::TextStyle,
    color: Color,
) -> El {
    let hint = t(if app.prefs.currency == Currency::Btc {
        "chrome:currencyToggleText.showFiat"
    } else {
        "chrome:currencyToggleText.showBitcoin"
    });
    let hidden = app.prefs.hide_sensitive();
    row()
        .key(key.to_owned())
        .radius(tk.r(0.125))
        .role("button")
        .label(if hidden {
            crate::ui::hidden_amount()
        } else {
            s.clone()
        })
        .help(hint)
        .focusable(true)
        .on_press(|a: &mut App| {
            a.prefs.currency = if a.prefs.currency == Currency::Btc {
                Currency::Fiat
            } else {
                Currency::Btc
            };
        })
        .child(sensitive(s, style, color, hidden))
}

/// The top stats (`WalletsMetricGrid`) as the home screen's stat tiles (an owner request): the
/// balance with its trend, the tax-free split where the book has one, the transaction count, and
/// only while something is wrong the sync and failure tiles ("Up to date 13/13" and "Needs
/// attention 0" say nothing; the header's "Last synced" line keeps the sync state in view).
fn metrics(app: &App, cx: &Cx<App>, tk: &Tk, conns: &[Conn], total: f64) -> El {
    use crate::overview::grid::stat::{Stat, row_of};
    let fiat_mode = app.prefs.currency == Currency::Fiat;
    let missing = rate_missing(app);
    let fiat_unavailable = fiat_mode && missing;
    let hidden = app.prefs.hide_sensitive();
    let bs = snapshot(app)
        .and_then(|s| s.get("balanceSummary"))
        .filter(|v| v.is_object());
    let bs_q = bs
        .and_then(|b| b.get("quarantines"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let balance_detail = if fiat_unavailable {
        t("connections:metrics.balanceNoMarketRate")
    } else if bs
        .and_then(|b| b.get("needsJournals"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        t("connections:metrics.balanceNeedsJournals")
    } else if bs_q > 0.0 {
        tc("connections:metrics.balanceQuarantines", bs_q as usize, &[])
    } else {
        match bs
            .and_then(|b| b.get("source"))
            .and_then(Value::as_str)
            .unwrap_or("mixed")
        {
            "chain" => t("connections:metrics.balanceChain"),
            "books" => t("connections:metrics.balanceBooks"),
            "transactions" => t("connections:metrics.balanceTransactions"),
            _ => t("connections:metrics.balanceMixed"),
        }
    };
    let total_tx: f64 = conns
        .iter()
        .map(|c| c.transaction_count.unwrap_or(0.0))
        .sum();
    let errors = conns.iter().filter(|c| c.status == "error").count();
    let snap_syncing = conns.iter().filter(|c| c.status == "syncing").count();
    let syncing = if app.busy.syncing {
        conns.len()
    } else {
        snap_syncing
    };
    let synced = conns.iter().filter(|c| c.status == "synced").count();
    let unsynced = conns.len() - synced;
    let up_detail = if syncing > 0 {
        tc(
            "connections:metrics.upToDateRefreshingNow",
            syncing,
            &[("count", &crate::format::count(syncing))],
        )
    } else {
        tc(
            "connections:metrics.upToDateNotUpToDate",
            unsynced,
            &[("count", &crate::format::count(unsynced))],
        )
    };
    let balance_text = if fiat_unavailable {
        "—".to_owned()
    } else if fiat_mode {
        fiat_value(app, total)
    } else {
        format!("₿ {}", format_btc8(total))
    };
    // The balance's shape over the book's history (the overview snapshot's series).
    let series: Vec<f64> = app
        .snapshot()
        .map(|s| s.balance_series.clone())
        .unwrap_or_default();
    let orange = rune::color::hex("#f7931a").unwrap_or(tk.fg);
    let (fg, corner) = (tk.fg, tk.r(0.125));
    let hint = t(if fiat_mode {
        "chrome:currencyToggleText.showBitcoin"
    } else {
        "chrome:currencyToggleText.showFiat"
    });
    let shown = balance_text.clone();
    let mut stats = vec![
        Stat::custom(
            "wallets-metric-balance",
            &I::BITCOIN,
            t("connections:metrics.walletBalance"),
            &shown,
            move |style| {
                // The amount is the currency toggle (`CurrencyToggleText`), in the tile's figure style.
                row()
                    .key("wallets-metric-balance-toggle")
                    .min_w(0.0)
                    .radius(corner)
                    .role("button")
                    .label(if hidden {
                        crate::ui::hidden_amount()
                    } else {
                        balance_text.clone()
                    })
                    .help(hint)
                    .focusable(true)
                    .on_press(|a: &mut App| {
                        a.prefs.currency = if a.prefs.currency == Currency::Btc {
                            Currency::Fiat
                        } else {
                            Currency::Btc
                        };
                    })
                    .child(
                        sensitive(balance_text, style, fg, hidden)
                            .single_line()
                            .truncate()
                            .min_w(0.0),
                    )
            },
        )
        .line(balance_detail)
        .spark(series, orange),
    ];
    if let Some(tfb) = tax_free(app) {
        let status = tax_free_status(tfb);
        let current = status == "current";
        let q = tfb
            .get("quarantines")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let sats = |k: &str| tfb.get(k).and_then(Value::as_f64).unwrap_or(0.0) / 100_000_000.0;
        let tone = match status {
            "quarantines" => Some(tk.alert.solid),
            "needs_journals" => Some(tk.warn.solid),
            _ => None,
        };
        let value = if current {
            display_btc(sats("taxFreeQuantitySats"), 3)
        } else if status == "needs_journals" {
            t("connections:metrics.taxFreeNeedsJournals")
        } else {
            tc("connections:metrics.taxFreeQuarantines", q as usize, &[])
        };
        let detail = if current {
            tr(
                "connections:metrics.taxFreeTaxable",
                &[("taxable", &display_btc(sats("taxableQuantitySats"), 3))],
            )
        } else if status == "quarantines" {
            t("connections:metrics.taxFreeBlockedQuarantines")
        } else {
            t("connections:metrics.taxFreeBlockedJournals")
        };
        let mut st = Stat::new(
            "wallets-metric-taxfree",
            &I::RECEIPT,
            t("connections:metrics.taxFreeBalance"),
            value,
        )
        .sensitive(current)
        .line_with(detail, tone, current);
        if let Some(c) = tone {
            st = st.ink(c).glyph_ink(c);
        }
        stats.push(st);
    }
    stats.push(
        Stat::new(
            "wallets-metric-transactions",
            &I::LIST,
            t("connections:metrics.totalTransactions"),
            crate::format::count(total_tx as usize),
        )
        .line(t("connections:metrics.totalTransactionsDetail"))
        .link(t("connections:metrics.openTransactions"), |a: &mut App| {
            a.navigate("/transactions")
        }),
    );
    if syncing > 0 || unsynced > 0 {
        stats.push(
            Stat::new(
                "wallets-metric-uptodate",
                &I::REFRESH_CW,
                t("connections:metrics.upToDate"),
                format!(
                    "{} / {}",
                    crate::format::count(synced),
                    crate::format::count(conns.len())
                ),
            )
            .line(up_detail)
            .glyph_ink(tk.warn.solid),
        );
    }
    if errors > 0 {
        stats.push(
            Stat::new(
                "wallets-metric-attention",
                &I::SHIELD_ALERT,
                t("connections:metrics.needsAttention"),
                crate::format::count(errors),
            )
            .ink(tk.alert.solid)
            .glyph_ink(tk.alert.solid)
            .line(t("connections:metrics.needsAttentionFailed"))
            .link(t("connections:metrics.openReview"), |a: &mut App| {
                a.navigate("/transactions?quick=review_queue")
            }),
        );
    }
    row_of(
        app,
        cx,
        tk,
        "wallets-metrics",
        t("connections:metrics.walletBalance"),
        stats,
    )
}

// ---- Filters ---------------------------------------------------------------------------------

fn close_popup(a: &mut App) {
    a.wallets.list.popup = None;
}

/// `WalletsFilters`: the heading, its count, the Filters menu and the active chips.
fn filters(app: &App, tk: &Tk, layer: &mut Vec<El>, count: usize) -> El {
    let st = &app.wallets.list;
    let active = st.status.is_some() as usize + st.kind.is_some() as usize;
    let sm = win_w(app) >= 640.0;
    let menu_label = t("connections:filters.menu");
    let open = st.popup.is_some();
    let (bfill, bborder, bhover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input,
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (tk.card, tk.border, tk.accent)
    };
    let mut trigger = row()
        .key("wallets-filters")
        .h(tk.r(if sm { 2.25 } else { 2.0 }))
        .px(tk.r(0.625))
        .gap(tk.r(if sm { 0.5 } else { 0.375 }))
        .center()
        .radius(tk.r(0.375))
        .fill(bfill)
        .border(1.0, if active > 0 { tk.primary } else { bborder })
        .border_layout()
        .hover(bhover)
        .role("button")
        .label(menu_label.clone())
        .focusable(true)
        .menu_trigger(open, |a: &mut App| {
            a.wallets.list.popup = if a.wallets.list.popup.is_some() {
                None
            } else {
                Some(Popup::Filters)
            }
        })
        .child(ic(
            &I::SLIDERS_HORIZONTAL,
            tk.r(if sm { 1.0 } else { 0.875 }),
            tk.fg,
        ));
    if tk.dark {
        trigger = trigger.box_shadow(Shadow {
            y: 1.0,
            color: Color::WHITE.alpha(0.06),
            inset: true,
            ..Shadow::default()
        });
    } else {
        trigger = trigger.box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        });
    }
    if sm {
        trigger = trigger.child(text(menu_label.clone()).font(tk.tsm(500.0)).color(tk.fg));
    }
    if active > 0 {
        trigger = trigger.child(
            row()
                .min_w(tk.r(1.0))
                .h(tk.r(1.0))
                .px(tk.r(0.25))
                .center()
                .radius(tk.r(0.5))
                .fill(tk.primary)
                .child(
                    text(active.to_string())
                        .font(tk.t2xs(600.0).lh(tk.r(1.0)))
                        .color(tk.primary_fg),
                ),
        );
    }
    let count_badge = crate::ui::count(tk, count, if sm { tk.tbase(400.0) } else { tk.tsm(400.0) });
    let head = row()
        .w_full()
        .px(if sm { tk.card_pad() } else { tk.r(0.75) })
        .py(tk.r(if sm { 0.875 } else { 0.75 }))
        .gap(tk.r(1.0))
        .center_y()
        // `SurfaceCardHeader`: the title is the card's `h2`.
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.5))
                .center_y()
                .flex_wrap()
                .child(
                    text(t("connections:filters.heading"))
                        .font(if sm { tk.tbase(500.0) } else { tk.tsm(500.0) })
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                )
                .child(count_badge),
        )
        .child(trigger);
    if open {
        filters_menu(app, tk, layer);
    }
    let mut out = col().w_full().child(head);
    if st.status.is_some() || st.kind.is_some() {
        let chip_font = if sm { tk.txs(500.0) } else { tk.t2xs(500.0) };
        let small = if sm { tk.txs(400.0) } else { tk.t2xs(400.0) };
        let chip = |key: &str, label: String, aria: String| {
            row()
                .key(key.to_owned())
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .gap(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.375))
                .fill(tk.background)
                .border(1.0, tk.border)
                .border_layout()
                .hover(tk.muted)
                .role("button")
                .label(aria)
                .focusable(true)
                .child(text(label).font(chip_font).color(tk.muted_fg))
                .child(ic(&I::X, tk.r(if sm { 0.75 } else { 0.625 }), tk.muted_fg))
        };
        let mut chips = row()
            .w_full()
            .flex_wrap()
            .gap(tk.r(0.5))
            .center_y()
            .px(if sm { tk.card_pad() } else { tk.r(0.75) })
            .pb(tk.r(0.75))
            .child(
                text(t("connections:filters.label"))
                    .font(small)
                    .color(tk.muted_fg),
            );
        if let Some(s) = st.status {
            let name = t(&format!("chrome:connectionStatus.{s}"));
            chips = chips.child(
                chip(
                    "wallets-chip-status",
                    name.clone(),
                    tr("connections:filters.clearStatus", &[("status", &name)]),
                )
                .on_press(|a: &mut App| a.wallets.list.status = None),
            );
        }
        if let Some(k) = &st.kind {
            chips = chips.child(
                chip(
                    "wallets-chip-kind",
                    super::conn::category_display(k),
                    tr(
                        "connections:filters.clearKind",
                        &[("kind", &super::conn::category_display(k))],
                    ),
                )
                .on_press(|a: &mut App| a.wallets.list.kind = None),
            );
        }
        let clear = t("connections:filters.clearAll");
        chips = chips.child(
            row()
                .key("wallets-chip-clear")
                .role("button")
                .label(clear.clone())
                .focusable(true)
                .on_press(|a: &mut App| {
                    a.wallets.list.status = None;
                    a.wallets.list.kind = None;
                })
                .child(text(clear).font(small).color(tk.destructive)),
        );
        out = out.child(chips);
    }
    out
}

type Pick = Rc<dyn Fn(&mut App)>;

/// The Filters `DropdownMenu` (`align="end" w-[220px]`): a label, a separator, and the Status
/// (`w-[180px]`) / Kind (`w-[200px]`) radio submenus.
fn filters_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let st = &app.wallets.list;
    let open_sub = match st.popup {
        Some(Popup::FiltersSub(s)) => Some(s),
        _ => None,
    };
    let row_h = tk.r(0.375) * 2.0 + tk.r(1.25);
    let pad = tk.r(0.25) + 1.0;
    let width = 220.0;
    let inner = width - 2.0 * pad;
    let base = |key: String, label: &str| {
        row()
            .key(key)
            .w_full()
            .h(row_h)
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .label(label.to_owned())
            .focusable(true)
    };
    let radio = |key: String, label: String, checked: bool, on: Pick| {
        let mut r = base(key, &label)
            .pl(tk.r(2.0))
            .pr(tk.r(0.5))
            .role("menuitemradio")
            .checked(checked)
            .on_press(move |a: &mut App| {
                on(a);
                close_popup(a);
            });
        if checked {
            r = r.child(
                row()
                    .absolute(tk.r(0.5), (row_h - tk.r(0.875)) / 2.0)
                    .size(tk.r(0.875), tk.r(0.875))
                    .center()
                    .child(crate::ui::dot(tk.r(0.5), tk.fg)),
            );
        }
        r.child(text(label).font(tk.tsm(400.0)).color(tk.fg).truncate())
    };
    let sub_trigger = |name: &'static str, label: String, dot: bool| {
        let open = open_sub == Some(name);
        let mut r = base(format!("wallets-filter-sub-{name}"), &label)
            .px(tk.r(0.5))
            .role("menuitem")
            .fill(if open { tk.accent } else { Color::TRANSPARENT })
            .child(text(label).font(tk.tsm(400.0)).color(tk.fg));
        if dot {
            r = r.child(crate::ui::dot(tk.r(0.375), tk.primary).ml(tk.r(0.25)));
        }
        // The sub content's first row sits level with this row (its padding and border above).
        let r = r
            .child(el().grow(1.0))
            .child(ic(&I::CHEVRON_RIGHT, tk.r(1.0), tk.fg))
            .child(popup::sub_anchor(
                &format!("wallets-filter-sub-{name}"),
                -pad,
            ));
        popup::sub_trigger(
            r,
            open,
            format!("wallets-filter-{name}-0"),
            move |a: &mut App| a.wallets.list.popup = Some(Popup::FiltersSub(name)),
        )
    };
    let sub_panel = |name: &'static str, w: f32, items: Vec<(El, String, String, bool)>| {
        let mut p = popup::Popup::sub(
            popup::Kind::Menu,
            "wallets-filters",
            format!("wallets-filter-sub-{name}"),
            close_popup,
            |a: &mut App| a.wallets.list.popup = Some(Popup::Filters),
        );
        for (el, key, label, checked) in items {
            p.item(el, key, label, true, checked);
        }
        // The menu hugs the window's right edge, so Radix flips the sub content to its left,
        // 3 pt over the menu's edge; its own layer, fitted to the window.
        let mut out = Vec::new();
        p.mount_sub(
            tk,
            &mut out,
            col().w(w - 2.0 * pad).label(name.to_owned()),
            |b| {
                menu_surface(tk, col().child(b))
                    .w(w)
                    .key(format!("wallets-filter-{name}-content"))
            },
            rune::Side::Left,
            pad - 3.0,
        );
        out
    };
    let mut p = popup::Popup::menu("wallets-filters", close_popup).list_key("wallets-filters-list");
    p.add(
        row().w_full().h(row_h).px(tk.r(0.5)).center_y().child(
            text(t("connections:filters.menu"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        ),
    );
    p.add(
        el().h(1.0)
            .mx(-tk.r(0.25))
            .my(tk.r(0.25))
            .w(inner + tk.r(0.5))
            .fill(tk.border),
    );
    p.item(
        sub_trigger(
            "status",
            t("connections:filters.status"),
            st.status.is_some(),
        ),
        "wallets-filter-sub-status",
        t("connections:filters.status"),
        true,
        false,
    );
    p.item(
        sub_trigger("kind", t("connections:filters.kind"), st.kind.is_some()),
        "wallets-filter-sub-kind",
        t("connections:filters.kind"),
        true,
        false,
    );
    let mut subs = vec![];
    if open_sub == Some("status") {
        let all = t("connections:filters.allStatuses");
        let mut items = vec![(
            radio(
                "wallets-filter-status-0".into(),
                all.clone(),
                st.status.is_none(),
                Rc::new(|a: &mut App| a.wallets.list.status = None),
            ),
            "wallets-filter-status-0".to_owned(),
            all,
            st.status.is_none(),
        )];
        for (i, s) in STATUSES.iter().enumerate() {
            let s: &'static str = s;
            let (k, l) = (
                format!("wallets-filter-status-{}", i + 1),
                t(&format!("chrome:connectionStatus.{s}")),
            );
            items.push((
                radio(
                    k.clone(),
                    l.clone(),
                    st.status == Some(s),
                    Rc::new(move |a: &mut App| a.wallets.list.status = Some(s)),
                ),
                k,
                l,
                st.status == Some(s),
            ));
        }
        subs.extend(sub_panel("status", 180.0, items));
    }
    if open_sub == Some("kind") {
        let all = t("connections:filters.allKinds");
        let mut items = vec![(
            radio(
                "wallets-filter-kind-0".into(),
                all.clone(),
                st.kind.is_none(),
                Rc::new(|a: &mut App| a.wallets.list.kind = None),
            ),
            "wallets-filter-kind-0".to_owned(),
            all,
            st.kind.is_none(),
        )];
        for (i, k) in kind_options().into_iter().enumerate() {
            let key = format!("wallets-filter-kind-{}", i + 1);
            let label = super::conn::category_display(k);
            items.push((
                radio(
                    key.clone(),
                    label.clone(),
                    st.kind.as_deref() == Some(k),
                    Rc::new(move |a: &mut App| a.wallets.list.kind = Some(k.to_owned())),
                ),
                key,
                label,
                st.kind.as_deref() == Some(k),
            ));
        }
        subs.extend(sub_panel("kind", 200.0, items));
    }
    let list = col().w_full().label(t("connections:filters.menu"));
    p.mount(
        tk,
        layer,
        list,
        |l| {
            menu_surface(tk, col().w(inner).min_h(0.0).child(l))
                .w(width)
                .key("wallets-filters-menu")
        },
        Placement::BottomEnd,
        4.0,
    );
    // An open submenu: its own layer above the menu, beside its trigger.
    layer.extend(subs);
}

/// `kb-glass-popover rounded-md border bg-popover p-1` with its drop shadow.
fn menu_surface(tk: &Tk, content: El) -> El {
    content
        .padding(tk.r(0.25) + 1.0)
        .radius(tk.r(0.375))
        .fill(tk.popover)
        .border(1.0, tk.fg.alpha(0.10))
        .box_shadow(Shadow {
            y: if tk.dark { 18.0 } else { 16.0 },
            blur: if tk.dark { 44.0 } else { 40.0 },
            spread: -18.0,
            color: Color::BLACK.alpha(if tk.dark { 0.8 } else { 0.55 }),
            ..Shadow::default()
        })
}

// ---- Table -----------------------------------------------------------------------------------

fn on_sort(a: &mut App, key: SortKey) {
    let cur = a.wallets.list.sort;
    a.wallets.list.sort = Some(match cur {
        Some((k, asc)) if k == key => (key, !asc),
        _ => (key, key.default_asc()),
    });
}

/// `SortableHead`: the label button (`text-xs sm:text-sm font-medium text-muted-foreground`)
/// with the sort glyph (shown when active).
fn sort_head(
    app: &App,
    tk: &Tk,
    label: &str,
    key: SortKey,
    right: bool,
    w: f32,
    px: (f32, f32),
) -> El {
    let sort = app.wallets.list.sort;
    let state = sort.filter(|(k, _)| *k == key).map(|(_, asc)| asc);
    // The table kit's sortable header label (the same glyphs and hover as every table).
    let b = crate::ui::table::sort_label(
        tk,
        format!("wallets-sort-{key:?}"),
        label,
        label.to_owned(),
        state,
        right,
    )
    .on_press(move |a: &mut App| on_sort(a, key));
    let cell = row()
        .w(w)
        .h(tk.r(crate::ui::table::HEAD_H) - 1.0)
        .pl(px.0)
        .pr(px.1)
        .center_y()
        .min_w(0.0)
        .child(b);
    let _ = app;
    if right { cell.justify_end() } else { cell }
}

/// Column widths of `table-fixed` with the web's percentage classes.
/// Below `lg` the Composition column (`hidden lg:table-cell`) is gone, so the others share its
/// width as well.
fn columns(tk: &Tk, table_w: f32, tax_col: bool, lg: bool) -> Vec<f32> {
    let mut pct: Vec<f32> = if tax_col {
        vec![43.0, 8.0, 7.0, 7.0, 17.0, 6.0, 12.0]
    } else {
        vec![45.0, 10.0, 8.0, 8.0, 17.0, 12.0]
    };
    if !lg {
        pct[4] = 0.0;
    }
    let sum: f32 = pct.iter().sum();
    // Percentages that don't add up to 100 % are scaled by the table layout to fill the width.
    let mut w: Vec<f32> = pct.iter().map(|p| table_w * p / sum).collect();
    // No header label is cut: each column gets at least its label, its sort glyph and the
    // table kit's padding, taken from the Connection column.
    let mut labels: Vec<(String, bool)> = vec![
        (t("connections:filters.kind"), true),
        (t("connections:table.columns.transactions"), true),
        (t("connections:table.columns.lastActivity"), true),
        (t("connections:table.columns.composition"), false),
    ];
    if tax_col {
        labels.push((t("connections:table.columns.taxFree"), true));
    }
    labels.push((t("connections:table.columns.balance"), true));
    let mut deficit = 0.0;
    for (i, (label, sortable)) in labels.iter().enumerate() {
        let k = i + 1;
        if w[k] <= 0.0 {
            continue;
        }
        let glyph = if *sortable {
            tk.r(0.875) + tk.r(0.25) + tk.r(0.75)
        } else {
            0.0
        };
        let need = rune::text::measure(label, &[], tk.tsm(500.0), None).0
            + glyph
            + 2.0 * tk.r(crate::ui::table::PX)
            + 1.0;
        if w[k] < need {
            deficit += need - w[k];
            w[k] = need;
        }
    }
    w[0] = (w[0] - deficit).max(260.0);
    w
}

fn table(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    shown: &[Conn],
    total: f64,
    total_count: usize,
) -> El {
    let tax_ids = tax_free_ids(app);
    let tax_col = tax_ids.is_some();
    let tax_ids = tax_ids.unwrap_or_default();
    let mut rows: Vec<Conn> = shown.to_vec();
    if let Some((key, asc)) = app.wallets.list.sort {
        rows.sort_by(|a, b| {
            let o = compare(a, b, key, &tax_ids);
            if asc { o } else { o.reverse() }
        });
    }
    let sm = win_w(app) >= 640.0;
    let lg = win_w(app) >= 1024.0;
    let pad_x = if sm { tk.card_pad() } else { tk.r(0.75) };
    // The table's width: the card less its horizontal padding, at least 1080.
    let card_w = cx
        .layout_size("wallets-table-wrap")
        .map(|[w, _]| w)
        .unwrap_or(1100.0);
    let table_w = (card_w - 2.0 * pad_x).max(1080.0);
    let w = columns(tk, table_w, tax_col, lg);
    let mut head = row()
        .w(table_w)
        .fill(tk.muted.alpha(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout();
    let mut i = 0;
    let mut next = || {
        let x = w[i];
        i += 1;
        x
    };
    head = head.child(sort_head(
        app,
        tk,
        &t("connections:table.columns.connection"),
        SortKey::Label,
        false,
        next(),
        (tk.r(0.5), tk.r(1.5)),
    ));
    head = head.child(sort_head(
        app,
        tk,
        &t("connections:filters.kind"),
        SortKey::Kind,
        false,
        next(),
        (tk.r(0.5), tk.r(0.5)),
    ));
    head = head.child(sort_head(
        app,
        tk,
        &t("connections:table.columns.transactions"),
        SortKey::Transactions,
        true,
        next(),
        (tk.r(0.5), tk.r(0.5)),
    ));
    head = head.child(sort_head(
        app,
        tk,
        &t("connections:table.columns.lastActivity"),
        SortKey::Last,
        true,
        next(),
        (tk.r(0.5), tk.r(0.5)),
    ));
    let comp_w = next();
    if lg {
        head = head.child(
            row()
                .w(comp_w)
                .h(tk.r(2.5) - 1.0)
                .px(tk.r(0.5))
                .center_y()
                .justify_end()
                .child(crate::ui::table::th_text(
                    tk,
                    t("connections:table.columns.composition"),
                )),
        );
    }
    if tax_col {
        head = head.child(sort_head(
            app,
            tk,
            &t("connections:table.columns.taxFree"),
            SortKey::TaxFree,
            true,
            next(),
            (tk.r(0.5), tk.r(0.5)),
        ));
    }
    head = head.child(sort_head(
        app,
        tk,
        &t("connections:table.columns.balance"),
        SortKey::Balance,
        true,
        next(),
        (tk.r(0.5), tk.r(0.5)),
    ));
    let mut body = col().w(table_w);
    if rows.is_empty() {
        let msg = t(if total_count == 0 {
            "connections:table.emptyNoWallets"
        } else {
            "connections:table.emptyFiltered"
        });
        body = body.child(crate::ui::table::empty_text(tk, &msg));
    }
    let overlap = snapshot(app)
        .and_then(|s| s.pointer("/balanceSummary/duplicateOutpointAdjustmentBtc"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        > 0.0;
    let n = rows.len();
    for (idx, c) in rows.iter().enumerate() {
        body = body.child(wallet_row(
            app,
            cx,
            tk,
            c,
            &w,
            lg,
            tax_col,
            tax_ids.contains(&c.id),
            total,
            overlap,
            idx + 1 == n,
        ));
    }
    let table = col().w(table_w).child(head).child(body);
    // `overflow-x-auto px-3 pt-3 sm:px-6` around the `min-w-[1080px]` table (in shadcn's own
    // `overflow-x-auto` box): narrower, the table scrolls sideways inside the padding.
    let table = crate::ui::table::hscroll(
        "wallets-table-scroll",
        t("connections:filters.heading"),
        table_w,
        table,
    );
    col()
        .key("wallets-table-wrap")
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(
            col()
                .w_full()
                .px(pad_x)
                .pt(tk.r(0.75))
                .pb(tk.r(if sm { 1.0 } else { 0.75 }))
                .child(table),
        )
}

#[allow(clippy::too_many_arguments)]
fn wallet_row(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    c: &Conn,
    w: &[f32],
    lg: bool,
    tax_col: bool,
    has_tax_free: bool,
    total: f64,
    overlap: bool,
    last: bool,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let hide_shares = app.prefs.hide_shares();
    let backend = c.is_backend();
    let fiat_mode = app.prefs.currency == Currency::Fiat;
    let pct = if total > 0.0 {
        c.balance / total * 100.0
    } else {
        0.0
    };
    let fiat_label = if rate_missing(app) {
        "—".to_owned()
    } else {
        fiat_value(app, c.balance)
    };
    let btc_label = format!("₿ {}", format_btc8(c.balance));
    let mut meta: Vec<String> = Vec::new();
    if backend {
        if let Some(e) = &c.endpoint {
            meta.push(e.clone());
        }
        if c.is_default_backend {
            meta.push(t("connections:table.meta.defaultBackend"));
        }
        meta.push(t("connections:table.meta.firstPartyInfra"));
    }
    if let Some(a) = c.addresses {
        meta.push(tc("connections:table.meta.addresses", a as usize, &[]));
    }
    if let Some(ch) = c.channels {
        meta.push(tc("connections:table.meta.channels", ch as usize, &[]));
    }
    if let Some(g) = c.gap {
        meta.push(tr(
            "connections:table.meta.gapLimit",
            &[("gap", &g.to_string())],
        ));
    }
    if c.deprecated {
        meta.push(t("connections:table.meta.deprecated"));
    }
    let comp_title = if backend {
        t("connections:table.share.firstParty")
    } else if hidden {
        t("connections:table.share.hidden")
    } else if overlap {
        t("connections:table.balanceOverlap")
    } else if pct < 0.1 {
        tr(
            "connections:table.share.below",
            &[("percent", &decimal(0.1, 1))],
        )
    } else {
        tr(
            "connections:table.share.ofTotal",
            &[("percent", &decimal(pct, if pct < 10.0 { 1 } else { 0 }))],
        )
    };
    let mut i = 0;
    let mut next = || {
        let x = w[i];
        i += 1;
        x
    };
    // Connection cell: `min-w-[260px] pr-6`, badge `mt-0.5`, name + metadata.
    let mut name_col = col()
        .min_w(0.0)
        .grow(1.0)
        .child(crate::ui::table::cell_text(
            c.label.clone(),
            tk.tsm(500.0),
            tk.fg,
        ));
    if !meta.is_empty() {
        name_col = name_col.child(
            crate::ui::table::cell_text(meta.join(" · "), tk.txs(400.0), tk.muted_fg)
                .mt(tk.r(0.25)),
        );
    }
    let name_row = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .items_start()
        .child(asset_badge(app, cx, tk, c, None, false).mt(tk.r(0.125)))
        .child(name_col);
    // `<td class="p-2 align-middle">`: the cell's own vertical-align beats the row's `align-top`.
    let cell = |w: f32, pl: f32, pr: f32| row().w(w).pl(pl).pr(pr).py(tk.r(0.5)).center_y();
    let mut r = row()
        .key(format!("wallets-row-{}", c.id))
        .w_full()
        .items_stretch()
        .hover(tk.muted.alpha(0.35))
        .role("button")
        .label(c.label.clone())
        .focusable(true);
    if !last {
        r = r
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    let id = c.id.clone();
    r = r.on_press(move |a: &mut App| a.navigate(&format!("/connections/{id}")));
    r = r.child(cell(next(), tk.r(0.5), tk.r(1.5)).child(name_row));
    // Kind: `Badge variant="outline" rounded-md`.
    r = r.child(
        cell(next(), tk.r(0.5), tk.r(0.5)).child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.125))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .center_y()
                .child(
                    text(super::conn::category_display(c.category_label()))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                ),
        ),
    );
    r = r.child(
        cell(next(), tk.r(0.5), tk.r(0.5))
            .justify_end()
            .child(if backend {
                crate::ui::table::dash(tk)
            } else {
                text(crate::format::count(
                    c.transaction_count.unwrap_or(0.0) as usize
                ))
                .font(tk.tsm(500.0).features("'tnum' 1"))
                .color(tk.fg)
            }),
    );
    r = r.child(
        cell(next(), tk.r(0.5), tk.r(0.5))
            .justify_end()
            .child(text(activity_label(c)).font(tk.tsm(400.0)).color(tk.fg)),
    );
    let comp_w = next();
    if lg {
        let content = if backend {
            text(
                c.sync_source
                    .clone()
                    .unwrap_or_else(|| t("connections:table.backendEndpoint")),
            )
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
        } else {
            let track_w = comp_w - 2.0 * tk.r(0.5);
            let mut bar = row()
                .w(track_w)
                .h(tk.r(0.5))
                .radius(tk.r(0.25))
                .fill(tk.muted)
                .clip_rounded(tk.r(0.25));
            if !hide_shares {
                bar = bar.child(
                    el().w(track_w * (pct.max(1.5) / 100.0).min(1.0) as f32)
                        .h_full()
                        .radius(tk.r(0.25))
                        .fill(tk.primary),
                );
            }
            bar
        };
        // The cell is `align-top`; the 0.5 rem bar sits at the top of its line box.
        r = r
            .child(cell(comp_w, tk.r(0.5), tk.r(0.5)).child(row().help(comp_title).child(content)));
    }
    if tax_col {
        r = r.child(
            cell(next(), tk.r(0.5), tk.r(0.5))
                .justify_end()
                .child(if backend {
                    text("—").font(tk.tsm(400.0)).color(tk.muted_fg)
                } else {
                    sensitive(
                        t(if has_tax_free {
                            "common:answer.yes"
                        } else {
                            "common:answer.no"
                        }),
                        tk.tsm(500.0),
                        if has_tax_free { tk.fg } else { tk.muted_fg },
                        hidden,
                    )
                }),
        );
    }
    let bal = if backend {
        col()
            .items_end()
            .child(
                text(t(if c.is_default_backend {
                    "connections:table.backendDefault"
                } else {
                    "connections:table.backendConfigured"
                }))
                .font(tk.tsm(500.0).features("'tnum' 1"))
                .color(tk.fg),
            )
            .child(
                text(if !c.wallet_refs.is_empty() {
                    tc("connections:table.wallets", c.wallet_refs.len(), &[])
                } else {
                    c.backend_kind.clone().unwrap_or_default()
                })
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg),
            )
    } else {
        let (top, bottom) = if fiat_mode {
            (fiat_label.clone(), btc_label.clone())
        } else {
            (btc_label.clone(), fiat_label.clone())
        };
        let key = c.id.clone();
        col()
            .items_end()
            .child(currency_toggle(
                app,
                tk,
                &format!("wallets-bal-{key}"),
                top,
                tk.tsm(500.0).features("'tnum' 1"),
                tk.fg,
            ))
            .child(currency_toggle(
                app,
                tk,
                &format!("wallets-bal2-{key}"),
                bottom,
                tk.txs(400.0).features("'tnum' 1"),
                tk.muted_fg,
            ))
    };
    r.child(cell(next(), tk.r(0.5), tk.r(0.5)).justify_end().child(bal))
}

/// `ScreenSkeleton titleWidth="w-32" metricCount={3}`.
pub fn skeleton(tk: &Tk) -> El {
    let bar = |w: f32, h: f32| {
        row()
            .w(w)
            .h(h)
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.7))
    };
    let mut metrics = row().w_full().gap(tk.card_gap());
    for _ in 0..3 {
        metrics = metrics.child(
            surface(tk)
                .grow(1.0)
                .padding(tk.r(1.0))
                .gap(tk.r(0.5))
                .child(bar(tk.r(5.0), tk.r(0.75)))
                .child(bar(tk.r(7.0), tk.r(1.5))),
        );
    }
    let mut list = surface(tk).w_full().padding(tk.r(1.0)).gap(tk.r(0.75));
    for _ in 0..8 {
        list = list.child(bar(tk.r(40.0), tk.r(1.0)));
    }
    col()
        .w_full()
        .gap(tk.section_gap())
        .busy(true)
        .child(bar(tk.r(8.0), tk.r(1.5)))
        .child(metrics)
        .child(list)
}

pub fn poll(_app: &mut App, _tx: &mut Tx) -> bool {
    false
}

pub fn on_reply(_app: &mut App, _tag: &str, _r: &Reply) {}

/// Escape closes the Filters menu (the submenu first, as Radix does).
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if app.route.path() != "/connections" {
        return false;
    }
    if matches!(ev.key, Key::Escape) {
        match app.wallets.list.popup {
            Some(Popup::FiltersSub(_)) => {
                app.wallets.list.popup = Some(Popup::Filters);
                return true;
            }
            Some(Popup::Filters) => {
                app.wallets.list.popup = None;
                return true;
            }
            None => {}
        }
    }
    false
}

pub fn prime(_app: &mut App) {}

/// SYNTHETIC: one source failed and one still syncing (the snapshot read, never the book), so
/// the stat row shows its signal-only tiles.
fn scene_attention(a: &mut App) {
    if let Some(r) = a.reads.get_mut("ui.overview.snapshot") {
        for (i, c) in r.data["connections"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .enumerate()
        {
            match i {
                1 => c["status"] = "error".into(),
                2 => c["status"] = "syncing".into(),
                _ => {}
            }
        }
    }
    super::scene_route(a, "/connections");
}

pub const SCENES: &[crate::app::Scene] = &[
    ("wallets-list", |a| super::scene_route(a, "/connections")),
    ("wallets-list-attention", scene_attention),
    ("wallets-list-attention-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_attention(a);
    }),
    ("wallets-list-hide-all", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAll;
        super::scene_route(a, "/connections");
    }),
    ("wallets-list-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        super::scene_route(a, "/connections");
    }),
    ("wallets-list-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        super::scene_route(a, "/connections");
    }),
    ("wallets-list-fiat", |a| {
        a.prefs.currency = Currency::Fiat;
        super::scene_route(a, "/connections");
    }),
    ("wallets-list-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        super::scene_route(a, "/connections");
    }),
    ("wallets-list-filters", |a| {
        super::scene_route(a, "/connections");
        a.wallets.list.popup = Some(Popup::Filters);
    }),
    ("wallets-list-filters-status", |a| {
        super::scene_route(a, "/connections");
        a.wallets.list.popup = Some(Popup::FiltersSub("status"));
    }),
    ("wallets-list-filters-kind", |a| {
        super::scene_route(a, "/connections");
        a.wallets.list.popup = Some(Popup::FiltersSub("kind"));
    }),
    ("wallets-list-filtered", |a| {
        super::scene_route(a, "/connections");
        a.wallets.list.kind = Some("Liquid".into());
    }),
    ("wallets-list-empty-filter", |a| {
        super::scene_route(a, "/connections");
        a.wallets.list.status = Some("error");
    }),
    ("wallets-list-sorted", |a| {
        super::scene_route(a, "/connections");
        a.wallets.list.sort = Some((SortKey::Balance, false));
    }),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(label: &str, kind: &str, chain: &str) -> Conn {
        Conn {
            label: label.into(),
            kind: kind.into(),
            chain: Some(chain.into()),
            id: label.into(),
            ..Default::default()
        }
    }

    #[test]
    fn kind_sort_groups_by_category_then_type_then_label() {
        let mut v = [
            conn("b", "descriptor", "liquid"),
            conn("Z", "address", "bitcoin"),
            conn("a", "descriptor", "bitcoin"),
        ];
        v.sort_by(|a, b| compare(a, b, SortKey::Kind, &[]));
        let labels: Vec<_> = v.iter().map(|c| c.label.as_str()).collect();
        // On-chain before Liquid; Address list before Wallet export.
        assert_eq!(labels, ["Z", "a", "b"]);
    }

    #[test]
    fn collator_is_numeric_and_case_insensitive() {
        assert_eq!(collate("Wallet 2", "wallet 10"), std::cmp::Ordering::Less);
        assert_eq!(collate("abc", "ABC"), std::cmp::Ordering::Equal);
    }

    /// Balances are valued in the book's currency at its market rate, not in euros.
    #[test]
    fn balances_value_in_the_book_currency() {
        let mut app = crate::app::tests::quiet_app();
        app.overview = crate::app::Load::Ready(
            crate::model::Snapshot::parse(&serde_json::json!({
                "priceEur": 80_000.0,
                "marketRate": {"fiatCurrency": "USD", "rate": 100_000.0},
                "fiat": {"fiatCurrency": "USD"},
            }))
            .unwrap(),
        );
        assert_eq!(fiat_value(&app, 0.5), "$50,000.00");
    }

    #[test]
    fn kind_options_end_with_liquid() {
        let k = kind_options();
        assert_eq!(k.first(), Some(&"On-chain"));
        assert_eq!(k.last(), Some(&"Liquid"));
    }
}
