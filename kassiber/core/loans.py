"""Bitcoin-backed lending: data model, provider presets, and the tax-pipeline
role map.

A *loan* is a facility (one row in ``loans``); its on-chain and off-chain events
are *legs* (rows in ``loan_legs``) that each link to a journal transaction with a
role. The role drives tax classification: a ``collateral_lock`` suppresses the
outbound disposal (the coins stay in the owned global pool, encumbered — NOT a
separate balance-bearing account, which would re-introduce the per-(exchange,
holder) "balance went negative" abort); a ``collateral_release`` suppresses the
inbound acquisition (the coins return to the pool they never left); a
``liquidation`` falls through to the normal disposal path (the one real SELL).

The design rationale and the per-provider handling live in
docs/plan/12-collateralized-loans.md. This module owns the enums, the editable
provider presets, validation, and the small amount of CRUD the CLI / daemon call.
"""

from __future__ import annotations

import uuid
from typing import Any, Mapping, Optional, Sequence

from ..errors import AppError
from ..time_utils import now_iso

# --- Enums -----------------------------------------------------------------

LOAN_ROLES = ("borrower", "lender")

# custody_type is the linchpin: who can move the collateral. Finer than a binary
# so the (advisory, never-default) tax consequence can differ per shape.
CUSTODY_TYPES = (
    "non_custodial_multisig",  # borrower holds a live key (2-of-3)
    "non_custodial_presigned",  # borrower's key generated once then discarded (Firefish)
    "collaborative_multisig",  # borrower 1-of-3 + sub-trust beneficial interest (Unchained)
    "custodial_segregated",  # provider holds all keys, ring-fenced/attested
    "custodial_rehypothecated",  # provider holds all keys, may re-lend
    "onchain_smartcontract",  # code custodies (no human key)
)

# Orthogonal to custody_type: re-lending — not key-count — is what pushes toward
# the contested-disposal branch. Even then the disposal reading is legally
# unconfirmed, so it is an advisory flag, never a default booking.
REHYPOTHECATION_VALUES = ("none", "allowed", "unknown")

CONTROL_MECHANISMS = ("live_key", "presigned_only", "none")

LOAN_STATUSES = ("open", "repaid", "defaulted", "liquidated", "cancelled", "disputed")

LEG_ROLES = (
    "collateral_lock",
    "collateral_topup",
    "principal_draw",
    "interest_payment",
    "principal_repay",
    "collateral_release",
    "liquidation",
    "liquidation_surplus_return",
    "collateral_repay_sale",
    "recovery_release",
    "cancellation_release",
    "escrow_consolidation",
    "wrapped_conversion_out",
)

# Outbound legs whose coins stay owned (encumbered): suppress the disposal, keep
# the lot in the global pool.
LOCK_SUPPRESS_ROLES = frozenset(
    {"collateral_lock", "collateral_topup", "escrow_consolidation"}
)
# Inbound legs whose coins return to the pool they never left (a repayment
# round-trip): suppress the acquisition so the round-trip nets to nothing. NOTE:
# liquidation_surplus_return is deliberately NOT here — on a liquidation the full
# collateral is disposed, so any surplus that comes back is a genuine NEW
# acquisition at its return value (booked as a normal BUY), not a suppressed
# round-trip.
RELEASE_SUPPRESS_ROLES = frozenset(
    {
        "collateral_release",
        "recovery_release",
        "cancellation_release",
    }
)
# Outbound legs that ARE the disposal — they fall through to the normal SELL path
# (listed for documentation / CLI validation; the engine needs no special case).
DISPOSAL_ROLES = frozenset({"liquidation", "collateral_repay_sale"})

# When a loan defaults or is liquidated, the collateral that was locked is gone:
# the lock outbound (which is otherwise suppressed) becomes THE disposal. Status,
# not a manual re-tag, drives this — the tax map applies it via effective_leg_role.
# Only the user's actual collateral funding converts (NOT an internal escrow
# consolidation hop, which would double-count).
_LIQUIDATING_SOURCE_ROLES = frozenset({"collateral_lock", "collateral_topup"})
_LIQUIDATING_STATUSES = frozenset({"liquidated", "defaulted"})


def effective_leg_role(role: str, loan_status: Optional[str]) -> str:
    """The role the tax engine should act on, after applying loan status.

    A collateral lock/top-up on a liquidated/defaulted loan is no longer a
    non-event: the coins were seized, so it becomes the disposal. A
    collateral_release on such a loan is coins coming back AFTER the full
    collateral was disposed, so it is a re-acquisition (not a suppressed
    repayment round-trip). Every other role passes through unchanged. With
    ``loan_status=None`` (e.g. a unit test that hand-builds legs) nothing is
    transformed."""
    if loan_status in _LIQUIDATING_STATUSES:
        if role in _LIQUIDATING_SOURCE_ROLES:
            return "liquidation"
        if role == "collateral_release":
            return "liquidation_surplus_return"
    return role
# Out-of-scope legs (e.g. BTC->cbBTC wrap): quarantine for review, never book.
QUARANTINE_ROLES = frozenset({"wrapped_conversion_out"})

# Roles whose leg must reference a real journal transaction (they book or suppress
# against an on-chain row). interest_payment / principal_draw / principal_repay may
# be off-chain (fiat/USDC) and carry transaction_id = NULL.
ONCHAIN_REQUIRED_ROLES = frozenset(
    LOCK_SUPPRESS_ROLES | RELEASE_SUPPRESS_ROLES | DISPOSAL_ROLES | QUARANTINE_ROLES
)

# --- Provider presets (editable suggestions, never silent commitments) ------
# Stored denormalized onto the loan as preset_label + preset_version, so a preset
# rename/removal can never orphan a facility (red-team D2). import_tier advertises
# the honest ingest path; the chain carries no loan semantics for any provider.

PRESET_VERSION = "2026-06"

PROVIDER_PRESETS: dict[str, dict[str, Any]] = {
    "firefish": {
        "label": "Firefish",
        "custody_type": "non_custodial_presigned",
        "rehypothecation": "none",
        "control_mechanism": "presigned_only",
        "import_tier": "watched_address",
        "caveat": "Borrower escrow key is generated once then discarded; non-disposal argument is pre-committed outcomes, not key retention.",
    },
    "hodlhodl": {
        "label": "Hodl Hodl Lend",
        "custody_type": "non_custodial_multisig",
        "rehypothecation": "none",
        "control_mechanism": "live_key",
        "import_tier": "api_anchor",
        "caveat": "2-of-3 escrow; release signed by lender+platform on the happy path.",
    },
    "unchained": {
        "label": "Unchained",
        "custody_type": "collaborative_multisig",
        "rehypothecation": "none",
        "control_mechanism": "live_key",
        "import_tier": "descriptor",
        "caveat": "2-of-3 collaborative custody + sub-trust; legal title in trust, borrower holds beneficial interest (advisory title caveat).",
    },
    "debifi": {
        "label": "Debifi",
        "custody_type": "non_custodial_multisig",
        "rehypothecation": "none",
        "control_mechanism": "live_key",
        "import_tier": "watched_address",
        "caveat": "3-of-4; the 4th key holder is a per-loan role.",
    },
    "ledn": {
        "label": "Ledn",
        "custody_type": "custodial_segregated",
        "rehypothecation": "none",
        "control_mechanism": "none",
        "import_tier": "csv",
        "caveat": "Custodied (no re-lend) for originations on/after 2025-07-01; legacy 'Standard' loans were rehypothecated — set rehypothecation=allowed and as_of_custody_date for those.",
    },
    "nexo": {
        "label": "Nexo",
        "custody_type": "custodial_segregated",
        "rehypothecation": "unknown",
        "control_mechanism": "none",
        "import_tier": "csv",
        "caveat": "Fully custodial; CSV Type literals unverified — review imported rows.",
    },
    "salt": {
        "label": "SALT",
        "custody_type": "custodial_rehypothecated",
        "rehypothecation": "allowed",
        "control_mechanism": "none",
        "import_tier": "csv",
        "caveat": "Explicit repledge/rehypothecation rights — strongest contested-disposal flag.",
    },
    "strike": {
        "label": "Strike",
        "custody_type": "custodial_segregated",
        "rehypothecation": "none",
        "control_mechanism": "none",
        "import_tier": "csv",
        "caveat": "Delegated third-party custody, segregated.",
    },
    "xapo": {
        "label": "Xapo",
        "custody_type": "custodial_segregated",
        "rehypothecation": "none",
        "control_mechanism": "none",
        "import_tier": "manual",
        "caveat": "MPC custody; manual entry.",
    },
    "coinbase": {
        "label": "Coinbase / Morpho",
        "custody_type": "onchain_smartcontract",
        "rehypothecation": "none",
        "control_mechanism": "none",
        "import_tier": "manual",
        "caveat": "BTC->cbBTC on Base: zero Bitcoin base-layer footprint for the lock; only the BTC withdrawal leg is in scope, as an out-of-scope wrap disposal question.",
    },
    "private": {
        "label": "Other / private",
        "custody_type": None,
        "rehypothecation": "unknown",
        "control_mechanism": "live_key",
        "import_tier": "manual",
        "caveat": "Generic; choose custody explicitly.",
    },
}


def list_provider_presets() -> list[dict[str, Any]]:
    """Editable suggestion rows for the create-loan wizard."""
    return [
        {"preset_id": key, "version": PRESET_VERSION, **preset}
        for key, preset in PROVIDER_PRESETS.items()
    ]


# --- Validation ------------------------------------------------------------


def _require_choice(value: Optional[str], allowed: Sequence[str], field: str) -> None:
    if value is not None and value not in allowed:
        raise AppError(
            f"Invalid {field} '{value}'. Allowed: {', '.join(allowed)}",
            code="validation",
            details={"field": field, "value": value},
        )


def validate_loan_fields(
    *,
    role: Optional[str] = None,
    custody_type: Optional[str] = None,
    rehypothecation: Optional[str] = None,
    control_mechanism: Optional[str] = None,
    status: Optional[str] = None,
) -> None:
    _require_choice(role, LOAN_ROLES, "role")
    _require_choice(custody_type, CUSTODY_TYPES, "custody_type")
    _require_choice(rehypothecation, REHYPOTHECATION_VALUES, "rehypothecation")
    _require_choice(control_mechanism, CONTROL_MECHANISMS, "control_mechanism")
    _require_choice(status, LOAN_STATUSES, "status")


def validate_leg_role(role: str) -> None:
    if role not in LEG_ROLES:
        raise AppError(
            f"Invalid loan leg role '{role}'. Allowed: {', '.join(LEG_ROLES)}",
            code="validation",
            details={"field": "role", "value": role},
        )


# --- Row converters --------------------------------------------------------


def loan_to_dict(row: Mapping[str, Any]) -> dict[str, Any]:
    return {key: row[key] for key in row.keys()}


def leg_to_dict(row: Mapping[str, Any]) -> dict[str, Any]:
    return {key: row[key] for key in row.keys()}


# --- CRUD ------------------------------------------------------------------


def create_loan(
    conn,
    workspace_id: str,
    profile_id: str,
    *,
    role: str = "borrower",
    platform: Optional[str] = None,
    preset_id: Optional[str] = None,
    custody_type: Optional[str] = None,
    rehypothecation: Optional[str] = None,
    control_mechanism: Optional[str] = None,
    principal_asset: Optional[str] = None,
    principal_amount: Optional[int] = None,
    collateral_asset: str = "BTC",
    status: str = "open",
    public_offering: bool = False,
    interest_asset: Optional[str] = None,
    interest_terms: Optional[str] = None,
    as_of_custody_date: Optional[str] = None,
    escrow_descriptor: Optional[str] = None,
    notes: Optional[str] = None,
    commit: bool = True,
) -> dict[str, Any]:
    """Create a loan facility. A ``preset_id`` seeds editable defaults (snapshotted
    as preset_label + preset_version); explicit kwargs always win."""

    preset_label = None
    preset_version = None
    if preset_id is not None:
        preset = PROVIDER_PRESETS.get(preset_id)
        if preset is None:
            raise AppError(
                f"Unknown provider preset '{preset_id}'. Known: {', '.join(PROVIDER_PRESETS)}",
                code="validation",
                details={"field": "preset_id", "value": preset_id},
            )
        preset_label = preset["label"]
        preset_version = PRESET_VERSION
        if platform is None:
            platform = preset["label"]
        if custody_type is None:
            custody_type = preset["custody_type"]
        if rehypothecation is None:
            rehypothecation = preset["rehypothecation"]
        if control_mechanism is None:
            control_mechanism = preset["control_mechanism"]

    rehypothecation = rehypothecation or "unknown"
    control_mechanism = control_mechanism or "live_key"
    validate_loan_fields(
        role=role,
        custody_type=custody_type,
        rehypothecation=rehypothecation,
        control_mechanism=control_mechanism,
        status=status,
    )

    loan_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO loans(
            id, workspace_id, profile_id, role, platform, preset_label, preset_version,
            custody_type, rehypothecation, control_mechanism, principal_asset,
            principal_amount, collateral_asset, status, public_offering, interest_asset,
            interest_terms, as_of_custody_date, escrow_descriptor, notes, deleted_at, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (
            loan_id,
            workspace_id,
            profile_id,
            role,
            platform,
            preset_label,
            preset_version,
            custody_type,
            rehypothecation,
            control_mechanism,
            principal_asset,
            principal_amount,
            collateral_asset,
            status,
            1 if public_offering else 0,
            interest_asset,
            interest_terms,
            as_of_custody_date,
            escrow_descriptor,
            notes,
            now_iso(),
        ),
    )
    if commit:
        conn.commit()
    return get_loan(conn, profile_id, loan_id)


_LOAN_MUTABLE_FIELDS = (
    "platform",
    "custody_type",
    "rehypothecation",
    "control_mechanism",
    "principal_asset",
    "principal_amount",
    "collateral_asset",
    "status",
    "public_offering",
    "interest_asset",
    "interest_terms",
    "as_of_custody_date",
    "escrow_descriptor",
    "notes",
)


def update_loan(
    conn, profile_id: str, loan_id: str, *, commit: bool = True, **fields: Any
) -> dict[str, Any]:
    existing = get_loan(conn, profile_id, loan_id)
    if existing is None:
        raise AppError(f"Loan '{loan_id}' not found", code="not_found")
    validate_loan_fields(
        custody_type=fields.get("custody_type"),
        rehypothecation=fields.get("rehypothecation"),
        control_mechanism=fields.get("control_mechanism"),
        status=fields.get("status"),
    )
    updates = {k: v for k, v in fields.items() if k in _LOAN_MUTABLE_FIELDS and v is not None}
    if "public_offering" in updates:
        updates["public_offering"] = 1 if updates["public_offering"] else 0
    if not updates:
        return existing
    assignments = ", ".join(f"{col} = ?" for col in updates)
    conn.execute(
        f"UPDATE loans SET {assignments} WHERE id = ? AND profile_id = ?",
        (*updates.values(), loan_id, profile_id),
    )
    if commit:
        conn.commit()
    return get_loan(conn, profile_id, loan_id)


def get_loan(conn, profile_id: str, loan_id: str) -> Optional[dict[str, Any]]:
    row = conn.execute(
        "SELECT * FROM loans WHERE id = ? AND profile_id = ? AND deleted_at IS NULL",
        (loan_id, profile_id),
    ).fetchone()
    return loan_to_dict(row) if row else None


def list_loans(
    conn, profile_id: str, *, include_deleted: bool = False
) -> list[dict[str, Any]]:
    extra = "" if include_deleted else "AND deleted_at IS NULL"
    rows = conn.execute(
        f"SELECT * FROM loans WHERE profile_id = ? {extra} ORDER BY created_at DESC",
        (profile_id,),
    ).fetchall()
    loans = [loan_to_dict(row) for row in rows]
    for loan in loans:
        loan["legs"] = list_loan_legs(conn, profile_id, loan_id=loan["id"])
    return loans


def delete_loan(conn, profile_id: str, loan_id: str, *, commit: bool = True) -> dict[str, Any]:
    existing = get_loan(conn, profile_id, loan_id)
    if existing is None:
        raise AppError(f"Loan '{loan_id}' not found", code="not_found")
    deleted_at = now_iso()
    conn.execute(
        "UPDATE loans SET deleted_at = ? WHERE id = ? AND profile_id = ?",
        (deleted_at, loan_id, profile_id),
    )
    conn.execute(
        "UPDATE loan_legs SET deleted_at = ? WHERE loan_id = ? AND profile_id = ? AND deleted_at IS NULL",
        (deleted_at, loan_id, profile_id),
    )
    if commit:
        conn.commit()
    return {"deleted": loan_id}


def create_loan_leg(
    conn,
    workspace_id: str,
    profile_id: str,
    loan_id: str,
    *,
    role: str,
    transaction_id: Optional[str] = None,
    escrow_address: Optional[str] = None,
    escrow_txid: Optional[str] = None,
    escrow_vout: Optional[int] = None,
    amount: Optional[int] = None,
    fiat_value: Optional[float] = None,
    occurred_at: Optional[str] = None,
    policy: str = "carrying-value",
    on_chain_present: bool = True,
    notes: Optional[str] = None,
    commit: bool = True,
) -> dict[str, Any]:
    validate_leg_role(role)
    if get_loan(conn, profile_id, loan_id) is None:
        raise AppError(f"Loan '{loan_id}' not found", code="not_found")
    if role in ONCHAIN_REQUIRED_ROLES and transaction_id is None:
        raise AppError(
            f"Loan leg role '{role}' must reference a journal transaction (--txid)",
            code="validation",
            details={"role": role, "field": "transaction_id"},
        )
    if transaction_id is not None:
        tx = conn.execute(
            "SELECT id FROM transactions WHERE id = ? AND profile_id = ?",
            (transaction_id, profile_id),
        ).fetchone()
        if tx is None:
            raise AppError(
                f"Transaction '{transaction_id}' not found in this profile",
                code="not_found",
                details={"transaction_id": transaction_id},
            )
        existing = conn.execute(
            "SELECT id FROM loan_legs WHERE profile_id = ? AND transaction_id = ? AND deleted_at IS NULL",
            (profile_id, transaction_id),
        ).fetchone()
        if existing is not None:
            raise AppError(
                f"Transaction '{transaction_id}' is already a leg of a loan",
                code="conflict",
                details={"transaction_id": transaction_id, "existing_leg": existing["id"]},
            )

    leg_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO loan_legs(
            id, workspace_id, profile_id, loan_id, role, transaction_id,
            escrow_address, escrow_txid, escrow_vout, amount, fiat_value,
            occurred_at, policy, on_chain_present, notes, deleted_at, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (
            leg_id,
            workspace_id,
            profile_id,
            loan_id,
            role,
            transaction_id,
            escrow_address,
            escrow_txid,
            escrow_vout,
            amount,
            fiat_value,
            occurred_at,
            policy,
            1 if on_chain_present else 0,
            notes,
            now_iso(),
        ),
    )
    if commit:
        conn.commit()
    row = conn.execute("SELECT * FROM loan_legs WHERE id = ?", (leg_id,)).fetchone()
    return leg_to_dict(row)


def list_loan_legs(
    conn, profile_id: str, *, loan_id: Optional[str] = None, include_deleted: bool = False
) -> list[dict[str, Any]]:
    clauses = ["profile_id = ?"]
    params: list[Any] = [profile_id]
    if loan_id is not None:
        clauses.append("loan_id = ?")
        params.append(loan_id)
    if not include_deleted:
        clauses.append("deleted_at IS NULL")
    rows = conn.execute(
        f"SELECT * FROM loan_legs WHERE {' AND '.join(clauses)} ORDER BY occurred_at IS NULL, occurred_at ASC, created_at ASC",
        tuple(params),
    ).fetchall()
    return [leg_to_dict(row) for row in rows]


def delete_loan_leg(conn, profile_id: str, leg_id: str, *, commit: bool = True) -> dict[str, Any]:
    row = conn.execute(
        "SELECT id FROM loan_legs WHERE id = ? AND profile_id = ? AND deleted_at IS NULL",
        (leg_id, profile_id),
    ).fetchone()
    if row is None:
        raise AppError(f"Loan leg '{leg_id}' not found", code="not_found")
    conn.execute(
        "UPDATE loan_legs SET deleted_at = ? WHERE id = ? AND profile_id = ?",
        (now_iso(), leg_id, profile_id),
    )
    if commit:
        conn.commit()
    return {"deleted": leg_id}


def load_loan_leg_role_map(conn, profile_id: str) -> dict[str, str]:
    """``{transaction_id: role}`` for active, on-chain loan legs — consumed by the
    tax pipeline to classify the matching journal transaction by its loan role."""
    rows = conn.execute(
        """
        SELECT transaction_id, role FROM loan_legs
        WHERE profile_id = ? AND deleted_at IS NULL AND transaction_id IS NOT NULL
        """,
        (profile_id,),
    ).fetchall()
    return {str(row["transaction_id"]): str(row["role"]) for row in rows}


# --- Actionable status (signal-not-reassurance) ----------------------------


def loan_action_items(conn, profile_id: str) -> list[dict[str, Any]]:
    """Return only actionable items — never a standing "all good" row. A clean,
    healthy loan produces nothing here."""
    items: list[dict[str, Any]] = []
    for loan in list_loans(conn, profile_id):
        legs = loan.get("legs", [])
        roles = {leg["role"] for leg in legs}
        loan_ref = {"loan_id": loan["id"], "platform": loan.get("platform")}
        if not any(r in LOCK_SUPPRESS_ROLES for r in roles):
            items.append({**loan_ref, "action": "needs_lock", "detail": "No collateral lock leg is recorded yet."})
        if loan.get("status") == "open" and "collateral_lock" in roles and not (
            roles & RELEASE_SUPPRESS_ROLES or roles & DISPOSAL_ROLES
        ):
            items.append({**loan_ref, "action": "needs_close_out", "detail": "Loan is open with a lock but no release or liquidation leg."})
        if loan.get("custody_type") == "custodial_rehypothecated":
            items.append({**loan_ref, "action": "rehyp_review", "detail": "Rehypothecating custodial lock — possible disposal at FMV (contested; advisory)."})
        elif loan.get("custody_type") in ("custodial_segregated",) and loan.get("rehypothecation") == "unknown":
            items.append({**loan_ref, "action": "custody_review", "detail": "Custodial lock with unknown rehypothecation — confirm custody terms."})
        if loan.get("status") in ("liquidated", "defaulted") and not (
            roles & DISPOSAL_ROLES or roles & LOCK_SUPPRESS_ROLES
        ):
            items.append({**loan_ref, "action": "needs_liquidation_leg", "detail": "Loan marked liquidated/defaulted but no lock or liquidation leg books the disposal."})
    return items


# --- Escrow positions (advanced: per-UTXO collateral tracking) -------------


def create_escrow_position(
    conn,
    workspace_id: str,
    profile_id: str,
    loan_id: str,
    *,
    escrow_address: Optional[str] = None,
    output_type: str = "unknown",
    amount: Optional[int] = None,
    acquired_basis_ref: Optional[str] = None,
    commit: bool = True,
) -> dict[str, Any]:
    if get_loan(conn, profile_id, loan_id) is None:
        raise AppError(f"Loan '{loan_id}' not found", code="not_found")
    position_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO loan_escrow_positions(
            id, workspace_id, profile_id, loan_id, escrow_address, output_type,
            amount, acquired_basis_ref, deleted_at, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (
            position_id,
            workspace_id,
            profile_id,
            loan_id,
            escrow_address,
            output_type or "unknown",
            amount,
            acquired_basis_ref,
            now_iso(),
        ),
    )
    if commit:
        conn.commit()
    row = conn.execute("SELECT * FROM loan_escrow_positions WHERE id = ?", (position_id,)).fetchone()
    return {key: row[key] for key in row.keys()}


def list_escrow_positions(
    conn, profile_id: str, *, loan_id: Optional[str] = None
) -> list[dict[str, Any]]:
    clauses = ["profile_id = ?", "deleted_at IS NULL"]
    params: list[Any] = [profile_id]
    if loan_id is not None:
        clauses.append("loan_id = ?")
        params.append(loan_id)
    rows = conn.execute(
        f"SELECT * FROM loan_escrow_positions WHERE {' AND '.join(clauses)} ORDER BY created_at ASC",
        tuple(params),
    ).fetchall()
    return [{key: row[key] for key in row.keys()} for row in rows]


# --- Advisory caveats + Steuerberater handoff ------------------------------

_ROLE_TAX_EFFECT = {
    "collateral_lock": "non-event (encumbered; coins stay in the owned pool)",
    "collateral_topup": "non-event (encumbered)",
    "escrow_consolidation": "non-event (internal hop)",
    "collateral_release": "non-event (basis carries back)",
    "recovery_release": "non-event (basis carries back)",
    "cancellation_release": "non-event (basis carries back)",
    "principal_draw": "not income (liability)",
    "principal_repay": "non-event (liability reduction)",
    "interest_payment": "cost; disposal of the sats if paid in BTC",
    "liquidation": "DISPOSAL at FMV",
    "collateral_repay_sale": "DISPOSAL at FMV",
    "liquidation_surplus_return": "re-acquisition at FMV",
    "wrapped_conversion_out": "out-of-scope wrap — quarantined for review",
}


def loan_advisory(loan: Mapping[str, Any]) -> list[str]:
    """The tax caveats that apply to this loan, surfaced (never silently applied)
    for the user / their Steuerberater. Empty for the clean cases."""
    caveats: list[str] = []
    custody = loan.get("custody_type")
    rehyp = loan.get("rehypothecation")
    if custody == "custodial_rehypothecated" or rehyp == "allowed":
        caveats.append(
            "Rehypothecating custodial collateral: posting it may be argued to be a "
            "disposal at FMV (contested; no Austrian ruling). Treated here as a "
            "non-disposal by default — confirm with a Steuerberater."
        )
    elif custody == "custodial_segregated":
        caveats.append(
            "Custodial (segregated) collateral: non-disposal treatment assumes beneficial "
            "ownership is retained. Confirm the custody terms."
        )
    if custody == "non_custodial_presigned":
        caveats.append(
            "The escrow key is generated once then discarded, so at steady state no live "
            "signing key is held; the non-disposal argument rests on pre-committed outcomes, "
            "not key retention (advisory, not BMF-confirmed)."
        )
    if custody == "collaborative_multisig":
        caveats.append(
            "Collaborative custody / sub-trust: legal title may sit in trust while you hold "
            "the beneficial interest — a title-transfer argument could recharacterize the lock."
        )
    if loan.get("role") == "lender":
        if loan.get("public_offering"):
            caveats.append(
                "Lender interest at the 27.5% special rate assumes the lending was publicly "
                "offered (§27a Abs 2). Confirm the public-offering test is met."
            )
        else:
            caveats.append(
                "Lender interest on a private/non-public loan defaults to the progressive "
                "tariff (up to 55%), NOT 27.5% (§27a Abs 2)."
            )
    if loan.get("interest_asset") == "BTC":
        caveats.append("Interest paid in BTC is itself a disposal of those sats.")
    if loan.get("status") in ("liquidated", "defaulted"):
        caveats.append(
            "Liquidation/default proceeds are modelled at the collateral lock's recorded "
            "value, not the seizure-date FMV — confirm the liquidation-date valuation with a "
            "Steuerberater."
        )
    return caveats


def build_steuerberater_report(conn, profile: Mapping[str, Any]) -> dict[str, Any]:
    """A structured, advisory handoff of every loan: facility terms, the per-leg
    roles and their tax effect, custody/rehypothecation, and the caveats that need
    a Steuerberater sign-off. Kassiber does no tax math here — it lays out what was
    modelled and why, so an advisor can check it."""
    profile_id = profile["id"]
    report_loans = []
    for loan in list_loans(conn, profile_id):
        legs = []
        for leg in loan.get("legs", []):
            effective = effective_leg_role(leg["role"], loan.get("status"))
            legs.append(
                {
                    "role": leg["role"],
                    "effective_role": effective,
                    "tax_effect": _ROLE_TAX_EFFECT.get(effective, "review"),
                    "transaction_id": leg.get("transaction_id"),
                    "escrow_txid": leg.get("escrow_txid"),
                    "amount": leg.get("amount"),
                    "occurred_at": leg.get("occurred_at"),
                    "on_chain": bool(leg.get("on_chain_present")),
                }
            )
        report_loans.append(
            {
                "id": loan["id"],
                "platform": loan.get("platform"),
                "role": loan.get("role"),
                "custody_type": loan.get("custody_type"),
                "rehypothecation": loan.get("rehypothecation"),
                "control_mechanism": loan.get("control_mechanism"),
                "status": loan.get("status"),
                "public_offering": bool(loan.get("public_offering")),
                "principal_asset": loan.get("principal_asset"),
                "principal_amount": loan.get("principal_amount"),
                "collateral_asset": loan.get("collateral_asset"),
                "interest_asset": loan.get("interest_asset"),
                "interest_terms": loan.get("interest_terms"),
                "as_of_custody_date": loan.get("as_of_custody_date"),
                "legs": legs,
                "escrow_positions": list_escrow_positions(conn, profile_id, loan_id=loan["id"]),
                "advisory": loan_advisory(loan),
            }
        )
    return {
        "workspace": profile.get("workspace_label") or profile.get("workspace_id"),
        "profile": profile.get("label") or profile.get("id"),
        "jurisdiction": str(profile.get("tax_country") or "generic").lower(),
        "fiat_currency": str(profile.get("fiat_currency") or "EUR").upper(),
        "loans": report_loans,
        "review_gate": (
            "Advisory only. The tax treatment of crypto-collateralized loans has no published "
            "BMF position; contested points are flagged per loan and must be confirmed by a "
            "Steuerberater (a verbindliche Auskunft, §118 BAO, gives certainty)."
        ),
    }
