# Verification

Use this reference to quickly confirm that Kassiber is ready before a larger workflow.

## Quick state check

```bash
kassiber status
kassiber workspaces list
kassiber profiles list
kassiber accounts list
kassiber wallets list
```

Use `--machine` when another tool needs the output.

## Helper script

This skill bundles a verification helper:

```bash
<skill-dir>/scripts/verify-state.sh
<skill-dir>/scripts/verify-state.sh --section context
<skill-dir>/scripts/verify-state.sh --section wallets
```

Requirements:

- `jq` must be installed
- if `kassiber` is not on `PATH`, the Kassiber repo needs to be available relative to the skill bundle so the script can resolve the repo root and run `uv`

It checks:

- runtime and path resolution
- current ledger/books (`workspace` and `profile` in CLI output)
- wallet count
- journal entry count
- quarantine count

The helper emits a machine-readable envelope with a `summary` section. Hard failures land in `summary.issues`; softer prompts like zero wallets on a fresh install or non-zero quarantine land in `summary.attention`.

## Useful smoke commands

```bash
kassiber backends list
kassiber wallets kinds
kassiber journals list
kassiber journals quarantined
kassiber --format plain reports balance-sheet
```

For fresh installs, a zero-wallet or zero-journal result is expected. For established ledgers, treat those as investigation prompts rather than silent success.
