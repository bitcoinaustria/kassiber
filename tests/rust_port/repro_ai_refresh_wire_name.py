"""Repro: model-initiated AI reads never refresh stale journals.

``_execute_read_only_ai_tool`` decides on the automatic journal refresh with
``call.name in _AI_AUTO_JOURNAL_REFRESH_TOOL_NAMES``. That set holds dotted
catalog names, but providers call tools by their wire names
(``ui_reports_summary``), so only the planner's auto-reads, which use dotted
names, ever refresh. The Rust port keeps this for parity.

    uv run --locked python tests/rust_port/repro_ai_refresh_wire_name.py
"""
from __future__ import annotations

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
            "INSERT INTO settings(key,value) VALUES('context_workspace','w'),('context_profile','p');"
        )
        calls = []
        original = daemon._auto_maintain_for_read
        daemon._auto_maintain_for_read = lambda *args, **kwargs: calls.append(1) or original(*args, **kwargs)

        class Inline:
            def put(self, task):
                task.response.put((True, task.callback(conn)))

        runtime = daemon.AiToolRuntime(str(Path(directory) / "data"), {}, Inline(), {})
        for name in ("ui.reports.summary", "ui_reports_summary"):
            calls.clear()
            result = daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall("c", name, {}), runtime)
            print(f"{name}: ok = {result.get('ok')}, refresh attempted = {bool(calls)}")
        conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
