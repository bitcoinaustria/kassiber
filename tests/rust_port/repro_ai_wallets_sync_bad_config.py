"""Check: one malformed wallet configuration is a typed AI wallet-sync refusal.

``_wallets_sync_payload`` plans the refresh through
``_freshness_wallet_source_specs``, which ran ``json.loads`` on each wallet's
``config_json`` unguarded. A single wallet whose stored configuration was not
JSON raised ``JSONDecodeError`` for the whole book, so the assistant's
consented ``ui.wallets.sync`` ended as the generic ``tool_error`` with a
traceback on stderr. The planner now refuses with ``invalid_wallet_config``
naming the wallet. The Rust port refuses such books with ``not_implemented``
(the direct-sync reference path) and should adopt the typed code.

    uv run --locked python tests/rust_port/repro_ai_wallets_sync_bad_config.py
"""
from __future__ import annotations

import contextlib
import io
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber import daemon
    from kassiber.db import open_db

    with tempfile.TemporaryDirectory() as directory:
        conn = open_db(Path(directory) / "data")
        conn.executescript(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Main','2025');"
            "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Book','EUR','at','2025');"
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES"
            " ('good','w','p','Good','custom','{}','2025'), ('bad','w','p','Bad','custom','bad','2025');"
            "INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');"
        )

        class Inline:
            def put(self, task):
                task.response.put((True, task.callback(conn)))

        runtime = daemon.AiToolRuntime(str(Path(directory) / "data"), {}, Inline(), {})
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            result = daemon._execute_mutating_ai_tool(daemon.ParsedAiToolCall("c", "ui.wallets.sync", {}), runtime)
        print(f"result: {result}")
        traceback = "JSONDecodeError" in stderr.getvalue() or "Traceback" in stderr.getvalue()
        print(f"traceback on stderr: {traceback}")
        conn.close()
    return 0 if not traceback and "invalid_wallet_config" in repr(result) else 1


if __name__ == "__main__":
    raise SystemExit(main())
