# Agent Overwrites

This file records owner-approved overwrites, regenerations, and scaffold passes
that replace existing files instead of making targeted edits.

## Required Entry Format

- Date:
- Approval source:
- Files:
- Reason:
- Command/tool:

## Entries

- Date: 2026-09-06
- Approval source: Owner approved splitting #546 after keeping #545 intact,
  with "do it", then explicitly authorized moving the feature, security and
  tests together: "ja du darfst das alles machen wie oft noch".
  Missing real-organization facts and measured pilot outcomes
  become separate product acceptance, not blanket code-merge blockers.
- Files: Selected financial AI context/proposals, disclosure daemon and CLI,
  sensitive provider isolation and their tests were removed from #546 and are
  restored together here in dependent `codex/accounting-selected-ai`.
  Unused accounting-export scope scaffolding in native `supervisor.rs` is
  removed following independent review; its former dedicated export UI was
  already removed. Shared files retain ordinary opaque
  task tools, exact local consent and durable export/restart behavior in #546.
- Reason: Separate financial-context disclosure from ordinary task execution
  without losing either feature. Recovery references are complete agent
  `32312f0e` on `codex/accounting-agent-presplit-20260906` and core `780f2da9`
  on `codex/accounting-core-presplit-20260906`. No financial records, live
  books or original preview are targets. Technical failures remain merge holds;
  the extraction and recombination passed independent reviews and full local gates.
- Command/tool: Scoped `apply_patch` extraction with preserved recovery refs.
  Task cut #546 is published at `9ef535f1`; third cut #550 is published at
  `2d5ab45f`. The recombined code/tests match preserved `32312f0e` except
  the intentionally removed unused native export scaffold; documentation
  retains the approved split and acceptance-policy updates. The feature was
  never left enabled with weaker guards in the second cut. Publication and
  passing gates do not waive the unresolved reliability hold.

- Date: 2026-09-05
- Approval source: Owner requested full accounting CLI/Agent-only instead of a
  new accounting UI, then explicitly approved implementation with "do it".
- Files: Dedicated `ui-tauri/src/routes/accounting/**`, accounting locale
  namespaces, capability hook and Settings panel removed. Shared AppShell,
  SettingsScreen, menuIntent, routeTree, assistantScreenContext,
  assistantSession, chrome/nav locales, and native lib.rs restored to the
  verified combined #542/#543 baseline `5371c851`. Minimal exact Assistant
  consent rendering retained under `components/ai/accounting`.
- Reason: Remove experimental accounting forms/navigation/export picker from
  delivery, not the ledger or security model. Complete prior UI retained
  at `fbfce410` on `codex/accounting-ui-preserved-20260905`. No database,
  financial record, encryption, private provider protection, or existing
  portfolio workflow was removed. Supervisor scope-invalidation and Vite
  configuration remain after automatic review rejected their restoration.
- Command/tool: `apply_patch`; local recovery commit/branch before removal.

- Date: 2026-09-05
- Approval source: Owner requested triage and relevant stacked PRs with merge
  after review, preserving the earlier CLI/Agent-only scope.
- Files: The extracted agent cut omits the former 99-line accounting additions
  to `ui-tauri/vite.config.ts`. Its renderer allowlist remains identical to
  native `lib.rs`; supervisor export/book-change protections are preserved.
- Reason: The full gate proved stale desktop exposure and an unsupported
  visibility command. Broad Vite expansion was rejected during extraction;
  it was not retried or restored. No functional CLI or opaque AI task tool
  requires direct renderer dispatch of these accounting commands.
- Command/tool: Scoped `apply_patch` extraction; no database or evidence edits.
