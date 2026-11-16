//! The stat tile the pages' top strips share with the home screen (an owner request): each stat
//! its own tile in the home screen's look (`frame`: the card tone, soft lift, continuous corner,
//! a glyph-and-name caption), one big figure, one muted line, and a sparkline where the stat has
//! a trend. Tiles sit in a row with the home screen's gutter and wrap into even rows when the
//! page is narrow. A tile that only says "all is well" is left out by its page; the page keeps
//! that information elsewhere.

use std::rc::Rc;

use rune::{Color, Cx, El, Icon, TextStyle, col, el, row, text};

use super::frame;
use crate::app::App;
use crate::theme::Tk;
use crate::ui::{ic, sensitive};

/// A tile's figure: text (masked by the caller where needed, blurred while amounts are hidden
/// when `sensitive`), or an element built in the row's figure style (a toggle, say).
pub enum Figure {
    Text {
        text: String,
        ink: Option<Color>,
        sensitive: bool,
    },
    /// Built in the row's figure style; `chars` is its text's length (for the row's sizing).
    Custom {
        chars: usize,
        build: Box<dyn FnOnce(TextStyle) -> El>,
    },
}

pub struct Stat {
    key: String,
    glyph: &'static Icon,
    glyph_ink: Option<Color>,
    label: String,
    figure: Figure,
    line: Option<(String, Option<Color>, bool)>,
    spark: Option<(Vec<f64>, Color)>,
    fade: f32,
    action: Option<(&'static str, String, crate::app::Handler)>,
    help: Option<String>,
    loading: bool,
}

impl Stat {
    pub fn new(
        key: impl Into<String>,
        glyph: &'static Icon,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Stat {
        Stat {
            key: key.into(),
            glyph,
            glyph_ink: None,
            label: label.into(),
            figure: Figure::Text {
                text: value.into(),
                ink: None,
                sensitive: false,
            },
            line: None,
            spark: None,
            fade: 1.0,
            action: None,
            help: None,
            loading: false,
        }
    }

    /// A tile whose figure is built in the row's figure style; `shown` is the text it shows.
    pub fn custom(
        key: impl Into<String>,
        glyph: &'static Icon,
        label: impl Into<String>,
        shown: &str,
        f: impl FnOnce(TextStyle) -> El + 'static,
    ) -> Stat {
        let mut s = Stat::new(key, glyph, label, "");
        s.figure = Figure::Custom {
            chars: shown.chars().count(),
            build: Box::new(f),
        };
        s
    }

    /// A loading tile: the caption, then skeleton bars.
    pub fn loading(key: impl Into<String>, glyph: &'static Icon, label: impl Into<String>) -> Stat {
        let mut s = Stat::new(key, glyph, label, "");
        s.loading = true;
        s
    }

    /// The figure's colour (a signal: alert, warning, gain).
    pub fn ink(mut self, c: Color) -> Stat {
        if let Figure::Text { ink, .. } = &mut self.figure {
            *ink = Some(c);
        }
        self
    }

    /// The caption glyph's colour (a signal); muted otherwise.
    pub fn glyph_ink(mut self, c: Color) -> Stat {
        self.glyph_ink = Some(c);
        self
    }

    /// The figure is an amount: it blurs while amounts are hidden.
    pub fn sensitive(mut self, on: bool) -> Stat {
        if let Figure::Text { sensitive, .. } = &mut self.figure {
            *sensitive = on;
        }
        self
    }

    pub fn line(mut self, s: impl Into<String>) -> Stat {
        self.line = Some((s.into(), None, false));
        self
    }

    /// The muted line with its own colour and whether it blurs while amounts are hidden.
    pub fn line_with(mut self, s: impl Into<String>, ink: Option<Color>, sensitive: bool) -> Stat {
        self.line = Some((s.into(), ink, sensitive));
        self
    }

    /// The stat's trend, oldest first, drawn in `ink` (nothing with fewer than two values).
    pub fn spark(mut self, values: Vec<f64>, ink: Color) -> Stat {
        if values.len() >= 2 {
            self.spark = Some((values, ink));
        }
        self
    }

    /// Dims the figure (a refresh in progress).
    pub fn fade(mut self, f: f32) -> Stat {
        self.fade = f;
        self
    }

    /// A tile that opens another place.
    pub fn link(mut self, aria: impl Into<String>, f: impl Fn(&mut App) + 'static) -> Stat {
        self.action = Some(("link", aria.into(), Rc::new(f)));
        self
    }

    /// A tile that does something in place (a filter).
    pub fn button(mut self, aria: impl Into<String>, f: impl Fn(&mut App) + 'static) -> Stat {
        self.action = Some(("button", aria.into(), Rc::new(f)));
        self
    }

    pub fn help(mut self, s: impl Into<String>) -> Stat {
        self.help = Some(s.into());
        self
    }
}

/// The narrowest a stat tile gets, in rem.
const MIN_TILE: f32 = 10.5;

/// Tiles per row for `n` tiles at `width`: all in one row when each keeps `MIN_TILE`, else the
/// fewest even rows.
pub fn per_row(n: usize, width: f32, min: f32, gap: f32) -> usize {
    if n == 0 {
        return 1;
    }
    let fit = (((width + gap) / (min + gap)).floor() as usize).clamp(1, n);
    let rows = n.div_ceil(fit);
    n.div_ceil(rows)
}

/// The figure size (rem) for the longest figure in `width`: 1.75 rem where it fits, down to
/// 1.25 rem (tabular digits are about 0.6 em wide).
pub fn figure_rem(chars: usize, inner_w: f32, rem: f32) -> f32 {
    if chars == 0 {
        return 1.75;
    }
    (inner_w / (chars as f32 * 0.6) / rem).clamp(1.25, 1.75)
}

/// The stats as a row of tiles under `key`; `label` names the group. The row measures itself
/// (one frame behind, as the home screen does).
pub fn row_of(app: &App, cx: &Cx<App>, tk: &Tk, key: &str, label: String, stats: Vec<Stat>) -> El {
    let gap = tk.card_gap();
    let width = cx.layout_size(key).map(|[w, _]| w).unwrap_or_else(|| {
        let win = if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        };
        (win - tk.r(18.0)).max(tk.r(30.0))
    });
    let n = stats.len();
    let per = per_row(n, width, tk.r(MIN_TILE), gap);
    let tile_w = (width - (per as f32 - 1.0) * gap) / per as f32;
    let chars = stats
        .iter()
        .filter_map(|s| match &s.figure {
            _ if s.loading => None,
            Figure::Text { text, .. } => Some(text.chars().count()),
            Figure::Custom { chars, .. } => Some(*chars),
        })
        .max()
        .unwrap_or(10)
        .max(4);
    let style = frame::figure(
        tk,
        figure_rem(chars, tile_w - 2.0 * frame::pad(tk), tk.r(1.0)),
    );
    let any_spark = stats.iter().any(|s| s.spark.is_some());
    let mut grid = col()
        .key(key.to_owned())
        .w_full()
        .gap(gap)
        .role("region")
        .label(label);
    let mut it = stats.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().gap(gap).items_stretch();
        for _ in 0..per {
            r = match it.next() {
                Some(s) => r.child(
                    tile(app, tk, s, style, tile_w - 2.0 * frame::pad(tk), any_spark)
                        .grow(1.0)
                        .w(0.0),
                ),
                None => r.child(el().grow(1.0).w(0.0)),
            };
        }
        grid = grid.child(r);
    }
    grid
}

/// The spark's height under a figure.
fn spark_h(tk: &Tk) -> f32 {
    tk.r(2.0)
}

/// The caption: the glyph (muted, or in a signal colour) and the stat's name.
fn caption(tk: &Tk, glyph: &'static Icon, ink: Option<Color>, label: &str) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .h(frame::caption_h(tk))
        .gap(tk.r(0.375))
        .center_y()
        .child(ic(glyph, tk.r(0.875), ink.unwrap_or(tk.muted_fg)).a11y_hidden())
        .child(
            text(label.to_owned())
                .font(tk.txs(600.0))
                .color(tk.muted_fg)
                .truncate()
                .min_w(0.0),
        )
}

fn tile(app: &App, tk: &Tk, s: Stat, style: TextStyle, inner_w: f32, room_for_spark: bool) -> El {
    let hide = app.prefs.hide_sensitive();
    let (a, b) = if tk.dark { (0.30, 0.22) } else { (0.06, 0.07) };
    let mut c = col()
        .key(s.key.clone())
        .min_w(0.0)
        .radius(frame::radius(tk))
        .continuous_corners()
        .fill(tk.card)
        .box_shadow(rune::Shadow {
            y: 1.0,
            blur: 2.0,
            color: Color::BLACK.alpha(a),
            ..rune::Shadow::default()
        })
        .box_shadow(rune::Shadow {
            y: 8.0,
            blur: 24.0,
            spread: -4.0,
            color: Color::BLACK.alpha(b),
            ..rune::Shadow::default()
        })
        .px(frame::pad(tk))
        .pt(frame::pad_top(tk))
        .pb(frame::pad_bottom(tk))
        .gap(frame::gap(tk))
        .child(caption(tk, s.glyph, s.glyph_ink, &s.label));
    if s.loading {
        let bar = |w: f32, h: f32| {
            row()
                .w(w)
                .h(h)
                .radius(tk.r(0.375))
                .fill(tk.muted.alpha(0.7))
        };
        return c
            .busy(true)
            .label(crate::i18n::t("common:state.loading"))
            .child(
                col()
                    .gap(tk.r(0.5))
                    .child(bar(tk.r(6.0), style.size * 1.1))
                    .child(bar(tk.r(8.0), tk.r(0.75))),
            );
    }
    let figure = match s.figure {
        Figure::Text {
            text: t,
            ink,
            sensitive: sens,
        } => sensitive(t, style, ink.unwrap_or(tk.fg), hide && sens)
            .single_line()
            .truncate()
            .min_w(0.0),
        Figure::Custom { build, .. } => build(style),
    };
    let mut body = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.125))
        .child(figure.opacity(s.fade));
    if let Some((line, ink, sens)) = s.line {
        body = body.child(
            sensitive(
                line.clone(),
                tk.txs(400.0),
                ink.unwrap_or(tk.muted_fg),
                hide && sens,
            )
            .single_line()
            .truncate()
            .min_w(0.0)
            .help_when_truncated(line),
        );
    }
    c = c.child(body);
    if let Some((values, ink)) = s.spark {
        // "Hide all" drops the trend (it tells how the figure moved); "Hide amounts" blurs it.
        let h = spark_h(tk);
        let line = if app.prefs.hide_shares() {
            el().w_full().h(h)
        } else {
            frame::spark(&values, inner_w, h, ink)
        };
        let line = if hide { line.blur(3.5) } else { line };
        c = c.child(col().w_full().grow(1.0).min_h(h).justify_end().child(line));
    } else if room_for_spark {
        c = c.child(el().w_full().grow(1.0));
    }
    if let Some(h) = s.help {
        c = c.help(h);
    }
    match s.action {
        Some((role, aria, f)) => c
            .role(role)
            .label(aria)
            .focusable(true)
            .focus_ring(2.0, 2.0, tk.ring)
            .cursor(rune::Cursor::Pointer)
            .hover(tk.card.mix(tk.fg, if tk.dark { 0.045 } else { 0.03 }))
            .press_scale(0.985)
            .on_press(move |a: &mut App| f(a)),
        None => c.role("group").label(s.label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_wrap_into_even_rows() {
        let (min, gap) = (200.0, 12.0);
        assert_eq!(per_row(6, 1300.0, min, gap), 6);
        assert_eq!(
            per_row(6, 1100.0, min, gap),
            3,
            "five fit, so two rows of three"
        );
        assert_eq!(per_row(4, 1100.0, min, gap), 4);
        assert_eq!(
            per_row(4, 700.0, min, gap),
            2,
            "three fit, so two rows of two"
        );
        assert_eq!(per_row(5, 700.0, min, gap), 3);
        assert_eq!(per_row(3, 100.0, min, gap), 1);
        assert_eq!(per_row(0, 900.0, min, gap), 1);
    }

    #[test]
    fn figures_shrink_to_fit_the_longest() {
        assert_eq!(figure_rem(4, 400.0, 16.0), 1.75);
        assert!((figure_rem(12, 160.0, 16.0) - 160.0 / 7.2 / 16.0).abs() < 1e-4);
        assert_eq!(figure_rem(40, 100.0, 16.0), 1.25);
    }
}
