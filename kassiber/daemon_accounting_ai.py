"""Desktop selected-context grants, separate from the general AI tool catalog."""
from .ai.providers import resolve_ai_provider, require_ai_provider_acknowledged
from .core.accounting.ai_context import DisclosureGrants
from .core.accounting import ledger
from .core.repo import resolve_scope
from .errors import AppError

KINDS = ("ui.accounting.ai_preview", "ui.accounting.ai_cancel", "ui.accounting.ai_result_preview", "ui.accounting.ai_result_apply")


def provider_binding(provider, model):
    return {"name": provider["name"], "kind": provider["kind"], "model": model,
        "configuration_digest": ledger.digest({key: provider.get(key) for key in
            ("name", "base_url", "kind", "updated_at", "api_key", "acknowledged_at")})}


def scope_binding(ctx):
    workspace, profile = resolve_scope(ctx.conn)
    return {"project": ledger.digest(str(ctx.data_root)), "workspace_id": workspace["id"],
        "profile_id": profile["id"], "ownership_generation": ctx.ownership_generation}


def dispatch(ctx, kind, args):
    if not isinstance(args, dict) or set(args) != {"profile_id", "payload"} or not isinstance(args["payload"], dict):
        raise AppError("Invalid selected-context request", code="accounting_invalid_fields")
    scope = scope_binding(ctx)
    if args["profile_id"] != scope["profile_id"]:
        raise AppError("The selected book changed", code="accounting_scope_changed")
    ledger.require_book(ctx.conn, scope["profile_id"])
    payload = args["payload"]
    if kind in {"ui.accounting.ai_result_preview", "ui.accounting.ai_result_apply"}:
        return _result_action(ctx, scope, kind, payload)
    if kind == "ui.accounting.ai_cancel":
        if set(payload) != {"token"} or not isinstance(payload["token"], str):
            raise AppError("Invalid disclosure token", code="accounting_invalid_fields")
        ctx.accounting_ai_grants.cancel(payload["token"])
        return {"cancelled": True}
    if set(payload) != {"provider", "model", "selection", "question", "purpose"} or not isinstance(payload["model"], str) or not payload["model"].strip() or len(payload["model"]) > 256:
        raise AppError("Invalid selected-context preview", code="accounting_invalid_fields")
    provider = resolve_ai_provider(ctx.conn, payload["provider"])
    return ctx.accounting_ai_grants.preview(ctx.conn, scope["profile_id"],
        selection=payload["selection"], question=payload["question"], purpose=payload["purpose"],
        provider_binding=provider_binding(provider, payload["model"]), scope_binding=scope)


def buffer_result(ctx, disclosed, content):
    """Record only structured candidates in bounded RAM, not chat history."""
    import secrets
    import time
    from .core.accounting.ai_proposals import decode_result
    recheck(ctx, disclosed)
    candidate = decode_result(content)
    if candidate is None:
        return None
    grants = ctx.accounting_ai_grants
    now = time.monotonic()
    grants._results = {key: row for key, row in grants._results.items() if row['expires'] > now}
    if len(grants._results) >= 16:
        oldest = min(grants._results, key=lambda key: grants._results[key]['expires'])
        grants._results.pop(oldest)
    token = secrets.token_urlsafe(32)
    grants._results[token] = {'candidate': candidate, 'disclosed': disclosed,
        'expires': now + grants.lifetime_seconds}
    # Do not retain the selected plaintext prompt in a result receipt.
    grants._results[token]['disclosed'] = {'binding': disclosed['binding']}
    return token


def _result_action(ctx, scope, kind, payload):
    import time
    from .core.accounting import ai_proposals
    allowed = {'result_token'} if kind.endswith('_preview') else {'result_token','expected_digest','confirm','reason'}
    if set(payload) != allowed or not isinstance(payload.get('result_token'), str):
        raise AppError('Invalid AI result approval fields', code='accounting_invalid_fields')
    result = ctx.accounting_ai_grants._results.get(payload['result_token'])
    if result is None or result['expires'] <= time.monotonic():
        ctx.accounting_ai_grants._results.pop(payload['result_token'], None)
        raise AppError('AI result expired; request a new suggestion', code='accounting_ai_grant_expired')
    recheck(ctx, result['disclosed'])
    if kind.endswith('_preview'):
        return ai_proposals.preview(ctx.conn, scope['profile_id'], candidate=result['candidate'], binding=result['disclosed']['binding'])
    if payload['confirm'] is not True:
        raise AppError('Creating drafts and reviewing fields requires explicit approval', code='accounting_ai_consent_required')
    try:
        applied = ai_proposals.apply(ctx.conn, scope['profile_id'], candidate=result['candidate'],
            binding=result['disclosed']['binding'], expected_digest=payload['expected_digest'], reason=payload['reason'])
        ctx.conn.commit()
    except Exception:
        ctx.conn.rollback()
        raise
    ctx.accounting_ai_grants._results.pop(payload['result_token'])
    return applied


def prepare(ctx, validated, provider):
    raw = validated["accounting_context"]
    if not isinstance(raw, dict) or set(raw) != {"profile_id", "token", "expected_digest", "confirm"}:
        raise AppError("Invalid selected-context approval", code="accounting_invalid_fields")
    # A disclosure is exactly one question, not a grant to append hidden chat
    # history, files, a custom system prompt or an unrestricted tools loop.
    if (validated["tools_enabled"] or validated["session_id"] is not None
        or validated["persist"] not in (False, None) or validated["attachment"] is not None
        or validated["screen_context"] is not None or validated["system_prompt"] is not None
        or validated["seed_history"] or set(validated["options"]) - {"reasoning_effort"}
        or len(validated["messages"]) != 1 or set(validated["messages"][0]) != {"role", "content"}
        or validated["messages"][0]["role"] != "user"):
        raise AppError("Selected accounting context requires a fresh tool-free turn", code="accounting_ai_context_invalid")
    scope = scope_binding(ctx)
    if raw["profile_id"] != scope["profile_id"]:
        raise AppError("The selected book changed", code="accounting_scope_changed")
    require_ai_provider_acknowledged(provider)
    disclosed = ctx.accounting_ai_grants.consume(ctx.conn, scope["profile_id"],
        token=raw["token"], expected_digest=raw["expected_digest"], confirm=raw["confirm"],
        provider_binding=provider_binding(provider, validated["model"]), scope_binding=scope)
    if validated["messages"][0]["content"] != disclosed["binding"]["question"]:
        raise AppError("The approved question changed", code="accounting_stale_approval")
    validated.update(messages=disclosed["messages"], system_prompt=disclosed["system_prompt"],
        system_prompt_kind="raw", persist=False, session_id=None,
        timeout_seconds=min(validated["timeout_seconds"], 300),
        options={**validated["options"], "sensitive_context": True})
    return disclosed


def recheck(ctx, disclosed):
    if ctx.conn is None:
        raise AppError("The book was locked", code="passphrase_required")
    binding = disclosed["binding"]
    scope = scope_binding(ctx)
    if scope != binding["scope"]:
        raise AppError("Disclosure scope changed", code="accounting_stale_approval")
    book = ledger.require_book(ctx.conn, scope["profile_id"])
    provider = resolve_ai_provider(ctx.conn, binding["provider"]["name"])
    require_ai_provider_acknowledged(provider)
    if book["revision"] != binding["book_revision"] or provider_binding(provider, binding["provider"]["model"]) != binding["provider"]:
        raise AppError("Disclosure context or destination changed", code="accounting_stale_approval")
