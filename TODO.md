# TODO

This file replaces the old extraction-only checklist. Treat the previous
"Step 7 / Step 8 / Step 9 / Step 10 / Step 11" plan as obsolete.

Current direction:

1. Keep the extracted `kassiber.core` / `kassiber.cli` split stable
2. Keep the CLI first-class and machine output stable while building on that core
3. Add the next cross-cutting features on top of the extracted core:
   attachments, tax-engine cleanup, Austrian RP2 integration
4. Keep desktop work layered on the shared core rather than reintroducing
   monoliths or duplicate logic

Backlog contract:

- `TODO.md` is the executable backlog and progress tracker.
- `docs/plan/` records goal state, architecture guardrails, and product
  boundaries; do not use plan docs as task lists.
- Active TODO items should keep the next actionable step and a concrete
  completion condition clear enough for agents to continue without guessing.
- When behavior moves, update the nearest current-truth doc, tests when needed,
  and the affected TODO item in the same change.
- If code and a plan doc disagree, trust the code, then update the plan doc so
  the drift does not survive.

## Rules for every session

Use `./scripts/quality-gate.sh` before calling work ready to push. It wraps the baseline compile, smoke, regression, and CLI help checks so humans and agents use the same verification path.

- Preserve the current JSON envelope contract and error shape
- Keep `tests/test_cli_smoke.py` as the behavior pin; prefer extending it
  over adding new test files
- Keep `db.py`, `envelope.py`, `errors.py`, `msat.py`, `backends.py`,
  `tax_policy.py`, `wallet_descriptors.py`, `transfers.py`, and
  `importers.py` as leaf modules unless moving them deletes real code
  from `app.py`
- Keep changes local-first, standard-library-first, and additive where
  possible
- Keep docs in lockstep with behavior changes
- When a TODO item is completed, check it in the same PR or commit that
  lands it; if scope changes, rewrite or split the item immediately
- Never use `git add -A`

## Right now

- [x] Finish the half-done `kassiber/importers.py` extraction:
  remove duplicate parser code from `kassiber/app.py`, import the shared
  helpers from `kassiber.importers`, then run the compile check and the
  CLI smoke suite
- [x] Map the current `app.py` seams before the next split:
  runtime/bootstrap, envelope/error emission, context resolution, SQL
  helpers, sync adapters, reports, and RP2 loading
- [x] Keep the repo in a behavior-preserving extraction mode until the
  shared core exists; do not start UI implementation work before Phase 0
  is green

## Phase 0 - Shared Core Extraction

Goal: turn `kassiber/app.py` into a thin CLI shim over reusable Python
modules without changing user-visible behavior.

### 0a - Skeleton and bootstrap

- [x] Create `kassiber/core/` and `kassiber/cli/` package skeletons
- [x] Add `kassiber/core/runtime.py` for shared data-root, env-file,
  settings, backend overlay, DB-open, and context bootstrap
- [x] Add `kassiber/cli/main.py` as the future argparse entrypoint
- [x] Keep the CLI cutover on `kassiber.cli.main:main`; the old `kassiber.app`
  shim has now been removed

### 0b - CRUD, settings, and rates

- [x] Extract workspace/profile/account/backend CRUD into `kassiber.core`
- [x] Extract wallet read/write orchestration into `kassiber.core`
- [x] Extract local rates cache operations into `kassiber.core`
- [x] Introduce small `core/repo/*.py` modules only where they simplify
  repeated SQL and reduce `app.py`

### 0c - Sync and import orchestration

- [x] Extract wallet sync orchestration into `kassiber/core/sync.py`
- [x] Keep parser-only logic in `kassiber/importers.py`
- [x] Split backend-specific sync code into dedicated modules only when it
  shrinks `app.py` and keeps the dependency graph clean
- [x] Preserve current `esplora`, `electrum`, and `bitcoinrpc` behavior

### 0d - Metadata, journals, reports, and RP2 seam

- [x] Extract metadata note/tag/excluded, records, and BIP329 flows into
  `kassiber.core`
- [x] Extract report builders/export paths into `kassiber.core`
- [x] Move `_emit_error` out of `app.py` and make the envelope boundary
  explicit
- [x] Move `_RP2_MODULES` / `get_rp2_modules` behind a journals or engine
  seam
- [x] Introduce a tax-engine interface that preserves today's RP2-backed
  generic behavior

### 0e - CLI cutover and cleanup

- [x] Move argparse tree and command dispatch into `kassiber/cli/main.py`
- [x] Update `pyproject.toml` entrypoints once the new CLI path is stable
- [x] Shrink `kassiber/app.py` to a tiny shim or delete it
- [x] Run the full smoke suite plus CLI help/smoke commands before
  calling Phase 0 done

### Phase 0 done when

- [x] `tests/test_cli_smoke.py` passes unchanged or only with deliberate,
  documented extensions
- [x] `kassiber/app.py` no longer contains business logic
- [x] RP2 imports are isolated to the extracted engine/journals surface
- [x] The CLI still emits the same machine envelope kinds and schema
  fields for existing commands

## Phase 0.5 - Attachments and Tax Engine Cleanup

Goal: add the next big capabilities on top of the extracted core, not on
top of the monolith.

### 0.5a - Shared tax-input normalization seam

- [x] Add normalized tax-event inputs between raw transactions and engine
  logic
- [x] Make ambiguous or under-specified tax semantics quarantineable
  instead of guessed
- [x] Keep raw `transactions` rows as the source of truth; do not persist
  derived regime state onto them

### 0.5b - RP2 extraction

- [x] Move the current generic RP2 journal flow behind the new engine seam
- [x] Preserve current transfer handling, quarantine behavior, and report
  outputs
- [x] Use fixture comparisons and smoke tests to catch rounding or journal
  regressions

### 0.5c - Transaction attachments

- [x] Add attachment storage and metadata table
- [x] Add CLI commands for add/list/remove plus `attachments gc` and
  `attachments verify`
- [x] Make backup/restore aware of attachment files
- [x] Keep URL attachments string-only; no fetching or indexing

### 0.5d - Austrian tax support

- [x] Fork RP2 to `bitcoinaustria/rp2` so Austrian tax logic can live in the
  tax engine rather than expanding Kassiber-side tax math
- [x] Add Austrian country / accounting / report plugins in the RP2 fork
- [x] Keep Kassiber-side normalization, provenance capture, and wallet-bucket
  transfer preparation feeding the RP2-backed Austrian path
- [x] Re-enable Austrian profiles now that the RP2-backed path is wired,
  tested, and documented
- [x] Replace Option C (quarantine-on-cross-asset-Neu-swap) with Option A
  (topological two-pass compute) so `carried_basis_fiat` is populated
  automatically; see [docs/austrian-handoff.md](docs/austrian-handoff.md)
- [x] Add E 1kv CSV/PDF/XLSX export path on top of Kassiber's persisted
  Austrian disposal buckets / Kennzahl mapping, including structured
  Steuerbericht-style sections, a section-by-section CSV bundle, and
  `reports austrian-tax-summary` / `reports export-austrian` aliases
- [x] When E 1kv export lands, surface Kassiber's current ausländisch /
  self-custody Kennzahl assumption in the CLI envelope and PDF/XLSX output;
  domestic-provider withheld KESt needs structured metadata before
  Kassiber can populate the inländisch rows

### 0.5e - Rates and journal follow-through

- [x] Wire the rates cache into journal pricing where it improves the
  current behavior without breaking the smoke contract
- [x] Add explicit per-event overrides only after the core engine boundary
  is stable

### 0.5f - External document reconciliation groundwork

- [ ] Add [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md) follow-through in code and schema rather than letting merchant/invoice scope drift ad hoc
- [x] Persist BTCPay confirmed wallet-sync config on wallets so `wallets sync` / `wallets sync --all` can reuse store-backed sources without retyping `--store-id`
- [ ] Keep BTCPay file import conservative (`deposit` / `withdrawal`) until a confirmed document match or explicit review step reclassifies the transaction

## Phase 1 - Desktop UI

Goal: build the desktop UI as Tauri 2 + React + TypeScript with a Python
sidecar daemon, per [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md)
and [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md).

### 1.0 Prep cleanup (parallel-safe, no UI change)

- [ ] Publish `rp2` as a versioned wheel artifact and update `pyproject.toml`
  to consume it (eliminates the VCS-pinned packaging risk)
- [ ] Decompose `kassiber/cli/handlers.py` into per-domain `kassiber/core/api/`
  modules so each handler is a pure `(args_dict) -> envelope_dict` callable
- [ ] Centralize the safe-view contract in `kassiber/core/api/safe_views.py`
  so every consumer sees the same redaction
- [ ] Add `~/.kassiber/logs/` (or per-project `logs/`) with rotation; teach
  `diagnostics collect` to fold all logs

### 1.1 Daemon mode (no UI yet)

- [x] Add `kassiber/daemon.py` and a `kassiber daemon` subcommand
- [x] JSONL request/response with `request_id`, `daemon.ready`, and a first
  `status` round-trip
- [ ] Add `progress` envelopes and mutation-safe long-running request handling
  beyond the AI chat cancel path
- [ ] Worker pool with one SQLite connection per worker
- [x] Smoke coverage for daemon ready/status/shutdown
- [ ] Redaction audit in CI

### 1.2 Tauri shell skeleton + typed IPC + first screen

- [x] `ui-tauri/` workspace skeleton: Vite + React 19 + TS + Tailwind v4
  + TanStack Query/Router + Zustand + theme tokens (Bitcoin Austria palette)
  + bundled Blinker/JetBrains Mono fonts + mock daemon transport. Claude
  Design originals staged under `ui-tauri/claude-design/` for reference;
  translation lands in `ui-tauri/src/routes/` per phase 1.3.
- [x] Configure shadcn registries for `@shadcnblocks` and `@blocks-so`,
  keep the local development API key in ignored `ui-tauri/.env`, and
  document `SHADCNBLOCKS_API_KEY` via `ui-tauri/.env.example`
- [x] Add the shadcn primitives and block dependencies currently needed by
  the mock shell and first dashboards: button/card/dialog/input/label/select/
  table/sidebar/sheet/dropdown-menu/tooltip/scroll-area/separator/switch/
  textarea/chart/avatar/skeleton. Future screens should still install only
  what they actually use.
- [x] Replace the old Kassiber nav with the shared shadcn desktop shell:
  sidebar, route header, larger search, privacy toggle, settings/donate/
  bug-report actions, profile switcher, centered version label, and global
  pre-alpha banner
- [x] Add reusable shell-level assistant mockup based on `@blocks-so/ai-02`,
  with local-model selector, Kassiber-specific suggestions, collapsed-on-scroll
  behavior, and hover/focus expansion
- [x] Wire the in-app assistant to a real OpenAI-compatible client over the
  daemon protocol — provider config in SQLite, CLI parity
  (`kassiber ai providers/models/chat`), streaming chat with `<think>`
  reasoning split, and a Settings → AI providers panel
- [x] Split the Tauri daemon supervisor by `request_id` so one streaming
  `ai.chat` call no longer single-flights unrelated daemon invokes; keep one
  daemon process, one narrow stdin write lock, and one stdout demux reader.
- [x] Add cooperative AI chat cancellation through `ai.chat.cancel` and a
  per-request `threading.Event` so Stop suppresses further deltas and the
  terminal chat envelope reports `finish_reason: "cancelled"`.
- [x] AI read-only tool use (PR 3): expose typed safe daemon snapshots as the
  assistant's tool surface, seed the system prompt from compact
  `skills/kassiber/` guidance, add tool cards, and start with a bounded
  read-only-by-default tool loop.
- [x] AI mutating tool consent (PR 4): require explicit per-call/session
  approval before executing mutating tools such as wallet sync.
- [x] In-app AI skill-aware read upgrade (PR 72): expand the compact Kassiber
  prompt, add `read_skill_reference("index")`, expose granular read-only daemon
  tools for wallets, backends, quarantine, transfers, rates, workspace health,
  next actions, and filtered transactions, and keep raw shell/filesystem/CLI
  access out of scope.
- [x] Dev browser bridge for real local AI: Vite keeps a loopback-only Python
  daemon supervisor, demuxes JSONL by `request_id`, and streams `ai.chat`
  records to browser clients as NDJSON so Codex/browser tools can test local
  AI, Stop, tool cards, and consent without launching the Tauri webview.
- [ ] Daemon worker pool: replace the surgical `ai.chat` thread with a real
  worker-pool model and one SQLite connection per worker when read-only tools
  or longer-running UI actions need daemon-side concurrency beyond the
  supervisor demux.
- [x] Overview screen now uses `@shadcnblocks/dashboard5` as the first
  dashboard screen, keeping Export -> Reports, Add connection modal, and
  Show all transactions wiring
- [x] Transactions screen now uses `@shadcnblocks/dashboard2` as the
  transaction dashboard, with ordered period controls, enlarged search copy,
  and privacy visibility toggle in the header
- [x] Initial Tauri 2 shell bootstrap with `ui-tauri/src-tauri/`, a locked
  CSP, a minimal capability file, and a whitelisted `daemon_invoke` command
  wired to the React daemon transport
- [x] Replace the temporary `daemon_unavailable` Tauri command body with a
  Rust supervisor that spawns the Python daemon and dispatches JSONL by
  `request_id`
- [ ] Generate the Rust daemon kind allowlist from Pydantic contracts instead
  of the current hand-maintained bootstrap list
- [ ] Pydantic v2 contracts to JSON Schema to TS types in CI; schema-drift
  fails the build
- [ ] Bridge mode containment tests (per
  [04-desktop-ui.md](docs/plan/04-desktop-ui.md) §2.6 + 2.7): negative
  tests for cross-origin / no-Origin / non-loopback bind / production-env
  startup / missing-or-wrong token / mutation-disabled-by-default;
  positive test that `daemon.log` and `supervisor.log` from a captured
  bridge session contain zero token occurrences. Each gate must be wired
  into the quality-gate before the bridge code is allowed to land.

### 1.3 Read-only screens

- [x] Overview dashboard shell using shadcn block components and mock daemon
  fixture data
- [x] Transactions dashboard shell using shadcn block components and mock
  daemon fixture data
- [x] Connections screen reshaped to the shared shadcn dashboard language,
  including connection metrics, source table, and the existing Add connection
  modal flow
- [x] Reports screen reshaped to the shared shadcn dashboard language,
  including capital-gains controls, preview table, and CSV/PDF/XLSX export
  format cards
- [x] Settings modal restyled with shadcn dialog/card/switch/input/select
  primitives, wider dashboard layout, daemon-backed lock controls, SQLCipher
  passphrase rotation, and passphrase plus workspace-name confirmation before
  deleting encrypted local workspace data
- [ ] Profiles screen
- [x] Welcome/onboarding screen refreshed with a shadcn-style, SQLCipher-aware
  setup flow that captures workspace/profile/tax defaults and database
  protection by initializing the local SQLCipher database through the daemon,
  lets users choose the default mempool/custom/skip backend setup with an
  explicit skip warning, keeps Austrian onboarding on moving-average-only
  current-rule defaults without profile-level long-term exemption controls,
  captures optional AI assistant intent with a disable-for-now button, imports
  existing local project/profile roots through the native desktop picker, and
  offers a dev-only mock preview shortcut.
- [x] Replace the Overview mock fixture with a read-only
  `ui.overview.snapshot` daemon kind backed by the current SQLite profile
- [x] Replace the Transactions table mock fixture with a read-only
  `ui.transactions.list` daemon kind and enable a loopback-only Vite dev
  bridge for browser testing against real local data
- [x] Replace Connections, Journals, and capital-gains Reports mock fixtures
  with first real daemon-backed snapshots and a `ui.wallets.sync` action
- [x] Wire Reports export cards to daemon-backed managed exports for PDF,
  capital-gains CSV, and Austrian E 1kv XLSX, with default-app opening for
  completed files
- [ ] Replace remaining Settings mock fixture data with typed daemon calls once
  phase 1.1 exists

### 1.4 Live actions and workers

- [ ] Sync, imports, journals process, metadata edits, transfer pairing,
  attachments, quarantine resolve, profile/wallet/backend CRUD,
  backup/restore
- [ ] Progress + cancellation UI
- [ ] Separate secret-entry IPC channel; OS-keychain-backed secret refs

### 1.5 Packaging, signing, distribution

- [x] Automated unsigned prerelease CLI binaries for macOS and Linux via
  GitHub Actions
- [x] Automated unsigned desktop preview artifacts for macOS, Linux, and
  Windows via GitHub Actions
- [x] Bundle one-file Kassiber CLI sidecars into unsigned desktop preview
  artifacts so preview apps do not require an external Python checkout and can
  forward installed-app CLI calls via `--cli`
- [ ] Add build metadata (`BUILD_INFO.json` or equivalent) to every
  prerelease artifact with commit, ref, run id, and build timestamp
- [ ] Decide whether production installers should keep the PyInstaller sidecar
  or switch to a `python-build-standalone` runtime tree
- [ ] Tauri bundler per OS; Apple Developer ID, Windows EV, GPG `.deb`
- [ ] User-initiated update check only; no background polling

## Later backlog

- [ ] Custom CSV mapping DSL for arbitrary wallet exports
- [ ] Rates/manual adjustment surface
- [ ] Full double-entry account model only if a future ledger design needs it:
  explicit counterpart postings, account-type rollups, adjustments, and
  migrations; current `accounts` are wallet/reporting buckets
- [ ] Per-profile Tor proxy configuration
- [ ] Extend BTCPay Greenfield sync beyond confirmed wallet history with stable invoice/payment ids and raw payload snapshots
- [ ] External document records for invoices, receipts, contracts, and related BTC-linked business evidence
- [ ] Many-to-many document/payment links with allocations and reconciliation state
- [ ] Deterministic matching rules before any AI assistance
- [ ] Review/confirmation workflow for proposed matches and commercial annotations
- [ ] Split commercial annotations from RP2-facing tax primitives during journal preparation
- [ ] Accountant-facing export of matched BTC subledger rows with document references
- [ ] Opt-in local AI extraction and tie-breaking only after deterministic matching is solid
- [ ] Richer transfer pairing for multi-leg self-transfers
- [ ] Better cross-asset transfer accounting beyond audit metadata
- [ ] Revisit per-wallet basis attribution if a jurisdiction ever needs
  physical-lot answers
- [ ] Adopt a per-project storage layout: one SQLite DB per project,
  minimal global app state, and no active top-level wallet side tree
- [ ] Keep transaction document links in the project DB; only add managed
  copied-file storage if a concrete offline/self-contained workflow needs it
- [x] Keep backend definitions and default-backend selection canonical in
  SQLite; dotenv files now bootstrap older/new stores instead of serving as
  the long-term storage path
- [x] Keep normal backend and wallet success output safe-to-record for
  secret-bearing config values by redacting raw credentials and raw descriptor
  material while preserving presence / state flags
- [ ] Finish the project-local part of backend storage once the per-project
  DB layout lands
- [x] Add public-safe diagnostics reports for bug reports, with aggregate
  state shape, sanitized error context, and optional `exports/diagnostics/`
  artifacts
- [ ] Keep private `--debug` traces and any future downloadable logs separate
  from the public diagnostics contract
- [x] Add stdin/fd-based secret-input flows (`--token-stdin` / `--token-fd FD`,
  `--password-stdin` / `--password-fd`, `--descriptor-stdin` /
  `--descriptor-fd`, etc.) so hosted agents and shells never need raw values
  in argv. Argv forms (`--token <value>`, `--password <value>`,
  `--descriptor <value>`, …) are kept as deprecated, warning-on-use shims;
  remove them once `tests/test_review_regressions.py` migrates off argv.
- [ ] Strip the deprecated argv credential forms from the parser and update
  `tests/test_review_regressions.py` to use stdin/fd in every backend/wallet
  setup site. At the same time, harden the `backends.env` plaintext-secret
  warning into a refusal once no test or example needs the dotenv path for
  secret seeding.
- [ ] Extend the stdin/fd secret-input pattern to `ai_providers.api_key`:
  `kassiber ai providers {create,update}` currently only accepts
  `--api-key <value>` via argv. Add `--api-key-stdin` / `--api-key-fd FD`
  (matching the backends pattern) and a daemon-side `ai.providers.set_api_key`
  reveal/rotate flow so hosted agents never have to put raw API keys on the
  command line.
- [ ] Split wallet descriptor and other sensitive config out of the generic
  `wallets.config_json` blob into typed project-local tables now that
  SQLCipher protects the file at rest.
- [x] Encrypt `kassiber.sqlite3` at rest with SQLCipher 4 behind a user
  passphrase, with `kassiber secrets {init,change-passphrase,verify,status,migrate-credentials}`,
  `kassiber backup {export,import}`, `--db-passphrase-fd` plumbing through the
  CLI and daemon, and a `tar | age` single-file backup format.
- [x] Move backend secrets (token, password, auth_header, basic-auth username
  + RPC aliases) out of the plaintext `config/backends.env` bootstrap and into
  the encrypted `backends` table. `kassiber secrets migrate-credentials` lifts
  pre-existing entries with a `.pre-credentials-migration-<ts>.bak` snapshot,
  and `bootstrap_runtime` warns to stderr whenever the dotenv still contains
  secret-shaped entries while the DB is encrypted. URLs / kinds / chain /
  network stay in the dotenv (they are addresses, not credentials).
- [ ] Tauri supervisor wiring: passphrase modal at startup, private fd hand-off
  to the Python sidecar, `auth_required`/`auth_response` relay for reveal
  flows, and log redaction of `passphrase_secret` / `token` / `descriptor` /
  `change_descriptor` / `blinding_key` / `auth_header` / `password` envelopes.
- [ ] Cross-platform CI for SQLCipher: PyInstaller bundle smoke tests on
  macOS arm64/x86_64, Linux x86_64, Windows x86_64. Today's tracer bullet
  ran on macOS arm64 only.
- [ ] Optional convenience: opt-in OS-keychain remember-me layer and biometric
  reveal gate. Both are convenience over the SQLCipher passphrase, never a
  cryptographic substitute.
- [x] Kassiber skill bundle for agents (`skills/kassiber`)
- [ ] Optional server/REST mode, still local-first and opt-in

## Open bugs and debt

- [ ] PDF report rendering is Latin-1 only after the PySide6 removal:
  `_ascii_text` in `kassiber/pdf_report.py` silently replaces every
  codepoint outside Latin-1 with `?` in exported PDFs — notably `€`,
  `₿`, arrows like `↔`, and any non-European script. German umlauts
  and `ß` are inside Latin-1 and survive, but the Euro sign in
  Austrian E 1kv exports does not. The current behavior is pinned by
  `test_pdf_report_substitutes_non_latin1_glyphs` in
  `tests/test_review_regressions.py`. Follow-up: pick a Unicode-safe
  renderer (`reportlab` / `fpdf2` / `weasyprint`), embed at least one
  Unicode-capable font, and flip that pin to assert preservation
  instead of substitution. Acceptance criterion for the follow-up PR:
  `test_austrian_e1kv_report_exports_summary_csv_pdf_and_xlsx` asserts
  the rendered PDF bytes contain `b"\xe2\x82\xac"` (the UTF-8 bytes
  for `€`) and at least one representative non-ASCII user-text token,
  and `test_pdf_report_substitutes_non_latin1_glyphs` flips from
  substitution to preservation. Until then, treat exported PDFs that
  contain Euro signs or non-Latin-1 user content as not audit-grade.
- [ ] Decide the permanent substitute-vs-fail policy for PDF rendering
  as part of the Unicode-renderer follow-up. Pre-release ships the
  silent-substitute behavior because we have no users yet and want
  exported PDFs to keep working through the rewrite. The follow-up
  must pick one of: (a) preserve all input glyphs (default once the
  Unicode renderer lands), (b) fail loudly with a structured
  `code: "pdf_unrepresentable"` error envelope listing the offending
  codepoints, (c) ship both with a `--strict-unicode` flag. Document
  the choice in [pdf_report.py](kassiber/pdf_report.py) and link the
  decision rationale from this entry.
- [ ] Fix `rates set` pair validation so malformed syntax like `BTCUSD`
  is rejected cleanly
- [ ] Keep the machine envelope boundary centralized and explicit
- [ ] Keep docs and examples Bitcoin-only
- [ ] Add a narrow docs-drift check for shared command / verification /
  safe-to-record surfaces so `README.md`, `AGENTS.md`, `SECURITY.md`, and
  `skills/kassiber/` do not quietly diverge

## Verification checklist

Run these after any extraction or behavior change:

- `PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc uv run python -m py_compile kassiber/*.py`
- `uv run python -m unittest tests.test_cli_smoke -v`
- `uv run python -m kassiber --help`
- `uv run python -m kassiber --machine status`
- `uv run python -m kassiber backends list`
- `uv run python -m kassiber wallets kinds`
- `uv run python -m kassiber profiles create --help`
- `uv run python -m kassiber metadata records --help`
- `uv run python -m kassiber attachments list --help`
- `uv run python -m kassiber journals events --help`
- `uv run python -m kassiber reports balance-history --help`
- `uv run python -m kassiber rates --help`
