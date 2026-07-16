"""SQLite persistence for local derived custody-quantity artifacts."""

from __future__ import annotations

import base64
import binascii
import hashlib
import json
import sqlite3
from typing import TYPE_CHECKING, Any, Mapping, Sequence

from ..errors import AppError
from .custody_evidence import (
    EvidenceSnapshot,
    QuantityObservation,
    build_canonical_quantity_input,
    canonical_event_key,
    enriched_quantity_rows,
)
from .custody_quantity import (
    INTERNAL_REVIEWED,
    INTERNAL_VERIFIED,
    QuantityDomain,
)

if TYPE_CHECKING:
    from .custody_quantity_runtime import CanonicalQuantityState


def persist_authored_evidence_snapshots(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    subject_kind: str,
    subject_id: str,
    snapshots: Sequence[EvidenceSnapshot],
    created_at: str,
) -> int:
    """Write evidence once for a durable authored subject.

    Rows cannot be updated. Scoped book reset and profile teardown may delete
    them together with the authored subject.
    """

    if subject_kind not in {"custody_component", "custody_claim"}:
        raise ValueError("unsupported authored custody evidence subject")
    inserted = 0
    for snapshot in snapshots:
        existing = conn.execute(
            """
            SELECT quantity_hash, payload_json
            FROM custody_authored_evidence_snapshots
            WHERE profile_id = ? AND subject_kind = ?
              AND subject_id = ? AND detail_hash = ?
            """,
            (profile_id, subject_kind, subject_id, snapshot.detail_hash),
        ).fetchone()
        if existing is not None:
            if (
                existing["quantity_hash"] != snapshot.quantity_hash
                or existing["payload_json"] != snapshot.payload_json
            ):
                raise ValueError("custody evidence hash collision")
            continue
        conn.execute(
            """
            INSERT INTO custody_authored_evidence_snapshots(
                workspace_id, profile_id, subject_kind, subject_id,
                detail_hash, quantity_hash, payload_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                workspace_id,
                profile_id,
                subject_kind,
                subject_id,
                snapshot.detail_hash,
                snapshot.quantity_hash,
                snapshot.payload_json,
                created_at,
            ),
        )
        inserted += 1
    return inserted


def evidence_commitment_id(component_id: str, ordinal: int) -> str:
    """Return the stable wire identity for one evidence commitment slot."""

    encoded = json.dumps(
        ["custody-component-evidence-v1", str(component_id), int(ordinal)],
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _commitment_detail_hash(snapshot: EvidenceSnapshot) -> str:
    """Hash decision-material evidence that survives private replication.

    Raw JSON never leaves its device and transaction fingerprints are replaced
    by a keyed sync identifier. Observation lifecycle fields may also change
    without contradicting the reviewed custody quantity: a mempool transaction
    can confirm, move during a reorg, receive a better timestamp, or gain richer
    raw graph detail. Those fields remain in the author's immutable audit
    snapshot, but they must not deactivate a component whose event identity and
    quantity are unchanged.

    Typed facts which can change the interpretation (kind, payment hash, and
    refund linkage) remain committed. The separate ``quantity_hash`` commits
    event identity, wallet, direction, asset, amount, fee, and fee semantics.
    """

    payload = json.loads(snapshot.payload_json)
    if not isinstance(payload, dict):
        raise ValueError("custody evidence payload is invalid")
    for volatile_key in (
        "fingerprint",
        "occurred_at",
        "confirmed_at",
        "raw_json",
    ):
        payload.pop(volatile_key, None)
    encoded = json.dumps(
        payload,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _component_anchor_ids(component: Mapping[str, Any]) -> tuple[str, ...]:
    legs = component.get("legs")
    if legs is None:
        return ()
    return tuple(
        sorted(
            {
                str(leg.get("anchor_transaction_id") or leg.get("transaction_id"))
                for leg in legs
                if leg.get("anchor_transaction_id") not in (None, "")
                or leg.get("transaction_id") not in (None, "")
            }
        )
    )


def _canonical_component_snapshots(
    conn: sqlite3.Connection,
    component: Mapping[str, Any],
) -> tuple[EvidenceSnapshot, ...]:
    transaction_ids = _component_anchor_ids(component)
    if not transaction_ids:
        return ()
    placeholders = ", ".join("?" for _ in transaction_ids)
    rows = conn.execute(
        f"""
        SELECT t.*, w.kind AS wallet_kind, w.config_json AS config_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.id IN ({placeholders})
        ORDER BY t.occurred_at, t.created_at, t.id
        """,
        (component["profile_id"], *transaction_ids),
    ).fetchall()
    canonical = build_canonical_quantity_input(enriched_quantity_rows(rows))
    rejected_ids = {
        transaction_id
        for issue in canonical.rejected_events
        for transaction_id in issue.transaction_ids
    }
    if rejected_ids:
        raise ValueError(
            "component evidence contains a rejected canonical event: "
            + ", ".join(sorted(rejected_ids))
        )
    alias_hashes = {
        transaction_id: quantity_hash
        for event in canonical.events
        for transaction_id, quantity_hash in event.observation_aliases
    }
    missing = sorted(set(transaction_ids) - set(alias_hashes))
    if missing:
        raise ValueError(
            "component evidence is unavailable for: " + ", ".join(missing)
        )
    relevant_hashes = {alias_hashes[value] for value in transaction_ids}
    snapshots_by_detail = {
        snapshot.detail_hash: snapshot
        for event in canonical.events
        for snapshot in event.evidence_snapshots
        if snapshot.quantity_hash in relevant_hashes
    }
    return tuple(
        sorted(
            snapshots_by_detail.values(),
            key=lambda item: (item.quantity_hash, item.detail_hash),
        )
    )


def persist_component_evidence_commitments(
    conn: sqlite3.Connection,
    *,
    component: Mapping[str, Any],
    snapshots: Sequence[EvidenceSnapshot],
    created_at: str,
) -> int:
    """Persist the payload-free, replicated commitment set exactly once."""

    ordered = tuple(
        sorted(
            (
                (snapshot, _commitment_detail_hash(snapshot))
                for snapshot in snapshots
            ),
            key=lambda item: (item[0].quantity_hash, item[1]),
        )
    )
    existing_rows = conn.execute(
        """
        SELECT id, ordinal, quantity_hash, detail_hash
        FROM custody_component_evidence_commitments
        WHERE component_id = ?
        ORDER BY ordinal, id
        """,
        (component["id"],),
    ).fetchall()
    if existing_rows:
        existing = tuple(
            (int(row["ordinal"]), row["quantity_hash"], row["detail_hash"])
            for row in existing_rows
        )
        expected = tuple(
            (ordinal, snapshot.quantity_hash, detail_hash)
            for ordinal, (snapshot, detail_hash) in enumerate(ordered)
        )
        if existing != expected:
            raise ValueError("custody evidence commitment collision")
        return len(existing)
    for ordinal, (snapshot, detail_hash) in enumerate(ordered):
        conn.execute(
            """
            INSERT INTO custody_component_evidence_commitments(
                id, component_id, workspace_id, profile_id, ordinal,
                quantity_hash, detail_hash, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                evidence_commitment_id(str(component["id"]), ordinal),
                component["id"],
                component["workspace_id"],
                component["profile_id"],
                ordinal,
                snapshot.quantity_hash,
                detail_hash,
                created_at,
            ),
        )
    return len(ordered)


def capture_component_evidence(
    conn: sqlite3.Connection,
    component: Mapping[str, Any],
    *,
    created_at: str,
) -> int:
    """Bind raw local evidence and author commitments during activation.

    The caller owns the surrounding activation savepoint and stores the
    returned count in ``custody_components.expected_evidence_count`` before
    commit.  Returning the total commitment count (not just newly inserted
    rows) keeps repeated activation idempotent.
    """

    snapshots = _canonical_component_snapshots(conn, component)
    persist_authored_evidence_snapshots(
        conn,
        workspace_id=component["workspace_id"],
        profile_id=component["profile_id"],
        subject_kind="custody_component",
        subject_id=component["id"],
        snapshots=snapshots,
        created_at=created_at,
    )
    return persist_component_evidence_commitments(
        conn,
        component=component,
        snapshots=snapshots,
        created_at=created_at,
    )


def component_evidence_status(
    conn: sqlite3.Connection,
    component: Mapping[str, Any],
) -> dict[str, Any]:
    """Compare author commitments with this replica's canonical anchors.

    This never reads or creates the author's raw activation snapshot.  The
    status therefore has the same semantics on the author and on a receiver:
    absent, incomplete, conflicted, or locally unverifiable commitments fail
    closed.
    """

    component_id = str(component.get("id") or "")
    profile_id = str(component.get("profile_id") or "")
    expected = component.get("expected_evidence_count")
    if expected is None and component_id:
        header = conn.execute(
            "SELECT expected_evidence_count FROM custody_components WHERE id = ?",
            (component_id,),
        ).fetchone()
        expected = header["expected_evidence_count"] if header is not None else None
    commitments = conn.execute(
        """
        SELECT id, ordinal, quantity_hash, detail_hash
        FROM custody_component_evidence_commitments
        WHERE component_id = ? AND profile_id = ?
        ORDER BY ordinal, id
        """,
        (component_id, profile_id),
    ).fetchall()
    base = {
        "expected_count": None if expected is None else int(expected),
        "commitment_count": len(commitments),
    }
    def result(status: str, valid: bool, **details: Any) -> dict[str, Any]:
        return {
            **base,
            "valid": valid,
            "usable": valid,
            "status": status,
            "reason": status,
            **details,
        }

    if expected is None:
        return result("commitment_header_missing", False)
    ordinal_values = [int(row["ordinal"]) for row in commitments]
    if len(commitments) != int(expected) or ordinal_values != list(range(int(expected))):
        return result("commitments_incomplete", False)

    commitment_keys = {json.dumps([row["id"]], separators=(",", ":")) for row in commitments}
    if commitment_keys:
        placeholders = ", ".join("?" for _ in commitment_keys)
        conflict = conn.execute(
            f"""
            SELECT 1 FROM sync_conflicts
            WHERE profile_id = ?
              AND entity_table = 'custody_component_evidence_commitments'
              AND entity_key IN ({placeholders})
              AND status = 'open'
            LIMIT 1
            """,
            (profile_id, *sorted(commitment_keys)),
        ).fetchone()
        if conflict is not None:
            return result("commitments_conflicted", False)

    legs = component.get("legs")
    if legs is None:
        legs = [
            dict(row)
            for row in conn.execute(
                """
                SELECT transaction_id, anchor_transaction_id
                FROM custody_component_legs
                WHERE component_id = ?
                ORDER BY ordinal, id
                """,
                (component_id,),
            ).fetchall()
        ]
    comparison_component = dict(component)
    comparison_component["legs"] = legs
    anchors = _component_anchor_ids(comparison_component)
    if int(expected) == 0:
        return result(
            "matched" if not anchors else "unexpected_transaction_anchors",
            not anchors,
            current_count=0,
        )
    if not anchors:
        return result("transaction_anchors_missing", False)
    try:
        snapshots = _canonical_component_snapshots(conn, comparison_component)
    except (sqlite3.Error, KeyError, TypeError, ValueError) as exc:
        return result("current_evidence_unavailable", False, detail=str(exc))
    current = tuple(
        sorted(
            (
                (item.quantity_hash, _commitment_detail_hash(item))
                for item in snapshots
            )
        )
    )
    committed = tuple(
        (str(row["quantity_hash"]), str(row["detail_hash"])) for row in commitments
    )
    return result(
        "matched" if current == committed else "evidence_mismatch",
        current == committed,
        current_count=len(current),
    )


def component_native_support_status(
    conn: sqlite3.Connection,
    component: Mapping[str, Any],
) -> dict[str, Any]:
    """Derive bounded native-event corroboration for reviewed boundaries.

    Recovered policies do not rewrite an authored component. Each observed
    source/destination boundary is checked for opposite-direction observations
    on imported profile wallets with the exact same protocol-qualified chain
    event. Amount-only and equal-denomination matches never count.
    """

    boundary_legs = tuple(
        leg
        for leg in (component.get("legs") or ())
        if str(leg.get("role") or "") in {"source", "destination", "retained"}
        and (leg.get("anchor_transaction_id") or leg.get("transaction_id"))
        and int(leg.get("amount_msat") or 0) > 0
    )
    base = {
        "boundary_count": len(boundary_legs),
        "supported_boundary_count": 0,
        "partially_supported_boundary_count": 0,
        "contradicted_boundary_count": 0,
    }
    if not boundary_legs:
        return {**base, "status": "unverified", "usable": True}

    rows = [
        dict(row)
        for row in conn.execute(
            """
            SELECT t.*, w.kind AS wallet_kind, w.config_json AS config_json
            FROM transactions t
            JOIN wallets w ON w.id = t.wallet_id
            WHERE t.profile_id = ? AND t.excluded = 0
            ORDER BY t.occurred_at, t.created_at, t.id
            """,
            (str(component.get("profile_id") or ""),),
        ).fetchall()
    ]
    observations: dict[str, QuantityObservation] = {}
    keys: dict[str, Any] = {}
    for row in enriched_quantity_rows(rows):
        row_id = str(row.get("id") or "")
        try:
            key = canonical_event_key(row)
            observations[row_id] = QuantityObservation.from_transaction(row, key)
            keys[row_id] = key
        except (TypeError, ValueError):
            continue

    supported = 0
    partially_supported = 0
    contradicted = 0
    for leg in boundary_legs:
        anchor_id = str(
            leg.get("anchor_transaction_id") or leg.get("transaction_id") or ""
        )
        anchor = observations.get(anchor_id)
        if (
            anchor is None
            or anchor.event_key.native_namespace != "chain"
            or anchor.event_key.chain not in {"bitcoin", "liquid"}
        ):
            continue
        opposite_direction = (
            "inbound" if anchor.direction == "outbound" else "outbound"
        )
        same_event = [
            observations[row_id]
            for row_id, key in keys.items()
            if row_id != anchor_id
            and observations[row_id].direction == opposite_direction
            and key == anchor.event_key
        ]
        if not same_event:
            continue
        # Liquid events can carry several assets. Other-asset observations are
        # neither support nor contradiction for this boundary quantity.
        compatible = [
            item
            for item in same_event
            if item.asset == anchor.asset
        ]
        if not compatible:
            continue
        # Deduplicate repeated imports of the same wallet/event leg while
        # retaining distinct owned outputs.
        unique = {item.quantity_hash: item for item in compatible}
        recovered_amount = sum(item.principal_msat for item in unique.values())
        # Guided source legs include the separately reviewed network fee in
        # their debit amount. Native owned counterparts cover principal; the
        # fee remains its own component/observation fact.
        if recovered_amount > anchor.principal_msat:
            contradicted += 1
            continue
        if recovered_amount == anchor.principal_msat:
            supported += 1
        else:
            partially_supported += 1

    status = (
        "contradicted"
        if contradicted
        else (
            "corroborated"
            if supported == len(boundary_legs)
            else "partial" if supported or partially_supported else "unverified"
        )
    )
    return {
        **base,
        "status": status,
        "usable": status != "contradicted",
        "supported_boundary_count": supported,
        "partially_supported_boundary_count": partially_supported,
        "contradicted_boundary_count": contradicted,
    }


def baseline_missing_component_evidence(
    conn: sqlite3.Connection,
    components: Sequence[Mapping[str, Any]],
    *,
    created_at: str,
) -> dict[str, Any]:
    """Legacy compatibility check that never blesses receiver-local evidence.

    Old local activation snapshots are migrated by ``open_db``.  Keeping this
    helper as a read-only status adapter avoids silently changing older callers
    while ensuring a replicated/current row can never become an author
    commitment source.
    """

    existing: list[str] = []
    blocked: list[dict[str, str]] = []
    for component in sorted(components, key=lambda item: str(item.get("id") or "")):
        component_id = str(component.get("id") or "")
        if not component_id:
            blocked.append({"component_id": "", "reason": "component_id_missing"})
            continue
        if component.get("effective_state") != "active":
            blocked.append(
                {"component_id": component_id, "reason": "component_not_effective"}
            )
            continue
        status = component_evidence_status(conn, component)
        if status["valid"]:
            existing.append(component_id)
            continue
        blocked.append({"component_id": component_id, "reason": status["status"]})
    return {
        "baselined_component_ids": [],
        "existing_component_ids": existing,
        "blocked": blocked,
    }


def load_component_evidence_snapshots(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, tuple[dict[str, str], ...]]:
    """Load author-local activation snapshots for optional stronger checks."""

    rows = conn.execute(
        """
        SELECT subject_id, detail_hash, quantity_hash, payload_json, created_at
        FROM custody_authored_evidence_snapshots
        WHERE profile_id = ? AND subject_kind = 'custody_component'
        ORDER BY subject_id, quantity_hash, detail_hash
        """,
        (profile_id,),
    ).fetchall()
    grouped: dict[str, list[dict[str, str]]] = {}
    for row in rows:
        grouped.setdefault(str(row["subject_id"]), []).append(dict(row))
    return {key: tuple(value) for key, value in grouped.items()}


def replace_canonical_quantity_state(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    state: CanonicalQuantityState,
    created_at: str,
) -> dict[str, int]:
    """Atomically replace all derived canonical quantity state for a book."""

    savepoint = "replace_canonical_quantity_state"
    conn.execute(f"SAVEPOINT {savepoint}")
    try:
        result = _replace_canonical_quantity_state(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            state=state,
            created_at=created_at,
        )
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        raise
    conn.execute(f"RELEASE SAVEPOINT {savepoint}")
    return result


def _replace_canonical_quantity_state(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    state: CanonicalQuantityState,
    created_at: str,
) -> dict[str, int]:
    """Replace derived rows inside the caller's active savepoint."""

    for table in (
        "journal_quantity_postings",
        "journal_quantity_issues",
        "journal_quantity_balances",
        "journal_custody_decisions",
        "journal_custody_economic_relations",
    ):
        conn.execute(f"DELETE FROM {table} WHERE profile_id = ?", (profile_id,))

    observations = {
        item.quantity_hash: item for item in state.projection.observations
    }
    posting_rows = []
    balances: dict[tuple[str, str, str], int] = {}
    for posting in state.projection.postings:
        observation = observations.get(posting.observation_hash or "")
        posting_rows.append(
            (
                posting.posting_id,
                workspace_id,
                profile_id,
                (
                    observation.anchor_transaction_id
                    if observation is not None
                    else None
                ),
                posting.observation_hash,
                observation.occurred_at if observation is not None else None,
                posting.asset,
                posting.location_kind,
                posting.location_id,
                posting.amount_msat,
                posting.state,
                created_at,
            )
        )
        key = (posting.location_kind, posting.location_id, posting.asset)
        balances[key] = balances.get(key, 0) + posting.amount_msat
    conn.executemany(
        """
        INSERT INTO journal_quantity_postings(
            posting_id, workspace_id, profile_id, transaction_id,
            observation_hash, occurred_at, asset, location_kind, location_id,
            amount_msat, state, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        posting_rows,
    )
    conn.executemany(
        """
        INSERT INTO journal_quantity_issues(
            issue_id, workspace_id, profile_id, issue_type, state, asset,
            amount_msat, occurred_at, transaction_ids_json, reason,
            detail_json, blocks_from, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                issue.issue_id,
                workspace_id,
                profile_id,
                issue.issue_type,
                issue.state,
                issue.asset,
                issue.amount_msat,
                issue.occurred_at or None,
                json.dumps(list(issue.transaction_ids), sort_keys=True),
                issue.reason,
                json.dumps(dict(issue.details), sort_keys=True),
                state.tax_eligibility.blocked_from,
                created_at,
            )
            for issue in state.issues
        ],
    )
    conn.executemany(
        """
        INSERT INTO journal_quantity_balances(
            workspace_id, profile_id, location_kind, location_id,
            asset, amount_msat, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?)
        """,
        [
            (
                workspace_id,
                profile_id,
                location_kind,
                location_id,
                asset,
                amount_msat,
                created_at,
            )
            for (location_kind, location_id, asset), amount_msat in sorted(
                balances.items()
            )
            if amount_msat != 0
        ],
    )
    component_review_defaults = {
        str(row["id"]): {
            "review_kind": row["component_type"],
            "policy": "carrying-value",
            "confidence_at_review": None,
            "review_source": row["authored_source"],
            "notes": row["notes"],
            "swap_fee_msat": None,
            "swap_fee_kind": None,
        }
        for row in conn.execute(
            "SELECT id, component_type, authored_source, notes "
            "FROM custody_components WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }
    component_review_by_edge: dict[tuple[str, str, str], dict[str, Any]] = {}
    for row in conn.execute(
        """
        SELECT
            term.component_id,
            COALESCE(source_leg.anchor_transaction_id,
                     source_leg.transaction_id) AS source_transaction_id,
            COALESCE(target_leg.anchor_transaction_id,
                     target_leg.transaction_id) AS target_transaction_id,
            term.review_kind,
            term.tax_policy AS policy,
            term.confidence_at_review,
            term.review_source,
            term.swap_fee_msat,
            term.swap_fee_kind,
            component.notes
        FROM custody_component_economic_terms term
        JOIN custody_component_legs source_leg ON source_leg.id = term.source_leg_id
        JOIN custody_component_legs target_leg ON target_leg.id = term.target_leg_id
        JOIN custody_components component ON component.id = term.component_id
        WHERE term.profile_id = ?
        ORDER BY term.component_id, term.ordinal, term.id
        """,
        (profile_id,),
    ).fetchall():
        source_transaction_id = str(row["source_transaction_id"] or "")
        target_transaction_id = str(row["target_transaction_id"] or "")
        if not source_transaction_id or not target_transaction_id:
            continue
        component_review_by_edge.setdefault(
            (str(row["component_id"]), source_transaction_id, target_transaction_id),
            {
                "review_kind": row["review_kind"],
                "policy": row["policy"],
                "confidence_at_review": row["confidence_at_review"],
                "review_source": row["review_source"],
                "notes": row["notes"],
                "swap_fee_msat": row["swap_fee_msat"],
                "swap_fee_kind": row["swap_fee_kind"],
            },
        )

    decision_rows = []
    eligible_decisions = set(state.tax_eligibility.eligible_decisions)
    for decision in state.projection.decisions:
        if (
            decision.state not in {INTERNAL_VERIFIED, INTERNAL_REVIEWED}
            or decision.target is None
        ):
            continue
        source = observations[decision.source.observation_hash]
        target = observations[decision.target.observation_hash]
        source_domain = QuantityDomain.from_observation(source)
        target_domain = QuantityDomain.from_observation(target)
        decision_payload = [
            "canonical-custody-decision-v1",
            decision.source.observation_hash,
            decision.source.start_msat,
            decision.source.end_msat,
            decision.target.observation_hash,
            decision.target.start_msat,
            decision.target.end_msat,
            decision.state,
            decision.reason,
            decision.atomic_bundle_id,
            decision.component_id,
        ]
        decision_id = hashlib.sha256(
            json.dumps(
                decision_payload,
                ensure_ascii=True,
                separators=(",", ":"),
            ).encode("utf-8")
        ).hexdigest()
        basis_barrier = state.tax_eligibility.barrier_for(source)
        review_meta = component_review_by_edge.get(
            (
                str(decision.component_id or ""),
                source.anchor_transaction_id,
                target.anchor_transaction_id,
            )
        ) or component_review_defaults.get(str(decision.component_id or ""), {})
        decision_rows.append(
            (
                decision_id,
                workspace_id,
                profile_id,
                source.anchor_transaction_id,
                target.anchor_transaction_id,
                decision.source.observation_hash,
                decision.source.start_msat,
                decision.source.end_msat,
                decision.target.observation_hash,
                decision.target.start_msat,
                decision.target.end_msat,
                source.wallet_id,
                target.wallet_id,
                source_domain.network,
                target_domain.network,
                source_domain.rail,
                target_domain.rail,
                source.asset,
                target.asset,
                decision.state,
                (
                    "eligible"
                    if decision in eligible_decisions
                    else "blocked_by_prior_custody_basis"
                ),
                (
                    basis_barrier[0] if basis_barrier is not None else None
                ),
                decision.reason,
                str(review_meta.get("review_kind") or decision.reason),
                str(review_meta.get("policy") or "carrying-value"),
                review_meta.get("confidence_at_review"),
                review_meta.get("review_source") or "journal_builder",
                review_meta.get("notes"),
                review_meta.get("swap_fee_msat"),
                review_meta.get("swap_fee_kind"),
                decision.atomic_bundle_id,
                decision.component_id,
                source.occurred_at or None,
                target.occurred_at or None,
                created_at,
            )
        )
    conn.executemany(
        """
        INSERT INTO journal_custody_decisions(
            decision_id, workspace_id, profile_id,
            source_transaction_id, target_transaction_id,
            source_observation_hash, source_start_msat, source_end_msat,
            target_observation_hash, target_start_msat, target_end_msat,
            source_wallet_id, target_wallet_id,
            source_network, target_network, source_rail, target_rail,
            source_asset, target_asset,
                state, basis_state, basis_barrier_at, reason,
                review_kind, policy, confidence_at_review, review_source, notes,
                swap_fee_msat, swap_fee_kind,
                atomic_group_id, component_id, occurred_at,
                target_occurred_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        decision_rows,
    )
    observations_by_transaction = {
        item.anchor_transaction_id: item
        for item in state.projection.observations
        if item.anchor_transaction_id
    }
    external_ids_by_transaction = {
        str(row["id"]): row["external_id"]
        for row in conn.execute(
            "SELECT id, external_id FROM transactions WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }
    eligible_source_slices = {
        (
            observations[item.source.observation_hash].anchor_transaction_id,
            item.source.amount_msat,
            item.component_id,
        )
        for item in state.tax_eligibility.eligible_decisions
    }
    relation_rows = []
    relations = (
        *(
            ("conversion", relation)
            for relation in state.reviewed_conversion_pairs
        ),
        *(
            ("direct_payout", relation)
            for relation in state.reviewed_direct_payouts
        ),
    )
    for relation_kind, relation in relations:
        source_transaction_id = str(
            relation.get("out_id")
            or relation.get("out_transaction_id")
            or ""
        )
        target_transaction_id = (
            str(relation.get("in_id"))
            if relation.get("in_id") not in (None, "")
            else None
        )
        source = observations_by_transaction.get(source_transaction_id)
        target = (
            observations_by_transaction.get(target_transaction_id)
            if target_transaction_id is not None
            else None
        )
        if source is None:
            continue
        source_amount = int(
            relation.get("out_amount")
            or relation.get("out_amount_msat")
            or 0
        )
        target_amount = int(
            relation.get("in_amount")
            or relation.get("payout_amount")
            or 0
        )
        if source_amount <= 0 or target_amount <= 0:
            continue
        target_external_id = (
            external_ids_by_transaction.get(target_transaction_id)
            if target_transaction_id is not None
            else relation.get("payout_external_id")
        )
        component_meta = component_review_defaults.get(
            str(relation.get("component_id") or ""), {}
        )
        relation_notes = relation.get("notes") or component_meta.get("notes")
        relation_review_source = (
            relation.get("review_source")
            or component_meta.get("review_source")
            or "journal_builder"
        )
        payload = [
            "canonical-custody-economic-relation-v2",
            relation_kind,
            source_transaction_id,
            target_transaction_id,
            relation.get("component_id"),
            source.asset,
            (target.asset if target is not None else relation.get("payout_asset")),
            source_amount,
            target_amount,
            relation.get("kind"),
            relation.get("policy"),
            relation.get("swap_fee_msat"),
            relation.get("swap_fee_kind"),
            relation_notes,
            relation.get("confidence_at_review"),
            relation_review_source,
            target_external_id,
            relation.get("counterparty"),
            relation.get("payout_fiat_value"),
        ]
        relation_id = hashlib.sha256(
            json.dumps(
                payload,
                ensure_ascii=True,
                separators=(",", ":"),
            ).encode("utf-8")
        ).hexdigest()
        relation_rows.append(
            (
                relation_id,
                workspace_id,
                profile_id,
                relation_kind,
                source_transaction_id,
                target_transaction_id,
                relation.get("component_id"),
                source.asset,
                target.asset if target is not None else relation.get("payout_asset"),
                source_amount,
                target_amount,
                str(relation.get("kind") or relation_kind),
                str(relation.get("policy") or "taxable"),
                relation.get("swap_fee_msat"),
                relation.get("swap_fee_kind"),
                relation_notes,
                relation.get("confidence_at_review"),
                relation_review_source,
                target_external_id,
                relation.get("counterparty"),
                relation.get("payout_fiat_value"),
                (
                    "eligible"
                    if (
                        source_transaction_id,
                        source_amount,
                        relation.get("component_id"),
                    )
                    in eligible_source_slices
                    else "blocked_by_prior_custody_basis"
                ),
                source.occurred_at or None,
                (
                    target.occurred_at
                    if target is not None
                    else relation.get("payout_occurred_at")
                ),
                created_at,
            )
        )
    conn.executemany(
        """
        INSERT INTO journal_custody_economic_relations(
            relation_id, workspace_id, profile_id, relation_kind,
            source_transaction_id, target_transaction_id, component_id,
            source_asset, target_asset, source_amount_msat,
            target_amount_msat, review_kind, policy, swap_fee_msat,
            swap_fee_kind, notes, confidence_at_review, review_source,
            target_external_id, counterparty,
            target_fiat_value_exact, basis_state,
            occurred_at, target_occurred_at, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        relation_rows,
    )
    return {
        "postings": len(posting_rows),
        "issues": len(state.issues),
        "balances": sum(amount != 0 for amount in balances.values()),
        "decisions": len(decision_rows),
        "economic_relations": len(relation_rows),
    }


def custody_decision_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    limit: int = 100,
    transaction_ids: Sequence[str] | None = None,
    cursor: str | None = None,
) -> dict[str, Any]:
    """Return a bounded, redacted semantic view of canonical custody edges.

    Observation commitments and exact slice offsets remain private storage
    details.  This reader exposes the amount and durable transaction/wallet
    anchors needed by transaction graphs and audit summaries, with custody
    finality named separately from downstream tax state.
    """

    bounded_limit = max(1, min(int(limit), 500))
    where = "d.profile_id = ?"
    params: list[Any] = [profile_id]
    selected: tuple[str, ...] | None = None
    if transaction_ids is not None:
        selected = tuple(
            sorted({str(value) for value in transaction_ids if str(value)})
        )
        if not selected:
            return {
                "records": [],
                "count": 0,
                "returned": 0,
                "truncated": False,
                "next_cursor": None,
                "observation_commitments_included": False,
                "replicated": False,
            }
        placeholders = ",".join("?" for _ in selected)
        where += (
            f" AND (d.source_transaction_id IN ({placeholders})"
            f" OR d.target_transaction_id IN ({placeholders}))"
        )
        params.extend(selected)
        params.extend(selected)
    scope_where = where
    scope_params = list(params)
    cursor_filters = {
        "profile_scope_hash": hashlib.sha256(
            profile_id.encode("utf-8")
        ).hexdigest(),
        "transaction_ids": list(selected) if selected is not None else None,
    }
    cursor_data = _decode_custody_decision_cursor(cursor, cursor_filters)
    if cursor_data is not None:
        cursor_occurred_at = cursor_data["occurred_at"]
        cursor_decision_id = cursor_data["decision_id"]
        if cursor_occurred_at is None:
            where += " AND d.occurred_at IS NULL AND d.decision_id < ?"
            params.append(cursor_decision_id)
        else:
            where += (
                " AND (d.occurred_at < ? OR d.occurred_at IS NULL"
                " OR (d.occurred_at = ? AND d.decision_id < ?))"
            )
            params.extend(
                [cursor_occurred_at, cursor_occurred_at, cursor_decision_id]
            )
    total_count = int(
        conn.execute(
            f"SELECT COUNT(*) FROM journal_custody_decisions d "
            f"WHERE {scope_where}",
            scope_params,
        ).fetchone()[0]
    )
    rows = conn.execute(
        f"""
        SELECT d.decision_id AS _cursor_decision_id,
               d.source_transaction_id, d.target_transaction_id,
               d.source_wallet_id, source_wallet.label AS source_wallet_label,
               d.target_wallet_id, target_wallet.label AS target_wallet_label,
               d.source_network, d.target_network,
               d.source_rail, d.target_rail,
               d.source_asset, d.target_asset,
               d.source_end_msat - d.source_start_msat AS amount_msat,
               d.state AS custody_state, d.basis_state, d.basis_barrier_at,
               d.reason, d.atomic_group_id,
               d.component_id, d.occurred_at, d.target_occurred_at,
               d.created_at
        FROM journal_custody_decisions d
        LEFT JOIN wallets source_wallet ON source_wallet.id = d.source_wallet_id
        LEFT JOIN wallets target_wallet ON target_wallet.id = d.target_wallet_id
        WHERE {where}
        ORDER BY d.occurred_at DESC, d.decision_id DESC
        LIMIT ?
        """,
        [*params, bounded_limit + 1],
    ).fetchall()
    has_more = len(rows) > bounded_limit
    page = rows[:bounded_limit]
    next_cursor = (
        _encode_custody_decision_cursor(page[-1], cursor_filters)
        if has_more and page
        else None
    )
    records = []
    for row in page:
        record = dict(row)
        record.pop("_cursor_decision_id", None)
        records.append(record)
    return {
        "records": records,
        "count": total_count,
        "returned": len(records),
        "truncated": has_more,
        "next_cursor": next_cursor,
        "observation_commitments_included": False,
        "replicated": False,
    }


def _encode_custody_decision_cursor(
    row: Mapping[str, Any],
    filters: Mapping[str, Any],
) -> str:
    payload = {
        "decision_id": row["_cursor_decision_id"],
        "filters": filters,
        "occurred_at": row["occurred_at"],
        "version": 1,
    }
    token = json.dumps(
        payload,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    )
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_custody_decision_cursor(
    cursor: str | None,
    filters: Mapping[str, Any],
) -> Mapping[str, Any] | None:
    if cursor in (None, ""):
        return None
    if not isinstance(cursor, str):
        raise AppError("cursor must be a string", code="validation", retryable=False)
    try:
        padding = "=" * (-len(cursor) % 4)
        payload = json.loads(
            base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        )
        if payload.get("version") != 1 or payload.get("filters") != filters:
            raise ValueError("cursor scope mismatch")
        if not isinstance(payload.get("decision_id"), str) or not payload[
            "decision_id"
        ]:
            raise ValueError("missing cursor decision id")
        occurred_at = payload.get("occurred_at")
        if occurred_at is not None and not isinstance(occurred_at, str):
            raise ValueError("invalid cursor timestamp")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            "Invalid cursor",
            code="validation",
            hint=(
                "Pass the exact next_cursor value from the previous response; "
                "do not modify it or change filters."
            ),
            retryable=False,
        ) from exc


def blocking_quantity_issues(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[Mapping[str, Any]]:
    try:
        rows = conn.execute(
            """
            SELECT issue_id, issue_type, state, asset, amount_msat,
                   occurred_at, reason, blocks_from
            FROM journal_quantity_issues
            WHERE profile_id = ?
            ORDER BY COALESCE(occurred_at, ''), issue_id
            """,
            (profile_id,),
        ).fetchall()
    except Exception as exc:
        # This table is the persisted accounting stop signal. Treating a
        # missing table, stale column set, unreadable page, or driver-specific
        # SQLCipher OperationalError as an empty result would turn an unknown
        # custody state into report clearance.
        raise AppError(
            "Custody quantity blocker state is unavailable.",
            code="custody_quantity_state_unavailable",
            hint=(
                "Repair or migrate the local book, then rebuild journals before "
                "relying on reports."
            ),
            details={
                "profile_id": profile_id,
                "operation": "read_journal_quantity_issues",
                "error_type": type(exc).__name__,
            },
            retryable=False,
        ) from exc
    return [dict(row) for row in rows]


def custody_quantity_readiness_summary(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    journal_status: str,
    include_journal_state: bool = True,
) -> dict[str, Any]:
    """Return the canonical, privacy-safe custody-gap readiness summary.

    An empty derived issue table is only described as clear when the journal
    projection is current.  The wording is intentionally qualified: Kassiber
    cannot prove that the user imported every wallet that ever existed.
    """

    issues = blocking_quantity_issues(conn, profile_id)
    try:
        presumed_rows = conn.execute(
            """
            SELECT asset, COUNT(*) AS slice_count,
                   COUNT(DISTINCT transaction_id) AS transaction_count,
                   SUM(amount_msat) AS amount_msat
            FROM journal_quantity_postings
            WHERE profile_id = ?
              AND location_kind = 'external'
              AND state = 'external_presumed'
              AND amount_msat > 0
            GROUP BY asset
            ORDER BY asset
            """,
            (profile_id,),
        ).fetchall()
    except sqlite3.OperationalError:
        presumed_rows = []
    presumed_by_asset = [
        {
            "asset": str(row["asset"]),
            "amount_msat": int(row["amount_msat"] or 0),
            "slice_count": int(row["slice_count"] or 0),
            "transaction_count": int(row["transaction_count"] or 0),
        }
        for row in presumed_rows
    ]
    presumed_external = {
        "slice_count": sum(item["slice_count"] for item in presumed_by_asset),
        "transaction_count": sum(
            item["transaction_count"] for item in presumed_by_asset
        ),
        "by_asset": presumed_by_asset,
        "treatment": "warning_not_blocker",
    }
    by_asset: dict[str, dict[str, int]] = {}
    by_state: dict[str, dict[str, Any]] = {}
    quantified_issue_count = 0
    for issue in issues:
        state = str(issue.get("state") or "unknown")
        state_summary = by_state.setdefault(
            state,
            {"state": state, "issue_count": 0, "unresolved_by_asset": {}},
        )
        state_summary["issue_count"] += 1
        amount = issue.get("amount_msat")
        if amount is None:
            continue
        quantified_issue_count += 1
        asset = str(issue.get("asset") or "UNKNOWN")
        asset_summary = by_asset.setdefault(
            asset,
            {"asset": asset, "amount_msat": 0, "issue_count": 0},
        )
        asset_summary["amount_msat"] += int(amount)
        asset_summary["issue_count"] += 1
        state_assets = state_summary["unresolved_by_asset"]
        state_assets[asset] = int(state_assets.get(asset, 0)) + int(amount)

    unresolved_by_asset = [by_asset[key] for key in sorted(by_asset)]
    state_rows = []
    for state in sorted(by_state):
        item = by_state[state]
        state_rows.append(
            {
                "state": item["state"],
                "issue_count": item["issue_count"],
                "unresolved_by_asset": [
                    {"asset": asset, "amount_msat": amount}
                    for asset, amount in sorted(item["unresolved_by_asset"].items())
                ],
            }
        )

    blocked_from = min(
        (str(item["blocks_from"]) for item in issues if item.get("blocks_from")),
        default=None,
    )
    if not include_journal_state:
        status = "excluded"
        status_text = "Custody gap status excluded by export options"
    elif journal_status == "current":
        if issues:
            status = "known_custody_gaps"
            status_text = "Known custody gaps require review"
        else:
            status = "no_known_custody_gaps"
            status_text = "No known custody gaps"
    elif journal_status == "no_transactions":
        status = "not_evaluated"
        status_text = "Custody gaps not evaluated: no active transactions"
    else:
        status = "needs_processing"
        status_text = "Custody gap status needs journal processing"

    return {
        "status": status,
        "status_text": status_text,
        "derived_state_current": journal_status == "current",
        "issue_count": len(issues),
        "quantified_issue_count": quantified_issue_count,
        "unquantified_issue_count": len(issues) - quantified_issue_count,
        "unresolved_by_asset": unresolved_by_asset,
        "by_state": state_rows,
        "blocked_from": blocked_from,
        "presumed_external": presumed_external,
        "warnings": (
            [
                {
                    "code": "external_custody_presumed",
                    "severity": "warning",
                    "message": (
                        "Unmatched outflows are treated as presumed external; "
                        "this is visible but does not block reports."
                    ),
                }
            ]
            if presumed_external["slice_count"]
            else []
        ),
        "qualification": (
            "This reports gaps detectable from current imported evidence; it "
            "does not assert that every wallet was imported."
        ),
    }


def authored_evidence_hash_summary(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    transaction_ids: Sequence[str] | None = None,
) -> dict[str, Any]:
    """Return authored evidence hash anchors without raw evidence payloads.

    A transaction-scoped export includes component snapshots only when one of
    that component's legs anchors a selected transaction.  Claim snapshots do
    not currently have a transaction-scoping relation and are therefore only
    included for full-profile exports.
    """

    params: list[Any] = [profile_id]
    where = "s.profile_id = ?"
    if transaction_ids is not None:
        selected = tuple(sorted({str(value) for value in transaction_ids}))
        if not selected:
            return {"snapshot_count": 0, "subject_count": 0, "hashes": []}
        placeholders = ", ".join("?" for _ in selected)
        where += f"""
            AND s.subject_kind = 'custody_component'
            AND EXISTS (
                SELECT 1
                FROM custody_component_legs l
                WHERE l.profile_id = s.profile_id
                  AND l.component_id = s.subject_id
                  AND COALESCE(l.anchor_transaction_id, l.transaction_id)
                      IN ({placeholders})
            )
        """
        params.extend(selected)
    try:
        rows = conn.execute(
            f"""
            SELECT s.subject_kind, s.subject_id, s.detail_hash,
                   s.quantity_hash, s.created_at
            FROM custody_authored_evidence_snapshots s
            WHERE {where}
            ORDER BY s.subject_kind, s.subject_id, s.created_at,
                     s.detail_hash, s.quantity_hash
            """,
            params,
        ).fetchall()
    except sqlite3.OperationalError:
        rows = []
    hashes = [dict(row) for row in rows]
    return {
        "snapshot_count": len(hashes),
        "subject_count": len(
            {(item["subject_kind"], item["subject_id"]) for item in hashes}
        ),
        "hashes": hashes,
    }


__all__ = [
    "authored_evidence_hash_summary",
    "baseline_missing_component_evidence",
    "blocking_quantity_issues",
    "capture_component_evidence",
    "component_evidence_status",
    "component_native_support_status",
    "custody_quantity_readiness_summary",
    "custody_decision_rows",
    "evidence_commitment_id",
    "load_component_evidence_snapshots",
    "persist_authored_evidence_snapshots",
    "persist_component_evidence_commitments",
    "replace_canonical_quantity_state",
]
