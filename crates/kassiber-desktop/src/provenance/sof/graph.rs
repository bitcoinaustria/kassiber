//! `SourceFundsFlowGraph` over `EvidenceGraph`: the report's canonical `simplified_flow` on a
//! pannable, zoomable canvas (dotted grid, curved reviewed edges with arrowheads, node cards, the
//! subject ring, the control strip, the overview minimap and the help line), plus the selection
//! strip under it. Presentation only: the adapter (`model::provenance_graph`) draws exactly the
//! report's explicit edges.

use rune::{
    Color, Cx, El, Key, KeyEvent, LineCap, Shape, Tx, WheelEvent, canvas, col, el, row, text,
};

use super::model::{self, GNode, Graph, Layout};
use super::{GraphView, kit};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};
use crate::ui::ic;

const HEIGHT: f32 = 360.0;
const PHYSICAL: &str = "#168ca6";
const CUSTODY: &str = "#9674da";
const HYPOTHESIS: &str = "#c08325";
const OWNED: &str = "#259e79";
const MUTED: &str = "#9297a0";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cam {
    pub x: f32,
    pub y: f32,
    pub s: f32,
}

fn fit(l: &Layout, w: f32, h: f32) -> Cam {
    let s = (w / l.width * 0.9)
        .min(h / l.height * 0.9)
        .clamp(0.001, 1.0);
    Cam {
        s,
        x: (w - l.width * s) / 2.0,
        y: (h - l.height * s) / 2.0,
    }
}

fn focus(p: (f32, f32), w: f32, h: f32) -> Cam {
    let s = ((w - 40.0) / 198.0)
        .min((h - 100.0) / 58.0)
        .clamp(0.025, 1.0);
    Cam {
        s,
        x: w / 2.0 - p.0 * s,
        y: h / 2.0 - p.1 * s,
    }
}

/// `initialGraphCamera` (with `initialView` fit for ≤ 10 nodes, readable otherwise).
fn initial(
    g: &Graph,
    l: &Layout,
    w: f32,
    h: f32,
    selected: Option<(f32, f32)>,
    fit_view: bool,
) -> Cam {
    if fit_view {
        return fit(l, w, h);
    }
    if let Some(p) = selected {
        return focus(p, w, h);
    }
    let fitted = fit(l, w, h);
    if fitted.s >= 0.9 || g.nodes.is_empty() {
        return fitted;
    }
    let anchor = g
        .subject
        .as_ref()
        .and_then(|id| l.pos.get(id))
        .copied()
        .or_else(|| {
            l.pos
                .values()
                .copied()
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
        });
    let Some(a) = anchor else { return fitted };
    let c = focus(a, w, h);
    Cam {
        x: (w / 2.0).min((123.0 * c.s).max(w * 0.3)) - a.0 * c.s,
        ..c
    }
}

fn zoom(c: Cam, factor: f32, px: f32, py: f32) -> Cam {
    let s = (c.s * factor).clamp(0.001, 4.0);
    let r = s / c.s;
    Cam {
        s,
        x: px - (px - c.x) * r,
        y: py - (py - c.y) * r,
    }
}

fn in_view(p: (f32, f32), c: Cam, w: f32, h: f32) -> bool {
    let (x, y) = (p.0 * c.s + c.x, p.1 * c.s + c.y);
    x + 99.0 * c.s > 0.0 && x - 99.0 * c.s < w && y + 29.0 * c.s > 0.0 && y - 29.0 * c.s < h
}

fn selection_in_view(p: (f32, f32), c: Cam, w: f32, h: f32) -> bool {
    let (x, y) = (p.0 * c.s + c.x, p.1 * c.s + c.y);
    c.s >= 0.75
        && x - 99.0 * c.s >= 12.0
        && x + 99.0 * c.s <= w - 12.0
        && y - 29.0 * c.s >= 60.0
        && y + 29.0 * c.s <= h - 80.0
}

fn view_mut<'a>(a: &'a mut App, key: &'static str) -> Option<&'a mut GraphView> {
    a.provenance
        .sof
        .case
        .as_mut()
        .map(|c| c.graph.entry(key).or_default())
}

fn set_cam(a: &mut App, key: &'static str, c: Cam) {
    if let Some(v) = view_mut(a, key) {
        v.touched = true;
        v.x = c.x;
        v.y = c.y;
        v.scale = c.s;
    }
}

/// The adapter with the route's labels (`journey.roles.*`, `flow.*`, `linkType.*`).
pub fn build(flow: &serde_json::Value) -> Graph {
    let role = |r: &str| {
        let k = format!("sourceFunds:journey.roles.{r}");
        if crate::i18n::has(&k) {
            t(&k)
        } else {
            model::pretty(r)
        }
    };
    let dir = |d: &str| {
        t(if d == "inbound" {
            "sourceFunds:flow.incoming"
        } else {
            "sourceFunds:flow.outgoing"
        })
    };
    let link = |ty: &str| {
        if ty.is_empty() {
            return String::new();
        }
        super::link_type_label(ty)
    };
    model::provenance_graph(flow, &role, &dir, &link)
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    } else {
        s.to_owned()
    }
}

/// Samples a cubic Bézier into a polyline.
fn cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], n: usize) -> Vec<[f32; 2]> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            [
                a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
                a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
            ]
        })
        .collect()
}

/// Splits a polyline into dashes of `on` / `off` length.
fn dashes(pts: &[[f32; 2]], on: f32, off: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out = vec![];
    let mut cur: Vec<[f32; 2]> = vec![];
    let mut drawing = true;
    let mut left = on;
    for w in pts.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while seg > 0.0 {
            let step = seg.min(left);
            let f = step / seg;
            let m = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
            if drawing {
                if cur.is_empty() {
                    cur.push(a);
                }
                cur.push(m);
            }
            seg -= step;
            left -= step;
            a = m;
            if left <= 0.0 {
                if drawing && cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                drawing = !drawing;
                left = if drawing { on } else { off };
            }
        }
    }
    if drawing && cur.len() > 1 {
        out.push(cur);
    }
    out
}

/// A rounded rectangle outline as a closed polyline.
fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<[f32; 2]> {
    let mut p = vec![];
    let corners = [
        (x + w - r, y + r, -90.0_f32),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, 90.0),
        (x + r, y + r, 180.0),
    ];
    for (cx, cy, start) in corners {
        for i in 0..=6 {
            let a = (start + 15.0 * i as f32).to_radians();
            p.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
    }
    p.push(p[0]);
    p
}

/// The graph section (`FlowPathPreview` body): canvas and the selection strip under it.
pub fn view(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    key: &'static str,
    flow: &serde_json::Value,
    open_tx: bool,
) -> El {
    let g = build(flow);
    let l = model::layout(&g);
    let gv = app
        .provenance
        .sof
        .case
        .as_ref()
        .and_then(|c| c.graph.get(key))
        .cloned()
        .unwrap_or_default();
    let box_key = format!("sof-graph-{key}");
    let w = cx
        .layout_size(&box_key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(1000.0);
    let h = HEIGHT;
    let sel_pos = gv.selected.as_ref().and_then(|(node, id)| {
        if *node {
            l.pos.get(id).copied()
        } else {
            g.edges.iter().find(|e| &e.id == id).and_then(|e| {
                Some((
                    (l.pos.get(&e.source)?.0 + l.pos.get(&e.target)?.0) / 2.0,
                    (l.pos.get(&e.source)?.1 + l.pos.get(&e.target)?.1) / 2.0,
                ))
            })
        }
    });
    let fit_view = g.nodes.len() <= 10;
    let cam = if gv.touched {
        Cam {
            x: gv.x,
            y: gv.y,
            s: gv.scale,
        }
    } else {
        initial(&g, &l, w, h, None, fit_view)
    };

    let physical = kit::h(PHYSICAL);
    let mut shape = Shape::new().view_box(w, h);
    // The dotted grid (`pattern` 24 × 24, r 0.7, foreground at 13 %).
    let grid = tk.fg.alpha(0.13);
    let mut gy = 1.0;
    while gy < h {
        let mut gx = 1.0;
        while gx < w {
            shape = shape.fill(
                vec![
                    [gx - 0.7, gy],
                    [gx, gy - 0.7],
                    [gx + 0.7, gy],
                    [gx, gy + 0.7],
                ],
                grid,
            );
            gx += 24.0;
        }
        gy += 24.0;
    }
    let to_s = |p: [f32; 2]| [p[0] * cam.s + cam.x, p[1] * cam.s + cam.y];
    let mut hotspots: Vec<El> = vec![];
    for e in &g.edges {
        let (Some(a), Some(b)) = (l.pos.get(&e.source), l.pos.get(&e.target)) else {
            continue;
        };
        let pts: Vec<[f32; 2]> = cubic(
            [a.0 + 96.0, a.1],
            [a.0 + 165.0, a.1],
            [b.0 - 165.0, b.1],
            [b.0 - 100.0, b.1],
            32,
        )
        .into_iter()
        .map(to_s)
        .collect();
        let active = gv
            .selected
            .as_ref()
            .is_some_and(|(n, id)| !*n && *id == e.id);
        let color = kit::h(if e.warning { HYPOTHESIS } else { CUSTODY }).alpha(if active {
            1.0
        } else {
            0.5
        });
        let width = if active { 3.0 } else { 1.5 } * cam.s;
        if e.warning {
            for d in dashes(&pts, 5.0 * cam.s, 5.0 * cam.s) {
                shape = shape.stroke_with(d, false, width, LineCap::Butt, color);
            }
        } else {
            shape = shape.stroke(pts.clone(), width, color);
        }
        // Arrowhead (`marker` 5 × stroke, tip one unit past the end).
        let end = to_s([b.0 - 100.0, b.1]);
        let k = 0.75 * cam.s * if active { 2.0 } else { 1.0 };
        let head = kit::h(if e.warning { HYPOTHESIS } else { CUSTODY });
        shape = shape.fill(
            vec![
                [end[0] + k, end[1]],
                [end[0] - 8.0 * k, end[1] - 4.0 * k],
                [end[0] - 8.0 * k, end[1] + 4.0 * k],
            ],
            head,
        );
        // An edge is a button (`role="button"`, its label); a hotspot at its midpoint.
        let mid = pts[pts.len() / 2];
        let id = e.id.clone();
        let (gw, gh) = (w, h);
        hotspots.push(
            el().key(format!("sof-g-{key}-e-{}", e.id))
                .absolute(mid[0] - 10.0, mid[1] - 10.0)
                .size(20.0, 20.0)
                .radius(10.0)
                .role("button")
                .label(e.label.clone())
                .help(e.title.clone())
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |a: &mut App| select(a, key, false, &id, cam, gw, gh)),
        );
    }
    // The subject ring (dashed, owned green), under the node cards.
    if let Some(p) = g.subject.as_ref().and_then(|id| l.pos.get(id)) {
        let c = to_s([p.0, p.1]);
        let ring = round_rect(
            c[0] - 105.0 * cam.s,
            c[1] - 35.0 * cam.s,
            210.0 * cam.s,
            70.0 * cam.s,
            12.0 * cam.s,
        );
        for d in dashes(&ring, 3.0 * cam.s, 3.0 * cam.s) {
            shape = shape.stroke_with(d, false, 1.5 * cam.s, LineCap::Butt, kit::h(OWNED));
        }
    }
    let mut layer = el()
        .key(box_key.clone())
        .w_full()
        .h(h)
        .clip_rounded(0.0)
        .fill(mix(tk.background, tk.muted, 0.85));
    let back = el()
        .key(format!("sof-g-{key}-bg"))
        .absolute(0.0, 0.0)
        .size(w, h)
        .child(canvas(shape).size(w, h).a11y_hidden());
    layer = layer.child(back);
    for n in &g.nodes {
        let Some(p) = l.pos.get(&n.id) else { continue };
        let selected = gv
            .selected
            .as_ref()
            .is_some_and(|(node, id)| *node && *id == n.id);
        layer = layer.child(node_card(
            tk,
            key,
            n,
            *p,
            cam,
            w,
            h,
            selected,
            g.subject.as_deref() == Some(n.id.as_str()),
        ));
    }
    for hs in hotspots {
        layer = layer.child(hs);
    }
    // Panning and zooming the camera.
    let (gw, gh) = (w, h);
    layer = layer
        .on_drag(move |a: &mut App, d: &mut rune::Drag, _tx: &mut Tx| {
            let Some(v) = view_mut(a, key) else { return };
            match d.phase {
                rune::Phase::Down => {
                    v.drag = Some((cam.x, cam.y));
                    v.touched = true;
                    v.x = cam.x;
                    v.y = cam.y;
                    v.scale = cam.s;
                }
                rune::Phase::Move => {
                    if let Some((ox, oy)) = v.drag {
                        v.x = ox + d.dx;
                        v.y = oy + d.dy;
                    }
                }
                rune::Phase::Up => v.drag = None,
            }
        })
        .on_wheel(move |a: &mut App, e: WheelEvent, _tx: &mut Tx| {
            let f = (e.dy.clamp(-200.0, 200.0) * 0.003).exp();
            set_cam(a, key, zoom(cam, f, e.x, e.y));
            true
        })
        .role("group")
        .label(t("sourceFunds:flowPath.title"))
        .help(t("chainAnalysis:graph.help"))
        .focusable(true)
        .on_key(move |a: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
            graph_key(a, key, ev, cam, &l_box(gw, gh))
        });
    // The control strip (top-left).
    let ctrl_btn =
        |k: &str, glyph: &'static rune::Icon, label: String, on: Box<dyn Fn(&mut App)>| {
            row()
                .key(format!("sof-g-{key}-{k}"))
                .size(28.0, 28.0)
                .center()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(label.clone())
                .help(label)
                .focusable(true)
                .on_press(move |a: &mut App| on(a))
                .child(ic(glyph, tk.r(1.0), tk.fg))
        };
    let lw = l.width;
    let lh = l.height;
    let fit_cam = fit(&l, w, h);
    let focus_cam = initial(&g, &l, w, h, sel_pos, false);
    let fit_label = t("chainAnalysis:graph.fit");
    let controls = row()
        .absolute(12.0, 12.0)
        .padding(4.0)
        .gap(4.0)
        .center_y()
        .radius(8.0)
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.card.alpha(0.95))
        .child(ctrl_btn(
            "zin",
            &I::PLUS,
            t("chainAnalysis:graph.zoomIn"),
            Box::new(move |a| set_cam(a, key, zoom(cam, 1.3, gw / 2.0, gh / 2.0))),
        ))
        .child(ctrl_btn(
            "zout",
            &I::MINUS,
            t("chainAnalysis:graph.zoomOut"),
            Box::new(move |a| set_cam(a, key, zoom(cam, 1.0 / 1.3, gw / 2.0, gh / 2.0))),
        ))
        .child(
            row()
                .key(format!("sof-g-{key}-fit"))
                .h(28.0)
                .px(6.0)
                .gap(4.0)
                .center_y()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(fit_label.clone())
                .help(fit_label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| set_cam(a, key, fit_cam))
                .child(ic(&I::FOCUS, tk.r(1.0), tk.fg))
                .child(text(fit_label).font(tk.px(11.0, 500.0)).color(tk.fg)),
        )
        .child(ctrl_btn(
            "focus",
            &I::CROSSHAIR,
            t(if gv.selected.is_some() {
                "chainAnalysis:graph.focusSelection"
            } else {
                "chainAnalysis:graph.readableView"
            }),
            Box::new(move |a| set_cam(a, key, focus_cam)),
        ))
        .child(
            text(format!("{}%", (cam.s * 100.0).round() as i64))
                .font(kit::tnum(tk.txs(400.0).mono()))
                .color(tk.fg)
                .min_w(40.0)
                .align(rune::Align::Center),
        );
    // The overview minimap (bottom-right).
    let os = ((164.0 - 12.0) / lw).min((60.0 - 12.0) / lh);
    let (ox, oy) = ((164.0 - lw * os) / 2.0, (60.0 - lh * os) / 2.0);
    let mut mini = Shape::new().view_box(164.0, 60.0);
    for (id, p) in &l.pos {
        let on = gv.selected.as_ref().is_some_and(|(_, s)| s == id);
        let r = if on { 2.5 } else { 1.5 };
        let (cxp, cyp) = (ox + p.0 * os, oy + p.1 * os);
        mini = mini.fill(
            (0..10)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 10.0;
                    [cxp + r * a.cos(), cyp + r * a.sin()]
                })
                .collect::<Vec<_>>(),
            if on { kit::h(OWNED) } else { physical },
        );
    }
    let left = (-cam.x / cam.s).clamp(0.0, lw);
    let top = (-cam.y / cam.s).clamp(0.0, lh);
    let right = ((w - cam.x) / cam.s).clamp(0.0, lw);
    let bottom = ((h - cam.y) / cam.s).clamp(0.0, lh);
    let (vx, vy, vw, vh) = (
        ox + left * os,
        oy + top * os,
        ((right - left) * os).max(2.0),
        ((bottom - top) * os).max(4.0),
    );
    mini = mini
        .fill(
            vec![[vx, vy], [vx + vw, vy], [vx + vw, vy + vh], [vx, vy + vh]],
            physical.alpha(0.12),
        )
        .stroke_with(
            vec![[vx, vy], [vx + vw, vy], [vx + vw, vy + vh], [vx, vy + vh]],
            true,
            1.0,
            LineCap::Butt,
            physical,
        );
    let visible = l.pos.values().filter(|p| in_view(**p, cam, w, h)).count();
    let overview = col()
        .absolute(w - 12.0 - 164.0, h - 60.0 - 80.0)
        .w(164.0)
        .radius(8.0)
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(8.0)
        .fill(tk.card.alpha(0.95))
        .child(
            el().key(format!("sof-g-{key}-mini"))
                .size(162.0, 59.0)
                .role("button")
                .label(t("chainAnalysis:graph.overview"))
                .focusable(true)
                .cursor(rune::Cursor::Crosshair)
                .on_drag(move |a: &mut App, d: &mut rune::Drag, _tx: &mut Tx| {
                    if d.phase == rune::Phase::Down {
                        let px = ((d.x * 164.0 / d.w.max(1.0) - ox) / os).clamp(0.0, lw);
                        let py = ((d.y * 60.0 / d.h.max(1.0) - oy) / os).clamp(0.0, lh);
                        set_cam(a, key, focus((px, py), gw, gh));
                    }
                })
                .on_key(move |a: &mut App, ev: &KeyEvent, _tx: &mut Tx| {
                    if matches!(ev.key, Key::Enter) || ev.key == Key::Text(" ".into()) {
                        set_cam(a, key, fit_cam);
                        return true;
                    }
                    false
                })
                .child(canvas(mini).size(162.0, 59.0).a11y_hidden()),
        )
        .child(
            text(tr(
                "chainAnalysis:graph.inView",
                &[
                    ("visible", &visible.to_string()),
                    ("total", &g.nodes.len().to_string()),
                ],
            ))
            .font(kit::tnum(tk.px(10.0, 400.0)))
            .color(tk.muted_fg)
            .align(rune::Align::Center)
            .w_full()
            .px(6.0)
            .pb(6.0),
        );
    let help = text(t("chainAnalysis:graph.help"))
        .font(tk.px(10.0, 400.0))
        .color(tk.muted_fg)
        .absolute(12.0, h - 10.0 - 13.0)
        .w(w - 24.0)
        .a11y_hidden();
    layer = layer.child(controls).child(overview).child(help);

    let mut out = col().w_full().min_w(0.0).gap(tk.r(0.75)).child(layer);
    // The selection strip.
    if let Some((node, id)) = &gv.selected {
        let (title, tx_id) = if *node {
            match g.nodes.iter().find(|n| &n.id == id) {
                Some(n) => (
                    Some(n.title.clone()),
                    (n.node_type == "transaction" && !n.transaction_id.is_empty())
                        .then(|| n.transaction_id.clone()),
                ),
                None => (None, None),
            }
        } else {
            (
                g.edges
                    .iter()
                    .find(|e| &e.id == id)
                    .map(|e| e.title.clone()),
                None,
            )
        };
        if let Some(title) = title {
            let mut strip = row()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(0.75))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .center_y()
                .justify_between()
                .flex_wrap()
                .radius(tk.r(0.5))
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.muted.alpha(0.2))
                .child(
                    text(title)
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap_anywhere()
                        .grow(1.0)
                        .min_w(0.0),
                );
            if let (Some(txid), true) = (tx_id, open_tx) {
                strip = strip.child(
                    kit::button(
                        tk,
                        format!("sof-g-{key}-open"),
                        Variant::Outline,
                        Size::Sm,
                        None,
                        &t("sourceFunds:workstation.viewTransactionDetails"),
                        true,
                    )
                    .on_press(move |a: &mut App| super::open_tx(a, &txid)),
                );
            }
            out = out.child(strip);
        }
    }
    out
}

/// Graph-box size for the key handler.
fn l_box(w: f32, h: f32) -> (f32, f32) {
    (w, h)
}

/// `+`/`=` zoom in, `-` out, `0`/Home fit, `f` readable view, arrows pan by 60.
fn graph_key(a: &mut App, key: &'static str, ev: &KeyEvent, cam: Cam, (w, h): &(f32, f32)) -> bool {
    let (w, h) = (*w, *h);
    let Some(c) = a.provenance.sof.case.as_ref() else {
        return false;
    };
    let flow = match key {
        "disclose" | "trace" | "brief" => super::report(a).map(|r| r["simplified_flow"].clone()),
        _ => None,
    };
    let _ = c;
    let Some(flow) = flow else { return false };
    let g = build(&flow);
    let l = model::layout(&g);
    let next = match &ev.key {
        Key::Text(s) if s == "+" || s == "=" => zoom(cam, 1.3, w / 2.0, h / 2.0),
        Key::Text(s) if s == "-" => zoom(cam, 1.0 / 1.3, w / 2.0, h / 2.0),
        Key::Text(s) if s == "0" => fit(&l, w, h),
        Key::Home => fit(&l, w, h),
        Key::Text(s) if s.eq_ignore_ascii_case("f") => initial(&g, &l, w, h, None, false),
        Key::Left => Cam {
            x: cam.x + 60.0,
            ..cam
        },
        Key::Right => Cam {
            x: cam.x - 60.0,
            ..cam
        },
        Key::Up => Cam {
            y: cam.y + 60.0,
            ..cam
        },
        Key::Down => Cam {
            y: cam.y - 60.0,
            ..cam
        },
        _ => return false,
    };
    set_cam(a, key, next);
    true
}

/// Selecting a node or edge: a selection off screen brings the camera to it.
fn select(a: &mut App, key: &'static str, node: bool, id: &str, cam: Cam, w: f32, h: f32) {
    let flow = super::report(a)
        .map(|r| r["simplified_flow"].clone())
        .unwrap_or_default();
    let g = build(&flow);
    let l = model::layout(&g);
    let pos = if node {
        l.pos.get(id).copied()
    } else {
        g.edges.iter().find(|e| e.id == id).and_then(|e| {
            Some((
                (l.pos.get(&e.source)?.0 + l.pos.get(&e.target)?.0) / 2.0,
                (l.pos.get(&e.source)?.1 + l.pos.get(&e.target)?.1) / 2.0,
            ))
        })
    };
    let next = match pos {
        Some(p) if !selection_in_view(p, cam, w, h) => focus(p, w, h),
        _ => cam,
    };
    if let Some(v) = view_mut(a, key) {
        v.selected = Some((node, id.to_owned()));
        v.touched = true;
        v.x = next.x;
        v.y = next.y;
        v.scale = next.s;
    }
}

/// `color-mix(in srgb, a p, b)`.
pub fn mix(a: Color, b: Color, p: f32) -> Color {
    Color {
        r: a.r * p + b.r * (1.0 - p),
        g: a.g * p + b.g * (1.0 - p),
        b: a.b * p + b.b * (1.0 - p),
        a: a.a * p + b.a * (1.0 - p),
    }
}

/// A node card (198 × 58, `rx 8`), drawn at the camera's scale about its centre.
#[allow(clippy::too_many_arguments)]
fn node_card(
    tk: &Tk,
    key: &'static str,
    n: &GNode,
    p: (f32, f32),
    cam: Cam,
    w: f32,
    h: f32,
    selected: bool,
    subject: bool,
) -> El {
    let (sx, sy) = (p.0 * cam.s + cam.x, p.1 * cam.s + cam.y);
    let physical = kit::h(PHYSICAL);
    let fill = if selected {
        mix(physical, tk.card, 0.12)
    } else {
        tk.card
    };
    let stroke = if selected {
        physical
    } else {
        tk.fg.alpha(0.18)
    };
    let dot = if n.tone == "positive" {
        kit::h(OWNED)
    } else {
        kit::h(MUTED)
    };
    let id = n.id.clone();
    let mut label = n.title.clone();
    if subject {
        label = format!("{label} · {}", t("sourceFunds:journey.roles.target"));
    }
    el().key(format!("sof-g-{key}-n-{}", n.id))
        .absolute(sx - 99.0, sy - 29.0)
        .size(198.0, 58.0)
        .scale(cam.s)
        .radius(8.0)
        .fill(fill)
        .border(if selected { 2.5 } else { 1.0 }, stroke)
        .hover(if selected { fill } else { tk.card })
        .role("button")
        .label(n.title.clone())
        .help(label)
        .checked(selected)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |a: &mut App| select(a, key, true, &id, cam, w, h))
        .child(
            el().absolute(15.0, 19.0)
                .size(6.0, 6.0)
                .radius(3.0)
                .fill(dot)
                .a11y_hidden(),
        )
        .child(
            text(trunc(&n.label, 26))
                .font(tk.px(14.0, 500.0))
                .color(tk.fg)
                .absolute(28.0, 15.0 - 4.0)
                .a11y_hidden(),
        )
        .child(
            text(trunc(&n.detail, 30))
                .font(kit::tnum(tk.px(13.0, 400.0)))
                .color(tk.muted_fg)
                .absolute(28.0, 34.0 - 3.0)
                .a11y_hidden(),
        )
}
