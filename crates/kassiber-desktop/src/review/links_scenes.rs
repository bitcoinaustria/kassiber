//! Scenes for the seams between Review, the detail panel, the assistant and Settings (`links-`).
//! The quarantine scenes need a book with causes, so each builds a variant of the demo book on a
//! temporary copy (parity/fixtures.json `books`): the Review-page scenes run on `coarse-review`
//! (coarse-price review required: many rows of one cause), `links-pair-*` on `residual-pair`
//! (a manual pair whose receipt is smaller than the send).

use crate::app::App;
use crate::txdetail::ReviewSource;

use super::model::{self, Group};

fn review(a: &mut App) {
    review_on(a, "coarse-review");
}

fn review_on(a: &mut App, book: &str) {
    crate::fixtures::use_book(a, book);
    a.navigate("/review");
    super::prime(a);
}

fn causes(a: &App) -> Vec<Group> {
    let Some(snap) = a.review.rv.q.attention() else {
        return vec![];
    };
    model::with_loaded_causes(snap.groups(), &snap.items)
}

/// The first cause whose rows (or pairs) are loaded, and its first row.
fn first_row(a: &App, pair: bool) -> Option<(Group, String)> {
    let snap = a.review.rv.q.attention()?;
    causes(a).into_iter().find_map(|g| {
        let rows = model::cause_rows(&g, &snap.items);
        let list = if pair { rows.pairs } else { rows.members };
        list.first().map(|i| (g.clone(), i.id.clone()))
    })
}

fn open_first(a: &mut App, pair: bool) {
    review_on(
        a,
        if pair {
            "residual-pair"
        } else {
            "coarse-review"
        },
    );
    if let Some((g, id)) = first_row(a, pair) {
        super::links::open_cause_row(a, &g, &id, None);
        crate::txdetail::settle(a);
    }
}

/// "Fix with assistant": the seed prompt parked in the composer, the panel open.
pub fn fix_assistant(a: &mut App) {
    review(a);
    let n = a
        .review
        .rv
        .q
        .attention()
        .map(|s| s.attention_count())
        .unwrap_or(0);
    crate::assistant::fix_quarantine(a, n);
    crate::assistant::prime(a);
}

pub fn fix_assistant_de(a: &mut App) {
    a.prefs.lang = crate::i18n::Lang::De;
    fix_assistant(a);
}

/// A cause card's "Ask about this": its root pinned, the quarantine question prefilled.
pub fn ask_cause(a: &mut App) {
    review(a);
    if let Some(g) = causes(a).into_iter().next()
        && let Some(root) = g
            .root_transaction_id
            .clone()
            .or_else(|| g.root_transaction_ids.first().cloned())
    {
        let date = g
            .root_occurred_at
            .clone()
            .or(g.earliest_occurred_at.clone());
        crate::assistant::ask_about(a, &root, date.as_deref(), true);
        crate::assistant::prime(a);
    }
}

/// A cause's first row in the panel, with the page's reading and its "Save & next" queue.
pub fn review_row(a: &mut App) {
    open_first(a, false);
}

/// The same with an unsaved note, so "Save & next" is enabled.
pub fn review_row_dirty(a: &mut App) {
    open_first(a, false);
    let d = a.detail.open.as_mut().map(|o| {
        o.draft.note = "Checked against the exchange statement".into();
        o.draft.clone()
    });
    if let Some(d) = d {
        crate::txdetail::sync_edits(a, &d);
    }
}

/// A row of the waiting list (Save & next walks that list).
pub fn review_waiting_row(a: &mut App) {
    review(a);
    super::state::set_waiting_shown(a, true);
    super::settle(a);
    let first = a
        .review
        .rv
        .q
        .waiting_snap()
        .and_then(|w| w.items.first().cloned());
    if let Some(it) = first {
        let tab = model::sheet_tab(&it.reason, &it.category_or_derived(), it.evidence()).to_owned();
        super::links::open_row(
            a,
            &it.id,
            &tab,
            super::links::item_context(&it),
            ReviewSource::List,
        );
        crate::txdetail::settle(a);
    }
}

/// A pair case opened from its card: the Linked tab offers Unpair.
pub fn pair_row(a: &mut App) {
    open_first(a, true);
    if let Some(o) = a.detail.open.as_mut() {
        o.tab = crate::txdetail::Tab::Linked;
    }
}

/// The panel's Unpair: the page's reviewed step, checked and previewed (no write).
pub fn pair_unpair(a: &mut App) {
    open_first(a, true);
    let pid = a
        .detail
        .open
        .as_ref()
        .and_then(|o| o.record.as_ref())
        .and_then(|r| r["pair"]["id"].as_str().map(str::to_owned));
    if let Some(pid) = pid {
        crate::txdetail::actions::unpair(a, &pid);
        super::settle(a);
    }
}

/// Settings → Lock with auto-lock on (its timer is the shell's).
pub fn auto_lock(a: &mut App) {
    a.settings.prefs.auto_lock = true;
    a.shell.settings_open = true;
    crate::settings::open_section(a, "security");
    crate::settings::prime(a);
}

/// SYNTHETIC: the idle lock armed on an encrypted workspace (the demo books are plaintext, where
/// the web never arms it): auto-lock on, 1 minute, `status.database_encrypted` set. A run on the
/// virtual clock locks at 60 s without input (`--snap --at 61`); a key restarts the countdown.
pub fn auto_lock_armed(a: &mut App) {
    a.settings.prefs.auto_lock = true;
    a.settings.prefs.idle_minutes = 1;
    a.settings.fx.started = true;
    let mut status = a
        .read("status")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    status["database_encrypted"] = serde_json::Value::Bool(true);
    a.reads.insert(
        "status".into(),
        crate::app::Reply {
            kind: "status".into(),
            data: status,
            error: None,
            error_detail: serde_json::Value::Null,
            elapsed: std::time::Duration::ZERO,
        },
    );
}
