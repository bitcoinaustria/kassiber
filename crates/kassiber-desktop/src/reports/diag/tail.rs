//! The Logs surface (assistant-panel/surfaces/LogsSurface.tsx): a live tail of the same bounded,
//! in-memory ring the Logs page reads (`logs::State::store`), always redacted here and with
//! amounts masked while sensitive values are hidden. A level filter (All / Warnings / Errors),
//! a line filter, the newest 500 lines in a terminal-style log that follows new lines until
//! scrolled up ("Follow new lines"), and a footer link to the page. Nothing is written anywhere.

use std::cell::Cell;

use rune::{Color, Cx, El, ScrollPos, ScrollSpec, TextEdit, Tx, col, row, text};
use rune_kit::TextInput;

use super::logstore::Record;
use super::redact::Opts;
use super::surfaces::{self, EmptyTone, hex};
use super::time;
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// The tail it keeps on screen; the Logs page pages through the rest.
pub const TAIL: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    All,
    Warning,
    Error,
}

impl Level {
    pub fn keeps(self, level: &str) -> bool {
        match self {
            Level::Error => level == "error",
            Level::Warning => level == "warning" || level == "error",
            Level::All => true,
        }
    }
}

pub struct State {
    pub level: Level,
    pub query: TextEdit,
    pub following: bool,
    /// Scroll to the end on the next poll (mount, Follow).
    pub scroll_end: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            level: Level::All,
            query: TextEdit::new(""),
            following: true,
            scroll_end: true,
        }
    }
}

thread_local! {
    /// Whether the tail sits at its end (from `on_scroll_offset`, which cannot touch state).
    static AT_END: Cell<bool> = const { Cell::new(true) };
}

/// `fieldText`: `key=value` pairs.
pub fn field_text(r: &Record) -> String {
    super::logstore::format_fields(&r.fields)
}

/// The surface's lines: level-filtered, redacted (amounts masked while sensitive values are
/// hidden), matched against module, message and fields, newest `TAIL` kept.
pub fn visible(app: &App, s: &State) -> Vec<Record> {
    let o = Opts::screen(true, app.prefs.hide_sensitive());
    let needle = s.query.text.trim().to_lowercase();
    let mut out: Vec<Record> = Vec::new();
    for r in app.reports.diag.logs.store.records.iter() {
        if !s.level.keeps(&r.level) {
            continue;
        }
        let shown = r.redacted(&o);
        if !needle.is_empty()
            && !format!("{} {} {}", shown.module, shown.msg, field_text(&shown))
                .to_lowercase()
                .contains(&needle)
        {
            continue;
        }
        out.push(shown);
    }
    let skip = out.len().saturating_sub(TAIL);
    out.split_off(skip)
}

/// Every frame while the surface is a tab of the panel: the follow state and the end scroll.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let s = &mut app.reports.diag.tail;
    let mut changed = false;
    let at_end = AT_END.with(Cell::get);
    if at_end != s.following && !s.scroll_end {
        s.following = at_end;
        changed = true;
    }
    if s.scroll_end {
        s.scroll_end = false;
        AT_END.with(|c| c.set(true));
        tx.scroll_to_bottom_instant("tail-log");
    }
    changed
}

/// `LEVEL_TONE`.
pub fn level_ink(tk: &Tk, level: &str) -> Color {
    match level {
        "trace" => tk.muted_fg.alpha(0.7),
        "debug" => hex(if tk.dark { "#a5b4fc" } else { "#4f46e5" }),
        "info" => hex(if tk.dark { "#7dd3fc" } else { "#0284c7" }),
        "warning" => hex(if tk.dark { "#fcd34d" } else { "#d97706" }),
        _ => tk.destructive,
    }
}

/// `LEVEL_SHORT`.
pub fn level_short(level: &str) -> &'static str {
    match level {
        "trace" => "TRC",
        "debug" => "DBG",
        "info" => "INF",
        "warning" => "WRN",
        _ => "ERR",
    }
}

/// `font-mono text-[0.6875rem] leading-[1.125rem]`.
pub fn mono(tk: &Tk, weight: f32) -> rune::TextStyle {
    rune::TextStyle::new(tk.r(0.6875), weight)
        .lh(tk.r(1.125))
        .mono()
}

/// `LogLine`: `grid-cols-[auto_auto_minmax(0,1fr)] gap-x-2 px-3`, time, level, then module,
/// message and fields as one wrapping run; errors on `bg-destructive/5`.
pub fn line(tk: &Tk, r: &Record, key: String, lead: Option<El>) -> El {
    let fields = field_text(r);
    let module = format!("{} ", r.module);
    let mut content = format!("{module}{}", r.msg);
    let msg_span = rune::Span::new(module.len(), content.len()).color(tk.fg);
    if !fields.is_empty() {
        content.push(' ');
        content.push_str(&fields);
    }
    let time = time::line_time(&r.ts);
    let mut el = row()
        .key(key)
        .w_full()
        .px(tk.r(0.75))
        .gap(tk.r(0.5))
        .items_start()
        .hover(tk.accent.alpha(0.4))
        .help(format!("{} {}", r.ts, r.module))
        .label(format!(
            "{time} {} {} {}",
            level_short(&r.level),
            r.module,
            r.msg
        ))
        .children(lead)
        .child(
            text(time)
                .font(mono(tk, 400.0))
                .color(tk.muted_fg.alpha(0.8)),
        )
        .child(
            text(level_short(&r.level))
                .font(mono(tk, 600.0))
                .color(level_ink(tk, &r.level)),
        )
        .child(
            rune::rich(content, vec![msg_span])
                .font(mono(tk, 400.0))
                .color(tk.muted_fg)
                .wrap_anywhere()
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        );
    if r.level == "error" {
        el = el.fill(tk.destructive.alpha(0.05));
    }
    el
}

/// The level radios.
pub fn level_filter(tk: &Tk, key: &str, current: Level, pick: fn(&mut App, Level)) -> El {
    surfaces::segmented(
        tk,
        key,
        t("assistant:panel.logs.level"),
        vec![
            (Level::All, None, t("assistant:panel.logs.levels.all")),
            (
                Level::Warning,
                None,
                t("assistant:panel.logs.levels.warning"),
            ),
            (Level::Error, None, t("assistant:panel.logs.levels.error")),
        ],
        current,
        pick,
    )
}

/// The line filter (`h-6 rounded-md border border-border/70 bg-background px-2`, a `size-3`
/// search glyph, `text-xs`).
pub fn search(tk: &Tk, key: &str, edit: &TextEdit, on_edit: fn(&mut App) -> &mut TextEdit) -> El {
    let k = crate::txdetail::widgets::form_kit(tk);
    let field = TextInput::new(edit)
        .label(t("assistant:panel.logs.search"))
        .placeholder(t("assistant:panel.logs.search"))
        .plain()
        .width(None)
        .height(tk.r(1.5) - 2.0)
        .text_style(tk.txs(400.0))
        .on_edit(move |a: &mut App| on_edit(a))
        .build(&k)
        .key(key.to_owned())
        .grow(1.0)
        .min_w(0.0);
    let ring = if edit.focused {
        tk.ring
    } else {
        tk.border.alpha(tk.border.a * 0.7)
    };
    row()
        .h(tk.r(1.5))
        .grow(1.0)
        .min_w(0.0)
        .pl(tk.r(0.5))
        .gap(tk.r(0.125))
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, ring)
        .border_layout()
        .fill(tk.background)
        .child(ic(&I::SEARCH, tk.r(0.75), tk.muted_fg).a11y_hidden())
        .child(field)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk) -> El {
    let s = &app.reports.diag.tail;
    let lines = visible(app, s);
    let total = app.reports.diag.logs.store.records.len();
    let toolbar = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .child(level_filter(tk, "tail-level", s.level, |a, l| {
            a.reports.diag.tail.level = l
        }))
        .child(search(tk, "tail-search", &s.query, |a| {
            &mut a.reports.diag.tail.query
        }));
    let footer = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .child(
            text(t("assistant:panel.logs.redacted"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        )
        .child(
            row()
                .key("tail-open-page")
                .h(tk.r(1.5))
                .px(tk.r(0.5))
                .gap(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.375))
                .hover(if tk.dark {
                    tk.accent.alpha(0.5)
                } else {
                    tk.accent
                })
                .role("button")
                .label(t("assistant:panel.logs.openPage"))
                .focusable(true)
                .on_press(|a: &mut App| a.navigate("/logs"))
                .child(
                    text(t("assistant:panel.logs.openPage"))
                        .font(tk.txs(500.0))
                        .color(tk.muted_fg),
                )
                .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.75), tk.muted_fg)),
        );
    let body = if lines.is_empty() {
        let title = t(if total > 0 {
            "assistant:panel.logs.noMatch"
        } else {
            "assistant:panel.logs.emptyTitle"
        });
        let body = (total == 0).then(|| t("assistant:panel.logs.emptyBody"));
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(surfaces::empty(
                tk,
                &I::SCROLL_TEXT,
                EmptyTone::Muted,
                title,
                body,
                None,
            ))
    } else {
        let mut log = col()
            .key("tail-log")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .py(tk.r(0.25))
            .fill(tk.muted.alpha(0.2))
            .role("log")
            .label(t("assistant:panel.surfaceLogs"))
            .focusable(true)
            .scroll(ScrollSpec {
                stick_bottom: s.following,
            })
            .on_scroll_offset(|at: ScrollPos| {
                AT_END.with(|c| c.set(at.max[1] - at.offset[1] < 24.0))
            });
        for (i, r) in lines.iter().enumerate() {
            log = log.child(line(tk, r, format!("tail-line-{i}"), None));
        }
        let mut wrap = col().w_full().grow(1.0).min_h(0.0).child(log);
        if !s.following {
            let label = t("assistant:panel.logs.follow");
            let [vw, vh] = cx.layout_size("tail-log").unwrap_or([400.0, 400.0]);
            let bw = tk.r(0.5) * 2.0
                + tk.r(0.75)
                + tk.r(0.25)
                + rune::text::measure(&label, &[], tk.txs(500.0), None).0;
            wrap = wrap.child(
                row()
                    .absolute(vw - bw - tk.r(0.75), vh - tk.r(1.75) - tk.r(0.75))
                    .child(
                        secondary_xs(tk, "tail-follow", &I::ARROW_DOWN, &label)
                            .box_shadow(rune::Shadow {
                                y: 1.0,
                                blur: 3.0,
                                color: Color::BLACK.alpha(0.1),
                                ..Default::default()
                            })
                            .on_press(|a: &mut App| {
                                a.reports.diag.tail.following = true;
                                a.reports.diag.tail.scroll_end = true;
                            }),
                    ),
            );
        }
        wrap
    };
    surfaces::frame(
        tk,
        t("assistant:panel.surfaceLogs"),
        Some(toolbar),
        body,
        Some(footer),
    )
}

/// shadcn `Button size="xs" variant="secondary"` (`h-7 px-1.5 gap-1 text-xs`, `bg-secondary`).
pub fn secondary_xs(tk: &Tk, key: &str, glyph: &'static rune::Icon, label: &str) -> El {
    row()
        .key(key.to_owned())
        .h(tk.r(1.75))
        .px(tk.r(0.5))
        .gap(tk.r(0.25))
        .center()
        .radius(tk.r(0.375))
        .fill(tk.muted)
        .hover(tk.muted.alpha(0.8))
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .child(ic(glyph, tk.r(0.75), tk.fg))
        .child(text(label.to_owned()).font(tk.txs(500.0)).color(tk.fg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_filter_keeps_at_or_above() {
        assert!(Level::All.keeps("trace"));
        assert!(
            Level::Warning.keeps("error")
                && Level::Warning.keeps("warning")
                && !Level::Warning.keeps("info")
        );
        assert!(Level::Error.keeps("error") && !Level::Error.keeps("warning"));
        assert_eq!(level_short("warning"), "WRN");
    }

    #[test]
    fn short_ids_keep_head_and_tail() {
        assert_eq!(super::super::changes::short_id("abc"), "abc");
        assert_eq!(
            super::super::changes::short_id("0123456789abcdefghijkl"),
            "0123456789…ghijkl"
        );
    }
}
