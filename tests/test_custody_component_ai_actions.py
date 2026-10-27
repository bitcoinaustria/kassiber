"""The in-app AI can revise, supersede, undo and activate custody components.

Every action keeps the plan -> consent -> apply sequence bound to the book's
input version, and a blocking conflict names its target and remedy.
"""

from __future__ import annotations

import queue
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from kassiber.ai.tools import get_tool, summarize_tool_call
from kassiber.daemon import (
    AiToolRuntime,
    ParsedAiToolCall,
    _execute_mutating_ai_tool,
    _execute_read_only_ai_tool,
    _ui_swap_matching_payload_from_conn,
)
from kassiber.db import open_db, set_setting
from tests.test_custody_component_surfaces import _component_spec, _fixture


def _ai_spec(note: str) -> dict:
    """The fixture spec in the AI's schema, which requires leg ids."""

    spec = _component_spec(note=note)
    spec["legs"] = [
        {"id": f"leg-{index}", **leg} for index, leg in enumerate(spec["legs"])
    ]
    return spec


class CustodyComponentAiActionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-component-ai-")
        self.addCleanup(self.tmp.cleanup)
        data_root = Path(self.tmp.name) / "data"
        _fixture(data_root)
        self.conn = open_db(str(data_root))
        self.addCleanup(self.conn.close)
        set_setting(self.conn, "context_workspace", "ws")
        set_setting(self.conn, "context_profile", "profile")
        self.conn.commit()
        self.runtime = AiToolRuntime(
            data_root=str(data_root),
            runtime_config={},
            main_thread_tasks=queue.Queue(),
            maintenance_state={"provider_kind": "openai", "provider_full_access": True},
        )
        scoped = patch(
            "kassiber.daemon._run_scoped_ai_operation",
            side_effect=lambda _runtime, callback: callback(self.conn),
        )
        scoped.start()
        self.addCleanup(scoped.stop)

    def _plan(self, arguments: dict) -> dict:
        return _execute_read_only_ai_tool(
            ParsedAiToolCall(
                call_id="plan", name="ui.transfers.components.plan", arguments=arguments
            ),
            self.runtime,
        )

    def _apply(self, arguments: dict) -> dict:
        return _execute_mutating_ai_tool(
            ParsedAiToolCall(
                call_id="apply", name="ui.transfers.components.apply", arguments=arguments
            ),
            self.runtime,
        )

    def _active_component(self, note: str = "user interpretation") -> dict:
        args = {
            "workspace": "Main",
            "profile": "Book",
            "components": [_component_spec(note=note)],
            "activate": True,
        }
        preview = _ui_swap_matching_payload_from_conn(
            self.conn, "ui.transfers.components.plan", args
        )
        result = _ui_swap_matching_payload_from_conn(
            self.conn,
            "ui.transfers.components.apply",
            {**args, "expected_input_version": preview["input_version"]},
        )
        return result["components"][0]

    def _state(self, component_id: str) -> str:
        return self.conn.execute(
            "SELECT state FROM custody_components WHERE id = ?", (component_id,)
        ).fetchone()["state"]

    def test_schemas_offer_every_action_and_only_apply_requires_a_version(self):
        plan = get_tool("ui.transfers.components.plan").parameters
        apply = get_tool("ui.transfers.components.apply").parameters
        for schema in (plan, apply):
            self.assertEqual(
                schema["properties"]["action"]["enum"],
                ["create", "revise", "supersede", "undo", "activate"],
            )
            for field in ("components", "component_id", "spec", "activate", "reason"):
                self.assertIn(field, schema["properties"])
        self.assertNotIn("required", plan)
        self.assertEqual(apply["required"], ["expected_input_version"])

    def test_conflict_names_the_component_and_revise_replaces_it_on_the_same_anchors(self):
        existing = self._active_component()

        conflict = self._plan({"components": [_ai_spec("ai attempt")]})

        self.assertFalse(conflict["ok"])
        self.assertEqual(conflict["reason"], "custody_component_not_activatable")
        remediation = conflict["remediation"]
        self.assertEqual(remediation["action"], "revise_conflicting_component")
        self.assertEqual(remediation["tool"], "ui_transfers_components_plan")
        self.assertEqual(remediation["target"]["component_ids"], [existing["id"]])
        self.assertEqual(remediation["target"]["transaction_ids"], ["in", "out"])
        self.assertFalse(remediation["desktop_only"])
        self.assertEqual(remediation["arguments_hint"]["component_id"], existing["id"])
        self.assertEqual(remediation["alternatives"][0]["action"], "supersede")
        issue_codes = {issue["code"] for issue in conflict["details"]["issues"]}
        self.assertIn("active_transaction_membership_conflict", issue_codes)

        corrected = _ai_spec("corrected interpretation")
        arguments = {
            "action": "revise",
            "component_id": existing["id"],
            "spec": {"notes": corrected["notes"], "legs": corrected["legs"]},
            "activate": True,
            "reason": "the old interpretation was wrong",
        }
        plan = self._plan(arguments)
        self.assertTrue(plan["ok"], plan)
        version = plan["envelope"]["data"]["input_version"]
        applied = self._apply({**arguments, "expected_input_version": version})

        self.assertTrue(applied["ok"], applied)
        revised = applied["envelope"]["data"]["component"]
        self.assertEqual(revised["effective_state"], "active")
        self.assertEqual(revised["lineage_id"], existing["lineage_id"])
        self.assertEqual(self._state(existing["id"]), "superseded")
        self.assertEqual(
            {leg["transaction_id"] for leg in revised["legs"]}, {"out", "in"}
        )

    def test_supersede_apply_fails_closed_on_a_moved_input_version(self):
        existing = self._active_component()
        arguments = {
            "action": "supersede",
            "component_id": existing["id"],
            "reason": "wrong anchors",
        }
        plan = self._plan(arguments)
        self.assertTrue(plan["ok"], plan)
        stale_version = plan["envelope"]["data"]["input_version"]
        self.conn.execute(
            "UPDATE profiles SET journal_input_version = journal_input_version + 1 "
            "WHERE id = 'profile'"
        )
        self.conn.commit()

        stale = self._apply({**arguments, "expected_input_version": stale_version})

        self.assertFalse(stale["ok"])
        self.assertEqual(stale["reason"], "custody_review_plan_stale")
        self.assertEqual(stale["remediation"]["action"], "replan")
        self.assertEqual(stale["remediation"]["tool"], "ui_transfers_components_plan")
        self.assertEqual(
            stale["details"]["current_input_version"], stale_version + 1
        )
        self.assertEqual(self._state(existing["id"]), "active")

        fresh = self._plan(arguments)["envelope"]["data"]["input_version"]
        retired = self._apply({**arguments, "expected_input_version": fresh})
        self.assertTrue(retired["ok"], retired)
        self.assertEqual(self._state(existing["id"]), "superseded")

        restore = {"action": "undo", "component_id": existing["id"]}
        version = self._plan(restore)["envelope"]["data"]["input_version"]
        restored = self._apply({**restore, "expected_input_version": version})
        self.assertTrue(restored["ok"], restored)
        draft = restored["envelope"]["data"]["component"]
        self.assertEqual(draft["state"], "draft")
        self.assertEqual(draft["lineage_id"], existing["lineage_id"])

        activate = {"action": "activate", "component_id": draft["id"]}
        version = self._plan(activate)["envelope"]["data"]["input_version"]
        activated = self._apply({**activate, "expected_input_version": version})
        self.assertTrue(activated["ok"], activated)
        self.assertEqual(self._state(draft["id"]), "active")

    def test_action_field_combinations_fail_closed(self):
        existing = self._active_component()
        for arguments in (
            {"action": "supersede", "component_id": existing["id"], "activate": True},
            {"action": "activate", "component_id": existing["id"], "reason": "x"},
            {"action": "revise", "component_id": existing["id"]},
            {"action": "supersede"},
            {"action": "create", "component_id": existing["id"]},
        ):
            with self.subTest(arguments=arguments):
                result = self._plan(arguments)
                self.assertFalse(result["ok"])
                self.assertEqual(result["reason"], "validation")
        self.assertEqual(self._state(existing["id"]), "active")

    def test_ai_revision_spec_keeps_the_create_boundary(self):
        existing = self._active_component()
        hidden_field = self._plan(
            {
                "action": "revise",
                "component_id": existing["id"],
                "spec": {"evidence": {"claim": "trust me"}},
            }
        )
        self.assertFalse(hidden_field["ok"])
        self.assertEqual(hidden_field["reason"], "validation")

        self_attested = self._plan(
            {
                "action": "revise",
                "component_id": existing["id"],
                "spec": {"conversion_reviewed": True},
            }
        )
        self.assertFalse(self_attested["ok"])

        conversion = self._plan(
            {
                "action": "revise",
                "component_id": existing["id"],
                "spec": {"conservation_mode": "conversion"},
                "activate": True,
            }
        )
        self.assertFalse(conversion["ok"])
        self.assertEqual(conversion["reason"], "interaction_required")

    def test_consent_summaries_name_the_action_and_target(self):
        apply = get_tool("ui.transfers.components.apply")
        plan = get_tool("ui.transfers.components.plan")
        self.assertEqual(
            summarize_tool_call(apply, {"action": "supersede", "component_id": "c-1"}),
            "Retire custody component c-1 (anchors return to raw booking)",
        )
        self.assertEqual(
            summarize_tool_call(
                apply, {"action": "revise", "component_id": "c-1", "activate": True}
            ),
            "Replace custody component c-1 with a new active revision",
        )
        self.assertEqual(
            summarize_tool_call(apply, {"action": "undo", "component_id": "c-1"}),
            "Restore superseded custody component c-1 as a draft",
        )
        self.assertEqual(
            summarize_tool_call(plan, {"action": "activate", "component_id": "c-1"}),
            "Preview: activate draft custody component c-1",
        )
        self.assertEqual(
            summarize_tool_call(apply, {"components": [{}, {}]}),
            "Create 2 gap-resolution components",
        )


class AiToolErrorRemediationTests(unittest.TestCase):
    def test_restricted_access_denial_says_only_the_user_can_lift_it(self):
        result = _execute_read_only_ai_tool(
            ParsedAiToolCall(
                call_id="call", name="ui.custody.gaps.list", arguments={}
            ),
            AiToolRuntime(
                data_root="/tmp",
                runtime_config={},
                main_thread_tasks=queue.Queue(),
                maintenance_state={
                    "provider_kind": "openai",
                    "provider_full_access": False,
                },
            ),
        )
        self.assertEqual(result["reason"], "agent_access_restricted")
        self.assertEqual(result["remediation"]["action"], "enable_full_agent_access")
        self.assertTrue(result["remediation"]["desktop_only"])
        self.assertIsNone(result["remediation"]["tool"])

    def test_details_of_other_codes_stay_out_of_the_provider_context(self):
        from kassiber.daemon import _tool_result_from_app_error
        from kassiber.errors import AppError

        result = _tool_result_from_app_error(
            AppError("bad", code="validation", details={"echo": "raw input"})
        )
        self.assertNotIn("details", result)
        self.assertNotIn("remediation", result)

        case = _tool_result_from_app_error(
            AppError("changed", code="review_case_changed"),
            get_tool("ui.review.apply"),
        )
        self.assertEqual(case["remediation"]["action"], "reread_case")
        self.assertEqual(case["remediation"]["tool"], "ui_review_cases")


if __name__ == "__main__":
    unittest.main()
