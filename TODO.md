# Active backlog

Keep the next action and completion condition here. Product direction and
architecture decisions live in [the plan overview](docs/plan/00-overview.md);
working rules and verification live in [AGENTS.md](AGENTS.md) and
[CONTRIBUTING.md](CONTRIBUTING.md). Remove completed tasks after preserving any
lasting contract in its owning reference. Git history retains prior checkpoints.

## Rust backend R0

Stage R0 of [plan 21](docs/plan/21-rust-backend.md#stages). The toolchain,
workspace, CI caching, `crates/pydecimal`, `crates/pyjson`, the contract and
test-disposition inventory, the session harness, the Rust no-egress guard, and
the feasibility spikes are in place; these remain:

- [ ] Add test-only clock and entropy injection for whole CLI and daemon runs,
  so recorded sessions replay strictly. The custody build clock and its hash/id
  golden vectors are in place.
- [ ] Bound what the session harness snapshots. Replay pins every file under
  the recorded roots, including `HOME`; a home holding a toolchain made one
  replay tokenize gigabytes, hang `test_cli_session_replay.py` for over five
  minutes, and fill `/tmp`. Skip or cap large trees and fail with a clear
  message instead of hanging.
- [ ] Validate the spike conditions natively: SQLCipher packaging and keyring
  formats on macOS and Windows, and the Touch ID helper's identity check
  against a signed Rust sidecar.
- [ ] Validate the Rust HTTPS/HTTP foundation on all release targets and run its
  owned-loopback regressions outside restricted sandboxes. The gate, inventory,
  TLS/HTTP/proxy adapters and bypass scanner are in
  [the Rust egress reference](docs/reference/rust-port-egress.md); service slices
  still need their own consent, envelopes and integration proof.
- [ ] Check the Cargo cache once a push to `main` has saved it: the uv jobs'
  binding build should drop from about 25 s cold. If the engine crate's own
  rebuild still dominates, share the built wheel across the shards instead.

## Rust backend R2

The [raw custody v1 snapshot, pure Python stages and Rust components/evidence/interpretation/gaps/quantity stages](docs/reference/custody-engine.md)
are in place with required shadow/census comparison, CPython parser vectors,
raw latest review rows and killed stage mutation catalogs. Python
remains the production reference; every subsequent stage needs its own killed
mutation entries.

- [x] Finish native component reconciliation and complete interpreter composition;
  [interpretation now participates in full-stage comparison](docs/reference/custody-engine.md#complete-interpretation-checkpoint-r2c).
- [x] Port bounded gap search over the captured SQL worklist, including matcher
  capacity and source blockers; [gaps now participates in full-stage comparison](docs/reference/custody-engine.md#gap-search-checkpoint-r2d).
- [x] Complete quantity arbitration and runtime composition, including gap
  holds, whole-state issues and scoped tax barriers;
  [quantity now participates in full-stage comparison](docs/reference/custody-engine.md#complete-quantity-runtime-checkpoint-r2e).
- [x] Complete finalized tax projection and compare it before journal fusion;
  [the raw adapter and whole-stage gate](docs/reference/custody-engine.md#whole-stage-parity-gate)
  cover the inherited books and recorded requests. Finalized projection now
  participates in PyO3, census, custody-shadow and replay; decision-only calls
  explicitly skip it. Rust results remain comparison-only and snapshot v1
  remains raw-only.
- [ ] Join custody and the tax engine in one `journal` operation only after
  every stage passes the frozen oracle, generated books and corpus comparisons.
  The [R2g request-builder checkpoint](docs/reference/custody-engine.md#rust-journal-request-builder-checkpoint-r2g-step-1)
  ports request construction; [fusion harness integration](docs/reference/custody-engine.md#fusion-harness-integration-checkpoint-r2g-step-3)
  now compares requests and ledger responses in PyO3/census/shadow/replay.
  Generated/retained/fresh replay campaigns pass; 2,211 inherited mutation
  recipes still need rerunning. Post-ledger journal joins remain outstanding.

## Rust backend R3 storage

- [x] Resolve the historical Python custody-view idempotency blocker and verify
  the private ordered Rust compatibility pass: both pair-free fixtures have
  exact first-pass dumps and unchanged Python reopen in plaintext/SQLCipher;
  both original fixtures retain exact prefixes and the explicit R2 refusal.
  This is fixture acceptance, not complete R3 acceptance; see
  [R3 storage status](docs/reference/rust-storage-r3-status.md).

The real binary now wires supported creation/open/unlock, compatible ownership,
ordered compatibility, hidden-home cutover and legacy project migration through
`init`, `status` and the daemon. Complete acceptance remains below.

- [ ] After R2's matcher port, port authored pair/payout consolidation,
  revisions/issues and componentless dismissal recovery; require complete
  original-fixture parity and unchanged Python reopen. Python must migrate
  affected books until then; the Rust binary refuses those layouts.
- [x] Port private legacy flat-book/project migration with exact logical database
  dumps, file bytes/modes and cross-runtime crash recovery; keep the Python fix
  independently applicable. Explicit network partition export belongs to R5.
- [ ] Complete remaining R0 `store` dispositions and full historical migration
  acceptance; supported runtime integration is not full R3 acceptance.
- [ ] Complete storage crash/backup and Windows/macOS acceptance, then wire
  maintenance commands with Python-safe journals and credential stale guards.

## Rust backend R5 rates

The partial [rates checkpoint](docs/reference/rust-shell.md#rates-service-checkpoint-r5-partial)
implements six CLI leaves, four daemon kinds and the shared cache/provider API.

- [ ] Complete desktop rebuild after pricing/journals are ported, preserving
  rollback, profile scope and transaction-price provenance.
- [ ] Finish the native rates intent inventory and the remaining provider
  error/Retry-After-date, archive/resource, label-context and installed-resource
  parity gates documented in the checkpoint before calling the slice complete.
## Rust backend R5 setup

The [first setup boundary](docs/plan/21-rust-backend.md#r5-setup-first-partial-boundary)
serves 35 CLI leaves, 27 desktop kinds and the owner-only backend reveal kind
through the shared setup crate.
`commands describe` returns the complete Python catalog.

- [x] Port the four project CLI commands, including selection and encrypted creation.
- [x] Port the five network CLI commands and six desktop network kinds,
  including closure-checked encrypted partition exports.
- [x] Port offline commands and both machine-preference desktop kinds.
- [x] Port backend configuration/secrets commands, seven desktop configuration
  kinds and current local Core credential references. Five explicit backend probes and Core detection use the consent-bound chain
  transport slice; Lightning probing still requires its node-snapshot adapters.
- [x] Port workspace desktop mutations and profile reset/snapshot/display labels.
- [ ] Port AI setup projections and their frozen-book access checks.
- [x] Port owner-only daemon `backends.reveal_token` with its fresh local-auth
  round-trip; the CLI `backends reveal-token` is already native.
- [ ] Run the remaining setup disposition modules and mixed daemon sessions
  outside the sandbox; a focused setup pass is not complete slice acceptance.

## Rust backend R5 chain clients (partial)

- [x] Add consent-bound Esplora/mempool, Electrum TCP/TLS and Core RPC clients,
  plus HTTP, Electrum, Core, BTCPay and local Core-detection probes.
- [ ] Port the Lightning node-snapshot adapters and `ui.backends.lightning.test`.
- [ ] Complete failure-path/TLS/proxy differential fixtures across all backend
  forms, including HTTP probe prefix-only/large-body reads, before declaring
  full chain-client parity.

## Rust backend R5 Bitcoin wallet sync (CLI registered; daemon routes pending)

- [x] Port the compatibility descriptor gap walker for Esplora and Electrum,
  with exact-action consent, cancellation, progress, loopback differential
  fixtures and killed scanning mutations. This helper has no product dispatch.
- [x] Add request-local Bitcoin protocol acquisition: Esplora pagination and
  pre/post stats; Electrum histories, raw transactions, prevouts and headers;
  exact satoshis and height/depth facts. Verify loopback preparation parity and
  acquisition mutations. No observer state, absence authority or persistence.
- [x] Add the inert observer-state/reuse kernel: Python-compatible BDK aggregate
  JSON, identity/version/epoch-bound snapshot reads, exact record fingerprints
  and closed graph/quantity provenance checks. This does not reconstruct or
  execute a BDK wallet or reuse protocol fetches; see the state checkpoint in
  `docs/reference/rust-shell.md`.
- [x] Execute native BDK wallet reconstruction/full/revealed-script sync and
  staged changeset merging using the Python-matching pure state crates. Fetch
  through Kassiber's gated clients; port compatibility checkpoint/fetch reuse.
  This is step 1 preparation, with no durable facts or product registration.
- [x] Prepare canonical BDK reorg/retraction decisions and complete, source-bound
  Esplora absence candidates (step 2), including stale-anchor rebuild, forced
  refresh height fences and point-of-agreement recovery. No durable apply.
- [x] Port pure Bitcoin normalized records and cross-observer composition,
  complete output-snapshot merging, structural provenance mappings and live-
  record retraction vetoes. This is preparation only; no product route or write.
- [x] Publish prepared Bitcoin descriptor-wallet fetches atomically: observer
  state, transaction insertion/merge/retraction, closed provenance, graph index,
  saved-report impacts, output inventory, policy coverage, wallet/freshness and
  journal staleness. Full normalized books match Python for ten fixtures and
  eight restart sequences on both protocols. The supported profile has one
  wallet; captured config, book scope and observer epochs are fenced. Additional
  wallets and unported repairs refuse. Failure injection compares logical rows;
  SQLCipher ciphertext can change after rollback in Python and Rust.
- [x] Assemble complete native prepared fetches, including output/coverage facts
  and named observer routes. Compare native preparation and full-book
  publication against Python on the 102-round fixture/restart/reviewed corpus.
  This is an unregistered preparation seam; see the native fetch handoff.
- [ ] Connect native preparation/publication to shared CLI/daemon wallet and
  freshness services, retaining pre-egress refusal of unsupported shapes.
  The request-bound service and CLI route now exist; the hybrid runner checks
  saved capabilities in an inert admitted-book preflight before consent. Daemon
  freshness execution now has an unregistered admitted-book implementation;
  fetch progress and batch controls now reach the guarded daemon seam. Product
  request lifecycle, graph handoff and default local journal refresh remain pending.
  The unregistered pre-dispatch queue now covers policy, state, recovery,
  single-flight/forced enqueue, cancellation controls, selected dispatch and
  workspace ordering. It grants no transport authority. See the queue checkpoint
  in `docs/reference/rust-shell.md` for the remaining execution and route work.
- [x] Port freshness source specifications and inert whole-request admission
  through `Book`, with foreground exclusion and selection/configuration
  revalidation. The admitted plan requires an explicit journal opt-out and
  refuses enabled unported sources before queue mutation or consent. This is
  an internal prerequisite; live daemon controls and route registration remain open.
- [x] Execute admitted Bitcoin freshness jobs with retries/backoff, SQLite-busy
  rollback, private checkpoints, active-book/workspace aggregation and UI
  redaction. Verify terminal envelopes and complete logical books against
  Python with gated Electrum/Esplora fixtures. Live request progress, controls,
  graph handoff and product route registration remain open.
- [x] Connect native fetch progress and synchronous authority checks through
  discovery, BDK and compatibility acquisition. Match Python's active-book
  prefetch/workspace event ordering; check persisted cancellation, book and
  wallet/backend inputs between batches. The guarded daemon streams these
  events; product lifecycle controls and the graph worker remain open.
- [x] Add the guarded daemon request runner with a dedicated connection,
  early/active cancellation, source controls, offline/revocation/replay fences
  and switch/lock/shutdown/EOF quiescence. Keep product registration gated on
  graph handoff, product validation/selection and full served-shape parity.
- [ ] Redact private checkpoints from Python freshness pause/resume results.
  The disposable `test_reference_source_control_checkpoint_repro_and_native_parity`
  repro pins the disclosure; Rust already applies the shared UI projection.
- [x] Integrate the separate Python tip-cache fix and match it in Rust:
  Esplora/Electrum compatibility evidence refreshes after a changed or missing
  block-identity anchor even when script statistics, status and heights match.
  Retain validated Electrum graphs while refreshing header timestamps; rerun
  fixture, restart-sequence and reviewed-book preparation/publication parity.
- [x] Enable supported CLI `wallets sync` with complete live fixture, restart and
  reviewed-book comparisons on Electrum/Esplora and zero hybrid fallbacks.
- [ ] Enable `ui.wallets.sync`, `ui.freshness.run` and
  `ui.workspace.freshness.run` after queue/state/progress and journal-refresh
  parity. Keep unsupported Liquid, Core RPC, Lightning, Silent Payments and
  additional-wallet overlap on Python in the hybrid runner; direct native
  unsupported calls refuse without retrying.

## Rust backend R5 secrets, backup and diagnostics

The [partial encryption checkpoint](docs/reference/rust-shell.md#encryption-service-checkpoint-r5-partial)
starts from `0d4fbc28`; the complete service slice remains open.

- [x] Port `secrets init`, `init-resume` and `verify`, preserving descriptor/TTY
  input, ownership exclusion, Python container formats and non-migrating reads.
  Verify empty-container publication with both runtimes and kill the named
  secret-handling/publication/rekey mutations.
- [x] Port the five remaining secrets leaves and the existing desktop
  rotation/revocation kinds, including native policy, stale guards and dotenv
  credential migration. See the [continuation](docs/reference/rust-shell.md#secrets-lifecycle-continuation-r5).
- [ ] Run the 29 gated disposable-bus/keyring scenarios and native platform/TTY
  acceptance; finish secrets dispositions and complete refusal/error parity.
- [ ] Port age `.kassiber` export/import and desktop preview/apply/cancel with
  archive safety mutations, both-runtime bundle round trips and publication
  interruption recovery. Export and staging-only import have a
  [partial checkpoint](docs/reference/rust-shell.md#backup-archive-checkpoint-r5-partial);
  [installation and shared restart recovery](docs/reference/rust-shell.md#backup-install-and-recovery-continuation-r5)
  are now ported. All four desktop kinds have a
  [native continuation](docs/reference/rust-shell.md#desktop-backup-continuation-r5),
  including scoped previews, cancellation and locked safe restore. Compressed
  archives, hardlink export and remaining archive/platform parity remain open.
- [ ] Consider attachment content hashes in a future backup format/product
  change. The current Python-compatible manifest records only file counts;
  this Rust port deliberately retains that behavior.
- [x] Port `diagnostics collect` and global `--diagnostics-out`, with ordered
  redaction fixtures, native runtime/stack provenance exceptions and planted
  privacy mutations. No diagnostics daemon kind exists in the reference.
- [ ] Port `maintenance purge-legacy-ledger` and the remaining desktop kinds;
  preserve RAM-only logging.
- [ ] Complete all slice dispositions and native-platform/TTY gates before
  accepting the slice.

## Rust backend R5 transaction evidence

The [partial local evidence boundary](docs/reference/rust-transactions-r5-status.md)
ports managed attachments and loan marks through both transports, plus 22 local
metadata CLI leaves and the desktop stale/history/revert kinds. Four BIP329
leaves and their three desktop kinds now cover local address/inventory matching,
label merges, pagination and all export modes; descriptor and complex input-graph
matching still refuse explicitly. Record pagination and append-only history
reverts and the grouped desktop metadata-update kind are implemented, including
exact pricing, review/tax validation and atomic audited edits. `transactions list`
now preserves observed CLI pages, scoped filters, sort/cursor bytes and unpaged
counts. Desktop list/search/extremes now preserve grouped custody rows, queue
status, signed display sorts, exact bounds and cursor pages; the five other
transaction desktop projections and exports remain unported. Documents now cover
registration, transactionless evidence and prior-report archives, with stored
commercial matching/context reads. AI
projections stay Python. Progress and remaining work for this slice:

- [x] Correct the Python reference's lexical fractional timestamp bounds in
  a separate regression/fix commit, then update Rust transaction/record/history
  filters. Exact microsecond bounds have regression, parity and mutation evidence
  in the acceptance record.
- [x] Port desktop transaction list/search/extremes over stored journal/custody
  state, with raw JSONL parity and killed filter/order/count/page mutations.
- [x] Port commercial documents through the CLI and daemon, with stored
  matching, context reads, parity and mutation evidence in the acceptance record.
- [ ] Port desktop dashboard/resolve/graph/review-context/template projections.
  Preserve the exact
  dependencies and remaining editor-smoke boundary in the acceptance record.
- [ ] Complete descriptor/input-graph BIP329 matching and standard-field
  surrogate/platform parity with the wallet lifecycle work.
- [ ] Complete the five loan tax-composition dispositions with the journal
  slice; hybrid scenarios currently verify Python consumes Rust-authored marks.
- [ ] Verify MIME inference against Windows registry overrides and native
  Windows/macOS attachment storage behavior.

## Rust backend R5 wallets and imports

- [x] Port birthdays, watch-only Silent Payments config and desktop wallet listing;
  add successful-create wire/storage comparison, broader malformed-config vectors,
  native lifecycle disposition replacements and computation mutations. See
  [the checkpoint](docs/reference/rust-shell.md#wallet-lifecycle-remainder-checkpoint-r5).

- [x] Port import-run listing, rollback planning and confirmed deletion through
  the real CLI/daemon; retain profile/run selection, collateral disclosure,
  journal invalidation and SQLCipher compatibility. Port the rollback-scale
  tests and kill the selection/cascade/invalidation mutations.
- [x] Freeze synthetic Bitcoin/Liquid descriptor and wallet-material vectors;
  port local `wallets derive` and desktop descriptor preview, with native
  computation mutations. See the partial R5 descriptor checkpoint in plan 21.
- [x] Port generic CSV and Phoenix file imports with exact msat/time/deduplication,
  import-run/rollback and journal invalidation parity, reference book dumps and
  named mutation killers. See the file-ingestion checkpoint in plan 21.
- [x] Port generic JSON imports with CPython parsing/coercion, complete book-dump
  parity, SQLCipher refusals and named mutation killers. See the partial exchange
  importer checkpoint in plan 21.
- [x] Port River CSV imports with ordered header aliases, exact cash/BTC fee
  mapping, full book-dump/rollback parity and named mutation killers. See the
  batch-1 continuation in plan 21.
- [x] Port Bull/Bullbitcoin order files with profile-wide commercial matching,
  excluded full-mode reconciliation, complete book-dump/rollback parity and
  named mutation killers. See the Bull continuation in plan 21.
- [x] Port Coinfinity order files with reference-first/economic matching,
  excluded full-mode reconciliation, complete book-dump/rollback parity and
  named mutation killers. See the Coinfinity continuation in plan 21.
- [x] Port 21bitcoin active custodial ledger files with exact provider identities,
  legacy-flag cleanup, complete book-dump/rollback parity and named mutation
  killers. See the 21bitcoin continuation in plan 21.
- [x] Port Pocket/Pocketbitcoin evidence files and both CLI aliases with exact
  withdrawal pairing, complete book-dump/rollback parity and named mutation
  killers. See the Pocket continuation in plan 21.
- [x] Port Strike active custodial history with separate Lightning/chain
  identities, complete book-dump/rollback parity and named mutation killers.
  Complete exchange-file batch 1; see the final checkpoint in plan 21.
- [ ] Port the remaining wallet commands after file-ingestion acceptance,
  including individual file importers with exact msat/time/deduplication
  parity and mutation entries.
  Complete malformed descriptor/preflight error parity beyond the frozen cases.
  Keep unsupported commands disabled.
- [ ] Complete wallet/import boundary and subprocess dispositions through the
  hybrid candidate with native served-by evidence; port remaining rust-test
  dispositions. Network sync stays in the later chain/service slice.

## Rust backend R5 reports

The [report-read boundaries](docs/plan/21-rust-backend.md#r5-reports-first-read-boundary-partial)
read Python-produced journals through ten CLI leaves and five daemon kinds.
The snapshot-write continuation adds the explicit saved/filed marker CLI leaf.

- [x] Port live summary, gains, balance sheet, wallet holdings and saved/filed
  snapshot listing, retaining readiness gates and exact JSON/table/plain output.
- [x] Port tax summary and Austrian E1kv/tax-summary reads, including
  Vienna-year boundaries, cent rounding, Kennzahlen, Alt/Neu sections and
  quarantine disclosure; port the desktop tax-summary kind.
- [x] Port the complete stored journal-entry listing with signed msat and
  deterministic split-row ordering.
- [x] Port CSV projections for the ten report reads and atomic `--output`
  publication, with FIFO/HIFO/Austrian byte parity and killed mutations.
- [ ] Port exit-tax reads and the associated amendment disclosures.
- [x] Port historical balance/holdings reads and the desktop balance-history
  kind with native parity and killed mutations.
- [x] Reduce Rust read startup and report scaling costs without changing schema,
  admission checks or output bytes; see the
  [read-path measurements](docs/reference/rust-read-performance.md).
- [ ] Port individual snapshot lookup and amendment-review state when process
  callers exist and the stack includes phase 5; preserve append-only history.
- [ ] Finish the remaining report read inventory and desktop kinds, including
  explanation, privacy, Lightning and cross-check reads.
  The reviewed commercial subledger CLI read and its CSV export now run
  natively, with reference vectors, killed mutations and hybrid routing proof.
- [x] Port full-report, Austrian bundle and transaction CSV artifacts, saved
  export registration and their daemon kinds. Preserve
  sealed summaries, hashes and phase-4 append-only impact history.
  Completion computes hashes, exact summaries and authoritative dependencies
  under retained admission. Complete normalized DB sequences and native
  CLI/daemon process oracles prove byte parity and zero CSV fallbacks, including
  desktop capital-gains CSV. Graphless Liquid dependency capture now matches
  Python's local parse-success boundary, with stored identity and ambiguity
  guards intact. Platform transaction
  formats remain separate work; see the
  [CSV transport checkpoint](docs/reference/rust-shell.md#csv-export-transport-checkpoint-r5-tenth-continuation).
- [x] Add the isolated deterministic `rust_xlsxwriter` renderer and strict
  openpyxl content oracle for all report workbook shapes; port transaction-list
  XLSX projection, publication and CLI/daemon registration with zero fallbacks.
- [x] Port Austrian E1kv XLSX projection and CLI/desktop registration through
  native completion; compare workbook content, actual hashes, explicit filing
  and every database table, with zero hybrid fallbacks.
- [ ] Port full-report (including verification), exit-tax,
  year-handoff and cross-check XLSX projections and registration. Verify their
  saved completions, Rust-byte hashes, explicit filing, complete normalized DB
  sequences and native routing before marking XLSX complete. PDF stays Python.
- [x] Port explicit saved/filed markers and filing a captured local export,
  preserving sealed identity, frozen dependency copies and existing impact history.
- [x] Keep PDF rendering and PDF-containing source-funds bundles in Python by
  the owner's decision. Keep unsupported commands disabled and AI projections
  in Python.

## Rust backend R4

The [R4 journal pipeline design](docs/reference/r4-journal-pipeline.md) holds
the measured phase profile, the typed architecture, the performance budget
and the owner decisions it needs.

The separate [tax-engine speed pass](docs/reference/tax-engine.md#ledger-performance-checkpoint-r4-2026-10-30)
records the byte-parity checkpoint, phase timings and mutation equivalences.

- [ ] Get the owner's remaining decisions on book-open cost and the allocator;
  decisions 1–4 are recorded in the design.
- [x] R4-0: integrate custody, projection fixes, performance, fusion and the
  service/reference fixes on the R5 stack; see the
  [R6 integration audit](docs/reference/rust-backend-r6-integration.md) for the
  linear replay and completed custody, parity and mutation gates. Round 4 adds
  the R5/R4 continuations in the
  [R7 integration audit](docs/reference/rust-backend-r7-integration.md).
- [x] R4-1: commit the Rust end-to-end bench, bench-book generators and the
  regression record; record the baseline. The harness, generators and record
  format and four recovered baseline records are integrated.
- [x] Establish compact retained-calculation storage in Python with the Rust
  schema mirror, report/AI parity, coexistence and backup/restore evidence; see
  [the storage measurements](docs/reference/r4-journal-pipeline.md#compact-storage-implementation-measurements).
- [x] Establish deterministic journal ids and changed-row writes in Python as
  the R4-12 persistence reference.
- [x] Scope the 128 MiB journal rebuild cache in Python as the R4-12 reference.
- [ ] R4-2 to R4-8: typed rows, custody stages and fusion, one step per run,
  each with the full mutation catalog rerun and before/after timings.
  R4-2's first prototype stopped at 0.301 s against its 0.250 s target;
  the continuation adds typed rows, book-local identifiers and typed network
  observation admission, with an explicit timed boundary to unported stages.
  This is a partial R4-2 checkpoint; R4-3 and the remaining acceptance gates
  remain open. See the [checkpoint](docs/reference/r4-journal-pipeline.md#typed-row-and-admission-checkpoint-r4-2-partial).
  Complete typed components/evidence and measure the remaining conversion cost
  before accepting R4-2/R4-3
  ([earlier probe](docs/reference/r4-journal-pipeline.md#recovered-baseline-and-r4-2-target-probe)).
  R4-4's first typed compilation/output probe stopped at 0.389 s against
  its 0.153 s generic target
  ([probe](docs/reference/r4-journal-pipeline.md#r4-4-typed-compilation-probe-stopped)).
  The subsequent R4-4/R4-5 kernel pass retains existing stage representations
  and meets both CPU targets: interpretation 3.05–4.12× and gaps 2.02–2.16×
  faster across generic/multi-wallet 2k/20k. See the
  [kernel checkpoint](docs/reference/custody-engine.md#interpretation-and-gap-kernel-checkpoint-r4)
  for parity and touched-file mutation scope. Overall custody and the
  R4-2–R4-8 integration budget remain open.
  R4-6's partial quantity probe also stopped for re-planning: compute missed
  both targets and boundary conversion grew; no implementation was accepted
  ([measurements](docs/reference/r4-journal-pipeline.md#r4-6-partial-quantity-target-probe)).
  Its [quantity kernel follow-up](docs/reference/r4-quantity-kernel-performance.md)
  retains the existing stage boundary and reaches quarter-time process CPU
  including output construction (4.3–4.8× faster); the earlier wall budget
  and the complete typed pipeline remain open.
  R4-7's finalized-output prototype also stopped: compute was 0.525 s generic
  and 1.135 s multi-wallet against 0.263/0.290 s limits; the typed rewrite is
  withdrawn pending [re-planning](docs/reference/r4-journal-pipeline.md#r4-7-finalized-projection-target-probe).
  A [projection/fusion kernel checkpoint](docs/reference/r4-journal-pipeline.md#projection-and-fusion-kernel-checkpoint)
  keeps stage models unchanged and reduces CPU, but the projection/request
  budgets and the no-engine-slowdown gate remain open.
  The [native conversion checkpoint](docs/reference/r4-journal-process-checkpoint.md)
  adds `ledger_typed`, passes its response directly to rendering and borrows
  renderer operands. Replace fusion's request value/decoder and the quantity
  and finalized `stage_value` adapters before claiming the typed R4-8 handoff.
- [x] R4-9 native loader: **fully accepted, with no pending exceptions**.
  Python/native mixed revision references now share one typed validation error.
  The fresh 22,528-case composed and 16,384-case non-BLOB audits match exactly;
  all forty former message exceptions are stable across eight hash seeds.
  All 357 loader recipes are killed; the inherited 2,292-entry custody campaign
  remains complete. Fresh 24,000 books/1,030 permutations, 159 retained replays
  and permutations, focused census/shadow and static gates pass. Python retains
  production authority ([acceptance and provenance](docs/reference/r4-native-loader-checkpoint.md#deterministic-revision-reference-acceptance)).
  Evidence inherited from t3code/r4-9-loader-determinism@aaf6216e3; pending fresh r9 verification.
- [ ] R4-10 to R4-12: pre-build repairs, rendering and joins, persistence,
  each with byte or database-dump parity.
- [ ] Finish the [R4-10 auto-pricing checkpoint](docs/reference/r4-journal-pipeline.md#r4-10-auto-pricing-checkpoint):
  complete generated/recorded book and legacy/refusal coverage, mutation and
  CPU acceptance.
  Cached pricing, source-overlap repair/warnings, and drifted legacy-pair refresh
  with profile reload have unregistered native interfaces and complete-dump
  oracles. The combined stage and split repair/price interface for R4-13 retain
  whole-pipeline rollback after later build/persistence failures.
  The [scenario replay checkpoint](docs/reference/r4-journal-pipeline.md#r4-10-scenario-replay-checkpoint)
  freezes 505 stored scenarios from the twelve generators and passes 1,010
  plaintext/SQLCipher file comparisons; ten schema refusals are tracked
  separately. The exhaustive generated campaign is still incomplete.
- [ ] Reject fractional legacy msat before Python's source-overlap fingerprint
  conversion can collapse distinct amounts. The disposable repro is
  `python -m tests.rust_port.repro_journal_fractional_source_overlap`;
  the native checkpoint preserves reference behavior pending that correction.
- [ ] Fix Python's lineage-exists refusal on repeated evidence-only legacy
  pair drift when sorted parent selection supersedes the current revision.
  Reproduce with `uv run --locked python -m tests.rust_port.repro_journal_repeated_legacy_drift`;
  native full-dump vectors preserve the reference refusal pending correction.
- [ ] Finish the [R4-12 persistence checkpoint](docs/reference/r4-journal-persistence-checkpoint.md):
  close the CPU write-floor budget, exhaustive persistence mutations and inherited
  campaign gates before wiring production in R4-13. The unregistered service,
  complete dump oracle and caller-owned storage scopes are implemented.
  Borrowed bindings, shared source encodings and cached posting metadata avoid
  repeated work; final 20k CPU passes one of eight required fresh/unchanged
  cells. Unchanged builds still need a cheap comparison path. Corrected appended
  benchmark inputs carry distinct raw identities and assert actual writes.
- [ ] Finish the [R4-11 rendering checkpoint](docs/reference/r4-journal-render-checkpoint.md):
  close legacy-value/refusal gaps and complete exhaustive branch mutations
  and the inherited campaign gates. The [performance follow-up](docs/reference/r4-journal-render-performance-checkpoint.md)
  meets the historical wall-clock targets. The [boundary continuation](docs/reference/r4-journal-render-boundary-checkpoint.md)
  repairs integer refusals, holding keys and nonfinite detail/summary values;
  actual surrogate scalars, the 0.15 s CPU budget and the remaining acceptance
  gates are open.
  The [allocation continuation](docs/reference/r4-journal-render-boundary-checkpoint.md#fifth-continuation-native-fragment-quantities-and-allocation-checkpoint)
  preserves all 379 prior records and expands the corpus to 383 cases. Eleven
  focused mutations are killed; the full campaigns remain open. Five alternating
  pinned pairs give 0.151179 s generic and 0.179153 s multi-wallet CPU at 20k;
  both still miss 0.15 s, and checkpoint comparisons do not isolate the regression.
  Python retains production authority.
- [x] R4-14: expose the native automatic-pairing journal stage by reusing
  transfer suggestions, rules and bulk exact pairing. Pin repeated-pass and
  caller-transaction database parity on SQLite and SQLCipher, hash-seed sweeps,
  generator/recorded books and named mutations. The [CPU acceptance](docs/reference/rust-pairing.md#cpu-acceptance-2026-10-31)
  reaches 13.02× / 11.75× Python at 20k. Journal routes remain Python.
- [ ] R4-13, R4-15 and R4-16: CLI and daemon journal commands, integration of
  automatic pairing, the freshness journal job and the authority switch.
  R4-13's [test-only composition](docs/reference/r4-journal-process-checkpoint.md)
  wires native load through persistence, ordered progress and outer rollback.
  The continuation imports cached pricing, R4-14 pairing and renderer boundary
  hardening; it adds real-pricing and abrupt-death recovery probes. Production
  resolution and both repair stages are still missing from the committed handoff.
  Integrate real R4-10 pre-build, close full-pipeline parity/mutation and speed
  gates, then register CLI and daemon in separate verified commits. Reconcile
  the desktop daemon's pairing default with R4-14 before changing its route;
  prepared-book probe results do not authorize production routing.

## Rust backend R5 transfers and review

The [partial checkpoint](docs/reference/rust-shell.md#transfers-and-review-classification-r5)
serves all five stored reads, fourteen authoring commands (rule CRUD/toggles,
dismissals, pair/unpair/update, payout create/delete, component and gap plan/apply),
suggestions/bulk pairing/rule application, component list/show, and gap list/review:
26/29 CLI leaves and 25 desktop kinds. Full
normalized authoring row parity is pinned for that subset; AI projections remain
Python.

- [x] Port pair/unpair/update using the shared custody stage, including native
  residual/conflict validation, append-only revisions, immutable rows, Python
  commitments and atomic journal invalidation. Pin 115 operations against Python.
- [x] Port payout deletion with complete-row parity, atomic rollback, desktop
  attribution and a named mutation catalog.
- [x] Port payout creation with fixed-source complete-row/refusal parity, atomic
  invalidation, native desktop dispatch and named mutation acceptance.
- [x] Port component plan/apply (create, revise, undo, activate, supersede) with
  version-bound apply, conservation/route barriers, complete normalized row
  parity after mixed authoring/revision sequences and named mutations.
- [x] Port gap plan/apply and desktop kinds with version-bound candidate
  fingerprints, append-only corrections and filed-report impacts. Reuse the
  pure R2 gap search; pin complete rows, late-failure rollback and mutations.
- [x] Register suggestions, bulk pairing and rule application through the shared
  R4 pairing library, with command-level full-row parity, atomic dry runs,
  daemon coercion/authorship, hybrid process witnesses and named mutations.
- [x] Port component list/show with the shared materializer, private CLI opt-in,
  safe desktop projection, exact filters/pagination and read-only DB parity.
- [x] Port gaps list/review with their desktop kinds, version-bound cursor pages,
  canonical freshness/blockers, historical reviews and unchanged database parity.
- [ ] Port review cases/plan/apply through the shared journal service: r7 has
  pure custody/fusion and a render checkpoint, but still needs the native SQL
  snapshot/ownership loader, complete build-state assembly and journal writer.
  Retain readiness barriers rather than reading stale journals; see the
  [review adapter handoff](docs/reference/rust-shell.md#review-integration-handoff).
  The [R4-14 pairing library](docs/reference/rust-pairing.md) now supplies the
  matcher/rules/bulk implementation and its oracle/mutation evidence; R4-13 and
  R4-15 still need to integrate its atomic journal boundary for journal processing.

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
- [ ] Stop an Austrian Neu shortfall from aborting the whole report. The
  engine's regime tracker (`tax-engine/core/src/ledger/regimes.rs`) tags a
  sale Neu whenever the wallet holds any Neu, even when the sale needs more,
  and the engine has no Alt fallback for explicit regimes (for example 1 BTC
  Alt plus 0.1 BTC Neu, then a 0.5 BTC sale). Decide between splitting the
  disposal across regimes and capping the Neu tag with the Wahlrecht ordering
  entry above; done when that book processes with test-pinned results and a
  stale migration. Pricing can release such a disposal, so pricing an unpriced
  row can turn a processed book into this abort (goldens
  `at-pricing-releases-a-neu-shortfall-*`, an explicit exclusion in
  `tax-engine/core/tests/ledger_properties.rs`); the fix must remove that
  exclusion too.
- [ ] Decide the Austrian treatment of a Neu swap's fee basis. The pool drops by
  amount plus fee, but only the amount's basis is carried, so the fee's share
  is neither realized nor carried. Needs legal review before any change. Done
  when the reviewed treatment is recorded in
  [the Austrian handoff](docs/austrian-handoff.md) and an engine fixture pins
  it, whether the review keeps today's behavior or changes it (a change also
  needs a stale migration).
- [ ] Stop parsing user description text as Austrian notes markers, as plan 20
  phase 5's first change. The engine composes each event's and move's notes
  from typed facts and appends the row's note, description, or kind, and its
  Austrian rules read markers from the whole text
  (`tax-engine/core/src/ledger/describe.rs`), so a user's `at_regime=`,
  `at_pool=`, or `at_swap_link=` token can add, duplicate, or conflict with
  the engine's markers. Paired user `at_swap_link=` tags on a Neu sale and a
  receipt of another asset give a swap carry under `moving_average_at`, and a
  consulted duplicate or conflicting marker aborts the report (goldens
  `at-usertag-*`). Done when the rules read typed markers built from the
  engine's own facts, and a stale migration marks Austrian profiles stale
  whose stored transaction note, description, or kind contains one of the
  three tokens. Importers write all three columns, and plain CSV and JSON
  imports copy `kind` verbatim. A move whose outgoing row has no text gets its
  wallets' labels as notes, but every token contains `=`, which the label rule
  below rejects.
- [ ] Keep transfer-leg quantities exact in the transfer audit. The engine
  returns each move's sent, received, and fee amounts exactly, but the reader
  (`kassiber/core/engines/ledger_response.py`) still passes them through
  `float` before `dec()`, as the phase-3 adapter did, which loses msat above
  about 15 significant digits. Exact values change stored `quantity_exact`
  text on most books (`1.0` becomes `1`), so done when the reader keeps them
  and a stale migration marks processed books stale.
- [ ] Reconcile [plan 06](docs/plan/06-austrian-tax-engine.md)'s "Neuvermögen
  from 2023 uses moving average" with the code, which applies the Neu moving
  average to every Neu disposal from 2021-03-01. Confirm against the
  KryptowährungsVO and fix whichever is wrong.
- [ ] Settle the generic Bitcoin rail carry in one pass (plan 20 phase 5). The
  engine keeps the phase-3 fixed-point loop
  (`tax-engine/core/src/ledger/carry.rs`), which reprices BTC↔LBTC
  carrying-value pairs over repeated whole-book passes. Plan 20's
  wait-for-carry rule changes three shapes the goldens pin: a pair blocked in
  an early pass stays blocked although a later pass prices its source
  (`carry-d1-unpriced-peg-in`); a same-instant round trip under moving average
  runs out of passes and keeps a gain on a carrying-value leg
  (`carry-d2-round-trip-moving-average`); and one under FIFO that converges
  today becomes a carry cycle (`carry-d3-round-trip-fifo`). The carry unit
  tests `a_source_short_of_lots_blocks_its_pair_for_good` and
  `carries_that_do_not_settle_keep_the_last_passs_pricing` pin the loop too.
  Done when the rule replaces the loop with a reviewed policy for same-instant
  cycles (error, quarantine, or iterate), the three shapes are listed
  differences, the rail property's exclusion for unsettled carries in
  `tax-engine/core/tests/ledger_properties.rs` is gone, and a stale migration
  marks the books it changes.
- [ ] Choose Austrian swap links on the final pass. The engine selects links
  on the first prepare pass and does not check them again on the second
  (`tax-engine/core/src/ledger/swaplinks.rs`), as the phase-3 adapter did. A
  chained swap whose first hop is blocked and a link whose out leg the second
  pass makes Alt both leave an `at_swap_link=` marker unpaired, and the whole
  report aborts with `RP2 cross-asset input validation failed: Unpaired ...`
  (goldens `at-chained-swap-unpaired-abort`, `at-swap-link-turns-alt-abort`).
  A linked swap-out also skips the balance gates, so it can overdraw an
  account the first pass funded from a row the second pass leaves out: within
  the lot engine's tolerance a later sale or move of that asset is
  quarantined as `missing_cost_basis` (goldens `at-pass-b-overdraw-*`), and
  beyond it the report aborts. Two same-pool swaps that depend on each other
  abort too. Done when an unresolvable link quarantines its pair instead of
  aborting, with goldens, the swap properties' shared-failure exclusions in
  `ledger_properties.rs` narrowed to match, and a stale migration for the
  books it changes.
- [ ] Rename the RP2 texts of engine failures. They keep the phase-3 adapter's
  wording byte for byte: the `RP2 cross-asset input validation failed:`,
  `RP2 multi-asset tax calculation failed:`, and `RP2 tax calculation failed
  for asset ...` wrappers (`kassiber/core/engines/errors.py`), the method
  gate's `Unsupported RP2 accounting method ...`
  (`tax-engine/core/src/ledger/run.rs`), the `RP2 configuration ...` checks
  (`ledger_request.py`), the raw-row `TypeError` naming
  `GenericRP2TaxEngine`, and the `rp2_input_validation` error code. Done when
  they name Kassiber's engine, the comparison maps old texts to new ones as a
  listed difference while RP2 remains the oracle, and the docs and tests that
  quote them follow. Messages the lot engine reproduces from RP2 itself (such
  as the unpaired-marker text) stay as the RP2 fixtures pin them.
- [ ] Relax the configuration label rules RP2's INI file imposed. The request
  builder keeps them with RP2's messages
  (`kassiber/core/engines/ledger_request.py`): a wallet, profile, or asset
  label containing `, \n \r [ ] = : # ;` fails the whole report, even for an
  unused wallet, and a `%` the INI interpolation cannot read raises
  `InterpolationSyntaxError` (goldens `error-forbidden-character`,
  `error-holder-forbidden-character`, `error-percent`). The engine reads no
  file, so done when the builder accepts every label the engine can tell
  apart, with goldens. Land it after typed-only markers (above): with `=`
  allowed, a wallet label can carry a marker into a move's notes
  (`tax-engine/core/src/ledger/normalize.rs`).
- [ ] Fix the whitespace artifacts of tax-processing labels. The
  configuration checks strip wallet and profile labels, but entries carry
  them unstripped: a label with surrounding whitespace fails the report with
  RP2's `Parameter 'exchange' value is not known` (or `'holder'`), and two
  wallet labels that differ only by surrounding whitespace fail as duplicates
  (golden `error-duplicate-after-strip`). Done when processing strips labels
  consistently or keeps them exact, with goldens.
- [ ] Record the engine and country-rules versions of each journal run (plan
  20, "Provenance"). The engine reports its version
  (`kassiber_tax.engine_version()` and every `ledger` response), but journals
  store neither it nor a country-rules version, which does not exist yet;
  `calculation` records only `"engine": "kassiber_tax"`. Adding them to
  `calculation_json` changes persisted bytes, so decide between a run-metadata
  table and adding them to `calculation` with the first phase-5 stale
  migration. Done when processed journals carry both versions and filed
  reports keep the versions that produced them.
- [ ] Decide with phase 5's first change whether RP2 stays a development
  dependency and the `tax-shadow` CI lane keeps running. Both compare the
  engine with the frozen phase-3 oracle (`tests/tax_oracle/`). Once phase 5
  changes results on purpose, either each change lists its differences
  against the oracle, or the stored goldens (`ledger_fixtures.json`) carry the
  proof alone and RP2, the oracle, and the lane retire together.
- [ ] Add authoritative Austrian tax fixtures (plan 20 Verification 3): BMF
  worked examples and Steuerberater-reviewed scenarios, starting with plan
  16's salary → savings-wallet case. Needs reviewed sources from the owner;
  done when each fixture cites its source and passes on the engine.

## Quarantine follow-ups

- [ ] Cover every synthetic projection-row prefix in the projection-id guard
  test. Plan 20 phase 4 deleted the pre-custody normalizer branches no
  finalized row can reach from the product (they remain only in the frozen
  oracle, `tests/tax_oracle/`), and the request builder refuses their shapes
  (`non_synthetic_row_id`, `shared_pair_leg`). The guard
  (`test_every_projected_row_id_is_synthetic_and_pairs_are_edge_disjoint`)
  compiles one fixture, whose rows are all `custody-tax:`. Done when the
  projection-id guard test covers every synthetic prefix in
  `transfers._SYNTHETIC_TRANSFER_ID_PREFIXES` and the suite stays green.
- [ ] Keep pricing from adding quarantines. A quarantined sale or move uses
  none of its wallet's coins, so pricing an earlier purchase can let it book,
  and a later move or sale then runs short. The short spend can also be a
  later Bitcoin rail pair's source, which the generic carry then blocks
  (`bitcoin_rail_carry_basis_unresolved` on both legs). Plan 20 requires that
  pricing an unknown lot never adds a quarantine; the engine keeps today's
  behavior (goldens `generic-pricing-releases-a-sale-*` and
  `carry-pricing-releases-a-sale-*`, and an explicit exclusion in
  `tax-engine/core/tests/ledger_properties.rs`). Done when the phase-5
  inventory change removes the exclusion, with a stale migration for the books
  it changes.
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
- [ ] Surface already-reviewed same-asset pairs whose legs carry different
  observed txids (the matcher no longer suggests them, but pairs authored
  before the fix stay active) for the owner to look at. Different txids are a
  hint, not proof: a hop through an untracked wallet also has two, so
  unpairing stays the owner's choice per pair. Done when such a pair is listed
  on its own with that hint, not only as evidence on a suspense.
- [ ] Decide whether receipts behind a custody basis barrier stay held: their
  own basis is their market value, but the pool timeline is cut at the
  barrier. Done when the policy is written down in the tax reference and the
  barrier follows it.
- [ ] Show journal staleness and quarantine once in the shell: the bell keeps
  standing "needs processing"/"quarantined" items that repeat the side-nav
  hints, staleness has several verbs ("Process", "Reprocess", "Recalculate",
  "Process ledger"), and a book with any quarantine shows the first-sync card
  on every refresh. Done when the bell carries events only and one verb is
  used everywhere.

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
- [ ] Offer a reasoning effort for HTTP models whose listing says nothing
  about it, such as OpenAI's reasoning models: the desktop offers none for
  them today. Learn it from something other than a guess by name (a
  connection-test probe, say); a refused effort is already retried without it.
  Contract: [the AI reference](docs/reference/ai.md#reasoning-effort-over-http).
- [ ] Ask OpenAI-compatible endpoints for reasoning summaries again once
  Kassiber can tell which models and organizations allow them (a model listing
  field or a connection-test probe); the Responses request leaves `summary`
  out because an unverified organization's refusal failed the turn.
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
