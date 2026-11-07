"""Repro: `kassiber chat --file` never attaches the file.

`_stage_chat_attachments` (kassiber/cli/chat.py) stages each `--file` with
`internal.document_import.stage` and then reads the grant from
``data["token"]``, but the daemon returns it as ``data["document_token"]``.
Every `--file` therefore fails with `ai_attachment_failed` ("could not attach
the file to this chat") after the daemon already staged it. The Rust CLI
reads `document_token` (crates/cli/src/chat/mod.rs). This script stages a
real file with a real Python daemon on a disposable data root and feeds the
reply to the CLI's own staging function. Exits non-zero while the bug lasts.

    uv run --locked python tests/rust_port/repro_chat_file_token_key.py
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


class _Client:
    """The two calls `_stage_chat_attachments` makes, on a real daemon."""

    def __init__(self, process: subprocess.Popen) -> None:
        self._process = process

    def send(self, request: dict) -> None:
        self._process.stdin.write(json.dumps(request) + "\n")
        self._process.stdin.flush()

    def read(self) -> dict:
        return json.loads(self._process.stdout.readline())


def main() -> int:
    from kassiber.cli import chat
    from kassiber.errors import AppError

    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        root = base / "data"
        export = base / "export.csv"
        export.write_text("time,amount\n2025-01-01,1\n")
        env = {
            **os.environ,
            "PYTHONPATH": str(ROOT),
            "KASSIBER_OFFLINE_PREFERENCE_FILE": str(base / "offline-mode.json"),
        }
        env.pop("XDG_DATA_HOME", None)
        init = [sys.executable, "-m", "kassiber", "--machine", "--data-root", str(root)]
        for command in (["init"], ["workspaces", "create", "Personal"],
                        ["profiles", "create", "--workspace", "Personal", "--fiat-currency", "EUR", "Main"]):
            subprocess.run([*init, *command], env=env, check=True, capture_output=True, timeout=300)
        process = subprocess.Popen(
            [*init, "daemon"], env=env, cwd=ROOT, text=True,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        )
        try:
            client = _Client(process)
            client.read()  # daemon.ready
            try:
                grants = chat._stage_chat_attachments(client, SimpleNamespace(file=[str(export)], file_context=None))
                outcome = f"attached: {grants}"
                failed = False
            except AppError as exc:
                outcome = f"{exc.code}: {exc}"
                failed = exc.code == "ai_attachment_failed"
        finally:
            process.stdin.close()
            process.wait(timeout=60)
    print(outcome)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
