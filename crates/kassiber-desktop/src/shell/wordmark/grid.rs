//! The pixel wordmark's grid and its frames (ui-tauri/src/components/kb/kassiberWordmarkGrid.ts),
//! ported value for value: the same cells, timings, easing, hash and rounding, so every frame
//! matches the web's at the same elapsed time. Frames are lists of unit-high runs of cells, the
//! web's SVG path data as rectangles.

/// Each `#` is one filled cell, so a letter is redrawn by editing its rows here.
pub const GRID: [&str; 18] = [
    "..#...#......#######....#######....#######....#..#######......#######....######.",
    ".##...##....########...########...########...##..########....########...########",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...###..###...###..###...##...###...##...###..###...###..###...##...###...###",
    "###..###...###...###..###...#....###...#....###..###...###..###...#....###...###",
    "###.###....###...###..###........###........###..###...##...###........###...###",
    "######....##########..########...########..####..########...#######...#########.",
    "######....##########...########...########.####..########...######....#######...",
    "###.###....###...###........###........###..###..###...##...###........###.###..",
    "###..###...###...###....#...###....#...###..###..###...###..###...#....###..###.",
    "###...###..###...###...##...###...##...###..###..###...###..###...##...###...###",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...##...###...##...########...########...##...########...########...###...###",
    "###...#....###...#....#######....#######....#....#######....#######....###...###",
    ".............................................................................##.",
    ".............................................................................#..",
];

pub const COLUMNS: usize = 80;
pub const ROWS: usize = 18;
/// The K's size in cells; the next column is the gap before the A.
pub const K_COLUMNS: usize = 9;
pub const K_ROWS: usize = 16;

/// How the word arrives. Decrypt settles flickering noise into the letters, left to right; rain
/// drops each cell into place; assemble gathers cells scattered across the mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveal {
    Decrypt,
    Rain,
    Assemble,
}

impl Reveal {
    /// `REVEAL_MS`.
    pub const fn ms(self) -> f64 {
        match self {
            Reveal::Decrypt => 1400.0,
            Reveal::Rain => 1600.0,
            Reveal::Assemble => 1800.0,
        }
    }
}

/// How often decrypt's noise reshuffles.
const FLICKER_MS: f64 = 70.0;
/// The working K's pause, whole, between decrypting in and scrambling out.
const WORKING_HOLD_MS: f64 = 500.0;
pub const WORKING_CYCLE_MS: f64 = 2.0 * Reveal::Decrypt.ms() + WORKING_HOLD_MS;

/// `w` cells from column `x` on row `y` (above the mark while rain falls, so `y` can be negative).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub x: i32,
    pub y: i32,
    pub w: i32,
}

/// Settled cells, drawn in the mark's colour; cells still moving or decoding, in orange.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub solid: Vec<Run>,
    pub noise: Vec<Run>,
}

fn filled(x: usize, y: usize) -> bool {
    GRID[y].as_bytes()[x] == b'#'
}

/// One run per horizontal stretch of the cells `include` picks in the first `columns` × `rows`.
fn cells(include: impl Fn(usize, usize) -> bool, columns: usize, rows: usize) -> Vec<Run> {
    let mut out = Vec::new();
    for y in 0..rows {
        let mut x = 0;
        while x < columns {
            if !include(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < columns && include(x, y) {
                x += 1;
            }
            out.push(Run {
                x: start as i32,
                y: y as i32,
                w: (x - start) as i32,
            });
        }
    }
    out
}

/// One square per cell, for cells off the grid while they move (`loosePath`).
fn loose(cells: &[(i32, i32)]) -> Vec<Run> {
    cells.iter().map(|&(x, y)| Run { x, y, w: 1 }).collect()
}

/// The whole word at rest.
pub fn word() -> Frame {
    Frame {
        solid: cells(filled, COLUMNS, ROWS),
        noise: vec![],
    }
}

/// The K at rest.
pub fn k() -> Frame {
    Frame {
        solid: cells(filled, K_COLUMNS, K_ROWS),
        noise: vec![],
    }
}

fn filled_cells() -> impl Iterator<Item = (usize, usize)> {
    (0..ROWS)
        .flat_map(|y| (0..COLUMNS).map(move |x| (x, y)))
        .filter(|&(x, y)| filled(x, y))
}

/// A stable pseudo-random value in [0, 1) for a cell and a seed: the web's `Math.imul` hash in
/// wrapping 32-bit arithmetic.
fn hash(x: i64, y: i64, seed: i64) -> f64 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u32).wrapping_mul(668_265_263))
        .wrapping_add((seed as u32).wrapping_mul(1_442_695_041));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    f64::from(h ^ (h >> 16)) / 4_294_967_296.0
}

fn clamp01(t: f64) -> f64 {
    t.clamp(0.0, 1.0)
}

fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powf(3.0)
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t.powf(3.0)
    } else {
        1.0 - (-2.0 * t + 2.0).powf(3.0) / 2.0
    }
}

/// `Math.round`: halves round up, toward +∞ (Rust's `round` takes -2.5 to -3).
fn js_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

/// The fraction of decrypt at which cell (x, y) settles, over `columns` columns.
fn settles_at(x: usize, y: usize, columns: usize) -> f64 {
    0.3 + (0.6 * x as f64) / columns as f64 + 0.08 * hash(x as i64, y as i64, 0)
}

/// Decrypt, `elapsed` ms in, over the first `columns` × `rows` cells. Columns settle left to
/// right with a little per-cell jitter; unsettled cells flicker.
pub fn decrypt(elapsed: f64, columns: usize, rows: usize) -> Frame {
    let t = elapsed / Reveal::Decrypt.ms();
    let tick = (elapsed / FLICKER_MS).floor() as i64 + 1;
    let settled = |x: usize, y: usize| t >= settles_at(x, y, columns);
    Frame {
        solid: cells(|x, y| filled(x, y) && settled(x, y), columns, rows),
        noise: cells(
            |x, y| {
                !settled(x, y)
                    && hash(x as i64, y as i64, tick) < if filled(x, y) { 0.55 } else { 0.1 }
            },
            columns,
            rows,
        ),
    }
}

/// Rain: cells fall from above the mark, in a ripple from the left.
fn rain(elapsed: f64) -> Frame {
    let t = elapsed / Reveal::Rain.ms();
    let (mut landed, mut falling) = (Vec::new(), Vec::new());
    for (x, y) in filled_cells() {
        let start = (x as f64 / COLUMNS as f64) * 0.45 + hash(x as i64, y as i64, 1) * 0.15;
        let p = clamp01((t - start) / 0.35);
        if p <= 0.0 {
            continue;
        }
        if p >= 1.0 {
            landed.push((x as i32, y as i32));
            continue;
        }
        falling.push((
            x as i32,
            js_round(-4.0 + (y as f64 + 4.0) * ease_out_cubic(p)),
        ));
    }
    Frame {
        solid: loose(&landed),
        noise: loose(&falling),
    }
}

/// Assemble: cells start scattered across the mark and gather into the word.
fn assemble(elapsed: f64) -> Frame {
    let t = elapsed / Reveal::Assemble.ms();
    let (mut home, mut moving) = (Vec::new(), Vec::new());
    for (x, y) in filled_cells() {
        let (xi, yi) = (x as i64, y as i64);
        let p = ease_in_out_cubic(clamp01((t - hash(xi, yi, 4) * 0.3) / 0.6));
        if p >= 1.0 {
            home.push((x as i32, y as i32));
            continue;
        }
        let from_x = hash(xi, yi, 2) * (COLUMNS - 1) as f64;
        let from_y = hash(xi, yi, 3) * (ROWS - 1) as f64;
        moving.push((
            js_round(from_x + (x as f64 - from_x) * p),
            js_round(from_y + (y as f64 - from_y) * p),
        ));
    }
    Frame {
        solid: loose(&home),
        noise: loose(&moving),
    }
}

/// One frame of `reveal`, `elapsed` ms in; past its length, the plain word.
pub fn reveal(reveal: Reveal, elapsed: f64) -> Frame {
    if elapsed >= reveal.ms() {
        return word();
    }
    match reveal {
        Reveal::Rain => rain(elapsed),
        Reveal::Assemble => assemble(elapsed),
        Reveal::Decrypt => decrypt(elapsed, COLUMNS, ROWS),
    }
}

/// Where in decrypt the working K stands `elapsed` ms into its loop: decrypting in, holding
/// whole, then scrambling back out (decrypt played backwards).
fn working_at(elapsed: f64) -> f64 {
    let at = elapsed % WORKING_CYCLE_MS;
    let d = Reveal::Decrypt.ms();
    if at < d {
        at
    } else if at < d + WORKING_HOLD_MS {
        d
    } else {
        WORKING_CYCLE_MS - at
    }
}

/// The K's working loop: decrypt in, hold, scramble back out, repeat.
pub fn working(elapsed: f64) -> Frame {
    decrypt(working_at(elapsed), K_COLUMNS, K_ROWS)
}

/// A hair past an edge, so the frame there is already the next one however `t >= …` rounds.
const PAST: f64 = 1e-6;

/// When decrypt over `columns` × `rows`, `at` ms in, next draws differently: going forward, the
/// first time past `at` (the next flicker tick or a cell settling); backward, the decrypt time
/// below `at` where it last changed (the tick it is in, or the latest cell to have settled).
fn decrypt_change(at: f64, columns: usize, rows: usize, forward: bool) -> f64 {
    let d = Reveal::Decrypt.ms();
    let t = at / d;
    let mut next = if forward {
        ((at / FLICKER_MS).floor() + 1.0) * FLICKER_MS + PAST
    } else {
        (at / FLICKER_MS).floor() * FLICKER_MS
    };
    for y in 0..rows {
        for x in 0..columns {
            // Settled exactly as `decrypt` decides it, so a cell on its edge is never skipped.
            let s = settles_at(x, y, columns);
            if forward && t < s {
                next = next.min(s * d + PAST);
            } else if !forward && t >= s {
                next = next.max(s * d);
            }
        }
    }
    next
}

/// The elapsed time, after `elapsed`, at which a reveal's or the working K's frame next changes
/// (None: it may change on every frame). Waking then, and not on every display frame, keeps the
/// poll and the view idle while nothing moves: the flicker's 70 ms ticks, the hold.
pub fn next_change(motion: Motion, elapsed: f64) -> Option<f64> {
    match motion {
        Motion::Reveal(Reveal::Decrypt) => {
            Some(decrypt_change(elapsed, COLUMNS, ROWS, true).min(Reveal::Decrypt.ms()))
        }
        Motion::Reveal(_) => None,
        Motion::Working => {
            let d = Reveal::Decrypt.ms();
            let cycle = (elapsed / WORKING_CYCLE_MS).floor() * WORKING_CYCLE_MS;
            let at = elapsed - cycle;
            Some(if at < d {
                cycle + decrypt_change(at, K_COLUMNS, K_ROWS, true).min(d)
            } else if at < d + WORKING_HOLD_MS {
                cycle + d + WORKING_HOLD_MS
            } else {
                // Scrambling out, decrypt runs backwards: its next edge below, just past it.
                let back = decrypt_change(WORKING_CYCLE_MS - at, K_COLUMNS, K_ROWS, false);
                (cycle + WORKING_CYCLE_MS - back)
                    .min(cycle + WORKING_CYCLE_MS)
                    .max(elapsed)
                    + 0.01
            })
        }
    }
}

/// What plays at a mark: a one-shot reveal of the word, or the K's endless working loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Reveal(Reveal),
    Working,
}

impl Motion {
    pub fn frame(self, elapsed: f64) -> Frame {
        match self {
            Motion::Reveal(r) => reveal(r, elapsed),
            Motion::Working => working(elapsed),
        }
    }

    pub fn one_shot(self) -> bool {
        matches!(self, Motion::Reveal(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The web's SVG path data for a frame's runs (`cellsPath`), or, for loose cells, one square
    /// each (`loosePath`), so frames compare with the React module's own output.
    fn path(runs: &[Run], loose: bool) -> String {
        runs.iter()
            .map(|r| {
                if loose {
                    format!("M{} {}h1v1h-1z", r.x, r.y)
                } else {
                    format!("M{} {}h{}v1h{}z", r.x, r.y, r.w, -r.w)
                }
            })
            .collect()
    }

    fn fnv(s: &str) -> u64 {
        s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    fn fingerprint(f: &Frame, loose: bool) -> (u64, u64) {
        (fnv(&path(&f.solid, loose)), fnv(&path(&f.noise, loose)))
    }

    /// FNV-1a 64 of the solid and noise path strings that kassiberWordmarkGrid.ts returns at
    /// these times (generated with node from the web module; see the wordmark worker's report).
    const GOLDEN: &[(&str, f64, u64, u64)] = &include!("golden.in");

    #[test]
    fn frames_match_the_web_module() {
        for &(what, at, solid, noise) in GOLDEN {
            let (f, loose) = match what {
                "decrypt" => (reveal(Reveal::Decrypt, at), false),
                "rain" => (reveal(Reveal::Rain, at), at < Reveal::Rain.ms()),
                "assemble" => (reveal(Reveal::Assemble, at), at < Reveal::Assemble.ms()),
                "working" => (working(at), false),
                _ => unreachable!(),
            };
            assert_eq!(fingerprint(&f, loose), (solid, noise), "{what} at {at} ms");
        }
    }

    /// The README's wordmarks (`docs/assets/`) are drawn from this grid.
    #[test]
    fn readme_wordmarks_are_drawn_from_this_grid() {
        let d = format!("d=\"{}\"", path(&word().solid, false));
        for variant in ["on-light", "on-dark"] {
            let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../docs/assets/kassiber-wordmark-{variant}.svg"));
            let svg = std::fs::read_to_string(&file).unwrap();
            assert!(
                svg.contains(&d),
                "{} is not drawn from the grid",
                file.display()
            );
        }
    }

    #[test]
    fn rows_are_even_and_the_k_column_gap_is_empty() {
        for row in GRID {
            assert_eq!(row.len(), COLUMNS);
            assert!(row.bytes().all(|b| b == b'#' || b == b'.'));
            assert_eq!(row.as_bytes()[K_COLUMNS], b'.');
        }
        for row in &GRID[K_ROWS..] {
            assert!(!row[..K_COLUMNS].contains('#'));
        }
        assert!(!k().solid.is_empty());
    }

    #[test]
    fn reveals_start_empty_and_end_on_the_word() {
        for r in [Reveal::Decrypt, Reveal::Rain, Reveal::Assemble] {
            assert!(reveal(r, 0.0).solid.is_empty(), "{r:?}");
            assert_eq!(reveal(r, r.ms()), word());
        }
        assert!(!decrypt(0.0, COLUMNS, ROWS).noise.is_empty());
    }

    #[test]
    fn the_working_k_loops_between_scrambled_and_whole_inside_its_box() {
        assert!(working(0.0).solid.is_empty());
        assert_eq!(working(Reveal::Decrypt.ms()), k());
        assert_eq!(working(WORKING_CYCLE_MS), working(0.0));
        let mut at = 0.0;
        while at < WORKING_CYCLE_MS {
            let f = working(at);
            assert!(
                f.solid
                    .iter()
                    .chain(&f.noise)
                    .all(|r| r.y < K_ROWS as i32 && r.x + r.w <= K_COLUMNS as i32)
            );
            at += 70.0;
        }
    }

    /// Between now and the next change the frame holds, so waking only then misses nothing.
    #[test]
    fn nothing_changes_before_the_next_change() {
        for motion in [Motion::Reveal(Reveal::Decrypt), Motion::Working] {
            let end = if motion == Motion::Working {
                2.0 * WORKING_CYCLE_MS
            } else {
                Reveal::Decrypt.ms()
            };
            let mut at = 0.0;
            while at < end {
                let next = next_change(motion, at).expect("decrypt-based");
                assert!(next > at, "{motion:?} at {at}: next {next}");
                // The stretch before the change, on a 1 ms grid (a backward edge lands just
                // before its wake).
                let mut s = at;
                while s < next - 0.02 {
                    assert_eq!(
                        motion.frame(s),
                        motion.frame(at),
                        "{motion:?} changed at {s} before {next} (from {at})"
                    );
                    s += 1.0;
                }
                at = next;
            }
        }
        // The opening 420 ms only flicker, every 70 ms; the hold sleeps through.
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-3);
        assert!(near(
            next_change(Motion::Reveal(Reveal::Decrypt), 0.0),
            70.0
        ));
        assert!(near(next_change(Motion::Working, 1500.0), 1900.0));
        assert_eq!(next_change(Motion::Reveal(Reveal::Rain), 10.0), None);
    }
}
