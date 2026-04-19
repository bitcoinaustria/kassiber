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

It checks:

- runtime and path resolution
- current workspace and profile
- wallet count
- journal entry count
- quarantine count

## Useful smoke commands

```bash
kassiber backends list
kassiber wallets kinds
kassiber journals list
kassiber journals quarantined
kassiber --format plain reports balance-sheet
```

For fresh installs, a zero-wallet or zero-journal result is expected. For established workspaces, treat those as investigation prompts rather than silent success.
