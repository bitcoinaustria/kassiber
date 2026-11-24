//! SYNTHETIC sample data for scenes. Sending is disabled in this build and the demo book keeps
//! no saved chats, so these fixtures stand in for a conversation, staged files and saved chats
//! to compare the rendering. Nothing here is sent anywhere or written to the book; ids, amounts
//! and labels are invented (regtest-flavoured, BTC only).

use serde_json::{Value, json};

use crate::app::App;
use crate::assistant::model::{
    AttachmentView, ChatFile, Consent, FileState, Msg, MsgStatus, Role, SessionList, SessionRow,
    ThinkingSegment, ToolCall, ToolStatus,
};

fn call(id: &str, name: &str, status: ToolStatus, args: Value, result: Option<Value>) -> ToolCall {
    ToolCall {
        call_id: id.into(),
        name: name.into(),
        arguments: args,
        kind_class: if name.contains("apply") || name.contains("update") {
            "mutating".into()
        } else {
            "read_only".into()
        },
        needs_consent: false,
        status,
        approval: None,
        result,
        reason: None,
        review_preview: None,
        segment_id: None,
    }
}

fn seg(id: &str, content: &str) -> ThinkingSegment {
    ThinkingSegment {
        id: id.into(),
        content: content.into(),
    }
}

fn assistant(id: &str, content: &str) -> Msg {
    Msg::new(id, Role::Assistant, content)
}

const DIGEST: &str = "3f9a1c0b7e2d4a6f8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";
const REQUEST: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

fn effects(q: u64) -> Value {
    json!({"entries_count": 412, "quarantine_count": q, "report_ready": q == 0,
        "quarantines": if q > 0 { json!([{"transaction_id": "tx-2025-0311-kraken", "reason": "missing_price"}]) } else { json!([]) }})
}

fn visual(
    id: &str,
    source: &str,
    measure: &str,
    unit: Value,
    extra: Value,
    data: Value,
) -> ToolCall {
    let mut v = json!({"version": 1, "source": source, "measure": measure, "unit": unit, "fiat_currency": "EUR", "as_of": "2026-11-13T00:00:00Z", "filters": {}});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    let mut d = data;
    d["visual"] = v;
    call(
        id,
        "ui.visuals.render",
        ToolStatus::Done,
        json!({"source": source}),
        Some(json!({"kind": "ui.visuals.render", "data": d})),
    )
}

/// The basic turn: a question with a file, reasoning, routine and notable tool calls, a
/// readiness visual, a markdown answer with facts, a notice and the provenance line.
fn basics() -> Vec<Msg> {
    let mut u1 = Msg::new(
        "u1",
        Role::User,
        "Is the book ready for the 2025 report? I attached the Kraken export.",
    );
    u1.attachments = vec![AttachmentView {
        filename: "kraken-ledger-2025.csv".into(),
        kind: Some("delimited".into()),
        size_bytes: Some(48_213),
    }];

    let mut a1 = assistant(
        "a1",
        "## Report readiness for 2025\n\nThe book is **almost ready**. Two things block the report:\n\n1. One Kraken withdrawal has no price (`tx-2025-0311-kraken`).\n2. Journals were processed before the last import.\n\n| Check | Status | Count |\n| --- | --- | ---: |\n| Quarantine | Needs attention | 1 |\n| Prices | Needs attention | 1 |\n| Custody | OK | 0 |\n\nTo fix the price, set it in the transaction's **Pricing** tab, then process journals:\n\n```sh\nkassiber journals process\n```\n\n> Supplying a price does not settle the case by itself; the journal is rebuilt and checked again.\n\nSee [Reports](/reports) for the readiness checklist, or the [Austrian handoff guide](https://kassiber.org/docs/austrian-handoff).",
    );
    a1.thinking_segments = vec![
        seg(
            "s1",
            "The user asks about **report readiness**. I should read the blockers, the quarantine and the rate coverage before answering.",
        ),
        seg(
            "s2",
            "One blocker is a missing price. The metadata fix was applied automatically; the backend listing failed, which does not matter for the answer.",
        ),
    ];
    let mut q = call(
        "c1",
        "ui.journals.quarantine",
        ToolStatus::Done,
        json!({"limit": 20}),
        Some(json!({"kind": "ui.journals.quarantine", "data": {"summary": {"count": 1}}})),
    );
    q.segment_id = Some("s1".into());
    let mut rc = call(
        "c2",
        "ui.rates.coverage",
        ToolStatus::Done,
        json!({}),
        Some(
            json!({"kind": "ui.rates.coverage", "data": {"summary": {"missing_price_transactions": 1, "cache_coverable_missing": 0}}}),
        ),
    );
    rc.segment_id = Some("s1".into());
    let mut rb = call(
        "c3",
        "ui.report.blockers",
        ToolStatus::Done,
        json!({"year": 2025}),
        Some(
            json!({"kind": "ui.report.blockers", "data": {"ready": false, "blockers": [{"title": "Missing price"}, {"title": "Journals outdated"}]}}),
        ),
    );
    rb.segment_id = Some("s1".into());
    let mut upd = call(
        "c4",
        "ui.transactions.metadata.update",
        ToolStatus::Done,
        json!({"transaction": "tx-2025-0311-kraken", "note": "Kraken withdrawal to cold storage"}),
        Some(
            json!({"kind": "ui.transactions.metadata.update", "data": {"updated": true, "undo": {"tool": "ui.transactions.metadata.update", "arguments": {"note": null}}, "local_reference": {"route": "/transactions", "transaction": "tx-2025-0311-kraken"}}}),
        ),
    );
    upd.approval = Some("auto".into());
    upd.segment_id = Some("s2".into());
    let mut fail = call("c5", "ui.backends.list", ToolStatus::Error, json!({}), None);
    fail.reason =
        Some("backend_unreachable: the daemon could not read the backend settings".into());
    fail.segment_id = Some("s2".into());
    let mut denied = call(
        "c6",
        "ui.freshness.run",
        ToolStatus::Denied,
        json!({"all": true}),
        None,
    );
    denied.reason = Some("Declined by the user.".into());
    denied.segment_id = Some("s2".into());
    let ready = visual(
        "c7",
        "readiness",
        "count",
        json!({"kind": "count"}),
        json!({"title": "Ready for the 2025 report?"}),
        json!({"summary": {"ready": false, "passed": 4, "total": 6}, "caveats": [{"code": "journals_stale"}], "rows": [
            {"check": "transactions", "status": "ok", "count": 393},
            {"check": "journals", "status": "warning", "detail": "2026-11-02T09:00:00Z", "action": "process_journals"},
            {"check": "quarantine", "status": "blocked", "count": 1, "action": "review_quarantine"},
            {"check": "prices", "status": "warning", "count": 1, "detail": "0", "action": "fill_prices"},
            {"check": "custody", "status": "ok", "count": 0},
            {"check": "review_queue", "status": "ok", "count": 0}
        ]}),
    );
    a1.tool_calls = vec![q, rc, rb, upd, fail, denied, ready];
    a1.notices = vec!["fast_mode_unavailable".into()];
    a1.provenance = Some(json!({
        "generated_at": "2026-11-14T16:42:00Z",
        "tools_used": ["ui.journals.quarantine", "ui.rates.coverage", "ui.report.blockers", "ui.transactions.metadata.update"],
        "active_transactions": 393, "quarantines": 1, "missing_price_transactions": 1,
        "journals_processed_at": "2026-11-02T09:00:00Z",
        "privacy_receipt": {"remote_provider": false, "advertised_tool_count": 42, "tools_denied": 1}
    }));
    vec![u1, a1]
}

/// Every visual source, drawn from invented figures.
fn visuals() -> Vec<Msg> {
    let u = Msg::new(
        "u2",
        Role::User,
        "Give me an overview, then my balance, holdings, tax years, cash flow and largest transactions.",
    );
    let mut a = assistant(
        "a2",
        "Here are your books at a glance. Every figure above was drawn by Kassiber from the local book.",
    );
    let fiat = json!({"kind": "fiat", "code": "EUR"});
    let series: Vec<Value> = (0..12).map(|i| json!({"date": format!("2025-{:02}-01", i + 1), "balance_btc": 90.0 + i as f64 * 0.8, "market_value": 3_900_000.0 + (i as f64 * 0.7).sin() * 600_000.0 + i as f64 * 150_000.0, "cost_basis": 2_900_000.0 + i as f64 * 40_000.0})).collect();
    let overview = visual(
        "v1",
        "overview",
        "market_value",
        fiat.clone(),
        json!({}),
        json!({
            "stats": {"portfolio_value": 5_812_000.0, "btc_balance": 98.584, "cost_basis": 3_331_000.0, "unrealized": 2_481_000.0, "realized_ytd": 18_420.5, "btc_price": 58_968.9, "price_at": "2026-11-13T00:00:00Z", "basis_complete": true, "transactions": 393},
            "series": series,
            "wallets": [{"label": "Regtest Cold Storage", "balance_btc": 61.2, "share": 0.62}, {"label": "Regtest Spending Wallet", "balance_btc": 22.4, "share": 0.227}, {"label": null, "other_count": 9, "balance_btc": 14.98, "share": 0.153}]
        }),
    );
    let bal_rows: Vec<Value> = (0..12).map(|i| json!({"period": format!("2025-{:02}-01", i + 1), "asset": "BTC", "quantity": 88.0 + i as f64 * 0.9, "market_value": 3_700_000.0 + i as f64 * 180_000.0, "cost_basis": 2_900_000.0 + i as f64 * 40_000.0})).collect();
    let balance = visual(
        "v2",
        "balance_history",
        "quantity",
        json!({"kind": "asset"}),
        json!({"interval": "month"}),
        json!({"rows": bal_rows}),
    );
    let holdings = visual(
        "v3",
        "portfolio",
        "market_value",
        fiat.clone(),
        json!({"group_by": "wallet"}),
        json!({"rows": [
            {"wallet": "Regtest Cold Storage", "asset": "BTC", "quantity": 61.2, "market_value": 3_608_896.0, "cost_basis": 1_980_000.0, "unrealized_pnl": 1_628_896.0},
            {"wallet": "Regtest Spending Wallet", "asset": "BTC", "quantity": 22.4, "market_value": 1_320_903.0, "cost_basis": 902_000.0, "unrealized_pnl": 418_903.0},
            {"wallet": "Regtest Liquid", "asset": "L-BTC", "quantity": 9.98, "market_value": 588_509.0, "cost_basis": 330_000.0, "unrealized_pnl": 258_509.0},
            {"wallet": "Regtest Lightning", "asset": "BTC", "quantity": 5.0, "market_value": 294_844.0, "cost_basis": null, "unrealized_pnl": null}
        ]}),
    );
    let tax = visual(
        "v4",
        "tax_summary",
        "gain_loss",
        fiat.clone(),
        json!({}),
        json!({
            "rows": [{"year": 2022, "proceeds": 41_000.0, "cost_basis": 52_000.0, "gain_loss": -11_000.0}, {"year": 2023, "proceeds": 88_000.0, "cost_basis": 61_000.0, "gain_loss": 27_000.0}, {"year": 2024, "proceeds": 126_500.0, "cost_basis": 70_200.0, "gain_loss": 56_300.0}, {"year": 2025, "proceeds": 64_900.0, "cost_basis": 46_480.0, "gain_loss": 18_420.0}],
            "details": [{"year": 2025, "asset": "BTC", "transaction_type": "sell", "capital_gains_type": "long", "proceeds": 50_000.0, "cost_basis": 33_000.0, "gain_loss": 17_000.0, "quantity": 0.9}, {"year": 2025, "asset": "BTC", "transaction_type": "spend", "capital_gains_type": "short", "proceeds": 14_900.0, "cost_basis": 13_480.0, "gain_loss": 1_420.0, "quantity": 0.25}]
        }),
    );
    let cf_rows: Vec<Value> = (0..6).map(|i| json!({"period": format!("2025-{:02}-01", i + 7), "bucket": "month", "in_btc": 0.4 + i as f64 * 0.15, "out_btc": 0.2 + (i % 3) as f64 * 0.2, "in_fiat": 24_000.0 + i as f64 * 9_000.0, "out_fiat": 12_000.0 + (i % 3) as f64 * 11_000.0, "in_count": 3 + i, "out_count": 2, "transfer_count": 1, "swap_count": 0, "missing_prices": 0})).collect();
    let flow_stats = |btc: f64, fiat: f64, count: u64| json!({"count": count, "btc": btc, "fiat": fiat, "missing_prices": 0, "largest": null});
    let cash = visual(
        "v5",
        "cash_flow",
        "fiat_value",
        fiat.clone(),
        json!({"period": "6months", "bucket": "month"}),
        json!({"rows": cf_rows, "totals": {"incoming": flow_stats(4.65, 279_000.0, 33), "outgoing": flow_stats(2.4, 138_000.0, 12), "transfers": flow_stats(0.0, 0.0, 6), "swaps": flow_stats(0.0, 0.0, 1), "review": 0, "pending": 1}}),
    );
    let txs = visual(
        "v6",
        "transactions",
        "quantity",
        json!({"kind": "sats"}),
        json!({"sort": "largest"}),
        json!({"summary": {"shown": 3, "total": 393, "in_sat": 1_534_123_000.0, "out_sat": 735_117_800.0}, "rows": [
            {"id": "tx-1", "date": "2025-11-09T16:08:00Z", "direction": "in", "type": "deposit", "asset": "BTC", "amount_sat": 1_234_567_000.0, "fee_sat": 0, "fiat_value": 72_812.0, "wallet": "Regtest Cold Storage", "counterparty": "Kraken", "note": "", "status": "completed", "tags": ["exchange"]},
            {"id": "tx-2", "date": "2025-11-09T15:18:00Z", "direction": "out", "type": "withdrawal", "asset": "BTC", "amount_sat": -735_117_800.0, "fee_sat": 1_200, "fiat_value": -43_350.0, "wallet": "Regtest Treasury", "counterparty": "", "note": "Treasury rotation", "status": "pending", "tags": []},
            {"id": "tx-3", "date": "2025-10-02T08:00:00Z", "direction": "in", "type": "income", "asset": "BTC", "amount_sat": 299_556_000.0, "fee_sat": 0, "fiat_value": 17_660.0, "wallet": "Regtest Spending Wallet", "counterparty": "Mining pool", "note": "", "status": "completed", "tags": ["income"]}
        ]}),
    );
    let flow = visual(
        "v7",
        "transaction_flow",
        "quantity",
        json!({"kind": "sats"}),
        json!({}),
        json!({"rows": [
        {"side": "input", "label": "Regtest Spending Wallet", "label_kind": "wallet", "position": 0, "ownership": "owned", "value_sats": 50_000_000},
        {"side": "input", "label": "input", "label_kind": "generic", "position": 1, "ownership": "unknown", "value_sats": 12_000_000},
        {"side": "output", "label": "output", "label_kind": "generic", "position": 0, "ownership": "external", "value_sats": 40_000_000},
        {"side": "output", "label": "Regtest Spending Wallet", "label_kind": "wallet", "position": 1, "ownership": "owned", "value_sats": 21_998_500},
        {"side": "fee", "label": "fee", "label_kind": "fee", "position": 0, "ownership": "unknown", "value_sats": 1_500}
    ], "transaction": {"id": "tx-2", "date": "2025-11-09T15:18:00Z", "direction": "out", "type": "withdrawal", "asset": "BTC", "amount_sat": -40_001_500, "fee_sat": 1_500, "fiat_value": -23_588.0, "wallet": "Regtest Spending Wallet", "counterparty": "Merchant", "note": "", "status": "completed", "tags": []},
       "local_reference": {"route": "/transactions", "transaction": "tx-2"}}),
    );
    let drawing = call(
        "v8",
        "ui.visuals.render",
        ToolStatus::Running,
        json!({"source": "cash_flow"}),
        None,
    );
    a.tool_calls = vec![overview, balance, holdings, tax, cash, txs, flow, drawing];
    vec![u, a]
}

/// A review plan with its proposal card, evidence and connection requests, a continuation
/// packet, an error, a stopped answer, and an answer waiting on two approvals.
fn approvals() -> (Vec<Msg>, Vec<Consent>) {
    let long = "Please go through every quarantined transaction from March and tell me what is missing.\n\nContext:\n- The Kraken export covers January to June.\n- I moved coins to cold storage twice.\n- Some withdrawals were to my own Liquid wallet.\n- The bank statement for March is attached separately.\n- One deposit came from a friend paying back a loan.\n- I am not sure about the fee on the second rotation.\n- The spending wallet was restored from seed in April.\n- Ignore the test transactions on regtest.";
    let u3 = Msg::new("u3", Role::User, long);
    let artifact = json!({
        "schema_version": 1, "workspace_id": "ws-demo", "profile_id": "pf-full", "base_input_version": 41, "digest": DIGEST,
        "operations": [
            {"type": "price_override", "transaction_id": "tx-2025-0311-kraken", "fiat_rate": "58210.40", "reason": "Kraken's own trade price on that day."},
            {"type": "pair", "out_transaction_id": "tx-2025-0402-out", "in_transaction_id": "tx-2025-0402-in", "pair_kind": "transfer", "reason": "Same amount minus the network fee, 14 minutes apart."}
        ],
        "before": effects(2), "after": effects(1),
        "delta": {"years": [{"year": 2025, "realized_gain_loss": "-1240.50", "income": "0", "estimated_tax_eur_cents": -34114}]}
    });
    let mut after = artifact["after"].clone();
    after["tax"] = json!({"fiat_currency": "EUR", "jurisdiction": "AT", "years": [{"year": 2025, "realized_gain_loss": "17180.00", "income": "0"}]});
    let mut artifact = artifact;
    artifact["after"] = after;
    let mut a3 = assistant(
        "a3",
        "I prepared corrections for two of the March cases. The third needs evidence I can't read from the book.\n\n```json\n{\"review_checkpoint\": {\"workspace_id\": \"ws-demo\", \"profile_id\": \"pf-full\", \"input_version\": 41, \"next_cursor\": null}}\n```",
    );
    let cases = call(
        "r1",
        "ui.review.cases",
        ToolStatus::Done,
        json!({"limit": 10}),
        Some(
            json!({"kind": "ui.review.cases", "data": {"cases": [{}, {}, {}], "next_cursor": null}}),
        ),
    );
    let plan = call(
        "r2",
        "ui.review.plan",
        ToolStatus::Done,
        json!({"operations": 2}),
        Some(json!({"kind": "ui.review.plan", "data": artifact})),
    );
    let evidence = call(
        "r3",
        "ui.review.request_input",
        ToolStatus::Done,
        json!({"action": "import_history"}),
        Some(json!({"kind": "ui.review.request_input", "data": {
            "schema_version": 1, "request_id": REQUEST, "workspace_id": "ws-demo", "profile_id": "pf-full", "input_version": 41, "action": "import_history",
            "explanation": null, "cases": [{"case_id": "quarantine:tx-2025-0320-in", "transaction_id": "tx-2025-0320-in", "reason": "unknown_source"}]
        }})),
    );
    let wallet = call(
        "r4",
        "ui.wallets.request_input",
        ToolStatus::Done,
        json!({"source": "liquid-descriptor"}),
        Some(json!({"kind": "ui.wallets.request_input", "data": {
            "schema_version": 1, "action": "connect_wallet", "desktop_only": true, "request_id": REQUEST, "workspace_id": "ws-demo", "profile_id": "pf-full", "source": "liquid-descriptor", "explanation": null
        }})),
    );
    a3.tool_calls = vec![cases, plan, evidence, wallet];

    let u4 = Msg::new("u4", Role::User, "Check the backends again.");
    let mut a4 = assistant("a4", "");
    a4.status = MsgStatus::Error;
    a4.error_message = Some("The model provider did not answer in time.".into());
    a4.error_code = Some("provider_timeout".into());

    let u5 = Msg::new("u5", Role::User, "Summarize the custody gaps.");
    let mut a5 = assistant("a5", "There are no open custody gaps in the");
    a5.status = MsgStatus::Cancelled;

    let u6 = Msg::new(
        "u6",
        Role::User,
        "Import the attached Kraken file into the exchange connection, then refresh.",
    );
    let mut a6 = assistant("a6", "");
    a6.status = MsgStatus::Streaming;
    a6.thinking_segments = vec![seg(
        "s6",
        "The import plan is ready; it needs the user's approval before anything is written.",
    )];
    let mut imp = call(
        "p1",
        "ui.wallets.import_attachment.apply",
        ToolStatus::AwaitingConsent,
        json!({"attachment": "file-1"}),
        None,
    );
    imp.needs_consent = true;
    imp.segment_id = Some("s6".into());
    let mut sync = call(
        "p2",
        "ui.freshness.run",
        ToolStatus::AwaitingConsent,
        json!({"all": true, "journals": true}),
        None,
    );
    sync.needs_consent = true;
    sync.segment_id = Some("s6".into());
    a6.tool_calls = vec![imp, sync];
    let consents = vec![
        Consent {
            call_id: "p1".into(),
            name: "ui.wallets.import_attachment.apply".into(),
            summary: "Import 42 transactions from kraken-ledger-2025.csv into Kraken".into(),
            arguments_preview: json!({"status": "ready", "plan": {
                "target": {"label": "Kraken", "action": "existing"}, "rows": {"to_import": 42, "already_present": 3, "would_update_existing": 0},
                "date_range": {"first": "2025-01-04", "last": "2025-06-28"},
                "totals": [{"asset": "BTC", "direction": "inbound", "rows": 30, "amount_msat": 412_345_678_000_i64, "fee_msat": 0}, {"asset": "BTC", "direction": "outbound", "rows": 12, "amount_msat": 205_000_000_000_i64, "fee_msat": 12_000_000}],
                "overlap": {"existing_rows_in_date_range": 3, "external_ids_in_other_wallets": 0},
                "warnings": ["rows_already_present", "type_map_decides_tax_kind"], "blockers": []
            }}),
            review_preview: None,
            reversible: true,
            egresses: false,
            can_allow_session: false,
        },
        Consent {
            call_id: "p2".into(),
            name: "ui.freshness.run".into(),
            summary: "Refresh every connection from its configured backend".into(),
            arguments_preview: json!({"all": true, "journals": true}),
            review_preview: None,
            reversible: true,
            egresses: true,
            can_allow_session: true,
        },
    ];
    (vec![u3, a3, u4, a4, u5, a5, u6, a6], consents)
}

/// The common changes as native cards: a link and an edit waiting for Apply, and finished
/// changes with and without an Undo the desktop can send (results shaped as the daemon's, with
/// their `undo` hints).
fn actions() -> (Vec<Msg>, Vec<Consent>) {
    let u7 = Msg::new(
        "u7",
        Role::User,
        "Link the Kraken withdrawal to my cold wallet deposit, and note that it went to cold storage.",
    );
    let mut a7 = assistant("a7", "");
    a7.status = MsgStatus::Streaming;
    a7.thinking_segments = vec![seg(
        "s7",
        "One candidate matches: same amount less the network fee, 14 minutes apart.",
    )];
    let pair_args = json!({"tx_out": "tx-2025-0402-out", "tx_in": "tx-2025-0402-in", "kind": "manual", "policy": "carrying-value"});
    let edit_args = json!({"transaction": "tx-2025-0402-out", "note": "Kraken withdrawal to cold storage", "tags": ["cold-storage"], "reason": "The owner said where it went."});
    let mut pair = call(
        "x1",
        "ui.transfers.pair",
        ToolStatus::AwaitingConsent,
        pair_args.clone(),
        None,
    );
    pair.needs_consent = true;
    pair.segment_id = Some("s7".into());
    let mut edit = call(
        "x2",
        "ui.transactions.metadata.update",
        ToolStatus::AwaitingConsent,
        edit_args.clone(),
        None,
    );
    edit.needs_consent = true;
    edit.segment_id = Some("s7".into());
    a7.tool_calls = vec![pair, edit];
    let consents = vec![
        Consent {
            call_id: "x1".into(),
            name: "ui.transfers.pair".into(),
            summary: "Pair transfer legs tx-2025-0402-out -> tx-2025-0402-in".into(),
            arguments_preview: pair_args,
            review_preview: None,
            reversible: true,
            egresses: false,
            can_allow_session: true,
        },
        Consent {
            call_id: "x2".into(),
            name: "ui.transactions.metadata.update".into(),
            summary: "Update transaction metadata tx-2025-0402-out".into(),
            arguments_preview: edit_args,
            review_preview: None,
            reversible: true,
            egresses: false,
            can_allow_session: true,
        },
    ];

    let u8 = Msg::new(
        "u8",
        Role::User,
        "Stop suggesting the March pair, and link the April one.",
    );
    let mut a8 = assistant(
        "a8",
        "Done: the April transfer is linked and the March suggestion is hidden for 30 days.",
    );
    let mut linked = call(
        "y1",
        "ui.transfers.pair",
        ToolStatus::Done,
        json!({"tx_out": "tx-2025-0402-out", "tx_in": "tx-2025-0402-in"}),
        Some(
            json!({"kind": "ui.transfers.pair", "data": {"id": "pair-0402", "undo": {"tool": "ui_transfers_unpair", "arguments": {"pair_id": "pair-0402"}}}}),
        ),
    );
    linked.approval = Some("required".into());
    let mut dismissed = call(
        "y2",
        "ui.transfers.dismiss",
        ToolStatus::Done,
        json!({"tx_out": "tx-2025-0320-out", "tx_in": "tx-2025-0320-in", "expires_in_days": 30, "reason": "Different amounts"}),
        Some(
            json!({"kind": "ui.transfers.dismiss", "data": {"id": "dismissal-0320", "undo": {"tool": "ui_transfers_dismissals_delete", "arguments": {"dismissal_id": "dismissal-0320"}}}}),
        ),
    );
    // Ran under the chat's auto mode: the applied card keeps the mode's badge.
    dismissed.approval = Some("auto".into());
    // A plan-bound inverse: the assistant shows the plan first, so there is no Undo button.
    let mut component = call(
        "y3",
        "ui.transfers.components.apply",
        ToolStatus::Done,
        json!({"action": "create"}),
        Some(
            json!({"kind": "ui.transfers.components.apply", "data": {"component": {"id": "comp-1", "state": "active"}, "undo": {"tool": "ui_transfers_components_plan", "arguments": {"action": "supersede", "component_id": "comp-1"}}}}),
        ),
    );
    component.approval = Some("auto".into());
    a8.tool_calls = vec![linked, dismissed, component];

    // The review decisions: a held transaction resolved (Undo reverts the edit), a custody gap
    // bridged (its inverse is a plan, so no Undo button), and two waiting on approval, the
    // custody one after the plan it applies.
    let u9 = Msg::new(
        "u9",
        Role::User,
        "Price the held Bitpanda buy from the invoice, and keep the Ledger gap as mine.",
    );
    let mut a9 = assistant(
        "a9",
        "Done: the buy is priced and the March gap is bridged.",
    );
    let mut priced = call(
        "z1",
        "ui.journals.quarantine.resolve",
        ToolStatus::Done,
        json!({"transaction": "tx-2025-0115-buy", "action": "price_override", "fiat_value": "1840.00", "reason": "Bitpanda invoice"}),
        Some(json!({"kind": "ui.journals.quarantine.resolve", "data": {
            "transaction_id": "tx-2025-0115-buy", "action": "price_override",
            "resolution": {"resolution": "price-override", "fiat_value_exact": "1840.00", "history_event_id": "h-1"},
            "reprocessed": true, "cleared": true,
            "dependents_cleared": {"count": 2, "transaction_ids": ["a", "b"], "truncated": false},
            "quarantine_delta": {"status": "measured", "newly_quarantined": {"count": 0, "transaction_ids": [], "truncated": false}},
            "undo": {"tool": "ui_transactions_history_revert", "arguments": {"transaction": "tx-2025-0115-buy", "event": "h-1"}}}})),
    );
    priced.approval = Some("required".into());
    let mut bridged = call(
        "z2",
        "ui.custody.review.apply",
        ToolStatus::Done,
        json!({"action": "create", "gap_id": "gap-2025-03", "expected_input_version": 41}),
        Some(json!({"kind": "ui.custody.review.apply", "data": {
            "gap_id": "gap-2025-03", "status": "resolved", "retained_msat": 9_990_000_000_i64,
            "residual_msat": 0, "fee_msat": 10_000_000, "filed_report_impacts": [],
            "undo": {"tool": "ui_custody_review_plan", "arguments": {"action": "reopen", "gap_id": "gap-2025-03"}}}})),
    );
    bridged.approval = Some("required".into());
    a9.tool_calls = vec![priced, bridged];

    let u10 = Msg::new(
        "u10",
        Role::User,
        "Exclude the unpriced test deposit, and the rest of the June gap was a gift.",
    );
    let mut a10 = assistant("a10", "");
    a10.status = MsgStatus::Streaming;
    let exclude_args = json!({"group_id": "root:tx-2025-0611-test", "action": "exclude", "reason": "A test deposit the exchange reversed"});
    let classify_args = json!({"action": "classify_residual", "gap_id": "gap-2025-06", "classification": "external_gift", "expected_input_version": 42, "reason": "Gift to my brother"});
    let plan = call(
        "w1",
        "ui.custody.review.plan",
        ToolStatus::Done,
        json!({"action": "classify_residual", "gap_id": "gap-2025-06", "classification": "external_gift"}),
        Some(json!({"kind": "ui.custody.review.plan", "data": {
            "action": "classify_residual", "gap_id": "gap-2025-06", "input_version": 42,
            "classification": "external_gift", "custody_state": "external_confirmed",
            "residual_msat": 1_500_000_000, "filed_report_impacts": []}})),
    );
    let mut exclude = call(
        "w2",
        "ui.journals.quarantine.resolve",
        ToolStatus::AwaitingConsent,
        exclude_args.clone(),
        None,
    );
    exclude.needs_consent = true;
    let mut classify = call(
        "w3",
        "ui.custody.review.apply",
        ToolStatus::AwaitingConsent,
        classify_args.clone(),
        None,
    );
    classify.needs_consent = true;
    a10.tool_calls = vec![plan, exclude, classify];
    let mut consents = consents;
    consents.extend([
        Consent {
            call_id: "w2".into(),
            name: "ui.journals.quarantine.resolve".into(),
            summary: "Exclude the root of root:tx-2025-0611-test from accounting".into(),
            arguments_preview: exclude_args,
            review_preview: None,
            reversible: true,
            egresses: false,
            can_allow_session: false,
        },
        Consent {
            call_id: "w3".into(),
            name: "ui.custody.review.apply".into(),
            summary: "Apply custody review".into(),
            arguments_preview: classify_args,
            review_preview: None,
            reversible: true,
            egresses: false,
            can_allow_session: false,
        },
    ]);
    (vec![u8, a8, u9, a9, u7, a7, u10, a10], consents)
}

/// The custody gaps the action cards name, as `ui.custody.gaps.review_context` returns them.
fn action_gaps() -> Vec<(&'static str, Value)> {
    vec![
        (
            "gap-2025-03",
            json!({"gap_id": "gap-2025-03", "status": "resolved", "asset": "BTC", "source_wallet_label": "Ledger", "destination_wallet_labels": ["Cold wallet"], "source_total_msat": "10000000000", "source_fee_msat": "10000000", "residual_msat": "0", "started_at": "2025-03-02T10:00:00Z", "ended_at": "2025-03-04T18:30:00Z"}),
        ),
        (
            "gap-2025-06",
            json!({"gap_id": "gap-2025-06", "status": "resolved", "asset": "BTC", "source_wallet_label": "Spending", "destination_wallet_labels": [], "source_total_msat": "4000000000", "source_fee_msat": "2000000", "residual_msat": "1500000000", "started_at": "2025-06-11T08:00:00Z", "ended_at": "2025-06-11T08:00:00Z"}),
        ),
    ]
}

/// The transactions the action cards name, as `ui.transactions.resolve` returns them.
fn action_legs() -> Vec<(&'static str, Value)> {
    vec![
        (
            "tx-2025-0402-out",
            json!({"id": "tx-2025-0402-out", "date": "2025-04-02 09:14", "account": "Kraken", "asset": "BTC", "amountSat": -25_000_000, "feeSat": 0, "note": "", "tags": []}),
        ),
        (
            "tx-2025-0402-in",
            json!({"id": "tx-2025-0402-in", "date": "2025-04-02 09:28", "account": "Cold wallet", "asset": "BTC", "amountSat": 24_998_590, "feeSat": 0}),
        ),
        (
            "tx-2025-0320-out",
            json!({"id": "tx-2025-0320-out", "date": "2025-03-20 17:02", "account": "Spending", "asset": "BTC", "amountSat": -1_200_000}),
        ),
        (
            "tx-2025-0320-in",
            json!({"id": "tx-2025-0320-in", "date": "2025-03-20 17:40", "account": "Cold wallet", "asset": "BTC", "amountSat": 950_000}),
        ),
        (
            "tx-2025-0115-buy",
            json!({"id": "tx-2025-0115-buy", "date": "2025-01-15 12:05", "account": "Bitpanda", "asset": "BTC", "amountSat": 2_000_000}),
        ),
        (
            "tx-2025-0611-test",
            json!({"id": "tx-2025-0611-test", "date": "2025-06-11 08:00", "account": "Kraken", "asset": "BTC", "amountSat": 3_000_000, "quarantineReason": "missing_spot_price"}),
        ),
    ]
}

pub fn load_actions(a: &mut App) {
    let (m, c) = actions();
    a.assistant.messages = m;
    a.assistant.pending_consents = c;
    a.assistant.streaming = true;
    a.assistant.synthetic = true;
    let book = super::chat::book_now(a);
    let st = &mut a.assistant.thread.actions;
    st.legs_book = book;
    for (id, tx) in action_legs() {
        st.legs.insert(
            id.into(),
            super::thread::action::Leg {
                loading: false,
                failed: false,
                tx: Some(tx),
            },
        );
    }
    for (id, gap) in action_gaps() {
        st.gaps.insert(
            id.into(),
            super::thread::action::Leg {
                loading: false,
                failed: false,
                tx: Some(gap),
            },
        );
    }
}

/// Binds the fixtures' book-scoped packets to the open book, so they read as this book's.
fn bind_scope(a: &mut App) {
    let scope = a.expected_scope();
    for m in &mut a.assistant.messages {
        for c in &mut m.tool_calls {
            if let Some(d) = c.result.as_mut().and_then(|r| r.get_mut("data"))
                && d.get("workspace_id").is_some()
                && scope["workspace_id"].is_string()
            {
                d["workspace_id"] = scope["workspace_id"].clone();
                d["profile_id"] = scope["profile_id"].clone();
            }
        }
    }
}

/// Fills the conversation with a sample covering every message part.
pub fn load(a: &mut App) {
    let (approvals, consents) = approvals();
    let mut messages = basics();
    messages.extend(visuals());
    messages.extend(approvals);
    let s = &mut a.assistant;
    s.messages = messages;
    s.pending_consents = consents;
    s.streaming = true;
    s.synthetic = true;
    bind_scope(a);
}

/// Only the first turn: reasoning, tool calls, a readiness visual and a markdown answer.
pub fn load_basics(a: &mut App) {
    a.assistant.messages = basics();
    a.assistant.synthetic = true;
}

/// Only the visuals turn: every visual source.
pub fn load_visuals(a: &mut App) {
    a.assistant.messages = visuals();
    a.assistant.synthetic = true;
}

/// Only the review, request, error, stopped and approval turns.
pub fn load_approvals(a: &mut App) {
    let (m, c) = approvals();
    a.assistant.messages = m;
    a.assistant.pending_consents = c;
    a.assistant.streaming = true;
    a.assistant.synthetic = true;
    bind_scope(a);
}

/// A question just sent: the answer is preparing (the loader), the Stop button shows.
pub fn load_pending(a: &mut App) {
    let u = Msg::new("lu1", Role::User, "Which transfers still need a match?");
    let mut ans = assistant("la1", "");
    ans.status = MsgStatus::Pending;
    ans.activity_label = Some("Preparing chat".into());
    a.assistant.messages = vec![u, ans];
    a.assistant.streaming = true;
    a.assistant.synthetic = true;
}

/// An answer streaming in (reasoning open, text arriving) with one question queued behind it.
pub fn load_streaming(a: &mut App) {
    let u = Msg::new("lu1", Role::User, "Which transfers still need a match?");
    let mut ans = assistant(
        "la1",
        "Two transfers still need a match. The first is the **0.25 BTC** withdrawal from Kraken on",
    );
    ans.status = MsgStatus::Streaming;
    ans.activity_label = Some("Generating".into());
    ans.thinking_segments = vec![seg(
        "ls1",
        "The pairing candidates list two unmatched outbound transfers; I should name both.",
    )];
    ans.tool_calls = vec![call(
        "lc1",
        "ui.transfers.candidates",
        ToolStatus::Done,
        json!({"limit": 20}),
        Some(json!({"kind": "ui.transfers.candidates", "data": {"candidates": []}})),
    )];
    ans.tool_calls[0].segment_id = Some("ls1".into());
    a.assistant.messages = vec![u, ans];
    a.assistant.streaming = true;
    a.assistant.synthetic = true;
    if let Some(book) = super::chat::book_now(a) {
        a.assistant.queued = vec![super::chat::Queued {
            prompt: "And which swaps?".into(),
            pin: None,
            context: super::screen::context_for("/overview", ""),
            handoff: false,
            book,
        }];
    }
}

/// Files staged in the composer: a ready CSV, an uploading PDF and one a message already sent.
pub fn files() -> Vec<ChatFile> {
    vec![
        ChatFile {
            key: "file-1".into(),
            view: AttachmentView {
                filename: "kraken-ledger-2025.csv".into(),
                kind: Some("delimited".into()),
                size_bytes: Some(48_213),
            },
            state: FileState::Ready,
            book: None,
        },
        ChatFile {
            key: "file-2".into(),
            view: AttachmentView {
                filename: "bank-statement-march.pdf".into(),
                kind: Some("pdf".into()),
                size_bytes: Some(1_204_000),
            },
            state: FileState::Uploading,
            book: None,
        },
        ChatFile {
            key: "file-3".into(),
            view: AttachmentView {
                filename: "receipt.png".into(),
                kind: Some("image".into()),
                size_bytes: Some(212_400),
            },
            state: FileState::Sent,
            book: None,
        },
    ]
}

/// Saved chats for the History submenu and the quick-resume list.
pub fn sessions() -> SessionList {
    let row = |id: &str, title: &str, at: &str, n: u64| SessionRow {
        id: id.into(),
        title: title.into(),
        updated_at: at.into(),
        message_count: n,
    };
    SessionList {
        sessions: vec![
            row(
                "synthetic-1",
                "Unmatched transfers in March",
                "2026-11-14T16:42:00Z",
                6,
            ),
            row(
                "synthetic-2",
                "Prepare the 2025 tax export",
                "2026-11-12T09:05:00Z",
                12,
            ),
            row(
                "synthetic-3",
                "Why is the cold storage balance off?",
                "2026-11-03T18:20:00Z",
                4,
            ),
            row(
                "synthetic-4",
                "Reprocess journals after the Kraken import",
                "2026-10-28T11:11:00Z",
                9,
            ),
        ],
        history_enabled: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::thread::{action, approval, visual};

    #[test]
    fn every_fixture_passes_the_web_validators() {
        let v = visuals();
        let drawn = v[1].tool_calls.iter().filter_map(visual::parse).count();
        assert_eq!(drawn, 7, "every visual source parses");
        let b = basics();
        assert_eq!(b[1].tool_calls.iter().filter_map(visual::parse).count(), 1);
        let (m, c) = approvals();
        assert!(
            m[1].tool_calls
                .iter()
                .any(|c| approval::review_result(c).is_some())
        );
        assert_eq!(c.len(), 2);
        let (m, c) = actions();
        for r in &c {
            assert!(
                action::parse(&r.name, &r.arguments_preview).is_some(),
                "{} shows as a native card",
                r.name
            );
        }
        let applied: Vec<_> = m[1]
            .tool_calls
            .iter()
            .filter(|c| action::shows_applied(c))
            .collect();
        assert_eq!(applied.len(), 2, "the two native changes");
        assert_eq!(
            m[1].tool_calls
                .iter()
                .filter(|c| action::desktop_undo(c).is_some())
                .count(),
            2,
            "no Undo for the plan-bound inverse"
        );
    }
}
