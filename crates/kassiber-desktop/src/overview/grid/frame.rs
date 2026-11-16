//! The one tile every Overview widget draws in, in the manner of a phone's home-screen widget:
//! a borderless, softly lifted surface with a large continuous corner, one padding, and the
//! widget's name as a small caption (glyph + name) above its content. A widget fills the
//! [`Slot`] it is given and never grows the tile. Empty and loading contents look the same in
//! every widget.

use rune::{Color, El, Icon, LineCap, Shadow, Shape, TextStyle, canvas, col, el, row, text};

use super::model::Size;
use crate::theme::Tk;
use crate::ui::ic;

/// The content area a widget fills, under the caption.
#[derive(Clone, Copy)]
pub struct Slot {
    pub size: Size,
    pub w: f32,
    pub h: f32,
    /// A gallery preview: its keys carry a prefix so they never collide with the placed widget's.
    pub pv: bool,
}

impl Slot {
    pub fn key(&self, k: impl AsRef<str>) -> String {
        if self.pv {
            format!("pv-{}", k.as_ref())
        } else {
            k.as_ref().to_owned()
        }
    }

    /// Wide enough for two columns of content (Medium and up).
    pub fn wide(&self) -> bool {
        self.size != Size::Small
    }

    /// Two rows tall (Large and Extra large).
    pub fn tall(&self) -> bool {
        matches!(self.size, Size::Large | Size::XLarge)
    }
}

/// The tile's inner padding at the sides: the app's card padding (`Tk::card_pad`).
pub fn pad(tk: &Tk) -> f32 {
    tk.card_pad()
}

/// The tile's inner padding above the caption and below the content: the card padding too, so
/// a tile reads like every other card.
pub fn pad_top(tk: &Tk) -> f32 {
    tk.card_pad()
}
pub fn pad_bottom(tk: &Tk) -> f32 {
    tk.card_pad()
}

/// Everything a tile adds around its content: the paddings, the caption and its gap.
pub fn chrome_h(tk: &Tk) -> f32 {
    pad_top(tk) + caption_h(tk) + gap(tk) + pad_bottom(tk)
}

/// The tile's corner radius (a continuous curve).
pub fn radius(tk: &Tk) -> f32 {
    tk.r(1.375)
}

pub fn caption_h(tk: &Tk) -> f32 {
    tk.r(1.25)
}

/// Space between the caption and the content.
pub fn gap(tk: &Tk) -> f32 {
    tk.r(0.375)
}

/// The slot of a `w` × `h` tile.
pub fn slot(tk: &Tk, size: Size, w: f32, h: f32, pv: bool) -> Slot {
    Slot {
        size,
        w: (w - 2.0 * pad(tk)).max(0.0),
        h: (h - chrome_h(tk)).max(0.0),
        pv,
    }
}

/// The tile surface, `w` × `h`: no border, the card tone, two soft shadows.
pub fn shell(tk: &Tk, key: String, label: String, w: f32, h: f32) -> El {
    let (a, b) = if tk.dark { (0.30, 0.22) } else { (0.06, 0.07) };
    col()
        .key(key)
        .size(w, h)
        .radius(radius(tk))
        .continuous_corners()
        .clip_rounded(radius(tk))
        .fill(tk.card)
        .box_shadow(Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(a),
            ..Shadow::default()
        })
        .box_shadow(Shadow {
            y: 8.0,
            blur: 24.0,
            spread: -4.0,
            color: Color::BLACK.alpha(b),
            ..Shadow::default()
        })
        .role("region")
        .label(label)
        .px(pad(tk))
        .pt(pad_top(tk))
        .pb(pad_bottom(tk))
        .gap(gap(tk))
}

/// The caption: the widget's glyph and name, small and muted.
pub fn caption(tk: &Tk, glyph: &'static Icon, title: &str) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .h(caption_h(tk))
        .gap(tk.r(0.375))
        .center_y()
        .child(ic(glyph, tk.r(0.875), tk.muted_fg))
        .child(
            text(title.to_owned())
                .font(tk.txs(600.0))
                .color(tk.muted_fg)
                .truncate()
                .min_w(0.0)
                .role("heading"),
        )
}

/// The glanceable figure of a tile: large, tabular, tight.
pub fn figure(tk: &Tk, rem: f32) -> TextStyle {
    TextStyle::new(tk.r(rem), 600.0)
        .lh(tk.r(rem) * 1.15)
        .features("'tnum' 1")
        .tracking(-0.4)
}

/// A small label over or under a figure.
pub fn label(tk: &Tk) -> TextStyle {
    tk.txs(500.0)
}

/// The empty state every widget shows: one muted line, centred in the slot.
pub fn empty(tk: &Tk, key: impl Into<String>, message: String) -> El {
    col()
        .key(key.into())
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .center()
        .px(tk.r(0.5))
        .child(
            text(message)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .align(rune::Align::Center)
                .wrap(),
        )
}

/// The loading state every widget shows: a figure-sized bar and a few lines, `h` tall.
pub fn skeleton(tk: &Tk, h: f32) -> El {
    let bar = |w: f32, h: f32| {
        row()
            .w(w)
            .h(h)
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.7))
    };
    let lines = (((h - tk.r(3.0)) / tk.r(1.75)).floor().max(0.0) as usize).min(8);
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(tk.r(0.75))
        .justify_end()
        .busy(true)
        .label(crate::i18n::t("common:state.loading"))
        .children((0..lines).map(|i| {
            bar(if i % 2 == 0 { tk.r(12.0) } else { tk.r(9.0) }, tk.r(0.75)).max_w(tk.r(40.0))
        }))
        .child(bar(tk.r(7.0), tk.r(1.75)))
        .child(bar(tk.r(5.0), tk.r(0.75)))
}

/// A line over a soft area, `w` × `h`, scaled to its own range: the shape of a series, not its
/// magnitude. Nothing with fewer than two values.
pub fn spark(values: &[f64], w: f32, h: f32, ink: Color) -> El {
    let (w, h) = (w.max(0.0), h.max(0.0));
    if values.len() < 2 || w < 8.0 || h < 8.0 {
        return el().size(w, h);
    }
    // At most one point every three points of width.
    let step = (values.len() as f32 / (w / 3.0)).ceil().max(1.0) as usize;
    let mut vals: Vec<f64> = values.iter().copied().step_by(step).collect();
    if !(values.len() - 1).is_multiple_of(step) {
        vals.push(values[values.len() - 1]);
    }
    let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = if max - min == 0.0 { 1.0 } else { max - min };
    let n = (vals.len() - 1) as f32;
    let pts: Vec<[f32; 2]> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            [
                i as f32 / n * w,
                h - 1.5 - ((v - min) / span) as f32 * (h - 3.0),
            ]
        })
        .collect();
    // One run per straight stretch, so the stroke's caps never stack along a flat line.
    let mut line: Vec<[f32; 2]> = pts.into_iter().fold(Vec::new(), |mut acc, p| {
        if acc.len() >= 2 {
            let [a, b] = [acc[acc.len() - 2], acc[acc.len() - 1]];
            if ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() < 1e-3 {
                acc.pop();
            }
        }
        acc.push(p);
        acc
    });
    line.dedup();
    // The area under the line, one quad per stretch (each convex).
    let mut shape = Shape::new();
    for seg in line.windows(2) {
        shape = shape.fill(
            vec![seg[0], seg[1], [seg[1][0], h], [seg[0][0], h]],
            ink.alpha(ink.a * 0.10),
        );
    }
    canvas(shape.stroke_with(line, false, 1.75, LineCap::Round, ink))
        .size(w, h)
        .inert()
        .a11y_hidden()
}

/// A stacked share bar: one segment per part, largest first, in steps of the foreground tone.
/// `neutral` draws one plain bar (the split is hidden).
pub fn share_bar(tk: &Tk, parts: &[f64], neutral: bool) -> El {
    let h = tk.r(0.5);
    let bar = row().w_full().h(h).gap(2.0).a11y_hidden();
    let total: f64 = parts.iter().filter(|p| **p > 0.0).sum();
    if neutral || total <= 0.0 {
        return bar.child(el().grow(1.0).h(h).radius(h / 2.0).fill(tk.muted));
    }
    let mut sorted: Vec<f64> = parts.iter().copied().filter(|p| *p > 0.0).collect();
    sorted.sort_by(|a, b| b.total_cmp(a));
    const TONES: [f32; 6] = [0.92, 0.62, 0.42, 0.28, 0.2, 0.14];
    bar.children(sorted.iter().enumerate().map(|(i, p)| {
        el().grow((*p / total) as f32 * 100.0)
            .w(0.0)
            .min_w(3.0)
            .h(h)
            .radius(h / 2.0)
            .fill(tk.fg.alpha(TONES[i.min(TONES.len() - 1)]))
    }))
}
