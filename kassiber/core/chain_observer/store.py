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
OBSERVER_COVERAGE_VERSION = 1
PRIVATE_OBSERVER_TABLES = frozenset(
    {"chain_observer_instances", "chain_observer_coverage"}
)


@dataclass(frozen=True, slots=True)
class CoveragePoint:
    branch_key: str
    scanned_to: int
    highest_used: int | None = None
    details: Mapping[str, Any] | None = None


@dataclass(frozen=True, slots=True)
class StoredObserverState:
    identity: ObserverIdentity
    payload: Mapping[str, Any]
    coverage: tuple[CoveragePoint, ...]


def _rebuild_required(
    identity: ObserverIdentity,
    *,
    stored_version: Any,
    representation: str,
) -> AppError:
    supported = (
        OBSERVER_STATE_VERSION
        if representation == "state"
        else OBSERVER_COVERAGE_VERSION
    )
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
    )


def persist_observer_state(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
    payload: Mapping[str, Any],
    coverage: Sequence[CoveragePoint],
) -> None:
    """Persist one applied state without starting or committing a transaction."""

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
        coverage_json[point.branch_key] = encode_json_payload(
            dict(point.details or {})
        )
    timestamp = now_iso()
    conn.execute(
        """
        INSERT INTO chain_observer_instances(
            id, workspace_id, profile_id, logical_wallet_id, source_wallet_id,
            source_key, observer_kind, chain, network, state_version,
            state_json, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            state_json = excluded.state_json,
            updated_at = excluded.updated_at
        """,
        (
            identity.id,
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
            timestamp,
        ),
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
    "PRIVATE_OBSERVER_TABLES",
    "CoveragePoint",
    "StoredObserverState",
    "delete_profile_observer_state",
    "delete_wallet_observer_state",
    "encode_json_payload",
    "load_observer_state",
    "persist_observer_state",
]
