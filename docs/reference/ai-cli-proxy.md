# CLI proxy providers

A CLI proxy is a server that exposes the coding-agent subscriptions you
are already signed in to (Claude Code, Codex, Gemini CLI, Antigravity, Grok,
Kimi, …) as one HTTP API in the OpenAI, Anthropic and Gemini formats, with
every model those subscriptions include. Kassiber supports
[CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI), the
implementation T3 Code works with, as the provider kind `cli_proxy`. Choose
**CLI proxy** in the provider wizard, give the proxy's address (and its client
key if it has one), press **Test & discover**, and its models are ready for
chat. You configure nothing per model.

Only the Rust backend supports this kind. The Python backend can still list
such a row, but every path that would contact it (chat, model listing, the
connection test) refuses it with `ai_provider_unsupported`, and it cannot
create one.

## What the proxy exposes

These notes come from CLIProxyAPI's source (`internal/api/server_routes.go`,
`internal/access/config_access/provider.go`, the OpenAI handlers and the
model registry) as of November 2026. The server listens on port 8317 by
default.

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/models` | Every model the signed-in accounts can serve, in the OpenAI list shape |
| `POST /v1/chat/completions` | OpenAI Chat Completions for every model, translated to the upstream format |
| `POST /v1/responses` (and `GET` as a WebSocket) | OpenAI Responses: native for Codex models, translated for the rest |
| `POST /v1/messages`, `/v1/messages/count_tokens` | Anthropic Messages |
| `GET /v1beta/models`, `POST /v1beta/models/*` | Gemini `generateContent` and streaming |
| `/v0/management/*`, `/v8/management/*` | Management API: accounts, auth files, quotas, configuration |

**Model list.** `GET /v1/models` returns
`{"object": "list", "data": [{"id", "object", "created", "owned_by"}]}`;
the OpenAI view strips every other field. `owned_by` names the upstream
family: `anthropic` (Claude Code), `openai` (Codex), `google` (Gemini CLI,
AI Studio, Vertex), `antigravity`, `xai` (Grok), `moonshot` (Kimi), `meta`,
and whatever an OpenAI-compatible upstream configured in the proxy reports.
The list also has image and video models (`gpt-image-2`, Grok's imagine
models) under the same families. A request carrying `Anthropic-Version` or a
`claude-cli` User-Agent gets the Anthropic-shaped list instead; Kassiber
sends neither.

**Authentication.** With `api-keys` set in the proxy's `config.yaml`, every
`/v1` request needs one of them, as `Authorization: Bearer <key>`,
`X-Api-Key`, `X-Goog-Api-Key`, or a `key` / `auth_token` query parameter.
A missing key gets `401 {"error": "Missing API key"}`, a wrong one
`401 {"error": "Invalid API key"}`. With no keys configured, the proxy takes
any request. The proxy itself holds the accounts' OAuth tokens. Its clients
never see them.

**Formats.** Each upstream has its own executor. The proxy translates OpenAI
Chat Completions and Responses requests (and their SSE streams) to and from
each upstream's own format, including function calls and their results.
Codex models are served natively on `/v1/responses`. Every other family is
translated.

**Reasoning.** The proxy reads `reasoning_effort` (Chat Completions) and
`reasoning.effort` (Responses), and a `model(level)` or `model(budget)`
suffix. Levels are `none`, `auto`, `minimal`, `low`, `medium`, `high`,
`xhigh` and `max`. For families that use token budgets (Claude, Gemini 2.5),
it maps each level to a budget (`xhigh` 32768, `max` 128000) and clamps it
to the model's range. For level-based families (Codex, Kimi), it passes the
level through, moving it to the nearest one the model supports.

**Streaming and errors.** Streams are standard SSE in the requested format.
Errors are JSON, usually `{"error": {"message", "type", …}}`. The auth
middleware's 401 uses the flat `{"error": "<text>"}` shape.

**Management API.** The management API is reached with a separate
management key: `Authorization: Bearer` or `X-Management-Key`. It is
loopback-only unless `remote-management.allow-remote` is set, and five
failures ban the client's IP for about 30 minutes. It lists auth files
(`/auth-files`, with each account's provider, email and plan), per-account
models (`/auth-files/models`) and model definitions per channel, and it can
proxy calls with an account's token (`/api-call`). Kassiber never asks for
the management key and never calls these endpoints. Inference needs only
the client key, and the management key can read and use every account's
credentials.

## How T3 Code uses it

In T3 Code (0.1.7, the version checked), CLIProxyAPI appears in two places:

- **Usage providers → Add hub**: a URL and a management key. T3 Code reads
  `/v0/management/auth-files`, then makes `api-call` requests for each Claude
  and Codex account's usage endpoints, to show subscription limits. This
  connection only reports usage.
- **Agent traffic** goes through the proxy through each provider instance's
  environment, for example a Claude instance with
  `ANTHROPIC_BASE_URL=http://127.0.0.1:8317` and
  `ANTHROPIC_AUTH_TOKEN=<client key>`. A single "Use proxy" switch for
  Claude, Codex and OpenCode, with a proxy URL, key and optional management
  key, is proposed in pingdotgg/t3code#27356 but not merged.

Kassiber's provider is the inference half of that setup as one first-class
provider: the proxy's own model list replaces per-CLI configuration, and
Kassiber's chat client talks to the proxy directly instead of starting a
CLI. It does not read usage or quotas.

## Kassiber's provider

**Kind and posture.** `kind = "cli_proxy"` is off this device even at a
loopback address, because the proxy sends prompts on to Anthropic, OpenAI,
Google and the other upstreams. It is never acknowledged implicitly. Chat,
model listing and the connection test need the off-device acknowledgement
first, like `remote`. [Agent access](ai.md#agent-access) is `full` unless
you restrict it, as for any off-device provider. The base URL is the
proxy's API root, such as `http://127.0.0.1:8317/v1` or a tailnet address.
There is no default address, so nothing points anywhere until you type one.
A `*-cli://` locator is refused. A CLI proxy takes no API format or
detection (each model has its own) and no `omit_store_field`: every request
carries `store: false`.

**Key.** The proxy's client key is optional and lives where every provider
key lives: the system keychain from the desktop (`ai.providers.set_api_key`),
or the book's SQLCipher store. Like any key, it is bound to the provider's
origin: moving the provider to another host drops it.

**Discovery only on request.** The model list is read only when you start
it: **Test & discover** or **Refresh models** in Settings
(`ai.test_connection`, `ai.list_models` with `refresh`), or
`kassiber ai models` and `kassiber ai providers test` in the CLI. Adding,
editing, listing or showing the provider, opening Settings, launch and idle
contact nothing. Testing a saved CLI proxy at its own address, a refresh,
and `kassiber ai models` keep the list as the provider's saved model check
(`ai_provider_checks`, kind `models`), bound to the current base URL and
cleared by a new key or address, as for every provider. Testing values that
are not saved yet returns the list without storing it. The wizard then
refreshes once after saving, as part of the same setup.

**Each model's format.** Kassiber annotates every listed model with the
format it is spoken in:

| `owned_by` | `api_format` |
| --- | --- |
| `openai` | `responses` (native for Codex) |
| `anthropic`, `google`, `antigravity`, `xai`, `moonshot` | `chat_completions` (translated by the proxy) |
| anything else, or missing | `null`, `unsupported_reason: "unknown_family"` |
| an image, video or speech model (an `image`, `imagine`, `video`, `tts`, `speech` or `audio` id segment) | `null`, `unsupported_reason: "not_a_chat_model"` |

A `<prefix>/<model>` id takes the same table with its own `owned_by`; see
[accounts and prefixes](#accounts-and-prefixes) for the one fallback when a
listing gives none. Such an id also gets `prefix` and `base_model`.
A chat model also gets `supports_tools: true`, `supports_reasoning_effort:
true` and `reasoning_efforts: ["low", "medium", "high", "xhigh", "max"]`.

**Chat.** `ai.chat` names a model. For a CLI proxy, the daemon finds that
model in the saved list and pins its format for the turn, so it never falls
back from one format to the other. It recomputes the format from the
listed `id` and `owned_by` rather than trusting a stored value. All of this
happens before the provider key is read or anything is sent. The chat is
refused with:

- `ai_cli_proxy_models_unchecked` when the provider has no saved list at its
  current address ("Refresh the models …").
- `ai_cli_proxy_model_unknown` when the list does not have the model.
- `ai_cli_proxy_format_unknown` (`details.reason` `unknown_family` or
  `not_a_chat_model`, with `owned_by`) when the model has no format.

A stale list (a later refresh failed) still answers chats. Streaming, tool
calls, consent, [permission modes](ai.md#permission-modes), undo hints,
history and the privacy receipt work exactly as for any HTTP provider. The
proxy changes only the transport.

**Reasoning effort.** A CLI proxy sends the proxy's levels instead of the
HTTP clamp: `low`, `medium`, `high`, `xhigh` and `max` as chosen, `ultra` as
`max`, and `auto` sends nothing. It uses `reasoning_effort` on Chat
Completions and `reasoning.effort` on the Responses API. The usual one-time
retry without the effort applies if the proxy refuses it.

**Connection test.** `ai.test_connection` with `kind: "cli_proxy"`, or a
saved CLI proxy, lists the models strictly: a 401 or an unreadable list is
an error, never an empty list. With a model (the form's default model, else
the saved one, else the only listed model), it sends the fixed test request
in that model's format. The result adds `models_saved`. It reports
`api_format: null` and `detected_api_format: null`, and `probe.api_format`
names the format used. A model the list does not have, or one without a
format, is not asked: `probe.status` is `skipped` with `reason`
`model_not_listed`, `not_a_chat_model` or `unknown_family`.

**Errors.** Proxy error bodies go through the same credential scrub as every
provider error: the API key, and any credential in the base URL, is
withheld from messages and `details.body`, even when the proxy repeats it.

**CLI.** `kassiber ai providers create NAME --kind cli_proxy --base-url URL
--acknowledge [--api-key-fd FD | --api-key-stdin] [--default-model ID]`
creates one with the desktop flow's fields: the base URL is required, the
acknowledgement too (a CLI proxy without `--acknowledge` is refused, as the
wizard does not save one), the key goes to the book's SQLCipher store, and
`--api-format` and `--omit-store-field` are refused. Creating contacts
nothing. `kassiber ai models --provider NAME` reads and saves the model list,
and `kassiber ai providers test NAME [--model ID]` does too and asks the
model (the default model when none is given) in its format.
`ai providers update --kind cli_proxy` turns an existing provider into one.
This grammar is Rust-only; see
[the Rust shell](rust-shell.md#cli-proxy-providers).

## Accounts and prefixes

Several accounts per provider (three Claude subscriptions, two Codex
accounts, …) are the proxy's business. Kassiber sees one endpoint, one client
key and one model list, and the proxy picks an account for each request.
These notes come from CLIProxyAPI's `config.example.yaml`, its model
registry and `sdk/cliproxy/service_models.go`.

**One list for every account.** The registry keeps one entry per model id
and counts the accounts that serve it, so a model several accounts share is
listed once. The list is the union of what the signed-in accounts offer: a
model shows up while at least one account serves it. A model whose accounts
are all in a quota cooldown stays listed, and a request then gets a 429 with
its retry time instead of an unknown model.

**Routing and failover.** `routing.strategy` picks the account:
`round-robin` (the default), `weighted-round-robin` (each credential's
`weight`) or `fill-first`. When an account answers 403, 408, 429 or 5xx, the
proxy tries other accounts first (`routing.retry.request-retry`, 3 by
default, waiting at most `max-retry-interval` for a cooldown). Kassiber sees
an error only once every eligible account has failed: a 429 as
`ai_rate_limited` (retryable), a 401 or 403 as `ai_auth_failed`.

**Session affinity.** Off by default (`routing.session-affinity: false`).
Turned on, a session stays on one account for `session-affinity-ttl` (1 hour
by default) and moves only on failover (`session-affinity-rebind-on`), which
keeps the upstream's prompt cache warm. The proxy recognises a session from
Claude Code, Codex, OpenCode and pi session headers, `prompt_cache_key`,
Responses conversation ids, and as a last resort a hash of the first
message. Kassiber sends none of the headers or ids, so the first-message
hash decides. Kassiber sends a chat's whole history on every turn, so that
hash stays the same through a chat and the chat should stay on one account.
This has not been tried against a real proxy yet.

**Prefixes.** A credential configured with `prefix: "work"` serves its
models a second time as `work/<model>`, and a request for that id goes only
to that credential, so a prefix is how to pick one account. CLIProxyAPI makes
the prefixed entry by copying the model's entry and changing only the id
(`applyModelPrefixes`): `owned_by`, and so the family, is kept. Without
`force-model-prefix` the credential also keeps serving the unprefixed id in
the shared pool; with it, only the prefixed one. An OpenAI-compatible
upstream configured in the proxy reports its own configured name as
`owned_by`, and the proxy's Devin models are listed as `devin/<model>` with
their real families.

Kassiber sends a prefixed id exactly as listed and takes its format from
the table above with the row's own `owned_by`, even when that names a family
Kassiber does not know. Only when a listing gives no `owned_by` at all (an
older or another proxy) does `<prefix>/<model>` take the family the same
listing gives `<model>`, the id after the first `/`. Nothing else is
guessed: without its own `owned_by` and without a listed base model (as with
`force-model-prefix`), the model has no format and is refused with
`ai_cli_proxy_format_unknown`, reason `unknown_family`. Listed models get
`prefix` and `base_model`; the model picker shows the base model with its
prefix beside it and the full id below, and keeps each prefix's models
together after the unprefixed ones.

**After account changes, refresh.** Chats read the saved list, never the
proxy's live one. After signing an account in or out, adding or changing a
prefix, or a plan change that adds or removes models, press **Refresh
models** on the provider's Settings row (or run `kassiber ai models
--provider NAME`). Until then, a model that disappeared still resolves and
the proxy refuses it, and a new one is refused with
`ai_cli_proxy_model_unknown`.

## Setting one up

1. Run CLIProxyAPI and sign your subscriptions in with its login options
   (`--claude-login`, `--codex-login`, …). Optionally set `api-keys` in its
   `config.yaml`, which is recommended for anything not on loopback.
2. In Kassiber, open **Settings → Assistant → Add provider → Through your
   subscription app → CLI proxy**.
3. Enter the address, such as `http://127.0.0.1:8317/v1` or
   `https://cli-proxy.<tailnet>.ts.net/v1`, and the client key if the proxy
   has one.
4. Acknowledge that prompts leave this device.
5. Press **Test & discover**. The models are listed, each with its family
   and format. Choose a default model and save.
6. Later, **Refresh models** in the provider row reads the list again, for
   example after signing another subscription in to the proxy.

## Tests

- `crates/ai/tests/cli_proxy.rs` runs a loopback fake CLIProxyAPI. Its
  model list mixes families and lists a `work/` credential's models as
  CLIProxyAPI does (a second listing gives prefixed ids without `owned_by`),
  and it serves Codex only on `/v1/responses` and
  the rest only on `/v1/chat/completions`, both streamed with tool calls,
  with auth on and off and errors that repeat the key. The tests cover no
  egress before consent, discovery and the saved inventory, probing in each
  model's format, the credential scrub, a stale list after a failed
  refresh, and effort levels.
- `crates/ai/tests/chat.rs` runs whole chat turns through a scripted proxy,
  with consent and `full_access`, in both formats.
- `tests/test_rust_cli_proxy_daemon.py` runs one Rust daemon session against
  a loopback fake: no request before Test & discover, the saved list, chats
  in each model's format, a consented tool call, and the refusals.
- `crates/ai/src/providers/tests.rs` and `cli_proxy.rs` cover the store rules
  and the family table. Entries named `cli-proxy-*` in
  `crates/ai/scripts/mutations.json` pin each rule to the tests that kill
  it.
- `tests/test_ai_cli_proxy_rust_only.py` checks that the Python backend
  refuses the kind, and `tests/test_rust_ai_providers.py` pins the one
  message difference (an unknown kind's refusal lists `cli_proxy`).
- `ui-tauri/src/components/kb/aiProviderFormModel.test.ts` and
  `AiProviderForm.dom.test.tsx` cover the wizard flow, and
  `ui-tauri/src/components/ai/ModelListRow.dom.test.tsx` the prefixed models
  in the picker.
