//! `InvestigationGraph` over `EvidenceGraph` (components/evidence-graph): the layered graph of
//! transactions and outputs under a pan/zoom camera, its zoom/fit/focus controls, the minimap
//! and the help line.
//!
//! Edges, the dot grid and the dashed subject ring are one canvas; nodes are elements (buttons
//! named by their title) for the nodes in view. The camera follows the web's rules: a readable
//! initial view until the user moves it, a selection outside the view brought into it.

use rune::{Color, Cx, El, Key, LineCap, Path, Shape, Tx, canvas, col, el, row, text};

use super::kit::ca;
use super::model::{self, Camera, Edge, Node, Pos, Sel};
use super::st;
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// The graph's camera once the user (or a selection) moved it, the viewport it was measured
/// in and a drag in progress.
#[derive(Default, Clone, Debug)]
pub struct GraphState {
    pub cam: Option<Camera>,
    pub vp: Option<[f32; 2]>,
    pub drag_from: Option<Camera>,
}

pub const HEIGHT: f32 = 520.0;
const KEY: &str = "ca-graph";

/// The graph's viewport: as measured, else estimated from the window (the page less the nav,
/// the panel's inset and border, the page gutters and the frame border).
pub fn viewport(app: &App) -> [f32; 2] {
    if let Some(v) = app.provenance.chain.graph.vp {
        return v;
    }
    let tk = app.tk();
    let win = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let nav = if app.prefs.nav_collapsed {
        tk.r(3.0)
    } else {
        tk.r(16.0)
    };
    [
        (win - nav - tk.r(0.375) - 2.0 - 2.0 * tk.page_pad() - 2.0).max(320.0),
        HEIGHT,
    ]
}

/// The camera the graph shows now: the user's, or the readable initial view.
pub fn camera(app: &App) -> Option<Camera> {
    let s = &app.provenance.chain;
    let r = s.result.as_ref()?;
    let vp = viewport(app);
    Some(s.graph.cam.unwrap_or_else(|| {
        model::initial_camera(&r.layout, vp, &r.nodes, &r.edges, s.sel.as_ref())
    }))
}

/// A new selection: the view is the user's from now on; a selection outside it is focused.
pub fn on_select(app: &mut App, sel: &Sel) {
    let Some(cam) = camera(app) else { return };
    let s = &app.provenance.chain;
    let Some(r) = s.result.clone() else { return };
    let vp = viewport(app);
    let changed = s.sel.as_ref() != Some(sel);
    let mut next = cam;
    if changed
        && let Some(p) = model::selection_position(&r.layout, &r.edges, Some(sel))
        && !model::selection_in_view(p, cam, vp)
    {
        next = model::focus_camera(p, vp);
    }
    st(app).graph.cam = Some(next);
}

fn set_cam(app: &mut App, c: Camera) {
    st(app).graph.cam = Some(c);
}

fn zoom(app: &mut App, factor: f32, point: Option<Pos>) {
    let Some(c) = camera(app) else { return };
    let vp = viewport(app);
    let p = point.unwrap_or(Pos {
        x: vp[0] / 2.0,
        y: vp[1] / 2.0,
    });
    set_cam(app, model::zoom_camera(c, factor, p));
}

fn fit(app: &mut App) {
    let s = &app.provenance.chain;
    let Some(r) = s.result.clone() else { return };
    let vp = viewport(app);
    set_cam(app, model::fit_camera(&r.layout, vp));
}

/// `F` / the crosshair: the readable view of the selection (or the initial anchor).
fn focus(app: &mut App) {
    let s = &app.provenance.chain;
    let Some(r) = s.result.clone() else { return };
    let vp = viewport(app);
    let c = model::initial_camera(&r.layout, vp, &r.nodes, &r.edges, s.sel.as_ref());
    set_cam(app, c);
}

fn pan(app: &mut App, dx: f32, dy: f32) {
    if let Some(c) = camera(app) {
        set_cam(
            app,
            Camera {
                x: c.x + dx,
                y: c.y + dy,
                ..c
            },
        );
    }
}

/// `edgeColor` of an appearance.
fn edge_color(tk: &Tk, appearance: &str) -> Color {
    match appearance {
        "danger" => tk.destructive,
        "muted" => ca::muted(),
        "warning" => ca::hypothesis(),
        "secondary" => ca::custody(),
        _ => ca::physical(),
    }
}

/// An edge's appearance and dash (`InvestigationGraph`).
fn edge_style(e: &Edge) -> (&'static str, Option<(f32, f32)>) {
    let status = e.status.as_deref().unwrap_or("");
    let appearance = match (status, e.kind.as_str()) {
        ("conflicting", _) => "danger",
        ("stale", _) => "muted",
        (_, "hypothesis") => "warning",
        (_, "custody") => "secondary",
        _ => "default",
    };
    let dash = if status == "stale" || status == "conflicting" {
        Some((2.0, 5.0))
    } else {
        match e.kind.as_str() {
            "custody" => Some((9.0, 3.0)),
            "hypothesis" => Some((5.0, 5.0)),
            _ => None,
        }
    };
    (appearance, dash)
}

/// The edge's cubic in layout units (`M a+96 C a+165, b-165, b-100`), flattened.
fn edge_points(a: Pos, b: Pos) -> Vec<[f32; 2]> {
    let p0 = [a.x + 96.0, a.y];
    let c1 = [a.x + 165.0, a.y];
    let c2 = [b.x - 165.0, b.y];
    let p1 = [b.x - 100.0, b.y];
    (0..=20)
        .map(|i| {
            let t = i as f32 / 20.0;
            let u = 1.0 - t;
            let f = |k: usize| {
                u * u * u * p0[k]
                    + 3.0 * u * u * t * c1[k]
                    + 3.0 * u * t * t * c2[k]
                    + t * t * t * p1[k]
            };
            [f(0), f(1)]
        })
        .collect()
}

fn to_screen(p: [f32; 2], c: Camera) -> [f32; 2] {
    [p[0] * c.scale + c.x, p[1] * c.scale + c.y]
}

/// Splits a polyline into dashes of `on`/`off` points.
fn dashed(points: &[[f32; 2]], on: f32, off: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out = vec![];
    let mut cur: Vec<[f32; 2]> = vec![];
    let mut drawing = true;
    let mut left = on;
    for w in points.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        while seg > 0.0 {
            let step = seg.min(left);
            let t = step / seg;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if drawing {
                if cur.is_empty() {
                    cur.push(a);
                }
                cur.push(p);
            }
            seg -= step;
            left -= step;
            a = p;
            if left <= 1e-4 {
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

fn dist_to_polyline(p: [f32; 2], pts: &[[f32; 2]]) -> f32 {
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx * dx + dy * dy;
            let t = if len > 0.0 {
                (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (x, y) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
            (x * x + y * y).sqrt()
        })
        .fold(f32::MAX, f32::min)
}

/// The edge under a click at `p` (viewport points), within the 15-unit hit stroke.
fn pick_edge(app: &App, p: [f32; 2]) -> Option<String> {
    let s = &app.provenance.chain;
    let r = s.result.as_ref()?;
    let c = camera(app)?;
    let mut best: Option<(f32, String)> = None;
    for e in &r.edges {
        let (Some(a), Some(b)) = (
            r.layout.positions.get(&e.source),
            r.layout.positions.get(&e.target),
        ) else {
            continue;
        };
        let pts: Vec<[f32; 2]> = edge_points(*a, *b)
            .into_iter()
            .map(|q| to_screen(q, c))
            .collect();
        let d = dist_to_polyline(p, &pts);
        if d <= (7.5 * c.scale).max(3.0) && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
            best = Some((d, e.id.clone()));
        }
    }
    best.map(|(_, id)| id)
}

fn circle(cx: f32, cy: f32, r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect()
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0);
    Path::new()
        .move_to([x + r, y])
        .line_to([x + w - r, y])
        .quad_to([x + w, y], [x + w, y + r])
        .line_to([x + w, y + h - r])
        .quad_to([x + w, y + h], [x + w - r, y + h])
        .line_to([x + r, y + h])
        .quad_to([x, y + h], [x, y + h - r])
        .line_to([x, y + r])
        .quad_to([x, y], [x + r, y])
        .close()
}

/// The rounded rectangle's outline as points (for dashing).
fn rect_outline(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<[f32; 2]> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pts = vec![];
    let corners = [
        (x + w - r, y + r, -90.0f32),
        (x + w - r, y + h - r, 0.0),
        (x + r, y + h - r, 90.0),
        (x + r, y + r, 180.0),
    ];
    for (cx, cy, start) in corners {
        for i in 0..=6 {
            let a = (start + i as f32 * 15.0).to_radians();
            pts.push([cx + r * a.cos(), cy + r * a.sin()]);
        }
    }
    pts.push(pts[0]);
    pts
}

/// The graph container: canvas, nodes, controls, minimap and help.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk) -> El {
    let s = &app.provenance.chain;
    let Some(r) = s.result.clone() else {
        return el();
    };
    let w = cx
        .layout_size(KEY)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or_else(|| viewport(app)[0]);
    let vp = [w, HEIGHT];
    let c = s.graph.cam.unwrap_or_else(|| {
        model::initial_camera(&r.layout, vp, &r.nodes, &r.edges, s.sel.as_ref())
    });
    let hide = app.prefs.hide_sensitive();
    let highlighted: std::collections::HashSet<&str> =
        s.highlighted.iter().map(String::as_str).collect();
    let subject = r.subject_node_id();
    let fg = tk.fg;
    let bg = tk.background.mix(tk.muted, 0.15);

    // The dot grid (screen space, 24 apart).
    let mut shape = Shape::new();
    let grid = fg.alpha(0.13);
    let mut y = 1.0;
    while y < HEIGHT {
        let mut x = 1.0;
        while x < w {
            shape = shape.fill(circle(x, y, 0.7, 6), grid);
            x += 24.0;
        }
        y += 24.0;
    }
    // Edges.
    let margin = 300.0 * c.scale.max(0.2);
    for e in &r.edges {
        let (Some(a), Some(b)) = (
            r.layout.positions.get(&e.source),
            r.layout.positions.get(&e.target),
        ) else {
            continue;
        };
        let (sa, sb) = (to_screen([a.x, a.y], c), to_screen([b.x, b.y], c));
        if (sa[0].max(sb[0]) < -margin)
            || (sa[0].min(sb[0]) > w + margin)
            || (sa[1].max(sb[1]) < -margin)
            || (sa[1].min(sb[1]) > HEIGHT + margin)
        {
            continue;
        }
        let (appearance, dash) = edge_style(e);
        let active = s.sel.as_ref().is_some_and(|x| x.edge && x.id == e.id)
            || (highlighted.contains(e.source.as_str()) && highlighted.contains(e.target.as_str()));
        let impaired = appearance == "muted" || appearance == "danger";
        let color = edge_color(tk, appearance).alpha(if active {
            1.0
        } else if impaired {
            0.7
        } else {
            0.5
        });
        let width = (if active { 3.0 } else { 1.5 }) * c.scale;
        let pts: Vec<[f32; 2]> = edge_points(*a, *b)
            .into_iter()
            .map(|q| to_screen(q, c))
            .collect();
        match dash {
            None => {
                shape = shape.stroke_with(pts.clone(), false, width.max(0.3), LineCap::Butt, color)
            }
            Some((on, off)) => {
                for d in dashed(&pts, on * c.scale, off * c.scale) {
                    shape = shape.stroke_with(d, false, width.max(0.3), LineCap::Butt, color);
                }
            }
        }
        // The arrow marker (5 × stroke width, oriented along the curve's end).
        let n = pts.len();
        let (end, prev) = (pts[n - 1], pts[n - 2]);
        let (dx, dy) = (end[0] - prev[0], end[1] - prev[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / len, dy / len);
        let size = 5.0 * width.max(0.3);
        let tip = [end[0] + ux * size * 0.1, end[1] + uy * size * 0.1];
        let base = [tip[0] - ux * size * 0.9, tip[1] - uy * size * 0.9];
        let half = size * 0.4;
        shape = shape.fill(
            vec![
                tip,
                [base[0] - uy * half, base[1] + ux * half],
                [base[0] + uy * half, base[1] - ux * half],
            ],
            edge_color(tk, appearance),
        );
    }
    // Node bodies are elements; dashed rings (the subject, missing/stale evidence) are drawn here.
    let in_view = |p: Pos| model::node_in_view(p, c, vp);
    for n in &r.nodes {
        let Some(p) = r
            .layout
            .positions
            .get(&n.id)
            .copied()
            .filter(|p| in_view(*p))
        else {
            continue;
        };
        let capsule = n.kind == "output";
        let [x, y] = to_screen([p.x, p.y], c);
        if subject.as_deref() == Some(n.id.as_str()) {
            let outline = rect_outline(
                x - 105.0 * c.scale,
                y - 35.0 * c.scale,
                210.0 * c.scale,
                70.0 * c.scale,
                (if capsule { 32.0 } else { 12.0 }) * c.scale,
            );
            for d in dashed(&outline, 3.0 * c.scale, 3.0 * c.scale) {
                shape = shape.stroke_with(d, false, 1.5 * c.scale, LineCap::Butt, ca::owned());
            }
        }
        let status = n.status.as_deref().unwrap_or("");
        let selected = s.sel.as_ref().is_some_and(|x| !x.edge && x.id == n.id);
        if !selected && matches!(status, "missing" | "stale" | "conflicting") {
            let ink = if status == "missing" {
                fg.alpha(0.18)
            } else {
                ca::hypothesis()
            };
            let outline = rect_outline(
                x - 99.0 * c.scale,
                y - 29.0 * c.scale,
                198.0 * c.scale,
                58.0 * c.scale,
                (if capsule { 26.0 } else { 8.0 }) * c.scale,
            );
            for d in dashed(&outline, 4.0 * c.scale, 3.0 * c.scale) {
                shape = shape.stroke_with(d, false, c.scale.max(0.5), LineCap::Butt, ink);
            }
        }
    }
    let _ = rounded_rect;

    let mut container = el()
        .key(KEY)
        .w_full()
        .h(HEIGHT)
        .fill(bg)
        .clip_rounded(0.0)
        .role("group")
        .label(t("chainAnalysis:graph.label"))
        .help(t("chainAnalysis:graph.help"))
        .focusable(true)
        .on_layout(move |a: &mut App, rect: [f32; 4]| {
            let next = [rect[2], rect[3]];
            let estimate = viewport(a);
            let g = &mut a.provenance.chain.graph;
            if let (prev, Some(cam)) = (g.vp.unwrap_or(estimate), g.cam)
                && prev != next
            {
                g.cam = Some(model::resize_camera(cam, prev, next));
            }
            g.vp = Some(next);
        })
        .on_drag(
            |a: &mut App, d: &mut rune::Drag, _tx: &mut Tx| match d.phase {
                rune::Phase::Down => {
                    let c = camera(a);
                    st(a).graph.drag_from = c;
                }
                rune::Phase::Up => {
                    let from = st(a).graph.drag_from.take();
                    if d.dx.abs() < 3.0 && d.dy.abs() < 3.0 {
                        if let Some(id) = pick_edge(a, [d.x, d.y]) {
                            super::select(a, Sel { edge: true, id });
                        }
                    } else if let Some(f) = from {
                        set_cam(
                            a,
                            Camera {
                                x: f.x + d.dx,
                                y: f.y + d.dy,
                                ..f
                            },
                        );
                    }
                }
                _ => {
                    if let Some(f) = a.provenance.chain.graph.drag_from
                        && (d.dx.abs() >= 3.0 || d.dy.abs() >= 3.0)
                    {
                        set_cam(
                            a,
                            Camera {
                                x: f.x + d.dx,
                                y: f.y + d.dy,
                                ..f
                            },
                        );
                    }
                }
            },
        )
        .on_wheel(|a: &mut App, e: rune::WheelEvent, _tx: &mut Tx| {
            let delta = (-e.dy).clamp(-200.0, 200.0);
            zoom(a, (-delta * 0.003).exp(), Some(Pos { x: e.x, y: e.y }));
            true
        })
        .on_magnify(|a: &mut App, e: rune::MagnifyEvent, _tx: &mut Tx| {
            zoom(a, (1.0 + e.delta).max(0.1), Some(Pos { x: e.x, y: e.y }));
            true
        })
        .on_key(|a: &mut App, ev: &rune::KeyEvent, tx: &mut Tx| {
            if tx.focused() != Some(KEY) || ev.mods.cmd || ev.mods.ctrl {
                return false;
            }
            match &ev.key {
                Key::Text(s) if s == "+" || s == "=" => zoom(a, 1.3, None),
                Key::Text(s) if s == "-" => zoom(a, 1.0 / 1.3, None),
                Key::Text(s) if s == "0" => fit(a),
                Key::Home => fit(a),
                Key::Text(s) if s.eq_ignore_ascii_case("f") => focus(a),
                Key::Left => pan(a, 60.0, 0.0),
                Key::Right => pan(a, -60.0, 0.0),
                Key::Up => pan(a, 0.0, 60.0),
                Key::Down => pan(a, 0.0, -60.0),
                _ => return false,
            }
            true
        })
        .child(canvas(shape).absolute(0.0, 0.0).size(w, HEIGHT).inert());

    // Edges in view, as buttons at their midpoints (the web's focusable edge paths).
    for e in &r.edges {
        let (Some(a), Some(b)) = (
            r.layout.positions.get(&e.source),
            r.layout.positions.get(&e.target),
        ) else {
            continue;
        };
        let pts = edge_points(*a, *b);
        let [mx, my] = to_screen(pts[pts.len() / 2], c);
        if mx < 0.0 || mx > w || my < 0.0 || my > HEIGHT {
            continue;
        }
        let status = e
            .status
            .as_deref()
            .map(super::inspector::translated_status)
            .unwrap_or_default();
        let label = [
            if e.label.is_empty() {
                e.kind.clone()
            } else {
                e.label.clone()
            },
            status,
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let id = e.id.clone();
        container = container.child(
            el().key(format!("ca-edge-{}", e.id))
                .absolute(mx - 5.0, my - 5.0)
                .size(10.0, 10.0)
                .radius(5.0)
                .role("button")
                .label(label.clone())
                .help(format!("{label} · {}", e.evidence_level))
                .focusable(true)
                .cursor(rune::Cursor::Pointer)
                .on_press(move |a: &mut App| {
                    super::select(
                        a,
                        Sel {
                            edge: true,
                            id: id.clone(),
                        },
                    )
                }),
        );
    }
    // Nodes in (and just around) the view, as buttons.
    let mut visible = 0usize;
    for n in &r.nodes {
        let Some(p) = r.layout.positions.get(&n.id).copied() else {
            continue;
        };
        let [sx, sy] = to_screen([p.x, p.y], c);
        if sx + 99.0 * c.scale < -40.0
            || sx - 99.0 * c.scale > w + 40.0
            || sy + 29.0 * c.scale < -40.0
            || sy - 29.0 * c.scale > HEIGHT + 40.0
        {
            continue;
        }
        if in_view(p) {
            visible += 1;
        }
        container = container.child(node_el(
            tk,
            n,
            p,
            c,
            s.sel.as_ref(),
            highlighted.contains(n.id.as_str()),
            hide,
        ));
    }
    container = container
        .child(controls(tk, c, s.sel.is_some()))
        .child(minimap(
            tk,
            &r.layout,
            c,
            vp,
            s.sel.as_ref(),
            visible,
            r.nodes.len(),
        ))
        .child(
            row()
                .pin(Some(12.0), None, None, Some(12.0))
                .px(8.0)
                .py(5.0)
                .radius(8.0)
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.card.alpha(0.95))
                .inert()
                .a11y_hidden()
                .child(
                    text(t("chainAnalysis:graph.help"))
                        .font(tk.px(10.0, 400.0))
                        .color(tk.muted_fg),
                ),
        );
    container
}

/// One node card or capsule at its camera position (`GraphMarks`).
fn node_el(
    tk: &Tk,
    n: &Node,
    p: Pos,
    c: Camera,
    sel: Option<&Sel>,
    highlighted: bool,
    hide: bool,
) -> El {
    let s = c.scale;
    let [x, y] = to_screen([p.x, p.y], c);
    let capsule = n.kind == "output";
    let selected = sel.is_some_and(|x| !x.edge && x.id == n.id);
    let status = n.status.as_deref().unwrap_or("");
    let amount = model::format_amount(n.amount_msat.as_deref(), n.asset.as_deref());
    let shown_amount = if hide {
        crate::theme::MASK.to_owned()
    } else {
        amount.clone()
    };
    let label = model::node_caption(n, 26);
    let label = if label.chars().count() > 26 {
        format!("{}…", label.chars().take(25).collect::<String>())
    } else {
        label
    };
    let detail = if capsule {
        shown_amount.clone()
    } else {
        format!("{} · {}", n.chain, n.network)
    };
    let detail = if detail.chars().count() > 30 {
        format!("{}…", detail.chars().take(29).collect::<String>())
    } else {
        detail
    };
    let title = format!("{}\n{}\n{}", n.label, n.identity(), shown_amount);
    let (fill, stroke, sw) = if selected {
        (tk.card.mix(ca::physical(), 0.12), ca::physical(), 2.5)
    } else if highlighted {
        (tk.card, ca::physical(), 2.0)
    } else if matches!(status, "stale" | "conflicting" | "missing") {
        (tk.card, Color::TRANSPARENT, 1.0)
    } else {
        (tk.card, tk.fg.alpha(0.18), 1.0)
    };
    let id = n.id.clone();
    let mut e = el()
        .key(format!("ca-node-{}", n.id))
        .absolute(x - 99.0 * s - sw * s / 2.0, y - 29.0 * s - sw * s / 2.0)
        .size((198.0 + sw) * s, (58.0 + sw) * s)
        .radius((if capsule { 26.0 } else { 8.0 }) * s + sw * s / 2.0)
        .fill(fill)
        .border(sw * s, stroke)
        .role("button")
        .label(title.clone())
        .help(title)
        .checked(selected)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |a: &mut App| super::select(a, Sel::node(id.clone())));
    if s >= 0.2 {
        let pad = sw * s / 2.0;
        let dot = if n.wallet_ids.is_empty() {
            ca::muted()
        } else {
            ca::owned()
        };
        e = e
            .child(crate::ui::dot(6.0 * s, dot).absolute(pad + 15.0 * s, pad + 19.0 * s))
            .child(
                text(label)
                    .font(tk.px(11.0 * s, 500.0).mono().lh(14.0 * s))
                    .color(tk.fg)
                    .absolute(pad + 28.0 * s, pad + 15.0 * s)
                    .a11y_hidden(),
            )
            .child(
                text(detail)
                    .font(tk.px(10.0 * s, 400.0).mono().lh(13.0 * s))
                    .color(tk.muted_fg)
                    .absolute(pad + 28.0 * s, pad + 34.0 * s)
                    .a11y_hidden(),
            );
    }
    e
}

/// `.ca-graph-controls`: zoom in/out, Fit graph, focus, and the zoom percentage.
fn controls(tk: &Tk, c: Camera, selected: bool) -> El {
    let btn = |k: &str, glyph: &'static rune::Icon, label: String| {
        row()
            .key(k.to_owned())
            .size(28.0, 28.0)
            .center()
            .radius(tk.r(0.375))
            .hover(tk.accent)
            .role("button")
            .label(label.clone())
            .help(label)
            .focusable(true)
            .child(ic(glyph, 16.0, tk.fg))
    };
    let fit_label = t("chainAnalysis:graph.fit");
    let focus_label = t(if selected {
        "chainAnalysis:graph.focusSelection"
    } else {
        "chainAnalysis:graph.readableView"
    });
    row()
        .absolute(12.0, 12.0)
        .padding(4.0)
        .gap(4.0)
        .center_y()
        .radius(8.0)
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.card.alpha(0.95))
        .child(
            btn("ca-zoom-in", &I::PLUS, t("chainAnalysis:graph.zoomIn"))
                .on_press(|a: &mut App| zoom(a, 1.3, None)),
        )
        .child(
            btn("ca-zoom-out", &I::MINUS, t("chainAnalysis:graph.zoomOut"))
                .on_press(|a: &mut App| zoom(a, 1.0 / 1.3, None)),
        )
        .child(
            row()
                .key("ca-fit")
                .h(28.0)
                .px(6.0)
                .gap(4.0)
                .center()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(fit_label.clone())
                .help(fit_label.clone())
                .focusable(true)
                .on_press(fit)
                .child(ic(&I::FOCUS, 16.0, tk.fg))
                .child(text(fit_label).font(tk.px(11.0, 500.0)).color(tk.fg)),
        )
        .child(btn("ca-focus", &I::CROSSHAIR, focus_label).on_press(focus))
        .child(
            row().min_w(40.0).justify_center().child(
                text(format!("{}%", (c.scale * 100.0).round() as i64))
                    .font(tk.txs(400.0).mono())
                    .color(tk.fg),
            ),
        )
}

/// `.ca-graph-overview`: every node as a dot, the viewport as a rectangle; a click centres the
/// view there, Enter fits.
fn minimap(
    tk: &Tk,
    l: &model::Layout,
    c: Camera,
    vp: [f32; 2],
    sel: Option<&Sel>,
    visible: usize,
    total: usize,
) -> El {
    let o = model::overview(l, c, vp, 164.0, 60.0);
    let mut shape = Shape::new();
    for id in &l.order {
        let Some(p) = l.positions.get(id) else {
            continue;
        };
        let chosen = sel.is_some_and(|s| s.id == *id);
        shape = shape.fill(
            circle(
                o.x + p.x * o.scale,
                o.y + p.y * o.scale,
                if chosen { 2.5 } else { 1.5 },
                8,
            ),
            if chosen { ca::owned() } else { ca::physical() },
        );
    }
    let [rx, ry, rw, rh] = o.rect;
    shape = shape.fill(
        vec![[rx, ry], [rx + rw, ry], [rx + rw, ry + rh], [rx, ry + rh]],
        ca::physical().alpha(0.12),
    );
    shape = shape.stroke_with(
        vec![
            [rx, ry],
            [rx + rw, ry],
            [rx + rw, ry + rh],
            [rx, ry + rh],
            [rx, ry],
        ],
        false,
        1.0,
        LineCap::Butt,
        ca::physical(),
    );
    let (scale, ox, oy, lw, lh) = (o.scale, o.x, o.y, l.width, l.height);
    col()
        .pin(None, None, Some(12.0), Some(12.0))
        .w(164.0)
        .radius(8.0)
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.card.alpha(0.95))
        .clip_rounded(8.0)
        .child(
            el().key("ca-overview")
                .size(162.0, 60.0)
                .role("button")
                .label(t("chainAnalysis:graph.overview"))
                .help(t("chainAnalysis:graph.overview"))
                .focusable(true)
                .cursor(rune::Cursor::Crosshair)
                .on_drag(move |a: &mut App, d: &mut rune::Drag, _tx: &mut Tx| {
                    if d.phase == rune::Phase::Up {
                        let pos = Pos {
                            x: ((d.x * 164.0 / 162.0 - ox) / scale).clamp(0.0, lw),
                            y: ((d.y - oy) / scale).clamp(0.0, lh),
                        };
                        let vp = viewport(a);
                        set_cam(a, model::focus_camera(pos, vp));
                    }
                })
                .on_key(|a: &mut App, ev: &rune::KeyEvent, tx: &mut Tx| {
                    if tx.focused() == Some("ca-overview")
                        && (matches!(ev.key, Key::Enter)
                            || matches!(&ev.key, Key::Text(s) if s == " "))
                    {
                        fit(a);
                        return true;
                    }
                    false
                })
                .child(canvas(shape).absolute(0.0, 0.0).size(164.0, 60.0).inert()),
        )
        .child(
            row().w_full().px(6.0).pb(6.0).justify_center().child(
                text(tr(
                    "chainAnalysis:graph.inView",
                    &[
                        ("visible", &visible.to_string()),
                        ("total", &total.to_string()),
                    ],
                ))
                .font(tk.px(10.0, 400.0).features("'tnum' 1"))
                .color(tk.muted_fg),
            ),
        )
}
