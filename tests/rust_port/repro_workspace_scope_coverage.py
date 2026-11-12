"""Disposable repro: scoped next-actions reads the active book's price coverage.

The native port preserves this Python behavior. No Python product code changes.
Run with TMPDIR under the worktree's target/.
"""
from pathlib import Path
import tempfile

from kassiber.core import ui_snapshot as ui
from kassiber.db import open_db
from crates.workspace.scripts.export_vectors import BASE, CURRENT


def main():
    with tempfile.TemporaryDirectory(prefix="workspace-scope-repro-") as directory:
        conn = open_db(Path(directory) / "data")
        conn.executescript(BASE + CURRENT)
        scoped = ui.build_next_actions_snapshot(conn, workspace_id="w", profile_id="foreign")
        active_missing = ui.build_rates_coverage_snapshot(conn)["summary"]["missing_price_transactions"]
        conn.execute("UPDATE settings SET value='foreign' WHERE key='context_profile'")
        requested_missing = ui.build_rates_coverage_snapshot(conn)["summary"]["missing_price_transactions"]
        conn.close()
        assert scoped["health"]["profile"]["id"] == "foreign"
        assert active_missing > 0 and requested_missing == 0
        assert any(s["id"] == "fetch_missing_prices" for s in scoped["suggestions"])
        print("Python reference bug reproduced: scoped next-actions uses active-book coverage.")


if __name__ == "__main__":
    main()
