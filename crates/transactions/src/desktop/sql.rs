//! SQL projection rules mirrored from core/ui_snapshot.py; no journal computation.
pub(super) const GROUP: &str = r#"COALESCE(
          (
            SELECT 'decision:' || first_decision.decision_id
            FROM (
              SELECT decision.decision_id AS decision_id,
                     decision.occurred_at AS occurred_at
              FROM journal_custody_decisions decision
              WHERE decision.profile_id = t.profile_id
                AND decision.source_transaction_id = t.id
              UNION ALL
              SELECT decision.decision_id, decision.occurred_at
              FROM journal_custody_decisions decision
              WHERE decision.profile_id = t.profile_id
                AND decision.target_transaction_id = t.id
            ) first_decision
            ORDER BY first_decision.occurred_at, first_decision.decision_id
            LIMIT 1
          ),
          (
            SELECT 'relation:' || first_relation.relation_id
            FROM (
              SELECT relation.relation_id AS relation_id,
                     relation.occurred_at AS occurred_at
              FROM journal_custody_economic_relations relation
              WHERE relation.profile_id = t.profile_id
                AND relation.source_transaction_id = t.id
                AND relation.relation_kind = 'conversion'
                AND relation.target_transaction_id IS NOT NULL
              UNION ALL
              SELECT relation.relation_id, relation.occurred_at
              FROM journal_custody_economic_relations relation
              WHERE relation.profile_id = t.profile_id
                AND relation.target_transaction_id = t.id
                AND relation.relation_kind = 'conversion'
            ) first_relation
            ORDER BY first_relation.occurred_at, first_relation.relation_id
            LIMIT 1
          ),
          'tx:' || t.id
        )"#;

pub(super) const TRANSFER: &str = r#"
        EXISTS (
          SELECT 1
          FROM journal_custody_decisions decision
          WHERE decision.profile_id = t.profile_id
            AND decision.source_asset = decision.target_asset
            AND (
              decision.source_transaction_id = t.id
              OR decision.target_transaction_id = t.id
            )
        )
        "#;

pub(super) const SWAP: &str = r#"
        EXISTS (
          SELECT 1
          FROM journal_custody_economic_relations relation
          WHERE relation.profile_id = t.profile_id
            AND relation.relation_kind = 'conversion'
            AND relation.target_transaction_id IS NOT NULL
            AND (
              relation.source_transaction_id = t.id
              OR relation.target_transaction_id = t.id
            )
        )
        "#;

pub(super) const PAYMENT: &str = r#"CASE
          WHEN lower(t.asset) = 'lbtc'
            OR lower(w.kind) IN ('liquid')
            OR lower(w.config_json) LIKE '%"chain"%liquid%'
            THEN 'Liquid'
          WHEN lower(w.kind) IN ('lnd', 'core-ln', 'coreln', 'nwc', 'phoenix')
            THEN 'Lightning'
          WHEN lower(w.kind) IN (
              'kraken', 'bitstamp', 'coinbase', 'bitpanda', 'river', 'binance',
              'bullbitcoin', 'coinfinity', 'strike', 'exchange', '21bitcoin', 'pocketbitcoin'
            )
            THEN 'Exchange'
          WHEN lower(w.kind) IN (
              'address', 'descriptor', 'xpub', 'samourai', 'wasabi',
              'silent-payment', 'ledgerlive'
            )
            THEN 'On-chain'
          WHEN lower(w.label) LIKE '%liquid%' OR lower(w.label) LIKE '%lbtc%'
            THEN 'Liquid'
          WHEN lower(w.label) LIKE '%lightning%' OR lower(w.label) LIKE '%phoenix%'
            OR lower(w.label) LIKE '% ln%' OR lower(w.label) LIKE 'ln %'
            OR lower(w.label) LIKE '%(ln)%'
            THEN 'Lightning'
          WHEN lower(w.label) LIKE '%exchange%' THEN 'Exchange'
          ELSE 'On-chain'
        END"#;

pub(super) const STATUS: &str = r#"CASE
          WHEN lower(COALESCE(t.review_status, '')) IN
               ('review', 'needs_review', 'needs-review', 'blocked', 'quarantined')
            OR jq.reason IS NOT NULL
            THEN 'review'
          WHEN lower(COALESCE(t.review_status, '')) IN ('failed', 'error')
            THEN 'failed'
          WHEN lower(COALESCE(t.review_status, '')) IN ('completed', 'complete')
            THEN 'completed'
          WHEN lower(COALESCE(t.review_status, '')) = 'pending'
            THEN 'pending'
          WHEN t.confirmed_at IS NULL
            THEN 'pending'
          ELSE 'completed'
        END"#;
pub(super) const AMOUNT: &str = r#"(CASE
      WHEN t.direction = 'outbound'
       AND lower(COALESCE(t.kind, '')) NOT IN (
         'chain-swap', 'consolidation', 'peg-in', 'peg-out', 'rebalance',
         'reverse-submarine-swap', 'submarine-swap', 'swap', 'swap-refund',
         'transfer'
       )
        THEN -1
      ELSE 1
    END) * (CASE
      WHEN t.direction = 'outbound'
       AND COALESCE(t.amount, 0) = 0
       AND COALESCE(t.fee, 0) > 0
        THEN ABS(t.fee)
      ELSE ABS(COALESCE(t.amount, 0))
    END)"#;

pub(super) const FIAT_VALUE: &str = r#"(CASE
      WHEN t.direction = 'outbound'
       AND lower(COALESCE(t.kind, '')) NOT IN (
         'chain-swap', 'consolidation', 'peg-in', 'peg-out', 'rebalance',
         'reverse-submarine-swap', 'submarine-swap', 'swap', 'swap-refund',
         'transfer'
       )
        THEN -1
      ELSE 1
    END) * (COALESCE(
      CASE
        WHEN CAST(t.fiat_value AS REAL) > 0
          THEN ABS(CAST(t.fiat_value AS REAL))
      END,
      CASE
        WHEN CAST(t.fiat_rate AS REAL) > 0
          THEN CAST((CASE
      WHEN t.direction = 'outbound'
       AND COALESCE(t.amount, 0) = 0
       AND COALESCE(t.fee, 0) > 0
        THEN ABS(t.fee)
      ELSE ABS(COALESCE(t.amount, 0))
    END) AS REAL)
               / 100000000000.0 * CAST(t.fiat_rate AS REAL)
      END
    ))"#;

pub(super) const FEE: &str = r#"COALESCE(t.fee, 0)"#;
