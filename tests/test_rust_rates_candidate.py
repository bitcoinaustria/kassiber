"""Rates slice execution through the hybrid candidate and native daemon."""
from __future__ import annotations

import json
import os
from pathlib import Path
import sys

import pytest

from kassiber.db import open_db
from tests import cli_runner
from tests.test_rust_shell_parity import _environment
from tests.test_rates_incremental import _seed_priced_scope

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")


def _setup(tmp_path, monkeypatch):
    env = _environment(tmp_path)
    env["KASSIBER_HYBRID_RUST_BIN"] = str(Path(RUST_BIN).resolve())
    root = tmp_path / "data"
    result = cli_runner.run(["--data-root", root, "--machine", "init"],
                            reference=True, env=env, capture_output=True)
    assert result.returncode == 0, result.stdout
    conn = open_db(root)
    _seed_priced_scope(conn)
    conn.executemany("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)",
                     [("context_workspace", "ws"), ("context_profile", "profile")])
    conn.execute("INSERT INTO rates_cache(pair,timestamp,rate,rate_exact,source,fetched_at,granularity,method) VALUES('BTC-EUR','2025-12-31T00:00:00Z',123.25,'123.25','manual','2025-12-31T00:00:00Z','exact','manual')")
    conn.commit()
    conn.close()
    monkeypatch.setenv("KASSIBER_TEST_CLI", json.dumps([
        sys.executable, str(cli_runner.ROOT / "tests/rust_port/hybrid_cli.py")]))
    return root, env


def test_all_six_cli_leaves_are_served_by_rust(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    log = tmp_path / "served.jsonl"
    # Preserve the caller's aggregate log; otherwise retain local provenance.
    env.setdefault("KASSIBER_HYBRID_LOG", str(log))
    csv = tmp_path / "XBTEUR_1.csv"
    csv.write_text("1767225600,100,110,90,105.123456789012345678901,1,2\n")

    def run(*args):
        result = cli_runner.run(["--data-root", root, "--machine", "rates", *args],
                                env=env, capture_output=True)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return json.loads(result.stdout)["data"]

    assert len(run("pairs")) == 2
    manual = run("set", "BTC-EUR", "2026-01-01T00:00:00Z", "65000.123456789012345678901")
    assert manual["rate_exact"] == "65000.123456789012345678901"
    assert run("latest", "BTC-EUR")["source"] == "manual"
    assert len(run("range", "BTC-EUR")) == 2
    assert run("sync", "--pair", "BTC-EUR", "--source", "kraken-csv", "--path", str(csv))[0]["samples"] == 1
    rebuilt = run("rebuild", "--pair", "BTC-EUR", "--source", "kraken-csv", "--path", str(csv))
    assert rebuilt["deleted"]["rates"] == 1
    assert rebuilt["sync"][0]["samples"] == 1
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    for command in ("pairs", "set", "latest", "range", "sync", "rebuild"):
        assert {"command": f"rates.{command}", "served_by": "rust"} in calls


def test_native_daemon_reads_and_validation_match_python_bytes(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    requests = [
        ("ui.rates.summary", {}),
        ("ui.rates.coverage", {}),
        ("ui.rates.coverage", {"limit": 0}),
        ("ui.rates.coverage", {"limit": None}),
        ("ui.rates.coverage", {"limit": "bad"}),
        ("ui.rates.coverage", {"unexpected": True}),
        ("ui.rates.latest", {}),
        ("ui.rates.latest", {"pair": 12}),
        ("ui.rates.latest", {"source": "kraken-csv"}),
        ("ui.rates.kraken_csv.import", {}),
        ("ui.rates.kraken_csv.import", {"path": 12}),
        ("ui.rates.kraken_csv.import", {"use_bundled": True, "operation": 12}),
        ("daemon.shutdown", {}),
    ]
    wire = "".join(json.dumps({"request_id": i, "kind": kind, "args": args}) + "\n"
                   for i, (kind, args) in enumerate(requests))
    options = dict(env=env, input=wire.encode(), capture_output=True, timeout=120)
    python = cli_runner.run(["--data-root", root, "daemon"], reference=True, **options)
    env["KASSIBER_HYBRID_RUST_DAEMON"] = "1"
    rust = cli_runner.run(["--data-root", root, "daemon"], **options)
    assert python.returncode == rust.returncode == 0
    assert python.stderr == rust.stderr == b""
    expected, actual = python.stdout.splitlines(), rust.stdout.splitlines()
    assert len(expected) == len(actual) == len(requests) + 1
    ready = json.loads(actual[0])
    assert "ui.rates.rebuild" in ready["data"]["supported_kinds"]
    assert actual[1:] == expected[1:]


def test_native_daemon_failed_import_rolls_back_before_next_request(tmp_path, monkeypatch):
    root, env = _setup(tmp_path, monkeypatch)
    env["KASSIBER_HYBRID_RUST_DAEMON"] = "1"
    directory = tmp_path / "archive"
    directory.mkdir()
    (directory / "XBTEUR_1.csv").write_text("1767225600,100,110,90,105,1,2\n")
    (directory / "XBTUSD_1.csv").write_bytes(b"\xff")
    wire = "".join(json.dumps({"request_id": i, "kind": kind, "args": args}) + "\n"
                   for i, (kind, args) in enumerate([
                       ("ui.rates.kraken_csv.import", {"path": str(directory)}),
                       ("ui.rates.summary", {}),
                       ("daemon.shutdown", {}),
                   ]))
    result = cli_runner.run(["--data-root", root, "daemon"], env=env,
                            input=wire.encode(), capture_output=True, timeout=120)
    assert result.returncode == 0, result.stderr
    records = [json.loads(line) for line in result.stdout.splitlines()]
    assert records[1]["kind"] == "error"
    assert records[2]["kind"] == "ui.rates.summary"
    assert records[2]["data"]["pairs"][0]["sample_count"] == 1
    conn = open_db(root)
    assert conn.execute("SELECT COUNT(*) FROM rates_cache").fetchone()[0] == 1
    conn.close()


@pytest.mark.parametrize("value", ["bad", "NaN", "Infinity", "1e400", "1e-20", "1e-300"])
def test_bounded_price_errors_match_python(tmp_path, monkeypatch, value):
    root, env = _setup(tmp_path, monkeypatch)
    args = ["--data-root", root, "--machine", "rates", "set", "BTC-EUR", "2026-01-01T00:00:00Z", value]
    expected = cli_runner.run(args, reference=True, env=env, capture_output=True)
    actual = cli_runner.run(args, env=env, capture_output=True)
    assert expected.returncode != 0
    assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr)
