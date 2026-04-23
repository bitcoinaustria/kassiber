# AI Reference

Kassiber has two different AI-related layers:

- a repo-local skill bundle for AI coding and terminal assistants
- planned in-product AI help for OCR, extraction, and reconciliation workflows

These are related, but they are not the same thing.

## What exists today

Today, the shipped AI surface is the repo-local Kassiber skill in
[`../../skills/kassiber/`](../../skills/kassiber/).

That skill helps an AI assistant use the Kassiber CLI safely and correctly for:

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
a simple local API on `http://localhost:11434/api`.

Example:

```bash
ollama run qwen3.6:35b
```

Local testing so far has used `qwen3.6:35b` with good results for Kassiber-style
assistant flows. Smaller and less powerful models can still be useful for
narrower tasks, and should become more practical as Kassiber's prompts, skill
bundle, and workflows get tighter.

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
