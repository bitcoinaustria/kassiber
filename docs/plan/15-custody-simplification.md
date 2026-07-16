# Custody architecture simplification

Status: active follow-up to merged PR #439 (`76d907f6`). `TODO.md` remains the
execution backlog; this document records the invariant and migration boundary.

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

## Current competing paths

The merged path assembles custody in `kassiber.cli.handlers.build_ledger_state`:

1. load transactions, observer provenance, wallet policy, loan and channel
   state;
2. interpret native matches, transaction pairs, direct payouts and components;
3. discover gap candidates and compile quantity claims;
4. arbitrate quantity and compile finalized tax inputs;
5. invoke RP2 and persist journal projections.

Reports and the transaction graph may rerun transfer detection. Source-of-funds
separately interprets components, pairs, UTXO relations, and Lightning hashes.
The review queue runs gap discovery independently and caches serialized pages.

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
  `custody_filed_report_impact_resolutions`;
- `custody_tax_migration_baselines`,
  `custody_tax_migration_baseline_events`, and
  `custody_tax_migration_reports`.

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

Review preview is a pure `plan_review` operation. Apply persists exactly that
plan only after verifying its input version and deterministic fingerprint.

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
3. Replace candidate transfer claims with scoped holds; persist normalized
   candidates/completeness once per input version; introduce pure plan/apply.
   The first part is complete: promotion now emits independent source/return
   holds and typed issues, never a source-to-target claim. The normalized
   projection is also complete: accounting/review consumers share one
   input-version/ignored-boundary projection, completeness lives on its header,
   boundary relations are normalized, and presentation pages use indexed
   keyset rows without rerunning discovery. Planning is now one read-only
   `plan_review` seam for create/revise/reopen/residual actions; it commits the
   current journal input version, exact deterministic component rows and filed
   report impacts. One `apply_review` seam replans and rejects any fingerprint
   drift before performing the reviewed mutation. Existing CLI/daemon kinds
   are redacted compatibility wrappers over those two operations.
4. Add typed replicated component economic terms, migrate pair/payout authored
   state deterministically, and freeze legacy writes. Staging and activation
   are complete: each active legacy review receives deterministic immutable,
   replicated, leg-bound economics; full-source connected pair graphs
   consolidate into one atomic 1:N/N:M component and valid payout components
   activate directly. Partial-source legacy pairs stay on compatibility reads
   until their unreviewed tail can be authored as an explicit residual rather
   than silently upgraded from presumed disposal to reviewed classification.
   Journal interpretation now uses effective active components, while invalid
   or historically malformed rows stay on the compatibility interpreter and
   fail closed. Reopening is idempotent, including pre-432/pre-435 schema
   upgrades whose term foreign keys are rebuilt with their custody legs.
   Linked active legacy rows are write-frozen below the handlers; revision and
   deletion retire the active aggregate before changing compatibility history,
   and bypass or replication writes fail closed. Pair/payout lifecycle SQL now
   belongs to the core authored-review store, not CLI handlers. That store still
   emits frozen compatibility projections for readers not yet cut over; those
   projection writes disappear with the consumer migration before physical
   legacy deletion can be considered.
5. Cut reports, graph, source-of-funds, UI and AI to stored decisions/lineage;
   require a gated report context; delete compatibility interpretation,
   rollback previews, speculative layer scaffolding, and obsolete commands.
   In progress: MOVE decisions and non-quantity conversion/payout relations are
   stored together by the canonical projection replacement. Transaction-graph
   accounting annotations and report transfer labels read that projection;
   when it is stale they expose no provisional booked truth. Consumer-side
   calls to transfer detection have consequently been deleted, leaving the
   custody journal interpreter as the only production caller.

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
- every candidate and lineage row is reachable with indexed keyset pagination;
- fetching another page never regenerates discovery.

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
| candidate projection, 100k | 5s once per input version |
| first lineage/candidate page at 1m | p95 100ms |
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

## Stop state

The series stops when one authored reviewed-custody aggregate remains, only the
builder interprets transfers, suggestions cannot carry basis, previews do not
write, consumers read the same stored projection, candidate discovery is
versioned and genuinely paginated, compatibility interpretation is deleted,
production code volume is materially lower, and all functional, replication,
regtest, migration, performance-invariant, and repository quality gates pass.
