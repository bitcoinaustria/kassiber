//! The range navigator under the plot (an owner-requested redesign of Recharts' Brush, in the
//! TradingView / Apple Stocks manner): the whole book history as a quiet mini balance line,
//! and the visible range as a window on it, clear inside and dimmed outside, with a slim
//! limiter on each edge.
//!
//! The window is the plot's zoom (`ChartUi::brush`, indices into `Geo::display`, which spans
//! the whole history): dragging a limiter shrinks or widens it and dragging inside slides it,
//! the plot following every frame; a plot zoom or pan moves the window; the range chips put it
//! at their period, and Reset zoom or a double-click returns it there.

use rune::{Cursor, Cx, El, KeyEvent, Phase, Shape, Tx, canvas, el, row, text};

use super::data::{BRUSH_MIN_WINDOW_MS, DAY_MS, Win};
use super::interact::pan_win;
use super::{BrushDrag, BrushPart, Geo, View, strings};
use crate::app::App;
use crate::format::Day;
use crate::theme::Tk;

/// The navigator's height, the same in the card and the full-window chart.
pub const H: f32 = 36.0;
/// Time slices of the mini line (about one per two points of track at full width).
pub const MINI_BUCKETS: usize = 480;
/// Half the limiter's grab width.
const GRAB: f32 = 8.0;
/// The drawn limiter grip.
const GRIP_W: f32 = 6.0;
const GRIP_H: f32 = 18.0;
const RADIUS: f32 = 6.0;

/// The track's horizontal geometry: the history's first to last moment over `[x0, x1]`,
/// aligned with the plot's frame.
#[derive(Clone, Copy, Debug)]
struct Track {
    x0: f32,
    x1: f32,
    t0: f64,
    t1: f64,
}

impl Track {
    fn x(&self, t: f64) -> f32 {
        if self.t1 <= self.t0 {
            return self.x0;
        }
        self.x0 + ((t - self.t0) / (self.t1 - self.t0)) as f32 * (self.x1 - self.x0)
    }
    fn t(&self, x: f32) -> f64 {
        self.t0 + ((x - self.x0) / (self.x1 - self.x0).max(1.0)) as f64 * (self.t1 - self.t0)
    }
    fn ms_per_pt(&self) -> f64 {
        (self.t1 - self.t0) / (self.x1 - self.x0).max(1.0) as f64
    }
}

/// The window with its start limiter at `t`: inside the history, never closer to the end
/// than the minimum window.
pub fn with_start(first: f64, w: Win, t: f64) -> Win {
    let min = BRUSH_MIN_WINDOW_MS.min(w.hi - first);
    Win {
        lo: t.clamp(first, w.hi - min),
        hi: w.hi,
    }
}

/// The window with its end limiter at `t`.
pub fn with_end(last: f64, w: Win, t: f64) -> Win {
    let min = BRUSH_MIN_WINDOW_MS.min(last - w.lo);
    Win {
        lo: w.lo,
        hi: t.clamp(w.lo + min, last),
    }
}

/// The window moved so its middle sits at `t`, its length kept.
pub fn center_on(first: f64, last: f64, w: Win, t: f64) -> Win {
    pan_win(first, last, w, t - (w.lo + w.hi) / 2.0)
}

/// What one navigator handler needs from the built geometry.
#[derive(Clone, Copy)]
struct Nav {
    view: View,
    track: Track,
    win: Win,
    chip: Win,
}

impl Nav {
    fn apply(&self, s: &mut App, next: Win) {
        s.chart.set_zoom(self.view, self.chip, next);
    }
    /// The drag handler for one part; `Track` is a press on the dimmed history, which centres
    /// the window there and then slides it.
    fn drag(self, part: BrushPart) -> impl Fn(&mut App, &mut rune::Drag, &mut Tx) + 'static {
        move |s: &mut App, d: &mut rune::Drag, tx: &mut Tx| {
            let nav = self;
            let v = nav.view as usize;
            let tr = nav.track;
            if tr.t1 <= tr.t0 || tr.t1.is_nan() || tr.t0.is_nan() {
                return;
            }
            // The plot and the limiters follow the hand at once, never springing after it.
            tx.instant();
            match d.phase {
                Phase::Down => {
                    let (at, part) = if part == BrushPart::Track {
                        let next = center_on(tr.t0, tr.t1, nav.win, tr.t(d.x));
                        nav.apply(s, next);
                        (next, BrushPart::Window)
                    } else {
                        (nav.win, part)
                    };
                    s.chart.drag[v] = Some(BrushDrag { part, at_press: at });
                    s.chart.hover[v] = None;
                }
                Phase::Move | Phase::Up => {
                    let Some(drag) = s.chart.drag[v] else { return };
                    let at = drag.at_press;
                    if d.phase == Phase::Up && d.cancelled() {
                        s.chart.drag[v] = None;
                        nav.apply(s, at);
                        return;
                    }
                    let dt = d.dx as f64 * tr.ms_per_pt();
                    let next = match drag.part {
                        BrushPart::Start => with_start(tr.t0, at, at.lo + dt),
                        BrushPart::End => with_end(tr.t1, at, at.hi + dt),
                        _ => pan_win(tr.t0, tr.t1, at, dt),
                    };
                    s.chart.drag[v] = if d.phase == Phase::Up {
                        None
                    } else {
                        Some(drag)
                    };
                    nav.apply(s, next);
                }
            }
        }
    }
    /// A limiter moved by `days` (Left / Right one, Page Up / Down ten, Shift a month), or to
    /// a moment (Home / End).
    fn step_edge(&self, s: &mut App, which: BrushPart, days: f64, to: Option<f64>) {
        let (w, tr) = (self.win, self.track);
        let next = match which {
            BrushPart::Start => with_start(tr.t0, w, to.unwrap_or(w.lo + days * DAY_MS)),
            _ => with_end(tr.t1, w, to.unwrap_or(w.hi + days * DAY_MS)),
        };
        self.apply(s, next);
    }
    /// The window slid by a fraction of its length.
    fn step_window(&self, s: &mut App, frac: f64) {
        self.apply(
            s,
            pan_win(
                self.track.t0,
                self.track.t1,
                self.win,
                self.win.span() * frac,
            ),
        );
    }
    /// Shift + an arrow, which the slider keys pass over (Shift navigates): a month for a
    /// limiter, half the length for the window.
    fn shift_key(
        self,
        which: BrushPart,
    ) -> impl Fn(&mut App, &KeyEvent, &mut Tx) -> bool + 'static {
        move |s: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            let dir = match ev.key {
                rune::Key::Left | rune::Key::Down => -1.0,
                rune::Key::Right | rune::Key::Up => 1.0,
                _ => return false,
            };
            if !ev.mods.shift || ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
                return false;
            }
            match which {
                BrushPart::Start | BrushPart::End => self.step_edge(s, which, dir * 30.0, None),
                _ => self.step_window(s, dir * 0.5),
            }
            tx.instant();
            true
        }
    }
}

/// The navigator: track, mini line, window and limiters.
pub fn brush(app: &App, cx: &mut Cx<App>, tk: &Tk, g: &Geo, view: View) -> El {
    let v = view as usize;
    let key = if view == View::Expanded {
        "chart-brush-x"
    } else {
        "chart-brush"
    };
    let w = cx.layout_size(key).map(|s| s[0]).unwrap_or(900.0);
    // Edge to edge with the plot's frame (`Frame::left` / `right`).
    let margin = if view == View::Expanded { 72.0 } else { 68.0 };
    let times = &g.display_times;
    let (x0, x1) = (margin, (w - margin).max(margin + 2.0));
    let track = Track {
        x0,
        x1,
        t0: times[0],
        t1: times[times.len() - 1],
    };
    let tw = x1 - x0;
    let win = g.win;
    let drag = app.chart.drag[v];
    let (sx, ex) = (track.x(win.lo), track.x(win.hi));
    let nav = Nav {
        view,
        track,
        win,
        chip: g.chip,
    };
    let days_from_start = |t: f64| ((t - track.t0) / DAY_MS).round();
    let total_days = days_from_start(track.t1);

    // The track's drawing, in track coordinates: the mini balance area and line over the whole
    // history, the year (or month) scale, then the dimming outside the window and its edges.
    // The scale's labels sit in the top 12 pt; the line keeps below them.
    let (pad_top, pad_bottom) = (13.0, 4.0);
    let (lo, hi) = g
        .mini
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
            (a.min(p.1), b.max(p.1))
        });
    let span = if hi > lo { hi - lo } else { 1.0 };
    let yv = |v: f64| {
        if hi > lo {
            pad_top + (H - pad_top - pad_bottom) * (1.0 - ((v - lo) / span) as f32)
        } else {
            (H + pad_top) / 2.0
        }
    };
    let lx = |t: f64| track.x(t) - x0;
    let mut shape = Shape::new();
    let mut line: Vec<[f32; 2]> = Vec::with_capacity(g.mini.len() * 2);
    let area = tk.chart_btc.alpha(0.12);
    for (i, &(t, b)) in g.mini.iter().enumerate() {
        let (x, y) = (lx(t), yv(b));
        if let Some(prev) = line.last().copied() {
            line.push([x, prev[1]]);
            if x > prev[0] {
                shape = shape.fill(
                    vec![[prev[0], prev[1]], [x, prev[1]], [x, H], [prev[0], H]],
                    area,
                );
            }
        }
        line.push([x, y]);
        if i + 1 == g.mini.len() && x < tw {
            shape = shape.fill(vec![[x, y], [tw, y], [tw, H], [x, H]], area);
            line.push([tw, y]);
        }
    }
    shape = shape.stroke_with(
        line,
        false,
        1.0,
        rune::LineCap::Butt,
        tk.chart_btc.alpha(0.55),
    );
    let mut labels: Vec<El> = Vec::new();
    let scale_ink = tk.muted_fg.alpha(0.75);
    let (_, marks) = super::axes::time_ticks(track.t0, track.t1, tw, 12);
    for (t, label) in marks {
        let x = lx(t);
        shape = shape.line(
            [x, 0.0],
            [x, H],
            1.0,
            rune::LineCap::Butt,
            tk.border.alpha(tk.border.a * 0.6),
        );
        labels.push(
            text(label)
                .font(tk.px(9.0, 500.0).lh(10.0))
                .color(scale_ink)
                .absolute(x + 3.0, 2.0)
                .inert(),
        );
    }
    let dim = tk.card.alpha(0.62);
    let (ls, le) = (sx - x0, ex - x0);
    if ls > 0.0 {
        shape = shape.fill(vec![[0.0, 0.0], [ls, 0.0], [ls, H], [0.0, H]], dim);
    }
    if le < tw {
        shape = shape.fill(vec![[le, 0.0], [tw, 0.0], [tw, H], [le, H]], dim);
    }
    let edge = tk.chart_btc;
    shape = shape
        .line([ls, 0.0], [ls, H], 1.5, rune::LineCap::Butt, edge)
        .line([le, 0.0], [le, H], 1.5, rune::LineCap::Butt, edge);
    // A hairline along the window's top and bottom closes it into a frame without weight.
    shape = shape
        .line(
            [ls, 0.5],
            [le, 0.5],
            1.0,
            rune::LineCap::Butt,
            edge.alpha(0.35),
        )
        .line(
            [ls, H - 0.5],
            [le, H - 0.5],
            1.0,
            rune::LineCap::Butt,
            edge.alpha(0.35),
        );
    let track_el = el()
        .absolute(x0, 0.0)
        .size(tw, H)
        .radius(RADIUS)
        .fill(tk.muted.alpha(0.28))
        .border(1.0, tk.border)
        .clip_rounded(RADIUS)
        .inert()
        .child(canvas(shape).absolute(0.0, 0.0).size(tw, H))
        .children(labels);

    // A window edge's day (the balance points sit at the end of their day).
    let date = |t: f64| Day::from_epoch_days((t / DAY_MS).floor() as i64).long_ui();
    let reset = move |s: &mut App, tx: &mut Tx| {
        s.chart.reset_zoom(view);
        tx.instant();
    };
    let held = drag.map(|d| d.part);
    // A limiter: a slim grip on the window's edge in a generous grab area.
    // Wide, the grips sit on the window's edges and the body lies between their grab areas.
    // Narrower than that, the body covers the window (at least 10 pt, so it can still be slid)
    // and the grips move just outside it; the edges stay drawn where the window is.
    let narrow = ex - sx < 2.0 * GRAB + 12.0;
    let (body_l, body_r, gs, ge, grip_s, grip_e) = if narrow {
        let (mid, half) = ((sx + ex) / 2.0, ((ex - sx) / 2.0).max(5.0));
        (
            mid - half,
            mid + half,
            mid - half - GRAB,
            mid + half + GRAB,
            2.0 * GRAB - GRIP_W - 1.0,
            1.0,
        )
    } else {
        (
            sx + GRAB,
            ex - GRAB,
            sx,
            ex,
            GRAB - GRIP_W / 2.0,
            GRAB - GRIP_W / 2.0,
        )
    };
    let limiter = |x: f32, which: BrushPart| {
        let start = which == BrushPart::Start;
        let at = if start { win.lo } else { win.hi };
        let active = held == Some(which);
        el().key(format!(
            "brush-traveller-{v}-{}",
            if start { "start" } else { "end" }
        ))
        .absolute(x - GRAB, 0.0)
        .size(2.0 * GRAB, H)
        .cursor(Cursor::ResizeColumn)
        .role("slider")
        .label(if start {
            strings::brush_start()
        } else {
            strings::brush_end()
        })
        .a11y_value(date(at))
        .range_value(days_from_start(at), 0.0, total_days)
        .range_step(1.0)
        .focusable(true)
        .on_step(move |s: &mut App, delta: i32, tx: &mut Tx| {
            nav.step_edge(s, which, delta as f64, None);
            tx.instant();
        })
        .on_step_limit(move |s: &mut App, max: bool, tx: &mut Tx| {
            nav.step_edge(
                s,
                which,
                0.0,
                Some(if max { nav.track.t1 } else { nav.track.t0 }),
            );
            tx.instant();
        })
        .on_key(nav.shift_key(which))
        .on_drag(nav.drag(which))
        .on_double_press_tx(reset)
        .child(
            el().absolute(if start { grip_s } else { grip_e }, (H - GRIP_H) / 2.0)
                .size(GRIP_W, GRIP_H)
                .radius(GRIP_W / 2.0)
                .fill(if active { edge } else { tk.background })
                .border(1.0, edge)
                .inert(),
        )
    };
    let window_held = held == Some(BrushPart::Window);
    let window = el()
        .key(format!("brush-window-{v}"))
        .absolute(body_l, 0.0)
        .size((body_r - body_l).max(0.0), H)
        .cursor(if window_held {
            Cursor::Grabbing
        } else {
            Cursor::Grab
        })
        .role("slider")
        .label(strings::brush_window())
        .a11y_value(format!("{} \u{2013} {}", date(win.lo), date(win.hi)))
        .range_value(
            days_from_start(win.lo),
            0.0,
            (total_days - days_from_start(win.hi) + days_from_start(win.lo)).max(0.0),
        )
        .range_step(1.0)
        .focusable(true)
        // Left / Right slide a tenth of the window (Page Up / Down a whole one), Shift half;
        // Home / End to either end of the history.
        .on_step(move |s: &mut App, delta: i32, tx: &mut Tx| {
            nav.step_window(s, delta as f64 * 0.1);
            tx.instant();
        })
        .on_step_limit(move |s: &mut App, max: bool, tx: &mut Tx| {
            let w = nav.win;
            nav.apply(
                s,
                pan_win(
                    nav.track.t0,
                    nav.track.t1,
                    w,
                    if max {
                        nav.track.t1 - w.hi
                    } else {
                        nav.track.t0 - w.lo
                    },
                ),
            );
            tx.instant();
        })
        .on_key(nav.shift_key(BrushPart::Window))
        .on_drag(nav.drag(BrushPart::Window))
        .on_double_press_tx(reset);
    // The rest of the track: a press there brings the window to it.
    let rest = el()
        .key(format!("brush-track-{v}"))
        .absolute(x0, 0.0)
        .size(tw, H)
        .cursor(Cursor::Pointer)
        .a11y_hidden()
        .on_drag(nav.drag(BrushPart::Track))
        .on_double_press_tx(reset);
    let mut root = el()
        .key(key)
        .w_full()
        .h(H)
        // Range changes land at once inside the navigator, whatever commit carries them.
        .instant_changes()
        .help(crate::i18n::t("overview:treasury.dragHint"))
        .on_hover_change(move |s: &mut App, on, _tx: &mut Tx| s.chart.brush_hover[v] = on)
        .child(track_el)
        .child(rest)
        .child(window)
        .child(limiter(gs, BrushPart::Start))
        .child(limiter(ge, BrushPart::End));
    // The window's dates while it is hovered or held: small pills beside each edge, outside
    // the window where there is room, else just inside it.
    if app.chart.brush_hover[v] || drag.is_some() {
        let st = tk.t2xs(500.0).features("'tnum' 1");
        let pill = |label: String, x: f32| {
            row()
                .absolute(x, (H - 16.0) / 2.0)
                .h(16.0)
                .px(6.0)
                .center_y()
                .radius(8.0)
                .fill(tk.popover)
                .border(1.0, tk.border)
                .inert()
                .child(text(label).font(st).color(tk.fg))
        };
        let (la, lb) = (date(win.lo), date(win.hi));
        let pw = |l: &str| l.chars().count() as f32 * st.size * 0.58 + 14.0;
        let (wa, wb) = (pw(&la), pw(&lb));
        let ax = if sx - x0 >= wa + GRAB + 2.0 {
            sx - GRAB - wa
        } else {
            sx + GRAB + 2.0
        };
        let bx = if x1 - ex >= wb + GRAB + 2.0 {
            ex + GRAB
        } else {
            ex - GRAB - 2.0 - wb
        };
        if bx > ax + wa + 4.0 || bx + wb < ax - 4.0 {
            root = root.child(pill(la, ax)).child(pill(lb, bx));
        } else {
            // No room for two: one pill with the span, over the window's middle.
            let both = format!("{la} \u{2013} {lb}");
            let w = pw(&both);
            root = root.child(pill(
                both,
                ((sx + ex) / 2.0 - w / 2.0).clamp(x0, (x1 - w).max(x0)),
            ));
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: f64 = DAY_MS;

    fn w(lo: f64, hi: f64) -> Win {
        Win {
            lo: lo * D,
            hi: hi * D,
        }
    }

    #[test]
    fn track_maps_time_to_points_and_back() {
        let t = Track {
            x0: 68.0,
            x1: 668.0,
            t0: 0.0,
            t1: 600.0 * D,
        };
        assert_eq!(t.x(0.0), 68.0);
        assert_eq!(t.x(300.0 * D), 368.0);
        assert!((t.t(368.0) - 300.0 * D).abs() < 1.0);
        assert!((t.ms_per_pt() - D).abs() < 1e-6);
    }

    #[test]
    fn limiters_never_cross_and_keep_a_week() {
        let r = w(100.0, 200.0);
        // Dragging the start past the end stops a week before it, and the end a week after.
        assert_eq!(with_start(0.0, r, 390.0 * D), w(193.0, 200.0));
        assert_eq!(with_end(400.0 * D, r, 0.0), w(100.0, 107.0));
        // Within bounds they land exactly where they are dropped, not on a point.
        assert_eq!(with_start(0.0, r, 50.4 * D), w(50.4, 200.0));
        assert_eq!(with_end(400.0 * D, r, 260.6 * D), w(100.0, 260.6));
        // And never past the history's ends.
        assert_eq!(with_start(0.0, r, -9.0 * D), w(0.0, 200.0));
        assert_eq!(with_end(400.0 * D, r, 900.0 * D), w(100.0, 400.0));
        // A history shorter than a week keeps the limiters apart at its ends.
        let short = with_start(0.0, w(0.0, 3.0), 3.0 * D);
        assert_eq!(short, w(0.0, 3.0));
    }

    #[test]
    fn the_window_slides_within_the_history() {
        let (a, b) = (0.0, 400.0 * D);
        let r = w(100.0, 200.0);
        assert_eq!(pan_win(a, b, r, 50.0 * D), w(150.0, 250.0));
        assert_eq!(pan_win(a, b, r, -500.0 * D), w(0.0, 100.0));
        assert_eq!(pan_win(a, b, r, 500.0 * D), w(300.0, 400.0));
        assert_eq!(center_on(a, b, r, 300.0 * D), w(250.0, 350.0));
        assert_eq!(center_on(a, b, r, 390.0 * D), w(300.0, 400.0));
    }
}
