"""Repro: a terminal Ctrl-C ends `kassiber chat`'s daemon child.

`_DaemonChatClient` starts `python -m kassiber ... daemon` with
`subprocess.Popen` and no `start_new_session` or `process_group`, so the
child shares the chat's process group. A terminal delivers Ctrl-C as
SIGINT to the whole foreground group: the chat catches its
`KeyboardInterrupt` to send `ai.chat.cancel`, but the daemon child gets
SIGINT too, has no handler, and dies with an uncaught `KeyboardInterrupt`
in its request loop. The cooperative cancel then meets a dead daemon
(`BrokenPipeError` or `daemon_closed`) instead of a cancelled turn.

The Rust port starts the child in its own process group, so only the chat
sees the terminal's Ctrl-C (crates/cli/src/chat/client.rs).

    uv run --locked python tests/rust_port/repro_chat_ctrl_c_kills_daemon.py

Exit 0 means the bug reproduced: the child shared the group and died.
"""
from __future__ import annotations

import os
from pathlib import Path
import signal
import sys
import tempfile
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    # Lead a process group of our own, as a terminal's foreground job does,
    # so the group's SIGINT below reaches only this process and its child.
    os.setpgid(0, 0)
    with tempfile.TemporaryDirectory(prefix="kassiber-repro-chat-") as tmp:
        base = Path(tmp).resolve()
        # The child inherits this environment: a disposable app-data and
        # state root, never the account's.
        for key in ("XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME"):
            os.environ[key] = str(base / key.lower())
        os.environ["KASSIBER_STATE_ROOT"] = str(base / "state")
        os.environ["KASSIBER_DISABLE_UPDATE_CHECK"] = "1"

        from kassiber.cli import chat
        from kassiber.db import open_db

        data_root = base / "data"
        open_db(data_root).close()
        received = []
        # This process plays the chat's part: it takes the terminal's SIGINT.
        signal.signal(signal.SIGINT, lambda *_: received.append("SIGINT"))
        args = SimpleNamespace(
            data_root=str(data_root),
            env_file=None,
            non_interactive=True,
            db_passphrase_fd=None,
            timeout_seconds=30,
        )
        client = chat._DaemonChatClient(args)
        child = client._proc
        try:
            same_group = os.getpgid(child.pid) == os.getpgrp()
            print(f"daemon child shares the chat's process group: {same_group}")
            # What a terminal does on Ctrl-C: SIGINT to the foreground group.
            os.killpg(os.getpgrp(), signal.SIGINT)
            deadline = time.monotonic() + 10
            while child.poll() is None and time.monotonic() < deadline:
                time.sleep(0.05)
            status = child.poll()
            print(f"chat saw: {received}; daemon child exit status: {status}")
            try:
                chat._send_cancel(client, "chat-x")
                record = client.read()
                print(f"cancel answered: {record.get('kind')}")
            except Exception as exc:  # noqa: BLE001 - the repro reports it
                print(f"cancel failed: {type(exc).__name__}: {exc}")
            return 0 if same_group and status is not None else 1
        finally:
            try:
                client.close()
            except BrokenPipeError:
                # close() itself fails on the dead child's stdin.
                print("close failed: BrokenPipeError")


if __name__ == "__main__":
    raise SystemExit(main())
