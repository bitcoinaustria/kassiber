"""Daemon-facing source freshness, maintenance, and background worker glue."""

from __future__ import annotations

import json
import logging
import re
import sqlite3
import threading
import time
from collections.abc import Mapping as AbcMapping
from datetime import datetime, timezone
from typing import Any, Callable, Mapping, Protocol

from .backends import merge_db_backends
from .cli.handlers import (
    apply_transfer_rules,
    bulk_pair_transfers,
    cache_swap_candidate_count,
    enrich_wallet_from_btcpay_provenance,
    process_journals,
    suggest_transfer_candidates,
    sync_configured_btcpay_wallet,
    sync_wallet,
    sync_wallet_from_backend,
)
from .core import freshness as core_freshness
from .core import rates as core_rates
from .core import wallets as core_wallets
from .core.repo import current_context_snapshot
from .core.sync import sync_progress_emitter
from .core.ui_snapshot import build_report_blockers_snapshot
from .db import open_db
from .envelope import build_envelope, build_event_envelope
from .redaction import redact_operational_text
from .errors import AppError
from .log_ring import current_request_id
from .time_utils import now_iso, parse_iso_datetime_or_none
from .util import str_or_none

_LOGGER = logging.getLogger("kassiber.daemon.freshness")


class FreshnessOutputChannel(Protocol):
    def write(self, payload: dict[str, Any]) -> None:
        pass


class FreshnessDaemonContext(Protocol):
    conn: sqlite3.Connection | None
    data_root: str
    runtime_config: dict[str, object]
    out: FreshnessOutputChannel
    freshness_stop_event: threading.Event
    db_passphrase: str | None
    freshness_worker: threading.Thread | None


AUTO_SYNC_PROFILE_MIN_INTERVAL_SECONDS = 60
FRESHNESS_BACKGROUND_POLL_SECONDS = 5.0
FRESHNESS_BACKGROUND_REFRESH_INTERVAL_SECONDS = 15 * 60
FRESHNESS_BACKGROUND_RATE_REFRESH_INTERVAL_SECONDS = 60 * 60
_AUTO_SYNC_PROFILE_LAST_ATTEMPT: dict[str, float] = {}
_AUTO_SYNC_PROFILE_LAST_RESULT: dict[str, dict[str, Any]] = {}
_AUTO_SYNC_PROFILE_LOCK = threading.Lock()


def _remember_unlocked_passphrase(
    ctx: FreshnessDaemonContext,
    passphrase: str | None,
) -> None:
    """Keep the DB passphrase only for the current unlocked daemon session."""
    if passphrase:
        ctx.db_passphrase = passphrase


def _clear_unlocked_passphrase(ctx: FreshnessDaemonContext) -> None:
    ctx.db_passphrase = None


def _coerce_wallets_sync_args(raw_args: dict[str, Any], *, strict: bool) -> dict[str, Any]:
    if strict:
        unknown = sorted(set(raw_args) - {"wallet", "all", "force_full"})
        if unknown:
            raise AppError(
                "ui.wallets.sync received unsupported arguments",
                code="validation",
                details={"unknown": unknown},
                retryable=False,
            )
    wallet = raw_args.get("wallet")
    if wallet is not None:
        if not isinstance(wallet, str) or not wallet.strip():
            raise AppError(
                "ui.wallets.sync wallet must be a non-empty string",
                code="validation",
                details={"type": type(wallet).__name__},
                retryable=False,
            )
        wallet = wallet.strip()
    sync_all_raw = raw_args.get("all")
    if sync_all_raw is not None and not isinstance(sync_all_raw, bool):
        raise AppError(
            "ui.wallets.sync all must be a boolean",
            code="validation",
            details={"type": type(sync_all_raw).__name__},
            retryable=False,
        )
    sync_all = bool(sync_all_raw if sync_all_raw is not None else wallet is None)
    if sync_all and wallet:
        raise AppError(
            "ui.wallets.sync wallet and all are mutually exclusive",
            code="validation",
            retryable=False,
        )
    force_full = raw_args.get("force_full")
    if force_full is not None and not isinstance(force_full, bool):
        raise AppError(
            "ui.wallets.sync force_full must be a boolean",
            code="validation",
            details={"type": type(force_full).__name__},
            retryable=False,
        )
    return {"wallet": wallet, "all": sync_all, "force_full": bool(force_full)}


def _wallets_sync_payload(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    raw_args: dict[str, Any],
    *,
    strict: bool,
    progress_observer: Callable[[Mapping[str, Any]], None] | None = None,
) -> dict[str, Any]:
    args = _coerce_wallets_sync_args(raw_args, strict=strict)
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {"results": []}
    payload = _freshness_run_payload(
        conn,
        runtime_config,
        {
            "wallet": args["wallet"],
            "all": args["all"],
            "rates": False,
            "journals": False,
            "run": True,
            "force_full": args["force_full"],
        },
        progress_observer=progress_observer,
    )
    payload["results"] = payload.get("results") or []
    if not payload["results"] and not payload.get("enqueued"):
        payload["results"] = sync_wallet(
            conn,
            runtime_config,
            None,
            None,
            wallet_ref=args["wallet"],
            sync_all=args["all"],
            force_full=args["force_full"],
        )
    return _redact_sync_payload_for_ui(payload)


def _redact_sync_payload_for_ui(value: Any) -> Any:
    if isinstance(value, dict):
        redacted: dict[str, Any] = {}
        for key, item in value.items():
            if key == "backend_url":
                redacted["has_backend_url"] = bool(item)
                continue
            redacted[key] = _redact_sync_payload_for_ui(item)
        if "results" in redacted:
            redacted.setdefault("ok", not _sync_payload_has_errors(redacted))
        return redacted
    if isinstance(value, list):
        return [_redact_sync_payload_for_ui(item) for item in value]
    if isinstance(value, str):
        return _redact_sync_text_for_ui(value)
    return value


def _freshness_snapshot_for_ui(
    conn: sqlite3.Connection, profile_id: str
) -> dict[str, Any]:
    """Render-time chokepoint for every freshness snapshot that reaches the UI.

    ``core_freshness.build_snapshot`` carries raw job/source error strings
    (e.g. ``last_error_message`` = ``str(exc)``) that can embed backend URLs and
    inline credentials. The structured ``redact_freshness_payload`` only scrubs
    secret *keys*, not URLs inside a free-text ``message``/``last_error_message``
    value — so the snapshot must go through the free-text URL scrubber before it
    is surfaced. Routing every consumer through here keeps that guarantee in one
    place instead of relying on each call site to remember.
    """
    return _redact_sync_payload_for_ui(
        core_freshness.build_snapshot(conn, profile_id)
    )


_SYNC_URL_RE = re.compile(
    r"\b[a-zA-Z][a-zA-Z0-9+.-]*://"
    r"(?:\[[^\]\s]+\][^\s,;)\"'\]]*|[^\s,;)\"'\]]+)"
)
_SYNC_URL_TRAILING_PUNCTUATION = ":.!?"
# Defense in depth: HTTP-client connection-error reprs (urllib3/requests/httpx)
# render the host schemeless as host='…' / host="…", which the scheme-form
# pattern above does not catch. The stdlib client kassiber uses today always
# embeds scheme-form URLs (kassiber/http_client.py), so this is not reachable in
# practice — but a future client swap, or a schemeless str(exc) routed through
# the generic freshness catch-all, would otherwise leak the host.
_SYNC_HOST_KW_RE = re.compile(r"\bhost\s*=\s*['\"]?[^\s,;)'\"]+['\"]?", re.IGNORECASE)


def _redact_sync_text_for_ui(value: str) -> str:
    def replace(match: re.Match[str]) -> str:
        url = match.group(0)
        suffix = url[len(url.rstrip(_SYNC_URL_TRAILING_PUNCTUATION)) :]
        return f"<backend-url>{suffix}"

    scrubbed = _SYNC_URL_RE.sub(replace, value)
    scrubbed = _SYNC_HOST_KW_RE.sub("host=<backend-host>", scrubbed)
    # Backend exception messages routinely interpolate a txid:vout outpoint;
    # pseudonymize txids/amounts before the snapshot reaches the UI (and the log
    # ring through it). Addresses stay readable per the operational-tier policy.
    return redact_operational_text(scrubbed)


def _sync_error_rows(payload: dict[str, Any] | None) -> list[dict[str, Any]]:
    if not isinstance(payload, dict):
        return []
    results = payload.get("results")
    if not isinstance(results, list):
        return []
    return [
        row
        for row in results
        if isinstance(row, dict) and str(row.get("status") or "").lower() == "error"
    ]


def _sync_payload_has_errors(payload: dict[str, Any] | None) -> bool:
    if not isinstance(payload, dict):
        return False
    return payload.get("ok") is False or bool(_sync_error_rows(payload))


def _sync_failure_blocker(payload: dict[str, Any] | None) -> dict[str, Any] | None:
    if not _sync_payload_has_errors(payload):
        return None
    errors = _sync_error_rows(payload)
    detail = (
        f"Automatic watch-only refresh failed for {len(errors)} source(s); reports may be stale."
        if errors
        else "Automatic watch-only refresh failed; reports may be stale."
    )
    return {
        "id": "sync_failed",
        "severity": "blocking",
        "title": "Connection refresh failed",
        "detail": detail,
        "daemon_kind": "ui.wallets.sync",
    }


def _apply_sync_failure_blocker(
    payload: dict[str, Any],
    sync_payload: dict[str, Any] | None,
) -> dict[str, Any]:
    blocker = _sync_failure_blocker(sync_payload)
    if blocker is None:
        return payload
    updated = dict(payload)
    blockers = list(updated.get("blockers") or [])
    if not any(isinstance(item, dict) and item.get("id") == blocker["id"] for item in blockers):
        blockers.insert(0, blocker)
    updated["blockers"] = blockers
    updated["ready"] = False
    return updated


def _journals_process_payload(conn: sqlite3.Connection) -> dict[str, Any]:
    return process_journals(conn, None, None)


def _job_scope_refs(conn: sqlite3.Connection, job: Mapping[str, Any]) -> tuple[str, str]:
    profile_id = str(job.get("profile_id") or "")
    profile = conn.execute(
        "SELECT id, workspace_id FROM profiles WHERE id = ?",
        (profile_id,),
    ).fetchone()
    if profile is None:
        raise AppError("Freshness job profile was not found", code="not_found")
    return str(profile["workspace_id"]), str(profile["id"])


def _transfer_candidate_counts(payload: Mapping[str, Any]) -> dict[str, int]:
    counts = payload.get("counts") if isinstance(payload.get("counts"), AbcMapping) else {}
    return {
        "total": int(counts.get("total") or 0),
        "exact": int(counts.get("exact") or 0),
        "strong": int(counts.get("strong") or 0),
        "conflicts": int(counts.get("conflicts") or 0),
        "rule_matches": int(counts.get("rule_matches") or 0),
    }


def _skipped_auto_pair_summary(exc: BaseException) -> dict[str, Any]:
    code = exc.code if isinstance(exc, AppError) else "auto_pair_failed"
    retryable = bool(exc.retryable) if isinstance(exc, AppError) else True
    return {
        "enabled": True,
        "applied": 0,
        "rules_applied": 0,
        "bulk_exact_applied": 0,
        "skipped": True,
        "error": {
            "code": code,
            "message": "Automatic pairing was skipped; journals were still processed.",
            "retryable": retryable,
        },
    }


def _auto_pair_before_journals(
    conn: sqlite3.Connection,
    job: Mapping[str, Any],
) -> dict[str, Any]:
    workspace_ref, profile_ref = _job_scope_refs(conn, job)
    before = _transfer_candidate_counts(
        suggest_transfer_candidates(conn, workspace_ref, profile_ref)
    )
    rules = apply_transfer_rules(conn, workspace_ref, profile_ref, commit=False)
    bulk_exact = bulk_pair_transfers(
        conn,
        workspace_ref,
        profile_ref,
        confidence="exact",
        commit=False,
    )
    remaining = _transfer_candidate_counts(
        suggest_transfer_candidates(conn, workspace_ref, profile_ref)
    )
    # Cache the post-pairing candidate count so the side-nav swaps hint can be
    # served cheaply (build_review_badges_snapshot reads it without re-running the
    # heavy matcher). This is the only writer, so the badge reflects the last
    # journal run: after a manual pair/dismiss the count can briefly over-state
    # until the next refresh reprocesses journals. Part of the atomic auto-pair +
    # journal step: committed on success, rolled back with the pairings if journal
    # processing fails.
    cache_swap_candidate_count(conn, workspace_ref, profile_ref, remaining["total"])
    rules_summary = (
        rules.get("summary") if isinstance(rules.get("summary"), AbcMapping) else {}
    )
    bulk_summary = (
        bulk_exact.get("summary")
        if isinstance(bulk_exact.get("summary"), AbcMapping)
        else {}
    )
    rules_applied = int(rules_summary.get("count") or 0)
    bulk_applied = int(bulk_summary.get("count") or 0)
    return {
        "enabled": True,
        "applied": rules_applied + bulk_applied,
        "rules_applied": rules_applied,
        "bulk_exact_applied": bulk_applied,
        "skipped_conflicts": int(bulk_summary.get("skipped_conflicts") or 0),
        "total_swap_fee_msat": int(rules_summary.get("total_swap_fee_msat") or 0)
        + int(bulk_summary.get("total_swap_fee_msat") or 0),
        "before": before,
        "remaining": remaining,
    }


def _active_profile_row(conn: sqlite3.Connection) -> sqlite3.Row | None:
    context = current_context_snapshot(conn)
    profile_id = context.get("profile_id")
    if not profile_id:
        return None
    return conn.execute("SELECT * FROM profiles WHERE id = ?", (profile_id,)).fetchone()


def _row_int(row: sqlite3.Row, key: str, default: int = 0) -> int:
    try:
        if key not in row.keys():
            return default
        value = row[key]
    except (IndexError, KeyError):
        return default
    return int(value or default)


def _profile_require_coarse_review(row: sqlite3.Row) -> bool:
    try:
        if "require_coarse_review" not in row.keys():
            return False
        return bool(row["require_coarse_review"])
    except (IndexError, KeyError):
        return False


def _wallet_lookup_sql(wallet_ref: str | None = None) -> tuple[str, tuple[Any, ...]]:
    if wallet_ref:
        return (
            """
            SELECT *
            FROM wallets
            WHERE profile_id = ?
              AND (id = ? OR lower(label) = lower(?))
            ORDER BY label ASC
            """,
            (wallet_ref, wallet_ref),
        )
    return (
        """
        SELECT *
        FROM wallets
        WHERE profile_id = ?
        ORDER BY label ASC
        """,
        (),
    )


def _load_wallets_for_freshness(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_ref: str | None = None,
) -> list[sqlite3.Row]:
    sql, extra = _wallet_lookup_sql(wallet_ref)
    rows = conn.execute(sql, (profile_id, *extra)).fetchall()
    if wallet_ref and not rows:
        raise AppError(
            f"Wallet '{wallet_ref}' was not found",
            code="not_found",
            retryable=False,
        )
    return list(rows)


def _wallet_has_onchain_source(wallet: Mapping[str, Any], config: Mapping[str, Any]) -> bool:
    if str(wallet["kind"]) == "coreln":
        return False
    if config.get("source_file") and config.get("source_format"):
        return False
    if str_or_none(config.get("descriptor")):
        return True
    addresses = core_wallets.normalize_addresses(config.get("addresses"))
    return bool(addresses)


def _freshness_wallet_source_specs(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_ref: str | None = None,
    include_rates: bool = True,
    include_journals: bool = True,
    auto_pair_before_journals: bool = False,
    force_full: bool = False,
) -> list[dict[str, Any]]:
    specs: list[dict[str, Any]] = []

    def wallet_payload(wallet: Mapping[str, Any]) -> dict[str, Any]:
        payload = {"wallet_id": wallet["id"], "wallet_label": wallet["label"]}
        if force_full:
            payload["force_full"] = True
        return payload

    for wallet in _load_wallets_for_freshness(conn, profile_id, wallet_ref):
        config = json.loads(wallet["config_json"] or "{}")
        if core_wallets.wallet_btcpay_sync_config(config):
            specs.append(
                {
                    "job_type": core_freshness.JOB_BTCPAY_WALLET,
                    "source_type": core_freshness.SOURCE_BTCPAY_WALLET,
                    "source_key": core_freshness.source_key(
                        core_freshness.SOURCE_BTCPAY_WALLET,
                        wallet["id"],
                    ),
                    "source_label": f"{wallet['label']} BTCPay wallet source",
                    "payload": wallet_payload(wallet),
                    "priority": 20,
                    "single_flight": not force_full,
                }
            )
        if _wallet_has_onchain_source(wallet, config):
            specs.append(
                {
                    "job_type": core_freshness.JOB_ONCHAIN_WALLET,
                    "source_type": core_freshness.SOURCE_ONCHAIN,
                    "source_key": core_freshness.source_key(
                        core_freshness.SOURCE_ONCHAIN,
                        wallet["id"],
                    ),
                    "source_label": f"{wallet['label']} on-chain history",
                    "payload": wallet_payload(wallet),
                    "priority": 30,
                    "single_flight": not force_full,
                }
            )
        if core_wallets.wallet_btcpay_provenance_config(config):
            specs.append(
                {
                    "job_type": core_freshness.JOB_BTCPAY_PROVENANCE,
                    "source_type": core_freshness.SOURCE_BTCPAY_PROVENANCE,
                    "source_key": core_freshness.source_key(
                        core_freshness.SOURCE_BTCPAY_PROVENANCE,
                        wallet["id"],
                    ),
                    "source_label": f"{wallet['label']} BTCPay provenance",
                    "payload": wallet_payload(wallet),
                    "priority": 40,
                    "single_flight": not force_full,
                }
            )
    if include_rates:
        specs.append(
            {
                "job_type": core_freshness.JOB_MARKET_RATES,
                "source_type": core_freshness.SOURCE_RATES,
                "source_key": core_freshness.rate_source_key(profile_id),
                "source_label": "Market-rate coverage",
                "payload": {},
                "priority": 70,
            }
        )
    if include_journals:
        journal_payload = {"auto_pair": True} if auto_pair_before_journals else {}
        specs.append(
            {
                "job_type": core_freshness.JOB_JOURNAL_REFRESH,
                "source_type": core_freshness.SOURCE_JOURNALS,
                "source_key": core_freshness.journal_source_key(profile_id),
                "source_label": "Journal refresh",
                "payload": journal_payload,
                "priority": 80,
            }
        )
    return specs


def _source_checkpoint(
    conn: sqlite3.Connection,
    profile_id: str,
    source_key: str,
) -> dict[str, Any]:
    state = core_freshness.get_source_state(conn, profile_id, source_key)
    checkpoint = (state or {}).get("checkpoint") if isinstance(state, dict) else None
    return dict(checkpoint) if isinstance(checkpoint, dict) else {}


def _job_force_full(job: Mapping[str, Any]) -> bool:
    payload = job.get("payload") if isinstance(job.get("payload"), dict) else {}
    return bool(payload.get("force_full"))


def _source_checkpoint_for_job(
    conn: sqlite3.Connection,
    profile_id: str,
    source_key: str,
    job: Mapping[str, Any],
) -> dict[str, Any]:
    return {} if _job_force_full(job) else _source_checkpoint(conn, profile_id, source_key)


def _wallet_with_freshness_checkpoint(
    wallet: Mapping[str, Any],
    checkpoint: Mapping[str, Any],
) -> dict[str, Any]:
    return {**dict(wallet), "_freshness_checkpoint": dict(checkpoint)}


def _mark_daemon_wallet_synced(conn: sqlite3.Connection, wallet: Mapping[str, Any]) -> None:
    config = json.loads(wallet["config_json"] or "{}")
    config["last_synced_at"] = now_iso()
    conn.execute(
        "UPDATE wallets SET config_json = ? WHERE id = ?",
        (json.dumps(config, sort_keys=True), wallet["id"]),
    )


def _load_freshness_profile_wallet(
    conn: sqlite3.Connection,
    job: Mapping[str, Any],
) -> tuple[sqlite3.Row, sqlite3.Row]:
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (job["profile_id"],),
    ).fetchone()
    if profile is None:
        raise AppError("Freshness job profile was not found", code="not_found")
    payload = job.get("payload") if isinstance(job.get("payload"), dict) else {}
    wallet_id = payload.get("wallet_id")
    wallet = conn.execute(
        "SELECT * FROM wallets WHERE profile_id = ? AND id = ?",
        (job["profile_id"], wallet_id),
    ).fetchone()
    if wallet is None:
        raise AppError("Freshness job wallet was not found", code="not_found")
    return profile, wallet


def _freshness_handlers(runtime_config: dict[str, object]) -> Mapping[str, core_freshness.JobHandler]:
    def onchain_wallet(
        conn: sqlite3.Connection,
        job: Mapping[str, Any],
        progress: Callable[[Mapping[str, Any]], None],
        check_cancelled: Callable[[], None],
    ) -> Mapping[str, Any]:
        profile, wallet = _load_freshness_profile_wallet(conn, job)
        force_full = _job_force_full(job)
        checkpoint = _source_checkpoint_for_job(conn, profile["id"], job["source_key"], job)
        wallet_with_checkpoint = _wallet_with_freshness_checkpoint(wallet, checkpoint)
        progress({"phase": core_freshness.PHASE_DISCOVERY, "wallet": wallet["label"]})
        check_cancelled()
        progress({"phase": core_freshness.PHASE_BACKEND_FETCH, "wallet": wallet["label"]})
        token = sync_progress_emitter.set(progress)
        try:
            outcome = sync_wallet_from_backend(
                conn,
                runtime_config,
                None,
                None,
                wallet_with_checkpoint,
                checkpoint=checkpoint,
                force_full=force_full,
            )
        finally:
            sync_progress_emitter.reset(token)
        check_cancelled()
        progress({"phase": core_freshness.PHASE_IMPORT, "wallet": wallet["label"]})
        _mark_daemon_wallet_synced(conn, wallet)
        conn.commit()
        return {"wallet": wallet["label"], "status": "synced", **outcome}

    def btcpay_wallet(
        conn: sqlite3.Connection,
        job: Mapping[str, Any],
        progress: Callable[[Mapping[str, Any]], None],
        check_cancelled: Callable[[], None],
    ) -> Mapping[str, Any]:
        profile, wallet = _load_freshness_profile_wallet(conn, job)
        force_full = _job_force_full(job)
        checkpoint = _source_checkpoint_for_job(conn, profile["id"], job["source_key"], job)
        wallet_with_checkpoint = _wallet_with_freshness_checkpoint(wallet, checkpoint)
        progress({"phase": core_freshness.PHASE_DISCOVERY, "wallet": wallet["label"]})
        check_cancelled()
        progress({"phase": core_freshness.PHASE_BACKEND_FETCH, "wallet": wallet["label"]})
        token = sync_progress_emitter.set(progress)
        try:
            outcome = sync_configured_btcpay_wallet(
                conn,
                runtime_config,
                profile,
                wallet_with_checkpoint,
            )
        finally:
            sync_progress_emitter.reset(token)
        check_cancelled()
        progress({"phase": core_freshness.PHASE_IMPORT, "wallet": wallet["label"]})
        _mark_daemon_wallet_synced(conn, wallet)
        conn.commit()
        if force_full:
            outcome = {"force_full": True, **dict(outcome)}
        return {"wallet": wallet["label"], "status": "synced", **outcome}

    def btcpay_provenance(
        conn: sqlite3.Connection,
        job: Mapping[str, Any],
        progress: Callable[[Mapping[str, Any]], None],
        check_cancelled: Callable[[], None],
    ) -> Mapping[str, Any]:
        profile, wallet = _load_freshness_profile_wallet(conn, job)
        force_full = _job_force_full(job)
        checkpoint = _source_checkpoint_for_job(conn, profile["id"], job["source_key"], job)
        wallet_with_checkpoint = _wallet_with_freshness_checkpoint(wallet, checkpoint)
        progress({"phase": core_freshness.PHASE_BACKEND_FETCH, "wallet": wallet["label"]})
        token = sync_progress_emitter.set(progress)
        try:
            outcome = enrich_wallet_from_btcpay_provenance(
                conn,
                runtime_config,
                profile,
                wallet_with_checkpoint,
            )
        finally:
            sync_progress_emitter.reset(token)
        check_cancelled()
        progress({"phase": core_freshness.PHASE_DECODE_ENRICH, "wallet": wallet["label"]})
        conn.commit()
        if force_full:
            outcome = {"force_full": True, **dict(outcome)}
        return {"wallet": wallet["label"], "status": "synced", **outcome}

    def market_rates(
        conn: sqlite3.Connection,
        job: Mapping[str, Any],
        progress: Callable[[Mapping[str, Any]], None],
        check_cancelled: Callable[[], None],
    ) -> Mapping[str, Any]:
        progress({"phase": core_freshness.PHASE_RATE_COVERAGE})
        check_cancelled()
        # The bundled Kraken daily seed is an offline, idempotent local-cache
        # fill, so it always runs regardless of the market-rate policy.
        archive_path, seed_summary = core_rates.ensure_bundled_kraken_btc_daily_seed(
            conn,
            commit=True,
        )
        check_cancelled()
        provider = core_rates.get_market_rate_provider(conn)
        bundled_seed = {
            "source": core_rates.RATE_SOURCE_KRAKEN_CSV,
            "path": archive_path,
            "summary": seed_summary,
        }
        # Defense in depth: the foreground/background enqueue paths already skip
        # the market-rate source when the market_rates source class is disabled,
        # but gate the live provider call here too so no enqueue path can ever
        # reach a hardcoded rate provider (Coinbase Exchange / CoinGecko /
        # mempool) for a profile that turned market-rate refresh off.
        profile_id = job.get("profile_id") if isinstance(job, Mapping) else None
        policy = (
            core_freshness.get_policy(conn, str(profile_id))
            if profile_id
            else core_freshness.default_policy()
        )
        if not policy.source_classes.get(core_freshness.SOURCE_RATES, False):
            return {
                "status": "synced",
                "provider": provider,
                "live_refresh": False,
                "skipped_reason": "market_rates_disabled",
                "bundled_seed": bundled_seed,
                "latest": [],
                "sync": [],
            }
        latest_summary = core_rates.sync_latest_rates(
            conn,
            source=provider,
            commit=True,
        )
        check_cancelled()
        transaction_sync_providers = {
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            core_rates.RATE_SOURCE_MEMPOOL,
        }
        summary = (
            core_rates.sync_rates(
                conn,
                source=provider,
                commit=True,
                warm_cache_when_idle=False,
            )
            if provider in transaction_sync_providers
            else []
        )
        return {
            "status": "synced",
            "provider": provider,
            "live_refresh": True,
            "bundled_seed": bundled_seed,
            "latest": latest_summary,
            "sync": summary,
        }

    def journal_refresh(
        conn: sqlite3.Connection,
        job: Mapping[str, Any],
        progress: Callable[[Mapping[str, Any]], None],
        check_cancelled: Callable[[], None],
    ) -> Mapping[str, Any]:
        job_payload = job.get("payload") if isinstance(job.get("payload"), dict) else {}
        auto_pair_requested = bool(job_payload.get("auto_pair"))
        if auto_pair_requested:
            progress({"phase": "auto_pair"})
        # Emit the journal-refresh phase BEFORE any auto-pair inserts. run_job's
        # progress callback COMMITS the connection, so emitting it after the
        # commit=False pair inserts would commit them prematurely and defeat the
        # rollback below. After this point no committing progress is issued until
        # process_journals, so the auto-pair + journal step stays atomic.
        progress({"phase": core_freshness.PHASE_JOURNAL_REFRESH})
        auto_pair = None
        try:
            check_cancelled()
            if auto_pair_requested:
                try:
                    auto_pair = _auto_pair_before_journals(conn, job)
                except AppError as exc:
                    conn.rollback()
                    _LOGGER.warning(
                        "Automatic pairing before journal refresh was skipped: %s",
                        exc.code,
                    )
                    auto_pair = _skipped_auto_pair_summary(exc)
                except Exception as exc:
                    conn.rollback()
                    _LOGGER.exception("Automatic pairing before journal refresh was skipped")
                    auto_pair = _skipped_auto_pair_summary(exc)
            payload = _journals_process_payload(conn)
        except Exception:
            # The auto-pair inserts above are pending (commit=False). If journal
            # processing fails or is cancelled, run_job's error/cancel handler
            # would otherwise commit the connection — persisting those pairs (and
            # the journal invalidation) for a refresh the user was told failed,
            # so the next retry would see already-paired legs without ever
            # getting the auto-pair summary. Roll back so the auto-pair + journal
            # step is atomic: either both land or neither does.
            conn.rollback()
            raise
        if auto_pair is not None:
            payload["auto_pair"] = auto_pair
        return {"status": "synced", **payload}

    return {
        core_freshness.JOB_ONCHAIN_WALLET: onchain_wallet,
        core_freshness.JOB_BTCPAY_WALLET: btcpay_wallet,
        core_freshness.JOB_BTCPAY_PROVENANCE: btcpay_provenance,
        core_freshness.JOB_MARKET_RATES: market_rates,
        core_freshness.JOB_JOURNAL_REFRESH: journal_refresh,
    }


def _freshness_status_payload(conn: sqlite3.Connection) -> dict[str, Any]:
    profile = _active_profile_row(conn)
    if profile is None:
        return {"profile": None, "policy": core_freshness.default_policy().to_payload(), "sources": [], "jobs": []}
    snapshot = _freshness_snapshot_for_ui(conn, profile["id"])
    return {
        "profile": {"id": profile["id"], "label": profile["label"]},
        **snapshot,
    }


def _active_market_rate_pair(profile: Mapping[str, Any] | None) -> str | None:
    if profile is None:
        return None
    try:
        fiat_currency = profile["fiat_currency"]
    except (KeyError, IndexError):
        fiat_currency = None
    return core_rates.transaction_rate_pair("BTC", fiat_currency)


def _market_rate_provider_settings(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    provider = core_rates.get_market_rate_provider(conn)
    return {
        "market_rate_provider": provider,
        "market_rate_providers": list(core_rates.LIVE_MARKET_RATE_SOURCES),
        "active_rate_pair": _active_market_rate_pair(profile),
        "market_rate_fiats": core_rates.spot_fiats_for_provider(provider),
    }


def _freshness_configure_payload(
    conn: sqlite3.Connection,
    raw_args: dict[str, Any],
) -> dict[str, Any]:
    unknown = sorted(
        set(raw_args)
        - {
            "auto_sync_before_report_reads",
            "background_enabled",
            "market_rate_provider",
            "report_read_sync",
            "source_classes",
        }
    )
    if unknown:
        raise AppError(
            "ui.freshness.configure received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    profile = _active_profile_row(conn)
    if profile is None:
        raise AppError("ui.freshness.configure requires an active profile", code="validation", retryable=False)
    source_classes = raw_args.get("source_classes")
    if source_classes is not None and not isinstance(source_classes, dict):
        raise AppError(
            "ui.freshness.configure source_classes must be an object",
            code="validation",
            details={"type": type(source_classes).__name__},
            retryable=False,
        )
    market_rate_provider = raw_args.get("market_rate_provider")
    if market_rate_provider is not None and not isinstance(market_rate_provider, str):
        raise AppError(
            "ui.freshness.configure market_rate_provider must be a string",
            code="validation",
            details={"type": type(market_rate_provider).__name__},
            retryable=False,
        )
    legacy_value = raw_args.get("auto_sync_before_report_reads")
    report_read_sync = raw_args.get("report_read_sync")
    if legacy_value is not None:
        if not isinstance(legacy_value, bool):
            raise AppError(
                "ui.freshness.configure auto_sync_before_report_reads must be a boolean",
                code="validation",
                details={"type": type(legacy_value).__name__},
                retryable=False,
            )
        report_read_sync = legacy_value
        source_classes = {
            core_freshness.SOURCE_ONCHAIN: legacy_value,
            core_freshness.SOURCE_BTCPAY_WALLET: legacy_value,
            core_freshness.SOURCE_BTCPAY_PROVENANCE: legacy_value,
        }
    if market_rate_provider is not None:
        core_rates.set_market_rate_provider(
            conn,
            market_rate_provider,
            commit=False,
        )
    policy = core_freshness.set_policy(
        conn,
        profile["id"],
        background_enabled=raw_args.get("background_enabled"),
        report_read_sync=report_read_sync,
        source_classes=source_classes,
    )
    conn.commit()
    return {
        "profile": {"id": profile["id"], "label": profile["label"]},
        "settings": {
            **policy.to_payload(),
            **_market_rate_provider_settings(conn, profile),
            "auto_sync_before_report_reads": policy.report_read_sync,
            "setting_key": core_freshness.policy_setting_key(profile["id"]),
        },
    }


def _enqueue_freshness_jobs(
    conn: sqlite3.Connection,
    profile_id: str,
    specs: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    jobs = []
    for spec in specs:
        job = core_freshness.enqueue_job(
            conn,
            profile_id=profile_id,
            job_type=spec["job_type"],
            source_key=spec["source_key"],
            source_type=spec["source_type"],
            source_label=spec["source_label"],
            payload=spec.get("payload") or {},
            priority=int(spec.get("priority") or 100),
            single_flight=bool(spec.get("single_flight", True)),
        )
        if job.get("job_type"):
            jobs.append(job)
    conn.commit()
    return jobs


def _filter_freshness_specs_by_policy(
    specs: list[dict[str, Any]],
    policy: core_freshness.FreshnessPolicy,
    *,
    force: bool = False,
) -> list[dict[str, Any]]:
    if force:
        return specs
    return [
        spec
        for spec in specs
        if policy.source_classes.get(spec["source_type"], False)
    ]


def _parse_freshness_timestamp(value: Any) -> datetime | None:
    return parse_iso_datetime_or_none(value)


def _freshness_background_source_due(
    conn: sqlite3.Connection,
    *,
    profile_id: str,
    spec: Mapping[str, Any],
    now: datetime,
) -> bool:
    active = conn.execute(
        """
        SELECT id
        FROM freshness_jobs
        WHERE profile_id = ?
          AND source_key = ?
          AND status IN ('queued', 'running', 'rate_limited')
        LIMIT 1
        """,
        (profile_id, spec["source_key"]),
    ).fetchone()
    if active is not None:
        return False
    state = core_freshness.get_source_state(conn, profile_id, str(spec["source_key"]))
    if state is None:
        return True
    if state.get("paused"):
        return False
    limited_until = _parse_freshness_timestamp(state.get("rate_limited_until"))
    if limited_until is not None and limited_until > now:
        return False
    status = state.get("status")
    if status in {
        core_freshness.STATUS_FAILED,
        core_freshness.STATUS_PARTIALLY_STALE,
        core_freshness.STATUS_BLOCKING_REPORTS,
        core_freshness.STATUS_RATE_LIMITED,
    }:
        return True
    last_success = _parse_freshness_timestamp(state.get("last_success_at"))
    if last_success is None:
        return True
    age_seconds = (now - last_success).total_seconds()
    refresh_interval = (
        FRESHNESS_BACKGROUND_RATE_REFRESH_INTERVAL_SECONDS
        if spec.get("source_type") == core_freshness.SOURCE_RATES
        else FRESHNESS_BACKGROUND_REFRESH_INTERVAL_SECONDS
    )
    return age_seconds >= refresh_interval


def _filter_freshness_specs_for_background(
    conn: sqlite3.Connection,
    profile_id: str,
    specs: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    now = datetime.now(timezone.utc)
    return [
        spec
        for spec in specs
        if _freshness_background_source_due(
            conn,
            profile_id=profile_id,
            spec=spec,
            now=now,
        )
    ]


def _emit_background_freshness_event(
    out: FreshnessOutputChannel,
    kind: str,
    payload: Mapping[str, Any],
) -> None:
    # The background worker has no originating request, so these records
    # must use the `event: true` envelope class — the desktop supervisor
    # kills the daemon over any other post-ready record without a
    # request_id.
    out.write(build_event_envelope(kind, core_freshness.redact_freshness_payload(dict(payload))))


def _freshness_background_tick(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    out: FreshnessOutputChannel,
) -> None:
    profile = _active_profile_row(conn)
    if profile is None:
        return
    profile_id = profile["id"]
    policy = core_freshness.get_policy(conn, profile_id)
    if not policy.background_enabled:
        return
    core_freshness.recover_interrupted_jobs(conn, profile_id=profile_id)
    conn.commit()
    specs = _freshness_wallet_source_specs(
        conn,
        profile_id,
        include_rates=policy.source_classes.get(core_freshness.SOURCE_RATES, False),
        include_journals=policy.source_classes.get(core_freshness.SOURCE_JOURNALS, False),
    )
    specs = _filter_freshness_specs_by_policy(specs, policy)
    specs = _filter_freshness_specs_for_background(conn, profile_id, specs)
    enqueued = _enqueue_freshness_jobs(conn, profile_id, specs)

    def _progress(payload: Mapping[str, Any]) -> None:
        _emit_background_freshness_event(
            out,
            "ui.freshness.progress",
            {"profile_id": profile_id, **dict(payload)},
        )

    completed = core_freshness.run_due_jobs(
        conn,
        _freshness_handlers(runtime_config),
        profile_id=profile_id,
        limit=1,
        progress_observer=_progress,
    )
    if enqueued or completed:
        _emit_background_freshness_event(
            out,
            "ui.freshness.background",
            {
                "profile": {"id": profile_id, "label": profile["label"]},
                "enqueued": enqueued,
                "completed": completed,
                **_freshness_snapshot_for_ui(conn, profile_id),
            },
        )


def _start_freshness_background_worker(
    ctx: FreshnessDaemonContext,
    *,
    passphrase: str | None = None,
) -> None:
    if ctx.freshness_worker is not None and ctx.freshness_worker.is_alive():
        return
    if ctx.freshness_stop_event.is_set():
        return
    if ctx.conn is None:
        return
    try:
        profile = _active_profile_row(ctx.conn)
        if profile is None:
            return
        policy = core_freshness.get_policy(ctx.conn, profile["id"])
        if not policy.background_enabled:
            return
    except sqlite3.Error:
        return
    passphrase_handoff: dict[str, str | None] = {
        "value": passphrase if passphrase is not None else ctx.db_passphrase
    }

    def _worker() -> None:
        current_request_id.set("background:freshness")
        worker_conn: sqlite3.Connection | None = None
        try:
            worker_conn = open_db(
                ctx.data_root,
                passphrase=passphrase_handoff.pop("value", None),
            )
            merge_db_backends(worker_conn, ctx.runtime_config)
        except AppError as exc:
            _LOGGER.warning("freshness worker unavailable", exc_info=exc)
            _emit_background_freshness_event(
                ctx.out,
                "ui.freshness.worker",
                {
                    "status": "unavailable",
                    "code": exc.code or "freshness_worker_unavailable",
                    "message": str(exc),
                    "hint": exc.hint,
                    "retryable": exc.retryable,
                },
            )
            return
        except Exception as exc:
            _LOGGER.error("freshness worker unavailable", exc_info=exc)
            _emit_background_freshness_event(
                ctx.out,
                "ui.freshness.worker",
                {
                    "status": "unavailable",
                    "code": "freshness_worker_unavailable",
                    "message": str(exc) or exc.__class__.__name__,
                    "retryable": True,
                },
            )
            return
        try:
            while not ctx.freshness_stop_event.wait(FRESHNESS_BACKGROUND_POLL_SECONDS):
                try:
                    _freshness_background_tick(worker_conn, ctx.runtime_config, ctx.out)
                except Exception as exc:
                    worker_conn.rollback()
                    _LOGGER.error("freshness background tick failed", exc_info=exc)
                    _emit_background_freshness_event(
                        ctx.out,
                        "ui.freshness.worker",
                        {
                            "status": "error",
                            "code": "freshness_background_error",
                            "message": str(exc) or exc.__class__.__name__,
                            "retryable": True,
                        },
                    )
        finally:
            worker_conn.close()

    worker = threading.Thread(
        target=_worker,
        daemon=True,
        name="kassiber-freshness-worker",
    )
    ctx.freshness_worker = worker
    worker.start()


def _stop_freshness_background_worker(
    ctx: FreshnessDaemonContext,
    *,
    cancel_running: bool = False,
    reset_event: bool = True,
) -> None:
    if cancel_running and ctx.conn is not None:
        try:
            ctx.conn.execute(
                """
                UPDATE freshness_jobs
                SET cancel_requested = 1, updated_at = ?
                WHERE status = 'running'
                """,
                (now_iso(),),
            )
            ctx.conn.commit()
        except sqlite3.Error:
            ctx.conn.rollback()
    ctx.freshness_stop_event.set()
    worker = ctx.freshness_worker
    if worker is not None:
        worker.join(timeout=2.0)
    if worker is None or not worker.is_alive():
        ctx.freshness_worker = None
    if reset_event:
        ctx.freshness_stop_event = threading.Event()


def _sync_results_from_freshness_jobs(jobs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for job in jobs:
        job_type = job.get("job_type")
        if job_type not in {
            core_freshness.JOB_ONCHAIN_WALLET,
            core_freshness.JOB_BTCPAY_WALLET,
            core_freshness.JOB_BTCPAY_PROVENANCE,
        }:
            continue
        if job.get("status") == core_freshness.JOB_DONE:
            result = job.get("result")
            if isinstance(result, dict):
                results.append(result)
            continue
        payload = job.get("payload") if isinstance(job.get("payload"), dict) else {}
        error = job.get("error") if isinstance(job.get("error"), dict) else {}
        results.append(
            {
                "wallet": payload.get("wallet_label") or job.get("source_label") or "Source",
                "status": "error",
                "code": error.get("code") or job.get("status"),
                "message": error.get("message") or "Refresh did not finish.",
                "hint": error.get("hint"),
                "details": error.get("details"),
                "retryable": bool(error.get("retryable")),
            }
        )
    return results


def _source_class_included_for_run(
    args: Mapping[str, Any],
    arg_name: str,
    policy: core_freshness.FreshnessPolicy,
    source_type: str,
) -> bool:
    policy_enabled = bool(policy.source_classes.get(source_type, False))
    requested = args.get(arg_name)
    if requested is None:
        return policy_enabled
    return bool(requested) and policy_enabled


def _freshness_run_payload(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    raw_args: dict[str, Any] | None = None,
    *,
    progress_observer: Callable[[Mapping[str, Any]], None] | None = None,
) -> dict[str, Any]:
    args = raw_args or {}
    unknown = sorted(
        set(args)
        - {
            "wallet",
            "all",
            "rates",
            "journals",
            "auto_pair",
            "run",
            "limit",
            "force_full",
        }
    )
    if unknown:
        raise AppError(
            "ui.freshness.run received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    profile = _active_profile_row(conn)
    if profile is None:
        return {"profile": None, "enqueued": [], "completed": [], **_freshness_snapshot_for_ui(conn, "")}
    wallet = args.get("wallet")
    if wallet is not None and (not isinstance(wallet, str) or not wallet.strip()):
        raise AppError("ui.freshness.run wallet must be a non-empty string", code="validation", retryable=False)
    sync_all = bool(args.get("all", wallet is None))
    policy = core_freshness.get_policy(conn, profile["id"])
    include_rates = _source_class_included_for_run(
        args,
        "rates",
        policy,
        core_freshness.SOURCE_RATES,
    )
    include_journals = _source_class_included_for_run(
        args,
        "journals",
        policy,
        core_freshness.SOURCE_JOURNALS,
    )
    auto_pair = args.get("auto_pair")
    if auto_pair is not None and not isinstance(auto_pair, bool):
        raise AppError(
            "ui.freshness.run auto_pair must be a boolean",
            code="validation",
            retryable=False,
        )
    auto_pair = bool(auto_pair)
    force_full = args.get("force_full")
    if force_full is not None and not isinstance(force_full, bool):
        raise AppError("ui.freshness.run force_full must be a boolean", code="validation", retryable=False)
    force_full = bool(force_full)
    recovered = core_freshness.recover_interrupted_jobs(conn, profile_id=profile["id"])
    if recovered:
        conn.commit()
    specs = _freshness_wallet_source_specs(
        conn,
        profile["id"],
        wallet_ref=None if sync_all else wallet.strip(),
        include_rates=include_rates,
        include_journals=include_journals,
        auto_pair_before_journals=auto_pair,
        force_full=force_full,
    )
    enqueued = _enqueue_freshness_jobs(conn, profile["id"], specs)
    completed: list[dict[str, Any]] = []
    if args.get("run", True):
        run_limit = int(args.get("limit") or max(1, len(enqueued)))
        run_total = max(1, min(run_limit, max(1, len(enqueued))))
        seen_job_ids: list[str] = []

        def _progress_with_run_context(payload: Mapping[str, Any]) -> None:
            if progress_observer is None:
                return
            job_id = str(payload.get("job_id") or "")
            if job_id and job_id not in seen_job_ids:
                seen_job_ids.append(job_id)
            job_index = (
                seen_job_ids.index(job_id) + 1
                if job_id in seen_job_ids
                else None
            )
            progress_observer(
                {
                    **dict(payload),
                    **(
                        {"job_index": job_index}
                        if job_index is not None
                        else {}
                    ),
                    "job_total": max(run_total, len(seen_job_ids)),
                }
            )

        completed = core_freshness.run_due_jobs(
            conn,
            _freshness_handlers(runtime_config),
            profile_id=profile["id"],
            limit=run_limit,
            progress_observer=(
                _progress_with_run_context
                if progress_observer is not None
                else None
            ),
        )
    snapshot = _freshness_snapshot_for_ui(conn, profile["id"])
    return {
        "profile": {"id": profile["id"], "label": profile["label"]},
        "results": _sync_results_from_freshness_jobs(completed),
        "force_full": force_full,
        "recovered": recovered,
        "enqueued": enqueued,
        "completed": completed,
        **snapshot,
    }


def _workspace_freshness_run_payload(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    raw_args: dict[str, Any] | None = None,
    *,
    progress_observer: Callable[[Mapping[str, Any]], None] | None = None,
) -> dict[str, Any]:
    args = raw_args or {}
    unknown = sorted(set(args) - {"workspace_id", "rates", "journals", "run", "limit"})
    if unknown:
        raise AppError(
            "ui.workspace.freshness.run received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    workspace_id = args.get("workspace_id")
    if not isinstance(workspace_id, str) or not workspace_id.strip():
        raise AppError(
            "ui.workspace.freshness.run requires args.workspace_id",
            code="validation",
            retryable=False,
        )
    workspace = conn.execute(
        "SELECT * FROM workspaces WHERE id = ?",
        (workspace_id.strip(),),
    ).fetchone()
    if workspace is None:
        raise AppError(
            f"Book set '{workspace_id}' was not found",
            code="not_found",
            retryable=False,
        )
    profile_rows = conn.execute(
        """
        SELECT *
        FROM profiles
        WHERE workspace_id = ?
        ORDER BY created_at ASC, label ASC
        """,
        (workspace["id"],),
    ).fetchall()
    run_now = bool(args.get("run", True))
    requested_limit = args.get("limit")
    books: list[dict[str, Any]] = []
    totals = {
        "books": len(profile_rows),
        "enqueued": 0,
        "completed": 0,
        "errors": 0,
        "rate_limited": 0,
        "blocked_books": 0,
        "synced_books": 0,
    }
    handlers = _freshness_handlers(runtime_config)
    for profile in profile_rows:
        policy = core_freshness.get_policy(conn, profile["id"])
        include_rates = _source_class_included_for_run(
            args,
            "rates",
            policy,
            core_freshness.SOURCE_RATES,
        )
        include_journals = _source_class_included_for_run(
            args,
            "journals",
            policy,
            core_freshness.SOURCE_JOURNALS,
        )
        recovered = core_freshness.recover_interrupted_jobs(
            conn,
            profile_id=profile["id"],
        )
        if recovered:
            conn.commit()
        specs = _freshness_wallet_source_specs(
            conn,
            profile["id"],
            include_rates=include_rates,
            include_journals=include_journals,
        )
        enqueued = _enqueue_freshness_jobs(conn, profile["id"], specs)

        def _book_progress(
            payload: Mapping[str, Any],
            *,
            profile_id: str = profile["id"],
            profile_label: str = profile["label"],
        ) -> None:
            if progress_observer is None:
                return
            progress_observer(
                {
                    "workspace": {"id": workspace["id"], "label": workspace["label"]},
                    "profile": {"id": profile_id, "label": profile_label},
                    **dict(payload),
                }
            )

        completed: list[dict[str, Any]] = []
        if run_now:
            limit = int(requested_limit or max(1, len(enqueued)))
            completed = core_freshness.run_due_jobs(
                conn,
                handlers,
                profile_id=profile["id"],
                limit=limit,
                progress_observer=_book_progress,
            )
        snapshot = _freshness_snapshot_for_ui(conn, profile["id"])
        result_errors = [
            job
            for job in completed
            if job.get("status")
            not in {
                core_freshness.JOB_DONE,
                core_freshness.JOB_CANCELLED,
                core_freshness.JOB_RATE_LIMITED,
            }
        ]
        rate_limited = int(snapshot.get("summary", {}).get("rate_limited") or 0)
        blocked = int(snapshot.get("summary", {}).get("blocking_reports") or 0)
        totals["enqueued"] += len(enqueued)
        totals["completed"] += len(completed)
        totals["errors"] += len(result_errors)
        totals["rate_limited"] += rate_limited
        if blocked:
            totals["blocked_books"] += 1
        else:
            totals["synced_books"] += 1
        books.append(
            {
                "profile": {"id": profile["id"], "label": profile["label"]},
                "results": _sync_results_from_freshness_jobs(completed),
                "recovered": recovered,
                "enqueued": enqueued,
                "completed": completed,
                "attention": {
                    "blockedReports": blocked > 0,
                    "rateLimited": rate_limited > 0,
                    "errors": len(result_errors),
                },
                **snapshot,
            }
        )
    return {
        "workspace": {"id": workspace["id"], "label": workspace["label"]},
        "books": books,
        "summary": {
            **totals,
            "ok": totals["errors"] == 0 and totals["blocked_books"] == 0,
            "reports_blocked": totals["blocked_books"],
        },
    }


def _freshness_control_payload(
    conn: sqlite3.Connection,
    raw_args: dict[str, Any],
    *,
    action: str,
) -> dict[str, Any]:
    profile = _active_profile_row(conn)
    if profile is None:
        raise AppError(f"ui.freshness.{action} requires an active profile", code="validation", retryable=False)
    if action == "cancel":
        job_id = raw_args.get("job_id")
        if not isinstance(job_id, str) or not job_id.strip():
            raise AppError("ui.freshness.cancel requires args.job_id", code="validation", retryable=False)
        result = core_freshness.cancel_job(conn, job_id.strip())
    else:
        source_key = raw_args.get("source_key")
        if not isinstance(source_key, str) or not source_key.strip():
            raise AppError(f"ui.freshness.{action} requires args.source_key", code="validation", retryable=False)
        if action == "pause":
            result = core_freshness.pause_source(conn, profile["id"], source_key.strip())
        elif action == "resume":
            result = core_freshness.resume_source(conn, profile["id"], source_key.strip())
        else:
            raise AppError(f"Unsupported freshness control action '{action}'", code="validation", retryable=False)
    conn.commit()
    return {"result": result, **_freshness_status_payload(conn)}


def _maintenance_settings_payload(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    profile = _active_profile_row(conn)
    if profile is None:
        return {
            "workspace": context.get("workspace_label") or None,
            "profile": None,
            "settings": {
                **core_freshness.default_policy().to_payload(),
                **_market_rate_provider_settings(conn),
                "active_rate_pair": None,
                "auto_sync_before_report_reads": False,
            },
            "freshness": {"sources": [], "jobs": []},
        }
    policy = core_freshness.get_policy(conn, profile["id"])
    snapshot = _freshness_snapshot_for_ui(conn, profile["id"])
    return {
        "workspace": context.get("workspace_label") or None,
        "profile": {
            "id": profile["id"],
            "label": profile["label"],
        },
        "settings": {
            **policy.to_payload(),
            **_market_rate_provider_settings(conn, profile),
            "auto_sync_before_report_reads": policy.report_read_sync,
            "require_coarse_review": _profile_require_coarse_review(profile),
            "coarse_priced_count": core_rates.count_coarse_priced_transactions(
                conn, profile["id"]
            ),
            "setting_key": core_freshness.policy_setting_key(profile["id"]),
        },
        "freshness": snapshot,
    }


def _maintenance_configure_payload(
    conn: sqlite3.Connection,
    raw_args: dict[str, Any],
) -> dict[str, Any]:
    unknown = sorted(
        set(raw_args)
        - {
            "auto_sync_before_report_reads",
            "background_enabled",
            "market_rate_provider",
            "report_read_sync",
            "require_coarse_review",
            "source_classes",
        }
    )
    if unknown:
        raise AppError(
            "ui.maintenance.configure received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    profile = _active_profile_row(conn)
    if profile is None:
        raise AppError(
            "ui.maintenance.configure requires an active profile",
            code="validation",
            retryable=False,
        )
    require_coarse_review = raw_args.get("require_coarse_review")
    if require_coarse_review is not None:
        if not isinstance(require_coarse_review, bool):
            raise AppError(
                "ui.maintenance.configure require_coarse_review must be a boolean",
                code="validation",
                details={"type": type(require_coarse_review).__name__},
                retryable=False,
            )
        # Reuse update_profile so the change is journal-invalidated consistently.
        from .core import accounts as core_accounts

        core_accounts.update_profile(
            conn,
            profile["workspace_id"],
            profile["id"],
            {"require_coarse_review": require_coarse_review},
        )
    freshness_args = {k: v for k, v in raw_args.items() if k != "require_coarse_review"}
    payload = _freshness_configure_payload(conn, freshness_args)
    return {**_maintenance_settings_payload(conn), "configured": payload["settings"]}


def _auto_process_journals_if_needed(conn: sqlite3.Connection) -> dict[str, Any] | None:
    profile = _active_profile_row(conn)
    if profile is None:
        return None
    profile_id = profile["id"]
    active_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile_id,),
    ).fetchone()["count"]
    active_count = int(active_count or 0)
    if active_count == 0:
        return None
    if (
        profile["last_processed_at"]
        and _row_int(profile, "last_processed_tx_count") == active_count
        and _row_int(profile, "last_processed_input_version")
        == _row_int(profile, "journal_input_version")
    ):
        return None
    return _journals_process_payload(conn)


def _auto_sync_wallets_if_enabled(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    *,
    state: dict[str, Any] | None = None,
    force: bool = False,
) -> dict[str, Any] | None:
    state = state if state is not None else {}
    if state.get("auto_sync_attempted") and not force:
        return None
    profile = _active_profile_row(conn)
    if profile is None:
        return None
    policy = core_freshness.get_policy(conn, profile["id"])
    enabled = policy.report_read_sync
    if not enabled and not force:
        return None
    state["auto_sync_attempted"] = True
    if not force:
        now = time.monotonic()
        with _AUTO_SYNC_PROFILE_LOCK:
            last_attempt = _AUTO_SYNC_PROFILE_LAST_ATTEMPT.get(profile["id"])
            if (
                last_attempt is not None
                and now - last_attempt < AUTO_SYNC_PROFILE_MIN_INTERVAL_SECONDS
            ):
                cached = _AUTO_SYNC_PROFILE_LAST_RESULT.get(profile["id"])
                if cached is None:
                    payload = {
                        "ok": True,
                        "status": "skipped",
                        "reason": "auto_sync_rate_limited",
                    }
                else:
                    payload = json.loads(json.dumps(cached))
                    payload["status"] = "cached"
                    payload["reason"] = "auto_sync_rate_limited"
                payload["retry_after_seconds"] = int(
                    AUTO_SYNC_PROFILE_MIN_INTERVAL_SECONDS - (now - last_attempt)
                )
                ok = not _sync_payload_has_errors(payload)
                state["auto_sync"] = {"ok": ok, "payload": payload}
                return payload
            _AUTO_SYNC_PROFILE_LAST_ATTEMPT[profile["id"]] = now
    try:
        if "default_backend" not in runtime_config and not runtime_config.get("backends"):
            payload = _wallets_sync_payload(
                conn,
                runtime_config,
                {"all": True},
                strict=False,
            )
            payload = _redact_sync_payload_for_ui(payload)
            ok = not _sync_payload_has_errors(payload)
            payload["ok"] = ok
            state["auto_sync"] = {"ok": ok, "payload": payload}
            if not force:
                with _AUTO_SYNC_PROFILE_LOCK:
                    _AUTO_SYNC_PROFILE_LAST_RESULT[profile["id"]] = dict(payload)
            return payload
        specs = _freshness_wallet_source_specs(
            conn,
            profile["id"],
            include_rates=policy.source_classes.get(core_freshness.SOURCE_RATES, False),
            include_journals=False,
        )
        specs = _filter_freshness_specs_by_policy(specs, policy, force=force)
        enqueued = _enqueue_freshness_jobs(conn, profile["id"], specs)
        completed = core_freshness.run_due_jobs(
            conn,
            _freshness_handlers(runtime_config),
            profile_id=profile["id"],
            limit=max(1, len(enqueued)),
        )
        payload = {
            "results": _sync_results_from_freshness_jobs(completed),
            "enqueued": enqueued,
            "completed": completed,
            "freshness": _freshness_snapshot_for_ui(conn, profile["id"]),
        }
        payload = _redact_sync_payload_for_ui(payload)
        ok = not _sync_payload_has_errors(payload)
        payload["ok"] = ok
        state["auto_sync"] = {"ok": ok, "payload": payload}
        if not force:
            with _AUTO_SYNC_PROFILE_LOCK:
                _AUTO_SYNC_PROFILE_LAST_RESULT[profile["id"]] = dict(payload)
        return payload
    except AppError as exc:
        payload = {
            "ok": False,
            "reason": exc.code or "sync_failed",
            "message": str(exc),
        }
        state["auto_sync"] = payload
        if not force:
            with _AUTO_SYNC_PROFILE_LOCK:
                _AUTO_SYNC_PROFILE_LAST_RESULT[profile["id"]] = dict(payload)
        return payload


def _auto_maintain_for_read(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    *,
    state: dict[str, Any] | None = None,
    sync_if_enabled: bool = True,
) -> dict[str, Any]:
    metadata: dict[str, Any] = {}
    if sync_if_enabled:
        auto_sync = _auto_sync_wallets_if_enabled(conn, runtime_config, state=state)
        if auto_sync is not None:
            metadata["auto_sync"] = build_envelope("ui.wallets.sync", auto_sync)
    auto_journal_process = _auto_process_journals_if_needed(conn)
    if auto_journal_process is not None:
        if state is not None:
            state["auto_journal_process"] = auto_journal_process
        metadata["auto_journal_process"] = build_envelope(
            "ui.journals.process",
            auto_journal_process,
        )
    return metadata


def _maintenance_run_payload(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
    raw_args: dict[str, Any] | None = None,
    *,
    state: dict[str, Any] | None = None,
) -> dict[str, Any]:
    args = raw_args or {}
    unknown = sorted(set(args) - {"sync"})
    if unknown:
        raise AppError(
            "ui.maintenance.run received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    sync_mode = args.get("sync", "if_enabled")
    if sync_mode not in {"never", "if_enabled", "always"}:
        raise AppError(
            "ui.maintenance.run sync must be never, if_enabled, or always",
            code="validation",
            details={"sync": sync_mode},
            retryable=False,
        )
    metadata: dict[str, Any] = {}
    sync_payload: dict[str, Any] | None = None
    if sync_mode == "always":
        auto_sync = _auto_sync_wallets_if_enabled(
            conn,
            runtime_config,
            state=state,
            force=True,
        )
        if auto_sync is not None:
            sync_payload = auto_sync
            metadata["sync"] = build_envelope("ui.wallets.sync", auto_sync)
    elif sync_mode == "if_enabled":
        auto_sync = _auto_sync_wallets_if_enabled(conn, runtime_config, state=state)
        if auto_sync is not None:
            sync_payload = auto_sync
            metadata["sync"] = build_envelope("ui.wallets.sync", auto_sync)
    journal_process = _auto_process_journals_if_needed(conn)
    if journal_process is not None:
        metadata["journals"] = build_envelope("ui.journals.process", journal_process)
    blockers = _apply_sync_failure_blocker(
        build_report_blockers_snapshot(conn),
        sync_payload,
    )
    return {
        "ready": blockers["ready"],
        "sync_mode": sync_mode,
        "maintenance": metadata,
        "blockers": blockers["blockers"],
        "health": blockers["health"],
        "settings": _maintenance_settings_payload(conn)["settings"],
    }
