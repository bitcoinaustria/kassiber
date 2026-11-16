//! Building blocks every part of the Transfers screen shares: shadcn buttons in the sizes the
//! screen uses, the titled card header and its count, the stat tile, rail icons, the cut wallet
//! name, exact bitcoin figures, row hints, the readiness pill, the developer badge, dropdown
//! menus and dialogs. Each part keeps its own popup state; a menu or dialog is told how to close.

use std::rc::Rc;

use rune::{Color, Cx, El, Icon, Shadow, TextStyle, col, el, row, text};
use rune_kit::Placement;

use super::model::Rail;
use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self, Size, Variant};
use crate::ui::{ic, sensitive};

pub use widgets::Variant as Var;

// ---- Colours -----------------------------------------------------------------------------------

/// Tailwind v4's palette (oklch), as the web's utilities resolve.
pub fn tw(name: &str) -> Color {
    let (l, c, h) = match name {
        "emerald-50" => (0.979, 0.021, 166.113),
        "emerald-300" => (0.845, 0.143, 164.978),
        "emerald-400" => (0.765, 0.177, 163.223),
        "emerald-600" => (0.596, 0.145, 163.225),
        "emerald-700" => (0.508, 0.118, 165.612),
        "emerald-900" => (0.378, 0.077, 168.94),
        "amber-50" => (0.987, 0.022, 95.277),
        "amber-200" => (0.924, 0.12, 95.746),
        "amber-300" => (0.879, 0.169, 91.605),
        "amber-400" => (0.828, 0.189, 84.429),
        "amber-500" => (0.769, 0.188, 70.08),
        "amber-600" => (0.666, 0.179, 58.318),
        "amber-700" => (0.555, 0.163, 48.998),
        "amber-800" => (0.473, 0.137, 46.201),
        "amber-900" => (0.414, 0.112, 45.904),
        "blue-50" => (0.97, 0.014, 254.604),
        "blue-400" => (0.707, 0.165, 254.624),
        "blue-700" => (0.488, 0.243, 264.376),
        "blue-900" => (0.379, 0.146, 265.522),
        "zinc-50" => (0.985, 0.0, 0.0),
        "zinc-300" => (0.871, 0.006, 286.286),
        "zinc-400" => (0.705, 0.015, 286.067),
        "zinc-500" => (0.552, 0.016, 285.938),
        "zinc-700" => (0.37, 0.013, 285.805),
        "zinc-800" => (0.274, 0.006, 286.033),
        "gray-50" => (0.985, 0.002, 247.839),
        "gray-400" => (0.707, 0.022, 261.325),
        "gray-500" => (0.551, 0.027, 264.364),
        "gray-600" => (0.446, 0.03, 256.802),
        "gray-800" => (0.278, 0.033, 256.848),
        "sky-300" => (0.828, 0.111, 230.318),
        "sky-500" => (0.685, 0.169, 237.323),
        "sky-700" => (0.5, 0.134, 242.749),
        "orange-500" => (0.705, 0.213, 47.604),
        "red-300" => (0.808, 0.114, 19.571),
        "red-700" => (0.505, 0.213, 27.518),
        _ => (0.5, 0.0, 0.0),
    };
    rune::oklch(l, c, h)
}

/// `text-<c>-700 dark:text-<c>-300`.
pub fn ink(tk: &Tk, hue: &str) -> Color {
    tw(&format!("{hue}-{}", if tk.dark { 300 } else { 700 }))
}

pub fn amber_text(tk: &Tk) -> Color {
    ink(tk, "amber")
}

/// `text-amber-600 dark:text-amber-400` (warning icons).
pub fn amber_icon(tk: &Tk) -> Color {
    tw(if tk.dark { "amber-400" } else { "amber-600" })
}

/// `text-emerald-600 dark:text-emerald-400`.
pub fn emerald_icon(tk: &Tk) -> Color {
    tw(if tk.dark {
        "emerald-400"
    } else {
        "emerald-600"
    })
}

// ---- Buttons -----------------------------------------------------------------------------------

/// shadcn `Button size="sm"` (`h-8`).
pub fn btn(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    widgets::button_with(
        tk,
        key,
        variant,
        Size::Sm,
        glyph,
        Some(tk.r(1.0)),
        label,
        enabled,
    )
}

/// `Button size="sm"` with a 0.875 rem icon (`size-3.5`).
pub fn btn35(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    widgets::button_with(
        tk,
        key,
        variant,
        Size::Sm,
        glyph,
        Some(tk.r(0.875)),
        label,
        enabled,
    )
}

/// `Button size="sm" className="h-7 gap-1.5 text-xs"` (the inline retry / check-again buttons).
pub fn btn_xs(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    widgets::button_with(
        tk,
        key,
        variant,
        Size::Xs,
        glyph,
        Some(tk.r(0.875)),
        label,
        enabled,
    )
    .gap(tk.r(0.375))
    .px(tk.r(0.625))
}

/// The default size (`h-9`).
pub fn btn_md(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    widgets::button(tk, key, variant, Size::Default, glyph, label, enabled)
}

/// A button whose leading glyph is the spinner while `busy`.
pub fn btn_busy(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    busy: bool,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    if !busy {
        return btn(tk, key, variant, glyph, label, enabled);
    }
    let fg = if variant == Variant::Default {
        tk.primary_fg
    } else {
        tk.fg
    };
    let mut b = widgets::button_surface(
        tk,
        row()
            .key(key.into())
            .h(tk.r(2.0))
            .px(tk.r(0.625))
            .gap(tk.r(0.375))
            .center(),
        variant,
    )
    .role("button")
    .label(label.to_owned())
    .busy(true)
    .focusable(enabled)
    .disabled(!enabled);
    if !enabled {
        b = b.opacity(0.5);
    }
    b.child(spinner(tk, tk.r(1.0), fg))
        .child(text(label.to_owned()).font(tk.tsm(500.0)).color(fg))
}

/// A ghost/outline square icon button.
pub fn icon_btn(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    side: f32,
    glyph: &'static Icon,
    glyph_size: f32,
    label: String,
) -> El {
    widgets::icon_button(tk, key, variant, side, glyph, glyph_size, label)
}

/// `Loader2 animate-spin`.
pub fn spinner(tk: &Tk, size: f32, color: Color) -> El {
    let _ = tk;
    ic(&I::LOADER_CIRCLE, size, color).looping(rune::LoopSpec::rotation(0.9))
}

/// `Skeleton`: `bg-accent rounded-md`.
pub fn skeleton(tk: &Tk, w: f32, h: f32) -> El {
    el().w(w).h(h).radius(tk.r(0.375)).fill(tk.accent)
}

// ---- Badges, chips, pills ----------------------------------------------------------------------

/// The developer badge: `Badge variant="outline" border-sky-500/40 text-2xs font-normal
/// text-sky-700 dark:text-sky-300`.
pub fn dev_badge(tk: &Tk) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .border(1.0, tw("sky-500").alpha(0.4))
        .child(
            text(crate::i18n::t("review:transfersPage.checked.developer"))
                .font(tk.t2xs(400.0))
                .color(ink(tk, "sky")),
        )
}

/// `surfaceCountClassName`: the count beside a card's title.
pub fn count_chip(tk: &Tk, label: String) -> El {
    let (fg, bg, ring) = if tk.dark {
        (
            tw("gray-400"),
            tw("gray-800").alpha(0.5),
            tw("gray-400").alpha(0.2),
        )
    } else {
        (tw("gray-600"), tw("gray-50"), tw("gray-500").alpha(0.1))
    };
    row()
        .ml(tk.r(0.25))
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .radius(tk.r(0.375))
        .fill(bg)
        .border(1.0, ring)
        .child(
            text(label)
                .font(tk.txs(500.0).features("'tnum' 1"))
                .color(fg),
        )
}

/// `surfaceFilterChipClassName` with an X: a removable filter chip.
pub fn filter_chip(tk: &Tk, key: impl Into<String>, label: &str, name: String) -> El {
    row()
        .key(key.into())
        .gap(tk.r(0.25))
        .center_y()
        .px(tk.r(0.5))
        .py(tk.r(0.25))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.background)
        .hover(tk.muted)
        .role("button")
        .label(name)
        .focusable(true)
        .child(
            text(label.to_owned())
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        )
        .child(ic(&I::X, tk.r(0.75), tk.muted_fg))
}
/// The amber icon box of a held question (`size-8 rounded-md border-amber-600/20 bg-amber-50`).
pub fn amber_box(tk: &Tk, glyph: &'static Icon) -> El {
    let (bg, fg) = if tk.dark {
        (tw("amber-900").alpha(0.25), tw("amber-300"))
    } else {
        (tw("amber-50"), tw("amber-700"))
    };
    row()
        .size(tk.r(2.0), tk.r(2.0))
        .mt(tk.r(0.125))
        .center()
        .radius(tk.r(0.375))
        .border(1.0, tw("amber-600").alpha(0.2))
        .fill(bg)
        .a11y_hidden()
        .child(ic(glyph, tk.r(1.0), fg))
}

#[derive(Clone, Copy)]
pub enum PillTone {
    Warning,
}

/// `ReadinessPill` as a button (`h-7 rounded-md border px-2 text-xs font-medium`).
#[allow(clippy::too_many_arguments)]
pub fn readiness_pill(
    tk: &Tk,
    key: &str,
    _tone: PillTone,
    glyph: &'static Icon,
    title: &str,
    description: &str,
    busy: bool,
    busy_title: Option<&str>,
) -> El {
    let fg = ink(tk, "amber");
    let mut icon = ic(glyph, tk.r(1.0), fg);
    if busy {
        icon = icon.looping(rune::LoopSpec::rotation(0.9));
    }
    let label = if busy {
        busy_title.unwrap_or(title)
    } else {
        title
    };
    row()
        .key(key.to_owned())
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tw("amber-500").alpha(0.25))
        .fill(tw("amber-500").alpha(0.1))
        .role("button")
        .label(label.to_owned())
        .help(description.to_owned())
        .focusable(!busy)
        .disabled(busy)
        .opacity(if busy { 0.6 } else { 1.0 })
        .hover_group()
        .child(icon)
        .child(text(label.to_owned()).font(tk.txs(500.0)).color(fg))
}

/// A row detail behind an icon: the text shows on hover or keyboard focus (`RowHint`).
pub fn row_hint(
    tk: &Tk,
    key: impl Into<String>,
    label: String,
    glyph: &'static Icon,
    color: Color,
) -> El {
    row()
        .key(key.into())
        .size(tk.r(1.25), tk.r(1.25))
        .center()
        .radius(tk.r(0.125))
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .hover_group()
        .on_press(|_: &mut App| {})
        .child(ic(glyph, tk.r(0.875), color).group_color(tk.fg))
}

// ---- Surfaces ----------------------------------------------------------------------------------

/// `.kb-surface overflow-hidden`.
pub fn surface(tk: &Tk) -> El {
    crate::ui::surface(tk).clip_rounded(tk.r(0.75))
}

/// `SurfaceCardHeader`: `sm:py-3.5` and the card padding at the sides (for `sm:px-6`), the title (`text-sm font-medium
/// sm:text-base`) with its count and extras, actions on the right.
pub fn card_header(
    tk: &Tk,
    title: String,
    count: Option<String>,
    extras: Vec<El>,
    actions: Option<El>,
) -> El {
    let mut left = row()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(
            text(title)
                .font(tk.tbase(500.0))
                .color(tk.fg)
                .role("heading"),
        );
    if let Some(c) = count {
        left = left.child(count_chip(tk, c));
    }
    left = left.children(extras);
    let mut r = row()
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.875))
        .gap(tk.r(1.0))
        .center_y()
        .child(left);
    if let Some(a) = actions {
        r = r.child(row().gap(tk.r(0.5)).center_y().child(a));
    }
    r
}

/// A 1 px top border line inside a card.
pub fn rule(tk: &Tk) -> El {
    el().w_full().h(1.0).fill(tk.border)
}

// ---- Rails, wallets, amounts -------------------------------------------------------------------

/// `RailIcon size="compact"`: the rail's logo on its own (no white disc or ring), see
/// `crate::logos::rail`.
pub fn rail_icon(app: &App, cx: &Cx<App>, tk: &Tk, rail: Rail) -> El {
    let logo = match rail {
        Rail::Lightning => crate::logos::RailLogo::Lightning,
        Rail::Liquid => crate::logos::RailLogo::Liquid,
        Rail::Onchain => crate::logos::RailLogo::Onchain,
    };
    crate::logos::rail(app, cx, tk, logo, rail.label())
}

/// A wallet name on one line, cut in the middle so the words that tell wallets apart stay
/// (`WalletName` / `RouteSide`'s split name).
pub fn wallet_name(_tk: &Tk, name: &str, style: TextStyle, color: Color, hidden: bool) -> El {
    let (head, tail) = super::model::split_wallet_name(name);
    if hidden {
        return sensitive(name.to_owned(), style, color, true).truncate();
    }
    let mut r = row()
        .min_w(0.0)
        .shrink()
        .help_when_truncated(name.to_owned())
        .child(
            text(head)
                .font(style)
                .color(color)
                .truncate()
                .min_w(if tail.is_empty() {
                    0.0
                } else {
                    style.size * 2.5
                })
                .shrink(),
        );
    if !tail.is_empty() {
        r = r.child(text(tail).font(style).color(color).single_line());
    }
    r
}

/// `wallet_name` fitted to a known width: the name whole when it fits, else the start cut and
/// the end kept, else the start at 2.5 em and the end cut too (the web's `min-w-[2.5em]` start
/// and `shrink-[0.01]` end).
pub fn wallet_name_fit(name: &str, style: TextStyle, color: Color, hidden: bool, avail: f32) -> El {
    let (head, tail) = super::model::split_wallet_name(name);
    if hidden {
        return sensitive(name.to_owned(), style, color, true).truncate();
    }
    let width = |s: &str| rune::text::measure(s, &[], style, None).0;
    let (hw, tw) = (width(&head), width(&tail));
    if tail.is_empty() || hw + tw <= avail {
        let mut r = row()
            .min_w(0.0)
            .max_w(avail)
            .help_when_truncated(name.to_owned())
            .child(
                text(head)
                    .font(style)
                    .color(color)
                    .truncate()
                    .max_w(avail)
                    .min_w(0.0)
                    .shrink(),
            );
        if !tail.is_empty() {
            r = r.child(text(tail).font(style).color(color).single_line());
        }
        return r;
    }
    let min_head = style.size * 2.5;
    let (head_w, tail_w) = if min_head + tw <= avail {
        (avail - tw, tw)
    } else {
        (min_head, (avail - min_head).max(0.0))
    };
    row()
        .min_w(0.0)
        .help(name.to_owned())
        .child(text(head).font(style).color(color).truncate().w(head_w))
        .child(text(tail).font(style).color(color).truncate().w(tail_w))
}
/// A transfer's two sides stacked, from over to, so each name gets the cell's whole width: the
/// rail logos sit in one column in the table kit's leading slot, joined by a thin line with an
/// arrowhead into the lower logo (an owner request: the direction reads as one connected route,
/// not a loose corner arrow), and each wallet name is fitted to what is left (cut in the
/// middle, the whole name in a tooltip). `cell_w` is the cell's inner width.
#[allow(clippy::too_many_arguments)]
pub fn route_stack(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    from: &str,
    to: &str,
    from_rail: Rail,
    to_rail: Rail,
    strong: bool,
    cell_w: f32,
) -> El {
    use crate::ui::table::{LEAD, LEAD_GAP};
    let hidden = app.prefs.hide_sensitive();
    let style = if strong { tk.tsm(500.0) } else { tk.tsm(400.0) };
    let (slot, gap) = (tk.r(LEAD), tk.r(0.375));
    let name_w = (cell_w - slot - tk.r(LEAD_GAP)).max(tk.r(2.5));
    let logo = |rail: Rail| {
        row()
            .size(slot, slot)
            .center()
            .child(rail_icon(app, cx, tk, rail))
    };
    // The connector runs from the upper logo's centre to the lower one's, behind both logos, so
    // only the stretch between them shows; the arrowhead sits just above the lower logo.
    let ink = tk.muted_fg.alpha(0.7);
    let line_w = 1.5_f32;
    let head = tk.r(0.625);
    let rails = col()
        .w(slot)
        .h(2.0 * slot + gap)
        .child(
            el().absolute((slot - line_w) / 2.0, slot / 2.0)
                .size(line_w, slot + gap - head * 0.35)
                .fill(ink)
                .a11y_hidden(),
        )
        .child(
            row()
                .absolute((slot - head) / 2.0, slot + gap - head * 0.55)
                .size(head, head)
                .center()
                .child(ic(&I::CHEVRON_DOWN, head, ink))
                .a11y_hidden(),
        )
        .child(logo(from_rail).absolute(0.0, 0.0))
        .child(logo(to_rail).absolute(0.0, slot + gap));
    let line = |name: &str| {
        row()
            .h(slot)
            .w_full()
            .min_w(0.0)
            .center_y()
            .child(wallet_name_fit(name, style, tk.fg, hidden, name_w))
    };
    row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(LEAD_GAP))
        .label(format!(
            "{from} {} {to}",
            crate::i18n::t("review:inbox.transfers.to")
        ))
        .child(rails)
        .child(
            col()
                .min_w(0.0)
                .grow(1.0)
                .gap(gap)
                .child(line(from))
                .child(line(to)),
        )
}

/// `BtcFigure` / `AssetFigure`: bitcoin to the sat, its leading zeros dimmed.
pub fn asset_figure(
    tk: &Tk,
    msat: i128,
    asset: &str,
    style: TextStyle,
    color: Color,
    hidden: bool,
) -> El {
    if hidden {
        return sensitive(super::model::msat_in(msat, asset), style, color, true);
    }
    if !super::model::is_bitcoin(asset) {
        return text(super::model::msat_asset(msat, asset, false))
            .font(style)
            .color(color);
    }
    match super::model::btc_figure_parts(msat) {
        (Some(lead), rest) => row()
            .label(super::model::msat_btc(msat))
            .child(
                text(lead)
                    .font(style)
                    .color(tk.muted_fg.alpha(0.7))
                    .a11y_hidden(),
            )
            .child(text(rest).font(style).color(color).a11y_hidden()),
        (None, all) => text(all).font(style).color(color),
    }
}

// ---- Menus -------------------------------------------------------------------------------------

pub type Action = Rc<dyn Fn(&mut App)>;

pub enum Entry {
    Label(String),
    Sep,
    /// An item with an optional icon, a second line and a trailing badge.
    Item {
        key: String,
        glyph: Option<&'static Icon>,
        label: String,
        hint: Option<String>,
        badge: bool,
        disabled: bool,
        title: Option<String>,
        on: Action,
    },
    /// A radio item with a muted trailing text.
    Radio {
        key: String,
        label: String,
        trailing: Option<String>,
        checked: bool,
        disabled: bool,
        on: Action,
    },
}

impl Entry {
    pub fn item(
        key: impl Into<String>,
        glyph: Option<&'static Icon>,
        label: String,
        on: impl Fn(&mut App) + 'static,
    ) -> Entry {
        Entry::Item {
            key: key.into(),
            glyph,
            label,
            hint: None,
            badge: false,
            disabled: false,
            title: None,
            on: Rc::new(on),
        }
    }
}

/// `kb-glass-popover rounded-md border bg-popover p-1`.
pub fn menu_surface(tk: &Tk, content: El) -> El {
    content
        .padding(tk.r(0.25) + 1.0)
        .radius(tk.r(0.375))
        .fill(tk.popover)
        .border(1.0, tk.fg.alpha(0.10))
        .box_shadow(Shadow {
            y: if tk.dark { 18.0 } else { 16.0 },
            blur: if tk.dark { 44.0 } else { 40.0 },
            spread: -18.0,
            color: Color::BLACK.alpha(if tk.dark { 0.8 } else { 0.55 }),
            ..Shadow::default()
        })
}

/// Mounts an open dropdown menu anchored to `anchor`, with the Radix close rules (scrim,
/// Escape through `close`, focus back to the trigger). `width` is the outer width.
#[allow(clippy::too_many_arguments)]
pub fn menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    width: Option<f32>,
    entries: Vec<Entry>,
    close: fn(&mut App),
) {
    menu_of(
        crate::ui::popup::Kind::Menu,
        tk,
        layer,
        key,
        anchor,
        placement,
        width,
        entries,
        close,
    );
}

/// [`menu`] for a web `Select` drawn with these rows: a listbox whose radio rows are options,
/// opening on the selected one.
#[allow(clippy::too_many_arguments)]
pub fn select_list(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    width: Option<f32>,
    entries: Vec<Entry>,
    close: fn(&mut App),
) {
    menu_of(
        crate::ui::popup::Kind::Listbox,
        tk,
        layer,
        key,
        anchor,
        placement,
        width,
        entries,
        close,
    );
}

#[allow(clippy::too_many_arguments)]
fn menu_of(
    kind: crate::ui::popup::Kind,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    width: Option<f32>,
    entries: Vec<Entry>,
    close: fn(&mut App),
) {
    let row_pad = tk.r(0.375);
    let mut p = crate::ui::popup::Popup::new(kind, anchor, close).list_key(format!("{key}-list"));
    let mut list = col();
    if let Some(w) = width {
        list = list.w(w - 2.0 * (tk.r(0.25) + 1.0));
    }
    for e in entries {
        match e {
            Entry::Label(l) => {
                p.add(
                    row()
                        .w_full()
                        .px(tk.r(0.5))
                        .py(row_pad)
                        .child(text(l).font(tk.tsm(500.0)).color(tk.fg)),
                );
            }
            Entry::Sep => {
                p.add(el().h(1.0).w_full().my(tk.r(0.25)).fill(tk.border));
            }
            Entry::Item {
                key,
                glyph,
                label,
                hint,
                badge,
                disabled,
                title,
                on,
            } => {
                let mut r = row()
                    .w_full()
                    .px(tk.r(0.5))
                    .py(row_pad)
                    .gap(tk.r(0.5))
                    .center_y()
                    .radius(tk.r(0.25))
                    .role("menuitem")
                    .label(label.clone());
                if disabled {
                    r = r.opacity(0.5);
                } else {
                    r = r.hover(tk.accent).focus_highlight(tk.accent).on_press(
                        move |a: &mut App| {
                            close(a);
                            on(a);
                        },
                    );
                }
                if let Some(t) = title {
                    r = r.help(t);
                }
                if let Some(g) = glyph {
                    r = r.child(ic(g, tk.r(1.0), tk.muted_fg));
                }
                let mut line = row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(text(label.clone()).font(tk.tsm(400.0)).color(tk.fg));
                if badge {
                    line = line.child(dev_badge(tk));
                }
                r = match hint {
                    Some(h) => r.child(
                        col()
                            .min_w(0.0)
                            .grow(1.0)
                            .child(line)
                            .child(text(h).font(tk.t2xs(400.0)).color(tk.muted_fg).wrap()),
                    ),
                    None => r.child(line),
                };
                p.item(r, key, label, !disabled, false);
            }
            Entry::Radio {
                key,
                label,
                trailing,
                checked,
                disabled,
                on,
            } => {
                let mut r = row()
                    .w_full()
                    .pl(tk.r(2.0))
                    .pr(tk.r(0.5))
                    .py(row_pad)
                    .gap(tk.r(0.5))
                    .center_y()
                    .radius(tk.r(0.25))
                    .role(if kind == crate::ui::popup::Kind::Listbox {
                        "option"
                    } else {
                        "menuitemradio"
                    })
                    .checked(checked)
                    .label(match &trailing {
                        Some(t) => format!("{label} {t}"),
                        None => label.clone(),
                    });
                if disabled {
                    r = r.opacity(0.5);
                } else {
                    r = r.hover(tk.accent).focus_highlight(tk.accent).on_press(
                        move |a: &mut App| {
                            close(a);
                            on(a);
                        },
                    );
                }
                if checked {
                    r = r.child(
                        row()
                            .absolute(tk.r(0.5), 0.0)
                            .size(tk.r(0.875), tk.r(1.25) + 2.0 * row_pad)
                            .center()
                            .child(crate::ui::dot(tk.r(0.5), tk.fg)),
                    );
                }
                r = r.child(
                    text(label.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .grow(1.0),
                );
                if let Some(t) = trailing {
                    r = r.child(
                        text(t)
                            .font(tk.txs(400.0).features("'tnum' 1"))
                            .color(tk.muted_fg),
                    );
                }
                p.item(r, key, label, !disabled, checked);
            }
        }
    }
    p.mount(
        tk,
        layer,
        list,
        |l| {
            let s = menu_surface(tk, col().min_h(0.0).child(l)).key(key.to_owned());
            if let Some(w) = width { s.w(w) } else { s }
        },
        placement,
        4.0,
    );
}

// ---- Dialogs -----------------------------------------------------------------------------------

/// `DialogContent` at `max_w` (rem): a faint backdrop, then the glass dialog surface at
/// `top: max(1rem, 8vh)` with the `top-3 right-3` close button.
pub fn dialog(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    max_w: f32,
    content: El,
    close: fn(&mut App),
) {
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        crate::shell::window_backdrop(format!("{key}-scrim"), wash)
            .on_press(close)
            .role("none"),
    );
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let top = crate::shell::dialog_top(tk, win_h);
    let (border, shadows) = if tk.dark {
        (
            Color::WHITE.alpha(0.08),
            vec![
                Shadow {
                    y: 1.0,
                    color: Color::WHITE.alpha(0.04),
                    inset: true,
                    ..Default::default()
                },
                Shadow {
                    y: 24.0,
                    blur: 72.0,
                    spread: -20.0,
                    color: Color::BLACK.alpha(0.9),
                    ..Default::default()
                },
            ],
        )
    } else {
        (
            tk.fg.alpha(0.1),
            vec![Shadow {
                y: 24.0,
                blur: 64.0,
                spread: -24.0,
                color: Color::BLACK.alpha(0.65),
                ..Default::default()
            }],
        )
    };
    let close_btn = widgets::icon_button(
        tk,
        format!("{key}-close"),
        Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        crate::i18n::t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .w(tk.r(max_w).min(win_w - tk.r(2.0)))
        .max_h(win_h - top - tk.r(1.0))
        .padding(tk.r(1.25))
        .gap(tk.r(1.0))
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .modal(true, close)
        .child(content)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    // Its own layer root, so its entrance and exit carry the blur (shell::centered).
    let width = tk.r(max_w).min(win_w - tk.r(2.0));
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// `DialogHeader`: title (`text-lg leading-none font-semibold`) and description, `gap-2`.
pub fn dialog_header(tk: &Tk, title: String, desc: Option<String>) -> El {
    let mut c = col().w_full().gap(tk.r(0.5)).pr(tk.r(1.5)).child(
        text(title)
            .font(tk.tlg(600.0).lh(tk.r(1.125)))
            .color(tk.fg)
            .wrap()
            .role("heading"),
    );
    if let Some(d) = desc {
        c = c.child(text(d).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());
    }
    c
}

/// `DialogFooter`: `flex-row justify-end gap-2`.
pub fn dialog_footer(tk: &Tk) -> El {
    row().w_full().justify_end().gap(tk.r(0.5))
}

/// The page's content width before the first layout: the window less the sidebar, the panel's
/// inset and border, and the page gutters (`reports::ledger::common::page_w`).
pub fn page_w(app: &App, cx: &Cx<App>, tk: &Tk) -> f32 {
    let nav = if crate::shell::nav_folded(app, cx) {
        tk.r(3.0)
    } else {
        tk.r(16.0)
    };
    let win = cx
        .layout_size("sh-root")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        });
    win - nav - tk.r(0.375) - 2.0 - 2.0 * tk.page_pad()
}

/// `CurrencyToggleText`: the amount names the control; pressing it switches the display
/// currency, which its tooltip says.
pub fn currency_toggle(
    app: &App,
    tk: &Tk,
    key: impl Into<String>,
    name: String,
    content: El,
) -> El {
    use crate::app::Currency;
    let hint = crate::i18n::t(if app.prefs.currency == Currency::Btc {
        "chrome:currencyToggleText.showFiat"
    } else {
        "chrome:currencyToggleText.showBitcoin"
    });
    let hidden = app.prefs.hide_sensitive();
    row()
        .key(key.into())
        .radius(tk.r(0.125))
        .role("button")
        .label(if hidden {
            crate::ui::hidden_amount()
        } else {
            name
        })
        .help(hint)
        .focusable(true)
        .on_press(|a: &mut App| {
            a.prefs.currency = if a.prefs.currency == Currency::Btc {
                Currency::Fiat
            } else {
                Currency::Btc
            };
        })
        .child(content)
}

/// `formatDisplayMoney` in fiat (the web formats it with its de-AT EUR formatter).
pub fn money(value: f64) -> String {
    crate::format::fiat(value, "EUR")
}
