from __future__ import annotations

"""Wallet sync orchestration helpers that stay above backend adapter details."""

import json
import sqlite3
from dataclasses import dataclass
from typing import Any, Callable, Mapping, MutableMapping, Sequence

from ..errors import AppError
from ..util import str_or_none
from .wallets import wallet_btcpay_sync_config

WalletRow = Mapping[str, Any]
ProfileRow = Mapping[str, Any]
RuntimeConfig = Mapping[str, Any]
SyncOutcome = dict[str, Any]
BackendRecord = Mapping[str, Any]
SyncTarget = Mapping[str, Any]
HistoryEntry = Mapping[str, Any]
HistoryCache = MutableMapping[str, Sequence[HistoryEntry]]
ImportFile = Callable[[sqlite3.Connection, ProfileRow, WalletRow, str, str], SyncOutcome]
InsertRecords = Callable[[sqlite3.Connection, ProfileRow, WalletRow, Sequence[BackendRecord], str], SyncOutcome]
ResolveBackend = Callable[[RuntimeConfig, str | None], Mapping[str, Any]]
ResolveSyncState = Callable[[Mapping[str, Any], WalletRow], "WalletSyncState"]
NormalizeAddresses = Callable[[Any], Sequence[str]]
BackendAdapter = Callable[
    [Mapping[str, Any], WalletRow, "WalletSyncState"],
    tuple[Sequence[BackendRecord], Mapping[str, Any]],
]
SyncBTCPayWallet = Callable[
    [sqlite3.Connection, RuntimeConfig, ProfileRow, WalletRow],
    SyncOutcome,
]


@dataclass(frozen=True, slots=True)
class WalletSyncState:
    chain: str
    network: str
    descriptor_plan: Any | None
    policy_asset_id: str
    targets: Sequence[SyncTarget]
    tracked_scripts: Mapping[str, SyncTarget]
    history_cache: HistoryCache


@dataclass(frozen=True)
class WalletSyncHooks:
    import_file: ImportFile
    insert_records: InsertRecords
    resolve_backend: ResolveBackend
    resolve_sync_state: ResolveSyncState
    normalize_addresses: NormalizeAddresses
    backend_adapters: Mapping[str, BackendAdapter]
    sync_btcpay_wallet: SyncBTCPayWallet | None = None


def normalize_backend_kind(kind: Any) -> str:
    value = str(kind).strip().lower()
    aliases = {
        "bitcoin-core": "bitcoinrpc",
        "bitcoincore": "bitcoinrpc",
        "core": "bitcoinrpc",
        "liquid-esplora": "esplora",
    }
    return aliases.get(value, value)


def sync_wallet_from_backend(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
) -> SyncOutcome:
    config = json.loads(wallet["config_json"] or "{}")
    backend = hooks.resolve_backend(runtime_config, config.get("backend"))
    sync_state = hooks.resolve_sync_state(backend, wallet)
    if not sync_state.targets:
        return {
            "wallet": wallet["label"],
            "status": "skipped",
            "reason": "no addresses or descriptors configured for backend sync",
        }
    kind = normalize_backend_kind(backend["kind"])
    adapter = hooks.backend_adapters.get(kind)
    if adapter is None:
        raise AppError(
            f"Wallet sync is not implemented for backend kind '{kind}'",
            hint="Use an esplora, electrum, or bitcoinrpc backend for live sync.",
        )
    normalized_records, adapter_meta = adapter(backend, wallet, sync_state)
    outcome = hooks.insert_records(
        conn,
        profile,
        wallet,
        normalized_records,
        f"backend:{backend['name']}",
    )
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = kind
    outcome["backend_url"] = backend["url"]
    outcome["chain"] = sync_state.chain
    outcome["network"] = sync_state.network
    outcome["sync_mode"] = "descriptor" if sync_state.descriptor_plan else "addresses"
    outcome["target_count"] = len(sync_state.targets)
    if sync_state.descriptor_plan:
        outcome["gap_limit"] = sync_state.descriptor_plan.gap_limit
    else:
        outcome["addresses"] = ",".join(
            target["address"] for target in sync_state.targets if target.get("address")
        )
    if sync_state.policy_asset_id:
        outcome["policy_asset"] = sync_state.policy_asset_id
    outcome.update(dict(adapter_meta or {}))
    return outcome


def sync_wallets(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallets: Sequence[WalletRow],
    hooks: WalletSyncHooks,
) -> list[SyncOutcome]:
    results = []
    for wallet in wallets:
        config = json.loads(wallet["config_json"] or "{}")
        btcpay_config = wallet_btcpay_sync_config(config)
        source_file = config.get("source_file")
        source_format = config.get("source_format")
        addresses = hooks.normalize_addresses(config.get("addresses"))
        has_descriptor = bool(str_or_none(config.get("descriptor")))
        if btcpay_config:
            if hooks.sync_btcpay_wallet is None:
                raise AppError("BTCPay wallet sync is not configured for this runtime")
            outcome = hooks.sync_btcpay_wallet(conn, runtime_config, profile, wallet)
            results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        if source_file and source_format:
            outcome = hooks.import_file(conn, profile, wallet, source_file, source_format)
            results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        if addresses or has_descriptor:
            outcome = sync_wallet_from_backend(conn, runtime_config, profile, wallet, hooks)
            if outcome.get("status") == "skipped":
                results.append(outcome)
            else:
                results.append({"wallet": wallet["label"], "status": "synced", **outcome})
            continue
        results.append(
            {
                "wallet": wallet["label"],
                "status": "skipped",
                "reason": "no file source, descriptor, or backend addresses configured",
            }
        )
    return results


__all__ = [
    "WalletSyncHooks",
    "WalletSyncState",
    "normalize_backend_kind",
    "sync_wallet_from_backend",
    "sync_wallets",
]
