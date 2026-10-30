"""Whole-process CSV export parity; every accepted command must route to Rust."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import uuid

import pytest

from kassiber.db import DEFAULT_DB_FILENAME, open_db
from tests import cli_runner

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
STAMP = "2026-01-02T03:04:05Z"
# Test-only clock injection precedes the product boundary, including subprocesses.
REFERENCE = r'''
import sys, os, time, importlib
from pathlib import Path
from dataclasses import replace
from datetime import datetime, timezone
from kassiber.operator import project
project._owner_lock_root = lambda: Path(os.environ["KASSIBER_RUST_TEST_OWNER_ROOT"])
cli = importlib.import_module("kassiber.cli.main")
from kassiber.core import reports, custody_filed_reports, filed_report_chain, export_range
stamp = os.environ["KASSIBER_RUST_TEST_REPORT_TIME"]
class FixedDateTime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls(2026,1,2,3,4,5,tzinfo=timezone.utc).astimezone(tz)
reports.datetime = FixedDateTime
export_range.datetime = FixedDateTime
custody_filed_reports.now_iso = lambda: stamp
filed_report_chain.now_iso = lambda: stamp
hooks = cli._report_hooks
cli._report_hooks = lambda: replace(hooks(), now_iso=lambda: stamp)
from kassiber import daemon
daemon._report_hooks = cli._report_hooks
time.gmtime = lambda *args: time.struct_time((2026,1,2,3,4,5,4,2,0))
raise SystemExit(cli.main(sys.argv[1:]))
'''


def _book(tmp_path, monkeypatch, country, algorithm, variant="ready"):
    root = tmp_path / "data"
    conn = open_db(root)
    cases = json.loads((ROOT / "crates/reports/tests/data/export_sequences.json").read_text())
    case = next(c for c in cases if c["name"] == f"{country}-{algorithm}-{variant}")
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + case["sql"])
    conn.close()
    # Finish profile compatibility after synthetic insertion, then seed the
    # already processed projection. Export processes must not repair fixtures.
    conn = open_db(root)
    conn.execute("""UPDATE profiles SET last_processed_at=?,last_processed_tx_count=6,
        last_processed_input_version=journal_input_version WHERE id='p'""", (STAMP,))
    if variant == "stale":
        conn.execute("UPDATE profiles SET journal_input_version=journal_input_version+1 WHERE id='p'")
    conn.commit()
    database = Path(conn.execute("PRAGMA database_list").fetchone()[2])
    conn.close()
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env = {**os.environ,
        "KASSIBER_RUST_TEST_OWNER_ROOT": str(owners),
        "KASSIBER_RUST_TEST_REPORT_TIME": STAMP,
        "KASSIBER_HYBRID_RUST_BIN": str(Path(RUST_BIN).resolve()),
        "KASSIBER_HYBRID_LOG": str(tmp_path / "served.jsonl"),
        "KASSIBER_HYBRID_RUST_DAEMON": "1",
        "KASSIBER_TEST_NO_EGRESS": "strict",
        "KASSIBER_DISABLE_UPDATE_CHECK": "1",
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(tmp_path / "offline.json"),
        "XDG_CONFIG_HOME": str(tmp_path / "config"),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "XDG_STATE_HOME": str(tmp_path / "state"),
    }
    monkeypatch.setenv("KASSIBER_TEST_CLI", json.dumps([sys.executable, str(ROOT / "tests/rust_port/hybrid_cli.py")]))
    original_prefix = cli_runner.prefix
    monkeypatch.setattr(cli_runner, "prefix", lambda *, reference=False, python_options=():
        (sys.executable, "-c", REFERENCE) if reference else original_prefix(python_options=python_options))
    def call(*args, reference=False, wire=None):
        return cli_runner.run(["--data-root", root, *args], reference=reference, env=env,
                              input=wire, capture_output=True, timeout=180)
    return root, database, env, call


def _dump(root, identities):
    # Observe the process result without allowing compatibility/migration code
    # to repair a difference before the whole-database comparison.
    conn = sqlite3.connect((root / DEFAULT_DB_FILENAME).as_uri() + "?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    def normalize(v):
        if isinstance(v, str):
            return identities.get(v, v)
        if isinstance(v, list):
            return [normalize(x) for x in v]
        if isinstance(v, dict):
            return {k: normalize(x) for k, x in v.items()}
        return v
    result = {t: sorted(normalize([dict(r) for r in conn.execute(f'SELECT * FROM "{t}"')]),
                        key=lambda r: json.dumps(r, sort_keys=True)) for t in tables}
    conn.close()
    return result


def _normalize(wire, identities):
    for actual, mapped in identities.items():
        wire = wire.replace(actual.encode(), mapped.encode())
    return wire


@pytest.mark.parametrize("country,algorithm", [("generic", "FIFO"), ("generic", "HIFO"), ("at", "MOVING_AVERAGE_AT")])
@pytest.mark.parametrize("variant", ["ready", "raw-hex"])
def test_cli_csv_export_sequences_and_rust_routing(tmp_path, monkeypatch, country, algorithm, variant):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm, variant)
    original = database.read_bytes()
    artifacts = tmp_path / "artifacts"
    def sequence(reference):
        shutil.rmtree(artifacts, ignore_errors=True)
        identities = {}
        outcomes = []
        commands = [
            ("reports", "export-csv", "--file", str(artifacts / "full.csv")),
            ("reports", "export-csv", "--wallet", "Cold", "--history-limit", "2", "--file", str(artifacts / "wallet.csv")),
            ("transactions", "export", "--export-format", "csv", "--wallet", "Hot", "--wallet", "Cold", "--from", "2024-01-01", "--to", "2024-12-31", "--timezone", "Europe/Vienna", "--file", str(artifacts / "transactions.csv")),
        ]
        if country == "at":
            commands.append(("reports", "export-austrian-e1kv-csv", "--year", "2024", "--dir", str(artifacts / "bundle")))
        first = None
        for index in range(len(commands) + 2):
            args = commands[index] if index < len(commands) else ("reports", "filed-snapshots", "create", "--state", "filed", "--saved-snapshot-id", first)
            result = call("--machine", *args, reference=reference)
            assert result.returncode == 0, result.stdout
            payload = json.loads(result.stdout)["data"]
            snapshot = payload.get("report_snapshot") or (payload if "report_state" in payload else None)
            if snapshot:
                assert uuid.UUID(snapshot["id"]).version == 4
                identities[snapshot["id"]] = f"action-{index}"
                first = first or snapshot["id"]
                assert snapshot["created_at"] == STAMP
            outcomes.append((_normalize(result.stdout, identities), result.stderr,
                             _dump(root, identities), {str(p.relative_to(artifacts)): p.read_bytes() for p in sorted(artifacts.rglob("*.csv"))}))
        return outcomes
    expected = sequence(True)
    database.write_bytes(original)
    assert sequence(False) == expected
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert calls and all(c["served_by"] == "rust" for c in calls)
    expected_commands = {"reports.export-csv", "transactions.export", "reports.filed-snapshots.create"}
    if country == "at":
        expected_commands.add("reports.export-austrian-e1kv-csv")
    assert {c["command"] for c in calls} == expected_commands


@pytest.mark.parametrize("country,algorithm", [("generic", "FIFO"), ("generic", "HIFO"), ("at", "MOVING_AVERAGE_AT")])
def test_daemon_csv_exports_response_artifact_and_state_parity(tmp_path, monkeypatch, country, algorithm):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm)
    original = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [
        ("ui.reports.export_csv", {}), ("ui.reports.export_csv", {"wallet": "Cold", "history_limit": 2}),
        ("ui.reports.export_capital_gains_csv", {}),
        *[("ui.reports.export_capital_gains_csv", {"year": year}) for year in (2024, 2025, "2024", 2024.9, True, [], "bad")],
        ("ui.transactions.export_csv", {}),
        ("ui.transactions.export_csv", {"wallets": ["Cold", "Hot"], "range": "custom", "from": "2024-01-01", "to": "2024-12-31", "timezone": "Europe/Vienna"}),
        ("ui.transactions.export_csv", {"wallets": []}),
        ("ui.transactions.export_csv", {"range": ""}),
        ("ui.transactions.export_csv", {"wallet": "Cold", "wallets": []}),
        ("ui.transactions.export_csv", {"z": 1, "a": 2}),
        ("ui.reports.export_csv", {"year": None}),
        ("ui.reports.export_csv", {"wallet": False}),
        ("ui.reports.export_austrian_e1kv_csv", {"year": 2024}),
    ]
    wire = b"".join((json.dumps({"request_id": i, "kind": kind, "args": args}) + "\n").encode() for i, (kind, args) in enumerate(requests))
    wire += b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        identities = {}
        records = result.stdout.splitlines(keepends=True)
        ready = json.loads(records.pop(0))
        if not reference:
            assert {k for k, _ in requests} <= set(ready["data"]["supported_kinds"])
        for index, record in enumerate(records):
            snapshot = json.loads(record).get("data", {}).get("report_snapshot")
            if snapshot:
                identities[snapshot["id"]] = f"action-{index}"
                assert snapshot["report_state"] == "saved"
        assert len(identities) == (3 if country == "at" else 2)
        return ([_normalize(r, identities) for r in records], _dump(root, identities),
                {str(p.relative_to(exports)): p.read_bytes() for p in sorted(exports.rglob("*.csv"))})
    expected = session(True)
    database.write_bytes(original)
    assert session(False) == expected
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert calls == [{"command": "daemon", "served_by": "rust"}]


@pytest.mark.parametrize("country,algorithm", [("generic", "FIFO"), ("generic", "HIFO"), ("at", "MOVING_AVERAGE_AT")])
def test_cli_stale_admission_and_transaction_drafts(tmp_path, monkeypatch, country, algorithm):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm, "stale")
    original = database.read_bytes()
    target = tmp_path / "report.csv"
    def sequence(reference):
        target.write_bytes(b"previous complete artifact\r\n")
        result = call("--machine", "reports", "export-csv", "--file", str(target), reference=reference)
        assert result.returncode == 1
        assert json.loads(result.stdout)["error"]["code"] == "app_error"
        published = target.read_bytes()
        assert published == b"previous complete artifact\r\n"
        draft = call("--machine", "transactions", "export", "--export-format", "csv", "--file", str(tmp_path / "draft.csv"), reference=reference)
        assert draft.returncode == 0, draft.stdout
        assert "report_snapshot" not in json.loads(draft.stdout)["data"]
        state = _dump(root, {})
        assert state["filed_report_snapshots"] == []
        return result.stdout, result.stderr, published, draft.stdout, draft.stderr, (tmp_path / "draft.csv").read_bytes(), state
    expected = sequence(True)
    database.write_bytes(original)
    assert sequence(False) == expected
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert calls and all(c["served_by"] == "rust" for c in calls)


@pytest.mark.parametrize("country,algorithm", [("generic", "FIFO"), ("generic", "HIFO"), ("at", "MOVING_AVERAGE_AT")])
def test_cli_csv_export_rendering_formats(tmp_path, monkeypatch, country, algorithm):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm)
    original = database.read_bytes()
    target = tmp_path / "rendered.csv"
    commands = [("reports", "export-csv", "--file", str(target)),
                ("transactions", "export", "--export-format", "csv", "--file", str(target))]
    if country == "at":
        commands.append(("reports", "export-austrian-e1kv-csv", "--year", "2024", "--dir", str(tmp_path / "bundle")))
    for format_ in ("table", "plain", "csv"):
        for command in commands:
            def execute(reference):
                database.write_bytes(original)
                result = call("--format", format_, *command, reference=reference)
                assert result.returncode == 0, result.stderr
                state = _dump(root, {})
                identities = {row["id"]: "<saved-id>" for row in state["filed_report_snapshots"]}
                files = {str(p.relative_to(tmp_path)): p.read_bytes() for p in sorted(tmp_path.rglob("*.csv"))}
                return _normalize(result.stdout, identities), result.stderr, files
            assert execute(False) == execute(True)
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert calls and all(c["served_by"] == "rust" for c in calls)


def test_cli_csv_export_filesystem_refusals(tmp_path, monkeypatch):
    root, database, env, call = _book(tmp_path, monkeypatch, "at", "MOVING_AVERAGE_AT")
    original = database.read_bytes()
    parent_file = tmp_path / "occupied"
    parent_file.write_bytes(b"keep existing file")
    for leaf, option, extra in (("export-csv", "--file", ()),
                                ("export-austrian-e1kv-csv", "--dir", ("--year", "2024")),
                                ("transactions", "--file", ())):
        targets = [parent_file / "report.csv", parent_file / "missing" / "report.csv"]
        for target in targets:
            args = ("transactions", "export", "--export-format", "csv") if leaf == "transactions" else ("reports", leaf)
            def execute(reference):
                database.write_bytes(original)
                result = call("--machine", *args, *extra, option, str(target), reference=reference)
                assert result.returncode == 1
                assert parent_file.read_bytes() == b"keep existing file"
                return result.returncode, result.stdout, result.stderr, _dump(root, {})
            assert execute(False) == execute(True)
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert calls and all(c["served_by"] == "rust" for c in calls)
