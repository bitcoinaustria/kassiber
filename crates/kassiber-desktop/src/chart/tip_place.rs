//! Where the chart's hover card goes in the window, as a trading chart places its tooltip:
//! right-below of the pointer; left of it when the right side would cross the window's right
//! edge; above it when below would cross the bottom; clamped inside the window when it fits
//! neither above nor below. It always stays `MARGIN` points inside the window, and keeps clear
//! of the pointer's own point (the hovered dot) wherever the window leaves room beside it.

/// Recharts' tooltip offset from the pointer, on both axes.
pub const GAP: f32 = 32.0;
/// How far the card stays inside the window's edges.
pub const MARGIN: f32 = 8.0;

/// The crosshair's date pill under the plot, in window points: the card keeps `clear` points
/// beside the pointer (instead of `GAP`) when its vertical span reaches the pill's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PillBand {
    pub top: f32,
    pub bottom: f32,
    pub clear: f32,
}

/// The card's top-left in window points, for a card of `size` (w, h) next to pointer `p`
/// (window points) in a `window` (w, h).
pub fn place(p: [f32; 2], size: [f32; 2], window: [f32; 2], pill: Option<PillBand>) -> [f32; 2] {
    let [px, py] = p;
    let [w, h] = size;
    let [ww, wh] = window;
    // Vertical: below, else above, else as low as the window lets it (beside the pointer).
    let below = py + GAP;
    let above = py - GAP - h;
    let y = if below + h <= wh - MARGIN {
        below
    } else if above >= MARGIN {
        above
    } else {
        // Fits neither: pinned to the window's bottom margin (top margin when taller than the
        // window), so it covers as little above the pointer as it can.
        (wh - MARGIN - h).max(MARGIN)
    };
    // The date pill under the plot: a card reaching down to it hangs further aside.
    let gap = match pill {
        Some(b) if y + h >= b.top && y <= b.bottom => GAP.max(b.clear),
        _ => GAP,
    };
    // Horizontal: right, else left, else clamped on the side with more room.
    let right = px + gap;
    let left = px - gap - w;
    let x = if right + w <= ww - MARGIN {
        right
    } else if left >= MARGIN {
        left
    } else {
        let start = if ww - px >= px { right } else { left };
        start.min(ww - MARGIN - w).max(MARGIN)
    };
    [x, y]
}

/// How many of a merged dot's `members` the event card lists so the card stays inside the
/// window: `want` (the 8 it lists at most), fewer when the window is short. `base` is the
/// card's height without its event list and `list_h(rows, more)` the list's height with
/// `rows` rows and, when `more`, the "+ N more events" line; at least one row stays.
pub fn rows_that_fit(
    want: usize,
    members: usize,
    base: f32,
    window_h: f32,
    list_h: impl Fn(usize, bool) -> f32,
) -> usize {
    let room = window_h - 2.0 * MARGIN - base;
    let want = want.min(members);
    (1..=want)
        .rev()
        .find(|&n| list_h(n, members > n) <= room)
        .unwrap_or(1.min(want))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: [f32; 2] = [1440.0, 900.0];
    const CARD: [f32; 2] = [300.0, 250.0];

    fn inside(r: [f32; 2], size: [f32; 2], win: [f32; 2]) -> bool {
        r[0] >= MARGIN
            && r[1] >= MARGIN
            && r[0] + size[0] <= win[0] - MARGIN
            && r[1] + size[1] <= win[1] - MARGIN
    }

    fn covers(r: [f32; 2], size: [f32; 2], p: [f32; 2]) -> bool {
        p[0] >= r[0] && p[0] <= r[0] + size[0] && p[1] >= r[1] && p[1] <= r[1] + size[1]
    }

    #[test]
    fn right_below_when_it_fits() {
        assert_eq!(place([400.0, 300.0], CARD, WIN, None), [432.0, 332.0]);
    }

    #[test]
    fn top_left_corner_stays_right_below() {
        let r = place([10.0, 10.0], CARD, WIN, None);
        assert_eq!(r, [42.0, 42.0]);
        assert!(inside(r, CARD, WIN));
    }

    #[test]
    fn flips_left_at_the_right_edge() {
        let r = place([1300.0, 300.0], CARD, WIN, None);
        assert_eq!(r, [1300.0 - 32.0 - 300.0, 332.0]);
        assert!(inside(r, CARD, WIN));
    }

    #[test]
    fn flips_above_at_the_bottom_edge() {
        let r = place([400.0, 800.0], CARD, WIN, None);
        assert_eq!(r, [432.0, 800.0 - 32.0 - 250.0]);
        assert!(inside(r, CARD, WIN));
    }

    #[test]
    fn bottom_right_corner_flips_both_ways() {
        let p = [1420.0, 880.0];
        let r = place(p, CARD, WIN, None);
        assert_eq!(r, [1420.0 - 332.0, 880.0 - 282.0]);
        assert!(inside(r, CARD, WIN));
        assert!(!covers(r, CARD, p));
    }

    #[test]
    fn exactly_at_the_margin_still_fits() {
        // Right edge lands exactly on the margin: no flip.
        let r = place(
            [1440.0 - 8.0 - 300.0 - 32.0, 900.0 - 8.0 - 250.0 - 32.0],
            CARD,
            WIN,
            None,
        );
        assert_eq!(r, [1440.0 - 8.0 - 300.0, 900.0 - 8.0 - 250.0]);
    }

    #[test]
    fn neither_above_nor_below_clamps_inside_beside_the_pointer() {
        // A 700 pt window and a 500 pt card with the pointer in the middle.
        let win = [1000.0, 700.0];
        let card = [300.0, 500.0];
        // Below fits up to y 160, above from y 540.
        for py in [161.0, 350.0, 539.0] {
            let p = [300.0, py];
            let r = place(p, card, win, None);
            assert!(inside(r, card, win), "{py}: {r:?}");
            assert_eq!(r[1], 700.0 - 8.0 - 500.0);
            // Beside the pointer, so the hovered dot stays visible.
            assert!(!covers(r, card, p));
            assert_eq!(r[0], 332.0);
        }
    }

    #[test]
    fn taller_than_the_window_pins_to_the_top_margin() {
        let r = place([300.0, 300.0], [300.0, 800.0], [1000.0, 700.0], None);
        assert_eq!(r[1], MARGIN);
    }

    #[test]
    fn narrow_window_clamps_horizontally() {
        // Neither side has 32 + 300 + 8 pt: the side with more room, kept inside.
        let win = [500.0, 700.0];
        let r = place([200.0, 100.0], CARD, win, None);
        assert!(inside(r, CARD, win), "{r:?}");
        let r = place([300.0, 100.0], CARD, win, None);
        assert!(inside(r, CARD, win), "{r:?}");
    }

    #[test]
    fn narrow_window_right_side_with_room_flips_left() {
        let win = [700.0, 700.0];
        let r = place([600.0, 100.0], CARD, win, None);
        assert_eq!(r[0], 600.0 - 332.0);
    }

    #[test]
    fn pill_widens_the_gap_only_when_the_card_reaches_it() {
        let pill = Some(PillBand {
            top: 600.0,
            bottom: 615.0,
            clear: 70.0,
        });
        // Reaches down past the pill: 70 pt aside.
        assert_eq!(place([400.0, 400.0], CARD, WIN, pill)[0], 470.0);
        // Ends above the pill: Recharts' 32.
        assert_eq!(place([400.0, 200.0], CARD, WIN, pill)[0], 432.0);
        // Flipped left, the wider gap applies on the left.
        assert_eq!(
            place([1300.0, 400.0], CARD, WIN, pill)[0],
            1300.0 - 70.0 - 300.0
        );
        // A narrower clearance never pulls it closer than 32.
        let small = Some(PillBand {
            top: 600.0,
            bottom: 615.0,
            clear: 10.0,
        });
        assert_eq!(place([400.0, 400.0], CARD, WIN, small)[0], 432.0);
    }

    #[test]
    fn pill_after_flipping_above_is_ignored() {
        // The pointer near the bottom: the card flips above and clears the pill under the plot.
        let pill = Some(PillBand {
            top: 860.0,
            bottom: 875.0,
            clear: 80.0,
        });
        let r = place([400.0, 840.0], CARD, WIN, pill);
        assert_eq!(r, [432.0, 840.0 - 282.0]);
    }

    #[test]
    fn every_pointer_position_stays_inside() {
        for win in [[1440.0, 900.0], [1000.0, 700.0], [800.0, 600.0]] {
            for size in [[262.0, 124.0], [300.0, 250.0], [320.0, 420.0]] {
                let mut x = 0.0;
                while x <= win[0] {
                    let mut y = 0.0;
                    while y <= win[1] {
                        let r = place([x, y], size, win, None);
                        assert!(inside(r, size, win), "{win:?} {size:?} at {x},{y}: {r:?}");
                        y += 25.0;
                    }
                    x += 25.0;
                }
            }
        }
    }

    #[test]
    fn rows_cap_to_the_window() {
        // 14 pt rows, a 20 pt title and 14 pt more-line; 300 pt of the rest of the card.
        let list = |n: usize, more: bool| 20.0 + n as f32 * 14.0 + if more { 14.0 } else { 0.0 };
        // Plenty of room: all 8.
        assert_eq!(rows_that_fit(8, 49, 300.0, 900.0, list), 8);
        // Fewer members than 8: all of them.
        assert_eq!(rows_that_fit(8, 3, 300.0, 900.0, list), 3);
        // 500 pt window: room = 484 - 300 = 184 → 20 + 14n + 14 ≤ 184 → n = 10 → capped at 8.
        assert_eq!(rows_that_fit(8, 49, 300.0, 500.0, list), 8);
        // 420 pt window: room = 104 → 14n ≤ 70 → n = 5.
        assert_eq!(rows_that_fit(8, 49, 300.0, 420.0, list), 5);
        // No room at all: one row stays.
        assert_eq!(rows_that_fit(8, 49, 300.0, 200.0, list), 1);
        // Exactly 8 members fit only without the more-line: 20 + 8 * 14 = 132 ≤ 134.
        assert_eq!(rows_that_fit(8, 8, 300.0, 300.0 + 16.0 + 134.0, list), 8);
        // 9 members need the more-line: 8 rows = 146 > 134 → 7.
        assert_eq!(rows_that_fit(8, 9, 300.0, 300.0 + 16.0 + 134.0, list), 7);
    }
}
