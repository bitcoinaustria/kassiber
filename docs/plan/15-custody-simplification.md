# Custody architecture simplification

Status: behavioral cutover and validation complete; final hard-stop audit found
one unmet criterion (raw custody-core LOC). `TODO.md` remains the execution
backlog; this document records the invariant and migration boundary.

## Why this follow-up exists

PR #439 established the fail-closed custody quantity model. It also left
reviewed meaning and consumer interpretation distributed across transaction
pairs, direct swap payouts, custody components, gap reviews, reports,
transaction graphs, source-of-funds, UI handlers, and AI tools.

This follow-up removes competing truths without weakening:

- authoritative, current observer provenance;
- immutable observation and authored-evidence identity;
- exact msat conservation;
- atomic reviewed N:M activation;
- suspense and scoped basis barriers;
- separation of custody finality from tax meaning;
- explicit review for a missing-wallet basis carry;
- fail-closed conflicts, replication, and audit history.

One profile remains one legal owner. A clear set of known custody issues never
means that every wallet owned by that person or company has been imported.
Descriptor presence establishes ownership only inside authoritative current
observer coverage.

## Starting competing paths and current authority

At the merge baseline, authored meaning could be read independently from
`transaction_pairs`, `direct_swap_payouts`, active custody components and gap
reviews. Transfer interpretation was repeated by the RP2 path, reports,
transaction graphs and source-of-funds, while mutation-specific CLI/daemon
handlers assembled their own lifecycle operations.

The current path has one accounting authority. `CustodyJournalBuilder`:

1. load transactions, observer provenance, wallet policy, loan and channel
   state;
2. interpret native matches and compile active reviewed components;
3. discover gap candidates and compile quantity claims;
4. arbitrate quantity and compile finalized tax inputs;
5. invoke RP2 and persist journal projections.

Reports, transaction graphs, source-of-funds, UI and AI consume the stored
projection. Gap discovery recomputes the same hard-capped derived population
for accounting and review reads; only the builder's ignored-boundary inputs are
stored, and presentation cursors are bound to the journal input version.

The legacy pair and payout tables are no longer accounting truths: they are
write-frozen migration and delayed signed-replay inputs. That narrow physical
compatibility exception cannot create, revise, delete, interpret or report a
current review.

## Merge baseline

The inspected merge is GitHub PR #439 at merge commit `76d907f6051694f5ec81ee8b4c7f4130b8de2b4f`;
its `ci` and `credential-platforms` checks were green and all review threads
were resolved. Before this slice, the focused custody baseline was 55 passing
tests in 29.67 seconds.

At that commit, `kassiber/core/custody*.py` comprised 17 production modules and
18,733 lines. The custody composition inside
`kassiber.cli.handlers.build_ledger_state` occupied another 457 lines. The
schema contained 20 custody-specific authored, audit, review, migration, or
projection tables:

- `transaction_pairs`, `direct_swap_payouts`, `custody_components`,
  `custody_component_legs`, `custody_component_allocations`,
  `custody_component_evidence_commitments`,
  `custody_component_transaction_memberships`, and
  `custody_component_purge_authorizations`;
- `custody_gap_reviews`, `custody_gap_candidate_snapshots`,
  `custody_gap_review_relation_sets`, and `custody_gap_review_transactions`;
- `journal_custody_decisions`, `custody_authored_evidence_snapshots`,
  `custody_ai_assistance_audits`, `custody_filed_report_impacts`, and
  `custody_filed_report_impact_resolutions`.

The follow-up removed the three local-only custody-tax cutover tables before
merge. Their journal hook could not distinguish pre-custody journal output
from the current builder's output, so a fresh book created a meaningless
self-comparison on its second rebuild. Filed-report impact history remains the
authoritative amendment record.

The daemon exposed 14 custody review/read kinds: coverage and lineage
snapshots; gap list, history and review context; bridge create/preview;
dismiss; reopen/preview; revise/preview; and residual classify/preview. This is
an inventory, not a claim that every table or kind is accidental: immutable
audit, filed-report, replication, and migration history remain protected until
their consumers and rollback windows are proven obsolete.

## Target data flow

```text
immutable observations + current observer authority
reviewed component aggregate + typed replicated economic terms
persisted advisory gap candidates + scoped holds
                         |
                         v
                CustodyJournalBuilder
                         |
       decisions + issues + barriers + lineage
             |           |             |
             v           v             v
 FinalizedTaxProjection reports     graph/source-of-funds/UI/AI
             |
             v
            RP2
```

Only authoritative native evidence or an active reviewed component may produce
a basis-carrying edge. A suggestion may hold a suspected source and return out
of premature tax projection, but it does not contain an authoritative
source-to-target allocation. Dismissal removes the holds; an unmatched outbound
then becomes an explicit `external_presumed` decision without manufacturing an
input fallback claim.

Review preview is a pure `plan_review` operation. Apply rebuilds the requested
plan and persists it only while the previewed journal input version is current.

## Authored substrate and replication

The target authored aggregate is a custody component with immutable legs,
allocations, revisions, evidence commitments, and typed replicated economic
terms. Those terms must preserve pair policy/source/confidence/swap-fee meaning
and direct-payout asset, amount, time, fiat value, external reference, and
counterparty.

`transaction_pair_dismissals` remain narrowly separate because they suppress a
matcher; they do not assert custody or carry basis.

Legacy `transaction_pairs` and `direct_swap_payouts` are migrated using
deterministic component IDs and source-row hashes. Their historical replication
events remain replayable, so the old tables are write-frozen and removed from
production interpretation before any physical deletion is considered. Physical
removal requires the replication acknowledgement/tombstone protocol; it is not
a prerequisite for one authored accounting truth.

New pair and direct-payout reviews write only immutable component revisions
and replicated economic terms. Revisions and fan-out deletion rebuild the whole
shared aggregate atomically without dropping sibling terms. Same-asset
shortfalls equal to observed miner fees become fee legs; otherwise they remain
explicit suspense. Legacy tables are read-only migration and signed-replication
inputs; they are not a live authored, interpreter, report, or mutation path.
Malformed historical rows create durable migration issues and scoped journal
barriers instead of falling back to their former specialized interpreter.

No dual mutable writes are allowed. Downgrade after component-only writes means
restoring the pre-migration backup, not silently continuing with an older
binary.

## Ordered slices

1. Extract the core custody journal builder, convert production capacity state
   to an ordinary result, add unpatched characterization, and measure the real
   builder boundary.
2. Centralize boundary-leg normalization and deterministic N:M allocation.
   Completed: imported-row, canonical-observation, gap-discovery, bridge-plan,
   and component-coverage arithmetic share one boundary normalizer; gap claims
   and bridge plans share one all-or-nothing FIFO allocator with exact offsets
   and residuals. Transactionless component-route flattening remains a narrow
   provenance operation rather than a competing allocation policy.
3. Replace candidate transfer claims with scoped holds; compute one bounded
   candidate population per read; introduce pure plan/apply.
   The first part is complete: promotion now emits independent source/return
   holds and typed issues, never a source-to-target claim. Accounting/review
   consumers now run the same deterministic matcher over current observations;
   only the journal builder's exact ignored-boundary ID lists are persisted.
   Completeness travels in the ordinary result, and list pages use a
   version-guarded in-memory ordinal/gap-id cursor over the hard-capped
   population. There are no candidate rows, serialized pages, full-book input
   hashes or retention pruning. Reviewed records that leave the current
   candidate population remain available through point lookup and immutable
   history.
   The canonical quantity runtime now requires that projected search result as
   an explicit input. It has no matcher fallback: only `CustodyJournalBuilder`
   invokes discovery while compiling holds and decisions, and the review
   surface invokes that same read path independently.
   Planning is now one read-only
   `plan_review` seam for create/revise/reopen/residual actions; it commits the
   current journal input version, exact deterministic component rows and filed
   report impacts. One `apply_review` seam replans and rejects stale journal
   input versions before performing the reviewed mutation. Existing CLI/daemon kinds
   are redacted compatibility wrappers over those two operations.
4. Add typed replicated component economic terms, migrate pair/payout authored
   state deterministically, and freeze legacy writes. Complete: active and
   deleted historical reviews migrate to immutable component history; connected
   pair graphs become one atomic N:M aggregate; explicit residual and fee legs
   cover every source boundary. New create/revise/delete operations are
   component-native. Incremental reusable same-asset reviews grow that same
   immutable aggregate for both 1:N fan-out and N:1 fan-in, so a shared
   boundary is never represented by overlapping active components. Historical
   rows that cannot satisfy current conservation or provenance rules produce
   durable `custody_authored_migration_issues` and
   block their exact transaction scope. Delayed signed legacy events are
   migrated during the same bundle import. Specialized pair/payout claims,
   arbitration conflicts, tax relations, list mutations and `apply_manual_pairs`
   are deleted. Physical table removal remains deferred until old replication
   streams can no longer arrive.
   The future-only custody-layer adapter and producerless authored component
   types are removed. The small canonical observation/interpreter boundary and
   real Lightning lifecycle interpreter remain; later layers must introduce a
   concrete adapter and tests with their actual protocol semantics.
   Unmatched outbound slices are now the arbiter's direct
   `external_presumed` default, not manufactured low-priority claims. A scoped
   hold still converts the uncovered remainder to suspense, preserving the
   report barrier without creating a basis edge.
5. Cut reports, graph, source-of-funds, UI and AI to stored decisions/lineage;
   require a gated report context; delete compatibility interpretation,
   rollback previews, speculative layer scaffolding, and obsolete commands.
   Behavioral cutover complete: MOVE decisions and non-quantity
   conversion/payout relations are
   stored together by the canonical projection replacement. The rebuildable
   rows retain only quantity/provenance coordinates and their processed
   `basis_state`; reviewed kind/policy/source, notes, fees and payout metadata
   remain authored once in immutable component terms. One normalized SQL view
   joins those facts for every consumer. The small economic-relation table is
   a deliberate narrow exception to full derivation from authored terms:
   targetless conversions and direct payouts have no target-bearing MOVE row,
   and their processed basis eligibility cannot be reconstructed from authored
   economics without rerunning arbitration. Removing that row would weaken the
   report barrier, so only its duplicated semantic columns were deleted. Reports,
   exports, transaction graphs, source-of-funds, transaction/journal UI and AI
   snapshots read that projection; stale books do not render old custody
   grouping as current booked truth. Source-of-funds no longer has private pair,
   component, UTXO or payment-hash allocation engines. Consumer-side calls to
   transfer detection have consequently been deleted, leaving the custody
   journal interpreter as the only production caller. Journal-derived reports
   and exports now enter through one core `ReportContext` that proves tax
   support, journal input-version freshness, active-component integrity and
   clear quantity barriers. Nested report composition reuses the same proof;
   the former CLI-owned `require_processed_journals` hook is deleted. The
   component-only producer cutover and speculative-layer deletion are complete.
   The bounded historical interpreter is now deleted. Remaining work is command
   consolidation, claim-shape simplification, unpatched database integration
   coverage, and the final performance/quality audit. Component batch preview
   no longer creates rows inside a rollback savepoint: one core read-only planner
   resolves and normalizes exact rows, validates database anchors and batch-wide
   conflicts, and returns the journal input version. CLI, GUI and AI apply all
   require that version and persist the revalidated plan atomically.
   Gap review UI and AI now expose only `ui.custody.review.plan` and
   `ui.custody.review.apply` for create, dismiss, revise, reopen and residual
   actions; ten mutation-specific daemon/tool kinds and their duplicate routing,
   consent and cache-invalidation branches are deleted. The CLI exposes the
   same `transfers gaps plan/apply --action ...` contract, and all three clients
   consume one privacy-safe plan serializer owned by the review core. The former
   Python preview/create/reopen/revise/residual/dismiss compatibility wrappers
   are deleted; every mutation now enters through `apply_review`.
   The unused specialized component-create CLI/daemon path is deleted as well;
   one-component authoring uses the same pure version-gated batch plan/apply
   path as N:M authoring.
   The serialized and normalized gap-candidate caches are physically dropped
   on open and removed from new schemas, reset accounting, replication policy
   and workspace-split metadata. The latest journal builder ignored-boundary
   lists are migrated into one small local table; every review page recomputes
   the capped candidate population and uses an input-version-guarded in-memory
   ordinal/gap-id cursor.
   Capacity-limited gap discovery now returns `CustodyGapSearchResult` with
   explicit completeness, limit, partial-population and scoped-blocker fields;
   `CustodyGapSearchLimitError` and all exception-carried partial results are
   deleted.
   The custody-gap screen now mirrors that contract as one pending-plan state:
   create, reopen, revise and residual previews are mutually exclusive and all
   confirm through the same apply mutation. The obsolete reviewed-row branch
   is deleted from the current-candidate list; immutable review events remain
   on the point history endpoint. A real JSONL daemon regression follows the
   opaque normalized-candidate keyset cursor across pages.
   Component authoring now exposes distinct `components plan/apply` CLI,
   desktop and AI operations; the overloaded `bulk_resolve(dry_run=...)`
   command/kind is deleted, and apply requires the previewed input version.
   Component activation and supersession now use those same operations. Their
   plans are read-only, expose the current journal input version, and share the
   activation validator used by apply.
   The direct CLI commands, desktop daemon kinds and CLI-handler mutation
   wrappers are deleted; desktop create and lifecycle changes consequently
   share one consent, cache invalidation and stale-plan boundary.
   Immutable revision and undo operations now use the same boundary too. The
   planner assigns deterministic component, leg, allocation and economic-term
   identities without writes, retains hidden local leg evidence without
   returning it to the renderer, and carries migrated pair/payout policy and
   fee terms onto note-only/restored revisions. Removing a term-bound leg fails
   closed. The direct update/undo CLI commands, daemon kinds, handler
   orchestration and unused core undo convenience path are deleted.
   The remaining component list/get and batch plan/apply handler wrappers are
   deleted as well. CLI and daemon routing now resolve scope and call the core
   component store/planner directly; `cli.handlers` contains no component
   review orchestration.
   Create, revise, undo, activate and supersede now enter one strict core
   `plan_component_review` / `apply_component_review` action contract. CLI and
   daemon no longer maintain parallel lifecycle schemas or select among three
   planner implementations; UI and AI consequently revalidate through the
   same core dispatcher.
   The core custody journal service now owns replacement of stored journal
   entries, canonical quantity decisions/issues/balances, quarantines, tax
   summaries, holdings, processed-version metadata, migration finalization and
   filed-report impact resolution. The CLI layer retains only pre-build source
   overlap repair and cached-rate pricing until their independent transaction
   edit/audit hooks move in the next ordered slice.
   The same core service now owns the complete journal-processing transaction:
   tax-policy and sync-conflict gates, legacy-baseline capture, one savepoint,
   canonical build, stored-projection replacement, commit/rollback and the
   stable result contract. The CLI wrapper only injects the existing audited
   source-overlap repair/warning and cached-rate pricing preflights; it no
   longer decides custody behavior or coordinates projection persistence.
   The obsolete CLI `build_ledger_state` and rate-loader compatibility wrappers
   are deleted; report hooks, transfer audit and integration tests call the
   core service directly. The source-overlap integration test now creates a
   real active reviewed component through plan/apply instead of manufacturing
   a legacy `transaction_pairs` row.
   Pair and direct-payout review policy, conflict detection, component
   authoring/revision/deletion, journal invalidation and public projection now
   live in one core component-term service. `cli.handlers` retains only scope
   and transaction-reference resolution for these commands; it no longer
   contains their custody mutation rules. Bulk/rule matching and CLI, daemon
   and AI entry points consequently invoke the same component-native service.
   Complete economic term sets are now normalized and inserted atomically with
   their draft component. The separate create-then-seal mutation path is
   deleted, while the replicated expected-term-count commitment remains so a
   partial child-row replay still fails closed.
   Balance sheet, current portfolio, tax summary and exit-tax reports now read
   only the gated stored journal projection. Their live-builder fallbacks and
   the `ReportHooks.build_ledger_state` injection seam are deleted. Exit tax
   reconstructs exact quantities and fiat fields through the core stored-ledger
   loader, so an empty projection stays empty instead of silently reinterpreting
   custody during report rendering.
   The CLI transfer-audit endpoint now loads MOVE decisions, reviewed
   conversions and direct payouts from `journal_custody_decisions` and
   `journal_custody_economic_relations`. Its live build, four presentation
   adapters and chunked transaction-ref loader are deleted. Consequently the
   core journal-processing service is the only production caller of
   `build_ledger_state`.
   Partial conversion and direct-payout reviews now claim only their reviewed
   source slice. If the same reviewed action has exactly one same-asset inbound
   row with the same canonical event and exact residual amount, that residual
   is stored as an equal retained allocation in the same component; otherwise
   it remains outside the conversion and follows the ordinary external-presumed
   path. Mixed components carry unlike-asset economics and exact native
   retention on separate allocations, and native corroboration verifies the
   retained allocation without comparing it to the unsplit outbound total.
   Same-wallet failed-swap refunds remain one reviewed MOVE: the observed
   principal shortfall is an explicit reviewed fee allocation, while the
   source transaction's separately observed network fee remains independent.
   Journal processing counts transfers from that canonical projection rather
   than the engine's older native-match audit list.

Consumer cutover and physical legacy-table deletion are separate decisions.

## Acceptance gates

- authoritative A to B and 1:N observations produce exact internal MOVE;
- untrusted duplicate rows cannot override authoritative external physics;
- descriptor presence without current coverage cannot establish ownership;
- later authoritative synchronization self-heals deterministically;
- missing Whirlpool history creates a suggestion, never an automatic edge;
- a reviewed 10 BTC to 9.9 BTC bridge carries 9.9 and leaves 0.1 explicit;
- dismissal restores presumed disposal;
- stale plans cannot apply;
- unresolved quantities never enter `FinalizedTaxProjection` or RP2;
- reviewed N:M lineage is identical in reports, graphs and source-of-funds;
- reports and exit tax block until the rebuilt projection is current;
- reorg or stale observer authority invalidates a false internal move;
- every lineage row is reachable with indexed keyset pagination;
- every capped candidate is reachable with a version-guarded cursor, and a
  stale cursor fails closed after evidence changes.

End-to-end custody tests use real temporary databases and do not patch builder,
discovery, projection, or report-gate internals. Mocks are reserved for actual
external boundaries and explicit atomicity fault injection.

## Performance methodology and budgets

The benchmark separates four workloads: the real custody decision builder,
atomic arbitration, stored lineage pagination, and bounded gap discovery.
Wall-clock values are perf-lab observations until representative hosts and
variance are recorded; structural invariants remain blocking in CI.

Reference-host budgets, median of three cold runs:

| Workload | Budget |
| --- | ---: |
| custody decisions, 100k observations | 15s, 750 MiB peak RSS |
| custody decisions, 250k | 40s |
| custody decisions, 500k | 85s |
| custody decisions, 1m | 180s when practical |
| complete gap-discovery read, 100k | 5s |
| first lineage page at 1m | p95 100ms |
| subsequent keyset page at 1m | p95 50ms |
| transaction-scoped lineage | p95 25ms |

Hot query plans must use the ordering/scope indexes and avoid full scans or
temporary sorts. Doubling observations must not cost more than 2.3x after fixed
startup cost.

The first database-backed baseline on the merge reference host measured 100k
simple alternating BTC observations at 117.58s and 1,000,932 KiB maximum RSS
for custody decisions, with 100k observations, 50k decisions, zero issues, and
exact conservation. This intentionally fails the target and establishes the
optimization debt rather than weakening the budget.

After indexing target slices once, reusing the canonical observation input,
bounding the large-book discovery worklists, and avoiding irrelevant protocol
interpreters, the same host measured a three-run median of **13.60s** and
**755,284 KiB (737.6 MiB)** maximum RSS at 100k. The runs were 13.57s, 13.60s,
and 18.13s; the slower sample reflects host scheduling variance, while the
declared median remains below both blocking budgets. Every run projected 100k
observations into 50k outbound decisions with exact conservation and no
quantity issues.

The complete 100k scalability run measured atomic arbitration at 0.086s,
first/subsequent 100-row lineage pages at 5.3ms/3.9ms, and a transaction-scoped
lineage read at 0.45ms. SQLite selected
`idx_journal_custody_decisions_profile_time` for ordered pages and a
multi-index OR over the source and target transaction indexes for scoped reads,
with no temporary page sort. A full gap-discovery read completed in 54ms,
retained the structured 10 BTC out / 9.9 BTC return scenario, and reported
ordinary `capacity_limited` completeness rather than throwing or implying a
complete wallet universe.

A complete measured 250k run produced 125k decisions with zero issues in
**37.16s**, below the 40s budget. Peak RSS was 1,769,316 KiB; atomic arbitration
was 0.241s, first/subsequent lineage pages were 12.5ms/7.7ms, transaction-scoped
lineage was 0.41ms, and a full gap-discovery read was 0.173s. Because the
250k run already consumed about 1.7 GiB, 500k and 1m are documented rather than
claimed as measured on this host: linear extrapolation from the measured 250k
run is approximately 74.3s and 148.6s respectively, inside the time budgets,
but those estimates are non-blocking until measured on a host with sufficient
memory.

After deleting the persisted gap cache, a post-change 100k validation on
Python 3.13.5 / SQLite 3.46.1 measured the real builder at **9.357s** with
755,196 KiB peak RSS, atomic arbitration at 58.1ms, lineage pages at
2.60ms/2.17ms, transaction-scoped lineage at 0.21ms, and a complete derived
gap read at **33.6ms**. A focused 250k validation measured lineage pages at
4.39ms/3.98ms, transaction-scoped lineage at 0.21ms, and the recomputed gap read
at **83.9ms**. Both runs retained the structured 10 BTC / 9.9 BTC candidate,
reported capacity limitation explicitly, used the profile-time and source/
target lineage indexes, and avoided a temporary page sort.

## Stop state

The series stops when one authored reviewed-custody aggregate remains, only the
builder interprets transfers, suggestions cannot carry basis, previews do not
write, consumers read the same stored projection, candidate discovery is
versioned and genuinely paginated, compatibility interpretation is deleted,
production code volume is materially lower, and all functional, replication,
regtest, migration, performance-invariant, and repository quality gates pass.

## Final hard-stop audit (updated 2026-07-17)

The initiative remains justified after the merged-code inspection. Every
reported competing accounting path was either reproduced and removed or shown
to be a bounded migration/replication concern rather than a live authority.
The completed series is grouped below by coherent slice:

- journal ownership and ordinary gap-capacity state: `156122fc`, `56e611b4`,
  `9c786906`, `48c18e86`, `a42d6c8c`, `fe46d5cf`;
- normalization, exact allocation and claim arbitration: `eed37863`,
  `9ed58f7c`, `0e25df1a`, `3e3b502e`, `f3737c2e`, `14ced060`;
- one versioned gap projection and pure review plan/apply: `3b903070`,
  `139d97dc`, `f3518296`, `08c6c4b9`, `ffe3efac`, `1b3d8046`, `d2dcb9ca`,
  `19f94edc`, `c51e010b`, `9940ac8e`, `3c6fc2e9`, `3003c518`, `d18554bf`,
  `c1acfd35`;
- component-authored pair/payout migration and lifecycle convergence:
  `4f226eb2`, `4b75309c`, `b4cd82b2`, `5fb881ae`, `65eb4110`, `52204aef`,
  `495bcd53`, `e982376d`, `0616b5e5`, `8a8ba05e`, `372e4486`, `c11976e4`,
  `6073010f`, `11f3bf40`, `6e7f897a`, `a2513315`, `949d3874`, `68e67636`;
- stored-projection consumers, report gate and surface consolidation:
  `3c41deea`, `f149357e`, `fa4621de`, `c13015fe`, `ddbdea8d`, `04d50951`;
- speculative-scaffold deletion, performance and final correctness repairs:
  `a8131a8a`, `5c12439a`, `9e75efd3`, `9ec629ed`, `1dc8bae1`.
- post-review correctness and simplicity pass: `abce8e2d`, `cb5f578e`,
  `6f15448d`, `31d2d3c4`, `f33503c0`, `b4c64246`, `dbb60817`,
  `7b0bb442`.

The final production flow is the target flow above. Static call-site audit
finds `detect_intra_transfers` only in the custody interpreter and
`build_canonical_quantity_state` only in `CustodyJournalBuilder`. The runtime
has no discovery fallback. No `CUSTODY_CANDIDATE`, `HEURISTIC_CANDIDATE`,
`CustodyGapSearchLimitError`, preview savepoint or manufactured fallback claim
remains. Seven custody daemon operations replace the baseline fourteen:
coverage, lineage, gap list/context/history, and shared review plan/apply.
The final reachability pass also removed test-only effective-component, gap,
native-audit, evidence-baseline and filed-impact compatibility APIs. Tests now
enter through the same production seams. `REVIEWED_PAIR` claim priority and its
interpreter branches are deleted: reviewed pair/payout meaning can enter
arbitration only through an effective `REVIEWED_COMPONENT`; interpreter pairs
are strictly authoritative native evidence.

### Acceptance evidence

The acceptance scenarios are covered by real database-backed tests, with live
observers reserved for protocol physics:

- authoritative MOVE, 1:N/N:1 and hostile duplicate provenance:
  `test_authoritative_rowless_native_proof_projects_only_internal_move`,
  `test_rowless_native_fanout_and_consolidation_use_aggregate_target_slots`,
  `test_fanout_becomes_moves_with_deriver` and the two
  `test_untrusted_same_txid_*_cannot_suppress_external_disposal` regressions;
- current-coverage ownership and reorg invalidation:
  `test_source_technical_coverage_cannot_confirm_an_unknown_destination`,
  `test_anchor_coverage_mismatch_cannot_activate_even_when_manually_reviewed`,
  `test_lagging_backend_fails_before_reorg_rebuild`, plus the live BDK/LWK and
  Core/Elements observer lanes;
- missing Whirlpool suggestion, explicit 9.9/0.1 bridge/residual, dismissal,
  later sale barrier and exit-tax rebuild:
  `test_missing_whirlpool_review_carries_99_and_keeps_residual_and_sale_blocked`,
  `test_exit_tax_blocks_until_exact_missing_wallet_bridge_carries_basis`,
  `test_guided_cli_dismissal_uses_only_gap_identity` and the real JSONL guided
  lifecycle tests;
- stale plans, atomic N:M and source-of-funds lineage:
  `test_component_state_apply_rejects_stale_input_version`,
  `test_bulk_apply_rejects_a_stale_input_version`, component replication tests
  and `test_effective_nm_bridge_becomes_reviewed_source_funds_lineage`;
- fail-closed pre-tax projection and report barriers:
  `test_rp2_boundary_spy_never_receives_residual_or_later_basis_consumer`,
  `test_unresolved_quantity_never_enters_finalized_projection_and_blocks_later`
  and `test_quantity_issue_blocks_reports_and_appears_in_blocker_snapshot`;
- genuine candidate/lineage pagination with bounded recomputation:
  `test_snapshot_cursor_reaches_every_normalized_candidate`,
  `test_snapshot_cursor_expires_when_journal_input_changes`, the real JSONL
  opaque-cursor test and the 100k/250k scalability runs.

Exact validation on the final code:

- repository quality gate: 2,968 Python tests passed, 8 skipped; TypeScript
  compilation passed; ESLint had zero errors (49 pre-existing warnings);
  744 Vitest tests passed; shard and compile checks passed;
- fast integration harness: 39 passed; custody desktop harness: 22 Python and
  49 focused UI tests passed;
- live Bitcoin Core journal/export and Fulcrum parity lanes passed;
- the live all-observer lane passed independent Core/Elements truth manifests,
  BDK Esplora/Electrum restart parity, and LWK multi-asset restart/no-op/reorg;
- migration, delayed signed-replay, component replication, focused custody and
  swap-refund regressions all passed. Performance and query-plan results are
  recorded in the preceding section and meet every declared blocking budget.

### Migration window and rollback risk

The live schema contains 23 custody-related tables. The net increase from the
baseline 20 is component economic terms, durable migration-issue state, the
narrow targetless-economic relation projection, and the builder's local gap
input row, offset by deleting the serialized gap snapshot table. All three
normalized gap-candidate cache tables are also gone.
Legacy pair/payout rows remain physically present only until every signed
replica has acknowledged the component-native epoch or a tombstone protocol is
available. Applying an older binary after component-native writes is unsafe;
rollback requires the pre-migration backup. Removing those tables sooner could
lose delayed signed events. Removing immutable migration snapshots, revision
chains or filed-report history could break audit and amendment evidence.

### Code-volume result and retained exceptions

At the merge, the 17 `kassiber/core/custody*.py` modules contained 18,733
lines. PR #447 reached 25,707 lines before this follow-up audit; the current 20
modules contain **24,868 lines**, 839 fewer than that reviewed state. Across the
follow-up commits, production Python deleted 1,162 more lines than it added.
The branch therefore finishes with materially less code than the starting PR,
but it remains 6,135 custody-core lines above the pre-PR #439 baseline.

The earlier claim that no safe deletion remained was wrong. This pass removed
the gap cache and its three tables, full-book identity hashing, retention and
display-cache machinery, two-phase legacy staging, duplicated projection
semantics, divergent freshness readers, the plan fingerprint protocol, the
economic-term seal mutation and constant compatibility output. A fresh
top-level reachability scan found no unreferenced custody definition after
those removals.

The remaining proposed cuts were evaluated rather than silently waived:

- deterministic planned row IDs remain because apply replans from the reviewed
  action and must persist the same exact leg/allocation identities previewed;
- `expected_economic_term_count` remains because replicated child rows need a
  fail-closed completeness commitment, although terms now insert atomically for
  local authors;
- `legacy_source_id`/`source_row_hash` remain the immutable authored-review
  identity and audit commitment for both migrated and component-native pair or
  payout terms;
- `journal_custody_economic_relations` remains only for targetless reviewed
  conversions/payouts whose processed basis state cannot be reconstructed by
  joining authored terms; all target-bearing semantics come from the shared
  projection view;
- canonical observation revalidation in gap-hold compilation remains because
  the derived transaction matcher and authoritative observation substrate are
  distinct trust boundaries.

Removing those narrow pieces would weaken exact-plan review, replication
completeness, immutable audit identity, targetless economic classification or
authoritative provenance. The numerical pre-PR baseline is therefore still an
explicit unmet criterion, but the final state no longer contains the cache,
fingerprint or quadruplicated projection machinery identified by the review.
