# TODO

This file replaces the old extraction-only checklist. Treat the previous
"Step 7 / Step 8 / Step 9 / Step 10 / Step 11" plan as obsolete.

Current direction:

1. Keep the extracted `kassiber.core` / `kassiber.cli` split stable
2. Keep the CLI first-class and machine output stable while building on that core
3. Add the next cross-cutting features on top of the extracted core:
   attachments, tax-engine cleanup, Austrian RP2 integration
4. Keep desktop work layered on the shared core rather than reintroducing
   monoliths or duplicate logic

## Rules for every session

Use `./scripts/quality-gate.sh` before calling work ready to push. It wraps the baseline compile, smoke, regression, and CLI help checks so humans and agents use the same verification path.

- Preserve the current JSON envelope contract and error shape
- Keep `tests/test_cli_smoke.py` as the behavior pin; prefer extending it
  over adding new test files
- Keep `db.py`, `envelope.py`, `errors.py`, `msat.py`, `backends.py`,
  `tax_policy.py`, `wallet_descriptors.py`, `transfers.py`, and
  `importers.py` as leaf modules unless moving them deletes real code
  from `app.py`
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
- [x] Keep the repo in a behavior-preserving extraction mode until the
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
- [x] Keep the CLI cutover on `kassiber.cli.main:main`; the old `kassiber.app`
  shim has now been removed

### 0b - CRUD, settings, and rates

- [x] Extract workspace/profile/account/backend CRUD into `kassiber.core`
- [x] Extract wallet read/write orchestration into `kassiber.core`
- [x] Extract local rates cache operations into `kassiber.core`
- [x] Introduce small `core/repo/*.py` modules only where they simplify
  repeated SQL and reduce `app.py`

### 0c - Sync and import orchestration

- [x] Extract wallet sync orchestration into `kassiber/core/sync.py`
- [x] Keep parser-only logic in `kassiber/importers.py`
- [x] Split backend-specific sync code into dedicated modules only when it
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
- [x] Introduce a tax-engine interface that preserves today's RP2-backed
  generic behavior

### 0e - CLI cutover and cleanup

- [x] Move argparse tree and command dispatch into `kassiber/cli/main.py`
- [x] Update `pyproject.toml` entrypoints once the new CLI path is stable
- [x] Shrink `kassiber/app.py` to a tiny shim or delete it
- [x] Run the full smoke suite plus CLI help/smoke commands before
  calling Phase 0 done

### Phase 0 done when

- [x] `tests/test_cli_smoke.py` passes unchanged or only with deliberate,
  documented extensions
- [x] `kassiber/app.py` no longer contains business logic
- [x] RP2 imports are isolated to the extracted engine/journals surface
- [x] The CLI still emits the same machine envelope kinds and schema
  fields for existing commands

## Phase 0.5 - Attachments and Tax Engine Cleanup

Goal: add the next big capabilities on top of the extracted core, not on
top of the monolith.

### 0.5a - Shared tax-input normalization seam

- [x] Add normalized tax-event inputs between raw transactions and engine
  logic
- [x] Make ambiguous or under-specified tax semantics quarantineable
  instead of guessed
- [x] Keep raw `transactions` rows as the source of truth; do not persist
  derived regime state onto them

### 0.5b - RP2 extraction

- [x] Move the current generic RP2 journal flow behind the new engine seam
- [x] Preserve current transfer handling, quarantine behavior, and report
  outputs
- [x] Use fixture comparisons and smoke tests to catch rounding or journal
  regressions

### 0.5c - Transaction attachments

- [x] Add attachment storage and metadata table
- [x] Add CLI commands for add/list/remove plus `attachments gc` and
  `attachments verify`
- [x] Make backup/restore aware of attachment files
- [x] Keep URL attachments string-only; no fetching or indexing

### 0.5d - Austrian tax support

- [x] Fork RP2 to `bitcoinaustria/rp2` so Austrian tax logic can live in the
  tax engine rather than expanding Kassiber-side tax math
- [x] Add Austrian country / accounting / report plugins in the RP2 fork
- [x] Keep Kassiber-side normalization, provenance capture, and multi-account
  transfer preparation feeding the RP2-backed Austrian path
- [x] Re-enable Austrian profiles now that the RP2-backed path is wired,
  tested, and documented
- [x] Replace Option C (quarantine-on-cross-asset-Neu-swap) with Option A
  (topological two-pass compute) so `carried_basis_fiat` is populated
  automatically; see [docs/austrian-handoff.md](docs/austrian-handoff.md)
- [ ] Add E 1kv CSV/PDF export path on top of Kassiber's persisted
  Austrian disposal buckets / Kennzahl mapping

### 0.5e - Rates and journal follow-through

- [x] Wire the rates cache into journal pricing where it improves the
  current behavior without breaking the smoke contract
- [x] Add explicit per-event overrides only after the core engine boundary
  is stable

### 0.5f - External document reconciliation groundwork

- [ ] Add [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md) follow-through in code and schema rather than letting merchant/invoice scope drift ad hoc
- [x] Persist BTCPay confirmed wallet-sync config on wallets so `wallets sync` / `wallets sync --all` can reuse store-backed sources without retyping `--store-id`
- [ ] Keep BTCPay file import conservative (`deposit` / `withdrawal`) until a confirmed document match or explicit review step reclassifies the transaction

## Phase 1 - Desktop App

Goal: build the Phase 1 PySide6/QML app shell from `docs/plan/04-desktop-ui.md`
over the shared core after the extraction work is done.

- [x] Add `kassiber ui` entrypoint once `kassiber.core` is stable
- [x] Start with a macOS-first PySide6/QML app shell and empty state
- [x] Persist window geometry in `settings.json` under a `ui` subkey
- [x] Replace the single placeholder panel with a routed mockup scaffold for Welcome, Overview, Connection Detail, Transaction View, Tax Reports, and Settings
- [x] Add the Phase 2 read-only dashboard tiles over `kassiber.core`
- [ ] Continue screenshot-driven polish for the Claude Design screens and close the remaining spacing/typography/detail gaps as assets are collected under `docs/design/`
- [ ] Keep long-running sync/import/journal work off the UI thread via QThreads once those actions land
- [ ] Add connections, imports, attachments, and fuller settings only after
  the app shell is solid
- [ ] Treat packaging/signing as a later step, not a blocker for the core
  refactor

## Later backlog

- [ ] Custom CSV mapping DSL for arbitrary wallet exports
- [ ] Rates/account adjustment surface
- [ ] Per-profile Tor proxy configuration
- [ ] Extend BTCPay Greenfield sync beyond confirmed wallet history with stable invoice/payment ids and raw payload snapshots
- [ ] External document records for invoices, receipts, contracts, and related BTC-linked business evidence
- [ ] Many-to-many document/payment links with allocations and reconciliation state
- [ ] Deterministic matching rules before any AI assistance
- [ ] Review/confirmation workflow for proposed matches and commercial annotations
- [ ] Split commercial annotations from RP2-facing tax primitives during journal preparation
- [ ] Accountant-facing export of matched BTC subledger rows with document references
- [ ] Opt-in local AI extraction and tie-breaking only after deterministic matching is solid
- [ ] Richer transfer pairing for multi-leg self-transfers
- [ ] Better cross-asset transfer accounting beyond audit metadata
- [ ] Revisit per-wallet basis attribution if a jurisdiction ever needs
  physical-lot answers
- [ ] Adopt a per-project storage layout: one SQLite DB per project,
  minimal global app state, and no active top-level wallet side tree
- [ ] Keep transaction document links in the project DB; only add managed
  copied-file storage if a concrete offline/self-contained workflow needs it
- [x] Keep backend definitions and default-backend selection canonical in
  SQLite; dotenv files now bootstrap older/new stores instead of serving as
  the long-term storage path
- [x] Keep normal backend and wallet success output safe-to-record for
  secret-bearing config values by redacting raw credentials and raw descriptor
  material while preserving presence / state flags
- [ ] Finish the project-local part of backend storage once the per-project
  DB layout lands
- [ ] Extend the safe-to-record contract beyond normal success output to
  `--debug`, error surfaces, and downloadable logs
- [ ] Replace plaintext secret enrollment through CLI args / dotenv with
  local-only secret capture flows or secret refs so hosted agents do not need
  raw values in prompts or command strings
- [ ] Split wallet descriptor and other sensitive config out of the generic
  `wallets.config_json` blob into typed project-local tables plus OS keychain
  references where appropriate
- [ ] Seal backend credentials, private descriptors, and blinding keys behind
  OS keychain-backed secret refs instead of leaving raw values in plaintext
  SQLite / dotenv storage
- [x] Kassiber skill bundle for agents (`skills/kassiber`)
- [ ] Optional server/REST mode, still local-first and opt-in

## Open bugs and debt

- [ ] Fix `rates set` pair validation so malformed syntax like `BTCUSD`
  is rejected cleanly
- [ ] Keep the machine envelope boundary centralized and explicit
- [ ] Keep docs and examples Bitcoin-only
- [ ] Add a narrow docs-drift check for shared command / verification /
  safe-to-record surfaces so `README.md`, `AGENTS.md`, `SECURITY.md`, and
  `skills/kassiber/` do not quietly diverge

## Verification checklist

Run these after any extraction or behavior change:

- `PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc uv run python -m py_compile kassiber/*.py kassiber/ui/*.py kassiber/ui/viewmodels/*.py`
- `uv run python -m unittest tests.test_cli_smoke -v`
- `uv run python -m kassiber --help`
- `uv run python -m kassiber --machine status`
- `uv run python -m kassiber backends list`
- `uv run python -m kassiber wallets kinds`
- `uv run python -m kassiber profiles create --help`
- `uv run python -m kassiber metadata records --help`
- `uv run python -m kassiber attachments list --help`
- `uv run python -m kassiber journals events --help`
- `uv run python -m kassiber reports balance-history --help`
- `uv run python -m kassiber rates --help`
- `uv run python -m kassiber ui --help`
