"""Whole-process PDF export parity; every accepted command must route to Rust.

The hand-written text reports must match Python byte for byte: bytes,
envelopes, files, saved markers and database effects are all compared.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil

import pytest

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


def _served(env):
    return [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]


@pytest.mark.parametrize("country,algorithm,variant", [
    ("generic", "FIFO", "ready"), ("at", "MOVING_AVERAGE_AT", "ready"), ("generic", "FIFO", "stale"),
])
def test_full_pdf_cli_bytes_completion_filing_and_native_routing(tmp_path, monkeypatch, country, algorithm, variant):
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
            content = None
            if payload["kind"] != "error":
                saved_id = _saved(payload["data"], target, identities, f"saved-{index}")
                content = target.read_bytes()
                filed = call("--machine", "reports", "filed-snapshots", "create", "--state", "filed",
                             "--saved-snapshot-id", saved_id, reference=reference)
                assert filed.returncode == 0, filed.stdout
                identities[json.loads(filed.stdout)["data"]["id"]] = f"filed-{index}"
                outcomes.append((filed.returncode, _normalize(filed.stdout, identities), filed.stderr))
            outcomes.append((result.returncode, _normalize(result.stdout, identities), result.stderr,
                             content, _dump(root, identities)))
        return outcomes

    assert sequence(False) == sequence(True)
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} <= {"reports.export-pdf", "reports.filed-snapshots.create"}


def test_full_pdf_daemon_bytes_completion_errors_collisions_and_native_routing(tmp_path, monkeypatch):
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
        records = result.stdout.splitlines()[1:]
        identities = {}
        for i, raw in enumerate(records):
            record = json.loads(raw)
            if record["kind"] != "error" and "file" in record.get("data", {}):
                _saved(record["data"], Path(record["data"]["file"]), identities, f"saved-{i}")
        return ([_normalize(r, identities) for r in records], _dump(root, identities),
                {str(p.relative_to(exports)): p.read_bytes() for p in exports.rglob("*.pdf")})

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

