//! The small shadcn pieces the Reports cards share: `Select` (trigger and item-aligned content),
//! `InfoNote`, `Checkbox` and the cards' arrow links.

use rune::{Color, El, Shadow, col, el, row, text};
use rune_kit::Placement;

use super::Popup;
use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, surface};

pub fn close(a: &mut App) {
    a.reports.report.popup = None;
}

pub fn toggle(a: &mut App, p: Popup) {
    a.reports.report.popup = if a.reports.report.popup.as_ref() == Some(&p) {
        None
    } else {
        Some(p)
    };
}

/// `tabular-nums`.
pub fn tnum(s: rune::TextStyle) -> rune::TextStyle {
    s.features("'tnum' 1")
}

/// `SelectTrigger` at `h-8` (`size="sm"` or `h-8` classes): `gap-2 rounded-md border
/// border-input px-3 shadow-xs`, `dark:bg-input/30`, the value and a 50 % chevron. `xs` is
/// `text-xs`, else `text-sm`. Named by `label` (its `aria-label`), or by the value.
#[allow(clippy::too_many_arguments)]
pub fn select_trigger(
    tk: &Tk,
    key: &str,
    label: Option<&str>,
    value: &str,
    placeholder: bool,
    xs: bool,
    enabled: bool,
    open: bool,
    full: bool,
) -> El {
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, Color::TRANSPARENT)
    };
    let font = tnum(if xs { tk.txs(400.0) } else { tk.tsm(400.0) });
    let mut r = row()
        .key(key.to_owned())
        .h(tk.r(2.0))
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
        .label(label.unwrap_or(value).to_owned())
        .a11y_value(value.to_owned())
        .expanded(open)
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 })
        .child(
            text(value.to_owned())
                .font(font)
                .color(if placeholder { tk.muted_fg } else { tk.fg })
                .truncate(),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    if let Some(l) = label {
        r = r.help(l.to_owned());
    }
    if full { r.w_full() } else { r }
}

/// `SelectContent` (`position="item-aligned"`, Radix's default): the current option sits over
/// the trigger and the list is at least as wide. Options are (value, label, disabled).
#[allow(clippy::too_many_arguments)]
pub fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    trigger_w: f32,
    options: Vec<(String, String, bool)>,
    current: &str,
    tnums: bool,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let selected = options.iter().position(|(v, _, _)| v == current);
    let mut p = crate::ui::popup::Popup::listbox(anchor, close);
    // `SelectItem`: `py-1.5 pr-8 pl-2 text-sm`, the check at `right-2`.
    let item_h = tk.r(0.375) * 2.0 + tk.r(1.25);
    let pad = tk.r(0.25);
    let list = col()
        .min_w(trigger_w - 2.0 - 2.0 * pad)
        .label(anchor.to_owned());
    for (v, label, disabled) in options {
        let checked = v == current;
        let pick = choose.clone();
        let val = v.clone();
        let font = if tnums {
            tnum(tk.tsm(400.0))
        } else {
            tk.tsm(400.0)
        };
        let mut item = row()
            .key(format!("{anchor}-opt-{v}"))
            .w_full()
            .h(item_h)
            .pl(tk.r(0.5))
            .pr(tk.r(2.0))
            .center_y()
            .radius(tk.r(0.25))
            .role("option")
            .checked(checked)
            .label(label.clone())
            .disabled(disabled)
            .focusable(!disabled)
            .focus_selected(checked)
            .opacity(if disabled { 0.5 } else { 1.0 })
            .child(text(label.clone()).font(font).color(tk.fg))
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
            let back = anchor.to_owned();
            item = item
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press_tx(move |a: &mut App, tx: &mut rune::Tx| {
                    close(a);
                    pick(a, &val);
                    // The year changes the route, which remounts the page under the trigger; while
                    // the report reloads the trigger is gone, so the page asks again once it shows.
                    crate::ui::popup::refocus(tx, &back);
                    a.reports.report.refocus = Some((back.clone(), tx.by_keys()));
                });
        }
        p.item(item, format!("{anchor}-opt-{v}"), label, !disabled, checked);
    }
    // Item-aligned: the selected option's row centred on the trigger (h-8).
    let gap = match selected {
        Some(i) => -(tk.r(2.0) / 2.0 + item_h / 2.0 + pad + 1.0 + i as f32 * item_h),
        None => 4.0,
    };
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).padding(pad).child(l)),
        Placement::BottomStart,
        gap,
    );
}

/// `InfoNote`: a small info button whose notes open on click (`PopoverContent` `w-72 p-3
/// text-xs`, `align="start"`).
pub fn info_note(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    label: &str,
    notes: &[String],
) -> Option<El> {
    if notes.is_empty() {
        return None;
    }
    let open = app.reports.report.popup.as_ref() == Some(&Popup::Note(key.to_owned()));
    let k = key.to_owned();
    let button = row()
        .key(key.to_owned())
        .size(tk.r(0.875), tk.r(0.875))
        .center()
        .radius(tk.r(0.125))
        .role("button")
        .label(label.to_owned())
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| toggle(a, Popup::Note(k.clone())))
        .child(ic(&I::INFO, tk.r(0.875), tk.muted_fg));
    if open {
        let kit = tk.kit();
        layer.push(rune_kit::scrim(format!("{key}-scrim"), close));
        let body = col()
            .w(tk.r(18.0))
            .padding(tk.r(0.75))
            .gap(tk.r(0.25))
            .radius(tk.r(0.375))
            .fill(tk.popover)
            .border(1.0, tk.border)
            .border_layout()
            .box_shadow(Shadow {
                y: 4.0,
                blur: 6.0,
                color: Color::BLACK.alpha(0.1),
                ..Shadow::default()
            })
            .role("dialog")
            .label(label.to_owned())
            .children(notes.iter().map(|n| {
                text(n.clone())
                    .font(tk.txs(400.0))
                    .color(tk.fg)
                    .wrap()
                    .w_full()
            }))
            .modal(true, close)
            .focus_return(key.to_owned());
        layer.push(rune_kit::popover::anchored(
            &kit,
            body,
            key,
            Placement::BottomStart,
            4.0,
        ));
    }
    Some(button)
}

/// shadcn `Checkbox`: `size-4 rounded-[4px] border border-input shadow-xs`, checked
/// `bg-primary` with a `size-3.5` check; decorative here (the label row is the control).
pub fn checkbox(tk: &Tk, on: bool) -> El {
    let b = row().size(tk.r(1.0), tk.r(1.0)).center().radius(4.0);
    if on {
        b.fill(tk.primary)
            .border(1.0, tk.primary)
            .child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b.fill(if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
        .border(1.0, tk.input)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
    }
}

/// A muted `text-xs` link with a trailing `ArrowUpRight` (`inline-flex items-center gap-1`).
pub fn arrow_link(tk: &Tk, key: &str, label: &str, href: String) -> El {
    row()
        .key(key.to_owned())
        .gap(tk.r(0.25))
        .center_y()
        .role("link")
        .label(label.to_owned())
        .focusable(true)
        .on_press(move |a: &mut App| a.navigate(&href))
        // `hover:text-foreground`: label and arrow brighten under the pointer.
        .cursor(rune::Cursor::Pointer)
        .hover_group()
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .group_color(tk.fg),
        )
        .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.875), tk.muted_fg).group_color(tk.fg))
}

/// The page's cards: `kb-surface overflow-hidden`.
pub fn card(tk: &Tk, key: &str) -> El {
    surface(tk)
        .key(key.to_owned())
        .w_full()
        .min_w(0.0)
        .clip_rounded(tk.r(0.75))
}
