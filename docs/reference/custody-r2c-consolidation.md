# R2c consolidation production checkpoint

Branch `t3code/r2c-interpretation-8` starts at ownership commit `98cb0fb2`.
This checkpoint ports consolidation before the separate producer-composition
slice. Python remains authoritative, snapshot v1 carries raw facts only, and
`interpretation` remains in the product's `skipped_stages`.

## Production and comparison boundary

`interpretation/consolidation.rs` ports `derive_multi_source_consolidations`
and `_complete_collaborative_destinations`. It retains first-seen scope admission,
synthetic-row exclusion, handled-group reservations, unique funding wallets,
merged-graph and component admission, normalized versus legacy fees, exact own
wallet deltas within collaborative Bitcoin events, scoped input/output ownership,
complete destination admission and ordered exact-msat FIFO cells. It reuses the
ownership producer's typed graph, integer, asset, index and priced-row helpers.
There are no dependencies, new timestamp parsers or raw `codec::wide()` calls.
CPython case helpers and the corrected shared timestamp parser remain in use.

The partial seam returns complete derivations, generic claims and rowless native
audits for independent Python/Rust comparison. It reserves no other producer's
rows. Direct vectors pin explicit handled-id behavior and Python's conversion,
allocation and decimal exception families. These vectors supply normalized
producer rows separately from their raw snapshot input; computed fee flags are
never snapshot facts. The raw-snapshot projection vectors use only loadable raw
facts. No partial result is product accounting authority.

There are **116 direct producer witnesses**, **18 collaborative helper witnesses**
and **31 raw-snapshot projection witnesses**. The complete partial fixture has
201 snapshots. Named Rust tests are `consolidation_production_vectors`,
`consolidation_collaborative_vectors` and
`interpretation_consolidation_projection`. Python regenerates and verifies every
frozen expectation independently. The direct seam can exercise integer strings,
unbounded arithmetic and malformed rows outside product snapshot admission;
it does not change snapshot admission.

## Successful producer controls

Every cell below is asserted separately, including zero producer blockers and
zero compiler blockers/quarantines. Consolidation successes have two valid
claims. Ownership successes have one native audit.

| Graph scenario | Python successes | Rust successes |
| --- | ---: | ---: |
| `bitcoin_consolidation` | 250 | 250 |
| `liquid_consolidation` | 250 | 250 |
| `bitcoin_ownership` | 250 | 250 |
| `liquid_ownership` | 250 | 250 |
| `liquid_rawhex` | 250 | 250 |
| `liquid_rawhex_short_locktime` | 250 | 250 |
| `liquid_rawhex_flag` | 250 | 250 |
| `liquid_rawhex_nonminimal` | 250 | 250 |
| Total | **2,000** | **2,000** |

The six generated suites compare 12,000 seeded requests. Configured main
permutations are evidence 58, components 2,000, Samourai 68, recorded pairs 92,
fan-out 94 and graphs 16. Existing activity controls remain evidence 345
reconciled books/690 receipts, Samourai 1,176 active/332 blocked/492 inactive,
recorded pairs 789/176/1,035 and fan-out 1,244/126/630.

## Mutation and branch accounting

| Mutation group | Entries | Named-test result |
| --- | ---: | --- |
| Retained decoder/graph/interpreter/component/evidence rules | 389 | All killed |
| Retained stored-mapping admission rules | 12 | All killed |
| Retained graph-arithmetic rules | 37 | All killed |
| Retained ownership production/helpers | 120 | All killed |
| Consolidation production/helpers | 84 | All killed |
| Total | **642** | **All killed** |

The retained catalog runs in three disjoint 186-entry scratch trees. New rules
run in two disjoint 41-entry trees plus two final identity/reference rules.
Every selection first passes unmutated. Earlier survivors exposed masked
checks: single-source conservation, duplicate source-wallet ownership, an
unknown additional component input, zero sinks, exact tolerance boundaries,
collaborative unknown values and independently nonconserving own deltas. Direct
helper witnesses distinguish an earlier refusal from a later equivalent review.
No survivor or nonbuilding mutation is counted as a kill. Exact mutation IDs,
patches and named tests are in `crates/custody/scripts/mutations.json`.

Shared parser, scope, ownership, synthetic-prefix, casing, integer, pricing and
claim-compiler branches retain their existing killed entries. These internal
branches do not require separate new decisions:

- Parsed positions and sat quantities are integers or absent by construction;
  JSON cannot supply Decimal objects. Unknown values are independently tested.
- Fee sets cannot be empty after admitting at least two senders. Their sole
  element's iteration order is irrelevant in legacy mode.
- The collaborative helper's empty-positive-row guard cannot fire at its
  production call: ordinary senders have positive quantities, while a population
  consisting only of normalized fee-only senders has a positive total fee and
  cannot enter the zero-fee collaborative path.
- Empty component inputs fail the preceding requirement that every sender fund
  an input. The wholly owned input loop's nonempty guard is therefore dominated.
- Destination keys are unique after aggregation. FIFO capacities are integers;
  positive sink admission precedes allocation, and exact conservation proves
  sufficient total capacity. Duplicate/empty source ids and negative source
  capacity retain the allocation exception family and have mutation witnesses.
- Only the first-cell source offset affects fees; sink offsets are not read by
  this producer. The FIFO loop omits unused sink-coordinate/result objects,
  while preserving cell order and quantities. Zero-capacity sources emit no cell.
- Shared clone helpers preserve outbound/inbound kinds, directions, anchors,
  wallet references and pricing. New consolidation mutations additionally pin
  prefixes, suffixes, canonical group IDs, exact cell amounts, source anchors,
  atomic destination rows and both synthetic/dropped row populations.

## Verification

Tooling uses `mise exec rust@1.99.1 uv@latest -- ...`,
`MATURIN_NO_INSTALL_RUST=1`, locked Python 3.11 and offline Cargo, including
child builds through `CARGO_NET_OFFLINE=true`. Environments, books, recordings
and mutation scratch trees use disposable `/tmp` roots.

- Formatting and workspace/all-target Clippy with warnings denied pass.
  `cargo test --workspace --exclude kassiber-net --locked --offline` passes
  **716 tests**. The catalog's **642 entries** all have named-test kills;
  final applicability validation passes.
- The six generated suites pass **12,000 requests**, their configured
  permutations and the per-scenario producer/activity controls above.
- The custody Python modules plus ownership/transfers/reviews, historical
  compatibility and onchain modules report **772 passed and 2,679 subtests
  passed, with four failures**. Census compares components, evidence and all
  seven partial projections on 214 successful snapshots without a difference;
  `interpretation` remains skipped on all 215 observed builds.
- The failures are the two `CustodyComponentDaemonSurfaceTests` daemon tests
  and the two real-JSONL `CustodyGapDaemonProtocolAcceptanceTests`, as in the
  previous handoff. An independent single-test probe confirms the daemon's
  `internal_error`: the stable owner-lock runtime `~/.kassiber/run` is read-only
  in this sandbox. No owner guard is weakened. The orchestrator needs a
  writable runtime for these tests.
- Both the earlier and fresh recording sets pass integrity and replay:
  **206 requests and 206 permutations per set**, zero pre-request refusals
  and zero differences.
- Final inventory/contracts/frozen checks pass **15 tests and 14 subtests**.
  Shared JSON-feature verification reports **145 byte-identical ledger outputs**.

Full census, custody-shadow shards and the full quality gate belong to the
orchestrator; this checkpoint does not claim them. No push or PR is made.

## Remaining slices

1. Compose consolidation -> recorded fan-out -> ownership using Python's exact
   row replacement, dropping, synthetic-row and handled-id rules. Compare
   residual claim annotations and event-wide producer-block quarantine expansion,
   and add per-scenario successful composed controls for both engines.
2. Port raw-observation holds/privacy, channel lifecycle and loans, native HTLC
   transition production, native component reconciliation and final interpreter
   composition. Keep `interpretation` skipped until these are complete.
3. Port gaps, quantity arbitration and finalized projection before joining
   custody with `ledger`.
