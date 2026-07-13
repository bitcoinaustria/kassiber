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
from ..wallet_descriptors import DEFAULT_DESCRIPTOR_GAP_LIMIT, MAX_DESCRIPTOR_GAP_LIMIT
from . import source_overlap
from .wallets import (
    has_descriptor_sync_material,
    has_silent_payment_sync_material,
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
RetractRecords = Callable[[sqlite3.Connection, ProfileRow, WalletRow, Sequence[str], str], SyncOutcome]
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
SyncLndWallet = Callable[
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
SourceOverlapPreflight = Callable[[WalletRow, "WalletSyncState"], "WalletSyncState | None"]
PersistObserverUpdate = Callable[
    [sqlite3.Connection, ProfileRow, WalletRow, "WalletBackendFetch"],
    None,
]
UpdateDerivationCoverage = Callable[
    [
        sqlite3.Connection,
        ProfileRow,
        WalletRow,
        "WalletSyncState",
        Mapping[str, Any],
    ],
    None,
]
AfterApplyStage = Callable[[str], None]
DiscardObserverUpdate = Callable[[WalletRow], None]


APPLY_STAGE_OBSERVER_PERSISTENCE = "observer_persistence"
APPLY_STAGE_RETRACTIONS = "retractions"
APPLY_STAGE_TRANSACTION_INSERTION = "transaction_insertion"
APPLY_STAGE_OUTPUT_INVENTORY = "output_inventory"
APPLY_STAGE_DERIVATION_COVERAGE = "derivation_coverage"
APPLY_STAGE_FRESHNESS_CHECKPOINT = "freshness_checkpoint"


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
    retract_records: RetractRecords | None = None
    sync_btcpay_wallet: SyncBTCPayWallet | None = None
    enrich_btcpay_wallet: EnrichBTCPayWallet | None = None
    enrich_bullbitcoin_wallet: EnrichBullBitcoinWallet | None = None
    sync_core_lightning_wallet: SyncCoreLightningWallet | None = None
    sync_lnd_wallet: SyncLndWallet | None = None
    update_output_inventory: UpdateOutputInventory | None = None
    persist_observer_update: PersistObserverUpdate | None = None
    update_derivation_coverage: UpdateDerivationCoverage | None = None
    after_apply_stage: AfterApplyStage | None = None
    discard_observer_update: DiscardObserverUpdate | None = None


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


@dataclass(frozen=True)
class WalletBackendDiscovery:
    """Resolved backend targets before any backend adapter side effects run."""

    backend: Mapping[str, Any]
    sync_state: "WalletSyncState | None"
    kind: str
    started: float
    force_full: bool
    skip_outcome: Mapping[str, Any] | None = None


def notify_apply_stage(hooks: WalletSyncHooks, stage: str) -> None:
    """Expose deterministic fault-injection seams without owning a transaction."""

    if hooks.after_apply_stage is not None:
        hooks.after_apply_stage(stage)


# Modest cap on concurrent per-wallet fetches. The per-host HTTP limiter already
# bounds network concurrency against any single host, so this only limits total
# thread count for the common case of a handful of distinct backends.
WALLET_FETCH_FANOUT = 4

# A negative running wallet balance means the local ledger is missing earlier
# inbound history. One repair refresh widens descriptor discovery enough to find
# common high-index receive/change gaps without permanently mutating the wallet.
NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT = 500


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


def _overlap_filtered_skip_outcome(
    wallet: WalletRow,
    *,
    started: float,
    force_full: bool,
    original_target_count: int,
) -> Mapping[str, Any]:
    filtered_count = max(0, int(original_target_count or 0))
    return {
        "wallet": _wallet_label(wallet),
        "status": "skipped",
        "reason": "all sync targets are already covered by canonical wallet sources",
        "target_count": 0,
        "filtered_overlap_targets": filtered_count,
        "elapsed_ms": int((time.monotonic() - started) * 1000),
        **({"force_full": True} if force_full else {}),
    }


def _apply_source_overlap_preflight(
    wallet: WalletRow,
    discovery: WalletBackendDiscovery,
    source_overlap_preflight: SourceOverlapPreflight | None,
) -> WalletBackendDiscovery:
    if (
        source_overlap_preflight is None
        or discovery.skip_outcome is not None
        or discovery.sync_state is None
    ):
        return discovery
    original_target_count = len(discovery.sync_state.targets)
    filtered_state = source_overlap_preflight(wallet, discovery.sync_state)
    if filtered_state is None:
        filtered_state = discovery.sync_state
    if filtered_state is not discovery.sync_state:
        discovery = replace(discovery, sync_state=filtered_state)
    if not filtered_state.targets:
        return replace(
            discovery,
            sync_state=None,
            kind=discovery.kind,
            skip_outcome=_overlap_filtered_skip_outcome(
                wallet,
                started=discovery.started,
                force_full=discovery.force_full,
                original_target_count=original_target_count,
            ),
        )
    return discovery


def _prefetched_overlap_payload_present(
    normalized_records: Sequence[BackendRecord],
    adapter_meta: Mapping[str, Any],
) -> bool:
    if normalized_records:
        return True
    return bool(adapter_meta)


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


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        return []
    seen: set[str] = set()
    output: list[str] = []
    for item in value:
        text = str_or_none(item)
        if text is None:
            continue
        normalized = text.strip().lower()
        if not normalized or normalized in seen:
            continue
        seen.add(normalized)
        output.append(normalized)
    return output


def discover_wallet_backend(
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    checkpoint: Mapping[str, Any] | None = None,
    *,
    force_full: bool = False,
) -> WalletBackendDiscovery:
    """Resolve backend targets for a wallet without running the backend adapter.

    Discovery may contact the selected backend to bound descriptor targets, but
    it must not mutate the backend or the local DB. The source-overlap preflight
    runs against this result before adapter code such as Bitcoin Core address
    import/rescan is allowed to execute.
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
            return WalletBackendDiscovery(
                backend=backend,
                sync_state=None,
                kind="",
                started=started,
                force_full=force_full,
                skip_outcome=skip_outcome,
            )
        kind = normalize_backend_kind(backend["kind"])
        if hooks.backend_adapters.get(kind) is None:
            raise AppError(
                f"Source refresh is not implemented for backend kind '{kind}'",
                hint="Use an esplora, electrum, or bitcoinrpc backend for live refresh.",
            )
    except AppError:
        raise
    except Exception as exc:
        raise _backend_sync_failure_error(exc, wallet, backend, phase) from exc
    finally:
        if token is not None:
            sync_progress_emitter.reset(token)
    return WalletBackendDiscovery(
        backend=backend,
        sync_state=sync_state,
        kind=kind,
        started=started,
        force_full=force_full,
    )


def fetch_wallet_backend_from_discovery(
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    discovery: WalletBackendDiscovery,
) -> WalletBackendFetch:
    if discovery.skip_outcome is not None:
        return WalletBackendFetch(
            backend=discovery.backend,
            sync_state=None,
            normalized_records=(),
            adapter_meta={},
            kind=discovery.kind,
            started=discovery.started,
            force_full=discovery.force_full,
            skip_outcome=discovery.skip_outcome,
        )
    sync_state = discovery.sync_state
    if sync_state is None:
        raise AppError("Wallet discovery did not return sync targets", code="sync_state_missing")
    adapter = hooks.backend_adapters.get(discovery.kind)
    if adapter is None:
        raise AppError(
            f"Source refresh is not implemented for backend kind '{discovery.kind}'",
            hint="Use an esplora, electrum, or bitcoinrpc backend for live refresh.",
        )
    token = _wrap_sync_progress_for_wallet(wallet)
    try:
        _emit_wallet_sync_progress(wallet, {"phase": "backend_fetch"})
        try:
            normalized_records, adapter_meta = adapter(discovery.backend, wallet, sync_state)
        except AppError:
            raise
        except Exception as exc:
            raise _backend_sync_failure_error(
                exc,
                wallet,
                discovery.backend,
                "backend_fetch",
            ) from exc
    finally:
        if token is not None:
            sync_progress_emitter.reset(token)
    return WalletBackendFetch(
        backend=discovery.backend,
        sync_state=sync_state,
        normalized_records=normalized_records,
        adapter_meta=dict(adapter_meta or {}),
        kind=discovery.kind,
        started=discovery.started,
        force_full=discovery.force_full,
    )


def fetch_wallet_backend(
    runtime_config: RuntimeConfig,
    profile: ProfileRow,
    wallet: WalletRow,
    hooks: WalletSyncHooks,
    checkpoint: Mapping[str, Any] | None = None,
    *,
    force_full: bool = False,
    source_overlap_preflight: SourceOverlapPreflight | None = None,
) -> WalletBackendFetch:
    """Run discovery, optional overlap preflight, then backend fetch."""
    discovery = discover_wallet_backend(
        runtime_config,
        profile,
        wallet,
        hooks,
        checkpoint,
        force_full=force_full,
    )
    if (
        source_overlap_preflight is not None
        and discovery.skip_outcome is None
        and discovery.sync_state is not None
    ):
        discovery = _apply_source_overlap_preflight(
            wallet,
            discovery,
            source_overlap_preflight,
        )
    return fetch_wallet_backend_from_discovery(wallet, hooks, discovery)


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
    _allow_negative_balance_rescan: bool = True,
) -> SyncOutcome:
    # `prefetched` lets the caller run the network fetch ahead of time (e.g. in
    # parallel across wallets). When omitted, fetch inline as before. A captured
    # AppError is re-raised here so it surfaces under this wallet's own savepoint.
    if prefetched is None:
        preflight = (
            (lambda preflight_wallet, sync_state: source_overlap.filter_sync_state_for_canonical_owner(
                conn,
                profile,
                preflight_wallet,
                sync_state,
            ))
            if conn is not None
            else None
        )
        prefetched = fetch_wallet_backend(
            runtime_config,
            profile,
            wallet,
            hooks,
            checkpoint,
            force_full=force_full,
            source_overlap_preflight=preflight,
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
    prepared_negative_balance_rescan = adapter_meta.pop(
        "_prepared_negative_balance_rescan",
        None,
    )
    if conn is not None and sync_state is not None:
        filtered_sync_state = source_overlap.filter_sync_state_for_canonical_owner(
            conn,
            profile,
            wallet,
            sync_state,
        )
        if filtered_sync_state is not sync_state:
            if _prefetched_overlap_payload_present(normalized_records, adapter_meta):
                raise AppError(
                    f"Wallet source overlap changed during refresh for {_wallet_label(wallet)}",
                    code="source_overlap_retry",
                    hint=(
                        "Retry refresh so Kassiber can filter overlapping sync targets before "
                        "contacting the backend and importing history."
                    ),
                    retryable=True,
                )
            if not filtered_sync_state.targets:
                return dict(
                    _overlap_filtered_skip_outcome(
                        wallet,
                        started=started,
                        force_full=force_full,
                        original_target_count=len(sync_state.targets),
                    )
                )
            sync_state = filtered_sync_state
    if hooks.persist_observer_update is not None:
        hooks.persist_observer_update(conn, profile, wallet, fetch)
    notify_apply_stage(hooks, APPLY_STAGE_OBSERVER_PERSISTENCE)
    observed_utxos = adapter_meta.pop("utxos", None)
    retracted_external_ids = _string_list(
        adapter_meta.pop("bitcoinrpc_retracted_txids", [])
    )
    retraction_outcome: SyncOutcome | None = None
    if retracted_external_ids and hooks.retract_records is not None:
        retraction_outcome = hooks.retract_records(
            conn,
            profile,
            wallet,
            retracted_external_ids,
            f"backend:{backend['name']}",
        )
    notify_apply_stage(hooks, APPLY_STAGE_RETRACTIONS)
    outcome = hooks.insert_records(
        conn,
        profile,
        wallet,
        normalized_records,
        f"backend:{backend['name']}",
    )
    notify_apply_stage(hooks, APPLY_STAGE_TRANSACTION_INSERTION)
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
    notify_apply_stage(hooks, APPLY_STAGE_OUTPUT_INVENTORY)
    if hooks.update_derivation_coverage is not None:
        hooks.update_derivation_coverage(
            conn,
            profile,
            wallet,
            sync_state,
            adapter_meta,
        )
    notify_apply_stage(hooks, APPLY_STAGE_DERIVATION_COVERAGE)
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = kind
    outcome["backend_url"] = redact_backend_url(backend["url"])
    outcome["chain"] = sync_state.chain
    outcome["network"] = sync_state.network
    if getattr(sync_state.descriptor_plan, "kind", None) == "silent-payment":
        outcome["sync_mode"] = "silent_payment"
    else:
        outcome["sync_mode"] = "descriptor" if sync_state.descriptor_plan else "addresses"
    outcome["target_count"] = len(sync_state.targets)
    outcome["records_fetched"] = len(normalized_records)
    if "updated" not in outcome and isinstance(outcome.get("updated_records"), list):
        outcome["updated"] = len(outcome["updated_records"])
    if retracted_external_ids:
        outcome["bitcoinrpc_retracted_txids"] = retracted_external_ids
    if retraction_outcome is not None:
        outcome["retracted"] = int(retraction_outcome.get("retracted") or 0)
        outcome["retracted_records"] = list(
            retraction_outcome.get("retracted_records") or []
        )
        outcome["journal_invalidated"] = bool(
            outcome.get("journal_invalidated")
            or retraction_outcome.get("journal_invalidated")
        )
    if force_full:
        outcome["force_full"] = True
    if sync_state.descriptor_plan and getattr(sync_state.descriptor_plan, "kind", None) != "silent-payment":
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
    if isinstance(prepared_negative_balance_rescan, Mapping):
        remaining_events = _wallet_negative_balance_events(
            conn,
            str(profile["id"]),
            str(wallet["id"]),
        )
        outcome["negative_balance_rescan"] = {
            **dict(prepared_negative_balance_rescan),
            "resolved": not remaining_events,
            "remaining_negative_events": remaining_events,
        }
        return outcome
    if _allow_negative_balance_rescan and conn is not None:
        negative_events = _wallet_negative_balance_events(
            conn,
            str(profile["id"]),
            str(wallet["id"]),
        )
        if negative_events:
            rescan_gap_limit = negative_balance_rescan_gap_limit(sync_state)
            repair_wallet = wallet_with_temporary_gap_limit(wallet, rescan_gap_limit)
            repair_outcome = sync_wallet_from_backend(
                conn,
                runtime_config,
                profile,
                repair_wallet,
                hooks,
                checkpoint={},
                force_full=True,
                _allow_negative_balance_rescan=False,
            )
            remaining_events = _wallet_negative_balance_events(
                conn,
                str(profile["id"]),
                str(wallet["id"]),
            )
            repair_outcome["negative_balance_rescan"] = {
                "triggered": True,
                "resolved": not remaining_events,
                "initial_negative_events": negative_events,
                "remaining_negative_events": remaining_events,
                "original_gap_limit": (
                    getattr(sync_state.descriptor_plan, "gap_limit", None)
                    if sync_state.descriptor_plan is not None
                    else None
                ),
                "rescan_gap_limit": rescan_gap_limit,
            }
            return repair_outcome
    return outcome


def _wallet_negative_balance_events(
    conn: sqlite3.Connection | None,
    profile_id: str,
    wallet_id: str,
) -> list[dict[str, Any]]:
    if conn is None:
        return []
    rows = conn.execute(
        """
        SELECT id, external_id, occurred_at, direction, asset, amount, fee, created_at
        FROM transactions
        WHERE profile_id = ? AND wallet_id = ? AND excluded = 0
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id, wallet_id),
    ).fetchall()
    balances: dict[str, int] = {}
    first_negative_by_asset: dict[str, dict[str, Any]] = {}
    for row in rows:
        asset = str(row["asset"] or "")
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        if row["direction"] == "inbound":
            delta = amount
        elif row["direction"] == "outbound":
            delta = -amount - fee
        else:
            delta = 0
        next_balance = balances.get(asset, 0) + delta
        balances[asset] = next_balance
        if next_balance < 0 and asset not in first_negative_by_asset:
            first_negative_by_asset[asset] = {
                "asset": asset,
                "transaction_id": row["id"],
                "external_id": row["external_id"],
                "occurred_at": row["occurred_at"],
                "delta_msat": delta,
                "running_balance_msat": next_balance,
            }
    return list(first_negative_by_asset.values())


def negative_balance_rescan_gap_limit(sync_state: WalletSyncState) -> int | None:
    plan = sync_state.descriptor_plan
    if plan is None:
        return None
    try:
        current = int(getattr(plan, "gap_limit", 0) or 0)
    except (TypeError, ValueError):
        current = DEFAULT_DESCRIPTOR_GAP_LIMIT
    current = max(current, DEFAULT_DESCRIPTOR_GAP_LIMIT)
    return min(
        MAX_DESCRIPTOR_GAP_LIMIT,
        max(NEGATIVE_BALANCE_RESCAN_MIN_GAP_LIMIT, current * 2),
    )


def wallet_with_temporary_gap_limit(wallet: WalletRow, gap_limit: int | None) -> WalletRow:
    if gap_limit is None:
        return wallet
    config = json.loads(wallet["config_json"] or "{}")
    config["gap_limit"] = int(gap_limit)
    return {**dict(wallet), "config_json": json.dumps(config, sort_keys=True)}


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
    if addresses or has_descriptor_sync_material(config) or has_silent_payment_sync_material(config):
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
    source_overlap_preflight: SourceOverlapPreflight | None = None,
) -> dict[str, "WalletBackendFetch | BaseException"]:
    """Run discovery/preflight/fetch for several backend wallets.

    Returns ``{wallet_id: WalletBackendFetch | AppError}``. Per-wallet AppErrors
    are captured (mirroring the serial path's per-wallet AppError isolation) and
    re-raised when applied under that wallet's savepoint; any non-AppError
    propagates, as it would have on the serial path. Discovery and fetch work run
    in worker threads; the optional overlap preflight runs on the caller thread
    between them so it can safely use the owning SQLite connection before
    adapter-side effects such as Bitcoin Core address import.
    """
    wallets = list(wallets)
    if not wallets:
        return {}

    def _parallel_wallet_step(step):
        if len(wallets) == 1 or max_workers <= 1:
            return {str(wallet["id"]): step(wallet) for wallet in wallets}
        results: dict[str, Any] = {}
        with ThreadPoolExecutor(max_workers=min(max_workers, len(wallets))) as executor:
            futures = {
                str(wallet["id"]): executor.submit(contextvars.copy_context().run, step, wallet)
                for wallet in wallets
            }
            for wallet_id, future in futures.items():
                results[wallet_id] = future.result()
        return results

    def _discover(wallet: WalletRow):
        checkpoint = {} if force_full else (checkpoints or {}).get(str(wallet["id"]))
        try:
            return discover_wallet_backend(
                runtime_config,
                profile,
                wallet,
                hooks,
                checkpoint,
                force_full=force_full,
            )
        except AppError as exc:
            return exc

    discoveries = _parallel_wallet_step(_discover)
    if source_overlap_preflight is not None:
        for wallet in wallets:
            wallet_id = str(wallet["id"])
            discovery = discoveries.get(wallet_id)
            if isinstance(discovery, BaseException):
                continue
            if (
                discovery is None
                or discovery.skip_outcome is not None
                or discovery.sync_state is None
            ):
                continue
            try:
                discoveries[wallet_id] = _apply_source_overlap_preflight(
                    wallet,
                    discovery,
                    source_overlap_preflight,
                )
            except AppError as exc:
                discoveries[wallet_id] = exc

    def _fetch(wallet: WalletRow):
        discovery = discoveries.get(str(wallet["id"]))
        if isinstance(discovery, BaseException):
            return discovery
        if discovery is None:
            return AppError("Wallet discovery result was not found", code="sync_state_missing")
        try:
            return fetch_wallet_backend_from_discovery(wallet, hooks, discovery)
        except AppError as exc:
            return exc

    return _parallel_wallet_step(_fetch)


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
        has_silent_payment = has_silent_payment_sync_material(config)
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
        if sync_wallet["kind"] == "lnd" and config.get("backend"):
            if hooks.sync_lnd_wallet is None:
                raise AppError("LND source refresh is not configured for this runtime")
            token = _wrap_sync_progress_for_wallet(sync_wallet)
            try:
                _emit_wallet_sync_progress(sync_wallet, {"phase": "backend_fetch"})
                outcome = hooks.sync_lnd_wallet(conn, runtime_config, profile, sync_wallet)
            finally:
                if token is not None:
                    sync_progress_emitter.reset(token)
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
        if addresses or has_descriptor or has_silent_payment:
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
    "APPLY_STAGE_DERIVATION_COVERAGE",
    "APPLY_STAGE_FRESHNESS_CHECKPOINT",
    "APPLY_STAGE_OBSERVER_PERSISTENCE",
    "APPLY_STAGE_OUTPUT_INVENTORY",
    "APPLY_STAGE_RETRACTIONS",
    "APPLY_STAGE_TRANSACTION_INSERTION",
    "WalletBackendFetch",
    "WalletSyncHooks",
    "WalletSyncState",
    "WALLET_FETCH_FANOUT",
    "classify_wallet_sync",
    "emit_sync_progress",
    "fetch_wallet_backend",
    "normalize_backend_kind",
    "negative_balance_rescan_gap_limit",
    "notify_apply_stage",
    "prefetch_wallets_backend",
    "sync_wallet_from_backend",
    "sync_progress_emitter",
    "sync_wallets",
    "wallet_with_temporary_gap_limit",
]
