"""AI provider configuration and chat clients.

Provider configuration and execution are split by responsibility:

- `kassiber.ai.providers` — SQLite-backed CRUD for AI provider records,
  mirroring the shape of `kassiber.backends`. Stores `name`, `base_url`,
  optional `api_key`, optional `default_model`, `kind` (`local` / `remote` /
  `tee`), and a `notes` field. Default-provider pointer lives in `settings`
  under `default_ai_provider`.

- `kassiber.ai.client` — OpenAI Responses-compatible HTTP transport and the
  provider-client factory. HTTP providers speak `/v1/models` and
  `/v1/responses`.
- `kassiber.ai.cli_client` — fixed Claude/Codex CLI adapters. CLI locators use
  `claude-cli://default` or `codex-cli://default` and are treated as off-device
  unless explicitly acknowledged.
- `kassiber.ai.contracts` / `kassiber.ai.model_metadata` — small shared client
  contracts and bounded provider capability metadata.

The chat surface is deliberately narrow: list models, chat once (blocking),
or stream semantic deltas. Tool calls and outputs cross the provider boundary
as typed Responses Items.
"""

from .providers import (
    AI_PROVIDER_KINDS,
    DEFAULT_AI_PROVIDER_SETTING,
    AI_PROVIDERS_SEEDED_SETTING,
    create_db_ai_provider,
    delete_db_ai_provider,
    get_db_ai_provider,
    get_ai_provider_api_key_for_use,
    list_db_ai_providers,
    mark_ai_provider_secret_ref_state,
    redact_ai_provider_for_output,
    require_ai_provider_acknowledged,
    resolve_ai_provider,
    seed_default_ai_provider_if_empty,
    set_default_ai_provider,
    set_db_ai_provider_api_key,
    set_db_ai_provider_native_secret_ref,
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
    "get_ai_provider_api_key_for_use",
    "list_db_ai_providers",
    "mark_ai_provider_secret_ref_state",
    "redact_ai_provider_for_output",
    "require_ai_provider_acknowledged",
    "resolve_ai_provider",
    "seed_default_ai_provider_if_empty",
    "set_default_ai_provider",
    "set_db_ai_provider_api_key",
    "set_db_ai_provider_native_secret_ref",
    "clear_default_ai_provider",
    "update_db_ai_provider",
]
