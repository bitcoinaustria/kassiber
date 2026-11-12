"""Whole-process native PSBT reads, hybrid routing, and native diagnostics."""
import json
import os
import subprocess
from pathlib import Path

import pytest

from kassiber.db import open_db
from tests.test_rust_report_exports import _book, _dump
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[1]
pytestmark = pytest.mark.skipif(not os.environ.get("KASSIBER_RUST_BIN"), reason="requires Rust test-harness binary")
COMMAND = "reports.psbt-privacy"
KIND = "ui.reports.psbt_privacy"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def setup(tmp_path, monkeypatch, name, offline):
    data, database, env, call = _book(tmp_path, monkeypatch, "generic", "FIFO")
    env.pop("KASSIBER_NO_EGRESS", None)
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    assert call("--machine", "status", reference=True).returncode == 0
    cases = json.loads((ROOT / "crates/reports/tests/data/psbt_history.json").read_text())
    case = next(case for case in cases if case["name"] == name)
    with open_db(data) as conn:
        conn.executescript(case["sql"])
    conn.close()
    conn = open_db(data)
    conn.close()
    before = _dump(data, {})
    assert tuple(sorted(before)) == TABLES
    return data, env, call, case["psbt"], canonical(before)


def unchanged(data, before):
    assert canonical(_dump(data, {})) == before


def routed(env, cli, daemon):
    records = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert records and all(record["served_by"] == "rust" for record in records), records
    assert records.count({"command": COMMAND, "served_by": "rust"}) == cli
    assert records.count({"command": "daemon", "served_by": "rust"}) == daemon
    assert len(records) == cli + daemon


SELECTED = ["inventory/unlinked-inputs", "spend/2/None/confirmed/both",
            "spend/2/coinjoin/confirmed/both", "source/dataset-observer-disclosed"]


# One selected case per test id, so xdist runs the four whole-process
# comparisons side by side.
@pytest.mark.parametrize("offline", [False, True])
@pytest.mark.parametrize("number,name", list(enumerate(SELECTED)), ids=[n.replace("/", "-") for n in SELECTED])
def test_psbt_product_bytes_files_all_tables_and_zero_hybrid_fallback(tmp_path, monkeypatch, offline, number, name):
    root = tmp_path / str(number)
    root.mkdir()
    data, env, call, proposal, before = setup(root, monkeypatch, name, offline)
    cli_count = 0

    def compare(*args, output=None):
        nonlocal cli_count
        expected = call(*args, reference=True)
        unchanged(data, before)
        saved = output.read_bytes() if output and output.exists() else None
        if saved is not None:
            output.write_bytes(b"previous complete artifact\r\n")
        actual = call(*args)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), (name, args, actual.stdout, expected.stdout, actual.stderr, expected.stderr)
        if saved is not None:
            assert output.read_bytes() == saved
        unchanged(data, before)
        assert b"sentinel-private-" not in actual.stdout
        assert proposal.encode() not in actual.stdout
        cli_count += 1
        return actual

    text_file = root / "proposal.psbt"
    text_file.write_text("\x1c\n" + proposal + "\r\n ")
    for flags in [(), ("--machine",), *[("--format", fmt) for fmt in ("json", "table", "plain", "csv")]]:
        for source in [("--psbt", proposal), ("--psbt-file", str(text_file))]:
            assert compare(*flags, "reports", "psbt-privacy", *source).returncode == 0
    args = ("--machine", "--format", "table", "reports", "psbt-privacy", "--psbt", proposal)
    expected = call(*args, reference=True)
    actual = subprocess.run([os.environ["KASSIBER_RUST_BIN"], "--data-root", str(data), *args],
                            env=env, capture_output=True, timeout=30)
    assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr)
    assert actual.returncode == 1
    unchanged(data, before)
    for scope in [("--workspace", "Main", "--profile", "Book"), ("--profile", "other"), ("--profile", "missing")]:
        compare("--machine", "reports", "psbt-privacy", "--psbt", proposal, *scope)
    for name_, raw in [("empty", b""), ("nul", b"before\0after"), ("utf8", b"\xff"), ("invalid", b"not-a-PSBT")]:
        file = root / name_
        file.write_bytes(raw)
        compare("--machine", "reports", "psbt-privacy", "--psbt-file", str(file), "--profile", "missing")
    for path in [str(root / "missing"), "~/sentinel-missing-proposal"]:
        compare("--machine", "reports", "psbt-privacy", "--psbt-file", path)
    compare("--machine", "reports", "psbt-privacy", "--psbt", "invalid")
    for format_ in ["json", "csv"]:
        output = root / f"report.{format_}"
        compare("--format", format_, "--output", str(output), "reports", "psbt-privacy", "--psbt", proposal, output=output)
    values = [None, {}, [], "", False, 1, {"psbt": None}, {"psbt": []}, {"psbt": 42}, {"psbt": "\x1c \n"},
              {"psbt": proposal}, {"psbt": " \n" + proposal + "\r\n"}, {"psbt": "invalid"},
              {"psbt": "\ud800"}, {"psbt": proposal + "\udfff"}, {"\ud800": 1, "a": 2},
              {"psbt": proposal, "profile": "other"}, {"z": 1, "a": 2}, {"psbt": proposal, "baseline": {}}]
    requests = [dict(request_id=index, kind=KIND, args=value) for index, value in enumerate(values, 1)]
    requests.extend([
        dict(request_id="scope-first", kind=KIND, args={"unknown": True}, expected_scope={"profile_id": "wrong"}),
        dict(request_id="scope-valid", kind=KIND, args={"psbt": proposal}, expected_scope={"profile_id": "p"}),
    ])
    wire = b"".join((json.dumps(request) + "\n").encode() for request in requests)
    expected = call("daemon", reference=True, wire=wire)
    unchanged(data, before)
    actual = call("daemon", wire=wire)
    assert actual.returncode == expected.returncode == 0
    assert actual.stderr == expected.stderr == b""
    native = actual.stdout.splitlines(keepends=True)
    reference = expected.stdout.splitlines(keepends=True)
    assert len(native) == len(reference) == len(requests) + 1
    assert KIND in json.loads(native[0])["data"]["supported_kinds"]
    assert native[1:] == reference[1:], (name, native[1:], reference[1:])
    assert b"sentinel-private-" not in actual.stdout
    assert proposal.encode() not in actual.stdout
    unchanged(data, before)
    routed(env, cli_count, 1)
    print(json.dumps({"offline": offline, "case": name, "rust_cli": cli_count, "rust_daemon_sessions": 1,
                      "rust_psbt_requests": len(requests), "python_fallbacks": 0, "tables": len(TABLES)}, sort_keys=True))


@pytest.mark.parametrize("offline", [False, True])
@pytest.mark.parametrize("debug", [False, True])
def test_psbt_decoder_daemon_preserves_native_unhandled_debug_contract(tmp_path, monkeypatch, offline, debug):
    data, env, call, proposal, before = setup(tmp_path, monkeypatch, "inventory/embedded-reference-bug", offline)
    wire = b"".join((json.dumps(dict(request_id=index, kind=KIND, args=args)) + "\n").encode()
                    for index, args in enumerate([{"unknown": True}, {"psbt": proposal}, False], 1))
    flags = ("--debug",) if debug else ()
    expected = call(*flags, "daemon", reference=True, wire=wire)
    unchanged(data, before)
    actual = call(*flags, "daemon", wire=wire)
    assert actual.returncode == expected.returncode == 0
    native = actual.stdout.splitlines(keepends=True)
    reference = expected.stdout.splitlines(keepends=True)
    assert len(native) == len(reference) == 4
    for index in [1, 3]:
        assert native[index] == reference[index]
        assert json.loads(native[index])["error"]["debug"] is None
    error = json.loads(reference[2])["error"]
    assert error["code"] == "internal_error"
    assert error["message"] == "Can't read one byte from the stream"
    assert isinstance(error["debug"], str) and "RuntimeError" in error["debug"]
    assert json.loads(native[2])["error"]["debug"] == ""
    debug_bytes = json.dumps(error["debug"], ensure_ascii=False).encode()
    assert reference[2].count(debug_bytes) == 1
    assert native[2] == reference[2].replace(debug_bytes, b'""', 1)
    assert expected.stderr.startswith(b"Traceback (most recent call last):\n")
    assert expected.stderr.endswith(b"RuntimeError: Can't read one byte from the stream\n")
    assert actual.stderr == b""
    unchanged(data, before)
    routed(env, 0, 1)
    print(json.dumps({"offline": offline, "debug": debug, "rust_daemon_sessions": 1,
                      "rust_psbt_requests": 3, "python_fallbacks": 0, "tables": len(TABLES)}, sort_keys=True))


def test_psbt_registered_product_routes_and_wire_order(tmp_path, monkeypatch):
    data, env, call, proposal, before = setup(tmp_path, monkeypatch, "spend/2/None/confirmed/both", False)
    args = ("--machine", "reports", "psbt-privacy", "--psbt", proposal)
    expected = call(*args, reference=True)
    actual = call(*args)
    assert actual.returncode == expected.returncode == 0
    assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
    unchanged(data, before)
    wire = (json.dumps(dict(request_id=1, kind=KIND, args={"psbt": proposal})) + "\n").encode()
    expected = call("daemon", reference=True, wire=wire)
    actual = call("daemon", wire=wire)
    assert actual.returncode == expected.returncode == 0
    assert actual.stderr == expected.stderr == b""
    native = actual.stdout.splitlines(keepends=True)
    assert KIND in json.loads(native[0])["data"]["supported_kinds"]
    assert native[1:] == expected.stdout.splitlines(keepends=True)[1:]
    payload = json.loads(native[1])["data"]
    assert payload["read_only"] is True
    assert payload["summary"]["decode_status"] == "decoded"
    unchanged(data, before)
    routed(env, 1, 1)
    print(json.dumps({"rust_cli": 1, "rust_daemon_sessions": 1, "rust_psbt_requests": 1,
                      "python_fallbacks": 0, "tables": len(TABLES)}, sort_keys=True))
