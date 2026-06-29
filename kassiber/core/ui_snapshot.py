from __future__ import annotations

import base64
import binascii
import json
import sqlite3
from collections import defaultdict
from datetime import date, datetime, timezone
from typing import Any

from ..backends import backend_value, redact_backend_for_output
from ..errors import AppError
from ..msat import msat_to_btc
from ..time_utils import _iso_z, _parse_iso_datetime
from ..wallet_descriptors import (
    BITCOIN_NETWORK_ALIASES,
    CHAIN_ALIASES,
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    LIQUID_NETWORK_ALIASES,
    liquid_plan_can_unblind,
    normalize_chain,
    normalize_network,
)
from . import output_inventory as core_output_inventory
from . import ownership as core_ownership
from . import rates as core_rates
from . import sync_backends as core_sync_backends
from . import reports as report_builders
from .samourai import samourai_metadata_from_wallet_config
from . import transaction_history
from .repo import current_context_snapshot
from .sync import normalize_backend_kind
from .wallets import (
    has_descriptor_sync_material,
    load_wallet_descriptor_plan_from_config,
    wallet_btcpay_provenance_config,
    wallet_is_deprecated,
)


MAX_UI_LIST_LIMIT = 500
MAX_UI_PREVIEW_LIMIT = 100
_UI_TRANSACTION_SORT_COLUMNS = {
    "occurred-at": "t.occurred_at",
    "amount": "t.amount",
    "fiat-value": "COALESCE(t.fiat_value, 0)",
    "fee": "t.fee",
}
_JOURNAL_DISPLAY_ENTRY_TYPE_SQL = """
CASE
    WHEN je.at_category = 'neu_swap' THEN 'neutral_swap'
    ELSE je.entry_type
END
""".strip()
_JOURNAL_DISPLAY_GAIN_LOSS_SQL = """
CASE
    WHEN je.at_category = 'neu_swap' THEN 0
    ELSE COALESCE(je.gain_loss, 0)
END
""".strip()
_AUDIT_PROFILE_TABLE_COLUMNS = {
    "transactions": "created_at",
    "transaction_edit_events": "changed_at",
    "journal_entries": "created_at",
    "journal_quarantines": "created_at",
    "wallets": "created_at",
}
_AUDIT_GLOBAL_TABLE_COLUMNS = {"rates_cache": "fetched_at"}


def _empty_overview_snapshot() -> dict[str, Any]:
    return {
        "priceEur": 0.0,
        "priceUsd": 0.0,
        "marketRate": {
            "asset": "BTC",
            "fiatCurrency": "EUR",
            "pair": "BTC-EUR",
            "rate": None,
            "timestamp": None,
            "source": None,
            "fetchedAt": None,
            "granularity": None,
            "method": None,
        },
        "connections": [],
        "txs": [],
        "balanceSeries": [0.0] * 12,
        "portfolioSeries": [],
        "fiat": {
            "fiatCurrency": "EUR",
            "eurBalance": 0.0,
            "eurCostBasis": 0.0,
            "eurUnrealized": 0.0,
            "eurRealizedYTD": 0.0,
        },
        "status": {
            "workspace": None,
            "profile": None,
            "transactionCount": 0,
            "needsJournals": False,
            "quarantines": 0,
        },
    }


def _empty_workspace_overview_snapshot(
    workspace: sqlite3.Row | None = None,
) -> dict[str, Any]:
    return {
        "workspace": (
            {
                "id": workspace["id"],
                "label": workspace["label"],
            }
            if workspace is not None
            else None
        ),
        "scope": {
            "kind": "workspace",
            "label": "Book set",
        },
        "books": [],
        "connections": [],
        "txs": [],
        "activityTxs": [],
        "balanceSeries": [0.0] * 12,
        "portfolioSeries": [],
        "fiat": {
            "mode": "empty",
            "fiatCurrency": None,
            "currencies": [],
            "mixed": False,
            "partial": False,
            "eurBalance": 0.0,
            "eurCostBasis": 0.0,
            "eurUnrealized": 0.0,
            "eurRealizedYTD": 0.0,
            "btcBalance": 0.0,
            "books": [],
        },
        "status": {
            "workspace": workspace["label"] if workspace is not None else None,
            "workspaceId": workspace["id"] if workspace is not None else None,
            "bookCount": 0,
            "transactionCount": 0,
            "needsJournals": False,
            "quarantines": 0,
            "ready": False,
            "mixedFiat": False,
        },
    }


def _latest_rate(conn: sqlite3.Connection, pair: str) -> float:
    row = _latest_rate_row(conn, pair)
    return float(row["rate"]) if row else 0.0


def _latest_rate_row(conn: sqlite3.Connection, pair: str) -> sqlite3.Row | None:
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, source, fetched_at, granularity, method
        FROM rates_cache
        WHERE pair = ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC,
                 source ASC
        LIMIT 1
        """,
        (pair,),
    ).fetchone()
    return row


def _market_rate_snapshot(conn: sqlite3.Connection, fiat_currency: str) -> dict[str, Any]:
    fiat_code = str(fiat_currency or "EUR").strip().upper() or "EUR"
    pair = core_rates.transaction_rate_pair("BTC", fiat_code)
    row = _latest_rate_row(conn, pair) if pair else None
    return {
        "asset": "BTC",
        "fiatCurrency": fiat_code,
        "pair": pair,
        "rate": float(row["rate"]) if row else None,
        "timestamp": row["timestamp"] if row else None,
        "source": row["source"] if row else None,
        "fetchedAt": row["fetched_at"] if row else None,
        "granularity": row["granularity"] if row else None,
        "method": row["method"] if row else None,
    }


def _latest_transaction_rate(
    conn: sqlite3.Connection,
    profile_id: str,
    fiat_currency: str,
) -> float:
    row = conn.execute(
        """
        SELECT fiat_rate
        FROM transactions
        WHERE profile_id = ?
          AND excluded = 0
          AND asset = 'BTC'
          AND fiat_currency = ?
          AND fiat_rate IS NOT NULL
        ORDER BY occurred_at DESC, created_at DESC, id DESC
        LIMIT 1
        """,
        (profile_id, fiat_currency),
    ).fetchone()
    return float(row["fiat_rate"]) if row else 0.0


def _coerce_args(args: dict[str, Any] | None) -> dict[str, Any]:
    if args is None:
        return {}
    if isinstance(args, dict):
        return args
    raise AppError(
        "ui snapshot args must be an object",
        code="validation",
        details={"type": type(args).__name__},
        retryable=False,
    )


def _coerce_limit(
    args: dict[str, Any],
    *,
    default: int,
    maximum: int,
) -> int:
    raw = args.get("limit", default)
    try:
        limit = int(raw)
    except (TypeError, ValueError):
        raise AppError(
            "limit must be an integer",
            code="validation",
            details={"limit": raw},
            retryable=False,
        ) from None
    if limit < 1:
        raise AppError(
            "limit must be positive",
            code="validation",
            details={"limit": raw},
            retryable=False,
        )
    return min(limit, maximum)


def _ui_transaction_cursor_filters(
    context: dict[str, Any],
    *,
    direction: str | None,
    asset: str | None,
    wallet: str | None,
    since: str | None,
    until: str | None,
    query: str | None,
) -> dict[str, str]:
    return {
        "workspace_id": str(context.get("workspace_id") or ""),
        "profile_id": str(context.get("profile_id") or ""),
        "direction": direction or "",
        "asset": asset or "",
        "wallet": wallet or "",
        "since": since or "",
        "until": until or "",
        "query": query or "",
    }


def _ui_transaction_cursor_value(row: sqlite3.Row, sort: str) -> int | float | str:
    if sort == "occurred-at":
        return row["occurred_at"] or ""
    if sort == "amount":
        return int(row["amount"] or 0)
    if sort == "fee":
        return int(row["fee"] or 0)
    if sort == "fiat-value":
        return float(row["fiat_value"] or 0)
    raise AppError(
        f"Unsupported transaction sort: {sort}",
        code="validation",
        hint="Use one of: occurred-at, amount, fiat-value, fee.",
        retryable=False,
    )


def _encode_ui_transaction_cursor(
    row: sqlite3.Row,
    *,
    sort: str,
    order: str,
    filters: dict[str, str],
    skip_pairs: set[str] | None = None,
) -> str:
    payload = {
        "sort": sort,
        "order": order,
        "filters": filters,
        "value": _ui_transaction_cursor_value(row, sort),
        "occurred_at": row["occurred_at"] or "",
        "created_at": row["_created_at"] or "",
        "id": row["id"],
        "skip_pairs": sorted(skip_pairs or set()),
    }
    token = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_ui_transaction_cursor(
    cursor: Any,
    *,
    sort: str,
    order: str,
    filters: dict[str, str],
) -> dict[str, Any] | None:
    if cursor in (None, ""):
        return None
    if not isinstance(cursor, str):
        raise AppError("cursor must be a string", code="validation", retryable=False)
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        if payload.get("sort") != sort or payload.get("order") != order:
            raise ValueError("cursor sort/order mismatch")
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        required = {
            "sort",
            "order",
            "filters",
            "value",
            "occurred_at",
            "created_at",
            "id",
        }
        if not required.issubset(payload):
            raise ValueError("missing cursor fields")
        if not isinstance(payload.get("skip_pairs", []), list):
            raise ValueError("invalid cursor skip_pairs")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            "Invalid cursor",
            code="validation",
            hint="Pass the exact nextCursor value from the previous response; do not modify it or change filters.",
            retryable=False,
        ) from exc


def _json_config(value: str | None) -> dict[str, Any]:
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return parsed if isinstance(parsed, dict) else {}


def _string_or_empty(value: Any) -> str:
    return value.strip() if isinstance(value, str) else ""


def _wallet_backend_summary(
    kind: str,
    config: dict[str, Any],
    default_backend: Any,
) -> dict[str, str]:
    explicit_backend = _string_or_empty(config.get("backend"))
    default_backend_name = _string_or_empty(default_backend)
    source_file = _string_or_empty(config.get("source_file"))
    source_format = _string_or_empty(config.get("source_format"))
    sync_source = _string_or_empty(config.get("sync_source"))
    has_descriptor = has_descriptor_sync_material(config)
    has_addresses = bool(config.get("addresses"))
    backend_name = explicit_backend
    backend_source = "explicit" if explicit_backend else "none"
    sync_mode = "not_configured"

    if sync_source == "btcpay":
        sync_mode = "btcpay"
    elif source_file and source_format:
        sync_mode = "file_import"
    elif has_descriptor and kind in {"descriptor", "xpub", "address"}:
        sync_mode = "backend_descriptor"
        if not backend_name and default_backend_name:
            backend_name = default_backend_name
            backend_source = "default"
    elif has_addresses and kind == "address":
        sync_mode = "backend_addresses"
        if not backend_name and default_backend_name:
            backend_name = default_backend_name
            backend_source = "default"

    if not backend_name:
        backend_source = "none"

    return {
        "name": backend_name,
        "source": backend_source,
        "sync_mode": sync_mode,
    }


def _unique_text_values(values: list[Any]) -> list[str]:
    output = []
    for value in values:
        text = _string_or_empty(value)
        if text and text not in output:
            output.append(text)
    return output


def _wallet_utxo_chain_filter_values(value: Any) -> list[str]:
    try:
        canonical = normalize_chain(value)
    except ValueError:
        return _unique_text_values([value])
    aliases = [alias for alias, target in CHAIN_ALIASES.items() if target == canonical]
    return _unique_text_values([canonical, value, *aliases])


def _wallet_utxo_network_filter_values(chain: str, value: Any) -> list[str]:
    try:
        canonical = normalize_network(chain, value)
    except ValueError:
        return _unique_text_values([value])
    aliases_map = BITCOIN_NETWORK_ALIASES if chain == "bitcoin" else LIQUID_NETWORK_ALIASES
    aliases = [alias for alias, target in aliases_map.items() if target == canonical]
    return _unique_text_values([canonical, value, *aliases])


def _wallet_utxo_source_filter(
    config: dict[str, Any],
    backend_summary: dict[str, str],
    backend: Any,
) -> dict[str, Any]:
    if _string_or_empty(config.get("source_format")) == "wasabi_bundle":
        chain_values = _wallet_utxo_chain_filter_values(config.get("chain") or "bitcoin")
        network_values = _wallet_utxo_network_filter_values(
            chain_values[0] if chain_values else "bitcoin",
            config.get("network") or "mainnet",
        )
        return {
            "backend_name": ["wasabi"],
            "backend_kind": ["wasabi_bundle"],
            "chain": chain_values,
            "network": network_values,
        }
    backend_chain = (
        backend_value(backend, "chain")
        if isinstance(backend, dict)
        else None
    )
    chain_source = backend_chain or config.get("chain")
    chain_values = _wallet_utxo_chain_filter_values(chain_source)
    chain = chain_values[0] if chain_values else _string_or_empty(chain_source)
    backend_network = (
        backend_value(backend, "network")
        if isinstance(backend, dict)
        else None
    )
    network_values = _wallet_utxo_network_filter_values(
        chain or "bitcoin",
        backend_network or config.get("network"),
    )
    backend_kind = (
        backend_value(backend, "kind")
        if isinstance(backend, dict)
        else None
    )
    backend_kind_values = (
        _unique_text_values([normalize_backend_kind(backend_kind), backend_kind])
        if _string_or_empty(backend_kind)
        else []
    )
    backend_name_values = _unique_text_values(
        [
            backend_summary.get("name"),
            backend.get("name") if isinstance(backend, dict) else None,
        ]
    )
    return {
        "backend_name": backend_name_values or None,
        "backend_kind": backend_kind_values or None,
        "chain": chain_values,
        "network": network_values,
    }


def _active_context_and_profile(
    conn: sqlite3.Connection,
) -> tuple[dict[str, str], sqlite3.Row | None]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return context, None
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    return context, profile


def _row_int(row: sqlite3.Row, key: str, default: int = 0) -> int:
    try:
        if key not in row.keys():
            return default
        value = row[key]
    except (IndexError, KeyError):
        return default
    return int(value or default)


def _journal_freshness(
    conn: sqlite3.Connection,
    profile: sqlite3.Row | None,
) -> dict[str, Any]:
    if profile is None:
        return {
            "status": "no_profile",
            "needs_processing": False,
            "last_processed_at": None,
            "last_processed_tx_count": 0,
            "journal_input_version": 0,
            "last_processed_input_version": 0,
            "active_transaction_count": 0,
            "journal_entry_count": 0,
            "quarantine_count": 0,
            "reason": "no active profile",
        }

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    journal_entries = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_entries WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    last_processed_at = profile["last_processed_at"]
    last_processed_tx_count = _row_int(profile, "last_processed_tx_count")
    journal_input_version = _row_int(profile, "journal_input_version")
    last_processed_input_version = _row_int(profile, "last_processed_input_version")
    active_count = int(active_transactions or 0)
    if active_count == 0:
        status = "no_transactions"
        reason = "no active transactions"
    elif not last_processed_at:
        status = "not_processed"
        reason = "journals have not been processed"
    elif last_processed_tx_count != active_count:
        status = "stale"
        reason = "active transaction count changed since last processing"
    elif journal_input_version != last_processed_input_version:
        status = "stale"
        reason = "journal inputs changed since last processing"
    else:
        status = "current"
        reason = "journals match the active transaction count and input version"
    return {
        "status": status,
        "needs_processing": status in {"not_processed", "stale"},
        "last_processed_at": last_processed_at,
        "last_processed_tx_count": last_processed_tx_count,
        "journal_input_version": journal_input_version,
        "last_processed_input_version": last_processed_input_version,
        "active_transaction_count": active_count,
        "journal_entry_count": int(journal_entries or 0),
        "quarantine_count": int(quarantines or 0),
        "reason": reason,
    }


def build_review_badges_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    """Cheap unresolved-item counts for the active book's side-nav hints.

    Quarantine count and journal freshness come from a single freshness pass;
    the swap/transfer count is read from the column cached when the matcher runs
    (see ``cache_swap_candidate_count``), so this never triggers the heavy matcher
    itself. ``swaps`` is ``None`` until the matcher has run at least once, which
    the UI renders as "no badge yet" rather than a misleading zero. Takes no
    args — the side nav only ever reflects the active book.
    """
    _context, profile = _active_context_and_profile(conn)
    freshness = _journal_freshness(conn, profile)
    swaps: int | None = None
    if profile is not None:
        try:
            if "swap_candidate_count" in profile.keys():
                raw = profile["swap_candidate_count"]
                swaps = int(raw) if raw is not None else None
        except (IndexError, KeyError, ValueError, TypeError):
            swaps = None
    return {
        "quarantine": int(freshness["quarantine_count"]),
        "journals_needs_processing": bool(freshness["needs_processing"]),
        "swaps": swaps,
    }


def _map_wallet_kind(kind: str) -> str:
    normalized = (kind or "").lower().replace("_", "-")
    aliases = {
        "address": "address",
        "coreln": "core-ln",
        "btcpay_csv": "btcpay",
        "btcpay-json": "btcpay",
        "btcpay-csv": "btcpay",
        "phoenix": "phoenix",
        "phoenix-csv": "csv",
    }
    return aliases.get(normalized, normalized or "csv")


def _public_explorer_id(external_id: str | None) -> str | None:
    candidate = (external_id or "").strip()
    if len(candidate) != 64:
        return None
    if all(char in "0123456789abcdefABCDEF" for char in candidate):
        return candidate
    return None


def _relative_last(value: str | None) -> str:
    if not value:
        return "never"
    try:
        dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return value
    now = datetime.now(timezone.utc)
    delta = now - dt.astimezone(timezone.utc)
    seconds = max(0, int(delta.total_seconds()))
    if seconds < 60:
        return "just now"
    minutes = seconds // 60
    if minutes < 60:
        return f"{minutes}m ago"
    hours = minutes // 60
    if hours < 24:
        return f"{hours}h ago"
    days = hours // 24
    if days < 30:
        return f"{days}d ago"
    return dt.date().isoformat()


def _workspace_jurisdiction(tax_countries: list[str]) -> str:
    normalized = {country.strip().lower() for country in tax_countries if country}
    if not normalized:
        return "Generic"
    if normalized == {"at"}:
        return "Austria"
    if len(normalized) == 1:
        return next(iter(normalized)).upper()
    return "Mixed"


def _tax_policy_label(profile: sqlite3.Row) -> str:
    country = str(profile["tax_country"] or "generic").strip().upper()
    if country == "AT":
        # Show the profile's ACTUAL stored method, never the AT policy default.
        # An Austrian book left on FIFO must read as "Austria - FIFO", not be
        # mislabeled "ATM" — that divergence hid a tax-affecting misconfiguration
        # (the engine computes with the stored method, not the AT default).
        return (
            f"Austria - {_human_tax_method(profile['gains_algorithm'])} - "
            f"{profile['fiat_currency']}"
        )
    elif country == "GENERIC":
        country_label = "Generic"
    else:
        country_label = country
    return (
        f"{country_label} - {profile['gains_algorithm']} - "
        f"{profile['fiat_currency']} - {profile['tax_long_term_days']} day long-term"
    )


def _human_tax_method(value: str) -> str:
    normalized = str(value or "").strip().upper()
    if normalized == "MOVING_AVERAGE_AT":
        return "ATM"
    if normalized == "MOVING_AVERAGE":
        return "moving average"
    return normalized or "unknown"


def _profile_policy_method(profile: sqlite3.Row) -> str:
    # Always reflect the stored gains_algorithm the engine actually uses, for
    # every country — no AT default-substitution that could mask the real method.
    return str(profile["gains_algorithm"] or "fifo").lower()


def build_profiles_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    profile_rows = conn.execute(
        """
        SELECT
            p.id,
            p.workspace_id,
            p.label,
            p.fiat_currency,
            p.tax_country,
            p.tax_long_term_days,
            p.gains_algorithm,
            p.last_processed_at,
            p.created_at,
            COUNT(DISTINCT a.id) AS account_count,
            COUNT(DISTINCT w.id) AS wallet_count
        FROM profiles p
        LEFT JOIN accounts a ON a.profile_id = p.id
        LEFT JOIN wallets w ON w.profile_id = p.id
        GROUP BY p.id
        ORDER BY p.created_at ASC, p.label ASC
        """
    ).fetchall()
    profiles_by_workspace: dict[str, list[dict[str, Any]]] = defaultdict(list)
    tax_countries_by_workspace: dict[str, list[str]] = defaultdict(list)
    currencies_by_workspace: dict[str, list[str]] = defaultdict(list)

    for row in profile_rows:
        is_active = row["id"] == context["profile_id"]
        tax_countries_by_workspace[row["workspace_id"]].append(row["tax_country"])
        currencies_by_workspace[row["workspace_id"]].append(row["fiat_currency"])
        profiles_by_workspace[row["workspace_id"]].append(
            {
                "id": row["id"],
                "name": row["label"],
                "taxPolicy": _tax_policy_label(row),
                "fiatCurrency": row["fiat_currency"],
                "taxCountry": row["tax_country"],
                "taxLongTermDays": int(row["tax_long_term_days"] or 0),
                "gainsAlgorithm": row["gains_algorithm"],
                "accounts": int(row["account_count"] or 0),
                "wallets": int(row["wallet_count"] or 0),
                "lastOpened": (
                    "Just now"
                    if is_active
                    else _relative_last(row["last_processed_at"] or row["created_at"])
                ),
                "active": is_active,
            }
        )

    workspace_rows = conn.execute(
        """
        SELECT id, label, created_at
        FROM workspaces
        ORDER BY created_at ASC, label ASC
        """
    ).fetchall()
    workspaces = []
    for row in workspace_rows:
        currencies = {
            currency.strip().upper()
            for currency in currencies_by_workspace.get(row["id"], [])
            if currency
        }
        workspaces.append(
            {
                "id": row["id"],
                "name": row["label"],
                "currency": next(iter(currencies)) if len(currencies) == 1 else "Mixed",
                "jurisdiction": _workspace_jurisdiction(
                    tax_countries_by_workspace.get(row["id"], []),
                ),
                "created": (row["created_at"] or "")[:10],
                "profiles": profiles_by_workspace.get(row["id"], []),
            }
        )

    return {
        "workspaces": workspaces,
        "activeWorkspaceId": context["workspace_id"],
        "activeProfileId": context["profile_id"],
    }


def _transaction_wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float]:
    rows = conn.execute(
        """
        SELECT
            wallet_id,
            SUM(
                CASE
                    WHEN direction = 'inbound' THEN amount
                    WHEN direction = 'outbound' THEN -amount - fee
                    ELSE 0
                END
            ) AS quantity
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset IN ('BTC', 'LBTC')
        GROUP BY wallet_id
        """,
        (profile_id,),
    ).fetchall()
    return {
        row["wallet_id"]: float(msat_to_btc(row["quantity"] or 0))
        for row in rows
    }


def _connections(
    conn: sqlite3.Connection,
    profile_id: str,
    balances: dict[str, float],
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            w.config_json,
            w.created_at,
            COUNT(t.id) AS tx_count,
            MAX(t.occurred_at) AS last_tx_at
        FROM wallets w
        LEFT JOIN transactions t ON t.wallet_id = w.id AND t.excluded = 0
        WHERE w.profile_id = ?
        GROUP BY w.id
        ORDER BY w.label ASC
        """,
        (profile_id,),
    ).fetchall()
    output = []
    for row in rows:
        tx_count = int(row["tx_count"] or 0)
        config = _json_config(row["config_json"])
        last_synced_at = _string_or_empty(config.get("last_synced_at"))
        backend_summary = _wallet_backend_summary(row["kind"], config, None)
        source_format = _string_or_empty(config.get("source_format"))
        sync_source = _string_or_empty(config.get("sync_source") or source_format)
        gap_limit = config.get("gap_limit")
        has_descriptor = has_descriptor_sync_material(config)
        connection = {
            "id": row["id"],
            "kind": _map_wallet_kind(row["kind"]),
            "label": row["label"],
            "last": _relative_last(
                last_synced_at or row["last_tx_at"] or row["created_at"]
            ),
            "lastSyncAt": last_synced_at or None,
            "lastTransactionAt": row["last_tx_at"],
            "balance": balances.get(row["id"], 0.0),
            "status": "synced" if tx_count else "idle",
            "transactionCount": tx_count,
            "syncMode": backend_summary["sync_mode"],
            "syncSource": sync_source,
            "sourceFormat": source_format,
            "deprecated": wallet_is_deprecated(config),
        }
        if has_descriptor:
            connection["gap"] = (
                int(gap_limit)
                if gap_limit not in (None, "")
                else DEFAULT_DESCRIPTOR_GAP_LIMIT
            )
        output.append(connection)
    return output


def _transactions(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_value,
            t.fiat_rate,
            t.pricing_source_kind,
            t.pricing_quality,
            t.pricing_external_ref,
            t.pricing_provider,
            t.pricing_pair,
            t.pricing_timestamp,
            t.pricing_fetched_at,
            t.pricing_granularity,
            t.pricing_method,
            t.review_status,
            t.taxability_override,
            t.at_regime_override,
            t.at_category_override,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.counterparty, '') AS counterparty,
            COALESCE(t.note, '') AS note,
            t.excluded,
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE t.profile_id = ?
        ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC
        LIMIT 40
        """,
        (profile_id,),
    ).fetchall()
    output = _transaction_rows_to_ui(conn, rows)
    return output[:20]


def _activity_transactions(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_value,
            t.fiat_rate,
            t.pricing_source_kind,
            t.pricing_quality,
            t.pricing_external_ref,
            t.pricing_provider,
            t.pricing_pair,
            t.pricing_timestamp,
            t.pricing_fetched_at,
            t.pricing_granularity,
            t.pricing_method,
            t.review_status,
            t.taxability_override,
            t.at_regime_override,
            t.at_category_override,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.counterparty, '') AS counterparty,
            COALESCE(t.note, '') AS note,
            t.excluded,
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE t.profile_id = ? AND t.asset IN ('BTC', 'LBTC')
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile_id,),
    ).fetchall()
    activity_rows: list[dict[str, Any]] = []
    quantity_msat = 0
    cost_basis_by_transaction = _portfolio_cost_basis_by_transaction(conn, profile_id)
    running_cost_basis = 0.0
    for row in rows:
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        quantity_msat += amount if row["direction"] == "inbound" else -amount - fee
        running_cost_basis = cost_basis_by_transaction.get(
            str(row["id"]),
            running_cost_basis,
        )
        activity_rows.append(
            {
                **dict(row),
                "running_balance_btc": float(msat_to_btc(quantity_msat)),
                "running_cost_basis_eur": running_cost_basis,
            }
        )
    return _activity_transaction_rows_to_ui(conn, activity_rows)


def _transaction_type(kind: str, direction: str, quarantine_reason: str | None) -> str:
    normalized = (kind or "").lower()
    if "transfer" in normalized and direction != "inbound":
        return "Transfer"
    if "swap" in normalized:
        return "Swap"
    if "fee" in normalized:
        return "Fee"
    if quarantine_reason:
        normalized_reason = quarantine_reason.lower()
        if normalized_reason == "missing_fee_price":
            return "Fee"
        if (
            "transfer" in normalized_reason
            or "pair" in normalized_reason
            or "swap" in normalized_reason
        ):
            return "Transfer"
    if direction == "inbound":
        return "Income"
    return "Expense"


def _ui_sat_amount(msat: int) -> int | float:
    amount = int(msat or 0)
    if amount % 1000 == 0:
        return amount // 1000
    return amount / 1000


def _transaction_tags_by_transaction(
    conn: sqlite3.Connection,
    transaction_ids: list[str],
) -> dict[str, list[str]]:
    ids = list(dict.fromkeys(str(tx_id) for tx_id in transaction_ids if tx_id))
    tags_by_transaction: dict[str, list[str]] = {tx_id: [] for tx_id in ids}
    if not ids:
        return tags_by_transaction
    placeholders = ", ".join("?" for _ in ids)
    tag_rows = conn.execute(
        f"""
        SELECT tt.transaction_id, tags.label
        FROM transaction_tags tt
        JOIN tags ON tags.id = tt.tag_id
        WHERE tt.transaction_id IN ({placeholders})
        ORDER BY tt.transaction_id ASC, tags.code ASC
        """,
        ids,
    ).fetchall()
    for tag in tag_rows:
        tags_by_transaction[tag["transaction_id"]].append(tag["label"])
    return tags_by_transaction


def _transaction_pair_display_meta(
    conn: sqlite3.Connection,
    rows: list[sqlite3.Row],
) -> dict[str, dict[str, Any]]:
    if not rows:
        return {}
    ids = [row["id"] for row in rows]
    placeholders = ", ".join("?" for _ in ids)
    pair_rows = conn.execute(
        f"""
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.swap_fee_msat,
            p.swap_fee_kind,
            p.out_transaction_id,
            p.in_transaction_id,
            tout.asset AS out_asset,
            -- Split cross-asset pairs cross only `out_amount`; keep the pair's
            -- out amount consistent with swap_fee_msat (NULL on whole pairs).
            COALESCE(p.out_amount, tout.amount) AS out_amount,
            tout.fiat_rate AS out_fiat_rate,
            tin.asset AS in_asset,
            tin.amount AS in_amount,
            tin.fiat_rate AS in_fiat_rate,
            wout.label AS out_wallet,
            win.label AS in_wallet
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE p.deleted_at IS NULL
          AND (p.out_transaction_id IN ({placeholders})
               OR p.in_transaction_id IN ({placeholders}))
        """,
        [*ids, *ids],
    ).fetchall()
    pair_meta: dict[str, dict[str, Any]] = {}
    for pair in pair_rows:
        out_asset = pair["out_asset"]
        in_asset = pair["in_asset"]
        pair_type = "transfer" if out_asset == in_asset else "swap"
        raw_fee_msat = pair["swap_fee_msat"]
        if raw_fee_msat is None:
            raw_fee_msat = int(pair["out_amount"] or 0) - int(pair["in_amount"] or 0)
        fee_msat = int(raw_fee_msat or 0)
        label = "Transfer" if pair_type == "transfer" else "Swap"
        counter = f"{label} fee - {out_asset} -> {in_asset}"
        account = f"{pair['out_wallet']} -> {pair['in_wallet']}"
        raw_display_rate = (
            pair["out_fiat_rate"]
            if pair["out_fiat_rate"] is not None
            else pair["in_fiat_rate"]
        )
        display_rate = _positive_float_or_none(raw_display_rate)
        base = {
            "pair_id": pair["id"],
            "out_transaction_id": pair["out_transaction_id"],
            "in_transaction_id": pair["in_transaction_id"],
            "pair_type": pair_type,
            "kind": pair["kind"],
            "policy": pair["policy"],
            "label": label,
            "counter": counter,
            "account": account,
            "fee_msat": fee_msat,
            "fee_kind": pair["swap_fee_kind"],
            "out_asset": out_asset,
            "out_amount_msat": int(pair["out_amount"] or 0),
            "out_wallet": pair["out_wallet"],
            "in_asset": in_asset,
            "in_amount_msat": int(pair["in_amount"] or 0),
            "in_wallet": pair["in_wallet"],
            "tag": label,
            "display_rate": display_rate,
        }
        pair_meta[pair["out_transaction_id"]] = {**base, "role": "out"}
        pair_meta[pair["in_transaction_id"]] = {**base, "role": "in"}
    return pair_meta


def _positive_float_or_none(value: Any) -> float | None:
    if value is None:
        return None
    parsed = float(value)
    return parsed if parsed > 0 else None


def _transaction_row_to_ui(
    row: sqlite3.Row,
    metadata_tags: list[str],
    pair_meta: dict[str, Any] | None = None,
) -> dict[str, Any]:
    fee_msat = int(row["fee"] or 0)
    rate = _positive_float_or_none(row["fiat_rate"])
    if pair_meta:
        rate = _positive_float_or_none(pair_meta["display_rate"])
        display_fee_msat = abs(int(pair_meta["fee_msat"] or 0))
        out_amount_msat = abs(int(pair_meta.get("out_amount_msat") or 0))
        in_amount_msat = abs(int(pair_meta.get("in_amount_msat") or 0))
        pair_tag = str(pair_meta["tag"])
        display_tags = [pair_tag, *[tag for tag in metadata_tags if tag != pair_tag]]
        if pair_meta["pair_type"] == "swap":
            amount_msat = -max(out_amount_msat, in_amount_msat)
            fiat_value = (
                -float(msat_to_btc(abs(amount_msat))) * rate
                if rate is not None
                else None
            )
            counter = f"Swap {pair_meta['out_asset']} -> {pair_meta['in_asset']}"
        else:
            amount_msat = -display_fee_msat
            fiat_value = (
                -float(msat_to_btc(display_fee_msat)) * rate
                if rate is not None
                else None
            )
            counter = str(pair_meta["counter"])
        type_label = str(pair_meta["label"])
        account = str(pair_meta["account"])
        internal = pair_meta["pair_type"] == "transfer"
        output_tags = metadata_tags
        fee_sat = _ui_sat_amount(display_fee_msat)
        row_id = row["id"]
        external_id = row["external_id"]
        occurred_at = row["occurred_at"]
        confirmed_at = row["confirmed_at"]
        note = row["note"] or ""
        excluded = bool(row["excluded"])
        include_empty_tags = False
    else:
        sign = 1 if row["direction"] == "inbound" else -1
        amount_msat = sign * int(row["amount"] or 0)
        raw_fiat_value = _positive_float_or_none(row["fiat_value"])
        fiat_value = (
            sign * abs(raw_fiat_value) if raw_fiat_value is not None else None
        )
        type_label = _transaction_type(
            row["kind"],
            row["direction"],
            row["quarantine_reason"],
        )
        if (
            row["direction"] == "outbound"
            and int(row["amount"] or 0) == 0
            and fee_msat > 0
        ):
            amount_msat = -fee_msat
            fiat_value = (
                -float(msat_to_btc(fee_msat)) * rate
                if rate is not None
                else fiat_value
            )
            type_label = "Fee"
        display_tags = list(metadata_tags)
        if not display_tags and row["quarantine_reason"]:
            display_tags = ["Review"]
        elif not display_tags:
            display_tags = [
                type_label
                if type_label != "Expense"
                else (row["kind"] or row["direction"])
            ]
        counter = (
            row["counterparty"]
            or row["description"]
            or row["note"]
            or row["external_id"]
            or row["id"]
        )
        account = row["wallet"]
        internal = (row["kind"] or "").lower() == "transfer"
        output_tags = metadata_tags
        fee_sat = _ui_sat_amount(fee_msat) if fee_msat else 0
        row_id = row["id"]
        external_id = row["external_id"]
        occurred_at = row["occurred_at"]
        confirmed_at = row["confirmed_at"]
        note = row["note"] or ""
        excluded = bool(row["excluded"])
        include_empty_tags = True

    row_keys = set(row.keys())
    payload = {
        "id": row_id,
        "externalId": external_id,
        "explorerId": _public_explorer_id(external_id),
        "date": (occurred_at or "")[:16].replace("T", " "),
        "type": type_label,
        "account": account,
        "counter": counter,
        "amountSat": _ui_sat_amount(amount_msat),
        "feeSat": fee_sat,
        "eur": fiat_value,
        "rate": rate,
        "fiatCurrency": row["fiat_currency"] or None,
        "pricingSourceKind": row["pricing_source_kind"] or None,
        "pricingQuality": row["pricing_quality"] or None,
        "pricingExternalRef": row["pricing_external_ref"] or None,
        "pricingProvider": row["pricing_provider"] if "pricing_provider" in row_keys else None,
        "pricingPair": row["pricing_pair"] if "pricing_pair" in row_keys else None,
        "pricingTimestamp": row["pricing_timestamp"] if "pricing_timestamp" in row_keys else None,
        "pricingFetchedAt": row["pricing_fetched_at"] if "pricing_fetched_at" in row_keys else None,
        "pricingGranularity": row["pricing_granularity"] if "pricing_granularity" in row_keys else None,
        "pricingMethod": row["pricing_method"] if "pricing_method" in row_keys else None,
        "reviewStatus": row["review_status"] if "review_status" in row_keys else None,
        "taxable": (
            None
            if "taxability_override" not in row_keys or row["taxability_override"] is None
            else bool(row["taxability_override"])
        ),
        "atRegime": row["at_regime_override"] if "at_regime_override" in row_keys else None,
        "atCategory": row["at_category_override"] if "at_category_override" in row_keys else None,
        "tag": ", ".join(display_tags) or "Unlabeled",
        "note": note,
        "excluded": excluded,
        "conf": 1 if confirmed_at else 0,
        "internal": internal,
    }
    if output_tags or include_empty_tags:
        payload["tags"] = output_tags
    if pair_meta:
        payload["pair"] = {
            "id": pair_meta["pair_id"],
            "type": pair_meta["pair_type"],
            "kind": pair_meta.get("kind"),
            "policy": pair_meta.get("policy"),
            "outWallet": pair_meta.get("out_wallet"),
            "outAsset": pair_meta.get("out_asset"),
            "outAmountSat": _ui_sat_amount(pair_meta.get("out_amount_msat") or 0),
            "inWallet": pair_meta.get("in_wallet"),
            "inAsset": pair_meta.get("in_asset"),
            "inAmountSat": _ui_sat_amount(pair_meta.get("in_amount_msat") or 0),
            "feeSat": fee_sat,
            "feeKind": pair_meta.get("fee_kind"),
        }
    if "quarantine_reason" in row_keys and row["quarantine_reason"]:
        payload["quarantineReason"] = row["quarantine_reason"]
    if occurred_at:
        payload["occurredAt"] = occurred_at
    if "running_balance_btc" in row_keys:
        payload["balanceBtc"] = float(row["running_balance_btc"] or 0)
    if "running_cost_basis_eur" in row_keys:
        payload["costBasisEur"] = float(row["running_cost_basis_eur"] or 0)
    return payload


def _balance_series(conn: sqlite3.Connection, profile_id: str) -> list[float]:
    rows = conn.execute(
        """
        SELECT occurred_at, direction, amount, fee
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    if not rows:
        return [0.0] * 12
    by_month: dict[str, int] = defaultdict(int)
    for row in rows:
        key = (row["occurred_at"] or "")[:7]
        if not key:
            continue
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        by_month[key] += amount if row["direction"] == "inbound" else -amount - fee
    months = sorted(by_month)[-12:]
    cumulative = 0
    series = []
    for month in sorted(by_month):
        cumulative += by_month[month]
        if month in months:
            series.append(float(msat_to_btc(cumulative)))
    if len(series) < 12:
        series = [series[0]] * (12 - len(series)) + series
    return series[-12:]


def _rate_from_transaction(row: sqlite3.Row) -> float | None:
    if row["fiat_rate"] is not None:
        return float(row["fiat_rate"])
    if row["fiat_value"] is not None and row["amount"]:
        amount_btc = float(msat_to_btc(row["amount"]))
        if amount_btc:
            return abs(float(row["fiat_value"])) / amount_btc
    return None


def _portfolio_cost_basis_by_date(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float]:
    rows = conn.execute(
        """
        SELECT occurred_at, quantity, fiat_value, COALESCE(cost_basis, 0) AS cost_basis
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    cost_basis = 0.0
    by_date: dict[str, float] = {}
    for row in rows:
        date_key = (row["occurred_at"] or "")[:10]
        if not date_key:
            continue
        quantity = int(row["quantity"] or 0)
        if quantity >= 0:
            cost_basis += float(row["fiat_value"] or 0)
        else:
            cost_basis -= float(row["cost_basis"] or 0)
        by_date[date_key] = cost_basis
    return by_date


def _parse_day(value: str | None) -> date | None:
    if not value:
        return None
    try:
        return date.fromisoformat(str(value)[:10])
    except ValueError:
        return None


def _daily_rate_rows(
    conn: sqlite3.Connection,
    pair: str | None,
    start_day: date,
) -> list[sqlite3.Row]:
    if not pair:
        return []
    return conn.execute(
        """
        WITH ranked AS (
            SELECT
                substr(timestamp, 1, 10) AS rate_day,
                timestamp,
                rate,
                source,
                fetched_at,
                granularity,
                method,
                ROW_NUMBER() OVER (
                    PARTITION BY substr(timestamp, 1, 10)
                    ORDER BY CASE
                                 WHEN source = 'manual'
                                      AND timestamp LIKE substr(timestamp, 1, 10) || 'T00:00:00%'
                                      THEN 0
                                 WHEN granularity = 'daily' THEN 1
                                 ELSE 2
                             END ASC,
                             timestamp DESC,
                             CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                             fetched_at DESC,
                             source ASC
                ) AS rn
            FROM rates_cache
            WHERE pair = ?
              AND timestamp >= ?
        )
        SELECT rate_day, timestamp, rate, source, fetched_at, granularity, method
        FROM ranked
        WHERE rn = 1
        ORDER BY rate_day ASC
        """,
        (pair, start_day.isoformat()),
    ).fetchall()


def _portfolio_cost_basis_by_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float]:
    rows = conn.execute(
        """
        SELECT transaction_id, quantity, fiat_value, cost_basis
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    cost_basis = 0.0
    by_transaction: dict[str, float] = {}
    for row in rows:
        transaction_id = row["transaction_id"]
        if not transaction_id:
            continue
        quantity = int(row["quantity"] or 0)
        if quantity >= 0:
            cost_basis += float(row["fiat_value"] or 0)
        else:
            cost_basis -= float(row["cost_basis"] or 0)
        by_transaction[str(transaction_id)] = cost_basis
    return by_transaction


def _current_portfolio_cost_basis(
    conn: sqlite3.Connection,
    profile_id: str,
) -> float:
    rows = conn.execute(
        """
        SELECT quantity, fiat_value, cost_basis
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    cost_basis = 0.0
    for row in rows:
        quantity = int(row["quantity"] or 0)
        if quantity >= 0:
            cost_basis += float(row["fiat_value"] or 0)
        else:
            cost_basis -= float(row["cost_basis"] or 0)
    return cost_basis


def _portfolio_series(
    conn: sqlite3.Connection,
    profile_id: str,
    fiat_currency: str,
    fallback_rate: float,
    final_balance_btc: float,
    final_value_eur: float,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT occurred_at, direction, amount, fee, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    if not rows:
        return []

    cost_basis_by_date = _portfolio_cost_basis_by_date(conn, profile_id)
    tx_deltas_by_day: dict[date, int] = defaultdict(int)
    for row in rows:
        day = _parse_day(row["occurred_at"])
        if day is None:
            continue
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        tx_deltas_by_day[day] += (
            amount if row["direction"] == "inbound" else -amount - fee
        )

    sorted_tx_days = sorted(tx_deltas_by_day)
    if not sorted_tx_days:
        return []

    pair = core_rates.transaction_rate_pair("BTC", fiat_currency)
    daily_rates = _daily_rate_rows(conn, pair, sorted_tx_days[0])
    if daily_rates:
        cost_basis_items = sorted(
            (day, value)
            for raw_day, value in cost_basis_by_date.items()
            if (day := _parse_day(raw_day)) is not None
        )
        quantity_msat = 0
        tx_index = 0
        cost_basis_index = 0
        day_cost_basis = 0.0
        output: list[dict[str, Any]] = []

        for rate_row in daily_rates:
            rate_day = _parse_day(rate_row["rate_day"])
            if rate_day is None:
                continue
            while tx_index < len(sorted_tx_days) and sorted_tx_days[tx_index] <= rate_day:
                quantity_msat += tx_deltas_by_day[sorted_tx_days[tx_index]]
                tx_index += 1
            while (
                cost_basis_index < len(cost_basis_items)
                and cost_basis_items[cost_basis_index][0] <= rate_day
            ):
                day_cost_basis = cost_basis_items[cost_basis_index][1]
                cost_basis_index += 1

            balance_btc = float(msat_to_btc(quantity_msat))
            price_eur = float(rate_row["rate"] or 0)
            day_key = rate_day.isoformat()
            output.append(
                {
                    "date": day_key,
                    "label": day_key,
                    "balanceBtc": balance_btc,
                    "valueEur": balance_btc * price_eur,
                    "costBasisEur": day_cost_basis,
                    "priceEur": price_eur,
                    "priceTimestamp": rate_row["timestamp"],
                    "priceSource": rate_row["source"],
                }
            )

        last_tx_day = sorted_tx_days[-1]
        last_output_day = _parse_day(output[-1]["date"]) if output else None
        if last_output_day is not None and last_tx_day > last_output_day:
            while tx_index < len(sorted_tx_days):
                quantity_msat += tx_deltas_by_day[sorted_tx_days[tx_index]]
                tx_index += 1
            while cost_basis_index < len(cost_basis_items):
                day_cost_basis = cost_basis_items[cost_basis_index][1]
                cost_basis_index += 1
            rate = fallback_rate or float(daily_rates[-1]["rate"] or 0)
            output.append(
                {
                    "date": last_tx_day.isoformat(),
                    "label": last_tx_day.isoformat(),
                    "balanceBtc": final_balance_btc,
                    "valueEur": (
                        final_value_eur
                        if fallback_rate
                        else float(msat_to_btc(quantity_msat)) * rate
                    ),
                    "costBasisEur": day_cost_basis,
                    "priceEur": rate,
                }
            )

        return output

    quantity_msat = 0
    latest_rate = fallback_rate
    output: list[dict[str, Any]] = []
    current_date = ""
    day_cost_basis = 0.0

    for row in rows:
        date_key = (row["occurred_at"] or "")[:10]
        if not date_key:
            continue
        if current_date and date_key != current_date:
            balance_btc = float(msat_to_btc(quantity_msat))
            output.append(
                {
                    "date": current_date,
                    "label": current_date,
                    "balanceBtc": balance_btc,
                    "valueEur": balance_btc * latest_rate,
                    "costBasisEur": day_cost_basis,
                }
            )

        current_date = date_key
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        quantity_msat += amount if row["direction"] == "inbound" else -amount - fee
        row_rate = _rate_from_transaction(row)
        if row_rate is not None:
            latest_rate = row_rate
        day_cost_basis = cost_basis_by_date.get(date_key, day_cost_basis)

    if current_date:
        output.append(
            {
                "date": current_date,
                "label": current_date,
                "balanceBtc": final_balance_btc,
                "valueEur": final_value_eur,
                "costBasisEur": day_cost_basis,
                "priceEur": fallback_rate or latest_rate,
            }
        )
    return output


def _fiat_snapshot(
    conn: sqlite3.Connection,
    profile_id: str,
    fiat_currency: str,
    fiat_rate: float,
    balances: dict[str, float],
) -> dict[str, Any]:
    market_value = sum(balances.values()) * fiat_rate
    realized_row = conn.execute(
        """
        SELECT SUM(COALESCE(gain_loss, 0)) AS gain_loss
        FROM journal_entries
        WHERE profile_id = ?
          AND gain_loss IS NOT NULL
          AND occurred_at >= date('now', 'start of year')
        """,
        (profile_id,),
    ).fetchone()
    cost_basis = _current_portfolio_cost_basis(conn, profile_id)
    return {
        "fiatCurrency": str(fiat_currency or "EUR").upper(),
        "eurBalance": float(market_value),
        "eurCostBasis": cost_basis,
        "eurUnrealized": float(market_value - cost_basis),
        "eurRealizedYTD": float(realized_row["gain_loss"] or 0),
    }


def _profile_readiness(
    *,
    wallet_count: int,
    transaction_count: int,
    freshness: dict[str, Any],
) -> dict[str, Any]:
    hints: list[str] = []
    if wallet_count == 0:
        hints.append("Add a watch-only source before refreshing or importing transactions.")
    elif transaction_count == 0:
        hints.append("Refresh sources or import files before journal processing.")
    if freshness["needs_processing"]:
        hints.append("Run journal processing before trusting reports.")
    if freshness["quarantine_count"]:
        hints.append("Review quarantined transactions before tax export.")
    ready = (
        wallet_count > 0
        and transaction_count > 0
        and freshness["status"] == "current"
        and freshness["quarantine_count"] == 0
    )
    if ready:
        hints.append("Reports are ready from the current processed journal state.")
    return {"ready": ready, "hints": hints}


def _readiness_ready(readiness: dict[str, Any]) -> bool:
    return bool(readiness.get("ready"))


def _build_profile_overview_snapshot(
    conn: sqlite3.Connection,
    *,
    workspace: sqlite3.Row,
    profile: sqlite3.Row,
) -> dict[str, Any]:
    freshness = _journal_freshness(conn, profile)

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    price_eur = _latest_rate(conn, "BTC-EUR") or _latest_transaction_rate(
        conn,
        profile["id"],
        "EUR",
    )
    price_usd = _latest_rate(conn, "BTC-USD") or _latest_transaction_rate(
        conn,
        profile["id"],
        "USD",
    )
    market_rate = _market_rate_snapshot(conn, profile["fiat_currency"])
    book_fiat_rate = (
        float(market_rate["rate"])
        if market_rate["rate"] is not None
        else _latest_transaction_rate(
            conn,
            profile["id"],
            market_rate["fiatCurrency"],
        )
    )
    # Connection tiles are a wallet/source status surface, not a tax-report
    # surface. Use raw synced transactions so quarantined or partially
    # processed journal rows do not make a wallet with imported funds look
    # empty in the GUI.
    balances = _transaction_wallet_balances(conn, profile["id"])
    fiat = _fiat_snapshot(
        conn,
        profile["id"],
        profile["fiat_currency"],
        book_fiat_rate,
        balances,
    )
    snapshot = {
        "priceEur": price_eur,
        "priceUsd": price_usd,
        "marketRate": market_rate,
        "connections": _connections(conn, profile["id"], balances),
        "txs": _transactions(conn, profile["id"]),
        "activityTxs": _activity_transactions(conn, profile["id"]),
        "balanceSeries": _balance_series(conn, profile["id"]),
        "portfolioSeries": _portfolio_series(
            conn,
            profile["id"],
            profile["fiat_currency"],
            book_fiat_rate,
            sum(balances.values()),
            fiat["eurBalance"],
        ),
        "fiat": fiat,
        "status": {
            "workspace": workspace["label"],
            "profile": profile["label"],
            "transactionCount": int(active_transactions or 0),
            "needsJournals": freshness["needs_processing"],
            "quarantines": freshness["quarantine_count"],
        },
    }
    return snapshot


def _profile_overview_for_status(
    conn: sqlite3.Connection,
    *,
    workspace: sqlite3.Row,
    profile: sqlite3.Row,
) -> dict[str, Any]:
    snapshot = _build_profile_overview_snapshot(
        conn,
        workspace=workspace,
        profile=profile,
    )
    wallet_count = len(snapshot["connections"])
    transaction_count = int(snapshot["status"].get("transactionCount") or 0)
    freshness = _journal_freshness(conn, profile)
    readiness = _profile_readiness(
        wallet_count=wallet_count,
        transaction_count=transaction_count,
        freshness=freshness,
    )
    return {
        "profile": {
            "id": profile["id"],
            "label": profile["label"],
            "fiatCurrency": str(profile["fiat_currency"] or "EUR").upper(),
            "taxCountry": profile["tax_country"],
            "taxLongTermDays": int(profile["tax_long_term_days"] or 0),
            "gainsAlgorithm": profile["gains_algorithm"],
        },
        "workspace": {
            "id": workspace["id"],
            "label": workspace["label"],
        },
        "connections": snapshot["connections"],
        "txs": snapshot["txs"],
        "activityTxs": snapshot.get("activityTxs", []),
        "balanceSeries": snapshot["balanceSeries"],
        "portfolioSeries": snapshot.get("portfolioSeries", []),
        "fiat": snapshot["fiat"],
        "marketRate": snapshot.get("marketRate"),
        "status": {
            **snapshot["status"],
            "workspaceId": workspace["id"],
            "profileId": profile["id"],
            "workspace": workspace["label"],
            "profile": profile["label"],
            "journalEntryCount": freshness["journal_entry_count"],
            "freshnessStatus": freshness["status"],
            "freshnessReason": freshness["reason"],
        },
        "journals": freshness,
        "readiness": readiness,
    }


def build_overview_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return _empty_overview_snapshot()

    workspace = conn.execute(
        "SELECT * FROM workspaces WHERE id = ?",
        (context["workspace_id"],),
    ).fetchone()
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    if workspace is None or profile is None:
        return _empty_overview_snapshot()
    return _build_profile_overview_snapshot(
        conn,
        workspace=workspace,
        profile=profile,
    )


def _workspace_overview_args(args: dict[str, Any] | None) -> str:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"workspace_id"})
    if unknown:
        raise AppError(
            "ui.workspace.overview.snapshot received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    workspace_id = raw_args.get("workspace_id")
    if not isinstance(workspace_id, str) or not workspace_id.strip():
        raise AppError(
            "ui.workspace.overview.snapshot requires args.workspace_id",
            code="validation",
            retryable=False,
        )
    return workspace_id.strip()


def _workspace_overview_row(
    conn: sqlite3.Connection,
    workspace_id: str,
) -> sqlite3.Row:
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ?",
        (workspace_id,),
    ).fetchone()
    if row is None:
        raise AppError(
            f"Book set '{workspace_id}' was not found",
            code="not_found",
            retryable=False,
        )
    return row


def _with_book_boundary(
    items: list[dict[str, Any]],
    *,
    workspace: sqlite3.Row,
    profile: sqlite3.Row,
) -> list[dict[str, Any]]:
    return [
        {
            **dict(item),
            "workspaceId": workspace["id"],
            "workspaceLabel": workspace["label"],
            "profileId": profile["id"],
            "profileLabel": profile["label"],
            "book": {
                "id": profile["id"],
                "label": profile["label"],
            },
        }
        for item in items
    ]


def _sum_balance_series(books: list[dict[str, Any]]) -> list[float]:
    totals = [0.0] * 12
    for book in books:
        series = list(book.get("balanceSeries") or [])
        if len(series) < 12:
            series = [series[0] if series else 0.0] * (12 - len(series)) + series
        for index, value in enumerate(series[-12:]):
            totals[index] += float(value or 0)
    return totals


def _workspace_portfolio_series(
    books: list[dict[str, Any]],
    *,
    same_fiat: bool,
) -> list[dict[str, Any]]:
    book_points: list[tuple[dict[str, Any], list[dict[str, Any]]]] = []
    for book in books:
        points = sorted(
            [
                dict(point)
                for point in (book.get("portfolioSeries") or [])
                if str(point.get("date") or "")
            ],
            key=lambda point: str(point.get("date") or ""),
        )
        if points:
            book_points.append((book, points))
    date_keys = sorted(
        {
            str(point.get("date") or "")
            for _, points in book_points
            for point in points
            if point.get("date")
        }
    )
    if not date_keys:
        return []

    indexes = {book["profile"]["id"]: 0 for book, _ in book_points}
    last_points: dict[str, dict[str, Any]] = {}
    output: list[dict[str, Any]] = []
    for date_key in date_keys:
        aggregate: dict[str, Any] = {
            "date": date_key,
            "label": date_key,
            "balanceBtc": 0.0,
            "books": [],
        }
        if same_fiat:
            aggregate["valueEur"] = 0.0
            aggregate["costBasisEur"] = 0.0
        carried_books: list[dict[str, Any]] = []
        for book, points in book_points:
            profile = book["profile"]
            profile_id = profile["id"]
            index = indexes[profile_id]
            while index < len(points) and str(points[index].get("date") or "") <= date_key:
                last_points[profile_id] = points[index]
                index += 1
            indexes[profile_id] = index
            point = last_points.get(profile_id)
            if point is None:
                continue
            balance_btc = float(point.get("balanceBtc") or 0)
            value = float(point.get("valueEur") or 0)
            cost_basis = float(point.get("costBasisEur") or 0)
            aggregate["balanceBtc"] += balance_btc
            if same_fiat:
                aggregate["valueEur"] += value
                aggregate["costBasisEur"] += cost_basis
            carried_books.append(
                {
                    "profileId": profile_id,
                    "profileLabel": profile["label"],
                    "fiatCurrency": profile["fiatCurrency"],
                    "balanceBtc": balance_btc,
                    "value": value,
                    "costBasis": cost_basis,
                }
            )
        aggregate["books"] = carried_books
        if same_fiat and aggregate["balanceBtc"]:
            aggregate["priceEur"] = aggregate["valueEur"] / aggregate["balanceBtc"]
        output.append(aggregate)
    return output


def _workspace_fiat_rollup(
    books: list[dict[str, Any]],
    *,
    currencies: list[str],
) -> dict[str, Any]:
    btc_balance = sum(
        sum(float(connection.get("balance") or 0) for connection in book["connections"])
        for book in books
    )
    book_rows = [
        {
            "profileId": book["profile"]["id"],
            "profileLabel": book["profile"]["label"],
            "fiatCurrency": book["profile"]["fiatCurrency"],
            "balance": float(book["fiat"].get("eurBalance") or 0),
            "costBasis": float(book["fiat"].get("eurCostBasis") or 0),
            "unrealized": float(book["fiat"].get("eurUnrealized") or 0),
            "realizedYTD": float(book["fiat"].get("eurRealizedYTD") or 0),
        }
        for book in books
    ]
    if not books:
        return _empty_workspace_overview_snapshot()["fiat"]
    if len(currencies) == 1:
        return {
            "mode": "single",
            "fiatCurrency": currencies[0],
            "currencies": currencies,
            "mixed": False,
            "partial": False,
            "eurBalance": sum(row["balance"] for row in book_rows),
            "eurCostBasis": sum(row["costBasis"] for row in book_rows),
            "eurUnrealized": sum(row["unrealized"] for row in book_rows),
            "eurRealizedYTD": sum(row["realizedYTD"] for row in book_rows),
            "btcBalance": btc_balance,
            "books": book_rows,
        }
    return {
        "mode": "mixed",
        "fiatCurrency": None,
        "currencies": currencies,
        "mixed": True,
        "partial": True,
        "eurBalance": None,
        "eurCostBasis": None,
        "eurUnrealized": None,
        "eurRealizedYTD": None,
        "btcBalance": btc_balance,
        "books": book_rows,
        "label": "Mixed fiat currencies; per-book fiat rows are shown without conversion.",
    }


def build_workspace_overview_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    workspace_id = _workspace_overview_args(args)
    workspace = _workspace_overview_row(conn, workspace_id)
    profile_rows = conn.execute(
        """
        SELECT *
        FROM profiles
        WHERE workspace_id = ?
        ORDER BY created_at ASC, label ASC
        """,
        (workspace["id"],),
    ).fetchall()
    if not profile_rows:
        return _empty_workspace_overview_snapshot(workspace)

    books = [
        _profile_overview_for_status(conn, workspace=workspace, profile=profile)
        for profile in profile_rows
    ]
    currencies = sorted(
        {
            str(book["profile"]["fiatCurrency"] or "").upper()
            for book in books
            if book["profile"].get("fiatCurrency")
        }
    )
    same_fiat = len(currencies) == 1
    connections: list[dict[str, Any]] = []
    txs: list[dict[str, Any]] = []
    activity_txs: list[dict[str, Any]] = []
    for book in books:
        profile = next(row for row in profile_rows if row["id"] == book["profile"]["id"])
        connections.extend(
            _with_book_boundary(book["connections"], workspace=workspace, profile=profile)
        )
        txs.extend(_with_book_boundary(book["txs"], workspace=workspace, profile=profile))
        activity_txs.extend(
            _with_book_boundary(
                book.get("activityTxs") or [],
                workspace=workspace,
                profile=profile,
            )
        )

    def recent_key(item: dict[str, Any]) -> tuple[str, str]:
        return (
            str(item.get("occurredAt") or item.get("date") or ""),
            str(item.get("id") or ""),
        )

    txs = sorted(txs, key=recent_key, reverse=True)[:20]
    activity_txs = sorted(activity_txs, key=recent_key, reverse=True)[:20]
    total_transactions = sum(
        int(book["status"].get("transactionCount") or 0) for book in books
    )
    quarantine_count = sum(
        int(book["journals"].get("quarantine_count") or 0) for book in books
    )
    needs_journals = any(bool(book["journals"].get("needs_processing")) for book in books)
    ready_books = sum(1 for book in books if _readiness_ready(book["readiness"]))
    blocked_books = len(books) - ready_books
    return {
        "workspace": {
            "id": workspace["id"],
            "label": workspace["label"],
        },
        "scope": {
            "kind": "workspace",
            "label": "Book set",
        },
        "books": books,
        "connections": connections,
        "txs": txs,
        "activityTxs": activity_txs,
        "balanceSeries": _sum_balance_series(books),
        "portfolioSeries": _workspace_portfolio_series(books, same_fiat=same_fiat),
        "fiat": _workspace_fiat_rollup(books, currencies=currencies),
        "status": {
            "workspace": workspace["label"],
            "workspaceId": workspace["id"],
            "bookCount": len(books),
            "transactionCount": total_transactions,
            "needsJournals": needs_journals,
            "quarantines": quarantine_count,
            "ready": blocked_books == 0,
            "readyBooks": ready_books,
            "blockedBooks": blocked_books,
            "mixedFiat": not same_fiat,
        },
    }


def _build_transactions_page_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None,
    *,
    kind: str,
    default_limit: int,
    maximum_limit: int,
    require_query: bool = False,
) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        filters: dict[str, Any] = {"limit": 0}
        if require_query:
            filters["query"] = ""
        return {
            "txs": [],
            "year": datetime.now(timezone.utc).year,
            "filters": filters,
            "nextCursor": None,
            "hasMore": False,
        }

    raw_args = _coerce_args(args)
    unknown = sorted(
        set(raw_args)
        - {
            "limit",
            "cursor",
            "query",
            "direction",
            "asset",
            "wallet",
            "since",
            "until",
            "sort",
            "order",
        }
    )
    if unknown:
        raise AppError(
            f"{kind} received unsupported filters",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    query = raw_args.get("query")
    if query is not None:
        if not isinstance(query, str) or not query.strip():
            raise AppError(
                f"{kind} query must be a non-empty string",
                code="validation",
                retryable=False,
            )
        query = query.strip()
    elif require_query:
        raise AppError(
            f"{kind} query must be a non-empty string",
            code="validation",
            retryable=False,
        )

    limit = _coerce_limit(raw_args, default=default_limit, maximum=maximum_limit)
    filters = ["t.profile_id = ?"]
    params: list[Any] = [context["profile_id"]]
    direction = raw_args.get("direction")
    if direction is not None:
        if direction not in {"inbound", "outbound"}:
            raise AppError(
                "direction must be inbound or outbound",
                code="validation",
                details={"direction": direction},
                retryable=False,
            )
        filters.append("t.direction = ?")
        params.append(direction)
    asset = raw_args.get("asset")
    asset_filter = None
    if asset is not None:
        if not isinstance(asset, str) or not asset.strip():
            raise AppError("asset must be a non-empty string", code="validation")
        asset_filter = asset.strip().upper()
        filters.append("upper(t.asset) = ?")
        params.append(asset_filter)
    wallet = raw_args.get("wallet")
    wallet_filter = None
    if wallet is not None:
        if not isinstance(wallet, str) or not wallet.strip():
            raise AppError("wallet must be a non-empty string", code="validation")
        wallet_filter = wallet.strip()
        filters.append("(t.wallet_id = ? OR lower(w.label) = lower(?))")
        params.extend([wallet_filter, wallet_filter])
    since = raw_args.get("since")
    since_filter = None
    if since is not None:
        if not isinstance(since, str) or not since.strip():
            raise AppError("since must be an RFC3339 timestamp", code="validation")
        since_filter = _iso_z(_parse_iso_datetime(since, "since"))
        filters.append("t.occurred_at >= ?")
        params.append(since_filter)
    until = raw_args.get("until")
    until_filter = None
    if until is not None:
        if not isinstance(until, str) or not until.strip():
            raise AppError("until must be an RFC3339 timestamp", code="validation")
        until_filter = _iso_z(_parse_iso_datetime(until, "until"))
        filters.append("t.occurred_at <= ?")
        params.append(until_filter)
    if query is not None:
        filters.append(
            """
            (
              lower(t.id) LIKE ?
              OR lower(COALESCE(t.external_id, '')) LIKE ?
              OR lower(COALESCE(t.kind, '')) LIKE ?
              OR lower(COALESCE(t.description, '')) LIKE ?
              OR lower(COALESCE(t.counterparty, '')) LIKE ?
              OR lower(COALESCE(t.note, '')) LIKE ?
              OR lower(w.label) LIKE ?
              OR EXISTS (
                SELECT 1
                FROM transaction_tags tt
                JOIN tags ON tags.id = tt.tag_id
                WHERE tt.transaction_id = t.id
                  AND lower(tags.label) LIKE ?
              )
            )
            """
        )
        like = f"%{query.lower()}%"
        params.extend([like] * 8)

    sort = raw_args.get("sort", "occurred-at")
    sort_column = _UI_TRANSACTION_SORT_COLUMNS.get(sort)
    if sort_column is None:
        raise AppError(
            "sort must be one of: occurred-at, amount, fiat-value, fee",
            code="validation",
            details={"sort": sort},
            retryable=False,
        )
    order = raw_args.get("order", "desc")
    if order not in {"asc", "desc"}:
        raise AppError(
            "order must be asc or desc",
            code="validation",
            details={"order": order},
            retryable=False,
        )
    order_sql = str(order).upper()
    if sort == "occurred-at":
        order_by = f"t.occurred_at {order_sql}, t.created_at {order_sql}, t.id {order_sql}"
    else:
        order_by = (
            f"{sort_column} {order_sql}, "
            "t.occurred_at DESC, t.created_at DESC, t.id DESC"
        )
    cursor_filters = _ui_transaction_cursor_filters(
        context,
        direction=direction,
        asset=asset_filter,
        wallet=wallet_filter,
        since=since_filter,
        until=until_filter,
        query=query,
    )
    cursor_data = _decode_ui_transaction_cursor(
        raw_args.get("cursor"),
        sort=sort,
        order=order,
        filters=cursor_filters,
    )
    skip_pairs = set(cursor_data.get("skip_pairs", [])) if cursor_data else set()
    if cursor_data:
        if sort == "occurred-at":
            op = ">" if order == "asc" else "<"
            filters.append(
                f"(t.occurred_at {op} ? OR "
                f"(t.occurred_at = ? AND t.created_at {op} ?) OR "
                f"(t.occurred_at = ? AND t.created_at = ? AND t.id {op} ?))"
            )
            params.extend(
                [
                    cursor_data["occurred_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["id"],
                ]
            )
        else:
            primary_op = ">" if order == "asc" else "<"
            filters.append(
                f"({sort_column} {primary_op} ? OR "
                f"({sort_column} = ? AND "
                "(t.occurred_at < ? OR "
                "(t.occurred_at = ? AND t.created_at < ?) OR "
                "(t.occurred_at = ? AND t.created_at = ? AND t.id < ?))))"
            )
            params.extend(
                [
                    cursor_data["value"],
                    cursor_data["value"],
                    cursor_data["occurred_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["id"],
                ]
            )
    raw_limit = max(limit * 6, limit + 20)
    params.append(raw_limit + 1)

    rows = conn.execute(
        f"""
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            t.created_at AS _created_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_value,
            t.fiat_rate,
            t.pricing_source_kind,
            t.pricing_quality,
            t.pricing_external_ref,
            t.pricing_provider,
            t.pricing_pair,
            t.pricing_timestamp,
            t.pricing_fetched_at,
            t.pricing_granularity,
            t.pricing_method,
            t.review_status,
            t.taxability_override,
            t.at_regime_override,
            t.at_category_override,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.counterparty, '') AS counterparty,
            COALESCE(t.note, '') AS note,
            t.excluded,
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE {' AND '.join(filters)}
        ORDER BY {order_by}
        LIMIT ?
        """,
        params,
    ).fetchall()
    raw_has_more = len(rows) > raw_limit
    rows_for_page = rows[:raw_limit]
    page, consumed_row, has_more, next_skip_pairs = _transaction_rows_to_ui_page(
        conn,
        rows_for_page,
        limit,
        skip_pairs,
    )
    has_more = has_more or raw_has_more
    next_cursor = (
        _encode_ui_transaction_cursor(
            consumed_row,
            sort=sort,
            order=order,
            filters=cursor_filters,
            skip_pairs=next_skip_pairs,
        )
        if has_more and consumed_row is not None
        else None
    )
    return {
        "txs": page,
        "year": _snapshot_year(rows_for_page),
        "filters": {
            "query": query,
            "limit": limit,
            "direction": direction,
            "asset": asset_filter,
            "wallet": wallet_filter,
            "since": since_filter,
            "until": until_filter,
            "sort": sort,
            "order": order,
        },
        "nextCursor": next_cursor,
        "hasMore": has_more,
    }


def build_transactions_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return _build_transactions_page_snapshot(
        conn,
        args,
        kind="ui.transactions.list",
        default_limit=100,
        maximum_limit=MAX_UI_LIST_LIMIT,
    )


def build_transactions_extremes_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(
        set(raw_args)
        - {
            "limit",
            "direction",
            "asset",
            "wallet",
            "since",
        }
    )
    if unknown:
        raise AppError(
            "ui.transactions.extremes received unsupported filters",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=3, maximum=20)
    base_args = {
        key: value
        for key, value in raw_args.items()
        if key in {"direction", "asset", "wallet", "since"}
    }
    largest = build_transactions_snapshot(
        conn,
        {**base_args, "limit": limit, "sort": "amount", "order": "desc"},
    )
    smallest = build_transactions_snapshot(
        conn,
        {**base_args, "limit": limit, "sort": "amount", "order": "asc"},
    )
    return {
        "largest": largest["txs"],
        "smallest": smallest["txs"],
        "filters": {
            "limit": limit,
            "direction": largest.get("filters", {}).get("direction"),
            "asset": largest.get("filters", {}).get("asset"),
            "wallet": largest.get("filters", {}).get("wallet"),
            "since": largest.get("filters", {}).get("since"),
            "sort": "amount",
            "scope": "all_time_before_limit",
        },
    }


def build_transactions_search_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return _build_transactions_page_snapshot(
        conn,
        args,
        kind="ui.transactions.search",
        default_limit=25,
        maximum_limit=100,
        require_query=True,
    )


def build_transactions_resolve_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {"transaction": None, "query": ""}

    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"query"})
    if unknown:
        raise AppError(
            "ui.transactions.resolve received unsupported filters",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    query = raw_args.get("query")
    if not isinstance(query, str) or not query.strip():
        raise AppError(
            "ui.transactions.resolve query must be a non-empty string",
            code="validation",
            retryable=False,
        )
    query = query.strip()
    id_candidates = [query]
    if query.lower() not in id_candidates:
        id_candidates.append(query.lower())
    external_id_candidates = list(id_candidates)
    if query.upper() not in external_id_candidates:
        external_id_candidates.append(query.upper())
    select_sql = """
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_value,
            t.fiat_rate,
            t.pricing_source_kind,
            t.pricing_quality,
            t.pricing_external_ref,
            t.pricing_provider,
            t.pricing_pair,
            t.pricing_timestamp,
            t.pricing_fetched_at,
            t.pricing_granularity,
            t.pricing_method,
            t.review_status,
            t.taxability_override,
            t.at_regime_override,
            t.at_category_override,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.counterparty, '') AS counterparty,
            COALESCE(t.note, '') AS note,
            t.excluded,
            jq.reason AS quarantine_reason
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
    """
    order_limit_sql = " ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC LIMIT 1"
    id_placeholders = ", ".join("?" for _ in id_candidates)
    row = conn.execute(
        select_sql
        + f"""
        WHERE t.profile_id = ?
          AND t.id IN ({id_placeholders})
        """,
        (context["profile_id"], *id_candidates),
    ).fetchone()
    if row is None:
        external_id_placeholders = ", ".join("?" for _ in external_id_candidates)
        row = conn.execute(
            select_sql
            + f"""
            WHERE t.profile_id = ?
              AND t.external_id IN ({external_id_placeholders})
            """
            + order_limit_sql,
            (context["profile_id"], *external_id_candidates),
        ).fetchone()
    transaction = _transaction_rows_to_ui(conn, [row])[0] if row else None
    return {"transaction": transaction, "query": query}


def _snapshot_year(rows: list[sqlite3.Row]) -> int:
    for row in rows:
        occurred_at = row["occurred_at"] or ""
        if len(occurred_at) >= 4 and occurred_at[:4].isdigit():
            return int(occurred_at[:4])
    return datetime.now(timezone.utc).year


def _capital_gains_available_years(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    primary_only: bool = False,
) -> list[int]:
    reportable_filter = (
        "((je.entry_type = 'disposal' AND COALESCE(je.at_category, '') != 'neu_swap') "
        "OR je.at_kennzahl IS NOT NULL)"
        if primary_only
        else "(je.entry_type IN ('disposal', 'income', 'fee', 'transfer_fee') "
        "OR je.at_kennzahl IS NOT NULL)"
    )
    rows = conn.execute(
        f"""
        SELECT DISTINCT substr(je.occurred_at, 1, 4) AS year
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
          AND COALESCE(t.taxability_override, 1) != 0
          AND {reportable_filter}
          AND je.occurred_at IS NOT NULL
          AND length(je.occurred_at) >= 4
        ORDER BY year DESC
        """,
        (profile_id,),
    ).fetchall()
    years: list[int] = []
    for row in rows:
        year = row["year"] or ""
        if str(year).isdigit():
            years.append(int(year))
    return years


def _capital_gains_transaction_years(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[int]:
    rows = conn.execute(
        """
        SELECT DISTINCT substr(occurred_at, 1, 4) AS year
        FROM transactions
        WHERE profile_id = ?
          AND excluded = 0
          AND occurred_at IS NOT NULL
          AND length(occurred_at) >= 4
        ORDER BY year DESC
        """,
        (profile_id,),
    ).fetchall()
    years: list[int] = []
    for row in rows:
        year = row["year"] or ""
        if str(year).isdigit():
            years.append(int(year))
    return years


def _merge_report_years(*year_lists: list[int]) -> list[int]:
    # Keep these bounds in sync with the desktop `reportYear` URL parser so
    # obviously invalid years fail consistently on both sides of the bridge.
    years = {
        int(year)
        for year_list in year_lists
        for year in year_list
        if 2009 <= int(year) <= 2100
    }
    return sorted(years, reverse=True)


def _austrian_kennzahl_snapshot_rows(
    conn: sqlite3.Connection,
    profile: sqlite3.Row,
    tax_year: int,
) -> list[dict[str, Any]]:
    if str(profile["tax_country"] or "").lower() != "at":
        return []

    summary_rows = report_builders.build_austrian_kennzahl_summary(
        conn,
        profile,
        tax_year,
    )
    return [
        {
            "code": str(row["kennzahl"]),
            "label": row["label"],
            "form": row.get("form", ""),
            "formSection": row.get("form_section", ""),
            "amount": int(row["amount_eur_cents"] or 0) / 100,
            "amountEurCents": int(row["amount_eur_cents"] or 0),
            "rowCount": int(row["row_count"] or 0),
            "source": "daemon",
        }
        for row in summary_rows
    ]


def _capital_gains_neutral_swap_rows(
    conn: sqlite3.Connection,
    profile_id: str,
    tax_year: int,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            je.occurred_at,
            je.quantity,
            je.cost_basis,
            je.proceeds,
            je.gain_loss,
            COALESCE(p.id, '') AS pair_id,
            COALESCE(p.kind, '') AS kind,
            COALESCE(p.policy, '') AS policy,
            COALESCE(p.swap_fee_msat, 0) AS swap_fee_msat,
            COALESCE(p.swap_fee_kind, '') AS swap_fee_kind,
            wout.label AS out_wallet,
            tout.asset AS out_asset,
            -- Split cross-asset swaps cross only `out_amount`; keep outSats
            -- consistent with feeSats (swap_fee_msat) on neu_swap detail rows.
            COALESCE(p.out_amount, tout.amount) AS out_amount,
            win.label AS in_wallet,
            tin.asset AS in_asset,
            tin.amount AS in_amount
        FROM journal_entries je
        LEFT JOIN transaction_pairs p
          ON p.out_transaction_id = je.transaction_id
         AND p.profile_id = je.profile_id
         AND p.deleted_at IS NULL
        LEFT JOIN transactions tout ON tout.id = p.out_transaction_id
        LEFT JOIN transactions tin ON tin.id = p.in_transaction_id
        LEFT JOIN wallets wout ON wout.id = tout.wallet_id
        LEFT JOIN wallets win ON win.id = tin.wallet_id
        WHERE je.profile_id = ?
          AND je.entry_type = 'disposal'
          AND je.at_category = 'neu_swap'
          AND COALESCE(tout.taxability_override, 1) != 0
          AND substr(je.occurred_at, 1, 4) = ?
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile_id, str(tax_year)),
    ).fetchall()
    output = []
    for row in rows:
        quantity_msat = abs(int(row["quantity"] or 0))
        out_amount_msat = abs(int(row["out_amount"] or 0)) or quantity_msat
        in_amount_msat = abs(int(row["in_amount"] or 0))
        fee_msat = int(row["swap_fee_msat"] or 0)
        output.append(
            {
                "date": (row["occurred_at"] or "")[:10],
                "pairId": row["pair_id"],
                "kind": row["kind"],
                "policy": row["policy"],
                "outWallet": row["out_wallet"] or "",
                "outAsset": row["out_asset"] or "",
                "outSats": _ui_sat_amount(out_amount_msat),
                "inWallet": row["in_wallet"] or "",
                "inAsset": row["in_asset"] or "",
                "inSats": _ui_sat_amount(in_amount_msat),
                "feeSats": _ui_sat_amount(fee_msat),
                "feeKind": row["swap_fee_kind"],
                "costEur": float(row["cost_basis"] or 0),
                "proceedsEur": float(row["cost_basis"] or 0),
                "gainEur": 0.0,
                "marketValueEur": float(row["proceeds"] or 0),
                "marketDeltaEur": float(row["gain_loss"] or 0),
            }
        )
    return output


def _normalize_snapshot_tax_year(year: Any) -> int:
    try:
        normalized = int(year)
    except (TypeError, ValueError) as exc:
        raise AppError("tax year must be a four-digit year", code="validation") from exc
    if normalized < 2009 or normalized > 2100:
        raise AppError("tax year must be a plausible four-digit year", code="validation")
    return normalized


def build_capital_gains_snapshot(
    conn: sqlite3.Connection,
    tax_year: int | str | None = None,
) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {
            "jurisdictionCode": "AT",
            "year": datetime.now(timezone.utc).year,
            "availableYears": [datetime.now(timezone.utc).year],
            "method": "fifo",
            "lots": [],
            "kennzahlRows": [],
            "status": {
                "needsJournals": False,
                "quarantines": 0,
            },
        }

    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    if profile is None:
        return {
            "jurisdictionCode": "AT",
            "year": datetime.now(timezone.utc).year,
            "availableYears": [datetime.now(timezone.utc).year],
            "method": "fifo",
            "lots": [],
            "kennzahlRows": [],
            "status": {
                "needsJournals": False,
                "quarantines": 0,
            },
        }

    active_transactions = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
        """,
        (profile["id"],),
    ).fetchone()["count"]
    needs_journals = _journal_freshness(conn, profile)["needs_processing"]
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    available_years = _merge_report_years(
        _capital_gains_available_years(conn, profile["id"]),
        _capital_gains_transaction_years(conn, profile["id"]),
    )
    primary_years = _capital_gains_available_years(
        conn,
        profile["id"],
        primary_only=True,
    )
    if tax_year is not None:
        latest_year = _normalize_snapshot_tax_year(tax_year)
    elif primary_years:
        latest_year = primary_years[0]
    elif available_years:
        latest_year = available_years[0]
    else:
        latest_year = datetime.now(timezone.utc).year
    if latest_year not in available_years:
        available_years = [latest_year, *available_years]
    rows = conn.execute(
        """
        SELECT je.occurred_at, je.quantity, je.cost_basis, je.proceeds, je.gain_loss
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
          AND je.entry_type = 'disposal'
          AND COALESCE(t.taxability_override, 1) != 0
          AND COALESCE(je.at_category, '') != 'neu_swap'
          AND substr(je.occurred_at, 1, 4) = ?
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT 200
        """,
        (profile["id"], str(latest_year)),
    ).fetchall()
    lots = [
        {
            "acquired": "",
            "disposed": (row["occurred_at"] or "")[:10],
            "sats": int(round(abs(float(msat_to_btc(row["quantity"] or 0))) * 100_000_000)),
            "costEur": float(row["cost_basis"] or 0),
            "proceedsEur": float(row["proceeds"] or 0),
            "type": "ST",
        }
        for row in reversed(rows)
    ]
    return {
        "jurisdictionCode": (profile["tax_country"] or "AT").upper(),
        "year": latest_year,
        "availableYears": available_years,
        "method": _profile_policy_method(profile),
        "lots": lots,
        "neutralSwapLots": _capital_gains_neutral_swap_rows(
            conn,
            profile["id"],
            latest_year,
        ),
        "kennzahlRows": _austrian_kennzahl_snapshot_rows(conn, profile, latest_year),
        "status": {
            "needsJournals": needs_journals,
            "quarantines": int(quarantines or 0),
        },
    }


def _journal_recent_row_payload(row: sqlite3.Row) -> dict[str, Any]:
    payload = {
        "date": (row["occurred_at"] or "")[:16].replace("T", " "),
        "type": row["entry_type"],
        "transactionId": row["transaction_id"] if "transaction_id" in row.keys() else "",
        "transactionExternalId": (
            row["transaction_external_id"]
            if "transaction_external_id" in row.keys()
            else ""
        ),
        "wallet": row["wallet"],
        "asset": row["asset"],
        "quantity": float(msat_to_btc(row["quantity"] or 0)),
        "fiatValueEur": float(row["fiat_value"] or 0),
        "gainLossEur": float(row["gain_loss"] or 0),
    }
    if "at_category" in row.keys():
        payload["atCategory"] = row["at_category"]
    pair = _journal_pair_payload(row)
    if pair:
        payload["pair"] = pair
    return payload


def _journal_pair_payload(row: sqlite3.Row) -> dict[str, Any] | None:
    if "pair_id" not in row.keys() or not row["pair_id"]:
        return None
    out_asset = row["pair_out_asset"]
    in_asset = row["pair_in_asset"]
    swap_fee_msat = int(row["pair_swap_fee_msat"] or 0)
    return {
        "pairId": row["pair_id"],
        "pairType": "transfer" if out_asset == in_asset else "swap",
        "kind": row["pair_kind"],
        "policy": row["pair_policy"],
        "swapFeeMsat": swap_fee_msat,
        "swapFee": float(msat_to_btc(swap_fee_msat)),
        "out": {
            "transactionId": row["pair_out_transaction_id"],
            "externalId": row["pair_out_external_id"] or "",
            "wallet": row["pair_out_wallet"],
            "asset": out_asset,
            "amountMsat": int(row["pair_out_amount"] or 0),
            "amount": float(msat_to_btc(row["pair_out_amount"] or 0)),
        },
        "in": {
            "transactionId": row["pair_in_transaction_id"],
            "externalId": row["pair_in_external_id"] or "",
            "wallet": row["pair_in_wallet"],
            "asset": in_asset,
            "amountMsat": int(row["pair_in_amount"] or 0),
            "amount": float(msat_to_btc(row["pair_in_amount"] or 0)),
        },
    }


_JOURNAL_PAIR_JOIN_SQL = """
            LEFT JOIN transaction_pairs p_out
              ON p_out.profile_id = je.profile_id
             AND p_out.deleted_at IS NULL
             AND p_out.out_transaction_id = je.transaction_id
            LEFT JOIN transaction_pairs p_in
              ON p_in.profile_id = je.profile_id
             AND p_in.deleted_at IS NULL
             AND p_in.in_transaction_id = je.transaction_id
"""


def build_journals_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context = current_context_snapshot(conn)
    if not context["workspace_id"] or not context["profile_id"]:
        return {
            "status": {
                "workspace": None,
                "profile": None,
                "transactionCount": 0,
                "journalEntryCount": 0,
                "needsJournals": False,
                "quarantines": 0,
                "lastProcessedAt": None,
            },
            "entryTypes": [],
            "recent": [],
            "recentByType": {},
        }

    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (context["profile_id"],),
    ).fetchone()
    if profile is None:
        return {
            "status": {
                "workspace": None,
                "profile": None,
                "transactionCount": 0,
                "journalEntryCount": 0,
                "needsJournals": False,
                "quarantines": 0,
                "lastProcessedAt": None,
            },
            "entryTypes": [],
            "recent": [],
            "recentByType": {},
        }

    freshness = _journal_freshness(conn, profile)
    entry_rows = conn.execute(
        f"""
        SELECT
            {_JOURNAL_DISPLAY_ENTRY_TYPE_SQL} AS entry_type,
            COUNT(*) AS count,
            SUM({_JOURNAL_DISPLAY_GAIN_LOSS_SQL}) AS gain_loss
        FROM journal_entries je
        WHERE profile_id = ?
        GROUP BY {_JOURNAL_DISPLAY_ENTRY_TYPE_SQL}
        ORDER BY count DESC, entry_type ASC
        """,
        (profile["id"],),
    ).fetchall()
    recent_rows = conn.execute(
        f"""
        WITH normalized AS (
            SELECT
                je.occurred_at,
                je.created_at,
                je.id,
                je.transaction_id,
                COALESCE(t.external_id, '') AS transaction_external_id,
                {_JOURNAL_DISPLAY_ENTRY_TYPE_SQL} AS entry_type,
                je.asset,
                je.quantity,
                je.fiat_value,
                {_JOURNAL_DISPLAY_GAIN_LOSS_SQL} AS gain_loss,
                je.at_category,
                w.label AS wallet,
                COALESCE(p_out.id, p_in.id) AS pair_id,
                COALESCE(p_out.kind, p_in.kind) AS pair_kind,
                COALESCE(p_out.policy, p_in.policy) AS pair_policy,
                COALESCE(p_out.swap_fee_msat, p_in.swap_fee_msat, 0) AS pair_swap_fee_msat,
                COALESCE(p_out.out_transaction_id, p_in.out_transaction_id) AS pair_out_transaction_id,
                tout.external_id AS pair_out_external_id,
                wout.label AS pair_out_wallet,
                tout.asset AS pair_out_asset,
                COALESCE(p_out.out_amount, p_in.out_amount, tout.amount) AS pair_out_amount,
                COALESCE(p_out.in_transaction_id, p_in.in_transaction_id) AS pair_in_transaction_id,
                tin.external_id AS pair_in_external_id,
                win.label AS pair_in_wallet,
                tin.asset AS pair_in_asset,
                tin.amount AS pair_in_amount
            FROM journal_entries je
            JOIN wallets w ON w.id = je.wallet_id
            LEFT JOIN transactions t ON t.id = je.transaction_id
            {_JOURNAL_PAIR_JOIN_SQL}
            LEFT JOIN transactions tout ON tout.id = COALESCE(p_out.out_transaction_id, p_in.out_transaction_id)
            LEFT JOIN transactions tin ON tin.id = COALESCE(p_out.in_transaction_id, p_in.in_transaction_id)
            LEFT JOIN wallets wout ON wout.id = tout.wallet_id
            LEFT JOIN wallets win ON win.id = tin.wallet_id
            WHERE je.profile_id = ?
        )
        SELECT *
        FROM normalized
        ORDER BY occurred_at DESC, created_at DESC, id DESC
        LIMIT 12
        """,
        (profile["id"],),
    ).fetchall()
    recent_by_type: dict[str, list[dict[str, Any]]] = {}
    for entry_row in entry_rows:
        typed_recent_rows = conn.execute(
            f"""
            WITH normalized AS (
                SELECT
                    je.occurred_at,
                    je.created_at,
                    je.id,
                    je.transaction_id,
                    COALESCE(t.external_id, '') AS transaction_external_id,
                    {_JOURNAL_DISPLAY_ENTRY_TYPE_SQL} AS entry_type,
                    je.asset,
                    je.quantity,
                    je.fiat_value,
                    {_JOURNAL_DISPLAY_GAIN_LOSS_SQL} AS gain_loss,
                    je.at_category,
                    w.label AS wallet,
                    COALESCE(p_out.id, p_in.id) AS pair_id,
                    COALESCE(p_out.kind, p_in.kind) AS pair_kind,
                    COALESCE(p_out.policy, p_in.policy) AS pair_policy,
                    COALESCE(p_out.swap_fee_msat, p_in.swap_fee_msat, 0) AS pair_swap_fee_msat,
                    COALESCE(p_out.out_transaction_id, p_in.out_transaction_id) AS pair_out_transaction_id,
                    tout.external_id AS pair_out_external_id,
                    wout.label AS pair_out_wallet,
                    tout.asset AS pair_out_asset,
                    COALESCE(p_out.out_amount, p_in.out_amount, tout.amount) AS pair_out_amount,
                    COALESCE(p_out.in_transaction_id, p_in.in_transaction_id) AS pair_in_transaction_id,
                    tin.external_id AS pair_in_external_id,
                    win.label AS pair_in_wallet,
                    tin.asset AS pair_in_asset,
                    tin.amount AS pair_in_amount
                FROM journal_entries je
                JOIN wallets w ON w.id = je.wallet_id
                LEFT JOIN transactions t ON t.id = je.transaction_id
                {_JOURNAL_PAIR_JOIN_SQL}
                LEFT JOIN transactions tout ON tout.id = COALESCE(p_out.out_transaction_id, p_in.out_transaction_id)
                LEFT JOIN transactions tin ON tin.id = COALESCE(p_out.in_transaction_id, p_in.in_transaction_id)
                LEFT JOIN wallets wout ON wout.id = tout.wallet_id
                LEFT JOIN wallets win ON win.id = tin.wallet_id
                WHERE je.profile_id = ?
            )
            SELECT *
            FROM normalized
            WHERE entry_type = ?
            ORDER BY occurred_at DESC, created_at DESC, id DESC
            LIMIT 12
            """,
            (profile["id"], entry_row["entry_type"]),
        ).fetchall()
        recent_by_type[entry_row["entry_type"]] = [
            _journal_recent_row_payload(row) for row in typed_recent_rows
        ]
    return {
        "status": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "transactionCount": freshness["active_transaction_count"],
            "journalEntryCount": freshness["journal_entry_count"],
            "needsJournals": freshness["needs_processing"],
            "quarantines": freshness["quarantine_count"],
            "lastProcessedAt": profile["last_processed_at"],
            "freshnessStatus": freshness["status"],
            "freshnessReason": freshness["reason"],
            "journalInputVersion": freshness["journal_input_version"],
            "lastProcessedInputVersion": freshness["last_processed_input_version"],
        },
        "entryTypes": [
            {
                "type": row["entry_type"],
                "count": int(row["count"] or 0),
                "gainLossEur": float(row["gain_loss"] or 0),
            }
            for row in entry_rows
        ],
        "recent": [_journal_recent_row_payload(row) for row in recent_rows],
        "recentByType": recent_by_type,
    }


def build_journal_events_list_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"limit", "transaction"})
    if unknown:
        raise AppError(
            "ui.journals.events.list received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=100, maximum=MAX_UI_LIST_LIMIT)
    transaction_ref = raw_args.get("transaction")
    if transaction_ref is not None:
        if not isinstance(transaction_ref, str):
            raise AppError(
                "ui.journals.events.list transaction must be a string",
                code="validation",
                retryable=False,
            )
        transaction_ref = transaction_ref.strip() or None
    context, profile = _active_context_and_profile(conn)
    empty_summary = {
        "workspace": None,
        "profile": None,
        "count": 0,
        "reportableCount": 0,
        "needsJournals": False,
        "lastProcessedAt": None,
        "freshnessStatus": "no_profile",
        "freshnessReason": "no active profile",
        "entryTypes": [],
        "limit": limit,
    }
    if profile is None:
        return {"summary": empty_summary, "events": []}

    freshness = _journal_freshness(conn, profile)
    where_sql = "WHERE je.profile_id = ?"
    params: list[Any] = [profile["id"]]
    if transaction_ref:
        where_sql += " AND (je.transaction_id = ? OR t.external_id = ?)"
        params.extend([transaction_ref, transaction_ref])
    summary_rows = conn.execute(
        f"""
        SELECT
            {_JOURNAL_DISPLAY_ENTRY_TYPE_SQL} AS entry_type,
            COUNT(*) AS count,
            SUM({_JOURNAL_DISPLAY_GAIN_LOSS_SQL}) AS gain_loss
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {where_sql}
        GROUP BY {_JOURNAL_DISPLAY_ENTRY_TYPE_SQL}
        ORDER BY count DESC, entry_type ASC
        """,
        params,
    ).fetchall()
    total = sum(int(row["count"] or 0) for row in summary_rows)
    reportable_count = conn.execute(
        f"""
        SELECT COUNT(*) AS count
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {where_sql}
          AND COALESCE(t.taxability_override, 1) != 0
          AND (
            (je.entry_type = 'disposal' AND COALESCE(je.at_category, '') != 'neu_swap')
            OR je.entry_type IN ('income', 'fee', 'transfer_fee')
            OR je.at_kennzahl IS NOT NULL
          )
        """,
        params,
    ).fetchone()["count"]
    rows = conn.execute(
        f"""
        SELECT
            je.id,
            je.transaction_id,
            je.occurred_at,
            je.created_at,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            je.unit_cost,
            je.cost_basis,
            je.proceeds,
            je.gain_loss,
            je.pricing_source_kind,
            je.pricing_quality,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            t.external_id AS transaction_external_id,
            t.direction AS transaction_direction,
            COALESCE(p_out.id, p_in.id) AS pair_id,
            COALESCE(p_out.kind, p_in.kind) AS pair_kind,
            COALESCE(p_out.policy, p_in.policy) AS pair_policy,
            COALESCE(p_out.swap_fee_msat, p_in.swap_fee_msat, 0) AS pair_swap_fee_msat,
            COALESCE(p_out.out_transaction_id, p_in.out_transaction_id) AS pair_out_transaction_id,
            tout.external_id AS pair_out_external_id,
            wout.label AS pair_out_wallet,
            tout.asset AS pair_out_asset,
            COALESCE(p_out.out_amount, p_in.out_amount, tout.amount) AS pair_out_amount,
            COALESCE(p_out.in_transaction_id, p_in.in_transaction_id) AS pair_in_transaction_id,
            tin.external_id AS pair_in_external_id,
            win.label AS pair_in_wallet,
            tin.asset AS pair_in_asset,
            tin.amount AS pair_in_amount
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {_JOURNAL_PAIR_JOIN_SQL}
        LEFT JOIN transactions tout ON tout.id = COALESCE(p_out.out_transaction_id, p_in.out_transaction_id)
        LEFT JOIN transactions tin ON tin.id = COALESCE(p_out.in_transaction_id, p_in.in_transaction_id)
        LEFT JOIN wallets wout ON wout.id = tout.wallet_id
        LEFT JOIN wallets win ON win.id = tin.wallet_id
        {where_sql}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT ?
        """,
        [*params, limit],
    ).fetchall()
    return {
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": total,
            "reportableCount": int(reportable_count or 0),
            "needsJournals": freshness["needs_processing"],
            "lastProcessedAt": profile["last_processed_at"],
            "freshnessStatus": freshness["status"],
            "freshnessReason": freshness["reason"],
            "entryTypes": [
                {
                    "type": row["entry_type"],
                    "count": int(row["count"] or 0),
                    "gainLossEur": float(row["gain_loss"] or 0),
                }
                for row in summary_rows
            ],
            "limit": limit,
        },
        "events": [
            {
                "id": row["id"],
                "transactionId": row["transaction_id"],
                "transactionExternalId": row["transaction_external_id"] or "",
                "transactionDirection": row["transaction_direction"] or "",
                "occurredAt": row["occurred_at"],
                "createdAt": row["created_at"],
                "entryType": (
                    "neutral_swap"
                    if row["at_category"] == "neu_swap"
                    else row["entry_type"]
                ),
                "wallet": row["wallet"],
                "account": row["account"],
                "accountLabel": row["account_label"],
                "asset": row["asset"],
                "quantity": float(msat_to_btc(row["quantity"] or 0)),
                "quantityMsat": int(row["quantity"] or 0),
                "fiatValueEur": float(row["fiat_value"] or 0),
                "unitCostEur": float(row["unit_cost"] or 0),
                "costBasisEur": (
                    float(row["cost_basis"]) if row["cost_basis"] is not None else None
                ),
                "proceedsEur": (
                    float(row["cost_basis"])
                    if row["at_category"] == "neu_swap"
                    and row["cost_basis"] is not None
                    else float(row["proceeds"])
                    if row["proceeds"] is not None
                    else None
                ),
                "gainLossEur": (
                    0.0
                    if row["at_category"] == "neu_swap"
                    else float(row["gain_loss"])
                    if row["gain_loss"] is not None
                    else None
                ),
                "marketValueEur": (
                    float(row["proceeds"])
                    if row["at_category"] == "neu_swap"
                    and row["proceeds"] is not None
                    else None
                ),
                "marketDeltaEur": (
                    float(row["gain_loss"])
                    if row["at_category"] == "neu_swap"
                    and row["gain_loss"] is not None
                    else None
                ),
                "pricingSourceKind": row["pricing_source_kind"] or "",
                "pricingQuality": row["pricing_quality"] or "",
                "description": row["description"],
                "atCategory": row["at_category"],
                "atKennzahl": row["at_kennzahl"],
                "pair": _journal_pair_payload(row),
            }
            for row in rows
        ],
    }


def build_wallets_list_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None = None,
) -> dict[str, Any]:
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "wallets": [],
            "summary": {
                "workspace": None,
                "profile": None,
                "count": 0,
                "transaction_count": 0,
                "needs_journals": False,
            },
        }

    freshness = _journal_freshness(conn, profile)
    rows = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            w.config_json,
            w.created_at,
            a.code AS account_code,
            a.label AS account_label,
            COUNT(t.id) AS tx_count,
            MAX(t.occurred_at) AS last_tx_at
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        LEFT JOIN transactions t ON t.wallet_id = w.id AND t.excluded = 0
        WHERE w.profile_id = ?
        GROUP BY w.id
        ORDER BY w.label ASC
        """,
        (profile["id"],),
    ).fetchall()
    runtime_backends = (
        runtime_config.get("backends", {})
        if isinstance(runtime_config, dict)
        else {}
    )
    default_backend = (
        runtime_config.get("default_backend")
        if isinstance(runtime_config, dict)
        else None
    )
    wallets = []
    for row in rows:
        config = _json_config(row["config_json"])
        samourai_metadata = samourai_metadata_from_wallet_config(config)
        backend_summary = _wallet_backend_summary(row["kind"], config, default_backend)
        backend_name = backend_summary["name"]
        last_synced_at = _string_or_empty(config.get("last_synced_at"))
        backend = (
            runtime_backends.get(str(backend_name))
            if isinstance(runtime_backends, dict) and backend_name
            else None
        )
        tx_count = int(row["tx_count"] or 0)
        # Provenance routes are non-secret routing metadata (backend name,
        # store id, payment method id). Exposing them lets the desktop
        # connection detail screen show and remove routes without needing
        # a separate single-wallet daemon endpoint.
        provenance_routes = wallet_btcpay_provenance_config(config)
        wallets.append(
            {
                "id": row["id"],
                "label": row["label"],
                "kind": row["kind"],
                "account": {
                    "code": row["account_code"] or "",
                    "label": row["account_label"] or "",
                },
                "backend": {
                    "name": str(backend_name) if backend_name else "",
                    "source": backend_summary["source"],
                    "kind": str(backend.get("kind") or "") if isinstance(backend, dict) else "",
                },
                "chain": str(config.get("chain") or ""),
                "network": str(config.get("network") or ""),
                "sync_mode": backend_summary["sync_mode"],
                "sync_source": str(config.get("sync_source") or config.get("source_format") or ""),
                "transaction_count": tx_count,
                "last_transaction_at": row["last_tx_at"],
                "last_synced_at": last_synced_at or None,
                "sync_status": "has_transactions" if tx_count else "empty",
                "deprecated": wallet_is_deprecated(config),
                "journals_stale": freshness["needs_processing"] and tx_count > 0,
                "btcpay_provenance": provenance_routes,
                "samourai": samourai_metadata,
                # Watched script types for an auto-detected xpub wallet (empty
                # for explicit-descriptor / non-xpub wallets); lets the detail
                # screen show and edit the set without revealing the key itself.
                "script_types": [
                    str(value)
                    for value in (config.get("script_types") or [])
                    if isinstance(value, str)
                ],
                "created_at": row["created_at"],
            }
        )

    return {
        "wallets": wallets,
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": len(wallets),
            "transaction_count": sum(wallet["transaction_count"] for wallet in wallets),
            "needs_journals": bool(freshness["needs_processing"]),
        },
    }


def _wallet_ref_arg(args: Any) -> str:
    if not isinstance(args, dict):
        raise AppError(
            "ui.wallets.utxos args must be an object",
            code="validation",
            retryable=False,
        )
    wallet = args.get("wallet") or args.get("connection")
    if not isinstance(wallet, str) or not wallet.strip():
        raise AppError(
            "ui.wallets.utxos wallet must be a non-empty string",
            code="validation",
            retryable=False,
        )
    return wallet.strip()


def _resolve_wallet_for_utxos(
    conn: sqlite3.Connection,
    profile_id: str,
    wallet_ref: str,
) -> sqlite3.Row:
    row = conn.execute(
        """
        SELECT
            w.id,
            w.label,
            w.kind,
            w.config_json,
            w.created_at,
            a.code AS account_code,
            a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ?
          AND (w.id = ? OR w.label = ?)
        ORDER BY CASE WHEN w.id = ? THEN 0 ELSE 1 END, w.label ASC
        LIMIT 1
        """,
        (profile_id, wallet_ref, wallet_ref, wallet_ref),
    ).fetchone()
    if row is None:
        raise AppError(
            f"Wallet '{wallet_ref}' was not found in the active profile",
            code="not_found",
            retryable=False,
        )
    return row


def _wallet_utxo_support(
    wallet: sqlite3.Row,
    config: dict[str, Any],
    backend_summary: dict[str, str],
    backend: Any,
) -> dict[str, Any]:
    sync_mode = backend_summary["sync_mode"]
    has_descriptor = has_descriptor_sync_material(config)
    has_addresses = bool(config.get("addresses"))
    if _string_or_empty(config.get("source_format")) == "wasabi_bundle":
        return {
            "supported": True,
            "status": "supported",
            "reason": "wasabi_import",
            "message": "",
        }
    if sync_mode not in {"backend_descriptor", "backend_addresses"}:
        return {
            "supported": False,
            "status": "unsupported_source",
            "reason": "not_chain_backed",
            "message": "This source is not a chain-backed watch-only wallet.",
        }
    if not isinstance(backend, dict):
        return {
            "supported": False,
            "status": "unsupported_source",
            "reason": "backend_missing",
            "message": "This wallet needs a configured Esplora, Electrum, or Bitcoin Core backend before UTXO inventory can refresh.",
        }
    backend_kind = normalize_backend_kind(backend.get("kind"))
    if backend_kind not in {"esplora", "electrum", "bitcoinrpc"}:
        return {
            "supported": False,
            "status": "unsupported_source",
            "reason": "backend_kind",
            "message": f"UTXO inventory is not implemented for {backend_kind or 'this backend'} sources yet.",
        }
    chain = str(config.get("chain") or "bitcoin").strip().lower() or "bitcoin"
    if has_descriptor and backend_kind == "bitcoinrpc":
        return {
            "supported": False,
            "status": "unsupported_source",
            "reason": "bitcoinrpc_descriptor",
            "message": "Bitcoin Core UTXO inventory is available for address-backed wallets only.",
        }
    if chain == "liquid":
        if not has_descriptor:
            return {
                "supported": False,
                "status": "liquid_unblind_blocked",
                "reason": "liquid_descriptor_required",
                "message": "Liquid UTXO inventory requires descriptor-backed outputs so Kassiber can unblind them locally.",
            }
        try:
            descriptor_plan = load_wallet_descriptor_plan_from_config(config)
        except AppError as exc:
            return {
                "supported": False,
                "status": "liquid_unblind_blocked",
                "reason": "descriptor_unavailable",
                "message": str(exc),
            }
        if not liquid_plan_can_unblind(descriptor_plan):
            return {
                "supported": False,
                "status": "liquid_unblind_blocked",
                "reason": "missing_blinding_keys",
                "message": "Liquid UTXO inventory needs private blinding keys before Kassiber can account for outputs.",
            }
    if not has_descriptor and not has_addresses:
        return {
            "supported": False,
            "status": "unsupported_source",
            "reason": "no_watch_targets",
            "message": "This wallet has no descriptor, xpub, or addresses to scan.",
        }
    return {
        "supported": True,
        "status": "supported",
        "reason": "",
        "message": "",
    }


def build_wallet_utxos_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None,
    args: Any,
) -> dict[str, Any]:
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "wallet": None,
            "utxos": [],
            "totals": [],
            "support": {
                "supported": False,
                "status": "unsupported_source",
                "reason": "no_active_profile",
                "message": "No active profile is selected.",
            },
            "freshness": {
                "status": "no_profile",
                "last_seen_at": None,
                "last_synced_at": None,
                "stale": False,
            },
        }
    wallet_ref = _wallet_ref_arg(args)
    wallet = _resolve_wallet_for_utxos(conn, profile["id"], wallet_ref)
    config = _json_config(wallet["config_json"])
    runtime_backends = (
        runtime_config.get("backends", {})
        if isinstance(runtime_config, dict)
        else {}
    )
    default_backend = (
        runtime_config.get("default_backend")
        if isinstance(runtime_config, dict)
        else None
    )
    backend_summary = _wallet_backend_summary(wallet["kind"], config, default_backend)
    backend_name = backend_summary["name"]
    backend = (
        runtime_backends.get(str(backend_name))
        if isinstance(runtime_backends, dict) and backend_name
        else None
    )
    support = _wallet_utxo_support(wallet, config, backend_summary, backend)
    if support["supported"]:
        source_filter = _wallet_utxo_source_filter(config, backend_summary, backend)
        rows = core_output_inventory.list_wallet_output_inventory(
            conn,
            wallet["id"],
            limit=core_output_inventory.DEFAULT_WALLET_OUTPUT_INVENTORY_LIMIT,
            **source_filter,
        )
        inventory_summary = core_output_inventory.wallet_output_inventory_summary(
            conn,
            wallet["id"],
            **source_filter,
        )
        totals = core_output_inventory.wallet_output_inventory_totals(
            conn,
            wallet["id"],
            **source_filter,
        )
        last_seen_at = inventory_summary["last_seen_at"]
    else:
        rows = []
        totals = []
        last_seen_at = None
    last_synced_at = _string_or_empty(config.get("last_synced_at")) or None
    if not support["supported"]:
        freshness_status = support["status"]
    elif last_seen_at:
        freshness_status = "current"
    elif last_synced_at:
        freshness_status = "stale"
    else:
        freshness_status = "never_refreshed"
    return {
        "wallet": {
            "id": wallet["id"],
            "label": wallet["label"],
            "kind": wallet["kind"],
            "account": {
                "code": wallet["account_code"] or "",
                "label": wallet["account_label"] or "",
            },
            "backend": {
                "name": str(backend_name) if backend_name else "",
                "source": backend_summary["source"],
                "kind": str(backend.get("kind") or "") if isinstance(backend, dict) else "",
            },
            "chain": str(config.get("chain") or ""),
            "network": str(config.get("network") or ""),
            "sync_mode": backend_summary["sync_mode"],
        },
        "utxos": rows,
        "totals": totals,
        "support": support,
        "freshness": {
            "status": freshness_status,
            "last_seen_at": last_seen_at,
            "last_synced_at": last_synced_at,
            "stale": support["supported"] and freshness_status != "current",
            "active_count": inventory_summary["active_count"] if support["supported"] else 0,
        },
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": inventory_summary["active_count"] if support["supported"] else 0,
            "returned_count": len(rows),
            "truncated": (
                support["supported"]
                and inventory_summary["active_count"] > len(rows)
            ),
            "row_limit": (
                core_output_inventory.DEFAULT_WALLET_OUTPUT_INVENTORY_LIMIT
                if support["supported"]
                else None
            ),
        },
    }


def _wallet_utxo_row_for_ai(row: dict[str, Any]) -> dict[str, Any]:
    redacted = dict(row)
    redacted.pop("address", None)
    redacted.pop("address_label", None)
    redacted.pop("branch_index", None)
    redacted.pop("address_index", None)
    redacted.pop("anon_history", None)
    return redacted


def build_wallet_utxos_snapshot_for_ai(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None,
    args: Any,
) -> dict[str, Any]:
    payload = build_wallet_utxos_snapshot(conn, runtime_config, args)
    return {
        **payload,
        "utxos": [
            _wallet_utxo_row_for_ai(row)
            for row in payload.get("utxos", [])
            if isinstance(row, dict)
        ],
    }


def _identify_inputs(args: Any) -> dict[str, Any]:
    empty = {
        "addresses": [],
        "txids": [],
        "candidates": [],
        "text": None,
        "csv_text": None,
        "scan_to_index": None,
    }
    if args is None:
        return empty
    if not isinstance(args, dict):
        raise AppError(
            "ui.wallets.identify args must be an object",
            code="validation",
            retryable=False,
        )

    def _as_list(value: Any) -> list[str]:
        if value is None:
            return []
        if isinstance(value, str):
            return [value]
        if isinstance(value, list):
            return [str(item) for item in value if isinstance(item, (str, int))]
        raise AppError(
            "ui.wallets.identify list fields must be arrays of strings",
            code="validation",
            retryable=False,
        )

    text = args.get("text")
    csv_text = args.get("csv_text")
    scan_to_index = args.get("scan_to_index")
    return {
        "addresses": _as_list(args.get("addresses")),
        "txids": _as_list(args.get("txids")),
        "candidates": _as_list(args.get("candidates")),
        "text": text if isinstance(text, str) else None,
        "csv_text": csv_text if isinstance(csv_text, str) else None,
        "scan_to_index": int(scan_to_index) if isinstance(scan_to_index, int) else None,
    }


def _empty_identify_payload() -> dict[str, Any]:
    return {
        "results": [],
        "summary": {
            "total": 0,
            "owned": 0,
            "external": 0,
            "unknown": 0,
            "invalid": 0,
            "wallets_scanned": 0,
            "scan_to_index": 0,
            "verified_on_chain": False,
        },
        "warnings": [],
        "context": {"workspace": None, "profile": None},
    }


def build_wallet_identify_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None,
    args: Any,
) -> dict[str, Any]:
    """Reconcile pasted addresses / txids against the active profile's wallets.

    Read-only and cache-only: matches against the watch-only output inventory,
    imported txids and offline descriptor derivation. It never contacts the
    network — on-chain verification is the separate ``ui.wallets.identify_onchain``
    action, so this read surface stays safe to call without consent.
    """
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return _empty_identify_payload()
    inputs = _identify_inputs(args)
    scan_to_index = inputs["scan_to_index"]
    if scan_to_index is None:
        scan_to_index = core_ownership.DEFAULT_SCAN_TO_INDEX
    report = core_ownership.identify(
        conn,
        profile["id"],
        addresses=inputs["addresses"],
        txids=inputs["txids"],
        candidates=inputs["candidates"],
        file_text=inputs["text"],
        csv_text=inputs["csv_text"],
        scan_to_index=scan_to_index,
        verify_fetcher=None,
    )
    report["context"] = {
        "workspace": context["workspace_label"] or None,
        "profile": context["profile_label"] or None,
    }
    return report


def build_wallet_identify_onchain_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None,
    args: Any,
) -> dict[str, Any]:
    """Reconcile with the opt-in on-chain tier: txids not in local history are
    fetched through an Esplora/Electrum backend for a per-leg verdict.

    This contacts the network, so it is a mutating daemon kind (the desktop
    "Verify on chain" action), never a read tool and never exposed to the AI.
    """
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return _empty_identify_payload()
    inputs = _identify_inputs(args)
    scan_to_index = inputs["scan_to_index"]
    if scan_to_index is None:
        scan_to_index = core_ownership.DEFAULT_SCAN_TO_INDEX
    backend_name = args.get("backend") if isinstance(args, dict) else None
    backend = core_sync_backends.resolve_verify_backend(runtime_config, backend_name)
    with core_sync_backends.verify_session(backend) as fetcher:
        report = core_ownership.identify(
            conn,
            profile["id"],
            addresses=inputs["addresses"],
            txids=inputs["txids"],
            candidates=inputs["candidates"],
            file_text=inputs["text"],
            csv_text=inputs["csv_text"],
            scan_to_index=scan_to_index,
            verify_fetcher=fetcher,
        )
    report["context"] = {
        "workspace": context["workspace_label"] or None,
        "profile": context["profile_label"] or None,
    }
    return report


def build_wallet_identify_snapshot_for_ai(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object] | None,
    args: Any,
) -> dict[str, Any]:
    # Defense in depth: the AI tool schema never exposes csv_text, but strip it
    # here too so the model can never drive a bulk file/CSV harvest.
    if isinstance(args, dict) and "csv_text" in args:
        args = {key: value for key, value in args.items() if key != "csv_text"}
    payload = build_wallet_identify_snapshot(conn, runtime_config, args)
    return {
        **payload,
        "results": [
            core_ownership.redact_result_for_ai(row)
            for row in payload.get("results", [])
            if isinstance(row, dict)
        ],
    }


def build_backends_list_snapshot(
    conn: sqlite3.Connection,
    runtime_config: dict[str, object],
) -> dict[str, Any]:
    runtime_backends = runtime_config.get("backends", {})
    default_backend = str(runtime_config.get("default_backend") or "")
    allowed_backend_fields = {
        "name",
        "kind",
        "chain",
        "network",
        "batch_size",
        "timeout",
        "insecure",
        "has_auth_header",
        "has_token",
        "has_cookiefile",
        "has_username",
        "has_password",
    }
    context, profile = _active_context_and_profile(conn)
    referenced_names: set[str] = set()
    if profile is not None:
        wallet_rows = conn.execute(
            """
            SELECT kind, config_json
            FROM wallets
            WHERE profile_id = ?
            ORDER BY label ASC
            """,
            (profile["id"],),
        ).fetchall()
        for row in wallet_rows:
            backend = _wallet_backend_summary(
                row["kind"],
                _json_config(row["config_json"]),
                default_backend,
            )
            if backend["name"]:
                referenced_names.add(backend["name"])

    rows = []
    if isinstance(runtime_backends, dict):
        for name, backend in sorted(runtime_backends.items()):
            if str(name) not in referenced_names:
                continue
            if not isinstance(backend, dict):
                continue
            safe = redact_backend_for_output(
                {
                    "name": name,
                    "kind": backend.get("kind", ""),
                    "chain": backend.get("chain", ""),
                    "network": backend.get("network", ""),
                    "url": backend.get("url", ""),
                    "batch_size": backend.get("batch_size", ""),
                    "source": backend.get("source", ""),
                    "auth_header": backend.get("auth_header", ""),
                    "token": backend.get("token", ""),
                    "cookiefile": backend.get("cookiefile", ""),
                    "username": backend.get("username", ""),
                    "password": backend.get("password", ""),
                    "config_json": backend.get("config_json", ""),
                }
            )
            safe = {
                key: value
                for key, value in safe.items()
                if key in allowed_backend_fields
            }
            safe["has_url"] = bool(_string_or_empty(backend.get("url")))
            safe["is_default"] = str(name) == default_backend
            rows.append(safe)
    return {
        "backends": rows,
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": len(rows),
            "default_backend": default_backend if default_backend in referenced_names else None,
            "scope": "active_profile",
        },
    }


def build_journals_quarantine_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"limit"})
    if unknown:
        raise AppError(
            "ui.journals.quarantine received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=20, maximum=MAX_UI_PREVIEW_LIMIT)
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "summary": {
                "workspace": None,
                "profile": None,
                "count": 0,
                "by_reason": [],
                "limit": limit,
            },
            "items": [],
        }

    reason_rows = conn.execute(
        """
        SELECT reason, COUNT(*) AS count
        FROM journal_quarantines
        WHERE profile_id = ?
        GROUP BY reason
        ORDER BY count DESC, reason ASC
        """,
        (profile["id"],),
    ).fetchall()
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            q.reason,
            q.detail_json,
            q.created_at,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            w.label AS wallet
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY q.created_at DESC, t.occurred_at DESC, q.transaction_id DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    total = sum(int(row["count"] or 0) for row in reason_rows)
    return {
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "count": total,
            "by_reason": [
                {"reason": row["reason"], "count": int(row["count"] or 0)}
                for row in reason_rows
            ],
            "limit": limit,
        },
        "items": [
            {
                "transaction_id": row["transaction_id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "confirmed_at": row["confirmed_at"],
                "wallet": row["wallet"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"] or 0)),
                "amount_msat": int(row["amount"] or 0),
                "fee": float(msat_to_btc(row["fee"] or 0)),
                "fee_msat": int(row["fee"] or 0),
                "reason": row["reason"],
                "detail": _json_config(row["detail_json"]),
                "created_at": row["created_at"],
            }
            for row in rows
        ],
    }


def build_journals_transfers_list_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"limit"})
    if unknown:
        raise AppError(
            "ui.journals.transfers.list received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=20, maximum=MAX_UI_PREVIEW_LIMIT)
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "summary": {
                "workspace": None,
                "profile": None,
                "manual_pairs": 0,
                "same_asset_pairs": 0,
                "cross_asset_pairs": 0,
                "journal_transfer_entries": 0,
                "limit": limit,
            },
            "pairs": [],
        }

    summary = conn.execute(
        """
        SELECT
            COUNT(*) AS manual_pairs,
            SUM(CASE WHEN tout.asset = tin.asset THEN 1 ELSE 0 END) AS same_asset_pairs,
            SUM(CASE WHEN tout.asset <> tin.asset THEN 1 ELSE 0 END) AS cross_asset_pairs
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        WHERE p.profile_id = ? AND p.deleted_at IS NULL
        """,
        (profile["id"],),
    ).fetchone()
    journal_transfer_entries = conn.execute(
        """
        SELECT COUNT(*) AS count
        FROM journal_entries
        WHERE profile_id = ? AND entry_type IN ('transfer_out', 'transfer_in', 'transfer_fee')
        """,
        (profile["id"],),
    ).fetchone()["count"]
    rows = conn.execute(
        """
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.created_at,
            p.out_transaction_id,
            p.in_transaction_id,
            tout.external_id AS out_external_id,
            tout.occurred_at AS out_occurred_at,
            tout.asset AS out_asset,
            -- Split cross-asset pairs cross only `out_amount`; mirror the CLI
            -- transfers-list (swap_fee_msat is measured against this portion).
            COALESCE(p.out_amount, tout.amount) AS out_amount,
            wout.label AS out_wallet,
            tin.external_id AS in_external_id,
            tin.occurred_at AS in_occurred_at,
            tin.asset AS in_asset,
            tin.amount AS in_amount,
            win.label AS in_wallet
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE p.profile_id = ? AND p.deleted_at IS NULL
        ORDER BY p.created_at DESC
        LIMIT ?
        """,
        (profile["id"], limit),
    ).fetchall()
    return {
        "summary": {
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "manual_pairs": int(summary["manual_pairs"] or 0),
            "same_asset_pairs": int(summary["same_asset_pairs"] or 0),
            "cross_asset_pairs": int(summary["cross_asset_pairs"] or 0),
            "journal_transfer_entries": int(journal_transfer_entries or 0),
            "limit": limit,
        },
        "pairs": [
            {
                "id": row["id"],
                "kind": row["kind"],
                "policy": row["policy"],
                "created_at": row["created_at"],
                "out": {
                    "transaction_id": row["out_transaction_id"],
                    "external_id": row["out_external_id"] or "",
                    "occurred_at": row["out_occurred_at"],
                    "wallet": row["out_wallet"],
                    "asset": row["out_asset"],
                    "amount": float(msat_to_btc(row["out_amount"] or 0)),
                    "amount_msat": int(row["out_amount"] or 0),
                },
                "in": {
                    "transaction_id": row["in_transaction_id"],
                    "external_id": row["in_external_id"] or "",
                    "occurred_at": row["in_occurred_at"],
                    "wallet": row["in_wallet"],
                    "asset": row["in_asset"],
                    "amount": float(msat_to_btc(row["in_amount"] or 0)),
                    "amount_msat": int(row["in_amount"] or 0),
                },
            }
            for row in rows
        ],
    }


def build_rates_summary_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    rows = conn.execute(
        """
        SELECT
            pair,
            COUNT(*) AS sample_count,
            MIN(timestamp) AS first_timestamp,
            MAX(timestamp) AS last_timestamp
        FROM rates_cache
        GROUP BY pair
        ORDER BY pair ASC
        """
    ).fetchall()
    latest_rows = conn.execute(
        """
        SELECT pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method
        FROM (
            SELECT
                pair,
                timestamp,
                rate,
                rate_exact,
                source,
                fetched_at,
                granularity,
                method,
                ROW_NUMBER() OVER (
                    PARTITION BY pair
                    ORDER BY timestamp DESC,
                             CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                             fetched_at DESC,
                             source ASC
                ) AS rn
            FROM rates_cache
        )
        WHERE rn = 1
        ORDER BY pair ASC
        """
    ).fetchall()
    latest_by_pair = {row["pair"]: dict(row) for row in latest_rows}
    pairs = []
    for row in rows:
        latest = latest_by_pair.get(row["pair"])
        pairs.append(
            {
                "pair": row["pair"],
                "sample_count": int(row["sample_count"] or 0),
                "first_timestamp": row["first_timestamp"],
                "last_timestamp": row["last_timestamp"],
                "latest": {
                    "timestamp": latest["timestamp"],
                    "rate": float(latest["rate"]),
                    "rate_exact": latest["rate_exact"],
                    "source": latest["source"],
                    "fetched_at": latest["fetched_at"],
                    "granularity": latest["granularity"],
                    "method": latest["method"],
                }
                if latest
                else None,
            }
        )
    return {"pairs": pairs, "summary": {"cached_pair_count": len(pairs)}}


def build_rates_coverage_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"limit"})
    if unknown:
        raise AppError(
            "ui.rates.coverage received unsupported filters",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=25, maximum=100)
    context, profile = _active_context_and_profile(conn)
    empty_summary = {
        "active_transactions": 0,
        "priced_transactions": 0,
        "missing_price_transactions": 0,
        "cache_coverable_missing": 0,
        "cache_uncovered_missing": 0,
    }
    if profile is None:
        return {
            "workspace": None,
            "profile": None,
            "summary": empty_summary,
            "items": [],
            "filters": {"limit": limit},
        }

    active_count = int(
        conn.execute(
            "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
            (profile["id"],),
        ).fetchone()["count"]
        or 0
    )
    missing_price_sql = core_rates.transaction_price_missing_sql()
    missing_rows_all = conn.execute(
        """
        SELECT
            t.id,
            t.external_id,
            t.occurred_at,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_rate,
            t.fiat_value,
            t.fiat_rate_exact,
            t.fiat_value_exact,
            w.label AS wallet
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.excluded = 0
          AND (t.amount > 0 OR t.fee > 0)
          AND {missing_price_sql}
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """.format(missing_price_sql=missing_price_sql),
        (profile["id"],),
    ).fetchall()

    def cache_lookup(row: sqlite3.Row) -> tuple[str, sqlite3.Row | None]:
        asset = str(row["asset"] or "").upper()
        fiat = str(row["fiat_currency"] or profile["fiat_currency"] or "EUR").upper()
        pair_asset = "BTC" if asset == "LBTC" else asset
        pair = f"{pair_asset}-{fiat}"
        cache_row = conn.execute(
            """
            SELECT rate, timestamp
            FROM rates_cache
            WHERE pair = ? AND timestamp <= ?
            ORDER BY timestamp DESC, fetched_at DESC
            LIMIT 1
            """,
            (pair, row["occurred_at"]),
        ).fetchone()
        return pair, cache_row

    cache_coverable_total = 0
    cache_uncovered_total = 0
    cache_by_id: dict[str, tuple[str, sqlite3.Row | None]] = {}
    for row in missing_rows_all:
        pair, cache_row = cache_lookup(row)
        cache_by_id[row["id"]] = (pair, cache_row)
        if cache_row:
            cache_coverable_total += 1
        else:
            cache_uncovered_total += 1

    items = []
    for row in missing_rows_all[:limit]:
        pair, cache_row = cache_by_id[row["id"]]
        sign = 1 if row["direction"] == "inbound" else -1
        amount_msat = sign * int(row["amount"] or 0)
        amount_sat = (
            amount_msat // 1000
            if amount_msat % 1000 == 0
            else amount_msat / 1000
        )
        asset = str(row["asset"] or "").upper()
        fiat = str(row["fiat_currency"] or profile["fiat_currency"] or "EUR").upper()
        items.append(
            {
                "id": row["id"],
                "externalId": row["external_id"],
                "date": row["occurred_at"],
                "wallet": row["wallet"],
                "direction": row["direction"],
                "asset": asset,
                "amountSat": amount_sat,
                "amountMsat": amount_msat,
                "fiatCurrency": fiat,
                "cachePair": pair,
                "cacheHasRate": bool(cache_row),
                "cacheRateAt": cache_row["timestamp"] if cache_row else None,
            }
        )

    missing_count = len(missing_rows_all)
    return {
        "workspace": context["workspace_label"] or None,
        "profile": context["profile_label"] or None,
        "summary": {
            "active_transactions": active_count,
            "priced_transactions": active_count - missing_count,
            "missing_price_transactions": missing_count,
            "cache_coverable_missing": cache_coverable_total,
            "cache_uncovered_missing": cache_uncovered_total,
        },
        "items": items,
        "filters": {"limit": limit},
    }


def build_workspace_health_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    context, profile = _active_context_and_profile(conn)
    if profile is None:
        return {
            "workspace": None,
            "profile": None,
            "counts": {
                "wallets": 0,
                "transactions": 0,
                "active_transactions": 0,
                "journal_entries": 0,
                "quarantines": 0,
                "rate_pairs": 0,
            },
            "journals": _journal_freshness(conn, None),
            "reports": {
                "ready": False,
                "hints": ["Create or select a workspace and profile first."],
            },
        }

    freshness = _journal_freshness(conn, profile)
    wallet_count = conn.execute(
        "SELECT COUNT(*) AS count FROM wallets WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    transaction_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    rate_pairs = conn.execute(
        "SELECT COUNT(DISTINCT pair) AS count FROM rates_cache",
    ).fetchone()["count"]
    readiness = _profile_readiness(
        wallet_count=int(wallet_count or 0),
        transaction_count=int(transaction_count or 0),
        freshness=freshness,
    )
    return {
        "workspace": {
            "id": context["workspace_id"],
            "label": context["workspace_label"],
        },
        "profile": {
            "id": profile["id"],
            "label": profile["label"],
            "fiat_currency": profile["fiat_currency"],
            "tax_country": profile["tax_country"],
            "tax_long_term_days": int(profile["tax_long_term_days"] or 0),
            "gains_algorithm": profile["gains_algorithm"],
        },
        "counts": {
            "wallets": int(wallet_count or 0),
            "transactions": int(transaction_count or 0),
            "active_transactions": freshness["active_transaction_count"],
            "journal_entries": freshness["journal_entry_count"],
            "quarantines": freshness["quarantine_count"],
            "rate_pairs": int(rate_pairs or 0),
        },
        "journals": freshness,
        "reports": {
            "ready": _readiness_ready(readiness),
            "hints": readiness["hints"],
        },
    }


def build_report_blockers_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    health = build_workspace_health_snapshot(conn)
    rates_coverage = build_rates_coverage_snapshot(conn, {"limit": 10})
    blockers: list[dict[str, Any]] = []
    if health["profile"] is None:
        blockers.append(
            {
                "id": "no_active_profile",
                "severity": "blocking",
                "title": "No active profile",
                "detail": "Create or select a workspace and profile first.",
                "daemon_kind": "ui.profiles.snapshot",
            }
        )
    else:
        counts = health["counts"]
        journals = health["journals"]
        edit_stale = transaction_history.stale_summary(
            conn,
            {"id": health["profile"]["id"], **journals},
        )
        if counts["wallets"] == 0:
            blockers.append(
                {
                    "id": "no_wallets",
                    "severity": "blocking",
                    "title": "No sources",
                    "detail": (
                        "Add a watch-only source before refreshing, importing, or reporting."
                    ),
                    "daemon_kind": "ui.wallets.list",
                }
            )
        if counts["transactions"] == 0:
            blockers.append(
                {
                    "id": "no_transactions",
                    "severity": "blocking",
                    "title": "No transactions",
                    "detail": "Refresh sources or import transactions before reports can be useful.",
                    "daemon_kind": "ui.wallets.sync",
                }
            )
        if journals["needs_processing"]:
            edit_count = int(edit_stale.get("edit_count") or 0)
            blockers.append(
                {
                    "id": "journals_stale",
                    "severity": "blocking",
                    "title": "Journals need processing",
                    "detail": (
                        f"{journals['reason']}; {edit_count} metadata edit(s) "
                        "changed report inputs."
                        if edit_count
                        else journals["reason"]
                    ),
                    "daemon_kind": "ui.journals.process",
                    "activity_kind": "ui.activity.history",
                    "edit_history": edit_stale,
                }
            )
        if journals["quarantine_count"]:
            blockers.append(
                {
                    "id": "journal_quarantine",
                    "severity": "blocking",
                    "title": "Quarantined journal rows",
                    "detail": f"{journals['quarantine_count']} transaction(s) need review.",
                    "daemon_kind": "ui.journals.quarantine",
                }
            )
        missing_prices = rates_coverage["summary"]["missing_price_transactions"]
        if missing_prices:
            uncovered = rates_coverage["summary"]["cache_uncovered_missing"]
            daemon_kind = "ui.rates.rebuild" if uncovered else "ui.journals.process"
            blockers.append(
                {
                    "id": "missing_prices",
                    "severity": "review",
                    "title": "Missing transaction prices",
                    "detail": f"{missing_prices} transaction(s) are missing fiat price fields.",
                    "daemon_kind": daemon_kind,
                }
            )
    return {
        "ready": not blockers,
        "blockers": blockers,
        "health": health,
        "rates_coverage": rates_coverage,
    }


def build_audit_changes_since_last_answer_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"since"})
    if unknown:
        raise AppError(
            "ui.audit.changes_since_last_answer received unsupported filters",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    since = raw_args.get("since")
    since_filter = None
    if since is not None:
        if not isinstance(since, str) or not since.strip():
            raise AppError("since must be an RFC3339 timestamp", code="validation")
        since_filter = _iso_z(_parse_iso_datetime(since, "since"))

    context, profile = _active_context_and_profile(conn)
    if profile is None:
        status = "no_active_profile" if since_filter is not None else "baseline_required"
        return {
            "status": status,
            "changed": False if status == "no_active_profile" else None,
            "baseline": {"since": since_filter},
            "workspace": None,
            "profile": None,
            "counts_since": {},
            "latest": {},
        }
    if since_filter is None:
        freshness = _journal_freshness(conn, profile)
        return {
            "status": "baseline_required",
            "changed": None,
            "baseline": {"since": None, "required": True},
            "workspace": context["workspace_label"] or None,
            "profile": context["profile_label"] or None,
            "counts_since": {},
            "latest": {
                "transactions": None,
                "journal_entries": None,
                "journal_quarantines": None,
                "transaction_edit_events": None,
                "wallets": None,
                "rates": None,
                "journals_processed_at": profile["last_processed_at"],
            },
            "current": {
                "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
                "active_transactions": freshness["active_transaction_count"],
                "journals_processed_at": profile["last_processed_at"],
                "quarantines": freshness["quarantine_count"],
            },
        }

    def profile_column(table: str, column: str = "created_at") -> str:
        allowed = _AUDIT_PROFILE_TABLE_COLUMNS.get(table)
        if allowed != column:
            raise AssertionError(f"unsupported audit table/column: {table}.{column}")
        return allowed

    def global_column(table: str, column: str) -> str:
        allowed = _AUDIT_GLOBAL_TABLE_COLUMNS.get(table)
        if allowed != column:
            raise AssertionError(f"unsupported audit table/column: {table}.{column}")
        return allowed

    def count_since(table: str, column: str = "created_at") -> int:
        if since_filter is None:
            return 0
        column = profile_column(table, column)
        return int(
            conn.execute(
                f"SELECT COUNT(*) AS count FROM {table} WHERE profile_id = ? AND {column} > ?",
                (profile["id"], since_filter),
            ).fetchone()["count"]
            or 0
        )

    def latest(table: str, column: str = "created_at") -> str | None:
        if table == "rates_cache":
            column = global_column(table, column)
            row = conn.execute(f"SELECT MAX({column}) AS latest FROM {table}").fetchone()
        else:
            column = profile_column(table, column)
            row = conn.execute(
                f"SELECT MAX({column}) AS latest FROM {table} WHERE profile_id = ?",
                (profile["id"],),
            ).fetchone()
        return row["latest"] if row and row["latest"] else None

    counts_since = {
        "transactions": count_since("transactions"),
        "journal_entries": count_since("journal_entries"),
        "journal_quarantines": count_since("journal_quarantines"),
        "transaction_edit_events": count_since("transaction_edit_events", "changed_at"),
        "wallets": count_since("wallets"),
        "rates": int(
            conn.execute(
                "SELECT COUNT(*) AS count FROM rates_cache WHERE fetched_at > ?",
                (since_filter,),
            ).fetchone()["count"]
            or 0
        )
        if since_filter
        else 0,
    }
    freshness = _journal_freshness(conn, profile)
    return {
        "status": "compared",
        "changed": any(value > 0 for value in counts_since.values()),
        "baseline": {"since": since_filter},
        "workspace": context["workspace_label"] or None,
        "profile": context["profile_label"] or None,
        "counts_since": counts_since,
        "latest": {
            "transactions": latest("transactions"),
            "journal_entries": latest("journal_entries"),
            "journal_quarantines": latest("journal_quarantines"),
            "transaction_edit_events": latest("transaction_edit_events", "changed_at"),
            "wallets": latest("wallets"),
            "rates": latest("rates_cache", "fetched_at"),
            "journals_processed_at": profile["last_processed_at"],
        },
        "current": {
            "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
            "active_transactions": freshness["active_transaction_count"],
            "journals_processed_at": profile["last_processed_at"],
            "quarantines": freshness["quarantine_count"],
        },
    }


def build_next_actions_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    health = build_workspace_health_snapshot(conn)
    suggestions: list[dict[str, Any]] = []
    if health["profile"] is None:
        suggestions.append(
            {
                "id": "create_workspace_profile",
                "title": "Create a workspace and profile",
                "reason": "Kassiber needs an active accounting scope before sources or reports.",
                "mutating": True,
                "requires_consent": True,
            }
        )
        return {"health": health, "suggestions": suggestions}

    counts = health["counts"]
    journals = health["journals"]
    if counts["wallets"] == 0:
        suggestions.append(
            {
                "id": "create_wallet",
                "title": "Add a watch-only source",
                "reason": "No sources exist in the active profile.",
                "mutating": True,
                "requires_consent": True,
            }
        )
    elif counts["transactions"] == 0:
        suggestions.append(
            {
                "id": "sync_or_import",
                "title": "Refresh sources or import transactions",
                "reason": "Sources exist but the active profile has no transactions yet.",
                "mutating": True,
                "requires_consent": True,
                "daemon_kind": "ui.wallets.sync",
            }
        )
    elif journals["needs_processing"]:
        suggestions.append(
            {
                "id": "process_journals",
                "title": "Process journals",
                "reason": journals["reason"],
                "mutating": True,
                "requires_consent": True,
                "daemon_kind": "ui.journals.process",
            }
        )
    else:
        rates_coverage = build_rates_coverage_snapshot(conn, {"limit": 1})
        missing_prices = rates_coverage["summary"]["missing_price_transactions"]
        if missing_prices:
            uncovered = rates_coverage["summary"]["cache_uncovered_missing"]
            suggestions.append(
                {
                    "id": "fetch_missing_prices" if uncovered else "apply_cached_prices",
                    "title": "Fetch missing spot prices" if uncovered else "Apply cached spot prices",
                    "reason": f"{missing_prices} transaction(s) still need fiat prices.",
                    "mutating": True,
                    "requires_consent": True,
                    "daemon_kind": "ui.rates.rebuild" if uncovered else "ui.journals.process",
                }
            )
    if journals["quarantine_count"]:
        suggestions.append(
            {
                "id": "review_quarantine",
                "title": "Review quarantine",
                "reason": f"{journals['quarantine_count']} transaction(s) need review before reports are complete.",
                "mutating": False,
                "requires_consent": False,
                "daemon_kind": "ui.journals.quarantine",
            }
        )
    if (
        counts["transactions"] > 0
        and not journals["needs_processing"]
        and journals["quarantine_count"] == 0
    ):
        suggestions.append(
            {
                "id": "run_report",
                "title": "Run reports",
                "reason": "Journals are current and no quarantine is blocking report review.",
                "mutating": False,
                "requires_consent": False,
                "daemon_kind": "ui.reports.capital_gains",
            }
        )
    if not suggestions:
        suggestions.append(
            {
                "id": "inspect_workspace",
                "title": "Inspect workspace",
                "reason": "No urgent blocker was detected from the current safe snapshot.",
                "mutating": False,
                "requires_consent": False,
                "daemon_kind": "ui.workspace.health",
            }
        )
    return {"health": health, "suggestions": suggestions}


def _transaction_rows_to_ui(
    conn: sqlite3.Connection,
    rows: list[sqlite3.Row],
) -> list[dict[str, Any]]:
    pair_meta_by_transaction = _transaction_pair_display_meta(conn, rows)
    tags_by_transaction = _transaction_tags_by_transaction(
        conn,
        [row["id"] for row in rows],
    )

    output = []
    rendered_pair_ids: set[str] = set()
    for row in rows:
        pair_meta = pair_meta_by_transaction.get(row["id"])
        metadata_tags = [
            str(tag) for tag in tags_by_transaction.get(row["id"], []) if tag
        ]
        if pair_meta:
            pair_id = str(pair_meta["pair_id"])
            if pair_id in rendered_pair_ids:
                continue
            rendered_pair_ids.add(pair_id)
        output.append(_transaction_row_to_ui(row, metadata_tags, pair_meta))
    return output


def _transaction_rows_to_ui_page(
    conn: sqlite3.Connection,
    rows: list[sqlite3.Row],
    limit: int,
    skip_pairs: set[str] | None = None,
) -> tuple[list[dict[str, Any]], sqlite3.Row | None, bool, set[str]]:
    pair_meta_by_transaction = _transaction_pair_display_meta(conn, rows)
    tags_by_transaction = _transaction_tags_by_transaction(
        conn,
        [row["id"] for row in rows],
    )

    output: list[dict[str, Any]] = []
    pending_skip_pairs = set(skip_pairs or set())
    rendered_pair_ids: set[str] = set()
    rendered_pair_meta: dict[str, dict[str, Any]] = {}
    consumed_ids: set[str] = set()
    consumed_row: sqlite3.Row | None = None

    for row in rows:
        pair_meta = pair_meta_by_transaction.get(row["id"])
        pair_id = str(pair_meta["pair_id"]) if pair_meta else None
        row_would_render = not pair_id or (
            pair_id not in pending_skip_pairs and pair_id not in rendered_pair_ids
        )
        if len(output) >= limit and row_would_render:
            return output, consumed_row, True, pending_skip_pairs | {
                rendered_pair_id
                for rendered_pair_id, meta in rendered_pair_meta.items()
                if meta.get("out_transaction_id") not in consumed_ids
                or meta.get("in_transaction_id") not in consumed_ids
            }

        consumed_row = row
        consumed_ids.add(str(row["id"]))

        if pair_id:
            if pair_id in pending_skip_pairs:
                pending_skip_pairs.discard(pair_id)
                continue
            if pair_id in rendered_pair_ids:
                continue
            rendered_pair_ids.add(pair_id)
            rendered_pair_meta[pair_id] = pair_meta or {}

        metadata_tags = [
            str(tag) for tag in tags_by_transaction.get(row["id"], []) if tag
        ]
        output.append(_transaction_row_to_ui(row, metadata_tags, pair_meta))

    next_skip_pairs = pending_skip_pairs | {
        rendered_pair_id
        for rendered_pair_id, meta in rendered_pair_meta.items()
        if meta.get("out_transaction_id") not in consumed_ids
        or meta.get("in_transaction_id") not in consumed_ids
    }
    return output, consumed_row, False, next_skip_pairs


def _activity_transaction_rows_to_ui(
    conn: sqlite3.Connection,
    rows: list[sqlite3.Row] | list[dict[str, Any]],
) -> list[dict[str, Any]]:
    pair_meta_by_transaction: dict[str, dict[str, Any]] = {}
    # Keep the overview chart uncapped without building giant IN (...) queries.
    for start in range(0, len(rows), 400):
        chunk = rows[start : start + 400]
        pair_meta_by_transaction.update(_transaction_pair_display_meta(conn, chunk))

    output = []
    rendered_pair_ids: set[str] = set()
    pair_final_rows: dict[str, sqlite3.Row | dict[str, Any]] = {}
    for row in rows:
        pair_meta = pair_meta_by_transaction.get(row["id"])
        if pair_meta:
            pair_final_rows[str(pair_meta["pair_id"])] = row

    for row in rows:
        pair_meta = pair_meta_by_transaction.get(row["id"])
        output_row = row
        if pair_meta:
            pair_id = str(pair_meta["pair_id"])
            if pair_id in rendered_pair_ids:
                continue
            rendered_pair_ids.add(pair_id)
            output_row = pair_final_rows.get(pair_id, row)
        # Activity chart rows intentionally skip metadata tags to keep the
        # uncapped overview payload and SQLite parameter count bounded.
        output.append(_transaction_row_to_ui(output_row, [], pair_meta))
    return output
