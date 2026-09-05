"""Agent review crosses the same scoped, consented accounting boundary as the CLI."""
from __future__ import annotations

import copy
import queue
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.core import custody_journal
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from tests.test_custody_component_surfaces import _fixture, _component_spec


@pytest.fixture
def book(tmp_path):
    _fixture(tmp_path)
    conn = open_db(str(tmp_path))
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "profile")
    profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    state = custody_journal.build_ledger_state(conn, profile)
    custody_journal.store_ledger_state(conn, profile, state)
    conn.commit()
    runtime = daemon.AiToolRuntime(
        data_root=str(tmp_path), runtime_config={}, main_thread_tasks=queue.Queue(),
        maintenance_state={"scope_workspace_id": "ws", "scope_profile_id": "profile", "provider_kind": "local"},
    )
    with patch("kassiber.daemon._run_on_daemon_main_thread", side_effect=lambda runtime, callback: callback(conn)):
        yield conn, runtime
    conn.close()


def _price_plan(conn):
    version = conn.execute("SELECT journal_input_version FROM profiles WHERE id='profile'").fetchone()[0]
    return daemon._review_workflow_payload(conn, "ui.review.plan", {
        "expected_input_version": version,
        "operations": [{"type": "price_override", "transaction_id": "in", "fiat_rate": "100000", "reason": "User supplied the verified acquisition rate"}],
    }, authored_source="ai_tool")


def test_plan_consent_apply_and_retry_use_actual_book(book):
    conn, runtime = book
    artifact = _price_plan(conn)
    arguments = {"artifact": artifact, "idempotency_key": "review-test"}
    preview = daemon._ai_review_consent_preview(runtime, arguments)
    assert preview == {"status": "ready", "artifact": artifact}
    receipt = daemon._review_workflow_payload(conn, "ui.review.apply", arguments, authored_source="ai_tool")
    assert receipt["status"] == "verified"
    assert receipt["verification"]["quarantine_count"] == artifact["after"]["quarantine_count"]
    assert receipt["authored_source"] == "ai_tool"
    assert daemon._ai_review_consent_preview(runtime, arguments) == {"status": "applied", "receipt": daemon.redact_ai_tool_result(receipt)}
    assert daemon._review_workflow_payload(conn, "ui.review.apply", arguments, authored_source="ai_tool") == receipt
    assert conn.execute("SELECT COUNT(*) FROM review_workflow_receipts").fetchone()[0] == 1


def test_consent_never_displays_model_fabricated_effects(book):
    conn, runtime = book
    artifact = _price_plan(conn)
    fabricated = copy.deepcopy(artifact)
    fabricated["after"]["quarantine_count"] = 0
    # Even a self-consistent caller-computed digest is not accounting authority.
    from kassiber.core.review_workflow import _digest
    fabricated["digest"] = _digest({key: value for key, value in fabricated.items() if key != "digest"})
    preview = daemon._ai_review_consent_preview(runtime, {"artifact": fabricated, "idempotency_key": "fake"})
    assert preview["status"] == "unavailable"
    assert "artifact" not in preview
    assert conn.execute("SELECT COUNT(*) FROM review_workflow_receipts").fetchone()[0] == 0


def test_plan_scope_is_pinned_until_consent(book):
    conn, runtime = book
    artifact = _price_plan(conn)
    set_setting(conn, "context_profile", "another-book")
    conn.commit()
    preview = daemon._ai_review_consent_preview(runtime, {"artifact": artifact, "idempotency_key": "scope"})
    assert preview == {"status": "unavailable", "code": "stale_context"}


@pytest.mark.parametrize("reason", ["Evidence at /Users/alice/private/report.pdf", "Read https://private.example/evidence"])
def test_ai_artifact_cannot_silently_change_during_redaction(book, reason):
    conn, _runtime = book
    operations = [{"type": "exclude", "transaction_id": "in", "reason": reason}]
    assert daemon.redact_ai_tool_result(operations) != operations
    with pytest.raises(AppError, match="private paths"):
        daemon._validate_ai_review_operations(operations)
    assert conn.execute("SELECT COUNT(*) FROM review_workflow_receipts").fetchone()[0] == 0


@pytest.mark.parametrize("action", ["revise", "activate", "undo"])
def test_batch_cannot_bypass_ai_component_authority(book, action):
    conn, _runtime = book
    with pytest.raises(AppError) as caught:
        daemon._review_workflow_payload(conn, "ui.review.plan", {
            "expected_input_version": 0,
            "operations": [{"type": "custody_component", "request": {"action": action, "component_id": "existing"}}],
        }, authored_source="ai_tool")
    assert caught.value.code == "interaction_required"


def test_batch_cannot_self_attest_conversion_review():
    spec = _component_spec()
    spec.update(conservation_mode="conversion", conversion_reviewed=True)
    with pytest.raises(AppError) as caught:
        daemon._validate_ai_review_operations([{"type": "custody_component", "request": {"action": "create", "components": [spec]}}])
    assert caught.value.code == "interaction_required"


def test_apply_requires_once_only_consent():
    assert "ui.review.apply" in daemon.AI_TOOL_ONCE_ONLY_CONSENT


@pytest.mark.parametrize("decision", ["allow_once", "deny"])
def test_model_tool_loop_investigates_plans_and_applies_only_after_consent(book, decision):
    import io
    import json
    from types import SimpleNamespace

    conn, runtime = book
    chats = daemon.ActiveAiChats()
    _key, active = chats.register("review-loop")
    output = io.StringIO()
    seen = []
    validated = daemon._ai_chat_args({
        "model": "test-model", "tools_enabled": True,
        "messages": [{"role": "user", "content": "Investigate quarantine using existing evidence and request approval for a review plan."}],
    })
    assert validated["tool_loop_max_iterations"] == 16

    def fake_turn(_rid, _client, _validated, context, offered, _out, _cancel):
        results = [json.loads(item["output"]) for item in context.input_items if item.get("type") == "function_call_output"]
        if results:
            assert results[-1].get("ok") or decision == "deny", results[-1]
        count = len(seen)
        if count == 0:
            name, args = "ui.review.cases", {"limit": 100}
        elif count == 1:
            name, args = "ui.review.plan", {
                "expected_input_version": results[-1]["envelope"]["data"]["input_version"],
                "operations": [{"type": "price_override", "transaction_id": "in", "fiat_rate": "100000", "reason": "User supplied the verified acquisition rate"}],
            }
        elif count == 2:
            name, args = "ui.review.apply", {"artifact": results[-1]["envelope"]["data"], "idempotency_key": "model-loop"}
        else:
            return daemon.AiToolTurnResult([], "Review complete; unresolved cases remain." if decision == "allow_once" else "No changes approved.", "", "stop", [])
        entry = daemon.get_tool(name)
        assert entry.provider_name in {tool["name"] for tool in offered}
        seen.append(name)
        call = {"id": f"review-{count}", "function": {"name": entry.provider_name, "arguments": json.dumps(args)}}
        return daemon.AiToolTurnResult([call], "", "", "tool_calls", [])

    with (
        patch.object(daemon, "_stream_ai_chat_tool_turn", fake_turn),
        patch.object(daemon, "_run_auto_read_tools", lambda **kwargs: None),
        patch.object(active.consent, "wait", return_value=decision) as consent,
        patch.object(daemon, "_write_ai_chat_terminal") as terminal,
    ):
        daemon._run_ai_chat_tool_loop(
            "review-loop", SimpleNamespace(last_provider_session_id=None),
            {"name": "test", "kind": "local", "base_url": "http://localhost"},
            validated, daemon._OutputChannel(output), active, runtime, chats,
        )
    assert seen == ["ui.review.cases", "ui.review.plan", "ui.review.apply"]
    consent.assert_called_once()
    events = [json.loads(line) for line in output.getvalue().splitlines()]
    approval = next(event["data"] for event in events if event["kind"] == "ai.chat.tool_consent_required")
    assert approval["review_preview"]["status"] == "ready"
    assert approval["review_preview"]["artifact"]["before"]["quarantine_count"] == 2
    assert conn.execute("SELECT COUNT(*) FROM review_workflow_receipts").fetchone()[0] == (decision == "allow_once")
    terminal.assert_called_once()


def test_budget_checkpoint_drops_stale_cursor_and_retains_receipt():
    checkpoint = {}
    daemon._update_review_checkpoint(checkpoint, "ui.review.cases", {"ok": True, "envelope": {"data": {"input_version": 7, "next_cursor": "cursor7", "cases": [{"transaction_id": "tx"}]}}})
    daemon._update_review_checkpoint(checkpoint, "ui.review.apply", {"ok": True, "envelope": {"data": {"id": "receipt8", "result_input_version": 8}}})
    assert checkpoint == {"input_version": 8, "receipt_ids": ["receipt8"]}


def test_agent_can_enumerate_more_than_one_hundred_unresolved_cases(book):
    conn, _runtime = book
    for index in range(125):
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,occurred_at,direction,asset,amount,fee,created_at) "
            "SELECT ?,workspace_id,profile_id,wallet_id,?,?,occurred_at,direction,asset,amount,fee,created_at FROM transactions WHERE id='in'",
            (f"case-{index:03}", f"fingerprint-case-{index:03}", f"external-case-{index:03}"),
        )
    daemon.invalidate_journals(conn, "profile")
    conn.commit()
    first = daemon._review_workflow_payload(conn, "ui.review.cases", {"limit": 100}, authored_source="ai_tool")
    assert len(first["cases"]) == 100
    second = daemon._review_workflow_payload(conn, "ui.review.cases", {"limit": 100, "cursor": first["next_cursor"]}, authored_source="ai_tool")
    ids = [case["transaction_id"] for case in first["cases"] + second["cases"]]
    assert len(ids) == len(set(ids)) == 127
    assert second["next_cursor"] is None
    assert not conn.in_transaction


def test_review_budget_preserves_explicit_override_and_ordinary_default():
    request = {"model": "test", "tools_enabled": True, "messages": [{"role": "user", "content": "Show my balance"}]}
    assert daemon._ai_chat_args(request)["tool_loop_max_iterations"] == 8
    request["messages"][0]["content"] = "Investigate quarantine"
    assert daemon._ai_chat_args(request)["tool_loop_max_iterations"] == 16
    request["tool_loop_max_iterations"] = 3
    assert daemon._ai_chat_args(request)["tool_loop_max_iterations"] == 3


@pytest.mark.parametrize("component_request", [{}, {"action": {}}, {"action": []}, {"action": "invented"}])
def test_malformed_component_plan_has_typed_error_without_writes(book, component_request):
    conn, _runtime = book
    with pytest.raises(AppError) as caught:
        daemon._review_workflow_payload(conn, "ui.review.plan", {
            "expected_input_version": 0,
            "operations": [{"type": "custody_component", "request": component_request}],
        })
    assert caught.value.code == "validation"
    assert not conn.in_transaction
    assert conn.execute("SELECT COUNT(*) FROM review_workflow_receipts").fetchone()[0] == 0
