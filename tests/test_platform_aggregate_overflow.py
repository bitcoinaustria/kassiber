"""Platform CSV regressions recorded by the Rust artifact port."""

from datetime import datetime, timezone
from types import SimpleNamespace

from kassiber.cli.handlers import _report_hooks
from kassiber.core import transaction_formats as formats
from kassiber.core.asset_presentation import describe_asset
from kassiber.db import open_db
from tests.test_custody_authored_migration import NOW, _scope, _tx


def test_platform_export_does_not_sum_unrelated_integer_movements(tmp_path, monkeypatch):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        for ident in ("a", "b"):
            _tx(conn, ident, "btc", "inbound", "BTC", 5_000_000_000_000_000_000, NOW)
        profile = conn.execute("SELECT * FROM profiles").fetchone()
        monkeypatch.setattr(formats, "_require_report_context", lambda *args: SimpleNamespace(profile=profile, workspace={"id": "ws"}))
        monkeypatch.setattr(formats.core_freshness, "require_report_freshness", lambda *args: None)
        result = formats.build_platform_rows(conn, "ws", "profile", _report_hooks())
        assert [row.amount for row in result["rows"]] == ["50000000.00000000"] * 2
    finally:
        conn.close()
