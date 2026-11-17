//! The period nav (PeriodPicker + PeriodYearMenu + Export) and the workbench card: six metric
//! tiles and the flow chart (TransactionWorkbench.tsx).

use rune::{Color, Cx, El, Shadow, col, el, oklch, row, text};
use rune_kit::{Placement, popover};
use serde_json::Value;

use super::model::*;
use super::{Popup, changed};
use crate::app::{App, Currency};
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::overview::grid::stat::{Stat, row_of};
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive, surface};

// ---- Period nav ---------------------------------------------------------------------------

/// `chipClass`: `h-7 rounded-md px-3 text-sm font-medium` (the page is always past `@4xl`),
/// active `bg-card text-foreground shadow-sm`.
fn period_chip(tk: &Tk, key: String, label: String, active: bool, empty: bool) -> El {
    let mut c = row()
        .key(key)
        .h(tk.r(1.75))
        .px(tk.r(0.75))
        .center()
        .radius(tk.r(0.375))
        .role("button")
        .checked(active)
        .label(label.clone())
        .focusable(!empty)
        .child(
            text(label)
                .font(tk.tsm(500.0))
                .color(if active { tk.fg } else { tk.muted_fg }),
        );
    if active {
        c = c
            .fill(tk.card)
            .box_shadow(Shadow {
                y: 1.0,
                blur: 3.0,
                color: Color::BLACK.alpha(0.1),
                ..Shadow::default()
            })
            .box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                spread: -1.0,
                color: Color::BLACK.alpha(0.1),
                ..Shadow::default()
            });
    }
    if empty {
        // `aria-disabled` + `opacity-40`: still a named control, just not offered.
        c = c.opacity(0.4).help(t("transactions:period.emptyRange"));
    }
    c
}

/// The sticky row over the page: period chips and year menu left, Export (or, expanded, the
/// table's actions) right.
pub fn period_nav(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.txs;
    let resolved = ui.resolved();
    let latest = ui.dash.as_ref().and_then(|d| format::parse_ts(&d.latest));
    let hist = ui
        .dash
        .as_ref()
        .map(|d| history_years(&d.earliest, &d.latest))
        .unwrap_or(0.0);
    let mut ranges = selectable(hist);
    if let Period::Range(r) = resolved
        && !ranges.contains(&r)
    {
        ranges.push(r);
        ranges.sort_by_key(|x| Range::ALL.iter().position(|y| y == x));
    }
    // `flex h-8 items-center gap-1 rounded-lg bg-muted p-0.5`.
    let mut pill = row()
        .h(tk.r(2.0))
        .padding(tk.r(0.125))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .focus_group();
    for r in ranges {
        let active = resolved == Period::Range(r);
        let empty = !active
            && r != Range::All
            && latest.is_some_and(|l| {
                period_window(Period::Range(r), &[])
                    .and_then(|w| format::parse_ts(&w.since))
                    .is_some_and(|s| s > l)
            });
        let mut chip = period_chip(
            tk,
            format!("txs-period-{}", r.key()),
            r.label(),
            active,
            empty,
        );
        if !empty {
            chip = chip.on_press(move |a: &mut App| {
                a.txs.period = Period::Range(r);
                changed(a);
            });
        }
        pill = pill.child(chip);
    }
    let years = ui.years();
    if !years.is_empty() || matches!(ui.period, Period::Year(_)) {
        pill = pill.child(el().w(1.0).h(tk.r(1.0)).fill(tk.border));
        let year_active = matches!(resolved, Period::Year(_));
        let label = match resolved {
            Period::Year(y) => y.to_string(),
            _ => t("common:period.year"),
        };
        // PeriodYearMenu: the chip class plus `inline-flex gap-1` and a 0.875 rem chevron at 70 %.
        let mut trigger = row()
            .key("txs-year-trigger")
            .h(tk.r(1.75))
            .px(tk.r(0.75))
            .gap(tk.r(0.25))
            .center_y()
            .radius(tk.r(0.375))
            .role("button")
            .label(t("common:period.yearMenuAria"))
            .focusable(true)
            .menu_trigger(ui.popup == Some(Popup::Year), |a: &mut App| {
                a.txs.popup = if a.txs.popup == Some(Popup::Year) {
                    None
                } else {
                    Some(Popup::Year)
                }
            })
            .child(
                text(label)
                    .font(tk.tsm(500.0).features("'tnum' 1"))
                    .color(if year_active { tk.fg } else { tk.muted_fg }),
            )
            .child(
                ic(
                    &I::CHEVRON_DOWN,
                    tk.r(0.875),
                    if year_active { tk.fg } else { tk.muted_fg },
                )
                .opacity(0.7),
            );
        if year_active {
            trigger = trigger
                .fill(tk.card)
                .box_shadow(Shadow {
                    y: 1.0,
                    blur: 3.0,
                    color: Color::BLACK.alpha(0.1),
                    ..Shadow::default()
                })
                .box_shadow(Shadow {
                    y: 1.0,
                    blur: 2.0,
                    spread: -1.0,
                    color: Color::BLACK.alpha(0.1),
                    ..Shadow::default()
                });
        }
        let this_year = today().y;
        if ui.popup == Some(Popup::Year) {
            // PeriodYearMenu: `DropdownMenuContent align="start" max-h-72 min-w-44` of radio items.
            let entries = years
                .iter()
                .map(|y| {
                    let year = y.year;
                    let count = if year == this_year {
                        tc("common:period.yearCountSoFar", y.count, &[])
                    } else {
                        tc("common:period.yearCount", y.count, &[])
                    };
                    super::menu::Entry::Radio {
                        key: format!("txs-year-{year}"),
                        label: year.to_string(),
                        trailing: Some(count),
                        checked: resolved == Period::Year(year),
                        on: std::rc::Rc::new(move |a: &mut App| {
                            a.txs.period = Period::Year(year);
                            a.txs.popup = None;
                            changed(a);
                        }),
                    }
                })
                .collect();
            super::menu::mount(
                tk,
                layer,
                "txs-year-menu",
                "txs-year-trigger",
                Placement::BottomStart,
                tk.r(11.0),
                Some(tk.r(18.0)),
                None,
                entries,
            );
        }
        pill = pill.child(trigger);
    }
    let right = if ui.expanded {
        super::table::actions(app, tk, layer)
    } else {
        let busy = ui.export.busy;
        // Button variant="outline" size="sm": `h-8 gap-1.5 px-3`, `dark:bg-input/30 dark:border-input`.
        row()
            .key("txs-export")
            .h(tk.r(2.0))
            .px(tk.r(0.75) + 1.0)
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                tk.background
            })
            .border(1.0, if tk.dark { tk.input } else { tk.border })
            .hover(if tk.dark {
                tk.input.alpha(tk.input.a * 0.5)
            } else {
                tk.accent
            })
            .role("button")
            .label(t("transactions:dashboard.export.label"))
            .disabled(busy)
            .focusable(!busy)
            .opacity(if busy { 0.5 } else { 1.0 })
            .on_press(super::export::open)
            .child(ic(&I::DOWNLOAD, tk.r(1.0), tk.fg))
            .child(
                text(t("transactions:dashboard.export.label"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            )
    };
    row()
        .key("transactions-period-nav")
        .w_full()
        .pt(tk.page_pad())
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .fill(tk.background)
        .sticky_top(0.0)
        .stuck_shadow(Shadow {
            y: 6.0,
            blur: 12.0,
            spread: -6.0,
            color: Color::BLACK.alpha(if tk.dark { 0.5 } else { 0.12 }),
            ..Shadow::default()
        })
        .role("toolbar")
        .label(t("transactions:period.auto"))
        .child(pill)
        .child(right)
}

// ---- Money helpers ------------------------------------------------------------------------

/// `formatDisplayMoney(eur, |btc|, currency)`.
pub fn display_money(app: &App, eur: f64, btc: f64, cur: &str) -> String {
    match app.prefs.currency {
        Currency::Btc => format!("₿{:.8}", btc.abs()),
        Currency::Fiat => format::fiat(eur, cur),
    }
}

fn stat(v: &Value, k: &str) -> (f64, f64, f64) {
    let s = &v[k];
    (
        s["count"].as_f64().unwrap_or(0.0),
        s["btc"].as_f64().unwrap_or(0.0),
        s["eur"].as_f64().unwrap_or(0.0),
    )
}

/// The stats' `eur` sums are transaction values in the book's currency.
fn cur(app: &App) -> String {
    format::book_currency(app)
}

// ---- Tiles ----------------------------------------------------------------------------------

fn segment_selection(a: &mut App, seg: Segment) {
    a.txs.filters.quick = None;
    a.txs.filters.breakdown = None;
    a.txs.filters.reset_table();
    let label = a.txs.resolved();
    let label = match label {
        Period::Year(y) => y.to_string(),
        Period::Range(r) => r.label(),
        Period::Auto => Range::Y1.label(),
    };
    a.txs.filters.chart = Some(ChartSel {
        bucket_key: None,
        bucket_label: label,
        segment: Some(seg),
        external: false,
    });
    changed(a);
}

/// The period's buckets as a series for one segment (oldest first): bitcoin, or fiat in fiat
/// mode, the way the chart plots them.
fn trend(app: &App, d: &super::Dash, seg: Segment) -> Vec<f64> {
    let key = if app.prefs.currency == Currency::Fiat {
        "eur"
    } else {
        "btc"
    };
    d.buckets
        .iter()
        .map(|b| b["stats"][seg.param()][key].as_f64().unwrap_or(0.0).abs())
        .collect()
}

/// The metric tiles (`TransactionWorkbench`'s six) as the home screen's stat tiles (an owner
/// request), each with its trend over the period's buckets. The review queue tile shows only
/// while something waits (the Review badge keeps the count otherwise). Each keeps its click: a
/// segment of the chart, the external flow, the pairing candidates, the review queue.
fn tiles(app: &App, tk: &Tk, d: &super::Dash) -> Vec<Stat> {
    let s = &d.summary;
    let c = cur(app);
    let (ic_n, ic_b, ic_e) = stat(s, "incoming");
    let (og_n, og_b, og_e) = stat(s, "outgoing");
    let (tr_n, tr_b, tr_e) = stat(s, "transfers");
    let (_, sw_b, sw_e) = stat(s, "swaps");
    let (net_e, net_b) = (ic_e - og_e, ic_b - og_b);
    let review = s["review"].as_f64().unwrap_or(0.0) as usize;
    let pending = s["pending"].as_f64().unwrap_or(0.0) as usize;
    let failed = s["failed"].as_f64().unwrap_or(0.0) as usize;
    let unpaired = d.swap_candidate_total;
    let paired = s["markedSwapCount"].as_f64().unwrap_or(0.0) as usize;
    let sw_meta = match unpaired {
        None => t("transactions:workbench.meta.unpairedUnknown"),
        Some(u) if u > 0.0 && paired > 0 => tr(
            "transactions:workbench.meta.unpairedAndPaired",
            &[
                ("unpaired", &format::count(u as usize)),
                ("paired", &format::count(paired)),
            ],
        ),
        Some(u) if u > 0.0 => tc("transactions:workbench.meta.unpaired", u as usize, &[]),
        Some(_) => tc("transactions:workbench.meta.paired", paired, &[]),
    };
    let sw_alert = unpaired.is_none_or(|u| u > 0.0);
    let queue = review + pending + failed;
    let money = |e: f64, b: f64| display_money(app, e, b, &c);
    let good = oklch(0.56, 0.16, 150.0);
    let amber = tk.warn.solid;
    let sign_word = if net_e >= 0.0 {
        t("transactions:workbench.meta.inflow")
    } else {
        t("transactions:workbench.meta.outflow")
    };
    let fade = if app.txs.refreshing { 0.6 } else { 1.0 };
    let incoming = trend(app, d, Segment::Incoming);
    let outgoing = trend(app, d, Segment::Outgoing);
    let net_trend: Vec<f64> = incoming.iter().zip(&outgoing).map(|(i, o)| i - o).collect();
    let mut out = Vec::new();
    let mut t_in = Stat::new(
        "txs-tile-incoming",
        &I::ARROW_DOWN_RIGHT,
        t("transactions:workbench.metric.incoming"),
        money(ic_e, ic_b),
    )
    .sensitive(true)
    .line(tc(
        "transactions:workbench.meta.txCount",
        ic_n as usize,
        &[],
    ))
    .spark(incoming, good)
    .fade(fade);
    if ic_n > 0.0 {
        t_in = t_in.button(
            t("transactions:workbench.aria.showIncoming"),
            |a: &mut App| segment_selection(a, Segment::Incoming),
        );
    }
    out.push(t_in);
    let mut t_out = Stat::new(
        "txs-tile-outgoing",
        &I::ARROW_UP_RIGHT,
        t("transactions:workbench.metric.outgoing"),
        money(og_e, og_b),
    )
    .sensitive(true)
    .line(tc(
        "transactions:workbench.meta.txCount",
        og_n as usize,
        &[],
    ))
    .spark(outgoing, tk.kb_accent)
    .fade(fade);
    if og_n > 0.0 {
        t_out = t_out.button(
            t("transactions:workbench.aria.showOutgoing"),
            |a: &mut App| segment_selection(a, Segment::Outgoing),
        );
    }
    out.push(t_out);
    // `formatDisplayMoney(netEur, Math.abs(netBtc))`: BTC without a sign, fiat signed.
    let mut t_net = Stat::new(
        "txs-tile-net",
        &I::ARROW_LEFT_RIGHT,
        t("transactions:workbench.metric.netFlow"),
        money(net_e, net_b),
    )
    .sensitive(true)
    .line(sign_word)
    .spark(net_trend, if net_e >= 0.0 { good } else { tk.kb_accent })
    .fade(fade);
    if ic_n + og_n > 0.0 {
        t_net = t_net.button(
            t("transactions:workbench.aria.showNetFlow"),
            |a: &mut App| {
                a.txs.filters.chart = None;
                a.txs.filters.breakdown = None;
                a.txs.filters.reset_table();
                a.txs.filters.quick = Some(Quick::ExternalFlow);
                changed(a);
            },
        );
    }
    out.push(t_net);
    out.push(
        Stat::new(
            "txs-tile-transfers",
            &I::WALLET,
            t("transactions:workbench.metric.transfers"),
            money(tr_e, tr_b),
        )
        .sensitive(true)
        .line(tc(
            "transactions:workbench.meta.moveCount",
            tr_n as usize,
            &[],
        ))
        .spark(trend(app, d, Segment::Transfers), tk.muted_fg)
        .fade(fade)
        .button(
            t("transactions:workbench.aria.showTransfers"),
            |a: &mut App| segment_selection(a, Segment::Transfers),
        ),
    );
    let mut t_sw = Stat::new(
        "txs-tile-swaps",
        &I::REFRESH_CW,
        t("transactions:workbench.metric.swaps"),
        money(sw_e, sw_b),
    )
    .sensitive(true)
    .line_with(sw_meta, sw_alert.then_some(amber), false)
    .spark(trend(app, d, Segment::Swaps), oklch(0.62, 0.16, 246.0))
    .fade(fade);
    t_sw = if sw_alert {
        t_sw.glyph_ink(amber).link(
            t("transactions:workbench.aria.openPairingCandidates"),
            |a: &mut App| a.navigate("/transfers"),
        )
    } else {
        t_sw.button(
            t("transactions:workbench.aria.showPairedSwaps"),
            |a: &mut App| segment_selection(a, Segment::Swaps),
        )
    };
    out.push(t_sw);
    if queue > 0 {
        out.push(
            Stat::new(
                "txs-tile-review",
                &I::SHIELD_ALERT,
                t("transactions:workbench.metric.reviewQueue"),
                format::count(queue),
            )
            .ink(amber)
            .glyph_ink(amber)
            .line(tr(
                "transactions:workbench.meta.reviewAndPending",
                &[
                    ("review", &format::count(review)),
                    ("pending", &format::count(pending)),
                ],
            ))
            .fade(fade)
            .link(
                t("transactions:workbench.aria.showReviewQueue"),
                |a: &mut App| {
                    a.txs.filters.chart = None;
                    a.txs.filters.breakdown = None;
                    a.txs.filters.reset_table();
                    a.txs.filters.quick = Some(Quick::ReviewQueue);
                    changed(a);
                    a.navigate("/review");
                },
            ),
        );
    }
    out
}

// ---- Flow chart ---------------------------------------------------------------------------

fn seg_color(tk: &Tk, s: Segment) -> Color {
    match s {
        Segment::Incoming => oklch(0.56, 0.16, 150.0),
        Segment::Outgoing => tk.kb_accent,
        Segment::Transfers => oklch(0.56, 0.04, 260.0),
        Segment::Swaps => oklch(0.62, 0.16, 246.0),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONATE: [&str; 12] = [
    "Jän.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez.",
];

/// Chart bucket labels: month `MMM yy` (UI language), quarter `Q{n} {yy}`, day the key, year.
fn bucket_label(key: &str) -> String {
    if key.len() == 7 && !key.contains('Q') {
        let (y, m): (i32, usize) = (
            key[0..4].parse().unwrap_or(0),
            key[5..7].parse().unwrap_or(1),
        );
        let names = if crate::i18n::lang() == crate::i18n::Lang::De {
            MONATE
        } else {
            MONTHS
        };
        return format!("{} {:02}", names[m.clamp(1, 12) - 1], y.rem_euclid(100));
    }
    if let Some((y, q)) = key.split_once("-Q") {
        return format!("Q{q} {}", &y[y.len().saturating_sub(2)..]);
    }
    key.to_owned()
}

struct Bar {
    key: String,
    values: [(Segment, f64); 4],
    stats: Value,
}

/// Recharts' `getAdaptiveStep` (the default `niceTicks` algorithm).
fn adaptive_step(rough: f64) -> f64 {
    if rough <= 0.0 {
        return 0.0;
    }
    let digits = rough.abs().log10().floor() + 1.0;
    let unit = 10f64.powf(digits);
    let ratio = rough / unit;
    let scale = if digits != 1.0 { 0.05 } else { 0.1 };
    let amend = (ratio / scale - 1e-12).ceil() * scale;
    let step = amend * unit;
    // decimal.js-light keeps this exact; trim the binary noise.
    let mag = 10f64.powi(12 - step.abs().log10().ceil() as i32);
    (step * mag).round() / mag
}

/// Recharts' `getTickValuesFixedDomain([min, max], tickCount)`: ticks from `min` by the nice
/// step while below `max`, then `max` itself. A fixed numeric domain takes this path.
fn fixed_domain_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if min == max {
        return vec![min];
    }
    let step = adaptive_step((max - min) / (count.max(2) - 1) as f64);
    let mut out = Vec::new();
    if step > 0.0 {
        let mut i = 0;
        loop {
            let v = min + step * i as f64;
            let v = (v * 1e10).round() / 1e10;
            if v >= max || i > 1000 {
                break;
            }
            out.push(v);
            i += 1;
        }
    }
    out.push(max);
    out
}

/// Plot geometry of the Recharts BarChart: margins `{top 18, right 42, bottom 0, left 0}`,
/// a 56 pt Y axis, the default 30 pt X axis.
const PLOT_H: f32 = 280.0;
const PLOT_TOP: f32 = 18.0;
const PLOT_BOTTOM: f32 = PLOT_H - 30.0;
const PLOT_LEFT: f32 = 56.0;
const PLOT_RIGHT_MARGIN: f32 = 42.0;

fn chart(app: &App, cx: &mut Cx<App>, tk: &Tk, d: &super::Dash, layer: &mut Vec<El>) -> El {
    let ui = &app.txs;
    let fiat = app.prefs.currency == Currency::Fiat;
    let segs: Vec<Segment> = if ui.chart_external {
        vec![Segment::Incoming, Segment::Outgoing]
    } else {
        Segment::ALL.to_vec()
    };
    let mut bars: Vec<Bar> = d
        .buckets
        .iter()
        .map(|b| {
            let st = &b["stats"];
            let v = |s: Segment| {
                if !segs.contains(&s) {
                    return 0.0;
                }
                let x = &st[s.param()];
                let raw = if ui.chart_count {
                    x["count"].as_f64().unwrap_or(0.0)
                } else if fiat {
                    x["eur"].as_f64().unwrap_or(0.0)
                } else {
                    x["btc"].as_f64().unwrap_or(0.0)
                };
                if s == Segment::Outgoing {
                    -raw.abs()
                } else {
                    raw.abs()
                }
            };
            Bar {
                key: b["bucketKey"].as_str().unwrap_or("").to_owned(),
                values: [
                    (Segment::Incoming, v(Segment::Incoming)),
                    (Segment::Outgoing, v(Segment::Outgoing)),
                    (Segment::Transfers, v(Segment::Transfers)),
                    (Segment::Swaps, v(Segment::Swaps)),
                ],
                stats: st.clone(),
            }
        })
        .collect();
    let count_total: f64 = d
        .buckets
        .iter()
        .map(|b| {
            Segment::ALL
                .iter()
                .map(|s| b["stats"][s.param()]["count"].as_f64().unwrap_or(0.0))
                .sum::<f64>()
        })
        .sum();
    let active: Vec<Bar> = bars
        .drain(..)
        .filter(|b| b.values.iter().any(|(_, v)| *v != 0.0))
        .collect();
    let n_active = active.len();
    let (title_key, unit) = match d.bucket_size.as_str() {
        "day" => ("titleDay", "subtitleDay"),
        "quarter" => ("titleQuarter", "subtitleQuarter"),
        "year" => ("titleYear", "subtitleYear"),
        _ => ("titleMonth", "subtitleMonth"),
    };
    let title = t(&format!("transactions:workbench.chart.{title_key}"));
    let subtitle = tc(
        &format!("transactions:workbench.chart.{unit}"),
        count_total as usize,
        &[("buckets", &format::count(n_active))],
    );
    // `flowAxisDomain`: ±1.12 × the largest absolute value (count mode at least 1).
    let max_abs = active
        .iter()
        .flat_map(|b| b.values.iter().map(|(_, v)| v.abs()))
        .fold(if ui.chart_count { 1.0 } else { 0.0 }, f64::max);
    let domain = if max_abs == 0.0 { 1.0 } else { max_abs * 1.12 };
    let sel = ui.filters.chart.as_ref();
    let hide = app.prefs.hide_sensitive();
    let c = cur(app);
    let tick = |v: f64| -> String {
        if hide {
            return String::new();
        }
        if ui.chart_count {
            return format!("{}", v.round() as i64);
        }
        if fiat {
            format::fiat_compact(v, &c)
        } else {
            // formatBtc(v, {precision 4, sign: v !== 0}) with every space removed.
            format::btc(v, 4, v != 0.0).replace(' ', "")
        }
    };
    // Header: legend buttons over the metric/mode toggles, right.
    let mut legend = row()
        .gaps(tk.r(0.25), tk.r(0.5))
        .center_y()
        .flex_wrap()
        .justify_end();
    for s in &segs {
        let s = *s;
        let active_leg = sel.is_some_and(|x| x.bucket_key.is_none() && x.segment == Some(s));
        legend = legend.child(
            row()
                .key(format!("txs-legend-{}", s.param()))
                .gap(tk.r(0.375))
                .px(tk.r(0.375))
                .py(tk.r(0.125))
                .center_y()
                .radius(tk.r(0.375))
                .fill(if active_leg {
                    tk.muted
                } else {
                    Color::TRANSPARENT
                })
                .hover(tk.muted)
                .role("button")
                .label(s.label())
                .focusable(true)
                .on_press(move |a: &mut App| select_bucket(a, None, String::new(), Some(s)))
                .child(
                    el().size(tk.r(0.625), tk.r(0.625))
                        .radius(tk.r(0.25))
                        .fill(seg_color(tk, s)),
                )
                .child(text(s.label()).font(tk.txs(400.0)).color(if active_leg {
                    tk.fg
                } else {
                    tk.muted_fg
                })),
        );
    }
    // `h-7 rounded-md border px-2 text-xs font-medium`, pressed: `border-input bg-accent`.
    let toggle = |key: &str, label: String, on: bool, f: fn(&mut App)| {
        row()
            .key(key.to_owned())
            .h(tk.r(1.75))
            .px(tk.r(0.5) + 1.0)
            .center()
            .radius(tk.r(0.375))
            .fill(if on { tk.accent } else { tk.background })
            .border(1.0, if on { tk.input } else { tk.border })
            .hover(if on { tk.accent } else { tk.background })
            .role("button")
            .checked(on)
            .label(label.clone())
            .focusable(true)
            .on_press(f)
            .child(
                text(label)
                    .font(tk.txs(500.0))
                    .color(if on { tk.fg } else { tk.muted_fg }),
            )
    };
    let toggles = row()
        .gap(tk.r(0.25))
        .flex_wrap()
        .justify_end()
        .child(toggle(
            "txs-metric-amount",
            t("transactions:chartMetric.amount"),
            !ui.chart_count,
            |a| a.txs.chart_count = false,
        ))
        .child(toggle(
            "txs-metric-count",
            t("transactions:chartMetric.count"),
            ui.chart_count,
            |a| a.txs.chart_count = true,
        ))
        .child(toggle(
            "txs-mode-external",
            t("transactions:chartMode.external"),
            ui.chart_external,
            |a| a.txs.chart_external = true,
        ))
        .child(toggle(
            "txs-mode-all",
            t("transactions:chartMode.all"),
            !ui.chart_external,
            |a| a.txs.chart_external = false,
        ));

    // ---- The plot, positioned like the Recharts SVG ----
    let width = cx
        .layout_size("txs-flow-plot")
        .map(|[w, _]| w)
        .unwrap_or(1135.0);
    let (x0, x1) = (PLOT_LEFT, (width - PLOT_RIGHT_MARGIN).max(PLOT_LEFT + 1.0));
    let (min, max) = (-domain, domain);
    let y_of = |v: f64| PLOT_BOTTOM - ((v - min) / (max - min)) as f32 * (PLOT_BOTTOM - PLOT_TOP);
    let ticks = fixed_domain_ticks(min, max, 5);
    let tick_font = tk.px(10.0, 400.0);
    let tick_lh = tick_font.line_px();
    let mut plot = el()
        .key("txs-flow-plot")
        .w_full()
        .h(PLOT_H)
        .role("application");
    // CartesianGrid: horizontal lines at the ticks, `stroke-border/50`.
    for v in &ticks {
        let y = y_of(*v).round() - 0.5;
        plot = plot.child(
            el().absolute(x0, y)
                .w(x1 - x0)
                .h(1.0)
                .fill(tk.border.alpha(tk.border.a * 0.5))
                .inert(),
        );
    }
    // Y ticks: right-aligned 8 pt left of the plot (tick size 6 + margin 2), centred on the value.
    for v in &ticks {
        let y = y_of(*v);
        plot = plot.child(
            row()
                .absolute(0.0, y - tick_lh / 2.0)
                .w(x0 - 8.0)
                .justify_end()
                .a11y_hidden()
                .child(text(tick(*v)).font(tick_font).color(tk.muted_fg)),
        );
    }
    let n = active.len().max(1);
    let band = (x1 - x0) / n as f32;
    // getBarPositions: 10 % category gap each side, 4 pt between the four series, sizes floored.
    let gap_off = band * 0.1;
    let mut size = (band - 2.0 * gap_off - 3.0 * 4.0) / 4.0;
    let bar_gap = if size <= 0.0 { 0.0 } else { 4.0 };
    if size <= 0.0 {
        size = (band - 2.0 * gap_off) / 4.0;
    }
    if size > 1.0 {
        size = size.floor();
    }
    let y_zero = y_of(0.0);
    let hovered = ui.hover_bar.as_ref().map(|(k, _)| k.as_str());
    let mut plot_layer = el().absolute(0.0, 0.0).w_full().h(PLOT_H).a11y_hidden();
    for (j, b) in active.iter().enumerate() {
        let bx = x0 + band * j as f32;
        let key = b.key.clone();
        let label = bucket_label(&b.key);
        // Tooltip cursor over the hovered band: `fill-muted` at 10 %, primary stroke at 55 %.
        if hovered == Some(b.key.as_str()) {
            plot_layer = plot_layer.child(
                el().absolute(bx, PLOT_TOP)
                    .size(band, PLOT_BOTTOM - PLOT_TOP)
                    .fill(tk.muted.alpha(0.1))
                    .border(1.0, tk.primary.alpha(0.55))
                    .inert(),
            );
        }
        // FlowBucketClickAreas: the whole band opens the bucket.
        let year_bucket = b.key.len() == 4;
        let (k2, l2, hk, hk2) = (key.clone(), label.clone(), key.clone(), key.clone());
        plot_layer = plot_layer.child(
            el().key(format!("txs-bucket-{key}"))
                // Recharts' click areas are SVG paths: pointer-only, not tab stops or a11y nodes.
                .focusable(false)
                .a11y_hidden()
                .absolute(bx, PLOT_TOP)
                .size(band, PLOT_BOTTOM - PLOT_TOP)
                .cursor(rune::Cursor::Pointer)
                .on_hover_change(move |a: &mut App, on: bool, _tx: &mut rune::Tx| {
                    if on {
                        a.txs.hover_bar = Some((hk.clone(), Segment::Incoming));
                    } else if a.txs.hover_bar.as_ref().is_some_and(|(k, _)| *k == hk2) {
                        a.txs.hover_bar = None;
                    }
                })
                .on_press(move |a: &mut App| {
                    if year_bucket {
                        a.txs.filters.chart = None;
                        if let Ok(y) = k2.parse() {
                            a.txs.period = Period::Year(y);
                        }
                        changed(a);
                    } else {
                        select_bucket(a, Some(k2.clone()), l2.clone(), None);
                    }
                }),
        );
        let bucket_selected = sel.is_some_and(|x| x.bucket_key.as_deref() == Some(b.key.as_str()));
        for (i, (s, v)) in b.values.iter().enumerate() {
            if *v == 0.0 {
                continue;
            }
            let x = bx + gap_off + (size + bar_gap) * i as f32;
            let yv = y_of(*v);
            let (top, h) = if *v > 0.0 {
                (yv, y_zero - yv)
            } else {
                (y_zero, yv - y_zero)
            };
            plot_layer = plot_layer.child(segment(
                tk,
                &key,
                &label,
                *s,
                x,
                top,
                size,
                h.max(0.0),
                bucket_selected,
                sel,
            ));
            if ui.chart_count {
                // LabelList: top for positive bars, bottom for outgoing.
                let txt = format!("{}", v.abs().round() as i64);
                let y = if *v > 0.0 {
                    top - 5.0 - tick_lh
                } else {
                    top + h + 5.0
                };
                plot_layer = plot_layer.child(
                    row()
                        .absolute(x - 20.0, y)
                        .w(size + 40.0)
                        .justify_center()
                        .inert()
                        .child(
                            text(txt)
                                .font(tk.px(tk.r(0.75), 400.0))
                                .color(rune::color::hex("#808080").unwrap()),
                        ),
                );
            }
        }
    }
    // ReferenceLine y=0: foreground at 55 %, 2 pt, over the bars.
    plot_layer = plot_layer.child(
        el().absolute(x0, y_zero - 1.0)
            .w(x1 - x0)
            .h(2.0)
            .fill(tk.fg.alpha(0.55))
            .inert(),
    );
    plot = plot.child(plot_layer);
    // X ticks: centred on each band, the label's centre 19.6 pt under the plot (tick 6 + margin 2
    // + `dy` 8 + the 0.71 em baseline shift); Recharts' `preserveEnd` drops labels that collide.
    let est = |s: &str| s.chars().count() as f32 * 5.6;
    let mut keep = vec![false; active.len()];
    let mut last_left: Option<f32> = None;
    for j in (0..active.len()).rev() {
        let l = bucket_label(&active[j].key);
        let c = x0 + band * (j as f32 + 0.5);
        let w = est(&l);
        let ok = match last_left {
            None => true,
            Some(left) => c + w / 2.0 + 5.0 <= left,
        };
        if ok {
            keep[j] = true;
            last_left = Some(c - w / 2.0);
        }
    }
    for (j, b) in active.iter().enumerate() {
        if !keep[j] {
            continue;
        }
        let c = x0 + band * (j as f32 + 0.5);
        let w = band.max(60.0);
        plot = plot.child(
            row()
                .absolute(c - w / 2.0, PLOT_BOTTOM + 19.6 - tick_lh / 2.0)
                .w(w)
                .justify_center()
                .a11y_hidden()
                .child(
                    text(bucket_label(&b.key))
                        .font(tick_font)
                        .color(tk.muted_fg),
                ),
        );
    }
    if let Some((bk, _)) = &ui.hover_bar
        && let Some(b) = active.iter().find(|b| &b.key == bk)
    {
        layer.push(tooltip(app, tk, b, &c));
    }
    col()
        .key("txs-flow-chart")
        .w_full()
        .min_h(360.0)
        .padding(tk.r(1.0))
        .child(
            row()
                .w_full()
                .mb(tk.r(0.75))
                .justify_between()
                .items_start()
                .gap(tk.r(0.75))
                .child(
                    col()
                        .child(text(title).font(tk.tsm(600.0)).color(tk.fg))
                        .child(text(subtitle).font(tk.txs(400.0)).color(tk.muted_fg)),
                )
                .child(
                    col()
                        .items_end()
                        .gap(tk.r(0.5))
                        .child(legend)
                        .child(toggles),
                ),
        )
        .child(plot.opacity(if ui.refreshing { 0.6 } else { 1.0 }))
}

/// One Recharts bar: radius 2 on the end away from zero, dimmed to 32 % when another part is
/// selected, a 1.5 pt foreground stroke when selected.
#[allow(clippy::too_many_arguments)]
fn segment(
    tk: &Tk,
    bucket: &str,
    label: &str,
    s: Segment,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    bucket_selected: bool,
    sel: Option<&ChartSel>,
) -> El {
    let selected = sel.is_some_and(|x| {
        (x.bucket_key.is_none() || bucket_selected) && (x.segment.is_none() || x.segment == Some(s))
    });
    let dimmed = sel.is_some() && !selected;
    let color = seg_color(tk, s);
    let (bk, bl) = (bucket.to_owned(), label.to_owned());
    let mut e = el()
        .key(format!("txs-seg-{bucket}-{}", s.param()))
        .focusable(false)
        .a11y_hidden()
        .absolute(x, y)
        .size(w, h)
        .fill(if dimmed { color.alpha(0.32) } else { color })
        .corners(if s == Segment::Outgoing {
            [0.0, 0.0, 2.0, 2.0]
        } else {
            [2.0, 2.0, 0.0, 0.0]
        })
        .cursor(rune::Cursor::Pointer)
        .on_press(move |a: &mut App| select_bucket(a, Some(bk.clone()), bl.clone(), Some(s)));
    if selected {
        e = e.border(1.5, tk.fg);
    }
    e
}

fn select_bucket(a: &mut App, bucket: Option<String>, label: String, seg: Option<Segment>) {
    a.txs.filters.quick = None;
    a.txs.filters.breakdown = None;
    a.txs.filters.reset_table();
    let label = if bucket.is_none() {
        match a.txs.resolved() {
            Period::Year(y) => y.to_string(),
            Period::Range(r) => r.label(),
            Period::Auto => Range::Y1.label(),
        }
    } else {
        label
    };
    a.txs.filters.chart = Some(ChartSel {
        bucket_key: bucket,
        bucket_label: label,
        segment: seg,
        external: a.txs.chart_external,
    });
    a.txs.hover_bar = None;
    changed(a);
}

/// FlowTooltip: `min-w-[240px] rounded-lg border bg-popover p-3 text-xs shadow-lg`, rows sorted by
/// signed value, each with a count/total line, the largest transaction and warning chips.
fn tooltip(app: &App, tk: &Tk, b: &Bar, c: &str) -> El {
    let k = tk.kit();
    let hide = app.prefs.hide_sensitive();
    let fiat = app.prefs.currency == Currency::Fiat;
    let mut rows = col().gap(tk.r(0.5));
    let mut segs: Vec<(Segment, f64)> = b
        .values
        .iter()
        .copied()
        .filter(|(_, v)| *v != 0.0)
        .collect();
    segs.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (s, v) in segs {
        let st = &b.stats[s.param()];
        let n = st["count"].as_f64().unwrap_or(0.0) as usize;
        let value = if app.txs.chart_count {
            if v >= 0.0 {
                tc(
                    "transactions:workbench.tooltip.txDeltaPositive",
                    v.abs() as usize,
                    &[],
                )
            } else {
                tc(
                    "transactions:workbench.tooltip.txDeltaNegative",
                    v.abs() as usize,
                    &[],
                )
            }
        } else if fiat {
            format::signed_fiat(v, c)
        } else {
            format::btc(v, 8, true)
        };
        let money = |btc: f64, eur: f64| {
            if fiat {
                format::fiat(eur, c)
            } else {
                format::btc(btc, 8, false)
            }
        };
        let mut detail = col().gap(tk.r(0.25)).pl(tk.r(1.0)).child(
            row()
                .w_full()
                .justify_between()
                .gap(tk.r(0.75))
                .child(
                    text(tc("transactions:workbench.tooltip.txCount", n, &[]))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
                .child(sensitive(
                    money(
                        st["btc"].as_f64().unwrap_or(0.0),
                        st["eur"].as_f64().unwrap_or(0.0),
                    ),
                    tk.txs(400.0),
                    tk.muted_fg,
                    hide,
                )),
        );
        if let Some(l) = st["largest"]["label"].as_str() {
            let lb = &st["largest"];
            detail = detail.child(
                row()
                    .w_full()
                    .justify_between()
                    .gap(tk.r(0.75))
                    .child(
                        text(tr(
                            "transactions:workbench.tooltip.largest",
                            &[("label", l)],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate(),
                    )
                    .child(sensitive(
                        money(
                            lb["btc"].as_f64().unwrap_or(0.0),
                            lb["eur"].as_f64().unwrap_or(0.0),
                        ),
                        tk.txs(500.0),
                        tk.muted_fg,
                        hide,
                    )),
            );
        }
        let missing = st["missingPrice"].as_f64().unwrap_or(0.0) as usize;
        let review = st["review"].as_f64().unwrap_or(0.0) as usize;
        let failed = st["failed"].as_f64().unwrap_or(0.0) as usize;
        if missing + review + failed > 0 {
            let chip = |label: String, fg: Color| {
                row()
                    .px(tk.r(0.375))
                    .py(tk.r(0.125))
                    .radius(tk.r(0.375))
                    .fill(fg.alpha(0.1))
                    .child(text(label).font(tk.txs(400.0)).color(fg))
            };
            let mut chips = row().gap(tk.r(0.25)).flex_wrap().pt(tk.r(0.125));
            if missing > 0 {
                chips = chips.child(chip(
                    tc("transactions:workbench.tooltip.missingPrice", missing, &[]),
                    rune::color::hex("#d97706").unwrap(),
                ));
            }
            if review > 0 {
                chips = chips.child(chip(
                    tc("transactions:workbench.tooltip.review", review, &[]),
                    rune::color::hex("#2563eb").unwrap(),
                ));
            }
            if failed > 0 {
                chips = chips.child(chip(
                    tc("transactions:workbench.tooltip.failed", failed, &[]),
                    tk.kb_accent,
                ));
            }
            detail = detail.child(chips);
        }
        rows = rows.child(
            col()
                .gap(tk.r(0.375))
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(
                            el().size(tk.r(0.5), tk.r(0.5))
                                .radius(tk.r(0.25))
                                .fill(seg_color(tk, s)),
                        )
                        .child(text(s.label()).font(tk.txs(400.0)).color(tk.muted_fg))
                        .child(el().grow(1.0))
                        .child(sensitive(value, tk.txs(500.0), tk.fg, hide)),
                )
                .child(detail),
        );
    }
    let body = col()
        .min_w(240.0)
        .padding(tk.r(0.75))
        .child(
            text(bucket_label(&b.key))
                .font(tk.txs(500.0))
                .color(tk.fg)
                .mb(tk.r(0.5)),
        )
        .child(rows);
    let surface = popover::raised(&k, body, tk.r(0.5))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .inert();
    popover::anchored(
        &k,
        surface,
        &format!("txs-bucket-{}", b.key),
        Placement::Right,
        32.0,
    )
}

/// The workbench: the metric tiles as their own row of tiles, then the flow chart's card.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.txs;
    let card = surface(tk)
        .key("txs-workbench")
        .w_full()
        .padding(1.0)
        .clip_rounded(tk.r(0.75));
    let Some(d) = &ui.dash else {
        // First load: the tiles' loading state, then eight bars.
        let names = [
            (
                &I::ARROW_DOWN_RIGHT,
                "transactions:workbench.metric.incoming",
            ),
            (&I::ARROW_UP_RIGHT, "transactions:workbench.metric.outgoing"),
            (
                &I::ARROW_LEFT_RIGHT,
                "transactions:workbench.metric.netFlow",
            ),
            (&I::WALLET, "transactions:workbench.metric.transfers"),
            (&I::REFRESH_CW, "transactions:workbench.metric.swaps"),
        ];
        let stats = names
            .iter()
            .enumerate()
            .map(|(i, (g, k))| Stat::loading(format!("txs-tile-loading-{i}"), g, t(k)))
            .collect();
        let mut bars = row()
            .w_full()
            .h(200.0)
            .gap(8.0)
            .items_end()
            .padding(tk.r(0.75));
        for h in [42.0, 64.0, 36.0, 78.0, 52.0, 88.0, 48.0, 70.0] {
            bars = bars.child(
                row()
                    .grow(1.0)
                    .h(200.0 * h / 100.0)
                    .radius(3.0)
                    .fill(tk.muted),
            );
        }
        return col()
            .w_full()
            .gap(tk.card_gap())
            .label(t("common:state.loading"))
            .child(row_of(
                app,
                cx,
                tk,
                "txs-tiles",
                t("transactions:period.auto"),
                stats,
            ))
            .child(card.child(bars));
    };
    let tiles = row_of(
        app,
        cx,
        tk,
        "txs-tiles",
        t("transactions:period.auto"),
        tiles(app, tk, d),
    );
    col()
        .w_full()
        .gap(tk.card_gap())
        .child(tiles)
        .child(card.child(chart(app, cx, tk, d, layer)))
}
