# Desktop UI Implementation Plan

**Status:** Accepted. Implementation guide for [01-stack-decision.md](01-stack-decision.md).
**Date:** 2026-04-25.
**Current source of truth:** this file plus code as the build progresses.

## Scope and preconditions

Build the desktop UI as Tauri 2 + React + TypeScript with a long-lived
Python sidecar daemon. The CLI surface (`kassiber` command), SQLite system
of record, RP2 tax engine, attachment store, and `--machine` JSON envelope
contract stay unchanged. No business logic moves languages.

Preconditions before each shipping milestone:

1. AGPL legal opinion on subprocess linkage to Tauri/JS bundle on file (or
   "residual risk accepted" sign-off) — required before Phase 5 ships.
2. `rp2` published as an installable wheel artifact (see Phase 0 below) —
   required before Phase 5 ships. The current `git+https://...` pin in
   [pyproject.toml](../../pyproject.toml) is fragile for a sidecar bundle.

Phase 0 work is safe to start independently — it's all cleanup that benefits
the project regardless of UI shape.

## Working principles (cross-cutting)

These thread through every phase. Treat them as P1 invariants, not
aspirations.

### Local-first stays absolute

- **No background network calls from the Tauri shell.** The webview makes
  zero outbound HTTP. Every request goes through Rust → Python daemon →
  existing sync code paths. The set of external endpoints in
  [SECURITY.md](../../SECURITY.md) does not grow because of this build.
- **No update polling.** No `analytics`, `crashlytics`, `error reporter`,
  or `phone-home` in any layer. Update checks are user-initiated only.
- **No CDN-loaded fonts/assets.** Everything bundled with the app.

### Webview is untrusted

The webview is a sandbox boundary, not a trust boundary. Treat it as if a
malicious page could appear there.

- **CSP locked to `'self'` + the IPC channel.** No remote script, no inline
  script, no `eval`. CSS allowed inline only if a build-time constraint
  forces it (it usually doesn't with Tailwind).
- **Tauri capability allowlist is minimal.** No `fs:`, `shell:`, `http:`,
  `dialog:` permissions to the webview by default. Every needed capability
  is added explicitly with the smallest scope (e.g., `dialog:open` for file
  pickers — not `fs:read-all`).
- **All filesystem and network access goes through Rust commands** that
  validate args and forward to the Python daemon.
- **Wallet config secrets never reach the webview.** The daemon serves the
  existing safe-view shape (`has_token`, `descriptor_state`, etc.). The
  TypeScript types reflect that — there is no `WalletFullConfig` type
  exposed to React at all.

### Vibeability is engineered, not accidental

- **One source of truth per concept.** Pydantic models on the Python side
  generate JSON Schema, which generates TypeScript types. Frontend AI never
  sees a hand-maintained type file that has drifted.
- **HMR end-to-end.** Vite for the web layer; daemon hot-reload via watch
  mode in dev so backend handler tweaks don't require a full rebuild.
- **Mock daemon for offline UI work.** Generated from the same JSON Schema
  with fixture responses, so screen iteration doesn't require a live DB or
  a real wallet.
- **Stack picks match v0/Lovable/Artifacts defaults** (React + TS +
  Tailwind + shadcn/ui + TanStack). Any AI-generated screen drops in with
  minimal reshaping.
- **Browser dev mode is a first-class workflow.** The same Vite dev
  server that the Tauri shell loads is reachable directly at
  `http://localhost:5173` from any browser — including Codex's in-app
  browser, Claude in Chrome, Claude Preview MCP, and any future
  AI-driven browser tool. See section 2.7 below for the concrete setup.

### Secrets stay out of stdout

Today secrets enter via CLI args (warned in
[SECURITY.md](../../SECURITY.md) as a shell-history risk). The daemon model
shifts the risk surface:

- **Daemon does not log raw request args** — only `kind`, `request_id`,
  duration, and an explicit allowlist of safe argument keys. Secret-bearing
  fields are redacted in logs the same way the CLI safe-view contract does.
- **Secret entry uses a dedicated IPC channel** with a one-shot, redacted
  payload. The Rust supervisor forwards once, never persists, and the
  daemon hands directly to the secret store (OS keychain when that lands;
  SQLite for now).
- **No secret value ever appears in a `progress` envelope or error**
  envelope. If an error must reference a secret-bearing field, it cites the
  field name only.

## Phase 0 — Prep cleanup (parallel-safe, no UI change)

**Goal:** unblock the daemon work and the Tauri build without touching
user-visible behavior. Every Phase 0 item is independently valuable.

### 0.1 Publish `rp2` as a wheel artifact

[pyproject.toml](../../pyproject.toml) currently pins
`rp2 @ git+https://github.com/bitcoinaustria/rp2.git@<sha>`. This is fragile
for any packaged build.

- Set up a `kassiber-rp2` (or `bitcoinaustria-rp2`) wheel build in the
  `bitcoinaustria/rp2` repo CI.
- Publish to GitHub Releases (or a private index) with a stable name and
  semantic version tags.
- Update [pyproject.toml](../../pyproject.toml) to reference the published
  wheel by version, with the git URL retained as a fallback for dev work
  via an extras group like `[dev-from-source]`.
- Confirm `pip install kassiber` works against a clean Python from the
  published artifact only.
- Update [README.md](../../README.md) installation section.

**Verification:** `./scripts/quality-gate.sh` passes against the
wheel-installed build. CI matrix adds a "wheel-only" install row.

### 0.2 Decompose `kassiber/cli/handlers.py`

Per [AGENTS.md](../../AGENTS.md), `handlers.py` is "remaining CLI command
handlers and compatibility-layer imports while deeper decomposition
continues." Phase 1 (daemon) needs each handler to be callable with
`(args_dict) -> envelope_dict` shape. Pre-position by:

- Extracting handler bodies into per-domain modules under
  `kassiber/core/api/` (e.g., `kassiber/core/api/wallets.py`,
  `kassiber/core/api/journals.py`).
- The CLI argparse path becomes a thin adapter: parse args → call core API
  → hand the dict to `kassiber/envelope.py:emit`.
- The daemon path (Phase 1) calls the same core API directly.

This is an additive refactor with the smoke suite as the pin. No CLI
behavior changes.

**Verification:** [tests/test_cli_smoke.py](../../tests/test_cli_smoke.py)
and `tests/test_review_regressions.py` pass unchanged.

### 0.3 Centralize the safe-view contract

Today wallet/backend safe views live next to their handlers. For the Tauri
boundary, the safe-view boundary becomes load-bearing — the webview must
**only** see safe shapes.

- Move safe-view projections into `kassiber/core/api/safe_views.py` so
  they're importable from both the CLI and daemon paths.
- Add a per-domain `to_safe_view()` plus an explicit `RAW_ONLY` marker for
  any field that must never leave the daemon. Type hints make this
  enforceable.

**Verification:** existing wallet/backend smoke assertions still pass; new
unit tests cover that `to_safe_view()` strips the documented sensitive
keys.

### 0.4 Add a logging directory contract

Today there is no central log dir. Add `~/.kassiber/logs/` (or per-project
`logs/` once the project-bundle migration lands per
[03-storage-conventions.md](03-storage-conventions.md)) with rotation
rules:

- `cli.log` — current CLI invocations (rotated daily, 7d retention).
- `daemon.log` — added in Phase 1.
- `supervisor.log` — added in Phase 2 (Rust side).
- `webview.log` — added in Phase 2 (intercepted webview console).

Logs follow the same redaction rules as the safe-view contract. The
`diagnostics collect` command grows to fold all logs.

**Verification:** logs appear, redaction is unit-tested,
`diagnostics collect` includes them in its sanitized envelope.

## Phase 1 — Daemon mode (no UI yet)

**Goal:** add a long-lived Python process that speaks JSONL and serves the
same envelopes the CLI emits. **No** UI work happens here.

### 1.1 Add `kassiber/daemon.py`

```text
kassiber/
  daemon.py          # NEW: stdin/stdout JSONL loop
  cli/
    main.py          # gains a `daemon` subcommand → kassiber.daemon.run()
  core/
    api/             # already split out in Phase 0.2
```

The daemon main loop:

1. Reads newline-framed JSON from stdin.
2. Validates against a Pydantic request model (Phase 2 wires the typed
   schemas; Phase 1 can use a hand-written dispatch for bootstrap and
   tighten in Phase 2).
3. Routes `kind` to the matching `kassiber.core.api.*` callable.
4. Writes one or more newline-framed JSON envelopes to stdout, each tagged
   with the same `request_id`.
5. Closes cleanly on EOF or on `kind: "daemon.shutdown"`.

### 1.2 Concurrency model

- One stdin reader.
- A worker pool sized 1 by default; configurable via env. SQLite
  serialization rules: one connection per worker, no sharing.
- Long ops emit `kind: "progress"` envelopes interspersed with other
  responses. Each progress event carries the originating `request_id`.
- A `kind: "cancel"` request matched to a `request_id` cooperatively
  interrupts. Workers check a per-request cancel flag at safe seams (per-
  page fetch, per-row processing).

### 1.3 Envelope additions

Existing success/error envelopes already match the docs. Add:

```jsonc
// streamed progress; unsolicited but always tied to a request_id
{"request_id":"r-42","kind":"progress","schema_version":1,
 "data":{"step":"sync.fetching","done":12,"total":80,"detail":"history"}}

// daemon-internal lifecycle
{"kind":"daemon.ready","schema_version":1,"data":{"version":"0.21.0"}}
{"kind":"daemon.shutdown","schema_version":1,"data":{}}
```

The CLI machine path keeps emitting one envelope per process exit (no
`progress`, no `daemon.ready`). The daemon emits both.

### 1.4 Smoke + regression coverage

Extend [tests/test_cli_smoke.py](../../tests/test_cli_smoke.py) with a
`test_daemon_smoke.py` sibling that:

- Spawns `python -m kassiber daemon`, writes one request, asserts the
  envelope shape.
- Asserts `daemon.ready` lifecycle.
- Asserts a long op emits at least one `progress` followed by a final
  envelope.
- Asserts `cancel` shortens execution and the final envelope is an error
  with `code: "cancelled"`.
- Asserts redaction: a request that includes a secret-bearing arg does
  **not** echo the secret in any emitted envelope or log line.

### 1.5 Document the daemon

- Add `docs/reference/daemon.md` describing the JSONL contract, lifecycle,
  redaction rules, and how to consume it from a non-Tauri client (third
  parties may want this).
- Update [README.md](../../README.md) "Architecture" with one paragraph.
- Mark this phase complete in `TODO.md`.

**Verification gates for Phase 1:**

- `./scripts/quality-gate.sh` runs daemon smoke automatically.
- Memory profile of a 1-hour-idle daemon shows no leak.

## Phase 2 — Tauri shell skeleton + typed IPC + first screen

**Goal:** stand up the Tauri shell with the Overview screen rendered from
real local data. By the end of Phase 2, launching a Tauri build shows one
real screen.

### 2.1 Repository layout

```text
ui-tauri/
  src-tauri/             # Rust supervisor
    src/
      main.rs
      supervisor.rs      # spawns + monitors the Python daemon
      ipc.rs             # JSONL framing + request_id correlation
      capabilities/      # per-window capability sets
    tauri.conf.json
    Cargo.toml
  src/                   # React + TS
    main.tsx
    routes/
    components/
    daemon/              # generated client + transport
    schema/              # generated TS types (gitignored at first; built in CI)
  package.json
  pnpm-lock.yaml
  vite.config.ts
  tsconfig.json
```

The crate is **scoped narrowly**: supervisor lifecycle, JSONL framing, path
resolution, and Tauri command bindings. No tax, accounting, sync, or
storage logic in Rust ever.

### 2.2 Typed IPC contract (the vibeability lever)

This is the single most important piece of the build. Without it,
AI-generated frontend code hallucinates endpoints; with it, autocomplete
carries the entire surface.

- Define request and response models in `kassiber/core/api/contracts.py`
  using Pydantic v2.
- Add a `kassiber/scripts/dump_schema.py` that emits one combined JSON
  Schema for every `kind`.
- Add a CI step that runs the schema dumper and feeds it into
  `quicktype` (or `json-schema-to-typescript`) to produce
  `ui-tauri/src/schema/contracts.ts`.
- Generated TS includes per-kind argument types, response data types, and
  a discriminated union over `kind` for response handlers.
- Wrap that in a `daemon/client.ts` that exposes a typed function per
  kind, e.g., `daemon.reports.summary({profile})` returning a typed
  response.
- TanStack Query keys are derived from `kind` + args, with a single
  `useDaemon(kind, args, options)` hook.

Drift safety: the CI step fails the build if generated TS is stale
relative to Pydantic models. There is no hand-maintained TS contract file.

### 2.3 Rust supervisor

- Spawns the daemon as a child process at app launch with stdin/stdout
  piped, stderr captured to `supervisor.log`.
- Closes all other fds; sets a clean env with only the keys the daemon
  needs (e.g., `KASSIBER_DATA_ROOT`, `RUST_LOG` for the supervisor
  itself).
- Monotonic `request_id` allocator. Pending-request map keyed by
  `request_id` with channels for response delivery.
- Reads JSONL from daemon stdout, dispatches by `request_id` to the
  matching channel; broadcasts unsolicited envelopes (e.g.,
  `daemon.ready`) to a separate event bus.
- Restart policy: if the daemon exits unexpectedly, the supervisor logs
  and shows a non-dismissable UI banner ("Backend unavailable — see
  logs"). It does **not** auto-restart by default — accounting state
  surprises are worse than downtime.

### 2.4 Tauri capabilities and CSP

`tauri.conf.json` highlights:

```jsonc
{
  "app": {
    "security": {
      "csp": "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost; font-src 'self' data:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'"
    }
  },
  "bundle": {
    "resources": {
      "python/**": "python/"
    }
  }
}
```

Capabilities (`src-tauri/capabilities/main.json`):

- **Allowed:** `core:default`, `dialog:open` (for file pickers),
  `dialog:save` (for export targets), the custom `daemon:invoke` command
  we define.
- **Disallowed by omission:** `fs:`, `shell:`, `http:`, `process:exit`,
  `clipboard-manager:write` (only allowed if a copy-button feature ships
  later, with explicit scope).

Every `daemon:invoke` call in Rust validates the `kind` against an
allowlist generated from the same Pydantic schema, so the webview can't
ask the daemon to run an unknown command — even if the React layer is
ever compromised.

### 2.5 First screen: Overview

> **JSX prototype as seed.** Existing Claude Design JSX mockups are the
> visual and structural starting point for each screen. Drop the JSX into
> `ui-tauri/src/routes/`, mechanically translate inline styles or CSS to
> Tailwind classes, replace any hardcoded fixture data with `useDaemon(...)`
> calls, and keep `useState` / `useEffect` / form state as-is. This
> collapses the design → static → review → wire loop into a single
> "import JSX, swap data hooks" step per screen.

- Add a daemon `kind: "ui.overview.snapshot"` that returns the dashboard
  data the Overview screen needs.
- Render with shadcn/ui `Card` + finance-ledger styling. Tailwind config
  encodes the warm-neutral background, white work surfaces, restrained
  borders, serif/display accent + mono typography.
- Wire the chart with the shadcn/ui chart block (Recharts under the
  hood) against the existing balance-history data shape.
- TanStack Query handles the fetch with a 5-minute stale time; manual
  refresh through a kbd shortcut and a refresh button.

### 2.6 Browser dev mode (AI-friendly, daemon-optional)

The frontend ships in three runtime modes, all sharing the same React app:

1. **Mock daemon, plain browser.** `pnpm dev` runs Vite at
   `http://localhost:5173`. A `VITE_DAEMON=mock` env var swaps the daemon
   client for one that returns fixture responses generated from the same
   JSON Schema as the real types. Any browser-driven tool — Codex's
   in-app browser, Claude in Chrome, Claude Preview MCP, browser-MCP
   automations, design-review screenshotters — can drive the full UI
   without a Python install. **This is the default dev mode** for layout,
   styling, component iteration, and AI-assisted screen work.

2. **Real daemon over loopback bridge, plain browser.** `python -m kassiber
   daemon --bridge ws://127.0.0.1:8765 --token <random>` exposes the
   daemon over an authenticated localhost WebSocket, dev-only. With
   `VITE_DAEMON=bridge` and the token from a `.env.local`, the same
   Vite-served React app talks to a real DB. AI browser tools can drive
   real data flows. The bridge is **gated to development builds only** —
   refusing to start when `KASSIBER_ENV=production`, refusing connections
   without the token, and binding only to `127.0.0.1`.

3. **Tauri shell.** `pnpm tauri dev` launches the production-shaped
   webview with `daemon:invoke` Tauri commands, native dialogs, and the
   capability allowlist. Used for IPC-shape verification, native-dialog
   testing, and shell-startup measurement. Not directly drivable from
   AI browser tools (it's not a URL they can navigate to), but the
   webview's DevTools is accessible for debugging.

A dev script (`pnpm dev:browser`, `pnpm dev:bridge`, `pnpm dev:shell`)
selects the mode. The Vite config rejects `VITE_DAEMON` values other
than `mock` / `bridge` / `tauri` so production builds can't accidentally
ship the bridge transport.

### 2.7 Verification gates for Phase 2

- `tauri dev` works on macOS, Windows, Linux dev hosts.
- `tauri build` produces an installable artifact on all three. Bundle
  size recorded (target: < 150 MB after compression; **measure**, don't
  promise).
- Cold start from icon click to "Overview rendered" ≤ 2 s on a 2020-era
  laptop on macOS and Windows; ≤ 3 s on Linux. **Measure**, don't
  promise.
- TypeScript build clean: `pnpm typecheck` passes.
- ESLint passes with project config.
- `./scripts/quality-gate.sh` extended to include `pnpm typecheck` and
  `pnpm test --run` (Vitest).
- Schema drift check: `python -m kassiber.scripts.dump_schema --check`
  fails the build if Pydantic and `ui-tauri/src/schema/contracts.ts`
  disagree.
- A "shell-of-shells" smoke: launch Tauri, fetch overview, screenshot
  matches a stored baseline within tolerance.

## Phase 3 — Screen-by-screen port

**Goal:** read-only parity for every primary screen.

Order picked for compounding wins:

1. **Connections** — `routes/connections/[id].tsx`. Live wallet kinds vs.
   placeholders; uses the existing wallet kinds catalog. Surfaces
   BTCPay-backed wallet config the safe-view way.
2. **Transactions** — `routes/transactions/index.tsx`. First real win for
   the new stack: TanStack Table with virtualization, sortable columns,
   server-driven pagination using the existing cursor protocol from
   [AGENTS.md](../../AGENTS.md). Filters by tag, account, wallet, date
   range.
3. **Reports** — `routes/reports/*.tsx`. Read-only readiness + preview.
   Austrian E 1kv preview should match the section layout from
   [06-austrian-tax-engine.md](06-austrian-tax-engine.md).
4. **Profiles** — `routes/profiles/index.tsx`. Reads `profiles list`
   envelopes; surfaces `tax_country`, `gains_algorithm`,
   `tax_long_term_days`, `fiat_currency`.
5. **Settings** — `routes/settings/index.tsx`. Path readouts; safe
   redaction; the "Check for updates" link to GitHub releases (no
   auto-poll).
6. **Welcome** — `routes/welcome.tsx`. Empty-state onboarding text.

Each screen lands as one PR with:

- Generated TS types regenerated.
- One screenshot test.
- One Vitest unit test for the view-model translation (any TS-side
  data-shaping logic).
- An updated entry in `docs/reference/desktop.md`.

### Tables in detail (Transactions screen)

- Column set: occurred_at, kind, asset, quantity (msat-aware), fiat_value,
  wallet, account, tags, included.
- Quantity column shows BTC with appropriate precision; mouseover reveals
  msat for verification.
- Cursor pagination — no offset/limit. Use TanStack Query's
  `useInfiniteQuery` with `getNextPageParam` reading `next_cursor` from
  the envelope.
- Selection model: ranged multi-select for batch metadata operations
  (Phase 4).
- A "stale journal" indicator follows existing CLI semantics — if
  metadata changed since last `journals process`, show a banner.

### Reports preview detail

- Read-only previews with download buttons that call existing CLI
  `reports export-*` paths via the daemon. Files write to the user-chosen
  path through the `dialog:save` capability, never directly from the
  webview.
- Austrian E 1kv preview uses the daemon's structured JSON output, not
  the PDF. PDF generation stays server-side.

**Verification gates for Phase 3:**

- Each screen has a screenshot in the PR description.
- `./scripts/quality-gate.sh` extended with `pnpm test --run` covering
  view-model logic and a `playwright` smoke that opens each screen.
- No Pydantic schema regression — generated TS types stay current.

## Phase 4 — Live actions, workers, secret entry

**Goal:** every CLI mutation is reachable from the UI. Long ops emit
progress; users can cancel; secrets enter through a redacted channel.

### 4.1 Mutating actions in priority order

1. Sync (`wallets sync`, `wallets sync-btcpay`)
2. Imports (`wallets import-csv`, `wallets import-btcpay`,
   `wallets import-phoenix`, `metadata bip329 import`)
3. Journals process (with quarantine review UX)
4. Metadata edits (`metadata records note`, `tags add/remove`,
   `excluded set/clear`)
5. Transfers pair / unpair
6. Attachments add / remove / verify / gc
7. Quarantine resolve (price override, exclude)
8. Report exports (PDF / XLSX / CSV bundle) via `dialog:save`
9. Profile / wallet / backend / account create / update
10. Backup / restore (via SQLite backup APIs per
    [03-storage-conventions.md](03-storage-conventions.md))

Each gets:

- A daemon `kind` already exposed via the CLI machine path.
- A typed mutation hook (`useDaemonMutation`).
- A progress UI for long ops with cancel.
- Cache invalidation rules in TanStack Query that mirror the existing
  "reprocess journals after metadata change" semantics — the UI shows a
  banner; it does **not** auto-rerun journals (matches CLI).

### 4.2 Secret entry flows

The current "secrets via CLI args end up in shell history" issue has a
desktop equivalent: secrets in JSONL stdin could end up in
`supervisor.log` or `daemon.log` if logging is naive.

- Add a separate `secret_handle` IPC channel: the React side opens a Tauri
  command `secret_capture()` that pops a native modal collecting the
  secret. The Rust supervisor sends a single redacted envelope over a
  side-channel descriptor (a second pipe, **not** stdin) to the daemon.
- Daemon accepts the secret, writes it to the existing wallet/backend
  secret store, and returns only a `secret_handle_id`.
- All subsequent operations reference the secret by handle, never by
  value.
- Logs at every layer: the secret value never appears, only the handle.

This is also the right time to land the keychain integration mentioned in
[03-storage-conventions.md](03-storage-conventions.md) and `TODO.md` (OS
keychain refs). The secret handle becomes a keychain ref, and the SQLite
column stores the ref ID, not the raw value.

### 4.3 Cancellation contract

- Each mutation hook returns a cancel function bound to the `request_id`.
- Cancel sends `{kind: "cancel", request_id: <id>}` to the daemon.
- The daemon's worker checks a per-request cancel flag at:
  - Each esplora/electrum batch boundary
  - Each BTCPay page boundary
  - Each row in journal processing
  - Each PDF/XLSX section boundary
- A cancelled op returns `{kind: "error", error: {code: "cancelled", ...}}`.
- The UI shows "Cancelled" and rolls back any partial state through query
  cache invalidation; the SQLite store is the source of truth.

### 4.4 Verification gates for Phase 4

- Every mutation has a Vitest covering the optimistic update + cache
  invalidation contract.
- A Playwright "happy path" suite: create profile → add wallet → sync →
  process journals → run a report.
- A redaction audit: a script greps `supervisor.log` and `daemon.log`
  from a CI run for known-secret patterns and fails if any leak.
- Cancellation latency ≤ 500 ms p99 from cancel click to cancelled
  envelope.

## Phase 5 — Packaging, signing, distribution

**Goal:** a real signed installable on each desktop OS.

### 5.1 Python sidecar bundling

- Use `python-build-standalone` (the relocatable distribution `uv`
  consumes). Pin a specific tag per Tauri release.
- CI build per-OS:
  1. Download the standalone Python tarball.
  2. Create a venv in a fixed relative path under
     `ui-tauri/src-tauri/python/`.
  3. `pip install kassiber[ui-tauri]` (a new optional extra in
     [pyproject.toml](../../pyproject.toml) limited to runtime daemon
     deps).
  4. Strip non-essentials (no test files, no docs, no `__pycache__`).
  5. Sanity-run `python -m kassiber daemon < /dev/null`.
- Bundle size impact: measure and record. Target ≤ 80 MB compressed for
  the Python tree alone.

### 5.2 Tauri bundler config

`tauri.conf.json` `bundle` section per OS:

- macOS: `.app` + `.dmg`. Hardened runtime entitlements declared
  explicitly. No microphone/camera/network entitlements (we sync via the
  Python child, which is bundled inside the app and inherits app-level
  network policy).
- Windows: `.msi` and `.exe` (NSIS). EV certificate signing.
- Linux: `.deb` + `.AppImage`. `.rpm` if there's user demand.

### 5.3 Code signing

- macOS: Apple Developer ID Application certificate. Notarization step in
  CI. Stapled artifacts.
- Windows: EV cert from a vendor that supports the Tauri signing flow
  (Sectigo, etc.).
- Linux: GPG signing keys for `.deb`. AppImage signing optional.
- All signing keys live in CI secrets, never in the repo.

### 5.4 Update model

Per [01-stack-decision.md](01-stack-decision.md) and
[SECURITY.md](../../SECURITY.md): **no auto-update on launch.**

- Settings page has "Check for updates" link → opens the GitHub releases
  page in the user's default browser via `dialog:open` capability or
  Tauri's `open_url`.
- A future signed-manifest updater is opt-in only. If implemented, it
  - never polls automatically,
  - shows a clear "do you want to check now?" dialog on user click,
  - validates manifest signatures against a built-in pubkey,
  - lets the user always download instead of auto-installing.
- `User-Agent` for any update-check fetch matches the existing
  `kassiber/<version>` convention.

### 5.5 Verification gates for Phase 5

- One signed build per OS produced by CI on tag push.
- A non-dev-machine install test: download artifact on a fresh VM,
  install, launch, reach Overview within 5 s.
- macOS Gatekeeper: install passes without "untrusted developer"
  warnings.
- Windows SmartScreen: install passes without warnings (EV cert
  reputation builds over time; budget for a few weeks of warnings on
  first releases).
- Linux: `.deb` installs cleanly on Ubuntu LTS; `.AppImage` runs without
  extra deps on a clean GNOME desktop.

## Cross-cutting: security checklist

This is the audit checklist used at every phase gate. Each item must be
green before merging the phase's PRs.

- [ ] Webview CSP locked, no remote origins, no inline script.
- [ ] Tauri capability set is minimal; each capability has a one-line
      justification in `capabilities/main.json` comments.
- [ ] Daemon allowlist of `kind` strings is generated from Pydantic
      models, not hand-maintained.
- [ ] Webview never receives raw wallet/backend config; only safe views.
- [ ] Secret entry channel is separate from the JSONL stdin path.
- [ ] Logs at every layer (Rust supervisor, Python daemon, webview
      console) follow the redaction rules.
- [ ] `diagnostics collect` covers Tauri layers and is still public-safe.
- [ ] No telemetry, crash reporter, analytics, license-check, or
      auto-updater background polling.
- [ ] User-Agent for any outbound HTTPS still matches
      `kassiber/<version>`.
- [ ] AGPL: every JS/Rust/Python dep tracked in
      [THIRD_PARTY_LICENSES.md](../../THIRD_PARTY_LICENSES.md);
      CI fails on any new dep with a license incompatible with
      AGPL-3.0-only.
- [ ] Code-signing identities live in CI secrets, never in the repo.
- [ ] Bundled Python tree is read-only at runtime; user-writable state
      stays under `~/.kassiber/`.
- [ ] Bundled Python and Rust crates have a documented refresh cadence:
      rebuild releases when CPython or any shipped wheel issues a
      security advisory above a defined severity threshold; rerun
      `cargo audit` and refresh on every release tag.

## Cross-cutting: vibeability checklist

These should stay green so AI-assisted iteration stays smooth.

- [ ] `pnpm tauri dev` from a fresh checkout works in under 60 s.
- [ ] Pydantic → JSON Schema → TS regenerates in under 5 s.
- [ ] `tsc --noEmit` clean.
- [ ] HMR round-trip from a `*.tsx` save to visible change ≤ 2 s.
- [ ] A new screen prompt to AI ("add a sortable column for X") drops in
      with no manual type fix-ups.
- [ ] Mock-daemon mode gives a working UI without a Python install.
- [ ] Designer asset pipeline: a Figma export → Tailwind classes → screen
      lands within an afternoon for a typical card layout.
- [ ] `shadcn/ui` components are added via the CLI, kept in
      `src/components/ui/`, not abstracted behind a wrapper layer.

## Cross-cutting: doc update map

Every phase touches docs in lockstep with code. Map of which docs change
when:

| Phase | Docs that update |
|---|---|
| 0 | [README.md](../../README.md) install section, [pyproject.toml](../../pyproject.toml) deps, [THIRD_PARTY_LICENSES.md](../../THIRD_PARTY_LICENSES.md), `TODO.md` |
| 1 | New `docs/reference/daemon.md`, [README.md](../../README.md) architecture paragraph, [AGENTS.md](../../AGENTS.md) "Current architecture", `TODO.md` |
| 2 | [docs/reference/desktop.md](../reference/desktop.md), [01-stack-decision.md](01-stack-decision.md) verification numbers updated, this file |
| 3 | [docs/reference/desktop.md](../reference/desktop.md) per-screen, `TODO.md` desktop items mapped 1:1 |
| 4 | [SECURITY.md](../../SECURITY.md) secret-entry flow update, [docs/reference/desktop.md](../reference/desktop.md) mutation patterns |
| 5 | New `docs/reference/packaging.md`, [README.md](../../README.md) install section (now offers a download link), [SECURITY.md](../../SECURITY.md) update-policy paragraph |

## Open risks

These should be tracked at the project level, not silently accepted.

1. **Bundle size explosion.** Python tree + node-built JS could push the
   final bundle past 200 MB. Mitigation: measure early in Phase 2;
   prune unused stdlib modules in the relocatable Python tree if
   needed; lazy-load TS chunks per screen.
2. **WebKitGTK regressions on Linux.** Each major Linux distro release
   ships its own WebKitGTK; CSS bugs surface late. Mitigation: pin a
   tested distro for releases (Ubuntu LTS); test pre-release on Fedora;
   AppImage hides this least, `.deb` more.
3. **rp2 fork divergence.** Already flagged in
   [06-austrian-tax-engine.md](06-austrian-tax-engine.md). Mitigation:
   Phase 0.1 publishes a wheel with a stable versioning policy.
4. **Schema drift at the boundary.** A handler quietly returns a new
   field; TS types lag; AI iteration produces broken code. Mitigation:
   the schema-drift CI check fails the build.
5. **Three-language debugging fatigue.** A bug spans Rust supervisor,
   Python daemon, and React webview. Mitigation: unified
   `request_id` across all three logs; `diagnostics collect` includes
   all three; a "show last 50 envelopes" devtools panel in dev mode.
6. **Apple notarization quirks.** Notarization can reject builds for
   embedded native libs (Python's `.dylib` files). Mitigation: budget
   for a notarization-only first cycle in Phase 5; capture the
   entitlements pattern as a sticky CI step.
7. **Project-bundle storage migration collision.** Moving to per-project
   `~/.kassiber/projects/<name>/` per
   [03-storage-conventions.md](03-storage-conventions.md) is a separate
   migration. Don't couple. Mitigation: Tauri shell honors
   `--data-root` and the existing settings manifest; project-bundle
   work happens independently.
8. **Secret-handle regression risk.** Refactoring secrets from inline
   columns to handles touches every wallet/backend handler. Mitigation:
   ship the handle abstraction once, behind a feature flag; flip
   wallet adapters one at a time; gate on smoke + regression coverage.

## References

- [01-stack-decision.md](01-stack-decision.md) — desktop stack ADR
- [00-overview.md](00-overview.md) — product invariants and track status
- [03-storage-conventions.md](03-storage-conventions.md) — storage direction
- [06-austrian-tax-engine.md](06-austrian-tax-engine.md) — RP2 boundary
- [SECURITY.md](../../SECURITY.md) — local-first / network policy / secrets
- [AGENTS.md](../../AGENTS.md) — architecture and verification
- [pyproject.toml](../../pyproject.toml) — current dep set
- [kassiber/envelope.py](../../kassiber/envelope.py) — envelope contract
- Tauri 2: https://v2.tauri.app/
- Tauri capabilities: https://v2.tauri.app/security/capabilities/
- python-build-standalone: https://github.com/astral-sh/python-build-standalone
- shadcn/ui: https://ui.shadcn.com
- TanStack Query / Table / Router: https://tanstack.com
- Pydantic v2 → JSON Schema: https://docs.pydantic.dev
