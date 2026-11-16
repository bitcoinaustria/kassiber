//! shadcn `DropdownMenu` as the Transactions screen uses it (components/ui/dropdown-menu.tsx):
//! the `kb-glass-popover` surface (`rounded-md border p-1`), `text-sm` rows (`px-2 py-1.5`),
//! medium labels, `-mx-1 my-1` separators, check items (`pl-8` with a 1 rem check at `left-2`),
//! radio items (a 0.5 rem dot) and sub triggers (`ml-auto` chevron, `bg-accent` while open).
//!
//! Behaviour comes from `crate::ui::popup`; an open submenu is its own non-modal layer beside
//! its trigger (`Popup::mount_sub`), fitted to the window as Radix's collision handling does,
//! while the menu's modal, Escape and outside press cover both levels.

use std::rc::Rc;

use rune::{Color, El, Shadow, col, el, row, text};
use rune_kit::Placement;

use super::Popup;
use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, popup};

pub type Action = Rc<dyn Fn(&mut App)>;

pub enum Entry {
    Label(String),
    Sep,
    /// `DropdownMenuCheckboxItem`.
    Check {
        key: String,
        label: String,
        checked: bool,
        on: Action,
    },
    /// `DropdownMenuRadioItem` with `justify-between gap-4` and a muted trailing text.
    Radio {
        key: String,
        label: String,
        trailing: Option<String>,
        checked: bool,
        on: Action,
    },
    /// `DropdownMenuSub`: the trigger and its content.
    Sub {
        name: &'static str,
        label: String,
        dot: bool,
        disabled: bool,
        width: f32,
        max_h: Option<f32>,
        entries: Vec<Entry>,
    },
}

const ROW: f32 = 28.8;

fn row_h(tk: &Tk) -> f32 {
    tk.r(0.375) * 2.0 + tk.r(1.25)
}

fn entry_h(tk: &Tk, e: &Entry) -> f32 {
    match e {
        Entry::Sep => 1.0 + tk.r(0.25) * 2.0,
        _ => row_h(tk),
    }
}

/// `kb-glass-popover rounded-md border bg-popover p-1`, with the glass popover's drop shadow.
pub fn surface(tk: &Tk, content: El) -> El {
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

fn close_all(a: &mut App) {
    a.txs.popup = None;
}

/// A row's press: its action, then, when that closed the menu, focus back on the trigger by
/// key (a filter pick re-renders the trigger with its count, so `focus_return` alone misses).
fn pick(anchor: &str, on: Action) -> impl Fn(&mut App, &mut rune::Tx) + 'static {
    let anchor = anchor.to_owned();
    move |a: &mut App, tx: &mut rune::Tx| {
        on(a);
        if a.txs.popup.is_none() {
            popup::refocus(tx, &anchor);
        }
    }
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
                        .child(text(l).font(tk.tsm(500.0)).color(tk.fg)),
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
            Entry::Check {
                key,
                label,
                checked,
                on,
            } => {
                let mut r = base_row(tk, key.clone(), &label)
                    .pl(tk.r(2.0))
                    .pr(tk.r(0.5))
                    .role("menuitemcheckbox")
                    .checked(checked)
                    .on_press_tx(pick(anchor, on));
                if checked {
                    r = r.child(
                        row()
                            .absolute(tk.r(0.5), (h - tk.r(0.875)) / 2.0)
                            .size(tk.r(0.875), tk.r(0.875))
                            .center()
                            .child(ic(&I::CHECK, tk.r(1.0), tk.fg)),
                    );
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
                    false,
                );
            }
            Entry::Radio {
                key,
                label,
                trailing,
                checked,
                on,
            } => {
                let mut r = base_row(tk, key.clone(), &label)
                    .pl(tk.r(2.0))
                    .pr(tk.r(0.5))
                    .gap(tk.r(1.0))
                    .justify_between()
                    .role("menuitemradio")
                    .checked(checked)
                    .on_press_tx(pick(anchor, on));
                if checked {
                    r = r.child(
                        row()
                            .absolute(tk.r(0.5), (h - tk.r(0.875)) / 2.0)
                            .size(tk.r(0.875), tk.r(0.875))
                            .center()
                            .child(crate::ui::dot(tk.r(0.5), tk.fg)),
                    );
                }
                r = r.child(
                    text(label.clone())
                        .font(tk.tsm(400.0).features("'tnum' 1"))
                        .color(tk.fg),
                );
                if let Some(tr) = trailing {
                    r = r.child(
                        text(tr)
                            .font(tk.txs(400.0).features("'tnum' 1"))
                            .color(tk.muted_fg),
                    );
                }
                p.item(r, key, label, true, checked);
            }
            Entry::Sub {
                name,
                label,
                dot,
                disabled,
                width: sw,
                max_h,
                entries,
            } => {
                let open = open_sub == Some(name);
                let tkey = format!("txs-menu-sub-{name}");
                let mut r = base_row(tk, tkey.clone(), &label)
                    .px(tk.r(0.5))
                    .role("menuitem")
                    .fill(if open { tk.accent } else { Color::TRANSPARENT })
                    .child(text(label.clone()).font(tk.tsm(400.0)).color(tk.fg));
                if dot {
                    r = r.child(crate::ui::dot(tk.r(0.375), tk.primary).ml(tk.r(0.25)));
                }
                r = r
                    .child(el().grow(1.0))
                    .child(ic(&I::CHEVRON_RIGHT, tk.r(1.0), tk.fg));
                if disabled {
                    r = r.opacity(0.5);
                } else {
                    r = popup::sub_trigger(
                        r,
                        open,
                        format!("txs-menu-{name}-0"),
                        move |a: &mut App| a.txs.popup = Some(Popup::FiltersSub(name)),
                    );
                }
                // Radix places the sub content beside its trigger; the Filters menu sits at the
                // window's right edge, so it opens to the left, its first row level with the
                // trigger (the content's padding and border above it), 3 pt over the menu's edge.
                let edge = tk.r(0.25) + 1.0;
                r = r.child(popup::sub_anchor(&tkey, -edge));
                if open {
                    let mut sp = popup::Popup::sub(
                        popup::Kind::Menu,
                        anchor,
                        tkey.clone(),
                        close_all,
                        |a: &mut App| a.txs.popup = Some(Popup::Filters),
                    );
                    if let Some(mh) = max_h {
                        sp = sp.max_h(mh - 2.0 * edge);
                    }
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
                                .key(format!("txs-menu-{name}-content"))
                        },
                        rune::Side::Left,
                        edge - 3.0,
                    );
                    sub_el = out.pop();
                }
                p.item(r, tkey, label, !disabled, false);
            }
        }
    }
    let _ = ROW;
    sub_el
}

/// Mounts an open menu anchored to `anchor` (with a scrim and the Radix close rules).
#[allow(clippy::too_many_arguments)]
pub fn mount(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    width: f32,
    max_h: Option<f32>,
    open_sub: Option<&'static str>,
    entries: Vec<Entry>,
) {
    // Sub menu keys are numbered in their render: give each first row a predictable key.
    let entries = number(entries);
    let inner = width - 2.0 * (tk.r(0.25) + 1.0);
    let mut p = popup::Popup::menu(anchor, close_all).list_key(format!("{key}-list"));
    let sub = render(tk, &mut p, anchor, open_sub, entries, width);
    let wrap = move |l: El| {
        let mut s = surface(tk, col().w(inner).min_h(0.0).child(l))
            .w(width)
            .key(key.to_owned());
        if let Some(mh) = max_h {
            s = s.max_h(mh);
        }
        s
    };
    p.mount(tk, layer, col().w_full(), wrap, placement, 4.0);
    // An open submenu: its own layer above the menu, beside its trigger.
    layer.extend(sub);
}

/// Keys sub entries `txs-menu-{name}-{i}` so a sub trigger can move focus to its first row.
fn number(entries: Vec<Entry>) -> Vec<Entry> {
    entries
        .into_iter()
        .map(|e| match e {
            Entry::Sub {
                name,
                label,
                dot,
                disabled,
                width,
                max_h,
                entries,
            } => {
                let mut i = 0;
                let entries = entries
                    .into_iter()
                    .map(|x| match x {
                        Entry::Check {
                            label, checked, on, ..
                        } => {
                            let key = format!("txs-menu-{name}-{i}");
                            i += 1;
                            Entry::Check {
                                key,
                                label,
                                checked,
                                on,
                            }
                        }
                        other => other,
                    })
                    .collect();
                Entry::Sub {
                    name,
                    label,
                    dot,
                    disabled,
                    width,
                    max_h,
                    entries,
                }
            }
            other => other,
        })
        .collect()
}
