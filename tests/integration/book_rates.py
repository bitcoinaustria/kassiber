"""Refresh or clear the demo book's latest market rate
(scripts/integration-harness.sh `demo_refresh_live_rate`).

With an explicit live source (`KASSIBER_DEMO_LIVE_SOURCE`), the product
fetches that provider's latest quote (`ui.rates.latest`), retrying for a
minute. Without one the demo keeps no provider's "latest" quote, so the book
prices only from the bundled history. Either way it drops the provider
selection mempool leaves behind.
"""

from __future__ import annotations

import os
import time

from tests.integration.book import (
    LIVE_MARKET_RATE_SOURCES,
    MARKET_RATE_PROVIDER_SETTING,
    RATE_SOURCE_MEMPOOL,
    latest_rate,
    open_book,
)


def main() -> int:
    data_root = os.environ["KASSIBER_DEMO_DATA_ROOT"]
    pair = os.environ.get("KASSIBER_DEMO_PAIR") or "BTC-EUR"
    live_source = os.environ.get("KASSIBER_DEMO_LIVE_SOURCE") or ""
    if live_source:
        deadline = time.monotonic() + 60
        while True:
            try:
                latest_rate(data_root, pair, live_source)
                break
            except RuntimeError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(2)
    conn = open_book(data_root)
    try:
        if not live_source:
            for source in LIVE_MARKET_RATE_SOURCES:
                conn.execute(
                    "DELETE FROM rates_cache WHERE pair = ? AND source = ? AND granularity = 'latest'",
                    (pair, source),
                )
        conn.execute(
            "DELETE FROM settings WHERE key = ? AND value = ?",
            (MARKET_RATE_PROVIDER_SETTING, RATE_SOURCE_MEMPOOL),
        )
        conn.commit()
    finally:
        conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
