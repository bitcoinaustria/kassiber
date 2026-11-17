//! The flat bowtie (TransactionFlowDiagram in TransactionGraphTab.tsx): the web's fallback when
//! WebGL is missing, offered here behind the 2D/3D switch. Strands carry the web's horizontal
//! gradients (drawn as colour-stepped polyline pieces), swallowtail and arrow markers, and copy
//! their reference on click.

use rune::{Color, LineCap, Shape};

use super::data::Payload;
use super::geometry::{self, Line, Opts};
use super::model::{self, Row, Side};

const ROW_H: f64 = 29.0;
const ZERO_TH: f64 = 3.0;
const MAX_EXPANDED_ROWS: usize = 250;

pub struct Strand {
    pub part: String,
    pub row: Row,
    pub points: Vec<[f32; 2]>,
    pub width: f32,
    pub zero: bool,
    pub fee: bool,
    pub input: bool,
}

pub struct Flat {
    pub width: f64,
    pub height: f64,
    pub viewport: f64,
    pub strands: Vec<Strand>,
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    rune::color::hex(&format!("#{r:02x}{g:02x}{b:02x}")).expect("colour")
}

/// Sizes and strands for a canvas `width` points wide.
pub fn layout(g: &Payload, hide: bool, expanded: bool, width: f64, window_h: f64) -> Flat {
    let max = if expanded {
        MAX_EXPANDED_ROWS
    } else {
        model::MAX_COMPACT_ROWS
    };
    let lr = model::layout_rows(g, hide, max);
    let rows = lr
        .layout_inputs
        .len()
        .max(lr.layout_destinations.len())
        .max(2);
    let height = 280.0f64.max(rows as f64 * ROW_H + 90.0);
    let viewport = if expanded {
        (0.72 * window_h).min(440.0f64.max((height + 24.0).min(760.0)))
    } else {
        340.0f64.max((height + 8.0).min(430.0))
    };
    let w = width.max(420.0).floor();
    let centre = w / 2.0;
    let pad = if expanded { 84.0 } else { 64.0 };
    let curve_width = centre - pad - 12.0;
    let cw =
        (if expanded { 96.0f64 } else { 82.0 }).min(26.0f64.max(((w - 2.0 * pad) / 9.0).floor()));
    let total = geometry::total(&lr.total_inputs, &lr.total_destinations, g.is_liquid());
    let o = Opts {
        height,
        combined_weight: cw,
        curve_width,
        outer_top: 40.0,
        outer_span: 120.0f64.max(height - 80.0),
        zero_thickness: ZERO_TH,
    };
    let mut strands = Vec::new();
    for (layout_rows, display, input) in [
        (&lr.layout_inputs, &lr.inputs, true),
        (&lr.layout_destinations, &lr.destinations, false),
    ] {
        for (i, l) in geometry::lines(layout_rows, total, o)
            .into_iter()
            .enumerate()
        {
            let row = display[i].clone();
            let fee = row.side == Side::Fee;
            let pts = if l.zero {
                stub(&l, pad, centre)
            } else {
                path(&l, pad, centre, fee)
            };
            let pts = pts
                .into_iter()
                .map(|[x, y]| [(if input { x } else { w - x }) as f32, y as f32])
                .collect();
            strands.push(Strand {
                part: row.part(),
                row,
                points: pts,
                width: (l.thickness + 1.0) as f32,
                zero: l.zero,
                fee,
                input,
            });
        }
    }
    Flat {
        width: w,
        height,
        viewport,
        strands,
    }
}

/// `makeBowtiePath` for the input side.
fn path(l: &Line, pad: f64, centre: f64, fee: bool) -> Vec<[f64; 2]> {
    let stroke = l.thickness + 1.0;
    let lead = if !fee && !l.zero { stroke * 0.5 } else { 0.0 };
    let start = pad + lead;
    let end = centre + 1.0;
    let cs = (start + 5.0)
        .max(pad + l.curve_base - l.offset)
        .min(end - 28.0);
    let ce = (end - l.offset - 10.0).max(cs + 18.0).min(end - 4.0);
    let mid = (cs + ce) / 2.0;
    let mut pts = Vec::new();
    // Straight runs are split so the gradient can step along them.
    for k in 0..=6 {
        pts.push([start + (cs - start) * k as f64 / 6.0, l.outer_y]);
    }
    for s in 1..=24 {
        let t = s as f64 / 24.0;
        pts.push([
            geometry::bezier(cs, mid, mid, ce, t),
            geometry::bezier(l.outer_y, l.outer_y, l.inner_y, l.inner_y, t),
        ]);
    }
    for k in 1..=4 {
        pts.push([ce + (end - ce) * k as f64 / 4.0, l.inner_y]);
    }
    pts
}

/// The zero-value stub: a short horizontal line that never reaches the band.
fn stub(l: &Line, pad: f64, centre: f64) -> Vec<[f64; 2]> {
    let half = 1.5f64.max((l.thickness + 1.0) / 2.0);
    let start = pad + half;
    let len = 60.0f64.min(20.0f64.max(centre - pad - 110.0));
    vec![[start, l.outer_y], [start + len, l.outer_y]]
}

fn gradient(input: bool, fee: bool, hover: bool, t: f32) -> Color {
    let lerp = |a: Color, b: Color, t: f32| a.mix(b, t.clamp(0.0, 1.0));
    let (blue, sky, amber) = if hover {
        (rgb(96, 165, 250), rgb(34, 211, 238), rgb(251, 191, 36))
    } else {
        (rgb(59, 130, 246), rgb(14, 165, 233), rgb(245, 158, 11))
    };
    if fee {
        let stop = if hover { 0.58 } else { 0.52 };
        return if t <= stop {
            lerp(sky, amber, t / stop)
        } else {
            lerp(amber, amber.alpha(0.0), (t - stop) / (1.0 - stop))
        };
    }
    if input {
        lerp(blue, sky, t)
    } else {
        lerp(sky, blue, t)
    }
}

pub fn render(f: &Flat, hover: Option<&str>) -> Shape {
    let mut shape = Shape::new();
    for s in &f.strands {
        let on = hover == Some(s.part.as_str());
        let (x0, x1) = s
            .points
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
        let span = (x1 - x0).max(1.0);
        if s.zero {
            // The stub keeps the SVG's round caps.
            shape = shape.stroke_with(
                s.points.clone(),
                false,
                s.width,
                LineCap::Round,
                gradient(s.input, s.fee, on, 0.5),
            );
        } else {
            // One quad per segment along the offset outline, coloured at its position in the
            // gradient. Each quad reaches half a point into the next, so the opaque pieces
            // overlap instead of leaving antialiased seams.
            let half = s.width / 2.0;
            let n = s.points.len();
            let normal = |i: usize| {
                let a = s.points[i.saturating_sub(1)];
                let b = s.points[(i + 1).min(n - 1)];
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                (-dy / len, dx / len, dx / len, dy / len)
            };
            for k in 0..n - 1 {
                let (nx0, ny0, _, _) = normal(k);
                let (nx1, ny1, tx1, ty1) = normal(k + 1);
                let [x0p, y0p] = s.points[k];
                let ext = if k + 2 < n { 0.5 } else { 0.0 };
                let [x1p, y1p] = [
                    s.points[k + 1][0] + tx1 * ext,
                    s.points[k + 1][1] + ty1 * ext,
                ];
                let t = ((x0p + x1p) / 2.0 - x0) / span;
                shape = shape.fill(
                    vec![
                        [x0p + nx0 * half, y0p + ny0 * half],
                        [x1p + nx1 * half, y1p + ny1 * half],
                        [x1p - nx1 * half, y1p - ny1 * half],
                        [x0p - nx0 * half, y0p - ny0 * half],
                    ],
                    gradient(s.input, s.fee, on, t),
                );
            }
        }
        if !s.fee && !s.zero {
            let unit = s.width / 10.0;
            let [sx, sy] = s.points[0];
            let marker: &[(f32, f32)] = if s.input {
                &[
                    (-5.0, -5.0),
                    (0.0, 0.0),
                    (-5.0, 5.0),
                    (1.0, 5.0),
                    (1.0, -5.0),
                ]
            } else {
                &[
                    (1.0, -5.0),
                    (0.0, -5.0),
                    (-5.0, 0.0),
                    (0.0, 5.0),
                    (1.0, 5.0),
                ]
            };
            let dir = if s.input { 1.0 } else { -1.0 };
            let pts: Vec<[f32; 2]> = marker
                .iter()
                .map(|&(u, v)| [sx + dir * u * unit, sy + dir * v * unit])
                .collect();
            shape = shape.fill(
                pts,
                if on {
                    rgb(96, 165, 250)
                } else {
                    rgb(59, 130, 246)
                },
            );
        }
    }
    shape
}

/// The strand under a point: within half its hit width (`max(18, thickness + 12)`), the last
/// painted wins as in the SVG.
pub fn pick(f: &Flat, p: [f32; 2]) -> Option<String> {
    f.strands
        .iter()
        .rev()
        .find(|s| {
            super::glass::dist_polyline(p, &s.points) <= (18.0f32).max(s.width - 1.0 + 12.0) / 2.0
        })
        .map(|s| s.part.clone())
}

#[cfg(test)]
mod tests {
    use super::super::data::{Node, Payload};
    use super::*;

    #[test]
    fn strands_meet_in_the_middle_and_mirror() {
        let n = |id: &str, v: f64| Node {
            id: id.into(),
            value_sats: Some(v),
            value_btc: Some(v / 1e8),
            value_state: Some("known".into()),
            ..Default::default()
        };
        let g = Payload {
            support_level: "full".into(),
            inputs: vec![n("in-0", 10.0)],
            outputs: vec![n("out-0", 6.0), n("out-1", 4.0)],
            ..Default::default()
        };
        let f = layout(&g, false, false, 960.0, 900.0);
        assert_eq!(f.strands.len(), 3);
        assert_eq!(f.height, 280.0);
        assert_eq!(f.viewport, 340.0);
        let input_end = f.strands[0].points.last().unwrap()[0];
        let output_end = f.strands[1].points.last().unwrap()[0];
        assert_eq!((input_end, output_end), (481.0, 479.0));
        let mid = f.strands[0].points[3];
        assert_eq!(pick(&f, mid).as_deref(), Some("input:in-0"));
    }
}
