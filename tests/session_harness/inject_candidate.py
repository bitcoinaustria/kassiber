"""A candidate that is the Python reference with one deliberate discrepancy.

`tests/test_cli_session_replay.py` replays recorded sessions against
`python inject_candidate.py <mode> <product args...>` to prove the comparator
fails on each kind of difference, the way `tests/test_tax_shadow_harness.py`
injects one into the shadow comparison. The real product runs as this
script's child; the discrepancy is applied to what the child printed, to its
exit status, or to the book after it exited, by a further child process in
the `subprocess-*` modes. Modes trigger on one command so the rest of the
session stays identical:

- `none`: pass everything through.
- `output-amount`: `transactions list` prints one amount one msat higher.
- `output-id`: `wallets list` prints the two wallets' ids swapped.
- `exit`: `wallets list` exits 3.
- `error-code`, `error-exit`: the duplicate `wallets create`, a caught
  error, reports another error code, or exits 2 instead of 1.
- `book-amount`, `book-link`, `book-timestamp`: after `transactions list`,
  one stored amount moves by one msat, one transaction moves to the other
  wallet, or one occurrence time moves one second before midnight.
- `subprocess-book-amount`: `book-amount`, applied by a grandchild process.
- `daemon-response`: a daemon's answer to `status` gains a key.
"""

from __future__ import annotations

import json
import os
import re
import sqlite3
import subprocess
import sys
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from tests.cli_runner import REFERENCE_PREFIX  # noqa: E402


_UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
_BOOK_EDITS = {
    "book-amount": "UPDATE transactions SET amount = amount + 1 WHERE rowid = (SELECT min(rowid) FROM transactions)",
    "book-link": (
        "UPDATE transactions SET wallet_id = (SELECT id FROM wallets WHERE id != transactions.wallet_id LIMIT 1) "
        "WHERE rowid = (SELECT min(rowid) FROM transactions)"
    ),
    "book-timestamp": (
        "UPDATE transactions SET occurred_at = '2025-12-31T23:59:59Z' "
        "WHERE rowid = (SELECT min(rowid) FROM transactions)"
    ),
}


def _has(args: list[str], *words: str) -> bool:
    return any(args[index : index + len(words)] == list(words) for index in range(len(args)))


def _database(args: list[str]) -> Path:
    data_root = Path(args[args.index("--data-root") + 1])
    return data_root / "kassiber.sqlite3"


def _edit_book(mode: str, args: list[str]) -> None:
    statement = _BOOK_EDITS[mode.removeprefix("subprocess-")]
    if mode.startswith("subprocess-"):
        code = "import sqlite3, sys; c = sqlite3.connect(sys.argv[1]); c.execute(sys.argv[2]); c.commit(); c.close()"
        subprocess.run([sys.executable, "-c", code, str(_database(args)), statement], check=True)
        return
    conn = sqlite3.connect(_database(args))
    try:
        conn.execute(statement)
        conn.commit()
    finally:
        conn.close()


def _alter_output(mode: str, args: list[str], stdout: bytes) -> bytes:
    text = stdout.decode("utf-8")
    if mode == "output-amount" and _has(args, "transactions", "list"):
        text = re.sub(r'"amount_msat": (\d+)', lambda match: f'"amount_msat": {int(match.group(1)) + 1}', text, count=1)
    elif mode == "output-id" and _has(args, "wallets", "list"):
        first, second = list(dict.fromkeys(_UUID.findall(text)))[:2]
        text = text.replace(first, "\0").replace(second, first).replace("\0", second)
    elif mode == "error-code" and '"kind": "error"' in text:
        text = re.sub(r'"code": "[^"]+"', '"code": "validation"', text, count=1)
    return text.encode("utf-8")


def _exit_status(mode: str, args: list[str], returncode: int, stdout: bytes) -> int:
    if mode == "exit" and _has(args, "wallets", "list"):
        return 3
    if mode == "error-exit" and b'"kind": "error"' in stdout:
        return 2
    return returncode


def _daemon(args: list[str]) -> int:
    child = subprocess.Popen([*REFERENCE_PREFIX, *args], stdin=subprocess.PIPE, stdout=subprocess.PIPE, cwd=os.getcwd())

    def relay_in() -> None:
        for line in sys.stdin.buffer:
            child.stdin.write(line)
            child.stdin.flush()
        child.stdin.close()

    threading.Thread(target=relay_in, daemon=True).start()
    for line in child.stdout:
        record = json.loads(line)
        if record.get("kind") == "status" and isinstance(record.get("data"), dict):
            record["data"]["injected"] = True
            line = (json.dumps(record) + "\n").encode("utf-8")
        sys.stdout.buffer.write(line)
        sys.stdout.buffer.flush()
    return child.wait()


def main(argv: list[str]) -> int:
    mode, args = argv[0], argv[1:]
    if mode == "daemon-response" and "daemon" in args:
        return _daemon(args)
    stdin = None if sys.stdin is None or sys.stdin.closed else sys.stdin.buffer.read()
    result = subprocess.run([*REFERENCE_PREFIX, *args], input=stdin, capture_output=True, cwd=os.getcwd())
    stdout = _alter_output(mode, args, result.stdout)
    returncode = _exit_status(mode, args, result.returncode, result.stdout)
    if mode.removeprefix("subprocess-") in _BOOK_EDITS and _has(args, "transactions", "list"):
        _edit_book(mode, args)
    sys.stdout.buffer.write(stdout)
    sys.stderr.buffer.write(result.stderr)
    return returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
