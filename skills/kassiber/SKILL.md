---
name: kassiber
description: Use this skill when the user wants to use the Kassiber CLI for local-first Bitcoin accounting, wallet onboarding, transaction imports, journal processing, metadata cleanup, or tax and portfolio reports. Applies to requests about Kassiber workspaces, profiles, accounts, wallets, backends, rates, attachments, BIP329 labels, quarantines, generic tax reporting, and Austrian-support planning/questions, even when the user does not say Kassiber by name.
---

# Kassiber

Use this skill for Kassiber CLI workflows. Kassiber has its own command surface, so agents should not guess flags or reuse commands from analogous tools.

All `scripts/` paths in this skill are relative to the directory containing this `SKILL.md` file. Resolve `<skill-dir>` first, then use paths like `<skill-dir>/scripts/verify-state.sh`.

## Rules

1. Prefer `kassiber` when it is on `PATH`. If it is not, fall back to `uv run kassiber` or `uv run python -m kassiber` from the Kassiber repo root.
2. Always read the relevant reference file before running a non-trivial Kassiber command. Do not guess flags from memory when a reference exists.
3. `--machine`, `--format`, and `--output` are global flags and must come before the subcommand tree, for example `kassiber --format plain reports balance-sheet`.
4. Use `--machine` whenever the output needs to be parsed or piped into later steps.
5. Use `--format plain` when the user wants report output shown in the terminal. Let Kassiber format financial values; do not recompute or restyle them.
6. Use `--format csv --output <path>` for spreadsheet-style exports.
7. Never perform your own arithmetic on Kassiber financial values. Do not sum, subtract, average, or convert amounts from raw JSON when Kassiber already has a command or output format for the answer.
8. For rollups like totals, fees, counts, realized gains, unrealized PnL, or "give me the summary", use `kassiber --machine reports summary` first. Quote Kassiber's returned fields directly.
9. If Kassiber returns both BTC and `*_msat` fields, quote those fields as-is. Never derive one from the other in your response.
10. If the exact answer is not exposed by an existing Kassiber command, say that the CLI surface is missing it and stop there instead of approximating.
11. Processing order is: wallet sync or import -> review likely transfer / swap pairs when relevant -> `kassiber rates sync` when pricing is needed -> `kassiber journals process` -> reports.
12. Re-run `kassiber journals process` after any transaction import, transfer pairing, note or tag change, exclusion change, or rate override before trusting reports.
13. Do not confuse `kassiber init` with onboarding. It only creates the local state tree; workspace, profile, account, and wallet records are created with their own commands.
14. Prefer explicit workspace and profile flags until context is verified; use `kassiber context show` or `kassiber status` before assuming the active scope.
15. For Liquid descriptor wallets, require an explicit backend and private blinding keys. If either is missing, stop and fix that before sync.
16. If a BTCPay or CSV export belongs to the same real wallet as an existing Kassiber wallet, import it into that wallet instead of creating a duplicate wallet record.
17. On errors, inspect the machine envelope first. Kassiber success responses are `{kind, schema_version, data}` and errors use `kind: "error"` with structured fields.
18. Treat normal `backends ...` and `wallets ...` success output as safe-to-record only for secret-bearing config values. Do not ask users to paste raw backend credentials, raw private descriptor material, or suppressed config blobs into chat just because `backends get` or `wallets get` returns an allowlisted safe view.

## Gotchas

- Empty or stale reports usually mean journals have not been processed since the last change.
- Reports do not auto-pair BTC ↔ LBTC peg-ins / peg-outs or submarine swaps. If the user has cross-asset swap activity, inspect for likely pairs and use `kassiber transfers pair` before trusting reports.
- Use `kassiber --machine journals transfers list` when you need the exact transfer / swap links Kassiber computed. Do not infer them from `journals process` counts alone.
- `--machine` implies JSON mode. Use it alone or with `--format json`; do not combine it with any other explicit `--format` value.
- `backends get/list` and `wallets get/create/update` intentionally return allowlisted safe views. Look for presence and state flags instead of expecting raw credentials, raw descriptors, or arbitrary config keys back.
- Quarantined transactions are omitted from accurate downstream reporting until resolved or excluded.
- Paginated list commands keep rows under command-specific keys such as `.data.records` and `.data.events`. Do not assume every list response uses the same field name.
- For paginated responses like `journals events list` and `metadata records list`, always follow `next_cursor` until it is `null`.
- Cross-asset `--policy carrying-value` pairing is Austrian-only right now. For other tax countries, BTC ↔ LBTC swaps still need `--policy taxable` and remain on the normal SELL + BUY path.
- `kassiber status` may resolve to a legacy XDG path on machines with older state trees. Use status output, not assumptions, to find the live database.
- Kassiber already has `reports export-pdf`; do not invent bespoke render scripts unless the user specifically wants a custom format beyond the built-in export.

## Data Model

Kassiber organizes data as:

`workspace -> profile -> accounts + wallets -> transactions -> journals -> reports`

Related notes:

- `backends` define sync transport endpoints.
- `metadata` covers notes, tags, exclusions, and BIP329 labels.
- `attachments` are managed separately from wallet config and transaction rows.
- Cost basis is pooled per asset across all wallets in a profile.

## Workflow Routing

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
| Current balances by account, asset, or wallet | `balance-sheet` |
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
