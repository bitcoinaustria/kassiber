#!/usr/bin/env python3
"""Pin the round 8 Python fixes of the former profitability/CSV quirks.

The generated vectors carry the same cases byte for byte; these assertions
name each fixed behaviour on disposable data under this checkout's target.
"""
from __future__ import annotations

from decimal import Decimal
import importlib
from pathlib import Path
import tempfile
from unittest.mock import patch

from kassiber.core.lightning import profitability_csv_rows, resolve_lightning_connection
from kassiber.daemon import _coerce_int
from kassiber.db import open_db
from kassiber.errors import AppError
from gen_lightning_vectors import (
    CONNECTION, ROOT, SCRATCH, SEED, make_report, report_specs, snapshot,
)


def run():
    SCRATCH.mkdir(parents=True, exist_ok=True)
    specs = {name: (raw, cost) for name, raw, cost in report_specs()}
    # 6ddda9c19: the CSV BTC column is the exact satoshi quotient.
    raw, cost = specs["amount-900719925474099301-1"]
    value = int(raw["routing"]["routing_revenue_sat"])
    rendered = profitability_csv_rows(make_report(raw, cost))[1][3]
    assert rendered == format(Decimal(value) / Decimal(100_000_000), ".8f") == "9007199254.74099301"
    # The adapter's net is authoritative, including a discrepancy with totals.
    raw, cost = specs["authoritative-net"]
    assert make_report(raw, cost).to_envelope_payload()["summary"]["netProfitSat"] == -98765
    # 7137ddb02: an absent routing window refuses instead of reporting zeros.
    raw, cost = specs["no-routing"]
    try:
        make_report(raw, cost)
    except AppError as exc:
        assert exc.code == "lightning_routing_unavailable"
    else:
        raise AssertionError("Python reported a zero summary without routing data")
    # 1f596872c: UTF-8 is checked before the destination is touched.
    cli = importlib.import_module("kassiber.cli.main")
    raw, _ = specs["surrogate-alias"]
    with tempfile.TemporaryDirectory(dir=SCRATCH, prefix="repro-") as directory:
        path = Path(directory) / "export.csv"
        path.write_bytes(b"old artifact\n")
        with patch.object(cli, "_cli_build_lightning_snapshot", return_value=(CONNECTION, snapshot(raw))):
            try:
                cli._cli_export_lightning_profitability_csv(None, "node", str(path), window_days=30)
            except UnicodeEncodeError:
                pass
            else:
                raise AssertionError("Python accepted a surrogate CSV alias")
        assert path.read_bytes() == b"old artifact\n"
        # 9ac7b32b8: over-nested saved config is recoverable like malformed config.
        conn = open_db(Path(directory) / "data")
        try:
            conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + SEED)
            conn.execute("UPDATE wallets SET config_json=? WHERE id='node'", ("["*1100 + "0" + "]"*1100,))
            assert resolve_lightning_connection(conn, "node")["config"] == {}
        finally:
            conn.close()
    # 3e74543df: an infinite daemon window falls back to the default.
    assert _coerce_int(float("inf"), default=30, minimum=1, maximum=365) == 30
    print("6 Lightning round 8 assertions passed; no product route acceptance claimed")


if __name__ == "__main__":
    run()
