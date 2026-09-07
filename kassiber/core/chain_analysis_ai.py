"""Provider projection for local chain investigations.

Remote models receive graph topology and exact amounts with process/book scoped
opaque references. A reference is a query handle, not anonymized public data:
amounts and topology can themselves be identifying. No public chain identifiers,
free-text attribution, descriptors or raw payloads cross this projection.
Acquisition artifacts must stay local-provider only; do not project and replay
an exact, version-bound acquisition plan through this interface.
"""
from __future__ import annotations

import hashlib
import hmac
import re
import secrets
import sqlite3
from typing import Any, Mapping
import uuid

from ..errors import AppError
from .chain_analysis import build_index
from .chain_analysis_runtime import scope_key as _scope

_PROCESS_KEY = secrets.token_bytes(32)
_REFERENCE_PREFIX = "ca-ref:"
_CODE_RE = re.compile(r"[a-z][a-z0-9_]*(?:-[a-z0-9_]+)*\Z")
_HANDLE_FIELDS = {
    "snapshot_id", "expected_snapshot_id", "base_snapshot_id", "current_snapshot_id",
    "case_id", "base_id", "other_id", "next_cursor", "cursor", "result_digest", "status_commitment",
    "job_id", "dataset_id", "source_token", "psbt_token", "expected_active_id", "content_sha256", "dataset_state_digest", "state_digest",
}
_REFERENCE_FIELDS = {
    "node_id", "edge_id", "input_id", "output_id", "transaction_node_id",
    "wallet_id", "transaction_id", "relation_id", "component_id", "reference",
    "subject", "target",
    "subject_id", "original_subject",
}
_REFERENCE_LISTS = {
    "node_ids", "edge_ids", "input_ids", "output_ids", "wallet_ids",
    "transaction_ids", "component_ids", "subjects",
    "inputs_added", "inputs_removed",
}
_CODE_FIELDS = {
    "kind", "code", "rule", "rule_version", "model", "status", "reason",
    "evidence_level", "level", "observer", "direction", "direction_from_label",
    "chain", "network", "scope", "authority", "basis_state", "confidence",
    "category", "comparison", "mode", "egress", "filter", "depth_unit",
    "ai_reference_scope", "source_kind", "observer_knowledge", "amount_visibility", "public_liquid_value_policy", "severity",
    "protocol", "phase", "algorithm", "availability", "field", "unit", "receipt_scope", "error_code", "visibility", "dataset_status", "format", "adapter", "network_source", "utxo_evidence", "value", "by_index",
}
_CODE_LISTS = {
    "rules", "premises", "assumptions", "limitations", "path_kinds",
    "stopped_reasons", "missing_tables",
    "feature_codes", "contradictions", "findings_added", "findings_removed",
}
_SOURCE_CODES = {
    "stored_transaction", "wallet_inventory", "reference_cache", "local_label",
    "journal_custody_decisions", "journal_custody_economic_relations",
    "reviewed_or_imported_privacy_boundary", "invalid_privacy_metadata",
    "equal_output_coinjoin", "large_equal_output_coinjoin", "local_acquisition", "observer_exclusion",
    "psbt", "psbt_proposal", "psbt_supplied", "local_dataset",
}
# Unknown additions are omitted until their disclosure has been considered.
# Numeric dictionary keys are also admitted for participant-group counts.
_FIELDS = set("""
    schema_version query summary nodes edges findings clusters patterns exposure
    entropy paths frontier coverage capabilities result items id created_at
    updated_at occurred_at confirmed_at observed_at confirmation_observed_at confirmations confirmed block_height deleted revision expected_revision claim
    hidden_private_node_count hidden_private_relation_count private_labels_withheld
    evidence boundary_evidence conditional_on_boundary_interpretation
    amount_msat target_amount_msat fee_msat min_amount_msat asset target_asset
    source_asset interpretation_count interpretation_count_lower_bound entropy_bits
    link_counts deterministic_links states_explored participant_group_counts
    examples examples_truncated conditional_on_model limits max_states
    max_duration_ms max_inputs max_outputs complete local_only missing stale
    budget_exhausted topology_complete amount_complete_transaction_count
    selected_node_count hypotheses_enabled hypothesis_count pattern_count label_count
    source_rows node_count edge_count transaction_count output_count record_count
    path_count visited_node_count inspected_edge_count invalid_observations
    cache_rejected custody_fresh missing_node_count conflicting_node_count reference_reconciling_count
    complete_transaction_count reference_node_count frontier_omitted_count
    pruning count analytics depth node_limit edge_limit include_relations
    include_hypotheses start end reversible accounting_authority taint_inference
    cluster_defining changed comparison base current added_nodes removed_nodes
    changed_nodes added_edges removed_edges changed_edges before after
    added_findings removed_findings changed_findings added_clusters removed_clusters changed_clusters
    added_patterns removed_patterns changed_patterns added_exposure removed_exposure changed_exposure
    added_transaction_features removed_transaction_features changed_transaction_features coverage_changed
    local_graph backward_forward_trace bounded_alternative_paths
    cross_rail_custody_relations network_acquisition taint_attribution
    global_chain_completeness labels error retryable details
    wallet_inventory stored_transaction transactions wallets wallet_utxos
    transaction_graph_cache journal_custody_decisions journal_custody_economic_relations
    chain_analysis_labels chain_analysis_observations
    transaction_features features extractor_version scenario computation cache_hits
    request progress cancel_requested elapsed_ms phase job_id error_code receipt_scope
    max_received_fee_msat max_paid_fee_msat completed input_count output_count
    dataset_id datasets manifest content_sha256 dataset_state_digest state_digest
    byte_count row_count record_number claims dataset_version visibility dataset_status
    match_count subject_count active_dataset_count inactive_by_date_count lookup_queries
    match_limit match_limit_per_visibility truncated complete_chain_coverage valid_from valid_until
    scripts_skipped subject_limit subjects_truncated
    transaction_facts inputs outputs totals psbt_version validation input_msat output_msat
    final_vsize final_fee_rate_sat_vb known_input_amounts previous_transaction_hashes_verified
    missing_input_indices network_source network_verified chain_membership_verified
    unspentness_verified signatures_verified input_metadata output_derivation_metadata_present
    global_xpubs_present transaction_modifiable_flags utxo_evidence input_index output_index
    delta inputs_added inputs_removed output_scripts_added output_scripts_removed
    features_changed findings_added findings_removed payjoin checks additional_input_count
    receiver_contribution_msat negotiation_verified safe_to_sign metadata
    payment_output_substitution_allowed
    feature_codes contradictions value values signals_bip125 relative_locks minimum
    relative_lock_interpretation_available enabled counts by_index observations sighash
    low_r low_s verified equal_groups round_1000_sat_count bip69_value_script_order
    p2pkh p2sh p2wpkh p2wsh p2tr op_return p2pk witness_unknown other unknown
""".split()) | _REFERENCE_FIELDS | _REFERENCE_LISTS | _HANDLE_FIELDS | _CODE_FIELDS | _CODE_LISTS | {"source"}


def _reference(scope: bytes, value: str) -> str:
    return _REFERENCE_PREFIX + hmac.new(_PROCESS_KEY, scope + b"\x00" + value.encode(), hashlib.sha256).hexdigest()


def _handle(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    try:
        return str(uuid.UUID(value)) == value.lower()
    except ValueError:
        return False


def project_ai_result(conn: sqlite3.Connection, profile_id: str, value: Any) -> Any:
    """Project a curated engine/case/label result for a remote provider.

    Call the existing generic ``redact_ai_tool_result`` afterwards too. Local
    providers may receive the original curated result without this projection.
    This function performs no data writes or graph/source scans.
    """
    scope = _scope(conn, profile_id)
    omitted = object()

    def project(item: Any, field: str = "", parent: Mapping | None = None) -> Any:
        if isinstance(item, Mapping):
            result = {}
            for key, child in item.items():
                if not isinstance(key, str) or key not in _FIELDS and not key.isdecimal():
                    continue
                projected = project(child, key, item)
                if projected is not omitted:
                    result[key] = projected
            return result
        if isinstance(item, (tuple, list)):
            values = [project(child, field, parent) for child in item]
            return [child for child in values if child is not omitted]
        if item is None or isinstance(item, (int, float, bool)):
            return item
        if not isinstance(item, str):
            return omitted
        if field in _REFERENCE_FIELDS or field in _REFERENCE_LISTS:
            return _reference(scope, item)
        if field == "id":
            # Case and label UUIDs are authored API handles, not chain identities.
            return item if _handle(item) else _reference(scope, item)
        if field in _HANDLE_FIELDS:
            return item
        if field == "source":
            if parent and parent.get("kind") in {"creates", "spends", "custody", "hypothesis"} and "target" in parent:
                return _reference(scope, item)
            return item if item in _SOURCE_CODES else omitted
        if field in _CODE_FIELDS or field in _CODE_LISTS:
            safe_code = len(item) <= 100 and _CODE_RE.fullmatch(item) and not re.fullmatch(r"[a-f0-9]{64}", item) and not item.startswith(("bc1", "tb1", "bcrt1", "ex1", "lq1", "tex1", "tlq1", "ert1", "el1", "xpub", "tpub", "ypub", "zpub"))
            return item if safe_code else omitted
        if field in {"asset", "target_asset", "source_asset"}:
            return item if re.fullmatch(r"[A-Z]{2,8}", item) else "unknown_asset"
        if field.endswith("_msat") or field in {"interpretation_count", "interpretation_count_lower_bound"} or field.isdecimal():
            return item if re.fullmatch(r"-?(?:0|[1-9][0-9]*)", item) else omitted
        if field == "final_fee_rate_sat_vb":
            return item if re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", item) else omitted
        if field in {"created_at", "updated_at", "occurred_at", "confirmed_at", "observed_at", "confirmation_observed_at", "start", "end", "valid_from", "valid_until"}:
            return item if re.fullmatch(r"[0-9T:.+Z-]{10,40}", item) else omitted
        return omitted

    result = project(value)
    if isinstance(result, dict):
        result["ai_reference_scope"] = "current_process_and_book"
        result["ai_projection"] = "opaque_graph_references_no_public_chain_identifiers"
    return result


def decode_ai_args(conn: sqlite3.Connection, profile_id: str, args: Mapping[str, Any]) -> dict[str, Any]:
    """Resolve only subject/target references within the active local book.

    Resolution intentionally refreshes against current observations/labels.
    Saved cases remain readable after changes, but stale graph query handles
    cannot silently retarget another subject. Raw user-provided subjects remain
    accepted and are validated by the canonical query/label implementation.
    """
    if not isinstance(args, Mapping):
        raise AppError("Chain analysis arguments must be an object", code="validation", retryable=False)
    refs = set()

    def collect(value: Any, key: str = "") -> None:
        if isinstance(value, Mapping):
            for name, child in value.items():
                collect(child, name)
        elif isinstance(value, (list, tuple)):
            for child in value:
                collect(child, key)
        elif key in {"subject", "target"} and isinstance(value, str) and value.startswith(_REFERENCE_PREFIX):
            refs.add(value)

    collect(args)
    lookup = {}
    if refs:
        scope = _scope(conn, profile_id)
        index = build_index(conn, profile_id)
        subjects = set(index.subjects) | set(index.nodes)
        subjects.update(row["subject"] for row in index.labels if isinstance(row.get("subject"), str) and not row.get("deleted"))
        for subject in subjects:
            reference = _reference(scope, subject)
            if reference in refs:
                lookup[reference] = subject
        if refs - set(lookup):
            raise AppError("Analysis reference expired or is not available in this book; refresh the investigation", code="chain_analysis_reference_stale", retryable=True)

    def decode(value: Any, key: str = "") -> Any:
        if isinstance(value, Mapping):
            return {name: decode(child, name) for name, child in value.items()}
        if isinstance(value, (list, tuple)):
            return [decode(child, key) for child in value]
        if key in {"subject", "target"} and isinstance(value, str):
            return lookup.get(value, value)
        return value

    return decode(args)
