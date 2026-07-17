from __future__ import annotations

import base64
import binascii
import json
import sqlite3
from collections import defaultdict
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal
from typing import Any

from ..backends import (
    DEFAULT_BACKEND_SETTING,
    backend_value,
    redact_backend_for_output,
)
from ..db import get_setting
from ..errors import AppError
from ..msat import dec, msat_to_btc
from .journal_markers import (
    MARKER_ALT_IN,
    MARKER_ALT_OUT,
    MARKER_REGIME,
    parse_marker,
    parse_marker_int,
)
from ..time_utils import _iso_z, _parse_iso_datetime, now_iso
from ..transfers import profile_bitcoin_rail_carrying_value
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
from . import freshness as core_freshness
from . import custody_components as core_custody_components
from . import custody_journal as core_custody_journal
from . import custody_quantity_store as core_custody_quantity_store
from . import lightning as core_lightning
from . import rates as core_rates
from . import silent_payments
from . import sync_backends as core_sync_backends
from . import source_overlap as core_source_overlap
from . import transfer_matching as core_transfer_matching
from . import reports as report_builders
from .austrian import vienna_local_date
from .samourai import samourai_metadata_from_wallet_config
from . import transaction_history
from .repo import current_context_snapshot
from .sync import normalize_backend_kind
from .wallets import (
    has_descriptor_sync_material,
    has_silent_payment_sync_material,
    load_wallet_descriptor_plan_from_config,
    wallet_btcpay_provenance_config,
    wallet_is_deprecated,
)


MAX_UI_LIST_LIMIT = 500
MAX_UI_PREVIEW_LIMIT = 100
DISPLAY_BALANCE_ASSETS = {"BTC", "LBTC", "L-BTC"}
_UI_TRANSACTION_SORT_COLUMNS = {
    "occurred-at": "t.occurred_at",
    "amount": "t.amount",
    "fiat-value": "COALESCE(t.fiat_value, 0)",
    "fee": "t.fee",
}
_UI_TRANSACTION_FLOW_KINDS = {
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap",
    "swap-refund",
}
_UI_TRANSACTION_LAYER_TRANSITION_KINDS = {
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
}
_UI_TRANSACTION_PAYMENT_METHODS = {
    "exchange": "Exchange",
    "lightning": "Lightning",
    "liquid": "Liquid",
    "on-chain": "On-chain",
    "onchain": "On-chain",
    "on chain": "On-chain",
}
_UI_TRANSACTION_PERIOD_DAYS = {
    "30days": 29,
    "30day": 29,
    "30d": 29,
    "3months": 92,
    "3month": 92,
    "3m": 92,
    "6months": 183,
    "6month": 183,
    "6m": 183,
    "1year": 365,
    "1years": 365,
    "1y": 365,
    "5years": 365 * 5,
    "5year": 365 * 5,
    "5y": 365 * 5,
    "10years": 365 * 10,
    "10year": 365 * 10,
    "10y": 365 * 10,
    "15years": 365 * 15,
    "15year": 365 * 15,
    "15y": 365 * 15,
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
        "taxFreeBalance": None,
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
    txids: list[str],
    status: str | None,
    flow: str | None,
    payment_method: str | None,
    network: str | None,
    with_fees: bool,
    quick: str | None,
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
        "txids": ",".join(txids),
        "status": status or "",
        "flow": flow or "",
        "payment_method": payment_method or "",
        "network": network or "",
        "with_fees": "1" if with_fees else "",
        "quick": quick or "",
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


def _coerce_optional_string_arg(args: dict[str, Any], key: str) -> str | None:
    value = args.get(key)
    if value is None:
        return None
    if not isinstance(value, str) or not value.strip():
        raise AppError(
            f"{key} must be a non-empty string",
            code="validation",
            details={key: value},
            retryable=False,
        )
    return value.strip()


def _coerce_optional_string_list_arg(args: dict[str, Any], key: str) -> list[str]:
    value = args.get(key)
    if value is None:
        return []
    values: list[Any]
    if isinstance(value, str):
        values = value.split(",")
    elif isinstance(value, list):
        values = value
    else:
        raise AppError(
            f"{key} must be a string or a list of strings",
            code="validation",
            details={key: value},
            retryable=False,
        )
    output: list[str] = []
    seen: set[str] = set()
    for item in values:
        if not isinstance(item, str):
            raise AppError(
                f"{key} must contain only strings",
                code="validation",
                details={key: value},
                retryable=False,
            )
        normalized = item.strip()
        if normalized and normalized.lower() not in seen:
            output.append(normalized)
            seen.add(normalized.lower())
    return output


def _coerce_optional_bool_arg(args: dict[str, Any], *keys: str) -> bool:
    for key in keys:
        if key not in args:
            continue
        value = args.get(key)
        if isinstance(value, bool):
            return value
        if value in (0, 1):
            return bool(value)
        if isinstance(value, str):
            normalized = value.strip().lower()
            if normalized in {"1", "true", "yes", "on", "with-fees"}:
                return True
            if normalized in {"0", "false", "no", "off", "all", ""}:
                return False
        raise AppError(
            f"{key} must be a boolean",
            code="validation",
            details={key: value},
            retryable=False,
        )
    return False


def _coerce_ui_transaction_period(value: str) -> str:
    normalized = value.strip().lower().replace("_", "").replace("-", "").replace(" ", "")
    if normalized == "ytd":
        return "ytd"
    if normalized in {"all", "max"}:
        return "all"
    if normalized in _UI_TRANSACTION_PERIOD_DAYS:
        return normalized
    raise AppError(
        "period must be one of: 30days, 3months, 6months, ytd, 1year, 5years, 10years, 15years, all",
        code="validation",
        details={"period": value},
        retryable=False,
    )


def _ui_transaction_since_for_period(period: str) -> str | None:
    if period == "all":
        return None
    now = datetime.now(timezone.utc)
    if period == "ytd":
        return _iso_z(datetime(now.year, 1, 1, tzinfo=timezone.utc))
    days = _UI_TRANSACTION_PERIOD_DAYS[period]
    start = now - timedelta(days=days)
    return _iso_z(start.replace(hour=0, minute=0, second=0, microsecond=0))


def _normalize_ui_transaction_status(value: str) -> str:
    normalized = value.strip().lower().replace("-", "_")
    aliases = {
        "complete": "completed",
        "error": "failed",
        "needs_review": "review",
        "blocked": "review",
        "quarantined": "review",
    }
    normalized = aliases.get(normalized, normalized)
    if normalized not in {"completed", "pending", "failed", "review"}:
        raise AppError(
            "status must be one of: completed, pending, failed, review",
            code="validation",
            details={"status": value},
            retryable=False,
        )
    return normalized


def _normalize_ui_transaction_flow(value: str) -> str:
    normalized = value.strip().lower().replace("_", "-")
    if normalized in {"incoming", "outgoing", "transfer", "swap", "layer-transition"}:
        return normalized
    raise AppError(
        "flow must be one of: incoming, outgoing, transfer, swap, layer-transition",
        code="validation",
        details={"flow": value},
        retryable=False,
    )


def _normalize_ui_transaction_payment_method(value: str) -> str:
    normalized = value.strip().lower()
    payment_method = _UI_TRANSACTION_PAYMENT_METHODS.get(normalized)
    if payment_method:
        return payment_method
    raise AppError(
        "payment_method must be one of: On-chain, Exchange, Lightning, Liquid",
        code="validation",
        details={"payment_method": value},
        retryable=False,
    )


def _normalize_ui_transaction_quick_filter(value: str) -> str:
    normalized = value.strip().lower().replace("-", "_")
    if normalized in {
        "external_flow",
        "review_queue",
        "no_explorer_id",
        "missing_price",
        "failed_import",
    }:
        return normalized
    raise AppError(
        "quick must be one of: external_flow, review_queue, no_explorer_id, missing_price, failed_import",
        code="validation",
        details={"quick": value},
        retryable=False,
    )


def _ui_transaction_pair_exists_sql(pair_type: str | None = None) -> str:
    if pair_type == "swap":
        return """
        EXISTS (
          SELECT 1
          FROM journal_custody_economic_relations relation
          WHERE relation.profile_id = t.profile_id
            AND relation.relation_kind = 'conversion'
            AND relation.target_transaction_id IS NOT NULL
            AND (
              relation.source_transaction_id = t.id
              OR relation.target_transaction_id = t.id
            )
        )
        """
    elif pair_type == "transfer":
        return """
        EXISTS (
          SELECT 1
          FROM journal_custody_decisions decision
          WHERE decision.profile_id = t.profile_id
            AND decision.source_asset = decision.target_asset
            AND (
              decision.source_transaction_id = t.id
              OR decision.target_transaction_id = t.id
            )
        )
        """
    return f"""
        ({_ui_transaction_pair_exists_sql('transfer')}
         OR {_ui_transaction_pair_exists_sql('swap')})
    """


def _ui_transaction_custody_group_sql(*, projection_current: bool = True) -> str:
    if not projection_current:
        return "'tx:' || t.id"
    return """
        COALESCE(
          (
            SELECT 'decision:' || decision.decision_id
            FROM journal_custody_decisions decision
            WHERE decision.profile_id = t.profile_id
              AND (
                decision.source_transaction_id = t.id
                OR decision.target_transaction_id = t.id
              )
            ORDER BY decision.occurred_at, decision.decision_id
            LIMIT 1
          ),
          (
            SELECT 'relation:' || relation.relation_id
            FROM journal_custody_economic_relations relation
            WHERE relation.profile_id = t.profile_id
              AND relation.relation_kind = 'conversion'
              AND relation.target_transaction_id IS NOT NULL
              AND (
                relation.source_transaction_id = t.id
                OR relation.target_transaction_id = t.id
              )
            ORDER BY relation.occurred_at, relation.relation_id
            LIMIT 1
          ),
          'tx:' || t.id
        )
    """.strip()


def _ui_transaction_payment_method_sql() -> str:
    return """
        CASE
          WHEN lower(t.asset) = 'lbtc'
            OR lower(w.kind) IN ('liquid')
            OR lower(w.config_json) LIKE '%"chain"%liquid%'
            OR lower(w.label) LIKE '%liquid%'
            OR lower(w.label) LIKE '%lbtc%'
            THEN 'Liquid'
          WHEN lower(w.kind) IN ('lnd', 'core-ln', 'coreln', 'nwc', 'phoenix')
            OR lower(w.label) LIKE '%lightning%'
            OR lower(w.label) LIKE '%phoenix%'
            OR lower(w.label) LIKE '% ln%'
            OR lower(w.label) LIKE 'ln %'
            OR lower(w.label) LIKE '%(ln)%'
            THEN 'Lightning'
          WHEN lower(w.kind) IN (
              'kraken', 'bitstamp', 'coinbase', 'bitpanda', 'river',
              'bullbitcoin', 'coinfinity', 'strike', 'exchange'
            )
            OR lower(w.label) LIKE '%exchange%'
            THEN 'Exchange'
          ELSE 'On-chain'
        END
    """.strip()


def _ui_transaction_status_sql() -> str:
    return """
        CASE
          WHEN lower(COALESCE(t.review_status, '')) IN
               ('review', 'needs_review', 'needs-review', 'blocked', 'quarantined')
            OR jq.reason IS NOT NULL
            THEN 'review'
          WHEN lower(COALESCE(t.review_status, '')) IN ('failed', 'error')
            THEN 'failed'
          WHEN lower(COALESCE(t.review_status, '')) IN ('completed', 'complete')
            THEN 'completed'
          WHEN t.confirmed_at IS NULL
            THEN 'pending'
          ELSE 'completed'
        END
    """.strip()


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
    has_silent_payment = has_silent_payment_sync_material(config)
    has_addresses = bool(config.get("addresses"))
    backend_name = explicit_backend
    backend_source = "explicit" if explicit_backend else "none"
    sync_mode = "not_configured"

    if sync_source == "btcpay":
        sync_mode = "btcpay"
    elif source_file and source_format:
        sync_mode = "file_import"
    elif has_silent_payment and kind == "silent-payment":
        sync_mode = "backend_silent_payment"
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

    freshness = core_custody_journal.projection_freshness(conn, profile)
    journal_entries = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_entries WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    quarantines = conn.execute(
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile["id"],),
    ).fetchone()["count"]
    return {
        **freshness,
        "journal_entry_count": int(journal_entries or 0),
        "quarantine_count": int(quarantines or 0),
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


def _book_quantity_delta(entry_type: str, quantity_msat: int | None) -> Decimal:
    return report_builders._holdings_quantity_delta(
        entry_type,
        msat_to_btc(quantity_msat),
    )


def _balance_series_from_month_deltas(
    by_month: dict[str, Decimal],
) -> list[float]:
    if not by_month:
        return [0.0] * 12
    months = set(sorted(by_month)[-12:])
    cumulative = Decimal("0")
    series = []
    for month in sorted(by_month):
        cumulative += by_month[month]
        if month in months:
            series.append(float(cumulative))
    if len(series) < 12:
        series = [series[0]] * (12 - len(series)) + series
    return series[-12:]


def _has_book_balance_state(freshness: dict[str, Any]) -> bool:
    if freshness.get("needs_processing"):
        return False
    return bool(freshness.get("last_processed_at")) or int(
        freshness.get("journal_entry_count") or 0,
    ) > 0


def _journal_wallet_holdings_balances(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float] | None:
    try:
        rows = conn.execute(
            """
            SELECT wallet_id, SUM(quantity) AS quantity
            FROM journal_wallet_holdings
            WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
            GROUP BY wallet_id
            """,
            (profile_id,),
        ).fetchall()
    except sqlite3.OperationalError:
        rows = []
    if rows:
        return {
            row["wallet_id"]: float(msat_to_btc(row["quantity"] or 0))
            for row in rows
        }
    return None


def _canonical_quantity_processed(
    conn: sqlite3.Connection,
    profile_id: str,
) -> bool:
    try:
        row = conn.execute(
            """
            SELECT EXISTS(
                SELECT 1 FROM journal_quantity_postings WHERE profile_id = ?
            ) OR EXISTS(
                SELECT 1 FROM journal_quantity_issues WHERE profile_id = ?
            ) AS present
            """,
            (profile_id, profile_id),
        ).fetchone()
    except sqlite3.OperationalError:
        return False
    return bool(row and row["present"])


def _canonical_wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float] | None:
    if not _canonical_quantity_processed(conn, profile_id):
        return None
    rows = conn.execute(
        """
        SELECT location_id AS wallet_id, SUM(amount_msat) AS amount_msat
        FROM journal_quantity_balances
        WHERE profile_id = ? AND location_kind = 'wallet'
          AND asset IN ('BTC', 'LBTC')
        GROUP BY location_id
        """,
        (profile_id,),
    ).fetchall()
    return {
        str(row["wallet_id"]): float(msat_to_btc(row["amount_msat"] or 0))
        for row in rows
    }


def _journal_entry_wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, float]:
    rows = conn.execute(
        """
        SELECT wallet_id, entry_type, quantity
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    balances: dict[str, Decimal] = defaultdict(lambda: Decimal("0"))
    for row in rows:
        balances[row["wallet_id"]] += _book_quantity_delta(
            row["entry_type"],
            row["quantity"],
        )
    return {wallet_id: float(quantity) for wallet_id, quantity in balances.items()}


def _journal_quantity_deltas_by_transaction(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, Decimal]:
    if _canonical_quantity_processed(conn, profile_id):
        rows = conn.execute(
            """
            SELECT transaction_id, SUM(amount_msat) AS amount_msat
            FROM journal_quantity_postings
            WHERE profile_id = ? AND location_kind = 'wallet'
              AND state = 'observed' AND transaction_id IS NOT NULL
              AND asset IN ('BTC', 'LBTC')
            GROUP BY transaction_id
            ORDER BY MIN(occurred_at), transaction_id
            """,
            (profile_id,),
        ).fetchall()
        return {
            str(row["transaction_id"]): msat_to_btc(row["amount_msat"] or 0)
            for row in rows
        }
    rows = conn.execute(
        """
        SELECT transaction_id, entry_type, quantity
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    deltas: dict[str, Decimal] = defaultdict(lambda: Decimal("0"))
    for row in rows:
        transaction_id = row["transaction_id"]
        if not transaction_id:
            continue
        deltas[str(transaction_id)] += _book_quantity_delta(
            row["entry_type"],
            row["quantity"],
        )
    return deltas


def _journal_quantity_deltas_by_day(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[date, Decimal]:
    if _canonical_quantity_processed(conn, profile_id):
        rows = conn.execute(
            """
            SELECT occurred_at, SUM(amount_msat) AS amount_msat
            FROM journal_quantity_postings
            WHERE profile_id = ? AND location_kind = 'wallet'
              AND state = 'observed' AND asset IN ('BTC', 'LBTC')
            GROUP BY occurred_at
            ORDER BY occurred_at
            """,
            (profile_id,),
        ).fetchall()
        deltas: dict[date, Decimal] = defaultdict(lambda: Decimal("0"))
        for row in rows:
            day = _parse_day(row["occurred_at"])
            if day is not None:
                deltas[day] += msat_to_btc(row["amount_msat"] or 0)
        return deltas
    rows = conn.execute(
        """
        SELECT
            COALESCE(t.occurred_at, je.occurred_at) AS occurred_at,
            je.entry_type,
            je.quantity
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ? AND je.asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile_id,),
    ).fetchall()
    deltas: dict[date, Decimal] = defaultdict(lambda: Decimal("0"))
    for row in rows:
        day = _parse_day(row["occurred_at"])
        if day is None:
            continue
        deltas[day] += _book_quantity_delta(row["entry_type"], row["quantity"])
    return deltas


def _journal_wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    has_book_state: bool,
) -> dict[str, float] | None:
    if not has_book_state:
        return None
    canonical_balances = _canonical_wallet_balances(conn, profile_id)
    if canonical_balances is not None:
        return canonical_balances
    holdings_balances = _journal_wallet_holdings_balances(conn, profile_id)
    if holdings_balances is not None:
        return holdings_balances
    return _journal_entry_wallet_balances(conn, profile_id)


def _wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    has_book_state: bool,
) -> dict[str, float]:
    book_balances = _journal_wallet_balances(
        conn,
        profile_id,
        has_book_state=has_book_state,
    )
    if book_balances is not None:
        return book_balances
    return _transaction_wallet_balances(conn, profile_id)


def _db_backend_for_wallet_balance(
    conn: sqlite3.Connection,
    backend_name: str,
) -> dict[str, Any] | None:
    if not backend_name:
        return None
    row = conn.execute(
        """
        SELECT name, kind, chain, network, batch_size, timeout, config_json
        FROM backends
        WHERE name = ?
        """,
        (backend_name,),
    ).fetchone()
    if row is None:
        return None
    backend = {
        "name": row["name"],
        "kind": row["kind"],
        "chain": row["chain"],
        "network": row["network"],
        "batch_size": row["batch_size"],
        "timeout": row["timeout"],
    }
    backend.update(_json_config(row["config_json"]))
    return backend


def _chain_balance_for_wallet(
    conn: sqlite3.Connection,
    row: sqlite3.Row,
    config: dict[str, Any],
    default_backend: str | None,
) -> dict[str, Any] | None:
    backend_summary = _wallet_backend_summary(row["kind"], config, default_backend)
    sync_mode = backend_summary["sync_mode"]
    if (
        _string_or_empty(config.get("source_format")) != "wasabi_bundle"
        and sync_mode
        not in {"backend_descriptor", "backend_addresses", "backend_silent_payment"}
    ):
        return None
    backend = _db_backend_for_wallet_balance(conn, backend_summary["name"])
    source_filter = _wallet_utxo_source_filter(config, backend_summary, backend)
    inventory_summary = core_output_inventory.wallet_output_inventory_summary(
        conn,
        row["id"],
        **source_filter,
    )
    if not inventory_summary["last_seen_at"]:
        return None
    totals = core_output_inventory.wallet_output_inventory_totals(
        conn,
        row["id"],
        **source_filter,
    )
    balance = sum(
        float(total.get("amount") or 0)
        for total in totals
        if str(total.get("asset") or "").upper() in DISPLAY_BALANCE_ASSETS
    )
    return {
        "balance": balance,
        "lastSeenAt": inventory_summary["last_seen_at"],
        "activeCount": inventory_summary["active_count"],
    }


def _display_wallet_balances(
    conn: sqlite3.Connection,
    profile_id: str,
    book_balances: dict[str, float],
    *,
    fallback_source: str,
) -> dict[str, dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT id, kind, config_json
        FROM wallets
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    default_backend = (
        _string_or_empty(get_setting(conn, DEFAULT_BACKEND_SETTING)) or None
    )
    balances: dict[str, dict[str, Any]] = {}
    for row in rows:
        wallet_id = str(row["id"])
        book_balance = book_balances.get(wallet_id, 0.0)
        config = _json_config(row["config_json"])
        chain_balance = _chain_balance_for_wallet(conn, row, config, default_backend)
        if chain_balance is None:
            balances[wallet_id] = {
                "balance": book_balance,
                "balanceSource": fallback_source,
                "bookBalance": book_balance,
                "chainBalance": None,
                "chainLastSeenAt": None,
                "chainActiveCount": None,
            }
            continue
        balances[wallet_id] = {
            "balance": chain_balance["balance"],
            "balanceSource": "chain",
            "bookBalance": book_balance,
            "chainBalance": chain_balance["balance"],
            "chainLastSeenAt": chain_balance["lastSeenAt"],
            "chainActiveCount": chain_balance["activeCount"],
        }
    return balances


def _chain_duplicate_outpoint_adjustment_btc(
    conn: sqlite3.Connection,
    profile_id: str,
    display_balance_info: dict[str, dict[str, Any]],
) -> float:
    chain_wallet_ids = sorted(
        wallet_id
        for wallet_id, info in display_balance_info.items()
        if info.get("balanceSource") == "chain"
    )
    if len(chain_wallet_ids) < 2:
        return 0.0
    placeholders = ", ".join("?" for _ in chain_wallet_ids)
    rows = conn.execute(
        f"""
        SELECT id, kind, config_json
        FROM wallets
        WHERE profile_id = ?
          AND id IN ({placeholders})
        """,
        (profile_id, *chain_wallet_ids),
    ).fetchall()
    default_backend = (
        _string_or_empty(get_setting(conn, DEFAULT_BACKEND_SETTING)) or None
    )
    # Only outpoints that actually feed a wallet's displayed chain balance may be
    # deduped. Apply the same per-wallet source filter as _chain_balance_for_wallet
    # so stale wallet_utxos rows (e.g. left behind by a backends update that
    # changed chain/network/kind) are never subtracted from the aggregate even
    # though they were never added to it.
    amounts_by_outpoint: dict[tuple[str, str], list[int]] = {}
    for row in rows:
        wallet_id = str(row["id"])
        config = _json_config(row["config_json"])
        backend_summary = _wallet_backend_summary(row["kind"], config, default_backend)
        backend = _db_backend_for_wallet_balance(conn, backend_summary["name"])
        source_filter = _wallet_utxo_source_filter(config, backend_summary, backend)
        for entry in core_output_inventory.wallet_unspent_outpoint_amounts(
            conn,
            wallet_id,
            assets=sorted(DISPLAY_BALANCE_ASSETS),
            **source_filter,
        ):
            key = (entry["asset"], entry["outpoint_key"])
            amounts_by_outpoint.setdefault(key, []).append(entry["amount_msat"])
    duplicate_msat = 0
    for amounts in amounts_by_outpoint.values():
        if len(amounts) < 2:
            continue
        duplicate_msat += max(0, sum(amounts) - max(amounts))
    return float(msat_to_btc(duplicate_msat))


def _connections(
    conn: sqlite3.Connection,
    profile_id: str,
    balances: dict[str, dict[str, Any]],
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
        chain = _string_or_empty(config.get("chain"))
        network = _string_or_empty(config.get("network"))
        policy_asset = _string_or_empty(config.get("policy_asset"))
        payment_method_id = _string_or_empty(config.get("payment_method_id"))
        gap_limit = config.get("gap_limit")
        has_descriptor = has_descriptor_sync_material(config)
        try:
            wallet_chain = normalize_chain(config.get("chain") or "bitcoin")
        except ValueError:
            wallet_chain = "bitcoin"
        connection = {
            "id": row["id"],
            "kind": _map_wallet_kind(row["kind"]),
            "label": row["label"],
            "last": _relative_last(
                last_synced_at or row["last_tx_at"] or row["created_at"]
            ),
            "lastSyncAt": last_synced_at or None,
            "lastTransactionAt": row["last_tx_at"],
            "balance": balances.get(row["id"], {}).get("balance", 0.0),
            "balanceSource": balances.get(row["id"], {}).get("balanceSource", "books"),
            "bookBalance": balances.get(row["id"], {}).get("bookBalance", 0.0),
            "chainBalance": balances.get(row["id"], {}).get("chainBalance"),
            "chainLastSeenAt": balances.get(row["id"], {}).get("chainLastSeenAt"),
            "chainActiveCount": balances.get(row["id"], {}).get("chainActiveCount"),
            "status": "synced" if tx_count else "idle",
            "transactionCount": tx_count,
            "syncMode": backend_summary["sync_mode"],
            "syncSource": sync_source,
            "sourceFormat": source_format,
            "deprecated": wallet_is_deprecated(config),
            "chain": chain or wallet_chain,
            "network": network or None,
            "policyAsset": policy_asset or None,
            "paymentMethodId": payment_method_id or None,
        }
        if has_descriptor:
            connection["gap"] = (
                int(gap_limit)
                if gap_limit not in (None, "")
                else DEFAULT_DESCRIPTOR_GAP_LIMIT
            )
        if row["kind"] in core_lightning.LIGHTNING_ADAPTER_KINDS:
            connection["lightningCapabilities"] = (
                core_lightning.registered_capabilities(row["kind"]).to_wire_dict()
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
            w.kind AS wallet_kind,
            w.config_json AS wallet_config_json,
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


def _activity_transactions(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    has_book_state: bool,
) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT
            t.id,
            t.external_id AS external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            w.kind AS wallet_kind,
            w.config_json AS wallet_config_json,
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
          AND t.asset IN ('BTC', 'LBTC')
          AND COALESCE(t.excluded, 0) = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile_id,),
    ).fetchall()
    activity_rows: list[dict[str, Any]] = []
    quantity_msat = 0
    quantity_btc = Decimal("0")
    book_deltas_by_transaction = (
        _journal_quantity_deltas_by_transaction(conn, profile_id)
        if has_book_state
        else {}
    )
    cost_basis_by_transaction = _portfolio_cost_basis_by_transaction(conn, profile_id)
    running_cost_basis = 0.0
    for row in rows:
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        if has_book_state:
            quantity_btc += book_deltas_by_transaction.get(
                str(row["id"]),
                Decimal("0"),
            )
            running_balance_btc = float(quantity_btc)
        else:
            quantity_msat += (
                amount if row["direction"] == "inbound" else -amount - fee
            )
            running_balance_btc = float(msat_to_btc(quantity_msat))
        running_cost_basis = cost_basis_by_transaction.get(
            str(row["id"]),
            running_cost_basis,
        )
        activity_rows.append(
            {
                **dict(row),
                "running_balance_btc": running_balance_btc,
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


def _transaction_row_chain_network(row: sqlite3.Row | dict[str, Any]) -> tuple[str, str]:
    row_keys = set(row.keys())
    asset = _string_or_empty(row["asset"] if "asset" in row_keys else "").upper()
    wallet_kind = _string_or_empty(
        row["wallet_kind"] if "wallet_kind" in row_keys else "",
    ).lower()
    config = _json_config(
        row["wallet_config_json"] if "wallet_config_json" in row_keys else None,
    )
    fallback_chain = (
        "liquid" if asset in {"LBTC", "L-BTC"} or "liquid" in wallet_kind else "bitcoin"
    )
    try:
        chain = normalize_chain(config.get("chain") or fallback_chain)
    except ValueError:
        chain = fallback_chain
    try:
        network = normalize_network(chain, config.get("network"))
    except ValueError:
        network = "liquidv1" if chain == "liquid" else "main"
    return chain, network


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
    profile_rows = conn.execute(
        f"SELECT DISTINCT profile_id FROM transactions WHERE id IN ({placeholders})",
        ids,
    ).fetchall()
    if len(profile_rows) != 1:
        return {}
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?",
        (profile_rows[0]["profile_id"],),
    ).fetchone()
    if profile is None or _journal_freshness(conn, profile)["needs_processing"]:
        return {}
    pair_rows = conn.execute(
        f"""
        SELECT
            relation.id,
            relation.kind,
            relation.policy,
            relation.swap_fee_msat,
            relation.swap_fee_kind,
            relation.out_transaction_id,
            relation.in_transaction_id,
            relation.out_asset,
            relation.out_amount,
            tout.fiat_rate AS out_fiat_rate,
            relation.in_asset,
            relation.in_amount,
            tin.fiat_rate AS in_fiat_rate,
            wout.label AS out_wallet,
            win.label AS in_wallet,
            relation.occurred_at AS sort_at
        FROM journal_custody_projection_relations relation
        JOIN transactions tout ON tout.id = relation.out_transaction_id
        JOIN transactions tin ON tin.id = relation.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE relation.relation_kind IN ('move', 'conversion')
          AND (relation.out_transaction_id IN ({placeholders})
               OR relation.in_transaction_id IN ({placeholders}))
        ORDER BY sort_at, relation.id
        """,
        [*ids, *ids],
    ).fetchall()
    pair_meta: dict[str, dict[str, Any]] = {}
    for pair in pair_rows:
        out_asset = pair["out_asset"]
        in_asset = pair["in_asset"]
        pair_type = "transfer" if out_asset == in_asset else "swap"
        # Same-asset transfers store swap_fee_msat=NULL on purpose; do not
        # invent out-in as a fee (matches reports._pair_swap_fee_msat).
        fee_msat = int(pair["swap_fee_msat"] or 0)
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
        # First pair wins (rows are ordered by created_at, id) so a
        # multi-paired leg renders one deterministic representative.
        pair_meta.setdefault(pair["out_transaction_id"], {**base, "role": "out"})
        pair_meta.setdefault(pair["in_transaction_id"], {**base, "role": "in"})
    for transaction_id, meta in _journal_transfer_display_meta(conn, rows).items():
        pair_meta.setdefault(transaction_id, meta)
    return pair_meta


def _journal_transfer_display_meta(
    conn: sqlite3.Connection,
    rows: list[sqlite3.Row],
) -> dict[str, dict[str, Any]]:
    ids = [row["id"] for row in rows]
    if not ids:
        return {}
    placeholders = ", ".join("?" for _ in ids)
    transfer_rows = conn.execute(
        f"""
        SELECT
            jin.id AS in_entry_id,
            jout.id AS out_entry_id,
            jin.transaction_id AS in_transaction_id,
            jout.transaction_id AS out_transaction_id,
            jin.asset AS asset,
            ABS(jout.quantity) AS out_amount,
            ABS(jin.quantity) AS in_amount,
            tin.fiat_rate AS in_fiat_rate,
            tout.fiat_rate AS out_fiat_rate,
            win.label AS in_wallet,
            wout.label AS out_wallet
        FROM journal_entries jin
        JOIN journal_entries jout
          ON jout.profile_id = jin.profile_id
         AND jout.entry_type = 'transfer_out'
         AND jout.occurred_at = jin.occurred_at
         AND jout.asset = jin.asset
         AND jout.description = jin.description
         AND ABS(jout.quantity) = ABS(jin.quantity)
        JOIN wallets win ON win.id = jin.wallet_id
        JOIN wallets wout ON wout.id = jout.wallet_id
        LEFT JOIN transactions tin ON tin.id = jin.transaction_id
        LEFT JOIN transactions tout ON tout.id = jout.transaction_id
        WHERE jin.entry_type = 'transfer_in'
          AND jin.transaction_id IN ({placeholders})
        ORDER BY jin.occurred_at ASC, jin.id ASC
        """,
        ids,
    ).fetchall()
    pair_meta: dict[str, dict[str, Any]] = {}
    for pair in transfer_rows:
        in_transaction_id = str(pair["in_transaction_id"])
        if in_transaction_id in pair_meta:
            continue
        raw_display_rate = (
            pair["out_fiat_rate"]
            if pair["out_fiat_rate"] is not None
            else pair["in_fiat_rate"]
        )
        pair_meta[in_transaction_id] = {
            "pair_id": f"journal-transfer:{pair['out_entry_id']}:{pair['in_entry_id']}",
            "out_transaction_id": pair["out_transaction_id"],
            "in_transaction_id": pair["in_transaction_id"],
            "pair_type": "transfer",
            "kind": "journal-derived",
            "policy": "carrying-value",
            "label": "Transfer",
            "counter": f"Transfer {pair['out_wallet']} -> {pair['in_wallet']}",
            "account": f"{pair['out_wallet']} -> {pair['in_wallet']}",
            "fee_msat": 0,
            "fee_kind": None,
            "out_asset": pair["asset"],
            "out_amount_msat": int(pair["out_amount"] or 0),
            "out_wallet": pair["out_wallet"],
            "in_asset": pair["asset"],
            "in_amount_msat": int(pair["in_amount"] or 0),
            "in_wallet": pair["in_wallet"],
            "tag": "Transfer",
            "display_rate": _positive_float_or_none(raw_display_rate),
            "role": "in",
        }
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
        # Sync backends stamp provenance boilerplate into description; that is
        # not a counterparty. An empty counter means "none recorded"; each UI
        # surface picks its own fallback (short txid in tables, hidden in the
        # detail header).
        description = row["description"] or ""
        if description.startswith("Synced from ") or description.startswith(
            "Silent Payment sync from "
        ):
            description = ""
        counter = row["counterparty"] or description or row["note"] or ""
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
    chain, network = _transaction_row_chain_network(row)
    payload = {
        "id": row_id,
        "externalId": external_id,
        "explorerId": _public_explorer_id(external_id),
        "date": (occurred_at or "")[:16].replace("T", " "),
        "type": type_label,
        "asset": row["asset"] if "asset" in row_keys else None,
        "chain": chain,
        "network": network,
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


def _journal_balance_series(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    has_book_state: bool,
) -> list[float] | None:
    if not has_book_state:
        return None
    if _canonical_quantity_processed(conn, profile_id):
        rows = conn.execute(
            """
            SELECT SUBSTR(occurred_at, 1, 7) AS month,
                   SUM(amount_msat) AS amount_msat
            FROM journal_quantity_postings
            WHERE profile_id = ? AND location_kind = 'wallet'
              AND state = 'observed' AND asset IN ('BTC', 'LBTC')
              AND occurred_at IS NOT NULL
            GROUP BY SUBSTR(occurred_at, 1, 7)
            ORDER BY month
            """,
            (profile_id,),
        ).fetchall()
        return _balance_series_from_month_deltas(
            {
                str(row["month"]): msat_to_btc(row["amount_msat"] or 0)
                for row in rows
                if row["month"]
            }
        )
    rows = conn.execute(
        """
        SELECT occurred_at, entry_type, quantity
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    by_month: dict[str, Decimal] = defaultdict(lambda: Decimal("0"))
    for row in rows:
        key = (row["occurred_at"] or "")[:7]
        if not key:
            continue
        by_month[key] += _book_quantity_delta(row["entry_type"], row["quantity"])
    return _balance_series_from_month_deltas(by_month)


def _transaction_balance_series(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[float]:
    rows = conn.execute(
        """
        SELECT occurred_at, direction, amount, fee
        FROM transactions
        WHERE profile_id = ? AND excluded = 0 AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    by_month: dict[str, Decimal] = defaultdict(lambda: Decimal("0"))
    for row in rows:
        key = (row["occurred_at"] or "")[:7]
        if not key:
            continue
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        delta = amount if row["direction"] == "inbound" else -amount - fee
        by_month[key] += msat_to_btc(delta)
    return _balance_series_from_month_deltas(by_month)


def _balance_series(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    has_book_state: bool,
) -> list[float]:
    book_series = _journal_balance_series(
        conn,
        profile_id,
        has_book_state=has_book_state,
    )
    if book_series is not None:
        return book_series
    return _transaction_balance_series(conn, profile_id)


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
        SELECT
            COALESCE(t.occurred_at, je.occurred_at) AS occurred_at,
            je.entry_type,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ? AND je.asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile_id,),
    ).fetchall()
    cost_basis = 0.0
    by_date: dict[str, float] = {}
    for row in rows:
        date_key = (row["occurred_at"] or "")[:10]
        if not date_key:
            continue
        cost_basis += float(
            report_builders._holdings_basis_delta(
                row["entry_type"],
                msat_to_btc(row["quantity"]),
                dec(row["fiat_value"]),
                dec(row["cost_basis"]),
            )
        )
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
        WITH keyed AS (
            SELECT
                CASE
                    WHEN source = 'kraken-csv'
                         AND granularity = 'daily'
                         AND method = 'ohlcvt_csv'
                    THEN date(timestamp, '-1 day')
                    ELSE substr(timestamp, 1, 10)
                END AS rate_day,
                timestamp,
                rate,
                source,
                fetched_at,
                granularity,
                method
            FROM rates_cache
            WHERE pair = ?
              AND timestamp >= ?
        ),
        ranked AS (
            SELECT
                rate_day,
                timestamp,
                rate,
                source,
                fetched_at,
                granularity,
                method,
                ROW_NUMBER() OVER (
                    PARTITION BY rate_day
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
            FROM keyed
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
        SELECT transaction_id, entry_type, quantity, fiat_value, cost_basis
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
        cost_basis += float(
            report_builders._holdings_basis_delta(
                row["entry_type"],
                msat_to_btc(row["quantity"]),
                dec(row["fiat_value"]),
                dec(row["cost_basis"]),
            )
        )
        by_transaction[str(transaction_id)] = cost_basis
    return by_transaction


def _current_portfolio_cost_basis(
    conn: sqlite3.Connection,
    profile_id: str,
) -> float:
    rows = conn.execute(
        """
        SELECT entry_type, quantity, fiat_value, cost_basis
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()
    cost_basis = 0.0
    for row in rows:
        cost_basis += float(
            report_builders._holdings_basis_delta(
                row["entry_type"],
                msat_to_btc(row["quantity"]),
                dec(row["fiat_value"]),
                dec(row["cost_basis"]),
            )
        )
    return cost_basis


def _portfolio_series(
    conn: sqlite3.Connection,
    profile_id: str,
    fiat_currency: str,
    fallback_rate: float,
    final_balance_btc: float,
    final_value_eur: float,
    *,
    has_book_state: bool,
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
    raw_tx_days: set[date] = set()
    raw_rates_by_day: dict[date, float] = {}
    raw_tx_deltas_by_day: dict[date, Decimal] = defaultdict(lambda: Decimal("0"))
    for row in rows:
        day = _parse_day(row["occurred_at"])
        if day is None:
            continue
        raw_tx_days.add(day)
        amount = int(row["amount"] or 0)
        fee = int(row["fee"] or 0)
        raw_tx_deltas_by_day[day] += msat_to_btc(
            amount if row["direction"] == "inbound" else -amount - fee
        )
        row_rate = _rate_from_transaction(row)
        if row_rate is not None:
            raw_rates_by_day[day] = row_rate

    quantity_deltas_by_day = (
        _journal_quantity_deltas_by_day(conn, profile_id)
        if has_book_state
        else raw_tx_deltas_by_day
    )
    sorted_tx_days = sorted(set(quantity_deltas_by_day) | raw_tx_days)
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
        quantity_btc = Decimal("0")
        tx_index = 0
        cost_basis_index = 0
        day_cost_basis = 0.0
        output: list[dict[str, Any]] = []

        for rate_row in daily_rates:
            rate_day = _parse_day(rate_row["rate_day"])
            if rate_day is None:
                continue
            while tx_index < len(sorted_tx_days) and sorted_tx_days[tx_index] <= rate_day:
                quantity_btc += quantity_deltas_by_day.get(
                    sorted_tx_days[tx_index],
                    Decimal("0"),
                )
                tx_index += 1
            while (
                cost_basis_index < len(cost_basis_items)
                and cost_basis_items[cost_basis_index][0] <= rate_day
            ):
                day_cost_basis = cost_basis_items[cost_basis_index][1]
                cost_basis_index += 1

            balance_btc = float(quantity_btc)
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
                quantity_btc += quantity_deltas_by_day.get(
                    sorted_tx_days[tx_index],
                    Decimal("0"),
                )
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
                        else float(quantity_btc) * rate
                    ),
                    "costBasisEur": day_cost_basis,
                    "priceEur": rate,
                }
            )

        return output

    quantity_btc = Decimal("0")
    latest_rate = fallback_rate
    output: list[dict[str, Any]] = []
    day_cost_basis = 0.0

    for day in sorted_tx_days:
        date_key = day.isoformat()
        quantity_btc += quantity_deltas_by_day.get(day, Decimal("0"))
        if day in raw_rates_by_day:
            latest_rate = raw_rates_by_day[day]
        day_cost_basis = cost_basis_by_date.get(date_key, day_cost_basis)
        balance_btc = float(quantity_btc)
        is_final = day == sorted_tx_days[-1]
        output.append(
            {
                "date": date_key,
                "label": date_key,
                "balanceBtc": final_balance_btc if is_final else balance_btc,
                "valueEur": (
                    final_value_eur if is_final else balance_btc * latest_rate
                ),
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
    *,
    balance_total: float | None = None,
    chain_duplicate_adjustment_btc: float = 0.0,
) -> dict[str, Any]:
    market_balance = sum(balances.values()) if balance_total is None else balance_total
    market_value = market_balance * fiat_rate
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
    payload = {
        "fiatCurrency": str(fiat_currency or "EUR").upper(),
        "eurBalance": float(market_value),
        "eurCostBasis": cost_basis,
        "eurUnrealized": float(market_value - cost_basis),
        "eurRealizedYTD": float(realized_row["gain_loss"] or 0),
    }
    if chain_duplicate_adjustment_btc > 0:
        payload["chainDuplicateOutpointAdjustmentBtc"] = float(
            chain_duplicate_adjustment_btc
        )
    return payload


def _tax_free_balance_snapshot(
    conn: sqlite3.Connection,
    profile: sqlite3.Row,
    freshness: dict[str, Any],
) -> dict[str, Any] | None:
    if str(profile["tax_country"] or "").lower() != "at":
        return None
    rows = conn.execute(
        """
        SELECT
            wallet_id, entry_type, asset, occurred_at, quantity, fiat_value,
            cost_basis, description, at_category
        FROM journal_entries
        WHERE profile_id = ? AND asset IN ('BTC', 'LBTC')
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"],),
    ).fetchall()
    entries: list[dict[str, Any]] = []
    for row in rows:
        entries.append(
            {
                "wallet_id": row["wallet_id"],
                "entry_type": row["entry_type"],
                "asset": row["asset"],
                "occurred_at": row["occurred_at"],
                "quantity": msat_to_btc(row["quantity"]) or Decimal("0"),
                "fiat_value": dec(row["fiat_value"]),
                "cost_basis": (
                    dec(row["cost_basis"])
                    if row["cost_basis"] is not None
                    else None
                ),
                "description": row["description"],
                "at_category": row["at_category"],
            }
        )
    today = datetime.now(timezone.utc).date().isoformat()
    report = report_builders.compute_deemed_disposal(
        conn,
        dict(profile),
        {
            "entries": entries,
            "wallet_holdings": {},
            "account_holdings": {},
            "quarantines": [],
            "latest_rates": {},
        },
        departure_date=today,
        destination="eu_eea",
    )
    totals = report["totals"]
    tax_free_sats = int(totals["altQuantitySats"] or 0)
    taxable_sats = int(totals["neuQuantitySats"] or 0)
    total_sats = tax_free_sats + taxable_sats
    needs_journals = bool(freshness.get("needs_processing"))
    quarantines = int(freshness.get("quarantine_count") or 0)
    status = (
        "needs_journals"
        if needs_journals
        else "quarantines"
        if quarantines
        else "current"
    )
    return {
        "rule": "austrian_altbestand",
        "jurisdictionCode": report["jurisdictionCode"],
        "fiatCurrency": report["fiatCurrency"],
        "status": status,
        "taxFreeQuantitySats": tax_free_sats,
        "taxableQuantitySats": taxable_sats,
        "totalQuantitySats": total_sats,
        "taxFreeMarketValue": totals["altMarketValue"],
        "taxableMarketValue": totals["neuMarketValue"],
        "needsJournals": needs_journals,
        "quarantines": quarantines,
        "wallets": _tax_free_wallet_summaries(entries),
        "buckets": [
            {
                "id": "altbestand",
                "regime": "alt",
                "label": "Altbestand",
                "quantitySats": tax_free_sats,
                "marketValue": totals["altMarketValue"],
                "taxFree": True,
            },
            {
                "id": "neubestand",
                "regime": "neu",
                "label": "Neubestand",
                "quantitySats": taxable_sats,
                "marketValue": totals["neuMarketValue"],
                "taxFree": False,
            },
        ],
    }


def _tax_free_wallet_summaries(entries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    tax_free_by_wallet: dict[str, Decimal] = defaultdict(Decimal)
    for entry in entries:
        entry_type = str(entry.get("entry_type") or "")
        if entry_type == "transfer_fee":
            continue
        wallet_id = str(entry.get("wallet_id") or "")
        if not wallet_id:
            continue
        qty = dec(entry.get("quantity") or 0)
        if qty == 0:
            continue
        # Transfers carry a per-regime quantity split (at_alt_out/at_alt_in):
        # a mixed-regime MOVE carries tax-free lots even when its fee-slice
        # at_regime marker says "neu", so classify the QUANTITIES when the
        # split is available and fall back to the whole-entry regime otherwise.
        if entry_type in ("transfer_out", "transfer_in"):
            alt_msat = _entry_alt_flow_msat(
                entry, MARKER_ALT_OUT if entry_type == "transfer_out" else MARKER_ALT_IN
            )
            if alt_msat is not None:
                alt_qty = dec(msat_to_btc(alt_msat))
                if entry_type == "transfer_out":
                    tax_free_by_wallet[wallet_id] -= alt_qty
                else:
                    tax_free_by_wallet[wallet_id] += alt_qty
                continue
        if _entry_has_alt_regime(entry):
            tax_free_by_wallet[wallet_id] += qty
    return [
        {
            "walletId": wallet_id,
            "hasTaxFreeBalance": qty > 0,
        }
        for wallet_id, qty in sorted(tax_free_by_wallet.items())
    ]


def _entry_alt_flow_msat(entry: dict[str, Any], marker: str) -> int | None:
    return parse_marker_int(entry.get("description"), marker)


def _entry_has_alt_regime(entry: dict[str, Any]) -> bool:
    marker = _entry_at_regime_marker(entry)
    if marker == "alt":
        return True
    if marker == "neu":
        return False
    category = entry.get("at_category")
    if category:
        return str(category).startswith("alt")
    occurred_at = entry.get("occurred_at")
    if not occurred_at:
        return False
    try:
        from .austrian import infer_regime_from_timestamp

        return infer_regime_from_timestamp(str(occurred_at)) == "alt"
    except ValueError:
        return False


def _entry_at_regime_marker(entry: dict[str, Any]) -> str | None:
    marker = parse_marker(entry.get("description"), MARKER_REGIME)
    return marker if marker in ("alt", "neu") else None


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
    # User-facing wallet balances prefer the chain-backed coin inventory when a
    # wallet has observed inventory. Book/report balances remain the accounting
    # fallback for sources without chain inventory and are carried alongside the
    # display value for reconciliation.
    has_book_state = _has_book_balance_state(freshness)
    book_balances = _wallet_balances(
        conn,
        profile["id"],
        has_book_state=has_book_state,
    )
    display_balance_info = _display_wallet_balances(
        conn,
        profile["id"],
        book_balances,
        fallback_source="books" if has_book_state else "transactions",
    )
    display_balances = {
        wallet_id: float(info.get("balance") or 0.0)
        for wallet_id, info in display_balance_info.items()
    }
    chain_duplicate_adjustment = _chain_duplicate_outpoint_adjustment_btc(
        conn,
        profile["id"],
        display_balance_info,
    )
    display_balance_total = max(
        0.0,
        sum(display_balances.values()) - chain_duplicate_adjustment,
    )
    balance_source_counts = {
        source: sum(
            1
            for info in display_balance_info.values()
            if info.get("balanceSource") == source
        )
        for source in ("chain", "books", "transactions")
    }
    active_balance_sources = [
        source for source, count in balance_source_counts.items() if count > 0
    ]
    if len(active_balance_sources) == 1:
        balance_source = active_balance_sources[0]
    elif active_balance_sources:
        balance_source = "mixed"
    else:
        balance_source = "books" if has_book_state else "transactions"
    if freshness["needs_processing"]:
        balance_status = "needs_journals"
    elif freshness["quarantine_count"]:
        balance_status = "quarantines"
    else:
        balance_status = "current"
    fiat = _fiat_snapshot(
        conn,
        profile["id"],
        profile["fiat_currency"],
        book_fiat_rate,
        display_balances,
        balance_total=display_balance_total,
        chain_duplicate_adjustment_btc=chain_duplicate_adjustment,
    )
    snapshot = {
        "priceEur": price_eur,
        "priceUsd": price_usd,
        "marketRate": market_rate,
        "connections": _connections(conn, profile["id"], display_balance_info),
        "txs": _transactions(conn, profile["id"]),
        "activityTxs": _activity_transactions(
            conn,
            profile["id"],
            has_book_state=has_book_state,
        ),
        "balanceSeries": _balance_series(
            conn,
            profile["id"],
            has_book_state=has_book_state,
        ),
        "portfolioSeries": _portfolio_series(
            conn,
            profile["id"],
            profile["fiat_currency"],
            book_fiat_rate,
            display_balance_total,
            fiat["eurBalance"],
            has_book_state=has_book_state,
        ),
        "fiat": fiat,
        "balanceSummary": {
            "totalBtc": display_balance_total,
            "status": balance_status,
            "source": balance_source,
            "needsJournals": freshness["needs_processing"],
            "quarantines": freshness["quarantine_count"],
            "chainWalletCount": balance_source_counts["chain"],
            "bookWalletCount": balance_source_counts["books"],
            "transactionWalletCount": balance_source_counts["transactions"],
            "duplicateOutpointAdjustmentBtc": chain_duplicate_adjustment,
        },
        "taxFreeBalance": _tax_free_balance_snapshot(conn, profile, freshness),
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
            "txids",
            "direction",
            "asset",
            "wallet",
            "since",
            "start",
            "until",
            "end",
            "period",
            "status",
            "flow",
            "payment_method",
            "paymentMethod",
            "network",
            "with_fees",
            "withFees",
            "quick",
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
    query = _coerce_optional_string_arg(raw_args, "query")
    if query is None and require_query:
        raise AppError(
            f"{kind} query must be a non-empty string",
            code="validation",
            retryable=False,
        )

    limit = _coerce_limit(raw_args, default=default_limit, maximum=maximum_limit)
    profile = conn.execute(
        "SELECT * FROM profiles WHERE id = ?", (context["profile_id"],)
    ).fetchone()
    projection_current = bool(
        profile is not None
        and not _journal_freshness(conn, profile)["needs_processing"]
    )

    def pair_exists_sql(pair_type: str | None = None) -> str:
        return _ui_transaction_pair_exists_sql(pair_type) if projection_current else "0"

    filters = ["t.profile_id = ?"]
    params: list[Any] = [context["profile_id"]]
    direction = _coerce_optional_string_arg(raw_args, "direction")
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
    asset = _coerce_optional_string_arg(raw_args, "asset")
    asset_filter = None
    if asset is not None:
        asset_filter = asset.strip().upper()
        filters.append("upper(t.asset) = ?")
        params.append(asset_filter)
    wallet = _coerce_optional_string_arg(raw_args, "wallet")
    wallet_filter = None
    if wallet is not None:
        wallet_filter = wallet.strip()
        filters.append("(t.wallet_id = ? OR lower(w.label) = lower(?))")
        params.extend([wallet_filter, wallet_filter])
    period = _coerce_optional_string_arg(raw_args, "period")
    period_filter = _coerce_ui_transaction_period(period) if period else None
    since = _coerce_optional_string_arg(raw_args, "since")
    if since is None:
        since = _coerce_optional_string_arg(raw_args, "start")
    if since is None and period_filter:
        since = _ui_transaction_since_for_period(period_filter)
    since_filter = None
    if since is not None:
        since_filter = _iso_z(_parse_iso_datetime(since, "since"))
        filters.append("t.occurred_at >= ?")
        params.append(since_filter)
    until = _coerce_optional_string_arg(raw_args, "until")
    if until is None:
        until = _coerce_optional_string_arg(raw_args, "end")
    until_filter = None
    if until is not None:
        until_filter = _iso_z(_parse_iso_datetime(until, "until"))
        filters.append("t.occurred_at <= ?")
        params.append(until_filter)
    txids_filter = _coerce_optional_string_list_arg(raw_args, "txids")
    if txids_filter:
        txid_terms = [value.lower() for value in txids_filter]
        placeholders = ", ".join("?" for _ in txid_terms)
        filters.append(
            f"""(
              lower(t.id) IN ({placeholders})
              OR lower(COALESCE(t.external_id, '')) IN ({placeholders})
              OR (
                length(COALESCE(t.external_id, '')) = 64
                AND lower(COALESCE(t.external_id, '')) IN ({placeholders})
              )
            )"""
        )
        params.extend([*txid_terms, *txid_terms, *txid_terms])
    status_filter_raw = _coerce_optional_string_arg(raw_args, "status")
    status_filter = _normalize_ui_transaction_status(status_filter_raw) if status_filter_raw else None
    if status_filter:
        filters.append(f"({_ui_transaction_status_sql()}) = ?")
        params.append(status_filter)
    flow_filter_raw = _coerce_optional_string_arg(raw_args, "flow")
    flow_filter = _normalize_ui_transaction_flow(flow_filter_raw) if flow_filter_raw else None
    if flow_filter == "incoming":
        filters.append(
            f"""(
              t.direction = 'inbound'
              AND lower(COALESCE(t.kind, '')) NOT IN ({", ".join("?" for _ in _UI_TRANSACTION_FLOW_KINDS | {"transfer"})})
              AND NOT {pair_exists_sql()}
            )"""
        )
        params.extend(sorted(_UI_TRANSACTION_FLOW_KINDS | {"transfer"}))
    elif flow_filter == "outgoing":
        filters.append(
            f"""(
              t.direction = 'outbound'
              AND lower(COALESCE(t.kind, '')) NOT IN ({", ".join("?" for _ in _UI_TRANSACTION_FLOW_KINDS | {"transfer"})})
              AND NOT {pair_exists_sql()}
            )"""
        )
        params.extend(sorted(_UI_TRANSACTION_FLOW_KINDS | {"transfer"}))
    elif flow_filter == "transfer":
        filters.append(
            f"(lower(COALESCE(t.kind, '')) = 'transfer' OR {pair_exists_sql('transfer')})"
        )
    elif flow_filter == "swap":
        filters.append(
            f"""(
              lower(COALESCE(t.kind, '')) IN ({", ".join("?" for _ in _UI_TRANSACTION_FLOW_KINDS)})
              OR {pair_exists_sql('swap')}
            )"""
        )
        params.extend(sorted(_UI_TRANSACTION_FLOW_KINDS))
    elif flow_filter == "layer-transition":
        filters.append(
            f"lower(COALESCE(t.kind, '')) IN ({', '.join('?' for _ in _UI_TRANSACTION_LAYER_TRANSITION_KINDS)})"
        )
        params.extend(sorted(_UI_TRANSACTION_LAYER_TRANSITION_KINDS))
    payment_raw = _coerce_optional_string_arg(raw_args, "payment_method")
    if payment_raw is None:
        payment_raw = _coerce_optional_string_arg(raw_args, "paymentMethod")
    payment_filter = _normalize_ui_transaction_payment_method(payment_raw) if payment_raw else None
    if payment_filter:
        filters.append(f"({_ui_transaction_payment_method_sql()}) = ?")
        params.append(payment_filter)
    network_filter = _coerce_optional_string_arg(raw_args, "network")
    if network_filter:
        normalized_network = network_filter.lower()
        maybe_payment = _UI_TRANSACTION_PAYMENT_METHODS.get(normalized_network)
        if maybe_payment:
            filters.append(f"({_ui_transaction_payment_method_sql()}) = ?")
            params.append(maybe_payment)
        else:
            filters.append(
                """(
                  lower(w.kind) = ?
                  OR lower(w.config_json) LIKE ?
                  OR lower(w.label) LIKE ?
                  OR upper(t.asset) = ?
                )"""
            )
            params.extend(
                [
                    normalized_network,
                    f"%{normalized_network}%",
                    f"%{normalized_network}%",
                    "LBTC" if normalized_network == "liquid" else normalized_network.upper(),
                ]
            )
    with_fees = _coerce_optional_bool_arg(raw_args, "with_fees", "withFees")
    if with_fees:
        filters.append("COALESCE(t.fee, 0) <> 0")
    quick_raw = _coerce_optional_string_arg(raw_args, "quick")
    quick_filter = _normalize_ui_transaction_quick_filter(quick_raw) if quick_raw else None
    if quick_filter == "external_flow":
        filters.append("t.direction IN ('inbound', 'outbound')")
        filters.append("lower(COALESCE(t.kind, '')) <> 'transfer'")
    elif quick_filter == "review_queue":
        filters.append(f"({_ui_transaction_status_sql()}) <> 'completed'")
    elif quick_filter == "no_explorer_id":
        filters.append(
            """(
              t.external_id IS NULL
              OR length(trim(t.external_id)) <> 64
              OR lower(trim(t.external_id)) GLOB '*[^0-9a-f]*'
            )"""
        )
    elif quick_filter == "missing_price":
        filters.append(
            """(
              t.fiat_rate IS NULL
              OR t.fiat_rate <= 0
              OR lower(COALESCE(t.pricing_quality, '')) = 'missing'
              OR lower(COALESCE(t.pricing_source_kind, '')) = 'missing'
            )"""
        )
    elif quick_filter == "failed_import":
        filters.append(f"({_ui_transaction_status_sql()}) = 'failed'")
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
        txids=txids_filter,
        status=status_filter,
        flow=flow_filter,
        payment_method=payment_filter,
        network=network_filter,
        with_fees=with_fees,
        quick=quick_filter,
    )
    base_filters = list(filters)
    base_params = list(params)
    total_row = conn.execute(
        f"""
        SELECT COUNT(DISTINCT {_ui_transaction_custody_group_sql(projection_current=projection_current)}) AS count
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN journal_quarantines jq ON jq.transaction_id = t.id
        WHERE {' AND '.join(base_filters)}
        """,
        base_params,
    ).fetchone()
    filtered_count = int(total_row["count"] or 0) if total_row else 0
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
            w.kind AS wallet_kind,
            w.config_json AS wallet_config_json,
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
            "period": period_filter,
            "direction": direction,
            "asset": asset_filter,
            "wallet": wallet_filter,
            "since": since_filter,
            "until": until_filter,
            "txids": txids_filter,
            "status": status_filter,
            "flow": flow_filter,
            "paymentMethod": payment_filter,
            "network": network_filter,
            "withFees": with_fees,
            "quick": quick_filter,
            "sort": sort,
            "order": order,
        },
        "count": filtered_count,
        "total": filtered_count,
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
            w.kind AS wallet_kind,
            w.config_json AS wallet_config_json,
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
    use_vienna_year: bool = False,
) -> list[int]:
    reportable_filter = (
        "((je.entry_type = 'disposal' AND COALESCE(je.at_category, '') != 'neu_swap') "
        "OR (je.entry_type NOT IN ('fee', 'transfer_fee') AND je.at_kennzahl IS NOT NULL))"
        if primary_only
        else "((je.entry_type IN ('disposal', 'income') "
        "AND COALESCE(je.at_category, '') != 'neu_swap') "
        "OR (je.entry_type NOT IN ('fee', 'transfer_fee', 'disposal') "
        "AND je.at_kennzahl IS NOT NULL))"
    )
    rows = conn.execute(
        f"""
        SELECT je.occurred_at
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
          AND COALESCE(t.taxability_override, 1) != 0
          AND {reportable_filter}
          AND je.occurred_at IS NOT NULL
          AND length(je.occurred_at) >= 4
        """,
        (profile_id,),
    ).fetchall()
    years: set[int] = set()
    if use_vienna_year:
        from .austrian import tax_year_in_vienna

        for row in rows:
            years.add(tax_year_in_vienna(str(row["occurred_at"])))
    else:
        for row in rows:
            year = str(row["occurred_at"] or "")[:4]
            if year.isdigit():
                years.add(int(year))
    return sorted(years, reverse=True)


def _capital_gains_transaction_years(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    use_vienna_year: bool = False,
) -> list[int]:
    rows = conn.execute(
        """
        SELECT occurred_at
        FROM transactions
        WHERE profile_id = ?
          AND excluded = 0
          AND occurred_at IS NOT NULL
          AND length(occurred_at) >= 4
        """,
        (profile_id,),
    ).fetchall()
    years: set[int] = set()
    if use_vienna_year:
        from .austrian import tax_year_in_vienna

        for row in rows:
            years.add(tax_year_in_vienna(str(row["occurred_at"])))
    else:
        for row in rows:
            year = str(row["occurred_at"] or "")[:4]
            if year.isdigit():
                years.add(int(year))
    return sorted(years, reverse=True)


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
    *,
    use_vienna_year: bool = False,
) -> list[dict[str, Any]]:
    where = [
        "je.profile_id = ?",
        "je.entry_type = 'disposal'",
        "je.at_category = 'neu_swap'",
        "COALESCE(tout.taxability_override, 1) != 0",
    ]
    params: list[Any] = [profile_id]
    if use_vienna_year:
        from .austrian import tax_year_in_vienna, vienna_tax_year_utc_window

        start, end = vienna_tax_year_utc_window(tax_year)
        where.append("je.occurred_at >= ?")
        where.append("je.occurred_at < ?")
        params.extend([start, end])
    else:
        where.append("substr(je.occurred_at, 1, 4) = ?")
        params.append(str(tax_year))
    rows = conn.execute(
        f"""
        SELECT
            je.occurred_at,
            je.quantity,
            je.cost_basis,
            je.proceeds,
            je.gain_loss,
            COALESCE(relation.id, '') AS pair_id,
            COALESCE(relation.kind, '') AS kind,
            COALESCE(relation.policy, '') AS policy,
            COALESCE(relation.swap_fee_msat, 0) AS swap_fee_msat,
            COALESCE(relation.swap_fee_kind, '') AS swap_fee_kind,
            wout.label AS out_wallet,
            relation.out_asset,
            -- Split cross-asset swaps cross only `out_amount`; keep outSats
            -- consistent with feeSats (swap_fee_msat) on neu_swap detail rows.
            COALESCE(relation.out_amount, tout.amount) AS out_amount,
            win.label AS in_wallet,
            relation.in_asset,
            relation.in_amount
        FROM journal_entries je
        LEFT JOIN journal_custody_projection_relations relation
          ON relation.out_transaction_id = je.transaction_id
         AND relation.profile_id = je.profile_id
         AND relation.relation_kind = 'conversion'
        LEFT JOIN transactions tout ON tout.id = relation.out_transaction_id
        LEFT JOIN transactions tin ON tin.id = relation.in_transaction_id
        LEFT JOIN wallets wout ON wout.id = tout.wallet_id
        LEFT JOIN wallets win ON win.id = tin.wallet_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        params,
    ).fetchall()
    if use_vienna_year:
        rows = [
            row
            for row in rows
            if tax_year_in_vienna(str(row["occurred_at"])) == int(tax_year)
        ]
    output = []
    for row in rows:
        quantity_msat = abs(int(row["quantity"] or 0))
        out_amount_msat = abs(int(row["out_amount"] or 0)) or quantity_msat
        in_amount_msat = abs(int(row["in_amount"] or 0))
        fee_msat = int(row["swap_fee_msat"] or 0)
        output.append(
            {
                "date": (
                    vienna_local_date(str(row["occurred_at"]))
                    if use_vienna_year
                    else (row["occurred_at"] or "")[:10]
                ),
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
    use_vienna_year = str(profile["tax_country"] or "").lower() == "at"
    available_years = _merge_report_years(
        _capital_gains_available_years(
            conn, profile["id"], use_vienna_year=use_vienna_year
        ),
        _capital_gains_transaction_years(
            conn, profile["id"], use_vienna_year=use_vienna_year
        ),
    )
    primary_years = _capital_gains_available_years(
        conn,
        profile["id"],
        primary_only=True,
        use_vienna_year=use_vienna_year,
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
    where = [
        "je.profile_id = ?",
        "je.entry_type IN ('disposal', 'income')",
        "COALESCE(t.taxability_override, 1) != 0",
        "COALESCE(je.at_category, '') != 'neu_swap'",
    ]
    params: list[Any] = [profile["id"]]
    if use_vienna_year:
        from .austrian import tax_year_in_vienna, vienna_tax_year_utc_window

        start, end = vienna_tax_year_utc_window(latest_year)
        where.append("je.occurred_at >= ?")
        where.append("je.occurred_at < ?")
        params.extend([start, end])
    else:
        where.append("substr(je.occurred_at, 1, 4) = ?")
        params.append(str(latest_year))
    # Fetch without a tight LIMIT first when using the loose Vienna UTC window,
    # then filter to the exact local year and keep the newest 200 lots.
    rows = conn.execute(
        f"""
        SELECT je.occurred_at, je.entry_type, je.quantity, je.cost_basis,
               je.proceeds, je.gain_loss, je.capital_gains_type
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        """,
        params,
    ).fetchall()
    if use_vienna_year:
        rows = [
            row
            for row in rows
            if tax_year_in_vienna(str(row["occurred_at"])) == int(latest_year)
        ]
    rows = rows[:200]
    lots = [
        {
            "acquired": "",
            "disposed": (
                vienna_local_date(str(row["occurred_at"]))
                if use_vienna_year
                else (row["occurred_at"] or "")[:10]
            ),
            "sats": int(round(abs(float(msat_to_btc(row["quantity"] or 0))) * 100_000_000)),
            "costEur": float(row["cost_basis"] or 0),
            "proceedsEur": float(
                row["proceeds"]
                if row["entry_type"] != "income"
                else (row["gain_loss"] or row["proceeds"] or 0)
            ),
            "type": (
                "LT"
                if str(row["capital_gains_type"] or "").lower() == "long"
                else "ST"
            ),
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
            use_vienna_year=use_vienna_year,
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


# A leg can carry several projected custody relations (fan-out, consolidation,
# or a split conversion). Journal rows show one deterministic representative
# relation as display metadata only; the stored decision/relation projection,
# never authored compatibility rows, is the source.
_JOURNAL_CUSTODY_RELATION_SQL = """
                    SELECT profile_id, id, kind, policy, swap_fee_msat,
                           swap_fee_kind, out_transaction_id,
                           in_transaction_id, out_amount, in_amount, created_at
                    FROM journal_custody_projection_relations
                    WHERE relation_kind IN ('move', 'conversion')
                      AND in_transaction_id IS NOT NULL
"""

_JOURNAL_PAIR_JOIN_SQL = f"""
            LEFT JOIN (
                SELECT * FROM (
                    SELECT relation.*,
                           ROW_NUMBER() OVER (
                               PARTITION BY profile_id, out_transaction_id
                               ORDER BY created_at, id
                           ) AS relation_rank
                    FROM ({_JOURNAL_CUSTODY_RELATION_SQL}) relation
                )
                WHERE relation_rank = 1
            ) p_out
              ON p_out.profile_id = je.profile_id
             AND p_out.out_transaction_id = je.transaction_id
            LEFT JOIN (
                SELECT * FROM (
                    SELECT relation.*,
                           ROW_NUMBER() OVER (
                               PARTITION BY profile_id, in_transaction_id
                               ORDER BY created_at, id
                           ) AS relation_rank
                    FROM ({_JOURNAL_CUSTODY_RELATION_SQL}) relation
                )
                WHERE relation_rank = 1
            ) p_in
              ON p_in.profile_id = je.profile_id
             AND p_in.in_transaction_id = je.transaction_id
"""

_JOURNAL_EMPTY_PAIR_JOIN_SQL = """
            LEFT JOIN (
                SELECT NULL AS id, NULL AS kind, NULL AS policy,
                       NULL AS swap_fee_msat, NULL AS out_transaction_id,
                       NULL AS in_transaction_id, NULL AS out_amount
                WHERE 0
            ) p_out ON 0
            LEFT JOIN (
                SELECT NULL AS id, NULL AS kind, NULL AS policy,
                       NULL AS swap_fee_msat, NULL AS out_transaction_id,
                       NULL AS in_transaction_id, NULL AS out_amount
                WHERE 0
            ) p_in ON 0
"""

_JOURNAL_PAIR_ID_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.id ELSE p_in.id END"
)
_JOURNAL_PAIR_KIND_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.kind ELSE p_in.kind END"
)
_JOURNAL_PAIR_POLICY_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.policy ELSE p_in.policy END"
)
_JOURNAL_PAIR_SWAP_FEE_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.swap_fee_msat "
    "ELSE p_in.swap_fee_msat END"
)
_JOURNAL_PAIR_OUT_TRANSACTION_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.out_transaction_id "
    "ELSE p_in.out_transaction_id END"
)
_JOURNAL_PAIR_OUT_AMOUNT_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.out_amount ELSE p_in.out_amount END"
)
_JOURNAL_PAIR_IN_TRANSACTION_SQL = (
    "CASE WHEN p_out.id IS NOT NULL THEN p_out.in_transaction_id "
    "ELSE p_in.in_transaction_id END"
)


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
    pair_join_sql = (
        _JOURNAL_EMPTY_PAIR_JOIN_SQL
        if freshness["needs_processing"]
        else _JOURNAL_PAIR_JOIN_SQL
    )
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
                {_JOURNAL_PAIR_ID_SQL} AS pair_id,
                {_JOURNAL_PAIR_KIND_SQL} AS pair_kind,
                {_JOURNAL_PAIR_POLICY_SQL} AS pair_policy,
                COALESCE({_JOURNAL_PAIR_SWAP_FEE_SQL}, 0) AS pair_swap_fee_msat,
                {_JOURNAL_PAIR_OUT_TRANSACTION_SQL} AS pair_out_transaction_id,
                tout.external_id AS pair_out_external_id,
                wout.label AS pair_out_wallet,
                tout.asset AS pair_out_asset,
                COALESCE({_JOURNAL_PAIR_OUT_AMOUNT_SQL}, tout.amount) AS pair_out_amount,
                {_JOURNAL_PAIR_IN_TRANSACTION_SQL} AS pair_in_transaction_id,
                tin.external_id AS pair_in_external_id,
                win.label AS pair_in_wallet,
                tin.asset AS pair_in_asset,
                tin.amount AS pair_in_amount
            FROM journal_entries je
            JOIN wallets w ON w.id = je.wallet_id
            LEFT JOIN transactions t ON t.id = je.transaction_id
            {pair_join_sql}
            LEFT JOIN transactions tout ON tout.id = {_JOURNAL_PAIR_OUT_TRANSACTION_SQL}
            LEFT JOIN transactions tin ON tin.id = {_JOURNAL_PAIR_IN_TRANSACTION_SQL}
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
                    {_JOURNAL_PAIR_ID_SQL} AS pair_id,
                    {_JOURNAL_PAIR_KIND_SQL} AS pair_kind,
                    {_JOURNAL_PAIR_POLICY_SQL} AS pair_policy,
                    COALESCE({_JOURNAL_PAIR_SWAP_FEE_SQL}, 0) AS pair_swap_fee_msat,
                    {_JOURNAL_PAIR_OUT_TRANSACTION_SQL} AS pair_out_transaction_id,
                    tout.external_id AS pair_out_external_id,
                    wout.label AS pair_out_wallet,
                    tout.asset AS pair_out_asset,
                    COALESCE({_JOURNAL_PAIR_OUT_AMOUNT_SQL}, tout.amount) AS pair_out_amount,
                    {_JOURNAL_PAIR_IN_TRANSACTION_SQL} AS pair_in_transaction_id,
                    tin.external_id AS pair_in_external_id,
                    win.label AS pair_in_wallet,
                    tin.asset AS pair_in_asset,
                    tin.amount AS pair_in_amount
                FROM journal_entries je
                JOIN wallets w ON w.id = je.wallet_id
                LEFT JOIN transactions t ON t.id = je.transaction_id
                {pair_join_sql}
                LEFT JOIN transactions tout ON tout.id = {_JOURNAL_PAIR_OUT_TRANSACTION_SQL}
                LEFT JOIN transactions tin ON tin.id = {_JOURNAL_PAIR_IN_TRANSACTION_SQL}
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


def build_custody_lineage_snapshot(
    conn: sqlite3.Connection,
    args: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Return canonical internal-custody edges for the active local book.

    The persisted reader deliberately omits observation commitments and exact
    quantity-slice offsets.  This snapshot keeps custody finality distinct from
    tax-basis eligibility: an exact wallet-to-wallet edge remains visible even
    when an earlier custody gap prevents later tax projection.
    """

    raw_args = _coerce_args(args)
    unknown = sorted(set(raw_args) - {"cursor", "limit", "transaction_id"})
    if unknown:
        raise AppError(
            "ui.custody.lineage.snapshot received unsupported arguments",
            code="validation",
            details={"unknown": unknown},
            retryable=False,
        )
    limit = _coerce_limit(raw_args, default=100, maximum=500)
    cursor = raw_args.get("cursor")
    if cursor is not None and (not isinstance(cursor, str) or not cursor):
        raise AppError(
            "ui.custody.lineage.snapshot cursor must be a non-empty string",
            code="validation",
            retryable=False,
        )
    transaction_id = raw_args.get("transaction_id")
    if transaction_id is not None:
        if not isinstance(transaction_id, str) or not transaction_id.strip():
            raise AppError(
                "ui.custody.lineage.snapshot transaction_id must be a non-empty string",
                code="validation",
                retryable=False,
            )
        transaction_id = transaction_id.strip()

    context, profile = _active_context_and_profile(conn)
    qualification = (
        "Derived locally from canonical evidence in the active project and selected "
        "book. Custody finality is separate from tax-basis eligibility: a verified "
        "custody edge may remain basis-blocked by an earlier unresolved gap. State "
        "counters describe the returned rows when the result is truncated."
    )
    scope = {
        "workspace_id": context["workspace_id"] or None,
        "workspace_label": context["workspace_label"] or None,
        "profile_id": context["profile_id"] or None,
        "profile_label": context["profile_label"] or None,
    }
    if profile is None:
        return {
            "scope": scope,
            "items": [],
            "next_cursor": None,
            "summary": {
                "total_count": 0,
                "returned_count": 0,
                "truncated": False,
                "internal_verified": 0,
                "internal_reviewed": 0,
                "basis_eligible": 0,
                "basis_blocked": 0,
                "qualification": qualification,
            },
            "observation_commitments_included": False,
            "replicated": False,
        }

    result = core_custody_quantity_store.custody_decision_rows(
        conn,
        str(profile["id"]),
        limit=limit,
        transaction_ids=[transaction_id] if transaction_id is not None else None,
        cursor=cursor,
    )
    items = []
    custody_counts: dict[str, int] = defaultdict(int)
    basis_counts: dict[str, int] = defaultdict(int)
    for raw_record in result.get("records", []):
        record = dict(raw_record)
        custody_state = str(record.get("custody_state") or "unknown")
        basis_state = str(record.get("basis_state") or "unknown")
        custody_counts[custody_state] += 1
        basis_counts[basis_state] += 1
        asset = str(record.get("source_asset") or record.get("target_asset") or "")
        source_network = str(record.get("source_network") or "unknown")
        target_network = str(record.get("target_network") or "unknown")
        source_rail = str(record.get("source_rail") or "unknown")
        target_rail = str(record.get("target_rail") or "unknown")
        items.append(
            {
                "out_transaction_id": record.get("source_transaction_id"),
                "in_transaction_id": record.get("target_transaction_id"),
                "occurred_at": record.get("occurred_at"),
                "asset": asset,
                "source_asset": record.get("source_asset"),
                "target_asset": record.get("target_asset"),
                "amount_msat": str(int(record.get("amount_msat") or 0)),
                "from_wallet_id": record.get("source_wallet_id"),
                "from_wallet_label": record.get("source_wallet_label"),
                "to_wallet_id": record.get("target_wallet_id"),
                "to_wallet_label": record.get("target_wallet_label"),
                "custody_state": custody_state,
                "basis_state": basis_state,
                "basis_barrier_at": record.get("basis_barrier_at"),
                "evidence_reason": record.get("reason"),
                "network": (
                    source_network
                    if source_network == target_network
                    else f"{source_network}->{target_network}"
                ),
                "source_network": source_network,
                "target_network": target_network,
                "rail": (
                    source_rail
                    if source_rail == target_rail
                    else f"{source_rail}->{target_rail}"
                ),
                "source_rail": source_rail,
                "target_rail": target_rail,
                "atomic_bundle_id": record.get("atomic_group_id"),
                "component_id": record.get("component_id"),
            }
        )

    return {
        "scope": scope,
        "items": items,
        "next_cursor": result.get("next_cursor"),
        "summary": {
            "total_count": int(result.get("count") or 0),
            "returned_count": int(result.get("returned", len(items)) or 0),
            "truncated": bool(result.get("truncated")),
            "internal_verified": custody_counts.get("internal_verified", 0),
            "internal_reviewed": custody_counts.get("internal_reviewed", 0),
            "basis_eligible": basis_counts.get("eligible", 0),
            "basis_blocked": basis_counts.get(
                "blocked_by_prior_custody_basis", 0
            ),
            "qualification": qualification,
        },
        "observation_commitments_included": bool(
            result.get("observation_commitments_included", False)
        ),
        "replicated": bool(result.get("replicated", False)),
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
    pair_join_sql = (
        _JOURNAL_EMPTY_PAIR_JOIN_SQL
        if freshness["needs_processing"]
        else _JOURNAL_PAIR_JOIN_SQL
    )
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
            OR je.entry_type = 'income'
            OR (je.entry_type NOT IN ('fee', 'transfer_fee') AND je.at_kennzahl IS NOT NULL)
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
            {_JOURNAL_PAIR_ID_SQL} AS pair_id,
            {_JOURNAL_PAIR_KIND_SQL} AS pair_kind,
            {_JOURNAL_PAIR_POLICY_SQL} AS pair_policy,
            COALESCE({_JOURNAL_PAIR_SWAP_FEE_SQL}, 0) AS pair_swap_fee_msat,
            {_JOURNAL_PAIR_OUT_TRANSACTION_SQL} AS pair_out_transaction_id,
            tout.external_id AS pair_out_external_id,
            wout.label AS pair_out_wallet,
            tout.asset AS pair_out_asset,
            COALESCE({_JOURNAL_PAIR_OUT_AMOUNT_SQL}, tout.amount) AS pair_out_amount,
            {_JOURNAL_PAIR_IN_TRANSACTION_SQL} AS pair_in_transaction_id,
            tin.external_id AS pair_in_external_id,
            win.label AS pair_in_wallet,
            tin.asset AS pair_in_asset,
            tin.amount AS pair_in_amount
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        {pair_join_sql}
        LEFT JOIN transactions tout ON tout.id = {_JOURNAL_PAIR_OUT_TRANSACTION_SQL}
        LEFT JOIN transactions tin ON tin.id = {_JOURNAL_PAIR_IN_TRANSACTION_SQL}
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
                "descriptor": bool(config.get("descriptor")),
                "change_descriptor": bool(config.get("change_descriptor")),
                "silent_payment": {
                    "configured": has_silent_payment_sync_material(config),
                    "material_format": str(config.get(silent_payments.CONFIG_MATERIAL_FORMAT) or ""),
                    "scan_mode": str(config.get(silent_payments.CONFIG_SCAN_MODE) or ""),
                    "scan_start_height": config.get(silent_payments.CONFIG_SCAN_START_HEIGHT),
                    "scan_start_date": str(config.get(silent_payments.CONFIG_SCAN_START_DATE) or ""),
                    "full_history": bool(config.get(silent_payments.CONFIG_FULL_HISTORY)),
                },
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
    if sync_mode not in {"backend_descriptor", "backend_addresses", "backend_silent_payment"}:
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
            "message": "This wallet needs a configured chain backend or local scanner before UTXO inventory can refresh.",
        }
    backend_kind = normalize_backend_kind(backend.get("kind"))
    if sync_mode == "backend_silent_payment":
        if backend_kind not in {"esplora", "electrum", "bitcoinrpc", "custom"}:
            return {
                "supported": False,
                "status": "silent_payment_backend_unsupported",
                "reason": "backend_kind",
                "message": f"Silent Payments scanning is not implemented for {backend_kind or 'this backend'} sources.",
            }
        if not silent_payments.backend_supports_silent_payments(backend):
            return {
                "supported": False,
                "status": "silent_payment_backend_unsupported",
                "reason": "missing_sp_capability",
                "message": "This backend is not marked Silent Payments capable; ordinary scripthash sync cannot discover BIP352 outputs.",
            }
        return {
            "supported": True,
            "status": "supported",
            "reason": "",
            "message": "",
        }
    if backend_kind not in {"esplora", "electrum", "bitcoinrpc"}:
        return {
            "supported": False,
            "status": "unsupported_source",
            "reason": "backend_kind",
            "message": f"UTXO inventory is not implemented for {backend_kind or 'this backend'} sources yet.",
        }
    chain = str(config.get("chain") or "bitcoin").strip().lower() or "bitcoin"
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
    redacted.pop("script_pubkey", None)
    redacted.pop("branch_label", None)
    redacted.pop("branch_index", None)
    redacted.pop("address_index", None)
    redacted.pop("derivation_path", None)
    redacted.pop("derivation_paths", None)
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
    exact stored receive outpoints, local transaction graphs, address lists,
    and offline active/retired policy derivation. It never contacts the
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
            if str(backend.get("kind") or "") in core_lightning.LIGHTNING_ADAPTER_KINDS:
                safe["lightningCapabilities"] = (
                    core_lightning.registered_capabilities(
                        str(backend.get("kind") or "")
                    ).to_wire_dict()
                )
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

    freshness = _journal_freshness(conn, profile)
    if freshness["needs_processing"]:
        return {
            "summary": {
                "workspace": context["workspace_label"] or None,
                "profile": context["profile_label"] or None,
                "manual_pairs": 0,
                "same_asset_pairs": 0,
                "cross_asset_pairs": 0,
                "journal_transfer_entries": 0,
                "limit": limit,
                "projection_status": "stale",
            },
            "pairs": [],
        }

    summary = conn.execute(
        """
        SELECT
            COUNT(*) AS manual_pairs,
            SUM(CASE WHEN source_asset = target_asset THEN 1 ELSE 0 END)
                AS same_asset_pairs,
            SUM(CASE WHEN source_asset <> target_asset THEN 1 ELSE 0 END)
                AS cross_asset_pairs
        FROM (
            SELECT source_asset, target_asset
            FROM journal_custody_decisions
            WHERE profile_id = ?
            UNION ALL
            SELECT source_asset, target_asset
            FROM journal_custody_economic_relations
            WHERE profile_id = ? AND relation_kind = 'conversion'
              AND target_transaction_id IS NOT NULL
        )
        """,
        (profile["id"], profile["id"]),
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
            relation.id,
            relation.kind,
            relation.policy,
            relation.created_at,
            relation.out_transaction_id,
            relation.in_transaction_id,
            tout.external_id AS out_external_id,
            tout.occurred_at AS out_occurred_at,
            relation.out_asset,
            relation.out_amount,
            wout.label AS out_wallet,
            tin.external_id AS in_external_id,
            tin.occurred_at AS in_occurred_at,
            relation.in_asset,
            relation.in_amount,
            win.label AS in_wallet,
            relation.occurred_at AS sort_at
        FROM journal_custody_projection_relations relation
        JOIN transactions tout ON tout.id = relation.out_transaction_id
        JOIN transactions tin ON tin.id = relation.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE relation.profile_id = ?
          AND relation.relation_kind IN ('move', 'conversion')
          AND relation.in_transaction_id IS NOT NULL
        ORDER BY sort_at DESC, relation.id DESC
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


def _silent_payment_report_blockers(conn: sqlite3.Connection, profile_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        SELECT id, label, config_json
        FROM wallets
        WHERE profile_id = ?
          AND kind = 'silent-payment'
        ORDER BY label ASC
        """,
        (profile_id,),
    ).fetchall()
    blockers: list[dict[str, Any]] = []
    for wallet in rows:
        config = _json_config(wallet["config_json"])
        if not has_silent_payment_sync_material(config):
            continue
        source_key = core_freshness.source_key(core_freshness.SOURCE_ONCHAIN, wallet["id"])
        state = conn.execute(
            """
            SELECT status, stale_reason, blocking_reports, checkpoint_json
            FROM freshness_source_states
            WHERE profile_id = ? AND source_key = ?
            """,
            (profile_id, source_key),
        ).fetchone()
        if state is None:
            blockers.append(
                {
                    "id": f"silent_payment_scan_pending:{wallet['id']}",
                    "code": "silent_payment_scan_pending",
                    "severity": "blocking",
                    "title": "Silent Payments scan pending",
                    "detail": f"{wallet['label']} has not completed its configured BIP352 scan range yet.",
                    "daemon_kind": "ui.wallets.sync",
                }
            )
            continue
        try:
            checkpoint = json.loads(state["checkpoint_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            checkpoint = {}
        sp_state = checkpoint.get("silent_payment") if isinstance(checkpoint, dict) else None
        sp_state = sp_state if isinstance(sp_state, dict) else {}
        scan_complete = bool(sp_state.get("scan_complete"))
        degraded = bool(sp_state.get("degraded")) or bool(state["blocking_reports"])
        if scan_complete and not degraded:
            continue
        reason = str(sp_state.get("degraded_reason") or state["stale_reason"] or "scan_incomplete")
        blockers.append(
            {
                "id": f"silent_payment_scan_degraded:{wallet['id']}",
                "code": "silent_payment_scan_incomplete",
                "severity": "blocking",
                "title": "Silent Payments scan incomplete",
                "detail": f"{wallet['label']} has incomplete BIP352 scan coverage ({reason}).",
                "daemon_kind": "ui.wallets.sync",
            }
        )
    return blockers


_REPORT_BLOCKING_SWAP_KINDS = frozenset(
    {
        core_transfer_matching.KIND_CHAIN_SWAP,
        core_transfer_matching.KIND_PEG_IN,
        core_transfer_matching.KIND_PEG_OUT,
        core_transfer_matching.KIND_REVERSE_SUBMARINE_SWAP,
        core_transfer_matching.KIND_SUBMARINE_SWAP,
        core_transfer_matching.KIND_SWAP_REFUND,
    }
)


def _load_swap_report_matcher_rows(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT
            t.id, t.profile_id, t.wallet_id, t.external_id, t.payment_hash,
            t.payment_hash_source,
            t.swap_refund_funding_txid,
            t.swap_refund_funding_vout,
            t.occurred_at, t.direction, t.asset, t.amount, t.amount_includes_fee,
            t.fee, t.kind, t.raw_json, t.excluded,
            w.label AS wallet_label, w.kind AS wallet_kind,
            w.config_json AS config_json
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ?
        """,
        (profile_id,),
    ).fetchall()


def _active_swap_review_refs(
    conn: sqlite3.Connection,
    profile_id: str,
) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT
            out_transaction_id,
            in_transaction_id,
            kind,
            policy,
            NULL AS deleted_at
        FROM journal_custody_projection_relations
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()


def _swap_candidate_blocks_reports(
    candidate: core_transfer_matching.SwapCandidate,
) -> bool:
    if candidate.default_kind in _REPORT_BLOCKING_SWAP_KINDS:
        return True
    if str(candidate.out_asset or "").upper() != str(candidate.in_asset or "").upper():
        return True
    return candidate.method in {
        core_transfer_matching.METHOD_PAYMENT_HASH,
        core_transfer_matching.METHOD_PROVIDER_SWAP_ID,
        core_transfer_matching.METHOD_HTLC_REFUND,
    }


def _unreviewed_swap_candidate_blocker(
    conn: sqlite3.Connection,
    profile: sqlite3.Row,
) -> dict[str, Any] | None:
    rows = _load_swap_report_matcher_rows(conn, profile["id"])
    pair_records = _active_swap_review_refs(conn, profile["id"])
    dismissals = conn.execute(
        """
        SELECT out_transaction_id, in_transaction_id, expires_at
        FROM transaction_pair_dismissals
        WHERE profile_id = ?
        """,
        (profile["id"],),
    ).fetchall()
    candidates = [
        candidate
        for candidate in core_transfer_matching.suggest_swap_candidates(
            rows,
            pair_records=pair_records,
            dismissals=dismissals,
        )
        if _swap_candidate_blocks_reports(candidate)
    ]
    if not candidates:
        return None
    exact_count = sum(
        1 for candidate in candidates
        if candidate.confidence == core_transfer_matching.CONFIDENCE_EXACT
    )
    strong_count = sum(
        1 for candidate in candidates
        if candidate.confidence == core_transfer_matching.CONFIDENCE_STRONG
    )
    route_samples = [
        {
            "out_asset": candidate.out_asset,
            "in_asset": candidate.in_asset,
            "confidence": candidate.confidence,
            "method": candidate.method,
            "default_kind": candidate.default_kind,
            "conflict_size": candidate.conflict_size,
        }
        for candidate in candidates[:5]
    ]
    return {
        "id": "unreviewed_swap_candidates",
        "severity": "blocking",
        "title": "Unreviewed swap candidates",
        "detail": (
            f"{len(candidates)} swap-shaped candidate(s) need pairing, payout "
            "review, or dismissal before final reports."
        ),
        "daemon_kind": "ui.transfers.suggest",
        "counts": {
            "total": len(candidates),
            "exact": exact_count,
            "strong": strong_count,
        },
        "routes": route_samples,
    }


def _ownership_review_candidate_blocker(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, Any] | None:
    profile = conn.execute(
        "SELECT ownership_review_counts_json FROM profiles WHERE id = ?",
        (profile_id,),
    ).fetchone()
    if profile is None or not profile["ownership_review_counts_json"]:
        return None
    try:
        cached = json.loads(profile["ownership_review_counts_json"])
        total = int(cached.get("total") or 0)
        by_reason = {
            str(reason): int(count)
            for reason, count in (cached.get("by_reason") or {}).items()
            if int(count) > 0
        }
    except (AttributeError, TypeError, ValueError, json.JSONDecodeError):
        return None
    if total <= 0:
        return None
    return {
        "id": "ownership_transfer_review",
        "severity": "blocking",
        "title": "Ownership transfers need review",
        "detail": (
            f"{total} ownership-proven transfer candidate(s) need pairing or "
            "wallet-data review before final reports."
        ),
        "daemon_kind": "ui.transfers.suggest",
        "daemon_args": {
            "candidate_type": "transfer",
            "method": core_transfer_matching.METHOD_OWNERSHIP_GRAPH,
        },
        "counts": {"total": total, "by_reason": by_reason},
    }


def build_report_blockers_snapshot(conn: sqlite3.Connection) -> dict[str, Any]:
    health = build_workspace_health_snapshot(conn)
    rates_coverage = build_rates_coverage_snapshot(conn, {"limit": 10})
    blockers: list[dict[str, Any]] = []
    custody_quantity: dict[str, Any] = {
        "status": "unavailable",
        "status_text": "Custody gap status unavailable: no active profile",
        "derived_state_current": False,
        "issue_count": 0,
        "quantified_issue_count": 0,
        "unquantified_issue_count": 0,
        "unresolved_by_asset": [],
        "by_state": [],
        "blocked_from": None,
        "presumed_external": {
            "slice_count": 0,
            "transaction_count": 0,
            "by_asset": [],
            "treatment": "warning_not_blocker",
        },
        "warnings": [],
        "qualification": (
            "This reports gaps detectable from current imported evidence; it "
            "does not assert that every wallet was imported."
        ),
    }
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
        custody_quantity = (
            core_custody_quantity_store.custody_quantity_readiness_summary(
                conn,
                health["profile"]["id"],
                journal_status=str(journals["status"]),
            )
        )
        edit_stale = transaction_history.stale_summary(
            conn,
            {"id": health["profile"]["id"], **journals},
        )
        sync_conflicts = conn.execute(
            """
            SELECT id, entity_table, entity_key, field, created_at
            FROM sync_conflicts
            WHERE profile_id = ? AND status = 'open'
            ORDER BY created_at, id
            LIMIT 20
            """,
            (health["profile"]["id"],),
        ).fetchall()
        if sync_conflicts:
            blockers.append(
                {
                    "id": "sync_conflicts",
                    "severity": "blocking",
                    "title": "Conflicting synced edits",
                    "detail": (
                        f"{len(sync_conflicts)} high-stakes concurrent edit(s) need "
                        "a human decision before journals can be processed."
                    ),
                    "daemon_kind": "ui.sync.conflicts.list",
                    "conflicts": [dict(row) for row in sync_conflicts],
                }
            )
        authored_active_components = list(
            core_custody_components.iter_authored_active_components(
                conn,
                profile_id=health["profile"]["id"],
                include_local_evidence=False,
            )
        )
        ineffective_components = [
            component
            for component in authored_active_components
            if component["effective_state"] != "active"
        ]
        if ineffective_components:
            blockers.append(
                {
                    "id": "custody_component_integrity",
                    "severity": "blocking",
                    "title": "Incomplete custody interpretation",
                    "detail": (
                        f"{len(ineffective_components)} authored active custody "
                        "component(s) are incomplete or conflicting. Repair or "
                        "supersede them before relying on reports."
                    ),
                    "daemon_kind": "ui.transfers.components.list",
                    "components": [
                        {
                            "id": component["id"],
                            "lineage_id": component["lineage_id"],
                            "revision": component["revision"],
                            "issue_codes": sorted(
                                {
                                    str(issue.get("code") or "unknown")
                                    for issue in component["validation"]["issues"]
                                }
                            ),
                            "known_anchor_count": sum(
                                leg.get("transaction_id") is not None
                                for leg in component["legs"]
                            ),
                        }
                        for component in ineffective_components[:20]
                    ],
                }
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
        blockers.extend(_silent_payment_report_blockers(conn, health["profile"]["id"]))
        overlap = core_source_overlap.detect_profile_source_overlaps(
            conn,
            health["profile"]["id"],
        )
        if overlap["overlaps"]:
            repair_preview = core_source_overlap.duplicate_transaction_preview(
                conn,
                health["profile"]["id"],
                overlap["overlaps"],
            )
            blockers.append(
                {
                    "id": "source_overlap",
                    "severity": "blocking",
                    "title": "Overlapping wallet sources",
                    "detail": (
                        f"{overlap['overlap_count']} concrete watched script(s) are "
                        "owned by multiple active sources. Kassiber can resolve "
                        "descriptor/xpub versus address-list duplicates by trimming "
                        "the address-list source and excluding exact duplicate rows "
                        "before rebuilding journals."
                    ),
                    "daemon_kind": "ui.journals.process",
                    "overlap": overlap,
                    "repair_preview": repair_preview,
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
        if custody_quantity["issue_count"]:
            blockers.append(
                {
                    "id": "custody_quantity_unresolved",
                    "severity": "blocking",
                    "title": "Unresolved custody quantity",
                    "detail": (
                        f"{custody_quantity['issue_count']} exact quantity issue(s) block "
                        "final basis from the earliest affected event onward."
                    ),
                    "daemon_kind": "ui.journals.process",
                    "blocked_from": custody_quantity["blocked_from"],
                    "states": [
                        item["state"] for item in custody_quantity["by_state"]
                    ],
                    "unresolved_by_asset": custody_quantity[
                        "unresolved_by_asset"
                    ],
                    "unresolved_msat": (
                        custody_quantity["unresolved_by_asset"][0]["amount_msat"]
                        if len(custody_quantity["unresolved_by_asset"]) == 1
                        else None
                    ),
                    "unquantified_issue_count": custody_quantity[
                        "unquantified_issue_count"
                    ],
                }
            )
        ownership_blocker = _ownership_review_candidate_blocker(
            conn, health["profile"]["id"]
        )
        if ownership_blocker is not None:
            blockers.append(ownership_blocker)
        swap_blocker = _unreviewed_swap_candidate_blocker(conn, health["profile"])
        if swap_blocker is not None:
            blockers.append(swap_blocker)
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
        "warnings": custody_quantity["warnings"],
        "custody_quantity": custody_quantity,
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
                "generated_at": now_iso(),
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
            "generated_at": now_iso(),
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
