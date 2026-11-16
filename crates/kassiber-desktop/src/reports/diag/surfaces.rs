//! The right panel's own surfaces (components/kb/assistant-panel/surfaces/SurfaceLayout.tsx):
//! the frame they share (a slim toolbar over a scrolling body, an optional footer), the centred
//! empty state, the segmented radio filter and the dispatch the panel calls. The Logs and
//! Changes surfaces live in `tail` and `changes`; the `/logs` and `/activity` pages reuse their
//! rows, filters and empty states.

use rune::{Color, Cx, El, Icon, Shadow, Tx, col, row, text};

use crate::app::App;
use crate::assistant::Surface;
use crate::theme::Tk;
use crate::ui::ic;

/// The surface the panel shows in front (its tab panel).
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, s: Surface, layer: &mut Vec<El>) -> El {
    let body = match s {
        Surface::Logs => super::tail::view(app, cx, tk),
        Surface::Changes => super::changes::view(app, tk, layer),
    };
    col()
        .key(format!("asst-surface-{}", s.id()))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("tabpanel")
        .label(s.label())
        .child(body)
}

/// `SurfaceFrame`: `section` with the toolbar (`min-h-9 border-b border-border/60 px-3 py-1.5
/// text-xs gap-2`), the body (`flex-1 overflow-y-auto`, here the caller's) and the footer
/// (`border-t border-border/60 px-3 py-2 gap-2`).
pub fn frame(tk: &Tk, label: String, toolbar: Option<El>, body: El, footer: Option<El>) -> El {
    let hair = tk.border.alpha(tk.border.a * 0.6);
    let mut c = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("region")
        .label(label);
    if let Some(t) = toolbar {
        c = c.child(
            row()
                .w_full()
                .min_h(tk.r(2.25))
                .px(tk.r(0.75))
                .py(tk.r(0.375))
                .gap(tk.r(0.5))
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], hair)
                .child(t),
        );
    }
    c = c.child(col().w_full().grow(1.0).min_h(0.0).child(body));
    if let Some(f) = footer {
        c = c.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .border_sides([1.0, 0.0, 0.0, 0.0], hair)
                .child(f),
        );
    }
    c
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmptyTone {
    Muted,
    Destructive,
}

/// `SurfaceEmpty`: a `size-9` round icon well, the title (`text-sm font-medium`), an optional
/// body (`max-w-64 text-xs text-muted-foreground`) and action; `min-h-48 px-6 py-10`.
pub fn empty(
    tk: &Tk,
    glyph: &'static Icon,
    tone: EmptyTone,
    title: String,
    body: Option<String>,
    action: Option<El>,
) -> El {
    let (border, fill, ink) = match tone {
        EmptyTone::Muted => (tk.border, tk.muted.alpha(0.5), tk.muted_fg),
        EmptyTone::Destructive => (
            tk.destructive.alpha(0.3),
            tk.destructive.alpha(0.1),
            tk.destructive,
        ),
    };
    let mut c = col()
        .w_full()
        .grow(1.0)
        .min_h(tk.r(12.0))
        .px(tk.card_pad())
        .py(tk.r(2.5))
        .gap(tk.r(0.5))
        .center()
        .role(if tone == EmptyTone::Destructive {
            "alert"
        } else {
            "status"
        })
        .label(title.clone())
        .child(
            row()
                .size(tk.r(2.25), tk.r(2.25))
                .center()
                .radius(999.0)
                .border(1.0, border)
                .fill(fill)
                .child(ic(glyph, tk.r(1.0), ink))
                .a11y_hidden(),
        )
        .child(
            text(title)
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .align(rune::Align::Center),
        );
    if let Some(b) = body {
        c = c.child(
            text(b)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .align(rune::Align::Center)
                .max_w(tk.r(16.0))
                .wrap(),
        );
    }
    if let Some(a) = action {
        c = c.child(a);
    }
    c
}

/// The surfaces' segmented filter (`role="radiogroup"`, `rounded-md bg-muted/60 p-0.5 gap-0.5`;
/// items `h-5 rounded-sm px-2 text-xs`, the checked one `bg-background shadow-xs`).
pub fn segmented<T: Copy + PartialEq + 'static>(
    tk: &Tk,
    key: &str,
    label: String,
    items: Vec<(T, Option<&'static Icon>, String)>,
    current: T,
    pick: fn(&mut App, T),
) -> El {
    let mut g = row()
        .key(key.to_owned())
        .gap(tk.r(0.125))
        .padding(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.375))
        .fill(tk.muted.alpha(0.6))
        .role("radiogroup")
        .label(label)
        .focus_group();
    for (i, (value, glyph, name)) in items.into_iter().enumerate() {
        let on = value == current;
        let ink = if on { tk.fg } else { tk.muted_fg };
        let mut b = row()
            .key(format!("{key}-{i}"))
            .h(tk.r(1.25))
            .px(tk.r(0.5))
            .gap(tk.r(0.25))
            .center_y()
            .radius(tk.r(0.25))
            .role("radio")
            .checked(on)
            .label(name.clone())
            .focusable(true)
            .focus_selected(on)
            .on_press_tx(move |a: &mut App, _: &mut Tx| pick(a, value));
        if on {
            b = b.fill(tk.background).box_shadow(Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Default::default()
            });
        }
        if let Some(gl) = glyph {
            b = b.child(ic(gl, tk.r(0.75), ink));
        }
        g = g.child(b.child(text(name).font(tk.txs(400.0)).color(ink)));
    }
    g
}

/// `animate-spin` on a `Loader2`.
pub fn spinner(tk: &Tk, size: f32) -> El {
    ic(&crate::icons::LOADER_CIRCLE, size, tk.muted_fg).looping(rune::LoopSpec::rotation(1.0))
}

/// Tailwind's `X-600 dark:X-300/400` text tone.
pub fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::WHITE)
}
