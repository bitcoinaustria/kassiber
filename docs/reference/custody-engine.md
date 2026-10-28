# Custody engine

R2 is a parity port, described in [plan 21](../plan/21-rust-backend.md).
Python remains the production reference. Rust receives a versioned, local,
raw-fact request through `kassiber_tax.custody`, alongside the independent
`ledger` operation. A custody result is never authority to modify observations
or authored components. The tax engine still accepts only the finalized tax
projection.

## Request schema 1

`kassiber.core.custody_snapshot.load_custody_snapshot` is the sole loader for
both computation and the test recorder. It runs in the journal transaction
after overlap repair, legacy component refresh and auto-pricing. Direct builder
callers retain their existing transaction ownership. The loader performs no
writes or external I/O. Every quantity remains an integer msat; fiat exact
columns remain decimal strings and historical REAL columns retain their float
semantics. Arrays preserve database order, including unordered reads. Latest-rate precedence
is `occurred_at DESC, created_at DESC, id DESC`; R2a deliberately adds the id
tie-break to the Python reference query and snapshot loader together. Rust
receives this ordered raw list; rates are still an unported stage.
Unknown schema versions and operations are refused.

| Field | Raw facts and replaced database reads |
| --- | --- |
| `schema_version`, `operation` | `1`, `custody` |
| `as_of` | Explicit RFC3339 build instant; pair-dismissal expiry never reads a clock in the pure path |
| `profile` | Profile facts, including scope, policy and journal input version |
| `book_network` | Nullable gate facts: stored binding, all network observations including excluded rows, reference chain/network/payload facts from chain analysis observations, observer instances, observation provenance, policy epochs and UTXOs; epoch private-material/source **presence** only. Replaces `require_book_accounting` and its inventory reads without copying private policy material |
| `wallets` | Database-order wallet id/label/kind/account refs and normalized configuration facts |
| `transactions` | Active `t.*` rows in `occurred_at, created_at, id` order, wallet/account facts and the joined `observation_*` provenance columns. No closed authority object is supplied as an input |
| `rate_rows` | Raw rate/value/exact/amount columns in descending occurrence/creation/id order, with the legacy fallback when exact columns are absent. Replaces latest-rate SQL, not its decimal computation |
| `owned_index` | Null when no active row has provenance. Otherwise script/outpoint/txid maps, including chain/network scoped maps, restricted to references in active rows' stored graphs. Match lists keep insertion order. Ownership policy derivation and its inventory/history/checkpoint reads stay at loading; descriptors, xpubs and private material do not cross the seam |
| `ownership_warnings` | Non-fatal index-loader diagnostics |
| `components` | Authored-active headers **as stored**, ordered legs, allocations, economic terms and evidence commitments. No materialized state, validation, inferred allocations or blockers |
| `anchors` | Map from every transaction/anchor id referenced by authored legs to its joined transaction facts, including excluded anchors; explicit null for missing anchors |
| `anchor_wallets` | Wallet scope facts for all named leg wallets, including wallets outside the active row population |
| `revision_headers` | Stored revision-link targets, including targets outside this profile (needed to diagnose wrong-scope links); absent target ids remain on their referring headers |
| `receipt_sources` | Finite `SourceScript` facts from `build_profile_source_index`, loaded only when a Bitcoin event has receipts in multiple wallets and restricted to those events' output scripts. Replaces wallet configuration, UTXO, descriptor-target and freshness-checkpoint reads; canonical owner recommendations and reconciled amounts are recomputed |
| `channel_records` | Database-order channel records with wallet configuration facts and backend chain/network, before lifecycle roles or transfer pairs are decided |
| `loan_legs` | Database-order nondeleted transaction ids and roles |
| `evidence_snapshots` | Stored author-local component activation snapshots, ordered by subject/quantity/detail hashes |
| `migration_issues` | Unresolved stored migration issues, ordered by creation/id; quarantines are recomputed |
| `gap_reviews` | Raw latest review rows, including every sibling at the maximum revision; the pure Python stage applies concurrent-review and `bridge_reopened` rules |
| `pair_dismissals` | Database-order stored pair-dismissal rows, including expiry |
| `gap_worklist` | `large_book` and ordered raw rows selected by existing gap SQL. Small books carry the complete active worklist; large books retain the bounded structured/high-value source and amount-range return selection |

Configuration JSON is reduced to `chain`, `network`, `bitcoin_network`,
`chain_network`, `chain_instance_id`, and the Samourai block's `role`, `group_id`
and `section`. `config_text_mentions_samourai` preserves the old substring gate
without retaining its text. All row copies and Rust responses obey this same
boundary, including finalized rows and intra pairs. Component `location_ref`
remains a validation input, but is omitted from materialized components as before.

Component repository selectors operate over these raw collections. They do not
parse or replay SQL. The evidence-commitment and native-support reads deliberately
omit observer provenance, matching the original queries; granting authority in
those reads changes current Python semantics. Invalid authored-active headers
remain inputs and their arrived anchors remain blocked. No database read is
allowed during pure computation.

## Stages and response schema 1

The Python stages are `components`, `evidence`, `interpretation`, `gaps`,
`quantity`, and `finalized_projection`. They expose materialization/blockers;
enriched rows, receipt reconciliation and canonical quantities; interpreter
compilation; gap search; quantity arbitration; and finalized tax inputs.
Warnings and both advisory/accounting gap-ignored id sets remain outputs.

The Rust response has `schema_version`, `engine_version`, `stages` (implemented
stage names mapped to results), and `skipped_stages` (explicit unported names).
A refusal has `error` with a stable code and message and cannot be interpreted as
a partial successful stage. Persisted hashes always use CPython-compatible JSON
from `kassiber-pyjson`; decimal arithmetic uses `kassiber-pydecimal`.

Rust currently implements `components` followed by `evidence`. The component
result is `{"components": [...], "blockers": [...]}` with the exact Python
materialization and integrity blockers. Its invalid authored-active rows stay
visible with `effective_state="draft"`; their anchor quarantine and readiness
barriers remain intact. Interpretation, gaps, quantity and finalized projection
are explicitly skipped. Schema 1's existing raw component/anchor/wallet/revision
and transaction facts suffice; R2b adds no input field or Python verdict.

## Test-only recording and comparison

`KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR` records an envelope whose `input` is exactly
the v1 request above. Stages and outputs are separate; they never become Rust
inputs. The envelope records the reference commit, schema, input digest, test
origin and comparison policy. Only synthetic books belong in committed fixtures.
Owner-book recordings stay local and ephemeral. Both `.json` and mismatch
`.replay` files are exclusively created with mode 0600, independent of umask.
Lookup auditing queries the full ownership index independently of the restricted
index; even an empty restricted lookup is checked against the full result. The recorder checks for wallet
key material before writing. `KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1` replays the
request in a fresh process under another hash seed, with no database image.

`KASSIBER_TEST_CUSTODY_ENGINE=shadow|census` is installed only by test plumbing,
including Python child processes. Both modes compare implemented Rust stages;
shadow additionally compares the frozen SQL composition, while census preserves
production read-budget measurements. Stage differences, refusal-code differences and binding failures are
recorded before raising, so catching an assertion cannot make the session pass.
The required `custody-shadow` CI job and quality gate run
`scripts/python_test_shards.py run-custody-shadow`. Only
`test_disjoint_reviewed_routes_limit_observation_reads` is deselected there:
its single-build budget is enforced in census, while shadow intentionally runs
a second frozen SQL build. A missing binding or an empty implemented-stage
inventory fails; no-build test lanes have no stage to compare.
CI partitions the complete overlay by the existing shard inventory with
`run-custody-shadow <shard> --ci`; ordered modules retain serial execution.
Unported stages are listed explicitly. Product code reads neither switch and
never records or logs snapshots on its own.

## R2 evidence milestone and next slices

The first Rust stage implements event scope and keys, closed observer authority,
shared and duplicate fee normalization, canonical quantity grouping and
rejections, native zero-delta suppression, and receipt reconciliation. It reuses
the R0 `evidence_snapshot`, `graph_hash`, `observed_quantity_hash` and
`reconciled_observation` families: 312 vectors, including exact preimage bytes.
Other hash/id families remain frozen for their owning future stages.

`tests/custody_snapshot_generator.py` produces 2,000 seeded requests. In
addition to the original accounting shapes it covers successful receipt
reconciliation with deprecated/priority/label/id owner choices, duplicate
wallet/direction/asset slots, tampered version-1 graph and quantity hashes, and
historical address-plus-descriptor overcounts (plus a non-address control).
The current corpus has 345 successful reconciliation books / 690 receipts and
138 duplicate-slot books / slots. Each tampered-hash branch has 69 books.
Permutations reverse receipt sources, ownership-index match lists and wallets
with duplicate labels, as well as the other unordered raw collections. Ordered
transactions and rate rows retain their precedence.

`tests/custody_evidence_vectors.py` records synthetic raw inputs and exact
CPython evidence stages for Rust tests. Every newly ported stage must ship
mutation entries in `crates/custody/scripts/mutations.json`, each killed by its
named tests. Run the catalog using the locked Python environment:

```sh
uv run --locked python crates/custody/scripts/mutation_check.py --check
uv run --locked python crates/custody/scripts/mutation_check.py
```

The shared tax mutation runner copies a scratch tree, proves named tests pass
before mutation, refuses nonbuilding mutations and empty selections, and
requires every named test to fail after mutation. A–F cover the fee keeper,
deprecated owner, closed hashes, native zero delta, leg tie and address-only
historical hold; four additional entries cover source priority, label, id and
CPython key classification for graphless on-chain identities.
The review supplied no exact replacement for D; the catalog explicitly uses
zero-delta predicate inversion for D rather than claiming an unspecified patch.

Snapshot configuration normalization is a deliberate secret-free boundary:
production row copies now carry normalized config text. Frozen-flow shadow
normalizes **only its expected reference** through this declared boundary, then
compares actual config text exactly. Recorder redaction does not participate in
comparison. No normalized amount/hash/id or array-order masking is permitted.

Raw-source loading needs a complete snapshot before successful pure computation.
If receipt-source derivation fails, it runs the earlier gates and pure component
materialization against those raw facts before surfacing the source error,
preserving the frozen flow's component-error precedence. Exact-rate schema
fallback, commitment profile filtering and SQL `COALESCE` semantics (including
empty anchor ids) are retained. Anchor/wallet/revision-target reads intentionally
remain by id, just like the original selectors: cross-profile references must
reach validation so it can distinguish wrong scope from a missing target.
Canonical-anchor and native-row selections still filter the active profile.

The first slice passed all eleven Python shards in custody and tax census
mode: 6,526 tests and 3,830 subtests, with 40 optional integration/feature
skips. The complete recorded test corpus replayed 1,693 raw requests and 1,693
unordered-read permutations against Rust without a difference. The generated
2,000-request comparison and the 203 Rust workspace tests also passed, along
with workspace formatting, clippy and the net crate's observer-feature tests.
Python snapshot/reference shadow comparisons, fresh-process replay and deliberate
subprocess/caught-error discrepancies verify the seams independently.

## R2 components milestone

`crates/custody/src/components` recomputes quantity and conversion conservation,
explicit/inferred allocation checks, suspense and fee coverage, anchor identity,
scope and chronology, continuity across untracked locations, child-count and
author-evidence commitments, native support, lineage/membership conflicts and
blockers. The Python implementation and frozen SQL reference are unchanged.
Both census and shadow compare this stage, including refusal codes; recorded
replay compares recorded/recomputed Python components before comparing Rust.

`tests/custody_component_vectors.py` freezes 156 synthetic scenarios plus 25
refusal/admission cases for independent named Rust tests. A separate CPython 3.11 fixture
pins Unicode decimal `int()` strings, Unicode 14 uppercase mappings, ISO calendar
and week dates, a full product of five separators and sixteen zones (including
date-plus-zone and whitespace before offsets), fractions and UTC overflow.
A separate 9,477-case seeded differential fuzz corpus is regenerated from
CPython by the vector check; these are tested spellings, not a claim of complete
`fromisoformat` grammar coverage.
Chronology keeps integer microseconds and the exact 48-hour tolerance, including
Python's overflow refusal on tolerance addition. Existing CPython strip/lower
helpers are reused; simultaneous malformed-child/header/time cases pin refusal
precedence. The decimal table is regenerated by
`tests/custody_python_text.py`. Activation detail hashes use `kassiber-pyjson`
and exclude only fingerprint, occurrence, confirmation and raw JSON fields.

The component generator adds 2,000 requests without changing the original
2,000 evidence books. It covers missing/excluded/retracted anchors, incomplete
components, conflicting active revisions, child ordering, N:M allocations,
conservation/valuation failures, suspense, commitment drift, scope laundering,
native support and CPython semantics. It prints counts for every scenario,
validation code, effective state and evidence/native status. Component-only
permutations reverse the native transaction population as well as component,
anchor, wallet and revision collections; ordered child rows retain their
ordinal/id order. Full-pipeline permutations preserve transaction/rate precedence.

The reviewed corpus has 156 scenarios over 2,000 books, with 474 active,
1,629 blocked and 13 inactive authored components. The generator asserts each
scenario's active count; successful controls include suspense, conversion,
N:M allocations, peg-in/out, LND/NWC/CLN receives, a reviewed swap with economic
terms, a two-component untracked route, and an unscoped transactionless Liquid
wallet leg. Counts are components, so one book can contribute multiple entries.

The mutation catalog has 167 entries. The [R2b review report](custody-r2b-review.md)
records every named killing test, every scenario's active/blocked/inactive counts,
the complete `wide()` audit, and this review's local verification. Reviewer
X07, X14 and X15 are equivalent or loader-equivalent rather than additional
independently killable rules. Python remains the accounting reference.

The following results describe the original R2b milestone before these review
fixes; the linked review report records the new local run. Full census and
custody-shadow shards are rerun by the orchestrator outside this sandbox.

R2b verification passed all eleven census shards (6,595 tests, 3,840 subtests,
55 skips) and all nine custody-shadow shards (3,181 tests, 571 subtests,
11 skips), with no stage or refusal-code differences. Census compared components
and evidence 827 times each; shadow compared each 818 times and completed 817
frozen Python comparisons. The generated suites passed 4,000 raw requests and
the 2,000 component permutations. Recorded replay passed 1,971 requests and
1,971 permutations. Focused generated/frozen refusal checks were rerun after
the final refusal-precedence correction.

Workspace formatting and clippy passed, as did all 394 workspace tests outside
`kassiber-net`. The full workspace run passed eight net tests but the sandbox
refused four owned loopback-server binds with `EPERM`; those tests were not
escalated. Inventory update/check, contract checks and all 145 ledger JSON
feature comparisons also passed. Every one of the 143 mutation entries was
killed after 124 named baseline tests passed unmutated.

## R2c interpreter checkpoint (partial)

The first isolated interpreter is Samourai's tracked Whirlpool lifecycle in
`crates/custody/src/interpretation/samourai.rs`: grouping, candidate/touched ids,
exact same-event MOVE slices, Tx0 coordinator fees, destination-neutral residual
suspense and unverified-group quarantines. It preserves component reservations,
canonical observation aliases, per-hash cursors, Python claim ordinals and
integer msat. Shared helpers preserve Python metadata `str()`/`repr()`, dict
insertion order and Unicode 14 printability; the table is regenerated by
`tests/custody_interpretation_text.py`. Quarantine JSON uses `kassiber-pyjson`.

This is **not an implemented product stage**. `interpretation` remains in
PyO3's `skipped_stages`. A separate, unpackaged Rust example accepts the same
schema-1 raw snapshot and recomputes components/evidence before returning the
isolated slices. `tests/custody_interpretation_partial.py` compares those slices
with Python's original helpers, including the ordinary pair compiler restricted
to Samourai pairs. Census, custody shadow, generated books and recorded replay
run the partial comparison and label it `partial.interpretation.samourai` (and
`partial.interpretation.shared`). The existing stage inventory stays unchanged.
Other interpreters' reservations, holds and interactions await full composition;
these isolated results must not be used as accounting output.

Snapshot v1 and its raw-fact inventory are unchanged: no Python pair, claim,
authority verdict or materialized component crosses the request boundary.
The initial checkpoint's 38 named frozen Rust cases cover shared compilation and 34 Samourai
scenarios plus a JSON float-overflow case. A separate 2,000-book Samourai
generator reaches 1,176 active books, 332 blocked books and 492 inactive/reserved
books, plus 68 full-input permutations. Active means a target-bearing claim without a blocked anchor;
blocked takes precedence when a book contains both a proposed edge and a hold.
The original evidence and component generator corpora remain unchanged.

The initial checkpoint contained 178 mutations: the 143 R2b entries, three shared
compilation entries, and 32 Samourai/claim/text entries. The new entries pin
lifecycle shapes, source/target authority, asset and amount guards, fee
classification/bundling, authored reservations, alias cursors, exact slices,
priority/state and Python text/order semantics. Each names its frozen Rust
case; the complete catalog must still be killed at this checkpoint.

Checkpoint verification killed all 178 mutations after 150 named baseline tests
passed. Workspace formatting and clippy passed, as did all 432 workspace tests
outside `kassiber-net`. The net crate passed eight tests; four owned loopback
binds were refused with `EPERM` and were not escalated. The generated suites
compared 6,000 raw books, including 2,000 component permutations and the 68
Samourai permutations. Recorded replay compared 1,596 requests and 1,596
permutations without a difference. Inventory update/check, contract checks and
the 145 ledger JSON-feature comparisons passed.

All eleven census and nine custody-shadow shards ran, but these lanes are **not
green in this sandbox**: census reported 6,172 passed / 446 failed / 55 skipped,
and shadow 2,986 passed / 214 failed / 11 skipped. The sandbox blocks writes to
the account-home operator runtime and socket creation; daemon/CLI tests can
therefore fail before reaching custody. The two custody daemon acceptance
failures were reproduced as read-only `~/.kassiber/run` errors. There were no
custody-stage, refusal-code or isolated-slice differences; the partial slices
completed 810 census and 801 shadow comparisons each. This does not substitute
for passing those blocked tests in a suitable verification environment.

### Recorded-pair checkpoint (R2c, slice 1)

The generic pair compiler and recorded Bitcoin, Liquid and Lightning matcher
are now ported in `interpretation/pairs.rs` and `interpretation/recorded.rs`.
Samourai uses the same generic compiler. The raw-snapshot comparison additionally
reports `partial.interpretation.recorded`; production `interpretation` remains
skipped, and Python remains authoritative.

The compiler revalidates native endpoint authority, exact recorded pairs and
complete recorded fan-out groups; preserves claim ids, sorted pair ordinals,
canonical hash cursors, reservations and atomic bundles; and reproduces
chronology, exposure, network, rail, fee and residual checks. It also ports
rowless audit adaptation and native-transition **proof validation**. Ownership,
channel and HTLC proof production and their composition remain unported: the
isolated recorded matcher cannot create those proofs. Unit compiler candidates
and proofs are separate fixture inputs, outside the unchanged schema-1 raw
snapshot.

Recorded matching preserves first-seen group order, positive-row cardinality,
Bitcoin wallet separation and native-node Lightning provenance, exact principal,
hash normalization and reliable network scope. Liquid grouping requires an
explicit network and one consensus asset identity, including consistent display
metadata. Nonfinite graph identity values retain Python's invalid-but-present
semantics instead of becoming absent during JSON conversion. The existing
`kassiber-pyjson` encoder produces quarantine JSON; msat remain integers, and
Python integer parsing, unbounded requested amounts, numeric equality and
microsecond timestamp/refusal behavior have frozen witnesses.

There are now 84 frozen raw-snapshot cases (38 previous and 46 recorded), 66
compiler-boundary cases and 36 transfer-predicate cases. The new 2,000-book
recorded generator reaches **789 active, 176 blocked and 1,035 inactive** books,
plus 92 input permutations. The unchanged Samourai corpus still reaches 1,176
active, 332 blocked and 492 inactive books, plus 68 permutations. Together with
the existing evidence/component corpora, verification exercises 8,000 seeded
books, including 2,000 component permutations and 58 evidence permutations.
Blocked status takes precedence over active claims; inactive includes declined
or reserved candidates.

All 232 mutations are killed by their named tests after 201 baseline test
selections pass unmutated:

| Family | Entries |
| --- | ---: |
| Previous components/evidence/shared/Samourai checkpoint | 178 |
| Recorded matching (`J-recorded-*`) | 11 |
| Physical scope and CPython nonfinite identity (`J-scope-*`) | 3 |
| Generic claims, proof validation, fan-out revalidation and rowless audits (`J-pair-*`) | 40 |

Slice 1 verification: workspace formatting and clippy pass; all 545 workspace
tests outside `kassiber-net` pass. The 16 focused custody/transfer modules pass
344 tests and 43 subtests under census, with 136 comparisons of each isolated
slice and no difference. Recorded replay checks 128 raw requests and 128
permutations with no difference. Inventory update/check, contract checks and
all 145 ledger JSON-feature comparisons pass. Full census and custody-shadow
shards are reserved for the orchestrator's environment; they are not attempted
in this sandbox.

### Recorded fan-out checkpoint (R2c, slice 2, partial)

`interpretation/fanout.rs` now ports `derive_recorded_fanout_transfers` and
its outbound-only `_clone_row` call. It counts every positive source before
applying reservations, requires distinct owned destination wallets and exact
conservation, orders legs by amount then id, puts the whole fee on the first
leg, reuses inbound records and drops the consumed source. The separate
`partial.interpretation.fanout` seam compares the complete derivation result,
claims, blocked ids, quarantines and native audits with Python. It reserves
complete recorded pairs and applies component reservations at the shared claim
compiler. Reservations from other interpreters and consolidation-first ordering
remain part of the future composition port.

Raw amounts and fees are converted fallibly where Python calls `int()`;
unbounded integer arithmetic preserves helper-level large sums. Decimal value
pricing retains Python's 32-digit half-even context, msat-to-BTC conversion,
exact-value precedence, zero and empty distinctions, Unicode digits/whitespace,
signed quiet NaN payloads and infinity text. Signalling NaNs and invalid values
refuse the derivation. No raw-value `wide()` reads, new timestamp parser,
dependencies, snapshot fields, Python authority or product-stage enablement are
introduced. `interpretation` remains in `skipped_stages`.

This run stops at a **recorded fan-out commit boundary**. Ownership and
multi-source consolidation are not ported. Their graph helpers need further
work before those candidates can meet parity: the existing Rust graph reader
does not decode Liquid `raw_hex`, and its generic graph-conflict markers do not
preserve Python's detailed field labels. Network recovery must also retain
Liquid asset identity and exact owned-index lookup precedence. These gaps were
identified while walking the Python helpers; no incomplete ownership producer
is left enabled or checked in.

There are 47 new frozen raw-snapshot cases (131 total) and 33 direct deriver
witnesses. Direct witnesses are outside snapshot admission and test handled
source/target ids, counting an already handled second source, nonpositive
destinations, Unicode/string integers, malformed conversions, arbitrary-width
sums and pricing. Every old frozen input and its existing slices remain
unchanged. The fan-out generator adds 2,000 raw books and 94 permutations,
including successful multi-wallet Bitcoin and Liquid graphs and confidential
graphless recorded transfers. It reaches **1,244 active, 126 blocked and 630
inactive** books. All three interpreter generators now assert each scenario's
active/blocked/inactive counts, including Samourai's mixed duplicate-target case.
Active means a target-bearing claim without a blocked anchor; blocked takes
precedence over active; inactive includes reserved and declined candidates.

The branch walk for the ported Python function is recorded below. Every
mutation entry names an independently selectable frozen Rust test in
[`mutations.json`](../../crates/custody/scripts/mutations.json).

| Python branches/rules | Mutation coverage or equivalence |
| --- | --- |
| Canonical grouping, source direction/positivity/cardinality, handled source/target, destination direction/wallet/cardinality/uniqueness | `J-fanout-group-*`, `positive-source`, `source-direction`, `true-source-count`, `one-source`, `handled-*`, `target-*`, `unique-target-wallets` |
| Integer conversions, exact sums, conservation, positive legs, sort amounts/ties and first-leg fee | `fallible-raw-integers`, `exact-msat`, `conservation`, `positive-leg`, `negative-leg`, `amount-order`, `id-tie`, `first-fee` |
| Synthetic ids, source/group labels, cloned rows and dropped source | `split-id`, `source-label`, `group-id`, `synthetic-row`, `drop-source`, `row-id`, `external-id`, `anchor-id`, `kind`, `leg-amount`, `leg-fee` |
| Existing/absent/zero rates, ordinary/exact/falsy values, pricing conversion and cleared absolute values | `preserve-*`, `zero-rate-presence`, `exact-value-precedence`, `falsy-value-fallback`, `missing-value`, `btc-scale`, `decimal-context`, `decimal-whitespace`, `rate-exact`, `clear-*` |
| Nonfinite decimal spelling, signs, Unicode payloads, context truncation, leading zeros and signalling refusal | `nonfinite-*`, `nan-*`, `signalling-nan` |
| Explicit synthetic-prefix rejection after `onchain_transfer_scope` | Equivalent: the shared scope helper already refuses every prefix in `_SYNTHETIC_ID_PREFIXES`; frozen synthetic-source/destination cases pin this. |
| `not ok or not leg_pairs` after the leg loop | Equivalent: nonpositive amounts sort first, so the only failed loop exits with no pairs; `positive-leg`/`negative-leg` cover the common refusal. |
| Clone `full > 0` false branch | Unreachable from this caller: the unique positive source guard precedes cloning. |
| Clone optional id/external-id/kind/journal-id branches | Always supplied by this caller and mutated individually; direction, wallet id and wallet reference are never supplied. The general ownership clone adapter remains unported. |

The catalog now contains 282 mutations: 232 previous entries and 50 new
`J-fanout-*` entries. All 282 are killed by named tests after 237 baseline
test selections pass unmutated:

| Mutation family | Entries | Result |
| --- | ---: | --- |
| Components/evidence/shared/Samourai | 178 | All killed |
| Recorded matcher | 11 | All killed |
| Physical scope and nonfinite identity | 3 | All killed |
| Generic pair compiler, proof validation and rowless audits | 40 | All killed |
| Recorded fan-out producer, raw conversions and split pricing | 50 | All killed |
| **Total** | **282** | **All killed** |

Workspace formatting and clippy pass, as do all 625 workspace tests outside
`kassiber-net`. The generated/frozen comparison suite passes 13 tests over
10,000 seeded books, including 2,000 component permutations, 58 evidence
permutations, 68 Samourai permutations, 92 recorded-pair permutations and 94
fan-out permutations. Sixteen custody-focused Python modules pass 460 tests
and 2,629 subtests under census, with 143 comparisons of every isolated slice
and no differences. Recorded replay compares 135 raw requests and 135
permutations with no difference. Inventory update/check and the inventory/
contract tests pass (10 tests); all 145 ledger feature comparisons are
byte-identical. Full census and custody-shadow shards are left to the
orchestrator outside this sandbox, as requested. Verification uses the pinned
Rust 1.99.1 toolchain, locked Python 3.11 environment, offline Cargo and
disposable temporary roots.

#### Generator activity by scenario

Counts below are from the 2,000 seeded books in each generator. All three
status counts are asserted for every scenario; no successful path can be
replaced by an inactive result while preserving an aggregate acceptance check.

Samourai:

| Scenario | Active | Blocked | Inactive |
| --- | ---: | ---: | ---: |
| `tx0` | 59 | 0 | 0 |
| `tx0_fee` | 59 | 0 | 0 |
| `mixing` | 59 | 0 | 0 |
| `remix` | 59 | 0 | 0 |
| `fanout` | 59 | 0 | 0 |
| `badbank` | 59 | 0 | 0 |
| `untrusted_source` | 0 | 59 | 0 |
| `untrusted_target` | 0 | 59 | 0 |
| `unknown_fee` | 59 | 0 | 0 |
| `implicit_delta` | 59 | 0 | 0 |
| `oversize` | 0 | 59 | 0 |
| `wrong_asset` | 0 | 59 | 0 |
| `group_mismatch` | 0 | 0 | 59 |
| `multiple_sources` | 0 | 0 | 59 |
| `invalid_role` | 0 | 0 | 59 |
| `empty_group` | 0 | 0 | 59 |
| `empty_section` | 0 | 0 | 59 |
| `bool_group` | 59 | 0 | 0 |
| `dict_group` | 59 | 0 | 0 |
| `list_group` | 59 | 0 | 0 |
| `nonprintable_group` | 59 | 0 | 0 |
| `numeric_group` | 59 | 0 | 0 |
| `false_presence_gate` | 0 | 0 | 59 |
| `decoded_config` | 0 | 0 | 59 |
| `mixing_residual` | 59 | 0 | 0 |
| `fee_inclusive` | 59 | 0 | 0 |
| `tampered_hash` | 0 | 59 | 0 |
| `sorted_targets` | 59 | 0 | 0 |
| `duplicate_target` | 0 | 37 | 21 |
| `reviewed_reservation` | 0 | 0 | 58 |
| `multiple_groups` | 58 | 0 | 0 |
| `post_unicode14_group` | 58 | 0 | 0 |
| `implicit_mix_residual` | 58 | 0 | 0 |
| `exact_msat` | 58 | 0 | 0 |
| **Total** | **1176** | **332** | **492** |

Recorded:

| Scenario | Active | Blocked | Inactive |
| --- | ---: | ---: | ---: |
| `bitcoin` | 44 | 0 | 0 |
| `bitcoin_upper_txid` | 44 | 0 | 0 |
| `bitcoin_typed_external` | 0 | 0 | 44 |
| `bitcoin_timestamp_skew` | 44 | 0 | 0 |
| `bitcoin_fee_inclusive` | 44 | 0 | 0 |
| `bitcoin_implicit_residual` | 44 | 0 | 0 |
| `bitcoin_partial` | 44 | 0 | 0 |
| `exact_msat` | 44 | 0 | 0 |
| `liquid_main` | 44 | 0 | 0 |
| `liquid_test` | 44 | 0 | 0 |
| `liquid_regtest` | 44 | 0 | 0 |
| `liquid_asset_id` | 44 | 0 | 0 |
| `lightning_lnd` | 44 | 0 | 0 |
| `lightning_cln` | 44 | 0 | 0 |
| `lightning_circular` | 44 | 0 | 0 |
| `lightning_hash_case` | 44 | 0 | 0 |
| `untrusted_source` | 0 | 44 | 0 |
| `untrusted_target` | 0 | 44 | 0 |
| `tampered_quantity` | 0 | 44 | 0 |
| `lightning_chronology` | 0 | 44 | 0 |
| `untrusted_both` | 0 | 0 | 44 |
| `same_wallet` | 0 | 0 | 44 |
| `zero_inbound` | 0 | 0 | 43 |
| `negative_outbound` | 0 | 0 | 43 |
| `multiple_sources` | 0 | 0 | 43 |
| `multiple_targets` | 0 | 0 | 43 |
| `different_txid` | 0 | 0 | 43 |
| `network_separation` | 0 | 0 | 43 |
| `lightning_shortfall` | 0 | 0 | 43 |
| `lightning_forged_source` | 0 | 0 | 43 |
| `lightning_forged_lnd` | 0 | 0 | 43 |
| `lightning_bad_hash` | 0 | 0 | 43 |
| `lightning_wrong_kind` | 0 | 0 | 43 |
| `lightning_chain_hash` | 0 | 0 | 43 |
| `lightning_duplicate` | 0 | 0 | 43 |
| `lightning_no_network` | 0 | 0 | 43 |
| `lightning_asset_case` | 0 | 0 | 43 |
| `liquid_missing_identity` | 0 | 0 | 43 |
| `liquid_asset_separation` | 0 | 0 | 43 |
| `synthetic_id` | 0 | 0 | 43 |
| `reviewed_reservation` | 0 | 0 | 43 |
| `multiple_events` | 43 | 0 | 0 |
| `nonpositive_placeholder` | 0 | 0 | 43 |
| `nonfinite_graph_identity` | 0 | 0 | 43 |
| `lightning_skew_boundary` | 43 | 0 | 0 |
| `lightning_invalid_timestamp` | 43 | 0 | 0 |
| **Total** | **789** | **176** | **1035** |

Fan-out:

| Scenario | Active | Blocked | Inactive |
| --- | ---: | ---: | ---: |
| `bitcoin` | 43 | 0 | 0 |
| `liquid_main` | 43 | 0 | 0 |
| `liquid_test` | 43 | 0 | 0 |
| `liquid_regtest` | 43 | 0 | 0 |
| `liquid_asset_id` | 43 | 0 | 0 |
| `bitcoin_graph` | 43 | 0 | 0 |
| `liquid_graph` | 43 | 0 | 0 |
| `three_destinations` | 43 | 0 | 0 |
| `equal_amount_ties` | 43 | 0 | 0 |
| `amount_order` | 43 | 0 | 0 |
| `provider_ids` | 43 | 0 | 0 |
| `multiple_events` | 43 | 0 | 0 |
| `exact_msat` | 43 | 0 | 0 |
| `value_pricing` | 43 | 0 | 0 |
| `exact_value_pricing` | 43 | 0 | 0 |
| `existing_rate` | 43 | 0 | 0 |
| `existing_exact_rate` | 43 | 0 | 0 |
| `zero_value` | 43 | 0 | 0 |
| `negative_value` | 43 | 0 | 0 |
| `missing_value` | 43 | 0 | 0 |
| `zero_exact_value_fallback` | 43 | 0 | 0 |
| `zero_rate` | 43 | 0 | 0 |
| `whitespace_value` | 43 | 0 | 0 |
| `unicode_value` | 43 | 0 | 0 |
| `nan_value` | 43 | 0 | 0 |
| `nan_payload` | 43 | 0 | 0 |
| `infinite_value` | 42 | 0 | 0 |
| `negative_infinite_value` | 42 | 0 | 0 |
| `untrusted_source` | 0 | 42 | 0 |
| `untrusted_target` | 0 | 42 | 0 |
| `tampered_quantity` | 0 | 42 | 0 |
| `fee_inclusive` | 42 | 0 | 0 |
| `partial_total` | 0 | 0 | 42 |
| `over_total` | 0 | 0 | 42 |
| `duplicate_wallet` | 0 | 0 | 42 |
| `same_wallet` | 0 | 0 | 42 |
| `one_destination` | 0 | 0 | 42 |
| `multiple_sources` | 0 | 0 | 42 |
| `zero_source` | 0 | 0 | 42 |
| `zero_destination` | 0 | 0 | 42 |
| `negative_destination` | 0 | 0 | 42 |
| `unknown_direction` | 0 | 0 | 42 |
| `asset_separation` | 0 | 0 | 42 |
| `network_separation` | 0 | 0 | 42 |
| `synthetic_source` | 0 | 0 | 42 |
| `synthetic_destination` | 0 | 0 | 42 |
| `reviewed_reservation` | 0 | 0 | 42 |
| **Total** | **1244** | **126** | **630** |

### Rebased interpretation audit (R2c)

The interpretation checkpoints are rebased onto components fixes at `16b5b57d`.
All 139 interpretation mutation rules are preserved alongside the fixed 167-entry
evidence/components catalog. Twelve audit regressions bring the catalog to 318.
Raw recorded comparisons, synthetic fan-out amounts and rowless audit amounts
no longer use `wide()`; conversion order and Python exception families are
pinned by reference-generated vectors. CPython casing and the corrected shared
timestamp parser apply throughout. Snapshot v1 and the product stage inventory
are unchanged; `interpretation` remains skipped.

The [rebase and audit report](custody-r2c-rebase.md) records conflict resolutions,
named mutation witnesses, generator counts, verification and the remaining
Liquid decoding compatibility work. The 10,000 generated books and configured
permutations pass, as do 195 recorded requests and their permutations. Four
daemon acceptance tests remain blocked by the unchanged sandbox owner-lock
restriction; full census/shadow verification belongs to the orchestrator.

Raw-observation holds, privacy boundaries and dependency closure are now ported
through the [safety checkpoint](custody-r2c-safety.md). Continue with:

1. Channel lifecycle and loan roles (`lightning/channel_lifecycle.py`, the
   journal's `channel_context_from_records`, and interpreter role branches).
2. Native HTLC transitions (`custody_native_transitions.py`, 302 lines), with
   `transfer_matching.py` (1,751 lines) and `htlc_parser.py` (352 lines): scoped
   populations, exact proofs, refunds, dismissals, policy and fee timing. The
   shared compiler's proof validator is ported; proof production is not.
3. Native component reconciliation (`custody_native_reconciliation.py`,
   318 lines), component claims (710), FIFO allocations (253), same-time
   chronology (135), and final compilation/hold precedence. Only then expose
   `interpretation` as an implemented stage.

Port the subsequent slices in this order; sizes are approximate Python source
lines to inspect, not estimates of Rust output:

1. Gap search over the supplied worklist: about 1,900 lines.
2. Quantity arbitration, postings and runtime composition: about 2,100 lines
   plus their small allocation and hold helpers.
3. Finalized tax projection: about 1,000 lines. Then join custody and `ledger`
   in `journal`, keeping persistence and pre-build repairs in Python until R4.

Rust admission runs the snapshot book-network and tax-country gates before any
stage and returns the Python error code (or exception class for untyped Python
errors). In particular the current unsupported-country normalizer raises
`ValueError`. Unknown fields on the request, component wrapper and gap worklist
are refused; database row records remain open for schema compatibility.

Rust deliberately refuses REAL `amount`/`fee` values with
`custody_snapshot_invalid`, even where Python's historical `int()` truncates
them. This is a listed fail-closed difference, not permission to truncate at a
Rust admission boundary. CPython 3.11 parser vectors in
`tests/fixtures/custody_semantics.json` pin identity whitespace, Unicode key
normalization, falsy chains, network refusal text and graph index fallbacks.

## Liquid decoder and graph checkpoint (R2c slice 3, partial)

The subsequent [stored-mapping and arithmetic audit](custody-r2c-admission-arithmetic.md)
retains Python scalar types through graph admission and profile merging. It
supersedes the decoder checkpoint's nonfinite-mapping admission caveat.
The arithmetic audit also keeps native deltas, network fees and graph-shape
indices unbounded, preserves Python fee equality and refusal order, and adds
five full-evidence regression fixtures. That checkpoint has 497 graph witnesses
and 438 mutation entries. The ownership producer is ported in
the subsequent checkpoint below; consolidation and pipeline composition remain.
`interpretation` stays skipped.

The public Liquid decoder now reproduces the Python reference's permissive
byte parsing without an additional dependency. Its 3,744 frozen vectors cover
explicit/confidential outputs, peg-ins, issuance, witnesses, truncation, all
prefixes/flags and malformed lengths/hex. Graph prerequisite comparisons add
exact conflict labels, scope recovery, index precedence and scoped profile
merges, with 297 direct witnesses and 2,000 generated successful Python
ownership/consolidation controls. Each generator scenario asserts exactly 250
active reference results; Rust compares the prerequisites only.

Snapshot v1 is unchanged and raw-only. Python remains authoritative and
`interpretation` remains skipped. The catalog adds 28 decoder, 42 graph
and one receipt precision mutation to the retained 318. See the
[decoder/graph checkpoint](custody-r2c-decoder-graph.md) for crate compatibility
differences, named mutation witnesses, verification and remaining graph admission
audit and ownership/consolidation work.

## Ownership production checkpoint (R2c, partial)

The isolated ownership producer now compares complete derivations, generic
claims and rowless native audits through `partial.interpretation.ownership`.
It selects component inputs, proves source ownership and Liquid conservation,
resolves or declines destinations, clones priced rows, allocates the fee once,
retains external residuals and releases tentative receipts on a declined split.
Stored graph scalars remain typed through production. Python remains
accounting authority; snapshot v1 is unchanged and `interpretation` stays skipped.

The graph generator asserts **250 successful Python and Rust ownership
results per scenario** for Bitcoin, Liquid and four Liquid raw-byte variants
(1,500 successes per producer). The two consolidation scenarios still assert
250 Python successes each; they do not claim a Rust consolidation producer.
The [ownership handoff](custody-r2c-ownership.md) records branch witnesses,
mutation accounting, verification and the next slice. This run stops at the
ownership commit boundary before starting consolidation.

## Consolidation production checkpoint (R2c, partial)

`partial.interpretation.consolidation` now compares the complete Python and Rust
producer results, generic claims and native audits. It preserves collaborative
own-delta qualification, normalized versus legacy fee handling, exact recorded
receipts, the mixed real/synthetic destination barrier, and deterministic FIFO
source/sink cells. Quantities remain unbounded after Python integer conversion;
raw facts do not use `codec::wide()`. Graph scalars remain typed and shared
CPython casing, timestamp and pricing helpers retain their previous contracts.

The graph corpus now asserts **250 successes in each engine for each of
`bitcoin_consolidation` and `liquid_consolidation`**, each with two valid claims.
The six ownership scenarios retain their 250 successes per engine. The
[consolidation handoff](custody-r2c-consolidation.md) records branch witnesses,
mutation accounting and verification. Snapshot v1 remains raw-only, Python
remains authoritative, and `interpretation` remains skipped. The consolidation
commit precedes any work on composing the three producers.

## Producer composition checkpoint (R2c, partial)

`partial.interpretation.pipeline` composes consolidation -> recorded fan-out ->
ownership using Python's row replacement/drop/append order and handled-id
reservations. Its partial compiler preserves complete recorded/Samourai
reservations, canonical implicit-fee normalization, ownership residual
annotations and event-wide producer quarantine expansion with authored-member
exemptions. Other interpreter reservations, holds, channels, native transitions,
reconciliation and final composition remain separate work.

The composed corpus compares 2,000 raw books and 24 permutations, asserting
per-scenario successes for **each engine and each producer**. Both engines
produce 1,000 successful consolidations, 668 fan-outs and 834 ownership results;
mixed books contribute to all three counts. A separate ambiguity control has
166 books with two atomic quarantine members each. Twenty-four controls also
match the full Python interpreter's claims and audits. The
[composition handoff](custody-r2c-composition.md) records the scenario table,
mutation accounting and verification. All 681 catalog entries have named-test
kills. Python remains authoritative, snapshot v1 stays raw-only and
`interpretation` stays skipped.

## Shared JSON feature contract

Cargo unifies custody's `serde_json/arbitrary_precision` and `float_roundtrip`
features into the ledger build. The former preserves unbounded JSON integers;
the latter makes float literal loading agree with CPython. This is an explicit
shared build contract, not an isolated custody dependency choice.
`crates/custody/scripts/check_ledger_features.py` builds the ledger example twice,
with and without those features, and compares all 145 stored ledger response
strings byte for byte. CI's Rust job and the quality gate require this check.

## Raw holds, privacy and dependency checkpoint (R2c, partial)

The new `partial.interpretation.safety` seam composes the existing producers in
Python order, then applies privacy resolution, raw conflicting/pending/fan-out/
unscoped holds, whole-group pair filtering and fixed-point dependency closure
before compiling claims and rowless audits. Current closed confirmation
chronology, reason precedence, reviewed exemptions, immediate causes, transient
propagation and quarantine ordering are retained. A held row never creates a
destination or carries basis. Canonical fee-only contributors use the existing
principal computation; raw snapshot facts never gain a `principal_msat` verdict.

There are 254 raw partial projections and 249 direct helper witnesses. The
2,000-book safety corpus has 39 asserted scenarios per engine, 78 permutations
and 78 full-Python controls, including each hold reason and privacy boundary.
The catalog adds 114 safety entries to the retained 681. The
[safety handoff](custody-r2c-safety.md) owns the complete branch dispositions,
per-scenario producer counts and verification results. Snapshot v1 is unchanged
and raw-only; `interpretation` remains skipped. Channels/loans, native HTLC
production, reconciliation and final composition are still separate slices.
