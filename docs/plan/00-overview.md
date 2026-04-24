# Kassiber Plan Overview

**Status:** Living architecture map.
**Current source of truth:** code, README, AGENTS.md, and TODO.md.
**Rule for agents:** if this document and code disagree, inspect code and update
the docs in the same change.

## Product

Kassiber is a local-first Bitcoin accounting CLI with an early PySide6/QML
desktop shell.

It owns wallet sync/import, local storage, provenance, metadata, attachments,
transfer pairing, review/quarantine workflows, CLI/desktop UX, and
accountant-facing BTC subledger exports.

RP2 owns tax computation. Kassiber prepares and explains; RP2 computes.

Out of scope unless a future design says otherwise:

- invoicing
- VAT/RKSV
- company general ledger
- remote multi-user service
- mobile
- broad altcoin product scope

## Current Architecture

- CLI entrypoint: `kassiber/cli/main.py`
- remaining CLI helper surface: `kassiber/cli/handlers.py`
- shared runtime/core: `kassiber/core/`
- desktop shell: `kassiber/ui/`
- storage: SQLite under current app-wide `~/.kassiber/` state root
- target storage direction: one DB per project under `~/.kassiber/projects/`
- tax engine: RP2 fork at `bitcoinaustria/rp2`
- machine envelope: `{kind, schema_version, data}` for success, structured
  `error` envelope for failure

## Product Invariants

- local-first by default
- CLI stays first-class
- no bundled Chromium and no Node runtime
- Bitcoin-first; L-BTC is in scope
- BTC amounts are integer msat
- reports are trusted only after journal processing
- ambiguous tax semantics quarantine instead of being guessed
- secret-bearing success output stays redacted/safe for agents
- docs and command behavior move together

## Track Status

| Track | Status | Current direction |
|---|---|---|
| Core extraction | Landed | keep logic in shared core, not CLI/UI copies |
| Attachments | Landed | use shipped `attachments`; keep links/file blobs bounded |
| Austrian RP2 path | Active | processing works; E 1kv export pending |
| Desktop UI | In progress | routed read-only shell exists; live workers/actions pending |
| Project storage | Target-state | app-wide to per-project migration still needs a focused plan |
| External documents | Design | reconcile BTC evidence without becoming ERP/invoicing |
| Packaging | Planned | Briefcase intended; macOS `.app` not proven yet |

## Stack

Desktop: PySide6 + QML.

Why: one Python runtime, direct core imports, no webview, no Node runtime, and
good enough visual fidelity for an accounting workbench.

Use current QML `Canvas` charting while it is enough. Prefer Qt Graphs over
QtCharts for richer future charting.

## Doc Index

- `01-stack-decision.md`: desktop stack ADR
- `02-core-extraction.md`: archived Phase 0 extraction reference
- `03-storage-conventions.md`: project-bundle storage target
- `04-desktop-ui.md`: desktop implementation guide
- `05-attachments.md`: attachment/link boundary
- `06-austrian-tax-engine.md`: Austrian RP2 boundary and E 1kv direction
- `07-austrian-tax-open-questions.md`: unresolved AT assumptions and review gates
- `08-external-document-reconciliation.md`: BTC-side evidence/reconciliation boundary

## Highest-Risk Drift Points

- treating historical phase lists as live work
- implementing schema sketches without checking shipped tables
- describing target project storage as current behavior
- expanding Kassiber into invoicing/VAT/general-ledger territory
- adding Austrian tax math in Kassiber instead of RP2
- relying on VCS-pinned RP2 for packaged builds without testing
- forgetting to re-run journals after metadata, pricing, pairing, or exclusion
  changes

## Next Executable Work

Use `TODO.md`. This overview is for orientation, not task assignment.
