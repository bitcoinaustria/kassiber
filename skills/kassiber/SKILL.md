---
name: kassiber
description: Use this skill when the user wants to use the Kassiber CLI for local-first Bitcoin accounting, wallet onboarding, transaction imports, journal processing, metadata cleanup, or tax and portfolio reports. Applies to requests about Kassiber workspaces, profiles, accounts, wallets, backends, rates, attachments, BIP329 labels, quarantines, generic tax reporting, and Austrian-support planning/questions, even when the user does not say Kassiber by name.
---

# Kassiber

Use this skill for Kassiber CLI workflows. Kassiber has its own command surface, so agents should not guess flags or reuse commands from analogous tools.

All `scripts/` paths in this skill are relative to the directory containing this `SKILL.md` file. Resolve `<skill-dir>` first, then use paths like `<skill-dir>/scripts/verify-state.sh`.

Kassiber accounts are wallet/reporting buckets, not a double-entry chart of accounts. Keep explanations simple unless the user explicitly asks for accounting theory.

## Fast Paths

Use these without opening extra references when the request clearly matches:

| User asks for... | First command |
|---|---|
| Sync project/current wallets | `kassiber --machine wallets sync --all` |
| Current balances by account, bucket, asset, or wallet | `kassiber --format plain reports balance-sheet` |
| Exact summary totals, counts, fees, PnL | `kassiber --machine reports summary` |
| Rebuild stale reports after imports/metadata/rates | `kassiber --machine journals process` |
| Largest inbound transactions | `kassiber --machine transactions list --direction inbound --sort amount --order desc --limit 10` |
| Largest outbound transactions | `kassiber --machine transactions list --direction outbound --sort amount --order desc --limit 10` |
| Smallest inbound transactions | `kassiber --machine transactions list --direction inbound --sort amount --order asc --limit 10` |
| Smallest outbound transactions | `kassiber --machine transactions list --direction outbound --sort amount --order asc --limit 10` |

If a fast-path command returns a structured error, inspect the envelope and take the hinted next step. For example, stale reports usually mean running `kassiber --machine journals process` once, then retrying the same report.

## Rules

1. Prefer `kassiber` when it is on `PATH`. If it is not, fall back to `uv run kassiber` or `uv run python -m kassiber` from the Kassiber repo root.
2. When falling back from `kassiber` to `uv run kassiber` or `uv run python -m kassiber`, keep the subcommand, flags, and operands identical. Only the launcher changes.
3. When the chat includes pasted Kassiber output or docs, identify the live user request separately from the quoted material before running commands.
4. Use fast paths for common workflows. Read the relevant reference file only when command shape is unclear, the action mutates durable config, secrets are involved, or the request is outside the fast-path table.
5. Before concluding a reference is missing, verify that you resolved it from `<skill-dir>` rather than the repo root or the current working directory.
6. If a Kassiber command fails with `unrecognized arguments`, stop and check `--help` or [references/command-templates.md](references/command-templates.md) before retrying. Do not keep guessing positional versus flagged forms.
7. `--machine`, `--format`, and `--output` are global flags and must come before the subcommand tree, for example `kassiber --format plain reports balance-sheet`.
8. Use `--machine` whenever the output needs to be parsed or piped into later steps.
9. Use `--format plain` when the user wants report output shown in the terminal. Let Kassiber format financial values; do not recompute or restyle them.
10. Use `--format csv --output <path>` for spreadsheet-style exports.
11. Never perform your own arithmetic on Kassiber financial values. Do not sum, subtract, average, or convert amounts from raw JSON when Kassiber already has a command or output format for the answer.
12. For current balances by account, bucket, asset, or wallet, use `kassiber --format plain reports balance-sheet` first. Do not detour through `reports summary`.
13. For rollups like totals, fees, counts, realized gains, unrealized PnL, or "give me the summary", use `kassiber --machine reports summary` first. Quote Kassiber's returned fields directly.
14. If Kassiber returns both BTC and `*_msat` fields, quote those fields as-is. Never derive one from the other in your response.
15. If the exact answer is not exposed by an existing Kassiber command, say that the CLI surface is missing it and stop there instead of approximating.
16. For "largest transaction", "smallest transaction", biggest inbound/outbound, or similar raw transaction rankings, use `kassiber --machine transactions list --sort amount --order desc|asc --direction inbound|outbound --limit <n>` so SQLite ranks the full dataset before limiting. Amounts are unsigned and direction is a separate field, so "largest inbound" and "largest outbound" both use `--order desc`; "smallest" uses `--order asc`. Do not fetch the default recent page and sort it client-side. If a machine response includes `next_cursor`, keep following it only when the user asked for the full list; a correctly sorted first row is already the largest/smallest row for top-N questions.
17. Processing order is: wallet sync or import -> review likely transfer / swap pairs when relevant -> `kassiber rates sync` when pricing is needed -> `kassiber journals process` -> reports.
18. Re-run `kassiber journals process` after any transaction import, transfer pairing, note or tag change, exclusion change, rate sync, or rate override before trusting reports.
19. Do not confuse `kassiber init` with onboarding. It only creates the local state tree; workspace, profile, account, and wallet records are created with their own commands.
20. Prefer explicit workspace and profile flags until context is verified; use `kassiber context show` or `kassiber status` before assuming the active scope.
21. For Liquid descriptor wallets, require an explicit backend and private blinding keys. If either is missing, stop and fix that before sync.
22. If the user already provided a secret-bearing descriptor or token, do not ask them to paste it again and do not quote it back in summaries. Use a local file or direct CLI input once, then rely on allowlisted safe views after creation.
23. For wallet-connection setup, ask for the wallet or backend type if it is unclear, but otherwise assume a mainnet connection. Only ask about network when the user explicitly says testnet, signet, regtest, or another non-mainnet environment.
24. For secret-bearing wallet or backend setup, prefer giving the user a paste-ready local command template with placeholders to fill in on their machine instead of collecting descriptors, API tokens, or cookie values in chat.
25. If a BTCPay or CSV export belongs to the same real wallet as an existing Kassiber wallet, import it into that wallet instead of creating a duplicate wallet record.
26. On errors, inspect the machine envelope first. Kassiber success responses are `{kind, schema_version, data}` and errors use `kind: "error"` with structured fields.
27. Treat normal `backends ...` and `wallets ...` success output as safe-to-record only for secret-bearing config values. Do not ask users to paste raw backend credentials, raw private descriptor material, or suppressed config blobs into chat just because `backends get` or `wallets get` returns an allowlisted safe view.
28. For BTCPay and other secret-bearing backends, do not ask users to paste raw API tokens into chat. Prefer a local shell variable, a local `backends.env` entry, or a command they run locally with the secret substituted on their machine.
29. Do not persist backend or wallet config changes just to work around a sync failure unless the user requested that mutation or explicitly agrees after you explain the tradeoff. `wallets update --backend ...`, `--gap-limit`, and `backends set-default` change durable state.
30. Never claim a BTC ↔ LBTC swap is already paired, carrying-value, or reflected in reports unless `kassiber --machine journals transfers list` shows the pair or `kassiber transfers pair` just succeeded and you reprocessed journals.
31. When quarantines remain, distinguish processed holdings from raw transaction-net estimates. Reports show processed journal state only; any netting from `transactions list` must be labeled as an approximate diagnostic rather than a Kassiber holding.
32. For rate coverage, do not infer the covered time window from `samples` or `days` alone. Use `kassiber rates range` with RFC3339 timestamps around the missing transactions.
33. Treat Kassiber accounts as wallet/reporting buckets. Do not recommend double-entry charts of accounts, automatic fee expense postings, or external equity counterpart accounts unless the product gains an explicit ledger model.
34. For planning or codebase work, treat `TODO.md` as the executable backlog and `docs/plan/` as orientation/guardrails. Verify current behavior against code before acting on a plan doc.

## Gotchas

- Empty or stale reports usually mean journals have not been processed since the last change.
- Reports do not auto-pair BTC ↔ LBTC peg-ins / peg-outs or submarine swaps. If the user has cross-asset swap activity, inspect for likely pairs and use `kassiber transfers pair` before trusting reports.
- Use `kassiber --machine journals transfers list` when you need the exact transfer / swap links Kassiber computed. Do not infer them from `journals process` counts alone.
- `--machine` implies JSON mode. Use it alone or with `--format json`; do not combine it with any other explicit `--format` value.
- `wallets sync` uses either `--wallet <label-or-id>` or `--all`, never both; `transactions` needs the `list` subcommand and ranks raw rows with `--sort amount --order asc|desc`; `journals quarantined` has no `--limit`; `rates range --start/--end` expect RFC3339 UTC strings and supports `--order asc|desc`.
- `backends get/list` and `wallets get/create/update` intentionally return allowlisted safe views. Look for presence and state flags instead of expecting raw credentials, raw descriptors, or arbitrary config keys back.
- For new wallet connections, default to mainnet unless the user says otherwise, and hand back a paste-ready local command template rather than asking them to type secrets into chat.
- Quarantined transactions are omitted from accurate downstream reporting until resolved or excluded.
- Paginated list commands keep rows under command-specific keys such as `.data.records` and `.data.events`. Do not assume every list response uses the same field name.
- Follow `next_cursor` only when the user asks for all/full/export/audit output. For top-N, largest/smallest, or summary questions, stop after the correctly sorted first page.
- Cross-asset `--policy carrying-value` pairing is Austrian-only right now. Outside Austrian profiles, BTC ↔ LBTC manual pairs still stay on the normal SELL + BUY path, so do not describe them as carrying-value.
- `kassiber status` may resolve to a legacy XDG path on machines with older state trees. Use status output, not assumptions, to find the live database.
- If `journals transfers list` reports `cross_asset_pairs: 0`, no cross-asset swap pair is active yet. Do not describe Austrian carry-value as already applied until that changes.
- Large `rates sync --days ...` requests may still yield limited history because the upstream source can cap the returned window. Verify actual coverage with `rates range` instead of hand-mathing sample counts.
- If a skill reference lookup fails, the most common mistake is resolving `references/...` from repo root instead of `<skill-dir>/references/...`.
- Kassiber already has `reports export-pdf`; do not invent bespoke render scripts unless the user specifically wants a custom format beyond the built-in export.
- Accounts are not a double-entry chart of accounts today. `account_type` and `asset` are descriptive bucket metadata; fees and external counterparties do not auto-post to separate accounts.

## Data Model

Kassiber organizes data as:

`workspace -> profile -> account buckets + wallets -> transactions -> journals -> reports`

Related notes:

- `workspace` is the top-level container for an organization, person, or set of books.
- `profile` is one accounting and tax scope inside a workspace.
- `wallet` is a transaction source that Kassiber syncs or imports; map it to the real underlying wallet, not every external store or export.
- `account` is a wallet/reporting bucket that wallets can belong to.
- `backends` define sync transport endpoints.
- `metadata` covers notes, tags, exclusions, and BIP329 labels.
- `attachments` are managed separately from wallet config and transaction rows.
- Cost basis is pooled per asset across all wallets in a profile.
- Balance-sheet output groups holdings by the wallet's assigned bucket, not by account-type rollups or counterpart postings.
- If multiple BTCPay stores point at the same real wallet, keep them in one Kassiber wallet or holdings will be duplicated.

## Workflow Routing

- For fragile CLI command shapes and safe invocation patterns, read [references/command-templates.md](references/command-templates.md).
- For first-run setup, roots, context, and profile creation, read [references/onboarding.md](references/onboarding.md).
- For wallet kinds, descriptor setup, backend selection, and imports, read [references/wallets-backends.md](references/wallets-backends.md).
- For journal processing, quarantine handling, and transfer pairing, read [references/journal-processing.md](references/journal-processing.md).
- For notes, tags, exclusions, BIP329 labels, and attachments, read [references/metadata.md](references/metadata.md).
- For balance sheet, portfolio, capital gains, balance history, PDF export, and rates, read [references/reports.md](references/reports.md).
- For quick state checks and smoke validation, read [references/verification.md](references/verification.md) and use `scripts/verify-state.sh` when helpful.
- For common failure modes and path confusion, read [references/troubleshooting.md](references/troubleshooting.md).

## Report Selection

| User is asking about... | Report |
|---|---|
| Cost basis, unrealized gains, portfolio value, average cost, wallet allocation | `portfolio-summary` |
| Realized gains/losses, disposals, tax reporting, capital gains | `capital-gains` |
| Yearly gain/loss buckets, long vs short summary, tax totals by year | `tax-summary` |
| Current balances by bucket, asset, or wallet | `balance-sheet` |
| Balance changes over time, trends, history | `balance-history` |
| Raw ledger export, journal rows, bookkeeping output | `journal-entries` |
| Exact rollups: fees, counts, totals, realized/unrealized summary | `summary` |

## Fallback

If command shape is unclear, consult:

```bash
kassiber --help
kassiber <command> --help
kassiber <command> <subcommand> --help
```
