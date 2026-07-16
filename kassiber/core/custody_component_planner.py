"""Pure planning and fingerprint-checked apply for custody component batches."""

from __future__ import annotations

import hashlib
import hmac
import json
import sqlite3
import uuid
from decimal import Decimal
from typing import Any, Mapping, Sequence

from ..errors import AppError
from ..msat import btc_to_msat
from ..wallet_descriptors import normalize_asset_code
from . import custody_components, wallets
from .transfer_matching import LIGHTNING_WALLET_KINDS


MAX_COMPONENTS = 50
MAX_LEGS_PER_COMPONENT = 256
MAX_ALLOCATIONS_PER_COMPONENT = 4096
MAX_TOTAL_LEGS = 2000
MAX_TOTAL_ALLOCATIONS = 10_000

_COMPONENT_FIELDS = (
    "component_type",
    "conservation_mode",
    "evidence_kind",
    "evidence_grade",
    "evidence",
    "conversion_policy",
    "conversion_reviewed",
    "conversion_metadata",
    "notes",
    "change_reason",
    "component_id",
    "lineage_id",
    "created_at",
)


def _error(message: str, *, code: str = "validation", **details: Any) -> AppError:
    return AppError(message, code=code, details=details or None)


def _scope(conn: sqlite3.Connection, workspace_id: str, profile_id: str) -> Mapping[str, Any]:
    row = conn.execute(
        "SELECT * FROM profiles WHERE id = ? AND workspace_id = ?",
        (profile_id, workspace_id),
    ).fetchone()
    if row is None:
        raise _error("Custody component profile was not found", code="not_found")
    return row


def _resolve_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
    reference: Any,
) -> Mapping[str, Any]:
    text = str(reference or "").strip()
    row = conn.execute(
        "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
        (profile_id, text),
    ).fetchone()
    if row is not None:
        return row
    rows = conn.execute(
        "SELECT * FROM transactions WHERE profile_id = ? AND external_id = ? "
        "ORDER BY occurred_at DESC, created_at DESC, id DESC LIMIT 2",
        (profile_id, text),
    ).fetchall()
    if len(rows) > 1:
        raise _error(
            f"Transaction external_id '{text}' is ambiguous",
            code="ambiguous_reference",
        )
    if not rows:
        raise _error(f"Transaction '{text}' was not found", code="not_found")
    return rows[0]


def _resolve_wallet(
    conn: sqlite3.Connection,
    profile_id: str,
    reference: Any,
) -> Mapping[str, Any]:
    text = str(reference or "").strip()
    row = conn.execute(
        "SELECT * FROM wallets WHERE profile_id = ? AND id = ?",
        (profile_id, text),
    ).fetchone()
    if row is not None:
        return row
    rows = conn.execute(
        "SELECT * FROM wallets WHERE profile_id = ? AND lower(label) = lower(?) "
        "ORDER BY label, id LIMIT 2",
        (profile_id, text),
    ).fetchall()
    if len(rows) > 1:
        raise _error(f"Wallet label '{text}' is ambiguous")
    if not rows:
        raise _error(f"Wallet '{text}' was not found", code="not_found")
    return rows[0]


def _planned_wallet_id(profile_id: str, label: str) -> str:
    return str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"kassiber:planned-untracked-wallet:{profile_id}:{label.casefold()}",
        )
    )


def _deterministic_id(component_id: str, kind: str, ordinal: int) -> str:
    return str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            f"kassiber:custody-component:{component_id}:{kind}:{ordinal}",
        )
    )


def _prepare_legs(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    component_id: str,
    raw_legs: Any,
    planned_wallets: dict[str, dict[str, str]],
) -> list[dict[str, Any]]:
    if not isinstance(raw_legs, list):
        raise _error("Custody component legs must be a JSON array")
    prepared: list[dict[str, Any]] = []
    for ordinal, raw in enumerate(raw_legs):
        if not isinstance(raw, Mapping):
            raise _error(f"Custody component leg {ordinal} must be an object")
        leg = dict(raw)
        leg.setdefault("id", _deterministic_id(component_id, "leg", ordinal))
        transaction_ref = leg.pop("transaction", None) or leg.pop(
            "transaction_ref", None
        )
        untracked_wallet = leg.pop("untracked_wallet", None)
        wallet_ref = leg.pop("wallet", None) or leg.pop("wallet_ref", None)
        if transaction_ref is not None and untracked_wallet is not None:
            raise _error(
                f"Custody component leg {ordinal} cannot combine transaction with untracked_wallet"
            )
        if untracked_wallet is not None and (
            wallet_ref is not None or leg.get("wallet_id")
        ):
            raise _error(
                f"Custody component leg {ordinal} cannot combine untracked_wallet with wallet"
            )
        wallet: Mapping[str, Any] | None = None
        if transaction_ref is not None:
            transaction = _resolve_transaction(conn, profile_id, transaction_ref)
            leg["transaction_id"] = transaction["id"]
            leg.setdefault("wallet_id", transaction["wallet_id"])
            leg.setdefault("asset", transaction["asset"])
            leg.setdefault("occurred_at", transaction["occurred_at"])
        if untracked_wallet is not None:
            if not isinstance(untracked_wallet, str) or not untracked_wallet.strip():
                raise _error(
                    f"Custody component leg {ordinal} untracked_wallet must be a label"
                )
            label = untracked_wallet.strip()
            try:
                wallet = _resolve_wallet(conn, profile_id, label)
            except AppError as error:
                if error.code != "not_found":
                    raise
                wallet_id = _planned_wallet_id(profile_id, label)
                planned_wallets.setdefault(
                    wallet_id,
                    {"id": wallet_id, "label": label, "kind": "untracked"},
                )
                leg["wallet_id"] = wallet_id
            else:
                if str(wallet["kind"] or "").lower() != "untracked":
                    raise _error(
                        f"Wallet '{label}' exists but is not an untracked placeholder",
                        code="conflict",
                    )
                leg["wallet_id"] = wallet["id"]
        if wallet_ref is not None:
            wallet = _resolve_wallet(conn, profile_id, wallet_ref)
            leg["wallet_id"] = wallet["id"]
        if "amount_btc" in leg:
            if "amount_msat" in leg:
                raise _error(
                    f"Custody component leg {ordinal} cannot set both amount_btc and amount_msat"
                )
            leg["amount_msat"] = btc_to_msat(Decimal(str(leg.pop("amount_btc"))))
        asset = normalize_asset_code(leg.get("asset") or "BTC")
        leg["asset"] = asset
        if wallet is None and leg.get("wallet_id") not in (None, ""):
            wallet_id = str(leg["wallet_id"])
            if wallet_id not in planned_wallets:
                wallet = _resolve_wallet(conn, profile_id, wallet_id)
        wallet_kind = (
            "untracked"
            if str(leg.get("wallet_id") or "") in planned_wallets
            else str(wallet["kind"] if wallet is not None else "").lower()
        )
        if not leg.get("rail"):
            if wallet_kind == "untracked":
                leg["rail"] = "untracked"
            elif wallet_kind in LIGHTNING_WALLET_KINDS:
                leg["rail"] = "lightning"
            elif asset == "LBTC":
                leg["rail"] = "liquid"
            else:
                leg["rail"] = "bitcoin"
        if not leg.get("exposure"):
            leg["exposure"] = "bitcoin" if asset in {"BTC", "LBTC"} else asset.lower()
        if not leg.get("conservation_unit"):
            leg["conservation_unit"] = (
                "msat" if asset in {"BTC", "LBTC"} else "asset-quantum"
            )
        prepared.append(leg)
    return prepared


def _prepare_allocations(
    component_id: str,
    raw_allocations: Any,
) -> list[dict[str, Any]]:
    allocations = [] if raw_allocations in (None, []) else raw_allocations
    if not isinstance(allocations, list):
        raise _error("Custody component allocations must be a JSON array")
    prepared = []
    for ordinal, raw in enumerate(allocations):
        if not isinstance(raw, Mapping):
            raise _error(f"Custody component allocation {ordinal} must be an object")
        prepared.append(
            {
                **dict(raw),
                "id": raw.get("id")
                or _deterministic_id(component_id, "allocation", ordinal),
            }
        )
    return prepared


def _batch_fingerprint(payload: Mapping[str, Any]) -> str:
    encoded = json.dumps(
        payload,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def plan_component_batch(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    specs: Sequence[Mapping[str, Any]],
    activate: bool,
    authored_source: str,
) -> dict[str, Any]:
    """Return a deterministic component batch without mutating SQLite."""

    if not isinstance(specs, list) or not specs:
        raise _error("Bulk custody resolution requires a non-empty components array")
    if len(specs) > MAX_COMPONENTS:
        raise _error(
            f"Bulk custody resolution accepts at most {MAX_COMPONENTS} components",
            count=len(specs),
            max_components=MAX_COMPONENTS,
        )
    if type(activate) is not bool:
        raise _error("activate must be a boolean")
    profile = _scope(conn, workspace_id, profile_id)
    planned_wallets: dict[str, dict[str, str]] = {}
    planned_components: list[dict[str, Any]] = []
    total_legs = 0
    total_allocations = 0
    for index, raw_spec in enumerate(specs):
        if not isinstance(raw_spec, Mapping):
            raise _error("Custody component spec must be a JSON object")
        spec = dict(raw_spec)
        try:
            canonical = json.dumps(spec, sort_keys=True, separators=(",", ":"))
        except (TypeError, ValueError) as error:
            raise _error("Custody component spec is not JSON-safe") from error
        requested_component_id = str(spec.get("component_id") or "").strip()
        component_id = requested_component_id or str(
            uuid.uuid5(
                uuid.NAMESPACE_URL,
                "kassiber:custody-bulk:"
                f"{profile_id}:{authored_source}:{int(activate)}:{index}:{canonical}",
            )
        )
        existing = conn.execute(
            "SELECT 1 FROM custody_components WHERE id = ? AND profile_id = ?",
            (component_id, profile_id),
        ).fetchone()
        if existing is not None:
            if requested_component_id:
                raise _error(
                    "Custody component id already exists",
                    code="conflict",
                    component_id=component_id,
                )
            component = custody_components.get_component(
                conn,
                component_id,
                profile_id=profile_id,
                include_local_evidence=False,
            )
            planned_components.append(
                {"existing": True, "spec": None, "component": component}
            )
            continue
        legs = _prepare_legs(
            conn,
            profile_id=profile_id,
            component_id=component_id,
            raw_legs=spec.get("legs"),
            planned_wallets=planned_wallets,
        )
        allocations = _prepare_allocations(component_id, spec.get("allocations"))
        total_legs += len(legs)
        total_allocations += len(allocations)
        if len(legs) > MAX_LEGS_PER_COMPONENT:
            raise _error(
                f"Custody component accepts at most {MAX_LEGS_PER_COMPONENT} legs",
                count=len(legs),
                max_legs=MAX_LEGS_PER_COMPONENT,
            )
        if len(allocations) > MAX_ALLOCATIONS_PER_COMPONENT:
            raise _error(
                "Custody component contains too many allocations",
                count=len(allocations),
                max_allocations=MAX_ALLOCATIONS_PER_COMPONENT,
            )
        header = custody_components.normalize_component_header(
            **{
                field: spec[field]
                for field in _COMPONENT_FIELDS
                if field in spec
                and field
                not in {"component_type", "conservation_mode", "component_id"}
            },
            component_type=spec.get("component_type") or "manual_bridge",
            conservation_mode=spec.get("conservation_mode") or "quantity",
            component_id=component_id,
            authored_source=authored_source,
        )
        component_type = header["component_type"]
        conservation_mode = header["conservation_mode"]
        validation = custody_components.validate_component_plan(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            component_type=component_type,
            legs=legs,
            allocations=allocations,
            conservation_mode=conservation_mode,
            conversion_policy=header["conversion_policy"],
            conversion_reviewed=header["conversion_reviewed"],
            evidence_grade=header["evidence_grade"],
            planned_wallet_ids=planned_wallets,
        )
        if activate and not validation["validation"]["activatable"]:
            raise _error(
                "Custody component plan cannot activate",
                code="custody_component_not_activatable",
                component_id=component_id,
                issues=validation["validation"]["issues"],
            )
        prepared_spec = {
            **{
                field: value
                for field, value in header.items()
                if field != "authored_source"
            },
            "legs": validation["legs"],
            "allocations": validation["allocations"],
        }
        component = {
            "id": component_id,
            "lineage_id": header["lineage_id"],
            "workspace_id": workspace_id,
            "profile_id": profile_id,
            "revision": 1,
            "component_type": component_type,
            "conservation_mode": conservation_mode,
            "state": "active" if activate else "draft",
            "effective_state": "active" if activate else "draft",
            "evidence_kind": header["evidence_kind"],
            "evidence_grade": header["evidence_grade"],
            "conversion_policy": header["conversion_policy"],
            "conversion_reviewed": header["conversion_reviewed"],
            "expected_leg_count": len(validation["legs"]),
            "expected_allocation_count": len(validation["allocations"]),
            "authored_source": authored_source,
            "notes": header["notes"],
            "change_reason": header["change_reason"],
            "activated_at": None,
            "superseded_at": None,
            "created_at": header["created_at"],
            "legs": validation["legs"],
            "allocations": validation["allocations"],
            "validation": validation["validation"],
        }
        planned_components.append(
            {"existing": False, "spec": prepared_spec, "component": component}
        )
    if total_legs > MAX_TOTAL_LEGS:
        raise _error(
            "Bulk custody resolution contains too many legs",
            count=total_legs,
            max_legs=MAX_TOTAL_LEGS,
        )
    if total_allocations > MAX_TOTAL_ALLOCATIONS:
        raise _error(
            "Bulk custody resolution contains too many allocations",
            count=total_allocations,
            max_allocations=MAX_TOTAL_ALLOCATIONS,
        )
    if activate:
        batch_issues = custody_components.validate_planned_active_batch(
            conn,
            profile_id=profile_id,
            components=[
                item["component"]
                for item in planned_components
                if not item["existing"]
            ],
            planned_untracked_wallet_ids=planned_wallets,
        )
        if batch_issues:
            raise _error(
                "Custody component batch cannot activate",
                code="custody_component_not_activatable",
                issues=batch_issues,
            )
    fingerprint_input = {
        "schema_version": 1,
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "input_version": int(profile["journal_input_version"] or 0),
        "activate": activate,
        "authored_source": authored_source,
        "planned_wallets": list(planned_wallets.values()),
        "components": planned_components,
    }
    fingerprint = _batch_fingerprint(fingerprint_input)
    components = [item["component"] for item in planned_components]
    return {
        **fingerprint_input,
        "fingerprint": fingerprint,
        "components": components,
        "prepared_components": planned_components,
        "summary": {
            "count": len(components),
            "active": sum(item.get("effective_state") == "active" for item in components),
            "draft": sum(item.get("effective_state") != "active" for item in components),
        },
    }


def apply_component_batch(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    specs: Sequence[Mapping[str, Any]],
    activate: bool,
    authored_source: str,
    expected_fingerprint: str,
    include_local_evidence: bool = False,
    commit: bool = True,
) -> dict[str, Any]:
    """Persist exactly the current plan after fingerprint revalidation."""

    if (
        not isinstance(expected_fingerprint, str)
        or len(expected_fingerprint) != 64
        or any(
            character not in "0123456789abcdef"
            for character in expected_fingerprint
        )
    ):
        raise _error(
            "Custody component plan fingerprint is invalid",
            expected_fingerprint=expected_fingerprint,
        )
    plan = plan_component_batch(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        specs=specs,
        activate=activate,
        authored_source=authored_source,
    )
    if not hmac.compare_digest(plan["fingerprint"], expected_fingerprint):
        raise _error(
            "Custody component plan is stale",
            code="custody_review_plan_stale",
            expected_fingerprint=expected_fingerprint,
            current_fingerprint=plan["fingerprint"],
        )
    savepoint = f"custody_component_apply_{uuid.uuid4().hex}"
    conn.execute(f"SAVEPOINT {savepoint}")
    try:
        for planned_wallet in plan["planned_wallets"]:
            wallets.create_wallet(
                conn,
                workspace_id,
                profile_id,
                planned_wallet["label"],
                "untracked",
                commit=False,
                wallet_id=planned_wallet["id"],
            )
        created = []
        for item in plan["prepared_components"]:
            if item["existing"]:
                created.append(
                    custody_components.get_component(
                        conn,
                        item["component"]["id"],
                        profile_id=profile_id,
                        include_local_evidence=include_local_evidence,
                    )
                )
                continue
            spec = item["spec"]
            component = custody_components.create_component(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                authored_source=authored_source,
                **{
                    key: value
                    for key, value in spec.items()
                    if key in _COMPONENT_FIELDS or key in {"legs", "allocations"}
                },
            )
            if activate:
                component = custody_components.activate_component(conn, component["id"])
            if not include_local_evidence:
                component = custody_components.get_component(
                    conn,
                    component["id"],
                    profile_id=profile_id,
                    include_local_evidence=False,
                )
            created.append(component)
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        raise
    conn.execute(f"RELEASE SAVEPOINT {savepoint}")
    if commit:
        conn.commit()
    return {
        "fingerprint": plan["fingerprint"],
        "components": created,
        "summary": {
            "count": len(created),
            "active": sum(item.get("effective_state") == "active" for item in created),
            "draft": sum(item.get("effective_state") != "active" for item in created),
        },
    }


def public_component_batch_plan(plan: Mapping[str, Any]) -> dict[str, Any]:
    """Drop apply-only prepared rows from the UI/AI preview payload."""

    return {
        key: value
        for key, value in plan.items()
        if key not in {"prepared_components", "planned_wallets"}
    }


__all__ = [
    "MAX_COMPONENTS",
    "apply_component_batch",
    "plan_component_batch",
    "public_component_batch_plan",
]
