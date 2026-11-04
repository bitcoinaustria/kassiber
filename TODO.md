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

## Rust backend R5 desktop projections

The [desktop projection checkpoint](docs/reference/rust-shell.md#desktop-projections-r5-continuation)
serves **10/15** requested unassigned desktop kinds natively, plus Python's
report-read journal rebuild for every report-like read. Remaining:

- [ ] Port `ui.transactions.graph` (local graph, owned index, consented public
  lookups through `kassiber-net`), then `ui.transactions.review_context`, which
  embeds the graph and evidence summary.
- [ ] Port `ui.audit.evidence.summary` (no PDF package export).
- [ ] Port `ui.report.blockers` once the workspace-health snapshot is native
  (workspace-shell slice); keep the sync-failure blocker.
- [ ] Port `ui.review.request_input` once the native review loader is accepted
  for production routes.
- [ ] Serve report-read wallet sync natively; the prelude now refuses with
  `not_implemented` when a book enables it.
- [ ] Add named Rust mutation kills for the new projections; current evidence
  is the Python-oracle parity suites listed in the checkpoint.

## Rust backend R5 desktop settings and sync status

The [desktop settings checkpoint](docs/reference/rust-shell.md#desktop-settings-and-sync-status-r5)
serves 7 of the 8 requested desktop request kinds natively; the two progress
events were already native.

- [x] Freshness: `ui.freshness.status` and `ui.freshness.configure`.
- [x] Maintenance: `ui.maintenance.settings` and `ui.maintenance.configure`.
- [ ] Maintenance: `ui.maintenance.run`. It needs the native report-blocker
  snapshot (`ui.report.blockers`, which needs workspace health), the report-read
  journal prelude from the desktop-projection slice, and native report-read
  wallet sync for `sync=always|if_enabled`. Until then it answers
  `not_implemented`.
- [x] Backends: `ui.backends.list` and `ui.backends.lightning.test`.
- [x] Wallet sync status: `ui.wallets.utxos`; `ui.wallets.sync.progress` and
  `ui.freshness.run.progress` events stay covered by the freshness product suite.

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
- [x] Port `ui.backends.lightning.test` over the native LND/CLN report clients,
  which now keep the node alias, network, height and peer count.
- [ ] Port the remaining Lightning node-snapshot routes (`ui.connections.node.snapshot`
  and the AI node projections) over the same clients.
- [ ] Complete failure-path/TLS/proxy differential fixtures across all backend
  forms, including HTTP probe prefix-only/large-body reads, before declaring
  full chain-client parity.

## Rust backend R5 Bitcoin wallet sync (CLI and no-refresh daemon registered)

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
  This preparation seam is shared by the registered CLI and no-refresh daemon
  services; see the native fetch handoff.
- [x] Connect native preparation/publication to shared CLI/daemon wallet and
  freshness services, retaining pre-egress refusal of unsupported shapes.
  The request-bound service and CLI route now exist; the hybrid runner checks
  saved capabilities in an inert admitted-book preflight before consent. The
  three daemon routes now serve admitted explicit no-refresh shapes, with
  product progress, controls, automatic graph follow-up and complete-book parity.
  Default refresh remains on Python pending native R4-13 journals.
  The shared pre-dispatch queue covers policy, state, recovery,
  single-flight/forced enqueue, cancellation controls, selected dispatch and
  workspace ordering. It grants no transport authority. See the queue checkpoint
  in `docs/reference/rust-shell.md` for the remaining default-refresh integration.
- [x] Port freshness source specifications and inert whole-request admission
  through `Book`, with foreground exclusion and selection/configuration
  revalidation. The admitted plan requires an explicit journal opt-out and
  refuses enabled unported sources before queue mutation or consent. This is
  shared by the registered no-refresh product routes and their live controls.
- [x] Execute admitted Bitcoin freshness jobs with retries/backoff, SQLite-busy
  rollback, private checkpoints, active-book/workspace aggregation and UI
  redaction. Verify terminal envelopes and complete logical books against
  Python with gated Electrum/Esplora fixtures. Product routes now use the same
  request progress, controls and graph handoff.
- [x] Connect native fetch progress and synchronous authority checks through
  discovery, BDK and compatibility acquisition. Match Python's active-book
  prefetch/workspace event ordering; check persisted cancellation, book and
  wallet/backend inputs between batches. Registered no-refresh product routes
  stream these events and arm graph follow-up after terminal flush.
- [x] Add the guarded daemon request runner with a dedicated connection,
  early/active cancellation, source controls, offline/revocation/replay fences
  and switch/lock/shutdown/EOF quiescence. Product dispatch retains whole-shape
  admission and the same lifecycle barriers.
- [x] Carry private successful-job graph grants through the guarded runner and
  publish them into a bounded mailbox after the terminal response is flushed.
  Preserve request cancellation, configuration checks and lifecycle discard.
- [x] Add the guarded connection-owning, stoppable graph worker and bounded native
  reference lookup, with grant revalidation, SQLCipher cleanup, unsolicited
  events and graph/lifecycle parity and mutation acceptance.
  Admitted product requests now arm it automatically after terminal flush.
- [x] Integrate automatic graph-worker startup with the native product
  request lifecycle, retaining the flushed-response barrier, source controls and
  complete product graph/event/restart/reviewed-book acceptance. Automatic
  arming and both source-control boundaries are verified through the guarded
  seam and registered no-refresh product acceptance.
- [x] Validate product argument shapes and protocol request IDs through the
  guarded adapter, with inert admitted-book selection for explicit no-refresh
  variants. Selection never opens or migrates a database and refuses defaults,
  unsupported sources and wallet sync's distinct direct-sync paths.
- [x] Register no-refresh shapes passing full envelope/event/table parity with
  zero served-shape fallbacks across the fixture/restart/reviewed-book corpus.
  Per-route native served-by counts are recorded in the latest Rust checkpoint.
- [ ] Reject mismatched Electrum raw-transaction IDs in Python graph lookup.
  `test_reference_electrum_mismatched_transaction_repro_and_native_refusal`
  demonstrates the reference bug; native decoding already refuses the response.
- [ ] Redact private checkpoints from Python freshness pause/resume results.
  The disposable `test_reference_source_control_checkpoint_repro_and_native_parity`
  repro pins the disclosure; Rust already applies the shared UI projection.
- [ ] Return a typed Python validation error for `all:false` without a wallet
  in wallet sync and active-book freshness. The disposable
  `test_reference_all_false_without_wallet_repro_and_native_refusal` pins the
  current `None.strip()` failure; native whole-invocation selection declines it.
- [x] Integrate the separate Python tip-cache fix and match it in Rust:
  Esplora/Electrum compatibility evidence refreshes after a changed or missing
  block-identity anchor even when script statistics, status and heights match.
  Retain validated Electrum graphs while refreshing header timestamps; rerun
  fixture, restart-sequence and reviewed-book preparation/publication parity.
- [x] Enable supported CLI `wallets sync` with complete live fixture, restart and
  reviewed-book comparisons on Electrum/Esplora and zero hybrid fallbacks.
- [x] Enable verified explicit no-refresh variants of `ui.wallets.sync`,
  `ui.freshness.run` and `ui.workspace.freshness.run` after inert whole-shape
  selection, full envelope/event/book parity and zero served-shape hybrid
  fallbacks. Keep unsupported Liquid, Core RPC, Lightning, Silent
  Payments and additional-wallet overlap on Python in the hybrid runner; direct
  native unsupported calls refuse without retrying.
- [ ] Enable default refresh after the native R4-13 `journals process` service
  is registered and adapted through the worker-owned freshness handler. Preserve
  wallet sync's separate follow-up, workspace/job ordering, journal summaries
  and assistant no-auto-pair authority; retain enabled unported rate refusals.
  The product checkpoint in `docs/reference/rust-shell.md` records the exact
  seams and required default-refresh proof. No Python journal bridge.

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
status, signed display sorts, exact bounds and cursor pages. Desktop resolution
now preserves ID/alias selection, focused-leg principal and reviewed pair
fingerprints. Dashboard, graph, review context and ledger template remain
unported; transaction exports use the separate report service. Documents now cover
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
- [x] Port desktop transaction resolution with raw JSONL parity, unchanged
  112-table snapshots and named mutation kills.
- [ ] Port desktop dashboard/graph/review-context/template projections.
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
- [x] Port Ledger Live movement files with sanitized provenance, exact
  amounts/fees, complete book-dump/rollback parity and named mutation killers.
  See the partial batch-2a checkpoint in plan 21 and subsequent checkpoints below.
- [x] Port CoinTracking core/CLI with frozen files, full-book parity and named
  mutation coverage. See the CoinTracking checkpoint in plan 21.
- [x] Port Blockpit core/CLI with localized labels and optional column aliases,
  full-book/wire parity on SQLite and SQLCipher, transactional reconciliation
  and named mutation killers. See the Blockpit checkpoint in plan 21.
- [x] Port Binance supplemental autoinvest/dividend core/CLI ingestion with
  frozen Python files, SQLite/SQLCipher full-book/wire parity, commercial
  coordination, reference-bug repros and named mutation killers. See the
  Binance checkpoint in plan 21.
- [x] Port BTCPay CSV/JSON files at core/CLI level with frozen Python files,
  SQLite/SQLCipher full-book/wire parity, notes/tags, reference-bug reproduction
  and named mutation killers. See the BTCPay checkpoint in plan 21.
- [x] Port generic-ledger core/CLI ingestion and dry-run previews with frozen
  Python CSV/TSV/XLSX files, SQLite/SQLCipher wire/full-book parity, bounded
  ZIP/XML reads, reference-bug repros and named mutation killers. See the
  generic-ledger checkpoint in plan 21.
- [x] Port Wasabi sanitized bundle core/CLI ingestion with frozen Python files,
  safe metadata/inventory, SQLite/SQLCipher full-book/wire parity, transactional
  privacy review, retained reference-bug repros and named mutation killers.
  See the Wasabi checkpoint in plan 21.
- [x] Port Samourai public source-set core/CLI ingestion with frozen files,
  SQLite/SQLCipher raw-wire/full-book parity, source-group coordination,
  retained reference-bug repros and named mutation killers. See the Samourai
  checkpoint in plan 21.
- [x] Port the unregistered reviewed-document core: frozen OCR/review vectors,
  source-digest binding, managed attachments, SQLite/SQLCipher full-book and core
  response comparisons, rollback and named mutation killers. See the partial
  [document checkpoint](docs/reference/rust-shell.md#reviewed-document-core-r5-partial).
- [x] Add unregistered document provider discovery/OCR through `kassiber-net`,
  with exact-action consent, local-only policy and Python offline/no-egress
  fencing. See the [provider checkpoint](docs/reference/rust-shell.md#document-provider-transport-checkpoint-r5-partial).
- [x] Add Python-compatible Poppler PDF preparation for OCR, including page
  selection, bounded images, timeouts and cleanup. Retain and justify the native
  database-error redaction and managed-digest cleanup privacy deviations.
- [x] Finish document ingestion at core/CLI level: interactive CLI review, native
  registration and CLI wire/full-book/zero-fallback acceptance.
  R6 AI tool projections stay Python. An attachment grants neither ownership
  nor tax facts.
- [ ] Complete the [shared file-import daemon parity follow-up](docs/reference/rust-shell.md#shared-file-import-daemon-parity-pending)
  after its native prerequisites land.
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
read Python-produced journals through shared native CLI and desktop services.
The export continuations retain explicit saved/filed markers where Python
registers them; exit-tax preserves its reference no-registration boundary.

- [x] Port live summary, gains, balance sheet, wallet holdings and saved/filed
  snapshot listing, retaining readiness gates and exact JSON/table/plain output.
- [x] Port tax summary and Austrian E1kv/tax-summary reads, including
  Vienna-year boundaries, cent rounding, Kennzahlen, Alt/Neu sections and
  quarantine disclosure; port the desktop tax-summary kind.
- [x] Port the complete stored journal-entry listing with signed msat and
  deterministic split-row ordering.
- [x] Port CSV projections for the ten report reads and atomic `--output`
  publication, with FIFO/HIFO/Austrian byte parity and killed mutations.
- [x] Port exit-tax reads and XLSX through native CLI/desktop services, preserving
  estimate disclosures, exact workbook content and all database tables. The
  [exit-tax checkpoint](docs/reference/rust-shell.md#exit-tax-service-checkpoint-r5-fourteenth-reports-continuation-partial)
  records the corrected round-6/6b/6c/6d mirror, native reviewer matrix and
  version-2 journal migration; valuation snapshot registration remains separate.
- [x] Port historical balance/holdings reads and the desktop balance-history
  kind with native parity and killed mutations.
- [x] Reduce Rust read startup and report scaling costs without changing schema,
  admission checks or output bytes; see the
  [read-path measurements](docs/reference/rust-read-performance.md).
- [ ] Port individual snapshot lookup and amendment-review state when process
  callers exist and the stack includes phase 5; preserve append-only history.
- [x] Port retained compact/legacy capital-gain explanations through the native
  CLI and daemon, with exact reconciliation, custody bounds, full-table parity
  and named mutation tests.
- [ ] Finish desktop gains daemon journal-refresh parity and registration. Its
  native core projection is verified; Python rebuilds stale journals before
  serving the kind, so registration waits for the accepted native R4 producer.
  See the [partial gain checkpoint](docs/reference/rust-shell.md#gain-explanation-checkpoint-r5-fifteenth-reports-continuation-partial).
- [x] Port cross-check reads and workbooks through native CLI/desktop services,
  retaining diagnostic stale-journal behavior and Python reference quirks.
  Compare openpyxl content, response bytes and all 112 tables with zero hybrid
  fallbacks and named mutation kills; cross-check creates no saved snapshot.
- [x] Port live Lightning profitability reads and CSV through the shared native
  CLI/daemon service, with exact-action consent, current book/config checks,
  reference quirks, product wire/CSV/all-112-table comparisons and zero fallbacks.
  The [product checkpoint](docs/reference/rust-shell.md#lightning-profitability-product-wiring-r5)
  records non-zero served-by counts; node-snapshot/backend-test routes and AI
  projections remain separately scoped.
- [ ] Finish the remaining report read inventory and desktop kinds, including
  remaining non-PDF source funds and Privacy Mirror. The reviewed commercial
  subledger CLI read and its CSV export run natively, with reference vectors,
  killed mutations and hybrid routing proof.
- [x] Port configuration privacy hygiene through the native service, CLI and
  daemon, retaining count-only disclosure, local header inspection, stored
  freshness and Python endpoint/provider classification. The accepted corpus
  compares exact wire output and all 112 tables with zero in-scope hybrid
  fallback; see the
  [privacy checkpoint](docs/reference/rust-shell.md#privacy-hygiene-checkpoint-r5-reports-privacy-partial).
- [x] Close the configuration hygiene corrupt-marker daemon diagnostic gap
  under plan 21's native diagnostic contract: unhandled debug strings are empty,
  typed errors retain null, and every non-debug response byte matches Python.
  Six sessions cover all three markers with and without `--debug`, preserving
  all 112 tables with zero fallbacks; see the
  [diagnostic checkpoint](docs/reference/rust-shell.md#corrupt-marker-daemon-diagnostics-r5-privacy-continuation).
- [ ] Port Privacy Mirror through the shared observation index, public-observer
  analytics, conditional entropy and process-bound redaction before registering
  its CLI/daemon reads. Preserve conditional hypotheses and watch-only analysis.
- [x] Port stored source-funds source/link/case/recipient listings and paginated
  desktop evidence listing, preserving full editor metadata, frozen case
  recipient fields and unchanged accounting state. This is a partial boundary;
  see the [stored-list checkpoint](docs/reference/rust-shell.md#source-funds-stored-list-checkpoint-r5-partial).
- [x] Accept source-funds internal-error diagnostics under plan 21's honest
  native-stack contract: native debug strings are empty for uncaught failures
  and null for typed refusals; both former xfail witnesses pass with every
  non-debug response byte, all 112 tables and native routing checked. The
  [diagnostic checkpoint](docs/reference/rust-shell.md#source-funds-native-diagnostic-checkpoint-r5-partial)
  records 239 passing parity tests, zero fallbacks and five new mutation kills.
- [x] Preserve source-funds internal-error diagnostics under the approved honest
  native contract: identical non-debug bytes, empty native debug strings where
  Python supplies traceback text, and null in the same remaining cases. Pin
  CLI/daemon behavior, redaction, rollback and both database bindings; see the
  [diagnostics checkpoint](docs/reference/rust-shell.md#source-funds-diagnostic-wire-checkpoint-r5-partial).
- [x] Port source creation and managed-id source evidence association through
  the native service, CLI and daemon, with complete helper envelopes, all 112
  tables, file/hash invariance and late rollback on SQLite and SQLCipher.
  This boundary added two CLI leaves and two source-funds daemon kinds;
  11 CLI mutation calls and ten daemon mutation requests are verified with zero
  fallback. See the [partial source mutation checkpoint](docs/reference/rust-shell.md#source-funds-source-mutation-checkpoint-r5-partial).
- [x] Record source mutation AI execution, consent and once-only admission as
  Python-owned under the owner's R6 decision. AI tools call the Python service
  directly; native CLI/daemon acceptance does not claim native AI execution.
- [x] Port recipient create/update/soft-delete through the shared native
  service, CLI and daemon. Preserve scoped exact references, partial updates,
  frozen case history and binding-specific constraint errors. Compare full
  envelopes, all 112 tables and unchanged file hashes, with late rollback on
  both bindings and zero transport fallbacks. The inventory is 162 native CLI
  leaves and ten source-funds daemon kinds; Python exposes no recipient AI tools.
  See the [recipient checkpoint](docs/reference/rust-shell.md#source-funds-recipient-mutation-checkpoint-r5-partial).
- [x] Complete the source-funds snapshot-bound report producer, disclosure,
  custody/structural-lineage checks, recipients, explorer references, findings,
  SVG diagrams and text output on `t3code/r5-sof-reads-4`; see the
  [producer checkpoint](docs/reference/rust-shell.md#source-funds-report-producer-checkpoint-r5-partial).
  CLI reports without case saving and daemon previews route natively.
  Retain the known Python privacy-boundary fee-attribution bug for parity.
  Native tied-flow ordering is deterministic and compared with measured Python
  seed variants, without normalizing the output bytes.
- [x] Port source-funds review-context/request-input with fingerprint-bound
  advisory handoffs, bounded inspection and current bulk eligibility; see the
  [review checkpoint](docs/reference/rust-shell.md#source-funds-review-and-input-checkpoint-r5-partial).
- [x] Port historical inbound source-funds coverage through the shared native
  report gates, preserving exact totals, caps and error families; see the
  [coverage checkpoint](docs/reference/rust-shell.md#source-funds-coverage-checkpoint-r5-partial).
- [x] Port `source-funds suggest --dry-run` with local candidate ordering,
  existing-link vetoes and rollback; see the
  [suggestion checkpoint](docs/reference/rust-shell.md#source-funds-suggestion-preview-checkpoint-r5-partial).
- [x] Port link creation, review and managed-id evidence association through
  the shared native service, CLI and daemon, preserving exact stored msat,
  stale-review preconditions, explicit allocation policy and scoped evidence.
  Compare all 112 tables and attachment hashes on SQLite and SQLCipher, prove
  late rollback and zero transport fallback, and kill named mutations. See the
  [link checkpoint](docs/reference/rust-shell.md#source-funds-link-mutation-checkpoint-r5-partial).
- [x] Port deterministic source-funds batch review through CLI and daemon.
  Revalidate scoped selections under write admission, preserve CLI dry runs,
  compare all 112 tables and attachment hashes on SQLite/SQLCipher, prove late
  rollback and zero transport fallback, and kill named mutations. AI admission
  remains Python-owned under R6.
- [x] Port complete source-funds report/SVG case saving through CLI and daemon,
  including native daemon inspection fingerprints, frozen recipient/external
  fields, same-lock revalidation, all 112 tables, attachment hashes and late
  rollback on SQLite/SQLCipher, zero fallbacks and named mutation kills.
  See the [case-save checkpoint](docs/reference/rust-shell.md#source-funds-case-save-checkpoint-r5-partial).
- [x] Port persisted source-funds suggest/assemble with separate suggestion and
  review commits inside each pass, frozen inputs and SQLite/SQLCipher rollback.
  Verify complete envelopes, all 112 tables, attachment hashes, late failure,
  zero fallbacks and named mutation kills; see the
  [persisted-pass checkpoint](docs/reference/rust-shell.md#source-funds-persisted-pass-checkpoint-r5).
- [x] Keep AI case saving, consent and once-only execution Python-owned under
  the owner's R6 scope decision. The tool executor invokes Python's scoped
  service directly, preserving frozen book binding, redaction and reviewed
  fingerprints without adding a native AI loop.
- [x] Audit every source-funds CLI command and daemon kind: nineteen native CLI
  leaves and 21 native daemon kinds have zero fallbacks; PDF-containing exports
  and AI tools are explicitly Python-owned. See the
  [served-by audit](docs/reference/source-funds-rust-route-audit.md).
- [x] Port the shared public transaction-feature extractor, closed cache
  sanitizer and evidence-bound rule evaluator as a local native prerequisite.
  Frozen Python vectors and named mutations cover script-bound signature
  categories, signed versions, locks, exact amounts and collaboration exclusions.
  Mirror delivery remains separate from this shared prerequisite; see the [feature checkpoint](docs/reference/rust-shell.md#public-feature-primitives-r5-privacy-continuation-partial).
- [x] Port the separate legacy hygiene scorer with Python-derived vectors for
  every scoring rule, exact amounts, complete 112-table reads and named killed
  mutations; see the [scorer prerequisite](docs/reference/rust-shell.md#legacy-hygiene-scorer-prerequisite-r5-privacy-legacy-partial).
- [x] Accept and register its existing `ui.privacy_hygiene.snapshot` through the
  CLI-launched daemon, with non-zero native served-by counts and zero fallbacks.
  Preserve the distinct `reports privacy-hygiene` configuration CLI; add no command.
  See the [daemon acceptance](docs/reference/rust-shell.md#legacy-hygiene-daemon-acceptance-r5-privacy-legacy).
- [x] Port the shared bounded PSBT v0/v2 parser, supplied-UTXO/script validation,
  required-locktime resolution and redacted public analysis as a native
  prerequisite. The transient script-bearing inventory adapter is local-only.
  Frozen Python vectors, complete 112-table reads and named mutations verify
  this prerequisite boundary. The native report and its product transports are
  completed below. See the
  [PSBT parser checkpoint](docs/reference/rust-shell.md#psbt-parser-prerequisite-r5-privacy-psbt-partial).
- [x] Port reduced PSBT input/output inventory matching, observer-only component
  deltas, adversary scenarios and report scope redaction over an admitted graph
  baseline. Keep the scope and baseline callback in one SQLite read snapshot.
  The completed historical graph and registered product transports build on
  this prerequisite below. See the
  [overlay checkpoint](docs/reference/rust-shell.md#psbt-inventorylinkage-overlay-prerequisite-r5-privacy-psbt-partial).
- [x] Add native PSBT baseline capture for the bounded inventory-only domain,
  with explicit source admission and no injected graph in its Python oracle.
  Preserve one read snapshot, all 112 tables, ambiguous inventory, branch
  provenance and existing script-reuse hypotheses. Refuse books requiring
  historical observations, occurrence/binding services or dataset matching.
  The full historical constructor and registered transports below supersede
  these source refusals for product reports; the bounded constructor is retained
  only as a prerequisite checkpoint.
  See the [inventory checkpoint](docs/reference/rust-shell.md#psbt-native-inventory-admission-checkpoint-r5-privacy-psbt-partial).
- [x] Capture PSBT historical/adversary baselines through Mirror's native index
  and SQL source interfaces. Preserve public-only components, reviewed anchor
  exposure and bounded uncertainty inside one scope snapshot; compare unchanged
  Python bytes and all 112 tables; see the
  [historical checkpoint](docs/reference/rust-shell.md#native-psbt-historical-baseline-r5-privacy-psbt).
- [x] Verify the native PSBT CLI/file and daemon adapters against unchanged
  Python bytes, both offline-switch states and all 112 tables; see the
  [adapter checkpoint](docs/reference/rust-shell.md#psbt-clifile-and-daemon-adapter-acceptance-r5-privacy-psbt).
- [x] Register the reduced PSBT privacy CLI/file and daemon reads with exact
  whole-process bytes, all 112 tables, strict no-egress and zero hybrid fallbacks.
  Preserve native unhandled-debug semantics and kill all 21 transport/registration
  mutations. Product served-by counts are non-zero; see the
  [registration checkpoint](docs/reference/rust-shell.md#registered-psbt-privacy-reads-r5-privacy-psbt).
- [x] Port immutable captured analysis snapshots, physical occurrence resolution
  and the in-memory public observer projection as a Mirror prerequisite.
  The 195-case oracle preserves all 112 tables and records Python's ambiguous
  occurrence ordering across eight hash seeds. This does not ingest raw
  observations, synchronize the durable index or register Mirror; see the
  [index prerequisite](docs/reference/rust-shell.md#mirror-index-prerequisite-r5-partial).
- [x] Port Mirror's observation merge kernel and confirmation overlay as an
  unregistered prerequisite. The 339-case oracle includes eight fixed Python
  ambiguity-order variants and chained public projections, with byte and
  all-112-table checks. Durable publication remains open; see the
  [ingestion prerequisite](docs/reference/rust-shell.md#mirror-observation-merge-and-confirmation-prerequisite-r5-partial).
- [x] Port Mirror's full SQL source orchestration and coverage over the existing
  immutable index. Preserve wallet enrichment, reference activity, authored
  labels, indexed dataset matches, visibility budgets and custody freshness.
  The source oracle retains byte comparisons, all 112 tables, hash-seed
  variants and a separate Python whitespace-script reproduction; see the
  [SQL source checkpoint](docs/reference/rust-shell.md#mirror-sql-source-orchestration-r5-partial).
- [x] Port bounded in-memory Mirror traversal with captured Python vectors,
  byte-exact observer-aware traces/paths and named mutation killers; see the
  [traversal checkpoint](docs/reference/rust-shell.md#mirror-traversal-algorithms-r5-partial).
  This does not register Mirror or complete durable source orchestration.
- [x] Port in-memory Mirror clustering, observed reuse/co-spend findings and
  reversible opt-in hypotheses; see the
  [clustering checkpoint](docs/reference/rust-shell.md#mirror-clustering-algorithms-r5-partial).
- [x] Port captured-snapshot Mirror topology patterns with bounded fan-in/out,
  reconvergence and conditional peel candidates; see the
  [pattern checkpoint](docs/reference/rust-shell.md#mirror-topology-patterns-r5-partial).
- [x] Port Mirror attribution paths, exact dataset subject canonicalization and
  captured-row matching with independent visibility budgets; see the
  [attribution checkpoint](docs/reference/rust-shell.md#mirror-attribution-and-dataset-algorithms-r5-partial).
  SQL sourcing and dataset lifecycle/query services remain separate.
- [x] Port conditional Mirror entropy (exact value-class partitions, bounded
  intrafees, link counts and cooperative budgets) and full captured-index
  analytics composition; see the
  [entropy checkpoint](docs/reference/rust-shell.md#mirror-entropy-and-analytics-composition-r5-partial).
- [x] Preserve legacy non-finite JSON and lone surrogates through Mirror SQL
  sourcing, source commitments and observer aliases. Retain independent typed
  node identity checks and the Python malformed-manifest reproduction; see the
  [legacy JSON checkpoint](docs/reference/rust-shell.md#mirror-legacy-json-compatibility-r5-partial).
- [x] Port bounded observer-scoped Mirror SQL views and selected analytics, with
  byte/all-table parity, no-egress and mutation checks; see the
  [SQL view checkpoint](docs/reference/rust-shell.md#mirror-bounded-sql-views-r5-partial).
- [x] Port bounded RAM-only process/book-bound Mirror handles with byte, replay,
  restart, all-table and no-egress checks; see the
  [handle checkpoint](docs/reference/rust-shell.md#mirror-process-and-book-bound-handles-r5-partial).
- [x] Compose the native Mirror report over SQL views and existing analytics,
  preserving public observer knowledge, private relevance and bounded report
  projection; see the [report checkpoint](docs/reference/rust-shell.md#mirror-report-projection-r5-partial).
- [x] Prepare Mirror report scope/table adapters, CLI scope/format forwarding and
  daemon empty-argument validation, keeping product routes unregistered until
  durable recovery acceptance; see the
  [transport checkpoint](docs/reference/rust-shell.md#mirror-transport-adapters-awaiting-registration-r5).
- [x] Port Mirror's durable source triggers and replaceable contributions with
  canonical bytes, all-112-table commitments and no-consent native driver checks;
  see the [durable checkpoint](docs/reference/rust-shell.md#mirror-durable-triggers-and-contributions-r5-partial).
- [x] Port Mirror's incremental synchronization and generation publication,
  retaining affected-region context, tip-only generations and attribution expiry;
  see the [synchronization checkpoint](docs/reference/rust-shell.md#mirror-incremental-synchronization-r5-partial).
- [x] Integrate durable Mirror report reads with retained generations, exact
  all-table parity and caller rollback; see the
  [integration checkpoint](docs/reference/rust-shell.md#mirror-durable-read-integration-awaiting-recovery-and-registration-r5).
- [x] Verify Mirror's durable SQLite/SQLCipher recovery after process kills at
  every write and commit boundary, with Python byte/all-table parity in DELETE
  and WAL modes; see the
  [recovery checkpoint](docs/reference/rust-shell.md#mirror-durable-recovery-r5-partial).
- [ ] Finish Mirror's CLI/daemon acceptance with
  non-zero served-by counts. Preserve deterministic native
  ambiguity ordering; retain the recorded Python reference repro.
- [x] Register native Chain Analysis overview/trace/path, synchronous entropy,
  case list/get/compare, label listing and dataset list/get/query reads through
  the CLI and daemon, retaining byte/all-table parity and zero fallback; see the
  [read checkpoint](docs/reference/rust-shell.md#chain-analysis-local-read-checkpoint-r5-partial).
- [x] Finish the Chain Analysis read port: PSBT analyze/compare/entropy, dataset
  preview, entropy/PSBT-entropy/dataset-preview `.start`, `jobs.get`/`jobs.cancel`
  and `internal.chain_analysis.stage`, with purpose/book-scoped source grants,
  lock/profile-switch/restore fencing and byte/all-table process parity; see the
  [completion checkpoint](docs/reference/rust-shell.md#chain-analysis-psbt-preview-and-receipt-completion-r5).
  AI projections stay Python.
- [ ] Share the native analysis receipt/grant runtime with the dataset
  import/discard `.start` workers when the operations slice registers them.
- [x] Port chain-analysis label upsert/import/delete, case save/delete and
  dataset revoke/discard through one native service, with CLI/daemon byte
  parity, all-112-table checks, SQLite/SQLCipher rollback and named mutations;
  see the [local mutation checkpoint](docs/reference/rust-shell.md#chain-analysis-local-mutations-r5-partial).
- [x] Port all eight chain-analysis `watches` commands and daemon kinds with
  CLI/daemon byte parity on encrypted books, all-112-table checks and named
  mutations; see the [watch checkpoint](docs/reference/rust-shell.md#chain-analysis-evidence-watches-r5-partial).
- [x] Port the five chain-analysis `sources` commands and daemon kinds (plan,
  authorize, list, revoke, queue) with byte parity, all-112-table checks and
  named mutations; see the [source-grant checkpoint](docs/reference/rust-shell.md#chain-analysis-source-grants-r5-partial).
- [x] Port the chain-analysis operations assignment: watches, sources,
  acquisition plan/apply, CLI and daemon dataset import with the import/discard
  job starters, and the daemon background worker (source runner, watch tick and
  event delivery); see the [plan checkpoint](docs/reference/rust-shell.md#chain-analysis-acquisition-plans-r5-partial),
  [import checkpoint](docs/reference/rust-shell.md#chain-analysis-dataset-import-r5-partial)
  and [source runner record](docs/reference/rust-port-egress.md#chain-analysis-source-runner-r5).
- [x] Integrate the chain-analysis operations and reads branches (R10): keep one copy
  of `analysis_index::runtime`, `reads/preview.rs` (plus `stream_claims`) and
  the daemon runtime hunks, sort the combined chain-analysis kinds into one
  block, and drop the `dataset_stream` shim.
- [x] Port full-report, Austrian bundle and transaction CSV artifacts, saved
  export registration and their daemon kinds. Preserve
  sealed summaries, hashes and phase-4 append-only impact history.
  Completion computes hashes, exact summaries and authoritative dependencies
  under retained admission. Complete normalized DB sequences and native
  CLI/daemon process oracles prove byte parity and zero CSV fallbacks, including
  desktop capital-gains CSV. Graphless Liquid dependency capture now matches
  Python's local parse-success boundary, with stored identity and ambiguity
  guards intact. Platform transaction formats are recorded below; see the
  [CSV transport checkpoint](docs/reference/rust-shell.md#csv-export-transport-checkpoint-r5-tenth-continuation).
- [x] Add the isolated deterministic `rust_xlsxwriter` renderer and strict
  openpyxl content oracle for all report workbook shapes; port transaction-list
  XLSX projection, publication and CLI/daemon registration with zero fallbacks.
- [x] Port Austrian E1kv XLSX projection and CLI/desktop registration through
  native completion; compare workbook content, actual hashes, explicit filing
  and every database table, with zero hybrid fallbacks.
- [x] Port full-report XLSX, including profile-wide verification, CLI/desktop
  registration, actual-byte saved hashes and repeated explicit filing; compare
  openpyxl content and every database table with zero hybrid fallbacks.
- [x] Port year-handoff XLSX projection, CLI/daemon registration, saved
  completion and explicit filing; compare openpyxl content, all 112 tables
  and native routing. See the [worker B checkpoint](docs/reference/rust-shell.md#year-handoff-xlsx-checkpoint-r5-reports-worker-b-partial).
- [x] Port CoinTracking/Blockpit CSV through `transactions export` and
  `ui.transactions.export_platform_csv`, preserving exact UTF-8 CSV bytes,
  current journals, scoped quarantines and the reference no-registration
  boundary. See the [platform checkpoint](docs/reference/rust-shell.md#platform-csv-checkpoint-r5).
- [ ] After migration parity, correct platform CSV reference bugs in a separate
  behavior change: unused aggregate overflow, empty-date all-time failure and
  empty exact journal values masking legacy amounts. The
  [platform checkpoint](docs/reference/rust-shell.md#platform-csv-checkpoint-r5)
  links the disposable repro; keep current parity until that change is accepted.
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
  Source exhaustive audits and mutation acceptance are inherited from
  t3code/r4-9-loader-determinism@aaf6216e3. Fresh R9 verification passes all
  274 loader/BLOB cases, the 24,000-book/1,030-permutation custody campaign,
  159 archived requests and permutations, and focused census/shadow. The
  [R9 audit](docs/reference/rust-backend-r9-integration.md#fresh-r9-verification)
  tracks the remaining combined parity and mutation gates.
- [ ] R4-10 to R4-12: pre-build repairs, rendering and joins, persistence,
  each with byte or database-dump parity.
- [ ] Finish the [R4-10 auto-pricing checkpoint](docs/reference/r4-journal-pipeline.md#r4-10-auto-pricing-checkpoint):
  finish census/shadow, mutation and CPU acceptance.
  Cached pricing, source-overlap repair/warnings, and drifted legacy-pair refresh
  with profile reload have unregistered native interfaces and complete-dump
  oracles. The combined stage and split repair/price interface for R4-13 retain
  whole-pipeline rollback after later build/persistence failures.
  The [scenario replay checkpoint](docs/reference/r4-journal-pipeline.md#r4-10-scenario-replay-checkpoint)
  freezes 505 stored scenarios from the twelve generators and passes 1,010
  plaintext/SQLCipher file comparisons; ten schema refusals are tracked
  separately. The [generated and recorded campaign](docs/reference/r4-journal-pipeline.md#r4-10-generated-and-recorded-campaign)
  now completes 24,000 seeds and 1,030 permutations: 24,815 stored operands,
  49,630 file comparisons and 215 separate schema refusals. Historical replay
  and 89 fresh full-book captures also pass both file drivers. Fresh source
  activity is measured separately. The [mutation/census checkpoint](docs/reference/r4-journal-pipeline.md#r4-10-mutation-and-census-checkpoint)
  closes the 81 pre-build recipe kill set and records fresh handler activation
  and domain-skip outcomes. Core shadow passes; census and report coverage
  completes across initial runs and exact continuations after recorder fixes.
  Three recorder/guard mutations and all 112 inherited journal mutations are
  killed. All 280 transfer/daemon recipes also pass. Custody mutation execution,
  the branch audit and whole-stage cache/CPU acceptance remain open.
  The [numeric branch follow-up](docs/reference/r4-journal-pipeline.md#r4-10-uncovered-numeric-paths)
  adds binary-number refusal and exact infinite-REAL dump witnesses, with 530
  file comparisons and 19 killed numeric/dump mutations. Remaining branch
  dispositions, inherited custody gates and final campaign binding still block
  acceptance. A further 32 input witnesses pass 658 file comparisons and 12
  additional mutations. The [branch/cache checkpoint](docs/reference/r4-journal-pipeline.md#r4-10-branch-and-cache-checkpoint)
  dispositions all 104 remaining executable-region entries and kills 116
  pre-build, 112 journal and three recorder recipes in an isolated final-source
  run. Whole-stage default-cache 20k medians meet 0.03 s; scoped-cache
  multi-wallet is 0.034666 s, with its complete cost breakdown recorded.
  All 2,327 inherited custody recipes also pass across verified prefixes and
  a targeted continuation. Final campaign binding and acceptance remain open.

> Inherited from `t3code/r4-10-prebuild-5@a0393276a`; pending r10 verification.
- [ ] R4-9 to R4-12: native loader, pre-build repairs, rendering and joins,
  persistence, each with byte or database-dump parity.
- [x] Finish the [R4-10 pre-build boundary](docs/reference/r4-journal-pipeline.md#r4-10-functional-acceptance):
  ordered source repair/warnings, legacy refresh/profile reload and cached pricing,
  including later-failure rollback and result-field parity. The final binary passes
  56,300 complete SQLite/SQLCipher comparisons across the full generated campaign,
  historical recordings, upgraded fixtures, frozen/edge cases and census/shadow
  books. All 2,838 catalogue entries are killed; combined-run qualifications and
  complete activity accounting are in the acceptance record. Python's known
  `L-BTC` and fractional-msat bugs remain unchanged, with repros.
- [ ] Resolve the scoped-cache multi-wallet pre-build CPU budget during R4-13
  whole-pipeline measurement. Its 20k median is 0.034666 s against 0.03 s;
  default-cache books meet the target. The acceptance record preserves all
  samples and the complete cost breakdown; the shared 128 MiB cache is unchanged.
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

> Inherited from `t3code/r4-12-persist-accept@d6e46912b`; pending r10 verification.
  repeated work; the last complete five-pair campaign passes one of eight
  required 20k fresh/unchanged cells. A later schema-preserving digest probe
  missed the generic unchanged CPU budget over five pairs and was withdrawn;
  its retained-row regression witnesses and measurements are recorded in the
  checkpoint. A borrowed retained-row probe records all four 20k read floors.
  Fresh-connection SQLCipher reads exceed the old allowance; read floor + 0.15
  seconds is proposed for unchanged cells. Fresh keeps the write floor. A
  quiet-host repeat and cheap unchanged comparison remain open. A later typed
  comparison prototype also missed the proposed unchanged allowance over five
  pairs and was withdrawn; its measurements and an integrated negative-zero
  rewrite witness remain, with a targeted `JP-signed-zero` kill. Corrected
  appended benchmark inputs carry distinct raw identities and assert actual writes.

> Inherited from `t3code/r4-12-persist-accept@9730f9e4b`; pending r10 verification.
  close the accepted CPU budgets (read floor + 0.15 s unchanged; write floor +
  0.15 s fresh), exhaustive persistence mutations and inherited campaign gates
  before wiring production in R4-13. The unregistered service, complete dump
  oracle and caller-owned storage scopes are implemented. Independent borrowed
  read floors were repeated over five connections per driver and shape, with
  exact dumps and load averages. The closest direct comparison is now retained
  under the owner's incremental optimization rule. Generic SQLite 20k baseline
  and candidate minima improve, but their load ranges differ and both accepted
  budgets remain open. The alternating native-baseline harness records a slower
  single fresh 2k sample explicitly. Exact coefficient comparisons now avoid
  unchanged decimal formatting at every width; five alternating pairs improve
  each tested width from 40 to 4,096 digits; scale-based layout checks improve
  full decimal equality from 12 to 4,096 digits. Singleton timestamp tracking also
  improves an exact changed-book comparison across six runtime orders. Native
  timing operations now use fresh processes so the SQL floor cannot bias
  allocator state. Bounded exponent validation also avoids coefficient digit
  counts; its focused pairs improve at every measured width, while complete
  store timings remain contention-qualified. Empty retained tables now skip
  comparison sorting and timestamp-group construction; fresh paired CPU improves,
  while the unchanged paired cell regresses slightly and acceptance stays open.
  The [correctness continuation](docs/reference/r4-journal-persistence-correctness.md)
  pins both physical drivers' caller contracts, kills all 410 journal recipes,
  and completes generated, retained and fresh custody/render replay shards.
  Finish the full CPU gate alongside the exhaustive per-decision branch map,
  full census/shadow and complete custody mutation campaign; acceptance remains
  open beyond CPU.
  Historical signed-zero, retained-column and mixed-clock witnesses remain.
  Corrected appended inputs carry distinct raw identities and assert actual writes.
  R4-9's test-only loader now constructs the current custody records directly;
  generic 20k still takes 0.505 s CPU / 0.509 s wall against 0.30 s. It remains
  unaccepted pending the typed consumer boundary, full legacy fidelity and
  complete parity/mutation gates
  ([evidence and remaining work](docs/reference/r4-native-loader-checkpoint.md)).
- [ ] Finish the [R4-11 rendering checkpoint](docs/reference/r4-journal-render-checkpoint.md):
  close scalar/refusal gaps and complete the final integrated mutation
  and inherited campaign gates. The [performance follow-up](docs/reference/r4-journal-render-performance-checkpoint.md)
  meets the historical wall-clock targets. The [boundary continuation](docs/reference/r4-journal-render-boundary-checkpoint.md)
  repairs integer refusals, holding keys and nonfinite detail/summary values;
  the later scalar continuations below close the actual-surrogate repros.
  The complete acceptance gates remain open.
  The [allocation continuation](docs/reference/r4-journal-render-boundary-checkpoint.md#fifth-continuation-native-fragment-quantities-and-allocation-checkpoint)
  preserves all 379 prior records and expands the corpus to 383 cases. Eleven
  focused mutations are killed; the full campaigns remain open. Five alternating
  pinned pairs give 0.151179 s generic and 0.179153 s multi-wallet CPU at 20k;
  both still miss 0.15 s, and checkpoint comparisons do not isolate the regression.
  The [borrowed-pricing continuation](docs/reference/r4-journal-render-boundary-checkpoint.md#sixth-continuation-borrowed-pricing-and-entry-identities)
  passes the isolated CPU budget with exact state bytes: 0.125116 s generic and
  0.145054 s multi-wallet in five alternating pinned pairs. Final fidelity and
  integration changes still need a fresh CPU check.
  The [legacy-float continuation](docs/reference/r4-journal-render-boundary-checkpoint.md#seventh-continuation-legacy-transfer-floats-and-carry-refusals)
  preserves those 383 cases and adds 39 audit/carry witnesses. Native audit
  floats and nonfinite transfer quantities preserve overflow through rendering;
  persistence refuses nonfinite msat and restores every table. Eleven focused
  mutations are killed. Fresh CPU pairs do not recertify both 20k budgets;
  surrogate/decimal refusal fidelity, typed integration and full gates remain open.
  The [controlled CPU comparison](docs/reference/r4-journal-render-boundary-checkpoint.md#eighth-continuation-controlled-cpu-comparison)
  reestablishes both budgets on unchanged native source in seven alternating
  pairs: 0.121545 s generic and 0.146205 s multi-wallet CPU, with exact state bytes.
  The [ordered-refusal continuation](docs/reference/r4-journal-render-boundary-checkpoint.md#ninth-continuation-ordered-response-refusals)
  repairs the malformed-decimal repro with a cold traversal and pins 550 refusal
  cases with exact UUID consumption. Seven focused mutations are killed;
  surrogate scalars, successful lazy decoding and the complete gates remain open.
  The [native ledger continuation](docs/reference/r4-journal-render-scalar-checkpoint.md#native-ledger-continuation)
  matches 990 Python ledger states/refusals with exact identity consumption.
  The [partial native builder assembly](docs/reference/r4-journal-render-scalar-checkpoint.md#native-builder-assembly-continuation-partial)
  matches 1,254 complete states/refusals with exact UUID consumption. Native
  ownership graph consumption, the probe handoff and final acceptance remain open.
  Python retains production authority.
- [x] R4-14: expose the native automatic-pairing journal stage by reusing
  transfer suggestions, rules and bulk exact pairing. Pin repeated-pass and
  caller-transaction database parity on SQLite and SQLCipher, hash-seed sweeps,
  generator/recorded books and named mutations. The [CPU acceptance](docs/reference/rust-pairing.md#cpu-acceptance-2026-10-31)
  reaches 13.02× / 11.75× Python at 20k. CLI pairing is integrated under R4-13;
  desktop, AI and freshness journal routes retain Python.
- [ ] R4-13, R4-15 and R4-16: CLI and daemon journal commands, integration of
  automatic pairing, the freshness journal job and the authority switch.

> Inherited from `t3code/r4-15-journal-refresh@dfeaf1533`; pending r10 verification.
  reaches 13.02× / 11.75× Python at 20k. Pairing journal routes remain Python.
- [ ] R4-13 and R4-16: CLI and daemon journal commands, integration of
  automatic pairing and the authority switch.
  R4-13's [composition checkpoint](docs/reference/r4-journal-process-checkpoint.md)
  wires native load through persistence, ordered progress and outer rollback.
  The continuation imports cached pricing, R4-14 pairing and renderer boundary
  hardening; it adds real-pricing and abrupt-death recovery probes. Production
  resolution and both repair stages are still missing from the committed handoff.
  Integrate real R4-10 pre-build, close full-pipeline parity/mutation and speed
  gates, then register CLI and daemon in separate verified commits. Reconcile
  the desktop daemon's pairing default with R4-14 before changing its route;

> Inherited from `t3code/r4-13-journals-process-6@684c76bad`; pending r10 verification.
  The continuation imports complete R4-10 pre-build, R4-14 pairing, renderer
  boundary hardening and persistence recovery. Its real pre-build adapter retains
  profile reload and distinct repair/build/store/filed-report clocks; complete
  process vectors include active repairs and late rollback. Production
  scope resolution is now bound and plain CLI processing is registered with a
  byte-identical rollback guard for refusal replay through the hybrid launcher.
  The CLI automatic-pairing flag now shares the scoped native refresh service,
  retaining Python's first stale-rebuild commit and rolling back pending pairs
  with a failed second rebuild. Register desktop and scoped AI in separate
  verified commits; finish registered-route campaigns, census/shadow, inherited
  mutations and benchmarks.
  The third continuation integrates the requested loader/BLOB, pre-build,
  render and persistence tips, adds resumable complete-process shards, and
  repairs network-refusal metadata and merged BLOB conversion defects. Its
  seventeen frozen process books preserve every inherited record. Fresh full
  campaigns pass 25,030 generated comparisons and 318 retained-recording
  comparisons, including ordered result bytes and exact refusals. The full
  inherited census/shadow and mutation gates remain open.
  The test-only refresh coordinator now owns atomic pairing/rebuild commits,
  ordered result bytes, skipped-pairing refusals and WAL/cache restoration.
  Its 57 complete reference books cover both commits, skip-current and failure
  recovery. Bind it to production while preserving desktop pairing and scoped
  AI no-pairing behavior. Track speed acceptance separately;
  unregistered probe results do not authorize production routing. The fresh
  four-book CPU record includes real native pre-build and 20 exact paired
  dumps; the 4× target remains open on the separate performance track.
  The fourth continuation integrates the later renderer audit/native scalar
  ledger and retained-row persistence tips, repairing their borrowed model
  compatibility. Full lossless builder/joins/persistence handoff remains open;
  its stage integration checkpoint does not register any journal route or
  recertify the earlier complete-process campaigns.
- [x] R4-15: bind the freshness journal handler and post-wallet local refresh
  through the shared native service, holding the foreground execution slot
  throughout. Default desktop sync/freshness routing matches Python on both
  loopback protocols, including pairing authority, cancellation and lifecycle
  retirement. Python cancellation rounds 7/7b and interruptible native backoff
  share the same fences. See the [integration checkpoint and refusal census](docs/reference/r4-journal-freshness-checkpoint.md).
  Python retains production authority until R4-16. Production Rust returns its
  refusal; only development hybrid replay can use Python after proven rollback.
  Lossless refusal closure, inherited campaigns and timing acceptance remain
  on the R4-13/R4-16 tracks.

> Inherited from `t3code/r4-11-render-fidelity-5@f1712c625`; pending r10 verification.
  ownership graph consumption is completed by the
  [native ownership continuation](docs/reference/r4-journal-render-scalar-checkpoint.md#native-ownership-graph-continuation),
  which compares 1,435 complete outcomes. The
  [lossless probe handoff](docs/reference/r4-journal-render-scalar-checkpoint.md#lossless-probe-handoff-continuation)
  closes all four actual-surrogate repros and matches all 1,435 cases through
  fresh and cached executable reads. The
  [nonfinite response continuation](docs/reference/r4-journal-render-scalar-checkpoint.md#nonfinite-response-and-diagnostic-continuation-partial)
  adds 1,776 builder/ledger outcomes and repairs diagnostic float sums, wallet
  mapping lookup and JSON NaN holding-key identity. Complete custody/Austrian
  nonfinite coverage, successful lazy decoding, shape/refusal/UUID campaigns
  and final acceptance remain open. Python retains production authority.
- [ ] R4-13 to R4-16: CLI and daemon commands, automatic pairing, the
  freshness journal job and the authority switch.

## Rust backend R5 transfers and review

The [partial checkpoint](docs/reference/rust-shell.md#transfers-and-review-classification-r5)
serves all five stored reads, fourteen authoring commands (rule CRUD/toggles,
dismissals, pair/unpair/update, payout create/delete, component and gap plan/apply),
suggestions/bulk pairing/rule application, component list/show, and gap list/review:
**26/29 CLI leaves and 25/28 desktop kinds**. Full
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
- [ ] Port review cases/plan/apply through the shared journal service. The
  `b5eb38ea8` base contains the native SQL snapshot/ownership loader and journal
  writer checkpoints. Transfers-10 joins loading, custody, fusion and rendering
  behind `test-harness`, with complete build-state comparisons and RAM-only
  SQLite/SQLCipher planning snapshots. Transfers-11 adds the tested case-reader
  library, cursor freshness, waiting roots and pair evidence/fingerprints.
  Transfers-12 joins it directly to the native assembler on the live connection,
  preserving pending caller edits and complete assembler refusals, and pins
  operation normalization/content digests against 216 Python vectors. Transfers-13
  adds complete economic/policy effects on the live native build and portable
  artifact construction/pre-transaction admission primitives; these do not
  simulate ordered operations or validate/apply a plan. Production counts
  remain **26/29 CLI leaves and 25/28 daemon kinds**. Transfers-14 integrates
  accepted loader-9 and BLOB work, including their missing prerequisites;
  transfers-16 integrates the deterministic refusal fix, closing all forty
  former seed-dependent message exceptions.
  Transfers-15 adds ordered mixed simulation, initial fingerprint prechecks,
  sibling-unpair baselines, refresh after intervening operations, RAM-only
  SQLite/SQLCipher planning and whole-artifact recomputing validation. These
  services remain behind `test-harness`. Transfers-16 adds writer-locked atomic
  apply, historical retries, canonical journals, filed-impact resolutions and
  receipts, with complete logical-row/authorship comparisons and late rollback
  on SQLite/SQLCipher. The caller's commit choice and connection cache are
  preserved. Transfers-17 composes the accepted raw loader directly through
  custody, fusion and rendering, preserving downstream byte/scalar handling,
  authoring-string refusals and metadata no-op ordering. Finish product
  CLI/daemon envelopes, AI once-only consent and zero-fallback proofs before
  registering all six routes. See the
  [raw-composition checkpoint](docs/reference/rust-shell.md#raw-review-composition-and-authoring-refusals-r5-transfers-17-partial).
  Retain readiness barriers rather than reading stale journals; see the
  [review adapter handoff](docs/reference/rust-shell.md#review-integration-handoff).
  The [R4-14 pairing library](docs/reference/rust-pairing.md) now supplies the
  matcher/rules/bulk implementation and its oracle/mutation evidence. R4-15 now
  binds its atomic journal boundary for freshness; standalone desktop/scoped-AI
  journal registration remains on R4-13.

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
- [ ] Remove raw node-response disclosure from Python LND HTTP/JSON errors.
  `crates/chain/scripts/repro_python_lnd_errors.py` reproduces response text in
  HTTP error messages and malformed-JSON `response_preview` details using
  synthetic responses and no sockets. Native LND report errors omit those
  bodies; Python remains unchanged in this port checkpoint.

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
