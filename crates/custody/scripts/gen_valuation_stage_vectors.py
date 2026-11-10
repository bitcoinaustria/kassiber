#!/usr/bin/env python3
"""Freeze whole finalized-stage results for books with valuation holds.

Each case is a real-writer book (rounds 5 and 8: invalid stored prices,
unusable cached valuations, the 1e19 limit) loaded into its custody snapshot,
with Python's finalized projection. Valuation holds recompute tax eligibility,
so later rows in the held pool stop at the basis barrier while keeping their
own price causes. Books live in a disposable directory
under the system temporary directory (point TMPDIR at the test root):

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/custody/scripts/gen_valuation_stage_vectors.py [--check]
"""
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.core.custody_compute import compute_custody_projection  # noqa: E402
from kassiber.core.custody_journal import CustodyJournalBuilder  # noqa: E402
from kassiber.db import open_db  # noqa: E402
from tests.custody_snapshot import to_jsonable  # noqa: E402
from tests.test_custody_authored_migration import NOW, _scope, _tx  # noqa: E402

OUT = ROOT / "tests/fixtures/custody_projection_valuation_stage.json"
def price(conn, tx, field, value):
    # A nonfinite legacy mirror stays NULL: JSON snapshots carry no Infinity.
    mirror = None if value is None or value == "Infinity" else float(value)
    conn.execute(f"UPDATE transactions SET {field}=?, {field}_exact=? WHERE id=?",
                 (mirror, value, tx))


def basis_books():
    """Pyfix review price_q.py: an invalid sale must not hide later causes."""
    for bad in (None, "1e-14", "1e40", "1e19", "9999999999999999999.5", "Infinity"):
        def build(conn, bad=bad):
            _tx(conn, "buy", "btc", "inbound", "BTC", 300000000000, "2025-01-01T00:00:00Z")
            _tx(conn, "out", "btc", "outbound", "BTC", 50000000000, NOW)
            _tx(conn, "sell-missing", "btc", "outbound", "BTC", 50000000000, "2026-02-01T00:00:00Z")
            _tx(conn, "sell-ok", "btc", "outbound", "BTC", 50000000000, "2026-03-01T00:00:00Z")
            _tx(conn, "receipt-missing", "btc", "inbound", "BTC", 50000000000, "2026-04-01T00:00:00Z")
            _tx(conn, "liquid-ok", "liquid", "inbound", "LBTC", 1000, "2026-05-01T00:00:00Z")
            for tx in ("buy", "sell-ok", "liquid-ok"):
                price(conn, tx, "fiat_rate", "50000")
            price(conn, "out", "fiat_rate", bad)
        yield f"basis_{bad}", build

    def derived(conn):
        _tx(conn, "prior", "btc", "inbound", "BTC", 100000000, "2025-01-01T00:00:00Z")
        _tx(conn, "out", "btc", "outbound", "BTC", 100000, NOW)
        _tx(conn, "later", "btc", "outbound", "BTC", 100000, "2027-01-01T00:00:00Z")
        for tx in ("prior", "later"):
            price(conn, tx, "fiat_rate", "100000")
        price(conn, "out", "fiat_rate", "1e-13")
    yield "derived_value_floor", derived

    def derived_rate(conn):
        derived(conn)
        price(conn, "out", "fiat_rate", None)
        price(conn, "out", "fiat_value", "1e18")
    yield "derived_rate_limit", derived_rate

    def cached(conn):
        derived(conn)
        price(conn, "out", "fiat_rate", None)
        conn.execute("UPDATE transactions SET pricing_method='unsupported_price_range', pricing_quality='missing' WHERE id='out'")
    yield "cached_rate_range", cached

    def unaffected(conn):
        derived(conn)
        price(conn, "out", "fiat_rate", "100000")
    yield "priced_control", unaffected


def cases(root):
    for name, build in basis_books():
        conn = open_db(root / name)
        try:
            _scope(conn)
            build(conn)
            conn.commit()
            profile = dict(conn.execute("SELECT * FROM profiles").fetchone())
            snapshot = CustodyJournalBuilder(conn, profile).load_snapshot()
            # The load clock is the only time not fixed by the book.
            snapshot["as_of"] = "2026-11-06T00:00:00.000000Z"
        finally:
            conn.close()
        try:
            result = compute_custody_projection(snapshot).finalized_tax_projection
            expected = to_jsonable(result, redact_config=False)
        except Exception as exc:
            expected = {"error": getattr(exc, "code", type(exc).__name__)}
        yield {"case": name, "input": snapshot, "expected": expected}


def render():
    with tempfile.TemporaryDirectory(prefix="valuation-stage-") as root:
        return json.dumps(list(cases(Path(root))), ensure_ascii=True, sort_keys=True) + "\n"


if __name__ == "__main__":
    text = render()
    if "--check" in sys.argv:
        raise SystemExit(0 if OUT.read_text() == text else "valuation stage vectors drifted")
    OUT.write_text(text)
    print(f"{len(json.loads(text))} valuation stage cases")
