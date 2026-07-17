"""Component-native transaction-pair and direct-payout review operations.

This is the single mutation boundary for the swap-matching review surface.
Callers resolve workspace/profile/transaction references, then pass those
database rows here; this module owns custody policy, conflicts, revision
history, journal invalidation, and the public review projection.
"""

from __future__ import annotations

from typing import Any, Mapping
import uuid

from . import custody_authored_migration, transfer_matching
from .custody_evidence import row_principal_msat
from .repo import invalidate_journals
from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..tax_policy import (
    cross_asset_carrying_value_supported,
    recommended_pair_policy,
)
from ..time_utils import now_iso
from ..transfers import bitcoin_network_domain_evidence
from ..wallet_descriptors import normalize_asset_code


BITCOIN_LAYER_TRANSITION_PAIR_KINDS = (
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
)
TRANSFER_PAIR_KINDS = (
    "manual",
    "coinjoin",
    "whirlpool",
    *BITCOIN_LAYER_TRANSITION_PAIR_KINDS,
)
TRANSFER_PAIR_POLICIES = ("carrying-value", "taxable")
DIRECT_SWAP_PAYOUT_KINDS = ("direct-swap-payout",)
REUSABLE_SAME_ASSET_PAIR_KINDS = ("manual", "coinjoin", "whirlpool")
PAIR_SOURCE_VALUES = ("manual", "bulk_exact", "bulk_selected", "rule_auto")

UNSET = object()


def _pair_stores_swap_fee(out_row, in_row, kind: str) -> bool:
    if out_row["asset"] != in_row["asset"]:
        return True
    return kind in BITCOIN_LAYER_TRANSITION_PAIR_KINDS


def _pair_allows_leg_reuse(out_asset, in_asset, kind, policy) -> bool:
    return (
        str(out_asset).upper() == str(in_asset).upper()
        and policy == "carrying-value"
        and kind in REUSABLE_SAME_ASSET_PAIR_KINDS
    )


def _review_ref_uses_transaction(review, transaction_ids) -> bool:
    return bool(
        {
            review.get("out_transaction_id"),
            review.get("in_transaction_id"),
        }
        & set(transaction_ids)
    )


def _raise_leg_reuse_conflict(existing_pair, out_transaction_id) -> None:
    reused_leg = (
        "--tx-out"
        if existing_pair["out_transaction_id"] == out_transaction_id
        else "--tx-in"
    )
    raise AppError(
        f"{reused_leg} already belongs to active pair id={existing_pair['id']}. "
        "Only same-asset manual, coinjoin, or whirlpool carrying-value links "
        "may reuse a transaction leg; cross-asset and layer-transition pairs "
        "must remain one-to-one.",
        code="conflict",
        hint=(
            f"Unpair `{existing_pair['id']}` first, or use a same-asset "
            "privacy/manual pair kind."
        ),
    )


def _reject_disallowed_leg_reuse(
    conn,
    profile_id,
    out_transaction_id,
    in_transaction_id,
    out_asset,
    in_asset,
    kind,
    policy,
    *,
    exclude_pair_id=None,
    review_refs=None,
) -> None:
    refs = review_refs
    if refs is None:
        refs = custody_authored_migration.list_active_review_refs(
            conn, profile_id=profile_id
        )
    new_pair_allows_reuse = _pair_allows_leg_reuse(
        out_asset, in_asset, kind, policy
    )
    for existing_pair in refs:
        if (
            existing_pair["term_kind"] != "transaction_pair"
            or existing_pair["id"] == exclude_pair_id
            or (
                existing_pair["out_transaction_id"] != out_transaction_id
                and existing_pair["in_transaction_id"] != in_transaction_id
            )
        ):
            continue
        existing_pair_allows_reuse = _pair_allows_leg_reuse(
            existing_pair["out_asset"],
            existing_pair["in_asset"],
            existing_pair["kind"],
            existing_pair["policy"],
        )
        if not (new_pair_allows_reuse and existing_pair_allows_reuse):
            _raise_leg_reuse_conflict(existing_pair, out_transaction_id)


def _raise_non_pair_review_conflict(review) -> None:
    review_id = review["id"]
    if review["term_kind"] == "direct_swap_payout":
        raise AppError(
            "One of the transactions already has an active direct swap payout "
            f"(id={review_id}). Delete that payout review before pairing.",
            code="conflict",
            hint=(
                "Run `kassiber transfers payouts delete --payout-id "
                f"{review_id}` first."
            ),
        )
    raise AppError(
        "One of the transactions belongs to active custody component "
        f"{review['component_id']}.",
        code="conflict",
        hint="Reopen or supersede that custody review before creating a pair.",
        details={"component_id": review["component_id"]},
    )


def pair_to_dict(row) -> dict[str, Any]:
    keys = set(row.keys()) if hasattr(row, "keys") else set()
    swap_fee_msat = row["swap_fee_msat"] if "swap_fee_msat" in keys else None
    swap_fee_kind = row["swap_fee_kind"] if "swap_fee_kind" in keys else None
    confidence = (
        row["confidence_at_pair"] if "confidence_at_pair" in keys else None
    )
    pair_source = row["pair_source"] if "pair_source" in keys else None
    deleted_at = row["deleted_at"] if "deleted_at" in keys else None
    out_amount = row["out_amount"] if "out_amount" in keys else None
    return {
        "id": row["id"],
        "component_id": row["component_id"] if "component_id" in keys else None,
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "in_transaction_id": row["in_transaction_id"],
        "kind": row["kind"],
        "policy": row["policy"],
        "notes": row["notes"],
        "swap_fee_msat": int(swap_fee_msat) if swap_fee_msat is not None else None,
        "swap_fee_kind": swap_fee_kind,
        "confidence_at_pair": confidence,
        "pair_source": pair_source,
        "out_amount": int(out_amount) if out_amount is not None else None,
        "deleted_at": deleted_at,
        "created_at": row["created_at"],
    }


def payout_to_dict(row) -> dict[str, Any]:
    keys = set(row.keys()) if hasattr(row, "keys") else set()
    payout_fiat_value = (
        row["payout_fiat_value"] if "payout_fiat_value" in keys else None
    )
    swap_fee_msat = row["swap_fee_msat"] if "swap_fee_msat" in keys else None
    out_amount = row["out_amount"] if "out_amount" in keys else None
    return {
        "id": row["id"],
        "component_id": row["component_id"] if "component_id" in keys else None,
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "kind": row["kind"],
        "policy": row["policy"],
        "payout_asset": row["payout_asset"],
        "payout_amount": float(msat_to_btc(row["payout_amount"])),
        "payout_amount_msat": int(row["payout_amount"]),
        "payout_occurred_at": row["payout_occurred_at"],
        "payout_fiat_value": (
            float(payout_fiat_value) if payout_fiat_value is not None else None
        ),
        "payout_external_id": row["payout_external_id"],
        "counterparty": row["counterparty"],
        "notes": row["notes"],
        "swap_fee_msat": int(swap_fee_msat) if swap_fee_msat is not None else None,
        "swap_fee_kind": row["swap_fee_kind"],
        "out_amount": int(out_amount) if out_amount is not None else None,
        "deleted_at": row["deleted_at"],
        "created_at": row["created_at"],
    }


def _positive_btc_amount_msat(value, flag_name: str) -> int:
    amount = dec(value)
    if amount <= 0:
        raise AppError(f"{flag_name} must be positive", code="validation")
    return btc_to_msat(amount)


def _transaction_pair_identity_row(conn, row) -> dict[str, Any]:
    payload = dict(row)
    wallet = conn.execute(
        "SELECT kind, config_json FROM wallets WHERE id = ?", (row["wallet_id"],)
    ).fetchone()
    if wallet is not None:
        payload["wallet_kind"] = wallet["kind"]
        payload["config_json"] = wallet["config_json"]
    return payload


def _validate_carrying_pair_network(conn, out_row, in_row, policy) -> None:
    if policy != "carrying-value":
        return
    out_domain, out_valid = bitcoin_network_domain_evidence(
        _transaction_pair_identity_row(conn, out_row)
    )
    in_domain, in_valid = bitcoin_network_domain_evidence(
        _transaction_pair_identity_row(conn, in_row)
    )
    if not out_valid or not in_valid:
        raise AppError(
            "A carrying-value pair has contradictory Bitcoin network metadata.",
            code="transfer_network_mismatch",
            hint=(
                "Correct the wallet/transaction chain and network metadata "
                "before pairing; conflicting observations cannot carry basis."
            ),
            details={
                "out_transaction_id": out_row["id"],
                "in_transaction_id": in_row["id"],
                "out_network_valid": out_valid,
                "in_network_valid": in_valid,
            },
        )
    if out_domain is None or in_domain is None or out_domain == in_domain:
        return
    raise AppError(
        "A carrying-value pair cannot cross Bitcoin network boundaries "
        f"({out_domain} -> {in_domain}).",
        code="transfer_network_mismatch",
        hint=(
            "Correct the wallet/transaction network metadata or leave these "
            "transactions unpaired; mainnet, testnet, signet, and regtest are "
            "distinct physical value domains."
        ),
        details={
            "out_transaction_id": out_row["id"],
            "in_transaction_id": in_row["id"],
            "out_network_domain": out_domain,
            "in_network_domain": in_domain,
        },
    )


def _outbound_fee_component_msat(row, *, split_pair=False) -> int:
    if split_pair:
        return 0
    try:
        if row["amount_includes_fee"]:
            return 0
        return max(0, int(row["fee"] or 0))
    except (TypeError, ValueError, IndexError, KeyError):
        return 0


def _validate_pair_policy(
    profile,
    out_asset,
    in_asset,
    policy,
    *,
    reject_same_asset_taxable=True,
) -> None:
    same_asset = str(out_asset).upper() == str(in_asset).upper()
    if same_asset and policy == "taxable" and reject_same_asset_taxable:
        raise AppError(
            "Same-asset taxable pairs are not supported yet "
            f"(asset={out_asset}). Leave the legs unpaired to keep normal "
            "SELL + BUY treatment, or use --policy carrying-value for a "
            "self-transfer.",
            code="validation",
            hint=(
                "Re-run with --policy carrying-value, or omit the pair entirely "
                "to preserve taxable SELL + BUY behavior."
            ),
        )
    if not same_asset and policy == "carrying-value":
        tax_country = str(profile["tax_country"] or "").strip().lower()
        if not cross_asset_carrying_value_supported(
            tax_country, out_asset, in_asset
        ):
            raise AppError(
                "Cross-asset carrying-value pairs are only supported for "
                "Austrian profiles or BTC/LBTC rail swaps right now "
                f"(out={out_asset}, in={in_asset}). Use --policy taxable for "
                "other cross-asset swaps.",
                code="validation",
                hint=(
                    "Re-run with --policy taxable, or pair only BTC/LBTC rail "
                    "changes as carrying-value outside Austrian profiles."
                ),
            )


def create_pair_review(
    conn,
    *,
    workspace_id: str,
    profile: Mapping[str, Any],
    out_row,
    in_row,
    kind="manual",
    policy=None,
    notes=None,
    pair_source="manual",
    confidence_at_pair=None,
    out_amount=None,
    commit=True,
    authored_source="cli",
) -> dict[str, Any]:
    if kind not in TRANSFER_PAIR_KINDS:
        raise AppError(
            f"Unsupported pair kind '{kind}'. Supported: "
            f"{', '.join(TRANSFER_PAIR_KINDS)}",
            code="validation",
        )
    if policy is not None and policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported pair policy '{policy}'. Supported: "
            f"{', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )
    if pair_source not in PAIR_SOURCE_VALUES:
        raise AppError(
            f"Unsupported pair_source '{pair_source}'. Supported: "
            f"{', '.join(PAIR_SOURCE_VALUES)}",
            code="validation",
        )
    if out_row["id"] == in_row["id"]:
        raise AppError(
            "--tx-out and --tx-in must reference different transactions",
            code="validation",
        )
    if policy is None:
        policy = (
            "carrying-value"
            if str(out_row["asset"]).upper() == str(in_row["asset"]).upper()
            else recommended_pair_policy(
                profile, out_row["asset"], in_row["asset"]
            )
        )
    _validate_carrying_pair_network(conn, out_row, in_row, policy)
    _validate_pair_policy(profile, out_row["asset"], in_row["asset"], policy)
    out_amount_msat = None
    if out_amount is not None:
        if out_row["asset"] == in_row["asset"]:
            raise AppError(
                "--out-amount only applies to cross-asset swap pairs: it is "
                "the portion of the outbound that was swapped, with the "
                "remainder treated as a same-asset self-transfer.",
                code="validation",
            )
        out_amount_msat = _positive_btc_amount_msat(out_amount, "--out-amount")
        full_out_msat = row_principal_msat(out_row)
        if out_amount_msat > full_out_msat:
            raise AppError(
                "--out-amount exceeds the outbound amount "
                f"({out_amount_msat} > {full_out_msat} msat).",
                code="validation",
            )
    review_refs = custody_authored_migration.list_active_review_refs(
        conn, profile_id=profile["id"]
    )
    existing = next(
        (
            row
            for row in review_refs
            if row["term_kind"] == "transaction_pair"
            and row["out_transaction_id"] == out_row["id"]
            and row["in_transaction_id"] == in_row["id"]
        ),
        None,
    )
    if existing:
        raise AppError(
            "Those transactions are already paired "
            f"(pair id={existing['id']}). Run `kassiber transfers unpair "
            f"--pair-id {existing['id']}` first.",
            code="conflict",
        )
    conflicting_review = next(
        (
            row
            for row in review_refs
            if row["term_kind"] != "transaction_pair"
            and _review_ref_uses_transaction(
                row, {out_row["id"], in_row["id"]}
            )
        ),
        None,
    )
    if conflicting_review:
        _raise_non_pair_review_conflict(conflicting_review)
    _reject_disallowed_leg_reuse(
        conn,
        profile["id"],
        out_row["id"],
        in_row["id"],
        out_row["asset"],
        in_row["asset"],
        kind,
        policy,
        review_refs=review_refs,
    )
    if _pair_stores_swap_fee(out_row, in_row, kind):
        split_pair = out_amount_msat is not None
        fee_source = out_amount_msat if split_pair else int(out_row["amount"] or 0)
        swap_fee_msat, swap_fee_kind = transfer_matching.compute_swap_fee(
            fee_source,
            int(in_row["amount"] or 0),
            _outbound_fee_component_msat(out_row, split_pair=split_pair),
        )
    else:
        swap_fee_msat, swap_fee_kind = None, None
    pair_row = custody_authored_migration.create_pair_review_component(
        conn,
        review_id=str(uuid.uuid4()),
        workspace_id=workspace_id,
        profile_id=profile["id"],
        out_transaction_id=out_row["id"],
        in_transaction_id=in_row["id"],
        kind=kind,
        policy=policy,
        notes=notes,
        swap_fee_msat=swap_fee_msat,
        swap_fee_kind=swap_fee_kind,
        confidence_at_pair=confidence_at_pair,
        pair_source=pair_source,
        out_amount_msat=out_amount_msat,
        created_at=now_iso(),
        authored_source=authored_source,
    )
    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return pair_to_dict(pair_row)


def create_payout_review(
    conn,
    *,
    workspace_id: str,
    profile: Mapping[str, Any],
    out_row,
    payout_asset,
    payout_amount,
    kind="direct-swap-payout",
    policy=None,
    payout_occurred_at=None,
    payout_fiat_value=None,
    payout_external_id=None,
    counterparty=None,
    notes=None,
    out_amount=None,
    commit=True,
    authored_source="cli",
) -> dict[str, Any]:
    if kind not in DIRECT_SWAP_PAYOUT_KINDS:
        raise AppError(
            f"Unsupported direct payout kind '{kind}'. Supported: "
            f"{', '.join(DIRECT_SWAP_PAYOUT_KINDS)}",
            code="validation",
        )
    if policy is not None and policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported direct payout policy '{policy}'. Supported: "
            f"{', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )
    target_asset = normalize_asset_code(payout_asset)
    if not target_asset:
        raise AppError("--payout-asset is required", code="validation")
    payout_amount_msat = _positive_btc_amount_msat(
        payout_amount, "--payout-amount"
    )
    out_amount_msat = None
    if out_amount is not None:
        out_amount_msat = _positive_btc_amount_msat(out_amount, "--out-amount")
        full_out_msat = row_principal_msat(out_row)
        if out_amount_msat > full_out_msat:
            raise AppError(
                "--out-amount exceeds the outbound amount "
                f"({out_amount_msat} > {full_out_msat} msat).",
                code="validation",
            )
    payout_value = dec(payout_fiat_value) if payout_fiat_value is not None else None
    if payout_value is not None and payout_value < 0:
        raise AppError("--payout-fiat-value must not be negative", code="validation")
    if policy is None:
        policy = (
            "carrying-value"
            if str(out_row["asset"]).upper() == target_asset
            else recommended_pair_policy(profile, out_row["asset"], target_asset)
        )
    _validate_pair_policy(
        profile,
        out_row["asset"],
        target_asset,
        policy,
        reject_same_asset_taxable=False,
    )
    review_refs = custody_authored_migration.list_active_review_refs(
        conn, profile_id=profile["id"]
    )
    existing = next(
        (
            row
            for row in review_refs
            if _review_ref_uses_transaction(row, {out_row["id"]})
        ),
        None,
    )
    if existing and existing["term_kind"] == "transaction_pair":
        raise AppError(
            f"Transaction is already paired (pair id={existing['id']}). Run "
            "`kassiber transfers unpair --pair-id "
            f"{existing['id']}` first.",
            code="conflict",
        )
    if existing and existing["term_kind"] == "direct_swap_payout":
        raise AppError(
            "Transaction already has an active direct swap payout "
            f"(id={existing['id']}).",
            code="conflict",
            hint="Delete the existing payout review before creating a replacement.",
        )
    if existing:
        raise AppError(
            "Transaction belongs to active custody component "
            f"{existing['component_id']}.",
            code="conflict",
            hint=(
                "Reopen or supersede that custody review before creating a "
                "direct payout."
            ),
            details={"component_id": existing["component_id"]},
        )
    swap_fee_msat, swap_fee_kind = transfer_matching.compute_swap_fee(
        out_amount_msat if out_amount_msat is not None else row_principal_msat(out_row),
        payout_amount_msat,
        _outbound_fee_component_msat(
            out_row, split_pair=out_amount_msat is not None
        ),
    )
    payout_row = custody_authored_migration.create_payout_review_component(
        conn,
        review_id=str(uuid.uuid4()),
        workspace_id=workspace_id,
        profile_id=profile["id"],
        out_transaction_id=out_row["id"],
        kind=kind,
        policy=policy,
        payout_asset=target_asset,
        payout_amount_msat=payout_amount_msat,
        payout_occurred_at=payout_occurred_at,
        payout_fiat_value=(
            float(payout_value) if payout_value is not None else None
        ),
        payout_external_id=payout_external_id,
        counterparty=counterparty,
        notes=notes,
        swap_fee_msat=swap_fee_msat,
        swap_fee_kind=swap_fee_kind,
        out_amount_msat=out_amount_msat,
        created_at=now_iso(),
        authored_source=authored_source,
    )
    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return payout_to_dict(payout_row)


def list_payout_reviews(
    conn, profile_id: str, *, include_deleted=False
) -> list[dict[str, Any]]:
    output = []
    for row in custody_authored_migration.list_payout_review_records(
        conn, profile_id=profile_id, include_deleted=include_deleted
    ):
        entry = payout_to_dict(row)
        entry["out"] = {
            "transaction_id": row["out_transaction_id"],
            "external_id": row["out_external_id"] or "",
            "wallet": row["out_wallet"],
            "asset": row["out_asset"],
            "amount": float(msat_to_btc(row["reviewed_out_amount_msat"])),
            "amount_msat": int(row["reviewed_out_amount_msat"]),
            "full_amount": float(msat_to_btc(row["full_out_amount_msat"])),
            "full_amount_msat": int(row["full_out_amount_msat"]),
            "occurred_at": row["out_occurred_at"],
        }
        entry["payout"] = {
            "asset": row["payout_asset"],
            "amount": float(msat_to_btc(row["payout_amount"])),
            "amount_msat": int(row["payout_amount"]),
            "occurred_at": row["payout_occurred_at"] or row["out_occurred_at"],
            "external_id": row["payout_external_id"],
            "counterparty": row["counterparty"],
        }
        output.append(entry)
    return output


def list_pair_reviews(
    conn, profile_id: str, *, include_deleted=False
) -> list[dict[str, Any]]:
    output = []
    for row in custody_authored_migration.list_pair_review_records(
        conn, profile_id=profile_id, include_deleted=include_deleted
    ):
        entry = pair_to_dict(row)
        entry["out"] = {
            "transaction_id": row["out_transaction_id"],
            "external_id": row["out_external_id"] or "",
            "wallet": row["out_wallet"],
            "wallet_kind": row["out_wallet_kind"],
            "asset": row["out_asset"],
            "occurred_at": row["out_occurred_at"],
            "amount": float(msat_to_btc(row["out_amount_msat"])),
            "amount_msat": int(row["out_amount_msat"]),
            "full_amount": float(msat_to_btc(row["out_full_amount_msat"])),
            "full_amount_msat": int(row["out_full_amount_msat"]),
        }
        entry["in"] = {
            "transaction_id": row["in_transaction_id"],
            "external_id": row["in_external_id"] or "",
            "wallet": row["in_wallet"],
            "wallet_kind": row["in_wallet_kind"],
            "asset": row["in_asset"],
            "occurred_at": row["in_occurred_at"],
            "amount": float(msat_to_btc(row["in_amount_msat"])),
            "amount_msat": int(row["in_amount_msat"]),
        }
        output.append(entry)
    return output


def delete_payout_review(
    conn,
    profile_id: str,
    payout_id: str,
    *,
    commit=True,
    authored_source="cli",
) -> dict[str, Any]:
    row = next(
        (
            item
            for item in custody_authored_migration.list_payout_review_records(
                conn, profile_id=profile_id, include_deleted=True
            )
            if item["id"] == payout_id
        ),
        None,
    )
    if not row:
        raise AppError("Direct swap payout not found", code="not_found")
    if row["deleted_at"]:
        return payout_to_dict(row)
    deleted = {**row, "deleted_at": now_iso()}
    custody_authored_migration.delete_authored_review(
        conn,
        profile_id=profile_id,
        review_id=payout_id,
        term_kind="direct_swap_payout",
        deleted_at=deleted["deleted_at"],
        authored_source=authored_source,
    )
    invalidate_journals(conn, profile_id)
    if commit:
        conn.commit()
    return payout_to_dict(deleted)


def delete_pair_review(
    conn,
    profile_id: str,
    pair_id: str,
    *,
    commit=True,
    authored_source="cli",
) -> dict[str, str]:
    row = next(
        (
            item
            for item in custody_authored_migration.list_pair_review_records(
                conn, profile_id=profile_id, include_deleted=True
            )
            if item["id"] == pair_id
        ),
        None,
    )
    if not row:
        if custody_authored_migration.authored_review_exists(
            conn,
            profile_id=profile_id,
            review_id=pair_id,
            term_kind="transaction_pair",
        ):
            return {"deleted": pair_id}
        raise AppError(f"Pair '{pair_id}' not found", code="not_found")
    if row["deleted_at"]:
        return {"deleted": pair_id}
    custody_authored_migration.delete_authored_review(
        conn,
        profile_id=profile_id,
        review_id=pair_id,
        term_kind="transaction_pair",
        deleted_at=now_iso(),
        authored_source=authored_source,
    )
    invalidate_journals(conn, profile_id)
    if commit:
        conn.commit()
    return {"deleted": pair_id}


def update_pair_review(
    conn,
    *,
    profile: Mapping[str, Any],
    pair_id: str,
    kind=None,
    policy=None,
    notes=UNSET,
    commit=True,
    authored_source="cli",
) -> dict[str, Any]:
    row = next(
        (
            item
            for item in custody_authored_migration.list_pair_review_records(
                conn, profile_id=profile["id"]
            )
            if item["id"] == pair_id
        ),
        None,
    )
    if not row:
        raise AppError(f"Pair '{pair_id}' not found", code="not_found")
    new_kind = row["kind"] if kind is None else kind
    new_policy = row["policy"] if policy is None else policy
    if new_kind not in TRANSFER_PAIR_KINDS:
        raise AppError(
            f"Unsupported pair kind '{new_kind}'. Supported: "
            f"{', '.join(TRANSFER_PAIR_KINDS)}",
            code="validation",
        )
    if new_policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported pair policy '{new_policy}'. Supported: "
            f"{', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )
    _validate_pair_policy(
        profile, row["out_asset"], row["in_asset"], new_policy
    )
    out_row = conn.execute(
        "SELECT * FROM transactions WHERE id = ?", (row["out_transaction_id"],)
    ).fetchone()
    in_row = conn.execute(
        "SELECT * FROM transactions WHERE id = ?", (row["in_transaction_id"],)
    ).fetchone()
    if out_row is not None and in_row is not None:
        _validate_carrying_pair_network(conn, out_row, in_row, new_policy)
    _reject_disallowed_leg_reuse(
        conn,
        profile["id"],
        row["out_transaction_id"],
        row["in_transaction_id"],
        row["out_asset"],
        row["in_asset"],
        new_kind,
        new_policy,
        exclude_pair_id=pair_id,
    )
    new_notes = row["notes"] if notes is UNSET else notes
    if (
        new_kind == row["kind"]
        and new_policy == row["policy"]
        and new_notes == row["notes"]
    ):
        return pair_to_dict(row)
    new_fee_msat = row["swap_fee_msat"]
    new_fee_kind = row["swap_fee_kind"]
    if new_kind != row["kind"]:
        if out_row and in_row and _pair_stores_swap_fee(out_row, in_row, new_kind):
            split_pair = (
                row["out_asset"] != row["in_asset"]
                and row["out_amount"] is not None
            )
            fee_source = (
                int(row["out_amount"])
                if split_pair
                else int(out_row["amount"] or 0)
            )
            new_fee_msat, new_fee_kind = transfer_matching.compute_swap_fee(
                fee_source,
                int(in_row["amount"] or 0),
                _outbound_fee_component_msat(out_row, split_pair=split_pair),
            )
        else:
            new_fee_msat, new_fee_kind = None, None
    updated = custody_authored_migration.revise_pair_review_component(
        conn,
        row,
        kind=new_kind,
        policy=new_policy,
        notes=new_notes,
        swap_fee_msat=new_fee_msat,
        swap_fee_kind=new_fee_kind,
        authored_source=authored_source,
    )
    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return pair_to_dict(updated)


__all__ = [
    "BITCOIN_LAYER_TRANSITION_PAIR_KINDS",
    "DIRECT_SWAP_PAYOUT_KINDS",
    "PAIR_SOURCE_VALUES",
    "TRANSFER_PAIR_KINDS",
    "TRANSFER_PAIR_POLICIES",
    "UNSET",
    "create_pair_review",
    "create_payout_review",
    "delete_pair_review",
    "delete_payout_review",
    "list_pair_reviews",
    "list_payout_reviews",
    "update_pair_review",
]
