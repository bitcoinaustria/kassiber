---
name: kassiber
description: Use this skill when the user wants to use the Kassiber CLI for local-first Bitcoin accounting, wallet onboarding, transaction imports, journal processing, metadata cleanup, or tax and portfolio reports. Applies to requests about Kassiber workspaces, profiles, accounts, wallets, backends, rates, attachments, BIP329 labels, quarantines, and Austrian or generic tax reporting.
---

# Kassiber

Use this skill for Kassiber CLI workflows. Kassiber is close in shape to Clams, but the command surface is different enough that agents should not guess flags or reuse Clams commands.

## Rules

1. Prefer `kassiber` when it is on `PATH`. If it is not, fall back to `uv run kassiber` or `uv run python -m kassiber` from the Kassiber repo root.
2. Use `--machine` whenever the output needs to be parsed or piped into later steps.
3. Use `--format plain` when the user wants report output shown in the terminal. Let Kassiber format financial values; do not recompute or restyle them.
4. Use `--format csv --output <path>` for spreadsheet-style exports.
5. Processing order is: wallet sync or import -> `kassiber rates sync` when pricing is needed -> `kassiber journals process` -> reports.
6. Re-run `kassiber journals process` after any transaction import, transfer pairing, note or tag change, exclusion change, or rate override before trusting reports.
7. Do not confuse `kassiber init` with onboarding. It only creates the local state tree; workspace, profile, account, and wallet records are created with their own commands.
8. Prefer explicit workspace and profile flags until context is verified; use `kassiber context show` or `kassiber status` before assuming the active scope.
9. For Liquid descriptor wallets, require an explicit backend and private blinding keys. If either is missing, stop and fix that before sync.
10. If a BTCPay or CSV export belongs to the same real wallet as an existing Kassiber wallet, import it into that wallet instead of creating a duplicate wallet record.
11. On errors, inspect the machine envelope first. Kassiber success responses are `{kind, schema_version, data}` and errors use `kind: "error"` with structured fields.

## Gotchas

- Empty or stale reports usually mean journals have not been processed since the last change.
- Quarantined transactions are omitted from accurate downstream reporting until resolved or excluded.
- `Altbestand` is wallet-level provenance metadata, not a profile-level setting. Split mixed holdings into separate wallet records when the history can be separated cleanly.
- `kassiber status` may resolve to a legacy XDG path on machines with older state trees. Use status output, not assumptions, to find the live database.
- Kassiber already has `reports export-pdf`; do not invent Clams-style custom render scripts unless the user specifically wants a custom format beyond the built-in export.

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

## Fallback

If command shape is unclear, consult:

```bash
kassiber --help
kassiber <command> --help
kassiber <command> <subcommand> --help
```
