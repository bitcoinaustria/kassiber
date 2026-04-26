# ADR: Desktop UI Stack

**Status:** Accepted.
**Decision:** Tauri 2 + React 19 + TypeScript + shadcn/ui, with the Python
core running as a long-lived sidecar daemon over stdin/stdout JSONL.
**Date:** 2026-04-25.
**Current source of truth:** `pyproject.toml`, the `ui-tauri/` workspace once
it lands, `kassiber/daemon.py` once it lands,
[04-desktop-ui.md](04-desktop-ui.md), and `TODO.md`.

## Decision

Adopt the following stack:

- **Shell:** Tauri 2.x (Rust + system webview)
- **Frontend:** React 19 + TypeScript + Vite
- **UI primitives:** shadcn/ui (Radix + Tailwind; copy-paste, no version lock)
- **Tables:** TanStack Table (virtualized, server-driven)
- **Charts:** shadcn/ui chart blocks (Recharts under the hood); reach for another lib only if a screen genuinely outgrows what shadcn charts cover
- **Forms:** React Hook Form + Zod
- **Routing:** TanStack Router
- **Server state:** TanStack Query against the Python daemon
- **Local UI state:** Zustand
- **IPC transport:** stdin/stdout JSONL with monotonic `request_id`
- **Python bundling:** current prerelease bundles use a one-file PyInstaller
  CLI sidecar packaged as a Tauri resource; production packaging may still
  switch to `python-build-standalone` if the runtime tree is easier to sign,
  debug, or update.
- **Packaging:** Tauri bundler — `.dmg`/`.app` (macOS), `.msi`/`.exe`
  (Windows), `.deb`/`.AppImage` (Linux)
- **Update model:** **no background update check** (preserves the
  [SECURITY.md](../../SECURITY.md) invariant). MVP ships a Settings link to
  the GitHub releases page. A signed-manifest, user-initiated "Check for
  updates" button via Tauri's updater is a follow-up decision, **off by
  default**, never poll on launch.

Rust scope stays small: process supervision, stdin/stdout framing, OS path
resolution, and Tauri command bindings. **No** tax, accounting, or storage
logic moves to Rust. `kassiber.core` and `rp2` remain authoritative in
Python.

## Why

- **Component ecosystem.** TanStack Table, shadcn/ui (charts and the rest),
  Recharts, and React Hook Form + Zod cover financial dashboards, accounting
  tables, and forms at production polish in person-weeks.
- **AI-assisted maintenance.** React + TypeScript + Tailwind + shadcn is
  among the densest training corpuses available. v0/Lovable/Artifacts all
  output exactly this stack natively; AI-generated screens drop in with
  minimal reshaping.
- **System webview, not bundled Chromium.** Patches arrive via OS updates.
  No `nodeIntegration` escape hatches; no Node runtime in the shipped app.
- **Existing IPC seam.** Every CLI command already emits the
  `{kind, schema_version, data}` machine envelope, designed for programmatic
  consumption. It carries cleanly over stdin/stdout JSONL — no protocol
  invented.
- **Security posture.** Tauri's deny-by-default capability system, locked
  CSP, Rust supervisor, and a `kind`-allowlist generated from Pydantic give
  a smaller and more granular sandbox than alternative GUI stacks.

## Topology

```
┌──────────────────────────────────────────────────────────────┐
│ Tauri app (single OS binary)                                 │
│                                                              │
│  ┌─────────────────────────┐    ┌──────────────────────────┐ │
│  │ Webview (system)        │    │ Rust core                │ │
│  │  React + TS + shadcn    │◄──►│  - Tauri commands        │ │
│  │  TanStack Query/Table   │IPC │  - sidecar supervisor    │ │
│  │  Recharts                │    │  - path resolution       │ │
│  └─────────────────────────┘    └─────────────┬────────────┘ │
│                                                │ stdin/stdout │
│                                                ▼ JSONL        │
│                                  ┌──────────────────────────┐ │
│                                  │ Python sidecar           │ │
│                                  │  python-build-standalone │ │
│                                  │  + kassiber + rp2        │ │
│                                  │  long-lived daemon       │ │
│                                  └─────────────┬────────────┘ │
└────────────────────────────────────────────────┼─────────────┘
                                                 ▼
                                ~/.kassiber/ (SQLite, attachments)
```

The webview never speaks to the Python child directly. Every call routes
through Rust Tauri commands, allowing future auth, rate-limit, or audit
layers without changing the frontend contract.

## Tradeoffs accepted

- **Three languages.** Rust (small supervisor surface), TypeScript
  (frontend), Python (existing core). Mitigated by keeping Rust scope tight
  and TypeScript the only genuinely new surface.
- **IPC overhead per call.** Negligible at human-interaction frequency.
- **Sidecar packaging** is more work than a single-language path, paid
  mostly in CI. PyInstaller gives the prerelease path a self-contained CLI
  quickly; `python-build-standalone` remains the production alternative if the
  one-file sidecar becomes hard to sign, debug, or patch.
- **WebKitGTK quirks on Linux.** Each major Linux distro ships its own
  WebKitGTK. Test on Ubuntu LTS; budget a CSS-fixup pass per release.
- **Bundle size.** Larger than a pure-native build. Acceptable for desktop
  accounting; measure per release, do not promise final numbers.
- **AGPL subprocess linkage.** Widely treated as arms-length aggregation;
  obtain a written legal opinion before public release.
- **Bundled Python is a CVE surface.** Versions don't auto-update with OS
  Python patches. Documented refresh cadence: rebuild releases when CPython
  or any shipped wheel issues a security advisory above a defined severity
  threshold.

## Alternatives considered

- **Electron.** Ships bundled Chromium plus a Node runtime in every app.
  Larger surface, slower OS patch arrival, and a class of `nodeIntegration`
  escapes Tauri does not have. Rejected.
- **Tauri + Leptos / Dioxus / Yew (Rust → WASM).** Avoids JavaScript, but
  the component ecosystem is dramatically smaller than React's, and AI
  assistance produces less reliable UI code. Rejected for this product.
- **Native GUI frameworks (Slint, Flet/Flutter, GTK).** Each trades the
  ecosystem advantage of Tauri+React for runtime/footprint claims that
  don't outweigh the productivity gap for this app. Rejected.

## Verification

Bundle size and cold-start are measured per OS at each release tag and
recorded in this file. Targets (not promises):

- bundle ≤ 150 MB compressed
- cold start (icon click → first screen rendered) ≤ 2 s on macOS/Windows,
  ≤ 3 s on Linux, on a 2020-era laptop

A signed legal opinion on the AGPL subprocess-linkage question is on file
before the first public release.

## References

- [04-desktop-ui.md](04-desktop-ui.md) — implementation plan
- [SECURITY.md](../../SECURITY.md) — local-first / network policy / secrets
- Tauri 2: https://v2.tauri.app/
- Tauri capabilities: https://v2.tauri.app/security/capabilities/
- python-build-standalone: https://github.com/astral-sh/python-build-standalone
- shadcn/ui: https://ui.shadcn.com
- TanStack Query / Table / Router: https://tanstack.com
- Pydantic v2 → JSON Schema: https://docs.pydantic.dev
