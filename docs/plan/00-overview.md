# Kassiber Plan Overview

**Status:** Living architecture map.
**Current source of truth:** code, README, AGENTS.md, and TODO.md.
**Rule for agents:** if this document and code disagree, inspect code and update
the docs in the same change.

## Product

Kassiber is a local-first Bitcoin accounting CLI. A desktop shell built on
Tauri 2 + React + TypeScript with a Python sidecar daemon is in active
development; see [01-stack-decision.md](01-stack-decision.md) for the stack
and [04-desktop-ui.md](04-desktop-ui.md) for the implementation plan.

It owns wallet sync/import, local storage, provenance, metadata, attachments,
transfer pairing, review/quarantine workflows, CLI/desktop UX, and
accountant-facing BTC subledger exports. Source-of-funds reporting is in scope
as a reviewed, path-scoped provenance report, not as chain-surveillance scoring.

RP2 owns the existing crypto lot/tax calculation path. Opt-in organizational
accounting adds a separate general ledger, book valuation adjustments, and
jurisdiction working papers; see plan 17. It does not reinterpret current
wallet buckets or personal tax journals as double-entry books.

The accepted product direction serves private individuals, businesses, and
associations. Organizational accounting is opt-in per book. Portfolio and
personal-tax workflows remain available without ledger or corporate setup.
AI assistance extends through scoped local processing and explicitly approved
remote disclosures under the existing daemon, secret, and consent contracts.

Out of scope unless a future design says otherwise:

- invoicing
- VAT/RKSV
- unreviewed production claims for the general ledger; the integrated local
  implementation follows `17-general-accounting-and-private-ai-spec.md` and
  its remaining acceptance gates are recorded in
  `../reference/general-accounting-acceptance.md`
- remote multi-user service
- mobile
- broad altcoin product scope

## Current Architecture

- CLI entrypoint: `kassiber/cli/main.py`
- remaining CLI helper surface: `kassiber/cli/handlers.py`
- shared runtime/core: `kassiber/core/`
- desktop shell: `ui-tauri/` (under construction per [01-stack-decision.md](01-stack-decision.md) and [04-desktop-ui.md](04-desktop-ui.md))
- storage: SQLite under the OS-native per-user app-data root, with meaningful
  `~/.kassiber` state moved there once when the native target does not exist
- storage shape: one DB per project under `<state-root>/projects/`
- tax engine: RP2 fork at `bitcoinaustria/rp2`
- machine envelope: `{kind, schema_version, data}` for success, structured
  `error` envelope for failure

## Product Invariants

- local-first by default
- CLI stays first-class
- no bundled browser runtime; no separately-installed user runtime (Tauri uses
  the OS webview; the bundled Python sidecar ships inside the app)
- Bitcoin-first; L-BTC is in scope
- BTC amounts are integer msat
- reports are trusted only after journal processing
- ambiguous tax semantics quarantine instead of being guessed
- every observed quantity is represented exactly once, while unresolved
  custody never becomes a taxable event
- secret-bearing success output stays redacted/safe for agents
- docs and command behavior move together

## Track Status

| Track | Status | Current direction |
|---|---|---|
| Core extraction | Landed | keep logic in shared core, not CLI/UI copies |
| Attachments | Landed | use shipped `attachments`; keep links/file blobs bounded |
| Austrian RP2 path | Active | processing and review-gated E 1kv PDF/XLSX export work; domestic-provider KESt metadata pending |
| Organizational accounting and AI | Proposed, not shipped | complete opt-in double-entry workflow, local document assistance, country packs, and the user's K2/annex acceptance case; plan 17 |
| Austrian corporate handoff | Proposed | K2 and required annexes consume reviewed accounting/tax facts; plan 16 is historical research superseded in scope by plan 17 |
| Desktop UI | In progress | Tauri 2 + React + TypeScript with a Python sidecar daemon, per [01-stack-decision.md](01-stack-decision.md) and [04-desktop-ui.md](04-desktop-ui.md) |
| Project storage | Target-state | app-wide to per-project migration still needs a focused plan |
| External documents | Design | reconcile BTC evidence without becoming ERP/invoicing |
| Source of funds | v1 landed | desktop review workstation, reviewed transaction-flow links, disclosure preview, immutable snapshots, and gated PDF export |
| Custody lineage | Design/active | separate quantity from tax, reconcile complete policies automatically, and review durable missing-wallet bridges |
| Packaging | In progress | Unsigned prerelease desktop bundles now carry a PyInstaller CLI sidecar; signed production packaging and any `python-build-standalone` replacement are still open |

## Stack

Desktop: Tauri 2 + React + TypeScript + shadcn/ui, with the Python core
running as a long-lived sidecar daemon over stdin/stdout JSONL.

See [01-stack-decision.md](01-stack-decision.md) for the stack decision and
[04-desktop-ui.md](04-desktop-ui.md) for the implementation plan.

## Doc Index

- `01-stack-decision.md`: desktop stack ADR (Tauri + React + Python sidecar)
- `02-core-extraction.md`: archived Phase 0 extraction reference
- `03-storage-conventions.md`: project-bundle storage target
- `04-desktop-ui.md`: desktop implementation plan
- `05-attachments.md`: attachment/link boundary
- `06-austrian-tax-engine.md`: Austrian RP2 boundary and E 1kv direction
- `07-austrian-tax-open-questions.md`: unresolved AT assumptions and review gates
- `08-external-document-reconciliation.md`: BTC-side evidence/reconciliation boundary
- `09-source-of-funds.md`: source-of-funds report boundary and flow-link design
- `10-secret-management.md`: SQLCipher/backup secret-handling boundary
- `11-exit-tax-deemed-disposal.md`: Wegzugsbesteuerung / deemed-disposal report design
- `12-collateralized-loans.md`: collateralized-loan leg modeling
- `13-device-sync.md`: shipped cross-device / multi-user sync guardrails (mailbox-first, no trusted server; issue #309)
- `14-custody-lineage.md`: custody quantity/tax separation, durable
  missing-wallet bridges, and long-horizon reconciliation
- `15-custody-simplification.md`: bounded simplification after the custody
  lineage implementation
- `16-cost-basis-pools-and-employment-compensation.md`: country-configurable
  pool scope, exact cross-pool basis carry, and compensation-as-acquisition
  handling across Kassiber and RP2
- `16-austrian-corporate-tax-handoff.md`: K1/K2 corporate-tax handoff
  research; earlier subledger-only scope superseded by plan 17
- `17-general-accounting-and-private-ai-spec.md`: consolidated One-Shot
  organizational-accounting specification, private-user compatibility, scoped
  AI assistance, security requirements, and K2/annex acceptance criteria

## Highest-Risk Drift Points

- treating historical phase lists as live work
- implementing schema sketches without checking shipped tables
- describing target project storage as current behavior
- confusing proposed organizational accounting with shipped functionality, or
  making organizational setup mandatory for private portfolio users
- treating `K2` and `K1` as the same Austrian crypto path
- treating source-of-funds reports as automatic proof when reviewed links or
  source evidence are missing
- duplicating RP2 crypto lot math or assuming its personal-tax result is always
  a valid organizational book carrying value
- relying on VCS-pinned RP2 for packaged builds without testing
- forgetting to re-run journals after metadata, pricing, pairing, or exclusion
  changes

## Next Executable Work

Use `TODO.md`. This overview is for orientation, not task assignment.
