import argparse
import base64
import binascii
import json
import os
import sqlite3
import sys
import uuid
from collections import defaultdict
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from pathlib import Path
from urllib import error as urlerror
from urllib import request as urlrequest

from .. import __version__
from ..backends import (
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
    resolve_backend,
    set_default_backend,
    update_db_backend,
)
from ..core import accounts as core_accounts
from ..core import attachments as core_attachments
from ..core import imports as core_imports
from ..core import metadata as core_metadata
from ..core import rates as core_rates
from ..core import reports as core_reports
from ..core import sync as core_sync
from ..core import sync_backends as core_sync_backends
from ..core import wallets as core_wallets
from ..core.engines import TaxEngineLedgerInputs, build_tax_engine
from ..core.repo import current_context_snapshot
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
    parse_bool,
    parse_int,
    str_or_none,
)
from ..tax_policy import (
    DEFAULT_LONG_TERM_DAYS,
    DEFAULT_TAX_COUNTRY,
    build_tax_policy,
    supported_tax_countries,
)
from ..wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    default_policy_asset_id,
    derive_descriptor_targets,
    liquid_plan_can_unblind,
    load_descriptor_plan,
    normalize_asset_code,
    normalize_chain,
    normalize_network,
)


ACCOUNT_TYPES = {"asset", "liability", "equity", "income", "expense"}
RP2_ACCOUNTING_METHODS = ("FIFO", "LIFO", "HIFO", "LOFO")
WALLET_KINDS = [
    "descriptor",
    "xpub",
    "address",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
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


def http_get_json(url, timeout=30):
    request = urlrequest.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": f"{APP_NAME}/{__version__}",
        },
    )
    try:
        with urlrequest.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise AppError(f"HTTP {exc.code} from backend for {url}: {detail[:200]}") from exc
    except urlerror.URLError as exc:
        raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc

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


def resolve_account(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM accounts
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Account '{ref}' not found")
    return row


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
    query = [
        "SELECT * FROM transactions WHERE profile_id = ?",
    ]
    params = [profile_id]
    if direction is not None:
        query.append("AND direction = ?")
        params.append(direction)
    query.append("AND (id = ? OR external_id = ?) LIMIT 1")
    params.extend([ref, ref])
    row = conn.execute(" ".join(query), tuple(params)).fetchone()
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
        "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0 WHERE id = ?",
        (profile_id,),
    )


TRANSFER_PAIR_KINDS = ("manual", "peg-in", "peg-out", "submarine-swap")
TRANSFER_PAIR_POLICIES = ("carrying-value", "taxable")


def _pair_to_dict(row):
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "in_transaction_id": row["in_transaction_id"],
        "kind": row["kind"],
        "policy": row["policy"],
        "notes": row["notes"],
        "created_at": row["created_at"],
    }


def create_transaction_pair(
    conn,
    workspace_ref,
    profile_ref,
    out_ref,
    in_ref,
    kind="manual",
    policy="carrying-value",
    notes=None,
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
    out_row = resolve_transaction(conn, profile["id"], out_ref, direction="outbound")
    in_row = resolve_transaction(conn, profile["id"], in_ref, direction="inbound")
    if out_row["id"] == in_row["id"]:
        raise AppError("--tx-out and --tx-in must reference different transactions", code="validation")
    if out_row["wallet_id"] == in_row["wallet_id"]:
        raise AppError("Pair legs must be in different wallets", code="validation")
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
        raise AppError(
            f"Cross-asset carrying-value pairs are not yet supported "
            f"(out={out_row['asset']}, in={in_row['asset']}). "
            f"Use --policy taxable for now; carrying-value support is tracked in TODO.md.",
            code="validation",
            hint="Re-run with --policy taxable, or pair two same-asset transactions.",
        )
    existing = conn.execute(
        """
        SELECT id FROM transaction_pairs
        WHERE profile_id = ? AND (out_transaction_id IN (?, ?) OR in_transaction_id IN (?, ?))
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
    pair_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            now_iso(),
        ),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return _pair_to_dict(
        conn.execute("SELECT * FROM transaction_pairs WHERE id = ?", (pair_id,)).fetchone()
    )


def list_transaction_pairs(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            p.*,
            tout.external_id AS out_external_id,
            tout.asset AS out_asset,
            tout.amount AS out_amount_msat,
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
        WHERE p.profile_id = ?
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
            "amount": float(msat_to_btc(row["out_amount_msat"])),
            "amount_msat": int(row["out_amount_msat"]),
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


def delete_transaction_pair(conn, workspace_ref, profile_ref, pair_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM transaction_pairs WHERE id = ? AND profile_id = ?",
        (pair_id, profile["id"]),
    ).fetchone()
    if not row:
        raise AppError(f"Pair '{pair_id}' not found", code="not_found")
    conn.execute("DELETE FROM transaction_pairs WHERE id = ?", (pair_id,))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"deleted": pair_id}


def init_app(conn):
    set_setting(conn, "app_version", __version__)
    conn.commit()


def create_workspace(conn, label):
    workspace_id = str(uuid.uuid4())
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
        (workspace_id, label, now_iso()),
    )
    set_setting(conn, "context_workspace", workspace_id)
    conn.commit()
    return conn.execute("SELECT * FROM workspaces WHERE id = ?", (workspace_id,)).fetchone()


def list_workspaces(conn):
    current = get_setting(conn, "context_workspace")
    rows = conn.execute(
        "SELECT id, label, created_at FROM workspaces ORDER BY created_at ASC"
    ).fetchall()
    return [
        {
            "id": row["id"],
            "label": row["label"],
            "current": "yes" if row["id"] == current else "",
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def ensure_default_accounts(conn, workspace_id, profile_id):
    defaults = [
        ("treasury", "Treasury", "asset", "BTC"),
        ("fees", "Fees", "expense", "BTC"),
        ("external", "External", "equity", None),
    ]
    created_at = now_iso()
    for code, label, account_type, asset in defaults:
        exists = conn.execute(
            "SELECT 1 FROM accounts WHERE profile_id = ? AND code = ?",
            (profile_id, code),
        ).fetchone()
        if exists:
            continue
        conn.execute(
            """
            INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (str(uuid.uuid4()), workspace_id, profile_id, code, label, account_type, asset, created_at),
        )


def create_profile(conn, workspace_ref, label, fiat_currency, gains_algorithm, tax_country, tax_long_term_days):
    workspace = resolve_workspace(conn, workspace_ref)
    if tax_long_term_days < 0:
        raise AppError("Tax long-term days cannot be negative")
    try:
        policy = build_tax_policy(
            {
                "fiat_currency": fiat_currency,
                "tax_country": tax_country,
                "tax_long_term_days": tax_long_term_days,
            }
        )
    except ValueError as exc:
        raise AppError(str(exc)) from exc
    profile_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO profiles(
            id, workspace_id, label, fiat_currency, tax_country, tax_long_term_days, gains_algorithm, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile_id,
            workspace["id"],
            label,
            policy.fiat_currency,
            policy.tax_country,
            policy.long_term_days,
            gains_algorithm.upper(),
            now_iso(),
        ),
    )
    ensure_default_accounts(conn, workspace["id"], profile_id)
    set_setting(conn, "context_workspace", workspace["id"])
    set_setting(conn, "context_profile", profile_id)
    conn.commit()
    return conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,)).fetchone()


def list_profiles(conn, workspace_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    current = get_setting(conn, "context_profile")
    rows = conn.execute(
        """
        SELECT id, label, fiat_currency, tax_country, tax_long_term_days, gains_algorithm, created_at
        FROM profiles
        WHERE workspace_id = ?
        ORDER BY created_at ASC
        """,
        (workspace["id"],),
    ).fetchall()
    return [
        {
            "id": row["id"],
            "label": row["label"],
            "fiat_currency": row["fiat_currency"],
            "tax_country": row["tax_country"],
            "tax_long_term_days": row["tax_long_term_days"],
            "gains_algorithm": row["gains_algorithm"],
            "current": "yes" if row["id"] == current else "",
            "created_at": row["created_at"],
        }
        for row in rows
    ]


def create_account(conn, workspace_ref, profile_ref, code, label, account_type, asset=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    code = normalize_code(code)
    account_type = account_type.lower()
    if account_type not in ACCOUNT_TYPES:
        raise AppError(f"Unsupported account type '{account_type}'. Supported: {', '.join(sorted(ACCOUNT_TYPES))}")
    account_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            account_id,
            workspace["id"],
            profile["id"],
            code,
            label,
            account_type,
            normalize_asset_code(asset) if asset else None,
            now_iso(),
        ),
    )
    conn.commit()
    return conn.execute("SELECT * FROM accounts WHERE id = ?", (account_id,)).fetchone()


def list_accounts(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT id, code, label, account_type, COALESCE(asset, '') AS asset, created_at
        FROM accounts
        WHERE profile_id = ?
        ORDER BY code ASC
        """,
        (profile["id"],),
    ).fetchall()
    return [dict(row) for row in rows]


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


def wallet_policy_asset_id(config, chain, network):
    explicit = str_or_none(config.get("policy_asset"))
    if explicit:
        return normalize_asset_code(explicit)
    if chain == "liquid":
        return normalize_asset_code(default_policy_asset_id(network))
    return ""


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    descriptor_text = read_text_argument(
        getattr(args, "descriptor", None),
        getattr(args, "descriptor_file", None),
        "Descriptor",
    )
    if descriptor_text:
        config["descriptor"] = descriptor_text
    change_descriptor_text = read_text_argument(
        getattr(args, "change_descriptor", None),
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
        config["gap_limit"] = args.gap_limit
    if getattr(args, "policy_asset", None):
        config["policy_asset"] = normalize_asset_code(args.policy_asset)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    if getattr(args, "altbestand", False):
        config["altbestand"] = True
    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network
    return config


def create_wallet(conn, workspace_ref, profile_ref, label, kind, account_ref=None, config=None):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if account_ref:
        account = resolve_account(conn, profile["id"], account_ref)
    else:
        account = resolve_account(conn, profile["id"], "treasury")
    normalized_kind = normalize_wallet_kind(kind)
    config = config or {}
    descriptor_plan = load_wallet_descriptor_plan_from_config(config) if config.get("descriptor") else None
    chain, network = wallet_live_chain_config(config)
    if normalized_kind == "address" and not config.get("addresses") and not config.get("source_file"):
        raise AppError("Address wallets require at least one --address or a file-based source")
    if normalized_kind == "descriptor" and descriptor_plan is None and not config.get("source_file"):
        raise AppError("Descriptor wallets require --descriptor/--descriptor-file or a file-based source")
    if chain == "liquid" and descriptor_plan is None and not config.get("source_file"):
        raise AppError("Liquid live sync currently requires a descriptor with private blinding keys")
    if descriptor_plan and descriptor_plan.chain == "liquid":
        if not liquid_plan_can_unblind(descriptor_plan):
            raise AppError("Liquid descriptor wallets require private blinding keys for full sync and fee accounting")
        if not config.get("backend") and not config.get("source_file"):
            raise AppError("Liquid descriptor wallets require an explicit --backend; no public Liquid default is built in")
        config["policy_asset"] = wallet_policy_asset_id(config, descriptor_plan.chain, descriptor_plan.network)
    elif chain == "liquid" and not config.get("backend") and not config.get("source_file"):
        raise AppError("Liquid wallets require an explicit --backend; no public Liquid default is built in")
    if chain and network:
        config["chain"] = chain
        config["network"] = network
    wallet_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            wallet_id,
            workspace["id"],
            profile["id"],
            account["id"],
            label,
            normalized_kind,
            json.dumps(config, sort_keys=True),
            now_iso(),
        ),
    )
    conn.commit()
    return conn.execute("SELECT * FROM wallets WHERE id = ?", (wallet_id,)).fetchone()


def list_wallets(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            COALESCE(a.code, '') AS account_code,
            COALESCE(a.label, '') AS account_label,
            w.config_json,
            w.created_at
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ?
        ORDER BY w.label ASC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        config = json.loads(row["config_json"] or "{}")
        descriptor_state = ""
        chain, network = wallet_live_chain_config(config)
        if config.get("descriptor"):
            try:
                descriptor_plan = load_descriptor_plan(config)
                descriptor_state = f"{descriptor_plan.chain}:{descriptor_plan.network}"
                chain = descriptor_plan.chain
                network = descriptor_plan.network
            except ValueError:
                descriptor_state = "invalid"
        output.append(
            {
                "id": row["id"],
                "label": row["label"],
                "kind": row["kind"],
                "account": row["account_code"] or row["account_label"],
                "chain": chain or "",
                "network": network or "",
                "backend": config.get("backend", ""),
                "addresses": ",".join(normalize_addresses(config.get("addresses"))),
                "descriptor": descriptor_state,
                "gap_limit": config.get("gap_limit", DEFAULT_DESCRIPTOR_GAP_LIMIT if descriptor_state else ""),
                "altbestand": "yes" if parse_bool(config.get("altbestand"), default=False) else "",
                "source_format": config.get("source_format", ""),
                "source_file": config.get("source_file", ""),
                "created_at": row["created_at"],
            }
        )
    return output


def set_wallet_altbestand(conn, workspace_ref, profile_ref, wallet_ref, enabled):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"] or "{}")
    if enabled:
        config["altbestand"] = True
    else:
        config.pop("altbestand", None)
    conn.execute("UPDATE wallets SET config_json = ? WHERE id = ?", (json.dumps(config, sort_keys=True), wallet["id"]))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "wallet": wallet["label"],
        "altbestand": bool(enabled),
    }


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
        "summary": "Core Lightning CSV-derived wallet (deposits/withdrawals from node exports).",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
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
        "summary": "River Financial CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "custom": {
        "summary": "Custom CSV/JSON source; use with --config/--config-file to describe field mapping.",
        "config_fields": ["source_file", "source_format", "config"],
        "requires": ["source_file"],
    },
}


def list_wallet_kinds():
    rows = []
    for kind in WALLET_KINDS:
        entry = WALLET_KIND_CATALOG.get(kind, {"summary": "", "config_fields": [], "requires": []})
        rows.append(
            {
                "kind": kind,
                "summary": entry["summary"],
                "requires": ", ".join(entry["requires"]),
                "config_fields": ", ".join(entry["config_fields"]),
            }
        )
    return rows


def _wallet_row_to_dict(row):
    config = json.loads(row["config_json"] or "{}")
    descriptor_state = ""
    chain, network = wallet_live_chain_config(config)
    if config.get("descriptor"):
        try:
            descriptor_plan = load_descriptor_plan(config)
            descriptor_state = f"{descriptor_plan.chain}:{descriptor_plan.network}"
            chain = descriptor_plan.chain
            network = descriptor_plan.network
        except ValueError:
            descriptor_state = "invalid"
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "account_id": row["account_id"],
        "account_code": row["account_code"] if "account_code" in row.keys() else None,
        "account_label": row["account_label"] if "account_label" in row.keys() else None,
        "label": row["label"],
        "kind": row["kind"],
        "chain": chain or "",
        "network": network or "",
        "backend": config.get("backend", ""),
        "addresses": normalize_addresses(config.get("addresses")),
        "descriptor": bool(config.get("descriptor")),
        "descriptor_state": descriptor_state,
        "change_descriptor": bool(config.get("change_descriptor")),
        "gap_limit": config.get("gap_limit"),
        "policy_asset": config.get("policy_asset"),
        "altbestand": parse_bool(config.get("altbestand"), default=False),
        "source_file": config.get("source_file", ""),
        "source_format": config.get("source_format", ""),
        "config": config,
        "created_at": row["created_at"],
    }


def get_wallet_details(conn, workspace_ref, profile_ref, wallet_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    return _wallet_row_to_dict(wallet)


def update_wallet(conn, workspace_ref, profile_ref, wallet_ref, updates):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    new_label = updates.get("label")
    new_account = updates.get("account")
    new_altbestand = updates.get("altbestand")
    config_updates = updates.get("config") or {}
    clear_fields = updates.get("clear") or []

    if (
        new_label is None
        and new_account is None
        and new_altbestand is None
        and not config_updates
        and not clear_fields
    ):
        raise AppError(
            "wallets update requires at least one field to change",
            code="validation",
            hint="Pass --label, --account, --set-altbestand/--clear-altbestand, --config/--config-file, or --clear <field>",
        )

    label_value = new_label if new_label is not None else wallet["label"]
    account_id = wallet["account_id"]
    if new_account is not None:
        account = resolve_account(conn, profile["id"], new_account)
        account_id = account["id"]

    config = json.loads(wallet["config_json"] or "{}")
    for field in clear_fields:
        config.pop(field, None)
    for key, value in config_updates.items():
        if value is None:
            config.pop(key, None)
        else:
            config[key] = value
    if new_altbestand is True:
        config["altbestand"] = True
    elif new_altbestand is False:
        config.pop("altbestand", None)

    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network

    conn.execute(
        """
        UPDATE wallets
        SET label = ?, account_id = ?, config_json = ?
        WHERE id = ?
        """,
        (label_value, account_id, json.dumps(config, sort_keys=True), wallet["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    updated = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.id = ?
        """,
        (wallet["id"],),
    ).fetchone()
    return _wallet_row_to_dict(updated)


def delete_wallet(conn, workspace_ref, profile_ref, wallet_ref, cascade=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    tx_count = conn.execute(
        "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = ?",
        (wallet["id"],),
    ).fetchone()["n"]
    if tx_count and not cascade:
        raise AppError(
            f"Wallet '{wallet['label']}' has {tx_count} transaction(s); pass --cascade to delete them too",
            code="conflict",
            hint="Use --cascade to remove the wallet and all associated transactions/journal entries.",
            details={"transactions": tx_count},
        )
    conn.execute("DELETE FROM wallets WHERE id = ?", (wallet["id"],))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "id": wallet["id"],
        "label": wallet["label"],
        "deleted": True,
        "cascaded_transactions": tx_count if cascade else 0,
    }

def import_into_wallet(conn, workspace_ref, profile_ref, wallet_ref, file_path, input_format):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    return _import_file_for_sync(conn, profile, wallet, file_path, input_format)


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


def _attachment_hooks():
    return core_attachments.AttachmentHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        now_iso=now_iso,
    )


def _report_hooks():
    return core_reports.ReportHooks(
        resolve_scope=resolve_scope,
        resolve_wallet=resolve_wallet,
        require_processed_journals=require_processed_journals,
        build_ledger_state=build_ledger_state,
        list_journal_entries=list_journal_entries,
        list_wallets=list_wallets,
        parse_iso_datetime=_parse_iso_datetime,
        iso_z=_iso_z,
        now_iso=now_iso,
        format_table=format_table,
        write_text_pdf=write_text_pdf,
    )


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


def _import_file_for_sync(conn, profile, wallet, file_path, input_format):
    return core_imports.import_file_into_wallet(
        conn,
        profile,
        wallet,
        file_path,
        input_format,
        _import_coordinator_hooks(),
    )


def _insert_records_for_sync(conn, profile, wallet, records, source_label):
    return core_imports.insert_wallet_records(
        conn,
        profile,
        wallet,
        records,
        source_label,
        _import_coordinator_hooks(),
    )


def _wallet_sync_hooks():
    return core_sync.WalletSyncHooks(
        import_file=_import_file_for_sync,
        insert_records=_insert_records_for_sync,
        resolve_backend=resolve_backend,
        resolve_sync_state=core_sync_backends.resolve_wallet_sync_targets,
        normalize_addresses=core_wallets.normalize_addresses,
        backend_adapters={
            "esplora": core_sync_backends.esplora_sync_adapter,
            "electrum": core_sync_backends.electrum_sync_adapter,
            "bitcoinrpc": core_sync_backends.bitcoinrpc_sync_adapter,
        },
    )


def sync_wallet_from_backend(conn, runtime_config, workspace_ref, profile_ref, wallet):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_sync.sync_wallet_from_backend(
        conn,
        runtime_config,
        profile,
        wallet,
        _wallet_sync_hooks(),
    )


def sync_wallet(conn, runtime_config, workspace_ref, profile_ref, wallet_ref=None, sync_all=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if sync_all:
        wallets = conn.execute("SELECT * FROM wallets WHERE profile_id = ? ORDER BY label ASC", (profile["id"],)).fetchall()
    else:
        if not wallet_ref:
            raise AppError("Provide --wallet or use --all")
        wallets = [resolve_wallet(conn, profile["id"], wallet_ref)]
    return core_sync.sync_wallets(
        conn,
        runtime_config,
        profile,
        wallets,
        _wallet_sync_hooks(),
    )


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


def list_transactions(conn, workspace_ref, profile_ref, wallet_ref=None, limit=100):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    params = [profile["id"]]
    wallet_clause = ""
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_clause = "AND t.wallet_id = ?"
        params.append(wallet["id"])
    params.append(limit)
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            COALESCE(t.external_id, '') AS external_id,
            t.occurred_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            COALESCE(t.fiat_rate, 0) AS fiat_rate,
            COALESCE(t.fiat_value, 0) AS fiat_value,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            CASE WHEN t.excluded = 1 THEN 'yes' ELSE '' END AS excluded,
            COALESCE(GROUP_CONCAT(tags.code, ','), '') AS tags
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
        LEFT JOIN tags ON tags.id = tt.tag_id
        WHERE t.profile_id = ? {wallet_clause}
        GROUP BY t.id
        ORDER BY t.occurred_at DESC, t.created_at DESC
        LIMIT ?
        """,
        params,
    ).fetchall()
    results = []
    for row in rows:
        record = dict(row)
        record["amount_msat"] = int(record["amount"])
        record["amount"] = float(msat_to_btc(record["amount"]))
        record["fee_msat"] = int(record["fee"])
        record["fee"] = float(msat_to_btc(record["fee"]))
        results.append(record)
    return results


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
    rows = conn.execute(
        """
        SELECT asset, fiat_rate, fiat_value, amount
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        ORDER BY occurred_at DESC, created_at DESC
        """,
        (profile_id,),
    ).fetchall()
    rates = {}
    for row in rows:
        asset = row["asset"]
        if asset in rates:
            continue
        if row["fiat_rate"] is not None:
            rates[asset] = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rates[asset] = dec(row["fiat_value"]) / msat_to_btc(row["amount"])
    return rates


# -- rates cache -------------------------------------------------------------

SUPPORTED_RATE_PAIRS = ("BTC-USD", "BTC-EUR")
_COINGECKO_VS = {"USD": "usd", "EUR": "eur"}
_COINGECKO_COIN = {"BTC": "bitcoin"}


def _normalize_rate_pair(pair):
    if not pair:
        raise AppError("Pair is required", code="validation")
    raw = pair.strip().upper().replace("/", "-")
    if "-" not in raw:
        raise AppError(
            f"Invalid pair '{pair}'",
            code="validation",
            hint="Use <ASSET>-<FIAT>, e.g. BTC-USD",
        )
    asset, _, fiat = raw.partition("-")
    if not asset or not fiat:
        raise AppError(f"Invalid pair '{pair}'", code="validation")
    return f"{asset}-{fiat}"


def _require_supported_pair(pair):
    normalized = _normalize_rate_pair(pair)
    if normalized not in SUPPORTED_RATE_PAIRS:
        raise AppError(
            f"Pair '{normalized}' is not supported",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    return normalized


def _rate_pair_parts(pair):
    asset, _, fiat = pair.partition("-")
    return asset, fiat


def _transaction_rate_pair(asset, fiat_currency):
    asset_code = str(asset or "").strip().upper()
    fiat_code = str(fiat_currency or "").strip().upper()
    if not asset_code or not fiat_code:
        return None
    asset_aliases = {
        "LBTC": "BTC",
    }
    asset_code = asset_aliases.get(asset_code, asset_code)
    pair = f"{asset_code}-{fiat_code}"
    if pair not in SUPPORTED_RATE_PAIRS:
        return None
    return pair


def upsert_rate(conn, pair, timestamp, rate, source, fetched_at=None):
    normalized = _normalize_rate_pair(pair)
    ts = _iso_z(_parse_iso_datetime(timestamp, "rate_timestamp"))
    fetched = fetched_at or _iso_z(datetime.now(timezone.utc))
    conn.execute(
        """
        INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
        VALUES (?, ?, ?, ?, ?)
        ON CONFLICT(pair, timestamp, source) DO UPDATE SET
            rate = excluded.rate,
            fetched_at = excluded.fetched_at
        """,
        (normalized, ts, float(rate), source, fetched),
    )
    return {
        "pair": normalized,
        "timestamp": ts,
        "rate": float(rate),
        "source": source,
        "fetched_at": fetched,
    }


def get_latest_rate(conn, pair):
    normalized = _normalize_rate_pair(pair)
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, source, fetched_at
        FROM rates_cache
        WHERE pair = ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC
        LIMIT 1
        """,
        (normalized,),
    ).fetchone()
    if not row:
        raise AppError(
            f"No cached rate for pair '{normalized}'",
            code="not_found",
            hint="Run `kassiber rates sync` first",
        )
    return {
        "pair": row["pair"],
        "timestamp": row["timestamp"],
        "rate": row["rate"],
        "source": row["source"],
        "fetched_at": row["fetched_at"],
    }


def get_rate_range(conn, pair, start=None, end=None, limit=None):
    normalized = _normalize_rate_pair(pair)
    sql = "SELECT pair, timestamp, rate, source, fetched_at FROM rates_cache WHERE pair = ?"
    params = [normalized]
    if start:
        start_dt = _parse_iso_datetime(start, "start")
        sql += " AND timestamp >= ?"
        params.append(_iso_z(start_dt))
    if end:
        end_dt = _parse_iso_datetime(end, "end")
        sql += " AND timestamp <= ?"
        params.append(_iso_z(end_dt))
    sql += " ORDER BY timestamp ASC"
    if limit:
        sql += " LIMIT ?"
        params.append(int(limit))
    rows = conn.execute(sql, params).fetchall()
    return [
        {
            "pair": r["pair"],
            "timestamp": r["timestamp"],
            "rate": r["rate"],
            "source": r["source"],
            "fetched_at": r["fetched_at"],
        }
        for r in rows
    ]


def get_cached_rate_at_or_before(conn, pair, occurred_at):
    normalized = _require_supported_pair(pair)
    occurred_ts = _iso_z(_parse_iso_datetime(occurred_at, "occurred_at"))
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, source, fetched_at
        FROM rates_cache
        WHERE pair = ? AND timestamp <= ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC
        LIMIT 1
        """,
        (normalized, occurred_ts),
    ).fetchone()
    if not row:
        return None
    return {
        "pair": row["pair"],
        "timestamp": row["timestamp"],
        "rate": row["rate"],
        "source": row["source"],
        "fetched_at": row["fetched_at"],
    }


def list_cached_pairs(conn):
    rows = conn.execute(
        """
        SELECT pair,
               COUNT(*) AS sample_count,
               MIN(timestamp) AS first_timestamp,
               MAX(timestamp) AS last_timestamp
        FROM rates_cache
        GROUP BY pair
        ORDER BY pair ASC
        """
    ).fetchall()
    known = {p: None for p in SUPPORTED_RATE_PAIRS}
    for row in rows:
        known[row["pair"]] = {
            "sample_count": int(row["sample_count"]),
            "first_timestamp": row["first_timestamp"],
            "last_timestamp": row["last_timestamp"],
        }
    result = []
    for pair in SUPPORTED_RATE_PAIRS:
        detail = known.get(pair)
        result.append(
            {
                "pair": pair,
                "supported": True,
                "cached": detail is not None,
                "sample_count": detail["sample_count"] if detail else 0,
                "first_timestamp": detail["first_timestamp"] if detail else None,
                "last_timestamp": detail["last_timestamp"] if detail else None,
            }
        )
    # Report any non-canonical pairs cached from manual `rates set`.
    for pair, detail in known.items():
        if pair in SUPPORTED_RATE_PAIRS:
            continue
        if detail is None:
            continue
        result.append(
            {
                "pair": pair,
                "supported": False,
                "cached": True,
                "sample_count": detail["sample_count"],
                "first_timestamp": detail["first_timestamp"],
                "last_timestamp": detail["last_timestamp"],
            }
        )
    return result


def _coingecko_market_chart(coin_id, vs, days):
    url = (
        "https://api.coingecko.com/api/v3/coins/"
        f"{coin_id}/market_chart?vs_currency={vs}&days={int(days)}"
    )
    payload = http_get_json(url, timeout=30)
    prices = payload.get("prices") if isinstance(payload, dict) else None
    if not isinstance(prices, list):
        raise AppError(
            "CoinGecko response did not contain a prices array",
            code="upstream_error",
            retryable=True,
        )
    out = []
    for entry in prices:
        if not isinstance(entry, list) or len(entry) < 2:
            continue
        ms, value = entry[0], entry[1]
        try:
            ts = datetime.fromtimestamp(float(ms) / 1000.0, tz=timezone.utc)
            rate = float(value)
        except (TypeError, ValueError):
            continue
        out.append((_iso_z(ts.replace(microsecond=0)), rate))
    return out


def fetch_rates_coingecko(pair, days=30):
    normalized = _require_supported_pair(pair)
    asset, fiat = _rate_pair_parts(normalized)
    coin_id = _COINGECKO_COIN.get(asset)
    vs = _COINGECKO_VS.get(fiat)
    if not coin_id or not vs:
        raise AppError(
            f"Pair '{normalized}' has no CoinGecko mapping",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    return _coingecko_market_chart(coin_id, vs, days)


def sync_rates(conn, pair=None, days=30, source="coingecko"):
    if source != "coingecko":
        raise AppError(
            f"Unknown rate source '{source}'",
            code="validation",
            hint="Supported sources: coingecko",
        )
    if pair:
        pairs = [_require_supported_pair(pair)]
    else:
        pairs = list(SUPPORTED_RATE_PAIRS)
    fetched_at = _iso_z(datetime.now(timezone.utc))
    summary = []
    for p in pairs:
        samples = fetch_rates_coingecko(p, days=days)
        inserted = 0
        for ts, rate in samples:
            upsert_rate(conn, p, ts, rate, source, fetched_at=fetched_at)
            inserted += 1
        conn.commit()
        summary.append(
            {
                "pair": p,
                "source": source,
                "samples": inserted,
                "days": int(days),
                "fetched_at": fetched_at,
            }
        )
    return summary


def set_manual_rate(conn, pair, timestamp, rate, source="manual"):
    normalized = _normalize_rate_pair(pair)
    try:
        value = float(rate)
    except (TypeError, ValueError) as exc:
        raise AppError(f"Invalid rate '{rate}'", code="validation") from exc
    if value <= 0:
        raise AppError("Rate must be positive", code="validation")
    row = upsert_rate(conn, normalized, timestamp, value, source)
    conn.commit()
    return row


def auto_price_transactions_from_rates_cache(conn, profile):
    tx_rows = conn.execute(
        """
        SELECT id, occurred_at, asset, amount, fiat_currency, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND fiat_rate IS NULL AND fiat_value IS NULL
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"],),
    ).fetchall()
    auto_priced = 0
    for row in tx_rows:
        pair = _transaction_rate_pair(row["asset"], row["fiat_currency"] or profile["fiat_currency"])
        if pair is None:
            continue
        cached_rate = get_cached_rate_at_or_before(conn, pair, row["occurred_at"])
        if cached_rate is None:
            continue
        rate = dec(cached_rate["rate"])
        fiat_value = rate * msat_to_btc(row["amount"]) if row["amount"] > 0 else None
        conn.execute(
            "UPDATE transactions SET fiat_rate = ?, fiat_value = ? WHERE id = ?",
            (float(rate), float(fiat_value) if fiat_value is not None else None, row["id"]),
        )
        auto_priced += 1
    return auto_priced


def build_ledger_state(conn, profile):
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
        "SELECT * FROM transaction_pairs WHERE profile_id = ?",
        (profile["id"],),
    ).fetchall()
    if not rows:
        return {
            "entries": [],
            "quarantines": [],
            "intra_audit": [],
            "cross_asset_pairs": [],
            "account_holdings": defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")}),
            "wallet_holdings": defaultdict(lambda: {"quantity": Decimal("0"), "cost_basis": Decimal("0")}),
            "latest_rates": latest_rates_for_profile(conn, profile["id"]),
        }
    tax_engine = build_tax_engine(profile)
    rates = latest_rates_for_profile(conn, profile["id"])
    wallet_refs_by_id = {}
    for row in rows:
        wallet_config = json.loads(row["config_json"] or "{}")
        wallet_refs_by_id[row["wallet_id"]] = {
            "id": row["wallet_id"],
            "label": row["wallet_label"],
            "wallet_account_id": row["wallet_account_id"],
            "account_code": row["account_code"],
            "account_label": row["account_label"],
            "altbestand": wallet_config.get("altbestand", False),
        }
    engine_state = tax_engine.build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=manual_pair_records,
        )
    )
    return {
        "entries": engine_state.entries,
        "quarantines": engine_state.quarantines,
        "intra_audit": engine_state.intra_audit,
        "cross_asset_pairs": engine_state.cross_asset_pairs,
        "account_holdings": engine_state.account_holdings,
        "wallet_holdings": engine_state.wallet_holdings,
        "latest_rates": rates,
    }


def process_journals(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    auto_priced = auto_price_transactions_from_rates_cache(conn, profile)
    state = build_ledger_state(conn, profile)
    conn.execute("DELETE FROM journal_entries WHERE profile_id = ?", (profile["id"],))
    conn.execute("DELETE FROM journal_quarantines WHERE profile_id = ?", (profile["id"],))
    created_at = now_iso()
    for entry in state["entries"]:
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
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
                entry["description"],
                created_at,
            ),
        )
    for quarantine in state["quarantines"]:
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                quarantine["transaction_id"],
                quarantine["workspace_id"],
                quarantine["profile_id"],
                quarantine["reason"],
                quarantine["detail_json"],
                created_at,
            ),
        )
    tx_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    conn.execute(
        "UPDATE profiles SET last_processed_at = ?, last_processed_tx_count = ? WHERE id = ?",
        (created_at, tx_count, profile["id"]),
    )
    conn.commit()
    return {
        "profile": profile["label"],
        "entries_created": len(state["entries"]),
        "quarantined": len(state["quarantines"]),
        "transfers_detected": len(state.get("intra_audit", [])),
        "cross_asset_pairs": len(state.get("cross_asset_pairs", [])),
        "auto_priced": auto_priced,
        "processed_transactions": tx_count,
        "processed_at": created_at,
    }


DEFAULT_EVENTS_LIMIT = 100
MAX_EVENTS_LIMIT = 1000


def _encode_event_cursor(row):
    token = f"{row['occurred_at']}|{row['created_at']}|{row['id']}"
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_event_cursor(cursor):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        occurred_at, created_at, event_id = decoded.split("|", 2)
        return {"occurred_at": occurred_at, "created_at": created_at, "id": event_id}
    except (ValueError, UnicodeDecodeError, binascii.Error) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it.",
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
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
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

    if wallet:
        wallet_row = resolve_wallet(conn, profile["id"], wallet)
        where.append("je.wallet_id = ?")
        params.append(wallet_row["id"])
    if account:
        account_row = resolve_account(conn, profile["id"], account)
        where.append("je.account_id = ?")
        params.append(account_row["id"])
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

    cursor_data = _decode_event_cursor(cursor)
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
            COALESCE(je.description, '') AS description
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
        events.append(event)
    next_cursor = _encode_event_cursor(page[-1]) if has_more and page else None

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


def list_journal_entries(conn, workspace_ref, profile_ref, limit=200):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            je.id,
            je.occurred_at,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at DESC, je.created_at DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    results = []
    for row in rows:
        entry = dict(row)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        results.append(entry)
    return results


def list_quarantines(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            t.external_id,
            t.occurred_at,
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
               w.label AS wallet, t.external_id, t.occurred_at, t.asset,
               t.amount, t.fee, t.fiat_rate, t.fiat_value, t.direction, t.excluded
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
        "direction": row["direction"],
        "asset": row["asset"],
        "amount": float(msat_to_btc(row["amount"])),
        "amount_msat": int(row["amount"]),
        "fee": float(msat_to_btc(row["fee"])),
        "fee_msat": int(row["fee"]),
        "fiat_rate": row["fiat_rate"],
        "fiat_value": row["fiat_value"],
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
    conn.execute(
        "UPDATE transactions SET fiat_rate = ?, fiat_value = ? WHERE id = ?",
        (
            float(new_rate) if new_rate is not None else None,
            float(new_value) if new_value is not None else None,
            tx["id"],
        ),
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "price-override",
        "fiat_rate": float(new_rate) if new_rate is not None else None,
        "fiat_value": float(new_value) if new_value is not None else None,
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def resolve_quarantine_exclude(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    conn.execute(
        "UPDATE transactions SET excluded = 1 WHERE id = ?",
        (tx["id"],),
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "exclude",
        "excluded": True,
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
    current_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    if not profile["last_processed_at"] or current_count != profile["last_processed_tx_count"]:
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
        "last_processed_tx_count": profile["last_processed_tx_count"],
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
    if new_country is not None and new_country not in supported_tax_countries():
        raise AppError(
            f"Unsupported tax country '{new_country}'",
            code="validation",
            hint=f"Choose one of: {', '.join(sorted(supported_tax_countries()))}",
        )
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
        conn.commit()
    elif args.profile:
        workspace = resolve_workspace(conn)
        profile = resolve_profile(conn, workspace["id"], args.profile)
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    else:
        raise AppError("Provide --workspace and/or --profile")
    cmd_context_show(conn, args)
