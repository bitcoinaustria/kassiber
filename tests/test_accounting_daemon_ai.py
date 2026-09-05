from types import SimpleNamespace

import pytest

from kassiber import daemon, daemon_accounting_ai as bridge
from kassiber.ai.client import OpenAIResponsesClient
from kassiber.core.accounting import document_text, evidence, ledger
from kassiber.core.accounting.ai_context import DisclosureGrants
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401


@pytest.fixture
def prepared(book, monkeypatch):
    conn, profile, root = book
    source = evidence.retain_evidence(conn, profile, content=b"Selected accounting evidence", name="Selected", media_type="text/plain")
    extraction = document_text.extract(conn, profile, evidence_id=source["id"])
    provider = {"name": "Local", "kind": "local", "base_url": "http://127.0.0.1:9999/v1", "updated_at": "1"}
    monkeypatch.setattr(bridge, "resolve_ai_provider", lambda conn, name: provider)
    ctx = SimpleNamespace(conn=conn, data_root=str(root), ownership_generation="test-generation",
        accounting_ai_grants=DisclosureGrants(), active_ai_chats=daemon.ActiveAiChats(), db_passphrase=None)
    payload = {"provider": "Local", "model": "test-model", "question": "Explain this document",
        "purpose": "document_fields", "selection": {"extractions": [{"id": extraction["id"], "pages": [1], "fields": []}]}}
    preview = bridge.dispatch(ctx, "ui.accounting.ai_preview", {"profile_id": profile, "payload": payload})
    args = {"provider": "Local", "model": "test-model", "messages": [{"role": "user", "content": payload["question"]}],
        "accounting_context": {"profile_id": profile, "token": preview["token"], "expected_digest": preview["expected_digest"], "confirm": True}}
    return ctx, provider, args


def test_selected_context_preparation_never_enables_tools_or_history(prepared):
    ctx, provider, args = prepared
    validated = daemon._ai_chat_args(args)
    receipt = bridge.prepare(ctx, validated, provider)
    assert validated["persist"] is False and validated["session_id"] is None
    assert validated["tools_enabled"] is False
    assert validated["options"] == {"sensitive_context": True}
    assert "Selected accounting evidence" in validated["messages"][0]["content"]
    bridge.recheck(ctx, receipt)
    provider["base_url"] = "https://changed.example/v1"
    with pytest.raises(AppError):
        bridge.recheck(ctx, receipt)


@pytest.mark.parametrize("extra", [{"tools_enabled": True}, {"persist": True}, {"session_id": "old"},
    {"options": {"provider_session_id": "old"}}, {"system_prompt_kind": "raw", "system_prompt": "bypass"},
    {"messages": [{"role": "user", "content": "Additional unapproved question"}]},
    {"messages": [{"role": "user", "content": "Explain this document"}, {"role": "assistant", "content": "old context"}]}])
def test_selected_context_rejects_hidden_expansion(prepared, extra):
    ctx, provider, args = prepared
    with pytest.raises(AppError):
        bridge.prepare(ctx, daemon._ai_chat_args(dict(args, **extra)), provider)


def test_lock_clears_grants_and_cancels_sensitive_chat_only(prepared):
    ctx, provider, args = prepared
    receipt = bridge.prepare(ctx, daemon._ai_chat_args(args), provider)
    _, sensitive = ctx.active_ai_chats.register("sensitive")
    _, ordinary = ctx.active_ai_chats.register("ordinary")
    sensitive.accounting_guard = lambda: bridge.recheck(ctx, receipt)
    daemon._clear_unlocked_passphrase(ctx)
    assert sensitive.cancel_event.is_set()
    assert not ordinary.cancel_event.is_set()
    assert ctx.accounting_ai_grants._pending == {}


def test_book_change_revokes_active_sensitive_chat(prepared):
    ctx, provider, args = prepared
    receipt = bridge.prepare(ctx, daemon._ai_chat_args(args), provider)
    _, chat = ctx.active_ai_chats.register("sensitive")
    chat.accounting_guard = lambda: bridge.recheck(ctx, receipt)
    ledger.create_account(ctx.conn, receipt["binding"]["profile_id"], code="new", name="New", kind="expense")
    ctx.active_ai_chats.validate_accounting_scopes()
    assert chat.cancel_event.is_set()


def test_http_sensitive_flag_never_reaches_provider_payload():
    client = OpenAIResponsesClient("http://127.0.0.1:9999/v1")
    body = client._request_body(messages=[{"role": "user", "content": "Selected"}], context=None,
        model="test", stream=True, options={"sensitive_context": True, "store": True}, tools=None, tool_choice=None)
    assert "sensitive_context" not in body
    assert body["store"] is False


def test_cancel_before_http_open_never_egresses(monkeypatch):
    client = OpenAIResponsesClient("http://127.0.0.1:9999/v1")
    monkeypatch.setattr("urllib.request.urlopen", lambda *a, **kw: pytest.fail("cancelled request contacted provider"))
    client.cancel()
    with pytest.raises(AppError) as exc:
        list(client.stream_chat(messages=[{"role": "user", "content": "Selected"}], model="test"))
    assert exc.value.code == "ai_cancelled"
