# AI Reference

Kassiber has three AI-related layers:

- an external Agent Skill for AI coding and terminal assistants
- the in-app assistant that ships with the desktop UI (and a CLI surface that
  reuses the same provider config)
- planned in-product AI help for OCR, extraction, and reconciliation workflows

These are related, but they are not the same thing.

## What exists today

Two surfaces ship today:

- The Kassiber CLI Agent Skill at
  [bitcoinaustria/kassiber-skill](https://github.com/bitcoinaustria/kassiber-skill) for AI
  coding and terminal assistants.
- An **in-app assistant** in the desktop UI that streams chat from an
  OpenAI Responses-compatible endpoint or fixed Claude/Codex CLI adapter, plus a
  parallel CLI surface (`kassiber chat`, `kassiber ai providers …`,
  `kassiber ai models`) that reuses the same provider config.

The external skill helps an AI assistant use the Kassiber CLI safely and
correctly for:

- onboarding and context checks
- wallet setup and imports
- journal processing
- reports
- metadata cleanup
- troubleshooting

The core accounting workflow does not depend on AI. Watch-only source refresh, imports,
journal processing, and reports should work without any model at all.

## Current direction

Kassiber's AI direction is intentionally narrow and review-gated.

The intended uses are:

- local OCR from invoice PDFs or images
- structured extraction from attached documents
- confidence-scored reconciliation suggestions
- tie-breaking when deterministic matching narrows the field but does not finish it

Deterministic matching should work without AI first. AI should stay optional.

## Why the Responses API

Kassiber's HTTP transport uses `POST /v1/responses` rather than the legacy
Chat Completions endpoint. OpenAI recommends Responses for new projects and
describes it as the future-facing interface for agentic and multimodal work.
For Kassiber, the practical improvements are:

- typed `message`, `function_call`, `function_call_output`, and `reasoning`
  Items instead of encoding every provider action as a chat message
- semantic streaming events such as `response.output_text.delta` and
  `response.completed`, which are less ambiguous than parsing choice deltas
- correct reasoning-model tool round-trips: Kassiber replays the complete
  response output alongside each matching function result
- a direct path to future Responses-only models and tools without another
  transport migration

OpenAI also reports a 3% SWE-bench improvement for reasoning models and
40–80% better cache utilization in its internal Responses-vs-Chat-Completions
tests. Those are OpenAI measurements, not a promised improvement for Ollama,
oMLX, or every model Kassiber can connect to. See the
[OpenAI migration guide](https://developers.openai.com/api/docs/guides/migrate-to-responses),
[function-calling guide](https://developers.openai.com/api/docs/guides/function-calling),
and [streaming guide](https://developers.openai.com/api/docs/guides/streaming-responses).

Kassiber deliberately does **not** enable provider-managed conversation state.
Every HTTP request sets `store: false`; system guidance is sent through
`instructions`, and the bounded tool loop replays typed output Items in memory.
Persisted chat history remains a Kassiber/SQLCipher concern and is sent again
as input only when the user resumes a session. This keeps the local-first
storage boundary and also works with providers that implement only stateless
Responses.

The implementation keeps those invariants in one request builder. Prepared
tool-loop context is an explicit typed input and cannot be combined with the
legacy message input accidentally. HTTP transport remains in
`kassiber.ai.client`; fixed external CLI adapters live in
`kassiber.ai.cli_client`, with shared delta/request contracts isolated in
`kassiber.ai.contracts`.

## Privacy warning

Treat AI prompts as sensitive accounting data.

Depending on what you send, prompts may contain:

- wallet labels
- addresses
- transaction notes
- imported document contents
- backend hostnames
- reconciliation context
- accounting or tax annotations

Normal Kassiber `backends ...` and `wallets ...` output is intentionally
narrowed for secret-bearing config values, but that is not a general privacy
guarantee. Do not paste raw credentials, raw private descriptors, Silent
Payments `sp()` / `spscan` scan material, wallet exports, or other sensitive
material into a remote model unless that is acceptable for your threat model.

If in doubt, keep inference local.

Claude CLI and Codex CLI are supported for convenience, but they are not a
local-privacy guarantee. Kassiber launches them in a narrow non-interactive mode
that still uses their normal local authentication/config, telemetry, and
model-provider routing. Treat them as off-device unless your local CLI setup is
explicitly backed by a local or confidential provider.

## Recommended inference setup

Local inference is the recommended default.

Kassiber seeds first-class local provider rows for:

- [Ollama](https://ollama.com/) at `http://localhost:11434/v1`
- [oMLX](https://omlx.ai/) at `http://127.0.0.1:8000/v1`

Ollama remains the default provider for compatibility, but oMLX appears as a
built-in local provider and Settings preset. Run the server (`ollama serve`, or
`omlx start` / the oMLX menu-bar app) and use **Test connection** in Settings
before saving a provider change.

HTTP providers must implement `POST /v1/responses`; a server that only exposes
`/v1/chat/completions` is no longer sufficient. Ollama added its stateless
Responses endpoint in v0.13.3, including streaming, function calling, and
reasoning summaries, so older Ollama installations must be upgraded. Current
oMLX releases and OpenRouter also expose `/v1/responses`. See
[Ollama's compatibility reference](https://docs.ollama.com/api/openai-compatibility),
[oMLX releases](https://github.com/jundot/omlx/releases), and the
[OpenRouter Responses reference](https://openrouter.ai/docs/api/api-reference/responses/create-responses).

If Kassiber itself is running inside a container and Ollama is running on the
host, seed the provider with the Docker host alias instead:

```bash
KASSIBER_DEFAULT_AI_BASE_URL=http://host.docker.internal:11434/v1
```

This only affects first-time provider seeding. For an existing book, update the
`ollama` provider's `base_url` instead.

To make oMLX the default for a brand-new book, set:

```bash
KASSIBER_DEFAULT_AI_PROVIDER=omlx
KASSIBER_DEFAULT_AI_BASE_URL=http://127.0.0.1:8000/v1
```

Per-provider seed overrides are also supported:

```bash
KASSIBER_OMLX_AI_BASE_URL=http://127.0.0.1:8000/v1
KASSIBER_OLLAMA_AI_BASE_URL=http://host.docker.internal:11434/v1
```

Example:

```bash
ollama run qwen3.6:35b
```

Local testing so far has used `qwen3.6:35b` with good results for Kassiber-style
assistant flows. Smaller and less powerful models can still be useful for
narrower tasks, and should become more practical as Kassiber's prompts, skill
bundle, and workflows get tighter.

The photo/PDF transaction importer is stricter than chat: it only accepts a
local loopback provider and an installed vision/OCR model. Good Ollama choices
for that surface are `glm-ocr` for fast document OCR, `qwen3-vl:8b` or
`qwen3-vl:4b` for stronger multimodal table reasoning, and
`llama3.2-vision:11b` / `minicpm-v:8b` as broad fallback models. Remote,
TEE, Claude CLI, and Codex CLI providers are hard-disabled for document OCR.

Claude CLI and Codex CLI can be added with fixed provider locators:

```bash
kassiber ai providers create claude-cli --base-url claude-cli://default --kind remote --acknowledge --default-model default
kassiber ai providers create codex-cli --base-url codex-cli://default --kind remote --acknowledge --default-model default
```

For these providers, `--model` / `default_model` is forwarded to the CLI when it
is not `default`. The assistant's thinking selector sends `reasoning_effort` for
Responses-compatible providers, maps to Claude CLI `--effort`, and maps to Codex
CLI's `model_reasoning_effort` config override.

## In-app surface

The desktop assistant lives at the bottom of every authenticated screen. Its
provider/model picker is fed by `ai.providers.list` and `ai.list_models` over
the daemon protocol; chat streaming is wired through Tauri events
(`daemon://stream`) so the UI can render loading status, reasoning
(`<think>`), and the answer in real time without blocking navigation.

For browser-driven development, the Vite dev server also exposes a loopback-only
daemon bridge. Run:

```bash
pnpm --dir ui-tauri run dev:bridge
```

Then open `http://127.0.0.1:5173`. In bridge mode, the browser talks to the
same local Python daemon protocol through Vite: normal calls use
`/__kassiber__/daemon`, and `ai.chat` streams NDJSON records from
`/__kassiber__/daemon/stream`. This is a development-only convenience for
testing real local AI, tool cards, cancellation, and consent from an ordinary
browser tab. Packaged builds and `tauri dev` do not use the bridge.

Provider configuration is mirrored in the CLI:

```bash
kassiber ai providers list
printf '%s\n' "$OPENAI_API_KEY" | kassiber ai providers create openai --base-url https://api.openai.com/v1 --kind remote --acknowledge --api-key-stdin --default-model gpt-4o-mini
kassiber ai providers create claude-cli --base-url claude-cli://default --kind remote --acknowledge --default-model default
kassiber ai providers set-default openai
kassiber ai models
kassiber chat "Summarise the last week of imports."
git log --oneline -20 | kassiber chat -
kassiber chat
```

`kassiber chat` is the CLI client for the same daemon-backed assistant used by
the desktop UI — there is exactly one chat surface and one protocol. It starts
a local daemon transport, sends `ai.chat` requests with `tools_enabled=true`,
renders streaming deltas in the terminal, and sends `ai.tool_call.consent`
decisions when mutating tools ask for approval. Pass a prompt positionally or
with `--prompt` for one turn; `kassiber chat -` reads the one-shot prompt from
stdin for pipelines and heredocs. After each rendered turn a dim provenance
footer shows provider/model, the tools that actually ran, and whether journals
were auto-refreshed — the same provenance the desktop Assistant records.
`--no-tools` disables the tool loop for a provider-only exchange, and
`--system "..."` replaces the built-in Kassiber system prompt with a raw one
(`system_prompt_kind="raw"`).

CLI chat defaults to `--tool-profile core`, a reduced schema for small local
models that covers common accounting, wallet, transaction, report, journal,
rate, readiness, and read-only swap-review workflows. Use
`--tool-profile full` when the model needs the specialist catalog, such as
source-of-funds editing, Lightning node snapshots, saved views, or advanced
swap mutations.

Use `--timeout SECONDS` for harnesses or local models that need a shorter or
longer wait. It caps daemon startup and provider stream inactivity (default
120 seconds). The value is local transport control only: Kassiber does not put
it in system prompts, user messages, tool schemas, tool results, or transcripts.

Omit the prompt for REPL mode, which has line editing and in-session history
on real terminals, and these commands:

- `/help` — command help; `/exit`, `/quit`, or Ctrl-D leaves.
- `/tools` — the daemon tool catalog with consent classes.
- `/model [id]`, `/provider [name]` — show or switch mid-session; a provider
  switch re-resolves that provider's default model and rolls back on error.
- `/allow <tool>`, `/allowed` — manage which mutating tools are pre-approved
  for this session.
- `/new` — start a fresh conversation without restarting the daemon.

Ctrl-C during a reply cancels that turn cooperatively and keeps the session.
Daemon-side `allow_session` consent spans a single `ai.chat` request, so the
REPL carries an interactive "[s] session" answer across turns client-side and
re-sends it for that tool.

Output modes cover scripting:

- default rendered text for humans. On a TTY, the model's markdown is
  rendered with ANSI styling — bold, inline code, headers, bullets, fenced
  code, and pipe tables re-drawn as box-aligned tables — while preserving
  token streaming, and successful tool results draw a compact deterministic
  table straight from the daemon envelope, so tabular numbers on screen never
  depend on the model retyping them correctly. `--plain` turns both off.
  With piped stdout, the raw answer text is the only thing on stdout —
  progress labels, tool announcements, tool tables, consent UI, and the
  provenance footer move to stderr;
- `--machine` / `--format json` (one-shot only) emits a single `chat` envelope
  with the final message, `finish_reason`, provenance, and tool-call summary;
- `--stream-json` (one-shot only, mutually exclusive with `--machine`) emits
  the raw daemon stream records — `ai.chat.status`, `ai.chat.delta`,
  `ai.chat.tool_call`, `ai.chat.tool_consent_required`, `ai.chat.tool_result`,
  then the terminal `ai.chat` — as NDJSON, mirroring what the desktop bridge
  streams;
- `--transcript PATH` (any mode, REPL included) appends every daemon request
  and stream record for the session to PATH as NDJSON — a local audit trail
  for debugging model answers and tool behavior. The file is plaintext and
  contains prompts and redacted tool results; treat it like notes, not like
  the encrypted database.

For automation, `kassiber chat --yes "..."` approves mutating tool requests for
that chat session without prompting. Prefer the narrower
`--allow-tool ui.journals.process` form when a script should approve only one
tool. Machine and `--stream-json` runs never prompt interactively even on a
TTY; there, and without a TTY in rendered mode, unapproved mutating tools are
denied and the denial is fed back to the model as `user_denied`.

If the local database is SQLCipher-encrypted and the CLI cannot unlock it,
`kassiber chat` fails with `passphrase_required` before contacting the model.
Headless scripts should pass the global `--db-passphrase-fd <FD>` flag from a
parent process. The passphrase is consumed by the local daemon only; it is not
embedded in prompts or tool output.

Test the assistant/tool path through the CLI, not only through the desktop GUI:
`tests/test_cli_chat.py` pins the daemon-backed chat loop, consent behavior,
locked-database handling, timeout controls, and `core`/`full` tool profiles.
Live backend checks should also be CLI-first, with explicit user-approved
endpoints and a fresh temporary data root.

## Chat history

Chat sessions can persist — inside the SQLite/SQLCipher database, next to the
data the answers were derived from, never as separate plaintext files. The
policy setting `ai_chat_history` has three values, managed via
`kassiber chats config [--history auto|on|off]`:

- `auto` (default) — persist only when the database file is
  SQLCipher-encrypted. A plaintext database stays ephemeral; running
  `kassiber secrets init` is what unlocks history.
- `on` — persist regardless of encryption (an explicit user choice).
- `off` — never persist.

`kassiber chat --incognito` skips persistence for one session regardless of
the setting. `kassiber chat --continue` resumes the most recently updated
session (the stored messages are replayed to the model as context);
`--session <id>` resumes a specific one. In the REPL, `/new` starts a fresh
session. Stored exchanges keep the user prompt, the assistant answer, the
`finish_reason`, and the answer provenance — not full tool result envelopes,
which remain reproducible from the database (use `--transcript` for
full-fidelity capture).

Answer provenance includes a UI-only `privacy_receipt`: provider kind,
local/remote classification, screen route, number of advertised schemas and
executed tools, plus outbound event/endpoint/byte counts recorded during that
turn. The receipt is computed after the provider call and is never fed back to
the model. Exact hosts remain available only on the dedicated local Egress
screen; the AI-facing egress tool receives aggregate subsystem counts.

Manage stored sessions with `kassiber chats list`, `chats show <id>`,
`chats delete <id>`, and `chats clear`. Machine chat envelopes and the
terminal `ai.chat` record carry `session_id` (null when nothing persisted).

On the wire, persistence is per request: `ai.chat` accepts
`persist: true | false | "auto"` plus `session_id` to append to an existing
session; unknown session ids fail before streaming starts. A request opts in
by sending `persist` true/`"auto"` or a `session_id`; with neither, nothing
persists, so existing clients are unchanged. The stored policy stays
authoritative over every write — `off` never persists and `auto` persists
only on encrypted databases, even for continuations of an existing session. Session management is exposed as the daemon
kinds `ui.chat.sessions.list`, `ui.chat.sessions.get`,
`ui.chat.sessions.delete`, and `ui.chat.sessions.clear`, profile-scoped like
the rest of the UI surface, plus `ui.chat.history.configure` for reading or
setting the policy (the desktop Settings control). The desktop Assistant
sends `persist: "auto"`, round-trips `session_id`, and surfaces history in
its toolbar; these kinds stay usable while the AI runtime toggle is off,
because seeing and deleting stored history is a privacy control, not an AI
feature. Chat history is intentionally **not** an AI tool:
the model cannot browse or search prior sessions on its own. Only a session
the user explicitly resumes is replayed as normal chat context, so
prompt-injection risk stays scoped to the resumed conversation. Diagnostics
reports and audit packages do not include chat content.

Provider API-key entry supports `--api-key-stdin` and `--api-key-fd FD`. The
legacy `--api-key <value>` form still works as a warning-on-use compatibility
shim, but docs and tests avoid it because argv can land in shell history and
process listings. Desktop Settings uses the narrow `ai.providers.set_api_key`
daemon kind to rotate/re-enter a key and `ai.providers.move_api_key` to move a
stored key between `sqlcipher_inline` and the selected native store. The daemon
rejects `api_key` on `ai.providers.create`, `ai.providers.update`, and
`ai.test_connection`; connection tests use the stored provider key after it has
been saved.

Provider envelopes expose only `has_api_key` plus
`secret_ref.{store_id,state}`. `sqlcipher_inline` keeps the key in the
SQLCipher database. Native desktop storage records only non-secret
`ai_provider_secret_refs` metadata and stores the value in macOS Keychain,
Windows user-scope Credential Manager/DPAPI, or Linux Secret Service when
platform policy selects that store. Backup export records only ref metadata for
non-inline AI keys and refuses inconsistent rows where a non-inline ref still
has an inline `api_key`; backup import surfaces a non-fatal
`secret_ref_unavailable` warning so Settings can prompt for re-entry. Backend
tokens, descriptors, xpubs, and blinding keys stay SQLCipher-protected. See
[`../plan/10-secret-management.md`](../plan/10-secret-management.md).

Reasoning-capable models surface provider-exposed reasoning summaries or
model-authored thinking text through one of two channels, and both are split
into collapsible reasoning pane(s) above the answer:

- Inline `<think>...</think>` tags inside the content stream — emitted by
  DeepSeek-R1 and QwQ.
- Structured Responses reasoning-summary events — emitted by supported OpenAI
  reasoning models and by compatible Ollama/oMLX thinking models.

Each user turn gets its own assistant message. Inside a tool-using turn,
each provider completion round (`waiting_for_model` before the next model
call) opens a fresh reasoning segment, so Ollama/oMLX traces stay
per-round instead of one continuous blob. Models that don't emit either
channel pass through unchanged.

Settings → AI providers exposes a **Test connection** action. It calls the
daemon's `ai.test_connection` kind with the *currently entered* base URL and
API key (or, when editing without changing the API-key field, the saved key)
and reports the model count without persisting anything. For Claude/Codex CLI
locators, this only verifies that the CLI executable is present; authentication
and model reachability are checked when chat starts. For HTTP providers, the
connection test probes `/v1/models`; it does not spend tokens on a generation,
so the first chat remains the final check that `/v1/responses` is enabled.

Remote, TEE, Claude CLI, and Codex CLI providers require explicit
acknowledgement before chat. The CLI uses
`kassiber ai providers update <name> --acknowledge` (or `--acknowledge` during
`create`), and the desktop Settings form prompts before saving an off-device
provider. Without that acknowledgement, `ai.chat` returns
`ai_remote_ack_required` before sending any prompt content.

Streaming is demuxed by `request_id`: the Tauri supervisor keeps one daemon
process and one stdout reader, but routes each JSON envelope to the matching
request. While a chat is streaming, unrelated daemon calls can complete
independently.

Before the first token arrives, `ai.chat` may emit `ai.chat.status` records
with phases such as `preparing`, `connecting`, and `waiting_for_model`. These
records are UI progress hints only; model-authored thinking is shown only when
the provider emits inline `<think>` content or structured reasoning-summary
deltas.

Pressing **Stop** in the desktop UI, choosing cancel at a terminal consent
prompt, or interrupting `kassiber chat` sends `ai.chat.cancel` with
`args.target_request_id = <active ai.chat request_id>`. Cancellation is
best-effort and cooperative: Kassiber stops forwarding deltas once the Python
worker returns between provider chunks, then emits the terminal `ai.chat`
envelope with `finish_reason: "cancelled"`. For metered remote providers, any
tokens already generated or in flight may still be billed.

## Tool use

The desktop assistant and `kassiber chat` opt into a bounded tool loop with
`ai.chat` top-level args:

```json
{
  "tools_enabled": true,
  "tool_loop_max_iterations": 8,
  "system_prompt_kind": "kassiber"
}
```

Tool control stays top-level; generation options still live under `options`.
When enabled, Kassiber prepends a compact Kassiber skill-aware system prompt,
sends flat Responses function definitions, emits `ai.chat.tool_call`,
`ai.chat.tool_consent_required`, and
`ai.chat.tool_result` stream records as needed, feeds tool results back as
typed `function_call_output` Items with matching `call_id` values, and finishes
with the normal terminal `ai.chat` envelope. The complete provider output is
replayed between tool rounds so reasoning Items are not discarded.

Before the provider is called, Kassiber also runs a small deterministic
read-only router for Kassiber questions. It looks for common accounting intents
such as pending work, sync readiness, totals, inflow/outflow, balances, tax
summaries, largest/smallest transactions, transaction search, quarantine,
the combined review worklist, loans, book-set views, transfers/direct payouts,
swap-review context, saved review filters, auto-pair rules, and pricing.
Matching read-only tool results are streamed to the UI and inserted into the
model context as exact local data, so small local models can answer from program
output instead of doing their own arithmetic. That auto-read context is sent as
untrusted accounting data, not as system instructions, and is bounded per tool
so large reports cannot silently crowd out everything else.
When one of those reads needs current reports or journal-derived state,
Kassiber refreshes stale local journals first and includes the
`ui.journals.process` result in the tool result metadata. This refresh is local
and deterministic, and the same refresh path applies when the desktop GUI reads
journal-derived daemon kinds such as `ui.reports.*` or `ui.report.blockers`
directly. Watch-only source refresh remains explicit unless the active profile has
enabled automatic refresh-before-report maintenance, because refresh can contact
external services and import new transactions. Automatic refresh results are
redacted before they enter AI/UI tool metadata; exact backend URLs are not sent
to the provider. If any source refresh row fails, the maintenance/report-blocker
path returns a blocking `sync_failed` item instead of saying reports are ready.

Quoted or search-like questions can trigger `ui.transactions.search` before the
provider is called. Matching transaction notes, descriptions, counterparties,
tags, and values may then be included in the provider context; keep inference
local when those fields are sensitive.

Change-audit reads require an explicit previous-answer timestamp. Without a
baseline, `ui.audit.changes_since_last_answer` returns
`status: "baseline_required"` rather than claiming that nothing changed.
When a baseline is provided, the answer includes transaction metadata edit
events alongside transaction, wallet, journal, quarantine, and rate changes.
Bounded edit-history reads are available only through the safe
`ui.transactions.history` and `ui.activity.history` daemon tools; the assistant
does not get raw SQLite or CLI access.

The in-app prompt is a digest, not a full Agent Skill dump. It teaches the model the local-first accounting
role, the normal workflow order, the journal reprocessing rule, and the
boundary between read-only information and mutating actions. The assistant is
skill-aware, but it is not shell-powered or CLI-powered: there is no raw command
execution, raw filesystem access, arbitrary daemon dispatch, or generic
Kassiber CLI tool.

Clients should upsert tool cards by `call_id`. Mutating tools emit an initial
`ai.chat.tool_call` with `needs_consent: true`, followed by
`ai.chat.tool_consent_required`. If the user approves the call, the daemon emits
another `ai.chat.tool_call` for the same `call_id` with `needs_consent: false`
before `ai.chat.tool_result`; that second record marks the approved call as
running and must not create a duplicate card.

Read-only provider tool names run automatically through safe daemon snapshot
surfaces:

Live chats do not advertise the entire catalog on every turn. The daemon picks
bounded capability packs (`core`, `workspace`, `transactions`, `reports`,
`wallets`, `loans`, `privacy`, `source_funds`, `merchant`, `transfers`,
`operations`) from the
latest question and optional typed `screen_context`. Capability-discovery
questions can still request the full catalog. This reduces schema/token load
and improves tool choice on smaller local models without widening execution:
`get_tool` and the daemon dispatcher remain the authoritative allowlists.

Desktop chat builds an ephemeral `screen_context` from a positive registry of
canonical routes and capability packs. It contains only a route and,
when available, a typed entity id, bounded filters, or explicit capability
hints. Sensitive keys and oversized filters are rejected. The context is
inserted as untrusted navigation state immediately before the current user
turn; it never grants filesystem access and is not a replacement for a typed
read tool.

Only schemas advertised for that turn may be requested by the provider. The
small deterministic pre-read router remains separately bounded and read-only.
The daemon validates tool
arguments against the catalog again at execution time, including required
fields, types, enums, bounds, and `additionalProperties`; provider output cannot
smuggle a hidden network or mutation argument into a narrower tool.

- `status`
- `ui_overview_snapshot` maps to daemon kind `ui.overview.snapshot`
- `ui_transactions_list` maps to daemon kind `ui.transactions.list` with
  bounded filters for `limit`, `direction`, `asset`, `wallet`, `since`, `sort`,
  and `order`
- `ui_transactions_extremes` maps to daemon kind
  `ui.transactions.extremes`; it returns the exact largest and smallest
  transactions after sorting before the limit
- `ui_transactions_search` maps to daemon kind `ui.transactions.search`; it
  searches safe transaction metadata such as ids, txids, wallet labels, notes,
  descriptions, counterparties, kinds, and tags
- `ui_transactions_history` maps to daemon kind `ui.transactions.history`; it
  returns bounded, redacted append-only metadata edit history for one
  transaction, including grouped pricing events and source attribution
- `ui_activity_history` maps to daemon kind `ui.activity.history`; it returns
  bounded, redacted global edit Activity with date, source, field-family,
  wallet, transaction, pricing-only, and AI-only filters
- `ui_wallets_list` maps to daemon kind `ui.wallets.list`
- `ui_wallets_utxos` maps to daemon kind `ui.wallets.utxos`; it returns one
  wallet's redacted watch-only UTXO inventory and source freshness, without
  wallet addresses, scriptPubKeys, branch labels, derivation indices,
  descriptors, xpubs, Silent Payments scan material, blinding keys, backend
  URLs/tokens, raw wallet config, or raw wallet files
- `ui_backends_list` maps to daemon kind `ui.backends.list`; it is scoped to
  backends referenced by the active books/profile and returns URL presence
  metadata, not exact endpoint URLs
- `ui_profiles_snapshot` maps to daemon kind `ui.profiles.snapshot`
- `ui_workspace_overview_snapshot` reads every book in the chat's original
  workspace only after an explicit book-set request. It preserves per-book
  boundaries and does not aggregate mixed fiat currencies.
- `ui_reports_capital_gains` maps to daemon kind `ui.reports.capital_gains`
- `ui_reports_summary` maps to daemon kind `ui.reports.summary`; it returns
  exact processed all-time summary totals, including asset and wallet
  inflow/outflow fields in BTC, sat, and msat, plus reviewed
  transfer/swap pair rows that explain paired movement inside raw flows
- `ui_reports_balance_sheet` maps to daemon kind `ui.reports.balance_sheet`;
  it returns exact processed current holdings by reporting bucket/account,
  including BTC, sat, msat, cost basis, market value, and unrealized PnL
- `ui_reports_portfolio_summary` maps to daemon kind
  `ui.reports.portfolio_summary`; it returns exact processed holdings by wallet
- `ui_reports_tax_summary` maps to daemon kind `ui.reports.tax_summary`; it
  returns exact processed tax-summary rows by year and asset
- `ui_reports_balance_history` maps to daemon kind
  `ui.reports.balance_history`; it returns processed balance-history buckets
  for trend questions
- `ui_reports_privacy_hygiene` maps to daemon kind
  `ui.reports.privacy_hygiene`; it returns the same redacted local-only
  privacy facts shown by Settings -> Privacy and `kassiber reports
  privacy-hygiene`, with `evidence_level` on findings and no addresses,
  scripts, descriptors, xpubs, backend URLs/tokens, wallet config, raw JSON,
  branch/index values, or derivation paths. The GUI may separately show
  operator-facing endpoint rows through backend settings permissions; the AI
  tool receives only this redacted payload.
- `ui_reports_privacy_mirror` maps to daemon kind
  `ui.reports.privacy_mirror`; it returns the redacted Privacy Mirror payload
  used by the dedicated page and `kassiber reports privacy-mirror`, including
  exposure summary, adversary cards, wallet/transaction/UTXO views, timeline,
  coverage, unknowns, evidence drilldowns, and the computed worst local privacy
  risk. It is read-only, local-only, advisory-only, and every result carries
  `evidence_level`. The AI tool does not receive raw PSBT text; PSBT preflight
  is reduced locally in the GUI/CLI before any assistant-facing summary can be
  discussed. See [`privacy-mirror.md`](privacy-mirror.md).
- `ui_journals_snapshot` maps to daemon kind `ui.journals.snapshot`; recent
  rows include reviewed pair context for swap/peg journal rows when available
- `ui_journals_quarantine` maps to daemon kind `ui.journals.quarantine`
- `ui_journals_events_list` maps to daemon kind `ui.journals.events.list`; it
  returns bounded processed journal events with transaction ids, Austrian
  category fields, reviewed pair context for swap/peg rows, and an optional
  transaction filter
- `ui_journals_transfers_list` maps to daemon kind
  `ui.journals.transfers.list`
- `ui_rates_summary` maps to daemon kind `ui.rates.summary`
- `ui_rates_coverage` maps to daemon kind `ui.rates.coverage`; it returns
  transaction pricing coverage, rows that still require a usable fiat spot
  price, and whether local rates-cache samples can cover those gaps
- `ui_rates_latest` is consent-gated and fetches one latest public market rate
  only when live-rate access is enabled for the active book
- `ui_rates_rebuild` maps to daemon kind `ui.rates.rebuild`; after consent it
  fetches missing provider spot-rate windows, clears provider-derived
  transaction prices, applies cache-backed prices, and attempts to reprocess
  journals. If journal processing is blocked by ledger or quarantine issues,
  the tool still returns the completed rate/price sync with a structured
  journal error instead of reporting the whole price sync as failed
- `ui_report_blockers` maps to daemon kind `ui.report.blockers`; it returns a
  deterministic report-readiness answer with blockers for missing scope,
  wallets, transactions, stale journals, quarantine, or missing prices
- `ui_audit_changes_since_last_answer` maps to daemon kind
  `ui.audit.changes_since_last_answer`; it answers whether transactions,
  metadata edits, wallets, journals, quarantines, or rates changed since an
  optional RFC3339 answer timestamp
- `ui_maintenance_settings` maps to daemon kind `ui.maintenance.settings`; it
  reads the active profile's AI maintenance settings
- `ui_workspace_health` maps to daemon kind `ui.workspace.health`
- `ui_next_actions` maps to daemon kind `ui.next_actions`
- `ui_transactions_resolve` and `ui_transactions_graph` expose the existing
  safe local lookup/graph surfaces without public-backend lookup
- `ui_transactions_review_context` maps to
  `ui.transactions.review_context`; it composes one bounded transaction row,
  local graph, journal events, edit history, evidence readiness, attachment
  labels, commercial context, source-funds links, privacy findings, and
  deterministic next actions. Each optional section degrades independently
  instead of making the whole packet fail.
- `ui_activity_stale`, `ui_attachments_list`,
  `ui_audit_evidence_summary`, and `ui_review_badges` expose local review and
  audit readiness. AI attachment/evidence lists are cursor-bounded and omit
  local paths and URL targets.
- `ui_review_worklist` combines bounded readiness blockers, quarantine, stale
  edits, transfer candidates, loan hints, and optional commercial/source-funds
  gaps into one deterministic local review queue
- `ui_loans_list` reads reviewed collateral/principal marks and open-lock hints;
  the latter are explicitly heuristic and never liquidation proof. Returned
  rows are bounded and the summary reports full counts/truncation.
- `ui_source_funds_sources_list` maps to daemon kind
  `ui.source_funds.sources.list`; attachment labels may be shown, but raw
  evidence URLs and stored attachment paths are redacted
- `ui_source_funds_links_list` maps to daemon kind
  `ui.source_funds.links.list`; pass `target_transaction` to inspect the
  reviewed/suggested provenance attached to one transaction without exposing
  raw evidence URLs or stored attachment paths
- `ui_source_funds_preview` maps to daemon kind `ui.source_funds.preview`; it
  returns a read-only path graph plus export gates for missing history,
  heuristic allocations, privacy-hop ambiguity, missing pricing, and other
  blockers before any PDF/export decision
- `ui_source_funds_evidence_list`, `ui_source_funds_coverage`, and
  `ui_source_funds_cases_list` complete the read side of the evidence workflow
- `ui_transactions_commercial_context`, `ui_btcpay_provenance_{list,suggest,links}`,
  and `ui_documents_list` expose redacted merchant/document reconciliation
  metadata; raw BTCPay payloads and document bytes stay local
- `ui_reports_exit_tax_preview` exposes the deterministic Austrian exit-tax
  preview
- `ui_egress_snapshot` returns only outbound counts/bytes by subsystem; it
  deliberately omits hosts, ports, backend identities, paths, headers, query
  strings, and request bodies from provider-bound content
- `ui_transfers_suggest` maps to daemon kind `ui.transfers.suggest`; it returns
  wallet-transfer candidates, Bitcoin swap/peg candidates, and other cross-asset
  swap candidates with confidence, method, computed fee, and conflict-cluster
  context without writing review decisions. Pass `candidate_type=transfer` for
  carrying-value Bitcoin movements (including Boltz/submarine swaps) or
  `candidate_type=swap` for other cross-asset swaps. Bitcoin swap review still
  requires ownership intent: if the swap route paid or received from an external
  counterparty, it should remain an ordinary payment or receipt.
- `ui_transfers_review_context` maps to daemon kind
  `ui.transfers.review_context`; it returns a bounded deterministic pair-review
  packet with candidate leg summaries, confidence reasons, fee assessment,
  conflict status, metadata clues, current journal impact if left unpaired,
  suggested next action, active pairs, rules, and saved candidate views. Pass
  `candidate_type=transfer` or `candidate_type=swap` when the review packet
  should follow one split queue; without a candidate type it includes both.
- `ui_transfers_list` maps to daemon kind `ui.transfers.list`; it returns active
  reviewed transfer/swap pairs
- `ui_transfers_payouts_list` returns reviewed direct/split payouts where the
  outbound leg is known but no inbound transaction was imported
- `ui_transfers_rules_list` maps to daemon kind `ui.transfers.rules.list`; it
  returns active auto-pair rules without applying them
- `ui_saved_views_list` maps to daemon kind `ui.saved_views.list`; it returns
  saved review-queue filters such as swap-candidate views
- `read_skill_reference`

`ui.workspace.health` summarizes the active books set and book
(`workspace`/`profile` internally), wallet and transaction counts,
journal freshness, quarantine count,
and report-readiness hints from the current database. `ui.next_actions` returns structured
recommendations such as create a wallet, sync/import, process journals, review
quarantine, or run reports. It only advises; it does not execute those actions.

`read_skill_reference` is a virtual tool. `read_skill_reference("index")`
returns a compact routing document derived from the Kassiber skill concepts and
points the model to deeper allowlisted references. The deeper references are
restricted to packaged files under `kassiber/ai/skill_references/`:
`command-templates`,
`journal-processing`, `metadata`, `onboarding`, `reports`,
`secrets-and-backup`, `swap-matching`, `troubleshooting`, `verification`, and
`wallets-backends`.

Mutating provider tools currently include `ui_wallets_sync`, which maps to
daemon kind `ui.wallets.sync`, `ui_journals_process`, which maps to
`ui.journals.process`, `ui_rates_latest`, which fetches one opted-in latest
public rate, `ui_rates_rebuild`, which refreshes provider spot prices
and reprocesses journals, `ui_maintenance_configure`, which changes
active-profile AI maintenance settings, and `ui_maintenance_run`, which runs
optional sync plus journal maintenance and returns report blockers. The same
consent path also
covers review-queue actions exposed to chat: `ui_transfers_pair`,
`ui_transfers_unpair`, `ui_transfers_bulk_pair`, `ui_transfers_dismiss`,
`ui_transfers_rules_create`, `ui_transfers_rules_delete`,
`ui_transfers_rules_set_enabled`, `ui_transfers_rules_apply`,
`ui_transfers_payouts_create`, `ui_transfers_payouts_delete`,
`ui_transfers_update`, `ui_saved_views_create`, and `ui_saved_views_delete`.
`ui_transfers_pair`
supports `coinjoin` and `whirlpool` kinds for user-reviewed same-asset
ownership hops, including reviewed one-to-many / many-to-one same-asset links.
Cross-asset and layer-transition links remain one-to-one. The AI may propose
these pairings, but the write still requires explicit user consent.
The desktop/CLI custody-component resolver handles 1:N, N:1, N:M, multi-hop,
and missing-wallet histories atomically; those authored component mutations are
not generic AI pairing shortcuts.
Source-funds evidence writes are also consent-gated:
`ui_source_funds_sources_create`,
`ui_source_funds_links_create`, `ui_source_funds_links_review`,
`ui_source_funds_suggest`, `ui_source_funds_links_bulk_review`,
`ui_source_funds_sources_attach`, and `ui_source_funds_links_attach`. Evidence
attach tools accept only existing managed attachment ids, never paths. These tools
create/review provenance evidence only; they do not mutate tax/journal
`transaction_pairs` and they support non-CoinJoin link types such as
self-transfer, exchange transfer, trade, swap, peg-in/peg-out, Lightning hops,
manual source, and missing-history edges. CoinJoin/PayJoin links should stay
explicit about privacy-hop ambiguity unless the user has reviewed stronger
evidence. `ui_source_funds_assemble`, `ui_source_funds_cases_save`, and the
virtual `ui_source_funds_export` complete that workflow; export results sent to
the model include the filename/format but not the managed local path.

Transaction review writes (`ui_transactions_metadata_update`,
`ui_transactions_history_revert`, `ui_attachments_copy`) preserve the
append-only edit/evidence audit trail and run only after consent. Commercial
review writes (`ui_btcpay_provenance_review`, `ui_documents_create`) are also
consent-gated. The virtual `ui_reports_export` maps a small report/format enum
onto existing deterministic PDF/XLSX/CSV/audit-package exporters and likewise
withholds managed paths from model context. Stale journals may also be
refreshed automatically before read/report tools as local maintenance. Wallet
sync before report reads is disabled by default; it runs automatically only
after `ui_maintenance_configure` enables that active-profile setting, or when
the user explicitly approves a maintenance/sync call. Tool-call arguments are
redacted before previews, stream events, auto-context entries, and tool-result
content are returned to the model/UI. When a model requests a mutating tool, the
daemon emits
`ai.chat.tool_consent_required` with a short summary and redacted argument
preview, then waits for:

Loan review writes (`ui_loans_mark`, `ui_loans_link`, `ui_loans_unmark`) are
also consent-gated and invalidate journals. Open-lock heuristics never create
marks automatically.

The daemon freezes the project/database and active workspace/profile ids when
the chat starts. Every read and approved mutation rechecks that scope on the
main SQLite thread immediately before execution. If the user switches projects
or books mid-turn, the operation fails with `stale_context`; history persistence
still targets the original book and never the newly active one.

```json
{
  "kind": "ai.tool_call.consent",
  "args": {
    "target_request_id": "<active ai.chat request_id>",
    "call_id": "<tool call id>",
    "decision": "allow_once"
  }
}
```

`decision` can be `allow_once`, `allow_session`, or `deny`. Session consent is
in-memory and lasts only for the current `ai.chat` request; it applies only to
subsequent calls to the same tool name in that chat. If the user denies or does
not respond before the consent timeout, the daemon feeds a tool result back to
the model with `ok: false` and `reason: "user_denied"` or
`"consent_timeout"`. Unknown tools still return `tool_not_allowed` and are not
executed.

The terminal `ai.chat` record includes a compact `provenance` object with the
provider/model, generation timestamp, local tool names used, journal refresh
status, sync-attempt status, successful versus denied tool attempts, and counts
learned from health/report-blocker tools. Denied calls never count as executed
or as cross-book disclosure. The GUI uses that object and the exact tool payloads to render source
chips beside the assistant answer, so small models can be checked against
program-derived facts.

## Remote inference

Remote inference should be an explicit choice, not the default.

If remote inference is needed, prefer a provider that documents encrypted
inference and attestation rather than a generic hosted model API. One example
is [Maple Proxy / Maple AI](https://blog.trymaple.ai/maple-proxy-documentation/),
which documents TEE-based encrypted inference behind a local proxy. Confirm
that any selected proxy exposes `/v1/responses` before configuring it.

Even then, users should make an intentional privacy decision before sending
accounting data off-device.

## Example usage with the Kassiber skill

Examples of prompt shapes that work well with an AI assistant using the Kassiber
skill:

- "Use the Kassiber skill to inspect my current books, list my wallets, and tell me whether journals need to be reprocessed before I trust the reports."
- "Use the Kassiber skill to import this Phoenix CSV into my existing wallet, re-run journals, and show me the summary report."
- "Use the Kassiber skill to find quarantined journal events, explain what is missing, and suggest the smallest fix."
- "Use the Kassiber skill to compare wallet balances, bucket allocations, and portfolio output for these books without doing your own arithmetic."
- "Find transactions tagged revenue, show my total inflow/outflow, largest transaction, current balance, and 2026 tax summary from tool output only."

Kassiber accounts are wallet/reporting buckets in the current product. AI
assistants should not recommend double-entry charts of accounts, automatic fee
expense postings, or external counterparty equity accounts unless a future
ledger design explicitly adds those behaviors.

## Planned AI-assisted workflows

These are directionally in scope, but should remain optional and review-gated:

- "Extract the key fields from this invoice PDF and suggest the most likely BTCPay settlement match."
- "Review these transactions and suggest likely transfer or swap pairs for human confirmation before journal processing."
- "Summarize which fields are missing from this document match and why confidence is low."

## Related files

- [Kassiber CLI Agent Skill](https://github.com/bitcoinaustria/kassiber-skill)
- [`../../kassiber/ai/client.py`](../../kassiber/ai/client.py)
- [`../../kassiber/ai/cli_client.py`](../../kassiber/ai/cli_client.py)
- [`../../kassiber/ai/contracts.py`](../../kassiber/ai/contracts.py)
- [`../plan/08-external-document-reconciliation.md`](../plan/08-external-document-reconciliation.md)
- [`../../SECURITY.md`](../../SECURITY.md)
