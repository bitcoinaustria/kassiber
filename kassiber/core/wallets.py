from __future__ import annotations

import json
import os
import uuid
from pathlib import Path

from ..errors import AppError
from ..time_utils import now_iso
from ..util import normalize_chain_value, normalize_network_value, parse_bool, str_or_none
from ..wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    default_policy_asset_id,
    load_descriptor_plan,
    liquid_plan_can_unblind,
    normalize_asset_code,
)
from .repo import (
    fetch_wallet_with_account,
    invalidate_journals,
    resolve_account,
    resolve_scope,
    resolve_wallet,
    wallet_transaction_count,
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
    "custom",
]


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
    account = resolve_account(conn, profile["id"], account_ref or "treasury")
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


def _wallet_descriptor_state(config):
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
    return descriptor_state, chain, network


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
        descriptor_state, chain, network = _wallet_descriptor_state(config)
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
    return {"wallet": wallet["label"], "altbestand": bool(enabled)}


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


def wallet_row_to_dict(row):
    config = json.loads(row["config_json"] or "{}")
    descriptor_state, chain, network = _wallet_descriptor_state(config)
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
    return wallet_row_to_dict(wallet)


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
    updated = fetch_wallet_with_account(conn, wallet["id"])
    return wallet_row_to_dict(updated)


def delete_wallet(conn, workspace_ref, profile_ref, wallet_ref, cascade=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    tx_count = wallet_transaction_count(conn, wallet["id"])
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


__all__ = [
    "WALLET_KINDS",
    "WALLET_KIND_CATALOG",
    "create_wallet",
    "delete_wallet",
    "get_wallet_details",
    "list_wallet_kinds",
    "list_wallets",
    "load_wallet_descriptor_plan_from_config",
    "normalize_addresses",
    "normalize_wallet_kind",
    "parse_wallet_config",
    "read_text_argument",
    "set_wallet_altbestand",
    "update_wallet",
    "wallet_live_chain_config",
    "wallet_policy_asset_id",
    "wallet_row_to_dict",
]
