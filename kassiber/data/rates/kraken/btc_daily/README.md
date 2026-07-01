# Kraken Offline History: BTC Daily Values

This directory contains a curated Bitcoin-only offline history subset in Kraken
OHLCVT daily-row format for `BTC-EUR` and `BTC-USD`.

The Kraken portion was copied from a local Kraken data export. It combines the
extracted long-history daily files with the 2023 quarterly archives through
`Kraken_OHLCVT_Q1_2026.zip`, preferring the newer quarterly archive rows where
they overlap. `BTCUSD_Daily_OHLC.csv` was used as a cross-check for the older
USD history; it did not contain timestamps beyond `XBTUSD_1440.csv`.

The pre-Kraken portion was backfilled in the same seven-column daily row shape
from Coin Metrics public BTC `PriceUSD` history. BTC-EUR rows use Coin Metrics
native EUR reference rates when present, otherwise `PriceUSD` divided by the
latest official ECB USD/EUR fixing at or before that date. Synthetic backfill
rows set `open=high=low=close` and `volume=trades=0`.

Coverage:

- `XBTEUR_1440.csv`: cached close timestamps from `2011-01-01` through
  `2026-04-01`.
- `XBTUSD_1440.csv`: cached close timestamps from `2011-01-01` through
  `2026-04-01`.

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
