"""The Rust AI tool executor (crates/ai, R6-5a) against Python.

Two layers:

- drift: the embedded catalog and the pure-function vectors the Rust tests
  replay are regenerated from the live Python registries and must match the
  checked-in files byte for byte;
- executor parity: each read tool runs through Python's
  ``_execute_read_only_ai_tool`` (``tests/rust_ai_tool_reference.py``, the
  reference wrapper) and through the Rust daemon's test-harness kind
  ``test.ai_tool`` on separate copies of the same fixture book, and the
  model-visible bytes (sorted compact JSON) and the ``ai.chat.tool_result``
  envelope must be identical.
"""
from __future__ import annotations

from contextlib import closing
import copy
from datetime import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
REFERENCE = ROOT / "tests" / "rust_ai_tool_reference.py"
CLOCK = "2026-03-04T05:06:07Z"
FIXTURES = {
    "empty": None,
    "desktop": ROOT / "crates/transactions/tests/data/desktop.sql",
    "reports": ROOT / "crates/reports/tests/data/book.sql",
}


def _load(relative: str):
    path = ROOT / relative
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AiCatalogDriftTest(unittest.TestCase):
    def test_embedded_catalog_matches_python(self):
        generator = _load("crates/ai/scripts/gen_catalog.py")
        self.assertEqual(
            generator.TARGET.read_text(encoding="utf-8"),
            generator.render(),
            "rerun crates/ai/scripts/gen_catalog.py and review crates/ai's constants",
        )

    def test_pure_function_vectors_match_python(self):
        generator = _load("crates/ai/scripts/gen_vectors.py")
        self.assertEqual(
            generator.TARGET.read_text(encoding="utf-8"),
            generator.render(),
            "rerun crates/ai/scripts/gen_vectors.py and the kassiber-ai tests",
        )

    def test_mutation_catalog_stays_applicable(self):
        catalog = ROOT / "crates/ai/scripts/mutations.json"
        entries = json.loads(catalog.read_text(encoding="utf-8"))
        result = subprocess.run(
            [sys.executable, str(ROOT / "crates/ai/scripts/mutation_check.py"), "--check"],
            capture_output=True, check=True, timeout=60, cwd=ROOT,
        )
        self.assertEqual(result.stdout, f"{len(entries)} entries apply\n".encode())
        # Entries name unit tests in the crates the checker runs (kassiber-ai,
        # kassiber-net and the transfers audit) as well as their integration tests.
        sources = "".join(
            path.read_text(encoding="utf-8")
            for path in [
                *(path for crate in ("crates/ai", "crates/net") for path in (ROOT / crate).rglob("*.rs")),
                ROOT / "crates/transfers/src/custody_ai_audit.rs",
            ]
        )
        oracles = Path(__file__).read_text(encoding="utf-8")
        for entry in entries:
            for name in entry["expect_failing"]:
                if entry["id"].startswith("wire-"):
                    self.assertIn(f"    def {name.rsplit('::', 1)[1]}(self", oracles, entry["id"])
                else:
                    self.assertIn(f"fn {name}()", sources, entry["id"])


_GENERATED_AT = re.compile(rb'(\\?"generated_at\\?":\\?")\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ')


_REFERENCE = re.compile(rb"ca-ref:[0-9a-f]{64}")


def _mask(line: bytes, references: dict[bytes, bytes] | None = None) -> bytes:
    """Declared mappings, not field masks.

    Read-time clocks (``generated_at``) are not compared. Chain-analysis
    ``ca-ref:`` handles are keyed by a per-process secret and the book's
    path, so each side's handles are renamed by first appearance in its
    session: the same subject must keep one name, and distinct subjects
    distinct names.
    """
    line = _GENERATED_AT.sub(rb"\1<generated_at>", line)
    if references is not None:
        line = _REFERENCE.sub(
            lambda match: references.setdefault(match.group(0), b"ca-ref:#%d" % len(references)), line
        )
    return line


REMOTE = {"provider_kind": "remote", "provider_on_device": False}
LOCAL = {"provider_kind": "local", "provider_on_device": True}


def _extra_arguments() -> dict[str, list[dict]]:
    from kassiber.ai.tools import SKILL_REFERENCE_NAMES

    return {
        "ui.transactions.list": [{"limit": 2}, {"limit": 20, "sort": "occurred-at", "order": "desc"}],
        "ui.transactions.extremes": [{"limit": 3}],
        "ui.transactions.search": [{"query": "unique-description-query", "limit": 25}, {"query": "merchant"}],
        "ui.transactions.resolve": [{"query": "a"}, {"query": "external-e"}],
        "ui.transactions.history": [{"transaction": "a"}, {"transaction": "missing"}],
        "ui.activity.history": [{"transaction": "a"}, {"wallet": "cold"}],
        "ui.transactions.commercial_context": [{"transaction": "a"}],
        "ui.attachments.list": [{"transaction": "a"}, {"limit": 1}],
        "ui.wallets.utxos": [{"wallet": "cold"}, {"wallet": "missing"}],
        "ui.reports.tax_summary": [{"year": 2025}],
        "ui.reports.balance_history": [{"interval": "month", "limit": 120}, {"interval": "day", "limit": 5}],
        "ui.reports.exit_tax_preview": [{"departure_date": "2025-06-30", "destination": "DE"}],
        "ui.rates.coverage": [{"limit": 25}],
        "ui.loans.list": [{"limit": 1}],
        "ui.transfers.review_context": [{"limit": 8}],
        "ui.custody.gaps.list": [{"limit": 20}],
        "ui.custody.lineage.snapshot": [{"limit": 100}],
        "ui.source_funds.preview": [{"target_transaction": "a"}],
        "ui.source_funds.review_context": [{"target_transaction": "a"}],
        "read_skill_reference": [{"name": name} for name in SKILL_REFERENCE_NAMES],
        # A handle this process never issued: stale, before any index read.
        "ui.chain_analysis.query": [{"mode": "trace", "subject": "ca-ref:" + "0" * 64}],
        "ui.chain_analysis.labels.list": [{}],
        "ui.workspace.overview.snapshot": [{"workspace_id": "w"}, {"workspace_id": " ws "}, {"workspace_id": "  "}],
        "ui.journals.events.list": [{"transaction": "a"}, {"limit": 2}],
        "ui.journals.quarantine": [{"limit": 1}],
        "ui.journals.transfers.list": [{"limit": 1}],
        "ui.transactions.graph": [{"transaction": "a"}, {"transaction": " a "}, {"transaction": "missing"}],
        "ui.transactions.review_context": [
            {"transaction": "a"},
            {"transaction": "a", "history_limit": 1, "include_graph": False, "include_privacy": False, "include_evidence": False},
            {"transaction": "missing"},
        ],
        "ui.review.worklist": [
            {"limit": 1},
            {"categories": ["commercial", "source_funds"]},
            {"categories": ["readiness", "quarantine", "stale_edits", "transfers", "loans", "commercial", "source_funds"], "limit": 2},
        ],
        "ui.review.cases": [{"limit": 1}],
        "ui.review.request_input": [
            {"action": "attach_evidence", "case_ids": ["quarantine:a"], "expected_input_version": 0, "explanation": "See /home/user/statement.pdf"},
        ],
        "ui.review.plan": [
            {"operations": [{"type": "exclude", "transaction_id": "a", "reason": "spam dust"}], "expected_input_version": 0},
            {"operations": [{"type": "price_override", "transaction_id": "a", "fiat_rate": "1", "reason": "see https://example.com/x"}], "expected_input_version": 0},
        ],
        "ui.audit.evidence.summary": [{"transaction": "a"}, {"transactions": ["a", "b"]}, {"transaction": "a", "source_funds_case": "c"}],
        "ui.wallets.identify": [
            {"addresses": ["bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", "not-an-address"]},
            {"txids": ["0" * 64], "text": "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"},
        ],
        "ui.wallets.analyze_file": [{"sample_rows": 1}, {"column_map": {"date": "Date", "amount": "Amount"}}, {"column_map": {"amount_header_asset": "BTC"}}],
        "ui.egress.snapshot": [{"after_id": 0, "limit": 5}, {"after_id": 3}],
        "ui.reports.lightning_profitability": [{"connection": "missing"}, {"connection": "cold", "window_days": 7}],
        "ui.connections.node.snapshot": [{"connection": "missing"}],
    }


_GENERATOR = []


def _fill(schema):
    if not _GENERATOR:
        _GENERATOR.append(_load("crates/ai/scripts/gen_vectors.py"))
    return _GENERATOR[0].fill(schema)


ATTACHMENT_CSV = "Date,Type,Amount,Fee\n2025-01-02,Buy,0.01,0\n2025-01-03,Sell,0.005,0\n"


def tool_requests(attachment=None):
    """Every read tool, by argument case and executor state.

    ``attachment`` names the chat's staged file for the file-analysis tool.
    Egressing reads carry a consent bound to their call, as the chat loop
    records it, so their native routes run on both sides.
    """
    from kassiber.ai.tools import TOOL_CATALOG
    from kassiber.mcp.tools import MCP_SOURCE_TOOL_NAMES

    extra = _extra_arguments()
    mcp_state = {
        **REMOTE,
        "advertised_tools": [entry.provider_name for entry in TOOL_CATALOG if entry.name in MCP_SOURCE_TOOL_NAMES],
        "cross_book_read_allowed": False,
        "read_maintenance": "disabled",
        "auto_sync_attempted": True,
        "scope_workspace_id": "w",
        "scope_profile_id": "p",
    }
    requests = []
    for tool in TOOL_CATALOG:
        if tool.kind_class != "read_only":
            continue
        cases = [{}]
        filled = _fill(tool.parameters)
        if filled and filled not in cases:
            cases.append(filled)
        cases += [case for case in extra.get(tool.name, []) if case not in cases]
        states = [REMOTE]
        if tool.name.startswith("ui.custody."):
            states.append(LOCAL)
        if tool.egresses:
            states = [{**state, "consented_call_ids": ["call_x"]} for state in [REMOTE, LOCAL]]
        if tool.name == "ui.wallets.analyze_file" and attachment is not None:
            staged = {"attachment_source_file": str(attachment), "attachment_label": "Exchange export"}
            states = [REMOTE, {**REMOTE, **staged}, {**LOCAL, **staged}]
        if tool.name == "ui.workspace.overview.snapshot":
            states = [
                REMOTE,
                {**REMOTE, "cross_book_read_allowed": False},
                {**REMOTE, "cross_book_read_allowed": True, "scope_workspace_id": "w", "scope_profile_id": "p"},
                {**REMOTE, "cross_book_read_allowed": True, "scope_workspace_id": "ws", "scope_profile_id": "p"},
            ]
        if tool.name == "ui.profiles.snapshot":
            states.append({**REMOTE, "cross_book_read_allowed": True, "scope_workspace_id": "w", "scope_profile_id": "p"})
        if tool.name in MCP_SOURCE_TOOL_NAMES:
            states.append(mcp_state)
        for arguments in cases:
            for state in states:
                requests.append({"kind": "test.ai_tool", "args": {"name": tool.provider_name, "arguments": arguments, "state": copy.deepcopy(state), "call_id": "call_x"}})
    for state in ({**REMOTE, "advertised_tools": ["status"]}, {**REMOTE, "scope_workspace_id": "w", "scope_profile_id": "other"}):
        for name in ("ui_rates_summary", "status"):
            requests.append({"kind": "test.ai_tool", "args": {"name": name, "arguments": {}, "state": state}})
    for raw in ({"id": "c1", "function": {"name": "status", "arguments": "{bad"}}, {"id": "c2", "function": {"name": "ui_rates_summary", "arguments": "{}"}}, {"function": {"name": "nope"}}):
        requests.append({"kind": "test.ai_tool", "args": {"raw_call": raw, "state": REMOTE}})
    requests.append({"kind": "test.ai_tool", "args": {"name": "status", "arguments": {}, "state": REMOTE, "runtime_data_root": "/nonexistent/other-project"}})
    return requests


AUTO_READ_QUESTIONS = (
    "What is my balance?",
    "Show my recent transactions",
    "find transactions for \"merchant\"",
    "What is the tax summary for 2025?",
    "Any loans or collateral?",
    "How are my wallets and backends connected?",
    "Show the treasury across all books",
    "What needs review next?",
    "Explain my swap and transfer pairs",
    "Is my privacy ok over tor?",
    "Show the balance history monthly",
    "What changed since last answer?",
    "custody gap from whirlpool",
)


def auto_read_requests():
    from kassiber.ai.tools import TOOL_CATALOG

    advertised = [entry.provider_name for entry in TOOL_CATALOG]
    requests = []
    for question in AUTO_READ_QUESTIONS:
        for state in ({**REMOTE, "advertised_tools": advertised}, {**LOCAL, "advertised_tools": advertised[:40]}):
            requests.append({"kind": "test.ai_auto_read", "args": {"messages": [{"role": "user", "content": question}], "system_prompt_kind": "kassiber", "state": copy.deepcopy(state)}})
    requests.append({"kind": "test.ai_auto_read", "args": {"messages": [{"role": "user", "content": "tax"}], "system_prompt_kind": "raw", "state": REMOTE}})
    return requests


@unittest.skipUnless(RUST_BIN, "requires the kassiber test-harness build (KASSIBER_RUST_BIN)")
class AiToolExecutorParityTest(unittest.TestCase):
    """Per tool and per fixture book: identical model-visible bytes and records."""

    maxDiff = None

    def setUp(self):
        from tests.test_rust_shell_parity import _environment

        directory = tempfile.TemporaryDirectory(prefix="kassiber-r6-ai-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.env = _environment(self.base / "home")
        self.env.update(
            {
                "PYTHONPATH": str(ROOT),
                "KASSIBER_STATE_ROOT": str(self.base / "state"),
                "KASSIBER_RUST_TEST_OWNER_ROOT": str(self.base / "owners"),
                "KASSIBER_RUST_TEST_JOURNAL_TIME": CLOCK,
                "KASSIBER_RUST_TEST_JOURNAL_IDS": "sequential",
                # The chain-analysis index draws its per-book instance id (and
                # so every snapshot_id) from one counter on both sides.
                "KASSIBER_RUST_TEST_ANALYSIS_IDS": "sequential",
                "KASSIBER_RUST_TEST_ANALYSIS_TIME": CLOCK,
                # Strict: any gated connection, loopback included, trips the
                # harness instead of reaching a socket.
                "KASSIBER_TEST_NO_EGRESS": "strict",
            }
        )
        (self.base / "home").mkdir()
        (self.base / "owners").mkdir()
        # Clocks a run samples (edit history, audit rows) mask from here on.
        from datetime import datetime, timedelta, timezone

        self.started = datetime.now(timezone.utc) - timedelta(seconds=1)

    def seed(self, fixture: str) -> Path:
        from kassiber.db import open_db

        root = self.base / fixture / "seed"
        if root.exists():
            return root
        # Open-time backfills only see rows that existed when the book
        # opened, so reopen once to settle them before any baseline.
        for script in (FIXTURES[fixture].read_text() if FIXTURES[fixture] else "", ""):
            with closing(open_db(root)) as conn, conn:
                conn.executescript(script)
        return root

    def session(self, argv_prefix, root, requests, *, sandbox=(), env=None):
        wire = [dict(request, request_id=str(index)) for index, request in enumerate(requests)]
        wire.append({"request_id": "stop", "kind": "daemon.shutdown"})
        payload = "".join(json.dumps(request) + "\n" for request in wire).encode()
        result = subprocess.run(
            [*sandbox, *argv_prefix, "--machine", "--data-root", str(root), "daemon"],
            env=env or self.env, input=payload, capture_output=True, timeout=900, cwd=ROOT,
        )
        self.assertEqual((result.returncode, result.stderr), (0, b""), result.stdout[-2000:])
        return result.stdout.splitlines()

    def compare(self, fixture, requests):
        seed = self.seed(fixture)
        python_root = self.base / fixture / "python"
        rust_root = self.base / fixture / "rust"
        shutil.copytree(seed, python_root)
        shutil.copytree(seed, rust_root)
        expected = self.session([sys.executable, str(REFERENCE)], python_root, requests)
        actual = self.session([str(Path(RUST_BIN).resolve())], rust_root, requests)
        self.assertNotEqual(expected[0], actual[0], "the candidate must be the Rust daemon")
        self.assertIn("test.ai_tool", json.loads(actual[0])["data"]["supported_kinds"])
        self.assertEqual(len(actual), len(expected))
        refused, compared, mismatches = set(), 0, []
        rust_references, python_references = {}, {}
        from kassiber.ai.tools import get_tool

        for index, (have, want) in enumerate(zip(actual[1:], expected[1:]), start=1):
            record = json.loads(have)
            request = requests[int(record["request_id"])] if record.get("request_id", "stop") != "stop" else None
            data = record.get("data") or {}
            tool_result = (data.get("tool_result") or {}).get("data") or {}
            if record.get("kind") == "ai.chat.tool_result":
                tool_result = data
            if tool_result.get("reason") == "consent_required":
                # Reviewed difference: Rust runs an egressing read only with a
                # consent bound to the call; Python leaves that to the chat loop.
                name = request["args"].get("name") if request else None
                self.assertTrue(get_tool(name).egresses, name)
                continue
            if tool_result.get("reason") == "not_implemented":
                name = request["args"].get("name") if request else None
                if record.get("kind") == "ai.chat.tool_result":
                    continue
                refused.add(get_tool(name).name)
                continue
            if record.get("kind") == "test.ai_auto_read" and have != want:
                # The planner's reads include refused tools; compare the rest.
                want_data = json.loads(want)["data"]
                self.assertEqual(data["planned"], want_data["planned"], request)
                continue
            compared += 1
            have, want = _mask(have, rust_references), _mask(want, python_references)
            if have != want:
                mismatches.append((request, have.decode()[:3000], want.decode()[:3000]))
        if mismatches and os.environ.get("KASSIBER_AI_PARITY_REPORT"):
            with open(os.environ["KASSIBER_AI_PARITY_REPORT"], "a", encoding="utf-8") as report:
                for request, have, want in mismatches:
                    report.write(json.dumps({"fixture": fixture, "request": request, "rust": have, "python": want}) + "\n")
        self.assertEqual(
            [request for request, _, _ in mismatches][:5], [],
            f"{len(mismatches)} of {compared} records differ",
        )
        return refused, compared, actual

    def test_read_tools_match_python_on_every_fixture(self):
        from kassiber.ai.tools import get_tool

        attachment = self.base / "export.csv"
        attachment.write_text(ATTACHMENT_CSV)
        requests = tool_requests(attachment)
        for fixture in FIXTURES:
            with self.subTest(fixture=fixture):
                refused, compared, _ = self.compare(fixture, requests)
                self.assertGreater(compared, 200)
                for name in refused:
                    self.assertTrue(
                        name.startswith("ui.chain_analysis.") or name in REFUSED_READS,
                        f"{name} refused without being listed",
                    )

    def test_auto_reads_match_python(self):
        for fixture in ("desktop", "reports"):
            with self.subTest(fixture=fixture):
                self.compare(fixture, auto_read_requests())

    def interactive(self, argv_prefix, root, first, follow_up):
        """One request, then a second built from the first one's result."""
        process = subprocess.Popen(
            [*argv_prefix, "--machine", "--data-root", str(root), "daemon"],
            env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT,
        )
        try:
            lines = [process.stdout.readline()]
            process.stdin.write((json.dumps(dict(first, request_id="1")) + "\n").encode())
            process.stdin.flush()
            lines.append(process.stdout.readline())
            for index, request in enumerate(follow_up(json.loads(lines[-1])), start=2):
                process.stdin.write((json.dumps(dict(request, request_id=str(index))) + "\n").encode())
                process.stdin.flush()
                lines.append(process.stdout.readline())
            process.stdin.close()
            self.assertEqual(process.wait(timeout=300), 0)
            self.assertEqual(process.stderr.read(), b"")
        finally:
            if process.poll() is None:
                process.kill()
        return lines

    def test_chain_analysis_handles_round_trip(self):
        """A remote provider's handle reaches the same subject in both."""
        seed = self.seed("desktop")
        remote = {"provider_kind": "remote", "provider_on_device": False}
        first = {"kind": "test.ai_tool", "args": {"name": "ui_chain_analysis_query", "arguments": {}, "state": remote}}

        def follow_up(record):
            output = json.loads(record["data"]["output"])
            nodes = output["envelope"]["data"]["nodes"]
            self.assertTrue(nodes)
            subject = nodes[0]["id"]
            self.assertTrue(subject.startswith("ca-ref:"))
            trace = {"mode": "trace", "subject": subject}
            return [
                {"kind": "test.ai_tool", "args": {"name": "ui_chain_analysis_query", "arguments": trace, "state": remote}},
                # On-device providers may use the same handle: it decodes, and
                # the result carries the real subject.
                {"kind": "test.ai_tool", "args": {"name": "ui_chain_analysis_query", "arguments": trace, "state": {"provider_kind": "local", "provider_on_device": True}}},
            ]

        results = []
        for name, prefix in (("python", [sys.executable, str(REFERENCE)]), ("rust", [str(Path(RUST_BIN).resolve())])):
            root = self.base / name
            shutil.copytree(seed, root)
            references = {}
            results.append([_mask(line, references) for line in self.interactive(prefix, root, first, follow_up)[1:]])
        self.assertEqual(results[1], results[0])
        traced = json.loads(results[1][1])["data"]["tool_result"]["data"]
        self.assertIs(traced["ok"], True, traced)

    def snapshot(self, root):
        conn = sqlite3.connect(root / "kassiber.sqlite3")
        try:
            tables = sorted(row[0] for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"))
            self.assertEqual(len(tables), 112)
            return {table: sorted(conn.execute(f'SELECT * FROM "{table}"').fetchall(), key=repr) for table in tables}
        finally:
            conn.close()

    def compare_writes(self, fixture, requests, seed, unchanged=False):
        """Envelope and model bytes per call, then all 112 tables, on copies."""
        if not requests:
            return []
        base = Path(tempfile.mkdtemp(prefix="writes-", dir=self.base))
        python_root, rust_root = base / "python", base / "rust"
        shutil.copytree(seed, python_root)
        shutil.copytree(seed, rust_root)
        # Exports render with the report clock both runtimes pin (as
        # tests/test_rust_report_exports.py does); reads keep the wall clock.
        env = {**self.env, "KASSIBER_RUST_TEST_REPORT_TIME": CLOCK}
        expected = self.session([sys.executable, str(REFERENCE)], python_root, requests, env=env)
        actual = self.session([str(Path(RUST_BIN).resolve())], rust_root, requests, env=env)
        self.assertEqual(len(actual), len(expected))
        masker = Masker(self.started)
        rust_references, python_references = {}, {}
        mismatches = []
        for request, have, want in zip(requests, actual[1:], expected[1:]):
            have, want = _xlsx_artifact(masker, have, 0), _xlsx_artifact(masker, want, 1)
            have = masker.text(_mask(have, rust_references), side=0)
            want = masker.text(_mask(want, python_references), side=1)
            if have != want:
                mismatches.append((request, have[:4000], want[:4000]))
        if mismatches and os.environ.get("KASSIBER_AI_PARITY_REPORT"):
            with open(os.environ["KASSIBER_AI_PARITY_REPORT"], "a", encoding="utf-8") as report:
                for request, have, want in mismatches:
                    report.write(json.dumps({"fixture": fixture, "request": request, "rust": have, "python": want}) + "\n")
        self.assertEqual([request for request, _, _ in mismatches][:3], [], f"{len(mismatches)} records differ")
        rust_tables = masker.tables(self.snapshot(rust_root), side=0)
        python_tables = masker.tables(self.snapshot(python_root), side=1)
        if rust_tables != python_tables and os.environ.get("KASSIBER_AI_PARITY_REPORT"):
            with open(os.environ["KASSIBER_AI_PARITY_REPORT"], "a", encoding="utf-8") as report:
                for table in sorted(rust_tables):
                    if rust_tables[table] != python_tables[table]:
                        report.write(json.dumps({"fixture": fixture, "table": table, "tool": requests[0]["args"]["name"],
                                                 "rust": repr(rust_tables[table])[:3000],
                                                 "python": repr(python_tables[table])[:3000]}) + "\n")
        self.assertEqual(rust_tables, python_tables, f"{fixture}: tables differ after {requests[0]['args']['name']}")
        if unchanged:
            # The local analysis index is a derived projection a preview may
            # refresh; nothing else may change.
            def authored(snapshot):
                return {table: rows for table, rows in snapshot.items() if not table.startswith("chain_index_")}

            self.assertEqual(authored(self.snapshot(rust_root)), authored(self.snapshot(seed)))
        return [json.loads(line) for line in actual[1:-1]]

    def test_mutating_tools_match_python_on_every_fixture(self):
        """Every mutating tool: whole envelope and all 112 tables after the calls."""
        from concurrent.futures import ThreadPoolExecutor
        from kassiber.ai.tools import TOOL_CATALOG

        tools = [tool for tool in TOOL_CATALOG if tool.kind_class == "mutating"]
        only = os.environ.get("KASSIBER_AI_ONLY_TOOLS")
        if only:
            tools = [tool for tool in tools if tool.name in only.split(",")]
        else:
            # The focused tests below cover these (and name them for the
            # mutation catalog).
            tools = [tool for tool in tools if tool.name not in FOCUSED_WRITES]
        for fixture in FIXTURES:
            seed = self.seed(fixture)
            with self.subTest(fixture=fixture), ThreadPoolExecutor(max_workers=4) as pool:
                jobs = []
                for tool in tools:
                    compared, refused = mutating_requests(fixture, tool)
                    jobs.append((tool, refused, pool.submit(self.compare_writes, fixture, compared, seed)))
                for tool, refused, job in jobs:
                    with self.subTest(fixture=fixture, tool=tool.name):
                        records = job.result()
                        self.assertTrue(records or refused)
                        if refused:
                            self.assert_refused(seed, refused)

    def compare_tools(self, names):
        from kassiber.ai.tools import get_tool

        for fixture in FIXTURES:
            seed = self.seed(fixture)
            for name in names:
                compared, refused = mutating_requests(fixture, get_tool(name))
                with self.subTest(fixture=fixture, tool=name):
                    self.compare_writes(fixture, compared, seed)
                    self.assertEqual(refused, [])

    def test_custody_writes_are_audited_or_rolled_back(self):
        """Consented custody writes commit with their audit row; refused ones
        roll back and commit a failed row; neither runs without the audit
        context, a local provider or the frozen book."""
        self.compare_tools(["ui.custody.review.apply"])

    def test_ai_authored_writes_match_python(self):
        """Writes Python authors as `ai_tool`, or runs without pairing."""
        self.compare_tools(sorted(FOCUSED_WRITES - {"ui.custody.review.apply"}))

    def test_consent_previews_match_python(self):
        """The consent card of every consented tool, recomputed previews
        included, writes nothing and matches Python."""
        from kassiber.ai.tools import TOOL_CATALOG

        for fixture in FIXTURES:
            ids = FIXTURE_IDS[fixture]
            scope = {"scope_workspace_id": ids["ws"], "scope_profile_id": ids["profile"]} if ids["ws"] else {}
            requests = []
            for tool in TOOL_CATALOG:
                if not (tool.kind_class == "mutating" or tool.egresses):
                    continue
                cases = [{}, _fill(tool.parameters)] + [_render(case, ids) for case in _mutating_extra().get(tool.name, [])]
                if tool.name in {"ui.chain_analysis.acquire.apply", "ui.chain_analysis.sources.authorize"}:
                    cases += [{"plan": {"args": {}}}, {"plan": {"spec": {}, "x": 1}}, {"plan": "x"}]
                if tool.name == "ui.review.apply":
                    cases += [{"artifact": {"operations": [{"type": "kind_override"}]}, "idempotency_key": "k"},
                              {"artifact": {"operations": []}, "idempotency_key": ""},
                              {"artifact": "x", "idempotency_key": "k"}]
                for arguments in cases:
                    for state in ({**REMOTE, **scope}, {**LOCAL, **scope}):
                        requests.append({"kind": "test.ai_consent_preview", "args": {
                            "name": tool.provider_name, "arguments": arguments, "call_id": "call_p", "state": state}})
            with self.subTest(fixture=fixture):
                records = self.compare_writes(fixture, requests, self.seed(fixture), unchanged=True)
                self.assertEqual(len(records), len(requests))

    def test_custody_audit_records_match_python(self):
        """Denied, timed-out and cancelled custody proposals are audited in
        the book exactly as Python writes them; other tools are not."""
        for fixture in ("desktop", "reports"):
            ids = FIXTURE_IDS[fixture]
            scope = {"scope_workspace_id": ids["ws"], "scope_profile_id": ids["profile"]}
            arguments = {"action": "create", "gap_id": "gap-" + "x" * 300, "expected_input_version": 3,
                         "reason": "r" * 600, "classification": "external_payment", "extra": [1]}
            requests = []
            for name, decision, status, code, state in (
                ("ui_custody_review_apply", "deny", "denied", "user_denied", scope),
                ("ui_custody_review_apply", "consent_timeout", "denied", "consent_timeout", scope),
                ("ui_custody_review_apply", "cancelled", "cancelled", "cancelled", scope),
                ("ui_custody_review_apply", "cancelled", "cancelled", "cancelled_before_consent", scope),
                ("ui_custody_review_apply", "deny", "denied", "user_denied", {}),
                ("ui_custody_review_apply", "maybe", "denied", "user_denied", scope),
                ("ui_transfers_pair", "deny", "denied", "user_denied", scope),
            ):
                requests.append({"kind": "test.ai_custody_record", "args": {
                    "name": name, "arguments": arguments, "call_id": "call_" + "c" * 300,
                    "state": {**LOCAL, **state},
                    "custody_audit": {**CUSTODY_AUDIT, "consent_decision": decision, "model": "m" * 300},
                    "execution_status": status, "execution_code": code}})
            with self.subTest(fixture=fixture):
                self.compare_writes(fixture, requests, self.seed(fixture))

    def assert_refused(self, seed, requests):
        """Rust-only: a refused write answers not_implemented (or refuses its
        arguments first) and changes nothing."""
        root = Path(tempfile.mkdtemp(prefix="refused-", dir=self.base)) / "rust"
        shutil.copytree(seed, root)
        before = self.snapshot(root)
        lines = self.session([str(Path(RUST_BIN).resolve())], root, requests)
        results = [json.loads(line)["data"]["tool_result"]["data"] for line in lines[1:-1]]
        reasons = [result.get("reason") for result in results]
        # A sync of a book without context answers an empty result.
        self.assertTrue(all(result.get("ok") is False or result.get("envelope", {}).get("data") == {"results": []}
                            for result in results), results)
        self.assertTrue(set(reasons) <= {None, "not_implemented", "validation", "stale_context"}, reasons)
        self.assertEqual(self.snapshot(root), before)

    def chained(self, argv_prefix, root, steps):
        """Requests built one at a time from every earlier record."""
        process = subprocess.Popen(
            [*argv_prefix, "--machine", "--data-root", str(root), "daemon"],
            env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=ROOT,
        )
        try:
            lines = [process.stdout.readline()]
            for index, step in enumerate(steps, start=1):
                request = step([json.loads(line) for line in lines[1:]])
                process.stdin.write((json.dumps(dict(request, request_id=str(index))) + "\n").encode())
                process.stdin.flush()
                lines.append(process.stdout.readline())
            process.stdin.close()
            self.assertEqual(process.wait(timeout=300), 0)
            self.assertEqual(process.stderr.read(), b"")
        finally:
            if process.poll() is None:
                process.kill()
        return lines

    def test_acquisition_effect_receipts_stay_local(self):
        """A kind-override review applied on the desktop: its receipt, and any
        AI plan or consent preview of such an operation, stay local."""
        scope = {"scope_workspace_id": "ws", "scope_profile_id": "p"}
        operation = {"type": "kind_override", "transaction_id": "in", "kind": "buy", "reason": "Bought on an exchange"}

        def data(records, index):
            return records[index]["data"]

        def ai(name, arguments, kind="test.ai_tool"):
            return {"kind": kind, "args": {"name": name, "arguments": arguments, "call_id": "call_k",
                                           "state": {**REMOTE, **scope, "consented_call_ids": ["call_k"]}}}

        steps = [
            lambda records: {"kind": "ui.review.cases", "args": {}},
            lambda records: {"kind": "ui.review.plan", "args": {
                "operations": [operation], "expected_input_version": data(records, 0)["input_version"]}},
            lambda records: {"kind": "ui.review.apply", "args": {"artifact": data(records, 1), "idempotency_key": "desktop-1"}},
            lambda records: ai("ui_review_receipt", {"idempotency_key": "desktop-1"}),
            lambda records: ai("ui_review_plan", {"operations": [operation],
                                                  "expected_input_version": data(records, 0)["input_version"]}),
            lambda records: ai("ui_review_apply", {"artifact": data(records, 1), "idempotency_key": "desktop-1"},
                               kind="test.ai_consent_preview"),
            lambda records: ai("ui_review_apply", {"artifact": data(records, 1), "idempotency_key": "ai-2"}),
        ]
        seed = self.seed("reports")
        masker = Masker(self.started)
        results = []
        for side, prefix in enumerate(([str(Path(RUST_BIN).resolve())], [sys.executable, str(REFERENCE)])):
            root = self.base / f"kind-override-{side}"
            shutil.copytree(seed, root)
            lines = self.chained(prefix, root, steps)
            results.append(([masker.text(line, side=side) for line in lines[1:]],
                            masker.tables(self.snapshot(root), side=side)))
        self.assertEqual(results[0][0], results[1][0])
        self.assertEqual(results[0][1], results[1][1])
        records = [json.loads(line) for line in results[0][0]]
        self.assertEqual(records[2]["kind"], "ui.review.apply", records[2])
        # The receipt stays local; the AI plan and apply schemas already have
        # no field for an acquisition kind, and the consent card refuses it.
        for index, reason in ((3, "interaction_required"), (4, "validation"), (6, "validation")):
            refused = records[index]["data"]["tool_result"]["data"]
            self.assertEqual(refused.get("reason"), reason, refused)
        self.assertEqual(records[5]["data"]["card"]["review_preview"],
                         {"status": "unavailable", "code": "interaction_required"})

    def test_chain_analysis_ai_context_matches_python(self):
        """The desktop's `ui.chain_analysis.ai_context`: an investigation bound
        to the snapshot shown, projected to handles, its subject resolved."""
        def query(records):
            return {"kind": "ui.chain_analysis.query", "args": {}}

        def context(subject=None, snapshot=None, extra=None):
            def build(records):
                result = records[0]["data"]
                args = {"query": result["query"], "expected_snapshot_id": snapshot or result["snapshot_id"]}
                nodes = result.get("nodes") or []
                if subject == "node" and nodes:
                    args["subject"] = nodes[0]["id"]
                elif subject is not None:
                    args["subject"] = subject
                return {"kind": "ui.chain_analysis.ai_context", "args": {**args, **(extra or {})}}
            return build

        steps = [
            query,
            context(),
            context(subject="node"),
            context(subject="unknown-subject"),
            context(subject=" "),
            context(snapshot="0" * 64),
            context(extra={"limit": 1}),
            lambda records: {"kind": "ui.chain_analysis.ai_context", "args": {"query": {}}},
        ]
        for fixture in ("desktop", "reports"):
            with self.subTest(fixture=fixture):
                seed = self.seed(fixture)
                results = []
                for side, prefix in enumerate(([str(Path(RUST_BIN).resolve())], [sys.executable, str(REFERENCE)])):
                    root = self.base / f"context-{fixture}-{side}"
                    shutil.copytree(seed, root)
                    references = {}
                    results.append([_mask(line, references) for line in self.chained(prefix, root, steps)[1:]])
                self.assertEqual(results[0], results[1])
                projected = json.loads(results[0][1])
                self.assertEqual(projected["kind"], "ui.chain_analysis.ai_context", projected)

    def test_review_apply_flow_matches_python(self):
        """Plan, consent preview, apply and receipt, as a chat turn runs them."""
        # The desktop book holds incompatible network histories, which block
        # review: it pins the refusals, the reports book the applied flow.
        for fixture in ("desktop", "reports"):
            ids = FIXTURE_IDS[fixture]
            scope = {"scope_workspace_id": ids["ws"], "scope_profile_id": ids["profile"]}

            def tool(name, arguments, call_id):
                return {"kind": "test.ai_tool", "args": {"name": name, "arguments": arguments, "call_id": call_id,
                        "state": {**REMOTE, **scope, "consented_call_ids": [call_id]}}}

            def output(records, index):
                return json.loads(records[index]["data"]["output"])

            def plan(records):
                cases = output(records, 0)
                version = cases["envelope"]["data"]["input_version"] if cases.get("ok") else 0
                operations = [{"type": "exclude", "transaction_id": ids["q"], "reason": "Spam dust"}]
                return tool("ui_review_plan", {"operations": operations, "expected_input_version": version}, "call_plan")

            def apply_arguments(records):
                planned = output(records, 2)
                artifact = planned["envelope"]["data"] if planned.get("ok") else {"operations": []}
                return {"artifact": artifact, "idempotency_key": "ai-review-1"}

            def preview(records):
                return {"kind": "test.ai_consent_preview", "args": {
                    "name": "ui_review_apply", "arguments": apply_arguments(records), "call_id": "call_apply",
                    "state": {**REMOTE, **scope}}}

            def request_input(records):
                cases = output(records, 0)
                found = cases["envelope"]["data"].get("cases") or [] if cases.get("ok") else []
                case_ids = [case["case_id"] for case in found if str(case.get("case_id", "")).startswith("quarantine:")]
                return tool("ui_review_request_input", {
                    "action": "attach_evidence", "case_ids": case_ids[:1] or ["quarantine:none"],
                    "expected_input_version": cases["envelope"]["data"]["input_version"] if cases.get("ok") else 0,
                    "explanation": "Attach the bank statement from /home/user/statements/2025.pdf"}, "call_input")

            steps = [
                lambda records: tool("ui_review_cases", {}, "call_cases"),
                # Before the plan changes the input version: the request's
                # digest covers the stored, redacted explanation.
                request_input,
                plan,
                preview,
                lambda records: tool("ui_review_apply", apply_arguments(records), "call_apply"),
                preview,
                lambda records: tool("ui_review_receipt", {"idempotency_key": "ai-review-1"}, "call_receipt"),
            ]
            with self.subTest(fixture=fixture):
                seed = self.seed(fixture)
                masker = Masker(self.started)
                results = []
                for side, prefix in enumerate(([str(Path(RUST_BIN).resolve())], [sys.executable, str(REFERENCE)])):
                    root = self.base / f"review-{fixture}-{side}"
                    shutil.copytree(seed, root)
                    lines = self.chained(prefix, root, steps)
                    results.append(([masker.text(line, side=side) for line in lines[1:]],
                                    masker.tables(self.snapshot(root), side=side)))
                self.assertEqual(results[0][0], results[1][0])
                self.assertEqual(results[0][1], results[1][1])
                applied = json.loads(results[0][0][4])["data"]["tool_result"]["data"]
                self.assertIs(applied["ok"], fixture == "reports", applied)
                requested = json.loads(results[0][0][1])["data"]["tool_result"]["data"]
                self.assertIs(requested["ok"], fixture == "reports", requested)

    @unittest.skipUnless(shutil.which("unshare"), "needs unshare for a network namespace")
    def test_no_network_traffic_without_a_network(self):
        """The whole executor run is identical inside an empty network namespace."""
        # Same uid inside, so book ownership checks see the real owner.
        sandbox = ("unshare", "--map-current-user", "--net")
        probe = subprocess.run([*sandbox, "true"], capture_output=True)
        if probe.returncode != 0:
            self.skipTest("unprivileged network namespaces are unavailable")
        seed = self.seed("desktop")
        requests = tool_requests() + auto_read_requests()
        outside = self.base / "outside"
        inside = self.base / "inside"
        shutil.copytree(seed, outside)
        shutil.copytree(seed, inside)
        rust = [str(Path(RUST_BIN).resolve())]
        normal = self.session(rust, outside, requests)
        isolated = self.session(rust, inside, requests, sandbox=sandbox)
        self.assertEqual(len(normal), len(isolated))
        inside_references, outside_references = {}, {}
        for have, want in zip(isolated[1:], normal[1:]):
            self.assertEqual(_mask(have, inside_references), _mask(want, outside_references))


_UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[45][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}")
_STAMP = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,6})?(?:Z|\+00:00)")


_XLSX_BYTES = re.compile(rb'(\\?"bytes\\?":)\d+')


def _xlsx_artifact(masker, line, side):
    """XLSX ZIP bytes intentionally differ between the writers
    (``tests/test_rust_full_xlsx.py``): their size and digest are declared
    per side; the workbook content is compared by the XLSX oracles."""
    record = json.loads(line)
    data = ((record.get("data") or {}).get("tool_result") or {}).get("data") or {}
    envelope = data.get("envelope") or {}
    if not str(envelope.get("kind", "")).endswith("xlsx"):
        return line
    snapshot = (envelope.get("data") or {}).get("report_snapshot") or {}
    if isinstance(snapshot.get("content_sha256"), str):
        masker.declare_digest(snapshot["content_sha256"], side)
    return _XLSX_BYTES.sub(rb"\1<bytes>", line)


class Masker:
    """``tests/test_rust_review_workflow.Masker`` for writes: generated
    identities (uuid4, and the uuid5 ids derived from them) map to their
    order of first use per side, and clocks a run samples, including the
    pinned journal clock either side may use, read as one token. Tables are
    walked in name order with rows ordered by their masked text, so no
    mapping depends on the random values themselves."""

    # Digests over generated identities (component terms and evidence
    # commitments hash their own uuid4 ids) map like the ids themselves.
    DERIVED_DIGEST_TABLES = {"custody_component_economic_terms", "custody_component_evidence_commitments"}
    _DIGEST = re.compile(r"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])")

    def __init__(self, since):
        self.since = since
        self.maps = ({}, {})
        self.digests = ({}, {})

    def declare_digest(self, value, side):
        """A digest of bytes that intentionally differ (XLSX ZIP containers)."""
        self.digests[side].setdefault(value, f"<digest:{len(self.digests[side])}>")

    def _digests(self, side, text, every=False):
        names = self.digests[side]
        if every:
            return self._DIGEST.sub(lambda match: names.setdefault(match.group(0), f"<digest:{len(names)}>"), text)
        return self._DIGEST.sub(lambda match: names.get(match.group(0), match.group(0)), text)

    def _clock(self, match):
        text = match.group(0)
        try:
            stamp = datetime.fromisoformat(text.replace("Z", "+00:00"))
        except ValueError:
            return text
        return "<now>" if stamp >= self.since or text.startswith(CLOCK[:19]) else text

    def _blank(self, ids, text):
        return _STAMP.sub(self._clock, _UUID.sub(lambda match: ids.get(match.group(0), "<uuid>"), text))

    def _text(self, ids, text):
        def identity(match):
            return ids.setdefault(match.group(0), f"<uuid:{len(ids)}>")
        return _STAMP.sub(self._clock, _UUID.sub(identity, text))

    def text(self, line, side):
        text = line.decode() if isinstance(line, bytes) else line
        return self._digests(side, self._text(self.maps[side], text))

    def tables(self, snapshot, side):
        ids = self.maps[side]
        result = {}
        for table in sorted(snapshot):
            every = table in self.DERIVED_DIGEST_TABLES

            def cell(value, blank=False):
                if not isinstance(value, str):
                    return value
                masked = self._blank(ids, value) if blank else self._text(ids, value)
                return self._DIGEST.sub("<digest>", masked) if blank and every else self._digests(side, masked, every)

            rows = sorted(snapshot[table], key=lambda row: repr(tuple(cell(value, blank=True) for value in row)))
            result[table] = sorted((tuple(cell(value) for value in row) for row in rows), key=repr)
        return result


FOCUSED_WRITES = {
    "ui.custody.review.apply",
    "ui.transactions.metadata.update",
    "ui.transactions.history.revert",
    "ui.journals.quarantine.resolve",
    "ui.journals.process",
    "ui.transfers.pair",
    "ui.transfers.components.apply",
}

FIXTURE_IDS = {
    # The empty book has no context: its calls carry no frozen scope.
    "empty": {"ws": None, "profile": None, "in": "a", "out": "g", "q": "c", "wallet": "cold"},
    "desktop": {"ws": "w", "profile": "p", "in": "a", "out": "g", "q": "c", "wallet": "cold"},
    "reports": {"ws": "ws", "profile": "p", "in": "in", "out": "out", "q": "no-tax", "wallet": "cold"},
}
CUSTODY_AUDIT = {
    "provider_kind": "local",
    "model": "test-model",
    "consent_decision": "allow_once",
    "consent_requested_at": "2026-03-04T05:06:00Z",
    "consent_decided_at": "2026-03-04T05:06:05Z",
}
# Writes Rust refuses, checked without Python: export targets whose
# renderers are not native (crates/ai REFUSED_EXPORTS), tools whose service
# is not native (REFUSED_MUTATIONS), and wallet sync of sources that need
# Python's direct-sync path, which every fixture book does. `None` matches
# every format.
REFUSED_MUTATIONS = {
    ("ui.reports.export", "pdf"),
    ("ui.reports.export", "package"),
    ("ui.source_funds.export", None),
    ("ui.btcpay.provenance.review", None),
    ("ui.wallets.sync", None),
    ("ui.attachments.add_from_chat", None),
    ("ui.imports.rollback", None),
    ("ui.transfers.dismissals.delete", None),
    ("ui.wallets.import_attachment.apply", None),
    ("ui.wallets.import_document.apply", None),
}


def _mutating_extra() -> dict[str, list[dict]]:
    """Argument cases beyond the schema fill; ``{in}`` style fields name the
    fixture's own rows."""
    conversion = {"component_type": "swap", "conservation_mode": "conversion", "legs": [
        {"id": "{out}", "role": "source", "amount_msat": 1000},
        {"id": "{in}", "role": "destination", "amount_msat": 1000},
    ]}
    return {
        "ui.journals.quarantine.resolve": [
            {"transaction": "{q}", "action": "exclude", "reason": "Dust from an old faucet"},
            {"transaction": "{q}", "action": "price_override", "fiat_rate": "50000", "reason": "Exchange statement", "reprocess": False},
            {"transaction": "{q}", "action": "price_override", "fiat_rate": "1", "fiat_value": "2", "reason": "both"},
        ],
        "ui.rates.rebuild": [{"reprice_transactions": False}],
        "ui.wallets.sync": [{"wallet": "{wallet}", "process_journals": False}, {"wallet": " {wallet} ", "all": True}],
        "ui.maintenance.configure": [{"report_read_sync": False, "background_enabled": False}, {"auto_sync_before_report_reads": True}],
        "ui.maintenance.run": [{"sync": "never"}, {"sync": "if_enabled"}],
        "ui.source_funds.sources.create": [
            {"source_type": "fiat_purchase", "label": "Salary savings", "amount": "0.01", "fiat_currency": "EUR", "fiat_value": "400"},
            {"source_type": "gift", "label": "See /home/user/notes.txt"},
        ],
        "ui.source_funds.links.create": [
            {"to_transaction": "{in}", "link_type": "self_transfer", "allocation_amount": "0.00001", "explanation": "Own wallet"},
        ],
        "ui.source_funds.suggest": [{"target_transaction": "{in}"}],
        "ui.source_funds.links.bulk_review": [{"target_transaction": "{in}"}],
        "ui.source_funds.assemble": [{"target_transaction": "{in}"}],
        "ui.source_funds.cases.save": [{"target_transaction": "{in}"}],
        "ui.custody.review.apply": [
            {"action": "reopen", "gap_id": "gap-unknown", "expected_input_version": 0, "reason": "AI proposal"},
        ],
        "ui.transfers.components.apply": [
            {"components": [conversion], "expected_input_version": 0},
            {"components": [{**conversion, "conversion_reviewed": True}], "expected_input_version": 0, "activate": False},
        ],
        "ui.transfers.pair": [{"tx_out": "{out}", "tx_in": "{in}", "kind": "self_transfer"}],
        "ui.transfers.dismiss": [{"tx_out": "{out}", "tx_in": "{in}"}],
        "ui.transfers.update": [{"pair_id": "x", "notes": "AI note"}],
        "ui.transfers.payouts.create": [{"tx_out": "{out}", "payout_asset": "BTC", "payout_amount": "0.0001"}],
        "ui.saved_views.create": [{"surface": "transactions", "name": "AI view"}],
        "ui.loans.mark": [{"txid": "{out}", "as": "collateral"}],
        "ui.loans.link": [{"txids": ["{out}", "{in}"]}],
        "ui.loans.unmark": [{"txid": "{out}"}],
        "ui.transactions.metadata.update": [
            {"transaction": "{in}", "note": "Reviewed with the assistant", "tags": ["ai-review"]},
            {"transaction": "{out}", "review_status": "completed", "reason": "Checked"},
        ],
        "ui.transactions.history.revert": [{"transaction": "{in}", "event": "missing"}],
        "ui.documents.create": [{"label": "Invoice 7", "document_type": "invoice", "notes": "https://shop.example/invoice/7"}],
        "ui.reports.export": [
            {"report": "full", "format": "csv"},
            {"report": "full", "format": "xlsx", "verify": True},
            {"report": "capital_gains", "format": "csv", "year": 2025},
            {"report": "austrian_e1kv", "format": "csv", "year": 2025},
            {"report": "exit_tax", "format": "xlsx", "departure_date": "2025-06-30", "destination": "eu_eea"},
            {"report": "summary", "format": "pdf", "wallet": "{wallet}"},
            {"report": "audit_package", "format": "package", "transaction": "{in}"},
            {"report": "full", "format": "package"},
        ],
        "ui.source_funds.export": [{"case": " c ", "format": "bundle"}],
        "ui.chain_analysis.labels.upsert": [
            {"subject": "tx:{in}", "chain": "bitcoin", "network": "main", "label": "Mine", "category": "self", "source": "user", "confidence": "user_confirmed"},
        ],
        "ui.chain_analysis.watches.evaluate": [{"limit": 1}],
    }


def _render(value, ids):
    if isinstance(value, str):
        for key, item in ids.items():
            if item is not None:
                value = value.replace("{" + key + "}", item)
        return value
    if isinstance(value, list):
        return [_render(item, ids) for item in value]
    if isinstance(value, dict):
        return {key: _render(item, ids) for key, item in value.items()}
    return value


def mutating_requests(fixture, tool):
    """One mutating tool's cases, each call bound to its own consent."""
    ids = FIXTURE_IDS[fixture]
    scope = {"scope_workspace_id": ids["ws"], "scope_profile_id": ids["profile"]} if ids["ws"] else {}
    cases = [{}]
    filled = _fill(tool.parameters)
    if filled and filled not in cases:
        cases.append(filled)
    cases += [_render(case, ids) for case in _mutating_extra().get(tool.name, []) if case not in cases]
    states = [{**REMOTE, **scope}]
    if tool.name == "ui.custody.review.apply":
        states = [{**LOCAL, **scope}, {**REMOTE, **scope}, dict(LOCAL)]
    elif tool.name.startswith("ui.chain_analysis."):
        states = [{**REMOTE, **scope}, {**LOCAL, **scope}]
    elif tool.name == "ui.maintenance.run":
        states = [{**REMOTE, **scope}, {**REMOTE, **scope, "auto_sync_attempted": True}]
    compared, refused = [], []
    for arguments in cases:
        for state in states:
            call_id = f"call_{len(compared) + len(refused) + 1}"
            args = {"name": tool.provider_name, "arguments": arguments, "call_id": call_id,
                    "state": {**copy.deepcopy(state), "consented_call_ids": [call_id]}}
            if tool.name == "ui.custody.review.apply":
                args["custody_audit"] = CUSTODY_AUDIT
            request = {"kind": "test.ai_tool", "args": args}
            target = (tool.name, arguments.get("format") if isinstance(arguments, dict) else None)
            listed = target in REFUSED_MUTATIONS or (tool.name, None) in REFUSED_MUTATIONS
            (refused if listed else compared).append(request)
    if tool.name == "ui.custody.review.apply":
        # Without its consent audit context the write is refused, unaudited.
        compared.append({"kind": "test.ai_tool", "args": {
            "name": tool.provider_name, "arguments": cases[-1], "call_id": "call_bare",
            "state": {**LOCAL, **scope, "consented_call_ids": ["call_bare"]}}})
    return compared, refused


# Read tools still answering `not_implemented` (crates/ai REFUSED_READS).
# Mirrors crates/ai REFUSED_READS: services other slices or later S12 steps
# own.
REFUSED_READS = {
    "ui.connections.node.snapshot",
    "ui.imports.list",
    "ui.imports.rollback.plan",
    "ui.journals.quarantine.groups",
    "ui.transactions.verify",
    "ui.transfers.dismissals.list",
    "ui.visuals.render",
    "ui.wallets.import_attachment.plan",
    "ui.wallets.import_document.plan",
    "ui.wallets.read_document",
    "ui.wallets.request_input",
}


if __name__ == "__main__":
    unittest.main()
