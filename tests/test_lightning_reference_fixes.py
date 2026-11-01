"""Lightning regressions discovered by the Rust reference comparisons."""

from dataclasses import replace
from decimal import Decimal, localcontext
import importlib

import pytest

from kassiber.core.lightning import build_profitability_report, profitability_csv_rows
from tests.test_lightning_scaffold import _snapshot_with_routing


def test_deep_lightning_config_uses_recoverable_malformed_config_path(tmp_path):
    from kassiber.core.lightning import resolve_lightning_connection
    from kassiber.db import open_db
    from tests.test_source_overlap import _seed_book, _wallet

    conn = open_db(tmp_path / "data")
    try:
        _seed_book(conn)
        _wallet(conn, "node", "Node", "coreln", {})
        depth = 10_000
        conn.execute("UPDATE wallets SET config_json=?", ("[" * depth + "0" + "]" * depth,))
        connection = resolve_lightning_connection(conn, "node")
        assert connection["config"] == {}
        assert connection["backend_name"] is None
    finally:
        conn.close()


@pytest.mark.parametrize("existing", [False, True])
def test_csv_encoding_failure_preserves_destination(tmp_path, monkeypatch, existing):
    cli = importlib.import_module("kassiber.cli.main")
    snapshot = _snapshot_with_routing()
    snapshot = replace(snapshot, channels=(replace(snapshot.channels[0], peer_alias="bad\ud800"),))
    monkeypatch.setattr(cli, "_cli_build_lightning_snapshot", lambda *a, **kw: ({"id": "node"}, snapshot))
    path = tmp_path / "profit.csv"
    if existing:
        path.write_bytes(b"previous artifact")
    with pytest.raises(UnicodeEncodeError):
        cli._cli_export_lightning_profitability_csv(None, "node", str(path), window_days=30)
    assert list(tmp_path.iterdir()) == ([path] if existing else [])
    if existing:
        assert path.read_bytes() == b"previous artifact"


def test_csv_publish_failure_preserves_destination_and_removes_staging(tmp_path, monkeypatch):
    cli = importlib.import_module("kassiber.cli.main")
    monkeypatch.setattr(cli, "_cli_build_lightning_snapshot", lambda *a, **kw: ({"id": "node"}, _snapshot_with_routing()))
    path = tmp_path / "profit.csv"
    path.write_bytes(b"previous artifact")

    def refuse(*args):
        raise OSError("synthetic publication refusal")

    monkeypatch.setattr("os.replace", refuse)
    with pytest.raises(OSError, match="synthetic publication refusal"):
        cli._cli_export_lightning_profitability_csv(None, "node", str(path), window_days=30)
    assert path.read_bytes() == b"previous artifact"
    assert list(tmp_path.iterdir()) == [path]


@pytest.mark.parametrize("amount", [900719925474099301, -900719925474099301, 10**40 + 1])
def test_profitability_csv_preserves_exact_sat_in_every_btc_column(amount):
    snapshot = _snapshot_with_routing()
    snapshot = replace(
        snapshot,
        routing=replace(snapshot.routing, routing_revenue_sat=amount),
        channels=(replace(snapshot.channels[0], earned_routing_sat=amount),),
    )
    report = build_profitability_report(
        connection_id="node", connection_label="Node", connection_kind="lnd", snapshot=snapshot,
    )
    with localcontext() as ctx:
        ctx.prec = 100
        for row in profitability_csv_rows(report):
            if row[3] and row[0] != "section":
                assert Decimal(row[3]) * 100_000_000 == Decimal(row[2])
