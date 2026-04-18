# TODO

This file replaces the old extraction-only checklist. Treat the previous
"Step 7 / Step 8 / Step 9 / Step 10 / Step 11" plan as obsolete.

Current direction:

1. Extract a reusable `kassiber.core` from `kassiber/app.py`
2. Keep the CLI first-class and machine output stable while extracting
3. Add the next cross-cutting features on top of that core:
   attachments, tax-engine cleanup, Austrian tax support
4. Build the desktop UI only after the shared core is real

## Rules for every session

- Preserve the current JSON envelope contract and error shape
- Keep `tests/test_cli_smoke.py` as the behavior pin; prefer extending it
  over adding new test files
- Keep `db.py`, `envelope.py`, `errors.py`, `msat.py`, `backends.py`,
  `tax_policy.py`, `wallet_descriptors.py`, `transfers.py`, and
  `importers.py` as leaf modules unless moving them deletes real code
  from `app.py`
- Keep wallet-level `Altbestand` provenance separate from profile-level
  tax policy unless there is an explicit migration plan
- Keep changes local-first, standard-library-first, and additive where
  possible
- Keep docs in lockstep with behavior changes
- When a TODO item is completed, check it in the same PR or commit that
  lands it; if scope changes, rewrite or split the item immediately
- Never use `git add -A`

## Right now

- [x] Finish the half-done `kassiber/importers.py` extraction:
  remove duplicate parser code from `kassiber/app.py`, import the shared
  helpers from `kassiber.importers`, then run the compile check and the
  CLI smoke suite
- [x] Map the current `app.py` seams before the next split:
  runtime/bootstrap, envelope/error emission, context resolution, SQL
  helpers, sync adapters, reports, and RP2 loading
- [ ] Keep the repo in a behavior-preserving extraction mode until the
  shared core exists; do not start UI implementation work before Phase 0
  is green

## Phase 0 - Shared Core Extraction

Goal: turn `kassiber/app.py` into a thin CLI shim over reusable Python
modules without changing user-visible behavior.

### 0a - Skeleton and bootstrap

- [x] Create `kassiber/core/` and `kassiber/cli/` package skeletons
- [x] Add `kassiber/core/runtime.py` for shared data-root, env-file,
  settings, backend overlay, DB-open, and context bootstrap
- [x] Add `kassiber/cli/main.py` as the future argparse entrypoint
- [x] Keep `kassiber.app:main` working through a shim until final cutover

### 0b - CRUD, settings, and rates

- [x] Extract workspace/profile/account/backend CRUD into `kassiber.core`
- [x] Extract wallet read/write orchestration into `kassiber.core`
- [x] Extract local rates cache operations into `kassiber.core`
- [x] Introduce small `core/repo/*.py` modules only where they simplify
  repeated SQL and reduce `app.py`

### 0c - Sync and import orchestration

- [x] Extract wallet sync orchestration into `kassiber/core/sync.py`
- [x] Keep parser-only logic in `kassiber/importers.py`
- [ ] Split backend-specific sync code into dedicated modules only when it
  shrinks `app.py` and keeps the dependency graph clean
- [x] Preserve current `esplora`, `electrum`, and `bitcoinrpc` behavior

### 0d - Metadata, journals, reports, and RP2 seam

- [x] Extract metadata note/tag/excluded, records, and BIP329 flows into
  `kassiber.core`
- [x] Extract report builders/export paths into `kassiber.core`
- [x] Move `_emit_error` out of `app.py` and make the envelope boundary
  explicit
- [x] Move `_RP2_MODULES` / `get_rp2_modules` behind a journals or engine
  seam
- [ ] Introduce a tax-engine interface that preserves today's RP2-backed
  generic behavior

### 0e - CLI cutover and cleanup

- [x] Move argparse tree and command dispatch into `kassiber/cli/main.py`
- [x] Update `pyproject.toml` entrypoints once the new CLI path is stable
- [ ] Shrink `kassiber/app.py` to a tiny shim or delete it
- [ ] Run the full smoke suite plus CLI help/smoke commands before
  calling Phase 0 done

### Phase 0 done when

- [ ] `tests/test_cli_smoke.py` passes unchanged or only with deliberate,
  documented extensions
- [ ] `kassiber/app.py` no longer contains business logic
- [ ] RP2 imports are isolated to the extracted engine/journals surface
- [ ] The CLI still emits the same machine envelope kinds and schema
  fields for existing commands

## Phase 0.5 - Attachments and Tax Engine Cleanup

Goal: add the next big capabilities on top of the extracted core, not on
top of the monolith.

### 0.5a - Shared tax-input normalization seam

- [ ] Add normalized tax-event inputs between raw transactions and engine
  logic
- [ ] Make ambiguous or under-specified tax semantics quarantineable
  instead of guessed
- [ ] Keep raw `transactions` rows as the source of truth; do not persist
  derived regime state onto them

### 0.5b - RP2 extraction

- [ ] Move the current generic RP2 journal flow behind the new engine seam
- [ ] Preserve current transfer handling, quarantine behavior, and report
  outputs
- [ ] Use fixture comparisons and smoke tests to catch rounding or journal
  regressions

### 0.5c - Transaction attachments

- [ ] Add attachment storage and metadata table
- [ ] Add CLI commands for add/list/remove plus `attachments gc` and
  `attachments verify`
- [ ] Make backup/restore aware of attachment files
- [ ] Keep URL attachments string-only; no fetching or indexing

### 0.5d - Austrian tax support

- [ ] Add Austrian policy registration on top of the shared engine seam
- [ ] Keep the Austrian path explicitly experimental until reviewed by a
  Steuerberater
- [ ] Implement Austrian defaults only where provenance is sufficient;
  quarantine the rest
- [ ] Add E 1kv CSV/PDF export only after the engine behavior is testable

### 0.5e - Rates and journal follow-through

- [ ] Wire the rates cache into journal pricing where it improves the
  current behavior without breaking the smoke contract
- [ ] Add explicit per-event overrides only after the core engine boundary
  is stable

## Phase 1 - Desktop App

Goal: build a local desktop UI over the shared core after the extraction
work is done.

- [ ] Add `kassiber ui` entrypoint once `kassiber.core` is stable
- [ ] Start with a macOS-first app shell and read-only dashboard
- [ ] Keep long-running sync/import/journal work off the UI thread
- [ ] Add connections, imports, attachments, and settings only after the
  read-only shell is solid
- [ ] Treat packaging/signing as a later step, not a blocker for the core
  refactor

## Later backlog

- [ ] Custom CSV mapping DSL for arbitrary wallet exports
- [ ] Rates/account adjustment surface
- [ ] Per-profile Tor proxy configuration
- [ ] BTCPay Greenfield API-backed sync/import flow
- [ ] Richer transfer pairing for multi-leg self-transfers
- [ ] Better cross-asset transfer accounting beyond audit metadata
- [ ] Revisit per-wallet basis attribution if a jurisdiction ever needs
  physical-lot answers
- [ ] At-rest encryption / OS keychain integration
- [ ] Kassiber skill bundle for agents
- [ ] Optional server/REST mode, still local-first and opt-in

## Open bugs and debt

- [ ] Fix `rates set` pair validation so malformed syntax like `BTCUSD`
  is rejected cleanly
- [ ] Keep the machine envelope boundary centralized and explicit
- [ ] Keep docs and examples Bitcoin-only

## Verification checklist

Run these after any extraction or behavior change:

- `PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc uv run python -m py_compile kassiber/*.py`
- `uv run python -m unittest tests.test_cli_smoke -v`
- `uv run python -m kassiber --help`
- `uv run python -m kassiber --machine status`
- `uv run python -m kassiber backends list`
- `uv run python -m kassiber wallets kinds`
- `uv run python -m kassiber profiles create --help`
- `uv run python -m kassiber metadata records --help`
- `uv run python -m kassiber journals events --help`
- `uv run python -m kassiber reports balance-history --help`
- `uv run python -m kassiber rates --help`
