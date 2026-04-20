# Phase 0: Core Library Extraction

**Goal:** Record how Phase 0 carved a reusable `kassiber.core` library out of the old `kassiber/app.py` monolith. The CLI is now a thin argparse translator that calls into `core`, and the UI imports the same core directly.

**Status:** Completed for the current repo shape. Keep this doc as the reference architecture, guardrails, and rationale for future extraction work; do not treat it as an open execution plan.

**Why it mattered:** Both the desktop UI and the Austrian RP2 integration depend on a clean library surface. Doing them against the old monolith would have scattered business logic further or forced duplication. The current extracted layout is the result of this phase.

## Why it's safe

The CLI already emits a **stable JSON envelope** for every command. `tests/test_cli_smoke.py` exercises ~200 scenarios against that envelope — subprocess invocation, field shape, msat field correctness, cross-wallet transfer detection, capital gains math, balance history. That is the contract we preserve.

**Refactor rule:** any reshaping that does not change the envelope is safe. The smoke test fails loudly if anything changes. Run it on every extracted command before moving on.

## Phase 0 guardrails

These are hard constraints for the extraction work:

1. **Preserve the current runtime/bootstrap contract.** Data-root resolution, env-file resolution, settings.json refresh, DB/env backend overlay, and context resolution are shared application concerns. They must move into a reusable runtime/bootstrap layer, not stay CLI-only.
2. **Preserve the current schema shape in Phase 0.** IDs stay `TEXT`, workspace/profile scoping stays explicit, and wallet-level tax provenance stays where it is today unless a dedicated migration plan says otherwise.
3. **Do not regress `open_db()` behavior.** Any extracted DB-open path must still leave old databases readable and writable by applying the current compatibility logic.
4. **Do not move already-good leaf modules just for aesthetics.** Extraction effort should go toward shrinking `app.py`, not renaming stable support code.

## Pre-extraction shape

```
kassiber/
  __main__.py            # called app.main
  app.py                 # ~7000 lines: argparse + dispatch + domain + SQL
  db.py                  # schema, connection, path resolution
  envelope.py            # JSON envelope emitter
  errors.py              # AppError exception
  msat.py                # int<->decimal conversions
  tax_policy.py          # TaxPolicy dataclass + POLICY_BUILDERS registry
  transfers.py           # self-transfer detection
  importers.py           # Phoenix, BTCPay, BIP329
  backends.py            # backend config (env + DB overlay)
  wallet_descriptors.py  # descriptor derivation helpers
  pdf_report.py          # PDF output for existing reports
  # ... a handful of other small helpers
```

Characteristics:
- Domain logic, SQL, argparse, envelope emission, and output printing are interleaved inside `app.py`
- RP2 is lazy-imported inside `app.py` (`get_rp2_modules()` at `app.py:543`), tax computation lives around lines 4125–4580
- Some modules (`db.py`, `envelope.py`, `msat.py`, `tax_policy.py`, `transfers.py`, `importers.py`, `backends.py`) are already clean and reusable — they stay put
- `app.py` was the thing we needed to break up

## Landed shape

The current repo no longer has that monolith. The landing shape is broadly:

```text
kassiber/
  __main__.py            # delegates to kassiber.cli.main
  backends.py
  db.py
  envelope.py
  errors.py
  importers.py
  msat.py
  tax_policy.py
  transfers.py
  wallet_descriptors.py
  cli/
    main.py
    handlers.py
  core/
    runtime.py
    attachments.py
    reports.py
    sync.py
    sync_backends.py
    tax_events.py
    engines/
      base.py
      rp2.py
  ui/
    app.py
    dashboard.py
    viewmodels/
```

## Extraction target shape (historical)

```
kassiber/
  __main__.py                    # delegates to kassiber.cli.main
  app.py                         # thin shim during migration in the original plan; the current repo no longer keeps it
  db.py                          # remains canonical DB bootstrap in Phase 0
  envelope.py                    # remains canonical envelope contract
  errors.py                      # remains canonical AppError definition
  msat.py                        # remains canonical conversion helpers
  backends.py                    # remains canonical env/backend overlay helpers
  importers.py                   # parser helpers already extracted
  transfers.py                   # self-transfer logic stays as a leaf module
  tax_policy.py                  # remains canonical policy registry
  core/
    __init__.py                  # public API re-exports
    runtime.py                   # NEW — shared bootstrap for CLI + UI
    repo/
      __init__.py
      wallets.py
      accounts.py
      transactions.py
      tags.py
      rates.py
      attachments.py             # NEW — see 05-attachments.md
    wallets.py                   # wallet + descriptor CRUD orchestration
    accounts.py                  # workspace / profile / account orchestration
    transactions.py              # list, filter, attachment-aware record ops
    metadata.py                  # note/tag/excluded + attachment orchestration
    rates.py                     # fiat price cache + CoinGecko sync
    sync.py                      # backend dispatch + orchestration
    journals.py                  # journal processing orchestration
    attachments.py               # NEW — file/url attachment logic
    tax_events.py                # NEW — Phase 0.5 tax normalization seam
    engines/
      __init__.py
      base.py                    # NEW — abstract TaxEngine interface
      rp2.py                     # RP2 adapter extracted from app.py
    reports/
      __init__.py
      capital_gains.py
      journal_entries.py
      e1kv.py                    # NEW — Austrian E 1kv PDF + CSV; backed by RP2-fork output; see 06
      pdf.py                     # moved from pdf_report.py
    migrations/
      __init__.py
      runner.py                  # wraps/extends current db bootstrap
      001_initial.sql            # captures current schema for fresh installs
      002_*.sql                  # anything additive going forward
  cli/
    __init__.py
    main.py                      # argparse tree + dispatch, calls core/runtime
    commands/
      wallets.py
      accounts.py
      transactions.py
      metadata.py
      rates.py
      sync.py
      import_.py
      journals.py
      reports.py
      attachments.py             # gc / verify / maintenance
      backup.py                  # backup / restore entrypoints
      backend.py
      profile.py
      workspace.py
      ui.py                      # launches kassiber.ui.app
  ui/                             # Phase 1+
    # ... see 04-desktop-ui.md
```

## Principles for `kassiber.core`

1. **No argparse.** No `sys.argv`. No `sys.exit`.
2. **No printing.** No `print`, no direct stdout writes. Return data; let CLI emit the envelope.
3. **Connection in, data out.** Public domain functions take a prepared `sqlite3.Connection` and return plain Python values — dataclasses, lists, dicts of primitives. Path-based bootstrap belongs in `core.runtime` / `db.py`, not in domain functions.
4. **No global state.** No module-level caches of config or connections. Pass what's needed.
5. **Exceptions are `AppError` subclasses** or standard Python exceptions. The CLI catches and maps to envelope error shape.
6. **Pure where possible.** `core.reports.capital_gains.build_report(entries) -> Report` should not touch the DB; DB reads happen in a caller function that bundles the inputs.
7. **Tax engine is behind an interface.** No file outside `core/engines/` imports `rp2`. The boundary pays for itself when Austrian support moves into the Kassiber-maintained RP2 fork.

## Shared runtime/bootstrap responsibilities

`core.runtime` is shared by CLI and UI. It owns:

1. Data-root and env-file resolution, including legacy fallbacks.
2. `settings.json` refresh and path manifest updates.
3. Opening the canonical DB connection through `db.py`.
4. Preserving the current schema/bootstrap guarantees from `open_db()`.
5. Loading runtime config and merging DB-backed backend overrides.
6. Resolving current workspace/profile context.

## CLI layer responsibilities

1. Parse argv with argparse.
2. Call the shared runtime/bootstrap layer.
3. Open DB connection with the standard pragmas and compatibility logic (see `03-storage-conventions.md`).
4. Call exactly one `core` function.
5. Wrap the return in an envelope via `envelope.emit(...)`.
6. Map exceptions to envelope error shape.
7. Exit with appropriate code.

Everything else is in `core` or the shared runtime/bootstrap layer.

## Migration approach

**Principle: one command at a time, smoke tests green after every step.**

The order matters. We extract the easiest, most self-contained commands first to establish the pattern, then harder ones.

### Wave 1 — infrastructure and trivial commands (1 day)

- Create `core/` and `cli/` skeletons with `__init__.py`
- Create `core/runtime.py` to hold the shared startup/bootstrap path currently embedded in `main()`
- Settle the repo shape as `core/repo/<domain>.py` and add typed dataclasses for existing tables (Wallet, Transaction, Account, Profile, etc.)
- Re-export existing leaf helpers through `core` where helpful; do **not** rename/move them yet unless doing so removes `app.py` code immediately
- Extract a trivial read-only command (e.g., `kassiber version`, `kassiber workspace list`) end-to-end to prove the pattern
- Introduce `core/migrations/runner.py` in a way that preserves today's `open_db()` self-bootstrap contract for every connection

### Wave 2 — CRUD commands (2 days)

Extract, one at a time:
- `workspace`, `profile`, `account`, `wallet` (list, add, update, remove)
- `tag` + `bip329` labels
- `backend` config commands
- `rates` set / sync / list

Each command:
1. Create `core/<module>.py` function with the right signature
2. Move SQL from `app.py` into it
3. Gut the CLI dispatch to call the new function
4. Run smoke tests
5. Commit

### Wave 3 — sync and imports (1–2 days)

- `core/sync.py` orchestrating backend-kind-specific fetchers (esplora, electrum, bitcoinrpc)
- Progress callback in the function signature: `sync_wallet(conn, wallet_id, *, on_progress=None)`. CLI passes `None`; UI will pass a callback.
- Keep the existing `importers.py` parser module as a leaf; wire orchestration through `core`

### Wave 4 — the RP2 tax path (2 days)

This is the biggest single chunk.

- Create `core/engines/base.py` with `TaxEngine` abstract interface:
  ```
  class TaxEngine(Protocol):
      name: str
      def compute_journal(
          self,
          transactions: list[TransactionIn],
          policy: TaxPolicy,
          *,
          on_progress: ProgressCallback | None = None,
      ) -> JournalResult: ...
  ```
- Move the RP2 integration (app.py:540–681, plus the ledger state + journal write at app.py:4125–4582) into `core/engines/rp2.py`
- The envelope-emitting wrapper (`kassiber journals process`) stays in the CLI, but its guts become: build inputs, call engine, persist `JournalResult` to `journal_entries` + `journal_quarantines`, emit envelope.

### Wave 4.5 — tax normalization seam (0.5–1 day)

- Add `core/tax_events.py` as the bridge between raw transactions and tax-engine input
- Normalize raw transactions + transfer-pair state + explicit annotations into typed tax events
- Quarantine ambiguous events instead of guessing at mining / inheritance / routing income / swap semantics
- Keep this seam shared so both the generic RP2 path and the future Austrian RP2-fork path consume the same normalized input contract

### Wave 5 — reports (1 day)

- `core/reports/capital_gains.py`, `core/reports/journal_entries.py`: existing CSV emitters moved.
- `pdf_report.py` moved to `core/reports/pdf.py`.
- E 1kv deferred to Phase 0.5 (AT engine work).

### Wave 6 — cleanup (0.5 day)

- Delete now-dead code in `app.py`. It should shrink to ~100 lines or disappear entirely (replaced by `cli/main.py`).
- Confirm `pyproject.toml` entry points point at `kassiber.cli.main`.
- Full smoke test run. All green.

**Estimated total:** 5–7 working days for a solo vibecoder with Claude.

## Landed outcomes

- [x] `tests/test_cli_smoke.py` remained the behavior pin through extraction
- [x] argparse and envelope emission moved out of the old monolith into the CLI layer
- [x] RP2 imports now live behind the engine seam
- [x] `kassiber/app.py` is gone and `__main__.py` delegates to `kassiber.cli.main`
- [x] Austrian-specific provenance did not become a profile-wide setting during extraction
- [x] the extracted layout is now the baseline for desktop work and Austrian RP2 integration

## Non-goals

- **Not an ORM introduction.** `core/repo/` is a thin dataclass returns layer, not SQLAlchemy. See `03-storage-conventions.md`.
- **Not a rewrite.** We move logic, we do not re-implement it. If we find bugs during extraction, we fix them and document them, but the default is byte-for-byte preservation.
- **Not changing the envelope shape.** Even obvious envelope improvements wait until after Phase 0 ships.
- **Not a schema redesign.** Phase 0 keeps today's `TEXT` IDs, explicit workspace/profile scoping, and current wallet tax provenance contract.
- **Not building the UI.** UI is Phase 1+.

## Risks

| Risk | Mitigation |
|---|---|
| Smoke tests miss a behavior | Write a unit test for the behavior before extracting |
| Circular imports between core modules | Keep `db.py` + `core/repo/` at the bottom of the dependency graph; modules import from repo, never the reverse |
| RP2 extraction changes a subtle decimal-rounding behavior | Compare journal_entries output on a fixture dataset before/after; diff should be empty |
| Duplication during wave transitions | Leave old code in place, extract, swap call sites, then delete. Short window of duplication is fine. |

## After Phase 0

With `core` in place, three things unlock in parallel:

1. The **desktop UI** (phases 1–4) can be built against a clean surface
2. The **Austrian RP2-fork integration** can be added on the same seam without touching CLI wiring
3. The **attachments feature** can be added as a cross-cutting capability that both CLI and UI use
