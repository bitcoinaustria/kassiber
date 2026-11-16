//! Form pieces the BTCPay and exchange setup panels share: `SetupField`, shadcn `Input`, the
//! native `<select>` the panels use (`SELECT_CLASS`), `Badge`, tone notices and buttons with a
//! `secondary` variant.

use rune::{
    Color, El, Icon, Shadow, Span, TextEdit, TextStyle, col, el, rich, row, text, text_input,
};
use rune_kit::Placement;

use super::model::Tone;
use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

fn hx(s: &str) -> Color {
    rune::color::hex(s).expect("hex")
}

/// `TONE_CLASSES`: (border, fill, ink).
pub fn tone_colors(tk: &Tk, tone: Tone) -> (Color, Color, Color) {
    match tone {
        Tone::Good => (
            hx("#10b981").alpha(0.4),
            hx("#10b981").alpha(0.1),
            if tk.dark {
                hx("#a7f3d0")
            } else {
                hx("#065f46")
            },
        ),
        Tone::Warning => (
            hx("#f59e0b").alpha(0.4),
            hx("#f59e0b").alpha(0.1),
            if tk.dark {
                hx("#fde68a")
            } else {
                hx("#78350f")
            },
        ),
        Tone::Danger => (
            tk.destructive.alpha(0.5),
            tk.destructive.alpha(0.1),
            tk.destructive,
        ),
        Tone::Neutral => (tk.border, tk.muted.alpha(0.4), tk.muted_fg),
    }
}

/// `text-amber-700 dark:text-amber-300`.
pub fn amber_ink(tk: &Tk) -> Color {
    if tk.dark {
        hx("#fcd34d")
    } else {
        hx("#b45309")
    }
}

/// `Notice`: `rounded-md border px-3 py-2 text-xs` in a tone.
pub fn notice(tk: &Tk, tone: Tone, body: impl Into<String>) -> El {
    notice_with(
        tk,
        tone,
        vec![
            text(body.into())
                .font(tk.txs(400.0))
                .color(tone_colors(tk, tone).2)
                .wrap(),
        ],
    )
}

/// `Notice` around several children (`space-y-1`).
pub fn notice_with(tk: &Tk, tone: Tone, children: Vec<El>) -> El {
    let (b, f, _) = tone_colors(tk, tone);
    let mut c = col()
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .fill(f)
        .border(1.0, b)
        .border_layout();
    for ch in children {
        c = c.child(ch);
    }
    c
}

/// `text-xs` line in a colour, wrapping.
pub fn xs(tk: &Tk, s: impl Into<String>, color: Color) -> El {
    text(s.into()).font(tk.txs(400.0)).color(color).wrap()
}

/// `text-sm font-medium` line.
pub fn sm_medium(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into()).font(tk.tsm(500.0)).color(tk.fg).wrap()
}

/// `text-xs text-destructive` (optionally `role="alert"`).
pub fn error_line(tk: &Tk, s: impl Into<String>, alert: bool) -> El {
    let e = text(s.into())
        .font(tk.txs(400.0))
        .color(tk.destructive)
        .wrap();
    if alert { e.role("alert") } else { e }
}

/// shadcn `Label` (`text-sm leading-none font-medium`).
pub fn label(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tsm(500.0).lh(tk.r(0.875)))
        .color(tk.fg)
}

/// `SetupField`: `space-y-2`, the label, the control, then the helper (hidden by an error)
/// or the error.
pub fn setup_field(
    tk: &Tk,
    label_text: &str,
    control: El,
    error: Option<String>,
    helper: Option<String>,
) -> El {
    let mut c = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .child(label(tk, label_text))
        .child(control);
    match (error, helper) {
        (Some(e), _) => c = c.child(error_line(tk, e, false)),
        (None, Some(h)) => c = c.child(xs(tk, h, tk.muted_fg)),
        _ => {}
    }
    c
}

/// shadcn `Input` (`h-9 w-full rounded-md border border-input px-3 py-1 text-sm shadow-xs`,
/// `dark:bg-input/30`). `secure` masks it (`type="password"`).
#[allow(clippy::too_many_arguments)]
pub fn input(
    tk: &Tk,
    key: &str,
    edit: &TextEdit,
    access: impl for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
    name: &str,
    placeholder: Option<&str>,
    secure: bool,
    on_change: impl Fn(&mut App) + 'static,
) -> El {
    let mut field = text_input(edit, tk.tsm(400.0))
        .key(key.to_owned())
        .single_line()
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(name.to_owned())
        .on_edit(access)
        .on_input_change(move |a: &mut App, _tx: &mut rune::Tx| on_change(a))
        .grow(1.0)
        .min_w(0.0);
    if let Some(p) = placeholder {
        field = field.placeholder(p.to_owned(), tk.muted_fg);
    }
    if secure {
        field = field.secure();
    }
    let fill = if tk.dark {
        tk.input.alpha(tk.input.a * 0.3)
    } else {
        Color::TRANSPARENT
    };
    row()
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .center_y()
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, if edit.focused { tk.ring } else { tk.input })
        .border_layout()
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
        .child(field)
}

/// One option of a native select: (value, label, enabled, title).
pub type Opt = (String, String, bool, Option<String>);

/// The panels' native `<select>` (`h-9 w-full rounded-md border border-input bg-background
/// px-3 text-sm`, disabled `opacity-60`), drawn as a trigger with the platform chevron.
pub fn select_trigger(
    tk: &Tk,
    key: &str,
    name: &str,
    shown: &str,
    enabled: bool,
    muted: bool,
) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .fill(tk.background)
        .border(1.0, tk.input)
        .border_layout()
        .role("combobox")
        .label(name.to_owned())
        .a11y_value(shown.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.6 })
        .child(
            text(shown.to_owned())
                .font(tk.tsm(400.0))
                .color(if muted { tk.muted_fg } else { tk.fg })
                .truncate()
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), tk.fg))
}

/// Mounts a select's option list under its trigger (`anchor`). `close` clears the open state;
/// `choose` gets the picked value.
#[allow(clippy::too_many_arguments)]
pub fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    width: f32,
    options: Vec<Opt>,
    current: &str,
    close: fn(&mut App),
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let mut p = crate::ui::popup::Popup::listbox(anchor, close);
    let list = col().w(width).padding(tk.r(0.25)).label(anchor.to_owned());
    for (v, lbl, enabled, title) in options {
        let checked = v == current;
        let pick = choose.clone();
        let val = v.clone();
        let mut r = row()
            .w_full()
            .min_h(tk.r(2.0))
            .pr(tk.r(0.5))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.25))
            .role("option")
            .checked(checked)
            .label(lbl.clone())
            .disabled(!enabled)
            .opacity(if enabled { 1.0 } else { 0.5 })
            .child(row().w(tk.r(1.5)).center().child(if checked {
                ic(&I::CHECK, tk.r(0.875), tk.fg)
            } else {
                el().size(1.0, 1.0)
            }))
            .child(
                text(lbl.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .grow(1.0)
                    .min_w(0.0)
                    .truncate(),
            );
        if let Some(t) = title {
            r = r.help(t);
        }
        if enabled {
            r = r
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .focusable(true)
                .focus_selected(checked)
                .on_press(move |a: &mut App| {
                    close(a);
                    pick(a, &val);
                });
        }
        p.item(r, format!("{anchor}-opt-{v}"), lbl, enabled, checked);
    }
    let p = p.max_h(tk.r(22.0));
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        4.0,
    );
}

/// shadcn `Button` variants used here.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Secondary,
    Outline,
    Ghost,
}

/// shadcn `Button`: `size` default (`h-9 px-4`) or `sm` (`h-8 px-3 gap-1.5`), an optional
/// leading glyph (`spin` animates it), disabled at 50 %.
#[allow(clippy::too_many_arguments)]
pub fn button(
    tk: &Tk,
    key: &str,
    variant: Variant,
    small: bool,
    glyph: Option<(&'static Icon, f32, bool)>,
    label: &str,
    enabled: bool,
) -> El {
    let (h, px) = if small {
        (
            tk.r(2.0),
            if glyph.is_some() {
                tk.r(0.625)
            } else {
                tk.r(0.75)
            },
        )
    } else {
        (
            tk.r(2.25),
            if glyph.is_some() {
                tk.r(0.75)
            } else {
                tk.r(1.0)
            },
        )
    };
    let ink = tk.fg;
    let mut b = row()
        .key(key.to_owned())
        .h(h)
        .px(px)
        .gap(if small { tk.r(0.375) } else { tk.r(0.5) })
        .center()
        .radius(tk.r(0.375));
    b = match variant {
        Variant::Secondary => b.fill(tk.muted).hover(tk.muted.alpha(0.8)),
        Variant::Outline => crate::txdetail::widgets::button_surface(
            tk,
            b,
            crate::txdetail::widgets::Variant::Outline,
        ),
        Variant::Ghost => crate::txdetail::widgets::button_surface(
            tk,
            b,
            crate::txdetail::widgets::Variant::Ghost,
        ),
    };
    b = b
        .role("button")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled);
    if !enabled {
        b = b.opacity(0.5);
    }
    if let Some((g, size, spin)) = glyph {
        let icon = ic(g, size, ink);
        b = b.child(if spin {
            icon.looping(rune::LoopSpec::rotation(0.9))
        } else {
            icon
        });
    }
    b.child(text(label.to_owned()).font(tk.tsm(500.0)).color(ink))
}

/// `Badge variant="outline"` (`rounded-md border px-2 py-0.5 text-xs font-medium`), optionally
/// in a tone, mono (`font-mono text-[11px]`) or struck through (`line-through opacity-60`).
pub fn badge(tk: &Tk, label: &str, tone: Option<Tone>, mono: bool, struck: bool) -> El {
    let (border, fill, ink) = match tone {
        Some(t) => tone_colors(tk, t),
        None => (tk.border, Color::TRANSPARENT, tk.fg),
    };
    let font = if mono {
        tk.px(11.0 * tk.rem / 16.0, 500.0).mono()
    } else {
        tk.txs(500.0)
    };
    let t = text(label.to_owned()).font(font).color(ink);
    // `line-through`: Rune exposes no strikethrough decoration, so a 1 px rule at mid-height.
    let t: El = if struck {
        col().child(t).child(
            el().absolute(0.0, tk.r(0.5))
                .w_full()
                .h(1.0)
                .fill(ink)
                .a11y_hidden(),
        )
    } else {
        t
    };
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .opacity(if struck { 0.6 } else { 1.0 })
        .child(t)
}

/// `Badge variant="secondary"`.
pub fn badge_secondary(tk: &Tk, label: &str) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .fill(tk.muted)
        .border(1.0, Color::TRANSPARENT)
        .border_layout()
        .child(text(label.to_owned()).font(tk.txs(500.0)).color(tk.fg))
}

/// A link-styled inline button (`inline-flex items-center gap-1 text-xs underline`).
pub fn link_button(tk: &Tk, key: &str, glyph: &'static Icon, label: &str, ink: Color) -> El {
    row()
        .key(key.to_owned())
        .gap(tk.r(0.25))
        .center_y()
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .child(ic(glyph, tk.r(0.75), ink))
        .child(underlined(label, tk.txs(400.0), ink))
}

/// A two-column grid (`grid grid-cols-2 gap-N`): pairs of equal-width cells.
pub fn grid2(gap: f32, cells: Vec<El>) -> El {
    let mut c = col().w_full().gap(gap);
    let mut it = cells.into_iter();
    while let Some(a) = it.next() {
        let mut r = row()
            .w_full()
            .gap(gap)
            .items_stretch()
            .child(col().grow(1.0).min_w(0.0).child(a));
        r = match it.next() {
            Some(b) => r.child(col().grow(1.0).min_w(0.0).child(b)),
            None => r.child(col().grow(1.0).min_w(0.0)),
        };
        c = c.child(r);
    }
    c
}

/// A bulleted list item (`list-disc` / `list-decimal`): the marker then the wrapped body.
pub fn list_item(tk: &Tk, marker: &str, body: El) -> El {
    row()
        .w_full()
        .gap(tk.r(0.375))
        .items_start()
        .child(
            text(marker.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .w(tk.r(0.875))
                .a11y_hidden(),
        )
        .child(col().grow(1.0).min_w(0.0).child(body))
        .role("listitem")
}

/// `underline underline-offset-2` text.
pub fn underlined(s: &str, font: TextStyle, ink: Color) -> El {
    rich(s.to_owned(), vec![Span::new(0, s.len()).underline(ink)])
        .font(font)
        .color(ink)
}
