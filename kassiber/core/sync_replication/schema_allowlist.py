"""Explicit allowlist for the authored book layer that may enter bundles.

Nothing is serialized merely because a table or column exists. Every synced
table and column is named here, and import validates against the same map.
Derived state and local/secret state are absent by construction.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
import re
import sqlite3
from typing import Any, Iterator, Mapping

from ...errors import AppError
from .crypto import decode_secret, hmac_identifier


@dataclass(frozen=True)
class TableSpec:
    table: str
    columns: tuple[str, ...]
    primary_key: tuple[str, ...]
    scope_sql: str
    high_stakes_fields: frozenset[str] = frozenset()
    add_win_fields: frozenset[str] = frozenset()
    soft_delete_column: str | None = None
    json_columns: frozenset[str] = frozenset()


_TRANSACTION_PRICING_FIELDS = frozenset(
    {
        "fiat_currency",
        "fiat_rate",
        "fiat_value",
        "fiat_price_source",
        "fiat_rate_exact",
        "fiat_value_exact",
        "pricing_source_kind",
        "pricing_provider",
        "pricing_pair",
        "pricing_timestamp",
        "pricing_fetched_at",
        "pricing_granularity",
        "pricing_method",
        "pricing_external_ref",
        "pricing_quality",
    }
)


def _profile_scope(table: str) -> str:
    return f"SELECT * FROM {table} WHERE profile_id = ?"


SYNC_TABLES: tuple[TableSpec, ...] = (
    TableSpec(
        "workspaces",
        ("id", "label", "created_at"),
        ("id",),
        "SELECT * FROM workspaces WHERE id = (SELECT workspace_id FROM profiles WHERE id = ?)",
    ),
    TableSpec(
        "profiles",
        (
            "id",
            "workspace_id",
            "label",
            "fiat_currency",
            "tax_country",
            "tax_long_term_days",
            "gains_algorithm",
            "require_coarse_review",
            "bitcoin_rail_carrying_value",
            "created_at",
        ),
        ("id",),
        "SELECT * FROM profiles WHERE id = ?",
        high_stakes_fields=frozenset(
            {
                "fiat_currency",
                "tax_country",
                "tax_long_term_days",
                "gains_algorithm",
                "require_coarse_review",
                "bitcoin_rail_carrying_value",
            }
        ),
    ),
    TableSpec(
        "accounts",
        ("id", "workspace_id", "profile_id", "code", "label", "account_type", "asset", "created_at"),
        ("id",),
        _profile_scope("accounts"),
    ),
    TableSpec(
        "wallets",
        ("id", "workspace_id", "profile_id", "account_id", "label", "kind", "config_json", "created_at"),
        ("id",),
        _profile_scope("wallets"),
        json_columns=frozenset({"config_json"}),
    ),
    TableSpec(
        "transactions",
        (
            "id",
            "workspace_id",
            "profile_id",
            "wallet_id",
            "external_id",
            "occurred_at",
            "confirmed_at",
            "direction",
            "asset",
            "amount",
            "fee",
            "amount_includes_fee",
            "fiat_currency",
            "fiat_rate",
            "fiat_value",
            "fiat_price_source",
            "fiat_rate_exact",
            "fiat_value_exact",
            "pricing_source_kind",
            "pricing_provider",
            "pricing_pair",
            "pricing_timestamp",
            "pricing_fetched_at",
            "pricing_granularity",
            "pricing_method",
            "pricing_external_ref",
            "pricing_quality",
            "review_status",
            "taxability_override",
            "at_regime_override",
            "at_category_override",
            "privacy_boundary",
            "kind",
            "description",
            "counterparty",
            "note",
            "excluded",
            "payment_hash",
            "payment_hash_source",
            "swap_refund_funding_txid",
            "created_at",
        ),
        ("id",),
        _profile_scope("transactions"),
        high_stakes_fields=frozenset(
            {
                "review_status",
                "taxability_override",
                "at_regime_override",
                "at_category_override",
                "excluded",
            }
        )
        | _TRANSACTION_PRICING_FIELDS,
        add_win_fields=frozenset({"note"}),
    ),
    TableSpec(
        "tags",
        ("id", "workspace_id", "profile_id", "code", "label", "created_at"),
        ("id",),
        _profile_scope("tags"),
        add_win_fields=frozenset({"code", "label"}),
    ),
    TableSpec(
        "transaction_tags",
        ("transaction_id", "tag_id"),
        ("transaction_id", "tag_id"),
        "SELECT tt.* FROM transaction_tags tt JOIN transactions t ON t.id = tt.transaction_id WHERE t.profile_id = ?",
        add_win_fields=frozenset({"transaction_id", "tag_id"}),
    ),
    TableSpec(
        "transaction_pairs",
        (
            "id", "workspace_id", "profile_id", "out_transaction_id", "in_transaction_id",
            "kind", "policy", "notes", "swap_fee_msat", "swap_fee_kind",
            "confidence_at_pair", "pair_source", "out_amount", "deleted_at", "created_at",
        ),
        ("id",),
        _profile_scope("transaction_pairs"),
        high_stakes_fields=frozenset({"kind", "policy", "out_transaction_id", "in_transaction_id", "deleted_at"}),
        soft_delete_column="deleted_at",
    ),
    TableSpec(
        "direct_swap_payouts",
        (
            "id", "workspace_id", "profile_id", "out_transaction_id", "kind", "policy",
            "payout_asset", "payout_amount", "payout_occurred_at", "payout_fiat_value",
            "payout_external_id", "counterparty", "notes", "swap_fee_msat",
            "swap_fee_kind", "out_amount", "deleted_at", "created_at",
        ),
        ("id",),
        _profile_scope("direct_swap_payouts"),
        high_stakes_fields=frozenset({"policy", "payout_amount", "payout_fiat_value", "deleted_at"}),
        soft_delete_column="deleted_at",
    ),
    TableSpec(
        "transaction_pair_dismissals",
        (
            "id", "workspace_id", "profile_id", "out_transaction_id", "in_transaction_id",
            "reason", "created_at", "expires_at",
        ),
        ("id",),
        _profile_scope("transaction_pair_dismissals"),
        high_stakes_fields=frozenset({"out_transaction_id", "in_transaction_id"}),
    ),
    TableSpec(
        "loan_legs",
        ("id", "workspace_id", "profile_id", "transaction_id", "loan_id", "role", "note", "deleted_at", "created_at"),
        ("id",),
        _profile_scope("loan_legs"),
        high_stakes_fields=frozenset({"transaction_id", "role", "deleted_at"}),
        soft_delete_column="deleted_at",
    ),
    TableSpec(
        "swap_matching_rules",
        ("id", "workspace_id", "profile_id", "name", "predicate_json", "kind", "policy", "enabled", "created_at", "updated_at"),
        ("id",),
        _profile_scope("swap_matching_rules"),
        high_stakes_fields=frozenset({"predicate_json", "kind", "policy", "enabled"}),
        json_columns=frozenset({"predicate_json"}),
    ),
    TableSpec(
        "saved_views",
        ("id", "workspace_id", "profile_id", "surface", "name", "filter_json", "created_at", "updated_at"),
        ("id",),
        _profile_scope("saved_views"),
        json_columns=frozenset({"filter_json"}),
    ),
    TableSpec(
        "bip329_labels",
        ("id", "workspace_id", "profile_id", "wallet_id", "record_type", "ref", "label", "origin", "spendable", "data_json", "created_at"),
        ("id",),
        _profile_scope("bip329_labels"),
        add_win_fields=frozenset({"label"}),
        json_columns=frozenset({"data_json"}),
    ),
    TableSpec(
        "attachments",
        (
            "id", "workspace_id", "profile_id", "transaction_id", "attachment_type", "label",
            "original_filename", "stored_relpath", "source_url", "media_type", "size_bytes",
            "copied_from_attachment_id", "copied_from_transaction_id", "created_at",
        ),
        ("id",),
        _profile_scope("attachments"),
    ),
    TableSpec(
        "external_documents",
        (
            "id", "workspace_id", "profile_id", "document_type", "label", "external_ref",
            "issuer", "counterparty", "issued_at", "due_at", "fiat_currency",
            "fiat_value_exact", "review_state", "notes", "created_at", "updated_at",
        ),
        ("id",),
        _profile_scope("external_documents"),
        high_stakes_fields=frozenset({"fiat_currency", "fiat_value_exact", "review_state"}),
    ),
    TableSpec(
        "external_document_attachments",
        ("document_id", "attachment_id", "created_at"),
        ("document_id", "attachment_id"),
        "SELECT j.* FROM external_document_attachments j JOIN external_documents d ON d.id = j.document_id WHERE d.profile_id = ?",
    ),
    TableSpec(
        "commercial_links",
        (
            "id", "workspace_id", "profile_id", "btcpay_record_id", "document_id",
            "transaction_id", "link_type", "state", "confidence", "method",
            "allocation_amount", "allocation_fiat_exact", "reconciliation_state",
            "commercial_kind", "applied_transaction_snapshot_json",
            "reviewed_record_snapshot_json", "notes", "reviewed_at", "created_at", "updated_at",
        ),
        ("id",),
        _profile_scope("commercial_links"),
        high_stakes_fields=frozenset({"state", "allocation_amount", "allocation_fiat_exact", "reconciliation_state"}),
        json_columns=frozenset({"applied_transaction_snapshot_json", "reviewed_record_snapshot_json"}),
    ),
    TableSpec(
        "source_funds_sources",
        (
            "id", "workspace_id", "profile_id", "source_type", "label", "asset", "amount",
            "fiat_currency", "fiat_value", "acquired_at", "description", "review_state",
            "created_at", "updated_at",
        ),
        ("id",),
        _profile_scope("source_funds_sources"),
        high_stakes_fields=frozenset({"amount", "fiat_currency", "fiat_value", "review_state"}),
    ),
    TableSpec(
        "source_funds_links",
        (
            "id", "workspace_id", "profile_id", "from_source_id", "from_transaction_id",
            "to_transaction_id", "link_type", "state", "confidence", "method", "asset",
            "allocation_amount", "from_asset", "from_allocation_amount", "allocation_policy",
            "explanation", "uses_chain_observation", "chain_data_confirmed", "created_at", "updated_at",
        ),
        ("id",),
        _profile_scope("source_funds_links"),
        high_stakes_fields=frozenset({"state", "allocation_amount", "from_allocation_amount", "allocation_policy"}),
    ),
    TableSpec(
        "source_funds_link_attachments",
        ("link_id", "attachment_id", "created_at"),
        ("link_id", "attachment_id"),
        "SELECT j.* FROM source_funds_link_attachments j JOIN source_funds_links l ON l.id = j.link_id WHERE l.profile_id = ?",
    ),
    TableSpec(
        "source_funds_source_attachments",
        ("source_id", "attachment_id", "created_at"),
        ("source_id", "attachment_id"),
        "SELECT j.* FROM source_funds_source_attachments j JOIN source_funds_sources s ON s.id = j.source_id WHERE s.profile_id = ?",
    ),
    TableSpec(
        "source_funds_cases",
        (
            "id", "workspace_id", "profile_id", "target_transaction_id", "target_external_id",
            "target_amount", "asset", "label", "reveal_mode", "status", "snapshot_json",
            "recipient_id", "recipient_label_snapshot", "recipient_kind_snapshot",
            "recipient_reveal_mode_snapshot", "created_at", "updated_at",
        ),
        ("id",),
        _profile_scope("source_funds_cases"),
        high_stakes_fields=frozenset({"target_amount", "reveal_mode", "status", "snapshot_json"}),
        json_columns=frozenset({"snapshot_json"}),
    ),
    TableSpec(
        "source_funds_snapshots",
        ("id", "case_id", "snapshot_json", "created_at"),
        ("id",),
        "SELECT s.* FROM source_funds_snapshots s JOIN source_funds_cases c ON c.id = s.case_id WHERE c.profile_id = ?",
        json_columns=frozenset({"snapshot_json"}),
    ),
    TableSpec(
        "source_funds_recipients",
        ("id", "workspace_id", "profile_id", "label", "kind", "default_reveal_mode", "notes", "active", "created_at", "updated_at"),
        ("id",),
        _profile_scope("source_funds_recipients"),
    ),
)


SYNC_TABLE_MAP: Mapping[str, TableSpec] = {spec.table: spec for spec in SYNC_TABLES}

# Explicit assertions document the privilege boundary. Adding one of these to
# ``SYNC_TABLES`` must fail loudly in tests/review.
NEVER_SYNC_TABLES = frozenset(
    {
        "settings",
        "backends",
        "ai_providers",
        "ai_provider_secret_refs",
        "ai_chat_sessions",
        "ai_chat_messages",
        "journal_entries",
        "journal_quarantines",
        "journal_tax_summary",
        "journal_account_holdings",
        "journal_wallet_holdings",
        "wallet_utxos",
        "wallet_utxo_refreshes",
        "rates_cache",
        "rates_checked_minutes",
        "freshness_source_states",
        "freshness_jobs",
        "transaction_graph_cache",
        "lightning_node_syncs",
        "lightning_node_records",
        "btcpay_provenance_records",
        "btcpay_account_routes",
        "sync_member_private_keys",
        "sync_device_private_keys",
        "sync_books",
        "sync_transports",
        "sync_mailbox_heads",
        "sync_peer_status",
        "sync_replica_acknowledgements",
        "sync_tombstone_gc_log",
        "sync_join_requests",
    }
)

_PRIVATE_EXTENDED_KEY = re.compile(r"(?:^|[^a-z])(xprv|yprv|zprv|tprv|uprv|vprv)[a-z0-9]*", re.IGNORECASE)
_SYNC_WALLET_CONFIG_FIELDS = frozenset(
    {
        "addresses",
        "chain",
        "network",
        "gap_limit",
        "policy_asset",
        "altbestand",
        "descriptor_source",
        "synthesize_change",
        "script_types",
        "descriptor",
        "change_descriptor",
        "xpub",
        "deprecated",
    }
)


def _is_public_watch_material(value: Any) -> bool:
    if not isinstance(value, str) or not value.strip():
        return False
    text = value.strip()
    if _PRIVATE_EXTENDED_KEY.search(text):
        return False
    # Confidential descriptors with embedded blinding material are excluded in
    # v1. Peers can re-enter that local secret; public Bitcoin descriptors and
    # xpubs remain eligible as the plan permits.
    if text.lower().startswith("ct(") or "slip77(" in text.lower():
        return False
    return True


def public_wallet_config(raw: Any) -> dict[str, Any]:
    if isinstance(raw, str):
        try:
            raw = json.loads(raw)
        except json.JSONDecodeError:
            return {}
    if not isinstance(raw, dict):
        return {}
    output: dict[str, Any] = {}
    for key in sorted(_SYNC_WALLET_CONFIG_FIELDS):
        if key not in raw:
            continue
        value = raw[key]
        if key in {"descriptor", "change_descriptor", "xpub"}:
            if _is_public_watch_material(value):
                output[key] = value
            continue
        output[key] = value
    return output


def _json_value(value: Any) -> Any:
    if value in (None, ""):
        return None
    if isinstance(value, (dict, list, int, float, bool)):
        return value
    try:
        return json.loads(str(value))
    except (TypeError, ValueError, json.JSONDecodeError):
        return None


def row_key(spec: TableSpec, row: Mapping[str, Any]) -> str:
    values = [str(row[column]) for column in spec.primary_key]
    return json.dumps(values, ensure_ascii=True, separators=(",", ":"))


def serialize_row(
    spec: TableSpec,
    row: Mapping[str, Any],
    *,
    hmac_key_b64: str,
) -> dict[str, Any]:
    """Project a DB row through the explicit column allowlist."""

    payload = {column: row[column] for column in spec.columns}
    if spec.table == "wallets":
        payload["config_json"] = public_wallet_config(row["config_json"])
    else:
        for column in spec.json_columns:
            payload[column] = _json_value(payload.get(column))
    book_key = decode_secret(hmac_key_b64)
    if spec.table == "transactions":
        payload["fingerprint_hmac"] = hmac_identifier(
            book_key,
            "transaction-fingerprint",
            str(row["fingerprint"]),
        )
    elif spec.table == "attachments" and row["sha256"]:
        payload["content_hmac"] = hmac_identifier(
            book_key,
            "attachment-sha256",
            str(row["sha256"]),
        )
    elif spec.table == "commercial_links" and row["btcpay_record_id"]:
        payload["btcpay_record_hmac"] = hmac_identifier(
            book_key,
            "btcpay-provenance-record",
            str(row["btcpay_record_id"]),
        )
        payload["btcpay_record_id"] = None
    return payload


def iter_rows(
    conn: sqlite3.Connection,
    spec: TableSpec,
    *,
    profile_id: str,
) -> Iterator[sqlite3.Row]:
    yield from conn.execute(spec.scope_sql, (profile_id,)).fetchall()


def validate_wire_row(table: str, payload: Mapping[str, Any]) -> TableSpec:
    spec = SYNC_TABLE_MAP.get(table)
    if spec is None:
        raise AppError(
            "bundle references a table outside the sync allowlist",
            code="sync_schema_forbidden",
            details={"table": table},
            retryable=False,
        )
    allowed = set(spec.columns)
    if table == "transactions":
        allowed.add("fingerprint_hmac")
    if table == "attachments":
        allowed.add("content_hmac")
    if table == "commercial_links":
        allowed.add("btcpay_record_hmac")
    unknown = sorted(set(payload) - allowed)
    missing = sorted(set(spec.primary_key) - set(payload))
    if unknown or missing:
        raise AppError(
            "bundle row does not match the sync schema allowlist",
            code="sync_schema_forbidden",
            details={"table": table, "unknown_fields": unknown, "missing_key_fields": missing},
            retryable=False,
        )
    return spec


def validate_schema_boundary() -> None:
    overlap = set(SYNC_TABLE_MAP) & NEVER_SYNC_TABLES
    if overlap:
        raise AssertionError(f"never-sync tables entered allowlist: {sorted(overlap)}")
    if len(SYNC_TABLE_MAP) != len(SYNC_TABLES):
        raise AssertionError("duplicate table in sync allowlist")


validate_schema_boundary()
