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
        "dry_run": True,
        "requires_explicit_confirmation": True,
        **{
            key: value
            for key, value in plan.items()
            if key not in {"prepared_components", "planned_wallets"}
        },
    }


_REVISION_FIELDS = frozenset(
    {
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
        "legs",
        "allocations",
    }
)


def _public_component(component: Mapping[str, Any]) -> dict[str, Any]:
    result = dict(component)
    result.pop("evidence", None)
    result.pop("conversion_metadata", None)
    result["legs"] = [
        {key: value for key, value in leg.items() if key != "location_ref"}
        for leg in component.get("legs", ())
    ]
    return result


def plan_component_revision(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    component_id: str,
    spec: Mapping[str, Any] | None = None,
    activate: bool = False,
    reason: str | None = None,
    authored_source: str = "user",
) -> dict[str, Any]:
    """Plan an immutable revision or undo without changing SQLite."""

    if action not in {"revise", "undo"}:
        raise _error("Custody component revision action is unsupported", action=action)
    if type(activate) is not bool:
        raise _error("activate must be a boolean")
    profile = _scope(conn, workspace_id, profile_id)
    old = custody_components.get_component(
        conn,
        component_id,
        profile_id=profile_id,
        include_local_evidence=True,
    )
    if action == "undo" and old["state"] != "superseded":
        raise _error(
            "Only a superseded revision can be restored",
            code="custody_component_not_superseded",
            component_id=component_id,
            state=old["state"],
        )
    raw_spec = {} if spec is None else spec
    if not isinstance(raw_spec, Mapping):
        raise _error("Custody component revision must be a JSON object")
    unknown = sorted(set(raw_spec) - _REVISION_FIELDS)
    if unknown:
        raise _error(
            "Custody component revision contains unsupported fields",
            fields=unknown,
        )
    spec_dict = dict(raw_spec)
    try:
        canonical = json.dumps(spec_dict, sort_keys=True, separators=(",", ":"))
    except (TypeError, ValueError) as error:
        raise _error("Custody component revision is not JSON-safe") from error
    existing_draft = conn.execute(
        "SELECT id FROM custody_components "
        "WHERE profile_id = ? AND lineage_id = ? AND state = 'draft' AND id != ?",
        (profile_id, old["lineage_id"], component_id),
    ).fetchone()
    if existing_draft:
        raise _error(
            "Component lineage already has a draft revision",
            code="custody_component_draft_exists",
            draft_component_id=existing_draft["id"],
        )
    next_revision = int(
        conn.execute(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM custody_components "
            "WHERE profile_id = ? AND lineage_id = ?",
            (profile_id, old["lineage_id"]),
        ).fetchone()[0]
    )
    new_component_id = str(
        uuid.uuid5(
            uuid.NAMESPACE_URL,
            "kassiber:custody-revision:"
            f"{profile_id}:{component_id}:{action}:{next_revision}:"
            f"{authored_source}:{int(activate)}:{reason or ''}:{canonical}",
        )
    )
    if conn.execute(
        "SELECT 1 FROM custody_components WHERE id = ?", (new_component_id,)
    ).fetchone():
        raise _error(
            "Planned custody component revision already exists",
            code="conflict",
            component_id=new_component_id,
        )

    planned_wallets: dict[str, dict[str, str]] = {}
    raw_legs = spec_dict.get("legs", old["legs"])
    if not isinstance(raw_legs, list):
        raise _error("Custody component legs must be a JSON array")
    hidden_by_id = {
        str(leg["id"]): leg.get("location_ref")
        for leg in old["legs"]
        if leg.get("location_ref") is not None
    }
    preserved_legs: list[Any] = []
    for raw_leg in raw_legs:
        if not isinstance(raw_leg, Mapping):
            preserved_legs.append(raw_leg)
            continue
        preserved = dict(raw_leg)
        raw_id = str(preserved.get("id") or "")
        if "location_ref" not in preserved and raw_id in hidden_by_id:
            preserved["location_ref"] = hidden_by_id[raw_id]
        preserved_legs.append(preserved)
    legs = _prepare_legs(
        conn,
        profile_id=profile_id,
        component_id=new_component_id,
        raw_legs=preserved_legs,
        planned_wallets=planned_wallets,
    )
    leg_id_map: dict[str, str] = {}
    for ordinal, (raw_leg, leg) in enumerate(zip(preserved_legs, legs, strict=True)):
        if isinstance(raw_leg, Mapping) and raw_leg.get("id") not in (None, ""):
            leg_id_map[str(raw_leg["id"])] = _deterministic_id(
                new_component_id, "leg", ordinal
            )
        leg["id"] = _deterministic_id(new_component_id, "leg", ordinal)

    if "allocations" in spec_dict:
        raw_allocations = spec_dict["allocations"]
    elif "legs" in spec_dict:
        raw_allocations = []
    else:
        raw_allocations = old["allocations"]
    if not isinstance(raw_allocations, list):
        raise _error("Custody component allocations must be a JSON array")
    remapped_allocations: list[Any] = []
    for raw_allocation in raw_allocations:
        if not isinstance(raw_allocation, Mapping):
            remapped_allocations.append(raw_allocation)
            continue
        remapped = dict(raw_allocation)
        for endpoint in ("source", "sink"):
            field = f"{endpoint}_leg_id"
            if str(remapped.get(field) or "") in leg_id_map:
                remapped[field] = leg_id_map[str(remapped[field])]
        remapped_allocations.append(remapped)
    allocations = _prepare_allocations(new_component_id, remapped_allocations)
    for ordinal, allocation in enumerate(allocations):
        allocation["id"] = _deterministic_id(
            new_component_id, "allocation", ordinal
        )

    economic_terms: list[dict[str, Any]] = []
    for ordinal, term in enumerate(old.get("economic_terms") or ()):
        source_id = leg_id_map.get(str(term["source_leg_id"]))
        target_id = leg_id_map.get(str(term["target_leg_id"]))
        if source_id is None or target_id is None:
            raise _error(
                "Revision must preserve legs bound by migrated economic terms",
                code="custody_component_economic_term_leg_changed",
                term_id=term["id"],
            )
        economic_terms.append(
            {
                **dict(term),
                "id": _deterministic_id(new_component_id, "economic-term", ordinal),
                "source_leg_id": source_id,
                "target_leg_id": target_id,
            }
        )
    header_values = {
        "component_type": old["component_type"],
        "conservation_mode": old["conservation_mode"],
        "evidence_kind": old["evidence_kind"],
        "evidence_grade": old["evidence_grade"],
        "evidence": old.get("evidence"),
        "conversion_policy": old["conversion_policy"],
        "conversion_reviewed": old["conversion_reviewed"],
        "conversion_metadata": old.get("conversion_metadata"),
        "notes": old["notes"],
    }
    header_values.update(
        {
            field: spec_dict[field]
            for field in header_values
            if field in spec_dict
        }
    )
    header = custody_components.normalize_component_header(
        **header_values,
        change_reason=(
            reason if action == "undo" else spec_dict.get("change_reason", reason)
        ),
        component_id=new_component_id,
        lineage_id=old["lineage_id"],
        authored_source=authored_source,
    )
    validation = custody_components.validate_component_plan(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        component_type=header["component_type"],
        legs=legs,
        allocations=allocations,
        conservation_mode=header["conservation_mode"],
        conversion_policy=header["conversion_policy"],
        conversion_reviewed=header["conversion_reviewed"],
        evidence_grade=header["evidence_grade"],
        replacing_lineage_id=old["lineage_id"],
        planned_wallet_ids=planned_wallets,
    )
    economic_terms = custody_components.normalize_economic_terms(
        economic_terms, validation["legs"]
    )
    if activate and not validation["validation"]["activatable"]:
        raise _error(
            "Custody component revision cannot activate",
            code="custody_component_not_activatable",
            component_id=new_component_id,
            issues=validation["validation"]["issues"],
        )
    prepared_spec = {
        **{key: value for key, value in header.items() if key != "lineage_id"},
        "legs": validation["legs"],
        "allocations": validation["allocations"],
        "economic_terms": economic_terms,
    }
    planned_component = {
        "id": new_component_id,
        "lineage_id": old["lineage_id"],
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "revision": next_revision,
        "component_type": header["component_type"],
        "conservation_mode": header["conservation_mode"],
        "state": "active" if activate else "draft",
        "effective_state": "active" if activate else "draft",
        "evidence_kind": header["evidence_kind"],
        "evidence_grade": header["evidence_grade"],
        "evidence": header["evidence"],
        "conversion_policy": header["conversion_policy"],
        "conversion_reviewed": header["conversion_reviewed"],
        "conversion_metadata": header["conversion_metadata"],
        "authored_source": authored_source,
        "notes": header["notes"],
        "change_reason": header["change_reason"],
        "supersedes_component_id": component_id,
        "activated_at": None,
        "superseded_at": None,
        "created_at": header["created_at"],
        "legs": validation["legs"],
        "allocations": validation["allocations"],
        "economic_terms": economic_terms,
        "validation": validation["validation"],
    }
    commitment = {
        "schema_version": 1,
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "input_version": int(profile["journal_input_version"] or 0),
        "action": action,
        "source_component_id": component_id,
        "source_component_commitment": _batch_fingerprint(old),
        "activate": activate,
        "authored_source": authored_source,
        "reason": reason or "",
        "planned_wallets": list(planned_wallets.values()),
        "prepared_spec": prepared_spec,
        "component": planned_component,
    }
    return {
        **commitment,
        "fingerprint": _batch_fingerprint(commitment),
        "dry_run": True,
        "requires_explicit_confirmation": True,
    }


def public_component_revision_plan(plan: Mapping[str, Any]) -> dict[str, Any]:
    """Return a renderer-safe revision plan without apply-only local evidence."""

    return {
        **{
            key: value
            for key, value in plan.items()
            if key not in {"prepared_spec", "planned_wallets"}
        },
        "component": _public_component(plan["component"]),
    }


def apply_component_revision(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    component_id: str,
    expected_fingerprint: str,
    spec: Mapping[str, Any] | None = None,
    activate: bool = False,
    reason: str | None = None,
    authored_source: str = "user",
    include_local_evidence: bool = False,
    commit: bool = True,
) -> dict[str, Any]:
    """Persist exactly a current immutable revision plan."""

    try:
        plan = plan_component_revision(
            conn,
            workspace_id=workspace_id,
            profile_id=profile_id,
            action=action,
            component_id=component_id,
            spec=spec,
            activate=activate,
            reason=reason,
            authored_source=authored_source,
        )
    except AppError as error:
        if error.code not in {
            "conflict",
            "custody_component_draft_exists",
            "custody_component_not_superseded",
        }:
            raise
        raise _error(
            "Custody component plan is stale",
            code="custody_review_plan_stale",
            expected_fingerprint=expected_fingerprint,
            current_error=error.code,
        ) from error
    if not isinstance(expected_fingerprint, str) or not hmac.compare_digest(
        plan["fingerprint"], expected_fingerprint
    ):
        raise _error(
            "Custody component plan is stale",
            code="custody_review_plan_stale",
            expected_fingerprint=expected_fingerprint,
            current_fingerprint=plan["fingerprint"],
        )
    savepoint = f"custody_component_revision_apply_{uuid.uuid4().hex}"
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
        prepared = dict(plan["prepared_spec"])
        prepared.pop("lineage_id", None)
        prepared.pop("component_id", None)
        created_at = prepared.pop("created_at", None)
        component = custody_components.update_component(
            conn,
            component_id,
            new_component_id=plan["component"]["id"],
            created_at=created_at,
            preserve_planned_row_ids=True,
            **prepared,
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
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        raise
    conn.execute(f"RELEASE SAVEPOINT {savepoint}")
    if commit:
        conn.commit()
    return {"fingerprint": plan["fingerprint"], "component": component}


def plan_component_state_change(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    component_id: str,
    reason: str | None = None,
) -> dict[str, Any]:
    """Plan activation or supersession without mutating component state."""

    if action not in {"activate", "supersede"}:
        raise _error("Custody component state action is unsupported", action=action)
    profile = _scope(conn, workspace_id, profile_id)
    component = custody_components.get_component(
        conn,
        component_id,
        profile_id=profile_id,
        include_local_evidence=True,
    )
    if action == "activate":
        component = custody_components.validate_component_activation(
            conn, component_id
        )
        resulting_state = "active"
    else:
        resulting_state = "superseded"
    component_commitment = _batch_fingerprint(component)
    commitment = {
        "schema_version": 1,
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "input_version": int(profile["journal_input_version"] or 0),
        "action": action,
        "component_id": component_id,
        "component_commitment": component_commitment,
        "current_state": component["state"],
        "resulting_state": resulting_state,
        "reason": reason or "",
    }
    public_component = custody_components.get_component(
        conn,
        component_id,
        profile_id=profile_id,
        include_local_evidence=False,
    )
    return {
        **commitment,
        "fingerprint": _batch_fingerprint(commitment),
        "component": public_component,
        "dry_run": True,
        "requires_explicit_confirmation": True,
    }


def apply_component_state_change(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    component_id: str,
    expected_fingerprint: str,
    reason: str | None = None,
    include_local_evidence: bool = False,
    commit: bool = True,
) -> dict[str, Any]:
    """Persist the current state-change plan after exact fingerprint checking."""

    plan = plan_component_state_change(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        action=action,
        component_id=component_id,
        reason=reason,
    )
    if not isinstance(expected_fingerprint, str) or not hmac.compare_digest(
        plan["fingerprint"], expected_fingerprint
    ):
        raise _error(
            "Custody component plan is stale",
            code="custody_review_plan_stale",
            expected_fingerprint=expected_fingerprint,
            current_fingerprint=plan["fingerprint"],
        )
    savepoint = f"custody_component_state_apply_{uuid.uuid4().hex}"
    conn.execute(f"SAVEPOINT {savepoint}")
    try:
        if action == "activate":
            component = custody_components.activate_component(conn, component_id)
        else:
            component = custody_components.supersede_component(
                conn, component_id, reason=reason
            )
        if not include_local_evidence:
            component = custody_components.get_component(
                conn,
                component["id"],
                profile_id=profile_id,
                include_local_evidence=False,
            )
    except Exception:
        conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        raise
    conn.execute(f"RELEASE SAVEPOINT {savepoint}")
    if commit:
        conn.commit()
    return {"fingerprint": plan["fingerprint"], "component": component}


COMPONENT_REVIEW_ACTIONS = frozenset(
    {"create", "revise", "undo", "activate", "supersede"}
)


def _component_review_arguments(
    *,
    action: str,
    components: Sequence[Mapping[str, Any]] | None,
    component_id: str | None,
    spec: Mapping[str, Any] | None,
    activate: bool | None,
    reason: str | None,
) -> dict[str, Any]:
    if action not in COMPONENT_REVIEW_ACTIONS:
        raise _error("Custody component review action is unsupported", action=action)
    if reason is not None and not isinstance(reason, str):
        raise _error("Custody component review reason must be text")
    if activate is not None and type(activate) is not bool:
        raise _error("Custody component review activate must be a boolean")
    if action == "create":
        if component_id is not None or spec is not None or reason is not None:
            raise _error(
                "Create accepts components and activate only",
                fields=[
                    name
                    for name, value in (
                        ("component_id", component_id),
                        ("spec", spec),
                        ("reason", reason),
                    )
                    if value is not None
                ],
            )
        return {"specs": components, "activate": True if activate is None else activate}
    if components is not None:
        raise _error(f"{action} does not accept components")
    if not isinstance(component_id, str) or not component_id.strip():
        raise _error(f"{action} requires component_id")
    component_id = component_id.strip()
    if action == "revise":
        if not isinstance(spec, Mapping):
            raise _error("Revise requires a component spec")
        return {
            "component_id": component_id,
            "spec": spec,
            "activate": False if activate is None else activate,
            "reason": reason,
        }
    if action == "undo":
        if spec is not None or activate not in (None, False):
            raise _error("Undo does not accept spec or activation")
        return {
            "component_id": component_id,
            "spec": None,
            "activate": False,
            "reason": reason or "undo",
        }
    if spec is not None or activate is not None:
        raise _error(f"{action} does not accept spec or activate")
    if action == "activate" and reason is not None:
        raise _error("Activate does not accept reason")
    return {"component_id": component_id, "reason": reason}


def plan_component_review(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    components: Sequence[Mapping[str, Any]] | None = None,
    component_id: str | None = None,
    spec: Mapping[str, Any] | None = None,
    activate: bool | None = None,
    reason: str | None = None,
    authored_source: str = "user",
) -> dict[str, Any]:
    """Plan every authored component action through one strict pure contract."""

    normalized = _component_review_arguments(
        action=action,
        components=components,
        component_id=component_id,
        spec=spec,
        activate=activate,
        reason=reason,
    )
    if action == "create":
        return public_component_batch_plan(
            plan_component_batch(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                authored_source=authored_source,
                **normalized,
            )
        )
    if action in {"revise", "undo"}:
        return public_component_revision_plan(
            plan_component_revision(
                conn,
                workspace_id=workspace_id,
                profile_id=profile_id,
                action=action,
                authored_source=authored_source,
                **normalized,
            )
        )
    return plan_component_state_change(
        conn,
        workspace_id=workspace_id,
        profile_id=profile_id,
        action=action,
        **normalized,
    )


def apply_component_review(
    conn: sqlite3.Connection,
    *,
    workspace_id: str,
    profile_id: str,
    action: str,
    expected_fingerprint: str,
    components: Sequence[Mapping[str, Any]] | None = None,
    component_id: str | None = None,
    spec: Mapping[str, Any] | None = None,
    activate: bool | None = None,
    reason: str | None = None,
    authored_source: str = "user",
    include_local_evidence: bool = False,
    commit: bool = True,
) -> dict[str, Any]:
    """Apply every authored component action through one strict stale-plan gate."""

    normalized = _component_review_arguments(
        action=action,
        components=components,
        component_id=component_id,
        spec=spec,
        activate=activate,
        reason=reason,
    )
    common = {
        "conn": conn,
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "expected_fingerprint": expected_fingerprint,
        "include_local_evidence": include_local_evidence,
        "commit": commit,
    }
    if action == "create":
        return apply_component_batch(
            authored_source=authored_source,
            **common,
            **normalized,
        )
    if action in {"revise", "undo"}:
        return apply_component_revision(
            action=action,
            authored_source=authored_source,
            **common,
            **normalized,
        )
    return apply_component_state_change(
        action=action,
        **common,
        **normalized,
    )


__all__ = [
    "COMPONENT_REVIEW_ACTIONS",
    "MAX_COMPONENTS",
    "apply_component_batch",
    "apply_component_revision",
    "apply_component_review",
    "apply_component_state_change",
    "plan_component_batch",
    "plan_component_revision",
    "plan_component_review",
    "plan_component_state_change",
    "public_component_batch_plan",
    "public_component_revision_plan",
]
