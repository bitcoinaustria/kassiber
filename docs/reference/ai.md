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
(`daemon://stream`) so the UI can render reasoning (`<think>`) and the answer
in real time.

Provider configuration is mirrored in the CLI:

```bash
kassiber ai providers list
kassiber ai providers create openai --base-url https://api.openai.com/v1 --kind remote --api-key $OPENAI_API_KEY --default-model gpt-4o-mini
kassiber ai providers set-default openai
kassiber ai models
kassiber ai chat "Summarise the last week of imports."
```

API keys are stored in plaintext in the SQLite database for now, mirroring the
existing `backends` pattern. An OS-keychain migration is tracked in
[`../../TODO.md`](../../TODO.md).

`<think>...</think>` content emitted inline by Qwen3, DeepSeek-R1, QwQ, and
similar thinking-capable models is split out into a collapsible reasoning pane
above the answer. Models that don't emit thinking tags pass through unchanged.

Streaming is one-shot: pressing **Stop** hides the in-flight assistant message
in the UI, but the underlying request keeps generating until it finishes.
Cooperative cancellation lands with the worker-pool refactor.

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

- "Use the Kassiber skill to inspect my current workspace and profile, list my wallets, and tell me whether journals need to be reprocessed before I trust the reports."
- "Use the Kassiber skill to import this Phoenix CSV into my existing wallet, re-run journals, and show me the summary report."
- "Use the Kassiber skill to find quarantined journal events, explain what is missing, and suggest the smallest fix."
- "Use the Kassiber skill to compare wallet balances, bucket allocations, and portfolio output for this profile without doing your own arithmetic."

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
