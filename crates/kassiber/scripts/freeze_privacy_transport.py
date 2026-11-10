"""Freeze the Python reference the privacy transport tests compared against.

`crates/kassiber/tests/{legacy_privacy_transport,privacy_reports,psbt_privacy}.rs`
used to run pytest modules and inline scripts that started this checkout's
Python CLI next to the Rust binary on one book and compared their bytes. This
recorder replays the same flows once, Python side only, and writes what the
Python reference printed to `crates/kassiber/tests/data/privacy_transport.json`.
The Rust tests now build the same books themselves and compare the binary with
that file, so they need no Python (docs/plan/22-python-removal.md).

Run from the repository root, before Python is removed:

    uv run --locked python crates/kassiber/scripts/freeze_privacy_transport.py

Paths under each session's scratch root are written as `{root}`, the home
directory as `{home}`. Every text is stored once under `blobs` and named by its
`sha256:` key. A daemon's first line (`daemon.ready`) is not frozen,
because the tests never compared it. Where the tests required the Rust daemon
to answer an unhandled error with `"debug": ""` and no traceback while Python
printed one, the frozen line is Python's with that one substitution, as the
tests asserted, and the traceback's last line is kept as a note.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
# tests.test_rust_report_exports reads it at import to name the hybrid
# candidate; only the Python reference runs here.
os.environ.setdefault("KASSIBER_RUST_BIN", str(ROOT / "target/debug/kassiber"))

from kassiber.db import open_db  # noqa: E402
from tests.rust_port.book_schema import TABLES  # noqa: E402
from tests.test_rust_report_exports import _book, _dump  # noqa: E402

OUTPUT = ROOT / "crates/kassiber/tests/data/privacy_transport.json"
WORK = ROOT / "target/freeze-privacy-transport"
DATA = ROOT / "crates/reports/tests/data"
LEGACY_KIND = "ui.privacy_hygiene.snapshot"
PSBT_KIND = "ui.reports.psbt_privacy"
HYGIENE_KIND = "ui.reports.privacy_hygiene"
TXID = "ab" * 32


def text(raw: bytes, root: Path) -> str:
    replaced = raw.replace(str(root).encode(), b"{root}").replace(
        str(Path.home()).encode(), b"{home}")
    return replaced.decode("utf-8")


def wire(requests) -> bytes:
    return b"".join((json.dumps(request) + "\n").encode() for request in requests)


class Session:
    def __init__(self, name: str, *, case_file: str | None, case: str | None,
                 offline: bool | None, profile_field: str | None = None):
        self.root = WORK / name.replace("/", "-")
        self.root.mkdir(parents=True)
        self.record = {"name": name, "case_file": case_file, "case": case,
                       "offline": offline, "corrupt_profile_field": profile_field,
                       "files": [], "steps": []}
        # The private marker the fixtures carry; psbt sessions also pass a
        # literal "~/sentinel-missing-proposal" path that errors echo.
        self.sentinel = b"sentinel-private-" if name.startswith("psbt") else b"sentinel-"
        self.monkey = pytest.MonkeyPatch()
        data, _, env, call = _book(self.root, self.monkey, "generic", "FIFO")
        self.data, self.env, self.call = data, env, call
        if offline:
            env["KASSIBER_NO_EGRESS"] = "1"
        else:
            env.pop("KASSIBER_NO_EGRESS", None)
        assert call("--machine", "status", reference=True).returncode == 0
        if case is not None:
            cases = json.loads((DATA / case_file).read_text())
            sql = next(c for c in cases if c["name"] == case)["sql"]
            conn = open_db(data)
            conn.executescript(sql)
            conn.commit()
            conn.close()
            open_db(data).close()
        if profile_field is not None:
            conn = open_db(data)
            conn.execute(f"UPDATE profiles SET {profile_field}='bad' WHERE id='p'")
            conn.commit()
            conn.close()
        self.before = _dump(data, {})
        assert tuple(sorted(self.before)) == TABLES
        self.record["table_rows"] = {table: len(rows) for table, rows in sorted(self.before.items())}

    def unchanged(self):
        assert _dump(self.data, {}) == self.before, self.record["name"]

    def file(self, relative: str, raw: bytes) -> Path:
        path = self.root / relative
        path.write_bytes(raw)
        self.record["files"].append({"path": relative, "bytes": list(raw)})
        return path

    def cli(self, *argv, output: Path | None = None, before: bytes | None = None):
        expected = self.call(*argv, reference=True)
        self.unchanged()
        step = {"mode": "cli", "argv": [text(str(a).encode(), self.root) for a in argv],
                "returncode": expected.returncode,
                "stdout": text(expected.stdout, self.root),
                "stderr": text(expected.stderr, self.root)}
        assert self.sentinel not in expected.stdout
        if output is not None:
            step["output"] = {"path": text(str(output).encode(), self.root),
                              "before": None if before is None else before.decode(),
                              "after": output.read_bytes().decode()}
        self.record["steps"].append(step)
        return expected

    def daemon(self, requests, *flags, kind: str, debug_line: int | None = None,
               debug_error: str | None = None):
        expected = self.call(*flags, "daemon", reference=True, wire=wire(requests))
        self.unchanged()
        assert expected.returncode == 0
        lines = expected.stdout.splitlines(keepends=True)
        assert len(lines) == len(requests) + 1
        stderr = expected.stderr
        note = None
        if debug_line is not None:
            error = json.loads(lines[debug_line])["error"]
            assert error["code"] == "internal_error"
            assert isinstance(error["debug"], str) and debug_error in error["debug"]
            debug_bytes = json.dumps(error["debug"], ensure_ascii=False).encode()
            assert lines[debug_line].count(debug_bytes) == 1
            lines[debug_line] = lines[debug_line].replace(debug_bytes, b'""', 1)
            assert stderr.startswith(b"Traceback (most recent call last):\n")
            note = stderr.decode().splitlines()[-1]
            stderr = b""
        assert stderr == b""
        assert self.sentinel not in expected.stdout
        self.record["steps"].append({
            "mode": "daemon", "flags": list(flags), "kind": kind,
            "stdin": text(wire(requests), self.root),
            "returncode": 0, "stdout_after_ready": text(b"".join(lines[1:]), self.root),
            "stderr": "", "python_traceback": note})

    def close(self) -> dict:
        self.monkey.undo()
        return self.record


def legacy() -> list[dict]:
    sessions = []
    names = ["common-input-5", "inventory-51", "other-marker-1-main-" + TXID,
             "cached-low-r-and-sighash", "ordering-limit-10", "empty-profile"]
    values = [None, {}, {"wallet": "cold"}, {"wallet": "Cold"}, {"transaction": "legacy"},
              {"transaction": TXID.upper()}, {"limit": 1}, {"limit": "٢"},
              {"limit": 200}, {"limit": 201}, {"limit": 0}, {"limit": False}, {"limit": "bad"},
              {"limit": {"z": 2, "a": 1}}, {"limit": []},
              {"wallet": "missing"}, {"transaction": "missing"}, {"z": 1, "a": 2}, [], False, "", 3,
              {"wallet": {"z": 2, "a": 1}}, {"transaction": [2, 1]}]
    for offline in (False, True):
        for name in names:
            session = Session(f"legacy-daemon/{offline}/{name}", case_file="legacy_privacy.json",
                              case=name, offline=offline)
            requests = [dict(kind=LEGACY_KIND, request_id=i, args=value) for i, value in enumerate(values, 1)]
            requests.extend([
                dict(kind=LEGACY_KIND, request_id="scope-match",
                     args={"expected_scope": {"workspace_id": "ws", "profile_id": "p"}}),
                dict(kind=LEGACY_KIND, request_id="scope-stale",
                     args={"expected_scope": {"workspace_id": "ws", "profile_id": "wrong"}}),
                dict(kind=LEGACY_KIND, request_id=None), dict(kind=LEGACY_KIND),
            ])
            session.daemon(requests, kind=LEGACY_KIND)
            sessions.append(session.close())
        session = Session(f"legacy-cli/{offline}", case_file="legacy_privacy.json",
                          case="common-input-5", offline=offline)
        for flags in [("--machine",), ("--format", "json"), ("--format", "table"),
                      ("--format", "plain"), ("--format", "csv")]:
            session.cli(*flags, "reports", "privacy-hygiene")
        output = session.root / "configuration.json"
        session.cli("--machine", "--output", str(output), "reports", "privacy-hygiene", output=output)
        sessions.append(session.close())
    for debug in (False, True):
        session = Session(f"legacy-corrupt/{debug}", case_file="legacy_privacy.json",
                          case="invalid-amount-bad-None", offline=False)
        requests = [dict(kind=LEGACY_KIND, request_id=1, args={"unknown": True}),
                    dict(kind=LEGACY_KIND, request_id=2), dict(kind=LEGACY_KIND, request_id=3, args=False)]
        session.daemon(requests, *(("--debug",) if debug else ()), kind=LEGACY_KIND,
                       debug_line=2, debug_error="ValueError")
        sessions.append(session.close())
    return sessions


def hygiene() -> list[dict]:
    sessions = []
    selected = ["baseline", "raw-boundary-quarantine", "disabled-provider", "wallet-descriptor",
                "no-active-transactions", "malformed-journal_input_version"]
    for name in selected:
        session = Session(f"hygiene/{name}", case_file="privacy_hygiene.json", case=name, offline=None)
        for flags in [("--format", "json"), ("--format", "table"), ("--format", "plain"),
                      ("--format", "csv"), ("--machine",)]:
            session.cli(*flags, "reports", "privacy-hygiene")
        for scope in [("other",), ("missing",)]:
            session.cli("--machine", "reports", "privacy-hygiene", "--profile", *scope)
        output = session.root / "report.json"
        session.cli("--machine", "--output", str(output), "reports", "privacy-hygiene", output=output)
        if not name.startswith("malformed-"):
            requests = [dict(request_id=i, kind=HYGIENE_KIND, args=value)
                        for i, value in enumerate([None, {}, {"wallet": "cold"}, {"z": 1, "a": 2}, [], "", True], 1)]
            requests.append(dict(request_id=8, kind=HYGIENE_KIND, expected_scope={"profile_id": "wrong"}))
            session.daemon(requests, kind=HYGIENE_KIND)
        sessions.append(session.close())
    for field in ("journal_input_version", "last_processed_input_version", "last_processed_tx_count"):
        for debug in (False, True):
            session = Session(f"hygiene-debug/{field}/{debug}", case_file=None, case=None,
                              offline=None, profile_field=field)
            requests = [dict(request_id=1, kind=HYGIENE_KIND, args={"unknown": True}),
                        dict(request_id=2, kind=HYGIENE_KIND),
                        dict(request_id=3, kind=HYGIENE_KIND, args=False)]
            session.daemon(requests, *(("--debug",) if debug else ()), kind=HYGIENE_KIND,
                           debug_line=2, debug_error="ValueError")
            sessions.append(session.close())
    return sessions


def psbt() -> list[dict]:
    sessions = []
    cases = {c["name"]: c for c in json.loads((DATA / "psbt_history.json").read_text())}
    for offline in (False, True):
        for name in ["inventory/unlinked-inputs", "spend/2/None/confirmed/both",
                     "spend/2/coinjoin/confirmed/both", "source/dataset-observer-disclosed"]:
            session = Session(f"psbt/{offline}/{name}", case_file="psbt_history.json", case=name,
                              offline=offline)
            proposal = cases[name]["psbt"]
            text_file = session.file("proposal.psbt", ("\x1c\n" + proposal + "\r\n ").encode())
            for flags in [(), ("--machine",), *[("--format", fmt) for fmt in ("json", "table", "plain", "csv")]]:
                for source in [("--psbt", proposal), ("--psbt-file", str(text_file))]:
                    assert session.cli(*flags, "reports", "psbt-privacy", *source).returncode == 0
            assert session.cli("--machine", "--format", "table", "reports", "psbt-privacy",
                               "--psbt", proposal).returncode == 1
            for scope in [("--workspace", "Main", "--profile", "Book"), ("--profile", "other"),
                          ("--profile", "missing")]:
                session.cli("--machine", "reports", "psbt-privacy", "--psbt", proposal, *scope)
            for file_name, raw in [("empty", b""), ("nul", b"before\0after"), ("utf8", b"\xff"),
                                   ("invalid", b"not-a-PSBT")]:
                path = session.file(file_name, raw)
                session.cli("--machine", "reports", "psbt-privacy", "--psbt-file", str(path),
                            "--profile", "missing")
            for path in [str(session.root / "missing"), "~/sentinel-missing-proposal"]:
                session.cli("--machine", "reports", "psbt-privacy", "--psbt-file", path)
            session.cli("--machine", "reports", "psbt-privacy", "--psbt", "invalid")
            for format_ in ["json", "csv"]:
                output = session.root / f"report.{format_}"
                session.cli("--format", format_, "--output", str(output), "reports", "psbt-privacy",
                            "--psbt", proposal, output=output, before=b"previous complete artifact\r\n")
            values = [None, {}, [], "", False, 1, {"psbt": None}, {"psbt": []}, {"psbt": 42},
                      {"psbt": "\x1c \n"}, {"psbt": proposal}, {"psbt": " \n" + proposal + "\r\n"},
                      {"psbt": "invalid"}, {"psbt": "\ud800"}, {"psbt": proposal + "\udfff"},
                      {"\ud800": 1, "a": 2}, {"psbt": proposal, "profile": "other"}, {"z": 1, "a": 2},
                      {"psbt": proposal, "baseline": {}}]
            requests = [dict(request_id=index, kind=PSBT_KIND, args=value) for index, value in enumerate(values, 1)]
            requests.extend([
                dict(request_id="scope-first", kind=PSBT_KIND, args={"unknown": True},
                     expected_scope={"profile_id": "wrong"}),
                dict(request_id="scope-valid", kind=PSBT_KIND, args={"psbt": proposal},
                     expected_scope={"profile_id": "p"}),
            ])
            session.daemon(requests, kind=PSBT_KIND)
            assert all(proposal not in step.get("stdout", step.get("stdout_after_ready", ""))
                       for step in session.record["steps"])
            sessions.append(session.close())
    for offline in (False, True):
        for debug in (False, True):
            session = Session(f"psbt-debug/{offline}/{debug}", case_file="psbt_history.json",
                              case="inventory/embedded-reference-bug", offline=offline)
            proposal = cases["inventory/embedded-reference-bug"]["psbt"]
            requests = [dict(request_id=index, kind=PSBT_KIND, args=args)
                        for index, args in enumerate([{"unknown": True}, {"psbt": proposal}, False], 1)]
            session.daemon(requests, *(("--debug",) if debug else ()), kind=PSBT_KIND,
                           debug_line=2, debug_error="RuntimeError")
            sessions.append(session.close())
    session = Session("psbt-routes", case_file="psbt_history.json", case="spend/2/None/confirmed/both",
                      offline=False)
    proposal = cases["spend/2/None/confirmed/both"]["psbt"]
    assert session.cli("--machine", "reports", "psbt-privacy", "--psbt", proposal).returncode == 0
    session.daemon([dict(request_id=1, kind=PSBT_KIND, args={"psbt": proposal})], kind=PSBT_KIND)
    sessions.append(session.close())
    return sessions


def main() -> int:
    os.environ.pop("KASSIBER_NO_EGRESS", None)
    if WORK.exists():
        shutil.rmtree(WORK)
    WORK.mkdir(parents=True)
    payload = {"legacy": legacy(), "hygiene": hygiene(), "psbt": psbt()}
    # Many sessions print the same bytes (online and offline, json and
    # --machine); keep each text once.
    blobs = {}

    def blob(value: str) -> str:
        key = "sha256:" + hashlib.sha256(value.encode()).hexdigest()
        blobs[key] = value
        return key

    for sessions in payload.values():
        for session in sessions:
            for step in session["steps"]:
                for field in ("stdin", "stdout", "stdout_after_ready", "stderr"):
                    if field in step:
                        step[field] = blob(step[field])
                if "output" in step:
                    step["output"]["after"] = blob(step["output"]["after"])
    payload["blobs"] = blobs
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(payload, indent=1, ensure_ascii=False, sort_keys=True) + "\n")
    shutil.rmtree(WORK)
    print(f"{OUTPUT.relative_to(ROOT)}: {sum(len(v) for k, v in payload.items() if k != 'blobs')} sessions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
