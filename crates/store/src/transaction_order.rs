//! Python's `kassiber/core/transaction_order.py`: the source order of
//! transactions that share `occurred_at`, as the same SQL text.
//!
//! After `occurred_at`: a confirmed chain row's block height, the row's
//! position in the file that imported it, the external id, the accounting
//! content and the wallet label, and the row id only for rows equal in all of
//! those. A missing key sorts first, as SQLite sorts NULL.

const BLOCK_HEIGHT: &str = "CASE WHEN {t}.external_id_kind = 'txid' AND json_valid({t}.raw_json) THEN CASE WHEN json_type({t}.raw_json, '$.status.confirmed') = 'true' AND json_type({t}.raw_json, '$.status.block_height') = 'integer' THEN json_extract({t}.raw_json, '$.status.block_height') END END";
const SOURCE_POSITION: &str = "(SELECT MIN(source_order.source_position) FROM import_batch_transactions source_order WHERE source_order.transaction_id = {t}.id)";
const CONTENT: [&str; 17] = [
    "{t}.external_id",
    "{t}.direction",
    "{t}.asset",
    "{t}.amount",
    "{t}.fee",
    "{t}.amount_includes_fee",
    "{t}.fiat_value_exact",
    "{t}.fiat_rate_exact",
    "{t}.fiat_value",
    "{t}.fiat_rate",
    "{t}.kind",
    "{t}.kind_override",
    "{t}.taxability_override",
    "{t}.at_regime_override",
    "{t}.at_category_override",
    "(SELECT source_wallet.label FROM wallets source_wallet WHERE source_wallet.id = {t}.wallet_id)",
    "{t}.id",
];

/// `same_time_order_sql`: the ORDER BY terms that follow `occurred_at`.
pub fn same_time_order_sql(alias: &str, descending: bool) -> String {
    let suffix = if descending { " DESC" } else { "" };
    [BLOCK_HEIGHT, SOURCE_POSITION]
        .iter()
        .chain(CONTENT.iter())
        .map(|term| format!("{}{suffix}", term.replace("{t}", alias)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `transaction_order_sql`: the full ORDER BY terms of the source order.
pub fn transaction_order_sql(alias: &str, descending: bool) -> String {
    let suffix = if descending { " DESC" } else { "" };
    format!(
        "{alias}.occurred_at{suffix}, {}",
        same_time_order_sql(alias, descending)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text Python's `transaction_order_sql` returns for these aliases.
    #[test]
    fn matches_python_text() {
        assert_eq!(transaction_order_sql("t", false), "t.occurred_at, CASE WHEN t.external_id_kind = 'txid' AND json_valid(t.raw_json) THEN CASE WHEN json_type(t.raw_json, '$.status.confirmed') = 'true' AND json_type(t.raw_json, '$.status.block_height') = 'integer' THEN json_extract(t.raw_json, '$.status.block_height') END END, (SELECT MIN(source_order.source_position) FROM import_batch_transactions source_order WHERE source_order.transaction_id = t.id), t.external_id, t.direction, t.asset, t.amount, t.fee, t.amount_includes_fee, t.fiat_value_exact, t.fiat_rate_exact, t.fiat_value, t.fiat_rate, t.kind, t.kind_override, t.taxability_override, t.at_regime_override, t.at_category_override, (SELECT source_wallet.label FROM wallets source_wallet WHERE source_wallet.id = t.wallet_id), t.id");
        assert_eq!(transaction_order_sql("transactions", true), "transactions.occurred_at DESC, CASE WHEN transactions.external_id_kind = 'txid' AND json_valid(transactions.raw_json) THEN CASE WHEN json_type(transactions.raw_json, '$.status.confirmed') = 'true' AND json_type(transactions.raw_json, '$.status.block_height') = 'integer' THEN json_extract(transactions.raw_json, '$.status.block_height') END END DESC, (SELECT MIN(source_order.source_position) FROM import_batch_transactions source_order WHERE source_order.transaction_id = transactions.id) DESC, transactions.external_id DESC, transactions.direction DESC, transactions.asset DESC, transactions.amount DESC, transactions.fee DESC, transactions.amount_includes_fee DESC, transactions.fiat_value_exact DESC, transactions.fiat_rate_exact DESC, transactions.fiat_value DESC, transactions.fiat_rate DESC, transactions.kind DESC, transactions.kind_override DESC, transactions.taxability_override DESC, transactions.at_regime_override DESC, transactions.at_category_override DESC, (SELECT source_wallet.label FROM wallets source_wallet WHERE source_wallet.id = transactions.wallet_id) DESC, transactions.id DESC");
    }
}
