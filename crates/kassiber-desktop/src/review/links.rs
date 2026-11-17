//! The seams between Review and the transaction detail panel (QuarantineDashboard.tsx): which
//! reading and "Save & next" queue a row opens with, what the panel's Save & next steps to
//! after the lists are re-read, and the panel's Unpair, which is the same reviewed plan/apply
//! step as a cause card's.

use serde_json::Value;

use super::model::{self, Group, Item, s};
use super::state::{self, Waiter};
use crate::app::{App, ToastTone};
use crate::i18n::t;
use crate::txdetail::{QContext, ReviewHost, ReviewSource};

/// `quarantineDetailContext`: a classified row's reading (older daemons without a category
/// give none).
pub fn item_context(item: &Item) -> Option<QContext> {
    let category = item.category.clone()?;
    Some(QContext {
        reason: item.reason.clone(),
        category,
        evidence: item.evidence().clone(),
        root_label: item.root().map(model::root_label),
    })
}

/// `quarantineGroupContext`: a cause's root, which may sit on another page.
pub fn group_context(g: &Group) -> Option<QContext> {
    g.root_transaction_id.as_ref()?;
    Some(QContext {
        reason: g.reason.clone(),
        category: g.category.clone(),
        evidence: g.evidence.clone(),
        root_label: None,
    })
}

/// The rows the page knows: what needs the user, then what waits (while that list is open).
fn known_items(app: &App) -> Vec<Item> {
    let q = &app.review.rv.q;
    let mut items = q.attention().map(|s| s.items).unwrap_or_default();
    if q.waiting_shown {
        items.extend(q.waiting_snap().map(|s| s.items).unwrap_or_default());
    }
    items
}

/// `detailContextFor`: the shown row's reading on this page, else the one its click came with.
pub fn detail_context(app: &App, id: &str, opened: Option<&QContext>) -> Option<QContext> {
    if id.is_empty() {
        return None;
    }
    known_items(app)
        .iter()
        .find(|i| i.id == id)
        .and_then(item_context)
        .or_else(|| opened.cloned())
}

/// `queueFor`: the ids "Save & next" walks for a source.
pub fn queue_for(app: &App, source: &ReviewSource) -> Vec<String> {
    let q = &app.review.rv.q;
    match source {
        ReviewSource::List => q
            .waiting_snap()
            .map(|s| s.items.iter().map(|i| i.id.clone()).collect())
            .unwrap_or_default(),
        ReviewSource::None => vec![],
        ReviewSource::Cause(key) => {
            let Some(att) = q.attention() else {
                return vec![];
            };
            let members: Vec<String> = att
                .items
                .iter()
                .filter(|i| i.group_key.as_deref() == Some(key.as_str()))
                .map(|i| i.id.clone())
                .collect();
            if !members.is_empty() {
                return members;
            }
            // Older daemons name only the cause's first roots.
            att.groups()
                .into_iter()
                .find(|g| g.key == *key)
                .map(|g| g.root_transaction_ids)
                .unwrap_or_default()
        }
    }
}

/// `openDetail` with the queue of its source.
pub fn open_row(
    app: &mut App,
    id: &str,
    tab: &str,
    context: Option<QContext>,
    source: ReviewSource,
) {
    let ids = queue_for(app, &source);
    crate::txdetail::open_for_review(app, id, tab, context, source, ids);
}

/// A cause card's row (`openRoot`): the row's reading, else the cause's; Save & next walks the
/// cause's loaded rows.
pub fn open_cause_row(app: &mut App, g: &Group, id: &str, tab: Option<&str>) {
    let item = app
        .review
        .rv
        .q
        .attention()
        .and_then(|s| s.items.into_iter().find(|i| i.id == id));
    let tab = tab.map(str::to_owned).unwrap_or_else(|| {
        let ev = item
            .as_ref()
            .map(|i| i.evidence().clone())
            .filter(Value::is_object)
            .unwrap_or_else(|| g.evidence.clone());
        model::sheet_tab(&g.reason, &g.category, &ev).to_owned()
    });
    let context = item
        .as_ref()
        .and_then(item_context)
        .or_else(|| group_context(g));
    open_row(app, id, &tab, context, ReviewSource::Cause(g.key.clone()));
}

/// A row of the waiting list, or the root it waits on (`openFromWaiting`).
pub fn open_waiting_row(app: &mut App, id: &str, tab: &str, context: Option<QContext>) {
    open_row(app, id, tab, context, ReviewSource::List);
}

/// `refreshedTarget`: the tab and reading a re-read row opens with.
fn refreshed_target(app: &App, id: &str) -> (String, Option<QContext>) {
    if let Some(item) = known_items(app).into_iter().find(|i| i.id == id) {
        let tab =
            model::sheet_tab(&item.reason, &item.category_or_derived(), item.evidence()).to_owned();
        return (tab, item_context(&item));
    }
    let group = app.review.rv.q.attention().and_then(|s| {
        s.groups()
            .into_iter()
            .find(|g| g.root_transaction_id.as_deref() == Some(id))
    });
    match group {
        Some(g) => (
            model::sheet_tab(&g.reason, &g.category, &g.evidence).to_owned(),
            group_context(&g),
        ),
        None => ("details".into(), None),
    }
}

/// After the panel saved a row the Review page opened: the page re-reads its lists (the save
/// invalidates them). With `next` ("Save & next"), the next row comes only from what they
/// return, since the save may have cleared the row or changed its reason.
pub fn after_sheet_save(app: &mut App, id: &str, next: Option<ReviewHost>) {
    match next {
        None => state::refetch_all(app),
        Some(h) => {
            app.review.rv.sheet_next = Some((id.to_owned(), h.ids, h.source));
            state::refetch(app, Some(Waiter::SheetNext));
            // Everything else the page shows is re-read too (the save's invalidation).
            state::refetch_all(app);
        }
    }
}

/// The re-read for "Save & next" landed (`ok`: the attention pages came back).
pub fn sheet_next_refreshed(app: &mut App, ok: bool) {
    let Some((id, prev, source)) = app.review.rv.sheet_next.take() else {
        return;
    };
    // The panel moved on (closed, or another row opened) while the lists were re-read.
    let still = app
        .detail
        .open
        .as_ref()
        .and_then(|o| o.id().map(str::to_owned))
        .as_deref()
        == Some(id.as_str());
    if !still {
        return;
    }
    if !ok {
        crate::txdetail::close(app);
        return;
    }
    let ids = queue_for(app, &source);
    let Some(next) = model::next_after_refresh(&id, &prev, &ids) else {
        crate::txdetail::close(app);
        return;
    };
    let (tab, context) = refreshed_target(app, &next);
    crate::txdetail::open_for_review(app, &next, &tab, context, source, ids);
}

/// `sheetPairCase`: the attention row whose pair review the shown pair is, with both legs read.
pub fn sheet_pair_case(app: &App, record: &Value) -> Option<Item> {
    let review_pair = s(&record["pair"], "reviewPairId");
    if review_pair.is_empty() {
        return None;
    }
    app.review
        .rv
        .q
        .attention()?
        .items
        .into_iter()
        .find(|i| i.pair_id() == review_pair && i.pair_legs().is_some())
}

/// Whether the page's unpair step is open (`isUnpairing = fixing !== null`).
pub fn unpairing(app: &App) -> bool {
    app.review.rv.fix.is_some()
}

/// `unpairFromSheet`: the sheet confirms the review it displays, with that reading's
/// fingerprint and book, through the page's reviewed plan/apply step; it is never swapped for a
/// newer reading the page happens to hold.
pub fn unpair_from_sheet(app: &mut App, relation_id: &str) {
    let Some(o) = app.detail.open.as_ref() else {
        return;
    };
    let Some(record) = o.record.clone() else {
        return;
    };
    let shown_scope = o.scope.clone();
    let pair = &record["pair"];
    let case = sheet_pair_case(app, &record);
    let fingerprint = s(pair, "pairFingerprint");
    let Some(case) = case.filter(|_| s(pair, "id") == relation_id && !fingerprint.is_empty())
    else {
        return refuse(app, t("journals:quarantine.fix.notACase"));
    };
    let page_scope = app.review.rv.q.attention().and_then(|s| s.book_scope());
    if shown_scope.is_none()
        || page_scope.is_none()
        || !state::scopes_match(shown_scope.as_ref(), page_scope.as_ref())
    {
        return refuse(app, t("journals:quarantine.fix.bookChanged"));
    }
    if s(case.evidence(), "pair_fingerprint") != fingerprint {
        // The page and the sheet read the pair differently: one of them is older. Nothing is
        // confirmed until both show the same reading.
        refuse(app, t("journals:quarantine.fix.sheetChanged"));
        crate::txdetail::reload(app);
        return;
    }
    crate::txdetail::close(app);
    state::start_fix(app, vec![case], shown_scope);
}

fn refuse(app: &mut App, body: String) {
    let title = t("journals:quarantine.pair.unpair");
    app.toast(
        ToastTone::Warning,
        &title,
        Some(body),
        Some("quarantine-unpair"),
    );
}

/// The Review page's "Open matching tools" from its sheet: "Link manually" on Transfers, on
/// the shown transaction.
pub fn open_pairing_review(app: &mut App) {
    let Some(o) = app.detail.open.as_ref() else {
        return;
    };
    let link = o
        .id()
        .filter(|id| !id.is_empty())
        .unwrap_or("new")
        .to_owned();
    crate::txdetail::close(app);
    app.navigate(&format!("/transfers?link={link}"));
}
