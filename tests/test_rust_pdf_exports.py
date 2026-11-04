"""Whole-process PDF export parity; every accepted command must route to Rust.

The hand-written text reports (exit tax) must match Python byte for byte:
bytes, envelopes, files, saved markers and database effects are all
compared. Reports in the shared design (docs/reference/report-design.md:
summary, full report) match everything but the rendered bytes, page count
and content hash.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil

import pytest

from tests import test_rust_report_exports
from tests.test_rust_report_exports import _book, _dump, _normalize

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")


def _saved(data, target, identities, name):
    snapshot = data["report_snapshot"]
    assert snapshot["content_sha256"] == hashlib.sha256(target.read_bytes()).hexdigest()
    assert snapshot["report_state"] == "saved"
    assert data["bytes"] == target.stat().st_size
    identities[snapshot["id"]] = name
    return snapshot["id"]


def _design_normalized(stdout, identities, artifact):
    """Reports in the shared design are not Python's PDFs: their bytes, page
    count and content hash differ by design. Everything else must match."""
    payload = json.loads(stdout)
    data = payload.get("data")
    if payload["kind"] != "error" and isinstance(data, dict) and "file" in data:
        assert artifact.read_bytes().startswith(b"%PDF-")
        assert data["pages"] >= 1 and data["bytes"] == artifact.stat().st_size
        data["pages"] = data["bytes"] = "<rendered>"
        sha = data["report_snapshot"]["content_sha256"]
        assert sha == hashlib.sha256(artifact.read_bytes()).hexdigest()
        identities[sha] = f"sha-{len(identities)}"
    return _normalize(json.dumps(payload, sort_keys=True).encode(), identities)


def _served(env):
    return [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]


@pytest.mark.parametrize("country,algorithm,variant", [
    ("generic", "FIFO", "ready"), ("at", "MOVING_AVERAGE_AT", "ready"), ("generic", "FIFO", "stale"),
])
def test_full_pdf_cli_envelope_completion_filing_and_native_routing(tmp_path, monkeypatch, country, algorithm, variant):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm, variant)
    before = database.read_bytes()
    target = tmp_path / "nested" / "full.pdf"

    def sequence(reference):
        database.write_bytes(before)
        shutil.rmtree(target.parent, ignore_errors=True)
        identities, outcomes = {}, []
        for index, flags in enumerate([[], ["--wallet", "hot", "--history-limit", "1"], ["--history-limit", "0"],
                                       ["--wallet", "absent"], ["--history-limit", "-1"]]):
            result = call("--machine", "reports", "export-pdf", "--file", str(target), *flags, reference=reference)
            payload = json.loads(result.stdout)
            if payload["kind"] != "error":
                saved_id = _saved(payload["data"], target, identities, f"saved-{index}")
                stdout = _design_normalized(result.stdout, identities, target)
                filed = call("--machine", "reports", "filed-snapshots", "create", "--state", "filed",
                             "--saved-snapshot-id", saved_id, reference=reference)
                assert filed.returncode == 0, filed.stdout
                identities[json.loads(filed.stdout)["data"]["id"]] = f"filed-{index}"
                outcomes.append((filed.returncode, _normalize(filed.stdout, identities), filed.stderr))
            else:
                stdout = _normalize(result.stdout, identities)
            outcomes.append((result.returncode, stdout, result.stderr, _dump(root, identities)))
        return outcomes

    assert sequence(False) == sequence(True)
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} <= {"reports.export-pdf", "reports.filed-snapshots.create"}


def test_full_pdf_daemon_envelope_completion_errors_collisions_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _book(tmp_path, monkeypatch, "generic", "FIFO")
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{}, {}, {"wallet": "hot"}, {"history_limit": 1}, {"wallet": []}, {"wallet": "absent"},
                {"year": 2024}, {"tax_year": None}, {"history_limit": -1},
                {"wallet": "hot", "history_limit": None}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.reports.export_pdf", args=args)) + "\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        identities, records = {}, []
        for i, raw in enumerate(result.stdout.splitlines()[1:]):
            record = json.loads(raw)
            if record["kind"] != "error" and "file" in record.get("data", {}):
                _saved(record["data"], Path(record["data"]["file"]), identities, f"saved-{i}")
                records.append(_design_normalized(raw, identities, Path(record["data"]["file"])))
            else:
                records.append(_normalize(raw, identities))
        return records, _dump(root, identities), sorted(str(p.relative_to(exports)) for p in exports.rglob("*"))

    assert session(False) == session(True)
    assert _served(env) == [{"command": "daemon", "served_by": "rust"}]


@pytest.mark.parametrize("country,algorithm,variant", [
    ("at", "MOVING_AVERAGE_AT", "ready"), ("at", "MOVING_AVERAGE_AT", "stale"), ("generic", "FIFO", "ready"),
])
def test_exit_tax_pdf_cli_bytes_and_native_routing(tmp_path, monkeypatch, country, algorithm, variant):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm, variant)
    before = database.read_bytes()
    target = tmp_path / "nested" / "exit.pdf"

    def sequence(reference):
        database.write_bytes(before)
        shutil.rmtree(target.parent, ignore_errors=True)
        outcomes = []
        for flags in [["--departure-date", "2025-06-30", "--destination", "third_country"],
                      ["--departure-date", "2025-06-30", "--destination", "eu_eea"],
                      ["--departure-date", "not-a-date"], ["--destination", "mars"]]:
            result = call("--machine", "reports", "export-exit-tax-pdf", "--file", str(target), *flags,
                          reference=reference)
            content = target.read_bytes() if target.exists() else None
            outcomes.append((result.returncode, result.stdout, result.stderr, content, _dump(root, {})))
        return outcomes

    assert sequence(False) == sequence(True)
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    # `--destination mars` is an argparse usage error, also answered by Rust.
    assert {c["command"] for c in served} == {"reports.export-exit-tax-pdf", "<usage>"}


def test_exit_tax_pdf_daemon_bytes_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _book(tmp_path, monkeypatch, "at", "MOVING_AVERAGE_AT")
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{"departure_date": "2025-06-30", "destination": "third_country"},
                {"departure_date": "2025-06-30", "destination": "third_country"},
                {"departure_date": "2025-06-30", "destination": "eu_eea"}, {"departure_date": "bad"},
                {"destination": "mars"}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.reports.export_exit_tax_pdf", args=args)) + "\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        return (result.stdout.splitlines()[1:], _dump(root, {}),
                {str(p.relative_to(exports)): p.read_bytes() for p in exports.rglob("*.pdf")})

    native, reference = session(False), session(True)
    for actual, expected in zip(native[0], reference[0]):
        assert actual == expected, (actual, expected)
    assert native == reference
    assert _served(env) == [{"command": "daemon", "served_by": "rust"}]



def _summary_book(tmp_path, monkeypatch, *args):
    # The summary admits only real datetimes; its clock is the report hooks'.
    monkeypatch.setattr(test_rust_report_exports, "REFERENCE",
                        test_rust_report_exports.REFERENCE.replace("reports.datetime = FixedDateTime\n", ""))
    return _book(tmp_path, monkeypatch, *args)


@pytest.mark.parametrize("country,algorithm,variant", [
    ("generic", "FIFO", "ready"), ("at", "MOVING_AVERAGE_AT", "ready"), ("generic", "FIFO", "stale"),
])
def test_summary_pdf_cli_envelope_completion_filing_and_native_routing(tmp_path, monkeypatch, country, algorithm,
                                                                       variant):
    root, database, env, call = _summary_book(tmp_path, monkeypatch, country, algorithm, variant)
    before = database.read_bytes()
    target = tmp_path / "nested" / "summary.pdf"

    def sequence(reference):
        database.write_bytes(before)
        shutil.rmtree(target.parent, ignore_errors=True)
        identities, outcomes = {}, []
        for index, flags in enumerate([
                [], ["--wallet", "hot", "--include-snapshot"],
                ["--start", "2024-01-01T00:00:00Z", "--end", "2025-12-31T23:59:59Z"],
                ["--wallet", "absent"], ["--start", "yesterday"],
                ["--start", "2026-01-01T00:00:00Z", "--end", "2025-01-01T00:00:00Z"]]):
            result = call("--machine", "reports", "export-summary-pdf", "--file", str(target), *flags,
                          reference=reference)
            payload = json.loads(result.stdout)
            if payload["kind"] != "error":
                identities[payload["data"]["report_snapshot"]["id"]] = f"saved-{index}"
                filed = call("--machine", "reports", "filed-snapshots", "create", "--state", "filed",
                             "--saved-snapshot-id", payload["data"]["report_snapshot"]["id"], reference=reference)
                assert filed.returncode == 0, filed.stdout
                identities[json.loads(filed.stdout)["data"]["id"]] = f"filed-{index}"
            stdout = _design_normalized(result.stdout, identities, target)
            outcomes.append((result.returncode, stdout, result.stderr, _dump(root, identities)))
        return outcomes

    native, reference = sequence(False), sequence(True)
    for actual, expected in zip(native, reference):
        assert actual == expected
    assert len(native) == len(reference)
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} <= {"reports.export-summary-pdf", "reports.filed-snapshots.create"}


def test_summary_pdf_daemon_envelope_errors_collisions_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _summary_book(tmp_path, monkeypatch, "at", "MOVING_AVERAGE_AT")
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{}, {}, {"wallets": ["hot"], "include_snapshot": True}, {"wallets": []}, {"wallets": "hot"},
                {"include_snapshot": None}, {"include_snapshot": "yes"}, {"year": 2024},
                {"start": "2024-01-01T00:00:00Z", "end": "2025-12-31T23:59:59Z"}, {"wallets": ["absent"]},
                {"start": "yesterday"}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.reports.export_summary_pdf", args=args)) + "\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        identities, records = {}, []
        for raw in result.stdout.splitlines()[1:]:
            record = json.loads(raw)
            data = record.get("data")
            if record["kind"] != "error" and isinstance(data, dict) and "file" in data:
                identities[data["report_snapshot"]["id"]] = f"saved-{len(identities)}"
                records.append(_design_normalized(raw, identities, Path(data["file"])))
            else:
                records.append(_normalize(raw, identities))
        return records, _dump(root, identities), sorted(str(p.relative_to(exports)) for p in exports.rglob("*"))

    native, reference = session(False), session(True)
    for actual, expected in zip(native[0], reference[0]):
        assert actual == expected
    assert native == reference
    assert _served(env) == [{"command": "daemon", "served_by": "rust"}]
