from __future__ import annotations

import json
import os
import sqlite3
import uuid
from pathlib import Path

from ..errors import AppError
from ..time_utils import now_iso
from ..time_utils import parse_timestamp
from ..util import normalize_chain_value, normalize_network_value, parse_bool, str_or_none
from ..wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    MAX_DESCRIPTOR_GAP_LIMIT,
    default_policy_asset_id,
    load_descriptor_plan,
    liquid_plan_can_unblind,
    normalize_asset_code,
)
from . import silent_payments
from ..wallet_setup import (
    BSMS_DESCRIPTOR_SOURCE,
    normalize_script_types,
    normalize_wallet_material,
    parse_bsms_descriptor_record,
)
from . import freshness as core_freshness
from . import output_inventory as core_output_inventory
from .address_scripts import scriptpubkey_for_address_or_none
from .chain_observer import delete_wallet_observer_state
from .ownership_policy_epochs import roll_wallet_policy_epoch
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
    "silent-payment",
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
    "ledgerlive",
    "kraken",
    "coinbase",
    "binance",
    "wasabi",
    "samourai",
    "untracked",
    "custom",
]
REDACTED_CONFIG_VALUE = "[redacted]"
BTCPAY_SYNC_SOURCE = "btcpay"
BTCPAY_DEFAULT_PAYMENT_METHOD_ID = "BTC-CHAIN"
BTCPAY_PROVENANCE_CONFIG_KEY = "btcpay_provenance"
BULLBITCOIN_WALLET_NETWORK_CONFIG_KEY = "bullbitcoin_wallet_network"
BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY = "bullbitcoin_wallet_exports"
BULLBITCOIN_WALLET_NETWORKS = ("bitcoin", "liquid", "lightning")
WALLET_DEPRECATED_CONFIG_KEY = "deprecated"
OWNERSHIP_HISTORY_CONFIG_KEY = "ownership_history"
OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY = "ownership_scan_to_index"
MAX_OWNERSHIP_SCAN_TO_INDEX = 20_000
_OWNERSHIP_MATERIAL_FIELDS = (
    "descriptor",
    "change_descriptor",
    "xpub",
    "script_types",
    OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY,
    "addresses",
    "chain",
    "network",
    "gap_limit",
    "synthesize_change",
)
WALLET_SAFE_CONFIG_FIELDS = (
    "addresses",
    "backend",
    "chain",
    "network",
    "gap_limit",
    "policy_asset",
    "sync_source",
    "store_id",
    "payment_method_id",
    BTCPAY_PROVENANCE_CONFIG_KEY,
    BULLBITCOIN_WALLET_NETWORK_CONFIG_KEY,
    BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY,
    "source_file",
    "source_format",
    "altbestand",
    "wasabi_metadata",
    "samourai",
    "descriptor_source",
    OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY,
    "synthesize_change",
    "script_types",
    *silent_payments.SAFE_CONFIG_FIELDS,
    WALLET_DEPRECATED_CONFIG_KEY,
)
# The xpub is as sensitive as the descriptor it expands into (it reveals every
# address), so it is redacted; its presence still tells the UI the wallet is
# xpub-derived, and script_types (the watched set) is surfaced for editing.
WALLET_REDACTED_CONFIG_FIELDS = (
    "descriptor",
    "change_descriptor",
    "xpub",
    *silent_payments.REDACTED_CONFIG_FIELDS,
)


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
        if not address:
            continue
        script_pubkey = scriptpubkey_for_address_or_none(address)
        key = f"script:{script_pubkey}" if script_pubkey else f"text:{address}"
        if key in seen:
            continue
        seen.add(key)
        output.append(address)
    return output


def redact_wallet_config_for_output(value):
    if not isinstance(value, dict):
        return {}
    safe = {}
    if "addresses" in value:
        safe["addresses"] = normalize_addresses(value.get("addresses"))
    for field in WALLET_SAFE_CONFIG_FIELDS:
        if field == "addresses" or field not in value:
            continue
        safe[field] = value[field]
    for field in WALLET_REDACTED_CONFIG_FIELDS:
        if field in value:
            safe[field] = REDACTED_CONFIG_VALUE
    return safe


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
            config.get("xpub"),
            config.get("addresses"),
            config.get(silent_payments.CONFIG_DESCRIPTOR),
            config.get("chain"),
            config.get("network"),
        ]
    ):
        return None, None
    chain = normalize_chain_value(config.get("chain"))
    network = normalize_network_value(chain, config.get("network"))
    return chain, network


def has_descriptor_sync_material(config):
    """True when a wallet config carries on-chain derivation material.

    Either an explicit output ``descriptor`` or a bare ``xpub`` with at least one
    enabled ``script_types`` entry (the multi-script wallet shape). Sync,
    freshness, and snapshot classification all key off this rather than the raw
    ``descriptor`` field so xpub-derived wallets are treated as syncable.
    """
    if not isinstance(config, dict):
        return False
    if str_or_none(config.get("descriptor")):
        return True
    return bool(str_or_none(config.get("xpub")) and config.get("script_types"))


def has_silent_payment_sync_material(config):
    return silent_payments.has_silent_payment_sync_material(config)


def wallet_is_deprecated(config):
    if not isinstance(config, dict):
        return False
    return parse_bool(config.get(WALLET_DEPRECATED_CONFIG_KEY), default=False)


def _reset_onchain_freshness_checkpoint(conn, profile_id: str, wallet_id: str) -> None:
    core_freshness.reset_source_checkpoint(
        conn,
        profile_id,
        core_freshness.source_key(core_freshness.SOURCE_ONCHAIN, wallet_id),
        stale_reason="wallet_config_changed",
    )


def load_wallet_descriptor_plan_from_config(config):
    try:
        return load_descriptor_plan(config)
    except ValueError as exc:
        raise AppError(str(exc)) from exc


def wallet_policy_asset_id(config, chain, network):
    explicit = str_or_none(config.get("policy_asset"))
    if explicit:
        normalized = normalize_asset_code(explicit)
        # Liquid sync compares the hex asset id from each output against this
        # value, so resolve a symbolic LBTC to the network's hex policy asset.
        if normalized == "LBTC" and chain == "liquid":
            resolved = default_policy_asset_id(network)
            if resolved:
                return normalize_asset_code(resolved)
        return normalized
    if chain == "liquid":
        return normalize_asset_code(default_policy_asset_id(network))
    return ""


def wallet_btcpay_sync_config(config):
    if not isinstance(config, dict):
        return None
    sync_source = str_or_none(config.get("sync_source"))
    if sync_source is None:
        return None
    store_id = str_or_none(config.get("store_id"))
    payment_method_id = str_or_none(config.get("payment_method_id"))
    normalized_source = sync_source.strip().lower()
    if normalized_source != BTCPAY_SYNC_SOURCE:
        raise AppError(
            f"Unsupported source refresh type '{normalized_source}'",
            code="validation",
            hint=f"Supported refresh sources: {BTCPAY_SYNC_SOURCE}",
        )
    backend = str_or_none(config.get("backend"))
    if backend is None:
        raise AppError(
            "BTCPay-backed wallets require a named --backend",
            code="validation",
            hint="Set --backend to a btcpay backend before refreshing this source.",
        )
    if store_id is None:
        raise AppError(
            "BTCPay-backed wallets require --store-id",
            code="validation",
        )
    return {
        "sync_source": BTCPAY_SYNC_SOURCE,
        "backend": backend.lower(),
        "store_id": store_id,
        "payment_method_id": payment_method_id or BTCPAY_DEFAULT_PAYMENT_METHOD_ID,
    }


def wallet_btcpay_provenance_config(config):
    if not isinstance(config, dict):
        return []
    raw_routes = config.get(BTCPAY_PROVENANCE_CONFIG_KEY)
    if raw_routes is None:
        return []
    if not isinstance(raw_routes, list):
        raise AppError(
            "BTCPay provenance config must be a list",
            code="validation",
        )
    routes = []
    seen = set()
    for raw_route in raw_routes:
        if not isinstance(raw_route, dict):
            raise AppError(
                "BTCPay provenance routes must be objects",
                code="validation",
            )
        backend = str_or_none(raw_route.get("backend"))
        store_id = str_or_none(raw_route.get("store_id"))
        payment_method_id = str_or_none(raw_route.get("payment_method_id"))
        if backend is None or store_id is None:
            raise AppError(
                "BTCPay provenance routes require backend and store_id",
                code="validation",
            )
        route = {
            "backend": backend.lower(),
            "store_id": store_id,
            "payment_method_id": normalize_btcpay_payment_method_id(
                payment_method_id or BTCPAY_DEFAULT_PAYMENT_METHOD_ID
            ),
        }
        key = (route["backend"], route["store_id"], route["payment_method_id"])
        if key not in seen:
            routes.append(route)
            seen.add(key)
    return routes


def normalize_bullbitcoin_wallet_network(value):
    network = str_or_none(value)
    if network is None:
        raise AppError("Bull Bitcoin wallet network cannot be empty", code="validation")
    normalized = network.strip().lower()
    aliases = {
        "btc": "bitcoin",
        "onchain": "bitcoin",
        "chain": "bitcoin",
        "lbtc": "liquid",
        "liquidv1": "liquid",
        "ln": "lightning",
    }
    normalized = aliases.get(normalized, normalized)
    if normalized not in BULLBITCOIN_WALLET_NETWORKS:
        raise AppError(
            f"Unsupported Bull Bitcoin wallet network '{value}'",
            code="validation",
            hint="Supported networks: bitcoin, liquid, lightning.",
        )
    return normalized


def wallet_bullbitcoin_wallet_network(config):
    if not isinstance(config, dict):
        return None
    value = config.get(BULLBITCOIN_WALLET_NETWORK_CONFIG_KEY)
    if value in (None, ""):
        return None
    return normalize_bullbitcoin_wallet_network(value)


def wallet_bullbitcoin_wallet_export_config(config):
    if not isinstance(config, dict):
        return []
    raw_routes = config.get(BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY)
    if raw_routes is None:
        return []
    if not isinstance(raw_routes, list):
        raise AppError(
            "Bull Bitcoin wallet export config must be a list",
            code="validation",
        )
    routes = []
    seen = set()
    for raw_route in raw_routes:
        if not isinstance(raw_route, dict):
            raise AppError(
                "Bull Bitcoin wallet export routes must be objects",
                code="validation",
            )
        source_file = str_or_none(raw_route.get("source_file"))
        if source_file is None:
            raise AppError(
                "Bull Bitcoin wallet export routes require source_file",
                code="validation",
            )
        route = {
            "source_file": os.path.abspath(os.path.expanduser(source_file)),
            "network": normalize_bullbitcoin_wallet_network(raw_route.get("network")),
        }
        key = (route["source_file"], route["network"])
        if key not in seen:
            routes.append(route)
            seen.add(key)
    return routes


def normalize_btcpay_store_id(value):
    store_id = str_or_none(value)
    if store_id is None:
        raise AppError("BTCPay store id cannot be empty", code="validation")
    return store_id


def normalize_btcpay_payment_method_id(value):
    payment_method_id = str_or_none(value)
    if payment_method_id is None:
        raise AppError("BTCPay payment method id cannot be empty", code="validation")
    # BTCPay treats payment method ids as "{CRYPTO}-{TYPE}" — e.g. BTC-CHAIN,
    # LBTC-CHAIN, BTC-LN. Canonicalize to upper case here so wallet config,
    # sync URLs, and the allowlist gate all agree regardless of how the
    # caller typed it.
    return payment_method_id.upper()


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    sp_descriptor_text = read_text_argument(
        getattr(args, "sp_descriptor", None),
        getattr(args, "sp_descriptor_file", None),
        "Silent Payments descriptor",
    )
    if sp_descriptor_text:
        config[silent_payments.CONFIG_DESCRIPTOR] = sp_descriptor_text
    descriptor_text = read_text_argument(
        getattr(args, "descriptor", None),
        getattr(args, "descriptor_file", None),
        "Descriptor",
    )
    change_descriptor_text = read_text_argument(
        getattr(args, "change_descriptor", None),
        getattr(args, "change_descriptor_file", None),
        "Change descriptor",
    )
    if descriptor_text:
        bsms_descriptors = parse_bsms_descriptor_record(descriptor_text)
        if bsms_descriptors:
            descriptor_text = bsms_descriptors["descriptor"]
            config["descriptor_source"] = BSMS_DESCRIPTOR_SOURCE
            config["synthesize_change"] = False
            if not change_descriptor_text:
                change_descriptor_text = bsms_descriptors.get("change_descriptor")
    if descriptor_text:
        config["descriptor"] = descriptor_text
    if change_descriptor_text:
        config["change_descriptor"] = change_descriptor_text
    script_types = normalize_script_types(getattr(args, "script_type", None))
    if script_types:
        if not descriptor_text:
            raise AppError(
                "--script-type requires a bare xpub via "
                "--descriptor/--descriptor-file/--descriptor-stdin",
                code="validation",
            )
        material_config = normalize_wallet_material(descriptor_text, script_types=script_types)
        if "xpub" in material_config:
            # A bare xpub + script types becomes a multi-script wallet: store the
            # key and the watched set, not a single rendered descriptor.
            config.pop("descriptor", None)
            config.pop("change_descriptor", None)
            config["xpub"] = material_config["xpub"]
            config["script_types"] = material_config["script_types"]
    addresses = normalize_addresses(getattr(args, "address", None))
    existing_addresses = normalize_addresses(config.get("addresses"))
    if addresses or existing_addresses:
        config["addresses"] = normalize_addresses(existing_addresses + addresses)
    if getattr(args, "chain", None):
        config["chain"] = normalize_chain_value(args.chain)
    if getattr(args, "network", None):
        chain = normalize_chain_value(config.get("chain"))
        config["network"] = normalize_network_value(chain, args.network)
    if getattr(args, "sp_scan_mode", None):
        config[silent_payments.CONFIG_SCAN_MODE] = args.sp_scan_mode
    if getattr(args, "sp_scan_start_height", None) is not None:
        config[silent_payments.CONFIG_SCAN_START_HEIGHT] = args.sp_scan_start_height
    if getattr(args, "sp_scan_start_date", None):
        config[silent_payments.CONFIG_SCAN_START_DATE] = args.sp_scan_start_date
    if getattr(args, "sp_full_history", False):
        config[silent_payments.CONFIG_FULL_HISTORY] = True
    if getattr(args, "sp_acknowledge_full_history_warning", False):
        config[silent_payments.CONFIG_FULL_HISTORY_ACK] = True
    if getattr(args, "sp_acknowledge_server_warning", False):
        config[silent_payments.CONFIG_SERVER_WARNING_ACK] = True
    if getattr(args, "gap_limit", None) is not None:
        if args.gap_limit <= 0:
            raise AppError("Descriptor gap limit must be positive")
        if args.gap_limit > MAX_DESCRIPTOR_GAP_LIMIT:
            raise AppError(
                f"Descriptor gap limit must be {MAX_DESCRIPTOR_GAP_LIMIT} or lower"
            )
        config["gap_limit"] = args.gap_limit
    if getattr(args, "birthday", None) is not None:
        config["birthday"] = args.birthday
    if getattr(args, "policy_asset", None):
        config["policy_asset"] = normalize_asset_code(args.policy_asset)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    has_btcpay_flag = False
    if getattr(args, "store_id", None) is not None:
        config["store_id"] = normalize_btcpay_store_id(args.store_id)
        has_btcpay_flag = True
    if getattr(args, "payment_method_id", None) is not None:
        config["payment_method_id"] = normalize_btcpay_payment_method_id(
            args.payment_method_id
        )
        has_btcpay_flag = True
    if has_btcpay_flag:
        config["sync_source"] = BTCPAY_SYNC_SOURCE
    bull_network = wallet_bullbitcoin_wallet_network(config)
    if bull_network:
        config[BULLBITCOIN_WALLET_NETWORK_CONFIG_KEY] = bull_network
    bull_routes = wallet_bullbitcoin_wallet_export_config(config)
    if bull_routes or BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY in config:
        config[BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY] = bull_routes
    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network
    btcpay_config = wallet_btcpay_sync_config(config)
    if btcpay_config:
        config.update(btcpay_config)
    btcpay_routes = wallet_btcpay_provenance_config(config)
    if btcpay_routes or BTCPAY_PROVENANCE_CONFIG_KEY in config:
        config[BTCPAY_PROVENANCE_CONFIG_KEY] = btcpay_routes
    return config


def create_wallet(
    conn,
    workspace_ref,
    profile_ref,
    label,
    kind,
    account_ref=None,
    config=None,
    *,
    commit=True,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    account = resolve_account(conn, profile["id"], account_ref or "treasury")
    normalized_kind = normalize_wallet_kind(kind)
    config = _validated_wallet_config(normalized_kind, config or {})
    wallet_id = str(uuid.uuid4())
    try:
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
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"Wallet '{label}' already exists in profile '{profile['label']}'",
            code="conflict",
            hint="Choose a different wallet label or update the existing wallet.",
        ) from exc
    if commit:
        conn.commit()
    created = fetch_wallet_with_account(conn, wallet_id)
    return wallet_row_to_dict(created)


def _validated_wallet_config(normalized_kind, config):
    config = dict(config or {})
    if OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY in config:
        try:
            ownership_scan_to_index = int(
                config[OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY]
            )
        except (TypeError, ValueError) as exc:
            raise AppError(
                "ownership_scan_to_index must be an integer",
                code="validation",
            ) from exc
        if not 0 <= ownership_scan_to_index <= MAX_OWNERSHIP_SCAN_TO_INDEX:
            raise AppError(
                f"ownership_scan_to_index must be between 0 and "
                f"{MAX_OWNERSHIP_SCAN_TO_INDEX}",
                code="validation",
            )
        config[OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY] = ownership_scan_to_index
    if "addresses" in config:
        config["addresses"] = normalize_addresses(config.get("addresses"))
    if "birthday" in config:
        birthday = str_or_none(config.get("birthday"))
        if birthday is None:
            config.pop("birthday", None)
        else:
            config["birthday"] = parse_timestamp(birthday)
    if WALLET_DEPRECATED_CONFIG_KEY in config:
        config[WALLET_DEPRECATED_CONFIG_KEY] = wallet_is_deprecated(config)
    if normalized_kind == silent_payments.WALLET_KIND:
        return silent_payments.validate_wallet_config(config)
    has_live_material = bool(config.get("descriptor") or config.get("xpub"))
    descriptor_plan = load_wallet_descriptor_plan_from_config(config) if has_live_material else None
    chain, network = wallet_live_chain_config(config)
    if normalized_kind == "address" and not config.get("addresses") and not config.get("source_file"):
        raise AppError(
            "Address wallets require at least one --address or a file-based source",
            code="validation",
        )
    if normalized_kind == "descriptor" and descriptor_plan is None and not config.get("source_file"):
        raise AppError(
            "Descriptor wallets require a descriptor, an xpub with script types, or a file-based source",
            code="validation",
        )
    if normalized_kind == "coreln" and not config.get("backend"):
        raise AppError(
            "Core Lightning wallets require a --backend for live read-only sync",
            code="validation",
        )
    if chain == "liquid" and descriptor_plan is None and not config.get("source_file"):
        raise AppError(
            "Liquid live refresh currently requires a descriptor with private blinding keys",
            code="validation",
        )
    if descriptor_plan and descriptor_plan.chain == "liquid":
        if not liquid_plan_can_unblind(descriptor_plan):
            raise AppError(
                "Liquid descriptor wallets require private blinding keys for full sync and fee accounting",
                code="validation",
            )
        if not config.get("backend") and not config.get("source_file"):
            raise AppError(
                "Liquid descriptor wallets require an explicit --backend; no public Liquid default is built in",
                code="validation",
            )
        config["policy_asset"] = wallet_policy_asset_id(config, descriptor_plan.chain, descriptor_plan.network)
    elif chain == "liquid" and not config.get("backend") and not config.get("source_file"):
        raise AppError(
            "Liquid wallets require an explicit --backend; no public Liquid default is built in",
            code="validation",
        )
    if chain and network:
        config["chain"] = chain
        config["network"] = network
    bull_network = wallet_bullbitcoin_wallet_network(config)
    if bull_network:
        config[BULLBITCOIN_WALLET_NETWORK_CONFIG_KEY] = bull_network
    bull_routes = wallet_bullbitcoin_wallet_export_config(config)
    if bull_routes or BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY in config:
        config[BULLBITCOIN_WALLET_EXPORTS_CONFIG_KEY] = bull_routes
    btcpay_config = wallet_btcpay_sync_config(config)
    if btcpay_config:
        config.update(btcpay_config)
    return config


def _sync_material_config_json(config):
    sync_config = dict(config or {})
    sync_config.pop(WALLET_DEPRECATED_CONFIG_KEY, None)
    sync_config.pop(OWNERSHIP_HISTORY_CONFIG_KEY, None)
    sync_config.pop(OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY, None)
    return json.dumps(sync_config, sort_keys=True)


def _ownership_material_snapshot(config):
    """The minimum private config needed to recognize historic scripts.

    This stays inside the encrypted wallet config. It is intentionally absent
    from ``WALLET_SAFE_CONFIG_FIELDS`` and therefore never appears in normal
    CLI, daemon, UI, or AI wallet payloads.
    """

    if not isinstance(config, dict):
        return {}
    return {
        field: config[field]
        for field in _OWNERSHIP_MATERIAL_FIELDS
        if config.get(field) not in (None, "", [])
    }


def _ownership_material_identity_snapshot(config):
    """Return script-policy identity without mutable coverage bookkeeping.

    Coverage declarations, scan depth, and gap limits describe how thoroughly
    existing material was searched. Updating them must not manufacture a
    retired policy epoch or clear the wallet's synced inventory.
    """

    snapshot = _ownership_material_snapshot(config)
    for field in (
        OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY,
        "gap_limit",
    ):
        snapshot.pop(field, None)
    return snapshot


def _wallet_descriptor_state(config):
    descriptor_state = ""
    chain, network = wallet_live_chain_config(config)
    if config.get("descriptor") or config.get("xpub"):
        try:
            descriptor_plan = load_descriptor_plan(config)
            descriptor_state = f"{descriptor_plan.chain}:{descriptor_plan.network}"
            chain = descriptor_plan.chain
            network = descriptor_plan.network
        except ValueError:
            descriptor_state = "invalid"
        except AppError as exc:
            if exc.code != "dependency_missing":
                raise
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
        config.pop("altbestand", None)
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
                "silent_payment": (
                    "configured" if has_silent_payment_sync_material(config) else ""
                ),
                "gap_limit": config.get("gap_limit", DEFAULT_DESCRIPTOR_GAP_LIMIT if descriptor_state else ""),
                "source_format": config.get("source_format", ""),
                "source_file": config.get("source_file", ""),
                "deprecated": wallet_is_deprecated(config),
                "created_at": row["created_at"],
            }
        )
    return output


WALLET_KIND_CATALOG = {
    "descriptor": {
        "summary": "Output-descriptor wallet with optional change branch; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "change_descriptor", "gap_limit", "backend", "chain", "network", "policy_asset"],
        "requires": ["descriptor"],
    },
    "xpub": {
        "summary": "Extended-public-key wallet: derives one or more script types (pinned with --script-type) to an address set; on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "xpub", "script_types", "gap_limit", "backend", "chain", "network"],
        "requires": ["descriptor|script_types"],
    },
    "address": {
        "summary": "Bare-address list wallet; useful for receive-only tracking or imports.",
        "config_fields": ["addresses", "backend", "chain", "network", "source_file", "source_format"],
        "requires": ["addresses|source_file"],
    },
    "untracked": {
        "summary": "Owned historical custody with no connected source; used only by reviewed custody components to bridge missing wallets or nodes.",
        "config_fields": [],
        "requires": [],
    },
    "silent-payment": {
        "summary": "Watch-only BIP352/BIP392 Silent Payments receive source; uses an explicit SP-capable backend or local scanner.",
        "config_fields": [
            "sp_descriptor",
            "sp_scan_start_height",
            "sp_scan_start_date",
            "sp_scan_mode",
            "backend",
            "chain",
            "network",
        ],
        "requires": ["sp_descriptor", "backend", "scan start"],
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
    "ledgerlive": {
        "summary": "Ledger Live CSV importer for BTC/LBTC wallet movement only.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "kraken": {
        "summary": "Kraken exchange import wallet for API or CSV execution evidence.",
        "config_fields": ["backend", "source_file", "source_format"],
        "requires": [],
    },
    "coinbase": {
        "summary": "Coinbase exchange import wallet for API execution and wallet movement evidence.",
        "config_fields": ["backend", "source_file", "source_format"],
        "requires": [],
    },
    "binance": {
        "summary": "Binance exchange import wallet for API rows and BTC supplemental CSVs.",
        "config_fields": ["backend", "source_file", "source_format"],
        "requires": [],
    },
    "wasabi": {
        "summary": "Wasabi Wallet sanitized RPC/export bundle importer with CoinJoin and anonymity evidence.",
        "config_fields": ["source_file", "source_format", "wasabi_metadata"],
        "requires": [],
    },
    "samourai": {
        "summary": "Logical Samourai/Whirlpool watch-only wallet group; child descriptor wallets carry sync targets.",
        "config_fields": ["backend", "chain", "network", "gap_limit", "samourai"],
        "requires": ["wallets import-samourai"],
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
    safe_config = redact_wallet_config_for_output(config)
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
        "silent_payment": {
            "configured": has_silent_payment_sync_material(config),
            "material_format": config.get(silent_payments.CONFIG_MATERIAL_FORMAT, ""),
            "scan_mode": config.get(silent_payments.CONFIG_SCAN_MODE, ""),
            "scan_start_height": config.get(silent_payments.CONFIG_SCAN_START_HEIGHT),
            "scan_start_date": config.get(silent_payments.CONFIG_SCAN_START_DATE, ""),
            "full_history": bool(config.get(silent_payments.CONFIG_FULL_HISTORY)),
        },
        "gap_limit": config.get("gap_limit"),
        "policy_asset": config.get("policy_asset"),
        "source_file": config.get("source_file", ""),
        "source_format": config.get("source_format", ""),
        "deprecated": wallet_is_deprecated(config),
        "config": safe_config,
        "created_at": row["created_at"],
    }


def get_wallet_details(conn, workspace_ref, profile_ref, wallet_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    return wallet_row_to_dict(wallet)


def wallet_descriptor_material(config):
    """Return the pasteable stored descriptor material for a wallet config."""
    descriptor = str_or_none(config.get("descriptor"))
    change_descriptor = str_or_none(config.get("change_descriptor"))
    sp_descriptor = str_or_none(config.get(silent_payments.CONFIG_DESCRIPTOR))
    return "\n".join(
        value for value in (descriptor, change_descriptor, sp_descriptor) if value
    )


def reveal_wallet_descriptor_material(conn, workspace_ref, profile_ref, wallet_ref):
    """Return only pasteable descriptor material for the desktop copy flow."""

    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"]) if wallet["config_json"] else {}
    return {
        "id": wallet["id"],
        "label": wallet["label"],
        "kind": wallet["kind"],
        "wallet_material": wallet_descriptor_material(config),
    }


def reveal_wallet_secrets(conn, workspace_ref, profile_ref, wallet_ref):
    """Return the raw descriptor / blinding-key material of a wallet.

    Bypasses `redact_wallet_config_for_output`. The DB must already be
    unlocked, which means the caller already supplied the SQLCipher
    passphrase for this invocation.
    """

    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"]) if wallet["config_json"] else {}
    return {
        "id": wallet["id"],
        "label": wallet["label"],
        "kind": wallet["kind"],
        "wallet_material": wallet_descriptor_material(config),
        "descriptor": config.get("descriptor"),
        "change_descriptor": config.get("change_descriptor"),
        "sp_descriptor": config.get(silent_payments.CONFIG_DESCRIPTOR),
        "blinding_key": config.get("blinding_key"),
        "addresses": config.get("addresses"),
        "config": config,
    }


def update_wallet(conn, workspace_ref, profile_ref, wallet_ref, updates):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    new_label = updates.get("label")
    new_account = updates.get("account")
    config_updates = updates.get("config") or {}
    clear_fields = updates.get("clear") or []

    if (
        new_label is None
        and new_account is None
        and not config_updates
        and not clear_fields
    ):
        raise AppError(
            "wallets update requires at least one field to change",
            code="validation",
            hint="Pass --label, --account, --config/--config-file, or --clear <field>",
        )

    label_value = new_label if new_label is not None else wallet["label"]
    account_id = wallet["account_id"]
    if new_account is not None:
        account = resolve_account(conn, profile["id"], new_account)
        account_id = account["id"]

    # Preserve legacy Austrian provenance metadata until a deliberate migration removes it.
    config = json.loads(wallet["config_json"] or "{}")
    original_ownership_material = _ownership_material_snapshot(config)
    original_ownership_identity = _ownership_material_identity_snapshot(config)
    original_sync_material_json = _sync_material_config_json(config)
    for field in clear_fields:
        if field not in config:
            raise AppError(
                f"Wallet config field '{field}' is not set",
                code="validation",
                hint="Use `wallets get` to inspect clearable config fields before clearing.",
            )
        config.pop(field, None)
    for key, value in config_updates.items():
        if value is None:
            config.pop(key, None)
        else:
            config[key] = value

    config = _validated_wallet_config(wallet["kind"], config)
    ownership_identity_changed = (
        _ownership_material_identity_snapshot(config) != original_ownership_identity
    )
    if ownership_identity_changed:
        # Retain retired material and its last technical coverage in a durable,
        # random-id policy epoch before disposable observer state is cleared.
        # This records only imported policy history, never an attestation that
        # every wallet owned by the profile has been supplied.
        roll_wallet_policy_epoch(
            conn,
            wallet,
            original_ownership_material,
            _ownership_material_snapshot(config),
        )
    config_json = json.dumps(config, sort_keys=True)
    sync_material_changed = (
        _sync_material_config_json(config) != original_sync_material_json
    )

    try:
        conn.execute(
            """
            UPDATE wallets
            SET label = ?, account_id = ?, config_json = ?
            WHERE id = ?
            """,
            (label_value, account_id, config_json, wallet["id"]),
        )
    except sqlite3.IntegrityError as exc:
        raise AppError(
            f"Wallet '{label_value}' already exists in profile '{profile['label']}'",
            code="conflict",
            hint="Choose a different wallet label.",
        ) from exc
    if sync_material_changed:
        delete_wallet_observer_state(conn, wallet["id"])
        core_output_inventory.clear_wallet_output_inventory(
            conn,
            wallet["id"],
            commit=False,
        )
        _reset_onchain_freshness_checkpoint(conn, profile["id"], wallet["id"])
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
    "BTCPAY_DEFAULT_PAYMENT_METHOD_ID",
    "BTCPAY_PROVENANCE_CONFIG_KEY",
    "BTCPAY_SYNC_SOURCE",
    "WALLET_KINDS",
    "WALLET_KIND_CATALOG",
    "create_wallet",
    "delete_wallet",
    "get_wallet_details",
    "list_wallet_kinds",
    "list_wallets",
    "load_wallet_descriptor_plan_from_config",
    "has_silent_payment_sync_material",
    "OWNERSHIP_HISTORY_CONFIG_KEY",
    "OWNERSHIP_SCAN_TO_INDEX_CONFIG_KEY",
    "normalize_addresses",
    "wallet_is_deprecated",
    "normalize_btcpay_payment_method_id",
    "normalize_btcpay_store_id",
    "normalize_wallet_kind",
    "parse_wallet_config",
    "read_text_argument",
    "update_wallet",
    "wallet_btcpay_provenance_config",
    "wallet_btcpay_sync_config",
    "wallet_descriptor_material",
    "reveal_wallet_descriptor_material",
    "wallet_live_chain_config",
    "wallet_policy_asset_id",
    "wallet_row_to_dict",
]
