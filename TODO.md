# Active backlog

Keep the next action and completion condition here. Product direction and
architecture decisions live in [the plan overview](docs/plan/00-overview.md);
working rules and verification live in [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). Remove completed tasks after preserving any
lasting contract in its owning reference. Git history retains prior checkpoints.

## Accounting correctness and custody

- [ ] Preserve native observer evidence during supporting CSV price enrichment.
  A matched generic-ledger import must not replace native `raw_json` authority
  with CSV fields and turn an exact claim/refund transfer into a disposal.
  Keep pricing provenance separate and test the subsequent journal rebuild.
- [ ] Reconcile obsolete native rows when watched scripts expand. Incremental
  history must match a complete rescan's quantity and basis when the same
  transaction changes from receipt to spend. Retain authored metadata and
  review references.
- [ ] Resolve the remaining custody-core code-volume stop criterion in
  [plan 15](docs/plan/15-custody-simplification.md). Consumer/report cutover and
  performance verification are recorded there as complete. Do not remove
  migration, signed replay, immutable audit, or planner protections to meet
  the remaining criterion; it has not been waived by documentation cleanup.
- [ ] Represent separately timed fees for native multi-date HTLC routes.
  Preserve exact linkage while blocking automatic tax projection that would
  place a principal shortfall at the funding timestamp. Prove fee timing,
  in-transit custody, and cross-period balances/basis/reports. Looser matching
  or quarantine exclusion is not a fix.
- [ ] Resolve the reported BTCPay folded-in-fee disposal cases: a standalone
  payment can overstate proceeds, and an owned-plus-external batched payment
  can absorb the external outflow into transfer fees. Recover fee evidence
  from an authoritative source or expose an explicit fee-unknown state;
  correct net holdings alone do not prove correct tax treatment.
- [ ] Resolve the deferred unknown-change-script case: a self-transfer's change
  outside the ownership index can appear as an external disposal. Archived
  policies, historical derivation floors, and bounded deeper scans mitigate
  known scripts; genuinely unknown address-list change remains unresolved.
  Do not add a generic large-residual quarantine: it regresses legitimate
  partial payments. Preserve
  `test_mixed_spend_books_move_and_residual_without_phantom_fee`.
- [ ] Re-check Austrian moving-average wallet/address pool interpretation
  against primary guidance before enabling narrower pools or expanding support
  claims. Keep global-only behavior until the legal/product and chronological
  replay gates in [the pool plan](docs/plan/16-cost-basis-pools-and-employment-compensation.md)
  are satisfied; fee-bearing narrower-pool movement must fail closed.
- [ ] Make the Austrian Alt/Neu disposal-ordering election configurable.
  Preserve the recorded `at_regime_basis=wahlrecht` decision and extend the
  basis marker to swap legs and self-transfer fees. Validate earliest-acquired
  ordering against current guidance and design an explicit migration; silently
  changing Neu-first would change existing users' tax outcomes.
- [ ] Evaluate per-wallet physical-lot attribution only if a jurisdiction
  requires it. Current per-wallet basis allocation is not a physical-lot claim.

## Organizational accounting acceptance

The accepted scope is [spec 17](docs/plan/17-general-accounting-and-private-ai-spec.md).
Implementation and measured acceptance are distinct; the
[coverage record](docs/reference/general-accounting-acceptance.md) owns evidence
and remaining gaps. Private portfolio use must not require corporate setup.
The [three-cut policy](docs/plan/18-general-accounting-pr-stack.md) preserves the
separation of deterministic core, ordinary opaque tasks, and selected financial
AI. The non-reproduced SQLCipher observation is an accepted residual risk,
not fixed or disproven. Other technical failures still block delivery; #551
remains outside that closeout.

- [ ] Close out the accounting cuts with independent review and required
  per-cut/full verification on the source being delivered. Historical green
  checkpoints do not prove a later revision or packaged runtime.
- [ ] Confirm the organization's actual source population, accounting regime,
  opening balances, and K2/annex/year applicability. Do not assume every Verein
  files K2. Missing pilot facts and measurements remain product-acceptance gaps,
  not automatic code-merge blockers.
- [ ] Complete the spec's coverage and acceptance matrix for exact double-entry
  books, manual/proposed postings, bank imports, source allocations, required
  schedules, reconciliation, financial reports, and two-year close/correction.
  Use retained artifacts and the independent arithmetic verifier; implementation
  checkpoints alone do not complete this requirement.
- [ ] Prove RP2 point-in-time/regime/pool suitability and retain replay context
  and result artifacts. Keep book valuation adjustments distinct from tax basis.
- [ ] Verify retained encrypted evidence, local OCR/extraction/search, reviewed
  batch proposals, and remote disclosure controls across real multi-round
  workflows. Preserve action-specific approval, separate disclosure grants,
  cancellation, idempotency, and manual fallback.
- [ ] Complete narrow country-pack and AT K2/required-annex coverage, including
  applicability, carryforwards, and independent close exports. K1 and additional
  country/form adapters need separately resolved scope. EBICS and FinanzOnline
  submission remain excluded.
- [ ] Complete CLI/scoped-agent and minimal localized Assistant-consent proof,
  adversarial accounting/privacy tests, golden books, package/build verification,
  and review. Include legacy personal workflows, book isolation, retained
  evidence, backup/restore, diagnostics exclusions, and local ledger storage.
- [ ] **AF-1:** finish a selected period from one task on a frozen mixed
  100-record benchmark. Cover every selected record and measure routine-case
  accuracy, exceptions, corrections, repeated inputs, and active user time.
- [ ] **AF-2:** execute approved close, final K2/annex working papers, and export
  through actual agent tools; verify retained artifacts rather than instructions.
- [ ] **AF-3:** turn an approved correction into a versioned, book-scoped,
  revocable proposal rule. Prove reuse, conflicts, and revocation without
  extending posting/disclosure consent or rewriting earlier decisions.
- [ ] **AF-4:** resolve missing evidence, ambiguous partial payments, and
  conflicting classifications within the same task. Preserve independent work
  and resume without duplicate actions or revived permissions. Existing
  deterministic amendment/resumption tests do not replace agent/pilot proof.
- [ ] **AF-5:** complete a private portfolio/assignment workflow through actual
  agent tools and verify recomputed reports without organizational setup.

## Imports, source evidence, and wallet workflows

- [ ] Offer a stored import run's `column_map` as the default for a repeat
  platform export, with review before reuse. Generic mapping already exists;
  this task is persistence/reuse in the next import flow.
- [ ] Identify any remaining rates/manual-adjustment workflow gap against the
  existing surfaces before defining an implementation.
- [ ] Add CLI parity for the desktop's bare-xpub script-type probe, reusing
  `detect_active_script_types`. Keep pinned script types available, backend
  scope explicit, and fallback behavior documented when no history is found.
- [ ] Build the dedicated commercial reconciliation queue/workbench on the
  existing BTCPay provenance/document APIs; transaction-detail context already
  exists. Scale reviewed suggestion resolution without duplicating balances.
- [ ] Improve dense source-of-funds graph presentation after real-user feedback.
  Preserve the existing editor and reviewed evidence workflow.
- [ ] Add optional configured-backend observations to source-of-funds review
  with an explicit public-backend privacy warning. Keep observations weak
  suggestions until reviewed; reuse the shared acquisition boundary.
- [ ] Drive cooperative Boltz v2 signing paths directly through an official
  client/SDK in the isolated regtest harness. External evidence ingestion
  already exists. Persist only redacted provider/route/principal/status facts;
  missing whole-row evidence stays heuristic/manual rather than becoming exact.
- [ ] Expose `detect_repeating_patterns` through a narrow daemon kind and a
  "Create rule from this pattern?" review action; the pure helper already exists.
- [ ] Design an opt-in encrypted Lightning evidence vault for proof-of-payment,
  invoice recovery, and audit custody. Keep it separate from normal daemon,
  AI, and diagnostics surfaces; preserve the adapter discard policy in
  [Lightning opsec](docs/reference/lightning-opsec.md).

## Desktop and handoffs

- [ ] Finish the localization long tail: deferred report/exit-tax/Lightning
  reporting surfaces, shared enum-to-label helpers, and locale-aware number/date
  formatting. Follow [i18n](docs/reference/i18n.md) and the Austrian glossary;
  keep CLI/daemon output machine-deterministic.
- [ ] Add a book-set treasury CSV/PDF export with BTC holdings/activity,
  per-book fiat rows, and a readiness manifest. Keep tax lots, transfers,
  capital gains, and mixed-fiat semantics book-scoped.
- [ ] Add destructive single-book deletion UX and a scoped `ui.profiles.delete`
  contract. Resetting book data and deleting a workspace are separate actions.
- [ ] Add GUI backup/restore on narrow daemon kinds using the existing backup
  core, with explicit review and isolated restore verification.
- [ ] Add scoped handoff import, selected-books audit-package export, and an
  actionable restricted technical-wallet-evidence path. Single-book audit export
  already exists; do not widen default disclosure when adding these paths.
- [ ] Add optional chat-history retention by session count or age, enforced
  at append time and configurable from CLI and desktop Settings.
- [ ] Finish biometric reveal gates for descriptor/token recovery and desktop
  remember-me affordances on Windows/Linux. Existing desktop Touch ID and CLI
  remembered unlock remain convenience over SQLCipher, never a substitute.

## Daemon, contracts, and performance

- [ ] Design general mutation-safe cancellation and worker execution beyond
  specialized AI/sync jobs. Use one SQLite connection per worker and preserve
  book scope, durable writes, shutdown, and cancellation semantics.
- [ ] Extend progress/cancellation UI to remaining long-running live actions
  after their execution semantics are safe.
- [ ] Decompose remaining CLI handlers into domain APIs with explicit input and
  envelope boundaries; do not rebuild shared domain logic in handlers.
- [ ] Centralize safe-view projection so CLI, desktop, and AI consumers retain
  their deliberate audience differences under one redaction contract.
- [ ] Generate daemon/renderer/bridge allowlists from a common contract while
  preserving separate authority scopes. Existing drift tests remain required.
- [ ] Derive JSON Schema and TypeScript contracts from validated Python models
  (the proposed Pydantic v2 path), with schema drift failing CI.
- [ ] Resolve the dev-bridge mutation/authentication policy: read-only by default
  with explicit mutation opt-in, or a per-launch token. The shipped transport
  is Vite HTTP middleware, not the old token-WebSocket plan. Complete the
  relevant missing/wrong-token, non-loopback-bind, production-env refusal, and
  no-token-in-logs checks for the chosen design; preserve origin containment.
- [ ] Add systematic generated-artifact secret-leak scanning in CI beyond the
  existing log/bridge redaction tests.
- [ ] Add focused documentation drift checks for command, verification, and
  safe-output contracts across contributor docs, security guidance, in-product
  AI references, and the CLI skill. Avoid recreating manual catalogs.
- [ ] Parallelize background `run_due_jobs` with single-flight handling and
  worker-owned connections; foreground cross-wallet parallelism does not prove
  background safety or performance.
- [ ] Batch cached-rate lookup and pricing updates in
  `auto_price_transactions_from_rates_cache`; prove equivalent pricing and
  journal results across missing/stale-rate cases.
- [ ] Represent a live but rate-limited connection as `throttled`, with a
  `rate_limited_until` countdown in freshness/sync results.
- [ ] Derive `kassiber.__version__` from package metadata; keep the existing
  version-drift check until duplicated version authority is removed.
- [ ] Replace the remaining generic Latin-1 PDF renderer with Unicode-capable
  output or a structured `pdf_unrepresentable` error listing codepoints.
  Update `test_pdf_report_substitutes_non_latin1_glyphs`; silent `?` substitution
  is not an acceptable final state.

## Privacy and security follow-ups

- [ ] Remove deprecated argv credential forms and migrate backend/wallet test
  setup to stdin/fd. Tighten plaintext dotenv-secret warnings into refusal
  after supported tests/examples no longer depend on secret seeding there.
- [ ] Separate descriptors and other sensitive wallet configuration from the
  generic `wallets.config_json` blob into typed project-local storage, preserving
  SQLCipher and compatibility boundaries.
- [ ] Complete explicit, opt-in guided Tor setup under
  [issue #311](https://github.com/bitcoinaustria/kassiber/issues/311). No silent
  install/start, global routing, or clearnet fallback for onion endpoints.
- [ ] Replace or independently audit the pinned SPAKE2 implementation before
  claiming hardened long-term LAN pairing. Preserve the one-guess session,
  strict deadline, and Ed25519 device proof while evaluating a replacement;
  the existing implementation does not claim constant-time behavior.
- [ ] Reassess the transitive `glib` VariantStrIter advisory during the next
  Tauri/GTK upgrade. Its recorded `not_used` dismissal is not an upstream fix;
  verify the dependency graph and affected usage before changing that assessment.

## Distribution

- [ ] Publish RP2 as a versioned wheel and update the locked dependency path,
  resolving the VCS-pinned packaging concern without changing engine behavior.
- [ ] Decide whether packaged runtime requirements justify replacing PyInstaller
  with a `python-build-standalone` tree. Preserve the installed-app CLI path.
- [ ] Complete Linux channel provisioning and first-release checks in the
  [operator backlog](docs/reference/linux-packaging-operator-todo.md). Verify
  live candidates/signatures before publishing package-manager install claims.
- [ ] Lower the frozen Linux CLI glibc floor before promising older RHEL/openSUSE
  support. Preserve the supported matrix in [Linux packaging](docs/reference/linux-packaging.md).
- [ ] Add Linux ARM64 packaging only when both pinned observer bindings can be
  built reproducibly and verified on native ARM CI across CLI, Debian,
  AppImage, and channel metadata.
- [ ] Complete production signing/distribution activation: live macOS credential
  validation and clean-Mac signed/notarized installation, Windows signing,
  and Linux repository/artifact signatures. Resolve the desktop plan's AGPL
  packaging opinion or explicit residual-risk acceptance gate before shipping.
- [ ] Establish the permanent offline OpenPGP release key, publish its public
  key and full fingerprint independently, add reviewed verification policy and
  client key material, and activate draft-only two-person finalization. Keep
  the offline release key separate from the CI-held Linux archive key. GitHub
  update notices remain a separate notification-only HTTPS trust path.

## Deferred product decisions

- [ ] Evaluate full-chain observation capacity and coverage before claiming it.
  Incremental local indexing and explicit evidence watches already exist; the
  remaining work needs bounded acquisition/storage/scale proof.
- [ ] Revisit optional server/REST operation only through an explicit future
  design. It is outside the accepted desktop/local CLI scope.
