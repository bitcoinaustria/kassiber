//! `OverviewWidgets.tsx`: the Tax year and Money in and out widgets' contents for the slot they
//! are given (their tile is `grid::frame`): this year in tax terms and the last twelve months of
//! money in and out. Both mask their amounts while amounts are hidden;
//! "Hide all" also drops the monthly bars, whose heights tell how money moved.

use rune::{Color, El, col, el, row, text};

use super::holdings::now_secs;
use super::holdings_model::{self as hm, MonthFlow};
use super::stats::mask;
use crate::app::{App, Currency};
use crate::format;
use crate::i18n::{Lang, lang, t, tc, tr};
use crate::model::Snapshot;
use crate::overview::grid::frame::{self, Slot};
use crate::theme::Tk;
use crate::ui::sensitive;

fn note(tk: &Tk, s: String) -> El {
    text(s).font(tk.t2xs(400.0)).color(tk.warn.text).wrap()
}

/// "Tax year 2026": the daemon counts "this year" from 1 January UTC; the label names the same year.
pub fn tax_title(app: &App) -> String {
    tr(
        "overview:widgets.tax.title",
        &[("year", &hm::utc_year(now_secs(app)).to_string())],
    )
}

/// `TaxYearWidget`: realized gains so far this year as the figure (Small), beside the unrealized
/// gains with the cost basis and the Austrian tax-free split underneath (Medium). Every figure
/// is the daemon's; with an incomplete basis it shows no gains and says why.
pub fn tax_year(app: &App, tk: &Tk, s: &Snapshot, x: &hm::Extras, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    let cur = s.fiat_currency();
    let comp = &s.fiat.completeness;
    let basis_known = comp.cost_basis_complete;
    let f = &s.fiat;
    let tone = |v: f64| {
        if v >= 0.0 {
            tk.good.solid
        } else {
            tk.alert.solid
        }
    };
    let block = |key: &str, label: String, value: Option<f64>| {
        let (txt, color) = match value {
            Some(v) => (mask(format::signed_fiat(v, &cur), hide), tone(v)),
            None => ("—".to_owned(), tk.muted_fg),
        };
        col()
            .key(slot.key(key))
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .gap(tk.r(0.125))
            .child(
                text(label)
                    .font(frame::label(tk))
                    .color(tk.muted_fg)
                    .truncate(),
            )
            .child(
                sensitive(txt, frame::figure(tk, 1.75), color, hide && value.is_some())
                    .truncate()
                    .min_w(0.0),
            )
    };
    let mut figures = row().w_full().min_w(0.0).gap(tk.r(1.0)).child(block(
        "tax-realized",
        t("overview:widgets.tax.realized"),
        basis_known.then_some(f.eur_realized_ytd),
    ));
    if slot.wide() {
        figures = figures.child(block(
            "tax-unrealized",
            t("overview:widgets.tax.unrealized"),
            (basis_known && !comp.market_rate_missing).then_some(f.eur_unrealized),
        ));
    }
    // As on Wallets: an inventory waiting on journals or blocked by quarantines is not shown.
    let current = x.tax_free.as_ref().map(|tf| tf.status() == "current");
    let mut foot: Vec<El> = Vec::new();
    if slot.wide() {
        let mut stats: Vec<(String, String)> = vec![(
            t("overview:widgets.tax.costBasis"),
            if basis_known {
                format::fiat(f.eur_cost_basis, &cur)
            } else {
                "—".into()
            },
        )];
        if let Some(tf) = &x.tax_free {
            let btc = |sats: f64| {
                if current == Some(true) {
                    format::btc(sats / 100_000_000.0, 8, false)
                } else {
                    "—".into()
                }
            };
            stats.push((t("overview:widgets.tax.taxFree"), btc(tf.tax_free_sats)));
            stats.push((t("overview:widgets.tax.taxable"), btc(tf.taxable_sats)));
        }
        let mut line = row().w_full().min_w(0.0).gap(tk.r(1.0)).role("list");
        for (i, (label, value)) in stats.into_iter().enumerate() {
            let dash = value == "—";
            let shown = if dash { value } else { mask(value, hide) };
            line = line.child(
                col()
                    .key(slot.key(format!("tax-row-{i}")))
                    .grow(1.0)
                    .w(0.0)
                    .min_w(0.0)
                    .child(
                        text(label)
                            .font(tk.t2xs(500.0))
                            .color(tk.muted_fg)
                            .truncate(),
                    )
                    .child(
                        sensitive(
                            shown,
                            tk.txs(500.0).features("'tnum' 1"),
                            tk.fg,
                            hide && !dash,
                        )
                        .truncate()
                        .min_w(0.0),
                    ),
            );
        }
        foot.push(line);
    }
    if !slot.wide() && basis_known && !comp.market_rate_missing {
        foot.push(
            col()
                .key(slot.key("tax-unrealized"))
                .w_full()
                .min_w(0.0)
                .child(
                    text(t("overview:widgets.tax.unrealized"))
                        .font(tk.t2xs(500.0))
                        .color(tk.muted_fg)
                        .truncate(),
                )
                .child(
                    sensitive(
                        mask(format::signed_fiat(f.eur_unrealized, &cur), hide),
                        tk.tsm(600.0).features("'tnum' 1"),
                        tone(f.eur_unrealized),
                        hide,
                    )
                    .truncate()
                    .min_w(0.0),
                ),
        );
    }
    if !basis_known {
        foot.push(note(tk, t("overview:widgets.tax.basisIncomplete")));
    } else if let (Some(tf), Some(false), true) = (&x.tax_free, current, slot.wide()) {
        foot.push(note(
            tk,
            if tf.status() == "quarantines" {
                tc(
                    "overview:widgets.tax.taxFreeQuarantines",
                    tf.quarantines as usize,
                    &[],
                )
            } else {
                t("overview:widgets.tax.taxFreeNeedsJournals")
            },
        ));
    }
    // Compact: the cost basis and the split sit right under the figures, not at the tile's foot.
    col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .clip_rounded(0.0)
        .gap(tk.r(0.75))
        .child(figures)
        .child(col().w_full().gap(tk.r(0.375)).children(foot))
}

/// The content height Tax year needs at `size`: the figures, then the cost basis line or the
/// unrealized figure, and a two-line note where one shows.
pub fn tax_natural(app: &App, tk: &Tk, s: &Snapshot, size: super::grid::model::Size) -> f32 {
    let x = hm::Extras::parse(app.read("ui.overview.snapshot"));
    let comp = &s.fiat.completeness;
    let figures = frame::label(tk).size * 1.5 + tk.r(0.125) + tk.r(1.75) * 1.15;
    let small_line = tk.t2xs(500.0).size * 1.5;
    let wide = size != super::grid::model::Size::Small;
    let mut foot = if wide {
        small_line + tk.txs(500.0).size * 1.5
    } else if comp.cost_basis_complete && !comp.market_rate_missing {
        small_line + tk.tsm(600.0).size * 1.5
    } else {
        0.0
    };
    let current = x.tax_free.as_ref().map(|tf| tf.status() == "current");
    let note = !comp.cost_basis_complete || (wide && current == Some(false));
    if note {
        foot += tk.r(0.375) + 2.0 * tk.t2xs(400.0).size * 1.5;
    }
    figures + tk.r(0.75) + foot
}

const MONTHS_EN: [&str; 12] = [
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
];
const MONTHS_DE: [&str; 12] = [
    "Januar",
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
];

/// `toLocaleDateString(lang, {month: "long", year: "numeric"})`.
fn month_title(m: &MonthFlow) -> String {
    let names = if lang() == Lang::De {
        MONTHS_DE
    } else {
        MONTHS_EN
    };
    format!("{} {}", names[m.month as usize - 1], m.year)
}

/// `{month: "narrow"}`: the month's initial (the same in English and German).
fn month_narrow(m: &MonthFlow) -> String {
    MONTHS_EN[m.month as usize - 1]
        .chars()
        .next()
        .map(String::from)
        .unwrap_or_default()
}

/// `FlowsWidget`: received above the line, spent below, the bars filling the slot; the totals
/// under them (Medium) or as figures above them, with the fees (Large). The web leaves the card
/// out in a year without any; here it is a widget the user chose to show, so it says so (the
/// shared empty state) rather than drawing a row of zeros.
pub fn flows(app: &App, tk: &Tk, s: &Snapshot, slot: &Slot) -> El {
    let hide = app.prefs.hide_sensitive();
    // "Hide all": the bars' heights tell how money moved, so they go too.
    let hide_bars = app.prefs.hide_shares();
    let cur = s.fiat_currency();
    let flows = hm::monthly_flows(s, hm::local_month(now_secs(app)));
    if !flows.has_activity {
        return frame::empty(
            tk,
            slot.key("flows-empty"),
            crate::overview::grid::copy::flows_empty(),
        );
    }
    let large = slot.tall();
    // Fiat totals only when every flow is valued in the book's currency; otherwise bitcoin.
    let btc_mode = app.prefs.currency == Currency::Btc || !flows.fiat_complete;
    let peak = flows
        .months
        .iter()
        .map(|m| m.in_btc.max(m.out_btc))
        .fold(0.0, f64::max);
    let peak = if peak > 0.0 { peak } else { 1.0 };
    let amount = |btc: f64, value: f64| {
        mask(
            if btc_mode {
                format::btc(btc, 4, false)
            } else {
                format::fiat_whole(value, &cur)
            },
            hide,
        )
    };
    let gap = tk.r(0.75);
    let incomplete = app.prefs.currency != Currency::Btc && !flows.fiat_complete;
    let totals_h = if large {
        tk.r(0.5 + 1.0 + 1.5 * 1.15)
    } else {
        tk.r(2.125)
    };
    let note_h = if incomplete && large {
        tk.r(1.0) + gap
    } else {
        0.0
    };
    let chart_h = (slot.h - totals_h - gap - note_h).max(tk.r(3.0));
    let label_h = tk.r(0.125) + tk.r(1.0);
    let half = ((chart_h - label_h - 1.0) / 2.0).max(0.0);
    let bar_h = |btc: f64| {
        if btc > 0.0 {
            ((btc / peak) as f32 * half).max(2.0)
        } else {
            0.0
        }
    };
    let radius = tk.r(0.3125);
    let mut bars = row()
        .key(slot.key("flows-chart"))
        .w_full()
        .h(chart_h)
        .gap(tk.r(if large { 0.5 } else { 0.375 }))
        .role("img")
        .label(t("overview:widgets.flows.chartAria"));
    for (i, m) in flows.months.iter().enumerate() {
        let title = if hide {
            month_title(m)
        } else {
            format!(
                "{}: {} {} · {} {}",
                month_title(m),
                t("overview:widgets.flows.received"),
                format::btc(m.in_btc, 4, false),
                t("overview:widgets.flows.spent"),
                format::btc(m.out_btc, 4, false)
            )
        };
        let mut top = col().w_full().h(half).justify_end();
        let mut bottom = col().w_full().h(half);
        if !hide_bars {
            if m.in_btc > 0.0 {
                top = top.child(
                    el().w_full()
                        .h(bar_h(m.in_btc))
                        .fill(hex_c("#10b981").alpha(0.8))
                        .corners([radius, radius, 0.0, 0.0]),
                );
            }
            if m.out_btc > 0.0 {
                bottom = bottom.child(
                    el().w_full()
                        .h(bar_h(m.out_btc))
                        .fill(hex_c("#ef4444").alpha(0.7))
                        .corners([0.0, 0.0, radius, radius]),
                );
            }
        }
        let column = col()
            .key(slot.key(format!("flows-month-{i}")))
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .help(title)
            .child(top)
            .child(el().w_full().h(1.0).fill(tk.fg.alpha(0.12)))
            .child(bottom)
            .child(
                row().w_full().pt(tk.r(0.125)).justify_center().child(
                    text(month_narrow(m))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg),
                ),
            );
        bars = bars.child(column);
    }
    let bars = if hide { bars.blur(3.5) } else { bars };
    let value_style = if large {
        frame::figure(tk, 1.5)
    } else {
        tk.tsm(600.0).features("'tnum' 1")
    };
    let total = |key: &str, value: String, color: Color, style: rune::TextStyle| {
        col()
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .gap(if large { tk.r(0.125) } else { 0.0 })
            .child(
                text(t(key))
                    .font(if large {
                        frame::label(tk)
                    } else {
                        tk.t2xs(500.0)
                    })
                    .color(tk.muted_fg)
                    .truncate(),
            )
            .child(sensitive(value, style, color, hide).truncate().min_w(0.0))
    };
    let totals = row()
        .w_full()
        .h(totals_h)
        .min_w(0.0)
        .gap(tk.r(0.75))
        .child(total(
            "overview:widgets.flows.received",
            amount(flows.in_btc, flows.in_value),
            tk.good.solid,
            value_style,
        ))
        .child(total(
            "overview:widgets.flows.spent",
            amount(flows.out_btc, flows.out_value),
            tk.alert.solid,
            value_style,
        ))
        .child(
            total(
                "overview:widgets.flows.fees",
                mask(format::btc(flows.fee_btc, 5, false), hide),
                tk.fg,
                if large {
                    frame::figure(tk, 1.0)
                } else {
                    value_style
                },
            )
            .grow(if large { 0.6 } else { 1.0 }),
        );
    let mut out = col()
        .w_full()
        .h_full()
        .min_h(0.0)
        .clip_rounded(0.0)
        .gap(gap);
    out = if large {
        out.child(totals).child(bars)
    } else {
        out.child(bars).child(totals)
    };
    if incomplete && large {
        out = out.child(
            text(tr(
                "overview:widgets.flows.fiatIncomplete",
                &[("currency", &cur)],
            ))
            .font(tk.t2xs(400.0))
            .color(tk.muted_fg)
            .truncate(),
        );
    }
    out
}

fn hex_c(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::TRANSPARENT)
}
