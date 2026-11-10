"""Native transaction XLSX projection, publication and CLI/daemon routing oracle."""
from dataclasses import replace
from datetime import date, datetime
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from unittest.mock import patch

import pytest

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.core.export_range import ExportRange
from kassiber.db import open_db
from kassiber.errors import AppError
from tests import cli_runner
from tests.rust_port.xlsx import RecordingWorkbook, canonical, first_difference, STAMP
from tests.test_rust_report_exports import _book, _dump, REFERENCE

ROOT = Path(__file__).resolve().parents[1]
SERVICE = os.environ.get("KASSIBER_XLSX_SERVICE_BIN")
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
CASES = [c for c in json.loads((ROOT / "crates/reports/tests/data/artifact_csv.json").read_text())
         if c["command"] == "reports.export-transactions-csv"]
CASES += [dict(name=f"xlsx-{name}", args_json="{}", sql=sql) for name, sql in [
    ("empty", "DELETE FROM journal_entries; DELETE FROM journal_quarantines; DELETE FROM transactions;"),
    ("unicode", "UPDATE transactions SET note='日本語 € 😀\nquoted, text' WHERE id='out';"),
    ("long-text", "UPDATE transactions SET note='" + "long " * 8000 + "' WHERE id='out';"),
    ("huge-fiat", "UPDATE transactions SET fiat_rate_exact='1e100' WHERE id='out';"),
    ("tiny-fiat", "UPDATE transactions SET fiat_rate_exact='1e-100' WHERE id='out';"),
    ("negative-fiat", "UPDATE transactions SET fiat_rate_exact='-12345.678901234567' WHERE id='out';"),
    ("single-link", """INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,
        attachment_type,label,source_url,created_at)
        VALUES('link','ws','p','out','url','Receipt €','https://example.invalid/a?x=1&y=2','a');"""),
    ("many-rows", """WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<500)
        INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,
            amount,fee,occurred_at,created_at,note)
        SELECT 'row-'||i,'ws','p','hot','row-'||i,'BTC','inbound',i,0,
            '2025-01-01T00:00:00Z','fixed','Row '||i FROM n;"""),
]]


def _period(payload):
    if not payload:
        return None
    return ExportRange(preset=payload["preset"], timezone=payload["timezone"],
        start=datetime.fromisoformat(payload["start"].replace("Z", "+00:00")),
        end=datetime.fromisoformat(payload["end"].replace("Z", "+00:00")),
        first_day=date.fromisoformat(payload["first_day"]), last_day=date.fromisoformat(payload["last_day"]))


def _wire(raw):
    # The ZIP byte count differs; all other response bytes must match.
    return re.sub(rb'("bytes"\s*:\s*)\d+', rb'\1<xlsx-bytes>', raw)


@pytest.mark.skipif(not SERVICE, reason="requires native XLSX service probe")
@pytest.mark.parametrize("case", CASES, ids=lambda c: c["name"])
def test_native_transaction_fixture_corpus(tmp_path, case):
    root = tmp_path / "data"
    conn = open_db(root)
    conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + case["sql"])
    conn.commit()
    database = Path(conn.execute("PRAGMA database_list").fetchone()[2])
    args = json.loads(case["args_json"])
    target = tmp_path / "list.xlsx"
    try:
        with patch("xlsxwriter.Workbook", RecordingWorkbook):
            expected = {"ok": reports.export_transactions_xlsx_report(conn, None, None, target,
                replace(_report_hooks(), now_iso=lambda: STAMP), wallet_ref=args.get("wallet"),
                wallet_refs=args.get("wallets"), export_range=_period(args.get("export_range")))}
    except AppError as e:
        expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
    except (ValueError, TypeError, OverflowError, AttributeError) as e:
        expected = dict(error=dict(code="internal_error", message=str(e), hint=None, details=None, retryable=False))
    conn.close()
    before = _dump(root, {})
    if "ok" in expected:
        expected_book = canonical(target)
        target.unlink()
    request = json.dumps(dict(database=str(database), args_json=case["args_json"], path=str(target))) + "\n"
    result = subprocess.run([SERVICE], input=request, capture_output=True, text=True, check=True)
    assert not result.stderr
    actual = json.loads(result.stdout)
    if "ok" in expected:
        assert list(actual["ok"]) == list(expected["ok"])
        assert actual["ok"]["bytes"] == target.stat().st_size
        expected["ok"]["bytes"] = actual["ok"]["bytes"]
        assert not (difference := first_difference(expected_book, canonical(target))), difference
    else:
        assert not target.exists()
    assert actual == expected
    assert _dump(root, {}) == before


def _process_book(tmp_path, monkeypatch, variant="ready"):
    root, database, env, _ = _book(tmp_path, monkeypatch, "generic", "FIFO", variant)
    # Pin the independent Python writer's clock as well as the report clock.
    reference = REFERENCE.replace("raise SystemExit(cli.main(sys.argv[1:]))", "import xlsxwriter.core\nxlsxwriter.core.datetime = FixedDateTime\nraise SystemExit(cli.main(sys.argv[1:]))")
    original_prefix = cli_runner.prefix
    monkeypatch.setattr(cli_runner, "prefix", lambda *, reference=False, python_options=():
        (sys.executable, "-c", reference_code) if reference else original_prefix(python_options=python_options))
    reference_code = reference
    def call(*args, reference=False, wire=None):
        return cli_runner.run(["--data-root", root, *args], reference=reference, env=env,
                              input=wire, capture_output=True, timeout=180)
    return root, database, env, call


@pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
@pytest.mark.parametrize("variant", ["ready", "stale"])
def test_transaction_cli_workbooks_state_and_native_routing(tmp_path, monkeypatch, variant):
    root, database, env, call = _process_book(tmp_path, monkeypatch, variant)
    before = database.read_bytes()
    target = tmp_path / "transactions.xlsx"
    for extra in [(), ("--wallet", "Cold"), ("--wallet", "Cold", "--wallet", "Hot"),
                  ("--period", "last-year"), ("--from", "2024-01-01", "--to", "2024-12-31", "--timezone", "Europe/Vienna"),
                  ("--wallet", "missing"), ("--period", "last-year", "--from", "2024-01-01"),
                  ("--from", "", "--to", "")]:
        def execute(reference):
            database.write_bytes(before)
            target.unlink(missing_ok=True)
            result = call("--machine", "transactions", "export", "--file", str(target), *extra, reference=reference)
            assert result.stdout, result.stderr
            payload = json.loads(result.stdout)
            content = None
            if payload["kind"] != "error":
                assert payload["data"]["bytes"] == target.stat().st_size
                payload["data"].pop("bytes")
                content = canonical(target)
            return result.returncode, result.stderr, _wire(result.stdout), payload, content, _dump(root, {})
        assert execute(False) == execute(True)
    served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert served == [{"command": "transactions.export", "served_by": "rust"}] * 8


@pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
def test_transaction_daemon_workbooks_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _process_book(tmp_path, monkeypatch)
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{}, {}, {"wallet": "Cold"}, {"wallets": ["Hot", "Cold"]}, {"wallets": []},
                {"wallet": "Cold", "wallets": []}, {"range": ""}, {"z": 1, "a": 2},
                {"range": "custom", "from": "2024-01-01", "to": "2024-12-31", "timezone": "Europe/Vienna"}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.transactions.export_xlsx", args=args))+"\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'
    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        records = [json.loads(line) for line in result.stdout.splitlines()]
        ready = records.pop(0)
        if not reference:
            assert "ui.transactions.export_xlsx" in ready["data"]["supported_kinds"]
        for record in records:
            if record["kind"] != "error" and "file" in record.get("data", {}):
                path = Path(record["data"]["file"])
                assert record["data"].pop("bytes") == path.stat().st_size
        return ([_wire(line) for line in result.stdout.splitlines()[1:]], records,
                {str(p.relative_to(exports)): canonical(p) for p in exports.rglob("*.xlsx")}, _dump(root, {}))
    assert session(False) == session(True)
    served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert served == [{"command": "daemon", "served_by": "rust"}]


@pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
def test_transaction_xlsx_filesystem_refusals_leave_database_unchanged(tmp_path, monkeypatch):
    root, database, env, call = _process_book(tmp_path, monkeypatch)
    original = database.read_bytes()
    parent = tmp_path / "occupied"
    parent.write_bytes(b"keep existing file")
    for target in (parent / "report.xlsx", parent / "missing" / "report.xlsx"):
        def execute(reference):
            database.write_bytes(original)
            result = call("--machine", "transactions", "export", "--file", str(target), reference=reference)
            assert result.returncode == 1
            assert parent.read_bytes() == b"keep existing file"
            return result.returncode, result.stdout, result.stderr, _dump(root, {})
        assert execute(False) == execute(True)
    served = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert served == [{"command": "transactions.export", "served_by": "rust"}] * 2
