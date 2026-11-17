//! The settings building blocks (SettingsControls.tsx): the hairline group box with divided
//! rows, the row (name + explanation left, control right), the fixed-width select, switches,
//! buttons, copy rows, the folding disclosure, inputs and the glass dialog frame.

use rune::{Color, El, Icon, TextEdit, col, el, row, text};
use rune_kit::{Placement, TextInput};

use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button_surface, field, form_kit};
use crate::ui::ic;

/// One option of a `select`.
#[derive(Clone)]
pub struct Opt {
    pub value: String,
    pub label: String,
    pub title: Option<String>,
    pub disabled: bool,
}

pub fn opt(value: impl Into<String>, label: impl Into<String>) -> Opt {
    Opt {
        value: value.into(),
        label: label.into(),
        title: None,
        disabled: false,
    }
}

/// `text-sm text-muted-foreground`, wrapping.
pub fn muted(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into()).font(tk.tsm(400.0)).color(tk.muted_fg).wrap()
}

/// A paragraph inside a group box (`px-4 py-3 text-sm`).
pub fn para(tk: &Tk, s: impl Into<String>, color: Color) -> El {
    col()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .child(text(s.into()).font(tk.tsm(400.0)).color(color).wrap())
}

/// An outcome row (an import's, a rebuild's, a backup's) with its × (owner request over the web;
/// leaving the section or the next run clears it too). `content` is a padded full-width row.
pub fn dismissable(
    tk: &Tk,
    key: &str,
    content: El,
    ink: Color,
    on_press: impl Fn(&mut App) + 'static,
) -> El {
    row()
        .w_full()
        .items_start()
        .pr(tk.r(0.5))
        .child(col().grow(1.0).w(0.0).min_w(0.0).child(content))
        .child(crate::shell::transient::dismiss(tk, key.to_owned(), ink, on_press).mt(tk.r(0.5)))
}

/// `text-emerald-600 dark:text-emerald-400`, `text-amber-600 dark:text-amber-400`.
pub fn emerald(tk: &Tk) -> Color {
    if tk.dark {
        rune::color::hex("#34d399").unwrap()
    } else {
        rune::color::hex("#059669").unwrap()
    }
}
pub fn amber(tk: &Tk) -> Color {
    if tk.dark {
        rune::color::hex("#fbbf24").unwrap()
    } else {
        rune::color::hex("#d97706").unwrap()
    }
}
/// `text-amber-700 dark:text-amber-300`.
pub fn amber_soft(tk: &Tk) -> Color {
    if tk.dark {
        rune::color::hex("#fcd34d").unwrap()
    } else {
        rune::color::hex("#b45309").unwrap()
    }
}

fn anchor_key(anchor: &str) -> String {
    format!("set-anchor-{anchor}")
}

/// The settings list box (`SettingsGroup`): optional title (`text-sm font-semibold`) and actions
/// above, `divide-y rounded-lg border bg-background` box, footer note (`px-1 text-xs`) below.
pub struct Group {
    pub title: Option<String>,
    pub actions: Vec<El>,
    pub footer: Option<El>,
    pub anchor: Option<&'static str>,
    pub rows: Vec<El>,
}

pub fn group() -> Group {
    Group {
        title: None,
        actions: vec![],
        footer: None,
        anchor: None,
        rows: vec![],
    }
}

impl Group {
    pub fn title(mut self, s: impl Into<String>) -> Self {
        self.title = Some(s.into());
        self
    }
    pub fn action(mut self, e: El) -> Self {
        self.actions.push(e);
        self
    }
    pub fn footer(mut self, s: impl Into<String>) -> Self {
        self.footer = Some(text(s.into()));
        self
    }
    pub fn footer_el(mut self, e: El) -> Self {
        self.footer = Some(e);
        self
    }
    pub fn anchor(mut self, a: &'static str) -> Self {
        self.anchor = Some(a);
        self
    }
    pub fn row(mut self, e: El) -> Self {
        self.rows.push(e);
        self
    }
    pub fn rows(mut self, e: impl IntoIterator<Item = El>) -> Self {
        self.rows.extend(e);
        self
    }
    pub fn build(self, app: &App, tk: &Tk) -> El {
        let mut c = col().w_full().gap(tk.r(0.5));
        if let Some(a) = self.anchor {
            c = c.key(anchor_key(a));
            // A group target is ringed rather than washed.
            if app.settings.highlight == Some(a) {
                c = c.radius(tk.r(0.5)).border(2.0, tk.ring);
            }
        }
        if self.title.is_some() || !self.actions.is_empty() {
            let mut head = row().w_full().gap(tk.r(0.75)).justify_between().items_end();
            head = head.child(match self.title {
                Some(tt) => text(tt)
                    .font(tk.tsm(600.0))
                    .color(tk.fg)
                    .role("heading")
                    .level(3),
                None => el(),
            });
            if !self.actions.is_empty() {
                head = head.child(row().gap(tk.r(0.5)).center_y().children(self.actions));
            }
            c = c.child(head);
        }
        c = c.child(boxed(tk, self.rows));
        if let Some(f) = self.footer {
            c = c.child(
                col()
                    .w_full()
                    .px(tk.r(0.25))
                    .child(f.font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
            );
        }
        c
    }
}

/// `divide-y overflow-hidden rounded-lg border bg-background`.
pub fn boxed(tk: &Tk, rows: Vec<El>) -> El {
    let mut b = col()
        .w_full()
        .radius(tk.r(0.5))
        .fill(tk.background)
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(tk.r(0.5));
    for (i, r) in rows.into_iter().enumerate() {
        b = b.child(if i == 0 {
            r
        } else {
            r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
        });
    }
    b
}

/// `SettingsRow`: `px-4 py-3`, label (`text-sm font-medium leading-none`) over its description
/// (`text-sm text-muted-foreground`, `space-y-0.5`), controls right (`gap-2`, `gap-6` between).
pub fn srow(
    app: &App,
    tk: &Tk,
    label: impl Into<String>,
    description: Option<El>,
    controls: Vec<El>,
) -> El {
    srow_a(app, tk, label, description, controls, None)
}

pub fn srow_a(
    app: &App,
    tk: &Tk,
    label: impl Into<String>,
    description: Option<El>,
    controls: Vec<El>,
    anchor: Option<&'static str>,
) -> El {
    let label = label.into();
    // `text-sm` on the Label wins over its `leading-none` (tailwind-merge), so the name takes
    // the 1.25 rem line.
    let mut left = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(text(label).font(tk.tsm(500.0)).color(tk.fg).wrap());
    if let Some(d) = description {
        left = left.child(d);
    }
    let mut r = row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(1.5))
        .center_y()
        .child(left);
    if !controls.is_empty() {
        r = r.child(row().gap(tk.r(0.5)).center_y().children(controls));
    }
    if let Some(a) = anchor {
        r = r.key(anchor_key(a));
        if app.settings.highlight == Some(a) {
            r = r.fill(tk.accent);
        }
    }
    r
}

/// A description line (`text-sm text-muted-foreground`).
pub fn desc(tk: &Tk, s: impl Into<String>) -> Option<El> {
    Some(muted(tk, s))
}

/// shadcn `Button` at `size="sm"` (outline, default, ghost) or destructive.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Outline,
    Primary,
    Ghost,
    Destructive,
}

pub fn btn(tk: &Tk, key: impl Into<String>, label: &str, kind: Btn, enabled: bool) -> El {
    btn_sized(tk, key, label, kind, enabled, Size::Sm)
}

/// The dialog footer buttons (`size="default"`, h-9).
pub fn btn_lg(tk: &Tk, key: impl Into<String>, label: &str, kind: Btn, enabled: bool) -> El {
    btn_sized(tk, key, label, kind, enabled, Size::Default)
}

fn btn_sized(
    tk: &Tk,
    key: impl Into<String>,
    label: &str,
    kind: Btn,
    enabled: bool,
    size: Size,
) -> El {
    let (h, px) = match size {
        Size::Default => (tk.r(2.25), tk.r(1.0)),
        Size::Sm => (tk.r(2.0), tk.r(0.75)),
        Size::Xs => (tk.r(1.75), tk.r(0.5)),
    };
    let base = row().key(key.into()).h(h).px(px).gap(tk.r(0.375)).center();
    let (b, ink) = match kind {
        Btn::Outline => (button_surface(tk, base, Variant::Outline), tk.fg),
        Btn::Primary => (button_surface(tk, base, Variant::Default), tk.primary_fg),
        Btn::Ghost => (button_surface(tk, base, Variant::Ghost), tk.fg),
        Btn::Destructive => {
            let fill = if tk.dark {
                tk.destructive.alpha(0.6)
            } else {
                tk.destructive
            };
            (
                base.radius(tk.r(0.375)).fill(fill).hover(fill.alpha(0.9)),
                Color::WHITE,
            )
        }
    };
    let mut b = b
        .role("button")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled);
    if !enabled {
        b = b.opacity(0.5);
    }
    b.child(text(label.to_owned()).font(tk.tsm(500.0)).color(ink))
}

/// A press handler only while enabled.
pub trait PressIf {
    fn press_if(self, enabled: bool, f: impl Fn(&mut App) + 'static) -> Self;
}
impl PressIf for El {
    fn press_if(self, enabled: bool, f: impl Fn(&mut App) + 'static) -> Self {
        if enabled { self.on_press(f) } else { self }
    }
}

/// shadcn `Switch`, pressable; `enabled` false dims and disables it.
pub fn switch(
    tk: &Tk,
    key: impl Into<String>,
    on: bool,
    label: &str,
    enabled: bool,
    toggle: impl Fn(&mut App, bool) + 'static,
) -> El {
    let key = key.into();
    let s = crate::ui::switch(tk, &key, on, label)
        .disabled(!enabled)
        .focusable(enabled);
    if enabled {
        s.on_press(move |a: &mut App| toggle(a, !on))
    } else {
        s.opacity(0.5)
    }
}

/// `SettingsSelect`: a `w-48 h-8` trigger whose option list mounts in `layer` while open.
#[allow(clippy::too_many_arguments)]
pub fn select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    label: &str,
    current: &str,
    options: Vec<Opt>,
    enabled: bool,
    placeholder: Option<&str>,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let shown = options
        .iter()
        .find(|o| o.value == current)
        .map(|o| o.label.clone());
    let open = app.settings.select.as_deref() == Some(key);
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, tk.accent)
    };
    let value_text = shown
        .clone()
        .unwrap_or_else(|| placeholder.unwrap_or("").to_owned());
    let k = key.to_owned();
    let mut trigger = row()
        .key(key.to_owned())
        .w(tk.r(12.0))
        .h(tk.r(2.0))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, tk.input)
        .border_layout()
        .fill(fill)
        .role("combobox")
        .label(label.to_owned())
        .a11y_value(value_text.clone())
        .expanded(open)
        .focusable(enabled)
        .disabled(!enabled)
        .child(
            text(value_text)
                .font(tk.tsm(400.0))
                .color(if shown.is_some() { tk.fg } else { tk.muted_fg })
                .truncate(),
        )
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    if enabled {
        trigger = crate::ui::popup::trigger(
            trigger.hover(hover),
            crate::ui::popup::Kind::Listbox,
            open,
            move |a: &mut App| {
                a.settings.select = if a.settings.select.as_deref() == Some(k.as_str()) {
                    None
                } else {
                    Some(k.clone())
                };
            },
        );
    } else {
        trigger = trigger.opacity(0.5);
    }
    if open && enabled {
        select_menu(tk, layer, key, options, current, choose);
    }
    trigger
}

fn select_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    options: Vec<Opt>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    // Radix `position="item-aligned"`: the list opens over its trigger with the chosen option
    // where the trigger's value was.
    let idx = options.iter().position(|o| o.value == current).unwrap_or(0) as f32;
    let gap = -tk.r(2.0) - tk.r(0.25) - 1.0 - idx * tk.r(2.0);
    let mut p = crate::ui::popup::Popup::listbox(anchor, |a: &mut App| a.settings.select = None);
    for o in options {
        let checked = o.value == current;
        let pick = choose.clone();
        let val = o.value.clone();
        let mut item = row()
            .w_full()
            .min_h(tk.r(2.0))
            .pl(tk.r(0.5))
            .pr(tk.r(2.0))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.25))
            .role("option")
            .checked(checked)
            .label(o.label.clone())
            .child(
                text(o.label.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .grow(1.0),
            )
            .child(
                row()
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h(tk.r(2.0))
                    .justify_end()
                    .pr(tk.r(0.5))
                    .center_y()
                    .child(if checked {
                        ic(&I::CHECK, tk.r(1.0), tk.fg)
                    } else {
                        el()
                    }),
            );
        if let Some(tt) = o.title {
            item = item.help(tt);
        }
        if o.disabled {
            item = item.opacity(0.5);
        } else {
            item = item
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press(move |a: &mut App| {
                    a.settings.select = None;
                    pick(a, &val);
                });
        }
        p.item(
            item,
            format!("{anchor}-opt-{}", o.value),
            o.label,
            !o.disabled,
            checked,
        );
    }
    let list = col()
        .min_w(tk.r(12.0))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
    let p = p.max_h(tk.r(22.0));
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        gap,
    );
}

/// The ghost copy button (`size="icon-sm"`, Copy → Check for 1.5 s).
pub fn copy_button(app: &App, tk: &Tk, key: &str, value: String, label: String) -> El {
    let copied = app
        .settings
        .copied
        .as_ref()
        .is_some_and(|(k, until)| k == key && *until > app.clock);
    let k = key.to_owned();
    let glyph = if copied {
        ic(
            &I::CHECK,
            tk.r(0.875),
            crate::settings::controls::emerald(tk),
        )
    } else {
        ic(&I::COPY, tk.r(0.875), tk.fg)
    };
    button_surface(
        tk,
        row()
            .key(key.to_owned())
            .size(tk.r(2.0), tk.r(2.0))
            .center(),
        Variant::Ghost,
    )
    .role("button")
    .label(label)
    .focusable(true)
    .on_press_tx(move |a: &mut App, tx: &mut rune::Tx| {
        // "Clear clipboard after copy" (30 s) applies to copied values, as `copyTextWithPolicy`.
        super::effects::copy(a, tx, value.clone());
        a.settings.copied = Some((k.clone(), a.clock + 1.5));
        tx.wake_after(std::time::Duration::from_millis(1550));
    })
    .child(glyph)
}

/// `SettingsCopyRow`: the value in mono under its name, a copy button right.
#[allow(clippy::too_many_arguments)]
pub fn copy_row(
    app: &App,
    tk: &Tk,
    key: &str,
    label: &str,
    value: Option<String>,
    copy_label: String,
    placeholder: &str,
    anchor: Option<&'static str>,
) -> El {
    let shown = value.clone().unwrap_or_else(|| placeholder.to_owned());
    let d = text(shown)
        .font(tk.txs(400.0).mono())
        .color(tk.muted_fg)
        .wrap_anywhere();
    let controls = value
        .map(|v| vec![copy_button(app, tk, key, v, copy_label)])
        .unwrap_or_default();
    srow_a(app, tk, label, Some(d), controls, anchor)
}

/// `CommandLine`: a mono command in a muted box with its copy button.
pub fn command_line(app: &App, tk: &Tk, key: &str, command: &str) -> El {
    row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.4))
        .px(tk.r(0.75))
        .py(tk.r(0.375))
        .child(
            text(command.to_owned())
                .font(tk.txs(400.0).mono())
                .color(tk.fg)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(copy_button(
            app,
            tk,
            key,
            command.to_owned(),
            format!("Copy \"{command}\""),
        ))
}

/// `SettingsDisclosure`: a folded group box (`<details>`), chevron rotating when open.
pub fn disclosure(
    app: &App,
    tk: &Tk,
    key: &str,
    title: El,
    default_open: bool,
    rows: Vec<El>,
    anchor: Option<&'static str>,
) -> El {
    let open = crate::settings::is_open(app, key, default_open);
    let k = key.to_owned();
    let summary = row()
        .key(format!("{key}-summary"))
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .hover(tk.muted.alpha(0.4))
        .role("button")
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| crate::settings::toggle_open(a, &k, default_open))
        .child(ic(
            if open {
                &I::CHEVRON_DOWN
            } else {
                &I::CHEVRON_RIGHT
            },
            tk.r(1.0),
            tk.muted_fg,
        ))
        .child(title.grow(1.0).min_w(0.0));
    let mut b = col()
        .key(key.to_owned())
        .w_full()
        .radius(tk.r(0.5))
        .fill(tk.background)
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(tk.r(0.5))
        .child(summary);
    if let Some(a) = anchor
        && app.settings.highlight == Some(a)
    {
        b = b.fill(tk.accent);
    }
    if open {
        for r in rows {
            b = b.child(
                r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                    .border_layout(),
            );
        }
    }
    b
}

/// A page's rare or technical settings, folded behind one "Advanced" toggle (collapsed by
/// default; an owner request, not in the web). `key` keeps the open state in
/// `app.settings.open`; a search result landing inside opens it (`search::reveal_key`). The
/// children are whole groups, laid out as the page lays out its own.
pub fn advanced(app: &App, tk: &Tk, key: &'static str, children: Vec<El>) -> El {
    let open = crate::settings::is_open(app, key, false);
    let label = t(if open {
        "settings:advanced.hide"
    } else {
        "settings:advanced.show"
    });
    let head = row()
        .key(format!("{key}-toggle"))
        .gap(tk.r(0.375))
        .px(tk.r(0.375))
        .py(tk.r(0.25))
        .ml(-tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .hover(tk.muted.alpha(0.6))
        .role("button")
        .label(label)
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| crate::settings::toggle_open(a, key, false))
        .child(
            ic(
                if open {
                    &I::CHEVRON_DOWN
                } else {
                    &I::CHEVRON_RIGHT
                },
                tk.r(1.0),
                tk.muted_fg,
            )
            .a11y_hidden(),
        )
        .child(
            text(t("settings:advanced.title"))
                .font(tk.tsm(500.0))
                .color(tk.muted_fg),
        );
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .gap(tk.r(1.5))
        .child(row().w_full().child(head));
    if open {
        c = c.children(children);
    }
    c
}

/// A text-sm medium title for a disclosure summary.
pub fn summary_title(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into()).font(tk.tsm(500.0)).color(tk.fg)
}

/// A shadcn `Input` (`h-9`, or `h-8` where the web says so).
#[allow(clippy::too_many_arguments)]
pub fn input<F>(
    tk: &Tk,
    edit: &TextEdit,
    key: &str,
    label: &str,
    placeholder: &str,
    secure: bool,
    enabled: bool,
    height: f32,
    mono: bool,
    on_edit: F,
) -> El
where
    F: for<'a> Fn(&'a mut App) -> &'a mut TextEdit + 'static,
{
    let mut k = form_kit(tk);
    if mono {
        k.ty.ui = tk.txs(400.0).mono();
    }
    let style = if mono {
        tk.txs(400.0).mono()
    } else {
        tk.tsm(400.0)
    };
    let mut i = TextInput::new(edit)
        .label(label.to_owned())
        .placeholder(placeholder.to_owned())
        .height(height)
        .width(None)
        .text_style(style)
        .disabled(!enabled)
        .on_edit(on_edit);
    if secure {
        i = i.secure();
    }
    let e = field(tk, i.build(&k).key(key.to_owned()));
    if enabled { e } else { e.opacity(0.5) }
}

/// A shadcn `Label` above a field (`text-sm font-medium leading-none`, `space-y-2`).
pub fn labelled(tk: &Tk, label: &str, field: El) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0).lh(tk.r(0.875)))
                .color(tk.fg),
        )
        .child(field)
}

/// A shadcn checkbox (`size-4 rounded-[4px] border`), checked = primary fill with a check.
pub fn checkbox(
    tk: &Tk,
    key: &str,
    on: bool,
    label: &str,
    toggle: impl Fn(&mut App) + 'static,
) -> El {
    let b = row()
        .key(key.to_owned())
        .size(tk.r(1.0), tk.r(1.0))
        .center()
        .radius(4.0)
        .border(1.0, if on { tk.primary } else { tk.input })
        .fill(if on {
            tk.primary
        } else if tk.dark {
            tk.input.alpha(tk.input.a * 0.3)
        } else {
            Color::TRANSPARENT
        })
        .role("checkbox")
        .checked(on)
        .label(label.to_owned())
        .focusable(true)
        .on_press(toggle);
    if on {
        b.child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b
    }
}

/// `DialogContent` (glass): the faint backdrop, `kb-glass-dialog` surface, top-anchored at
/// `max(1rem, 8vh)`, the close button top right. `pad` false drops the card padding for dialogs
/// that lay out their own header/footer bands.
#[allow(clippy::too_many_arguments)]
pub fn dialog(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    width: f32,
    content: El,
    close: fn(&mut App),
    pad: bool,
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
    let close_btn = crate::txdetail::widgets::icon_button(
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
    let mut surface = col()
        .key(key.to_owned())
        .role("dialog")
        .w(width)
        .max_h(win_h - top - tk.r(1.0) - 40.0);
    if pad {
        surface = surface.padding(tk.r(1.25)).gap(tk.r(1.0));
    }
    let surface = crate::shell::glass_dialog(
        tk,
        surface.modal(true, close).child(content).child(close_btn),
    )
    .clip_rounded(tk.r(1.0));
    let win_w = if app.shell.window_w > 0.0 {
        app.shell.window_w
    } else {
        1440.0
    };
    layer.push(crate::shell::centered(surface, win_w, width, top));
}

/// `DialogTitle` (`text-lg leading-none font-semibold`).
pub fn dialog_title(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into())
        .font(tk.tlg(600.0).lh(tk.r(1.125)))
        .color(tk.fg)
        .role("heading")
        .level(2)
}

/// A destructive error line.
pub fn error_line(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into())
        .font(tk.tsm(400.0))
        .color(tk.destructive)
        .wrap()
        .role("alert")
}

/// A small inline icon + text note (`flex items-start gap-1.5 text-xs`).
pub fn note(tk: &Tk, glyph: &'static Icon, s: impl Into<String>, color: Color) -> El {
    row()
        .w_full()
        .gap(tk.r(0.375))
        .items_start()
        .child(ic(glyph, tk.r(0.875), color).mt(1.0))
        .child(
            text(s.into())
                .font(tk.txs(400.0))
                .color(color)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}
