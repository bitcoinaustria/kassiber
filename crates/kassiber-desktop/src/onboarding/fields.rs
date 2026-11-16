//! Setup's building blocks (`fields.tsx`, `frame.tsx`, `network.tsx`, `stepper.tsx`): labelled
//! text and select fields, option cards, check rows, settings-style groups and rows, the step
//! footer and the step track.

use rune::{Color, El, LoopSpec, TextEdit, TextStyle, col, el, row, text};
use rune_kit::Placement;
use rune_kit::TextInput;

use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant, button_surface, button_with, field, form_kit};
use crate::ui::ic;

use super::model::{Network, Reach};

/// The web's own neutrals (`--color-ink*`, `--color-line*`, `--color-paper-2`).
pub struct Ink {
    pub ink: Color,
    pub ink2: Color,
    pub ink3: Color,
    pub line: Color,
    pub line2: Color,
    pub paper2: Color,
}

pub fn ink(tk: &Tk) -> Ink {
    let h = |s: &str| rune::color::hex(s).expect("hex");
    if tk.dark {
        Ink {
            ink: h("#f4f4f5"),
            ink2: h("#c7c7c9"),
            ink3: h("#a1a1aa"),
            line: h("#2a2a2d"),
            line2: h("#424248"),
            paper2: h("#171717"),
        }
    } else {
        Ink {
            ink: h("#18181b"),
            ink2: h("#3f3f46"),
            ink3: h("#71717a"),
            line: h("#e4e4e7"),
            line2: h("#d4d4d8"),
            paper2: h("#fafafa"),
        }
    }
}

pub type EditFn = fn(&mut App) -> &mut TextEdit;

/// A shadcn `Label` (`text-sm font-medium leading-none`).
pub fn label(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tsm(500.0).lh(tk.r(0.875)))
        .color(tk.fg)
}

/// The hint line (`font-mono text-2xs uppercase tracking-[0.08em] text-[var(--kb-accent)]`).
fn hint_line(tk: &Tk, key: &str, s: &str) -> El {
    let style = tk.t2xs(400.0).mono().tracking(tk.r(0.694) * 0.08);
    text(s.to_uppercase())
        .key(format!("{key}-hint"))
        .font(style)
        .color(tk.kb_accent_text)
        .wrap()
        .role("alert")
}

fn description(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.txs(400.0).lh(tk.r(1.25)))
        .color(ink(tk).ink2)
        .wrap()
}

/// A `TextField`: the label, the input (`h-9 rounded-md border-line`), then the hint once the
/// field was touched or has content, else the description. `revealable` adds the eye button.
pub struct Text<'a> {
    pub key: &'static str,
    pub label: String,
    pub edit: &'a TextEdit,
    pub placeholder: String,
    pub secure: bool,
    pub revealed: bool,
    pub disabled: bool,
    pub hint: Option<String>,
    pub description: Option<String>,
    pub touched: bool,
    pub on_edit: EditFn,
    pub on_change: Option<fn(&mut App)>,
}

impl<'a> Text<'a> {
    pub fn new(
        key: &'static str,
        label: String,
        edit: &'a TextEdit,
        placeholder: String,
        on_edit: EditFn,
    ) -> Text<'a> {
        Text {
            key,
            label,
            edit,
            placeholder,
            secure: false,
            revealed: false,
            disabled: false,
            hint: None,
            description: None,
            touched: false,
            on_edit,
            on_change: None,
        }
    }

    pub fn build(self, tk: &Tk) -> El {
        let show_hint = self.hint.is_some() && (self.touched || !self.edit.text.is_empty());
        let mut k = form_kit(tk);
        k.c.text_faint = tk.muted_fg;
        let mut input = TextInput::new(self.edit)
            .label(self.label.clone())
            .placeholder(self.placeholder.clone())
            .height(tk.r(2.25))
            .width(None)
            .text_style(tk.tsm(400.0))
            .disabled(self.disabled)
            .invalid(show_hint)
            .on_edit(self.on_edit);
        if let Some(f) = self.on_change {
            input = input.on_change(move |a: &mut App, _tx: &mut rune::Tx| f(a));
        }
        if self.secure {
            input = input.secure().revealed(self.revealed);
            let key = self.key;
            let show = t("common:passphraseReveal.show");
            let reveal = row()
                .key(format!("{key}-reveal"))
                .size(tk.r(2.0), tk.r(2.0))
                .center()
                .radius(tk.r(0.375))
                .hover(tk.accent)
                .role("button")
                .label(show)
                .help(t(if self.revealed {
                    "common:passphraseReveal.hide"
                } else {
                    "common:passphraseReveal.show"
                }))
                .checked(self.revealed)
                .focusable(!self.disabled)
                .disabled(self.disabled)
                .on_press(move |a: &mut App| {
                    if !a.onboarding.reveal.remove(key) {
                        a.onboarding.reveal.insert(key);
                    }
                })
                .child(ic(
                    if self.revealed { &I::EYE_OFF } else { &I::EYE },
                    tk.r(1.0),
                    tk.muted_fg,
                ));
            input = input.trailing(row().pr(tk.r(0.125)).child(reveal));
        }
        let mut f = field(tk, input.build(&k).key(self.key.to_owned())).border(
            1.0,
            if show_hint {
                tk.destructive
            } else {
                ink(tk).line
            },
        );
        if self.disabled {
            f = f.opacity(0.5);
        }
        let mut c = col()
            .w_full()
            .min_w(0.0)
            .gap(tk.r(0.5))
            .child(label(tk, &self.label))
            .child(f);
        if show_hint {
            c = c.child(hint_line(
                tk,
                self.key,
                self.hint.as_deref().unwrap_or_default(),
            ));
        } else if let Some(d) = self.description.as_deref() {
            c = c.child(description(tk, d));
        }
        c
    }
}

/// One option of a select.
#[derive(Clone)]
pub struct Opt {
    pub value: String,
    pub label: String,
    /// The tax-country preview badge after the label.
    pub badge: Option<String>,
}

pub fn opt(value: impl Into<String>, label: impl Into<String>) -> Opt {
    Opt {
        value: value.into(),
        label: label.into(),
        badge: None,
    }
}

fn badge(tk: &Tk, s: &str) -> El {
    // `Badge variant="outline"` + `ml-1.5`: rounded-md border px-2 py-0.5 text-xs font-medium.
    row()
        .ml(tk.r(0.375))
        .px(tk.r(0.5))
        .py(tk.r(0.125))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .child(text(s.to_owned()).font(tk.txs(500.0)).color(tk.fg))
}

/// A `SelectField`: the label, a full-width trigger (`h-9 rounded-md border-line`), the list in
/// `layer` while open, then an optional description.
#[allow(clippy::too_many_arguments)]
pub fn select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &'static str,
    label_text: &str,
    current: &str,
    options: Vec<Opt>,
    desc: Option<String>,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let open = app.onboarding.select.as_deref() == Some(key);
    let shown = options.iter().find(|o| o.value == current).cloned();
    let (fill, hover) = if tk.dark {
        (
            tk.input.alpha(tk.input.a * 0.3),
            tk.input.alpha(tk.input.a * 0.5),
        )
    } else {
        (Color::TRANSPARENT, tk.accent)
    };
    let value_text = shown
        .as_ref()
        .map(|o| o.label.clone())
        .unwrap_or_else(|| current.to_owned());
    let mut value = row().gap(tk.r(0.5)).center_y().min_w(0.0).child(
        text(value_text.clone())
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .truncate(),
    );
    if let Some(b) = shown.as_ref().and_then(|o| o.badge.as_deref()) {
        value = value.child(badge(tk, b));
    }
    let trigger = row()
        .key(key.to_owned())
        .w_full()
        .h(tk.r(2.25))
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .radius(tk.r(0.375))
        .border(1.0, ink(tk).line)
        .border_layout()
        .fill(fill)
        .hover(hover)
        .role("combobox")
        .label(label_text.to_owned())
        .a11y_value(value_text)
        .focusable(true)
        .child(value)
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
    let trigger = crate::ui::popup::trigger(
        trigger,
        crate::ui::popup::Kind::Listbox,
        open,
        move |a: &mut App| {
            a.onboarding.select = if a.onboarding.select.as_deref() == Some(key) {
                None
            } else {
                Some(key.to_owned())
            };
        },
    );
    if open {
        menu(tk, layer, key, options, current, choose);
    }
    let mut c = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .child(label(tk, label_text))
        .child(trigger);
    if let Some(d) = desc {
        c = c.child(description(tk, &d));
    }
    c
}

fn menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    options: Vec<Opt>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    // Radix `position="item-aligned"`: the chosen option opens over the trigger's value.
    let idx = options.iter().position(|o| o.value == current).unwrap_or(0) as f32;
    let gap = -tk.r(2.25) - tk.r(0.25) - 1.0 - idx * tk.r(2.0) + tk.r(0.125);
    let mut p = crate::ui::popup::Popup::listbox(anchor, |a: &mut App| a.onboarding.select = None);
    for o in options {
        let checked = o.value == current;
        let pick = choose.clone();
        let val = o.value.clone();
        let mut name = row()
            .gap(0.0)
            .center_y()
            .grow(1.0)
            .child(text(o.label.clone()).font(tk.tsm(400.0)).color(tk.fg));
        if let Some(b) = o.badge.as_deref() {
            name = name.child(badge(tk, b));
        }
        let item = row()
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
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .on_press(move |a: &mut App| {
                a.onboarding.select = None;
                pick(a, &val);
            })
            .child(name)
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
        p.item(
            item,
            format!("{anchor}-opt-{}", o.value),
            o.label,
            true,
            checked,
        );
    }
    let list = col()
        .min_w(tk.r(12.0))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
    let p = p.max_h(tk.r(24.0));
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        gap,
    );
}

/// A shadcn checkbox with its label and description (`CheckRow`).
pub fn check_row(
    tk: &Tk,
    key: &'static str,
    on: bool,
    label_text: &str,
    desc: El,
    toggle: fn(&mut App),
) -> El {
    let i = ink(tk);
    let b = row()
        .key(key.to_owned())
        .size(tk.r(1.0), tk.r(1.0))
        .mt(tk.r(0.125))
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
        .label(label_text.to_owned())
        .focusable(true)
        .on_press(move |a: &mut App| toggle(a));
    let b = if on {
        b.child(ic(&I::CHECK, tk.r(0.875), tk.primary_fg))
    } else {
        b
    };
    row().w_full().gap(tk.r(0.75)).items_start().child(b).child(
        col()
            .grow(1.0)
            .min_w(0.0)
            .gap(tk.r(0.25))
            .child(
                text(label_text.to_owned())
                    .font(tk.tsm(500.0).lh(tk.r(1.25)))
                    .color(i.ink)
                    .wrap()
                    .a11y_hidden()
                    .on_press(move |a: &mut App| toggle(a)),
            )
            .child(desc),
    )
}

pub fn check_desc(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.txs(400.0).lh(tk.r(1.25)))
        .color(ink(tk).ink2)
        .wrap()
}

/// `NetworkStatus`: the reach icon and its label, muted.
pub fn network_status(tk: &Tk, reach: Reach, label_text: &str) -> El {
    let i = ink(tk);
    let glyph = match reach {
        Reach::Internet => &I::EARTH,
        Reach::Device => &I::LAPTOP,
        Reach::None => &I::BAN,
    };
    let c = if reach == Reach::Internet {
        i.ink2
    } else {
        i.ink3
    };
    row()
        .gap(tk.r(0.375))
        .center_y()
        .child(ic(glyph, tk.r(0.875), c))
        .child(text(label_text.to_owned()).font(tk.txs(400.0)).color(c))
}

/// `ChoiceCard`: a radio, an icon, the title with what it reaches, the hosts, the trade-off; the
/// chosen card holds `children`.
pub struct Choice {
    pub key: &'static str,
    pub active: bool,
    pub glyph: Option<&'static rune::Icon>,
    pub title: String,
    pub description: String,
    pub detail: Option<String>,
    pub network: Option<Network>,
    pub warning: bool,
    pub on_press: fn(&mut App),
}

pub fn choice(tk: &Tk, c: Choice, children: Vec<El>) -> El {
    let i = ink(tk);
    let accent = tk.kb_accent;
    let border = if c.active {
        if c.warning {
            accent.alpha(0.7)
        } else {
            tk.fg.alpha(0.4)
        }
    } else {
        tk.border
    };
    let radio_fill = if c.active {
        if c.warning { accent } else { tk.primary }
    } else {
        Color::TRANSPARENT
    };
    let mut radio = row()
        .size(tk.r(1.0), tk.r(1.0))
        .mt(tk.r(0.125))
        .center()
        .radius(tk.r(0.5))
        .a11y_hidden();
    radio = if c.active {
        radio.fill(radio_fill).border(1.0, radio_fill)
    } else {
        radio.border(1.0, i.line2)
    };
    if c.active {
        radio = radio.child(
            el().size(tk.r(0.375), tk.r(0.375))
                .radius(tk.r(0.1875))
                .fill(if tk.dark {
                    rune::color::hex("#101010").expect("hex")
                } else {
                    Color::WHITE
                }),
        );
    }
    let mut head = row()
        .w_full()
        .flex_wrap()
        .gaps(tk.r(0.25), tk.r(0.75))
        .justify_between()
        .center_y()
        .child(text(c.title.clone()).font(tk.tsm(500.0)).color(i.ink));
    if let Some(n) = &c.network {
        head = head.child(network_status(tk, n.reach, &n.label));
    }
    let mut body = col().grow(1.0).min_w(0.0).child(head).child(
        text(c.description.clone())
            .font(tk.txs(400.0).lh(tk.r(1.125)))
            .color(i.ink3)
            .wrap()
            .mt(tk.r(0.25)),
    );
    if let Some(n) = &c.network {
        if !n.hosts.is_empty() {
            let mut hosts = col().mt(tk.r(0.375)).gap(tk.r(0.125));
            for (h, l) in &n.hosts {
                let mut r = row().w_full().gap(tk.r(1.0)).child(
                    text(h.clone())
                        .font(tk.t2xs(400.0).mono().lh(tk.r(1.25)))
                        .color(i.ink2)
                        .w(tk.r(15.0))
                        .wrap(),
                );
                if let Some(l) = l {
                    r = r.child(
                        text(l.clone())
                            .font(tk.t2xs(400.0).lh(tk.r(1.25)))
                            .color(i.ink3),
                    );
                }
                hosts = hosts.child(r);
            }
            body = body.child(hosts);
        } else if let Some(p) = &n.pending {
            body = body.child(
                text(p.clone())
                    .font(tk.txs(400.0).italic())
                    .color(i.ink3)
                    .mt(tk.r(0.375)),
            );
        }
    }
    if let Some(d) = &c.detail {
        body = body.child(
            text(d.clone())
                .font(tk.txs(400.0).lh(tk.r(1.125)))
                .color(if c.active { i.ink2 } else { i.ink3 })
                .wrap()
                .mt(tk.r(0.375)),
        );
    }
    let mut button = row()
        .key(c.key.to_owned())
        .w_full()
        .px(tk.r(0.875))
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .items_start()
        .radius(tk.r(0.75))
        .role("button")
        .label(c.title.clone())
        .checked(c.active)
        .focusable(true)
        .on_press(c.on_press)
        .child(radio);
    if let Some(g) = c.glyph {
        button =
            button.child(ic(g, tk.r(1.0), if c.active { i.ink } else { i.ink3 }).mt(tk.r(0.125)));
    }
    button = button.child(body);
    let mut card = col()
        .key(format!("{}-card", c.key))
        .w_full()
        .radius(tk.r(0.75))
        .border(1.0, border)
        .border_layout()
        .fill(if c.active { tk.card } else { tk.background });
    if !c.active {
        card = card.hover(tk.background);
    }
    card = card.child(button);
    if c.active && !children.is_empty() {
        let pl = if c.glyph.is_some() {
            tk.r(3.75)
        } else {
            tk.r(2.25)
        };
        let mut inner = col()
            .w_full()
            .gap(tk.r(1.0))
            .pl(pl)
            .pr(tk.r(0.875))
            .py(tk.r(1.0))
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
            .border_layout();
        for ch in children {
            inner = inner.child(ch);
        }
        card = card.child(inner);
    }
    card
}

/// `OnboardingStepPage`: the title, an optional lead, then the step's content.
pub fn step_page(tk: &Tk, title: &str, lead: Option<String>, content: El) -> El {
    let i = ink(tk);
    let mut head = col().w_full().child(
        text(title.to_owned())
            .font(
                TextStyle::new(tk.r(1.5), 600.0)
                    .lh(tk.r(2.0))
                    .tracking(-tk.r(1.5) * 0.025),
            )
            .color(i.ink)
            .role("heading")
            .level(1)
            .wrap(),
    );
    if let Some(l) = lead {
        head = head.child(
            text(l)
                .font(tk.tsm(400.0).lh(tk.r(1.421_875)))
                .color(i.ink2)
                .wrap()
                .mt(tk.r(0.625)),
        );
    }
    col().w_full().gap(tk.r(1.5)).child(head).child(content)
}

/// `OnboardingSection`: a dimmed heading over a group.
pub fn section(tk: &Tk, title: &str, rows: Vec<El>) -> El {
    col()
        .w_full()
        .gap(tk.r(0.625))
        .child(
            text(title.to_owned())
                .font(tk.tsm(400.0))
                .color(ink(tk).ink.alpha(0.7))
                .px(tk.r(0.25)),
        )
        .child(group(tk, rows))
}

/// `OnboardingGroup`: rows in one outlined box, divided by hairlines.
pub fn group(tk: &Tk, rows: Vec<El>) -> El {
    let mut g = col()
        .w_full()
        .radius(tk.r(0.75))
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .fill(tk.card.alpha(0.4))
        .clip_rounded(tk.r(0.75));
    for (n, r) in rows.into_iter().enumerate() {
        g = g.child(if n > 0 {
            r.border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.5))
                .border_layout()
        } else {
            r
        });
    }
    g
}

/// `OnboardingRow`: title, description and status left, the control right.
pub fn srow(
    tk: &Tk,
    title: El,
    desc: Option<El>,
    status: Option<El>,
    control: Option<El>,
    disabled: bool,
) -> El {
    let mut left = col().grow(1.0).min_w(0.0).gap(tk.r(0.25)).child(title);
    if let Some(d) = desc {
        left = left.child(d.max_w(tk.r(36.0)));
    }
    if let Some(s) = status {
        left = left.child(s.pt(tk.r(0.125)));
    }
    let mut r = row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .gap(tk.r(2.0))
        .center_y()
        .child(left);
    if let Some(c) = control {
        r = r.child(row().gap(tk.r(0.5)).center_y().justify_end().child(c));
    }
    if disabled { r.opacity(0.6) } else { r }
}

pub fn row_title(tk: &Tk, s: &str) -> El {
    text(s.to_owned())
        .font(tk.tsm(500.0))
        .color(ink(tk).ink)
        .role("heading")
        .level(3)
}

/// `OnboardingStepActions`: the network note left, Back and the primary action right.
pub fn step_actions(
    tk: &Tk,
    back: Option<fn(&mut App)>,
    label_text: Option<String>,
    busy_label: Option<String>,
    busy: bool,
    disabled: bool,
    submit: fn(&mut App),
) -> El {
    let i = ink(tk);
    let mut right = row().gap(tk.r(0.5)).center_y().justify_end();
    if let Some(b) = back {
        let back_label = t("common:actions.back");
        right = right.child(
            button_surface(
                tk,
                row()
                    .key("onb-back")
                    .h(tk.r(2.25))
                    .px(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .center(),
                Variant::Ghost,
            )
            .role("button")
            .label(back_label.clone())
            .focusable(true)
            .on_press(b)
            .child(ic(&I::ARROW_LEFT, tk.r(0.875), i.ink2))
            .child(text(back_label).font(tk.tsm(500.0)).color(i.ink2)),
        );
    }
    let enabled = !disabled && !busy;
    let text_label = if busy {
        busy_label.or(label_text.clone()).unwrap_or_default()
    } else {
        label_text.unwrap_or_else(|| t("common:actions.continue"))
    };
    let mut primary = button_surface(
        tk,
        row()
            .key("onb-continue")
            .h(tk.r(2.25))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center(),
        Variant::Default,
    )
    .role("button")
    .label(text_label.clone())
    .focusable(enabled)
    .disabled(!enabled)
    .default_button();
    if enabled {
        primary = primary.on_press(submit);
    } else {
        primary = primary.opacity(0.5);
    }
    if busy {
        primary = primary
            .child(ic(&I::LOADER_CIRCLE, tk.r(1.0), tk.primary_fg).looping(LoopSpec::rotation(1.0)))
            .child(text(text_label).font(tk.tsm(500.0)).color(tk.primary_fg));
    } else {
        primary = primary
            .child(text(text_label).font(tk.tsm(500.0)).color(tk.primary_fg))
            .child(ic(&I::ARROW_RIGHT, tk.r(0.875), tk.primary_fg));
    }
    right = right.child(primary);
    row()
        .w_full()
        .mt(tk.r(0.5))
        .pt(tk.r(1.25))
        .gap(tk.r(1.5))
        .center_y()
        .justify_between()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .child(
            text(t("onboarding:frame.setupConnects"))
                .font(tk.txs(400.0).lh(tk.r(1.21875)))
                .color(i.ink3)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(right)
}

/// `OnboardingStepper`: a segmented track; done steps jump back, later ones stay locked.
pub fn stepper(
    tk: &Tk,
    labels: &[String],
    summaries: &[Option<String>],
    current: usize,
    narrow: bool,
) -> El {
    let i = ink(tk);
    let mut track = row()
        .w_full()
        .gap(tk.r(0.25))
        .padding(tk.r(0.25))
        .radius(tk.r(0.75))
        .fill(tk.background)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout();
    for (n, label_text) in labels.iter().enumerate() {
        let done = n < current;
        let active = n == current;
        let name = match (&summaries[n], done) {
            (Some(s), true) => format!("{label_text}: {s}"),
            _ => label_text.clone(),
        };
        let (dot_fill, dot_ring, dot_ink) = if done {
            (tk.primary, tk.primary, tk.primary_fg)
        } else if active {
            (tk.primary.alpha(0.1), tk.primary.alpha(0.3), i.ink)
        } else {
            (tk.card, tk.border, i.ink3)
        };
        let mut dot = row()
            .size(tk.r(1.25), tk.r(1.25))
            .center()
            .radius(tk.r(0.625))
            .fill(dot_fill)
            .border(1.0, dot_ring)
            .a11y_hidden();
        dot = if done {
            dot.child(ic(&I::CHECK, tk.r(0.875), dot_ink))
        } else {
            dot.child(text((n + 1).to_string()).font(tk.txs(500.0)).color(dot_ink))
        };
        let mut b = row()
            .key(format!("onb-step-{n}"))
            .w(0.0)
            .grow(1.0)
            .min_w(0.0)
            .px(if narrow { tk.r(0.5) } else { tk.r(0.625) })
            .py(tk.r(0.5))
            .gap(tk.r(0.5))
            .center_y()
            .radius(tk.r(0.5))
            .role("button")
            .label(name)
            .position_in_set(n + 1, labels.len())
            .focusable(done)
            .disabled(!done)
            .child(dot);
        if narrow {
            b = b.justify_center();
        } else {
            b = b.child(
                text(label_text.clone())
                    .font(tk.tsm(500.0))
                    .color(if active { i.ink } else { i.ink3 })
                    .truncate(),
            );
        }
        if active {
            b = b
                .fill(tk.card)
                .border(1.0, tk.border)
                .shadow(1.0, 2.0, Color::BLACK.alpha(0.05));
        }
        if done {
            b = b
                .hover(tk.card)
                .on_press(move |a: &mut App| super::jump(a, n));
        }
        track = track.child(b);
    }
    track
        .role("navigation")
        .label(t("onboarding:stepper.ariaLabel"))
}

/// The error banner (`role="alert"`, destructive tint).
pub fn error_banner(tk: &Tk, msg: &str) -> El {
    row()
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.75))
        .radius(tk.r(0.75))
        .border(1.0, tk.destructive.alpha(0.3))
        .fill(tk.destructive.alpha(0.1))
        .role("alert")
        .label(msg.to_owned())
        .child(
            text(msg.to_owned())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap(),
        )
}

/// A small icon + text note (`flex items-start gap-2 text-xs leading-5 text-ink-3`).
pub fn note(tk: &Tk, glyph: &'static rune::Icon, s: &str) -> El {
    let c = ink(tk).ink3;
    row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .child(ic(glyph, tk.r(0.875), c).mt(tk.r(0.125)))
        .child(
            text(s.to_owned())
                .font(tk.txs(400.0).lh(tk.r(1.25)))
                .color(c)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

/// `Button size="lg"` (h-10, px-6 / px-4 with an icon).
pub fn button_lg(
    tk: &Tk,
    key: &str,
    variant: Variant,
    glyph: Option<&'static rune::Icon>,
    label_text: &str,
    trailing: Option<&'static rune::Icon>,
    enabled: bool,
) -> El {
    let fg = if variant == Variant::Default {
        tk.primary_fg
    } else {
        tk.fg
    };
    let mut b = button_surface(
        tk,
        row()
            .key(key.to_owned())
            .h(tk.r(2.5))
            .px(if glyph.is_some() {
                tk.r(1.0)
            } else {
                tk.r(1.5)
            })
            .gap(tk.r(0.5))
            .center(),
        variant,
    )
    .role("button")
    .label(label_text.to_owned())
    .focusable(enabled)
    .disabled(!enabled);
    if !enabled {
        b = b.opacity(0.5);
    }
    if let Some(g) = glyph {
        b = b.child(ic(g, tk.r(1.0), fg));
    }
    b = b.child(text(label_text.to_owned()).font(tk.tsm(500.0)).color(fg));
    if let Some(g) = trailing {
        b = b.child(ic(g, tk.r(1.0), fg));
    }
    b
}

/// A plain shadcn button (re-export for the views).
pub fn button(
    tk: &Tk,
    key: &str,
    variant: Variant,
    size: Size,
    glyph: Option<&'static rune::Icon>,
    label_text: &str,
    enabled: bool,
) -> El {
    button_with(
        tk,
        key.to_owned(),
        variant,
        size,
        glyph,
        None,
        label_text,
        enabled,
    )
}

/// `t("network.label", …)` for a reach and moment.
pub fn reach_label(reach: Reach, when: &str) -> String {
    tr(
        "onboarding:network.label",
        &[
            (
                "reach",
                &t(&format!("onboarding:network.reach.{}", reach.id())),
            ),
            ("when", &t(&format!("onboarding:network.when.{when}"))),
        ],
    )
}
