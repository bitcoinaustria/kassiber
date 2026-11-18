//! The start page's and the lock screen's artwork: the chain as a pixel diorama ([`calm`]).
//! Pure decoration: nothing drawn here is a real block, fee, transaction or amount.

pub mod calm;

use std::time::Duration;

use rune::{Color, El, Shape, Tx, canvas, el};

use crate::app::App;

/// When the artwork on screen next changes.
pub enum Next {
    At(f64),
    Never,
}

/// Redraws the artwork on screen exactly when it changes, and sleeps until then (a wake alone
/// draws nothing new).
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let clock = app.clock;
    let next = if crate::onboarding::shows(app)
        && app.onboarding.flow == crate::onboarding::Flow::Start
        && app.onboarding.import_selection.is_none()
    {
        calm::next(clock, calm::Mode::Open)
    } else if app.locked {
        let busy = app.lock.submitting || app.lock.touch_submitting;
        calm::next(clock, calm::Mode::Locked { busy })
    } else {
        Next::Never
    };
    match next {
        Next::Never => false,
        Next::At(t) => {
            let due = app.art_due;
            app.art_due = t;
            tx.wake_after(Duration::from_secs_f64((t - clock).max(0.0)));
            clock >= due
        }
    }
}

/// The lock screen without a book behind it: an identity stands in, so the shell (not
/// onboarding) draws, and the lock covers its page panel.
fn locked(a: &mut App) {
    a.onboarding.identity = Some(crate::onboarding::model::Identity {
        name: "Satoshi".into(),
        encrypted: true,
        ..Default::default()
    });
    a.encrypted = true;
    a.locked = true;
}

pub static SCENES: &[crate::app::Scene] = &[
    ("art-lock", locked),
    ("art-lock-attempt", |a| {
        locked(a);
        a.lock.attempt = 1;
        a.lock.submitting = true;
    }),
    // A refused passphrase: the error line under the field.
    ("art-lock-error", |a| {
        locked(a);
        a.lock.attempt = 1;
        a.lock.error = Some(crate::i18n::t("chrome:lock.passphraseFailed"));
    }),
];

// ---- A pixel canvas -------------------------------------------------------------------------

/// A canvas in whole cells: every mark lands on the cell grid, so shapes step like pixel art.
pub struct Px {
    s: Shape,
    pub cell: f32,
    pub ox: f32,
    pub oy: f32,
}

impl Px {
    pub fn new(cell: f32, ox: f32, oy: f32) -> Px {
        Px {
            s: Shape::new(),
            cell,
            ox,
            oy,
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        if w <= 0 || h <= 0 || c.a <= 0.0 {
            return;
        }
        let x0 = self.ox + x as f32 * self.cell;
        let y0 = self.oy + y as f32 * self.cell;
        let x1 = x0 + w as f32 * self.cell;
        let y1 = y0 + h as f32 * self.cell;
        let s = std::mem::replace(&mut self.s, Shape::new());
        self.s = s.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], c);
    }

    /// A polygon (cell coordinates) filled cell by cell: a cell is in when its centre is.
    pub fn poly(&mut self, pts: &[(f32, f32)], c: Color) {
        if pts.len() < 3 || c.a <= 0.0 {
            return;
        }
        let min_y = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32;
        let max_y = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32;
        let mut xs = Vec::with_capacity(8);
        for y in min_y..max_y {
            let yc = y as f32 + 0.5;
            xs.clear();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a.1 <= yc && b.1 > yc) || (b.1 <= yc && a.1 > yc) {
                    xs.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks(2) {
                if let [xa, xb] = pair {
                    let from = (xa - 0.5).ceil() as i32;
                    let to = (xb - 0.5).ceil() as i32;
                    self.rect(from, y, to - from, 1, c);
                }
            }
        }
    }

    /// A one-cell line from `a` to `b` (cell coordinates); `dash` skips every other pair.
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), c: Color, dash: bool) {
        let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil().max(1.0) as i32;
        let mut last = None;
        for i in 0..=steps {
            if dash && (i / 2) % 2 == 1 {
                continue;
            }
            let t = i as f32 / steps as f32;
            let cell = (
                (a.0 + (b.0 - a.0) * t).floor() as i32,
                (a.1 + (b.1 - a.1) * t).floor() as i32,
            );
            if last != Some(cell) {
                self.rect(cell.0, cell.1, 1, 1, c);
                last = Some(cell);
            }
        }
    }

    pub fn el(self, w: f32, h: f32) -> El {
        el().w(w)
            .h(h)
            .a11y_hidden()
            .child(canvas(self.s).size(w, h).a11y_hidden())
    }
}

// ---- Isometric boxes ------------------------------------------------------------------------

/// World (x right-down, y up, z left-down) to cells: 2:1 isometric, `(ox, oy)` the origin.
#[derive(Clone, Copy)]
pub struct Iso {
    pub ox: f32,
    pub oy: f32,
}

impl Iso {
    pub fn p(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        (self.ox + 2.0 * (x - z), self.oy + (x + z) - 2.0 * y)
    }
}

/// A box's three visible faces' colours: top, the left (front) face and the right side.
#[derive(Clone, Copy)]
pub struct Faces {
    pub top: Color,
    pub left: Color,
    pub right: Color,
}

impl Faces {
    /// The artwork's shading: the front full, the top lighter, the side dim.
    pub fn shaded(c: Color) -> Faces {
        Faces {
            top: c.alpha(c.a * 0.7),
            left: c,
            right: c.alpha(c.a * 0.4),
        }
    }
}

pub fn orange() -> Color {
    rune::color::hex("#f7931a").expect("hex")
}

pub fn orange_faces() -> Faces {
    Faces {
        top: rune::color::hex("#fbab4a").expect("hex"),
        left: orange(),
        right: rune::color::hex("#d97d0e").expect("hex"),
    }
}

/// A solid box at `(x, y, z)` sized `w × h × d`, its faces filled and, with `edge`, outlined.
#[allow(clippy::too_many_arguments)]
pub fn iso_box(
    px: &mut Px,
    iso: Iso,
    (x, y, z): (f32, f32, f32),
    (w, h, d): (f32, f32, f32),
    faces: Faces,
    edge: Option<Color>,
) {
    let p = |x, y, z| iso.p(x, y, z);
    let top = [
        p(x, y + h, z),
        p(x + w, y + h, z),
        p(x + w, y + h, z + d),
        p(x, y + h, z + d),
    ];
    let left = [
        p(x, y, z + d),
        p(x + w, y, z + d),
        p(x + w, y + h, z + d),
        p(x, y + h, z + d),
    ];
    let right = [
        p(x + w, y, z),
        p(x + w, y, z + d),
        p(x + w, y + h, z + d),
        p(x + w, y + h, z),
    ];
    px.poly(&right, faces.right);
    px.poly(&left, faces.left);
    px.poly(&top, faces.top);
    if let Some(e) = edge {
        for f in [&top, &left, &right] {
            for i in 0..4 {
                px.line(f[i], f[(i + 1) % 4], e, false);
            }
        }
    }
}

/// A box's wire outline: the visible edges solid, the hidden ones dashed.
pub fn iso_wire(
    px: &mut Px,
    iso: Iso,
    (x, y, z): (f32, f32, f32),
    (w, h, d): (f32, f32, f32),
    ink: Color,
) {
    let p = |x, y, z| iso.p(x, y, z);
    let solid = [
        (p(x, y + h, z), p(x + w, y + h, z)),
        (p(x + w, y + h, z), p(x + w, y + h, z + d)),
        (p(x + w, y + h, z + d), p(x, y + h, z + d)),
        (p(x, y + h, z + d), p(x, y + h, z)),
        (p(x, y, z + d), p(x + w, y, z + d)),
        (p(x + w, y, z + d), p(x + w, y, z)),
        (p(x, y, z + d), p(x, y + h, z + d)),
        (p(x + w, y, z + d), p(x + w, y + h, z + d)),
        (p(x + w, y, z), p(x + w, y + h, z)),
    ];
    for (a, b) in solid {
        px.line(a, b, ink, false);
    }
    let hidden = ink.alpha(ink.a * 0.5);
    for (a, b) in [
        (p(x, y, z), p(x + w, y, z)),
        (p(x, y, z), p(x, y, z + d)),
        (p(x, y, z), p(x, y + h, z)),
    ] {
        px.line(a, b, hidden, true);
    }
}

/// mulberry32, as the chain artwork seeds its layouts.
pub fn rng(seed: u32) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x6d2b_79f5);
        let mut t = state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}
