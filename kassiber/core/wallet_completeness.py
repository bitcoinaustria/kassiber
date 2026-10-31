"""How complete can a wallet's observed history be, and what bounds it.

Kassiber cannot prove that a book holds every wallet a user owns, and for most
sources it cannot prove that one wallet's history is whole either. This module
does not try to. It states, per wallet, which *kind* of source the history came
from (`completeness_class`) and the observable edges of that history
(`scan_bounds`), so a reviewer or the assistant can bound what is unknown
instead of assuming completeness.

Both values are derived from the wallet kind, the presence (never the content)
of its source configuration, its sync timestamp, and its transaction rows. No
descriptor, xpub, address, backend URL or file path is read into the output.
The CLI `wallets list` and the daemon `ui.wallets.list` share this function.

Classes (stable strings):

- ``descriptor-derived``: addresses derived from a descriptor, xpub or Silent
  Payments scan key (including Samourai groups); a chain scan can see every
  derived script within the gap limit.
- ``address-list``: a fixed list of addresses; change and unlisted receive
  addresses are invisible, so the history is incomplete by construction.
- ``lightning-node``: read from a connected Lightning node's own database.
- ``provider-api``: fetched from a provider API (exchange API, BTCPay store).
- ``exchange-export``: a custodial platform's file export.
- ``file-import``: another wallet or tool export read from a file.
- ``manual``: user-entered or user-mapped rows (generic ledger, no source).
- ``untracked``: no source at all; a placeholder for reviewed custody.
"""

from __future__ import annotations

import json
import sqlite3
from typing import Any, Iterable, Mapping

from ..time_utils import UNKNOWN_OCCURRED_AT
from ..util import parse_bool, str_or_none
from . import silent_payments
from .sync import negative_balance_events
from .wallets import has_descriptor_sync_material, normalize_addresses

COMPLETENESS_CLASSES = (
    "descriptor-derived",
    "address-list",
    "lightning-node",
    "provider-api",
    "exchange-export",
    "file-import",
    "manual",
    "untracked",
)

# Classes whose history comes from a live source Kassiber scans itself, so a
# missing sync timestamp means nothing was observed yet.
LIVE_CLASSES = frozenset(
    {"descriptor-derived", "address-list", "lightning-node", "provider-api"}
)
# Classes a user-supplied ledger file may be imported into. Live, chain-scanned
# sources are authoritative for their own history and must not be mixed with
# hand-mapped rows.
FILE_IMPORT_TARGET_CLASSES = frozenset({"exchange-export", "file-import", "manual"})

_DESCRIPTOR_KINDS = frozenset({"descriptor", "xpub", silent_payments.WALLET_KIND, "samourai"})
_LIGHTNING_NODE_KINDS = frozenset({"coreln", "lnd"})
_EXCHANGE_API_KINDS = frozenset({"kraken", "coinbase", "binance"})
_CUSTODIAL_EXPORT_KINDS = frozenset(
    {
        "river",
        "bullbitcoin",
        "coinfinity",
        "21bitcoin",
        "pocketbitcoin",
        "strike",
        *_EXCHANGE_API_KINDS,
    }
)

HISTORY_SCANNED = "scanned"
HISTORY_DECLARED_START = "declared_start"
HISTORY_UNKNOWN = "unknown"


def _config(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def completeness_class(kind: Any, config: Mapping[str, Any]) -> str:
    """Classify a wallet by the shape of its source, never its content."""

    kind = str(kind or "").strip().lower()
    has_file = bool(str_or_none(config.get("source_file")))
    source_format = str_or_none(config.get("source_format"))
    if kind == "untracked":
        return "untracked"
    sync_source = str_or_none(config.get("sync_source"))
    if sync_source and sync_source.strip().lower() == "btcpay":
        return "provider-api"
    if kind in _DESCRIPTOR_KINDS:
        if (
            kind == "samourai"
            or has_descriptor_sync_material(config)
            or silent_payments.has_silent_payment_sync_material(config)
        ):
            return "descriptor-derived"
        return "file-import" if has_file else "descriptor-derived"
    if kind == "address":
        return "address-list"
    if kind in _LIGHTNING_NODE_KINDS and str_or_none(config.get("backend")):
        return "lightning-node"
    if kind in _EXCHANGE_API_KINDS and str_or_none(config.get("backend")) and not has_file:
        return "provider-api"
    if kind in _CUSTODIAL_EXPORT_KINDS:
        return "exchange-export"
    if kind == "custom" and source_format in (None, "generic_ledger"):
        return "manual"
    return "file-import"


def _declared_scan_start(kind: str, config: Mapping[str, Any]) -> tuple[str | None, int | None]:
    """A user-declared point before which nothing is scanned, if any."""

    if kind == silent_payments.WALLET_KIND and not parse_bool(
        config.get(silent_payments.CONFIG_FULL_HISTORY), default=False
    ):
        start_date = str_or_none(config.get(silent_payments.CONFIG_SCAN_START_DATE))
        start_height = config.get(silent_payments.CONFIG_SCAN_START_HEIGHT)
        height = start_height if isinstance(start_height, int) and not isinstance(start_height, bool) else None
        if start_date or height is not None:
            return start_date, height
    birthday = str_or_none(config.get("birthday"))
    return birthday, None


def _wallet_rows(
    conn: sqlite3.Connection, profile_id: str, wallet_ids: Iterable[str]
) -> dict[str, list[dict[str, Any]]]:
    ids = sorted({str(wallet_id) for wallet_id in wallet_ids})
    rows_by_wallet: dict[str, list[dict[str, Any]]] = {wallet_id: [] for wallet_id in ids}
    if not ids:
        return rows_by_wallet
    # raw_json is only consulted to order unconfirmed chain rows; skip the
    # payload everywhere else so a large book does not pay for it.
    for row in conn.execute(
        """
        SELECT wallet_id, id, external_id, occurred_at, confirmed_at,
               CASE WHEN confirmed_at IS NULL THEN raw_json END AS raw_json,
               direction, asset, amount, fee, created_at
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile_id,),
    ):
        bucket = rows_by_wallet.get(str(row["wallet_id"]))
        if bucket is not None:
            bucket.append(dict(row))
    return rows_by_wallet


def scan_bounds(
    kind: Any,
    config: Mapping[str, Any],
    rows: list[dict[str, Any]],
    *,
    klass: str | None = None,
) -> dict[str, Any]:
    kind = str(kind or "").strip().lower()
    klass = klass or completeness_class(kind, config)
    dated = sorted(
        str(row["occurred_at"])
        for row in rows
        if row.get("occurred_at") not in (None, "", UNKNOWN_OCCURRED_AT)
    )
    undated = len(rows) - len(dated)
    last_synced_at = str_or_none(config.get("last_synced_at"))
    declared_at, declared_height = _declared_scan_start(kind, config)
    warnings: list[str] = []
    if klass in {"address-list", "untracked"}:
        warnings.append("incomplete_by_construction")
    live = klass in LIVE_CLASSES and not (
        klass == "address-list"
        and not normalize_addresses(config.get("addresses"))
    )
    if live and not last_synced_at:
        warnings.append("never_synced")
    if not rows:
        warnings.append("no_transactions")
    if undated:
        warnings.append("undated_transactions")
    deficits = negative_balance_events(rows) if rows else []
    if deficits:
        warnings.append("history_starts_after_first_funding")
    if not live or not last_synced_at:
        history = HISTORY_UNKNOWN
    elif declared_at or declared_height is not None:
        history = HISTORY_DECLARED_START
    else:
        history = HISTORY_SCANNED
    return {
        "first_transaction_at": dated[0] if dated else None,
        "last_transaction_at": dated[-1] if dated else None,
        "undated_transactions": undated,
        "last_synced_at": last_synced_at,
        "declared_scan_start_at": declared_at,
        "declared_scan_start_height": declared_height,
        "history_before_first_transaction": history,
        "deficit_assets": sorted({str(item["asset"]) for item in deficits}),
        "warnings": warnings,
    }


def describe_wallets(
    conn: sqlite3.Connection,
    profile_id: str,
    wallets: Iterable[Mapping[str, Any]],
) -> dict[str, dict[str, Any]]:
    """Return ``{wallet_id: {completeness_class, scan_bounds}}`` for a book.

    ``wallets`` need ``id``, ``kind`` and ``config_json`` (or a parsed
    ``config``); one transaction query serves the whole list.
    """

    wallets = list(wallets)
    rows_by_wallet = _wallet_rows(conn, profile_id, (wallet["id"] for wallet in wallets))
    described: dict[str, dict[str, Any]] = {}
    for wallet in wallets:
        config = _config(wallet.get("config") if "config" in wallet else wallet.get("config_json"))
        klass = completeness_class(wallet.get("kind"), config)
        described[str(wallet["id"])] = {
            "completeness_class": klass,
            "scan_bounds": scan_bounds(
                wallet.get("kind"),
                config,
                rows_by_wallet.get(str(wallet["id"]), []),
                klass=klass,
            ),
        }
    return described


def annotate_wallet_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    rows: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Add ``completeness_class`` and ``scan_bounds`` to listed wallet rows."""

    stored = conn.execute(
        "SELECT id, kind, config_json FROM wallets WHERE profile_id = ?",
        (profile_id,),
    ).fetchall()
    described = describe_wallets(conn, profile_id, (dict(row) for row in stored))
    return [{**row, **described.get(str(row.get("id")), {})} for row in rows]


def wallet_completeness_class(conn: sqlite3.Connection, wallet_id: str) -> str:
    row = conn.execute(
        "SELECT kind, config_json FROM wallets WHERE id = ?", (wallet_id,)
    ).fetchone()
    if row is None:
        return "untracked"
    return completeness_class(row["kind"], _config(row["config_json"]))
