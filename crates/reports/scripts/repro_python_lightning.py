#!/usr/bin/env python3
"""Reproduce retained profitability/CSV reference quirks on disposable data."""
from __future__ import annotations

from decimal import Decimal
import importlib
from pathlib import Path
import tempfile
from unittest.mock import patch

from kassiber.core.lightning import profitability_csv_rows, resolve_lightning_connection
from kassiber.daemon import _coerce_int
from kassiber.db import open_db
from gen_lightning_vectors import (
    CONNECTION, ROOT, SCRATCH, SEED, make_report, report_specs, snapshot,
)


def run():
    SCRATCH.mkdir(parents=True, exist_ok=True)
    specs = {name: (raw, cost) for name, raw, cost in report_specs()}
    # CSV uses binary float division even though the sat integer remains exact.
    raw, cost = specs["amount-900719925474099301-1"]
    value = int(raw["routing"]["routing_revenue_sat"])
    rendered = profitability_csv_rows(make_report(raw, cost))[1][3]
    assert rendered == "9007199254.74099350"
    assert rendered != format(Decimal(value) / Decimal(100_000_000), ".8f")
    # The adapter's net is authoritative, including a discrepancy with totals.
    raw, cost = specs["authoritative-net"]
    assert make_report(raw, cost).to_envelope_payload()["summary"]["netProfitSat"] == -98765
    # No routing window becomes a zero summary while channel earnings survive.
    raw, cost = specs["no-routing"]
    payload = make_report(raw, cost).to_envelope_payload()
    assert payload["summary"]["routingRevenueSat"] == 0 and payload["channels"][1]["earnedRoutingSat"] == 2500
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
        partial = path.read_bytes()
        assert partial.startswith(b"section,key,value_sat,value_btc,detail\r\n")
        assert b"old artifact" not in partial and b"channel," not in partial
        conn = open_db(Path(directory) / "data")
        try:
            conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + SEED)
            conn.execute("UPDATE wallets SET config_json=? WHERE id='node'", ("["*1100 + "0" + "]"*1100,))
            try:
                resolve_lightning_connection(conn, "node")
            except RecursionError as exc:
                assert "while decoding a JSON array" in str(exc)
            else:
                raise AssertionError("Python masked config RecursionError")
        finally:
            conn.close()
    try:
        _coerce_int(float("inf"), default=30, minimum=1, maximum=365)
    except OverflowError as exc:
        assert str(exc) == "cannot convert float infinity to integer"
    else:
        raise AssertionError("Python defaulted an infinite routing window")
    print("8 Lightning reference-quirk assertions passed; no product route acceptance claimed")


if __name__ == "__main__":
    run()
