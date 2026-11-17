//! rune-kit's floating `Menu` look (the chart's year and gear menus were drawn with it) on the
//! shared popup behaviour ([`super::popup`]): the same rows, keys and frame, with Radix's
//! keyboard, Escape and submenus. Submenus open beside their row as their own layer, on hover,
//! a press, ArrowRight or Enter; ArrowLeft closes them, Escape or a press outside closes all.

use std::rc::Rc;

use rune::{El, Icon, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::Placement;

use super::popup::{self, Kind, Popup};
use crate::app::App;
use crate::theme::Tk;

pub type Act = Rc<dyn Fn(&mut App, &mut Tx)>;

pub enum Entry {
    Item(Item),
    Sep,
    Label(String),
}

/// A row: `checked` makes it a check item; `sub` a submenu trigger.
pub struct Item {
    pub label: String,
    pub description: Option<String>,
    pub badge: Option<String>,
    pub shortcut: Option<String>,
    pub icon: Option<&'static Icon>,
    pub checked: Option<bool>,
    pub disabled: bool,
    pub on: Option<Act>,
    pub sub: Option<Sub>,
}

pub struct Sub {
    pub open: bool,
    pub entries: Vec<Entry>,
    /// Opens the submenu (hover, press, ArrowRight); `close` goes back to the parent alone.
    pub open_fn: Rc<dyn Fn(&mut App)>,
    pub close_fn: Rc<dyn Fn(&mut App)>,
}

impl Item {
    pub fn new(label: impl Into<String>) -> Self {
        Item {
            label: label.into(),
            description: None,
            badge: None,
            shortcut: None,
            icon: None,
            checked: None,
            disabled: false,
            on: None,
            sub: None,
        }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
    pub fn badge(mut self, b: impl Into<String>) -> Self {
        self.badge = Some(b.into());
        self
    }
    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }
    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        self.shortcut = Some(s.into());
        self
    }
    pub fn icon(mut self, i: &'static Icon) -> Self {
        self.icon = Some(i);
        self
    }
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }
    pub fn on_select(self, f: impl Fn(&mut App) + 'static) -> Self {
        self.on_select_tx(move |a: &mut App, _tx: &mut Tx| f(a))
    }
    pub fn on_select_tx(mut self, f: impl Fn(&mut App, &mut Tx) + 'static) -> Self {
        self.on = Some(Rc::new(f));
        self
    }
    pub fn submenu(
        mut self,
        entries: Vec<Entry>,
        open: bool,
        open_fn: impl Fn(&mut App) + 'static,
        close_fn: impl Fn(&mut App) + 'static,
    ) -> Self {
        self.sub = Some(Sub {
            open,
            entries,
            open_fn: Rc::new(open_fn),
            close_fn: Rc::new(close_fn),
        });
        self
    }
}

/// One menu (or submenu) mounted in `layer`: `key` names it as rune-kit did (rows
/// `{key}-item{i}`, a submenu `{row}-sub`), `anchor` is its trigger (or the sub's row).
#[allow(clippy::too_many_arguments)]
pub fn mount(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    placement: Placement,
    gutter: f32,
    min_width: f32,
    entries: Vec<Entry>,
    close: Rc<dyn Fn(&mut App)>,
) {
    mount_level(
        tk, layer, key, anchor, None, placement, gutter, min_width, entries, close,
    );
}

#[allow(clippy::too_many_arguments)]
fn mount_level(
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    anchor: &str,
    sub_of: Option<(&str, crate::app::Handler)>,
    placement: Placement,
    gutter: f32,
    min_width: f32,
    entries: Vec<Entry>,
    close: Rc<dyn Fn(&mut App)>,
) {
    let k = tk.kit();
    let c = &k.c;
    let cl = close.clone();
    let mut p = match &sub_of {
        None => Popup::menu(anchor, move |a: &mut App| cl(a)),
        Some((root, close_sub)) => {
            let cs = close_sub.clone();
            Popup::sub(
                Kind::Menu,
                *root,
                anchor,
                move |a: &mut App| cl(a),
                move |a: &mut App| cs(a),
            )
        }
    }
    .list_key(format!("{key}-menu"));
    let root_anchor = sub_of
        .as_ref()
        .map(|(r, _)| r.to_string())
        .unwrap_or_else(|| anchor.to_owned());
    let mut deeper: Vec<(String, Sub)> = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        match entry {
            Entry::Sep => p.add(
                el().key(format!("sep{i}"))
                    .h(1.0)
                    .mt(2.0)
                    .mb(2.0)
                    .ml(-2.0)
                    .mr(-2.0)
                    .fill(c.border_muted)
                    .a11y_hidden(),
            ),
            Entry::Label(l) => p.add(
                row()
                    .key(format!("label{i}"))
                    .center_y()
                    .h(28.0)
                    .px(12.0)
                    .child(text(l).font(k.ty.small_medium).color(c.text_faint)),
            ),
            Entry::Item(it) => {
                let ikey = format!("{key}-item{i}");
                let disabled = it.disabled;
                let sub_open = it.sub.as_ref().is_some_and(|s| s.open);
                let mut content = row()
                    .key("content")
                    .grow(1.0)
                    .shrink()
                    .gap(8.0)
                    .center_y()
                    .a11y_hidden();
                if let Some(g) = it.icon {
                    content = content.child(icon(g, 14.0).key("icon").color(c.icon_base));
                }
                let label = text(it.label.clone())
                    .key("label")
                    .font(k.ty.ui_compact)
                    .color(c.text_base)
                    .truncate();
                content = match &it.description {
                    Some(d) => content.child(
                        col()
                            .key("text")
                            .grow(1.0)
                            .shrink()
                            .gap(2.0)
                            .child(label)
                            .child(
                                text(d.clone())
                                    .key("description")
                                    .font(k.ty.small)
                                    .color(c.text_faint)
                                    .truncate(),
                            ),
                    ),
                    None => content.child(label),
                };
                let mut e = row()
                    .role(if it.checked.is_some() {
                        "menuitemcheckbox"
                    } else {
                        "menuitem"
                    })
                    .label(it.label.clone())
                    .radius(4.0)
                    .gap(8.0)
                    .center_y();
                if let Some(on) = it.checked {
                    e = e.checked(on);
                }
                if let Some(s) = &it.shortcut {
                    e = e.shortcut_hint(s.clone());
                }
                match (&it.description, &it.badge) {
                    (Some(d), Some(b)) => e = e.description(format!("{d}, {b}")),
                    (Some(d), None) | (None, Some(d)) => e = e.description(d.clone()),
                    (None, None) => {}
                }
                e = if it.description.is_some() {
                    e.min_h(28.0).py(6.0).pl(12.0).pr(6.0)
                } else {
                    e.h(28.0).pl(12.0).pr(6.0)
                };
                e = if sub_open {
                    e.fill(c.hover)
                } else if !disabled {
                    e.focus_highlight(c.hover)
                } else {
                    e
                };
                e = e.child(content);
                if let Some(s) = it.shortcut {
                    e = e.child(
                        text(s)
                            .key("shortcut")
                            .font(TextStyle {
                                line_height: 11.0,
                                ..k.ty.small_medium
                            })
                            .color(c.text_faint)
                            .a11y_hidden(),
                    );
                }
                if let Some(b) = it.badge {
                    e = e.child(
                        row()
                            .a11y_hidden()
                            .key("badge")
                            .h(16.0)
                            .px(4.0)
                            .center()
                            .radius(2.0)
                            .fill(c.layer_02)
                            .border(0.5, c.border_base)
                            .child(
                                text(b)
                                    .font(TextStyle {
                                        line_height: 11.0,
                                        ..k.ty.small_medium
                                    })
                                    .color(c.text_muted),
                            ),
                    );
                }
                if let Some(on) = it.checked {
                    let mark = icon(k.glyph.check_small, 14.0)
                        .color(c.icon_base)
                        .opacity(if on { 1.0 } else { 0.0 });
                    e = e.child(
                        el().key("indicator")
                            .size(16.0, 16.0)
                            .center()
                            .child(mark)
                            .a11y_hidden(),
                    );
                }
                if disabled {
                    e = e.layer_opacity(0.5);
                }
                if let Some(s) = it.sub {
                    e = e.child(
                        icon(k.glyph.caret_right, 16.0)
                            .key("chevron")
                            .a11y_hidden()
                            .mirror_in_rtl()
                            .color(c.text_faint),
                    );
                    if !disabled {
                        let of = s.open_fn.clone();
                        e = popup::sub_trigger(
                            e,
                            s.open,
                            format!("{ikey}-sub-item0"),
                            move |a: &mut App| of(a),
                        );
                    }
                    if s.open {
                        deeper.push((ikey.clone(), s));
                    }
                } else if let Some(on) = it.on
                    && !disabled
                {
                    e = e.on_press_tx(move |a: &mut App, tx: &mut Tx| on(a, tx));
                }
                p.item(e, ikey, it.label, !disabled, it.checked == Some(true));
            }
        }
    }
    let frame = |l: El| {
        k.elevate(
            col()
                .min_w(min_width)
                .min_h(0.0)
                .items_stretch()
                .radius(6.0)
                .fill(c.layer_01)
                .child(l),
            &k.elevation.floating,
        )
    };
    let container = col().items_stretch().padding(2.0);
    match &sub_of {
        None => p.mount(tk, layer, container, frame, placement, gutter),
        Some(_) => {
            // A submenu: a non-modal layer beside its row, as rune-kit's own menu mounts it, so
            // the parent stays live (and in the tree) under the modal it already owns; the
            // parent's scrim closes both on an outside press.
            let (list, keys) = p.finish(container);
            let list = col()
                .key(format!("{anchor}-scroll"))
                .shrink()
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list);
            let s = keys.attach(frame(list).min_h(0.0));
            layer.push(crate::shell::anchored(tk, s, anchor, placement, gutter));
        }
    }
    for (ikey, s) in deeper {
        mount_level(
            tk,
            layer,
            &format!("{ikey}-sub"),
            &ikey,
            Some((&root_anchor, s.close_fn.clone())),
            Placement::RightStart,
            0.0,
            160.0,
            s.entries,
            close.clone(),
        );
    }
}
