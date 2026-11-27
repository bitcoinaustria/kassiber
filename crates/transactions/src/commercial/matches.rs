//! Current settlement evidence for BTCPay links. An exact match answers which
//! invoice was paid. Accounting effects still go through commercial review.
use super::*;

// A provider-keyed transaction is written by a BTCPay wallet sync after
// checking settlement identity. A direct Lightning wallet can independently
// prove the same payment hash. A bare on-chain txid can refer to many outputs.
const ASSESS: &str = r#"
    WITH scoped_links AS MATERIALIZED (
      SELECT * FROM commercial_links
      WHERE profile_id = ?1 AND link_type = 'btcpay_payment_transaction'
        AND state IN ('suggested', 'reviewed')
        AND (?2 IS NULL OR transaction_id = ?2)
    ),
    competing_transactions AS (
      SELECT transaction_id FROM scoped_links GROUP BY transaction_id HAVING count(*) > 1
    )
    SELECT cl.id, CASE
      WHEN cl.reviewed_record_snapshot_sha256 IS NOT NULL THEN 'reopened'
      WHEN p.record_type = 'payment' AND lower(p.status) = 'settled'
       AND t.direction = 'inbound' AND EXISTS (
          SELECT 1 FROM transactions sender
          WHERE sender.profile_id = cl.profile_id AND sender.excluded = 0
            AND sender.direction = 'outbound' AND sender.wallet_id != t.wallet_id
            AND (sender.payment_hash = lower(p.payment_hash)
              OR sender.external_id = p.txid)
       ) THEN 'own_wallet'
      WHEN p.amount IS NOT NULL AND p.amount != CASE
         WHEN t.direction = 'outbound' AND t.amount_includes_fee != 0
           THEN t.amount - coalesce(t.fee, 0)
         ELSE t.amount END THEN 'amount_differs'
      WHEN p.amount > 0 AND p.asset = t.asset AND t.excluded = 0
       AND ((p.record_type = 'payment' AND lower(p.status) = 'settled'
                 AND t.direction = 'inbound')
            OR (p.record_type = 'payout' AND p.origin_kind = 'refund'
                 AND lower(p.status) = 'completed' AND t.direction = 'outbound'))
       AND EXISTS (
          SELECT 1 FROM btcpay_provenance_records inv
          WHERE inv.profile_id = p.profile_id AND inv.record_type = 'invoice'
            AND inv.backend_name = p.backend_name COLLATE NOCASE
            AND inv.store_id = p.store_id AND inv.invoice_id = p.invoice_id
            AND lower(inv.status) = 'settled'
       )
       AND (
          (length(p.payment_hash) = 64
           AND lower(p.payment_hash) NOT GLOB '*[^0-9a-f]*'
           AND lower(t.payment_hash) = lower(p.payment_hash))
          OR (t.external_id = p.stable_key AND CASE
            WHEN json_valid(t.raw_json) AND json_valid(w.config_json) THEN
              json_extract(w.config_json, '$.sync_source') = 'btcpay'
              AND json_extract(w.config_json, '$.store_id') = p.store_id
              AND json_extract(t.raw_json, '$.store_id') = p.store_id
              AND (
                (json_extract(t.raw_json, '$.source') = 'btcpay_payment_ledger'
                  AND json_extract(t.raw_json, '$.stable_key') = p.stable_key)
                OR (json_extract(t.raw_json, '$.source') = 'btcpay_bark_wallet'
                  AND json_extract(t.raw_json, '$._kassiber_provenance.import_source') = 'btcpay-bark'
                  AND json_extract(t.raw_json, '$.part') = 'principal')
              )
            ELSE 0 END)
       )
       THEN CASE WHEN EXISTS (
          SELECT 1 FROM transactions other
          WHERE other.profile_id = cl.profile_id AND other.excluded = 0 AND other.id != t.id
            AND (other.external_id = p.stable_key
              OR other.payment_hash IN (p.payment_hash, lower(p.payment_hash))
              OR other.external_id = p.txid)
       ) OR competing_transactions.transaction_id IS NOT NULL OR EXISTS (
          SELECT 1 FROM commercial_links other_link
          WHERE other_link.profile_id = cl.profile_id AND coalesce(other_link.btcpay_record_id, '') = p.id
            AND other_link.transaction_id != t.id AND other_link.state != 'rejected'
            AND other_link.link_type = 'btcpay_payment_transaction'
       ) THEN 'competing_match' ELSE 'verified' END
      ELSE 'needs_review' END AS match_status
    FROM scoped_links cl
    JOIN btcpay_provenance_records p ON p.id = cl.btcpay_record_id
    JOIN transactions t ON t.id = cl.transaction_id
    JOIN wallets w ON w.id = t.wallet_id
    LEFT JOIN competing_transactions ON competing_transactions.transaction_id = t.id
    WHERE cl.state = 'suggested' AND t.excluded = 0
      AND NOT EXISTS (
        SELECT 1 FROM commercial_links dismissed
        WHERE dismissed.profile_id = cl.profile_id
          AND dismissed.transaction_id = cl.transaction_id
          AND dismissed.btcpay_record_id = cl.btcpay_record_id AND dismissed.link_type = cl.link_type
          AND dismissed.state = 'rejected' AND dismissed.reconciliation_state = 'ignored'
      )
"#;

pub(super) fn transaction(conn: &Connection, profile: &str, tx: &str) -> Result<Vec<Value>> {
    rows(conn, ASSESS, params![profile, tx])
}

pub(super) fn needs_review(conn: &Connection, profile: &str, limit: i64) -> Result<Value> {
    let records = rows(
        conn,
        &format!(
            "WITH assessment AS ({ASSESS})
             SELECT * FROM (
               SELECT listed.*, assessment.match_status,
                 row_number() OVER (PARTITION BY listed.btcpay_record_id
                   ORDER BY listed.created_at DESC, listed.id DESC) AS record_rank
               FROM ({LINK_SELECT}) listed JOIN assessment ON assessment.id = listed.id
               WHERE assessment.match_status != 'verified'
             ) WHERE record_rank = 1 ORDER BY created_at DESC, id DESC LIMIT ?3"
        ),
        params![profile, Option::<&str>::None, limit],
    )?;
    Ok(Value::List(
        records
            .iter()
            .map(|row| {
                let mut result = link_payload(row)?;
                kassiber_envelope::set(&mut result, "match_status", value(row, "match_status"));
                Ok(result)
            })
            .collect::<Result<_>>()?,
    ))
}
