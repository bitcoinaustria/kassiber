//! The shadcn pieces the three screens share: page shell and header, Badge, Input, Select,
//! DropdownMenu, the confirm dialog (the web's `window.confirm`) and the section surfaces.

use std::rc::Rc;

use rune::{Color, El, Icon, Shadow, TextEdit, Tx, col, el, row, text};
use rune_kit::{Placement, TextInput};

use super::Popup;
use crate::app::App;
use crate::icons as I;
use crate::theme::{Tk, Tone};
use crate::ui::ic;

/// `screenShellClassName` on the app's spacing scale (`Tk::page_pad`, `Tk::section_gap`).
pub fn shell(tk: &Tk) -> El {
    col()
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background)
}

/// `pageDescriptionClassName`: `max-w-3xl text-sm text-muted-foreground`.
pub fn description(tk: &Tk, s: String) -> El {
    text(s)
        .font(tk.tsm(400.0))
        .color(tk.muted_fg)
        .max_w(tk.r(48.0))
        .wrap()
}

/// `pageHeaderClassName` at `sm`+: a row, items centred, space between, `gap-3`.
pub fn header(tk: &Tk) -> El {
    row().w_full().gap(tk.r(0.75)).center_y().justify_between()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BadgeVariant {
    Outline,
}

/// A neutral label (`crate::ui::tag`).
pub fn badge(tk: &Tk, _variant: BadgeVariant, label: &str) -> El {
    crate::ui::tag(tk, tk.muted_fg, None, label)
}

/// A label in a status tone, optional leading glyph (`crate::ui::tag`).
pub fn tone_badge(
    tk: &Tk,
    tone: &Tone,
    glyph: Option<(&'static Icon, Color)>,
    label: &str,
    _rounded_md: bool,
) -> El {
    let mut b = row().gap(tk.r(0.25)).center_y();
    if let Some((g, c)) = glyph {
        b = b.child(ic(g, tk.r(0.75), c));
    }
    b.child(text(label.to_owned()).font(tk.txs(500.0)).color(tone.text))
}

/// The destructive tone (`border-destructive/30 bg-destructive/10 text-destructive`).
pub fn destructive_tone(tk: &Tk) -> Tone {
    Tone {
        bg: tk.destructive.alpha(0.10),
        border: tk.destructive.alpha(0.30),
        text: tk.destructive,
        solid: tk.destructive,
    }
}

/// A `kb-surface` card.
pub fn surface(tk: &Tk) -> El {
    crate::ui::surface(tk)
}

/// shadcn `Input` (`h-9 rounded-md border border-input px-3 text-sm shadow-xs`, `dark:bg-input/30`)
/// over a rune-kit field. `mono` is `font-mono text-xs`; `height` overrides `h-9`.
#[allow(clippy::too_many_arguments)]
pub fn input(
    tk: &Tk,
    edit: &TextEdit,
    key: &str,
    name: String,
    placeholder: String,
    mono: bool,
    height: f32,
    on_edit: fn(&mut App) -> &mut TextEdit,
) -> El {
    let k = crate::txdetail::widgets::form_kit(tk);
    let style = if mono {
        tk.txs(400.0).mono()
    } else {
        tk.tsm(400.0)
    };
    let field = TextInput::new(edit)
        .label(name)
        .placeholder(placeholder)
        .height(height)
        .text_style(style)
        .on_edit(move |a: &mut App| on_edit(a))
        .build(&k)
        .key(key.to_owned());
    let field = field.radius(tk.r(0.375)).border(1.0, tk.input);
    if tk.dark {
        field
    } else {
        field.box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
    }
}

/// A `SelectTrigger` (`w-fit h-9`), opening `popup`.
pub fn select(
    tk: &Tk,
    key: &'static str,
    name: String,
    value: String,
    popup: Popup,
    open: bool,
) -> El {
    crate::ui::popup::trigger(
        crate::txdetail::widgets::select_trigger(tk, key, &name, &value, true),
        crate::ui::popup::Kind::Listbox,
        open,
        move |a: &mut App| toggle(a, popup),
    )
}

pub fn toggle(a: &mut App, p: Popup) {
    a.reports.diag.popup = if a.reports.diag.popup == Some(p) {
        None
    } else {
        Some(p)
    };
}

pub fn close(a: &mut App) {
    a.reports.diag.popup = None;
}

/// A `SelectContent` under `anchor`: options with a check on the current one.
pub fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &'static str,
    options: Vec<(&'static str, String)>,
    current: &str,
    choose: impl Fn(&mut App, &'static str) + Clone + 'static,
) {
    let selected = options.iter().position(|(v, _)| *v == current).unwrap_or(0);
    let mut p = crate::ui::popup::Popup::listbox(anchor, close);
    for (v, label) in options {
        let checked = v == current;
        let pick = choose.clone();
        // `SelectItem`: `py-1.5 pr-8 pl-2 text-sm`, the check at `right-2`.
        let r = row()
            .w_full()
            .h(tk.r(0.375) * 2.0 + tk.r(1.25))
            .pl(tk.r(0.5))
            .pr(tk.r(2.0))
            .center_y()
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .role("option")
            .checked(checked)
            .label(label.clone())
            .on_press(move |a: &mut App| {
                close(a);
                pick(a, v);
            })
            .child(text(label.clone()).font(tk.tsm(400.0)).color(tk.fg))
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
        p.item(r, format!("{anchor}-opt-{v}"), label, true, checked);
    }
    // Radix's item-aligned position: the current option sits over the trigger.
    let row = tk.r(0.375) * 2.0 + tk.r(1.25);
    let gap = -(tk.r(2.25) / 2.0 + row / 2.0 + tk.r(0.25) + 1.0 + selected as f32 * row);
    let list = col()
        .min_w(tk.r(8.0))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        gap,
    );
}

pub type Action = Rc<dyn Fn(&mut App, &mut Tx)>;

pub enum Entry {
    Label(String, bool),
    Sep,
    Item {
        key: String,
        glyph: Option<&'static Icon>,
        label: String,
        on: Action,
    },
    Check {
        key: String,
        glyph: Option<&'static Icon>,
        label: String,
        mono: bool,
        upper: bool,
        trailing: Option<String>,
        checked: bool,
        on: Action,
    },
}

fn row_h(tk: &Tk) -> f32 {
    tk.r(0.375) * 2.0 + tk.r(1.25)
}

/// A `DropdownMenuContent` (`kb-glass-popover rounded-md border p-1`) of `width` under `anchor`.
pub fn menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    width: Option<f32>,
    entries: Vec<Entry>,
) {
    let mut p = crate::ui::popup::Popup::menu(anchor, close).list_key(format!("{key}-list"));
    let mut list = col();
    if let Some(w) = width {
        list = list.w(w - 2.0 * (tk.r(0.25) + 1.0));
    } else {
        list = list.min_w(tk.r(8.0) - 2.0 * (tk.r(0.25) + 1.0));
    }
    let base = |label: &str| {
        row()
            .w_full()
            .h(row_h(tk))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .label(label.to_owned())
    };
    for e in entries {
        match e {
            Entry::Label(l, small) => {
                let st = if small {
                    tk.txs(500.0).mono()
                } else {
                    tk.tsm(500.0)
                };
                let h = if small {
                    tk.r(0.375) * 2.0 + tk.r(1.0)
                } else {
                    row_h(tk)
                };
                p.add(
                    row().w_full().h(h).px(tk.r(0.5)).center_y().child(
                        text(l)
                            .font(st)
                            .color(if small { tk.muted_fg } else { tk.fg }),
                    ),
                );
            }
            Entry::Sep => p.add(el().h(1.0).w_full().my(tk.r(0.25)).fill(tk.border)),
            Entry::Item {
                key,
                glyph,
                label,
                on,
            } => {
                let mut r = base(&label).px(tk.r(0.5)).role("menuitem").on_press_tx(
                    move |a: &mut App, tx: &mut Tx| {
                        close(a);
                        on(a, tx)
                    },
                );
                if let Some(g) = glyph {
                    r = r.child(ic(g, tk.r(1.0), tk.muted_fg));
                }
                p.item(
                    r.child(text(label.clone()).font(tk.tsm(400.0)).color(tk.fg)),
                    key,
                    label,
                    true,
                    false,
                );
            }
            Entry::Check {
                key,
                glyph,
                label,
                mono,
                upper,
                trailing,
                checked,
                on,
            } => {
                // `DropdownMenuCheckboxItem`: `py-1.5 pr-2 pl-8`, the check at `left-2`. Picking
                // closes the menu (Radix closes on select).
                let mut r = base(&label)
                    .pl(tk.r(2.0))
                    .pr(tk.r(0.5))
                    .role("menuitemcheckbox")
                    .checked(checked)
                    .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                        close(a);
                        on(a, tx)
                    });
                if checked {
                    r = r.child(
                        row()
                            .absolute(tk.r(0.5), (row_h(tk) - tk.r(0.875)) / 2.0)
                            .size(tk.r(0.875), tk.r(0.875))
                            .center()
                            .child(ic(&I::CHECK, tk.r(1.0), tk.fg)),
                    );
                }
                if let Some(g) = glyph {
                    r = r.child(ic(g, tk.r(1.0), tk.muted_fg));
                }
                let shown = if upper {
                    label.to_uppercase()
                } else {
                    label.clone()
                };
                let st = if mono {
                    tk.tsm(400.0).mono()
                } else {
                    tk.tsm(400.0)
                };
                r = r.child(text(shown).font(st).color(tk.fg).truncate().shrink());
                if let Some(t) = trailing {
                    r = r
                        .child(el().grow(1.0))
                        .child(text(t).font(tk.txs(400.0)).color(tk.muted_fg));
                }
                p.item(r, key, label, true, false);
            }
        }
    }
    p.mount(
        tk,
        layer,
        list,
        |l| {
            let mut s = menu_surface(tk, l);
            if let Some(w) = width {
                s = s.w(w);
            }
            s.key(key.to_owned()).max_h(tk.r(28.0))
        },
        placement,
        4.0,
    );
}

/// The web's `window.confirm`: a small alert dialog with Cancel and OK.
pub fn confirm(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    message: String,
    on_ok: impl Fn(&mut App, &mut Tx) + 'static,
    on_cancel: impl Fn(&mut App) + Clone + 'static,
) {
    let cancel = on_cancel.clone();
    let dialog = col()
        .key(key.to_owned())
        .w(420.0)
        .padding(tk.r(1.25))
        .gap(tk.r(1.0))
        .radius(tk.r(0.75))
        .fill(tk.popover)
        .border(1.0, tk.border)
        .role("alertdialog")
        .label(message.clone())
        .modal(true, on_cancel.clone())
        .on_press(|_s: &mut App| {})
        .child(
            text(message)
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap()
                .w_full(),
        )
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(tk.r(0.5))
                .child(
                    crate::ui::outline_button(
                        tk,
                        &format!("{key}-cancel"),
                        None,
                        &crate::i18n::t("common:actions.cancel"),
                    )
                    .on_press(move |a: &mut App| cancel(a)),
                )
                .child(
                    crate::ui::primary_button(tk, &format!("{key}-ok"), None, "OK")
                        .autofocus()
                        .on_press_tx(on_ok),
                ),
        );
    layer.push(
        row()
            .key(format!("{key}-wrap"))
            .label(crate::i18n::t("common:actions.cancel"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .fill(Color::BLACK.alpha(if tk.dark { 0.5 } else { 0.25 }))
            .on_press(on_cancel)
            .child(dialog),
    );
}

/// The ⌄/› chevron of a disclosure (`ChevronRight` turned 90° when open).
pub fn chevron(tk: &Tk, open: bool, size: f32) -> El {
    ic(
        if open {
            &I::CHEVRON_DOWN
        } else {
            &I::CHEVRON_RIGHT
        },
        size,
        tk.muted_fg,
    )
}

/// `kb-glass-popover rounded-md border bg-popover p-1` with the glass popover's drop shadow
/// (transactions/menu.rs, copied: that module is private).
pub fn menu_surface(tk: &Tk, content: El) -> El {
    col()
        .child(content)
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

/// `SurfaceCardHeader` as the Wallets card uses it (`py-3.5 gap-4`, the card padding at the
/// sides in place of `px-6`): the card's `h2`
/// (`text-base font-medium`), a gray count badge, then the card's controls on the right.
pub fn card_header(tk: &Tk, title: String, count: Option<usize>, right: El) -> El {
    let mut left = row().grow(1.0).min_w(0.0).gap(tk.r(0.5)).center_y().child(
        text(title)
            .font(tk.tbase(500.0))
            .color(tk.fg)
            .role("heading")
            .level(2),
    );
    if let Some(n) = count {
        left = left.child(crate::ui::count(tk, n, tk.tbase(400.0)));
    }
    row()
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.875))
        .gap(tk.r(1.0))
        .center_y()
        .child(left)
        .child(right)
}
