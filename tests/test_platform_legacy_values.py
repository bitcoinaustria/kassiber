"""Platform CSV regressions recorded by the Rust artifact port."""

from datetime import datetime, timezone
from types import SimpleNamespace

from kassiber.cli.handlers import _report_hooks
from kassiber.core import transaction_formats as formats
from kassiber.core.asset_presentation import describe_asset
from kassiber.db import open_db
from tests.test_custody_authored_migration import NOW, _scope, _tx


def test_empty_exact_journal_values_use_legacy_cash_values(tmp_path):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _tx(conn, "tx", "btc", "inbound", "BTC", 1000, NOW)
        for kind in ("income", "disposal", "acquisition"):
            conn.execute("INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,transaction_id,entry_type,occurred_at,asset,quantity,fiat_value,fiat_value_exact,proceeds,proceeds_exact,created_at) VALUES(?,'ws','profile','btc','tx',?,?,'BTC',1000,12.5,'',23.5,'',?)", (kind, kind, NOW, NOW))
        assert formats._journal_values(conn, "profile")["tx"] == {"income": 12.5, "proceeds": 23.5, "cost": 12.5}
    finally:
        conn.close()
