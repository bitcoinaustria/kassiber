//! Setup's Bitcoin artwork (`chain/chainLayout.ts` + `chain/ChainArtwork.tsx`): a short chain
//! of blocks packed with transactions and the next block, an outline, filling from the
//! mempool, rasterized into whole cells as the web's SVG is. Pure decoration from fixed seeds:
//! no block, fee or transaction is real. Drawn as one canvas; the live loop (a layer per beat,
//! found, the chain moves back a slot) runs from `poll`.

use rune::{Color, El, Shape, canvas, el};

use crate::theme::Tk;

pub const BLOCK_SIZE: f64 = 1.6;
pub const BLOCK_GAP: f64 = 1.35;
const RECEDE_X: f64 = -0.62;
const RECEDE_Z: f64 = -0.78;
pub const BLOCK_LAYERS: u32 = 4;
const GRID: usize = 4;
const WALL: f64 = 0.1;
const TX_GAP: f64 = 0.035;
const FEE_SHARE: f64 = 0.18;
pub const PENDING_COUNT: usize = 6;
/// One beat of the live chain.
pub const TICK: f64 = 0.9;
/// `.kb-chain-block`'s slide (600 ms, 4 steps).
const SLIDE: f64 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Shell,
    Tx,
    Fee,
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box3 {
    pub kind: Kind,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w: f64,
    pub h: f64,
    pub d: f64,
}

/// mulberry32.
fn random(seed: u32) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x6d2b_79f5);
        let mut t = state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

const FOOTPRINTS: [(usize, usize); 4] = [(2, 2), (2, 1), (1, 2), (1, 1)];

fn layer_tiles(next: &mut impl FnMut() -> f64) -> Vec<(usize, usize, usize, usize)> {
    let mut taken = [[false; GRID]; GRID];
    let mut tiles = vec![];
    for row in 0..GRID {
        for col in 0..GRID {
            if taken[row][col] {
                continue;
            }
            let roll = next();
            let wanted = if roll < 0.2 {
                0
            } else if roll < 0.4 {
                1
            } else if roll < 0.6 {
                2
            } else {
                3
            };
            let fits = |(w, h): &(usize, usize)| {
                if col + w > GRID || row + h > GRID {
                    return false;
                }
                (row..row + h).all(|r| (col..col + w).all(|c| !taken[r][c]))
            };
            let (cols, rows) = FOOTPRINTS[wanted..]
                .iter()
                .copied()
                .find(fits)
                .unwrap_or((1, 1));
            for line in taken.iter_mut().skip(row).take(rows) {
                for cell in line.iter_mut().skip(col).take(cols) {
                    *cell = true;
                }
            }
            tiles.push((col, row, cols, rows));
        }
    }
    tiles
}

/// `blockCentre`.
pub fn block_centre(index: u32) -> (f64, f64) {
    let pitch = BLOCK_SIZE + BLOCK_GAP;
    (
        f64::from(index) * pitch * RECEDE_X,
        f64::from(index) * pitch * RECEDE_Z,
    )
}

fn block_transactions(layers: u32, id: i64) -> Vec<Box3> {
    let seed = 0x6b61_7373u32 ^ (id as i32 as u32).wrapping_mul(0x9e37_79b1);
    let mut next = random(seed);
    let inner = BLOCK_SIZE - WALL * 2.0;
    let cell = inner / GRID as f64;
    let layer_h = inner / f64::from(BLOCK_LAYERS);
    let floor = -BLOCK_SIZE / 2.0 + WALL;
    let (left, back) = (-inner / 2.0, -inner / 2.0);
    let mut out = vec![];
    for layer in 0..layers {
        for (col, row, cols, rows) in layer_tiles(&mut next) {
            let h = layer_h * (0.86 + next() * 0.14) - TX_GAP;
            let kind = if next() < FEE_SHARE {
                Kind::Fee
            } else {
                Kind::Tx
            };
            out.push(Box3 {
                kind,
                x: left + (col as f64 + cols as f64 / 2.0) * cell,
                y: floor + f64::from(layer) * layer_h + TX_GAP / 2.0 + h / 2.0,
                z: back + (row as f64 + rows as f64 / 2.0) * cell,
                w: cols as f64 * cell - TX_GAP,
                h,
                d: rows as f64 * cell - TX_GAP,
            });
        }
    }
    out
}

/// `mempool()`.
pub fn mempool() -> Vec<Box3> {
    let mut next = random(0x6d65_6d70);
    let half = BLOCK_SIZE / 2.0;
    (0..PENDING_COUNT)
        .map(|i| {
            let size = 0.2 + next() * 0.2;
            let reach = half - size / 2.0 - WALL;
            let x = (next() * 2.0 - 1.0) * reach;
            let y = half + 0.3 + (i as f64 / PENDING_COUNT as f64) * 1.3 + next() * 0.15;
            let z = (next() * 2.0 - 1.0) * reach;
            Box3 {
                kind: Kind::Pending,
                x,
                y,
                z,
                w: size,
                h: size,
                d: size,
            }
        })
        .collect()
}

const SHELL: Box3 = Box3 {
    kind: Kind::Shell,
    x: 0.0,
    y: 0.0,
    z: 0.0,
    w: BLOCK_SIZE,
    h: BLOCK_SIZE,
    d: BLOCK_SIZE,
};

pub struct Block {
    pub offset: u32,
    pub boxes: Vec<Box3>,
}

/// `chainBlocks`.
pub fn chain_blocks(tip: i64, mined: u32, layers: u32, trailing: bool) -> Vec<Block> {
    let next_layers = layers.min(BLOCK_LAYERS);
    let mut out = vec![];
    for offset in (0..=mined + u32::from(trailing)).rev() {
        let block_layers = if offset == 0 {
            next_layers
        } else {
            BLOCK_LAYERS
        };
        let mut boxes = block_transactions(block_layers, tip - i64::from(offset));
        boxes.push(SHELL);
        out.push(Block { offset, boxes });
    }
    out
}

/// `LoopState` + `advanceChain`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Loop {
    pub tip: i64,
    pub layers: u32,
    pub found: bool,
    /// App-clock time of the last beat (the slide runs from it).
    pub at: f64,
    /// App-clock time the picture next changes (`next_change`): the start page redraws then.
    pub due: f64,
}

impl Loop {
    pub fn new(fill: f64) -> Loop {
        Loop {
            tip: 0,
            layers: (fill.clamp(0.0, 1.0) * f64::from(BLOCK_LAYERS)).round() as u32,
            found: false,
            at: 0.0,
            due: 0.0,
        }
    }
    pub fn advance(&mut self, now: f64) {
        if self.found {
            self.tip += 1;
            self.layers = 0;
            self.found = false;
        } else if self.layers >= BLOCK_LAYERS {
            self.found = true;
        } else {
            self.layers += 1;
        }
        self.at = now;
    }
}

/// When the live artwork next looks different after `clock`: the next beat, slide step, step of
/// a landing layer's flicker or packet phase, as `view` steps them. The start page wakes and
/// redraws exactly then, so the stepped motion lands on time instead of on a polling grid.
pub fn next_change(state: &Loop, clock: f64) -> f64 {
    let since = clock - state.at;
    let mut next = state.at + TICK;
    let mut after = |t: f64| {
        if t > clock && t < next {
            next = t;
        }
    };
    // The slide's four steps (`view`: step = ⌊since / SLIDE × 4⌋ + 1).
    if state.layers == 0 && !state.found && state.tip > 0 {
        for k in 1..4 {
            after(state.at + SLIDE * f64::from(k) / 4.0);
        }
    }
    // The landing layer's flicker: each transaction staggered 9 ms (capped at 0.3 s), hidden
    // and shown in 72 ms phases.
    if !state.found && state.layers > 0 && since < 0.66 {
        for i in 0..=34 {
            let start = state.at + (f64::from(i) * 0.009).min(0.3);
            for phase in [0.0, 0.2, 0.4, 0.6] {
                after(start + phase * 0.36);
            }
        }
        after(state.at + 0.66);
    }
    // The pending packets' 1.8 s loop, 240 ms apart, three phases each.
    if state.layers < BLOCK_LAYERS && !state.found {
        for i in 0..PENDING_COUNT {
            let offset = i as f64 * 0.24;
            let u = (clock + offset).rem_euclid(1.8);
            for edge in [0.63, 1.26, 1.8] {
                after(clock - u + edge);
            }
        }
    }
    // Land just past the edge, so the frame drawn then shows the new step.
    next + 1e-4
}

// ---- Rasterizing (ChainArtwork.tsx) -------------------------------------------------------------

const DEPTH_X: f64 = 0.35;
const DEPTH_Y: f64 = 0.25;
const PIXELS: f64 = 24.0;

type P = (i64, i64);

fn project(x: f64, y: f64, z: f64) -> P {
    (
        js_round((x + z * DEPTH_X) * PIXELS),
        js_round((-y + z * DEPTH_Y) * PIXELS),
    )
}

/// `Math.round` (halves toward +∞).
fn js_round(v: f64) -> i64 {
    (v + 0.5).floor() as i64
}

fn corners(b: &Box3) -> [f64; 6] {
    [
        b.x - b.w / 2.0,
        b.x + b.w / 2.0,
        b.y - b.h / 2.0,
        b.y + b.h / 2.0,
        b.z - b.d / 2.0,
        b.z + b.d / 2.0,
    ]
}

struct Faces {
    front: [P; 4],
    top: [P; 4],
    side: [P; 4],
}

fn faces(b: &Box3) -> Faces {
    let [x0, x1, y0, y1, z0, z1] = corners(b);
    Faces {
        front: [
            project(x0, y0, z1),
            project(x1, y0, z1),
            project(x1, y1, z1),
            project(x0, y1, z1),
        ],
        top: [
            project(x0, y1, z1),
            project(x1, y1, z1),
            project(x1, y1, z0),
            project(x0, y1, z0),
        ],
        side: [
            project(x0, y0, z1),
            project(x0, y0, z0),
            project(x0, y1, z0),
            project(x0, y1, z1),
        ],
    }
}

/// A horizontal run of cells: (x, y, length).
type Run = (i64, i64, i64);

/// `pixelFace`: each face scanned into whole-cell runs.
fn pixel_face(v: &[P]) -> Vec<Run> {
    let mut out = vec![];
    let (lo, hi) = (
        v.iter().map(|p| p.1).min().unwrap_or(0),
        v.iter().map(|p| p.1).max().unwrap_or(0),
    );
    for y in lo..hi {
        let scan = y as f64 + 0.5;
        let mut xs: Vec<f64> = vec![];
        for i in 0..v.len() {
            let (x0, y0) = (v[i].0 as f64, v[i].1 as f64);
            let (x1, y1) = (v[(i + 1) % v.len()].0 as f64, v[(i + 1) % v.len()].1 as f64);
            if (y0 > scan) != (y1 > scan) {
                xs.push(x0 + (scan - y0) * (x1 - x0) / (y1 - y0));
            }
        }
        if xs.len() < 2 {
            continue;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let start = (xs[0] - 0.5).ceil() as i64;
        let end = (xs[xs.len() - 1] - 0.5).ceil() as i64;
        if end > start {
            out.push((start, y, end - start));
        }
    }
    out
}

/// `pixelEdge` (Bresenham, optionally dashed two on, two off).
fn pixel_edge(a: P, b: P, dashed: bool, out: &mut Vec<P>) {
    let (mut x0, mut y0) = a;
    let (x1, y1) = b;
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut tick = 0;
    loop {
        if !dashed || (tick / 2) % 2 == 0 {
            out.push((x0, y0));
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice = 2 * err;
        if twice >= dy {
            err += dy;
            x0 += sx;
        }
        if twice <= dx {
            err += dx;
            y0 += sy;
        }
        tick += 1;
    }
}

fn outline(v: &[P], out: &mut Vec<P>) {
    for i in 0..v.len() {
        pixel_edge(v[i], v[(i + 1) % v.len()], false, out);
    }
}

/// Cells merged into runs per row (one fill per run, not per cell).
fn runs_of(mut cells: Vec<P>) -> Vec<Run> {
    cells.sort_by_key(|&(x, y)| (y, x));
    cells.dedup();
    let mut out: Vec<Run> = vec![];
    for (x, y) in cells {
        match out.last_mut() {
            Some((rx, ry, len)) if *ry == y && *rx + *len == x => *len += 1,
            _ => out.push((x, y, 1)),
        }
    }
    out
}

fn hidden_edges(b: &Box3) -> Vec<Run> {
    let [x0, x1, y0, y1, z0, z1] = corners(b);
    let corner = project(x1, y0, z0);
    let mut cells = vec![];
    for end in [
        project(x0, y0, z0),
        project(x1, y1, z0),
        project(x1, y0, z1),
    ] {
        pixel_edge(corner, end, true, &mut cells);
    }
    runs_of(cells)
}

fn paint_order(a: &Box3, b: &Box3) -> std::cmp::Ordering {
    a.z.partial_cmp(&b.z)
        .unwrap()
        .then(a.y.partial_cmp(&b.y).unwrap())
        .then(b.x.partial_cmp(&a.x).unwrap())
}

/// `frame(mined)`: the fixed view box (min x, min y, width, height).
pub fn frame(mined: u32) -> [f64; 4] {
    let mut xs = vec![];
    let mut ys = vec![];
    let mut boxes = vec![];
    for b in chain_blocks(0, mined, BLOCK_LAYERS, false) {
        let (cx, cz) = block_centre(b.offset);
        for bx in b.boxes {
            if bx.kind == Kind::Shell && b.offset != 0 {
                continue;
            }
            boxes.push(Box3 {
                x: bx.x + cx,
                z: bx.z + cz,
                ..bx
            });
        }
    }
    boxes.extend(mempool());
    for b in &boxes {
        let [x0, x1, y0, y1, z0, z1] = corners(b);
        for x in [x0, x1] {
            for y in [y0, y1] {
                for z in [z0, z1] {
                    let (sx, sy) = project(x, y, z);
                    xs.push(sx as f64);
                    ys.push(sy as f64);
                }
            }
        }
    }
    let pad = 0.25 * PIXELS;
    let min_x = xs.iter().cloned().fold(f64::MAX, f64::min) - pad;
    let min_y = ys.iter().cloned().fold(f64::MAX, f64::min) - pad;
    let max_x = xs.iter().cloned().fold(f64::MIN, f64::max) + pad;
    let max_y = ys.iter().cloned().fold(f64::MIN, f64::max) + pad;
    [min_x, min_y, max_x - min_x, max_y - min_y]
}

/// What the artwork draws at a moment: the loop state, how long since its last beat, and how
/// long it has been on screen (the packets' own loop).
pub struct Paint<'a> {
    pub tk: &'a Tk,
    pub mined: u32,
    pub state: Loop,
    pub since_beat: f64,
    pub clock: f64,
    pub live: bool,
}

/// The artwork sized `w` × `h` (SVG `xMidYMid meet`).
pub fn view(p: Paint, w: f32, h: f32) -> El {
    let vb = frame(p.mined);
    let scale = (f64::from(w) / vb[2]).min(f64::from(h) / vb[3]);
    let ox = (f64::from(w) - vb[2] * scale) / 2.0 - vb[0] * scale;
    let oy = (f64::from(h) - vb[3] * scale) / 2.0 - vb[1] * scale;
    let fg = p.tk.fg;
    let bg = p.tk.background;
    let mut s = Shape::new();
    let add = |s: Shape, runs: &[Run], dx: i64, dy: i64, c: Color| -> Shape {
        let mut s = s;
        for &(x, y, len) in runs {
            let x0 = ((x + dx) as f64 * scale + ox) as f32;
            let y0 = ((y + dy) as f64 * scale + oy) as f32;
            let x1 = ((x + dx + len) as f64 * scale + ox) as f32;
            let y1 = ((y + dy + 1) as f64 * scale + oy) as f32;
            s = s.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], c);
        }
        s
    };
    let st = p.state;
    let blocks = chain_blocks(st.tip, p.mined, st.layers, p.live);
    // The slide: four steps over 600 ms from the beat that moved the chain.
    let moving = p.live && st.layers == 0 && !st.found && st.tip > 0;
    let step = if moving {
        ((p.since_beat / SLIDE * 4.0).floor() as i64 + 1).clamp(1, 4)
    } else {
        4
    };
    for b in &blocks {
        let at = |off: u32| {
            let (x, z) = block_centre(off);
            let (dx, dy) = project(x, 0.0, z);
            (js_round(dx as f64 / 4.0) * 4, js_round(dy as f64 / 4.0) * 4)
        };
        let to = at(b.offset);
        let (dx, dy, in_chain) = if moving && step < 4 {
            let from = at(b.offset.saturating_sub(1));
            let k = step as f64 / 4.0;
            let lerp = |a: i64, b: i64| a + ((b - a) as f64 * k).round() as i64;
            // The block leaving the chain fades out over the same steps.
            (
                lerp(from.0, to.0),
                lerp(from.1, to.1),
                b.offset <= p.mined || step < 2,
            )
        } else {
            (to.0, to.1, b.offset <= p.mined)
        };
        if !in_chain {
            continue;
        }
        let mut txs: Vec<&Box3> = b.boxes.iter().filter(|x| x.kind != Kind::Shell).collect();
        txs.sort_by(|a, b| paint_order(a, b));
        // `kb-chain-decode`: a newly landed layer flickers in (360 ms, 9 ms stagger).
        let fresh_layer =
            p.live && b.offset == 0 && !st.found && st.layers > 0 && p.since_beat < 0.66;
        let layer_h = (BLOCK_SIZE - WALL * 2.0) / f64::from(BLOCK_LAYERS);
        let top_floor = -BLOCK_SIZE / 2.0 + WALL + f64::from(st.layers.saturating_sub(1)) * layer_h;
        for (i, bx) in txs.iter().enumerate() {
            if fresh_layer && bx.y > top_floor {
                let t = p.since_beat - (i as f64 * 0.009).min(0.3);
                let phase = t / 0.36;
                // 0–20 % hidden, 20–40 % shown, 40–60 % hidden, then shown.
                if t < 0.0 || (0.0..0.2).contains(&phase) || (0.4..0.6).contains(&phase) {
                    continue;
                }
            }
            let f = faces(bx);
            let (front, top, side) = if bx.kind == Kind::Fee {
                (
                    rune::color::hex("#f7931a").expect("hex"),
                    rune::color::hex("#fbab4a").expect("hex"),
                    rune::color::hex("#d97d0e").expect("hex"),
                )
            } else {
                (fg, fg, fg)
            };
            s = add(s, &pixel_face(&f.side), dx, dy, side.alpha(side.a * 0.4));
            s = add(s, &pixel_face(&f.top), dx, dy, top.alpha(top.a * 0.7));
            s = add(s, &pixel_face(&f.front), dx, dy, front);
            let mut cells = vec![];
            outline(&f.side, &mut cells);
            outline(&f.top, &mut cells);
            outline(&f.front, &mut cells);
            s = add(s, &runs_of(cells), dx, dy, bg);
        }
        if b.offset == 0 {
            let shell = SHELL;
            let found = st.found;
            let ink = if found {
                rune::color::hex("#f7931a").expect("hex")
            } else {
                fg.alpha(fg.a * 0.45)
            };
            let f = faces(&shell);
            s = add(s, &hidden_edges(&shell), dx, dy, ink.alpha(ink.a * 0.5));
            let mut cells = vec![];
            outline(&f.front, &mut cells);
            outline(&f.top, &mut cells);
            outline(&f.side, &mut cells);
            s = add(s, &runs_of(cells), dx, dy, ink);
            s = add(s, &pixel_face(&f.side), dx, dy, p.tk.fg.alpha(0.03));
            s = add(s, &pixel_face(&f.top), dx, dy, p.tk.fg.alpha(0.05));
            s = add(s, &pixel_face(&f.front), dx, dy, p.tk.fg.alpha(0.02));
        }
    }
    if st.layers < BLOCK_LAYERS && !st.found {
        let mut pend = mempool();
        pend.sort_by(paint_order);
        for (i, bx) in pend.iter().enumerate() {
            // `kb-chain-packet`: 1.8 s loop, 240 ms apart.
            let (alpha, drop) = if p.live {
                let t = (p.clock + i as f64 * 0.24).rem_euclid(1.8) / 1.8;
                if t < 0.35 {
                    (0.25, 0)
                } else if t < 0.7 {
                    (1.0, 4)
                } else {
                    (0.5, 8)
                }
            } else {
                (0.25, 0)
            };
            let f = faces(bx);
            let c = fg.alpha(fg.a * alpha);
            let mut side = vec![];
            outline(&f.side, &mut side);
            s = add(s, &runs_of(side), 0, drop, c.alpha(c.a * 0.5));
            let mut rest = vec![];
            outline(&f.top, &mut rest);
            outline(&f.front, &mut rest);
            s = add(s, &runs_of(rest), 0, drop, c);
        }
    }
    el().w(w)
        .h(h)
        .a11y_hidden()
        .child(canvas(s).size(w, h).a11y_hidden())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_deterministic_and_tiles_each_layer() {
        let a = chain_blocks(0, 3, 2, false);
        let b = chain_blocks(0, 3, 2, false);
        assert_eq!(a.len(), 4);
        assert_eq!(a[3].offset, 0);
        assert_eq!(
            a.iter().map(|b| b.boxes.len()).collect::<Vec<_>>(),
            b.iter().map(|b| b.boxes.len()).collect::<Vec<_>>()
        );
        // Each layer covers the 4×4 floor exactly.
        let mut next = random(7);
        let area: usize = layer_tiles(&mut next).iter().map(|t| t.2 * t.3).sum();
        assert_eq!(area, GRID * GRID);
        assert_eq!(mempool().len(), PENDING_COUNT);
        let f = frame(3);
        assert!(f[2] > 0.0 && f[3] > 0.0);
    }

    #[test]
    fn the_loop_fills_finds_and_advances() {
        let mut l = Loop::new(0.5);
        assert_eq!(l.layers, 2);
        l.advance(1.0);
        l.advance(2.0);
        assert_eq!(l.layers, 4);
        l.advance(3.0);
        assert!(l.found);
        l.advance(4.0);
        assert_eq!((l.tip, l.layers, l.found), (1, 0, false));
    }

    /// The redraw lands on each slide step after a block is found, not on a polling grid.
    #[test]
    fn the_next_change_follows_the_slide_steps() {
        let st = Loop {
            tip: 2,
            layers: 0,
            found: false,
            at: 10.0,
            due: 0.0,
        };
        let a = next_change(&st, 10.0);
        // The packets also step; the first change is no later than the first slide step.
        assert!(a > 10.0 && a <= 10.15 + 2e-4, "{a}");
        // Never past the next beat.
        assert!(next_change(&st, 10.85) <= 10.0 + TICK + 2e-4);
        // A landing layer's flicker asks for frames about every stagger step.
        let landing = Loop { layers: 2, ..st };
        let b = next_change(&landing, 10.05);
        assert!(b - 10.05 <= 0.0091, "{}", b - 10.05);
    }

    #[test]
    fn mulberry32_matches_the_reference() {
        // mulberry32(1): first output 0.6270739405881613 (the JS reference implementation).
        let mut r = random(1);
        assert!((r() - 0.627_073_940_588_161_3).abs() < 1e-12);
    }
}
