"""Selected CLI disclosure reuses real grants and commands; provider transport is synthetic."""
import copy
import hashlib
import io
import json
from types import SimpleNamespace

import pytest

from kassiber import daemon, daemon_accounting_ai as bridge
from kassiber.cli import accounting_assist, chat
from kassiber.core.accounting import ledger
from kassiber.errors import AppError
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_ai_result_tokens import result  # noqa: F401


class Tty(io.StringIO):
    def isatty(self):
        return True


@pytest.fixture
def selected(result, tmp_path, monkeypatch):
    ctx, profile_id, provider, disclosed, candidate, _ = result
    ctx.accounting_ai_grants.clear()
    request = {key: disclosed["binding"][key] for key in ("profile_id", "question", "purpose", "selection")}
    path = tmp_path / "selection.json"
    raw = json.dumps(request).encode()
    path.write_bytes(raw)
    args = SimpleNamespace(accounting_selection=str(path), accounting_selection_sha256=hashlib.sha256(raw).hexdigest(),
        provider=provider["name"], model=disclosed["binding"]["provider"]["model"], data_root=str(ctx.data_root))

    class Client:
        def __init__(self):
            self.events, self.sent, self.model_requests = [], [], []
            self.closed = False
            self.before_chat = self.before_apply = None
            self.content = json.dumps(candidate)
        def send(self, event, *, record=True):
            assert record is False, "Selected financial requests must never enter transcripts"
            self.sent.append(copy.deepcopy(event))
            kind, payload = event["kind"], event["args"]
            if kind == "ai.chat":
                if self.before_chat:
                    self.before_chat()
                validated = daemon._ai_chat_args(payload)
                grant = bridge.prepare(ctx, validated, provider)
                self.model_requests.append(copy.deepcopy(validated))
                token = bridge.buffer_result(ctx, grant, self.content)
                self.events.extend([
                    {"request_id": event["request_id"], "kind": "ai.chat.delta", "data": {"delta": {"content": self.content}}},
                    {"request_id": event["request_id"], "kind": "ai.chat", "data": {"finish_reason": "stop", "accounting_result_token": token}},
                ])
            elif kind == "ai.chat.cancel":
                pass
            else:
                if kind == "ui.accounting.ai_result_apply" and self.before_apply:
                    self.before_apply()
                answer = bridge.dispatch(ctx, kind, payload)
                self.events.append({"request_id": event["request_id"], "kind": kind, "data": answer})
        def read(self, *, record=True):
            assert record is False
            return self.events.pop(0)
        def close(self):
            self.closed = True
            ctx.accounting_ai_grants.clear()
    client = Client()
    def start(_args, *, transcript):
        assert transcript is None
        return client
    monkeypatch.setattr(chat, "_DaemonChatClient", start)
    return args, client, ctx, profile_id, path


def run(selected, answer="y\ny\n"):
    args, _, _, _, _ = selected
    output = Tty()
    session = chat.run_chat_command(args, stdin=Tty(answer), stdout=output)
    return session, output.getvalue()


def test_exact_disclosure_then_separate_review_creates_only_drafts_and_fields(selected):
    _, client, ctx, profile_id, _ = selected
    session, output = run(selected, "s\ny\ns\ny\n")
    assert len(session.turns) == 1 and len(client.model_requests) == 1
    request = client.model_requests[0]
    assert request["tools_enabled"] is False and request["persist"] is False and request["session_id"] is None
    assert request["seed_history"] is False and request["attachment"] is None
    assert "Invoice 42 Total EUR 10.00" in request["messages"][0]["content"]
    assert "Invoice 42 Total EUR 10.00" in output
    assert "SELECTED FINANCIAL DISCLOSURE" in output and "LOCAL CANDIDATE REVIEW" in output
    assert "LOCAL APPLICATION RECEIPT" in output
    assert '"posted": false' in output
    assert output.count("Only a fresh once-only decision") == 2
    assert [row[0] for row in ctx.conn.execute("SELECT status FROM gl_entries")] == ["draft"]
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_evidence_field_reviews").fetchone()[0] == 1
    assert ctx.conn.execute("SELECT COUNT(*) FROM ai_chat_messages").fetchone()[0] == 0
    assert client.closed and not ctx.accounting_ai_grants._pending and not ctx.accounting_ai_grants._results
    assert ledger.require_book(ctx.conn, profile_id)


def test_disclosure_deny_never_calls_model_or_mutates(selected):
    _, client, ctx, _, _ = selected
    session, output = run(selected, "n\n")
    assert session.turns == [] and client.model_requests == []
    assert [call["kind"] for call in client.sent] == ["ui.accounting.ai_preview", "ui.accounting.ai_cancel"]
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_entries").fetchone()[0] == 0
    assert "Disclosure denied" in output and client.closed


def test_result_deny_does_not_convert_disclosure_into_mutation_consent(selected):
    _, client, ctx, _, _ = selected
    _, output = run(selected, "y\nn\n")
    assert len(client.model_requests) == 1
    assert "Candidate denied" in output
    assert "ui.accounting.ai_result_apply" not in [call["kind"] for call in client.sent]
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_entries").fetchone()[0] == 0
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_evidence_field_reviews").fetchone()[0] == 0


@pytest.mark.parametrize("flag,value", [("yes", True), ("allow_tool", ["ui.accounting.ai_result_apply"]),
    ("transcript", "forbidden.ndjson"), ("stream_json", True), ("format", "json"), ("output", "forbidden.json"),
    ("session", "old-session"), ("continue_session", True), ("file", "other.pdf"), ("system", "custom"),
    ("prompt", "different question"), ("temperature", 0), ("max_tokens", 100), ("tool_profile", "full")])
def test_unsafe_option_combinations_fail_before_daemon_or_file_output(selected, flag, value):
    args, client, _, _, path = selected
    setattr(args, flag, value)
    with pytest.raises(AppError) as error:
        run(selected)
    assert error.value.code == "accounting_ai_context_invalid"
    assert client.sent == [] and not client.closed
    assert not (path.parent / "forbidden.ndjson").exists()


@pytest.mark.parametrize("input_tty,output_tty,noninteractive", [(False, True, False), (True, False, False), (True, True, True)])
def test_noninteractive_and_redirected_output_deny_before_daemon(selected, input_tty, output_tty, noninteractive):
    args, client, _, _, _ = selected
    args.non_interactive = noninteractive
    with pytest.raises(AppError) as error:
        chat.run_chat_command(args, stdin=(Tty if input_tty else io.StringIO)("y\n"), stdout=(Tty if output_tty else io.StringIO)())
    assert error.value.code == "interaction_required" and client.sent == []


@pytest.mark.parametrize("mutation", ["hash", "changed_file", "oversized", "duplicate", "extra_field", "utf16"])
def test_selection_is_bounded_exact_and_hash_bound(selected, mutation):
    args, client, _, _, path = selected
    if mutation == "hash":
        args.accounting_selection_sha256 = None
    elif mutation == "changed_file":
        path.write_bytes(path.read_bytes() + b" ")
    elif mutation == "utf16":
        raw = path.read_text().encode("utf-16")
        path.write_bytes(raw)
        args.accounting_selection_sha256 = hashlib.sha256(raw).hexdigest()
    else:
        raw = b" " * (accounting_assist.MAX_SELECTION_BYTES + 1) if mutation == "oversized" else b'{"selection":{},"selection":{}}' if mutation == "duplicate" else b'{"extra":true}'
        path.write_bytes(raw)
        args.accounting_selection_sha256 = hashlib.sha256(raw).hexdigest()
    with pytest.raises(AppError):
        run(selected)
    assert client.sent == []


@pytest.mark.parametrize("when", ["before_chat", "before_apply"])
def test_existing_daemon_rejects_stale_book_before_disclosure_or_apply(selected, when):
    _, client, ctx, profile_id, _ = selected
    setattr(client, when, lambda: ledger.create_account(ctx.conn, profile_id, code="changed", name="changed", kind="expense"))
    with pytest.raises(AppError) as error:
        run(selected)
    assert error.value.code == "accounting_stale_approval"
    assert len(client.model_requests) == (0 if when == "before_chat" else 1)
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_entries").fetchone()[0] == 0
    assert client.closed


def test_plain_explanation_is_displayed_escaped_without_apply(selected):
    _, client, ctx, _, _ = selected
    client.content = "\x1b[2J\u202eUntrusted model explanation"
    _, output = run(selected, "y\n")
    assert "\x1b" not in output and "\u202e" not in output
    assert r"\u001b" in output and r"\u202e" in output
    assert "ui.accounting.ai_result_preview" not in [call["kind"] for call in client.sent]
    assert ctx.conn.execute("SELECT COUNT(*) FROM gl_entries").fetchone()[0] == 0


def test_cross_book_selection_refused_by_existing_daemon_scope(selected):
    args, client, _, _, path = selected
    request = json.loads(path.read_text())
    request["profile_id"] = "other"
    raw = json.dumps(request).encode()
    path.write_bytes(raw)
    args.accounting_selection_sha256 = hashlib.sha256(raw).hexdigest()
    with pytest.raises(AppError) as error:
        run(selected)
    assert error.value.code == "accounting_scope_changed" and client.model_requests == []


@pytest.mark.parametrize("code", ["accounting_stale_approval", "\x1b[2Jprivate-data"])
def test_sensitive_daemon_errors_never_relay_financial_text_or_terminal_codes(code):
    event = {"request_id": "request", "kind": "error", "error": {
        "code": code, "message": "\x1b[2JPRIVATE-INVOICE", "details": {"amount_minor": "999999"},
    }}
    client = SimpleNamespace(read=lambda **_: event)
    with pytest.raises(AppError) as error:
        accounting_assist._read(client, "request")
    assert "PRIVATE-INVOICE" not in str(error.value)
    assert "\x1b" not in str(error.value) + error.value.code
    assert not error.value.details
    assert error.value.code == (code if code == "accounting_stale_approval" else "accounting_ai_failed")


def test_selection_file_identity_and_digest_are_excluded_from_public_diagnostics():
    from kassiber.diagnostics import _argument_summary
    args = SimpleNamespace(accounting_selection="private-board-salary-selection.json",
                           accounting_selection_sha256="b" * 64)
    summary = _argument_summary(args)
    assert {row["value_class"] for row in summary} == {"redacted"}
    assert "private-board" not in json.dumps(summary) and "b" * 64 not in json.dumps(summary)


def test_selected_mode_accepts_real_parser_defaults(selected):
    from kassiber.cli.main import build_parser
    args, _, _, _, path = selected
    parsed = build_parser().parse_args(["chat", "--accounting-selection", str(path),
        "--accounting-selection-sha256", args.accounting_selection_sha256])
    assert accounting_assist._request(parsed, Tty(), Tty())["profile_id"]
