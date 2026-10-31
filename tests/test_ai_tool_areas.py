"""Small models get a bounded catalog, an area index, and `load_tools`.

Advertising every schema costs ~36k tokens, more than a small local model's
context. These tests pin the budget, the index the system prompt carries
instead, and the mid-turn loading that reaches any area the routing missed.
"""

from __future__ import annotations

import io
import json
import queue
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from kassiber import daemon
from kassiber.ai.broker_client import BrokerAIClient
from kassiber.ai.prompt import (
    DEFAULT_KASSIBER_SYSTEM_PROMPT,
    KASSIBER_SYSTEM_PROMPT,
    build_chat_messages,
    build_responses_tools,
)
from kassiber.ai.tools import (
    CORE_TOOL_NAMES,
    LOAD_TOOLS_NAME,
    LOADABLE_TOOL_AREAS,
    TOOL_AREA_SUMMARIES,
    TOOL_CAPABILITY_NAMES,
    get_tool,
    responses_tool_definitions,
    select_tool_capabilities,
    tool_area_index,
)

# Compact JSON characters per area (about four per token). Raise a ceiling only
# with a reason: every schema in an area is paid for on each question that
# routes to it.
AREA_SCHEMA_CHAR_BUDGETS = {
    "core": 2_600,
    "review": 53_000,
    "workspace": 1_000,
    "transactions": 22_000,
    "reports": 20_000,
    # Raised from 10k for the attachment import (plan + consented apply), its
    # rollback and the connect-wallet handoff: without them the assistant
    # could only print a CLI command for a file it had already mapped.
    "wallets": 14_500,
    "loans": 2_000,
    "privacy": 40_000,
    "source_funds": 18_000,
    "merchant": 3_500,
    "transfers": 37_000,
    "operations": 11_500,
}


def _chars(tools: list[dict]) -> int:
    return len(json.dumps(tools, separators=(",", ":")))


def _names(tools: list[dict]) -> set[str]:
    return {tool["name"] for tool in tools}


class ToolAreaIndexTest(unittest.TestCase):
    def test_every_area_but_core_is_indexed_and_loadable(self):
        self.assertEqual(set(TOOL_CAPABILITY_NAMES) - {"core"}, set(TOOL_AREA_SUMMARIES))
        self.assertEqual(tuple(TOOL_AREA_SUMMARIES), LOADABLE_TOOL_AREAS)
        schema = get_tool(LOAD_TOOLS_NAME).parameters["properties"]["areas"]
        self.assertEqual(schema["items"]["enum"], list(LOADABLE_TOOL_AREAS))

    def test_system_prompt_carries_a_static_index_only_when_tools_run(self):
        index = tool_area_index()
        self.assertLess(len(index), 1_500)
        for area in LOADABLE_TOOL_AREAS:
            self.assertIn(f"- {area}: ", index)
        self.assertIn("never apply a tax rate yourself", index)

        first = build_chat_messages([{"role": "user", "content": "a"}], system_prompt_kind="kassiber")
        second = build_chat_messages([{"role": "user", "content": "b"}], system_prompt_kind="kassiber")
        # Byte-identical across turns, so provider prompt caches keep working.
        self.assertEqual(first[0]["content"], second[0]["content"])
        self.assertEqual(first[0]["content"], KASSIBER_SYSTEM_PROMPT)
        self.assertIn(index, KASSIBER_SYSTEM_PROMPT)
        # Raised from 3,600 for the prompt's evidence rules (about 520 chars).
        self.assertLess(len(KASSIBER_SYSTEM_PROMPT), 4_200)

        plain = build_chat_messages(
            [{"role": "user", "content": "a"}],
            system_prompt_kind="kassiber",
            tools_enabled=False,
        )
        self.assertEqual(plain[0]["content"], DEFAULT_KASSIBER_SYSTEM_PROMPT)


class ToolCatalogBudgetTest(unittest.TestCase):
    def test_each_area_stays_within_its_schema_budget(self):
        self.assertEqual(set(AREA_SCHEMA_CHAR_BUDGETS), set(TOOL_CAPABILITY_NAMES))
        for area, budget in AREA_SCHEMA_CHAR_BUDGETS.items():
            with self.subTest(area=area):
                tools = responses_tool_definitions(
                    include_mutating=True, capabilities=frozenset({area})
                )
                self.assertLessEqual(_chars(tools), budget)

    def test_discovery_questions_get_the_core_area_not_the_catalog(self):
        for question in (
            "What can you do?",
            "Show all tools and capabilities",
            "Which tools would you want to have?",
            "Was kannst du alles?",
        ):
            with self.subTest(question=question):
                tools = build_responses_tools([{"role": "user", "content": question}])
                self.assertLessEqual(_chars(tools), AREA_SCHEMA_CHAR_BUDGETS["core"])

    def test_load_tools_is_never_part_of_an_area_or_profile(self):
        for profile in (None, "core", "scoped", "full"):
            with self.subTest(profile=profile):
                tools = build_responses_tools(
                    [{"role": "user", "content": "report"}], profile=profile
                )
                self.assertNotIn(LOAD_TOOLS_NAME, _names(tools))
        self.assertNotIn(LOAD_TOOLS_NAME, _names(build_responses_tools()))


class GainsRoutingTest(unittest.TestCase):
    def test_gains_and_income_wording_reaches_the_reports_area(self):
        for question in (
            "Show me lot-by-lot for 2026",
            "What was my staking income?",
            "How much mining income did I get",
            "What's my unrealized PnL?",
            "Wie hoch waren meine Einkünfte 2025?",
            "Welchen Gewinn habe ich gemacht?",
            "Zeig mir die Kennzahl 174",
        ):
            with self.subTest(question=question):
                self.assertIn(
                    "reports",
                    select_tool_capabilities([{"role": "user", "content": question}]),
                )
                names = _names(build_responses_tools([{"role": "user", "content": question}]))
                self.assertIn("ui_reports_capital_gains", names)
                self.assertIn("ui_reports_tax_summary", names)

    def test_capital_gains_tool_takes_year_asset_and_limit(self):
        properties = get_tool("ui.reports.capital_gains").parameters["properties"]
        self.assertEqual(set(properties), {"tax_year", "asset", "limit"})
        self.assertEqual(properties["limit"]["maximum"], 200)


class LoadToolsLoopTest(unittest.TestCase):
    class Out:
        def write(self, payload):
            pass

    def _runtime(self):
        return daemon.AiToolRuntime(
            data_root="/not-used",
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={"egress_after_id": 0},
        )

    def _validated(self, *, profile="scoped", question="Hello there"):
        return {
            "messages": [{"role": "user", "content": question}],
            "model": "qwen",
            "options": {},
            "session_id": None,
            "persist": False,
            "system_prompt_kind": "kassiber",
            "system_prompt": None,
            "tools_enabled": True,
            "tool_profile": profile,
            "tool_loop_max_iterations": 3,
            "screen_context": None,
            "attachments": [],
        }

    def _run(self, client, validated, calls_by_round):
        seen: list[set[str]] = []
        runtime = self._runtime()

        def fake_turn(_request_id, _client, _validated, _context, tools, *_rest):
            seen.append(_names(tools))
            calls = calls_by_round[len(seen) - 1] if len(seen) <= len(calls_by_round) else []
            return daemon.AiToolTurnResult(
                tool_calls=calls,
                content="" if calls else "done",
                reasoning="",
                finish_reason="tool_calls" if calls else "stop",
                response_output=[],
                notices=(),
            )

        stream = io.StringIO()
        active_ai_chats = daemon.ActiveAiChats()
        _key, active_chat = active_ai_chats.register("req-1")
        with patch.object(daemon, "_stream_ai_chat_tool_turn", fake_turn), patch.object(
            daemon, "_run_auto_read_tools", lambda **_kwargs: None
        ):
            daemon._run_ai_chat_tool_loop(
                "req-1",
                client,
                {"name": "local", "kind": "openai", "base_url": "http://127.0.0.1:1/v1"},
                validated,
                daemon._OutputChannel(stream),
                active_chat,
                runtime,
                active_ai_chats,
            )
        records = [json.loads(line) for line in stream.getvalue().splitlines()]
        return seen, runtime, records

    @staticmethod
    def _load_call(*areas):
        return {
            "id": "call_load",
            "function": {"name": LOAD_TOOLS_NAME, "arguments": json.dumps({"areas": list(areas)})},
        }

    def test_http_model_loads_an_area_and_can_call_it_next_round(self):
        client = SimpleNamespace(last_provider_session_id=None)
        seen, runtime, records = self._run(
            client, self._validated(), [[self._load_call("reports")]]
        )

        self.assertEqual(len(seen), 2)
        self.assertIn(LOAD_TOOLS_NAME, seen[0])
        self.assertNotIn("ui_reports_capital_gains", seen[0])
        self.assertIn("ui_reports_capital_gains", seen[1])
        # The authorization set and the privacy receipt grow with the schemas.
        self.assertIn("ui_reports_capital_gains", runtime.maintenance_state["advertised_tools"])
        result = next(r for r in records if r["kind"] == "ai.chat.tool_result")
        self.assertTrue(result["data"]["ok"])
        self.assertIn("ui_reports_tax_summary", result["data"]["envelope"]["data"]["added_tools"])

    def test_loading_keeps_the_core_profile_limits(self):
        client = SimpleNamespace(last_provider_session_id=None)
        seen, runtime, _records = self._run(
            client, self._validated(profile="core"), [[self._load_call("privacy")]]
        )
        offered = {get_tool(name).name for name in seen[-1] if get_tool(name)}
        self.assertLessEqual(offered - {LOAD_TOOLS_NAME}, set(CORE_TOOL_NAMES))
        self.assertNotIn("ui_reports_privacy_mirror", runtime.maintenance_state["advertised_tools"])

    def test_on_device_only_tools_stay_out_for_a_remote_provider(self):
        client = SimpleNamespace(last_provider_session_id=None)
        seen, _runtime, _records = self._run(
            client, self._validated(), [[self._load_call("privacy")]]
        )
        self.assertFalse(any(name.startswith("ui_chain_analysis_sources_") for name in seen[-1]))
        self.assertNotIn("ui_chain_analysis_acquire_apply", seen[-1])

    def test_cli_agents_and_the_full_profile_get_no_loader(self):
        broker = object.__new__(BrokerAIClient)
        self.assertFalse(daemon._ai_turn_can_load_tools(broker, {"tool_profile": "scoped"}))
        http = SimpleNamespace()
        self.assertFalse(daemon._ai_turn_can_load_tools(http, {"tool_profile": "full"}))
        self.assertTrue(daemon._ai_turn_can_load_tools(http, {"tool_profile": "core"}))

    def test_direct_call_without_a_live_loader_is_denied(self):
        call = daemon.ParsedAiToolCall(
            call_id="c1", name=LOAD_TOOLS_NAME, arguments={"areas": ["reports"]}
        )
        result = daemon._execute_read_only_ai_tool(call, self._runtime())
        self.assertFalse(result["ok"])


if __name__ == "__main__":
    unittest.main()
