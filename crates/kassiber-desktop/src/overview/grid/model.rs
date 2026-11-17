//! The Overview's widget layout as data, in the manner of a phone's home screen: an ordered list
//! of widgets, each at one of a few fixed sizes, packed onto a uniform tile grid in that order.
//! There is no free placement and no free resizing: reordering changes the list, and a widget
//! only ever takes a size it has a layout for. Persisted as `overviewLayout` in the `kb.ui`
//! document (see `persist::model`); one layout for the whole app, the same for every book.

use serde_json::{Value, json};

/// `overviewLayout.version`. Version 1 was the free 12-column grid; it migrates (see
/// [`Layout::from_json`]).
pub const VERSION: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WidgetId {
    Stats,
    Chart,
    Recent,
    Holdings,
    TaxYear,
    Flows,
    Assets,
    /// What waits on the owner (review, failed sources, stale journals, blocked reports).
    Attention,
    /// The stack by how long it has been held.
    Age,
    /// Progress to a stack size the owner sets.
    Goal,
}

/// A widget's size in tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Size {
    /// 1 × 1, a square.
    Small,
    /// 2 × 1.
    Medium,
    /// 2 × 2.
    Large,
    /// 4 × 2, the full width of a four-column grid.
    XLarge,
}

impl Size {
    pub const ALL: [Size; 4] = [Size::Small, Size::Medium, Size::Large, Size::XLarge];

    pub fn id(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Large => "large",
            Size::XLarge => "xlarge",
        }
    }

    pub fn from_id(s: &str) -> Option<Size> {
        Size::ALL.into_iter().find(|z| z.id() == s)
    }

    /// (columns, rows).
    pub fn span(self) -> (u8, u8) {
        match self {
            Size::Small => (1, 1),
            Size::Medium => (2, 1),
            Size::Large => (2, 2),
            Size::XLarge => (4, 2),
        }
    }
}

impl WidgetId {
    pub const ALL: [WidgetId; 10] = [
        WidgetId::Stats,
        WidgetId::Chart,
        WidgetId::Recent,
        WidgetId::Holdings,
        WidgetId::TaxYear,
        WidgetId::Flows,
        WidgetId::Assets,
        WidgetId::Attention,
        WidgetId::Age,
        WidgetId::Goal,
    ];

    pub fn id(self) -> &'static str {
        match self {
            WidgetId::Stats => "stats",
            WidgetId::Chart => "chart",
            WidgetId::Recent => "recent",
            WidgetId::Holdings => "holdings",
            WidgetId::TaxYear => "taxYear",
            WidgetId::Flows => "flows",
            WidgetId::Assets => "assets",
            WidgetId::Attention => "attention",
            WidgetId::Age => "holdingAge",
            WidgetId::Goal => "stackGoal",
        }
    }

    pub fn from_id(s: &str) -> Option<WidgetId> {
        WidgetId::ALL.into_iter().find(|w| w.id() == s)
    }

    /// The sizes this widget has a layout for, smallest first.
    pub fn sizes(self) -> &'static [Size] {
        use Size::*;
        match self {
            WidgetId::Stats => &[Small, Medium, Large],
            WidgetId::Chart => &[Large, XLarge],
            WidgetId::Recent => &[Medium, Large],
            WidgetId::Holdings => &[Medium, Large],
            WidgetId::TaxYear => &[Small, Medium],
            WidgetId::Flows => &[Medium, Large],
            WidgetId::Assets => &[Small, Medium],
            WidgetId::Attention | WidgetId::Age | WidgetId::Goal => &[Small, Medium],
        }
    }

    /// The size a widget takes when added, and when a stored size is not one of its own.
    pub fn default_size(self) -> Size {
        match self {
            WidgetId::Stats
            | WidgetId::TaxYear
            | WidgetId::Flows
            | WidgetId::Assets
            | WidgetId::Attention
            | WidgetId::Age
            | WidgetId::Goal => Size::Medium,
            WidgetId::Chart => Size::XLarge,
            WidgetId::Recent | WidgetId::Holdings => Size::Large,
        }
    }

    pub fn supports(self, size: Size) -> bool {
        self.sizes().contains(&size)
    }
}

/// One placed widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub id: WidgetId,
    pub size: Size,
}

/// The largest stack goal: every bitcoin there will be, in sats.
pub const GOAL_MAX_SATS: u64 = 21_000_000 * 100_000_000;

/// The placed widgets in reading order; each widget at most once. A widget that is not in the
/// list is not shown (and not mounted); the gallery offers it. Widget settings that outlive the
/// tile (the stack goal) ride along in the same document and survive "Reset to default".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    tiles: Vec<Tile>,
    /// The Stack goal widget's target in sats (1 sat to 21 M BTC); `None` until the owner sets one.
    goal_sats: Option<u64>,
}

impl Default for Layout {
    /// The owner's own arrangement: Key figures beside Tax year, the chart across under them,
    /// Recent transactions then Holdings by Source. On a six-column grid the last two stack in
    /// the right column beside the others; on four columns they sit side by side under the
    /// chart; on two every tile keeps this order. The other widgets wait in the gallery.
    fn default() -> Self {
        let t = |id, size| Tile { id, size };
        Layout {
            tiles: vec![
                t(WidgetId::Stats, Size::Medium),
                t(WidgetId::TaxYear, Size::Medium),
                t(WidgetId::Chart, Size::XLarge),
                t(WidgetId::Recent, Size::Large),
                t(WidgetId::Holdings, Size::Large),
            ],
            goal_sats: None,
        }
    }
}

impl Layout {
    pub fn empty() -> Layout {
        Layout {
            tiles: Vec::new(),
            goal_sats: None,
        }
    }

    /// The default arrangement, keeping the widgets' own settings (the stack goal).
    pub fn reset(&mut self) {
        let goal = self.goal_sats;
        *self = Layout {
            goal_sats: goal,
            ..Layout::default()
        };
    }

    /// True when the arrangement is the default one (settings aside).
    pub fn is_default_arrangement(&self) -> bool {
        self.tiles == Layout::default().tiles
    }

    pub fn goal_sats(&self) -> Option<u64> {
        self.goal_sats
    }

    /// Sets (or clears) the stack goal; a target outside 1 sat … 21 M BTC is refused.
    pub fn set_goal_sats(&mut self, sats: Option<u64>) -> bool {
        match sats {
            Some(s) if !(1..=GOAL_MAX_SATS).contains(&s) => false,
            _ => {
                self.goal_sats = sats;
                true
            }
        }
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn contains(&self, id: WidgetId) -> bool {
        self.index_of(id).is_some()
    }

    pub fn index_of(&self, id: WidgetId) -> Option<usize> {
        self.tiles.iter().position(|t| t.id == id)
    }

    pub fn size_of(&self, id: WidgetId) -> Option<Size> {
        self.tiles.iter().find(|t| t.id == id).map(|t| t.size)
    }

    /// The widgets the gallery can still add.
    pub fn unplaced(&self) -> Vec<WidgetId> {
        WidgetId::ALL
            .into_iter()
            .filter(|w| !self.contains(*w))
            .collect()
    }

    /// Adds a widget at the end (or changes the size of one already placed). An unsupported
    /// size becomes the widget's default. True when anything changed.
    pub fn add(&mut self, id: WidgetId, size: Size) -> bool {
        let size = if id.supports(size) {
            size
        } else {
            id.default_size()
        };
        match self.index_of(id) {
            Some(_) => self.set_size(id, size),
            None => {
                self.tiles.push(Tile { id, size });
                true
            }
        }
    }

    pub fn remove(&mut self, id: WidgetId) -> bool {
        let before = self.tiles.len();
        self.tiles.retain(|t| t.id != id);
        self.tiles.len() != before
    }

    /// Sets a placed widget's size; false for a size it has no layout for, or no change.
    pub fn set_size(&mut self, id: WidgetId, size: Size) -> bool {
        if !id.supports(size) {
            return false;
        }
        match self.tiles.iter_mut().find(|t| t.id == id) {
            Some(t) if t.size != size => {
                t.size = size;
                true
            }
            _ => false,
        }
    }

    /// The widget's next supported size, wrapping to its smallest; the new size.
    pub fn cycle_size(&mut self, id: WidgetId) -> Option<Size> {
        let cur = self.size_of(id)?;
        let sizes = id.sizes();
        let at = sizes.iter().position(|s| *s == cur).unwrap_or(0);
        let next = sizes[(at + 1) % sizes.len()];
        self.set_size(id, next).then_some(next)
    }

    /// Moves a widget to `index` in the order (clamped); true when the order changed.
    pub fn move_to(&mut self, id: WidgetId, index: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let to = index.min(self.tiles.len() - 1);
        if from == to {
            return false;
        }
        let t = self.tiles.remove(from);
        self.tiles.insert(to, t);
        true
    }

    /// Puts the tiles in the order they read on a grid `cols` wide (row by row, left to
    /// right). A small tile that packed up into an earlier gap moves up in the order too, so
    /// "the place before" and "the place after" are the ones the eye sees. Nothing moves on
    /// screen when the reading order packs to the same places (the usual case).
    pub fn settle(&mut self, cols: u8) {
        for _ in 0..4 {
            let mut cells = pack(&self.tiles, cols);
            cells.sort_by_key(|c| (c.y, c.x));
            let next: Vec<Tile> = cells
                .iter()
                .map(|c| Tile {
                    id: c.id,
                    size: c.size,
                })
                .collect();
            if next == self.tiles {
                return;
            }
            self.tiles = next;
        }
    }

    /// Moves a widget one place earlier (`-1`) or later (`1`); false at the edge.
    pub fn shift(&mut self, id: WidgetId, by: i32) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let to = from as i64 + i64::from(by);
        if to < 0 || to >= self.tiles.len() as i64 {
            return false;
        }
        self.move_to(id, to as usize)
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "version": VERSION,
            "tiles": self.tiles.iter().map(|t| json!({ "id": t.id.id(), "size": t.size.id() })).collect::<Vec<_>>(),
        });
        // Additive within version 2: absent means no goal, and older builds ignore it.
        if let Some(sats) = self.goal_sats {
            v["stackGoal"] = json!({ "sats": sats });
        }
        v
    }

    /// `stackGoal.sats`: a whole number of sats within 1 … 21 M BTC, else no goal.
    fn read_goal(v: &Value) -> Option<u64> {
        v.get("stackGoal")?
            .get("sats")?
            .as_u64()
            .filter(|s| (1..=GOAL_MAX_SATS).contains(s))
    }

    /// Reads a stored layout, keeping what is valid: an unknown widget or a second entry for the
    /// same widget is dropped, a size the widget has no layout for becomes its default. A
    /// document that is not a layout at all gives the default arrangement. An empty list is
    /// valid (the user removed everything). A version 1 document (the free grid) keeps which
    /// widgets were shown and their reading order, at each widget's default size.
    pub fn from_json(v: &Value) -> Layout {
        match v.get("version").and_then(Value::as_u64) {
            Some(VERSION) => {
                let Some(list) = v.get("tiles").and_then(Value::as_array) else {
                    return Layout::default();
                };
                let mut out = Layout::empty();
                out.goal_sats = Layout::read_goal(v);
                for e in list {
                    let Some(id) = e
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(WidgetId::from_id)
                    else {
                        continue;
                    };
                    if out.contains(id) {
                        continue;
                    }
                    let size = e
                        .get("size")
                        .and_then(Value::as_str)
                        .and_then(Size::from_id)
                        .filter(|s| id.supports(*s))
                        .unwrap_or_else(|| id.default_size());
                    out.tiles.push(Tile { id, size });
                }
                out
            }
            Some(1) => Layout::migrate_v1(v).unwrap_or_default(),
            _ => Layout::default(),
        }
    }

    /// `{"version":1,"widgets":[{"id","x","y","w","h","visible"}]}`: the visible widgets in
    /// `(y, x)` order. A widget the document does not name keeps its default visibility. `None`
    /// when the document has no usable widget list.
    fn migrate_v1(v: &Value) -> Option<Layout> {
        let list = v.get("widgets").and_then(Value::as_array)?;
        let defaults = Layout::default();
        // (visible, y, x) per widget; unnamed ones sort after the named ones in default order.
        let mut seen: Vec<(WidgetId, bool, u64, u64)> = Vec::new();
        for e in list {
            let Some(id) = e
                .get("id")
                .and_then(Value::as_str)
                .and_then(WidgetId::from_id)
            else {
                continue;
            };
            if seen.iter().any(|s| s.0 == id) {
                continue;
            }
            let (Some(x), Some(y)) = (
                e.get("x").and_then(Value::as_u64),
                e.get("y").and_then(Value::as_u64),
            ) else {
                continue;
            };
            let visible = e.get("visible").and_then(Value::as_bool).unwrap_or(true);
            seen.push((id, visible, y, x));
        }
        if seen.is_empty() {
            return None;
        }
        seen.sort_by_key(|s| (s.2, s.3));
        let mut out = Layout::empty();
        for (id, visible, ..) in &seen {
            if *visible {
                out.tiles.push(Tile {
                    id: *id,
                    size: id.default_size(),
                });
            }
        }
        for t in defaults.tiles {
            if !seen.iter().any(|s| s.0 == t.id) {
                out.tiles.push(t);
            }
        }
        Some(out)
    }
}

/// A packed tile: its cell (column, row) and span on a grid `cols` wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub id: WidgetId,
    pub size: Size,
    pub x: u8,
    pub y: u16,
    pub w: u8,
    pub h: u8,
}

/// Packs the tiles in order onto a grid `cols` wide, as a phone's home screen does: each tile
/// takes the first free place in reading order (row by row, left to right), so a small tile
/// fills a gap an earlier, wider tile left. Nothing overlaps; a tile wider than the grid takes
/// the full width.
pub fn pack(tiles: &[Tile], cols: u8) -> Vec<Cell> {
    let cols = cols.max(1);
    let mut taken: Vec<Vec<bool>> = Vec::new();
    let mut out = Vec::with_capacity(tiles.len());
    for t in tiles {
        let (w, h) = t.size.span();
        let w = w.min(cols);
        let mut x;
        let mut y = 0u16;
        'find: loop {
            while taken.len() < usize::from(y) + usize::from(h) {
                taken.push(vec![false; usize::from(cols)]);
            }
            x = 0;
            while x + w <= cols {
                let free = (0..h).all(|dy| {
                    (0..w).all(|dx| !taken[usize::from(y) + usize::from(dy)][usize::from(x + dx)])
                });
                if free {
                    break 'find;
                }
                x += 1;
            }
            y += 1;
        }
        for dy in 0..h {
            for dx in 0..w {
                taken[usize::from(y) + usize::from(dy)][usize::from(x + dx)] = true;
            }
        }
        out.push(Cell {
            id: t.id,
            size: t.size,
            x,
            y,
            w,
            h,
        });
    }
    out
}

/// The rows a packed arrangement takes.
pub fn rows(cells: &[Cell]) -> u16 {
    cells
        .iter()
        .map(|c| c.y + u16::from(c.h))
        .max()
        .unwrap_or(0)
}
