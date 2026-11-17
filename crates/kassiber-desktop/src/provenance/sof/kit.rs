//! The shadcn pieces the source-of-funds stages use (Input, Textarea, Select, Checkbox, Card,
//! Label, `<details>`), copied from the shared widgets rather than refactoring them, plus the
//! Tailwind tones the route's classes name.

use rune::{Color, El, Icon, Shadow, TextEdit, TextStyle, col, el, row, text, text_input};
use rune_kit::Placement;

use super::Popup;
use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Size, Variant};
use crate::ui::ic;

pub type EditFn = fn(&mut App) -> &mut TextEdit;

pub fn h(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::WHITE)
}

/// `tabular-nums`.
pub fn tnum(s: TextStyle) -> TextStyle {
    s.features("'tnum' 1")
}

#[allow(dead_code)]
/// A Tailwind v4 hue (oklch, as the web's utilities resolve): steps 50 … 950.
#[derive(Clone, Copy)]
pub struct Hue {
    pub c50: Color,
    pub c100: Color,
    pub c200: Color,
    pub c300: Color,
    pub c400: Color,
    pub c500: Color,
    pub c600: Color,
    pub c700: Color,
    pub c800: Color,
    pub c900: Color,
    pub c950: Color,
}

fn hue(t: [(f32, f32, f32); 11]) -> Hue {
    let c = |i: usize| rune::oklch(t[i].0, t[i].1, t[i].2);
    Hue {
        c50: c(0),
        c100: c(1),
        c200: c(2),
        c300: c(3),
        c400: c(4),
        c500: c(5),
        c600: c(6),
        c700: c(7),
        c800: c(8),
        c900: c(9),
        c950: c(10),
    }
}

pub fn amber() -> Hue {
    hue([
        (0.987, 0.022, 95.277),
        (0.962, 0.059, 95.617),
        (0.924, 0.12, 95.746),
        (0.879, 0.169, 91.605),
        (0.828, 0.189, 84.429),
        (0.769, 0.188, 70.08),
        (0.666, 0.179, 58.318),
        (0.555, 0.163, 48.998),
        (0.473, 0.137, 46.201),
        (0.414, 0.112, 45.904),
        (0.279, 0.077, 45.635),
    ])
}
pub fn emerald() -> Hue {
    hue([
        (0.979, 0.021, 166.113),
        (0.95, 0.052, 163.051),
        (0.905, 0.093, 164.15),
        (0.845, 0.143, 164.978),
        (0.765, 0.177, 163.223),
        (0.696, 0.17, 162.48),
        (0.596, 0.145, 163.225),
        (0.508, 0.118, 165.612),
        (0.432, 0.095, 166.913),
        (0.378, 0.077, 168.94),
        (0.262, 0.051, 172.552),
    ])
}
pub fn red() -> Hue {
    hue([
        (0.971, 0.013, 17.38),
        (0.936, 0.032, 17.717),
        (0.885, 0.062, 18.334),
        (0.808, 0.114, 19.571),
        (0.704, 0.191, 22.216),
        (0.637, 0.237, 25.331),
        (0.577, 0.245, 27.325),
        (0.505, 0.213, 27.518),
        (0.444, 0.177, 26.899),
        (0.396, 0.141, 25.723),
        (0.258, 0.092, 26.042),
    ])
}
pub fn rose() -> Hue {
    hue([
        (0.969, 0.015, 12.422),
        (0.941, 0.03, 12.58),
        (0.892, 0.058, 10.001),
        (0.81, 0.117, 11.638),
        (0.712, 0.194, 13.428),
        (0.645, 0.246, 16.439),
        (0.586, 0.253, 17.585),
        (0.514, 0.222, 16.935),
        (0.455, 0.188, 13.697),
        (0.41, 0.159, 10.272),
        (0.271, 0.105, 12.094),
    ])
}
pub fn sky() -> Hue {
    hue([
        (0.977, 0.013, 236.62),
        (0.951, 0.026, 236.824),
        (0.901, 0.058, 230.902),
        (0.828, 0.111, 230.318),
        (0.746, 0.16, 232.661),
        (0.685, 0.169, 237.323),
        (0.588, 0.158, 241.966),
        (0.5, 0.134, 242.749),
        (0.443, 0.11, 240.79),
        (0.391, 0.09, 240.876),
        (0.293, 0.066, 243.157),
    ])
}
pub fn orange() -> Hue {
    hue([
        (0.98, 0.016, 73.684),
        (0.954, 0.038, 75.164),
        (0.901, 0.076, 70.697),
        (0.837, 0.128, 66.29),
        (0.75, 0.183, 55.934),
        (0.705, 0.213, 47.604),
        (0.646, 0.222, 41.116),
        (0.553, 0.195, 38.402),
        (0.47, 0.157, 37.304),
        (0.408, 0.123, 38.172),
        (0.266, 0.079, 36.259),
    ])
}
pub fn zinc() -> Hue {
    hue([
        (0.985, 0.0, 0.0),
        (0.967, 0.001, 286.375),
        (0.92, 0.004, 286.32),
        (0.871, 0.006, 286.286),
        (0.705, 0.015, 286.067),
        (0.552, 0.016, 285.938),
        (0.442, 0.017, 285.786),
        (0.37, 0.013, 285.805),
        (0.274, 0.006, 286.033),
        (0.21, 0.006, 285.885),
        (0.141, 0.005, 285.823),
    ])
}

/// `text-<hue>-700 dark:text-<hue>-300`.
pub fn ink(tk: &Tk, hue: &Hue) -> Color {
    if tk.dark { hue.c300 } else { hue.c700 }
}

/// The soft notice tones: `border-<hue>-200 bg-<hue>-50 text-<hue>-<text_l>` / dark
/// `border-<hue>-900/60 bg-<hue>-950/40 text-<hue>-<text_d>`. (fill, border, text).
pub fn notice(tk: &Tk, hue: &Hue, text_l: Color, text_d: Color) -> (Color, Color, Color) {
    if tk.dark {
        (hue.c950.alpha(0.4), hue.c900.alpha(0.6), text_d)
    } else {
        (hue.c50, hue.c200, text_l)
    }
}

/// A box in a notice tone: `rounded-md border px-3 py-2`.
pub fn notice_box(tk: &Tk, tone: (Color, Color, Color)) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .fill(tone.0)
        .border(1.0, tone.1)
        .border_layout()
}

pub fn button(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    size: Size,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    w::button(tk, key, variant, size, glyph, label, enabled)
}

/// `rounded-md border` (the plain bordered boxes).
pub fn bordered(tk: &Tk) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
}

/// `EmptyState`: `rounded-md border px-3 py-6 text-center text-sm text-muted-foreground`.
pub fn empty_state(tk: &Tk, s: String) -> El {
    bordered(tk)
        .px(tk.r(0.75))
        .py(tk.r(1.5))
        .items_center()
        .child(
            text(s)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .align(rune::Align::Center)
                .w_full(),
        )
}

/// shadcn `Label` (`text-sm leading-none font-medium`).
pub fn label(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tsm(500.0).lh(tk.r(0.875)))
        .color(tk.fg)
}

/// `Field`: `space-y-2` label then the control.
pub fn field(tk: &Tk, l: &str, control: El) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .child(label(tk, l))
        .child(control)
}

fn frame(tk: &Tk, key: &str) -> El {
    let fill = if tk.dark {
        tk.input.alpha(tk.input.a * 0.3)
    } else {
        Color::TRANSPARENT
    };
    row()
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(fill)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
}

/// shadcn `Input`, `h` tall (h-9 by default, h-8 / h-10 where the route says so).
pub fn input(
    tk: &Tk,
    key: &str,
    edit: &TextEdit,
    l: &str,
    placeholder: &str,
    access: EditFn,
    hgt: f32,
) -> El {
    let f = text_input(edit, tk.tsm(400.0))
        .key(format!("{key}-field"))
        .single_line()
        .placeholder(placeholder.to_owned(), tk.muted_fg)
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(l.to_owned())
        .on_edit(access)
        .grow(1.0)
        .min_w(0.0);
    frame(tk, key).h(hgt).px(tk.r(0.75)).center_y().child(f)
}

/// The search input (`pl-9`, a Search glyph at `left-3`).
pub fn search_input(
    tk: &Tk,
    key: &str,
    edit: &TextEdit,
    l: &str,
    placeholder: &str,
    access: EditFn,
) -> El {
    let f = text_input(edit, tk.tsm(400.0))
        .key(format!("{key}-field"))
        .single_line()
        .placeholder(placeholder.to_owned(), tk.muted_fg)
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(l.to_owned())
        .role("searchbox")
        .on_edit(access)
        .grow(1.0)
        .min_w(0.0);
    frame(tk, key)
        .h(tk.r(2.0))
        .pl(tk.r(0.75))
        .pr(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .child(ic(&I::SEARCH, tk.r(1.0), tk.muted_fg).a11y_hidden())
        .child(f)
}

/// shadcn `Textarea` (`min-h-16 px-3 py-2`).
pub fn textarea(tk: &Tk, key: &str, edit: &TextEdit, l: &str, access: EditFn) -> El {
    let f = text_input(edit, tk.tsm(400.0))
        .key(format!("{key}-field"))
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(l.to_owned())
        .on_edit(access)
        .w_full();
    frame(tk, key)
        .min_h(tk.r(4.0))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .items_start()
        .child(f)
}

/// shadcn `Checkbox` with its label as one control (`flex items-center gap-2 text-sm`).
pub fn checkbox(tk: &Tk, key: &str, on: bool, l: &str, toggle: impl Fn(&mut App) + 'static) -> El {
    let b = row()
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(4.0)
        .border(1.0, if on { tk.primary } else { tk.input })
        .border_layout();
    let b = if on {
        b.fill(tk.primary)
            .child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b.fill(if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
    };
    row()
        .key(key.to_owned())
        .gap(tk.r(0.5))
        .center_y()
        .role("checkbox")
        .checked(on)
        .label(l.to_owned())
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(toggle)
        .child(b.a11y_hidden())
        .child(
            text(l.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .shrink(),
        )
}

/// A `<details>` summary: the disclosure triangle then the label; `open` flips the triangle.
pub fn summary(
    tk: &Tk,
    key: &str,
    open: bool,
    l: &str,
    font: TextStyle,
    color: Color,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    // The `<summary>` disclosure marker: a filled triangle, pointing down when open.
    let s = tk.r(0.5);
    let pts = if open {
        vec![[0.0, s * 0.15], [s, s * 0.15], [s * 0.5, s * 0.9]]
    } else {
        vec![[s * 0.15, 0.0], [s * 0.9, s * 0.5], [s * 0.15, s]]
    };
    let tri = rune::canvas(rune::Shape::new().view_box(s, s).fill(pts, color))
        .size(s, s)
        .mr(tk.r(0.125))
        .a11y_hidden();
    row()
        .key(key.to_owned())
        .gap(tk.r(0.375))
        .center_y()
        .role("button")
        .label(l.to_owned())
        .expanded(open)
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(toggle)
        .child(tri)
        .child(text(l.to_owned()).font(font).color(color).wrap().shrink())
}

/// shadcn `Card` (`kb-surface flex flex-col gap-4 py-5`), the header `border-b pb-5` with a
/// `text-base` title (an optional icon) and its description, then `CardContent p-4`.
pub fn card(
    tk: &Tk,
    key: &str,
    glyph: Option<&'static Icon>,
    title: &str,
    desc: Option<String>,
    content: El,
) -> El {
    let mut t = row().gap(tk.r(0.5)).center_y();
    if let Some(g) = glyph {
        t = t.child(ic(g, tk.r(1.0), tk.fg).a11y_hidden());
    }
    t = t.child(
        text(title.to_owned())
            .font(tk.tbase(600.0).lh(tk.r(1.0)))
            .color(tk.fg)
            .role("heading")
            .level(3),
    );
    let mut header = col()
        .w_full()
        .px(tk.card_pad())
        .pb(tk.card_pad())
        .gap(tk.r(0.375))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(t);
    if let Some(d) = desc {
        header = header.child(
            text(d)
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        );
    }
    crate::ui::surface(tk)
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .py(tk.card_pad())
        .gap(tk.r(1.0))
        .child(header)
        .child(content.w_full().padding(tk.r(1.0)))
}

/// `kb-surface-inset`.
pub fn inset(tk: &Tk) -> El {
    w::surface_inset(tk)
}

// ---- Select -----------------------------------------------------------------------------------

pub fn close(a: &mut App) {
    a.provenance.sof.popup = None;
}

pub fn toggle(a: &mut App, p: Popup) {
    a.provenance.sof.popup = if a.provenance.sof.popup.as_ref() == Some(&p) {
        None
    } else {
        Some(p)
    };
}

/// `SelectTrigger` (`h` tall, `rounded-md border border-input px-3`, chevron at 50 %).
pub fn select_trigger(
    tk: &Tk,
    key: &str,
    l: &str,
    value: &str,
    placeholder: bool,
    open: bool,
    hgt: f32,
) -> El {
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, Color::TRANSPARENT)
    };
    row()
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .h(hgt)
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(fill)
        .hover(hover)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
        .role("combobox")
        .label(l.to_owned())
        .a11y_value(value.to_owned())
        .expanded(open)
        .focusable(true)
        .child(
            text(value.to_owned())
                .font(tk.tsm(400.0))
                .color(if placeholder { tk.muted_fg } else { tk.fg })
                .truncate()
                .shrink(),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)))
}

/// `SelectContent` (`position="item-aligned"`): the current option over the trigger. Options are
/// (value, label, disabled).
pub fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    min_w: f32,
    options: Vec<(String, String, bool)>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let selected = options.iter().position(|(v, _, _)| v == current);
    let item_h = tk.r(0.375) * 2.0 + tk.r(1.25);
    let pad = tk.r(0.25);
    let n = options.len();
    let mut p = crate::ui::popup::Popup::listbox(anchor, close);
    for (v, l, disabled) in options {
        let checked = v == current;
        let pick = choose.clone();
        let val = v.clone();
        let mut item = row()
            .w_full()
            .h(item_h)
            .pl(tk.r(0.5))
            .pr(tk.r(2.0))
            .center_y()
            .radius(tk.r(0.25))
            .role("option")
            .checked(checked)
            .label(l.clone())
            .opacity(if disabled { 0.5 } else { 1.0 })
            .child(text(l.clone()).font(tk.tsm(400.0)).color(tk.fg).truncate())
            .child(
                row()
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .justify_end()
                    .center_y()
                    .pr(tk.r(0.5))
                    .child(if checked {
                        ic(&I::CHECK, tk.r(1.0), tk.fg)
                    } else {
                        el().size(1.0, 1.0)
                    }),
            );
        if !disabled {
            item = item
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press(move |a: &mut App| {
                    close(a);
                    pick(a, &val);
                });
        }
        p.item(item, format!("{anchor}-opt-{v}"), l, !disabled, checked);
    }
    // Long lists (the transaction pickers) scroll inside a bounded menu, as Radix's does.
    if n > 12 {
        p = p.max_h(tk.r(24.0));
    }
    let gap = match selected {
        Some(i) if n <= 12 => -(tk.r(2.25) / 2.0 + item_h / 2.0 + pad + 1.0 + i as f32 * item_h),
        _ => 4.0,
    };
    let list = col()
        .min_w(min_w - 2.0 - 2.0 * pad)
        .label(anchor.to_owned());
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).padding(pad).child(l)),
        Placement::BottomStart,
        gap,
    );
}

/// A select bound to one `Popup::Select(id)`: the trigger, and the menu while it is open.
#[allow(clippy::too_many_arguments)]
pub fn select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    id: &'static str,
    l: &str,
    options: Vec<(String, String, bool)>,
    current: &str,
    placeholder: &str,
    hgt: f32,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let open = app.provenance.sof.popup.as_ref() == Some(&Popup::Select(id));
    let shown = options
        .iter()
        .find(|(v, _, _)| v == current)
        .map(|(_, l, _)| l.clone());
    let key = format!("sof-sel-{id}");
    if open {
        let min_w = app
            .provenance
            .sof
            .trigger_w
            .get(id)
            .copied()
            .unwrap_or(tk.r(12.0));
        select_menu(tk, layer, &key, min_w, options, current, choose);
    }
    let trigger = select_trigger(
        tk,
        &key,
        l,
        shown.as_deref().unwrap_or(placeholder),
        shown.is_none(),
        open,
        hgt,
    );
    crate::ui::popup::trigger(
        trigger,
        crate::ui::popup::Kind::Listbox,
        open,
        move |a: &mut App| toggle(a, Popup::Select(id)),
    )
}

/// The accent-ink inline action (`text-xs font-medium text-[var(--kb-accent)] hover:underline`).
pub fn accent_link(
    tk: &Tk,
    key: String,
    glyph: &'static Icon,
    l: &str,
    on: impl Fn(&mut App) + 'static,
) -> El {
    row()
        .key(key)
        .mt(tk.r(0.5))
        .gap(tk.r(0.25))
        .center_y()
        .role("button")
        .label(l.to_owned())
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(on)
        .child(ic(glyph, tk.r(0.875), tk.kb_accent).a11y_hidden())
        .child(text(l.to_owned()).font(tk.txs(500.0)).color(tk.kb_accent))
}

/// A responsive grid: `cols` equal columns `gap` apart, filled row by row.
pub fn grid(tk: &Tk, cols: usize, gap: f32, items: Vec<El>) -> El {
    let _ = tk;
    let cols = cols.max(1);
    let mut out = col().w_full().min_w(0.0).gap(gap);
    let mut it = items.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().min_w(0.0).gap(gap).items_start();
        for _ in 0..cols {
            match it.next() {
                Some(e) => r = r.child(col().grow(1.0).w(0.0).min_w(0.0).child(e.w_full())),
                None => r = r.child(el().grow(1.0).w(0.0)),
            }
        }
        out = out.child(r);
    }
    out
}
