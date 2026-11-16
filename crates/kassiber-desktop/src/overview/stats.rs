//! Key figures (`StatsCards.tsx`'s three stats as a home-screen widget): the Bitcoin balance
//! alone (Small), with the BTC price (Medium; a click on the price refreshes the rate), and with
//! the portfolio value over a balance sparkline (Large).

use rune::{El, col, row, text};

use super::grid::frame::{self, Slot};
use super::grid::model::{Size, WidgetId};
use super::holdings_model as hm;
use serde_json::json;

use crate::app::{App, Reply, ToastTone};
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::model::{self, Snapshot};
use crate::theme::{MASK, Tk};
use crate::ui::sensitive;

/// The masked text the privacy modes show (`maskSensitive`).
pub(super) fn mask(s: String, hidden: bool) -> String {
    if hidden { MASK.to_owned() } else { s }
}

/// `refreshMarketRateState`: a network read of the latest BTC price, only from a click.
fn refresh_rate(app: &mut App) {
    if app.busy.rate_refresh || app.busy.syncing || app.busy.processing {
        return;
    }
    let pair = app
        .snapshot()
        .and_then(|s| s.market_rate.as_ref())
        .and_then(|m| m.pair.clone());
    let args = match &pair {
        Some(p) => json!({ "pair": p }),
        None => json!({}),
    };
    if app.send("ui.rates.latest", args, "ov.rates").is_some() {
        app.busy.rate_refresh = true;
        // OverviewDashboard.tsx hard-codes this progress text in English.
        app.busy.progress = Some(match pair {
            Some(p) => format!("Checking market rates · Fetching {p}."),
            None => "Checking market rates · Fetching BTC market rate.".into(),
        });
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "ov.rates" {
        return;
    }
    app.busy.rate_refresh = false;
    app.busy.progress = None;
    match &r.error {
        Some(e) => app.toast(
            ToastTone::Error,
            &t("chrome:notifications.feedback.btcPriceFailed"),
            Some(e.clone()),
            Some("market-rate-refresh"),
        ),
        None => {
            let samples: f64 = r.data["latest"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .map(|row| row["samples"].as_f64().unwrap_or(0.0))
                        .sum()
                })
                .unwrap_or(0.0);
            if samples > 0.0 {
                app.toast(
                    ToastTone::Success,
                    &t("chrome:notifications.feedback.btcPriceRefreshed"),
                    None,
                    Some("market-rate-refresh"),
                );
            } else {
                app.toast(
                    ToastTone::Info,
                    &t("chrome:notifications.feedback.btcPriceUnchanged"),
                    None,
                    Some("market-rate-refresh"),
                );
            }
        }
    }
    app.refresh_reads();
}

/// One figure: its label, the figure, a caption.
fn block(tk: &Tk, key: String, label: El, figure: El, caption: El) -> El {
    col()
        .key(key)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(label)
        .child(figure)
        .child(caption)
}

/// The content height Small and Medium need: one figure block and a short sparkline (the
/// sparkline takes whatever the row gives beyond that).
pub fn natural(tk: &Tk) -> f32 {
    let block = frame::label(tk).size * 1.5
        + tk.r(2.0) * 1.15
        + tk.txs(400.0).size * 1.5
        + 2.0 * tk.r(0.125);
    block + tk.r(0.5) + tk.r(2.5)
}

/// The widget's content for its slot.
pub fn body(app: &App, tk: &Tk, s: &Snapshot, x: &hm::Extras, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    let cur = s.fiat_currency();
    let refreshing = app.busy.syncing || app.busy.processing;
    let fade = if refreshing { 0.6 } else { 1.0 };
    let pv = slot.pv;
    let label = |s: String| {
        text(s)
            .font(frame::label(tk))
            .color(tk.muted_fg)
            .truncate()
            .min_w(0.0)
    };
    let caption_style = tk.txs(400.0);
    let big = slot.size == Size::Large;

    // Bitcoin balance: observed, carries no status; opens the wallets.
    let held = s.connections.iter().filter(|c| c.balance > 0.0).count();
    let balance = block(
        tk,
        slot.key("stat-balance"),
        label(t("overview:stats.bitcoinBalance")),
        sensitive(
            mask(format::btc(hm::display_balance_btc(s, x), 3, false), hide),
            frame::figure(tk, if big { 2.5 } else { 2.0 }),
            tk.fg,
            hide,
        )
        .truncate()
        .opacity(fade),
        text(tc("overview:stats.heldIn", held, &[]))
            .font(caption_style)
            .color(tk.muted_fg)
            .truncate(),
    )
    .role("link")
    .label(tr(
        "overview:stats.openStat",
        &[("title", &t("overview:stats.bitcoinBalance"))],
    ))
    .focusable(true)
    .focus_ring(2.0, 2.0, tk.ring);
    let balance = super::grid::press(balance, WidgetId::Stats, pv, |a| a.navigate("/connections"));

    // The balance's shape over the book's history. "Hide all" drops it: it tells how the
    // balance moved.
    let spark_h = |used: f32| (slot.h - used).max(0.0);
    let spark = |h: f32| {
        let ink = rune::color::hex("#f7931a").unwrap_or(tk.fg);
        let c = if app.prefs.hide_shares() {
            rune::el().size(slot.w, h)
        } else {
            frame::spark(&s.balance_series, slot.w, h, ink)
        };
        let c = if hide { c.blur(3.5) } else { c };
        col().w_full().grow(1.0).min_h(0.0).justify_end().child(c)
    };
    // label + figure + caption.
    let block_h = |rem: f32| {
        frame::label(tk).size * 1.5
            + tk.r(rem) * 1.15
            + caption_style.size * 1.5
            + 2.0 * tk.r(0.125)
    };

    let mut out = col()
        .key(slot.key("stats-tiles"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(tk.r(0.5));
    if refreshing {
        out = out
            .busy(true)
            .description(t("overview:stats.refreshingStats"));
    }
    if slot.size == Size::Small {
        return out
            .child(balance)
            .child(spark(spark_h(block_h(2.0) + tk.r(0.5))));
    }

    // BTC price: a button that fetches the latest rate (only from a click).
    let price = super::l10n::market_value(s, &cur);
    // The label already says what it prices; the unit joins it where there is room (Large).
    let price = if big {
        price
    } else {
        price
            .rsplit_once(" / ")
            .map(|(p, _)| p.to_owned())
            .unwrap_or(price)
    };
    let (caption, fresh) = super::l10n::market_caption(s, format::now_secs());
    let mut head = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .child(label(t("overview:stats.btcPrice")));
    if app.busy.rate_refresh {
        head = head.child(
            text(t("overview:stats.refreshing"))
                .font(tk.t2xs(500.0))
                .color(tk.primary),
        );
    }
    let disabled = refreshing || app.busy.rate_refresh;
    let small = if big { 1.5 } else { 2.0 };
    let mut price_el = block(
        tk,
        slot.key("stat-price"),
        head,
        text(price)
            .font(frame::figure(tk, small))
            .color(tk.fg)
            .truncate()
            .opacity(if app.busy.rate_refresh { 0.6 } else { 1.0 }),
        text(caption)
            .font(rune::TextStyle {
                weight: 500.0,
                ..caption_style
            })
            .color(if fresh { tk.good.solid } else { tk.muted_fg })
            .truncate(),
    )
    .role("button")
    .label(t("overview:stats.refreshBtcPrice"))
    .help(model::market_rate_detail(s))
    .focusable(true)
    .focus_ring(2.0, 2.0, tk.ring)
    .cursor(rune::Cursor::Pointer)
    .disabled(disabled);
    if !disabled && !pv {
        price_el = super::grid::press(price_el, WidgetId::Stats, pv, refresh_rate);
    }
    if slot.size == Size::Medium {
        return out
            .child(
                row()
                    .w_full()
                    .gap(tk.r(1.0))
                    .items_start()
                    .child(balance.grow(1.0).w(0.0))
                    .child(price_el.grow(1.0).w(0.0)),
            )
            .child(spark(spark_h(block_h(2.0) + tk.r(0.5))));
    }

    // Portfolio value: its status replaces the percent while the basis is in doubt.
    let stat = model::portfolio_stat(s);
    let c = &s.fiat.completeness;
    let unavailable = stat.issue.is_some() && c.market_rate_missing;
    let warning_href = stat
        .issue
        .as_ref()
        .and_then(|i| i.href)
        .or(match stat.status.as_deref() {
            Some("Needs journals") => Some("/journals"),
            Some(st) if st.ends_with("quarantine") || st.ends_with("quarantines") => {
                Some("/review")
            }
            _ => None,
        });
    let value_text = if unavailable {
        "—".to_owned()
    } else {
        mask(format::fiat_compact(stat.value, &cur), hide)
    };
    let value_color = if stat.issue.is_some() {
        tk.warn.solid
    } else {
        tk.fg
    };
    let status_text = stat
        .status
        .as_deref()
        .map(super::l10n::stat_status)
        .unwrap_or_else(|| {
            let pct = format!(
                "{}{:.1}%",
                if stat.positive { "+" } else { "-" },
                stat.change_percent.abs()
            );
            mask(pct, app.prefs.hide_shares())
        });
    let status_color = if warning_href.is_some() {
        tk.warn.solid
    } else if stat.positive {
        tk.good.solid
    } else {
        tk.alert.solid
    };
    let issue_text = super::l10n::completeness(s);
    let note = issue_text
        .as_ref()
        .map(|(copy, _)| copy.clone())
        .unwrap_or_else(|| super::l10n::comparison(stat.comparison));
    let href = warning_href.unwrap_or(stat.href);
    let value_style = frame::figure(tk, small);
    let value_fig = if unavailable {
        text(value_text)
            .font(value_style)
            .color(value_color)
            .opacity(fade)
    } else {
        sensitive(value_text, value_style, value_color, hide)
            .truncate()
            .opacity(fade)
    };
    let mut value_el = block(
        tk,
        slot.key("stat-value"),
        label(t("overview:stats.title.portfolioValue")),
        value_fig,
        row()
            .w_full()
            .min_w(0.0)
            .gap(tk.r(0.375))
            .center_y()
            .child(sensitive(
                status_text,
                rune::TextStyle {
                    weight: 500.0,
                    ..caption_style
                },
                status_color,
                hide && stat.status.is_none(),
            ))
            .child(
                if stat.issue.is_some() {
                    sensitive(note, caption_style, tk.muted_fg, hide)
                } else {
                    text(note).font(caption_style).color(tk.muted_fg)
                }
                .truncate()
                .min_w(0.0),
            ),
    )
    .role("link")
    .label(tr(
        "overview:stats.openStat",
        &[("title", &t("overview:stats.title.portfolioValue"))],
    ))
    .focusable(true)
    .focus_ring(2.0, 2.0, tk.ring);
    if let Some((_, hint)) = &issue_text {
        value_el = value_el.help(hint.clone());
    }
    let value_el = super::grid::press(value_el, WidgetId::Stats, pv, move |a| a.navigate(href));
    let used = block_h(2.5) + block_h(1.5) + 2.0 * tk.r(0.5) + tk.r(0.75) + 1.0;
    out.child(balance).child(spark(spark_h(used))).child(
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(crate::ui::hairline(tk.border.alpha(tk.border.a * 0.6)))
            .child(
                row()
                    .w_full()
                    .gap(tk.r(1.0))
                    .items_start()
                    .child(price_el.grow(1.0).w(0.0))
                    .child(value_el.grow(1.0).w(0.0)),
            ),
    )
}
