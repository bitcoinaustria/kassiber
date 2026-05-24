# Kraken Offline History: BTC Daily Values

This directory contains a curated Bitcoin-only Kraken offline history subset of
OHLCVT daily values for `BTC-EUR` and `BTC-USD`.

The source files were copied from a local Kraken data export. They combine the
extracted long-history daily files with the 2023 quarterly archives through
`Kraken_OHLCVT_Q1_2026.zip`, preferring the newer quarterly archive rows where
they overlap. `BTCUSD_Daily_OHLC.csv` was used as a cross-check for the older
USD history; it did not contain timestamps beyond `XBTUSD_1440.csv`.

Coverage:

- `XBTEUR_1440.csv`: `2013-09-10` through `2026-03-31`, with 5 missing early
  no-candle days.
- `XBTUSD_1440.csv`: `2013-10-06` through `2026-03-31`, with 12 missing early
  no-candle days.

`XBTUSD_1440.csv` was missing `2024-03-31` in the daily archives, so that row
was derived by rolling up Kraken's own `XBTUSD_1.csv` minute candles for that
day.

Import with:

```bash
uv run python -m kassiber rates sync --source kraken-csv --path kassiber/data/rates/kraken/btc_daily
```

Files use Kraken's original OHLCVT row format:

```text
timestamp,open,high,low,close,volume,trades
```

Kassiber stores Kraken candles at their close timestamp. For daily values, that
means a row whose source timestamp is `D 00:00:00Z` is stored at
`D+1 00:00:00Z`. Transaction auto-pricing uses the latest cached timestamp at
or before the transaction time, so this bundled daily fallback behaves like a
prior daily close for intraday transactions and is marked as coarse fallback
pricing that requires review.
