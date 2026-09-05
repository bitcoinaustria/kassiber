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
