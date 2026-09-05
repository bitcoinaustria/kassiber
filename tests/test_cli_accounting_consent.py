"""CLI accounting approval uses local daemon effects, not model arguments."""
import copy
import io
import json
import queue
from types import SimpleNamespace

import pytest

from kassiber.cli import accounting_consent, chat
from kassiber.core.accounting import commands, tasks
from tests.test_accounting_integration import book  # noqa: F401
from tests.test_accounting_tasks import setup, rule


class Tty(io.StringIO):
    def isatty(self):
        return True


@pytest.fixture
def consent_data(book):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id, 3)
    rule(conn, profile_id)
    preview = commands.wire_values(tasks.execute(conn, profile_id, "task-preview", {"task_id": task["id"], "step": "prepare"}))
    return {"call_id": "accounting-1", "name": "ui.accounting.task_apply", "needs_consent": True,
        "arguments_preview": {"task_id": task["id"], "approval_id": "opaque", "idempotency_key": "apply"},
        "accounting_task_preview": {"status": "ready", "step": "prepare", "preview": preview,
            "book": {"currency": "EUR", "minor_unit_exponent": 2}}}


def run_consent(data, *, answer="y\n", tty=True, **options):
    class Client:
        sent = None
        def send(self, value):
            self.sent = value
    client = Client()
    output = io.StringIO()
    session = {data["name"]}
    chat._decide_and_send_consent(client, SimpleNamespace(**options), request_id="request", call_id=data["call_id"],
        name=data["name"], data=data, stdin=(Tty if tty else io.StringIO)(answer), chrome=output,
        session_allowed=session, control_requests=set())
    return client.sent, output.getvalue(), session


def test_exact_local_preview_and_once_only_even_with_all_blanket_policies(consent_data):
    sent, output, _ = run_consent(consent_data, answer="s\ny\n", yes=True, allow_tool=["ui_accounting_task_apply"])
    assert sent["args"]["decision"] == "allow_once"
    assert "Session-wide approval is not available" in output
    assert "Membership" in output
    assert consent_data["accounting_task_preview"]["preview"]["expected_digest"] in output
    assert "debit_minor" in output and "credit_minor" in output
    assert "not sent to the model" in output


@pytest.mark.parametrize("options", [{"yes": True}, {"allow_tool": ["ui.accounting.task_apply"]},
    {"format": "json", "yes": True}, {"stream_json": True, "yes": True}, {"non_interactive": True, "yes": True}])
def test_noninteractive_denies_without_financial_output(consent_data, options):
    sent, output, _ = run_consent(consent_data, tty=False, **options)
    assert sent["args"]["decision"] == "deny"
    assert "interactive, once-only" in output
    assert "Membership" not in output


def test_explicit_deny_and_cancel(consent_data):
    assert run_consent(consent_data, answer="n\n")[0]["args"]["decision"] == "deny"
    assert run_consent(consent_data, answer="c\n")[0]["kind"] == "ai.chat.cancel"


@pytest.mark.parametrize("mutation", ["spoof", "task", "digest", "revision", "blocked", "missing_effects", "book", "step"])
def test_incomplete_or_spoofed_preview_cannot_be_approved(consent_data, mutation):
    value = copy.deepcopy(consent_data)
    preview = value["accounting_task_preview"]["preview"]
    if mutation == "spoof":
        value["arguments_preview"]["accounting_task_preview"] = value.pop("accounting_task_preview")
    elif mutation == "task":
        preview["id"] = "f" * 32
    elif mutation == "digest":
        preview["expected_digest"] = "no"
    elif mutation == "revision":
        preview["expected_revision"] = True
    elif mutation == "blocked":
        preview["blockers"] = [{"kind": "blocked"}]
    elif mutation == "missing_effects":
        del preview["proposals"]
    elif mutation == "step":
        preview["step"] = []
    else:
        value["accounting_task_preview"]["book"]["minor_unit_exponent"] = -1
    sent, output, _ = run_consent(value, yes=True)
    assert sent["args"]["decision"] == "deny"
    assert "Membership" not in output


def test_terminal_control_sequences_are_escaped(consent_data):
    consent_data["accounting_task_preview"]["preview"]["proposals"][0]["payload"]["description"] = "\x1b[2J\u202eFORGED"
    _, output, _ = run_consent(consent_data)
    assert "\x1b" not in output and "\u202e" not in output
    assert r"\u001b" in output and r"\u202e" in output


def test_transcript_strips_local_preview_but_live_record_retains_it(consent_data):
    client = chat._DaemonChatClient.__new__(chat._DaemonChatClient)
    client._read_timeout_seconds = 1
    client._stdout_queue = queue.Queue()
    client._transcript = io.StringIO()
    event = {"kind": "ai.chat.tool_consent_required", "request_id": "request", "data": consent_data}
    client._stdout_queue.put(json.dumps(event))
    live = client.read()
    assert live == event
    saved = json.loads(client._transcript.getvalue())
    assert "accounting_task_preview" not in saved["data"]
    assert "Membership" not in client._transcript.getvalue()
    assert saved["data"]["arguments_preview"] == event["data"]["arguments_preview"]


def test_stream_waits_for_daemon_consent_event_and_keeps_result_clean(consent_data):
    class Client:
        def __init__(self):
            self.index, self.sent = 0, []
            call = {k: v for k, v in consent_data.items() if k != "accounting_task_preview"}
            call["arguments"] = call.pop("arguments_preview")
            self.events = [{"kind": "ai.chat.tool_call", "data": call},
                {"kind": "ai.chat.tool_consent_required", "data": consent_data},
                {"kind": "ai.chat", "data": {"finish_reason": "stop"}}]
        def send(self, record):
            if record["kind"] == "ai.chat":
                self.request_id = record["request_id"]
            else:
                assert self.index == 2, "Must wait for local daemon preview"
                self.sent.append(record)
        def read(self):
            event = self.events[self.index]
            self.index += 1
            return dict(event, request_id=self.request_id)
    client = Client()
    args = SimpleNamespace(model="fake", yes=True)
    output, raw = io.StringIO(), io.StringIO()
    result = chat._run_turn(client, args, [], stdin=Tty("y\n"), out=output, chrome=output, render=False, stream_out=raw)
    assert len(client.sent) == 1
    assert client.sent[0]["args"]["decision"] == "allow_once"
    assert "Membership" not in json.dumps(result.tool_calls)
    assert "Membership" in raw.getvalue()  # Explicit raw stream is not a transcript.


def test_repl_cannot_preapprove_accounting():
    allowed, output = set(), io.StringIO()
    chat._handle_allow_command("ui_accounting_task_apply", allowed, output)
    assert not allowed
    assert "fresh once-only review" in output.getvalue()


def test_cancel_task_is_once_only_without_financial_preview(consent_data):
    consent_data["name"] = "ui.accounting.task_cancel"
    consent_data.pop("accounting_task_preview")
    sent, output, _ = run_consent(consent_data, yes=True, answer="s\ny\n")
    assert sent["args"]["decision"] == "allow_once"
    assert "Already committed entries remain unchanged" in output


def test_real_post_preview_includes_every_entry(book):
    conn, profile_id, _ = book
    task, _ = setup(conn, profile_id, 3)
    rule(conn, profile_id)
    preview = tasks.execute(conn, profile_id, "task-preview", {"task_id": task["id"], "step": "prepare"})
    tasks.execute(conn, profile_id, "task-apply", {"task_id": task["id"], "step": "prepare",
        "expected_digest": preview["expected_digest"], "expected_revision": preview["expected_revision"],
        "idempotency_key": "prepare", "confirmed": True})
    posted = commands.wire_values(tasks.execute(conn, profile_id, "task-preview", {"task_id": task["id"], "step": "post"}))
    value = {"status": "ready", "step": "post", "preview": posted, "book": {"currency": "EUR", "minor_unit_exponent": 2}}
    assert accounting_consent._valid_preview(value, task["id"])
    assert len(posted["detail"]["entries"]) == 3


@pytest.mark.parametrize("step,detail", [
    ("close", {"period_id": "period", "ready": True, "blockers": [],
        "trial_balance": {"rows": [], "balanced": True, "debit_minor": "0", "credit_minor": "0"},
        "statements": {"profit_and_loss": [], "balance_sheet": [], "balanced": True}}),
    ("tax_finalize", {"forms": [{"form_id": "K2", "fields": {"amount": "123456789"}}]}),
    ("export_close", {"id": "artifact", "snapshot_digest": "a" * 64}),
    ("export_tax", {"final_id": "final", "report_digest": "b" * 64}),
])
def test_each_financial_step_requires_its_complete_specific_preview(consent_data, step, detail):
    value = consent_data["accounting_task_preview"]
    value["step"] = value["preview"]["step"] = step
    value["preview"]["period_id"] = "period"
    value["preview"]["detail"] = detail
    sent, output, _ = run_consent(consent_data)
    assert sent["args"]["decision"] == "allow_once"
    assert json.dumps(detail, indent=2, sort_keys=True).splitlines()[1].strip() in output
    value["preview"]["detail"] = {}
    assert run_consent(consent_data)[0]["args"]["decision"] == "deny"
