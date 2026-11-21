//! Bisq's import result and hold reasons in words, shared by the Add connection summary and the
//! Review explanation of a held trade.

use serde_json::Value;

use crate::i18n::{t, tr};

/// The import's `bisq_review_required` trades as lines: what was held and why.
pub fn held_lines(r: &Value) -> Vec<String> {
    r.get("bisq_review_required")
        .and_then(Value::as_array)
        .map(|held| {
            held.iter()
                .map(|h| {
                    let s = |k: &str| h.get(k).and_then(Value::as_str).unwrap_or("");
                    tr(
                        "connections:add.summary.bisqHeldTrade",
                        &[
                            ("trade", s("trade_id")),
                            ("reason", &reason(s("reason"))),
                            ("status", s("status")),
                        ],
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A Bisq hold reason in words; an unknown code reads as itself.
pub fn reason(code: &str) -> String {
    let key = format!("connections:add.summary.bisqReason.{code}");
    if !code.is_empty() && crate::i18n::has(&key) {
        t(&key)
    } else {
        code.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn held_trades_read_with_their_reason_and_status() {
        let r = json!({"bisq_rows": 3, "bisq_review_required": [
            {"trade_id": "AbC12", "status": "Mediation", "reason": "not_completed"},
            {"trade_id": "XyZ9", "status": "Completed", "reason": "something_new"},
        ]});
        let lines = held_lines(&r);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("AbC12"), "{}", lines[0]);
        assert!(lines[0].contains("Mediation"), "{}", lines[0]);
        assert!(
            lines[0].contains(&t("connections:add.summary.bisqReason.not_completed")),
            "{}",
            lines[0]
        );
        // An unknown code reads as itself rather than a missing key.
        assert!(lines[1].contains("something_new"), "{}", lines[1]);
    }

    #[test]
    fn an_import_without_holds_shows_nothing() {
        assert!(held_lines(&json!({"bisq_rows": 2})).is_empty());
        assert!(held_lines(&json!({"bisq_review_required": []})).is_empty());
        assert_eq!(reason(""), "");
    }

    #[test]
    fn every_reason_the_importer_writes_has_words() {
        for code in [
            "not_completed",
            "missing_deposit_tx",
            "missing_payout_tx",
            "conflicting_escrow_txs",
            "deposit_amount_mismatch",
            "payout_amount_mismatch",
            "deposit_not_in_book",
            "deposit_debit_mismatch",
            "payout_receipt_mismatch",
            "payout_precedes_deposit",
            "tax_year_boundary",
        ] {
            assert_ne!(reason(code), code, "{code}");
        }
    }
}
