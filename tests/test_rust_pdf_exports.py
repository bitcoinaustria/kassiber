"""Whole-process PDF export parity; every accepted command must route to Rust.

Every report is in the shared design (docs/reference/report-design.md):
envelopes, errors, files, saved and filed markers and database effects match
Python; the rendered bytes, page count, content hash and the listed envelope
differences do not.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import zipfile

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
        # The Austrian PDF names its renderer: ReportLab in Python, Typst here.
        if "renderer" in data:
            assert data["renderer"] in {"reportlab", "typst"}
            data["renderer"] = "<renderer>"
        sha = data["report_snapshot"]["content_sha256"]
        assert sha == hashlib.sha256(artifact.read_bytes()).hexdigest()
        # Typst output repeats byte for byte where ReportLab's may not, so
        # every artifact hash maps to one placeholder.
        identities[sha] = "<artifact-sha256>"
    return _normalize(json.dumps(payload, sort_keys=True).encode(), identities)


def _exit_normalized(stdout, artifact):
    """Exports without a saved marker (exit tax, source of funds): only the
    pages, bytes and renderer differ."""
    payload = json.loads(stdout)
    data = payload["data"]
    assert artifact.read_bytes().startswith(b"%PDF-")
    assert data["pages"] >= 1 and data["bytes"] == artifact.stat().st_size
    data["pages"] = data["bytes"] = "<rendered>"
    if "renderer" in data:
        assert data["renderer"] in {"reportlab", "typst"}
        data["renderer"] = "<renderer>"
    return json.dumps(payload, sort_keys=True).encode()


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
def test_exit_tax_pdf_cli_envelope_and_native_routing(tmp_path, monkeypatch, country, algorithm, variant):
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
            stdout = (_exit_normalized(result.stdout, target) if result.returncode == 0
                      else _normalize(result.stdout, {}))
            outcomes.append((result.returncode, stdout, result.stderr, _dump(root, {})))
        return outcomes

    assert sequence(False) == sequence(True)
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    # `--destination mars` is an argparse usage error, also answered by Rust.
    assert {c["command"] for c in served} == {"reports.export-exit-tax-pdf", "<usage>"}


def test_exit_tax_pdf_daemon_envelope_and_native_routing(tmp_path, monkeypatch):
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
        records = []
        for raw in result.stdout.splitlines()[1:]:
            record = json.loads(raw)
            data = record.get("data")
            records.append(_exit_normalized(raw, Path(data["file"])) if record["kind"] != "error"
                           and isinstance(data, dict) and "file" in data else raw)
        return records, _dump(root, {}), sorted(str(p.relative_to(exports)) for p in exports.rglob("*"))

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


@pytest.mark.parametrize("country,algorithm,variant", [
    ("at", "MOVING_AVERAGE_AT", "ready"), ("at", "MOVING_AVERAGE_AT", "stale"), ("generic", "FIFO", "ready"),
])
def test_austrian_pdf_cli_envelope_completion_filing_and_native_routing(tmp_path, monkeypatch, country, algorithm,
                                                                        variant):
    root, database, env, call = _book(tmp_path, monkeypatch, country, algorithm, variant)
    before = database.read_bytes()
    target = tmp_path / "nested" / "austrian.pdf"

    def sequence(reference):
        database.write_bytes(before)
        shutil.rmtree(target.parent, ignore_errors=True)
        identities, outcomes = {}, []
        for index, (command, flags) in enumerate([
                ("export-austrian-e1kv-pdf", ["--year", "2025"]), ("export-austrian", ["--year", "2024"]),
                ("export-austrian-e1kv-pdf", ["--year", "2008"]), ("export-austrian", [])]):
            result = call("--machine", "reports", command, "--file", str(target), *flags, reference=reference)
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

    native, reference = sequence(False), sequence(True)
    for actual, expected in zip(native, reference):
        assert actual == expected
    assert native == reference
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} <= {"reports.export-austrian-e1kv-pdf", "reports.export-austrian",
                                              "reports.filed-snapshots.create", "<usage>"}


def test_austrian_pdf_daemon_envelope_errors_collisions_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _book(tmp_path, monkeypatch, "at", "MOVING_AVERAGE_AT")
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{"year": 2025}, {"year": 2025}, {"year": 2024}, {}, {"year": "2025"}, {"year": 1999}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.reports.export_austrian_e1kv_pdf", args=args)) + "\n")
                    .encode() for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

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

    native, reference = session(False), session(True)
    for actual, expected in zip(native[0], reference[0]):
        assert actual == expected
    assert native == reference
    assert _served(env) == [{"command": "daemon", "served_by": "rust"}]


def _source_funds_book(tmp_path, monkeypatch):
    """A book with the saved source-of-funds case of the PDF fixtures."""
    root, database, env, call = _book(tmp_path, monkeypatch, "generic", "FIFO")
    vectors = json.loads((Path(__file__).resolve().parents[1]
                          / "crates/reports/tests/data/source_funds_pdf.json").read_text())
    case = next(c for c in vectors["cases"] if c["name"] == "standard")
    conn = sqlite3.connect(database)
    conn.executescript(vectors["base_sql"] + case["sql"])
    conn.commit()
    conn.close()
    return root, database, env, call


def test_source_funds_pdf_cli_envelope_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _source_funds_book(tmp_path, monkeypatch)
    before = database.read_bytes()
    target = tmp_path / "nested" / "source-funds.pdf"

    def sequence(reference):
        database.write_bytes(before)
        shutil.rmtree(target.parent, ignore_errors=True)
        outcomes = []
        for flags in [["--case", "case-1"], ["--case", "case-1"], ["--case", "missing"], [], ["--case", "c-z"],
                      ["--case", "c-a"]]:
            result = call("--machine", "reports", "export-source-funds-pdf", "--file", str(target), *flags,
                          reference=reference)
            stdout = (_exit_normalized(result.stdout, target) if result.returncode == 0
                      else _normalize(result.stdout, {}))
            outcomes.append((result.returncode, stdout, result.stderr, _dump(root, {})))
        return outcomes

    native, reference = sequence(False), sequence(True)
    for actual, expected in zip(native, reference):
        assert actual == expected
    assert native == reference
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} == {"reports.export-source-funds-pdf"}


def test_source_funds_pdf_daemon_envelope_errors_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call = _source_funds_book(tmp_path, monkeypatch)
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{"case": "case-1"}, {"case": "case-1"}, {"case": 1}, {}, {"case": "missing"}, {"case": None}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.source_funds.export_pdf", args=args)) + "\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        records = []
        for raw in result.stdout.splitlines()[1:]:
            record = json.loads(raw)
            data = record.get("data")
            records.append(_exit_normalized(raw, Path(data["file"])) if record["kind"] != "error"
                           and isinstance(data, dict) and "file" in data else raw)
        return records, _dump(root, {}), sorted(str(p.relative_to(exports)) for p in exports.rglob("*.pdf"))

    native, reference = session(False), session(True)
    for actual, expected in zip(native[0], reference[0]):
        assert actual == expected
    assert native == reference
    assert _served(env) == [{"command": "daemon", "served_by": "rust"}]


def _bundle_book(tmp_path, monkeypatch):
    """The source-of-funds book with cases for the bundle: `case-1` with its
    evidence file on disk and hashed in the snapshot, `c-labels` withholding
    evidence, `c-changed` whose file no longer matches and `c-gone` whose
    file is missing."""
    from kassiber.core.source_funds import _snapshot_hash
    from kassiber.db import resolve_attachments_root

    root, database, env, call = _source_funds_book(tmp_path, monkeypatch)
    evidence = b"%PDF-1.4 invoice evidence\n"
    stored = resolve_attachments_root(root) / "not-read" / "secret.pdf"
    stored.parent.mkdir(parents=True)
    stored.write_bytes(evidence)
    conn = sqlite3.connect(database)
    conn.row_factory = sqlite3.Row
    row = dict(conn.execute("SELECT * FROM source_funds_cases WHERE id='case-1'").fetchone())

    def save(case_id, sha, reveal="standard"):
        snapshot = json.loads(row["snapshot_json"])
        snapshot["reveal_mode"] = reveal
        for item in snapshot["disclosure_preview"]["attachments"]:
            if item["id"] == "a-a":
                item["sha256"] = sha
        if isinstance(snapshot.get("case"), dict):
            snapshot["case"]["id"] = case_id
        fields = dict(row, id=case_id, reveal_mode=reveal, snapshot_json=json.dumps(snapshot),
                      snapshot_hash=_snapshot_hash(snapshot))
        conn.execute("DELETE FROM source_funds_cases WHERE id=?", (case_id,))
        conn.execute(f"INSERT INTO source_funds_cases({','.join(fields)}) VALUES({','.join('?' * len(fields))})",
                     tuple(fields.values()))

    digest = hashlib.sha256(evidence).hexdigest()
    save("case-1", digest)
    save("c-labels", digest, "labels_only")
    save("c-changed", "0" * 64)
    conn.commit()
    conn.close()
    return root, database, env, call, stored


def _bundle_normalized(stdout, artifact):
    """A bundle's response and contents. The PDF inside is the shared
    design's, so the ZIP's size and the PDF's hash differ; the manifest,
    member names and evidence bytes must not."""
    payload = json.loads(stdout)
    data = payload["data"]
    assert data["bytes"] == artifact.stat().st_size
    data["bytes"] = "<zip>"
    with zipfile.ZipFile(artifact) as bundle:
        names = bundle.namelist()
        assert bundle.read("source-of-funds-report.pdf").startswith(b"%PDF-")
        manifest = json.loads(bundle.read("manifest.json"))
        manifest["report_pdf"]["sha256"] = "<pdf-sha256>"
        # The reference process freezes the report clock, not source-funds'
        # `now_iso`; Rust's harness freezes both.
        assert manifest["generated_at"].endswith("Z")
        manifest["generated_at"] = "<generated-at>"
        evidence = {name: bundle.read(name) for name in names if name.startswith("evidence/")}
    return json.dumps(payload, sort_keys=True).encode(), names, manifest, evidence


def test_source_funds_bundle_cli_envelope_contents_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call, stored = _bundle_book(tmp_path, monkeypatch)
    before = database.read_bytes()
    target = tmp_path / "nested" / "bundle.out"

    def sequence(reference):
        database.write_bytes(before)
        shutil.rmtree(target.parent, ignore_errors=True)
        outcomes = []
        for flags in [["--case", "case-1"], ["--case", "case-1"], ["--case", "c-labels"], ["--case", "c-changed"],
                      ["--case", "missing"], [], ["--case", "c-z"], ["--case", "c-a"], ["--case", "gone"]]:
            if flags == ["--case", "gone"]:
                stored.rename(stored.with_suffix(".moved"))
                flags = ["--case", "case-1"]
            result = call("--machine", "reports", "export-source-funds-bundle", "--file", str(target), *flags,
                          reference=reference)
            stdout = (_bundle_normalized(result.stdout, target.with_suffix(".zip")) if result.returncode == 0
                      else _normalize(result.stdout, {}))
            outcomes.append((result.returncode, stdout, result.stderr, _dump(root, {})))
        stored.with_suffix(".moved").rename(stored)
        return outcomes

    native, reference = sequence(False), sequence(True)
    for actual, expected in zip(native, reference):
        if isinstance(actual[1], tuple) and isinstance(expected[1], tuple):
            for part, (mine, theirs) in zip(("envelope", "members", "manifest", "evidence"),
                                            zip(actual[1], expected[1])):
                assert mine == theirs, part
        assert actual == expected
    assert native == reference
    served = _served(env)
    assert served and all(c["served_by"] == "rust" for c in served)
    assert {c["command"] for c in served} == {"reports.export-source-funds-bundle"}


def test_source_funds_bundle_daemon_envelope_and_native_routing(tmp_path, monkeypatch):
    root, database, env, call, _ = _bundle_book(tmp_path, monkeypatch)
    before = database.read_bytes()
    exports = tmp_path / "exports"
    requests = [{"case": "case-1"}, {"case": "case-1"}, {"case": "c-labels"}, {"case": 1}, {}, {"case": "missing"},
                {"case": "c-changed"}, {"case": None}]
    wire = b"".join((json.dumps(dict(request_id=i, kind="ui.source_funds.export_bundle", args=args)) + "\n").encode()
                    for i, args in enumerate(requests)) + b'{"request_id":"done","kind":"daemon.shutdown"}\n'

    def session(reference):
        shutil.rmtree(exports, ignore_errors=True)
        database.write_bytes(before)
        result = call("daemon", reference=reference, wire=wire)
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        records = []
        for raw in result.stdout.splitlines()[1:]:
            record = json.loads(raw)
            data = record.get("data")
            records.append(_bundle_normalized(raw, Path(data["file"])) if record["kind"] != "error"
                           and isinstance(data, dict) and "file" in data else raw)
        return records, _dump(root, {}), sorted(str(p.relative_to(exports)) for p in exports.rglob("*"))

    native, reference = session(False), session(True)
    for actual, expected in zip(native[0], reference[0]):
        assert actual == expected
    assert native == reference
    assert _served(env) == [{"command": "daemon", "served_by": "rust"}]
