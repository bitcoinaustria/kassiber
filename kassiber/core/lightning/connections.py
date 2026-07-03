"""Shared helpers for resolving a Lightning connection from a SQLite row.

Both the daemon (``ui.connections.node.snapshot`` /
``ui.reports.lightning_profitability``) and the CLI
(``reports lightning-profitability``) need the same lookup-by-id-or-label
and the same kind validation. Keeping it here means LND/CLN adapter PRs do
not have to choose between two near-identical helpers.

The resolver is profile-scoped because wallet labels are only unique within
a profile (``UNIQUE (profile_id, label)`` in :mod:`kassiber.db`). Resolving
against the global ``wallets`` table by ``lower(label)`` alone could pick
the wrong row when two profiles share a label like ``Home Node``. The
resolver also parses ``config_json`` so adapters can read their backend
name + adapter config without re-running the query.
"""

from __future__ import annotations

import json
import sqlite3
from typing import Any

from ...backends import get_db_backend, resolve_backend
from ...errors import AppError
from ..repo.context import resolve_scope

LIGHTNING_ADAPTER_KINDS: tuple[str, ...] = ("coreln", "lnd", "nwc")


def resolve_lightning_connection(
    conn: sqlite3.Connection,
    ref: str | None,
    *,
    workspace_ref: str | None = None,
    profile_ref: str | None = None,
) -> dict[str, Any]:
    """Look up a Lightning-kind wallet by id or label within the active profile.

    Scope is resolved via :func:`kassiber.core.repo.context.resolve_scope`
    (so the caller can override it with explicit ``workspace_ref`` /
    ``profile_ref``, or let the helper fall back to the persisted context
    settings). Lookup is then ``profile_id = ? AND (id = ? OR lower(label)
    = lower(?))`` so two profiles can share a wallet label without the
    daemon picking the wrong row.

    The returned dict has::

        {
            "id": str,
            "label": str,
            "kind": str,
            "profile_id": str,
            "config": dict[str, Any],   # parsed config_json
            "backend_name": str | None, # config["backend"] if any
        }

    Adapters read ``backend_name`` to find their backend row; the daemon's
    ``_resolve_backend_row`` helper already keys off that field.

    Raises :class:`AppError` with stable ``code`` values:
    - ``validation`` — missing ref or non-Lightning kind.
    - ``not_found`` — no wallet matched in the active profile.
    - ``ambiguous`` — multiple matches in the same profile (should not
      happen given the ``UNIQUE (profile_id, label)`` constraint, but the
      guard keeps the failure mode explicit if the schema ever drifts).
    """
    if not ref or not isinstance(ref, str):
        raise AppError(
            "Specify which Lightning connection to read.",
            code="validation",
            hint="Pass `connection` (wallet id or label).",
        )
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = list(
        conn.execute(
            "SELECT id, label, kind, profile_id, config_json FROM wallets"
            " WHERE profile_id = ? AND (id = ? OR lower(label) = lower(?))",
            (profile["id"], ref, ref),
        )
    )
    if not rows:
        raise AppError(
            f"Lightning connection '{ref}' not found.",
            code="not_found",
            hint="Run `kassiber wallets list` to see configured connections.",
        )
    if len(rows) > 1:
        # The UNIQUE (profile_id, label) constraint makes this unreachable
        # in practice, but if the schema ever drifts (or someone queries
        # via a future composite ref) we want a deterministic error rather
        # than picking row 0 silently.
        raise AppError(
            f"Lightning connection '{ref}' is ambiguous in the active profile.",
            code="ambiguous",
            hint="Pass the wallet id instead of the label to disambiguate.",
        )
    row = dict(rows[0])
    kind = str(row.get("kind") or "")
    if kind not in LIGHTNING_ADAPTER_KINDS:
        raise AppError(
            f"Connection '{row.get('label') or ref}' is not a Lightning node"
            f" (kind={kind!r}).",
            code="validation",
            hint=f"Lightning kinds are {', '.join(LIGHTNING_ADAPTER_KINDS)}.",
        )
    try:
        config: dict[str, Any] = json.loads(row.pop("config_json") or "{}")
    except (TypeError, ValueError):
        # A wallet with malformed config_json is recoverable for read-only
        # snapshot calls: adapters can still see the kind/profile and use
        # their defaults. We surface an empty config so the caller does
        # not have to special-case None.
        config = {}
    if not isinstance(config, dict):
        config = {}
    row["config"] = config
    backend = config.get("backend")
    row["backend_name"] = str(backend) if isinstance(backend, str) and backend else None
    return row


def resolve_lightning_backend(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None,
    wallet: dict[str, Any],
) -> dict[str, Any] | None:
    """Resolve the raw backend row/config referenced by a Lightning wallet.

    Display helpers intentionally redact backend secrets before returning rows
    to CLI/UI callers. Adapter calls need the real backend config instead:
    local ``lightning_cli`` wrapper paths, socket hints, and commando tokens
    are transport inputs, not presentation data. This helper keeps the lookup
    small and shared across CLI + daemon surfaces without widening any
    user-facing backend output.
    """
    backend_name = wallet.get("backend_name") if isinstance(wallet, dict) else None
    if not backend_name:
        return None
    name = str(backend_name)
    if runtime_config is not None:
        try:
            return dict(resolve_backend(runtime_config, name))
        except AppError:
            # Runtime config may be a minimal fixture, or a caller may have a
            # stale env/bootstrap view while the canonical DB row exists.
            # The DB lookup below produces the stable not_found if both miss.
            pass
        except (KeyError, TypeError, ValueError):
            # Fall through to the canonical DB row lookup below. This keeps
            # CLI tests with tiny runtime_config fixtures simple while still
            # surfacing DB-level not_found errors deterministically.
            pass
    return get_db_backend(conn, name)
