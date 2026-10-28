# R2c ownership production checkpoint

Branch `t3code/r2c-interpretation-7` starts at `48c5b585`. This slice ports
ownership production and stops at its own verified commit. Consolidation has
not started. Python remains authoritative; snapshot v1 holds raw facts only,
and the product still lists `interpretation` in `skipped_stages`.

## Production and partial boundary

`interpretation/ownership.rs` ports `derive_ownership_transfers`, including the
index-free Liquid shortfall barrier, missing funding-wallet checks, duplicate
economic observations, component-asset selection, per-asset conservation,
source ownership and narrow historical fallback, exact folded fees, destination
aggregation and reuse/decline/synthesis, atomic receipt rollback, split pricing,
external residuals and dropped source rows. Quantities and output positions stay
unbounded after Python integer admission. Raw facts never use `codec::wide()`.

The scoped index, parser and native-delta checks reuse the audited graph
prerequisites. Profile merging now exposes typed `kassiber-pyjson` graphs to the
producer, retaining nonfinite scalars and distinguishing literal tagged objects.
CPython case helpers remain in use. No new timestamp parser is introduced;
the shared compiler keeps the corrected timestamp implementation. Row cloning
shares the already tested fan-out pricing helper. No dependencies change.

The isolated partial seam returns the complete derivation plus generic pair
claims and rowless native audits. It reserves no other interpreter's rows;
explicit handled-id behavior is tested at the direct producer boundary. Raw
`blocked_sources` are compared verbatim inside the derivation. Their expansion
into event-wide quarantines, residual claim annotations, normalized-fee inputs,
other interpreter reservations and row-pipeline interactions remain composition
work. No partial result authorizes accounting in the product.

The frozen corpus adds **155 direct producer witnesses**, **54 helper
witnesses** and **39 raw-snapshot projection witnesses**. Direct cases outside
snapshot admission pin conversion/error ordering and unbounded arithmetic;
they do not weaken the snapshot's signed-64 msat admission. The full partial
fixture has 170 snapshots. Named tests are `ownership_production_vectors`,
`ownership_helper_vectors` and `interpretation_ownership_projection`.

## Generator production controls

Every count below is asserted per scenario. Success means one derived pair,
no producer blockers, one native audit and no compiler blocker/quarantine for
the ownership scenarios. The graph generator also compares all raw partial
results, including inactive ownership on consolidation books.

| Graph scenario | Python ownership successes | Rust ownership successes |
| --- | ---: | ---: |
| `bitcoin_ownership` | 250 | 250 |
| `liquid_ownership` | 250 | 250 |
| `liquid_rawhex` | 250 | 250 |
| `liquid_rawhex_short_locktime` | 250 | 250 |
| `liquid_rawhex_flag` | 250 | 250 |
| `liquid_rawhex_nonminimal` | 250 | 250 |
| Total | **1,500** | **1,500** |

`bitcoin_consolidation` and `liquid_consolidation` each retain **250 successful
Python consolidation controls** (two claims per success). A Rust consolidation
producer is not present. Those counts must not be reported as Rust production.

The six generated suites pass **12,000 seeded requests** and their configured
main permutations: evidence 58, components 2,000, Samourai 68, recorded pairs 92,
fan-out 94 and graphs 16. Existing scenario activity assertions also pass:
evidence 345 reconciled books/690 receipts; Samourai 1,176 active/332 blocked/492
inactive; recorded pairs 789/176/1,035; fan-out 1,244/126/630.

## Branch and mutation accounting

| Mutation group | Entries | Result |
| --- | ---: | --- |
| Retained decoder/graph/interpreter/component/evidence rules | 389 | All killed |
| Retained stored-mapping admission rules | 12 | All killed |
| Retained graph-arithmetic rules | 37 | All killed |
| Ownership production and helpers | 120 | All killed |
| Total | **558** | **All killed** |

The four initial disjoint scratch runs check 535 entries and kill all 438
retained rules, while exposing 13 ownership survivors. Additional witnesses
pin empty input selections, wrong-scope evidence with independent fallbacks,
Liquid symbol aliases, amount-tolerance boundaries, synthetic invalid amounts,
positive candidate admission, incomplete external quantities and receipt
rollback. A duplicate-destination mutation now changes its decision; merely
removing that early return is equivalent to the later ambiguity check.
All 106 ownership rules in the next four disjoint runs are killed (27/27/26/26).
Eight blocker-payload/synthetic-prefix rules, the wallet-reference precedence
rule and the first-seen group-order rule are also killed. Manual review caught
the wallet-reference edge: Python retains the last row for a repeated id. The
Rust lookup now does the same. An indexed ordered vector preserves first-seen
scope admission without a linear scan over all previous groups.
Four final output-completeness, ambiguity, ordinary-payment and atomic-staging
rules also have named-test kills. Every run proves its named tests pass
unmutated. No survivor or nonbuilding mutation is counted as a kill. Final
catalog validation applies all 558 entries.

The table below maps the port's decisions to named tests. Existing parser,
index, graph-scope, integer, decimal-pricing and generic compiler branches
retain their previous catalog entries. These internal branches need no new
independent mutation:

- Parsed `value_sats` and output positions are integers or absent by parser
  construction; JSON cannot supply Decimal instances. The producer cannot
  observe noninteger parsed positions.
- Strict source fallback's direction, positive amount and source-wallet equality
  guards are already established by the caller. The input-count, missing-owner,
  outpoint and scoped txid decisions have independent witnesses.
- Exact-fee graph-conflict and empty-side guards are dominated by earlier
  producer checks; helper tests nevertheless kill their mutations. Likewise,
  an unknown owned change is normally rejected by output routing before the
  conservation helper; its direct helper witness pins the same refusal.
- The destination slot's stored wallet label is never read by Python, so Rust
  omits it. Parsed output indices are unique; equal minimum indices for two
  different destination wallets cannot reach routing without a graph conflict.
- Producer staging always appends an outbound row with each successful pair.
  A successful nonempty destination loop cannot end with an empty pair list.
  Empty/declined loops and all receipt-release paths are witnessed.
- Fake/legacy ownership-index interfaces remain outside decoded snapshot v1,
  as in the graph prerequisite checkpoint. `None` versus an empty decoded
  index remains observable and is tested.

| Mutation | Named Rust test |
| --- | --- |
| `R2c-ownership-asset-id-case` | `ownership_helper_vectors` |
| `R2c-ownership-asset-display-case` | `ownership_helper_vectors` |
| `R2c-ownership-asset-row-fallback` | `ownership_helper_vectors` |
| `R2c-ownership-asset-id-equality` | `ownership_helper_vectors` |
| `R2c-ownership-asset-unknown-id` | `ownership_helper_vectors` |
| `R2c-ownership-asset-display-equality` | `ownership_helper_vectors` |
| `R2c-ownership-asset-unknown-display` | `ownership_helper_vectors` |
| `R2c-ownership-input-selection` | `ownership_helper_vectors` |
| `R2c-ownership-input-unknown` | `ownership_helper_vectors` |
| `R2c-ownership-input-different` | `ownership_helper_vectors` |
| `R2c-ownership-input-nonempty` | `ownership_helper_vectors` |
| `R2c-ownership-conservation-input-value` | `ownership_production_vectors` |
| `R2c-ownership-conservation-change-value` | `ownership_helper_vectors` |
| `R2c-ownership-conservation-change` | `ownership_production_vectors` |
| `R2c-ownership-conservation-exact` | `ownership_production_vectors` |
| `R2c-ownership-fee-conflicts` | `ownership_helper_vectors` |
| `R2c-ownership-fee-empty-bitcoin` | `ownership_helper_vectors` |
| `R2c-ownership-fee-bitcoin-sign` | `ownership_helper_vectors` |
| `R2c-ownership-fee-liquid-presence` | `ownership_helper_vectors` |
| `R2c-ownership-fee-liquid-role` | `ownership_helper_vectors` |
| `R2c-ownership-fee-liquid-id` | `ownership_helper_vectors` |
| `R2c-ownership-fee-liquid-display` | `ownership_helper_vectors` |
| `R2c-ownership-fee-liquid-unique` | `ownership_helper_vectors` |
| `R2c-ownership-fee-liquid-sign` | `ownership_helper_vectors` |
| `R2c-ownership-graphless-exact` | `ownership_production_vectors` |
| `R2c-ownership-graphless-other-wallet` | `ownership_production_vectors` |
| `R2c-ownership-graphless-asset` | `ownership_production_vectors` |
| `R2c-ownership-graphless-scope` | `ownership_production_vectors` |
| `R2c-ownership-contradiction-outpoint` | `ownership_production_vectors` |
| `R2c-ownership-contradiction-script` | `ownership_production_vectors` |
| `R2c-ownership-contradiction-scope` | `ownership_production_vectors` |
| `R2c-ownership-contradiction-txid-source` | `ownership_production_vectors` |
| `R2c-ownership-contradiction-txid-scoped` | `ownership_production_vectors` |
| `R2c-ownership-source-owner-set` | `ownership_production_vectors` |
| `R2c-ownership-source-fallback-count` | `ownership_production_vectors` |
| `R2c-ownership-source-fallback-absent` | `ownership_production_vectors` |
| `R2c-ownership-source-fallback-txid` | `ownership_production_vectors` |
| `R2c-ownership-destination-consumed` | `ownership_production_vectors` |
| `R2c-ownership-destination-handled` | `ownership_production_vectors` |
| `R2c-ownership-destination-asset` | `ownership_production_vectors` |
| `R2c-ownership-destination-positive` | `ownership_production_vectors` |
| `R2c-ownership-destination-exact` | `ownership_production_vectors` |
| `R2c-ownership-destination-scope` | `ownership_production_vectors` |
| `R2c-ownership-destination-reuse` | `ownership_production_vectors` |
| `R2c-ownership-destination-duplicate` | `ownership_production_vectors` |
| `R2c-ownership-destination-different-tx` | `ownership_production_vectors` |
| `R2c-ownership-destination-tolerance-floor` | `ownership_production_vectors` |
| `R2c-ownership-destination-tolerance-percent` | `ownership_production_vectors` |
| `R2c-ownership-destination-tolerance-boundary` | `ownership_production_vectors` |
| `R2c-ownership-clone-in-direction` | `ownership_production_vectors` |
| `R2c-ownership-clone-in-wallet` | `ownership_production_vectors` |
| `R2c-ownership-clone-ref` | `ownership_production_vectors` |
| `R2c-ownership-clone-in-kind` | `ownership_production_vectors` |
| `R2c-ownership-clone-anchor` | `ownership_production_vectors` |
| `R2c-ownership-no-index-liquid` | `ownership_production_vectors` |
| `R2c-ownership-no-index-handled` | `ownership_production_vectors` |
| `R2c-ownership-missing-recorded-sources` | `ownership_production_vectors` |
| `R2c-ownership-missing-handled` | `ownership_production_vectors` |
| `R2c-ownership-missing-authority` | `ownership_production_vectors` |
| `R2c-ownership-missing-source-self` | `ownership_production_vectors` |
| `R2c-ownership-missing-source-delta` | `ownership_production_vectors` |
| `R2c-ownership-missing-source-unknown` | `ownership_production_vectors` |
| `R2c-ownership-synthetic-inbound` | `ownership_production_vectors` |
| `R2c-ownership-duplicate-group-handled` | `ownership_production_vectors` |
| `R2c-ownership-duplicate-key-fee` | `ownership_production_vectors` |
| `R2c-ownership-source-handled` | `ownership_production_vectors` |
| `R2c-ownership-source-synthetic` | `ownership_production_vectors` |
| `R2c-ownership-source-positive` | `ownership_production_vectors` |
| `R2c-ownership-source-graphless-liquid` | `ownership_production_vectors` |
| `R2c-ownership-asset-complete` | `ownership_production_vectors` |
| `R2c-ownership-graph-conflict` | `ownership_production_vectors` |
| `R2c-ownership-source-scope-contradiction` | `ownership_production_vectors` |
| `R2c-ownership-external-quantity` | `ownership_production_vectors` |
| `R2c-ownership-external-completeness` | `ownership_production_vectors` |
| `R2c-ownership-output-other-asset` | `ownership_production_vectors` |
| `R2c-ownership-output-change` | `ownership_production_vectors` |
| `R2c-ownership-output-ambiguous` | `ownership_production_vectors` |
| `R2c-ownership-output-quantity` | `ownership_production_vectors` |
| `R2c-ownership-output-first-index` | `ownership_production_vectors` |
| `R2c-ownership-duplicate-blocker` | `ownership_production_vectors` |
| `R2c-ownership-source-ownership` | `ownership_production_vectors` |
| `R2c-ownership-inclusive-fee-path` | `ownership_production_vectors` |
| `R2c-ownership-explicit-fee-total` | `ownership_production_vectors` |
| `R2c-ownership-implicit-attribution` | `ownership_production_vectors` |
| `R2c-ownership-conservation-blocker` | `ownership_production_vectors` |
| `R2c-ownership-owned-capacity` | `ownership_production_vectors` |
| `R2c-ownership-inclusive-gap` | `ownership_production_vectors` |
| `R2c-ownership-inclusive-fee-capacity` | `ownership_production_vectors` |
| `R2c-ownership-fee-budget` | `ownership_production_vectors` |
| `R2c-ownership-destination-order` | `ownership_production_vectors` |
| `R2c-ownership-leg-positive` | `ownership_production_vectors` |
| `R2c-ownership-leg-fee-once` | `ownership_production_vectors` |
| `R2c-ownership-rollback` | `ownership_production_vectors` |
| `R2c-ownership-group-id` | `ownership_production_vectors` |
| `R2c-ownership-residual-amount` | `ownership_production_vectors` |
| `R2c-ownership-residual-fee` | `ownership_production_vectors` |
| `R2c-ownership-drop-source` | `ownership_production_vectors` |
| `R2c-ownership-component-display-precedence` | `ownership_helper_vectors` |
| `R2c-ownership-relation-bitcoin` | `ownership_helper_vectors` |
| `R2c-ownership-inputs-bitcoin` | `ownership_helper_vectors` |
| `R2c-ownership-conservation-bitcoin` | `ownership_helper_vectors` |
| `R2c-ownership-conservation-other-change` | `ownership_helper_vectors` |
| `R2c-ownership-liquid-chain` | `ownership_helper_vectors` |
| `R2c-ownership-liquid-component` | `ownership_helper_vectors` |
| `R2c-ownership-liquid-symbol` | `ownership_helper_vectors` |
| `R2c-ownership-index-absence` | `ownership_production_vectors` |
| `R2c-ownership-block-complete-scope` | `ownership_production_vectors` |
| `R2c-ownership-block-wallet-label` | `ownership_production_vectors` |
| `R2c-ownership-block-wallet-fallback` | `ownership_production_vectors` |
| `R2c-ownership-synthetic-prefix-owned-derive` | `ownership_production_vectors` |
| `R2c-ownership-synthetic-prefix-recorded-fanout` | `ownership_production_vectors` |
| `R2c-ownership-synthetic-prefix-cross-split` | `ownership_production_vectors` |
| `R2c-ownership-synthetic-prefix-direct-payout` | `ownership_production_vectors` |
| `R2c-ownership-synthetic-prefix-multi-consol` | `ownership_production_vectors` |
| `R2c-ownership-wallet-ref-last` | `ownership_production_vectors` |
| `R2c-ownership-source-group-order` | `ownership_production_vectors` |
| `R2c-ownership-owned-output-incomplete` | `ownership_production_vectors` |
| `R2c-ownership-owned-output-ambiguity` | `ownership_production_vectors` |
| `R2c-ownership-ordinary-no-destination` | `ownership_production_vectors` |
| `R2c-ownership-atomic-leg-staging` | `ownership_production_vectors` |

## Verification

Tooling uses `mise exec rust@1.99.1 uv@latest -- ...`,
`MATURIN_NO_INSTALL_RUST=1`, locked Python 3.11 and offline Cargo (including
child builds via `CARGO_NET_OFFLINE=true`). The locked uv environment, test
books, recordings and mutation trees live under disposable `/tmp` roots.

- `cargo fmt --all --check`, workspace/all-target Clippy with warnings denied,
  and `cargo test --workspace --exclude kassiber-net --locked` pass:
  **713 Rust tests**. Cargo invocations also use `--offline`.
- All **558 mutations** have named-test kills as detailed above; catalog
  validation passes.
- Generated comparisons pass **14 tests**, exercising the 12,000 requests and
  the per-producer/per-scenario controls above. The first earlier partial-only
  run also passed seven tests. Final frozen/snapshot/regression/inventory and
  contracts verification passes **43 tests and 2,583 subtests**. An intermediate
  helper fixture equality check exposed a tuple/list projection mismatch in
  the test generator; its correction and the entire focused selection pass.
- The custody Python modules plus ownership, ownership transfers/reviews,
  historical compatibility and onchain modules report **758 passed and 2,679
  subtests passed, with four failures**. Census compares components, evidence
  and all six partial projections on **214 successful snapshots**, without a
  custody difference. Interpretation remains skipped on all 215 observed
  builds. The generated modules run separately above.
- The four failures are the two `CustodyComponentDaemonSurfaceTests` daemon
  tests and the two real-JSONL `CustodyGapDaemonProtocolAcceptanceTests` tests,
  unchanged from the prerequisite checkpoint. The stable account-owned runtime
  root uses the passwd home and cannot be redirected by a disposable HOME or
  XDG root. In this sandbox `~/.kassiber/run` is read-only. No owner guard is
  weakened; the orchestrator must run these tests with a writable runtime.
- Both the previous 206-snapshot recording set and this run's fresh
  206-snapshot set pass integrity checks. Each set replays **206 requests and
  206 permutations**, with zero pre-request refusals and zero differences.
- Inventory validation and its focused contracts/mutation tests pass. The
  shared JSON-feature comparison reports **145 ledger outputs byte-identical**.

Full census, custody-shadow shards and the full quality gate are not claimed
here; the orchestrator owns the census/shadow lanes. No push or PR is made.

## Remaining slices

1. Port multi-source consolidation and verify its own successful Bitcoin and
   Liquid controls before committing it separately. Retain collaborative
   own-delta qualification, normalized versus legacy fee treatment, complete
   recorded destinations and exact FIFO source/sink cells.
2. Compose consolidation -> recorded fan-out -> ownership using
   `derive_profile_transfers`' row replacement, dropping, synthetic-row and
   handled-id rules. Add successful composed controls in both engines, residual
   claim annotations and event-wide producer-block quarantine comparison.
3. Port raw-observation holds/privacy, channel lifecycle and loans, native
   HTLC transition production, native component reconciliation and final
   interpreter composition. Keep `interpretation` skipped until then.
4. Port gaps, quantity arbitration and finalized projection; only then fuse
   custody with `ledger`.
