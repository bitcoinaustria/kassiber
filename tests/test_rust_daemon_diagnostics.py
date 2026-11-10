"""ui.logs.snapshot and ui.egress.snapshot: native vs Python daemon sessions.

Both rings are per-process RAM. Wall-clock values (record and ring stamps,
request durations), Python's source file/line and the byte accounting that
depends on them are normalized; everything else must match byte for byte.
"""
from __future__ import annotations

import json
from pathlib import Path
import sqlite3

import pytest

from tests.test_rust_project_desktop import BINARY
from tests.test_rust_workspace_shell import KEY, dump, run, session

pytestmark = pytest.mark.skipif(not BINARY, reason="set KASSIBER_RUST_BIN to the test-harness binary")
STAMP = "<stamp>"


def normalize(line: bytes) -> dict:
    record = json.loads(line)
    data = record.get("data")
    if record.get("kind") == "ui.logs.snapshot" and isinstance(data, dict):
        data["started_at"] = data["buffer_bytes"] = STAMP
        for row in data["records"]:
            row["ts"] = row["file"] = row["line"] = STAMP
            duration = row["fields"].get("duration_ms")
            if duration is not None:
                duration["value"] = STAMP
    if record.get("kind") == "ui.egress.snapshot" and isinstance(data, dict):
        data["started_at"] = STAMP
    return record


def compare(tmp_path, root, requests, key=None, unlock=True):
    database = root / "kassiber.sqlite3"
    before = database.read_bytes()
    rows = dump(database, key)
    unlock = key if unlock else None
    expected = [normalize(line) for line in session(tmp_path, root, requests, reference=True, unlock=unlock)]
    database.write_bytes(before)
    for sidecar in ("-wal", "-shm"):
        Path(str(database) + sidecar).unlink(missing_ok=True)
    actual = [normalize(line) for line in session(tmp_path, root, requests, reference=False, unlock=unlock)]
    assert actual == expected
    assert dump(database, key) == rows
    return actual


def book(tmp_path, key=None):
    root = tmp_path / "book/data"
    env = tmp_path / "book/config/backends.env"
    env.parent.mkdir(parents=True)
    # A dotenv backend joins the runtime allowlist; its token never appears.
    env.write_text("KASSIBER_BACKEND_NODE_KIND=esplora\nKASSIBER_BACKEND_NODE_URL=HTTPS://Node.Example.:8443/api\n"
                   "KASSIBER_BACKEND_NODE_TOKEN=synthetic-egress-token\n")
    argv = ["--machine", "--data-root", str(root)]
    assert run(tmp_path, [*argv, "init"], reference=True).returncode == 0
    conn = sqlite3.connect(root / "kassiber.sqlite3")
    conn.executescript(
        "INSERT INTO backends(name,kind,url,created_at,updated_at) VALUES('core','bitcoinrpc','http://127.0.0.1:8332','2025','2025');"
        "INSERT INTO ai_providers(name,base_url,kind,enabled,api_format,omit_store_field,created_at,updated_at) VALUES"
        "('local','http://127.0.0.1:11434/v1','ollama',1,'chat',0,'2025','2025'),"
        "('remote','https://api.example.com/v1','',1,'chat',0,'2025','2025'),"
        "('socket','unix:///tmp/x','x',1,'chat',0,'2025','2025');")
    conn.commit()
    conn.close()
    # Settle compatibility backfills (e.g. provider display names) first.
    from kassiber.db import open_db
    open_db(root).close()
    if key:
        secret = run(tmp_path, [*argv, "secrets", "init", "--new-passphrase-fd", __import__("tests.cli_runner", fromlist=["InputFd"]).InputFd(key.encode())], reference=True)
        assert secret.returncode == 0, secret.stderr
    return root


REQUESTS = [
    ("ui.egress.snapshot", {}),
    ("ui.logs.snapshot", {}),
    ("status", {}),
    ("ui.workspace.health", {}),
    ("nope", {}),
    ("ui.projects.select", {}),
    ("ui.logs.snapshot", None),
    ("ui.logs.snapshot", {"after_id": 2}),
    ("ui.logs.snapshot", {"after_id": 1, "limit": 1}),
    ("ui.logs.snapshot", {"limit": 0}),
    ("ui.logs.snapshot", {"limit": 2001}),
    ("ui.logs.snapshot", {"after_id": -1}),
    ("ui.logs.snapshot", {"after_id": True}),
    ("ui.logs.snapshot", {"after_id": "1"}),
    ("ui.logs.snapshot", {"after_id": 10**30}),
    ("ui.logs.snapshot", {"zeta": 1, "alpha": 2, "limit": 5}),
    ("ui.logs.snapshot", [1]),
    ("ui.egress.snapshot", {"after_id": 0, "limit": 2000}),
    ("ui.egress.snapshot", {"limit": 1.5}),
    ("ui.egress.snapshot", {"beta": None}),
    ("ui.egress.snapshot", "x"),
    ("daemon.lock", {}),
    ("ui.egress.snapshot", {}),
    ("ui.logs.snapshot", {"after_id": 3}),
]


def test_snapshots_match_python_on_plaintext_book(tmp_path):
    root = book(tmp_path)
    responses = compare(tmp_path, root, REQUESTS)
    egress = responses[0]["data"]
    assert egress["records"] == [] and egress["allowlist_complete"] is False
    assert egress["db_header"]["classification"] == "plaintext-sqlite"
    assert responses[17]["data"]["allowlist_complete"] is True
    assert any(e["label"] == "ai:local" for e in responses[17]["data"]["allowlist"])
    logs = responses[6]["data"]["records"]
    assert [r["msg"] for r in logs][:2] == ["request started", "request finished"]
    assert all(b"synthetic-egress-token" not in json.dumps(r).encode() for r in responses)


def test_snapshots_match_python_on_locked_encrypted_book(tmp_path):
    root = book(tmp_path, key=KEY)
    requests = [r for r in REQUESTS if r[0] != "daemon.lock"]
    responses = compare(tmp_path, root, [("ui.egress.snapshot", {}), ("status", {}), *requests], key=KEY, unlock=False)
    assert responses[0]["data"]["db_header"]["classification"] == "ciphertext-like"
    assert responses[1]["kind"] == "auth_required"
    compare(tmp_path, root, requests, key=KEY)
