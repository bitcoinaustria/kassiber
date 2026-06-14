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

- [x] Land Lightning adapters on top of the
  [`kassiber.core.lightning`](kassiber/core/lightning/) scaffold. The
  scaffold ships the `NodeSnapshot` / `NodeChannel` / `NodeForward`
  shapes, a `LightningAdapter` Protocol with `register_adapter` /
  `resolve_adapter`, a generic profitability report, daemon kinds
  (`ui.connections.node.snapshot`,
  `ui.reports.lightning_profitability`), CLI commands
  (`reports lightning-profitability`,
  `reports export-lightning-profitability-csv`), AI tool registrations,
  and the desktop wiring. The adapters ship as stacked PRs on the
  shared scaffold: PR #158 adds `kassiber/core/lightning/lnd.py`
  (replacing #154) and PR #159 adds `kassiber/core/lightning/cln.py`
  (replacing #155); both PRs are scoped to a single adapter so review
  stays focused per-implementation.
- [x] Add watch-only UTXO inventory for chain-backed wallet sources:
  source refresh now persists current unspent outputs for Esplora/Electrum
  descriptor/xpub/address wallets and Bitcoin Core address wallets, exposes
  the redacted `ui.wallets.utxos` daemon surface, and renders a desktop
  wallet-detail UTXOs table with loading, empty, unsupported, stale, refresh,
  sorting, explorer-link, and Liquid-unblind-blocker states.
- [x] Add the shared privacy-import substrate for future wallet importers:
  typed transaction-level privacy-boundary markers, generic privacy-hop tax
  quarantine, source-funds opaque-boundary warnings, and typed desktop
  source-format plumbing. Protocol-specific Wasabi coin-anonymity fields and
  Samourai/Whirlpool importers landed in separate follow-up PRs.
- [x] Add Wasabi Wallet watch-only import and privacy-hop evidence handling:
  sanitized `wasabi_bundle` imports normalize `gethistory`, refresh durable
  Coins/UTXO anonymity state from `listcoins` / `listunspentcoins`, preserve
  safe wallet metadata only, wire CLI/daemon/desktop catalog surfaces, and
  mark ambiguous CoinJoin/PayJoin-style evidence as `privacy_hop_unresolved`
  instead of guessing provenance or taxable proceeds.
- [x] Add Samourai/Whirlpool watch-only recovery import: local backup v1/v2
  decryption, mnemonic and explicit descriptor/xpub source-set import,
  Deposit/Badbank/Premix/Postmix/Ricochet child sources, redacted daemon/CLI
  output, UTXO provenance, desktop import/detail surfaces, internal
  Tx0/mix/remix tax handling, and source-funds privacy-boundary suggestions.
  Deferred: coordinator-backed mix-status fetches, exact round metadata beyond
  imported/local evidence, and active Whirlpool client behavior remain out of
  scope.
- [ ] Design an opt-in encrypted Lightning **evidence vault** for
  operators who need proof-of-payment for legal disputes, full invoice
  replay for corrupted-bookkeeper recovery, or chain-of-custody records
  for audits. The vault must live separately from the normal daemon /
  AI / diagnostics surfaces — see Tier 1 in
  [`docs/reference/lightning-opsec.md`](docs/reference/lightning-opsec.md).
  The default discard policy on adapters does not change; the vault is
  an explicit additive export workflow.
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
- [x] Add CLI commands for add/list/rename/remove plus `attachments gc` and
  `attachments verify`
- [x] Make backup/restore aware of attachment files
- [x] Keep URL attachments reference-only; no fetching or indexing. Kassiber
  derives an editable display label from the URL itself

### 0.5d - Austrian tax support

- [x] Fork RP2 to `bitcoinaustria/rp2` so Austrian tax logic can live in the
  tax engine rather than expanding Kassiber-side tax math
- [x] Add Austrian country / accounting / report plugins in the RP2 fork
- [x] Keep Kassiber-side normalization, provenance capture, and wallet-bucket
  transfer preparation feeding the RP2-backed Austrian path
- [x] Re-enable Austrian profiles now that the RP2-backed path is wired,
  tested, and documented
- [x] Replace Option C (quarantine-on-cross-asset-Neu-swap) with native
  rp2 multi-asset carry so Austrian swap basis stays in the tax engine;
  see [docs/austrian-handoff.md](docs/austrian-handoff.md)
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
- [x] Store transaction pricing provenance and exact decimal strings so
  imported prices, manual overrides, provider FMV, and coarse fallback are
  distinguishable during journal processing
- [x] Add explicit per-event overrides only after the core engine boundary
  is stable

### 0.5f - External document reconciliation groundwork

- [ ] Add [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md) follow-through in code and schema rather than letting merchant/invoice scope drift ad hoc
- [x] Persist BTCPay confirmed wallet-sync config on wallets so `wallets sync` / `wallets sync --all` can reuse store-backed sources without retyping `--store-id`
- [x] Let desktop BTCPay setup either create BTCPay-only wallet sources or map store payment methods onto existing settlement wallets for provenance
- [ ] Keep BTCPay file import conservative (`deposit` / `withdrawal`) until a confirmed document match or explicit review step reclassifies the transaction

### 0.5g - Source-of-funds reports

- [x] Add [docs/plan/09-source-of-funds.md](docs/plan/09-source-of-funds.md) follow-through in code and schema rather than generating compliance PDFs from unreviewed heuristics
- [x] Add reviewed root-source and transaction-flow-link tables that reuse the existing attachment store for evidence
- [x] Add explicit allocation, immutable case snapshot, and PDF export-gate semantics before schema work hardens
- [x] Add CLI review surfaces for source-funds sources, link suggestions, manual links, and data-quality gaps
- [x] Add `reports source-funds --target-transaction ...` as a machine-readable path graph with `explain_gates` blockers before adding a PDF renderer
- [x] Add plain/table rendering for source-funds gaps and graph review before PDF export work
- [x] Add `reports export-source-funds-pdf` only after graph nodes, edge allocations, reveal modes, and missing-history markers are stable
- [x] Deepen desktop source-funds editing beyond the first daemon-backed preview/export screen: accept/reject links, edit allocations, attach evidence, and mark reviewed missing-history gaps in-app
- [x] Make report purpose explicit in the desktop workflow and report payload: planned exchange sale / bank pre-disclosure versus already-completed transaction
- [x] Add deterministic-hop bulk review for consolidation/self-transfer chains while keeping weak time/amount and chain-observation suggestions manual
- [x] Add local report-shape rollups for overview, narrative, data sources, source mix, simplified flow charts, and level-by-level flow details, and simplify the desktop default workflow while keeping advanced review controls optional
- [x] Add a basic Austria/EUR source-funds report context with bilingual title, evidence checklist, and fictitious local demo generator
- [x] Add a DB-backed Reports audit package export for trusted auditor
  handoff: deterministic `manifest.json`, selected managed evidence files,
  URL references as labels/links, transaction evidence readiness warnings,
  source-funds review state, journal context, and explicit exclusion of
  descriptors/xpubs/backend credentials/logs/AI settings/technical wallet
  evidence.
- [x] Add manual transaction-detail evidence reuse: choose another transaction,
  copy selected URL references as new rows, duplicate managed file attachments
  under new attachment ids, preserve provenance, and surface copied evidence in
  readiness/audit package manifests.
- [x] Match strict exchange-facing granularity in the report: per-transaction
  fee and import-provenance columns (chain sync / platform export / manual
  import), level-grouped PDF transaction detail tables with in/out amounts and
  per-level fiat subtotals, provenance-based data-source ring, root-source
  detail table, contents list on the cover, a missing-history section with
  unexplained amounts, and a disclosure-footprint summary (wallets named +
  common-ownership note) in payload, PDF, and desktop preview.
- [x] Auto-assemble the flow graph from real transaction structure across
  layers: `utxo_spend` deriver joins vin outpoints (esplora/electrum
  `raw_json`) and Wasabi `spent_by` against owned `wallet_utxos` for exact
  same-wallet parent chaining and multi-wallet leg funding (Bitcoin and
  Liquid); `payment_hash` deriver links Lightning legs; both are
  deterministic bulk-review methods; one-pair-one-link guard prevents
  cross-method double allocation; `source-funds assemble` /
  `ui.source_funds.assemble` loop suggest + bulk review to convergence
  (local evidence only, no network); desktop review step is assemble-first
  with actionable gap cards dispatching `next_step.action`.
- [ ] Add graph visualization polish for dense source-funds cases after real-user feedback; keep the current editor workflow as the source of truth
- [ ] Add optional configured-backend chain observations with an explicit public-backend privacy warning; keep them weak suggestions unless reviewed
- [ ] Add optional OCR/photo/invoice extraction after the evidence review and
  audit package workflow has real-user feedback; keep suggestions review-gated
  and never auto-mark evidence complete.

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
  - [x] Add a redacted in-app daemon log screen with a downloadable JSON
    export so prerelease/dev desktop failures can be inspected without losing
    the terse notification surface
  - [x] Promote the desktop screen into a Developer tools-gated typed Logs view
    with subscription-level control, a bounded RAM-only local ring buffer,
    field-type redaction, copy-last-200, and Markdown/JSONL/log exports
  - [x] Add a support bundle export from Logs with High-signal and Public-safe
    redaction modes, an issue description, request-correlated redacted events,
    last-failure context, redaction report, and AI provenance for
    Codex-assisted debugging

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
  bug-report actions, books switcher, centered version label, and global
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
  approval before executing mutating tools such as source refresh.
- [x] In-app AI skill-aware read upgrade (PR 72): expand the compact Kassiber
  prompt, add `read_skill_reference("index")`, expose granular read-only daemon
  tools for wallets, backends, quarantine, transfers, rates, workspace health,
  next actions, and filtered transactions, and keep raw shell/filesystem/CLI
  access out of scope.
- [x] In-app AI reliability pass: auto-read exact local context before the
  provider for common accounting questions, add deterministic report summary /
  balance / portfolio / tax / history tools, add transaction extremes/search,
  auto-refresh stale local journals before AI read/report tools, expose sat/msat
  fields, add report-blocker / rate-coverage / change-audit / maintenance
  tools, expose answer provenance in GUI/export, and export tool args/results
  for debugging inaccurate small-model answers.
- [x] Wire the in-app AI swap-review surface end to end: advertised
  `ui.transfers.*` and `ui.saved_views.*` tools now execute through the same
  daemon dispatcher as the GUI, with writes still behind tool consent; the chat
  can also read the swap-matching skill reference and the deterministic
  `ui.transfers.review_context` packet for useful review guidance.
- [x] Dev browser bridge for real local AI: Vite keeps a loopback-only Python
  daemon supervisor, demuxes JSONL by `request_id`, and streams `ai.chat`
  records to browser clients as NDJSON so Codex/browser tools can test local
  AI, Stop, tool cards, and consent without launching the Tauri webview.
- [ ] Daemon worker pool: replace the surgical `ai.chat` thread with a real
  worker-pool model and one SQLite connection per worker when read-only tools
  or longer-running UI actions need daemon-side concurrency beyond the
  supervisor demux.
- [x] Desktop Assistant chat history: the GUI sends `persist: "auto"` and
  round-trips `session_id` on `ai.chat`, the Assistant toolbar has a History
  panel (list/resume/delete via `ui.chat.sessions.*`) plus an incognito
  toggle, and Settings → AI providers exposes the `ai_chat_history` policy
  (`ui.chat.history.configure`) with a clear-stored-chats action. The mock
  daemon mirrors the persistence semantics for browser dev.
- [ ] Chat-history retention: optional cap (keep last N sessions or days) on
  persisted `ai_chat_sessions`, enforced at append time, surfaced in
  `chats config` and desktop Settings.
- [x] Overview screen now uses `@shadcnblocks/dashboard5` as the first
  dashboard screen, keeping Export -> Reports, Add connection modal, and
  Show all transactions wiring
- [x] Transactions screen now uses `@shadcnblocks/dashboard2` as the
  transaction dashboard, with ordered period controls, enlarged search copy,
  and privacy visibility toggle in the header
- [x] Route transaction explorer modal opens through a validated desktop
  external URL opener so packaged builds use the system default browser without
  widening the file-opening boundary
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
  deleting encrypted local books set data
- [x] Books screen now uses daemon-backed book sets/books, create/rename/switch
  actions, and a book-set overview route for workspace-level treasury and
  readiness reads without switching the active book.
- [ ] Add a book-set treasury export (CSV/PDF) with BTC holdings/activity,
  per-book fiat rows, and a readiness manifest; keep capital-gains/tax exports
  book-scoped so lots, transfers, and mixed-fiat semantics never merge across
  books.
- [ ] Continue hardening book-management edges: destructive book/book-set
  deletion UX, backup/restore path, and remaining Settings fixture replacement.
- [x] Welcome/onboarding screen refreshed with a shadcn-style, SQLCipher-aware
  setup flow that captures books/tax defaults and database
  protection by initializing the local SQLCipher database through the daemon,
  lets users choose the default mempool/custom/skip backend setup with an
  explicit skip warning, keeps Austrian onboarding on moving-average-only
  current-rule defaults without book-level long-term exemption controls,
  captures optional AI assistant intent with a disable-for-now button, opens
  existing local Kassiber roots through the native desktop picker, and
  offers a dev-only mock preview shortcut.
- [x] Replace the Overview mock fixture with a read-only
  `ui.overview.snapshot` daemon kind backed by the current SQLite profile
- [x] Replace the Transactions table mock fixture with a read-only
  `ui.transactions.list` daemon kind and enable a loopback-only Vite dev
  bridge for browser testing against real local data
- [x] Wire the transaction detail editor to persist notes, tags,
  classification labels, and exclusion state through
  `ui.transactions.metadata.update`
- [x] Replace Connections, Journals, and capital-gains Reports mock fixtures
  with first real daemon-backed snapshots and a `ui.wallets.sync` action
- [x] Wire the desktop Quarantine and Tax Events review screens to real daemon
  data: Quarantine uses `ui.journals.quarantine`, Tax Events uses
  `ui.journals.events.list`, and Journals remains the processing/readiness
  surface linking both review paths.
- [x] Wire Reports export cards to daemon-backed managed exports for PDF,
  capital-gains CSV, and Austrian E 1kv XLSX, with default-app opening for
  completed files
- [x] Add a Developer tools-gated Logs screen that shows the recent structured
  daemon/transport stream from RAM only and exports redacted snapshots for
  local debugging on explicit download
- [ ] Replace remaining Settings mock fixture data with typed daemon calls once
  phase 1.1 exists

### 1.4 Live actions and workers

- [x] Add a daemon-owned freshness/job subsystem for live source refresh:
  persistent per-profile source states and jobs, separate on-chain / BTCPay
  wallet-source / BTCPay provenance / market-rate / journal job types,
  checkpointed Electrum/Esplora/BTCPay/rate work, cancellation, pause/resume,
  opt-in background worker, provider cooldowns, redacted envelopes, and Tauri
  allowlist support.
- [x] Follow-up hardening for freshness workers: move daemon freshness glue into
  a focused module, share Retry-After and persisted timestamp parsing helpers,
  scope the unlocked SQLCipher passphrase to the local daemon session plus
  one-shot worker handoff, and bound repeat BTCPay page scans with stable-id
  fingerprints, explicit stop reasons, and rotating deep audits for older
  metadata edits.
- [ ] Finish the remaining live-action worker surfaces: file/import flows,
  metadata edits, transfer pairing, attachments, quarantine resolve,
  profile/wallet/backend CRUD, backup/restore.
- [ ] Expand the dedicated progress + cancellation UI beyond sync/freshness
  helpers into every long-running live action.
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

- [x] Split `TransactionDetailSheet.tsx` tab bodies into siblings —
  the detail sheet now delegates display helpers, header chrome, the
  right rail, attachments/commercial panels, and each tab body to focused
  transaction-detail sibling components, leaving the main state/save
  coordinator under 800 lines without changing behavior.
- [x] Wire transaction-detail pricing edits through
  `ui.transactions.metadata.update` — the daemon accepts pricing source
  kind/quality, fiat currency, manual price/value, and evidence reference,
  persists them on the transaction pricing provenance columns, and
  invalidates journals after save.
- [x] Extend `ui.transactions.metadata.update` to accept tax handling and
  review-state fields — review status is durable UI state, taxable=false
  keeps a transaction out of journal inputs without hiding it from the
  transaction list, Austrian regime override feeds the AT marker handoff,
  and Austrian category override is persisted onto reviewed AT journal
  output.
- [x] Desktop attachments daemon kinds — `ui.attachments.list`,
  `ui.attachments.add`, `ui.attachments.remove`, `ui.attachments.open`
  to wire the `AttachmentsPanel` in the transaction detail sheet to
  the existing `kassiber/core/attachments.py` (which already handles
  file + URL, multi per tx, integrity verification, and orphan GC).
  The desktop sheet now lists real attachments, copies selected files,
  stores URL references, opens URL/file targets through the Tauri shell,
  and removes attachment records through daemon mutations.
- [x] Change provenance for transaction metadata edits — append-only
  `transaction_edit_events` / `transaction_edit_fields` rows capture notes,
  tags, exclusions, review/tax status, Austrian overrides, and pricing
  provenance/value changes from CLI, desktop, and AI-tool sources. No-op
  saves are suppressed; revert creates a new forward edit. Users can inspect
  per-transaction history, browse the global Activity route with filters,
  see stale-report prompts, and opt edit history into audit-package export
  without exposing descriptors, xpubs, backend credentials, wallet files, or
  unrelated wallet history.
- [ ] Custom CSV mapping DSL for arbitrary wallet exports
- [ ] Rates/manual adjustment surface
- [ ] Full double-entry account model only if a future ledger design needs it:
  explicit counterpart postings, account-type rollups, adjustments, and
  migrations; current `accounts` are wallet/reporting buckets
- [ ] Per-profile Tor proxy configuration. The Electrum client now
  speaks SOCKS5 against `backend.tor_proxy`, and onboarding/settings
  expose a proxy field for the `ui.backends.electrum.test` flow, but
  the proxy value still has to be wired through `kassiber backends
  create/update` and through the desktop save path so it actually
  reaches the column at rest.
- [ ] SOCKS5 username/password auth (RFC 1928 method 0x02) for
  Electrum proxies. Today `_connect_via_socks5` only offers no-auth
  and emits a precise error when a proxy refuses it, which covers Tor
  but not corporate SOCKS5 endpoints that require credentials.
- [x] Extend BTCPay Greenfield sync beyond confirmed wallet history with stable invoice/payment ids and raw payload snapshots
- [x] Import BTCPay invoice/payment fiat facts as authoritative pricing
  observations and reconcile them to wallet transactions before merchant
  receipts are treated as exact BTCPay-priced income
- [x] External document records for invoices, receipts, contracts, and related BTC-linked business evidence
- [x] Many-to-many document/payment links with allocations and reconciliation state
- [x] Deterministic matching rules before any AI assistance
- [x] Review/confirmation workflow for proposed matches and commercial annotations
- [x] Split commercial annotations from RP2-facing tax primitives during journal preparation
- [x] Accountant-facing export of matched BTC subledger rows with document references
- [ ] Opt-in local AI extraction and tie-breaking only after deterministic matching is solid
- [ ] Build the richer desktop visual reconciliation workflow on top of
  the new `ui.btcpay.provenance.*` and `ui.documents.*` daemon-safe
  surfaces. The transaction detail sheet now has a first-pass commercial
  provenance panel for BTCPay payment -> invoice -> payment-request/app-origin
  -> document context; the remaining work is the dedicated reconciliation
  queue/workbench for reviewing and resolving suggestions at scale.
- [ ] Richer transfer pairing for multi-leg self-transfers, including
  one-outbound/multiple-inbound same-txid moves that should not linger as
  swap-review noise
- [x] Better cross-asset transfer accounting beyond audit metadata
  (matcher + rules + saved views + `/swaps` review queue land swap
  pairing end-to-end; AT carrying-value continues through rp2; generic
  profiles still SELL + BUY pending upstream rp2 multi-asset carry).
- [x] Direct swap payout reviews for provider-settled external payments:
  source outflow, target payout amount, reviewed sale proceeds, swap fee,
  and Austrian carrying-value handoff are modeled without fake recipient
  wallets.
- [ ] Daemon kind for ``detect_repeating_patterns`` + "Create rule from
  this pattern?" prompt in the swap review UI (pattern-detector helper
  already exists in `kassiber/core/swap_rules.py`).
- [ ] Promote bitcoinrpc-synced wallets to opportunistic HTLC enrichment
  via a per-tx `getrawtransaction` fetch when payment_hash is missing.
- [ ] Revisit per-wallet basis attribution if a jurisdiction ever needs
  physical-lot answers
- [ ] Adopt a per-project storage layout: one SQLite DB per project,
  minimal global app state, and no active top-level wallet side tree
- [ ] Add scoped handoff export/import flows on top of the per-project layout:
  tax advisor reports stay report-only, audit packages are explicit
  one-book-or-selected-books packages, and technical wallet evidence remains a
  separate restricted approval path rather than a normal export checkbox
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
- [x] Extend the stdin/fd secret-input pattern to `ai_providers.api_key`:
  OpenAI-compatible remote providers now support `--api-key-stdin` /
  `--api-key-fd FD` and desktop Settings uses the daemon-side
  `ai.providers.set_api_key` rotate/re-enter flow. The legacy
  `--api-key <value>` argv form remains a warning-on-use shim for scripts.
  Claude/Codex CLI providers use local CLI auth and do not add Kassiber
  API-key storage.
- [x] Implement and document the desktop secret-management boundary model for
  the AI-key pilot: SQLCipher remains the DB/accounting at-rest perimeter,
  desktop native stores can hold AI provider API keys only behind the narrow
  daemon/supervisor bridge, backend tokens/descriptors/blinding keys stay
  SQLCipher-protected, and no runtime or OS-compromise protection claims are
  made. See `docs/plan/10-secret-management.md`.
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
  reveal gate. macOS desktop builds now have the first half for database
  unlock: first lock-screen passphrase entry can enroll Touch ID for the next
  unlock, and **Settings → Security → Set up Touch ID unlock** verifies the DB
  passphrase and stores it immediately in Keychain. Later reads require a
  native LocalAuthentication Touch ID prompt before the passphrase is returned.
  Passphrase changes update the Keychain copy or disable Touch ID if the
  native store rejects the update; forgetting the setting deletes Kassiber's
  saved copy.
  Remaining work: biometric reveal gates for descriptor/token recovery and
  equivalent remember-me affordances for Windows/Linux where the platform
  policy can support them. These remain convenience over the SQLCipher
  passphrase, never a cryptographic substitute.
- [x] Kassiber skill bundle for agents (`skills/kassiber`)
- [ ] Optional server/REST mode, still local-first and opt-in

## Open bugs and debt

- [x] Austrian E 1kv PDF export no longer uses the Latin-1 text writer:
  `reports export-austrian-e1kv-pdf` / `reports export-austrian` now render a
  ReportLab-backed Steuerbericht with cover, summary/detail sections,
  holdings, Besonderheiten, explanations, transaction appendix,
  FinanzOnline-style Kennzahl summary, and FAQ. The focused regression extracts the generated PDF with
  `pdftotext` when available and checks for `€ (EUR)` plus the new section
  names.
- [ ] Generic text PDF rendering is still Latin-1 only:
  `_ascii_text` in `kassiber/pdf_report.py` silently replaces every
  codepoint outside Latin-1 with `?` in exported PDFs — notably `€`,
  `₿`, arrows like `↔`, and any non-European script. German umlauts
  and `ß` are inside Latin-1 and survive. The current behavior remains pinned
  by `test_pdf_report_substitutes_non_latin1_glyphs` in
  `tests/test_review_regressions.py`. Source-of-funds PDF export and the
  desktop Summary PDF export now use ReportLab renderers. Follow-up: either
  migrate the remaining generic export to the ReportLab renderer or fail loudly
  with a structured `code: "pdf_unrepresentable"` error envelope listing the
  offending codepoints. Document the choice in
  [pdf_report.py](kassiber/pdf_report.py) and update the test pin.
- [ ] Bump `vitest` 3.2.4 -> 3.2.6 to clear the critical Dependabot alert
  (Vitest UI server arbitrary file read/exec, fixed in 3.2.6). Not exploitable
  as used here: the `test` script runs plain `vitest` with no `--ui`, so the
  vulnerable UI server never starts, and it is dev-only tooling. Deferred
  because 3.2.6 was published 2026-06-01 and the `ui-tauri/.npmrc`
  `minimum-release-age` (90 days) blocks it until ~2026-08-30; bumping sooner
  needs the explicit owner approval that policy requires. Revisit after the
  release-age window opens (or with owner approval), keep `pnpm-lock.yaml` in
  the same commit, and re-run the gate.
- [ ] Address the medium Dependabot alert on transitive `glib` 0.18.5
  (unsoundness in `VariantStrIter` `Iterator`/`DoubleEndedIterator` impls,
  fixed in 0.20.0). `glib` is not a direct dependency — the Tauri/GTK-rs stack
  (`atk-sys`, `gdk`, ...) pins it to `^0.18`, so reaching 0.20 requires
  upgrading that whole stack (Linux-only surface). Fold into the next
  Tauri/GTK dependency upgrade rather than forcing a standalone `cargo update`.
- [ ] Add live provider-backed FX adapters beyond CoinGecko and local Kraken
  CSV archive ingest only after the UI and daemon can expose provider limits,
  granularity, and review state honestly
- [ ] Keep the machine envelope boundary centralized and explicit
- [ ] Keep docs and examples Bitcoin-only
- [ ] Add a narrow docs-drift check for shared command / verification /
  safe-to-record surfaces so `README.md`, `AGENTS.md`, `SECURITY.md`, and
  `skills/kassiber/` do not quietly diverge
- [x] Run Vitest from `scripts/quality-gate.sh`; CI installs
  `ui-tauri/node_modules` with `pnpm --dir ui-tauri install --frozen-lockfile`
  before running the gate, and the gate fails clearly if dependencies are
  missing instead of silently installing them.
- [ ] Harden the dev Vite bridge: either default to read-only and require
  an explicit flag to allow mutating kinds, or require a per-launch token
  header. Today the bridge accepts every kind in
  `ALLOWED_BRIDGE_KINDS` from any same-origin loopback POST (see
  `ui-tauri/vite.config.ts` `readBridgeRequest`). Defense-in-depth only —
  dev-only loopback, not a production concern.
- [ ] Replace the hand-curated `supported_kinds` example in
  `docs/reference/daemon.md` with a snippet generated from the daemon's
  actual dispatch table, or drop the example and direct readers to the
  live `daemon.ready` payload. The PR added a "representative" disclaimer
  as an interim.
- [ ] Clean up legacy schema-version fixtures called out as a follow-up in
  PR #101. Original author should point at the specific fixtures meant.
- [ ] Derive `kassiber.__version__` from package metadata
  (`importlib.metadata.version("kassiber")`) so it stops being a fifth
  place the version drifts. Drift test already catches divergence; this
  removes the cause.
- [x] Rate-limit-safe sync refresh speedups (shipped): WAL-tuned SQLite
  pragmas (`synchronous=NORMAL`, `temp_store=MEMORY`, `cache_size`, plaintext
  `mmap_size`) in `kassiber/db.py`; a shared per-host HTTP concurrency semaphore
  plus bounded 429/503 backoff honoring (and clamping) `Retry-After` in
  `kassiber/http_client.py`, used by both `kassiber/core/sync_backends.py` and
  `kassiber/core/rates.py`; parallelized within-wallet esplora UTXO + Liquid
  raw-tx fetches; cross-wallet parallel network fetch (`prefetch_wallets_backend`
  + the `fetch_wallet_backend`/apply split, DB writes still serial under the
  per-wallet savepoint loop, progress ContextVar propagated to workers via
  `copy_context`); a `rate_limited` sync-progress phase surfaced through to the
  desktop progress UI so a backoff no longer looks like a hang; and an
  `executemany` UTXO inventory write.
- [ ] Sync/refresh performance follow-ups still open:
  - Parallelize the daemon's background refresh itself: `run_due_jobs`
    (`kassiber/core/freshness.py`) syncs one wallet per serial job, so it does
    not benefit from `core_sync.sync_wallets` cross-wallet parallelism. Speeding
    up background refresh means running due jobs concurrently with one DB
    connection per worker plus single-flight handling — a separate, larger
    change.
  - Batch `auto_price_transactions_from_rates_cache` (currently one
    `get_cached_rate_at_or_before` SELECT + one UPDATE per unpriced transaction,
    run at the start of every `journals process`) into a set-based join +
    `executemany`. Different subsystem (journal pricing), kept separate.
  - Add a `throttled` `ConnectionHealthStatus`
    (`ui-tauri/src/lib/connectionHealth.ts`) so a rate-limiting-but-alive backend
    is not shown as healthy/green, and a live `rate_limited_until` countdown in
    the freshness/sync-results UI.

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
