// SQL projections shared with the Python reference reports.py.
pub const SUMMARY: &str = r#"
        SELECT
            COUNT(*) AS total_transactions,
            SUM(CASE WHEN t.excluded = 0 THEN 1 ELSE 0 END) AS active_transactions,
            SUM(CASE WHEN t.excluded = 1 THEN 1 ELSE 0 END) AS excluded_transactions,
            SUM(CASE WHEN t.excluded = 0 AND t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_transactions,
            SUM(CASE WHEN t.excluded = 0 AND t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_transactions,
            COUNT(DISTINCT CASE WHEN t.excluded = 0 THEN t.asset END) AS asset_count,
            MIN(CASE WHEN t.excluded = 0 THEN t.occurred_at END) AS first_transaction_at,
            MAX(CASE WHEN t.excluded = 0 THEN t.occurred_at END) AS last_transaction_at,
            SUM(CASE WHEN t.excluded = 0 AND (t.fiat_rate IS NOT NULL OR t.fiat_value IS NOT NULL) THEN 1 ELSE 0 END) AS priced_transactions,
            SUM(CASE WHEN t.excluded = 0 AND COALESCE(TRIM(t.note), '') != '' THEN 1 ELSE 0 END) AS noted_transactions
        FROM transactions t
        WHERE {tx_where}
        "#;
pub const PAIRS: &str = r#"
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.swap_fee_msat,
            COALESCE(p.swap_fee_kind, '') AS swap_fee_kind,
            COALESCE(p.notes, '') AS notes,
            p.created_at,
            tout.occurred_at AS out_occurred_at,
            COALESCE(tout.external_id, '') AS out_transaction_id,
            wout.label AS out_wallet,
            tout.asset AS out_asset,
            p.out_amount,
            tout.fee AS out_fee,
            tin.occurred_at AS in_occurred_at,
            COALESCE(tin.external_id, '') AS in_transaction_id,
            win.label AS in_wallet,
            tin.asset AS in_asset,
            p.in_amount,
            tin.fee AS in_fee
        FROM journal_custody_projection_relations p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE p.relation_kind IN ('move', 'conversion')
          AND p.in_transaction_id IS NOT NULL
          AND {pair_where}
        ORDER BY
            MIN(tout.occurred_at, tin.occurred_at) ASC,
            p.created_at ASC,
            p.id ASC
        "#;
pub const DIRECT: &str = r#"
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.in_asset AS payout_asset,
            p.in_amount AS payout_amount,
            p.target_occurred_at AS payout_occurred_at,
            p.target_external_id AS payout_external_id,
            p.counterparty,
            p.swap_fee_msat,
            COALESCE(p.swap_fee_kind, '') AS swap_fee_kind,
            COALESCE(p.notes, '') AS notes,
            p.created_at,
            tout.occurred_at AS out_occurred_at,
            COALESCE(tout.external_id, '') AS out_transaction_id,
            wout.label AS out_wallet,
            tout.asset AS out_asset,
            p.out_amount,
            tout.fee AS out_fee
        FROM journal_custody_projection_relations p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        WHERE p.relation_kind = 'direct_payout'
          AND {direct_payout_where}
        ORDER BY
            COALESCE(p.target_occurred_at, tout.occurred_at) ASC,
            p.created_at ASC,
            p.id ASC
        "#;
