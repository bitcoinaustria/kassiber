"""Crash-safe compatibility migration into the authored custody aggregate.

Pending legacy rows become final component aggregates in one transaction:
connected pairs migrate as one atomic group and each payout migrates as one
active conversion. The nullable ``component_id`` on each legacy row is the
bounded compatibility link; only an effective active component replaces that
row at the journal boundary.

Every migrated revision carries two kinds of immutable data:

* physical boundary legs and exact source-to-sink allocations; and
* a typed economic-terms row for policy, swap-fee, payout and review metadata
  which cannot coherently be represented as physical quantity legs.

Reopening an already migrated book performs only an indexed pending-row check.
Compatibility-row edits after activation fail closed because reviewed
economics must be revised on the component.
"""

from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
from decimal import Decimal
import hashlib
import json
import sqlite3
from typing import Any, Iterator, Mapping, Sequence
import uuid

from ..errors import AppError
from ..time_utils import now_iso
from ..wallet_descriptors import normalize_asset_code
from .custody_components import (
    activate_component,
    create_component,
    get_component,
    seal_component_economic_terms,
    supersede_component,
    update_component,
)
from .custody_evidence import resolve_protocol_scope, row_boundary_amounts


_MIGRATION_NAMESPACE = uuid.UUID("95ed148a-743a-4dcc-9b55-ca8cc203d547")
_VALUATION_UNIT = "reviewed_source_msat"


@dataclass(frozen=True)
class ConsolidationResult:
    activated: int = 0
    unchanged: int = 0
    skipped: int = 0

    @property
    def changed(self) -> bool:
        return bool(self.activated or self.skipped)


def _record_migration_issue(
    conn: sqlite3.Connection,
    rows: Sequence[Mapping[str, Any]],
    error: AppError,
) -> None:
    timestamp = now_iso()
    for row in rows:
        legacy_table = str(_field(row, "legacy_table"))
        legacy_source_id = str(_field(row, "id"))
        transaction_ids = sorted(
            {
                str(value)
                for value in (
                    _field(row, "out_transaction_id"),
                    _field(row, "in_transaction_id"),
                )
                if value not in (None, "")
            }
        )
        conn.execute(
            """
            INSERT INTO custody_authored_migration_issues(
                id, workspace_id, profile_id, legacy_table, legacy_source_id,
                issue_code, transaction_ids_json, details_json,
                resolved_at, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)
            ON CONFLICT(legacy_table, legacy_source_id) DO UPDATE SET
                issue_code = excluded.issue_code,
                transaction_ids_json = excluded.transaction_ids_json,
                details_json = excluded.details_json,
                resolved_at = NULL,
                updated_at = excluded.updated_at
            """,
            (
                _stable_id("custody_authored_migration_issue", legacy_table, legacy_source_id),
                _field(row, "workspace_id"),
                _field(row, "profile_id"),
                legacy_table,
                legacy_source_id,
                error.code or "custody_legacy_migration_failed",
                json.dumps(transaction_ids, separators=(",", ":")),
                json.dumps(error.details or {}, sort_keys=True, separators=(",", ":")),
                timestamp,
                timestamp,
            ),
        )


def _resolve_migration_issues(
    conn: sqlite3.Connection,
    rows: Sequence[Mapping[str, Any]],
) -> None:
    timestamp = now_iso()
    for row in rows:
        conn.execute(
            "UPDATE custody_authored_migration_issues "
            "SET resolved_at = COALESCE(resolved_at, ?), updated_at = ? "
            "WHERE legacy_table = ? AND legacy_source_id = ? "
            "AND resolved_at IS NULL",
            (
                timestamp,
                timestamp,
                _field(row, "legacy_table"),
                _field(row, "id"),
            ),
        )


@contextmanager
def _savepoint(conn: sqlite3.Connection, name: str) -> Iterator[None]:
    conn.execute(f"SAVEPOINT {name}")
    try:
        yield
    except Exception:
        conn.execute(f"ROLLBACK TO {name}")
        conn.execute(f"RELEASE {name}")
        raise
    else:
        conn.execute(f"RELEASE {name}")


def _field(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    return row[key] if key in row.keys() else default


def _canonical_float(value: Any) -> str | None:
    if value in (None, ""):
        return None
    if isinstance(value, float):
        # 17 significant digits round-trip the exact SQLite REAL value.  The
        # legacy schema has already discarded any more precise source text.
        return format(value, ".17g")
    return str(Decimal(str(value)))


def _canonical_payload(row: Mapping[str, Any], fields: Sequence[str]) -> dict[str, Any]:
    payload: dict[str, Any] = {}
    for field in fields:
        value = _field(row, field)
        if field == "payout_fiat_value":
            value = _canonical_float(value)
        payload[field] = value
    return payload


def _hash_payload(payload: Mapping[str, Any]) -> str:
    encoded = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _stable_id(*parts: str) -> str:
    return str(uuid.uuid5(_MIGRATION_NAMESPACE, "\x1f".join(parts)))


def _exposure(asset: str) -> str:
    normalized = normalize_asset_code(asset)
    if normalized in {"BTC", "LBTC"}:
        return "bitcoin"
    return f"asset:{normalized.lower()}"


def _protocol_fields(row: Mapping[str, Any]) -> dict[str, str | None]:
    try:
        scope = resolve_protocol_scope(row)
    except (TypeError, ValueError):
        # Existing generic imports may carry assets/rails unknown to today's
        # protocol adapters. Preserve them as explicit external observations;
        # a later review can specialize the rail without migration guessing.
        return {"rail": "external", "chain": None, "network": None}
    return {
        "rail": scope.rail,
        "chain": scope.base_chain,
        "network": scope.network,
    }


def _leg(
    row: Mapping[str, Any] | None,
    *,
    leg_id: str,
    role: str,
    asset: str,
    amount_msat: int,
    occurred_at: str | None,
    location_ref: str | None = None,
    valuation_amount: int | None = None,
) -> dict[str, Any]:
    protocol = (
        _protocol_fields(row)
        if row is not None
        else {"rail": "external", "chain": None, "network": None}
    )
    return {
        "id": leg_id,
        "role": role,
        **protocol,
        "asset": normalize_asset_code(asset),
        "exposure": _exposure(asset),
        "conservation_unit": "msat",
        "amount_msat": amount_msat,
        "valuation_unit": _VALUATION_UNIT if valuation_amount is not None else None,
        "valuation_amount": valuation_amount,
        "occurred_at": occurred_at,
        "transaction_id": _field(row, "transaction_id") if row is not None else None,
        "wallet_id": _field(row, "wallet_id") if row is not None else None,
        "location_ref": location_ref,
    }


def _allocation(
    *,
    allocation_id: str,
    source_leg_id: str,
    sink_leg_id: str,
    source_amount_msat: int,
    sink_amount_msat: int,
) -> dict[str, Any]:
    return {
        "id": allocation_id,
        "source_leg_id": source_leg_id,
        "sink_leg_id": sink_leg_id,
        "source_amount_msat": source_amount_msat,
        "sink_amount_msat": sink_amount_msat,
    }


_PAIR_HASH_FIELDS = (
    "id", "workspace_id", "profile_id", "out_transaction_id",
    "in_transaction_id", "kind", "policy", "notes", "swap_fee_msat",
    "swap_fee_kind", "confidence_at_pair", "pair_source", "out_amount",
    "deleted_at", "created_at",
)
_PAYOUT_HASH_FIELDS = (
    "id", "workspace_id", "profile_id", "out_transaction_id", "kind",
    "policy", "payout_asset", "payout_amount", "payout_occurred_at",
    "payout_fiat_value", "payout_external_id", "counterparty", "notes",
    "swap_fee_msat", "swap_fee_kind", "out_amount", "deleted_at",
    "created_at",
)


def _pair_residual_classification(row: Mapping[str, Any]) -> str:
    return (
        "network_fee"
        if str(_field(row, "kind")) == "swap-refund"
        else "suspense_continuation"
    )


def _pair_spec(row: Mapping[str, Any], _source_hash: str) -> dict[str, Any]:
    return _pair_group_spec([row], require_full_source=False)


def _payout_spec(row: Mapping[str, Any], source_hash: str) -> dict[str, Any]:
    out_asset = normalize_asset_code(_field(row, "out_asset"))
    payout_asset = normalize_asset_code(_field(row, "payout_asset"))
    source_principal = row_boundary_amounts(
        {
            "direction": _field(row, "out_direction"),
            "amount": _field(row, "out_tx_amount"),
            "fee": _field(row, "out_fee"),
            "amount_includes_fee": _field(row, "out_amount_includes_fee"),
        }
    ).principal_msat
    explicit_source = _field(row, "out_amount")
    reviewed_source = (
        source_principal if explicit_source in (None, "") else int(explicit_source)
    )
    payout_amount = int(_field(row, "payout_amount"))
    physical_source_amount = max(0, reviewed_source)
    physical_payout_amount = max(0, payout_amount)
    component_id = _stable_id(
        "direct_swap_payout", str(_field(row, "profile_id")), str(_field(row, "id")), source_hash
    )
    source_leg_id = _stable_id(component_id, "leg", "source")
    target_leg_id = _stable_id(component_id, "leg", "target")
    out_row = {
        "transaction_id": _field(row, "out_transaction_id"),
        "wallet_id": _field(row, "out_wallet_id"),
        "wallet_kind": _field(row, "out_wallet_kind"),
        "config_json": _field(row, "out_wallet_config_json"),
        "raw_json": _field(row, "out_raw_json"),
        "asset": out_asset,
    }
    return {
        "component_id": component_id,
        "lineage_id": _stable_id(
            "direct_swap_payout", str(_field(row, "profile_id")), str(_field(row, "id")), "lineage"
        ),
        "component_type": "swap",
        "conservation_mode": "conversion",
        "conversion_policy": str(_field(row, "policy")),
        "conversion_reviewed": True,
        "legs": [
            _leg(
                out_row,
                leg_id=source_leg_id,
                role="source",
                asset=out_asset,
                amount_msat=physical_source_amount,
                occurred_at=_field(row, "out_occurred_at"),
                valuation_amount=physical_source_amount,
            ),
            {
                **_leg(
                    None,
                    leg_id=target_leg_id,
                    role="retained",
                    asset=payout_asset,
                    amount_msat=physical_payout_amount,
                    occurred_at=_field(row, "payout_occurred_at")
                    or _field(row, "out_occurred_at"),
                    location_ref=f"legacy-payout:{_field(row, 'id')}",
                    valuation_amount=physical_source_amount,
                ),
                "rail": "untracked",
            },
        ],
        "allocations": [
            _allocation(
                allocation_id=_stable_id(component_id, "allocation", "0"),
                source_leg_id=source_leg_id,
                sink_leg_id=target_leg_id,
                source_amount_msat=physical_source_amount,
                sink_amount_msat=physical_payout_amount,
            )
        ],
        "reviewed_source_amount_msat": reviewed_source,
    }


def _retarget_revision_spec(
    spec: Mapping[str, Any],
    *,
    supersedes_component_id: str,
) -> dict[str, Any]:
    """Give a repeated source payload fresh deterministic immutable row ids."""

    component_id = _stable_id(
        "legacy_review_revision",
        str(spec["component_id"]),
        supersedes_component_id,
    )
    leg_ids = {
        str(leg["id"]): _stable_id(component_id, "leg", str(index))
        for index, leg in enumerate(spec["legs"])
    }
    return {
        **spec,
        "component_id": component_id,
        "legs": [
            {**leg, "id": leg_ids[str(leg["id"])]}
            for leg in spec["legs"]
        ],
        "allocations": [
            {
                **allocation,
                "id": _stable_id(component_id, "allocation", str(index)),
                "source_leg_id": leg_ids[str(allocation["source_leg_id"])],
                "sink_leg_id": leg_ids[str(allocation["sink_leg_id"])],
            }
            for index, allocation in enumerate(spec["allocations"])
        ],
        "terms": [
            {
                **term,
                "id": _stable_id(component_id, "term", str(index)),
                "source_leg_id": leg_ids[str(term["source_leg_id"])],
                "target_leg_id": leg_ids[str(term["target_leg_id"])],
            }
            for index, term in enumerate(spec.get("terms", ()))
        ],
    }


def _link_legacy_row(
    conn: sqlite3.Connection,
    *,
    legacy_table: str,
    legacy_source_id: str,
    component_id: str,
) -> None:
    if legacy_table == "transaction_pairs":
        conn.execute(
            "UPDATE transaction_pairs SET component_id = ? WHERE id = ?",
            (component_id, legacy_source_id),
        )
        return
    if legacy_table == "direct_swap_payouts":
        conn.execute(
            "UPDATE direct_swap_payouts SET component_id = ? WHERE id = ?",
            (component_id, legacy_source_id),
        )
        return
    raise AssertionError(f"unsupported legacy custody table: {legacy_table}")


def _insert_terms(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    spec: Mapping[str, Any],
    *,
    term_kind: str,
    source_hash: str,
) -> None:
    seal_component_economic_terms(
        conn,
        spec["component_id"],
        [
            {
                "id": _stable_id(
                    spec["component_id"],
                    "term",
                    term_kind,
                    str(_field(row, "id")),
                ),
                "source_leg_id": spec["legs"][0]["id"],
                "target_leg_id": spec["legs"][1]["id"],
                "term_kind": term_kind,
                "legacy_source_id": str(_field(row, "id")),
                "source_row_hash": source_hash,
                "review_kind": str(_field(row, "kind")),
                "tax_policy": str(_field(row, "policy")),
                "reviewed_source_amount_msat": spec[
                    "reviewed_source_amount_msat"
                ],
                "swap_fee_msat": _field(row, "swap_fee_msat"),
                "swap_fee_kind": _field(row, "swap_fee_kind"),
                "confidence_at_review": _field(row, "confidence_at_pair"),
                "review_source": _field(row, "pair_source"),
                "review_notes": _field(row, "notes"),
                "payout_asset": _field(row, "payout_asset"),
                "payout_amount_msat": (
                    None
                    if _field(row, "payout_amount") is None
                    else int(_field(row, "payout_amount"))
                ),
                "payout_occurred_at": _field(row, "payout_occurred_at"),
                "payout_fiat_value_exact": _canonical_float(
                    _field(row, "payout_fiat_value")
                ),
                "payout_external_id": _field(row, "payout_external_id"),
                "counterparty": _field(row, "counterparty"),
            }
        ],
    )


def _pair_rows(
    conn: sqlite3.Connection,
    *,
    include_deleted: bool = False,
) -> list[sqlite3.Row]:
    deleted_filter = "" if include_deleted else "WHERE p.deleted_at IS NULL"
    return conn.execute(
        f"""
        SELECT p.*, 'transaction_pairs' AS legacy_table,
               out_tx.asset AS out_asset, out_tx.amount AS out_tx_amount,
               out_tx.fee AS out_fee,
               out_tx.amount_includes_fee AS out_amount_includes_fee,
               out_tx.direction AS out_direction,
               out_tx.occurred_at AS out_occurred_at,
               out_tx.raw_json AS out_raw_json,
               out_tx.wallet_id AS out_wallet_id,
               out_wallet.kind AS out_wallet_kind,
               out_wallet.config_json AS out_wallet_config_json,
               in_tx.asset AS in_asset, in_tx.amount AS in_tx_amount,
               in_tx.fee AS in_fee,
               in_tx.amount_includes_fee AS in_amount_includes_fee,
               in_tx.direction AS in_direction,
               in_tx.occurred_at AS in_occurred_at,
               in_tx.raw_json AS in_raw_json,
               in_tx.wallet_id AS in_wallet_id,
               in_wallet.kind AS in_wallet_kind,
               in_wallet.config_json AS in_wallet_config_json
        FROM transaction_pairs p
        JOIN transactions out_tx ON out_tx.id = p.out_transaction_id
        JOIN wallets out_wallet ON out_wallet.id = out_tx.wallet_id
        JOIN transactions in_tx ON in_tx.id = p.in_transaction_id
        JOIN wallets in_wallet ON in_wallet.id = in_tx.wallet_id
        {deleted_filter}
        ORDER BY p.profile_id, p.created_at, p.id
        """
    ).fetchall()


def _payout_rows(
    conn: sqlite3.Connection,
    *,
    include_deleted: bool = False,
) -> list[sqlite3.Row]:
    deleted_filter = "" if include_deleted else "WHERE p.deleted_at IS NULL"
    return conn.execute(
        f"""
        SELECT p.*, 'direct_swap_payouts' AS legacy_table,
               out_tx.asset AS out_asset, out_tx.amount AS out_tx_amount,
               out_tx.fee AS out_fee,
               out_tx.amount_includes_fee AS out_amount_includes_fee,
               out_tx.direction AS out_direction,
               out_tx.occurred_at AS out_occurred_at,
               out_tx.raw_json AS out_raw_json,
               out_tx.wallet_id AS out_wallet_id,
               out_wallet.kind AS out_wallet_kind,
               out_wallet.config_json AS out_wallet_config_json
        FROM direct_swap_payouts p
        JOIN transactions out_tx ON out_tx.id = p.out_transaction_id
        JOIN wallets out_wallet ON out_wallet.id = out_tx.wallet_id
        {deleted_filter}
        ORDER BY p.profile_id, p.created_at, p.id
        """
    ).fetchall()


def _connected_pair_groups(rows: Sequence[Mapping[str, Any]]) -> list[list[Mapping[str, Any]]]:
    by_transaction: dict[str, set[int]] = {}
    for index, row in enumerate(rows):
        for field in ("out_transaction_id", "in_transaction_id"):
            by_transaction.setdefault(str(_field(row, field)), set()).add(index)
    remaining = set(range(len(rows)))
    groups: list[list[Mapping[str, Any]]] = []
    while remaining:
        pending = [min(remaining)]
        indexes: set[int] = set()
        while pending:
            index = pending.pop()
            if index in indexes:
                continue
            indexes.add(index)
            remaining.discard(index)
            row = rows[index]
            neighbors: set[int] = set()
            for field in ("out_transaction_id", "in_transaction_id"):
                neighbors.update(by_transaction[str(_field(row, field))])
            pending.extend(sorted(neighbors - indexes, reverse=True))
        groups.append([rows[index] for index in sorted(indexes)])
    return groups


def _pair_group_spec(
    rows: Sequence[Mapping[str, Any]],
    *,
    require_full_source: bool = True,
) -> dict[str, Any]:
    ordered = sorted(
        rows,
        key=lambda row: (str(_field(row, "created_at")), str(_field(row, "id"))),
    )
    identities = [str(_field(row, "id")) for row in ordered]
    hashes = [
        _hash_payload(_canonical_payload(row, _PAIR_HASH_FIELDS)) for row in ordered
    ]
    profile_id = str(_field(ordered[0], "profile_id"))
    component_id = _stable_id("transaction_pair_group", profile_id, *hashes)
    conversion = any(
        normalize_asset_code(_field(row, "out_asset"))
        != normalize_asset_code(_field(row, "in_asset"))
        or str(_field(row, "policy")) != "carrying-value"
        for row in ordered
    )
    if conversion and len(ordered) != 1:
        raise AppError(
            "a connected reviewed conversion group is ambiguous",
            code="custody_legacy_group_ambiguous",
            details={"pair_ids": identities},
            retryable=False,
        )
    source_used: dict[str, int] = {}
    source_totals: dict[str, int] = {}
    target_used: dict[str, int] = {}
    legs: list[dict[str, Any]] = []
    allocations: list[dict[str, Any]] = []
    terms: list[dict[str, Any]] = []
    for ordinal, (row, source_hash) in enumerate(zip(ordered, hashes)):
        source_id = str(_field(row, "out_transaction_id"))
        target_id = str(_field(row, "in_transaction_id"))
        source_principal = row_boundary_amounts(
            {
                "direction": _field(row, "out_direction"),
                "amount": _field(row, "out_tx_amount"),
                "fee": _field(row, "out_fee"),
                "amount_includes_fee": _field(row, "out_amount_includes_fee"),
            }
        ).principal_msat
        target_principal = row_boundary_amounts(
            {
                "direction": _field(row, "in_direction"),
                "amount": _field(row, "in_tx_amount"),
                "fee": _field(row, "in_fee"),
                "amount_includes_fee": _field(row, "in_amount_includes_fee"),
            }
        ).principal_msat
        available_source = source_principal - source_used.get(source_id, 0)
        source_totals[source_id] = source_principal
        available_target = target_principal - target_used.get(target_id, 0)
        explicit = _field(row, "out_amount")
        requested = available_source if explicit in (None, "") else int(explicit)
        if requested > available_source:
            raise AppError(
                "a reviewed pair exceeds its available source quantity",
                code="custody_legacy_group_invalid",
                details={
                    "pair_id": _field(row, "id"),
                    "requested_msat": requested,
                    "available_msat": available_source,
                },
                retryable=False,
            )
        if conversion:
            source_amount = min(requested, available_source)
            target_amount = available_target
        else:
            # A same-asset row reviews only the exact common slice. Consuming
            # the whole source on the first edge would make an incrementally
            # authored 1:N group impossible to complete and could activate a
            # partial source as if its residual had been classified.
            source_amount = min(requested, available_source, available_target)
            target_amount = source_amount
        if source_amount <= 0 or target_amount <= 0:
            raise AppError(
                "a reviewed pair has no remaining positive component slice",
                code="custody_legacy_group_invalid",
                details={"pair_id": _field(row, "id")},
                retryable=False,
            )
        source_used[source_id] = source_used.get(source_id, 0) + source_amount
        target_used[target_id] = target_used.get(target_id, 0) + target_amount
        source_leg_id = _stable_id(component_id, "source", str(ordinal))
        target_leg_id = _stable_id(component_id, "target", str(ordinal))
        source_row = {
            "transaction_id": source_id,
            "wallet_id": _field(row, "out_wallet_id"),
            "wallet_kind": _field(row, "out_wallet_kind"),
            "config_json": _field(row, "out_wallet_config_json"),
            "raw_json": _field(row, "out_raw_json"),
            "asset": _field(row, "out_asset"),
        }
        target_row = {
            "transaction_id": target_id,
            "wallet_id": _field(row, "in_wallet_id"),
            "wallet_kind": _field(row, "in_wallet_kind"),
            "config_json": _field(row, "in_wallet_config_json"),
            "raw_json": _field(row, "in_raw_json"),
            "asset": _field(row, "in_asset"),
        }
        legs.extend(
            (
                _leg(
                    source_row,
                    leg_id=source_leg_id,
                    role="source",
                    asset=str(_field(row, "out_asset")),
                    amount_msat=source_amount,
                    occurred_at=_field(row, "out_occurred_at"),
                    valuation_amount=source_amount if conversion else None,
                ),
                _leg(
                    target_row,
                    leg_id=target_leg_id,
                    role="destination",
                    asset=str(_field(row, "in_asset")),
                    amount_msat=target_amount,
                    occurred_at=_field(row, "in_occurred_at"),
                    valuation_amount=source_amount if conversion else None,
                ),
            )
        )
        allocations.append(
            _allocation(
                allocation_id=_stable_id(component_id, "allocation", str(ordinal)),
                source_leg_id=source_leg_id,
                sink_leg_id=target_leg_id,
                source_amount_msat=source_amount,
                sink_amount_msat=target_amount,
            )
        )
        terms.append(
            {
                "id": _stable_id(component_id, "term", str(ordinal)),
                "source_leg_id": source_leg_id,
                "target_leg_id": target_leg_id,
                "term_kind": "transaction_pair",
                "legacy_source_id": str(_field(row, "id")),
                "source_row_hash": source_hash,
                "review_kind": str(_field(row, "kind")),
                "tax_policy": str(_field(row, "policy")),
                "reviewed_source_amount_msat": source_amount,
                "swap_fee_msat": _field(row, "swap_fee_msat"),
                "swap_fee_kind": _field(row, "swap_fee_kind"),
                "confidence_at_review": _field(row, "confidence_at_pair"),
                "review_source": _field(row, "pair_source"),
                "review_notes": _field(row, "notes"),
            }
        )
    partial_sources = {
        source_id: {
            "reviewed_msat": source_used.get(source_id, 0),
            "principal_msat": principal_msat,
        }
        for source_id, principal_msat in source_totals.items()
        if source_used.get(source_id, 0) != principal_msat
    }
    spec = {
        "component_id": component_id,
        "lineage_id": _stable_id("transaction_pair_group", profile_id, *identities),
        "component_type": "swap" if conversion else "manual_bridge",
        "conservation_mode": "conversion" if conversion else "quantity",
        "conversion_policy": (
            str(_field(ordered[0], "policy")) if conversion else None
        ),
        "conversion_reviewed": conversion,
        "legs": legs,
        "allocations": allocations,
        "terms": terms,
        "created_at": min(str(_field(row, "created_at")) for row in ordered),
    }
    if partial_sources and require_full_source:
        rows_by_source: dict[str, list[Mapping[str, Any]]] = {}
        source_leg_ids: dict[str, str] = {}
        for row, leg in zip(ordered, legs[::2]):
            source_id = str(_field(row, "out_transaction_id"))
            rows_by_source.setdefault(source_id, []).append(row)
            source_leg_ids.setdefault(source_id, str(leg["id"]))
        for source_id, amounts in sorted(partial_sources.items()):
            source_rows = rows_by_source[source_id]
            _append_source_residual(
                spec,
                source_rows[0],
                reviewed_source_msat=int(amounts["reviewed_msat"]),
                classification=_pair_residual_classification(source_rows[0]),
                source_leg_id=source_leg_ids[source_id],
            )
    return spec


def _migrate_legacy_pair_groups(
    conn: sqlite3.Connection,
) -> ConsolidationResult:
    """Activate one atomic component for each connected active pair group."""

    rows = _pair_rows(conn)
    activated = 0
    unchanged = 0
    skipped = 0
    with _savepoint(conn, "custody_pair_consolidation"):
        for group in _connected_pair_groups(rows):
            try:
                outcome = _consolidate_pair_group(conn, group)
            except AppError as error:
                _record_migration_issue(conn, group, error)
                skipped += 1
                continue
            _resolve_migration_issues(conn, group)
            if outcome == "unchanged":
                unchanged += 1
            else:
                activated += 1
    return ConsolidationResult(
        activated=activated, unchanged=unchanged, skipped=skipped
    )


def _migrate_legacy_payouts(
    conn: sqlite3.Connection,
) -> ConsolidationResult:
    """Create and activate each legacy payout atomically in one pass."""

    activated = 0
    unchanged = 0
    skipped = 0
    with _savepoint(conn, "custody_payout_consolidation"):
        for row in _payout_rows(conn):
            try:
                linked_id = str(_field(row, "component_id") or "")
                if linked_id and get_component(conn, linked_id)["effective_state"] == "active":
                    unchanged += 1
                    continue
                with _savepoint(conn, f"custody_payout_{uuid.uuid4().hex}"):
                    source_hash = _hash_payload(
                        _canonical_payload(row, _PAYOUT_HASH_FIELDS)
                    )
                    spec = _payout_spec(row, source_hash)
                    reviewed = int(spec["legs"][0]["amount_msat"])
                    if not _append_exact_native_residual(
                        conn, spec, row, reviewed_source_msat=reviewed
                    ):
                        _append_source_residual(
                            spec,
                            row,
                            reviewed_source_msat=reviewed,
                            classification="suspense_continuation",
                        )
                    if linked_id:
                        spec = _retarget_revision_spec(
                            spec, supersedes_component_id=linked_id
                        )
                        supersede_component(
                            conn,
                            linked_id,
                            reason="replace staged legacy direct payout",
                        )
                    migrated = _activate_native_review(
                        conn,
                        row,
                        spec,
                        term_kind="direct_swap_payout",
                        source_hash=source_hash,
                        authored_source="migration",
                        change_reason="migrate legacy direct payout",
                        evidence_kind="legacy_review_migration",
                        evidence={
                            "legacy_table": "direct_swap_payouts",
                            "legacy_source_id": str(_field(row, "id")),
                        },
                    )
                    _link_legacy_row(
                        conn,
                        legacy_table="direct_swap_payouts",
                        legacy_source_id=str(_field(row, "id")),
                        component_id=str(migrated["component_id"]),
                    )
            except AppError as error:
                _record_migration_issue(conn, [row], error)
                skipped += 1
                continue
            _resolve_migration_issues(conn, [row])
            activated += 1
    return ConsolidationResult(
        activated=activated, unchanged=unchanged, skipped=skipped
    )


def _consolidate_pair_group(
    conn: sqlite3.Connection,
    group: Sequence[Mapping[str, Any]],
) -> str:
    with _savepoint(conn, f"custody_pair_group_{uuid.uuid4().hex}"):
        spec = _pair_group_spec(group)
        linked = {
            str(_field(row, "component_id"))
            for row in group
            if _field(row, "component_id") not in (None, "")
        }
        if len(linked) == 1:
            current = get_component(conn, next(iter(linked)))
            if current["effective_state"] == "active" and len(
                current["economic_terms"]
            ) == len(group):
                return "unchanged"
        exact = conn.execute(
            "SELECT id FROM custody_components WHERE id = ?",
            (spec["component_id"],),
        ).fetchone()
        if exact is not None:
            exact_component = get_component(conn, str(exact["id"]))
            if exact_component["effective_state"] == "active" and len(
                exact_component["economic_terms"]
            ) == len(group):
                for row in group:
                    _link_legacy_row(
                        conn,
                        legacy_table="transaction_pairs",
                        legacy_source_id=str(_field(row, "id")),
                        component_id=str(exact["id"]),
                    )
                return "activated"
            linked.add(str(exact["id"]))
            spec = _retarget_revision_spec(
                spec, supersedes_component_id=str(exact["id"])
            )
        for old_id in sorted(linked):
            supersede_component(
                conn, old_id, reason="consolidated into authored pair group"
            )
        source_hash = _hash_payload(
            _canonical_payload(group[0], _PAIR_HASH_FIELDS)
        )
        migrated = _activate_native_review(
            conn,
            group[0],
            spec,
            term_kind="transaction_pair",
            source_hash=source_hash,
            authored_source="migration",
            change_reason="consolidate legacy reviewed pair group",
            evidence_kind="legacy_review_migration",
            evidence={
                "legacy_table": "transaction_pairs",
                "legacy_source_ids": sorted(
                    str(_field(row, "id")) for row in group
                ),
            },
        )
        for row in group:
            _link_legacy_row(
                conn,
                legacy_table="transaction_pairs",
                legacy_source_id=str(_field(row, "id")),
                component_id=str(migrated["component_id"]),
            )
    return "activated"


def _migrate_deleted_legacy_rows(conn: sqlite3.Connection) -> ConsolidationResult:
    migrated = 0
    unchanged = 0
    skipped = 0
    for rows, term_kind, fields, builder in (
        (
            _pair_rows(conn, include_deleted=True),
            "transaction_pair",
            _PAIR_HASH_FIELDS,
            _pair_spec,
        ),
        (
            _payout_rows(conn, include_deleted=True),
            "direct_swap_payout",
            _PAYOUT_HASH_FIELDS,
            _payout_spec,
        ),
    ):
        for row in rows:
            if _field(row, "deleted_at") in (None, ""):
                continue
            linked_id = str(_field(row, "component_id") or "")
            if linked_id:
                try:
                    if get_component(conn, linked_id)["state"] == "superseded":
                        unchanged += 1
                        continue
                except AppError:
                    pass
            try:
                with _savepoint(conn, f"custody_deleted_{uuid.uuid4().hex}"):
                    source_hash = _hash_payload(_canonical_payload(row, fields))
                    spec = builder(row, source_hash)
                    if linked_id:
                        spec = _retarget_revision_spec(
                            spec, supersedes_component_id=linked_id
                        )
                        supersede_component(
                            conn,
                            linked_id,
                            reason="replace staged deleted legacy review",
                        )
                    migrated_review = _activate_native_review(
                        conn,
                        row,
                        spec,
                        term_kind=term_kind,
                        source_hash=source_hash,
                        authored_source="migration",
                        change_reason="migrate deleted legacy custody review",
                        evidence_kind="legacy_review_migration",
                        evidence={
                            "legacy_table": str(_field(row, "legacy_table")),
                            "legacy_source_id": str(_field(row, "id")),
                        },
                    )
                    component_id = str(migrated_review["component_id"])
                    supersede_component(
                        conn,
                        component_id,
                        reason="migrate deleted legacy custody review",
                        superseded_at=str(_field(row, "deleted_at")),
                    )
                    _link_legacy_row(
                        conn,
                        legacy_table=str(_field(row, "legacy_table")),
                        legacy_source_id=str(_field(row, "id")),
                        component_id=component_id,
                    )
            except AppError as error:
                _record_migration_issue(conn, [row], error)
                skipped += 1
                continue
            _resolve_migration_issues(conn, [row])
            migrated += 1
    return ConsolidationResult(
        activated=migrated,
        unchanged=unchanged,
        skipped=skipped,
    )


def refresh_legacy_authored_components(
    conn: sqlite3.Connection,
) -> ConsolidationResult:
    """Migrate pending legacy rows atomically without a draft staging phase."""

    pending = int(
        conn.execute(
            """
            SELECT
              (SELECT COUNT(*) FROM transaction_pairs
               WHERE component_id IS NULL)
              +
              (SELECT COUNT(*) FROM direct_swap_payouts
               WHERE component_id IS NULL)
              +
              (SELECT COUNT(*) FROM custody_authored_migration_issues
               WHERE resolved_at IS NULL)
            """
        ).fetchone()[0]
        or 0
    )
    if pending == 0:
        return ConsolidationResult(unchanged=1)

    with _savepoint(conn, "custody_authored_migration"):
        deleted = _migrate_deleted_legacy_rows(conn)
        pairs = _migrate_legacy_pair_groups(conn)
        payouts = _migrate_legacy_payouts(conn)
    return ConsolidationResult(
        activated=deleted.activated + pairs.activated + payouts.activated,
        unchanged=deleted.unchanged + pairs.unchanged + payouts.unchanged,
        skipped=deleted.skipped + pairs.skipped + payouts.skipped,
    )


def load_migration_quarantines(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
) -> tuple[dict[str, Any], ...]:
    """Materialize unresolved legacy upgrade failures as fail-closed holds."""

    quarantines: list[dict[str, Any]] = []
    for row in conn.execute(
        "SELECT * FROM custody_authored_migration_issues "
        "WHERE profile_id = ? AND resolved_at IS NULL "
        "ORDER BY created_at, id",
        (profile_id,),
    ).fetchall():
        try:
            transaction_ids = json.loads(row["transaction_ids_json"] or "[]")
        except (TypeError, ValueError, json.JSONDecodeError):
            transaction_ids = []
        detail = {
            "migration_issue_id": row["id"],
            "legacy_table": row["legacy_table"],
            "legacy_source_id": row["legacy_source_id"],
            "issue_code": row["issue_code"],
        }
        try:
            stored_details = json.loads(row["details_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            stored_details = {}
        if isinstance(stored_details, Mapping):
            detail["migration_details"] = dict(stored_details)
        for transaction_id in transaction_ids:
            quarantines.append(
                {
                    "transaction_id": str(transaction_id),
                    "workspace_id": row["workspace_id"],
                    "profile_id": row["profile_id"],
                    "reason": "custody_authored_migration_incomplete",
                    "detail_json": json.dumps(detail, sort_keys=True),
                }
            )
    return tuple(quarantines)


def effective_component_ids(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
) -> set[str]:
    """Return component ids that own at least one effective membership."""

    return {
        str(row["component_id"])
        for row in conn.execute(
            "SELECT DISTINCT component_id "
            "FROM custody_component_transaction_memberships "
            "WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }


def list_active_review_refs(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
) -> list[Mapping[str, Any]]:
    """Return matcher-shaped refs from components plus bounded exceptions."""

    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    component_rows: list[Mapping[str, Any]] = []
    if effective_ids:
        placeholders = ",".join("?" for _ in effective_ids)
        component_rows = [
            {**dict(row), "compatibility": False}
            for row in conn.execute(
                f"""
                SELECT term.legacy_source_id AS id,
                       term.component_id,
                       term.term_kind,
                       COALESCE(source.anchor_transaction_id,
                                source.transaction_id) AS out_transaction_id,
                       CASE WHEN term.term_kind = 'transaction_pair'
                            THEN COALESCE(target.anchor_transaction_id,
                                          target.transaction_id)
                            ELSE NULL END AS in_transaction_id,
                       term.review_kind AS kind,
                       term.tax_policy AS policy,
                       NULL AS deleted_at
                FROM custody_component_economic_terms term
                JOIN custody_component_legs source
                  ON source.id = term.source_leg_id
                JOIN custody_component_legs target
                  ON target.id = term.target_leg_id
                WHERE term.profile_id = ?
                  AND term.component_id IN ({placeholders})
                ORDER BY term.created_at, term.component_id, term.ordinal
                """,
                (profile_id, *sorted(effective_ids)),
            ).fetchall()
        ]
        claimed_transaction_ids = {
            str(transaction_id)
            for row in component_rows
            for transaction_id in (
                _field(row, "out_transaction_id"),
                _field(row, "in_transaction_id"),
            )
            if transaction_id not in (None, "")
        }
        component_rows.extend(
            {
                "id": str(row["component_id"]),
                "component_id": str(row["component_id"]),
                "out_transaction_id": (
                    str(row["transaction_id"])
                    if row["role"] == "source"
                    else None
                ),
                "in_transaction_id": (
                    str(row["transaction_id"])
                    if row["role"] != "source"
                    else None
                ),
                "kind": "custody-component",
                "policy": None,
                "term_kind": "custody_component",
                "deleted_at": None,
                "compatibility": False,
            }
            for row in conn.execute(
                f"""
                SELECT component_id, role,
                       COALESCE(transaction_id,
                                anchor_transaction_id) AS transaction_id
                FROM custody_component_legs
                WHERE profile_id = ?
                  AND component_id IN ({placeholders})
                  AND COALESCE(transaction_id, anchor_transaction_id) IS NOT NULL
                ORDER BY component_id, ordinal, id
                """,
                (profile_id, *sorted(effective_ids)),
            ).fetchall()
            if str(row["transaction_id"]) not in claimed_transaction_ids
        )
    rows = component_rows
    transaction_ids = sorted(
        {
            str(transaction_id)
            for row in rows
            for transaction_id in (
                _field(row, "out_transaction_id"),
                _field(row, "in_transaction_id"),
            )
            if transaction_id not in (None, "")
        }
    )
    assets_by_transaction: dict[str, str] = {}
    if transaction_ids:
        placeholders = ",".join("?" for _ in transaction_ids)
        assets_by_transaction = {
            str(row["id"]): str(row["asset"])
            for row in conn.execute(
                f"SELECT id, asset FROM transactions WHERE id IN ({placeholders})",
                transaction_ids,
            ).fetchall()
        }
    return [
        {
            **row,
            "out_asset": assets_by_transaction.get(
                str(_field(row, "out_transaction_id") or "")
            ),
            "in_asset": assets_by_transaction.get(
                str(_field(row, "in_transaction_id") or "")
            ),
        }
        for row in rows
    ]


def find_active_review_for_transaction(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    transaction_id: str,
) -> Mapping[str, Any] | None:
    """Find the authored component or bounded exception owning a boundary."""

    component = conn.execute(
        """
        SELECT component.id AS component_id,
               term.legacy_source_id AS review_id,
               term.term_kind
        FROM custody_component_legs leg
        JOIN custody_components component ON component.id = leg.component_id
        LEFT JOIN custody_component_economic_terms term
          ON term.component_id = component.id
         AND (term.source_leg_id = leg.id OR term.target_leg_id = leg.id)
        WHERE component.profile_id = ?
          AND component.state = 'active'
          AND COALESCE(leg.anchor_transaction_id, leg.transaction_id) = ?
        ORDER BY component.created_at, component.id, term.ordinal
        LIMIT 1
        """,
        (profile_id, transaction_id),
    ).fetchone()
    if component is not None:
        return {
            "id": component["review_id"] or component["component_id"],
            "component_id": component["component_id"],
            "term_kind": component["term_kind"],
            "compatibility": False,
        }

    return None


def _review_transaction_contexts(
    conn: sqlite3.Connection,
    transaction_ids: Sequence[str],
) -> dict[str, dict[str, Any]]:
    if not transaction_ids:
        return {}
    placeholders = ",".join("?" for _ in transaction_ids)
    return {
        str(row["id"]): dict(row)
        for row in conn.execute(
            f"""
            SELECT transaction_row.id,
                   transaction_row.external_id,
                   transaction_row.asset,
                   transaction_row.amount,
                   transaction_row.occurred_at,
                   wallet.label AS wallet,
                   wallet.kind AS wallet_kind
            FROM transactions transaction_row
            JOIN wallets wallet ON wallet.id = transaction_row.wallet_id
            WHERE transaction_row.id IN ({placeholders})
            """,
            tuple(transaction_ids),
        ).fetchall()
    }


def _current_component_term_rows(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    term_kind: str,
    include_deleted: bool = False,
) -> list[dict[str, Any]]:
    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    conditions: list[str] = []
    params: list[Any] = [profile_id, term_kind]
    if effective_ids:
        placeholders = ",".join("?" for _ in effective_ids)
        conditions.append(f"term.component_id IN ({placeholders})")
        params.extend(sorted(effective_ids))
    if include_deleted:
        conditions.append(
            "(component.state = 'superseded' AND NOT EXISTS ("
            "SELECT 1 FROM custody_components successor "
            "WHERE successor.supersedes_component_id = component.id))"
        )
    if not conditions:
        return []
    return [
        dict(row)
        for row in conn.execute(
            f"""
            SELECT term.*,
                   component.state AS component_state,
                   component.superseded_at,
                   COALESCE(source.anchor_transaction_id,
                            source.transaction_id) AS out_transaction_id,
                   COALESCE(target.anchor_transaction_id,
                            target.transaction_id) AS in_transaction_id
            FROM custody_component_economic_terms term
            JOIN custody_components component ON component.id = term.component_id
            JOIN custody_component_legs source ON source.id = term.source_leg_id
            JOIN custody_component_legs target ON target.id = term.target_leg_id
            WHERE term.profile_id = ?
              AND term.term_kind = ?
              AND ({' OR '.join(conditions)})
            ORDER BY term.created_at DESC, term.component_id, term.ordinal
            """,
            tuple(params),
        ).fetchall()
    ]


def list_pair_review_records(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    include_deleted: bool = False,
) -> list[dict[str, Any]]:
    """Materialize current pair list rows from authored component terms."""
    component_rows = _current_component_term_rows(
        conn,
        profile_id=profile_id,
        term_kind="transaction_pair",
        include_deleted=include_deleted,
    )
    transaction_ids = sorted(
        {
            str(transaction_id)
            for row in component_rows
            for transaction_id in (
                _field(row, "out_transaction_id"),
                _field(row, "in_transaction_id"),
            )
            if transaction_id not in (None, "")
        }
    )
    contexts = _review_transaction_contexts(conn, transaction_ids)
    records: list[dict[str, Any]] = []
    for row in component_rows:
        out_id = str(row["out_transaction_id"])
        in_id = str(row["in_transaction_id"])
        out_context = contexts[out_id]
        in_context = contexts[in_id]
        reviewed_out = row["reviewed_source_amount_msat"]
        records.append(
            {
                "id": row["legacy_source_id"],
                "component_id": row["component_id"],
                "workspace_id": row["workspace_id"],
                "profile_id": row["profile_id"],
                "out_transaction_id": out_id,
                "in_transaction_id": in_id,
                "kind": row["review_kind"],
                "policy": row["tax_policy"],
                "notes": row["review_notes"],
                "swap_fee_msat": row["swap_fee_msat"],
                "swap_fee_kind": row["swap_fee_kind"],
                "confidence_at_pair": row["confidence_at_review"],
                "pair_source": row["review_source"],
                "out_amount": reviewed_out,
                "deleted_at": row.get("superseded_at"),
                "created_at": row["created_at"],
                "out_external_id": out_context["external_id"],
                "out_asset": out_context["asset"],
                "out_amount_msat": (
                    int(reviewed_out)
                    if reviewed_out is not None
                    else int(out_context["amount"])
                ),
                "out_full_amount_msat": int(out_context["amount"]),
                "out_occurred_at": out_context["occurred_at"],
                "out_wallet": out_context["wallet"],
                "out_wallet_kind": out_context["wallet_kind"],
                "in_external_id": in_context["external_id"],
                "in_asset": in_context["asset"],
                "in_amount_msat": int(in_context["amount"]),
                "in_occurred_at": in_context["occurred_at"],
                "in_wallet": in_context["wallet"],
                "in_wallet_kind": in_context["wallet_kind"],
            }
        )
    current_by_review_id = {str(row["id"]): row for row in records}
    return sorted(
        current_by_review_id.values(),
        key=lambda row: (row["created_at"], row["id"]),
        reverse=True,
    )


def list_payout_review_records(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    include_deleted: bool = False,
) -> list[dict[str, Any]]:
    """Materialize current payout list rows from authored component terms."""
    component_rows = _current_component_term_rows(
        conn,
        profile_id=profile_id,
        term_kind="direct_swap_payout",
        include_deleted=include_deleted,
    )
    transaction_ids = sorted(
        {
            str(row["out_transaction_id"])
            for row in component_rows
        }
    )
    contexts = _review_transaction_contexts(conn, transaction_ids)
    records: list[dict[str, Any]] = []
    for row in component_rows:
        out_id = str(row["out_transaction_id"])
        out_context = contexts[out_id]
        reviewed_out = row["reviewed_source_amount_msat"]
        records.append(
            {
                "id": row["legacy_source_id"],
                "component_id": row["component_id"],
                "workspace_id": row["workspace_id"],
                "profile_id": row["profile_id"],
                "out_transaction_id": out_id,
                "kind": row["review_kind"],
                "policy": row["tax_policy"],
                "payout_asset": row["payout_asset"],
                "payout_amount": row["payout_amount_msat"],
                "payout_occurred_at": row["payout_occurred_at"],
                "payout_fiat_value": row["payout_fiat_value_exact"],
                "payout_external_id": row["payout_external_id"],
                "counterparty": row["counterparty"],
                "notes": row["review_notes"],
                "swap_fee_msat": row["swap_fee_msat"],
                "swap_fee_kind": row["swap_fee_kind"],
                "out_amount": reviewed_out,
                "deleted_at": row.get("superseded_at"),
                "created_at": row["created_at"],
                "out_external_id": out_context["external_id"],
                "out_asset": out_context["asset"],
                "reviewed_out_amount_msat": (
                    int(reviewed_out)
                    if reviewed_out is not None
                    else int(out_context["amount"])
                ),
                "full_out_amount_msat": int(out_context["amount"]),
                "out_occurred_at": out_context["occurred_at"],
                "out_wallet": out_context["wallet"],
            }
        )
    current_by_review_id = {str(row["id"]): row for row in records}
    return sorted(
        current_by_review_id.values(),
        key=lambda row: (row["created_at"], row["id"]),
        reverse=True,
    )


def _active_review_component(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    review_id: str,
    term_kind: str,
) -> Mapping[str, Any] | None:
    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    if not effective_ids:
        return None
    placeholders = ",".join("?" for _ in effective_ids)
    return conn.execute(
        f"""
        SELECT component.id AS component_id, component.lineage_id
        FROM custody_component_economic_terms term
        JOIN custody_components component ON component.id = term.component_id
        WHERE term.profile_id = ? AND term.legacy_source_id = ?
          AND term.term_kind = ?
          AND term.component_id IN ({placeholders})
        ORDER BY component.revision DESC, component.created_at DESC
        LIMIT 1
        """,
        (profile_id, review_id, term_kind, *sorted(effective_ids)),
    ).fetchone()


def _review_boundary_row(
    conn: sqlite3.Connection,
    base: Mapping[str, Any],
    *,
    include_target: bool,
) -> dict[str, Any]:
    target_select = """
        , in_tx.asset AS in_asset, in_tx.amount AS in_tx_amount,
          in_tx.fee AS in_fee,
          in_tx.amount_includes_fee AS in_amount_includes_fee,
          in_tx.direction AS in_direction,
          in_tx.occurred_at AS in_occurred_at,
          in_tx.raw_json AS in_raw_json,
          in_tx.wallet_id AS in_wallet_id,
          in_wallet.kind AS in_wallet_kind,
          in_wallet.config_json AS in_wallet_config_json
    """ if include_target else ""
    target_join = """
        JOIN transactions in_tx ON in_tx.id = ?
        JOIN wallets in_wallet ON in_wallet.id = in_tx.wallet_id
    """ if include_target else ""
    params = [base["out_transaction_id"]]
    if include_target:
        params.append(base["in_transaction_id"])
    row = conn.execute(
        f"""
        SELECT out_tx.asset AS out_asset, out_tx.amount AS out_tx_amount,
               out_tx.fee AS out_fee,
               out_tx.external_id AS out_external_id,
               out_tx.amount_includes_fee AS out_amount_includes_fee,
               out_tx.direction AS out_direction,
               out_tx.occurred_at AS out_occurred_at,
               out_tx.raw_json AS out_raw_json,
               out_tx.wallet_id AS out_wallet_id,
               out_wallet.kind AS out_wallet_kind,
               out_wallet.config_json AS out_wallet_config_json
               {target_select}
        FROM transactions out_tx
        JOIN wallets out_wallet ON out_wallet.id = out_tx.wallet_id
        {target_join}
        WHERE out_tx.id = ?
        """,
        (*params[1:], params[0]),
    ).fetchone()
    if row is None:
        raise AppError("Custody review boundary was not found", code="not_found")
    return {**dict(base), **dict(row)}


def _append_source_residual(
    spec: dict[str, Any],
    row: Mapping[str, Any],
    *,
    reviewed_source_msat: int,
    classification: str,
    source_leg_id: str | None = None,
) -> None:
    if spec.get("conservation_mode") == "conversion":
        # Conversion residuals are handled by the exact-native helper or fall
        # through to the ordinary external-presumed interpretation. Adding a
        # quantity-suspense leg would conflate unlike assets.
        return
    boundary = row_boundary_amounts(
        {
            "direction": _field(row, "out_direction"),
            "amount": _field(row, "out_tx_amount"),
            "fee": _field(row, "out_fee"),
            "amount_includes_fee": _field(row, "out_amount_includes_fee"),
        }
    )
    residual = boundary.principal_msat - reviewed_source_msat
    if residual <= 0:
        return
    # An imported miner fee is authoritative physical evidence, not custody
    # suspense.  Historical pair matching commonly represented a destination
    # shortfall equal to that fee, so preserve it as an explicit fee sink.  A
    # larger or fee-less shortfall remains unresolved custody until reviewed.
    residual_parts = [(classification, residual)]
    if (
        classification == "suspense_continuation"
        and 0 < boundary.fee_msat <= residual
    ):
        residual_parts = [("network_fee", boundary.fee_msat)]
        if residual > boundary.fee_msat:
            residual_parts.append(
                ("suspense_continuation", residual - boundary.fee_msat)
            )
    component_id = str(spec["component_id"])
    source = next(
        (
            leg
            for leg in spec["legs"]
            if source_leg_id is None or str(leg["id"]) == source_leg_id
        ),
        None,
    )
    if source is None or source.get("role") != "source":
        raise AppError(
            "Custody residual source leg was not found",
            code="custody_component_validation",
            retryable=False,
        )
    source_id = str(source["id"])
    source["amount_msat"] = int(source["amount_msat"]) + residual
    for residual_classification, residual_amount in residual_parts:
        sink_id = _stable_id(
            component_id,
            "leg",
            "residual-sink",
            source_id,
            residual_classification,
        )
        sink = {
            **source,
            "id": sink_id,
            "amount_msat": residual_amount,
            "valuation_unit": None,
            "valuation_amount": None,
            "role": (
                "fee" if residual_classification == "network_fee" else "suspense"
            ),
            "transaction_id": None,
            "anchor_transaction_id": None,
            "location_ref": (
                f"network-fee:{row['id']}"
                if residual_classification == "network_fee"
                else f"reviewed-custody-suspense:{row['id']}"
            ),
            "notes": f"reviewed_residual:{residual_classification}",
        }
        sink["wallet_id"] = None
        spec["legs"].append(sink)
        spec["allocations"].append(
            _allocation(
                allocation_id=_stable_id(
                    component_id,
                    "allocation",
                    "residual",
                    source_id,
                    residual_classification,
                ),
                source_leg_id=source_id,
                sink_leg_id=sink_id,
                source_amount_msat=residual_amount,
                sink_amount_msat=residual_amount,
            )
        )


def _append_exact_native_residual(
    conn: sqlite3.Connection,
    spec: dict[str, Any],
    row: Mapping[str, Any],
    *,
    reviewed_source_msat: int,
) -> bool:
    """Bind a reviewed split remainder only when one exact native return exists."""

    if spec.get("conservation_mode") != "conversion":
        return False
    boundary = row_boundary_amounts(
        {
            "direction": _field(row, "out_direction"),
            "amount": _field(row, "out_tx_amount"),
            "fee": _field(row, "out_fee"),
            "amount_includes_fee": _field(row, "out_amount_includes_fee"),
        }
    )
    residual = boundary.principal_msat - reviewed_source_msat
    external_id = str(_field(row, "out_external_id") or "").strip()
    if residual <= 0 or not external_id:
        return False
    targets = conn.execute(
        """
        SELECT t.id AS transaction_id, t.wallet_id, t.asset, t.occurred_at,
               t.raw_json, w.kind AS wallet_kind, w.config_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.excluded = 0
          AND t.direction = 'inbound' AND upper(t.asset) = upper(?)
          AND lower(t.external_id) = lower(?) AND t.amount = ?
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        LIMIT 2
        """,
        (
            _field(row, "profile_id"),
            _field(row, "out_asset"),
            external_id,
            residual,
        ),
    ).fetchall()
    if len(targets) != 1:
        return False
    source = next(
        (leg for leg in spec["legs"] if leg.get("role") == "source"),
        None,
    )
    if source is None:
        return False
    source["amount_msat"] = int(source["amount_msat"]) + residual
    source["valuation_amount"] = int(source.get("valuation_amount") or 0) + residual
    component_id = str(spec["component_id"])
    sink_id = _stable_id(component_id, "leg", "exact-native-residual")
    target = dict(targets[0])
    sink = _leg(
        target,
        leg_id=sink_id,
        role="retained",
        asset=str(target["asset"]),
        amount_msat=residual,
        occurred_at=target["occurred_at"],
        valuation_amount=residual,
    )
    spec["legs"].append(sink)
    spec["allocations"].append(
        _allocation(
            allocation_id=_stable_id(
                component_id, "allocation", "exact-native-residual"
            ),
            source_leg_id=str(source["id"]),
            sink_leg_id=sink_id,
            source_amount_msat=residual,
            sink_amount_msat=residual,
        )
    )
    return True


def _activate_native_review(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    spec: dict[str, Any],
    *,
    term_kind: str,
    source_hash: str,
    authored_source: str,
    change_reason: str | None = None,
    evidence_kind: str = "reviewed_custody",
    evidence: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    prior = conn.execute(
        "SELECT id FROM custody_components "
        "WHERE profile_id = ? AND lineage_id = ? AND state = 'active' "
        "ORDER BY revision DESC LIMIT 1",
        (row["profile_id"], spec["lineage_id"]),
    ).fetchone()
    if prior is not None and conn.execute(
        "SELECT 1 FROM custody_components WHERE id = ?", (spec["component_id"],)
    ).fetchone() is not None:
        spec = _retarget_revision_spec(
            spec, supersedes_component_id=str(prior["id"])
        )
    component_kwargs = {
        "component_type": str(spec["component_type"]),
        "conservation_mode": str(spec["conservation_mode"]),
        "conversion_policy": spec.get("conversion_policy"),
        "conversion_reviewed": bool(spec.get("conversion_reviewed")),
        "legs": spec["legs"],
        "allocations": spec["allocations"],
        "evidence_kind": evidence_kind,
        "evidence_grade": "reviewed",
        "evidence": (
            dict(evidence)
            if evidence is not None
            else {"review_id": row["id"], "term_kind": term_kind}
        ),
        "notes": _field(row, "notes"),
        "authored_source": authored_source,
        "created_at": str(row["created_at"]),
    }
    if prior is None:
        component = create_component(
            conn,
            workspace_id=str(row["workspace_id"]),
            profile_id=str(row["profile_id"]),
            component_id=str(spec["component_id"]),
            lineage_id=str(spec["lineage_id"]),
            **component_kwargs,
        )
    else:
        component = update_component(
            conn,
            str(prior["id"]),
            new_component_id=str(spec["component_id"]),
            preserve_planned_row_ids=True,
            change_reason=change_reason or f"{term_kind} review revised",
            **component_kwargs,
        )
    if spec.get("terms"):
        seal_component_economic_terms(conn, component["id"], spec["terms"])
    else:
        _insert_terms(conn, row, spec, term_kind=term_kind, source_hash=source_hash)
    activate_component(conn, component["id"], activated_at=str(row["created_at"]))
    return {**dict(row), "component_id": component["id"]}


def create_pair_review_component(
    conn: sqlite3.Connection,
    *,
    review_id: str,
    workspace_id: str,
    profile_id: str,
    out_transaction_id: str,
    in_transaction_id: str,
    kind: str,
    policy: str,
    notes: str | None,
    swap_fee_msat: int | None,
    swap_fee_kind: str | None,
    confidence_at_pair: str | None,
    pair_source: str,
    out_amount_msat: int | None,
    created_at: str,
    authored_source: str,
) -> Mapping[str, Any]:
    """Author a pair directly as one immutable active component."""

    base = {
        "id": review_id, "workspace_id": workspace_id, "profile_id": profile_id,
        "out_transaction_id": out_transaction_id,
        "in_transaction_id": in_transaction_id, "kind": kind, "policy": policy,
        "notes": notes, "swap_fee_msat": swap_fee_msat,
        "swap_fee_kind": swap_fee_kind, "confidence_at_pair": confidence_at_pair,
        "pair_source": pair_source, "out_amount": out_amount_msat,
        "deleted_at": None, "created_at": created_at,
    }
    row = _review_boundary_row(conn, base, include_target=True)
    source_hash = _hash_payload(_canonical_payload(row, _PAIR_HASH_FIELDS))
    related = [
        record
        for record in list_pair_review_records(conn, profile_id=profile_id)
        if (
            record["out_transaction_id"] == out_transaction_id
            or record["in_transaction_id"] == in_transaction_id
        )
    ]
    if related:
        component_ids = {
            str(record.get("component_id") or "") for record in related
        }
        if len(component_ids) != 1 or "" in component_ids:
            raise AppError(
                "Connected reviewed pair terms do not have one active component",
                code="custody_component_membership_conflict",
            )
        active_component = get_component(conn, component_ids.pop())
        group_rows = [
            _review_boundary_row(conn, record, include_target=True)
            for record in related
        ] + [row]
        spec = _pair_group_spec(group_rows)
        spec["lineage_id"] = active_component["lineage_id"]
    else:
        spec = _pair_spec(row, source_hash)
    reviewed = int(spec["legs"][0]["amount_msat"])
    if not related:
        if not _append_exact_native_residual(
            conn, spec, row, reviewed_source_msat=reviewed
        ):
            _append_source_residual(
                spec,
                row,
                reviewed_source_msat=reviewed,
                classification=_pair_residual_classification(row),
            )
    return _activate_native_review(
        conn,
        row,
        spec,
        term_kind="transaction_pair",
        source_hash=source_hash,
        authored_source=authored_source,
    )


def create_payout_review_component(
    conn: sqlite3.Connection,
    *,
    review_id: str,
    workspace_id: str,
    profile_id: str,
    out_transaction_id: str,
    kind: str,
    policy: str,
    payout_asset: str,
    payout_amount_msat: int,
    payout_occurred_at: str | None,
    payout_fiat_value: float | None,
    payout_external_id: str | None,
    counterparty: str | None,
    notes: str | None,
    swap_fee_msat: int | None,
    swap_fee_kind: str | None,
    out_amount_msat: int | None,
    created_at: str,
    authored_source: str,
) -> Mapping[str, Any]:
    """Author a direct payout directly as one immutable active component."""

    base = {
        "id": review_id, "workspace_id": workspace_id, "profile_id": profile_id,
        "out_transaction_id": out_transaction_id, "kind": kind, "policy": policy,
        "payout_asset": payout_asset, "payout_amount": payout_amount_msat,
        "payout_occurred_at": payout_occurred_at,
        "payout_fiat_value": payout_fiat_value,
        "payout_external_id": payout_external_id, "counterparty": counterparty,
        "notes": notes, "swap_fee_msat": swap_fee_msat,
        "swap_fee_kind": swap_fee_kind, "out_amount": out_amount_msat,
        "deleted_at": None, "created_at": created_at,
    }
    row = _review_boundary_row(conn, base, include_target=False)
    source_hash = _hash_payload(_canonical_payload(row, _PAYOUT_HASH_FIELDS))
    spec = _payout_spec(row, source_hash)
    reviewed = int(spec["legs"][0]["amount_msat"])
    if not _append_exact_native_residual(
        conn, spec, row, reviewed_source_msat=reviewed
    ):
        _append_source_residual(
            spec,
            row,
            reviewed_source_msat=reviewed,
            classification="suspense_continuation",
        )
    return _activate_native_review(
        conn,
        row,
        spec,
        term_kind="direct_swap_payout",
        source_hash=source_hash,
        authored_source=authored_source,
    )


def revise_pair_review_component(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    *,
    kind: str,
    policy: str,
    notes: str | None,
    swap_fee_msat: int | None,
    swap_fee_kind: str | None,
    authored_source: str,
) -> Mapping[str, Any]:
    """Append an immutable pair component revision."""

    review_id = str(_field(row, "id"))
    active = _active_review_component(
        conn,
        profile_id=str(row["profile_id"]),
        review_id=review_id,
        term_kind="transaction_pair",
    )
    if active is None:
        raise AppError("Transaction pair review was not found", code="not_found")
    revised = {
        **dict(row),
        "kind": kind,
        "policy": policy,
        "notes": notes,
        "swap_fee_msat": swap_fee_msat,
        "swap_fee_kind": swap_fee_kind,
        "created_at": now_iso(),
    }
    boundary = _review_boundary_row(conn, revised, include_target=True)
    source_hash = _hash_payload(_canonical_payload(boundary, _PAIR_HASH_FIELDS))
    component_records = [
        record
        for record in list_pair_review_records(
            conn, profile_id=str(row["profile_id"])
        )
        if str(record.get("component_id") or "") == str(active["component_id"])
    ]
    if len(component_records) > 1:
        group_rows = [
            boundary
            if str(record["id"]) == review_id
            else _review_boundary_row(conn, record, include_target=True)
            for record in component_records
        ]
        spec = _pair_group_spec(group_rows)
    else:
        spec = _pair_spec(boundary, source_hash)
        reviewed = int(spec["legs"][0]["amount_msat"])
        if not _append_exact_native_residual(
            conn, spec, boundary, reviewed_source_msat=reviewed
        ):
            _append_source_residual(
                spec,
                boundary,
                reviewed_source_msat=reviewed,
                classification=_pair_residual_classification(boundary),
            )
    spec["lineage_id"] = str(active["lineage_id"])
    return _activate_native_review(
        conn,
        boundary,
        spec,
        term_kind="transaction_pair",
        source_hash=source_hash,
        authored_source=authored_source,
    )


def delete_authored_review(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    review_id: str,
    term_kind: str,
    deleted_at: str,
    authored_source: str,
) -> None:
    active = _active_review_component(
        conn,
        profile_id=profile_id,
        review_id=review_id,
        term_kind=term_kind,
    )
    if active is None:
        raise AppError("Custody review was not found", code="not_found")
    if term_kind == "transaction_pair":
        component_records = [
            record
            for record in list_pair_review_records(conn, profile_id=profile_id)
            if str(record.get("component_id") or "") == str(active["component_id"])
            and str(record["id"]) != review_id
        ]
        if component_records:
            rows = [
                _review_boundary_row(conn, record, include_target=True)
                for record in component_records
            ]
            spec = (
                _pair_group_spec(rows, require_full_source=False)
                if len(rows) > 1
                else _pair_spec(
                    rows[0],
                    _hash_payload(_canonical_payload(rows[0], _PAIR_HASH_FIELDS)),
                )
            )
            spec["lineage_id"] = str(active["lineage_id"])
            activation_row = {**rows[0], "created_at": deleted_at}
            _activate_native_review(
                conn,
                activation_row,
                spec,
                term_kind="transaction_pair",
                source_hash=_hash_payload(
                    _canonical_payload(rows[0], _PAIR_HASH_FIELDS)
                ),
                authored_source=authored_source,
                change_reason=(
                    f"transaction_pair review {review_id} deleted"
                ),
            )
            return
    supersede_component(
        conn,
        str(active["component_id"]),
        reason=f"{term_kind} review deleted by {authored_source}",
        superseded_at=deleted_at,
    )


def authored_review_exists(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    review_id: str,
    term_kind: str,
) -> bool:
    """Return whether immutable authored history contains the review id."""

    return conn.execute(
        "SELECT 1 FROM custody_component_economic_terms "
        "WHERE profile_id = ? AND legacy_source_id = ? AND term_kind = ? LIMIT 1",
        (profile_id, review_id, term_kind),
    ).fetchone() is not None
