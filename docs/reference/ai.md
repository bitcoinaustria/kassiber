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
  OpenAI-compatible endpoint, plus a parallel CLI surface
  (`kassiber ai providers …`, `kassiber ai models`, `kassiber ai chat`) that
  reuses the same provider config.

The repo-local skill helps an AI assistant use the Kassiber CLI safely and
correctly for:

- onboarding and context checks
- wallet setup and imports
- journal processing
- reports
- metadata cleanup
- troubleshooting

The core accounting workflow does not depend on AI. Wallet sync, imports,
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
kassiber ai providers create openai --base-url https://api.openai.com/v1 --kind remote --acknowledge --api-key $OPENAI_API_KEY --default-model gpt-4o-mini
kassiber ai providers set-default openai
kassiber ai models
kassiber ai chat "Summarise the last week of imports."
```

API keys are stored in plaintext in the SQLite database for now, mirroring the
existing `backends` pattern. An OS-keychain migration is tracked in
[`../../TODO.md`](../../TODO.md).

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
and reports the model count without persisting anything.

Remote and TEE providers require explicit acknowledgement before chat. The CLI
uses `kassiber ai providers update <name> --acknowledge` (or
`--acknowledge` during `create`), and the desktop Settings form prompts before
saving an off-device provider. Without that acknowledgement, `ai.chat` returns
`ai_remote_ack_required` before sending any prompt content.

Streaming is demuxed by `request_id`: the Tauri supervisor keeps one daemon
process and one stdout reader, but routes each JSON envelope to the matching
request. While a chat is streaming, unrelated daemon calls can complete
independently.

Before the first token arrives, `ai.chat` may emit `ai.chat.status` records
with phases such as `preparing`, `connecting`, and `waiting_for_model`. These
records are UI progress hints only; chain-of-thought is shown only when the
provider emits inline `<think>` content or structured `reasoning` deltas.

Pressing **Stop** sends `ai.chat.cancel` with
`args.target_request_id = <active ai.chat request_id>`. Cancellation is
best-effort and cooperative: Kassiber stops forwarding deltas once the Python
worker returns between provider chunks, then emits the terminal `ai.chat`
envelope with `finish_reason: "cancelled"`. For metered remote providers, any
tokens already generated or in flight may still be billed.

## Tool use

The in-app assistant can opt into a bounded tool loop with
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
- `ui_wallets_list` maps to daemon kind `ui.wallets.list`
- `ui_backends_list` maps to daemon kind `ui.backends.list`; it is scoped to
  backends referenced by the active books/profile and returns URL presence
  metadata, not exact endpoint URLs
- `ui_profiles_snapshot` maps to daemon kind `ui.profiles.snapshot`
- `ui_reports_capital_gains` maps to daemon kind `ui.reports.capital_gains`
- `ui_journals_snapshot` maps to daemon kind `ui.journals.snapshot`
- `ui_journals_quarantine` maps to daemon kind `ui.journals.quarantine`
- `ui_journals_transfers_list` maps to daemon kind
  `ui.journals.transfers.list`
- `ui_rates_summary` maps to daemon kind `ui.rates.summary`
- `ui_workspace_health` maps to daemon kind `ui.workspace.health`
- `ui_next_actions` maps to daemon kind `ui.next_actions`
- `read_skill_reference`

`ui.workspace.health` summarizes the active ledger/books (`workspace`/`profile`
internally), wallet and transaction counts, journal freshness, quarantine count,
and report-readiness hints from the current database. `ui.next_actions` returns structured
recommendations such as create a wallet, sync/import, process journals, review
quarantine, or run reports. It only advises; it does not execute those actions.

`read_skill_reference` is a virtual tool. `read_skill_reference("index")`
returns a compact routing document derived from the Kassiber skill concepts and
points the model to deeper allowlisted references. The deeper references are
restricted to files under `skills/kassiber/references/`: `command-templates`,
`journal-processing`, `metadata`, `onboarding`, `reports`,
`secrets-and-backup`, `troubleshooting`, `verification`, and
`wallets-backends`.

Mutating provider tools currently include `ui_wallets_sync`, which maps to
daemon kind `ui.wallets.sync`, and `ui_journals_process`, which maps to
`ui.journals.process`. When a model requests one, the daemon emits
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

- "Use the Kassiber skill to inspect my current ledger and books, list my wallets, and tell me whether journals need to be reprocessed before I trust the reports."
- "Use the Kassiber skill to import this Phoenix CSV into my existing wallet, re-run journals, and show me the summary report."
- "Use the Kassiber skill to find quarantined journal events, explain what is missing, and suggest the smallest fix."
- "Use the Kassiber skill to compare wallet balances, bucket allocations, and portfolio output for these books without doing your own arithmetic."

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
