"""Python journal pricing skips the documented L-BTC rate alias."""
import sqlite3

from kassiber.cli.handlers import auto_price_transactions_from_rates_cache
from tests.rust_port.journal_prebuild_oracle import rate, seed, transaction


def main():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    try:
        seed(conn)
        rate(conn)
        for asset in ("BTC", "LBTC", "L-BTC"):
            transaction(conn, id=asset, asset=asset)
        count = auto_price_transactions_from_rates_cache(conn, {"id": "p", "fiat_currency": "EUR"})
        actual = [tuple(row) for row in conn.execute("SELECT asset,fiat_rate_exact IS NOT NULL FROM transactions ORDER BY asset")]
        assert actual == [("BTC", 1), ("L-BTC", 0), ("LBTC", 1)]
        assert count == 2
        print("Reproduced: BTC and LBTC are priced; the documented L-BTC alias is skipped.")
    finally:
        conn.close()


if __name__ == "__main__": main()
