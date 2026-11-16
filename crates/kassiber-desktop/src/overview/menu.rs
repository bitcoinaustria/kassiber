//! The Holdings settings menu: shadcn `DropdownMenu` (components/ui/dropdown-menu.tsx) as
//! `HoldingsSettingsMenu` uses it. A copy of the Transactions menu adapted to the Overview's
//! state: `text-2xs` muted labels, radio and checkbox items that keep the menu open
//! (`onSelect={keepOpen}`), a disabled checkbox, and sub triggers that name their current value
//! (`SettingsSubTrigger`: label, a muted `max-w-28` value, the chevron).
//!
//! Behaviour comes from `crate::ui::popup`; an open submenu is its own non-modal layer beside
//! its trigger (`Popup::mount_sub`), fitted to the window as Radix's collision handling does,
//! while the menu's modal, Escape and outside press cover both levels.

use std::rc::Rc;

use rune::{Color, El, Shadow, col, el, row, text};
use rune_kit::Placement;

use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, popup};

pub type Action = Rc<dyn Fn(&mut App)>;

pub enum Entry {
    /// `DropdownMenuLabel className="text-2xs text-muted-foreground"`.
    Label(String),
    Sep,
    /// `DropdownMenuRadioItem`.
    Radio {
        key: String,
        label: String,
        checked: bool,
        on: Action,
    },
    /// `DropdownMenuCheckboxItem`.
    Check {
        key: String,
        label: String,
        checked: bool,
        disabled: bool,
        on: Action,
    },
    /// `DropdownMenuSub` with a `SettingsSubTrigger`: the label, its current value, the content.
    Sub {
        name: &'static str,
        label: String,
        value: String,
        width: f32,
        entries: Vec<Entry>,
    },
    /// A plain `DropdownMenuItem` (`px-2`): runs its action and closes the menu.
    Item {
        key: String,
        label: String,
        on: Action,
    },
}

fn row_h(tk: &Tk) -> f32 {
    tk.r(0.375) * 2.0 + tk.r(1.25)
}

fn entry_h(tk: &Tk, e: &Entry) -> f32 {
    match e {
        Entry::Sep => 1.0 + tk.r(0.25) * 2.0,
        Entry::Label(_) => tk.r(0.375) * 2.0 + tk.r(1.0),
        _ => row_h(tk),
    }
}

/// `kb-glass-popover rounded-md border bg-popover p-1`, with the glass popover's drop shadow.
fn surface(tk: &Tk, content: El) -> El {
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

fn base_row(tk: &Tk, key: String, label: &str) -> El {
    row()
        .key(key)
        .w_full()
        .h(row_h(tk))
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.25))
        .hover(tk.accent)
        .focus_highlight(tk.accent)
        .label(label.to_owned())
        .focusable(true)
}

fn close(a: &mut App) {
    a.ov.menu_open = false;
    a.ov.menu_sub = None;
    a.ov.menu_at = None;
}

/// One level's rows into `p`; an open sub's content comes back to be placed beside the list.
/// `width` is the menu's outer (border-box) width, as the web's `w-[…]` classes give it.
fn render(
    tk: &Tk,
    p: &mut popup::Popup,
    anchor: &str,
    open_sub: Option<&'static str>,
    entries: Vec<Entry>,
    width: f32,
) -> Option<El> {
    let inner = width - 2.0 * (tk.r(0.25) + 1.0);
    let mut sub_el: Option<El> = None;
    let indicator = |h: f32, child: El| {
        row()
            .absolute(tk.r(0.5), (h - tk.r(0.875)) / 2.0)
            .size(tk.r(0.875), tk.r(0.875))
            .center()
            .child(child)
    };
    for e in entries {
        let h = entry_h(tk, &e);
        match e {
            Entry::Label(l) => {
                p.add(
                    row()
                        .w_full()
                        .h(h)
                        .px(tk.r(0.5))
                        .center_y()
                        .child(text(l).font(tk.t2xs(500.0)).color(tk.muted_fg)),
                );
            }
            Entry::Sep => {
                p.add(
                    el().h(1.0)
                        .mx(-tk.r(0.25))
                        .my(tk.r(0.25))
                        .w(inner + tk.r(0.5))
                        .fill(tk.border),
                );
            }
            Entry::Radio {
                key,
                label,
                checked,
                on,
            } => {
                let mut r = base_row(tk, key.clone(), &label)
                    .pl(tk.r(2.0))
                    .pr(tk.r(0.5))
                    .role("menuitemradio")
                    .checked(checked)
                    .on_press(move |a: &mut App| on(a));
                if checked {
                    r = r.child(indicator(h, crate::ui::dot(tk.r(0.5), tk.fg)));
                }
                p.item(
                    r.child(
                        text(label.clone())
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .truncate(),
                    ),
                    key,
                    label,
                    true,
                    checked,
                );
            }
            Entry::Check {
                key,
                label,
                checked,
                disabled,
                on,
            } => {
                let mut r = base_row(tk, key.clone(), &label)
                    .pl(tk.r(2.0))
                    .pr(tk.r(0.5))
                    .role("menuitemcheckbox")
                    .checked(checked);
                if checked {
                    r = r.child(indicator(h, ic(&I::CHECK, tk.r(1.0), tk.fg)));
                }
                r = if disabled {
                    // `data-[disabled]:pointer-events-none data-[disabled]:opacity-50`.
                    r.opacity(0.5).hover(Color::TRANSPARENT)
                } else {
                    r.on_press(move |a: &mut App| on(a))
                };
                p.item(
                    r.child(
                        text(label.clone())
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .truncate(),
                    ),
                    key,
                    label,
                    !disabled,
                    false,
                );
            }
            Entry::Item { key, label, on } => {
                let r = base_row(tk, key.clone(), &label)
                    .px(tk.r(0.5))
                    .role("menuitem")
                    .on_press(move |a: &mut App| {
                        close(a);
                        on(a);
                    });
                p.item(
                    r.child(
                        text(label.clone())
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .truncate(),
                    ),
                    key,
                    label,
                    true,
                    false,
                );
            }
            Entry::Sub {
                name,
                label,
                value,
                width: sw,
                entries,
            } => {
                let open = open_sub == Some(name);
                let trigger_key = format!("ov-menu-sub-{name}");
                let r = base_row(tk, trigger_key.clone(), &format!("{label} {value}"))
                    .px(tk.r(0.5))
                    .gap(tk.r(0.75))
                    .role("menuitem")
                    .fill(if open { tk.accent } else { Color::TRANSPARENT })
                    // `flex-1 whitespace-nowrap` label; the `max-w-28 truncate` value gives way first.
                    .child(
                        text(label.clone())
                            .font(tk.tsm(400.0))
                            .color(tk.fg)
                            .grow(1.0),
                    )
                    .child(
                        text(value)
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .truncate()
                            .max_w(tk.r(7.0))
                            .shrink(),
                    )
                    .child(ic(&I::CHEVRON_RIGHT, tk.r(1.0), tk.muted_fg));
                // Radix places the sub content beside its trigger; the card's menu sits at the
                // page's right edge, so it opens to the left, its first row level with the trigger
                // (the content's padding and border above it). Measured against the PR build:
                // Radix leaves the content 10 pt right of and 10 pt below that position.
                let edge = tk.r(0.25) + 1.0;
                let r =
                    popup::sub_trigger(r, open, format!("ov-menu-{name}-0"), move |a: &mut App| {
                        a.ov.menu_sub = Some(name)
                    })
                    .child(popup::sub_anchor(&trigger_key, 10.0 - edge));
                if open {
                    let mut sp = popup::Popup::sub(
                        popup::Kind::Menu,
                        anchor,
                        trigger_key.clone(),
                        close,
                        |a: &mut App| a.ov.menu_sub = None,
                    );
                    let _ = render(tk, &mut sp, anchor, None, entries, sw);
                    let mut out = Vec::new();
                    // The surface styles its own element: keep the list (its `menu` role) inside.
                    sp.mount_sub(
                        tk,
                        &mut out,
                        col().w(sw - 2.0 * edge),
                        |b| {
                            surface(tk, col().min_h(0.0).child(b))
                                .w(sw)
                                .key(format!("ov-menu-{name}-content"))
                        },
                        rune::Side::Left,
                        edge - 10.0,
                    );
                    sub_el = out.pop();
                }
                p.item(r, trigger_key, label, true, false);
            }
        }
    }
    sub_el
}

/// Mounts the open menu anchored to `anchor` (with a scrim and the Radix close rules).
#[allow(clippy::too_many_arguments)]
pub fn mount(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    width: f32,
    open_sub: Option<&'static str>,
    entries: Vec<Entry>,
    placement: Placement,
) {
    let entries = number(entries);
    let inner = width - 2.0 * (tk.r(0.25) + 1.0);
    let mut p = popup::Popup::menu(anchor, close).list_key(format!("{key}-list"));
    let sub = render(tk, &mut p, anchor, open_sub, entries, width);
    let wrap = move |l: El| {
        surface(tk, col().w(inner).min_h(0.0).child(l))
            .w(width)
            .key(key.to_owned())
    };
    p.mount(tk, layer, col().w_full(), wrap, placement, 4.0);
    // An open submenu: its own layer above the menu, beside its trigger.
    layer.extend(sub);
}

/// Keys sub entries `ov-menu-{name}-{i}` so a sub trigger can move focus to its first row.
fn number(entries: Vec<Entry>) -> Vec<Entry> {
    entries
        .into_iter()
        .map(|e| match e {
            Entry::Sub {
                name,
                label,
                value,
                width,
                entries,
            } => {
                let entries = entries
                    .into_iter()
                    .enumerate()
                    .map(|(i, x)| match x {
                        Entry::Check {
                            label,
                            checked,
                            disabled,
                            on,
                            ..
                        } => Entry::Check {
                            key: format!("ov-menu-{name}-{i}"),
                            label,
                            checked,
                            disabled,
                            on,
                        },
                        Entry::Radio {
                            label, checked, on, ..
                        } => Entry::Radio {
                            key: format!("ov-menu-{name}-{i}"),
                            label,
                            checked,
                            on,
                        },
                        other => other,
                    })
                    .collect();
                Entry::Sub {
                    name,
                    label,
                    value,
                    width,
                    entries,
                }
            }
            other => other,
        })
        .collect()
}
