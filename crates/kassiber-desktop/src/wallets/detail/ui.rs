//! Local building blocks of the wallet detail: shadcn Input/Textarea/Checkbox/Select, the
//! Dialog frame, Card variants, Badge, CopyButton, DetailRow and the Tailwind tint helpers.

use rune::{Color, El, Icon, Shadow, TextEdit, TextStyle, col, el, row, text, text_input};

use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button, icon_button};
use crate::ui::ic;

pub fn h(s: &str) -> Color {
    rune::color::hex(s).expect("hex")
}

/// Field accessor for `text_input(..).on_edit(..)`.
pub type EditFn = fn(&mut App) -> &mut TextEdit;

/// The wallet page's `kb-surface` card (`rounded-xl border bg-card shadow`), clipped.
pub fn card(tk: &Tk) -> El {
    crate::ui::surface(tk)
        .w_full()
        .min_w(0.0)
        .clip_rounded(tk.r(0.75))
}

/// A card header strip: `border-b px-3 py-2.5 sm:px-4`.
pub fn strip(tk: &Tk) -> El {
    row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.625))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
}

pub fn card_title(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tbase(600.0).lh(tk.r(1.0)))
        .color(tk.fg)
        .role("heading")
}

/// `Badge variant="secondary"`: `bg-secondary text-secondary-foreground rounded-md px-2 py-0.5
/// text-xs font-medium`.
pub fn badge_secondary(tk: &Tk, s: &str) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .fill(tk.muted)
        .center_y()
        .child(text(s.to_owned()).font(tk.txs(500.0)).color(tk.fg))
}

/// `Badge variant="outline"`.
pub fn badge_outline(tk: &Tk, s: &str) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .center_y()
        .child(text(s.to_owned()).font(tk.txs(500.0)).color(tk.fg))
}

/// The amber notice tones (`border-amber-300 bg-amber-50 text-amber-900`, dark
/// `border-amber-900/60 bg-amber-950/40 text-amber-200`): (fill, border, ink).
pub fn amber(tk: &Tk) -> (Color, Color, Color) {
    if tk.dark {
        (
            h("#451a03").alpha(0.4),
            h("#78350f").alpha(0.6),
            h("#fde68a"),
        )
    } else {
        (h("#fffbeb"), h("#fcd34d"), h("#78350f"))
    }
}

/// The red notice tones (`border-red-200 bg-red-50 text-red-700`, dark `border-red-900/60
/// bg-red-950/40 text-red-300`).
pub fn red(tk: &Tk) -> (Color, Color, Color) {
    if tk.dark {
        (
            h("#450a0a").alpha(0.4),
            h("#7f1d1d").alpha(0.6),
            h("#fca5a5"),
        )
    } else {
        (h("#fef2f2"), h("#fecaca"), h("#b91c1c"))
    }
}

/// The emerald / red text tones (`text-emerald-600 dark:text-emerald-400`).
pub fn emerald_text(tk: &Tk) -> Color {
    if tk.dark { h("#34d399") } else { h("#059669") }
}
pub fn red_text(tk: &Tk) -> Color {
    if tk.dark { h("#f87171") } else { h("#dc2626") }
}

/// shadcn `Input` (`h-9 rounded-md border border-input px-3 py-1 text-sm`, dark `bg-input/30`).
pub fn input(
    tk: &Tk,
    key: &str,
    edit: &TextEdit,
    label: &str,
    placeholder: &str,
    access: EditFn,
    secure: bool,
) -> El {
    let mut f = text_input(edit, tk.tsm(400.0))
        .key(format!("{key}-field"))
        .single_line()
        .placeholder(placeholder.to_owned(), tk.muted_fg)
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(label.to_owned())
        .on_edit(access)
        .grow(1.0)
        .min_w(0.0);
    if secure {
        f = f.secure();
    }
    field_frame(tk, key)
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .center_y()
        .child(f)
}

/// `input` that submits its form on Return.
#[allow(clippy::too_many_arguments)]
pub fn input_submit(
    tk: &Tk,
    key: &str,
    edit: &TextEdit,
    label: &str,
    placeholder: &str,
    access: EditFn,
    secure: bool,
    submit: fn(&mut App),
) -> El {
    let mut f = text_input(edit, tk.tsm(400.0))
        .key(format!("{key}-field"))
        .single_line()
        .placeholder(placeholder.to_owned(), tk.muted_fg)
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(label.to_owned())
        .on_edit(access)
        .on_submit(move |a: &mut App, _tx: &mut rune::Tx| submit(a))
        .grow(1.0)
        .min_w(0.0);
    if secure {
        f = f.secure();
    }
    field_frame(tk, key)
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .center_y()
        .child(f)
}

/// shadcn `Textarea` (`min-h-16 rounded-md border border-input px-3 py-2`), here with the
/// caller's style (`font-mono text-xs` in the edit dialog) and minimum height.
#[allow(clippy::too_many_arguments)]
pub fn textarea(
    tk: &Tk,
    key: &str,
    edit: &TextEdit,
    label: &str,
    placeholder: &str,
    style: TextStyle,
    min_h: f32,
    access: EditFn,
) -> El {
    let f = text_input(edit, style)
        .key(format!("{key}-field"))
        .placeholder(placeholder.to_owned(), tk.muted_fg)
        .caret(tk.fg, tk.primary.alpha(0.3))
        .label(label.to_owned())
        .on_edit(access)
        .w_full();
    field_frame(tk, key)
        .min_h(min_h)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .items_start()
        .child(f)
}

fn field_frame(tk: &Tk, key: &str) -> El {
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

/// shadcn `Label` (`text-sm leading-none font-medium`).
pub fn label(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tsm(500.0).lh(tk.r(0.875)))
        .color(tk.fg)
}

/// A helper line (`text-xs text-muted-foreground`).
pub fn helper(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.txs(400.0))
        .color(tk.muted_fg)
        .wrap()
        .w_full()
}

/// `space-y-2` field group: label, control, helper.
pub fn group(tk: &Tk, items: Vec<El>) -> El {
    col().w_full().gap(tk.r(0.5)).children(items)
}

/// shadcn `Checkbox`: a 1 rem `rounded-[4px] border-input` box, `bg-primary` with a check when
/// on. `toggle` runs on press; the row around it names it.
pub fn checkbox_box(tk: &Tk, on: bool, enabled: bool) -> El {
    let b = row()
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(4.0)
        .opacity(if enabled { 1.0 } else { 0.5 });
    if on {
        b.fill(tk.primary)
            .child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b.border(1.0, tk.input)
            .border_layout()
            .fill(if tk.dark {
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
    }
}

/// A `<label>` row holding a checkbox and its texts: a checkbox control named `name`.
pub fn checkbox_row(
    tk: &Tk,
    key: &str,
    on: bool,
    enabled: bool,
    name: &str,
    content: El,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    let mut r = row()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .role("checkbox")
        .checked(on)
        .label(name.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .child(checkbox_box(tk, on, enabled).mt(tk.r(0.125)))
        .child(content);
    if enabled {
        r = r.on_press(toggle);
    }
    r
}

/// A native `<select>` trigger (`h-9 w-full rounded-md border border-input bg-background px-3
/// text-sm`), drawn as the platform draws it in Chromium: value left, chevron right.
pub fn select_trigger(tk: &Tk, key: &str, label: &str, value: &str, expanded: bool) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(tk.background)
        .role("combobox")
        .label(label.to_owned())
        .a11y_value(value.to_owned())
        .expanded(expanded)
        .focusable(true)
        .child(
            text(value.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .truncate()
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.fg))
}

/// `DetailRow`: `flex items-center gap-3 text-sm`; the label `text-xs font-medium muted`, the
/// value right-aligned and truncated (`font-mono text-xs` when `mono`), an optional trailing
/// element (a copy button).
pub fn detail_row(tk: &Tk, label: &str, value: &str, mono: bool, trailing: Option<El>) -> El {
    let style = if mono {
        tk.txs(400.0).mono()
    } else {
        tk.tsm(400.0)
    };
    let mut r = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .center_y()
        .min_h(tk.r(1.25))
        .child(
            text(label.to_owned())
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        )
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .justify_end()
                .child(text(value.to_owned()).font(style).color(tk.fg).truncate()),
        );
    if let Some(t) = trailing {
        r = r.child(t);
    }
    r
}

/// `CopyButton` (`outline icon-xs`, or `ghost size-5`): copies `value`, flips to a check for
/// 1.1 s. `copied` says whether this button's check is showing.
pub fn copy_button(
    tk: &Tk,
    key: &str,
    value: &str,
    aria: Option<String>,
    ghost: bool,
    copied: bool,
) -> El {
    let name = if copied {
        t("chrome:copyButton.copied")
    } else {
        aria.unwrap_or_else(|| t("chrome:copyButton.copy"))
    };
    let glyph: &'static Icon = if copied { &I::CHECK } else { &I::COPY };
    let ink = if copied {
        emerald_text(tk)
    } else if ghost {
        tk.muted_fg
    } else {
        tk.fg
    };
    let side = if ghost { tk.r(1.25) } else { tk.r(1.75) };
    let variant = if ghost {
        Variant::Ghost
    } else {
        Variant::Outline
    };
    let value = value.to_owned();
    let k = key.to_owned();
    crate::txdetail::widgets::button_surface(
        tk,
        row().key(key.to_owned()).size(side, side).center(),
        variant,
    )
    .role("button")
    .label(name.clone())
    .help(name)
    .focusable(true)
    .on_press_tx(move |a: &mut App, tx: &mut rune::Tx| {
        crate::osleaks::clipboard::copy(a, tx, value.clone());
        a.wallets.detail.copied = Some((k.clone(), a.clock + 1.1));
        tx.wake_after(std::time::Duration::from_millis(1150));
    })
    .child(ic(glyph, tk.r(0.75), ink))
}

/// `DialogContent` (ui/dialog.tsx): the faint glass backdrop, then the `kb-glass-dialog` surface
/// (`background` at 80 % over a blur, a hairline, a deep soft shadow), `rounded-[1rem]`,
/// `p-(--kb-card-padding)` (1.25 rem), `gap-4`, at `top: max(1rem, 8vh)`, with the close button.
#[allow(clippy::too_many_arguments)]
pub fn dialog(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    width: f32,
    label: &str,
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
    let close_btn = icon_button(
        tk,
        format!("{key}-close"),
        Variant::Ghost,
        tk.r(1.75),
        &I::X,
        tk.r(1.0),
        t("common:actions.close"),
    )
    .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None)
    .on_press(close);
    // `max-h-(--kb-dialog-max-h)`: the window less the top offset and 1 rem; scrolls inside.
    let max_h = win_h - top - tk.r(1.0);
    let body = col()
        .key(format!("{key}-scroll"))
        .w_full()
        .max_h(max_h - 2.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .padding(tk.r(1.25))
                .gap(tk.r(1.0))
                .child(content),
        );
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .label(label.to_owned())
        .w(width)
        .radius(tk.r(1.0))
        .fill(tk.background.alpha(0.8))
        .backdrop_blur(6.0)
        .transition(crate::shell::dialog_enter())
        .border(1.0, border)
        .border_layout()
        .clip_rounded(tk.r(1.0))
        .modal(true, close)
        .child(body)
        .child(close_btn);
    for sh in shadows {
        surface = surface.box_shadow(sh);
    }
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// `DialogHeader` (`flex flex-col gap-2`): title (`text-lg leading-none font-semibold`) and
/// description (`text-sm text-muted-foreground`).
pub fn dialog_header(tk: &Tk, title: &str, desc: &str) -> El {
    col()
        .w_full()
        .gap(tk.r(0.5))
        .pr(tk.r(1.5))
        .child(
            text(title.to_owned())
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            text(desc.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        )
}

/// `DialogFooter` (`flex-col-reverse gap-2 sm:flex-row sm:justify-end`).
pub fn dialog_footer(tk: &Tk, items: Vec<El>) -> El {
    row().w_full().justify_end().gap(tk.r(0.5)).children(items)
}

/// shadcn `Button variant="destructive"` (`bg-destructive text-white`, dark `bg-destructive/60`).
pub fn destructive_button(tk: &Tk, key: &str, label: &str, enabled: bool) -> El {
    crate::ui::button::button(
        tk,
        key,
        crate::ui::button::Variant::Destructive,
        crate::ui::button::Size::Default,
        None,
        label,
        enabled,
    )
}

/// The error line under a form (`m-0 text-sm text-destructive`).
pub fn error_line(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tsm(400.0))
        .color(tk.destructive)
        .wrap()
        .w_full()
        .role("alert")
}

/// A pending-safe button: default/outline from txdetail's widgets.
pub fn btn(
    tk: &Tk,
    key: &str,
    variant: Variant,
    size: Size,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    button(tk, key.to_owned(), variant, size, glyph, label, enabled)
}

/// `Skeleton` (`bg-accent animate-pulse rounded-md`).
pub fn skeleton(tk: &Tk, w: Option<f32>, h: f32) -> El {
    let e = el().h(h).radius(tk.r(0.375)).fill(tk.accent);
    match w {
        Some(w) => e.w(w),
        None => e.w_full(),
    }
}
