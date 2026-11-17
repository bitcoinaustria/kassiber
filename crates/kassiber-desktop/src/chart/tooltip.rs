//! Hover cards (TreasuryTooltip.tsx), the basis-incomplete region label
//! (BasisIncompleteRegion.tsx) and the basis-gap popover (BasisGapPopover.tsx).

use rune::{Color, El, Shadow, col, row, text};
use rune_kit::{Placement, popover};

use super::data::{AFlow, Pt};
use super::plot::Ctx;
use super::tip_place::{GAP, PillBand, place, rows_that_fit};
use super::{ChartPopup, View, flow_color};
use crate::app::App;
use crate::format::{self, Day};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::model::{Snapshot, TxStatus};
use crate::theme::Tk;
use crate::ui::{dot, ic, sensitive};

fn card(tk: &Tk, min_w: f32, max_w: f32, pad: f32) -> El {
    col()
        .min_w(min_w)
        .max_w(max_w)
        .padding(pad)
        .gap(6.0)
        .radius(tk.r(0.375))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .box_shadow(Shadow {
            y: 20.0,
            blur: 25.0,
            spread: -5.0,
            color: Color::BLACK.alpha(if tk.dark { 0.45 } else { 0.12 }),
            ..Shadow::default()
        })
        .box_shadow(Shadow {
            y: 8.0,
            blur: 10.0,
            spread: -6.0,
            color: Color::BLACK.alpha(if tk.dark { 0.35 } else { 0.1 }),
            ..Shadow::default()
        })
}

fn kv(tk: &Tk, label: &str, value: String, hidden: bool, tone: Option<Color>) -> El {
    row()
        .w_full()
        .gap(12.0)
        .justify_between()
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        )
        .child(sensitive(
            value,
            tk.txs(500.0).features("'tnum' 1"),
            tone.unwrap_or(tk.fg),
            hidden,
        ))
}

fn chip(tk: &Tk, label: String, mono: bool) -> El {
    let mut st = tk.t2xs(500.0);
    if mono {
        st = st.mono();
    }
    row()
        .px(6.0)
        .py(1.0)
        .radius(tk.r(0.25))
        .fill(tk.muted)
        .child(text(label).font(st).color(tk.muted_fg))
}

/// What the view measured for the hover card: its last laid-out size and event-list height,
/// the plot's window origin, and the window's size.
#[derive(Clone, Copy, Debug, Default)]
pub struct TipEnv {
    pub card: Option<[f32; 2]>,
    pub list_h: Option<f32>,
    pub origin: Option<[f32; 2]>,
    pub window: [f32; 2],
}

/// How the hover card is placed beside the pointer: built by the view, kept by the plot's
/// pointer handler so the card follows the pointer (and flips at the window's edges) between
/// view runs.
#[derive(Clone)]
pub struct TipPlan {
    size: [f32; 2],
    window: [f32; 2],
    /// The crosshair's date pill under the plot (top, bottom in plot points) and how far
    /// beside the pointer a card reaching it keeps; `None` without the crosshair.
    pill: Option<(f32, f32, f32)>,
    plot_y: (f32, f32),
    delta: rune::Live<[f32; 2]>,
}

impl TipPlan {
    /// The card's offset from the pointer at `local` (plot points), which is at `win` in the
    /// window.
    fn delta_at(&self, local: [f32; 2], win: [f32; 2]) -> [f32; 2] {
        let oy = win[1] - local[1];
        let shown = local[1] >= self.plot_y.0 && local[1] <= self.plot_y.1;
        let pill = self
            .pill
            .filter(|_| shown)
            .map(|(top, bottom, clear)| PillBand {
                top: top + oy,
                bottom: bottom + oy,
                clear,
            });
        let at = place(win, self.size, self.window, pill);
        [at[0] - win[0], at[1] - win[1]]
    }
    /// The pointer moved to `local` (plot points), `win` in the window: the card follows.
    pub fn follow(&self, local: [f32; 2], win: [f32; 2]) {
        self.delta.set(self.delta_at(local, win));
    }
}

/// The hover card: `anchor` goes in the plot and follows the pointer by its live offset;
/// `card` goes in the window layer, so no card or widget clips it, beside that anchor.
pub struct Tip {
    pub anchor: El,
    pub card: El,
    pub plan: TipPlan,
}

/// The tooltip for the hovered dot or column, positioned beside the live pointer.
pub fn tooltip(c: &Ctx, env: &TipEnv) -> Option<Tip> {
    let v = c.view as usize;
    let geo = c.geo;
    let ui = &c.app.chart;
    // No hover card under a drag, under an open menu, or on the card behind the full-window chart.
    if ui.gesture[v].is_some_and(|g| g.moved)
        || ui.popup.is_some()
        || (ui.expanded && c.view == View::Compact)
    {
        return None;
    }
    let dot = c.app.chart.hover_dot[v].and_then(|i| geo.markers.get(i));
    let col = c.app.chart.hover[v].and_then(|i| geo.plot.get(i));
    // Recharts' shared tooltip also collects the Scatter entry whose `date` is the hovered
    // category, and TreasuryTooltip prefers an event payload over the base point.
    let col_marker = col
        .filter(|p| !p.is_event())
        .and_then(|p| geo.markers.iter().find(|m| m.date == p.date));
    let event = dot.is_some() || col.is_some_and(|p| p.is_event()) || col_marker.is_some();
    let rows = |p: &Pt| event_rows(c, p, env);
    let content = match (dot, col, col_marker) {
        (Some(m), _, _) => event_card(c, m, rows(m)),
        (None, Some(p), _) if p.is_event() => event_card(c, p, rows(p)),
        (None, Some(_), Some(m)) => event_card(c, m, rows(m)),
        (None, Some(p), None) => base_card(c, p),
        _ => return None,
    };
    // Sizes at rest (the day card measures 262 x 124 pt) until the card has been laid out.
    let size = env
        .card
        .filter(|s| s[0] > 0.0 && s[1] > 0.0)
        .unwrap_or(if event {
            [300.0, 250.0]
        } else {
            [262.0, 124.0]
        });
    let window = if env.window[0] > 0.0 && env.window[1] > 0.0 {
        env.window
    } else {
        [1440.0, 900.0]
    };
    let f = c.frame;
    let plan = TipPlan {
        size,
        window,
        pill: pill_band(c),
        plot_y: (f.top, f.bottom),
        delta: ui.tip_delta[v].clone(),
    };
    let local = ui.tip_pos.get();
    let origin = env.origin.unwrap_or([0.0, 0.0]);
    plan.delta
        .set(plan.delta_at(local, [origin[0] + local[0], origin[1] + local[1]]));
    let at = format!("chart-tip-at-{v}");
    // A point that rides the pointer inside the plot (and the plot through any scroll).
    let anchor = rune::el()
        .key(at.clone())
        .absolute(0.0, 0.0)
        .size(1.0, 1.0)
        .live_offset(&ui.tip_pos)
        .inert()
        .instant_changes();
    // Recharts shows and swaps the card at once. A fade-in (or a cross-fade between the day
    // and event cards) leaves it translucent for 0.2 s, and what is under it shows through.
    let card = rune::el()
        .key(format!("chart-tip-{v}"))
        .size(0.0, 0.0)
        // On the pointer: the gap of -1 puts the wrapper's top at the anchor's, and a 0 x 0
        // wrapper never flips. The card hangs at the planned offset from it.
        .anchored(at, rune::Side::Bottom, rune::Align::Start, -1.0)
        .inert()
        .transition(rune::Transition::none())
        .motion_intent(rune::MotionIntent::CUT)
        .instant_changes()
        .child(
            content
                .key(format!("chart-tip-card-{v}"))
                .absolute(0.0, 0.0)
                .live_offset(&plan.delta),
        );
    Some(Tip { anchor, card, plan })
}

/// The crosshair's date pill under the plot, in plot points, and how far beside the pointer a
/// card reaching down to it hangs: far enough to clear it, or Recharts' 32 pt.
fn pill_band(c: &Ctx) -> Option<(f32, f32, f32)> {
    let ui = &c.app.chart;
    if !ui.crosshair {
        return None;
    }
    let f = c.frame;
    let v = c.view as usize;
    // The pill is centred on the hovered column, which the pointer can sit up to half a
    // column from without the view re-running: clear its half-width from either side.
    let i = ui.hover[v].unwrap_or(0);
    let label = c
        .geo
        .display
        .get(c.geo.display_of(i))
        .map(|p| {
            Day::parse(&p.date)
                .map(|d| d.long_ui())
                .unwrap_or_else(|| p.detail.clone())
        })
        .unwrap_or_default();
    let pill_w = 30f32.max((label.chars().count() as f32 * 5.8).round() + 10.0);
    let half_col = (f.right - f.left) / c.geo.plot.len().max(1) as f32 / 2.0;
    // The pill is 15 pt tall, centred 20.2 pt under the plot.
    Some((
        f.bottom + 12.7,
        f.bottom + 27.7,
        GAP.max(pill_w / 2.0 + half_col + 6.0),
    ))
}

/// How many of a merged dot's events the card lists: 8, fewer when the window is too short
/// for the card (its height without the list measured from the last layout).
fn event_rows(c: &Ctx, p: &Pt, env: &TipEnv) -> usize {
    let members = p.ev.as_ref().map_or(0, |e| e.members.len());
    let (Some([_, card_h]), Some(list_h)) = (env.card, env.list_h) else {
        return MAX_ROWS;
    };
    if env.window[1] <= 0.0 {
        return MAX_ROWS;
    }
    let tk = c.tk;
    // The list: 8 pt padding and a 1 pt border each side, its title, then 3 pt gaps between
    // rows of one text-2xs line.
    let line = tk.r(1.0);
    let title = tk.r(0.875);
    let list = |n: usize, more: bool| {
        let lines = n + more as usize;
        18.0 + title + lines as f32 * (line + 3.0)
    };
    rows_that_fit(MAX_ROWS, members, card_h - list_h, env.window[1], list)
}

/// The most events a merged dot's card lists.
const MAX_ROWS: usize = 8;

fn status_text(status: Option<TxStatus>, conf: Option<f64>) -> String {
    match status {
        Some(TxStatus::Confirmed) => tc(
            "overview:tooltip.confirmations",
            conf.unwrap_or(0.0) as usize,
            &[],
        ),
        Some(TxStatus::Pending) => t("overview:txStatus.pending"),
        Some(TxStatus::Review) => t("overview:txStatus.review"),
        Some(TxStatus::Failed) => t("overview:txStatus.failed"),
        None => t("overview:tooltip.unknown"),
    }
}

fn type_label(label: &str) -> String {
    crate::overview::transaction_type_label(label)
}

fn basis_hint() -> String {
    t("overview:completeness.hint.incomplete")
}

fn event_card(c: &Ctx, p: &Pt, max_rows: usize) -> El {
    let tk = c.tk;
    let hide = c.hide;
    let e = p.ev.as_ref().unwrap();
    let merged = e.count > 1;
    let good = tk.good.solid;
    let bad = tk.kb_accent;
    // Amount and tone.
    let (caption, amount, tone) = if merged {
        if e.signed != 0.0 {
            (
                Some(t("overview:tooltip.netAmount")),
                format::btc(e.signed, 8, true),
                if e.signed >= 0.0 { good } else { bad },
            )
        } else {
            (
                None,
                tr(
                    "overview:tooltip.volume",
                    &[("value", &format::btc(e.activity_btc, 8, false))],
                ),
                tk.fg,
            )
        }
    } else {
        match e.flow {
            AFlow::Movement => (
                None,
                tr(
                    "overview:tooltip.volume",
                    &[("value", &format::btc(e.activity_btc, 8, false))],
                ),
                tk.fg,
            ),
            AFlow::Fee => (
                None,
                format::btc(-(if e.fee > 0.0 { e.fee } else { e.activity_btc }), 8, true),
                bad,
            ),
            AFlow::Incoming => (None, format::btc(e.signed, 8, true), good),
            AFlow::Outgoing => (None, format::btc(e.signed, 8, true), bad),
        }
    };
    let mut head_left = row()
        .gap(6.0)
        .center_y()
        .child(dot(10.0, flow_color(tk, e.flow)))
        .child(text(e.flow.label()).font(tk.txs(600.0)).color(tk.fg));
    if merged {
        head_left = head_left.child(chip(
            tk,
            tc("overview:tooltip.mergedEvents", e.count, &[]),
            false,
        ));
    } else if let Some(kind) = &e.tx_type {
        head_left = head_left.child(chip(tk, type_label(kind), false));
    }
    let mut right = col().items_end();
    if let Some(cap) = caption {
        right = right.child(text(cap).font(tk.t3xs(500.0)).color(tk.muted_fg));
    }
    right = right.child(sensitive(
        amount,
        tk.tsm(600.0).features("'tnum' 1"),
        tone,
        hide,
    ));
    let mut card = card(tk, 280.0, 320.0, 12.0).child(
        row()
            .w_full()
            .gap(12.0)
            .justify_between()
            .items_start()
            .child(
                col().gap(2.0).child(head_left).child(
                    text(p.detail.clone())
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg),
                ),
            )
            .child(right),
    );
    if merged && e.members.len() > 1 {
        let mut members: Vec<&Pt> = e.members.iter().collect();
        members.sort_by(|a, b| {
            b.ev.as_ref()
                .unwrap()
                .size
                .total_cmp(&a.ev.as_ref().unwrap().size)
        });
        let mut list = col()
            .key(format!("chart-tip-list-{}", c.view as usize))
            .w_full()
            .gap(3.0)
            .padding(8.0)
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .child(
                text(t("overview:tooltip.groupedEvents").to_uppercase())
                    .font(tk.t3xs(600.0).tracking(0.4))
                    .color(tk.muted_fg),
            );
        let shown = max_rows.clamp(1, MAX_ROWS);
        for m in members.iter().take(shown) {
            let me = m.ev.as_ref().unwrap();
            let who = me
                .counter
                .clone()
                .or(me.account.clone())
                .unwrap_or_default();
            let label = if who.is_empty() {
                me.flow.label()
            } else {
                format!("{} · {}", me.flow.label(), who)
            };
            let amt = match me.flow {
                AFlow::Movement => format::btc(me.activity_btc, 6, false),
                _ => format::btc(me.signed, 6, true),
            };
            list = list.child(
                row()
                    .w_full()
                    .gap(6.0)
                    .center_y()
                    .child(dot(6.0, flow_color(tk, me.flow)))
                    .child(
                        text(label)
                            .font(tk.t2xs(400.0))
                            .color(tk.fg)
                            .truncate()
                            .grow(1.0)
                            .min_w(0.0),
                    )
                    .child(sensitive(
                        amt,
                        tk.t2xs(500.0).features("'tnum' 1"),
                        tk.fg,
                        hide,
                    )),
            );
        }
        if members.len() > shown {
            let more = members.len() - shown;
            list = list.child(
                text(tc("overview:tooltip.groupedMore", more, &[]))
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg),
            );
        }
        card = card.child(list);
    }
    let mut rows = col().w_full().gap(4.0);
    if let Some(a) = &e.account {
        rows = rows.child(kv(
            tk,
            &t("overview:tooltip.source"),
            a.clone(),
            false,
            None,
        ));
    }
    if let Some(cp) = &e.counter {
        rows = rows.child(kv(
            tk,
            &t("overview:tooltip.counterparty"),
            cp.clone(),
            false,
            None,
        ));
    }
    rows = rows.child(kv(
        tk,
        &t(if merged {
            "overview:tooltip.fiatVolume"
        } else {
            "overview:tooltip.fiatValue"
        }),
        format::fiat(e.fiat, c.cur),
        hide,
        None,
    ));
    rows = rows.child(kv(
        tk,
        &t("overview:tooltip.btcPrice"),
        format::fiat_price(p.price, c.cur),
        false,
        None,
    ));
    if e.fee > 0.0 {
        rows = rows.child(kv(
            tk,
            &t("overview:tooltip.fee"),
            format::btc(e.fee, 8, false),
            hide,
            None,
        ));
    }
    rows = rows.child(kv(
        tk,
        &t("overview:tooltip.positionAfter"),
        format::btc(p.balance, 8, false),
        hide,
        None,
    ));
    let avg = if p.basis_incomplete { None } else { p.avg_cost };
    rows = rows.child(kv(
        tk,
        &t("overview:tooltip.avgBasisAfter"),
        avg.map(|v| format::fiat_price(v, c.cur))
            .unwrap_or_else(|| "—".into()),
        hide,
        None,
    ));
    if e.status.is_some() || !merged {
        rows = rows.child(kv(
            tk,
            &t("overview:tooltip.status"),
            status_text(e.status, e.conf),
            false,
            None,
        ));
    }
    card = card.child(rows);
    if p.basis_incomplete {
        card = card.child(
            text(basis_hint())
                .font(tk.t2xs(400.0))
                .color(tk.warn.text)
                .wrap()
                .w(296.0),
        );
    }
    let mut chips = row().gap(6.0).flex_wrap();
    let mut any = false;
    if let Some(tag) = e.tag.clone().filter(|t| !t.is_empty()) {
        chips = chips.child(chip(tk, tag, false));
        any = true;
    }
    if let Some(id) = &e.id {
        chips = chips.child(chip(tk, format::compact_id(id), true));
        any = true;
    }
    if any {
        card = card.child(chips);
    }
    card
}

fn base_card(c: &Ctx, p: &Pt) -> El {
    let tk = c.tk;
    let hide = c.hide;
    let avg = if p.basis_incomplete { None } else { p.avg_cost };
    let unreal = if p.basis_incomplete {
        ("—".to_owned(), tk.fg)
    } else {
        let pct = if p.cost_basis != 0.0 {
            format!(
                " ({}{:.1}%)",
                if p.unrealized >= 0.0 { "+" } else { "\u{2212}" },
                (p.unrealized / p.cost_basis.abs() * 100.0).abs()
            )
        } else {
            String::new()
        };
        (
            format!("{}{}", format::signed_fiat(p.unrealized, c.cur), pct),
            if p.unrealized >= 0.0 {
                tk.good.solid
            } else {
                tk.kb_accent
            },
        )
    };
    let mut card = card(tk, 220.0, 300.0, 10.0)
        .child(text(p.detail.clone()).font(tk.txs(600.0)).color(tk.fg))
        .child(kv(
            tk,
            &t("overview:tooltip.btcBalance"),
            format::btc(p.balance, 8, false),
            hide,
            None,
        ))
        .child(kv(
            tk,
            &t("overview:tooltip.btcPrice"),
            format::fiat_price(p.price, c.cur),
            false,
            None,
        ))
        .child(kv(
            tk,
            &t("overview:tooltip.avgBasis"),
            avg.map(|v| format::fiat_price(v, c.cur))
                .unwrap_or_else(|| "—".into()),
            hide,
            None,
        ))
        .child(kv(
            tk,
            &t("overview:tooltip.unrealized"),
            unreal.0,
            hide,
            Some(unreal.1),
        ));
    if p.basis_incomplete {
        card = card.child(
            text(basis_hint())
                .font(tk.t2xs(400.0))
                .color(tk.warn.text)
                .wrap()
                .w(276.0),
        );
    }
    card
}

/// The amber pill in the 40 pt band above the plot, anchored right of the boundary (left of
/// it past the middle). Opens the basis-gap popover.
pub fn region_label(c: &Ctx, x: f32) -> El {
    let tk = c.tk;
    let from = c
        .app
        .overview
        .ready()
        .and_then(|s| s.fiat.completeness.earliest_incomplete_at.clone())
        .and_then(|d| Day::parse(&d));
    let label = match from {
        Some(d) if x > c.frame.left + 0.5 => tr(
            "overview:treasury.basisIncompleteFrom",
            &[("date", &d.long_ui())],
        ),
        _ => t("overview:treasury.basisIncomplete"),
    };
    let w = label.chars().count() as f32 * tk.t2xs(500.0).size * 0.56 + 34.0;
    let left_side = x > (c.frame.left + c.frame.right) / 2.0;
    let px = if left_side { x - w - 6.0 } else { x + 6.0 };
    let view = c.view;
    row()
        .key(format!("basis-label-{}-1", view as usize))
        .absolute(px.max(c.frame.left), c.frame.top - 34.0)
        .h(20.0)
        .px(8.0)
        .gap(4.0)
        .center_y()
        .radius(10.0)
        .fill(tk.warn.bg)
        .border(1.0, tk.warn.border)
        .role("button")
        .label(label.clone())
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |a: &mut App| a.chart.popup = Some(ChartPopup::BasisGap(view, 1)))
        .child(ic(&I::TRIANGLE_ALERT, tk.r(0.75), tk.warn.text))
        .child(text(label).font(tk.t2xs(500.0)).color(tk.warn.text))
}

/// `BasisGapPopover`: what blocks the cost basis, each with its fix.
pub fn basis_gap_popover(tk: &Tk, s: &Snapshot, anchor: &str, layer: &mut Vec<El>) {
    let k = tk.kit();
    let c = &s.fiat.completeness;
    let from = c.earliest_incomplete_at.as_deref().and_then(Day::parse);
    let body = match from {
        Some(d) => tr(
            "overview:treasury.basisGap.bodyFrom",
            &[("date", &d.long_ui())],
        ),
        None => t("overview:treasury.basisGap.body"),
    };
    let journals = format!("{} →", t("overview:treasury.basisGap.openJournals"));
    let review = format!("{} →", t("overview:treasury.basisGap.openQuarantine"));
    let mut causes: Vec<(String, String, &'static str)> = Vec::new();
    if c.has("journals_stale") {
        causes.push((
            t("overview:treasury.basisGap.stale"),
            journals.clone(),
            "/journals",
        ));
    }
    if c.has("quarantines") {
        causes.push((
            tc(
                "overview:treasury.basisGap.quarantines",
                c.quarantine_count as usize,
                &[],
            ),
            review.clone(),
            "/review",
        ));
    }
    if c.has("custody_unresolved") {
        causes.push((
            t("overview:treasury.basisGap.custody"),
            journals.clone(),
            "/journals",
        ));
    }
    if c.has("missing_prices") {
        causes.push((
            tc(
                "overview:treasury.basisGap.missingPrices",
                c.missing_price_count as usize,
                &[],
            ),
            review.clone(),
            "/review",
        ));
    }
    let mut list = col().w_full().radius(tk.r(0.375)).border(1.0, tk.border);
    for (i, (label, link, href)) in causes.into_iter().enumerate() {
        let mut r = row()
            .w_full()
            .px(10.0)
            .py(6.0)
            .gap(8.0)
            .justify_between()
            .center_y();
        if i > 0 {
            r = r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        }
        list = list.child(
            r.child(text(label).font(tk.txs(400.0)).color(tk.fg)).child(
                row()
                    .role("link")
                    .label(link.clone())
                    .focusable(true)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(move |a: &mut App| {
                        a.chart.popup = None;
                        a.chart.expanded = false;
                        a.navigate(href);
                    })
                    .child(text(link).font(tk.txs(500.0)).color(tk.fg)),
            ),
        );
    }
    let content = col()
        .key("basis-gap")
        .w(tk.r(22.0))
        .padding(12.0)
        .gap(8.0)
        .role("dialog")
        .label(t("overview:treasury.basisIncomplete"))
        .child(
            text(t("overview:treasury.basisIncomplete"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        )
        .child(
            text(body)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        )
        .child(list);
    let surface = popover::raised(&k, content, tk.r(0.375))
        .fill(tk.popover)
        .modal(true, |a: &mut App| a.chart.popup = None);
    layer.push(popover::anchored(
        &k,
        surface,
        anchor,
        Placement::BottomStart,
        6.0,
    ));
    let _ = View::Compact;
}
