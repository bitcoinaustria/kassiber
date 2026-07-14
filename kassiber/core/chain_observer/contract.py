"""Prepare/apply contract for observation-only chain dependencies."""

from __future__ import annotations

import json
import sqlite3
from dataclasses import dataclass, field
from typing import Any, Mapping, Protocol, Sequence, runtime_checkable

from ...errors import AppError
from .identity import ObserverIdentity
from .store import (
    CoveragePoint,
    StoredObserverState,
    encode_json_payload,
    load_observer_state,
    persist_observer_state,
)


@dataclass(frozen=True, slots=True)
class ObserverPrepareRequest:
    backend_name: str
    backend_kind: str
    force_full: bool = False
    checkpoint: Mapping[str, Any] = field(default_factory=dict)
    options: Mapping[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ChainFacts:
    """Dependency-neutral facts consumed by Kassiber's projection layer."""

    transaction_records: tuple[Mapping[str, Any], ...] = ()
    retracted_external_ids: tuple[str, ...] = ()
    outputs: tuple[Mapping[str, Any], ...] = ()
    coverage: tuple[CoveragePoint, ...] = ()
    freshness_checkpoint: Mapping[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ObserverApplication:
    state: Mapping[str, Any]
    facts: ChainFacts


@runtime_checkable
class ChainObserver(Protocol):
    """One request-scoped dependency wrapper.

    Implementations keep any dependency wallet/builder object private. The
    values crossing this protocol are explicit JSON primitives and normalized
    chain facts only; signing and transaction-building objects have no place in
    this interface.
    """

    def prepare(
        self,
        request: ObserverPrepareRequest,
        prior_state: StoredObserverState | None,
    ) -> Mapping[str, Any]: ...

    def apply(
        self,
        prepared_update: Mapping[str, Any],
        prior_state: StoredObserverState | None,
    ) -> ObserverApplication: ...

    def discard(self) -> None: ...


@dataclass(slots=True)
class PreparedObserverUpdate:
    """Unapplied, JSON-safe update returned by the network phase."""

    identity: ObserverIdentity
    update: Mapping[str, Any]
    prior_state: StoredObserverState | None
    _observer: ChainObserver = field(repr=False)
    applied: bool = False
    discarded: bool = False


def _plain_object(value: Mapping[str, Any]) -> dict[str, Any]:
    # Reuse the persistence validator so arbitrary Python objects and bytes
    # cannot cross the observer boundary even before anything is written.
    return json.loads(encode_json_payload(value))


def _plain_objects(
    values: Sequence[Mapping[str, Any]],
    *,
    field_name: str,
) -> tuple[Mapping[str, Any], ...]:
    normalized: list[Mapping[str, Any]] = []
    for index, value in enumerate(values):
        if not isinstance(value, Mapping):
            raise AppError(
                "Observer facts must use explicit objects",
                code="observer_state_invalid",
                details={"field": field_name, "index": index},
                retryable=False,
            )
        normalized.append(_plain_object(value))
    return tuple(normalized)


def prepare_observer_update(
    conn: sqlite3.Connection,
    identity: ObserverIdentity,
    observer: ChainObserver,
    request: ObserverPrepareRequest,
) -> PreparedObserverUpdate:
    """Load prior state and perform dependency/network preparation read-only."""

    if conn.in_transaction:
        raise AppError(
            "Observer network preparation must finish before the wallet transaction begins",
            code="observer_prepare_in_transaction",
            retryable=False,
        )
    try:
        prior_state = load_observer_state(conn, identity)
    except AppError as exc:
        if not request.force_full or exc.code != "observer_state_rebuild_required":
            raise
        # Rebuild in memory first. The incompatible encrypted rows remain
        # untouched until the coordinator atomically applies the replacement.
        prior_state = None
    prepared = observer.prepare(request, prior_state)
    if not isinstance(prepared, Mapping):
        raise AppError(
            "Observer preparation must return an explicit object",
            code="observer_state_invalid",
            retryable=False,
        )
    return PreparedObserverUpdate(
        identity=identity,
        update=_plain_object(prepared),
        prior_state=prior_state,
        _observer=observer,
    )


def apply_prepared_observer_update(
    conn: sqlite3.Connection,
    prepared: PreparedObserverUpdate,
) -> ChainFacts:
    """Apply and persist an update inside the caller-owned wallet savepoint."""

    if prepared.discarded:
        raise AppError(
            "Discarded observer updates cannot be applied",
            code="observer_update_discarded",
            retryable=False,
        )
    if prepared.applied:
        raise AppError(
            "Observer update was already applied",
            code="observer_update_already_applied",
            retryable=False,
        )
    if not conn.in_transaction:
        raise AppError(
            "Observer application requires the caller's wallet transaction",
            code="observer_apply_outside_transaction",
            retryable=False,
        )
    application = prepared._observer.apply(prepared.update, prepared.prior_state)
    if not isinstance(application, ObserverApplication):
        raise AppError(
            "Observer application returned an invalid result",
            code="observer_state_invalid",
            retryable=False,
        )
    facts = application.facts
    if not isinstance(facts, ChainFacts):
        raise AppError(
            "Observer application must expose normalized chain facts",
            code="observer_state_invalid",
            retryable=False,
        )
    normalized_state = _plain_object(application.state)
    normalized_facts = ChainFacts(
        transaction_records=_plain_objects(
            facts.transaction_records,
            field_name="transaction_records",
        ),
        retracted_external_ids=tuple(
            dict.fromkeys(
                str(value).strip()
                for value in facts.retracted_external_ids
                if str(value).strip()
            )
        ),
        outputs=_plain_objects(facts.outputs, field_name="outputs"),
        coverage=tuple(facts.coverage),
        freshness_checkpoint=_plain_object(dict(facts.freshness_checkpoint)),
    )
    persist_observer_state(
        conn,
        prepared.identity,
        normalized_state,
        normalized_facts.coverage,
    )
    persist_opaque = getattr(prepared._observer, "persist_opaque_state", None)
    if callable(persist_opaque):
        # The dependency's buffered ForeignStore mutations become durable only
        # here, inside the same caller-owned savepoint as facts and JSON state.
        persist_opaque(conn)
    prepared.applied = True
    return normalized_facts


def discard_prepared_observer_update(prepared: PreparedObserverUpdate) -> None:
    """Drop request-local dependency state after rollback/cancellation."""

    if prepared.discarded:
        return
    prepared._observer.discard()
    prepared.discarded = True


def discard_prepared_observer_updates(
    prepared_updates: Sequence[PreparedObserverUpdate],
) -> None:
    for prepared in prepared_updates:
        discard_prepared_observer_update(prepared)


__all__ = [
    "ChainFacts",
    "ChainObserver",
    "ObserverApplication",
    "ObserverPrepareRequest",
    "PreparedObserverUpdate",
    "apply_prepared_observer_update",
    "discard_prepared_observer_update",
    "discard_prepared_observer_updates",
    "prepare_observer_update",
]
