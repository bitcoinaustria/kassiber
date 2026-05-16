from __future__ import annotations

import csv
import io
import json
import logging
from pathlib import Path
import zipfile
from datetime import datetime, timedelta, timezone
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .. import __version__
from ..db import APP_NAME
from ..errors import AppError
from . import pricing
from ..time_utils import _iso_z, _parse_iso_datetime

logger = logging.getLogger(__name__)

SUPPORTED_RATE_PAIRS = ("BTC-USD", "BTC-EUR")
RATE_SOURCE_COINBASE_EXCHANGE = "coinbase-exchange"
RATE_SOURCE_COINGECKO = "coingecko"
RATE_SOURCE_KRAKEN_CSV = "kraken-csv"
SUPPORTED_RATE_SOURCES = (
    RATE_SOURCE_COINBASE_EXCHANGE,
    RATE_SOURCE_KRAKEN_CSV,
    RATE_SOURCE_COINGECKO,
)
_COINGECKO_VS = {"USD": "usd", "EUR": "eur"}
_COINGECKO_COIN = {"BTC": "bitcoin"}
_COINBASE_EXCHANGE_PRODUCT = {"BTC-USD": "BTC-USD", "BTC-EUR": "BTC-EUR"}
_RATE_ASSET_ALIASES = {"LBTC": "BTC"}
_KRAKEN_STABLECOIN_QUOTES = {"DAI", "USDC", "USDT"}
_KRAKEN_SUPPORTED_QUOTES = {"EUR", "USD"}
_KRAKEN_BATCH_SIZE = 10_000
_COINBASE_MAX_CANDLES = 300


_RATE_UPSERT_SQL = """
    INSERT INTO rates_cache(
        pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method,
        open_rate, open_rate_exact, high_rate, high_rate_exact,
        low_rate, low_rate_exact, close_rate, close_rate_exact,
        volume, volume_exact, trades
    )
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(pair, timestamp, source) DO UPDATE SET
        rate = excluded.rate,
        rate_exact = excluded.rate_exact,
        fetched_at = excluded.fetched_at,
        granularity = excluded.granularity,
        method = excluded.method,
        open_rate = excluded.open_rate,
        open_rate_exact = excluded.open_rate_exact,
        high_rate = excluded.high_rate,
        high_rate_exact = excluded.high_rate_exact,
        low_rate = excluded.low_rate,
        low_rate_exact = excluded.low_rate_exact,
        close_rate = excluded.close_rate,
        close_rate_exact = excluded.close_rate_exact,
        volume = excluded.volume,
        volume_exact = excluded.volume_exact,
        trades = excluded.trades
"""


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


def _transaction_price_missing_sql(prefix: str):
    return f"""
            (
              ({prefix}fiat_rate IS NULL OR {prefix}fiat_rate <= 0)
              AND (
                {prefix}fiat_rate_exact IS NULL
                OR CAST({prefix}fiat_rate_exact AS REAL) <= 0
              )
              AND ({prefix}fiat_value IS NULL OR {prefix}fiat_value <= 0)
              AND (
                {prefix}fiat_value_exact IS NULL
                OR CAST({prefix}fiat_value_exact AS REAL) <= 0
              )
            )
    """


def transaction_price_missing_sql():
    return _transaction_price_missing_sql("t.")


def transaction_price_missing_sql_unqualified():
    return _transaction_price_missing_sql("")


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


def _float_from_exact(value):
    if value in (None, ""):
        return None, None
    exact = pricing.exact_decimal(value)
    if exact is None:
        return None, None
    return float(pricing.decimal_from_exact(exact)), exact


def _rate_insert_params(
    pair,
    timestamp,
    rate,
    source,
    fetched_at,
    granularity,
    method,
    *,
    open_rate=None,
    high_rate=None,
    low_rate=None,
    close_rate=None,
    volume=None,
    trades=None,
):
    normalized = _normalize_rate_pair(pair)
    ts = _iso_z(_parse_iso_datetime(timestamp, "rate_timestamp"))
    rate_float, rate_exact = _float_from_exact(rate)
    if rate_exact is None:
        raise AppError(f"Invalid rate '{rate}'", code="validation")
    close_input = close_rate if close_rate is not None else rate
    open_float, open_exact = _float_from_exact(open_rate) if open_rate is not None else (None, None)
    high_float, high_exact = _float_from_exact(high_rate) if high_rate is not None else (None, None)
    low_float, low_exact = _float_from_exact(low_rate) if low_rate is not None else (None, None)
    close_float, close_exact = _float_from_exact(close_input)
    volume_float, volume_exact = _float_from_exact(volume) if volume is not None else (None, None)
    trades_int = int(trades) if trades is not None else None
    return (
        normalized,
        ts,
        rate_float,
        rate_exact,
        source,
        fetched_at,
        granularity,
        method,
        open_float,
        open_exact,
        high_float,
        high_exact,
        low_float,
        low_exact,
        close_float,
        close_exact,
        volume_float,
        volume_exact,
        trades_int,
    )


def upsert_rate(
    conn,
    pair,
    timestamp,
    rate,
    source,
    fetched_at=None,
    granularity=None,
    method=None,
    *,
    open_rate=None,
    high_rate=None,
    low_rate=None,
    close_rate=None,
    volume=None,
    trades=None,
):
    fetched = fetched_at or _iso_z(datetime.now(timezone.utc))
    params = _rate_insert_params(
        pair,
        timestamp,
        rate,
        source,
        fetched,
        granularity,
        method,
        open_rate=open_rate,
        high_rate=high_rate,
        low_rate=low_rate,
        close_rate=close_rate,
        volume=volume,
        trades=trades,
    )
    conn.execute(_RATE_UPSERT_SQL, params)
    return {
        "pair": params[0],
        "timestamp": params[1],
        "rate": params[2],
        "rate_exact": params[3],
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
        SET last_processed_at = NULL,
            last_processed_tx_count = 0,
            journal_input_version = journal_input_version + 1
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


def _sync_rates_coingecko(
    conn,
    pair=None,
    days=30,
    source=RATE_SOURCE_COINGECKO,
    commit=True,
):
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
        if commit:
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


_COINBASE_GRANULARITIES = {60, 300, 900, 3600, 21600, 86400}


def _coinbase_exchange_url(product_id, start, end, granularity):
    granularity_int = int(granularity)
    if granularity_int not in _COINBASE_GRANULARITIES:
        raise AppError(
            f"Coinbase Exchange granularity must be one of {sorted(_COINBASE_GRANULARITIES)}",
            code="validation",
        )
    query = urlparse.urlencode(
        {
            "granularity": str(granularity_int),
            "start": _iso_z(start),
            "end": _iso_z(end),
        }
    )
    return f"https://api.exchange.coinbase.com/products/{product_id}/candles?{query}"


def _coinbase_exchange_candles(pair, start, end, granularity=60):
    normalized = require_supported_pair(pair)
    product_id = _COINBASE_EXCHANGE_PRODUCT.get(normalized)
    if not product_id:
        raise AppError(
            f"Pair '{normalized}' has no Coinbase Exchange mapping",
            code="validation",
            hint=f"Supported pairs: {', '.join(SUPPORTED_RATE_PAIRS)}",
        )
    payload = http_get_json(
        _coinbase_exchange_url(product_id, start, end, granularity),
        timeout=30,
    )
    if not isinstance(payload, list):
        raise AppError(
            "Coinbase Exchange response did not contain candle rows",
            code="upstream_error",
            retryable=True,
        )
    return payload


def _parse_coinbase_exchange_rows(rows, granularity=60):
    output = []
    granularity_seconds = int(granularity)
    for row in rows:
        if not isinstance(row, list) or len(row) < 6:
            continue
        try:
            timestamp_seconds = int(row[0])
            close_timestamp = _iso_z(
                datetime.fromtimestamp(
                    timestamp_seconds + granularity_seconds,
                    tz=timezone.utc,
                )
            )
            candle = {
                "timestamp": close_timestamp,
                "low": str(row[1]),
                "high": str(row[2]),
                "open": str(row[3]),
                "close": str(row[4]),
                "volume": str(row[5]),
                "trades": None,
            }
            for field in ("open", "high", "low", "close", "volume"):
                pricing.decimal_from_exact(candle[field])
        except (TypeError, ValueError):
            continue
        output.append(candle)
    return sorted(output, key=lambda candle: candle["timestamp"])


def _floor_to_minute(value):
    dt = _parse_iso_datetime(value, "timestamp")
    return _iso_z(dt.replace(second=0, microsecond=0))


def _chunked(items, size=900):
    chunk = []
    for item in items:
        chunk.append(item)
        if len(chunk) >= size:
            yield chunk
            chunk = []
    if chunk:
        yield chunk


def _collect_coinbase_needed_minutes(conn, pairs):
    pair_set = set(pairs)
    needed = {pair: set() for pair in pair_set}
    now_minute = datetime.now(timezone.utc).replace(second=0, microsecond=0)
    missing_price_sql = transaction_price_missing_sql()
    rows = conn.execute(
        """
        SELECT t.occurred_at, t.confirmed_at, t.asset, t.fiat_currency,
               p.fiat_currency AS profile_fiat_currency
        FROM transactions t
        JOIN profiles p ON p.id = t.profile_id
        WHERE t.excluded = 0
          AND (
            {missing_price_sql}
            OR (
              t.fiat_price_source = ?
              AND t.pricing_source_kind IS NULL
              AND t.pricing_quality IS NULL
            )
          )
        ORDER BY COALESCE(t.confirmed_at, t.occurred_at) ASC, t.created_at ASC, t.id ASC
        """.format(missing_price_sql=missing_price_sql),
        (pricing.LEGACY_SOURCE_RATES_CACHE,),
    ).fetchall()
    for row in rows:
        pair = transaction_rate_pair(
            row["asset"],
            row["fiat_currency"] or row["profile_fiat_currency"],
        )
        if pair not in pair_set:
            continue
        pricing_at = row["confirmed_at"] or row["occurred_at"]
        try:
            minute = _floor_to_minute(pricing_at)
        except AppError:
            logger.warning(
                "Skipping transaction with invalid pricing timestamp: %s",
                pricing_at,
            )
            continue
        if _parse_iso_datetime(minute, "rate_timestamp") > now_minute:
            continue
        needed[pair].add(minute)
    return needed


def _existing_rate_minutes(conn, pair, timestamps):
    existing = set()
    ordered = sorted(timestamps)
    for chunk in _chunked(ordered):
        placeholders = ", ".join("?" for _ in chunk)
        rows = conn.execute(
            f"""
            SELECT timestamp
            FROM rates_cache
            WHERE pair = ?
              AND timestamp IN ({placeholders})
              AND (
                granularity = 'minute'
                OR source IN ('manual', ?, ?)
              )
            """,
            [pair, *chunk, RATE_SOURCE_COINBASE_EXCHANGE, RATE_SOURCE_KRAKEN_CSV],
        ).fetchall()
        existing.update(row["timestamp"] for row in rows)
    return existing


def _checked_rate_minutes(conn, pair, timestamps, source=RATE_SOURCE_COINBASE_EXCHANGE):
    checked = set()
    ordered = sorted(timestamps)
    for chunk in _chunked(ordered):
        placeholders = ", ".join("?" for _ in chunk)
        rows = conn.execute(
            f"""
            SELECT timestamp
            FROM rates_checked_minutes
            WHERE pair = ?
              AND source = ?
              AND timestamp IN ({placeholders})
            """,
            [pair, source, *chunk],
        ).fetchall()
        checked.update(row["timestamp"] for row in rows)
    return checked


def _filter_missing_coinbase_minutes(conn, pair, timestamps):
    needed = set(timestamps)
    if not needed:
        return {
            "missing": set(),
            "cached": set(),
            "checked": set(),
        }
    cached = _existing_rate_minutes(conn, pair, needed)
    checked = _checked_rate_minutes(conn, pair, needed - cached)
    return {
        "missing": needed - cached - checked,
        "cached": cached,
        "checked": checked,
    }


def _coinbase_windows_for_close_minutes(minutes, granularity=60, now=None):
    granularity_seconds = int(granularity)
    delta = timedelta(seconds=granularity_seconds)
    step = delta * _COINBASE_MAX_CANDLES
    now_dt = (now or datetime.now(timezone.utc)).replace(second=0, microsecond=0)
    close_times = sorted(
        dt
        for dt in (
            _parse_iso_datetime(minute, "rate_timestamp").replace(second=0, microsecond=0)
            for minute in minutes
        )
        if dt <= now_dt
    )
    windows = []
    index = 0
    while index < len(close_times):
        close_start = close_times[index]
        close_end = min(close_start + step - delta, now_dt)
        windows.append((close_start - delta, close_end))
        index += 1
        while index < len(close_times) and close_times[index] <= close_end:
            index += 1
    return windows


def _coinbase_checked_minutes_for_window(start, end, granularity=60):
    delta = timedelta(seconds=int(granularity))
    cursor = start + delta
    minutes = []
    while cursor <= end:
        minutes.append(_iso_z(cursor))
        cursor += delta
    return minutes


def _mark_rate_minutes_checked(
    conn,
    pair,
    timestamps,
    checked_at,
    source=RATE_SOURCE_COINBASE_EXCHANGE,
    granularity="minute",
    method="product_candles",
):
    rows = [
        (pair, timestamp, source, checked_at, granularity, method)
        for timestamp in sorted(set(timestamps))
    ]
    if not rows:
        return 0
    conn.executemany(
        """
        INSERT INTO rates_checked_minutes(
            pair, timestamp, source, checked_at, granularity, method
        )
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(pair, timestamp, source) DO UPDATE SET
            checked_at = excluded.checked_at,
            granularity = excluded.granularity,
            method = excluded.method
        """,
        rows,
    )
    return len(rows)


def _delete_rate_rows(conn, pair=None, source=None):
    clauses = []
    params = []
    if pair:
        clauses.append("pair = ?")
        params.append(require_supported_pair(pair))
    if source:
        clauses.append("source = ?")
        params.append(str(source).strip().lower())
    where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
    cursor = conn.execute(f"DELETE FROM rates_cache {where}", params)
    return cursor.rowcount if cursor.rowcount is not None else 0


def _delete_checked_rate_minutes(conn, pair=None, source=None):
    clauses = []
    params = []
    if pair:
        clauses.append("pair = ?")
        params.append(require_supported_pair(pair))
    if source:
        clauses.append("source = ?")
        params.append(str(source).strip().lower())
    where = f"WHERE {' AND '.join(clauses)}" if clauses else ""
    cursor = conn.execute(f"DELETE FROM rates_checked_minutes {where}", params)
    return cursor.rowcount if cursor.rowcount is not None else 0


def _provider_price_transaction_rows(
    conn,
    pair=None,
    source=None,
    profile_id=None,
    pairs=None,
):
    normalized_pairs = set()
    if pair:
        normalized_pairs.add(require_supported_pair(pair))
    elif pairs:
        normalized_pairs.update(require_supported_pair(candidate) for candidate in pairs)
    normalized_source = str(source).strip().lower() if source else None
    clauses = [
        """
        (
          pricing_source_kind = ?
          OR (
            fiat_price_source = ?
            AND pricing_source_kind IS NULL
            AND pricing_quality IS NULL
          )
        )
        """,
    ]
    params = [pricing.SOURCE_FMV_PROVIDER, pricing.LEGACY_SOURCE_RATES_CACHE]
    if profile_id:
        clauses.append("t.profile_id = ?")
        params.append(profile_id)
    where = " AND ".join(clauses)
    rows = conn.execute(
        f"""
        SELECT t.id, t.profile_id, t.asset, t.fiat_currency,
               t.pricing_source_kind, t.pricing_provider, t.pricing_pair,
               t.fiat_price_source, t.pricing_quality,
               p.fiat_currency AS profile_fiat_currency
        FROM transactions t
        JOIN profiles p ON p.id = t.profile_id
        WHERE {where}
        """,
        params,
    ).fetchall()
    matched = []
    for row in rows:
        is_modern_provider = row["pricing_source_kind"] == pricing.SOURCE_FMV_PROVIDER
        if (
            normalized_source
            and is_modern_provider
            and row["pricing_provider"] != normalized_source
        ):
            continue
        if normalized_pairs:
            candidate_pair = row["pricing_pair"] or transaction_rate_pair(
                row["asset"],
                row["fiat_currency"] or row["profile_fiat_currency"],
            )
            if candidate_pair not in normalized_pairs:
                continue
        matched.append(row)
    return matched


def _clear_provider_transaction_prices(
    conn,
    pair=None,
    source=None,
    profile_id=None,
    pairs=None,
):
    rows = _provider_price_transaction_rows(
        conn,
        pair=pair,
        source=source,
        profile_id=profile_id,
        pairs=pairs,
    )
    ids = [row["id"] for row in rows]
    if not ids:
        return {"transactions": 0, "profiles": 0}
    for chunk in _chunked(ids):
        placeholders = ", ".join("?" for _ in chunk)
        conn.execute(
            f"""
        UPDATE transactions
        SET fiat_rate = NULL,
            fiat_value = NULL,
            fiat_price_source = NULL,
            fiat_rate_exact = NULL,
            fiat_value_exact = NULL,
            pricing_source_kind = NULL,
            pricing_provider = NULL,
            pricing_pair = NULL,
            pricing_timestamp = NULL,
            pricing_fetched_at = NULL,
            pricing_granularity = NULL,
            pricing_method = NULL,
            pricing_external_ref = NULL,
            pricing_quality = NULL
        WHERE id IN ({placeholders})
        """,
            chunk,
        )
    affected_profiles = sorted({row["profile_id"] for row in rows})
    for profile in affected_profiles:
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = NULL,
                last_processed_tx_count = 0,
                journal_input_version = journal_input_version + 1
            WHERE id = ?
            """,
            (profile,),
        )
    return {
        "transactions": len(ids),
        "profiles": len(affected_profiles),
    }


def rebuild_rates_cache(
    conn,
    pair=None,
    days=30,
    source=RATE_SOURCE_COINBASE_EXCHANGE,
    path=None,
    reprice_transactions=False,
    profile_id=None,
):
    normalized_source = str(source or "").strip().lower()
    if normalized_source not in SUPPORTED_RATE_SOURCES:
        raise AppError(
            f"Unknown rate source '{source}'",
            code="validation",
            hint=f"Supported sources: {', '.join(SUPPORTED_RATE_SOURCES)}",
        )
    normalized_pair = require_supported_pair(pair) if pair else None
    if normalized_source == RATE_SOURCE_KRAKEN_CSV and not path:
        raise AppError(
            "--path is required for --source kraken-csv",
            code="validation",
        )
    if normalized_source != RATE_SOURCE_KRAKEN_CSV and path:
        raise AppError(
            "--path is only supported for --source kraken-csv",
            code="validation",
        )

    try:
        days_int = int(days)
    except (TypeError, ValueError) as exc:
        raise AppError("--days must be positive", code="validation") from exc
    if days_int <= 0:
        raise AppError("--days must be positive", code="validation")
    conn.execute("SAVEPOINT rates_rebuild")
    try:
        supported_pairs = [normalized_pair] if normalized_pair else list(SUPPORTED_RATE_PAIRS)
        transaction_prices = {"transactions": 0, "profiles": 0}
        if reprice_transactions:
            transaction_prices = _clear_provider_transaction_prices(
                conn,
                pair=normalized_pair,
                source=normalized_source,
                profile_id=profile_id,
                pairs=supported_pairs,
            )
        deleted_rates = _delete_rate_rows(
            conn,
            pair=normalized_pair,
            source=normalized_source,
        )
        deleted_checked_minutes = _delete_checked_rate_minutes(
            conn,
            pair=normalized_pair,
            source=normalized_source,
        )
        sync_summary = sync_rates(
            conn,
            pair=normalized_pair,
            days=days_int,
            source=normalized_source,
            path=path,
            commit=False,
        )
    except Exception:
        conn.execute("ROLLBACK TO SAVEPOINT rates_rebuild")
        conn.execute("RELEASE SAVEPOINT rates_rebuild")
        raise
    conn.execute("RELEASE SAVEPOINT rates_rebuild")
    conn.commit()
    return {
        "source": normalized_source,
        "pair": normalized_pair,
        "days": days_int,
        "reprice_transactions": bool(reprice_transactions),
        "deleted": {
            "rates": deleted_rates,
            "checked_minutes": deleted_checked_minutes,
            "transaction_prices": transaction_prices["transactions"],
            "profiles_invalidated": transaction_prices["profiles"],
        },
        "sync": sync_summary,
    }


def fetch_rates_coinbase_exchange(pair, days=30, granularity=60):
    if int(days) <= 0:
        raise AppError("--days must be positive", code="validation")
    granularity_int = int(granularity)
    if granularity_int not in _COINBASE_GRANULARITIES:
        raise AppError(
            f"Coinbase Exchange granularity must be one of {sorted(_COINBASE_GRANULARITIES)}",
            code="validation",
        )
    end = datetime.now(timezone.utc).replace(second=0, microsecond=0)
    start = end - timedelta(days=int(days))
    step = timedelta(seconds=granularity_int * _COINBASE_MAX_CANDLES)
    output = []
    cursor = start
    while cursor < end:
        chunk_end = min(cursor + step, end)
        rows = _coinbase_exchange_candles(
            pair,
            cursor,
            chunk_end,
            granularity=granularity_int,
        )
        output.extend(
            _parse_coinbase_exchange_rows(rows, granularity=granularity_int)
        )
        cursor = chunk_end
    seen = set()
    deduped = []
    for candle in sorted(output, key=lambda item: item["timestamp"]):
        if candle["timestamp"] in seen:
            continue
        seen.add(candle["timestamp"])
        deduped.append(candle)
    return deduped


def _sync_rates_coinbase_exchange(
    conn,
    pair=None,
    days=30,
    source=RATE_SOURCE_COINBASE_EXCHANGE,
    commit=True,
):
    if int(days) <= 0:
        raise AppError("--days must be positive", code="validation")
    if pair:
        pairs = [require_supported_pair(pair)]
    else:
        pairs = list(SUPPORTED_RATE_PAIRS)
    fetched_at = _iso_z(datetime.now(timezone.utc))
    summary = []
    needed_by_pair = _collect_coinbase_needed_minutes(conn, pairs)
    has_any_needed_minutes = any(needed_by_pair.get(pair) for pair in pairs)
    for normalized_pair in pairs:
        needed_minutes = needed_by_pair.get(normalized_pair, set())
        filter_result = _filter_missing_coinbase_minutes(
            conn,
            normalized_pair,
            needed_minutes,
        )
        missing_minutes = filter_result["missing"]
        windows = _coinbase_windows_for_close_minutes(missing_minutes, granularity=60)
        inserted = 0
        checked_minutes = 0
        mode = "transaction_need"
        if needed_minutes:
            for start, end in windows:
                rows = _coinbase_exchange_candles(
                    normalized_pair,
                    start,
                    end,
                    granularity=60,
                )
                samples = _parse_coinbase_exchange_rows(rows, granularity=60)
                for candle in samples:
                    upsert_rate(
                        conn,
                        normalized_pair,
                        candle["timestamp"],
                        candle["close"],
                        source,
                        fetched_at=fetched_at,
                        granularity="minute",
                        method="product_candles",
                        open_rate=candle["open"],
                        high_rate=candle["high"],
                        low_rate=candle["low"],
                        close_rate=candle["close"],
                        volume=candle["volume"],
                        trades=candle["trades"],
                    )
                    inserted += 1
                checked_minutes += _mark_rate_minutes_checked(
                    conn,
                    normalized_pair,
                    _coinbase_checked_minutes_for_window(start, end, granularity=60),
                    fetched_at,
                    source=source,
                    granularity="minute",
                    method="product_candles",
                )
                if samples:
                    _invalidate_profile_journals_for_pair(conn, normalized_pair)
                if commit:
                    conn.commit()
        elif not has_any_needed_minutes:
            mode = "continuous_days"
            samples = fetch_rates_coinbase_exchange(
                normalized_pair,
                days=days,
                granularity=60,
            )
            for candle in samples:
                upsert_rate(
                    conn,
                    normalized_pair,
                    candle["timestamp"],
                    candle["close"],
                    source,
                    fetched_at=fetched_at,
                    granularity="minute",
                    method="product_candles",
                    open_rate=candle["open"],
                    high_rate=candle["high"],
                    low_rate=candle["low"],
                    close_rate=candle["close"],
                    volume=candle["volume"],
                    trades=candle["trades"],
                )
                inserted += 1
            if samples:
                _invalidate_profile_journals_for_pair(conn, normalized_pair)
            if commit:
                conn.commit()
        summary.append(
            {
                "pair": normalized_pair,
                "source": source,
                "samples": inserted,
                "days": int(days),
                "granularity": "minute",
                "method": "product_candles",
                "mode": mode,
                "needed_minutes": len(needed_minutes),
                "cached_minutes": len(filter_result["cached"]),
                "already_checked_minutes": len(filter_result["checked"]),
                "missing_minutes": len(missing_minutes),
                "windows": len(windows),
                "checked_minutes": checked_minutes,
                "fetched_at": fetched_at,
            }
        )
    return summary


def _kraken_csv_members(path):
    source_path = Path(path).expanduser()
    if not source_path.exists():
        raise AppError(
            f"Rate source path does not exist: {source_path}",
            code="not_found",
            hint="Pass --path to a Kraken OHLCVT .csv file, .zip archive, or extracted directory",
        )
    if source_path.is_dir():
        for csv_path in sorted(source_path.glob("*.csv")):
            if not csv_path.is_file():
                continue
            with csv_path.open("r", encoding="utf-8", newline="") as handle:
                yield csv_path.name, handle
        return
    if source_path.suffix.lower() == ".zip":
        with zipfile.ZipFile(source_path) as archive:
            for name in sorted(archive.namelist()):
                info = archive.getinfo(name)
                if info.is_dir():
                    continue
                with archive.open(info) as raw:
                    text = io.TextIOWrapper(raw, encoding="utf-8", newline="")
                    yield name, text
    else:
        with source_path.open("r", encoding="utf-8", newline="") as handle:
            yield source_path.name, handle


def _normalize_kraken_pair_code(pair_code):
    code = str(pair_code or "").strip().upper()
    if code.startswith("XXBT"):
        quote = code[4:]
    elif code.startswith("XBT"):
        quote = code[3:]
    else:
        return None, "non_btc"
    if quote.startswith("Z") and quote[1:] in (_KRAKEN_SUPPORTED_QUOTES | _KRAKEN_STABLECOIN_QUOTES):
        quote = quote[1:]
    if quote in _KRAKEN_STABLECOIN_QUOTES:
        return None, "stablecoin_quote"
    if quote not in _KRAKEN_SUPPORTED_QUOTES:
        return None, "unsupported_quote"
    normalized = f"BTC-{quote}"
    if normalized not in SUPPORTED_RATE_PAIRS:
        return None, "unsupported_pair"
    return normalized, None


def _kraken_member_pair(member_name):
    base = Path(member_name).name
    if not base.lower().endswith(".csv"):
        return None, "not_csv"
    stem = base[:-4]
    pair_code, sep, interval = stem.rpartition("_")
    if not sep or not pair_code or not interval:
        return None, "invalid_filename"
    if interval != "1":
        return None, "unsupported_interval"
    return _normalize_kraken_pair_code(pair_code)


def _parse_kraken_csv_row(row, member_name, line_number):
    if len(row) != 7:
        raise AppError(
            f"Expected 7 columns, got {len(row)}",
            code="validation",
        )
    timestamp_raw, open_raw, high_raw, low_raw, close_raw, volume_raw, trades_raw = [
        cell.strip() for cell in row
    ]
    try:
        timestamp_seconds = int(timestamp_raw)
        timestamp = _iso_z(datetime.fromtimestamp(timestamp_seconds, tz=timezone.utc))
        close_timestamp = _iso_z(
            datetime.fromtimestamp(timestamp_seconds + 60, tz=timezone.utc)
        )
        open_value = pricing.decimal_from_exact(open_raw)
        high_value = pricing.decimal_from_exact(high_raw)
        low_value = pricing.decimal_from_exact(low_raw)
        close_value = pricing.decimal_from_exact(close_raw)
        volume_value = pricing.decimal_from_exact(volume_raw)
        trades = int(trades_raw)
    except Exception as exc:
        raise AppError(f"{member_name}:{line_number}: invalid row values", code="validation") from exc
    if not open_value or not high_value or not low_value or not close_value:
        raise AppError(f"{member_name}:{line_number}: OHLC prices must be positive", code="validation")
    if volume_value is None or volume_value < 0:
        raise AppError(f"{member_name}:{line_number}: volume must be non-negative", code="validation")
    if trades < 0:
        raise AppError(f"{member_name}:{line_number}: trades must be non-negative", code="validation")
    return {
        "timestamp": timestamp,
        "close_timestamp": close_timestamp,
        "open": open_raw,
        "high": high_raw,
        "low": low_raw,
        "close": close_raw,
        "volume": volume_raw,
        "trades": trades,
    }


def _flush_kraken_batch(conn, batch):
    if not batch:
        return
    conn.executemany(_RATE_UPSERT_SQL, batch)


def _sync_rates_kraken_csv(conn, pair=None, path=None, commit=True):
    if not path:
        raise AppError(
            "--path is required for --source kraken-csv",
            code="validation",
            hint="Download Kraken's OHLCVT archive yourself and pass the local .zip or .csv path",
        )
    pair_filter = require_supported_pair(pair) if pair else None
    fetched_at = _iso_z(datetime.now(timezone.utc))
    summaries = {}
    skipped_files = 0

    for member_name, handle in _kraken_csv_members(path):
        normalized_pair, skip_reason = _kraken_member_pair(member_name)
        if skip_reason:
            skipped_files += 1
            logger.info("Skipping Kraken CSV member %s: %s", member_name, skip_reason)
            continue
        if pair_filter and normalized_pair != pair_filter:
            skipped_files += 1
            continue

        summary = summaries.setdefault(
            normalized_pair,
            {
                "pair": normalized_pair,
                "source": RATE_SOURCE_KRAKEN_CSV,
                "samples": 0,
                "rows": 0,
                "files": 0,
                "skipped_rows": 0,
                "skipped_files": 0,
                "first_timestamp": None,
                "last_timestamp": None,
                "granularity": "minute",
                "method": "ohlcvt_csv",
                "fetched_at": fetched_at,
            },
        )
        summary["files"] += 1
        batch = []
        reader = csv.reader(handle)
        for line_number, row in enumerate(reader, start=1):
            try:
                candle = _parse_kraken_csv_row(row, member_name, line_number)
            except AppError as exc:
                summary["skipped_rows"] += 1
                logger.warning("Skipping Kraken CSV row: %s", exc)
                continue
            summary["samples"] += 1
            summary["rows"] += 1
            ts = candle["close_timestamp"]
            if summary["first_timestamp"] is None or ts < summary["first_timestamp"]:
                summary["first_timestamp"] = ts
            if summary["last_timestamp"] is None or ts > summary["last_timestamp"]:
                summary["last_timestamp"] = ts
            batch.append(
                _rate_insert_params(
                    normalized_pair,
                    ts,
                    candle["close"],
                    RATE_SOURCE_KRAKEN_CSV,
                    fetched_at,
                    "minute",
                    "ohlcvt_csv",
                    open_rate=candle["open"],
                    high_rate=candle["high"],
                    low_rate=candle["low"],
                    close_rate=candle["close"],
                    volume=candle["volume"],
                    trades=candle["trades"],
                )
            )
            if len(batch) >= _KRAKEN_BATCH_SIZE:
                _flush_kraken_batch(conn, batch)
                batch.clear()
        _flush_kraken_batch(conn, batch)

    for summary in summaries.values():
        summary["skipped_files"] = skipped_files
        if summary["samples"]:
            _invalidate_profile_journals_for_pair(conn, summary["pair"])
    if commit:
        conn.commit()
    return [summaries[pair] for pair in sorted(summaries)]


def sync_rates(
    conn,
    pair=None,
    days=30,
    source=RATE_SOURCE_COINBASE_EXCHANGE,
    path=None,
    commit=True,
):
    normalized_source = str(source or "").strip().lower()
    if normalized_source == RATE_SOURCE_COINBASE_EXCHANGE:
        if path:
            raise AppError(
                "--path is only supported for --source kraken-csv",
                code="validation",
            )
        return _sync_rates_coinbase_exchange(
            conn,
            pair=pair,
            days=days,
            source=normalized_source,
            commit=commit,
        )
    if normalized_source == RATE_SOURCE_COINGECKO:
        if path:
            raise AppError(
                "--path is only supported for --source kraken-csv",
                code="validation",
            )
        return _sync_rates_coingecko(
            conn,
            pair=pair,
            days=days,
            source=normalized_source,
            commit=commit,
        )
    if normalized_source == RATE_SOURCE_KRAKEN_CSV:
        return _sync_rates_kraken_csv(conn, pair=pair, path=path, commit=commit)
    raise AppError(
        f"Unknown rate source '{source}'",
        code="validation",
        hint=f"Supported sources: {', '.join(SUPPORTED_RATE_SOURCES)}",
    )


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
    "RATE_SOURCE_COINBASE_EXCHANGE",
    "RATE_SOURCE_COINGECKO",
    "RATE_SOURCE_KRAKEN_CSV",
    "SUPPORTED_RATE_PAIRS",
    "SUPPORTED_RATE_SOURCES",
    "fetch_rates_coinbase_exchange",
    "fetch_rates_coingecko",
    "get_cached_rate_at_or_before",
    "get_latest_rate",
    "get_rate_range",
    "list_cached_pairs",
    "rate_pair_parts",
    "rebuild_rates_cache",
    "require_supported_pair",
    "set_manual_rate",
    "sync_rates",
    "transaction_price_missing_sql",
    "transaction_price_missing_sql_unqualified",
    "transaction_rate_pair",
    "upsert_rate",
]
