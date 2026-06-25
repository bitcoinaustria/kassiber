"""Phase 2 import on-ramps for Bitcoin-backed loans.

The chain carries no loan semantics for any provider (verified across Firefish,
Hodl Hodl, Unchained, Ledn, Nexo, ...), so import is heuristic / quarantine-first
— never auto-detection. An on-chain leg is only *booked* (affects tax) once it
resolves to a real journal transaction; rows that don't resolve are returned as
``unresolved`` for the user to reconcile (import the wallet, then re-run), never
silently turned into a disposal.

Each importer is a thin populator in front of ``kassiber.core.loans`` CRUD,
dispatched by format. A drifted/unknown export degrades to ``unresolved`` +
warnings rather than mis-mapping.
"""

from __future__ import annotations

import csv
import io
import json
from decimal import Decimal, InvalidOperation
from typing import Any, Mapping, Optional

from ..errors import AppError
from ..msat import btc_to_msat
from . import loans as core_loans

# --- tolerant column access (mirrors importers.py idioms) ------------------


def _norm_key(value: Any) -> str:
    return " ".join(str(value).replace("\xa0", " ").strip().split()).casefold()


def _cell(record: Mapping[str, Any], *names: str) -> Optional[str]:
    folded = {_norm_key(k): v for k, v in record.items() if k is not None}
    for name in names:
        value = folded.get(_norm_key(name))
        if value not in (None, ""):
            return str(value)
    return None


def _parse_btc(text: Optional[str]) -> Optional[int]:
    """Parse an amount cell into msat. Accepts a `BTC` or `sats` suffix, and
    tolerates a European decimal comma ("1,5" = 1.5) vs US thousands separators
    ("1,000.5"). The leg amount is informational — it never drives tax quantity
    or basis (the engine sees only txid/role/status) — but it must still display
    correctly."""
    if text in (None, ""):
        return None
    cleaned = str(text).strip()
    is_sats = False
    low = cleaned.lower()
    for suffix in ("btc", "sats"):
        if low.endswith(suffix):
            cleaned = cleaned[: -len(suffix)].strip()
            is_sats = suffix == "sats"
            break
    # European decimal comma (no period present) -> dot; otherwise commas are
    # thousands separators and are dropped.
    if "," in cleaned and "." not in cleaned:
        cleaned = cleaned.replace(",", ".")
    else:
        cleaned = cleaned.replace(",", "")
    try:
        value = abs(Decimal(cleaned))
    except (InvalidOperation, ValueError):
        return None
    return int(value * 1000) if is_sats else btc_to_msat(value)


# Type text -> loan-leg role. Order matters (most specific first).
_ROLE_KEYWORDS = (
    ("liquidation", ("liquidat", "seiz", "default", "margin call sold")),
    ("collateral_release", ("collateral release", "release collateral", "collateral returned", "collateral withdrawal", "unlock", "release")),
    ("collateral_topup", ("top-up", "top up", "topup", "add collateral", "margin top")),
    ("collateral_lock", ("collateral deposit", "collateral in", "lock", "post collateral", "fund", "collateral")),
    ("principal_repay", ("repay", "repayment", "loan payment", "principal payment")),
    ("interest_payment", ("interest",)),
    ("principal_draw", ("disburs", "principal", "loan advance", "borrow", "loan funded", "payout")),
)


def _infer_role(type_text: Optional[str]) -> Optional[str]:
    text = (type_text or "").lower()
    if not text:
        return None
    for role, keywords in _ROLE_KEYWORDS:
        if any(kw in text for kw in keywords):
            return role
    return None


def _resolve_journal_txid(conn, profile_id: str, txid: Optional[str]) -> Optional[str]:
    """Return the journal transaction id for a txid/external_id, or None."""
    if not txid:
        return None
    row = conn.execute(
        "SELECT id FROM transactions WHERE profile_id = ? AND (id = ? OR external_id = ?) AND excluded = 0",
        (profile_id, txid, txid),
    ).fetchone()
    return row["id"] if row else None


def _seed_loan(conn, workspace_id, profile_id, *, platform, preset, loan_id, custody_default=None):
    if loan_id is not None:
        loan = core_loans.get_loan(conn, profile_id, loan_id)
        if loan is None:
            raise AppError(f"Loan '{loan_id}' not found", code="not_found")
        return loan
    return core_loans.create_loan(
        conn,
        workspace_id,
        profile_id,
        platform=platform,
        preset_id=preset,
        custody_type=None if preset else custody_default,
        commit=False,
    )


# --- CSV (Ledn / Nexo / generic provider export) ---------------------------


def import_csv(conn, workspace_id, profile_id, *, file_text, platform=None, preset=None, loan_id=None):
    rows = list(csv.DictReader(io.StringIO(file_text)))
    if not rows:
        raise AppError("Loan CSV has no rows", code="validation")
    loan = _seed_loan(conn, workspace_id, profile_id, platform=platform, preset=preset, loan_id=loan_id)

    legs: list[dict[str, Any]] = []
    unresolved: list[dict[str, Any]] = []
    warnings: list[str] = []
    for index, record in enumerate(rows, start=1):
        type_text = _cell(record, "type", "transaction type", "activity", "kind", "category")
        role = _infer_role(type_text)
        if role is None:
            warnings.append(f"Row {index}: unrecognized type {type_text!r} — skipped (review manually).")
            continue
        asset = (_cell(record, "asset", "currency", "coin") or "BTC").upper()
        amount_msat = _parse_btc(_cell(record, "amount", "quantity", "collateral", "btc amount"))
        occurred_at = _cell(record, "date", "timestamp", "time", "created at")
        txid = _cell(record, "txid", "transaction id", "tx id", "hash", "reference", "on-chain transaction")
        resolved = _resolve_journal_txid(conn, profile_id, txid)
        on_chain_required = role in core_loans.ONCHAIN_REQUIRED_ROLES or (role == "interest_payment" and asset == "BTC")

        if on_chain_required and resolved is None:
            unresolved.append(
                {"row": index, "role": role, "asset": asset, "txid": txid, "occurred_at": occurred_at, "amount": amount_msat}
            )
            continue
        leg = core_loans.create_loan_leg(
            conn,
            workspace_id,
            profile_id,
            loan["id"],
            role=role,
            transaction_id=resolved,
            escrow_txid=txid if resolved is None else None,
            amount=amount_msat,
            occurred_at=occurred_at,
            on_chain_present=resolved is not None,
            notes=f"imported:{platform or preset or 'csv'}",
            commit=False,
        )
        legs.append(leg)
    conn.commit()
    return {"loan": loan, "legs": legs, "unresolved": unresolved, "warnings": warnings}


# --- Unchained / Caravan wallet-config descriptor --------------------------


def import_unchained_descriptor(conn, workspace_id, profile_id, *, file_text, platform=None, preset="unchained", loan_id=None):
    try:
        config = json.loads(file_text)
    except json.JSONDecodeError as exc:
        raise AppError(f"Invalid wallet-config JSON: {exc}", code="validation") from exc
    if not isinstance(config, dict):
        raise AppError("Wallet config must be a JSON object", code="validation")
    address_type = str(config.get("addressType") or config.get("address_type") or "unknown")
    descriptor = config.get("descriptor") or json.dumps(config, sort_keys=True)
    addresses = config.get("addresses") or []

    loan = _seed_loan(
        conn, workspace_id, profile_id, platform=platform or "Unchained", preset=preset, loan_id=loan_id,
        custody_default="collaborative_multisig",
    )
    # Record the escrow descriptor on the facility — as an ENCUMBERED reference,
    # NOT an owned wallet (so ownership reconciliation never treats escrow UTXOs
    # as spendable and auto-pairs a liquidation as a self-transfer).
    core_loans.update_loan(conn, profile_id, loan["id"], escrow_descriptor=str(descriptor), commit=False)

    positions = []
    seen = addresses if isinstance(addresses, list) else []
    for addr in seen or [None]:
        positions.append(
            core_loans.create_escrow_position(
                conn, workspace_id, profile_id, loan["id"],
                escrow_address=str(addr) if addr else None,
                output_type=address_type,
                commit=False,
            )
        )
    conn.commit()
    loan = core_loans.get_loan(conn, profile_id, loan["id"])
    return {
        "loan": loan,
        "positions": positions,
        "warnings": [
            "Imported as a read-only ENCUMBERED descriptor — not an owned wallet. "
            "Label the lock/release/liquidation legs against the collateral transactions to drive tax."
        ],
    }


# --- Hodl Hodl escrow object (REST) ----------------------------------------


def import_hodlhodl_escrow(conn, workspace_id, profile_id, *, file_text, platform=None, preset="hodlhodl", loan_id=None):
    try:
        escrow = json.loads(file_text)
    except json.JSONDecodeError as exc:
        raise AppError(f"Invalid escrow JSON: {exc}", code="validation") from exc
    if not isinstance(escrow, dict):
        raise AppError("Escrow object must be a JSON object", code="validation")
    address = escrow.get("address") or escrow.get("escrow_address")
    deposit_txid = escrow.get("deposit_transaction_id") or escrow.get("deposit_txid")
    release_txid = escrow.get("release_transaction_id") or escrow.get("release_txid")

    loan = _seed_loan(
        conn, workspace_id, profile_id, platform=platform or "Hodl Hodl Lend", preset=preset, loan_id=loan_id,
        custody_default="non_custodial_multisig",
    )

    legs = []
    unresolved = []
    warnings = []
    for txid, role in ((deposit_txid, "collateral_lock"), (release_txid, "collateral_release")):
        if not txid:
            continue  # field-presence gate: a missing leg degrades to manual, no crash
        resolved = _resolve_journal_txid(conn, profile_id, txid)
        if resolved is None:
            unresolved.append({"role": role, "txid": txid})
            continue
        legs.append(
            core_loans.create_loan_leg(
                conn, workspace_id, profile_id, loan["id"],
                role=role, transaction_id=resolved, escrow_address=str(address) if address else None,
                notes="imported:hodlhodl", commit=False,
            )
        )
    if address:
        core_loans.create_escrow_position(
            conn, workspace_id, profile_id, loan["id"], escrow_address=str(address), output_type="unknown", commit=False
        )
    conn.commit()
    loan = core_loans.get_loan(conn, profile_id, loan["id"])
    return {"loan": loan, "legs": legs, "unresolved": unresolved, "warnings": warnings}


# --- BIP329 labels -> candidate leg roles ----------------------------------

_LABEL_ROLE_KEYWORDS = (
    ("liquidation", ("liquidat", "seiz")),
    ("collateral_release", ("release", "collateral back", "collateral return")),
    ("collateral_lock", ("collateral", "lock", "escrow")),
    ("interest_payment", ("interest",)),
)


def import_bip329(conn, workspace_id, profile_id, *, file_text, loan_id, platform=None, preset=None):
    if loan_id is None:
        raise AppError("BIP329 loan import needs --loan-id to attach legs to", code="validation")
    loan = core_loans.get_loan(conn, profile_id, loan_id)
    if loan is None:
        raise AppError(f"Loan '{loan_id}' not found", code="not_found")
    legs = []
    unresolved = []
    warnings = []
    for line_no, raw in enumerate(file_text.splitlines(), start=1):
        line = raw.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            warnings.append(f"Line {line_no}: invalid JSON — skipped.")
            continue
        if record.get("type") != "tx":
            continue
        label = str(record.get("label") or "").lower()
        role = next((r for r, kws in _LABEL_ROLE_KEYWORDS if any(kw in label for kw in kws)), None)
        if role is None:
            continue
        txid = record.get("ref")
        resolved = _resolve_journal_txid(conn, profile_id, txid)
        if resolved is None:
            unresolved.append({"role": role, "txid": txid, "label": record.get("label")})
            continue
        try:
            legs.append(
                core_loans.create_loan_leg(
                    conn, workspace_id, profile_id, loan_id,
                    role=role, transaction_id=resolved, notes=f"bip329:{record.get('label')}", commit=False,
                )
            )
        except AppError as exc:
            warnings.append(f"Line {line_no} ({txid}): {exc}")
    conn.commit()
    return {"loan": core_loans.get_loan(conn, profile_id, loan_id), "legs": legs, "unresolved": unresolved, "warnings": warnings}


LOAN_IMPORT_FORMATS = ("csv", "unchained", "hodlhodl", "bip329")

_IMPORTERS = {
    "csv": import_csv,
    "unchained": import_unchained_descriptor,
    "hodlhodl": import_hodlhodl_escrow,
    "bip329": import_bip329,
}


def import_loan(conn, workspace_id, profile_id, *, fmt, file_text, platform=None, preset=None, loan_id=None):
    """Dispatch a loan import by format. Quarantine-first: on-chain legs only book
    when they resolve to a real journal transaction."""
    importer = _IMPORTERS.get(fmt)
    if importer is None:
        raise AppError(
            f"Unknown loan import format '{fmt}'. Known: {', '.join(LOAN_IMPORT_FORMATS)}",
            code="validation",
        )
    # Atomic: an importer seeds a loan + legs across several commit=False inserts
    # and commits once at the end. If it raises mid-way, roll back the partial
    # work so a half-baked orphan loan can't be persisted by a later commit on a
    # long-lived (daemon) connection.
    try:
        return importer(
            conn, workspace_id, profile_id,
            file_text=file_text, platform=platform, preset=preset, loan_id=loan_id,
        )
    except Exception:
        conn.rollback()
        raise
