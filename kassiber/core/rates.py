from __future__ import annotations

import csv
import io
import json
import logging
from pathlib import Path
import zipfile
from datetime import datetime, timezone
from urllib import error as urlerror
from urllib import request as urlrequest

from .. import __version__
from ..db import APP_NAME
from ..errors import AppError
from . import pricing
from ..time_utils import _iso_z, _parse_iso_datetime

logger = logging.getLogger(__name__)

SUPPORTED_RATE_PAIRS = ("BTC-USD", "BTC-EUR")
RATE_SOURCE_COINGECKO = "coingecko"
RATE_SOURCE_KRAKEN_CSV = "kraken-csv"
SUPPORTED_RATE_SOURCES = (RATE_SOURCE_COINGECKO, RATE_SOURCE_KRAKEN_CSV)
_COINGECKO_VS = {"USD": "usd", "EUR": "eur"}
_COINGECKO_COIN = {"BTC": "bitcoin"}
_RATE_ASSET_ALIASES = {"LBTC": "BTC"}
_KRAKEN_STABLECOIN_QUOTES = {"DAI", "USDC", "USDT"}
_KRAKEN_SUPPORTED_QUOTES = {"EUR", "USD"}
_KRAKEN_BATCH_SIZE = 10_000


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


def _sync_rates_coingecko(conn, pair=None, days=30, source=RATE_SOURCE_COINGECKO):
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
        timestamp = _iso_z(datetime.fromtimestamp(int(timestamp_raw), tz=timezone.utc))
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


def _sync_rates_kraken_csv(conn, pair=None, path=None):
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
            ts = candle["timestamp"]
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
    conn.commit()
    return [summaries[pair] for pair in sorted(summaries)]


def sync_rates(conn, pair=None, days=30, source=RATE_SOURCE_COINGECKO, path=None):
    normalized_source = str(source or "").strip().lower()
    if normalized_source == RATE_SOURCE_COINGECKO:
        if path:
            raise AppError(
                "--path is only supported for --source kraken-csv",
                code="validation",
            )
        return _sync_rates_coingecko(conn, pair=pair, days=days, source=normalized_source)
    if normalized_source == RATE_SOURCE_KRAKEN_CSV:
        return _sync_rates_kraken_csv(conn, pair=pair, path=path)
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
    "RATE_SOURCE_COINGECKO",
    "RATE_SOURCE_KRAKEN_CSV",
    "SUPPORTED_RATE_PAIRS",
    "SUPPORTED_RATE_SOURCES",
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
