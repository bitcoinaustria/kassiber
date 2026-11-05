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
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
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



_GENERATED_AT = re.compile(rb'(\\?"generated_at\\?":\\?")\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ')


def _mask(line: bytes) -> bytes:
    """Declared mapping: read-time clocks (``generated_at``) are not compared."""
    return _GENERATED_AT.sub(rb"\1<generated_at>", line)


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
    }


def _fill(schema):
    generator = _load("crates/ai/scripts/gen_vectors.py")
    return generator.fill(schema)


def tool_requests():
    """Every read tool, by argument case and executor state."""
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
    for raw in ({"id": "c1", "function": {"name": "status", "arguments": "{bad"}}, {"id": "c2", "function": {"name": "ui_wallets_sync", "arguments": "{}"}}, {"function": {"name": "nope"}}):
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
                # Strict: any gated connection, loopback included, trips the
                # harness instead of reaching a socket.
                "KASSIBER_TEST_NO_EGRESS": "strict",
            }
        )
        (self.base / "home").mkdir()
        (self.base / "owners").mkdir()

    def seed(self, fixture: str) -> Path:
        from kassiber.db import open_db

        root = self.base / fixture / "seed"
        with closing(open_db(root)) as conn, conn:
            if FIXTURES[fixture] is not None:
                conn.executescript(FIXTURES[fixture].read_text())
        return root

    def session(self, argv_prefix, root, requests, *, sandbox=()):
        wire = [dict(request, request_id=str(index)) for index, request in enumerate(requests)]
        wire.append({"request_id": "stop", "kind": "daemon.shutdown"})
        payload = "".join(json.dumps(request) + "\n" for request in wire).encode()
        result = subprocess.run(
            [*sandbox, *argv_prefix, "--machine", "--data-root", str(root), "daemon"],
            env=self.env, input=payload, capture_output=True, timeout=900, cwd=ROOT,
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
            have, want = _mask(have), _mask(want)
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

        requests = tool_requests()
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
        for have, want in zip(isolated[1:], normal[1:]):
            self.assertEqual(_mask(have), _mask(want))


REFUSED_READS = {
    "ui.overview.snapshot", "ui.wallets.identify", "ui.wallets.analyze_file", "ui.connections.node.snapshot",
    "ui.reports.lightning_profitability", "ui.reports.privacy_mirror", "ui.journals.snapshot",
    "ui.journals.quarantine", "ui.journals.events.list", "ui.journals.transfers.list", "ui.report.blockers",
    "ui.workspace.health", "ui.next_actions", "ui.workspace.overview.snapshot", "ui.transactions.graph",
    "ui.transactions.review_context", "ui.audit.evidence.summary", "ui.review.badges", "ui.review.worklist",
    "ui.egress.snapshot", "ui.review.cases", "ui.review.request_input", "ui.review.plan",
}


if __name__ == "__main__":
    unittest.main()
