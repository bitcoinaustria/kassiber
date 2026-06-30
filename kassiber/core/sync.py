from __future__ import annotations

"""Wallet sync orchestration helpers that stay above backend adapter details."""

import json
import sqlite3
import contextvars
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, replace
from typing import Any, Callable, Mapping, MutableMapping, Sequence

from ..backends import redact_backend_text, redact_backend_url
from ..errors import AppError
from ..util import str_or_none
from . import source_overlap
from .wallets import (
    has_descriptor_sync_material,
    wallet_btcpay_provenance_config,
    wallet_btcpay_sync_config,
    wallet_bullbitcoin_wallet_export_config,
)

WalletRow = Mapping[str, Any]
ProfileRow = Mapping[str, Any]
RuntimeConfig = Mapping[str, Any]
SyncOutcome = dict[str, Any]
BackendRecord = Mapping[str, Any]
SyncTarget = Mapping[str, Any]
HistoryEntry = Mapping[str, Any]
HistoryCache = MutableMapping[str, Sequence[HistoryEntry]]
ProgressCallback = Callable[[Mapping[str, Any]], None]
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
EnrichBTCPayWallet = Callable[
    [sqlite3.Connection, RuntimeConfig, ProfileRow, WalletRow],
    SyncOutcome,
]
EnrichBullBitcoinWallet = Callable[
    [sqlite3.Connection, RuntimeConfig, ProfileRow, WalletRow],
    SyncOutcome,
]
SyncCoreLightningWallet = Callable[
    [sqlite3.Connection, RuntimeConfig, ProfileRow, WalletRow],
    SyncOutcome,
]
UpdateOutputInventory = Callable[
    [
        sqlite3.Connection,
        ProfileRow,
        WalletRow,
        BackendRecord,
        "WalletSyncState",
        Sequence[Mapping[str, Any]],
    ],
    Mapping[str, Any],
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
    checkpoint: MutableMapping[str, Any] | None = None


@dataclass(frozen=True)
class WalletSyncHooks:
    import_file: ImportFile
    insert_records: InsertRecords
    resolve_backend: ResolveBackend
    resolve_sync_state: ResolveSyncState
    normalize_addresses: NormalizeAddresses
    backend_adapters: Mapping[str, BackendAdapter]
    sync_btcpay_wallet: SyncBTCPayWallet | None = None
    enrich_btcpay_wallet: EnrichBTCPayWallet | None = None
    enrich_bullbitcoin_wallet: EnrichBullBitcoinWallet | None = None
    sync_core_lightning_wallet: SyncCoreLightningWallet | None = None
    update_output_inventory: UpdateOutputInventory | None = None


@dataclass(frozen=True)
class WalletBackendFetch:
    """Result of the network-only fetch phase for one backend-synced wallet.

    Produced by `fetch_wallet_backend` (no DB access, safe on a worker thread)
    and consumed by `sync_wallet_from_backend`'s DB write phase. `skip_outcome`
    is set instead of fetch data when the wallet has no sync targets.
    """

    backend: Mapping[str, Any]
    sync_state: "WalletSyncState | None"
    normalized_records: Sequence[BackendRecord]
    adapter_meta: Mapping[str, Any]
    kind: str
    started: float
    force_full: bool
    skip_outcome: Mapping[str, Any] | None = None


# Modest cap on concurrent per-wallet fetches. The per-host HTTP limiter already
# bounds network concurrency against any single host, so this only limits total
# thread count for the common case of a handful of distinct backends.
WALLET_FETCH_FANOUT = 4


# Contextvar threaded by the daemon when it wants long-running source refreshes
# to emit progress over the JSONL stream. The CLI leaves this empty so terminal
# sync behavior stays unchanged.
sync_progress_emitter: contextvars.ContextVar[ProgressCallback | None] = (
    contextvars.ContextVar("kassiber.sync_progress_emitter", default=None)
)

# Serializes progress-callback invocation so concurrent fetch workers (within a
# wallet and, for cross-wallet parallel fetch, across wallets) cannot interleave
# writes into a shared sink such as the daemon's JSONL stream.
_progress_emit_lock = threading.Lock()


def emit_sync_progress(payload: Mapping[str, Any]) -> None:
    progress = sync_progress_emitter.get()
    if progress is not None:
        with _progress_emit_lock:
            progress(payload)


def _wallet_label(wallet: WalletRow) -> str:
    try:
        value = wallet["label"]
    except (KeyError, IndexError, TypeError):
        value = None
    if value is None and isinstance(wallet, Mapping):
        value = wallet.get("label")
    return str(value or "Wallet")


def _backend_sync_failure_error(
    exc: Exception,
    wallet: WalletRow,
    backend: Mapping[str, Any] | None,
    phase: str,
) -> AppError:
    details: dict[str, Any] = {
        "wallet": _wallet_label(wallet),
        "phase": phase,
        "error_type": exc.__class__.__name__,
    }
    if backend is not None:
        details.update(
            {
                "backend": backend.get("name"),
                "backend_kind": backend.get("kind"),
                "chain": backend.get("chain"),
                "network": backend.get("network"),
                "has_backend_url": bool(backend.get("url")),
            }
        )
    return AppError(
        f"Source refresh failed for {_wallet_label(wallet)} during {phase}: "
        f"{redact_backend_text(str(exc) or exc.__class__.__name__)}",
        code="backend_sync_failed",
        hint=(
            "Test the selected sync backend in Settings, then retry refresh. "
            "If it still fails, include this error's details from Logs."
        ),
        details=details,
        retryable=True,
    )


def _emit_wallet_sync_progress(wallet: WalletRow, payload: Mapping[str, Any]) -> None:
    wallet_label = _wallet_label(wallet)
    emit_sync_progress({"wallet": wallet_label, **dict(payload)})


def _wrap_sync_progress_for_wallet(wallet: WalletRow):
    progress = sync_progress_emitter.get()
    if progress is None:
        return None

    wallet_label = _wallet_label(wallet)

    def _progress(payload: Mapping[str, Any]) -> None:
        progress({"wallet": wallet_label, **dict(payload)})

    return sync_progress_emitter.set(_progress)


def _merge_btcpay_enrichment(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    outcome: SyncOutcome,
) -> SyncOutcome:
    config = json.loads(wallet["config_json"] or "{}")
    routes = wallet_btcpay_provenance_config(config)
    if not routes:
        return outcome
    if hooks.enrich_btcpay_wallet is None:
        raise AppError("BTCPay provenance refresh is not configured for this runtime")
    enriched = hooks.enrich_btcpay_wallet(conn, runtime_config, profile, wallet)
    merged = dict(outcome)
    merged["btcpay_provenance"] = enriched
    return merged


def _merge_bullbitcoin_enrichment(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    outcome: SyncOutcome,
) -> SyncOutcome:
    config = json.loads(wallet["config_json"] or "{}")
    routes = wallet_bullbitcoin_wallet_export_config(config)
    if not routes:
        return outcome
    if hooks.enrich_bullbitcoin_wallet is None:
        raise AppError("Bull Bitcoin wallet export refresh is not configured for this runtime")
    enriched = hooks.enrich_bullbitcoin_wallet(conn, runtime_config, profile, wallet)
    merged = dict(outcome)
    merged["bullbitcoin_wallet_exports"] = enriched
    return merged


def _merge_wallet_enrichments(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    outcome: SyncOutcome,
) -> SyncOutcome:
    outcome = _merge_btcpay_enrichment(conn, runtime_config, profile, wallet, hooks, outcome)
    return _merge_bullbitcoin_enrichment(conn, runtime_config, profile, wallet, hooks, outcome)


def normalize_backend_kind(kind: Any) -> str:
    value = str(kind).strip().lower()
    aliases = {
        "bitcoin-core": "bitcoinrpc",
        "bitcoincore": "bitcoinrpc",
        "core": "bitcoinrpc",
        "core-ln": "coreln",
        "core-lightning": "coreln",
        "liquid-esplora": "esplora",
    }
    return aliases.get(value, value)


def fetch_wallet_backend(
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    checkpoint: Mapping[str, Any] | None = None,
    *,
    force_full: bool = False,
) -> WalletBackendFetch:
    """Run the network-only fetch phase for a backend-synced wallet.

    Touches no database connection — `resolve_backend` is an in-memory lookup,
    and discovery plus the adapter are network/compute only — so this is safe to
    run on a worker thread for cross-wallet parallel fetch. The DB write phase
    stays in `sync_wallet_from_backend` on the owning connection's thread.
    """
    del profile  # not needed to fetch; kept for call-shape symmetry with apply
    started = time.monotonic()
    config = json.loads(wallet["config_json"] or "{}")
    effective_checkpoint = {} if force_full else checkpoint
    resolver_wallet: WalletRow = (
        {**dict(wallet), "_freshness_checkpoint": dict(effective_checkpoint)}
        if effective_checkpoint is not None
        else wallet
    )
    token = _wrap_sync_progress_for_wallet(wallet)
    backend: Mapping[str, Any] | None = None
    phase = "resolve_backend"
    try:
        backend = hooks.resolve_backend(runtime_config, config.get("backend"))
        phase = "discovery"
        _emit_wallet_sync_progress(wallet, {"phase": "discovery"})
        sync_state = hooks.resolve_sync_state(backend, resolver_wallet)
        if effective_checkpoint is not None:
            sync_state = replace(sync_state, checkpoint=dict(effective_checkpoint))
        if not sync_state.targets:
            skip_outcome = {
                "wallet": wallet["label"],
                "status": "skipped",
                "reason": "no addresses or descriptors configured for backend sync",
                "elapsed_ms": int((time.monotonic() - started) * 1000),
                **({"force_full": True} if force_full else {}),
            }
            return WalletBackendFetch(
                backend=backend,
                sync_state=None,
                normalized_records=(),
                adapter_meta={},
                kind="",
                started=started,
                force_full=force_full,
                skip_outcome=skip_outcome,
            )
        kind = normalize_backend_kind(backend["kind"])
        adapter = hooks.backend_adapters.get(kind)
        if adapter is None:
            raise AppError(
                f"Source refresh is not implemented for backend kind '{kind}'",
                hint="Use an esplora, electrum, or bitcoinrpc backend for live refresh.",
            )
        _emit_wallet_sync_progress(
            wallet,
            {"phase": "backend_fetch"},
        )
        phase = "backend_fetch"
        normalized_records, adapter_meta = adapter(backend, wallet, sync_state)
    except AppError:
        raise
    except Exception as exc:
        raise _backend_sync_failure_error(exc, wallet, backend, phase) from exc
    finally:
        if token is not None:
            sync_progress_emitter.reset(token)
    return WalletBackendFetch(
        backend=backend,
        sync_state=sync_state,
        normalized_records=normalized_records,
        adapter_meta=dict(adapter_meta or {}),
        kind=kind,
        started=started,
        force_full=force_full,
    )


def sync_wallet_from_backend(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    checkpoint: Mapping[str, Any] | None = None,
    *,
    force_full: bool = False,
    prefetched: "WalletBackendFetch | BaseException | None" = None,
) -> SyncOutcome:
    # `prefetched` lets the caller run the network fetch ahead of time (e.g. in
    # parallel across wallets). When omitted, fetch inline as before. A captured
    # AppError is re-raised here so it surfaces under this wallet's own savepoint.
    if prefetched is None:
        prefetched = fetch_wallet_backend(
            runtime_config, profile, wallet, hooks, checkpoint, force_full=force_full
        )
    if isinstance(prefetched, BaseException):
        raise prefetched
    fetch = prefetched
    if fetch.skip_outcome is not None:
        return dict(fetch.skip_outcome)
    started = fetch.started
    force_full = fetch.force_full
    backend = fetch.backend
    sync_state = fetch.sync_state
    normalized_records = fetch.normalized_records
    kind = fetch.kind
    adapter_meta = dict(fetch.adapter_meta or {})
    observed_utxos = adapter_meta.pop("utxos", None)
    if conn is not None:
        source_overlap.raise_for_sync_source_overlap(conn, profile, wallet, sync_state)
    outcome = hooks.insert_records(
        conn,
        profile,
        wallet,
        normalized_records,
        f"backend:{backend['name']}",
    )
    if observed_utxos is not None and hooks.update_output_inventory is not None:
        outcome["output_inventory"] = dict(
            hooks.update_output_inventory(
                conn,
                profile,
                wallet,
                backend,
                sync_state,
                observed_utxos,
            )
        )
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = kind
    outcome["backend_url"] = redact_backend_url(backend["url"])
    outcome["chain"] = sync_state.chain
    outcome["network"] = sync_state.network
    outcome["sync_mode"] = "descriptor" if sync_state.descriptor_plan else "addresses"
    outcome["target_count"] = len(sync_state.targets)
    outcome["records_fetched"] = len(normalized_records)
    if "updated" not in outcome and isinstance(outcome.get("updated_records"), list):
        outcome["updated"] = len(outcome["updated_records"])
    if force_full:
        outcome["force_full"] = True
    if sync_state.descriptor_plan:
        outcome["gap_limit"] = sync_state.descriptor_plan.gap_limit
    else:
        outcome["addresses"] = ",".join(
            target["address"] for target in sync_state.targets if target.get("address")
        )
    if sync_state.policy_asset_id:
        outcome["policy_asset"] = sync_state.policy_asset_id
    outcome.update(dict(adapter_meta or {}))
    scripts_changed = int(outcome.get("scripts_changed") or 0)
    scripts_unchanged = int(outcome.get("scripts_unchanged") or 0)
    if scripts_changed or scripts_unchanged:
        outcome["scripts_checked"] = scripts_changed + scripts_unchanged
    outcome["utxos_refreshed"] = observed_utxos is not None
    outcome["elapsed_ms"] = int((time.monotonic() - started) * 1000)
    return outcome


def classify_wallet_sync(wallet: WalletRow, normalize_addresses: NormalizeAddresses) -> str:
    """Bucket a wallet by how `sync_wallets` will dispatch it.

    Returns one of ``btcpay`` / ``coreln`` / ``file`` / ``backend`` / ``none``.
    Only ``backend`` wallets are eligible for parallel network prefetch.
    """
    config = json.loads(wallet["config_json"] or "{}")
    if wallet_btcpay_sync_config(config):
        return "btcpay"
    if wallet["kind"] == "coreln" and config.get("backend"):
        return "coreln"
    if config.get("source_file") and config.get("source_format"):
        return "file"
    addresses = normalize_addresses(config.get("addresses"))
    if addresses or has_descriptor_sync_material(config):
        return "backend"
    return "none"


def prefetch_wallets_backend(
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallets: Sequence[WalletRow],
    hooks: WalletSyncHooks,
    checkpoints: Mapping[str, Mapping[str, Any]] | None = None,
    *,
    force_full: bool = False,
    max_workers: int = WALLET_FETCH_FANOUT,
) -> dict[str, "WalletBackendFetch | BaseException"]:
    """Run the network-only fetch for several backend wallets concurrently.

    Returns ``{wallet_id: WalletBackendFetch | AppError}``. Per-wallet AppErrors
    are captured (mirroring the serial path's per-wallet AppError isolation) and
    re-raised when applied under that wallet's savepoint; any non-AppError
    propagates, as it would have on the serial path. Each fetch runs inside a
    copy of this thread's context so the per-wallet progress emitter propagates
    to the worker.
    """
    wallets = list(wallets)
    if not wallets:
        return {}

    def _fetch(wallet: WalletRow):
        checkpoint = {} if force_full else (checkpoints or {}).get(str(wallet["id"]))
        try:
            return fetch_wallet_backend(
                runtime_config, profile, wallet, hooks, checkpoint, force_full=force_full
            )
        except AppError as exc:
            return exc

    if len(wallets) == 1 or max_workers <= 1:
        return {str(wallet["id"]): _fetch(wallet) for wallet in wallets}
    results: dict[str, Any] = {}
    with ThreadPoolExecutor(max_workers=min(max_workers, len(wallets))) as executor:
        futures = {
            str(wallet["id"]): executor.submit(contextvars.copy_context().run, _fetch, wallet)
            for wallet in wallets
        }
        for wallet_id, future in futures.items():
            results[wallet_id] = future.result()
    return results


def sync_wallets(
    conn: sqlite3.Connection,
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallets: Sequence[WalletRow],
    hooks: WalletSyncHooks,
    checkpoints: Mapping[str, Mapping[str, Any]] | None = None,
    *,
    force_full: bool = False,
    prefetched: Mapping[str, "WalletBackendFetch | BaseException"] | None = None,
) -> list[SyncOutcome]:
    results = []
    for wallet in wallets:
        wallet_checkpoint = {} if force_full else (checkpoints or {}).get(str(wallet["id"]))
        sync_wallet: WalletRow = (
            {**dict(wallet), "_freshness_checkpoint": dict(wallet_checkpoint)}
            if wallet_checkpoint is not None
            else wallet
        )
        config = json.loads(sync_wallet["config_json"] or "{}")
        btcpay_config = wallet_btcpay_sync_config(config)
        source_file = config.get("source_file")
        source_format = config.get("source_format")
        addresses = hooks.normalize_addresses(config.get("addresses"))
        has_descriptor = has_descriptor_sync_material(config)
        if btcpay_config:
            if hooks.sync_btcpay_wallet is None:
                raise AppError("BTCPay source refresh is not configured for this runtime")
            token = _wrap_sync_progress_for_wallet(sync_wallet)
            try:
                _emit_wallet_sync_progress(sync_wallet, {"phase": "backend_fetch"})
                outcome = hooks.sync_btcpay_wallet(conn, runtime_config, profile, sync_wallet)
            finally:
                if token is not None:
                    sync_progress_emitter.reset(token)
            outcome = _merge_bullbitcoin_enrichment(
                conn,
                runtime_config,
                profile,
                sync_wallet,
                hooks,
                outcome,
            )
            results.append({"wallet": sync_wallet["label"], "status": "synced", **outcome})
            continue
        if sync_wallet["kind"] == "coreln" and config.get("backend"):
            if hooks.sync_core_lightning_wallet is None:
                raise AppError("Core Lightning source refresh is not configured for this runtime")
            token = _wrap_sync_progress_for_wallet(sync_wallet)
            try:
                _emit_wallet_sync_progress(sync_wallet, {"phase": "backend_fetch"})
                outcome = hooks.sync_core_lightning_wallet(conn, runtime_config, profile, sync_wallet)
            finally:
                if token is not None:
                    sync_progress_emitter.reset(token)
            outcome = _merge_bullbitcoin_enrichment(
                conn,
                runtime_config,
                profile,
                sync_wallet,
                hooks,
                outcome,
            )
            results.append({"wallet": sync_wallet["label"], **outcome})
            continue
        if source_file and source_format:
            outcome = hooks.import_file(conn, profile, sync_wallet, source_file, source_format)
            outcome = _merge_wallet_enrichments(
                conn,
                runtime_config,
                profile,
                sync_wallet,
                hooks,
                outcome,
            )
            results.append({"wallet": sync_wallet["label"], "status": "synced", **outcome})
            continue
        if addresses or has_descriptor:
            checkpoint = {} if force_full else (checkpoints or {}).get(str(wallet["id"]))
            outcome = sync_wallet_from_backend(
                conn,
                runtime_config,
                profile,
                sync_wallet,
                hooks,
                checkpoint=checkpoint,
                force_full=force_full,
                prefetched=(prefetched or {}).get(str(wallet["id"])),
            )
            if outcome.get("status") == "skipped":
                results.append(outcome)
            else:
                outcome = _merge_wallet_enrichments(
                    conn,
                    runtime_config,
                    profile,
                    sync_wallet,
                    hooks,
                    outcome,
                )
                results.append({"wallet": sync_wallet["label"], "status": "synced", **outcome})
            continue
        results.append(
            {
                "wallet": sync_wallet["label"],
                "status": "skipped",
                "reason": "no file source, descriptor, or backend addresses configured",
            }
        )
    return results


__all__ = [
    "WalletBackendFetch",
    "WalletSyncHooks",
    "WalletSyncState",
    "WALLET_FETCH_FANOUT",
    "classify_wallet_sync",
    "emit_sync_progress",
    "fetch_wallet_backend",
    "normalize_backend_kind",
    "prefetch_wallets_backend",
    "sync_wallet_from_backend",
    "sync_progress_emitter",
    "sync_wallets",
]
