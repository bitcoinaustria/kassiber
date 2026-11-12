"""The source order of transactions that share a timestamp.

Same-time rows are common: one exchange export holds many fills in one second,
and one block holds many transactions. Whatever reads them in order (custody
composition, the tax engine's lot order, the latest transaction rate) must
see the same order in every book built from the same data, so the order never
falls back to a random row id or to when a row was inserted. After
``occurred_at`` it is:

1. a confirmed chain row's block height (``raw_json`` ``status``);
2. the row's position in the file that imported it
   (``import_batch_transactions.source_position``);
3. the external id;
4. the row's accounting content, then its wallet's label;
5. the row id, only for rows equal in all of the above.

A missing key sorts first, as SQLite sorts NULL. Labels are unique in a book,
so rows equal through step 4 are in one wallet with the same time, amounts,
prices, kind and tax overrides. Two such rows that both reach the lot engine
are interchangeable lots or disposals, so the id cannot change an amount.
Python and the Rust loader run the same SQL text.
"""

from __future__ import annotations


_BLOCK_HEIGHT = (
    "CASE WHEN {t}.external_id_kind = 'txid' AND json_valid({t}.raw_json) THEN"
    " CASE WHEN json_type({t}.raw_json, '$.status.confirmed') = 'true'"
    " AND json_type({t}.raw_json, '$.status.block_height') = 'integer'"
    " THEN json_extract({t}.raw_json, '$.status.block_height') END END"
)
_SOURCE_POSITION = (
    "(SELECT MIN(source_order.source_position) FROM import_batch_transactions source_order"
    " WHERE source_order.transaction_id = {t}.id)"
)
_CONTENT = (
    "{t}.external_id", "{t}.direction", "{t}.asset", "{t}.amount", "{t}.fee",
    "{t}.amount_includes_fee", "{t}.fiat_value_exact", "{t}.fiat_rate_exact",
    "{t}.fiat_value", "{t}.fiat_rate", "{t}.kind", "{t}.kind_override",
    "{t}.taxability_override", "{t}.at_regime_override", "{t}.at_category_override",
    "(SELECT source_wallet.label FROM wallets source_wallet WHERE source_wallet.id = {t}.wallet_id)",
    "{t}.id",
)


def same_time_order_sql(alias: str = "t", *, descending: bool = False) -> str:
    """The ORDER BY terms that follow ``occurred_at``, for table ``alias``."""
    terms = (_BLOCK_HEIGHT, _SOURCE_POSITION, *_CONTENT)
    suffix = " DESC" if descending else ""
    return ", ".join(term.format(t=alias) + suffix for term in terms)


def transaction_order_sql(alias: str = "t", *, descending: bool = False) -> str:
    """The full ORDER BY terms of the source order, for table ``alias``."""
    suffix = " DESC" if descending else ""
    return f"{alias}.occurred_at{suffix}, {same_time_order_sql(alias, descending=descending)}"


__all__ = ["same_time_order_sql", "transaction_order_sql"]
