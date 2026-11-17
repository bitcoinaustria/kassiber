//! `WalletBalanceHistory` + `useWalletBalanceHistory`: the monthly booked balance as the
//! Overview chart draws it (step line, soft gradient, BTC or month-end market value), with the
//! 1Y / 3Y / All range chips, the change since the range start and a hover tooltip.

use rune::{Color, El, LineCap, Shape, canvas, col, el, row, text};
use serde_json::Value;

use super::{Range, btc, f, s};
use crate::app::{App, Currency};
use crate::i18n::{t, tr};
use crate::theme::Tk;
use crate::ui::sensitive;
use crate::wallets::conn::Conn;

/// `BalancePoint`: a `YYYY-MM` month, its BTC quantity and its value (None = unpriced).
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub y: i32,
    pub m: u32,
    pub qty: f64,
    pub value: Option<f64>,
}

impl Point {
    fn key(&self) -> i32 {
        self.y * 12 + self.m as i32 - 1
    }
    fn from_key(k: i32) -> (i32, u32) {
        (k.div_euclid(12), (k.rem_euclid(12) + 1) as u32)
    }
}

/// `balancePointsFromRows`: one point per month from the first booked month to now's, BTC
/// and L-BTC summed; months without rows are zero; a truncated history drops its oldest month.
pub fn points_from(data: &Value, now: (i32, u32)) -> (Vec<Point>, bool) {
    let truncated = data
        .get("summary")
        .and_then(|s| s.get("truncated"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut by: std::collections::BTreeMap<i32, Point> = Default::default();
    for row in data
        .get("rows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let asset = s(row, "asset").unwrap_or("").to_uppercase();
        if !matches!(asset.as_str(), "BTC" | "LBTC" | "L-BTC") {
            continue;
        }
        let Some(period) = s(row, "period_start").or_else(|| s(row, "bucket")) else {
            continue;
        };
        let (Some(y), Some(m)) = (
            period.get(0..4).and_then(|x| x.parse::<i32>().ok()),
            period.get(5..7).and_then(|x| x.parse::<u32>().ok()),
        ) else {
            continue;
        };
        let qty = f(row, "quantity").unwrap_or(0.0);
        let mv = row
            .get("market_value")
            .filter(|v| !v.is_null())
            .and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            });
        let priced = mv.is_some_and(|v| v.is_finite() && (v != 0.0 || qty == 0.0));
        let p = Point {
            y,
            m,
            qty: 0.0,
            value: Some(0.0),
        };
        let k = p.key();
        let cur = by.entry(k).or_insert(p);
        cur.qty += qty;
        cur.value = match (cur.value, priced) {
            (Some(v), true) => Some(v + mv.unwrap_or(0.0)),
            _ => None,
        };
    }
    let mut booked: Vec<i32> = by.keys().copied().collect();
    if truncated && !booked.is_empty() {
        booked.remove(0);
    }
    if booked.is_empty() {
        return (vec![], !truncated);
    }
    let now_k = now.0 * 12 + now.1 as i32 - 1;
    let last = (*booked.last().unwrap()).max(now_k);
    let mut out = vec![];
    let mut k = booked[0];
    while k <= last && out.len() < 1200 {
        let (y, m) = Point::from_key(k);
        out.push(by.get(&k).cloned().unwrap_or(Point {
            y,
            m,
            qty: 0.0,
            value: Some(0.0),
        }));
        k += 1;
    }
    (out, !truncated)
}

fn now_month() -> (i32, u32) {
    let d =
        crate::format::Day::from_epoch_days((crate::format::now_secs() / 86_400.0).floor() as i64);
    (d.y, d.m)
}

pub fn points(app: &App) -> (Vec<Point>, bool) {
    match app.wallets.detail.history.ready() {
        Some(d) => points_from(d, now_month()),
        None => (vec![], true),
    }
}

/// `hasHistory`: still loading, or at least two months to draw.
pub fn has_history(app: &App) -> bool {
    app.wallets.detail.history.loading() || points(app).0.len() >= 2
}

/// The axis tick, as on the Overview chart: `Okt 21` (de `Okt. 21`).
fn tick(p: &Point) -> String {
    // `Intl` month-only short names are the standalone forms (de-AT `Okt`, `Jun`, no dot).
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const DE: [&str; 12] = [
        "Jän", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ];
    let names = if crate::i18n::lang() == crate::i18n::Lang::De {
        DE
    } else {
        EN
    };
    format!(
        "{} {:02}",
        names[(p.m.clamp(1, 12) - 1) as usize],
        p.y.rem_euclid(100)
    )
}

/// The width of a 10 pt axis label (the system UI font, from Helvetica's advance widths
/// scaled to SF's), standing in for recharts' DOM measurement.
fn label_width(s: &str) -> f32 {
    let units: f32 = s
        .chars()
        .map(|c| match c {
            '0'..='9' | 'a' | 'b' | 'd' | 'e' | 'g' | 'h' | 'n' | 'o' | 'p' | 'q' | 'u' | 'ä' => {
                556.0
            }
            ' ' | 't' | 'f' | 'r' => {
                if c == 'r' {
                    333.0
                } else {
                    278.0
                }
            }
            'i' | 'j' | 'l' => 222.0,
            'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' | 'J' => 500.0,
            'F' | 'T' => 611.0,
            'M' => 833.0,
            'A' | 'S' | 'E' | 'P' | 'K' => 667.0,
            'O' => 778.0,
            'N' | 'D' | 'H' | 'U' => 722.0,
            '.' => 278.0,
            _ => 600.0,
        })
        .sum();
    units / 1000.0 * 10.0 * 1.12
}

/// recharts' `getNiceTickValues([0, max], 5)`: (step, top).
pub fn nice_ticks(max: f64) -> (f64, f64) {
    if max <= 0.0 || max.is_nan() {
        return (0.25, 1.0);
    }
    let rough = max / 4.0;
    let digits = rough.abs().log10().floor() as i32 + 1;
    let ratio = rough / 10f64.powi(digits);
    let scale = if digits != 1 { 0.05 } else { 0.1 };
    let mut correction = 0.0;
    loop {
        let amend = ((ratio / scale).ceil() + correction) * scale;
        let step = amend * 10f64.powi(digits);
        let step = (step * 1e12).round() / 1e12;
        if step * 4.0 >= max - 1e-12 {
            return (step, step * 4.0);
        }
        correction += 1.0;
    }
}

/// `formatBtcAxis`: `₿20`, `₿5.00`, `₿0.020`.
fn btc_axis(v: f64) -> String {
    let p = if v.abs() >= 10.0 {
        0
    } else if v.abs() >= 1.0 {
        2
    } else {
        3
    };
    format!("₿{:.*}", p, v.abs())
}

pub fn view(app: &App, tk: &Tk, c: &Conn, chart_w: f32) -> El {
    let d = &app.wallets.detail;
    let hide = app.prefs.hide_sensitive();
    let (cur, _) = super::fiat(app);
    if d.history.loading() {
        return col()
            .w_full()
            .child(
                row().h(tk.r(1.25)).child(
                    text(t("connections:detail.balanceHistory.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                ),
            )
            .child(super::ui::skeleton(tk, None, tk.r(13.0)).mt(tk.r(0.5)));
    }
    let (pts, complete) = points(app);
    let start = d
        .range
        .months()
        .map(|m| pts.len().saturating_sub(m))
        .unwrap_or(0);
    let range_pts = &pts[start..];
    let baseline: Option<Point> = if start > 0 {
        Some(pts[start - 1].clone())
    } else if !pts.is_empty() && complete {
        let (y, m) = Point::from_key(pts[0].key() - 1);
        Some(Point {
            y,
            m,
            qty: 0.0,
            value: Some(0.0),
        })
    } else {
        None
    };
    let mut visible: Vec<Point> = baseline.iter().cloned().collect();
    visible.extend(range_pts.iter().cloned());
    let show_fiat = app.prefs.currency == Currency::Fiat
        && !visible.is_empty()
        && visible.iter().all(|p| p.value.is_some());
    let val = |p: &Point| {
        if show_fiat {
            p.value.unwrap_or(0.0)
        } else {
            p.qty
        }
    };
    let fmt = |v: f64, sign: bool| -> String {
        if !show_fiat {
            return btc(v, 8, sign);
        }
        let s = crate::format::fiat(v.abs(), &cur);
        if sign {
            format!("{}{s}", if v < 0.0 { "− " } else { "+ " })
        } else {
            s
        }
    };
    let color = if show_fiat {
        tk.chart_value
    } else {
        tk.chart_btc
    };
    let first = range_pts.first();
    let latest = range_pts.last();
    let from = baseline.as_ref().or(first);
    let change = match (from, latest) {
        (Some(a), Some(b)) => val(b) - val(a),
        _ => 0.0,
    };

    // Header: title + as-of, the change line, the range chips.
    let mut title = row().gap(tk.r(0.5)).center_y().flex_wrap().child(
        text(t("connections:detail.balanceHistory.title"))
            .font(tk.tsm(500.0))
            .color(tk.fg),
    );
    if let Some(l) = latest {
        title = title.child(
            text(tr(
                "connections:detail.balanceHistory.asOf",
                &[("month", &super::hero::month_year_of(l.y, l.m))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    let mut head_left = col().min_w(0.0).child(title);
    if let Some(fp) = first {
        let ink = if change > 0.0 {
            super::ui::emerald_text(tk)
        } else if change < 0.0 {
            super::ui::red_text(tk)
        } else {
            tk.muted_fg
        };
        head_left = head_left.child(
            row()
                .mt(tk.r(0.25))
                .gap(tk.r(0.25))
                .center_y()
                .child(sensitive(
                    fmt(change, true),
                    tk.txs(500.0).features("'tnum' 1"),
                    ink,
                    hide,
                ))
                .child(
                    text(tr(
                        "connections:detail.balanceHistory.since",
                        &[("month", &super::hero::month_year_of(fp.y, fp.m))],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
        );
    }
    let options: Vec<Range> = Range::ALL
        .into_iter()
        .filter(|r| r.months().is_none_or(|m| pts.len() > m))
        .collect();
    let mut head = row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .justify_between()
        .child(head_left);
    if options.len() > 1 {
        let mut chips = row()
            .gap(tk.r(0.125))
            .center_y()
            .role("group")
            .label(t("connections:detail.balanceHistory.rangeLabel"));
        for o in options {
            let on = d.range == o;
            chips = chips.child(
                row()
                    .key(format!("wd-range-{}", o.key()))
                    .px(tk.r(0.5))
                    .py(tk.r(0.25))
                    .radius(tk.r(0.375))
                    .fill(if on { tk.muted } else { Color::TRANSPARENT })
                    .role("button")
                    .checked(on)
                    .label(t(&format!(
                        "connections:detail.balanceHistory.range.{}",
                        o.key()
                    )))
                    .focusable(true)
                    .on_press(move |a: &mut App| {
                        a.wallets.detail.range = o;
                        a.wallets.detail.hist_hover = None;
                    })
                    .child(
                        text(t(&format!(
                            "connections:detail.balanceHistory.range.{}",
                            o.key()
                        )))
                        .font(tk.txs(500.0))
                        .color(if on { tk.fg } else { tk.muted_fg }),
                    ),
            );
        }
        head = head.child(chips);
    }
    let _ = c;
    col()
        .w_full()
        .min_w(0.0)
        .child(head)
        .child(plot(app, tk, chart_w, &visible, show_fiat, color, hide, &cur).mt(tk.r(0.5)))
}

/// The `h-52` area chart (`margin: top 8, right 12, bottom 4, left 4`; Y axis 64 wide; X axis
/// 30 tall, ticks 10 px muted, `minTickGap 28`, `preserveEnd`).
#[allow(clippy::too_many_arguments)]
fn plot(
    app: &App,
    tk: &Tk,
    w: f32,
    pts: &[Point],
    fiat: bool,
    color: Color,
    hide: bool,
    cur: &str,
) -> El {
    let hgt = tk.r(13.0);
    // The chart is `w-full` inside `-mx-1`: its width is the card column's, known after layout.
    let w = w.max(200.0);
    let (left, right, top, bottom) = (4.0 + 64.0, w - 12.0, 8.0, hgt - 4.0 - 30.0);
    let vals: Vec<f64> = pts
        .iter()
        .map(|p| if fiat { p.value.unwrap_or(0.0) } else { p.qty })
        .collect();
    let max = vals.iter().cloned().fold(0.0, f64::max);
    let (step, top_v) = nice_ticks(max);
    let n = pts.len();
    let x_of = |i: usize| {
        if n <= 1 {
            left
        } else {
            left + (right - left) * i as f32 / (n - 1) as f32
        }
    };
    let y_of = |v: f64| bottom - ((v / top_v) as f32) * (bottom - top);
    let mut shape = Shape::new();
    let grid = tk.border.alpha(tk.border.a * 0.5 * 0.45);
    let mut labels: Vec<El> = vec![];
    for i in 0..=4 {
        let v = step * i as f64;
        let y = y_of(v);
        shape = shape.line([left, y], [right, y], 1.0, LineCap::Butt, grid);
        let label = if hide {
            String::new()
        } else if fiat {
            crate::format::fiat_compact(v, cur)
        } else {
            btc_axis(v)
        };
        labels.push(
            row()
                .absolute(0.0, y - 6.0)
                .w(left - 4.0)
                .h(12.0)
                .justify_end()
                .center_y()
                .inert()
                .child(text(label).font(tk.px(10.0, 400.0)).color(tk.muted_fg)),
        );
    }
    // Area: per step, a rectangle from its value down to zero, banded to the gradient (24 % at
    // the top of the series' box, 2 % at the baseline).
    if n >= 2 {
        let y_top = vals.iter().map(|v| y_of(*v)).fold(bottom, f32::min);
        let bands = 24;
        for b in 0..bands {
            let ya = y_top + (bottom - y_top) * b as f32 / bands as f32;
            let yb = y_top + (bottom - y_top) * (b + 1) as f32 / bands as f32;
            let tmid = (b as f32 + 0.5) / bands as f32;
            let alpha = 0.24 + (0.02 - 0.24) * tmid;
            let mut i = 0;
            while i + 1 < n {
                let yv = y_of(vals[i]);
                if yv >= yb {
                    i += 1;
                    continue;
                }
                let y0 = yv.max(ya);
                let start = i;
                i += 1;
                if yv <= ya {
                    while i + 1 < n && y_of(vals[i]) <= ya {
                        i += 1;
                    }
                }
                let (x0, x1) = (x_of(start), x_of(i));
                shape = shape.fill(
                    vec![[x0, y0], [x1, y0], [x1, yb], [x0, yb]],
                    color.alpha(alpha),
                );
            }
        }
        // The step line.
        let mut line = Vec::with_capacity(n * 2);
        for (i, v) in vals.iter().enumerate() {
            let (x, y) = (x_of(i), y_of(*v));
            if i > 0 {
                line.push([x, y_of(vals[i - 1])]);
            }
            line.push([x, y]);
        }
        shape = shape.stroke_with(line, false, 1.75, LineCap::Round, color);
    }
    // X ticks: recharts' `getTicksEnd` (`interval="preserveEnd"`) over the chart's box: from
    // the last point back, a tick shows when its label fits before the previous one less
    // `minTickGap`; the last one is pulled inside the box.
    let mut shown: Vec<usize> = vec![];
    let mut end = w;
    for i in (0..n).rev() {
        let label_w = label_width(&tick(&pts[i]));
        let mut x = x_of(i);
        if i == n - 1 && x + label_w / 2.0 > end {
            x = end - label_w / 2.0;
        }
        if x - label_w / 2.0 >= 0.0 && x + label_w / 2.0 <= end {
            shown.push(i);
            end = x - label_w / 2.0 - 28.0;
        }
    }
    for i in shown {
        let label = tick(&pts[i]);
        let lw = label_width(&label) + 8.0;
        let mut x = x_of(i) - lw / 2.0;
        if i == n - 1 {
            x = x.min(w + 4.0 - lw);
        }
        labels.push(
            row()
                .absolute(x, bottom + 6.0 + 4.0)
                .w(lw)
                .justify_center()
                .inert()
                .child(text(label).font(tk.px(10.0, 400.0)).color(tk.muted_fg)),
        );
    }
    let mut stack = el()
        .key("wd-history-plot")
        .w(w)
        .ml(-tk.r(0.25))
        .h(hgt)
        .child(canvas(shape).absolute(0.0, 0.0).size(w, hgt))
        .children(labels);
    // Hover: one column per point; the tooltip shows the month, its BTC and its value.
    let hover = app.wallets.detail.hist_hover.filter(|i| *i < n);
    if n >= 2 {
        let col_w = (right - left) / (n - 1) as f32;
        for i in 0..n {
            let x = x_of(i) - col_w / 2.0;
            stack = stack.child(
                el().absolute(x.max(left - col_w / 2.0), top)
                    .size(col_w, bottom - top)
                    .on_hover_change(move |a: &mut App, h, _tx: &mut rune::Tx| {
                        if h {
                            a.wallets.detail.hist_hover = Some(i);
                        } else if a.wallets.detail.hist_hover == Some(i) {
                            a.wallets.detail.hist_hover = None;
                        }
                    }),
            );
        }
    }
    if let Some(i) = hover {
        let x = x_of(i);
        let y = y_of(vals[i]);
        let p = &pts[i];
        stack = stack.child(
            el().absolute(x, top)
                .size(1.0, bottom - top)
                .fill(tk.border)
                .inert(),
        );
        stack = stack.child(
            el().absolute(x - 3.0, y - 3.0)
                .size(6.0, 6.0)
                .radius(3.0)
                .fill(color)
                .inert(),
        );
        let mut tip = col()
            .px(tk.r(0.625))
            .py(tk.r(0.375))
            .radius(tk.r(0.375))
            .fill(tk.popover)
            .border(1.0, tk.border)
            .border_layout()
            .box_shadow(rune::Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Default::default()
            })
            .inert()
            .child(
                text(super::hero::month_year_of(p.y, p.m))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
            .child(sensitive(
                btc(p.qty, 8, false),
                tk.txs(500.0).features("'tnum' 1"),
                tk.fg,
                hide,
            ));
        if let Some(v) = p.value {
            tip = tip.child(sensitive(
                crate::format::fiat(v, cur),
                tk.txs(400.0).features("'tnum' 1"),
                tk.muted_fg,
                hide,
            ));
        }
        let tx = if x + 10.0 + 150.0 > w {
            x - 10.0 - 150.0
        } else {
            x + 10.0
        };
        stack = stack.child(tip.absolute(tx, (y - 30.0).max(0.0)));
    }
    stack
}
