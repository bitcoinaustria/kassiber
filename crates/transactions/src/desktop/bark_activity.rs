//! A bounded wallet-scoped projection of stored Bark operations.
use super::{num, set, text, val};
use crate::sql::{rows, Result};
use kassiber_envelope::{s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::{
    optional::{self, BTCPAY_BARK_ACTIVITY},
    rusqlite::{params, Connection},
};

pub(super) fn recent(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    current: bool,
    limit: i64,
) -> Result<Value> {
    if !optional::present(conn, BTCPAY_BARK_ACTIVITY)
        .map_err(|e| AppError::new(e.code, e.message))?
    {
        return Ok(Value::List(vec![]));
    }
    let mut activity = rows(
        conn,
        "WITH recent AS (
           SELECT a.* FROM btcpay_bark_activity a
           WHERE a.wallet_id IN (SELECT id FROM wallets WHERE profile_id=?1 AND (id=?2 OR lower(label)=lower(?2)))
           ORDER BY a.occurred_at DESC, a.row_id, a.source_key LIMIT ?3
         ), resolved AS (
           SELECT recent.*, (SELECT CASE WHEN COUNT(*)=1 THEN MIN(t.id) END FROM transactions t
             WHERE t.wallet_id=recent.wallet_id AND t.excluded=0
               AND CASE WHEN json_valid(t.raw_json) THEN
                    json_extract(t.raw_json,'$.source')='btcpay_bark_wallet'
                AND json_extract(t.raw_json,'$.bark_row_id')=recent.row_id
                AND json_extract(t.raw_json,'$.store_id')=substr(recent.source_key,66)
                AND json_extract(t.raw_json,'$.part')='principal'
               END
             ) AS transaction_id
           FROM recent
         )
         SELECT resolved.row_id, resolved.kind, resolved.occurred_at,
                resolved.amount_msat, resolved.fee_msat, resolved.has_txid,
                resolved.comment, resolved.labels_json, resolved.transaction_id,
                json_extract(t.raw_json,'$._kassiber_provenance.import_source') = 'btcpay-bark' AS sync_provenance,
                CASE WHEN ?4 THEN q.reason END AS hold_reason,
                CASE WHEN ?4 THEN EXISTS (
                   SELECT 1 FROM journal_custody_decisions d
                   WHERE d.profile_id=?1 AND d.source_transaction_id=resolved.transaction_id
                     AND d.state IN ('internal_verified','internal_reviewed')
                ) END AS paired,
                (SELECT CASE WHEN COUNT(DISTINCT p.id)=1 THEN MAX(p.origin_kind) END
                   FROM btcpay_provenance_records p
                   LEFT JOIN commercial_links c ON c.btcpay_record_id=p.id AND c.state='reviewed'
                        AND c.transaction_id=resolved.transaction_id
                   WHERE p.profile_id=?1 AND p.record_type='payout' AND lower(p.status)='completed'
                     AND (p.stable_key=t.external_id OR c.id IS NOT NULL)
                ) AS payout_kind
         FROM resolved
         LEFT JOIN transactions t ON t.id=resolved.transaction_id
         LEFT JOIN journal_quarantines q ON q.transaction_id=resolved.transaction_id
         ORDER BY resolved.occurred_at DESC, resolved.row_id, resolved.source_key",
        params![profile, wallet, limit, current],
    )?;
    for row in &mut activity {
        let labels =
            kassiber_pytext::loads(text(row, "labels_json")).unwrap_or(Value::List(vec![]));
        set(row, "labels_json", labels);
        let onchain_send = matches!(
            text(row, "kind"),
            "offboard" | "send-onchain" | "onchain-send"
        );
        if onchain_send && num(row, "amount_msat") < -num(row, "fee_msat") {
            let status = if !current {
                "journal_pending"
            } else if kassiber_envelope::truthy(&val(row, "paired")) {
                "paired"
            } else if !kassiber_envelope::truthy(&val(row, "has_txid")) {
                "missing_txid"
            } else if val(row, "transaction_id") != Value::Null
                && !kassiber_envelope::truthy(&val(row, "sync_provenance"))
            {
                "missing_sync_provenance"
            } else if let reason @ Value::Str(_) = val(row, "hold_reason") {
                set(row, "transfer_status", reason);
                continue;
            } else {
                "no_pair"
            };
            set(row, "transfer_status", s(status));
        }
    }
    Ok(Value::List(activity))
}
