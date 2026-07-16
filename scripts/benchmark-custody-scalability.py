#!/usr/bin/env python3
"""Reproducible custody scalability benchmark.

The normal test suite proves query-plan and traversal invariants without hard
wall-clock thresholds.  This script adds comparable timings at realistic book
sizes.  It creates disposable Kassiber databases under the system temporary
directory, inserts in fixed-size batches, and emits one JSON object per line so
an interrupted long run still leaves machine-readable results.

Examples:

    .venv/bin/python scripts/benchmark-custody-scalability.py --smoke
    .venv/bin/python scripts/benchmark-custody-scalability.py \
        --sizes 100000,250000,500000,1000000

Timing values are observations, not pass/fail thresholds.  The ``invariants``
fields are the regression contract: atomic validation traverses decisions a
constant number of times, lineage pages use keyset cursors and the matching
indexes, and structured gap candidates survive large-book capacity limits.
"""

from __future__ import annotations

import argparse
import gc
import itertools
import json
import platform
import sqlite3
import sys
import tempfile
import time
from collections.abc import Iterable, Iterator, Sequence
from pathlib import Path
from typing import Any, TypeVar


REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from kassiber.core.custody_gaps import build_gap_snapshot
from kassiber.core.custody_quantity import (
    ArbitratedSlice,
    ClaimPriority,
    INTERNAL_REVIEWED,
    QuantityClaim,
    QuantitySlice,
    _fail_closed_atomic_bundles,
)
from kassiber.core.custody_quantity_store import custody_decision_rows
from kassiber.db import open_db


PROFILE_ID = "custody-scale-profile"
WORKSPACE_ID = "custody-scale-workspace"
SOURCE_WALLET_ID = "custody-scale-source"
TARGET_WALLET_ID = "custody-scale-target"
BTC_MSAT = 100_000_000_000
_T = TypeVar("_T")


class _CountingSequence(Sequence[ArbitratedSlice]):
    """Count traversal without changing the arbiter's sequence contract."""

    def __init__(self, rows: list[ArbitratedSlice]) -> None:
        self._rows = rows
        self.yield_count = 0

    def __len__(self) -> int:
        return len(self._rows)

    def __getitem__(self, index: int) -> ArbitratedSlice:
        return self._rows[index]

    def __iter__(self) -> Iterator[ArbitratedSlice]:
        for row in self._rows:
            self.yield_count += 1
            yield row


def _batches(rows: Iterable[_T], size: int) -> Iterator[list[_T]]:
    iterator = iter(rows)
    while batch := list(itertools.islice(iterator, size)):
        yield batch


def _timed(callable_: Any) -> tuple[Any, float]:
    started = time.perf_counter()
    result = callable_()
    return result, time.perf_counter() - started


def _seed_scope(conn: sqlite3.Connection) -> None:
    conn.execute(
        "INSERT INTO workspaces(id, label, created_at) VALUES (?, ?, ?)",
        (WORKSPACE_ID, "Custody scale workspace", "2026-01-01T00:00:00Z"),
    )
    conn.execute(
        """
        INSERT INTO profiles(id, workspace_id, label, created_at)
        VALUES (?, ?, ?, ?)
        """,
        (
            PROFILE_ID,
            WORKSPACE_ID,
            "Custody scale profile",
            "2026-01-01T00:00:00Z",
        ),
    )
    conn.executemany(
        """
        INSERT INTO wallets(
            id, workspace_id, profile_id, label, kind, created_at
        ) VALUES (?, ?, ?, ?, ?, ?)
        """,
        [
            (
                SOURCE_WALLET_ID,
                WORKSPACE_ID,
                PROFILE_ID,
                "Scale source",
                "descriptor",
                "2026-01-01T00:00:00Z",
            ),
            (
                TARGET_WALLET_ID,
                WORKSPACE_ID,
                PROFILE_ID,
                "Scale target",
                "descriptor",
                "2026-01-01T00:00:00Z",
            ),
        ],
    )
    conn.commit()


def benchmark_atomic(transaction_count: int, decision_ratio: float) -> dict[str, Any]:
    decision_count = max(1, int(transaction_count * decision_ratio))
    claims: list[QuantityClaim] = []
    decisions: list[ArbitratedSlice] = []
    for index in range(decision_count):
        source = QuantitySlice(f"source-{index}", 0, 1)
        target = QuantitySlice(f"target-{index}", 0, 1)
        claim_id = f"claim-{index}"
        bundle_id = f"bundle-{index}"
        claims.append(
            QuantityClaim(
                claim_id=claim_id,
                source=source,
                target=target,
                state=INTERNAL_REVIEWED,
                priority=ClaimPriority.REVIEWED_COMPONENT,
                reason="scale_benchmark",
                atomic_bundle_id=bundle_id,
            )
        )
        decisions.append(
            ArbitratedSlice(
                source=source,
                target=target,
                state=INTERNAL_REVIEWED,
                reason="scale_benchmark",
                selected_claim_id=claim_id,
                atomic_bundle_id=bundle_id,
            )
        )

    counted = _CountingSequence(decisions)
    result, elapsed = _timed(
        lambda: _fail_closed_atomic_bundles(counted, claims)
    )
    valid_count = sum(row.state == INTERNAL_REVIEWED for row in result)
    expected_max_traversals = decision_count * 2
    return {
        "decision_count": decision_count,
        "elapsed_seconds": round(elapsed, 6),
        "decisions_per_second": round(decision_count / elapsed, 2),
        "result_count": len(result),
        "valid_count": valid_count,
        "decision_rows_yielded": counted.yield_count,
        "invariants": {
            "all_bundles_remain_selected": valid_count == decision_count,
            "constant_decision_traversals": (
                counted.yield_count <= expected_max_traversals
            ),
            "maximum_expected_decision_rows_yielded": expected_max_traversals,
        },
    }


def _lineage_transaction_rows(transaction_count: int) -> Iterator[tuple[Any, ...]]:
    for index in range(transaction_count):
        wallet_id = SOURCE_WALLET_ID if index % 2 == 0 else TARGET_WALLET_ID
        yield (
            f"lineage-tx-{index}",
            WORKSPACE_ID,
            PROFILE_ID,
            wallet_id,
            f"lineage-fingerprint-{index}",
            f"2026-01-01T00:00:00.{index % 1_000_000:06d}Z",
            "outbound" if index % 2 == 0 else "inbound",
            "BTC",
            1,
            "{}",
            "2026-01-01T00:00:00Z",
        )


def _lineage_decision_rows(decision_count: int) -> Iterator[tuple[Any, ...]]:
    for index in range(decision_count):
        source_transaction = f"lineage-tx-{index * 2}"
        target_transaction = f"lineage-tx-{index * 2 + 1}"
        decision_id = f"{index:064x}"
        observation_hash = f"{index + 1:064x}"
        occurred_at = f"2026-01-01T00:00:00.{index % 1_000_000:06d}Z"
        yield (
            decision_id,
            WORKSPACE_ID,
            PROFILE_ID,
            source_transaction,
            target_transaction,
            observation_hash,
            0,
            1,
            observation_hash,
            0,
            1,
            SOURCE_WALLET_ID,
            TARGET_WALLET_ID,
            "main",
            "main",
            "bitcoin",
            "bitcoin",
            "BTC",
            "BTC",
            INTERNAL_REVIEWED,
            "eligible",
            "scale_benchmark",
            occurred_at,
            occurred_at,
            "2026-01-01T00:00:00Z",
        )


def _seed_lineage(
    conn: sqlite3.Connection,
    transaction_count: int,
    decision_count: int,
    batch_size: int,
) -> float:
    started = time.perf_counter()
    transaction_sql = """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
            direction, asset, amount, raw_json, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """
    decision_sql = """
        INSERT INTO journal_custody_decisions(
            decision_id, workspace_id, profile_id,
            source_transaction_id, target_transaction_id,
            source_observation_hash, source_start_msat, source_end_msat,
            target_observation_hash, target_start_msat, target_end_msat,
            source_wallet_id, target_wallet_id,
            source_network, target_network, source_rail, target_rail,
            source_asset, target_asset, state, basis_state, reason,
            occurred_at, target_occurred_at, created_at
        ) VALUES (
            ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?
        )
    """
    with conn:
        for batch in _batches(_lineage_transaction_rows(transaction_count), batch_size):
            conn.executemany(transaction_sql, batch)
        for batch in _batches(_lineage_decision_rows(decision_count), batch_size):
            conn.executemany(decision_sql, batch)
    return time.perf_counter() - started


def _plan_text(conn: sqlite3.Connection, sql: str, params: Sequence[Any]) -> str:
    rows = conn.execute(f"EXPLAIN QUERY PLAN {sql}", params).fetchall()
    return " | ".join(str(row["detail"]) for row in rows)


def benchmark_lineage(
    transaction_count: int,
    decision_ratio: float,
    page_size: int,
    batch_size: int,
) -> dict[str, Any]:
    decision_count = max(2, int(transaction_count * decision_ratio))
    required_transactions = decision_count * 2
    if required_transactions > transaction_count:
        raise ValueError("decision ratio cannot require more than two transaction legs")
    with tempfile.TemporaryDirectory(prefix="kassiber-custody-lineage-") as tmp:
        conn = open_db(Path(tmp) / "data")
        try:
            _seed_scope(conn)
            seed_seconds = _seed_lineage(
                conn, transaction_count, decision_count, batch_size
            )
            # Let the planner see that transaction ids are selective even in
            # smoke-sized databases. Production books naturally accumulate
            # these statistics; making it explicit keeps the query-plan
            # invariant deterministic across SQLite versions.
            conn.execute("ANALYZE")
            first, first_seconds = _timed(
                lambda: custody_decision_rows(
                    conn, PROFILE_ID, limit=page_size
                )
            )
            second, second_seconds = _timed(
                lambda: custody_decision_rows(
                    conn,
                    PROFILE_ID,
                    limit=page_size,
                    cursor=first["next_cursor"],
                )
            )
            scoped_id = f"lineage-tx-{(decision_count // 2) * 2 + 1}"
            scoped, scoped_seconds = _timed(
                lambda: custody_decision_rows(
                    conn,
                    PROFILE_ID,
                    limit=page_size,
                    transaction_ids=[scoped_id],
                )
            )
            ordered_plan = _plan_text(
                conn,
                """
                SELECT d.decision_id
                FROM journal_custody_decisions d
                WHERE d.profile_id = ?
                ORDER BY d.occurred_at DESC, d.decision_id DESC
                LIMIT ?
                """,
                (PROFILE_ID, page_size),
            )
            scoped_plan = _plan_text(
                conn,
                """
                SELECT d.decision_id
                FROM journal_custody_decisions d
                WHERE d.profile_id = ?
                  AND (d.source_transaction_id IN (?)
                       OR d.target_transaction_id IN (?))
                """,
                (PROFILE_ID, scoped_id, scoped_id),
            )
        finally:
            conn.close()
    ordered_upper = ordered_plan.upper()
    return {
        "decision_count": decision_count,
        "seed_seconds": round(seed_seconds, 6),
        "first_page_seconds": round(first_seconds, 6),
        "subsequent_page_seconds": round(second_seconds, 6),
        "transaction_scoped_seconds": round(scoped_seconds, 6),
        "page_size": page_size,
        "first_returned": first["returned"],
        "second_returned": second["returned"],
        "transaction_scoped_returned": scoped["returned"],
        "ordered_query_plan": ordered_plan,
        "transaction_scoped_query_plan": scoped_plan,
        "invariants": {
            "keyset_cursor_available": first["next_cursor"] is not None,
            "subsequent_page_available": second["returned"] == page_size,
            "ordered_page_uses_profile_time_index": (
                "idx_journal_custody_decisions_profile_time" in ordered_plan
            ),
            "ordered_page_avoids_temp_sort": "TEMP B-TREE" not in ordered_upper,
            "transaction_lookup_uses_multi_index_or": (
                "MULTI-INDEX OR" in scoped_plan.upper()
            ),
            "transaction_lookup_uses_source_index": (
                "idx_journal_custody_decisions_source" in scoped_plan
            ),
            "transaction_lookup_uses_target_index": (
                "idx_journal_custody_decisions_target" in scoped_plan
            ),
        },
    }


def _gap_rows(
    transaction_count: int, relevant_outbounds: int
) -> Iterator[tuple[Any, ...]]:
    ordinary_count = transaction_count - relevant_outbounds - 1
    for index in range(ordinary_count):
        yield (
            f"ordinary-{index}",
            WORKSPACE_ID,
            PROFILE_ID,
            TARGET_WALLET_ID,
            f"ordinary-fingerprint-{index}",
            "2019-01-01T00:00:00Z",
            "inbound",
            "BTC",
            1,
            None,
            None,
            "{}",
            "2019-01-01T00:00:00Z",
        )
    for index in range(relevant_outbounds):
        occurred_at = (
            "2020-01-01T00:00:00Z"
            if index == 0
            else f"2022-01-{(index % 28) + 1:02d}T00:00:{index % 60:02d}Z"
        )
        yield (
            f"boundary-out-{index}",
            WORKSPACE_ID,
            PROFILE_ID,
            SOURCE_WALLET_ID,
            f"boundary-fingerprint-{index}",
            occurred_at,
            "outbound",
            "BTC",
            10 * BTC_MSAT + index * BTC_MSAT,
            "samourai_deposit",
            "whirlpool",
            "{}",
            occurred_at,
        )
    yield (
        "seeded-return",
        WORKSPACE_ID,
        PROFILE_ID,
        TARGET_WALLET_ID,
        "seeded-return-fingerprint",
        "2021-01-01T00:00:00Z",
        "inbound",
        "BTC",
        99 * BTC_MSAT // 10,
        None,
        None,
        "{}",
        "2021-01-01T00:00:00Z",
    )


def _seed_gap_book(
    conn: sqlite3.Connection,
    transaction_count: int,
    relevant_outbounds: int,
    batch_size: int,
) -> float:
    sql = """
        INSERT INTO transactions(
            id, workspace_id, profile_id, wallet_id, fingerprint, occurred_at,
            direction, asset, amount, kind, privacy_boundary, raw_json,
            created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    """
    started = time.perf_counter()
    with conn:
        for batch in _batches(
            _gap_rows(transaction_count, relevant_outbounds), batch_size
        ):
            conn.executemany(sql, batch)
    return time.perf_counter() - started


def benchmark_gaps(
    transaction_count: int,
    relevant_outbounds: int,
    page_size: int,
    batch_size: int,
) -> dict[str, Any]:
    if transaction_count <= relevant_outbounds:
        raise ValueError("transaction count must exceed relevant outbound count")
    with tempfile.TemporaryDirectory(prefix="kassiber-custody-gaps-") as tmp:
        conn = open_db(Path(tmp) / "data")
        try:
            _seed_scope(conn)
            seed_seconds = _seed_gap_book(
                conn, transaction_count, relevant_outbounds, batch_size
            )
            first, first_seconds = _timed(
                lambda: build_gap_snapshot(
                    conn, PROFILE_ID, limit=page_size
                )
            )
            second = None
            second_seconds = None
            if first.get("next_cursor"):
                second, second_seconds = _timed(
                    lambda: build_gap_snapshot(
                        conn,
                        PROFILE_ID,
                        limit=page_size,
                        cursor=first["next_cursor"],
                    )
                )
        finally:
            conn.close()
    gaps = first.get("gaps", [])
    # The public snapshot intentionally omits raw transaction ids. Identify
    # the deterministic seed through its exact quantities and structured
    # evidence instead of weakening that privacy boundary for a benchmark.
    seeded_visible = any(
        gap.get("source_total_msat") == 10 * BTC_MSAT
        and gap.get("return_total_msat") == 99 * BTC_MSAT // 10
        and "structured_privacy_boundary" in gap.get("reason_codes", [])
        for gap in gaps
    )
    summary = first.get("summary", {})
    return {
        "relevant_outbounds": relevant_outbounds,
        "seed_seconds": round(seed_seconds, 6),
        "first_page_seconds": round(first_seconds, 6),
        "subsequent_page_seconds": (
            None if second_seconds is None else round(second_seconds, 6)
        ),
        "returned": len(gaps),
        "second_returned": 0 if second is None else len(second.get("gaps", [])),
        "next_cursor_available": first.get("next_cursor") is not None,
        "search_complete": summary.get("search_complete"),
        "search_status": summary.get("search_status"),
        "search_limit_kind": summary.get("search_limit_kind"),
        "seeded_candidate_visible": seeded_visible,
        "invariants": {
            "structured_candidate_survives_large_book": seeded_visible,
            "more_than_87_outbounds_do_not_collapse_queue": (
                relevant_outbounds <= 87 or len(gaps) > 0
            ),
            "capacity_is_explicit": (
                bool(summary.get("search_complete"))
                or summary.get("search_status") == "capacity_limited"
            ),
        },
    }


def _parse_sizes(value: str) -> list[int]:
    try:
        sizes = [int(item.strip()) for item in value.split(",") if item.strip()]
    except ValueError as exc:
        raise argparse.ArgumentTypeError("sizes must be comma-separated integers") from exc
    if not sizes or any(size < 2 for size in sizes):
        raise argparse.ArgumentTypeError("every size must be at least 2")
    return sizes


def _emit(payload: dict[str, Any]) -> None:
    print(json.dumps(payload, ensure_ascii=True, separators=(",", ":")), flush=True)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--sizes",
        type=_parse_sizes,
        default=[100_000],
        help="comma-separated total transaction counts (default: 100000)",
    )
    parser.add_argument(
        "--stages",
        default="atomic,lineage,gaps",
        help="comma-separated subset of atomic,lineage,gaps",
    )
    parser.add_argument(
        "--decision-ratio",
        type=float,
        default=0.4,
        help="lineage/atomic decisions per transaction (default: 0.4)",
    )
    parser.add_argument("--page-size", type=int, default=100)
    parser.add_argument("--batch-size", type=int, default=5_000)
    parser.add_argument("--relevant-outbounds", type=int, default=90)
    parser.add_argument(
        "--smoke",
        action="store_true",
        help="run all stages with a small deterministic book",
    )
    args = parser.parse_args(argv)
    if args.smoke:
        args.sizes = [500]
        args.decision_ratio = 0.2
        args.page_size = 20
        args.relevant_outbounds = 90
    stages = [item.strip() for item in args.stages.split(",") if item.strip()]
    unknown = sorted(set(stages) - {"atomic", "lineage", "gaps"})
    if unknown:
        parser.error(f"unknown stages: {', '.join(unknown)}")
    if not 0 < args.decision_ratio <= 0.5:
        parser.error("--decision-ratio must be greater than zero and at most 0.5")
    if not 1 <= args.page_size <= 500:
        parser.error("--page-size must be between 1 and 500")
    if args.batch_size < 1:
        parser.error("--batch-size must be positive")
    if args.relevant_outbounds < 1:
        parser.error("--relevant-outbounds must be positive")

    _emit(
        {
            "event": "benchmark_start",
            "schema_version": 1,
            "sizes": args.sizes,
            "stages": stages,
            "decision_ratio": args.decision_ratio,
            "page_size": args.page_size,
            "relevant_outbounds": args.relevant_outbounds,
            "environment": {
                "python": platform.python_version(),
                "sqlite": sqlite3.sqlite_version,
                "platform": platform.platform(),
            },
        }
    )
    all_invariants_hold = True
    for transaction_count in args.sizes:
        for stage in stages:
            started = time.perf_counter()
            try:
                if stage == "atomic":
                    metrics = benchmark_atomic(
                        transaction_count, args.decision_ratio
                    )
                elif stage == "lineage":
                    metrics = benchmark_lineage(
                        transaction_count,
                        args.decision_ratio,
                        args.page_size,
                        args.batch_size,
                    )
                else:
                    metrics = benchmark_gaps(
                        transaction_count,
                        args.relevant_outbounds,
                        args.page_size,
                        args.batch_size,
                    )
                invariants = metrics.get("invariants", {})
                stage_holds = all(bool(value) for value in invariants.values())
                all_invariants_hold = all_invariants_hold and stage_holds
                _emit(
                    {
                        "event": "benchmark_result",
                        "schema_version": 1,
                        "stage": stage,
                        "transactions": transaction_count,
                        "ok": stage_holds,
                        "metrics": metrics,
                    }
                )
            except Exception as exc:
                all_invariants_hold = False
                _emit(
                    {
                        "event": "benchmark_error",
                        "schema_version": 1,
                        "stage": stage,
                        "transactions": transaction_count,
                        "ok": False,
                        "error_type": type(exc).__name__,
                        "error": str(exc),
                        "elapsed_seconds": round(time.perf_counter() - started, 6),
                    }
                )
            finally:
                gc.collect()
    _emit(
        {
            "event": "benchmark_complete",
            "schema_version": 1,
            "ok": all_invariants_hold,
        }
    )
    return 0 if all_invariants_hold else 1


if __name__ == "__main__":
    raise SystemExit(main())
