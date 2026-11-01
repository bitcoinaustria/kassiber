"""Direct native transport acceptance before product registration."""
from contextlib import closing
import importlib
import base64
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env
from tests.test_rust_psbt_history import REFERENCE
from kassiber.errors import AppError

CLI = importlib.import_module("kassiber.cli.main")
DAEMON = importlib.import_module("kassiber.daemon")
PARSER = CLI.build_parser()


def expected(conn, case):
    try:
        if case["transport"] == "daemon":
            args = DAEMON._coerce_args_dict("fixture", case.get("args"))
            payload = DAEMON._reports_psbt_privacy_payload(conn, args)
        else:
            args = PARSER.parse_args(case["argv"])
            text = args.psbt
            if text is None:
                text = CLI._read_optional_text_file(args.psbt_file, "PSBT")
            payload = REFERENCE.inventory.reports.report_psbt_privacy(conn, args.workspace, args.profile,
                CLI._report_hooks(), psbt_text=text or "")
            if args.format in {"table", "plain", "csv"}:
                payload = REFERENCE.inventory.reports.psbt_privacy_table_rows(payload)
        return {"ok": payload}
    except AppError as exc:
        return {"error": dict(code=exc.code, message=str(exc), hint=exc.hint, details=exc.details, retryable=exc.retryable)}
    except (ValueError, RuntimeError) as exc:
        return {"error": dict(code="internal_error", message=str(exc), hint=None, details=None, retryable=False)}


def cases(root, proposal):
    for flags in [[], ["--machine"], *[["--format", f] for f in ("json", "plain", "table", "csv")],
                  ["--machine", "--format", "table"], ["--machine", "--format", "plain"]]:
        for scope in ([], ["--workspace", "Main", "--profile", "Book"], ["--profile", "other"], ["--profile", "missing"]):
            yield dict(transport="cli", argv=[*flags, "reports", "psbt-privacy", "--psbt", proposal, *scope])
    files = [("base64", proposal.encode()),
        ("wrapped-crlf-at-text-bound", (proposal[:10] + "\r\n" * (3 * 1024 * 1024) + proposal[10:]).encode()), ("whitespace", ("\x1c\n" + proposal + "\r\n ").encode()),
        ("empty", b""), ("spaces", b" \n\r\t"), ("nul", b"before\0after"),
        ("invalid-utf8", b"\xff"), ("truncated-utf8", b"\xe2\x82"), ("continuation-utf8", b"abc\xe2\x82X"),
        ("bom", b"\xef\xbb\xbf" + proposal.encode()), ("invalid", b"not-a-PSBT"),
        ("hex", base64.b64decode(proposal).hex().encode()),
        ("binary", base64.b64decode(proposal)),
        ("oversize", b"a" * (2 * 1024 * 1024 + 1))]
    paths = []
    for name, data in files:
        path = root / name
        path.write_bytes(data)
        paths.append(str(path))
    paths.extend([str(root / "missing"), str(root), "", "~/sentinel-missing-proposal", "path\0with-nul"])
    for path in paths:
        for scope in ([], ["--profile", "missing"]):
            yield dict(transport="cli", argv=["--machine", "reports", "psbt-privacy", "--psbt-file", path, *scope])
    for raw in [None, {}, [], "", False, 1, "text", {"psbt": None}, {"psbt": []}, {"psbt": 42},
                {"psbt": "\x1c\t "}, {"psbt": "invalid"}, {"psbt": " \n" + proposal + "\r\n"},
                {"psbt": proposal}, {"psbt": "\ud800"}, {"psbt": proposal + "\udfff"}, {"\ud800": 1, "a": 2}, {"psbt": proposal, "profile": "other"}, {"z": 1, "a": 2},
                {"psbt": proposal, "database": "private", "baseline": {}}]:
        yield dict(transport="daemon", args=raw)


@pytest.mark.parametrize("offline", [False, True])
def test_psbt_cli_file_daemon_adapters_match_python_all_112_tables(tmp_path, offline):
    executable = os.environ.get("KASSIBER_PSBT_TRANSPORT_BIN")
    if not executable:
        pytest.skip("requires psbt_transport_fixture test-harness build")
    corpus = json.loads(REFERENCE.DEST.read_text())
    selected = ["inventory/unlinked-inputs", "spend/2/None/confirmed/both", "spend/2/coinjoin/confirmed/both"]
    counts = {"cli": 0, "daemon": 0}
    for number, name in enumerate(selected):
        case = next(c for c in corpus if c["name"] == name)
        root = tmp_path / f"book-{number}"
        root.mkdir()
        database = root / "book.sqlite"
        env = child_env(root)
        env["KASSIBER_TEST_NO_EGRESS"] = "strict"
        if offline:
            env["KASSIBER_NO_EGRESS"] = "1"
        else:
            env.pop("KASSIBER_NO_EGRESS", None)
        with closing(sqlite3.connect(database)) as conn:
            conn.row_factory = sqlite3.Row
            conn.executescript("PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;" + REFERENCE.OVERLAY.SEED.read_text() + REFERENCE.OVERLAY.BOOK.read_text() + case["sql"])
            before = REFERENCE.OVERLAY.tables(conn)
            for request in cases(root, case["psbt"]):
                actual = subprocess.run([str(Path(executable).resolve())], input=(json.dumps(dict(request, database=str(database))) + "\n").encode(),
                    capture_output=True, check=True, env=env, timeout=30)
                reference = expected(conn, request)
                assert actual.stdout == (REFERENCE.OVERLAY.canonical(reference) + "\n").encode(), (name, request["transport"], request.get("argv"), actual.stdout[:5000], reference)
                assert actual.stderr == b""
                assert b"sentinel-private-" not in actual.stdout
                assert REFERENCE.OVERLAY.tables(conn) == before
                counts[request["transport"]] += 1
    assert counts == {"cli": 210, "daemon": 60}


def test_psbt_transport_mutations_stay_applicable(tmp_path):
    root = Path(__file__).resolve().parents[1]
    catalog = root / "crates/reports/scripts/psbt_transport_mutations.json"
    result = subprocess.run([sys.executable, str(root / "crates/reports/scripts/psbt_transport_mutation_check.py"), "--check"],
                            env=child_env(tmp_path), capture_output=True, check=True, timeout=30)
    entries = json.loads(catalog.read_text())
    assert result.stdout == f"{len(entries)} entries apply\n".encode()
    source = (root / "crates/kassiber/tests/psbt_privacy.rs").read_text()
    assert all(f"fn {name}()" in source for entry in entries for name in entry["expect_failing"])


@pytest.mark.parametrize("offline", [False, True])
def test_psbt_historical_transport_preserves_unsorted_wire_bytes(tmp_path, offline):
    executable = os.environ.get("KASSIBER_PSBT_TRANSPORT_BIN")
    if not executable:
        pytest.skip("requires psbt_transport_fixture test-harness build")
    env = child_env(tmp_path)
    env["KASSIBER_TEST_NO_EGRESS"] = "strict"
    env.pop("KASSIBER_NO_EGRESS", None)
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    database = tmp_path / "book.sqlite"
    for case in json.loads(REFERENCE.DEST.read_text()):
        with closing(sqlite3.connect(database)) as conn:
            conn.row_factory = sqlite3.Row
            conn.executescript("PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;" + REFERENCE.OVERLAY.SEED.read_text() + REFERENCE.OVERLAY.BOOK.read_text() + case["sql"])
            argv = ["--machine", "reports", "psbt-privacy", "--psbt", case["psbt"]]
            for key, value in case["args"].items():
                argv.extend(["--" + key, value])
            request = dict(transport="cli", argv=argv, wire_order=True, database=str(database))
            before = REFERENCE.OVERLAY.tables(conn)
            reference = expected(conn, request)
            actual = subprocess.run([str(Path(executable).resolve())], input=(json.dumps(request) + "\n").encode(),
                                    env=env, capture_output=True, check=True, timeout=30)
            assert actual.stdout == (json.dumps(reference, ensure_ascii=False, separators=(",", ":")) + "\n").encode(), case["name"]
            assert actual.stderr == b""
            assert REFERENCE.OVERLAY.tables(conn) == before
        database.unlink()
