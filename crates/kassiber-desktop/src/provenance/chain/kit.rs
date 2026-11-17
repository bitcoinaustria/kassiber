//! The workbench's own controls (workbench.css): `.ca-field`, `.ca-input`, `.ca-select` (a
//! native `<select>`, drawn as a trigger and a listbox), `<details>` disclosures, `Fact`,
//! `EvidenceDetails`, `StructuredValue` and `CodeList`, plus the shadcn buttons the panels use.

use rune::{Color, El, Icon, LineCap, Shape, TextEdit, Tx, canvas, col, el, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::Value;

use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Size, Variant};
use crate::ui::ic;

use super::model::readable;

/// `--ca-*` (workbench.css).
pub mod ca {
    use rune::Color;
    pub fn physical() -> Color {
        rune::color::hex("#168ca6").unwrap()
    }
    pub fn custody() -> Color {
        rune::color::hex("#9674da").unwrap()
    }
    pub fn hypothesis() -> Color {
        rune::color::hex("#c08325").unwrap()
    }
    pub fn owned() -> Color {
        rune::color::hex("#259e79").unwrap()
    }
    pub fn muted() -> Color {
        rune::color::hex("#9297a0").unwrap()
    }
}

/// `text-amber-700 dark:text-amber-300`.
pub fn amber(tk: &Tk) -> Color {
    rune::color::hex(if tk.dark { "#fcd34d" } else { "#b45309" }).unwrap()
}

/// Whether a disclosure or popup keyed `k` is open.
pub fn is_open(app: &App, k: &str) -> bool {
    app.provenance.chain.open.contains(k)
}

pub fn toggle_open(app: &mut App, k: &str) {
    let s = &mut app.provenance.chain.open;
    if !s.remove(k) {
        s.insert(k.to_owned());
    }
}

/// The text of field `k`.
pub fn val(app: &App, k: &str) -> String {
    app.provenance
        .chain
        .edits
        .get(k)
        .map(|e| e.text.clone())
        .unwrap_or_default()
}

/// Sets field `k` (programmatic changes: a loaded case, a picked node).
pub fn set_val(app: &mut App, k: &str, v: &str) {
    app.provenance
        .chain
        .edits
        .insert(k.to_owned(), TextEdit::new(v));
}

/// `.ca-field`: the label over its control (`flex-col gap-[5px] text-[11px] muted`).
pub fn field(tk: &Tk, label: &str, control: El) -> El {
    col()
        .min_w(0.0)
        .gap(5.0)
        .child(
            text(label.to_owned())
                .font(tk.px(11.0, 400.0))
                .color(tk.muted_fg),
        )
        .child(control)
}

/// `.ca-input` (32 px, 1 px border, 6 px radius, 12 px text) over a rune-kit field keyed `k`.
/// `on_change` runs after each edit.
#[allow(clippy::too_many_arguments)]
pub fn input(
    tk: &Tk,
    app: &App,
    k: &str,
    label: &str,
    placeholder: &str,
    mono: bool,
    h: f32,
    enabled: bool,
    on_change: Option<fn(&mut App)>,
) -> El {
    let empty = TextEdit::new("");
    let edit = app.provenance.chain.edits.get(k).unwrap_or(&empty);
    let mut kit = w::form_kit(tk);
    kit.c.bg_base = tk.background;
    kit.c.hover = tk.background;
    let style = if mono {
        tk.px(12.0, 400.0).mono()
    } else {
        tk.px(12.0, 400.0)
    };
    kit.ty.ui = style;
    let key = k.to_owned();
    let mut i = TextInput::new(edit)
        .label(label.to_owned())
        .placeholder(placeholder.to_owned())
        .height(h)
        .width(None)
        .text_style(style)
        .disabled(!enabled)
        .on_edit(move |a: &mut App| {
            a.provenance
                .chain
                .edits
                .entry(key.clone())
                .or_insert_with(|| TextEdit::new(""))
        });
    if let Some(f) = on_change {
        i = i.on_change(move |a: &mut App, _tx: &mut Tx| f(a));
    }
    let e = i
        .build(&kit)
        .key(k.to_owned())
        .radius(6.0)
        .border(1.0, tk.border);
    if enabled { e } else { e.opacity(0.5) }
}

/// One option of a `<select>`.
pub struct Opt {
    pub value: String,
    pub label: String,
    pub disabled: bool,
}

pub fn opt(value: impl Into<String>, label: impl Into<String>) -> Opt {
    Opt {
        value: value.into(),
        label: label.into(),
        disabled: false,
    }
}

/// A native `<select>` (`.ca-select`): the trigger shows the current option; pressing it opens
/// a listbox under it. `choose` gets the picked value.
#[allow(clippy::too_many_arguments)]
pub fn select(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    k: &str,
    label: &str,
    current: &str,
    options: Vec<Opt>,
    enabled: bool,
    h: f32,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let shown = options
        .iter()
        .find(|o| o.value == current)
        .map(|o| o.label.clone())
        .unwrap_or_default();
    let open = app.provenance.chain.popup.as_deref() == Some(k);
    let key = k.to_owned();
    let mut trigger = row()
        .key(k.to_owned())
        .h(h)
        .min_w(0.0)
        .pl(8.0)
        .pr(6.0)
        .gap(6.0)
        .center_y()
        .justify_between()
        .radius(6.0)
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.background)
        .role("combobox")
        .label(label.to_owned())
        .a11y_value(shown.clone())
        .expanded(open)
        .focusable(enabled)
        .disabled(!enabled)
        .child(
            text(shown)
                .font(tk.px(12.0, 400.0))
                .color(tk.fg)
                .truncate()
                .min_w(0.0),
        )
        .child(ic(&I::CHEVRON_DOWN, 14.0, tk.fg));
    if enabled {
        trigger = crate::ui::popup::trigger(
            trigger,
            crate::ui::popup::Kind::Listbox,
            open,
            move |a: &mut App| {
                let p = &mut a.provenance.chain.popup;
                *p = if p.as_deref() == Some(key.as_str()) {
                    None
                } else {
                    Some(key.clone())
                };
            },
        );
    } else {
        trigger = trigger.opacity(0.5);
    }
    if open && enabled {
        menu(tk, layer, k, options, current, choose);
    }
    trigger
}

pub fn close_popup(a: &mut App) {
    a.provenance.chain.popup = None;
}

fn menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    options: Vec<Opt>,
    current: &str,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) {
    let mut p = crate::ui::popup::Popup::listbox(anchor, close_popup).max_h(tk.r(22.0));
    for o in options {
        let checked = o.value == current;
        let pick = choose.clone();
        let v = o.value.clone();
        let mut item = row()
            .w_full()
            .min_h(tk.r(1.75))
            .pl(tk.r(0.5))
            .pr(tk.r(2.0))
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
                    .h_full()
                    .justify_end()
                    .center_y()
                    .pr(tk.r(0.5))
                    .child(if checked {
                        ic(&I::CHECK, tk.r(0.875), tk.fg)
                    } else {
                        el().size(1.0, 1.0)
                    }),
            );
        if o.disabled {
            item = item.opacity(0.5);
        } else {
            item = item
                .hover(tk.accent)
                .focus_highlight(tk.accent)
                .on_press(move |a: &mut App| {
                    close_popup(a);
                    pick(a, &v);
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
        .min_w(tk.r(10.0))
        .padding(tk.r(0.25))
        .label(anchor.to_owned());
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, col().min_h(0.0).child(l)),
        Placement::BottomStart,
        4.0,
    );
}

/// A `<details>` disclosure marker (the browser's ▸ / ▾ triangle).
pub fn marker(open: bool, color: Color, size: f32) -> El {
    let s = size;
    let pts = if open {
        vec![[0.0, s * 0.2], [s, s * 0.2], [s * 0.5, s * 0.9]]
    } else {
        vec![[s * 0.2, 0.0], [s * 0.9, s * 0.5], [s * 0.2, s]]
    };
    canvas(Shape::new().fill(pts, color))
        .size(s, s)
        .a11y_hidden()
}

/// A `<summary>` row keyed `k` (pressing toggles it): the marker, then `content`.
pub fn summary(tk: &Tk, app: &App, k: &str, content: El, pad: [f32; 2]) -> El {
    let open = is_open(app, k);
    let key = k.to_owned();
    row()
        .key(format!("{k}-summary"))
        .w_full()
        .px(pad[0])
        .py(pad[1])
        .gap(6.0)
        .center_y()
        .role("button")
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| toggle_open(a, &key))
        .child(marker(open, tk.fg, 8.0))
        .child(content.min_w(0.0))
}

/// `Fact`: nothing for an empty value; else the `.ca-detail-key` label over the mono value.
pub fn fact(tk: &Tk, label: &str, value: &Value) -> Option<El> {
    match value {
        Value::Null => return None,
        Value::String(s) if s.is_empty() => return None,
        Value::Array(a) if a.is_empty() => return None,
        _ => {}
    }
    Some(fact_text(tk, label, readable(value)))
}

/// A `Fact` with a value already formatted.
pub fn fact_text(tk: &Tk, label: &str, value: String) -> El {
    col()
        .min_w(0.0)
        .gap(4.0)
        .child(detail_key(tk, label))
        .child(
            text(value)
                .font(tk.txs(400.0).mono().lh(tk.r(1.21875)))
                .color(tk.fg)
                .wrap_anywhere()
                .selectable(true),
        )
}

/// `.ca-detail-key` (10 px uppercase, 0.06 em tracking, muted).
pub fn detail_key(tk: &Tk, label: &str) -> El {
    text(label.to_uppercase())
        .font(tk.px(10.0, 400.0).tracking(0.6))
        .color(tk.muted_fg)
}

/// `EvidenceDetails`: a `.ca-evidence` disclosure of the value's JSON.
pub fn evidence(tk: &Tk, app: &App, k: &str, value: &Value, label: Option<String>) -> El {
    let open = is_open(app, k);
    let label = label.unwrap_or_else(|| t("chainAnalysis:showEvidence"));
    let mut c = col()
        .w_full()
        .min_w(0.0)
        .mt(10.0)
        .padding(10.0)
        .radius(6.0)
        .fill(tk.muted.alpha(0.45))
        .child(summary_plain(
            tk,
            app,
            k,
            text(label).font(tk.px(11.0, 400.0)).color(tk.muted_fg),
        ));
    if open {
        c = c.child(
            col()
                .mt(8.0)
                .max_h(tk.r(18.0))
                .child(
                    text(readable(value))
                        .font(tk.px(11.0, 400.0).mono())
                        .color(tk.fg)
                        .wrap_anywhere()
                        .selectable(true),
                )
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }),
        );
    }
    c
}

/// A `<summary>` without padding, its marker in the muted ink.
pub fn summary_plain(tk: &Tk, app: &App, k: &str, content: El) -> El {
    let open = is_open(app, k);
    let key = k.to_owned();
    row()
        .key(format!("{k}-summary"))
        .gap(6.0)
        .center_y()
        .role("button")
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| toggle_open(a, &key))
        .child(marker(open, tk.muted_fg, 7.0))
        .child(content)
}

/// `StructuredValue`: nested objects as key/value lists, arrays capped at 32 rows.
pub fn structured(tk: &Tk, value: &Value, depth: usize) -> El {
    let small = tk.txs(400.0);
    match value {
        Value::Null => text(t("chainAnalysis:workbench.unknown"))
            .font(small)
            .color(tk.muted_fg),
        Value::Bool(b) => text(t(if *b {
            "chainAnalysis:workbench.yes"
        } else {
            "chainAnalysis:workbench.no"
        }))
        .font(small)
        .color(tk.fg),
        Value::String(s) => text(s.clone())
            .font(small.mono())
            .color(tk.fg)
            .wrap_anywhere(),
        Value::Number(n) => text(n.to_string()).font(small.mono()).color(tk.fg),
        _ if depth > 3 => text(t("chainAnalysis:workbench.nested"))
            .font(small)
            .color(tk.fg),
        Value::Array(a) => {
            let mut c = col().gap(4.0).min_w(0.0);
            if a.is_empty() {
                c = c.child(text("—").font(small).color(tk.fg));
            }
            for item in a.iter().take(32) {
                c = c.child(structured(tk, item, depth + 1));
            }
            if a.len() > 32 {
                c = c.child(
                    text(tr(
                        "chainAnalysis:workbench.firstRows",
                        &[("count", "32"), ("total", &a.len().to_string())],
                    ))
                    .font(small)
                    .color(tk.fg),
                );
            }
            c
        }
        Value::Object(m) => {
            let mut c = col().gap(8.0).min_w(0.0);
            for (k, v) in m {
                c = c.child(
                    row()
                        .gap(12.0)
                        .min_w(0.0)
                        .flex_wrap()
                        .child(text(k.replace('_', " ")).font(small).color(tk.muted_fg))
                        .child(structured(tk, v, depth + 1)),
                );
            }
            c
        }
    }
}

/// `CodeList`: a titled list of codes, nothing when empty.
pub fn code_list(tk: &Tk, title: &str, values: Option<&Value>) -> Option<El> {
    let a = values?.as_array().filter(|a| !a.is_empty())?;
    let mut c = col().gap(4.0).child(
        text(title.to_owned())
            .font(tk.txs(500.0))
            .color(tk.fg)
            .mb(4.0),
    );
    for v in a {
        c = c.child(
            text(readable(v))
                .font(tk.px(11.0, 400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    Some(c)
}

/// shadcn `Button size="sm"` (enabled or dimmed), `glyph` at 0.875 rem.
pub fn btn(
    tk: &Tk,
    k: impl Into<String>,
    variant: Variant,
    glyph: Option<&'static Icon>,
    label: &str,
    enabled: bool,
) -> El {
    w::button_with(
        tk,
        k,
        variant,
        Size::Sm,
        glyph,
        Some(tk.r(0.875)),
        label,
        enabled,
    )
}

/// `Button variant="destructive" size="sm"`.
pub fn btn_destructive(tk: &Tk, k: impl Into<String>, label: &str, enabled: bool) -> El {
    let fill = if tk.dark {
        tk.destructive.alpha(0.6)
    } else {
        tk.destructive
    };
    let mut b = row()
        .key(k.into())
        .h(tk.r(2.0))
        .px(tk.r(0.75))
        .center()
        .radius(tk.r(0.375))
        .fill(fill)
        .hover(fill.alpha(0.9))
        .role("button")
        .label(label.to_owned())
        .focusable(enabled)
        .disabled(!enabled)
        .child(
            text(label.to_owned())
                .font(tk.tsm(500.0))
                .color(Color::WHITE),
        );
    if !enabled {
        b = b.opacity(0.5);
    }
    b
}

/// An underlined link (`underline underline-offset-4`).
pub fn link(
    tk: &Tk,
    k: impl Into<String>,
    label: &str,
    style: rune::TextStyle,
    to: &'static str,
) -> El {
    underlined(label, style, tk.fg)
        .key(k.into())
        .role("link")
        .label(label.to_owned())
        .focusable(true)
        .on_press(move |a: &mut App| crate::shell::go(a, to))
}

/// `rounded-md border` (an inset card) with its padding.
pub fn bordered(tk: &Tk, pad: f32) -> El {
    col()
        .w_full()
        .min_w(0.0)
        .padding(pad)
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
}

/// `role="alert"` text in the destructive ink.
pub fn alert_text(tk: &Tk, s: String, style: rune::TextStyle) -> El {
    text(s)
        .font(style)
        .color(tk.destructive)
        .wrap()
        .role("alert")
}

/// A dashed horizontal stroke `w` long (the legend swatches).
pub fn swatch(color: Color, dashed: bool, w: f32) -> El {
    let mut s = Shape::new();
    if dashed {
        let mut x = 0.0;
        while x < w {
            s = s.line([x, 1.0], [(x + 3.0).min(w), 1.0], 2.0, LineCap::Butt, color);
            x += 5.0;
        }
    } else {
        s = s.line([0.0, 1.0], [w, 1.0], 2.0, LineCap::Butt, color);
    }
    canvas(s).size(w, 2.0).a11y_hidden()
}

/// Text underlined in its own ink (`underline`).
pub fn underlined(s: &str, style: rune::TextStyle, color: Color) -> El {
    let n = s.len();
    rune::rich(s.to_owned(), vec![rune::Span::new(0, n).underline(color)])
        .font(style)
        .color(color)
}
