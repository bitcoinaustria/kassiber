"""AI provider configuration and OpenAI-compatible chat client.

Two layers:

- `kassiber.ai.providers` — SQLite-backed CRUD for AI provider records,
  mirroring the shape of `kassiber.backends`. Stores `name`, `base_url`,
  optional `api_key`, optional `default_model`, `kind` (`local` / `remote` /
  `tee`), and a `notes` field. Default-provider pointer lives in `settings`
  under `default_ai_provider`.

- `kassiber.ai.client` — `OpenAICompatClient` over stdlib `urllib.request`.
  Speaks the OpenAI-compatible wire format (`/v1/models`,
  `/v1/chat/completions`) so the same code path works against local Ollama,
  LM Studio, OpenAI itself, Maple AI, OpenRouter, and similar.

The chat surface is deliberately narrow: list models, chat once (blocking),
or stream chat (yields delta dicts). Tool-use plumbing lands in a follow-up.
"""

from .providers import (
    AI_PROVIDER_KINDS,
    DEFAULT_AI_PROVIDER_SETTING,
    AI_PROVIDERS_SEEDED_SETTING,
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    list_db_ai_providers,
    redact_ai_provider_for_output,
    resolve_ai_provider,
    seed_default_ai_provider_if_empty,
    set_default_ai_provider,
    clear_default_ai_provider,
    update_db_ai_provider,
)

__all__ = [
    "AI_PROVIDER_KINDS",
    "DEFAULT_AI_PROVIDER_SETTING",
    "AI_PROVIDERS_SEEDED_SETTING",
    "create_db_ai_provider",
    "delete_db_ai_provider",
    "get_db_ai_provider",
    "list_db_ai_providers",
    "redact_ai_provider_for_output",
    "resolve_ai_provider",
    "seed_default_ai_provider_if_empty",
    "set_default_ai_provider",
    "clear_default_ai_provider",
    "update_db_ai_provider",
]
