//! Shared building blocks: the card surface, icon buttons, pills, chips and the privacy blur.

use rune::{Color, El, Icon, Shadow, TextStyle, col, el, icon, row, text};

use crate::theme::{Tk, Tone};

pub mod button;
pub mod kit_menu;
pub mod popup;
pub mod table;

/// `.kb-surface`: 0.75 rem radius, 1 px border at 70 %, card fill, soft two-layer shadow.
pub fn surface(tk: &Tk) -> El {
    let (a, b) = if tk.dark {
        (0.35, 0.20)
    } else {
        (0.035, 0.035)
    };
    col()
        .radius(tk.r(0.75))
        .fill(tk.card)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(a),
            ..Shadow::default()
        })
        .box_shadow(Shadow {
            y: 10.0,
            blur: 30.0,
            color: Color::BLACK.alpha(b),
            ..Shadow::default()
        })
}

/// A glyph tinted `color`, `size` points square.
pub fn ic(i: &'static Icon, size: f32, color: Color) -> El {
    icon(i, size).color(color)
}

/// Text that the privacy modes blur (`.sensitive`: blur 7 px, no selection) and announce as
/// masked; `hidden` false draws it plainly.
pub fn sensitive(s: impl Into<String>, style: TextStyle, color: Color, hidden: bool) -> El {
    let s = s.into();
    if hidden {
        let name = masked_name(&s);
        text(s)
            .font(style)
            .color(color.alpha(color.a * 0.85))
            .blur(3.5)
            .label(name)
    } else {
        text(s).font(style).color(color)
    }
}

/// The accessible name of a masked value (Sensitive.tsx): "Hidden share" for a percentage,
/// "Hidden amount" otherwise, never the value or the mask's bullets.
pub fn masked_name(shown: &str) -> String {
    crate::i18n::t(if shown.trim_end().ends_with('%') {
        "common:privacy.hiddenShare"
    } else {
        "common:privacy.hiddenAmount"
    })
}

/// `masked_name` for a value known to be an amount.
pub fn hidden_amount() -> String {
    crate::i18n::t("common:privacy.hiddenAmount")
}

/// The neutral pill (`border-border bg-muted/45 text-foreground`).
pub fn neutral_tone(tk: &Tk) -> Tone {
    Tone {
        bg: tk.muted.alpha(0.45),
        border: tk.border,
        text: tk.fg,
        solid: tk.muted_fg,
    }
}

/// A ghost icon button (`size-8`, muted glyph).
pub fn icon_button(tk: &Tk, key: &str, glyph: &'static Icon, label: &str) -> El {
    button::icon_button(
        tk,
        key,
        button::Variant::Ghost,
        tk.r(2.0),
        glyph,
        tk.r(1.0),
        label.to_owned(),
    )
}

/// A small toggle chip (`px-1.5 py-0.5 rounded-md text-xs`), active = `bg-muted text-foreground`.
pub fn chip(tk: &Tk, key: &str, label: &str, active: bool) -> El {
    row()
        .key(key.to_owned())
        .px(tk.r(0.375))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .fill(if active { tk.muted } else { Color::TRANSPARENT })
        .hover(tk.muted.alpha(0.45))
        .role("button")
        .checked(active)
        .focusable(true)
        .child(text(label.to_owned()).font(tk.txs(500.0)).color(if active {
            tk.fg
        } else {
            tk.muted_fg
        }))
}

/// A primary button (`h-8`).
pub fn primary_button(tk: &Tk, key: &str, glyph: Option<&'static Icon>, label: &str) -> El {
    button::button(
        tk,
        key,
        button::Variant::Default,
        button::Size::Sm,
        glyph,
        label,
        true,
    )
}

/// An outline button (`h-8`).
pub fn outline_button(tk: &Tk, key: &str, glyph: Option<&'static Icon>, label: &str) -> El {
    button::button(
        tk,
        key,
        button::Variant::Outline,
        button::Size::Sm,
        glyph,
        label,
        true,
    )
}

/// A 1 px rule.
pub fn hairline(color: Color) -> El {
    el().h(1.0).w_full().fill(color)
}

/// A filled dot.
pub fn dot(size: f32, color: Color) -> El {
    el().size(size, size).radius(size / 2.0).fill(color)
}

/// The router's not-found page (routes/NotFound.tsx), inside the shell so the nav, the palette
/// and the breadcrumb stay: a compass, the title, the path that led here, and a way back.
pub fn not_found_page(app: &crate::app::App, tk: &Tk) -> El {
    let path = app.route.path().to_owned();
    // `screenShellClassName`, then `flex flex-col items-start gap-2 py-10`.
    col()
        .w_full()
        .padding(tk.page_pad())
        .fill(tk.background)
        .child(
            col()
                .key("not-found")
                .w_full()
                .py(tk.r(2.5))
                .gap(tk.r(0.5))
                .child(ic(&crate::icons::COMPASS, tk.r(1.25), tk.muted_fg).a11y_hidden())
                .child(
                    text(crate::i18n::t("chrome:notFound.title"))
                        .font(tk.tbase(600.0))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(
                    text(crate::i18n::tr(
                        "chrome:notFound.message",
                        &[("path", &path)],
                    ))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .max_w(tk.r(40.0))
                    .wrap(),
                )
                .child(
                    // `Button asChild size="sm" className="mt-2"` around a Link.
                    row().pt(tk.r(0.5)).child(
                        primary_button(
                            tk,
                            "not-found-back",
                            None,
                            &crate::i18n::t("chrome:notFound.backToOverview"),
                        )
                        .role("link")
                        .on_press(|a: &mut crate::app::App| a.navigate("/overview")),
                    ),
                ),
        )
}

#[cfg(feature = "tooling")]
/// Dev page (`/dev/typo`): the same words at the web's sizes and weights, for text-metric
/// comparison against Chromium (parity work). `a-` rows use the app's styles, `b-` rows pin the
/// variable `wght` axis explicitly.
pub fn typo_page(tk: &Tk) -> El {
    const WORDS: [&str; 10] = [
        "Transactions",
        "Activity",
        "Support",
        "Overview",
        "Full Accounting",
        "Ready for reports",
        "Kassiber v0.22.78",
        "Up to date",
        "Regtest Spending Wallet",
        "BTC price",
    ];
    const SIZES: [f32; 5] = [10.0, 10.8, 12.6, 14.4, 18.0];
    const WEIGHTS: [f32; 4] = [400.0, 500.0, 600.0, 700.0];
    let mut page = col().padding(8.0).gap(0.0);
    for (si, s) in SIZES.iter().enumerate() {
        for (wi, w) in WEIGHTS.iter().enumerate() {
            let mut r = row().gap(6.0);
            for (i, word) in WORDS.iter().enumerate() {
                r = r.child(
                    text(*word)
                        .font(TextStyle::new(*s, *w))
                        .color(tk.fg)
                        .key(format!("typo-a-{si}-{wi}-{i}")),
                );
            }
            page = page.child(r);
        }
    }
    let _ = rune::row;
    page
}

/// `Button variant="outline"` fill/border per theme (ui/button.tsx): `border bg-card shadow-xs`
/// in light; `border-input bg-input/30` with a 6 % top highlight in dark.
pub fn outline_surface(tk: &Tk, el: El) -> El {
    if tk.dark {
        el.fill(tk.input.alpha(tk.input.a * 0.3))
            .border(1.0, tk.input)
            .border_layout()
            .box_shadow(Shadow {
                y: 1.0,
                blur: 0.0,
                color: Color::WHITE.alpha(0.06),
                inset: true,
                ..Shadow::default()
            })
            .hover(tk.input.alpha(tk.input.a * 0.5))
    } else {
        el.fill(tk.card)
            .border(1.0, tk.border)
            .border_layout()
            .box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Shadow::default()
            })
            .hover(tk.accent)
    }
}

/// An extra-small outline button (`h-7`).
pub fn button_xs_outline(tk: &Tk, key: &str, label: &str) -> El {
    button::button(
        tk,
        key,
        button::Variant::Outline,
        button::Size::Xs,
        None,
        label,
        true,
    )
}

/// shadcn `Switch` (ui/switch.tsx): `h-[1.15rem] w-8 rounded-full border border-transparent`,
/// checked `bg-primary`, unchecked `bg-foreground/20` (dark `bg-input/80`); the `size-4` thumb is
/// `bg-card` (dark: `bg-foreground` off, `bg-primary-foreground` on), moved `100% - 2px` when on.
/// It has no visible label: `label` names it for assistive technology only.
pub fn switch(tk: &Tk, key: &str, checked: bool, label: &str) -> El {
    let track = if checked {
        tk.primary
    } else if tk.dark {
        tk.input.alpha(tk.input.a * 0.8)
    } else {
        tk.fg.alpha(0.2)
    };
    let thumb_fill = match (tk.dark, checked) {
        (false, _) => tk.card,
        (true, false) => tk.fg,
        (true, true) => tk.primary_fg,
    };
    let thumb = tk.r(1.0);
    let mut knob = el()
        .size(thumb, thumb)
        .radius(thumb / 2.0)
        .fill(thumb_fill)
        .offset(if checked { thumb - 2.0 } else { 0.0 }, 0.0);
    if !tk.dark {
        knob = knob.box_shadow(Shadow {
            y: 1.0,
            blur: 3.0,
            color: Color::BLACK.alpha(0.1),
            ..Shadow::default()
        });
    }
    row()
        .key(key.to_owned())
        .h(tk.r(1.15))
        .w(tk.r(2.0))
        .center_y()
        .radius(tk.r(0.575))
        .fill(track)
        .border(1.0, Color::TRANSPARENT)
        .border_layout()
        .role("switch")
        .checked(checked)
        .label(label.to_owned())
        .focusable(true)
        .child(knob)
}
