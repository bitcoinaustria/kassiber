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
- if `kassiber` is not on `PATH`, set `KASSIBER_BIN` to the binary, or run the
  helper from a Kassiber checkout with `cargo` installed; it then runs
  `cargo run -p kassiber --` from the checkout

It checks:

- runtime and path resolution
- current books set / book (`workspace` and `profile` in CLI output)
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

For fresh installs, a zero-wallet or zero-journal result is expected. For established books, treat those as investigation prompts rather than silent success.

## Are the journals reproducible?

`ui.journals.verify` (CLI `kassiber journals verify`) recomputes the journals
from the book's current inputs in a private in-memory copy and compares them
with the stored ones. It writes nothing and contacts nothing.

- `identical`: the stored journals are exactly what the inputs produce.
- `different`: name the tables, wallets and years it lists. With
  `deterministic: false` two recomputations disagreed, which points at a
  Kassiber bug rather than the user's data.
- `stale` / `not_processed`: inputs changed since processing; process the
  journals first. This is not a sign of wrong numbers.
- `refused`: processing itself would refuse; follow its error.

For a reproducible bug report the user can export an encrypted replay bundle
(`kassiber diagnostics replay-bundle export`, or Logs → Export → Replay
bundle in the desktop). Only the user can export it; never ask for its
passphrase in chat.
