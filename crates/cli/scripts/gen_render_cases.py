"""Write tests/data/render_cases.txt: Python's `emit` in every format.

Each line holds a payload (JSON) and what `kassiber.envelope.emit` writes
for it with `--format table|plain|csv|json`, or the error it raises.

    .venv/bin/python crates/cli/scripts/gen_render_cases.py
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TARGET = ROOT / "crates" / "cli" / "tests" / "data" / "render_cases.txt"
sys.path.insert(0, str(ROOT))

from kassiber.envelope import emit  # noqa: E402
from kassiber.errors import AppError  # noqa: E402

PAYLOADS = [
    {
        "version": "0.22.78", "schema_version": 1,
        "auth": {"mode": "local", "authenticated": True},
        "database_encrypted": False, "data_root": "/tmp/x", "project_id": None,
        "current_workspace": "Personal", "workspaces": 1, "default_backend": None,
    },
    [{"id": 1, "label": "a,b", "note": "say \"hi\"", "rate": 1.5}, {"id": 22, "label": "c\nd", "rate": 0.1}],
    [{"a": 1}, {"b": 2}],
    [],
    {},
    {"amount": 0.00000001, "big": 1e16, "neg": -2.5, "zero": 0.0, "inf": float("inf")},
    {"nested": [1, {"x": None}], "text": "café €", "flag": False},
    "plain text",
    None,
    [1, "two", None],
    {"empty": ""},
    [{"only": ""}],
]


def main() -> int:
    lines = []
    for payload in PAYLOADS:
        for fmt in ("table", "plain", "csv", "json"):
            out = io.StringIO()
            args = argparse.Namespace(format=fmt, output=None, command="status")
            try:
                with contextlib.redirect_stdout(out):
                    emit(args, payload, kind="status")
                result = {"ok": out.getvalue()}
            except AppError as exc:
                result = {"error": exc.code, "message": str(exc)}
            except Exception as exc:  # main() reports these as internal_error
                result = {"error": "internal_error", "message": str(exc)}
            lines.append(json.dumps({"payload": payload, "format": fmt, "out": result}, ensure_ascii=True))
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text("\n".join(lines) + "\n", encoding="ascii")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
