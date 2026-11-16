//! Building blocks the Review and Reconcile screens share: shadcn buttons in the sizes these
//! screens use, link buttons, the `InfoNote` popover, a checkbox, dialogs and skeletons.

use rune::{Color, El, Icon, Shadow, col, el, row, text};
use rune_kit::Placement;

use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self, Size, Variant};
use crate::ui::ic;

pub use widgets::{Variant as Var, spinner};

/// shadcn `Button` (`size="sm"` unless said otherwise).
pub fn btn(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    widgets::button(tk, key, variant, Size::Sm, glyph, label, enabled)
}

/// shadcn `Button` in the default size (`h-9`).
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

/// A button whose leading glyph is the spinner while `busy` (`size="sm"`).
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
        return widgets::button(tk, key, variant, Size::Sm, glyph, label, enabled);
    }
    let (fg, icon_fg) = match variant {
        Variant::Default => (tk.primary_fg, tk.primary_fg),
        Variant::Outline => (tk.fg, tk.fg),
        Variant::Ghost => (tk.fg, tk.muted_fg),
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
    b.child(ic(&I::LOADER_CIRCLE, tk.r(1.0), icon_fg).looping(rune::LoopSpec::rotation(0.9)))
        .child(text(label.to_owned()).font(tk.tsm(500.0)).color(fg))
}

/// `Button variant="link" size="sm" className="h-auto p-0 text-xs"`.
pub fn link(tk: &Tk, key: impl Into<String>, label: &str, enabled: bool) -> El {
    let key = key.into();
    row()
        .key(key)
        .gap(tk.r(0.25))
        .center_y()
        .role("button")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 })
        .hover_group()
        .child(
            text(label.to_owned())
                .font(tk.txs(500.0))
                .color(tk.primary)
                .group_opacity(1.0, 0.8),
        )
}

/// A link button with an explicit accessible name (`aria-label`).
pub fn link_named(
    tk: &Tk,
    key: impl Into<String>,
    label: &str,
    name: String,
    enabled: bool,
    busy: bool,
) -> El {
    let key = key.into();
    let mut r = row()
        .key(key)
        .gap(tk.r(0.25))
        .center_y()
        .role("button")
        .label(name)
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 })
        .hover_group();
    if busy {
        r = r.child(
            ic(&I::LOADER_CIRCLE, tk.r(0.75), tk.primary).looping(rune::LoopSpec::rotation(0.9)),
        );
    }
    r.child(
        text(label.to_owned())
            .font(tk.txs(500.0))
            .color(tk.primary)
            .group_opacity(1.0, 0.8),
    )
}

/// `InfoNote`: an info button whose notes open in a popover (`w-72 p-3 text-xs space-y-1`).
pub fn info_note(
    tk: &Tk,
    key: &str,
    label: String,
    notes: &[String],
    open: bool,
    layer: &mut Vec<El>,
    toggle: fn(&mut App, &str),
) -> El {
    let k = key.to_owned();
    let button = row()
        .key(key.to_owned())
        .center()
        .radius(tk.r(0.125))
        .role("button")
        .label(label)
        .expanded(open)
        .focusable(true)
        .hover_group()
        .on_press(move |a: &mut App| toggle(a, &k))
        .child(ic(&I::INFO, tk.r(0.875), tk.muted_fg).group_color(tk.fg));
    if open {
        let close_key = key.to_owned();
        let mut body = col().w(tk.r(18.0)).padding(tk.r(0.75)).gap(tk.r(0.25));
        for n in notes {
            body = body.child(text(n.clone()).font(tk.txs(400.0)).color(tk.fg).wrap());
        }
        let surface = crate::shell::glass(tk, body)
            .modal(true, move |a: &mut App| toggle(a, &close_key))
            .focus_return(key.to_owned());
        layer.push(crate::shell::anchored(
            tk,
            surface,
            key,
            Placement::BottomStart,
            4.0,
        ));
    }
    button
}

/// shadcn `Checkbox`: `size-4 rounded-[4px] border border-input`, checked `bg-primary` with a
/// `size-3.5` check; `dark:bg-input/30` unchecked.
pub fn checkbox(
    tk: &Tk,
    key: impl Into<String>,
    checked: bool,
    enabled: bool,
    label: String,
) -> El {
    let side = tk.r(1.0);
    let mut b = row()
        .key(key.into())
        .size(side, side)
        .center()
        .radius(4.0)
        .role("checkbox")
        .checked(checked)
        .label(label)
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 });
    if checked {
        b = b.fill(tk.primary).border(1.0, tk.primary).child(ic(
            &I::CHECK,
            tk.r(0.875),
            tk.primary_fg,
        ));
    } else {
        b = b
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                Color::TRANSPARENT
            })
            .border(1.0, tk.input);
    }
    b.box_shadow(Shadow {
        y: 1.0,
        blur: 2.0,
        color: Color::BLACK.alpha(0.05),
        ..Shadow::default()
    })
}

/// `Skeleton`: `bg-accent animate-pulse rounded-md`.
pub fn skeleton(tk: &Tk, w: Option<f32>, h: f32) -> El {
    let e = el().h(h).radius(tk.r(0.375)).fill(tk.accent);
    match w {
        Some(w) => e.w(w),
        None => e.w_full(),
    }
}

/// `kb-surface-inset`: card fill, 1 px border at 70 %, 0.5 rem radius.
pub fn inset(tk: &Tk) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .radius(tk.r(0.5))
        .fill(tk.card)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
}

/// `DialogContent` (ui/dialog.tsx) at `max_w` (rem): a faint backdrop, then the glass dialog
/// surface centred at `top: max(1rem, 8vh)` with the `top-3 right-3` close button.
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
    let w = tk.r(max_w).min(win_w - tk.r(2.0));
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .w(w)
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
    layer.push(crate::shell::centered(surface, win_w, w, top));
}

/// `DialogHeader`: title (`text-lg leading-none font-semibold`) and description (`text-sm
/// text-muted-foreground`), `gap-2`.
pub fn dialog_header(tk: &Tk, title: String, desc: Option<El>) -> El {
    let mut c = col().w_full().gap(tk.r(0.5)).pr(tk.r(1.5)).child(
        text(title)
            .font(tk.tlg(600.0).lh(tk.r(1.125)))
            .color(tk.fg)
            .wrap(),
    );
    if let Some(d) = desc {
        c = c.child(d);
    }
    c
}

/// `DialogDescription`.
pub fn dialog_desc(tk: &Tk, s: String) -> El {
    text(s).font(tk.tsm(400.0)).color(tk.muted_fg).wrap()
}

/// `DialogFooter`: `flex-row justify-end gap-2`.
pub fn dialog_footer(tk: &Tk) -> El {
    row().w_full().justify_end().gap(tk.r(0.5))
}

/// Tailwind v4's palette (oklch), as the web's utilities resolve.
pub fn tw(name: &str) -> Color {
    let (l, c, h) = match name {
        "emerald-300" => (0.845, 0.143, 164.978),
        "emerald-400" => (0.765, 0.177, 163.223),
        "emerald-500" => (0.696, 0.17, 162.48),
        "emerald-600" => (0.596, 0.145, 163.225),
        "emerald-700" => (0.508, 0.118, 165.612),
        "red-100" => (0.936, 0.032, 17.717),
        "red-200" => (0.885, 0.062, 18.334),
        "red-300" => (0.808, 0.114, 19.571),
        "red-700" => (0.505, 0.213, 27.518),
        "red-800" => (0.444, 0.177, 26.899),
        "red-900" => (0.396, 0.141, 25.723),
        "red-950" => (0.258, 0.092, 26.042),
        "amber-300" => (0.879, 0.169, 91.605),
        "amber-500" => (0.769, 0.188, 70.08),
        "amber-600" => (0.666, 0.179, 58.318),
        "amber-700" => (0.555, 0.163, 48.998),
        "sky-500" => (0.685, 0.169, 237.323),
        _ => (0.5, 0.0, 0.0),
    };
    rune::oklch(l, c, h)
}

/// The amber/red/emerald text tones these screens use (`text-amber-700 dark:text-amber-300`…).
pub fn amber_text(tk: &Tk) -> Color {
    tw(if tk.dark { "amber-300" } else { "amber-700" })
}
pub fn red_text(tk: &Tk) -> Color {
    tw(if tk.dark { "red-300" } else { "red-700" })
}
pub fn emerald_text(tk: &Tk) -> Color {
    tw(if tk.dark {
        "emerald-300"
    } else {
        "emerald-700"
    })
}
