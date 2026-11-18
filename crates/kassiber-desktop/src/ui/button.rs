//! The app's one button: every area's button helpers draw through these, so heights, corners,
//! fills and the disabled state agree everywhere.
//!
//! - Variants: `Default` (the filled primary: one per surface), `Outline` (secondary),
//!   `Ghost` (toolbar and tertiary actions), `Destructive`.
//! - Sizes: `Xs` 1.75 rem, `Sm` 2 rem (the default in pages and toolbars), `Default` 2.25 rem
//!   (dialog footers and setup).
//! - Corners follow the size (`radius`), so a button sits with the inputs beside it.

use rune::{Color, El, Icon, row, text};

use crate::theme::Tk;
use crate::ui::ic;

/// Button variants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    Default,
    Outline,
    Ghost,
    Destructive,
}

/// Button sizes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    /// 2.25 rem, text-sm.
    Default,
    /// 2 rem, text-sm.
    Sm,
    /// 1.75 rem, text-xs, 0.75 rem glyph.
    Xs,
}

/// The corner radius of a control `h` tall: 0.5 rem from 2 rem up, 0.375 rem below.
pub fn radius(tk: &Tk, h: f32) -> f32 {
    if h >= tk.r(2.0) {
        tk.r(0.5)
    } else {
        tk.r(0.375)
    }
}

/// A button with an optional leading icon and a label.
pub fn button(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    size: Size,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    button_with(tk, key, variant, size, glyph, None, label, enabled)
}

/// `button` whose leading glyph is a spinner while `busy` (and is marked busy for assistive
/// technology).
#[allow(clippy::too_many_arguments)]
pub fn button_busy(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    size: Size,
    busy: bool,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    build(tk, key, variant, size, glyph, None, label, enabled, busy)
}

/// `button` with an explicit icon size.
#[allow(clippy::too_many_arguments)]
pub fn button_with(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    size: Size,
    glyph: Option<&'static Icon>,
    glyph_size: Option<f32>,
    label: &str,
    enabled: bool,
) -> El {
    build(
        tk, key, variant, size, glyph, glyph_size, label, enabled, false,
    )
}

#[allow(clippy::too_many_arguments)]
fn build(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    size: Size,
    glyph: Option<&'static Icon>,
    glyph_size: Option<f32>,
    label: &str,
    enabled: bool,
    busy: bool,
) -> El {
    let (h, px, gap, font, icon) = match size {
        Size::Default => (
            tk.r(2.25),
            if glyph.is_some() {
                tk.r(0.75)
            } else {
                tk.r(1.0)
            },
            tk.r(0.5),
            tk.tsm(500.0),
            tk.r(1.0),
        ),
        Size::Sm => (
            tk.r(2.0),
            if glyph.is_some() {
                tk.r(0.625)
            } else {
                tk.r(0.75)
            },
            tk.r(0.375),
            tk.tsm(500.0),
            tk.r(0.875),
        ),
        Size::Xs => (
            tk.r(1.75),
            if glyph.is_some() {
                tk.r(0.375)
            } else {
                tk.r(0.5)
            },
            tk.r(0.25),
            tk.txs(500.0),
            tk.r(0.75),
        ),
    };
    let (fg, icon_fg) = inks(tk, variant);
    let mut b = surface(
        tk,
        row().key(key.into()).h(h).px(px).gap(gap).center(),
        variant,
    )
    .radius(radius(tk, h))
    .role("button")
    .label(label.to_owned())
    .focusable(enabled)
    .disabled(!enabled);
    if !enabled {
        b = b.opacity(0.5);
    }
    if busy {
        b = b.busy(true).child(
            ic(
                &crate::icons::LOADER_CIRCLE,
                glyph_size.unwrap_or(icon),
                icon_fg,
            )
            .looping(rune::LoopSpec::rotation(0.9)),
        );
    } else if let Some(g) = glyph {
        b = b.child(ic(g, glyph_size.unwrap_or(icon), icon_fg));
    }
    b.child(text(label.to_owned()).font(font).color(fg))
}

/// A square icon button named `label` (shown as its tooltip). `side` 2.25, 2 or 1.75 rem.
pub fn icon_button(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    side: f32,
    glyph: &'static Icon,
    glyph_size: f32,
    label: String,
) -> El {
    let (_, ink) = inks(tk, variant);
    surface(tk, row().key(key.into()).size(side, side).center(), variant)
        .radius(radius(tk, side))
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .child(ic(glyph, glyph_size, ink))
}

/// Label and icon inks of a variant.
fn inks(tk: &Tk, variant: Variant) -> (Color, Color) {
    match variant {
        Variant::Default => (tk.primary_fg, tk.primary_fg),
        Variant::Outline => (tk.fg, tk.fg),
        Variant::Ghost => (tk.fg, tk.muted_fg),
        Variant::Destructive => (Color::WHITE, Color::WHITE),
    }
}

/// Fill, border and hover of a variant (the corner radius is the caller's: `radius`).
pub fn surface(tk: &Tk, el: El, variant: Variant) -> El {
    match variant {
        Variant::Default => el.fill(tk.primary).hover(tk.primary_fg.alpha(0.1)),
        Variant::Outline => {
            if tk.dark {
                el.fill(tk.input.alpha(tk.input.a * 0.3))
                    .border(1.0, tk.input)
                    .hover(tk.input.alpha(tk.input.a * 0.6))
            } else {
                el.fill(tk.card).border(1.0, tk.border).hover(tk.accent)
            }
        }
        Variant::Ghost => el.hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        }),
        Variant::Destructive => {
            let fill = if tk.dark {
                tk.destructive.alpha(0.6)
            } else {
                tk.destructive
            };
            el.fill(fill).hover(fill.alpha(0.9))
        }
    }
}
