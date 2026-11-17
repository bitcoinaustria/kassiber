//! The page frame and widgets Journals and Exit tax share: `screenShellClassName`, shadcn
//! `Tabs`, `Table`, `Badge`, `Button` and `Skeleton`, the review tones and `ScreenSkeleton`.

use rune::{Color, Cx, El, Icon, TextStyle, col, el, row, text};

use crate::app::App;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Variant};
use crate::ui::{ic, surface};

/// The window width the web's media queries test (`sm` 640, `lg` 1024, `xl` 1280, `2xl` 1536;
/// CSS media queries take rem at the initial 16 px, not the scaled root).
pub fn win_w(app: &App, cx: &Cx<App>) -> f32 {
    let w = cx
        .layout_size("sh-root")
        .map(|[w, _]| w)
        .unwrap_or(app.shell.window_w);
    if w > 0.0 { w } else { 1440.0 }
}

/// `screenShellClassName` on the app's spacing scale (`Tk::page_pad`, `Tk::section_gap`).
pub fn shell(tk: &Tk) -> El {
    col()
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background)
}

/// `tabular-nums`.
pub fn tnum(s: TextStyle) -> TextStyle {
    s.features("'tnum' 1")
}

/// `ReviewTone` / `JournalTone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warning,
    Alert,
    Neutral,
}

fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::WHITE)
}

/// Tailwind palette steps the banners use: (500, 600, 400, 700).
pub struct Hue {
    pub c500: Color,
    pub c600: Color,
    pub c400: Color,
    pub c700: Color,
}

pub fn amber() -> Hue {
    Hue {
        c500: hex("#f59e0b"),
        c600: hex("#d97706"),
        c400: hex("#fbbf24"),
        c700: hex("#b45309"),
    }
}
pub fn sky() -> Hue {
    Hue {
        c500: hex("#0ea5e9"),
        c600: hex("#0284c7"),
        c400: hex("#38bdf8"),
        c700: hex("#0369a1"),
    }
}
pub fn emerald() -> Hue {
    Hue {
        c500: hex("#10b981"),
        c600: hex("#059669"),
        c400: hex("#34d399"),
        c700: hex("#047857"),
    }
}

/// `text-<hue>-600 dark:text-<hue>-400`.
pub fn hue_ink(tk: &Tk, h: &Hue) -> Color {
    if tk.dark { h.c400 } else { h.c600 }
}

/// A `rounded-lg border border-<hue>-500/30 bg-<hue>-500/10 p-4` banner.
pub fn banner(tk: &Tk, h: &Hue) -> El {
    row()
        .w_full()
        .gap(tk.r(0.75))
        .padding(tk.r(1.0))
        .radius(tk.r(0.5))
        .fill(h.c500.alpha(0.1))
        .border(1.0, h.c500.alpha(0.3))
        .border_layout()
}

/// A table column's CSS width: `Auto(min-w)` (0 for none) or a fixed `w-[…px]`.
#[derive(Clone, Copy)]
pub enum ColW {
    Auto(f32),
    Fixed(f32),
}

/// A table column: its CSS width and alignment.
#[derive(Clone, Copy)]
pub struct Col {
    pub w: ColW,
    pub right: bool,
}

pub const fn c(min: f32) -> Col {
    Col {
        w: ColW::Auto(min),
        right: false,
    }
}
pub const fn cr(min: f32) -> Col {
    Col {
        w: ColW::Auto(min),
        right: true,
    }
}
/// The width `s` takes in `style` (one line).
pub fn tw(s: &str, style: TextStyle) -> f32 {
    rune::text::measure(s, &[], style, None).0
}

/// An HTML table's column widths (`table-layout: auto`, every cell `whitespace-nowrap`, so a
/// column's min- and max-content agree), following the browser's staged guesses: every column
/// at its content; fixed columns growing toward their `w-[…]`; then the rest of the table's
/// width shared by the auto columns in proportion to their widths (by the fixed ones when all are
/// fixed). `content` is each column's widest cell, padding included.
pub fn column_widths(cols: &[Col], content: &[f32], table_w: f32) -> Vec<f32> {
    let minc: Vec<f32> = cols
        .iter()
        .zip(content)
        .map(|(c, &w)| match c.w {
            ColW::Auto(m) => w.max(m),
            ColW::Fixed(_) => w,
        })
        .collect();
    let spec: Vec<f32> = cols
        .iter()
        .zip(&minc)
        .map(|(c, &m)| match c.w {
            ColW::Auto(_) => m,
            ColW::Fixed(f) => f.max(m),
        })
        .collect();
    let (g_min, g_spec): (f32, f32) = (minc.iter().sum(), spec.iter().sum());
    if table_w <= g_min {
        return minc;
    }
    if table_w < g_spec {
        let k = (table_w - g_min) / (g_spec - g_min);
        return minc
            .iter()
            .zip(&spec)
            .map(|(m, s)| m + (s - m) * k)
            .collect();
    }
    let extra = table_w - g_spec;
    let auto = cols.iter().any(|c| matches!(c.w, ColW::Auto(_)));
    let base: f32 = cols
        .iter()
        .zip(&spec)
        .filter(|(c, _)| !auto || matches!(c.w, ColW::Auto(_)))
        .map(|(_, s)| s)
        .sum();
    cols.iter()
        .zip(&spec)
        .map(|(c, &s)| {
            if base > 0.0 && (!auto || matches!(c.w, ColW::Auto(_))) {
                s + extra * s / base
            } else {
                s
            }
        })
        .collect()
}

/// A laid-out table: widths per column.
pub struct Table {
    pub cols: Vec<Col>,
    pub widths: Vec<f32>,
    /// Wider than its box (`min-w-[…]` above the box's width): it scrolls sideways.
    pub overflow: bool,
}

impl Table {
    /// Lays out `cols` for `labels` (the header) and the widest body cell per column
    /// (`content`, without padding) in a table of `w-full` with `min-w-[min]`.
    pub fn auto(
        tk: &Tk,
        cols: &[Col],
        labels: &[f32],
        content: &[f32],
        avail: f32,
        min: f32,
    ) -> Table {
        let pad = tk.r(1.0);
        let widest: Vec<f32> = labels
            .iter()
            .zip(content)
            .map(|(l, c)| l.max(*c) + pad)
            .collect();
        let widths = column_widths(cols, &widest, avail.max(min));
        let overflow = widths.iter().sum::<f32>() > avail + 0.5;
        Table {
            cols: cols.to_vec(),
            widths,
            overflow,
        }
    }

    fn total(&self) -> f32 {
        self.widths.iter().sum()
    }

    fn boxed(&self, i: usize) -> El {
        let b = rune::col()
            .w(self.widths[i])
            .min_w(self.widths[i])
            .px(0.0)
            .justify_center();
        if self.cols[i].right {
            b.items_end()
        } else {
            b.items_start()
        }
    }

    /// A header cell: the table kit's label (`text-sm font-medium text-muted-foreground`).
    pub fn th(&self, tk: &Tk, i: usize, label: &str) -> El {
        self.boxed(i)
            .h(tk.r(crate::ui::table::HEAD_H))
            .px(tk.r(crate::ui::table::PX))
            .child(crate::ui::table::th_text(tk, label))
    }

    /// `TableCell` (`p-2 align-middle`).
    pub fn td(&self, tk: &Tk, i: usize, content: El) -> El {
        self.boxed(i).padding(tk.r(0.5)).child(content)
    }

    /// The table in its `overflow-x-auto` box (`key` names the box, whose width the next
    /// layout reads back through [`avail`]), a scroll region named `label`.
    pub fn view(&self, key: &str, label: String, header: El, rows: Vec<El>) -> El {
        crate::ui::table::hscroll_if(
            key,
            label,
            self.overflow,
            col().w(self.total()).child(header).children(rows),
        )
    }
}

/// The width a table box had at the last layout, else `fallback`.
pub fn avail(cx: &Cx<App>, key: &str, fallback: f32) -> f32 {
    cx.layout_size(key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(fallback)
}

/// The page's content width before the first layout: the window, less the sidebar (16 rem),
/// the panel's inset and border, and the page gutters.
pub fn page_w(app: &App, cx: &Cx<App>, tk: &Tk) -> f32 {
    let nav = if crate::shell::nav_folded(app, cx) {
        tk.r(3.0)
    } else {
        tk.r(16.0)
    };
    win_w(app, cx) - nav - tk.r(0.375) - 2.0 - 2.0 * tk.page_pad()
}

/// The header row (the table kit's band).
pub fn thead(tk: &Tk, cells: Vec<El>) -> El {
    crate::ui::table::head(tk).children(cells)
}

/// A body row (the table kit's: a rule under it but the last, `hover:bg-muted/35`).
pub fn trow(tk: &Tk, last: bool, cells: Vec<El>) -> El {
    crate::ui::table::tr(tk, last)
        .items_stretch()
        .children(cells)
}

/// shadcn `Badge` (`rounded-full border px-2 py-0.5 text-xs font-medium`).
pub fn badge(tk: &Tk, label: &str, ink: Color, fill: Color, border: Color) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(999.0)
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .child(text(label.to_owned()).font(tk.txs(500.0)).color(ink))
}

/// `Badge variant="outline"`.
pub fn outline_badge(tk: &Tk, label: &str, ink: Color) -> El {
    badge(tk, label, ink, Color::TRANSPARENT, tk.border)
}

/// `Skeleton`: `animate-pulse rounded-md bg-accent`.
pub fn bone(tk: &Tk, w: f32, h: f32) -> El {
    let b = el().h(h).radius(tk.r(0.375)).fill(tk.accent);
    if w > 0.0 { b.w(w) } else { b.w_full() }
}

/// `ScreenSkeleton` (the page frame while a screen's first read loads).
pub fn screen_skeleton(tk: &Tk, title_w: f32, wide: bool) -> El {
    let header = row()
        .w_full()
        .center_y()
        .justify_between()
        .child(
            col()
                .gap(tk.r(0.5))
                .child(bone(tk, tk.r(7.0), tk.r(0.75)))
                .child(bone(tk, title_w, tk.r(1.25))),
        )
        .child(
            row()
                .gap(tk.r(0.75))
                .child(bone(tk, tk.r(6.0), tk.r(2.0)))
                .child(bone(tk, tk.r(7.0), tk.r(2.0))),
        );
    let metric = |last: bool| {
        let m = col()
            .grow(1.0)
            .w(0.0)
            .padding(tk.r(1.0))
            .gap(tk.r(0.5))
            .child(bone(tk, tk.r(6.0), tk.r(0.75)))
            .child(bone(tk, tk.r(5.0), tk.r(1.5)))
            .child(bone(tk, tk.r(8.0), tk.r(0.75)));
        if last {
            m
        } else {
            m.border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
                .border_layout()
        }
    };
    let metrics = surface(tk)
        .w_full()
        .clip_rounded(tk.r(0.75))
        .child(row().w_full().children((0..4).map(|i| metric(i == 3))));
    let list = surface(tk)
        .w_full()
        .child(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.5))
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout()
                .child(bone(tk, tk.r(9.0), tk.r(1.0)))
                .child(bone(tk, tk.r(14.0), tk.r(0.75))),
        )
        .children((0..8).map(|i| {
            let r = row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .child(
                    col()
                        .grow(1.0)
                        .gap(tk.r(0.5))
                        .child(bone(tk, tk.r(11.0), tk.r(1.0)))
                        .child(bone(tk, tk.r(16.0), tk.r(0.75))),
                )
                .child(
                    col()
                        .w(96.0)
                        .items_end()
                        .gap(tk.r(0.5))
                        .child(bone(tk, tk.r(5.0), tk.r(1.0)))
                        .child(bone(tk, tk.r(3.5), tk.r(0.75))),
                );
            if i == 7 {
                r
            } else {
                r.border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                    .border_layout()
            }
        }));
    let side = col()
        .gap(tk.card_gap())
        .child(
            surface(tk)
                .w_full()
                .padding(tk.card_pad())
                .gap(tk.r(0.75))
                .child(bone(tk, tk.r(9.0), tk.r(1.0)))
                .child(el().h(tk.r(0.25)))
                .children((0..3).map(|_| bone(tk, 0.0, tk.r(2.25)))),
        )
        .child(
            surface(tk)
                .w_full()
                .padding(tk.card_pad())
                .gap(tk.r(0.5))
                .child(bone(tk, tk.r(8.0), tk.r(1.0)))
                .child(el().h(tk.r(0.5)))
                .child(bone(tk, 0.0, tk.r(0.75)))
                .child(
                    row()
                        .w_full()
                        .child(el().grow(5.0).child(bone(tk, 0.0, tk.r(0.75))))
                        .child(el().grow(1.0)),
                )
                .child(
                    row()
                        .w_full()
                        .child(el().grow(2.0).child(bone(tk, 0.0, tk.r(0.75))))
                        .child(el().grow(1.0)),
                ),
        );
    let body = if wide {
        row()
            .w_full()
            .gap(tk.card_gap())
            .items_start()
            .child(list.grow(1.0).w(0.0))
            .child(side.w(380.0))
    } else {
        col()
            .w_full()
            .gap(tk.card_gap())
            .child(list)
            .child(side.w_full())
    };
    shell(tk)
        .label(crate::i18n::t("common:state.loading"))
        .child(header)
        .child(metrics)
        .child(body)
}

/// Text that blurs while amounts are hidden (`.sensitive`).
pub fn sens(s: impl Into<String>, style: TextStyle, color: Color, hidden: bool) -> El {
    crate::ui::sensitive(s, style, color, hidden)
}

/// `underline underline-offset-2`: Rune text has no decoration, so a 1 px rule 2 px under it.
pub fn underlined(tk: &Tk, t: El, color: Color) -> El {
    let _ = tk;
    col()
        .child(t)
        .child(el().h(1.0).w_full().mt(-1.0).fill(color))
}

// ---- Page kit -----------------------------------------------------------------------------
//
// The look Journals and Reconcile share with the reworked screens (Transfers, Review): a header
// row with the segmented tabs on the left and one visible action plus a "⋯" menu on the right;
// stat tiles with a glyph, a label and the figure; cards whose header sits on the table kit's
// 1.5 rem inset; and Review's centred empty state. Status colour only where something needs
// the owner (no green "all good" figures).

/// One segment of [`seg_tabs`].
pub struct Seg {
    pub key: String,
    pub label: String,
    pub glyph: Option<&'static Icon>,
    /// A status dot before the label (Reconcile's result filters).
    pub dot: Option<Color>,
    pub count: Option<String>,
    pub active: bool,
    pub on: std::rc::Rc<dyn Fn(&mut App)>,
}

impl Seg {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        on: impl Fn(&mut App) + 'static,
    ) -> Seg {
        Seg {
            key: key.into(),
            label: label.into(),
            glyph: None,
            dot: None,
            count: None,
            active,
            on: std::rc::Rc::new(on),
        }
    }
    pub fn glyph(mut self, g: &'static Icon) -> Seg {
        self.glyph = Some(g);
        self
    }
    pub fn dot(mut self, c: Color) -> Seg {
        self.dot = Some(c);
        self
    }
    pub fn count(mut self, c: impl Into<String>) -> Seg {
        self.count = Some(c.into());
        self
    }
}

/// The segmented track Transfers' status tabs use (`h-8 rounded-lg bg-muted p-0.5`, the chosen
/// segment raised on the card colour). `tabs` makes it a tab list (else a group of toggle
/// buttons, as a filter is).
pub fn seg_tabs(tk: &Tk, key: &str, label: String, tabs: bool, items: Vec<Seg>) -> El {
    seg_track(tk, key, label, tabs, items, false)
}

/// [`seg_tabs`] whose segments wrap onto further rows when the track runs out of width (a
/// filter in a card that can get narrow), instead of being cut off.
pub fn seg_tabs_wrap(tk: &Tk, key: &str, label: String, tabs: bool, items: Vec<Seg>) -> El {
    seg_track(tk, key, label, tabs, items, true)
}

fn seg_track(tk: &Tk, key: &str, label: String, tabs: bool, items: Vec<Seg>, wrap: bool) -> El {
    let track = row().key(key.to_owned());
    let track = if wrap {
        track.min_h(tk.r(2.0)).flex_wrap().shrink()
    } else {
        track.h(tk.r(2.0))
    };
    let mut track = track
        .padding(tk.r(0.125))
        .gap(tk.r(0.25))
        .center_y()
        .self_start()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role(if tabs { "tablist" } else { "group" })
        .label(label);
    if tabs {
        track = track.focus_group();
    }
    for s in items {
        let ink = if s.active { tk.fg } else { tk.muted_fg };
        let name = match &s.count {
            Some(c) => format!("{} {c}", s.label),
            None => s.label.clone(),
        };
        let on = s.on.clone();
        let mut b = row()
            .key(s.key.clone())
            .h(tk.r(1.75))
            .px(tk.r(0.75))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .role(if tabs { "tab" } else { "button" })
            .checked(s.active)
            .label(name)
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |a: &mut App| on(a));
        if tabs {
            b = b.focus_selected(s.active);
        }
        if s.active {
            b = b.fill(tk.card).box_shadow(rune::Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.08),
                ..Default::default()
            });
        } else {
            b = b.hover(tk.background.alpha(0.5));
        }
        if let Some(g) = s.glyph {
            b = b.child(ic(g, tk.r(0.875), ink));
        }
        if let Some(d) = s.dot {
            b = b.child(crate::ui::dot(tk.r(0.375), d).a11y_hidden());
        }
        b = b.child(text(s.label).font(tk.tsm(500.0)).color(ink).single_line());
        if let Some(c) = s.count {
            let (fill, cink) = if s.active {
                (tk.muted, tk.fg)
            } else {
                (Color::TRANSPARENT, tk.muted_fg)
            };
            b = b.child(
                row()
                    .px(tk.r(0.375))
                    .radius(999.0)
                    .fill(fill)
                    .a11y_hidden()
                    .child(text(c).font(tnum(tk.t2xs(400.0))).color(cink)),
            );
        }
        track = track.child(b);
    }
    track
}

/// The page header: tabs (and an inline status) on the left, actions on the right.
pub fn page_header(tk: &Tk, left: El, right: Option<El>) -> El {
    let mut r = row()
        .w_full()
        .min_h(tk.r(2.0))
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(
            row()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.75))
                .center_y()
                .flex_wrap()
                .child(left),
        );
    if let Some(a) = right {
        r = r.child(a);
    }
    r
}

/// A running job's inline status beside the tabs (Transfers' "Updating"): spinner and text.
pub fn inline_status(tk: &Tk, label: String) -> El {
    row()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .center_y()
        .role("status")
        .label(label.clone())
        .live_region(rune::LiveRegion::Polite)
        .child(
            ic(&crate::icons::LOADER_CIRCLE, tk.r(0.75), tk.muted_fg)
                .looping(rune::LoopSpec::rotation(0.9)),
        )
        .child(
            text(label)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .min_w(0.0),
        )
}

/// A `size="sm"` page action (`h-8`), its glyph swapped for the spinner while `busy`.
pub fn action(
    tk: &Tk,
    key: &str,
    variant: Variant,
    glyph: &'static Icon,
    label: &str,
    busy: bool,
    enabled: bool,
) -> El {
    let fg = if variant == Variant::Default {
        tk.primary_fg
    } else {
        tk.fg
    };
    let on = enabled && !busy;
    let mut b = w::button_surface(
        tk,
        row()
            .key(key.to_owned())
            .h(tk.r(2.0))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center(),
        variant,
    )
    .role("button")
    .label(label.to_owned())
    .focusable(on)
    .disabled(!on);
    if busy {
        b = b.busy(true).child(
            ic(&crate::icons::LOADER_CIRCLE, tk.r(1.0), fg).looping(rune::LoopSpec::rotation(0.9)),
        );
    } else {
        b = b.child(ic(glyph, tk.r(1.0), fg));
    }
    if !on {
        b = b.opacity(0.5);
    }
    b.child(
        text(label.to_owned())
            .font(tk.tsm(500.0))
            .color(fg)
            .single_line(),
    )
}

/// The page's "⋯" menu button (outline, `size-8`).
pub fn more_button(
    tk: &Tk,
    key: &str,
    open: bool,
    toggle: impl Fn(&mut App) + Clone + 'static,
) -> El {
    let b = w::icon_button(
        tk,
        key.to_owned(),
        Variant::Outline,
        tk.r(2.0),
        &crate::icons::ELLIPSIS,
        tk.r(1.0),
        crate::i18n::t("connections:detail.moreActions"),
    );
    crate::ui::popup::trigger(b, crate::ui::popup::Kind::Menu, open, toggle)
}

/// A stat tile's figure (`text-xl font-semibold tracking-tight tabular-nums`).
pub fn stat_value(tk: &Tk, s: impl Into<String>, color: Color, hidden: bool) -> El {
    sens(
        s,
        tnum(tk.txl(600.0).tracking(-0.025 * tk.r(1.25))),
        color,
        hidden,
    )
    .single_line()
    .truncate()
    .min_w(0.0)
}

/// Stat tiles side by side in one card, a rule between them.
pub fn stat_strip(tk: &Tk, key: &str, label: String, tiles: Vec<El>) -> El {
    let mut r = row().w_full();
    for (i, t) in tiles.into_iter().enumerate() {
        if i > 0 {
            r = r.child(el().w(1.0).h_full().fill(tk.border));
        }
        r = r.child(t);
    }
    surface(tk)
        .key(key.to_owned())
        .w_full()
        .role("region")
        .label(label)
        .clip_rounded(tk.r(0.75))
        .child(r.items_stretch())
}

/// A card header on the table kit's inset (`px-6 py-3.5`): the title (`text-base
/// font-medium`), an optional muted line under it, actions on the right.
pub fn card_head(tk: &Tk, title: String, desc: Option<String>, actions: Option<El>) -> El {
    let mut left = col().min_w(0.0).grow(1.0).w(0.0).gap(tk.r(0.125)).child(
        text(title)
            .font(tk.tbase(500.0))
            .color(tk.fg)
            .role("heading")
            .level(2)
            .single_line()
            .truncate()
            .min_w(0.0),
    );
    if let Some(d) = desc {
        left = left.child(
            text(d)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    }
    let mut r = row()
        .w_full()
        .min_w(0.0)
        .px(tk.r(crate::ui::table::EDGE))
        .py(tk.r(0.875))
        .gap(tk.r(1.0))
        .center_y()
        .child(left);
    if let Some(a) = actions {
        r = r.child(row().gap(tk.r(0.5)).center_y().child(a));
    }
    r
}

/// Review's centred empty state: a tinted glyph disc, the title and one muted paragraph,
/// optionally an action, in the middle of `min_h` of free space (no card around it).
#[allow(clippy::too_many_arguments)]
pub fn centered(
    tk: &Tk,
    key: &str,
    glyph: &'static Icon,
    ink: Color,
    title: String,
    body: String,
    action: Option<El>,
    min_h: f32,
) -> El {
    let mut c = col()
        .w_full()
        .max_w(tk.r(32.0))
        .items_center()
        .gap(tk.r(0.5))
        .child(
            row()
                .size(tk.r(3.0), tk.r(3.0))
                .radius(tk.r(1.5))
                .center()
                .fill(ink.alpha(0.12))
                .mb(tk.r(0.5))
                .a11y_hidden()
                .child(ic(glyph, tk.r(1.5), ink)),
        )
        .child(
            text(title)
                .font(tk.tbase(600.0))
                .color(tk.fg)
                .align(rune::Align::Center)
                .role("heading")
                .level(2),
        )
        .child(
            text(body)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .align(rune::Align::Center),
        );
    if let Some(a) = action {
        c = c.child(a.mt(tk.r(0.75)));
    }
    col()
        .key(key.to_owned())
        .w_full()
        .min_h(min_h)
        .padding(tk.r(1.5))
        .center()
        .child(c)
}

/// The free height under the page header (Review's centred state fills it).
pub fn free_h(app: &App, tk: &Tk) -> f32 {
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    (win_h - crate::shell::window_top_inset() - 2.0 * tk.page_pad() - tk.r(6.0)).max(tk.r(16.0))
}
