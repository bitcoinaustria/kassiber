"""Versioned observer persistence inside Kassiber's main database transaction."""

from __future__ import annotations

import json
import math
import sqlite3
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ...errors import AppError
from ...time_utils import now_iso
from .identity import ObserverIdentity


OBSERVER_STATE_VERSION = 1
OBSERVER_COVERAGE_VERSION = 2
PRIVATE_OBSERVER_TABLES = frozenset(
    {
        "chain_observer_instances",
        "chain_observer_coverage",
        "chain_observer_values",
        "chain_observation_provenance",
    }
)
OBSERVER_VALUES_NAMESPACE_VERSION = 1


@dataclass(frozen=True, slots=True)
class CoveragePoint:
    """One observer branch's exact derivation-index coverage.

    ``scanned_to`` is an exclusive boundary: a value of ``20`` means that
    indices ``0`` through ``19`` were included in the successful observation.
    It is a technical scan fact for one imported source, not an attestation
    that every wallet or policy belonging to the profile is known.
    """

    branch_key: str
    scanned_to: int
    highest_used: int | None = None
    details: Mapping[str, Any] | None = None


@dataclass(frozen=True, slots=True)
class StoredObserverState:
    identity: ObserverIdentity
    payload: Mapping[str, Any]
    coverage: tuple[CoveragePoint, ...]
    epoch: int = 0


_UNCONDITIONAL_EPOCH = object()


def _stale_observer_state(
    identity: ObserverIdentity,
    *,
    expected_epoch: int | None,
) -> AppError:
    return AppError(
        "Chain observer state changed after this refresh was prepared",
        code="observer_state_stale",
        hint="Retry the wallet refresh against the latest observer state.",
        details={
            "observer_id": identity.id,
            "expected_epoch": expected_epoch,
        },
        retryable=True,
    )


def _rebuild_required(
    identity: ObserverIdentity,
    *,
    stored_version: Any,
    representation: str,
) -> AppError:
    supported = {
        "state": OBSERVER_STATE_VERSION,
        "coverage": OBSERVER_COVERAGE_VERSION,
        "opaque_values": OBSERVER_VALUES_NAMESPACE_VERSION,
    }.get(representation)
    return AppError(
        "Stored chain observer state must be rebuilt before this wallet can refresh",
        code="observer_state_rebuild_required",
        hint="Run a full wallet refresh to rebuild derived observer state.",
        details={
            "observer_id": identity.id,
            "representation": representation,
            "stored_version": stored_version,
            "supported_version": supported,
        },
        retryable=False,
    )


def _json_value(value: Any, *, path: str = "$") -> Any:
    """Return a plain JSON value or reject object/byte serialization."""

    if value is None or isinstance(value, (str, bool, int)):
        return value
    if isinstance(value, float):
        if not math.isfinite(value):
            raise AppError(
                "Observer state contains a non-finite number",
                code="observer_state_invalid",
                details={"path": path},
                retryable=False,
            )
        return value
    if isinstance(value, Mapping):
        normalized: dict[str, Any] = {}
        for key, item in value.items():
            if not isinstance(key, str):
                raise AppError(
                    "Observer state object keys must be strings",
                    code="observer_state_invalid",
                    details={"path": path},
                    retryable=False,
                )
            normalized[key] = _json_value(item, path=f"{path}.{key}")
        return normalized
    if isinstance(value, (list, tuple)):
        return [
            _json_value(item, path=f"{path}[{index}]")
            for index, item in enumerate(value)
        ]
    raise AppError(
        "Observer state must use an explicit JSON representation",
        code="observer_state_invalid",
        hint="Encode dependency state into versioned JSON primitives; pickle and Python objects are not supported.",
        details={"path": path, "type": type(value).__name__},
        retryable=False,
    )


def encode_json_payload(value: Mapping[str, Any]) -> str:
    normalized = _json_value(value)
    if not isinstance(normalized, dict):
        raise AppError(
            "Observer state payload must be an object",
            code="observer_state_invalid",
            retryable=False,
        )
    return json.dumps(normalized, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def _decode_object(
    raw: Any,
    identity: ObserverIdentity,
    *,
    representation: str,
) -> dict[str, Any]:
    try:
        parsed = json.loads(str(raw or "{}"))
    except (TypeError, ValueError, json.JSONDecodeError) as exc:
        raise _rebuild_required(
            identity,
            stored_version="invalid_json",
            representation=representation,
        ) from exc
    if not isinstance(parsed, dict):
        raise _rebuild_required(
            identity,
            stored_version="invalid_shape",
            representation=representation,
        )
    return parsed


def load_observer_state(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
) -> StoredObserverState | None:
    row = conn.execute(
        "SELECT * FROM chain_observer_instances WHERE id = ?",
        (identity.id,),
    ).fetchone()
    if row is None:
        return None
    state_version = int(row["state_version"])
    if state_version != OBSERVER_STATE_VERSION:
        raise _rebuild_required(
            identity,
            stored_version=state_version,
            representation="state",
        )
    if (
        row["observer_kind"] != identity.observer_kind
        or row["chain"] != identity.chain
        or row["network"] != identity.network
        or row["logical_wallet_id"] != identity.logical_wallet_id
        or row["source_wallet_id"] != identity.source_wallet_id
        or row["source_key"] != identity.source_key
    ):
        raise _rebuild_required(
            identity,
            stored_version="identity_mismatch",
            representation="state",
        )
    payload = _decode_object(row["state_json"], identity, representation="state")
    coverage_rows = conn.execute(
        """
        SELECT * FROM chain_observer_coverage
        WHERE observer_id = ?
        ORDER BY branch_key
        """,
        (identity.id,),
    ).fetchall()
    coverage: list[CoveragePoint] = []
    for coverage_row in coverage_rows:
        version = int(coverage_row["coverage_version"])
        if version != OBSERVER_COVERAGE_VERSION:
            raise _rebuild_required(
                identity,
                stored_version=version,
                representation="coverage",
            )
        coverage.append(
            CoveragePoint(
                branch_key=str(coverage_row["branch_key"]),
                highest_used=(
                    int(coverage_row["highest_used"])
                    if coverage_row["highest_used"] is not None
                    else None
                ),
                scanned_to=int(coverage_row["scanned_to"]),
                details=_decode_object(
                    coverage_row["details_json"],
                    identity,
                    representation="coverage",
                ),
            )
        )
    return StoredObserverState(
        identity=identity,
        payload=payload,
        coverage=tuple(coverage),
        epoch=int(row["state_epoch"]),
    )


def observer_state_epoch(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
) -> int | None:
    """Return the raw persistence epoch without decoding incompatible state."""

    row = conn.execute(
        "SELECT state_epoch FROM chain_observer_instances WHERE id = ?",
        (identity.id,),
    ).fetchone()
    return int(row["state_epoch"]) if row is not None else None


def persist_observer_state(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
    payload: Mapping[str, Any],
    coverage: Sequence[CoveragePoint],
    *,
    expected_epoch: int | None | object = _UNCONDITIONAL_EPOCH,
) -> None:
    """Persist one applied state without starting or committing a transaction.

    Contract callers pass the epoch captured during preparation.  The state
    row is then the compare-and-swap fence for both JSON and opaque dependency
    state written later in the same wallet savepoint.  Direct maintenance
    callers may omit the epoch and retain unconditional replacement semantics.
    """

    if not conn.in_transaction:
        raise AppError(
            "Observer state application requires the caller's wallet transaction",
            code="observer_apply_outside_transaction",
            retryable=False,
        )
    state_json = encode_json_payload(payload)
    normalized_coverage = tuple(sorted(coverage, key=lambda point: point.branch_key))
    seen_branches: set[str] = set()
    coverage_json: dict[str, str] = {}
    for point in normalized_coverage:
        if point.branch_key in seen_branches:
            raise AppError(
                "Observer coverage contains a duplicate branch",
                code="observer_state_invalid",
                details={"branch_key": point.branch_key},
                retryable=False,
            )
        seen_branches.add(point.branch_key)
        if point.branch_key not in identity.branch_keys:
            raise AppError(
                "Observer coverage names an unknown branch",
                code="observer_state_invalid",
                details={"branch_key": point.branch_key},
                retryable=False,
            )
        if point.scanned_to < 0 or (
            point.highest_used is not None and point.highest_used < 0
        ):
            raise AppError(
                "Observer coverage indices must be non-negative",
                code="observer_state_invalid",
                details={"branch_key": point.branch_key},
                retryable=False,
            )
        if (
            point.highest_used is not None
            and point.highest_used >= point.scanned_to
        ):
            raise AppError(
                "Observer coverage cannot contain a used index outside its exclusive scan boundary",
                code="observer_state_invalid",
                details={
                    "branch_key": point.branch_key,
                    "highest_used": point.highest_used,
                    "scanned_to": point.scanned_to,
                },
                retryable=False,
            )
        coverage_json[point.branch_key] = encode_json_payload(
            dict(point.details or {})
        )
    timestamp = now_iso()
    values = (
        identity.workspace_id,
        identity.profile_id,
        identity.logical_wallet_id,
        identity.source_wallet_id,
        identity.source_key,
        identity.observer_kind,
        identity.chain,
        identity.network,
        OBSERVER_STATE_VERSION,
        state_json,
        timestamp,
    )
    if expected_epoch is _UNCONDITIONAL_EPOCH:
        conn.execute(
            """
            INSERT INTO chain_observer_instances(
                id, workspace_id, profile_id, logical_wallet_id, source_wallet_id,
                source_key, observer_kind, chain, network, state_version,
                state_epoch, state_json, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                workspace_id = excluded.workspace_id,
                profile_id = excluded.profile_id,
                logical_wallet_id = excluded.logical_wallet_id,
                source_wallet_id = excluded.source_wallet_id,
                source_key = excluded.source_key,
                observer_kind = excluded.observer_kind,
                chain = excluded.chain,
                network = excluded.network,
                state_version = excluded.state_version,
                state_epoch = chain_observer_instances.state_epoch + 1,
                state_json = excluded.state_json,
                updated_at = excluded.updated_at
            """,
            (identity.id, *values, timestamp),
        )
    elif expected_epoch is None:
        cursor = conn.execute(
            """
            INSERT INTO chain_observer_instances(
                id, workspace_id, profile_id, logical_wallet_id, source_wallet_id,
                source_key, observer_kind, chain, network, state_version,
                state_epoch, state_json, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)
            ON CONFLICT(id) DO NOTHING
            """,
            (identity.id, *values, timestamp),
        )
        if cursor.rowcount != 1:
            raise _stale_observer_state(identity, expected_epoch=None)
    else:
        cursor = conn.execute(
            """
            UPDATE chain_observer_instances
            SET workspace_id = ?, profile_id = ?, logical_wallet_id = ?,
                source_wallet_id = ?, source_key = ?, observer_kind = ?,
                chain = ?, network = ?, state_version = ?, state_json = ?,
                state_epoch = state_epoch + 1, updated_at = ?
            WHERE id = ? AND state_epoch = ?
            """,
            (*values, identity.id, int(expected_epoch)),
        )
        if cursor.rowcount != 1:
            raise _stale_observer_state(
                identity,
                expected_epoch=int(expected_epoch),
            )
    conn.execute(
        "DELETE FROM chain_observer_coverage WHERE observer_id = ?",
        (identity.id,),
    )
    for point in normalized_coverage:
        conn.execute(
            """
            INSERT INTO chain_observer_coverage(
                observer_id, branch_key, coverage_version, highest_used,
                scanned_to, details_json, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                identity.id,
                point.branch_key,
                OBSERVER_COVERAGE_VERSION,
                point.highest_used,
                point.scanned_to,
                coverage_json[point.branch_key],
                timestamp,
            ),
        )


def load_observer_values(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
) -> dict[str, bytes]:
    """Load opaque dependency values without exposing them through JSON state."""

    rows = conn.execute(
        """
        SELECT namespace_version, key, value
        FROM chain_observer_values
        WHERE observer_id = ?
        ORDER BY key
        """,
        (identity.id,),
    ).fetchall()
    values: dict[str, bytes] = {}
    for row in rows:
        version = int(row["namespace_version"])
        if version != OBSERVER_VALUES_NAMESPACE_VERSION:
            raise _rebuild_required(
                identity,
                stored_version=version,
                representation="opaque_values",
            )
        values[str(row["key"])] = bytes(row["value"])
    return values


def persist_observer_values(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
    values: Mapping[str, bytes],
) -> None:
    """Replace opaque values inside the caller-owned wallet savepoint."""

    if not conn.in_transaction:
        raise AppError(
            "Observer values require the caller's wallet transaction",
            code="observer_apply_outside_transaction",
            retryable=False,
        )
    normalized: list[tuple[str, bytes]] = []
    for key, value in values.items():
        if not isinstance(key, str) or not key:
            raise AppError(
                "Observer value keys must be non-empty strings",
                code="observer_state_invalid",
                retryable=False,
            )
        if not isinstance(value, (bytes, bytearray, memoryview)):
            raise AppError(
                "Observer values must remain opaque bytes",
                code="observer_state_invalid",
                details={"key": key},
                retryable=False,
            )
        normalized.append((key, bytes(value)))
    conn.execute("DELETE FROM chain_observer_values WHERE observer_id = ?", (identity.id,))
    timestamp = now_iso()
    conn.executemany(
        """
        INSERT INTO chain_observer_values(
            observer_id, namespace_version, key, value, updated_at
        ) VALUES(?, ?, ?, ?, ?)
        """,
        [
            (identity.id, OBSERVER_VALUES_NAMESPACE_VERSION, key, value, timestamp)
            for key, value in sorted(normalized)
        ],
    )


def delete_wallet_observer_state(
    conn: sqlite3.Connection,
    wallet_id: str,
) -> int:
    cursor = conn.execute(
        """
        DELETE FROM chain_observer_instances
        WHERE logical_wallet_id = ? OR source_wallet_id = ?
        """,
        (wallet_id, wallet_id),
    )
    return max(int(cursor.rowcount or 0), 0)


def delete_profile_observer_state(
    conn: sqlite3.Connection,
    profile_id: str,
) -> int:
    cursor = conn.execute(
        "DELETE FROM chain_observer_instances WHERE profile_id = ?",
        (profile_id,),
    )
    return max(int(cursor.rowcount or 0), 0)


__all__ = [
    "OBSERVER_COVERAGE_VERSION",
    "OBSERVER_STATE_VERSION",
    "OBSERVER_VALUES_NAMESPACE_VERSION",
    "PRIVATE_OBSERVER_TABLES",
    "CoveragePoint",
    "StoredObserverState",
    "delete_profile_observer_state",
    "delete_wallet_observer_state",
    "encode_json_payload",
    "load_observer_state",
    "observer_state_epoch",
    "load_observer_values",
    "persist_observer_state",
    "persist_observer_values",
]
