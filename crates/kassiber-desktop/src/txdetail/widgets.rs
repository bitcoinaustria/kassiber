//! Small building blocks shared by the detail panel's sections (Fact, DetailField, InfoHint,
//! dirty dots, select menus, badges and buttons from TransactionDetailSheetParts.tsx).

use rune::{Color, El, Icon, TextStyle, col, el, row, text};
use rune_kit::Placement;

use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::{Tk, Tone};
use crate::ui::ic;

/// An Info icon whose tooltip explains `label` (`InfoHint`): a 1 rem button (not a tab stop),
/// `Info` at 0.75 rem, 60 % opacity, named "What does "<label>" mean?".
pub fn hint(tk: &Tk, label: &str, body: String) -> El {
    hint_named(
        tk,
        tr("transactions:infoHint.fieldMeaning", &[("label", label)]),
        body,
        tk.muted_fg,
    )
}

/// `InfoHint` with an explicit accessible name and ink colour (`text-current`).
pub fn hint_named(tk: &Tk, name: String, body: String, color: Color) -> El {
    row()
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(tk.r(0.375))
        .role("button")
        .label(name)
        .help(body)
        .child(ic(&I::INFO, tk.r(0.75), color.alpha(color.a * 0.6)))
}

/// shadcn `Button` variants (ui/button.tsx).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Default,
    Outline,
    Ghost,
}

/// shadcn `Button` sizes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// `h-9 px-4` (`px-3` with an icon), text-sm.
    Default,
    /// `h-8 px-3 gap-1.5` (`px-2.5` with an icon), text-sm.
    Sm,
    /// `h-7 px-2 gap-1 text-xs` (`px-1.5` with an icon, icon 0.75 rem).
    Xs,
}

/// A shadcn `Button` with an optional leading icon and a label.
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

/// `button` with an explicit icon size (`size-3.5` on the icon).
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
            tk.r(1.0),
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
    let (fg, icon_fg) = match variant {
        Variant::Default => (tk.primary_fg, tk.primary_fg),
        Variant::Outline => (tk.fg, tk.fg),
        Variant::Ghost => (tk.fg, tk.muted_fg),
    };
    let mut b = button_surface(
        tk,
        row().key(key.into()).h(h).px(px).gap(gap).center(),
        variant,
    )
    .role("button")
    .label(label.to_owned())
    .focusable(enabled)
    .disabled(!enabled);
    if !enabled {
        b = b.opacity(0.5);
    }
    if let Some(g) = glyph {
        b = b.child(ic(g, glyph_size.unwrap_or(icon), icon_fg));
    }
    b.child(text(label.to_owned()).font(font).color(fg))
}

/// A square shadcn icon button (`icon` 2.25 rem, `icon-sm` 2 rem, `icon-xs` 1.75 rem with a
/// 0.75 rem glyph unless `glyph_size` says otherwise).
pub fn icon_button(
    tk: &Tk,
    key: impl Into<String>,
    variant: Variant,
    side: f32,
    glyph: &'static Icon,
    glyph_size: f32,
    label: String,
) -> El {
    let ink = if variant == Variant::Ghost {
        tk.muted_fg
    } else {
        tk.fg
    };
    button_surface(tk, row().key(key.into()).size(side, side).center(), variant)
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .child(ic(glyph, glyph_size, ink))
}

/// Fill, border and hover of a shadcn button variant.
pub fn button_surface(tk: &Tk, el: El, variant: Variant) -> El {
    let el = el.radius(tk.r(0.375));
    match variant {
        Variant::Default => el.fill(tk.primary).hover(tk.primary_fg.alpha(0.1)),
        Variant::Outline => {
            if tk.dark {
                // `dark:border-input dark:bg-input/30` + a 1 px top highlight.
                el.fill(tk.input.alpha(tk.input.a * 0.3))
                    .border(1.0, tk.input)
                    .box_shadow(rune::Shadow {
                        y: 1.0,
                        color: Color::WHITE.alpha(0.06),
                        inset: true,
                        ..Default::default()
                    })
                    .hover(tk.input.alpha(tk.input.a * 0.5))
            } else {
                el.fill(tk.card)
                    .border(1.0, tk.border)
                    .box_shadow(rune::Shadow {
                        y: 1.0,
                        blur: 2.0,
                        color: Color::BLACK.alpha(0.05),
                        ..Default::default()
                    })
                    .hover(tk.accent)
            }
        }
        Variant::Ghost => el.hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        }),
    }
}

/// `kb-surface-inset`: card fill, 1 px border at 70 %, 0.5 rem radius.
pub fn surface_inset(tk: &Tk) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .radius(tk.r(0.5))
        .fill(tk.card)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
}

/// The 6 px amber "Unsaved change" dot.
pub fn dirty_dot(tk: &Tk) -> El {
    el().size(6.0, 6.0)
        .radius(3.0)
        .fill(tk.warn.solid)
        .role("img")
        .label(t("transactions:dirty.unsavedChange"))
        .help(t("transactions:dirty.unsavedChange"))
}

/// An uppercase field label with optional hint and dirty dot (`text-2xs font-medium uppercase
/// tracking-wide`, `gap-1`).
pub fn label_row(tk: &Tk, label: &str, hint_text: Option<String>, dirty: bool) -> El {
    let mut r = row().h(tk.r(1.0)).gap(tk.r(0.25)).center_y().child(
        text(label.to_uppercase())
            .font(tk.t2xs(500.0).tracking(0.25))
            .color(tk.muted_fg),
    );
    if let Some(h) = hint_text {
        r = r.child(hint(tk, label, h));
    }
    if dirty {
        r = r.child(dirty_dot(tk));
    }
    r
}

/// A `<dt>/<dd>` pair of the compact grids (`Fact`): the label row (hint, dirty dot, copy
/// button) and `mt-0.5` the value.
pub fn fact(tk: &Tk, label: &str, hint_text: Option<String>, dirty: bool, value: El) -> El {
    fact_with(tk, label, hint_text, dirty, None, value)
}

/// `Fact` with a copy button in its label row (`copyValue`).
pub fn fact_with(
    tk: &Tk,
    label: &str,
    hint_text: Option<String>,
    dirty: bool,
    copy: Option<El>,
    value: El,
) -> El {
    let mut lr = label_row(tk, label, hint_text, dirty);
    if let Some(c) = copy {
        lr = lr.child(c);
    }
    col().min_w(0.0).gap(tk.r(0.125)).child(lr).child(value)
}

/// A plain value line (`text-sm font-medium`).
pub fn value(tk: &Tk, s: impl Into<String>) -> El {
    let s = s.into();
    text(s.clone())
        .font(tk.tsm(500.0))
        .color(tk.fg)
        .truncate()
        .help_when_truncated(s)
}

/// The rune-kit tokens for shadcn form fields (`Input`, `Textarea`): `border-input`, `dark:bg-input/30`
/// (`dark:hover:bg-input/50`), the ring colour on focus, muted placeholders, `rounded-md`.
pub fn form_kit(tk: &Tk) -> rune_kit::Kit {
    let mut k = tk.kit();
    k.c.bg_base = if tk.dark {
        tk.input.alpha(tk.input.a * 0.3)
    } else {
        Color::TRANSPARENT
    };
    k.c.hover = if tk.dark {
        tk.input.alpha(tk.input.a * 0.2)
    } else {
        Color::TRANSPARENT
    };
    k.c.text_faint = tk.muted_fg;
    k.c.text_base = tk.fg;
    k.c.border_focus = tk.ring;
    // The border is drawn by `field` (a 1 px `border-input`); the kit's own ring is cleared.
    k.elevation = k.elevation.clone().with_input(vec![rune::Shadow {
        color: Color::TRANSPARENT,
        ..Default::default()
    }]);
    k
}

/// A built rune-kit field in shadcn's frame: `border border-input`.
pub fn field(tk: &Tk, e: El) -> El {
    e.border(1.0, tk.input)
}

/// A `Label` (`flex items-center gap-2 text-sm leading-none font-medium`; call sites use
/// `gap-1.5`), then the hint and dirty dot.
pub fn field_label(tk: &Tk, label: &str, hint_text: Option<String>, dirty: bool) -> El {
    field_label_ink(tk, label, hint_text, dirty, tk.fg)
}

/// `field_label` in another ink (`text-muted-foreground` for Review status).
pub fn field_label_ink(
    tk: &Tk,
    label: &str,
    hint_text: Option<String>,
    dirty: bool,
    ink: Color,
) -> El {
    let mut r = row().gap(tk.r(0.375)).center_y().child(
        text(label.to_owned())
            .font(tk.tsm(500.0).lh(tk.r(0.875)))
            .color(ink),
    );
    if let Some(h) = hint_text {
        r = r.child(hint(tk, label, h));
    }
    if dirty {
        r = r.child(dirty_dot(tk));
    }
    r
}

/// An inset surface (`rounded-md border bg-muted/20 p-3`).
pub fn inset(tk: &Tk) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.2))
}

/// A tinted callout box (amber/emerald) with an icon and title.
pub fn callout(
    tk: &Tk,
    tone: &Tone,
    glyph: &'static Icon,
    title: &str,
    body: Option<String>,
) -> El {
    let mut c = col()
        .min_w(0.0)
        .gap(tk.r(0.125))
        .grow(1.0)
        .child(text(title.to_owned()).font(tk.txs(600.0)).color(tone.text));
    if let Some(b) = body {
        c = c.child(text(b).font(tk.txs(400.0)).color(tk.fg.alpha(0.85)).wrap());
    }
    row()
        .w_full()
        .padding(tk.r(0.625))
        .gap(tk.r(0.5))
        .items_start()
        .radius(tk.r(0.375))
        .border(1.0, tone.border)
        .fill(tone.bg)
        .child(ic(glyph, tk.r(1.0), tone.text))
        .child(c)
}

/// An outline badge (`rounded-md border px-1.5 text-2xs`).
pub fn badge(tk: &Tk, label: &str, fg: Color, bg: Color, border: Color) -> El {
    row()
        .h(tk.r(1.25))
        .px(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .fill(bg)
        .border(1.0, border)
        .child(text(label.to_owned()).font(tk.t2xs(500.0)).color(fg))
}

pub fn muted_badge(tk: &Tk, label: &str) -> El {
    badge(tk, label, tk.muted_fg, tk.muted, tk.border)
}

/// A small button (`h-7 px-2 text-xs`), outline or ghost.
pub fn small_button(
    tk: &Tk,
    key: impl Into<String>,
    glyph: Option<&'static Icon>,
    label: &str,
    outline: bool,
    enabled: bool,
) -> El {
    let mut r = row()
        .key(key.into())
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .role("button")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 });
    if outline {
        r = r.border(1.0, tk.input).fill(tk.background);
    }
    if enabled {
        r = r.hover(tk.accent);
    }
    if let Some(g) = glyph {
        r = r.child(ic(g, tk.r(0.875), tk.fg));
    }
    r.child(text(label.to_owned()).font(tk.txs(500.0)).color(tk.fg))
}

/// A square icon button with a name.
pub fn icon_btn(
    tk: &Tk,
    key: impl Into<String>,
    glyph: &'static Icon,
    label: String,
    size: f32,
) -> El {
    row()
        .key(key.into())
        .size(size, size)
        .center()
        .radius(tk.r(0.375))
        .hover(tk.accent)
        .role("button")
        .label(label.clone())
        .help(label)
        .focusable(true)
        .child(ic(glyph, size * 0.5, tk.muted_fg))
}

/// A `SelectTrigger` (`w-fit h-9 rounded-md border border-input px-3 py-2 text-sm`,
/// `dark:bg-input/30 dark:hover:bg-input/50`, a muted 1 rem chevron); `.w_full()` where the web
/// says `w-full`.
pub fn select_trigger(tk: &Tk, key: &str, label: &str, value: &str, enabled: bool) -> El {
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, tk.accent)
    };
    row()
        .key(key.to_owned())
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .fill(fill)
        .hover(hover)
        .role("combobox")
        .label(label.to_owned())
        .a11y_value(value.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 })
        .child(
            text(value.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .truncate()
                .help_when_truncated(value.to_owned())
                .shrink(),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)))
}

/// Mounts a select's option list under its trigger. `choose` gets the picked value.
pub fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    options: Vec<(String, String)>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let mut p = crate::ui::popup::Popup::listbox(anchor, close_select);
    for (v, label) in options {
        let checked = v == current;
        let pick = choose.clone();
        let val = v.clone();
        p.item(
            row()
                .w_full()
                .min_h(tk.r(2.0))
                .pr(tk.r(0.5))
                .gap(tk.r(0.375))
                .center_y()
                .radius(tk.r(0.25))
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .role("option")
                .checked(checked)
                .label(label.clone())
                .on_press(move |a: &mut App| {
                    close_select(a);
                    pick(a, &val);
                })
                .child(row().w(tk.r(1.5)).center().child(if checked {
                    ic(&I::CHECK, tk.r(0.875), tk.fg)
                } else {
                    el().size(1.0, 1.0)
                }))
                .child(
                    text(label.clone())
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .grow(1.0),
                ),
            format!("{anchor}-opt-{v}"),
            label,
            true,
            checked,
        );
    }
    let list = col()
        .min_w(tk.r(14.0))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
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

pub fn close_select(a: &mut App) {
    if let Some(o) = a.detail.open.as_mut() {
        o.select = None;
    }
}

pub fn toggle_select(a: &mut App, s: super::Select) {
    if let Some(o) = a.detail.open.as_mut() {
        o.select = if o.select == Some(s) { None } else { Some(s) };
    }
}

/// shadcn `Switch`: a `w-8 h-[1.15rem]` pill (`bg-primary` on; off `bg-foreground/20`, dark
/// `bg-input/80`) with a `size-4` thumb (`bg-card`; dark off `bg-foreground`, on
/// `bg-primary-foreground`) that slides `100% - 2px`.
pub fn ui_switch(
    tk: &Tk,
    key: &str,
    on: bool,
    label: &str,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    let (w, h, knob) = (tk.r(2.0), tk.r(1.15), tk.r(1.0));
    let track = if on {
        tk.primary
    } else if tk.dark {
        tk.input.alpha((tk.input.a * 0.8).min(1.0))
    } else {
        tk.fg.alpha(0.2)
    };
    let thumb = if !tk.dark {
        tk.card
    } else if on {
        tk.primary_fg
    } else {
        tk.fg
    };
    let x = if on { knob - 2.0 } else { 0.0 };
    row()
        .key(key.to_owned())
        .size(w, h)
        .radius(h / 2.0)
        .fill(track)
        .border(1.0, Color::TRANSPARENT)
        .center_y()
        .role("switch")
        .checked(on)
        .label(label.to_owned())
        .focusable(true)
        .on_press(toggle)
        .child(
            el().size(knob, knob)
                .radius(knob / 2.0)
                .fill(thumb)
                .ml(x)
                .motion(rune::tok::motion::SNAPPY),
        )
}

/// A keyboard hint chip.
pub fn kbd(tk: &Tk, k: &str) -> El {
    // `rounded-md border bg-muted px-1` in the surrounding `text-xs` (kbd is monospace).
    row()
        .px(tk.r(0.25))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.muted)
        .child(
            text(k.to_owned())
                .font(tk.txs(400.0).mono())
                .color(tk.muted_fg),
        )
}

/// Text that the privacy modes blur.
pub fn sens(tk: &Tk, s: impl Into<String>, style: TextStyle, color: Color, hidden: bool) -> El {
    let _ = tk;
    crate::ui::sensitive(s, style, color, hidden)
}

/// A spinning loader glyph.
pub fn spinner(tk: &Tk, size: f32) -> El {
    ic(&I::LOADER_CIRCLE, size, tk.muted_fg).looping(rune::LoopSpec::rotation(0.9))
}

/// `LedgerRow`: `grid min-h-10 grid-cols-[minmax(140px,0.9fr)_minmax(0,1.1fr)] items-center gap-3
/// border-b px-3 py-2` (none on the last row), muted rows `bg-muted/50`; the label `text-xs
/// text-muted-foreground` with its hint, the value `text-sm font-medium text-right tabular-nums`.
pub fn ledger_row(
    tk: &Tk,
    label: &str,
    hint_text: Option<String>,
    value: El,
    muted: bool,
    last: bool,
) -> El {
    let mut l = row()
        .gap(tk.r(0.375))
        .center_y()
        .grow(0.9)
        .min_w(140.0)
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .help_when_truncated(label.to_owned()),
        );
    if let Some(h) = hint_text {
        l = l.child(hint(tk, label, h));
    }
    row()
        .w_full()
        .min_h(tk.r(2.5))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(tk.r(0.75))
        .center_y()
        .fill(if muted {
            tk.muted.alpha(0.5)
        } else {
            Color::TRANSPARENT
        })
        .border_sides([0.0, 0.0, if last { 0.0 } else { 1.0 }, 0.0], tk.border)
        .child(l)
        .child(row().grow(1.1).min_w(0.0).justify_end().child(value))
}

/// A ledger value (`text-sm font-medium tabular-nums`).
pub fn ledger_value(tk: &Tk, s: impl Into<String>, hidden: bool) -> El {
    let s = s.into();
    let e = sens(
        tk,
        s.clone(),
        tk.tsm(500.0).features("'tnum' 1"),
        tk.fg,
        hidden,
    )
    .truncate();
    if hidden { e } else { e.help_when_truncated(s) }
}

/// A ledger table's header strip (`border-b bg-muted px-3 py-1.5`; `gap-1.5 text-2xs
/// font-semibold uppercase tracking-wide text-muted-foreground`, a 0.75 rem icon).
pub fn ledger_header(
    tk: &Tk,
    glyph: &'static Icon,
    title: &str,
    extra: Option<El>,
    action: Option<El>,
) -> El {
    let mut left = row()
        .gap(tk.r(0.375))
        .center_y()
        .child(ic(glyph, tk.r(0.75), tk.muted_fg))
        .child(
            text(title.to_uppercase())
                .font(tk.t2xs(600.0).tracking(0.25))
                .color(tk.muted_fg),
        );
    if let Some(e) = extra {
        left = left.child(e);
    }
    let mut r = row()
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .fill(tk.muted)
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(left);
    if let Some(a) = action {
        r = r.child(a);
    }
    r
}

/// `overflow-hidden rounded-md border`.
pub fn ledger_table(tk: &Tk) -> El {
    col()
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .clip_rounded(tk.r(0.375))
}

/// An outline badge in `text-2xs` (`Badge variant="outline" rounded-md text-2xs`). In a ledger
/// header it inherits the header's `uppercase`; callers pass the label uppercased there.
pub fn outline_badge_2xs(tk: &Tk, label: &str) -> El {
    row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .child(text(label.to_owned()).font(tk.t2xs(500.0)).color(tk.fg))
}
