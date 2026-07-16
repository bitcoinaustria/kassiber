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

Use `./scripts/quality-gate.sh` before calling work ready to push. It compiles
Python, validates the disjoint CI shard manifest, runs every Python module once
through pytest, exercises in-process help plus bounded real CLI subprocesses,
and runs the TypeScript, ESLint, and Vitest checks so humans and agents use the
same verification surface.

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

- [x] **Replace manual Bitcoin/Liquid chain state with dependency-backed
  observers** in one phase-checkpointed PR. Current truth and the initial
  capability matrix live in
  [`docs/reference/chain-observers.md`](docs/reference/chain-observers.md).
  Completion means supported BDK/LWK routes own chain state without a
  production shadow observer, all derived state remains inside SQLCipher, and
  one refresh atomically commits observer state + accounting projections.
  - [x] Phase 0 — baseline and inventory: all existing quality/fast/Core/
    Electrum/demo gates pass; the reference document names concrete manual
    observer functions, ingress paths, commit boundaries, packaging references,
    tests and honest initial compatibility routes.
  - [x] Phase 1 — cross-platform packaging: retain universal macOS desktop and
    Intel CLI/sidecar builds, collect BDK on both Mac architectures, and keep
    the named Liquid compatibility observer outside macOS arm64, Linux x86-64,
    and Windows AMD64 because LWK 0.18.0 publishes wheels only for those
    platforms.
  - [x] Phase 2 — watch-only boundary: introduce one shared descriptor
    capability/preflight layer for CLI, daemon, files, BSMS, bare xpub,
    Samourai and compatibility importers; always reject spending-private
    Bitcoin/Liquid material while permitting sensitive Liquid view keys.
  - [x] Phase 3 — atomic refresh substrate: finish chain fetch before the write
    savepoint; apply observer state, transactions, retractions, graph evidence,
    inventory, coverage and freshness under one coordinator-owned rollback
    boundary with no sub-hook or progress-callback commit.
  - [x] Phase 4 — observer contract and SQLCipher store: add deterministic
    multi-instance identities, explicit versioned state/coverage persistence,
    fetch/apply contracts, rollback discard and strict exclusion from public,
    AI, diagnostics, audit and replication surfaces.
  - [x] Phase 5 — Docker/regtest observer oracle: extend the existing disposable
    Core/Elements stack with independent Bitcoin and Liquid truth manifests for
    confirmation, RBF, reorg, ownership, UTXO and restart transitions.
  - [x] Phase 6 — Bitcoin BDK observer: pin/package the official dependency and
    route supported Esplora/Electrum descriptor wallets through it with
    deterministic normalization and no production shadow/fallback observer.
  - [x] Phase 7 — Liquid LWK observer: pin/package the official dependency,
    route executable-proven Liquid descriptor forms through it, and persist
    wollet state only inside SQLCipher while preserving local unblinding.
  - [x] Phase 8 — cleanup and final proof: delete replaced manual observer
    engines, retain only named compatibility routes and `embit` primitives,
    update all user/developer/release/license docs, and leave the same draft PR
    green after full unit, integration, demo, package and secret-scan gates.
  - [x] Follow-up — reduce compatibility by capability, without weakening
    accounting ownership: pass supported Liquid Esplora auth and Electrum TLS
    policy directly to LWK; normalize `mempool`; fail closed when Esplora
    custom trust cannot be represented; canonicalize only structurally
    equivalent Liquid receive/change descriptors; exercise dependency clients
    over every supported transport; retain partial-descriptor compatibility
    until mixed-input fees, filtered retractions and branch coverage have a
    formally tested accounting-ownership model; and describe Core RPC,
    Bitcoin address scripts and Silent Payments as first-class specialized
    observers rather than generic dependency fallbacks. The direct TLS probe
    additionally proved pinned LWK 0.18.0's insecure Rustls verifier unusable;
    keep that named compatibility route until a packaged LWK release contains
    the upstream signature-scheme fix and passes the local oracle.
- [x] **Custody lineage and missing-wallet reconciliation.** Implement the
  bounded architecture in
  [`docs/plan/14-custody-lineage.md`](docs/plan/14-custody-lineage.md). One
  profile is one legal owner; there is no "all wallets imported" attestation.
  The terminal invariant is: every observed quantity is represented exactly
  once, while unresolved custody never becomes a taxable event.
  - [x] Publish the architecture, vocabulary, scope guards, executable gates,
    flagship OG-treasury fixture, and terminal stop state.
  - [x] **Gate 0 — honest pre-split behavior.** Remove row-deletion suspense;
    retain the conservative disposal plus hard quarantine until the quantity
    split exists; prove exact source debit, fee separation, wallet holdings,
    and that the spent source quantity cannot fund a later disposal.
  - [x] **Gate 1 — evidence and projection boundary.** Add stable quantity
    identities, immutable evidence snapshots for authored claims, and canonical
    physical-event identity; define the single custody
    claim arbitrator; split quantity projection from finalized tax input; add
    custody suspense. Complete when a full-engine regression proves candidate,
    suspense, conflict, and failed-component quantities cannot reach RP2 while
    a known source-wallet debit still affects observed quantity. Run the first
    architecture review here before continuing.
  - [x] **Gate 2 — interpretation parity.** Port exact same-txid moves,
    policy/output ownership, fan-out/consolidation, manual pairs, direct payouts,
    swaps/refunds, Lightning lifecycle, and active custody components into the
    one arbitrator. Add deterministic, bounded, full-history 1:N/N:1/N:M custody
    candidates and durable reviewed bridges. Complete when known-correct
    differential fixtures match, the complete-policy and missing-Whirlpool
    flagship cases pass, and no later phase can restore or fallback-book an
    already decided quantity. Add an explicit activatable `suspense` component
    sink and keep `unresolved` non-activatable. Because basis is global across
    wallets, later tax output stays provisional when resolving an earlier
    suspense slice can change lot selection.
  - [x] **Gate 3 — product completion.** Add the Custody gaps queue, lineage
    timeline, guided bridge/residual workflow, downstream and filed-report
    impact preview, CLI/daemon kinds, localized desktop allowlists, AI-safe read
    and consented write tools, privacy receipts, migration, replication, and
    audit history. Complete when the workflow works without raw component JSON,
    old authored history survives migration, and deterministic operation remains
    fully usable without AI.
  - [x] **Gate 4 — verification and stop.** Run differential, property,
    migration, performance, fast replay, Bitcoin Core, Electrum, Silent
    Payments, Liquid/Boltz, CLN/LND, desktop, and repository quality gates.
    Delete superseded ownership/matching precedence, withholding/restoration,
    and fallback-disposal paths. Run a simplicity pass plus an independent final
    architecture/security/privacy and merge-readiness review. Complete only when
    every terminal-stop condition in the plan is met and no issue-scoped P0/P1
    remains; move unrelated P2+ findings to separate TODO items and stop.

- [x] Scale custody-gap discovery beyond the former 50k-input and 87-source
  all-or-nothing ceilings. Large books now use a bounded typed/high-value source
  worklist and amount-indexed return ranges, keep incomplete sampled candidates
  review-only, quarantine only exact typed source boundaries, and expose stable
  version-bound pages without rerunning discovery. Long-lived-book regressions
  and the 100k-1m benchmark prove useful candidates still surface while
  incomplete searches remain explicit.

- [ ] **Simplify custody architecture after PR #439.** Follow the ordered,
  invariant-preserving cutover in
  [`docs/plan/15-custody-simplification.md`](docs/plan/15-custody-simplification.md).
  - [x] Extract the production custody journal composition from CLI handlers,
    expose the decision boundary separately from RP2, convert production gap
    capacity state to an ordinary typed result, and add an unpatched builder
    characterization plus a real 100k benchmark baseline.
  - [x] Centralize boundary-leg principal/fee/wallet-movement normalization and
    deterministic FIFO N:M allocation; make gap claims and reviewed bridge
    plans consume the same exact-msat cells and residuals.
  - [x] Replace heuristic transfer claims with independently scoped
    source/return holds; delete `CUSTODY_CANDIDATE` / `HEURISTIC_CANDIDATE`
    arbitration so suggestions cannot contain a basis-carrying target edge.
  - [x] Persist one normalized, versioned candidate projection with explicit
    completeness metadata and indexed keyset pagination; make journal, UI and
    AI reuse it and clear the obsolete serialized page cache after replacement.
  - [x] Add one read-only `plan_review` and one fingerprint/version-checked
    `apply_review` seam for bridge creation, revision, reopening and residual
    classification. Plans contain deterministic exact component rows and
    report impacts; compatibility previews expose only redacted summaries and
    perform zero SQLite writes.
  - [ ] Migrate pair/payout authored meaning into components with typed
    replicated economic terms; freeze legacy writes and retain replay history.
    Deterministic draft staging, one-to-many leg-bound terms, compatibility
    links, full-source connected 1:N/N:M consolidation, activation,
    idempotence and atomic rollback coverage are complete. The journal consumes
    effective active components and falls back to legacy interpretation for
    skipped, ineffective, or partial-source historical rows; component-native
    residual decisions and writes remain. Linked active compatibility rows are
    database-write-frozen, and current mutation handlers retire/revise their
    component atomically through the core authored-review store. CLI handlers
    no longer own pair/payout INSERT, UPDATE or tombstone SQL. Removing the
    frozen compatibility projections from new writes is the final producer
    cutover after all readers move to component terms. The journal builder no
    longer queries either legacy table directly: the core authored store now
    exposes only linked rows whose component is ineffective, keeping the
    partial-source residual exception explicit and deletable.
  - [ ] Cut every consumer to stored decisions/lineage, require gated report
    contexts, delete compatibility interpretation and speculative scaffolding,
    and demonstrate the final simplicity/LOC/performance stop state. The
    transaction graph, report/export transfer and swap rows, source-of-funds
    lineage, transaction UI, journal UI and AI snapshots now consume only
    stored custody decisions/economic relations. Reviewed kind, policy,
    authorship, notes, swap fee and direct-payout metadata are losslessly
    projected for those readers. Stale books expose an explicit projection
    state and do not render old custody grouping as current booked truth. Every
    journal-derived report/export now acquires one core `ReportContext` proving
    tax support, current journals, complete active components and clear
    quantity barriers; composed reports reuse that proof, and the CLI report
    hook/back-edge has been deleted. Builder compatibility deletion, producer
    cutover and speculative-scaffolding removal remain.

- [x] Harden the pre-msat legacy schema migration so rebuilding a very old
  database preserves columns added after that historical table shape. Add an
  ancient-schema fixture and assert column/data parity after migration. This
  predates the custody-lineage branch and is intentionally outside its stop
  state.

- [x] Harden the CLI for one-shot agents: `--machine` now implies
  `--non-interactive`; `commands describe` exposes an argparse-derived command
  contract; `health` / `next-actions` expose shared readiness snapshots;
  paginated envelopes include uniform `page` metadata; high-impact automatic
  pairing/review paths have rollback-backed `--dry-run`; secrets/backup use one
  envelope owner; managed settings updates are locked + atomic; remembered chat
  unlock uses a private cross-platform daemon message and only native OS
  credential backends are trusted.

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
- [x] Ship "your own node, end to end" for Bitcoin Core RPC:
  descriptor/xpub-native sync imports ranged watch-only descriptors inside the
  bitcoinrpc adapter (discovery stays read-only), persists per-branch imported
  range ends plus observed `highest_used`, supports wallet `birthday` dates for
  bounded rescans, adds desktop local-Core detection and health probes
  (`ui.backends.detect_core`, `ui.backends.bitcoinrpc.test`) with
  `bitcoin.conf` parsing plus wallet-RPC / BIP158 capability reporting, and
  wires the bilingual Add Connection flow for Core setup.
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
- [x] Add address/txid **ownership reconciliation** (`wallets identify` +
  `ui.wallets.identify` + desktop **Reconcile** screen + AI read tool): paste
  addresses/txids (Bitcoin or Liquid, mixed) and flag which belong to a wallet
  (receive or change, naming wallet + derivation index) vs external, classifying
  each transaction as a self-transfer / outbound payment / inbound receipt. Pure
  engine in [`kassiber/core/ownership.py`](kassiber/core/ownership.py) matches on
  canonical scriptPubKey (address-string fallback for Liquid confidential
  addresses), seeds from output inventory, exact stored receive outpoints,
  local transaction graphs, and address lists, derives active/retired policies
  offline up to `--scan-to-index`, and accepts an injected on-chain fetcher
  (`--verify-on-chain`) so read surfaces stay cache-only. AI variant drops
  scriptPubKeys / derivation paths / address indices.
- [x] **Localize the desktop UI (i18n groundwork)** — added i18next +
  react-i18next under [`ui-tauri/src/i18n/`](ui-tauri/src/i18n/): English/German
  resource bundles, type-safe keys, a store-driven language bridge (the UI store
  `lang` is the single source of truth → i18next + `<html lang>`), a working
  Settings/header language switcher, an en/de key-parity test, and a vitest i18n
  setup. First pilot surfaces (this groundwork milestone): Settings appearance
  panel + `BirdsEye` navigation labels; full surface transcreation is the next
  item. CLI/daemon stay machine-deterministic. Conventions in
  [docs/reference/i18n.md](docs/reference/i18n.md).
- [x] **Transcreate the desktop UI into Austrian German (du register)** — main
  surfaces translated under per-surface namespaces (chrome, overview,
  transactions, connections, journals, settings, onboarding, assistant,
  sourceFunds, review, search) using the researched, BMF-sourced
  [glossary](docs/reference/i18n-glossary.md): Bitcoin jargon kept English,
  ordinary words translated, Austrian tax terms (Anschaffungskosten,
  Veräußerung, gleitender Durchschnittspreis, Wegzugsbesteuerung, Kennzahl,
  Beilage E 1kv, KESt…), Austrian month/“heuer”. Verified via typecheck, the
  en/de parity test, build, and live browser screenshots.
- [x] **Bitcoin-backed loan marks (per-transaction non-events)** — mark BTC
  collateral posted/returned for fiat loans and BTC principal received/repaid for
  BTC-denominated loans. Collateral locks/releases preserve owned basis;
  borrowed BTC principal is liability principal, not owned-coin
  acquisition/disposal. The mark suppresses the relevant branches at
  [`tax_events.py`](kassiber/core/tax_events.py) / [`rp2.py`](kassiber/core/engines/rp2.py);
  storage is one minimal `loan_legs` row (`transaction_id` + role + optional
  `loan_id`). Deliberately **not** a facility: no custody / rehypothecation /
  interest / liquidation modelling. Liquidation is handled by **un-marking**
  (the outbound reverts to the disposal it really was); `open_collateral_locks`
  surfaces locks that haven't returned as a reconcile hint. UI is a Transactions
  row action + badge/detail linked-leg section (no `/loans` screen); CLI
  `loans mark|link|unmark|list`.
  - [x] Resilience precursor: a carrying-value swap whose leg was blocked in
    phase 1 (e.g. `insufficient_lots` on a self-custody round-trip paired as a BTC↔L-BTC
    swap) is quarantined as a pair in `_select_at_cross_asset_swap_links` instead of
    being promoted to an `at_swap_link` that bypassed the quantity gate and aborted the
    whole report. Regression: `ATSwapOverSellQuarantineTest`; contract in
    [docs/austrian-handoff.md](docs/austrian-handoff.md).
  - The facility design (provider presets, custody/rehypothecation matrix, import
    on-ramps, Steuerberater export) was explored in
    [docs/plan/12-collateralized-loans.md](docs/plan/12-collateralized-loans.md) and
    deliberately dropped as over-built — the per-tx mark covers the actual tax fact.
- [ ] **Finish the i18n long tail:**
  - Reporting surfaces deferred by product call: `routes/Reports.tsx`,
    `routes/ExitTax.tsx`, `LightningProfitabilityPanel.tsx`, and report-output
    strings.
  - Shared `lib/` enum→label maps consumed across surfaces
    (`lib/connectionDisplay.ts`, `lib/syncProgress.ts`,
    `lib/connectionCatalog.tsx`, `components/kb/journalReportableEntriesModel.ts`):
    thread the active `t`/`TFunction` so the conversion stays consistent across
    every consumer.
  - Locale-driven formatting: migrate hardcoded `en-US` `toLocaleString`/`Intl`
    call sites (chart axes, dates, percentages) to `localeForLanguage(lang)`
    with AT decimal-comma / space-before-`%` / `Jänner` conventions. This is
    number/date formatting, not translatable strings.
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

- [x] Add [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md) follow-through in code and schema (Implementation Order steps 1-6 shipped: 4-table schema, BTCPay provenance ingest, deterministic matching/allocation, review/confirm workflow, conservative tax normalization, CSV subledger export — daemon/CLI-backed with tests). The remaining tail is tracked separately: optional local AI extraction/tie-breaking (step 7, later-backlog item below) and the richer desktop reconciliation workbench
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
- [x] Add optional local OCR/photo/PDF transaction extraction as a review-gated
  draft importer: `wallets preview-document`, `wallets import-document`, and
  desktop `ui.wallets.document_import.{preview,import}` require loopback local
  vision/OCR models, quarantine ambiguous rows, bind reviewed drafts to the
  source-file SHA-256, and attach the source document as managed evidence.
  Source-funds-specific extraction polish remains future
  feedback-driven work.

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
- [x] Local log inspection & export (RAM-only by design). The original
  "`~/.kassiber/logs/` with rotation + `diagnostics collect` folds all logs"
  plan is **retired**: an always-on on-disk log file is an explicitly rejected
  design (see [docs/reference/logging.md](docs/reference/logging.md)) and
  `collect_public_diagnostics` deliberately folds no logs. The log-export need
  was met instead by the RAM-only Logs view + redacted support bundles below.
  (Follow-up: the stale on-disk-logs section in
  [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md) still contradicts
  logging.md and should be reconciled.)
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
- [x] `progress` envelopes for sync/freshness kinds beyond the AI chat cancel
  path (`ui.wallets.sync.progress`, `ui.freshness.run.progress`,
  `ui.workspace.freshness.run.progress`, background `ui.freshness.progress`),
  with supervisor streaming/non-streaming classification + non-fatal request
  timeout
- [ ] Generic mutation-safe cancellation + long-running handling for the
  non-sync mutating kinds: the generic `cancel` kind still returns "daemon
  cancellation is not wired yet" (`daemon.py`) and the serial main loop has no
  worker pool (depends on the worker-pool item below; overlaps the live-action
  items in §1.4)
- [ ] Worker pool with one SQLite connection per worker
- [x] Smoke coverage for daemon ready/status/shutdown
- [ ] Redaction audit in CI: redaction is already partially exercised in the
  gate (`test_secrets_smoke`, `test_freshness`, and the Vitest
  `appLogs`/`bridgeContainment` tests), but the dedicated Python redaction suite
  `tests/test_log_ring.py` is not wired into `scripts/quality-gate.sh` and there
  is no systematic secret-leak scan — add both

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
  Kassiber skill guidance, add tool cards, and start with a bounded
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
- [x] In-app AI review/operator expansion: capability-scope the advertised
  schemas for smaller local models; pass typed ephemeral screen context; add a
  composite transaction review packet; expose consent-gated metadata/evidence,
  source-funds assembly/export, commercial reconciliation, and report handoff
  tools; and attach an outbound/privacy receipt to every answer. OCR files and
  raw document bytes remain desktop-local and outside the chat tool surface.
- [x] In-app AI second review pass: add a deterministic cross-workflow
  worklist, explicit book-set overview, loan accounting review, direct payout
  maintenance, managed-evidence linking, and consent-gated latest rates; build
  typed route context in the renderer and invalidate affected UI caches after
  successful AI writes. Freeze every read/write/history target to the original
  project/book, enforce advertised schemas at execution, and close hidden
  public-lookup and OCR proxy/source-race paths.
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
- [ ] Generate the daemon kind allowlists from a single contract source instead
  of the three hand-maintained lists kept in sync only by
  `tests/test_connection_catalog_drift.py` (Python `SUPPORTED_KINDS`, Rust
  `ALLOWED_DAEMON_KINDS`, Vite `ALLOWED_BRIDGE_KINDS`). Note: the `lib.rs` error
  text already says "the generated daemon allowlist" although nothing generates
  it yet
- [ ] Pydantic v2 contracts to JSON Schema to TS types in CI; schema-drift
  fails the build
- [ ] Bridge containment tests — partial + spec drift. What shipped is a Vite
  dev-server HTTP middleware (`ui-tauri/vite.config.ts`), **not** the
  `daemon --bridge ws://` token-authed WebSocket assumed by
  [04-desktop-ui.md](docs/plan/04-desktop-ui.md) §2.6/2.7.
  `ui-tauri/src/lib/bridgeContainment.test.ts` covers loopback-host +
  cross-origin/no-Origin rejection and stderr redaction (gate-wired via
  `vitest`). Still open: (a) reconcile the §2.6 token-WS spec with the shipped
  Vite-proxy model; (b) missing/wrong-token + non-loopback-bind + production-env
  (`KASSIBER_ENV`) startup refusal + zero-token log grep — none exist today;
  (c) reconcile "mutation-disabled-by-default" — `ALLOWED_BRIDGE_KINDS`
  currently permits mutations with no read-only/`--allow-mutations` gate.

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
- [ ] Continue hardening book-management edges. Done: destructive **book-set**
  (workspace) deletion UX (`ui.workspace.delete` with passphrase + label +
  plaintext-ack), and Settings fixture replacement (folded into the line below).
  Remaining: (a) destructive single-**book** deletion UX + a `ui.profiles.delete`
  daemon kind (today only `ui.profiles.reset_data` exists, which clears data but
  keeps the book); (b) a GUI backup/restore action behind `ui.backup.*` daemon
  kinds (currently CLI-only command hints in the Data settings panel).
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
- [x] Replace remaining Settings mock fixture data with typed daemon calls
  (phase 1.1 daemon mode exists; SettingsScreen + panels read backends /
  maintenance / AI-provider / status through real daemon kinds). Residual
  cleanup only: delete the now-orphaned `DEFAULT_BACKENDS`/`DEFAULT_RATE_BACKENDS`
  exports in `SettingsModel.ts` and the stale "controls are local UI state"
  header comment in `SettingsScreen.tsx`

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
- [ ] Finish the remaining live-action worker surfaces. Now wired (daemon kind +
  UI mutation): file/import flows, metadata edits, transfer pairing, attachments,
  quarantine resolve (via the per-transaction metadata editor's price-override +
  exclude), and profile/wallet/backend CRUD. Genuinely unbuilt as a UI/daemon
  surface: **backup/restore** (CLI-only via `kassiber/backup/cli.py`; Settings
  explicitly defers to the terminal).
- [ ] Expand the dedicated progress + cancellation UI beyond sync/freshness
  helpers into every long-running live action.
- [x] Separate secret-entry IPC channel (daemon-only
  `supervisor.ai_secret_store.request/.response` control bridge, not exposed to
  the webview/assistant) and OS-keychain-backed AI-provider secret refs
  (`NativeSecretStore`: macOS Keychain / Windows user-scope DPAPI / Linux Secret
  Service) with per-platform policy, `ai_provider_secret_refs` state,
  `ai.providers.set_api_key` as the sole api-key ingress, use-time
  `secret_ref_unavailable`, move/repair/restore handling, a `MockSecretStore`
  for CI, and leak-regression tests (PR #116). Backend tokens / descriptors /
  passphrases stay SQLCipher-only by design

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
- [x] Per-OS Tauri bundles produced as **unsigned** previews in CI (macOS
  dmg/app, Linux AppImage, Windows msi/nsis via `pnpm tauri build --bundles`)
- [ ] Production code-signing & distribution: Apple Developer ID + notarization
  (macOS), Windows EV cert, GPG-signed `.deb` (Linux target is currently
  AppImage, not `.deb`); flip `tauri.conf.json` `bundle.active` for production
- [ ] User-initiated update check only; no background polling

## Later backlog

- [ ] **CLI `--detect-script-types` probe for bare-xpub wallets.** The desktop
  add-wallet flow auto-detects which script types an xpub has on-chain history
  for (daemon `ui.wallets.detect_script_types`, core
  `sync_backends.detect_active_script_types`). The CLI can already create a
  multi-script xpub wallet by *pinning* types (`wallet create --kind xpub
  --descriptor xpub… --script-type p2wpkh --script-type p2tr`), but has no
  auto-detect flag. Add `--detect-script-types` to `wallet create` that resolves
  the chosen/default backend and calls the shared `detect_active_script_types`
  helper to fill `config["script_types"]` (fallback to `p2wpkh` when none/no
  backend), for full GUI/CLI parity.
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
- [x] Cross-device / multi-user sync (no trusted server) — inspired by
  https://github.com/bitcoinaustria/kassiber/issues/309 but not a literal
  blueprint: the design in
  [docs/plan/13-device-sync.md](docs/plan/13-device-sync.md) is
  mailbox-first (sealed bundles over any dumb user-owned storage, ciphertext
  only, async, no listener) with LAN direct as fast path and Tor as optional
  leg, and pulls org multi-user (person identities, roles, invitations) into
  scope. S1-S5 now ship signed person/device identities, HLC + version-vector
  replay, sealed courier and mailbox bundles, folder/WebDAV/S3 transports,
  owner-attested join snapshots, desktop Settings/Sync and conflict review,
  explicit SPAKE2 LAN pairing with rotating mDNS names, an optional
  user-managed Tor onion leg, and quorum-gated tombstone compaction. The live
  database file, derived journals/reports, backend/AI secrets, raw fingerprints,
  and private wallet material never enter the replication layer. Issue #309 is
  the durable product record; operational details live in
  [docs/reference/device-sync.md](docs/reference/device-sync.md).
- [ ] Custom CSV mapping DSL for arbitrary wallet exports
- [ ] Rates/manual adjustment surface
- [ ] Full double-entry account model only if a future ledger design needs it:
  explicit counterpart postings, account-type rollups, adjustments, and
  migrations; current `accounts` are wallet/reporting buckets
- [x] Per-profile Tor proxy configuration. The Electrum client speaks SOCKS5
  against `backend.tor_proxy`, Esplora / Explorer-API HTTP reads, BTCPay
  Greenfield sync, Bitcoin Core RPC, and mempool-rate fetches honor the same
  backend proxy, and the proxy value is now wired end-to-end:
  `kassiber backends create/update --tor-proxy` → `core.accounts` →
  `backends.py` INSERT/UPDATE of the `tor_proxy` column, and the desktop save
  path serializes `payload.tor_proxy` (or clears it) through
  `ui.backends.create/update` to the same write. Proxy routing is intentionally
  per-backend; partial routing is supported and called out in UI/docs. Desktop
  setup detects `.onion` backend hosts and prefills the standard local Tor SOCKS
  proxy for that backend only.
- [ ] Guided Tor setup / managed Tor helper is tracked in
  https://github.com/bitcoinaustria/kassiber/issues/311. Keep it explicit and
  opt-in: no silent Tor install/start, no global routing, and no clearnet
  fallback for `.onion` endpoints.
- [x] SOCKS5 username/password auth (RFC 1929 subnegotiation via SOCKS5
  method `0x02`) for backend proxies. Proxy URLs may include credentials as
  `socks5h://USER:PASS@HOST:PORT`; credentials are redacted from backend
  output snapshots and preserved by desktop edits when already configured.
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
- [x] Opt-in local AI extraction for long-tail documents now exists as a
  hard-local OCR draft importer; AI tie-breaking for source-funds/commercial
  matching remains deferred until there is real-user feedback.
- [ ] Build the richer desktop visual reconciliation workflow on top of
  the new `ui.btcpay.provenance.*` and `ui.documents.*` daemon-safe
  surfaces. The transaction detail sheet now has a first-pass commercial
  provenance panel for BTCPay payment -> invoice -> payment-request/app-origin
  -> document context; the remaining work is the dedicated reconciliation
  queue/workbench for reviewing and resolving suggestions at scale.
- [x] Richer transfer pairing for multi-leg self-transfers. The tax pipeline
  decomposes graph-backed and recorded 1→N fan-outs; deterministic swap
  suppression now covers conserving 1-out/N-in same-txid groups; and current
  `owned_fanout_unresolved` / pairable `ownership_transfer_*` quarantines emit
  confidence-preserving `ownership_graph` cards through the pair store: exact
  requires whole-row canonical coverage; amount-compatible cards stay strong.
  Ownership cards are deliberately manual-only (excluded from rules and every
  bulk path), while conflict clustering supports choosing each real receipt.
- [x] Better cross-asset transfer accounting beyond audit metadata
  (matcher + rules + saved views + `/swaps` review queue land swap
  pairing end-to-end; BTC/LBTC rail changes can carry Bitcoin basis on every
  profile, AT carrying-value for other reviewed crypto swaps continues through
  rp2, and unsupported unlike-asset generic swaps stay SELL + BUY).
- [x] Direct swap payout reviews for provider-settled external payments:
  source outflow, target payout amount, reviewed sale proceeds, swap fee,
  and Austrian carrying-value handoff are modeled without fake recipient
  wallets.
- [x] Failed-swap refund handling: same-wallet same-asset pairs are
  allowed (carrying-value, no longer rejected), the HTLC parser decodes
  the refund/timeout branch, esplora/electrum/Liquid sync links a refund
  to its lockup via `transactions.swap_refund_funding_txid` (also accepted
  as a generic import-record/CSV field), and the matcher surfaces it as an
  `swap-refund` candidate (method `htlc_refund`, same-wallet and
  window-independent). Exact requires one witness-proven canonical funding
  input plus whole-row amount coverage; txid/outpoint metadata stays strong.
  Surfacing only — no silent auto-pair; the round
  trip books only the fee, not a SELL + BUY.
- [x] Widen the failed-swap refund link beyond freshly chain-synced Boltz
  v1 P2WSH refunds: matcher replay now recovers a unique funding outpoint from
  stored Esplora/Electrum/Core-style `raw_json` witnesses for rows synced before
  the dedicated columns existed. Ambiguous batch refunds decline instead of
  choosing one. Fresh sync still persists the columns. Boltz v2 Taproot cooperative/key-path
  refunds reveal no witness, so chain-only rows stay heuristic/manual by
  physics; redacted provider/SDK metadata with a swap id, canonical route txids,
  and explicit whole-row principal amounts is the exact path for those
  cooperative spends. Route-only metadata stays strong/manual.
- [ ] Add native Boltz SDK/client import or regtest bridge for cooperative v2
  swaps: persist only redacted provider id, flow, route txids, explicit principal
  amounts, status/version, and Taproot/cooperative spend hints so chain/reverse/refund swaps can be
  audited without treating chain-only key-path spends as exact evidence.
  Shipped in the Boltz regtest lane: optional `KASSIBER_BOLTZ_V2_EVIDENCE`
  / `--v2-evidence` ingestion for real wallet/client/provider evidence, with
  placeholder-looking ids rejected and exact whole-row `provider_swap_id`
  pairing asserted in a temporary Kassiber book. If those facts are missing, Kassiber
  should stay on heuristic/manual swap suggestions. Still open: drive the
  cooperative signing paths directly through Boltz's official client/SDK inside
  the harness.
- [ ] Daemon kind for ``detect_repeating_patterns`` + "Create rule from
  this pattern?" prompt in the swap review UI (pattern-detector helper
  already exists in `kassiber/core/swap_rules.py`).
- [ ] Promote bitcoinrpc-synced wallets to opportunistic HTLC enrichment
  via a per-tx `getrawtransaction` fetch when payment_hash is missing. Keep the
  script-path parser first-class for chain-sync-only/watch-only books: when no
  provider CSV/SDK metadata exists, a v1/uncooperative HTLC witness may be the
  only automated exact swap/refund signal available.
- [ ] Revisit per-wallet basis attribution if a jurisdiction ever needs
  physical-lot answers
- [x] Adopt a per-project storage layout: one SQLite DB per project,
  minimal global app state, and no active top-level wallet side tree
- [ ] Add scoped handoff export/import flows on top of the per-project layout.
  Shipped: the book-scoped audit-package **export** and the
  tax-advisor/technical-evidence taxonomy (`ui-tauri/src/lib/handoffExports.ts`).
  Remaining: (a) the **import** side (none exists); (b) extend audit-package
  scope from single-book to explicit selected-books packaging; (c) make the
  restricted technical-wallet-evidence path actionable (today a display-only
  card with no daemon kind)
- [x] When the per-project storage layout lands, migrate attachment links
  **and** the managed-copy blobs into each project bundle rather than the
  global state-root tree. New project roots use project-local `attachments/`,
  backups include those files, and legacy migration copies the old managed tree
  into `projects/default/attachments/`, then moves the old active plaintext
  artifacts into a timestamped `pre-project-migration-*` rollback directory.
  Multi-workspace legacy DBs migrate as one project container until an explicit
  split/import workflow exists.
- [x] Keep backend definitions and default-backend selection canonical in
  SQLite; dotenv files now bootstrap older/new stores instead of serving as
  the long-term storage path
- [x] Keep normal backend and wallet success output safe-to-record for
  secret-bearing config values by redacting raw credentials and raw descriptor
  material while preserving presence / state flags
- [x] Finish the project-local part of backend storage once the per-project
  DB layout lands: canonical backend rows live in the selected project DB and
  the plaintext bootstrap dotenv resolves under that project's `config/`.
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
  passphrase, with `kassiber secrets {init,change-passphrase,remember-unlock,forget-unlock,verify,status,migrate-credentials}`,
  `kassiber backup {export,import}`, `--db-passphrase-fd` plumbing through the
  CLI and daemon, and a `tar | age` single-file backup format.
- [x] Introduce first-class project/book-set containers with per-project
  SQLCipher boundaries: the default runtime resolves to
  `~/.kassiber/projects/<project>/data/kassiber.sqlite3`, the global
  `projects.json` catalog stores only non-secret routing metadata, CLI/daemon
  project create/list/select flows close the active DB before switching, and
  backups are scoped to the selected project container. Legacy app-wide installs
  (including old XDG roots) are copied into `projects/default` with rollback
  artifacts moved aside; multi-workspace legacy DBs migrate as one project
  container and record the future split policy instead of pretending
  books/profiles are cryptographic boundaries.
- [x] Move backend secrets (token, password, auth_header, basic-auth username
  + RPC aliases) out of the plaintext `config/backends.env` bootstrap and into
  the encrypted `backends` table. `kassiber secrets migrate-credentials` lifts
  pre-existing entries with a `.pre-credentials-migration-<ts>.bak` snapshot,
  and `bootstrap_runtime` warns to stderr whenever the dotenv still contains
  secret-shaped entries while the DB is encrypted. URLs / kinds / chain /
  network stay in the dotenv (they are addresses, not credentials).
- [ ] Tauri supervisor wiring — mostly shipped; one redaction gap remains. Done:
  startup passphrase modal (LockScreen → `daemon.unlock`), the stdin
  `auth_response.passphrase_secret` hand-off, the `auth_required`/`auth_response`
  reveal relay, and redaction of every named field **except `blinding_key`**.
  Remaining (security-relevant): add `blinding_key` (and the bare `blinding`
  substring) to all three secret-floor redaction layers (`supervisor.rs`,
  `kassiber/redaction.py`, `ui-tauri/src/lib/appLogs.ts`) plus a regression
  test — `docs/reference/daemon.md` requires it but the reveal payload's
  `blinding_key` currently passes through unredacted.
- [ ] Cross-platform CI for SQLCipher: PyInstaller bundle smoke tests on
  macOS arm64/x86_64, Linux x86_64, Windows x86_64. The CLI-binary smoke matrix
  runs **macOS arm64 (macos-latest) + macOS x86_64 (macos-15-intel) + Linux
  x86_64 (ubuntu-22.04)**; the remaining gap is **Windows x86_64** CLI-bundle
  smoke (Windows currently builds only the desktop preview).
- [ ] Optional convenience: opt-in OS-keychain remember-me layer and biometric
  reveal gate. macOS desktop builds now have the first half for database
  unlock: first lock-screen passphrase entry can enroll Touch ID for the next
  unlock, and **Settings → Security → Set up Touch ID unlock** verifies the DB
  passphrase and stores it immediately in Keychain. Later reads require a
  native LocalAuthentication Touch ID prompt before the passphrase is returned.
  Passphrase changes update the Keychain copy or disable Touch ID if the
  native store rejects the update; forgetting the setting deletes Kassiber's
  saved copy.
  The CLI now also has explicit `secrets remember-unlock` / `forget-unlock` on
  macOS Keychain, Windows Credential Manager, and available/unlocked Linux
  Secret Service, gated by a separate non-secret settings marker. Remaining
  work: biometric reveal gates for descriptor/token recovery and desktop
  remember-me affordances for Windows/Linux. These remain convenience over the
  SQLCipher passphrase, never a cryptographic substitute.
- [x] Kassiber skill bundle for agents (moved to
  https://github.com/bitcoinaustria/kassiber-skill)
- [ ] Optional server/REST mode, still local-first and opt-in

## Open bugs and debt

- [ ] Self-transfer audit follow-ups (deferred from the
  `claude/self-transfer-fixes` PR, which fixed the as-of/balance-history fee +
  income double-count, the non-positive-inbound detection asymmetry, and
  mixed-case txid grouping). These remain because each needs a representation or
  ordering decision and touches shared import/persistence paths where a rushed
  change risks regressions:
  - [x] **BTCPay amount/fee convention — false self-transfer quarantine (P1).**
    BTCPay's Greenfield wallet-transactions API has no per-tx fee field, so
    `normalize_btcpay_record` stores `amount = abs(net wallet delta)` (fee folded
    in) with `fee = 0`, diverging from esplora/electrum/bitcoinrpc (recipient-only
    amount + separate fee). Fixed via a `transactions.amount_includes_fee` marker
    (set on BTCPay outbound rows): the transfer-fee guard
    (`tax_events.normalize_tax_asset_inputs`) and its lockstep mirror
    (`transfer_matching._deterministic_self_transfer_ids`) now treat the out/in
    gap on a fee-inclusive leg as the miner fee, not an unrecognized outflow, so a
    BTCPay-leg self-transfer books correctly instead of being quarantined /
    routed to swap review. Node backends (marker 0) are unchanged.
  - [ ] **BTCPay folded-in fee — residual disposals (P2, residual of the above).**
    The marker fixes the *pure* self-transfer, but the fee value still cannot be
    read back from the Greenfield API, so the folded-in fee cannot be separated
    from any other outflow in `amount`. Two residual cases remain, both
    BTCPay-only, both leaving holdings correct (the net delta is what left):
    (a) *Standalone payment* — a BTCPay external payment folds the fee into
    `amount` with `fee = 0`, so its disposal overstates taxable proceeds by
    `fee * spot` (`rp2.py` SELL `crypto_out_no_fee = amount`, `crypto_fee = 0`).
    (b) *Paired/batched* — when one BTCPay tx pays a separately-synced owned
    wallet *and* an external recipient, `detect_intra_transfers` pairs the
    1-out/1-in legs and the P1 fix's `unrecognized_outflow = 0` books the whole
    out/in gap (external payment + miner fee) as a transfer fee instead of
    quarantining it, so the external payment is absorbed silently *regardless of
    size* — unlike the node-backed sub-ceiling case below, which is bounded by
    `max(1% of out, 2500 sats)`. A real fix needs out-of-band fee recovery (raw
    tx / NBXplorer) or a surfaced "fee unknown" state on the disposal; the marker
    added here is the groundwork.
  - [x] **Sub-ceiling external payment absorbed as a transfer fee (P2).** The
    canonical custody interpreter now decomposes graph-proven owned legs and
    external residuals before tax projection. Exact quantities are arbitrated
    once, so the owned slice becomes a MOVE and the external slice remains a
    disposal even when it is smaller than the old fee heuristic. Pure recorded
    pairs still require exact conservation; graphless ambiguity fails closed.
    Tests: `test_rp2_ownership_transfers.PartialPaymentCustodyArbitrationEngineTest`.
  - [x] **Cross-wallet consolidation quarantined instead of booked (was implicit
    in the fan-out limitation).** A spend funded by inputs from two or more owned
    wallets (consolidating e.g. Cold + Hot into Savings) was the one self-transfer
    shape both `detect_intra_transfers` and `derive_ownership_transfers` declined
    — each contributor syncs the tx independently and stamps the whole fee onto
    its own row, so summing the per-wallet rows double-counts the fee — leaving it
    in the `owned_fanout_unresolved` quarantine. Fixed via
    `ownership_transfers.derive_multi_source_consolidations` (run before the
    single-source deriver, feeding its touched ids forward): it reads the single
    fee once and the destination total from the graph, books one carrying MOVE per
    contributor (whole fee on the largest contributor), and replaces the recorded
    destination receipt with synthetic legs. Conservative scope — `>=2` contributing
    wallets, exactly one owned destination, no external output, all inputs owned,
    readable esplora graph, exact conservation; anything else still quarantines.
    Tests: `test_ownership_transfers.MultiSourceConsolidationDeriverTests`,
    `test_rp2_ownership_transfers.MultiSourceConsolidationEngineTest`.
  - [x] **Ambiguous partial payment cannot leak a phantom leg (P1).** If an
    owned output maps to multiple wallets or the physical event does not
    conserve, canonical arbitration emits a blocking custody conflict for the
    complete event. No partial decision reaches the finalized tax projection,
    so neither a full disposal nor a phantom acquisition can be booked. Test:
    `test_rp2_ownership_transfers.PartialPaymentCustodyArbitrationEngineTest.test_graph_proven_external_residual_blocks_when_owned_output_is_ambiguous`.
  - [x] **Multi-source consolidation double-counts an off-group destination receipt
    (P1, regression).** `has_external_receipt` required EXACT amount equality, so a
    destination receipt recorded under a different id at a slightly different amount
    (sat rounding / net-of-internal-fee, e.g. 0.79999 vs a 0.8 graph total) slipped
    past and the synthetic legs PLUS the surviving receipt inflated the destination
    ~2x. Fixed in `ownership_transfers.derive_multi_source_consolidations`: decline
    when the destination has any off-group same-asset inbound that matches the total
    OR lands within the spend's time window. Tests:
    `test_ownership_transfers.MultiSourceConsolidationDeriverTests.test_off_group_receipt_with_nonexact_amount_declined`,
    `test_rp2_ownership_transfers.MultiSourceConsolidationEngineTest.test_off_group_nonexact_receipt_does_not_double_count`.
  - [x] **Austrian self-transfer MOVE-fee disposal carried no regime → whole asset
    aborted (P1, pre-existing).** The IntraTransaction miner fee is a taxable
    disposal; with both Alt and Neu lots present rp2's moving-average raised
    `Ambiguous Austrian disposal` and the entire BTC report/journals process aborted
    (no quarantine). Fixed by stamping `at_regime` on the fee disposal:
    `austrian._move_transfer_availability` now returns the regime the fee draws from
    using a fee-first depletion model (the remaining carried quantity moves to the
    destination), and `infer_outbound_regimes` records it for the out row;
    `NormalizedTaxTransfer` gained an `at_regime` field and `_compose_transfer_notes` emits `at_regime=...`
    (matching the SELL path). Affects plain self-transfers and the derived
    consolidation/fan-out legs. Test:
    `test_rp2_ownership_transfers.AustrianSelfTransferEngineTest`.
  - [x] **Automatic ownership grouping trusted arbitrary `external_id` values
    (P1, final architecture audit).** Provider batch ids and CSV labels could be
    promoted to physical self-transfer identity, while the same txid on different
    networks was not scoped apart. `detect_intra_transfers`, deterministic swap
    suppression, graph merging, recorded fan-out/consolidation derivation, and the
    fan-out quarantine now share a canonical `(chain, network, 64-hex txid, asset)`
    boundary. Stored graph txids win; conflicting/unsupported chain metadata fails
    closed; Liquid never guesses a blank network. Arbitrary ids remain available to
    reviewed pairs/custody components. Mixed-case real txids still normalize.
  - [x] **Provider `swap_id` was exact/bulk-eligible without whole-row coverage
    (P1, final architecture audit).** Provider evidence is now exact only for a
    unique 1:1 key with canonical route txids plus explicit msat-denominated send
    and receive amounts equal to both complete rows. Route-only, duplicated,
    conflicting, or merely fee-tolerance-plausible evidence stays strong/manual.
  - [x] **`amount_includes_fee` silently called every transfer gap a miner fee
    (P1, final architecture audit).** Fee-inclusive gaps now require a complete
    valued canonical transaction graph (from any consistent owned observation) and
    exact fee equality. Missing/mismatched evidence fails closed for 1:1 and manual
    multi-pair booking; graph derivation allocates the proven fee and leaves any
    external residual as a disposal. Explicit custody components remain the escape
    hatch for missing wallets and N:M attribution.
  - [x] **bitcoinrpc multi-output tx double-counts the network fee (P2,
    pre-existing).** `record_from_bitcoinrpc_details` summed `fee` per detail, but
    Bitcoin Core stamps the SAME whole-tx fee on every `send`-category detail, so an
    N-output send booked the fee N×: a within-wallet split debited holdings by the
    phantom extra fee and a multi-recipient send overstated the taxable fee disposal.
    Fixed: take the fee once (`max` across details). Test:
    `test_sync_backends...test_bitcoinrpc_multi_output_send_does_not_double_count_fee`.
  - [x] **Derived multi-leg self-transfer not quarantined atomically (P2).** A
    consolidation / fan-out split into N MOVE legs could partially book if one leg
    was quarantined downstream (e.g. a fee leg needed pricing review under
    `require_coarse_review`), while the recorded destination receipt had already
    been dropped. Fixed by carrying a `group_id` from every derived multi-leg
    deriver (`derive_multi_source_consolidations`, `derive_ownership_transfers`,
    `derive_recorded_fanout_transfers`) into `NormalizedTaxTransfer`, then
    blocking the whole group in normalization and preflighting grouped MOVE legs
    atomically in the RP2 gate; real destination receipts replaced by synthetic
    group legs are carried as block rows so successful `transfer_in` journal
    entries point at the recorded receipt and failed groups quarantine it. Tests:
    `test_tax_events...test_derived_transfer_group_blocks_siblings_when_one_leg_needs_review`,
    `test_rp2_ownership_transfers...test_grouped_consolidation_gate_quarantines_atomically`.
  - [x] **Same-timestamp chained self-transfers mis-ordered → false
    insufficient_lots (P2).** Two self-transfers in the SAME block (shared
    `occurred_at`) where one funds a wallet and the next spends it (Cold→Hot then
    Hot→Exch — a consolidate-then-forward / batch pattern) could be processed
    spend-before-fund. Fixed by replacing the stream-index tie-break with
    `_ordered_rp2_items`, which keeps inbound events before MOVEs before outbound
    events and topologically orders same-timestamp transfers by wallet dependency;
    the gate and `IntraTransaction` insertion now share that order. Test:
    `test_rp2_ownership_transfers...test_same_timestamp_transfer_chain_books_funding_move_first`.
  - [x] **Swap-review fee omits out.fee (P2).** `compute_swap_fee = out.amount -
    in.amount` (`transfer_matching.py:257`) ignores `out.fee`, so the review
    surface, the persisted `transaction_pairs.swap_fee_msat`, and the GUI
    "Transfer fee" line (`ui_snapshot.py:1085`) show 0 / "no_fee_detected" for a
    same-asset on-chain self-transfer whose journal books a real miner-fee
    disposal. Fixed for matcher candidates plus pair/direct-payout persistence:
    whole-transaction swaps use `swap_fee_msat = out.amount + out.fee -
    in.amount` when the source stores fee separately, while reviewed split pairs
    keep the existing reviewed-principal semantics until a separate fee
    allocation is supplied.
  - [x] **RBF dropped-import phantom disposal (P3).** An RBF-replaced outbound
    captured in the mempool before eviction has no matching inbound and falls
    through to a taxable disposal (`tax_events.py:877`). Gated behind the
    esplora/mempool backend + a sync-timing race (BTCPay is confirmed-only and
    immune). Fixed with an explicit chain-status gate: only rows whose stored
    sync payload says `status.confirmed=false` are quarantined as
    `pending_onchain_confirmation` until a sibling leg proves confirmation.
    This intentionally does not use `confirmed_at IS NOT NULL`, because CSV and
    provider rows can lack that timestamp without being mempool transactions.
    The existing shared-prevout conflict pass remains independent.
  - [x] **Fee-tolerance flag desync (P3).** `_deterministic_self_transfer_ids`
    honors caller `--fee-pct-max/--fee-sats-min` but `tax_events`'s
    `transfer_fee_implausible` ceiling is hardcoded to the defaults, so
    `transfers suggest --fee-pct-max 0.5` empties a pair from the swap queue
    while the report still quarantines it. Deterministic self-transfer
    suppression now always uses the journal defaults; caller flags widen
    heuristic candidate generation only.
  - [ ] **Self-transfer change on an un-indexed script booked as a phantom
    external disposal (P1, round-2 deep audit).** A pure self-transfer A→B whose
    change returns to a source script the `OwnedIndex` does not contain (change
    branch past the derive ceiling, an un-indexed address-list change address, or
    an un-indexed multi-script-xpub change type) is mis-booked: the recorded
    outbound `amount` includes the un-indexed change, `_parse_onchain_tx`
    (`ownership_transfers.py`) discards prevout values so there is no input/output
    reconciliation, and the deriver folds the change into the external bucket and
    books the residual (`~341-349`) as a real disposal — silent over-taxation +
    holdings loss, strictly worse than the deriver-off `transfer_fee_implausible`
    quarantine. ASSESSED + deferred deliberately (the only round-2 P1 left open):
    a deriver-only guard cannot distinguish un-indexed change from a real external
    payment (the graph residual is identical either way), and quarantining large
    residuals would regress legitimate large partial payments (`test_mixed_spend_books_move_and_residual_without_phantom_fee`).
    Mitigation landed without a residual heuristic: wallet descriptor/xpub/
    address material is privately archived on migration and remains in the
    ownership index, inventory-derived historic floors survive the migration,
    and a bounded per-wallet `ownership_scan_to_index` can raise journal depth
    above 500 (hard-capped at 20,000). Residual risk remains for genuinely
    unknown address-list change or scripts beyond the configured ceiling. Do not
    close this last edge with a generic large-residual quarantine: that regresses
    legitimate partial payments.
  - [x] **Whole-row direct-swap-payout disposal lost to a same-txid auto-pair
    hijack (P2, round-2 deep audit).** A reviewed whole-row taxable direct payout
    whose out tx shares a txid with another owned wallet's recorded inbound was
    auto-paired by `detect_intra_transfers` (computed before the direct-payout
    claim set is built) and booked as a non-taxable MOVE — the declared disposal +
    proceeds vanished silently, no quarantine. Fixed in `rp2.py`: `auto_pairs` is
    pruned of any pair touching a whole-row direct-payout-claimed id before
    `apply_manual_pairs`, so the disposal books. Test:
    `test_rp2_ownership_transfers...test_whole_row_payout_not_hijacked_by_same_txid_inbound`.
  - [x] **Clamped amount=0 self-send invisible → phantom acquisition (P2, round-2
    deep audit).** A coinjoin/payjoin-shaped self-send where an owned wallet's net
    outflow fell below the whole-tx fee gets its outbound `amount` clamped to 0,
    while a positive inbound lands in another owned wallet under the same txid.
    Every positive-amount filter skipped the clamped source, so the destination
    booked a phantom standalone acquisition. Fixed in `tax_events._owned_fanout_row_ids`:
    a clamped amount=0 outbound sharing a txid with a positive inbound in a
    DIFFERENT owned wallet is quarantined (`owned_fanout_unresolved`) for review;
    single-wallet fee/consolidations (no cross-wallet inbound) are untouched.
    Tests: `test_tax_events.ClampedZeroSelfSendTest`.
  - [x] **Conflicting (shared-prevout / RBF) self-transfers both booked as MOVEs
    (P2, round-2 deep audit).** Two self-transfer outbounds spending the SAME
    prevout (RBF bump / reorg replacement) carry distinct txids, so no pass
    reconciled them and BOTH booked as carrying MOVEs → destination inflated.
    Fixed via `ownership_transfers.detect_conflicting_spend_ids` + a quarantine in
    `tax_events.normalize_tax_asset_inputs`: rows whose txids share an input
    outpoint are reconciled from the stored graph — a lone confirmed txid wins and
    its replacements' legs are quarantined `conflicting_spend`; if none or several
    are confirmed, the whole conflict is quarantined. Quarantine-only, never books.
    Tests: `test_ownership_transfers.ConflictingSpendTests`. (The broader RBF
    *dropped-import* phantom-disposal P3 above — opposite shape — remains separate.)
  - [x] **Direct-payout common path returned engine rows unsorted (P3, latent,
    round-2 deep audit).** The former synthetic direct-payout path returned rows
    in caller order on its no-payout branch. That path was fixed at the time and
    has since been removed by the custody-lineage cutover: finalized custody tax
    projection now owns event ordering before RP2. (The gate-ordering /
    same-timestamp determinism part of this finding was already addressed by the
    F5 same-timestamp fix, which removed the old `_gate_order_key`.)
  - [x] **Cross-chain script-collision guard defeated by blank chain/network (P3,
    round-2 deep audit).** `_norm_chain_network('', '')` defaults to
    `('bitcoin', 'main')`, so a blank-metadata reused-key Liquid match could pass
    the bitcoin/main chain filter and book a cross-chain disposal as a non-taxable
    MOVE. A comparison-time fix (treat blank as a distinct "unknown") was tried but
    REVERTED — Codex review flagged that legacy address-list / inventory matches
    legitimately store blank chain metadata and ARE bitcoin/main, so a real
    bitcoin/main source paying one of them would then fail the same-chain filter
    and have its owned output mis-booked as an external disposal. The correct fix
    is to normalize blank Bitcoin address metadata to bitcoin/main when the index
    is BUILT (`build_owned_index` / address-list + inventory seeding), so genuine
    cross-chain blanks (if any are reachable) stay distinguishable from
    legacy-mainnet blanks. Fixed at index construction: legacy blank address-list
    and inventory metadata is stamped `bitcoin/main`, while an explicit/fallback
    Liquid chain receives `liquid/liquidv1`; comparison remains strict.
  - [x] **Blocked ownership proof was quarantine-only (P2, 2026-07-10).** Journal
    ownership blocks and unresolved fan-outs now feed `transfers suggest` as
    redacted, pair-store-compatible `ownership_graph` candidates, report blockers
    point to that queue, quarantine deep-links prefilter and focus the exact leg,
    and `conflicting_spend` has a distinct non-pairing review action. Journal and
    graph preview share `derive_profile_transfers`; identify, ownership, and
    source-funds share one stored vin/vout parser. Pairing changes holdings only
    after journal reprocessing; no candidate payload exposes scripts or wallet
    derivation material.
  - [x] **Round-3 self-transfer batch (2026-07-08).** ~30 one-bug-per-commit
    fixes across the transfer machinery: Samourai tx0 group atomicity +
    manual-pair collision quarantines, multi-pair fee ceiling + privacy-leg
    quarantines + display dedup (journals list, summary PDF, per-(leg, role)
    pair counts), SoF fee-bearing N:1 allocation, pair kind-edit fee
    reconciliation, LN sub-sat clamp (booking + regime inference in lockstep
    via `pair_allocation.clamped_receipt_msat`), matcher mirror for
    journal-netted LN hash pairs, blocked-source suppression premise,
    per-leg group gate application with re-check (both intermediate-spend
    abort directions), tax-free hints classifying per-regime quantity flows
    (`at_alt_out`/`at_alt_in` markers via the new `journal_markers` module),
    Lightning channel lifecycle (force-close sweep matching, grouped
    multi-sweep close fee with an explicit CHANNEL_CLOSE_MISMATCH ceiling —
    the generic `unrecognized_outflow` guard is definitionally zero for
    cloned-amount synthesized pairs — funding-with-external-payment
    quarantine, batched-open sums, bkpr sanitizer keeping
    credit_msat/debit_msat), and shared infrastructure in
    `pair_allocation.py` (ordering, allocator, clamp, component builder).
  - [x] **Round-3 accepted remainders (P3).** Multi-leg components now clamp
    sub-sat receipt excess through the shared `pair_allocation.py` helper in
    both booking and Austrian inference; journal pair payloads select one
    representative pair before reading pair columns, so chain-reused legs cannot
    mix fields from different pairs; Austrian inference now uses the same
    reviewed-pair component membership as booking, leaving derived pairs to
    their own group path.
  - [ ] **Make the Austrian disposal-ordering election configurable.** For a
    disposal from a wallet holding both Alt and Neu inventory, Kassiber picks
    Neu-first; the KryptowährungsVO presumption absent a designation is
    earliest-acquired-first (usually Alt). The choice is now recorded in the
    audit trail (`at_regime_basis=wahlrecht` on the journal disposal, emitted
    from `infer_outbound_regimes`' election set), so the exercised Wahlrecht is
    documentable — but it should become a profile-level setting with
    earliest-first available as the statutory default, and the basis marker
    should extend to swap legs and self-transfer fee slices (today only
    standalone disposals carry it). Changing the default silently would change
    existing users' tax outcomes, so it needs an explicit migration story.
  - [x] **Harden LND and CLN channel-lifecycle evidence.** LND records now use
    a stable namespaced funding-outpoint channel id, retain settled + timelocked
    close balances, skip remote-funded opens, and require an explicit local
    funding contribution before suppressing an L1 open with a compensating
    MOVE. Stock LND REST capacity is deliberately *not* treated as the user's
    contribution (push amounts, leases, and dual funding make that unsafe):
    when the contribution is unavailable, the amount remains incomplete and
    the lifecycle path fails closed into review. CLN now retains one lifecycle
    record per bookkeeper channel account, including every account in a shared
    `multifundchannel` funding transaction; account-aware external ids prevent
    persistence collisions, while the lifecycle engine still sums all channel
    contributions for one whole-transaction mismatch check and emits one
    atomic on-chain-wallet → node MOVE. On close, multiple competing force-close
    vin matches require an explicit local commitment outpoint; otherwise the
    engine emits `CHANNEL_CLOSE_MISMATCH` and no MOVE.
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
- [x] Add live provider-backed FX adapters beyond CoinGecko and local Kraken CSV
  (Coinbase Exchange — now the default — and Mempool shipped), under the
  honest-exposure bar: the UI/daemon expose the provider list, per-rate
  granularity, and coarse-review state (and dropped fabricated synthetic
  provider-health rows). Any further providers must keep that same bar
- [ ] Keep the machine envelope boundary centralized and explicit
- [ ] Keep docs and examples Bitcoin-only
- [ ] Add a narrow docs-drift check for shared command / verification /
  safe-to-record surfaces so `README.md`, `AGENTS.md`, `SECURITY.md`, the
  in-app AI references, and the external Kassiber Agent Skill do not quietly
  diverge
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
- [ ] Replace or independently audit `spake2==0.9` before treating LAN pairing
  as a hardened long-term transport. The package is pure Python, has not been
  maintained since 2018, and explicitly does not claim constant-time
  behavior. The current use remains a one-guess, short-lived PAKE protected by
  Ed25519 device proof-of-possession and a strict session deadline; preserve
  those compensating controls during any replacement.

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
