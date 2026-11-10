"""Python reference repro: MCP analysis-index reads under `query_only`.

The MCP server runs every read with `PRAGMA query_only = ON`
(`kassiber/mcp/tools.py`). `read_index` opens its consistent view with
`chain_analysis_cases.atomic`, i.e. `db.begin_write_savepoint`, which since
470334f61 takes the write lock with `UPDATE settings SET value = value WHERE 0`.
SQLite refused that statement under `query_only`, so `reports_privacy_mirror`,
`chain_analysis_query`, `chain_analysis_entropy` and
`chain_analysis_cases_compare` answered
"AI tool execution failed unexpectedly" at every grant. Round 13 (587bd73d3)
skips the reservation where the connection cannot write, as Rust does. This
publishes the index first, as a book's first analysis read does, then reads
it again under `query_only`; a never-indexed book refuses there by design.

    uv run --locked python tests/rust_port/repro_mcp_analysis_index_query_only.py
"""
from pathlib import Path
import sqlite3
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber.core.chain_analysis.projection import read_index
    from kassiber.db import open_db

    with tempfile.TemporaryDirectory(prefix="kassiber-mcp-query-only-") as directory:
        conn = open_db(Path(directory) / "data")
        conn.executescript(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');"
            "INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','w','Book','EUR','2025');"
        )
        conn.commit()
        # Publish the index generation; a query_only read cannot.
        with read_index(conn, "p"):
            pass
        conn.commit()
        conn.execute("PRAGMA query_only = ON")
        try:
            with read_index(conn, "p"):
                pass
        except sqlite3.OperationalError as error:
            print(f"read_index under query_only: {error}")
            return 1
        finally:
            conn.close()
    print("read_index reads under query_only")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
