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
from ..chain_observer.store import PRIVATE_OBSERVER_TABLES
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
    # Additive nullable wire fields may be absent in older signed bundles. New
    # captures always include them; merge uses NULL on insert and preserves an
    # already-materialized value on update when an older event omits the field.
    optional_columns: frozenset[str] = frozenset()
    # Fields on append-only authored revisions which may never be rewritten in
    # place.  A custody revision changes by inserting a new component/leg/
    # allocation id; only the component lifecycle fields remain mutable.
    immutable_fields: frozenset[str] = frozenset()


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


# Wire references use the referenced row's stable authored identity, which may
# differ from a device-local primary key after fingerprint deduplication.
REFERENCE_TABLES: Mapping[str, str] = {
    "account_id": "accounts",
    "wallet_id": "wallets",
    "transaction_id": "transactions",
    "anchor_transaction_id": "transactions",
    "out_transaction_id": "transactions",
    "in_transaction_id": "transactions",
    "from_transaction_id": "transactions",
    "to_transaction_id": "transactions",
    "target_transaction_id": "transactions",
    "copied_from_transaction_id": "transactions",
    "tag_id": "tags",
    "attachment_id": "attachments",
    "copied_from_attachment_id": "attachments",
    "document_id": "external_documents",
    "from_source_id": "source_funds_sources",
    "source_id": "source_funds_sources",
    "link_id": "source_funds_links",
    "case_id": "source_funds_cases",
    "component_id": "custody_components",
    "filed_report_snapshot_id": "filed_report_snapshots",
    "impact_id": "custody_filed_report_impacts",
    "review_id": "custody_gap_reviews",
    "supersedes_component_id": "custody_components",
    "superseded_by_component_id": "custody_components",
    "source_leg_id": "custody_component_legs",
    "sink_leg_id": "custody_component_legs",
}


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
        "filed_report_snapshots",
        (
            "id", "workspace_id", "profile_id", "report_kind", "report_state",
            "period_start_year", "period_end_year", "content_sha256",
            "classification_summary_json", "gain_summary_json", "report_scope_json", "authored_source",
            "notes", "created_at",
        ),
        ("id",),
        _profile_scope("filed_report_snapshots"),
        high_stakes_fields=frozenset(
            {
                "report_kind", "report_state", "period_start_year",
                "period_end_year", "content_sha256",
                "classification_summary_json", "gain_summary_json",
                "report_scope_json",
                "authored_source",
            }
        ),
        json_columns=frozenset(
            {"classification_summary_json", "gain_summary_json", "report_scope_json"}
        ),
        immutable_fields=frozenset(
            {
                "workspace_id", "profile_id", "report_kind", "report_state",
                "period_start_year", "period_end_year", "content_sha256",
                "classification_summary_json", "gain_summary_json",
                "report_scope_json",
                "authored_source", "notes", "created_at",
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
            "external_id_kind",
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
            "swap_refund_funding_vout",
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
        optional_columns=frozenset(
            {"external_id_kind", "swap_refund_funding_vout"}
        ),
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
        "custody_components",
        (
            "id", "lineage_id", "workspace_id", "profile_id", "revision",
            "component_type", "conservation_mode", "state", "evidence_kind",
            "evidence_grade", "conversion_policy", "conversion_reviewed",
            "expected_leg_count", "expected_allocation_count",
            "expected_evidence_count", "authored_source", "notes",
            "change_reason", "supersedes_component_id",
            "superseded_by_component_id", "activated_at", "superseded_at",
            "created_at",
        ),
        ("id",),
        _profile_scope("custody_components"),
        high_stakes_fields=frozenset(
            {
                "revision", "component_type", "conservation_mode", "state",
                "evidence_kind", "evidence_grade", "conversion_policy",
                "conversion_reviewed", "expected_leg_count",
                "expected_allocation_count", "expected_evidence_count",
                "authored_source", "supersedes_component_id",
                "superseded_by_component_id",
            }
        ),
        immutable_fields=frozenset(
            {
                "lineage_id", "workspace_id", "profile_id", "revision",
                "component_type", "conservation_mode", "evidence_kind",
                "evidence_grade", "conversion_policy", "conversion_reviewed",
                "expected_leg_count", "expected_allocation_count",
                "expected_evidence_count", "authored_source", "notes",
                "supersedes_component_id", "created_at",
            }
        ),
        optional_columns=frozenset(
            {
                "expected_leg_count", "expected_allocation_count",
                "expected_evidence_count", "authored_source",
            }
        ),
    ),
    TableSpec(
        "custody_component_legs",
        (
            "id", "component_id", "workspace_id", "profile_id", "ordinal",
            "role", "rail", "chain", "network", "asset", "exposure",
            "conservation_unit", "amount_msat", "valuation_unit",
            "valuation_amount", "occurred_at", "transaction_id",
            "anchor_transaction_id", "wallet_id", "notes",
            "created_at",
        ),
        ("id",),
        _profile_scope("custody_component_legs"),
        high_stakes_fields=frozenset(
            {
                "component_id", "ordinal", "role", "rail", "chain", "network",
                "asset", "exposure", "conservation_unit", "amount_msat",
                "valuation_unit", "valuation_amount", "occurred_at",
                "transaction_id", "anchor_transaction_id", "wallet_id",
            }
        ),
        optional_columns=frozenset({"anchor_transaction_id"}),
        immutable_fields=frozenset(
            {
                "component_id", "workspace_id", "profile_id", "ordinal",
                "role", "rail", "chain", "network", "asset", "exposure",
                "conservation_unit", "amount_msat", "valuation_unit",
                "valuation_amount", "occurred_at", "transaction_id",
                "anchor_transaction_id", "wallet_id", "notes", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_component_allocations",
        (
            "id", "component_id", "workspace_id", "profile_id", "ordinal",
            "source_leg_id", "sink_leg_id", "source_amount_msat",
            "sink_amount_msat", "created_at",
        ),
        ("id",),
        _profile_scope("custody_component_allocations"),
        high_stakes_fields=frozenset(
            {
                "component_id", "ordinal", "source_leg_id", "sink_leg_id",
                "source_amount_msat", "sink_amount_msat",
            }
        ),
        immutable_fields=frozenset(
            {
                "component_id", "workspace_id", "profile_id", "ordinal",
                "source_leg_id", "sink_leg_id", "source_amount_msat",
                "sink_amount_msat", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_component_evidence_commitments",
        (
            "id", "component_id", "workspace_id", "profile_id", "ordinal",
            "quantity_hash", "detail_hash", "created_at",
        ),
        ("id",),
        _profile_scope("custody_component_evidence_commitments"),
        high_stakes_fields=frozenset(
            {
                "component_id", "workspace_id", "profile_id", "ordinal",
                "quantity_hash", "detail_hash", "created_at",
            }
        ),
        immutable_fields=frozenset(
            {
                "component_id", "workspace_id", "profile_id", "ordinal",
                "quantity_hash", "detail_hash", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_gap_reviews",
        (
            "id", "workspace_id", "profile_id", "gap_id", "revision",
            "candidate_fingerprint", "action", "event_kind", "component_id",
            "authored_source", "reason", "snapshot_json", "created_at",
        ),
        ("id",),
        _profile_scope("custody_gap_reviews"),
        high_stakes_fields=frozenset(
            {
                "gap_id", "revision", "candidate_fingerprint", "action", "event_kind",
                "component_id", "authored_source", "snapshot_json",
            }
        ),
        json_columns=frozenset({"snapshot_json"}),
        immutable_fields=frozenset(
            {
                "workspace_id", "profile_id", "gap_id", "revision",
                "candidate_fingerprint", "action", "event_kind", "component_id",
                "authored_source", "reason", "snapshot_json", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_gap_review_relation_sets",
        (
            "id", "review_id", "workspace_id", "profile_id",
            "expected_source_count", "expected_return_count", "created_at",
        ),
        ("id",),
        _profile_scope("custody_gap_review_relation_sets"),
        high_stakes_fields=frozenset(
            {
                "review_id", "workspace_id", "profile_id",
                "expected_source_count", "expected_return_count", "created_at",
            }
        ),
        immutable_fields=frozenset(
            {
                "review_id", "workspace_id", "profile_id",
                "expected_source_count", "expected_return_count", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_gap_review_transactions",
        (
            "id", "review_id", "workspace_id", "profile_id",
            "role", "transaction_id", "created_at",
        ),
        ("id",),
        _profile_scope("custody_gap_review_transactions"),
        high_stakes_fields=frozenset(
            {
                "review_id", "workspace_id", "profile_id",
                "role", "transaction_id", "created_at",
            }
        ),
        immutable_fields=frozenset(
            {
                "review_id", "workspace_id", "profile_id",
                "role", "transaction_id", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_filed_report_impacts",
        (
            "id", "workspace_id", "profile_id", "filed_report_snapshot_id",
            "component_id", "review_id", "gap_id",
            "affected_period_start_year", "affected_period_end_year",
            "before_classification_summary_json",
            "after_classification_summary_json", "before_gain_summary_json",
            "after_gain_summary_json", "amendment_warning", "created_at",
        ),
        ("id",),
        _profile_scope("custody_filed_report_impacts"),
        high_stakes_fields=frozenset(
            {
                "filed_report_snapshot_id", "component_id", "review_id", "gap_id",
                "affected_period_start_year", "affected_period_end_year",
                "before_classification_summary_json",
                "after_classification_summary_json", "before_gain_summary_json",
                "after_gain_summary_json", "amendment_warning",
            }
        ),
        json_columns=frozenset(
            {
                "before_classification_summary_json",
                "after_classification_summary_json",
                "before_gain_summary_json",
                "after_gain_summary_json",
            }
        ),
        immutable_fields=frozenset(
            {
                "workspace_id", "profile_id", "filed_report_snapshot_id",
                "component_id", "review_id", "gap_id",
                "affected_period_start_year", "affected_period_end_year",
                "before_classification_summary_json",
                "after_classification_summary_json", "before_gain_summary_json",
                "after_gain_summary_json", "amendment_warning", "created_at",
            }
        ),
    ),
    TableSpec(
        "custody_filed_report_impact_resolutions",
        (
            "id", "workspace_id", "profile_id", "impact_id", "rebuilt_at",
            "after_classification_summary_json", "after_gain_summary_json",
            "classification_changed", "gain_changed", "amendment_status",
            "created_at",
        ),
        ("id",),
        _profile_scope("custody_filed_report_impact_resolutions"),
        high_stakes_fields=frozenset(
            {
                "impact_id", "rebuilt_at", "after_classification_summary_json",
                "after_gain_summary_json", "classification_changed",
                "gain_changed", "amendment_status",
            }
        ),
        json_columns=frozenset(
            {
                "after_classification_summary_json",
                "after_gain_summary_json",
            }
        ),
        immutable_fields=frozenset(
            {
                "workspace_id", "profile_id", "impact_id", "rebuilt_at",
                "after_classification_summary_json", "after_gain_summary_json",
                "classification_changed", "gain_changed", "amendment_status",
                "created_at",
            }
        ),
    ),
    TableSpec(
        "transaction_pairs",
        (
            "id", "workspace_id", "profile_id", "out_transaction_id", "in_transaction_id",
            "kind", "policy", "notes", "swap_fee_msat", "swap_fee_kind",
            "confidence_at_pair", "pair_source", "out_amount", "component_id",
            "deleted_at", "created_at",
        ),
        ("id",),
        _profile_scope("transaction_pairs"),
        high_stakes_fields=frozenset({"kind", "policy", "out_transaction_id", "in_transaction_id", "component_id", "deleted_at"}),
        soft_delete_column="deleted_at",
        optional_columns=frozenset({"component_id"}),
    ),
    TableSpec(
        "direct_swap_payouts",
        (
            "id", "workspace_id", "profile_id", "out_transaction_id", "kind", "policy",
            "payout_asset", "payout_amount", "payout_occurred_at", "payout_fiat_value",
            "payout_external_id", "counterparty", "notes", "swap_fee_msat",
            "swap_fee_kind", "out_amount", "component_id", "deleted_at", "created_at",
        ),
        ("id",),
        _profile_scope("direct_swap_payouts"),
        high_stakes_fields=frozenset({"policy", "payout_amount", "payout_fiat_value", "component_id", "deleted_at"}),
        soft_delete_column="deleted_at",
        optional_columns=frozenset({"component_id"}),
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
NEVER_SYNC_TABLES = PRIVATE_OBSERVER_TABLES | frozenset(
    {
        "settings",
        "schema_migration_audits",
        "custody_tax_migration_baselines",
        "custody_tax_migration_baseline_events",
        "custody_tax_migration_reports",
        "backends",
        "ai_providers",
        "ai_provider_secret_refs",
        "ai_chat_sessions",
        "ai_chat_messages",
        "custody_ai_assistance_audits",
        "custody_gap_candidate_snapshots",
        "custody_gap_candidate_projections",
        "custody_gap_candidates",
        "custody_gap_candidate_boundaries",
        "custody_gap_projection_rows",
        "journal_entries",
        "journal_quarantines",
        "journal_tax_summary",
        "journal_account_holdings",
        "journal_wallet_holdings",
        "journal_quantity_postings",
        "journal_quantity_issues",
        "journal_quantity_balances",
        "journal_custody_decisions",
        "custody_authored_evidence_snapshots",
        "wallet_utxos",
        "wallet_utxo_refreshes",
        "wallet_policy_epochs",
        "wallet_policy_sources",
        "wallet_policy_coverage_witnesses",
        "rates_cache",
        "rates_checked_minutes",
        "freshness_source_states",
        "freshness_jobs",
        "transaction_graph_cache",
        "lightning_node_syncs",
        "lightning_node_records",
        "custody_component_transaction_memberships",
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
_WIF_PRIVATE_KEY = re.compile(r"(?<![A-Za-z0-9])[5KLc9][1-9A-HJ-NP-Za-km-z]{50,51}(?![A-Za-z0-9])")
_RAW_HEX_PRIVATE_KEY = re.compile(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{64}(?![0-9A-Fa-f])")
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
_WATCH_POLICY_FIELDS = frozenset(
    {
        "addresses",
        "chain",
        "network",
        "policy_asset",
        "descriptor_source",
        "synthesize_change",
        "script_types",
        "descriptor",
        "change_descriptor",
        "xpub",
    }
)


def _is_public_watch_material(value: Any) -> bool:
    if not isinstance(value, str) or not value.strip():
        return False
    text = value.strip()
    if (
        _PRIVATE_EXTENDED_KEY.search(text)
        or _WIF_PRIVATE_KEY.search(text)
        or _RAW_HEX_PRIVATE_KEY.search(text)
    ):
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


def merge_public_wallet_config(local: Any, incoming: Any) -> dict[str, Any]:
    """Merge one complete replicated public projection into local config.

    Synchronized fields are snapshot state, not a JSON merge-patch: omission
    removes an old public xpub, address set, script family, or coverage setting.
    Fields outside the positive allowlist remain device-local. A private
    descriptor omitted by serialization is always retained: replication may
    flag a competing public policy for local review, but it must never delete
    device-local secret material or construct a hybrid active policy.
    """

    local_config = dict(local) if isinstance(local, Mapping) else {}
    incoming_config = public_wallet_config(incoming)
    merged = {
        key: value
        for key, value in local_config.items()
        if key not in _SYNC_WALLET_CONFIG_FIELDS
    }
    private_watch_material = {
        key: local_config[key]
        for key in ("descriptor", "change_descriptor", "xpub")
        if key in local_config and not _is_public_watch_material(local_config[key])
    }
    if private_watch_material:
        for key in _WATCH_POLICY_FIELDS:
            incoming_config.pop(key, None)
            if key in local_config:
                merged[key] = local_config[key]
        merged.update(private_watch_material)
    merged.update(incoming_config)
    return merged


def private_wallet_policy_requires_review(local: Any, incoming: Any) -> bool:
    """Return whether a replicated public policy conflicts with local secrets."""

    local_config = dict(local) if isinstance(local, Mapping) else {}
    private_watch_material = any(
        key in local_config and not _is_public_watch_material(local_config[key])
        for key in ("descriptor", "change_descriptor", "xpub")
    )
    if not private_watch_material:
        return False
    incoming_config = public_wallet_config(incoming)
    if any(
        key in incoming_config
        for key in ("addresses", "descriptor", "change_descriptor", "xpub")
    ):
        return True
    return any(
        key in incoming_config
        and incoming_config[key] != local_config.get(key)
        for key in ("chain", "network", "policy_asset", "script_types")
    )


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


_NATIVE_TXID_KEYS = frozenset(
    {"txid", "txhash", "transactionid", "transactionhash", "onchaintxid"}
)


def _canonical_txid(value: Any) -> str | None:
    text = str(value or "").strip().lower()
    return text if re.fullmatch(r"[0-9a-f]{64}", text) else None


def _replicated_external_id_kind(row: Mapping[str, Any]) -> str | None:
    """Return the minimal public type needed to interpret ``external_id``.

    Raw transaction payloads never enter replication. An external identifier
    is labelled as a txid only when the stored label already says so or an
    explicit transaction-hash field in local raw evidence contains that exact
    canonical identifier. A bare 64-hex provider/order id is not enough.
    """

    external_id = _canonical_txid(row["external_id"])
    if external_id is None:
        return None
    declared = str(row["external_id_kind"] or "").strip().lower()
    if declared == "txid":
        return "txid"
    raw = _json_value(row["raw_json"])
    if not isinstance(raw, Mapping):
        return None
    payloads = [raw]
    for key in ("tx", "ownership_graph"):
        nested = raw.get(key)
        if isinstance(nested, Mapping):
            payloads.append(nested)
    for payload in payloads:
        for key, value in payload.items():
            normalized_key = "".join(
                char for char in str(key).lower() if char.isalnum()
            )
            if (
                normalized_key in _NATIVE_TXID_KEYS
                and _canonical_txid(value) == external_id
            ):
                return "txid"
    return None


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
        payload["external_id_kind"] = _replicated_external_id_kind(row)
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
    required = set(spec.columns) - set(spec.optional_columns)
    if table == "transactions":
        required.add("fingerprint_hmac")
    unknown = sorted(set(payload) - allowed)
    missing = sorted(required - set(payload))
    if unknown or missing:
        raise AppError(
            "bundle row does not match the sync schema allowlist",
            code="sync_schema_forbidden",
            details={"table": table, "unknown_fields": unknown, "missing_fields": missing},
            retryable=False,
        )
    if table == "custody_component_legs":
        # Column allowlisting alone cannot protect an older/newer peer from a
        # closed economic role it does not understand. Refuse before SQLite's
        # CHECK constraint (or a later projector KeyError) turns version skew
        # into a partial replay or untyped failure.
        from ..custody_components import LEG_ROLES

        role = str(payload.get("role") or "")
        if role not in LEG_ROLES:
            raise AppError(
                "bundle contains a custody leg role unsupported by this application",
                code="sync_schema_incompatible",
                hint="Upgrade Kassiber on every replica before syncing this component.",
                details={"table": table, "role": role},
                retryable=False,
            )
    if table == "transactions":
        external_id_kind = payload.get("external_id_kind")
        if external_id_kind not in (None, "txid") or (
            external_id_kind == "txid"
            and _canonical_txid(payload.get("external_id")) is None
        ):
            raise AppError(
                "bundle contains an unsupported transaction identity discriminator",
                code="sync_schema_incompatible",
                hint="Upgrade Kassiber on every replica before syncing this transaction.",
                details={
                    "table": table,
                    "external_id_kind": external_id_kind,
                },
                retryable=False,
            )
    return spec


def validate_schema_boundary() -> None:
    overlap = set(SYNC_TABLE_MAP) & NEVER_SYNC_TABLES
    if overlap:
        raise AssertionError(f"never-sync tables entered allowlist: {sorted(overlap)}")
    if len(SYNC_TABLE_MAP) != len(SYNC_TABLES):
        raise AssertionError("duplicate table in sync allowlist")
    invalid_optional = [
        spec.table
        for spec in SYNC_TABLES
        if not spec.optional_columns.issubset(spec.columns)
    ]
    if invalid_optional:
        raise AssertionError(
            f"optional sync columns are outside their table allowlist: {sorted(invalid_optional)}"
        )
    invalid_immutable = [
        spec.table
        for spec in SYNC_TABLES
        if not spec.immutable_fields.issubset(spec.columns)
    ]
    if invalid_immutable:
        raise AssertionError(
            "immutable sync columns are outside their table allowlist: "
            f"{sorted(invalid_immutable)}"
        )


validate_schema_boundary()
