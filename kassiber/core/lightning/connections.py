"""Shared helpers for resolving a Lightning connection from a SQLite row.

Both the daemon (``ui.connections.node.snapshot`` /
``ui.reports.lightning_profitability``) and the CLI
(``reports lightning-profitability``) need the same lookup-by-id-or-label
and the same kind validation. Keeping it here means LND/CLN adapter PRs do
not have to choose between two near-identical helpers.
"""

from __future__ import annotations

import sqlite3
from typing import Any

from ...errors import AppError

LIGHTNING_WALLET_KINDS: tuple[str, ...] = ("coreln", "lnd", "nwc")


def resolve_lightning_connection(
    conn: sqlite3.Connection, ref: str | None
) -> dict[str, Any]:
    """Look up a Lightning-kind wallet by id or label.

    Raises :class:`AppError` with stable ``code`` values:
    - ``validation`` — missing ref or non-Lightning kind.
    - ``not_found`` — no wallet matched.
    """
    if not ref or not isinstance(ref, str):
        raise AppError(
            "Specify which Lightning connection to read.",
            code="validation",
            hint="Pass `connection` (wallet id or label).",
        )
    rows = list(
        conn.execute(
            "SELECT id, label, kind FROM wallets"
            " WHERE id = ? OR lower(label) = lower(?) LIMIT 1",
            (ref, ref),
        )
    )
    if not rows:
        raise AppError(
            f"Lightning connection '{ref}' not found.",
            code="not_found",
            hint="Run `kassiber wallets list` to see configured connections.",
        )
    row = dict(rows[0])
    kind = str(row.get("kind") or "")
    if kind not in LIGHTNING_WALLET_KINDS:
        raise AppError(
            f"Connection '{row.get('label') or ref}' is not a Lightning node"
            f" (kind={kind!r}).",
            code="validation",
            hint=f"Lightning kinds are {', '.join(LIGHTNING_WALLET_KINDS)}.",
        )
    return row
