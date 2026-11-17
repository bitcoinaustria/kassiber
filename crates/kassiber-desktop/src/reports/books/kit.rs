//! shadcn pieces Books and the bird's-eye view draw with: Dialog, Select, DropdownMenu, Badge,
//! Label, the destructive error box and the page's outline/ghost buttons. Popups close through
//! this part's own state (`app.reports.books`), so they are copies of the shell's helpers.

use rune::{Color, El, Icon, Shadow, col, el, row, text};
use rune_kit::Placement;

use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// Closes whichever select or menu is open in this part.
pub fn close_popup(a: &mut App) {
    a.reports.books.popup = None;
}

/// `DialogContent className="sm:max-w-md"`: a faint `kb-glass-backdrop`, then the
/// `kb-glass-dialog` surface (`background` at 80 % over a 12 px blur, 1 rem radius, hairline,
/// deep shadow), `p-(--kb-card-padding)` and `gap-4`, centred at `top: inset + max(1rem, 8vh)`,
/// with the `absolute top-3 right-3 size-7` close ×. `content` is the form (`space-y-4`).
#[allow(clippy::too_many_arguments)]
pub fn dialog(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    title: &str,
    description: &str,
    content: El,
    close: fn(&mut App),
) {
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    // `top: max(1rem, 8vh)` below the window's top inset.
    let top = crate::shell::dialog_top(tk, win_h);
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        el().key(format!("{key}-scrim"))
            .absolute(0.0, crate::shell::window_top_inset())
            .w_full()
            .h_full()
            .fill(wash)
            .on_press(close)
            .a11y_hidden(),
    );
    let close_btn = row()
        .key(format!("{key}-close"))
        .size(tk.r(1.75), tk.r(1.75))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.accent)
        .role("button")
        .label(t("common:actions.close"))
        .focusable(true)
        .on_press(close)
        .child(ic(&I::X, tk.r(1.0), tk.muted_fg))
        .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None);
    let width = (win_w - tk.r(2.0)).min(tk.r(28.0));
    let surface = crate::shell::glass_dialog(
        tk,
        col()
            .key(key.to_owned())
            .role("dialog")
            .label(title.to_owned())
            .description(description.to_owned())
            .w(width)
            .padding(tk.r(1.25))
            .modal(true, close)
            .on_press(|_: &mut App| {})
            .child(content)
            .child(close_btn),
    );
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// [`dialog`] `width` wide whose content scrolls inside once it is taller than the window
/// (`max-h-[calc(100vh-top-1rem)] overflow-y-auto`): the book settings dialog's sections. The
/// close × scrolls with the content, as it does in the web's scrolling `DialogContent`, so it
/// never sits over a field.
#[allow(clippy::too_many_arguments)]
pub fn dialog_scroll(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    title: &str,
    description: &str,
    width: f32,
    content: El,
    close: fn(&mut App),
) {
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    let win_h = if app.shell.window_h > 0.0 {
        app.shell.window_h
    } else {
        900.0
    };
    let top = crate::shell::dialog_top(tk, win_h);
    let wash = if tk.dark {
        Color::BLACK.alpha(0.22)
    } else {
        tk.fg.alpha(0.06)
    };
    layer.push(
        el().key(format!("{key}-scrim"))
            .absolute(0.0, crate::shell::window_top_inset())
            .w_full()
            .h_full()
            .fill(wash)
            .on_press(close)
            .a11y_hidden(),
    );
    let close_btn = row()
        .key(format!("{key}-close"))
        .size(tk.r(1.75), tk.r(1.75))
        .center()
        .radius(tk.r(0.375))
        .hover(tk.accent)
        .role("button")
        .label(t("common:actions.close"))
        .focusable(true)
        .on_press(close)
        .child(ic(&I::X, tk.r(1.0), tk.muted_fg))
        .pin(None, Some(tk.r(0.75)), Some(tk.r(0.75)), None);
    let width = (win_w - tk.r(2.0)).min(width);
    let max_h = (win_h - top - tk.r(1.0)).max(tk.r(12.0));
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
                .child(content)
                .child(close_btn),
        );
    let surface = crate::shell::glass_dialog(
        tk,
        col()
            .key(key.to_owned())
            .role("dialog")
            .label(title.to_owned())
            .description(description.to_owned())
            .w(width)
            .modal(true, close)
            .on_press(|_: &mut App| {})
            .child(body),
    );
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// A dialog section: a `text-sm font-semibold` heading (`role="heading"`), an optional muted
/// line, then its fields `space-y-3`; a hairline above every section but the first.
pub fn section(
    tk: &Tk,
    key: &str,
    title: &str,
    lead: Option<&str>,
    first: bool,
    fields: Vec<El>,
) -> El {
    let mut head = col().w_full().gap(tk.r(0.25)).child(
        text(title.to_owned())
            .font(tk.tsm(600.0))
            .color(tk.fg)
            .role("heading")
            .level(3),
    );
    if let Some(l) = lead {
        head = head.child(note(tk, l));
    }
    let mut s = col()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(0.75))
        .role("group")
        .label(title.to_owned())
        .child(head);
    for f in fields {
        s = s.child(f);
    }
    if first {
        s
    } else {
        s.pt(tk.r(1.0))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout()
    }
}

/// `DialogHeader` (`flex flex-col gap-2`): `DialogTitle` (`text-lg leading-none font-semibold`)
/// over `DialogDescription` (`text-sm text-muted-foreground`).
pub fn dialog_header(tk: &Tk, title: &str, description: &str) -> El {
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            text(title.to_owned())
                .font(tk.tlg(600.0).lh(tk.r(1.125)))
                .color(tk.fg)
                .pr(tk.r(1.5)),
        )
        .child(
            text(description.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .w_full(),
        )
}

/// `rounded-lg border bg-muted/25 p-3 text-sm`: the name, then `mt-1 text-xs` meta.
pub fn identity_box(tk: &Tk, name: &str, meta: &str) -> El {
    col()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.25))
        .child(
            text(name.to_owned())
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .wrap()
                .w_full(),
        )
        .child(
            text(meta.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .mt(tk.r(0.25))
                .wrap()
                .w_full(),
        )
}

/// `rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive`.
pub fn error_box(tk: &Tk, msg: &str) -> El {
    row()
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.destructive.alpha(0.25))
        .fill(tk.destructive.alpha(0.1))
        .role("alert")
        .child(
            text(msg.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .w_full(),
        )
}

/// `Label` (`text-sm leading-none font-medium`) over its control, `space-y-2`.
pub fn labelled(tk: &Tk, label: &str, control: El) -> El {
    col()
        .w_full()
        .items_start()
        .gap(tk.r(0.5))
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0).lh(tk.r(0.875)))
                .color(tk.fg),
        )
        .child(control)
}

/// A muted `text-xs` note under a field.
pub fn note(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.txs(400.0))
        .color(tk.muted_fg)
        .wrap()
        .w_full()
}

/// `TaxCountryPreviewBadge` (`Badge variant="outline" ml-1.5`).
pub fn preview_badge(tk: &Tk) -> El {
    badge_outline(
        tk,
        &t("common:taxCountry.previewBadge"),
        None,
        tk.fg,
        tk.border,
    )
    .ml(tk.r(0.375))
}

/// `Badge variant="secondary"`: `rounded-full px-2 py-0.5 text-xs font-medium bg-secondary`.
pub fn badge_secondary(tk: &Tk, label: &str, glyph: Option<&'static Icon>) -> El {
    badge_base(tk, label, glyph, tk.fg)
        .fill(tk.muted)
        .border(1.0, Color::TRANSPARENT)
}

/// `Badge variant="outline"` in `ink` with a `border` colour.
pub fn badge_outline(
    tk: &Tk,
    label: &str,
    glyph: Option<&'static Icon>,
    ink: Color,
    border: Color,
) -> El {
    badge_base(tk, label, glyph, ink).border(1.0, border)
}

fn badge_base(tk: &Tk, label: &str, glyph: Option<&'static Icon>, ink: Color) -> El {
    let mut b = row()
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .gap(tk.r(0.25))
        .center()
        .radius(999.0)
        .border_layout();
    if let Some(g) = glyph {
        b = b.child(ic(g, tk.r(0.75), ink));
    }
    b.child(text(label.to_owned()).font(tk.txs(500.0)).color(ink))
}

/// Amber ink (`text-amber-700 dark:text-amber-300`).
pub fn amber(tk: &Tk) -> Color {
    rune::color::hex(if tk.dark { "#fcd34d" } else { "#b45309" }).unwrap_or(tk.fg)
}

/// `SelectTrigger`: `h-9 gap-2 rounded-md border border-input px-3 text-sm shadow-xs`
/// (`dark:bg-input/30`), the value (with an optional badge) and a 50 % chevron. `full` is
/// `w-full`; otherwise it is `w-fit`.
pub fn select_trigger(
    tk: &Tk,
    key: &str,
    label: &str,
    value: &str,
    badge: Option<El>,
    enabled: bool,
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
    let mut v = row().min_w(0.0).center_y().child(
        text(value.to_owned())
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .truncate(),
    );
    if let Some(b) = badge {
        v = v.child(b);
    }
    let mut r = row()
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
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(0.05),
            ..Shadow::default()
        })
        // Named by its value, as the web's trigger is (its text content); the field label
        // describes it.
        .role("combobox")
        .label(value.to_owned())
        .description(label.to_owned())
        .a11y_value(value.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .opacity(if enabled { 1.0 } else { 0.5 })
        .child(v)
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    if full {
        r = r.w_full();
    }
    r
}

/// `SelectContent` + `SelectItem`s under the trigger keyed `anchor`: `p-1`, items `py-1.5 pr-8 pl-2
/// text-sm`, the check at `right-2`. Options are (value, label, badge?).
pub fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    min_w: f32,
    options: Vec<(String, String, bool)>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let mut p = crate::ui::popup::Popup::listbox(anchor, close_popup);
    // The popup's scroll viewport doesn't pass its content's width up, so the list takes the
    // widest row's width itself (label, Preview badge, the check's `pr-8`): never narrower
    // than its trigger, and no row clipped mid-badge.
    let widest = options
        .iter()
        .map(|(_, label, badge)| option_w(tk, label, *badge))
        .fold(0.0, f32::max);
    let list = col()
        .min_w(min_w.max(widest + 2.0 * tk.r(0.25)))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
    for (v, label, badge) in options {
        let checked = v == current;
        let pick = choose.clone();
        let val = v.clone();
        let mut item = row()
            .min_w(0.0)
            .grow(1.0)
            .center_y()
            .child(text(label.clone()).font(tk.tsm(400.0)).color(tk.fg));
        if badge {
            item = item.child(preview_badge(tk));
        }
        p.item(
            row()
                .w_full()
                .py(tk.r(0.375))
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
                    close_popup(a);
                    pick(a, &val);
                })
                .child(item)
                .child(
                    row()
                        .size(tk.r(0.875), tk.r(0.875))
                        .center()
                        .pin(None, None, Some(tk.r(0.5)), None)
                        .child(if checked {
                            ic(&I::CHECK, tk.r(1.0), tk.fg)
                        } else {
                            el().size(1.0, 1.0)
                        }),
                ),
            format!("{anchor}-opt-{v}"),
            label,
            true,
            checked,
        );
    }
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        4.0,
    );
}

/// An option row's natural width: `pl-2`, the label, the Preview badge (`ml-1.5`, `px-2`,
/// its hairline) where it has one, then `pr-8` for the check.
fn option_w(tk: &Tk, label: &str, badge: bool) -> f32 {
    let m = |s: &str, style: rune::TextStyle| rune::text::measure(s, &[], style, None).0;
    let mut w = tk.r(0.5) + m(label, tk.tsm(400.0)) + tk.r(2.0);
    if badge {
        w += tk.r(0.375)
            + 2.0 * tk.r(0.5)
            + 2.0
            + m(&t("common:taxCountry.previewBadge"), tk.txs(500.0));
    }
    w.ceil() + 1.0
}

/// A `DropdownMenuContent` under `anchor` (`p-1`, `w-48` here), modal as Radix's default.
/// `items` are (row, key, label) from [`menu_item`].
pub fn dropdown(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    width: f32,
    placement: Placement,
    items: Vec<(El, String, String)>,
) {
    let mut p = crate::ui::popup::Popup::menu(anchor, close_popup);
    for (el, key, label) in items {
        p.item(el, key, label, true, false);
    }
    p.mount(
        tk,
        layer,
        col().w(width).padding(tk.r(0.25)),
        |l| crate::shell::glass(tk, l),
        placement,
        4.0,
    );
}

/// `DropdownMenuItem`: `gap-2 rounded-sm px-2 py-1.5 text-sm`, a muted 1 rem glyph.
pub fn menu_item(tk: &Tk, key: &str, glyph: &'static Icon, label: &str) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .px(tk.r(0.5))
        .py(tk.r(0.375))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .role("menuitem")
        .label(label.to_owned())
        .focusable(true)
        .child(ic(glyph, tk.r(1.0), tk.muted_fg))
        .child(text(label.to_owned()).font(tk.tsm(400.0)).color(tk.fg))
}
