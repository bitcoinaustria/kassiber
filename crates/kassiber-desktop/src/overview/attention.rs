//! Needs attention (Rune only, an owner-requested widget): only what waits on the owner, each
//! row a way to the screen that settles it. Signal, never reassurance: with nothing to do the
//! widget leaves the home screen in normal use (as iOS's Smart Stack shows a widget only while
//! it is relevant, and as the owner's rule wants no standing "all good"), and shows a small
//! calm line only while editing the home screen or in the gallery, so it can still be found,
//! sized and removed.
//!
//! Reads, all local and already made by the shell on mount and after each refresh:
//! - `ui.review.badges`: the review queue (`review`, else `quarantine_attention` + `swaps`),
//!   `quarantine` (whether it blocks reports) and `journals_needs_processing`;
//! - `ui.freshness.status`: sources that failed or block reports (`status == "failed"`,
//!   `blocking_reports`), and sources gone stale (`state == "stale"`);
//! - `ui.overview.snapshot`: sources in error, journals waiting (`status.needsJournals`),
//!   reports blocked by custody gaps or missing prices (`fiat.completeness.reasons`);
//! - the app's own state: a failed book refresh and a failed journal run.

use rune::{Color, El, col, row, text};

use super::grid::frame::{self, Slot};
use super::grid::model::WidgetId;
use crate::app::App;
use crate::i18n::{Lang, lang, t, tc};
use crate::icons as I;
use crate::model::Snapshot;
use crate::theme::Tk;
use crate::ui::ic;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Blocks reports or failed.
    Alert,
    /// Waits, but nothing is wrong yet.
    Warn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: &'static str,
    pub level: Level,
    pub title: String,
    pub detail: Option<String>,
    pub href: &'static str,
}

/// What needs attention now, most serious first.
pub fn items(app: &App, s: Option<&Snapshot>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut push = |key, level, title: String, detail: Option<String>, href| {
        out.push(Item {
            key,
            level,
            title,
            detail,
            href,
        })
    };
    let fresh = app
        .read("ui.freshness.status")
        .and_then(|f| f["sources"].as_array());
    let count = |f: &dyn Fn(&serde_json::Value) -> bool| {
        fresh
            .map(|x| x.iter().filter(|v| f(v)).count())
            .unwrap_or(0)
    };
    let blocking = count(&|x| x["blocking_reports"] == true);
    let failed = count(&|x| x["status"] == "failed" && x["blocking_reports"] != true);
    let stale = count(&|x| {
        x["state"] == "stale" && x["status"] != "failed" && x["blocking_reports"] != true
    });
    let errors = s
        .map(|s| s.connections.iter().filter(|c| c.status == "error").count())
        .unwrap_or(0);

    if blocking > 0 {
        push(
            "blocking",
            Level::Alert,
            t("overview:readiness.sourcesBlocking.title"),
            Some(tc(
                "overview:readiness.sourcesBlocking.detail",
                blocking,
                &[],
            )),
            "/connections",
        );
    }
    let failing = failed.max(errors);
    if failing > 0 {
        push(
            "sources",
            Level::Alert,
            t("overview:readiness.sourceAttention.title"),
            Some(tc(
                "overview:readiness.sourceAttention.detail",
                failing,
                &[],
            )),
            "/connections",
        );
    }
    if app.shell.sync_failed {
        push(
            "refresh",
            Level::Alert,
            t("chrome:notifications.condition.bookRefreshFailed.title"),
            Some(t("chrome:notifications.condition.bookRefreshFailed.body")),
            "/connections",
        );
    }
    if app.shell.journals_failed.is_some() {
        push(
            "journals-failed",
            Level::Alert,
            t("chrome:notifications.condition.journalsFailed.title"),
            Some(t("chrome:notifications.condition.journalsFailed.body")),
            "/journals",
        );
    }
    let badges = app.badges.as_ref();
    let review = badges.map(|b| b.total() as usize).unwrap_or(0);
    let quarantined = badges.and_then(|b| b.quarantine).unwrap_or(0.0) > 0.0
        || s.and_then(|s| s.status.as_ref())
            .is_some_and(|x| x.quarantines > 0.0);
    if review > 0 {
        push(
            "review",
            if quarantined {
                Level::Alert
            } else {
                Level::Warn
            },
            t("overview:readiness.reviewQueueOpen.title"),
            Some(tc("overview:readiness.reviewQueueOpen.detail", review, &[])),
            "/review",
        );
    }
    let completeness = s.map(|s| &s.fiat.completeness);
    if completeness.is_some_and(|c| c.has("custody_unresolved")) {
        push(
            "custody",
            Level::Alert,
            t("overview:readiness.custodyUnresolved.title"),
            Some(t("overview:readiness.custodyUnresolved.detail")),
            "/journals",
        );
    }
    if badges.is_some_and(|b| b.journals_needs_processing)
        || s.is_some_and(Snapshot::needs_journals)
    {
        push(
            "journals",
            Level::Warn,
            t("overview:readiness.reprocessJournals.title"),
            Some(t("overview:readiness.reprocessJournals.detail")),
            "/journals",
        );
    }
    if let Some(c) = completeness.filter(|c| c.has("missing_prices")) {
        let n = c.missing_price_count as usize;
        push(
            "prices",
            Level::Warn,
            t("overview:readiness.missingPrices.title"),
            Some(tc("overview:readiness.missingPrices.detail", n, &[])),
            "/review",
        );
    }
    if stale > 0 {
        push(
            "stale",
            Level::Warn,
            tc(
                "connections:metrics.upToDateNotUpToDate",
                stale,
                &[("count", &crate::format::count(stale))],
            ),
            None,
            "/connections",
        );
    }
    out.sort_by_key(|i| i.level);
    out
}

pub fn title() -> String {
    t("connections:metrics.needsAttention")
}

pub fn description() -> String {
    l(
        "Only what waits on you: review, failed sources, journals, blocked reports.",
        "Nur was auf dich wartet: Prüfung, fehlgeschlagene Quellen, Journale, blockierte Berichte.",
    )
}

fn l(en: &str, de: &str) -> String {
    match lang() {
        Lang::De => de.to_owned(),
        Lang::En => en.to_owned(),
    }
}

fn calm() -> String {
    l("Nothing needs attention.", "Nichts braucht Aufmerksamkeit.")
}

fn more(n: usize) -> String {
    match lang() {
        Lang::De => format!("+{n} weitere"),
        Lang::En => format!("+{n} more"),
    }
}

fn tone(tk: &Tk, level: Level) -> Color {
    match level {
        Level::Alert => tk.alert.solid,
        Level::Warn => tk.warn.solid,
    }
}

fn glyph(level: Level) -> &'static rune::Icon {
    match level {
        Level::Alert => &I::SHIELD_ALERT,
        Level::Warn => &I::TRIANGLE_ALERT,
    }
}

/// True when the widget has something to show outside edit mode (it leaves the grid otherwise).
pub fn has_items(app: &App) -> bool {
    !items(app, app.snapshot()).is_empty()
}

/// The first item's screen and how it is named: where a click on a Small tile goes.
pub fn first(app: &App) -> Option<(&'static str, String)> {
    items(app, app.snapshot())
        .into_iter()
        .next()
        .map(|i| (i.href, i.title))
}

/// The content height at `size`: the count and the first item (Small), or a row per item up
/// to three (Medium); the calm line while editing.
pub fn natural(app: &App, tk: &Tk, s: &Snapshot, size: super::grid::model::Size) -> f32 {
    let n = items(app, Some(s)).len();
    if n == 0 {
        return tk.r(3.5);
    }
    if size == super::grid::model::Size::Small {
        return tk.r(2.25) * 1.15
            + tk.r(0.375)
            + 2.0 * tk.txs(600.0).size * 1.5
            + 2.0 * tk.t2xs(400.0).size * 1.5
            + tk.r(0.25);
    }
    let rows = n.min(3) as f32;
    let more = if n > 3 {
        tk.t2xs(500.0).size * 1.5 + tk.r(0.25)
    } else {
        0.0
    };
    rows * tk.r(2.25) + (rows - 1.0) * tk.r(0.25) + more
}

/// The widget's content for its slot.
pub fn body(app: &App, tk: &Tk, s: &Snapshot, slot: &Slot) -> El {
    let list = items(app, Some(s));
    if list.is_empty() {
        // Only seen while editing or in the gallery: one small, quiet line.
        return col()
            .key(slot.key("attention-calm"))
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .center()
            .gap(tk.r(0.375))
            .role("status")
            .child(ic(&I::CIRCLE_CHECK, tk.r(1.25), tk.muted_fg.alpha(0.7)))
            .child(
                text(calm())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .align(rune::Align::Center)
                    .wrap(),
            );
    }
    let worst = list[0].level;
    if !slot.wide() {
        // Small: how many, the most serious one named, the rest counted.
        let mut c = col()
            .key(slot.key("attention-small"))
            .w_full()
            .h_full()
            .min_h(0.0)
            .gap(tk.r(0.375))
            .justify_between()
            .role("status")
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(
                        text(crate::format::count(list.len()))
                            .font(frame::figure(tk, 2.25))
                            .color(tone(tk, worst)),
                    )
                    .child(ic(glyph(worst), tk.r(1.25), tone(tk, worst)).a11y_hidden()),
            );
        let mut words = col().w_full().min_w(0.0).gap(tk.r(0.125)).child(
            text(list[0].title.clone())
                .font(tk.txs(600.0))
                .color(tk.fg)
                .wrap()
                .max_lines(2),
        );
        if let Some(d) = &list[0].detail {
            words = words.child(
                text(d.clone())
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        }
        if list.len() > 1 {
            words = words.child(
                text(more(list.len() - 1))
                    .font(tk.t2xs(500.0))
                    .color(tk.muted_fg),
            );
        }
        c = c.child(words);
        return c;
    }
    // Medium: a row each, as many as fit, the rest counted.
    let row_h = tk.r(2.25);
    let gap = tk.r(0.25);
    let more_h = gap + tk.t2xs(500.0).size * 1.5;
    let fits = |k: usize| {
        let k_f = k as f32;
        k_f * row_h + (k_f - 1.0).max(0.0) * gap + if k < list.len() { more_h } else { 0.0 }
            <= slot.h + 0.5
    };
    let shown = (1..=list.len()).rev().find(|k| fits(*k)).unwrap_or(1);
    let mut out = col()
        .key(slot.key("attention-list"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(tk.r(0.25))
        .role("list")
        .label(title());
    for item in list.iter().take(shown) {
        let ink = tone(tk, item.level);
        let href = item.href;
        let name = match &item.detail {
            Some(d) => format!("{}: {d}", item.title),
            None => item.title.clone(),
        };
        let mut words = col().grow(1.0).w(0.0).min_w(0.0).child(
            text(item.title.clone())
                .font(tk.txs(600.0))
                .color(tk.fg)
                .truncate(),
        );
        if let Some(d) = &item.detail {
            words = words.child(
                text(d.clone())
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg)
                    .truncate(),
            );
        }
        let line = row()
            .key(slot.key(format!("attention-{}", item.key)))
            .w_full()
            .h(row_h)
            .px(tk.r(0.5))
            .gap(tk.r(0.625))
            .center_y()
            .radius(tk.r(0.625))
            .continuous_corners()
            .fill(ink.alpha(if tk.dark { 0.10 } else { 0.07 }))
            .hover(ink.alpha(if tk.dark { 0.16 } else { 0.11 }))
            .role("link")
            .label(name)
            .focusable(!slot.pv)
            .focus_ring(2.0, 1.0, tk.ring)
            .cursor(rune::Cursor::Pointer)
            .child(ic(glyph(item.level), tk.r(1.0), ink).a11y_hidden())
            .child(words)
            .child(ic(&I::CHEVRON_RIGHT, tk.r(0.875), tk.muted_fg).a11y_hidden());
        out = out.child(super::grid::press(
            line,
            WidgetId::Attention,
            slot.pv,
            move |a| a.navigate(href),
        ));
    }
    if list.len() > shown {
        out = out.child(
            text(more(list.len() - shown))
                .font(tk.t2xs(500.0))
                .color(tk.muted_fg)
                .px(tk.r(0.5)),
        );
    }
    out
}
