"""Explicit source/projection command contracts, shared by CLI and desktop."""
from ...errors import AppError
from . import artifacts, ledger, opening, projection, projection_views, sources, valuation

READ_ACTIONS = frozenset({"source-preview", "source-get", "source-binding-get", "source-coverage",
    "calculation-get", "projection-policy-get", "projection-get", "projection-controls",
    "projection-events", "projection-list", "projection-policy-list", "opening-preview", "valuation-get", "valuation-list"})
WRITE_ACTIONS = frozenset({"source-capture", "source-bind", "source-void", "calculation-capture",
    "projection-policy-create", "projection-create", "projection-post", "projection-void-quantity",
    "opening-bind", "opening-create", "valuation-create", "valuation-post"})


def execute(conn, profile_id, action, payload):
    ledger.require_book(conn, profile_id)
    contracts = {
        "source-preview": (set(), set(), lambda **p: sources.preview_sources(conn, profile_id)),
        "source-capture": (set(), set(), lambda **p: sources.capture_sources(conn, profile_id)),
        "source-get": ({"snapshot_id"}, {"snapshot_id"}, lambda **p: sources.get_snapshot(conn, profile_id, **p)),
        "source-binding-get": ({"binding_id"}, {"binding_id"}, lambda **p: sources.get_binding(conn, profile_id, **p)),
        "source-coverage": (set(), set(), lambda **p: sources.source_coverage(conn, profile_id)),
        "source-bind": ({"snapshot_id", "expected_digest", "economic_id", "role", "claims", "reason", "idempotency_key"},
            {"snapshot_id", "expected_digest", "economic_id", "role", "claims", "reason", "idempotency_key"}, lambda **p: sources.bind_sources(conn, profile_id, **p)),
        "source-void": ({"binding_id", "reason", "idempotency_key"}, {"binding_id", "reason", "idempotency_key"}, lambda **p: sources.void_binding(conn, profile_id, **p)),
        "calculation-capture": ({"snapshot_id", "period_id", "boundary", "as_of_date"}, {"snapshot_id", "period_id"}, lambda **p: artifacts.capture_calculation(conn, profile_id, **p)),
        "valuation-get": ({"valuation_id"}, {"valuation_id"}, lambda **p: valuation.get_valuation(conn, profile_id, **p)),
        "valuation-list": ({"period_id", "limit", "cursor"}, {"period_id"}, lambda **p: valuation.list_valuations(conn, profile_id, **p)),
        "valuation-create": ({"policy_id", "artifact_id", "period_id", "effective_date", "asset", "adjustment_minor", "evidence_id", "offset_account", "valuation_kind", "reason", "idempotency_key"},
            {"policy_id", "artifact_id", "period_id", "effective_date", "asset", "adjustment_minor", "evidence_id", "offset_account", "valuation_kind", "reason", "idempotency_key"}, lambda **p: valuation.create_valuation(conn, profile_id, **p)),
        "valuation-post": ({"valuation_id", "expected_digest"}, {"valuation_id", "expected_digest"}, lambda **p: valuation.post_valuation(conn, profile_id, **p)),
        "calculation-get": ({"artifact_id"}, {"artifact_id"}, lambda **p: artifacts.get_calculation(conn, profile_id, **p)),
        "projection-policy-create": ({"period_id", "asset_accounts", "transit_accounts", "settlement_account", "income_account", "capital_account", "gain_account", "fee_account", "acknowledge_tax_book_basis", "reason"},
            {"period_id", "asset_accounts", "settlement_account", "income_account", "capital_account", "gain_account", "fee_account", "acknowledge_tax_book_basis", "reason"}, lambda **p: projection.configure_policy(conn, profile_id, **p)),
        "projection-policy-get": ({"policy_id"}, {"policy_id"}, lambda **p: projection.get_policy(conn, profile_id, **p)),
        "projection-create": ({"policy_id", "artifact_id", "binding_id", "event_id", "category", "period_id", "idempotency_key"},
            {"policy_id", "artifact_id", "binding_id", "event_id", "category", "period_id", "idempotency_key"}, lambda **p: projection.create_proposal(conn, profile_id, **p)),
        "projection-get": ({"proposal_id"}, {"proposal_id"}, lambda **p: projection.get_proposal(conn, profile_id, **p)),
        "projection-post": ({"proposal_id", "expected_digest"}, {"proposal_id", "expected_digest"}, lambda **p: projection.post_proposal(conn, profile_id, **p)),
        "projection-void-quantity": ({"proposal_id", "expected_digest", "entry_date", "period_id", "reason"},
            {"proposal_id", "expected_digest", "entry_date", "period_id", "reason"}, lambda **p: projection.void_quantity_proposal(conn, profile_id, **p)),
        "projection-controls": ({"period_id"}, {"period_id"}, lambda **p: _controls(conn, profile_id, **p)),
        "projection-events": ({"artifact_id", "period_id", "limit", "cursor"}, {"artifact_id", "period_id"}, lambda **p: projection_views.list_events(conn, profile_id, **p)),
        "projection-list": ({"period_id", "limit", "cursor"}, {"period_id"}, lambda **p: projection_views.list_proposals(conn, profile_id, **p)),
        "projection-policy-list": ({"period_id", "limit", "cursor"}, {"period_id"}, lambda **p: projection_views.list_policies(conn, profile_id, **p)),
        "opening-preview": ({"artifact_id", "period_id"}, {"artifact_id", "period_id"}, lambda **p: opening.preview_opening(conn, profile_id, **p)),
        "opening-bind": ({"artifact_id", "period_id", "expected_source_digest", "reason", "idempotency_key"}, {"artifact_id", "period_id", "expected_source_digest", "reason", "idempotency_key"}, lambda **p: opening.bind_opening_sources(conn, profile_id, **p)),
        "opening-create": ({"policy_id", "artifact_id", "binding_id", "period_id", "idempotency_key", "additional_balances"}, {"policy_id", "artifact_id", "binding_id", "period_id", "idempotency_key"}, lambda **p: opening.create_opening_proposal(conn, profile_id, **p)),
    }
    if action not in contracts:
        raise AppError("Unknown source projection command", code="accounting_unknown_operation")
    allowed, required, call = contracts[action]
    if not isinstance(payload, dict) or set(payload) - allowed or required - set(payload):
        raise AppError("Invalid source projection command fields", code="accounting_invalid_fields")
    for key, value in payload.items():
        if key.endswith(("_id", "_account")) or key in {"idempotency_key", "expected_digest", "category", "role", "entry_date"}:
            ledger._text(value, key, maximum=1000 if key == "event_id" else 128)
        elif key in {"asset_accounts", "transit_accounts"}:
            if not isinstance(value, dict):
                raise AppError("Account mapping must be an object", code="accounting_invalid_fields")
            for code in value.values():
                ledger._text(code, "account_code", maximum=64)
    return call(**payload)


def _controls(conn, profile_id, *, period_id):
    period = ledger._period(conn, profile_id, period_id)
    return projection.validate_close(conn, profile_id, period["start_date"], period["end_date"])
