//! Direct manipulation of the plot: drag to zoom, Shift-drag to measure, Option-drag (or a
//! drag on the date axis) to pan, wheel and pinch zoom around the pointer, and the keyboard.
//!
//! The zoom is the brush range (`ChartUi::brush`), in indices into `Geo::display`, so the
//! brush under the plot and these gestures are one state and follow each other.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, Phase, Tx};

use super::data::{self, Win};
use super::plot::Ctx;
use super::{ChartUi, View, geo};
use crate::app::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    Zoom,
    Measure,
    Pan,
}

/// A press on the plot that may become a drag. `from` / `to` index `Geo::display`.
#[derive(Clone, Copy, Debug)]
pub struct PlotGesture {
    pub kind: GestureKind,
    pub from: usize,
    pub to: usize,
    /// Past the click slop: a drag, not a click.
    pub moved: bool,
    /// The stored window and the shown one at the press, to pan from and to restore on cancel.
    pub brush_at_press: Option<Win>,
    pub range_at_press: Win,
}

/// Pointer travel under which a press and release is a click.
const SLOP: f32 = 4.0;

/// The window `factor` times as long as `cur` (below 1 zooms in), keeping the moment `anchor`
/// at the same fraction of it, within the history and the minimum window.
pub fn zoom_about(first: f64, last: f64, cur: Win, anchor: f64, factor: f64) -> Win {
    if !factor.is_finite() || factor <= 0.0 || last <= first || last.is_nan() || first.is_nan() {
        return cur;
    }
    let total = last - first;
    let span = cur.span();
    let new_span = (span * factor).clamp(data::BRUSH_MIN_WINDOW_MS.min(total), total);
    let frac = if span > 0.0 {
        ((anchor - cur.lo) / span).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let lo = anchor.clamp(cur.lo, cur.hi) - frac * new_span;
    data::clamp_win(
        Win {
            lo,
            hi: lo + new_span,
        },
        first,
        last,
    )
}

/// `cur` moved by `shift` ms, its length kept, stopping at the ends of the history.
pub fn pan_win(first: f64, last: f64, cur: Win, shift: f64) -> Win {
    let shift = shift.clamp((first - cur.lo).min(0.0), (last - cur.hi).max(0.0));
    Win {
        lo: cur.lo + shift,
        hi: cur.hi + shift,
    }
}

/// The window a zoom drag between two display points selects (minimum window applied).
pub fn select(times: &[f64], a: usize, b: usize) -> Win {
    let (Some(&first), Some(&last)) = (times.first(), times.last()) else {
        return Win { lo: 0.0, hi: 0.0 };
    };
    let (ta, tb) = (times[a.min(times.len() - 1)], times[b.min(times.len() - 1)]);
    data::clamp_win(
        Win {
            lo: ta.min(tb),
            hi: ta.max(tb),
        },
        first,
        last,
    )
}

impl ChartUi {
    /// Stores the visible window; the range chip's own window is "not zoomed".
    pub fn set_zoom(&mut self, view: View, chip: Win, w: Win) {
        self.brush[view as usize] = if w.same(&chip) { None } else { Some(w) };
    }
    pub fn reset_zoom(&mut self, view: View) {
        self.brush[view as usize] = None;
        self.wheel_acc[view as usize] = [0.0; 3];
    }
    pub fn set_pct(&mut self, on: bool) {
        self.pct = on;
        if on {
            self.log_y = false;
        }
    }
    pub fn set_log_y(&mut self, on: bool) {
        self.log_y = on;
        if on {
            self.pct = false;
        }
    }
    /// Drops everything that indexes the display points (they change with the period).
    pub(super) fn clear_plot_tools(&mut self) {
        self.gesture = [None; 2];
        self.measure = [None; 2];
        self.cross_y = [None; 2];
        self.wheel_acc = [[0.0; 3]; 2];
    }
}

/// Nearest plot column to `x` (plot points).
pub fn nearest(cols: &[f32], x: f32) -> usize {
    match cols.binary_search_by(|c| c.total_cmp(&x)) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) if i >= cols.len() => cols.len().saturating_sub(1),
        Err(i) => {
            if (cols[i] - x).abs() < (x - cols[i - 1]).abs() {
                i
            } else {
                i - 1
            }
        }
    }
}

/// What the plot's handlers need from the built geometry.
#[derive(Clone)]
struct Handles {
    view: View,
    cols: Rc<Vec<f32>>,
    /// The display index of each column.
    idx: Rc<Vec<usize>>,
    times: Rc<Vec<f64>>,
    display: Rc<Vec<data::Pt>>,
    win: Win,
    chip: Win,
    /// The plot's sampling cap, to find the columns of a new window.
    max: usize,
    left: f32,
    right: f32,
    bottom: f32,
}

impl Handles {
    fn first(&self) -> f64 {
        self.times.first().copied().unwrap_or(0.0)
    }
    fn last(&self) -> f64 {
        self.times.last().copied().unwrap_or(0.0)
    }
    fn display_at(&self, x: f32) -> usize {
        self.idx.get(nearest(&self.cols, x)).copied().unwrap_or(0)
    }
    /// The moment under `x` in a window.
    fn t_at(&self, w: Win, x: f32) -> f64 {
        w.lo + w.span() * ((x - self.left) / (self.right - self.left).max(1.0)) as f64
    }
    /// Milliseconds per point of pointer travel at a window.
    fn ms_per_pt(&self, w: Win) -> f64 {
        w.span() / (self.right - self.left).max(1.0) as f64
    }
    /// The column of a new window nearest to `x`, so a hover survives the zoom or pan that
    /// moved the plot under a still pointer.
    fn col_in(&self, next: Win, x: f32) -> Option<usize> {
        if x < self.left - 4.0 || x > self.right + 4.0 {
            return None;
        }
        let ts: Vec<f64> = super::plot_columns(&self.display, &self.times, next, self.max)
            .into_iter()
            .map(|c| c.1)
            .collect();
        (!ts.is_empty()).then(|| data::nearest_time(&ts, self.t_at(next, x)))
    }
    fn zoom(&self, s: &mut App, x: f32, factor: f64) {
        let next = zoom_about(
            self.first(),
            self.last(),
            self.win,
            self.t_at(self.win, x),
            factor,
        );
        s.chart.set_zoom(self.view, self.chip, next);
        s.chart.hover[self.view as usize] = self.col_in(next, x);
    }
    /// Pans by `shift` ms. With the pointer still over the plot at `x`, the hover card stays
    /// up on the column now under it instead of vanishing until the pointer moves.
    fn pan(&self, s: &mut App, shift: f64, x: Option<f32>) {
        let next = pan_win(self.first(), self.last(), self.win, shift);
        if next.same(&self.win) {
            return;
        }
        s.chart.set_zoom(self.view, self.chip, next);
        let v = self.view as usize;
        s.chart.hover[v] = x.and_then(|x| self.col_in(next, x));
        if self.view == View::Expanded {
            s.chart.inspect_col = s.chart.hover[v];
        }
    }
}

/// Wheel travel (points) per zoom step, and the zoom per step.
const WHEEL_STEP: f32 = 14.0;
const ZOOM_STEP: f64 = 0.85;
const PINCH_STEP: f32 = 0.05;

/// Adds the drag, wheel, pinch and key handlers to the plot element.
pub fn attach(root: El, c: &Ctx, cols: Rc<Vec<f32>>) -> El {
    let f = c.frame;
    let g = c.geo;
    let max = data::sample_max(g.resolved, c.view == View::Expanded);
    let h = Handles {
        view: c.view,
        cols,
        idx: Rc::new(g.plot_idx.clone()),
        times: g.display_times.clone(),
        display: g.display.clone(),
        win: g.win,
        chip: g.chip,
        max,
        left: f.left,
        right: f.right,
        bottom: f.bottom,
    };
    let v = c.view as usize;
    let zoomable = g.navigable();
    let (hd, hw, hm) = (h.clone(), h.clone(), h.clone());
    let view = c.view;
    root.on_drag(move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| {
        let h = &hd;
        if h.cols.is_empty() {
            return;
        }
        match d.phase {
            Phase::Down => {
                let mods = tx.mods();
                let kind = if mods.alt || d.y > h.bottom + 2.0 {
                    GestureKind::Pan
                } else if mods.shift || s.chart.measure_armed {
                    GestureKind::Measure
                } else {
                    GestureKind::Zoom
                };
                let at = h.display_at(d.x);
                s.chart.gesture[v] = Some(PlotGesture {
                    kind,
                    from: at,
                    to: at,
                    moved: false,
                    brush_at_press: s.chart.brush[v],
                    range_at_press: h.win,
                });
            }
            Phase::Move => {
                let Some(mut gs) = s.chart.gesture[v] else {
                    return;
                };
                if !gs.moved && d.dx.abs() < SLOP {
                    return;
                }
                gs.moved = true;
                match gs.kind {
                    GestureKind::Zoom | GestureKind::Measure => gs.to = h.display_at(d.x),
                    GestureKind::Pan => {
                        // Any window pans, the chip's own too: the history runs on either side.
                        if zoomable {
                            let shift = -(d.dx as f64) * h.ms_per_pt(gs.range_at_press);
                            let next = pan_win(h.first(), h.last(), gs.range_at_press, shift);
                            s.chart.set_zoom(h.view, h.chip, next);
                        }
                    }
                }
                s.chart.gesture[v] = Some(gs);
                s.chart.hover[v] = None;
                s.chart.hover_dot[v] = None;
            }
            Phase::Up => {
                let Some(gs) = s.chart.gesture[v].take() else {
                    return;
                };
                // The plot re-lays out at the new range: land it, don't fly the dots there.
                tx.instant();
                if d.cancelled() {
                    if gs.kind == GestureKind::Pan {
                        s.chart.brush[v] = gs.brush_at_press;
                    }
                    return;
                }
                if !gs.moved {
                    // A click: put away a measurement.
                    s.chart.measure[v] = None;
                    s.chart.measure_armed = false;
                    return;
                }
                match gs.kind {
                    GestureKind::Zoom => {
                        if zoomable && gs.from != gs.to {
                            let next = select(&h.times, gs.from, gs.to);
                            s.chart.set_zoom(h.view, h.chip, next);
                        }
                    }
                    GestureKind::Measure => {
                        s.chart.measure[v] = (gs.from != gs.to).then_some((gs.from, gs.to));
                        s.chart.measure_armed = false;
                    }
                    GestureKind::Pan => {
                        // Back to hovering the column under the pointer (same window length,
                        // same columns).
                        if d.x >= h.left - 4.0 && d.x <= h.right + 4.0 {
                            s.chart.hover[v] = Some(nearest(&h.cols, d.x));
                            // The card follows the pointer, which the drag held still.
                            s.chart.tip_pos.set([d.x, d.y]);
                        }
                    }
                }
            }
        }
    })
    .on_wheel(move |s: &mut App, e: rune::WheelEvent, tx: &mut Tx| {
        let h = &hw;
        if !zoomable || h.cols.is_empty() {
            return false;
        }
        // Shift + a notched wheel is a horizontal scroll on platforms that don't turn it.
        let (dx, dy) = if e.mods.shift && e.dx == 0.0 {
            (e.dy, 0.0)
        } else {
            (e.dx, e.dy)
        };
        let horizontal = dx.abs() > dy.abs();
        let acc = &mut s.chart.wheel_acc[v];
        if horizontal {
            // The window follows the fingers point for point (time, not data points).
            h.pan(s, -(dx as f64) * h.ms_per_pt(h.win), Some(e.x));
            tx.instant();
            return true;
        }
        // The compact card sits in the page's scroll view: a plain wheel scrolls the page, and
        // ⌘ / Ctrl + wheel zooms. The full-window chart has nothing to scroll, so a plain wheel
        // zooms there.
        if view == View::Compact && !(e.mods.cmd || e.mods.ctrl) {
            return false;
        }
        let steps = if e.precise {
            acc[1] += dy;
            let n = (acc[1] / WHEEL_STEP).trunc();
            acc[1] -= n * WHEEL_STEP;
            n
        } else {
            dy.signum()
        };
        if steps != 0.0 {
            // Wheel away from you (content down) zooms in, as on TradingView.
            h.zoom(s, e.x, ZOOM_STEP.powf(steps as f64));
            tx.instant();
        }
        true
    })
    .on_magnify(move |s: &mut App, e: rune::MagnifyEvent, tx: &mut Tx| {
        let h = &hm;
        if !zoomable || h.cols.is_empty() {
            return false;
        }
        let acc = &mut s.chart.wheel_acc[v];
        acc[2] += e.delta;
        let n = (acc[2] / PINCH_STEP).trunc();
        if n != 0.0 {
            acc[2] -= n * PINCH_STEP;
            h.zoom(s, e.x, ZOOM_STEP.powf(n as f64));
            tx.instant();
        }
        true
    })
    .on_key(move |s: &mut App, ev: &KeyEvent, tx: &mut Tx| key(s, view, ev, tx))
}

/// The plot's keys: `+` / `-` zoom, the arrows pan (Shift: by half a window), `0` resets.
pub fn key(app: &mut App, view: View, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
        return false;
    }
    enum Act {
        Zoom(f64),
        Pan(f64),
        Reset,
    }
    let act = match &ev.key {
        Key::Text(t) => match t.as_str() {
            "+" | "=" => Act::Zoom(0.5),
            "-" | "\u{2212}" | "_" => Act::Zoom(2.0),
            "0" => Act::Reset,
            _ => return false,
        },
        Key::Left => Act::Pan(-1.0),
        Key::Right => Act::Pan(1.0),
        _ => return false,
    };
    let v = view as usize;
    let Some(s) = app.snapshot() else {
        return false;
    };
    let g = geo(&app.chart, s, view);
    if !g.navigable() {
        return false;
    }
    match act {
        Act::Reset => {
            if !g.zoomed {
                return false;
            }
            app.chart.reset_zoom(view);
        }
        Act::Zoom(f) => {
            // Around the hovered column, else the middle of the window.
            let t = &g.display_times;
            let (first, last) = (t[0], t[t.len() - 1]);
            let anchor = app.chart.hover[v]
                .and_then(|i| g.plot.get(i))
                .map(|p| p.sort_ms)
                .unwrap_or((g.win.lo + g.win.hi) / 2.0);
            let next = zoom_about(first, last, g.win, anchor, f);
            app.chart.set_zoom(view, g.chip, next);
        }
        Act::Pan(dir) => {
            let t = &g.display_times;
            let next = pan_win(
                t[0],
                t[t.len() - 1],
                g.win,
                g.win.span() * if ev.mods.shift { 0.5 } else { 0.1 } * dir,
            );
            if next.same(&g.win) {
                return false;
            }
            app.chart.set_zoom(view, g.chip, next);
        }
    }
    app.chart.hover[v] = None;
    tx.instant();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: f64 = data::DAY_MS;

    fn w(lo: f64, hi: f64) -> Win {
        Win {
            lo: lo * D,
            hi: hi * D,
        }
    }

    #[test]
    fn zoom_keeps_the_anchor_in_place() {
        let full = w(0.0, 400.0);
        let r = zoom_about(0.0, 400.0 * D, full, 100.0 * D, 0.5);
        // The anchor was a quarter of the way in, and still is.
        assert_eq!(r, w(50.0, 250.0));
        // Zooming back out around the same moment returns to the full window.
        assert_eq!(zoom_about(0.0, 400.0 * D, r, 100.0 * D, 2.0), full);
    }

    #[test]
    fn zoom_stops_at_the_limits() {
        let full = w(0.0, 400.0);
        assert_eq!(zoom_about(0.0, 400.0 * D, full, 200.0 * D, 4.0), full);
        // Never under the minimum window (a week).
        let mut r = full;
        for _ in 0..40 {
            r = zoom_about(0.0, 400.0 * D, r, 200.0 * D, 0.5);
        }
        assert!((r.span() - data::BRUSH_MIN_WINDOW_MS).abs() < 1.0);
        assert!(r.lo <= 200.0 * D && 200.0 * D <= r.hi);
        // At an edge it stays inside the history.
        let edge = zoom_about(0.0, 400.0 * D, w(300.0, 400.0), 400.0 * D, 1.5);
        assert_eq!(edge, w(250.0, 400.0));
    }

    #[test]
    fn pan_clamps_to_the_data() {
        let (a, b) = (0.0, 99.0 * D);
        let r = w(10.0, 30.0);
        assert_eq!(pan_win(a, b, r, 5.0 * D), w(15.0, 35.0));
        assert_eq!(pan_win(a, b, r, -50.0 * D), w(0.0, 20.0));
        assert_eq!(pan_win(a, b, r, 500.0 * D), w(79.0, 99.0));
    }

    #[test]
    fn select_orders_and_applies_the_minimum_window() {
        let t: Vec<f64> = (0..401).map(|i| i as f64 * D).collect();
        assert_eq!(select(&t, 300, 120), w(120.0, 300.0));
        let tiny = select(&t, 200, 201);
        assert!(tiny.span() >= data::BRUSH_MIN_WINDOW_MS - 1.0);
    }

    #[test]
    fn nearest_column() {
        let cols = [10.0, 20.0, 40.0];
        assert_eq!(nearest(&cols, 0.0), 0);
        assert_eq!(nearest(&cols, 14.0), 0);
        assert_eq!(nearest(&cols, 16.0), 1);
        assert_eq!(nearest(&cols, 31.0), 2);
        assert_eq!(nearest(&cols, 99.0), 2);
    }
}
