"""Opt-in terminal adapter for existing one-turn selected-context grants.

No provider implementation, tool dispatch, history or new grant authority lives
here. One daemon owns disclosure, result validation and application until exit.
"""
from __future__ import annotations

import hashlib
import json
import re
import uuid
from pathlib import Path

from ..errors import AppError

MAX_SELECTION_BYTES = 64 * 1024


def _fail(message, code="accounting_ai_context_invalid"):
    raise AppError(message, code=code, retryable=False)


def _request(args, stdin, stdout):
    if not stdin.isatty() or not stdout.isatty() or getattr(args, "non_interactive", False):
        _fail("Selected accounting assistance requires an interactive terminal", "interaction_required")
    forbidden = ("prompt", "prompt_text", "file", "file_context", "system", "temperature", "max_tokens",
        "tool_loop_max_iterations", "yes", "allow_tool", "stream_json", "transcript", "continue_session",
        "session", "output", "chat_attachment")
    if any(getattr(args, key, None) is not None and getattr(args, key) is not False for key in forbidden):
        _fail("Selected accounting assistance cannot combine prompts, files, history, transcripts, output files, tool policies or generation overrides")
    if getattr(args, "format", None) not in (None, "table", "plain") or getattr(args, "tool_profile", "core") != "core":
        _fail("Selected accounting assistance is a fresh tool-free terminal turn")
    digest = getattr(args, "accounting_selection_sha256", None)
    if not isinstance(digest, str) or not re.fullmatch(r"[a-f0-9]{64}", digest):
        _fail("Provide the lowercase SHA-256 of the exact selection file bytes", "accounting_payload_digest_required")
    try:
        path = Path(args.accounting_selection).expanduser()
        if not path.is_file():
            _fail("Selection must be an existing local file", "accounting_input_unavailable")
        with path.open("rb") as source:
            raw = source.read(MAX_SELECTION_BYTES + 1)
    except OSError as exc:
        raise AppError("Could not read the selected accounting request", code="accounting_input_unavailable") from exc
    if len(raw) > MAX_SELECTION_BYTES:
        _fail("Selection metadata exceeds 64 KiB", "accounting_payload_too_large")
    if hashlib.sha256(raw).hexdigest() != digest:
        _fail("Selection file changed after approval", "accounting_stale_approval")
    def unique_fields(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("Duplicate field")
            result[key] = value
        return result
    try:
        request = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_fields)
    except (ValueError, UnicodeError, RecursionError) as exc:
        raise AppError("Selection file must contain one valid UTF-8 JSON object", code="accounting_invalid_fields") from exc
    if (not isinstance(request, dict) or set(request) != {"profile_id", "question", "purpose", "selection"}
        or not isinstance(request["profile_id"], str) or not request["profile_id"] or len(request["profile_id"]) > 200
        or not isinstance(request["question"], str) or not request["question"] or len(request["question"]) > 4000
        or not isinstance(request["purpose"], str) or not isinstance(request["selection"], dict)):
        _fail("Selection requires profile_id, question, purpose and selection", "accounting_invalid_fields")
    return request


def _show(out, title, value):
    # JSON quotes every user/model-controlled string and escapes terminal/bidi
    # controls. Never render raw model markdown on this sensitive surface.
    out.write(title + "\n" + json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n")
    out.flush()


def _approve(stdin, out, question):
    while True:
        out.write(question + " [y] once, [n] deny: ")
        out.flush()
        answer = stdin.readline().strip().lower()
        if answer in ("y", "yes"):
            return True
        if answer in ("", "n", "no", "deny"):
            return False
        out.write("Only a fresh once-only decision is accepted.\n")


def _read(client, request_id):
    from .chat import _auth_required_error
    while True:
        record = client.read(record=False)
        if record.get("request_id") != request_id:
            continue
        if record.get("kind") == "auth_required":
            raise _auth_required_error(record)
        if record.get("kind") == "error":
            # Provider/domain failures can quote selected documents. Keep this
            # sensitive terminal adapter categorical, including outer CLI errors.
            error = record.get("error")
            code = error.get("code") if isinstance(error, dict) else None
            if not isinstance(code, str) or not re.fullmatch(r"[a-z_]{1,80}", code):
                code = "accounting_ai_failed"
            raise AppError("Selected accounting assistance failed; review the local request before retrying",
                           code=code, retryable=False)
        return record


def _call(client, kind, payload):
    request_id = "accounting-assist-" + uuid.uuid4().hex
    client.send({"request_id": request_id, "kind": kind, "args": payload}, record=False)
    record = _read(client, request_id)
    if record.get("kind") != kind or not isinstance(record.get("data"), dict):
        _fail("Unexpected selected-assistance response", "daemon_protocol_error")
    return record["data"]


def run(args, *, stdin, stdout):
    from .chat import _DaemonChatClient, _resolve_default_model, _timeout_seconds, ChatSessionResult, ChatTurnResult
    request = _request(args, stdin, stdout)  # Fail before opening daemon/transcript/provider.
    client = _DaemonChatClient(args, transcript=None)
    active_request = None
    try:
        _resolve_default_model(client, args)
        scoped = lambda payload: {"profile_id": request["profile_id"], "payload": payload}
        preview = _call(client, "ui.accounting.ai_preview", scoped({
            "provider": getattr(args, "provider", None), "model": args.model,
            **{key: request[key] for key in ("selection", "question", "purpose")}}))
        provider, context = preview.get("provider"), preview.get("context")
        if (not isinstance(provider, dict) or not isinstance(provider.get("name"), str)
            or provider.get("model") != args.model
            or (getattr(args, "provider", None) is not None and provider["name"] != args.provider)
            or not isinstance(context, dict) or not isinstance(context.get("question"), str)
            or not isinstance(preview.get("token"), str)
            or not isinstance(preview.get("expected_digest"), str)
            or not re.fullmatch(r"[a-f0-9]{64}", preview["expected_digest"])):
            _fail("A complete destination-bound disclosure preview is unavailable")
        _show(stdout, "SELECTED FINANCIAL DISCLOSURE — sensitive, not anonymized; not saved to chat history", {
            "profile_id": request["profile_id"], "provider": provider, "context": context,
            "expected_digest": preview["expected_digest"], "context_bytes": preview.get("context_bytes"),
            "notice": preview.get("notice")})
        if not _approve(stdin, stdout, "Disclose exactly this selected context to the shown provider?"):
            _call(client, "ui.accounting.ai_cancel", scoped({"token": preview["token"]}))
            stdout.write("Disclosure denied. No model request or bookkeeping change was made.\n")
            return ChatSessionResult(provider=provider["name"], model=args.model)
        active_request = "accounting-assist-chat-" + uuid.uuid4().hex
        options = {} if getattr(args, "reasoning_effort", "auto") == "auto" else {"reasoning_effort": args.reasoning_effort}
        client.send({"request_id": active_request, "kind": "ai.chat", "args": {
            "provider": provider["name"], "model": args.model, "tools_enabled": False,
            "messages": [{"role": "user", "content": context["question"]}], "persist": False, "session_id": None,
            "timeout_seconds": _timeout_seconds(args), "options": options,
            "accounting_context": {"profile_id": request["profile_id"], "token": preview["token"],
                "expected_digest": preview["expected_digest"], "confirm": True}}}, record=False)
        parts, size = [], 0
        while True:
            record = _read(client, active_request)
            if record["kind"] == "ai.chat":
                break
            if record["kind"] not in ("ai.chat.status", "ai.chat.delta"):
                _fail("Unexpected tool event in a tool-free accounting turn", "daemon_protocol_error")
            data = record.get("data", {})
            delta = data.get("delta", {}) if isinstance(data, dict) else {}
            content = delta.get("content") if isinstance(delta, dict) else None
            if isinstance(content, str):
                size += len(content.encode("utf-8"))
                if size > 1024 * 1024:
                    _fail("Selected response exceeded its budget", "accounting_ai_output_limit")
                parts.append(content)
        active_request = None
        content = "".join(parts)
        _show(stdout, "MODEL SUGGESTION — untrusted; no bookkeeping change made", content)
        terminal = record.get("data", {})
        token = terminal.get("accounting_result_token") if isinstance(terminal, dict) else None
        if token is not None and terminal.get("finish_reason") != "cancelled":
            if not isinstance(token, str):
                _fail("Invalid suggestion result token", "daemon_protocol_error")
            result = _call(client, "ui.accounting.ai_result_preview", scoped({"result_token": token}))
            if (result.get("effect") != "create_drafts_and_review_document_fields_not_post"
                or not isinstance(result.get("plan"), dict) or not isinstance(result.get("expected_digest"), str)
                or not re.fullmatch(r"[a-f0-9]{64}", result["expected_digest"])):
                _fail("A complete local candidate preview is unavailable")
            _show(stdout, "LOCAL CANDIDATE REVIEW — separate from disclosure approval; never posts entries", result)
            if _approve(stdin, stdout, "Create only these drafts and reviewed document fields?"):
                applied = _call(client, "ui.accounting.ai_result_apply", scoped({"result_token": token,
                    "expected_digest": result["expected_digest"], "confirm": True,
                    "reason": "Explicit once-only terminal review of the exact selected-context candidate"}))
                _show(stdout, "LOCAL APPLICATION RECEIPT", applied)
            else:
                stdout.write("Candidate denied. No drafts or document fields were applied.\n")
        return ChatSessionResult(provider=provider["name"], model=args.model,
            turns=[ChatTurnResult(content=content, terminal=record)])
    finally:
        try:
            if active_request is not None:
                client.send({"request_id": "accounting-cancel-" + uuid.uuid4().hex, "kind": "ai.chat.cancel",
                    "args": {"target_request_id": active_request}}, record=False)
        finally:
            client.close()  # Drops all unconsumed disclosure/result tokens from RAM.
