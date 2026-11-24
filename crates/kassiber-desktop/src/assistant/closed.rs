//! What stays below the page while the assistant is out of view (AssistantLayout.tsx,
//! AssistantClosedBar.tsx): the approval notice, the closed bar (Settings → Appearance:
//! "Composer" or "Launcher", placed left, center or right by the dock position), and the
//! polite announcement of each new approval. Also the layout's effects: a new approval or a
//! failed turn asks for the panel once.

use rune::{El, Tx, col, el, row, text};

use super::composer::{self, Opts};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// `enabled`: everywhere the panel can be but the full assistant page (the assistant itself).
fn enabled(app: &App) -> bool {
    super::hosts_panel(app) && !super::owns(app.route.path())
}

/// `showAssistant`: the panel is shown with the assistant as one of its tabs.
fn assistant_shown(app: &App) -> bool {
    super::panel_shown(app) && super::tabs(app).contains(&super::Front::Assistant)
}

/// Whether the closed bar shows, and in which mode.
pub fn bar_mode(app: &App) -> Option<&'static str> {
    let mode = app.settings.prefs.closed_bar;
    (enabled(app) && !assistant_shown(app) && !super::column_claimed(app) && mode != "hidden")
        .then_some(mode)
}

/// `openAssistantPanel({ focusComposer: true })`.
fn open_focused(s: &mut App) {
    super::open_panel(s);
    s.assistant.draft.focused = true;
}

/// Hook for the shell: the elements under the page's scroll, in order (none on the assistant
/// page, while locked, or with AI features off).
pub fn below_page(app: &App, cx: &mut rune::Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Vec<El> {
    let mut v = Vec::new();
    if !enabled(app) || app.locked {
        return v;
    }
    if !super::assistant_in_view(app) && !app.assistant.pending_consents.is_empty() {
        v.push(notice(app, tk));
    }
    if let Some(mode) = bar_mode(app) {
        v.push(bar(app, cx, tk, mode, layer));
    }
    v
}

fn amber(tk: &Tk) -> rune::Color {
    if tk.dark {
        rune::color::hex("#fbbf24").unwrap_or(tk.fg)
    } else {
        rune::color::hex("#b45309").unwrap_or(tk.fg)
    }
}

fn amber_strong(tk: &Tk) -> rune::Color {
    if tk.dark {
        rune::color::hex("#fcd34d").unwrap_or(tk.fg)
    } else {
        rune::color::hex("#92400e").unwrap_or(tk.fg)
    }
}

/// `PendingApprovalNotice`: `border-t border-amber-500/30 bg-amber-500/5 px-3 py-1.5 text-xs`.
fn notice(app: &App, tk: &Tk) -> El {
    let consents = &app.assistant.pending_consents;
    let amber500 = rune::color::hex("#f59e0b").unwrap_or(tk.fg);
    let summary = consents[0].summary.clone();
    let review = t("assistant:consent.notice.review");
    row()
        .key("asst-approval-notice")
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .px(tk.r(1.0))
        .py(tk.r(0.375))
        .fill(amber500.alpha(0.05))
        .border_sides([1.0, 0.0, 0.0, 0.0], amber500.alpha(0.3))
        .role("region")
        .label(t("assistant:consent.notice.label"))
        .child(ic(&I::SHIELD, tk.r(0.875), amber(tk)).a11y_hidden())
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.25))
                .center_y()
                .child(
                    text(tc("assistant:consent.notice.waiting", consents.len(), &[]))
                        .font(tk.txs(500.0))
                        .color(amber_strong(tk)),
                )
                .child(
                    text(summary)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate()
                        .min_w(0.0),
                ),
        )
        .child(
            text(review.clone())
                .key("asst-approval-notice-review")
                .font(tk.txs(500.0))
                .color(tk.fg)
                .radius(tk.r(0.125))
                .role("button")
                .label(review)
                .focusable(true)
                .on_press(|s: &mut App| {
                    super::open_panel(s);
                    super::set_front(s, super::Front::Assistant);
                }),
        )
}

/// `AssistantClosedBar`: `border-t border-border/50 bg-background px-3 py-1.5 sm:px-4`.
fn bar(app: &App, cx: &mut rune::Cx<App>, tk: &Tk, mode: &'static str, layer: &mut Vec<El>) -> El {
    let shortcut = crate::shell::sidebar::shortcut("I", false);
    let place = |r: El| match app.settings.prefs.dock_position {
        "left" => r,
        "center" => row().w_full().justify_center().child(r),
        _ => row().w_full().justify_end().child(r),
    };
    let inner = if mode == "launcher" {
        let launcher = row()
            .key("asst-bar-launcher")
            .gap(tk.r(0.5))
            .center_y()
            .radius(999.0)
            .px(tk.r(0.625))
            .py(tk.r(0.375))
            .hover(tk.muted)
            .role("button")
            .label(t("assistant:panel.open"))
            .focusable(true)
            .on_press(open_focused)
            .child(ic(&I::SPARKLES, tk.r(0.875), tk.primary).a11y_hidden())
            .child(
                text(t("assistant:panel.launcher"))
                    .font(tk.txs(500.0))
                    .color(tk.muted_fg),
            )
            .child(crate::txdetail::widgets::kbd(tk, &shortcut).a11y_hidden());
        place(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .children(conversation_button(app, tk, &shortcut))
                .child(launcher),
        )
    } else {
        let mut r = row().w_full().max_w(tk.r(36.0)).gap(tk.r(0.25)).items_end();
        if let Some(b) = conversation_button(app, tk, &shortcut) {
            r = r.child(row().h(tk.r(2.75)).center_y().child(b));
        }
        let unfolded = app.assistant.bar_unfolded || app.assistant.picker.open == Some("bar");
        let comp = composer::view(
            app,
            cx,
            tk,
            Opts {
                host: "bar",
                hero: false,
                suggestions: false,
                compact: !unfolded,
            },
            layer,
        );
        let mut column = col().grow(1.0).min_w(0.0).on_hover_change(
            |s: &mut App, inside: bool, _tx: &mut Tx| s.assistant.bar_hover = inside,
        );
        // A question sent from the dock shares the same context as the panel's: show the tray
        // (and its remove buttons) whenever it shares more than the page, so nothing is shared
        // that the user cannot see before Send.
        if bar_shares_more_than_the_page(app) {
            column = column.child(composer::context_tray(app, tk, false).mb(tk.r(0.375)));
        }
        r = r.child(column.child(comp));
        place(r)
    };
    col()
        .key("asst-closed-bar")
        .w_full()
        .px(tk.r(1.0))
        .py(tk.r(0.375))
        .fill(tk.background)
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border.alpha(tk.border.a * 0.5))
        .role("region")
        .label(t("assistant:dock.label"))
        .child(inner)
}

/// Whether the next question shares an item or a chart selection, not only the page.
fn bar_shares_more_than_the_page(app: &App) -> bool {
    super::tray::build(app)
        .chips
        .iter()
        .any(|c| c.part != super::tray::Part::Page)
}

/// The "N messages" / "Working" button that opens the conversation (only with messages).
fn conversation_button(app: &App, tk: &Tk, shortcut: &str) -> Option<El> {
    let a = &app.assistant;
    if a.messages.is_empty() {
        return None;
    }
    let label = t("assistant:panel.openConversation");
    let lead = if a.streaming {
        el().size(6.0, 6.0)
            .radius(3.0)
            .fill(tk.primary)
            .a11y_hidden()
    } else {
        ic(&I::MESSAGE_SQUARE_TEXT, tk.r(0.875), tk.muted_fg).a11y_hidden()
    };
    let words = if a.streaming {
        t("assistant:dock.working")
    } else {
        tc("assistant:page.messageCount", a.messages.len(), &[])
    };
    Some(
        row()
            .key("asst-bar-conversation")
            .gap(tk.r(0.375))
            .center_y()
            .radius(999.0)
            .px(tk.r(0.625))
            .py(tk.r(0.375))
            .hover(tk.muted)
            .role("button")
            .label(label.clone())
            .help(format!("{label} ({shortcut})"))
            .focusable(true)
            .on_press(open_focused)
            .child(lead)
            .child(text(words).font(tk.txs(500.0)).color(tk.muted_fg)),
    )
}

/// Every frame: the closed composer's fold, the layout's open-on-approval/failure effects and
/// the approval announcements. True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    // The compact composer unfolds while focus is in it (field, attach, picker, send) or the
    // pointer is on it, so a press on its toolbar lands before focus leaves the field.
    let focus_in = tx.focused().is_some_and(|f| {
        f.split('/')
            .any(|seg| seg.starts_with("asst-") && seg.ends_with("-bar"))
    });
    let unfolded = bar_mode(app) == Some("composer")
        && (focus_in || app.assistant.bar_hover || app.assistant.picker.open == Some("bar"));
    if unfolded != app.assistant.bar_unfolded {
        app.assistant.bar_unfolded = unfolded;
        changed = true;
    }
    let on = enabled(app);
    // A new approval, or a failed turn, asks for the column once (the page holding it gives
    // it back); closing the panel again is the user's choice.
    let newest = app
        .assistant
        .pending_consents
        .last()
        .map(|c| c.call_id.clone());
    if newest != app.assistant.seen_approval {
        app.assistant.seen_approval = newest.clone();
        if on && newest.is_some() {
            super::open_panel(app);
        }
        changed = true;
    }
    if app.assistant.error != app.assistant.seen_error {
        app.assistant.seen_error = app.assistant.error.clone();
        if on && app.assistant.error.is_some() {
            super::open_panel(app);
        }
        changed = true;
    }
    // `PendingApprovalAnnouncer`: names each new approval, wherever the conversation is.
    let fresh: Vec<String> = app
        .assistant
        .pending_consents
        .iter()
        .filter(|c| !app.assistant.announced.contains(&c.call_id))
        .map(|c| c.summary.clone())
        .collect();
    let fresh_ids: Vec<String> = app
        .assistant
        .pending_consents
        .iter()
        .filter(|c| !app.assistant.announced.contains(&c.call_id))
        .map(|c| c.call_id.clone())
        .collect();
    for c in &app.assistant.pending_consents {
        app.assistant.announced.insert(c.call_id.clone());
    }
    // A new row takes focus (`PendingApprovalRow`), unless the user is typing in the composer,
    // another approval has it, or a menu is open (`focusIsTaken`).
    if let Some(id) = app
        .assistant
        .pending_consents
        .iter()
        .rev()
        .find(|c| fresh_ids.contains(&c.call_id))
        .map(|c| c.call_id.clone())
    {
        let focus = tx.focused().unwrap_or_default();
        let typing = focus.split('/').any(|seg| seg.starts_with("asst-input-"))
            && !app.assistant.draft.text.is_empty();
        let other_row = focus
            .split('/')
            .any(|seg| seg.starts_with("asst-approval-"));
        let menu = app.assistant.menu.is_some()
            || app.assistant.picker.open.is_some()
            || app.assistant.thread.consent_menu.is_some();
        if !typing && !other_row && !menu {
            tx.focus(format!("asst-approval-{id}"), true);
        }
    }
    if let Some(summary) = fresh.last()
        && app.prefs.ai_features
    {
        tx.announce(
            tr("assistant:consent.announce", &[("summary", summary)]),
            rune::LiveRegion::Polite,
        );
    }
    changed
}

#[cfg(test)]
mod dock_tray_tests {
    use super::*;

    /// The dock's composer shows the tray as soon as the next question shares more than the page.
    #[test]
    fn the_dock_shows_the_tray_when_it_shares_an_item() {
        let mut app = crate::app::tests::quiet_app();
        app.route = crate::app::Route("/reports".into());
        assert!(!bar_shares_more_than_the_page(&app));
        let book = ("w".to_owned(), "p".to_owned());
        app.assistant.focus =
            super::super::transaction_focus("tx-1", Some("2025-03-05"), false, book);
        assert!(bar_shares_more_than_the_page(&app));
    }
}
