//! The primary graph view: the web's three.js glass ribbons (graph3d/ribbonLayout.ts,
//! glassScene.ts, kb/glass3d/*) as a 2.5D projection.
//!
//! The layout is the web's exactly (1500 × span virtual canvas, 1 scene unit = 100 px,
//! mempool's band of 100 px). The scene is orthographic and nearly planar (ribbons are
//! 0.07 deep, coins at most 0.5), so it is drawn by rotating every vertex with the same
//! pitch/yaw pivot and projecting orthographically: ribbons as per-segment quads with a
//! translucent tone fill, an attenuation-coloured core, a rim highlight and satin edge lines;
//! coins as shaded boxes (visible faces only, painter-sorted); the collar as a tube loop split
//! into a back half (behind the ribbons) and a front half. What it cannot do: transmission,
//! refraction, clearcoat reflections of the room environment and tone mapping.

use std::collections::HashMap;

use rune::color::hex;
use rune::{Color, LineCap, Shape};

use super::data::Payload;
use super::geometry::{self, Line, Opts};
use super::model::{self, Row, Side};

const CANVAS_W: f64 = 1500.0;
const CANVAS_H: f64 = 600.0;
const COMBINED: f64 = 100.0;
const OUTER_EDGE: f64 = 70.0;
const UNIT: f64 = 0.01;
const ZERO_STUB: f64 = 60.0;
const ZERO_TH: f64 = 4.0;
const SAMPLES: usize = 40;
pub const RIBBON_DEPTH: f64 = 0.07;
const BLOCK_W: f64 = 0.42;
const BLOCK_D: f64 = 0.5;
const MIN_BLOCK_H: f64 = 0.03;
const EDGED: f64 = 0.08;

/// The one angle the piece is seen from (yaw, pitch): the upper left, as in the lab renders,
/// so block tops and inner faces show. The view does not turn (`stage.ts` `DEFAULT_VIEW`).
pub const REST: (f32, f32) = (0.5, 0.3);

/// `GLASS_LINE_LIMIT`: the glass piece keeps mempool's shape only while its legs fit the canvas
/// height at minimum spacing, so a side shows this many legs and folds the rest into one
/// "+N more" strand, sooner than mempool's 250. The list below the drawing keeps every leg.
pub const GLASS_LINE_LIMIT: usize = 60;

/// `glassFoldPart`: the part a folded side's "+N more" strand answers to. Not the row's own id,
/// which the server's overflow row (a list row of its own) shares.
pub fn fold_part(side: Side) -> String {
    format!(
        "{}:glass-fold",
        if side == Side::Input {
            "input"
        } else {
            "output"
        }
    )
}

#[derive(Clone, Debug)]
pub struct Ribbon {
    pub part: String,
    pub estimated: bool,
    pub fee: bool,
    /// A "+N more" leg (the fold, or an overflow row the server sent): no row to bring.
    pub overflow: bool,
    pub thickness: f64,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub part: String,
    pub owned: bool,
    pub x: f64,
    pub y: f64,
    pub h: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub ribbons: Vec<Ribbon>,
    pub blocks: Vec<Block>,
    pub collar_half: f64,
    pub rows: HashMap<String, Row>,
}

impl Scene {
    pub fn legend(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        let mut add = |k: &'static str| {
            if !keys.contains(&k) {
                keys.push(k);
            }
        };
        for r in &self.ribbons {
            add(if r.fee {
                "fee"
            } else if r.estimated {
                "unknown"
            } else {
                "known"
            });
        }
        for b in &self.blocks {
            add(if b.owned { "own" } else { "other" });
        }
        ["known", "unknown", "own", "other", "fee"]
            .into_iter()
            .filter(|k| keys.contains(k))
            .collect()
    }
    /// The camera frame (`createGlassStage`): half extents of the content seen at `REST`, with
    /// the web's minimums and margins, fitted once so the drawing keeps one scale.
    pub fn frame(&self) -> (f64, f64) {
        self.frame_at(REST.0, REST.1)
    }
    /// The frame at a pose.
    pub fn frame_at(&self, yaw: f32, pitch: f32) -> (f64, f64) {
        let rot = Rot::new(yaw, pitch);
        let (mut hw, mut hh) = (4.4f64, 2.5f64);
        let mut probe = |x: f64, y: f64, z: f64| {
            let p = rot.apply([x, y, z]);
            hw = hw.max(p[0].abs() * 1.06);
            hh = hh.max(p[1].abs() * 1.1);
        };
        for r in &self.ribbons {
            for p in r.points.iter().step_by(4).chain(r.points.last()) {
                for dz in [-RIBBON_DEPTH / 2.0, RIBBON_DEPTH / 2.0] {
                    probe(p[0], p[1] + r.thickness / 2.0, dz);
                    probe(p[0], p[1] - r.thickness / 2.0, dz);
                }
            }
        }
        for b in &self.blocks {
            let d = block_depth(b.h) / 2.0;
            for dx in [-1.0, 1.0] {
                for dy in [-1.0, 1.0] {
                    for dz in [-1.0, 1.0] {
                        probe(b.x + dx * BLOCK_W / 2.0, b.y + dy * b.h / 2.0, dz * d);
                    }
                }
            }
        }
        (hw, hh)
    }
    /// A click selects an input or output; the fee and a "+N more" leg have no row of their own.
    pub fn selectable(&self, part: &str) -> bool {
        self.ribbons
            .iter()
            .any(|r| r.part == part && !r.fee && !r.overflow)
    }
    /// The part to light for one lit elsewhere (a list row): itself when drawn, else its side's
    /// "+N more" strand when that side folded (`drawnPart` in TransactionGraph3D.tsx).
    pub fn drawn_part(&self, part: &str) -> String {
        if self.ribbons.iter().any(|r| r.part == part) {
            return part.to_owned();
        }
        let side = if part.starts_with("input:") {
            Some(Side::Input)
        } else if part.starts_with("output:") {
            Some(Side::Output)
        } else {
            None
        };
        match side.map(fold_part) {
            Some(fold) if self.ribbons.iter().any(|r| r.part == fold) => fold,
            _ => part.to_owned(),
        }
    }
}

fn strand_points(l: &Line) -> Vec<[f64; 2]> {
    let start = OUTER_EDGE;
    let end = CANVAS_W / 2.0;
    let cs = (start + 5.0).max(OUTER_EDGE + l.curve_base - l.offset);
    let ce = (cs + 18.0).max(end - l.offset - 10.0);
    let mid = (cs + ce) / 2.0;
    let mut pts = vec![[start, l.outer_y]];
    for s in 0..=SAMPLES {
        let t = s as f64 / SAMPLES as f64;
        pts.push([
            geometry::bezier(cs, mid, mid, ce, t),
            geometry::bezier(l.outer_y, l.outer_y, l.inner_y, l.inner_y, t),
        ]);
    }
    pts.push([end, l.inner_y]);
    pts
}

thread_local! {
    static CACHE: std::cell::RefCell<Option<(usize, bool, std::rc::Rc<Scene>)>> = const { std::cell::RefCell::new(None) };
}

/// The layout for a payload, kept while the same payload is drawn (views re-run on every
/// hover and drag step; the payload is shared by `Rc`, so its address identifies it).
pub fn layout_cached(g: &std::rc::Rc<Payload>, hide: bool) -> std::rc::Rc<Scene> {
    let id = std::rc::Rc::as_ptr(g) as usize;
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((i, h, s)) = c.as_ref()
            && *i == id
            && *h == hide
        {
            return s.clone();
        }
        let s = std::rc::Rc::new(layout(g, hide));
        *c = Some((id, hide, s.clone()));
        s
    })
}

/// `ribbonLayout` with `maxRows = GLASS_LINE_LIMIT`: mempool keeps `lineLimit` legs and folds
/// the rest into one more-leg.
pub fn layout(g: &Payload, hide: bool) -> Scene {
    let lr = model::layout_rows(g, hide, GLASS_LINE_LIMIT + 1);
    // Once a side folds, its only overflow row is the fold: any overflow row the server sent
    // lies past the limit, inside it.
    let folded = [
        lr.layout_inputs.len() < lr.total_inputs.len(),
        lr.layout_destinations.len() < lr.total_destinations.len(),
    ];
    let leg_part = |r: &Row, side: Side| {
        let f = folded[if side == Side::Input { 0 } else { 1 }];
        if f && r.node.overflow {
            fold_part(side)
        } else {
            r.part()
        }
    };
    let total = geometry::total(&lr.total_inputs, &lr.total_destinations, g.is_liquid());
    let span = CANVAS_H
        .max(geometry::minimum_span(
            &lr.layout_inputs,
            total,
            COMBINED,
            ZERO_TH,
        ))
        .max(geometry::minimum_span(
            &lr.layout_destinations,
            total,
            COMBINED,
            ZERO_TH,
        ));
    let o = Opts {
        height: span,
        combined_weight: COMBINED,
        curve_width: CANVAS_W / 2.0 - OUTER_EDGE - 12.0,
        outer_top: 0.0,
        outer_span: span,
        zero_thickness: ZERO_TH,
    };
    let to_scene = |x: f64, y: f64| [(x - CANVAS_W / 2.0) * UNIT, (span / 2.0 - y) * UNIT];
    let mut sc = Scene {
        collar_half: (COMBINED + 0.5) / 2.0 * UNIT,
        ..Default::default()
    };
    // Display rows (with real values) keyed like the drawing, for the hover card.
    for (rows, side) in [(&lr.inputs, Side::Input), (&lr.destinations, Side::Output)] {
        for r in rows {
            sc.rows.insert(leg_part(r, side), r.clone());
        }
    }
    for (rows, side) in [
        (&lr.layout_inputs, Side::Input),
        (&lr.layout_destinations, Side::Output),
    ] {
        for (l, r) in geometry::lines(rows, total, o).iter().zip(rows.iter()) {
            let part = leg_part(r, side);
            let fee = r.side == Side::Fee;
            let outer = if l.zero {
                vec![[OUTER_EDGE, l.outer_y], [OUTER_EDGE + ZERO_STUB, l.outer_y]]
            } else {
                strand_points(l)
            };
            let pts = outer
                .iter()
                .map(|&[x, y]| {
                    if side == Side::Input {
                        to_scene(x, y)
                    } else {
                        to_scene(CANVAS_W - x, y)
                    }
                })
                .collect();
            sc.ribbons.push(Ribbon {
                part: part.clone(),
                estimated: l.estimated,
                fee,
                overflow: r.node.overflow,
                thickness: l.thickness * UNIT,
                points: pts,
            });
            if fee {
                continue;
            }
            let centre = OUTER_EDGE - BLOCK_W / UNIT / 2.0;
            let [x, y] = to_scene(
                if side == Side::Input {
                    centre
                } else {
                    CANVAS_W - centre
                },
                l.outer_y,
            );
            sc.blocks.push(Block {
                part,
                owned: r.node.ownership.as_deref() == Some("owned"),
                x,
                y,
                h: (l.thickness * UNIT).max(MIN_BLOCK_H),
            });
        }
    }
    sc
}

fn block_depth(h: f64) -> f64 {
    BLOCK_D.min(EDGED.max(3.0 * h))
}

/// The pivot rotation: three.js Euler XYZ (pitch about X, then yaw about Y).
#[derive(Clone, Copy, Debug)]
pub struct Rot {
    cy: f64,
    sy: f64,
    cp: f64,
    sp: f64,
}

impl Rot {
    pub fn new(yaw: f32, pitch: f32) -> Rot {
        let (yaw, pitch) = (yaw as f64, pitch as f64);
        Rot {
            cy: yaw.cos(),
            sy: yaw.sin(),
            cp: pitch.cos(),
            sp: pitch.sin(),
        }
    }
    /// Rx(pitch) · Ry(yaw) · p.
    pub fn apply(&self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let x1 = x * self.cy + z * self.sy;
        let z1 = -x * self.sy + z * self.cy;
        [x1, y * self.cp - z1 * self.sp, y * self.sp + z1 * self.cp]
    }
}

/// Screen projection for one canvas.
#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub rot: Rot,
    pub w: f64,
    pub h: f64,
    pub ppu: f64,
}

impl Cam {
    pub fn new(frame: (f64, f64), yaw: f32, pitch: f32, w: f32, h: f32) -> Cam {
        let (w, h) = (w as f64, h as f64);
        let (mut hw, mut hh) = frame;
        if hw / hh < w / h {
            hw = hh * w / h;
        } else {
            hh = hw * h / w;
        }
        let _ = hh;
        Cam {
            rot: Rot::new(yaw, pitch),
            w,
            h,
            ppu: w / (2.0 * hw),
        }
    }
    /// Screen point and depth (larger = nearer).
    pub fn project(&self, p: [f64; 3]) -> ([f32; 2], f64) {
        let q = self.rot.apply(p);
        (
            [
                (self.w / 2.0 + q[0] * self.ppu) as f32,
                (self.h / 2.0 - q[1] * self.ppu) as f32,
            ],
            q[2],
        )
    }
    pub fn normal_z(&self, n: [f64; 3]) -> f64 {
        self.rot.apply(n)[2]
    }
    /// Lambert term for the scene's key light at (-4, 7, 8).
    pub fn light(&self, n: [f64; 3]) -> f64 {
        let q = self.rot.apply(n);
        let l = [-4.0, 7.0, 8.0];
        let len = (16.0f64 + 49.0 + 64.0).sqrt();
        ((q[0] * l[0] + q[1] * l[1] + q[2] * l[2]) / len).max(0.0)
    }
}

/// One glass tone (the material tables in kb/glass3d/materials.ts, approximated as
/// translucent fills).
#[derive(Clone, Copy, Debug)]
struct Tone {
    fill: Color,
    core: Color,
    rim: Color,
}

fn h(s: &str) -> Color {
    hex(s).expect("colour")
}

fn tone(dark: bool, fee: bool, estimated: bool) -> Tone {
    match (dark, fee, estimated) {
        (false, false, false) => Tone {
            fill: h("#d6e6ff").alpha(0.82),
            core: h("#2f7cf6").alpha(0.26),
            rim: h("#ffffff").alpha(0.95),
        },
        (false, false, true) => Tone {
            fill: h("#f5f7fa").alpha(0.86),
            core: h("#b8c2d0").alpha(0.30),
            rim: h("#ffffff").alpha(0.95),
        },
        (false, true, false) => Tone {
            fill: h("#ffe9a8").alpha(0.86),
            core: h("#f59e0b").alpha(0.34),
            rim: h("#fff7d6").alpha(0.95),
        },
        (false, true, true) => Tone {
            fill: h("#fbf3dc").alpha(0.86),
            core: h("#e7c77e").alpha(0.32),
            rim: h("#fffaf0").alpha(0.95),
        },
        (true, false, false) => Tone {
            fill: h("#9cc4ff").mix(h("#1d4ed8"), 0.35).alpha(0.62),
            core: h("#3b82f6").alpha(0.42),
            rim: h("#cfe1ff").alpha(0.75),
        },
        (true, false, true) => Tone {
            fill: h("#cbd3de").mix(h("#334155"), 0.35).alpha(0.46),
            core: h("#94a3b8").alpha(0.28),
            rim: h("#e2e8f0").alpha(0.55),
        },
        (true, true, false) => Tone {
            fill: h("#ffd466").mix(h("#b45309"), 0.35).alpha(0.64),
            core: h("#f59e0b").alpha(0.45),
            rim: h("#fff1c2").alpha(0.75),
        },
        (true, true, true) => Tone {
            fill: h("#e9dcb8").mix(h("#57451f"), 0.35).alpha(0.46),
            core: h("#c9a860").alpha(0.30),
            rim: h("#f6ecd2").alpha(0.55),
        },
    }
}

fn shade(c: Color, light: f64) -> Color {
    // 0.65 ambient (hemisphere + environment) plus the key light, as a mix toward white/black.
    let k = (0.65 + 0.55 * light as f32).clamp(0.0, 1.25);
    if k >= 1.0 {
        c.mix(Color::WHITE.alpha(c.a), (k - 1.0) * 1.2)
    } else {
        c.mix(Color::BLACK.alpha(c.a), (1.0 - k) * 0.9)
    }
}

/// Screen-space pick geometry for one frame.
#[derive(Clone, Debug, Default)]
pub struct Picks {
    /// (part, projected centre polyline, half width in px, selectable).
    pub ribbons: Vec<(String, Vec<[f32; 2]>, f32)>,
    /// (part, visible face polygons).
    pub blocks: Vec<(String, Vec<Vec<[f32; 2]>>)>,
    /// Each ribbon's screen bounding box (min x, min y, max x, max y), same order as `ribbons`.
    pub boxes: Vec<[f32; 4]>,
}

impl Picks {
    /// Fills `boxes` from the ribbons' projected centre lines.
    pub fn with_boxes(mut self) -> Picks {
        self.boxes = self
            .ribbons
            .iter()
            .map(|(_, pts, _)| {
                pts.iter()
                    .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
                        [
                            b[0].min(p[0]),
                            b[1].min(p[1]),
                            b[2].max(p[0]),
                            b[3].max(p[1]),
                        ]
                    })
            })
            .collect();
        self
    }
}

fn dist_seg(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
    (x * x + y * y).sqrt()
}

pub fn dist_polyline(p: [f32; 2], pts: &[[f32; 2]]) -> f32 {
    pts.windows(2)
        .map(|w| dist_seg(p, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

fn inside(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            c = !c;
        }
    }
    c
}

impl Picks {
    /// Coins first, then the ribbon the point is most deeply inside.
    pub fn pick(&self, p: [f32; 2]) -> Option<String> {
        for (part, faces) in self.blocks.iter().rev() {
            if faces.iter().any(|f| inside(p, f)) {
                return Some(part.clone());
            }
        }
        let mut best: Option<(f32, &String)> = None;
        for (i, (part, pts, half)) in self.ribbons.iter().enumerate() {
            let limit = half.max(6.0);
            // Skip ribbons whose box (grown by the reach) misses the point.
            if let Some(b) = self.boxes.get(i)
                && (p[0] < b[0] - limit
                    || p[0] > b[2] + limit
                    || p[1] < b[1] - limit
                    || p[1] > b[3] + limit)
            {
                continue;
            }
            let d = dist_polyline(p, pts);
            if d <= limit {
                let depth = d / limit;
                if best.is_none_or(|(b, _)| depth < b) {
                    best = Some((depth, part));
                }
            }
        }
        best.map(|(_, p)| p.clone())
    }
}

/// The edge points of a ribbon: left and right offsets of the centre line in the xy plane.
fn edges(pts: &[[f64; 2]], half: f64) -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    let n = pts.len();
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        let a = pts[i.saturating_sub(1)];
        let b = pts[(i + 1).min(n - 1)];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-9);
        let (nx, ny) = (-dy / len, dx / len);
        l.push([pts[i][0] + nx * half, pts[i][1] + ny * half]);
        r.push([pts[i][0] - nx * half, pts[i][1] - ny * half]);
    }
    (l, r)
}

/// The rounded-rectangle loop of the collar in the yz plane (x = 0).
fn collar_loop(half_h: f64, half_d: f64, corner: f64) -> Vec<[f64; 3]> {
    let (cy, cz) = (half_h - corner, half_d - corner);
    let mut pts = Vec::new();
    for (k, (oy, oz)) in [(cy, cz), (-cy, cz), (-cy, -cz), (cy, -cz)]
        .into_iter()
        .enumerate()
    {
        for s in 0..=8 {
            let a = ((k * 90) as f64 + s as f64 * 90.0 / 8.0).to_radians();
            pts.push([0.0, oy + corner * a.cos(), oz + corner * a.sin()]);
        }
    }
    pts
}

/// The collar's front or back half (by rotated depth) as tube strokes with a lighter core.
fn collar_half(
    mut shape: Shape,
    projected: &[([f32; 2], f64)],
    front: bool,
    tube: f32,
    c: Color,
    core: Color,
) -> Shape {
    let mut run: Vec<[f32; 2]> = Vec::new();
    let flush = |shape: Shape, run: &mut Vec<[f32; 2]>| -> Shape {
        if run.len() < 2 {
            run.clear();
            return shape;
        }
        let pts = std::mem::take(run);
        shape
            .stroke_with(pts.clone(), false, tube, LineCap::Round, c)
            .stroke_with(pts, false, (tube * 0.35).max(1.0), LineCap::Round, core)
    };
    for i in 0..=projected.len() {
        let (p, z) = projected[i % projected.len()];
        if (z >= 0.0) == front {
            run.push(p);
        } else {
            shape = flush(shape, &mut run);
        }
    }
    flush(shape, &mut run)
}

/// Every other point, keeping both ends.
fn decimate(pts: &[[f32; 2]]) -> impl DoubleEndedIterator<Item = [f32; 2]> + '_ {
    let n = pts.len();
    pts.iter()
        .enumerate()
        .filter(move |(i, _)| i % 2 == 0 || *i + 1 == n)
        .map(|(_, p)| *p)
}

pub struct Draw<'a> {
    pub scene: &'a Scene,
    pub cam: Cam,
    pub dark: bool,
    pub highlight: Option<&'a str>,
}

/// The frame's pick geometry alone (the GPU view draws the scene itself): ribbon centre lines
/// and the visible faces of each coin, projected through the same camera.
pub fn picks(sc: &Scene, cam: Cam) -> Picks {
    let mut picks = Picks::default();
    for r in &sc.ribbons {
        let centre: Vec<[f32; 2]> = r
            .points
            .iter()
            .map(|p| cam.project([p[0], p[1], 0.0]).0)
            .collect();
        picks
            .ribbons
            .push((r.part.clone(), centre, (r.thickness * cam.ppu) as f32 / 2.0));
    }
    for b in &sc.blocks {
        let (hx, hy, hz) = (BLOCK_W / 2.0, b.h / 2.0, block_depth(b.h) / 2.0);
        let v = |sx: f64, sy: f64, sz: f64| [b.x + sx * hx, b.y + sy * hy, sz * hz];
        let quads: [([f64; 3], [[f64; 3]; 4]); 6] = [
            (
                [0.0, 0.0, 1.0],
                [
                    v(-1.0, -1.0, 1.0),
                    v(1.0, -1.0, 1.0),
                    v(1.0, 1.0, 1.0),
                    v(-1.0, 1.0, 1.0),
                ],
            ),
            (
                [0.0, 0.0, -1.0],
                [
                    v(-1.0, -1.0, -1.0),
                    v(-1.0, 1.0, -1.0),
                    v(1.0, 1.0, -1.0),
                    v(1.0, -1.0, -1.0),
                ],
            ),
            (
                [0.0, 1.0, 0.0],
                [
                    v(-1.0, 1.0, -1.0),
                    v(-1.0, 1.0, 1.0),
                    v(1.0, 1.0, 1.0),
                    v(1.0, 1.0, -1.0),
                ],
            ),
            (
                [0.0, -1.0, 0.0],
                [
                    v(-1.0, -1.0, -1.0),
                    v(1.0, -1.0, -1.0),
                    v(1.0, -1.0, 1.0),
                    v(-1.0, -1.0, 1.0),
                ],
            ),
            (
                [1.0, 0.0, 0.0],
                [
                    v(1.0, -1.0, -1.0),
                    v(1.0, 1.0, -1.0),
                    v(1.0, 1.0, 1.0),
                    v(1.0, -1.0, 1.0),
                ],
            ),
            (
                [-1.0, 0.0, 0.0],
                [
                    v(-1.0, -1.0, -1.0),
                    v(-1.0, -1.0, 1.0),
                    v(-1.0, 1.0, 1.0),
                    v(-1.0, 1.0, -1.0),
                ],
            ),
        ];
        let visible = quads
            .iter()
            .filter(|(n, _)| cam.normal_z(*n) > 0.0)
            .map(|(_, q)| q.iter().map(|p| cam.project(*p).0).collect())
            .collect();
        picks.blocks.push((b.part.clone(), visible));
    }
    picks.with_boxes()
}

impl Draw<'_> {
    /// The whole scene as one shape, plus the frame's pick geometry.
    pub fn render(&self) -> (Shape, Picks) {
        let sc = self.scene;
        let cam = self.cam;
        let mut shape = Shape::new();
        let mut picks = Picks::default();
        let ppu = cam.ppu;

        // Collar: back half first.
        let half_d = 0.035 + RIBBON_DEPTH;
        let corner = 0.9 * half_d.min(sc.collar_half);
        let collar = collar_loop(sc.collar_half, half_d, corner);
        let tube = (2.0 * 0.05 * ppu).max(2.0) as f32;
        let (collar_c, collar_core) = if self.dark {
            (
                h("#dbe5f3").mix(h("#1e293b"), 0.4).alpha(0.55),
                h("#60a5fa").alpha(0.55),
            )
        } else {
            (h("#f3f7fc").alpha(0.9), h("#8fb8f5").alpha(0.7))
        };
        let projected: Vec<([f32; 2], f64)> = collar.iter().map(|p| cam.project(*p)).collect();
        shape = collar_half(shape, &projected, false, tube, collar_c, collar_core);

        // Ribbons: back faces, then front faces painter-sorted.
        let d = RIBBON_DEPTH / 2.0;
        let front_n = [0.0, 0.0, 1.0];
        let facing = cam.normal_z(front_n) >= 0.0;
        let face_z = if facing { d } else { -d };
        let _ = d;
        let light = cam.light(if facing { front_n } else { [0.0, 0.0, -1.0] });
        let edge_c = if self.dark {
            h("#0b1220").alpha(0.85)
        } else {
            h("#3f4a5c").alpha(0.85)
        };
        let depths: Vec<f64> = sc
            .ribbons
            .iter()
            .map(|r| {
                r.points
                    .iter()
                    .step_by(4)
                    .map(|p| cam.rot.apply([p[0], p[1], 0.0])[2])
                    .sum::<f64>()
            })
            .collect();
        let mut order: Vec<usize> = (0..sc.ribbons.len()).collect();
        order.sort_by(|&a, &b| depths[a].total_cmp(&depths[b]));
        for &i in &order {
            let r = &sc.ribbons[i];
            let t = tone(self.dark, r.fee, r.estimated);
            let (l, rr) = edges(&r.points, r.thickness / 2.0);
            let (lc, rc) = edges(&r.points, r.thickness / 4.0);
            let proj = |pts: &[[f64; 2]], z: f64| {
                pts.iter()
                    .map(|p| cam.project([p[0], p[1], z]).0)
                    .collect::<Vec<_>>()
            };
            let (lf, rf) = (proj(&l, face_z), proj(&rr, face_z));
            let (lcf, rcf) = (proj(&lc, face_z), proj(&rc, face_z));
            // The front face and its attenuation-tinted core, each one polygon so antialiasing
            // stays on the outline (no seams). Fills use every other curve sample: Rune
            // ear-clips fills in O(n²), and 21 segments per curve still read smooth; the edge
            // and rim strokes keep every sample. The back face sits 0.07 behind and shows as at
            // most a sub-point sliver at these angles, so the darker core stands in for it.
            let outline = |a: &[[f32; 2]], b: &[[f32; 2]]| {
                decimate(a).chain(decimate(b).rev()).collect::<Vec<_>>()
            };
            shape = shape.fill(outline(&lf, &rf), shade(t.fill, light));
            shape = shape.fill(outline(&lcf, &rcf), t.core);
            let px = (r.thickness * ppu) as f32;
            if !r.fee && r.thickness >= EDGED {
                let w = (2.0 * 0.014 * ppu).max(1.0) as f32;
                shape = shape
                    .stroke_with(lf.clone(), false, w, LineCap::Butt, edge_c)
                    .stroke_with(rf.clone(), false, w, LineCap::Butt, edge_c);
            }
            if px >= 3.0 {
                shape = shape.stroke_with(lf.clone(), false, 1.0, LineCap::Butt, t.rim);
            }
            if self.highlight == Some(r.part.as_str()) {
                let sleeve = if r.fee {
                    if self.dark {
                        h("#fcd34d")
                    } else {
                        h("#f59e0b")
                    }
                } else if self.dark {
                    h("#67e8f9")
                } else {
                    h("#06b6d4")
                };
                let (sl, sr) = edges(&r.points, r.thickness / 2.0 + 0.0175);
                let (slf, srf) = (proj(&sl, face_z + 0.0175), proj(&sr, face_z + 0.0175));
                shape = shape.fill(outline(&slf, &srf), sleeve.alpha(0.42));
            }
            let centre: Vec<[f32; 2]> = r
                .points
                .iter()
                .map(|p| cam.project([p[0], p[1], 0.0]).0)
                .collect();
            picks.ribbons.push((r.part.clone(), centre, px / 2.0));
        }

        // Coins: visible faces of each box, nearest faces last.
        let satin = |owned: bool, hover: bool| match (self.dark, owned, hover) {
            (_, true, false) => h("#2563eb"),
            (false, false, false) => h("#7b8798"),
            (true, false, false) => h("#64748b"),
            (false, true, true) => h("#3b82f6"),
            (true, true, true) => h("#60a5fa"),
            (false, false, true) => h("#a3adbb"),
            (true, false, true) => h("#94a3b8"),
        };
        let mut faces: Vec<(f64, Vec<[f32; 2]>, Color, usize)> = Vec::new();
        for (bi, b) in sc.blocks.iter().enumerate() {
            let (hx, hy, hz) = (BLOCK_W / 2.0, b.h / 2.0, block_depth(b.h) / 2.0);
            let c = satin(b.owned, self.highlight == Some(b.part.as_str()));
            let v = |sx: f64, sy: f64, sz: f64| [b.x + sx * hx, b.y + sy * hy, sz * hz];
            let quads: [([f64; 3], [[f64; 3]; 4]); 6] = [
                (
                    [0.0, 0.0, 1.0],
                    [
                        v(-1.0, -1.0, 1.0),
                        v(1.0, -1.0, 1.0),
                        v(1.0, 1.0, 1.0),
                        v(-1.0, 1.0, 1.0),
                    ],
                ),
                (
                    [0.0, 0.0, -1.0],
                    [
                        v(-1.0, -1.0, -1.0),
                        v(-1.0, 1.0, -1.0),
                        v(1.0, 1.0, -1.0),
                        v(1.0, -1.0, -1.0),
                    ],
                ),
                (
                    [0.0, 1.0, 0.0],
                    [
                        v(-1.0, 1.0, -1.0),
                        v(-1.0, 1.0, 1.0),
                        v(1.0, 1.0, 1.0),
                        v(1.0, 1.0, -1.0),
                    ],
                ),
                (
                    [0.0, -1.0, 0.0],
                    [
                        v(-1.0, -1.0, -1.0),
                        v(1.0, -1.0, -1.0),
                        v(1.0, -1.0, 1.0),
                        v(-1.0, -1.0, 1.0),
                    ],
                ),
                (
                    [1.0, 0.0, 0.0],
                    [
                        v(1.0, -1.0, -1.0),
                        v(1.0, 1.0, -1.0),
                        v(1.0, 1.0, 1.0),
                        v(1.0, -1.0, 1.0),
                    ],
                ),
                (
                    [-1.0, 0.0, 0.0],
                    [
                        v(-1.0, -1.0, -1.0),
                        v(-1.0, -1.0, 1.0),
                        v(-1.0, 1.0, 1.0),
                        v(-1.0, 1.0, -1.0),
                    ],
                ),
            ];
            let mut visible = Vec::new();
            for (n, q) in quads {
                if cam.normal_z(n) <= 0.0 {
                    continue;
                }
                let pts: Vec<[f32; 2]> = q.iter().map(|p| cam.project(*p).0).collect();
                let depth = q.iter().map(|p| cam.rot.apply(*p)[2]).sum::<f64>() / 4.0;
                faces.push((depth, pts.clone(), shade(c, cam.light(n)), bi));
                visible.push(pts);
            }
            picks.blocks.push((b.part.clone(), visible));
        }
        faces.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, pts, c, _) in faces {
            let outline = pts.clone();
            shape = shape.fill(pts, c).stroke_with(
                outline,
                true,
                0.75,
                LineCap::Butt,
                c.mix(Color::BLACK, 0.25),
            );
        }

        shape = collar_half(shape, &projected, true, tube, collar_c, collar_core);
        (shape, picks.with_boxes())
    }
}

#[cfg(test)]
mod tests {
    use super::super::data::{Node, Payload};
    use super::*;

    fn n(id: &str, v: f64, owned: bool) -> Node {
        Node {
            id: id.into(),
            value_sats: Some(v),
            value_btc: Some(v / 1e8),
            value_state: Some("known".into()),
            ownership: Some(if owned { "owned" } else { "unknown" }.into()),
            ..Default::default()
        }
    }

    #[test]
    fn layout_mirrors_outputs_and_keeps_the_fee_without_a_coin() {
        let g = Payload {
            support_level: "full".into(),
            inputs: vec![n("in-0", 100.0, true)],
            outputs: vec![n("out-0", 60.0, false), n("out-1", 39.0, true)],
            fee: Some(Node {
                id: "fee".into(),
                role: Some("fee".into()),
                value_sats: Some(1.0),
                value_state: Some("known".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let sc = layout(&g, false);
        assert_eq!(sc.ribbons.len(), 4);
        assert_eq!(sc.blocks.len(), 3);
        let input = &sc.ribbons[0];
        assert!(input.points[0][0] < 0.0 && sc.ribbons[2].points[0][0] > 0.0);
        assert!((input.points[0][0] - (70.0 - 750.0) * 0.01).abs() < 1e-9);
        assert_eq!(sc.legend(), vec!["known", "own", "other", "fee"]);
        assert!(!sc.selectable("output:fee"));
        let cam = Cam::new(sc.frame_at(REST.0, REST.1), REST.0, REST.1, 900.0, 380.0);
        let (_, picks) = Draw {
            scene: &sc,
            cam,
            dark: true,
            highlight: None,
        }
        .render();
        // Picks are in paint order; probe the input ribbon near its outer end, past its coin.
        let input = picks.ribbons.iter().find(|r| r.0 == "input:in-0").unwrap();
        assert_eq!(picks.pick(input.1[3]).as_deref(), Some("input:in-0"));
    }

    fn many(prefix: &str, count: usize) -> Vec<Node> {
        (0..count)
            .map(|i| n(&format!("{prefix}{i}"), 10_000.0 + i as f64, false))
            .collect()
    }

    fn side<'a>(sc: &'a Scene, s: &str) -> Vec<&'a Block> {
        sc.blocks.iter().filter(|b| b.part.starts_with(s)).collect()
    }

    /// The web test's `outerSpan`: from the lowest to the highest ribbon edge at the outer end.
    fn outer_span(sc: &Scene, s: &str) -> f64 {
        let ends: Vec<(f64, f64)> = sc
            .ribbons
            .iter()
            .filter(|r| r.part.starts_with(s))
            .map(|r| (r.points[0][1], r.thickness / 2.0))
            .collect();
        ends.iter().map(|(y, h)| y + h).fold(f64::MIN, f64::max)
            - ends.iter().map(|(y, h)| y - h).fold(f64::MAX, f64::min)
    }

    #[test]
    fn keeps_the_glass_limits_legs_and_folds_the_rest_into_one_more_leg() {
        let g = Payload {
            support_level: "full".into(),
            inputs: many("i", 300),
            outputs: many("o", 2),
            ..Default::default()
        };
        let sc = layout(&g, false);
        let inputs = side(&sc, "input:");
        assert_eq!(inputs.len(), GLASS_LINE_LIMIT + 1);
        // The fold is not the server's overflow row, which shares the row id.
        assert_eq!(inputs.last().unwrap().part, "input:glass-fold");
        assert_eq!(
            sc.rows["input:glass-fold"].node.overflow_count,
            Some((300 - GLASS_LINE_LIMIT) as f64)
        );
    }

    #[test]
    fn counts_the_fee_towards_the_destination_sides_legs() {
        let fee = Node {
            id: "fee".into(),
            role: Some("fee".into()),
            value_sats: Some(1.0),
            value_state: Some("known".into()),
            ..Default::default()
        };
        let g = Payload {
            support_level: "full".into(),
            inputs: many("i", 2),
            outputs: many("o", 300),
            fee: Some(fee),
            ..Default::default()
        };
        let sc = layout(&g, false);
        // The fee and 59 outputs stay individual, then one more-leg for the other 241.
        let destination = sc
            .ribbons
            .iter()
            .filter(|r| r.part.starts_with("output:"))
            .count();
        assert_eq!(destination, GLASS_LINE_LIMIT + 1);
        assert_eq!(
            sc.rows["output:glass-fold"].node.overflow_count,
            Some((300 - GLASS_LINE_LIMIT + 1) as f64)
        );
    }

    #[test]
    fn keeps_a_big_transaction_as_tall_as_a_small_one() {
        let small = layout(
            &Payload {
                support_level: "full".into(),
                inputs: many("i", 3),
                outputs: many("o", 3),
                ..Default::default()
            },
            false,
        );
        let big = layout(
            &Payload {
                support_level: "full".into(),
                inputs: many("i", 300),
                outputs: many("o", 300),
                ..Default::default()
            },
            false,
        );
        assert!((outer_span(&big, "input:") - outer_span(&small, "input:")).abs() < 0.05);
    }

    #[test]
    fn a_folded_rows_leg_lights_the_fold_and_neither_fold_nor_fee_is_selectable() {
        let fee = Node {
            id: "fee".into(),
            role: Some("fee".into()),
            value_sats: Some(1.0),
            value_state: Some("known".into()),
            ..Default::default()
        };
        let g = Payload {
            support_level: "full".into(),
            inputs: many("i", 100),
            outputs: many("o", 2),
            fee: Some(fee),
            ..Default::default()
        };
        let sc = layout(&g, false);
        assert_eq!(sc.drawn_part("input:i99"), "input:glass-fold");
        // A leg that is drawn lights itself; an unfolded side keeps an unknown part as it is.
        assert_eq!(sc.drawn_part("input:i0"), "input:i0");
        assert_eq!(sc.drawn_part("output:nope"), "output:nope");
        assert!(sc.selectable("input:i0"));
        assert!(!sc.selectable("input:glass-fold"));
        assert!(!sc.selectable("output:fee"));
    }

    #[test]
    fn picks_find_legs_where_they_are_drawn_and_nothing_in_a_corner() {
        let g = Payload {
            support_level: "full".into(),
            inputs: vec![n("in", 100.0, true)],
            outputs: vec![n("out", 99.0, false)],
            ..Default::default()
        };
        let sc = layout(&g, false);
        for (w, h) in [(900.0, 360.0), (500.0, 700.0)] {
            let picks = super::picks(&sc, Cam::new(sc.frame(), REST.0, REST.1, w, h));
            // The ribbons meet at the middle.
            assert!(picks.pick([w / 2.0, h / 2.0]).is_some());
            assert_eq!(picks.pick([2.0, 2.0]), None);
        }
    }
}
