# ADR: Desktop UI Stack

**Status:** Accepted.
**Decision:** Tauri 2 + React 19 + TypeScript + shadcn/ui, with the backend
running as a long-lived sidecar daemon over stdin/stdout JSONL. The backend
was Python when this ADR was written; since plans 21 and 22 it is the Rust
`kassiber` binary (see the amendments below).
**Date:** 2026-04-25.
**Current source of truth:** the `ui-tauri/` workspace, the daemon crate
(`crates/daemon`), the [daemon contract](../reference/daemon.md),
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
- **Server state:** TanStack Query against the daemon
- **Local UI state:** Zustand
- **IPC transport:** stdin/stdout JSONL with monotonic `request_id`
- **Sidecar bundling:** the Rust `kassiber` binary, packaged as a Tauri
  resource ([Rust packaging](../reference/rust-packaging.md)). Until plan 21's
  cutover this was a one-file PyInstaller CLI.
- **Packaging:** Tauri bundler — `.dmg`/`.app` (macOS), `.msi`/`.exe`
  (Windows), `.deb`/`.AppImage` (Linux)
- **Update model:** minimal release notification, never automatic download or
  installation. Installed desktop builds may query the fixed public GitHub
  releases endpoint after a short startup delay and daily while open; setup and
  **Settings → Privacy → Allow GitHub update checks** control the shared
  desktop/CLI permission. macOS exposes **Check for Updates…**, but the action
  also refuses network access while that permission is disabled.
  The notification trusts GitHub/TLS and is not an authenticated release
  announcement. Builds carry a versioned SHA-256 manifest; signed releases add
  a Sparrow-style detached OpenSSH signature from the pinned release key. The complete disclosure lives in
  [Privacy & security](../reference/privacy-and-security.md).

The shell's Rust scope stays small: process supervision, stdin/stdout
framing, OS path resolution, and Tauri command bindings. Tax, accounting, and
storage logic stays out of the shell. As first written, this paragraph kept
that logic in Python (`kassiber.core` and `rp2`); the amendments below moved
it to the Rust backend crates, which the shell still does not contain.

**Amended 2026-10-01:** [plan 20](20-kassiber-tax-engine.md) replaces RP2 with
a Kassiber-owned tax engine written in Rust, called from Python. Since that
plan's cutover, the engine is authoritative for tax calculations, and RP2 is
only a test oracle that the sidecar no longer bundles. Custody, storage, and
the daemon stay in Python until a later plan.

**Amended 2026-10-26:** [plan 21](21-rust-backend.md) moves the whole backend
to Rust: the sidecar becomes a Rust binary with the same CLI, daemon protocol,
and data, and Python is retired at its cutover stage.

**Amended 2026-11-10:** [plan 22](22-python-removal.md) removes Python from
the repository. The sidecar is the Rust `kassiber` binary; there is no Python
interpreter, PyInstaller bundle, or PyO3 binding. The last Python commit is
tagged `archive/python-backend`. The Python sidecar, `python-build-standalone`,
and Pydantic mentions below are the original rationale, kept as history.

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
│                                  │ kassiber sidecar (Rust)  │ │
│                                  │  CLI + daemon + tax      │ │
│                                  │  engine, one binary      │ │
│                                  │  long-lived daemon       │ │
│                                  └─────────────┬────────────┘ │
└────────────────────────────────────────────────┼─────────────┘
                                                 ▼
                                <state-root>/ (SQLite, attachments)
```

The webview never speaks to the sidecar child directly. Every call routes
through Rust Tauri commands, allowing future auth, rate-limit, or audit
layers without changing the frontend contract.

## Tradeoffs accepted

- **Three languages** (as accepted in 2026-04). Rust (small supervisor
  surface), TypeScript (frontend), Python (existing core). Since plan 22 it
  is two: Rust for the shell and the backend, TypeScript for the frontend.
- **IPC overhead per call.** Negligible at human-interaction frequency.
- **Sidecar packaging** is more work than a single-language path, paid
  mostly in CI. The sidecar was a PyInstaller CLI until plan 21's cutover; it
  is now the Rust binary, built per target in CI.
- **WebKitGTK quirks on Linux.** Each major Linux distro ships its own
  WebKitGTK. Test on Ubuntu LTS; budget a CSS-fixup pass per release.
- **Bundle size.** Larger than a pure-native build. Acceptable for desktop
  accounting; measure per release, do not promise final numbers.
- **Bundled dependencies are a CVE surface.** They don't update with the
  OS. Rebuild releases when a shipped crate (or, before plan 22, CPython or a
  shipped wheel) issues a security advisory above a defined severity
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

## References

- [04-desktop-ui.md](04-desktop-ui.md) — implementation plan
- [Privacy & security](../reference/privacy-and-security.md) — local-first / network policy / secrets
- Tauri 2: https://v2.tauri.app/
- Tauri capabilities: https://v2.tauri.app/security/capabilities/
- python-build-standalone: https://github.com/astral-sh/python-build-standalone
- shadcn/ui: https://ui.shadcn.com
- TanStack Query / Table / Router: https://tanstack.com
- Pydantic v2 → JSON Schema: https://docs.pydantic.dev
