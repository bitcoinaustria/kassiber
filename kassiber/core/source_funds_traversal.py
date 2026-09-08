"""Bounded dependency ordering for reviewed source-of-funds disclosures.

Amounts and evidence authority remain with the report builder. This plan only
ensures all downstream demands arrive before an upstream transaction is used.
"""

from __future__ import annotations

import sqlite3
from collections import deque
from dataclasses import dataclass
from graphlib import CycleError, TopologicalSorter
from typing import Mapping


@dataclass(frozen=True)
class ReportTraversal:
    rank: Mapping[str, int]
    links: Mapping[str, tuple[sqlite3.Row, ...]]
    cycle_link_id: str | None
    truncated: bool


def load_report_traversal(
    conn: sqlite3.Connection,
    profile_id: str,
    target_id: str,
    *,
    max_depth: int,
    node_limit: int,
    edge_limit: int,
) -> ReportTraversal:
    dependencies: dict[str, set[str]] = {target_id: set()}
    links: dict[str, tuple[sqlite3.Row, ...]] = {}
    nodes = {f"tx:{target_id}"}
    queue = deque([(target_id, 0)])
    seen = {target_id}
    edge_count = 0
    truncated = False
    while queue and not truncated:
        tx_id, depth = queue.popleft()
        if depth >= max_depth:
            continue
        rows = conn.execute(
            """SELECT * FROM source_funds_links
            WHERE profile_id = ? AND to_transaction_id = ? AND state != 'rejected'
            ORDER BY state DESC, created_at ASC, id ASC LIMIT ?""",
            (profile_id, tx_id, edge_limit - edge_count + 1),
        ).fetchall()
        retained = []
        for row in rows:
            if edge_count >= edge_limit:
                truncated = True
                break
            parent_id = row["from_transaction_id"]
            if row["state"] == "reviewed":
                endpoint = f"tx:{parent_id}" if parent_id else f"source:{row['from_source_id']}"
                if endpoint not in nodes and len(nodes) >= node_limit:
                    truncated = True
                    break
                nodes.add(endpoint)
                if parent_id:
                    # The upstream parent depends on its downstream child.
                    dependencies.setdefault(parent_id, set()).add(tx_id)
                    if parent_id not in seen:
                        seen.add(parent_id)
                        queue.append((parent_id, depth + 1))
            retained.append(row)
            edge_count += 1
        links[tx_id] = tuple(retained)

    sorter = TopologicalSorter(dependencies)
    cycle_link_id = None
    try:
        sorter.prepare()
    except CycleError as exc:
        cycle = exc.args[1]
        cycle_edges = set(zip(cycle, cycle[1:]))
        cycle_link_id = next(
            (
                row["id"] for rows in links.values() for row in rows
                if (row["from_transaction_id"], row["to_transaction_id"]) in cycle_edges
            ),
            target_id,
        )
    order: list[str] = []
    while sorter.is_active():
        ready = sorted(sorter.get_ready())
        order.extend(ready)
        sorter.done(*ready)
    # Keep a bounded display of malformed cyclic paths, with export blocked.
    ordered = set(order)
    order.extend(tx_id for tx_id in dependencies if tx_id not in ordered)
    return ReportTraversal(
        rank={tx_id: ordinal for ordinal, tx_id in enumerate(order)},
        links=links,
        cycle_link_id=cycle_link_id,
        truncated=truncated,
    )
