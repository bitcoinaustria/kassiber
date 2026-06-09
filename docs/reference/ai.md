# AI Reference

Kassiber has three AI-related layers:

- a repo-local skill bundle for AI coding and terminal assistants
- the in-app assistant that ships with the desktop UI (and a CLI surface that
  reuses the same provider config)
- planned in-product AI help for OCR, extraction, and reconciliation workflows

These are related, but they are not the same thing.

## What exists today

Two surfaces ship today:

- The repo-local Kassiber skill in
  [`../../skills/kassiber/`](../../skills/kassiber/) for AI coding and terminal
  assistants.
- An **in-app assistant** in the desktop UI that streams chat from an
  OpenAI-compatible endpoint or fixed Claude/Codex CLI adapter, plus a
  parallel CLI surface (`kassiber chat`, `kassiber ai providers …`,
  `kassiber ai models`) that reuses the same provider config.

The repo-local skill helps an AI assistant use the Kassiber CLI safely and
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
guarantee. Do not paste raw credentials, raw private descriptors, wallet
exports, or other sensitive material into a remote model unless that is
acceptable for your threat model.

If in doubt, keep inference local.

Claude CLI and Codex CLI are supported for convenience, but they are not a
local-privacy guarantee. Kassiber launches them in a narrow non-interactive mode
that still uses their normal local authentication/config, telemetry, and
model-provider routing. Treat them as off-device unless your local CLI setup is
explicitly backed by a local or confidential provider.

## Recommended inference setup

Local inference is the recommended default.

[Ollama](https://ollama.com/) is a good fit because it runs locally and exposes
an OpenAI-compatible API at `http://localhost:11434/v1`. The first time the
in-app assistant or CLI is invoked, Kassiber seeds a default `ollama` provider
pointing at that endpoint. Run `ollama serve` (or have Ollama auto-start) and
the assistant Just Works.

Example:

```bash
ollama run qwen3.6:35b
```

Local testing so far has used `qwen3.6:35b` with good results for Kassiber-style
assistant flows. Smaller and less powerful models can still be useful for
narrower tasks, and should become more practical as Kassiber's prompts, skill
bundle, and workflows get tighter.

Claude CLI and Codex CLI can be added with fixed provider locators:

```bash
kassiber ai providers create claude-cli --base-url claude-cli://default --kind remote --acknowledge --default-model default
kassiber ai providers create codex-cli --base-url codex-cli://default --kind remote --acknowledge --default-model default
```

For these providers, `--model` / `default_model` is forwarded to the CLI when it
is not `default`. The assistant's thinking selector sends `reasoning_effort` for
OpenAI-compatible providers, maps to Claude CLI `--effort`, and maps to Codex
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

- default rendered text for humans. With piped stdout, the answer text is the
  only thing on stdout — progress labels, tool announcements, consent UI, and
  the provenance footer move to stderr;
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

Reasoning-capable models surface chain-of-thought through one of two
channels, and both are split into a collapsible reasoning pane above the
answer:

- Inline `<think>...</think>` tags inside the content stream — emitted by
  DeepSeek-R1 and QwQ.
- A structured `reasoning` field on the delta — emitted by OpenAI o1/o3
  and by Ollama's OpenAI-compat shim for Qwen3 / Gemma reasoning builds.

Models that don't emit either pass through unchanged.

Settings → AI providers exposes a **Test connection** action. It calls the
daemon's `ai.test_connection` kind with the *currently entered* base URL and
API key (or, when editing without changing the API-key field, the saved key)
and reports the model count without persisting anything. For Claude/Codex CLI
locators, this only verifies that the CLI executable is present; authentication
and model reachability are checked when chat starts.

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
records are UI progress hints only; chain-of-thought is shown only when the
provider emits inline `<think>` content or structured `reasoning` deltas.

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
sends OpenAI-style tool definitions, emits `ai.chat.tool_call`,
`ai.chat.tool_consent_required`, and
`ai.chat.tool_result` stream records as needed, feeds tool results back as
`role: "tool"` messages, and finishes with the normal terminal `ai.chat`
envelope.

Before the provider is called, Kassiber also runs a small deterministic
read-only router for Kassiber questions. It looks for common accounting intents
such as pending work, sync readiness, totals, inflow/outflow, balances, tax
summaries, largest/smallest transactions, transaction search, quarantine,
transfers, swap-review context, saved review filters, auto-pair rules, and pricing.
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

The in-app prompt is a digest, not a full dump of
`skills/kassiber/SKILL.md`. It teaches the model the local-first accounting
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
  wallet addresses, derivation indices, descriptors, xpubs, blinding keys,
  backend URLs/tokens, raw wallet config, or raw wallet files
- `ui_backends_list` maps to daemon kind `ui.backends.list`; it is scoped to
  backends referenced by the active books/profile and returns URL presence
  metadata, not exact endpoint URLs
- `ui_profiles_snapshot` maps to daemon kind `ui.profiles.snapshot`
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
- `ui_journals_snapshot` maps to daemon kind `ui.journals.snapshot`; recent
  rows include reviewed pair context for swap/peg journal rows when available
- `ui_journals_quarantine` maps to daemon kind `ui.journals.quarantine`
- `ui_journals_events_list` maps to daemon kind `ui.journals.events.list`; it
  returns bounded processed journal events with transaction ids, Austrian
  category fields, and reviewed pair context for swap/peg rows
- `ui_journals_transfers_list` maps to daemon kind
  `ui.journals.transfers.list`
- `ui_rates_summary` maps to daemon kind `ui.rates.summary`
- `ui_rates_coverage` maps to daemon kind `ui.rates.coverage`; it returns
  transaction pricing coverage, rows that still require a usable fiat spot
  price, and whether local rates-cache samples can cover those gaps
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
- `ui_transfers_suggest` maps to daemon kind `ui.transfers.suggest`; it returns
  same-asset transfer candidates and cross-asset swap/peg candidates with
  confidence, method, computed fee, and conflict-cluster context without writing
  review decisions. Pass `candidate_type=transfer` or `candidate_type=swap` to
  keep those queues separate.
- `ui_transfers_review_context` maps to daemon kind
  `ui.transfers.review_context`; it returns a bounded deterministic swap-review
  packet with candidate leg summaries, confidence reasons, fee assessment,
  conflict status, metadata clues, current journal impact if left unpaired,
  suggested next action, active pairs, rules, and saved swap-candidate views.
  Pass `candidate_type=transfer` or `candidate_type=swap` when the review packet
  should follow the split queues.
- `ui_transfers_list` maps to daemon kind `ui.transfers.list`; it returns active
  reviewed transfer/swap pairs
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
restricted to files under `skills/kassiber/references/`: `command-templates`,
`journal-processing`, `metadata`, `onboarding`, `reports`,
`secrets-and-backup`, `swap-matching`, `troubleshooting`, `verification`, and
`wallets-backends`.

Mutating provider tools currently include `ui_wallets_sync`, which maps to
daemon kind `ui.wallets.sync`, `ui_journals_process`, which maps to
`ui.journals.process`, `ui_rates_rebuild`, which refreshes provider spot prices
and reprocesses journals, `ui_maintenance_configure`, which changes
active-profile AI maintenance settings, and `ui_maintenance_run`, which runs
optional sync plus journal maintenance and returns report blockers. The same
consent path also
covers review-queue actions exposed to chat: `ui_transfers_pair`,
`ui_transfers_unpair`, `ui_transfers_bulk_pair`, `ui_transfers_dismiss`,
`ui_transfers_rules_create`, `ui_transfers_rules_delete`,
`ui_transfers_rules_set_enabled`, `ui_transfers_rules_apply`,
`ui_saved_views_create`, and `ui_saved_views_delete`. Stale journals may also be
refreshed automatically before read/report tools as local maintenance. Wallet
sync before report reads is disabled by default; it runs automatically only
after `ui_maintenance_configure` enables that active-profile setting, or when
the user explicitly approves a maintenance/sync call. Tool-call arguments are
redacted before previews, stream events, auto-context entries, and tool-result
content are returned to the model/UI. When a model requests a mutating tool, the
daemon emits
`ai.chat.tool_consent_required` with a short summary and redacted argument
preview, then waits for:

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
status, sync-attempt status, and counts learned from health/report-blocker
tools. The GUI uses that object and the exact tool payloads to render source
chips beside the assistant answer, so small models can be checked against
program-derived facts.

## Remote inference

Remote inference should be an explicit choice, not the default.

If remote inference is needed, prefer a provider that documents encrypted
inference and attestation rather than a generic hosted model API. One example
is [Maple Proxy / Maple AI](https://blog.trymaple.ai/maple-proxy-documentation/),
which documents TEE-based encrypted inference behind an OpenAI-compatible local
proxy.

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

- [`../../skills/kassiber/SKILL.md`](../../skills/kassiber/SKILL.md)
- [`../plan/08-external-document-reconciliation.md`](../plan/08-external-document-reconciliation.md)
- [`../../SECURITY.md`](../../SECURITY.md)
