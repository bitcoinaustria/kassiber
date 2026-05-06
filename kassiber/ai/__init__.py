"""AI provider configuration and chat clients.

Two layers:

- `kassiber.ai.providers` — SQLite-backed CRUD for AI provider records,
  mirroring the shape of `kassiber.backends`. Stores `name`, `base_url`,
  optional `api_key`, optional `default_model`, `kind` (`local` / `remote` /
  `tee`), and a `notes` field. Default-provider pointer lives in `settings`
  under `default_ai_provider`.

- `kassiber.ai.client` — OpenAI-compatible HTTP plus fixed Claude/Codex CLI
  adapters. HTTP providers speak `/v1/models` and `/v1/chat/completions`;
  CLI locators use `claude-cli://default` or `codex-cli://default` and are
  treated as off-device unless explicitly acknowledged.

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
    require_ai_provider_acknowledged,
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
    "require_ai_provider_acknowledged",
    "resolve_ai_provider",
    "seed_default_ai_provider_if_empty",
    "set_default_ai_provider",
    "clear_default_ai_provider",
    "update_db_ai_provider",
]
