# Active backlog

Keep the next action and completion condition here. Product direction and
architecture decisions live in [the plan overview](docs/plan/00-overview.md);
working rules and verification live in [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). Remove completed tasks after preserving any
lasting contract in its owning reference. Git history retains prior checkpoints.

## Accounting correctness and custody

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
- [ ] Resolve the remaining BTCPay folded-in-fee disposal cases. Sends that
  completed BTCPay payouts explain now book their fee separately; a send BTCPay
  did not create as a payout (for example from its wallet send page) still
  carries the fee inside the amount and can overstate proceeds, and an
  owned-plus-external batched payment can absorb the external outflow into
  transfer fees. Recover fee evidence from an authoritative source or expose
  an explicit fee-unknown state; correct net holdings alone do not prove
  correct tax treatment.
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

## Quarantine follow-ups

- [ ] Delete the pre-custody normalizer branches that no finalized row can
  reach (`tax_events` Samourai/multi-pair/unscoped/loan/channel paths,
  `pair_allocation` max-flow, the `austrian` multi-source branch, RP2 loan
  branches). First keep the restored `unscoped_transfer_review` and
  `owned_fanout_unresolved` holds in the live interpreter and migrate the
  raw-row tests that pin the dead branches; done when the projection-id guard
  test also covers `accounting-transit:` rows and the suite stays green.
- [ ] Tell the owner when `ownership_transfer_source_missing` points at an
  address beyond the wallet's synced depth (the ownership index derives deeper
  than the sync gap limit) and offer that wallet's scoped rescan; done when the
  quarantine row carries the used index and the rescan is a user action.
- [ ] Bound `ui.journals.quarantine` for very large quarantines: page items in
  SQL and compute the whole-book groups, counts and assumptions with aggregate
  queries; done when the first page no longer parses every stored row.
  Carry downstream rows (`at_swap_basis_carry_unresolved`,
  `bitcoin_rail_carry_basis_unresolved`) and `basis_provenance_incomplete`
  should record their causal transaction ids so the root is exact.
- [ ] Let the owner confirm presumed disposals in bulk (an outbound
  `kind_override` operation in `review plan/apply`) and show per-wallet
  history coverage; decide whether custody-gap review leaves the developer
  gate.

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
  existing BTCPay provenance/document APIs; transaction detail already shows
  suggested payments/payouts with confirm/reject review. Scale reviewed
  suggestion resolution without duplicating balances.
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

- [ ] Let an agent request a brokered session that the user approves with
  Touch ID on signed macOS builds: allow `operator unlock --auth touch-id`
  without a terminal when explicitly requested, since the biometric prompt is
  the user's action and no secret reaches the caller. Keep enrollment and mode
  changes human-only, document it as the one explicit GUI prompt, and verify on
  a signed, notarized build. The desktop's *Unlock for agents* already covers a
  user who has the book open; this item is for agent-initiated requests
  without the desktop.

- [ ] Finish the localization long tail: deferred report/exit-tax/Lightning
  reporting surfaces, shared enum-to-label helpers, and locale-aware number/date
  formatting. Follow [i18n](docs/reference/i18n.md) and the Austrian glossary;
  keep CLI/daemon output machine-deterministic.
- [ ] Add a book-set treasury CSV/PDF export with BTC holdings/activity,
  per-book fiat rows, and a readiness manifest. Keep tax lots, transfers,
  capital gains, and mixed-fiat semantics book-scoped.
- [ ] Add destructive single-book deletion UX and a scoped `ui.profiles.delete`
  contract. Resetting book data and deleting a workspace are separate actions.
- [ ] Add scoped handoff import, selected-books audit-package export, and an
  actionable restricted technical-wallet-evidence path. Single-book audit export
  already exists; do not widen default disclosure when adding these paths.
- [ ] Add optional chat-history retention by session count or age, enforced
  at append time and configurable from CLI and desktop Settings.
- [ ] Finish biometric reveal gates for descriptor/token recovery and desktop
  remember-me affordances on Windows/Linux. Existing desktop Touch ID and CLI
  remembered unlock remain convenience over SQLCipher, never a substitute.

## Daemon, contracts, and performance

- [ ] Enforce AI consent, advertisement, and scope in one daemon-owned
  `authorize_and_execute` path shared by chat and MCP. Executors fail open
  when the runtime state is empty, and fresh human review for `review.apply`
  is enforced only by the CLI client. Add per-tool
  approval modes (`none` / `consent` / `local_human`), explicit per-tool
  network/destructive/idempotent metadata pinned to the egress table (four
  mutating tools egress with `egresses=False`), and per-call capability
  checks, before any mutating tool is exposed through `kassiber mcp`. Consent
  for MCP mutations must bind a plan digest and input version and must not
  trust a host-answered elicitation alone.
- [ ] Give brokered operations a full agent lifecycle: caller-supplied
  operation ids (reusing the broker's dedupe), `operator operations list`,
  `--no-wait` plus `operator operation result`, precise failed-vs-unknown for
  pre-transaction errors, and specific startup diagnostics instead of
  retryable `operator_broker_start_failed` (for example missing logind).
- [ ] Make a repeat open of a current database write nothing. Each open
  still runs about 30 idempotent write statements (source-funds link and case
  normalization, AI provider display names, BIP-329 wallet ids, custody
  component legs, the wages sentinel `INSERT OR IGNORE`, and `DROP`/`CREATE
  TRIGGER`/`INDEX IF EXISTS` pairs), so a CLI or MCP read opened while the
  desktop or a broker child writes waits for the lock and can end in
  `database_busy`. Gate them on schema version or a completed-migration
  marker, with a test that a second open issues no writes.
- [ ] Extend `commands describe` to catalog v2: argument JSON schemas (types,
  booleans, defaults, mutually exclusive groups), explicit secret-argument and
  sensitive-output registries, emitted kinds, egress class, and pagination
  location, with drift tests. Unify list pagination on `data.page`.
- [ ] Echo the resolved book scope in envelopes and report silent context
  switches (`workspaces create`, `profiles create`), so unscoped agent
  commands cannot act on a book changed by another client.

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

- [ ] Harden against infostealers running as the user: `exports/` and
  `attachments/` sit outside SQLCipher, CLI remembered unlock on Linux is
  readable by any same-user process through the unlocked Secret Service, and
  terminal operator leases default to "until lock". Encrypt or clearly warn
  about the first two, surface the Linux risk in `secrets status`, and give
  terminal leases a bounded default.
- [ ] Decide whether MCP results pseudonymize addresses and txids by default
  (stable per-book aliases, with a switch for real ids), which also limits
  what reaches an agent's AI provider.
- [ ] Verify whether Claude Code's and Codex's agent sandboxes block the
  operator broker socket for the agent's own commands while the host-launched
  MCP server still reaches it, and document the recommended settings.
- [ ] End the AI broker's whole process tree on Windows (a Job Object, plus a
  cooperative shutdown message so it can clean up). `Popen.terminate()` ends
  only the broker there, so a cancelled or timed-out chat leaves the provider
  CLI running; the daemon-owned temp root is removed best-effort afterwards.
- [ ] Add more Agent Client Protocol agents to `provider-broker/src/acp.ts`
  once each can start with every built-in tool removed and none of the user's
  MCP servers, hooks, memory, or settings loaded, verified against a live
  signed-in CLI. Gemini needs a non-root way to override user settings (and to
  stop `tools.discoveryCommand` and `GEMINI.md` memory); Cursor and Grok need
  a tool-free ACP mode. See [the AI reference](docs/reference/ai.md#agent-client-protocol-agents).

- [ ] Finish the desktop restore contract for CLI `backup import --install`:
  it already refuses a live desktop or broker owner, but it still skips
  credential invalidation and proof that the restored inner DB is encrypted
  with a known passphrase, so a partition archive can silently turn a project
  plaintext.
- [ ] Keep encryption sticky: record that a project must be encrypted and
  return `resume_required` or `missing_database` instead of creating a new
  plaintext DB when the file is missing (interrupted `secrets init`, unmounted
  volume). An agent's `status` currently answers "run `kassiber init`".
- [ ] Surface and clean up plaintext leftovers: `kassiber.pre-encryption.sqlite3.bak`
  and `backends.env.pre-credentials-migration-*.bak` in `secrets status`,
  startup warnings, and the desktop, with a cleanup command after
  `secrets verify`. Stage backup/restore/export temp trees inside the project,
  and create project dirs 0700 and DB/attachment/export/transcript files 0600
  regardless of umask.
- [ ] Stop `secrets status` from connecting to the Linux Secret Service when
  CLI remembered unlock was never enabled.

- [ ] Remove deprecated argv credential forms and migrate backend/wallet test
  setup to stdin/fd. Tighten plaintext dotenv-secret warnings into refusal
  after supported tests/examples no longer depend on secret seeding there.
- [ ] Separate descriptors and other sensitive wallet configuration from the
  generic `wallets.config_json` blob into typed project-local storage, preserving
  SQLCipher and compatibility boundaries.
- [ ] Complete explicit, opt-in guided Tor setup under
  [issue #311](https://github.com/bitcoinaustria/kassiber/issues/311). No silent
  install/start, global routing, or clearnet fallback for onion endpoints.
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
- [ ] Publish the SSH release-key fingerprint
  `SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M` on the Kassiber website
  and through one independent Bitcoin Austria channel, compare them with this
  repository, and only then embed the key in packaged builds. Add two-person
  finalization once a second operator exists, and consider moving the key to
  an `ed25519-sk` hardware key; see [release signing](docs/reference/release-signing.md).
  Keep it separate from the CI-held Linux archive key. GitHub update notices
  remain a separate notification-only HTTPS trust path.

## Deferred product decisions

- [ ] Evaluate full-chain observation capacity and coverage before claiming it.
  Incremental local indexing and explicit evidence watches already exist; the
  remaining work needs bounded acquisition/storage/scale proof.
- [ ] Revisit optional server/REST operation only through an explicit future
  design. It is outside the accepted desktop/local CLI scope.
