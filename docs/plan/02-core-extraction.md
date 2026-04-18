# Phase 0: Core Library Extraction

**Goal:** Carve a reusable `kassiber.core` library out of the current 7k-line `kassiber/app.py`. The CLI becomes a thin argparse translator that calls into `core`. The UI (later phases) imports `core` directly.

**Status:** Not started. This doc defines the target shape and the migration plan.

**Why now:** Both the desktop UI and the Austrian tax engine depend on a clean library surface. Doing them against the current monolith would either scatter business logic further or require duplication. `AGENTS.md` already flags this refactor as in progress.

## Why it's safe

The CLI already emits a **stable JSON envelope** for every command. `tests/test_cli_smoke.py` exercises ~200 scenarios against that envelope — subprocess invocation, field shape, msat field correctness, cross-wallet transfer detection, capital gains math, balance history. That is the contract we preserve.

**Refactor rule:** any reshaping that does not change the envelope is safe. The smoke test fails loudly if anything changes. Run it on every extracted command before moving on.

## Phase 0 guardrails

These are hard constraints for the extraction work:

1. **Preserve the current runtime/bootstrap contract.** Data-root resolution, env-file resolution, settings.json refresh, DB/env backend overlay, and context resolution are shared application concerns. They must move into a reusable runtime/bootstrap layer, not stay CLI-only.
2. **Preserve the current schema shape in Phase 0.** IDs stay `TEXT`, workspace/profile scoping stays explicit, and wallet-level tax provenance stays where it is today unless a dedicated migration plan says otherwise.
3. **Do not regress `open_db()` behavior.** Any extracted DB-open path must still leave old databases readable and writable by applying the current compatibility logic.
4. **Do not move already-good leaf modules just for aesthetics.** Extraction effort should go toward shrinking `app.py`, not renaming stable support code.

## Current shape

```
kassiber/
  __main__.py            # thin: calls app.main
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
- `app.py` is the thing we break up

## Target shape

```
kassiber/
  __main__.py                    # delegates to kassiber.cli.main
  app.py                         # thin shim during migration; may disappear at the end
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
    normalized_events.py         # NEW — Phase 0.5 tax normalization seam
    engines/
      __init__.py
      base.py                    # NEW — abstract TaxEngine interface
      rp2_generic.py             # RP2 path extracted from app.py
      at_kryptovo.py             # NEW — Austrian engine; see 06-austrian-tax-engine.md
    reports/
      __init__.py
      capital_gains.py
      journal_entries.py
      e1kv.py                    # NEW — Austrian E 1kv PDF + CSV; see 06
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
7. **Tax engine is behind an interface.** No file outside `core/engines/` imports `rp2`. The boundary pays for itself when Austrian engine arrives.

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
- Move the RP2 integration (app.py:540–681, plus the ledger state + journal write at app.py:4125–4582) into `core/engines/rp2_generic.py`
- The envelope-emitting wrapper (`kassiber journals process`) stays in the CLI, but its guts become: build inputs, call engine, persist `JournalResult` to `journal_entries` + `journal_quarantines`, emit envelope.

### Wave 4.5 — tax normalization seam (0.5–1 day)

- Add `core/normalized_events.py` as the bridge between raw transactions and tax-engine input
- Normalize raw transactions + transfer-pair state + explicit annotations into typed tax events
- Quarantine ambiguous events instead of guessing at mining / inheritance / routing income / swap semantics
- Keep this seam shared so both `rp2_generic` and `at_kryptovo` consume the same normalized input contract

### Wave 5 — reports (1 day)

- `core/reports/capital_gains.py`, `core/reports/journal_entries.py`: existing CSV emitters moved.
- `pdf_report.py` moved to `core/reports/pdf.py`.
- E 1kv deferred to Phase 0.5 (AT engine work).

### Wave 6 — cleanup (0.5 day)

- Delete now-dead code in `app.py`. It should shrink to ~100 lines or disappear entirely (replaced by `cli/main.py`).
- Confirm `pyproject.toml` entry points point at `kassiber.cli.main`.
- Full smoke test run. All green.

**Estimated total:** 5–7 working days for a solo vibecoder with Claude.

## Success criteria

- [ ] `tests/test_cli_smoke.py` passes unchanged
- [ ] `grep -r "argparse\|sys.argv\|sys.exit\|^print\|\.print(" kassiber/core/` returns zero hits
- [ ] `grep -r "import rp2\|from rp2" kassiber/` returns hits only inside `kassiber/core/engines/rp2_generic.py`
- [ ] Every CLI command's source footprint in `cli/commands/*.py` is <100 lines (guideline, not hard rule)
- [ ] `kassiber/app.py` is either deleted or contains only a deprecation shim
- [ ] The Phase 0 extraction does not change ID types, scope fields, or the location of wallet-level Altbestand provenance
- [ ] A new unit-test file `tests/test_core_unit.py` covers the direct-call surface of at least `core.wallets`, `core.accounts`, `core.transactions`, `core.rates`
- [ ] `python -c "from kassiber.core import wallets, accounts, transactions, rates, sync, engines"` imports cleanly without loading argparse, sqlite3 connections, or RP2

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
2. The **Austrian tax engine** can be added as a sibling of `rp2_generic` without touching CLI wiring
3. The **attachments feature** can be added as a cross-cutting capability that both CLI and UI use
