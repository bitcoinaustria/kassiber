"""Atomic custody interpretation components above imported transactions.

Imported ``transactions`` remain immutable evidence anchors.  A custody
component is a versioned interpretation of one or more source/destination legs
that only becomes effective when the whole component conserves exactly.  The
module deliberately knows nothing about RP2 or CLI handlers so future Bitcoin
layers can contribute rail-specific evidence without creating back-edges.

Two states are exposed for reads:

``state``
    The authored lifecycle value stored in SQLite (draft/active/superseded).
``effective_state``
    ``active`` only when the complete current leg set validates and no other
    authored active component claims one of its transaction anchors.  This is
    the state projection uses to decide whether it may emit a custody move.  A
    journal run still loads every authored-active component so a header arriving
    before all of its legs claims the anchors already present and quarantines
    them fail-closed instead of silently restoring the raw tax interpretation.
"""

from __future__ import annotations

from collections import defaultdict
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
import json
import re
import sqlite3
from typing import Any, Iterable, Iterator, Mapping, Sequence
import uuid

from ..errors import AppError
from ..time_utils import parse_iso_datetime_or_none, parse_timestamp
from ..transfers import (
    LIGHTNING_INFERENCE_WALLET_KINDS,
    normalize_wallet_kind_alias,
    onchain_transfer_scope,
)
from ..wallet_descriptors import normalize_asset_code, normalize_chain, normalize_network


COMPONENT_TYPES = frozenset(
    {
        "native_transfer",
        "channel_lifecycle",
        "peg",
        "swap",
        "refund",
        "manual_bridge",
    }
)
COMPONENT_STATES = frozenset({"draft", "active", "superseded"})
CONSERVATION_MODES = frozenset({"quantity", "conversion"})
AUTHORED_SOURCES = frozenset({"user", "cli", "gui", "ai_tool"})
# Block timestamps are not a strict sequence clock, and exchange/L2 records may
# stamp completion after the receiving chain transaction. Custody components are
# reviewed exact allocations, so tolerate bounded evidence-clock skew while
# still rejecting materially reversed routes.
CUSTODY_CHRONOLOGY_SKEW_TOLERANCE = timedelta(days=7)
LEG_ROLES = frozenset(
    {"source", "destination", "fee", "external", "retained", "unresolved"}
)
SINK_ROLES = LEG_ROLES - {"source"}

_TOKEN_RE = re.compile(r"^[a-z][a-z0-9_.:-]*$")
_UNSET = object()
_SQLITE_MAX_INTEGER = (1 << 63) - 1

# Quantity conservation can cross Bitcoin rails, but it can never cross a
# physical network boundary. Keep this mapping here (rather than in tax
# policy) because mainnet/testnet/regtest identity is a property of the value
# movement itself. New Bitcoin layers can participate by naming their base
# chain/network; no country-specific rule is involved.
_RAIL_ALIASES = {
    "btc": "bitcoin",
    "onchain": "bitcoin",
    "elements": "liquid",
    "lbtc": "liquid",
    "cln": "lightning",
    "coreln": "lightning",
    "lnd": "lightning",
    "nwc": "lightning",
}
_RAIL_BASE_CHAIN = {
    "bitcoin": "bitcoin",
    "lightning": "bitcoin",
    "liquid": "liquid",
}
_NETWORK_DOMAIN = {
    ("bitcoin", "main"): "main",
    ("liquid", "liquidv1"): "main",
    ("bitcoin", "test"): "test",
    ("liquid", "liquidtestnet"): "test",
    ("bitcoin", "regtest"): "regtest",
    ("liquid", "elementsregtest"): "regtest",
    ("bitcoin", "signet"): "signet",
}
_GENERIC_NETWORK_DOMAIN = {
    "bitcoin": "main",
    "liquid": "main",
    "liquidv1": "main",
    "main": "main",
    "mainnet": "main",
    "test": "test",
    "testnet": "test",
    "liquidtestnet": "test",
    "elements": "regtest",
    "elementsregtest": "regtest",
    "regtest": "regtest",
    "signet": "signet",
}


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def _error(message: str, code: str, *, details: Mapping[str, Any] | None = None) -> AppError:
    return AppError(message, code=code, details=dict(details or {}), retryable=False)


def _required_text(value: Any, field: str, *, token: bool = False) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _error(
            f"{field} is required",
            "custody_component_validation",
            details={"field": field},
        )
    text = value.strip()
    if token and not _TOKEN_RE.fullmatch(text):
        raise _error(
            f"{field} must be a lowercase stable token",
            "custody_component_validation",
            details={"field": field, "value": text},
        )
    return text


def _optional_text(value: Any, field: str, *, token: bool = False) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _error(
            f"{field} must be text",
            "custody_component_validation",
            details={"field": field},
        )
    text = value.strip()
    if not text:
        return None
    if token and not _TOKEN_RE.fullmatch(text):
        raise _error(
            f"{field} must be a lowercase stable token",
            "custody_component_validation",
            details={"field": field, "value": text},
        )
    return text


def _exact_nonnegative_int(value: Any, field: str) -> int:
    # JSON numbers above JavaScript's safe-integer limit cannot survive the
    # desktop renderer boundary.  Accept their canonical lossless wire form as
    # an unsigned decimal string while retaining integers for existing callers.
    # bool is an int subclass but is never a financial quantity.
    if type(value) is int:
        parsed = value
    elif isinstance(value, str) and re.fullmatch(r"[0-9]+", value):
        normalized = value.lstrip("0") or "0"
        if len(normalized) > 19:
            raise _error(
                f"{field} must fit SQLite's non-negative integer range",
                "custody_component_validation",
                details={"field": field, "value": value},
            )
        parsed = int(normalized, 10)
    else:
        raise _error(
            f"{field} must be an exact non-negative integer",
            "custody_component_validation",
            details={"field": field, "value": value},
        )
    if parsed < 0 or parsed > _SQLITE_MAX_INTEGER:
        raise _error(
            f"{field} must fit SQLite's non-negative integer range",
            "custody_component_validation",
            details={"field": field, "value": value},
        )
    return parsed


def _json_object(value: Any, field: str) -> dict[str, Any]:
    if value in (None, ""):
        return {}
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except json.JSONDecodeError as exc:
            raise _error(
                f"{field} must contain valid JSON",
                "custody_component_validation",
                details={"field": field},
            ) from exc
    if not isinstance(value, Mapping):
        raise _error(
            f"{field} must be a JSON object",
            "custody_component_validation",
            details={"field": field},
        )
    # Round-trip now so unsupported/non-deterministic objects fail at the API
    # boundary rather than halfway through an authored SQL transaction.
    try:
        return json.loads(json.dumps(dict(value), sort_keys=True, separators=(",", ":")))
    except (TypeError, ValueError) as exc:
        raise _error(
            f"{field} contains a value that cannot be encoded as JSON",
            "custody_component_validation",
            details={"field": field},
        ) from exc


def _json_text(value: Any, field: str) -> str:
    return json.dumps(_json_object(value, field), sort_keys=True, separators=(",", ":"))


@contextmanager
def _savepoint(conn: sqlite3.Connection):
    """Provide nested atomicity without committing the caller's transaction."""

    name = f"custody_{uuid.uuid4().hex}"
    conn.execute(f"SAVEPOINT {name}")
    try:
        yield
    except Exception:
        conn.execute(f"ROLLBACK TO {name}")
        conn.execute(f"RELEASE {name}")
        raise
    else:
        conn.execute(f"RELEASE {name}")


def _normalize_component_type(value: Any) -> str:
    component_type = _required_text(value, "component_type", token=True)
    if component_type not in COMPONENT_TYPES:
        raise _error(
            "component_type is not supported",
            "custody_component_validation",
            details={"component_type": component_type, "supported": sorted(COMPONENT_TYPES)},
        )
    return component_type


def _normalize_mode(value: Any) -> str:
    mode = _required_text(value, "conservation_mode", token=True)
    if mode not in CONSERVATION_MODES:
        raise _error(
            "conservation_mode is not supported",
            "custody_component_validation",
            details={"conservation_mode": mode, "supported": sorted(CONSERVATION_MODES)},
        )
    return mode


def _normalize_leg(raw: Mapping[str, Any], ordinal: int) -> dict[str, Any]:
    if not isinstance(raw, Mapping):
        raise _error(
            "each custody leg must be an object",
            "custody_component_validation",
            details={"ordinal": ordinal},
        )
    role = _required_text(raw.get("role"), "role", token=True)
    if role not in LEG_ROLES:
        raise _error(
            "custody leg role is not supported",
            "custody_component_validation",
            details={"ordinal": ordinal, "role": role, "supported": sorted(LEG_ROLES)},
        )
    amount = _exact_nonnegative_int(raw.get("amount_msat"), "amount_msat")
    valuation_unit = _optional_text(raw.get("valuation_unit"), "valuation_unit", token=True)
    raw_valuation = raw.get("valuation_amount")
    valuation_amount = (
        None
        if raw_valuation is None
        else _exact_nonnegative_int(raw_valuation, "valuation_amount")
    )
    if (valuation_unit is None) != (valuation_amount is None):
        raise _error(
            "valuation_unit and valuation_amount must be supplied together",
            "custody_component_validation",
            details={"ordinal": ordinal},
        )
    transaction_id = _optional_text(raw.get("transaction_id"), "transaction_id")
    anchor_transaction_id = _optional_text(
        raw.get("anchor_transaction_id"), "anchor_transaction_id"
    )
    if (
        transaction_id is not None
        and anchor_transaction_id is not None
        and transaction_id != anchor_transaction_id
    ):
        raise _error(
            "anchor_transaction_id must identify the live transaction anchor",
            "custody_component_validation",
            details={"ordinal": ordinal},
        )
    occurred_at = _optional_text(raw.get("occurred_at"), "occurred_at")
    if occurred_at is not None:
        occurred_at = parse_timestamp(occurred_at)
    return {
        "id": _optional_text(raw.get("id"), "id") or str(uuid.uuid4()),
        "ordinal": ordinal,
        "role": role,
        "rail": _required_text(raw.get("rail"), "rail", token=True),
        "chain": _optional_text(raw.get("chain"), "chain", token=True),
        "network": _optional_text(raw.get("network"), "network", token=True),
        "asset": _required_text(raw.get("asset"), "asset"),
        "exposure": _required_text(raw.get("exposure"), "exposure", token=True),
        "conservation_unit": _required_text(
            raw.get("conservation_unit"), "conservation_unit", token=True
        ),
        "amount_msat": amount,
        "valuation_unit": valuation_unit,
        "valuation_amount": valuation_amount,
        "occurred_at": occurred_at,
        "transaction_id": transaction_id,
        "anchor_transaction_id": anchor_transaction_id or transaction_id,
        "wallet_id": _optional_text(raw.get("wallet_id"), "wallet_id"),
        "location_ref": _optional_text(raw.get("location_ref"), "location_ref"),
        "notes": _optional_text(raw.get("notes"), "notes"),
    }


def normalize_legs(legs: Iterable[Mapping[str, Any]]) -> list[dict[str, Any]]:
    try:
        raw_legs = list(legs)
    except TypeError as exc:
        raise _error(
            "legs must be a sequence",
            "custody_component_validation",
        ) from exc
    normalized = [_normalize_leg(raw, ordinal) for ordinal, raw in enumerate(raw_legs)]
    ids = [leg["id"] for leg in normalized]
    if len(ids) != len(set(ids)):
        raise _error(
            "custody leg ids must be unique",
            "custody_component_validation",
        )
    return normalized


def normalize_allocations(
    allocations: Iterable[Mapping[str, Any]] | None,
    legs: Sequence[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    if allocations is None:
        return []
    try:
        raw_allocations = list(allocations)
    except TypeError as exc:
        raise _error(
            "allocations must be a sequence",
            "custody_component_validation",
        ) from exc
    by_id = {str(leg["id"]): leg for leg in legs}
    by_ordinal = {int(leg["ordinal"]): leg for leg in legs}
    normalized: list[dict[str, Any]] = []
    for ordinal, raw in enumerate(raw_allocations):
        if not isinstance(raw, Mapping):
            raise _error(
                "each custody allocation must be an object",
                "custody_component_validation",
                details={"ordinal": ordinal},
            )

        def resolve_leg(kind: str) -> Mapping[str, Any]:
            leg_id = _optional_text(raw.get(f"{kind}_leg_id"), f"{kind}_leg_id")
            if leg_id is not None:
                leg = by_id.get(leg_id)
            else:
                raw_ordinal = raw.get(f"{kind}_ordinal")
                if type(raw_ordinal) is not int:
                    raise _error(
                        f"{kind}_leg_id or {kind}_ordinal is required",
                        "custody_component_validation",
                        details={"allocation_ordinal": ordinal},
                    )
                leg = by_ordinal.get(raw_ordinal)
            if leg is None:
                raise _error(
                    f"allocation {kind} leg was not found",
                    "custody_component_validation",
                    details={"allocation_ordinal": ordinal},
                )
            return leg

        source = resolve_leg("source")
        sink = resolve_leg("sink")
        if source["role"] != "source" or sink["role"] == "source":
            raise _error(
                "allocation edges must run from a source leg to a sink leg",
                "custody_component_validation",
                details={"allocation_ordinal": ordinal},
            )
        normalized.append(
            {
                "id": _optional_text(raw.get("id"), "id") or str(uuid.uuid4()),
                "ordinal": ordinal,
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                "source_amount_msat": _exact_nonnegative_int(
                    raw.get("source_amount_msat"), "source_amount_msat"
                ),
                "sink_amount_msat": _exact_nonnegative_int(
                    raw.get("sink_amount_msat"), "sink_amount_msat"
                ),
            }
        )
    ids = [allocation["id"] for allocation in normalized]
    edges = [
        (allocation["source_leg_id"], allocation["sink_leg_id"])
        for allocation in normalized
    ]
    if len(ids) != len(set(ids)) or len(edges) != len(set(edges)):
        raise _error(
            "custody allocation ids and source/sink edges must be unique",
            "custody_component_validation",
        )
    return normalized


def _canonical_rail(value: Any) -> str:
    rail = str(value or "").strip().lower().replace("_", "-")
    return _RAIL_ALIASES.get(rail, rail)


def _quantity_network_scope(leg: Mapping[str, Any]) -> dict[str, Any]:
    """Return a country-neutral physical network domain for a custody leg.

    ``domain=None`` means historical evidence does not identify a network; it
    is not silently guessed. ``valid=False`` means the leg's own authored
    rail/chain/network fields contradict each other and therefore cannot prove
    a quantity-preserving edge.
    """

    rail = _canonical_rail(leg.get("rail"))
    expected_chain = _RAIL_BASE_CHAIN.get(rail)
    raw_chain = leg.get("chain")
    chain: str | None = None
    if raw_chain not in (None, ""):
        try:
            chain = normalize_chain(raw_chain)
        except ValueError:
            if expected_chain is not None:
                return {
                    "valid": False,
                    "rail": rail,
                    "chain": raw_chain,
                    "network": leg.get("network"),
                    "domain": None,
                    "reason": "unsupported_chain",
                }
            # Future rails may use a chain token Kassiber does not know yet.
            # Preserve that identity instead of rejecting the extension point;
            # an explicit Bitcoin-domain network can still participate in the
            # country-neutral compatibility check below.
            chain = str(raw_chain).strip().lower()
    if chain is not None and expected_chain is not None and chain != expected_chain:
        return {
            "valid": False,
            "rail": rail,
            "chain": chain,
            "network": leg.get("network"),
            "domain": None,
            "reason": "rail_chain_mismatch",
        }
    chain = chain or expected_chain
    raw_network = leg.get("network")
    if raw_network in (None, ""):
        return {
            "valid": True,
            "rail": rail,
            "chain": chain,
            "network": None,
            "domain": None,
        }

    network_text = str(raw_network).strip().lower()
    if chain in {"bitcoin", "liquid"}:
        try:
            network = normalize_network(chain, network_text)
        except ValueError:
            return {
                "valid": False,
                "rail": rail,
                "chain": chain,
                "network": raw_network,
                "domain": None,
                "reason": "network_invalid_for_chain",
            }
        return {
            "valid": True,
            "rail": rail,
            "chain": chain,
            "network": network,
            "domain": _NETWORK_DOMAIN.get((chain, network)),
        }

    # An untracked/future rail may still carry an explicit canonical Bitcoin
    # network name. This preserves extensibility without inventing a default.
    return {
        "valid": True,
        "rail": rail,
        "chain": chain,
        "network": network_text,
        "domain": _GENERIC_NETWORK_DOMAIN.get(network_text),
    }


def _inferred_allocation_leg_pairs(
    positive_sources: Sequence[Mapping[str, Any]],
    positive_sinks: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> list[tuple[Mapping[str, Any], Mapping[str, Any]]]:
    """Mirror the journal projector's allocation inference for validation."""

    if conservation_mode == "conversion":
        return (
            [(positive_sources[0], positive_sinks[0])]
            if len(positive_sources) == 1 and len(positive_sinks) == 1
            else []
        )
    if len(positive_sources) == 1:
        return [(positive_sources[0], sink) for sink in positive_sinks]
    owned_sinks = [
        sink
        for sink in positive_sinks
        if sink.get("role") in {"destination", "retained"}
    ]
    attributed_sinks = [
        sink
        for sink in positive_sinks
        if sink.get("role") in {"fee", "external", "unresolved"}
    ]
    if len(owned_sinks) == 1 and not attributed_sinks:
        return [(source, owned_sinks[0]) for source in positive_sources]
    return []


def _quantity_allocation_scope_issues(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> list[dict[str, Any]]:
    """Reject quantity allocations that cross incompatible network domains."""

    if conservation_mode != "quantity":
        return []
    by_id = {str(leg["id"]): leg for leg in legs}
    pairs: list[tuple[Any, Mapping[str, Any], Mapping[str, Any]]] = []
    if allocations:
        for allocation in allocations:
            source = by_id.get(str(allocation.get("source_leg_id")))
            sink = by_id.get(str(allocation.get("sink_leg_id")))
            if source is None or sink is None:
                continue
            pairs.append((allocation.get("id"), source, sink))
    else:
        positive_sources = [
            leg
            for leg in legs
            if leg.get("role") == "source"
            and int(leg.get("amount_msat") or 0) > 0
        ]
        positive_sinks = [
            leg
            for leg in legs
            if leg.get("role") in SINK_ROLES
            and int(leg.get("amount_msat") or 0) > 0
        ]
        pairs = [
            (None, source, sink)
            for source, sink in _inferred_allocation_leg_pairs(
                positive_sources,
                positive_sinks,
                conservation_mode=conservation_mode,
            )
        ]

    issues: list[dict[str, Any]] = []
    for allocation_id, source, sink in pairs:
        source_scope = _quantity_network_scope(source)
        sink_scope = _quantity_network_scope(sink)
        issue_base = {
            "allocation_id": allocation_id,
            "source_leg_id": source["id"],
            "sink_leg_id": sink["id"],
        }
        if not source_scope["valid"] or not sink_scope["valid"]:
            issues.append(
                {
                    "code": "allocation_network_scope_invalid",
                    **issue_base,
                    "source_scope": source_scope,
                    "sink_scope": sink_scope,
                }
            )
            continue
        source_domain = source_scope.get("domain")
        sink_domain = sink_scope.get("domain")
        if (
            source_domain is not None
            and sink_domain is not None
            and source_domain != sink_domain
        ):
            issues.append(
                {
                    "code": "allocation_network_mismatch",
                    **issue_base,
                    "source_network_domain": source_domain,
                    "sink_network_domain": sink_domain,
                    "source_scope": source_scope,
                    "sink_scope": sink_scope,
                }
            )
    return issues


def _allocation_pairs(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> list[tuple[Any, Mapping[str, Any], Mapping[str, Any], int, int]]:
    """Materialize explicit or projector-inferred allocation endpoints."""

    by_id = {str(leg["id"]): leg for leg in legs}
    pairs: list[tuple[Any, Mapping[str, Any], Mapping[str, Any], int, int]] = []
    if allocations:
        for allocation in allocations:
            source = by_id.get(str(allocation.get("source_leg_id")))
            sink = by_id.get(str(allocation.get("sink_leg_id")))
            if source is None or sink is None:
                continue
            pairs.append(
                (
                    allocation.get("id"),
                    source,
                    sink,
                    int(allocation.get("source_amount_msat") or 0),
                    int(allocation.get("sink_amount_msat") or 0),
                )
            )
        return pairs

    positive_sources = [
        leg
        for leg in legs
        if leg.get("role") == "source" and int(leg.get("amount_msat") or 0) > 0
    ]
    positive_sinks = [
        leg
        for leg in legs
        if leg.get("role") in SINK_ROLES and int(leg.get("amount_msat") or 0) > 0
    ]
    for source, sink in _inferred_allocation_leg_pairs(
        positive_sources,
        positive_sinks,
        conservation_mode=conservation_mode,
    ):
        pairs.append(
            (
                None,
                source,
                sink,
                int(source.get("amount_msat") or 0),
                int(sink.get("amount_msat") or 0),
            )
        )
    return pairs


def _allocations_with_component_inference(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> list[dict[str, Any]]:
    """Preserve each component's 1:N/N:1 inference in a combined graph."""

    legs_by_component: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for leg in legs:
        component_id = str(leg.get("component_id") or "__single__")
        legs_by_component[component_id].append(leg)
    allocations_by_component: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for allocation in allocations:
        component_id = str(allocation.get("component_id") or "__single__")
        allocations_by_component[component_id].append(allocation)

    complete: list[dict[str, Any]] = []
    for component_id, component_legs in sorted(legs_by_component.items()):
        explicit = allocations_by_component.get(component_id, [])
        if explicit:
            complete.extend(dict(allocation) for allocation in explicit)
            continue
        positive_sources = [
            leg
            for leg in component_legs
            if leg.get("role") == "source"
            and int(leg.get("amount_msat") or 0) > 0
        ]
        positive_sinks = [
            leg
            for leg in component_legs
            if leg.get("role") in SINK_ROLES
            and int(leg.get("amount_msat") or 0) > 0
        ]
        for ordinal, (source, sink) in enumerate(
            _inferred_allocation_leg_pairs(
                positive_sources,
                positive_sinks,
                conservation_mode=conservation_mode,
            )
        ):
            complete.append(
                {
                    "id": f"inferred:{component_id}:{ordinal}",
                    "component_id": component_id,
                    "source_leg_id": source["id"],
                    "sink_leg_id": sink["id"],
                    "source_amount_msat": int(source.get("amount_msat") or 0),
                    "sink_amount_msat": int(sink.get("amount_msat") or 0),
                }
            )
    return complete


def _allocation_chronology_issues(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> list[dict[str, Any]]:
    """Reject value edges whose destination predates their source."""

    issues: list[dict[str, Any]] = []
    for allocation_id, source, sink, source_amount, sink_amount in _allocation_pairs(
        legs,
        allocations,
        conservation_mode=conservation_mode,
    ):
        if source_amount <= 0 and sink_amount <= 0:
            continue
        source_when = parse_iso_datetime_or_none(source.get("occurred_at"))
        sink_when = parse_iso_datetime_or_none(sink.get("occurred_at"))
        if (
            source_when is None
            or sink_when is None
            or source_when <= sink_when + CUSTODY_CHRONOLOGY_SKEW_TOLERANCE
        ):
            continue
        issues.append(
            {
                "code": "allocation_chronology_mismatch",
                "message": "a custody allocation destination predates its source",
                "allocation_id": allocation_id,
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                "source_occurred_at": source.get("occurred_at"),
                "sink_occurred_at": sink.get("occurred_at"),
            }
        )
    return issues


def _quantity_scope_connectivity_issues(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> list[dict[str, Any]]:
    """Propagate known network domains through unknown custody locations.

    An unknown leg is a valid representation of missing evidence, but it is
    not a scope reset. Allocation edges plus receive/spend legs at the same
    custody location form one physical route. If that route reaches two known
    incompatible Bitcoin network domains, the component must remain draft.
    """

    if conservation_mode != "quantity":
        return []
    material_legs = [
        leg for leg in legs if int(leg.get("amount_msat") or 0) > 0
    ]
    by_id = {str(leg["id"]): leg for leg in material_legs}
    parent = {leg_id: leg_id for leg_id in by_id}

    def find(leg_id: str) -> str:
        root = leg_id
        while parent[root] != root:
            root = parent[root]
        while parent[leg_id] != leg_id:
            next_id = parent[leg_id]
            parent[leg_id] = root
            leg_id = next_id
        return root

    def union(left: str, right: str) -> None:
        if left not in parent or right not in parent:
            return
        left_root = find(left)
        right_root = find(right)
        if left_root != right_root:
            parent[max(left_root, right_root)] = min(left_root, right_root)

    for _allocation_id, source, sink, source_amount, sink_amount in _allocation_pairs(
        material_legs,
        allocations,
        conservation_mode=conservation_mode,
    ):
        if source_amount > 0 or sink_amount > 0:
            union(str(source["id"]), str(sink["id"]))

    by_location: dict[tuple[str, str, str], list[str]] = defaultdict(list)
    for leg in material_legs:
        if (
            leg.get("wallet_id") in (None, "")
            or leg.get("role") not in {"source", "destination", "retained"}
        ):
            continue
        location_key = (
            str(leg["wallet_id"]),
            str(leg.get("exposure") or ""),
            str(leg.get("conservation_unit") or ""),
        )
        by_location[location_key].append(str(leg["id"]))
    for leg_ids in by_location.values():
        if len(leg_ids) < 2:
            continue
        anchor = leg_ids[0]
        for leg_id in leg_ids[1:]:
            union(anchor, leg_id)

    legs_by_root: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    for leg_id, leg in by_id.items():
        legs_by_root[find(leg_id)].append(leg)

    issues: list[dict[str, Any]] = []
    for connected_legs in legs_by_root.values():
        domains: dict[str, list[str]] = defaultdict(list)
        for leg in connected_legs:
            scope = _quantity_network_scope(leg)
            domain = scope.get("domain") if scope.get("valid") else None
            if domain is not None:
                domains[str(domain)].append(str(leg["id"]))
        if len(domains) <= 1:
            continue
        issues.append(
            {
                "code": "custody_network_scope_laundering",
                "message": (
                    "unknown custody legs cannot connect incompatible known "
                    "Bitcoin network domains"
                ),
                "network_domains": sorted(domains),
                "leg_ids_by_domain": {
                    domain: sorted(leg_ids)
                    for domain, leg_ids in sorted(domains.items())
                },
                "component_ids": sorted(
                    {
                        str(leg["component_id"])
                        for leg in connected_legs
                        if leg.get("component_id") not in (None, "")
                    }
                ),
            }
        )
    return issues


def _validate_allocations(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
    *,
    conservation_mode: str,
) -> dict[str, Any]:
    by_id = {str(leg["id"]): leg for leg in legs}
    positive_sources = [
        leg for leg in legs if leg["role"] == "source" and int(leg["amount_msat"]) > 0
    ]
    positive_sinks = [
        leg for leg in legs if leg["role"] in SINK_ROLES and int(leg["amount_msat"]) > 0
    ]
    issues: list[dict[str, Any]] = []
    coverage_source: dict[str, int] = defaultdict(int)
    coverage_sink: dict[str, int] = defaultdict(int)
    fee_source_ids: set[str] = set()
    material_source_ids: set[str] = set()
    edges: list[dict[str, Any]] = []
    for allocation in allocations:
        source = by_id.get(str(allocation["source_leg_id"]))
        sink = by_id.get(str(allocation["sink_leg_id"]))
        if source is None or sink is None or source["role"] != "source" or sink["role"] == "source":
            issues.append(
                {
                    "code": "allocation_leg_invalid",
                    "allocation_id": allocation.get("id"),
                }
            )
            continue
        source_amount = int(allocation["source_amount_msat"])
        sink_amount = int(allocation["sink_amount_msat"])
        coverage_source[str(source["id"])] += source_amount
        coverage_sink[str(sink["id"])] += sink_amount
        if source_amount > 0:
            if sink["role"] == "fee":
                fee_source_ids.add(str(source["id"]))
            else:
                material_source_ids.add(str(source["id"]))
        same_quantity_unit = (
            source["exposure"] == sink["exposure"]
            and source["conservation_unit"] == sink["conservation_unit"]
        )
        if sink["role"] == "fee":
            # A fee edge is projected as loss from its allocation source.  Do
            # not accept metadata that says a different asset/wallet paid it;
            # that would activate successfully and then charge the wrong lot.
            # Destination-paid or third-asset fees remain representable as a
            # separate source->fee edge from that wallet/asset, provided that
            # source also carries a material journal row below.
            if normalize_asset_code(source["asset"]) != normalize_asset_code(
                sink["asset"]
            ):
                issues.append(
                    {
                        "code": "fee_source_asset_mismatch",
                        "allocation_id": allocation.get("id"),
                        "source_leg_id": source["id"],
                        "fee_leg_id": sink["id"],
                        "source_asset": source["asset"],
                        "fee_asset": sink["asset"],
                    }
                )
            if (
                sink.get("wallet_id") is not None
                and source.get("wallet_id") is not None
                and str(sink["wallet_id"]) != str(source["wallet_id"])
            ):
                issues.append(
                    {
                        "code": "fee_source_wallet_mismatch",
                        "allocation_id": allocation.get("id"),
                        "source_leg_id": source["id"],
                        "fee_leg_id": sink["id"],
                        "source_wallet_id": source["wallet_id"],
                        "fee_wallet_id": sink["wallet_id"],
                    }
                )
            source_scope = _quantity_network_scope(source)
            fee_scope = _quantity_network_scope(sink)
            if (
                not source_scope["valid"]
                or not fee_scope["valid"]
                or source_scope["rail"] != fee_scope["rail"]
                or (
                    source_scope.get("domain") is not None
                    and fee_scope.get("domain") is not None
                    and source_scope["domain"] != fee_scope["domain"]
                )
            ):
                issues.append(
                    {
                        "code": "fee_source_scope_mismatch",
                        "allocation_id": allocation.get("id"),
                        "source_leg_id": source["id"],
                        "fee_leg_id": sink["id"],
                        "source_scope": source_scope,
                        "fee_scope": fee_scope,
                    }
                )
            if conservation_mode == "conversion":
                if source_amount != sink_amount:
                    issues.append(
                        {
                            "code": "conversion_fee_quantity_mismatch",
                            "allocation_id": allocation.get("id"),
                            "source_leg_id": source["id"],
                            "fee_leg_id": sink["id"],
                            "source_amount_msat": source_amount,
                            "fee_amount_msat": sink_amount,
                        }
                    )
                source_valuation = source.get("valuation_amount")
                fee_valuation = sink.get("valuation_amount")
                source_leg_amount = int(source.get("amount_msat") or 0)
                if (
                    source_valuation is not None
                    and fee_valuation is not None
                    and source.get("valuation_unit") == sink.get("valuation_unit")
                    and source_leg_amount > 0
                    and int(fee_valuation) * source_leg_amount
                    != int(source_valuation) * source_amount
                ):
                    issues.append(
                        {
                            "code": "conversion_fee_valuation_mismatch",
                            "allocation_id": allocation.get("id"),
                            "source_leg_id": source["id"],
                            "fee_leg_id": sink["id"],
                        }
                    )
        if conservation_mode == "quantity" and (
            not same_quantity_unit or source_amount != sink_amount
        ):
            issues.append(
                {
                    "code": "allocation_quantity_mismatch",
                    "allocation_id": allocation.get("id"),
                    "source_leg_id": source["id"],
                    "sink_leg_id": sink["id"],
                }
            )
        edges.append(
            {
                "id": allocation.get("id"),
                "source_leg_id": source["id"],
                "sink_leg_id": sink["id"],
                "source_amount_msat": source_amount,
                "sink_amount_msat": sink_amount,
            }
        )

    if allocations:
        for leg in positive_sources:
            covered = coverage_source.get(str(leg["id"]), 0)
            if covered != int(leg["amount_msat"]):
                issues.append(
                    {
                        "code": "allocation_source_coverage_mismatch",
                        "leg_id": leg["id"],
                        "expected_msat": int(leg["amount_msat"]),
                        "covered_msat": covered,
                    }
                )
        orphan_fee_sources = sorted(fee_source_ids - material_source_ids)
        if orphan_fee_sources:
            issues.append(
                {
                    "code": "custody_component_fee_orphaned",
                    "message": (
                        "a fee source needs an owned transfer or external "
                        "disposal row to carry it into journals"
                    ),
                    "source_leg_ids": orphan_fee_sources,
                }
            )
        for leg in positive_sinks:
            covered = coverage_sink.get(str(leg["id"]), 0)
            if covered != int(leg["amount_msat"]):
                issues.append(
                    {
                        "code": "allocation_sink_coverage_mismatch",
                        "leg_id": leg["id"],
                        "expected_msat": int(leg["amount_msat"]),
                        "covered_msat": covered,
                    }
                )
    else:
        owned_sinks = [leg for leg in positive_sinks if leg["role"] in {"destination", "retained"}]
        attributed_sinks = [
            leg for leg in positive_sinks if leg["role"] in {"fee", "external", "unresolved"}
        ]
        unambiguous = (
            len(positive_sources) == 1 and len(positive_sinks) == 1
            if conservation_mode == "conversion"
            else len(positive_sources) <= 1
            or (len(owned_sinks) == 1 and not attributed_sinks)
        )
        if positive_sources and positive_sinks and not unambiguous:
            issues.append(
                {
                    "code": "allocation_required",
                    "message": "N:M custody flow requires explicit source-to-sink allocation",
                    "source_leg_ids": [leg["id"] for leg in positive_sources],
                    "sink_leg_ids": [leg["id"] for leg in positive_sinks],
                }
            )
        if len(positive_sources) == 1:
            source = positive_sources[0]
            for sink in positive_sinks:
                if sink["role"] != "fee":
                    continue
                if normalize_asset_code(source["asset"]) != normalize_asset_code(
                    sink["asset"]
                ):
                    issues.append(
                        {
                            "code": "fee_source_asset_mismatch",
                            "source_leg_id": source["id"],
                            "fee_leg_id": sink["id"],
                            "source_asset": source["asset"],
                            "fee_asset": sink["asset"],
                        }
                    )
                if (
                    sink.get("wallet_id") is not None
                    and source.get("wallet_id") is not None
                    and str(sink["wallet_id"]) != str(source["wallet_id"])
                ):
                    issues.append(
                        {
                            "code": "fee_source_wallet_mismatch",
                            "source_leg_id": source["id"],
                            "fee_leg_id": sink["id"],
                            "source_wallet_id": source["wallet_id"],
                            "fee_wallet_id": sink["wallet_id"],
                        }
                    )
                source_scope = _quantity_network_scope(source)
                fee_scope = _quantity_network_scope(sink)
                if (
                    not source_scope["valid"]
                    or not fee_scope["valid"]
                    or source_scope["rail"] != fee_scope["rail"]
                    or (
                        source_scope.get("domain") is not None
                        and fee_scope.get("domain") is not None
                        and source_scope["domain"] != fee_scope["domain"]
                    )
                ):
                    issues.append(
                        {
                            "code": "fee_source_scope_mismatch",
                            "source_leg_id": source["id"],
                            "fee_leg_id": sink["id"],
                            "source_scope": source_scope,
                            "fee_scope": fee_scope,
                        }
                    )
    issues.extend(
        _quantity_allocation_scope_issues(
            legs,
            allocations,
            conservation_mode=conservation_mode,
        )
    )
    return {"valid": not issues, "issues": issues, "edges": edges}


def _balance_rows(
    legs: Sequence[Mapping[str, Any]],
    *,
    key_fields: Sequence[str],
    amount_field: str,
    skip_missing: bool = False,
) -> list[dict[str, Any]]:
    buckets: dict[tuple[str, ...], dict[str, int]] = defaultdict(
        lambda: {"source": 0, "destination": 0, "fee": 0, "external": 0,
                 "retained": 0, "unresolved": 0}
    )
    for leg in legs:
        values = tuple(leg.get(field) for field in key_fields)
        if skip_missing and any(value is None for value in values):
            continue
        key = tuple("" if value is None else str(value) for value in values)
        raw_amount = leg.get(amount_field)
        if raw_amount is None:
            continue
        buckets[key][str(leg["role"])] += int(raw_amount)
    rows: list[dict[str, Any]] = []
    for key in sorted(buckets):
        totals = buckets[key]
        sinks = sum(totals[role] for role in SINK_ROLES)
        row = {field: value for field, value in zip(key_fields, key)}
        row.update(
            {
                "source_msat" if amount_field == "amount_msat" else "source_amount": totals["source"],
                "destination_msat" if amount_field == "amount_msat" else "destination_amount": totals["destination"],
                "fee_msat" if amount_field == "amount_msat" else "fee_amount": totals["fee"],
                "external_msat" if amount_field == "amount_msat" else "external_amount": totals["external"],
                "retained_msat" if amount_field == "amount_msat" else "retained_amount": totals["retained"],
                "unresolved_msat" if amount_field == "amount_msat" else "unresolved_amount": totals["unresolved"],
                "sink_msat" if amount_field == "amount_msat" else "sink_amount": sinks,
                "residual_msat" if amount_field == "amount_msat" else "residual_amount": totals["source"] - sinks,
            }
        )
        rows.append(row)
    return rows


def _location_continuity_issues(
    legs: Sequence[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Validate inferred multi-hop continuity inside one component.

    Independent source wallets may begin a component with pre-existing lots.
    Once the same component also credits a wallet/exposure/unit, however, a
    transactionless source from that location is an intermediate hop and must
    not run ahead of those credits. Anchored later sources are checked too once
    an earlier component credit exists. This prevents unrelated pre-funded lots
    from making a reversed missing-wallet route look valid.
    """

    groups: dict[tuple[str, str, str], dict[str, list[Mapping[str, Any]]]] = defaultdict(
        lambda: {"sources": [], "sinks": []}
    )
    for leg in legs:
        if int(leg.get("amount_msat") or 0) <= 0 or not leg.get("wallet_id"):
            continue
        key = (
            str(leg["wallet_id"]),
            str(leg.get("exposure") or ""),
            str(leg.get("conservation_unit") or ""),
        )
        if leg.get("role") == "source":
            groups[key]["sources"].append(leg)
        elif leg.get("role") in {"destination", "retained"}:
            groups[key]["sinks"].append(leg)

    issues: list[dict[str, Any]] = []
    for (wallet_id, exposure, unit), group in sorted(groups.items()):
        sources = group["sources"]
        sinks = group["sinks"]
        if not sources or not sinks:
            continue
        parsed_sinks = [
            (parse_iso_datetime_or_none(leg.get("occurred_at")), leg)
            for leg in sinks
        ]
        parsed_sources = [
            (parse_iso_datetime_or_none(leg.get("occurred_at")), leg)
            for leg in sources
        ]
        if any(when is None for when, _leg in (*parsed_sinks, *parsed_sources)):
            # The timestamp issue is reported separately; avoid inventing an
            # ordering from malformed evidence.
            continue

        intermediate_sources = []
        for source_when, source in parsed_sources:
            assert source_when is not None
            has_prior_credit = any(
                sink_when is not None
                and sink_when != source_when
                and sink_when
                <= source_when + CUSTODY_CHRONOLOGY_SKEW_TOLERANCE
                for sink_when, _sink in parsed_sinks
            )
            if source.get("transaction_id") is None or has_prior_credit:
                intermediate_sources.append((source_when, source))
        if not intermediate_sources:
            continue

        events = [
            (when - CUSTODY_CHRONOLOGY_SKEW_TOLERANCE, 0, leg)
            for when, leg in parsed_sinks
            if when is not None
        ] + [
            (when, 1, leg) for when, leg in intermediate_sources
        ]
        events.sort(key=lambda item: (item[0], item[1], int(item[2].get("ordinal") or 0)))
        available = 0
        for _when, kind, leg in events:
            amount = int(leg.get("amount_msat") or 0)
            if kind == 0:
                available += amount
                continue
            if amount > available:
                issues.append(
                    {
                        "code": "custody_location_continuity_mismatch",
                        "message": (
                            "an intermediate custody location spends before or "
                            "beyond the value this component credited to it"
                        ),
                        "wallet_id": wallet_id,
                        "exposure": exposure,
                        "conservation_unit": unit,
                        "source_leg_id": leg["id"],
                        "available_msat": available,
                        "source_msat": amount,
                    }
                )
                break
            available -= amount
    return issues


def validate_conservation(
    legs: Sequence[Mapping[str, Any]],
    *,
    allocations: Sequence[Mapping[str, Any]] | None = None,
    conservation_mode: str = "quantity",
    conversion_policy: str | None = None,
    conversion_reviewed: bool = False,
) -> dict[str, Any]:
    """Return an audit-friendly, deterministic activation validation report.

    Quantity mode conserves each explicit ``(exposure, conservation_unit)``.
    BTC, LBTC and Lightning BTC therefore use exposure ``bitcoin`` and unit
    ``msat`` while retaining their original rail/asset facts.  Conversion mode
    never assumes unlike quantities are equal: it requires a reviewed policy
    and exact per-leg valuations which themselves balance by valuation unit.
    """

    mode = _normalize_mode(conservation_mode)
    materialized = [dict(leg) for leg in legs]
    for index, leg in enumerate(materialized):
        leg.setdefault("id", f"leg:{index}")
        leg.setdefault("ordinal", index)
    materialized_allocations = [dict(allocation) for allocation in (allocations or [])]
    issues: list[dict[str, Any]] = []
    source_total = sum(int(leg.get("amount_msat") or 0) for leg in materialized if leg.get("role") == "source")
    owned_destination_total = sum(
        int(leg.get("amount_msat") or 0)
        for leg in materialized
        if leg.get("role") in {"destination", "retained"}
    )
    unresolved_total = sum(
        int(leg.get("amount_msat") or 0)
        for leg in materialized
        if leg.get("role") == "unresolved"
    )

    if not materialized:
        issues.append({"code": "no_legs", "message": "component has no legs"})
    if source_total <= 0:
        issues.append({"code": "missing_source", "message": "component has no positive source"})
    if owned_destination_total <= 0:
        issues.append(
            {
                "code": "missing_owned_destination",
                "message": "component has no positive destination or retained-custody leg",
            }
        )
    if unresolved_total:
        issues.append(
            {
                "code": "unresolved_value",
                "message": "component still contains unresolved value",
                "amount_msat": unresolved_total,
            }
        )
    value_only_losses = [
        str(leg.get("id") or int(leg.get("ordinal", index)))
        for index, leg in enumerate(materialized)
        if leg.get("role") in {"fee", "external"}
        and int(leg.get("amount_msat") or 0) == 0
        and int(leg.get("valuation_amount") or 0) > 0
    ]
    if value_only_losses:
        issues.append(
            {
                "code": "custody_component_value_only_loss_unsupported",
                "message": (
                    "a fiat-only fee or external loss needs a positive quantity "
                    "leg before it can be projected into journals"
                ),
                "leg_ids": value_only_losses,
            }
        )
    missing_occurrence = [
        int(leg.get("ordinal", index))
        for index, leg in enumerate(materialized)
        if leg.get("transaction_id") is None
        and leg.get("role") in {"source", "destination", "retained"}
        and int(leg.get("amount_msat") or 0) > 0
        and not leg.get("occurred_at")
    ]
    if missing_occurrence:
        issues.append(
            {
                "code": "leg_occurred_at_missing",
                "message": "transaction-less custody legs require occurred_at",
                "ordinals": missing_occurrence,
            }
        )
    invalid_occurrence = [
        int(leg.get("ordinal", index))
        for index, leg in enumerate(materialized)
        if leg.get("occurred_at") not in (None, "")
        and parse_iso_datetime_or_none(leg.get("occurred_at")) is None
    ]
    if invalid_occurrence:
        issues.append(
            {
                "code": "leg_occurred_at_invalid",
                "message": "custody leg timestamps must be valid RFC3339 values",
                "ordinals": invalid_occurrence,
            }
        )
    issues.extend(
        _allocation_chronology_issues(
            materialized,
            materialized_allocations,
            conservation_mode=mode,
        )
    )
    issues.extend(_location_continuity_issues(materialized))

    by_asset = _balance_rows(materialized, key_fields=("asset",), amount_field="amount_msat")
    by_unit = _balance_rows(
        materialized,
        key_fields=("exposure", "conservation_unit"),
        amount_field="amount_msat",
    )
    by_valuation = _balance_rows(
        materialized,
        key_fields=("valuation_unit",),
        amount_field="valuation_amount",
        skip_missing=True,
    )

    if mode == "quantity":
        unbalanced = [row for row in by_unit if row["residual_msat"] != 0]
        if unbalanced:
            issues.append(
                {
                    "code": "unbalanced_quantity",
                    "message": "source and sink quantities do not conserve",
                    "groups": unbalanced,
                }
            )

    else:
        positive_sources = [
            leg
            for leg in materialized
            if leg.get("role") == "source" and int(leg.get("amount_msat") or 0) > 0
        ]
        positive_owned_sinks = [
            leg
            for leg in materialized
            if leg.get("role") in {"destination", "retained"}
            and int(leg.get("amount_msat") or 0) > 0
        ]
        if len(positive_sources) != 1 or len(positive_owned_sinks) != 1:
            issues.append(
                {
                    "code": "conversion_topology_unsupported",
                    "message": (
                        "reviewed conversions currently require exactly one "
                        "quantity source and one owned destination; split the "
                        "conversion into auditable components"
                    ),
                    "source_leg_ids": [leg["id"] for leg in positive_sources],
                    "destination_leg_ids": [
                        leg["id"] for leg in positive_owned_sinks
                    ],
                }
            )
        if not conversion_reviewed:
            issues.append(
                {
                    "code": "conversion_not_reviewed",
                    "message": "conversion requires explicit review",
                }
            )
        if not isinstance(conversion_policy, str) or not conversion_policy.strip():
            issues.append(
                {
                    "code": "conversion_policy_missing",
                    "message": "conversion requires an explicit policy",
                }
            )
        missing_valuations = [
            int(leg.get("ordinal", index))
            for index, leg in enumerate(materialized)
            if (int(leg.get("amount_msat") or 0) > 0 or int(leg.get("valuation_amount") or 0) > 0)
            and (leg.get("valuation_unit") is None or leg.get("valuation_amount") is None)
        ]
        if missing_valuations:
            issues.append(
                {
                    "code": "conversion_valuation_missing",
                    "message": "every material conversion leg requires an exact valuation",
                    "ordinals": missing_valuations,
                }
            )
        if not by_valuation:
            issues.append(
                {
                    "code": "conversion_valuation_missing",
                    "message": "conversion has no exact valuation basis",
                }
            )
        unbalanced_values = [row for row in by_valuation if row["residual_amount"] != 0]
        if unbalanced_values:
            issues.append(
                {
                    "code": "unbalanced_conversion_valuation",
                    "message": "conversion valuations do not conserve",
                    "groups": unbalanced_values,
                }
            )

    allocation_validation = _validate_allocations(
        materialized,
        materialized_allocations,
        conservation_mode=mode,
    )
    issues.extend(allocation_validation["issues"])
    issues.extend(
        _quantity_scope_connectivity_issues(
            materialized,
            materialized_allocations,
            conservation_mode=mode,
        )
    )

    return {
        "conservation_mode": mode,
        "activatable": not issues,
        "issues": issues,
        "unresolved_msat": unresolved_total,
        "source_msat": source_total,
        "owned_destination_msat": owned_destination_total,
        "by_asset": by_asset,
        "by_conservation_unit": by_unit,
        "by_valuation_unit": by_valuation,
        "allocations": allocation_validation,
    }


def _scope(conn: sqlite3.Connection, workspace_id: str, profile_id: str) -> None:
    row = conn.execute(
        "SELECT workspace_id FROM profiles WHERE id = ?", (profile_id,)
    ).fetchone()
    if not row:
        raise _error(
            "profile was not found",
            "not_found",
            details={"profile_id": profile_id},
        )
    if str(row["workspace_id"]) != workspace_id:
        raise _error(
            "profile does not belong to the workspace",
            "custody_component_scope_mismatch",
            details={"workspace_id": workspace_id, "profile_id": profile_id},
        )


def _validate_leg_anchors(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    legs: Sequence[Mapping[str, Any]],
) -> None:
    for leg in legs:
        transaction_id = leg.get("transaction_id")
        if transaction_id is not None:
            row = conn.execute(
                "SELECT workspace_id, profile_id, occurred_at FROM transactions WHERE id = ?",
                (transaction_id,),
            ).fetchone()
            if not row:
                raise _error(
                    "custody leg transaction was not found",
                    "not_found",
                    details={"transaction_id": transaction_id, "ordinal": leg["ordinal"]},
                )
            if row["workspace_id"] != workspace_id or row["profile_id"] != profile_id:
                raise _error(
                    "custody leg transaction is outside the component scope",
                    "custody_component_scope_mismatch",
                    details={"transaction_id": transaction_id, "ordinal": leg["ordinal"]},
                )
            canonical_occurred_at = parse_timestamp(row["occurred_at"])
            if (
                leg.get("occurred_at") is not None
                and parse_timestamp(leg["occurred_at"]) != canonical_occurred_at
            ):
                raise _error(
                    "anchored custody legs must use the transaction occurrence time",
                    "custody_component_anchor_time_mismatch",
                    details={
                        "transaction_id": transaction_id,
                        "ordinal": leg["ordinal"],
                        "transaction_occurred_at": canonical_occurred_at,
                        "leg_occurred_at": leg["occurred_at"],
                    },
                )
            leg["occurred_at"] = canonical_occurred_at
        wallet_id = leg.get("wallet_id")
        if wallet_id is not None:
            row = conn.execute(
                "SELECT workspace_id, profile_id FROM wallets WHERE id = ?", (wallet_id,)
            ).fetchone()
            if not row:
                raise _error(
                    "custody leg wallet was not found",
                    "not_found",
                    details={"wallet_id": wallet_id, "ordinal": leg["ordinal"]},
                )
            if row["workspace_id"] != workspace_id or row["profile_id"] != profile_id:
                raise _error(
                    "custody leg wallet is outside the component scope",
                    "custody_component_scope_mismatch",
                    details={"wallet_id": wallet_id, "ordinal": leg["ordinal"]},
                )


def _invalidate_journals(conn: sqlite3.Connection, profile_id: str) -> None:
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = NULL,
            last_processed_tx_count = 0,
            journal_input_version = journal_input_version + 1,
            ownership_review_counts_json = NULL
        WHERE id = ?
        """,
        (profile_id,),
    )


def _insert_legs(
    conn: sqlite3.Connection,
    *,
    component_id: str,
    workspace_id: str,
    profile_id: str,
    legs: Sequence[Mapping[str, Any]],
    created_at: str,
) -> None:
    conn.executemany(
        """
        INSERT INTO custody_component_legs(
            id, component_id, workspace_id, profile_id, ordinal, role, rail,
            chain, network, asset, exposure, conservation_unit, amount_msat,
            valuation_unit, valuation_amount, occurred_at, transaction_id,
            anchor_transaction_id, wallet_id, location_ref, notes, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                leg["id"], component_id, workspace_id, profile_id, leg["ordinal"],
                leg["role"], leg["rail"], leg["chain"], leg["network"],
                leg["asset"], leg["exposure"], leg["conservation_unit"],
                leg["amount_msat"], leg["valuation_unit"], leg["valuation_amount"],
                leg["occurred_at"], leg["transaction_id"],
                leg["anchor_transaction_id"], leg["wallet_id"],
                leg["location_ref"], leg["notes"], created_at,
            )
            for leg in legs
        ],
    )


def _insert_allocations(
    conn: sqlite3.Connection,
    *,
    component_id: str,
    workspace_id: str,
    profile_id: str,
    allocations: Sequence[Mapping[str, Any]],
    created_at: str,
) -> None:
    conn.executemany(
        """
        INSERT INTO custody_component_allocations(
            id, component_id, workspace_id, profile_id, ordinal,
            source_leg_id, sink_leg_id, source_amount_msat,
            sink_amount_msat, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                allocation["id"], component_id, workspace_id, profile_id,
                allocation["ordinal"], allocation["source_leg_id"],
                allocation["sink_leg_id"], allocation["source_amount_msat"],
                allocation["sink_amount_msat"], created_at,
            )
            for allocation in allocations
        ],
    )


def _row(conn: sqlite3.Connection, component_id: str) -> sqlite3.Row:
    row = conn.execute(
        "SELECT * FROM custody_components WHERE id = ?", (component_id,)
    ).fetchone()
    if not row:
        raise _error(
            "custody component was not found",
            "not_found",
            details={"component_id": component_id},
        )
    return row


def _leg_dict(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": row["id"],
        "component_id": row["component_id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "ordinal": int(row["ordinal"]),
        "role": row["role"],
        "rail": row["rail"],
        "chain": row["chain"],
        "network": row["network"],
        "asset": row["asset"],
        "exposure": row["exposure"],
        "conservation_unit": row["conservation_unit"],
        "amount_msat": int(row["amount_msat"]),
        "valuation_unit": row["valuation_unit"],
        "valuation_amount": (
            None if row["valuation_amount"] is None else int(row["valuation_amount"])
        ),
        "occurred_at": row["occurred_at"],
        "transaction_id": row["transaction_id"],
        "anchor_transaction_id": row["anchor_transaction_id"],
        "wallet_id": row["wallet_id"],
        "location_ref": row["location_ref"],
        "notes": row["notes"],
        "created_at": row["created_at"],
    }


def _allocation_dict(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": row["id"],
        "component_id": row["component_id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "ordinal": int(row["ordinal"]),
        "source_leg_id": row["source_leg_id"],
        "sink_leg_id": row["sink_leg_id"],
        "source_amount_msat": int(row["source_amount_msat"]),
        "sink_amount_msat": int(row["sink_amount_msat"]),
        "created_at": row["created_at"],
    }


def _active_membership_conflicts(
    conn: sqlite3.Connection,
    *,
    component_id: str,
    profile_id: str,
) -> list[dict[str, str]]:
    return [
        {"transaction_id": row["transaction_id"], "component_id": row["component_id"]}
        for row in conn.execute(
            """
            SELECT DISTINCT mine.transaction_id, other.component_id
            FROM custody_component_legs mine
            JOIN custody_component_legs other
              ON other.profile_id = mine.profile_id
             AND other.transaction_id = mine.transaction_id
             AND other.component_id != mine.component_id
            JOIN custody_components other_component
              ON other_component.id = other.component_id
             AND other_component.state = 'active'
            WHERE mine.component_id = ?
              AND mine.profile_id = ?
              AND mine.transaction_id IS NOT NULL
            ORDER BY mine.transaction_id, other.component_id
            """,
            (component_id, profile_id),
        ).fetchall()
    ]


def _replicated_lineage_issues(
    conn: sqlite3.Connection,
    row: sqlite3.Row,
) -> list[dict[str, Any]]:
    """Return reviewable lineage conflicts without hiding authored rows."""

    issues: list[dict[str, Any]] = []
    lifecycle_conflicts = conn.execute(
        """
        SELECT id, field
        FROM sync_conflicts
        WHERE profile_id = ?
          AND entity_table = 'custody_components'
          AND entity_key = ?
          AND status = 'open'
          AND field IN (
              'state', 'activated_at', 'superseded_by_component_id',
              'superseded_at', '__exists__'
          )
        ORDER BY field, id
        """,
        (
            row["profile_id"],
            json.dumps([row["id"]], separators=(",", ":")),
        ),
    ).fetchall()
    if lifecycle_conflicts:
        issues.append(
            {
                "code": "component_lifecycle_conflict",
                "message": (
                    "an unresolved replicated lifecycle conflict keeps this "
                    "revision ineffective"
                ),
                "conflicts": [
                    {"conflict_id": conflict["id"], "field": conflict["field"]}
                    for conflict in lifecycle_conflicts
                ],
            }
        )
    if row["state"] == "active":
        competing = [
            {"component_id": other["id"], "revision": int(other["revision"])}
            for other in conn.execute(
                """
                SELECT id, revision FROM custody_components
                WHERE profile_id = ? AND lineage_id = ?
                  AND state = 'active' AND id != ?
                ORDER BY revision, id
                """,
                (row["profile_id"], row["lineage_id"], row["id"]),
            ).fetchall()
        ]
        if competing:
            issues.append(
                {
                    "code": "active_lineage_conflict",
                    "message": "multiple authored active revisions exist in this lineage",
                    "conflicts": competing,
                }
            )

    links = (
        ("supersedes_component_id", -1),
        ("superseded_by_component_id", 1),
    )
    for field, expected_order in links:
        target_id = row[field]
        if target_id is None:
            continue
        target = conn.execute(
            "SELECT id, profile_id, lineage_id, revision FROM custody_components WHERE id = ?",
            (target_id,),
        ).fetchone()
        if not target:
            issues.append(
                {
                    "code": "revision_link_missing",
                    "field": field,
                    "target_component_id": target_id,
                }
            )
            continue
        wrong_scope = (
            target["profile_id"] != row["profile_id"]
            or target["lineage_id"] != row["lineage_id"]
        )
        revision_delta = int(target["revision"]) - int(row["revision"])
        wrong_order = revision_delta * expected_order <= 0
        if wrong_scope or wrong_order:
            issues.append(
                {
                    "code": "revision_link_invalid",
                    "field": field,
                    "target_component_id": target_id,
                    "target_revision": int(target["revision"]),
                }
            )
    return issues


def _canonical_chain_or_none(value: Any) -> str | None:
    if value in (None, ""):
        return None
    try:
        return normalize_chain(value)
    except ValueError:
        return None


def _canonical_network_or_none(chain: str | None, value: Any) -> str | None:
    if chain is None or value in (None, ""):
        return None
    try:
        return normalize_network(chain, value)
    except ValueError:
        return None


def _record_value(row: Any, key: str) -> Any:
    if isinstance(row, Mapping):
        return row.get(key)
    try:
        return row[key] if key in row.keys() else None
    except (AttributeError, KeyError, TypeError):
        return getattr(row, key, None)


def _anchor_expected_rail_scope(
    row: Mapping[str, Any],
) -> tuple[str | None, str | None, str | None]:
    """Return the rail/chain/network established by imported evidence."""

    wallet_kind = normalize_wallet_kind_alias(_record_value(row, "wallet_kind"))
    asset = normalize_asset_code(_record_value(row, "asset"))
    config_raw = _record_value(row, "config_json")
    try:
        config = (
            json.loads(config_raw)
            if isinstance(config_raw, str) and config_raw
            else config_raw
        )
    except (TypeError, ValueError, json.JSONDecodeError):
        config = {}
    if not isinstance(config, Mapping):
        config = {}

    exact_scope = onchain_transfer_scope(row)
    expected_rail: str | None = None
    expected_chain: str | None = None
    expected_network: str | None = None
    if exact_scope is not None:
        expected_chain, expected_network = exact_scope[0], exact_scope[1]
        expected_rail = expected_chain
    else:
        configured_chain = _canonical_chain_or_none(config.get("chain"))
        if wallet_kind in LIGHTNING_INFERENCE_WALLET_KINDS:
            expected_rail = "lightning"
            expected_chain = configured_chain or "bitcoin"
        elif (
            configured_chain == "liquid"
            or wallet_kind in {"elements", "liquid"}
            or "liquid" in wallet_kind
            or asset == "LBTC"
        ):
            expected_rail = "liquid"
            expected_chain = "liquid"
        elif wallet_kind in {
            "address",
            "descriptor",
            "samourai",
            "silent-payment",
            "wasabi",
            "xpub",
        } or configured_chain == "bitcoin":
            expected_rail = "bitcoin"
            expected_chain = "bitcoin"
        elif configured_chain is not None:
            expected_rail = configured_chain
            expected_chain = configured_chain
        expected_network = _canonical_network_or_none(
            expected_chain, config.get("network")
        )
    return expected_rail, expected_chain, expected_network


def _anchor_rail_scope_issues(
    leg: Mapping[str, Any], row: Mapping[str, Any]
) -> list[dict[str, Any]]:
    """Reject authored rail facts that contradict their imported anchor.

    A component may omit chain/network when an old importer did not retain
    them, but it must never claim a different Bitcoin layer or network from
    evidence that is present. The check is descriptive Bitcoin plumbing; no
    tax profile or country participates.
    """

    issue_base = {
        "leg_id": leg.get("id"),
        "transaction_id": str(_record_value(row, "id") or ""),
    }
    expected_rail, expected_chain, expected_network = (
        _anchor_expected_rail_scope(row)
    )

    issues: list[dict[str, Any]] = []
    authored_rail = _canonical_rail(leg.get("rail"))
    if expected_rail is not None and authored_rail != expected_rail:
        issues.append(
            {
                "code": "anchor_rail_mismatch",
                **issue_base,
                "leg_rail": authored_rail,
                "transaction_rail": expected_rail,
            }
        )

    authored_chain_raw = leg.get("chain")
    if expected_chain is not None and authored_chain_raw not in (None, ""):
        authored_chain = _canonical_chain_or_none(authored_chain_raw)
        if authored_chain != expected_chain:
            issues.append(
                {
                    "code": "anchor_chain_mismatch",
                    **issue_base,
                    "leg_chain": authored_chain_raw,
                    "transaction_chain": expected_chain,
                }
            )

    authored_network_raw = leg.get("network")
    if expected_network is not None and authored_network_raw not in (None, ""):
        authored_network = _canonical_network_or_none(
            expected_chain, authored_network_raw
        )
        if authored_network != expected_network:
            issues.append(
                {
                    "code": "anchor_network_mismatch",
                    **issue_base,
                    "leg_network": authored_network_raw,
                    "transaction_network": expected_network,
                }
            )
    return issues


def _scoped_leg_copies(
    legs: Sequence[Mapping[str, Any]],
    transaction_rows: Mapping[str, Mapping[str, Any]],
    wallet_rows: Mapping[str, Mapping[str, Any]],
) -> list[dict[str, Any]]:
    """Enrich omitted physical scope from local anchor/wallet evidence."""

    scoped_legs: list[dict[str, Any]] = []
    for leg in legs:
        scoped_leg = dict(leg)
        transaction_id = leg.get("transaction_id")
        evidence_row: Mapping[str, Any] | None = None
        if transaction_id not in (None, ""):
            evidence_row = transaction_rows.get(str(transaction_id))
        if evidence_row is None and leg.get("wallet_id") not in (None, ""):
            wallet_row = wallet_rows.get(str(leg["wallet_id"]))
            if wallet_row is not None:
                evidence_row = {
                    "id": f"wallet:{wallet_row['id']}",
                    "wallet_kind": wallet_row["wallet_kind"],
                    "config_json": wallet_row["config_json"],
                    "asset": leg.get("asset"),
                    "raw_json": "{}",
                    "external_id": None,
                }
        if evidence_row is not None:
            expected_rail, expected_chain, expected_network = (
                _anchor_expected_rail_scope(evidence_row)
            )
            if scoped_leg.get("rail") in (None, "") and expected_rail is not None:
                scoped_leg["rail"] = expected_rail
            if scoped_leg.get("chain") in (None, "") and expected_chain is not None:
                scoped_leg["chain"] = expected_chain
            if (
                scoped_leg.get("network") in (None, "")
                and expected_network is not None
            ):
                scoped_leg["network"] = expected_network
            canonical_occurred_at = _record_value(evidence_row, "occurred_at")
            if (
                transaction_id not in (None, "")
                and canonical_occurred_at not in (None, "")
            ):
                scoped_leg["occurred_at"] = canonical_occurred_at
        scoped_legs.append(scoped_leg)
    return scoped_legs


def _load_scope_evidence(
    conn: sqlite3.Connection,
    legs: Sequence[Mapping[str, Any]],
) -> tuple[dict[str, sqlite3.Row], dict[str, sqlite3.Row]]:
    """Load only the safe local fields needed to establish leg scope."""

    transaction_rows: dict[str, sqlite3.Row] = {}
    for transaction_id in sorted(
        {
            str(leg["transaction_id"])
            for leg in legs
            if leg.get("transaction_id") not in (None, "")
        }
    ):
        row = conn.execute(
            """
            SELECT t.id, t.wallet_id, t.direction, t.asset, t.amount, t.fee,
                   t.amount_includes_fee, t.occurred_at, t.excluded,
                   t.external_id, t.raw_json,
                   w.kind AS wallet_kind, w.config_json AS config_json
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
            WHERE t.id = ?
            """,
            (transaction_id,),
        ).fetchone()
        if row is not None:
            transaction_rows[transaction_id] = row
    wallet_rows: dict[str, sqlite3.Row] = {}
    for wallet_id in sorted(
        {
            str(leg["wallet_id"])
            for leg in legs
            if leg.get("wallet_id") not in (None, "")
        }
    ):
        row = conn.execute(
            "SELECT id, kind AS wallet_kind, config_json FROM wallets WHERE id = ?",
            (wallet_id,),
        ).fetchone()
        if row is not None:
            wallet_rows[wallet_id] = row
    return transaction_rows, wallet_rows


def _other_active_quantity_content(
    conn: sqlite3.Connection,
    legs: Sequence[Mapping[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Load authored-active quantity components sharing this profile."""

    component_ids = {
        str(leg["component_id"])
        for leg in legs
        if leg.get("component_id") not in (None, "")
    }
    profile_ids = {
        str(leg["profile_id"])
        for leg in legs
        if leg.get("profile_id") not in (None, "")
    }
    if len(component_ids) != 1 or len(profile_ids) != 1:
        return [], []
    component_id = next(iter(component_ids))
    profile_id = next(iter(profile_ids))
    other_legs = [
        _leg_dict(row)
        for row in conn.execute(
            """
            SELECT l.*
            FROM custody_component_legs l
            JOIN custody_components c ON c.id = l.component_id
            WHERE c.profile_id = ?
              AND c.state = 'active'
              AND c.conservation_mode = 'quantity'
              AND c.id != ?
            ORDER BY c.id, l.ordinal, l.id
            """,
            (profile_id, component_id),
        ).fetchall()
    ]
    other_allocations = [
        _allocation_dict(row)
        for row in conn.execute(
            """
            SELECT a.*
            FROM custody_component_allocations a
            JOIN custody_components c ON c.id = a.component_id
            WHERE c.profile_id = ?
              AND c.state = 'active'
              AND c.conservation_mode = 'quantity'
              AND c.id != ?
            ORDER BY c.id, a.ordinal, a.id
            """,
            (profile_id, component_id),
        ).fetchall()
    ]
    return other_legs, other_allocations


def _cross_component_untracked_continuity_issues(
    legs: Sequence[Mapping[str, Any]],
    wallet_rows: Mapping[str, Mapping[str, Any]],
    *,
    current_component_ids: set[str],
) -> list[dict[str, Any]]:
    """Apply chronological continuity to reused missing-wallet placeholders."""

    untracked_wallet_ids = {
        wallet_id
        for wallet_id, row in wallet_rows.items()
        if normalize_wallet_kind_alias(_record_value(row, "wallet_kind"))
        == "untracked"
    }
    if not untracked_wallet_ids:
        return []
    by_location: dict[tuple[str, str, str], list[Mapping[str, Any]]] = defaultdict(list)
    for leg in legs:
        wallet_id = str(leg.get("wallet_id") or "")
        if wallet_id not in untracked_wallet_ids:
            continue
        key = (
            wallet_id,
            str(leg.get("exposure") or ""),
            str(leg.get("conservation_unit") or ""),
        )
        by_location[key].append(leg)

    issues: list[dict[str, Any]] = []
    for location_legs in by_location.values():
        component_ids = {
            str(leg["component_id"])
            for leg in location_legs
            if leg.get("component_id") not in (None, "")
        }
        if len(component_ids) < 2 or not (component_ids & current_component_ids):
            continue
        for issue in _location_continuity_issues(location_legs):
            issues.append(
                {
                    **issue,
                    "component_ids": sorted(component_ids),
                    "message": (
                        "a reused untracked custody location spends before or "
                        "beyond value credited by the connected active route"
                    ),
                }
            )
    return issues


def _db_anchor_validation(
    conn: sqlite3.Connection,
    legs: Sequence[Mapping[str, Any]],
    *,
    allocations: Sequence[Mapping[str, Any]] = (),
    conservation_mode: str = "quantity",
    profile_route_issues: Sequence[Mapping[str, Any]] | None = None,
) -> dict[str, Any]:
    """Validate evidence-row direction, scope facts, and complete coverage."""

    issues: list[dict[str, Any]] = []
    by_transaction: dict[str, list[Mapping[str, Any]]] = defaultdict(list)
    transaction_rows: dict[str, sqlite3.Row] = {}
    wallet_rows: dict[str, sqlite3.Row] = {}
    for wallet_id in sorted(
        {
            str(leg["wallet_id"])
            for leg in legs
            if leg.get("wallet_id") not in (None, "")
        }
    ):
        wallet_row = conn.execute(
            "SELECT id, kind AS wallet_kind, config_json FROM wallets WHERE id = ?",
            (wallet_id,),
        ).fetchone()
        if wallet_row is not None:
            wallet_rows[wallet_id] = wallet_row
    excluded_transactions: set[str] = set()
    for leg in legs:
        transaction_id = leg.get("transaction_id")
        anchor_transaction_id = leg.get("anchor_transaction_id") or transaction_id
        role = str(leg["role"])
        if (
            role in {"source", "destination", "retained"}
            and int(leg.get("amount_msat") or 0) > 0
            and not leg.get("wallet_id")
        ):
            issues.append(
                {
                    "code": "owned_leg_wallet_missing",
                    "leg_id": leg["id"],
                    "role": role,
                }
            )
        if transaction_id is None:
            if anchor_transaction_id is not None:
                issues.append(
                    {
                        "code": "anchor_transaction_retracted",
                        "message": (
                            "an imported transaction anchor was removed; re-import "
                            "it or author a reviewed replacement revision"
                        ),
                        "leg_id": leg["id"],
                        "transaction_id": str(anchor_transaction_id),
                    }
                )
                continue
            if role in {"source", "destination", "retained"} and int(leg["amount_msat"]) > 0:
                if not leg.get("wallet_id"):
                    issues.append(
                        {
                            "code": "transactionless_leg_wallet_missing",
                            "leg_id": leg["id"],
                            "role": role,
                        }
                    )
                if not leg.get("occurred_at"):
                    issues.append(
                        {
                            "code": "leg_occurred_at_missing",
                            "leg_id": leg["id"],
                            "role": role,
                        }
                    )
            continue
        tx_id = str(transaction_id)
        if anchor_transaction_id is not None and str(anchor_transaction_id) != tx_id:
            issues.append(
                {
                    "code": "anchor_transaction_identity_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "anchor_transaction_id": str(anchor_transaction_id),
                }
            )
        by_transaction[tx_id].append(leg)
        row = transaction_rows.get(tx_id)
        if row is None:
            row = conn.execute(
                """
                SELECT t.id, t.wallet_id, t.direction, t.asset, t.amount, t.fee,
                       t.amount_includes_fee, t.occurred_at, t.excluded,
                       t.external_id, t.raw_json,
                       w.kind AS wallet_kind, w.config_json AS config_json
                FROM transactions t
                JOIN wallets w ON w.id = t.wallet_id
                WHERE t.id = ?
                """,
                (tx_id,),
            ).fetchone()
            if row is None:
                issues.append(
                    {"code": "anchor_transaction_missing", "transaction_id": tx_id}
                )
                continue
            transaction_rows[tx_id] = row
        issues.extend(_anchor_rail_scope_issues(leg, row))
        if bool(row["excluded"]) and tx_id not in excluded_transactions:
            excluded_transactions.add(tx_id)
            issues.append(
                {
                    "code": "anchor_transaction_excluded",
                    "message": (
                        "an imported transaction anchor is excluded from journals; "
                        "supersede the component or include the evidence again"
                    ),
                    "transaction_id": tx_id,
                }
            )
        if role == "source" and row["direction"] != "outbound":
            issues.append(
                {
                    "code": "source_anchor_direction_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "direction": row["direction"],
                }
            )
        if role in {"destination", "retained"} and row["direction"] != "inbound":
            issues.append(
                {
                    "code": "destination_anchor_direction_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "direction": row["direction"],
                }
            )
        if role in {"fee", "external"} and row["direction"] != "outbound":
            issues.append(
                {
                    "code": "loss_anchor_direction_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "direction": row["direction"],
                    "role": role,
                }
            )
        if normalize_asset_code(leg["asset"]) != normalize_asset_code(row["asset"]):
            issues.append(
                {
                    "code": "anchor_asset_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "leg_asset": leg["asset"],
                    "transaction_asset": row["asset"],
                }
            )
        if leg.get("wallet_id") is not None and leg["wallet_id"] != row["wallet_id"]:
            issues.append(
                {
                    "code": "anchor_wallet_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "leg_wallet_id": leg["wallet_id"],
                    "transaction_wallet_id": row["wallet_id"],
                }
            )
        leg_occurred_at = parse_iso_datetime_or_none(leg.get("occurred_at"))
        row_occurred_at = parse_iso_datetime_or_none(row["occurred_at"])
        if leg_occurred_at is None:
            issues.append(
                {
                    "code": "leg_occurred_at_invalid",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                }
            )
        elif row_occurred_at is None or leg_occurred_at != row_occurred_at:
            issues.append(
                {
                    "code": "anchor_occurred_at_mismatch",
                    "leg_id": leg["id"],
                    "transaction_id": tx_id,
                    "leg_occurred_at": leg.get("occurred_at"),
                    "transaction_occurred_at": row["occurred_at"],
                }
            )

    coverage: list[dict[str, Any]] = []
    for transaction_id in sorted(by_transaction):
        row = transaction_rows.get(transaction_id)
        if row is None:
            continue
        actual = sum(
            int(leg["amount_msat"])
            for leg in by_transaction[transaction_id]
            if (
                (row["direction"] == "outbound" and leg["role"] == "source")
                or (
                    row["direction"] == "inbound"
                    and leg["role"] in {"destination", "retained"}
                )
            )
        )
        expected = int(row["amount"] or 0)
        if row["direction"] == "outbound" and not bool(row["amount_includes_fee"]):
            expected += int(row["fee"] or 0)
        coverage_row = {
            "transaction_id": transaction_id,
            "direction": row["direction"],
            "raw_economic_msat": expected,
            "reviewed_component_msat": actual,
            "reviewed_minus_raw_msat": actual - expected,
        }
        coverage.append(coverage_row)
        if actual != expected:
            issues.append(
                {
                    "code": "anchor_coverage_mismatch",
                    **coverage_row,
                }
            )

    if conservation_mode == "quantity":
        # Pure validation can inspect authored scopes. For anchored or tracked
        # legs whose authored chain/network is intentionally omitted, enrich a
        # copy from the imported transaction/wallet evidence before checking
        # cross-rail network compatibility. This prevents two omitted fields
        # from disguising a mainnet -> regtest allocation.
        scoped_legs = _scoped_leg_copies(legs, transaction_rows, wallet_rows)
        existing_scope_keys = {
            (
                issue.get("code"),
                issue.get("allocation_id"),
                issue.get("source_leg_id"),
                issue.get("sink_leg_id"),
            )
            for issue in issues
            if str(issue.get("code") or "").startswith("allocation_network_")
        }
        for scope_issue in _quantity_allocation_scope_issues(
            scoped_legs,
            allocations,
            conservation_mode=conservation_mode,
        ):
            scope_key = (
                scope_issue.get("code"),
                scope_issue.get("allocation_id"),
                scope_issue.get("source_leg_id"),
                scope_issue.get("sink_leg_id"),
            )
            if scope_key not in existing_scope_keys:
                issues.append(scope_issue)
                existing_scope_keys.add(scope_key)

        # Enriched anchors must pass the same route-wide and chronological
        # checks as authored pure inputs. Canonical transaction timestamps are
        # used above, so replicated or stale authored values cannot reverse an
        # otherwise valid edge.
        additional_issues = [
            *_allocation_chronology_issues(
                scoped_legs,
                allocations,
                conservation_mode=conservation_mode,
            ),
            *_quantity_scope_connectivity_issues(
                scoped_legs,
                allocations,
                conservation_mode=conservation_mode,
            ),
        ]

        # An unknown location is also not a reset between separately authored
        # active components. Combine the candidate with other authored-active
        # quantity routes in the profile and propagate known domains through
        # their shared wallet locations.
        current_component_ids = {
            str(leg["component_id"])
            for leg in legs
            if leg.get("component_id") not in (None, "")
        }
        if profile_route_issues is not None:
            additional_issues.extend(
                dict(issue)
                for issue in profile_route_issues
                if current_component_ids
                & set(str(item) for item in issue.get("component_ids", ()))
            )
        else:
            other_legs, other_allocations = _other_active_quantity_content(conn, legs)
        if profile_route_issues is None and other_legs:
            combined_legs = [*legs, *other_legs]
            combined_transaction_rows, combined_wallet_rows = _load_scope_evidence(
                conn, combined_legs
            )
            combined_scoped_legs = _scoped_leg_copies(
                combined_legs,
                combined_transaction_rows,
                combined_wallet_rows,
            )
            combined_allocations = _allocations_with_component_inference(
                combined_scoped_legs,
                [*allocations, *other_allocations],
                conservation_mode=conservation_mode,
            )
            additional_issues.extend(
                issue
                for issue in _quantity_scope_connectivity_issues(
                    combined_scoped_legs,
                    combined_allocations,
                    conservation_mode=conservation_mode,
                )
                if current_component_ids
                & set(str(item) for item in issue.get("component_ids", ()))
            )
            additional_issues.extend(
                _cross_component_untracked_continuity_issues(
                    combined_scoped_legs,
                    combined_wallet_rows,
                    current_component_ids=current_component_ids,
                )
            )

        existing_issue_payloads = {
            json.dumps(issue, sort_keys=True, separators=(",", ":"))
            for issue in issues
        }
        for issue in additional_issues:
            payload = json.dumps(issue, sort_keys=True, separators=(",", ":"))
            if payload not in existing_issue_payloads:
                issues.append(issue)
                existing_issue_payloads.add(payload)
    if not by_transaction:
        issues.append(
            {
                "code": "transaction_anchor_missing",
                "message": "an effective component requires at least one imported transaction anchor",
            }
        )
    return {"valid": not issues, "issues": issues, "transaction_coverage": coverage}


def _profile_active_route_issues(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
) -> list[dict[str, Any]]:
    """Compute cross-component route issues once for a profile operation."""

    legs = [
        _leg_dict(row)
        for row in conn.execute(
            """
            SELECT l.*
            FROM custody_component_legs l
            JOIN custody_components c ON c.id = l.component_id
            WHERE c.profile_id = ? AND c.state = 'active'
              AND c.conservation_mode = 'quantity'
            ORDER BY c.id, l.ordinal, l.id
            """,
            (profile_id,),
        ).fetchall()
    ]
    if not legs:
        return []
    allocations = [
        _allocation_dict(row)
        for row in conn.execute(
            """
            SELECT a.*
            FROM custody_component_allocations a
            JOIN custody_components c ON c.id = a.component_id
            WHERE c.profile_id = ? AND c.state = 'active'
              AND c.conservation_mode = 'quantity'
            ORDER BY c.id, a.ordinal, a.id
            """,
            (profile_id,),
        ).fetchall()
    ]
    transaction_rows, wallet_rows = _load_scope_evidence(conn, legs)
    scoped_legs = _scoped_leg_copies(legs, transaction_rows, wallet_rows)
    completed_allocations = _allocations_with_component_inference(
        scoped_legs,
        allocations,
        conservation_mode="quantity",
    )
    component_ids = {
        str(leg["component_id"])
        for leg in scoped_legs
        if leg.get("component_id") not in (None, "")
    }
    return [
        *_quantity_scope_connectivity_issues(
            scoped_legs,
            completed_allocations,
            conservation_mode="quantity",
        ),
        *_cross_component_untracked_continuity_issues(
            scoped_legs,
            wallet_rows,
            current_component_ids=component_ids,
        ),
    ]


def _materialize_component(
    conn: sqlite3.Connection,
    row: sqlite3.Row,
    *,
    include_local_evidence: bool = True,
    profile_route_issues: Sequence[Mapping[str, Any]] | None = None,
) -> dict[str, Any]:
    legs = [
        _leg_dict(leg)
        for leg in conn.execute(
            "SELECT * FROM custody_component_legs WHERE component_id = ? ORDER BY ordinal, id",
            (row["id"],),
        ).fetchall()
    ]
    if not include_local_evidence:
        # ``location_ref`` may identify a private node/channel, local file, or
        # other Tier-3 custody location. Keep it in SQLite/explicit local CLI
        # reads, but never include it in renderer/AI-safe materializations.
        legs = [
            {key: value for key, value in leg.items() if key != "location_ref"}
            for leg in legs
        ]
    allocations = [
        _allocation_dict(allocation)
        for allocation in conn.execute(
            "SELECT * FROM custody_component_allocations "
            "WHERE component_id = ? ORDER BY ordinal, id",
            (row["id"],),
        ).fetchall()
    ]
    validation = validate_conservation(
        legs,
        allocations=allocations,
        conservation_mode=row["conservation_mode"],
        conversion_policy=row["conversion_policy"],
        conversion_reviewed=bool(row["conversion_reviewed"]),
    )
    commitment_issues: list[dict[str, Any]] = []
    expected_leg_count = row["expected_leg_count"]
    expected_allocation_count = row["expected_allocation_count"]
    if expected_leg_count is None or expected_allocation_count is None:
        commitment_issues.append(
            {
                "code": "component_content_commitment_missing",
                "message": (
                    "this legacy component header does not commit to its child row counts; "
                    "create a new revision before activation"
                ),
            }
        )
    else:
        if int(expected_leg_count) != len(legs):
            commitment_issues.append(
                {
                    "code": "component_leg_count_mismatch",
                    "expected": int(expected_leg_count),
                    "actual": len(legs),
                }
            )
        if int(expected_allocation_count) != len(allocations):
            commitment_issues.append(
                {
                    "code": "component_allocation_count_mismatch",
                    "expected": int(expected_allocation_count),
                    "actual": len(allocations),
                }
            )
    if commitment_issues:
        validation = dict(validation)
        validation["activatable"] = False
        validation["issues"] = [*validation["issues"], *commitment_issues]
    anchor_validation = _db_anchor_validation(
        conn,
        legs,
        allocations=allocations,
        conservation_mode=row["conservation_mode"],
        profile_route_issues=profile_route_issues,
    )
    if anchor_validation["issues"]:
        validation = dict(validation)
        validation["activatable"] = False
        validation["issues"] = [*validation["issues"], *anchor_validation["issues"]]
    validation["anchors"] = anchor_validation
    lineage_issues = _replicated_lineage_issues(conn, row)
    if lineage_issues:
        validation = dict(validation)
        validation["activatable"] = False
        validation["issues"] = [*validation["issues"], *lineage_issues]
    conflicts = (
        _active_membership_conflicts(
            conn, component_id=row["id"], profile_id=row["profile_id"]
        )
        if row["state"] == "active"
        else []
    )
    if conflicts:
        validation = dict(validation)
        validation["activatable"] = False
        validation["issues"] = [
            *validation["issues"],
            {
                "code": "active_transaction_membership_conflict",
                "message": "a transaction belongs to another authored active component",
                "conflicts": conflicts,
            },
        ]
    effective_state = (
        "active"
        if row["state"] == "active" and validation["activatable"]
        else ("superseded" if row["state"] == "superseded" else "draft")
    )
    result = {
        "id": row["id"],
        "lineage_id": row["lineage_id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "revision": int(row["revision"]),
        "component_type": row["component_type"],
        "conservation_mode": row["conservation_mode"],
        "state": row["state"],
        "effective_state": effective_state,
        "evidence_kind": row["evidence_kind"],
        "evidence_grade": row["evidence_grade"],
        "conversion_policy": row["conversion_policy"],
        "conversion_reviewed": bool(row["conversion_reviewed"]),
        "expected_leg_count": (
            None if expected_leg_count is None else int(expected_leg_count)
        ),
        "expected_allocation_count": (
            None
            if expected_allocation_count is None
            else int(expected_allocation_count)
        ),
        "authored_source": row["authored_source"] or "user",
        "notes": row["notes"],
        "change_reason": row["change_reason"],
        "supersedes_component_id": row["supersedes_component_id"],
        "superseded_by_component_id": row["superseded_by_component_id"],
        "activated_at": row["activated_at"],
        "superseded_at": row["superseded_at"],
        "created_at": row["created_at"],
        "legs": legs,
        "allocations": allocations,
        "validation": validation,
    }
    if include_local_evidence:
        result["evidence"] = _json_object(row["evidence_json"], "evidence_json")
        result["conversion_metadata"] = _json_object(
            row["conversion_metadata_json"], "conversion_metadata_json"
        )
    return result


def get_component(
    conn: sqlite3.Connection,
    component_id: str,
    *,
    profile_id: str | None = None,
    include_local_evidence: bool = True,
) -> dict[str, Any]:
    row = _row(conn, component_id)
    if profile_id is not None and row["profile_id"] != profile_id:
        raise _error(
            "custody component was not found in the profile",
            "not_found",
            details={"component_id": component_id, "profile_id": profile_id},
        )
    return _materialize_component(
        conn, row, include_local_evidence=include_local_evidence
    )


def create_component(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    component_type: str,
    legs: Iterable[Mapping[str, Any]],
    allocations: Iterable[Mapping[str, Any]] | None = None,
    conservation_mode: str = "quantity",
    evidence_kind: str | None = None,
    evidence_grade: str | None = None,
    evidence: Mapping[str, Any] | str | None = None,
    conversion_policy: str | None = None,
    conversion_reviewed: bool = False,
    conversion_metadata: Mapping[str, Any] | str | None = None,
    notes: str | None = None,
    change_reason: str | None = None,
    component_id: str | None = None,
    lineage_id: str | None = None,
    created_at: str | None = None,
    authored_source: str = "user",
) -> dict[str, Any]:
    workspace_id = _required_text(workspace_id, "workspace_id")
    profile_id = _required_text(profile_id, "profile_id")
    component_type = _normalize_component_type(component_type)
    conservation_mode = _normalize_mode(conservation_mode)
    normalized_legs = normalize_legs(legs)
    normalized_allocations = normalize_allocations(allocations, normalized_legs)
    component_id = _optional_text(component_id, "component_id") or str(uuid.uuid4())
    lineage_id = _optional_text(lineage_id, "lineage_id") or component_id
    timestamp = parse_timestamp(created_at) if created_at is not None else _now_iso()
    authored_source = _required_text(authored_source, "authored_source")
    if authored_source not in AUTHORED_SOURCES:
        raise _error(
            "authored_source is invalid",
            "custody_component_validation",
            details={"authored_source": authored_source},
        )
    if type(conversion_reviewed) is not bool:
        raise _error(
            "conversion_reviewed must be a boolean",
            "custody_component_validation",
        )
    _scope(conn, workspace_id, profile_id)
    _validate_leg_anchors(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        legs=normalized_legs,
    )
    existing_lineage = conn.execute(
        "SELECT id FROM custody_components WHERE profile_id = ? AND lineage_id = ? LIMIT 1",
        (profile_id, lineage_id),
    ).fetchone()
    if existing_lineage:
        raise _error(
            "component lineage already exists; create a revision instead",
            "custody_component_lineage_exists",
            details={"lineage_id": lineage_id, "component_id": existing_lineage["id"]},
        )
    with _savepoint(conn):
        conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, conservation_mode, state, evidence_kind,
                evidence_grade, evidence_json, conversion_policy,
                conversion_reviewed, conversion_metadata_json,
                expected_leg_count, expected_allocation_count, authored_source, notes,
                change_reason, created_at
            ) VALUES(?, ?, ?, ?, 1, ?, ?, 'draft', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                component_id, lineage_id, workspace_id, profile_id, component_type,
                conservation_mode,
                _optional_text(evidence_kind, "evidence_kind", token=True),
                _optional_text(evidence_grade, "evidence_grade", token=True),
                _json_text(evidence, "evidence"),
                _optional_text(conversion_policy, "conversion_policy", token=True),
                int(conversion_reviewed),
                _json_text(conversion_metadata, "conversion_metadata"),
                len(normalized_legs), len(normalized_allocations),
                authored_source,
                _optional_text(notes, "notes"),
                _optional_text(change_reason, "change_reason"),
                timestamp,
            ),
        )
        _insert_legs(
            conn,
            component_id=component_id,
            workspace_id=workspace_id,
            profile_id=profile_id,
            legs=normalized_legs,
            created_at=timestamp,
        )
        _insert_allocations(
            conn,
            component_id=component_id,
            workspace_id=workspace_id,
            profile_id=profile_id,
            allocations=normalized_allocations,
            created_at=timestamp,
        )
    return get_component(conn, component_id)


def _copy_leg_inputs(conn: sqlite3.Connection, component_id: str) -> list[dict[str, Any]]:
    return [
        {
            key: value
            for key, value in _leg_dict(row).items()
            if key
            in {
                "role", "rail", "chain", "network", "asset", "exposure",
                "conservation_unit", "amount_msat", "valuation_unit",
                "valuation_amount", "occurred_at", "transaction_id",
                "anchor_transaction_id", "wallet_id", "location_ref", "notes",
            }
        }
        for row in conn.execute(
            "SELECT * FROM custody_component_legs WHERE component_id = ? ORDER BY ordinal, id",
            (component_id,),
        ).fetchall()
    ]


def _copy_allocation_inputs(
    conn: sqlite3.Connection, component_id: str
) -> list[dict[str, Any]]:
    return [
        {
            "source_leg_id": row["source_leg_id"],
            "sink_leg_id": row["sink_leg_id"],
            "source_amount_msat": int(row["source_amount_msat"]),
            "sink_amount_msat": int(row["sink_amount_msat"]),
        }
        for row in conn.execute(
            "SELECT * FROM custody_component_allocations "
            "WHERE component_id = ? ORDER BY ordinal, id",
            (component_id,),
        ).fetchall()
    ]


def update_component(
    conn: sqlite3.Connection,
    component_id: str,
    *,
    legs: Iterable[Mapping[str, Any]] | object = _UNSET,
    allocations: Iterable[Mapping[str, Any]] | None | object = _UNSET,
    component_type: str | object = _UNSET,
    conservation_mode: str | object = _UNSET,
    evidence_kind: str | None | object = _UNSET,
    evidence_grade: str | None | object = _UNSET,
    evidence: Mapping[str, Any] | str | None | object = _UNSET,
    conversion_policy: str | None | object = _UNSET,
    conversion_reviewed: bool | object = _UNSET,
    conversion_metadata: Mapping[str, Any] | str | None | object = _UNSET,
    notes: str | None | object = _UNSET,
    change_reason: str | None = None,
    new_component_id: str | None = None,
    created_at: str | None = None,
    authored_source: str = "user",
) -> dict[str, Any]:
    """Create a new immutable draft revision; never rewrite economic legs."""

    old = _row(conn, component_id)
    existing_draft = conn.execute(
        """
        SELECT id FROM custody_components
        WHERE profile_id = ? AND lineage_id = ? AND state = 'draft' AND id != ?
        """,
        (old["profile_id"], old["lineage_id"], component_id),
    ).fetchone()
    if existing_draft:
        raise _error(
            "component lineage already has a draft revision",
            "custody_component_draft_exists",
            details={"draft_component_id": existing_draft["id"]},
        )
    legs_unchanged = legs is _UNSET
    raw_legs = _copy_leg_inputs(conn, component_id) if legs_unchanged else legs
    normalized_legs = normalize_legs(raw_legs)  # type: ignore[arg-type]
    input_leg_ordinals = {
        str(leg["id"]): int(leg["ordinal"]) for leg in normalized_legs
    }
    # Leg ids identify immutable revision rows, so a new revision always gets
    # new ids even when a caller feeds a previous get_component payload back.
    for leg in normalized_legs:
        leg["id"] = str(uuid.uuid4())
    if allocations is _UNSET:
        if legs_unchanged:
            # Copied legs receive fresh ids, so map old allocation endpoints by
            # ordinal rather than leaking the superseded leg ids.
            old_rows = conn.execute(
                "SELECT id, ordinal FROM custody_component_legs "
                "WHERE component_id = ? ORDER BY ordinal, id",
                (component_id,),
            ).fetchall()
            old_ordinal = {row["id"]: int(row["ordinal"]) for row in old_rows}
            raw_allocations = [
                {
                    "source_ordinal": old_ordinal[allocation["source_leg_id"]],
                    "sink_ordinal": old_ordinal[allocation["sink_leg_id"]],
                    "source_amount_msat": allocation["source_amount_msat"],
                    "sink_amount_msat": allocation["sink_amount_msat"],
                }
                for allocation in _copy_allocation_inputs(conn, component_id)
            ]
        else:
            raw_allocations = []
    else:
        raw_allocations = []
        for allocation in allocations or []:  # type: ignore[union-attr]
            rewritten = dict(allocation)
            for endpoint in ("source", "sink"):
                leg_id = rewritten.get(f"{endpoint}_leg_id")
                if leg_id is not None and str(leg_id) in input_leg_ordinals:
                    rewritten[f"{endpoint}_ordinal"] = input_leg_ordinals[str(leg_id)]
                    rewritten.pop(f"{endpoint}_leg_id", None)
            # Allocation rows are immutable revision records too.
            rewritten.pop("id", None)
            raw_allocations.append(rewritten)
    normalized_allocations = normalize_allocations(raw_allocations, normalized_legs)  # type: ignore[arg-type]
    _validate_leg_anchors(
        conn,
        workspace_id=old["workspace_id"],
        profile_id=old["profile_id"],
        legs=normalized_legs,
    )
    next_revision = int(
        conn.execute(
            "SELECT COALESCE(MAX(revision), 0) + 1 AS revision "
            "FROM custody_components WHERE profile_id = ? AND lineage_id = ?",
            (old["profile_id"], old["lineage_id"]),
        ).fetchone()["revision"]
    )
    new_id = _optional_text(new_component_id, "new_component_id") or str(uuid.uuid4())
    timestamp = parse_timestamp(created_at) if created_at is not None else _now_iso()
    authored_source = _required_text(authored_source, "authored_source")
    if authored_source not in AUTHORED_SOURCES:
        raise _error(
            "authored_source is invalid",
            "custody_component_validation",
            details={"authored_source": authored_source},
        )

    new_type = old["component_type"] if component_type is _UNSET else _normalize_component_type(component_type)
    new_mode = old["conservation_mode"] if conservation_mode is _UNSET else _normalize_mode(conservation_mode)
    new_evidence_kind = old["evidence_kind"] if evidence_kind is _UNSET else _optional_text(evidence_kind, "evidence_kind", token=True)
    new_evidence_grade = old["evidence_grade"] if evidence_grade is _UNSET else _optional_text(evidence_grade, "evidence_grade", token=True)
    new_evidence_json = old["evidence_json"] if evidence is _UNSET else _json_text(evidence, "evidence")
    new_policy = old["conversion_policy"] if conversion_policy is _UNSET else _optional_text(conversion_policy, "conversion_policy", token=True)
    if conversion_reviewed is _UNSET:
        new_reviewed = bool(old["conversion_reviewed"])
    else:
        if type(conversion_reviewed) is not bool:
            raise _error("conversion_reviewed must be a boolean", "custody_component_validation")
        new_reviewed = conversion_reviewed
    new_conversion_json = old["conversion_metadata_json"] if conversion_metadata is _UNSET else _json_text(conversion_metadata, "conversion_metadata")
    new_notes = old["notes"] if notes is _UNSET else _optional_text(notes, "notes")

    with _savepoint(conn):
        if old["state"] == "draft":
            conn.execute(
                """
                UPDATE custody_components
                SET state = 'superseded', superseded_at = ?,
                    superseded_by_component_id = ?,
                    change_reason = COALESCE(?, change_reason)
                WHERE id = ? AND state = 'draft'
                """,
                (timestamp, new_id, _optional_text(change_reason, "change_reason"), component_id),
            )
        conn.execute(
            """
            INSERT INTO custody_components(
                id, lineage_id, workspace_id, profile_id, revision,
                component_type, conservation_mode, state, evidence_kind,
                evidence_grade, evidence_json, conversion_policy,
                conversion_reviewed, conversion_metadata_json,
                expected_leg_count, expected_allocation_count, authored_source, notes,
                change_reason, supersedes_component_id, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, 'draft', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                new_id, old["lineage_id"], old["workspace_id"], old["profile_id"],
                next_revision, new_type, new_mode, new_evidence_kind,
                new_evidence_grade, new_evidence_json, new_policy,
                int(new_reviewed), new_conversion_json,
                len(normalized_legs), len(normalized_allocations), authored_source,
                new_notes,
                _optional_text(change_reason, "change_reason"), component_id, timestamp,
            ),
        )
        _insert_legs(
            conn,
            component_id=new_id,
            workspace_id=old["workspace_id"],
            profile_id=old["profile_id"],
            legs=normalized_legs,
            created_at=timestamp,
        )
        _insert_allocations(
            conn,
            component_id=new_id,
            workspace_id=old["workspace_id"],
            profile_id=old["profile_id"],
            allocations=normalized_allocations,
            created_at=timestamp,
        )
    return get_component(conn, new_id)


def _activation_error(component: Mapping[str, Any]) -> AppError:
    return _error(
        "custody component is incomplete or does not conserve",
        "custody_component_incomplete",
        details={
            "component_id": component["id"],
            "validation": component["validation"],
        },
    )


def activate_component(
    conn: sqlite3.Connection,
    component_id: str,
    *,
    activated_at: str | None = None,
) -> dict[str, Any]:
    component = get_component(conn, component_id)
    if component["state"] == "superseded":
        raise _error(
            "superseded revisions cannot be activated directly",
            "custody_component_superseded",
            details={"component_id": component_id},
        )
    # Conflicts with the active revision being replaced in the same lineage are
    # expected; all other raw-active overlaps are blocking.
    external_conflicts = [
        conflict
        for conflict in _active_membership_conflicts(
            conn, component_id=component_id, profile_id=component["profile_id"]
        )
        if conn.execute(
            "SELECT lineage_id FROM custody_components WHERE id = ?",
            (conflict["component_id"],),
        ).fetchone()["lineage_id"]
        != component["lineage_id"]
    ]
    validation = dict(component["validation"])
    validation["issues"] = list(validation["issues"])
    if external_conflicts:
        validation["activatable"] = False
        validation["issues"].append(
            {
                "code": "active_transaction_membership_conflict",
                "message": "a transaction belongs to another active component",
                "conflicts": external_conflicts,
            }
        )
    if not validation["activatable"]:
        failed = dict(component)
        failed["validation"] = validation
        raise _activation_error(failed)

    timestamp = activated_at or _now_iso()
    transaction_ids = sorted(
        {
            str(leg["transaction_id"])
            for leg in component["legs"]
            if leg["transaction_id"] is not None
        }
    )
    with _savepoint(conn):
        conn.execute(
            """
            UPDATE custody_components
            SET state = 'superseded', superseded_at = ?,
                superseded_by_component_id = ?
            WHERE profile_id = ? AND lineage_id = ?
              AND state = 'active' AND id != ?
            """,
            (
                timestamp, component_id, component["profile_id"],
                component["lineage_id"], component_id,
            ),
        )
        conn.execute(
            "DELETE FROM custody_component_transaction_memberships WHERE component_id = ?",
            (component_id,),
        )
        try:
            conn.executemany(
                """
                INSERT INTO custody_component_transaction_memberships(
                    component_id, profile_id, transaction_id, created_at
                ) VALUES(?, ?, ?, ?)
                """,
                [
                    (component_id, component["profile_id"], tx_id, timestamp)
                    for tx_id in transaction_ids
                ],
            )
        except sqlite3.IntegrityError as exc:
            raise _error(
                "a transaction already belongs to another active custody component",
                "custody_component_membership_conflict",
                details={"component_id": component_id, "transaction_ids": transaction_ids},
            ) from exc
        if component["state"] != "active":
            conn.execute(
                """
                UPDATE custody_components
                SET state = 'active', activated_at = ?, superseded_at = NULL,
                    superseded_by_component_id = NULL
                WHERE id = ? AND state = 'draft'
                """,
                (timestamp, component_id),
            )
            if not conn.execute(
                "SELECT 1 FROM custody_components WHERE id = ? AND state = 'active'",
                (component_id,),
            ).fetchone():
                raise _error(
                    "custody component changed before activation",
                    "custody_component_state_conflict",
                    details={"component_id": component_id},
                )
        conn.execute(
            """
            UPDATE custody_components
            SET superseded_by_component_id = ?
            WHERE id = ? AND state = 'superseded'
            """,
            (component_id, component["supersedes_component_id"]),
        )
        _invalidate_journals(conn, component["profile_id"])
    return get_component(conn, component_id)


def supersede_component(
    conn: sqlite3.Connection,
    component_id: str,
    *,
    reason: str | None = None,
    superseded_at: str | None = None,
) -> dict[str, Any]:
    row = _row(conn, component_id)
    if row["state"] == "superseded":
        return get_component(conn, component_id)
    timestamp = superseded_at or _now_iso()
    with _savepoint(conn):
        conn.execute(
            """
            UPDATE custody_components
            SET state = 'superseded', superseded_at = ?,
                change_reason = COALESCE(?, change_reason)
            WHERE id = ? AND state != 'superseded'
            """,
            (timestamp, _optional_text(reason, "reason"), component_id),
        )
        if row["state"] == "active":
            _invalidate_journals(conn, row["profile_id"])
    if row["state"] == "active":
        # A replicated conflict can leave two raw-active revisions with no
        # effective memberships.  Superseding the losing revision is the
        # manual resolution; immediately restore memberships for the surviving
        # effective component instead of waiting for another sync replay.
        reconcile_active_memberships(conn, profile_id=row["profile_id"])
    return get_component(conn, component_id)


def undo_supersede(
    conn: sqlite3.Connection,
    component_id: str,
    *,
    reason: str | None = "undo_supersede",
    new_component_id: str | None = None,
    created_at: str | None = None,
    authored_source: str = "user",
) -> dict[str, Any]:
    row = _row(conn, component_id)
    if row["state"] != "superseded":
        raise _error(
            "only a superseded revision can be restored",
            "custody_component_not_superseded",
            details={"component_id": component_id, "state": row["state"]},
        )
    return update_component(
        conn,
        component_id,
        change_reason=reason,
        new_component_id=new_component_id,
        created_at=created_at,
        authored_source=authored_source,
    )


def list_components(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    state: str | None = None,
    component_type: str | None = None,
    transaction_id: str | None = None,
    effective_only: bool = False,
    include_local_evidence: bool = True,
    limit: int = 200,
) -> list[dict[str, Any]]:
    if type(limit) is not int or not 1 <= limit <= 1001:
        raise _error(
            "limit must be between 1 and 1001",
            "custody_component_validation",
            details={"limit": limit},
        )
    params: list[Any] = [profile_id]
    where = ["c.profile_id = ?"]
    if state is not None:
        normalized_state = _required_text(state, "state", token=True)
        if normalized_state not in COMPONENT_STATES:
            raise _error("state is not supported", "custody_component_validation")
        where.append("c.state = ?")
        params.append(normalized_state)
    if component_type is not None:
        where.append("c.component_type = ?")
        params.append(_normalize_component_type(component_type))
    if transaction_id is not None:
        where.append(
            "EXISTS (SELECT 1 FROM custody_component_legs l "
            "WHERE l.component_id = c.id AND l.transaction_id = ?)"
        )
        params.append(transaction_id)
    rows = conn.execute(
        f"""
        SELECT c.* FROM custody_components c
        WHERE {' AND '.join(where)}
        ORDER BY c.created_at DESC, c.revision DESC, c.id DESC
        LIMIT ?
        """,
        (*params, limit),
    ).fetchall()
    profile_route_issues = _profile_active_route_issues(
        conn,
        profile_id=profile_id,
    )
    result = [
        _materialize_component(
            conn,
            row,
            include_local_evidence=include_local_evidence,
            profile_route_issues=profile_route_issues,
        )
        for row in rows
    ]
    return [item for item in result if item["effective_state"] == "active"] if effective_only else result


def list_effective_components(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transaction_id: str | None = None,
    include_local_evidence: bool = False,
    limit: int = 1000,
) -> list[dict[str, Any]]:
    return list_components(
        conn,
        profile_id=profile_id,
        state="active",
        transaction_id=transaction_id,
        effective_only=True,
        include_local_evidence=include_local_evidence,
        limit=limit,
    )


def iter_effective_components(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transaction_id: str | None = None,
    include_local_evidence: bool = False,
) -> Iterator[dict[str, Any]]:
    """Yield every effective component for effective-only consumers.

    Unlike the user-facing list API this iterator is deliberately unbounded;
    callers process one materialized component at a time and therefore cannot
    silently truncate long wallet-migration histories at a page limit. Journal
    assembly must use ``iter_authored_active_components`` instead, because it
    must also fail-close incomplete or conflicting active revisions.
    """

    for component in iter_authored_active_components(
        conn,
        profile_id=profile_id,
        transaction_id=transaction_id,
        include_local_evidence=include_local_evidence,
    ):
        if component["effective_state"] == "active":
            yield component


def iter_authored_active_components(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transaction_id: str | None = None,
    include_local_evidence: bool = False,
) -> Iterator[dict[str, Any]]:
    """Yield every authored-active component for fail-closed journal input.

    ``iter_effective_components`` is appropriate for views that only want
    usable interpretations.  Journal materialization has a stricter contract:
    an authored ``active`` header must still claim every transaction anchor
    that has arrived locally when its remaining legs are incomplete, invalid,
    or overlap another active component.  Otherwise row-wise replication can
    temporarily turn the raw anchors back into ordinary acquisitions or
    disposals.  Projection decides whether each returned component is usable
    or must produce a component-wide quarantine.

    This internal iterator is deliberately unbounded for long migration
    histories, just like ``iter_effective_components``.
    """

    params: list[Any] = [profile_id]
    transaction_filter = ""
    if transaction_id is not None:
        transaction_filter = (
            "AND EXISTS (SELECT 1 FROM custody_component_legs l "
            "WHERE l.component_id = c.id AND l.transaction_id = ?)"
        )
        params.append(transaction_id)
    rows = conn.execute(
        f"""
        SELECT c.*
        FROM custody_components c
        WHERE c.profile_id = ? AND c.state = 'active'
          {transaction_filter}
        ORDER BY c.created_at, c.revision, c.id
        """,
        params,
    )
    profile_route_issues = _profile_active_route_issues(
        conn,
        profile_id=profile_id,
    )
    for row in rows:
        yield _materialize_component(
            conn,
            row,
            include_local_evidence=include_local_evidence,
            profile_route_issues=profile_route_issues,
        )


def reconcile_active_memberships(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
) -> dict[str, Any]:
    """Rebuild the local uniqueness guard after row-wise replication.

    Invalid, incomplete, or overlapping active rows remain authored as active
    for conflict/audit visibility but receive no effective membership. Journal
    projection still consumes them through ``iter_authored_active_components``
    so their known anchors are quarantined rather than interpreted raw.
    """

    # Replication reconciliation is an integrity operation, not a paginated UI
    # read. Truncating here would delete valid membership rows for every active
    # component after the first 1,000 in a long migration history.
    profile_route_issues = _profile_active_route_issues(
        conn,
        profile_id=profile_id,
    )
    active = [
        _materialize_component(
            conn,
            row,
            include_local_evidence=False,
            profile_route_issues=profile_route_issues,
        )
        for row in conn.execute(
            """
            SELECT * FROM custody_components
            WHERE profile_id = ? AND state = 'active'
            ORDER BY created_at, revision, id
            """,
            (profile_id,),
        ).fetchall()
    ]
    effective = [item for item in active if item["effective_state"] == "active"]
    timestamp = _now_iso()
    with _savepoint(conn):
        conn.execute(
            "DELETE FROM custody_component_transaction_memberships WHERE profile_id = ?",
            (profile_id,),
        )
        for component in effective:
            tx_ids = sorted(
                {
                    leg["transaction_id"]
                    for leg in component["legs"]
                    if leg["transaction_id"] is not None
                }
            )
            conn.executemany(
                """
                INSERT INTO custody_component_transaction_memberships(
                    component_id, profile_id, transaction_id, created_at
                ) VALUES(?, ?, ?, ?)
                """,
                [(component["id"], profile_id, tx_id, timestamp) for tx_id in tx_ids],
            )
    return {
        "profile_id": profile_id,
        "effective_component_ids": [item["id"] for item in effective],
        "incomplete": [
            {"component_id": item["id"], "issues": item["validation"]["issues"]}
            for item in active
            if item["effective_state"] != "active"
        ],
    }


__all__ = [
    "COMPONENT_STATES",
    "COMPONENT_TYPES",
    "CONSERVATION_MODES",
    "LEG_ROLES",
    "activate_component",
    "create_component",
    "get_component",
    "iter_authored_active_components",
    "iter_effective_components",
    "list_components",
    "list_effective_components",
    "normalize_legs",
    "normalize_allocations",
    "reconcile_active_memberships",
    "supersede_component",
    "undo_supersede",
    "update_component",
    "validate_conservation",
]
