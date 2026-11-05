"""Python reference for the Rust daemon's ``test.ai_tool`` and ``test.ai_auto_read``.

Runs the real CLI (``kassiber ... daemon``) with ``handle_request`` wrapped so
the two test-harness kinds reach the executor exactly as a chat turn does:
database work is queued to the daemon's main-thread task queue and drained
there (``_drain_daemon_main_thread_tasks``), each result is recorded for
provenance and redacted, and the model-visible bytes are the sorted compact
JSON of the redacted result. Every other request goes to the unchanged
daemon. With ``KASSIBER_RUST_TEST_JOURNAL_TIME`` set, journal clocks are bound
as ``tests/rust_port/journal_cli_reference.py`` binds them.
"""
from __future__ import annotations

from contextlib import ExitStack
from datetime import datetime
import itertools
import json
import os
from pathlib import Path
import sys
import threading
from unittest.mock import patch
import uuid

TEST_KINDS = ("test.ai_tool", "test.ai_auto_read")


class _DrainingQueue:
    """``main_thread_tasks`` for a call made on the main loop itself."""

    def __init__(self, ctx, daemon):
        self._ctx = ctx
        self._daemon = daemon

    def put(self, task):
        self._ctx.main_thread_tasks.put(task)
        self._daemon._drain_daemon_main_thread_tasks(self._ctx)


def _runtime(daemon, ctx, args):
    # A chat turn starts on an open book; a fresh daemon opens plaintext
    # books lazily, as the Rust host does.
    if ctx.conn is None:
        daemon._open_daemon_connection(ctx)
    state = args.get("state") or {}
    if not isinstance(state, dict):
        raise daemon.AppError("state must be an object", code="validation")
    data_root = args.get("runtime_data_root")
    runtime = daemon.AiToolRuntime(
        data_root=data_root if isinstance(data_root, str) else ctx.data_root,
        runtime_config=ctx.runtime_config,
        main_thread_tasks=_DrainingQueue(ctx, daemon),
        maintenance_state=state,
    )
    return runtime, state


def _ai_tool(daemon, ctx, request):
    from kassiber.ai.tools import get_tool, redact_ai_tool_result
    from kassiber.envelope import build_envelope, json_ready

    args = request.get("args") or {}
    runtime, state = _runtime(daemon, ctx, args)
    if "raw_call" in args:
        call = daemon._parse_ai_tool_call(args["raw_call"], 0)
    else:
        call = daemon.ParsedAiToolCall(
            call_id=args.get("call_id") or "call_0",
            name=args.get("name") if isinstance(args.get("name"), str) else "",
            arguments=args.get("arguments", {}),
        )
    result = daemon._execute_read_only_ai_tool(
        call, runtime, planned_auto_read=args.get("planned_auto_read") is True
    )
    entry = get_tool(call.name)
    daemon._record_ai_tool_usage(runtime, entry.name if entry is not None else call.name, result)
    safe_result = redact_ai_tool_result(result)
    data = {
        "tool_result": build_envelope("ai.chat.tool_result", {"call_id": call.call_id, **safe_result}),
        "output": json.dumps(json_ready(redact_ai_tool_result(result)), sort_keys=True, separators=(",", ":")),
        "state": state,
    }
    return daemon._with_request_id(build_envelope("test.ai_tool", data), request.get("request_id"))


def _ai_auto_read(daemon, ctx, request, out):
    from kassiber.envelope import build_envelope

    args = request.get("args") or {}
    runtime, state = _runtime(daemon, ctx, args)
    messages = list(args.get("messages") or [])
    validated = {"system_prompt_kind": args.get("system_prompt_kind"), "messages": list(messages)}
    daemon._run_auto_read_tools(
        request_id=request.get("request_id"),
        messages=messages,
        validated=validated,
        out=out,
        runtime=runtime,
        cancel_event=threading.Event(),
    )
    planned = [[call.name, call.arguments] for call in daemon._planned_auto_read_tools(validated)]
    data = {"planned": planned, "messages": messages, "state": state}
    return daemon._with_request_id(build_envelope("test.ai_auto_read", data), request.get("request_id"))


def _bind_journal_clock(stack):
    clock = os.environ.get("KASSIBER_RUST_TEST_JOURNAL_TIME")
    if not clock:
        return
    for target in (
        "kassiber.core.custody_journal.now_iso",
        "kassiber.core.custody_journal.custody_build_clock",
        "kassiber.core.custody_filed_reports.now_iso",
        "kassiber.core.filed_report_chain.now_iso",
        "kassiber.core.custody_components._now_iso",
        "kassiber.core.custody_review_terms.now_iso",
        "kassiber.core.custody_authored_migration.now_iso",
        "kassiber.cli.handlers.now_iso",
        "kassiber.daemon.now_iso",
    ):
        stack.enter_context(patch(target, return_value=clock))

    class Frozen(datetime):
        @classmethod
        def now(cls, tz=None):
            return datetime.fromisoformat(clock.replace("Z", "+00:00"))

    stack.enter_context(patch("kassiber.core.transfer_matching.datetime", Frozen))
    if os.environ.get("KASSIBER_RUST_TEST_JOURNAL_IDS") == "sequential":
        from kassiber.core import custody_journal

        counter = itertools.count(1)

        class DurableUUID(uuid.UUID):
            def __str__(self):
                return str(uuid.UUID(int=next(counter), version=4))

        original_build = custody_journal.build_ledger_state
        original_uuid = uuid.uuid4

        def build(*args, **kwargs):
            with patch.object(uuid, "uuid4", original_uuid):
                return original_build(*args, **kwargs)

        stack.enter_context(patch.object(uuid, "uuid4", lambda: DurableUUID(int=0, version=4)))
        stack.enter_context(patch.object(custody_journal, "build_ledger_state", build))


def main() -> int:
    from kassiber import daemon
    from kassiber.cli.main import main as cli
    from kassiber.operator import project

    original = daemon.handle_request

    def handle_request(ctx, request, out):
        kind = request.get("kind") if isinstance(request, dict) else None
        if kind == "test.ai_tool":
            return _ai_tool(daemon, ctx, request), False
        if kind == "test.ai_auto_read":
            return _ai_auto_read(daemon, ctx, request, out), False
        return original(ctx, request, out)

    with ExitStack() as stack:
        if owner := os.environ.get("KASSIBER_RUST_TEST_OWNER_ROOT"):
            stack.enter_context(patch.object(project, "_owner_lock_root", lambda: Path(owner)))
        stack.enter_context(patch.object(daemon, "handle_request", handle_request))
        _bind_journal_clock(stack)
        return cli(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())
