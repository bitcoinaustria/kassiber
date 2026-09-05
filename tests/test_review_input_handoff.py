"""Missing-input handoffs identify current cases without granting custody authority."""
from types import SimpleNamespace
from unittest.mock import Mock, patch

import pytest

from kassiber import daemon
from kassiber.ai.tools import get_tool, select_tool_capabilities
from kassiber.core import review_workflow
from kassiber.errors import AppError
from tests.test_daemon_review_workflow import book  # noqa: F401


def request_args(conn):
    page = daemon._review_workflow_payload(conn, "ui.review.cases", {})
    return {
        "action": "import_history",
        "case_ids": [case["case_id"] for case in page["cases"]],
        "expected_input_version": page["input_version"],
        "explanation": "Supply the earlier acquisition history for these movements.",
    }


def test_handoff_is_canonical_stable_and_read_only(book):
    conn, _runtime = book
    args = request_args(conn)
    before = list(conn.iterdump())
    packet = daemon._review_workflow_payload(conn, "ui.review.request_input", args)
    assert packet == daemon._review_workflow_payload(conn, "ui.review.request_input", args)
    assert len(packet["request_id"]) == 64
    assert packet["workspace_id"] == "ws"
    assert packet["profile_id"] == "profile"
    assert {case["wallet_id"] for case in packet["cases"]} == {"source-wallet", "sink-wallet"}
    assert all(set(case) == {"case_id", "transaction_id", "wallet_id", "direction", "asset", "occurred_at", "reason"}
               for case in packet["cases"])
    assert list(conn.iterdump()) == before
    assert not conn.in_transaction


@pytest.mark.parametrize("change", [
    {"case_ids": ["quarantine:foreign"]},
    {"case_ids": ["quarantine:in", "quarantine:in"]},
    {"case_ids": []},
    {"case_ids": ["gap:private-graph"]},
    {"expected_input_version": -1},
    {"expected_input_version": True},
    {"action": "exclude"},
    {"explanation": "x" * 1001},
    {"explanation": "control\x00text"},
])
def test_handoff_rejects_invalid_requests_without_writes(book, change):
    conn, _runtime = book
    args = {**request_args(conn), **change}
    before = list(conn.iterdump())
    with pytest.raises(AppError):
        daemon._review_workflow_payload(conn, "ui.review.request_input", args)
    assert list(conn.iterdump()) == before
    assert not conn.in_transaction


def test_handoff_rejects_stale_version_and_already_resolved_case(book):
    conn, _runtime = book
    args = request_args(conn)
    daemon.invalidate_journals(conn, "profile")
    conn.commit()
    with pytest.raises(AppError) as raised:
        daemon._review_workflow_payload(conn, "ui.review.request_input", args)
    assert raised.value.code == "custody_review_plan_stale"
    args = request_args(conn)
    conn.execute("UPDATE transactions SET excluded=1 WHERE id='in'")
    conn.commit()
    # A fresh canonical rebuild also protects callers if journal versions lag.
    with pytest.raises(AppError) as raised:
        daemon._review_workflow_payload(conn, "ui.review.request_input", args)
    assert raised.value.code == "review_case_stale"


def test_remote_handoff_has_no_private_gap_or_location_payload(book):
    conn, runtime = book
    runtime.maintenance_state["provider_kind"] = "openai"
    args = request_args(conn)
    args["explanation"] = "Read /Users/alice/private/export.csv"
    call = daemon.ParsedAiToolCall("handoff", "ui.review.request_input", args)
    with patch.object(daemon, "_ai_tool_is_advertised", return_value=True):
        result = daemon._execute_read_only_ai_tool(call, runtime)
    assert result["ok"] is True
    packet = result["envelope"]["data"]
    assert packet["explanation"] == "Read <redacted-path>"
    assert "raw_json" not in str(packet)
    assert packet["request_id"] == review_workflow._digest({key: value for key, value in packet.items() if key != "request_id"})
    gap = daemon._execute_read_only_ai_tool(
        daemon.ParsedAiToolCall("gap", "ui.custody.gaps.list", {}), runtime,
    )
    assert gap["ok"] is False
    assert "local_provider_required" in str(gap)


@pytest.mark.parametrize("kind,handler", [
    ("ui.backends.bitcoinrpc.test", "_test_bitcoinrpc_backend_payload"),
    ("ui.wallets.create", "_create_wallet_payload"),
])
def test_expected_scope_prevents_setup_before_egress_or_write(book, kind, handler):
    conn, _runtime = book
    ctx = SimpleNamespace(conn=conn)
    request = {"request_id": "setup", "kind": kind, "args": {
        "expected_scope": {"workspace_id": "ws", "profile_id": "different-book"},
    }}
    with patch.object(daemon, handler) as operation:
        with pytest.raises(AppError) as raised:
            daemon.handle_request(ctx, request, Mock())
        assert raised.value.code == "stale_context"
        operation.assert_not_called()
        request["args"]["expected_scope"]["profile_id"] = "profile"
        operation.return_value = {"ok": True}
        envelope, _ = daemon.handle_request(ctx, request, Mock())
        assert envelope["data"] == {"ok": True}
        assert operation.call_args.args[-1] == {}
    assert "expected_scope" in request["args"]  # Caller packet remains immutable.


def test_handoff_schema_is_in_review_pack_and_rejects_extra_authority():
    assert "review" in select_tool_capabilities([{"role": "user", "content": "Investigate quarantine"}])
    tool = get_tool("ui.review.request_input")
    assert tool.kind_class == "read_only"
    assert not tool.egresses
    arguments = {"action": "connect_wallet", "case_ids": ["quarantine:in"], "expected_input_version": 0}
    daemon._validate_ai_tool_arguments(tool, arguments)
    with pytest.raises(AppError):
        daemon._validate_ai_tool_arguments(tool, {**arguments, "raw_graph": {"private": True}})


@pytest.mark.parametrize("selector", [
    {"workspace": "ws", "profile": "other"},
    {"workspace": "Main", "profile": "Other book"},
    {"profile_id": "other"},
])
def test_expected_scope_rejects_redirected_same_database_mutation(book, selector):
    conn, _runtime = book
    conn.execute(
        "INSERT INTO profiles(id,workspace_id,label,created_at) "
        "SELECT 'other',workspace_id,'Other book',created_at FROM profiles WHERE id='profile'",
    )
    conn.commit()
    args = {
        "expected_scope": {"workspace_id": "ws", "profile_id": "profile"},
        "surface": "swap_candidates", "name": "review handoff", **selector,
    }
    with pytest.raises(AppError) as raised:
        daemon.handle_request(SimpleNamespace(conn=conn), {
            "request_id": "redirect", "kind": "ui.saved_views.create", "args": args,
        }, Mock())
    assert raised.value.code == "stale_context"
    assert conn.execute("SELECT COUNT(*) FROM saved_views").fetchone()[0] == 0
    args.pop("profile_id", None)
    args.update(workspace="Main", profile="Book")
    daemon.handle_request(SimpleNamespace(conn=conn), {
        "request_id": "same-book", "kind": "ui.saved_views.create", "args": args,
    }, Mock())
    assert conn.execute("SELECT profile_id FROM saved_views").fetchone()[0] == "profile"


@pytest.mark.parametrize("scope", [None, {}, {"workspace_id": "ws"},
    {"workspace_id": "ws", "profile_id": "profile", "path": "/private"},
    {"workspace_id": "ws", "profile_id": 1},
])
def test_expected_scope_cannot_be_malformed(book, scope):
    conn, _runtime = book
    with patch.object(daemon, "_test_bitcoinrpc_backend_payload") as probe:
        with pytest.raises(AppError) as raised:
            daemon.handle_request(SimpleNamespace(conn=conn), {
                "request_id": "bad", "kind": "ui.backends.bitcoinrpc.test",
                "args": {"expected_scope": scope},
            }, Mock())
        assert raised.value.code == "validation"
        probe.assert_not_called()


def staging_context(book):
    conn, runtime = book
    return SimpleNamespace(
        conn=conn, data_root=runtime.data_root,
        document_import_sessions=daemon.DocumentImportSessions(),
    )


def test_review_attachment_is_durable_and_token_uses_identical_managed_bytes(book, tmp_path):
    from hashlib import sha256
    from pathlib import Path

    ctx = staging_context(book)
    source = tmp_path / "receipt.png"
    original = b"original verified receipt bytes"
    source.write_bytes(original)
    args = {
        "source_file": str(source), "review_case_id": "quarantine:in",
        "expected_scope": {"workspace_id": "ws", "profile_id": "profile"},
    }
    envelope, _ = daemon.handle_request(ctx, {
        "kind": "internal.document_import.stage", "request_id": "pick", "args": args,
    }, Mock())
    staged = envelope["data"]
    row = ctx.conn.execute("SELECT * FROM attachments WHERE id=?", (staged["attachment_id"],)).fetchone()
    assert row["transaction_id"] == staged["transaction_id"] == "in"
    assert row["profile_id"] == "profile"
    assert row["sha256"] == sha256(original).hexdigest()
    assert staged["source"]["filename"] == "receipt.png"
    managed = Path(ctx.document_import_sessions.source_for_preview(
        staged["document_token"], workspace_id="ws", profile_id="profile", data_root=ctx.data_root,
    ))
    assert managed != source
    assert managed.read_bytes() == original
    source.write_bytes(b"the original selected file was replaced")
    assert managed.read_bytes() == original
    assert str(source) not in str(staged)
    assert str(managed) not in str(staged)
    assert "stored_relpath" not in str(staged)
    assert row["sha256"] not in str(staged)
    public = daemon.redact_ai_tool_result(staged)
    assert public["document_token"] == "<redacted>"
    assert public["attachment_id"] == staged["attachment_id"]
    assert public["transaction_id"] == "in"
    assert ctx.conn.execute("SELECT COUNT(*) FROM review_workflow_receipts").fetchone()[0] == 0
    # The token retains the same profile restriction as ordinary chat attachments.
    with pytest.raises(AppError):
        ctx.document_import_sessions.source_for_preview(
            staged["document_token"], workspace_id="ws", profile_id="other", data_root=ctx.data_root,
        )


@pytest.mark.parametrize("case_id", ["quarantine:unknown", "gap:private", "", None])
def test_invalid_review_attachment_target_copies_nothing(book, tmp_path, case_id):
    ctx = staging_context(book)
    source = tmp_path / "receipt.png"
    source.write_bytes(b"evidence")
    with patch.object(daemon.core_attachments, "add_attachment") as attach:
        with pytest.raises(AppError):
            daemon._document_import_stage_payload(ctx, {
                "source_file": str(source), "review_case_id": case_id,
            })
        attach.assert_not_called()
    assert ctx.conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0] == 0


def test_review_attachment_cannot_retarget_case_in_different_active_profile(book, tmp_path):
    from kassiber.db import set_setting

    ctx = staging_context(book)
    source = tmp_path / "receipt.png"
    source.write_bytes(b"evidence")
    ctx.conn.execute(
        "INSERT INTO profiles(id,workspace_id,label,created_at) "
        "SELECT 'other',workspace_id,'Other',created_at FROM profiles WHERE id='profile'",
    )
    set_setting(ctx.conn, "context_profile", "other")
    ctx.conn.commit()
    with pytest.raises(AppError) as raised:
        daemon._document_import_stage_payload(ctx, {
            "source_file": str(source), "review_case_id": "quarantine:in",
        })
    assert raised.value.code == "review_case_stale"
    assert ctx.conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0] == 0


def test_attachment_handoff_requires_single_audit_target(book):
    conn, _runtime = book
    args = {**request_args(conn), "action": "attach_evidence"}
    with pytest.raises(AppError, match="exactly one"):
        daemon._review_workflow_payload(conn, "ui.review.request_input", args)
    args["case_ids"] = args["case_ids"][:1]
    assert len(daemon._review_workflow_payload(conn, "ui.review.request_input", args)["cases"]) == 1


def test_cold_daemon_opens_plaintext_book_before_handoff_scope_check(book):
    import json
    import os
    import subprocess
    import sys

    _conn, runtime = book
    request = {
        "request_id": "cold-scope", "kind": "ui.review.cases",
        "args": {"expected_scope": {"workspace_id": "ws", "profile_id": "profile"}},
    }
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--data-root", runtime.data_root, "daemon"],
        input=json.dumps(request) + '\n' + json.dumps({"request_id": "end", "kind": "daemon.shutdown"}) + '\n',
        text=True, capture_output=True, timeout=30, check=True,
        env={**os.environ, "KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR": "1",
             "KASSIBER_OPERATOR_RUNTIME_DIR": runtime.data_root + "/operator"},
    )
    responses = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    response = next(item for item in responses if item.get("request_id") == "cold-scope")
    assert response["kind"] == "ui.review.cases", response
    assert response["data"]["profile_id"] == "profile"


def test_cold_scope_validation_precedes_setup_egress_and_keeps_unlock_boundary(book):
    conn, _runtime = book
    ctx = SimpleNamespace(conn=None)
    request = {
        "kind": "ui.backends.bitcoinrpc.test", "request_id": "cold-probe",
        "args": {"expected_scope": {"workspace_id": "ws", "profile_id": "other"}},
    }

    def open_plaintext(context):
        context.conn = conn
        return conn

    with patch.object(daemon, "_open_daemon_connection", side_effect=open_plaintext) as opener:
        with patch.object(daemon, "_test_bitcoinrpc_backend_payload") as probe:
            with pytest.raises(AppError) as raised:
                daemon.handle_request(ctx, request, Mock())
            assert raised.value.code == "stale_context"
            opener.assert_called_once_with(ctx)
            probe.assert_not_called()

    ctx.conn = None
    with patch.object(daemon, "_open_daemon_connection") as opener:
        with pytest.raises(AppError) as raised:
            daemon.handle_request(ctx, {**request, "args": {"expected_scope": {}}}, Mock())
        assert raised.value.code == "validation"
        opener.assert_not_called()

    with patch.object(daemon, "_open_daemon_connection", side_effect=AppError("Encrypted", code="passphrase_required")):
        with patch.object(daemon, "_test_bitcoinrpc_backend_payload") as probe:
            envelope, stopped = daemon.handle_request(ctx, request, Mock())
            assert envelope["kind"] == "auth_required"
            assert envelope["data"]["scope"] == "unlock_database"
            assert stopped is False
            assert ctx.conn is None
            probe.assert_not_called()
