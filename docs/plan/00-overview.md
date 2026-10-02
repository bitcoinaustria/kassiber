# Kassiber Plan Overview

**Status:** Living architecture map.
**Current source of truth:** code, README, AGENTS.md, and TODO.md.
**Rule for agents:** if this document and code disagree, inspect code and update
the docs in the same change.

## Product

Kassiber is local-first Bitcoin accounting with a desktop app and a first-class
CLI. The desktop uses Tauri, React, and a Python sidecar daemon;
see [01-stack-decision.md](01-stack-decision.md) for the stack
and [04-desktop-ui.md](04-desktop-ui.md) for the historical implementation roadmap.

It owns wallet sync/import, local storage, provenance, metadata, attachments,
transfer pairing, review/quarantine workflows, CLI/desktop UX, and
accountant-facing BTC subledger exports. Source-of-funds reporting is in scope
as a reviewed, path-scoped provenance report, not as chain-surveillance scoring.

Kassiber's own Rust [tax engine](../reference/tax-engine.md) owns the crypto
lot/tax calculation path. It replaced RP2 under
[plan 20](20-kassiber-tax-engine.md), which keeps RP2 as a test-only parity
oracle. Wallet buckets and personal tax journals are not double-entry books.
The opt-in general ledger of
plans 17/18 was removed on 2026-09-30 under [plan 19](19-extensions.md#removed);
an organization that needs one would get it as an extension.

AI assistance extends through scoped local processing and explicitly approved
remote disclosures under the existing daemon, secret, and consent contracts.

Out of scope unless a future design says otherwise:

- invoice issuance, payment initiation, RKSV, EBICS, and FinanzOnline transmission;
  reviewed VAT/tax supporting records do not imply an automatic filing engine
- a general ledger, double-entry bookkeeping, or K1/K2 corporate-tax
  working papers in core
- remote multi-user service
- mobile
- broad altcoin product scope in core; third-party asset extensions are
  proposed in [19-extensions.md](19-extensions.md)

## Current Architecture

- CLI entrypoint: `kassiber/cli/main.py`
- remaining CLI helper surface: `kassiber/cli/handlers.py`
- shared runtime/core: `kassiber/core/`
- desktop shell: `ui-tauri/`, sharing the daemon contract with the browser bridge
- storage: SQLite under the OS-native per-user app-data root, with meaningful
  `~/.kassiber` state moved there once when the native target does not exist
- storage shape: one DB per project under `<state-root>/projects/`
- tax engine: Rust crate in `tax-engine/`, called through the `kassiber_tax`
  binding ([contract](../reference/tax-engine.md)); RP2 is a test-only oracle
- machine envelope: `{kind, schema_version, data}` for success, structured
  `error` envelope for failure

The production accounting path is observations and reviewed evidence →
`core/custody_journal.py` → finalized tax projection → tax engine → stored journals
and reports. See
[the tax implementation boundary](../reference/tax.md#implementation-boundary)
and [the daemon contract](../reference/daemon.md#desktop-invoke-contract).

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

## Reading historical plans

Plan 02 records completed extraction. Plan 04 is the historical desktop roadmap;
use current reference docs for commands and TODO for unfinished gates. Plans
16 (Austrian corporate-tax handoff), 17 and 18 describe the general ledger
removed under plan 19; their code is preserved at git tag
`archive/general-ledger`. The chain-analysis audit records a dated baseline.
Shipped design documents retain their domain invariants; historical
checklists and test counts are not current delivery evidence.

## Track Status

| Track | Status | Current direction |
|---|---|---|
| Core extraction | Landed | keep logic in shared core, not CLI/UI copies |
| Attachments | Landed | use shipped `attachments`; keep links/file blobs bounded |
| Austrian tax path | Active | processing and review-gated E 1kv PDF/XLSX export work; domestic-provider KESt metadata pending |
| Kassiber tax engine | Active | phases 0-3 implemented: the pure Rust engine is the default behind the existing seam, with RP2-identical results and RP2 as a test-only oracle; phase 4 (simplify behind zero difference) is next, per [20-kassiber-tax-engine.md](20-kassiber-tax-engine.md) |
| Organizational accounting and AI | Removed | plans 16-18 removed 2026-09-30 under plan 19; code preserved at git tag `archive/general-ledger` |
| Desktop UI | In progress | Tauri 2 + React + TypeScript with a Python sidecar daemon, per [01-stack-decision.md](01-stack-decision.md) and [04-desktop-ui.md](04-desktop-ui.md) |
| Project storage | Implemented | per-project databases; [compatibility](../reference/database-compatibility.md) covers legacy upgrades |
| External documents | Design | reconcile BTC evidence without becoming ERP/invoicing |
| Source of funds | v1 landed | desktop review workstation, reviewed transaction-flow links, disclosure preview, immutable snapshots, and gated PDF export |
| Custody lineage | Design/active | separate quantity from tax, reconcile complete policies automatically, and review durable missing-wallet bridges |
| Local chain analysis | Audited / proposed expansion | share observed graph facts across transaction understanding and privacy; keep ownership hypotheses, observer knowledge and economic meaning separate |
| Extensions | Proposed | bundled and installed extensions on one API; general ledger removed, device-sync removal decided; feature split and open decisions in [19-extensions.md](19-extensions.md) |
| Packaging | Release-gated | bundled CLI runtime; public releases follow [local signing and notarization](../reference/macos-release.md) |

## Stack

Desktop: Tauri 2 + React + TypeScript + shadcn/ui, with the Python core
running as a long-lived sidecar daemon over stdin/stdout JSONL.

See [01-stack-decision.md](01-stack-decision.md) for the stack decision and
[04-desktop-ui.md](04-desktop-ui.md) for the implementation plan.

## Doc Index

- `01-stack-decision.md`: desktop stack ADR (Tauri + React + Python sidecar)
- `02-core-extraction.md`: archived Phase 0 extraction reference
- `03-storage-conventions.md`: project-bundle storage target
- `04-desktop-ui.md`: historical desktop implementation roadmap
- `05-attachments.md`: attachment/link boundary
- `06-austrian-tax-engine.md`: Austrian engine boundary and E 1kv direction
- `07-austrian-tax-open-questions.md`: unresolved AT assumptions and review gates
- `08-external-document-reconciliation.md`: BTC-side evidence/reconciliation boundary
- `09-source-of-funds.md`: source-of-funds report boundary and flow-link design
- `10-secret-management.md`: SQLCipher/backup secret-handling boundary
- `11-exit-tax-deemed-disposal.md`: Wegzugsbesteuerung / deemed-disposal report design
- `12-collateralized-loans.md`: collateralized-loan leg modeling
- `13-device-sync.md`: removed cross-device / multi-user sync design (removed 2026-09-30 per plan 19; archived at tag `archive/device-sync`)
- `14-custody-lineage.md`: custody quantity/tax separation, durable
  missing-wallet bridges, and long-horizon reconciliation
- `15-custody-simplification.md`: bounded simplification after the custody
  lineage implementation
- `16-cost-basis-pools-and-employment-compensation.md`: country-configurable
  pool scope, exact cross-pool basis carry, and compensation-as-acquisition
  handling across Kassiber and RP2
- `17-local-chain-analysis.md`: audited current capabilities, primary-source
  research and shared local investigation architecture for Bitcoin, Liquid
  and the user's Lightning evidence
- `16-austrian-corporate-tax-handoff.md`: removed K1/K2 corporate-tax
  handoff research (historical)
- `17-general-accounting-and-private-ai-spec.md`: removed general-ledger
  specification (historical)
- `18-general-accounting-pr-stack.md`: removed general-ledger delivery stack
  (historical)
- `19-extensions.md`: proposed extension model, core/extension/removal
  split, monorepo boundary checks, and asset-model prerequisites
- `20-kassiber-tax-engine.md`: RP2 replacement by a Kassiber-owned Rust tax
  engine, parity requirements, country manifests, and phases

## Highest-Risk Drift Points

- treating historical phase lists as live work
- implementing schema sketches without checking shipped tables
- describing target project storage as current behavior
- reintroducing general-ledger records or making organizational setup
  mandatory for private portfolio users
- treating source-of-funds reports as automatic proof when reviewed links or
  source evidence are missing
- duplicating crypto lot math outside the tax engine, changing its results
  outside a reviewed plan 20 change, or assuming its personal-tax result is
  always a valid organizational book carrying value
- forgetting to re-run journals after metadata, pricing, pairing, or exclusion
  changes

## Next Executable Work

Use `TODO.md`. This overview is for orientation, not task assignment.
