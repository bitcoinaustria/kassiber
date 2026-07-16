"""Crash-safe compatibility migration into the authored custody aggregate.

Legacy rows are first staged as exact draft revisions, then valid payout
revisions and connected pair groups are activated atomically.  The nullable
``component_id`` on each legacy row is the bounded compatibility link; only an
effective active component replaces that row at the journal boundary.

Every migrated revision carries two kinds of immutable data:

* physical boundary legs and exact source-to-sink allocations; and
* a typed economic-terms row for policy, swap-fee, payout and review metadata
  which cannot coherently be represented as physical quantity legs.

Re-running the migration is a no-op.  Compatibility-row edits after activation
fail closed because reviewed economics must be revised on the component.
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
class MigrationResult:
    created: int = 0
    revised: int = 0
    unchanged: int = 0

    @property
    def changed(self) -> bool:
        return bool(self.created or self.revised)


@dataclass(frozen=True)
class ConsolidationResult:
    activated: int = 0
    unchanged: int = 0
    skipped: int = 0

    @property
    def changed(self) -> bool:
        return bool(self.activated)


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


def _pair_spec(row: Mapping[str, Any], source_hash: str) -> dict[str, Any]:
    out_asset = normalize_asset_code(_field(row, "out_asset"))
    in_asset = normalize_asset_code(_field(row, "in_asset"))
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
    explicit_source = _field(row, "out_amount")
    reviewed_source = (
        source_principal if explicit_source in (None, "") else int(explicit_source)
    )
    conversion = out_asset != in_asset or str(_field(row, "policy")) != "carrying-value"
    source_amount = max(0, reviewed_source)
    target_amount = max(0, target_principal)
    if not conversion:
        # Existing same-asset review semantics carry only the common slice;
        # the remainder keeps its independent classification.
        source_amount = max(0, min(reviewed_source, target_principal))
        target_amount = source_amount
    component_id = _stable_id(
        "transaction_pair", str(_field(row, "profile_id")), str(_field(row, "id")), source_hash
    )
    source_leg_id = _stable_id(component_id, "leg", "source")
    target_leg_id = _stable_id(component_id, "leg", "target")
    source_valuation = source_amount if conversion else None
    target_valuation = source_amount if conversion else None
    out_row = {
        "transaction_id": _field(row, "out_transaction_id"),
        "wallet_id": _field(row, "out_wallet_id"),
        "wallet_kind": _field(row, "out_wallet_kind"),
        "config_json": _field(row, "out_wallet_config_json"),
        "raw_json": _field(row, "out_raw_json"),
        "asset": out_asset,
    }
    in_row = {
        "transaction_id": _field(row, "in_transaction_id"),
        "wallet_id": _field(row, "in_wallet_id"),
        "wallet_kind": _field(row, "in_wallet_kind"),
        "config_json": _field(row, "in_wallet_config_json"),
        "raw_json": _field(row, "in_raw_json"),
        "asset": in_asset,
    }
    return {
        "component_id": component_id,
        "lineage_id": _stable_id(
            "transaction_pair", str(_field(row, "profile_id")), str(_field(row, "id")), "lineage"
        ),
        "component_type": "swap" if conversion else "manual_bridge",
        "conservation_mode": "conversion" if conversion else "quantity",
        "conversion_policy": str(_field(row, "policy")) if conversion else None,
        "conversion_reviewed": conversion,
        "legs": [
            _leg(
                out_row,
                leg_id=source_leg_id,
                role="source",
                asset=out_asset,
                amount_msat=source_amount,
                occurred_at=_field(row, "out_occurred_at"),
                valuation_amount=source_valuation,
            ),
            _leg(
                in_row,
                leg_id=target_leg_id,
                role="destination",
                asset=in_asset,
                amount_msat=target_amount,
                occurred_at=_field(row, "in_occurred_at"),
                valuation_amount=target_valuation,
            ),
        ],
        "allocations": [
            _allocation(
                allocation_id=_stable_id(component_id, "allocation", "0"),
                source_leg_id=source_leg_id,
                sink_leg_id=target_leg_id,
                source_amount_msat=source_amount,
                sink_amount_msat=target_amount,
            )
        ],
        "reviewed_source_amount_msat": reviewed_source,
    }


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


def _component_kwargs(row: Mapping[str, Any], spec: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "component_type": spec["component_type"],
        "conservation_mode": spec["conservation_mode"],
        "conversion_policy": spec["conversion_policy"],
        "conversion_reviewed": spec["conversion_reviewed"],
        "legs": spec["legs"],
        "allocations": spec["allocations"],
        "evidence_kind": "legacy_review_migration",
        "evidence_grade": "reviewed",
        "evidence": {
            "legacy_table": _field(row, "legacy_table"),
            "legacy_source_id": _field(row, "id"),
        },
        "notes": _field(row, "notes"),
        "change_reason": "migrate legacy authored custody review",
        "created_at": _field(row, "created_at"),
        "authored_source": "migration",
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


def _migrate_row(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    *,
    term_kind: str,
    hash_fields: Sequence[str],
    build_spec: Any,
) -> str:
    source_hash = _hash_payload(_canonical_payload(row, hash_fields))
    spec = build_spec(row, source_hash)
    linked_id = _field(row, "component_id")
    if linked_id not in (None, ""):
        existing_term = conn.execute(
            "SELECT source_row_hash FROM custody_component_economic_terms "
            "WHERE component_id = ? AND term_kind = ? "
            "AND legacy_source_id = ? ORDER BY ordinal, id LIMIT 1",
            (linked_id, term_kind, _field(row, "id")),
        ).fetchone()
        if existing_term is not None and existing_term["source_row_hash"] == source_hash:
            return "unchanged"
        linked = get_component(conn, str(linked_id))
        if linked["state"] == "active":
            raise AppError(
                "an activated migrated custody review no longer matches its legacy source",
                code="custody_legacy_review_changed_after_activation",
                hint="Revise the custody component instead of editing compatibility rows.",
                details={"legacy_source_id": _field(row, "id"), "component_id": linked_id},
                retryable=False,
            )
        if existing_term is None and str(linked_id) == spec["component_id"]:
            _insert_terms(
                conn, row, spec, term_kind=term_kind, source_hash=source_hash
            )
            return "revised"
        if conn.execute(
            "SELECT 1 FROM custody_components WHERE id = ?",
            (spec["component_id"],),
        ).fetchone() is not None:
            spec = _retarget_revision_spec(
                spec,
                supersedes_component_id=str(linked_id),
            )
        kwargs = _component_kwargs(row, spec)
        revised = update_component(
            conn,
            str(linked_id),
            new_component_id=spec["component_id"],
            preserve_planned_row_ids=True,
            **kwargs,
        )
        if revised["id"] != spec["component_id"]:
            raise AssertionError("migrated custody revision id changed")
        _insert_terms(conn, row, spec, term_kind=term_kind, source_hash=source_hash)
        _link_legacy_row(
            conn,
            legacy_table=str(_field(row, "legacy_table")),
            legacy_source_id=str(_field(row, "id")),
            component_id=spec["component_id"],
        )
        return "revised"

    kwargs = _component_kwargs(row, spec)
    created = create_component(
        conn,
        workspace_id=str(_field(row, "workspace_id")),
        profile_id=str(_field(row, "profile_id")),
        component_id=spec["component_id"],
        lineage_id=spec["lineage_id"],
        **kwargs,
    )
    if created["id"] != spec["component_id"]:
        raise AssertionError("migrated custody component id changed")
    _insert_terms(conn, row, spec, term_kind=term_kind, source_hash=source_hash)
    _link_legacy_row(
        conn,
        legacy_table=str(_field(row, "legacy_table")),
        legacy_source_id=str(_field(row, "id")),
        component_id=spec["component_id"],
    )
    return "created"


def _pair_rows(conn: sqlite3.Connection) -> list[sqlite3.Row]:
    return conn.execute(
        """
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
        WHERE p.deleted_at IS NULL
        ORDER BY p.profile_id, p.created_at, p.id
        """
    ).fetchall()


def _payout_rows(conn: sqlite3.Connection) -> list[sqlite3.Row]:
    return conn.execute(
        """
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
        WHERE p.deleted_at IS NULL
        ORDER BY p.profile_id, p.created_at, p.id
        """
    ).fetchall()


def backfill_legacy_authored_components(conn: sqlite3.Connection) -> MigrationResult:
    """Create/link draft components for every active legacy authored review."""

    counts = {"created": 0, "revised": 0, "unchanged": 0}
    with _savepoint(conn, "custody_authored_backfill"):
        for rows, term_kind, fields, builder in (
            (_pair_rows(conn), "transaction_pair", _PAIR_HASH_FIELDS, _pair_spec),
            (_payout_rows(conn), "direct_swap_payout", _PAYOUT_HASH_FIELDS, _payout_spec),
        ):
            for row in rows:
                outcome = _migrate_row(
                    conn,
                    row,
                    term_kind=term_kind,
                    hash_fields=fields,
                    build_spec=builder,
                )
                counts[outcome] += 1
    return MigrationResult(**counts)


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


def _allocation_signature(
    legs: Sequence[Mapping[str, Any]],
    allocations: Sequence[Mapping[str, Any]],
) -> tuple[tuple[str, str, int, int], ...]:
    legs_by_id = {str(_field(leg, "id")): leg for leg in legs}

    def transaction_id(leg_id: Any) -> str:
        leg = legs_by_id[str(leg_id)]
        return str(
            _field(leg, "anchor_transaction_id")
            or _field(leg, "transaction_id")
            or ""
        )

    return tuple(
        sorted(
            (
                transaction_id(_field(allocation, "source_leg_id")),
                transaction_id(_field(allocation, "sink_leg_id")),
                int(_field(allocation, "source_amount_msat") or 0),
                int(_field(allocation, "sink_amount_msat") or 0),
            )
            for allocation in allocations
        )
    )


def _pair_group_spec(rows: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
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
                "reviewed_source_amount_msat": requested,
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
    if partial_sources:
        # The legacy interpreter still owns the unreviewed tail. Activating a
        # component before that residual becomes an explicit authored decision
        # would either lose it or falsely upgrade a presumed disposal to a
        # reviewed external allocation.
        raise AppError(
            "a reviewed pair group has an unclassified source residual",
            code="custody_legacy_group_requires_residual",
            details={"sources": partial_sources, "pair_ids": identities},
            retryable=False,
        )
    return {
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


def consolidate_legacy_pair_components(
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
            except AppError:
                # Malformed historical rows remain on the compatibility
                # interpreter, which already emits the precise quarantine.
                # One bad review must not prevent unrelated valid groups from
                # converging during database open.
                skipped += 1
                continue
            if outcome == "unchanged":
                unchanged += 1
            else:
                activated += 1
    return ConsolidationResult(
        activated=activated, unchanged=unchanged, skipped=skipped
    )


def consolidate_legacy_payout_components(
    conn: sqlite3.Connection,
) -> ConsolidationResult:
    """Activate staged one-source direct-payout component aggregates."""

    activated = 0
    unchanged = 0
    skipped = 0
    with _savepoint(conn, "custody_payout_consolidation"):
        for row in _payout_rows(conn):
            component_id = str(_field(row, "component_id") or "")
            if not component_id:
                skipped += 1
                continue
            try:
                component = get_component(conn, component_id)
                if component["effective_state"] == "active":
                    unchanged += 1
                    continue
                source_principal = row_boundary_amounts(
                    {
                        "direction": _field(row, "out_direction"),
                        "amount": _field(row, "out_tx_amount"),
                        "fee": _field(row, "out_fee"),
                        "amount_includes_fee": _field(
                            row, "out_amount_includes_fee"
                        ),
                    }
                ).principal_msat
                reviewed_source = _field(row, "out_amount")
                if reviewed_source not in (None, "") and int(
                    reviewed_source
                ) != source_principal:
                    raise AppError(
                        "a direct payout has an unclassified source residual",
                        code="custody_legacy_group_requires_residual",
                        details={"payout_id": _field(row, "id")},
                        retryable=False,
                    )
                if not component["validation"]["activatable"]:
                    raise AppError(
                        "a migrated direct payout cannot activate",
                        code="custody_legacy_group_invalid",
                        details={
                            "payout_id": _field(row, "id"),
                            "issues": component["validation"]["issues"],
                        },
                        retryable=False,
                    )
                activate_component(conn, component_id)
            except AppError:
                skipped += 1
                continue
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
            if (
                len(group) == 1
                and current["state"] == "draft"
                and current["validation"]["activatable"]
                and _allocation_signature(
                    current["legs"], current["allocations"]
                )
                == _allocation_signature(spec["legs"], spec["allocations"])
            ):
                activate_component(conn, current["id"])
                return "activated"
        existing = conn.execute(
            "SELECT state FROM custody_components WHERE id = ?",
            (spec["component_id"],),
        ).fetchone()
        if existing is None:
            component = create_component(
                conn,
                workspace_id=str(_field(group[0], "workspace_id")),
                profile_id=str(_field(group[0], "profile_id")),
                component_id=spec["component_id"],
                lineage_id=spec["lineage_id"],
                component_type=spec["component_type"],
                conservation_mode=spec["conservation_mode"],
                conversion_policy=spec["conversion_policy"],
                conversion_reviewed=spec["conversion_reviewed"],
                evidence_kind="legacy_review_migration",
                evidence_grade="reviewed",
                evidence={
                    "legacy_table": "transaction_pairs",
                    "legacy_source_ids": sorted(
                        str(_field(row, "id")) for row in group
                    ),
                },
                legs=spec["legs"],
                allocations=spec["allocations"],
                authored_source="migration",
                change_reason="consolidate legacy reviewed pair group",
                created_at=spec["created_at"],
            )
            seal_component_economic_terms(conn, component["id"], spec["terms"])
        component = get_component(conn, spec["component_id"])
        if not component["validation"]["activatable"]:
            raise AppError(
                "a migrated reviewed pair group cannot activate",
                code="custody_legacy_group_invalid",
                details={
                    "pair_ids": sorted(str(_field(row, "id")) for row in group),
                    "issues": component["validation"]["issues"],
                },
                retryable=False,
            )
        for old_id in sorted(linked - {component["id"]}):
            supersede_component(
                conn, old_id, reason="consolidated into authored pair group"
            )
        if component["state"] != "active":
            activate_component(conn, component["id"])
        for row in group:
            _link_legacy_row(
                conn,
                legacy_table="transaction_pairs",
                legacy_source_id=str(_field(row, "id")),
                component_id=component["id"],
            )
    return "activated"


def refresh_legacy_authored_components(
    conn: sqlite3.Connection,
) -> tuple[MigrationResult, ConsolidationResult, ConsolidationResult]:
    """Synchronize the bounded compatibility rows inside the caller's txn."""

    staged = backfill_legacy_authored_components(conn)
    pairs = consolidate_legacy_pair_components(conn)
    payouts = consolidate_legacy_payout_components(conn)
    return staged, pairs, payouts


def load_legacy_compatibility_records(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    effective_component_ids: set[str],
) -> tuple[list[sqlite3.Row], list[sqlite3.Row]]:
    """Load only historical reviews that lack an effective component.

    This is the bounded compatibility exception for partial-source reviews
    whose unreviewed residual cannot be silently reclassified during
    migration. The producer cutover must replace these rows with an explicitly
    planned residual before this loader can be deleted.
    """

    pairs = conn.execute(
        "SELECT * FROM transaction_pairs "
        "WHERE profile_id = ? AND deleted_at IS NULL "
        "ORDER BY created_at, id",
        (profile_id,),
    ).fetchall()
    payouts = conn.execute(
        """
        SELECT p.*, t.asset AS out_asset, t.amount AS out_amount_msat
        FROM direct_swap_payouts p
        JOIN transactions t ON t.id = p.out_transaction_id
        WHERE p.profile_id = ? AND p.deleted_at IS NULL
        ORDER BY p.created_at, p.id
        """,
        (profile_id,),
    ).fetchall()
    return (
        [
            row
            for row in pairs
            if str(_field(row, "component_id") or "")
            not in effective_component_ids
        ],
        [
            row
            for row in payouts
            if str(_field(row, "component_id") or "")
            not in effective_component_ids
        ],
    )


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
    pairs, payouts = load_legacy_compatibility_records(
        conn,
        profile_id=profile_id,
        effective_component_ids=effective_ids,
    )
    compatibility_rows = [
        {**dict(row), "term_kind": "transaction_pair", "compatibility": True}
        for row in pairs
    ] + [
        {
            **dict(row),
            "term_kind": "direct_swap_payout",
            "compatibility": True,
        }
        for row in payouts
    ]
    rows = [*component_rows, *compatibility_rows]
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

    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    pairs, payouts = load_legacy_compatibility_records(
        conn,
        profile_id=profile_id,
        effective_component_ids=effective_ids,
    )
    for term_kind, records in (
        ("transaction_pair", pairs),
        ("direct_swap_payout", payouts),
    ):
        for row in records:
            if transaction_id in {
                str(_field(row, "out_transaction_id") or ""),
                str(_field(row, "in_transaction_id") or ""),
            }:
                return {
                    "id": str(_field(row, "id")),
                    "component_id": _field(row, "component_id"),
                    "term_kind": term_kind,
                    "compatibility": True,
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


def _active_component_term_rows(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    term_kind: str,
) -> list[dict[str, Any]]:
    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    if not effective_ids:
        return []
    placeholders = ",".join("?" for _ in effective_ids)
    return [
        dict(row)
        for row in conn.execute(
            f"""
            SELECT term.*,
                   COALESCE(source.anchor_transaction_id,
                            source.transaction_id) AS out_transaction_id,
                   COALESCE(target.anchor_transaction_id,
                            target.transaction_id) AS in_transaction_id
            FROM custody_component_economic_terms term
            JOIN custody_component_legs source ON source.id = term.source_leg_id
            JOIN custody_component_legs target ON target.id = term.target_leg_id
            WHERE term.profile_id = ?
              AND term.term_kind = ?
              AND term.component_id IN ({placeholders})
            ORDER BY term.created_at DESC, term.component_id, term.ordinal
            """,
            (profile_id, term_kind, *sorted(effective_ids)),
        ).fetchall()
    ]


def list_pair_review_records(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    include_deleted: bool = False,
) -> list[dict[str, Any]]:
    """Materialize pair list rows from effective terms plus exceptions."""

    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    legacy_pairs, _ = load_legacy_compatibility_records(
        conn,
        profile_id=profile_id,
        effective_component_ids=effective_ids,
    )
    compatibility_rows = [dict(row) for row in legacy_pairs]
    if include_deleted:
        compatibility_rows.extend(
            dict(row)
            for row in conn.execute(
                "SELECT * FROM transaction_pairs "
                "WHERE profile_id = ? AND deleted_at IS NOT NULL",
                (profile_id,),
            ).fetchall()
        )
    component_rows = _active_component_term_rows(
        conn,
        profile_id=profile_id,
        term_kind="transaction_pair",
    )
    transaction_ids = sorted(
        {
            str(transaction_id)
            for row in (*component_rows, *compatibility_rows)
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
                "deleted_at": None,
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
    for row in compatibility_rows:
        out_context = contexts[str(row["out_transaction_id"])]
        in_context = contexts[str(row["in_transaction_id"])]
        reviewed_out = row.get("out_amount")
        records.append(
            {
                **row,
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
    return sorted(records, key=lambda row: (row["created_at"], row["id"]), reverse=True)


def list_payout_review_records(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    include_deleted: bool = False,
) -> list[dict[str, Any]]:
    """Materialize payout list rows from effective terms plus exceptions."""

    effective_ids = effective_component_ids(conn, profile_id=profile_id)
    _, legacy_payouts = load_legacy_compatibility_records(
        conn,
        profile_id=profile_id,
        effective_component_ids=effective_ids,
    )
    compatibility_rows = [dict(row) for row in legacy_payouts]
    if include_deleted:
        compatibility_rows.extend(
            dict(row)
            for row in conn.execute(
                "SELECT * FROM direct_swap_payouts "
                "WHERE profile_id = ? AND deleted_at IS NOT NULL",
                (profile_id,),
            ).fetchall()
        )
    component_rows = _active_component_term_rows(
        conn,
        profile_id=profile_id,
        term_kind="direct_swap_payout",
    )
    transaction_ids = sorted(
        {
            str(row["out_transaction_id"])
            for row in (*component_rows, *compatibility_rows)
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
                "deleted_at": None,
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
    for row in compatibility_rows:
        out_context = contexts[str(row["out_transaction_id"])]
        reviewed_out = row.get("out_amount")
        records.append(
            {
                **row,
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
    return sorted(records, key=lambda row: (row["created_at"], row["id"]), reverse=True)


def retire_linked_component(
    conn: sqlite3.Connection,
    component_id: Any,
    *,
    reason: str,
) -> None:
    """Retire an effective compatibility aggregate before changing its row."""

    if component_id in (None, ""):
        return
    component = get_component(conn, str(component_id))
    if component["state"] == "active":
        supersede_component(conn, component["id"], reason=reason)


def create_pair_review_projection(
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
) -> sqlite3.Row:
    """Author a pair and its frozen compatibility projection atomically."""

    with _savepoint(conn, f"custody_pair_create_{uuid.uuid4().hex}"):
        conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id,
                in_transaction_id, kind, policy, notes, swap_fee_msat,
                swap_fee_kind, confidence_at_pair, pair_source, out_amount,
                deleted_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
            """,
            (
                review_id,
                workspace_id,
                profile_id,
                out_transaction_id,
                in_transaction_id,
                kind,
                policy,
                notes,
                swap_fee_msat,
                swap_fee_kind,
                confidence_at_pair,
                pair_source,
                out_amount_msat,
                created_at,
            ),
        )
        refresh_legacy_authored_components(conn)
    return conn.execute(
        "SELECT * FROM transaction_pairs WHERE id = ?", (review_id,)
    ).fetchone()


def create_payout_review_projection(
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
) -> sqlite3.Row:
    """Author a direct payout and its frozen projection atomically."""

    with _savepoint(conn, f"custody_payout_create_{uuid.uuid4().hex}"):
        conn.execute(
            """
            INSERT INTO direct_swap_payouts(
                id, workspace_id, profile_id, out_transaction_id, kind,
                policy, payout_asset, payout_amount, payout_occurred_at,
                payout_fiat_value, payout_external_id, counterparty, notes,
                swap_fee_msat, swap_fee_kind, out_amount, deleted_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
            """,
            (
                review_id,
                workspace_id,
                profile_id,
                out_transaction_id,
                kind,
                policy,
                payout_asset,
                payout_amount_msat,
                payout_occurred_at,
                payout_fiat_value,
                payout_external_id,
                counterparty,
                notes,
                swap_fee_msat,
                swap_fee_kind,
                out_amount_msat,
                created_at,
            ),
        )
        refresh_legacy_authored_components(conn)
    return conn.execute(
        "SELECT * FROM direct_swap_payouts WHERE id = ?", (review_id,)
    ).fetchone()


def revise_pair_review_projection(
    conn: sqlite3.Connection,
    row: Mapping[str, Any],
    *,
    kind: str,
    policy: str,
    notes: str | None,
    swap_fee_msat: int | None,
    swap_fee_kind: str | None,
) -> sqlite3.Row:
    """Append a component revision, then refresh the frozen pair projection."""

    review_id = str(_field(row, "id"))
    with _savepoint(conn, f"custody_pair_revise_{uuid.uuid4().hex}"):
        retire_linked_component(
            conn,
            _field(row, "component_id"),
            reason="transaction pair review revised",
        )
        conn.execute(
            "UPDATE transaction_pairs SET kind = ?, policy = ?, notes = ?, "
            "swap_fee_msat = ?, swap_fee_kind = ? WHERE id = ?",
            (kind, policy, notes, swap_fee_msat, swap_fee_kind, review_id),
        )
        refresh_legacy_authored_components(conn)
    return conn.execute(
        "SELECT * FROM transaction_pairs WHERE id = ?", (review_id,)
    ).fetchone()


def delete_review_projection(
    conn: sqlite3.Connection,
    *,
    table: str,
    row: Mapping[str, Any],
    deleted_at: str,
) -> sqlite3.Row:
    """Retire the authored aggregate before tombstoning its projection."""

    if table not in {"transaction_pairs", "direct_swap_payouts"}:
        raise AssertionError(f"unsupported custody review projection: {table}")
    review_id = str(_field(row, "id"))
    reason = (
        "transaction pair review deleted"
        if table == "transaction_pairs"
        else "direct payout review deleted"
    )
    with _savepoint(conn, f"custody_review_delete_{uuid.uuid4().hex}"):
        retire_linked_component(
            conn,
            _field(row, "component_id"),
            reason=reason,
        )
        conn.execute(
            f"UPDATE {table} SET deleted_at = ? WHERE id = ?",
            (deleted_at, review_id),
        )
    return conn.execute(
        f"SELECT * FROM {table} WHERE id = ?", (review_id,)
    ).fetchone()
