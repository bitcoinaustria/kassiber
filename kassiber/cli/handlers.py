import argparse
import base64
import binascii
import json
import os
import sqlite3
import sys
import uuid
from collections import defaultdict
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from functools import lru_cache
from pathlib import Path

from .. import __version__
from ..backends import (
    BACKEND_CLEAR_FIELD_ALIASES,
    BACKEND_KINDS,
    DEFAULT_ENV_FILENAME,
    DEFAULT_BACKENDS,
    _backend_row_to_dict,
    _validate_backend_kind,
    clear_default_backend,
    create_db_backend,
    delete_db_backend,
    get_db_backend,
    list_backends,
    list_db_backends,
    redact_backend_url,
    resolve_backend,
    set_default_backend,
    update_db_backend,
)
from ..core import accounts as core_accounts
from ..core import attachments as core_attachments
from ..core import commercial as core_commercial
from ..core import imports as core_imports
from ..core.lightning import cln as core_lightning_cln
from ..core import metadata as core_metadata
from ..core import output_inventory as core_output_inventory
from ..core import ownership as core_ownership
from ..core import chat_history as core_chat_history
from ..core import pricing
from ..core import rates as core_rates
from ..core import reports as core_reports
from ..core import saved_views as core_saved_views
from ..core import swap_rules as core_swap_rules
from ..core import sync as core_sync
from ..core import sync_backends as core_sync_backends
from ..core import tax_events as core_tax_events
from ..core import transfer_matching as core_transfer_matching
from ..core import wallets as core_wallets
from ..core.engines import TaxEngineLedgerInputs, build_tax_engine
from ..core.repo import current_context_snapshot, resolve_account
from ..core.runtime import (
    build_status_payload,
)
from ..db import (
    APP_NAME,
    DEFAULT_DATA_ROOT,
    SCHEMA,
    ensure_column,
    ensure_schema_compat,
    get_setting,
    resolve_attachments_root,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    resolve_effective_state_root,
    resolve_exports_root,
    resolve_settings_path,
    set_setting,
)
from ..envelope import (
    OUTPUT_FORMATS,
    build_envelope,
    derive_kind,
    emit,
    format_table_value,
    print_table,
)
from ..errors import AppError
from ..msat import (
    MSAT_PER_BTC,
    SATS_PER_BTC,
    btc_to_msat,
    dec,
    msat_to_btc,
)
from ..pdf_report import format_table, write_text_pdf
from ..time_utils import (
    UNKNOWN_OCCURRED_AT,
    _iso_z,
    _parse_iso_datetime,
    now_iso,
    timestamp_to_iso,
)
from ..util import (
    normalize_chain_value,
    normalize_network_value,
    parse_int,
)
from ..tax_policy import (
    DEFAULT_LONG_TERM_DAYS,
    DEFAULT_TAX_COUNTRY,
    build_tax_policy,
    require_tax_country_supported_for_profile_mutation,
    require_tax_processing_supported,
)
from ..wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    MAX_DESCRIPTOR_GAP_LIMIT,
    derive_descriptor_targets,
    liquid_plan_can_unblind,
    load_descriptor_plan,
    normalize_asset_code,
    normalize_chain,
    normalize_network,
)
from ..importers import load_import_records
from ..sync_btcpay import (
    DEFAULT_PAGE_SIZE as BTCPAY_DEFAULT_PAGE_SIZE,
    DEFAULT_PAYMENT_METHOD_ID as BTCPAY_DEFAULT_PAYMENT_METHOD_ID,
    fetch_btcpay_invoice_provenance,
    fetch_btcpay_records,
    require_wallet_history_payment_method,
)


ACCOUNT_TYPES = {"asset", "liability", "equity", "income", "expense"}
RP2_ACCOUNTING_METHODS = (
    "FIFO",
    "LIFO",
    "HIFO",
    "LOFO",
    "MOVING_AVERAGE",
    "MOVING_AVERAGE_AT",
)
WALLET_KINDS = [
    "descriptor",
    "xpub",
    "address",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
    "bullbitcoin",
    "coinfinity",
    "21bitcoin",
    "pocketbitcoin",
    "strike",
    "wasabi",
    "custom",
]

def normalize_code(value):
    code = str(value).strip().lower().replace(" ", "-")
    if not code:
        raise AppError("Code cannot be empty")
    return code


def normalize_wallet_kind(value):
    kind = str(value).strip().lower()
    if kind not in WALLET_KINDS:
        raise AppError(f"Unsupported wallet kind '{value}'. Supported: {', '.join(WALLET_KINDS)}")
    return kind


def normalize_addresses(values):
    if values is None:
        return []
    if isinstance(values, str):
        values = [part.strip() for part in values.split(",")]
    output = []
    seen = set()
    for value in values:
        address = str(value).strip()
        if not address or address in seen:
            continue
        seen.add(address)
        output.append(address)
    return output


def resolve_workspace(conn, ref=None):
    ref = ref or get_setting(conn, "context_workspace")
    if not ref:
        raise AppError("No workspace selected. Create one or run `kassiber context set --workspace ...`.")
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ? OR lower(label) = lower(?) LIMIT 1",
        (ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Workspace '{ref}' not found")
    return row


def resolve_profile(conn, workspace_id, ref=None):
    ref = ref or get_setting(conn, "context_profile")
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    row = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND (id = ? OR lower(label) = lower(?))
        LIMIT 1
        """,
        (workspace_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Profile '{ref}' not found in the selected workspace")
    return row


def resolve_scope(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    return workspace, profile


def resolve_wallet(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND (w.id = ? OR lower(w.label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Wallet '{ref}' not found")
    return row


def resolve_transaction(conn, profile_id, ref, direction=None):
    id_query = ["SELECT * FROM transactions WHERE profile_id = ? AND id = ?"]
    id_params = [profile_id, ref]
    if direction is not None:
        id_query.append("AND direction = ?")
        id_params.append(direction)
    row = conn.execute(" ".join(id_query), tuple(id_params)).fetchone()
    if row:
        return row

    external_query = ["SELECT * FROM transactions WHERE profile_id = ? AND external_id = ?"]
    external_params = [profile_id, ref]
    if direction is not None:
        external_query.append("AND direction = ?")
        external_params.append(direction)
    external_query.append("ORDER BY occurred_at DESC, created_at DESC, id DESC LIMIT 2")
    rows = conn.execute(" ".join(external_query), tuple(external_params)).fetchall()
    if len(rows) > 1:
        direction_hint = f" {direction}" if direction else ""
        raise AppError(
            f"Transaction external_id '{ref}' matches multiple{direction_hint} transactions",
            code="ambiguous_reference",
            hint="Use the Kassiber transaction id from `transactions list` or narrow the command to an unambiguous row.",
        )
    row = rows[0] if rows else None
    if not row:
        if direction is None:
            raise AppError(f"Transaction '{ref}' not found")
        raise AppError(f"{direction.capitalize()} transaction '{ref}' not found")
    return row


def resolve_tag(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM tags
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Tag '{ref}' not found")
    return row


def invalidate_journals(conn, profile_id):
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = NULL,
            last_processed_tx_count = 0,
            journal_input_version = journal_input_version + 1
        WHERE id = ?
        """,
        (profile_id,),
    )


def _row_int(row, key, default=0):
    try:
        if hasattr(row, "keys") and key not in row.keys():
            return default
        value = row[key]
    except (IndexError, KeyError):
        return default
    return int(value or default)


def _journals_current_for_profile(conn, profile):
    current_count = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    input_version = _row_int(profile, "journal_input_version")
    processed_version = _row_int(profile, "last_processed_input_version")
    return (
        int(current_count or 0),
        bool(
            profile["last_processed_at"]
            and int(current_count or 0) == _row_int(profile, "last_processed_tx_count")
            and input_version == processed_version
        ),
    )


TRANSFER_PAIR_KINDS = ("manual", "peg-in", "peg-out", "submarine-swap", "swap-refund")
TRANSFER_PAIR_POLICIES = ("carrying-value", "taxable")
DIRECT_SWAP_PAYOUT_KINDS = ("direct-swap-payout",)


_PAIR_SOURCE_VALUES = ("manual", "bulk_exact", "bulk_selected", "rule_auto")


def _pair_to_dict(row):
    keys = set(row.keys()) if hasattr(row, "keys") else set()
    swap_fee_msat = row["swap_fee_msat"] if "swap_fee_msat" in keys else None
    swap_fee_kind = row["swap_fee_kind"] if "swap_fee_kind" in keys else None
    confidence_at_pair = row["confidence_at_pair"] if "confidence_at_pair" in keys else None
    pair_source = row["pair_source"] if "pair_source" in keys else None
    deleted_at = row["deleted_at"] if "deleted_at" in keys else None
    out_amount = row["out_amount"] if "out_amount" in keys else None
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "in_transaction_id": row["in_transaction_id"],
        "kind": row["kind"],
        "policy": row["policy"],
        "notes": row["notes"],
        "swap_fee_msat": int(swap_fee_msat) if swap_fee_msat is not None else None,
        "swap_fee_kind": swap_fee_kind,
        "confidence_at_pair": confidence_at_pair,
        "pair_source": pair_source,
        "out_amount": int(out_amount) if out_amount is not None else None,
        "deleted_at": deleted_at,
        "created_at": row["created_at"],
    }


def _direct_payout_to_dict(row):
    keys = set(row.keys()) if hasattr(row, "keys") else set()
    payout_fiat_value = row["payout_fiat_value"] if "payout_fiat_value" in keys else None
    swap_fee_msat = row["swap_fee_msat"] if "swap_fee_msat" in keys else None
    out_amount = row["out_amount"] if "out_amount" in keys else None
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "kind": row["kind"],
        "policy": row["policy"],
        "payout_asset": row["payout_asset"],
        "payout_amount": float(msat_to_btc(row["payout_amount"])),
        "payout_amount_msat": int(row["payout_amount"]),
        "payout_occurred_at": row["payout_occurred_at"],
        "payout_fiat_value": float(payout_fiat_value) if payout_fiat_value is not None else None,
        "payout_external_id": row["payout_external_id"],
        "counterparty": row["counterparty"],
        "notes": row["notes"],
        "swap_fee_msat": int(swap_fee_msat) if swap_fee_msat is not None else None,
        "swap_fee_kind": row["swap_fee_kind"],
        "out_amount": int(out_amount) if out_amount is not None else None,
        "deleted_at": row["deleted_at"],
        "created_at": row["created_at"],
    }


def _positive_btc_amount_msat(value, flag_name):
    amount = dec(value)
    if amount <= 0:
        raise AppError(f"{flag_name} must be positive", code="validation")
    return btc_to_msat(amount)


def create_transaction_pair(
    conn,
    workspace_ref,
    profile_ref,
    out_ref,
    in_ref,
    kind="manual",
    policy="carrying-value",
    notes=None,
    *,
    pair_source="manual",
    confidence_at_pair=None,
    out_amount=None,
    commit=True,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if kind not in TRANSFER_PAIR_KINDS:
        raise AppError(
            f"Unsupported pair kind '{kind}'. Supported: {', '.join(TRANSFER_PAIR_KINDS)}",
            code="validation",
        )
    if policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported pair policy '{policy}'. Supported: {', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )
    if pair_source not in _PAIR_SOURCE_VALUES:
        raise AppError(
            f"Unsupported pair_source '{pair_source}'. Supported: {', '.join(_PAIR_SOURCE_VALUES)}",
            code="validation",
        )
    out_row = resolve_transaction(conn, profile["id"], out_ref, direction="outbound")
    in_row = resolve_transaction(conn, profile["id"], in_ref, direction="inbound")
    if out_row["id"] == in_row["id"]:
        raise AppError("--tx-out and --tx-in must reference different transactions", code="validation")
    if out_row["asset"] == in_row["asset"] and policy == "taxable":
        raise AppError(
            f"Same-asset taxable pairs are not supported yet "
            f"(asset={out_row['asset']}). Leave the legs unpaired to keep "
            f"normal SELL + BUY treatment, or use --policy carrying-value "
            f"for a self-transfer.",
            code="validation",
            hint="Re-run with --policy carrying-value, or omit the pair entirely to preserve taxable SELL + BUY behavior.",
        )
    if out_row["asset"] != in_row["asset"] and policy == "carrying-value":
        tax_country = str(profile["tax_country"] or "").strip().lower()
        if tax_country != "at":
            raise AppError(
                f"Cross-asset carrying-value pairs are only supported for Austrian profiles right now "
                f"(out={out_row['asset']}, in={in_row['asset']}). "
                f"Use --policy taxable for other tax countries.",
                code="validation",
                hint="Re-run with --policy taxable, or use an Austrian profile for cross-asset carrying-value swaps.",
            )
    out_amount_msat = None
    if out_amount is not None:
        if out_row["asset"] == in_row["asset"]:
            raise AppError(
                "--out-amount only applies to cross-asset swap pairs: it is the "
                "portion of the outbound that was swapped, with the remainder "
                "treated as a same-asset self-transfer.",
                code="validation",
            )
        out_amount_msat = _positive_btc_amount_msat(out_amount, "--out-amount")
        full_out_msat = int(out_row["amount"] or 0)
        if out_amount_msat > full_out_msat:
            raise AppError(
                f"--out-amount exceeds the outbound amount "
                f"({out_amount_msat} > {full_out_msat} msat).",
                code="validation",
            )
    existing = conn.execute(
        """
        SELECT id FROM transaction_pairs
        WHERE profile_id = ? AND deleted_at IS NULL
          AND (out_transaction_id IN (?, ?) OR in_transaction_id IN (?, ?))
        LIMIT 1
        """,
        (profile["id"], out_row["id"], in_row["id"], out_row["id"], in_row["id"]),
    ).fetchone()
    if existing:
        raise AppError(
            f"One of the transactions is already paired (pair id={existing['id']}). "
            f"Run `kassiber transfers unpair --pair-id {existing['id']}` first.",
            code="conflict",
        )
    existing_payout = conn.execute(
        """
        SELECT id FROM direct_swap_payouts
        WHERE profile_id = ? AND deleted_at IS NULL
          AND out_transaction_id IN (?, ?)
        LIMIT 1
        """,
        (profile["id"], out_row["id"], in_row["id"]),
    ).fetchone()
    if existing_payout:
        raise AppError(
            f"One of the transactions already has an active direct swap payout "
            f"(id={existing_payout['id']}). Delete that payout review before pairing.",
            code="conflict",
            hint="Run `kassiber transfers payouts delete --payout-id "
            f"{existing_payout['id']}` first.",
        )
    # On a split pair only the swapped portion (`out_amount`) crosses to the
    # other asset, so the persisted swap fee must be measured against that, not
    # the full outbound (the remainder is a same-asset self-transfer).
    swap_fee_out_msat = (
        out_amount_msat if out_amount_msat is not None else int(out_row["amount"] or 0)
    )
    swap_fee_msat, swap_fee_kind = core_transfer_matching.compute_swap_fee(
        swap_fee_out_msat,
        int(in_row["amount"] or 0),
    )
    pair_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, swap_fee_msat, swap_fee_kind, confidence_at_pair,
            pair_source, out_amount, deleted_at, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (
            pair_id,
            workspace["id"],
            profile["id"],
            out_row["id"],
            in_row["id"],
            kind,
            policy,
            notes,
            swap_fee_msat,
            swap_fee_kind,
            confidence_at_pair,
            pair_source,
            out_amount_msat,
            now_iso(),
        ),
    )
    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return _pair_to_dict(
        conn.execute("SELECT * FROM transaction_pairs WHERE id = ?", (pair_id,)).fetchone()
    )


def create_direct_swap_payout(
    conn,
    workspace_ref,
    profile_ref,
    out_ref,
    *,
    payout_asset,
    payout_amount,
    kind="direct-swap-payout",
    policy="carrying-value",
    payout_occurred_at=None,
    payout_fiat_value=None,
    payout_external_id=None,
    counterparty=None,
    notes=None,
    out_amount=None,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if kind not in DIRECT_SWAP_PAYOUT_KINDS:
        raise AppError(
            f"Unsupported direct payout kind '{kind}'. Supported: {', '.join(DIRECT_SWAP_PAYOUT_KINDS)}",
            code="validation",
        )
    if policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported direct payout policy '{policy}'. Supported: {', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )

    out_row = resolve_transaction(conn, profile["id"], out_ref, direction="outbound")
    target_asset = normalize_asset_code(payout_asset)
    if not target_asset:
        raise AppError("--payout-asset is required", code="validation")
    payout_amount_msat = _positive_btc_amount_msat(payout_amount, "--payout-amount")
    out_amount_msat = None
    if out_amount is not None:
        out_amount_msat = _positive_btc_amount_msat(out_amount, "--out-amount")
        full_out_msat = int(out_row["amount"] or 0)
        if out_amount_msat > full_out_msat:
            raise AppError(
                f"--out-amount exceeds the outbound amount "
                f"({out_amount_msat} > {full_out_msat} msat).",
                code="validation",
            )
    payout_value = dec(payout_fiat_value) if payout_fiat_value is not None else None
    if payout_value is not None and payout_value < 0:
        raise AppError("--payout-fiat-value must not be negative", code="validation")

    if out_row["asset"] != target_asset and policy == "carrying-value":
        tax_country = str(profile["tax_country"] or "").strip().lower()
        if tax_country != "at":
            raise AppError(
                "Cross-asset direct swap payouts with carrying value are only supported for Austrian profiles right now.",
                code="validation",
                hint="Re-run with --policy taxable, or use an Austrian profile for cross-asset carrying-value payouts.",
            )

    existing_pair = conn.execute(
        """
        SELECT id FROM transaction_pairs
        WHERE profile_id = ? AND deleted_at IS NULL
          AND (out_transaction_id = ? OR in_transaction_id = ?)
        LIMIT 1
        """,
        (profile["id"], out_row["id"], out_row["id"]),
    ).fetchone()
    if existing_pair:
        raise AppError(
            f"Transaction is already paired (pair id={existing_pair['id']}). "
            f"Run `kassiber transfers unpair --pair-id {existing_pair['id']}` first.",
            code="conflict",
        )
    existing_payout = conn.execute(
        """
        SELECT id FROM direct_swap_payouts
        WHERE profile_id = ? AND out_transaction_id = ? AND deleted_at IS NULL
        LIMIT 1
        """,
        (profile["id"], out_row["id"]),
    ).fetchone()
    if existing_payout:
        raise AppError(
            f"Transaction already has an active direct swap payout (id={existing_payout['id']}).",
            code="conflict",
            hint="Delete the existing payout review before creating a replacement.",
        )

    swap_fee_msat, swap_fee_kind = core_transfer_matching.compute_swap_fee(
        out_amount_msat if out_amount_msat is not None else int(out_row["amount"] or 0),
        payout_amount_msat,
    )
    payout_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO direct_swap_payouts(
            id, workspace_id, profile_id, out_transaction_id, kind, policy,
            payout_asset, payout_amount, payout_occurred_at, payout_fiat_value,
            payout_external_id, counterparty, notes, swap_fee_msat, swap_fee_kind,
            out_amount, deleted_at, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)
        """,
        (
            payout_id,
            workspace["id"],
            profile["id"],
            out_row["id"],
            kind,
            policy,
            target_asset,
            payout_amount_msat,
            payout_occurred_at,
            float(payout_value) if payout_value is not None else None,
            payout_external_id,
            counterparty,
            notes,
            swap_fee_msat,
            swap_fee_kind,
            out_amount_msat,
            now_iso(),
        ),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return _direct_payout_to_dict(
        conn.execute("SELECT * FROM direct_swap_payouts WHERE id = ?", (payout_id,)).fetchone()
    )


def list_direct_swap_payouts(conn, workspace_ref, profile_ref, *, include_deleted=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    extra_where = "" if include_deleted else "AND p.deleted_at IS NULL"
    rows = conn.execute(
        f"""
        SELECT
            p.*,
            tout.external_id AS out_external_id,
            tout.asset AS out_asset,
            COALESCE(p.out_amount, tout.amount) AS reviewed_out_amount_msat,
            tout.amount AS full_out_amount_msat,
            tout.occurred_at AS out_occurred_at,
            wout.label AS out_wallet
        FROM direct_swap_payouts p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        WHERE p.profile_id = ? {extra_where}
        ORDER BY p.created_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        entry = _direct_payout_to_dict(row)
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


def delete_direct_swap_payout(conn, workspace_ref, profile_ref, payout_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM direct_swap_payouts WHERE id = ? AND profile_id = ?",
        (payout_id, profile["id"]),
    ).fetchone()
    if not row:
        raise AppError("Direct swap payout not found", code="not_found")
    if row["deleted_at"]:
        return _direct_payout_to_dict(row)
    deleted_at = now_iso()
    conn.execute(
        "UPDATE direct_swap_payouts SET deleted_at = ? WHERE id = ?",
        (deleted_at, payout_id),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return _direct_payout_to_dict(
        conn.execute("SELECT * FROM direct_swap_payouts WHERE id = ?", (payout_id,)).fetchone()
    )


def list_transaction_pairs(conn, workspace_ref, profile_ref, *, include_deleted=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    extra_where = "" if include_deleted else "AND p.deleted_at IS NULL"
    rows = conn.execute(
        f"""
        SELECT
            p.*,
            tout.external_id AS out_external_id,
            tout.asset AS out_asset,
            -- On a split cross-asset pair only `out_amount` crossed to the other
            -- asset; swap_fee_msat was computed from that portion, so the pair's
            -- out amount must match it. Same-asset / whole pairs keep tout.amount.
            COALESCE(p.out_amount, tout.amount) AS out_amount_msat,
            tout.amount AS out_full_amount_msat,
            wout.label AS out_wallet,
            tin.external_id AS in_external_id,
            tin.asset AS in_asset,
            tin.amount AS in_amount_msat,
            win.label AS in_wallet
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE p.profile_id = ? {extra_where}
        ORDER BY p.created_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        entry = _pair_to_dict(row)
        entry["out"] = {
            "transaction_id": row["out_transaction_id"],
            "external_id": row["out_external_id"] or "",
            "wallet": row["out_wallet"],
            "asset": row["out_asset"],
            # `amount` is the swapped portion on a split pair; `full_amount`
            # carries the underlying transaction's total for transparency.
            "amount": float(msat_to_btc(row["out_amount_msat"])),
            "amount_msat": int(row["out_amount_msat"]),
            "full_amount": float(msat_to_btc(row["out_full_amount_msat"])),
            "full_amount_msat": int(row["out_full_amount_msat"]),
        }
        entry["in"] = {
            "transaction_id": row["in_transaction_id"],
            "external_id": row["in_external_id"] or "",
            "wallet": row["in_wallet"],
            "asset": row["in_asset"],
            "amount": float(msat_to_btc(row["in_amount_msat"])),
            "amount_msat": int(row["in_amount_msat"]),
        }
        output.append(entry)
    return output


def _candidate_to_dict(candidate):
    data = {
        "out_id": candidate.out_id,
        "in_id": candidate.in_id,
        "out_asset": candidate.out_asset,
        "in_asset": candidate.in_asset,
        "out_amount_msat": candidate.out_amount_msat,
        "out_amount": float(msat_to_btc(candidate.out_amount_msat)),
        "in_amount_msat": candidate.in_amount_msat,
        "in_amount": float(msat_to_btc(candidate.in_amount_msat)),
        "out_wallet_id": candidate.out_wallet_id,
        "in_wallet_id": candidate.in_wallet_id,
        "out_wallet_label": candidate.out_wallet_label,
        "in_wallet_label": candidate.in_wallet_label,
        "out_wallet_kind": candidate.out_wallet_kind,
        "in_wallet_kind": candidate.in_wallet_kind,
        "out_occurred_at": candidate.out_occurred_at,
        "in_occurred_at": candidate.in_occurred_at,
        "confidence": candidate.confidence,
        "method": candidate.method,
        "swap_fee_msat": candidate.swap_fee_msat,
        "swap_fee": float(msat_to_btc(candidate.swap_fee_msat)) if candidate.swap_fee_msat else 0.0,
        "swap_fee_kind": candidate.swap_fee_kind,
        "default_kind": candidate.default_kind,
        "default_policy": candidate.default_policy,
        "conflict_set_id": candidate.conflict_set_id,
        "conflict_size": candidate.conflict_size,
    }
    return data


def _load_transfer_rules(conn, profile_id):
    rows = conn.execute(
        "SELECT * FROM swap_matching_rules WHERE profile_id = ? ORDER BY created_at ASC, id ASC",
        (profile_id,),
    ).fetchall()
    return [core_swap_rules.load_rule(row) for row in rows]


def _candidate_key(candidate):
    return f"{candidate.out_id}->{candidate.in_id}"


def _candidate_dicts_with_rule_matches(candidates, rules, rule_matches):
    rules_by_id = {rule.id: rule for rule in rules}
    rule_by_key = {
        _candidate_key(match.candidate): {
            "rule_id": match.rule_id,
            "rule_name": match.rule_name,
            "kind": rules_by_id[match.rule_id].kind,
            "policy": rules_by_id[match.rule_id].policy,
        }
        for match in rule_matches
        if match.rule_id in rules_by_id
    }
    output = []
    for candidate in candidates:
        data = _candidate_to_dict(candidate)
        match = rule_by_key.get(_candidate_key(candidate))
        if match:
            data["rule_match"] = match
        output.append(data)
    return output


def _load_matcher_rows(conn, profile_id):
    """Fetch transaction rows enriched with wallet metadata for the matcher."""
    return conn.execute(
        """
        SELECT
            t.id, t.profile_id, t.wallet_id, t.external_id, t.payment_hash,
            t.swap_refund_funding_txid,
            t.occurred_at, t.direction, t.asset, t.amount, t.excluded,
            w.label AS wallet_label, w.kind AS wallet_kind
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ?
        """,
        (profile_id,),
    ).fetchall()


def _candidate_route_asset(asset, wallet_kind):
    asset_key = str(asset or "").upper()
    kind_key = str(wallet_kind or "").lower()
    if asset_key == "LBTC" or "liquid" in kind_key:
        return "LBTC"
    if asset_key == "BTC" and kind_key in core_transfer_matching.LIGHTNING_WALLET_KINDS:
        return "LNBTC"
    return asset_key


def _candidate_same_asset(candidate):
    return str(candidate.out_asset or "").upper() == str(candidate.in_asset or "").upper()


def _filter_transfer_candidates(
    candidates,
    *,
    confidence=None,
    asset_pair=None,
    route_pair=None,
    method=None,
    candidate_type=None,
):
    if candidate_type:
        if candidate_type not in ("transfer", "swap"):
            raise AppError(
                f"Invalid candidate_type '{candidate_type}', expected 'transfer' or 'swap'",
                code="validation",
            )
        same_asset = candidate_type == "transfer"
        candidates = [
            c
            for c in candidates
            if _candidate_same_asset(c) == same_asset
        ]
    if confidence:
        candidates = [c for c in candidates if c.confidence == confidence]
    if method:
        candidates = [c for c in candidates if c.method == method]
    if asset_pair:
        try:
            out_asset, in_asset = asset_pair.split("-", 1)
        except ValueError as exc:
            raise AppError(
                f"Invalid asset_pair '{asset_pair}', expected OUT-IN like 'LBTC-BTC'",
                code="validation",
            ) from exc
        candidates = [
            c for c in candidates if c.out_asset == out_asset and c.in_asset == in_asset
        ]
    if route_pair:
        try:
            out_route_asset, in_route_asset = route_pair.split("-", 1)
        except ValueError as exc:
            raise AppError(
                f"Invalid route_pair '{route_pair}', expected OUT-IN like 'LNBTC-BTC'",
                code="validation",
            ) from exc
        candidates = [
            c
            for c in candidates
            if _candidate_route_asset(c.out_asset, c.out_wallet_kind) == out_route_asset
            and _candidate_route_asset(c.in_asset, c.in_wallet_kind) == in_route_asset
        ]
    return candidates


def _load_active_transfer_review_refs(conn, profile_id):
    pair_records = conn.execute(
        "SELECT out_transaction_id, in_transaction_id, deleted_at FROM transaction_pairs WHERE profile_id = ?",
        (profile_id,),
    ).fetchall()
    payout_records = conn.execute(
        """
        SELECT out_transaction_id, NULL AS in_transaction_id, deleted_at
        FROM direct_swap_payouts
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    return [*pair_records, *payout_records]


def suggest_transfer_candidates(
    conn,
    workspace_ref,
    profile_ref,
    *,
    time_window_seconds=core_transfer_matching.DEFAULT_TIME_WINDOW_SECONDS,
    fee_pct_max=core_transfer_matching.DEFAULT_FEE_PCT_MAX,
    fee_sats_min=core_transfer_matching.DEFAULT_FEE_SATS_MIN,
    confidence=None,
    asset_pair=None,
    route_pair=None,
    method=None,
    candidate_type=None,
):
    """Run the matcher and return the candidate envelope.

    Honours optional filters used by the review queue: ``confidence``
    pins to exact / strong; ``asset_pair`` matches the legacy asset-only
    ``OUT-IN`` shape (e.g. ``"LBTC-BTC"``); ``route_pair`` matches the
    rail-aware route shape (e.g. ``"LNBTC-BTC"``); ``method`` pins to
    ``payment_hash`` or ``heuristic``.
    """
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = _load_matcher_rows(conn, profile["id"])
    pair_records = _load_active_transfer_review_refs(conn, profile["id"])
    dismissals = conn.execute(
        "SELECT out_transaction_id, in_transaction_id, expires_at FROM transaction_pair_dismissals WHERE profile_id = ?",
        (profile["id"],),
    ).fetchall()
    candidates = core_transfer_matching.suggest_swap_candidates(
        rows,
        pair_records=pair_records,
        dismissals=dismissals,
        time_window_seconds=int(time_window_seconds),
        fee_pct_max=float(fee_pct_max),
        fee_sats_min=int(fee_sats_min),
        tax_country=str(profile["tax_country"] or ""),
    )
    candidates = _filter_transfer_candidates(
        candidates,
        confidence=confidence,
        asset_pair=asset_pair,
        route_pair=route_pair,
        method=method,
        candidate_type=candidate_type,
    )
    rules = _load_transfer_rules(conn, profile["id"])
    rule_matches, _ = core_swap_rules.apply_rules(candidates, rules)
    counts = {
        "total": len(candidates),
        "exact": sum(1 for c in candidates if c.confidence == "exact"),
        "strong": sum(1 for c in candidates if c.confidence == "strong"),
        "conflicts": _count_conflict_clusters(candidates),
        "rule_matches": len(rule_matches),
    }
    return {
        "candidates": _candidate_dicts_with_rule_matches(candidates, rules, rule_matches),
        "counts": counts,
    }


def _count_conflict_clusters(candidates):
    """Count distinct conflict clusters among the given candidates.

    Uses the matcher-stamped ``conflict_size`` (computed over the full
    candidate set) so a filtered view still reports a cluster whose
    siblings are hidden by the active filters.
    """
    return len({c.conflict_set_id for c in candidates if c.conflict_size > 1})


def bulk_pair_transfers(
    conn,
    workspace_ref,
    profile_ref,
    *,
    confidence="exact",
    time_window_seconds=core_transfer_matching.DEFAULT_TIME_WINDOW_SECONDS,
    fee_pct_max=core_transfer_matching.DEFAULT_FEE_PCT_MAX,
    fee_sats_min=core_transfer_matching.DEFAULT_FEE_SATS_MIN,
    asset_pair=None,
    route_pair=None,
    method=None,
    candidate_type=None,
    commit=True,
):
    """Run the matcher and auto-pair every solo (non-conflicted) candidate
    whose confidence meets the threshold.

    Defaults to ``confidence="exact"`` so only payment-hash matches
    auto-apply without further user review. Conflict clusters are
    always skipped — disambiguation stays manual.
    """
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = _load_matcher_rows(conn, profile["id"])
    pair_records = _load_active_transfer_review_refs(conn, profile["id"])
    dismissals = conn.execute(
        "SELECT out_transaction_id, in_transaction_id, expires_at FROM transaction_pair_dismissals WHERE profile_id = ?",
        (profile["id"],),
    ).fetchall()
    candidates = core_transfer_matching.suggest_swap_candidates(
        rows,
        pair_records=pair_records,
        dismissals=dismissals,
        time_window_seconds=int(time_window_seconds),
        fee_pct_max=float(fee_pct_max),
        fee_sats_min=int(fee_sats_min),
        tax_country=str(profile["tax_country"] or ""),
    )
    if confidence not in ("exact", "strong"):
        raise AppError(
            f"Unsupported confidence '{confidence}'. Use 'exact' or 'strong'.",
            code="validation",
        )
    candidates = _filter_transfer_candidates(
        candidates,
        asset_pair=asset_pair,
        route_pair=route_pair,
        method=method,
        candidate_type=candidate_type,
    )
    applied = []
    pair_source = "bulk_exact" if confidence == "exact" else "bulk_selected"
    try:
        for candidate in candidates:
            # conflict_size is stamped over the unfiltered candidate set, so
            # a cluster split across filters (e.g. the swap vs transfer tabs)
            # still blocks bulk-pairing of every member.
            if candidate.conflict_size > 1:
                continue
            if confidence == "exact" and candidate.confidence != "exact":
                continue
            pair = create_transaction_pair(
                conn,
                workspace["id"],
                profile["id"],
                candidate.out_id,
                candidate.in_id,
                kind=candidate.default_kind,
                policy=candidate.default_policy,
                pair_source=pair_source,
                confidence_at_pair=candidate.confidence,
                commit=False,
            )
            applied.append(pair)
    except Exception:
        conn.rollback()
        raise
    if applied and commit:
        conn.commit()
    total_fee_msat = sum(int(pair.get("swap_fee_msat") or 0) for pair in applied)
    return {
        "applied": applied,
        "summary": {
            "count": len(applied),
            "skipped_conflicts": sum(1 for c in candidates if c.conflict_size > 1),
            "total_swap_fee_msat": total_fee_msat,
        },
    }


def apply_transfer_rules(
    conn,
    workspace_ref,
    profile_ref,
    *,
    time_window_seconds=core_transfer_matching.DEFAULT_TIME_WINDOW_SECONDS,
    fee_pct_max=core_transfer_matching.DEFAULT_FEE_PCT_MAX,
    fee_sats_min=core_transfer_matching.DEFAULT_FEE_SATS_MIN,
    confidence=None,
    asset_pair=None,
    route_pair=None,
    method=None,
    candidate_type=None,
    commit=True,
):
    """Auto-pair every non-conflicted candidate matched by enabled rules."""
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = _load_matcher_rows(conn, profile["id"])
    pair_records = _load_active_transfer_review_refs(conn, profile["id"])
    dismissals = conn.execute(
        "SELECT out_transaction_id, in_transaction_id, expires_at FROM transaction_pair_dismissals WHERE profile_id = ?",
        (profile["id"],),
    ).fetchall()
    candidates = core_transfer_matching.suggest_swap_candidates(
        rows,
        pair_records=pair_records,
        dismissals=dismissals,
        time_window_seconds=int(time_window_seconds),
        fee_pct_max=float(fee_pct_max),
        fee_sats_min=int(fee_sats_min),
        tax_country=str(profile["tax_country"] or ""),
    )
    candidates = _filter_transfer_candidates(
        candidates,
        confidence=confidence,
        asset_pair=asset_pair,
        route_pair=route_pair,
        method=method,
        candidate_type=candidate_type,
    )
    rules = _load_transfer_rules(conn, profile["id"])
    rules_by_id = {rule.id: rule for rule in rules}
    rule_matches, remaining = core_swap_rules.apply_rules(candidates, rules)
    applied = []
    try:
        for match in rule_matches:
            rule = rules_by_id[match.rule_id]
            pair = create_transaction_pair(
                conn,
                workspace["id"],
                profile["id"],
                match.candidate.out_id,
                match.candidate.in_id,
                kind=rule.kind,
                policy=rule.policy,
                pair_source="rule_auto",
                confidence_at_pair=match.candidate.confidence,
                commit=False,
            )
            applied.append(pair)
    except Exception:
        conn.rollback()
        raise
    if applied and commit:
        conn.commit()
    return {
        "applied": applied,
        "summary": {
            "count": len(applied),
            "remaining": len(remaining),
            "total_swap_fee_msat": sum(int(pair.get("swap_fee_msat") or 0) for pair in applied),
        },
    }


_DEFAULT_DISMISSAL_DAYS = 90


def dismiss_transfer_candidate(
    conn,
    workspace_ref,
    profile_ref,
    out_ref,
    in_ref,
    *,
    reason=None,
    expires_in_days=_DEFAULT_DISMISSAL_DAYS,
):
    """Record a "not a swap" dismissal so the matcher stops suggesting this
    exact pair.

    Defaults to a 90-day expiry — long enough that the user doesn't keep
    seeing the same rejected suggestion, short enough that updated
    evidence (e.g. a payment_hash later landing on one of the legs)
    eventually re-surfaces it.
    """
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    out_row = resolve_transaction(conn, profile["id"], out_ref)
    in_row = resolve_transaction(conn, profile["id"], in_ref)
    expires_at = None
    if expires_in_days and int(expires_in_days) > 0:
        from datetime import datetime, timedelta, timezone

        expires_at = (
            datetime.now(timezone.utc) + timedelta(days=int(expires_in_days))
        ).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    dismissal_id = str(uuid.uuid4())
    try:
        conn.execute(
            """
            INSERT INTO transaction_pair_dismissals(
                id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                reason, created_at, expires_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                dismissal_id,
                workspace["id"],
                profile["id"],
                out_row["id"],
                in_row["id"],
                reason,
                now_iso(),
                expires_at,
            ),
        )
    except sqlite3.IntegrityError:
        # Update the existing dismissal to refresh the expiry instead.
        conn.execute(
            """
            UPDATE transaction_pair_dismissals
            SET reason = COALESCE(?, reason), expires_at = ?
            WHERE profile_id = ? AND out_transaction_id = ? AND in_transaction_id = ?
            """,
            (reason, expires_at, profile["id"], out_row["id"], in_row["id"]),
        )
    conn.commit()
    row = conn.execute(
        "SELECT * FROM transaction_pair_dismissals "
        "WHERE profile_id = ? AND out_transaction_id = ? AND in_transaction_id = ?",
        (profile["id"], out_row["id"], in_row["id"]),
    ).fetchone()
    return _dismissal_to_dict(row)


def _dismissal_to_dict(row):
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "in_transaction_id": row["in_transaction_id"],
        "reason": row["reason"],
        "created_at": row["created_at"],
        "expires_at": row["expires_at"],
    }


# -- rules CRUD --------------------------------------------------------------


def list_transfer_rules(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        "SELECT * FROM swap_matching_rules WHERE profile_id = ? ORDER BY created_at DESC, id ASC",
        (profile["id"],),
    ).fetchall()
    return [_rule_row_to_dict(row) for row in rows]


def create_transfer_rule(
    conn,
    workspace_ref,
    profile_ref,
    *,
    name=None,
    predicate=None,
    kind="manual",
    policy="carrying-value",
    enabled=True,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if kind not in TRANSFER_PAIR_KINDS:
        raise AppError(
            f"Unsupported pair kind '{kind}'. Supported: {', '.join(TRANSFER_PAIR_KINDS)}",
            code="validation",
        )
    if policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported pair policy '{policy}'. Supported: {', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )
    predicate = predicate or {}
    if not isinstance(predicate, dict):
        raise AppError("predicate must be a JSON object", code="validation")
    rule_id = str(uuid.uuid4())
    timestamp = now_iso()
    conn.execute(
        """
        INSERT INTO swap_matching_rules(
            id, workspace_id, profile_id, name, predicate_json, kind, policy,
            enabled, created_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            rule_id,
            workspace["id"],
            profile["id"],
            name,
            json.dumps(predicate, sort_keys=True),
            kind,
            policy,
            1 if enabled else 0,
            timestamp,
            timestamp,
        ),
    )
    conn.commit()
    row = conn.execute("SELECT * FROM swap_matching_rules WHERE id = ?", (rule_id,)).fetchone()
    return _rule_row_to_dict(row)


def delete_transfer_rule(conn, workspace_ref, profile_ref, rule_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM swap_matching_rules WHERE id = ? AND profile_id = ?",
        (rule_id, profile["id"]),
    ).fetchone()
    if not row:
        raise AppError(f"Rule '{rule_id}' not found", code="not_found")
    conn.execute("DELETE FROM swap_matching_rules WHERE id = ?", (rule_id,))
    conn.commit()
    return {"deleted": rule_id}


def set_transfer_rule_enabled(conn, workspace_ref, profile_ref, rule_id, enabled):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM swap_matching_rules WHERE id = ? AND profile_id = ?",
        (rule_id, profile["id"]),
    ).fetchone()
    if not row:
        raise AppError(f"Rule '{rule_id}' not found", code="not_found")
    conn.execute(
        "UPDATE swap_matching_rules SET enabled = ?, updated_at = ? WHERE id = ?",
        (1 if enabled else 0, now_iso(), rule_id),
    )
    conn.commit()
    updated = conn.execute("SELECT * FROM swap_matching_rules WHERE id = ?", (rule_id,)).fetchone()
    return _rule_row_to_dict(updated)


def _rule_row_to_dict(row):
    predicate = {}
    try:
        predicate = json.loads(row["predicate_json"] or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        predicate = {}
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "name": row["name"],
        "predicate": predicate,
        "kind": row["kind"],
        "policy": row["policy"],
        "enabled": bool(row["enabled"]),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


# -- saved views CRUD --------------------------------------------------------


def list_saved_views_cli(conn, workspace_ref, profile_ref, *, surface=None):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_saved_views.list_views(conn, profile["id"], surface=surface)


def create_saved_view_cli(
    conn, workspace_ref, profile_ref, *, surface, name, filter_payload=None
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_saved_views.create_view(
        conn,
        workspace["id"],
        profile["id"],
        surface=surface,
        name=name,
        filter_payload=filter_payload,
    )


def delete_saved_view_cli(conn, workspace_ref, profile_ref, view_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_saved_views.delete_view(conn, profile["id"], view_id)


def list_chat_sessions_cli(conn, workspace_ref, profile_ref, *, limit=50):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return {
        "sessions": core_chat_history.list_sessions(conn, profile["id"], limit=limit),
        "history_mode": core_chat_history.history_mode(conn),
    }


def show_chat_session_cli(conn, workspace_ref, profile_ref, session_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_chat_history.get_session(conn, profile["id"], session_id)


def delete_chat_session_cli(conn, workspace_ref, profile_ref, session_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_chat_history.delete_session(conn, profile["id"], session_id)


def clear_chat_sessions_cli(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_chat_history.clear_sessions(conn, profile["id"])


def chat_history_config_cli(conn, *, history=None, database_encrypted):
    if history is not None:
        core_chat_history.set_history_mode(conn, history)
    return {
        "history": core_chat_history.history_mode(conn),
        "history_enabled": core_chat_history.history_enabled(
            conn, database_encrypted=database_encrypted
        ),
        "database_encrypted": database_encrypted,
    }


def delete_transaction_pair(conn, workspace_ref, profile_ref, pair_id):
    """Soft-delete a transaction pair.

    Sets ``deleted_at`` so the row stays around for audit and the
    partial-unique indexes from commit 1 stop covering it — meaning the
    user can immediately re-pair the same legs without first hard-deleting
    the historic record. Already soft-deleted pairs are a no-op.
    """
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM transaction_pairs WHERE id = ? AND profile_id = ?",
        (pair_id, profile["id"]),
    ).fetchone()
    if not row:
        raise AppError(f"Pair '{pair_id}' not found", code="not_found")
    if row["deleted_at"] is None:
        conn.execute(
            "UPDATE transaction_pairs SET deleted_at = ? WHERE id = ?",
            (now_iso(), pair_id),
        )
        invalidate_journals(conn, profile["id"])
        conn.commit()
    return {"deleted": pair_id}


def init_app(conn):
    set_setting(conn, "app_version", __version__)
    conn.commit()


def read_text_argument(value, file_path, label):
    if value not in (None, ""):
        return str(value).strip()
    if not file_path:
        return None
    text = Path(file_path).expanduser().read_text(encoding="utf-8").strip()
    if not text:
        raise AppError(f"{label} file '{file_path}' is empty")
    return text


def wallet_live_chain_config(config):
    if not any(
        [
            config.get("descriptor"),
            config.get("change_descriptor"),
            config.get("addresses"),
            config.get("chain"),
            config.get("network"),
        ]
    ):
        return None, None
    chain = normalize_chain_value(config.get("chain"))
    network = normalize_network_value(chain, config.get("network"))
    return chain, network


def load_wallet_descriptor_plan_from_config(config):
    try:
        return load_descriptor_plan(config)
    except ValueError as exc:
        raise AppError(str(exc)) from exc


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    from ..secrets.cli_input import enforce_single_stdin_consumer, read_secret_from_args

    enforce_single_stdin_consumer(args, ("descriptor", "change_descriptor"))
    descriptor_text = read_secret_from_args(args, "descriptor")
    if descriptor_text is None:
        descriptor_text = read_text_argument(
            None,
            getattr(args, "descriptor_file", None),
            "Descriptor",
        )
    if descriptor_text:
        config["descriptor"] = descriptor_text
    change_descriptor_text = read_secret_from_args(
        args, "change-descriptor", legacy_attr="change_descriptor"
    )
    if change_descriptor_text is None:
        change_descriptor_text = read_text_argument(
            None,
            getattr(args, "change_descriptor_file", None),
            "Change descriptor",
        )
    if change_descriptor_text:
        config["change_descriptor"] = change_descriptor_text
    addresses = normalize_addresses(getattr(args, "address", None))
    existing_addresses = normalize_addresses(config.get("addresses"))
    if addresses or existing_addresses:
        config["addresses"] = normalize_addresses(existing_addresses + addresses)
    if getattr(args, "chain", None):
        config["chain"] = normalize_chain_value(args.chain)
    if getattr(args, "network", None):
        chain = normalize_chain_value(config.get("chain"))
        config["network"] = normalize_network_value(chain, args.network)
    if getattr(args, "gap_limit", None) is not None:
        if args.gap_limit <= 0:
            raise AppError("Descriptor gap limit must be positive")
        if args.gap_limit > MAX_DESCRIPTOR_GAP_LIMIT:
            raise AppError(
                f"Descriptor gap limit must be {MAX_DESCRIPTOR_GAP_LIMIT} or lower"
            )
        config["gap_limit"] = args.gap_limit
    if getattr(args, "policy_asset", None):
        config["policy_asset"] = normalize_asset_code(args.policy_asset)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network
    return config


WALLET_KIND_CATALOG = {
    "descriptor": {
        "summary": "Output-descriptor wallet with optional change branch; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "change_descriptor", "gap_limit", "backend", "chain", "network", "policy_asset"],
        "requires": ["descriptor"],
    },
    "xpub": {
        "summary": "Extended-public-key wallet derived to address set; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "gap_limit", "backend", "chain", "network"],
        "requires": ["descriptor"],
    },
    "address": {
        "summary": "Bare-address list wallet; useful for receive-only tracking or imports.",
        "config_fields": ["addresses", "backend", "chain", "network", "source_file", "source_format"],
        "requires": ["addresses|source_file"],
    },
    "coreln": {
        "summary": "Core Lightning node wallet; read-only live sync through a coreln backend.",
        "config_fields": ["backend"],
        "requires": ["backend"],
    },
    "lnd": {
        "summary": "LND CSV-derived wallet (deposits/withdrawals from node exports).",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "nwc": {
        "summary": "Nostr Wallet Connect wallet fed by CSV exports.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "phoenix": {
        "summary": "Phoenix Wallet CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "river": {
        "summary": "River Bitcoin Activity or Account Activity CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "bullbitcoin": {
        "summary": "Bull Bitcoin order evidence and unified wallet CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "coinfinity": {
        "summary": "Coinfinity order CSV importer for exact buy/sell execution pricing.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "21bitcoin": {
        "summary": "21bitcoin custodial platform CSV importer with exact trade pricing.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "pocketbitcoin": {
        "summary": "Pocket Bitcoin account CSV importer for exact buy/sell execution pricing.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "strike": {
        "summary": "Strike custodial platform CSV importer for exchange, Bitcoin, and Lightning rows.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "wasabi": {
        "summary": "Wasabi Wallet sanitized RPC/export bundle importer with CoinJoin and anonymity evidence.",
        "config_fields": ["source_file", "source_format", "wasabi_metadata"],
        "requires": [],
    },
    "custom": {
        "summary": "Custom CSV/JSON source; use with --config/--config-file to describe field mapping.",
        "config_fields": ["source_file", "source_format", "config"],
        "requires": ["source_file"],
    },
}


DEFAULT_BULLBITCOIN_WALLET_LABEL = "Bull Bitcoin"
DEFAULT_COINFINITY_WALLET_LABEL = "Coinfinity"
DEFAULT_TWENTYONEBITCOIN_WALLET_LABEL = "21bitcoin"
DEFAULT_POCKETBITCOIN_WALLET_LABEL = "Pocket Bitcoin"
DEFAULT_STRIKE_WALLET_LABEL = "Strike"


def _get_or_create_provider_import_wallet(conn, profile, input_format, wallet_ref=None):
    if wallet_ref:
        return resolve_wallet(conn, profile["id"], wallet_ref)
    if input_format == "21bitcoin_csv":
        default_label = DEFAULT_TWENTYONEBITCOIN_WALLET_LABEL
        wallet_kind = "21bitcoin"
    elif input_format == "coinfinity_csv":
        default_label = DEFAULT_COINFINITY_WALLET_LABEL
        wallet_kind = "coinfinity"
    elif input_format == "pocketbitcoin_csv":
        default_label = DEFAULT_POCKETBITCOIN_WALLET_LABEL
        wallet_kind = "pocketbitcoin"
    elif input_format == "strike_csv":
        default_label = DEFAULT_STRIKE_WALLET_LABEL
        wallet_kind = "strike"
    else:
        default_label = DEFAULT_BULLBITCOIN_WALLET_LABEL
        wallet_kind = "bullbitcoin"
    existing = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND lower(w.label) = lower(?)
        LIMIT 1
        """,
        (profile["id"], default_label),
    ).fetchone()
    if existing:
        return existing
    created = core_wallets.create_wallet(
        conn,
        profile["workspace_id"],
        profile["id"],
        default_label,
        wallet_kind,
        config={"source_format": input_format},
    )
    return resolve_wallet(conn, profile["id"], created["id"])


def import_into_wallet(
    conn,
    workspace_ref,
    profile_ref,
    wallet_ref,
    file_path,
    input_format,
    import_mode=None,
):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if input_format in {"21bitcoin_csv", "strike_csv"}:
        default_mode = core_imports.BULLBITCOIN_IMPORT_MODE_FULL if input_format == "strike_csv" else None
        mode = core_imports.normalize_bullbitcoin_import_mode(import_mode or default_mode)
        if input_format == "21bitcoin_csv" and mode == core_imports.BULLBITCOIN_IMPORT_MODE_RELEVANT:
            if wallet_ref:
                resolve_wallet(conn, profile["id"], wallet_ref)
            return _import_file_for_profile(
                conn,
                profile,
                file_path,
                input_format,
                import_mode=mode,
            )
        wallet = _get_or_create_provider_import_wallet(conn, profile, input_format, wallet_ref)
        outcome = _import_file_for_sync(conn, profile, wallet, file_path, input_format)
        outcome["mode"] = mode
        return outcome
    if core_imports.is_exchange_evidence_format(input_format):
        mode = core_imports.normalize_bullbitcoin_import_mode(import_mode)
        if mode == core_imports.BULLBITCOIN_IMPORT_MODE_FULL:
            wallet = _get_or_create_provider_import_wallet(conn, profile, input_format, wallet_ref)
            return _import_file_for_profile(
                conn,
                profile,
                file_path,
                input_format,
                import_mode=mode,
                wallet=wallet,
            )
        if wallet_ref:
            resolve_wallet(conn, profile["id"], wallet_ref)
        return _import_file_for_profile(
            conn,
            profile,
            file_path,
            input_format,
            import_mode=mode,
        )
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    return _import_file_for_sync(conn, profile, wallet, file_path, input_format)


def import_into_profile(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    input_format,
    import_mode=None,
    wallet_ref=None,
):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    mode = core_imports.normalize_bullbitcoin_import_mode(import_mode)
    if input_format == "21bitcoin_csv" and mode == core_imports.BULLBITCOIN_IMPORT_MODE_FULL:
        wallet = _get_or_create_provider_import_wallet(conn, profile, input_format, wallet_ref)
        outcome = _import_file_for_sync(conn, profile, wallet, file_path, input_format)
        outcome["mode"] = mode
        return outcome
    wallet = None
    if core_imports.is_exchange_evidence_format(input_format) and mode == core_imports.BULLBITCOIN_IMPORT_MODE_FULL:
        wallet = _get_or_create_provider_import_wallet(conn, profile, input_format, wallet_ref)
    return _import_file_for_profile(
        conn,
        profile,
        file_path,
        input_format,
        import_mode=mode,
        wallet=wallet,
    )


@lru_cache(maxsize=1)
def _metadata_hooks():
    return core_metadata.MetadataHooks(
        resolve_scope=resolve_scope,
        resolve_wallet=resolve_wallet,
        resolve_tag=resolve_tag,
        resolve_transaction=resolve_transaction,
        normalize_code=normalize_code,
        now_iso=now_iso,
        invalidate_journals=invalidate_journals,
        parse_iso_datetime=_parse_iso_datetime,
        iso_z=_iso_z,
        encode_cursor=_encode_event_cursor,
        decode_cursor=_decode_event_cursor,
    )


@lru_cache(maxsize=1)
def _attachment_hooks():
    return core_attachments.AttachmentHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        now_iso=now_iso,
    )


@lru_cache(maxsize=1)
def _commercial_hooks():
    return core_commercial.CommercialHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        invalidate_journals=invalidate_journals,
    )


@lru_cache(maxsize=1)
def _report_hooks():
    return core_reports.ReportHooks(
        resolve_scope=resolve_scope,
        resolve_account=resolve_account,
        resolve_wallet=resolve_wallet,
        require_processed_journals=require_processed_journals,
        build_ledger_state=build_ledger_state,
        list_journal_entries=list_journal_entries,
        list_wallets=core_wallets.list_wallets,
        parse_iso_datetime=_parse_iso_datetime,
        iso_z=_iso_z,
        now_iso=now_iso,
        format_table=format_table,
        write_text_pdf=write_text_pdf,
    )


@lru_cache(maxsize=1)
def _import_coordinator_hooks():
    return core_imports.ImportCoordinatorHooks(
        ensure_tag_row=lambda conn, workspace_id, profile_id, code, label: core_metadata.ensure_tag_row(
            conn,
            workspace_id,
            profile_id,
            code,
            label,
            _metadata_hooks(),
        ),
        invalidate_journals=invalidate_journals,
    )


def _import_file_for_sync(conn, profile, wallet, file_path, input_format, *, commit=True):
    if input_format in {"21bitcoin_csv", "strike_csv"}:
        return core_imports.import_file_into_wallet(
            conn,
            profile,
            wallet,
            file_path,
            input_format,
            _import_coordinator_hooks(),
            commit=commit,
        )
    if core_imports.is_exchange_evidence_format(input_format):
        config = json.loads(wallet["config_json"] or "{}")
        mode = config.get("import_mode") or core_imports.BULLBITCOIN_IMPORT_MODE_RELEVANT
        return _import_file_for_profile(
            conn,
            profile,
            file_path,
            input_format,
            import_mode=mode,
            wallet=wallet if mode == core_imports.BULLBITCOIN_IMPORT_MODE_FULL else None,
            commit=commit,
        )
    return core_imports.import_file_into_wallet(
        conn,
        profile,
        wallet,
        file_path,
        input_format,
        _import_coordinator_hooks(),
        commit=commit,
    )


def _import_file_for_profile(
    conn,
    profile,
    file_path,
    input_format,
    *,
    import_mode=None,
    wallet=None,
    commit=True,
):
    return core_imports.import_file_into_profile(
        conn,
        profile,
        file_path,
        input_format,
        _import_coordinator_hooks(),
        import_mode=import_mode or core_imports.BULLBITCOIN_IMPORT_MODE_RELEVANT,
        wallet=wallet,
        commit=commit,
    )


def _import_records_for_sync(
    conn,
    profile,
    wallet,
    records,
    source_label,
    *,
    apply_btcpay=False,
    apply_phoenix=False,
    commit=True,
):
    return core_imports.import_records_into_wallet(
        conn,
        profile,
        wallet,
        records,
        source_label,
        _import_coordinator_hooks(),
        apply_btcpay=apply_btcpay,
        apply_phoenix=apply_phoenix,
        commit=commit,
    )


def _insert_records_for_sync(conn, profile, wallet, records, source_label, *, commit=True):
    return _import_records_for_sync(
        conn,
        profile,
        wallet,
        records,
        source_label,
        commit=commit,
    )


def _wallet_sync_hooks(commit=True):
    return core_sync.WalletSyncHooks(
        import_file=lambda conn, profile, wallet, file_path, input_format: _import_file_for_sync(
            conn,
            profile,
            wallet,
            file_path,
            input_format,
            commit=commit,
        ),
        insert_records=lambda conn, profile, wallet, records, source_label: _insert_records_for_sync(
            conn,
            profile,
            wallet,
            records,
            source_label,
            commit=commit,
        ),
        resolve_backend=resolve_backend,
        resolve_sync_state=core_sync_backends.resolve_wallet_sync_targets,
        normalize_addresses=core_wallets.normalize_addresses,
        backend_adapters=core_sync_backends.SYNC_BACKEND_ADAPTERS,
        update_output_inventory=lambda conn, profile, wallet, backend, sync_state, outputs: core_output_inventory.update_wallet_output_inventory(
            conn,
            profile,
            wallet,
            backend,
            sync_state,
            outputs,
            commit=commit,
        ),
        sync_btcpay_wallet=lambda conn, runtime_config, profile, wallet: sync_configured_btcpay_wallet(
            conn,
            runtime_config,
            profile,
            wallet,
            commit=commit,
        ),
        enrich_btcpay_wallet=lambda conn, runtime_config, profile, wallet: enrich_wallet_from_btcpay_provenance(
            conn,
            runtime_config,
            profile,
            wallet,
            commit=commit,
        ),
        enrich_bullbitcoin_wallet=lambda conn, runtime_config, profile, wallet: enrich_wallet_from_bullbitcoin_wallet_exports(
            conn,
            runtime_config,
            profile,
            wallet,
            commit=commit,
        ),
        sync_core_lightning_wallet=lambda conn, runtime_config, profile, wallet: core_lightning_cln.sync_core_lightning_wallet(
            conn,
            profile,
            wallet,
            resolve_backend(
                runtime_config,
                json.loads(wallet["config_json"] or "{}").get("backend"),
            ),
            _import_coordinator_hooks(),
            commit=commit,
        ),
    )


def _mark_wallet_synced(conn, wallet, synced_at=None):
    timestamp = synced_at or now_iso()
    config = json.loads(wallet["config_json"] or "{}")
    config["last_synced_at"] = timestamp
    conn.execute(
        "UPDATE wallets SET config_json = ? WHERE id = ?",
        (json.dumps(config, sort_keys=True), wallet["id"]),
    )
    return timestamp


def _mark_wallet_synced_from_results(conn, wallet, results):
    if any(result.get("status") == "synced" for result in results):
        _mark_wallet_synced(conn, wallet)


def sync_wallet_from_backend(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    wallet,
    *,
    checkpoint=None,
    force_full=False,
):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_sync.sync_wallet_from_backend(
        conn,
        runtime_config,
        profile,
        wallet,
        _wallet_sync_hooks(),
        checkpoint=checkpoint,
        force_full=force_full,
    )


def sync_wallet(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    wallet_ref=None,
    sync_all=False,
    *,
    freshness_checkpoints=None,
    force_full=False,
):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if sync_all and wallet_ref:
        raise AppError("--wallet and --all are mutually exclusive", code="validation")
    if sync_all:
        wallets = conn.execute("SELECT * FROM wallets WHERE profile_id = ? ORDER BY label ASC", (profile["id"],)).fetchall()
        hooks = _wallet_sync_hooks(commit=False)
        # Run the network-only fetch for backend-synced wallets concurrently
        # before the serial write loop. DB writes and the per-wallet savepoint
        # isolation below are unchanged — only the fetch is parallelized, and the
        # per-host limiter keeps total concurrency against any one host within a
        # single wallet's existing budget. AppErrors are captured per wallet and
        # re-raised under that wallet's savepoint, preserving rollback isolation.
        backend_wallets = [
            wallet
            for wallet in wallets
            if core_sync.classify_wallet_sync(wallet, hooks.normalize_addresses) == "backend"
        ]
        prefetched = core_sync.prefetch_wallets_backend(
            runtime_config,
            profile,
            backend_wallets,
            hooks,
            checkpoints=freshness_checkpoints,
            force_full=force_full,
        )
        results = []
        for idx, wallet in enumerate(wallets):
            savepoint = f"wallet_sync_{idx}"
            conn.execute(f"SAVEPOINT {savepoint}")
            try:
                wallet_results = core_sync.sync_wallets(
                    conn,
                    runtime_config,
                    profile,
                    [wallet],
                    hooks,
                    checkpoints=freshness_checkpoints,
                    force_full=force_full,
                    prefetched=prefetched,
                )
                _mark_wallet_synced_from_results(conn, wallet, wallet_results)
                results.extend(wallet_results)
            except AppError as exc:
                conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
                conn.execute(f"RELEASE SAVEPOINT {savepoint}")
                results.append(
                    {
                        "wallet": wallet["label"],
                        "status": "error",
                        "code": exc.code,
                        "message": str(exc),
                        "hint": exc.hint or "",
                        "details": exc.details,
                        "retryable": bool(exc.retryable),
                    }
                )
            else:
                conn.execute(f"RELEASE SAVEPOINT {savepoint}")
                conn.commit()
        return results
    else:
        if not wallet_ref:
            raise AppError("Provide --wallet or use --all")
        wallets = [resolve_wallet(conn, profile["id"], wallet_ref)]
        results = core_sync.sync_wallets(
            conn,
            runtime_config,
            profile,
            wallets,
            _wallet_sync_hooks(),
            checkpoints=freshness_checkpoints,
            force_full=force_full,
        )
        _mark_wallet_synced_from_results(conn, wallets[0], results)
        conn.commit()
        return results


def _sync_btcpay_wallet(
    conn,
    runtime_config,
    profile,
    wallet,
    *,
    page_size=BTCPAY_DEFAULT_PAGE_SIZE,
    commit=True,
):
    config = json.loads(wallet["config_json"] or "{}")
    checkpoint = {}
    try:
        checkpoint = dict(wallet["_freshness_checkpoint"] or {})
    except (KeyError, IndexError, TypeError):
        checkpoint = {}
    btcpay_config = core_wallets.wallet_btcpay_sync_config(config)
    if btcpay_config is None:
        raise AppError(
            f"Wallet '{wallet['label']}' does not have BTCPay sync configured",
            code="validation",
            hint="Run `kassiber wallets sync-btcpay --wallet ... --backend ... --store-id ...` first, or store the config with `wallets update`.",
        )
    backend = resolve_backend(runtime_config, btcpay_config["backend"])
    kind = core_sync.normalize_backend_kind(backend["kind"])
    if kind != "btcpay":
        raise AppError(
            f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
            code="validation",
            hint="Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token-stdin` or `--token-fd FD`.",
        )
    btcpay_meta = {}
    records = fetch_btcpay_records(
        backend,
        store_id=btcpay_config["store_id"],
        payment_method_id=btcpay_config["payment_method_id"],
        page_size=page_size,
        checkpoint=checkpoint,
        metadata=btcpay_meta,
    )
    outcome = _import_records_for_sync(
        conn,
        profile,
        wallet,
        records,
        f"btcpay:{backend['name']}:{btcpay_config['store_id']}",
        apply_btcpay=True,
        commit=commit,
    )
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = kind
    outcome["backend_url"] = redact_backend_url(backend["url"])
    outcome["store_id"] = btcpay_config["store_id"]
    outcome["payment_method_id"] = btcpay_config["payment_method_id"]
    outcome["page_size"] = page_size
    outcome["fetched"] = len(records)
    if btcpay_meta:
        checkpoint.update(
            {
                "backend": {"name": backend["name"], "kind": kind},
                "btcpay_pages": btcpay_meta.get("btcpay_pages", {}),
                "btcpay_pagination": btcpay_meta.get("btcpay_pagination", {}),
                "store_id": btcpay_config["store_id"],
                "payment_method_id": btcpay_config["payment_method_id"],
            }
        )
        outcome["freshness_checkpoint"] = checkpoint
        outcome["pages_fetched"] = btcpay_meta.get("pages_fetched", 0)
        outcome["stopped_by_known_page"] = bool(btcpay_meta.get("stopped_by_known_page"))
        outcome["stop_reason"] = btcpay_meta.get("stop_reason")
        outcome["deep_audit"] = btcpay_meta.get("deep_audit")
    return outcome


def sync_configured_btcpay_wallet(conn, runtime_config, profile, wallet, *, commit=True):
    return _sync_btcpay_wallet(
        conn,
        runtime_config,
        profile,
        wallet,
        page_size=BTCPAY_DEFAULT_PAGE_SIZE,
        commit=commit,
    )


def enrich_wallet_from_btcpay_provenance(
    conn,
    runtime_config,
    profile,
    wallet,
    *,
    page_size=BTCPAY_DEFAULT_PAGE_SIZE,
    commit=True,
):
    config = json.loads(wallet["config_json"] or "{}")
    checkpoint = {}
    try:
        checkpoint = dict(wallet["_freshness_checkpoint"] or {})
    except (KeyError, IndexError, TypeError):
        checkpoint = {}
    route_checkpoints = checkpoint.get("routes") if isinstance(checkpoint.get("routes"), dict) else {}
    next_route_checkpoints = {}
    routes = core_wallets.wallet_btcpay_provenance_config(config)
    totals = {
        "routes": 0,
        "fetched": 0,
        "btcpay_notes_set": 0,
        "btcpay_tags_added": 0,
        "btcpay_tags_created": 0,
    }
    route_results = []
    for route in routes:
        backend = resolve_backend(runtime_config, route["backend"])
        kind = core_sync.normalize_backend_kind(backend["kind"])
        if kind != "btcpay":
            raise AppError(
                f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
                code="validation",
                hint="Use a BTCPay backend for BTCPay provenance enrichment.",
            )
        route_key = f"{backend['name']}:{route['store_id']}:{route['payment_method_id']}"
        btcpay_meta = {}
        records = fetch_btcpay_records(
            backend,
            store_id=route["store_id"],
            payment_method_id=route["payment_method_id"],
            page_size=page_size,
            checkpoint=route_checkpoints.get(route_key, {}),
            metadata=btcpay_meta,
        )
        metadata = core_imports.apply_btcpay_metadata(
            conn,
            profile,
            wallet,
            records,
            _import_coordinator_hooks(),
            commit=False,
        )
        route_result = {
            "backend": backend["name"],
            "backend_kind": kind,
            "backend_url": redact_backend_url(backend["url"]),
            "store_id": route["store_id"],
            "payment_method_id": route["payment_method_id"],
            "fetched": len(records),
            "pages_fetched": btcpay_meta.get("pages_fetched", 0),
            "stopped_by_known_page": bool(btcpay_meta.get("stopped_by_known_page")),
            "stop_reason": btcpay_meta.get("stop_reason"),
            "deep_audit": btcpay_meta.get("deep_audit"),
            **metadata,
        }
        next_route_checkpoints[route_key] = {
            "backend": {"name": backend["name"], "kind": kind},
            "btcpay_pages": btcpay_meta.get("btcpay_pages", {}),
            "btcpay_pagination": btcpay_meta.get("btcpay_pagination", {}),
            "store_id": route["store_id"],
            "payment_method_id": route["payment_method_id"],
        }
        route_results.append(route_result)
        totals["routes"] += 1
        totals["fetched"] += len(records)
        totals["btcpay_notes_set"] += metadata["btcpay_notes_set"]
        totals["btcpay_tags_added"] += metadata["btcpay_tags_added"]
        totals["btcpay_tags_created"] += metadata["btcpay_tags_created"]
    if commit:
        conn.commit()
    checkpoint.update({"routes": dict(sorted(next_route_checkpoints.items()))})
    return {
        **totals,
        "route_results": route_results,
        "freshness_checkpoint": checkpoint,
    }


def enrich_wallet_from_bullbitcoin_wallet_exports(
    conn,
    runtime_config,
    profile,
    wallet,
    *,
    commit=True,
):
    del runtime_config
    config = json.loads(wallet["config_json"] or "{}")
    routes = core_wallets.wallet_bullbitcoin_wallet_export_config(config)
    totals = {
        "routes": 0,
        "rows": 0,
        "rows_total": 0,
        "updated": 0,
        "unchanged": 0,
        "skipped": 0,
    }
    route_results = []
    for route in routes:
        all_records = load_import_records(
            route["source_file"],
            "bullbitcoin_wallet_csv",
        )
        records = core_imports.filter_bullbitcoin_wallet_records(
            all_records,
            route["network"],
        )
        outcome = core_imports.import_records_into_wallet(
            conn,
            profile,
            wallet,
            records,
            f"bullbitcoin-wallet:{route['network']}",
            _import_coordinator_hooks(),
            match_existing_only=True,
            report_updates=True,
            commit=False,
        )
        updated = len(outcome.get("updated_records") or [])
        route_result = {
            "source_file": route["source_file"],
            "network": route["network"],
            "rows": len(records),
            "rows_total": len(all_records),
            "updated": updated,
            "unchanged": int(outcome.get("unchanged") or 0),
            "skipped": int(outcome.get("skipped") or 0),
            "journal_invalidated": bool(outcome.get("journal_invalidated")),
        }
        route_results.append(route_result)
        totals["routes"] += 1
        totals["rows"] += route_result["rows"]
        totals["rows_total"] += route_result["rows_total"]
        totals["updated"] += route_result["updated"]
        totals["unchanged"] += route_result["unchanged"]
        totals["skipped"] += route_result["skipped"]
    if commit:
        conn.commit()
    return {**totals, "route_results": route_results}


def sync_btcpay_into_wallet(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    wallet_ref,
    backend_name,
    store_id,
    payment_method_id,
    page_size,
):
    store_id = core_wallets.normalize_btcpay_store_id(store_id)
    payment_method_id = core_wallets.normalize_btcpay_payment_method_id(
        payment_method_id
    )
    require_wallet_history_payment_method(payment_method_id)
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    backend = resolve_backend(runtime_config, backend_name)
    kind = core_sync.normalize_backend_kind(backend["kind"])
    if kind != "btcpay":
        raise AppError(
            f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
            code="validation",
            hint="Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token-stdin` or `--token-fd FD`.",
        )
    core_wallets.update_wallet(
        conn,
        workspace_ref,
        profile_ref,
        wallet_ref,
        {
            "config": {
                "backend": backend_name,
                "store_id": store_id,
                "payment_method_id": payment_method_id,
                "sync_source": core_wallets.BTCPAY_SYNC_SOURCE,
            },
            "clear": [],
        },
    )
    wallet = resolve_wallet(conn, profile["id"], wallet["id"])
    outcome = _sync_btcpay_wallet(
        conn,
        runtime_config,
        profile,
        wallet,
        page_size=page_size,
    )
    _mark_wallet_synced(conn, wallet)
    conn.commit()
    return outcome


def attach_bullbitcoin_wallet_export_to_wallet(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    wallet_ref,
    source_file,
    network,
):
    """Record a Bull Bitcoin wallet-export route on an existing wallet.

    Descriptor/file sync remains the source of truth. During `wallets sync`,
    matching rows from the unified Bull export can backfill safe wallet
    metadata such as swap kind and payment hashes without inserting rows that
    would duplicate the descriptor wallet.
    """

    del runtime_config
    normalized_network = core_wallets.normalize_bullbitcoin_wallet_network(network)
    normalized_file = os.path.abspath(os.path.expanduser(source_file))
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    existing_config = json.loads(wallet["config_json"] or "{}")
    existing_routes = list(
        core_wallets.wallet_bullbitcoin_wallet_export_config(existing_config)
    )
    next_route = {
        "source_file": normalized_file,
        "network": normalized_network,
    }
    if next_route not in existing_routes:
        existing_routes.append(next_route)
    updated = core_wallets.update_wallet(
        conn,
        workspace_ref,
        profile_ref,
        wallet_ref,
        {
            "config": {
                core_wallets.BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY: existing_routes,
            },
            "clear": [],
        },
    )
    return {
        "wallet": updated,
        "source_file": normalized_file,
        "network": normalized_network,
        "routes": existing_routes,
    }


def attach_btcpay_provenance_to_wallet(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    wallet_ref,
    backend_name,
    store_id,
    payment_method_id,
):
    """Record a BTCPay provenance route on an already-configured wallet.

    Mirrors the desktop "Map existing wallets" mode. Descriptor/file sync
    remains the balance source; BTCPay just enriches matching transactions
    with comments and labels during `wallets sync`.
    """

    store_id = core_wallets.normalize_btcpay_store_id(store_id)
    payment_method_id = core_wallets.normalize_btcpay_payment_method_id(
        payment_method_id
    )
    require_wallet_history_payment_method(payment_method_id)
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    backend = resolve_backend(runtime_config, backend_name)
    kind = core_sync.normalize_backend_kind(backend["kind"])
    if kind != "btcpay":
        raise AppError(
            f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
            code="validation",
            hint="Use a BTCPay backend for BTCPay provenance enrichment.",
        )
    existing_config = json.loads(wallet["config_json"] or "{}")
    existing_routes = list(
        core_wallets.wallet_btcpay_provenance_config(existing_config)
    )
    next_route = {
        "backend": backend["name"].lower(),
        "store_id": store_id,
        "payment_method_id": payment_method_id,
    }
    if next_route not in existing_routes:
        existing_routes.append(next_route)
    return core_wallets.update_wallet(
        conn,
        workspace_ref,
        profile_ref,
        wallet_ref,
        {
            "config": {
                core_wallets.BTCPAY_PROVENANCE_CONFIG_KEY: existing_routes,
            },
        },
    )


def sync_btcpay_commercial_provenance(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    backend_name,
    store_id,
    page_size,
    checkpoint=None,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    backend = resolve_backend(runtime_config, backend_name)
    kind = core_sync.normalize_backend_kind(backend["kind"])
    if kind != "btcpay":
        raise AppError(
            f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
            code="validation",
            hint="Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token-stdin` or `--token-fd FD`.",
        )
    btcpay_meta = {}
    checkpoint = checkpoint if isinstance(checkpoint, dict) else {}
    invoices = fetch_btcpay_invoice_provenance(
        backend,
        store_id=store_id,
        page_size=page_size,
        checkpoint=checkpoint,
        metadata=btcpay_meta,
    )
    outcome = core_commercial.upsert_btcpay_provenance(
        conn,
        workspace,
        profile,
        backend_name=backend["name"],
        invoices=invoices,
    )
    checkpoint.update(
        {
            "backend": {"name": backend["name"], "kind": kind},
            "btcpay_invoice_pages": btcpay_meta.get("btcpay_invoice_pages", {}),
            "btcpay_invoice_pagination": btcpay_meta.get("btcpay_invoice_pagination", {}),
            "store_id": store_id,
        }
    )
    return {
        **outcome,
        "backend": backend["name"],
        "backend_kind": kind,
        "backend_url": redact_backend_url(backend["url"]),
        "store_id": store_id,
        "page_size": page_size,
        "pages_fetched": btcpay_meta.get("pages_fetched", 0),
        "stopped_by_known_page": bool(btcpay_meta.get("stopped_by_known_page")),
        "stop_reason": btcpay_meta.get("stop_reason"),
        "deep_audit": btcpay_meta.get("deep_audit"),
        "freshness_checkpoint": checkpoint,
    }


def resolve_descriptor_branch_index(plan, branch):
    if branch in (None, "", "all"):
        return None
    normalized = str(branch).strip().lower()
    if normalized in {"0", "receive", "external"}:
        return 0
    if normalized in {"1", "change", "internal"}:
        return 1
    raise AppError("Descriptor branch must be one of: all, receive, change, 0, 1")


def derive_wallet_targets(conn, workspace_ref, profile_ref, wallet_ref, branch=None, start=0, count=None):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"] or "{}")
    plan = load_wallet_descriptor_plan_from_config(config) if config.get("descriptor") else None
    if plan is None:
        raise AppError(f"Wallet '{wallet['label']}' does not have a descriptor configured")
    if start < 0:
        raise AppError("Descriptor derivation start must be non-negative")
    count = count if count is not None else plan.gap_limit
    if count <= 0:
        raise AppError("Descriptor derivation count must be positive")
    branch_index = resolve_descriptor_branch_index(plan, branch)
    return [
        core_sync_backends.sync_target_from_derived(target)
        for target in derive_descriptor_targets(
            plan,
            branch_index=branch_index,
            start=start,
            end=start + count,
        )
    ]


def identify_wallet_owners(
    conn,
    workspace_ref,
    profile_ref,
    *,
    wallet_refs=None,
    addresses=None,
    txids=None,
    candidates=None,
    file=None,
    csv=None,
    scan_to_index=None,
    verify_on_chain=False,
    verify_backend=None,
    runtime_config=None,
):
    """Reconcile a list of addresses / txids against the profile's wallets.

    Returns a structured report (``results`` + ``summary`` + ``warnings``)
    classifying each input as owned (naming the wallet, branch and derivation
    index) or external/unknown. ``--csv`` smart-harvests addresses/txids from a
    spreadsheet of any common shape. ``--verify-on-chain`` resolves an Esplora or
    Electrum backend so unseen txids get a per-leg payment/transfer breakdown.
    """
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)

    wallet_ids = None
    if wallet_refs:
        wallet_ids = [resolve_wallet(conn, profile["id"], ref)["id"] for ref in wallet_refs]

    file_text = core_ownership.read_text_file(file, label="candidate file") if file else None
    csv_text = core_ownership.read_text_file(csv, label="CSV file") if csv else None

    if scan_to_index is None:
        scan_to_index = core_ownership.DEFAULT_SCAN_TO_INDEX
    if scan_to_index < 0:
        raise AppError("--scan-to-index must be non-negative", code="validation")

    if not any([addresses, txids, candidates, file_text, csv_text]):
        raise AppError(
            "Provide at least one --address, --txid, --candidate, --file, or --csv input to check",
            code="validation",
            hint="Example: wallets identify --address bc1q... --txid <64-hex>",
        )

    def _run(verify_fetcher):
        return core_ownership.identify(
            conn,
            profile["id"],
            addresses=addresses,
            txids=txids,
            candidates=candidates,
            file_text=file_text,
            csv_text=csv_text,
            wallet_ids=wallet_ids,
            scan_to_index=scan_to_index,
            verify_fetcher=verify_fetcher,
        )

    if not verify_on_chain:
        return _run(None)

    backend = core_sync_backends.resolve_verify_backend(runtime_config, verify_backend)
    # One reused connection for the whole batch (Electrum); stateless for Esplora.
    with core_sync_backends.verify_session(backend) as fetcher:
        return _run(fetcher)


TRANSACTION_SORT_COLUMNS = {
    "occurred-at": "t.occurred_at",
    "amount": "t.amount",
    "fiat-value": "COALESCE(t.fiat_value, 0)",
    "fee": "t.fee",
}
MAX_TRANSACTION_PAGE_SIZE = 1000


def _transaction_cursor_filters(
    workspace_id,
    profile_id,
    wallet_id=None,
    direction=None,
    asset=None,
    start_ts=None,
    end_ts=None,
):
    return {
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "wallet_id": wallet_id or "",
        "direction": direction or "",
        "asset": asset.upper() if asset else "",
        "start": start_ts or "",
        "end": end_ts or "",
    }


def _decode_transaction_cursor(cursor, sort, order, filters):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        if payload.get("sort") != sort or payload.get("order") != order:
            raise ValueError("cursor sort/order mismatch")
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        required = {"sort", "order", "filters", "value", "occurred_at", "created_at", "id"}
        if not required.issubset(payload):
            raise ValueError("missing cursor fields")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        ) from exc


def _transaction_cursor_value(row, sort):
    if sort == "occurred-at":
        return row["occurred_at"]
    if sort == "amount":
        return int(row["amount"])
    if sort == "fee":
        return int(row["fee"])
    if sort == "fiat-value":
        return float(row["fiat_value"] or 0)
    raise AppError(
        f"Unsupported transaction sort: {sort}",
        code="validation",
        hint="Use one of: occurred-at, amount, fiat-value, fee.",
    )


def _encode_transaction_cursor(row, sort, order, filters):
    payload = {
        "sort": sort,
        "order": order,
        "filters": filters,
        "value": _transaction_cursor_value(row, sort),
        "occurred_at": row["occurred_at"],
        "created_at": row["_created_at"],
        "id": row["id"],
    }
    token = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def list_transactions(
    conn,
    workspace_ref,
    profile_ref,
    wallet_ref=None,
    limit=100,
    *,
    direction=None,
    asset=None,
    start=None,
    end=None,
    cursor=None,
    sort="occurred-at",
    order="desc",
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if limit > MAX_TRANSACTION_PAGE_SIZE:
        raise AppError(
            f"--limit cannot exceed {MAX_TRANSACTION_PAGE_SIZE}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_TRANSACTION_PAGE_SIZE}.",
        )
    if direction and direction not in {"inbound", "outbound"}:
        raise AppError("--direction must be inbound or outbound", code="validation")
    sort_column = TRANSACTION_SORT_COLUMNS.get(sort)
    if not sort_column:
        raise AppError(
            f"Unsupported transaction sort: {sort}",
            code="validation",
            hint="Use one of: occurred-at, amount, fiat-value, fee.",
        )
    if order not in {"asc", "desc"}:
        raise AppError("--order must be asc or desc", code="validation")
    order_sql = order.upper()
    if sort == "occurred-at":
        order_by = f"t.occurred_at {order_sql}, t.created_at {order_sql}, t.id {order_sql}"
    else:
        order_by = f"{sort_column} {order_sql}, t.occurred_at DESC, t.created_at DESC, t.id DESC"

    params = [profile["id"]]
    filters = ["t.profile_id = ?"]
    start_ts = _iso_z(_parse_iso_datetime(start, "start")) if start else None
    end_ts = _iso_z(_parse_iso_datetime(end, "end")) if end else None
    wallet_id = ""
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_id = wallet["id"]
        filters.append("t.wallet_id = ?")
        params.append(wallet_id)
    if direction:
        filters.append("t.direction = ?")
        params.append(direction)
    if asset:
        filters.append("upper(t.asset) = ?")
        params.append(asset.upper())
    if start_ts:
        filters.append("t.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        filters.append("t.occurred_at <= ?")
        params.append(end_ts)

    cursor_filters = _transaction_cursor_filters(
        workspace["id"],
        profile["id"],
        wallet_id,
        direction,
        asset,
        start_ts,
        end_ts,
    )
    cursor_data = _decode_transaction_cursor(cursor, sort, order, cursor_filters)
    if cursor_data:
        if sort == "occurred-at":
            op = ">" if order == "asc" else "<"
            filters.append(
                f"(t.occurred_at {op} ? OR "
                f"(t.occurred_at = ? AND t.created_at {op} ?) OR "
                f"(t.occurred_at = ? AND t.created_at = ? AND t.id {op} ?))"
            )
            params.extend(
                [
                    cursor_data["occurred_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["id"],
                ]
            )
        else:
            primary_op = ">" if order == "asc" else "<"
            filters.append(
                f"({sort_column} {primary_op} ? OR "
                f"({sort_column} = ? AND "
                "(t.occurred_at < ? OR "
                "(t.occurred_at = ? AND t.created_at < ?) OR "
                "(t.occurred_at = ? AND t.created_at = ? AND t.id < ?))))"
            )
            params.extend(
                [
                    cursor_data["value"],
                    cursor_data["value"],
                    cursor_data["occurred_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["id"],
                ]
            )
    params.append(limit + 1)
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            COALESCE(t.external_id, '') AS external_id,
            t.occurred_at,
            t.confirmed_at,
            t.created_at AS _created_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_rate,
            t.fiat_value,
            t.fiat_rate_exact,
            t.fiat_value_exact,
            t.fiat_price_source,
            t.pricing_source_kind,
            t.pricing_provider,
            t.pricing_pair,
            t.pricing_timestamp,
            t.pricing_fetched_at,
            t.pricing_granularity,
            t.pricing_method,
            t.pricing_external_ref,
            t.pricing_quality,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            t.excluded
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {' AND '.join(filters)}
        ORDER BY {order_by}
        LIMIT ?
        """,
        params,
    ).fetchall()
    has_more = len(rows) > limit
    page = rows[:limit]
    tags_by_transaction = {row["id"]: [] for row in page}
    if page:
        placeholders = ", ".join("?" for _ in page)
        for tag in conn.execute(
            f"""
            SELECT tt.transaction_id, tags.code, tags.label
            FROM transaction_tags tt
            JOIN tags ON tags.id = tt.tag_id
            WHERE tt.transaction_id IN ({placeholders})
            ORDER BY tt.transaction_id ASC, tags.code ASC
            """,
            [row["id"] for row in page],
        ).fetchall():
            tags_by_transaction.setdefault(tag["transaction_id"], []).append(
                {"code": tag["code"], "label": tag["label"]}
            )
    results = []
    for row in page:
        record = dict(row)
        record.pop("_created_at", None)
        record["amount_msat"] = int(record["amount"])
        record["amount"] = float(msat_to_btc(record["amount"]))
        record["fee_msat"] = int(record["fee"])
        record["fee"] = float(msat_to_btc(record["fee"]))
        if record["fiat_rate"] is not None and float(record["fiat_rate"]) <= 0:
            record["fiat_rate"] = None
        if record["fiat_value"] is not None and float(record["fiat_value"]) <= 0:
            record["fiat_value"] = None
        record["excluded"] = bool(record["excluded"])
        record["tags"] = tags_by_transaction.get(record["id"], [])
        results.append(record)
    next_cursor = _encode_transaction_cursor(page[-1], sort, order, cursor_filters) if has_more and page else None
    return results, {
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": limit,
        "sort": sort,
        "order": order,
    }


def available_quantity(lots):
    total = Decimal("0")
    for lot in lots:
        total += lot["quantity"]
    return total


def consume_lots(lots, quantity, algorithm):
    remaining = dec(quantity)
    cost_basis = Decimal("0")
    while remaining > 0:
        if not lots:
            raise AppError("Not enough lots to consume")
        lot = lots[0] if algorithm == "FIFO" else lots[-1]
        take = min(remaining, lot["quantity"])
        cost_basis += take * lot["unit_cost"]
        lot["quantity"] -= take
        remaining -= take
        if lot["quantity"] <= Decimal("0"):
            if algorithm == "FIFO":
                lots.pop(0)
            else:
                lots.pop()
    return cost_basis


def latest_rates_for_profile(conn, profile_id):
    return core_reports.latest_transaction_rates_for_profile(conn, profile_id)


def auto_price_transactions_from_rates_cache(conn, profile):
    missing_price_sql = core_rates.transaction_price_missing_sql_unqualified()
    tx_rows = conn.execute(
        """
        SELECT id, occurred_at, asset, amount, fiat_currency, fiat_rate, fiat_value,
               fiat_rate_exact, fiat_value_exact, fiat_price_source,
               pricing_source_kind, pricing_quality, confirmed_at
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
          AND (
            {missing_price_sql}
            OR (
              fiat_price_source = ?
              AND pricing_source_kind IS NULL
              AND pricing_quality IS NULL
            )
          )
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """.format(missing_price_sql=missing_price_sql),
        (profile["id"], pricing.LEGACY_SOURCE_RATES_CACHE),
    ).fetchall()
    auto_priced = 0
    for row in tx_rows:
        price_was_missing = (
            row["fiat_rate"] is None
            and row["fiat_value"] is None
            and row["fiat_rate_exact"] is None
            and row["fiat_value_exact"] is None
        )
        pair = core_rates.transaction_rate_pair(row["asset"], row["fiat_currency"] or profile["fiat_currency"])
        if pair is None:
            continue
        pricing_at = row["confirmed_at"] or row["occurred_at"]
        cached_rate = core_rates.get_cached_rate_at_or_before(conn, pair, pricing_at)
        if cached_rate is None:
            continue
        rate = pricing.decimal_from_exact(
            row["fiat_rate_exact"],
            cached_rate.get("rate_exact"),
            row["fiat_rate"],
            cached_rate["rate"],
        )
        fiat_value = pricing.decimal_from_exact(row["fiat_value_exact"])
        if fiat_value is None and rate is not None and row["amount"] > 0:
            fiat_value = rate * msat_to_btc(row["amount"])
        if fiat_value is None:
            fiat_value = pricing.decimal_from_exact(row["fiat_value"])
        if rate is None and fiat_value is not None and row["amount"] > 0:
            rate = fiat_value / msat_to_btc(row["amount"])
        source_kind = pricing.rate_cache_source_kind(cached_rate)
        quality = pricing.rate_cache_quality(cached_rate)
        payload = pricing.pricing_payload(
            rate=rate,
            value=fiat_value,
            source_kind=source_kind,
            quality=quality,
            provider=cached_rate["source"],
            pair=cached_rate["pair"],
            pricing_timestamp=cached_rate["timestamp"],
            fetched_at=cached_rate["fetched_at"],
            granularity=cached_rate.get("granularity"),
            method=cached_rate.get("method"),
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = ?, fiat_value = ?, fiat_price_source = ?,
                fiat_rate_exact = ?, fiat_value_exact = ?,
                pricing_source_kind = ?, pricing_provider = ?, pricing_pair = ?,
                pricing_timestamp = ?, pricing_fetched_at = ?,
                pricing_granularity = ?, pricing_method = ?,
                pricing_external_ref = ?, pricing_quality = ?
            WHERE id = ?
            """,
            (
                payload["fiat_rate"],
                payload["fiat_value"],
                payload["fiat_price_source"],
                payload["fiat_rate_exact"],
                payload["fiat_value_exact"],
                payload["pricing_source_kind"],
                payload["pricing_provider"],
                payload["pricing_pair"],
                payload["pricing_timestamp"],
                payload["pricing_fetched_at"],
                payload["pricing_granularity"],
                payload["pricing_method"],
                payload["pricing_external_ref"],
                payload["pricing_quality"],
                row["id"],
            ),
        )
        if price_was_missing:
            auto_priced += 1
    return auto_priced


def _duplicate_label_warnings(wallet_refs_by_id):
    """Warn when two wallets in a profile share a label.

    RP2 keys per-account holdings (and exchange identity) by the wallet *label*,
    so two wallets sharing one label merge their balances, and a derived
    self-transfer routed by label can land on the wrong account. The books'
    totals stay correct; the per-wallet attribution does not. Surfaced as a
    non-blocking warning so the user renames them.
    """
    ids_by_label: dict[str, list[str]] = {}
    for ref in wallet_refs_by_id.values():
        label = ref.get("label")
        if not label:
            continue
        ids_by_label.setdefault(str(label), []).append(str(ref.get("id")))
    warnings = []
    for label, ids in sorted(ids_by_label.items()):
        if len(ids) > 1:
            warnings.append(
                {
                    "code": "duplicate_wallet_label",
                    "label": label,
                    "wallet_ids": sorted(ids),
                    "message": (
                        f"{len(ids)} wallets share the label '{label}'. Reports key "
                        "holdings by wallet label, so their balances merge and a "
                        "derived self-transfer can be attributed to the wrong "
                        "wallet. Rename them to be unique."
                    ),
                }
            )
    return warnings


def build_ledger_state(conn, profile):
    require_tax_processing_supported(profile)
    rows = conn.execute(
        """
        SELECT
            t.*,
            w.label AS wallet_label,
            w.kind AS wallet_kind,
            w.account_id AS wallet_account_id,
            w.config_json AS config_json,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE t.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile["id"],),
    ).fetchall()
    manual_pair_records = conn.execute(
        "SELECT * FROM transaction_pairs WHERE profile_id = ? AND deleted_at IS NULL",
        (profile["id"],),
    ).fetchall()
    direct_payout_records = conn.execute(
        """
        SELECT p.*, t.asset AS out_asset, t.amount AS out_amount_msat
        FROM direct_swap_payouts p
        JOIN transactions t ON t.id = p.out_transaction_id
        WHERE p.profile_id = ? AND p.deleted_at IS NULL
        """,
        (profile["id"],),
    ).fetchall()
    tax_engine = build_tax_engine(profile)
    rates = latest_rates_for_profile(conn, profile["id"])
    # Build refs for EVERY profile wallet, not just wallets that have rows: the
    # ownership deriver can route a self-transfer into a wallet that recorded no
    # inbound row (sync gap), and the intra path resolves the destination ref by
    # wallet_id — a rowless destination would otherwise KeyError.
    wallet_refs_by_id = {}
    for wallet in conn.execute(
        """
        SELECT
            w.id AS id,
            w.label AS label,
            w.account_id AS wallet_account_id,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ?
        """,
        (profile["id"],),
    ).fetchall():
        wallet_refs_by_id[wallet["id"]] = {
            "id": wallet["id"],
            "label": wallet["label"],
            "wallet_account_id": wallet["wallet_account_id"],
            "account_code": wallet["account_code"],
            "account_label": wallet["account_label"],
        }
    # The address-ownership index is only useful (and only worth its descriptor
    # derivation cost) when some on-chain outbound carries full transaction JSON
    # to read outputs from. Skip the build entirely for pure CSV / Lightning
    # profiles.
    warnings = _duplicate_label_warnings(wallet_refs_by_id)
    owned_index = None
    has_onchain_outbound = any(
        row["direction"] == "outbound" and (row["raw_json"] or "").find('"vout"') != -1
        for row in rows
    )
    if has_onchain_outbound:
        index_wallets = core_ownership.load_profile_wallets(conn, profile["id"])
        owned_index, ownership_warnings = core_ownership.build_owned_index(
            conn, profile["id"], index_wallets
        )
        warnings.extend(
            {"code": "ownership_index", "message": str(message)}
            for message in ownership_warnings or ()
        )
    engine_state = tax_engine.build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=manual_pair_records,
            direct_payout_records=direct_payout_records,
            owned_index=owned_index,
        )
    )
    return {
        "entries": engine_state.entries,
        "quarantines": engine_state.quarantines,
        "intra_audit": engine_state.intra_audit,
        "cross_asset_pairs": engine_state.cross_asset_pairs,
        "direct_swap_payouts": engine_state.direct_swap_payouts,
        "tax_summary": engine_state.tax_summary,
        "account_holdings": engine_state.account_holdings,
        "wallet_holdings": engine_state.wallet_holdings,
        "latest_rates": rates,
        "warnings": warnings,
    }


def process_journals(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(profile)
    conn.execute("SAVEPOINT journals_process")
    try:
        auto_priced = auto_price_transactions_from_rates_cache(conn, profile)
        state = build_ledger_state(conn, profile)
        conn.execute("DELETE FROM journal_entries WHERE profile_id = ?", (profile["id"],))
        conn.execute("DELETE FROM journal_quarantines WHERE profile_id = ?", (profile["id"],))
        conn.execute("DELETE FROM journal_tax_summary WHERE profile_id = ?", (profile["id"],))
        conn.execute("DELETE FROM journal_account_holdings WHERE profile_id = ?", (profile["id"],))
        conn.execute("DELETE FROM journal_wallet_holdings WHERE profile_id = ?", (profile["id"],))
        created_at = now_iso()
        pricing_by_tx = {
            row["id"]: row
            for row in conn.execute(
                """
                SELECT id, pricing_source_kind, pricing_quality
                FROM transactions
                WHERE profile_id = ?
                """,
                (profile["id"],),
            ).fetchall()
        }
        journal_entry_rows = []
        for entry in state["entries"]:
            exact_payload = pricing.journal_exact_payload(entry)
            tx_pricing = pricing_by_tx.get(entry["transaction_id"])
            journal_entry_rows.append(
                (
                    entry["id"],
                    entry["workspace_id"],
                    entry["profile_id"],
                    entry["transaction_id"],
                    entry["wallet_id"],
                    entry["account_id"],
                    entry["occurred_at"],
                    entry["entry_type"],
                    entry["asset"],
                    btc_to_msat(entry["quantity"]),
                    float(entry["fiat_value"]),
                    float(entry["unit_cost"]),
                    float(entry["cost_basis"]) if entry["cost_basis"] is not None else None,
                    float(entry["proceeds"]) if entry["proceeds"] is not None else None,
                    float(entry["gain_loss"]) if entry["gain_loss"] is not None else None,
                    exact_payload["fiat_value_exact"],
                    exact_payload["unit_cost_exact"],
                    exact_payload["cost_basis_exact"],
                    exact_payload["proceeds_exact"],
                    exact_payload["gain_loss_exact"],
                    tx_pricing["pricing_source_kind"] if tx_pricing else None,
                    tx_pricing["pricing_quality"] if tx_pricing else None,
                    entry["description"],
                    entry.get("at_category"),
                    entry.get("at_kennzahl"),
                    entry.get("capital_gains_type"),
                    created_at,
                )
            )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, fiat_value_exact, unit_cost_exact,
                cost_basis_exact, proceeds_exact, gain_loss_exact, pricing_source_kind,
                pricing_quality, description, at_category, at_kennzahl, capital_gains_type,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            journal_entry_rows,
        )
        # Collapse legs that map to the same real transaction (e.g. multiple
        # synthetic payout/split legs of one out row) so two rows never collide
        # on journal_quarantines' PRIMARY KEY and abort the whole run.
        deduped_quarantines = core_tax_events.dedupe_quarantines(state["quarantines"])
        conn.executemany(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    quarantine["transaction_id"],
                    quarantine["workspace_id"],
                    quarantine["profile_id"],
                    quarantine["reason"],
                    quarantine["detail_json"],
                    created_at,
                )
                for quarantine in deduped_quarantines
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_tax_summary(
                id, workspace_id, profile_id, year, asset, transaction_type,
                capital_gains_type, quantity, proceeds, cost_basis, gain_loss, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    str(uuid.uuid4()),
                    profile["workspace_id"],
                    profile["id"],
                    int(row["year"]),
                    row["asset"],
                    row["transaction_type"],
                    row.get("capital_gains_type"),
                    int(row.get("quantity_msat") or btc_to_msat(row["quantity"])),
                    float(row["proceeds"]),
                    float(row["cost_basis"]),
                    float(row["gain_loss"]),
                    created_at,
                )
                for row in state["tax_summary"]
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_account_holdings(
                id, workspace_id, profile_id, account_id, account_code, account_label,
                asset, quantity, cost_basis, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    str(uuid.uuid4()),
                    profile["workspace_id"],
                    profile["id"],
                    account_id,
                    account_code,
                    account_label,
                    asset,
                    btc_to_msat(value["quantity"]),
                    float(value["cost_basis"]),
                    created_at,
                )
                for (account_id, account_code, account_label, asset), value in state[
                    "account_holdings"
                ].items()
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_wallet_holdings(
                id, workspace_id, profile_id, wallet_id, wallet_label, account_code,
                asset, quantity, cost_basis, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    str(uuid.uuid4()),
                    profile["workspace_id"],
                    profile["id"],
                    wallet_id,
                    wallet_label,
                    account_code,
                    asset,
                    btc_to_msat(value["quantity"]),
                    float(value["cost_basis"]),
                    created_at,
                )
                for (wallet_id, wallet_label, account_code, asset), value in state[
                    "wallet_holdings"
                ].items()
            ],
        )
        tx_count = conn.execute(
            """
            SELECT COUNT(*) AS count
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            """,
            (profile["id"],),
        ).fetchone()["count"]
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = ?,
                last_processed_tx_count = ?,
                last_processed_input_version = journal_input_version
            WHERE id = ?
            """,
            (created_at, tx_count, profile["id"]),
        )
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT journals_process")
        conn.execute("RELEASE SAVEPOINT journals_process")
        raise
    conn.execute("RELEASE SAVEPOINT journals_process")
    conn.commit()
    result = {
        "profile": profile["label"],
        "entries_created": len(state["entries"]),
        "quarantined": len(deduped_quarantines),
        "transfers_detected": len(state.get("intra_audit", [])),
        "cross_asset_pairs": len(state.get("cross_asset_pairs", [])),
        "auto_priced": auto_priced,
        "processed_transactions": tx_count,
        "processed_at": created_at,
    }
    if state.get("direct_swap_payouts"):
        result["direct_swap_payouts"] = len(state["direct_swap_payouts"])
    if state.get("warnings"):
        result["warnings"] = state["warnings"]
    return result


def _journal_processing_status(conn, profile):
    current_count, processed_current = _journals_current_for_profile(conn, profile)
    return {
        "last_processed_at": profile["last_processed_at"],
        "last_processed_tx_count": _row_int(profile, "last_processed_tx_count"),
        "journal_input_version": _row_int(profile, "journal_input_version"),
        "last_processed_input_version": _row_int(profile, "last_processed_input_version"),
        "current_active_tx_count": int(current_count or 0),
        "processed_journals_current": processed_current,
    }


def _audit_transaction_refs(conn, profile_id, transaction_ids):
    ids = list(dict.fromkeys(str(value) for value in transaction_ids if value))
    if not ids:
        return {}
    rows = []
    chunk_size = 400
    for start in range(0, len(ids), chunk_size):
        chunk = ids[start : start + chunk_size]
        placeholders = ", ".join("?" for _ in chunk)
        rows.extend(
            conn.execute(
                f"""
                SELECT
                    t.id,
                    t.external_id,
                    t.occurred_at,
                    t.asset,
                    w.label AS wallet
                FROM transactions t
                JOIN wallets w ON w.id = t.wallet_id
                WHERE t.profile_id = ? AND t.id IN ({placeholders})
                """,
                [profile_id, *chunk],
            ).fetchall()
        )
    return {str(row["id"]): dict(row) for row in rows}


def _serialize_intra_audit(rows):
    return [
        {
            "out_id": row["out_id"],
            "in_id": row["in_id"],
            "external_id": row["external_id"],
            "occurred_at": row["occurred_at"],
            "asset": row["asset"],
            "from_wallet": row["from_wallet_label"],
            "to_wallet": row["to_wallet_label"],
            "sent": float(dec(row["crypto_sent"])),
            "sent_msat": btc_to_msat(dec(row["crypto_sent"])),
            "received": float(dec(row["crypto_received"])),
            "received_msat": btc_to_msat(dec(row["crypto_received"])),
            "fee": float(dec(row["crypto_fee"])),
            "fee_msat": btc_to_msat(dec(row["crypto_fee"])),
            "spot_price": float(dec(row["spot_price"])),
        }
        for row in sorted(
            rows,
            key=lambda item: (item["occurred_at"], item["out_id"], item["in_id"]),
        )
    ]


def _serialize_cross_asset_pairs(rows, refs_by_id):
    serialized = []
    for row in sorted(
        rows,
        key=lambda item: (
            refs_by_id.get(str(item["out_id"]), {}).get("occurred_at", ""),
            str(item.get("pair_id") or ""),
            str(item["out_id"]),
            str(item["in_id"]),
        ),
    ):
        out_ref = refs_by_id.get(str(row["out_id"]), {})
        in_ref = refs_by_id.get(str(row["in_id"]), {})
        serialized.append(
            {
                "pair_id": row.get("pair_id"),
                "kind": row.get("kind"),
                "policy": row.get("policy"),
                "out_id": row["out_id"],
                "out_asset": row["out_asset"],
                "out_wallet": out_ref.get("wallet"),
                "out_external_id": out_ref.get("external_id"),
                "out_occurred_at": out_ref.get("occurred_at"),
                "in_id": row["in_id"],
                "in_asset": row["in_asset"],
                "in_wallet": in_ref.get("wallet"),
                "in_external_id": in_ref.get("external_id"),
                "in_occurred_at": in_ref.get("occurred_at"),
            }
        )
    return serialized


def _serialize_direct_swap_payouts(rows, refs_by_id):
    serialized = []
    for row in sorted(
        rows,
        key=lambda item: (
            item.get("payout_occurred_at") or refs_by_id.get(str(item["out_id"]), {}).get("occurred_at", ""),
            str(item.get("payout_id") or ""),
        ),
    ):
        out_ref = refs_by_id.get(str(row["out_id"]), {})
        serialized.append(
            {
                "payout_id": row["payout_id"],
                "kind": row["kind"],
                "policy": row["policy"],
                "out_id": row["out_id"],
                "out_asset": row["out_asset"],
                "out_wallet": out_ref.get("wallet"),
                "out_external_id": out_ref.get("external_id"),
                "out_occurred_at": out_ref.get("occurred_at"),
                "out_amount": float(msat_to_btc(row["out_amount_msat"])),
                "out_amount_msat": int(row["out_amount_msat"]),
                "payout_asset": row["payout_asset"],
                "payout_amount": float(msat_to_btc(row["payout_amount_msat"])),
                "payout_amount_msat": int(row["payout_amount_msat"]),
                "payout_occurred_at": row["payout_occurred_at"],
                "payout_external_id": row["payout_external_id"],
                "counterparty": row["counterparty"],
                "swap_fee": float(msat_to_btc(row["swap_fee_msat"])),
                "swap_fee_msat": int(row["swap_fee_msat"]),
                "swap_fee_kind": row["swap_fee_kind"],
            }
        )
    return serialized


def inspect_transfer_audit(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(profile)
    state = build_ledger_state(conn, profile)
    tx_refs = _audit_transaction_refs(
        conn,
        profile["id"],
        [row["out_id"] for row in state["cross_asset_pairs"]]
        + [row["in_id"] for row in state["cross_asset_pairs"]],
    )
    payout_refs = _audit_transaction_refs(
        conn,
        profile["id"],
        [row["out_id"] for row in state["direct_swap_payouts"]],
    )
    intra_transfers = _serialize_intra_audit(state["intra_audit"])
    cross_asset_pairs = _serialize_cross_asset_pairs(state["cross_asset_pairs"], tx_refs)
    direct_swap_payouts = _serialize_direct_swap_payouts(
        state["direct_swap_payouts"],
        payout_refs,
    )
    return {
        "profile": profile["label"],
        "processing": _journal_processing_status(conn, profile),
        "summary": {
            "same_asset_transfers": len(intra_transfers),
            "cross_asset_pairs": len(cross_asset_pairs),
            "direct_swap_payouts": len(direct_swap_payouts),
            "quarantines": len(core_tax_events.dedupe_quarantines(state["quarantines"])),
        },
        "same_asset_transfers": intra_transfers,
        "cross_asset_pairs": cross_asset_pairs,
        "direct_swap_payouts": direct_swap_payouts,
    }


DEFAULT_EVENTS_LIMIT = 100
MAX_EVENTS_LIMIT = 1000


def _journal_cursor_filters(
    workspace_id,
    profile_id,
    wallet_id=None,
    account_id=None,
    asset=None,
    entry_type=None,
    start_ts=None,
    end_ts=None,
):
    return {
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "wallet_id": wallet_id or "",
        "account_id": account_id or "",
        "asset": asset.upper() if asset else "",
        "entry_type": entry_type.lower() if entry_type else "",
        "start": start_ts or "",
        "end": end_ts or "",
    }


def _cursor_created_at(row):
    if "created_at" in row.keys():
        return row["created_at"]
    return row["_created_at"]


def _encode_event_cursor(row, filters):
    payload = {
        "filters": filters,
        "occurred_at": row["occurred_at"],
        "created_at": _cursor_created_at(row),
        "id": row["id"],
    }
    token = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_event_cursor(cursor, filters):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        required = {"filters", "occurred_at", "created_at", "id"}
        if not required.issubset(payload):
            raise ValueError("missing cursor fields")
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        ) from exc


def list_journal_events(
    conn,
    workspace_ref,
    profile_ref,
    wallet=None,
    account=None,
    asset=None,
    entry_type=None,
    start=None,
    end=None,
    cursor=None,
    limit=None,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_EVENTS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_EVENTS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_EVENTS_LIMIT}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_EVENTS_LIMIT}.",
        )

    where = ["je.profile_id = ?"]
    params = [profile["id"]]
    start_ts = _iso_z(_parse_iso_datetime(start, "start")) if start else None
    end_ts = _iso_z(_parse_iso_datetime(end, "end")) if end else None

    wallet_id = ""
    account_id = ""
    if wallet:
        wallet_row = resolve_wallet(conn, profile["id"], wallet)
        wallet_id = wallet_row["id"]
        where.append("je.wallet_id = ?")
        params.append(wallet_id)
    if account:
        account_row = resolve_account(conn, profile["id"], account)
        account_id = account_row["id"]
        where.append("je.account_id = ?")
        params.append(account_id)
    if asset:
        where.append("upper(je.asset) = ?")
        params.append(asset.upper())
    if entry_type:
        where.append("lower(je.entry_type) = ?")
        params.append(entry_type.lower())
    if start_ts:
        where.append("je.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("je.occurred_at <= ?")
        params.append(end_ts)

    cursor_filters = _journal_cursor_filters(
        workspace["id"],
        profile["id"],
        wallet_id,
        account_id,
        asset,
        entry_type,
        start_ts,
        end_ts,
    )
    cursor_data = _decode_event_cursor(cursor, cursor_filters)
    if cursor_data:
        where.append(
            "(je.occurred_at < ? OR "
            "(je.occurred_at = ? AND je.created_at < ?) OR "
            "(je.occurred_at = ? AND je.created_at = ? AND je.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )

    query = f"""
        SELECT
            je.id,
            je.occurred_at,
            je.created_at,
            je.transaction_id,
            je.wallet_id,
            w.label AS wallet,
            je.account_id,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            je.unit_cost,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            je.fiat_value_exact,
            je.unit_cost_exact,
            je.cost_basis_exact,
            je.proceeds_exact,
            je.gain_loss_exact,
            je.pricing_source_kind,
            je.pricing_quality,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT ?
    """
    params.append(effective_limit + 1)
    rows = conn.execute(query, params).fetchall()

    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    events = []
    for row in page:
        event = dict(row)
        event["quantity_msat"] = int(event["quantity"])
        event["quantity"] = float(msat_to_btc(event["quantity"]))
        if event.get("at_category") is None:
            event.pop("at_category", None)
        if event.get("at_kennzahl") is None:
            event.pop("at_kennzahl", None)
        events.append(event)
    next_cursor = _encode_event_cursor(page[-1], cursor_filters) if has_more and page else None

    return {
        "events": events,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def get_journal_event(conn, workspace_ref, profile_ref, event_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        """
        SELECT
            je.*,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            t.external_id AS transaction_external_id,
            t.direction AS transaction_direction,
            t.counterparty AS transaction_counterparty,
            t.note AS transaction_note
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ? AND je.id = ?
        """,
        (profile["id"], event_id),
    ).fetchone()
    if not row:
        raise AppError(
            f"Journal event '{event_id}' not found",
            code="not_found",
            hint="Run `kassiber journals events list` to find valid event ids.",
        )
    event = dict(row)
    event["quantity_msat"] = int(event["quantity"])
    event["quantity"] = float(msat_to_btc(event["quantity"]))
    return event


def list_journal_entries(conn, workspace_ref, profile_ref, limit=200, cursor=None, return_meta=False):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if limit is not None and limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    limit_clause = ""
    params = [profile["id"]]
    cursor_clause = ""
    cursor_filters = _journal_cursor_filters(workspace["id"], profile["id"])
    cursor_data = _decode_event_cursor(cursor, cursor_filters)
    if cursor_data:
        cursor_clause = (
            "AND (je.occurred_at < ? OR "
            "(je.occurred_at = ? AND je.created_at < ?) OR "
            "(je.occurred_at = ? AND je.created_at = ? AND je.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )
    if limit is not None:
        limit_clause = "LIMIT ?"
        params.append(limit + 1)
    rows = conn.execute(
        f"""
        SELECT
            je.id,
            je.occurred_at,
            je.created_at AS _created_at,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ? {cursor_clause}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        {limit_clause}
        """,
        params,
    ).fetchall()
    has_more = bool(limit is not None and len(rows) > limit)
    page = rows[:limit] if limit is not None else rows
    results = []
    for row in page:
        entry = dict(row)
        entry.pop("_created_at", None)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        if entry.get("at_category") is None:
            entry.pop("at_category", None)
        if entry.get("at_kennzahl") is None:
            entry.pop("at_kennzahl", None)
        results.append(entry)
    meta = {
        "next_cursor": _encode_event_cursor(
            {
                "occurred_at": page[-1]["occurred_at"],
                "created_at": page[-1]["_created_at"],
                "id": page[-1]["id"],
            },
            cursor_filters,
        )
        if has_more and page
        else None,
        "has_more": has_more,
        "limit": limit,
    }
    return (results, meta) if return_meta else results


def list_quarantines(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.asset,
            t.amount,
            t.fee,
            q.reason,
            q.detail_json
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY t.occurred_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        detail = json.loads(row["detail_json"] or "{}")
        output.append(
            {
                "transaction_id": row["transaction_id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "confirmed_at": row["confirmed_at"],
                "wallet": row["wallet"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"])),
                "amount_msat": int(row["amount"]),
                "fee": float(msat_to_btc(row["fee"])),
                "fee_msat": int(row["fee"]),
                "reason": row["reason"],
                "detail": detail,
            }
        )
    return output


def show_quarantine(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    row = conn.execute(
        """
        SELECT q.transaction_id, q.reason, q.detail_json, q.created_at,
               w.label AS wallet, t.external_id, t.occurred_at, t.confirmed_at, t.asset,
               t.amount, t.fee, t.fiat_rate, t.fiat_value, t.fiat_rate_exact,
               t.fiat_value_exact, t.pricing_source_kind, t.pricing_provider,
               t.pricing_pair, t.pricing_timestamp, t.pricing_granularity,
               t.pricing_method, t.pricing_quality, t.direction, t.excluded
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ? AND q.transaction_id = ?
        """,
        (profile["id"], tx["id"]),
    ).fetchone()
    if not row:
        raise AppError(
            f"Transaction '{tx_ref}' has no active quarantine",
            code="not_found",
            hint="Only transactions flagged during `journals process` appear here.",
        )
    return {
        "transaction_id": row["transaction_id"],
        "external_id": row["external_id"] or "",
        "wallet": row["wallet"],
        "occurred_at": row["occurred_at"],
        "confirmed_at": row["confirmed_at"],
        "direction": row["direction"],
        "asset": row["asset"],
        "amount": float(msat_to_btc(row["amount"])),
        "amount_msat": int(row["amount"]),
        "fee": float(msat_to_btc(row["fee"])),
        "fee_msat": int(row["fee"]),
        "fiat_rate": row["fiat_rate"],
        "fiat_value": row["fiat_value"],
        "fiat_rate_exact": row["fiat_rate_exact"],
        "fiat_value_exact": row["fiat_value_exact"],
        "pricing": {
            "source_kind": row["pricing_source_kind"],
            "provider": row["pricing_provider"],
            "pair": row["pricing_pair"],
            "timestamp": row["pricing_timestamp"],
            "granularity": row["pricing_granularity"],
            "method": row["pricing_method"],
            "quality": row["pricing_quality"],
        },
        "excluded": bool(row["excluded"]),
        "reason": row["reason"],
        "detail": json.loads(row["detail_json"] or "{}"),
        "quarantined_at": row["created_at"],
    }


def _ensure_quarantined(conn, profile_id, transaction_id):
    row = conn.execute(
        "SELECT reason FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile_id, transaction_id),
    ).fetchone()
    if not row:
        raise AppError(
            "Transaction is not quarantined",
            code="not_found",
            hint="Run `kassiber journals quarantined` to see active entries.",
        )
    return row["reason"]


def resolve_quarantine_price_override(
    conn, workspace_ref, profile_ref, tx_ref, fiat_rate=None, fiat_value=None
):
    if fiat_rate is None and fiat_value is None:
        raise AppError(
            "Provide at least one of --fiat-rate or --fiat-value",
            code="validation",
        )
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    new_rate = dec(fiat_rate) if fiat_rate is not None else None
    new_value = dec(fiat_value) if fiat_value is not None else None
    amount = abs(msat_to_btc(tx["amount"]))
    if new_rate is None and new_value is not None and amount > 0:
        new_rate = new_value / amount
    if new_value is None and new_rate is not None and amount > 0:
        new_value = new_rate * amount
    if new_rate is not None and new_rate <= 0:
        raise AppError("--fiat-rate must be positive", code="validation")
    if new_value is not None and new_value < 0:
        raise AppError("--fiat-value must not be negative", code="validation")
    record = core_metadata.update_transaction_metadata(
        conn,
        workspace_ref,
        profile_ref,
        tx["id"],
        _metadata_hooks(),
        pricing_update={
            "fiat_rate": str(new_rate) if new_rate is not None else None,
            "fiat_value": str(new_value) if new_value is not None else None,
            "source_kind": pricing.SOURCE_MANUAL_OVERRIDE,
            "quality": pricing.QUALITY_EXACT,
            "method": "quarantine_price_override",
        },
        source="cli",
        reason="Resolved quarantine with manual pricing override",
        commit=False,
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    if not record["updated"]:
        invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "price-override",
        "fiat_rate": float(new_rate) if new_rate is not None else None,
        "fiat_value": float(new_value) if new_value is not None else None,
        "fiat_rate_exact": record["fiat_rate_exact"],
        "fiat_value_exact": record["fiat_value_exact"],
        "pricing_source_kind": record["pricing_source_kind"],
        "history_event_id": record["history_event_id"],
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def resolve_quarantine_exclude(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    record = core_metadata.update_transaction_metadata(
        conn,
        workspace_ref,
        profile_ref,
        tx["id"],
        _metadata_hooks(),
        excluded=True,
        source="cli",
        reason="Resolved quarantine by excluding transaction",
        commit=False,
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    if not record["updated"]:
        invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "exclude",
        "excluded": True,
        "history_event_id": record["history_event_id"],
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def clear_quarantine(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "clear",
        "note": "Run `kassiber journals process` to re-evaluate.",
    }


def require_processed_journals(conn, profile):
    _, processed_current = _journals_current_for_profile(conn, profile)
    if not processed_current:
        raise AppError("Reports require fresh journals. Run `kassiber journals process` first.")


def show_status(conn, data_root):
    return build_status_payload(conn, data_root)


def get_profile_details(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    current_profile_id = get_setting(conn, "context_profile")
    current_workspace_id = get_setting(conn, "context_workspace")
    return {
        "id": profile["id"],
        "workspace_id": profile["workspace_id"],
        "workspace_label": workspace["label"],
        "label": profile["label"],
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "tax_long_term_days": profile["tax_long_term_days"],
        "gains_algorithm": profile["gains_algorithm"],
        "last_processed_at": profile["last_processed_at"],
        "last_processed_tx_count": _row_int(profile, "last_processed_tx_count"),
        "journal_input_version": _row_int(profile, "journal_input_version"),
        "last_processed_input_version": _row_int(profile, "last_processed_input_version"),
        "created_at": profile["created_at"],
        "is_current": profile["id"] == current_profile_id and profile["workspace_id"] == current_workspace_id,
    }


def update_profile(conn, workspace_ref, profile_ref, updates):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)

    new_label = updates.get("label")
    new_fiat = updates.get("fiat_currency")
    new_country = updates.get("tax_country")
    new_long_term = updates.get("tax_long_term_days")
    new_algo = updates.get("gains_algorithm")

    merged_fiat = new_fiat if new_fiat is not None else profile["fiat_currency"]
    merged_country = new_country if new_country is not None else profile["tax_country"]
    merged_long_term = new_long_term if new_long_term is not None else profile["tax_long_term_days"]
    merged_algo = new_algo if new_algo is not None else profile["gains_algorithm"]
    merged_label = new_label if new_label is not None else profile["label"]

    if new_long_term is not None and new_long_term < 0:
        raise AppError(
            "Tax long-term days cannot be negative",
            code="validation",
            hint="Use a non-negative integer; pass 0 to treat every disposal as short-term.",
        )
    if new_algo is not None and new_algo.upper() not in RP2_ACCOUNTING_METHODS:
        raise AppError(
            f"Unsupported gains algorithm '{new_algo}'",
            code="validation",
            hint=f"Choose one of: {', '.join(RP2_ACCOUNTING_METHODS)}",
        )
    if new_country is not None:
        require_tax_country_supported_for_profile_mutation(new_country)
    try:
        policy = build_tax_policy(
            {
                "fiat_currency": merged_fiat,
                "tax_country": merged_country,
                "tax_long_term_days": merged_long_term,
            }
        )
    except ValueError as exc:
        raise AppError(str(exc), code="validation") from exc
    normalized_algo = merged_algo.upper()
    policy_changed = (
        policy.fiat_currency != profile["fiat_currency"]
        or policy.tax_country != profile["tax_country"]
        or policy.long_term_days != profile["tax_long_term_days"]
        or normalized_algo != profile["gains_algorithm"]
    )

    conn.execute(
        """
        UPDATE profiles
        SET label = ?, fiat_currency = ?, tax_country = ?, tax_long_term_days = ?, gains_algorithm = ?
        WHERE id = ?
        """,
        (
            merged_label,
            policy.fiat_currency,
            policy.tax_country,
            policy.long_term_days,
            normalized_algo,
            profile["id"],
        ),
    )
    if policy_changed:
        invalidate_journals(conn, profile["id"])
    conn.commit()
    return get_profile_details(conn, workspace["label"], profile["id"])


def cmd_init(conn, args):
    init_app(conn)
    state_root = resolve_effective_state_root(args.data_root)
    effective_data_root = resolve_effective_data_root(args.data_root)
    emit(
        args,
        {
            "version": __version__,
            "state_root": str(state_root),
            "data_root": str(effective_data_root),
            "database": str(resolve_database_path(effective_data_root)),
            "config_root": str(resolve_config_root(args.data_root)),
            "settings_file": str(resolve_settings_path(args.data_root)),
            "exports_root": str(resolve_exports_root(args.data_root)),
            "attachments_root": str(resolve_attachments_root(args.data_root)),
            "env_file": str(args.env_file),
        },
    )


def cmd_status(conn, args):
    payload = show_status(conn, args.data_root)
    payload["default_backend"] = args.runtime_config["default_backend"]
    payload["env_file"] = args.runtime_config["env_file"]
    emit(args, payload)


def cmd_context_show(conn, args):
    emit(args, current_context_snapshot(conn))


def cmd_context_set(conn, args):
    if args.workspace:
        workspace = resolve_workspace(conn, args.workspace)
        set_setting(conn, "context_workspace", workspace["id"])
        if args.profile:
            profile = resolve_profile(conn, workspace["id"], args.profile)
            set_setting(conn, "context_profile", profile["id"])
        else:
            current_profile_id = get_setting(conn, "context_profile")
            if current_profile_id:
                profile = conn.execute(
                    "SELECT 1 FROM profiles WHERE id = ? AND workspace_id = ?",
                    (current_profile_id, workspace["id"]),
                ).fetchone()
                if not profile:
                    set_setting(conn, "context_profile", "")
        conn.commit()
    elif args.profile:
        workspace = resolve_workspace(conn)
        profile = resolve_profile(conn, workspace["id"], args.profile)
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    else:
        raise AppError("Provide --workspace and/or --profile")
    cmd_context_show(conn, args)
