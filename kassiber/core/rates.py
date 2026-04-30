from __future__ import annotations

import json
from datetime import datetime, timezone
from urllib import error as urlerror
from urllib import request as urlrequest

from .. import __version__
from ..db import APP_NAME
from ..errors import AppError
from . import pricing
from ..time_utils import _iso_z, _parse_iso_datetime

SUPPORTED_RATE_PAIRS = ("BTC-USD", "BTC-EUR")
_COINGECKO_VS = {"USD": "usd", "EUR": "eur"}
_COINGECKO_COIN = {"BTC": "bitcoin"}
_RATE_ASSET_ALIASES = {"LBTC": "BTC"}


def _normalize_rate_pair(pair):
    if not pair:
        raise AppError("Pair is required", code="validation")
    raw = pair.strip().upper().replace("/", "-")
    if "-" not in raw:
        raise AppError(
            f"Invalid pair '{pair}'",
            code="validation",
            hint="Use <ASSET>-<FIAT>, e.g. BTC-USD",
        )
    asset, _, fiat = raw.partition("-")
    if not asset or not fiat:
        raise AppError(f"Invalid pair '{pair}'", code="validation")
    return f"{asset}-{fiat}"


def require_supported_pair(pair):
    normalized = _normalize_rate_pair(pair)
    if normalized not in SUPPORTED_RATE_PAIRS:
        raise AppError(
            f"Pair '{normalized}' is not supported",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    return normalized


def rate_pair_parts(pair):
    asset, _, fiat = pair.partition("-")
    return asset, fiat


def transaction_rate_pair(asset, fiat_currency):
    asset_code = str(asset or "").strip().upper()
    fiat_code = str(fiat_currency or "").strip().upper()
    if not asset_code or not fiat_code:
        return None
    asset_code = _RATE_ASSET_ALIASES.get(asset_code, asset_code)
    pair = f"{asset_code}-{fiat_code}"
    if pair not in SUPPORTED_RATE_PAIRS:
        return None
    return pair


def http_get_json(url, timeout=30):
    request = urlrequest.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": f"{APP_NAME}/{__version__}",
        },
    )
    try:
        with urlrequest.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except urlerror.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise AppError(f"HTTP {exc.code} from backend for {url}: {detail[:200]}") from exc
    except urlerror.URLError as exc:
        raise AppError(f"Failed to reach backend {url}: {exc.reason}") from exc


def upsert_rate(conn, pair, timestamp, rate, source, fetched_at=None, granularity=None, method=None):
    normalized = _normalize_rate_pair(pair)
    ts = _iso_z(_parse_iso_datetime(timestamp, "rate_timestamp"))
    fetched = fetched_at or _iso_z(datetime.now(timezone.utc))
    rate_exact = pricing.exact_decimal(rate)
    conn.execute(
        """
        INSERT INTO rates_cache(pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(pair, timestamp, source) DO UPDATE SET
            rate = excluded.rate,
            rate_exact = excluded.rate_exact,
            fetched_at = excluded.fetched_at,
            granularity = excluded.granularity,
            method = excluded.method
        """,
        (
            normalized,
            ts,
            float(pricing.decimal_from_exact(rate_exact)),
            rate_exact,
            source,
            fetched,
            granularity,
            method,
        ),
    )
    return {
        "pair": normalized,
        "timestamp": ts,
        "rate": float(pricing.decimal_from_exact(rate_exact)),
        "rate_exact": rate_exact,
        "source": source,
        "fetched_at": fetched,
        "granularity": granularity,
        "method": method,
    }


def get_latest_rate(conn, pair):
    normalized = _normalize_rate_pair(pair)
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method
        FROM rates_cache
        WHERE pair = ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC
        LIMIT 1
        """,
        (normalized,),
    ).fetchone()
    if not row:
        raise AppError(
            f"No cached rate for pair '{normalized}'",
            code="not_found",
            hint="Run `kassiber rates sync` first",
        )
    return {
        "pair": row["pair"],
        "timestamp": row["timestamp"],
        "rate": row["rate"],
        "rate_exact": row["rate_exact"],
        "source": row["source"],
        "fetched_at": row["fetched_at"],
        "granularity": row["granularity"],
        "method": row["method"],
    }


def get_rate_range(conn, pair, start=None, end=None, order="asc", limit=None):
    normalized = _normalize_rate_pair(pair)
    effective_limit = None
    if limit is not None:
        effective_limit = int(limit)
        if effective_limit <= 0:
            raise AppError("--limit must be positive", code="validation")
    if order not in {"asc", "desc"}:
        raise AppError("--order must be asc or desc", code="validation")
    order_sql = order.upper()
    sql = "SELECT pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method FROM rates_cache WHERE pair = ?"
    params = [normalized]
    if start:
        start_dt = _parse_iso_datetime(start, "start")
        sql += " AND timestamp >= ?"
        params.append(_iso_z(start_dt))
    if end:
        end_dt = _parse_iso_datetime(end, "end")
        sql += " AND timestamp <= ?"
        params.append(_iso_z(end_dt))
    sql += (
        f" ORDER BY timestamp {order_sql},"
        " CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,"
        " fetched_at DESC,"
        " source ASC"
    )
    if effective_limit is not None:
        sql += " LIMIT ?"
        params.append(effective_limit)
    rows = conn.execute(sql, params).fetchall()
    return [
        {
            "pair": row["pair"],
            "timestamp": row["timestamp"],
            "rate": row["rate"],
            "rate_exact": row["rate_exact"],
            "source": row["source"],
            "fetched_at": row["fetched_at"],
            "granularity": row["granularity"],
            "method": row["method"],
        }
        for row in rows
    ]


def get_cached_rate_at_or_before(conn, pair, occurred_at):
    normalized = require_supported_pair(pair)
    occurred_ts = _iso_z(_parse_iso_datetime(occurred_at, "occurred_at"))
    row = conn.execute(
        """
        SELECT pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method
        FROM rates_cache
        WHERE pair = ? AND timestamp <= ?
        ORDER BY timestamp DESC,
                 CASE WHEN source = 'manual' THEN 0 ELSE 1 END ASC,
                 fetched_at DESC
        LIMIT 1
        """,
        (normalized, occurred_ts),
    ).fetchone()
    if not row:
        return None
    return {
        "pair": row["pair"],
        "timestamp": row["timestamp"],
        "rate": row["rate"],
        "rate_exact": row["rate_exact"],
        "source": row["source"],
        "fetched_at": row["fetched_at"],
        "granularity": row["granularity"],
        "method": row["method"],
    }


def _invalidate_profile_journals_for_pair(conn, pair):
    normalized = _normalize_rate_pair(pair)
    if normalized not in SUPPORTED_RATE_PAIRS:
        return
    _, fiat = rate_pair_parts(normalized)
    conn.execute(
        """
        UPDATE profiles
        SET last_processed_at = NULL, last_processed_tx_count = 0
        WHERE upper(fiat_currency) = ?
        """,
        (fiat.upper(),),
    )


def list_cached_pairs(conn):
    rows = conn.execute(
        """
        SELECT pair,
               COUNT(*) AS sample_count,
               MIN(timestamp) AS first_timestamp,
               MAX(timestamp) AS last_timestamp
        FROM rates_cache
        GROUP BY pair
        ORDER BY pair ASC
        """
    ).fetchall()
    known = {pair: None for pair in SUPPORTED_RATE_PAIRS}
    for row in rows:
        known[row["pair"]] = {
            "sample_count": int(row["sample_count"]),
            "first_timestamp": row["first_timestamp"],
            "last_timestamp": row["last_timestamp"],
        }
    result = []
    for pair in SUPPORTED_RATE_PAIRS:
        detail = known.get(pair)
        result.append(
            {
                "pair": pair,
                "supported": True,
                "cached": detail is not None,
                "sample_count": detail["sample_count"] if detail else 0,
                "first_timestamp": detail["first_timestamp"] if detail else None,
                "last_timestamp": detail["last_timestamp"] if detail else None,
            }
        )
    for pair, detail in known.items():
        if pair in SUPPORTED_RATE_PAIRS or detail is None:
            continue
        result.append(
            {
                "pair": pair,
                "supported": False,
                "cached": True,
                "sample_count": detail["sample_count"],
                "first_timestamp": detail["first_timestamp"],
                "last_timestamp": detail["last_timestamp"],
            }
        )
    return result


def _coingecko_granularity(days):
    days_int = int(days)
    if days_int > 90:
        return "daily"
    if days_int > 1:
        return "hourly"
    return "five_minute"


def _coingecko_market_chart(coin_id, vs, days):
    url = (
        "https://api.coingecko.com/api/v3/coins/"
        f"{coin_id}/market_chart?vs_currency={vs}&days={int(days)}"
    )
    payload = http_get_json(url, timeout=30)
    prices = payload.get("prices") if isinstance(payload, dict) else None
    if not isinstance(prices, list):
        raise AppError(
            "CoinGecko response did not contain a prices array",
            code="upstream_error",
            retryable=True,
        )
    output = []
    for entry in prices:
        if not isinstance(entry, list) or len(entry) < 2:
            continue
        ms, value = entry[0], entry[1]
        try:
            ts = datetime.fromtimestamp(float(ms) / 1000.0, tz=timezone.utc)
            rate = float(value)
        except (TypeError, ValueError):
            continue
        output.append((_iso_z(ts.replace(microsecond=0)), rate))
    return output


def fetch_rates_coingecko(pair, days=30):
    normalized = require_supported_pair(pair)
    asset, fiat = rate_pair_parts(normalized)
    coin_id = _COINGECKO_COIN.get(asset)
    vs = _COINGECKO_VS.get(fiat)
    if not coin_id or not vs:
        raise AppError(
            f"Pair '{normalized}' has no CoinGecko mapping",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    return _coingecko_market_chart(coin_id, vs, days)


def sync_rates(conn, pair=None, days=30, source="coingecko"):
    if source != "coingecko":
        raise AppError(
            f"Unknown rate source '{source}'",
            code="validation",
            hint="Supported sources: coingecko",
        )
    if int(days) <= 0:
        raise AppError("--days must be positive", code="validation")
    if pair:
        pairs = [require_supported_pair(pair)]
    else:
        pairs = list(SUPPORTED_RATE_PAIRS)
    fetched_at = _iso_z(datetime.now(timezone.utc))
    summary = []
    granularity = _coingecko_granularity(days)
    for normalized_pair in pairs:
        samples = fetch_rates_coingecko(normalized_pair, days=days)
        inserted = 0
        for timestamp, rate in samples:
            upsert_rate(
                conn,
                normalized_pair,
                timestamp,
                rate,
                source,
                fetched_at=fetched_at,
                granularity=granularity,
                method="market_chart",
            )
            inserted += 1
        _invalidate_profile_journals_for_pair(conn, normalized_pair)
        conn.commit()
        summary.append(
            {
                "pair": normalized_pair,
                "source": source,
                "samples": inserted,
                "days": int(days),
                "granularity": granularity,
                "fetched_at": fetched_at,
            }
        )
    return summary


def set_manual_rate(conn, pair, timestamp, rate, source="manual", granularity=None, method=None):
    normalized = _normalize_rate_pair(pair)
    try:
        value = pricing.decimal_from_exact(rate)
    except Exception as exc:
        raise AppError(f"Invalid rate '{rate}'", code="validation") from exc
    if value is None:
        raise AppError(f"Invalid rate '{rate}'", code="validation")
    if value <= 0:
        raise AppError("Rate must be positive", code="validation")
    effective_granularity = granularity or ("exact" if source == "manual" else "unknown")
    row = upsert_rate(
        conn,
        normalized,
        timestamp,
        value,
        source,
        granularity=effective_granularity,
        method=method or ("manual" if source == "manual" else "operator_supplied"),
    )
    _invalidate_profile_journals_for_pair(conn, normalized)
    conn.commit()
    return row


__all__ = [
    "SUPPORTED_RATE_PAIRS",
    "fetch_rates_coingecko",
    "get_cached_rate_at_or_before",
    "get_latest_rate",
    "get_rate_range",
    "list_cached_pairs",
    "rate_pair_parts",
    "require_supported_pair",
    "set_manual_rate",
    "sync_rates",
    "transaction_rate_pair",
    "upsert_rate",
]
