//! The chain as a still diorama on a dotted ground, the newest block half filled in its
//! outline. On the start page the only motion is a glint that sweeps the orange fee cubes every
//! few seconds. The lock screen draws the same chain encrypted, the blocks' outlines without
//! their contents, and the same glint runs along the outlines; while an unlock runs, a decrypt
//! scan reveals the real blocks oldest first.

use rune::El;

use super::{Faces, Iso, Next, Px, iso_box, iso_wire, orange, orange_faces, rng};
use crate::theme::Tk;

const B: f32 = 10.0;
const SLOT: f32 = 15.0;
const SLOTS: i32 = 6;
const PERIOD: f64 = 3.6;
const SWEEP: f64 = 0.7;
/// One pass of the decrypt scan over the chain while an unlock runs.
const SCAN: f64 = 1.6;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// The start page: the chain in the clear.
    Open,
    /// The lock screen: the chain encrypted; `busy` while an unlock runs.
    Locked { busy: bool },
}

/// When the picture next changes after `clock`: every frame while the glint sweeps or an
/// unlock's scan runs, otherwise at the next glint.
pub fn next(clock: f64, mode: Mode) -> Next {
    let phase = clock.rem_euclid(PERIOD);
    if phase < SWEEP || mode == (Mode::Locked { busy: true }) {
        Next::At(clock + 1.0 / 60.0)
    } else {
        Next::At(clock - phase + PERIOD + 1e-4)
    }
}

/// The glint's position across the scene, 0–1, while it sweeps.
fn glint(clock: f64) -> Option<f32> {
    let phase = clock.rem_euclid(PERIOD);
    (phase < SWEEP).then(|| (phase / SWEEP) as f32)
}

/// How many blocks (oldest first) the decrypt scan has revealed.
fn revealed(clock: f64, busy: bool) -> i32 {
    if !busy {
        return 0;
    }
    // A pass reveals the chain block by block, holds it whole, and starts again.
    let pass = (clock.rem_euclid(SCAN + 0.5) / SCAN).min(1.0);
    (pass * f64::from(SLOTS) + 0.999).floor() as i32
}

pub fn view(tk: &Tk, clock: f64, w: f32, h: f32) -> El {
    draw(tk, clock, w, h, Mode::Open)
}

pub fn draw(tk: &Tk, clock: f64, w: f32, h: f32, mode: Mode) -> El {
    let fg = tk.fg;
    let bg = tk.background;
    let cell = (w / 205.0).min(h / 125.0).floor().max(1.0);
    let mut px = Px::new(cell, 0.0, 0.0);
    let span_w = 2.0 * B + 2.0 * (B + (SLOTS - 1) as f32 * SLOT);
    let gx = (w / cell - span_w) / 2.0;
    let gy = (h / cell - 118.0) / 2.0;
    let iso = Iso {
        ox: gx + 2.0 * B,
        oy: gy + 96.0,
    };
    // The ground: a sparse dot grid under the chain.
    let dot = fg.alpha(0.14);
    let mut z = -(SLOTS as f32) * SLOT - 10.0;
    while z <= B + 12.0 {
        let mut x = -12.0;
        while x <= B + 14.0 {
            let (sx, sy) = iso.p(x, 0.0, z);
            px.rect(sx.floor() as i32, sy.floor() as i32, 1, 1, dot);
            x += 5.0;
        }
        z += 5.0;
    }
    // The glint runs at rest; an unlock's scan has the stage to itself.
    let band = (mode != Mode::Locked { busy: true })
        .then(|| glint(clock))
        .flatten()
        .map(|t| gx - 4.0 + t * (span_w + 8.0));
    let shine = Faces {
        top: rune::color::hex("#ffe2b8").expect("hex"),
        left: rune::color::hex("#fbab4a").expect("hex"),
        right: rune::color::hex("#f7931a").expect("hex"),
    };
    let (open_from, busy) = match mode {
        Mode::Open => (0, false),
        Mode::Locked { busy } => (revealed(clock, busy), busy),
    };
    // Oldest (front, left) to newest (back, right) is drawn back to front.
    for k in (0..SLOTS).rev() {
        let z0 = -(k as f32) * SLOT;
        let newest = k == SLOTS - 1;
        // Blocks count from the oldest (k = 0, front left): the scan reveals in chain order.
        let order = k;
        let shown = mode == Mode::Open || order < open_from;
        let mut next = rng(0xca1d + k as u32);
        let count = if newest { 5 } else { 8 };
        for i in 0..count {
            let layer = (i / 4) as f32;
            let c = i % 4;
            let (lx, ly, lz) = (
                0.5 + (c % 2) as f32 * 5.0,
                0.5 + layer * 4.5,
                0.5 + (c / 2) as f32 * 5.0,
            );
            let fee = next() < 0.22;
            if shown {
                let faces = if fee {
                    let (cx, _) = iso.p(lx + 2.0, ly, z0 + lz + 2.0);
                    match band {
                        Some(b) if (cx - b).abs() < 7.0 => shine,
                        _ => orange_faces(),
                    }
                } else {
                    Faces::shaded(fg)
                };
                iso_box(
                    &mut px,
                    iso,
                    (lx, ly, z0 + lz),
                    (4.0, 3.8, 4.0),
                    faces,
                    Some(bg),
                );
            }
        }
        if !shown {
            // Encrypted: the block's outline, orange while the glint passes it.
            let (cx, _) = iso.p(B / 2.0, 0.0, z0 + B / 2.0);
            let ink = match band {
                Some(b) if (cx - b).abs() < 10.0 => orange(),
                _ => fg.alpha(0.5),
            };
            iso_wire(&mut px, iso, (0.0, 0.0, z0), (B, B, B), ink);
        }
        if newest && shown {
            iso_wire(&mut px, iso, (0.0, 0.0, z0), (B, B, B), fg.alpha(0.45));
        }
        // The scan's edge: the block being decrypted now is outlined in orange.
        if busy && order == open_from - 1 && order < SLOTS {
            iso_wire(&mut px, iso, (0.0, 0.0, z0), (B, B, B), orange().alpha(0.9));
        }
    }
    px.el(w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Between glints the scene sleeps until the next one; during one it asks for frames.
    #[test]
    fn the_chain_sleeps_between_glints() {
        match next(1.0, Mode::Open) {
            Next::At(t) => assert!((t - PERIOD).abs() < 1e-3),
            _ => panic!(),
        }
        match next(PERIOD + 0.1, Mode::Open) {
            Next::At(t) => assert!(t - (PERIOD + 0.1) < 0.02),
            _ => panic!(),
        }
        assert!(glint(1.0).is_none() && glint(PERIOD + 0.35).is_some());
    }

    /// Locked, nothing is revealed at rest; an unlock's scan reveals the chain oldest first.
    #[test]
    fn the_decrypt_scan_reveals_in_chain_order() {
        assert_eq!(revealed(5.0, false), 0);
        assert!(revealed(0.05, true) >= 1);
        assert_eq!(revealed(SCAN + 0.1, true), SLOTS);
        // At rest the lock screen sleeps between glints like the start page; an unlock draws
        // every frame.
        match next(1.0, Mode::Locked { busy: false }) {
            Next::At(t) => assert!((t - PERIOD).abs() < 1e-3),
            _ => panic!(),
        }
        match next(1.0, Mode::Locked { busy: true }) {
            Next::At(t) => assert!(t - 1.0 < 0.02),
            _ => panic!(),
        }
    }
}
