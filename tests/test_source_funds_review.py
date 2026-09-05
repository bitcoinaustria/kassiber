"""One provenance target can be investigated and resumed outside quarantine."""
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

import pytest

from kassiber import daemon
from kassiber.ai.tools import get_tool, select_tool_capabilities
from kassiber.core import source_funds_review
from kassiber.db import set_setting
from kassiber.errors import AppError
from tests.test_daemon_review_workflow import book  # noqa: F401
from tests.test_source_funds_cli import run_cli


def context(conn, **options):
    return daemon._ui_source_funds_payload_from_conn(
        conn, "ui.source_funds.review_context", {"target_transaction": "in", **options},
    )


def request(conn, inspected, action="attach_evidence"):
    return daemon._ui_source_funds_payload_from_conn(conn, "ui.source_funds.request_input", {
        "action": action, "target_transaction": inspected["target"]["transaction_id"],
        "recipe": inspected["recipe"], "expected_review_fingerprint": inspected["review_fingerprint"],
    })


def test_context_is_read_only_stable_and_preserves_exact_recipe(book):
    conn, _runtime = book
    before = list(conn.iterdump())
    inspected = context(conn, target_amount="0.00000050", report_purpose="planned_exchange_sale",
                        planned_destination="Reviewed destination", planned_note="Reviewed purpose")
    assert inspected["domain"] == "source_funds"
    assert inspected["target"]["transaction_id"] == "in"
    assert inspected["case_id"] == "source_funds:in"
    assert inspected["report"]["allocations"]["target_amount_msat"] == 50_000
    assert inspected["recipe"]["planned_destination"] == "Reviewed destination"
    assert inspected == context(conn, **inspected["recipe"])
    assert inspected["input_needs"]
    assert inspected["scope_truncated"] is False
    assert list(conn.iterdump()) == before
    assert not conn.in_transaction
    handoff = request(conn, inspected)
    assert handoff["cases"][0]["reason"] == "evidence_review"
    assert handoff["cases"][0]["case_id"] == "source_funds:in"
    assert handoff["recipe"] == inspected["recipe"]
    assert handoff["review_fingerprint"] == inspected["review_fingerprint"]


def seed_source(conn):
    hooks = daemon._source_funds_hooks()
    source = daemon.core_source_funds.create_source(
        conn, "ws", "profile", hooks, source_type="fiat_purchase", label="Purchase evidence",
        amount="0.00000099", fiat_value="0.099", acquired_at="2024-01-01T00:00:00Z",
    )
    link = daemon.core_source_funds.create_link(
        conn, "ws", "profile", hooks, from_source_ref=source["id"], to_transaction_ref="in",
        link_type="manual_source", allocation_amount="0.00000099", allocation_policy="explicit",
    )
    return source, link


def test_provenance_edits_invalidate_fingerprint_without_journal_version_change(book):
    conn, _runtime = book
    source, link = seed_source(conn)
    first = context(conn)
    assert len(first["sources"]) == len(first["links"]) == 1
    conn.execute("UPDATE source_funds_sources SET description='New source explanation' WHERE id=?", (source["id"],))
    conn.commit()
    second = context(conn)
    assert second["input_version"] == first["input_version"]
    assert second["review_fingerprint"] != first["review_fingerprint"]
    with pytest.raises(AppError) as raised:
        request(conn, first)
    assert raised.value.code == "source_funds_review_stale"
    conn.execute("UPDATE source_funds_links SET explanation='Changed link evidence' WHERE id=?", (link["id"],))
    conn.commit()
    assert context(conn)["review_fingerprint"] != second["review_fingerprint"]


def test_reachable_evidence_does_not_include_unrelated_transactions(book, tmp_path):
    conn, runtime = book
    source, _link = seed_source(conn)
    file = tmp_path / "statement.png"
    file.write_bytes(b"evidence")
    before = context(conn)
    attachment = daemon.core_attachments.add_attachment(
        conn, runtime.data_root, "ws", "profile", "out", daemon._attachment_hooks(), file_path=str(file),
    )
    # Unrelated outbound evidence is not part of this target's investigation.
    assert context(conn)["evidence"] == []
    assert context(conn)["review_fingerprint"] == before["review_fingerprint"]
    daemon.core_source_funds.attach_source_evidence(
        conn, "ws", "profile", daemon._source_funds_hooks(),
        source_ref=source["id"], attachment_id=attachment["id"],
    )
    after = context(conn)
    assert [item["id"] for item in after["evidence"]] == [attachment["id"]]
    assert after["review_fingerprint"] != before["review_fingerprint"]
    assert after["input_version"] == before["input_version"]


def test_nonquarantine_target_can_request_and_durably_attach_evidence(book, tmp_path):
    conn, runtime = book
    conn.execute("UPDATE transactions SET fiat_rate=100000 WHERE id='in'")
    conn.commit()
    cases = daemon._review_workflow_payload(conn, "ui.review.cases", {})
    assert "in" not in {case["transaction_id"] for case in cases["cases"]}
    inspected = context(conn)
    handoff = request(conn, inspected)
    file = tmp_path / "origin.png"
    file.write_bytes(b"original source statement")
    ctx = SimpleNamespace(conn=conn, data_root=runtime.data_root, document_import_sessions=daemon.DocumentImportSessions())
    args = {
        "source_file": str(file), "review_case_id": handoff["cases"][0]["case_id"],
        "review_recipe": inspected["recipe"], "expected_review_fingerprint": inspected["review_fingerprint"],
    }
    staged = daemon._document_import_stage_payload(ctx, args)
    assert staged["transaction_id"] == "in"
    assert conn.execute("SELECT transaction_id FROM attachments WHERE id=?", (staged["attachment_id"],)).fetchone()[0] == "in"
    managed = Path(ctx.document_import_sessions.source_for_preview(
        staged["document_token"], workspace_id="ws", profile_id="profile", data_root=runtime.data_root,
    ))
    file.write_bytes(b"replaced source")
    assert managed.read_bytes() == b"original source statement"
    assert str(managed) not in str(staged)
    # A second staged upload cannot reuse the now-stale evidence fingerprint.
    with pytest.raises(AppError) as raised:
        daemon._document_import_stage_payload(ctx, args)
    assert raised.value.code == "source_funds_review_stale"
    assert conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0] == 1


def test_planned_sale_recipe_is_valid_for_both_ai_preview_and_save():
    recipe = {"target_transaction": "in", "report_purpose": "planned_exchange_sale",
              "planned_destination": "Recipient", "planned_note": "Same reviewed purpose"}
    for name in ("ui.source_funds.preview", "ui.source_funds.cases.save", "ui.source_funds.review_context"):
        daemon._validate_ai_tool_arguments(get_tool(name), recipe)
    assert "source_funds" in select_tool_capabilities([{"role": "user", "content": "Bitte prüfe meine Mittelherkunft"}])
    args = {"model": "test", "tools_enabled": True, "messages": [{"role": "user", "content": "Mittelherkunft"}]}
    assert daemon._ai_chat_args(args)["tool_loop_max_iterations"] == 16
    assert daemon._ai_chat_args({**args, "tool_loop_max_iterations": 3})["tool_loop_max_iterations"] == 3


@pytest.mark.parametrize("change", [
    {"review_recipe": {"target_amount": "0.00000001"}},
    {"expected_review_fingerprint": "0" * 64},
    {"review_case_id": "source_funds:unknown"},
])
def test_stale_or_unknown_source_funds_file_target_never_writes(book, tmp_path, change):
    conn, runtime = book
    inspected = context(conn)
    file = tmp_path / "source.png"
    file.write_bytes(b"proof")
    args = {
        "source_file": str(file), "review_case_id": inspected["case_id"],
        "review_recipe": inspected["recipe"], "expected_review_fingerprint": inspected["review_fingerprint"],
        **change,
    }
    ctx = SimpleNamespace(conn=conn, data_root=runtime.data_root, document_import_sessions=daemon.DocumentImportSessions())
    with patch.object(daemon.core_attachments, "add_attachment") as attach:
        with pytest.raises(AppError):
            daemon._document_import_stage_payload(ctx, args)
        attach.assert_not_called()


def test_source_funds_stage_needs_both_recipe_and_fingerprint_in_current_book(book, tmp_path):
    conn, runtime = book
    inspected = context(conn)
    file = tmp_path / "source.png"
    file.write_bytes(b"proof")
    args = {"source_file": str(file), "review_case_id": inspected["case_id"],
            "review_recipe": inspected["recipe"], "expected_review_fingerprint": inspected["review_fingerprint"]}
    ctx = SimpleNamespace(conn=conn, data_root=runtime.data_root, document_import_sessions=daemon.DocumentImportSessions())
    for field in ("review_recipe", "expected_review_fingerprint", "review_case_id"):
        with pytest.raises(AppError):
            daemon._document_import_stage_payload(ctx, {key: value for key, value in args.items() if key != field})
    conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) SELECT 'other',workspace_id,'Other',created_at FROM profiles WHERE id='profile'")
    set_setting(conn, "context_profile", "other")
    conn.commit()
    with pytest.raises(AppError):
        daemon._document_import_stage_payload(ctx, args)
    assert conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0] == 0


def test_scope_truncation_is_explicit_and_cannot_authorize_a_handoff(book):
    conn, _runtime = book
    seed_source(conn)
    with patch.object(source_funds_review, "MAX_CONTEXT_RECORDS", 0):
        inspected = context(conn)
        assert inspected["scope_truncated"] is True
        with pytest.raises(AppError, match="truncated"):
            request(conn, inspected)


def test_ai_context_and_handoff_use_same_fingerprint_without_private_paths(book, tmp_path):
    conn, runtime = book
    file = tmp_path / "private-proof.png"
    file.write_bytes(b"proof")
    daemon.core_attachments.add_attachment(
        conn, runtime.data_root, "ws", "profile", "in", daemon._attachment_hooks(), file_path=str(file),
    )
    runtime.maintenance_state["provider_kind"] = "openai"
    with patch.object(daemon, "_ai_tool_is_advertised", return_value=True):
        result = daemon._execute_read_only_ai_tool(
            daemon.ParsedAiToolCall("context", "ui.source_funds.review_context", {"target_transaction": "in"}), runtime,
        )
        assert result["ok"] is True
        inspected = result["envelope"]["data"]
        assert runtime.data_root not in str(inspected)
        assert "stored_relpath" not in str(inspected)
        handoff = daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall(
            "input", "ui.source_funds.request_input", {
                "action": "attach_evidence", "target_transaction": "in", "recipe": inspected["recipe"],
                "expected_review_fingerprint": inspected["review_fingerprint"],
            }), runtime)
        assert handoff["ok"] is True
        assert handoff["envelope"]["data"]["review_fingerprint"] == inspected["review_fingerprint"]


def test_cli_context_and_input_share_canonical_recipe_and_target(book, tmp_path):
    import json

    _conn, runtime = book
    inspected, status = run_cli(runtime.data_root, "source-funds", "review-context", "--target-transaction", "in")
    assert status == 0, inspected
    assert inspected["kind"] == "source-funds.review-context"
    recipe = tmp_path / "context.json"
    recipe.write_text(json.dumps(inspected))
    handoff, status = run_cli(runtime.data_root, "source-funds", "request-input", "--target-transaction", "in",
        "--action", "import_history", "--recipe-file", str(recipe), "--expected-review-fingerprint", inspected["data"]["review_fingerprint"])
    assert status == 0, handoff
    assert handoff["kind"] == "source-funds.request-input"
    assert handoff["data"]["cases"][0]["case_id"] == "source_funds:in"


@pytest.mark.parametrize("private_recipe", [
    {"planned_destination": "https://exchange.example/deposit"},
    {"planned_note": "Read /Users/alice/private/proof.pdf"},
])
def test_private_recipe_rejected_before_unusable_ai_fingerprint_is_emitted(book, private_recipe):
    conn, runtime = book
    # Local UI/CLI retains the complete recipe; only the outbound AI packet is rejected.
    local = context(conn, **private_recipe)
    assert all(local["recipe"][key] == value for key, value in private_recipe.items())
    runtime.maintenance_state["provider_kind"] = "openai"
    with patch.object(daemon, "_ai_tool_is_advertised", return_value=True):
        result = daemon._execute_read_only_ai_tool(daemon.ParsedAiToolCall(
            "private", "ui.source_funds.review_context", {"target_transaction": "in", **private_recipe},
        ), runtime)
    assert result["ok"] is False
    assert "source_funds_recipe_private" in str(result)
    assert "review_fingerprint" not in str(result)
    assert all(value not in str(result) for value in private_recipe.values())
    assert request(conn, local)["review_fingerprint"] == local["review_fingerprint"]


def test_redacted_handoff_explanation_keeps_replayable_request_id(book):
    conn, runtime = book
    inspected = context(conn)
    args = {"action": "attach_evidence", "target_transaction": "in", "recipe": inspected["recipe"],
            "expected_review_fingerprint": inspected["review_fingerprint"], "explanation": "Read /Users/alice/proof.pdf"}
    with patch.object(daemon, "_ai_tool_is_advertised", return_value=True):
        result = daemon._execute_read_only_ai_tool(
            daemon.ParsedAiToolCall("redact", "ui.source_funds.request_input", args), runtime,
        )
    assert result["ok"] is True
    packet = result["envelope"]["data"]
    repeated = daemon._ui_source_funds_payload_from_conn(conn, "ui.source_funds.request_input", {
        **args, "explanation": packet["explanation"],
    })
    assert repeated["request_id"] == packet["request_id"]
    assert "/Users/alice" not in packet["explanation"]


def test_snapshot_save_rejects_intervening_provenance_edit_without_journal_change(book):
    conn, _runtime = book
    source, _link = seed_source(conn)
    original = context(conn, report_purpose="planned_exchange_sale", planned_destination="Reviewed recipient",
                       planned_note="Exact purpose", target_amount="0.00000050")
    conn.execute("UPDATE source_funds_sources SET description='Changed documentary facts' WHERE id=?", (source["id"],))
    conn.commit()
    arguments = {"target_transaction": "in", **original["recipe"],
                 "expected_review_fingerprint": original["review_fingerprint"], "case_label": "Inspected case"}
    with pytest.raises(AppError) as raised:
        daemon._ui_source_funds_payload_from_conn(conn, "ui.source_funds.cases.save", arguments)
    assert raised.value.code == "source_funds_review_stale"
    assert conn.execute("SELECT COUNT(*) FROM source_funds_cases").fetchone()[0] == 0
    assert conn.execute("SELECT COUNT(*) FROM source_funds_snapshots").fetchone()[0] == 0
    assert not conn.in_transaction
    fresh = context(conn, **original["recipe"])
    assert fresh["input_version"] == original["input_version"]
    arguments["expected_review_fingerprint"] = fresh["review_fingerprint"]
    saved = daemon._ui_source_funds_payload_from_conn(conn, "ui.source_funds.cases.save", arguments)
    assert saved["purpose"] == fresh["report"]["purpose"]
    assert saved["allocations"] == fresh["report"]["allocations"]
    assert saved["reveal_mode"] == fresh["report"]["reveal_mode"]
    assert saved["case"]["id"]
    assert conn.execute("SELECT COUNT(*) FROM source_funds_snapshots").fetchone()[0] == 1
    assert not conn.in_transaction


def test_safe_recipe_fits_typed_continuation_screen_context(book):
    conn, _runtime = book
    inspected = context(conn)
    screen = {
        "route": "/source-of-funds", "entity_type": "transaction", "entity_id": "in",
        "filters": {"source_funds_recipe": inspected["recipe"]}, "capabilities": ["source_funds"],
    }
    assert daemon._ai_chat_screen_context(screen) == screen


def test_oversized_recipe_is_rejected_for_ai_but_available_to_local_workflow(book):
    conn, runtime = book
    args = {"target_transaction": "in", "planned_note": "n" * 4000}
    assert context(conn, planned_note=args["planned_note"])["recipe"]["planned_note"] == args["planned_note"]
    with patch.object(daemon, "_ai_tool_is_advertised", return_value=True):
        result = daemon._execute_read_only_ai_tool(
            daemon.ParsedAiToolCall("large", "ui.source_funds.review_context", args), runtime,
        )
    assert result["ok"] is False
    assert "source_funds_recipe_too_large" in str(result)
    assert "review_fingerprint" not in str(result)
