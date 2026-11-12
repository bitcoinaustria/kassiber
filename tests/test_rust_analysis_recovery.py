import json
import os
from pathlib import Path
import select
import shutil
import signal
import sqlite3
import subprocess

import pytest
from sqlcipher3 import dbapi2 as sqlcipher

from tests.rust_sync_env import child_env
from tests.test_rust_analysis_durable import REFERENCE, ROOT, SYNC


SEED = ROOT / "crates/reports/tests/data/privacy_hygiene_seed.sql"
BOOK = ROOT / "crates/reports/tests/data/book.sql"


def connect(path, encrypted):
    codec = sqlcipher if encrypted else sqlite3
    conn = codec.connect(str(path))
    if encrypted:
        conn.execute("PRAGMA key='mirror-kill-fixture'")
    conn.row_factory = codec.Row
    conn.execute("PRAGMA foreign_keys=ON")
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    return conn


def native(request, env):
    executable = os.environ.get("KASSIBER_ANALYSIS_DURABLE_BIN")
    if not executable:
        pytest.skip("requires analysis_durable_fixture test-harness build")
    result = subprocess.run(
        [executable], input=json.dumps(request).encode(), capture_output=True,
        env=env, check=True, timeout=30,
    )
    assert result.stderr == b""
    value = json.loads(result.stdout)
    assert result.stdout == REFERENCE.canonical(value).encode() + b"\n"
    return value


def synchronize_reference(path, encrypted, action):
    conn = connect(path, encrypted)
    try:
        result = REFERENCE.evaluate(
            {"actions": [{"operation": "synchronize", **action}]}, conn
        )[0]
        return {"state": result["result"], "capture": result["state"]}
    finally:
        conn.close()


def pause_and_kill(request, env):
    child = subprocess.Popen(
        [os.environ["KASSIBER_ANALYSIS_DURABLE_BIN"]], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env,
    )
    try:
        child.stdin.write(json.dumps(request).encode())
        child.stdin.close()
        assert select.select([child.stdout], [], [], 30)[0], "native write barrier did not arrive"
        line = child.stdout.readline()
        assert json.loads(line) == {"paused": request["pause"]}, line
        child.kill()
        assert child.wait(timeout=15) == -signal.SIGKILL
        assert child.stderr.read() == b""
    finally:
        if child.poll() is None:
            child.kill()
        child.wait(timeout=15)
        child.stdout.close()
        child.stderr.close()


def remove_book(path):
    for suffix in ("", "-journal", "-wal", "-shm"):
        Path(str(path) + suffix).unlink(missing_ok=True)


@pytest.mark.skipif(os.name != "posix", reason="requires POSIX SIGKILL barriers")
@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("journal", ["DELETE", "WAL"])
def test_native_mirror_every_write_kill_recovers_python_state(tmp_path, encrypted, journal):
    if not os.environ.get("KASSIBER_ANALYSIS_DURABLE_BIN"):
        pytest.skip("requires analysis_durable_fixture test-harness build")
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    corpus = json.loads(SYNC.DEST.read_text())
    selected = {
        "cold-warm-rebuild": ("cold", False, {}),
        "creator-output-invalidates-spender": ("incremental", True, {}),
        "verified-tip-only": ("tip", True, {}),
        "dataset-half-open-expiry": ("expiry", True, {"as_of": "2026-01-02T00:00:00Z"}),
        "repair-dropped-node-table": ("repair", True, {}),
        "clock-without-dirty-sources": ("watermark", True, {}),
        "warm": ("warm", True, {}),
        "rebuild": ("rebuild", True, {"rebuild": True}),
    }
    cold = next(case for case in corpus if case["name"] == "cold-warm-rebuild")
    corpus += [{**cold, "name": "warm"}, {**cold, "name": "rebuild"}]
    summary = []
    phases = set()
    for case in corpus:
        if case["name"] not in selected:
            continue
        name, warm, action = selected[case["name"]]
        original = tmp_path / "original.sqlite"
        expected_path = tmp_path / "expected.sqlite"
        interrupted = tmp_path / "interrupted.sqlite"
        python_restart = tmp_path / "python.sqlite"
        conn = connect(original, encrypted)
        conn.executescript(SEED.read_text())
        conn.executescript("BEGIN;" + BOOK.read_text() + case["sql"] + "COMMIT;")
        conn.execute(f"PRAGMA journal_mode={journal}")
        conn.execute("PRAGMA synchronous=FULL")
        conn.close()
        if warm:
            synchronize_reference(original, encrypted, {})
            conn = connect(original, encrypted)
            for edit in case["actions"][1:-2]:
                if edit["operation"] == "sql":
                    conn.executescript(edit["sql"])
            if name == "repair":
                conn.executescript("DROP TRIGGER chain_index_track_transactions_update;")
            conn.close()
        conn = connect(original, encrypted)
        before = REFERENCE.capture(conn)
        conn.close()
        shutil.copyfile(original, expected_path)
        expected = synchronize_reference(expected_path, encrypted, action)
        request = {
            "database": str(interrupted), "encrypted": encrypted,
            "journal": journal.lower(), **action,
        }
        shutil.copyfile(original, interrupted)
        baseline = native(request, env)
        assert {"state": baseline["state"], "capture": baseline["capture"]} == expected
        assert baseline["writes"] == len(baseline["phases"])
        assert baseline["phases"][-2:] == ["before_commit", "after_commit"]
        phases.update(baseline["phases"])
        print(f"{journal} encrypted={encrypted} {name}: {baseline['writes']} kill positions", flush=True)
        remove_book(interrupted)
        for pause in range(1, baseline["writes"] + 1):
            shutil.copyfile(original, interrupted)
            if pause % 2:
                env["KASSIBER_NO_EGRESS"] = "1"
            else:
                env.pop("KASSIBER_NO_EGRESS", None)
            pause_and_kill({**request, "pause": pause}, env)
            conn = connect(interrupted, encrypted)
            killed = REFERENCE.capture(conn)
            conn.close()
            expected_killed = expected["capture"] if pause == baseline["writes"] else before
            assert killed == expected_killed, (name, pause, baseline["phases"][pause - 1])
            shutil.copyfile(interrupted, python_restart)
            resumed = native(request, env)
            assert REFERENCE.canonical({
                "state": resumed["state"], "capture": resumed["capture"]
            }) == REFERENCE.canonical(expected), (name, pause)
            assert synchronize_reference(python_restart, encrypted, action) == expected, (
                name, pause, "python restart"
            )
            remove_book(interrupted)
            remove_book(python_restart)
        print(f"{journal} encrypted={encrypted} {name}: complete", flush=True)
        summary.append({
            "scenario": name, "kills": baseline["writes"],
            "phases": sorted(set(baseline["phases"])),
            "tables": expected["capture"]["table_count"],
        })
        remove_book(original)
        remove_book(expected_path)
    assert phases == {
        "installation", "source_triggers", "generation", "contributions",
        "materialization", "labels", "before_commit", "after_commit",
    }
    assert len(summary) == 8
    destination = Path(os.environ.get("KASSIBER_MIRROR_RECOVERY_SUMMARY", str(tmp_path)))
    destination.mkdir(parents=True, exist_ok=True)
    (destination / f"{journal.lower()}-{encrypted}.json").write_text(
        json.dumps(summary, indent=2) + "\n"
    )
