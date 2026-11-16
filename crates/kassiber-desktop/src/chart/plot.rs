//! The plot: grid, the four lines, last-value tags, the basis-incomplete region, activity
//! dots (single, merged and mixed-flow pies), the hover crosshair and active dots.
//!
//! Lines and fills are one `canvas(Shape)`; dots, tags and labels are elements so they hit
//! test and lay out text. Hover moves only a `Live` offset while it stays in one column (and,
//! with the crosshair on, at one height). The gestures are in `interact`.

use std::rc::Rc;

use rune::{Color, El, LineCap, Shape, Tx, canvas, col, el, row, text};

use super::axes::{XAxis, pct_label};
use super::data::Pt;
use super::interact::GestureKind;
use super::{Geo, SeriesKey, View, Visible, flow_color, strings};
use crate::app::App;
use crate::format::{self, Day};
use crate::i18n::{tc, tr};
use crate::theme::Tk;

/// Plot box inside the plot element, in points.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub w: f32,
    pub h: f32,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Frame {
    pub fn new(w: f32, h: f32, expanded: bool, region: bool, brush: bool) -> Frame {
        let side = if expanded { 8.0 } else { 4.0 };
        let top = if region { 44.0 } else { 12.0 };
        let bottom_margin = if brush {
            if expanded { 14.0 } else { 8.0 }
        } else {
            0.0
        };
        Frame {
            w,
            h,
            left: side + 64.0,
            right: w - side - 64.0,
            top,
            bottom: h - bottom_margin - 30.0,
        }
    }
}

/// Splits a polyline into dashes of `on`/`off` points.
pub fn dashed(points: &[[f32; 2]], on: f32, off: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let (mut drawing, mut left) = (true, on);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        if len <= 0.0 {
            continue;
        }
        let mut t = 0.0;
        if drawing && cur.is_empty() {
            cur.push(a);
        }
        while len - t > left {
            t += left;
            let p = [
                a[0] + (b[0] - a[0]) * t / len,
                a[1] + (b[1] - a[1]) * t / len,
            ];
            if drawing {
                cur.push(p);
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(p);
            }
            drawing = !drawing;
            left = if drawing { on } else { off };
            if !drawing {
                cur.clear();
            }
        }
        left -= len - t;
        if drawing {
            cur.push(b);
        }
    }
    if cur.len() >= 2 && drawing {
        out.push(cur);
    }
    out
}

fn stroke_dashed(
    mut s: Shape,
    pts: &[[f32; 2]],
    dash: Option<(f32, f32)>,
    width: f32,
    color: Color,
) -> Shape {
    match dash {
        None => s.stroke_with(pts.to_vec(), false, width, LineCap::Round, color),
        Some((on, off)) => {
            for d in dashed(pts, on, off) {
                s = s.stroke_with(d, false, width, LineCap::Butt, color);
            }
            s
        }
    }
}

/// A circle as a polygon (fills) — the renderer has rounded rects but no arcs in shapes.
fn circle(cx: f32, cy: f32, r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [cx + r * a.sin(), cy - r * a.cos()]
        })
        .collect()
}

/// A pie slice from 12 o'clock clockwise (`activityFlowSlicePath`), as a fan polygon.
fn slice(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<[f32; 2]> {
    let steps = (((to - from) * 48.0).ceil() as usize).max(2);
    let mut v = vec![[cx, cy]];
    for i in 0..=steps {
        let t = (from + (to - from) * i as f32 / steps as f32) * std::f32::consts::TAU;
        v.push([cx + r * t.sin(), cy - r * t.cos()]);
    }
    v
}

/// Step-after path through the present values (`connectNulls`).
fn step_after(xy: &[(f32, f32)]) -> Vec<[f32; 2]> {
    let mut v = Vec::with_capacity(xy.len() * 2);
    for (i, &(x, y)) in xy.iter().enumerate() {
        if i > 0 {
            v.push([x, xy[i - 1].1]);
        }
        v.push([x, y]);
    }
    v
}

fn luminance_ink(c: Color) -> Color {
    let e = |v: f32| rune::color::srgb_encode(v) * 255.0;
    if (299.0 * e(c.r) + 587.0 * e(c.g) + 114.0 * e(c.b)) / 1000.0 >= 140.0 {
        rune::color::hex("#09090b").unwrap()
    } else {
        Color::WHITE
    }
}

pub struct Ctx<'a> {
    pub app: &'a App,
    pub tk: &'a Tk,
    pub geo: &'a Geo,
    pub view: View,
    pub frame: Frame,
    pub hide: bool,
    pub vis: Visible,
    pub active: Option<SeriesKey>,
    pub cur: &'a str,
}

impl Ctx<'_> {
    fn x(&self, _col: usize, p: &Pt) -> f32 {
        self.geo
            .xaxis
            .x_pt(p, self.frame.left as f64, self.frame.right as f64) as f32
    }
    /// The x of a moment.
    fn x_ms(&self, ms: f64) -> f32 {
        self.geo
            .xaxis
            .x_ms(ms, self.frame.left as f64, self.frame.right as f64) as f32
    }
    fn y_btc(&self, v: f64) -> f32 {
        self.geo
            .btc_axis
            .y(v, self.frame.top as f64, self.frame.bottom as f64) as f32
    }
    fn y_price(&self, v: f64) -> f32 {
        self.geo
            .price_axis
            .y(v, self.frame.top as f64, self.frame.bottom as f64) as f32
    }
    fn dim(&self, key: SeriesKey, rest: f32, dimmed: f32) -> f32 {
        if self.active.is_none() || self.active == Some(key) {
            rest
        } else {
            dimmed
        }
    }
    fn width(&self, key: SeriesKey, rest: f32, hover: f32) -> f32 {
        if self.active == Some(key) {
            hover
        } else {
            rest
        }
    }
    /// A dot's x: on its anchor day's balance point (the end of that day), or at its own
    /// moment in the 30-day view, where it is a stop on the line.
    pub fn marker_x(&self, m: &Pt) -> f32 {
        match &self.geo.xaxis {
            XAxis::Time { .. } => match Day::parse(&m.date).filter(|_| m.date.len() == 10) {
                Some(d) => self
                    .x_ms(d.epoch_days() as f64 * super::data::DAY_MS + super::data::DAY_MS - 1.0),
                None => self.x_ms(m.sort_ms),
            },
            XAxis::PowerLaw { .. } => self.geo.xaxis.x_days(
                m.power_days,
                self.frame.left as f64,
                self.frame.right as f64,
            ) as f32,
        }
    }
}

/// The plot element. `hover` is the hovered column, if any.
pub fn plot(
    c: &Ctx,
    key: &str,
    hover: Option<usize>,
    tip: Option<(El, super::tooltip::TipPlan)>,
) -> El {
    let (tk, f, geo) = (c.tk, c.frame, c.geo);
    // Keys of the plot's children carry the view: the compact card and the full-window chart
    // are both mounted while the dialog is open.
    let vk = c.view as usize;
    let mut shape = Shape::new();
    let grid = tk.border.alpha(tk.border.a * 0.5 * 0.45);
    // Grid: horizontal at the BTC ticks, vertical at the x ticks.
    for t in &geo.btc_axis.ticks {
        let y = c.y_btc(*t);
        if y.is_finite() {
            shape = shape.line([f.left, y], [f.right, y], 1.0, LineCap::Butt, grid);
        }
    }
    let xticks = x_ticks(c);
    for (x, _) in &xticks {
        shape = shape.line([*x, f.top], [*x, f.bottom], 1.0, LineCap::Butt, grid);
    }

    // Basis-incomplete region.
    let mut overlays: Vec<El> = Vec::new();
    if let Some(start_x) = geo.region.map(|r| region_x(c, r)) {
        let hatch = tk.muted_fg.alpha(0.14);
        let (x0, x1, y0, y1) = (start_x, f.right, f.top, f.bottom);
        let step = 6.0 * std::f32::consts::SQRT_2;
        // Lines x - y = k at 45°, clipped to the box.
        let mut k = x0 - y1;
        while k < x1 - y0 {
            let lo = x0.max(y0 + k);
            let hi = x1.min(y1 + k);
            if hi > lo {
                shape = shape.line([lo, lo - k], [hi, hi - k], 1.0, LineCap::Butt, hatch);
            }
            k += step;
        }
        if start_x > f.left + 0.5 {
            for d in dashed(&[[start_x, f.top - 8.0], [start_x, f.bottom]], 3.0, 4.0) {
                shape = shape.stroke_with(
                    d,
                    false,
                    1.0,
                    LineCap::Butt,
                    rune::color::hex("#f59e0b").unwrap().alpha(0.6),
                );
            }
        }
        overlays.push(super::tooltip::region_label(c, start_x));
    }

    // Last-value reference lines and tags. The dashed lines go under the series: in the web
    // they sit exactly on the line in its own colour at 45 % and never show through it.
    let mut tags: Vec<El> = Vec::new();
    let pct_span = geo.btc_axis.hi - geo.btc_axis.lo;
    let btc_label = |v: f64| {
        if geo.pct {
            pct_label(v, pct_span)
        } else {
            format::btc_axis(v, geo.btc_tag_domain)
        }
    };
    let fiat_label = |v: f64| {
        if geo.pct {
            pct_label(v, pct_span)
        } else {
            format::plain_number(v, c.cur)
        }
    };
    // Percent change is not an amount: it stays readable with amounts hidden.
    let values_shown = !c.hide || geo.pct;
    if values_shown && c.app.chart.show_last {
        let mut tag =
            |series: &str, v: Option<f64>, y: f32, color: Color, label: String, left: bool| {
                if v.is_none() || !y.is_finite() {
                    return;
                }
                for d in dashed(&[[f.left, y], [f.right, y]], 2.0, 4.0) {
                    shape = std::mem::take(&mut shape).stroke_with(
                        d,
                        false,
                        1.0,
                        LineCap::Butt,
                        color.alpha(0.45),
                    );
                }
                let w = 30f32.max((label.chars().count() as f32 * 5.8).round() + 10.0);
                let x = if left { f.left + 2.0 } else { f.right + 2.0 };
                tags.push(last_tag(
                    series,
                    vk,
                    x,
                    y,
                    w,
                    color,
                    text(label)
                        .font(tk.px(9.5, 600.0))
                        .color(luminance_ink(color)),
                ));
            };
        if c.vis.primary
            && let Some(v) = geo.last_bal
        {
            tag("btc", Some(v), c.y_btc(v), tk.chart_btc, btc_label(v), true);
        }
        if c.vis.price
            && let Some(v) = geo.last_price
        {
            tag(
                "price",
                Some(v),
                c.y_price(v),
                tk.chart_price,
                fiat_label(v),
                false,
            );
        }
        if c.vis.portfolio_value
            && let Some(v) = geo.last_value
        {
            tag(
                "value",
                Some(v),
                c.y_price(v),
                tk.chart_value,
                fiat_label(v),
                false,
            );
        }
        if c.vis.basis
            && geo.basis_trusted
            && let Some(v) = geo.last_avg
        {
            tag(
                "basis",
                Some(v),
                c.y_price(v),
                tk.chart_basis,
                fiat_label(v),
                false,
            );
        }
    }

    // Lines.
    let pts = &geo.plot;
    let collect = |pick: &dyn Fn(&Pt) -> Option<f64>, y: &dyn Fn(f64) -> f32| -> Vec<(f32, f32)> {
        pts.iter()
            .enumerate()
            .filter_map(|(i, p)| pick(p).map(|v| (c.x(i, p), y(v))))
            .filter(|(_, y)| y.is_finite())
            .collect()
    };
    if c.vis.portfolio_value {
        let xy = collect(&|p| p.l_val, &|v| c.y_price(v));
        let col = tk
            .chart_value
            .alpha(c.dim(SeriesKey::PortfolioValue, 0.95, 0.24));
        shape = stroke_dashed(
            shape,
            &step_after(&xy),
            None,
            c.width(SeriesKey::PortfolioValue, 2.15, 3.0),
            col,
        );
    }
    if c.vis.price {
        let xy: Vec<[f32; 2]> = collect(&|p| p.l_price, &|v| c.y_price(v))
            .into_iter()
            .map(|(x, y)| [x, y])
            .collect();
        let col = tk.chart_price.alpha(c.dim(SeriesKey::Price, 0.72, 0.2));
        shape = stroke_dashed(
            shape,
            &xy,
            Some((3.0, 5.0)),
            c.width(SeriesKey::Price, 1.6, 2.4),
            col,
        );
    }
    if c.vis.basis {
        let xy = collect(&|p| p.l_avg, &|v| c.y_price(v));
        let col = tk.chart_basis.alpha(c.dim(SeriesKey::Basis, 1.0, 0.32));
        shape = stroke_dashed(
            shape,
            &step_after(&xy),
            Some((5.0, 5.0)),
            c.width(SeriesKey::Basis, 2.0, 3.0),
            col,
        );
    }
    if c.vis.primary {
        let xy = collect(&|p| p.l_bal, &|v| c.y_btc(v));
        let col = tk.chart_btc.alpha(c.dim(SeriesKey::Primary, 1.0, 0.3));
        shape = stroke_dashed(
            shape,
            &step_after(&xy),
            None,
            c.width(SeriesKey::Primary, 2.5, 3.0),
            col,
        );
    }

    let v = c.view as usize;
    let ui = &c.app.chart;
    let cols: Rc<Vec<f32>> = Rc::new(pts.iter().enumerate().map(|(i, p)| c.x(i, p)).collect());
    let gesture = ui.gesture[v].filter(|g| g.moved);
    // Everything that follows the pointer. Its container lands layout changes at once: the
    // markers sit on the pointer's column, they don't spring after it.
    let mut cursor_els: Vec<El> = Vec::new();

    // Selection and measurement bands (indices into the display points).
    let band_x = |a: usize, b: usize| -> Option<(f32, f32)> {
        let (lo, hi) = (a.min(b), a.max(b));
        if cols.is_empty() || hi < geo.range.start || lo > geo.range.end {
            return None;
        }
        let x = |i: usize| {
            geo.display
                .get(i.clamp(geo.range.start, geo.range.end))
                .map(|p| c.x(0, p).clamp(f.left, f.right))
        };
        Some((x(lo)?, x(hi)?))
    };
    let band = |shape: Shape, x0: f32, x1: f32, color: Color| -> Shape {
        let mut s = shape.fill(
            vec![[x0, f.top], [x1, f.top], [x1, f.bottom], [x0, f.bottom]],
            color.alpha(0.12),
        );
        for x in [x0, x1] {
            s = s.line(
                [x, f.top],
                [x, f.bottom],
                1.0,
                LineCap::Butt,
                color.alpha(0.7),
            );
        }
        s
    };
    let day = |i: usize| {
        geo.display
            .get(i)
            .map(|p| {
                Day::parse(&p.date)
                    .map(|d| d.long_ui())
                    .unwrap_or_else(|| p.detail.clone())
            })
            .unwrap_or_default()
    };
    let pill = |label: String, cx: f32, y: f32| {
        let w = label.chars().count() as f32 * tk.t2xs(500.0).size * 0.58 + 16.0;
        row()
            .absolute((cx - w / 2.0).clamp(f.left, (f.right - w).max(f.left)), y)
            .size(w, 20.0)
            .center()
            .radius(10.0)
            .fill(tk.popover)
            .border(1.0, tk.border)
            .inert()
            .child(
                text(label)
                    .font(tk.t2xs(500.0).features("'tnum' 1"))
                    .color(tk.fg),
            )
    };
    let measuring = gesture
        .filter(|g| g.kind == GestureKind::Measure)
        .map(|g| (g.from, g.to))
        .or(ui.measure[v]);
    if let Some(g) = gesture.filter(|g| g.kind == GestureKind::Zoom && g.from != g.to)
        && let Some((x0, x1)) = band_x(g.from, g.to)
    {
        shape = band(shape, x0, x1, tk.fg);
        cursor_els.push(pill(
            format!(
                "{} \u{2013} {}",
                day(g.from.min(g.to)),
                day(g.from.max(g.to))
            ),
            (x0 + x1) / 2.0,
            f.top + 6.0,
        ));
    }
    if let Some((a, b)) = measuring.filter(|(a, b)| a != b) {
        let (lo, hi) = (a.min(b), a.max(b));
        if let (Some((x0, x1)), Some(p0), Some(p1)) =
            (band_x(lo, hi), geo.display.get(lo), geo.display.get(hi))
        {
            let up = p1.balance >= p0.balance;
            shape = band(shape, x0, x1, if up { tk.good.solid } else { tk.kb_accent });
            cursor_els.push(measure_card(c, p0, p1, &day(lo), &day(hi), (x0 + x1) / 2.0));
        }
    }

    // Hover: crosshair and active dots on each visible line.
    if let Some(i) = hover.filter(|i| *i < pts.len() && gesture.is_none()) {
        let p = &pts[i];
        let x = c.x(i, p);
        let hair = tk.border.alpha(tk.border.a.max(0.25) * 0.8);
        shape = shape.line([x, f.top], [x, f.bottom], 1.0, LineCap::Butt, hair);
        let r = if c.view == View::Expanded { 4.0 } else { 3.0 };
        let mut dot = |v: Option<f64>, y: &dyn Fn(f64) -> f32, color: Color, r: f32| {
            if let Some(v) = v {
                let yy = y(v);
                if yy.is_finite() {
                    cursor_els.push(
                        el().absolute(x - r - 1.0, yy - r - 1.0)
                            .size(2.0 * r + 2.0, 2.0 * r + 2.0)
                            .radius(r + 1.0)
                            .fill(color)
                            .border(2.0, Color::WHITE)
                            .inert(),
                    );
                }
            }
        };
        if c.vis.primary {
            dot(p.l_bal, &|v| c.y_btc(v), tk.chart_btc, r);
        }
        if c.vis.portfolio_value {
            dot(p.l_val, &|v| c.y_price(v), tk.chart_value, r);
        }
        if c.vis.price {
            dot(p.l_price, &|v| c.y_price(v), tk.chart_price, r - 1.0);
        }
        if c.vis.basis {
            dot(p.l_avg, &|v| c.y_price(v), tk.chart_basis, r);
        }
        // Crosshair: the horizontal line at the pointer, its value on each axis, and the date.
        if let Some(y) = ui.cross_y[v].filter(|y| ui.crosshair && *y >= f.top && *y <= f.bottom) {
            for d in dashed(&[[f.left, y], [f.right, y]], 3.0, 3.0) {
                shape = shape.stroke_with(d, false, 1.0, LineCap::Butt, hair);
            }
            let tag = |label: String, x: f32, y: f32, right_aligned: bool, centred: bool| {
                let w = 30f32.max((label.chars().count() as f32 * 5.8).round() + 10.0);
                let x = if centred {
                    (x - w / 2.0).clamp(f.left, (f.right - w).max(f.left))
                } else if right_aligned {
                    x - w
                } else {
                    x
                };
                row()
                    .absolute(x, y - 7.5)
                    .size(w, 15.0)
                    .radius(3.0)
                    .fill(tk.fg)
                    .center()
                    .inert()
                    .child(
                        text(label)
                            .font(tk.px(9.5, 600.0).features("'tnum' 1"))
                            .color(tk.background),
                    )
            };
            if values_shown && (c.vis.primary || c.vis.events) {
                let val = geo
                    .btc_axis
                    .value_at(y as f64, f.top as f64, f.bottom as f64);
                let label = if geo.pct {
                    pct_label(val, pct_span)
                } else {
                    format::btc_axis(val, geo.btc_axis.fit.or(geo.btc_tag_domain))
                };
                cursor_els.push(tag(label, f.left - 4.0, y, true, false));
            }
            if !c.hide && geo.fiat_axis && (c.vis.price || c.vis.portfolio_value || c.vis.basis) {
                let val = geo
                    .price_axis
                    .value_at(y as f64, f.top as f64, f.bottom as f64);
                cursor_els.push(tag(
                    format::plain_number(val, c.cur),
                    f.right + 2.0,
                    y,
                    false,
                    false,
                ));
            }
            cursor_els.push(tag(day(geo.display_of(i)), x, f.bottom + 20.2, false, true));
        }
    }

    // Axis labels.
    // Recharts' tick text: 10 px, its default `#666` fill in both themes, drawn `tickSize` (6)
    // + `tickMargin` (4) off the plot. Each label is a 10 px line box centred on its value.
    const TICK_LINE: f32 = 10.0;
    let tick_style = tk.px(10.0, 400.0).lh(TICK_LINE);
    let tick_ink = rune::color::hex("#666666").expect("colour");
    let mut labels: Vec<El> = Vec::new();
    for (x, label) in xticks {
        // XAxis: `dy={8}` plus the default `0.71em` baseline shift: the line centres 20.2 pt
        // under the plot.
        labels.push(
            row()
                .key(format!("xtick-{vk}-{}", labels.len()))
                .absolute(x - 50.0, f.bottom + 20.2 - TICK_LINE / 2.0)
                .w(100.0)
                .justify_center()
                .inert()
                .child(text(label).font(tick_style).color(tick_ink)),
        );
    }
    if values_shown {
        for t in &geo.btc_axis.ticks {
            let y = c.y_btc(*t);
            if y.is_finite() {
                let n = labels.len();
                labels.push(
                    row()
                        .key(format!("btctick-{vk}-{n}"))
                        .absolute(f.left - 10.0 - 80.0, y - TICK_LINE / 2.0)
                        .w(80.0)
                        .justify_end()
                        .inert()
                        .child(
                            text(if geo.pct {
                                pct_label(*t, pct_span)
                            } else {
                                format::btc_axis(*t, geo.btc_axis.fit)
                            })
                            .font(tick_style)
                            .color(tick_ink),
                        ),
                );
            }
        }
        if geo.fiat_axis {
            for t in &geo.price_axis.ticks {
                let y = c.y_price(*t);
                if y.is_finite() {
                    // Recharts' `Text` wraps at spaces when the label is wider than the axis
                    // (64 − 6 − 4 = 54 pt): `120 000` over `EUR`, centred on the value.
                    let lines = super::metrics::wrap(&format::fiat_price(*t, c.cur), 10.0, 54.0);
                    let top = y - lines.len() as f32 * TICK_LINE / 2.0;
                    let mut col_el = rune::col()
                        .key(format!("fiattick-{vk}-{}", labels.len()))
                        .absolute(f.right + 10.0, top)
                        .inert();
                    for l in lines {
                        col_el = col_el.child(text(l).font(tick_style).color(tick_ink));
                    }
                    labels.push(col_el);
                }
            }
        }
    }

    // Activity dots.
    let dots: Vec<El> = if c.vis.events {
        geo.markers
            .iter()
            .enumerate()
            .map(|(i, m)| dot_el(c, i, m))
            .collect()
    } else {
        vec![]
    };

    // The hover marks (crosshair tags, active dots) and the selection and measure cards paint
    // above the rest of the card (the Reset zoom pill, the brush), as the hover card does, but
    // not over an open chart menu or over the full-window chart from the card behind it.
    let mut cursor_layer = el()
        .key(format!("plot-cursor-{vk}"))
        .absolute(0.0, 0.0)
        .size(f.w, f.h)
        .inert()
        .instant_changes()
        .children(cursor_els);
    if ui.popup.is_none() && !(ui.expanded && c.view == View::Compact) {
        cursor_layer = cursor_layer.raise();
    }
    let view = c.view;
    let (left, right, top, bottom) = (f.left, f.right, f.top, f.bottom);
    let live = ui.tip_pos.clone();
    let cross = ui.crosshair;
    let move_cols = cols.clone();
    let (tip_anchor, tip_plan) = tip.map_or((None, None), |(a, p)| (Some(a), Some(p)));
    let mut root = surface(key)
        .w_full()
        .h(f.h)
        .cursor(rune::Cursor::Crosshair)
        // Focusable so the zoom and pan keys work on the compact card; a click focuses it.
        .role("group")
        .label(strings::plot_label())
        .description(strings::plot_keys())
        .focusable(true)
        .on_pointer_move_local(move |s: &mut App, e, tx: &mut Tx| {
            let cols = &move_cols;
            let v = view as usize;
            if e.x < left - 4.0 || e.x > right + 4.0 || cols.is_empty() {
                let had = s.chart.hover[v].take().is_some() | s.chart.cross_y[v].take().is_some();
                if !had {
                    tx.live_only();
                }
                return;
            }
            // A drag owns the plot: no hover card under the selection.
            if s.chart.gesture[v].is_some_and(|g| g.moved) {
                tx.live_only();
                return;
            }
            // Nearest column (band centers or power-law positions).
            let i = super::interact::nearest(cols, e.x);
            live.set([e.x, e.y]);
            if let Some(plan) = &tip_plan {
                plan.follow([e.x, e.y], [e.window_x, e.window_y]);
            }
            // Half-point steps: the value labels change no faster than they can be read.
            let cy = (cross && e.y >= top && e.y <= bottom).then(|| (e.y * 2.0).round() / 2.0);
            if s.chart.hover[v] == Some(i) && s.chart.cross_y[v] == cy {
                tx.live_only();
            } else {
                s.chart.hover[v] = Some(i);
                s.chart.cross_y[v] = cy;
                if view == View::Expanded {
                    s.chart.inspect_col = Some(i);
                }
            }
        })
        .on_hover_change(move |s: &mut App, on, _tx: &mut Tx| {
            if !on {
                s.chart.hover[view as usize] = None;
                s.chart.hover_dot[view as usize] = None;
                s.chart.cross_y[view as usize] = None;
            }
        })
        .on_double_press_tx(move |s: &mut App, tx: &mut Tx| {
            s.chart.reset_zoom(view);
            tx.instant();
        })
        .child(
            canvas(shape)
                .key(format!("plot-shape-{vk}"))
                .absolute(0.0, 0.0)
                .size(f.w, f.h)
                .inert(),
        )
        .children(labels)
        .children(dots)
        // The last-value tags cover the dots and the axis ticks (the web's `zIndex={2100}`):
        // zoomed in, the first dot can sit on the balance tag at the plot's left edge.
        .children(tags)
        .children(overlays)
        .child(cursor_layer);
    root = super::interact::attach(root, c, cols);
    if geo.zoomed {
        // Shown only while zoomed: the way back, next to the range it undoes.
        let label = strings::reset_zoom();
        let w = label.chars().count() as f32 * tk.t2xs(500.0).size * 0.58 + 18.0;
        root = root.child(
            row()
                .key(format!("zoom-reset-{}", view as usize))
                // Bottom right: the lines and dots gather at the top, the zero line is quiet.
                .absolute(f.right - w - 6.0, f.bottom - 26.0)
                .size(w, 20.0)
                .center()
                .radius(10.0)
                .fill(tk.popover)
                .hover(tk.muted)
                .border(1.0, tk.border)
                .role("button")
                .label(label.clone())
                .help(format!("{label} (0)"))
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    s.chart.reset_zoom(view);
                    tx.instant();
                })
                .child(text(label).font(tk.t2xs(500.0)).color(tk.fg)),
        );
    }
    if ui.measure_armed && gesture.is_none() {
        let label = strings::measure_hint();
        let w = label.chars().count() as f32 * tk.t2xs(500.0).size * 0.58 + 18.0;
        root = root.child(
            row()
                .key(format!("measure-hint-{vk}"))
                .absolute((f.left + f.right - w) / 2.0, f.top + 6.0)
                .size(w, 20.0)
                .center()
                .radius(10.0)
                .fill(tk.fg)
                .inert()
                .child(text(label).font(tk.t2xs(500.0)).color(tk.background)),
        );
    }
    if let Some(anchor) = tip_anchor {
        root = root.child(anchor);
    }
    root
}

/// The plot's own element. Everything in it is placed by the data's domain, so on a range
/// change, zoom or pan it lands at once, as the web's Recharts layers do
/// (`isAnimationActive={false}`). Eased, a tag or dot glides from a stale place (or a stale
/// element's rect) across the plot.
fn surface(key: &str) -> El {
    el().key(key.to_owned()).instant_changes()
}

/// One last-value tag: the pill on the plot's edge at `y`, `w` wide, in its series' colour.
/// Keyed per series and view: an unkeyed tag was matched by its index among the plot's children,
/// so a range change that shifted the label and dot counts handed it the full-plot cursor layer's
/// rect, and the balance tag swept the whole chart orange while it shrank back to its place.
fn last_tag(series: &str, view: usize, x: f32, y: f32, w: f32, color: Color, label: El) -> El {
    row()
        .key(format!("last-{series}-{view}"))
        .absolute(x, y - 7.5)
        .size(w, 15.0)
        .radius(3.0)
        .fill(color)
        .center()
        .inert()
        .child(label)
}

/// The measurement between two display points: change in balance and in value, each with its
/// percentage, and the time between them.
fn measure_card(c: &Ctx, p0: &Pt, p1: &Pt, from: &str, to: &str, cx: f32) -> El {
    let (tk, f) = (c.tk, c.frame);
    let pct = |d: f64, base: f64| {
        if base.abs() > 1e-12 {
            format!(" ({})", pct_label(d / base.abs() * 100.0, 10.0))
        } else {
            String::new()
        }
    };
    let tone = |d: f64| {
        if d > 0.0 {
            tk.good.solid
        } else if d < 0.0 {
            tk.kb_accent
        } else {
            tk.fg
        }
    };
    let (db, dv) = (p1.balance - p0.balance, p1.value_eur - p0.value_eur);
    let days = ((p1.sort_ms - p0.sort_ms) / super::data::DAY_MS).round() as i64;
    let line = |label: String, value: String, color: Color| {
        row()
            .w_full()
            .gap(12.0)
            .justify_between()
            .child(text(label).font(tk.t2xs(400.0)).color(tk.muted_fg))
            .child(crate::ui::sensitive(
                value,
                tk.t2xs(600.0).features("'tnum' 1"),
                color,
                c.hide,
            ))
    };
    const W: f32 = 236.0;
    col()
        .absolute(
            (cx - W / 2.0).clamp(f.left, (f.right - W).max(f.left)),
            f.top + 6.0,
        )
        .w(W)
        .px(8.0)
        .py(6.0)
        .gap(2.0)
        .radius(tk.r(0.375))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .inert()
        .child(line(
            strings::measure_balance(),
            format!("{}{}", format::btc(db, 8, true), pct(db, p0.balance)),
            tone(db),
        ))
        .child(line(
            strings::measure_value(),
            format!(
                "{}{}",
                format::signed_fiat(dv, c.cur),
                pct(dv, p0.value_eur)
            ),
            tone(dv),
        ))
        .child(
            text(format!(
                "{from} \u{2013} {to} \u{00b7} {}",
                strings::days(days)
            ))
            .font(tk.t3xs(500.0))
            .color(tk.muted_fg),
        )
}

/// X ticks with labels: evenly spaced calendar ticks over the visible window (the web's
/// category ticks fall on whichever points exist, so a gap in the history bunched them), or
/// the power-law axis's years.
fn x_ticks(c: &Ctx) -> Vec<(f32, String)> {
    let f = c.frame;
    match &c.geo.xaxis {
        XAxis::PowerLaw { lo, hi } => super::axes::power_law_ticks((*lo, *hi), 6)
            .into_iter()
            .map(|(d, l)| {
                (
                    c.geo.xaxis.x_days(d, f.left as f64, f.right as f64) as f32,
                    l,
                )
            })
            .collect(),
        XAxis::Time { lo, hi } => {
            let max = if c.view == View::Expanded { 10 } else { 8 };
            super::axes::time_ticks(*lo, *hi, f.right - f.left, max)
                .1
                .into_iter()
                .map(|(t, l)| (c.x_ms(t), l))
                .collect()
        }
    }
}

/// The x where the basis-incomplete region starts (`from` ms).
fn region_x(c: &Ctx, from: f64) -> f32 {
    let f = c.frame;
    if from == f64::NEG_INFINITY {
        return f.left;
    }
    match c
        .geo
        .plot
        .iter()
        .position(|p| super::data::day_ms(&p.date) + super::data::DAY_MS > from)
    {
        Some(0) | None => f.left,
        Some(i) => c.x(i, &c.geo.plot[i]),
    }
}

/// One activity dot: hit area, the dot (or a mixed-flow pie), and a count label when merged.
fn dot_el(c: &Ctx, i: usize, m: &Pt) -> El {
    let tk = c.tk;
    let e = m.ev.as_ref().unwrap();
    let x = c.marker_x(m);
    let y = c.y_btc(e.marker_bal);
    if !y.is_finite() {
        return el().size(0.0, 0.0);
    }
    let (amin, amax) = if c.view == View::Expanded {
        (80.0, 620.0)
    } else {
        (80.0, 480.0)
    };
    let area = amin + (amax - amin) * (e.size / c.geo.max_size.max(1e-12)) as f32;
    let r = (area / std::f32::consts::PI).sqrt().max(3.0);
    let merged = e.count > 1;
    let hit = if merged {
        (r + 2.0).max(8.0)
    } else {
        (r + 6.0).max(10.0)
    };
    let hovered = c.app.chart.hover_dot[c.view as usize] == Some(i);
    let opacity = if c.active.is_some() && c.active != Some(SeriesKey::Events) {
        0.28
    } else {
        0.92
    };
    let scale = if hovered { 1.1 } else { 1.0 };
    // The hover grows the dot by a transform, as the web's `scale-110` does, so its layout stays
    // put; only that scale eases (`duration-150 ease-out`). The label still sits on the grown dot.
    let rr = r * scale;
    // The ring is `stroke=\"var(--background)\"`, the page colour, not the card's.
    let bg = tk.background;
    let body: El = if e.mixed {
        let mut s = Shape::new();
        let mut from = 0.0f32;
        let shares = super::data::flow_shares(&e.members);
        for (flow, share) in &shares {
            let to = from + *share as f32;
            s = s.fill(
                slice(r, r, r, from, to),
                flow_color(tk, *flow).alpha(opacity),
            );
            from = to;
        }
        s = s.stroke_with(circle(r, r, r, 40), true, 2.5, LineCap::Butt, bg);
        canvas(s).size(2.0 * r, 2.0 * r)
    } else {
        el().size(2.0 * r + 2.5, 2.0 * r + 2.5)
            .radius(r + 1.25)
            .fill(flow_color(tk, e.flow).alpha(opacity))
            .border(2.5, bg)
    };
    // The plot lands its changes at once; this body opts back in for the hover scale alone. Its
    // layout and colours stay instant (`Spring::NONE`) and it mounts without a fade: it moves
    // with its dot on a range change and takes another dot's colour at once when dots re-bin.
    let body = body
        .motion(rune::Spring::NONE)
        .transition(rune::Transition::none())
        .scale(scale)
        .scale_motion(rune::Curve::tween(0.15, rune::Easing::EASE_OUT))
        .animated_changes();
    let label = if merged {
        tc("overview:treasury.marker.openGroup", e.count, &[])
    } else {
        tr(
            "overview:treasury.marker.openTransaction",
            &[("flow", &e.flow.label())],
        )
    };
    let view = c.view;
    let members: Vec<String> = e
        .members
        .iter()
        .filter_map(|p| p.ev.as_ref().and_then(|e| e.tx_id.clone()))
        .collect();
    let tx_id = e.tx_id.clone().or(e.id.clone());
    let mut d = col()
        .key(format!("dot-{}-{i}", view as usize))
        .absolute(x - hit, y - hit)
        .size(2.0 * hit, 2.0 * hit)
        .center()
        .role("button")
        .label(label)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_hover_change(move |s: &mut App, on, _tx: &mut Tx| {
            let h = &mut s.chart.hover_dot[view as usize];
            if on {
                *h = Some(i);
            } else if *h == Some(i) {
                *h = None;
            }
        })
        .on_press(move |s: &mut App| {
            if members.len() > 1 {
                let href = super::transaction_set_href(s, &members);
                s.navigate(&href);
            } else if let Some(id) = &tx_id {
                crate::overview::open_transaction_detail(s, id);
            }
        })
        .child(body);
    if merged {
        // The count's SVG baseline sits at `cy - r - 4`; the web paints a 3 px stroke in the page
        // background under the digits (`paint-order: stroke`). Rune text has no stroke, so one
        // heavier, slightly blurred copy in the background colour stands in for it.
        let label = e.count.to_string();
        let st = tk.t2xs(600.0).features("'tnum' 1");
        let line = st.line_px();
        // Baseline offset inside a DM Sans line box: half-leading + ascent (0.992 em).
        let baseline = (line - 1.302 * st.size) / 2.0 + 0.992 * st.size;
        let top = hit - rr - 4.0 - baseline;
        let halo = text(label.clone())
            .font(rune::TextStyle {
                weight: 1000.0,
                ..st
            })
            .color(tk.background)
            .blur(0.7)
            .absolute(hit - 20.0, top)
            .w(40.0)
            .align(rune::Align::Center)
            .a11y_hidden();
        d = d.child(halo).child(
            text(label)
                .font(st)
                .color(tk.fg)
                .absolute(hit - 20.0, top)
                .w(40.0)
                .align(rune::Align::Center),
        );
    }
    d
}

#[cfg(test)]
mod tests {
    use rune::{App as RuneApp, AppSpec, Cx, El, Env, Runtime, canvas, el, text};

    use super::{Color, Shape, last_tag, surface};

    /// A plot whose range decides how many unkeyed children (tick labels, dots) come before the
    /// tag, and a full-plot layer after it, as `plot` lays them out; a button switches range.
    #[derive(Default)]
    struct Plot {
        wide: bool,
        fixed: bool,
    }

    const ORANGE: Color = Color {
        r: 0.93,
        g: 0.36,
        b: 0.01,
        a: 1.0,
    };

    fn view(p: &Plot, _: &mut Cx<Plot>) -> El {
        // One more child before the tag in the wide range: the tag then lands on the index the
        // full-plot layer held in the narrow one, as the balance tag did on All → YTD.
        let n = if p.wide { 7 } else { 6 };
        let tag = if p.fixed {
            last_tag("btc", 0, 70.0, 30.0, 33.0, ORANGE, text("B111"))
        } else {
            rune::row()
                .absolute(70.0, 22.5)
                .size(33.0, 15.0)
                .fill(ORANGE)
                .child(text("B111"))
        };
        let mut plot = if p.fixed {
            surface("plot")
        } else {
            el().key("plot")
        }
        .w(800.0)
        .h(300.0);
        plot = plot.child(canvas(Shape::new()).absolute(0.0, 0.0).size(800.0, 300.0));
        for i in 0..n {
            plot = plot.child(el().absolute(i as f32 * 12.0, 280.0).size(10.0, 10.0));
        }
        let layer = el().absolute(0.0, 0.0).size(800.0, 300.0);
        let layer = if p.fixed {
            layer.key("plot-cursor-0")
        } else {
            layer
        };
        rune::col()
            .child(
                el().key("switch")
                    .size(60.0, 20.0)
                    .role("button")
                    .label("Range")
                    .on_press(|p: &mut Plot| p.wide = !p.wide),
            )
            .child(plot.child(tag).child(layer))
    }

    fn runtime(fixed: bool) -> Runtime<Plot> {
        let mut rt = RuneApp::new(AppSpec::new("plot", (900.0, 400.0), Plot::default, view))
            .headless_runtime((900.0, 400.0), Env::new(true, 1.0));
        rt.state.fixed = fixed;
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    /// The widest orange element on the plot, one frame after each of a few range switches.
    fn widest_orange(rt: &mut Runtime<Plot>) -> f32 {
        let mut widest: f32 = 0.0;
        for _ in 0..4 {
            let (x, y) = rt.find("switch").expect("switch");
            rt.pointer_move(x, y);
            rt.pointer_down(x, y);
            rt.pointer_up(x, y);
            rt.frame(1.0 / 60.0);
            for e in rt
                .elements()
                .into_iter()
                .filter(|e| !e.exiting && e.fill.a > 0.0 && (e.fill.r - ORANGE.r).abs() < 0.05)
            {
                widest = widest.max(e.rect[2]);
            }
            for _ in 0..30 {
                rt.frame(1.0 / 60.0);
            }
        }
        widest
    }

    #[test]
    fn a_range_switch_never_spreads_the_balance_tag_over_the_plot() {
        let mut rt = runtime(true);
        assert!(
            (widest_orange(&mut rt) - 33.0).abs() < 0.5,
            "the tag keeps its own 33 pt from the first frame of each switch"
        );
        let tag = rt
            .elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("last-btc-0"))
            .expect("tag");
        assert_eq!(tag.rect, tag.target, "nothing in the plot is still easing");
    }

    /// The control: the same plot with an unkeyed tag in an eased container reproduces the flash,
    /// so the test above is not vacuous.
    #[test]
    fn the_unkeyed_eased_tag_did_flash() {
        let mut rt = runtime(false);
        assert!(
            widest_orange(&mut rt) > 300.0,
            "the old tag took a full-plot sibling's rect on a switch"
        );
    }
}
