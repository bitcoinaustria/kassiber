//! Triangle meshes for the glass scene, built once per layout (glassScene.ts):
//! ribbons as a stadium cross-section swept along the centre line (depth 0.07, both ends capped),
//! satin edge lines on wide ribbons, rounded-box coins, the tube collar, and highlight sleeves.
//! Coordinates are scene units (1 = 100 virtual px), y up, z toward the viewer.

use bytemuck::{Pod, Zeroable};

use super::super::glass::{RIBBON_DEPTH, Scene};

/// Material slots (the shader's table, filled per theme in `materials`).
pub const M_KNOWN: u32 = 0;
pub const M_ESTIMATED: u32 = 1;
pub const M_FEE: u32 = 2;
pub const M_FEE_ESTIMATED: u32 = 3;
pub const M_CENTER: u32 = 4;
pub const M_EDGE: u32 = 5;
pub const M_COIN_OWN: u32 = 6;
pub const M_COIN_OTHER: u32 = 7;
pub const M_SLEEVE: u32 = 10;
pub const M_SLEEVE_FEE: u32 = 11;

const EDGED_THICKNESS: f64 = 0.08;
const EDGE_HALF_WIDTH: f64 = 0.014;
const EDGE_DEPTH: f64 = 0.049;
const EDGE_OFFSET: f64 = 0.0056;
const BLOCK_W: f64 = 0.42;
const SLEEVE_GROW: f64 = 0.035;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub mat: u32,
    pub part: u32,
}

#[derive(Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    fn v(&mut self, p: [f64; 3], n: [f64; 3], mat: u32, part: u32) -> u32 {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        self.vertices.push(Vertex {
            pos: [p[0] as f32, p[1] as f32, p[2] as f32],
            normal: [
                (n[0] / len) as f32,
                (n[1] / len) as f32,
                (n[2] / len) as f32,
            ],
            mat,
            part,
        });
        (self.vertices.len() - 1) as u32
    }
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }
}

/// The meshes of one scene, by pass.
#[derive(Default)]
pub struct SceneMeshes {
    /// Satin: coins and edge lines (drawn first, seen through the glass).
    pub opaque: Mesh,
    /// Glass: ribbons and the collar.
    pub glass: Mesh,
    /// Highlight sleeves for every selectable leg (the shader keeps the lit one).
    pub sleeves: Mesh,
    /// Part ids in order (index + 1 is the id the vertices carry).
    pub parts: Vec<String>,
}

impl SceneMeshes {
    pub fn part_id(&self, part: &str) -> u32 {
        self.parts
            .iter()
            .position(|p| p == part)
            .map(|i| i as u32 + 1)
            .unwrap_or(0)
    }
}

/// A rounded rectangle cross-section (half extents `a` across, `b` deep, corner radius
/// `min(a, b)` as the web's stadium), as (u, v, nu, nv) points in loop order.
fn profile(a: f64, b: f64, quarter_steps: usize) -> Vec<[f64; 4]> {
    let r = a.min(b);
    let (cu, cv) = (a - r, b - r);
    let mut out = Vec::new();
    for (k, (ou, ov)) in [(cu, cv), (-cu, cv), (-cu, -cv), (cu, -cv)]
        .into_iter()
        .enumerate()
    {
        for s in 0..=quarter_steps {
            let t = (k as f64 + s as f64 / quarter_steps as f64) * std::f64::consts::FRAC_PI_2;
            let (c, sn) = (t.cos(), t.sin());
            out.push([ou + r * c, ov + r * sn, c, sn]);
        }
    }
    out
}

/// In-plane unit normals of a polyline (left of the direction of travel).
fn normals(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts[i.saturating_sub(1)];
            let b = pts[(i + 1).min(n - 1)];
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-12);
            [-dy / len, dx / len]
        })
        .collect()
}

/// Sweeps a cross-section of half width `a` (in the xy plane, across the line) and half depth
/// `b` (z) along `pts`, its centre shifted `offset` along the in-plane normal; caps both ends.
fn sweep(m: &mut Mesh, pts: &[[f64; 2]], offset: f64, a: f64, b: f64, mat: u32, part: u32) {
    if pts.len() < 2 || a <= 0.0 {
        return;
    }
    let prof = profile(a, b, 2);
    let ns = normals(pts);
    let pn = prof.len() as u32;
    let base = m.vertices.len() as u32;
    for (p, n) in pts.iter().zip(&ns) {
        let c = [p[0] + n[0] * offset, p[1] + n[1] * offset];
        for q in &prof {
            let pos = [c[0] + n[0] * q[0], c[1] + n[1] * q[0], q[1]];
            let nrm = [n[0] * q[2], n[1] * q[2], q[3]];
            m.v(pos, nrm, mat, part);
        }
    }
    for i in 0..pts.len() as u32 - 1 {
        for j in 0..pn {
            let j2 = (j + 1) % pn;
            let (a0, a1) = (base + i * pn + j, base + i * pn + j2);
            let (b0, b1) = (base + (i + 1) * pn + j, base + (i + 1) * pn + j2);
            m.tri(a0, b0, b1);
            m.tri(a0, b1, a1);
        }
    }
    // End caps, facing out along the line.
    for (i, sign) in [(0usize, -1.0), (pts.len() - 1, 1.0)] {
        let p = pts[i];
        let n = ns[i];
        let t = [n[1] * sign, -n[0] * sign, 0.0];
        let c = [p[0] + n[0] * offset, p[1] + n[1] * offset];
        let centre = m.v([c[0], c[1], 0.0], t, mat, part);
        let first = m.vertices.len() as u32;
        for q in &prof {
            m.v([c[0] + n[0] * q[0], c[1] + n[1] * q[0], q[1]], t, mat, part);
        }
        for j in 0..pn {
            let (x, y) = (first + j, first + (j + 1) % pn);
            if sign < 0.0 {
                m.tri(centre, y, x)
            } else {
                m.tri(centre, x, y)
            }
        }
    }
}

/// three's RoundedBoxGeometry: a subdivided box whose vertices are pulled onto rounded edges.
fn rounded_box(m: &mut Mesh, centre: [f64; 3], half: [f64; 3], radius: f64, mat: u32, part: u32) {
    let seg = 6usize;
    let inner = [half[0] - radius, half[1] - radius, half[2] - radius];
    // (normal axis, sign, u axis, v axis) per face, wound counter-clockwise from outside.
    let faces: [(usize, f64, usize, usize); 6] = [
        (2, 1.0, 0, 1),
        (2, -1.0, 1, 0),
        (0, 1.0, 1, 2),
        (0, -1.0, 2, 1),
        (1, 1.0, 2, 0),
        (1, -1.0, 0, 2),
    ];
    for (ax, sign, ua, va) in faces {
        let base = m.vertices.len() as u32;
        for j in 0..=seg {
            for i in 0..=seg {
                let mut q = [0.0f64; 3];
                q[ax] = sign * half[ax];
                q[ua] = (i as f64 / seg as f64 * 2.0 - 1.0) * half[ua];
                q[va] = (j as f64 / seg as f64 * 2.0 - 1.0) * half[va];
                let c = [
                    q[0].clamp(-inner[0], inner[0]),
                    q[1].clamp(-inner[1], inner[1]),
                    q[2].clamp(-inner[2], inner[2]),
                ];
                let mut d = [q[0] - c[0], q[1] - c[1], q[2] - c[2]];
                let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if len < 1e-12 {
                    d = [0.0; 3];
                    d[ax] = sign;
                } else {
                    d = [d[0] / len, d[1] / len, d[2] / len];
                }
                let p = [
                    centre[0] + c[0] + d[0] * radius,
                    centre[1] + c[1] + d[1] * radius,
                    centre[2] + c[2] + d[2] * radius,
                ];
                m.v(p, d, mat, part);
            }
        }
        let row = seg as u32 + 1;
        for j in 0..seg as u32 {
            for i in 0..seg as u32 {
                let a = base + j * row + i;
                m.tri(a, a + 1, a + row + 1);
                m.tri(a, a + row + 1, a + row);
            }
        }
    }
}

/// The collar: a closed rounded-rectangle tube in the yz plane at x = 0 (glassScene.ts).
fn collar(m: &mut Mesh, half_h: f64, mat: u32) {
    let half_d = 0.035 + RIBBON_DEPTH;
    let corner = 0.9 * half_d.min(half_h);
    let tube = 0.05;
    let (cy, cz) = (half_h - corner, half_d - corner);
    // Path around the loop: straight sides and quarter arcs, 96 samples.
    let mut path: Vec<[f64; 2]> = Vec::new();
    let arcs = [
        (cy, cz, 0.0),
        (-cy, cz, 90.0),
        (-cy, -cz, 180.0),
        (cy, -cz, 270.0),
    ];
    for (oy, oz, a0) in arcs {
        for s in 0..24 {
            let a = (a0 + s as f64 * 90.0 / 23.0).to_radians();
            path.push([oy + corner * a.cos(), oz + corner * a.sin()]);
        }
    }
    let n = path.len();
    let ring = 12usize;
    let base = m.vertices.len() as u32;
    for i in 0..n {
        let p = path[i];
        let nx = path[(i + 1) % n];
        let pv = path[(i + n - 1) % n];
        let (ty, tz) = (nx[0] - pv[0], nx[1] - pv[1]);
        let tl = (ty * ty + tz * tz).sqrt().max(1e-12);
        // In-loop outward normal (yz) and the x axis span the tube's cross-section.
        let (oy, oz) = (tz / tl, -ty / tl);
        for k in 0..ring {
            let a = k as f64 / ring as f64 * std::f64::consts::TAU;
            let (c, s) = (a.cos(), a.sin());
            let nrm = [s, c * oy, c * oz];
            m.v(
                [tube * nrm[0], p[0] + tube * nrm[1], p[1] + tube * nrm[2]],
                nrm,
                mat,
                0,
            );
        }
    }
    let r = ring as u32;
    for i in 0..n as u32 {
        let i2 = (i + 1) % n as u32;
        for k in 0..r {
            let k2 = (k + 1) % r;
            let (a, b, c, d) = (
                base + i * r + k,
                base + i * r + k2,
                base + i2 * r + k,
                base + i2 * r + k2,
            );
            m.tri(a, c, d);
            m.tri(a, d, b);
        }
    }
}

pub fn build(sc: &Scene) -> SceneMeshes {
    let mut out = SceneMeshes::default();
    let id = |out: &mut SceneMeshes, part: &str| -> u32 {
        if let Some(i) = out.parts.iter().position(|p| p == part) {
            return i as u32 + 1;
        }
        out.parts.push(part.to_owned());
        out.parts.len() as u32
    };
    let half_d = RIBBON_DEPTH / 2.0;
    for r in &sc.ribbons {
        let part = id(&mut out, &r.part);
        let mat = match (r.fee, r.estimated) {
            (false, false) => M_KNOWN,
            (false, true) => M_ESTIMATED,
            (true, false) => M_FEE,
            (true, true) => M_FEE_ESTIMATED,
        };
        let a = r.thickness / 2.0;
        sweep(&mut out.glass, &r.points, 0.0, a, half_d, mat, part);
        if !r.fee && r.thickness >= EDGED_THICKNESS {
            for side in [1.0, -1.0] {
                sweep(
                    &mut out.opaque,
                    &r.points,
                    side * (a + EDGE_OFFSET),
                    EDGE_HALF_WIDTH,
                    EDGE_DEPTH / 2.0,
                    M_EDGE,
                    part,
                );
            }
        }
        let sleeve_mat = if r.fee { M_SLEEVE_FEE } else { M_SLEEVE };
        sweep(
            &mut out.sleeves,
            &r.points,
            0.0,
            a + SLEEVE_GROW / 2.0,
            half_d + SLEEVE_GROW / 2.0,
            sleeve_mat,
            part,
        );
    }
    for b in &sc.blocks {
        let part = id(&mut out, &b.part);
        let depth = 0.5f64.min(0.08f64.max(3.0 * b.h));
        let radius = 0.05f64.min(b.h / 2.5);
        rounded_box(
            &mut out.opaque,
            [b.x, b.y, 0.0],
            [BLOCK_W / 2.0, b.h / 2.0, depth / 2.0],
            radius,
            if b.owned { M_COIN_OWN } else { M_COIN_OTHER },
            part,
        );
    }
    collar(&mut out.glass, sc.collar_half, M_CENTER);
    out
}

#[cfg(test)]
impl SceneMeshes {
    fn indices_ok(&self) -> bool {
        [&self.opaque, &self.glass, &self.sleeves].iter().all(|m| {
            m.indices.iter().all(|&i| (i as usize) < m.vertices.len()) && m.indices.len() % 3 == 0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::glass::{Block, Ribbon, Scene};
    use super::*;

    #[test]
    fn meshes_cover_every_part_and_keep_the_fee_coinless() {
        let line = vec![[-6.8, 0.0], [-3.0, 0.2], [0.0, 0.3]];
        let sc = Scene {
            ribbons: vec![
                Ribbon {
                    part: "input:in-0".into(),
                    estimated: false,
                    fee: false,
                    overflow: false,
                    thickness: 0.5,
                    points: line.clone(),
                },
                Ribbon {
                    part: "output:fee".into(),
                    estimated: false,
                    fee: true,
                    overflow: false,
                    thickness: 0.01,
                    points: line.clone(),
                },
            ],
            blocks: vec![Block {
                part: "input:in-0".into(),
                owned: true,
                x: -7.0,
                y: 0.0,
                h: 0.5,
            }],
            collar_half: 0.5025,
            rows: Default::default(),
        };
        let m = build(&sc);
        assert_eq!(
            m.parts,
            vec!["input:in-0".to_owned(), "output:fee".to_owned()]
        );
        assert_eq!(m.part_id("output:fee"), 2);
        // Ribbon + collar in the glass pass; two edge lines (wide, non-fee) and one coin opaque.
        assert!(!m.glass.indices.is_empty() && !m.opaque.indices.is_empty());
        assert!(
            m.opaque
                .vertices
                .iter()
                .all(|v| v.mat == M_EDGE || v.mat == M_COIN_OWN)
        );
        assert!(
            m.glass.vertices.iter().any(|v| v.mat == M_FEE)
                && m.glass.vertices.iter().any(|v| v.mat == M_CENTER)
        );
        assert!(m.indices_ok());
    }
}
