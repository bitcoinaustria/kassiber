//! Named states for `--scene` (headless snapshots and the parity harness).

use crate::app::{App, ThemeMode};
use crate::i18n::Lang;

/// The Review page on the demo book's `coarse-review` variant (parity/fixtures.json `books`): the
/// demo book itself has nothing to review, so the cause cards, rows and price scenes need it.
fn review(a: &mut App) {
    crate::fixtures::use_book(a, "coarse-review");
    a.navigate("/review");
    super::prime(a);
}

fn clear(a: &mut App) {
    a.navigate("/review");
    super::prime(a);
    if let Some(b) = a.badges.as_mut() {
        b.quarantine = Some(0.0);
        b.quarantine_attention = Some(0.0);
        b.journals_needs_processing = false;
    }
    a.review.rv.tr.live = Some(Vec::new());
}

fn reconcile(a: &mut App, tools: bool) {
    a.navigate(if tools {
        "/reconcile?mode=tools"
    } else {
        "/reconcile"
    });
    super::prime(a);
}

/// The first cause card's key (the quarantine book's first cause).
fn first_cause(a: &App) -> Option<String> {
    let snap = a.review.rv.q.attention()?;
    super::model::with_loaded_causes(snap.groups(), &snap.items)
        .first()
        .map(|g| g.key.clone())
}

fn cause_by_reason(a: &App, reason: &str) -> Option<String> {
    let snap = a.review.rv.q.attention()?;
    super::model::with_loaded_causes(snap.groups(), &snap.items)
        .into_iter()
        .find(|g| g.reason == reason)
        .map(|g| g.key)
}

pub const SCENES: &[crate::app::Scene] = &[
    ("review", review),
    // Nothing to review: the centred empty state (the plain demo book, kept clear synthetically).
    ("review-clear", clear),
    ("review-clear-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        clear(a);
    }),
    ("review-clear-de", |a| {
        a.prefs.lang = Lang::De;
        clear(a);
    }),
    ("review-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        review(a);
    }),
    ("review-de", |a| {
        a.prefs.lang = Lang::De;
        review(a);
    }),
    // Every cause card unfolded (rows shown past the first three).
    ("review-expanded", |a| {
        review(a);
        // As the parity recipe does: the first card's "Show more".
        if let Some(k) = first_cause(a) {
            super::state::card_mut(a, &k).expanded = true;
        }
    }),
    // The waiting list open.
    ("review-waiting", |a| {
        review(a);
        super::state::set_waiting_shown(a, true);
        super::settle(a);
    }),
    // The approximate-price card's exact prices being typed.
    ("review-prices", |a| {
        review(a);
        if let Some(k) = cause_by_reason(a, "pricing_review_required").or_else(|| first_cause(a)) {
            super::state::card_mut(a, &k).entering = true;
        }
    }),
    // The accept-prices confirmation with the daemon's preview (cases + plan: reads only).
    ("review-accept", |a| {
        review(a);
        let scope = a.review.rv.q.attention().and_then(|s| s.book_scope());
        if let Some(scope) = scope {
            super::state::start_proposal(
                a,
                super::state::PropKind::Accept,
                scope,
                vec![
                    serde_json::json!({ "type": "accept_coarse_prices", "reason": super::model::ACCEPT_PRICES_REASON }),
                ],
            );
            super::settle(a);
        }
    }),
    ("review-accept-de", |a| {
        a.prefs.lang = Lang::De;
        review(a);
        let scope = a.review.rv.q.attention().and_then(|s| s.book_scope());
        if let Some(scope) = scope {
            super::state::start_proposal(
                a,
                super::state::PropKind::Accept,
                scope,
                vec![
                    serde_json::json!({ "type": "accept_coarse_prices", "reason": super::model::ACCEPT_PRICES_REASON }),
                ],
            );
            super::settle(a);
        }
    }),
    // The first cause's first row open in the detail panel.
    ("review-open-row", |a| {
        review(a);
        let id = a.review.rv.q.attention().and_then(|s| {
            s.items.first().map(|i| {
                (
                    i.id.clone(),
                    super::model::sheet_tab(&i.reason, &i.category_or_derived(), i.evidence())
                        .to_owned(),
                )
            })
        });
        if let Some((id, tab)) = id {
            crate::txdetail::open(a, &id, &tab);
            crate::txdetail::settle(a);
        }
    }),
    // The seams with the assistant, the detail panel and Settings (links_scenes.rs).
    ("links-fix-assistant", super::links_scenes::fix_assistant),
    (
        "links-fix-assistant-de",
        super::links_scenes::fix_assistant_de,
    ),
    ("links-ask-cause", super::links_scenes::ask_cause),
    ("links-review-row", super::links_scenes::review_row),
    (
        "links-review-row-dirty",
        super::links_scenes::review_row_dirty,
    ),
    (
        "links-review-waiting-row",
        super::links_scenes::review_waiting_row,
    ),
    ("links-pair-row", super::links_scenes::pair_row),
    ("links-pair-unpair", super::links_scenes::pair_unpair),
    ("links-auto-lock", super::links_scenes::auto_lock),
    (
        "links-auto-lock-armed",
        super::links_scenes::auto_lock_armed,
    ),
    // Suggested links: a row's menu open, two rows ticked for "Set 2 aside". On a book with
    // suggestions (e.g. the delta parity book) they are its own; the demo book has none, so the
    // synthetic list below stands in.
    ("review-links-menu", |a| {
        review(a);
        if first_candidates(a, 1).is_empty() {
            synthetic_links(a);
        }
        if let Some(k) = first_candidates(a, 1).pop() {
            a.review.popup = Some(super::Popup::MoveMenu(k));
        }
    }),
    ("review-links-ticked", |a| {
        review(a);
        if first_candidates(a, 2).is_empty() {
            synthetic_links(a);
        }
        for k in first_candidates(a, 2) {
            super::state::toggle_selected(a, &k);
        }
    }),
    // The same list on any book: one-click links, a likely match, a rule's terms, a swap.
    ("review-links-synthetic", |a| {
        review(a);
        synthetic_links(a);
    }),
    ("review-links-synthetic-de", |a| {
        a.prefs.lang = Lang::De;
        review(a);
        synthetic_links(a);
    }),
    ("review-links-synthetic-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        review(a);
        synthetic_links(a);
        if let Some(k) = first_candidates(a, 1).pop() {
            a.review.popup = Some(super::Popup::MoveMenu(k));
        }
    }),
    // A held send whose automatic link the owner removed: "Link it back".
    ("review-cause-unlinked", |a| {
        review(a);
        synthetic_unlinked(a);
    }),
    ("review-cause-unlinked-de", |a| {
        a.prefs.lang = Lang::De;
        review(a);
        synthetic_unlinked(a);
    }),
    ("review-reconcile", |a| reconcile(a, false)),
    ("review-reconcile-tools", |a| reconcile(a, true)),
    ("review-reconcile-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        reconcile(a, false);
    }),
    ("review-reconcile-de", |a| {
        a.prefs.lang = Lang::De;
        reconcile(a, false);
    }),
    ("review-reconcile-tools-de", |a| {
        a.prefs.lang = Lang::De;
        reconcile(a, true);
    }),
    // A synthetic CoinTracking export compared with the book (`ui.reports.cross_check`, local).
    ("review-reconcile-tools-loaded", |a| {
        reconcile(a, true);
        super::tools::scene_load(a, "cointracking-export.csv", CT_SAMPLE.as_bytes());
        super::settle(a);
    }),
    ("review-reconcile-tools-loaded-light", |a| {
        a.prefs.theme = ThemeMode::Light;
        reconcile(a, true);
        super::tools::scene_load(a, "cointracking-export.csv", CT_SAMPLE.as_bytes());
        super::settle(a);
    }),
    // A file the tool cannot read (its columns are not CoinTracking's): the failed state.
    ("review-reconcile-tools-failed", |a| {
        reconcile(a, true);
        super::tools::scene_load(
            a,
            "cointracking-export.csv",
            CT_SAMPLE.replace("Buy Amount", "Buy").as_bytes(),
        );
        super::settle(a);
    }),
    // A pasted list checked locally (`ui.wallets.identify`, no network).
    ("review-reconcile-checked", |a| {
        reconcile(a, false);
        a.review.rc.input = rune::TextEdit::new(SAMPLE);
        super::reconcile::check_now(a);
        super::settle(a);
    }),
];

fn first_candidates(a: &App, n: usize) -> Vec<String> {
    a.review
        .rv
        .tr
        .candidates()
        .iter()
        .take(n)
        .map(super::model::candidate_key)
        .collect()
}

fn synthetic_links(a: &mut App) {
    use serde_json::json;
    let c = |id: &str, from: &str, to: &str, extra: serde_json::Value| {
        let mut v = json!({
            "out_id": format!("out-{id}"), "in_id": format!("in-{id}"),
            "out_wallet_label": from, "in_wallet_label": to, "out_wallet_kind": "descriptor", "in_wallet_kind": "descriptor",
            "out_asset": "BTC", "in_asset": "BTC", "out_amount_msat": 5_000_000_000_i64, "in_amount_msat": 5_000_000_000_i64, "swap_fee_msat": 0,
            "out_occurred_at": "2025-03-10T09:00:00Z", "in_occurred_at": "2025-03-10T09:12:00Z",
            "confidence": "exact", "method": "txid", "conflict_size": 1, "conflict_set_id": format!("set-{id}"),
            "candidate_type": "transfer", "default_kind": "manual", "default_policy": "carrying-value",
        });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    let list = vec![
        c(
            "a",
            "Regtest Cold Storage",
            "Regtest Spending Wallet",
            json!({}),
        ),
        c(
            "b",
            "Regtest Spending Wallet",
            "Regtest Cold Storage",
            json!({"out_amount_msat": 2_000_000_000_i64, "in_amount_msat": 1_999_000_000_i64, "swap_fee_msat": 1_000_000, "confidence": "strong", "method": "heuristic", "in_occurred_at": "2025-03-10T14:00:00Z"}),
        ),
        c(
            "c",
            "Regtest Cold Storage",
            "Regtest Liquid Treasury",
            json!({"in_asset": "LBTC", "in_wallet_kind": "liquid", "default_kind": "peg-in", "candidate_type": "transfer", "rule_match": {"rule_id": "r1", "rule_name": "Pegs", "kind": "peg-in", "policy": "carrying-value"}}),
        ),
        c(
            "d",
            "Regtest Spending Wallet",
            "Regtest Treasury",
            json!({"conflict_size": 2}),
        ),
    ];
    let tr = &mut a.review.rv.tr;
    tr.live = Some(list);
    tr.frozen = None;
    tr.loading = false;
    tr.failed = false;
}

fn synthetic_unlinked(a: &mut App) {
    use serde_json::json;
    let actions = json!([
        {"kind": "restore_pairing", "dismissal_id": "dismissal-9", "out_transaction_id": "out-bark", "in_transaction_id": "in-store"},
        {"kind": "pair_transfer", "transaction_id": "out-bark"},
        {"kind": "sync_wallet", "wallet_id": "w-bark", "wallet_label": "BTCPay Bark"}
    ]);
    let reason = "recorded_transfer_authority_conflict";
    let scope = a.expected_scope();
    let page = json!({
        "summary": {
            "count": 1, "attention_count": 1, "scope_count": 1, "waiting_count": 0, "offset": 0,
            "workspace_id": scope["workspace_id"], "profile_id": scope["profile_id"], "fiat_currency": "EUR",
            "freshness": {"needs_processing": false},
            "groups": [{"key": "unlinked-1", "category": "needs_decision", "reason": reason, "root_transaction_id": "out-bark",
                "root_occurred_at": "2026-11-13T11:52:00Z", "root_wallet": "BTCPay Bark", "root_asset": "BTC", "downstream_count": 0,
                "blocks_reports": true, "evidence": {}, "actions": actions, "root_transaction_ids": ["out-bark"], "root_count": 1}],
        },
        "items": [{"transaction_id": "out-bark", "external_id": "6bb93c0819b689af5cc948d718c3978671d832ff764c6ab73702e5d97edce2ea",
            "occurred_at": "2026-11-13T11:52:00Z", "wallet": "BTCPay Bark", "direction": "outbound", "asset": "BTC",
            "amount": -0.006, "amount_msat": -600_000_000, "reason": reason, "category": "needs_decision", "is_downstream": false,
            "blocks_reports": true, "group_key": "unlinked-1", "actions": actions, "evidence": {}}],
    });
    // The side-nav count the daemon would give for it (no "nothing to review" banner).
    if let Some(b) = a.badges.as_mut() {
        b.quarantine = Some(1.0);
        b.quarantine_attention = Some(1.0);
    }
    let q = &mut a.review.rv.q;
    q.pages = vec![page];
    q.loading = false;
    q.error = None;
}

/// Three entries: a txid of the demo book, an address that is not the book's, and junk.
const SAMPLE: &str = "bffdcc622b5aa58df0188c829769a9ea26b7921cf6b1ca26ce2015efcec5cb11\nbcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080\nnot-an-address";

/// A made-up CoinTracking export: a buy and a withdrawal on the demo book's wallets, one deposit
/// the book does not have.
const CT_SAMPLE: &str = "\"Type\",\"Buy Amount\",\"Buy Currency\",\"Sell Amount\",\"Sell Currency\",\"Fee\",\"Fee Currency\",\"Exchange\",\"Trade-Group\",\"Comment\",\"Date\"
\"Trade\",\"0.00534123\",\"BTC\",\"472.28\",\"EUR\",\"\",\"\",\"Regtest Spending Wallet\",\"\",\"\",\"09.11.2025 16:08:00\"
\"Withdrawal\",\"\",\"\",\"0.01019126\",\"BTC\",\"\",\"\",\"Regtest Spending Wallet\",\"\",\"\",\"09.11.2025 11:54:00\"
\"Deposit\",\"0.25\",\"BTC\",\"\",\"\",\"\",\"\",\"Regtest Cold Storage\",\"\",\"\",\"01.03.2024 10:00:00\"
";
