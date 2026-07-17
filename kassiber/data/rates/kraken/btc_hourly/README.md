# Kraken Offline History: BTC Hourly Values

This directory contains a Bitcoin-only offline history subset in Kraken OHLCVT
hourly-row format for `BTC-EUR` and `BTC-USD`.

Files are plain CSVs, not compressed or obfuscated. They use Kraken's original
seven-column OHLCVT row format:

```text
timestamp,open,high,low,close,volume,trades
```

Build with:

```bash
python3 scripts/build-bundled-hourly-rates.py --kraken-root "/Users/dev/Downloads/Kraken Data"
```

The build script reads:

- `Pre Incremental Updates till 2015/XBTEUR_60.csv`
- `Pre Incremental Updates till 2015/XBTUSD_60.csv`
- `Kraken_OHLCVT_Q1_2023.zip` through `Kraken_OHLCVT_Q1_2026.zip`, plus the
  available Q2-Q4 archives for 2023, 2024, and 2025
- the existing documented daily bundle in `../btc_daily` for the earliest
  pre-Kraken hourly prefix

Rows copied from Kraken's hourly archives keep Kraken's OHLCVT values as-is.
Rows before Kraken's own hourly BTC coverage are expanded from the bundled
daily cache into hourly fallback rows where `open=high=low=close` and
`volume=trades=0`. The daily cache provenance is documented in
`../btc_daily/README.md` and is based on Coin Metrics BTC history plus official
ECB USD/EUR fixings.

Coverage:

- `XBTEUR_60.csv`: 132402 rows, cached close timestamps from `2011-01-01`
  through `2026-04-01`.
- `XBTUSD_60.csv`: 122779 rows, cached close timestamps from `2011-01-01`
  through `2026-04-01`.

The official Kraken downloadable OHLCVT archive page was checked on
`2026-07-06`; its linked quarterly archive folder listed data through
`Kraken_OHLCVT_Q1_2026.zip`, and no Q2 2026 ZIP was available there yet. If a
future `Kraken_OHLCVT_Q2_2026.zip` is placed in the Kraken root, the build
script will include it automatically.

Import with:

```bash
uv run --locked python -m kassiber rates sync --source kraken-csv --path kassiber/data/rates/kraken/btc_hourly
```

Kassiber stores Kraken candles at their close timestamp. For hourly values, a
row whose source timestamp is `H:00:00Z` is stored at `H+1:00:00Z`.
