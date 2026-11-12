"""`kassiber mcp` in Rust (crates/mcp, R6-8) against Python.

Stdio transcripts: the same newline-delimited JSON-RPC session goes to
``python -m kassiber ... mcp serve`` and to the Rust test-harness binary on
separate copies of the same fixture book, and every reply must be identical
byte for byte (read-time clocks aside). Requests that are not tool calls
come first and calls are answered in order by the single tool worker, so
the reply order is deterministic. A tool the Rust executor still refuses
(``not_implemented``, R6-5a's list) is skipped and checked against that
list. The one-shot leaves (``mcp call``, ``tools``, ``status``, ``enable``,
``disable``) are compared on stdout bytes and exit codes.

Since #686 the user's External agents grant decides the listing and what a
call may do, so the transcripts run at every grant and access mode; writes
land on both copies in the same order. The privacy fences run against Rust
alone: launch and listing touch no file, open no socket and reach no
operator endpoint; the server never starts a broker; only the granted tools
are reachable.

Every child gets a disposable home and app-data root (``XDG_DATA_HOME``
included) and the operator runtime directory override, so nothing reaches
the account's real state or broker.
"""
from __future__ import annotations

from contextlib import closing
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from tests import cli_runner
from tests.egress_guard import guarded_pythonpath

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
FIXTURES = {
    "empty": None,
    "desktop": ROOT / "crates/transactions/tests/data/desktop.sql",
    "reports": ROOT / "crates/reports/tests/data/book.sql",
}
MODERN_META = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientCapabilities": {},
}
# Tools whose replies differ for a main-sync slice other than S12 (see
# docs/reference/rust-main-sync.md); not compared until that slice lands,
# which deletes its line.
PENDING = {
    "wallets_list": "S8 completeness_class and scan_bounds",
    "reports_capital_gains": "S7 the Reports payload (fiatCurrency, longTermDays, asset)",
    "transactions_resolve": "S2 sharedExternalIdCount and matches",
    "review_cases": "S4 fiat_currency",
    "review_badges": "S4 review and definitions",
    "review_worklist": "S4 badges review and definitions",
    "journals_quarantine": "S5 summary fiat_currency and definitions",
    "journals_quarantine_resolve": "S5 group_id",
    "transfers_suggest": "S6 suggestion exclusions",
    "transfers_review_context": "S6 suggestion exclusions",
    "workspace_health": "S10 definitions",
    "next_actions": "S10 health definitions",
    "report_blockers": "S10 health definitions",
}
PYTHON_CRASHES: set[str] = set()
PYTHON_CRASH_TEXT = b"AI tool execution failed unexpectedly"
GRANTS = ("read", "operator", "accounting_decisions")
ACCESS = ("full", "restricted")


def _refused() -> set[str]:
    """Tools the Rust executor still refuses with not_implemented
    (crates/ai/src/executor.rs, mutate.rs)."""
    from tests.test_rust_ai_tools import REFUSED_MUTATIONS, REFUSED_READS

    return REFUSED_READS | {name for name, _ in REFUSED_MUTATIONS}
_GENERATED_AT = re.compile(rb'(\\?"generated_at\\?":\\?")\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ')
_UUID = re.compile(rb"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
_SNAPSHOT_ID = re.compile(rb'(snapshot_id\\?":\\?")([0-9a-f]{64})')
# Chain-analysis references are keyed per process (`current_process_and_book`).
_REFERENCE = re.compile(rb"ca-ref:[0-9a-f]{64}")
_STAMP = re.compile(rb"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,6})?(?:Z|\+00:00)")


class _Masker:
    """Per side: generated ids (uuid4, analysis snapshot digests over them,
    per-process analysis references) in order of first use, and clocks
    sampled since the session began."""

    def __init__(self):
        self.since = datetime.now(timezone.utc).replace(microsecond=0)
        self.ids = {}
        self.snapshots = {}
        self.references = {}

    def _clock(self, match):
        text = match.group(0).decode()
        try:
            stamp = datetime.fromisoformat(text.replace("Z", "+00:00"))
        except ValueError:
            return match.group(0)
        return b"<now>" if stamp >= self.since else match.group(0)

    def __call__(self, line):
        line = _GENERATED_AT.sub(rb"\1<t>", line)
        line = _UUID.sub(lambda m: self.ids.setdefault(m.group(0), b"<uuid:%d>" % len(self.ids)), line)
        line = _SNAPSHOT_ID.sub(
            lambda m: m.group(1) + self.snapshots.setdefault(m.group(2), b"<snapshot:%d>" % len(self.snapshots)),
            line,
        )
        line = _REFERENCE.sub(
            lambda m: self.references.setdefault(m.group(0), b"ca-ref:<%d>" % len(self.references)),
            line,
        )
        return _STAMP.sub(self._clock, line)


def _request(request_id, method, params=None):
    message = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        message["params"] = params
    return message


def _call(request_id, name, arguments, *, modern=True):
    params = {"name": name, "arguments": arguments}
    if modern:
        params["_meta"] = MODERN_META
    return _request(request_id, "tools/call", params)


def _lines(messages) -> bytes:
    return "".join(json.dumps(message) + "\n" for message in messages).encode()


def _mcp_calls(tools=None):
    """Every MCP tool (or ``tools``) with the AI driver's argument cases."""
    from kassiber.mcp import tools as mcp_tools
    from tests.test_rust_ai_tools import _extra_arguments, _fill

    extra = _extra_arguments()
    calls = []
    for tool in tools if tools is not None else mcp_tools.mcp_tools():
        entry, name = tool.entry, tool.name
        cases = [{}]
        filled = _fill(entry.parameters)
        if filled and filled not in cases:
            cases.append(filled)
        cases += [case for case in extra.get(entry.name, []) if case not in cases]
        calls += [(entry.name, name, case) for case in cases]
    return calls


class McpMutationCatalogTest(unittest.TestCase):
    def test_mutation_catalog_stays_applicable(self):
        catalog = ROOT / "crates/mcp/scripts/mutations.json"
        entries = json.loads(catalog.read_text(encoding="utf-8"))
        result = subprocess.run(
            [sys.executable, str(ROOT / "crates/mcp/scripts/mutation_check.py"), "--check"],
            capture_output=True, check=True, timeout=60, cwd=ROOT,
        )
        self.assertEqual(result.stdout, f"{len(entries)} entries apply\n".encode())
        sources = "".join(
            path.read_text(encoding="utf-8")
            for crate in (
                "crates/mcp", "crates/cli", "crates/setup", "crates/store", "crates/ipc",
                "crates/daemon", "crates/transfers",
            )
            for path in (ROOT / crate).rglob("*.rs")
        )
        for entry in entries:
            for name in entry["expect_failing"]:
                self.assertIn(f"fn {name}()", sources, entry["id"])


class _Sandbox(unittest.TestCase):
    def setUp(self):
        from tests.test_rust_shell_parity import _environment

        directory = tempfile.TemporaryDirectory(prefix="kassiber-r6-mcp-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.home = self.base / "home"
        self.home.mkdir()
        # AF_UNIX paths stop at 108 bytes on Linux (104 on macOS), and a test
        # TMPDIR can leave too little room, so the operator socket gets a
        # short private directory under /tmp (mkdtemp makes it 0o700).
        runtime = tempfile.TemporaryDirectory(prefix="kb-mcp-", dir="/tmp")
        self.addCleanup(runtime.cleanup)
        self.runtime = Path(runtime.name)
        self.env = _environment(self.home)
        self.env.update(
            {
                "PYTHONPATH": guarded_pythonpath(),
                "KASSIBER_STATE_ROOT": str(self.base / "state"),
                "KASSIBER_RUST_TEST_OWNER_ROOT": str(self.base / "owners"),
                # Never the account's broker endpoint.
                "KASSIBER_OPERATOR_RUNTIME_DIR": str(self.runtime),
                "KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR": "1",
                # Strict: any gated connection, loopback included, trips.
                "KASSIBER_TEST_NO_EGRESS": "strict",
            }
        )
        (self.base / "owners").mkdir()
        self.access = Path(self.env["KASSIBER_AGENT_ACCESS_FILE"])
        self.allow(True)

    def allow(self, mcp: bool, ai: bool | None = None, grant=None, agent_access=None):
        from kassiber.agent_access import set_agent_access

        set_agent_access(
            mcp_enabled=mcp,
            ai_features_enabled=ai,
            grant=grant,
            agent_access=agent_access,
            path=self.access,
        )

    def seed(self, fixture: str) -> Path:
        from kassiber.db import open_db

        root = self.base / fixture / "seed"
        with closing(open_db(root)) as conn, conn:
            if FIXTURES[fixture] is not None:
                conn.executescript(FIXTURES[fixture].read_text())
        return root

    def copies(self, seed: Path, name: str) -> tuple[Path, Path]:
        # Equal path lengths keep anything path-shaped comparable.
        python_root = self.base / name / "py"
        rust_root = self.base / name / "rs"
        shutil.copytree(seed, python_root)
        shutil.copytree(seed, rust_root)
        return python_root, rust_root

    def run_side(self, prefix, argv, *, data=b"", env=None):
        return subprocess.run(
            [*prefix, *argv],
            input=data,
            capture_output=True,
            timeout=900,
            env=env or self.env,
            cwd=ROOT,
        )

    @property
    def python(self):
        return list(cli_runner.prefix(reference=True))

    @property
    def rust(self):
        return [str(Path(RUST_BIN).resolve())]


@unittest.skipUnless(RUST_BIN and os.name == "posix", "set KASSIBER_RUST_BIN to a test-harness build")
class McpTranscriptParityTest(_Sandbox):
    maxDiff = None

    def serve(self, prefix, root, messages, *extra):
        """One session; input ends only once every request was answered, so
        the 30 s drain at end of input never decides how many replies come.
        Requests go in windows answered before the next is sent: the server
        refuses calls beyond 32 in flight, and every call must run."""
        process = subprocess.Popen(
            [*prefix, "--data-root", str(root), "mcp", "serve", *extra],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            cwd=ROOT,
        )
        lines = []
        stderr = []
        drain = threading.Thread(target=lambda: stderr.append(process.stderr.read()), daemon=True)
        drain.start()
        try:
            expected = 0
            for start in range(0, len(messages), 16):
                window = messages[start:start + 16]
                expected += sum(1 for message in window if "method" in message and "id" in message)
                process.stdin.write(_lines(window))
                process.stdin.flush()
                while len(lines) < expected:
                    line = process.stdout.readline()
                    if not line:
                        break
                    lines.append(line)
            process.stdin.close()
            lines += process.stdout.readlines()
            self.assertEqual(process.wait(timeout=900), 0, lines[-3:])
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdout.close()
        drain.join(60)
        process.stderr.close()
        self.assertEqual(stderr, [b""])
        return lines

    def compare(self, python_root, rust_root, messages, *extra, refused=None):
        python_mask, rust_mask = _Masker(), _Masker()
        expected = self.serve(self.python, python_root, messages, *extra)
        actual = self.serve(self.rust, rust_root, messages, *extra)
        self.assertEqual(len(actual), len(expected), (actual[-3:], expected[-3:]))
        by_id = {json.dumps(message.get("id")): message for message in messages}
        compared, mismatches = 0, []
        for have, want in zip(actual, expected):
            record = json.loads(have)
            request = by_id.get(json.dumps(record.get("id")))
            structured = (record.get("result") or {}).get("structuredContent") or {}
            code = (structured.get("error") or {}).get("code")
            if code == "not_implemented" and refused is not None:
                refused.add(request["params"]["name"])
                continue
            params = (request or {}).get("params")
            if isinstance(params, dict) and params.get("name") in PENDING:
                continue
            if (isinstance(params, dict) and params.get("name") in PYTHON_CRASHES
                    and PYTHON_CRASH_TEXT in want):
                self.assertNotIn(PYTHON_CRASH_TEXT, have, request)
                continue
            compared += 1
            have, want = rust_mask(have), python_mask(want)
            if have != want:
                mismatches.append((request, have.decode()[:2000], want.decode()[:2000]))
        self.assertEqual(mismatches[:3], [], f"{len(mismatches)} of {compared} replies differ")
        return compared, actual

    def head(self):
        """Every request that is not a tool call, in both eras."""
        return [
            _request("init", "initialize", {"protocolVersion": "2025-06-18", "capabilities": {}}),
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            _request("legacy-list", "tools/list"),
            _request(1, "server/discover", {"_meta": MODERN_META}),
            _request(2, "tools/list", {"_meta": MODERN_META}),
            _request(3, "tools/list", {"_meta": MODERN_META, "cursor": "next"}),
            _request(4, "ping"),
            _request(5, "resources/list", {"_meta": MODERN_META}),
            _request(6, "tools/list", {"_meta": {**MODERN_META, "io.modelcontextprotocol/protocolVersion": "1900-01-01"}}),
            {"jsonrpc": "2.0", "id": 7, "method": "ping", "params": [1]},
            {"jsonrpc": "1.0", "id": 8, "method": "ping"},
            {"jsonrpc": "2.0", "id": None, "method": "ping"},
            {"jsonrpc": "2.0", "id": 9, "result": {}},
            _request(10, "subscriptions/listen", {"_meta": MODERN_META, "notifications": {}}),
        ]

    def calls(self, tools=None):
        """Every MCP tool and its argument cases, then protocol refusals."""
        messages = []
        for index, (_source, name, arguments) in enumerate(_mcp_calls(tools)):
            messages.append(_call(f"c{index}", name, arguments))
        messages += [
            # The legacy era carries structured content too.
            _call("legacy-status", "status", {}, modern=False),
            _call("legacy-rates", "rates_summary", {}, modern=False),
            # `1` and `"1"` are different requests.
            _call(1001, "wallets_list", {}),
            _call("1001", "wallets_list", {}),
            _call("unknown", "shell", {}),
            _call("catalog-name", "ui.transactions.list", {}),
            _call("mutating", "transfers_pair", {}),
            _call("arguments-list", "status", [1]),
            _call("invalid", "transactions_list", {"bogus": 1}),
            _call("bounds", "transactions_list", {"limit": 0}),
            _request("no-name", "tools/call", {"_meta": MODERN_META}),
        ]
        return messages

    def test_transcripts_match_python_on_every_fixture(self):
        """The default grant (read, restricted): every tool is called, so
        the refusals of the ones it does not cover are compared too."""
        from kassiber.ai.tools import get_tool
        from kassiber.mcp import tools as mcp_tools

        expected_refused = {mcp_tools.mcp_tool_name(get_tool(name)) for name in _refused()}
        listing = mcp_tools.available_tools(grant="read", agent_access="restricted")
        for fixture in FIXTURES:
            with self.subTest(fixture=fixture):
                python_root, rust_root = self.copies(self.seed(fixture), fixture)
                refused = set()
                compared, actual = self.compare(
                    python_root, rust_root, self.head() + self.calls(), refused=refused
                )
                self.assertGreater(compared, 80)
                # Every tool's service is native: nothing answers
                # not_implemented, and every reply is compared.
                self.assertLessEqual(refused, expected_refused)
                # The Rust server answered: its tool list is the grant's.
                listed = next(json.loads(line) for line in actual if b'"id":2,' in line)
                self.assertEqual(len(listed["result"]["tools"]), len(listing))

    def test_every_grant_lists_and_runs_what_it_covers(self):
        """Each grant and access mode on its own copy of the desktop book:
        the head (listing, instructions) and every covered tool, writes
        included, so the books change the same way on both sides."""
        from kassiber.ai.tools import get_tool
        from kassiber.mcp import tools as mcp_tools

        expected_refused = {mcp_tools.mcp_tool_name(get_tool(name)) for name in _refused()}
        seed = self.seed("desktop")
        for grant in GRANTS:
            for access in ACCESS:
                with self.subTest(grant=grant, access=access):
                    self.allow(True, grant=grant, agent_access=access)
                    tools = mcp_tools.available_tools(grant=grant, agent_access=access)
                    python_root, rust_root = self.copies(seed, f"{grant}-{access}")
                    refused = set()
                    # Pinned: an unpinned server refuses every write.
                    compared, actual = self.compare(
                        python_root,
                        rust_root,
                        self.head() + self.calls(tools),
                        "--workspace",
                        "w",
                        "--profile",
                        "p",
                        refused=refused,
                    )
                    self.assertLessEqual(refused, expected_refused)
                    self.assertNotIn(b"book_pin_required", b"".join(actual))

    def test_pinned_books_never_touch_the_shared_context(self):
        from kassiber.db import get_setting, open_db

        python_root, rust_root = self.copies(self.seed("desktop"), "pinned")
        messages = [
            _call("status", "status", {}),
            _call("wallets", "wallets_list", {}),
            _call("transactions", "transactions_list", {"limit": 2}),
        ]
        self.compare(python_root, rust_root, messages, "--workspace", "w", "--profile", "foreign")
        self.compare(python_root, rust_root, messages, "--workspace", "w", "--profile", "Other")
        for root in (python_root, rust_root):
            with closing(open_db(root)) as conn:
                self.assertEqual(get_setting(conn, "context_profile"), "p")
                self.assertEqual(conn.execute("PRAGMA query_only").fetchone()[0], 0)
        # An unknown book is a tool error, the same on both sides.
        self.compare(python_root, rust_root, [_call("status", "status", {})], "--workspace", "w", "--profile", "nope")
        # A half pin never starts a server.
        for prefix, root in ((self.python, python_root), (self.rust, rust_root)):
            result = self.run_side(prefix, ["--data-root", str(root), "mcp", "serve", "--profile", "p"])
            self.assertEqual(result.returncode, 1)
        python_half = self.run_side(self.python, ["--machine", "--data-root", str(python_root), "mcp", "serve", "--profile", "p"])
        rust_half = self.run_side(self.rust, ["--machine", "--data-root", str(rust_root), "mcp", "serve", "--profile", "p"])
        self.assertEqual(rust_half.stdout, python_half.stdout)

    def test_refusals_before_any_book_match_python(self):
        missing_python = self.base / "missing" / "py"
        missing_rust = self.base / "missing" / "rs"
        messages = [_call("status", "status", {})]
        self.compare(missing_python, missing_rust, messages)
        self.assertFalse(missing_python.exists() or missing_rust.exists())
        python_root, rust_root = self.copies(self.seed("empty"), "gates")
        self.allow(False)
        self.compare(python_root, rust_root, messages)
        self.allow(True, False)
        self.compare(python_root, rust_root, messages)
        self.access.unlink()
        self.compare(python_root, rust_root, messages)

    def test_manual_encrypted_books_ask_for_the_users_unlock(self):
        from kassiber.core import accounts as core_accounts
        from kassiber.db import open_db, resolve_database_path
        from kassiber.secrets.migration import create_empty_encrypted_database
        from kassiber.secrets.sqlcipher import sqlcipher_available

        if not sqlcipher_available():
            self.skipTest("SQLCipher is required")
        seed = self.base / "encrypted" / "seed"
        seed.mkdir(parents=True)
        create_empty_encrypted_database(resolve_database_path(seed), "tracer-pass-12345")
        with closing(open_db(seed, passphrase="tracer-pass-12345")) as conn:
            workspace = core_accounts.create_workspace(conn, "Personal")
            core_accounts.create_profile(conn, workspace["id"], "Main", "EUR", "FIFO", "generic", 365)
            conn.commit()
        python_root, rust_root = self.copies(seed, "encrypted")
        self.compare(python_root, rust_root, [_call("status", "status", {}), _call("w", "wallets_list", {})])

    def test_one_shot_leaves_match_python(self):
        python_root, rust_root = self.copies(self.seed("desktop"), "oneshot")
        cases = [
            ["--machine", "mcp", "tools"],
            ["mcp", "tools"],
            ["--machine", "mcp", "status"],
            ["--machine", "mcp", "enable"],
            ["--machine", "mcp", "disable"],
            ["--machine", "mcp", "status"],
        ]
        for argv in cases:
            with self.subTest(argv=argv):
                self.allow(False)
                python = self.run_side(self.python, argv)
                self.allow(False)
                rust = self.run_side(self.rust, argv)
                self.assertEqual((rust.returncode, rust.stdout), (python.returncode, python.stdout))
        self.allow(True)
        calls = [
            ["mcp", "call", "--tool", "status"],
            ["mcp", "call", "--tool", "wallets_list", "--workspace", "w", "--profile", "foreign"],
            ["mcp", "call", "--tool", "transactions_list", "--arguments", '{"limit": 2}'],
            ["mcp", "call", "--tool", "transactions_list", "--arguments", '{"limit": Infinity}'],
            ["mcp", "call", "--tool", "transactions_list", "--arguments", "[1]"],
            ["mcp", "call", "--tool", "transactions_list", "--arguments", ""],
            ["mcp", "call", "--tool", "transactions_list", "--arguments", '{"bogus": 1}'],
            ["mcp", "call", "--tool", "shell"],
            ["mcp", "call", "--tool", "status", "--profile", "p"],
            ["mcp", "call", "--tool", "read_skill_reference", "--arguments", '{"name": "nope"}'],
        ]
        for argv in calls:
            if argv[3] in PENDING:
                continue
            for machine in (["--machine"], []):
                with self.subTest(argv=argv, machine=machine):
                    python = self.run_side(self.python, [*machine, "--data-root", str(python_root), *argv])
                    rust = self.run_side(self.rust, [*machine, "--data-root", str(rust_root), *argv])
                    self.assertEqual(
                        (rust.returncode, _GENERATED_AT.sub(rb"\1<t>", rust.stdout)),
                        (python.returncode, _GENERATED_AT.sub(rb"\1<t>", python.stdout)),
                    )
        missing = self.base / "never" / "data"
        for prefix in (self.python, self.rust):
            result = self.run_side(prefix, ["--machine", "--data-root", str(missing), "mcp", "call", "--tool", "status"])
            self.assertEqual(json.loads(result.stdout)["error"]["code"], "not_initialized")
        self.assertFalse(missing.exists())


@unittest.skipUnless(
    RUST_BIN and sys.platform.startswith("linux"), "the fences read /proc; set KASSIBER_RUST_BIN"
)
class McpPrivacyFenceTest(_Sandbox):
    def listener(self):
        """A live operator endpoint that counts every connection."""
        path = self.runtime / "operator-v1.sock"
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(str(path))
        server.listen(8)
        server.settimeout(0.2)
        accepted = []
        stop = threading.Event()

        def accept():
            while not stop.is_set():
                try:
                    connection, _ = server.accept()
                except (socket.timeout, OSError):
                    continue
                accepted.append(connection)
                connection.close()

        thread = threading.Thread(target=accept, daemon=True)
        thread.start()

        def close():
            stop.set()
            thread.join(5)
            server.close()

        self.addCleanup(close)
        return accepted

    def open_descriptors(self, pid: int) -> list[str]:
        fds = Path(f"/proc/{pid}/fd")
        return sorted(os.readlink(fd) for fd in fds.iterdir())

    def converse(self, process, messages):
        replies = []
        for message in messages:
            process.stdin.write((json.dumps(message) + "\n").encode())
            process.stdin.flush()
            if "id" in message:
                replies.append(json.loads(process.stdout.readline()))
        return replies

    def tree(self) -> list[Path]:
        return sorted(path for path in self.home.rglob("*"))

    def test_launch_and_listing_touch_no_file_and_no_socket(self):
        accepted = self.listener()
        before = self.tree()
        data_root = self.base / "not-yet" / "data"
        process = subprocess.Popen(
            [*self.rust, "--data-root", str(data_root), "mcp", "serve"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            cwd=self.base,
        )
        try:
            replies = self.converse(
                process,
                [
                    _request(1, "initialize", {"protocolVersion": "2025-11-25", "capabilities": {}}),
                    {"jsonrpc": "2.0", "method": "notifications/initialized"},
                    _request(2, "tools/list"),
                    _request(3, "server/discover", {"_meta": MODERN_META}),
                    _request(4, "tools/list", {"_meta": MODERN_META}),
                ],
            )
            self.assertEqual([reply["id"] for reply in replies], [1, 2, 3, 4])
            # Standard streams point at /dev/null; the protocol runs on the
            # two private pipe copies; nothing else is open.
            descriptors = self.open_descriptors(process.pid)
            self.assertEqual(
                [d for d in descriptors if not d.startswith("pipe:")],
                ["/dev/null"] * 3,
                descriptors,
            )
            self.assertEqual(len([d for d in descriptors if d.startswith("pipe:")]), 2, descriptors)
        finally:
            process.stdin.close()
            stdout = process.stdout.read()
            stderr = process.stderr.read()
            self.assertEqual(process.wait(timeout=60), 0)
            process.stdout.close()
            process.stderr.close()
        self.assertEqual((stdout, stderr), (b"", b""))
        self.assertEqual(accepted, [])
        self.assertEqual(self.tree(), before)
        self.assertFalse((self.base / "not-yet").exists())
        self.assertFalse((self.base / "state").exists())

    def test_a_local_read_reaches_no_socket(self):
        from kassiber.ai.tools import SKILL_REFERENCE_NAMES

        accepted = self.listener()
        seed = self.seed("desktop")
        process = subprocess.Popen(
            [*self.rust, "--data-root", str(seed), "mcp", "serve"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            cwd=self.base,
        )
        try:
            replies = self.converse(
                process,
                [
                    _call(1, "wallets_list", {}),
                    _call(2, "transactions_list", {"limit": 2}),
                    _call(3, "read_skill_reference", {"name": SKILL_REFERENCE_NAMES[0]}),
                ],
            )
            self.assertTrue(all("isError" not in reply["result"] for reply in replies), replies)
            # Each call closed its book: nothing but the protocol stays open.
            descriptors = self.open_descriptors(process.pid)
            self.assertEqual([d for d in descriptors if not d.startswith("pipe:")], ["/dev/null"] * 3)
        finally:
            process.stdin.close()
            process.stdout.read()
            self.assertEqual(process.stderr.read(), b"")
            self.assertEqual(process.wait(timeout=60), 0)
            process.stdout.close()
            process.stderr.close()
        self.assertEqual(accepted, [])

    def test_the_server_never_starts_a_broker(self):
        from kassiber.core import accounts as core_accounts
        from kassiber.db import database_instance_id, open_db, resolve_database_path
        from kassiber.secrets.migration import create_empty_encrypted_database
        from kassiber.secrets.sqlcipher import sqlcipher_available

        if not sqlcipher_available():
            self.skipTest("SQLCipher is required")
        root = self.base / "brokered"
        root.mkdir()
        create_empty_encrypted_database(resolve_database_path(root), "tracer-pass-12345")
        with closing(open_db(root, passphrase="tracer-pass-12345")) as conn:
            workspace = core_accounts.create_workspace(conn, "Personal")
            core_accounts.create_profile(conn, workspace["id"], "Main", "EUR", "FIFO", "generic", 365)
            conn.commit()
            identity = database_instance_id(conn)
        # The children's state root decides the mode: set it in theirs.
        subprocess.run(
            [
                sys.executable,
                "-c",
                "import sys\n"
                "from kassiber.core.runtime import ensure_runtime_layout, resolve_runtime_paths\n"
                "from kassiber.operator.modes import effective_unlock_mode, set_unlock_mode\n"
                "ensure_runtime_layout(resolve_runtime_paths(sys.argv[1], None, None))\n"
                "set_unlock_mode(sys.argv[1], 'brokered', database_identity=sys.argv[2])\n"
                "assert effective_unlock_mode(sys.argv[1]) == 'brokered'\n",
                str(root),
                identity,
            ],
            env=self.env,
            cwd=ROOT,
            check=True,
            timeout=120,
        )
        messages = [_call(1, "status", {}), _call(2, "wallets_list", {})]
        result = self.run_side(self.rust, ["--data-root", str(root), "mcp", "serve"], data=_lines(messages))
        self.assertEqual((result.returncode, result.stderr), (0, b""))
        for line in result.stdout.splitlines():
            error = json.loads(line)["result"]["structuredContent"]["error"]
            self.assertEqual((error.get("details") or {}).get("reason"), "operator_lease_required", error)
        # A launched broker would have bound its endpoint here.
        time.sleep(1.0)
        self.assertEqual(sorted(self.runtime.iterdir()), [])
        # Python answers the same, also without a broker.
        python = self.run_side(self.python, ["--data-root", str(root), "mcp", "serve"], data=_lines(messages))
        self.assertEqual(result.stdout, python.stdout)
        self.assertEqual(sorted(self.runtime.iterdir()), [])

    def test_only_the_granted_tools_are_reachable(self):
        from kassiber.ai.tools import TOOL_CATALOG
        from kassiber.mcp import tools as mcp_tools

        seed = self.seed("desktop")
        exposed = {tool.name for tool in mcp_tools.mcp_tools()}
        listed = {
            tool.name for tool in mcp_tools.available_tools(grant="read", agent_access="restricted")
        }
        others = []
        for entry in TOOL_CATALOG:
            for name in (entry.name, entry.provider_name, entry.provider_name.replace(".", "_").removeprefix("ui_")):
                if name not in exposed:
                    others.append(name)
        messages = [_request(0, "tools/list", {"_meta": MODERN_META})]
        messages += [_call(index + 1, name, {}) for index, name in enumerate(sorted(set(others)))]
        result = self.run_side(self.rust, ["--data-root", str(seed), "mcp", "serve"], data=_lines(messages))
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual({tool["name"] for tool in replies[0]["result"]["tools"]}, listed)
        for reply in replies[1:]:
            self.assertEqual(reply["error"]["code"], -32602, reply)
        # A tool the grant does not cover is refused before any book opens,
        # in sessions below the server's 32 calls in flight.
        names = sorted(exposed - listed)
        self.assertTrue(names)
        for start in range(0, len(names), 16):
            messages = [_call(index, name, {}) for index, name in enumerate(names[start:start + 16])]
            result = self.run_side(self.rust, ["--data-root", str(seed), "mcp", "serve"], data=_lines(messages))
            lines = result.stdout.splitlines()
            self.assertEqual(len(lines), len(messages))
            for line in lines:
                error = json.loads(line)["result"]["structuredContent"]["error"]
                self.assertIn(error["code"], {"mcp_grant_insufficient", "agent_access_restricted"}, error)


if __name__ == "__main__":
    unittest.main()
