"""Durable review decisions and exact guided bridges for custody gaps.

Candidate discovery is derived and may change whenever imported evidence
changes. Dismissals therefore bind to the complete suggestion fingerprint, so
changed rankings or competing explanations reopen the hint. A reviewed bridge
binds to a narrower authored-claim commitment: unrelated future candidates do
not invalidate an otherwise unchanged economic interpretation.

This module is local application state; it does not expose raw transaction ids,
descriptors, or wallet configuration through its UI result.
"""

from __future__ import annotations

import hashlib
import json
import sqlite3
from types import SimpleNamespace
import uuid
from typing import Any, Mapping, Sequence

from ..db import (
    backfill_custody_gap_review_relation_set,
    backfill_custody_gap_review_relations,
    custody_gap_review_transaction_id,
)
from ..errors import AppError
from ..time_utils import now_iso
from . import custody_components
from . import custody_filed_reports
from .custody_allocations import CustodyAllocationError, allocate_msat_fifo
from .custody_evidence import row_boundary_amounts, resolve_protocol_scope
from .custody_gaps import CustodyGapCandidate


RESIDUAL_CLASSIFICATIONS = frozenset(
    {
        "external_payment",
        "external_disposal",
        "external_gift",
        "external_loss",
        "retained_custody",
        "suspense_continuation",
    }
)
_EXTERNAL_RESIDUAL_CLASSIFICATIONS = frozenset(
    {
        "external_payment",
        "external_disposal",
        "external_gift",
        "external_loss",
    }
)
_REVIEW_EVENT_KINDS = frozenset(
    {
        "review_decision",
        "bridge_created",
        "bridge_reopened",
        "bridge_revised",
        "residual_classified",
    }
)


def _residual_custody_state(classification: str) -> str:
    if classification in _EXTERNAL_RESIDUAL_CLASSIFICATIONS:
        return "external_confirmed"
    if classification == "retained_custody":
        return "internal_reviewed"
    return "custody_suspense"


def residual_custody_state(classification: str) -> str:
    """Return the country-neutral custody state for a reviewed residual action."""

    return _residual_custody_state(_normalize_residual_classification(classification))


def _event_kind(review: Mapping[str, Any]) -> str:
    return str(review.get("event_kind") or "review_decision")


def candidate_fingerprint(candidate: CustodyGapCandidate) -> str:
    """Hash the complete derived suggestion, including ranking context."""

    payload = {
        "schema_version": 2,
        "gap_id": candidate.gap_id,
        "profile_id": candidate.profile_id,
        "asset": candidate.asset,
        "protocol_chain": candidate.protocol_chain,
        "network": candidate.network,
        "source_ids": candidate.source_ids,
        "return_ids": candidate.return_ids,
        "source_wallet_ids": candidate.source_wallet_ids,
        "destination_wallet_ids": candidate.destination_wallet_ids,
        "source_total_msat": candidate.source_total_msat,
        "source_fee_msat": candidate.source_fee_msat,
        "source_debit_msat": candidate.source_debit_msat,
        "return_total_msat": candidate.return_total_msat,
        "retained_msat": candidate.retained_msat,
        "residual_msat": candidate.residual_msat,
        "excess_msat": candidate.excess_msat,
        "started_at": candidate.started_at,
        "ended_at": candidate.ended_at,
        "promotion_eligible": candidate.promotion_eligible,
        "competitor_score_margin": candidate.competitor_score_margin,
        "conflict_set_id": candidate.conflict_set_id,
        "conflict_size": candidate.conflict_size,
        "reason_codes": candidate.reason_codes,
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def authored_claim_fingerprint(candidate: CustodyGapCandidate) -> str:
    """Hash only facts committed by a reviewed bridge.

    Scores, confidence, reason codes, competitor margins and conflict-set
    cardinality belong to discovery. They may change when an unrelated
    transaction is imported and must not make a durable authored component
    stale. Transaction quantity/evidence commitments remain independently
    enforced by ``custody_quantity_store.component_evidence_status``.
    """

    payload = {
        "schema_version": 1,
        "gap_id": candidate.gap_id,
        "profile_id": candidate.profile_id,
        "asset": candidate.asset,
        "protocol_chain": candidate.protocol_chain,
        "network": candidate.network,
        "source_ids": candidate.source_ids,
        "return_ids": candidate.return_ids,
        "source_wallet_ids": candidate.source_wallet_ids,
        "destination_wallet_ids": candidate.destination_wallet_ids,
        "source_total_msat": candidate.source_total_msat,
        "source_fee_msat": candidate.source_fee_msat,
        "source_debit_msat": candidate.source_debit_msat,
        "return_total_msat": candidate.return_total_msat,
        "retained_msat": candidate.retained_msat,
        "residual_msat": candidate.residual_msat,
        "excess_msat": candidate.excess_msat,
        "started_at": candidate.started_at,
        "ended_at": candidate.ended_at,
    }
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def latest_reviews(conn: sqlite3.Connection, profile_id: str) -> dict[str, dict[str, Any]]:
    try:
        rows = conn.execute(
            """
            SELECT r.*
            FROM custody_gap_reviews r
            JOIN (
                SELECT gap_id, MAX(revision) AS revision
                FROM custody_gap_reviews
                WHERE profile_id = ?
                GROUP BY gap_id
            ) latest ON latest.gap_id = r.gap_id AND latest.revision = r.revision
            WHERE r.profile_id = ?
            ORDER BY r.gap_id, r.created_at, r.id
            """,
            (profile_id, profile_id),
        ).fetchall()
    except sqlite3.OperationalError:
        return {}
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        grouped.setdefault(str(row["gap_id"]), []).append(dict(row))
    output: dict[str, dict[str, Any]] = {}
    for gap_id, siblings in grouped.items():
        signatures = {
            (
                row["candidate_fingerprint"],
                row["action"],
                row.get("event_kind") or "review_decision",
                row["component_id"],
            )
            for row in siblings
        }
        output[gap_id] = dict(siblings[-1])
        if len(signatures) > 1:
            output[gap_id]["action"] = "conflicting_review"
    return output


def review_state(
    conn: sqlite3.Connection,
    candidate: CustodyGapCandidate,
    review: Mapping[str, Any] | None,
) -> dict[str, str | None]:
    fallback = "conflicting" if candidate.conflict_size > 1 else "needs_review"
    if not review:
        return {
            "status": fallback,
            "reason": "competing_candidates" if candidate.conflict_size > 1 else None,
        }
    if review.get("action") == "conflicting_review":
        return {"status": "conflicting", "reason": "concurrent_review_conflict"}
    if _event_kind(review) == "bridge_reopened":
        return {"status": "needs_review", "reason": "bridge_reopened"}
    expected_fingerprint = (
        authored_claim_fingerprint(candidate)
        if review.get("action") == "resolved"
        else candidate_fingerprint(candidate)
    )
    if review.get("candidate_fingerprint") != expected_fingerprint:
        # A stale dismissal simply reopens. A formerly resolved bridge is a
        # stronger historical claim: evidence drift must stay conspicuous as
        # a conflict until the bridge is reviewed again. Exact recovered
        # native support is the exception: it corroborates the unchanged
        # authored bridge even though the advisory candidate population has
        # necessarily changed around it.
        if review.get("action") == "resolved":
            component_status = _component_review_status(
                conn, str(review.get("component_id") or "")
            )
            native_status = component_status.get("native_support_status")
            if component_status["usable"] and native_status in {
                "partial",
                "corroborated",
            }:
                return {
                    "status": "resolved",
                    "reason": None,
                    "native_support_status": str(native_status),
                }
            return {
                "status": "conflicting",
                "reason": (
                    component_status["reason"]
                    if not component_status["usable"]
                    else "candidate_evidence_drift"
                ),
                "native_support_status": str(native_status or "unverified"),
            }
        return {
            "status": fallback,
            "reason": "competing_candidates" if candidate.conflict_size > 1 else None,
        }
    if review.get("action") == "dismissed":
        return {"status": "dismissed", "reason": None}
    if review.get("action") == "resolved":
        component_status = _component_review_status(
            conn, str(review.get("component_id") or "")
        )
        return {
            "status": "resolved" if component_status["usable"] else "conflicting",
            "reason": None if component_status["usable"] else component_status["reason"],
            "native_support_status": component_status.get(
                "native_support_status", "unverified"
            ),
        }
    return {
        "status": fallback,
        "reason": "competing_candidates" if candidate.conflict_size > 1 else None,
    }


def review_status(
    conn: sqlite3.Connection,
    candidate: CustodyGapCandidate,
    review: Mapping[str, Any] | None,
) -> str:
    return str(review_state(conn, candidate, review)["status"])


def latest_dismissed_fingerprints(
    conn: sqlite3.Connection, profile_id: str
) -> dict[str, str]:
    """Return unambiguous latest dismissal fingerprints for runtime input."""

    return {
        gap_id: str(review["candidate_fingerprint"])
        for gap_id, review in latest_reviews(conn, profile_id).items()
        if review.get("action") == "dismissed"
        and _event_kind(review) != "bridge_reopened"
    }


def _profile_input_version(conn: sqlite3.Connection, profile_id: str) -> int:
    row = conn.execute(
        "SELECT journal_input_version FROM profiles WHERE id = ?", (profile_id,)
    ).fetchone()
    if not row:
        raise AppError("Profile was not found", code="not_found")
    return int(row["journal_input_version"] or 0)


def _stable_plan_rows(
    spec: Mapping[str, Any], component_id: str
) -> dict[str, Any]:
    """Give every prospective immutable row a deterministic component-local id."""

    planned = dict(spec)
    raw_legs = [dict(leg) for leg in spec.get("legs", ())]
    leg_ids: dict[str, str] = {}
    for ordinal, leg in enumerate(raw_legs):
        old_id = str(leg.get("id") or f"leg:{ordinal}")
        new_id = str(
            uuid.uuid5(
                uuid.NAMESPACE_URL,
                f"kassiber:custody-plan:{component_id}:leg:{ordinal}",
            )
        )
        leg_ids[old_id] = new_id
        leg["id"] = new_id
    raw_allocations = [dict(row) for row in spec.get("allocations", ())]
    for ordinal, allocation in enumerate(raw_allocations):
        allocation["id"] = str(
            uuid.uuid5(
                uuid.NAMESPACE_URL,
                f"kassiber:custody-plan:{component_id}:allocation:{ordinal}",
            )
        )
        for endpoint in ("source", "sink"):
            key = f"{endpoint}_leg_id"
            try:
                allocation[key] = leg_ids[str(allocation[key])]
            except KeyError as exc:
                raise AppError(
                    "Custody review plan references an unknown leg",
                    code="custody_component_validation",
                    details={"allocation_ordinal": ordinal, "endpoint": endpoint},
                ) from exc
    planned["component_id"] = component_id
    planned["legs"] = raw_legs
    planned["allocations"] = raw_allocations
    return planned


def _plan_hash(payload: Mapping[str, Any]) -> str:
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _component_plan(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    spec: Mapping[str, Any],
    component_id: str,
    replacing_lineage_id: str | None = None,
) -> dict[str, Any]:
    planned = _stable_plan_rows(spec, component_id)
    checked = custody_components.validate_component_plan(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        component_type=str(planned["component_type"]),
        legs=planned["legs"],
        allocations=planned["allocations"],
        conservation_mode=str(planned.get("conservation_mode") or "quantity"),
        conversion_policy=planned.get("conversion_policy"),
        conversion_reviewed=bool(planned.get("conversion_reviewed", False)),
        evidence_grade=planned.get("evidence_grade"),
        replacing_lineage_id=replacing_lineage_id,
    )
    planned["legs"] = checked["legs"]
    planned["allocations"] = checked["allocations"]
    planned["validation"] = checked["validation"]
    return planned


def plan_review(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    candidate: CustodyGapCandidate | None = None,
    gap_id: str | None = None,
    classification: str | None = None,
    reason: str | None = None,
    authored_source: str = "user",
) -> dict[str, Any]:
    """Build the exact custody review mutation using database reads only."""

    input_version = _profile_input_version(conn, profile_id)
    component_plan: dict[str, Any] | None = None
    context: dict[str, Any] | None = None
    residual_msat = 0
    activatable = True
    new_component_revision: int | None = None
    authored_fingerprint: str | None = None
    warnings: list[str] = []
    if action in {"create", "revise", "dismiss"}:
        if candidate is None:
            raise AppError("Custody-gap candidate is required", code="invalid_argument")
        candidate = _require_current_candidate(conn, candidate)
        gap_id = candidate.gap_id
        authored_fingerprint = authored_claim_fingerprint(candidate)
        warnings = list(_guided_bridge_warnings(candidate))
        if action == "dismiss":
            base_fingerprint = authored_fingerprint
            impacts = _preview_filed_report_impacts(conn, candidate)
        elif action == "create":
            base_fingerprint = authored_fingerprint
            new_component_revision = 1
            if candidate.excess_msat:
                spec = None
                component_id = ""
                activatable = False
            else:
                spec = _guided_component_spec(
                    conn, candidate, authored_fingerprint, authored_source
                )
                component_id = str(spec["component_id"])
            replacing_lineage_id = None
        else:
            context = _review_context(
                conn, profile_id, candidate.gap_id, expected_state="reopened"
            )
            base_fingerprint = _correction_fingerprint(
                conn,
                context,
                operation="bridge_revised",
                details={
                    "authored_claim_fingerprint": authored_fingerprint,
                    "reason": reason or "",
                },
            )
            spec = _guided_component_spec(
                conn, candidate, authored_fingerprint, authored_source
            )
            component_id = str(
                uuid.uuid5(
                    uuid.NAMESPACE_URL,
                    f"kassiber:custody-gap-revision:{base_fingerprint}",
                )
            )
            replacing_lineage_id = str(context["component"]["lineage_id"])
            new_component_revision = _next_component_revision(conn, context)
        if action != "dismiss" and spec is not None:
            component_plan = _component_plan(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                spec=spec,
                component_id=component_id,
                replacing_lineage_id=replacing_lineage_id,
            )
            activatable = bool(component_plan["validation"].get("activatable"))
        if action != "dismiss":
            impacts = _preview_filed_report_impacts(conn, candidate)
    elif action == "reopen":
        if not gap_id:
            raise AppError("Custody-gap id is required", code="invalid_argument")
        context = _review_context(conn, profile_id, gap_id, expected_state="active")
        base_fingerprint = _correction_fingerprint(
            conn,
            context,
            operation="bridge_reopened",
            details={"reason": reason or ""},
        )
        impacts = _preview_snapshot_impacts(
            conn,
            profile_id,
            context["snapshot"],
            after_classification_summary=_reopened_classification_summary(
                context["snapshot"]
            ),
        )
    elif action == "classify_residual":
        if not gap_id:
            raise AppError("Custody-gap id is required", code="invalid_argument")
        normalized = _normalize_residual_classification(classification or "")
        classification = normalized
        context = _review_context(conn, profile_id, gap_id, expected_state="active")
        new_component_revision = _next_component_revision(conn, context)
        base_fingerprint = _correction_fingerprint(
            conn,
            context,
            operation="residual_classified",
            details={"classification": normalized, "reason": reason or ""},
        )
        spec, residual_msat = _residual_revision_spec(
            context=context,
            classification=normalized,
            authored_source=authored_source,
        )
        component_id = str(
            uuid.uuid5(
                uuid.NAMESPACE_URL,
                f"kassiber:custody-gap-residual:{base_fingerprint}",
            )
        )
        component_plan = _component_plan(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            spec=spec,
            component_id=component_id,
            replacing_lineage_id=str(context["component"]["lineage_id"]),
        )
        activatable = bool(component_plan["validation"].get("activatable"))
        impacts = _preview_snapshot_impacts(
            conn,
            profile_id,
            context["snapshot"],
            after_classification_summary=_residual_classification_summary(
                context["snapshot"], normalized, residual_msat
            ),
        )
    else:
        raise AppError(
            "Custody review action is unsupported",
            code="invalid_argument",
            details={"action": action},
        )

    commitment = {
        "schema_version": 1,
        "action": action,
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "gap_id": gap_id,
        "input_version": input_version,
        "base_fingerprint": base_fingerprint,
        "classification": classification,
        "reason": reason or "",
        "authored_source": authored_source,
        "new_component_revision": new_component_revision,
        "component_plan": component_plan,
        "filed_report_impacts": impacts,
    }
    fingerprint = _plan_hash(commitment)
    return {
        **commitment,
        "fingerprint": fingerprint,
        "candidate": candidate,
        "context": context,
        "authored_claim_fingerprint": authored_fingerprint,
        "component_plan": component_plan,
        "activatable": activatable,
        "warnings": warnings,
        "residual_msat": residual_msat,
    }


def public_review_plan(plan: Mapping[str, Any]) -> dict[str, Any]:
    """Return the serializable, privacy-safe review contract shared by all clients."""

    component = plan.get("component_plan") or {}
    public_component = None
    if component:
        validation = component.get("validation") or {}
        public_component = {
            "id": component.get("component_id"),
            "component_type": component.get("component_type"),
            "conservation_mode": component.get("conservation_mode"),
            "legs": [
                {
                    key: leg.get(key)
                    for key in (
                        "id",
                        "role",
                        "rail",
                        "asset",
                        "exposure",
                        "conservation_unit",
                        "amount_msat",
                        "valuation_unit",
                        "valuation_amount",
                    )
                }
                for leg in component.get("legs", ())
            ],
            "allocations": [
                {
                    key: allocation.get(key)
                    for key in (
                        "id",
                        "source_leg_id",
                        "sink_leg_id",
                        "source_amount_msat",
                        "sink_amount_msat",
                    )
                }
                for allocation in component.get("allocations", ())
            ],
            "validation": {
                "activatable": bool(validation.get("activatable")),
                "issues": [
                    {
                        key: issue.get(key)
                        for key in ("code", "message")
                        if issue.get(key) is not None
                    }
                    for issue in validation.get("issues", ())
                ],
                "warnings": [
                    {
                        key: warning.get(key)
                        for key in ("code", "message")
                        if warning.get(key) is not None
                    }
                    for warning in validation.get("warnings", ())
                ],
            },
        }
    candidate = plan.get("candidate")
    action = str(plan["action"])
    return {
        "action": action,
        "gap_id": plan["gap_id"],
        "fingerprint": plan["fingerprint"],
        "input_version": plan["input_version"],
        "dry_run": True,
        "activatable": plan["activatable"],
        "requires_explicit_confirmation": True,
        "classification": plan.get("classification"),
        "country_tax_meaning": (
            "not_assigned" if action == "classify_residual" else None
        ),
        "custody_state": (
            residual_custody_state(str(plan["classification"]))
            if action == "classify_residual"
            else None
        ),
        "new_component_revision": plan.get("new_component_revision"),
        "current_component_revision": (
            (plan.get("context") or {}).get("component", {}).get("revision")
        ),
        "review_mode": (
            "structured_candidate"
            if candidate is not None
            and candidate.promotion_eligible
            and candidate.conflict_size == 1
            else ("manual_weak_hint" if candidate is not None else None)
        ),
        "retained_msat": candidate.retained_msat if candidate is not None else 0,
        "residual_msat": (
            candidate.residual_msat
            if candidate is not None
            else plan.get("residual_msat", 0)
        ),
        "fee_msat": candidate.source_fee_msat if candidate is not None else 0,
        "source_count": len(candidate.source_ids) if candidate is not None else 0,
        "destination_count": (
            len(candidate.return_ids) if candidate is not None else 0
        ),
        "warnings": plan["warnings"],
        "filed_report_impacts": plan["filed_report_impacts"],
        "component_plan": public_component,
    }


def _persist_component_plan(
    conn: sqlite3.Connection, plan: Mapping[str, Any]
) -> dict[str, Any]:
    spec = dict(plan.get("component_plan") or {})
    candidate = plan.get("candidate")
    if (
        plan.get("action") == "create"
        and candidate is not None
        and int(getattr(candidate, "excess_msat", 0) or 0) > 0
    ):
        raise AppError(
            "The return exceeds the source principal",
            code="custody_gap_bridge_excess_return",
            hint="Classify the excess origin separately, then preview again.",
            details={"excess_msat": int(candidate.excess_msat)},
        )
    validation = spec.pop("validation", {})
    if not validation.get("activatable"):
        raise AppError(
            "Custody review plan is not activatable",
            code="custody_component_incomplete",
            details={"validation": validation},
        )
    component_id = str(spec.pop("component_id"))
    context = plan.get("context")
    if plan["action"] == "create":
        component = custody_components.create_component(
            conn,
            workspace_id=str(plan["workspace_id"]),
            profile_id=str(plan["profile_id"]),
            component_id=component_id,
            **spec,
        )
    else:
        if not isinstance(context, Mapping):
            raise AppError("Custody review context is missing", code="custody_gap_stale")
        allowed = {
            key: value
            for key, value in spec.items()
            if key
            in {
                "legs",
                "allocations",
                "component_type",
                "conservation_mode",
                "evidence_kind",
                "evidence_grade",
                "evidence",
                "conversion_policy",
                "conversion_reviewed",
                "conversion_metadata",
                "notes",
                "authored_source",
            }
        }
        component = custody_components.update_component(
            conn,
            str(context["component"]["id"]),
            new_component_id=component_id,
            change_reason=str(plan.get("reason") or plan["action"]),
            preserve_planned_row_ids=True,
            **allowed,
        )
    return custody_components.activate_component(conn, component["id"])


def apply_review(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    expected_fingerprint: str,
    candidate: CustodyGapCandidate | None = None,
    gap_id: str | None = None,
    classification: str | None = None,
    reason: str | None = None,
    authored_source: str = "user",
    commit: bool = True,
) -> dict[str, Any]:
    """Atomically replan, mutate, append audit history and record impacts."""

    conn.execute("SAVEPOINT custody_gap_review_apply")
    try:
        plan = plan_review(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            action=action,
            candidate=candidate,
            gap_id=gap_id,
            classification=classification,
            reason=reason,
            authored_source=authored_source,
        )
        _require_expected_correction(expected_fingerprint, plan["fingerprint"])
        context = plan.get("context")
        if action == "dismiss":
            current_candidate = plan["candidate"]
            review = _append_review(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                candidate=current_candidate,
                fingerprint=candidate_fingerprint(current_candidate),
                action="dismissed",
                component_id=None,
                authored_source=authored_source,
                reason=reason,
                event_kind="review_decision",
            )
            _invalidate_journals(conn, profile_id)
            result = {
                **_public_review(review),
                "filed_report_impacts": plan["filed_report_impacts"],
            }
        elif action == "reopen":
            component = custody_components.supersede_component(
                conn,
                context["component"]["id"],
                reason=reason or "guided_bridge_reopened",
            )
        elif action != "dismiss":
            component = _persist_component_plan(conn, plan)

        if action in {"create", "revise"}:
            current_candidate = plan["candidate"]
            event_kind = "bridge_created" if action == "create" else "bridge_revised"
            review = _append_review(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                candidate=current_candidate,
                fingerprint=plan["authored_claim_fingerprint"],
                action="resolved",
                component_id=component["id"],
                authored_source=authored_source,
                reason=(
                    "guided_custody_bridge"
                    if action == "create"
                    else reason or "guided_bridge_revised"
                ),
                event_kind=event_kind,
            )
            impacts = custody_filed_reports.append_custody_impacts(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                component_id=component["id"],
                review_id=review["id"],
                gap_id=current_candidate.gap_id,
                candidate=current_candidate,
                downstream_years=_downstream_affected_years(
                    conn, current_candidate
                ),
            )
            result = {
                "gap_id": current_candidate.gap_id,
                "status": "resolved",
                "component_id": component["id"],
                "component_revision": component["revision"],
                "review_id": review["id"],
                "review_revision": review["revision"],
                "retained_msat": current_candidate.retained_msat,
                "residual_msat": current_candidate.residual_msat,
                "filed_report_impacts": impacts,
            }
            if action == "create":
                result["fee_msat"] = current_candidate.source_fee_msat
        elif action == "reopen":
            snapshot = dict(context["snapshot"])
            snapshot["status"] = "needs_review"
            snapshot["correction"] = {
                "strategy": "create_revision_then_activate",
                "event_kind": "bridge_reopened",
                "plan_fingerprint": plan["fingerprint"],
                "correction_fingerprint": plan["base_fingerprint"],
            }
            review = _append_review_snapshot(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                gap_id=str(plan["gap_id"]),
                fingerprint=plan["base_fingerprint"],
                action="dismissed",
                event_kind="bridge_reopened",
                component_id=component["id"],
                authored_source=authored_source,
                reason=reason or "guided_bridge_reopened",
                snapshot=snapshot,
            )
            impacts = _append_snapshot_impacts(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                component_id=component["id"],
                review_id=review["id"],
                gap_id=str(plan["gap_id"]),
                snapshot=snapshot,
                after_classification_summary=_reopened_classification_summary(
                    snapshot
                ),
            )
            result = {
                "gap_id": plan["gap_id"],
                "status": "needs_review",
                "component_id": component["id"],
                "component_revision": component["revision"],
                "review_id": review["id"],
                "review_revision": review["revision"],
                "filed_report_impacts": impacts,
            }
        elif action == "classify_residual":
            normalized = str(plan["classification"])
            residual_msat = int(plan["residual_msat"])
            snapshot = dict(context["snapshot"])
            snapshot["plan_fingerprint"] = plan["fingerprint"]
            snapshot["correction_fingerprint"] = plan["base_fingerprint"]
            snapshot["residual_classification"] = {
                "classification": normalized,
                "custody_state": _residual_custody_state(normalized),
                "country_tax_meaning": "not_assigned",
                "amount_msat": residual_msat,
            }
            review = _append_review_snapshot(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                gap_id=str(plan["gap_id"]),
                fingerprint=str(context["review"]["candidate_fingerprint"]),
                action="resolved",
                event_kind="residual_classified",
                component_id=component["id"],
                authored_source=authored_source,
                reason=reason or f"reviewed_{normalized}",
                snapshot=snapshot,
            )
            impacts = _append_snapshot_impacts(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                component_id=component["id"],
                review_id=review["id"],
                gap_id=str(plan["gap_id"]),
                snapshot=snapshot,
                after_classification_summary=_residual_classification_summary(
                    snapshot, normalized, residual_msat
                ),
            )
            result = {
                "gap_id": plan["gap_id"],
                "status": "resolved",
                "classification": normalized,
                "custody_state": _residual_custody_state(normalized),
                "country_tax_meaning": "not_assigned",
                "residual_msat": residual_msat,
                "component_id": component["id"],
                "component_revision": component["revision"],
                "review_id": review["id"],
                "review_revision": review["revision"],
                "filed_report_impacts": impacts,
            }
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT custody_gap_review_apply")
        conn.execute("RELEASE SAVEPOINT custody_gap_review_apply")
        raise
    conn.execute("RELEASE SAVEPOINT custody_gap_review_apply")
    if commit:
        conn.commit()
    return result


def list_review_history(
    conn: sqlite3.Connection,
    profile_id: str,
    gap_id: str,
    *,
    limit: int = 100,
) -> dict[str, Any]:
    """Return bounded append-only review history without boundary txids."""

    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError(
            "Custody review history limit must be between 1 and 500",
            code="custody_gap_review_validation",
        )
    rows = conn.execute(
        """
        SELECT r.*, c.revision AS component_revision,
               (SELECT COUNT(*) FROM custody_filed_report_impacts i
                WHERE i.review_id = r.id) AS filed_report_impact_count
        FROM custody_gap_reviews r
        LEFT JOIN custody_components c ON c.id = r.component_id
        WHERE r.profile_id = ? AND r.gap_id = ?
        ORDER BY r.revision DESC, r.created_at DESC, r.id DESC
        LIMIT ?
        """,
        (profile_id, gap_id, limit),
    ).fetchall()
    history = [_redacted_review_history_row(row) for row in reversed(rows)]
    return {"gap_id": gap_id, "count": len(history), "history": history}


def list_audit_review_history(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    transaction_ids: Sequence[str] | None = None,
    limit: int = 500,
) -> dict[str, Any]:
    """Return bounded review facts for auditor handoff, never raw snapshots.

    A transaction-scoped package includes reviews with a normalized authored
    boundary anchor in the selected set. Legacy component reviews without a
    normalized row may still use their durable leg anchor. Raw candidate
    snapshots are never used to decide transaction scope.
    """

    if type(limit) is not int or not 1 <= limit <= 500:
        raise AppError(
            "Custody audit review history limit must be between 1 and 500",
            code="custody_gap_review_validation",
        )
    incomplete_review_ids = _incomplete_review_scope_ids(conn, profile_id)
    where = "r.profile_id = ?"
    params: list[Any] = [profile_id]
    selected_transaction_ids: frozenset[str] | None = None
    if transaction_ids is not None:
        selected = tuple(sorted({str(value) for value in transaction_ids if value}))
        if not selected:
            return _audit_review_history_result(
                count=0,
                records=[],
                bounded=True,
                incomplete_review_ids=incomplete_review_ids,
            )
        selected_transaction_ids = frozenset(selected)
        placeholders = ",".join("?" for _ in selected)
        where += f"""
            AND (
                EXISTS (
                    SELECT 1 FROM custody_gap_review_transactions x
                    WHERE x.review_id = r.id
                      AND x.profile_id = r.profile_id
                      AND x.transaction_id IN ({placeholders})
                )
                OR (
                    NOT EXISTS (
                        SELECT 1 FROM custody_gap_review_transactions x
                        WHERE x.review_id = r.id
                    )
                    AND r.component_id IS NOT NULL
                    AND EXISTS (
                        SELECT 1 FROM custody_component_legs l
                        WHERE l.profile_id = r.profile_id
                          AND l.component_id = r.component_id
                          AND COALESCE(l.anchor_transaction_id, l.transaction_id)
                              IN ({placeholders})
                    )
                )
            )
        """
        params.extend(selected)
        params.extend(selected)
    count = int(
        conn.execute(
            f"SELECT COUNT(*) FROM custody_gap_reviews r WHERE {where}",
            params,
        ).fetchone()[0]
    )
    rows = conn.execute(
        f"""
        SELECT r.*, c.revision AS component_revision,
               (SELECT COUNT(*) FROM custody_filed_report_impacts i
                WHERE i.review_id = r.id) AS filed_report_impact_count
        FROM custody_gap_reviews r
        LEFT JOIN custody_components c ON c.id = r.component_id
        WHERE {where}
        ORDER BY r.created_at, r.id
        LIMIT ?
        """,
        (*params, limit),
    ).fetchall()
    fully_selected_review_ids: frozenset[str] = frozenset()
    if selected_transaction_ids is not None and rows:
        fully_selected_review_ids = complete_selected_review_ids(
            conn,
            profile_id,
            tuple(selected_transaction_ids),
            review_ids=tuple(str(row["id"]) for row in rows),
        )
    records = [
        _redacted_review_history_row(
            row,
            include_gap_id=True,
            include_candidate_wide_payload=(
                selected_transaction_ids is None
                or str(row["id"]) in fully_selected_review_ids
            ),
        )
        for row in rows
    ]
    return _audit_review_history_result(
        count=count,
        records=records,
        bounded=selected_transaction_ids is not None,
        incomplete_review_ids=incomplete_review_ids,
    )


def _audit_review_history_result(
    *,
    count: int,
    records: list[dict[str, Any]],
    bounded: bool,
    incomplete_review_ids: set[str],
) -> dict[str, Any]:
    incomplete = bool(incomplete_review_ids)
    result: dict[str, Any] = {
        "count": count,
        "returned": len(records),
        "truncated": count > len(records),
        "scope_completeness": (
            "legacy_unscoped_history_present" if incomplete else "complete"
        ),
        "records": records,
    }
    # A bounded handoff must acknowledge that older history cannot be scoped,
    # but enumerating the profile-wide count would leak unrelated book state.
    if not bounded:
        result["legacy_unscoped_review_count"] = len(incomplete_review_ids)
    return result


def _incomplete_review_scope_ids(
    conn: sqlite3.Connection, profile_id: str
) -> set[str]:
    """Identify reviews without an exactly satisfied durable boundary header."""

    rows = conn.execute(
        """
        SELECT r.id,
               s.expected_source_count,
               s.expected_return_count,
               SUM(CASE WHEN x.role = 'source' THEN 1 ELSE 0 END)
                   AS actual_source_count,
               SUM(CASE WHEN x.role = 'return' THEN 1 ELSE 0 END)
                   AS actual_return_count
        FROM custody_gap_reviews r
        LEFT JOIN custody_gap_review_relation_sets s ON s.review_id = r.id
        LEFT JOIN custody_gap_review_transactions x ON x.review_id = r.id
        WHERE r.profile_id = ?
        GROUP BY r.id, s.expected_source_count, s.expected_return_count
        ORDER BY r.created_at, r.id
        """,
        (profile_id,),
    ).fetchall()
    return {
        str(row["id"])
        for row in rows
        if row["expected_source_count"] is None
        or row["expected_return_count"] is None
        or int(row["expected_source_count"]) <= 0
        or int(row["expected_return_count"]) <= 0
        or int(row["expected_source_count"]) != int(row["actual_source_count"])
        or int(row["expected_return_count"]) != int(row["actual_return_count"])
    }


def complete_selected_review_ids(
    conn: sqlite3.Connection,
    profile_id: str,
    transaction_ids: Sequence[str],
    *,
    review_ids: Sequence[str] | None = None,
) -> frozenset[str]:
    """Return exactly complete review boundaries contained in a selection.

    This is deliberately read-only.  A signed relation prefix cannot unlock a
    candidate-wide payload: a review qualifies only when its immutable header
    is present, each role count matches exactly, the boundary is non-empty,
    and every authored transaction anchor is selected.
    """

    selected = frozenset(str(value) for value in transaction_ids if value)
    if not selected:
        return frozenset()
    where = "r.profile_id = ?"
    params: list[Any] = [profile_id]
    if review_ids is not None:
        candidates = tuple(sorted({str(value) for value in review_ids if value}))
        if not candidates:
            return frozenset()
        placeholders = ",".join("?" for _ in candidates)
        where += f" AND r.id IN ({placeholders})"
        params.extend(candidates)
    rows = conn.execute(
        f"""
        SELECT r.id, s.expected_source_count, s.expected_return_count,
               x.role, x.transaction_id
        FROM custody_gap_reviews r
        JOIN custody_gap_review_relation_sets s ON s.review_id = r.id
        LEFT JOIN custody_gap_review_transactions x ON x.review_id = r.id
        WHERE {where}
        ORDER BY r.id, x.role, x.transaction_id, x.id
        """,
        params,
    ).fetchall()
    state: dict[str, dict[str, Any]] = {}
    for row in rows:
        review_id = str(row["id"])
        current = state.setdefault(
            review_id,
            {
                "expected_source": int(row["expected_source_count"]),
                "expected_return": int(row["expected_return_count"]),
                "source": set(),
                "return": set(),
            },
        )
        if row["role"] is not None and row["transaction_id"] is not None:
            current[str(row["role"])].add(str(row["transaction_id"]))
    complete: set[str] = set()
    for review_id, current in state.items():
        boundary = current["source"] | current["return"]
        if (
            current["expected_source"] > 0
            and current["expected_return"] > 0
            and len(current["source"]) == current["expected_source"]
            and len(current["return"]) == current["expected_return"]
            and boundary.issubset(selected)
        ):
            complete.add(review_id)
    return frozenset(complete)


def backfill_legacy_componentless_review_relations(
    conn: sqlite3.Connection,
) -> int:
    """Recover old dismissal anchors only from an exact current candidate.

    Early review snapshots intentionally omitted boundary transaction ids. A
    stored fingerprint is not reversible, so recovery re-runs the deterministic
    matcher and requires both the stable gap id and the complete candidate
    fingerprint to match.  Changed or unavailable evidence stays explicitly
    unscoped. This migration is invoked after schema compatibility work, never
    from audit/AI read paths.
    """

    pending = conn.execute(
        """
        SELECT r.id, r.workspace_id, r.profile_id, r.gap_id,
               r.candidate_fingerprint, r.created_at
        FROM custody_gap_reviews r
        WHERE r.component_id IS NULL
          AND r.action = 'dismissed'
          AND COALESCE(r.event_kind, 'review_decision') = 'review_decision'
        ORDER BY r.profile_id, r.created_at, r.id
        """
    ).fetchall()
    if not pending:
        return 0

    from .custody_gaps import load_gap_search_result

    inserted = 0
    by_profile: dict[str, list[Mapping[str, Any]]] = {}
    for row in pending:
        by_profile.setdefault(str(row["profile_id"]), []).append(row)
    for profile_id, reviews in by_profile.items():
        search_result, _ = load_gap_search_result(
            conn,
            profile_id,
            include_journal_claims=False,
        )
        candidates = {
            candidate.gap_id: candidate
            for candidate in (
                *search_result.candidates,
                *search_result.accounting_candidates,
            )
        }.values()
        exact = {
            (candidate.gap_id, candidate_fingerprint(candidate)): candidate
            for candidate in candidates
        }
        for review in reviews:
            candidate = exact.get(
                (str(review["gap_id"]), str(review["candidate_fingerprint"]))
            )
            if candidate is None:
                continue
            inserted += backfill_custody_gap_review_relations(
                conn,
                review_id=review["id"],
                workspace_id=review["workspace_id"],
                profile_id=review["profile_id"],
                created_at=review["created_at"],
                relations=(
                    *(("source", value) for value in candidate.source_ids),
                    *(("return", value) for value in candidate.return_ids),
                ),
            )
            backfill_custody_gap_review_relation_set(
                conn,
                review_id=review["id"],
                workspace_id=review["workspace_id"],
                profile_id=review["profile_id"],
                created_at=review["created_at"],
                expected_source_count=len(set(candidate.source_ids)),
                expected_return_count=len(set(candidate.return_ids)),
            )
    return inserted


def _redacted_review_history_row(
    row: Mapping[str, Any],
    *,
    include_gap_id: bool = False,
    include_candidate_wide_payload: bool = True,
) -> dict[str, Any]:
    event_kind = str(row["event_kind"] or "review_decision")
    status = (
        "needs_review"
        if event_kind == "bridge_reopened"
        else ("resolved" if row["action"] == "resolved" else "dismissed")
    )
    output = {
        "revision": int(row["revision"]),
        "event_kind": event_kind,
        "status": status,
        "component_id": row["component_id"],
        "component_revision": (
            int(row["component_revision"])
            if row["component_revision"] is not None
            else None
        ),
        "authored_source": row["authored_source"],
        "created_at": row["created_at"],
    }
    if include_candidate_wide_payload:
        try:
            snapshot = json.loads(str(row["snapshot_json"] or "{}"))
        except (TypeError, ValueError, json.JSONDecodeError):
            snapshot = {}
        residual = snapshot.get("residual_classification")
        output.update(
            {
                "reason": row["reason"],
                "retained_msat": int(snapshot.get("retained_msat") or 0),
                "residual_msat": int(snapshot.get("residual_msat") or 0),
                "residual_classification": (
                    str(residual.get("classification"))
                    if isinstance(residual, Mapping)
                    and residual.get("classification")
                    else None
                ),
                "filed_report_impact_count": int(
                    row["filed_report_impact_count"] or 0
                ),
            }
        )
    else:
        output.update(
            {
                "candidate_wide_payload_excluded": True,
                "excluded_fields": [
                    "reason",
                    "retained_msat",
                    "residual_msat",
                    "residual_classification",
                    "filed_report_impact_count",
                ],
            }
        )
    if include_gap_id:
        output["gap_id"] = row["gap_id"]
    return output


def historical_review_gaps(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    exclude_gap_ids: Sequence[str] = (),
) -> list[dict[str, Any]]:
    excluded = set(exclude_gap_ids)
    output: list[dict[str, Any]] = []
    for gap_id, review in latest_reviews(conn, profile_id).items():
        if gap_id in excluded:
            continue
        try:
            snapshot = json.loads(str(review.get("snapshot_json") or "{}"))
        except ValueError:
            continue
        if not isinstance(snapshot, dict):
            continue
        if review.get("action") == "conflicting_review":
            snapshot["status"] = "conflicting"
            snapshot["status_reason"] = "concurrent_review_conflict"
        elif _event_kind(review) == "bridge_reopened":
            snapshot["status"] = "needs_review"
            snapshot["status_reason"] = "bridge_reopened"
            snapshot["correction"] = {
                "component_id": str(review.get("component_id") or ""),
                "strategy": "create_revision_then_activate",
            }
        elif review.get("action") == "resolved":
            component_status = _component_review_status(
                conn, str(review.get("component_id") or "")
            )
            snapshot["status"] = (
                "resolved" if component_status["usable"] else "conflicting"
            )
            if not component_status["usable"]:
                snapshot["status_reason"] = component_status["reason"]
            snapshot["native_support_status"] = component_status.get(
                "native_support_status", "unverified"
            )
            snapshot["correction"] = {
                "component_id": str(review.get("component_id") or ""),
                "strategy": "create_revision_then_activate",
            }
        elif review.get("action") == "dismissed":
            snapshot["status"] = "dismissed"
        else:
            continue
        output.append(snapshot)
    return output


def _review_context(
    conn: sqlite3.Connection,
    profile_id: str,
    gap_id: str,
    *,
    expected_state: str,
) -> dict[str, Any]:
    rows = conn.execute(
        """
        SELECT * FROM custody_gap_reviews
        WHERE profile_id = ? AND gap_id = ?
          AND revision = (
              SELECT MAX(revision) FROM custody_gap_reviews
              WHERE profile_id = ? AND gap_id = ?
          )
        ORDER BY created_at, id
        """,
        (profile_id, gap_id, profile_id, gap_id),
    ).fetchall()
    if len(rows) != 1:
        raise AppError(
            "Custody bridge review history is missing or conflicting",
            code="custody_gap_review_conflict",
            hint="Resolve concurrent review history before correcting this bridge.",
        )
    review = dict(rows[0])
    component_id = str(review.get("component_id") or "")
    if not component_id:
        raise AppError(
            "Custody bridge component is missing",
            code="custody_gap_review_conflict",
        )
    try:
        component = custody_components.get_component(conn, component_id)
    except AppError as exc:
        raise AppError(
            "Custody bridge component is missing",
            code="custody_gap_review_conflict",
        ) from exc
    from .custody_quantity_store import component_evidence_status

    evidence = component_evidence_status(conn, component)
    if not evidence.get("valid"):
        raise AppError(
            "Custody bridge evidence changed after review",
            code="custody_gap_stale",
            hint="Re-import or restore the reviewed evidence before correcting it.",
            details={"evidence_status": evidence.get("status")},
        )
    event_kind = _event_kind(review)
    if expected_state == "active":
        valid_state = (
            review.get("action") == "resolved"
            and event_kind != "bridge_reopened"
            and component.get("effective_state") == "active"
        )
    elif expected_state == "reopened":
        valid_state = (
            event_kind == "bridge_reopened"
            and component.get("state") == "superseded"
        )
    else:
        raise AssertionError(f"unsupported review context state {expected_state}")
    if not valid_state:
        raise AppError(
            "Custody bridge is not in the required correction state",
            code="custody_gap_review_state",
            details={
                "required_state": expected_state,
                "event_kind": event_kind,
                "component_state": component.get("effective_state"),
            },
        )
    try:
        snapshot = json.loads(str(review.get("snapshot_json") or "{}"))
    except (TypeError, ValueError, json.JSONDecodeError) as exc:
        raise AppError(
            "Custody bridge review snapshot is invalid",
            code="custody_gap_review_conflict",
        ) from exc
    if not isinstance(snapshot, dict):
        raise AppError(
            "Custody bridge review snapshot is invalid",
            code="custody_gap_review_conflict",
        )
    return {
        "review": review,
        "component": component,
        "evidence": evidence,
        "snapshot": snapshot,
    }


def _correction_fingerprint(
    conn: sqlite3.Connection,
    context: Mapping[str, Any],
    *,
    operation: str,
    details: Mapping[str, Any],
) -> str:
    component = context["component"]
    commitments = [
        {
            "ordinal": int(row["ordinal"]),
            "quantity_hash": str(row["quantity_hash"]),
            "detail_hash": str(row["detail_hash"]),
        }
        for row in conn.execute(
            """
            SELECT ordinal, quantity_hash, detail_hash
            FROM custody_component_evidence_commitments
            WHERE component_id = ?
            ORDER BY ordinal, id
            """,
            (component["id"],),
        ).fetchall()
    ]
    payload = {
        "schema_version": 1,
        "operation": operation,
        "gap_id": context["review"]["gap_id"],
        "review_id": context["review"]["id"],
        "review_revision": int(context["review"]["revision"]),
        "review_event_kind": _event_kind(context["review"]),
        "component_id": component["id"],
        "component_lineage_id": component["lineage_id"],
        "component_revision": int(component["revision"]),
        "evidence_commitments": commitments,
        "details": dict(details),
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(encoded.encode("utf-8")).hexdigest()


def _next_component_revision(
    conn: sqlite3.Connection, context: Mapping[str, Any]
) -> int:
    component = context["component"]
    return int(
        conn.execute(
            "SELECT COALESCE(MAX(revision), 0) + 1 "
            "FROM custody_components WHERE profile_id = ? AND lineage_id = ?",
            (component["profile_id"], component["lineage_id"]),
        ).fetchone()[0]
    )


def _require_expected_correction(expected: str, actual: str) -> None:
    if not isinstance(expected, str) or expected != actual:
        raise AppError(
            "Custody bridge evidence changed after preview",
            code="custody_gap_stale",
            hint="Run the exact preview again before confirming this correction.",
        )


def _component_revision_inputs(
    component: Mapping[str, Any],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    leg_fields = {
        "id",
        "role",
        "rail",
        "chain",
        "network",
        "asset",
        "exposure",
        "conservation_unit",
        "amount_msat",
        "valuation_unit",
        "valuation_amount",
        "occurred_at",
        "transaction_id",
        "anchor_transaction_id",
        "wallet_id",
        "location_ref",
        "notes",
    }
    legs = [
        {key: value for key, value in leg.items() if key in leg_fields}
        for leg in component.get("legs", ())
    ]
    allocations = [
        {
            key: allocation[key]
            for key in (
                "source_leg_id",
                "sink_leg_id",
                "source_amount_msat",
                "sink_amount_msat",
            )
        }
        for allocation in component.get("allocations", ())
    ]
    return legs, allocations


def _normalize_residual_classification(value: str) -> str:
    normalized = str(value or "").strip().lower().replace("-", "_")
    if normalized not in RESIDUAL_CLASSIFICATIONS:
        raise AppError(
            "Residual classification is unsupported",
            code="custody_gap_residual_classification",
            details={"supported": sorted(RESIDUAL_CLASSIFICATIONS)},
        )
    return normalized


def _residual_revision_spec(
    *,
    context: Mapping[str, Any],
    classification: str,
    authored_source: str,
) -> tuple[dict[str, Any], int]:
    old_component = context["component"]
    legs, allocations = _component_revision_inputs(old_component)
    suspense_legs = [
        leg
        for leg in legs
        if leg.get("role") == "suspense" and int(leg.get("amount_msat") or 0) > 0
    ]
    residual_msat = sum(int(leg["amount_msat"]) for leg in suspense_legs)
    if residual_msat <= 0:
        raise AppError(
            "Reviewed bridge has no suspense residual to classify",
            code="custody_gap_residual_missing",
        )
    if classification in _EXTERNAL_RESIDUAL_CLASSIFICATIONS:
        for leg in suspense_legs:
            leg["role"] = "external"
            leg["notes"] = f"reviewed_residual:{classification}"
    elif classification == "retained_custody":
        for leg in suspense_legs:
            leg["role"] = "retained"
            leg["location_ref"] = (
                f"reviewed-retained-custody:{context['review']['gap_id']}"
            )
            leg["notes"] = "reviewed_residual:retained_custody"
    else:
        for leg in suspense_legs:
            leg["notes"] = "reviewed_residual:suspense_continuation"
    evidence = dict(old_component.get("evidence") or {})
    evidence["residual_classification"] = {
        "schema_version": 1,
        "classification": classification,
        "custody_state": _residual_custody_state(classification),
        "country_tax_meaning": "not_assigned",
        "amount_msat": residual_msat,
    }
    return (
        {
            "component_type": old_component["component_type"],
            "conservation_mode": old_component["conservation_mode"],
            "evidence_kind": old_component.get("evidence_kind"),
            "evidence_grade": old_component.get("evidence_grade"),
            "evidence": evidence,
            "conversion_policy": old_component.get("conversion_policy"),
            "conversion_reviewed": bool(old_component.get("conversion_reviewed")),
            "conversion_metadata": old_component.get("conversion_metadata") or {},
            "notes": old_component.get("notes"),
            "authored_source": authored_source,
            "legs": legs,
            "allocations": allocations,
        },
        residual_msat,
    )


def _snapshot_candidate(snapshot: Mapping[str, Any]) -> SimpleNamespace:
    return SimpleNamespace(
        started_at=snapshot.get("started_at"),
        ended_at=snapshot.get("ended_at"),
        retained_msat=int(snapshot.get("retained_msat") or 0),
        residual_msat=int(snapshot.get("residual_msat") or 0),
        source_fee_msat=int(snapshot.get("source_fee_msat") or 0),
    )


def _snapshot_downstream_years(snapshot: Mapping[str, Any]) -> tuple[int, ...]:
    downstream = snapshot.get("downstream")
    years = downstream.get("affected_years", ()) if isinstance(downstream, Mapping) else ()
    return tuple(int(year) for year in years)


def _preview_snapshot_impacts(
    conn: sqlite3.Connection,
    profile_id: str,
    snapshot: Mapping[str, Any],
    *,
    after_classification_summary: Mapping[str, Any],
) -> list[dict[str, Any]]:
    return custody_filed_reports.preview_custody_impacts(
        conn,
        profile_id=profile_id,
        candidate=_snapshot_candidate(snapshot),
        downstream_years=_snapshot_downstream_years(snapshot),
        after_classification_summary=after_classification_summary,
    )


def _append_snapshot_impacts(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    component_id: str,
    review_id: str,
    gap_id: str,
    snapshot: Mapping[str, Any],
    after_classification_summary: Mapping[str, Any],
) -> list[dict[str, Any]]:
    return custody_filed_reports.append_custody_impacts(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        component_id=component_id,
        review_id=review_id,
        gap_id=gap_id,
        candidate=_snapshot_candidate(snapshot),
        downstream_years=_snapshot_downstream_years(snapshot),
        after_classification_summary=after_classification_summary,
    )


def _reopened_classification_summary(
    snapshot: Mapping[str, Any],
) -> dict[str, Any]:
    source_total = int(snapshot.get("source_total_msat") or 0)
    return (
        {"custody_review_reopened": {"count": 1, "amount_msat": source_total}}
        if source_total
        else {}
    )


def _residual_classification_summary(
    snapshot: Mapping[str, Any],
    classification: str,
    residual_msat: int,
) -> dict[str, Any]:
    summary: dict[str, Any] = {}
    retained = int(snapshot.get("retained_msat") or 0)
    fee = int(snapshot.get("source_fee_msat") or 0)
    if retained:
        summary["internal_retained"] = {"count": 1, "amount_msat": retained}
    if residual_msat:
        bucket = (
            f"reviewed_{classification}"
            if classification != "suspense_continuation"
            else "custody_suspense"
        )
        summary[bucket] = {"count": 1, "amount_msat": residual_msat}
    if fee:
        summary["network_fee"] = {"count": 1, "amount_msat": fee}
    return summary


def _guided_component_spec(
    conn: sqlite3.Connection,
    candidate: CustodyGapCandidate,
    fingerprint: str,
    authored_source: str,
) -> dict[str, Any]:
    # Discovery strength is never authority. Even an amount/time-only hint may
    # be bridged after this exact local preview and an explicit user decision.
    # Conversely, no suggestion — including a promoted one — writes here by
    # itself. Ambiguity is returned as review warnings rather than disguised as
    # proof or an automatic activation path.
    if candidate.excess_msat:
        raise AppError(
            "The return exceeds the source principal",
            code="custody_gap_bridge_excess_return",
            hint=(
                "Classify the excess origin separately, then preview this exact "
                "bridge again."
            ),
            details={"excess_msat": candidate.excess_msat},
        )
    transaction_ids = (*candidate.source_ids, *candidate.return_ids)
    placeholders = ",".join("?" for _ in transaction_ids)
    rows = conn.execute(
        f"""
        SELECT t.id, t.wallet_id, t.direction, t.asset, t.amount, t.fee,
               t.amount_includes_fee, t.occurred_at, t.external_id, t.raw_json,
               w.kind AS wallet_kind, w.config_json
        FROM transactions t JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.id IN ({placeholders})
        """,
        (candidate.profile_id, *transaction_ids),
    ).fetchall()
    by_id = {str(row["id"]): row for row in rows}
    if set(by_id) != set(transaction_ids):
        raise AppError("Custody-gap evidence changed", code="custody_gap_stale")
    try:
        scopes = {
            transaction_id: resolve_protocol_scope(row)
            for transaction_id, row in by_id.items()
        }
    except (TypeError, ValueError) as exc:
        raise AppError(
            "Custody-gap protocol scope is no longer canonical",
            code="custody_gap_stale",
        ) from exc
    if any(
        scope.protocol_chain != candidate.protocol_chain
        or scope.network != candidate.network
        for scope in scopes.values()
    ):
        raise AppError("Custody-gap protocol scope changed", code="custody_gap_stale")

    legs: list[dict[str, Any]] = []
    allocations: list[dict[str, Any]] = []
    source_contexts: list[
        tuple[str, int, Mapping[str, Any], Mapping[str, str]]
    ] = []
    for index, transaction_id in enumerate(candidate.source_ids):
        row = by_id[transaction_id]
        boundary = row_boundary_amounts(row)
        scope = _transaction_scope(row)
        source_id = f"source-{index}"
        legs.append(
            _anchored_leg(
                source_id,
                "source",
                boundary.wallet_movement_msat,
                row,
                scope,
            )
        )
        source_contexts.append((source_id, boundary.principal_msat, row, scope))

    retained_remaining = candidate.retained_msat
    destination_amounts: list[tuple[str, int]] = []
    for index, transaction_id in enumerate(candidate.return_ids):
        row = by_id[transaction_id]
        amount = min(row_boundary_amounts(row).principal_msat, retained_remaining)
        if amount <= 0:
            continue
        destination_id = f"destination-{index}"
        legs.append(
            _anchored_leg(
                destination_id,
                "destination",
                amount,
                row,
                _transaction_scope(row),
            )
        )
        destination_amounts.append((destination_id, amount))
        retained_remaining -= amount

    if retained_remaining:
        raise AppError(
            "Custody-gap retained quantity is no longer available",
            code="custody_gap_stale",
        )

    try:
        retained_plan = allocate_msat_fifo(
            [
                (source_id, principal)
                for source_id, principal, _row, _scope in source_contexts
            ],
            destination_amounts,
            amount_msat=candidate.retained_msat,
        )
    except CustodyAllocationError as exc:
        raise AppError(
            "Custody-gap retained quantity is no longer available",
            code="custody_gap_stale",
            details={"allocation_code": exc.code, **exc.details},
        ) from exc
    allocations.extend(
        _allocation(cell.source_id, cell.sink_id, cell.amount_msat)
        for cell in retained_plan.cells
    )
    source_remaining = dict(retained_plan.source_remaining)

    for index, (source_id, _principal, row, scope) in enumerate(source_contexts):
        remaining = source_remaining[source_id]
        if remaining:
            suspense_id = f"suspense-{index}"
            legs.append(
                {
                    "id": suspense_id,
                    "role": "suspense",
                    "rail": "untracked",
                    "chain": None,
                    "network": scope["network"],
                    "asset": row["asset"],
                    "exposure": "bitcoin",
                    "conservation_unit": "msat",
                    "amount_msat": remaining,
                    "occurred_at": row["occurred_at"],
                }
            )
            allocations.append(_allocation(source_id, suspense_id, remaining))
        fee = row_boundary_amounts(row).fee_msat
        if fee:
            fee_id = f"fee-{index}"
            legs.append(_anchored_leg(fee_id, "fee", fee, row, scope))
            allocations.append(_allocation(source_id, fee_id, fee))
    component_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"kassiber:custody-gap-bridge:{candidate.profile_id}:{candidate.gap_id}:{fingerprint}",
        )
    )
    return {
        "component_id": component_id,
        "component_type": "manual_bridge",
        "conservation_mode": "quantity",
        "evidence_kind": "custody_gap_review",
        "evidence_grade": "reviewed",
        "evidence": {
            "gap_id": candidate.gap_id,
            "authored_claim_fingerprint": fingerprint,
            "review_warnings": list(_guided_bridge_warnings(candidate)),
        },
        "notes": "Guided bridge from explicitly reviewed custody-gap evidence",
        "authored_source": authored_source,
        "legs": legs,
        "allocations": allocations,
    }


def _allocation(source_id: str, sink_id: str, amount: int) -> dict[str, Any]:
    return {
        "source_leg_id": source_id,
        "sink_leg_id": sink_id,
        "source_amount_msat": amount,
        "sink_amount_msat": amount,
    }


def _transaction_scope(row: Mapping[str, Any]) -> dict[str, str]:
    scope = resolve_protocol_scope(row)
    return {
        "rail": scope.rail,
        "chain": scope.base_chain,
        "network": scope.network,
    }


def _anchored_leg(
    leg_id: str, role: str, amount: int, row: Mapping[str, Any], scope: Mapping[str, str]
) -> dict[str, Any]:
    return {
        "id": leg_id,
        "role": role,
        "rail": scope["rail"],
        "chain": scope["chain"],
        "network": scope["network"],
        "asset": row["asset"],
        "exposure": "bitcoin",
        "conservation_unit": "msat",
        "amount_msat": amount,
        "transaction_id": row["id"],
        "wallet_id": row["wallet_id"],
        "occurred_at": row["occurred_at"],
    }


def _append_review(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    candidate: CustodyGapCandidate,
    fingerprint: str,
    action: str,
    component_id: str | None,
    authored_source: str,
    reason: str | None,
    event_kind: str = "review_decision",
) -> dict[str, Any]:
    return _append_review_snapshot(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        gap_id=candidate.gap_id,
        fingerprint=fingerprint,
        action=action,
        event_kind=event_kind,
        component_id=component_id,
        authored_source=authored_source,
        reason=reason,
        snapshot=_candidate_snapshot(conn, candidate),
        transaction_relations=(
            *(("source", transaction_id) for transaction_id in candidate.source_ids),
            *(("return", transaction_id) for transaction_id in candidate.return_ids),
        ),
    )


def _append_review_snapshot(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    gap_id: str,
    fingerprint: str,
    action: str,
    event_kind: str,
    component_id: str | None,
    authored_source: str,
    reason: str | None,
    snapshot: Mapping[str, Any],
    transaction_relations: Sequence[tuple[str, str]] | None = None,
) -> dict[str, Any]:
    if event_kind not in _REVIEW_EVENT_KINDS:
        raise AppError(
            "Custody review event kind is unsupported",
            code="custody_gap_review_validation",
        )
    revision = int(
        conn.execute(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM custody_gap_reviews "
            "WHERE profile_id = ? AND gap_id = ?",
            (profile_id, gap_id),
        ).fetchone()[0]
    )
    review_id = str(uuid.uuid4())
    created_at = now_iso()
    if transaction_relations is None:
        transaction_relations = _prior_review_transaction_relations(
            conn,
            profile_id=profile_id,
            gap_id=gap_id,
            component_id=component_id,
        )
    transaction_relations = _normalize_review_transaction_relations(
        transaction_relations
    )
    conn.execute(
        """
        INSERT INTO custody_gap_reviews(
            id, workspace_id, profile_id, gap_id, revision,
            candidate_fingerprint, action, event_kind, component_id,
            authored_source, reason, snapshot_json, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            review_id, workspace_id, profile_id, gap_id, revision,
            fingerprint, action, event_kind, component_id, authored_source, reason,
            json.dumps(snapshot, sort_keys=True, separators=(",", ":")), created_at,
        ),
    )
    backfill_custody_gap_review_relation_set(
        conn,
        review_id=review_id,
        workspace_id=workspace_id,
        profile_id=profile_id,
        created_at=created_at,
        expected_source_count=sum(
            1 for role, _ in transaction_relations if role == "source"
        ),
        expected_return_count=sum(
            1 for role, _ in transaction_relations if role == "return"
        ),
    )
    _append_review_transaction_relations(
        conn,
        review_id=review_id,
        workspace_id=workspace_id,
        profile_id=profile_id,
        created_at=created_at,
        relations=transaction_relations,
    )
    return {
        "id": review_id,
        "gap_id": gap_id,
        "revision": revision,
        "candidate_fingerprint": fingerprint,
        "action": action,
        "event_kind": event_kind,
        "component_id": component_id,
        "authored_source": authored_source,
        "reason": reason,
        "created_at": created_at,
    }


def _prior_review_transaction_relations(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    gap_id: str,
    component_id: str | None,
) -> tuple[tuple[str, str], ...]:
    rows = conn.execute(
        """
        SELECT x.role, x.transaction_id
        FROM custody_gap_review_transactions x
        JOIN custody_gap_reviews r ON r.id = x.review_id
        WHERE r.profile_id = ? AND r.gap_id = ?
          AND r.revision = (
              SELECT MAX(revision) FROM custody_gap_reviews
              WHERE profile_id = ? AND gap_id = ?
          )
        ORDER BY r.created_at DESC, r.id DESC,
                 CASE x.role WHEN 'source' THEN 0 ELSE 1 END,
                 x.transaction_id, x.id
        """,
        (profile_id, gap_id, profile_id, gap_id),
    ).fetchall()
    if not rows and component_id:
        rows = conn.execute(
            """
            SELECT CASE role WHEN 'destination' THEN 'return' ELSE 'source' END AS role,
                   COALESCE(anchor_transaction_id, transaction_id) AS transaction_id
            FROM custody_component_legs
            WHERE profile_id = ? AND component_id = ?
              AND role IN ('source', 'fee', 'destination')
              AND COALESCE(anchor_transaction_id, transaction_id) IS NOT NULL
            ORDER BY ordinal, id
            """,
            (profile_id, component_id),
        ).fetchall()
    relations: list[tuple[str, str]] = []
    seen: set[tuple[str, str]] = set()
    for row in rows:
        relation = (str(row["role"]), str(row["transaction_id"]))
        if relation not in seen:
            seen.add(relation)
            relations.append(relation)
    return tuple(relations)


def _append_review_transaction_relations(
    conn: sqlite3.Connection,
    *,
    review_id: str,
    workspace_id: str,
    profile_id: str,
    created_at: str,
    relations: Sequence[tuple[str, str]],
) -> None:
    relations = _normalize_review_transaction_relations(relations)
    for role, transaction_id in relations:
        conn.execute(
            """
            INSERT INTO custody_gap_review_transactions(
                id, review_id, workspace_id, profile_id,
                role, transaction_id, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                custody_gap_review_transaction_id(
                    review_id, role, transaction_id
                ),
                review_id,
                workspace_id,
                profile_id,
                role,
                transaction_id,
                created_at,
            ),
        )


def _normalize_review_transaction_relations(
    relations: Sequence[tuple[str, str]],
) -> tuple[tuple[str, str], ...]:
    grouped: dict[str, list[str]] = {"source": [], "return": []}
    for role, transaction_id in relations:
        if role not in grouped:
            raise AppError(
                "Custody review transaction role is unsupported",
                code="custody_gap_review_validation",
            )
        normalized_id = str(transaction_id or "")
        if not normalized_id:
            raise AppError(
                "Custody review transaction identity is missing",
                code="custody_gap_review_validation",
            )
        if normalized_id not in grouped[role]:
            grouped[role].append(normalized_id)
    return tuple(
        (role, transaction_id)
        for role in ("source", "return")
        for transaction_id in sorted(grouped[role])
    )


def _candidate_snapshot(
    conn: sqlite3.Connection, candidate: CustodyGapCandidate
) -> dict[str, Any]:
    affected_rows = conn.execute(
        """
        SELECT occurred_at FROM transactions
        WHERE profile_id = ? AND direction = 'outbound'
          AND asset = ? AND occurred_at > ?
        """,
        (candidate.profile_id, candidate.asset, candidate.ended_at),
    ).fetchall()
    affected_years = sorted(
        {
            int(str(row["occurred_at"])[:4])
            for row in affected_rows
            if str(row["occurred_at"])[:4].isdigit()
        }
    )
    return {
        "gap_id": candidate.gap_id,
        "candidate_fingerprint": candidate_fingerprint(candidate),
        "authored_claim_fingerprint": authored_claim_fingerprint(candidate),
        "status": "needs_review",
        "asset": candidate.asset,
        "source_wallet_label": " + ".join(candidate.source_wallet_labels),
        "destination_wallet_labels": list(candidate.destination_wallet_labels),
        "source_total_msat": candidate.source_total_msat,
        "source_fee_msat": candidate.source_fee_msat,
        "source_debit_msat": candidate.source_debit_msat,
        "return_total_msat": candidate.return_total_msat,
        "retained_msat": candidate.retained_msat,
        "residual_msat": candidate.residual_msat,
        "excess_msat": candidate.excess_msat,
        "started_at": candidate.started_at,
        "ended_at": candidate.ended_at,
        "confidence": candidate.confidence,
        "promotion_eligible": candidate.promotion_eligible,
        "competitor_score_margin": candidate.competitor_score_margin,
        "reason_codes": list(candidate.reason_codes),
        "review_warnings": list(_guided_bridge_warnings(candidate)),
        "downstream": {
            "affected_disposals": len(affected_rows),
            "affected_years": affected_years,
        },
    }


def _downstream_affected_years(
    conn: sqlite3.Connection, candidate: CustodyGapCandidate
) -> tuple[int, ...]:
    snapshot = _candidate_snapshot(conn, candidate)
    return tuple(snapshot["downstream"]["affected_years"])


def _preview_filed_report_impacts(
    conn: sqlite3.Connection, candidate: CustodyGapCandidate
) -> list[dict[str, Any]]:
    return custody_filed_reports.preview_custody_impacts(
        conn,
        profile_id=candidate.profile_id,
        candidate=candidate,
        downstream_years=_downstream_affected_years(conn, candidate),
    )


def _guided_bridge_warnings(candidate: CustodyGapCandidate) -> tuple[str, ...]:
    warnings = ["manual_review_required"]
    if not candidate.promotion_eligible:
        warnings.append("weak_advisory_evidence")
    if candidate.conflict_size > 1:
        warnings.append("competing_candidates")
    if candidate.residual_msat:
        warnings.append("unresolved_residual")
    if candidate.excess_msat:
        warnings.append("excess_return_unclassified")
    return tuple(warnings)


def _require_current_candidate(
    conn: sqlite3.Connection,
    candidate: CustodyGapCandidate,
) -> CustodyGapCandidate:
    """Re-derive reviewed facts inside the action's SQLite snapshot.

    Callers normally resolve a candidate immediately before entering this
    module, but another connection can change imported evidence between those
    reads.  A caller-supplied dataclass is therefore never the authority for a
    durable review.  The savepoint established by each public action pins this
    re-read through component/review persistence.
    """

    from .custody_gaps import find_gap_candidate

    try:
        current = find_gap_candidate(
            conn,
            candidate.profile_id,
            candidate.gap_id,
            persist_projection=False,
        )
    except (AppError, TypeError, ValueError) as exc:
        raise AppError(
            "Custody-gap evidence changed after it was loaded",
            code="custody_gap_stale",
            hint="Reload the review queue before confirming.",
        ) from exc
    if candidate_fingerprint(current) != candidate_fingerprint(candidate):
        raise AppError(
            "Custody-gap evidence changed after it was loaded",
            code="custody_gap_stale",
            hint="Reload the review queue before confirming.",
        )
    return current


def _component_review_status(
    conn: sqlite3.Connection, component_id: str
) -> dict[str, Any]:
    if not component_id:
        return {"usable": False, "reason": "component_missing"}
    try:
        component = custody_components.get_component(
            conn, component_id, include_local_evidence=False
        )
    except AppError:
        return {"usable": False, "reason": "component_missing"}
    native_support_status = str(
        (component.get("native_support_status") or {}).get("status")
        or "unverified"
    )
    if component.get("effective_state") != "active":
        return {
            "usable": False,
            "reason": (
                "component_native_support_contradicted"
                if native_support_status == "contradicted"
                else "component_not_effective"
            ),
            "native_support_status": native_support_status,
        }
    from .custody_quantity_store import component_evidence_status

    evidence = component_evidence_status(conn, component)
    if not evidence.get("valid"):
        return {
            "usable": False,
            "reason": str(
                evidence.get("status")
                or "component_evidence_unusable"
            ),
            "native_support_status": native_support_status,
        }
    return {
        "usable": True,
        "reason": None,
        "native_support_status": native_support_status,
    }


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


def _public_review(review: Mapping[str, Any]) -> dict[str, Any]:
    return {
        key: review.get(key)
        for key in (
            "gap_id", "revision", "action", "event_kind", "authored_source",
            "reason", "created_at"
        )
    }


__all__ = [
    "RESIDUAL_CLASSIFICATIONS",
    "apply_review",
    "authored_claim_fingerprint",
    "candidate_fingerprint",
    "historical_review_gaps",
    "latest_reviews",
    "latest_dismissed_fingerprints",
    "list_review_history",
    "plan_review",
    "public_review_plan",
    "residual_custody_state",
    "review_state",
    "review_status",
]
