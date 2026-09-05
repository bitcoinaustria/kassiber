"""The terminal must not turn blanket policies into exact-plan approvals."""
from __future__ import annotations

import copy
import io
from types import SimpleNamespace

import pytest

from kassiber.cli import chat, review_consent
from kassiber import daemon
from tests.test_daemon_review_workflow import book, _price_plan  # noqa: F401


class Terminal(io.StringIO):
    def isatty(self):
        return True


class Client:
    def __init__(self, records=()):
        self.records = iter(records)
        self.sent = []

    def send(self, record):
        self.sent.append(record)

    def read(self):
        return next(self.records)


@pytest.fixture
def consent_data(book):
    conn, runtime = book
    arguments = {"artifact": _price_plan(conn), "idempotency_key": "cli-review"}
    return {
        "name": review_consent.TOOL_NAME, "arguments_preview": arguments,
        "review_preview": daemon._ai_review_consent_preview(runtime, arguments),
    }


def decide(data, stdin, *, session_allowed=None, **overrides):
    args = SimpleNamespace(yes=True, allow_tool=[review_consent.TOOL_NAME], format="table",
                           non_interactive=False, stream_json=False)
    for key, value in overrides.items():
        setattr(args, key, value)
    client, out = Client(), io.StringIO()
    result = chat._decide_and_send_consent(
        client, args, request_id="chat", call_id="call", name=review_consent.TOOL_NAME,
        data=data, stdin=stdin, chrome=out, session_allowed=session_allowed,
        control_requests=set(),
    )
    return result, out.getvalue(), client.sent


def test_repeated_proposals_ignore_blanket_and_prior_session_approvals(consent_data):
    session = {review_consent.TOOL_NAME}
    answers = Terminal("s\ny\nn\n")
    first, text, sent = decide(consent_data, answers, session_allowed=session)
    assert first == "allow_once"
    assert sent[0]["args"]["decision"] == "allow_once"
    assert '"before"' in text and '"after"' in text and '"operations"' in text
    assert "100000" in text and "User supplied the verified acquisition rate" in text
    assert "[s]" not in text
    second, _, sent = decide(consent_data, answers, session_allowed=session)
    assert second == "deny"
    assert sent[0]["args"]["decision"] == "deny"


@pytest.mark.parametrize("options", [{"format": "json"}, {"stream_json": True}, {"non_interactive": True}, {}])
def test_machine_or_non_tty_deny_even_with_blanket_flags(consent_data, options):
    stdin = Terminal("y\n") if options else io.StringIO("y\n")
    result, text, _ = decide(consent_data, stdin, **options)
    assert result == "deny"
    assert stdin.tell() == 0
    assert "Daemon-validated review" not in text


@pytest.mark.parametrize("variant", ["missing", "nested", "unavailable", "different", "malformed"])
def test_missing_forged_or_unavailable_preview_never_prompts(consent_data, variant):
    data = copy.deepcopy(consent_data)
    if variant == "missing":
        data.pop("review_preview")
    elif variant == "nested":
        data["arguments_preview"]["review_preview"] = data.pop("review_preview")
    elif variant == "unavailable":
        data["review_preview"] = {"status": "unavailable", "code": "review_artifact_invalid"}
    elif variant == "different":
        data["arguments_preview"]["artifact"]["after"]["quarantine_count"] = 1234
    else:
        # Even identical malformed objects cannot be used as a display contract.
        data["review_preview"]["artifact"] = data["arguments_preview"]["artifact"] = {"digest": "a" * 64}
    stdin = Terminal("y\n")
    result, _, _ = decide(data, stdin)
    assert result == "deny"
    assert stdin.tell() == 0


def test_real_historical_retry_receipt_requires_own_approval(consent_data, book):
    conn, runtime = book
    arguments = consent_data["arguments_preview"]
    receipt = daemon._review_workflow_payload(conn, review_consent.TOOL_NAME, arguments, authored_source="ai_tool")
    data = {**consent_data, "review_preview": daemon._ai_review_consent_preview(runtime, arguments)}
    assert data["review_preview"]["status"] == "applied"
    result, text, _ = decide(data, Terminal("y\n"))
    assert result == "allow_once" and receipt["id"] in text
    data["arguments_preview"] = {**arguments, "idempotency_key": "different"}
    assert decide(data, Terminal("y\n"))[0] == "deny"


def test_stream_waits_for_separate_daemon_preview(consent_data):
    arguments = consent_data["arguments_preview"]
    records = [
        {"request_id": "chat", "kind": "ai.chat.tool_call", "data": {
            "call_id": "call", "name": review_consent.TOOL_NAME,
            "needs_consent": True, "arguments": arguments,
        }},
        {"request_id": "chat", "kind": "ai.chat.tool_consent_required", "data": {
            **consent_data, "call_id": "call",
        }},
        {"request_id": "chat", "kind": "ai.chat", "data": {}},
    ]
    class CheckedClient(Client):
        def read(self):
            record = super().read()
            if record["kind"] == "ai.chat.tool_consent_required":
                assert not self.sent, "Approval was sent before the validated preview"
            return record
    client = CheckedClient(records)
    chat._stream_turn_records(
        client, SimpleNamespace(yes=True), "chat", [], {}, set(),
        stdin=Terminal("y\n"), out=io.StringIO(), chrome=io.StringIO(),
        render=False, stream_out=None, session_allowed={review_consent.TOOL_NAME}, markdown=None,
    )
    assert len(client.sent) == 1
    assert client.sent[0]["args"]["decision"] == "allow_once"


def test_allow_command_and_status_do_not_claim_review_is_preapproved():
    session, out = set(), io.StringIO()
    chat._handle_allow_command(review_consent.TOOL_NAME, session, out)
    assert not session
    assert "cannot be pre-allowed" in out.getvalue()
    out = io.StringIO()
    chat._render_allowed(SimpleNamespace(yes=True), session, out)
    assert "except ui.review.apply" in out.getvalue()


@pytest.mark.parametrize("answer,expected", [("n\n", "deny"), ("", "deny"), ("c\n", "cancel")])
def test_deny_eof_and_cancel_remain_available(consent_data, answer, expected):
    result, _, sent = decide(consent_data, Terminal(answer))
    assert result == expected
    assert sent[0]["kind"] == ("ai.chat.cancel" if expected == "cancel" else "ai.tool_call.consent")
