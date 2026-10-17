"""AI provider configuration and chat clients.

Provider configuration and execution are split by responsibility:

- `kassiber.ai.providers` — SQLite-backed CRUD for AI provider records,
  mirroring the shape of `kassiber.backends`. Stores `name`, `base_url`,
  optional `api_key`, optional `default_model`, `kind` (`local` / `remote` /
  `tee`), a `notes` field, an `enabled` switch, an optional default
  reasoning effort, and the HTTP wire format (`api_format`, plus the
  separately stored `detected_api_format`). Default-provider pointer lives in `settings`
  under `default_ai_provider`.

- `kassiber.ai.client` — OpenAI-compatible HTTP transport and the
  provider-client factory. HTTP providers speak `/v1/models` and either
  `/v1/responses` or `/v1/chat/completions`, per the provider's `api_format`
  (`auto` uses the detected format, else Responses).
- `kassiber.ai.broker_client` — supervisor for the bundled
  Node broker for Codex, Claude, OpenCode, and GitHub Copilot (over the
  Agent Client Protocol). CLI locators are treated as off-device
  unless explicitly acknowledged.
- `kassiber.ai.contracts` / `kassiber.ai.model_metadata` — small shared client
  contracts and bounded provider capability metadata.

The chat surface is deliberately narrow: list models, chat once (blocking),
or stream semantic deltas. Inside Kassiber, tool calls and outputs are typed
Responses Items whichever format the provider speaks.
"""

from .providers import (
    AI_PROVIDER_KINDS,
    AI_REASONING_EFFORTS,
    DEFAULT_AI_PROVIDER_SETTING,
    AI_PROVIDERS_SEEDED_SETTING,
    create_db_ai_provider,
    contactable_cli_runtimes,
    delete_db_ai_provider,
    get_db_ai_provider,
    get_ai_provider_api_key_for_use,
    list_db_ai_providers,
    mark_ai_provider_secret_ref_state,
    redact_ai_provider_for_output,
    require_ai_provider_acknowledged,
    require_ai_provider_contactable,
    require_ai_provider_enabled,
    require_ai_provider_endpoint,
    require_ai_test_target_acknowledged,
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
    "AI_REASONING_EFFORTS",
    "DEFAULT_AI_PROVIDER_SETTING",
    "AI_PROVIDERS_SEEDED_SETTING",
    "create_db_ai_provider",
    "contactable_cli_runtimes",
    "delete_db_ai_provider",
    "get_db_ai_provider",
    "get_ai_provider_api_key_for_use",
    "list_db_ai_providers",
    "mark_ai_provider_secret_ref_state",
    "redact_ai_provider_for_output",
    "require_ai_provider_acknowledged",
    "require_ai_provider_contactable",
    "require_ai_provider_enabled",
    "require_ai_provider_endpoint",
    "require_ai_test_target_acknowledged",
    "resolve_ai_provider",
    "seed_default_ai_provider_if_empty",
    "set_default_ai_provider",
    "set_db_ai_provider_api_key",
    "set_db_ai_provider_native_secret_ref",
    "clear_default_ai_provider",
    "update_db_ai_provider",
]
