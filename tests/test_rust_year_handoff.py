"""Native year-handoff workbook, completion and process parity."""
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import shutil
import subprocess
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import year_handoff, custody_filed_reports, filed_report_chain
from kassiber.db import open_db
from kassiber.errors import AppError
from tests.rust_port.xlsx import RecordingWorkbook, canonical, first_difference, STAMP
from tests.test_rust_report_exports import _dump, _normalize
from tests.test_rust_xlsx_exports import _wire
from tests.test_rust_austrian_xlsx import _clocked_book

ROOT = Path(__file__).resolve().parents[1]
SERVICE = os.environ.get("KASSIBER_XLSX_SERVICE_BIN")
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
CASES = json.loads((ROOT / "crates/reports/tests/data/year_handoff.json").read_text())


def _state(root, identities):
    tables = _dump(root, identities)
    assert len(tables) == 113
    return json.dumps(tables, sort_keys=True, separators=(",", ":")).encode()


def _saved(data, target, identities, name):
    snapshot = data["report_snapshot"]
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    assert snapshot["content_sha256"] == digest
    assert snapshot["report_kind"] == "year-handoff.xlsx"
    assert snapshot["report_state"] == "saved"
    assert snapshot["created_at"] == STAMP
    assert data["bytes"] == target.stat().st_size
    identities[snapshot["id"]] = name
    # ZIP bytes intentionally differ: normalize only the independently verified
    # new artifact digest, preserving all preexisting saved commitments literally.
    identities[digest] = name + "-verified-digest"
    return snapshot["id"]


@pytest.mark.skipif(not SERVICE, reason="requires native XLSX service probe")
@pytest.mark.parametrize("case", CASES, ids=lambda c: c["name"])
def test_year_handoff_service_content_errors_and_all_112_tables(tmp_path, case):
    root = tmp_path / "data"
    conn = open_db(root)
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + case["sql"])
    conn.commit()
    database = Path(conn.execute("PRAGMA database_list").fetchone()[2])
    conn.close()
    before = database.read_bytes()
    conn = sqlite3.connect(database)
    conn.row_factory = sqlite3.Row
    target = tmp_path / "handoff.xlsx"
    args = json.loads(case["args_json"])
    identities = {}
    try:
        with patch("xlsxwriter.Workbook", RecordingWorkbook), \
             patch.object(custody_filed_reports, "now_iso", lambda: STAMP), \
             patch.object(filed_report_chain, "now_iso", lambda: STAMP):
            expected = dict(ok=year_handoff.export_year_handoff_xlsx(conn,
                args.get("workspace"), args.get("profile"), target,
                replace(_report_hooks(), now_iso=lambda: STAMP), year=args.get("year")))
    except AppError as e:
        expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
    except (ValueError, TypeError, OverflowError, AttributeError) as e:
        expected = dict(error=dict(code="internal_error", message=str(e), hint=None, details=None, retryable=False))
    conn.close()
    if "ok" in expected:
        _saved(expected["ok"], target, identities, "saved")
        content = canonical(target)
    expected_state = _state(root, identities)
    expected_wire = _wire(_normalize(json.dumps(expected).encode(), identities))
    database.write_bytes(before)
    target.unlink(missing_ok=True)
    request = dict(database=str(database), command="reports.export-year-handoff",
                   args_json=case["args_json"], path=str(target))
    result = subprocess.run([SERVICE], input=json.dumps(request) + "\n", capture_output=True,
                            text=True, timeout=180, check=True)
    assert not result.stderr
    actual = json.loads(result.stdout)
    identities = {}
    if "ok" in expected:
        assert list(actual["ok"]) == list(expected["ok"])
        _saved(actual["ok"], target, identities, "saved")
        assert not (difference := first_difference(content, canonical(target))), difference
    else:
        assert not target.exists()
    assert _wire(_normalize(json.dumps(actual).encode(), identities)) == expected_wire
    assert _state(root, identities) == expected_state


@pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
@pytest.mark.parametrize("country,variant", [("at", "ready"), ("at", "raw-hex"), ("at", "stale"), ("generic", "ready")])
def test_year_handoff_cli_content_filing_and_native_routing(tmp_path, monkeypatch, country, variant):
    root, database, env, call = _clocked_book(tmp_path, monkeypatch, country, variant)
    conn = sqlite3.connect(database)
    conn.execute("DELETE FROM journal_quarantines")
    conn.commit()
    conn.close()
    before = database.read_bytes()
    target = tmp_path / "handoff.xlsx"
    def sequence(reference):
        database.write_bytes(before)
        target.unlink(missing_ok=True)
        identities, outcomes = {}, []
        for year in (2024, 2025, 2026, 2008):
            flags = ["--year", str(year)]
            result = call("--machine", "reports", "export-year-handoff", "--file", str(target), *flags, reference=reference)
            payload = json.loads(result.stdout)
            content = None
            if payload["kind"] != "error":
                saved_id = _saved(payload["data"], target, identities, f"saved-{year}")
                content = canonical(target)
            outcomes.append((result.returncode, _wire(_normalize(result.stdout, identities)), result.stderr,
                             content, _state(root, identities)))
            if payload["kind"] != "error":
                for index in range(2):
                    filed = call("--machine", "reports", "filed-snapshots", "create", "--state", "filed",
                                 "--saved-snapshot-id", saved_id, reference=reference)
                    assert filed.returncode == 0, filed.stdout
                    item = json.loads(filed.stdout)["data"]
                    assert item["content_sha256"] == payload["data"]["report_snapshot"]["content_sha256"]
                    identities[item["id"]] = f"filed-{year}-{index}"
                    outcomes.append((filed.returncode, _normalize(filed.stdout, identities), filed.stderr,
                                     _state(root, identities)))
        return outcomes
    assert sequence(False) == sequence(True)
    served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} <= {"reports.export-year-handoff", "reports.filed-snapshots.create"}


@pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
@pytest.mark.parametrize("blocked", [False, True])
def test_year_handoff_daemon_content_filing_collisions_and_native_routing(tmp_path, monkeypatch, blocked):
    root, database, env, call = _clocked_book(tmp_path, monkeypatch)
    conn = sqlite3.connect(database)
    conn.execute("DELETE FROM journal_quarantines")
    conn.commit()
    conn.close()
    if blocked:
        conn=sqlite3.connect(database)
        conn.executescript("INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','failed','wallet','Cold','failed',1,'fixed'); UPDATE profiles SET last_processed_input_version=-1 WHERE id='p';")
        conn.close()
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{"year": 2024}, {"year": 2024}, {"year": 2025}, {"year": 2026}, {}, {"year": True},
                {"year": "2025"}, {"year": 2024.9}, {"year": []}, {"year": "bad"},
                {"year": 2008}, {"year": 2024, "workspace": "absent"}, None, [], 1]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.reports.export_year_handoff", args=args))+"\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        records = result.stdout.splitlines()[1:]
        identities = {}
        for i, raw in enumerate(records):
            record = json.loads(raw)
            if record["kind"] != "error" and "file" in record.get("data", {}):
                _saved(record["data"], Path(record["data"]["file"]), identities, f"saved-{i}")
        return ([_wire(_normalize(r, identities)) for r in records], _state(root, identities),
                {str(p.relative_to(exports)): canonical(p) for p in exports.rglob("*.xlsx")})
    assert session(False) == session(True)
    served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert served == [{"command": "daemon", "served_by": "rust"}]
