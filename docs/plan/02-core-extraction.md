# Phase 0: Core Library Extraction

**Status:** Archived reference. Phase 0 is complete.
**Current source of truth:** `kassiber/cli/`, `kassiber/core/`, `kassiber/db.py`, `tests/test_cli_smoke.py`, and `TODO.md`.
**Do not implement from:** the old wave plan. It records history, not open work.

## Essence

Kassiber used to have a large `kassiber/app.py` monolith that mixed argparse,
SQL, domain logic, RP2 integration, envelope emission, and output printing.
Phase 0 split that surface into a reusable Python core plus a thin CLI layer.

The reason still matters: desktop UI, reports, sync, attachments, and Austrian
tax support must build on shared core modules instead of re-growing CLI-only
logic.

## Current Shape

```text
kassiber/
  __main__.py            # delegates to kassiber.cli.main
  cli/
    main.py              # argparse tree and command dispatch
    handlers.py          # compatibility-layer helpers while extraction continues
  core/
    runtime.py           # shared bootstrap, DB open, context/runtime config
    accounts.py
    wallets.py
    imports.py
    metadata.py
    rates.py
    sync.py
    sync_backends.py
    attachments.py
    tax_events.py
    reports.py
    engines/
      base.py
      rp2.py
  ui/
    app.py
    dashboard.py
    viewmodels/
```

`kassiber/app.py` is gone. Treat the extracted layout as the baseline.

## Guardrails

- Preserve the machine envelope: success is `{kind, schema_version, data}`;
  errors use the structured `error` envelope.
- Keep `tests/test_cli_smoke.py` as the behavior pin. Extend it when behavior
  changes; do not weaken it to make refactors pass.
- Keep runtime/bootstrap shared. Data-root resolution, env-file loading,
  settings manifest refresh, DB open, backend overlay, and context resolution
  must not become CLI-only again.
- Keep support modules leaf-like: `db.py`, `envelope.py`, `errors.py`,
  `msat.py`, `backends.py`, `tax_policy.py`, `transfers.py`,
  `wallet_descriptors.py`, and `importers.py` should not import the CLI layer.
- RP2 imports stay behind `kassiber/core/engines/`.
- Core functions should return data or raise `AppError`; printing and process
  exit belong at the CLI boundary.
- Use explicit SQL and small typed helpers. Do not introduce an ORM without a
  separate architecture decision.

## Landed Outcomes

- [x] CLI entrypoint moved to `kassiber/cli/main.py`
- [x] shared bootstrap lives in `kassiber/core/runtime.py`
- [x] report, sync, attachment, metadata, import, and rates flows moved into
  `kassiber.core`
- [x] tax normalization lives in `kassiber/core/tax_events.py`
- [x] RP2 journal building lives behind `kassiber/core/engines/rp2.py`
- [x] desktop UI imports the same local core instead of shelling out to the CLI

## Current Follow-Ups

Open work is tracked in `TODO.md`, not here. The important follow-ups are:

- keep the envelope boundary centralized
- finish project-local storage migration when the project-bundle design lands
- keep long-running desktop work in worker threads with dedicated SQLite
  connections
- harden Austrian E 1kv export as practitioner feedback arrives
- add external-document reconciliation without pushing invoice/VAT/general
  ledger concepts into RP2

## Non-Goals

- no rewrite of tax math inside Kassiber
- no ORM
- no REST/server mode by default
- no double-entry ledger model unless a future design explicitly adds it
- no compatibility aliases for pre-release command reshapes unless the project
  owner asks for them
