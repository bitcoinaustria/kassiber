//! The layout's rules: packing, reordering, size changes, adding and removing, and what a stored
//! document may contain.

use serde_json::json;

use super::model::{Cell, Layout, Size, Tile, WidgetId as W, pack, rows};
use super::{Geo, drop_target};

fn t(id: W, size: Size) -> Tile {
    Tile { id, size }
}

fn at(cells: &[Cell], id: W) -> (u8, u16) {
    let c = cells.iter().find(|c| c.id == id).expect("placed");
    (c.x, c.y)
}

fn overlap(cells: &[Cell]) -> bool {
    cells.iter().enumerate().any(|(i, a)| {
        cells.iter().skip(i + 1).any(|b| {
            a.x < b.x + b.w
                && b.x < a.x + a.w
                && a.y < b.y + u16::from(b.h)
                && b.y < a.y + u16::from(a.h)
        })
    })
}

#[test]
fn default_layout_is_the_owners_arrangement() {
    let l = Layout::default();
    let got: Vec<(W, Size)> = l.tiles().iter().map(|t| (t.id, t.size)).collect();
    assert_eq!(
        got,
        vec![
            (W::Stats, Size::Medium),
            (W::TaxYear, Size::Medium),
            (W::Chart, Size::XLarge),
            (W::Recent, Size::Large),
            (W::Holdings, Size::Large)
        ]
    );
    assert_eq!(
        l.unplaced(),
        vec![W::Flows, W::Assets, W::Attention, W::Age, W::Goal]
    );
    assert_eq!(l.goal_sats(), None);
    for w in W::ALL {
        assert!(w.supports(w.default_size()), "{w:?}");
        assert!(!w.sizes().is_empty());
        assert_eq!(W::from_id(w.id()), Some(w), "{w:?} round-trips its id");
    }
    for w in [W::Attention, W::Age, W::Goal] {
        assert_eq!(w.sizes(), &[Size::Small, Size::Medium], "{w:?}");
    }
}

#[test]
fn the_default_stacks_the_right_column_on_six_columns() {
    // The owner's screen: Key figures and Tax year on top, the chart under them, Recent
    // transactions over Holdings by Source in the right column.
    let cells = pack(Layout::default().tiles(), 6);
    assert_eq!(at(&cells, W::Stats), (0, 0));
    assert_eq!(at(&cells, W::TaxYear), (2, 0));
    assert_eq!(at(&cells, W::Recent), (4, 0));
    assert_eq!(at(&cells, W::Chart), (0, 1));
    assert_eq!(at(&cells, W::Holdings), (4, 2));
    assert_eq!(rows(&cells), 4);
    assert!(!overlap(&cells));
}

#[test]
fn packing_fills_gaps_in_reading_order_without_overlap() {
    let cells = pack(Layout::default().tiles(), 4);
    // Four columns: no room for a right column, so Recent and Holdings share a row under the chart.
    assert_eq!(at(&cells, W::Stats), (0, 0));
    assert_eq!(at(&cells, W::TaxYear), (2, 0));
    assert_eq!(at(&cells, W::Chart), (0, 1));
    assert_eq!(at(&cells, W::Recent), (0, 3));
    assert_eq!(at(&cells, W::Holdings), (2, 3));
    assert_eq!(rows(&cells), 5);
    assert!(!overlap(&cells));
}

#[test]
fn packing_on_two_columns_keeps_the_order_and_clamps_the_wide_tile() {
    let cells = pack(Layout::default().tiles(), 2);
    let chart = cells.iter().find(|c| c.id == W::Chart).unwrap();
    assert_eq!((chart.w, chart.h), (2, 2));
    assert_eq!(at(&cells, W::Stats), (0, 0));
    assert_eq!(at(&cells, W::TaxYear), (0, 1));
    assert_eq!(at(&cells, W::Chart), (0, 2));
    assert_eq!(at(&cells, W::Recent), (0, 4));
    assert_eq!(at(&cells, W::Holdings), (0, 6));
    assert!(!overlap(&cells));
    assert!(cells.iter().all(|c| c.x + c.w <= 2));
}

#[test]
fn the_default_at_the_four_window_widths() {
    // The grid's width at each window (sidebar open; at 980 pt it collapses), less the page's
    // 0.75 rem insets.
    let tk = crate::theme::Tk::new(true, 1.0);
    for (window, width, cols) in [
        (1440, 1152.0, 4u8),
        (1728, 1440.0, 4),
        (1920, 1632.0, 6),
        (980, 900.0, 2),
    ] {
        let geo = Geo::new(&tk, width);
        assert_eq!(geo.cols, cols, "{window} pt");
        let mut cells = pack(Layout::default().tiles(), geo.cols);
        cells.sort_by_key(|c| (c.y, c.x));
        assert!(!overlap(&cells));
        let order: Vec<W> = cells.iter().map(|c| c.id).collect();
        match cols {
            6 => assert_eq!(
                order,
                vec![W::Stats, W::TaxYear, W::Recent, W::Chart, W::Holdings],
                "{window}: right column"
            ),
            _ => assert_eq!(
                order,
                vec![W::Stats, W::TaxYear, W::Chart, W::Recent, W::Holdings],
                "{window}: list order"
            ),
        }
    }
    // With the sidebar collapsed, 1728 pt has room for six.
    assert_eq!(Geo::new(&tk, 1650.0).cols, 6);
}

#[test]
fn small_tiles_share_a_row_and_never_overlap_in_any_order() {
    let tiles = [
        t(W::Stats, Size::Small),
        t(W::Recent, Size::Large),
        t(W::TaxYear, Size::Small),
        t(W::Assets, Size::Small),
        t(W::Flows, Size::Medium),
        t(W::Chart, Size::XLarge),
    ];
    for cols in [1u8, 2, 3, 4, 6] {
        let cells = pack(&tiles, cols);
        assert_eq!(cells.len(), tiles.len());
        assert!(!overlap(&cells), "{cols} columns");
        assert!(cells.iter().all(|c| c.x + c.w <= cols), "{cols} columns");
    }
    let cells = pack(&tiles, 4);
    assert_eq!(at(&cells, W::Stats), (0, 0));
    assert_eq!(at(&cells, W::Recent), (1, 0));
    assert_eq!(at(&cells, W::TaxYear), (3, 0));
    // The next small tile drops into the gap under the first one.
    assert_eq!(at(&cells, W::Assets), (0, 1));
    assert_eq!(at(&cells, W::Flows), (0, 2));
    assert_eq!(pack(&[], 4), vec![]);
}

#[test]
fn reorder_moves_one_widget_and_keeps_the_rest_in_order() {
    let mut l = Layout::default();
    assert!(l.move_to(W::Holdings, 0));
    let ids: Vec<W> = l.tiles().iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        vec![W::Holdings, W::Stats, W::TaxYear, W::Chart, W::Recent]
    );
    assert!(!l.move_to(W::Holdings, 0), "already there");
    assert!(l.move_to(W::TaxYear, 99), "clamped to the end");
    assert_eq!(l.index_of(W::TaxYear), Some(4));
    assert!(!l.move_to(W::Flows, 0), "not placed");
    assert!(l.shift(W::Chart, -1));
    assert!(l.shift(W::Chart, -1));
    assert_eq!(l.index_of(W::Chart), Some(0));
    assert!(!l.shift(W::Chart, -1), "at the start");
    assert!(!l.shift(W::TaxYear, 1), "at the end");
    assert_eq!(l.len(), 5);
}

#[test]
fn sizes_only_change_to_a_supported_one() {
    let mut l = Layout::default();
    assert!(!l.set_size(W::Chart, Size::Small));
    assert_eq!(l.size_of(W::Chart), Some(Size::XLarge));
    assert!(l.set_size(W::Chart, Size::Large));
    assert!(!l.set_size(W::Chart, Size::Large), "no change");
    assert_eq!(l.cycle_size(W::Stats), Some(Size::Large));
    assert_eq!(l.cycle_size(W::Stats), Some(Size::Small));
    assert_eq!(l.cycle_size(W::Stats), Some(Size::Medium));
    assert_eq!(l.cycle_size(W::Flows), None, "not placed");
}

#[test]
fn add_and_remove() {
    let mut l = Layout::default();
    assert!(l.add(W::Flows, Size::Large));
    assert_eq!(l.tiles().last(), Some(&t(W::Flows, Size::Large)));
    // An unsupported size falls back to the widget's default.
    assert!(l.add(W::Assets, Size::XLarge));
    assert_eq!(l.size_of(W::Assets), Some(Size::Medium));
    // Adding a placed widget changes its size and never duplicates it.
    assert!(l.add(W::Flows, Size::Medium));
    assert_eq!(l.tiles().iter().filter(|t| t.id == W::Flows).count(), 1);
    assert!(!l.add(W::Flows, Size::Medium));
    assert_eq!(l.unplaced(), vec![W::Attention, W::Age, W::Goal]);
    assert!(l.remove(W::Stats));
    assert!(!l.remove(W::Stats));
    assert!(!l.contains(W::Stats));
    for w in W::ALL {
        l.remove(w);
    }
    assert_eq!(l.len(), 0);
}

#[test]
fn stored_layout_round_trips() {
    let mut l = Layout::default();
    l.remove(W::Stats);
    l.add(W::Assets, Size::Small);
    l.set_size(W::Chart, Size::Large);
    l.move_to(W::TaxYear, 0);
    assert_eq!(Layout::from_json(&l.to_json()), l);
    let empty = Layout::empty();
    assert_eq!(
        Layout::from_json(&empty.to_json()),
        empty,
        "an empty home screen stays empty"
    );
}

#[test]
fn stored_layout_is_validated() {
    let l = Layout::from_json(&json!({"version": 2, "tiles": [
        {"id": "flows", "size": "large"},
        {"id": "ticker", "size": "small"},
        {"id": "chart", "size": "small"},
        {"id": "flows", "size": "medium"},
        {"id": "recent"},
        {"id": "taxYear", "size": 3},
        "holdings",
    ]}));
    assert_eq!(
        l.tiles(),
        &[
            t(W::Flows, Size::Large),
            t(W::Chart, Size::XLarge),
            t(W::Recent, Size::Large),
            t(W::TaxYear, Size::Medium)
        ]
    );
    for bad in [
        json!(null),
        json!("grid"),
        json!({}),
        json!({"version": 2}),
        json!({"version": 2, "tiles": "all"}),
        json!({"version": 9, "tiles": []}),
        json!([1, 2]),
    ] {
        assert_eq!(Layout::from_json(&bad), Layout::default(), "{bad}");
    }
}

#[test]
fn version_one_layout_migrates_its_order_and_visibility() {
    let v1 = json!({"version": 1, "widgets": [
        {"id": "stats", "x": 0, "y": 0, "w": 12, "h": 6, "visible": false},
        {"id": "recent", "x": 0, "y": 6, "w": 4, "h": 24, "visible": true},
        {"id": "chart", "x": 4, "y": 6, "w": 8, "h": 28, "visible": true},
        {"id": "holdings", "x": 0, "y": 40, "w": 12, "h": 6, "visible": true},
        {"id": "taxYear", "x": 0, "y": 30, "w": 4, "h": 6, "visible": true},
        {"id": "flows", "x": 4, "y": 30, "w": 4, "h": 8, "visible": true},
        {"id": "assets", "x": 8, "y": 30, "w": 4, "h": 6, "visible": false},
    ]});
    let l = Layout::from_json(&v1);
    assert_eq!(
        l.tiles(),
        &[
            t(W::Recent, Size::Large),
            t(W::Chart, Size::XLarge),
            t(W::TaxYear, Size::Medium),
            t(W::Flows, Size::Medium),
            t(W::Holdings, Size::Large)
        ]
    );
    // Malformed version 1 documents give the defaults.
    assert_eq!(Layout::from_json(&json!({"version": 1})), Layout::default());
    assert_eq!(
        Layout::from_json(
            &json!({"version": 1, "widgets": [{"id": "chart", "x": "0"}, {"id": "ticker"}]})
        ),
        Layout::default()
    );
}

#[test]
fn grid_has_four_columns_at_the_wide_window_and_two_at_the_minimum() {
    let tk = crate::theme::Tk::new(true, 1.0);
    let wide = Geo::new(&tk, 1144.0);
    assert_eq!(wide.cols, 4);
    assert!((wide.col_w * 4.0 + wide.gap * 3.0 - 1144.0).abs() < 0.01);
    assert!(
        wide.row_h < wide.col_w && wide.row_h <= tk.r(15.0),
        "a tile is a little wider than tall"
    );
    assert!(
        (wide.gap - tk.card_gap()).abs() < 0.01,
        "the app's card gap between tiles"
    );
    let narrow = Geo::new(&tk, 700.0);
    assert_eq!(narrow.cols, 2);
    assert_eq!(Geo::new(&tk, 200.0).cols, 2, "never fewer than two");
    assert_eq!(
        Geo::new(&tk, 850.0).cols,
        2,
        "three would fit, but the count stays even"
    );
    let (w, h) = wide.span(2, 2);
    assert!((w - (2.0 * wide.col_w + wide.gap)).abs() < 0.01);
    assert!((h - (2.0 * wide.row_h + wide.gap)).abs() < 0.01);
}

#[test]
fn a_drag_drops_onto_the_tile_under_its_centre() {
    let tk = crate::theme::Tk::new(true, 1.0);
    let geo = Geo::new(&tk, 1144.0);
    let l = Layout::default();
    let cells = pack(l.tiles(), geo.cols);
    let centre = |id: W| {
        let c = cells.iter().find(|c| c.id == id).unwrap();
        let (x, y) = geo.origin(c.x, c.y);
        let (w, h) = geo.span(c.w, c.h);
        (x + w / 2.0, y + h / 2.0)
    };
    // Tax year dragged over Key figures takes its place in the order.
    let (x, y) = centre(W::Stats);
    assert_eq!(drop_target(&l, geo, W::TaxYear, x, y), Some((W::Stats, 0)));
    // Over itself the order stays.
    let (x, y) = centre(W::TaxYear);
    assert_eq!(drop_target(&l, geo, W::TaxYear, x, y), None);
    // Key figures dragged onto Recent transactions lands at its place.
    let (x, y) = centre(W::Recent);
    assert_eq!(drop_target(&l, geo, W::Stats, x, y), Some((W::Recent, 3)));
    // Below the last row: the end of the list.
    assert_eq!(
        drop_target(&l, geo, W::Stats, 10.0, 99_999.0),
        Some((W::Holdings, 4))
    );
    // In a gutter between tiles: nothing.
    let (x, _) = geo.origin(2, 0);
    assert_eq!(
        drop_target(&l, geo, W::Chart, x - geo.gap / 2.0, 10.0),
        None
    );
}

#[test]
fn settle_puts_the_tiles_in_reading_order_without_moving_them() {
    let mut l = Layout::default();
    let before = pack(l.tiles(), 4);
    l.settle(4);
    let ids: Vec<W> = l.tiles().iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        vec![W::Stats, W::TaxYear, W::Chart, W::Recent, W::Holdings]
    );
    let after = pack(l.tiles(), 4);
    for c in &before {
        let d = after.iter().find(|d| d.id == c.id).unwrap();
        assert_eq!(
            (c.x, c.y, c.w, c.h),
            (d.x, d.y, d.w, d.h),
            "{:?} stays put",
            c.id
        );
    }
    // Settled, one place later trades Key figures with its neighbour.
    assert!(l.shift(W::Stats, 1));
    let cells = pack(l.tiles(), 4);
    let at = |id| {
        cells
            .iter()
            .find(|c| c.id == id)
            .map(|c| (c.x, c.y))
            .unwrap()
    };
    assert_eq!(at(W::TaxYear), (0, 0));
    assert_eq!(at(W::Stats), (2, 0));
}

#[test]
fn rows_take_the_height_their_one_row_tiles_need() {
    let tk = crate::theme::Tk::new(true, 1.0);
    let geo = Geo::new(&tk, 1152.0);
    let cells = pack(Layout::default().tiles(), geo.cols);
    // Key figures wants 150, Tax year 120: the top row is 150; the chart's and the lists' rows
    // keep the base height.
    let heights = super::row_heights(&geo, &cells, |c, _| match c.id {
        W::Stats => 150.0,
        W::TaxYear => 120.0,
        _ => 9_999.0,
    });
    assert_eq!(heights.len(), 5);
    assert_eq!(heights[0], 150.0);
    assert!(heights[1..].iter().all(|h| (*h - geo.row_h).abs() < 0.01));
    let g = geo.with_rows(&heights);
    assert_eq!(g.origin(0, 1).1, 150.0 + g.gap);
    assert_eq!(g.span_at(1, 4, 2).1, 2.0 * g.row_h + g.gap);
    assert_eq!(g.total(5), 150.0 + 4.0 * g.row_h + 4.0 * g.gap);
    // Never taller than the base row.
    let tall = super::row_heights(&geo, &cells, |_, _| 9_999.0);
    assert!(tall.iter().all(|h| (*h - geo.row_h).abs() < 0.01));
    // A drop below the shorter rows still finds the tile under the point.
    let l = Layout::default();
    let (x, y) = g.origin(0, 3);
    assert_eq!(
        drop_target(&l, g, W::Stats, x + 5.0, y + 5.0),
        Some((W::Recent, 3))
    );
}
